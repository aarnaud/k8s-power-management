//! Backend detection across the actual fleet shapes — the highest-value
//! test given it directly models the user's heterogeneous hardware.

mod fixtures;

use cpu_power_hal::{BackendKind, detect_backend};

#[test]
fn detects_intel_epp_nuc() {
    let (_dir, io) = fixtures::intel_epp_nuc();
    let backend = detect_backend(io);
    assert_eq!(backend.kind(), BackendKind::IntelEpp);
    assert!(backend.is_supported());
}

#[test]
fn detects_intel_governor_only_nuc() {
    let (_dir, io) = fixtures::intel_governor_only_nuc();
    let backend = detect_backend(io);
    assert_eq!(backend.kind(), BackendKind::GovernorFallback);
    assert!(backend.is_supported());
}

#[test]
fn detects_amd_epp_strix_halo() {
    let (_dir, io) = fixtures::amd_epp_strix_halo();
    let backend = detect_backend(io);
    assert_eq!(backend.kind(), BackendKind::AmdEpp);
    assert!(backend.is_supported());
}

#[test]
fn detects_unsupported_qemu() {
    let (_dir, io) = fixtures::unsupported_qemu();
    let backend = detect_backend(io);
    assert_eq!(backend.kind(), BackendKind::Unsupported);
    assert!(!backend.is_supported());
}
