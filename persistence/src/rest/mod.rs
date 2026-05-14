pub mod handlers;

use axum::{
    Router,
    routing::{delete, get, post, put},
};
use tower_http::trace::TraceLayer;

use crate::websocket::server::AppState;

/// Build the axum router for the REST API.
pub fn build_rest_router(state: AppState) -> Router {
    Router::new()
        .route("/items", get(handlers::get_items))
        .route("/items", post(handlers::create_item))
        .route("/items/{uuid}", get(handlers::get_item))
        .route("/items/{uuid}", put(handlers::put_item))
        .route("/items/{uuid}", delete(handlers::delete_item))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
