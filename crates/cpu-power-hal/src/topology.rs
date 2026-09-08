use std::path::{Path, PathBuf};

use crate::error::PowerError;
use crate::sysfs::SysfsIo;

pub const CPUFREQ_ROOT: &str = "/sys/devices/system/cpu/cpufreq";

/// One `cpufreq` scaling policy (`/sys/devices/system/cpu/cpufreq/policyN`).
///
/// Deliberately modeled per-policy rather than per-logical-CPU: some
/// drivers group multiple logical CPUs under one policy object, and
/// `cpuN/cpufreq/*` is just a symlink back to the owning `policyN`
/// directory. Iterating policies avoids redundant/duplicate writes.
#[derive(Debug, Clone)]
pub struct CpuPolicy {
    pub id: u32,
    pub path: PathBuf,
    pub related_cpus: Vec<u32>,
    pub scaling_driver: String,
    /// Capability is gated on file *presence*, not on matching the driver
    /// name against a known list — that's the reliable signal, since the
    /// kernel itself only creates this file when the driver registered EPP
    /// support, whereas driver-name strings vary across kernel versions.
    pub has_epp: bool,
    pub available_epp: Vec<String>,
    pub available_governors: Vec<String>,
}

/// Enumerate all cpufreq policies visible in sysfs.
///
/// An empty result (no `cpufreq` directory, or one with zero `policyN`
/// subdirectories) is a valid, expected outcome — e.g. inside a QEMU guest
/// without host CPU passthrough — and must not be treated as an error; it's
/// the signal that feeds `detect_backend()` into the `Unsupported` backend.
pub fn discover_policies(io: &dyn SysfsIo) -> Result<Vec<CpuPolicy>, PowerError> {
    let root = Path::new(CPUFREQ_ROOT);
    if !io.exists(root) {
        return Ok(Vec::new());
    }

    let mut policies = Vec::new();
    let entries = io.read_dir(root).map_err(|source| PowerError::Io {
        path: root.to_path_buf(),
        source,
    })?;

    for entry in entries {
        let Some(name) = entry.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(id_str) = name.strip_prefix("policy") else {
            continue;
        };
        let Ok(id) = id_str.parse::<u32>() else {
            continue;
        };

        let scaling_driver = read_trimmed(io, &entry.join("scaling_driver")).unwrap_or_default();
        let related_cpus = read_trimmed(io, &entry.join("related_cpus"))
            .map(|s| parse_cpu_list(&s))
            .unwrap_or_default();

        let epp_path = entry.join("energy_performance_preference");
        let has_epp = io.exists(&epp_path);
        let available_epp = if has_epp {
            read_trimmed(io, &entry.join("energy_performance_available_preferences"))
                .map(|s| s.split_whitespace().map(str::to_string).collect())
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        let available_governors = read_trimmed(io, &entry.join("scaling_available_governors"))
            .map(|s| s.split_whitespace().map(str::to_string).collect())
            .unwrap_or_default();

        policies.push(CpuPolicy {
            id,
            path: entry,
            related_cpus,
            scaling_driver,
            has_epp,
            available_epp,
            available_governors,
        });
    }

    policies.sort_by_key(|p| p.id);
    Ok(policies)
}

fn read_trimmed(io: &dyn SysfsIo, path: &Path) -> Option<String> {
    io.read_to_string(path).ok()
}

fn parse_cpu_list(s: &str) -> Vec<u32> {
    s.split_whitespace()
        .filter_map(|tok| tok.parse().ok())
        .collect()
}
