//! Villages, warehouses, routes, village policies and flows, warehouse capacities, mineral
//! sources, observed stock levels and the political cache (ADR 0013).

use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::error::Result;
use crate::graph::RouteEdge;

/// POI data cached from persistence.
#[derive(Debug, Clone, Default)]
pub struct PoiCache {
    pub name: String,
    pub scenename: Option<String>,
    pub position: Option<[f64; 3]>,
}

fn xyz(p: &Option<[f64; 3]>) -> [Option<f64>; 3] {
    p.map(|p| p.map(Some)).unwrap_or([None, None, None])
}

// ── Villages ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Village {
    pub id: Uuid,
    pub code: String,
    pub name: String,
    pub federation_id: Uuid,
    pub political_entity_id: Uuid,
    pub village_type: Option<String>,
    pub population: i64,
    pub radius_m: Option<f64>,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub z: Option<f64>,
    pub main_warehouse_id: Option<Uuid>,
    pub active: bool,
}

const VILLAGE_COLS: &str =
    "id, code, name, federation_id, political_entity_id, village_type, population, radius_m, x, y, z, main_warehouse_id, active";

/// A village as read from persistence.
#[derive(Debug, Clone)]
pub struct VillageInput {
    pub id: Uuid,
    pub code: String,
    pub name: String,
    pub federation_id: Uuid,
    pub political_entity_id: Uuid,
    pub village_type: Option<String>,
    pub population: i64,
    pub radius_m: Option<f64>,
    pub position: Option<[f64; 3]>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VillageFilter {
    pub federation: Option<Uuid>,
    pub entity: Option<Uuid>,
}

pub async fn list_villages(pool: &PgPool, f: &VillageFilter) -> Result<Vec<Village>> {
    Ok(sqlx::query_as(&format!(
        "SELECT {VILLAGE_COLS} FROM villages
         WHERE ($1::UUID IS NULL OR federation_id = $1) AND ($2::UUID IS NULL OR political_entity_id = $2)
         ORDER BY code"
    ))
    .bind(f.federation)
    .bind(f.entity)
    .fetch_all(pool)
    .await?)
}

pub async fn active_villages(pool: &PgPool, federation: Uuid) -> Result<Vec<Village>> {
    Ok(sqlx::query_as(&format!("SELECT {VILLAGE_COLS} FROM villages WHERE federation_id = $1 AND active ORDER BY code"))
        .bind(federation)
        .fetch_all(pool)
        .await?)
}

pub async fn get_village(pool: &PgPool, id: Uuid) -> Result<Option<Village>> {
    Ok(sqlx::query_as(&format!("SELECT {VILLAGE_COLS} FROM villages WHERE id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await?)
}

pub async fn upsert_village(pool: &PgPool, v: &VillageInput) -> Result<Village> {
    let [x, y, z] = xyz(&v.position);
    Ok(sqlx::query_as(&format!(
        "INSERT INTO villages (id, code, name, federation_id, political_entity_id, village_type, population, radius_m, x, y, z)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
         ON CONFLICT (id) DO UPDATE SET
            code = $2, name = $3, federation_id = $4, political_entity_id = $5, village_type = $6,
            population = $7, radius_m = $8, x = $9, y = $10, z = $11, updated_at = now()
         RETURNING {VILLAGE_COLS}"
    ))
    .bind(v.id)
    .bind(&v.code)
    .bind(&v.name)
    .bind(v.federation_id)
    .bind(v.political_entity_id)
    .bind(&v.village_type)
    .bind(v.population)
    .bind(v.radius_m)
    .bind(x)
    .bind(y)
    .bind(z)
    .fetch_one(pool)
    .await?)
}

/// Sets the main warehouse when the village has none (or when `force`).
pub async fn set_main_warehouse(pool: &PgPool, village: Uuid, warehouse: Uuid, force: bool) -> Result<()> {
    sqlx::query(
        "UPDATE villages SET main_warehouse_id = $2, updated_at = now()
         WHERE id = $1 AND ($3 OR main_warehouse_id IS NULL)",
    )
    .bind(village)
    .bind(warehouse)
    .bind(force)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn set_village_active(pool: &PgPool, id: Uuid, active: bool) -> Result<bool> {
    Ok(sqlx::query("UPDATE villages SET active = $2, updated_at = now() WHERE id = $1")
        .bind(id)
        .bind(active)
        .execute(pool)
        .await?
        .rows_affected()
        > 0)
}

// ── Warehouses ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Warehouse {
    pub id: Uuid,
    pub village_id: Uuid,
    pub code: String,
    pub name: String,
    pub scenename: Option<String>,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub z: Option<f64>,
    pub active: bool,
}

const WAREHOUSE_COLS: &str = "w.id, w.village_id, w.code, w.name, w.scenename, w.x, w.y, w.z, w.active";

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WarehouseFilter {
    pub federation: Option<Uuid>,
    pub village: Option<Uuid>,
}

pub async fn list_warehouses(pool: &PgPool, f: &WarehouseFilter) -> Result<Vec<Warehouse>> {
    Ok(sqlx::query_as(&format!(
        "SELECT {WAREHOUSE_COLS} FROM warehouses w JOIN villages v ON v.id = w.village_id
         WHERE ($1::UUID IS NULL OR v.federation_id = $1) AND ($2::UUID IS NULL OR w.village_id = $2)
         ORDER BY w.code"
    ))
    .bind(f.federation)
    .bind(f.village)
    .fetch_all(pool)
    .await?)
}

/// Active warehouses of active villages of a federation.
pub async fn active_warehouses(pool: &PgPool, federation: Uuid) -> Result<Vec<Warehouse>> {
    Ok(sqlx::query_as(&format!(
        "SELECT {WAREHOUSE_COLS} FROM warehouses w JOIN villages v ON v.id = w.village_id
         WHERE v.federation_id = $1 AND v.active AND w.active ORDER BY w.code"
    ))
    .bind(federation)
    .fetch_all(pool)
    .await?)
}

pub async fn get_warehouse(pool: &PgPool, id: Uuid) -> Result<Option<Warehouse>> {
    Ok(sqlx::query_as(&format!("SELECT {WAREHOUSE_COLS} FROM warehouses w WHERE w.id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await?)
}

pub async fn upsert_warehouse(pool: &PgPool, id: Uuid, village: Uuid, code: &str, poi: &PoiCache) -> Result<Warehouse> {
    let [x, y, z] = xyz(&poi.position);
    Ok(sqlx::query_as(&format!(
        "INSERT INTO warehouses AS w (id, village_id, code, name, scenename, x, y, z)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         ON CONFLICT (id) DO UPDATE SET
            village_id = $2, name = $4, scenename = $5, x = $6, y = $7, z = $8, updated_at = now()
         RETURNING {WAREHOUSE_COLS}"
    ))
    .bind(id)
    .bind(village)
    .bind(code)
    .bind(&poi.name)
    .bind(&poi.scenename)
    .bind(x)
    .bind(y)
    .bind(z)
    .fetch_one(pool)
    .await?)
}

pub async fn set_warehouse_active(pool: &PgPool, id: Uuid, active: bool) -> Result<bool> {
    Ok(sqlx::query("UPDATE warehouses SET active = $2, updated_at = now() WHERE id = $1")
        .bind(id)
        .bind(active)
        .execute(pool)
        .await?
        .rows_affected()
        > 0)
}

// ── Routes ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct TravelRoute {
    pub id: Uuid,
    pub from_village_id: Uuid,
    pub to_village_id: Uuid,
    pub distance_m: f64,
    pub cost: f64,
    pub bidirectional: bool,
    pub active: bool,
    /// Created by the import (recomputed by it); `false` for an admin route.
    pub auto: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TravelRouteInput {
    pub from_village_id: Uuid,
    pub to_village_id: Uuid,
    pub distance_m: f64,
    /// Defaults to the distance in km.
    pub cost: Option<f64>,
    #[serde(default = "yes")]
    pub bidirectional: bool,
    #[serde(default = "yes")]
    pub active: bool,
}

fn yes() -> bool {
    true
}

const ROUTE_COLS: &str = "id, from_village_id, to_village_id, distance_m, cost, bidirectional, active, auto";

/// Routes of a federation (from one of its villages), or all routes.
pub async fn list_routes(pool: &PgPool, federation: Option<Uuid>) -> Result<Vec<TravelRoute>> {
    Ok(sqlx::query_as(&format!(
        "SELECT {ROUTE_COLS} FROM travel_routes r
         WHERE $1::UUID IS NULL OR EXISTS (
            SELECT 1 FROM villages v WHERE v.id = r.from_village_id AND v.federation_id = $1)
         ORDER BY created_at"
    ))
    .bind(federation)
    .fetch_all(pool)
    .await?)
}

pub async fn route_edges(pool: &PgPool, federation: Uuid) -> Result<Vec<RouteEdge>> {
    Ok(list_routes(pool, Some(federation))
        .await?
        .into_iter()
        .filter(|r| r.active)
        .map(|r| RouteEdge {
            from: r.from_village_id,
            to: r.to_village_id,
            cost: r.cost,
            distance_m: r.distance_m,
            bidirectional: r.bidirectional,
        })
        .collect())
}

/// Admin route: created or replaced, and no longer managed by the import.
pub async fn upsert_route(pool: &PgPool, r: &TravelRouteInput) -> Result<TravelRoute> {
    Ok(sqlx::query_as(&format!(
        "INSERT INTO travel_routes (from_village_id, to_village_id, distance_m, cost, bidirectional, active, auto)
         VALUES ($1, $2, $3, COALESCE($4, $3 / 1000.0), $5, $6, FALSE)
         ON CONFLICT (from_village_id, to_village_id) DO UPDATE SET
            distance_m = $3, cost = COALESCE($4, $3 / 1000.0), bidirectional = $5, active = $6, auto = FALSE, updated_at = now()
         RETURNING {ROUTE_COLS}"
    ))
    .bind(r.from_village_id)
    .bind(r.to_village_id)
    .bind(r.distance_m)
    .bind(r.cost)
    .bind(r.bidirectional)
    .bind(r.active)
    .fetch_one(pool)
    .await?)
}

/// Import: creates or updates an automatic route between two villages, unless an admin route
/// already links them (in either direction). Returns `true` when created.
pub async fn ensure_auto_route(pool: &PgPool, a: Uuid, b: Uuid, distance_m: f64, cost: Option<f64>) -> Result<bool> {
    let existing: Option<(Uuid, bool)> = sqlx::query_as(
        "SELECT id, auto FROM travel_routes WHERE (from_village_id = $1 AND to_village_id = $2)
                                              OR (from_village_id = $2 AND to_village_id = $1) LIMIT 1",
    )
    .bind(a)
    .bind(b)
    .fetch_optional(pool)
    .await?;
    match existing {
        Some((_, false)) => Ok(false),
        Some((id, true)) => {
            sqlx::query("UPDATE travel_routes SET distance_m = $2, cost = COALESCE($3, $2 / 1000.0), updated_at = now() WHERE id = $1")
                .bind(id)
                .bind(distance_m)
                .bind(cost)
                .execute(pool)
                .await?;
            Ok(false)
        }
        None => {
            sqlx::query(
                "INSERT INTO travel_routes (from_village_id, to_village_id, distance_m, cost, bidirectional, active, auto)
                 VALUES ($1, $2, $3, COALESCE($4, $3 / 1000.0), TRUE, TRUE, TRUE)",
            )
            .bind(a)
            .bind(b)
            .bind(distance_m)
            .bind(cost)
            .execute(pool)
            .await?;
            Ok(true)
        }
    }
}

/// Import: removes the automatic routes of a federation that are not in `keep` (unordered
/// village pairs). Returns how many were removed.
pub async fn prune_auto_routes(pool: &PgPool, federation: Uuid, keep: &std::collections::HashSet<(Uuid, Uuid)>) -> Result<usize> {
    let mut removed = 0;
    for r in list_routes(pool, Some(federation)).await?.into_iter().filter(|r| r.auto) {
        let pair = if r.from_village_id < r.to_village_id { (r.from_village_id, r.to_village_id) } else { (r.to_village_id, r.from_village_id) };
        if !keep.contains(&pair) {
            removed += delete_route(pool, r.id).await? as usize;
        }
    }
    Ok(removed)
}

/// Whether a route already links two villages, in either direction.
pub async fn route_exists(pool: &PgPool, a: Uuid, b: Uuid) -> Result<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM travel_routes WHERE (from_village_id = $1 AND to_village_id = $2)
                                                       OR (from_village_id = $2 AND to_village_id = $1))",
    )
    .bind(a)
    .bind(b)
    .fetch_one(pool)
    .await?)
}

pub async fn delete_route(pool: &PgPool, id: Uuid) -> Result<bool> {
    Ok(sqlx::query("DELETE FROM travel_routes WHERE id = $1").bind(id).execute(pool).await?.rows_affected() > 0)
}

// ── Village policies ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VillagePolicy {
    pub village_id: Uuid,
    pub resource_id: Uuid,
    pub min_threshold: i64,
    pub target_level: i64,
}

const POLICY_COLS: &str = "village_id, resource_id, min_threshold, target_level";

pub async fn list_policies(pool: &PgPool, village: Option<Uuid>, federation: Option<Uuid>) -> Result<Vec<VillagePolicy>> {
    Ok(sqlx::query_as(&format!(
        "SELECT {POLICY_COLS} FROM village_policies p
         WHERE ($1::UUID IS NULL OR village_id = $1)
           AND ($2::UUID IS NULL OR EXISTS (SELECT 1 FROM villages v WHERE v.id = p.village_id AND v.federation_id = $2 AND v.active))"
    ))
    .bind(village)
    .bind(federation)
    .fetch_all(pool)
    .await?)
}

pub async fn get_policy(pool: &PgPool, village: Uuid, resource: Uuid) -> Result<Option<VillagePolicy>> {
    Ok(sqlx::query_as(&format!("SELECT {POLICY_COLS} FROM village_policies WHERE village_id = $1 AND resource_id = $2"))
        .bind(village)
        .bind(resource)
        .fetch_optional(pool)
        .await?)
}

pub async fn upsert_policy(pool: &PgPool, p: &VillagePolicy) -> Result<VillagePolicy> {
    Ok(sqlx::query_as(&format!(
        "INSERT INTO village_policies (village_id, resource_id, min_threshold, target_level) VALUES ($1, $2, $3, $4)
         ON CONFLICT (village_id, resource_id) DO UPDATE SET min_threshold = $3, target_level = $4, updated_at = now()
         RETURNING {POLICY_COLS}"
    ))
    .bind(p.village_id)
    .bind(p.resource_id)
    .bind(p.min_threshold)
    .bind(p.target_level)
    .fetch_one(pool)
    .await?)
}

/// Inserts a policy only when the village has none for this resource (import defaults never
/// overwrite what an admin set). Returns `true` when inserted.
pub async fn insert_policy_if_missing(pool: &PgPool, p: &VillagePolicy) -> Result<bool> {
    Ok(sqlx::query(
        "INSERT INTO village_policies (village_id, resource_id, min_threshold, target_level) VALUES ($1, $2, $3, $4)
         ON CONFLICT (village_id, resource_id) DO NOTHING",
    )
    .bind(p.village_id)
    .bind(p.resource_id)
    .bind(p.min_threshold)
    .bind(p.target_level)
    .execute(pool)
    .await?
    .rows_affected()
        > 0)
}

pub async fn delete_policy(pool: &PgPool, village: Uuid, resource: Uuid) -> Result<bool> {
    Ok(sqlx::query("DELETE FROM village_policies WHERE village_id = $1 AND resource_id = $2")
        .bind(village)
        .bind(resource)
        .execute(pool)
        .await?
        .rows_affected()
        > 0)
}

// ── Warehouse capacities ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct WarehouseCapacity {
    pub warehouse_id: Uuid,
    pub resource_id: Uuid,
    pub capacity: Option<i64>,
    pub initial_stock: i64,
    pub initial_stock_applied: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WarehouseCapacityInput {
    pub warehouse_id: Uuid,
    pub resource_id: Uuid,
    /// `null`: unlimited.
    pub capacity: Option<i64>,
}

const CAPACITY_COLS: &str = "warehouse_id, resource_id, capacity, initial_stock, initial_stock_applied";

pub async fn list_capacities(pool: &PgPool, warehouse: Option<Uuid>, village: Option<Uuid>, federation: Option<Uuid>) -> Result<Vec<WarehouseCapacity>> {
    Ok(sqlx::query_as(&format!(
        "SELECT {CAPACITY_COLS} FROM warehouse_capacities c
         WHERE ($1::UUID IS NULL OR warehouse_id = $1)
           AND ($2::UUID IS NULL OR EXISTS (SELECT 1 FROM warehouses w WHERE w.id = c.warehouse_id AND w.village_id = $2))
           AND ($3::UUID IS NULL OR EXISTS (SELECT 1 FROM warehouses w JOIN villages v ON v.id = w.village_id
                                             WHERE w.id = c.warehouse_id AND v.federation_id = $3))"
    ))
    .bind(warehouse)
    .bind(village)
    .bind(federation)
    .fetch_all(pool)
    .await?)
}

pub async fn get_capacity(pool: &PgPool, warehouse: Uuid, resource: Uuid) -> Result<Option<i64>> {
    Ok(sqlx::query_scalar::<_, Option<i64>>("SELECT capacity FROM warehouse_capacities WHERE warehouse_id = $1 AND resource_id = $2")
        .bind(warehouse)
        .bind(resource)
        .fetch_optional(pool)
        .await?
        .flatten())
}

pub async fn upsert_capacity(pool: &PgPool, c: &WarehouseCapacityInput) -> Result<WarehouseCapacity> {
    Ok(sqlx::query_as(&format!(
        "INSERT INTO warehouse_capacities (warehouse_id, resource_id, capacity) VALUES ($1, $2, $3)
         ON CONFLICT (warehouse_id, resource_id) DO UPDATE SET capacity = $3, updated_at = now()
         RETURNING {CAPACITY_COLS}"
    ))
    .bind(c.warehouse_id)
    .bind(c.resource_id)
    .bind(c.capacity)
    .fetch_one(pool)
    .await?)
}

/// Import: capacity and initial stock, set only when the row does not exist yet.
pub async fn insert_capacity_if_missing(pool: &PgPool, warehouse: Uuid, resource: Uuid, capacity: Option<i64>, initial_stock: i64) -> Result<WarehouseCapacity> {
    sqlx::query(
        "INSERT INTO warehouse_capacities (warehouse_id, resource_id, capacity, initial_stock) VALUES ($1, $2, $3, $4)
         ON CONFLICT (warehouse_id, resource_id) DO NOTHING",
    )
    .bind(warehouse)
    .bind(resource)
    .bind(capacity)
    .bind(initial_stock)
    .execute(pool)
    .await?;
    Ok(sqlx::query_as(&format!("SELECT {CAPACITY_COLS} FROM warehouse_capacities WHERE warehouse_id = $1 AND resource_id = $2"))
        .bind(warehouse)
        .bind(resource)
        .fetch_one(pool)
        .await?)
}

/// Import override: capacity always set; the initial stock only while not yet credited.
pub async fn force_capacity(pool: &PgPool, warehouse: Uuid, resource: Uuid, capacity: Option<i64>, initial_stock: i64) -> Result<WarehouseCapacity> {
    Ok(sqlx::query_as(&format!(
        "INSERT INTO warehouse_capacities (warehouse_id, resource_id, capacity, initial_stock) VALUES ($1, $2, $3, $4)
         ON CONFLICT (warehouse_id, resource_id) DO UPDATE SET capacity = $3,
            initial_stock = CASE WHEN warehouse_capacities.initial_stock_applied THEN warehouse_capacities.initial_stock ELSE $4 END,
            updated_at = now()
         RETURNING {CAPACITY_COLS}"
    ))
    .bind(warehouse)
    .bind(resource)
    .bind(capacity)
    .bind(initial_stock)
    .fetch_one(pool)
    .await?)
}

pub async fn mark_initial_stock_applied(pool: &PgPool, warehouse: Uuid, resource: Uuid) -> Result<()> {
    sqlx::query("UPDATE warehouse_capacities SET initial_stock_applied = TRUE WHERE warehouse_id = $1 AND resource_id = $2")
        .bind(warehouse)
        .bind(resource)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete_capacity(pool: &PgPool, warehouse: Uuid, resource: Uuid) -> Result<bool> {
    Ok(sqlx::query("DELETE FROM warehouse_capacities WHERE warehouse_id = $1 AND resource_id = $2")
        .bind(warehouse)
        .bind(resource)
        .execute(pool)
        .await?
        .rows_affected()
        > 0)
}

// ── Flows ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Flow {
    pub id: Uuid,
    pub village_id: Uuid,
    pub resource_id: Uuid,
    pub rate_per_hour: f64,
    pub carry: f64,
    pub source: String,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FlowInput {
    pub village_id: Uuid,
    pub resource_id: Uuid,
    pub rate_per_hour: f64,
    #[serde(default = "manual")]
    pub source: String,
    pub label: Option<String>,
}

fn manual() -> String {
    "manual".into()
}

const FLOW_COLS: &str = "id, village_id, resource_id, rate_per_hour, carry, source, label";

pub async fn list_flows(pool: &PgPool, village: Option<Uuid>, federation: Option<Uuid>) -> Result<Vec<Flow>> {
    Ok(sqlx::query_as(&format!(
        "SELECT {FLOW_COLS} FROM village_flows f
         WHERE ($1::UUID IS NULL OR village_id = $1)
           AND ($2::UUID IS NULL OR EXISTS (SELECT 1 FROM villages v WHERE v.id = f.village_id AND v.federation_id = $2 AND v.active))
         ORDER BY created_at"
    ))
    .bind(village)
    .bind(federation)
    .fetch_all(pool)
    .await?)
}

pub async fn create_flow(pool: &PgPool, f: &FlowInput) -> Result<Flow> {
    Ok(sqlx::query_as(&format!(
        "INSERT INTO village_flows (village_id, resource_id, rate_per_hour, source, label)
         VALUES ($1, $2, $3, $4, $5) RETURNING {FLOW_COLS}"
    ))
    .bind(f.village_id)
    .bind(f.resource_id)
    .bind(f.rate_per_hour)
    .bind(&f.source)
    .bind(&f.label)
    .fetch_one(pool)
    .await?)
}

/// Import: one flow per (village, resource, source), created only when missing.
pub async fn insert_flow_if_missing(pool: &PgPool, f: &FlowInput) -> Result<bool> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM village_flows WHERE village_id = $1 AND resource_id = $2 AND source = $3)",
    )
    .bind(f.village_id)
    .bind(f.resource_id)
    .bind(&f.source)
    .fetch_one(pool)
    .await?;
    if exists {
        return Ok(false);
    }
    create_flow(pool, f).await?;
    Ok(true)
}

