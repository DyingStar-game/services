//! Federations and resources.

use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::error::Result;
use crate::pricing;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Federation {
    pub id: Uuid,
    pub code: String,
    pub name: String,
    pub political_entity_id: Uuid,
    pub planet_internal_name: String,
    pub planet_poi_uuid: Option<Uuid>,
    pub currency: String,
    pub managed_categories: Vec<String>,
    pub treasury_floor: i64,
    pub mint_batch: i64,
    pub mint_ceiling: Option<i64>,
    pub elasticity: f64,
    pub price_min_factor: f64,
    pub price_max_factor: f64,
    pub spread: f64,
    pub freight_rate_per_tkm: f64,
    pub urgency_max: f64,
    pub miner_share: f64,
    pub leg_base_reward: i64,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

impl Federation {
    pub fn pricing(&self) -> pricing::Params {
        pricing::Params {
            elasticity: self.elasticity,
            min_factor: self.price_min_factor,
            max_factor: self.price_max_factor,
            spread: self.spread,
            freight_rate_per_tkm: self.freight_rate_per_tkm,
            urgency_max: self.urgency_max,
            miner_share: self.miner_share,
            leg_base_reward: self.leg_base_reward,
        }
    }
}

/// Create/update body. On update, every field is optional (COALESCE).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FederationInput {
    pub code: Option<String>,
    pub name: Option<String>,
    pub political_entity_id: Option<Uuid>,
    pub planet_internal_name: Option<String>,
    pub planet_poi_uuid: Option<Uuid>,
    pub currency: Option<String>,
    pub managed_categories: Option<Vec<String>>,
    pub treasury_floor: Option<i64>,
    pub mint_batch: Option<i64>,
    pub mint_ceiling: Option<i64>,
    pub elasticity: Option<f64>,
    pub price_min_factor: Option<f64>,
    pub price_max_factor: Option<f64>,
    pub spread: Option<f64>,
    pub freight_rate_per_tkm: Option<f64>,
    pub urgency_max: Option<f64>,
    pub miner_share: Option<f64>,
    pub leg_base_reward: Option<i64>,
}

pub async fn list_federations(pool: &PgPool) -> Result<Vec<Federation>> {
    Ok(sqlx::query_as("SELECT * FROM federations ORDER BY code").fetch_all(pool).await?)
}

pub async fn get_federation(pool: &PgPool, id: Uuid) -> Result<Option<Federation>> {
    Ok(sqlx::query_as("SELECT * FROM federations WHERE id = $1").bind(id).fetch_optional(pool).await?)
}

