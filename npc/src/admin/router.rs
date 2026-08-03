use std::sync::Arc;

use axum::{Router, routing::{delete, get}};
use tower_http::trace::TraceLayer;

use crate::npc::manager::NpcManager;

use super::handlers;

pub fn build_router(manager: Arc<NpcManager>) -> Router {
    Router::new()
        .route("/healthz", get(handlers::healthz))
        .route("/npcs", get(handlers::list_npcs).post(handlers::create_npc))
        .route("/npcs/{name}", delete(handlers::delete_npc))
        .layer(TraceLayer::new_for_http())
        .with_state(manager)
}
