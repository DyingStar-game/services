//! Clients of the services the simulator orchestrates (ADR 0003). Each client is a trait with
//! an HTTP implementation and the in-memory [`fake::FakeWorld`], used by `EXTERNAL_MODE=dry_run`
//! and by the tests.

pub mod fake;
pub mod http;
pub mod live;
pub mod token;

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::{Config, ExternalMode};
use crate::error::Result;

// ── Social ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PoliticalEntity {
    pub id: Uuid,
    #[serde(rename = "type")]
    pub entity_type: String,
    pub name: String,
    #[serde(default)]
    pub parent_id: Option<Uuid>,
}

#[async_trait]
pub trait SocialClient: Send + Sync {
    async fn get_entity(&self, id: Uuid) -> Result<Option<PoliticalEntity>>;
    async fn find_entity_by_name(&self, name: &str) -> Result<Option<PoliticalEntity>>;
    async fn create_entity(&self, entity_type: &str, name: &str, description: Option<&str>) -> Result<PoliticalEntity>;
    async fn set_parent(&self, id: Uuid, parent_id: Option<Uuid>) -> Result<()>;
    /// Ancestors from the direct parent up to the root.
    async fn ancestors(&self, id: Uuid) -> Result<Vec<PoliticalEntity>>;
}

// ── Inventory ────────────────────────────────────────────────────────────────

pub const WAREHOUSE: &str = "warehouse";

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct Holder {
    pub holder_type: &'static str,
    pub holder_id: Uuid,
}

impl Holder {
    pub fn warehouse(id: Uuid) -> Self {
        Self { holder_type: WAREHOUSE, holder_id: id }
    }

    pub fn of(kind: &str, id: Uuid) -> Option<Self> {
        let holder_type = match kind {
            "player" => "player",
            "npc" => "npc",
            WAREHOUSE => WAREHOUSE,
            _ => return None,
        };
        Some(Self { holder_type, holder_id: id })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StackView {
    pub good_type: String,
    pub quantity: i64,
    pub held: i64,
    pub available: i64,
}

/// A POI to create in `inventory` (`POST /api/internal/pois`): a point in the frame of the body
/// named by `scene`, owned by `owner_type` / `owner_id`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewPoi {
    pub owner_type: &'static str,
    pub owner_id: Uuid,
    pub name: String,
    pub scene: String,
    /// The persistence object it stands for, kept in its `metadata.persistence`: `inventory`
    /// takes a `parentId` for another POI only.
    pub persistence_id: Uuid,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub radius_m: f64,
}

#[async_trait]
pub trait InventoryClient: Send + Sync {
    async fn stacks(&self, holder: Holder) -> Result<Vec<StackView>>;
    async fn stack(&self, holder: Holder, good_type: &str) -> Result<Option<StackView>>;
    async fn credit(&self, holder: Holder, good_type: &str, quantity: i64) -> Result<()>;
    /// Fails with upstream 409 `INSUFFICIENT_GOODS` when the available quantity is too low.
    async fn debit(&self, holder: Holder, good_type: &str, quantity: i64) -> Result<()>;
    async fn transfer(&self, from: Holder, to: Holder, good_type: &str, quantity: i64) -> Result<()>;
    /// Reserves goods (`refType: supply_order`) and returns the hold id.
    async fn create_hold(&self, holder: Holder, good_type: &str, quantity: i64, ref_id: &str) -> Result<Uuid>;
    async fn consume_hold(&self, hold_id: Uuid, to: Holder) -> Result<()>;
    async fn release_hold(&self, hold_id: Uuid) -> Result<()>;
    /// Creates a POI; returns the id `inventory` gave it.
    async fn create_poi(&self, poi: &NewPoi) -> Result<Uuid>;
}

// ── Mission ──────────────────────────────────────────────────────────────────

/// Body of `POST /api/internal/missions` (mission service `createMissionBody`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewMission {
    pub title: String,
    pub description: String,
    pub kind: &'static str,
    pub category: &'static str,
    pub issuer_type: &'static str,
    pub issuer_id: Uuid,
    pub rewards: Vec<serde_json::Value>,
    pub escrow: Option<serde_json::Value>,
    pub script_id: String,
    pub objectives: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MissionSummary {
    pub id: Uuid,
    pub status: String,
    #[serde(default)]
    pub script_id: Option<String>,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub created_at: Option<time::OffsetDateTime>,
}

#[async_trait]
pub trait MissionClient: Send + Sync {
    async fn create(&self, mission: &NewMission) -> Result<MissionSummary>;
    /// Missions of an issuer with a status, newest first, down to `created_after` (all pages
    /// when `None`).
    async fn list(&self, issuer_type: &str, issuer_id: Uuid, status: &str, created_after: Option<time::OffsetDateTime>) -> Result<Vec<MissionSummary>>;
    async fn cancel(&self, id: Uuid) -> Result<()>;
}

/// Upstream error codes meaning the escrow payer cannot pay.
pub const INSUFFICIENT_FUNDS_CODES: &[&str] =
    &["ESCROW_INSUFFICIENT_FUNDS", "INSUFFICIENT_FUNDS", "INSUFFICIENT_BALANCE"];

// ── Economie ─────────────────────────────────────────────────────────────────

#[async_trait]
pub trait EconomieClient: Send + Sync {
    async fn ensure_political_wallet(&self, entity_id: Uuid) -> Result<()>;
    async fn political_balance(&self, entity_id: Uuid, currency: &str) -> Result<i64>;
    async fn set_minting(&self, entity_id: Uuid, allow: bool, ceiling: Option<i64>) -> Result<()>;
    async fn mint(&self, entity_id: Uuid, amount: i64, currency: &str, reason: &str) -> Result<()>;
}

// ── Persistence ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoiSnapshot {
    pub object_type: String,
    pub name: String,
    pub scenename: Option<String>,
    pub position: Option<[f64; 3]>,
}

