//! Read routes shared by players, services and admins.

use std::collections::{BTreeMap, HashMap};

use axum::{extract::{Path, Query, State}, Json};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::db::{catalog::{self, Federation}, history, orders::{self, OrderFilter}, world::{self, StockLevel, VillageFilter, VillagePolicy, WarehouseFilter}};
use crate::error::{AppError, Result};
use crate::simulation::{planner, stocks};
use crate::state::AppState;

/// The requested federation, or the first one.
pub async fn federation(st: &AppState, id: Option<Uuid>) -> Result<Federation> {
    match id {
        Some(id) => catalog::get_federation(&st.pool, id).await?.ok_or_else(|| AppError::NotFound(format!("federation {id}"))),
        None => catalog::list_federations(&st.pool).await?.into_iter().next().ok_or_else(|| AppError::NotFound("no federation".into())),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FederationQuery {
    pub federation: Option<Uuid>,
}

pub async fn federations(State(st): State<AppState>) -> Result<Json<Value>> {
    Ok(Json(json!({ "items": catalog::list_federations(&st.pool).await? })))
}

/// Political entities mirrored from social (federation, communes…).
pub async fn entities(State(st): State<AppState>, Query(q): Query<FederationQuery>) -> Result<Json<Value>> {
    Ok(Json(json!({ "items": world::list_cached_entities(&st.pool, q.federation).await? })))
}

pub async fn resources(State(st): State<AppState>) -> Result<Json<Value>> {
    Ok(Json(json!({ "items": catalog::list_resources(&st.pool).await? })))
}

pub async fn prices(State(st): State<AppState>, Query(q): Query<FederationQuery>) -> Result<Json<Value>> {
    let fed = federation(&st, q.federation).await?;
    Ok(Json(json!({ "federationId": fed.id, "items": history::price_indices(&st.pool, fed.id).await? })))
}

/// Stock of a village for a resource: sum of its warehouses, with its policy and net flow.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VillageStock {
    pub resource_id: Uuid,
    pub quantity: i64,
    pub held: i64,
    pub available: i64,
    pub min_threshold: Option<i64>,
    pub target_level: Option<i64>,
    pub net_flow_per_hour: f64,
    /// Hours until empty at the current net consumption (`null` when not consuming).
    pub autonomy_hours: Option<f64>,
}

/// Aggregates the observed levels of a set of villages per (village, resource).
pub fn village_stocks(levels: &[StockLevel], policies: &[VillagePolicy], flows: &[world::Flow]) -> HashMap<Uuid, Vec<VillageStock>> {
    let mut sums: BTreeMap<(Uuid, Uuid), (i64, i64)> = BTreeMap::new();
    for l in levels {
        let e = sums.entry((l.village_id, l.resource_id)).or_default();
        e.0 += l.quantity;
        e.1 += l.held;
    }
    for p in policies {
        sums.entry((p.village_id, p.resource_id)).or_default();
    }
    let mut net: HashMap<(Uuid, Uuid), f64> = HashMap::new();
    for f in flows {
        *net.entry((f.village_id, f.resource_id)).or_default() += f.rate_per_hour;
        sums.entry((f.village_id, f.resource_id)).or_default();
    }
    let policy: HashMap<_, _> = policies.iter().map(|p| ((p.village_id, p.resource_id), p)).collect();
    let mut out: HashMap<Uuid, Vec<VillageStock>> = HashMap::new();
    for ((v, r), (quantity, held)) in sums {
        let rate = net.get(&(v, r)).copied().unwrap_or(0.0);
        let available = quantity - held;
        out.entry(v).or_default().push(VillageStock {
            resource_id: r,
            quantity,
            held,
            available,
            min_threshold: policy.get(&(v, r)).map(|p| p.min_threshold),
            target_level: policy.get(&(v, r)).map(|p| p.target_level),
            net_flow_per_hour: rate,
            autonomy_hours: (rate < 0.0).then(|| available.max(0) as f64 / -rate),
        });
    }
    out
}

/// Villages with their aggregated stocks.
pub async fn villages(State(st): State<AppState>, Query(f): Query<VillageFilter>) -> Result<Json<Value>> {
    let villages = world::list_villages(&st.pool, &f).await?;
    let levels = world::list_stock_levels(&st.pool, f.federation, None).await?;
    let policies = world::list_policies(&st.pool, None, f.federation).await?;
    let flows = world::list_flows(&st.pool, None, f.federation).await?;
    let mut stocks = village_stocks(&levels, &policies, &flows);
    let mut counts: HashMap<Uuid, usize> = HashMap::new();
    for w in world::list_warehouses(&st.pool, &WarehouseFilter { federation: f.federation, ..Default::default() }).await? {
        if w.active {
            *counts.entry(w.village_id).or_default() += 1;
        }
    }
    let items: Vec<Value> = villages
        .into_iter()
        .map(|v| {
            let mut value = serde_json::to_value(&v).unwrap_or_default();
            value["warehouseCount"] = json!(counts.get(&v.id).copied().unwrap_or(0));
            value["stocks"] = json!(stocks.remove(&v.id).unwrap_or_default());
            value
        })
        .collect();
    Ok(Json(json!({ "items": items })))
}

/// A village: aggregated stocks (read live from inventory), its warehouses with their own
/// stocks and capacities, policies, flows, mineral sources, local prices and incoming legs.
pub async fn village(State(st): State<AppState>, Path(id): Path<Uuid>) -> Result<Json<Value>> {
    let v = world::get_village(&st.pool, id).await?.ok_or_else(|| AppError::NotFound(format!("village {id}")))?;
    let resources = catalog::list_resources(&st.pool).await?;
    let warehouses = stocks::village_warehouses(&st, id, v.main_warehouse_id).await?;
    let capacities = world::list_capacities(&st.pool, None, Some(id), None).await?;
    let policies = world::list_policies(&st.pool, Some(id), None).await?;
    let flows = world::list_flows(&st.pool, Some(id), None).await?;
    let mut levels = Vec::new();
    let mut per_warehouse = Vec::new();
    for w in &warehouses {
        let mut rows = Vec::new();
        for r in &resources {
            let level = stocks::refresh_one(&st, w.id, r.id, &r.inventory_good_type).await?;
            let capacity = capacities.iter().find(|c| c.warehouse_id == w.id && c.resource_id == r.id).and_then(|c| c.capacity);
            if level.quantity != 0 || capacity.is_some() {
                rows.push(json!({ "resourceId": r.id, "quantity": level.quantity, "held": level.held, "available": level.available(), "capacity": capacity }));
            }
            levels.push(StockLevel { warehouse_id: w.id, village_id: id, resource_id: r.id, quantity: level.quantity, held: level.held, observed_at: time::OffsetDateTime::now_utc() });
        }
        per_warehouse.push(json!({ "warehouse": w, "main": Some(w.id) == v.main_warehouse_id, "stocks": rows }));
    }
    let stocks = village_stocks(&levels, &policies, &flows).remove(&id).unwrap_or_default();
    let incoming: Vec<_> = orders::active_legs(&st.pool, v.federation_id).await?.into_iter().filter(|l| l.to_village_id == id).collect();
    Ok(Json(json!({
        "village": v,
        "stocks": stocks,
        "warehouses": per_warehouse,
        "policies": policies,
        "flows": flows,
        "capacities": capacities,
        "mineralSources": world::list_mineral_sources(&st.pool, None, Some(id)).await?,
        "localPrices": history::local_prices(&st.pool, v.federation_id, None, Some(id)).await?,
        "incoming": incoming,
    })))
}

pub async fn warehouses(State(st): State<AppState>, Query(f): Query<WarehouseFilter>) -> Result<Json<Value>> {
    Ok(Json(json!({ "items": world::list_warehouses(&st.pool, &f).await? })))
}

/// A warehouse with its live stocks and capacities.
pub async fn warehouse(State(st): State<AppState>, Path(id): Path<Uuid>) -> Result<Json<Value>> {
    let w = world::get_warehouse(&st.pool, id).await?.ok_or_else(|| AppError::NotFound(format!("warehouse {id}")))?;
    let capacities = world::list_capacities(&st.pool, Some(id), None, None).await?;
    let mut rows = Vec::new();
    for r in catalog::list_resources(&st.pool).await? {
        let level = stocks::refresh_one(&st, id, r.id, &r.inventory_good_type).await?;
        let capacity = capacities.iter().find(|c| c.resource_id == r.id).and_then(|c| c.capacity);
        if level.quantity != 0 || capacity.is_some() {
            rows.push(json!({ "resourceId": r.id, "code": r.code, "quantity": level.quantity, "held": level.held, "available": level.available(), "capacity": capacity }));
        }
    }
    let village = world::get_village(&st.pool, w.village_id).await?;
    Ok(Json(json!({ "warehouse": w, "village": village, "stocks": rows, "capacities": capacities })))
}

/// Stock of a political entity: villages whose commune is it or one of its descendants.
pub async fn entity_stock(State(st): State<AppState>, Path(id): Path<Uuid>) -> Result<Json<Value>> {
    let entities = world::entity_subtree(&st.pool, id).await?;
    if entities.is_empty() {
        return Err(AppError::NotFound(format!("entity {id}")));
    }
    let villages: Vec<_> = world::list_villages(&st.pool, &VillageFilter::default())
        .await?
        .into_iter()
        .filter(|v| v.active && entities.contains(&v.political_entity_id))
        .collect();
    let mut levels = Vec::new();
    let mut policies = Vec::new();
    let mut flows = Vec::new();
    for v in &villages {
        levels.extend(world::list_stock_levels(&st.pool, None, Some(v.id)).await?);
        policies.extend(world::list_policies(&st.pool, Some(v.id), None).await?);
        flows.extend(world::list_flows(&st.pool, Some(v.id), None).await?);
    }
    let mut totals: BTreeMap<Uuid, (i64, i64, i64, i64)> = BTreeMap::new();
    for stock in village_stocks(&levels, &policies, &flows).into_values().flatten() {
        let t = totals.entry(stock.resource_id).or_default();
        t.0 += stock.quantity;
        t.1 += stock.held;
        t.2 += stock.min_threshold.unwrap_or(0);
        t.3 += stock.target_level.unwrap_or(0);
    }
    let stocks: Vec<Value> = totals
        .into_iter()
        .map(|(r, (q, h, min, target))| json!({ "resourceId": r, "quantity": q, "held": h, "available": q - h, "minThreshold": min, "targetLevel": target }))
        .collect();
    Ok(Json(json!({ "entityId": id, "entities": entities, "villages": villages, "stocks": stocks })))
}

#[derive(Debug, Deserialize)]
pub struct PathQuery {
    pub from: Uuid,
    pub to: Uuid,
}

/// Shortest route between two villages.
pub async fn path(State(st): State<AppState>, Query(q): Query<PathQuery>) -> Result<Json<Value>> {
    let from = world::get_village(&st.pool, q.from).await?.ok_or_else(|| AppError::NotFound(format!("village {}", q.from)))?;
    let fed = federation(&st, Some(from.federation_id)).await?;
    let graph = planner::graph(&st, &fed).await?;
    let path = graph.shortest_path(q.from, q.to).ok_or_else(|| AppError::NotFound("no route between these villages".into()))?;
    let names: HashMap<Uuid, String> = world::active_villages(&st.pool, fed.id).await?.into_iter().map(|v| (v.id, v.name)).collect();
    let hops: Vec<Value> = path
        .hops
        .iter()
        .map(|h| json!({ "from": h.from, "fromName": names.get(&h.from), "to": h.to, "toName": names.get(&h.to), "cost": h.cost, "distanceM": h.distance_m }))
        .collect();
    Ok(Json(json!({ "hops": hops, "totalCost": path.total_cost, "totalDistanceM": path.total_distance_m })))
}

pub async fn orders(State(st): State<AppState>, Query(f): Query<OrderFilter>) -> Result<Json<Value>> {
    let (items, total) = orders::list_orders(&st.pool, &f).await?;
    let ids: Vec<Uuid> = items.iter().map(|o| o.id).collect();
    let mut legs: HashMap<Uuid, Vec<orders::Leg>> = HashMap::new();
    for l in orders::legs_of_orders(&st.pool, &ids).await? {
        legs.entry(l.order_id).or_default().push(l);
    }
    let items: Vec<Value> = items
        .into_iter()
        .map(|o| {
            let mut v = serde_json::to_value(&o).unwrap_or_default();
            v["legs"] = json!(legs.remove(&o.id).unwrap_or_default());
            v
        })
        .collect();
    Ok(Json(json!({ "items": items, "total": total, "limit": f.limit.unwrap_or(50).clamp(1, 200), "offset": f.offset.unwrap_or(0).max(0) })))
}

pub async fn order(State(st): State<AppState>, Path(id): Path<Uuid>) -> Result<Json<Value>> {
    let order = orders::get_order(&st.pool, id).await?.ok_or_else(|| AppError::NotFound(format!("order {id}")))?;
    let mut v = serde_json::to_value(&order).unwrap_or_default();
    v["legs"] = json!(orders::order_legs(&st.pool, id).await?);
    Ok(Json(v))
}
