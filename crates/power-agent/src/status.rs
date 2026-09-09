use std::collections::BTreeMap;

use cpu_power_hal::BackendKind;
use k8s_openapi::api::core::v1::Node;
use kube::api::{Api, Patch, PatchParams};
use kube::runtime::events::{Event as K8sEvent, EventType, Recorder};
use kube::{Resource, ResourceExt};
use serde_json::{Map, Value, json};

use crate::reconcile::ReconcileReport;

const ANNOTATION_PREFIX: &str = "cpu-power.io";
/// Shared `action` field for every event this agent emits, so they group
/// naturally under one `kubectl describe node` heading.
const RECONCILE_ACTION: &str = "ReconcilePowerState";

/// Patches this node's status annotations via a JSON merge patch, but only
/// when they actually differ from what's already on the Node.
///
/// This is deliberately idempotent and deliberately excludes anything that
/// would change on every call (e.g. a timestamp): this agent watches its
/// own Node, so a patch that always changes something would always bump
/// `resourceVersion`, which would immediately re-trigger the watch stream
/// and reconcile again -- a self-inflicted, zero-delay hot loop. "When did
/// this last run" is answered by the `power_agent_last_reconcile_timestamp_seconds`
/// metric instead, precisely because metrics are pull-based and never get
/// written back onto the object we're watching. Only touches
/// `.metadata.annotations`, never `.status`, so this needs only plain
/// `nodes: patch` RBAC, not `nodes/status`.
pub async fn patch_status(
    nodes: &Api<Node>,
    node: &Node,
    report: &ReconcileReport,
) -> Result<(), kube::Error> {
    let desired = desired_annotations(report);
    if !needs_patch(node.annotations(), &desired) {
        return Ok(());
    }

    let mut annotations = Map::new();
    for (key, value) in desired {
        annotations.insert(key, Value::String(value));
    }
    let patch = json!({
        "metadata": {
            "annotations": Value::Object(annotations)
        }
    });

    nodes
        .patch(
            &node.name_any(),
            &PatchParams::default(),
            &Patch::Merge(&patch),
        )
        .await?;
    Ok(())
}

/// The full (prefixed) annotation keys/values this reconcile wants on the
/// Node. Deliberately excludes anything that would change on every call
/// (e.g. a timestamp) -- see `needs_patch`.
fn desired_annotations(report: &ReconcileReport) -> BTreeMap<String, String> {
    let applied_turbo = match report.turbo {
        Some(true) => "enabled",
        Some(false) => "disabled",
        None if report.turbo_unsupported => "unsupported",
        None => "unmanaged",
    };

    BTreeMap::from([
        (
            format!("{ANNOTATION_PREFIX}/applied-profile"),
            report.profile.as_kernel_str().to_string(),
        ),
        (
            format!("{ANNOTATION_PREFIX}/applied-turbo"),
            applied_turbo.to_string(),
        ),
        (
            format!("{ANNOTATION_PREFIX}/backend"),
            report.backend_kind.to_string(),
        ),
        (
            format!("{ANNOTATION_PREFIX}/status"),
            report.result_label().to_string(),
        ),
    ])
}

/// Whether `current` (the Node's existing annotations) actually differs
/// from `desired`. This is the loop-prevention check: this agent watches
/// its own Node, so patching every reconcile regardless of content would
/// bump `resourceVersion` every time, which would immediately re-trigger
/// the watch stream and reconcile again -- a self-inflicted, zero-delay
/// hot loop. Skipping the patch once `desired` is already reflected is
/// what lets a run converge instead of patching forever.
fn needs_patch(current: &BTreeMap<String, String>, desired: &BTreeMap<String, String>) -> bool {
    desired
        .iter()
        .any(|(key, value)| current.get(key) != Some(value))
}