/// A persistence object with its raw `object_data`.
#[derive(Debug, Clone)]
pub struct PoiItem {
    pub uuid: Uuid,
    pub object_type: String,
    pub data: serde_json::Value,
}

impl PoiItem {
    pub fn snapshot(&self) -> PoiSnapshot {
        let d = &self.data;
        let position = d["position"].as_object().and_then(|p| {
            Some([p.get("x")?.as_f64()?, p.get("y")?.as_f64()?, p.get("z")?.as_f64()?])
        });
        PoiSnapshot {
            name: d["name"].as_str().map(str::to_string).unwrap_or_else(|| self.object_type.clone()),
            scenename: d["scenename"].as_str().map(str::to_string),
            object_type: self.object_type.clone(),
            position,
        }
    }

    /// A UUID-valued field of `object_data` (e.g. `poi_uuid`, `parent_id`).
    pub fn uuid_field(&self, key: &str) -> Option<Uuid> {
        self.data[key].as_str().and_then(|s| Uuid::parse_str(s).ok())
    }
}

#[async_trait]
pub trait PersistenceClient: Send + Sync {
    async fn get_item(&self, uuid: Uuid) -> Result<Option<PoiSnapshot>>;
    /// Every object of a type under a parent (all pages).
    async fn list_items(&self, object_type: &str, parent_id: Uuid) -> Result<Vec<PoiItem>>;
}

// ── Bundle ───────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct Clients {
    pub social: Arc<dyn SocialClient>,
    pub inventory: Arc<dyn InventoryClient>,
    pub mission: Arc<dyn MissionClient>,
    pub economie: Arc<dyn EconomieClient>,
    pub persistence: Arc<dyn PersistenceClient>,
}

impl Clients {
    pub fn from_config(cfg: &Config) -> Self {
        match cfg.external_mode {
            ExternalMode::Live => live::build(cfg),
            ExternalMode::DryRun => Self::from_fake(Arc::new(fake::FakeWorld::permissive())),
        }
    }

    pub fn from_fake(world: Arc<fake::FakeWorld>) -> Self {
        Self {
            social: world.clone(),
            inventory: world.clone(),
            mission: world.clone(),
            economie: world.clone(),
            persistence: world,
        }
    }
}
