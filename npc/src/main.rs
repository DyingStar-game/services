pub mod actions;
pub mod admin;
pub mod auth;
pub mod clock;
pub mod config;
pub mod crypto;
pub mod db;
pub mod depot;
pub mod error;
pub mod events;
pub mod horizon;
pub mod keycloak;
pub mod logs;
pub mod npc;
pub mod resources_dynamic;
pub mod roles;
pub mod scene_layouts;
pub mod schedule;
pub mod schedule_import;
pub mod slot_layouts;
pub mod state;
pub mod usim;
pub mod world;
pub mod world_data;

use std::{sync::Arc, time::Duration};

use anyhow::Context;
use tokio::net::TcpListener;
use tracing::{info, warn};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use crate::{
    auth::Auth,
    clock::ClockParams,
    config::Config,
    crypto::PasswordBox,
    depot::DepotWorkforce,
    events::EventSink,
    keycloak::{KcAdmin, ServiceToken, TokenManager},
    logs::{LogHub, NpcLogLayer},
    npc::{
        manager::{NpcManager, Services},
        names::NameGenerator,
    },
    resources_dynamic::PlanetCatalogHandle,
    roles::RoleTemplates,
    schedule::{Schedule, ScheduleStore},
    schedule_import::{ImportMode, import_schedules},
    state::AppState,
    usim::Usim,
};

// musl's allocator is slow under multi-threaded contention (see Cargo.toml).
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    let config = Arc::new(Config::from_env());

    // Recent log lines of each NPC are kept for the admin panel (see `logs`).
    let logs = LogHub::new(config.log_buffer_lines);
    tracing_subscriber::registry()
        .with(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "npc=info,tower_http=info".parse().unwrap()),
        )
        .with(tracing_subscriber::fmt::layer())
        .with(NpcLogLayer::new(Arc::clone(&logs)))
        .init();

    info!(?config, "Starting NPC service");
    let pool = db::connection::init(&config.database_url).await?;

    // `npc import-schedules [--overwrite] [file]`: import and exit.
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("import-schedules") {
        let overwrite = args.iter().any(|a| a == "--overwrite");
        let path = args[2..]
            .iter()
            .find(|a| !a.starts_with("--"))
            .cloned()
            .unwrap_or_else(|| config.schedules_path.clone());
        let yaml = std::fs::read_to_string(&path).with_context(|| format!("reading {path}"))?;
        let mode = if overwrite { ImportMode::Overwrite } else { ImportMode::Missing };
        let report = import_schedules(&pool, &yaml, mode, Some("cli")).await?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    let orphans = db::tasks::fail_orphans(&pool).await?;
    if orphans > 0 {
        warn!(count = orphans, "manual tasks left running by the previous process marked failed");
    }

    // Shift schedules live in the database; the YAML seeds an empty table.
    if db::patterns::count(&pool).await? == 0 {
        match std::fs::read_to_string(&config.schedules_path) {
            Ok(yaml) => {
                let report = import_schedules(&pool, &yaml, ImportMode::Missing, Some("first boot")).await?;
                info!(
                    path = %config.schedules_path,
                    created = report.created.len(),
                    errors = ?report.errors,
                    "imported shift schedules on first boot"
                );
            }
            Err(e) => warn!(error = %e, path = %config.schedules_path, "no schedules file to import"),
        }
    }
    let schedule = Arc::new(ScheduleStore::new(Schedule::empty()));
    schedule.reload(&pool).await?;

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

    // Static world data for the driving actions and the village of a spawn:
    // villages + road graph (QGIS exports) and the scene layouts exported from
    // Godot. Loaded now so a missing or broken file shows in the startup logs.
    let _ = world_data::WorldData::shared();
    let _ = scene_layouts::SceneLayouts::shared();

    let passwords = PasswordBox::from_base64(&config.npc_password_key)
        .context("NPC_PASSWORD_KEY (base64 of 32 bytes: `openssl rand -base64 32`)")?;
    let roles = Arc::new(RoleTemplates::load(&config.roles_path).context("role templates")?);
    let names = NameGenerator::new();

    let http = reqwest::Client::builder().timeout(Duration::from_secs(15)).build()?;
    let service_token = Arc::new(ServiceToken::new(
        http.clone(),
        config.token_url(),
        config.service_client_id.clone(),
        config.service_client_secret.clone(),
    ));
    if !service_token.configured() {
        warn!("SERVICE_CLIENT_SECRET is not set: NPC accounts cannot be created or repaired");
    }
    let kc = Arc::new(KcAdmin::new(
        http.clone(),
        &config.keycloak_url,
        &config.keycloak_realm,
        Arc::clone(&service_token),
    ));
    let usim = Usim::new(http.clone(), &config.universesimulator_url, Arc::clone(&service_token)).map(Arc::new);
    if usim.is_none() {
        warn!("universesimulator not configured: storekeepers find their warehouse from what they see");
    }

    let events = EventSink::spawn(pool.clone());
    events::spawn_retention(pool.clone(), config.events_retention_days);

    let tokens = Arc::new(TokenManager::new(
        http.clone(),
        config.token_url(),
        config.npc_login_client_id.clone(),
        config.npc_login_client_secret.clone(),
        pool.clone(),
        passwords.clone(),
        Arc::clone(&kc),
        events.clone(),
    ));

    let svc = Arc::new(Services {
        config: Arc::clone(&config),
        pool: pool.clone(),
        clock_params,
        planets,
        schedule,
        workforce: Arc::new(DepotWorkforce::new()),
        tokens,
        events,
    });
    let manager = NpcManager::new(svc, kc, usim, roles, names, passwords, logs);
    manager.start();

    let st = AppState { mgr: Arc::clone(&manager), auth: Arc::new(Auth::new(&config, http)) };
    let app = admin::router::build_router(st);
    let listener = TcpListener::bind(&config.admin_bind)
        .await
        .with_context(|| format!("binding admin API to {}", config.admin_bind))?;
    info!("Admin API listening on http://{}", config.admin_bind);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    // Close every Horizon session cleanly (Recreate deploys start the next
    // pod only once this one is gone).
    manager.shutdown().await;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {}
        _ = term => {}
    }
    info!("shutting down");
}