/// Emits Kubernetes Events on the Node object reflecting this reconcile's
/// outcome. The `Recorder` dedupes identical (reason, note, ...) events
/// into a series rather than spamming new objects, so it's safe to call
/// this on every reconcile including the periodic resync.
pub async fn emit_events(recorder: &Recorder, node: &Node, report: &ReconcileReport) {
    let reference = node.object_ref(&());

    if let Some(invalid) = &report.profile_invalid_value {
        publish(
            recorder,
            &reference,
            EventType::Warning,
            "InvalidProfileLabel",
            format!(
                "'{invalid}' is not a recognized cpu-power.io/profile value; falling back to default"
            ),
        )
        .await;
    }

    if let Some(invalid) = &report.turbo_invalid_value {
        publish(
            recorder,
            &reference,
            EventType::Warning,
            "InvalidTurboLabel",
            format!(
                "'{invalid}' is not a valid cpu-power.io/turbo value (expected enabled/disabled); turbo left unmanaged"
            ),
        )
        .await;
    }

    if let Some(err) = &report.profile_apply_error {
        publish(
            recorder,
            &reference,
            EventType::Warning,
            "ApplyFailed",
            format!("failed to apply power profile {}: {err}", report.profile),
        )
        .await;
    } else if report.backend_kind == BackendKind::Unsupported {
        publish(
            recorder,
            &reference,
            EventType::Normal,
            "ProfileUnsupported",
            "no cpufreq policies detected on this node; power management is a no-op here"
                .to_string(),
        )
        .await;
    } else {
        publish(
            recorder,
            &reference,
            EventType::Normal,
            "ProfileApplied",
            format!("applied power profile {}", report.profile),
        )
        .await;
    }

    if let Some(err) = &report.turbo_apply_error {
        publish(
            recorder,
            &reference,
            EventType::Warning,
            "ApplyFailed",
            format!("failed to apply turbo state: {err}"),
        )
        .await;
    } else if let Some(enabled) = report.turbo {
        publish(
            recorder,
            &reference,
            EventType::Normal,
            "TurboApplied",
            format!(
                "turbo boost set to {}",
                if enabled { "enabled" } else { "disabled" }
            ),
        )
        .await;
    }
}

async fn publish(
    recorder: &Recorder,
    reference: &k8s_openapi::api::core::v1::ObjectReference,
    type_: EventType,
    reason: &str,
    note: String,
) {
    let event = K8sEvent {
        type_,
        reason: reason.to_string(),
        note: Some(note),
        action: RECONCILE_ACTION.to_string(),
        secondary: None,
    };
    if let Err(e) = recorder.publish(&event, reference).await {
        tracing::warn!(error = %e, reason, "failed to publish Kubernetes event");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cpu_power_hal::{BackendKind, PowerProfile};

    fn ok_report() -> ReconcileReport {
        ReconcileReport {
            profile: PowerProfile::Power,
            profile_invalid_value: None,
            profile_apply_error: None,
            turbo: Some(false),
            turbo_invalid_value: None,
            turbo_apply_error: None,
            turbo_unsupported: false,
            backend_kind: BackendKind::IntelEpp,
        }
    }

    #[test]
    fn needs_patch_when_node_has_no_annotations_yet() {
        let desired = desired_annotations(&ok_report());
        assert!(needs_patch(&BTreeMap::new(), &desired));
    }

    #[test]
    fn no_patch_needed_once_annotations_already_match() {
        let desired = desired_annotations(&ok_report());
        // Simulates the watch event the agent's own prior patch produces:
        // the Node it's handed back already carries exactly what it wants.
        assert!(!needs_patch(&desired, &desired));
    }

    #[test]
    fn needs_patch_when_one_value_differs() {
        let desired = desired_annotations(&ok_report());
        let mut stale = desired.clone();
        stale.insert(
            format!("{ANNOTATION_PREFIX}/applied-profile"),
            "performance".to_string(),
        );
        assert!(needs_patch(&stale, &desired));
    }

    #[test]
    fn desired_annotations_never_includes_a_timestamp() {
        // Regression test for the self-triggered reconcile loop: any field
        // that changes on every call (like a timestamp) defeats
        // needs_patch's convergence, since the patch would never stop
        // differing from itself. Every key/value here must be fully
        // determined by ReconcileReport alone.
        let a = desired_annotations(&ok_report());
        let b = desired_annotations(&ok_report());
        assert_eq!(a, b);
    }
}
