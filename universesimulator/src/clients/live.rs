//! HTTP implementations of the service clients.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use super::http::{optional, Upstream};
use super::token::TokenProvider;
use super::*;
use crate::config::Config;
use crate::error::Result;

pub fn build(cfg: &Config) -> Clients {
    let http = reqwest::Client::builder()
        .timeout(Duration::from_millis(cfg.http_timeout_ms))
        .build()
        .expect("reqwest client");
    let token = Arc::new(TokenProvider::new(
        http.clone(),
        cfg.oidc_token_url.clone(),
        cfg.service_client_id.clone(),
        cfg.service_client_secret.clone(),
        cfg.outbound_internal_key.clone(),
    ));
    let api = |name, url: &str| Upstream::new(name, &format!("{}/api/internal", url.trim_end_matches('/')), http.clone(), Some(token.clone()));
    Clients {
        social: Arc::new(HttpSocial(api("social", &cfg.social_api_url))),
        inventory: Arc::new(HttpInventory(api("inventory", &cfg.inventory_api_url))),
        mission: Arc::new(HttpMission(api("mission", &cfg.mission_api_url))),
        economie: Arc::new(HttpEconomie(api("economie", &cfg.economie_api_url))),
        persistence: Arc::new(HttpPersistence(Upstream::new("persistence", &cfg.persistence_api_url, http, None))),
    }
}

fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[derive(Deserialize)]
struct Page<T> {
    items: Vec<T>,
}

// ── Social ───────────────────────────────────────────────────────────────────

struct HttpSocial(Upstream);

#[async_trait]
impl SocialClient for HttpSocial {
    async fn get_entity(&self, id: Uuid) -> Result<Option<PoliticalEntity>> {
        optional(self.0.get(&format!("/politics/{id}")).await)
    }

    async fn find_entity_by_name(&self, name: &str) -> Result<Option<PoliticalEntity>> {
        let page: Page<PoliticalEntity> = self.0.get(&format!("/politics?name={}&limit=1", enc(name))).await?;
        Ok(page.items.into_iter().next())
    }

    async fn create_entity(&self, entity_type: &str, name: &str, description: Option<&str>) -> Result<PoliticalEntity> {
        let mut body = json!({ "type": entity_type, "name": name });
        if let Some(d) = description {
            body["description"] = json!(d);
        }
        self.0.post("/politics", &body).await
    }

    async fn set_parent(&self, id: Uuid, parent_id: Option<Uuid>) -> Result<()> {
        let _: Value = self.0.put(&format!("/politics/{id}/parent"), &json!({ "parentId": parent_id })).await?;
        Ok(())
    }

    async fn ancestors(&self, id: Uuid) -> Result<Vec<PoliticalEntity>> {
        self.0.get(&format!("/politics/{id}/ancestors")).await
    }
}

// ── Inventory ────────────────────────────────────────────────────────────────

struct HttpInventory(Upstream);

#[derive(Deserialize)]
struct HolderInventory {
    stacks: Vec<StackView>,
}

#[derive(Deserialize)]
struct HoldCreated {
    id: Uuid,
}

#[async_trait]
impl InventoryClient for HttpInventory {
    async fn stacks(&self, h: Holder) -> Result<Vec<StackView>> {
        let inv: HolderInventory = self.0.get(&format!("/holders/{}/{}?limit=1", h.holder_type, h.holder_id)).await?;
        Ok(inv.stacks)
    }

    async fn stack(&self, h: Holder, good_type: &str) -> Result<Option<StackView>> {
        optional(self.0.get(&format!("/holders/{}/{}/stacks/{}", h.holder_type, h.holder_id, enc(good_type))).await)
    }

    async fn credit(&self, h: Holder, good_type: &str, quantity: i64) -> Result<()> {
        let path = format!("/holders/{}/{}/credit", h.holder_type, h.holder_id);
        let _: Value = self.0.post(&path, &json!({ "goodType": good_type, "quantity": quantity })).await?;
        Ok(())
    }

    async fn debit(&self, h: Holder, good_type: &str, quantity: i64) -> Result<()> {
        let path = format!("/holders/{}/{}/debit", h.holder_type, h.holder_id);
        let _: Value = self.0.post(&path, &json!({ "goodType": good_type, "quantity": quantity })).await?;
        Ok(())
    }

    async fn transfer(&self, from: Holder, to: Holder, good_type: &str, quantity: i64) -> Result<()> {
        let body = json!({ "from": from, "to": to, "goodType": good_type, "quantity": quantity });
        let _: Value = self.0.post("/transfers", &body).await?;
        Ok(())
    }

    async fn create_hold(&self, h: Holder, good_type: &str, quantity: i64, ref_id: &str) -> Result<Uuid> {
        let path = format!("/holders/{}/{}/holds", h.holder_type, h.holder_id);
        let body = json!({ "kind": "stack", "goodType": good_type, "quantity": quantity, "refType": "supply_order", "refId": ref_id });
        let hold: HoldCreated = self.0.post(&path, &body).await?;
        Ok(hold.id)
    }