/// Updates a flow. `label`: absent keeps it, `Some(None)` (JSON null or "") clears it.
pub async fn update_flow(pool: &PgPool, id: Uuid, rate_per_hour: Option<f64>, label: Option<Option<String>>) -> Result<Option<Flow>> {
    let (set_label, label) = match label {
        None => (false, None),
        Some(l) => (true, l.filter(|s| !s.trim().is_empty())),
    };
    Ok(sqlx::query_as(&format!(
        "UPDATE village_flows SET rate_per_hour = COALESCE($2, rate_per_hour),
            label = CASE WHEN $3 THEN $4 ELSE label END, updated_at = now()
         WHERE id = $1 RETURNING {FLOW_COLS}"
    ))
    .bind(id)
    .bind(rate_per_hour)
    .bind(set_label)
    .bind(label)
    .fetch_optional(pool)
    .await?)
}

pub async fn set_flow_carry(tx: &mut Transaction<'_, Postgres>, id: Uuid, carry: f64) -> Result<()> {
    sqlx::query("UPDATE village_flows SET carry = $2 WHERE id = $1").bind(id).bind(carry).execute(&mut **tx).await?;
    Ok(())
}

pub async fn delete_flow(pool: &PgPool, id: Uuid) -> Result<bool> {
    Ok(sqlx::query("DELETE FROM village_flows WHERE id = $1").bind(id).execute(pool).await?.rows_affected() > 0)
}

