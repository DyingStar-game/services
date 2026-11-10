//! Publishes pending legs: check the goods are there, price the leg, create the mission escrowed on
//! the federation treasury (ADR 0007, 0008, 0009).
//!
//! The goods are NOT reserved any more (no inventory hold): players move them physically — a crate
//! taken off a shelf of the source warehouse, shelved in the destination's — and the mission counts
//! those moves (`via: warehouse`). It is turned in at the town hall of the destination village.

use serde_json::{json, Value};
use tracing::{info, warn};
use uuid::Uuid;

use super::tiers::{self, Tier};
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
    let names = village_names(st, fed, &leg, input.id != resource.id).await?;
    let reward = reward(st, fed, &order, &leg, &resource).await?;
    let from_poi = match &from {
        Some(w) => Some(warehouse_poi(st, fed, w).await?),
        None => None,
    };
    let to_poi = warehouse_poi(st, fed, &to).await?;
    // Shown only in the villages it concerns: where the goods are loaded, where they go.
    let mut zones = Vec::new();
    for village in leg.from_village_id.into_iter().chain([leg.to_village_id]) {
        if let Some(poi) = village_poi(st, fed, village).await? {
            zones.push(poi);
        }
    }
    let mut mission = build_mission(fed, &order, &leg, (&resource, &input), from.as_ref().zip(from_poi), (&to, to_poi), &names, reward);
    mission.zones = zones_of(&zones);
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

/// Village names of a leg, for mission texts: (from, to, refinery) -- the refinery, asked for only when
/// the goods are [refined] on the way, being the destination when it refines, else the refining village
/// nearest to it (None when the federation has none).
async fn village_names(st: &AppState, fed: &Federation, leg: &Leg, refined: bool) -> Result<(Option<String>, String, Option<String>)> {
    let to_village = world::get_village(&st.pool, leg.to_village_id).await?;
    let refinery = match &to_village {
        Some(_) if !refined => None,
        Some(v) if v.refines => Some(v.name.clone()),
        Some(v) => nearest_refinery(&world::active_villages(&st.pool, fed.id).await?, v),
        None => None,
    };
    let to = to_village.map(|v| v.name).unwrap_or_default();
    let from = match leg.from_village_id {
        Some(id) => world::get_village(&st.pool, id).await?.map(|v| v.name),
        None => None,
    };
    Ok((from, to, refinery))
}

