//! Prices and rewards (ADR 0008). Pure functions.

/// Pricing parameters of a federation.
#[derive(Debug, Clone, Copy)]
pub struct Params {
    pub elasticity: f64,
    pub min_factor: f64,
    pub max_factor: f64,
    pub spread: f64,
    pub freight_rate_per_tkm: f64,
    pub urgency_max: f64,
    pub miner_share: f64,
    pub leg_base_reward: i64,
}

/// Share of the cargo value added to a transport reward.
const CARGO_VALUE_SHARE: f64 = 0.05;
/// Demand horizon, in game hours of consumption.
pub const DEMAND_HORIZON_H: f64 = 24.0;

fn scarcity(p: &Params, need: f64, have: f64) -> f64 {
    let ratio = need.max(0.0) / have.max(1.0);
    ratio.powf(p.elasticity).clamp(p.min_factor, p.max_factor)
}

/// Federation unit price from total supply and demand.
pub fn unit_price(p: &Params, base_price: f64, supply: i64, demand: i64) -> f64 {
    base_price * scarcity(p, demand as f64, supply as f64)
}

/// Local factor of a warehouse (target level vs available stock).
pub fn local_factor(p: &Params, target_level: i64, available: i64) -> f64 {
    scarcity(p, target_level as f64, available as f64)
}

/// (buy, sell) prices of a warehouse.
pub fn local_prices(p: &Params, unit_price: f64, local_factor: f64) -> (f64, f64) {
    let mid = unit_price * local_factor;
    (mid * (1.0 - p.spread / 2.0), mid * (1.0 + p.spread / 2.0))
}

/// Urgency of a destination: 1 when at or above its threshold, up to `1 + urgency_max` when empty.
pub fn urgency(p: &Params, projected: i64, min_threshold: i64) -> f64 {
    if min_threshold <= 0 {
        return 1.0;
    }
    let gap = 1.0 - projected as f64 / min_threshold as f64;
    1.0 + p.urgency_max * gap.clamp(0.0, 1.0)
}

/// Reward of a transport leg, in currency minor units.
pub fn transport_reward(p: &Params, quantity: i64, mass_per_unit_kg: f64, distance_m: f64, unit_price: f64, urgency: f64) -> i64 {
    let tonnes = quantity as f64 * mass_per_unit_kg / 1000.0;
    let freight = tonnes * (distance_m / 1000.0) * p.freight_rate_per_tkm;
    let cargo = quantity as f64 * unit_price * CARGO_VALUE_SHARE;
    ((p.leg_base_reward as f64 + freight + cargo) * urgency).round().max(1.0) as i64
}

/// Reward of a mining mission, in currency minor units.
pub fn mining_reward(p: &Params, quantity: i64, unit_price: f64, urgency: f64) -> i64 {
    (quantity as f64 * unit_price * p.miner_share * urgency).round().max(1.0) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    const P: Params = Params {
        elasticity: 1.0,
        min_factor: 0.25,
        max_factor: 4.0,
        spread: 0.1,
        freight_rate_per_tkm: 2.0,
        urgency_max: 1.0,
        miner_share: 0.5,
        leg_base_reward: 50,
    };

    #[test]
    fn price_follows_supply_and_demand() {
        assert_eq!(unit_price(&P, 10.0, 100, 100), 10.0);
        assert_eq!(unit_price(&P, 10.0, 50, 100), 20.0);
        assert_eq!(unit_price(&P, 10.0, 200, 100), 5.0);
    }

    #[test]
    fn price_is_bounded() {
        assert_eq!(unit_price(&P, 10.0, 0, 1_000_000), 40.0);
        assert_eq!(unit_price(&P, 10.0, 1_000_000, 1), 2.5);
    }

    #[test]
    fn elasticity_dampens() {
        let p = Params { elasticity: 0.5, ..P };
        assert!((unit_price(&p, 10.0, 25, 100) - 20.0).abs() < 1e-9);
    }

    #[test]
    fn local_prices_have_spread() {
        let (buy, sell) = local_prices(&P, 10.0, 2.0);
        assert!((buy - 19.0).abs() < 1e-9);
        assert!((sell - 21.0).abs() < 1e-9);
        assert!(local_factor(&P, 100, 10) > local_factor(&P, 100, 100));
    }

    #[test]
    fn urgency_scales_with_shortage() {
        assert_eq!(urgency(&P, 100, 100), 1.0);
        assert_eq!(urgency(&P, 200, 100), 1.0);
        assert_eq!(urgency(&P, 50, 100), 1.5);
        assert_eq!(urgency(&P, 0, 100), 2.0);
        assert_eq!(urgency(&P, -10, 100), 2.0);
        assert_eq!(urgency(&P, 0, 0), 1.0);
    }

    #[test]
    fn transport_reward_grows_with_distance() {
        // 1000 units × 1 kg = 1 t over 10 km at 2/tkm = 20; cargo 1000×10×0.05 = 500; base 50.
        assert_eq!(transport_reward(&P, 1000, 1.0, 10_000.0, 10.0, 1.0), 570);
        assert!(transport_reward(&P, 1000, 1.0, 50_000.0, 10.0, 1.0) > 570);
        assert_eq!(transport_reward(&P, 1000, 1.0, 10_000.0, 10.0, 2.0), 1140);
    }

    #[test]
    fn mining_reward_uses_miner_share() {
        assert_eq!(mining_reward(&P, 100, 10.0, 1.0), 500);
        assert_eq!(mining_reward(&P, 0, 10.0, 1.0), 1);
    }
}
