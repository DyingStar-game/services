//! History, prices, treasury events, deliveries and simulation state.

use serde::Serialize;
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::error::Result;

// ── Stock history ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct StockPoint {
    pub quantity: i64,
    pub held: i64,
    #[serde(with = "time::serde::rfc3339")]
    pub tick_at: OffsetDateTime,
}

/// Snapshots `stock_levels` of a federation into `stock_history`.
pub async fn snapshot_stocks(pool: &PgPool, federation: Uuid) -> Result<()> {
    sqlx::query(
        "INSERT INTO stock_history (warehouse_id, resource_id, quantity, held)
         SELECT s.warehouse_id, s.resource_id, s.quantity, s.held FROM stock_levels s
         JOIN warehouses w ON w.id = s.warehouse_id JOIN villages v ON v.id = w.village_id
         WHERE v.federation_id = $1 AND w.active",
    )
    .bind(federation)
    .execute(pool)
    .await?;
    Ok(())
}

/// History of a warehouse, or of a village (sum of its warehouses at each tick).
pub async fn stock_history(pool: &PgPool, warehouse: Option<Uuid>, village: Option<Uuid>, resource: Uuid, from: OffsetDateTime) -> Result<Vec<StockPoint>> {
    Ok(sqlx::query_as(
        "SELECT sum(h.quantity)::BIGINT AS quantity, sum(h.held)::BIGINT AS held, h.tick_at FROM stock_history h
         JOIN warehouses w ON w.id = h.warehouse_id
         WHERE ($1::UUID IS NULL OR h.warehouse_id = $1) AND ($2::UUID IS NULL OR w.village_id = $2)
           AND h.resource_id = $3 AND h.tick_at >= $4
         GROUP BY h.tick_at ORDER BY h.tick_at LIMIT 5000",
    )
    .bind(warehouse)
    .bind(village)
    .bind(resource)
    .bind(from)
    .fetch_all(pool)
    .await?)
}

pub async fn purge_history(pool: &PgPool, days: i64) -> Result<()> {
    let cutoff = OffsetDateTime::now_utc() - time::Duration::days(days);
    sqlx::query("DELETE FROM stock_history WHERE tick_at < $1").bind(cutoff).execute(pool).await?;
    sqlx::query("DELETE FROM price_history WHERE computed_at < $1").bind(cutoff).execute(pool).await?;
    Ok(())
}

// ── Prices ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct PriceIndex {
    pub federation_id: Uuid,
    pub resource_id: Uuid,
    pub supply: i64,
    pub demand: i64,
    pub unit_price: f64,
    #[serde(with = "time::serde::rfc3339")]
    pub computed_at: OffsetDateTime,
}

