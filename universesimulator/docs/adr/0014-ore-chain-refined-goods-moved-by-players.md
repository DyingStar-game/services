# 0014. The ore chain: refined goods, goods moved by the players, turn-in at a town hall

- **Status:** Proposed
- **Date:** 2026-11-06
- **Deciders:** WarpZone (with the team, Discord 2026-11-06)
- **Scope:** universesimulator, mission, inventory, game
- **Amends:** 0007 (holds consumed by the simulator)

## Context

The game now has an ore chain: a Crusher in the mining villages packs crates of crushed ore
(`ore.<mineral>`), a Refinery in the factory cities turns one into a crate of ingots
(`ingot.<mineral>`). Crates are physical: a player shelves one in a cargo depot (a warehouse) and
the game server moves the goods in the inventory and tells the mission service (goods moves). The
team decided missions are turned in at the town hall of the village they deliver to.

Under 0007 the simulator reserved the goods of a leg (inventory hold) and consumed the hold into the
destination when the mission completed: with players now moving the goods themselves, they would be
moved twice, and a hold makes the reserved goods impossible to load physically.

## Decision

- A resource may be **refined from** another (`resources.refined_from`): a factory's shortage of
  ingots is met with crushed ore. The planner looks for a surplus of the INPUT; found, it publishes a
  `transport` whose objective loads the input at the source warehouse and delivers the refined
  resource at the destination's (`goodType` / `deliverGoodType`). Not found, it publishes one
  **`mining_transport`** order: mine the input where a village has a source, deliver the refined
  resource at the destination (the mine's source is spent on completion).
- **No holds.** The goods are moved by the players and counted by the mission (`via: warehouse`); a
  completed leg moves nothing (a hold left by an older publication is still consumed).
- Every published mission is measured on the warehouses (`via: warehouse`) and **turned in at a town
  hall** (`autoComplete: false`).
- The seed of tarsis_3 carries the game's chain, gold only: `ore.gold` (mined), `ingot.gold`
  (refined from it, consumed by the factories), in litres.

## Consequences

- Goods promised to a leg are no longer locked: another player may take them meanwhile; the planner
  still counts them as promised (`orders::outgoing`), and the mission measures what really arrives.
- `supply_orders.kind` gains `mining_transport` (migration 0005).
