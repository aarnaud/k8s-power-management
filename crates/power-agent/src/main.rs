mod config;
mod http;
mod metrics;
mod reconcile;
mod status;

use std::sync::Arc;
use std::sync::atomic::AtomicI64;

use anyhow::Context;
use cpu_power_hal::{PowerBackend, RootedSysfs, detect_backend};
use kube::Client;
use tracing_subscriber::EnvFilter;

use config::{Config, LogFormat};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::from_env().context("invalid configuration")?;
    init_tracing(config.log_format);

    tracing::info!(node = %config.node_name, "starting power-agent");

    let backend: Arc<dyn PowerBackend> = Arc::from(detect_backend(Arc::new(RootedSysfs::host())));
    tracing::info!(backend = %backend.kind(), supported = backend.is_supported(), "detected CPU power backend");

    let client = Client::try_default()
        .await
        .context("failed to build Kubernetes client")?;

    let metrics = metrics::Metrics::new().context("failed to initialize metrics")?;
    let last_reconcile_unix = Arc::new(AtomicI64::new(0));

    let http_state = http::HttpState {
        metrics: metrics.clone(),
        last_reconcile_unix: last_reconcile_unix.clone(),
        staleness_threshold: config.resync_interval * 3,
    };
    let listener = tokio::net::TcpListener::bind(&config.metrics_addr)
        .await
        .with_context(|| format!("failed to bind metrics server on {}", config.metrics_addr))?;
    tracing::info!(addr = %config.metrics_addr, "serving /metrics and /healthz");
    let http_server = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, http::router(http_state)).await {
            tracing::error!(error = %e, "metrics server exited unexpectedly");
        }
    });

    let runtime = reconcile::Runtime {
        client,
        config,
        backend,
        metrics,
        last_reconcile_unix,
    };

    tokio::select! {
        result = runtime.run() => {
            result.context("reconcile loop exited")?;
        }
        () = shutdown_signal() => {
            // Deliberately do not revert sysfs to some other profile here:
            // whatever was last applied is the intended steady state and
            // should persist across the pod restart/reschedule that
            // triggered this shutdown.
            tracing::info!("received shutdown signal, exiting");
        }
    }

    http_server.abort();
    Ok(())
}

async fn shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};
    let mut sigterm = match signal(SignalKind::terminate()) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "failed to install SIGTERM handler, falling back to Ctrl-C only");
            let _ = tokio::signal::ctrl_c().await;
            return;
        }
    };
    tokio::select! {
        _ = sigterm.recv() => {}
        _ = tokio::signal::ctrl_c() => {}
    }
}

fn init_tracing(format: LogFormat) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    match format {
        LogFormat::Json => {
            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .json()
                .init();
        }
        LogFormat::Pretty => {
            tracing_subscriber::fmt().with_env_filter(filter).init();
        }
    }
}
