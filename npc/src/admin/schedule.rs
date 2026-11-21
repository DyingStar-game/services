//! `/api/admin/schedule…`: shift patterns, the activity → actions mapping and
//! the import of `config/schedules.yaml`. Every write reloads the schedules in
//! force; running NPCs pick the change up on their next tick.

use std::collections::BTreeMap;

use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    auth::Caller,
    db::patterns,
    error::{NpcError, Result},
    schedule::validate_pattern,
    schedule_import::{ImportMode, import_schedules},
    state::AppState,
};

const ACTIVITIES: [&str; 6] = ["work", "transmission", "meal", "mandatory", "off", "always"];

async fn reload(st: &AppState) -> Result<()> {
    st.mgr.svc.schedule.reload(st.mgr.pool()).await?;
    st.mgr.poke();
    Ok(())
}

pub async fn list(State(st): State<AppState>) -> Result<Json<Value>> {
    let items = patterns::list(st.mgr.pool()).await?;
    Ok(Json(json!({ "items": items })))
}

pub async fn get(State(st): State<AppState>, Path(name): Path<String>) -> Result<Json<Value>> {
    let p = patterns::get(st.mgr.pool(), &name)
        .await?
        .ok_or_else(|| NpcError::NotFound(format!("pattern '{name}'")))?;
    Ok(Json(json!({ "pattern": p })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatternBody {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    pub days: Vec<String>,
}

pub async fn create(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Json(b): Json<PatternBody>,
) -> Result<(StatusCode, Json<Value>)> {
    caller.require_admin()?;
    let name = b.name.clone().ok_or_else(|| NpcError::BadRequest("name is required".into()))?;
    validate_pattern(&name, &b.days).map_err(NpcError::BadRequest)?;
    if patterns::get(st.mgr.pool(), &name).await?.is_some() {
        return Err(NpcError::Conflict(format!("pattern '{name}' already exists")));
    }
    patterns::upsert(st.mgr.pool(), &name, b.description.as_deref(), &b.days, "panel", Some(&caller.label())).await?;
    reload(&st).await?;
    st.mgr.events().global("pattern_created", format!("pattern {name} created by {}", caller.label()), None);
    let p = patterns::get(st.mgr.pool(), &name).await?;
    Ok((StatusCode::CREATED, Json(json!({ "pattern": p }))))
}

pub async fn update(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(name): Path<String>,
    Json(b): Json<PatternBody>,
) -> Result<Json<Value>> {
    caller.require_admin()?;
    validate_pattern(&name, &b.days).map_err(NpcError::BadRequest)?;
    if patterns::get(st.mgr.pool(), &name).await?.is_none() {
        return Err(NpcError::NotFound(format!("pattern '{name}'")));
    }
    patterns::upsert(st.mgr.pool(), &name, b.description.as_deref(), &b.days, "panel", Some(&caller.label())).await?;
    reload(&st).await?;
    st.mgr.events().global("pattern_updated", format!("pattern {name} edited by {}", caller.label()), None);
    let p = patterns::get(st.mgr.pool(), &name).await?;
    Ok(Json(json!({ "pattern": p })))
}

pub async fn delete(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(name): Path<String>,
) -> Result<StatusCode> {
    caller.require_admin()?;
    patterns::delete(st.mgr.pool(), &name).await?;
    reload(&st).await?;
    st.mgr.events().global("pattern_deleted", format!("pattern {name} deleted by {}", caller.label()), None);
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub struct ImportBody {
    #[serde(default)]
    pub mode: ImportMode,
    /// YAML text in the format of `config/schedules.yaml`; absent = that file.
    #[serde(default)]
    pub yaml: Option<String>,
}

pub async fn import(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Json(b): Json<ImportBody>,
) -> Result<Json<Value>> {
    caller.require_admin()?;
    let yaml = match b.yaml {
        Some(y) => y,
        None => std::fs::read_to_string(&st.mgr.svc.config.schedules_path)?,
    };
    let by = caller.label();
    let report = import_schedules(st.mgr.pool(), &yaml, b.mode, Some(&by)).await?;
    if report.changed() {
        reload(&st).await?;
    }
    st.mgr.events().global(
        "schedules_imported",
        format!(
            "schedules imported by {by}: {} created, {} updated, {} errors",
            report.created.len(),
            report.updated.len(),
            report.errors.len()
        ),
        Some(json!({ "mode": b.mode })),
    );
    Ok(Json(json!({ "report": report })))
}

pub async fn activity_actions(State(st): State<AppState>) -> Json<Value> {
    Json(json!({ "items": st.mgr.svc.schedule.load().activity_actions() }))
}

/// Replace the actions of the activities given (others unchanged).
pub async fn set_activity_actions(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Json(b): Json<BTreeMap<String, Vec<String>>>,
) -> Result<Json<Value>> {
    caller.require_admin()?;
    let known = super::npcs::catalogue_names();
    for (activity, actions) in &b {
        if !ACTIVITIES.contains(&activity.as_str()) {
            return Err(NpcError::BadRequest(format!("unknown activity '{activity}' (expected {ACTIVITIES:?})")));
        }
        if let Some(bad) = actions.iter().find(|a| !known.contains(&a.as_str())) {
            return Err(NpcError::BadRequest(format!("unknown action '{bad}'")));
        }
    }
    for (activity, actions) in &b {
        patterns::upsert_activity_actions(st.mgr.pool(), activity, actions, Some(&caller.label())).await?;
    }
    reload(&st).await?;
    st.mgr.events().global("activity_actions_updated", format!("activity mapping edited by {}", caller.label()), None);
    Ok(Json(json!({ "items": st.mgr.svc.schedule.load().activity_actions() })))
}

/// What each grid letter means, for the pattern editor.
pub async fn legend() -> Json<Value> {
    Json(json!({
        "hoursPerDay": 25,
        "letters": [
            { "letter": "T", "activity": "work", "label": "travail" },
            { "letter": "t", "activity": "transmission", "label": "transmission (relève)" },
            { "letter": "r", "activity": "meal", "label": "repas" },
            { "letter": "a", "activity": "mandatory", "label": "activités obligatoires" },
            { "letter": ".", "activity": "off", "label": "repos (maison)" },
            { "letter": "l", "activity": "off", "label": "loisir (pas encore d'action : comme repos)" },
        ],
    }))
}
