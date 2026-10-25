use serde::Serialize;
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::error::Result;

/// Where a village's storekeepers work: the cargo_depot object (its uuid is the
/// universesimulator warehouse id and the Horizon object uuid).
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Workplace {
    pub village_code: String,
    pub warehouse_id: Uuid,
    pub warehouse_code: Option<String>,
    pub scenename: Option<String>,
    pub source: String,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    pub updated_by: Option<String>,
}

const COLS: &str = "village_code, warehouse_id, warehouse_code, scenename, source, updated_at, updated_by";

pub async fn get(pool: &PgPool, village: &str) -> Result<Option<Workplace>> {
    Ok(sqlx::query_as(&format!("SELECT {COLS} FROM village_workplaces WHERE village_code = $1"))
        .bind(village)
        .fetch_optional(pool)
        .await?)
}

pub async fn list(pool: &PgPool) -> Result<Vec<Workplace>> {
    Ok(sqlx::query_as(&format!("SELECT {COLS} FROM village_workplaces ORDER BY village_code"))
        .fetch_all(pool)
        .await?)
}

/// Record a resolved workplace. A `manual` one is never replaced by an
/// automatic source.
pub async fn upsert(
    pool: &PgPool,
    village: &str,
    warehouse_id: Uuid,
    warehouse_code: Option<&str>,
    scenename: Option<&str>,
    source: &str,
    by: Option<&str>,
) -> Result<Workplace> {
    let updated: Option<Workplace> = sqlx::query_as(&format!(
        "INSERT INTO village_workplaces (village_code, warehouse_id, warehouse_code, scenename, source, updated_by)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (village_code) DO UPDATE SET
             warehouse_id = EXCLUDED.warehouse_id, warehouse_code = EXCLUDED.warehouse_code,
             scenename = EXCLUDED.scenename, source = EXCLUDED.source,
             updated_by = EXCLUDED.updated_by, updated_at = now()
         WHERE village_workplaces.source <> 'manual' OR EXCLUDED.source = 'manual'
         RETURNING {COLS}"
    ))
    .bind(village)
    .bind(warehouse_id)
    .bind(warehouse_code)
    .bind(scenename)
    .bind(source)
    .bind(by)
    .fetch_optional(pool)
    .await?;
    match updated {
        Some(w) => Ok(w),
        // A manual workplace stood in the way: it is the answer.
        None => get(pool, village)
            .await?
            .ok_or_else(|| crate::error::NpcError::NotFound(format!("workplace of {village}"))),
    }
}

pub async fn delete(pool: &PgPool, village: &str) -> Result<bool> {
    let res = sqlx::query("DELETE FROM village_workplaces WHERE village_code = $1")
        .bind(village)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}
