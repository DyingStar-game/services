//! Breakdowns: ARES Machinery has a machine of a village repaired (ADR 0020). A mission that moves no
//! goods, like an inspection: the simulator breaks a machine the game server listed, offers its repair,
//! and the machine stays broken -- unusable in game -- until the game server says a player repaired it.
//!
//! The repair itself is the game's (a diagnosis on the machine, then gauges to align): the game server
//! confirms the mission's objective with the mission service, which pays, and tells the simulator.

use std::collections::HashMap;

use serde_json::json;
use tracing::info;
use uuid::Uuid;

use super::publisher::{find_by_script, giver_in, village_poi, zones_of};
use super::TickReport;
use crate::clients::{NewMission, INSUFFICIENT_FUNDS_CODES};
use crate::db::catalog::Federation;
use crate::db::machines::{self, Breakdown};
use crate::db::world::{self, Village};
use crate::error::Result;
use crate::state::AppState;

/// What a repair pays (credits): a trip to the village and a few minutes on the machine.
pub const REPAIR_REWARD: i64 = 1_500;
/// How long a player has to repair once the mission is taken (hours).
pub const REPAIR_HOURS: i32 = 3;
/// A machine further than this from every village is nobody's (m).
pub const MAX_VILLAGE_DISTANCE_M: f64 = 3_000.0;

/// A machine's name in a mission's text.
#[derive(Debug, PartialEq, Eq)]
pub struct MachineName {
    pub en: &'static str,
    /// The bare French name, for a title ("raffinerie" in the repair of the refinery of M0001).
    pub fr: &'static str,
    /// The French name with its article, for a sentence ("la raffinerie", "le concasseur").
    pub fr_the: &'static str,
    /// The French pronoun that stands for it at the end of the sentence ("le", "la").
    pub fr_it: &'static str,
}

/// The name of a machine of [kind]; "machine" for a kind unknown here. A lift's kind says which lift it
/// is (a mine's, a canyon's) and which of its parts broke -- the platform, or a landing, whose screen
/// calls it: "the lift of M0001" was several machines under one name, and the player repaired the
/// wrong one (playtest 2026-11-12).
#[rustfmt::skip]
pub fn machine_name(kind: &str) -> MachineName {
    let (en, fr, fr_the, fr_it) = match kind {
        "crusher" => ("crusher", "concasseur", "le concasseur", "le"),
        "refinery" => ("refinery", "raffinerie", "la raffinerie", "la"),
        "dispenser" => ("dispenser", "distributeur", "le distributeur", "le"),
        "terminal" => ("vehicle terminal", "borne de v\u{e9}hicules", "la borne de v\u{e9}hicules", "la"),
        "kiosk" => ("mission kiosk", "borne de missions", "la borne de missions", "la"),
        "screen" => ("information screen", "\u{e9}cran d'information", "l'\u{e9}cran d'information", "le"),
        "lift" => ("lift", "ascenseur", "l'ascenseur", "le"),
        "teleporter" => ("teleporter", "t\u{e9}l\u{e9}porteur", "le t\u{e9}l\u{e9}porteur", "le"),
        "mine_lift_platform" => ("mine lift platform", "plateforme de l'ascenseur de mine", "la plateforme de l'ascenseur de mine", "la"),
        "mine_lift_landing_upper" => ("mine lift upper landing", "palier haut de l'ascenseur de mine", "le palier haut de l'ascenseur de mine", "le"),
        "mine_lift_landing_lower" => ("mine lift lower landing", "palier bas de l'ascenseur de mine", "le palier bas de l'ascenseur de mine", "le"),
        "mine_lift_landing_stop" => ("mine lift level landing", "palier interm\u{e9}diaire de l'ascenseur de mine", "le palier interm\u{e9}diaire de l'ascenseur de mine", "le"),
        "canyon_lift_platform" => ("canyon lift platform", "plateforme de l'ascenseur de canyon", "la plateforme de l'ascenseur de canyon", "la"),
        "canyon_lift_landing_upper" => ("canyon lift upper landing", "palier haut de l'ascenseur de canyon", "le palier haut de l'ascenseur de canyon", "le"),
        "canyon_lift_landing_lower" => ("canyon lift lower landing", "palier bas de l'ascenseur de canyon", "le palier bas de l'ascenseur de canyon", "le"),
        "canyon_lift_landing_stop" => ("canyon lift level landing", "palier interm\u{e9}diaire de l'ascenseur de canyon", "le palier interm\u{e9}diaire de l'ascenseur de canyon", "le"),
        _ => ("machine", "machine", "la machine", "la"),
    };
    MachineName { en, fr, fr_the, fr_it }
}

