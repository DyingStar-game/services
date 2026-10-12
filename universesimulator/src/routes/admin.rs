//! Admin configuration, orders and simulation control. Reads need a moderation role, writes
//! need `admin` (ADR 0011).

use axum::{extract::{Path, Query, State}, http::StatusCode, Extension, Json};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use super::reads::FederationQuery;
use crate::auth::{Caller, ROLE_WRITE};
use crate::db::{catalog::{self, FederationInput, ResourceInput}, history, orders::SupplyOrder, world::{self, FlowInput, MineralSourceInput, PoiCache, TravelRouteInput, VillagePolicy, WarehouseCapacityInput}};
use crate::error::{AppError, Result};
use crate::import::{self, ImportReport};
use crate::simulation::{self, planner, reconciler, TickReport};
use crate::state::AppState;

fn not_found(what: &str, found: bool) -> Result<StatusCode> {
    if found { Ok(StatusCode::NO_CONTENT) } else { Err(AppError::NotFound(what.to_string())) }
}

// ── Federations, resources ───────────────────────────────────────────────────

pub async fn create_federation(State(st): State<AppState>, Extension(c): Extension<Caller>, Json(body): Json<FederationInput>) -> Result<(StatusCode, Json<catalog::Federation>)> {
    c.require_admin()?;
    if body.code.is_none() || body.name.is_none() || body.political_entity_id.is_none() || body.planet_internal_name.is_none() {
        return Err(AppError::BadRequest("code, name, politicalEntityId and planetInternalName are required".into()));
    }
    Ok((StatusCode::CREATED, Json(catalog::upsert_federation(&st.pool, None, &body).await?)))
}

pub async fn update_federation(State(st): State<AppState>, Extension(c): Extension<Caller>, Path(id): Path<Uuid>, Json(mut body): Json<FederationInput>) -> Result<Json<catalog::Federation>> {
    c.require_admin()?;
    let current = catalog::get_federation(&st.pool, id).await?.ok_or_else(|| AppError::NotFound(format!("federation {id}")))?;
    body.code = Some(current.code);
    Ok(Json(catalog::upsert_federation(&st.pool, Some(id), &body).await?))
}

pub async fn upsert_resource(State(st): State<AppState>, Extension(c): Extension<Caller>, Json(body): Json<ResourceInput>) -> Result<(StatusCode, Json<catalog::Resource>)> {
    c.require_admin()?;
    Ok((StatusCode::CREATED, Json(catalog::upsert_resource(&st.pool, &body).await?)))
}

pub async fn delete_resource(State(st): State<AppState>, Extension(c): Extension<Caller>, Path(id): Path<Uuid>) -> Result<StatusCode> {
    c.require_admin()?;
    not_found("resource", catalog::delete_resource(&st.pool, id).await?)
}

// ── Villages and warehouses ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VillageUpdate {
    pub active: Option<bool>,
    /// Must be one of the village's warehouses.
    pub main_warehouse_id: Option<Uuid>,
}

pub async fn update_village(State(st): State<AppState>, Extension(c): Extension<Caller>, Path(id): Path<Uuid>, Json(body): Json<VillageUpdate>) -> Result<Json<world::Village>> {
    c.require_admin()?;
    world::get_village(&st.pool, id).await?.ok_or_else(|| AppError::NotFound(format!("village {id}")))?;
    if let Some(main) = body.main_warehouse_id {
        let w = world::get_warehouse(&st.pool, main).await?.filter(|w| w.village_id == id);
        if w.is_none() {
            return Err(AppError::BadRequest("mainWarehouseId is not a warehouse of this village".into()));
        }
        world::set_main_warehouse(&st.pool, id, main, true).await?;
    }
    if let Some(active) = body.active {
        world::set_village_active(&st.pool, id, active).await?;
    }
    Ok(Json(world::get_village(&st.pool, id).await?.ok_or_else(|| AppError::NotFound(format!("village {id}")))?))
}

pub async fn deactivate_warehouse(State(st): State<AppState>, Extension(c): Extension<Caller>, Path(id): Path<Uuid>) -> Result<StatusCode> {
    c.require_admin()?;
    not_found("warehouse", world::set_warehouse_active(&st.pool, id, false).await?)
}

pub async fn refresh_warehouse_poi(State(st): State<AppState>, Extension(c): Extension<Caller>, Path(id): Path<Uuid>) -> Result<Json<world::Warehouse>> {
    c.require_admin()?;
    let w = world::get_warehouse(&st.pool, id).await?.ok_or_else(|| AppError::NotFound(format!("warehouse {id}")))?;
    let snap = st.clients.persistence.get_item(id).await?.ok_or_else(|| AppError::BadRequest(format!("POI_NOT_FOUND: persistence object {id} does not exist")))?;
    let poi = PoiCache { name: w.name.clone(), scenename: snap.scenename, position: snap.position };
    Ok(Json(world::upsert_warehouse(&st.pool, id, w.village_id, &w.code, &poi).await?))
}

