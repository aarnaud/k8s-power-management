use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::warn;

use crate::error::PowerError;
use crate::profile::PowerProfile;
use crate::sysfs::SysfsIo;
use crate::topology::CpuPolicy;

use super::{BackendKind, PowerBackend, Vendor};

const INTEL_NO_TURBO_PATH: &str = "/sys/devices/system/cpu/intel_pstate/no_turbo";
/// Best-effort AMD-specific boost path, preferred over the generic one
/// when present. The exact sysfs surface for AMD boost control has moved
/// around across kernel versions; this is flagged for confirmation against
/// real Framework Desktop hardware via `pstate-cli probe` (see the AMD
/// validation fast-follow item), with the generic path below as the
/// well-documented fallback that works across drivers.
const AMD_PSTATE_BOOST_PATH: &str = "/sys/devices/system/cpu/amd_pstate/boost";
const GENERIC_CPUFREQ_BOOST_PATH: &str = "/sys/devices/system/cpu/cpufreq/boost";

/// Handles both Intel `intel_pstate` (active/HWP mode) and AMD
/// `amd-pstate-epp` through the *same* code path: the mechanism for
/// applying/reading a profile is byte-for-byte identical at the kernel ABI
/// level (`energy_performance_preference`, same 5 string values) on both
/// vendors, so there is no `IntelEppBackend`/`AmdEppBackend` duplication to
/// begin with. `Vendor` only matters for `set_turbo`, the one place the
/// underlying mechanism genuinely diverges.
pub struct EppBackend {
    io: Arc<dyn SysfsIo>,
    policies: Vec<CpuPolicy>,
    vendor: Vendor,
}

impl EppBackend {
    pub fn new(io: Arc<dyn SysfsIo>, policies: Vec<CpuPolicy>, vendor: Vendor) -> Self {
        Self {
            io,
            policies,
            vendor,
        }
    }

    fn epp_path(policy: &CpuPolicy) -> PathBuf {
        policy.path.join("energy_performance_preference")
    }
}

impl PowerBackend for EppBackend {
    fn kind(&self) -> BackendKind {
        match self.vendor {
            Vendor::Intel => BackendKind::IntelEpp,
            Vendor::Amd => BackendKind::AmdEpp,
            Vendor::Unknown => BackendKind::UnknownEpp,
        }
    }

    fn is_supported(&self) -> bool {
        true
    }

    fn apply(&self, profile: PowerProfile) -> Result<(), PowerError> {
        let value = profile.as_kernel_str();
        for policy in &self.policies {
            if !policy.available_epp.is_empty() && !policy.available_epp.iter().any(|v| v == value)
            {
                return Err(PowerError::UnsupportedProfile {
                    policy: policy.id,
                    profile: value.to_string(),
                    available: policy.available_epp.clone(),
                });
            }
            let path = Self::epp_path(policy);
            self.io
                .write(&path, value)
                .map_err(|source| PowerError::Io { path, source })?;
        }
        Ok(())
    }

    fn current(&self) -> Result<Option<PowerProfile>, PowerError> {
        let Some(first) = self.policies.first() else {
            return Ok(None);
        };
        let path = Self::epp_path(first);
        let raw = self
            .io
            .read_to_string(&path)
            .map_err(|source| PowerError::Io { path, source })?;
        match raw.parse::<PowerProfile>() {
            Ok(profile) => Ok(Some(profile)),
            Err(_) => {
                warn!(value = %raw, "kernel reported an energy_performance_preference value outside the known profile set");
                Ok(None)
            }
        }
    }

