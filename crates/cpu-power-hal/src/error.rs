use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum PowerError {
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("'{0}' is not a recognized power profile")]
    InvalidProfile(String),

    #[error(
        "profile '{profile}' is not among the values advertised by cpufreq policy{policy}: {available:?}"
    )]
    UnsupportedProfile {
        policy: u32,
        profile: String,
        available: Vec<String>,
    },

    #[error("turbo boost control is not supported by this backend")]
    TurboUnsupported,
}

pub type Result<T> = std::result::Result<T, PowerError>;
