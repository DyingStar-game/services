//! Automatic role at an NPC's first spawn. Godot spawns an NPC in a habitation
//! of some village; the runtime reports where it stands, and here the NPC
//! becomes one of the five storekeepers (magasiniers) of that Mining village
//! (Mxxxx) — working at its warehouse — or a villager once those are taken.

use serde_json::json;
use tracing::{info, warn};
use uuid::Uuid;

use crate::{
    db::{self, workplaces::Workplace},
    error::Result,
    horizon::messages::Vec3,
    npc::manager::{DepotCandidate, NpcManager},
    roles::Depot,
    world_data::WorldData,
};

/// How far from the NPC a replicated cargo_depot may be to count as its
/// village's warehouse (fallback when universesimulator does not know).
const DEPOT_FALLBACK_RANGE_M: f64 = 2_000.0;

pub async fn assign(mgr: &NpcManager, id: Uuid, planet_pos: Vec3, depots: &[DepotCandidate]) -> Result<()> {
    let row = db::npcs::require(mgr.pool(), id).await?;
    if !row.assignment_pending() {
        return Ok(());
    }
    let wd = WorldData::shared();
    let planet = wd
        .as_ref()
        .map(|w| w.config.planet.clone())
        .unwrap_or_else(|| mgr.svc.config.planet_internal_name.clone());
    let village = wd.as_ref().and_then(|w| w.home_mining_village(planet_pos)).map(|v| v.name.clone());

    let Some(code) = village else {
        // Not in a Mining village: a villager wherever it is.
        let mut row = row;
        mgr.roles.apply_villager(&mut row, None, mgr.roles.villager.schedule.clone());
        db::npcs::save(mgr.pool(), &row).await?;
        info!(login = %row.login, "not in a Mining village; assigned villager");
        mgr.events().info(id, "role_assigned", "villager (not in a Mining village)", Some(json!({ "role": "villager" })));
        mgr.poke();
        return Ok(());
    };

    // Network calls stay outside the transaction.
    let workplace = resolve_workplace(mgr, &code, planet_pos, depots).await;

    let mut tx = mgr.pool().begin().await?;
    db::npcs::lock_village(&mut tx, &code).await?;
    // Re-read under the village lock: another pass may have assigned it.
    let Some(mut row) = sqlx::query_as::<_, db::npcs::NpcRow>(&format!(
        "SELECT * FROM npcs WHERE id = $1 FOR UPDATE"
    ))
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(());
    };
    if !row.assignment_pending() {
        return Ok(());
    }
    let taken = db::npcs::storekeeper_slots(&mut tx, &code).await?;
    let slots = mgr.roles.storekeeper_slots();
    let free = (1..=slots).find(|s| !taken.contains(&(*s as i16)));
    let schedule = mgr.svc.schedule.load();

    let (message, data) = match (free, &workplace) {
        (Some(slot), Some(wp)) => {
            let pattern = mgr.roles.storekeeper_schedule(slot);
            let pattern = if schedule.has_pattern(&pattern) {
                Some(pattern)
            } else {
                warn!(login = %row.login, %pattern, "storekeeper pattern missing; import schedules.yaml");
                None
            };
            let depot = Depot { id: wp.warehouse_id, scenename: wp.scenename.as_deref(), planet: &planet };
            mgr.roles.apply_storekeeper(&mut row, &code, slot, &depot, pattern.clone());
            (
                format!("storekeeper {slot}/{slots} of {code}"),
                json!({ "role": "storekeeper", "village": code, "slot": slot, "warehouse": wp.warehouse_id,
                        "source": wp.source, "schedule": pattern }),
            )
        }
        (Some(_), None) => {
            // A storekeeper slot is free but the warehouse is unknown: behave as
            // a villager for now and try again at the next session.
            mgr.roles.apply_villager(&mut row, Some(&code), mgr.roles.villager.schedule.clone());
            row.role = "unassigned".into();
            mgr.events().warn(
                id,
                "assign_failed",
                format!("no warehouse found for {code}; retrying at the next session"),
                Some(json!({ "village": code })),
            );
            (format!("villager of {code} until its warehouse is known"), json!({ "role": "unassigned", "village": code }))
        }
        (None, _) => {
            mgr.roles.apply_villager(&mut row, Some(&code), mgr.roles.villager.schedule.clone());
            (format!("villager of {code}"), json!({ "role": "villager", "village": code }))
        }
    };
    db::npcs::save(&mut *tx, &row).await?;
    tx.commit().await?;

    info!(login = %row.login, %message, "role assigned");
    mgr.events().info(id, "role_assigned", message, Some(data));
    mgr.poke();
    Ok(())
}

/// The cargo_depot a village's storekeepers work at: the cached answer, else
/// universesimulator's main warehouse, else the nearest depot the NPC sees.
async fn resolve_workplace(
    mgr: &NpcManager,
    code: &str,
    planet_pos: Vec3,
    depots: &[DepotCandidate],
) -> Option<Workplace> {
    match db::workplaces::get(mgr.pool(), code).await {
        Ok(Some(w)) => return Some(w),
        Ok(None) => {}
        Err(e) => warn!(village = %code, error = %e, "cannot read workplace cache"),
    }
    if let Some(usim) = &mgr.usim {
        match usim.main_warehouse(code).await {
            Ok(Some(w)) => {
                match db::workplaces::upsert(mgr.pool(), code, w.id, Some(&w.code), w.scenename.as_deref(), "usim", None)
                    .await
                {
                    Ok(wp) => return Some(wp),
                    Err(e) => warn!(village = %code, error = %e, "cannot cache workplace"),
                }
            }
            Ok(None) => info!(village = %code, "universesimulator has no warehouse for this village"),
            Err(e) => warn!(village = %code, error = %e, "universesimulator lookup failed"),
        }
    }
    let nearest = depots
        .iter()
        .map(|d| (d, distance(d.planet_pos, planet_pos)))
        .filter(|(_, dist)| *dist <= DEPOT_FALLBACK_RANGE_M)
        .min_by(|a, b| a.1.total_cmp(&b.1))?;
    let d = nearest.0;
    match db::workplaces::upsert(mgr.pool(), code, d.uuid, None, d.scenename.as_deref(), "world", None).await {
        Ok(wp) => Some(wp),
        Err(e) => {
            warn!(village = %code, error = %e, "cannot cache workplace");
            None
        }
    }
}

fn distance(a: Vec3, b: Vec3) -> f64 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2) + (a.z - b.z).powi(2)).sqrt()
}