/// The village among [villages] nearest to [at], no further than MAX_VILLAGE_DISTANCE_M.
pub fn village_at(villages: &[Village], at: [f64; 3]) -> Option<&Village> {
    villages
        .iter()
        .filter_map(|v| {
            let (x, y, z) = (v.x?, v.y?, v.z?);
            let d = ((x - at[0]).powi(2) + (y - at[1]).powi(2) + (z - at[2]).powi(2)).sqrt();
            (d <= MAX_VILLAGE_DISTANCE_M).then_some((v, d))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(v, _)| v)
}

/// Reads the missions that ended, breaks machines up to `sim_breakdowns_open`, and offers the repair of
/// every broken machine that has no mission. Returns `true` when the treasury could not pay an escrow.
pub async fn run(st: &AppState, fed: &Federation, report: &mut TickReport) -> Result<bool> {
    let mut rows = machines::pending(&st.pool, fed.id).await?;
    if close_ended(st, fed, &rows).await? {
        rows = machines::pending(&st.pool, fed.id).await?;
    }
    // Repaired with its mission still running (the game repaired it for another reason): nothing left
    // to do for the player who holds it. 409: over on the mission's side, read at the next tick.
    for row in rows.iter().filter(|r| r.repaired_at.is_some()) {
        let Some(mission) = row.mission_id else { continue };
        match st.clients.mission.cancel(mission).await {
            Err(e) if e.upstream_status() != Some(409) => return Err(e),
            _ => machines::clear_mission(&st.pool, row.id).await?,
        }
    }

    let mut open = rows.iter().filter(|r| r.repaired_at.is_none()).count();
    let mut broke = false;
    while open < st.cfg.sim_breakdowns_open {
        let Some(machine) = machines::next_to_break(&st.pool, fed.id).await? else { break };
        machines::break_machine(&st.pool, machine).await?;
        info!(%machine, "machine broken");
        open += 1;
        broke = true;
    }
    if broke {
        rows = machines::pending(&st.pool, fed.id).await?;
    }

    let villages: HashMap<Uuid, Village> = world::active_villages(&st.pool, fed.id).await?.into_iter().map(|v| (v.id, v)).collect();
    let mut funds_short = false;
    for row in rows.iter().filter(|r| r.repaired_at.is_none() && r.mission_id.is_none()) {
        let Some(village) = villages.get(&row.village_id) else { continue };
        match publish(st, fed, village, row).await {
            Ok(()) => report.repairs_published += 1,
            Err(e) if e.upstream_code().is_some_and(|c| INSUFFICIENT_FUNDS_CODES.contains(&c)) => funds_short = true,
            Err(e) => report.error("publish repair", e),
        }
    }
    Ok(funds_short)
}

/// Reads the missions that ended: done, the machine works again (if the game server had not said so);
/// cancelled or expired, the machine stays broken and gets another mission. Returns whether any ended.
async fn close_ended(st: &AppState, fed: &Federation, rows: &[Breakdown]) -> Result<bool> {
    let Some(oldest) = rows.iter().filter(|r| r.mission_id.is_some()).filter_map(|r| r.published_at).min() else { return Ok(false) };
    let since = Some(oldest - time::Duration::hours(1));
    let mut closed = false;
    for status in ["completed", "cancelled", "expired"] {
        for m in st.clients.mission.list("politics", fed.political_entity_id, status, since).await? {
            let Some(row) = rows.iter().find(|r| r.mission_id == Some(m.id)) else { continue };
            if status == "completed" {
                machines::repair(&st.pool, row.id, None).await?;
            }
            machines::clear_mission(&st.pool, row.id).await?;
            closed = true;
            info!(machine = %row.machine_id, status, "repair mission ended");
        }
    }
    Ok(closed)
}

async fn publish(st: &AppState, fed: &Federation, village: &Village, row: &Breakdown) -> Result<()> {
    let poi = village_poi(st, fed, village.id).await?;
    let mut mission = build_mission(fed, village, row, poi);
    // Offered in its village, at the town hall.
    mission.zones = zones_of(&poi.into_iter().collect::<Vec<_>>());
    let created = match st.clients.mission.create(&mission).await {
        Ok(m) => m,
        Err(e) if e.upstream_code() == Some("DUPLICATE_SCRIPT_ID") => find_by_script(st, fed, &mission.script_id).await?,
        Err(e) => return Err(e),
    };
    machines::set_published(&st.pool, row.id, created.id).await?;
    info!(machine = %row.machine_id, village = %village.code, mission = %created.id, "repair published");
    Ok(())
}

/// The mission that has [row]'s machine repaired, [poi] its village's place in `inventory`.
pub fn build_mission(fed: &Federation, village: &Village, row: &Breakdown, poi: Option<Uuid>) -> NewMission {
    let name = machine_name(&row.kind);
    let title = format!("Solo \u{b7} Repair the {} of {}", name.en, village.name);
    let title_fr = format!("Solo \u{b7} R\u{e9}paration : {} de {}", name.fr, village.name);
    let objective = format!("Diagnose the {} of {} and repair it", name.en, village.name);
    let objective_fr = format!(
        "Diagnostiquez {} de {} et r\u{e9}parez-{}",
        name.fr_the, village.name, name.fr_it
    );
    let description = format!("{} maintenance order.", fed.name);
    NewMission {
        title_i18n: Some(json!({ "en": title, "fr": title_fr })),
        description_i18n: Some(json!({ "en": description, "fr": format!("Ordre de maintenance {}.", fed.name) })),
        title,
        description,
        kind: "dynamic",
        category: "divers",
        issuer_type: "politics",
        issuer_id: fed.political_entity_id,
        rewards: vec![json!({ "type": "credits", "currency": fed.currency, "amount": REPAIR_REWARD })],
        escrow: Some(json!({ "payerType": "politics", "payerId": fed.political_entity_id })),
        script_id: format!("usim:repair:{}:{}", row.id, row.attempts),
        objectives: vec![json!({
            // Confirmed by the game server once the player has repaired the machine in game.
            "type": "manual",
            "title": objective,
            "titleI18n": { "en": objective, "fr": objective_fr },
            "locationTo": {
                "name": village.name,
                "scene": fed.planet_internal_name,
                "village": village.name,
                "villageId": village.id,
                "poiUuid": poi,
                "machineId": row.machine_id,
                "machineKind": row.kind,
            },
            "params": {},
            "payload": {
                "source": "universesimulator",
                "breakdownId": row.id,
                "machineId": row.machine_id,
                "machineKind": row.kind,
                "issuer": { "name": fed.name, "kind": fed.kind },
            },
        })],
        // The repair is the whole job: paid when the machine works, nothing to turn in.
        auto_complete: true,
        giver: giver_in(&fed.name, "machinery"),
        zones: Vec::new(),
        max_assignees: 1,
        assignment_deadline_hours: Some(REPAIR_HOURS),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kind_unknown_here_is_a_machine() {
        assert_eq!(machine_name("crusher").fr, "concasseur");
        assert_eq!(machine_name("espresso").en, "machine");
    }

    #[test]
    fn a_lift_s_part_is_named_with_its_lift() {
        // "The lift of M0001" was the mine's and the canyon's, and any of their screens.
        let landing = machine_name("mine_lift_landing_upper");
        assert_eq!(landing.en, "mine lift upper landing");
        assert_eq!(landing.fr_the, "le palier haut de l'ascenseur de mine");
        let platform = machine_name("canyon_lift_platform");
        assert_eq!(platform.fr_the, "la plateforme de l'ascenseur de canyon");
        assert_eq!(platform.fr_it, "la");
    }

    #[test]
    fn a_french_sentence_takes_the_machine_s_article() {
        // "Diagnostiquez le raffinerie" before: every machine was "le".
        assert_eq!(machine_name("refinery").fr_the, "la raffinerie");
        assert_eq!(machine_name("screen").fr_the, "l'\u{e9}cran d'information");
    }
}
