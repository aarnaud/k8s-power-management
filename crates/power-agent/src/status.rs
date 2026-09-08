use cpu_power_hal::BackendKind;
use k8s_openapi::api::core::v1::Node;
use kube::Resource;
use kube::api::{Api, Patch, PatchParams};
use kube::runtime::events::{Event as K8sEvent, EventType, Recorder};
use serde_json::{Map, Value, json};

use crate::reconcile::ReconcileReport;

const ANNOTATION_PREFIX: &str = "cpu-power.io";
/// Shared `action` field for every event this agent emits, so they group
/// naturally under one `kubectl describe node` heading.
const RECONCILE_ACTION: &str = "ReconcilePowerState";

/// Patches this node's status annotations via a JSON merge patch. Only
/// touches `.metadata.annotations`, never `.status`, so this needs only
/// plain `nodes: patch` RBAC, not `nodes/status`.
pub async fn patch_status(
    nodes: &Api<Node>,
    node_name: &str,
    report: &ReconcileReport,
) -> Result<(), kube::Error> {
    let applied_turbo = match report.turbo {
        Some(true) => "enabled",
        Some(false) => "disabled",
        None if report.turbo_unsupported => "unsupported",
        None => "unmanaged",
    };

    let mut annotations = Map::new();
    annotations.insert(
        format!("{ANNOTATION_PREFIX}/applied-profile"),
        Value::String(report.profile.as_kernel_str().to_string()),
    );
    annotations.insert(
        format!("{ANNOTATION_PREFIX}/applied-turbo"),
        Value::String(applied_turbo.to_string()),
    );
    annotations.insert(
        format!("{ANNOTATION_PREFIX}/backend"),
        Value::String(report.backend_kind.to_string()),
    );
    annotations.insert(
        format!("{ANNOTATION_PREFIX}/status"),
        Value::String(report.result_label().to_string()),
    );
    annotations.insert(
        format!("{ANNOTATION_PREFIX}/last-reconcile-time"),
        Value::String(now_rfc3339()),
    );

    let patch = json!({
        "metadata": {
            "annotations": Value::Object(annotations)
        }
    });

    nodes
        .patch(node_name, &PatchParams::default(), &Patch::Merge(&patch))
        .await?;
    Ok(())
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

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}
