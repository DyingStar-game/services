pub mod cache;
pub mod config;
pub mod db;
pub mod error;
pub mod rest;
pub mod websocket;

use std::{net::SocketAddr, sync::Arc};

use tokio::{net::TcpListener, sync::broadcast, time};
use tracing::{info, warn};

use scylla::client::session::Session;

use crate::{
    cache::DualCache,
    config::Config,
    db::{connection::connect, queries::Queries},
    rest::build_rest_router,
    websocket::server::{AppState, build_router},
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load .env file if present (non-fatal if absent).
    let _ = dotenvy::dotenv();

    // Initialise structured logging.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "persistence=info,tower_http=debug".parse().unwrap()),
        )
        .init();

    let config = Arc::new(Config::from_env());
    info!(?config, "Starting persistence service");

    // Connect to ScyllaDB and prepare statements.
    let session = Arc::new(
        connect(
            &config.scylla_nodes,
            &config.scylla_keyspace,
            config.scylla_username.as_deref(),
            config.scylla_password.as_deref(),
        )
        .await?,
    );
    let queries = Arc::new(Queries::prepare(&session, &config.scylla_keyspace).await?);

    // Shared dual-cache instance.
    let cache = DualCache::new();

    // Broadcast channel for pushing events from REST → WebSocket clients.
    // Capacity of 256 means up to 256 unread messages per slow subscriber
    // before they start being dropped (with a Lagged warning).
    let (broadcaster, _) = broadcast::channel::<String>(256);

    // Spawn the periodic flush task.
    spawn_flush_task(
        cache.clone(),
        Arc::clone(&session),
        Arc::clone(&queries),
        Arc::clone(&config),
    );

    // Build and start the axum server.
    let state = AppState {
        session,
        queries,
        cache,
        config: Arc::clone(&config),
        broadcaster,
    };

    let ws_addr = SocketAddr::from(([0, 0, 0, 0], config.ws_port));
    let ws_listener = TcpListener::bind(ws_addr).await?;
    info!("WebSocket server listening on ws://{ws_addr}/ws");

    let rest_addr = SocketAddr::from(([0, 0, 0, 0], config.rest_port));
    let rest_listener = TcpListener::bind(rest_addr).await?;
    info!("REST API server listening on http://{rest_addr}");

    let rest_router = build_rest_router(state.clone());
    tokio::spawn(async move {
        if let Err(e) = axum::serve(rest_listener, rest_router).await {
            tracing::error!("REST server error: {e}");
        }
    });

    axum::serve(ws_listener, build_router(state)).await?;

    Ok(())
}

/// Spawn a background task that swaps the dual-cache and flushes the drained
/// items to ScyllaDB every `config.cache_flush_interval_secs` seconds.
fn spawn_flush_task(
    cache: DualCache,
    session: Arc<Session>,
    queries: Arc<Queries>,
    config: Arc<Config>,
) {
    let interval_secs = config.cache_flush_interval_secs;

    tokio::spawn(async move {
        let mut interval = time::interval(time::Duration::from_secs(interval_secs));
        // The first tick fires immediately; skip it so we don't flush an empty
        // cache right after startup.
        interval.tick().await;

        loop {
            interval.tick().await;

            let pending = cache.pending_count();
            if pending == 0 {
                info!("Flush tick: cache empty, nothing to write");
                continue;
            }

            info!("Flush tick: draining {pending} item(s) from cache");
            let items = cache.swap_and_drain();

            if let Err(e) =
                crate::db::queries::batch_upsert_items(&session, &queries, items).await
            {
                warn!("Cache flush error: {e}");
            } else {
                info!("Flush complete: wrote {pending} item(s) to ScyllaDB");
            }
        }
    });
}
