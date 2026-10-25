//! `/api/admin/…`: the global switch, villages and their workplaces, debug
//! views and the Keycloak consistency report.

use std::{collections::HashSet, sync::Arc};

use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    auth::Caller,
    db,
    error::{NpcError, Result},
    state::AppState,
    world_data::{WorldData, geo, poi::VillageKind},
};

pub async fn healthz() -> &'static str {
    "ok"
}

pub async fn settings(State(st): State<AppState>) -> Result<Json<Value>> {
    let s = db::settings::get(st.mgr.pool()).await?;
    let counts = st.mgr.counts().await?;
    Ok(Json(json!({ "settings": s, "counts": counts })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsBody {
    pub npcs_enabled: bool,
}

/// The global switch: connect (or disconnect) every enabled NPC.
pub async fn set_settings(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Json(b): Json<SettingsBody>,
) -> Result<Json<Value>> {
    caller.require_admin()?;
    let s = db::settings::set_enabled(st.mgr.pool(), b.npcs_enabled, Some(&caller.label())).await?;
    st.mgr.events().global(
        "global_switch",
        format!("NPCs {} by {}", if b.npcs_enabled { "enabled" } else { "disabled" }, caller.label()),
        Some(json!({ "npcsEnabled": b.npcs_enabled })),
    );
    st.mgr.poke();
    let counts = st.mgr.counts().await?;
    Ok(Json(json!({ "settings": s, "counts": counts })))
}

fn world_data() -> Result<Arc<WorldData>> {
    WorldData::shared().ok_or_else(|| NpcError::Config("no world data loaded (see WORLD_DATA_DIR)".into()))
}

/// Mining villages with their storekeepers, NPC count, workplace and the
/// Factory their drivers go to; then the Factory villages.
pub async fn villages(State(st): State<AppState>) -> Result<Json<Value>> {
    let wd = world_data()?;
    let pool = st.mgr.pool();
    let storekeepers: std::collections::HashMap<String, i64> =
        db::npcs::storekeepers_by_village(pool).await?.into_iter().collect();
    let npcs: std::collections::HashMap<String, i64> = db::npcs::count_by_village(pool).await?.into_iter().collect();
    let workplaces: std::collections::HashMap<String, db::workplaces::Workplace> = db::workplaces::list(pool)
        .await?
        .into_iter()
        .map(|w| (w.village_code.clone(), w))
        .collect();
    let mining: Vec<Value> = wd
        .villages
        .of_kind(VillageKind::Mining)
        .map(|v| {
            json!({
                "name": v.name,
                "lon": v.lon,
                "lat": v.lat,
                "npcs": npcs.get(&v.name).copied().unwrap_or(0),
                "storekeepers": storekeepers.get(&v.name).copied().unwrap_or(0),
                "storekeeperSlots": st.mgr.roles.storekeeper_slots(),
                "workplace": workplaces.get(&v.name),
                "factory": wd.factory_for.get(&v.name),
            })
        })
        .collect();
    let factories: Vec<Value> = wd
        .villages
        .of_kind(VillageKind::Factory)
        .map(|v| json!({ "name": v.name, "lon": v.lon, "lat": v.lat }))
        .collect();
    Ok(Json(json!({ "planet": wd.config.planet, "mining": mining, "factories": factories })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkplaceBody {
    pub warehouse_id: Uuid,
    #[serde(default)]
    pub warehouse_code: Option<String>,
    #[serde(default)]
    pub scenename: Option<String>,
}

/// Set a village's workplace by hand (wins over the automatic sources).
pub async fn set_workplace(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(code): Path<String>,
    Json(b): Json<WorkplaceBody>,
) -> Result<Json<Value>> {
    caller.require_admin()?;
    let w = db::workplaces::upsert(
        st.mgr.pool(),
        &code,
        b.warehouse_id,
        b.warehouse_code.as_deref(),
        b.scenename.as_deref(),
        "manual",
        Some(&caller.label()),
    )
    .await?;
    st.mgr.events().global("workplace_set", format!("workplace of {code} set by {}", caller.label()), None);
    Ok(Json(json!({ "workplace": w })))
}

/// Forget a village's workplace: the next assignment resolves it again.
pub async fn delete_workplace(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(code): Path<String>,
) -> Result<StatusCode> {
    caller.require_admin()?;
    if !db::workplaces::delete(st.mgr.pool(), &code).await? {
        return Err(NpcError::NotFound(format!("workplace of {code}")));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// What each running NPC sees on its workplace (debug).
pub async fn debug_world(State(st): State<AppState>) -> Json<Value> {
    let reports = st.mgr.world_status().await;
    Json(json!({ "count": reports.len(), "npcs": reports }))
}

/// The road route between two villages (by name, e.g. `M0012` → `F0003`), as
/// GeoJSON — drop it into QGIS over the road layer.
pub async fn debug_route(Path((from, to)): Path<(String, String)>) -> Result<Json<Value>> {
    let wd = world_data()?;
    let a = wd.villages.by_name(&from).ok_or_else(|| NpcError::NotFound(from.clone()))?;
    let b = wd.villages.by_name(&to).ok_or_else(|| NpcError::NotFound(to.clone()))?;
    let access = wd.graph.locate(b.unit).ok_or_else(|| NpcError::NotFound(format!("road near {to}")))?;
    let end = geo::scale(geo::lonlat_to_unit(access.lonlat.0, access.lonlat.1), wd.radius_m());
    let route = wd
        .route(wd.village_position(a), end)
        .ok_or_else(|| NpcError::NotFound(format!("road route {from} -> {to}")))?;
    Ok(Json(route.to_geojson(json!({
        "from": a.name,
        "to": b.name,
        "length_m": route.length_m.round(),
        "points": route.points.len(),
    }))))
}

/// Keycloak users of group `npcs` without a row (orphans, never deleted
/// automatically) and rows without a user.
pub async fn keycloak_health(State(st): State<AppState>) -> Result<Json<Value>> {
    let members = st.mgr.kc.list_group_members().await?;
    let rows: Vec<(Uuid, String)> = sqlx::query_as("SELECT id, login FROM npcs").fetch_all(st.mgr.pool()).await?;
    let row_ids: HashSet<Uuid> = rows.iter().map(|r| r.0).collect();
    let member_ids: HashSet<Uuid> = members.iter().map(|m| m.id).collect();
    let orphans: Vec<Value> = members
        .iter()
        .filter(|m| !row_ids.contains(&m.id))
        .map(|m| json!({ "id": m.id, "username": m.username }))
        .collect();
    let missing: Vec<Value> = rows
        .iter()
        .filter(|r| !member_ids.contains(&r.0))
        .map(|r| json!({ "id": r.0, "login": r.1 }))
        .collect();
    Ok(Json(json!({ "groupMembers": members.len(), "orphans": orphans, "missing": missing })))
}