/// The name of the refining village of [villages] nearest to [to] (the first one when positions are unknown).
pub fn nearest_refinery(villages: &[world::Village], to: &world::Village) -> Option<String> {
    let distance = |v: &world::Village| match (v.x, v.y, v.z, to.x, to.y, to.z) {
        (Some(x), Some(y), Some(z), Some(tx), Some(ty), Some(tz)) => ((x - tx).powi(2) + (y - ty).powi(2) + (z - tz).powi(2)).sqrt(),
        _ => f64::MAX,
    };
    villages.iter().filter(|v| v.refines).min_by(|a, b| distance(a).total_cmp(&distance(b))).map(|v| v.name.clone())
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

/// The POI of a village in `inventory` (its `metadata.persistence` the village's uuid), created over the
/// village's radius when there is none yet: a mission zoned on it shows to the players in the village,
/// at its town hall. None when the village has no position (its missions then show everywhere).
async fn village_poi(st: &AppState, fed: &Federation, village_id: Uuid) -> Result<Option<Uuid>> {
    if let Some(id) = st.clients.inventory.find_poi_by_persistence(village_id).await? {
        return Ok(Some(id));
    }
    let Some(v) = world::get_village(&st.pool, village_id).await? else { return Ok(None) };
    let (Some(x), Some(y), Some(z)) = (v.x, v.y, v.z) else { return Ok(None) };
    let poi = NewPoi {
        owner_type: "political",
        owner_id: fed.political_entity_id,
        name: v.name.clone(),
        scene: fed.planet_internal_name.clone(),
        persistence_id: v.id,
        x,
        y,
        z,
        radius_m: v.radius_m.filter(|r| *r > 0.0).unwrap_or(VILLAGE_POI_RADIUS_M),
    };
    let id = st.clients.inventory.create_poi(&poi).await?;
    info!(village = %v.code, poi = %id, "village POI created");
    Ok(Some(id))
}

/// Radius of a village's POI when persistence gives none.
const VILLAGE_POI_RADIUS_M: f64 = 300.0;

/// The mission zones of these POIs, each once, in order.
pub fn zones_of(pois: &[Uuid]) -> Vec<Value> {
    let mut seen = Vec::new();
    for poi in pois {
        if !seen.contains(poi) {
            seen.push(*poi);
        }
    }
    seen.into_iter().map(|id| json!({ "kind": "poi", "poiId": id })).collect()
}

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
    (from_village, to_village, refinery): &(Option<String>, String, Option<String>),
    reward: i64,
) -> NewMission {
    // `refineAt`: where the ore is refined on the way (the game lists it as a step); absent when nothing is.
    let refined = input.id != resource.id;
    let mut payload = json!({ "source": "universesimulator", "orderId": order.id, "legId": leg.id, "seq": leg.seq, "resource": resource.code });
    if let (true, Some(name)) = (refined, refinery) {
        payload["refineAt"] = json!(name);
    }
    // Where to refine, as the texts say it: the refining village, else any factory city.
    let refine_en = refinery.as_ref().map_or("in a factory city".to_string(), |r| format!("at {r}"));
    let refine_fr = refinery.as_ref().map_or("dans une ville usine".to_string(), |r| format!("à {r}"));
    let zone = json!({ "kind": "poi", "poiId": to_poi });
    let fr = |r: &Resource| r.name_fr.clone().unwrap_or_else(|| r.name.clone());
    // "Mine gold, crush it": the mineral is mined, then crushed -- crushed ore is not mined (playtest
    // 2026-11-10). A resource that does not say what is mined reads "Mine and crush".
    let mine_en = |r: &Resource| r.mined.as_ref().map_or("Mine and crush".to_string(), |m| format!("Mine {m}, crush it"));
    let mine_fr = |r: &Resource| r.mined_fr.as_ref().map_or("Minez et concassez".to_string(), |m| format!("Minez {m}, concassez-le"));
    let qty = quantity_text(leg.quantity, &resource.unit);
    // Each mission text in English and French: [title], then the objective's.
    let (title, title_fr, objective_fr, category, objective) = match (order.kind.as_str(), from) {
        ("transport", Some((from, from_poi))) => {
            let origin = from_village.as_deref().unwrap_or(&from.name);
            (
                if refined { format!("Refine and deliver {} {} → {}", qty, resource.name, to_village) } else { format!("Transport {} {} → {}", qty, resource.name, to_village) },
                if refined {
                    format!("Raffinage et livraison : {} — {} → {}", qty, fr(resource), to_village)
                } else {
                    format!("Transport : {} — {} → {}", qty, fr(resource), to_village)
                },
                if !refined {
                    format!("Livrez {} — {} de {} à {}", qty, fr(resource), origin, to_village)
                } else {
                    format!("Chargez « {} » à l'entrepôt de {}, raffinez-le {}, livrez « {} » à l'entrepôt de {}", fr(input), origin, refine_fr, fr(resource), to_village)
                },
                "transport",
                json!({
                    "type": "transport",
                    "title": if !refined {
                        format!("Deliver {} of {} from {} to {}", quantity_text(leg.quantity, &resource.unit), resource.name, origin, to_village)
                    } else {
                        format!("Load {} at the warehouse of {}, refine it {}, deliver the {} to the warehouse of {}", input.name, origin, refine_en, resource.name, to_village)
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
            format!("Mine and deliver {} {} to {}", qty, resource.name, to_village),
            format!("Minage et livraison : {} — {} → {}", qty, fr(resource), to_village),
            format!("{} dans un village minier, faites-le raffiner {}, livrez « {} » à l'entrepôt de {}", mine_fr(input), refine_fr, fr(resource), to_village),
            "mining",
            json!({
                "type": "deliver_material",
                // Any mining village's crusher will do: the delivery counts the good, wherever it was
                // mined (the planner's source village is only its bookkeeping).
                "title": format!(
                    "{} in any mining village, have it refined {}, deliver the {} to the warehouse of {}",
                    mine_en(input), refine_en, resource.name, to_village
                ),
                "targetQuantity": leg.quantity,
                "unit": resource.unit,
                "locationTo": location(to, to_village),
                "params": { "goodType": resource.inventory_good_type, "zone": zone, "via": "warehouse" },
                "payload": payload,
            }),
        ),
        _ => (
            format!("Mine and crush {} {} for {}", qty, resource.name, to_village),
            format!("Minage et concassage : {} — {} pour {}", qty, fr(resource), to_village),
            format!("{}, puis rangez {} de « {} » à l'entrepôt de {}", mine_fr(resource), qty, fr(resource), to_village),
            "mining",
            json!({
                "type": "deliver_material",
                "title": format!("{}, then shelve {} of {} at the warehouse of {}", mine_en(resource), qty, resource.name, to_village),
                "targetQuantity": leg.quantity,
                "unit": resource.unit,
                "locationTo": location(to, to_village),
                // `mission`'s `deliver_material` kind (strict): the good, shelved in its warehouse.
                "params": { "goodType": resource.inventory_good_type, "zone": zone, "via": "warehouse" },
                "payload": payload,
            }),
        ),
    };
    let mut objective = objective;
    if let Some(o) = objective.as_object_mut() {
        let en = o.get("title").cloned().unwrap_or(Value::Null);
        o.insert("titleI18n".into(), json!({ "en": en, "fr": objective_fr }));
    }
    // The issuer's name alone: no word for what it is (a corporation, a federation...), and not the
    // order's trigger, an internal code players read as "(auto_mining)" (playtest 2026-11-10).
    let description = format!("{} supply order.", fed.name);
    // Its size (tiers): named first in its title, and how many may take it and for how long.
    let tier = Tier::of(leg.quantity, resource.truckload());
    let pace = tiers::pace_of(tier, leg.id.as_u128());
    let hours = tier.deadline_hours(pace, tiers::work_hours(order.kind.as_str(), refined));
    let reward = (reward as f64 * tier.reward_share(pace)).round() as i64;
    let (tier_en, tier_fr) = tier.label(pace);
    let title = format!("{tier_en} · {title}");
    let title_fr = format!("{tier_fr} · {title_fr}");
    NewMission {
        title_i18n: Some(json!({ "en": title, "fr": title_fr })),
        description_i18n: Some(json!({ "en": description, "fr": format!("Commande d'approvisionnement {}.", fed.name) })),
        title,
        description,
        kind: "dynamic",
        category,
        issuer_type: "politics",
        issuer_id: fed.political_entity_id,
        rewards: vec![json!({ "type": "credits", "currency": fed.currency, "amount": reward })],
        escrow: Some(json!({ "payerType": "politics", "payerId": fed.political_entity_id })),
        script_id: script_id(leg.id, leg.attempts),
        objectives: vec![objective],
        auto_complete: false,
        giver: giver_of(&fed.name, category),
        zones: Vec::new(),
        max_assignees: tier.max_assignees(),
        assignment_deadline_hours: Some(hours),
    }
}

/// The giver of a federation's mission: its name as the game's faction slug ("ARES" -> `ares`) and the
/// division of the job (`hauling` for a transport, `mining` otherwise). None when the name makes no slug.
pub fn giver_of(federation: &str, category: &str) -> Option<Value> {
    let faction: String = federation.trim().to_lowercase().chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-').collect();
    if faction.is_empty() || faction.len() > 32 || !faction.chars().next().is_some_and(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    let division = if category == "transport" { "hauling" } else { "mining" };
    Some(json!({ "faction": faction, "division": division }))
}

#[cfg(test)]
mod quantity_tests {
    use super::{quantity_text, zones_of};
    use serde_json::json;
    use uuid::Uuid;

    #[test]
    fn a_federation_s_mission_is_given_by_its_company() {
        assert_eq!(super::giver_of("ARES", "transport"), Some(serde_json::json!({ "faction": "ares", "division": "hauling" })));
        assert_eq!(super::giver_of("ARES", "mining"), Some(serde_json::json!({ "faction": "ares", "division": "mining" })));
        assert_eq!(super::giver_of("  ", "mining"), None);
    }

    #[test]
    fn a_mission_shows_once_in_each_village_it_concerns() {
        let (m, f) = (Uuid::new_v4(), Uuid::new_v4());
        let zones = zones_of(&[m, f, f]);
        assert_eq!(zones, vec![json!({ "kind": "poi", "poiId": m }), json!({ "kind": "poi", "poiId": f })]);
        assert!(zones_of(&[]).is_empty(), "no village known: shown everywhere");
    }

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
