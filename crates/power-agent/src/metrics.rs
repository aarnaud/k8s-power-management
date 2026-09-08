use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use cpu_power_hal::{BackendKind, PowerProfile};
use prometheus::{Encoder, GaugeVec, IntCounterVec, IntGauge, Opts, Registry, TextEncoder};

use crate::reconcile::ReconcileReport;

const ALL_BACKEND_KINDS: [BackendKind; 5] = [
    BackendKind::IntelEpp,
    BackendKind::AmdEpp,
    BackendKind::UnknownEpp,
    BackendKind::GovernorFallback,
    BackendKind::Unsupported,
];

pub struct Metrics {
    registry: Registry,
    profile_applied: GaugeVec,
    backend_kind: GaugeVec,
    turbo_enabled: IntGauge,
    turbo_managed: IntGauge,
    reconcile_total: IntCounterVec,
    last_reconcile_timestamp: IntGauge,
}

impl Metrics {
    pub fn new() -> anyhow::Result<Arc<Self>> {
        let registry = Registry::new();

        let profile_applied = GaugeVec::new(
            Opts::new(
                "power_agent_profile_applied_info",
                "Currently applied CPU power profile (1 = active, 0 = inactive)",
            ),
            &["profile"],
        )?;
        let backend_kind = GaugeVec::new(
            Opts::new(
                "power_agent_backend_kind_info",
                "Detected cpu-power-hal backend for this node (1 = active, 0 = inactive)",
            ),
            &["kind"],
        )?;
        let turbo_enabled = IntGauge::new(
            "power_agent_turbo_enabled",
            "1 if turbo boost was last set enabled, 0 if disabled (meaningless if turbo_managed is 0)",
        )?;
        let turbo_managed = IntGauge::new(
            "power_agent_turbo_managed",
            "1 if the turbo label is present, valid, and being actively managed on this node",
        )?;
        let reconcile_total = IntCounterVec::new(
            Opts::new(
                "power_agent_reconcile_total",
                "Total reconcile attempts by result",
            ),
            &["result"],
        )?;
        let last_reconcile_timestamp = IntGauge::new(
            "power_agent_last_reconcile_timestamp_seconds",
            "Unix timestamp of the last completed reconcile",
        )?;

        registry.register(Box::new(profile_applied.clone()))?;
        registry.register(Box::new(backend_kind.clone()))?;
        registry.register(Box::new(turbo_enabled.clone()))?;
        registry.register(Box::new(turbo_managed.clone()))?;
        registry.register(Box::new(reconcile_total.clone()))?;
        registry.register(Box::new(last_reconcile_timestamp.clone()))?;

        Ok(Arc::new(Self {
            registry,
            profile_applied,
            backend_kind,
            turbo_enabled,
            turbo_managed,
            reconcile_total,
            last_reconcile_timestamp,
        }))
    }

    pub fn observe(&self, report: &ReconcileReport) {
        for profile in PowerProfile::ALL {
            let value = if profile == report.profile { 1.0 } else { 0.0 };
            self.profile_applied
                .with_label_values(&[profile.as_kernel_str()])
                .set(value);
        }

        for kind in ALL_BACKEND_KINDS {
            let value = if kind == report.backend_kind {
                1.0
            } else {
                0.0
            };
            self.backend_kind
                .with_label_values(&[&kind.to_string()])
                .set(value);
        }

        match report.turbo {
            Some(enabled) => {
                self.turbo_managed.set(1);
                self.turbo_enabled.set(i64::from(enabled));
            }
            None => self.turbo_managed.set(0),
        }

        self.reconcile_total
            .with_label_values(&[report.result_label()])
            .inc();
        self.last_reconcile_timestamp.set(now_unix());
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut buffer = Vec::new();
        let encoder = TextEncoder::new();
        let families = self.registry.gather();
        // Encoding a well-formed metric family into a Vec<u8> buffer cannot
        // realistically fail; swallow rather than propagate into the HTTP
        // handler's error path.
        let _ = encoder.encode(&families, &mut buffer);
        buffer
    }
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
