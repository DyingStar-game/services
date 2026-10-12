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

- transport leg: `(leg_base_reward + quantity × mass_t × distance_km × freight_rate_per_tkm + quantity × unit_price × 0.05) × urgency`;
- mining: `quantity × unit_price × miner_share × urgency`.

The reward is fixed when a leg is published and stored on the leg. All parameters belong to
the federation. Prices are kept in `price_index`, `price_history` and `local_prices`.

## Consequences

Pricing is a pure module with unit tests. Local prices are informative for now; the market
and paid deliveries can use them later.
