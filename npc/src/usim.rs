//! universesimulator internal API (as `svc-npc`, role `usim:read`): where a
//! village's storekeepers work. A warehouse id is the uuid of its cargo_depot
//! object on Horizon, so it is directly the NPC's `place_where_work.uuid`.

use std::{collections::HashMap, sync::Arc};

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
    #[serde(default)]
    pub village_id: Option<Uuid>,
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
        Ok(pick_main(village.main_warehouse_id, warehouses.items))
    }

    /// The main warehouse of every village universesimulator knows, keyed by
    /// upper-case village code. Two calls, whatever the number of villages.
    pub async fn main_warehouses(&self) -> Result<HashMap<String, Warehouse>> {
        let villages: Items<Village> = self.get("/villages").await?;
        let warehouses: Items<Warehouse> = self.get("/warehouses").await?;
        Ok(index_main_warehouses(villages.items, warehouses.items))
    }
}

/// The village's main warehouse if it is active, else its first active one.
fn pick_main(main: Option<Uuid>, warehouses: Vec<Warehouse>) -> Option<Warehouse> {
    let active: Vec<Warehouse> = warehouses.into_iter().filter(|w| w.active).collect();
    main.and_then(|m| active.iter().find(|w| w.id == m).cloned())
        .or_else(|| active.into_iter().next())
}

fn index_main_warehouses(villages: Vec<Village>, warehouses: Vec<Warehouse>) -> HashMap<String, Warehouse> {
    let mut by_village: HashMap<Uuid, Vec<Warehouse>> = HashMap::new();
    for w in warehouses {
        if let Some(v) = w.village_id {
            by_village.entry(v).or_default().push(w);
        }
    }
    villages
        .into_iter()
        .filter_map(|v| {
            let main = pick_main(v.main_warehouse_id, by_village.remove(&v.id)?)?;
            Some((v.code.to_uppercase(), main))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wh(id: u128, village: u128, active: bool) -> Warehouse {
        Warehouse {
            id: Uuid::from_u128(id),
            village_id: Some(Uuid::from_u128(village)),
            code: format!("W{id}"),
            name: format!("Depot {id}"),
            scenename: None,
            active,
        }
    }

    fn village(id: u128, code: &str, main: Option<u128>) -> Village {
        Village { id: Uuid::from_u128(id), code: code.into(), main_warehouse_id: main.map(Uuid::from_u128) }
    }

    #[test]
    fn main_warehouse_wins_when_active() {
        let got = pick_main(Some(Uuid::from_u128(2)), vec![wh(1, 9, true), wh(2, 9, true)]).unwrap();
        assert_eq!(got.id, Uuid::from_u128(2));
    }

    #[test]
    fn inactive_main_falls_back_to_first_active() {
        let got = pick_main(Some(Uuid::from_u128(2)), vec![wh(2, 9, false), wh(3, 9, true)]).unwrap();
        assert_eq!(got.id, Uuid::from_u128(3));
        assert!(pick_main(None, vec![wh(4, 9, false)]).is_none());
    }

    #[test]
    fn indexes_by_upper_case_code_and_skips_villages_without_depot() {
        let villages = vec![village(10, "m0042", None), village(11, "M0043", Some(5)), village(12, "M0044", None)];
        let warehouses = vec![wh(4, 10, true), wh(5, 11, true), wh(6, 11, true)];
        let index = index_main_warehouses(villages, warehouses);
        assert_eq!(index.len(), 2);
        assert_eq!(index["M0042"].id, Uuid::from_u128(4));
        assert_eq!(index["M0043"].id, Uuid::from_u128(5));
        assert!(!index.contains_key("M0044"));
    }

    #[test]
    fn reads_the_internal_api_shape() {
        let w: Warehouse = serde_json::from_value(serde_json::json!({
            "id": "00000000-0000-0000-0000-000000000001",
            "villageId": "00000000-0000-0000-0000-000000000009",
            "code": "W1", "name": "Depot", "scenename": "tarsis_3", "x": 1.0, "y": 2.0, "z": 3.0, "active": true
        }))
        .unwrap();
        assert_eq!(w.village_id, Some(Uuid::from_u128(9)));
    }
}
