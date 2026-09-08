//! The clearest regression test for the bug class this abstraction exists
//! to prevent: Intel's `no_turbo` is an inverted boolean, AMD's `boost` is
//! direct, and callers of `set_turbo(enabled)` must never need to know
//! which vendor they're on.

mod fixtures;

use cpu_power_hal::{PowerError, detect_backend};

fn read(dir: &std::path::Path, rel: &str) -> String {
    std::fs::read_to_string(dir.join(rel)).unwrap()
}

#[test]
fn intel_no_turbo_is_inverted() {
    let (dir, io) = fixtures::intel_epp_nuc();
    let backend = detect_backend(io);

    backend.set_turbo(true).unwrap();
    assert_eq!(
        read(dir.path(), "sys/devices/system/cpu/intel_pstate/no_turbo"),
        "0"
    );

    backend.set_turbo(false).unwrap();
    assert_eq!(
        read(dir.path(), "sys/devices/system/cpu/intel_pstate/no_turbo"),
        "1"
    );
}

#[test]
fn amd_boost_is_direct() {
    let (dir, io) = fixtures::amd_epp_strix_halo();
    let backend = detect_backend(io);

    backend.set_turbo(true).unwrap();
    assert_eq!(
        read(dir.path(), "sys/devices/system/cpu/amd_pstate/boost"),
        "1"
    );

    backend.set_turbo(false).unwrap();
    assert_eq!(
        read(dir.path(), "sys/devices/system/cpu/amd_pstate/boost"),
        "0"
    );
}

#[test]
fn governor_fallback_reports_turbo_unsupported_not_an_error_masquerade() {
    let (_dir, io) = fixtures::intel_governor_only_nuc();
    let backend = detect_backend(io);
    assert!(matches!(
        backend.set_turbo(true),
        Err(PowerError::TurboUnsupported)
    ));
}

#[test]
fn unsupported_backend_turbo_is_a_no_op_not_an_error() {
    let (_dir, io) = fixtures::unsupported_qemu();
    let backend = detect_backend(io);
    assert!(backend.set_turbo(true).is_ok());
    assert!(backend.set_turbo(false).is_ok());
}
