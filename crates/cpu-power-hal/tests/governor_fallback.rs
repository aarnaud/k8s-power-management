//! Confirms the lossy collapse behavior of the governor-fallback backend
//! against the older-NUC fixture: performance and power stay distinct,
//! the middle three profiles collapse onto whatever shared governor this
//! driver actually offers.

mod fixtures;

use cpu_power_hal::{PowerProfile, detect_backend};

fn read_governor(dir: &std::path::Path) -> String {
    std::fs::read_to_string(dir.join("sys/devices/system/cpu/cpufreq/policy0/scaling_governor"))
        .unwrap()
}

#[test]
fn extremes_stay_distinct_middle_profiles_collapse() {
    let (dir, io) = fixtures::intel_governor_only_nuc();
    let backend = detect_backend(io);

    backend.apply(PowerProfile::Performance).unwrap();
    assert_eq!(read_governor(dir.path()), "performance");

    backend.apply(PowerProfile::Power).unwrap();
    assert_eq!(read_governor(dir.path()), "powersave");

    // This fixture's scaling_available_governors has no "schedutil", so all
    // three of these should fall back to "ondemand".
    for profile in [
        PowerProfile::Default,
        PowerProfile::BalancePerformance,
        PowerProfile::BalancePower,
    ] {
        backend.apply(profile).unwrap();
        assert_eq!(read_governor(dir.path()), "ondemand");
    }
}

#[test]
fn current_is_not_invertible_on_governor_fallback() {
    let (_dir, io) = fixtures::intel_governor_only_nuc();
    let backend = detect_backend(io);
    backend.apply(PowerProfile::Performance).unwrap();
    // Governor -> profile is many-to-one; current() must not fabricate a
    // profile it can't actually distinguish.
    assert_eq!(backend.current().unwrap(), None);
}
