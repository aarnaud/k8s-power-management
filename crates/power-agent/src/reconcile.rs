use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use cpu_power_hal::{BackendKind, PowerBackend, PowerError, PowerProfile};
use futures::{StreamExt, TryStreamExt};
use k8s_openapi::api::core::v1::Node;
use kube::runtime::events::{Recorder, Reporter};
use kube::runtime::{WatchStreamExt, watcher};
use kube::{Api, Client, ResourceExt};
use tracing::{info, warn};

use crate::config::Config;
use crate::metrics::Metrics;
use crate::status;

/// Outcome of applying the desired state for one Node object, independent
/// of *how* that state was discovered (watch event vs. periodic resync).
/// Both entry points converge on [`reconcile_once`] so there's a single
/// source of truth for "what do we do given these label values."
#[derive(Debug)]
pub struct ReconcileReport {
    pub profile: PowerProfile,
    pub profile_invalid_value: Option<String>,
    pub profile_apply_error: Option<String>,
    pub turbo: Option<bool>,
    pub turbo_invalid_value: Option<String>,
    pub turbo_apply_error: Option<String>,
    pub turbo_unsupported: bool,
    pub backend_kind: BackendKind,
}

impl ReconcileReport {
    /// A single coarse status label shared by metrics and Node annotations,
    /// in priority order: a genuine apply error outranks an invalid label,
    /// which outranks "this node just doesn't support power management."
    pub fn result_label(&self) -> &'static str {
        if self.profile_apply_error.is_some() || self.turbo_apply_error.is_some() {
            "error"
        } else if self.profile_invalid_value.is_some() || self.turbo_invalid_value.is_some() {
            "invalid_label"
        } else if self.backend_kind == BackendKind::Unsupported {
            "unsupported"
        } else {
            "ok"
        }
    }
}

fn parse_profile_label(raw: Option<&str>) -> (PowerProfile, Option<String>) {
    match raw {
        None => (PowerProfile::Default, None),
        Some(s) => match s.parse::<PowerProfile>() {
            Ok(p) => (p, None),
            Err(_) => (PowerProfile::Default, Some(s.to_string())),
        },
    }
}

/// `None` means "label absent or invalid, don't touch turbo at all" — there
/// is no natural default turbo state to force the way `Default` is a
/// natural fallback profile.
fn parse_turbo_label(raw: Option<&str>) -> (Option<bool>, Option<String>) {
    match raw {
        None => (None, None),
        Some("enabled") => (Some(true), None),
        Some("disabled") => (Some(false), None),
        Some(other) => (None, Some(other.to_string())),
    }
}

/// The single source of truth for "what do we do given these label
/// values" — deliberately synchronous and free of any Kubernetes-client
/// calls, so it's unit-testable against a fake [`PowerBackend`] without a
/// cluster (see the `tests` module below).
pub fn reconcile_once(
    backend: &dyn PowerBackend,
    profile_label: Option<&str>,
    turbo_label: Option<&str>,
) -> ReconcileReport {
    let (profile, profile_invalid_value) = parse_profile_label(profile_label);
    let (turbo, turbo_invalid_value) = parse_turbo_label(turbo_label);

    let profile_apply_error = backend.apply(profile).err().map(|e| e.to_string());

    let mut turbo_apply_error = None;
    let mut turbo_unsupported = false;
    if let Some(enabled) = turbo {
        match backend.set_turbo(enabled) {
            Ok(()) => {}
            Err(PowerError::TurboUnsupported) => turbo_unsupported = true,
            Err(e) => turbo_apply_error = Some(e.to_string()),
        }
    }

    ReconcileReport {
        profile,
        profile_invalid_value,
        profile_apply_error,
        turbo,
        turbo_invalid_value,
        turbo_apply_error,
        turbo_unsupported,
        backend_kind: backend.kind(),
    }
}

