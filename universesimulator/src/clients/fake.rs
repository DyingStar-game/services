//! In-memory stand-in for social, inventory, mission, economie and persistence.
//! Used by `EXTERNAL_MODE=dry_run` (local development) and by the tests.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use uuid::Uuid;

use super::*;
use crate::error::{AppError, Result};

#[derive(Debug, Clone)]
pub struct FakeHold {
    pub holder: Holder,
    pub good_type: String,
    pub quantity: i64,
    pub active: bool,
}

#[derive(Debug, Clone)]
pub struct FakeMission {
    pub mission: NewMission,
    pub status: String,
}

#[derive(Default)]
pub struct FakeState {
    pub entities: HashMap<Uuid, PoliticalEntity>,
    pub stacks: HashMap<(Holder, String), i64>,
    pub holds: HashMap<Uuid, FakeHold>,
    pub missions: HashMap<Uuid, FakeMission>,
    /// Political treasuries, in `credits`.
    pub wallets: HashMap<Uuid, i64>,
    pub minted: Vec<(Uuid, i64)>,
    pub items: HashMap<Uuid, PoiItem>,
}

pub struct FakeWorld {
    pub state: Mutex<FakeState>,
    /// Unknown persistence items are reported as existing POIs.
    permissive: bool,
}

impl FakeWorld {
    pub fn new() -> Self {
        Self { state: Mutex::new(FakeState::default()), permissive: false }
    }

    pub fn permissive() -> Self {
        Self { state: Mutex::new(FakeState::default()), permissive: true }
    }

    pub fn with<R>(&self, f: impl FnOnce(&mut FakeState) -> R) -> R {
        f(&mut self.state.lock().unwrap())
    }

    /// Adds a persistence object under `parent` (position and extra fields in `data`).
    pub fn add_item(&self, uuid: Uuid, object_type: &str, parent: Uuid, mut data: serde_json::Value) {
        data["parent_id"] = serde_json::json!(parent.to_string());
        self.with(|s| s.items.insert(uuid, PoiItem { uuid, object_type: object_type.into(), data }));
    }

    pub fn quantity(&self, holder: Holder, good_type: &str) -> i64 {
        self.with(|s| s.stacks.get(&(holder, good_type.to_string())).copied().unwrap_or(0))
    }

    pub fn set_quantity(&self, holder: Holder, good_type: &str, quantity: i64) {
        self.with(|s| s.stacks.insert((holder, good_type.to_string()), quantity));
    }

    pub fn set_mission_status(&self, id: Uuid, status: &str) {
        self.with(|s| {
            if let Some(m) = s.missions.get_mut(&id) {
                m.status = status.into();
            }
        });
    }

    /// Completes a mission the way the mission service would: for `deliver_items`
    /// objectives, goods are moved into the target holder.
    pub fn complete_mission(&self, id: Uuid) {
        self.with(|s| {
            let Some(m) = s.missions.get_mut(&id) else { return };
            m.status = "completed".into();
            for o in &m.mission.objectives {
                if o["type"] == "deliver_items" {
                    let to = Holder::warehouse(o["params"]["to"]["holderId"].as_str().unwrap().parse().unwrap());
                    let good = o["params"]["itemId"].as_str().unwrap().to_string();
                    let qty = o["targetQuantity"].as_i64().unwrap_or(0);
                    *s.stacks.entry((to, good)).or_default() += qty;
                }
            }
        });
    }

    fn held(s: &FakeState, holder: Holder, good_type: &str) -> i64 {
        s.holds
            .values()
            .filter(|h| h.active && h.holder == holder && h.good_type == good_type)
            .map(|h| h.quantity)
            .sum()
    }
}

impl Default for FakeWorld {
    fn default() -> Self {
        Self::new()
    }
}

fn err(service: &'static str, status: u16, code: &str) -> AppError {
    AppError::Upstream { service, status, code: code.into(), message: code.to_lowercase() }
}

#[async_trait]
impl SocialClient for FakeWorld {
    async fn get_entity(&self, id: Uuid) -> Result<Option<PoliticalEntity>> {
        Ok(self.with(|s| s.entities.get(&id).cloned()))
    }

    async fn find_entity_by_name(&self, name: &str) -> Result<Option<PoliticalEntity>> {
        Ok(self.with(|s| s.entities.values().find(|e| e.name == name).cloned()))
    }

