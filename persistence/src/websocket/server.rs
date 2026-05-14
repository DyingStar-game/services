use std::sync::Arc;

use axum::{
    Router,
    extract::{State, WebSocketUpgrade, ws::WebSocket},
    response::IntoResponse,
    routing::get,
};
use scylla::client::session::Session;
use tokio::sync::broadcast;
use tower_http::trace::TraceLayer;

use crate::{
    cache::DualCache,
    config::Config,
    db::queries::Queries,
    websocket::handlers::handle_socket,
};

/// Shared application state passed to each handler.
#[derive(Clone)]
pub struct AppState {
    pub session: Arc<Session>,
    pub queries: Arc<Queries>,
    pub cache: DualCache,
    pub config: Arc<Config>,
    /// Broadcast channel used to push events from the REST API to all active
    /// WebSocket connections.
    pub broadcaster: broadcast::Sender<String>,
}

/// Build the axum router with the `/ws` WebSocket endpoint.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/ws", get(ws_handler))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn ws_handler(
    upgrade: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    upgrade.on_upgrade(move |socket: WebSocket| async move {
        handle_socket(
            socket,
            state.session,
            state.queries,
            state.cache,
            state.config,
            state.broadcaster,
        )
        .await;
    })
}