    fn set_turbo(&self, enabled: bool) -> Result<(), PowerError> {
        match self.vendor {
            Vendor::Intel => {
                // no_turbo is inverted: "1" means turbo is *disabled*.
                let value = if enabled { "0" } else { "1" };
                let path = Path::new(INTEL_NO_TURBO_PATH);
                self.io.write(path, value).map_err(|source| PowerError::Io {
                    path: path.to_path_buf(),
                    source,
                })
            }
            Vendor::Amd => {
                let value = if enabled { "1" } else { "0" };
                let preferred = Path::new(AMD_PSTATE_BOOST_PATH);
                let path = if self.io.exists(preferred) {
                    preferred
                } else {
                    Path::new(GENERIC_CPUFREQ_BOOST_PATH)
                };
                self.io.write(path, value).map_err(|source| PowerError::Io {
                    path: path.to_path_buf(),
                    source,
                })
            }
            Vendor::Unknown => {
                warn!(
                    "turbo control skipped: EPP driver present but vendor could not be inferred from scaling_driver"
                );
                Err(PowerError::TurboUnsupported)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sysfs::RootedSysfs;
    use std::fs;

    fn policy(io_root: &Path, id: u32, driver: &str, available: &[&str]) -> CpuPolicy {
        let path = io_root
            .join("sys/devices/system/cpu/cpufreq")
            .join(format!("policy{id}"));
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("scaling_driver"), driver).unwrap();
        fs::write(
            path.join("energy_performance_available_preferences"),
            available.join(" "),
        )
        .unwrap();
        fs::write(path.join("energy_performance_preference"), "default").unwrap();
        CpuPolicy {
            id,
            path: PathBuf::from(format!("/sys/devices/system/cpu/cpufreq/policy{id}")),
            related_cpus: vec![id],
            scaling_driver: driver.to_string(),
            has_epp: true,
            available_epp: available.iter().map(|s| s.to_string()).collect(),
            available_governors: vec![],
        }
    }

    #[test]
    fn intel_turbo_is_inverted() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("sys/devices/system/cpu/intel_pstate")).unwrap();
        let policies = vec![policy(
            dir.path(),
            0,
            "intel_pstate",
            &[
                "default",
                "performance",
                "balance_performance",
                "balance_power",
                "power",
            ],
        )];
        let io: Arc<dyn SysfsIo> = Arc::new(RootedSysfs::new(dir.path()));
        let backend = EppBackend::new(io.clone(), policies, Vendor::Intel);

        backend.set_turbo(true).unwrap();
        assert_eq!(
            io.read_to_string(Path::new(INTEL_NO_TURBO_PATH)).unwrap(),
            "0"
        );

        backend.set_turbo(false).unwrap();
        assert_eq!(
            io.read_to_string(Path::new(INTEL_NO_TURBO_PATH)).unwrap(),
            "1"
        );
    }

    #[test]
    fn amd_turbo_is_direct() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("sys/devices/system/cpu/amd_pstate")).unwrap();
        // Real sysfs files pre-exist with some initial value; `exists()` is
        // how the backend decides whether to use the amd_pstate-specific
        // boost knob or fall back to the generic cpufreq one.
        fs::write(
            dir.path().join("sys/devices/system/cpu/amd_pstate/boost"),
            "0",
        )
        .unwrap();
        let policies = vec![policy(
            dir.path(),
            0,
            "amd-pstate-epp",
            &[
                "default",
                "performance",
                "balance_performance",
                "balance_power",
                "power",
            ],
        )];
        let io: Arc<dyn SysfsIo> = Arc::new(RootedSysfs::new(dir.path()));
        let backend = EppBackend::new(io.clone(), policies, Vendor::Amd);

        backend.set_turbo(true).unwrap();
        assert_eq!(
            io.read_to_string(Path::new(AMD_PSTATE_BOOST_PATH)).unwrap(),
            "1"
        );

        backend.set_turbo(false).unwrap();
        assert_eq!(
            io.read_to_string(Path::new(AMD_PSTATE_BOOST_PATH)).unwrap(),
            "0"
        );
    }

    #[test]
    fn apply_rejects_profile_not_advertised_by_policy() {
        let dir = tempfile::tempdir().unwrap();
        let policies = vec![policy(
            dir.path(),
            0,
            "intel_pstate",
            &["default", "performance"],
        )];
        let io: Arc<dyn SysfsIo> = Arc::new(RootedSysfs::new(dir.path()));
        let backend = EppBackend::new(io, policies, Vendor::Intel);

        let err = backend.apply(PowerProfile::Power).unwrap_err();
        assert!(matches!(err, PowerError::UnsupportedProfile { .. }));
    }

    #[test]
    fn apply_writes_every_policy() {
        let dir = tempfile::tempdir().unwrap();
        let policies = vec![
            policy(
                dir.path(),
                0,
                "intel_pstate",
                &["default", "performance", "power"],
            ),
            policy(
                dir.path(),
                1,
                "intel_pstate",
                &["default", "performance", "power"],
            ),
        ];
        let io: Arc<dyn SysfsIo> = Arc::new(RootedSysfs::new(dir.path()));
        let backend = EppBackend::new(io.clone(), policies, Vendor::Intel);

        backend.apply(PowerProfile::Power).unwrap();
        for id in [0, 1] {
            let path =
                format!("/sys/devices/system/cpu/cpufreq/policy{id}/energy_performance_preference");
            assert_eq!(io.read_to_string(Path::new(&path)).unwrap(), "power");
        }
        assert_eq!(backend.current().unwrap(), Some(PowerProfile::Power));
    }
}
