//! The machines of the villages and their breakdowns (ADR 0020).

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::Result;

/// A breakdown with its machine.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Breakdown {
    pub id: Uuid,
    pub machine_id: Uuid,
    pub village_id: Uuid,
    pub kind: String,
    pub repaired_at: Option<time::OffsetDateTime>,
    pub mission_id: Option<Uuid>,
    pub published_at: Option<time::OffsetDateTime>,
    pub attempts: i32,
}

const COLS: &str = "b.id, b.machine_id, m.village_id, m.kind, b.repaired_at, b.mission_id, b.published_at, b.attempts";

/// The game server lists a machine: made, or seen again (its village and kind as said now).
pub async fn upsert(pool: &PgPool, id: Uuid, federation: Uuid, village: Uuid, kind: &str) -> Result<()> {
    sqlx::query(
        "INSERT INTO machines (id, federation_id, village_id, kind) VALUES ($1, $2, $3, $4)
         ON CONFLICT (id) DO UPDATE SET federation_id = $2, village_id = $3, kind = $4, seen_at = now()",
    )
    .bind(id)
    .bind(federation)
    .bind(village)
    .bind(kind)
    .execute(pool)
    .await?;
    Ok(())
}

/// The breakdowns of a federation the tick has work on: the open ones, and the repaired ones whose
/// mission was not read as ended yet. The oldest first.
pub async fn pending(pool: &PgPool, federation: Uuid) -> Result<Vec<Breakdown>> {
    Ok(sqlx::query_as(&format!(
        "SELECT {COLS} FROM machine_breakdowns b JOIN machines m ON m.id = b.machine_id
         WHERE m.federation_id = $1 AND (b.repaired_at IS NULL OR b.mission_id IS NOT NULL) ORDER BY b.broken_at, b.id"
    ))
    .bind(federation)
    .fetch_all(pool)
    .await?)
}

/// The open breakdowns standing on the planet [scene], whatever their federation: (machine, breakdown,
/// mission).
pub async fn open_on(pool: &PgPool, scene: &str) -> Result<Vec<(Uuid, Uuid, Option<Uuid>)>> {
    Ok(sqlx::query_as(
        "SELECT b.machine_id, b.id, b.mission_id FROM machine_breakdowns b
         JOIN machines m ON m.id = b.machine_id JOIN federations f ON f.id = m.federation_id
         WHERE b.repaired_at IS NULL AND f.planet_internal_name = $1 ORDER BY b.broken_at, b.id",
    )
    .bind(scene)
    .fetch_all(pool)
    .await?)
}

/// Every open breakdown: (machine, its kind).
pub async fn open_all(pool: &PgPool) -> Result<Vec<(Uuid, String)>> {
    Ok(sqlx::query_as(
        "SELECT b.machine_id, m.kind FROM machine_breakdowns b JOIN machines m ON m.id = b.machine_id
         WHERE b.repaired_at IS NULL ORDER BY b.broken_at, b.id",
    )
    .fetch_all(pool)
    .await?)
}

/// The machine to break next: seen by the game server in the last day, in an active village where
/// nothing is broken; the one repaired the longest ago (never, first), then by its id's hash -- no kind
/// nor village is always first.
pub async fn next_to_break(pool: &PgPool, federation: Uuid) -> Result<Option<Uuid>> {
    Ok(sqlx::query_scalar(
        "SELECT m.id FROM machines m JOIN villages v ON v.id = m.village_id
         WHERE m.federation_id = $1 AND v.active AND m.seen_at > now() - INTERVAL '1 day'
           AND NOT EXISTS (
               SELECT 1 FROM machine_breakdowns b JOIN machines o ON o.id = b.machine_id
               WHERE o.village_id = m.village_id AND b.repaired_at IS NULL)
         ORDER BY m.last_repaired_at NULLS FIRST, md5(m.id::text) LIMIT 1",
    )
    .bind(federation)
    .fetch_optional(pool)
    .await?)
}

pub async fn break_machine(pool: &PgPool, machine: Uuid) -> Result<()> {
    sqlx::query("INSERT INTO machine_breakdowns (machine_id) VALUES ($1) ON CONFLICT DO NOTHING").bind(machine).execute(pool).await?;
    Ok(())
}

pub async fn set_published(pool: &PgPool, breakdown: Uuid, mission: Uuid) -> Result<()> {
    sqlx::query("UPDATE machine_breakdowns SET mission_id = $2, published_at = now() WHERE id = $1").bind(breakdown).bind(mission).execute(pool).await?;
    Ok(())
}

/// The breakdown's mission is over. Its next one, if the machine is still broken, is another attempt.
pub async fn clear_mission(pool: &PgPool, breakdown: Uuid) -> Result<()> {
    sqlx::query("UPDATE machine_breakdowns SET mission_id = NULL, published_at = NULL, attempts = attempts + 1 WHERE id = $1")
        .bind(breakdown)
        .execute(pool)
        .await?;
    Ok(())
}

/// The machine works again, repaired by [by] (None: not said). Once: returns whether this call did it.
pub async fn repair(pool: &PgPool, breakdown: Uuid, by: Option<Uuid>) -> Result<bool> {
    let machine: Option<Uuid> = sqlx::query_scalar(
        "UPDATE machine_breakdowns SET repaired_at = now(), repaired_by = $2 WHERE id = $1 AND repaired_at IS NULL RETURNING machine_id",
    )
    .bind(breakdown)
    .bind(by)
    .fetch_optional(pool)
    .await?;
    let Some(machine) = machine else { return Ok(false) };
    sqlx::query("UPDATE machines SET last_repaired_at = now() WHERE id = $1").bind(machine).execute(pool).await?;
    Ok(true)
}
