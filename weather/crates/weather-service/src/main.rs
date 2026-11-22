//! DyingStar weather service.
//!
//! One process, one port: `/ws` for Horizon's `ds_bridge` (we push `genericprops` envelopes that
//! create, update and delete `weather_system` objects), `/api/v1/...` for maps, `/health` and
//! `/metrics`. The weather itself is a pure function of (data, seed, time) in `weather-core`; this
//! binary only schedules it and talks to the outside.

mod config;
mod http;
mod sim;
mod state;
mod ws;

use axum::routing::get;
use axum::Router;
use std::net::SocketAddr;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "weather=info,tower_http=warn".parse().unwrap()))
        .init();

    let config = config::Config::from_env();
    info!(?config, "starting weather service");
    let state = state::AppState::load(config.clone())?;
    if let Err(e) = state.refresh_planet_uuids().await {
        info!(error = %e, "celestial service not reachable at boot; will retry");
    }

    tokio::spawn(sim::run(state.clone()));

    let app = Router::new()
        .route("/ws", get(ws::ws_handler))
        .route("/health", get(http::health))
        .route("/metrics", get(http::metrics))
        .route("/api/v1/bodies", get(http::bodies))
        .route("/api/v1/{body}/climate", get(http::climate))
        .route("/api/v1/{body}/systems", get(http::systems))
        .route("/api/v1/{body}/point", get(http::point))
        .route("/api/v1/{body}/fields", get(http::fields))
        .route("/api/v1/{body}/forecast", get(http::forecast))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr: SocketAddr = format!("0.0.0.0:{}", config.port).parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!(%addr, "listening");
    axum::serve(listener, app).await?;
    Ok(())
}
