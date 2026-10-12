//! Supply orders (meta-missions) and their legs (ADR 0007).

use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::error::Result;

pub const OPEN_STATUSES: &[&str] = &["planned", "in_progress"];

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct SupplyOrder {
    pub id: Uuid,
    pub kind: String,
    pub federation_id: Uuid,
    pub resource_id: Uuid,
    pub quantity: i64,
    pub origin_village_id: Option<Uuid>,
    pub destination_village_id: Uuid,
    pub path: Vec<Uuid>,
    pub status: String,
    pub trigger: String,
    pub total_reward: i64,
    pub created_by: Option<String>,
    pub last_error: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Leg {
    pub id: Uuid,
    pub order_id: Uuid,
    pub seq: i32,
    pub from_village_id: Option<Uuid>,
    pub to_village_id: Uuid,
    pub from_warehouse_id: Option<Uuid>,
    pub to_warehouse_id: Option<Uuid>,
    pub quantity: i64,
    pub distance_m: f64,
    pub reward_amount: Option<i64>,
    pub status: String,
    pub hold_id: Option<Uuid>,
    pub mission_id: Option<Uuid>,
    pub attempts: i32,
    pub last_error: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub published_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub completed_at: Option<OffsetDateTime>,
}

/// A leg to create with its order.
#[derive(Debug, Clone)]
pub struct NewLeg {
    pub from: Option<Uuid>,
    pub to: Uuid,
    pub distance_m: f64,
}

#[derive(Debug, Clone)]
pub struct NewOrder {
    pub kind: &'static str,
    pub federation_id: Uuid,
    pub resource_id: Uuid,
    pub quantity: i64,
    pub origin: Option<Uuid>,
    pub destination: Uuid,
    pub path: Vec<Uuid>,
    pub trigger: &'static str,
    pub created_by: Option<String>,
    pub legs: Vec<NewLeg>,
}

/// Creates an order with its legs; the first leg is `pending`, the others `waiting`.
pub async fn create_order(pool: &PgPool, o: &NewOrder) -> Result<SupplyOrder> {
    let mut tx = pool.begin().await?;
    let order: SupplyOrder = sqlx::query_as(
        "INSERT INTO supply_orders (kind, federation_id, resource_id, quantity, origin_village_id,
            destination_village_id, path, status, trigger, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'in_progress', $8, $9) RETURNING *",
    )
    .bind(o.kind)
    .bind(o.federation_id)
    .bind(o.resource_id)
    .bind(o.quantity)
    .bind(o.origin)
    .bind(o.destination)
    .bind(&o.path)
    .bind(o.trigger)
    .bind(&o.created_by)
    .fetch_one(&mut *tx)
    .await?;
    for (seq, leg) in o.legs.iter().enumerate() {
        sqlx::query(
            "INSERT INTO supply_order_legs (order_id, seq, from_village_id, to_village_id, quantity, distance_m, status)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(order.id)
        .bind(seq as i32)
        .bind(leg.from)
        .bind(leg.to)
        .bind(o.quantity)
        .bind(leg.distance_m)
        .bind(if seq == 0 { "pending" } else { "waiting" })
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(order)
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderFilter {
    pub federation: Option<Uuid>,
    pub status: Option<String>,
    pub kind: Option<String>,
    pub resource: Option<Uuid>,
    /// Orders whose path goes through this village.
    pub village: Option<Uuid>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

pub async fn list_orders(pool: &PgPool, f: &OrderFilter) -> Result<(Vec<SupplyOrder>, i64)> {
    const WHERE: &str = "($1::UUID IS NULL OR federation_id = $1)
        AND ($2::TEXT IS NULL OR status = $2)
        AND ($3::TEXT IS NULL OR kind = $3)
        AND ($4::UUID IS NULL OR resource_id = $4)
        AND ($5::UUID IS NULL OR destination_village_id = $5 OR $5 = ANY(path))";
    let items = sqlx::query_as(&format!(
        "SELECT * FROM supply_orders WHERE {WHERE} ORDER BY created_at DESC, id LIMIT $6 OFFSET $7"
    ))
    .bind(f.federation)
    .bind(&f.status)
    .bind(&f.kind)
    .bind(f.resource)
    .bind(f.village)
    .bind(f.limit.unwrap_or(50).clamp(1, 200))
    .bind(f.offset.unwrap_or(0).max(0))
    .fetch_all(pool)
    .await?;
    let total = sqlx::query_scalar(&format!("SELECT count(*) FROM supply_orders WHERE {WHERE}"))
        .bind(f.federation)
        .bind(&f.status)
        .bind(&f.kind)
        .bind(f.resource)
        .bind(f.village)
        .fetch_one(pool)
        .await?;
    Ok((items, total))
}

pub async fn get_order(pool: &PgPool, id: Uuid) -> Result<Option<SupplyOrder>> {
    Ok(sqlx::query_as("SELECT * FROM supply_orders WHERE id = $1").bind(id).fetch_optional(pool).await?)
}

pub async fn order_legs(pool: &PgPool, order: Uuid) -> Result<Vec<Leg>> {
    Ok(sqlx::query_as(&format!("SELECT {LEG_COLS} FROM supply_order_legs WHERE order_id = $1 ORDER BY seq"))
        .bind(order)
        .fetch_all(pool)
        .await?)
}

pub async fn legs_of_orders(pool: &PgPool, orders: &[Uuid]) -> Result<Vec<Leg>> {
    Ok(sqlx::query_as(&format!("SELECT {LEG_COLS} FROM supply_order_legs WHERE order_id = ANY($1) ORDER BY order_id, seq"))
        .bind(orders)
        .fetch_all(pool)
        .await?)
}

const LEG_COLS: &str = "id, order_id, seq, from_village_id, to_village_id, from_warehouse_id, to_warehouse_id, quantity, distance_m, reward_amount,
    status, hold_id, mission_id, attempts, last_error, published_at, completed_at";

/// Legs in the given statuses, oldest first, with the federation of their order.
pub async fn legs_by_status(pool: &PgPool, federation: Uuid, statuses: &[&str]) -> Result<Vec<Leg>> {
    let statuses: Vec<String> = statuses.iter().map(|s| s.to_string()).collect();
    Ok(sqlx::query_as(&format!(
        "SELECT {} FROM supply_order_legs l JOIN supply_orders o ON o.id = l.order_id
         WHERE o.federation_id = $1 AND l.status = ANY($2) ORDER BY l.created_at, l.seq",
        LEG_COLS.split(',').map(|c| format!("l.{}", c.trim())).collect::<Vec<_>>().join(", ")
    ))
    .bind(federation)
    .bind(&statuses)
    .fetch_all(pool)
    .await?)
}

/// Goods reserved: where they are held (`from`, none for mining) and where they will be delivered.
pub async fn set_leg_held(pool: &PgPool, leg: Uuid, hold_id: Option<Uuid>, from: Option<Uuid>, to: Uuid) -> Result<()> {
    sqlx::query(
        "UPDATE supply_order_legs SET status = 'held', hold_id = $2, from_warehouse_id = $3, to_warehouse_id = $4,
            last_error = NULL, updated_at = now() WHERE id = $1",
    )
        .bind(leg)
        .bind(hold_id)
        .bind(from)
        .bind(to)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_leg_published(pool: &PgPool, leg: Uuid, mission_id: Uuid, reward: i64) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "UPDATE supply_order_legs SET status = 'published', mission_id = $2, reward_amount = $3,
            published_at = now(), last_error = NULL, updated_at = now() WHERE id = $1",
    )
    .bind(leg)
    .bind(mission_id)
    .bind(reward)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE supply_orders SET total_reward = (SELECT COALESCE(sum(reward_amount), 0) FROM supply_order_legs WHERE order_id = supply_orders.id),
            updated_at = now()
         WHERE id = (SELECT order_id FROM supply_order_legs WHERE id = $1)",
    )
    .bind(leg)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn set_leg_error(pool: &PgPool, leg: Uuid, error: &str) -> Result<()> {
    sqlx::query("UPDATE supply_order_legs SET last_error = $2, updated_at = now() WHERE id = $1")
        .bind(leg)
        .bind(error)
        .execute(pool)
        .await?;
    Ok(())
}

/// Mission cancelled or expired: back to `held` (the hold is kept) for another publication.
pub async fn retry_leg(pool: &PgPool, leg: Uuid, reason: &str) -> Result<()> {
    sqlx::query(
        "UPDATE supply_order_legs SET status = 'held', mission_id = NULL, attempts = attempts + 1,
            last_error = $2, updated_at = now() WHERE id = $1",
    )
    .bind(leg)
    .bind(reason)
    .execute(pool)
    .await?;
    Ok(())
}

/// Completes a leg and starts the next one, or completes the order.
pub async fn complete_leg(tx: &mut Transaction<'_, Postgres>, leg: &Leg) -> Result<()> {
    sqlx::query("UPDATE supply_order_legs SET status = 'completed', completed_at = now(), updated_at = now() WHERE id = $1")
        .bind(leg.id)
        .execute(&mut **tx)
        .await?;
    let next = sqlx::query(
        "UPDATE supply_order_legs SET status = 'pending', updated_at = now() WHERE order_id = $1 AND seq = $2 AND status = 'waiting'",
    )
    .bind(leg.order_id)
    .bind(leg.seq + 1)
    .execute(&mut **tx)
    .await?;
    if next.rows_affected() == 0 {
        sqlx::query("UPDATE supply_orders SET status = 'completed', updated_at = now() WHERE id = $1")
            .bind(leg.order_id)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

/// Ends an order (`failed` or `cancelled`) with all its unfinished legs.
pub async fn close_order(pool: &PgPool, order: Uuid, status: &str, error: Option<&str>) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE supply_orders SET status = $2, last_error = COALESCE($3, last_error), updated_at = now() WHERE id = $1")
        .bind(order)
        .bind(status)
        .bind(error)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "UPDATE supply_order_legs SET status = $2, updated_at = now()
         WHERE order_id = $1 AND status NOT IN ('completed', 'failed', 'cancelled')",
    )
    .bind(order)
    .bind(status)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Quantity of open orders heading to each (village, resource) of a federation.
pub async fn incoming(pool: &PgPool, federation: Uuid) -> Result<Vec<(Uuid, Uuid, i64)>> {
    Ok(sqlx::query_as(
        "SELECT destination_village_id, resource_id, sum(quantity)::BIGINT FROM supply_orders
         WHERE federation_id = $1 AND status IN ('planned', 'in_progress')
         GROUP BY destination_village_id, resource_id",
    )
    .bind(federation)
    .fetch_all(pool)
    .await?)
}

/// Quantity promised by each village to legs not yet held (per village, resource).
pub async fn outgoing(pool: &PgPool, federation: Uuid) -> Result<Vec<(Uuid, Uuid, i64)>> {
    Ok(sqlx::query_as(
        "SELECT l.from_village_id, o.resource_id, sum(l.quantity)::BIGINT FROM supply_order_legs l
         JOIN supply_orders o ON o.id = l.order_id
         WHERE o.federation_id = $1 AND o.status IN ('planned', 'in_progress')
           AND l.from_village_id IS NOT NULL AND l.status IN ('pending')
         GROUP BY l.from_village_id, o.resource_id",
    )
    .bind(federation)
    .fetch_all(pool)
    .await?)
}

/// Open order count per (destination, resource).
pub async fn open_order_count(pool: &PgPool, destination: Uuid, resource: Uuid) -> Result<i64> {
    Ok(sqlx::query_scalar(
        "SELECT count(*) FROM supply_orders WHERE destination_village_id = $1 AND resource_id = $2
         AND status IN ('planned', 'in_progress')",
    )
    .bind(destination)
    .bind(resource)
    .fetch_one(pool)
    .await?)
}

/// Active legs with their order's resource, for the network view.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ActiveLeg {
    pub leg_id: Uuid,
    pub order_id: Uuid,
    pub kind: String,
    pub resource_id: Uuid,
    pub from_village_id: Option<Uuid>,
    pub to_village_id: Uuid,
    pub from_warehouse_id: Option<Uuid>,
    pub to_warehouse_id: Option<Uuid>,
    pub quantity: i64,
    pub status: String,
    pub reward_amount: Option<i64>,
    pub mission_id: Option<Uuid>,
}

pub async fn active_legs(pool: &PgPool, federation: Uuid) -> Result<Vec<ActiveLeg>> {
    Ok(sqlx::query_as(
        "SELECT l.id AS leg_id, o.id AS order_id, o.kind, o.resource_id, l.from_village_id, l.to_village_id,
                l.from_warehouse_id, l.to_warehouse_id,
                l.quantity, l.status, l.reward_amount, l.mission_id
         FROM supply_order_legs l JOIN supply_orders o ON o.id = l.order_id
         WHERE o.federation_id = $1 AND l.status IN ('pending', 'held', 'published', 'waiting')
           AND o.status IN ('planned', 'in_progress')
         ORDER BY o.created_at, l.seq",
    )
    .bind(federation)
    .fetch_all(pool)
    .await?)
}
