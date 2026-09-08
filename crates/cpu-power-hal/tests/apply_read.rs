//! Table-driven apply/read round-trips over all 5 profiles, against both
//! EPP-capable fixtures (Intel and AMD), through the public detect_backend
//! entry point rather than constructing backends directly.

mod fixtures;

use cpu_power_hal::{PowerProfile, detect_backend};

#[test]
fn intel_apply_and_read_round_trip_all_profiles() {
    let (_dir, io) = fixtures::intel_epp_nuc();
    let backend = detect_backend(io);
    for profile in PowerProfile::ALL {
        backend.apply(profile).unwrap();
        assert_eq!(backend.current().unwrap(), Some(profile));
    }
}

#[test]
fn amd_apply_and_read_round_trip_all_profiles() {
    let (_dir, io) = fixtures::amd_epp_strix_halo();
    let backend = detect_backend(io);
    for profile in PowerProfile::ALL {
        backend.apply(profile).unwrap();
        assert_eq!(backend.current().unwrap(), Some(profile));
    }
}
