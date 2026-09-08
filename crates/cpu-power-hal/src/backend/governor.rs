use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tracing::warn;

use crate::error::PowerError;
use crate::profile::PowerProfile;
use crate::sysfs::SysfsIo;
use crate::topology::CpuPolicy;

use super::{BackendKind, PowerBackend};

/// Fallback for CPUs/drivers without EPP support: older/passive-mode
/// `intel_pstate`, or generic `acpi-cpufreq`. `scaling_governor` only
/// offers a handful of values (driver-dependent), so this backend
/// necessarily collapses the 5-level profile scale onto fewer distinct
/// kernel states — expected to be exercised by the older generations among
/// a mixed-generation NUC fleet.
pub struct GovernorFallbackBackend {
    io: Arc<dyn SysfsIo>,
    policies: Vec<CpuPolicy>,
    warned_collapse: AtomicBool,
}

impl GovernorFallbackBackend {
    pub fn new(io: Arc<dyn SysfsIo>, policies: Vec<CpuPolicy>) -> Self {
        Self {
            io,
            policies,
            warned_collapse: AtomicBool::new(false),
        }
    }

    fn governor_path(policy: &CpuPolicy) -> PathBuf {
        policy.path.join("scaling_governor")
    }

    /// Maps a profile onto the best available governor for this policy's
    /// advertised `scaling_available_governors`.
    fn map_governor(profile: PowerProfile, available: &[String]) -> &'static str {
        let has = |name: &str| available.iter().any(|g| g == name);
        match profile {
            PowerProfile::Performance => "performance",
            PowerProfile::Power => "powersave",
            PowerProfile::Default
            | PowerProfile::BalancePerformance
            | PowerProfile::BalancePower => {
                if has("schedutil") {
                    "schedutil"
                } else if has("ondemand") {
                    "ondemand"
                } else {
                    "powersave"
                }
            }
        }
    }
}

impl PowerBackend for GovernorFallbackBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::GovernorFallback
    }

    fn is_supported(&self) -> bool {
        true
    }

    fn apply(&self, profile: PowerProfile) -> Result<(), PowerError> {
        let collapses = matches!(
            profile,
            PowerProfile::Default | PowerProfile::BalancePerformance | PowerProfile::BalancePower
        );
        if collapses && !self.warned_collapse.swap(true, Ordering::Relaxed) {
            warn!(
                "no EPP support on this CPU/driver: default, balance_performance and \
                 balance_power all collapse onto the same cpufreq governor here \
                 (performance and power remain distinct)"
            );
        }

        for policy in &self.policies {
            let governor = Self::map_governor(profile, &policy.available_governors);
            if !policy.available_governors.iter().any(|g| g == governor) {
                return Err(PowerError::UnsupportedProfile {
                    policy: policy.id,
                    profile: profile.as_kernel_str().to_string(),
                    available: policy.available_governors.clone(),
                });
            }
            let path = Self::governor_path(policy);
            self.io
                .write(&path, governor)
                .map_err(|source| PowerError::Io { path, source })?;
        }
        Ok(())
    }

    fn current(&self) -> Result<Option<PowerProfile>, PowerError> {
        // Governor -> profile is many-to-one and thus not invertible; do
        // not pretend we can recover a unique PowerProfile from it.
        Ok(None)
    }

    fn set_turbo(&self, _enabled: bool) -> Result<(), PowerError> {
        Err(PowerError::TurboUnsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sysfs::RootedSysfs;
    use std::fs;
    use std::path::Path;

    fn policy(io_root: &Path, id: u32, governors: &[&str]) -> CpuPolicy {
        let path = io_root
            .join("sys/devices/system/cpu/cpufreq")
            .join(format!("policy{id}"));
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("scaling_driver"), "acpi-cpufreq").unwrap();
        fs::write(
            path.join("scaling_available_governors"),
            governors.join(" "),
        )
        .unwrap();
        fs::write(path.join("scaling_governor"), "ondemand").unwrap();
        CpuPolicy {
            id,
            path: PathBuf::from(format!("/sys/devices/system/cpu/cpufreq/policy{id}")),
            related_cpus: vec![id],
            scaling_driver: "acpi-cpufreq".to_string(),
            has_epp: false,
            available_epp: vec![],
            available_governors: governors.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn maps_performance_and_power_distinctly() {
        let dir = tempfile::tempdir().unwrap();
        let policies = vec![policy(
            dir.path(),
            0,
            &["performance", "powersave", "schedutil"],
        )];
        let io: Arc<dyn SysfsIo> = Arc::new(RootedSysfs::new(dir.path()));
        let backend = GovernorFallbackBackend::new(io.clone(), policies);

        backend.apply(PowerProfile::Performance).unwrap();
        assert_eq!(
            io.read_to_string(Path::new(
                "/sys/devices/system/cpu/cpufreq/policy0/scaling_governor"
            ))
            .unwrap(),
            "performance"
        );

        backend.apply(PowerProfile::Power).unwrap();
        assert_eq!(
            io.read_to_string(Path::new(
                "/sys/devices/system/cpu/cpufreq/policy0/scaling_governor"
            ))
            .unwrap(),
            "powersave"
        );
    }

    #[test]
    fn collapses_middle_profiles_onto_schedutil() {
        let dir = tempfile::tempdir().unwrap();
        let policies = vec![policy(
            dir.path(),
            0,
            &["performance", "powersave", "schedutil"],
        )];
        let io: Arc<dyn SysfsIo> = Arc::new(RootedSysfs::new(dir.path()));
        let backend = GovernorFallbackBackend::new(io.clone(), policies);

        for profile in [
            PowerProfile::Default,
            PowerProfile::BalancePerformance,
            PowerProfile::BalancePower,
        ] {
            backend.apply(profile).unwrap();
            assert_eq!(
                io.read_to_string(Path::new(
                    "/sys/devices/system/cpu/cpufreq/policy0/scaling_governor"
                ))
                .unwrap(),
                "schedutil"
            );
        }
    }

    #[test]
    fn falls_back_to_ondemand_when_schedutil_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let policies = vec![policy(
            dir.path(),
            0,
            &["performance", "powersave", "ondemand"],
        )];
        let io: Arc<dyn SysfsIo> = Arc::new(RootedSysfs::new(dir.path()));
        let backend = GovernorFallbackBackend::new(io.clone(), policies);

        backend.apply(PowerProfile::BalancePower).unwrap();
        assert_eq!(
            io.read_to_string(Path::new(
                "/sys/devices/system/cpu/cpufreq/policy0/scaling_governor"
            ))
            .unwrap(),
            "ondemand"
        );
    }

    #[test]
    fn turbo_is_unsupported() {
        let dir = tempfile::tempdir().unwrap();
        let policies = vec![policy(dir.path(), 0, &["performance", "powersave"])];
        let io: Arc<dyn SysfsIo> = Arc::new(RootedSysfs::new(dir.path()));
        let backend = GovernorFallbackBackend::new(io, policies);

        assert!(matches!(
            backend.set_turbo(true),
            Err(PowerError::TurboUnsupported)
        ));
    }
}
