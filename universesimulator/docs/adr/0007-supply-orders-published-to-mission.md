# 0007. Supply orders published to the mission service

- **Status:** Proposed
- **Date:** 2026-10-11
- **Deciders:** David Durieux
- **Scope:** universesimulator, mission, inventory

## Context

Missions are owned by the mission service. It has no notion of parent mission and sends no
callback when a mission ends.

## Options considered

1. **Own missions in the simulator**. Duplicates the mission service and its player UI.
2. **One scenario mission per order** (ordered objectives). Goods cannot stop at an
   intermediate warehouse, and one carrier must do every hop.
3. **Supply orders (meta-missions) in the simulator, one mission per leg in the mission service.**

## Decision

We will keep supply orders and their legs in the simulator and publish each leg as a
`dynamic` mission through `POST /api/internal/missions`, issued by the federation
(`issuerType: politics`). Legs are published one at a time: the next leg starts when goods have
reached the intermediate warehouse. The goods of the current leg are reserved by an inventory
hold (`refType: supply_order`, `refId: <leg id>`) on the origin warehouse; when the mission is
completed the hold is consumed into the destination warehouse. Mining orders publish a
`deliver_items` mission whose target holder is the mining warehouse, so the mission service
moves the ore from the miner to the warehouse itself.

Each leg is a small state machine (`pending → held → published → completed | failed`), and
each external call is idempotent: the mission `scriptId` is `usim:leg:<leg id>:<attempt>` (unique in the
mission service; a republication after a cancelled or expired mission is a new attempt), and a hold is created only once per leg. The simulator learns the outcome by
polling the missions issued by the federation on each tick.

## Consequences

Transport is accounted for at completion (the hold moves goods); physical pick-up by the
carrier is a later improvement. Cancelled or expired missions are retried up to
`SIM_LEG_MAX_ATTEMPTS`, then the order fails and the hold is released.
