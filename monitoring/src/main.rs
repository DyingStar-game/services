pub mod config;
pub mod metrics;
pub mod websocket;

use std::{net::SocketAddr, sync::Arc};

use axum::{routing::get, Router};
use prometheus::{Encoder, TextEncoder};
use tokio::net::TcpListener;
use tracing::info;

use crate::{config::Config, metrics::Metrics, websocket::ws_handler};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load .env file if present (non-fatal if absent).
    let _ = dotenvy::dotenv();

    // Initialise structured logging.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "monitoring=debug,tower_http=info".parse().unwrap()),
        )
        .init();

    let config = Arc::new(Config::from_env());
    info!(?config, "Starting monitoring service");

    // Register all Prometheus metrics.
    Metrics::init();

    // Build the axum router:
    //   GET /ws      — WebSocket endpoint (ds_bridge connects here)
    //   GET /metrics — Prometheus text endpoint (Prometheus scrapes here)
    //   GET /health  — Simple liveness probe
    let app = Router::new()
        .route("/ws", get(ws_handler))
        .route("/metrics", get(metrics_handler))
        .route("/health", get(|| async { "ok" }));

    let addr: SocketAddr = format!("0.0.0.0:{}", config.port).parse()?;
    let listener = TcpListener::bind(addr).await?;
    info!(%addr, "listening");

    axum::serve(listener, app).await?;
    Ok(())
}

/// Render all registered Prometheus metrics as text.
async fn metrics_handler() -> impl axum::response::IntoResponse {
    let mut buf = Vec::new();
    let encoder = TextEncoder::new();
    let content_type = encoder.format_type().to_owned();
    let metric_families = prometheus::gather();
    encoder.encode(&metric_families, &mut buf).unwrap_or(());
    (
        [(axum::http::header::CONTENT_TYPE, content_type)],
        buf,
    )
}
