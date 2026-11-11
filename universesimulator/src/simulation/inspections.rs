//! Inspections (ADR 0019): each village's security keeps one mission open that sends a player to look at
//! a place 5 to 40 km away. The mission is the mission service's `visit`: being there is enough, and it
//! pays at once (no turn-in). Unlike the supply orders, nothing is moved and no stock is read.
//!
//! A village has SITES_PER_VILLAGE sites, made once from its position (the same village always gets the
//! same); the one inspected the longest ago gets the next mission.

use std::collections::{HashMap, HashSet};

use serde_json::json;
use tracing::info;
use uuid::Uuid;

use super::publisher::{find_by_script, giver_of, village_poi, zones_of};
use super::TickReport;
use crate::clients::{NewMission, NewPoi, INSUFFICIENT_FUNDS_CODES};
use crate::db::{catalog::Federation, inspections::{self, Site}, world::{self, Village}};
use crate::error::Result;
use crate::state::AppState;

/// Sites around a village.
pub const SITES_PER_VILLAGE: i32 = 4;
/// How far from its village a site is (m).
pub const MIN_DISTANCE_M: f64 = 5_000.0;
pub const MAX_DISTANCE_M: f64 = 40_000.0;
/// Radius of a site (m): a zone to drive into, wide enough for the relief between it and its village,
/// whose altitude it is given.
pub const SITE_RADIUS_M: f64 = 1_500.0;
/// Hours a player has to get there, once the mission is taken.
pub const INSPECTION_HOURS: i32 = 3;
/// An inspection pays the base reward of a leg, once more per this distance (m).
pub const REWARD_STEP_M: f64 = 10_000.0;

/// The point at [distance] (m, along the ground) from [from] in the direction [bearing] (rad, around
/// the vertical of [from]), at the same distance from the planet's centre, the origin.
pub fn point_at(from: [f64; 3], bearing: f64, distance: f64) -> [f64; 3] {
    let length = |v: [f64; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    let radius = length(from);
    if radius <= 0.0 {
        return from;
    }
    let up = from.map(|c| c / radius);
    // Two directions along the ground: across the planet's axis (or another one, at a pole), and the third.
    let axis = if up[1].abs() < 0.9 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] };
    let cross = |a: [f64; 3], b: [f64; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let east = cross(axis, up);
    let norm = length(east);
    let east = east.map(|c| c / norm);
    let north = cross(up, east);
    let angle = distance / radius;
    std::array::from_fn(|i| {
        let along = east[i] * bearing.sin() + north[i] * bearing.cos();
        radius * (up[i] * angle.cos() + along * angle.sin())
    })
}

/// Where the site [seq] of [village] lies: (bearing in rad, distance in m). The sites of a village are
/// spread around it, a quarter turn each, and each as far as the village's id says -- so a village's
/// sites never change.
pub fn site_of(village: Uuid, seq: i32) -> (f64, f64) {
    // Two fractions in [0, 1) from the id and the rank (splitmix64).
    let id = village.as_u128();
    let mut state = (id as u64) ^ ((id >> 64) as u64) ^ (seq as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let mut fraction = || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
    };
    let turn = std::f64::consts::TAU / SITES_PER_VILLAGE as f64;
    let bearing = (seq as f64 + fraction()) * turn;
    (bearing, MIN_DISTANCE_M + fraction() * (MAX_DISTANCE_M - MIN_DISTANCE_M))
}

/// The name of the site [seq] of a village: "F0001-A".
pub fn site_name(village: &str, seq: i32) -> String {
    format!("{village}-{}", (b'A' + seq.clamp(0, 25) as u8) as char)
}

/// What an inspection [distance_m] away pays, from the base reward of a leg.
pub fn reward(base: i64, distance_m: f64) -> i64 {
    ((base as f64) * (1.0 + distance_m / REWARD_STEP_M)).round().max(1.0) as i64
}

