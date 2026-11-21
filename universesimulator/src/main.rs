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

enum Command {
    Serve,
    Import(String),
    Tick,
}

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

    // Operator commands, run inside the pod (`kubectl exec … -- ./universesimulator <cmd>`):
    // cluster access replaces the admin token the HTTP routes require.
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        [] => Command::Serve,
        ["import", path] => Command::Import(path.to_string()),
        ["tick"] => Command::Tick,
        _ => anyhow::bail!("usage: universesimulator [import <world-file.json> | tick]"),
    };

    let cfg = Config::from_env().context("loading config")?;
    let pool = db::connection::init(&cfg).await.context("database init")?;

    let st = AppState {
        pool,
        clients: Clients::from_config(&cfg),
        auth: Arc::new(Auth::new(&cfg)),
        cfg: Arc::new(cfg.clone()),
    };

    match command {
        Command::Serve => {}
        Command::Import(path) => {
            let raw: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).with_context(|| format!("reading {path}"))?)
                .with_context(|| format!("parsing {path}"))?;
            let report = import::import_and_keep(&st, raw).await?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            return Ok(());
        }
        Command::Tick => {
            let report = simulation::tick(&st, true).await?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            return Ok(());
        }
    }

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
