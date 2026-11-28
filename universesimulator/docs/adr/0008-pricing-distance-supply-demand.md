# 0008. Pricing: supply/demand index, local urgency, distance freight

- **Status:** Proposed
- **Date:** 2026-10-11
- **Deciders:** David Durieux
- **Scope:** universesimulator

## Context

Mission rewards must depend on the distance travelled and on supply and demand. The admin
panel also needs buy and sell prices per place to show where goods are scarce.

## Options considered

1. **Fixed price list**. Easy, but no economy.
2. **Formula-based prices** recomputed each tick from stocks.

## Decision

We will compute, per federation and resource, on every tick:

- `supply` = sum of available stock in the federation's warehouses;
- `demand` = sum of target levels + consumption over 24 game hours;
- `unit_price = base_price × clamp((demand / max(supply, 1))^elasticity, min_factor, max_factor)`.

Per warehouse: `local_factor = clamp((target / max(available, 1))^elasticity, min, max)`,
`buy = unit_price × local_factor × (1 − spread/2)` and `sell = unit_price × local_factor × (1 + spread/2)`.

Rewards use the urgency of the destination,
`urgency = 1 + urgency_max × clamp(1 − projected / min_threshold, 0, 1)`:

- transport leg: `(leg_base_reward + quantity × mass_t × distance_km × freight_rate_per_tkm + quantity × unit_price × 0.5) × urgency`;
- mining: `quantity × unit_price × miner_share × urgency`.

The reward is fixed when a leg is published and stored on the leg. All parameters belong to
the federation. Prices are kept in `price_index`, `price_history` and `local_prices`.

## Consequences

Pricing is a pure module with unit tests. Local prices are informative for now; the market
and paid deliveries can use them later.

- **Amended 2026-11-28.** Hauling paid too little for the hours it takes (playtest: 2 677 credits to
  refine and deliver 120 kg within 3 h, 3 491 to deliver 240 kg, when an inspection 20 km away pays
  10 000). The tarsis_3 seed pays the trip ten times more: `legBaseReward` 200 → 2 000 and
  `freightRatePerTkm` 25 → 250, and the share of the cargo's value a carrier earns goes from 0.05 to
  0.5 (a constant of `pricing`): on a short haul the cargo is most of the reward, and the trip's rates
  alone gave ×2.6 where ×10 was asked. The resources' base prices are left as they were: what a
  warehouse buys and sells at does not move. The treasury follows
  (ADR 0009): `treasuryFloor` 200 000 → 2 000 000, above the dearest single mission (an operation
  mined, refined and carried over several hops is about a million), so a mission is published
  without first being refused; `mintBatch` 1 000 000 → 10 000 000, so one mint, hence one tick, still
  covers a burst of publications.
