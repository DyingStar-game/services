//! Turns village shortages into supply orders: mining where the village has a mineral source,
//! otherwise transport from the nearest village with a surplus (ADR 0006, 0007, 0013).

use std::collections::{HashMap, HashSet};

use tracing::{info, warn};
use uuid::Uuid;

use super::stocks::Stocks;
use super::TickReport;
use crate::db::{catalog::{self, Federation}, orders::{self, NewLeg, NewOrder, SupplyOrder}, world};
use crate::error::{AppError, Result};
use crate::graph::{Graph, Path};
use crate::state::AppState;

/// Quantity to order for a stock, or `None` when it needs nothing.
pub fn need(available: i64, incoming: i64, min_threshold: i64, target_level: i64) -> Option<i64> {
    let projected = available + incoming;
    (projected < min_threshold).then(|| target_level - projected).filter(|q| *q > 0)
}

/// Surplus a village can give away (above its target level, or all of it without a policy).
pub fn surplus(available: i64, target_level: Option<i64>) -> i64 {
    (available - target_level.unwrap_or(0)).max(0)
}

/// Route graph between the active villages of a federation that have a warehouse.
pub async fn graph(st: &AppState, fed: &Federation) -> Result<Graph> {
    let nodes: HashSet<Uuid> = world::active_villages(&st.pool, fed.id)
        .await?
        .into_iter()
        .filter(|v| v.main_warehouse_id.is_some())
        .map(|v| v.id)
        .collect();
    Ok(Graph::new(&nodes, &world::route_edges(&st.pool, fed.id).await?))
}

pub async fn run(st: &AppState, fed: &Federation, stocks: &Stocks, report: &mut TickReport) -> Result<()> {
    let resources: HashMap<_, _> = catalog::list_resources(&st.pool).await?.into_iter().map(|r| (r.id, r)).collect();
    let policies = world::list_policies(&st.pool, None, Some(fed.id)).await?;
    let targets: HashMap<_, _> = policies.iter().map(|p| ((p.village_id, p.resource_id), p.target_level)).collect();
    let sources: HashMap<_, _> = world::list_mineral_sources(&st.pool, Some(fed.id), None)
        .await?
        .into_iter()
        .map(|s| ((s.village_id, s.resource_id), s))
        .collect();
    let mut incoming: HashMap<_, _> = orders::incoming(&st.pool, fed.id).await?.into_iter().map(|(v, r, q)| ((v, r), q)).collect();
    // Goods promised by legs not yet held are not available to new orders.
    let mut promised: HashMap<_, _> = orders::outgoing(&st.pool, fed.id).await?.into_iter().map(|(v, r, q)| ((v, r), q)).collect();
    let graph = graph(st, fed).await?;

    for p in &policies {
        let Some(res) = resources.get(&p.resource_id) else { continue };
        if !fed.managed_categories.contains(&res.category) || !graph.contains(p.village_id) {
            continue;
        }
        let key = (p.village_id, p.resource_id);
        let available = stocks.village(p.village_id, p.resource_id).available();
        let Some(mut qty) = need(available, incoming.get(&key).copied().unwrap_or(0), p.min_threshold, p.target_level) else {
            continue;
        };
        if orders::open_order_count(&st.pool, p.village_id, p.resource_id).await? > 0 {
            continue;
        }

        if let Some(src) = sources.get(&key) {
            qty = qty.min(src.max_units_per_mission);
            if let Some(left) = src.remaining_units {
                qty = qty.min(left);
            }
            if qty <= 0 {
                warn!(village = %p.village_id, resource = %res.code, "mineral source exhausted");
                continue;
            }
            let order = NewOrder {
                kind: "mining",
                federation_id: fed.id,
                resource_id: p.resource_id,
                quantity: qty,
                origin: None,
                destination: p.village_id,
                path: vec![p.village_id],
                trigger: "auto_mining",
                created_by: None,
                legs: vec![NewLeg { from: None, to: p.village_id, distance_m: 0.0 }],
            };
            orders::create_order(&st.pool, &order).await?;
            *incoming.entry(key).or_default() += qty;
            report.orders_created += 1;
            info!(village = %p.village_id, resource = %res.code, qty, "mining order");
            continue;
        }

        let surplus_of = |v: Uuid, promised: &HashMap<(Uuid, Uuid), i64>| {
            let avail = stocks.village(v, p.resource_id).available() - promised.get(&(v, p.resource_id)).copied().unwrap_or(0);
            surplus(avail, targets.get(&(v, p.resource_id)).copied())
        };
        // Closest village covering the whole need, else the closest with any surplus.
        let path = graph
            .nearest_source(p.village_id, |v| surplus_of(v, &promised) >= qty)
            .or_else(|| graph.nearest_source(p.village_id, |v| surplus_of(v, &promised) > 0));
        let Some(path) = path else {
            // Nobody has a surplus: mine it in the nearest village with a source, the ore
            // becomes that village's surplus and is shipped by a later tick.
            let mine = graph.nearest_source(p.village_id, |v| sources.contains_key(&(v, p.resource_id))).map(|path| path.hops[0].from);
            let Some(mine) = mine else {
                warn!(village = %p.village_id, resource = %res.code, qty, "no supplier reachable");
                continue;
            };
            let src = &sources[&(mine, p.resource_id)];
            let amount = src.remaining_units.map_or(src.max_units_per_mission, |left| left.min(src.max_units_per_mission)).min(qty);
            if amount > 0 && orders::open_order_count(&st.pool, mine, p.resource_id).await? == 0 {
                let order = NewOrder {
                    kind: "mining",
                    federation_id: fed.id,
                    resource_id: p.resource_id,
                    quantity: amount,
                    origin: None,
                    destination: mine,
                    path: vec![mine],
                    trigger: "auto_mining",
                    created_by: None,
                    legs: vec![NewLeg { from: None, to: mine, distance_m: 0.0 }],
                };
                orders::create_order(&st.pool, &order).await?;
                report.orders_created += 1;
                info!(village = %mine, for_village = %p.village_id, resource = %res.code, amount, "mining order for a remote shortage");
            }
            continue;
        };
        let origin = path.hops[0].from;
        qty = qty.min(surplus_of(origin, &promised));
        create_transport_order(st, fed, p.resource_id, qty, &path, "auto_shortage", None).await?;
        *incoming.entry(key).or_default() += qty;
        *promised.entry((origin, p.resource_id)).or_default() += qty;
        report.orders_created += 1;
        info!(from = %origin, to = %p.village_id, resource = %res.code, qty, hops = path.hops.len(), "transport order");
    }
    Ok(())
}