// ── Routes ───────────────────────────────────────────────────────────────────

pub async fn list_routes(State(st): State<AppState>, Query(q): Query<FederationQuery>) -> Result<Json<Value>> {
    Ok(Json(json!({ "items": world::list_routes(&st.pool, q.federation).await? })))
}

pub async fn upsert_route(State(st): State<AppState>, Extension(c): Extension<Caller>, Json(body): Json<TravelRouteInput>) -> Result<(StatusCode, Json<world::TravelRoute>)> {
    c.require_admin()?;
    let from = world::get_village(&st.pool, body.from_village_id).await?.ok_or_else(|| AppError::BadRequest("unknown fromVillageId".into()))?;
    let to = world::get_village(&st.pool, body.to_village_id).await?.ok_or_else(|| AppError::BadRequest("unknown toVillageId".into()))?;
    if from.federation_id != to.federation_id {
        return Err(AppError::BadRequest("a route cannot cross federations".into()));
    }
    Ok((StatusCode::CREATED, Json(world::upsert_route(&st.pool, &body).await?)))
}

pub async fn delete_route(State(st): State<AppState>, Extension(c): Extension<Caller>, Path(id): Path<Uuid>) -> Result<StatusCode> {
    c.require_admin()?;
    not_found("route", world::delete_route(&st.pool, id).await?)
}

// ── Village policies, flows, mineral sources; warehouse capacities ───────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldQuery {
    pub federation: Option<Uuid>,
    pub village: Option<Uuid>,
    pub warehouse: Option<Uuid>,
}

pub async fn list_policies(State(st): State<AppState>, Query(q): Query<WorldQuery>) -> Result<Json<Value>> {
    Ok(Json(json!({ "items": world::list_policies(&st.pool, q.village, q.federation).await? })))
}

pub async fn upsert_policy(State(st): State<AppState>, Extension(c): Extension<Caller>, Json(body): Json<VillagePolicy>) -> Result<Json<VillagePolicy>> {
    c.require_admin()?;
    if body.min_threshold < 0 || body.target_level < body.min_threshold {
        return Err(AppError::BadRequest("expected 0 <= minThreshold <= targetLevel".into()));
    }
    Ok(Json(world::upsert_policy(&st.pool, &body).await?))
}

pub async fn delete_policy(State(st): State<AppState>, Extension(c): Extension<Caller>, Path((v, r)): Path<(Uuid, Uuid)>) -> Result<StatusCode> {
    c.require_admin()?;
    not_found("village policy", world::delete_policy(&st.pool, v, r).await?)
}

pub async fn list_capacities(State(st): State<AppState>, Query(q): Query<WorldQuery>) -> Result<Json<Value>> {
    Ok(Json(json!({ "items": world::list_capacities(&st.pool, q.warehouse, q.village, q.federation).await? })))
}

pub async fn upsert_capacity(State(st): State<AppState>, Extension(c): Extension<Caller>, Json(body): Json<WarehouseCapacityInput>) -> Result<Json<world::WarehouseCapacity>> {
    c.require_admin()?;
    if body.capacity.is_some_and(|c| c < 0) {
        return Err(AppError::BadRequest("capacity must be positive or null".into()));
    }
    Ok(Json(world::upsert_capacity(&st.pool, &body).await?))
}

pub async fn delete_capacity(State(st): State<AppState>, Extension(c): Extension<Caller>, Path((w, r)): Path<(Uuid, Uuid)>) -> Result<StatusCode> {
    c.require_admin()?;
    not_found("warehouse capacity", world::delete_capacity(&st.pool, w, r).await?)
}

pub async fn list_flows(State(st): State<AppState>, Query(q): Query<WorldQuery>) -> Result<Json<Value>> {
    Ok(Json(json!({ "items": world::list_flows(&st.pool, q.village, q.federation).await? })))
}

