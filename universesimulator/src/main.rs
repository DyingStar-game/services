mod auth;
mod clients;
mod config;
mod db;
mod error;
mod graph;
mod import;
mod pricing;
mod routes;
mod simulation;
mod state;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use anyhow::Context;
use tower_http::trace::TraceLayer;
use tracing::info;

use auth::Auth;
use clients::Clients;
use config::Config;
use state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load .env if present (non-fatal).
    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "universesimulator=info,tower_http=info".parse().unwrap()),
        )
        .init();

    let cfg = Config::from_env().context("loading config")?;
    let pool = db::connection::init(&cfg).await.context("database init")?;

    let st = AppState {
        pool,
        clients: Clients::from_config(&cfg),
        auth: Arc::new(Auth::new(&cfg)),
        cfg: Arc::new(cfg.clone()),
    };

    if cfg.sim_enabled {
        tokio::spawn(simulation::run_loop(st.clone()));
    }

    let app = routes::build(st).layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(&cfg.bind)
        .await
        .with_context(|| format!("binding to {}", cfg.bind))?;

    info!(addr = %cfg.bind, mode = ?cfg.external_mode, "universesimulator listening");
    axum::serve(listener, app).await.context("server error")?;
    Ok(())
}
