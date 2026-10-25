//! universesimulator internal API (as `svc-npc`, role `usim:read`): where a
//! village's storekeepers work. A warehouse id is the uuid of its cargo_depot
//! object on Horizon, so it is directly the NPC's `place_where_work.uuid`.

use std::sync::Arc;

use serde::Deserialize;
use uuid::Uuid;

use crate::{
    error::{NpcError, Result},
    keycloak::ServiceToken,
};

pub struct Usim {
    http: reqwest::Client,
    base: String,
    token: Arc<ServiceToken>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Village {
    id: Uuid,
    code: String,
    main_warehouse_id: Option<Uuid>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Warehouse {
    pub id: Uuid,
    pub code: String,
    pub name: String,
    pub scenename: Option<String>,
    #[serde(default = "yes")]
    pub active: bool,
}

fn yes() -> bool {
    true
}

#[derive(Deserialize)]
struct Items<T> {
    items: Vec<T>,
}

impl Usim {
    /// `None` when `UNIVERSESIMULATOR_URL` or the service secret is unset.
    pub fn new(http: reqwest::Client, base: &str, token: Arc<ServiceToken>) -> Option<Self> {
        (!base.is_empty() && token.configured()).then(|| Self { http, base: base.to_string(), token })
    }

    async fn get<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T> {
        let bearer = self.token.bearer().await?;
        let res = self
            .http
            .get(format!("{}/api/internal{path}", self.base))
            .bearer_auth(bearer)
            .send()
            .await
            .map_err(|e| NpcError::upstream("universesimulator", 0, e.to_string()))?;
        if !res.status().is_success() {
            let status = res.status().as_u16();
            let body = res.text().await.unwrap_or_default();
            return Err(NpcError::upstream("universesimulator", status, body));
        }
        res.json().await.map_err(|e| NpcError::upstream("universesimulator", 0, e.to_string()))
    }

    /// The main warehouse of village `code` (e.g. `M0012`), else its first
    /// active one. `None` when the village is unknown or has no warehouse.
    pub async fn main_warehouse(&self, code: &str) -> Result<Option<Warehouse>> {
        let villages: Items<Village> = self.get("/villages").await?;
        let Some(village) = villages.items.into_iter().find(|v| v.code.eq_ignore_ascii_case(code)) else {
            return Ok(None);
        };
        let warehouses: Items<Warehouse> = self.get(&format!("/warehouses?village={}", village.id)).await?;
        let mut active = warehouses.items.into_iter().filter(|w| w.active);
        Ok(match village.main_warehouse_id {
            Some(main) => {
                let all: Vec<Warehouse> = active.collect();
                all.iter().find(|w| w.id == main).cloned().or_else(|| all.into_iter().next())
            }
            None => active.next(),
        })
    }
}
