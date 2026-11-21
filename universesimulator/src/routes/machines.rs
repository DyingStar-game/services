//! Machines and their breakdowns (ADR 0020), for the game server: it lists the machines it has loaded
//! and is told which are broken; it says when a player repaired one.

use axum::{extract::{Path, State}, Extension, Json};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::{Caller, ROLE_DELIVER};
use crate::db::{catalog, machines, world};
use crate::error::Result;
use crate::simulation::breakdowns::village_at;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Position {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Machine {
    pub id: Uuid,
    pub kind: String,
    /// From its planet's centre, like a village's.
    pub position: Position,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Listing {
    /// The planet the machines stand on.
    pub scene: String,
    pub machines: Vec<Machine>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Repaired {
    pub player_id: Option<Uuid>,
}

/// The game server's machines on a planet. Each is given to the village it stands in (the nearest, a
/// machine far from all is left out). Answers the breakdowns open on that planet:
/// `{known, broken: [{machineId, breakdownId, missionId}]}`.
pub async fn list(State(st): State<AppState>, Extension(caller): Extension<Caller>, Json(body): Json<Listing>) -> Result<Json<Value>> {
    caller.require_role(ROLE_DELIVER)?;
    let mut known = 0;
    for fed in catalog::list_federations(&st.pool).await?.into_iter().filter(|f| f.planet_internal_name == body.scene) {
        let villages = world::active_villages(&st.pool, fed.id).await?;
        for m in &body.machines {
            let kind = m.kind.trim();
            if kind.is_empty() || kind.len() > 32 {
                continue;
            }
            let Some(village) = village_at(&villages, [m.position.x, m.position.y, m.position.z]) else { continue };
            machines::upsert(&st.pool, m.id, fed.id, village.id, kind).await?;
            known += 1;
        }
    }
    let broken: Vec<Value> = machines::open_on(&st.pool, &body.scene)
        .await?
        .into_iter()
        .map(|(machine, breakdown, mission)| json!({ "machineId": machine, "breakdownId": breakdown, "missionId": mission }))
        .collect();
    Ok(Json(json!({ "known": known, "broken": broken })))
}

/// The machines broken down, for a client to draw them so: `{items: [{machineId, kind}]}`.
pub async fn broken(State(st): State<AppState>) -> Result<Json<Value>> {
    let items: Vec<Value> = machines::open_all(&st.pool)
        .await?
        .into_iter()
        .map(|(machine, kind)| json!({ "machineId": machine, "kind": kind }))
        .collect();
    Ok(Json(json!({ "items": items })))
}

/// A player repaired the machine. Once: a second call changes nothing (`repaired: false`).
pub async fn repaired(State(st): State<AppState>, Extension(caller): Extension<Caller>, Path(id): Path<Uuid>, Json(body): Json<Repaired>) -> Result<Json<Value>> {
    caller.require_role(ROLE_DELIVER)?;
    Ok(Json(json!({ "repaired": machines::repair(&st.pool, id, body.player_id).await? })))
}
