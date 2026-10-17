pub mod actions;
pub mod admin;
pub mod clock;
pub mod config;
pub mod depot;
pub mod error;
pub mod horizon;
pub mod jwt;
pub mod npc;
pub mod resources_dynamic;
pub mod schedule;
pub mod slot_layouts;
pub mod world;

use std::{sync::Arc, time::Duration};

use anyhow::Context;
use tokio::net::TcpListener;
use tracing::info;

use crate::{
    clock::ClockParams, config::Config, npc::manager::NpcManager,
    resources_dynamic::PlanetCatalogHandle, schedule::Schedule,
};

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

    // Planetary clock parameters (process-wide anchor for the time-scale hook).
    let clock_params = Arc::new(ClockParams::new(
        config.rota_days,
        config.rota_epoch_offset_days,
        config.clock_override_unix,
        config.clock_time_scale,
    ));

    // Planet catalogue from resourcesDynamic (per-planet rotation period), with
    // a periodic refresh. Non-fatal if the service is unreachable.
    let planets = PlanetCatalogHandle::bootstrap(&config.resources_dynamic_http).await;
    planets.spawn_refresh(
        config.resources_dynamic_http.clone(),
        Duration::from_secs(config.clock_refresh_secs.max(1)),
    );

    // Shift schedules (non-fatal if the file is missing: no gating).
    let schedule = Arc::new(match std::fs::read_to_string(&config.schedules_path) {
        Ok(text) => match Schedule::from_yaml(&text) {
            Ok(s) => {
                info!(path = %config.schedules_path, "loaded shift schedules");
                s
            }
            Err(e) => {
                tracing::warn!(error = %e, path = %config.schedules_path, "invalid schedules file; no schedule gating");
                Schedule::empty()
            }
        },
        Err(_) => {
            info!(path = %config.schedules_path, "no schedules file; no schedule gating");
            Schedule::empty()
        }
    });

    let manager = Arc::new(NpcManager::new(
        Arc::clone(&config),
        clock_params,
        planets,
        schedule,
    ));

    // Load roster from YAML (non-fatal if file missing). The reconciler spawns
    // its first NPC_ACTIVE_COUNT entries, SPAWN_STAGGER_MS apart.
    if let Err(e) = manager.reload_roster().await {
        tracing::warn!(error = %e, path = %config.roster_path, "Failed to load roster file");
    }
    manager.spawn_reconciler();

    let app = admin::router::build_router(Arc::clone(&manager));
    let listener = TcpListener::bind(&config.admin_bind)
        .await
        .with_context(|| format!("binding admin API to {}", config.admin_bind))?;
    info!("Admin API listening on http://{}", config.admin_bind);

    axum::serve(listener, app).await?;
    Ok(())
}
