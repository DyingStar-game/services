use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgPool, QueryBuilder, types::Json};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::error::Result;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct EventRow {
    pub id: i64,
    pub npc_id: Option<Uuid>,
    #[serde(with = "time::serde::rfc3339")]
    pub ts: OffsetDateTime,
    pub kind: String,
    pub level: String,
    pub message: String,
    pub data: Option<Json<Value>>,
}

/// An event waiting to be written (see [`crate::events::EventSink`]).
#[derive(Debug, Clone)]
pub struct NewEvent {
    pub npc_id: Option<Uuid>,
    pub ts: OffsetDateTime,
    pub kind: String,
    pub level: &'static str,
    pub message: String,
    pub data: Option<Value>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventFilter {
    pub kind: Option<String>,
    /// Only events with an id below this (pagination, newest first).
    pub before: Option<i64>,
    pub limit: Option<i64>,
}

pub async fn insert_batch(pool: &PgPool, events: &[NewEvent]) -> Result<()> {
    if events.is_empty() {
        return Ok(());
    }
    let mut qb = QueryBuilder::new("INSERT INTO npc_events (npc_id, ts, kind, level, message, data) ");
    qb.push_values(events, |mut b, e| {
        b.push_bind(e.npc_id)
            .push_bind(e.ts)
            .push_bind(&e.kind)
            .push_bind(e.level)
            .push_bind(&e.message)
            .push_bind(e.data.as_ref().map(Json));
    });
    match qb.build().execute(pool).await {
        Ok(_) => Ok(()),
        Err(sqlx::Error::Database(e)) if e.is_foreign_key_violation() => {
            // An NPC deleted meanwhile breaks the whole batch on the FK: retry
            // one by one, skipping its events, so the others still land.
            for e in events {
                let _ = sqlx::query(
                    "INSERT INTO npc_events (npc_id, ts, kind, level, message, data)
                     SELECT $1, $2, $3, $4, $5, $6
                     WHERE $1::UUID IS NULL OR EXISTS (SELECT 1 FROM npcs WHERE id = $1)",
                )
                .bind(e.npc_id)
                .bind(e.ts)
                .bind(&e.kind)
                .bind(e.level)
                .bind(&e.message)
                .bind(e.data.as_ref().map(Json))
                .execute(pool)
                .await;
            }
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}

/// Events of one NPC (`Some`) or service-wide ones and every NPC's (`None`),
/// newest first.
pub async fn list(pool: &PgPool, npc_id: Option<Uuid>, f: &EventFilter) -> Result<Vec<EventRow>> {
    Ok(sqlx::query_as(
        "SELECT id, npc_id, ts, kind, level, message, data FROM npc_events
         WHERE ($1::UUID IS NULL OR npc_id = $1)
           AND ($2::TEXT IS NULL OR kind = $2)
           AND ($3::BIGINT IS NULL OR id < $3)
         ORDER BY id DESC LIMIT $4",
    )
    .bind(npc_id)
    .bind(&f.kind)
    .bind(f.before)
    .bind(f.limit.unwrap_or(100).clamp(1, 1000))
    .fetch_all(pool)
    .await?)
}

/// Delete events older than `days`.
pub async fn prune(pool: &PgPool, days: i32) -> Result<u64> {
    Ok(sqlx::query("DELETE FROM npc_events WHERE ts < now() - make_interval(days => $1)")
        .bind(days)
        .execute(pool)
        .await?
        .rows_affected())
}
