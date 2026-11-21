# 0012. Off-mission warehouse deliveries from the game server

- **Status:** Proposed
- **Date:** 2026-10-11
- **Deciders:** David Durieux
- **Scope:** universesimulator, game server

## Context

A player or an NPC may bring ore or food to a warehouse without a mission. That delivery must
be accounted for in the warehouse stock. The game server (Godot, `svc-game`) is the only party
that knows the drop happened.

## Options considered

1. **Godot calls inventory directly**. The simulator would not know about deliveries, capacity
   would not be enforced, and retries could double transfer (inventory has no idempotency key).
2. **Godot calls the simulator**, which checks capacity and moves the goods once.

## Decision

We will expose `POST /api/internal/warehouse-deliveries` (role `usim:deliver`). The body carries
an `idempotencyKey`, the warehouse (id or persistence UUID), the resource (code or inventory
good type), the quantity and the deliverer. The simulator records the delivery, clamps the
quantity to the remaining capacity, transfers the accepted quantity from the deliverer to the
warehouse in inventory and answers `201 {acceptedQuantity, rejectedQuantity, stock}`. A replay
with the same key returns `200` with the same answer; the same key with another body returns
`409`. There is no payment.

## Consequences

Godot returns the rejected quantity to the player. Paid deliveries (using local buy prices,
ADR 0008) and withdrawals or purchases are deferred to a later ADR.
