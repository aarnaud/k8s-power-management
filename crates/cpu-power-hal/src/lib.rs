//! Vendor/generation-agnostic Linux CPU power management.
//!
//! This crate abstracts away the differences between Intel and AMD cpufreq
//! drivers, and between CPU generations that do or don't support EPP
//! (Energy Performance Preference), behind a small [`PowerBackend`] trait.
//! It has no dependency on Kubernetes, tokio, or any async runtime — it is
//! pure, synchronous, sysfs-in/sysfs-out, and is fully testable via the
//! [`SysfsIo`] boundary without real hardware.

pub mod backend;
pub mod error;
pub mod profile;
pub mod sysfs;
pub mod topology;

pub use backend::{BackendKind, PowerBackend, Vendor, detect_backend};
pub use error::PowerError;
pub use profile::PowerProfile;
pub use sysfs::{RootedSysfs, SysfsIo};
pub use topology::{CpuPolicy, discover_policies};
