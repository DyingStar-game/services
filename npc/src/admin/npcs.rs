//! `/api/admin/npcs…`: the roster, one NPC (live status, routine, logs,
//! events), manual tasks and bulk spawn/despawn/delete.

use std::collections::HashMap;

use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};
use sqlx::types::Json as SqlJson;
use tracing::Level;
use uuid::Uuid;

use crate::{
    actions::{ActionRegistry, manual},
    auth::Caller,
    crypto::random_password,
    db::{
        self,
        events::EventFilter,
        npcs::{NpcFilter, NpcRow, ROLES},
    },
    error::{NpcError, Result},
    npc::{
        manager::LiveStatus,
        provision::{self, CreateRequest},
        spec::NpcSpec,
    },
    state::AppState,
};

/// A row as the admin panel shows it: stored configuration plus live status.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NpcView {
    #[serde(flatten)]
    pub row: NpcRow,
    pub live: LiveStatus,
}

/// No `#[serde(flatten)]`: query strings lose the field types through it.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListQuery {
    pub village: Option<String>,
    pub role: Option<String>,
    pub enabled: Option<bool>,
    pub q: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    /// Only NPCs with a live Horizon session (`true`) or without (`false`).
    pub online: Option<bool>,
}

pub async fn list(State(st): State<AppState>, Query(q): Query<ListQuery>) -> Result<Json<Value>> {
    let filter = NpcFilter {
        village: q.village,
        role: q.role,
        enabled: q.enabled,
        q: q.q.filter(|s| !s.is_empty()),
        limit: q.limit,
        offset: q.offset,
    };
    let (rows, total) = db::npcs::list(st.mgr.pool(), &filter).await?;
    let live = st.mgr.live_all().await;
    let items: Vec<NpcView> = rows
        .into_iter()
        .map(|row| {
            let live = live.get(&row.id).cloned().unwrap_or_else(stopped);
            NpcView { row, live }
        })
        .filter(|v| q.online.is_none_or(|o| v.live.connected == o))
        .collect();
    Ok(Json(json!({ "items": items, "total": total })))
}

fn stopped() -> LiveStatus {
    LiveStatus { running: false, connected: false, status: None, current_action: None, running_since: None }
}

pub async fn create(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Json(req): Json<CreateRequest>,
) -> Result<(StatusCode, Json<Value>)> {
    caller.require_admin()?;
    req.validate()?;
    let by = caller.label();
    let mut created = Vec::new();
    let mut errors = Vec::new();
    for _ in 0..req.count {
        match provision::create_one(&st.mgr, &req, Some(&by)).await {
            Ok(row) => created.push(row),
            Err(e) => {
                errors.push(e.to_string());
                // Keycloak or the database is down: no point going on.
                if matches!(e, NpcError::Upstream { .. } | NpcError::Db(_) | NpcError::Config(_)) {
                    if created.is_empty() {
                        return Err(e);
                    }
                    break;
                }
            }
        }
    }
    if created.is_empty() {
        return Err(NpcError::Conflict(errors.join("; ")));
    }
    st.mgr.poke();
    let items: Vec<NpcView> = created.into_iter().map(|row| NpcView { row, live: stopped() }).collect();
    Ok((StatusCode::CREATED, Json(json!({ "created": items, "errors": errors }))))
}

pub async fn get(State(st): State<AppState>, Path(id): Path<Uuid>) -> Result<Json<Value>> {
    let row = db::npcs::require(st.mgr.pool(), id).await?;
    let routine = st.mgr.routine(&row).await.ok();
    let tasks = db::tasks::list(st.mgr.pool(), id, 20).await?;
    let state = st.mgr.state_snapshot(id).await;
    let workplace = match &row.village_code {
        Some(v) => db::workplaces::get(st.mgr.pool(), v).await?,
        None => None,
    };
    let live = st.mgr.live(id).await;
    Ok(Json(json!({
        "npc": NpcView { row, live },
        "routine": routine,
        "tasks": tasks,
        "state": state,
        "workplace": workplace,
    })))
}

