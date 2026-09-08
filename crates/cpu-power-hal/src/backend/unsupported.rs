use crate::error::PowerError;
use crate::profile::PowerProfile;

use super::{BackendKind, PowerBackend};

/// No cpufreq policies were found at all — the expected shape for e.g. a
/// QEMU guest without host CPU passthrough. Every operation is a
/// first-class no-op success, never an `Err`: this is the property that
/// keeps that node's agent pod out of CrashLoopBackOff regardless of what
/// profile/turbo labels get applied to it.
pub struct UnsupportedBackend;

impl PowerBackend for UnsupportedBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Unsupported
    }

    fn is_supported(&self) -> bool {
        false
    }

    fn apply(&self, _profile: PowerProfile) -> Result<(), PowerError> {
        Ok(())
    }

    fn current(&self) -> Result<Option<PowerProfile>, PowerError> {
        Ok(None)
    }

    fn set_turbo(&self, _enabled: bool) -> Result<(), PowerError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_operation_is_a_no_op_success() {
        let backend = UnsupportedBackend;
        assert!(backend.apply(PowerProfile::Performance).is_ok());
        assert_eq!(backend.current().unwrap(), None);
        assert!(backend.set_turbo(true).is_ok());
        assert!(backend.set_turbo(false).is_ok());
        assert!(!backend.is_supported());
    }
}
