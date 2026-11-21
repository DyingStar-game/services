use axum::{
    Router, middleware,
    routing::{delete, get, post, put},
};
use tower_http::trace::TraceLayer;

use super::{misc, npcs, schedule};
use crate::{auth, state::AppState};

pub fn build_router(st: AppState) -> Router {
    let admin = Router::new()
        .route("/settings", get(misc::settings).put(misc::set_settings))
        .route("/npcs", get(npcs::list).post(npcs::create))
        .route("/npcs/bulk", post(npcs::bulk))
        .route("/npcs/{id}", get(npcs::get).patch(npcs::patch).delete(npcs::delete))
        .route("/npcs/{id}/spawn", post(npcs::spawn))
        .route("/npcs/{id}/despawn", post(npcs::despawn))
        .route("/npcs/{id}/reassign", post(npcs::reassign))
        .route("/npcs/{id}/reset-credentials", post(npcs::reset_credentials))
        .route("/npcs/{id}/logs", get(npcs::logs))
        .route("/npcs/{id}/events", get(npcs::npc_events))
        .route("/npcs/{id}/tasks", get(npcs::list_tasks).post(npcs::create_task))
        .route("/npcs/{id}/tasks/{task_id}", delete(npcs::cancel_task))
        .route("/events", get(npcs::all_events))
        .route("/task-kinds", get(npcs::task_kinds))
        .route("/schedule/patterns", get(schedule::list).post(schedule::create))
        .route(
            "/schedule/patterns/{name}",
            get(schedule::get).put(schedule::update).delete(schedule::delete),
        )
        .route("/schedule/import", post(schedule::import))
        .route(
            "/schedule/activity-actions",
            get(schedule::activity_actions).put(schedule::set_activity_actions),
        )
        .route("/schedule/legend", get(schedule::legend))
        .route("/villages", get(misc::villages))
        .route("/villages/{code}/workplace", put(misc::set_workplace).delete(misc::delete_workplace))
        .route("/debug/world", get(misc::debug_world))
        .route("/debug/routes/{from}/{to}", get(misc::debug_route))
        .route("/health/keycloak", get(misc::keycloak_health))
        .layer(middleware::from_fn_with_state(st.clone(), auth::admin_layer));

    Router::new()
        .route("/healthz", get(misc::healthz))
        .nest("/api/admin", admin)
        .layer(TraceLayer::new_for_http())
        .with_state(st)
}