/// `Some(None)` for an explicit `null`, `None` when the field is absent.
fn double<'de, D, T>(d: D) -> std::result::Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NpcPatch {
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub enabled: Option<bool>,
    pub role: Option<String>,
    pub role_locked: Option<bool>,
    #[serde(default, deserialize_with = "double")]
    pub village_code: Option<Option<String>>,
    #[serde(default, deserialize_with = "double")]
    pub storekeeper_slot: Option<Option<i16>>,
    #[serde(default, deserialize_with = "double")]
    pub schedule_pattern: Option<Option<String>>,
    pub action_profile: Option<Vec<String>>,
    #[serde(default, deserialize_with = "double")]
    pub place_where_work: Option<Option<Value>>,
    #[serde(default, deserialize_with = "double")]
    pub home: Option<Option<Value>>,
    #[serde(default, deserialize_with = "double")]
    pub meal_place: Option<Option<Value>>,
    #[serde(default, deserialize_with = "double")]
    pub mandatory_place: Option<Option<Value>>,
    pub action_timing: Option<Value>,
    pub action_config: Option<Value>,
    #[serde(default, deserialize_with = "double")]
    pub notes: Option<Option<String>>,
}

/// Apply `p` to `row` and check the result. Returns whether the names changed.
pub fn apply_patch(row: &mut NpcRow, p: NpcPatch) -> Result<bool> {
    let old_names = (row.first_name.clone(), row.last_name.clone());
    if let Some(v) = p.first_name {
        row.first_name = non_empty(v, "firstName")?;
    }
    if let Some(v) = p.last_name {
        row.last_name = non_empty(v, "lastName")?;
    }
    let renamed = old_names != (row.first_name.clone(), row.last_name.clone());
    if renamed {
        row.display_name = format!("{} {}", row.first_name, row.last_name);
    }
    if let Some(v) = p.enabled {
        row.enabled = v;
    }
    if let Some(role) = p.role {
        if !ROLES.contains(&role.as_str()) {
            return Err(NpcError::BadRequest(format!("role must be one of {ROLES:?}")));
        }
        // A role set by hand sticks, unless it asks for a new assignment.
        row.role_locked = role != "unassigned";
        if role != "storekeeper" {
            row.storekeeper_slot = None;
        }
        row.role = role;
    }
    if let Some(v) = p.role_locked {
        row.role_locked = v;
    }
    if let Some(v) = p.village_code {
        row.village_code = v.filter(|s| !s.trim().is_empty());
    }
    if let Some(v) = p.storekeeper_slot {
        row.storekeeper_slot = v;
    }
    if let Some(v) = p.schedule_pattern {
        row.schedule_pattern = v.filter(|s| !s.is_empty());
    }
    if let Some(v) = p.action_profile {
        let known = catalogue_names();
        if let Some(bad) = v.iter().find(|a| !known.contains(&a.as_str())) {
            return Err(NpcError::BadRequest(format!("unknown action '{bad}'")));
        }
        row.action_profile = v;
    }
    let place = |v: Option<Value>| v.filter(|v| !v.is_null()).map(SqlJson);
    if let Some(v) = p.place_where_work {
        row.place_where_work = place(v);
    }
    if let Some(v) = p.home {
        row.home = place(v);
    }
    if let Some(v) = p.meal_place {
        row.meal_place = place(v);
    }
    if let Some(v) = p.mandatory_place {
        row.mandatory_place = place(v);
    }
    if let Some(v) = p.action_timing {
        row.action_timing = SqlJson(if v.is_null() { json!({}) } else { v });
    }
    if let Some(v) = p.action_config {
        row.action_config = SqlJson(if v.is_null() { json!({}) } else { v });
    }
    if let Some(v) = p.notes {
        row.notes = v;
    }
    if row.role == "storekeeper" && (row.storekeeper_slot.is_none() || row.village_code.is_none()) {
        return Err(NpcError::BadRequest("a storekeeper needs villageCode and storekeeperSlot".into()));
    }
    if row.role != "storekeeper" && row.storekeeper_slot.is_some() {
        return Err(NpcError::BadRequest("storekeeperSlot is only for storekeepers".into()));
    }
    // The runtime must be able to run it.
    row.to_spec()?;
    Ok(renamed)
}

