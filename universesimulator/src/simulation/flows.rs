//! Production and consumption flows of villages applied to inventory (ADR 0010, 0013):
//! production goes to the main warehouse, consumption is taken from the most stocked ones.

use std::collections::HashMap;

use tracing::warn;

use super::stocks::Stocks;
use super::TickReport;
use crate::clients::Holder;
use crate::db::{catalog::{self, Federation}, world};
use crate::error::Result;
use crate::state::AppState;

/// Splits a flow into whole units to apply now and the fraction carried to the next tick.
pub fn split(rate_per_hour: f64, hours: f64, carry: f64) -> (i64, f64) {
    let total = rate_per_hour * hours + carry;
    let whole = total.trunc();
    (whole as i64, total - whole)
}

pub async fn apply(st: &AppState, fed: &Federation, hours: f64, stocks: &mut Stocks, report: &mut TickReport) -> Result<()> {
    if hours <= 0.0 {
        return Ok(());
    }
    let goods: HashMap<_, _> = catalog::list_resources(&st.pool).await?.into_iter().map(|r| (r.id, r.inventory_good_type)).collect();
    let capacities: HashMap<_, _> = world::list_capacities(&st.pool, None, None, Some(fed.id))
        .await?
        .into_iter()
        .map(|c| ((c.warehouse_id, c.resource_id), c.capacity))
        .collect();
    let mains: HashMap<_, _> = world::active_villages(&st.pool, fed.id).await?.into_iter().map(|v| (v.id, v.main_warehouse_id)).collect();
    for flow in world::list_flows(&st.pool, None, Some(fed.id)).await? {
        let Some(good) = goods.get(&flow.resource_id) else { continue };
        let (whole, mut carry) = split(flow.rate_per_hour, hours, flow.carry);
        if whole > 0 {
            let Some(Some(main)) = mains.get(&flow.village_id) else {
                warn!(village = %flow.village_id, "production without a main warehouse");
                continue;
            };
            let level = stocks.level_mut(*main, flow.resource_id);
            let room = capacities.get(&(*main, flow.resource_id)).copied().flatten().map(|c| (c - level.quantity).max(0)).unwrap_or(i64::MAX);
            let qty = whole.min(room);
            if qty > 0 {
                st.clients.inventory.credit(Holder::warehouse(*main), good, qty).await?;
                level.quantity += qty;
                world::set_stock_level(&st.pool, *main, flow.resource_id, level.quantity, level.held).await?;
            }
        } else if whole < 0 {
            let mut wanted = -whole;
            for (w, available) in stocks.by_availability(flow.village_id, flow.resource_id) {
                if wanted == 0 || available <= 0 {
                    break;
                }
                let qty = wanted.min(available);
                st.clients.inventory.debit(Holder::warehouse(w), good, qty).await?;
                let level = stocks.level_mut(w, flow.resource_id);
                level.quantity -= qty;
                world::set_stock_level(&st.pool, w, flow.resource_id, level.quantity, level.held).await?;
                wanted -= qty;
            }
            if wanted > 0 {
                report.shortages += 1;
                carry = 0.0;
                warn!(village = %flow.village_id, resource = %good, missing = wanted, "shortage");
            }
        }
        let mut tx = st.pool.begin().await?;
        world::set_flow_carry(&mut tx, flow.id, carry).await?;
        tx.commit().await?;
        report.flows_applied += 1;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::split;

    #[test]
    fn carries_fractions() {
        let (w, c) = split(2.5, 1.0, 0.0);
        assert_eq!(w, 2);
        assert!((c - 0.5).abs() < 1e-9);
        let (w, c) = split(2.5, 1.0, c);
        assert_eq!(w, 3);
        assert!(c.abs() < 1e-9);
    }

    #[test]
    fn consumption_is_negative() {
        let (w, c) = split(-1.5, 1.0, 0.0);
        assert_eq!(w, -1);
        assert!((c + 0.5).abs() < 1e-9);
        let (w, _) = split(-1.5, 1.0, c);
        assert_eq!(w, -2);
    }

    #[test]
    fn small_rates_accumulate() {
        let mut carry = 0.0;
        let mut total = 0;
        for _ in 0..12 {
            let (w, c) = split(1.0, 1.0 / 12.0, carry);
            total += w;
            carry = c;
        }
        assert_eq!(total + carry.round() as i64, 1);
    }
}
