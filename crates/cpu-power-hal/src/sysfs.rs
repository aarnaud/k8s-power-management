use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The filesystem I/O boundary every backend goes through instead of
/// calling `std::fs` directly. This is what makes the whole crate testable
/// without real hardware: `RootedSysfs` is the only implementation, used
/// both in production (rooted at `/`) and in tests (rooted at a tempdir
/// built to mimic a real sysfs tree), so discovery/apply logic is exercised
/// against real filesystem semantics in both cases.
pub trait SysfsIo: Send + Sync {
    fn read_to_string(&self, path: &Path) -> io::Result<String>;
    fn write(&self, path: &Path, contents: &str) -> io::Result<()>;
    fn read_dir(&self, path: &Path) -> io::Result<Vec<PathBuf>>;
    fn exists(&self, path: &Path) -> bool;
}

#[derive(Debug, Clone)]
pub struct RootedSysfs {
    root: PathBuf,
}

impl RootedSysfs {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Rooted at `/`: paths are used as-is against the real host filesystem.
    pub fn host() -> Self {
        Self::new("/")
    }

    /// Resolve a sysfs-absolute path (e.g. `/sys/devices/system/cpu/...`)
    /// onto this instance's root, so callers never need to know whether
    /// they're pointed at `/` or a test fixture directory.
    fn resolve(&self, path: &Path) -> PathBuf {
        match path.strip_prefix("/") {
            Ok(rel) => self.root.join(rel),
            Err(_) => self.root.join(path),
        }
    }
}

impl SysfsIo for RootedSysfs {
    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        let contents = fs::read_to_string(self.resolve(path))?;
        Ok(contents.trim_end_matches('\n').to_string())
    }

    fn write(&self, path: &Path, contents: &str) -> io::Result<()> {
        fs::write(self.resolve(path), contents)
    }

    fn read_dir(&self, path: &Path) -> io::Result<Vec<PathBuf>> {
        let mut entries = Vec::new();
        for entry in fs::read_dir(self.resolve(path))? {
            let entry = entry?;
            // Return paths back in the caller's (sysfs-relative) namespace,
            // not resolved ones, so callers can keep round-tripping them
            // through this same SysfsIo without ever seeing `root`.
            entries.push(path.join(entry.file_name()));
        }
        entries.sort();
        Ok(entries)
    }

    fn exists(&self, path: &Path) -> bool {
        self.resolve(path).exists()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_write_and_read() {
        let dir = tempfile::tempdir().unwrap();
        let io = RootedSysfs::new(dir.path());
        let path =
            Path::new("/sys/devices/system/cpu/cpufreq/policy0/energy_performance_preference");
        fs::create_dir_all(dir.path().join("sys/devices/system/cpu/cpufreq/policy0")).unwrap();

        io.write(path, "performance").unwrap();
        assert_eq!(io.read_to_string(path).unwrap(), "performance");
        assert!(io.exists(path));
    }

    #[test]
    fn missing_path_is_not_exists_not_panic() {
        let dir = tempfile::tempdir().unwrap();
        let io = RootedSysfs::new(dir.path());
        assert!(!io.exists(Path::new("/sys/devices/system/cpu/cpufreq")));
    }

    #[test]
    fn read_dir_returns_sysfs_relative_paths() {
        let dir = tempfile::tempdir().unwrap();
        let io = RootedSysfs::new(dir.path());
        fs::create_dir_all(dir.path().join("sys/devices/system/cpu/cpufreq/policy0")).unwrap();
        fs::create_dir_all(dir.path().join("sys/devices/system/cpu/cpufreq/policy1")).unwrap();

        let entries = io
            .read_dir(Path::new("/sys/devices/system/cpu/cpufreq"))
            .unwrap();
        assert_eq!(
            entries,
            vec![
                PathBuf::from("/sys/devices/system/cpu/cpufreq/policy0"),
                PathBuf::from("/sys/devices/system/cpu/cpufreq/policy1"),
            ]
        );
    }
}
