//! Supervision views for the admin panel.

use std::collections::HashMap;

use axum::{extract::{Query, State}, Json};
use serde::Deserialize;
use serde_json::{json, Value};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use uuid::Uuid;

use super::reads::{federation, village_stocks, FederationQuery};
use crate::db::{history, orders, world};
use crate::error::{AppError, Result};
use crate::state::AppState;

fn since(from: Option<&str>) -> Result<OffsetDateTime> {
    match from {
        Some(s) => OffsetDateTime::parse(s, &Rfc3339).map_err(|_| AppError::BadRequest("from must be RFC 3339".into())),
        None => Ok(OffsetDateTime::now_utc() - time::Duration::hours(24)),
    }
}

pub async fn dashboard(State(st): State<AppState>, Query(q): Query<FederationQuery>) -> Result<Json<Value>> {
    let fed = federation(&st, q.federation).await?;
    let simulation = history::simulation_state(&st.pool).await?;
    // The panel must render even when economie is down.
    let treasury = st.clients.economie.political_balance(fed.political_entity_id, &fed.currency).await.ok();
    let minted_24h = history::minted_since(&st.pool, fed.id, OffsetDateTime::now_utc() - time::Duration::hours(24)).await?;
    let (_, open_orders) = orders::list_orders(&st.pool, &orders::OrderFilter { federation: Some(fed.id), status: Some("in_progress".into()), ..Default::default() }).await?;
    let legs = orders::active_legs(&st.pool, fed.id).await?;
    let published: Vec<_> = legs.iter().filter(|l| l.status == "published").collect();
    let escrowed: i64 = published.iter().filter_map(|l| l.reward_amount).sum();

    let levels = world::list_stock_levels(&st.pool, Some(fed.id), None).await?;
    let policies = world::list_policies(&st.pool, None, Some(fed.id)).await?;
    let flows = world::list_flows(&st.pool, None, Some(fed.id)).await?;
    let mut alerts = Vec::new();
    for (village, stocks) in village_stocks(&levels, &policies, &flows) {
        for s in stocks {
            if let Some(min) = s.min_threshold.filter(|min| s.available < *min) {
                alerts.push(json!({
                    "villageId": village,
                    "resourceId": s.resource_id,
                    "available": s.available,
                    "minThreshold": min,
                    "targetLevel": s.target_level,
                    "netFlowPerHour": s.net_flow_per_hour,
                    "autonomyHours": s.autonomy_hours,
                }));
            }
        }
    }
    Ok(Json(json!({
        "simulation": simulation,
        "federation": fed,
        "treasury": { "balance": treasury, "currency": fed.currency, "floor": fed.treasury_floor, "minted24h": minted_24h, "escrowed": escrowed },
        "openOrders": open_orders,
        "activeMissions": published.len(),
        "alerts": alerts,
    })))
}

/// Villages (with aggregated stocks and their warehouses' positions), routes and active legs.
pub async fn network(State(st): State<AppState>, Query(q): Query<FederationQuery>) -> Result<Json<Value>> {
    let fed = federation(&st, q.federation).await?;
    let villages = world::list_villages(&st.pool, &world::VillageFilter { federation: Some(fed.id), ..Default::default() }).await?;
    let levels = world::list_stock_levels(&st.pool, Some(fed.id), None).await?;
    let policies = world::list_policies(&st.pool, None, Some(fed.id)).await?;
    let flows = world::list_flows(&st.pool, None, Some(fed.id)).await?;
    let mut stocks = village_stocks(&levels, &policies, &flows);
    let mut warehouses: HashMap<Uuid, Vec<world::Warehouse>> = HashMap::new();
    for w in world::list_warehouses(&st.pool, &world::WarehouseFilter { federation: Some(fed.id), ..Default::default() }).await? {
        warehouses.entry(w.village_id).or_default().push(w);
    }
    let villages: Vec<Value> = villages
        .into_iter()
        .map(|v| {
            let mut value = serde_json::to_value(&v).unwrap_or_default();
            value["stocks"] = json!(stocks.remove(&v.id).unwrap_or_default());
            value["warehouses"] = json!(warehouses.remove(&v.id).unwrap_or_default());
            value
        })
        .collect();
    Ok(Json(json!({
        "federationId": fed.id,
        "villages": villages,
        "routes": world::list_routes(&st.pool, Some(fed.id)).await?,
        "activeLegs": orders::active_legs(&st.pool, fed.id).await?,
    })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockHistoryQuery {
    pub village: Option<Uuid>,
    pub warehouse: Option<Uuid>,
    pub resource: Uuid,
    pub from: Option<String>,
}

/// History of a village (sum of its warehouses) or of one warehouse.
pub async fn stock_history(State(st): State<AppState>, Query(q): Query<StockHistoryQuery>) -> Result<Json<Value>> {
    if q.village.is_none() && q.warehouse.is_none() {
        return Err(AppError::BadRequest("village or warehouse is required".into()));
    }
    let points = history::stock_history(&st.pool, q.warehouse, q.village, q.resource, since(q.from.as_deref())?).await?;
    let policy = match q.village {
        Some(v) => world::get_policy(&st.pool, v, q.resource).await?,
        None => None,
    };
    Ok(Json(json!({ "points": points, "minThreshold": policy.as_ref().map(|p| p.min_threshold), "targetLevel": policy.map(|p| p.target_level) })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PriceHistoryQuery {
    pub federation: Option<Uuid>,
    pub resource: Uuid,
    pub from: Option<String>,
}

pub async fn price_history(State(st): State<AppState>, Query(q): Query<PriceHistoryQuery>) -> Result<Json<Value>> {
    let fed = federation(&st, q.federation).await?;
    Ok(Json(json!({ "points": history::price_history(&st.pool, fed.id, q.resource, since(q.from.as_deref())?).await? })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalPricesQuery {
    pub federation: Option<Uuid>,
    pub resource: Option<Uuid>,
}

pub async fn local_prices(State(st): State<AppState>, Query(q): Query<LocalPricesQuery>) -> Result<Json<Value>> {
    let fed = federation(&st, q.federation).await?;
    Ok(Json(json!({ "items": history::local_prices(&st.pool, fed.id, q.resource, None).await? })))
}

#[derive(Debug, Deserialize)]
pub struct IncomingQuery {
    pub village: Uuid,
}

/// Goods on their way to a village (active legs ending there).
pub async fn incoming(State(st): State<AppState>, Query(q): Query<IncomingQuery>) -> Result<Json<Value>> {
    let v = world::get_village(&st.pool, q.village).await?.ok_or_else(|| AppError::NotFound(format!("village {}", q.village)))?;
    let items: Vec<_> = orders::active_legs(&st.pool, v.federation_id).await?.into_iter().filter(|l| l.to_village_id == v.id).collect();
    Ok(Json(json!({ "items": items })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MintQuery {
    pub federation: Option<Uuid>,
    pub limit: Option<i64>,
}

pub async fn mint_events(State(st): State<AppState>, Query(q): Query<MintQuery>) -> Result<Json<Value>> {
    let fed = federation(&st, q.federation).await?;
    Ok(Json(json!({ "items": history::mint_events(&st.pool, fed.id, q.limit.unwrap_or(100)).await? })))
}
