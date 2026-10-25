use serde::Serialize;
use sqlx::PgPool;
use time::OffsetDateTime;

use crate::error::{NpcError, Result};

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct PatternRow {
    pub name: String,
    pub description: Option<String>,
    pub days: Vec<String>,
    pub source: String,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    pub updated_by: Option<String>,
}

const COLS: &str = "name, description, days, source, updated_at, updated_by";

pub async fn list(pool: &PgPool) -> Result<Vec<PatternRow>> {
    Ok(sqlx::query_as(&format!("SELECT {COLS} FROM schedule_patterns ORDER BY name"))
        .fetch_all(pool)
        .await?)
}

pub async fn get(pool: &PgPool, name: &str) -> Result<Option<PatternRow>> {
    Ok(sqlx::query_as(&format!("SELECT {COLS} FROM schedule_patterns WHERE name = $1"))
        .bind(name)
        .fetch_optional(pool)
        .await?)
}

pub async fn count(pool: &PgPool) -> Result<i64> {
    Ok(sqlx::query_scalar("SELECT count(*) FROM schedule_patterns").fetch_one(pool).await?)
}

/// Insert or replace. Returns `true` when the row was created.
pub async fn upsert(
    pool: &PgPool,
    name: &str,
    description: Option<&str>,
    days: &[String],
    source: &str,
    by: Option<&str>,
) -> Result<bool> {
    let created: bool = sqlx::query_scalar(
        "INSERT INTO schedule_patterns (name, description, days, source, updated_by)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (name) DO UPDATE SET
             description = EXCLUDED.description, days = EXCLUDED.days,
             source = EXCLUDED.source, updated_by = EXCLUDED.updated_by, updated_at = now()
         RETURNING (xmax = 0)",
    )
    .bind(name)
    .bind(description)
    .bind(days)
    .bind(source)
    .bind(by)
    .fetch_one(pool)
    .await?;
    Ok(created)
}

/// Insert only when absent. Returns `true` when the row was created.
pub async fn insert_missing(
    pool: &PgPool,
    name: &str,
    description: Option<&str>,
    days: &[String],
    by: Option<&str>,
) -> Result<bool> {
    let res = sqlx::query(
        "INSERT INTO schedule_patterns (name, description, days, source, updated_by)
         VALUES ($1, $2, $3, 'import', $4) ON CONFLICT (name) DO NOTHING",
    )
    .bind(name)
    .bind(description)
    .bind(days)
    .bind(by)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() == 1)
}

/// Delete a pattern no NPC uses (409 otherwise).
pub async fn delete(pool: &PgPool, name: &str) -> Result<()> {
    let users: i64 = sqlx::query_scalar("SELECT count(*) FROM npcs WHERE schedule_pattern = $1")
        .bind(name)
        .fetch_one(pool)
        .await?;
    if users > 0 {
        return Err(NpcError::Conflict(format!("pattern '{name}' is used by {users} NPC(s)")));
    }
    let res = sqlx::query("DELETE FROM schedule_patterns WHERE name = $1")
        .bind(name)
        .execute(pool)
        .await?;
    if res.rows_affected() == 0 {
        return Err(NpcError::NotFound(format!("pattern '{name}'")));
    }
    Ok(())
}

pub async fn list_activity_actions(pool: &PgPool) -> Result<Vec<(String, Vec<String>)>> {
    Ok(sqlx::query_as("SELECT activity, actions FROM activity_actions ORDER BY activity")
        .fetch_all(pool)
        .await?)
}

pub async fn upsert_activity_actions(
    pool: &PgPool,
    activity: &str,
    actions: &[String],
    by: Option<&str>,
) -> Result<bool> {
    let created: bool = sqlx::query_scalar(
        "INSERT INTO activity_actions (activity, actions, updated_by) VALUES ($1, $2, $3)
         ON CONFLICT (activity) DO UPDATE SET
             actions = EXCLUDED.actions, updated_by = EXCLUDED.updated_by, updated_at = now()
         RETURNING (xmax = 0)",
    )
    .bind(activity)
    .bind(actions)
    .bind(by)
    .fetch_one(pool)
    .await?;
    Ok(created)
}

pub async fn insert_missing_activity_actions(
    pool: &PgPool,
    activity: &str,
    actions: &[String],
    by: Option<&str>,
) -> Result<bool> {
    let res = sqlx::query(
        "INSERT INTO activity_actions (activity, actions, updated_by) VALUES ($1, $2, $3)
         ON CONFLICT (activity) DO NOTHING",
    )
    .bind(activity)
    .bind(actions)
    .bind(by)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() == 1)
}