pub async fn create_transport_order(
    st: &AppState,
    fed: &Federation,
    resource: Uuid,
    quantity: i64,
    path: &Path,
    trigger: &'static str,
    created_by: Option<String>,
) -> Result<SupplyOrder> {
    let order = NewOrder {
        kind: "transport",
        federation_id: fed.id,
        resource_id: resource,
        quantity,
        origin: Some(path.hops[0].from),
        destination: path.hops.last().map(|h| h.to).unwrap_or(path.hops[0].to),
        path: path.nodes(),
        trigger,
        created_by,
        legs: path.hops.iter().map(|h| NewLeg { from: Some(h.from), to: h.to, distance_m: h.distance_m }).collect(),
    };
    orders::create_order(&st.pool, &order).await
}

/// Manual order (admin or internal API): from `origin` when given, else from the nearest
/// village with a surplus.
pub async fn manual_order(
    st: &AppState,
    resource: Uuid,
    quantity: i64,
    destination: Uuid,
    origin: Option<Uuid>,
    created_by: String,
) -> Result<SupplyOrder> {
    let dest = world::get_village(&st.pool, destination).await?.ok_or_else(|| AppError::NotFound(format!("village {destination}")))?;
    let fed = catalog::get_federation(&st.pool, dest.federation_id).await?.ok_or_else(|| AppError::NotFound("federation".into()))?;
    let graph = graph(st, &fed).await?;
    let path = match origin {
        Some(o) => graph.shortest_path(o, destination),
        None => {
            let levels = world::list_stock_levels(&st.pool, Some(fed.id), None).await?;
            let targets: HashMap<_, _> = world::list_policies(&st.pool, None, Some(fed.id))
                .await?
                .into_iter()
                .map(|p| ((p.village_id, p.resource_id), p.target_level))
                .collect();
            let surplus_of = |v: Uuid| {
                let avail: i64 = levels.iter().filter(|l| l.village_id == v && l.resource_id == resource).map(|l| l.available()).sum();
                surplus(avail, targets.get(&(v, resource)).copied())
            };
            graph.nearest_source(destination, |v| surplus_of(v) >= quantity)
        }
    }
    .ok_or_else(|| AppError::conflict("NO_ROUTE", "no route or no supplier for this order"))?;
    create_transport_order(st, &fed, resource, quantity, &path, "manual", Some(created_by)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn need_only_below_threshold() {
        assert_eq!(need(100, 0, 50, 200), None);
        assert_eq!(need(50, 0, 50, 200), None);
        assert_eq!(need(40, 0, 50, 200), Some(160));
        assert_eq!(need(40, 20, 50, 200), None);
        assert_eq!(need(0, 0, 0, 0), None);
    }

    #[test]
    fn surplus_above_target() {
        assert_eq!(surplus(500, Some(200)), 300);
        assert_eq!(surplus(100, Some(200)), 0);
        assert_eq!(surplus(100, None), 100);
    }
}