pub struct Runtime {
    pub client: Client,
    pub config: Config,
    pub backend: Arc<dyn PowerBackend>,
    pub metrics: Arc<Metrics>,
    pub last_reconcile_unix: Arc<AtomicI64>,
}

impl Runtime {
    async fn reconcile_node(&self, node: &Node, recorder: &Recorder) {
        let labels = node.labels();
        let profile_label = labels
            .get(self.config.profile_label_key.as_str())
            .map(String::as_str);
        let turbo_label = labels
            .get(self.config.turbo_label_key.as_str())
            .map(String::as_str);

        let report = reconcile_once(self.backend.as_ref(), profile_label, turbo_label);

        info!(
            profile = %report.profile,
            backend = %report.backend_kind,
            turbo = ?report.turbo,
            result = report.result_label(),
            "reconciled node power state"
        );

        self.metrics.observe(&report);
        self.last_reconcile_unix
            .store(now_unix(), Ordering::Relaxed);

        let nodes: Api<Node> = Api::all(self.client.clone());
        if let Err(e) = status::patch_status(&nodes, node, &report).await {
            warn!(error = %e, "failed to patch node status annotations");
        }
        status::emit_events(recorder, node, &report).await;
    }

    pub async fn run(self) -> anyhow::Result<()> {
        let nodes: Api<Node> = Api::all(self.client.clone());
        let watch_config =
            watcher::Config::default().fields(&format!("metadata.name={}", self.config.node_name));
        let mut stream = watcher(nodes.clone(), watch_config)
            .default_backoff()
            .applied_objects()
            .boxed();

        let reporter = Reporter {
            controller: "power-agent".into(),
            instance: Some(self.config.node_name.clone()),
        };
        let recorder = Recorder::new(self.client.clone(), reporter);

        let mut resync = tokio::time::interval(self.config.resync_interval);
        resync.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // The first tick fires immediately; skip it so startup doesn't
        // double-reconcile alongside the watch stream's own initial event.
        resync.tick().await;

        loop {
            tokio::select! {
                next = stream.try_next() => {
                    match next {
                        Ok(Some(node)) => self.reconcile_node(&node, &recorder).await,
                        Ok(None) => {
                            warn!("watch stream ended unexpectedly, exiting");
                            break;
                        }
                        Err(e) => warn!(error = %e, "watch stream error, backoff will retry"),
                    }
                }
                _ = resync.tick() => {
                    match nodes.get_opt(&self.config.node_name).await {
                        Ok(Some(node)) => self.reconcile_node(&node, &recorder).await,
                        Ok(None) => warn!(node = %self.config.node_name, "own node not found during periodic resync"),
                        Err(e) => warn!(error = %e, "failed to fetch own node during periodic resync"),
                    }
                }
            }
        }

        Ok(())
    }
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cpu_power_hal::BackendKind;
    use std::sync::Mutex;

    /// A fake backend recording every `apply`/`set_turbo` call, so
    /// `reconcile_once` can be tested without touching real sysfs or the
    /// full `cpu-power-hal` fixture machinery. Uses `Mutex` rather than
    /// `RefCell` because `PowerBackend: Send + Sync`.
    struct RecordingBackend {
        applied: Mutex<Vec<PowerProfile>>,
        turbo_calls: Mutex<Vec<bool>>,
        turbo_unsupported: bool,
        kind: BackendKind,
    }

    impl PowerBackend for RecordingBackend {
        fn kind(&self) -> BackendKind {
            self.kind
        }
        fn is_supported(&self) -> bool {
            true
        }
        fn apply(&self, profile: PowerProfile) -> Result<(), PowerError> {
            self.applied.lock().unwrap().push(profile);
            Ok(())
        }
        fn current(&self) -> Result<Option<PowerProfile>, PowerError> {
            Ok(self.applied.lock().unwrap().last().copied())
        }
        fn set_turbo(&self, enabled: bool) -> Result<(), PowerError> {
            self.turbo_calls.lock().unwrap().push(enabled);
            if self.turbo_unsupported {
                Err(PowerError::TurboUnsupported)
            } else {
                Ok(())
            }
        }
    }

