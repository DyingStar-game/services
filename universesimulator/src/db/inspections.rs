//! Inspection sites and the mission open on each (ADR 0019).

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::Result;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Site {
    pub id: Uuid,
    pub village_id: Uuid,
    pub name: String,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub distance_m: f64,
    pub inventory_poi_id: Option<Uuid>,
    pub mission_id: Option<Uuid>,
    pub published_at: Option<time::OffsetDateTime>,
    pub attempts: i32,
}

const COLS: &str = "id, village_id, name, x, y, z, distance_m, inventory_poi_id, mission_id, published_at, attempts";

/// The federation's sites, a village's together, the one to inspect next first (never inspected, then
/// the longest ago).
pub async fn list(pool: &PgPool, federation: Uuid) -> Result<Vec<Site>> {
    Ok(sqlx::query_as(&format!(
        "SELECT {COLS} FROM inspection_sites WHERE federation_id = $1 ORDER BY village_id, last_inspected_at NULLS FIRST, seq"
    ))
    .bind(federation)
    .fetch_all(pool)
    .await?)
}

/// Makes the site [seq] of [village]; nothing when it exists.
pub async fn insert(pool: &PgPool, federation: Uuid, village: Uuid, seq: i32, name: &str, [x, y, z]: [f64; 3], distance_m: f64) -> Result<()> {
    sqlx::query(
        "INSERT INTO inspection_sites (federation_id, village_id, seq, name, x, y, z, distance_m)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) ON CONFLICT (village_id, seq) DO NOTHING",
    )
    .bind(federation)
    .bind(village)
    .bind(seq)
    .bind(name)
    .bind(x)
    .bind(y)
    .bind(z)
    .bind(distance_m)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn set_poi(pool: &PgPool, site: Uuid, poi: Uuid) -> Result<()> {
    sqlx::query("UPDATE inspection_sites SET inventory_poi_id = $2 WHERE id = $1").bind(site).bind(poi).execute(pool).await?;
    Ok(())
}

pub async fn set_published(pool: &PgPool, site: Uuid, mission: Uuid) -> Result<()> {
    sqlx::query("UPDATE inspection_sites SET mission_id = $2, published_at = now() WHERE id = $1").bind(site).bind(mission).execute(pool).await?;
    Ok(())
}

/// The site's mission is over: [inspected] when it was done, else cancelled or expired. Its next mission
/// is another attempt.
pub async fn close(pool: &PgPool, site: Uuid, inspected: bool) -> Result<()> {
    sqlx::query(
        "UPDATE inspection_sites SET mission_id = NULL, published_at = NULL, attempts = attempts + 1,
         last_inspected_at = CASE WHEN $2 THEN now() ELSE last_inspected_at END WHERE id = $1",
    )
    .bind(site)
    .bind(inspected)
    .execute(pool)
    .await?;
    Ok(())
}
