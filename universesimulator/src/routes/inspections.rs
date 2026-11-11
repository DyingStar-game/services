//! Inspection sites and the ground (ADR 0019). The simulator makes a site at its village's altitude, for
//! it knows no relief; the game server, which has the terrain, asks for the sites not laid yet and
//! answers where the ground is under each. A site gets its place in `inventory`, and its missions, after.

use axum::{extract::{Path, State}, Extension, Json};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::{Caller, ROLE_DELIVER};
use crate::db::inspections;
use crate::error::{AppError, Result};
use crate::simulation::inspections::{MAX_GROUND_SHIFT_M, SITE_RADIUS_M};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ground {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// The sites to lay on the ground: `{items: [{id, scene, position}]}`, `scene` their planet.
pub async fn ungrounded(State(st): State<AppState>, Extension(caller): Extension<Caller>) -> Result<Json<Value>> {
    caller.require_role(ROLE_DELIVER)?;
    let items: Vec<Value> = inspections::ungrounded(&st.pool)
        .await?
        .into_iter()
        .map(|(id, scene, x, y, z)| json!({ "id": id, "scene": scene, "position": { "x": x, "y": y, "z": z } }))
        .collect();
    Ok(Json(json!({ "items": items })))
}

/// Lays a site on the ground. Once: a second answer changes nothing (`laid: false`).
pub async fn set_ground(State(st): State<AppState>, Extension(caller): Extension<Caller>, Path(id): Path<Uuid>, Json(body): Json<Ground>) -> Result<Json<Value>> {
    caller.require_role(ROLE_DELIVER)?;
    let site = inspections::get(&st.pool, id).await?.ok_or_else(|| AppError::NotFound(format!("inspection site {id}")))?;
    if site.grounded_at.is_some() {
        return Ok(Json(json!({ "laid": false })));
    }
    let shift = ((body.x - site.x).powi(2) + (body.y - site.y).powi(2) + (body.z - site.z).powi(2)).sqrt();
    if !shift.is_finite() || shift > MAX_GROUND_SHIFT_M {
        return Err(AppError::BadRequest(format!("the ground is at most {MAX_GROUND_SHIFT_M} m from where the site was made")));
    }
    // A place it already has in `inventory` goes with it, first: a failure there leaves the site to lay.
    if let Some(poi) = site.inventory_poi_id {
        st.clients.inventory.move_poi(poi, [body.x, body.y, body.z], SITE_RADIUS_M).await?;
    }
    let laid = inspections::set_ground(&st.pool, id, [body.x, body.y, body.z]).await?;
    Ok(Json(json!({ "laid": laid })))
}
