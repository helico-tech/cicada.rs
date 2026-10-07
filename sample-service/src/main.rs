//! Starts the sample node on `0.0.0.0:$PORT`, configured by environment variables (see
//! `SampleConfig::from_vars`). SIGTERM or SIGINT stops it gracefully: the HTTP server stops accepting requests, then
//! the node stops claiming, lets running turns finish within the shutdown grace period and releases the leases of the
//! rest, then the store is closed and telemetry flushed.

use std::process::ExitCode;

use cicada_sample_service::{SampleConfig, SampleNode, serve};
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> ExitCode {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,sqlx=warn"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("{e:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> anyhow::Result<()> {
    let config = SampleConfig::from_env()?;
    tracing::info!("{config:?}");
    let node = SampleNode::start(&config).await?;
    let listener = TcpListener::bind(("0.0.0.0", config.port)).await?;
    tracing::info!("Listening on {}", listener.local_addr()?);
    serve(node, listener, shutdown_signal()).await
}

/// Completes on SIGTERM (`docker stop`, Kubernetes) or SIGINT (Ctrl-C).
async fn shutdown_signal() {
    let interrupt = async {
        if let Err(e) = tokio::signal::ctrl_c().await {
            tracing::warn!("Cannot listen for SIGINT: {e}");
            std::future::pending::<()>().await;
        }
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(e) => {
                tracing::warn!("Cannot listen for SIGTERM: {e}");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = interrupt => tracing::info!("SIGINT received; shutting down"),
        () = terminate => tracing::info!("SIGTERM received; shutting down"),
    }
}
