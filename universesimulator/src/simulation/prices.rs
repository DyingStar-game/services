//! Federation price indices and local buy/sell prices per village (ADR 0008).

use std::collections::HashMap;

use super::stocks::Stocks;
use crate::db::{catalog::{self, Federation}, history, world};
use crate::error::Result;
use crate::pricing;
use crate::state::AppState;

pub async fn run(st: &AppState, fed: &Federation, stocks: &Stocks) -> Result<()> {
    let params = fed.pricing();
    let policies = world::list_policies(&st.pool, None, Some(fed.id)).await?;
    let flows = world::list_flows(&st.pool, None, Some(fed.id)).await?;
    let mut prices = HashMap::new();
    for r in catalog::list_resources(&st.pool).await? {
        if !fed.managed_categories.contains(&r.category) {
            continue;
        }
        let supply: i64 = stocks.warehouses.iter().filter(|((_, res), _)| *res == r.id).map(|(_, l)| l.available().max(0)).sum();
        let targets: i64 = policies.iter().filter(|p| p.resource_id == r.id).map(|p| p.target_level).sum();
        let consumption: f64 = flows.iter().filter(|f| f.resource_id == r.id && f.rate_per_hour < 0.0).map(|f| -f.rate_per_hour).sum();
        let demand = targets + (consumption * pricing::DEMAND_HORIZON_H).round() as i64;
        let price = pricing::unit_price(&params, r.base_price, supply, demand);
        history::set_price_index(&st.pool, fed.id, r.id, supply, demand, price).await?;
        prices.insert(r.id, price);
    }
    for p in &policies {
        let Some(price) = prices.get(&p.resource_id) else { continue };
        let factor = pricing::local_factor(&params, p.target_level, stocks.village(p.village_id, p.resource_id).available());
        let (buy, sell) = pricing::local_prices(&params, *price, factor);
        history::set_local_price(&st.pool, p.village_id, p.resource_id, factor, buy, sell).await?;
    }
    Ok(())
}
