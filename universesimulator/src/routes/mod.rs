//! REST API: public, player (`/api`), internal (`/api/internal`) and admin (`/api/admin`).

pub mod admin;
pub mod deliveries;
pub mod health;
pub mod reads;
pub mod views;

use axum::{
    middleware,
    routing::{delete, get, post, put},
    Router,
};

use crate::auth;
use crate::state::AppState;

/// Read routes shared by players and services.
fn reads() -> Router<AppState> {
    Router::new()
        .route("/federations", get(reads::federations))
        .route("/resources", get(reads::resources))
        .route("/prices", get(reads::prices))
        .route("/villages", get(reads::villages))
        .route("/villages/{id}", get(reads::village))
        .route("/warehouses", get(reads::warehouses))
        .route("/warehouses/{id}", get(reads::warehouse))
        .route("/entities", get(reads::entities))
        .route("/entities/{id}/stock", get(reads::entity_stock))
        .route("/paths", get(reads::path))
        .route("/orders", get(reads::orders))
        .route("/orders/{id}", get(reads::order))
}

/// Admin routes extend the read routes; axum merges the methods of a repeated path.
pub fn build(st: AppState) -> Router {
    let player = reads().layer(middleware::from_fn_with_state(st.clone(), auth::player_layer));

    let internal = reads()
        .route("/warehouses/by-poi/{uuid}", get(reads::warehouse))
        .route("/warehouse-deliveries", post(deliveries::create))
        .route("/orders", post(admin::create_order_internal))
        .layer(middleware::from_fn_with_state(st.clone(), auth::service_layer));

    let admin = reads()
        // Supervision views (adminpanel)
        .route("/dashboard", get(views::dashboard))
        .route("/network", get(views::network))
        .route("/stock-history", get(views::stock_history))
        .route("/price-history", get(views::price_history))
        .route("/local-prices", get(views::local_prices))
        .route("/incoming", get(views::incoming))
        .route("/mint-events", get(views::mint_events))
        // Configuration
        .route("/federations", post(admin::create_federation))
        .route("/federations/{id}", put(admin::update_federation))
        .route("/resources", post(admin::upsert_resource))
        .route("/resources/{id}", delete(admin::delete_resource))
        .route("/villages/{id}", put(admin::update_village))
        .route("/warehouses/{id}", delete(admin::deactivate_warehouse))
        .route("/warehouses/{id}/refresh-poi", post(admin::refresh_warehouse_poi))
        .route("/travel-routes", get(admin::list_routes).post(admin::upsert_route))
        .route("/travel-routes/{id}", delete(admin::delete_route))
        .route("/village-policies", get(admin::list_policies).put(admin::upsert_policy))
        .route("/village-policies/{village}/{resource}", delete(admin::delete_policy))
        .route("/warehouse-capacities", get(admin::list_capacities).put(admin::upsert_capacity))
        .route("/warehouse-capacities/{warehouse}/{resource}", delete(admin::delete_capacity))
        .route("/flows", get(admin::list_flows).post(admin::create_flow))
        .route("/flows/{id}", put(admin::update_flow).delete(admin::delete_flow))
        .route("/mineral-sources", get(admin::list_mineral_sources).put(admin::upsert_mineral_source))
        .route("/mineral-sources/{id}", delete(admin::delete_mineral_source))
        // Orders
        .route("/orders", post(admin::create_order_admin))
        .route("/orders/{id}/cancel", post(admin::cancel_order))
        // Simulation
        .route("/simulation", get(admin::simulation_state))
        .route("/simulation/tick", post(admin::force_tick))
        .route("/simulation/pause", post(admin::pause))
        .route("/simulation/resume", post(admin::resume))
        .route("/import", post(admin::import))
        .layer(middleware::from_fn_with_state(st.clone(), auth::admin_layer));

    Router::new()
        .route("/healthz", get(health::healthz))
        .route("/openapi.yaml", get(health::openapi))
        .nest("/api/internal", internal)
        .nest("/api/admin", admin)
        .nest("/api", player)
        .with_state(st)
}
