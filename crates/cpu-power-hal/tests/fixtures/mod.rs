//! Tempdir-based sysfs fixture builders modeling the actual homelab fleet:
//! mixed-generation Intel NUCs (some with EPP, some without), the AMD
//! Framework Desktop (Strix Halo, amd-pstate-epp), and a QEMU VM with no
//! cpufreq exposed to the guest at all.
//!
//! Each `tests/*.rs` file compiles this module separately and only uses a
//! subset of the builders below, so per-binary dead-code warnings are
//! expected and silenced here rather than worked around.
#![allow(dead_code)]

use std::fs;
use std::path::Path;
use std::sync::Arc;

use cpu_power_hal::{RootedSysfs, SysfsIo};
use tempfile::TempDir;

const CPUFREQ: &str = "sys/devices/system/cpu/cpufreq";

fn write_policy(root: &Path, id: u32, files: &[(&str, &str)]) {
    let policy_dir = root.join(CPUFREQ).join(format!("policy{id}"));
    fs::create_dir_all(&policy_dir).unwrap();
    for (name, contents) in files {
        fs::write(policy_dir.join(name), contents).unwrap();
    }
}

/// A newer NUC generation (e.g. Alder Lake/Raptor Lake) running
/// `intel_pstate` in active/HWP mode with full EPP support.
pub fn intel_epp_nuc() -> (TempDir, Arc<dyn SysfsIo>) {
    let dir = tempfile::tempdir().unwrap();
    for id in 0..8u32 {
        write_policy(
            dir.path(),
            id,
            &[
                ("scaling_driver", "intel_pstate"),
                ("energy_performance_preference", "balance_performance"),
                (
                    "energy_performance_available_preferences",
                    "default performance balance_performance balance_power power",
                ),
                ("scaling_governor", "powersave"),
                ("scaling_available_governors", "performance powersave"),
                ("related_cpus", &id.to_string()),
            ],
        );
    }
    fs::create_dir_all(dir.path().join("sys/devices/system/cpu/intel_pstate")).unwrap();
    fs::write(
        dir.path()
            .join("sys/devices/system/cpu/intel_pstate/no_turbo"),
        "0",
    )
    .unwrap();
    let io: Arc<dyn SysfsIo> = Arc::new(RootedSysfs::new(dir.path()));
    (dir, io)
}

/// An older NUC generation: `intel_pstate` in passive mode / lacking HWP,
/// so no `energy_performance_preference` file exists at all — only the
/// generic `scaling_governor` interface.
pub fn intel_governor_only_nuc() -> (TempDir, Arc<dyn SysfsIo>) {
    let dir = tempfile::tempdir().unwrap();
    for id in 0..4u32 {
        write_policy(
            dir.path(),
            id,
            &[
                ("scaling_driver", "intel_pstate"),
                ("scaling_governor", "powersave"),
                (
                    "scaling_available_governors",
                    "performance powersave ondemand",
                ),
                ("related_cpus", &id.to_string()),
            ],
        );
    }
    let io: Arc<dyn SysfsIo> = Arc::new(RootedSysfs::new(dir.path()));
    (dir, io)
}

/// The AMD Framework Desktop (Strix Halo, Zen 5) running `amd-pstate-epp`
/// on a modern (6.x+) kernel — same EPP file shape as Intel, plus an
/// AMD-specific boost toggle.
pub fn amd_epp_strix_halo() -> (TempDir, Arc<dyn SysfsIo>) {
    let dir = tempfile::tempdir().unwrap();
    for id in 0..16u32 {
        write_policy(
            dir.path(),
            id,
            &[
                ("scaling_driver", "amd-pstate-epp"),
                ("energy_performance_preference", "performance"),
                (
                    "energy_performance_available_preferences",
                    "default performance balance_performance balance_power power",
                ),
                ("scaling_governor", "powersave"),
                ("scaling_available_governors", "performance powersave"),
                ("related_cpus", &id.to_string()),
            ],
        );
    }
    fs::create_dir_all(dir.path().join("sys/devices/system/cpu/amd_pstate")).unwrap();
    fs::write(
        dir.path().join("sys/devices/system/cpu/amd_pstate/boost"),
        "1",
    )
    .unwrap();
    let io: Arc<dyn SysfsIo> = Arc::new(RootedSysfs::new(dir.path()));
    (dir, io)
}

/// The QEMU VM: no cpufreq policies exposed to the guest at all (the
/// common case without host CPU passthrough).
pub fn unsupported_qemu() -> (TempDir, Arc<dyn SysfsIo>) {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("sys/devices/system/cpu")).unwrap();
    let io: Arc<dyn SysfsIo> = Arc::new(RootedSysfs::new(dir.path()));
    (dir, io)
}