// ── Mineral sources ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct MineralSource {
    pub id: Uuid,
    pub village_id: Uuid,
    pub resource_id: Uuid,
    pub remaining_units: Option<i64>,
    pub max_units_per_mission: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MineralSourceInput {
    pub village_id: Uuid,
    pub resource_id: Uuid,
    pub remaining_units: Option<i64>,
    pub max_units_per_mission: i64,
}

const SOURCE_COLS: &str = "id, village_id, resource_id, remaining_units, max_units_per_mission";

pub async fn list_mineral_sources(pool: &PgPool, federation: Option<Uuid>, village: Option<Uuid>) -> Result<Vec<MineralSource>> {
    Ok(sqlx::query_as(&format!(
        "SELECT {SOURCE_COLS} FROM mineral_sources s
         WHERE ($1::UUID IS NULL OR EXISTS (SELECT 1 FROM villages v WHERE v.id = s.village_id AND v.federation_id = $1))
           AND ($2::UUID IS NULL OR village_id = $2)
         ORDER BY created_at"
    ))
    .bind(federation)
    .bind(village)
    .fetch_all(pool)
    .await?)
}

pub async fn upsert_mineral_source(pool: &PgPool, s: &MineralSourceInput) -> Result<MineralSource> {
    Ok(sqlx::query_as(&format!(
        "INSERT INTO mineral_sources (village_id, resource_id, remaining_units, max_units_per_mission)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (village_id, resource_id) DO UPDATE SET
            remaining_units = $3, max_units_per_mission = $4, updated_at = now()
         RETURNING {SOURCE_COLS}"
    ))
    .bind(s.village_id)
    .bind(s.resource_id)
    .bind(s.remaining_units)
    .bind(s.max_units_per_mission)
    .fetch_one(pool)
    .await?)
}

