//! Publishes pending legs: reserve the goods (inventory hold), price the leg, create the mission
//! escrowed on the federation treasury (ADR 0007, 0008, 0009).

use serde_json::{json, Value};
use tracing::{info, warn};
use uuid::Uuid;

use super::{stocks, TickReport};
use crate::clients::{Holder, MissionSummary, NewMission, NewPoi, INSUFFICIENT_FUNDS_CODES};
use crate::db::{catalog::{self, Federation, Resource}, history, orders::{self, Leg, SupplyOrder}, world::{self, Warehouse}};
use crate::error::{AppError, Result};
use crate::pricing;
use crate::state::AppState;

/// Idempotency key of a leg's mission; a republication after a cancelled or expired mission
/// is a new attempt, hence a new mission.
pub fn script_id(leg: Uuid, attempt: i32) -> String {
    format!("usim:leg:{leg}:{attempt}")
}

/// Publishes the federation's pending and held legs. Returns `true` when the treasury could not
/// pay an escrow (the treasury step then mints).
pub async fn run(st: &AppState, fed: &Federation, report: &mut TickReport) -> Result<bool> {
    let mut funds_short = false;
    for leg in orders::legs_by_status(&st.pool, fed.id, &["pending", "held"]).await? {
        match publish(st, fed, &leg).await {
            Ok(true) => report.legs_published += 1,
            Ok(false) => {}
            Err(e) if e.upstream_code().is_some_and(|c| INSUFFICIENT_FUNDS_CODES.contains(&c)) => {
                funds_short = true;
                orders::set_leg_error(&st.pool, leg.id, "treasury insufficient").await?;
            }
            Err(e) => {
                orders::set_leg_error(&st.pool, leg.id, &e.to_string()).await?;
                report.error("publish leg", e);
            }
        }
    }
    Ok(funds_short)
}

/// Advances one leg; returns `true` when a mission was published.
async fn publish(st: &AppState, fed: &Federation, leg: &Leg) -> Result<bool> {
    let order = orders::get_order(&st.pool, leg.order_id).await?.ok_or_else(|| AppError::NotFound("order".into()))?;
    let resource = catalog::get_resource(&st.pool, order.resource_id).await?.ok_or_else(|| AppError::NotFound("resource".into()))?;

    let leg = if leg.status == "pending" {
        let to_village = world::get_village(&st.pool, leg.to_village_id).await?.ok_or_else(|| AppError::NotFound("village".into()))?;
        let to = to_village
            .main_warehouse_id
            .ok_or_else(|| AppError::conflict("NO_WAREHOUSE", format!("village {} has no warehouse", to_village.code)))?;
        let (hold, from) = match leg.from_village_id {
            None => (None, None),
            Some(from_village) => {
                let Some(from) = source_warehouse(st, &order, leg, from_village, &resource).await? else {
                    if leg.seq == 0 {
                        warn!(order = %order.id, "origin no longer has the goods; order failed");
                        orders::close_order(&st.pool, order.id, "failed", Some("origin stock insufficient")).await?;
                        return Ok(false);
                    }
                    return Err(AppError::conflict("INSUFFICIENT_GOODS", "no warehouse holds the goods of this leg"));
                };
                match st.clients.inventory.create_hold(Holder::warehouse(from), &resource.inventory_good_type, leg.quantity, &leg.id.to_string()).await {
                    Ok(id) => (Some(id), Some(from)),
                    Err(e) if e.upstream_code() == Some("INSUFFICIENT_GOODS") && leg.seq == 0 => {
                        warn!(order = %order.id, "origin no longer has the goods; order failed");
                        orders::close_order(&st.pool, order.id, "failed", Some("origin stock insufficient")).await?;
                        return Ok(false);
                    }
                    Err(e) => return Err(e),
                }
            }
        };
        orders::set_leg_held(&st.pool, leg.id, hold, from, to).await?;
        Leg { status: "held".into(), hold_id: hold, from_warehouse_id: from, to_warehouse_id: Some(to), ..leg.clone() }
    } else {
        leg.clone()
    };

    let to_id = leg.to_warehouse_id.ok_or_else(|| AppError::conflict("NO_WAREHOUSE", "leg without destination warehouse"))?;
    let to = world::get_warehouse(&st.pool, to_id).await?.ok_or_else(|| AppError::NotFound("warehouse".into()))?;
    let from = match leg.from_warehouse_id {
        Some(id) => world::get_warehouse(&st.pool, id).await?,
        None => None,
    };
    let names = village_names(st, &leg).await?;
    let reward = reward(st, fed, &order, &leg, &resource).await?;
    let from_poi = match &from {
        Some(w) => Some(warehouse_poi(st, fed, w).await?),
        None => None,
    };
    let to_poi = warehouse_poi(st, fed, &to).await?;
    let mission = build_mission(fed, &order, &leg, &resource, from.as_ref().zip(from_poi), (&to, to_poi), &names, reward);
    let created = match st.clients.mission.create(&mission).await {
        Ok(m) => m,
        Err(e) if e.upstream_code() == Some("DUPLICATE_SCRIPT_ID") => find_by_script(st, fed, &mission.script_id).await?,
        Err(e) => return Err(e),
    };
    orders::set_leg_published(&st.pool, leg.id, created.id, reward).await?;
    info!(order = %order.id, leg = leg.seq, mission = %created.id, reward, "leg published");
    Ok(true)
}