/// Inserts or updates a federation by `code` (all of code, name, politicalEntityId and
/// planetInternalName are required on insert).
pub async fn upsert_federation(pool: &PgPool, id: Option<Uuid>, f: &FederationInput) -> Result<Federation> {
    Ok(sqlx::query_as(
        r#"
        INSERT INTO federations (id, code, name, political_entity_id, planet_internal_name, currency,
            managed_categories, treasury_floor, mint_batch, mint_ceiling, elasticity, price_min_factor,
            price_max_factor, spread, freight_rate_per_tkm, urgency_max, miner_share, leg_base_reward, planet_poi_uuid)
        VALUES (COALESCE($1, gen_random_uuid()), $2, $3, $4, $5, COALESCE($6, 'credits'),
            COALESCE($7, '{ore,food,water}'), COALESCE($8, 100000), COALESCE($9, 1000000), $10,
            COALESCE($11, 0.8), COALESCE($12, 0.25), COALESCE($13, 4.0), COALESCE($14, 0.1),
            COALESCE($15, 2.0), COALESCE($16, 1.0), COALESCE($17, 0.6), COALESCE($18, 50), $19)
        ON CONFLICT (code) DO UPDATE SET
            name = COALESCE($3, federations.name),
            political_entity_id = COALESCE($4, federations.political_entity_id),
            planet_internal_name = COALESCE($5, federations.planet_internal_name),
            currency = COALESCE($6, federations.currency),
            managed_categories = COALESCE($7, federations.managed_categories),
            treasury_floor = COALESCE($8, federations.treasury_floor),
            mint_batch = COALESCE($9, federations.mint_batch),
            mint_ceiling = COALESCE($10, federations.mint_ceiling),
            elasticity = COALESCE($11, federations.elasticity),
            price_min_factor = COALESCE($12, federations.price_min_factor),
            price_max_factor = COALESCE($13, federations.price_max_factor),
            spread = COALESCE($14, federations.spread),
            freight_rate_per_tkm = COALESCE($15, federations.freight_rate_per_tkm),
            urgency_max = COALESCE($16, federations.urgency_max),
            miner_share = COALESCE($17, federations.miner_share),
            leg_base_reward = COALESCE($18, federations.leg_base_reward),
            planet_poi_uuid = COALESCE($19, federations.planet_poi_uuid),
            updated_at = now()
        RETURNING *
        "#,
    )
    .bind(id)
    .bind(&f.code)
    .bind(&f.name)
    .bind(f.political_entity_id)
    .bind(&f.planet_internal_name)
    .bind(&f.currency)
    .bind(&f.managed_categories)
    .bind(f.treasury_floor)
    .bind(f.mint_batch)
    .bind(f.mint_ceiling)
    .bind(f.elasticity)
    .bind(f.price_min_factor)
    .bind(f.price_max_factor)
    .bind(f.spread)
    .bind(f.freight_rate_per_tkm)
    .bind(f.urgency_max)
    .bind(f.miner_share)
    .bind(f.leg_base_reward)
    .bind(f.planet_poi_uuid)
    .fetch_one(pool)
    .await?)
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Resource {
    pub id: Uuid,
    pub code: String,
    pub name: String,
    pub category: String,
    pub unit: String,
    pub inventory_good_type: String,
    pub mass_per_unit_kg: f64,
    pub base_price: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceInput {
    pub code: String,
    pub name: String,
    pub category: String,
    #[serde(default = "default_unit")]
    pub unit: String,
    pub inventory_good_type: String,
    #[serde(default = "default_mass")]
    pub mass_per_unit_kg: f64,
    pub base_price: f64,
}

fn default_unit() -> String {
    "unit".into()
}

fn default_mass() -> f64 {
    1.0
}

pub async fn list_resources(pool: &PgPool) -> Result<Vec<Resource>> {
    Ok(sqlx::query_as("SELECT id, code, name, category, unit, inventory_good_type, mass_per_unit_kg, base_price FROM resources ORDER BY code")
        .fetch_all(pool)
        .await?)
}

pub async fn get_resource(pool: &PgPool, id: Uuid) -> Result<Option<Resource>> {
    Ok(sqlx::query_as("SELECT id, code, name, category, unit, inventory_good_type, mass_per_unit_kg, base_price FROM resources WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await?)
}

/// Finds a resource by code or inventory good type.
pub async fn find_resource(pool: &PgPool, code: Option<&str>, good_type: Option<&str>) -> Result<Option<Resource>> {
    Ok(sqlx::query_as(
        "SELECT id, code, name, category, unit, inventory_good_type, mass_per_unit_kg, base_price FROM resources
         WHERE ($1::TEXT IS NOT NULL AND code = $1) OR ($2::TEXT IS NOT NULL AND inventory_good_type = $2)",
    )
    .bind(code)
    .bind(good_type)
    .fetch_optional(pool)
    .await?)
}

pub async fn upsert_resource(pool: &PgPool, r: &ResourceInput) -> Result<Resource> {
    Ok(sqlx::query_as(
        r#"
        INSERT INTO resources (code, name, category, unit, inventory_good_type, mass_per_unit_kg, base_price)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        ON CONFLICT (code) DO UPDATE SET
            name = $2, category = $3, unit = $4, inventory_good_type = $5,
            mass_per_unit_kg = $6, base_price = $7, updated_at = now()
        RETURNING id, code, name, category, unit, inventory_good_type, mass_per_unit_kg, base_price
        "#,
    )
    .bind(&r.code)
    .bind(&r.name)
    .bind(&r.category)
    .bind(&r.unit)
    .bind(&r.inventory_good_type)
    .bind(r.mass_per_unit_kg)
    .bind(r.base_price)
    .fetch_one(pool)
    .await?)
}

pub async fn delete_resource(pool: &PgPool, id: Uuid) -> Result<bool> {
    Ok(sqlx::query("DELETE FROM resources WHERE id = $1").bind(id).execute(pool).await?.rows_affected() > 0)
}
