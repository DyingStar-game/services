# 0013. Villages are the simulation nodes; they aggregate their warehouses

- **Status:** Proposed
- **Date:** 2026-10-12
- **Deciders:** David Durieux
- **Scope:** universesimulator, persistence objects, adminpanel
- **Amends:** 0004 (owner of a warehouse), 0005 (communes), 0006 (graph nodes)

## Context

In the game, a village (`poi_village` objects named `Mxxxx` for mining villages and `Fxxxx`
for factory towns, with `population`, `type` and `radius_m`) owns one or more `cargo_depot`
objects, which hold the goods on their storage shelves. `mining_depot` objects hold no stock.
Persistence has no field linking a cargo_depot to its village: its `parent_id` is the planet.
Players reason in villages: "M0002 lacks water", not "this depot lacks water".

## Options considered

1. **Warehouses as nodes** (ADR 0004/0006 as first written). Thresholds and routes per depot;
   a village is only a view. Diverges from how the game and the players see the world.
2. **Villages as nodes, warehouses as storage.** The stock of a village is the sum of its
   cargo_depots.

For the depot → village link: proximity only (fragile when villages are close: one depot of
SandBox is 2.7 km from its village, beyond its 2 km radius), an explicit field only, or both.

## Decision

We will make **villages** the nodes of the simulation:

- A village is a persistence `poi_village` (`villages.id` = its UUID) and a commune in social,
  created under the federation by the import.
- A **warehouse** is a persistence `cargo_depot` attached to its village by the field
  **`poi_uuid`** of its object_data (the UUID of the poi_village). A depot without a valid
  `poi_uuid` is not simulated (the import reports it). The game, or the admin panel, sets it.
  `mining_depot` objects are ignored.
- The **stock of a village** is the sum of its warehouses' stocks in inventory (holder type
  `warehouse`, id = cargo_depot UUID, ADR 0004).
- **Thresholds, target levels, flows, mineral sources, routes and local prices are per village.**
  **Capacities and initial stocks are per warehouse.**
- Each village has a **main warehouse** (the first attached, changeable by an admin): deliveries,
  production and mined ore go there; consumption and outgoing transport take from the
  warehouses holding the most.
- A transport leg goes from village to village: its goods are held in a warehouse of the origin
  village (the one where the previous leg delivered them, or the most stocked for the first leg)
  and delivered to the main warehouse of the next village.
- The **import** reads the planet's villages and depots from persistence; every village with at
  least one depot is simulated. Policies, flows, capacities and mineral sources come from
  profiles (defaults, per village `type`, per village name), scaled by population when asked;
  defaults only fill what is missing, so a re-import keeps admin changes. Routes link each
  village to its nearest neighbours (configurable, within `maxDistanceM`, but always at least
  the nearest one so that no village is isolated), plus extra routes. These automatic routes
  are recomputed at every import and tick replay: one no longer wanted (e.g. a long link
  replaced by a village that appeared in between) is removed. Routes created by an admin are
  never removed by the import.
- The import keeps the world file (`world_files`); the tick replays it every
  `SIM_POLITICAL_REFRESH_MINUTES` and on every forced tick, so a village newly linked to a
  depot, or a new depot, is simulated without a manual import.
- A village's profile is chosen by its persistence `type` when the world file has a profile for
  it, else by its name prefix (`typeByNamePrefix`, e.g. `M` → mining, `F` → factory): some
  villages carry the generic type `poi_village`.
- When no village has a surplus of a resource, the planner orders mining in the nearest
  village holding a source of it; the ore becomes that village's surplus and is shipped later.

## Consequences

- The game must write `poi_uuid` on each cargo_depot (or an admin sets it). Until then, the
  depot and its village are not simulated.
- The admin panel lists villages, shows their aggregated stocks and, per village, its
  warehouses; thresholds and flows are edited per village, capacities per warehouse.
- Not yet released: the initial migration was rewritten instead of adding a new one.