/// Warehouse the goods of a transport leg are taken from: where the previous leg delivered
/// them, or for the first leg the warehouse of the origin village holding the most.
async fn source_warehouse(st: &AppState, order: &SupplyOrder, leg: &Leg, village: Uuid, resource: &Resource) -> Result<Option<Uuid>> {
    if leg.seq > 0 {
        let previous = orders::order_legs(&st.pool, order.id).await?.into_iter().find(|l| l.seq == leg.seq - 1);
        return Ok(previous.and_then(|l| l.to_warehouse_id));
    }
    let mut best: Option<(Uuid, i64)> = None;
    for w in stocks::village_warehouses(st, village, None).await? {
        let available = stocks::refresh_one(st, w.id, resource.id, &resource.inventory_good_type).await?.available();
        if available >= leg.quantity && best.is_none_or(|(_, b)| available > b) {
            best = Some((w.id, available));
        }
    }
    Ok(best.map(|(w, _)| w))
}

/// Village names of a leg, for mission texts: (from, to).
async fn village_names(st: &AppState, leg: &Leg) -> Result<(Option<String>, String)> {
    let to = world::get_village(&st.pool, leg.to_village_id).await?.map(|v| v.name).unwrap_or_default();
    let from = match leg.from_village_id {
        Some(id) => world::get_village(&st.pool, id).await?.map(|v| v.name),
        None => None,
    };
    Ok((from, to))
}

/// A previous publication created the mission but its answer was lost.
async fn find_by_script(st: &AppState, fed: &Federation, script: &str) -> Result<MissionSummary> {
    for status in ["available", "active", "completed", "cancelled", "expired"] {
        let found = st.clients.mission.list("politics", fed.political_entity_id, status, None).await?;
        if let Some(m) = found.into_iter().find(|m| m.script_id.as_deref() == Some(script)) {
            return Ok(m);
        }
    }
    Err(AppError::conflict("DUPLICATE_SCRIPT_ID", format!("mission {script} exists but was not found")))
}

async fn reward(st: &AppState, fed: &Federation, order: &SupplyOrder, leg: &Leg, resource: &Resource) -> Result<i64> {
    let params = fed.pricing();
    let unit_price = history::price_indices(&st.pool, fed.id)
        .await?
        .into_iter()
        .find(|p| p.resource_id == resource.id)
        .map(|p| p.unit_price)
        .unwrap_or(resource.base_price);
    let dest = order.destination_village_id;
    let urgency = match world::get_policy(&st.pool, dest, resource.id).await? {
        Some(p) => {
            let available: i64 = world::list_stock_levels(&st.pool, Some(fed.id), Some(dest))
                .await?
                .into_iter()
                .filter(|l| l.resource_id == resource.id)
                .map(|l| l.available())
                .sum();
            pricing::urgency(&params, available, p.min_threshold)
        }
        None => 1.0,
    };
    Ok(match order.kind.as_str() {
        "mining" => pricing::mining_reward(&params, leg.quantity, unit_price, urgency),
        _ => pricing::transport_reward(&params, leg.quantity, resource.mass_per_unit_kg, leg.distance_m, unit_price, urgency),
    })
}

