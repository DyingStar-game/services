pub mod actions;
pub mod admin;
pub mod config;
pub mod error;
pub mod horizon;
pub mod jwt;
pub mod npc;
pub mod slot_layouts;
pub mod world;

use std::sync::Arc;

use anyhow::Context;
use tokio::net::TcpListener;
use tracing::info;

use crate::{config::Config, npc::manager::NpcManager};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "npc=info,tower_http=debug".parse().unwrap()),
        )
        .init();

    let config = Arc::new(Config::from_env());
    info!(?config, "Starting NPC service");

    let manager = Arc::new(NpcManager::new(Arc::clone(&config)));

    // Load roster from YAML (non-fatal if file missing).
    if let Err(e) = manager.load_roster_file(&config.roster_path).await {
        tracing::warn!(error = %e, path = %config.roster_path, "Failed to load roster file");
    }

    let app = admin::router::build_router(Arc::clone(&manager));
    let listener = TcpListener::bind(&config.admin_bind)
        .await
        .with_context(|| format!("binding admin API to {}", config.admin_bind))?;
    info!("Admin API listening on http://{}", config.admin_bind);

    axum::serve(listener, app).await?;
    Ok(())
}