pub async fn insert_mineral_source_if_missing(pool: &PgPool, s: &MineralSourceInput) -> Result<bool> {
    Ok(sqlx::query(
        "INSERT INTO mineral_sources (village_id, resource_id, remaining_units, max_units_per_mission)
         VALUES ($1, $2, $3, $4) ON CONFLICT (village_id, resource_id) DO NOTHING",
    )
    .bind(s.village_id)
    .bind(s.resource_id)
    .bind(s.remaining_units)
    .bind(s.max_units_per_mission)
    .execute(pool)
    .await?
    .rows_affected()
        > 0)
}

pub async fn consume_mineral_source(tx: &mut Transaction<'_, Postgres>, village: Uuid, resource: Uuid, units: i64) -> Result<()> {
    sqlx::query(
        "UPDATE mineral_sources SET remaining_units = GREATEST(remaining_units - $3, 0), updated_at = now()
         WHERE village_id = $1 AND resource_id = $2 AND remaining_units IS NOT NULL",
    )
    .bind(village)
    .bind(resource)
    .bind(units)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn delete_mineral_source(pool: &PgPool, id: Uuid) -> Result<bool> {
    Ok(sqlx::query("DELETE FROM mineral_sources WHERE id = $1").bind(id).execute(pool).await?.rows_affected() > 0)
}

// ── Observed stock levels ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct StockLevel {
    pub warehouse_id: Uuid,
    pub village_id: Uuid,
    pub resource_id: Uuid,
    pub quantity: i64,
    pub held: i64,
    #[serde(with = "time::serde::rfc3339")]
    pub observed_at: OffsetDateTime,
}

