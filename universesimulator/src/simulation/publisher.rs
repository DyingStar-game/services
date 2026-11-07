//! Publishes pending legs: check the goods are there, price the leg, create the mission escrowed on
//! the federation treasury (ADR 0007, 0008, 0009).
//!
//! The goods are NOT reserved any more (no inventory hold): players move them physically — a crate
//! taken off a shelf of the source warehouse, shelved in the destination's — and the mission counts
//! those moves (`via: warehouse`). It is turned in at the town hall of the destination village.

use serde_json::{json, Value};
use tracing::{info, warn};
use uuid::Uuid;

use super::{stocks, TickReport};
use crate::clients::{MissionSummary, NewMission, NewPoi, INSUFFICIENT_FUNDS_CODES};
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
    // The goods picked up: the input of a refined resource (crushed ore for ingots), else itself.
    let input = match &resource.refined_from {
        Some(code) => catalog::find_resource(&st.pool, Some(code), None).await?.unwrap_or_else(|| resource.clone()),
        None => resource.clone(),
    };

    let leg = if leg.status == "pending" {
        let to_village = world::get_village(&st.pool, leg.to_village_id).await?.ok_or_else(|| AppError::NotFound("village".into()))?;
        let to = to_village
            .main_warehouse_id
            .ok_or_else(|| AppError::conflict("NO_WAREHOUSE", format!("village {} has no warehouse", to_village.code)))?;
        // A transport picks its goods up at a warehouse that still has them; a mining leg (and the
        // mine of a mining_transport) has nothing to pick up.
        let from = match leg.from_village_id {
            Some(from_village) if order.kind == "transport" => {
                let Some(from) = source_warehouse(st, &order, leg, from_village, &input).await? else {
                    if leg.seq == 0 {
                        warn!(order = %order.id, "origin no longer has the goods; order failed");
                        orders::close_order(&st.pool, order.id, "failed", Some("origin stock insufficient")).await?;
                        return Ok(false);
                    }
                    return Err(AppError::conflict("INSUFFICIENT_GOODS", "no warehouse holds the goods of this leg"));
                };
                Some(from)
            }
            _ => None,
        };
        orders::set_leg_held(&st.pool, leg.id, None, from, to).await?;
        Leg { status: "held".into(), hold_id: None, from_warehouse_id: from, to_warehouse_id: Some(to), ..leg.clone() }
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
    let mission = build_mission(fed, &order, &leg, (&resource, &input), from.as_ref().zip(from_poi), (&to, to_poi), &names, reward);
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
        // Mining it, then carrying it the whole way: both rewards.
        "mining_transport" => {
            pricing::mining_reward(&params, leg.quantity, unit_price, urgency)
                + pricing::transport_reward(&params, leg.quantity, resource.mass_per_unit_kg, leg.distance_m, unit_price, urgency)
        }
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

/// A quantity in its resource's unit, as a player reads it: kilograms past 1 000 in tonnes, litres
/// past 1 000 in cubic metres (`kg` solids, `l` liquids, `m3` gases, `unit` things counted).
pub fn quantity_text(quantity: i64, unit: &str) -> String {
    let big = |q: i64, scale: &str| {
        let v = q as f64 / 1000.0;
        if v.fract() == 0.0 { format!("{v:.0} {scale}") } else { format!("{v:.1} {scale}") }
    };
    match unit {
        "kg" if quantity >= 1000 => big(quantity, "t"),
        "l" if quantity >= 1000 => big(quantity, "m³"),
        "l" => format!("{quantity} L"),
        "m3" => format!("{quantity} m³"),
        "unit" | "" => quantity.to_string(),
        _ => format!("{quantity} {unit}"),
    }
}

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

/// The mission of a leg. Three kinds, every one measured on the warehouses (`via: warehouse`: the
/// goods taken off a shelf, the goods shelved) and turned in at the destination's town hall:
///   - transport: load [input] at the source warehouse, deliver [resource] at the destination's
///     (the same good, or its refined form — crushed ore loaded, ingots delivered);
///   - mining: mine [resource] and deliver it at the destination's warehouse;
///   - mining_transport: mine [input] in any mining village (the planner's [from_village] is only
///     where its source is debited), deliver [resource] at the destination's warehouse.
// One value per part of the mission text and params: grouping them would only hide what each is.
#[allow(clippy::too_many_arguments)]
pub fn build_mission(
    fed: &Federation,
    order: &SupplyOrder,
    leg: &Leg,
    (resource, input): (&Resource, &Resource),
    from: Option<(&Warehouse, Uuid)>,
    (to, to_poi): (&Warehouse, Uuid),
    (from_village, to_village): &(Option<String>, String),
    reward: i64,
) -> NewMission {
    let payload = json!({ "source": "universesimulator", "orderId": order.id, "legId": leg.id, "seq": leg.seq, "resource": resource.code });
    let zone = json!({ "kind": "poi", "poiId": to_poi });
    let (title, category, objective) = match (order.kind.as_str(), from) {
        ("transport", Some((from, from_poi))) => {
            let origin = from_village.as_deref().unwrap_or(&from.name);
            (
                format!("Transport {} {} → {}", quantity_text(leg.quantity, &resource.unit), resource.name, to_village),
                "transport",
                json!({
                    "type": "transport",
                    "title": if input.id == resource.id {
                        format!("Deliver {} of {} from {} to {}", quantity_text(leg.quantity, &resource.unit), resource.name, origin, to_village)
                    } else {
                        format!("Load {} at {}, refine it at {}, deliver the {} to its warehouse", input.name, origin, to_village, resource.name)
                    },
                    "targetQuantity": leg.quantity,
                    "unit": resource.unit,
                    "locationFrom": location(from, from_village.as_deref().unwrap_or_default()),
                    "locationTo": location(to, to_village),
                    // `mission`'s `transport` kind (strict): picked up at the source warehouse,
                    // delivered (refined or not) into the destination's.
                    "params": {
                        "goodType": input.inventory_good_type,
                        "deliverGoodType": resource.inventory_good_type,
                        "source": { "kind": "poi", "poiId": from_poi },
                        "zone": zone,
                        "via": "warehouse",
                    },
                    "payload": payload,
                }),
            )
        }
        ("mining_transport", _) => (
            format!("Mine and deliver {} {} to {}", quantity_text(leg.quantity, &resource.unit), resource.name, to_village),
            "mining",
            json!({
                "type": "deliver_material",
                // Any mining village's crusher will do: the delivery counts the good, wherever it was
                // mined (the planner's source village is only its bookkeeping).
                "title": format!(
                    "Mine {} in any mining village, refine it, deliver the {} to the warehouse of {}",
                    input.name, resource.name, to_village
                ),
                "targetQuantity": leg.quantity,
                "unit": resource.unit,
                "locationTo": location(to, to_village),
                "params": { "goodType": resource.inventory_good_type, "zone": zone, "via": "warehouse" },
                "payload": payload,
            }),
        ),
        _ => (
            format!("Mine {} {} for {}", quantity_text(leg.quantity, &resource.unit), resource.name, to_village),
            "mining",
            json!({
                "type": "deliver_material",
                "title": format!("Mine {} of {} and bring it to the warehouse of {}", quantity_text(leg.quantity, &resource.unit), resource.name, to_village),
                "targetQuantity": leg.quantity,
                "unit": resource.unit,
                "locationTo": location(to, to_village),
                // `mission`'s `deliver_material` kind (strict): the good, shelved in its warehouse.
                "params": { "goodType": resource.inventory_good_type, "zone": zone, "via": "warehouse" },
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
        auto_complete: false,
    }
}

#[cfg(test)]
mod quantity_tests {
    use super::quantity_text;

    #[test]
    fn a_quantity_reads_in_its_unit() {
        assert_eq!(quantity_text(90, "kg"), "90 kg");
        assert_eq!(quantity_text(1500, "kg"), "1.5 t");
        assert_eq!(quantity_text(2000, "kg"), "2 t");
        assert_eq!(quantity_text(400, "l"), "400 L");
        assert_eq!(quantity_text(20000, "l"), "20 m³");
        assert_eq!(quantity_text(12, "m3"), "12 m³");
        assert_eq!(quantity_text(3, "unit"), "3");
    }
}