pub async fn set_price_index(pool: &PgPool, federation: Uuid, resource: Uuid, supply: i64, demand: i64, unit_price: f64) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO price_index (federation_id, resource_id, supply, demand, unit_price, computed_at)
         VALUES ($1, $2, $3, $4, $5, now())
         ON CONFLICT (federation_id, resource_id) DO UPDATE SET supply = $3, demand = $4, unit_price = $5, computed_at = now()",
    )
    .bind(federation)
    .bind(resource)
    .bind(supply)
    .bind(demand)
    .bind(unit_price)
    .execute(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO price_history (federation_id, resource_id, supply, demand, unit_price) VALUES ($1, $2, $3, $4, $5)")
        .bind(federation)
        .bind(resource)
        .bind(supply)
        .bind(demand)
        .bind(unit_price)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn price_indices(pool: &PgPool, federation: Uuid) -> Result<Vec<PriceIndex>> {
    Ok(sqlx::query_as("SELECT * FROM price_index WHERE federation_id = $1").bind(federation).fetch_all(pool).await?)
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct PricePoint {
    pub supply: i64,
    pub demand: i64,
    pub unit_price: f64,
    #[serde(with = "time::serde::rfc3339")]
    pub computed_at: OffsetDateTime,
}

pub async fn price_history(pool: &PgPool, federation: Uuid, resource: Uuid, from: OffsetDateTime) -> Result<Vec<PricePoint>> {
    Ok(sqlx::query_as(
        "SELECT supply, demand, unit_price, computed_at FROM price_history
         WHERE federation_id = $1 AND resource_id = $2 AND computed_at >= $3 ORDER BY computed_at LIMIT 5000",
    )
    .bind(federation)
    .bind(resource)
    .bind(from)
    .fetch_all(pool)
    .await?)
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct LocalPrice {
    pub village_id: Uuid,
    pub resource_id: Uuid,
    pub local_factor: f64,
    pub buy_price: f64,
    pub sell_price: f64,
    #[serde(with = "time::serde::rfc3339")]
    pub computed_at: OffsetDateTime,
}

pub async fn set_local_price(pool: &PgPool, village: Uuid, resource: Uuid, factor: f64, buy: f64, sell: f64) -> Result<()> {
    sqlx::query(
        "INSERT INTO local_prices (village_id, resource_id, local_factor, buy_price, sell_price, computed_at)
         VALUES ($1, $2, $3, $4, $5, now())
         ON CONFLICT (village_id, resource_id) DO UPDATE SET local_factor = $3, buy_price = $4, sell_price = $5, computed_at = now()",
    )
    .bind(village)
    .bind(resource)
    .bind(factor)
    .bind(buy)
    .bind(sell)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn local_prices(pool: &PgPool, federation: Uuid, resource: Option<Uuid>, village: Option<Uuid>) -> Result<Vec<LocalPrice>> {
    Ok(sqlx::query_as(
        "SELECT p.* FROM local_prices p JOIN villages v ON v.id = p.village_id
         WHERE v.federation_id = $1 AND ($2::UUID IS NULL OR p.resource_id = $2) AND ($3::UUID IS NULL OR p.village_id = $3)
         ORDER BY p.sell_price - p.buy_price DESC",
    )
    .bind(federation)
    .bind(resource)
    .bind(village)
    .fetch_all(pool)
    .await?)
}

// ── Treasury ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct MintEvent {
    pub id: Uuid,
    pub federation_id: Uuid,
    pub amount: i64,
    pub reason: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

pub async fn record_mint(pool: &PgPool, federation: Uuid, amount: i64, reason: &str) -> Result<()> {
    sqlx::query("INSERT INTO mint_events (federation_id, amount, reason) VALUES ($1, $2, $3)")
        .bind(federation)
        .bind(amount)
        .bind(reason)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn mint_events(pool: &PgPool, federation: Uuid, limit: i64) -> Result<Vec<MintEvent>> {
    Ok(sqlx::query_as("SELECT * FROM mint_events WHERE federation_id = $1 ORDER BY created_at DESC LIMIT $2")
        .bind(federation)
        .bind(limit.clamp(1, 500))
        .fetch_all(pool)
        .await?)
}

pub async fn minted_since(pool: &PgPool, federation: Uuid, since: OffsetDateTime) -> Result<i64> {
    Ok(sqlx::query_scalar("SELECT COALESCE(sum(amount), 0)::BIGINT FROM mint_events WHERE federation_id = $1 AND created_at >= $2")
        .bind(federation)
        .bind(since)
        .fetch_one(pool)
        .await?)
}

// ── Deliveries ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Delivery {
    pub id: Uuid,
    pub idempotency_key: String,
    #[serde(skip)]
    pub request_hash: String,
    pub warehouse_id: Uuid,
    pub resource_id: Uuid,
    pub requested_quantity: i64,
    pub accepted_quantity: i64,
    pub deliverer_type: String,
    pub deliverer_id: Uuid,
    pub status: String,
    pub stock_after: Option<i64>,
    pub metadata: serde_json::Value,
    pub last_error: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

const DELIVERY_COLS: &str = "id, idempotency_key, request_hash, warehouse_id, resource_id, requested_quantity, accepted_quantity,
    deliverer_type, deliverer_id, status, stock_after, metadata, last_error, created_at";

pub async fn get_delivery(pool: &PgPool, key: &str) -> Result<Option<Delivery>> {
    Ok(sqlx::query_as(&format!("SELECT {DELIVERY_COLS} FROM warehouse_deliveries WHERE idempotency_key = $1"))
        .bind(key)
        .fetch_optional(pool)
        .await?)
}

pub struct NewDelivery<'a> {
    pub key: &'a str,
    pub hash: &'a str,
    pub warehouse: Uuid,
    pub resource: Uuid,
    pub requested: i64,
    pub accepted: i64,
    pub deliverer_type: &'a str,
    pub deliverer_id: Uuid,
    pub metadata: &'a serde_json::Value,
}

/// Inserts a pending delivery; returns `None` when the key already exists.
pub async fn insert_delivery(pool: &PgPool, d: &NewDelivery<'_>) -> Result<Option<Delivery>> {
    Ok(sqlx::query_as(&format!(
        "INSERT INTO warehouse_deliveries (idempotency_key, request_hash, warehouse_id, resource_id, requested_quantity,
            accepted_quantity, deliverer_type, deliverer_id, metadata)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         ON CONFLICT (idempotency_key) DO NOTHING RETURNING {DELIVERY_COLS}"
    ))
    .bind(d.key)
    .bind(d.hash)
    .bind(d.warehouse)
    .bind(d.resource)
    .bind(d.requested)
    .bind(d.accepted)
    .bind(d.deliverer_type)
    .bind(d.deliverer_id)
    .bind(d.metadata)
    .fetch_optional(pool)
    .await?)
}

pub async fn finish_delivery(pool: &PgPool, id: Uuid, status: &str, accepted: i64, stock_after: Option<i64>, error: Option<&str>) -> Result<Delivery> {
    Ok(sqlx::query_as(&format!(
        "UPDATE warehouse_deliveries SET status = $2, accepted_quantity = $3, stock_after = $4, last_error = $5, updated_at = now()
         WHERE id = $1 RETURNING {DELIVERY_COLS}"
    ))
    .bind(id)
    .bind(status)
    .bind(accepted)
    .bind(stock_after)
    .bind(error)
    .fetch_one(pool)
    .await?)
}

/// Retries a failed delivery under the same key (status back to pending).
pub async fn reopen_delivery(pool: &PgPool, id: Uuid, accepted: i64) -> Result<()> {
    sqlx::query("UPDATE warehouse_deliveries SET status = 'pending', accepted_quantity = $2, last_error = NULL, updated_at = now() WHERE id = $1")
        .bind(id)
        .bind(accepted)
        .execute(pool)
        .await?;
    Ok(())
}

// ── Simulation state ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct SimulationState {
    pub paused: bool,
    #[serde(with = "time::serde::rfc3339::option")]
    pub last_tick_at: Option<OffsetDateTime>,
    pub tick_count: i64,
    pub last_tick_duration_ms: Option<i64>,
    pub last_error: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub last_political_refresh: Option<OffsetDateTime>,
}

pub async fn simulation_state(pool: &PgPool) -> Result<SimulationState> {
    Ok(sqlx::query_as(
        "SELECT paused, last_tick_at, tick_count, last_tick_duration_ms, last_error, last_political_refresh FROM simulation_state WHERE id = 1",
    )
    .fetch_one(pool)
    .await?)
}

pub async fn set_paused(pool: &PgPool, paused: bool) -> Result<SimulationState> {
    sqlx::query("UPDATE simulation_state SET paused = $1 WHERE id = 1").bind(paused).execute(pool).await?;
    simulation_state(pool).await
}

pub async fn record_tick(pool: &PgPool, at: OffsetDateTime, duration_ms: i64, error: Option<&str>) -> Result<()> {
    sqlx::query(
        "UPDATE simulation_state SET last_tick_at = $1, tick_count = tick_count + 1, last_tick_duration_ms = $2, last_error = $3 WHERE id = 1",
    )
    .bind(at)
    .bind(duration_ms)
    .bind(error)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn record_political_refresh(pool: &PgPool) -> Result<()> {
    sqlx::query("UPDATE simulation_state SET last_political_refresh = now() WHERE id = 1").execute(pool).await?;
    Ok(())
}
