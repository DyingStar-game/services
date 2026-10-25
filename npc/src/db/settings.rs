use serde::Serialize;
use sqlx::PgPool;
use time::OffsetDateTime;

use crate::error::Result;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub npcs_enabled: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    pub updated_by: Option<String>,
}

pub async fn get(pool: &PgPool) -> Result<Settings> {
    Ok(sqlx::query_as("SELECT npcs_enabled, updated_at, updated_by FROM settings")
        .fetch_one(pool)
        .await?)
}

pub async fn set_enabled(pool: &PgPool, enabled: bool, by: Option<&str>) -> Result<Settings> {
    Ok(sqlx::query_as(
        "UPDATE settings SET npcs_enabled = $1, updated_at = now(), updated_by = $2
         RETURNING npcs_enabled, updated_at, updated_by",
    )
    .bind(enabled)
    .bind(by)
    .fetch_one(pool)
    .await?)
}