impl StockLevel {
    pub fn available(&self) -> i64 {
        self.quantity - self.held
    }
}

pub async fn set_stock_level(pool: &PgPool, warehouse: Uuid, resource: Uuid, quantity: i64, held: i64) -> Result<()> {
    sqlx::query(
        "INSERT INTO stock_levels (warehouse_id, resource_id, quantity, held, observed_at) VALUES ($1, $2, $3, $4, now())
         ON CONFLICT (warehouse_id, resource_id) DO UPDATE SET quantity = $3, held = $4, observed_at = now()",
    )
    .bind(warehouse)
    .bind(resource)
    .bind(quantity)
    .bind(held)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_stock_levels(pool: &PgPool, federation: Option<Uuid>, village: Option<Uuid>) -> Result<Vec<StockLevel>> {
    Ok(sqlx::query_as(
        "SELECT s.warehouse_id, w.village_id, s.resource_id, s.quantity, s.held, s.observed_at FROM stock_levels s
         JOIN warehouses w ON w.id = s.warehouse_id JOIN villages v ON v.id = w.village_id
         WHERE w.active AND ($1::UUID IS NULL OR v.federation_id = $1) AND ($2::UUID IS NULL OR w.village_id = $2)",
    )
    .bind(federation)
    .bind(village)
    .fetch_all(pool)
    .await?)
}

// ── Political cache ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct CachedEntity {
    pub id: Uuid,
    #[serde(rename = "type")]
    pub r#type: String,
    pub name: String,
    pub parent_id: Option<Uuid>,
    pub federation_id: Option<Uuid>,
}

