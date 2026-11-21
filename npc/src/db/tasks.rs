use serde::Serialize;
use serde_json::Value;
use sqlx::{PgPool, types::Json};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::error::{NpcError, Result};

pub const KINDS: [&str; 3] = ["go_to_position", "run_action", "wait"];

/// A task given by hand from the admin panel.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct TaskRow {
    pub id: i64,
    pub npc_id: Uuid,
    pub kind: String,
    pub params: Json<Value>,
    pub force_online: bool,
    pub status: String,
    pub error: Option<String>,
    pub created_by: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub started_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub finished_at: Option<OffsetDateTime>,
}

const COLS: &str =
    "id, npc_id, kind, params, force_online, status, error, created_by, created_at, started_at, finished_at";

pub async fn create(
    pool: &PgPool,
    npc_id: Uuid,
    kind: &str,
    params: &Value,
    force_online: bool,
    by: Option<&str>,
) -> Result<TaskRow> {
    Ok(sqlx::query_as(&format!(
        "INSERT INTO npc_tasks (npc_id, kind, params, force_online, created_by)
         VALUES ($1, $2, $3, $4, $5) RETURNING {COLS}"
    ))
    .bind(npc_id)
    .bind(kind)
    .bind(Json(params))
    .bind(force_online)
    .bind(by)
    .fetch_one(pool)
    .await?)
}

/// Open tasks first, then the most recent finished ones.
pub async fn list(pool: &PgPool, npc_id: Uuid, limit: i64) -> Result<Vec<TaskRow>> {
    Ok(sqlx::query_as(&format!(
        "SELECT {COLS} FROM npc_tasks WHERE npc_id = $1
         ORDER BY (status IN ('pending', 'running')) DESC, id DESC LIMIT $2"
    ))
    .bind(npc_id)
    .bind(limit.clamp(1, 500))
    .fetch_all(pool)
    .await?)
}

pub async fn get(pool: &PgPool, id: i64) -> Result<TaskRow> {
    sqlx::query_as(&format!("SELECT {COLS} FROM npc_tasks WHERE id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| NpcError::NotFound(format!("task {id}")))
}

/// Take the oldest pending task of `npc_id` and mark it running.
pub async fn claim_next(pool: &PgPool, npc_id: Uuid) -> Result<Option<TaskRow>> {
    Ok(sqlx::query_as(&format!(
        "UPDATE npc_tasks SET status = 'running', started_at = now()
         WHERE id = (SELECT id FROM npc_tasks WHERE npc_id = $1 AND status = 'pending'
                     ORDER BY id LIMIT 1 FOR UPDATE SKIP LOCKED)
         RETURNING {COLS}"
    ))
    .bind(npc_id)
    .fetch_optional(pool)
    .await?)
}

/// Close a running task (`done`, `failed`, `timeout`, `cancelled`).
pub async fn finish(pool: &PgPool, id: i64, status: &str, error: Option<&str>) -> Result<()> {
    sqlx::query(
        "UPDATE npc_tasks SET status = $2, error = $3, finished_at = now()
         WHERE id = $1 AND status IN ('pending', 'running')",
    )
    .bind(id)
    .bind(status)
    .bind(error)
    .execute(pool)
    .await?;
    Ok(())
}

/// Cancel an open task. Returns its status before the cancel.
pub async fn cancel(pool: &PgPool, id: i64) -> Result<String> {
    let before: Option<String> = sqlx::query_scalar(
        "UPDATE npc_tasks t SET status = 'cancelled', finished_at = now(), error = 'cancelled by an admin'
         FROM (SELECT id, status FROM npc_tasks WHERE id = $1 FOR UPDATE) old
         WHERE t.id = old.id AND old.status IN ('pending', 'running')
         RETURNING old.status",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    before.ok_or_else(|| NpcError::Conflict(format!("task {id} is not open")))
}

/// Whether `npc_id` has an open task that asks to connect it off-shift.
pub async fn has_open_force_online(pool: &PgPool, npc_id: Uuid) -> Result<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM npc_tasks
                        WHERE npc_id = $1 AND force_online AND status IN ('pending', 'running'))",
    )
    .bind(npc_id)
    .fetch_one(pool)
    .await?)
}

/// Tasks a previous process left running cannot resume: fail them at startup.
pub async fn fail_orphans(pool: &PgPool) -> Result<u64> {
    Ok(sqlx::query(
        "UPDATE npc_tasks SET status = 'failed', error = 'service restarted', finished_at = now()
         WHERE status = 'running'",
    )
    .execute(pool)
    .await?
    .rows_affected())
}

/// Delete finished tasks older than `days`.
pub async fn prune(pool: &PgPool, days: i32) -> Result<u64> {
    Ok(sqlx::query("DELETE FROM npc_tasks WHERE finished_at < now() - make_interval(days => $1)")
        .bind(days)
        .execute(pool)
        .await?
        .rows_affected())
}