fn non_empty(v: String, field: &str) -> Result<String> {
    let v = v.trim().to_string();
    if v.is_empty() || v.len() > 64 {
        return Err(NpcError::BadRequest(format!("{field} must be 1 to 64 characters")));
    }
    Ok(v)
}

/// Every action name an NPC profile may list.
pub fn catalogue_names() -> Vec<&'static str> {
    let spec: NpcSpec = serde_json::from_value(json!({ "uuid": "" })).expect("bare spec");
    ActionRegistry::catalogue(&spec).iter().map(|a| a.name()).collect()
}

pub async fn patch(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<Uuid>,
    Json(p): Json<NpcPatch>,
) -> Result<Json<Value>> {
    caller.require_admin()?;
    let mut row = db::npcs::require(st.mgr.pool(), id).await?;
    let renamed = apply_patch(&mut row, p)?;
    if let Some(pattern) = &row.schedule_pattern {
        if db::patterns::get(st.mgr.pool(), pattern).await?.is_none() {
            return Err(NpcError::BadRequest(format!("unknown schedule pattern '{pattern}'")));
        }
    }
    if renamed {
        if db::npcs::display_name_taken(st.mgr.pool(), &row.display_name).await? {
            return Err(NpcError::Conflict(format!("an NPC is already named {}", row.display_name)));
        }
        st.mgr.kc.update_profile(id, &row.first_name, &row.last_name, &row.display_name).await?;
    }
    let row = db::npcs::save(st.mgr.pool(), &row).await?;
    st.mgr.events().info(id, "updated", format!("configuration edited by {}", caller.label()), None);
    st.mgr.poke();
    if renamed {
        // The name is in the token: log in again so Horizon shows the new one
        // at the next connection.
        st.mgr.svc.tokens.forget(id).await;
    }
    let live = st.mgr.live(id).await;
    Ok(Json(json!({ "npc": NpcView { row, live } })))
}