pub async fn cache_entity(pool: &PgPool, e: &CachedEntity) -> Result<()> {
    sqlx::query(
        "INSERT INTO political_cache (id, type, name, parent_id, federation_id, refreshed_at) VALUES ($1, $2, $3, $4, $5, now())
         ON CONFLICT (id) DO UPDATE SET type = $2, name = $3, parent_id = $4, federation_id = $5, refreshed_at = now()",
    )
    .bind(e.id)
    .bind(&e.r#type)
    .bind(&e.name)
    .bind(e.parent_id)
    .bind(e.federation_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_cached_entities(pool: &PgPool, federation: Option<Uuid>) -> Result<Vec<CachedEntity>> {
    Ok(sqlx::query_as(
        "SELECT id, type, name, parent_id, federation_id FROM political_cache
         WHERE $1::UUID IS NULL OR federation_id = $1 ORDER BY name",
    )
    .bind(federation)
    .fetch_all(pool)
    .await?)
}

/// The entity and all its descendants (from the cache).
pub async fn entity_subtree(pool: &PgPool, root: Uuid) -> Result<Vec<Uuid>> {
    Ok(sqlx::query_scalar(
        "WITH RECURSIVE t(id) AS (
            SELECT id FROM political_cache WHERE id = $1
            UNION SELECT c.id FROM political_cache c JOIN t ON c.parent_id = t.id)
         SELECT id FROM t",
    )
    .bind(root)
    .fetch_all(pool)
    .await?)
}