    async fn create_entity(&self, entity_type: &str, name: &str, _description: Option<&str>) -> Result<PoliticalEntity> {
        self.with(|s| {
            if s.entities.values().any(|e| e.name == name) {
                return Err(err("social", 409, "NAME_TAKEN"));
            }
            let e = PoliticalEntity { id: Uuid::new_v4(), entity_type: entity_type.into(), name: name.into(), parent_id: None };
            s.entities.insert(e.id, e.clone());
            Ok(e)
        })
    }

    async fn set_parent(&self, id: Uuid, parent_id: Option<Uuid>) -> Result<()> {
        self.with(|s| {
            let e = s.entities.get_mut(&id).ok_or_else(|| err("social", 404, "NOT_FOUND"))?;
            e.parent_id = parent_id;
            Ok(())
        })
    }

    async fn ancestors(&self, id: Uuid) -> Result<Vec<PoliticalEntity>> {
        self.with(|s| {
            let mut out = Vec::new();
            let mut cur = s.entities.get(&id).ok_or_else(|| err("social", 404, "NOT_FOUND"))?.parent_id;
            while let Some(p) = cur {
                let Some(e) = s.entities.get(&p) else { break };
                if out.iter().any(|x: &PoliticalEntity| x.id == e.id) {
                    break;
                }
                out.push(e.clone());
                cur = e.parent_id;
            }
            Ok(out)
        })
    }
}

#[async_trait]
impl InventoryClient for FakeWorld {
    async fn stacks(&self, holder: Holder) -> Result<Vec<StackView>> {
        Ok(self.with(|s| {
            s.stacks
                .iter()
                .filter(|((h, _), _)| *h == holder)
                .map(|((_, g), q)| {
                    let held = Self::held(s, holder, g);
                    StackView { good_type: g.clone(), quantity: *q, held, available: q - held }
                })
                .collect()
        }))
    }

    async fn stack(&self, holder: Holder, good_type: &str) -> Result<Option<StackView>> {
        Ok(self.with(|s| {
            s.stacks.get(&(holder, good_type.to_string())).map(|q| {
                let held = Self::held(s, holder, good_type);
                StackView { good_type: good_type.into(), quantity: *q, held, available: q - held }
            })
        }))
    }

    async fn credit(&self, holder: Holder, good_type: &str, quantity: i64) -> Result<()> {
        self.with(|s| *s.stacks.entry((holder, good_type.to_string())).or_default() += quantity);
        Ok(())
    }

    async fn debit(&self, holder: Holder, good_type: &str, quantity: i64) -> Result<()> {
        self.with(|s| {
            let held = Self::held(s, holder, good_type);
            let q = s.stacks.entry((holder, good_type.to_string())).or_default();
            if *q - held < quantity {
                return Err(err("inventory", 409, "INSUFFICIENT_GOODS"));
            }
            *q -= quantity;
            Ok(())
        })
    }

    async fn transfer(&self, from: Holder, to: Holder, good_type: &str, quantity: i64) -> Result<()> {
        self.debit(from, good_type, quantity).await?;
        self.credit(to, good_type, quantity).await
    }

    async fn create_hold(&self, holder: Holder, good_type: &str, quantity: i64, _ref_id: &str) -> Result<Uuid> {
        self.with(|s| {
            let held = Self::held(s, holder, good_type);
            let q = s.stacks.get(&(holder, good_type.to_string())).copied().unwrap_or(0);
            if q - held < quantity {
                return Err(err("inventory", 409, "INSUFFICIENT_GOODS"));
            }
            let id = Uuid::new_v4();
            s.holds.insert(id, FakeHold { holder, good_type: good_type.into(), quantity, active: true });
            Ok(id)
        })
    }

    async fn consume_hold(&self, hold_id: Uuid, to: Holder) -> Result<()> {
        self.with(|s| {
            let h = s.holds.get_mut(&hold_id).ok_or_else(|| err("inventory", 404, "NOT_FOUND"))?;
            if !h.active {
                return Err(err("inventory", 409, "HOLD_NOT_ACTIVE"));
            }
            h.active = false;
            let h = h.clone();
            *s.stacks.entry((h.holder, h.good_type.clone())).or_default() -= h.quantity;
            *s.stacks.entry((to, h.good_type)).or_default() += h.quantity;
            Ok(())
        })
    }