pub async fn delete(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode> {
    caller.require_admin()?;
    st.mgr.delete_npc(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn set_enabled(st: &AppState, caller: &Caller, id: Uuid, enabled: bool) -> Result<Json<Value>> {
    caller.require_admin()?;
    db::npcs::require(st.mgr.pool(), id).await?;
    db::npcs::set_enabled(st.mgr.pool(), &[id], enabled).await?;
    st.mgr.poke();
    let row = db::npcs::require(st.mgr.pool(), id).await?;
    let live = st.mgr.live(id).await;
    Ok(Json(json!({ "npc": NpcView { row, live } })))
}

pub async fn spawn(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>> {
    set_enabled(&st, &caller, id, true).await
}

pub async fn despawn(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>> {
    set_enabled(&st, &caller, id, false).await
}

/// Forget the role and workplace: the assignment runs again at the NPC's next
/// session (right away when it is connected).
pub async fn reassign(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>> {
    caller.require_admin()?;
    let mut row = db::npcs::require(st.mgr.pool(), id).await?;
    row.role = "unassigned".into();
    row.role_locked = false;
    row.village_code = None;
    row.storekeeper_slot = None;
    row.schedule_pattern = None;
    row.action_profile = Vec::new();
    row.place_where_work = None;
    row.home = None;
    row.meal_place = None;
    row.mandatory_place = None;
    row.action_config = SqlJson(json!({}));
    let row = db::npcs::save(st.mgr.pool(), &row).await?;
    st.mgr.events().info(id, "reassign", format!("role reset by {}", caller.label()), None);
    st.mgr.poke();
    let live = st.mgr.live(id).await;
    Ok(Json(json!({ "npc": NpcView { row, live } })))
}

pub async fn reset_credentials(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode> {
    caller.require_admin()?;
    db::npcs::require(st.mgr.pool(), id).await?;
    st.mgr.svc.tokens.rotate(id, &random_password()).await?;
    st.mgr.events().info(id, "credentials_rotated", format!("password reset by {}", caller.label()), None);
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkRequest {
    /// `spawn`, `despawn` or `delete`.
    pub action: String,
    /// The NPCs to act on…
    #[serde(default)]
    pub ids: Option<Vec<Uuid>>,
    /// …or how many to pick (spawn: oldest disabled; despawn/delete: most
    /// recent enabled non-storekeepers first), within `village` / `role`.
    #[serde(default)]
    pub count: Option<i64>,
    #[serde(default)]
    pub village: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
}

pub async fn bulk(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Json(req): Json<BulkRequest>,
) -> Result<Json<Value>> {
    caller.require_admin()?;
    let pool = st.mgr.pool();
    let ids = match (&req.ids, req.count) {
        (Some(ids), None) => ids.clone(),
        (None, Some(n)) if (1..=1000).contains(&n) => {
            let enable = req.action == "spawn";
            db::npcs::pick_for_toggle(pool, enable, n, req.village.as_deref(), req.role.as_deref()).await?
        }
        _ => return Err(NpcError::BadRequest("give either ids or count (1..=1000)".into())),
    };
    let mut errors = Vec::new();
    let affected = match req.action.as_str() {
        "spawn" => db::npcs::set_enabled(pool, &ids, true).await?,
        "despawn" => db::npcs::set_enabled(pool, &ids, false).await?,
        "delete" => {
            let mut done = Vec::new();
            for id in &ids {
                match st.mgr.delete_npc(*id).await {
                    Ok(()) => done.push(*id),
                    Err(e) => errors.push(format!("{id}: {e}")),
                }
            }
            done
        }
        _ => return Err(NpcError::BadRequest("action must be spawn, despawn or delete".into())),
    };
    st.mgr.events().global(
        "bulk",
        format!("{} {} NPC(s) by {}", req.action, affected.len(), caller.label()),
        Some(json!({ "action": req.action, "count": affected.len() })),
    );
    st.mgr.poke();
    Ok(Json(json!({ "affected": affected, "errors": errors })))
}

#[derive(Debug, Deserialize)]
pub struct LogsQuery {
    #[serde(default)]
    pub since: u64,
    /// Minimum level: `error`, `warn`, `info`, `debug`, `trace`.
    pub level: Option<String>,
}

pub async fn logs(
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
    Query(q): Query<LogsQuery>,
) -> Result<Json<Value>> {
    let level = match q.level.as_deref() {
        Some(l) => Some(
            l.parse::<Level>()
                .map_err(|_| NpcError::BadRequest(format!("unknown level '{l}'")))?,
        ),
        None => None,
    };
    let items = st.mgr.logs.lines(&id.to_string(), q.since, level);
    let last_seq = items.last().map(|l| l.seq).unwrap_or(q.since);
    Ok(Json(json!({ "items": items, "lastSeq": last_seq })))
}

pub async fn npc_events(
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
    Query(f): Query<EventFilter>,
) -> Result<Json<Value>> {
    let items = db::events::list(st.mgr.pool(), Some(id), &f).await?;
    Ok(Json(json!({ "items": items })))
}

pub async fn all_events(State(st): State<AppState>, Query(f): Query<EventFilter>) -> Result<Json<Value>> {
    let items = db::events::list(st.mgr.pool(), None, &f).await?;
    Ok(Json(json!({ "items": items })))
}

pub async fn list_tasks(State(st): State<AppState>, Path(id): Path<Uuid>) -> Result<Json<Value>> {
    let items = db::tasks::list(st.mgr.pool(), id, 100).await?;
    Ok(Json(json!({ "items": items })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewTask {
    pub kind: String,
    #[serde(default)]
    pub params: Value,
    #[serde(default)]
    pub force_online: bool,
}

pub async fn create_task(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<Uuid>,
    Json(t): Json<NewTask>,
) -> Result<(StatusCode, Json<Value>)> {
    caller.require_admin()?;
    db::npcs::require(st.mgr.pool(), id).await?;
    let params = if t.params.is_null() { json!({}) } else { t.params };
    manual::validate(&t.kind, &params)?;
    let task = db::tasks::create(st.mgr.pool(), id, &t.kind, &params, t.force_online, Some(&caller.label())).await?;
    st.mgr.events().info(
        id,
        "task_created",
        format!("task {} ({}) queued by {}", task.id, task.kind, caller.label()),
        Some(json!({ "task_id": task.id })),
    );
    st.mgr.kick_tasks(id).await;
    Ok((StatusCode::CREATED, Json(json!({ "task": task }))))
}

pub async fn cancel_task(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path((id, task_id)): Path<(Uuid, i64)>,
) -> Result<Json<Value>> {
    caller.require_admin()?;
    let task = db::tasks::get(st.mgr.pool(), task_id).await?;
    if task.npc_id != id {
        return Err(NpcError::NotFound(format!("task {task_id} of NPC {id}")));
    }
    let before = db::tasks::cancel(st.mgr.pool(), task_id).await?;
    if before == "running" {
        st.mgr.cancel_task(id, task_id).await;
    }
    let task = db::tasks::get(st.mgr.pool(), task_id).await?;
    Ok(Json(json!({ "task": task })))
}

/// What the task dialog of the admin panel offers.
pub async fn task_kinds() -> Json<Value> {
    let mut configs: HashMap<&str, Value> = HashMap::new();
    configs.insert("drive_to_factory", json!({ "destination": "string (e.g. F0003)", "max_speed_kmh": "number" }));
    configs.insert("take_a_walk", json!({ "radius_m": "number", "pause_s": "integer", "max_distance_m": "number" }));
    Json(json!({
        "kinds": [
            {
                "kind": "go_to_position",
                "params": {
                    "position": "{x, y, z} (required)",
                    "uuid": "object the position is local to (empty = the NPC's own frame)",
                    "hold_s": "seconds to stay once arrived (default 60)",
                    "timeout_s": "give up after (default 300)",
                },
            },
            {
                "kind": "run_action",
                "params": {
                    "action": "one of `actions` (required)",
                    "config": "overrides of that action's config block",
                    "duration_s": "focus on it for at most (default 300)",
                },
            },
            { "kind": "wait", "params": { "seconds": "1..3600 (required)" } },
        ],
        "actions": manual::manual_actions(),
        "actionConfigs": configs,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch(v: Value) -> NpcPatch {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn patch_distinguishes_null_from_absent() {
        let p = patch(json!({ "schedulePattern": null }));
        assert_eq!(p.schedule_pattern, Some(None));
        let p = patch(json!({}));
        assert_eq!(p.schedule_pattern, None);
        assert!(serde_json::from_value::<NpcPatch>(json!({ "password": "x" })).is_err());
    }

    #[test]
    fn rename_rebuilds_display_name() {
        let mut row = crate::db::npcs::tests::row();
        let renamed = apply_patch(&mut row, patch(json!({ "firstName": " Alain ", "lastName": "Deloin" }))).unwrap();
        assert!(renamed);
        assert_eq!(row.display_name, "Alain Deloin");
        assert!(!apply_patch(&mut row, patch(json!({ "enabled": false }))).unwrap());
    }

    #[test]
    fn manual_role_locks_and_unassigned_unlocks() {
        let mut row = crate::db::npcs::tests::row();
        apply_patch(&mut row, patch(json!({ "role": "custom", "actionProfile": ["take_a_walk"] }))).unwrap();
        assert!(row.role_locked);
        apply_patch(&mut row, patch(json!({ "role": "unassigned" }))).unwrap();
        assert!(!row.role_locked);
    }

    #[test]
    fn rejects_inconsistent_edits() {
        let mut row = crate::db::npcs::tests::row();
        assert!(apply_patch(&mut row, patch(json!({ "actionProfile": ["fly"] }))).is_err());
        let mut row = crate::db::npcs::tests::row();
        assert!(apply_patch(&mut row, patch(json!({ "role": "storekeeper" }))).is_err());
        let mut row = crate::db::npcs::tests::row();
        assert!(apply_patch(&mut row, patch(json!({ "placeWhereWork": { "uuid": "x" } }))).is_err());
        let mut row = crate::db::npcs::tests::row();
        assert!(apply_patch(&mut row, patch(json!({ "firstName": "" }))).is_err());
    }
}
