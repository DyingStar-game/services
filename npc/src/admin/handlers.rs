use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde_json::{Value, json};

use crate::{error::Result, npc::{manager::NpcManager, spec::NpcSpec}};

pub async fn healthz() -> &'static str {
    "ok"
}

pub async fn list_npcs(State(mgr): State<Arc<NpcManager>>) -> Json<Value> {
    let specs = mgr.list().await;
    Json(json!({
        "count": specs.len(),
        "npcs": specs.iter().map(|s| json!({
            "name": s.name,
            "actions": s.action_profile,
            "schedule": s.schedule,
        })).collect::<Vec<_>>(),
    }))
}

/// Current planetary time and scheduled activity for every NPC — for verifying
/// the rota as the 25 h day drifts against real time.
pub async fn schedule_status(State(mgr): State<Arc<NpcManager>>) -> Json<Value> {
    let reports = mgr.schedule_status().await;
    Json(json!({ "count": reports.len(), "npcs": reports }))
}

/// What each NPC sees on its workplace object — the replicated properties every
/// depot behaviour gates on. First stop when an NPC is idle at its post.
pub async fn world_status(State(mgr): State<Arc<NpcManager>>) -> Json<Value> {
    let reports = mgr.world_status().await;
    Json(json!({ "count": reports.len(), "npcs": reports }))
}

pub async fn create_npc(
    State(mgr): State<Arc<NpcManager>>,
    Json(spec): Json<NpcSpec>,
) -> Result<(StatusCode, Json<Value>)> {
    let name = mgr.spawn(spec).await?;
    Ok((StatusCode::CREATED, Json(json!({ "name": name }))))
}

pub async fn delete_npc(
    State(mgr): State<Arc<NpcManager>>,
    Path(name): Path<String>,
) -> Result<StatusCode> {
    mgr.despawn(&name).await?;
    Ok(StatusCode::NO_CONTENT)
}
