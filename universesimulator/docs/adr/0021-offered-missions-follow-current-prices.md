# 0021. Offered missions follow the current prices

- **Status:** Proposed
- **Date:** 2026-11-28
- **Deciders:** David Durieux
- **Scope:** universesimulator, mission (read only)

## Context

A mission's reward is frozen when it is published: the mission service escrows it and its spec is
immutable. A mission nobody takes stays open for ever, so after a change of pricing players saw old
prices beside new ones: an inspection 30 km away at 2 804 credits (the formula of 2026-11-11) beside one
36 km away at 15 743 (`3 000 + 350/km`). Hauling rewards were raised on 2026-11-28, which left every open
hauling mission underpriced.

## Decision

- Each tick reads the federation's `available` missions once (taken ones are `active`: the mission
  service sets a mission back to `available` only when it has no active assignment left).
- **Inspections**: a mission whose credits differ from `reward(distance)` is cancelled. The reward
  moves only with its formula, so any difference counts. The cancelled mission frees its site
  (`close_ended`) and the site is published again, at the current price, in the same tick.
- **Legs**: a mission whose credits are more than `REPRICE_TOLERANCE` (10 %) away from what the leg
  would be published at now (the publisher's own `reward` and tier share) is cancelled; the leg goes
  back to `held` and is published again in the same tick. The price index and the urgency are read
  again every tick (an urgency can move the reward by some 10 % an hour), so a tighter tolerance would
  cancel the same mission nearly every tick.
- The cancellation is the mission service's: it refunds the escrow to the treasury, and the new
  publication escrows the new reward. The simulator moves no money for it.
- A repriced leg is a new attempt (a new `scriptId`) but not a failure: `supply_order_legs.reprices`
  (migration 0016) counts those attempts, and `SIM_LEG_MAX_ATTEMPTS` counts the others only.
- A mission taken is never repriced: its players took it at its price.

## Consequences

- A repriced mission gets a new id: a player looking at the list may see it vanish and come back.
- The cancel call is not conditional on the mission being untaken: a player accepting it between the
  read and the cancel loses it (a window of a few milliseconds per tick).
- Repairs (ADR 0020) pay a constant and are not repriced.
