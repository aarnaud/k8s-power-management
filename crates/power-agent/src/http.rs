use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;

use crate::metrics::Metrics;

#[derive(Clone)]
pub struct HttpState {
    pub metrics: Arc<Metrics>,
    pub last_reconcile_unix: Arc<AtomicI64>,
    /// `/healthz` fails once this much time has passed since the last
    /// completed reconcile — catches a stuck watch stream rather than
    /// reporting healthy forever off a single early success.
    pub staleness_threshold: Duration,
}

pub fn router(state: HttpState) -> Router {
    Router::new()
        .route("/metrics", get(metrics_handler))
        .route("/healthz", get(healthz_handler))
        .with_state(state)
}

async fn metrics_handler(State(state): State<HttpState>) -> impl IntoResponse {
    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4")],
        state.metrics.encode(),
    )
}

async fn healthz_handler(State(state): State<HttpState>) -> impl IntoResponse {
    let last = state.last_reconcile_unix.load(Ordering::Relaxed);
    if last == 0 {
        // No reconcile has completed yet; still starting up rather than
        // unhealthy.
        return (StatusCode::OK, "starting").into_response();
    }

    let age = now_unix().saturating_sub(last);
    if age as u64 > state.staleness_threshold.as_secs() {
        return (StatusCode::SERVICE_UNAVAILABLE, "stale").into_response();
    }
    (StatusCode::OK, "ok").into_response()
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