/// The POI `inventory` holds for a warehouse, created the first time a mission points at it:
/// `mission` resolves its objectives' places there, and `inventory` makes up its own ids.
async fn warehouse_poi(st: &AppState, fed: &Federation, w: &Warehouse) -> Result<Uuid> {
    if let Some(id) = w.inventory_poi_id {
        return Ok(id);
    }
    let (Some(x), Some(y), Some(z)) = (w.x, w.y, w.z) else {
        return Err(AppError::conflict("NO_POSITION", format!("warehouse {} has no position", w.code)));
    };
    let poi = NewPoi {
        owner_type: "political",
        owner_id: fed.political_entity_id,
        name: w.name.clone(),
        // Depots stand on the federation's planet, their position relative to its centre.
        scene: fed.planet_internal_name.clone(),
        persistence_id: w.id,
        x,
        y,
        z,
        radius_m: WAREHOUSE_POI_RADIUS_M,
    };
    let id = st.clients.inventory.create_poi(&poi).await?;
    world::set_warehouse_poi(&st.pool, w.id, id).await?;
    info!(warehouse = %w.code, poi = %id, "warehouse POI created");
    Ok(id)
}

/// Radius of a warehouse's POI: a cargo depot and its loading area.
const WAREHOUSE_POI_RADIUS_M: f64 = 40.0;

fn location(w: &Warehouse, village: &str) -> Value {
    json!({
        "warehouseId": w.id,
        "villageId": w.village_id,
        "village": village,
        "poiUuid": w.id,
        "name": w.name,
        "scenename": w.scenename,
        "position": match (w.x, w.y, w.z) { (Some(x), Some(y), Some(z)) => json!({ "x": x, "y": y, "z": z }), _ => Value::Null },
    })
}

pub fn build_mission(
    fed: &Federation,
    order: &SupplyOrder,
    leg: &Leg,
    resource: &Resource,
    from: Option<(&Warehouse, Uuid)>,
    (to, to_poi): (&Warehouse, Uuid),
    (from_village, to_village): &(Option<String>, String),
    reward: i64,
) -> NewMission {
    let payload = json!({ "source": "universesimulator", "orderId": order.id, "legId": leg.id, "seq": leg.seq, "resource": resource.code });
    let (title, category, objective) = match from {
        Some((from, from_poi)) => (
            format!("Transport {} {} {} → {}", leg.quantity, resource.unit, resource.name, to_village),
            "transport",
            json!({
                "type": "transport",
                "title": format!(
                    "Deliver {} {} of {} from {} to {}",
                    leg.quantity,
                    resource.unit,
                    resource.name,
                    from_village.as_deref().unwrap_or(&from.name),
                    to_village
                ),
                "targetQuantity": leg.quantity,
                "unit": resource.unit,
                "locationFrom": location(from, from_village.as_deref().unwrap_or_default()),
                "locationTo": location(to, to_village),
                // `mission`'s `transport` kind (strict): the good, its pickup and delivery POIs.
                "params": {
                    "goodType": resource.inventory_good_type,
                    "source": { "kind": "poi", "poiId": from_poi },
                    "zone": { "kind": "poi", "poiId": to_poi },
                },
                "payload": payload,
            }),
        ),
        None => (
            format!("Mine {} {} {} for {}", leg.quantity, resource.unit, resource.name, to_village),
            "mining",
            json!({
                "type": "deliver_material",
                "title": format!("Mine {} {} of {} and deliver it to {}", leg.quantity, resource.unit, resource.name, to_village),
                "targetQuantity": leg.quantity,
                "unit": resource.unit,
                "locationTo": location(to, to_village),
                // `mission`'s `deliver_material` kind (strict): the good and its delivery POI.
                "params": {
                    "goodType": resource.inventory_good_type,
                    "zone": { "kind": "poi", "poiId": to_poi },
                },
                "payload": payload,
            }),
        ),
    };
    NewMission {
        title,
        description: format!("Supply order of {} ({}).", fed.name, order.trigger),
        kind: "dynamic",
        category,
        issuer_type: "politics",
        issuer_id: fed.political_entity_id,
        rewards: vec![json!({ "type": "credits", "currency": fed.currency, "amount": reward })],
        escrow: Some(json!({ "payerType": "politics", "payerId": fed.political_entity_id })),
        script_id: script_id(leg.id, leg.attempts),
        objectives: vec![objective],
    }
}