    async fn release_hold(&self, hold_id: Uuid) -> Result<()> {
        self.with(|s| {
            let h = s.holds.get_mut(&hold_id).ok_or_else(|| err("inventory", 404, "NOT_FOUND"))?;
            h.active = false;
            Ok(())
        })
    }
}

#[async_trait]
impl MissionClient for FakeWorld {
    async fn create(&self, mission: &NewMission) -> Result<MissionSummary> {
        self.with(|s| {
            if s.missions.values().any(|m| m.mission.script_id == mission.script_id) {
                return Err(err("mission", 409, "DUPLICATE_SCRIPT_ID"));
            }
            if let Some(escrow) = &mission.escrow {
                let payer: Uuid = escrow["payerId"].as_str().unwrap().parse().unwrap();
                let amount = mission.rewards.iter().filter_map(|r| r["amount"].as_i64()).sum::<i64>();
                let balance = s.wallets.entry(payer).or_default();
                if *balance < amount {
                    return Err(err("mission", 409, "ESCROW_INSUFFICIENT_FUNDS"));
                }
                *balance -= amount;
            }
            let id = Uuid::new_v4();
            s.missions.insert(id, FakeMission { mission: mission.clone(), status: "available".into() });
            Ok(MissionSummary { id, status: "available".into(), script_id: Some(mission.script_id.clone()), created_at: None })
        })
    }

    async fn list(&self, issuer_type: &str, issuer_id: Uuid, status: &str, _created_after: Option<time::OffsetDateTime>) -> Result<Vec<MissionSummary>> {
        Ok(self.with(|s| {
            s.missions
                .iter()
                .filter(|(_, m)| m.mission.issuer_type == issuer_type && m.mission.issuer_id == issuer_id && m.status == status)
                .map(|(id, m)| MissionSummary { id: *id, status: m.status.clone(), script_id: Some(m.mission.script_id.clone()), created_at: None })
                .collect()
        }))
    }

    async fn cancel(&self, id: Uuid) -> Result<()> {
        self.with(|s| {
            let m = s.missions.get_mut(&id).ok_or_else(|| err("mission", 404, "NOT_FOUND"))?;
            m.status = "cancelled".into();
            Ok(())
        })
    }
}

#[async_trait]
impl EconomieClient for FakeWorld {
    async fn ensure_political_wallet(&self, entity_id: Uuid) -> Result<()> {
        self.with(|s| {
            s.wallets.entry(entity_id).or_default();
        });
        Ok(())
    }

    async fn political_balance(&self, entity_id: Uuid, _currency: &str) -> Result<i64> {
        Ok(self.with(|s| s.wallets.get(&entity_id).copied().unwrap_or(0)))
    }

    async fn set_minting(&self, _entity_id: Uuid, _allow: bool, _ceiling: Option<i64>) -> Result<()> {
        Ok(())
    }

    async fn mint(&self, entity_id: Uuid, amount: i64, _currency: &str, _reason: &str) -> Result<()> {
        self.with(|s| {
            *s.wallets.entry(entity_id).or_default() += amount;
            s.minted.push((entity_id, amount));
        });
        Ok(())
    }
}

#[async_trait]
impl PersistenceClient for FakeWorld {
    async fn get_item(&self, uuid: Uuid) -> Result<Option<PoiSnapshot>> {
        Ok(self.with(|s| s.items.get(&uuid).map(PoiItem::snapshot)).or_else(|| {
            self.permissive.then(|| PoiSnapshot {
                object_type: "cargo_depot".into(),
                name: format!("POI {}", &uuid.to_string()[..8]),
                scenename: None,
                position: None,
            })
        }))
    }

    async fn list_items(&self, object_type: &str, parent_id: Uuid) -> Result<Vec<PoiItem>> {
        Ok(self.with(|s| {
            let mut items: Vec<PoiItem> = s
                .items
                .values()
                .filter(|i| i.object_type == object_type && i.uuid_field("parent_id") == Some(parent_id))
                .cloned()
                .collect();
            items.sort_by_key(|i| i.uuid);
            items
        }))
    }
}
