pub mod epp;
pub mod governor;
pub mod unsupported;

use std::fmt;
use std::sync::Arc;

use crate::error::PowerError;
use crate::profile::PowerProfile;
use crate::sysfs::SysfsIo;
use crate::topology::discover_policies;

/// Which concrete backend was selected for this node. Surfaced up through
/// the k8s agent's Node annotations/metrics so `kubectl get node -o yaml`
/// shows *how* a profile is actually being enforced, not just what was
/// requested — this matters because `GovernorFallback` silently collapses
/// some of the 5 requested profiles onto shared governors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    IntelEpp,
    AmdEpp,
    /// An EPP-capable driver was found but its name didn't match a known
    /// Intel/AMD pattern. Profile control still works identically (same
    /// kernel ABI); only vendor-specific turbo handling is unavailable.
    UnknownEpp,
    GovernorFallback,
    Unsupported,
}

impl fmt::Display for BackendKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            BackendKind::IntelEpp => "intel_epp",
            BackendKind::AmdEpp => "amd_epp",
            BackendKind::UnknownEpp => "unknown_epp",
            BackendKind::GovernorFallback => "governor_fallback",
            BackendKind::Unsupported => "unsupported",
        };
        f.write_str(s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vendor {
    Intel,
    Amd,
    Unknown,
}

/// Uniform mechanism surface every concrete backend implements. Detection
/// (`detect_backend`) never fails — it always returns a usable backend, in
/// the worst case `UnsupportedBackend` — so callers never need a separate
/// "no backend available" error path on top of this trait's own `Result`s.
pub trait PowerBackend: Send + Sync {
    fn kind(&self) -> BackendKind;
    fn is_supported(&self) -> bool;
    fn apply(&self, profile: PowerProfile) -> Result<(), PowerError>;
    fn current(&self) -> Result<Option<PowerProfile>, PowerError>;
    /// `enabled` is always the positive sense ("turbo is allowed to
    /// engage"); each backend hides its own sign convention internally
    /// (Intel's `intel_pstate/no_turbo` is inverted, AMD's `cpufreq/boost`
    /// is direct) so callers never need to know which vendor they're on.
    fn set_turbo(&self, enabled: bool) -> Result<(), PowerError>;
}

/// Detect the appropriate backend for this host by inspecting its cpufreq
/// topology. Always returns a usable backend.
pub fn detect_backend(io: Arc<dyn SysfsIo>) -> Box<dyn PowerBackend> {
    let policies = discover_policies(io.as_ref()).unwrap_or_default();

    let Some(first) = policies.first() else {
        return Box::new(unsupported::UnsupportedBackend);
    };

    if first.has_epp {
        let vendor = infer_vendor(&first.scaling_driver);
        return Box::new(epp::EppBackend::new(io, policies, vendor));
    }

    if !first.available_governors.is_empty() {
        return Box::new(governor::GovernorFallbackBackend::new(io, policies));
    }

    Box::new(unsupported::UnsupportedBackend)
}

fn infer_vendor(scaling_driver: &str) -> Vendor {
    let lower = scaling_driver.to_ascii_lowercase();
    if lower.contains("intel") {
        Vendor::Intel
    } else if lower.contains("amd") {
        Vendor::Amd
    } else {
        Vendor::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vendor_inference_matches_known_driver_names() {
        assert_eq!(infer_vendor("intel_pstate"), Vendor::Intel);
        assert_eq!(infer_vendor("amd-pstate-epp"), Vendor::Amd);
        assert_eq!(infer_vendor("amd_pstate_epp"), Vendor::Amd);
        assert_eq!(infer_vendor("acpi-cpufreq"), Vendor::Unknown);
    }
}
