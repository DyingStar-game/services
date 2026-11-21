//! Reading warehouse stocks from inventory, aggregating them per village (ADR 0013) and
//! mirroring the political hierarchy.

use std::collections::HashMap;

use uuid::Uuid;

use crate::clients::Holder;
use crate::db::{catalog::{self, Federation}, world::{self, CachedEntity, Warehouse}};
use crate::error::Result;
use crate::state::AppState;

/// Observed stock of a (warehouse, resource), or a sum of them for a village.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Level {
    pub quantity: i64,
    pub held: i64,
}

impl Level {
    pub fn available(&self) -> i64 {
        self.quantity - self.held
    }
}

/// Stocks of a federation: per warehouse, and the village each warehouse belongs to.
#[derive(Debug, Default, Clone)]
pub struct Stocks {
    /// (warehouse, resource) → level.
    pub warehouses: HashMap<(Uuid, Uuid), Level>,
    /// warehouse → village.
    pub village_of: HashMap<Uuid, Uuid>,
}

impl Stocks {
    /// Sum over the warehouses of a village.
    pub fn village(&self, village: Uuid, resource: Uuid) -> Level {
        self.warehouses
            .iter()
            .filter(|((w, r), _)| *r == resource && self.village_of.get(w) == Some(&village))
            .fold(Level::default(), |acc, (_, l)| Level { quantity: acc.quantity + l.quantity, held: acc.held + l.held })
    }

    /// Warehouses of a village with their available quantity of a resource, most stocked first.
    pub fn by_availability(&self, village: Uuid, resource: Uuid) -> Vec<(Uuid, i64)> {
        let mut out: Vec<(Uuid, i64)> = self
            .village_of
            .iter()
            .filter(|(_, v)| **v == village)
            .map(|(w, _)| (*w, self.warehouses.get(&(*w, resource)).map(|l| l.available()).unwrap_or(0)))
            .collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        out
    }

    pub fn level_mut(&mut self, warehouse: Uuid, resource: Uuid) -> &mut Level {
        self.warehouses.entry((warehouse, resource)).or_default()
    }
}

/// Reads the stacks of every active warehouse of the federation and stores them in
/// `stock_levels`. Goods unknown to the resource catalog are ignored.
pub async fn refresh(st: &AppState, fed: &Federation) -> Result<Stocks> {
    let resources = catalog::list_resources(&st.pool).await?;
    let by_good: HashMap<&str, Uuid> = resources.iter().map(|r| (r.inventory_good_type.as_str(), r.id)).collect();
    let mut stocks = Stocks::default();
    for w in world::active_warehouses(&st.pool, fed.id).await? {
        stocks.village_of.insert(w.id, w.village_id);
        for s in st.clients.inventory.stacks(Holder::warehouse(w.id)).await? {
            if let Some(r) = by_good.get(s.good_type.as_str()) {
                stocks.warehouses.insert((w.id, *r), Level { quantity: s.quantity, held: s.held });
            }
        }
    }
    for ((w, r), l) in &stocks.warehouses {
        world::set_stock_level(&st.pool, *w, *r, l.quantity, l.held).await?;
    }
    Ok(stocks)
}

/// Reads one stock from inventory and stores it in `stock_levels`.
pub async fn refresh_one(st: &AppState, warehouse: Uuid, resource: Uuid, good_type: &str) -> Result<Level> {
    let level = st
        .clients
        .inventory
        .stack(Holder::warehouse(warehouse), good_type)
        .await?
        .map(|s| Level { quantity: s.quantity, held: s.held })
        .unwrap_or_default();
    world::set_stock_level(&st.pool, warehouse, resource, level.quantity, level.held).await?;
    Ok(level)
}

/// Warehouses of a village, the main one first.
pub async fn village_warehouses(st: &AppState, village: Uuid, main: Option<Uuid>) -> Result<Vec<Warehouse>> {
    let mut all: Vec<Warehouse> = world::list_warehouses(&st.pool, &world::WarehouseFilter { village: Some(village), ..Default::default() })
        .await?
        .into_iter()
        .filter(|w| w.active)
        .collect();
    all.sort_by_key(|w| (Some(w.id) != main, w.code.clone()));
    Ok(all)
}

/// Mirrors the villages' communes and their ancestors into `political_cache`.
pub async fn refresh_political_cache(st: &AppState, fed: &Federation) -> Result<()> {
    let mut owners: Vec<Uuid> = world::list_villages(&st.pool, &world::VillageFilter { federation: Some(fed.id), ..Default::default() })
        .await?
        .into_iter()
        .map(|v| v.political_entity_id)
        .collect();
    owners.push(fed.political_entity_id);
    owners.sort();
    owners.dedup();
    let mut seen = std::collections::HashSet::new();
    for id in owners {
        let Some(entity) = st.clients.social.get_entity(id).await? else { continue };
        let ancestors = st.clients.social.ancestors(id).await?;
        for e in std::iter::once(entity).chain(ancestors) {
            if seen.insert(e.id) {
                let cached = CachedEntity { id: e.id, r#type: e.entity_type, name: e.name, parent_id: e.parent_id, federation_id: Some(fed.id) };
                world::cache_entity(&st.pool, &cached).await?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregates_per_village() {
        let (v1, v2, w1, w2, w3, r) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(11), Uuid::from_u128(12), Uuid::from_u128(13), Uuid::from_u128(99));
        let mut s = Stocks::default();
        s.village_of.extend([(w1, v1), (w2, v1), (w3, v2)]);
        s.warehouses.insert((w1, r), Level { quantity: 100, held: 10 });
        s.warehouses.insert((w2, r), Level { quantity: 50, held: 0 });
        s.warehouses.insert((w3, r), Level { quantity: 7, held: 0 });
        assert_eq!(s.village(v1, r), Level { quantity: 150, held: 10 });
        assert_eq!(s.village(v1, r).available(), 140);
        assert_eq!(s.by_availability(v1, r), vec![(w1, 90), (w2, 50)]);
        assert_eq!(s.village(v2, Uuid::from_u128(5)), Level::default());
    }
}
