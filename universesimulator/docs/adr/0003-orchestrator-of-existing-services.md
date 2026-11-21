# 0003. universesimulator orchestrates; data ownership split

- **Status:** Proposed
- **Date:** 2026-10-11
- **Deciders:** David Durieux
- **Scope:** universesimulator, social, inventory, persistence, mission, economie

## Context

The economy needs villages and cities, their stocks, the missions that move goods, and the
money that pays for them. Each of these already has an owner in the platform:

| Concern | Existing owner |
|---|---|
| Political hierarchy (commune → … → federation) | social (`political_entities`) |
| Goods and stocks | inventory (stacks, holds, transfers) |
| Places in the world | persistence (game objects such as `poi_village`, `cargo_depot`, `mining_depot`) |
| Missions, assignments, rewards | mission |
| Wallets, treasuries, minting | economie |

## Options considered

1. **Self-contained simulator** with its own locations, stocks and missions. Fast to build,
   but duplicates five domains and diverges from what players see in the game.
2. **Orchestrator**: the simulator keeps only what nobody else owns and drives the other
   services through their internal APIs.

## Decision

We will make `universesimulator` an orchestrator. It owns only: the warehouse registry and its
route graph, stock policies (thresholds, targets, capacities), production and consumption flows,
mineral sources, supply orders (meta-missions) and their legs, price indices, history and the
simulation tick. Quantities live in inventory, hierarchy in social, places in persistence,
missions in mission and money in economie. References to other services are opaque UUIDs
without foreign keys.

## Consequences

The simulator depends on five services and needs a service account with roles in each
(ADR 0011). Every outbound write must be idempotent or replayable, because a tick can fail
half way (ADR 0007, 0010). Small changes are needed in inventory (ADR 0004), social (ADR 0005)
and mission (ADR 0009).