pub async fn create_flow(State(st): State<AppState>, Extension(c): Extension<Caller>, Json(body): Json<FlowInput>) -> Result<(StatusCode, Json<world::Flow>)> {
    c.require_admin()?;
    Ok((StatusCode::CREATED, Json(world::create_flow(&st.pool, &body).await?)))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FlowUpdate {
    pub rate_per_hour: Option<f64>,
    /// Absent: unchanged; `null` or empty: cleared.
    #[serde(default, deserialize_with = "present")]
    pub label: Option<Option<String>>,
}

/// Distinguishes an absent field (`None`) from an explicit `null` (`Some(None)`).
fn present<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Option<Option<String>>, D::Error> {
    Ok(Some(Option::<String>::deserialize(d)?))
}

pub async fn update_flow(State(st): State<AppState>, Extension(c): Extension<Caller>, Path(id): Path<Uuid>, Json(body): Json<FlowUpdate>) -> Result<Json<world::Flow>> {
    c.require_admin()?;
    Ok(Json(world::update_flow(&st.pool, id, body.rate_per_hour, body.label).await?.ok_or_else(|| AppError::NotFound(format!("flow {id}")))?))
}

pub async fn delete_flow(State(st): State<AppState>, Extension(c): Extension<Caller>, Path(id): Path<Uuid>) -> Result<StatusCode> {
    c.require_admin()?;
    not_found("flow", world::delete_flow(&st.pool, id).await?)
}

pub async fn list_mineral_sources(State(st): State<AppState>, Query(q): Query<WorldQuery>) -> Result<Json<Value>> {
    Ok(Json(json!({ "items": world::list_mineral_sources(&st.pool, q.federation, q.village).await? })))
}

pub async fn upsert_mineral_source(State(st): State<AppState>, Extension(c): Extension<Caller>, Json(body): Json<MineralSourceInput>) -> Result<Json<world::MineralSource>> {
    c.require_admin()?;
    Ok(Json(world::upsert_mineral_source(&st.pool, &body).await?))
}

pub async fn delete_mineral_source(State(st): State<AppState>, Extension(c): Extension<Caller>, Path(id): Path<Uuid>) -> Result<StatusCode> {
    c.require_admin()?;
    not_found("mineral source", world::delete_mineral_source(&st.pool, id).await?)
}

// ── Orders ───────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OrderInput {
    pub resource_id: Uuid,
    pub quantity: i64,
    pub destination_village_id: Uuid,
    pub origin_village_id: Option<Uuid>,
}

async fn create_order(st: &AppState, c: &Caller, body: OrderInput) -> Result<(StatusCode, Json<SupplyOrder>)> {
    if body.quantity <= 0 {
        return Err(AppError::BadRequest("quantity must be positive".into()));
    }
    catalog::get_resource(&st.pool, body.resource_id).await?.ok_or_else(|| AppError::BadRequest("unknown resourceId".into()))?;
    let order = planner::manual_order(st, body.resource_id, body.quantity, body.destination_village_id, body.origin_village_id, c.label()).await?;
    Ok((StatusCode::CREATED, Json(order)))
}

pub async fn create_order_admin(State(st): State<AppState>, Extension(c): Extension<Caller>, Json(body): Json<OrderInput>) -> Result<(StatusCode, Json<SupplyOrder>)> {
    c.require_admin()?;
    create_order(&st, &c, body).await
}

pub async fn create_order_internal(State(st): State<AppState>, Extension(c): Extension<Caller>, Json(body): Json<OrderInput>) -> Result<(StatusCode, Json<SupplyOrder>)> {
    c.require_role(ROLE_WRITE)?;
    create_order(&st, &c, body).await
}

pub async fn cancel_order(State(st): State<AppState>, Extension(c): Extension<Caller>, Path(id): Path<Uuid>) -> Result<StatusCode> {
    c.require_admin()?;
    reconciler::cancel_order(&st, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Simulation ───────────────────────────────────────────────────────────────

pub async fn simulation_state(State(st): State<AppState>) -> Result<Json<Value>> {
    let state = history::simulation_state(&st.pool).await?;
    Ok(Json(json!({
        "state": state,
        "enabled": st.cfg.sim_enabled,
        "tickSeconds": st.cfg.sim_tick_seconds,
        "timeScale": st.cfg.sim_time_scale,
        "externalMode": format!("{:?}", st.cfg.external_mode).to_lowercase(),
    })))
}

pub async fn force_tick(State(st): State<AppState>, Extension(c): Extension<Caller>) -> Result<Json<TickReport>> {
    c.require_admin()?;
    Ok(Json(simulation::tick(&st, true).await?))
}

pub async fn pause(State(st): State<AppState>, Extension(c): Extension<Caller>) -> Result<Json<history::SimulationState>> {
    c.require_admin()?;
    Ok(Json(history::set_paused(&st.pool, true).await?))
}

pub async fn resume(State(st): State<AppState>, Extension(c): Extension<Caller>) -> Result<Json<history::SimulationState>> {
    c.require_admin()?;
    Ok(Json(history::set_paused(&st.pool, false).await?))
}

pub async fn import(State(st): State<AppState>, Extension(c): Extension<Caller>, Json(body): Json<Value>) -> Result<Json<ImportReport>> {
    c.require_admin()?;
    Ok(Json(import::import_and_keep(&st, body).await?))
}
