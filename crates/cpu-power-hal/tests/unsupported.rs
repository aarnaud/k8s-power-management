//! The QEMU/no-cpufreq case must never error or panic for any input — this
//! is the property that keeps that node's agent pod out of
//! CrashLoopBackOff regardless of what labels get applied to it.

mod fixtures;

use cpu_power_hal::{PowerProfile, detect_backend};

#[test]
fn every_profile_applies_as_a_no_op() {
    let (dir, io) = fixtures::unsupported_qemu();
    let backend = detect_backend(io);

    for profile in PowerProfile::ALL {
        assert!(backend.apply(profile).is_ok());
    }
    assert_eq!(backend.current().unwrap(), None);
    assert!(backend.set_turbo(true).is_ok());
    assert!(backend.set_turbo(false).is_ok());

    // None of the above should have fabricated a cpufreq tree that wasn't
    // there to begin with.
    assert!(!dir.path().join("sys/devices/system/cpu/cpufreq").exists());
}