/// Closes the inspections that ended, makes the sites of new villages, and opens a mission for every
/// village that has none. Returns `true` when the treasury could not pay an escrow.
pub async fn run(st: &AppState, fed: &Federation, report: &mut TickReport) -> Result<bool> {
    let villages: Vec<(Village, [f64; 3])> = world::active_villages(&st.pool, fed.id)
        .await?
        .into_iter()
        .filter_map(|v| match (v.x, v.y, v.z) {
            (Some(x), Some(y), Some(z)) => Some((v, [x, y, z])),
            _ => None,
        })
        .collect();
    let mut sites = inspections::list(&st.pool, fed.id).await?;
    let known: HashSet<Uuid> = sites.iter().map(|s| s.village_id).collect();
    let mut made = false;
    for (v, from) in villages.iter().filter(|(v, _)| !known.contains(&v.id)) {
        for seq in 0..SITES_PER_VILLAGE {
            let (bearing, distance) = site_of(v.id, seq);
            inspections::insert(&st.pool, fed.id, v.id, seq, &site_name(&v.name, seq), point_at(*from, bearing, distance), distance).await?;
        }
        made = true;
    }
    if made {
        sites = inspections::list(&st.pool, fed.id).await?;
    }
    // A site just inspected goes to the back of its village's: read them again, in their new order.
    if close_ended(st, fed, &sites).await? {
        sites = inspections::list(&st.pool, fed.id).await?;
    }

    let by_id: HashMap<Uuid, &Village> = villages.iter().map(|(v, _)| (v.id, v)).collect();
    let busy: HashSet<Uuid> = sites.iter().filter(|s| s.mission_id.is_some()).map(|s| s.village_id).collect();
    let mut seen = HashSet::new();
    let mut funds_short = false;
    // The sites come a village's together, the next to inspect first: the first of each village is it.
    for site in sites.iter().filter(|s| !busy.contains(&s.village_id)) {
        let Some(village) = by_id.get(&site.village_id) else { continue };
        if !seen.insert(site.village_id) {
            continue;
        }
        match publish(st, fed, village, site).await {
            Ok(()) => report.inspections_published += 1,
            Err(e) if e.upstream_code().is_some_and(|c| INSUFFICIENT_FUNDS_CODES.contains(&c)) => funds_short = true,
            Err(e) => report.error("publish inspection", e),
        }
    }
    Ok(funds_short)
}

/// Frees the sites whose mission is completed, cancelled or expired. Returns whether any was.
async fn close_ended(st: &AppState, fed: &Federation, sites: &[Site]) -> Result<bool> {
    let Some(oldest) = sites.iter().filter(|s| s.mission_id.is_some()).filter_map(|s| s.published_at).min() else { return Ok(false) };
    let mut closed = false;
    let since = Some(oldest - time::Duration::hours(1));
    for status in ["completed", "cancelled", "expired"] {
        for m in st.clients.mission.list("politics", fed.political_entity_id, status, since).await? {
            let Some(site) = sites.iter().find(|s| s.mission_id == Some(m.id)) else { continue };
            inspections::close(&st.pool, site.id, status == "completed").await?;
            closed = true;
            info!(site = %site.name, status, "inspection closed");
        }
    }
    Ok(closed)
}

async fn publish(st: &AppState, fed: &Federation, village: &Village, site: &Site) -> Result<()> {
    let poi = match site.inventory_poi_id {
        Some(id) => id,
        None => {
            let id = match st.clients.inventory.find_poi_by_persistence(site.id).await? {
                Some(id) => id,
                None => {
                    let poi = NewPoi {
                        owner_type: "political",
                        owner_id: fed.political_entity_id,
                        name: site.name.clone(),
                        scene: fed.planet_internal_name.clone(),
                        persistence_id: site.id,
                        x: site.x,
                        y: site.y,
                        z: site.z,
                        radius_m: SITE_RADIUS_M,
                    };
                    st.clients.inventory.create_poi(&poi).await?
                }
            };
            inspections::set_poi(&st.pool, site.id, id).await?;
            id
        }
    };
    let mut mission = build_mission(fed, village, site, poi);
    // Offered in its village, at the town hall.
    mission.zones = zones_of(&village_poi(st, fed, village.id).await?.into_iter().collect::<Vec<_>>());
    let created = match st.clients.mission.create(&mission).await {
        Ok(m) => m,
        Err(e) if e.upstream_code() == Some("DUPLICATE_SCRIPT_ID") => find_by_script(st, fed, &mission.script_id).await?,
        Err(e) => return Err(e),
    };
    inspections::set_published(&st.pool, site.id, created.id).await?;
    info!(site = %site.name, mission = %created.id, "inspection published");
    Ok(())
}

