# 0004. Warehouses are persistence POIs and inventory holders

- **Status:** Proposed
- **Date:** 2026-10-11
- **Deciders:** David Durieux
- **Scope:** universesimulator, inventory

## Context

A place where goods are stored must be a real point of interest of the game, i.e. a
persistence object the game server knows (`poi_village`, `cargo_depot`, `mining_depot`…).
A village can have several warehouses (a mining depot and a cargo depot at the other end of the
village). Inventory's goods holders were `player | npc | corporation | system`.

## Options considered

1. **Stock per political entity** (`political` holder). One shared stock per commune: goods
   dropped at one depot would appear at another, and no mission could move goods between
   two depots of the same village.
2. **Stock per warehouse** (`warehouse` holder, id = persistence object UUID), owned by a
   political entity.

## Decision

We will add the goods holder type `warehouse` to inventory. A warehouse's holder id is the UUID
of its persistence object. `universesimulator.warehouses` uses that UUID as primary key, stores
the owning political entity (commune or agglomeration) and caches the object's name, scene and
position. Creating a warehouse checks that the persistence object exists. The stock of a
village or a city is the sum of its warehouses (and of its child entities' warehouses).
Inventory also gets the hold `refType` `supply_order`, used to reserve goods for a supply leg.

## Consequences

Godot identifies warehouses by the persistence UUID it already knows. Warehouse holders are
reachable only through inventory's internal API. Cached POI data must be refreshed when the
object moves (admin refresh, import).