    fn backend(kind: BackendKind) -> RecordingBackend {
        RecordingBackend {
            applied: Mutex::new(Vec::new()),
            turbo_calls: Mutex::new(Vec::new()),
            turbo_unsupported: false,
            kind,
        }
    }

    #[test]
    fn absent_profile_label_falls_back_to_default() {
        let b = backend(BackendKind::IntelEpp);
        let report = reconcile_once(&b, None, None);
        assert_eq!(report.profile, PowerProfile::Default);
        assert!(report.profile_invalid_value.is_none());
        assert_eq!(
            b.applied.lock().unwrap().as_slice(),
            [PowerProfile::Default]
        );
    }

    #[test]
    fn invalid_profile_label_falls_back_to_default_and_is_flagged() {
        let b = backend(BackendKind::IntelEpp);
        let report = reconcile_once(&b, Some("ultra"), None);
        assert_eq!(report.profile, PowerProfile::Default);
        assert_eq!(report.profile_invalid_value.as_deref(), Some("ultra"));
        assert_eq!(report.result_label(), "invalid_label");
    }

    #[test]
    fn valid_profile_label_is_applied_verbatim() {
        let b = backend(BackendKind::AmdEpp);
        let report = reconcile_once(&b, Some("power"), None);
        assert_eq!(report.profile, PowerProfile::Power);
        assert_eq!(report.result_label(), "ok");
    }

    #[test]
    fn absent_turbo_label_never_calls_set_turbo() {
        let b = backend(BackendKind::IntelEpp);
        let report = reconcile_once(&b, Some("performance"), None);
        assert_eq!(report.turbo, None);
        assert!(b.turbo_calls.lock().unwrap().is_empty());
    }

    #[test]
    fn invalid_turbo_label_is_flagged_and_not_applied() {
        let b = backend(BackendKind::IntelEpp);
        let report = reconcile_once(&b, None, Some("maybe"));
        assert_eq!(report.turbo, None);
        assert_eq!(report.turbo_invalid_value.as_deref(), Some("maybe"));
        assert!(b.turbo_calls.lock().unwrap().is_empty());
    }

    #[test]
    fn valid_turbo_label_is_applied() {
        let b = backend(BackendKind::AmdEpp);
        let report = reconcile_once(&b, None, Some("enabled"));
        assert_eq!(report.turbo, Some(true));
        assert_eq!(b.turbo_calls.lock().unwrap().as_slice(), [true]);
    }

    #[test]
    fn primary_target_scenario_nuc_power_turbo_disabled() {
        let b = backend(BackendKind::IntelEpp);
        let report = reconcile_once(&b, Some("power"), Some("disabled"));
        assert_eq!(report.profile, PowerProfile::Power);
        assert_eq!(report.turbo, Some(false));
        assert_eq!(report.result_label(), "ok");
    }

    #[test]
    fn primary_target_scenario_amd_performance_turbo_enabled() {
        let b = backend(BackendKind::AmdEpp);
        let report = reconcile_once(&b, Some("performance"), Some("enabled"));
        assert_eq!(report.profile, PowerProfile::Performance);
        assert_eq!(report.turbo, Some(true));
        assert_eq!(report.result_label(), "ok");
    }

    #[test]
    fn turbo_unsupported_on_backend_is_reported_not_an_error() {
        let mut b = backend(BackendKind::GovernorFallback);
        b.turbo_unsupported = true;
        let report = reconcile_once(&b, None, Some("enabled"));
        assert!(report.turbo_unsupported);
        assert!(report.turbo_apply_error.is_none());
        assert_eq!(report.result_label(), "ok");
    }

    #[test]
    fn unsupported_backend_never_reports_error() {
        let b = backend(BackendKind::Unsupported);
        let report = reconcile_once(&b, Some("power"), Some("disabled"));
        assert_eq!(report.result_label(), "unsupported");
    }
}