    async fn consume_hold(&self, hold_id: Uuid, to: Holder) -> Result<()> {
        let _: Value = self.0.post(&format!("/holds/{hold_id}/consume"), &json!({ "to": to })).await?;
        Ok(())
    }

    async fn release_hold(&self, hold_id: Uuid) -> Result<()> {
        let _: Value = self.0.post(&format!("/holds/{hold_id}/release"), &json!({})).await?;
        Ok(())
    }
}

// ── Mission ──────────────────────────────────────────────────────────────────

struct HttpMission(Upstream);

#[derive(Deserialize)]
struct MissionCreated {
    mission: MissionSummary,
}

#[derive(Deserialize)]
struct MissionPage {
    missions: Vec<MissionSummary>,
    total: i64,
}

#[async_trait]
impl MissionClient for HttpMission {
    async fn create(&self, mission: &NewMission) -> Result<MissionSummary> {
        let created: MissionCreated = self.0.post("/missions", mission).await?;
        Ok(created.mission)
    }

    async fn list(&self, issuer_type: &str, issuer_id: Uuid, status: &str, created_after: Option<time::OffsetDateTime>) -> Result<Vec<MissionSummary>> {
        let mut out = Vec::new();
        let limit = 100;
        loop {
            let path = format!(
                "/missions?issuerType={issuer_type}&issuerId={issuer_id}&status={status}&limit={limit}&offset={}",
                out.len()
            );
            let page: MissionPage = self.0.get(&path).await?;
            let n = page.missions.len();
            // Pages are ordered by creation date, newest first.
            let older = created_after.is_some_and(|after| page.missions.last().and_then(|m| m.created_at).is_some_and(|c| c < after));
            out.extend(page.missions);
            if n < limit || older || out.len() as i64 >= page.total {
                return Ok(out);
            }
        }
    }

    async fn cancel(&self, id: Uuid) -> Result<()> {
        let _: Value = self.0.post(&format!("/missions/{id}/cancel"), &json!({})).await?;
        Ok(())
    }
}

// ── Economie ─────────────────────────────────────────────────────────────────

struct HttpEconomie(Upstream);

#[derive(Deserialize)]
struct Accounts {
    accounts: Vec<Account>,
}

#[derive(Deserialize)]
struct Account {
    currency: String,
    balance: i64,
}

#[async_trait]
impl EconomieClient for HttpEconomie {
    async fn ensure_political_wallet(&self, entity_id: Uuid) -> Result<()> {
        let _: Value = self.0.put(&format!("/politics/{entity_id}/wallet"), &json!({})).await?;
        Ok(())
    }

    async fn political_balance(&self, entity_id: Uuid, currency: &str) -> Result<i64> {
        let acc: Accounts = self.0.get(&format!("/politics/{entity_id}/wallet")).await?;
        Ok(acc.accounts.into_iter().find(|a| a.currency == currency).map(|a| a.balance).unwrap_or(0))
    }

    async fn set_minting(&self, entity_id: Uuid, allow: bool, ceiling: Option<i64>) -> Result<()> {
        let mut body = json!({ "allowMinting": allow });
        if let Some(c) = ceiling {
            body["mintCeiling"] = json!(c);
        }
        let _: Value = self.0.put(&format!("/politics/{entity_id}/settings"), &body).await?;
        Ok(())
    }

    async fn mint(&self, entity_id: Uuid, amount: i64, currency: &str, reason: &str) -> Result<()> {
        let body = json!({ "amount": amount, "currency": currency, "reason": reason });
        let _: Value = self.0.post(&format!("/politics/{entity_id}/mint"), &body).await?;
        Ok(())
    }
}

// ── Persistence ──────────────────────────────────────────────────────────────

struct HttpPersistence(Upstream);

#[derive(Deserialize)]
struct Item {
    object_type: String,
    #[serde(default)]
    object_uuid: Option<Uuid>,
    #[serde(default)]
    object_data: Value,
}

#[derive(Deserialize)]
struct ItemPage {
    items: Vec<Item>,
    total: Option<i64>,
}

#[async_trait]
impl PersistenceClient for HttpPersistence {
    async fn get_item(&self, uuid: Uuid) -> Result<Option<PoiSnapshot>> {
        let Some(item) = optional(self.0.get::<Item>(&format!("/items/{uuid}")).await)? else {
            return Ok(None);
        };
        Ok(Some(PoiItem { uuid, object_type: item.object_type, data: item.object_data }.snapshot()))
    }

    async fn list_items(&self, object_type: &str, parent_id: Uuid) -> Result<Vec<PoiItem>> {
        let mut out = Vec::new();
        let page_size = 200;
        for page in 1.. {
            let path = format!("/items?object_type={}&parent_id={parent_id}&page={page}&page_size={page_size}", enc(object_type));
            let res: ItemPage = self.0.get(&path).await?;
            let n = res.items.len();
            out.extend(res.items.into_iter().filter_map(|i| {
                Some(PoiItem { uuid: i.object_uuid?, object_type: i.object_type, data: i.object_data })
            }));
            if n < page_size || res.total.is_some_and(|t| out.len() as i64 >= t) {
                break;
            }
        }
        Ok(out)
    }
}
