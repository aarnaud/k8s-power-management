use std::env;
use std::time::Duration;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("required environment variable {0} is not set (expected via the pod's Downward API)")]
    MissingEnv(&'static str),
    #[error("environment variable {0}={1:?} is not valid: {2}")]
    InvalidEnv(&'static str, String, String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    Pretty,
    Json,
}

#[derive(Debug, Clone)]
pub struct Config {
    /// This node's name, injected via the Downward API (`spec.nodeName`).
    /// The whole design depends on it, so a missing value is a startup
    /// failure, not a silent fallback.
    pub node_name: String,
    pub profile_label_key: String,
    pub turbo_label_key: String,
    pub resync_interval: Duration,
    pub metrics_addr: String,
    pub log_format: LogFormat,
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        let node_name = env::var("NODE_NAME").map_err(|_| ConfigError::MissingEnv("NODE_NAME"))?;

        let profile_label_key =
            env::var("PROFILE_LABEL_KEY").unwrap_or_else(|_| "cpu-power.io/profile".to_string());
        let turbo_label_key =
            env::var("TURBO_LABEL_KEY").unwrap_or_else(|_| "cpu-power.io/turbo".to_string());

        let resync_interval_secs: u64 = match env::var("RESYNC_INTERVAL_SECONDS") {
            Ok(v) => v.parse().map_err(|e: std::num::ParseIntError| {
                ConfigError::InvalidEnv("RESYNC_INTERVAL_SECONDS", v, e.to_string())
            })?,
            Err(_) => 300,
        };

        let metrics_addr = env::var("METRICS_ADDR").unwrap_or_else(|_| "0.0.0.0:9090".to_string());

        let log_format = match env::var("LOG_FORMAT").ok().as_deref() {
            Some("json") => LogFormat::Json,
            _ => LogFormat::Pretty,
        };

        Ok(Config {
            node_name,
            profile_label_key,
            turbo_label_key,
            resync_interval: Duration::from_secs(resync_interval_secs),
            metrics_addr,
            log_format,
        })
    }
}