/// The mission that sends a player to [site], [poi] its place in `inventory`.
pub fn build_mission(fed: &Federation, village: &Village, site: &Site, poi: Uuid) -> NewMission {
    let km = (site.distance_m / 1000.0).round() as i64;
    let title = format!("Solo · Inspect zone {} ({} km from {})", site.name, km, village.name);
    let title_fr = format!("Solo · Inspection : zone {} (à {} km de {})", site.name, km, village.name);
    let objective = format!("Go to zone {}, {} km from {}", site.name, km, village.name);
    let objective_fr = format!("Rendez-vous dans la zone {}, à {} km de {}", site.name, km, village.name);
    let description = format!("{} security patrol.", fed.name);
    let category = "exploration";
    NewMission {
        title_i18n: Some(json!({ "en": title, "fr": title_fr })),
        description_i18n: Some(json!({ "en": description, "fr": format!("Patrouille de sécurité {}.", fed.name) })),
        title,
        description,
        kind: "dynamic",
        category,
        issuer_type: "politics",
        issuer_id: fed.political_entity_id,
        rewards: vec![json!({ "type": "credits", "currency": fed.currency, "amount": reward(fed.leg_base_reward, site.distance_m) })],
        escrow: Some(json!({ "payerType": "politics", "payerId": fed.political_entity_id })),
        script_id: format!("usim:inspection:{}:{}", site.id, site.attempts),
        objectives: vec![json!({
            "type": "visit",
            "title": objective,
            "titleI18n": { "en": objective, "fr": objective_fr },
            "locationTo": {
                "name": site.name,
                "scene": fed.planet_internal_name,
                "village": village.name,
                "villageId": village.id,
                "poiUuid": poi,
                "radiusM": SITE_RADIUS_M,
                "distanceM": site.distance_m,
                "position": { "x": site.x, "y": site.y, "z": site.z },
            },
            "params": { "zone": { "kind": "poi", "poiId": poi } },
            "payload": { "source": "universesimulator", "siteId": site.id, "issuer": { "name": fed.name, "kind": fed.kind } },
        })],
        // Being there is the whole job: paid on the spot, nothing to turn in.
        auto_complete: true,
        giver: giver_of(&fed.name, category),
        zones: Vec::new(),
        max_assignees: 1,
        assignment_deadline_hours: Some(INSPECTION_HOURS),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_point_stays_on_the_planet_at_the_distance_asked() {
        let from = [3_000_000.0, 4_000_000.0, 3_900_000.0];
        let radius = |p: [f64; 3]| (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
        for bearing in [0.0, 1.0, 3.0, 5.5] {
            let to = point_at(from, bearing, 25_000.0);
            assert!((radius(to) - radius(from)).abs() < 1e-3);
            let chord = radius([to[0] - from[0], to[1] - from[1], to[2] - from[2]]);
            assert!((chord - 25_000.0).abs() < 1.0, "{chord}");
        }
        // At a pole the planet's axis gives no direction: another one does.
        let pole = point_at([0.0, 6_000_000.0, 0.0], 0.3, 10_000.0);
        assert!(pole.iter().all(|c| c.is_finite()));
    }

    #[test]
    fn the_sites_of_a_village_are_spread_and_always_the_same() {
        let village = Uuid::from_u128(0xA1);
        let quarter = std::f64::consts::TAU / SITES_PER_VILLAGE as f64;
        for seq in 0..SITES_PER_VILLAGE {
            let (bearing, distance) = site_of(village, seq);
            assert_eq!((bearing, distance), site_of(village, seq));
            assert!((MIN_DISTANCE_M..=MAX_DISTANCE_M).contains(&distance));
            assert!(bearing >= seq as f64 * quarter && bearing < (seq + 1) as f64 * quarter);
        }
        assert_ne!(site_of(village, 0).1, site_of(Uuid::from_u128(0xA2), 0).1);
        assert_eq!(site_name("F0001", 2), "F0001-C");
    }

    #[test]
    fn a_far_inspection_pays_more() {
        assert_eq!(reward(100, 0.0), 100);
        assert_eq!(reward(100, 40_000.0), 500);
    }
}
