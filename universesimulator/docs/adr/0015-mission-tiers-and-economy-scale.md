# 0015. Mission tiers, and one scale for the whole economy

- **Status:** Proposed
- **Date:** 2026-11-09
- **Deciders:** WarpZone (with ddurieux, 2026-11-09)
- **Scope:** universesimulator, mission

## Context

A supply mission asked for one or two crates (32 kg, 60 kg): a truck carries 1 300 kg, and there was no
work for a team, nor for a big group over several days. The villages' profiles were tuned per thousand
people with small numbers (a factory's threshold of 40 kg, 4 kg an hour), and changing the size of the
world meant editing every one of them.

## Options considered

1. **Bigger numbers in the seed** — every threshold, flow, capacity and source multiplied by hand. Simple,
   but the next change of scale is the same chore, and nothing tells a mission's size.
2. **One scale, and missions cut into tiers** — the seed keeps base values and a single `economyScale`;
   a shortage is cut into missions of a few sizes, each with its seats and deadline.

## Decision

We will take option 2.

- **`economyScale`** (seed root, default 1; 25 for tarsis_3) multiplies, at import, every quantity of the
  villages' profiles: thresholds and targets, flows, capacities, initial stocks, mineral sources and their
  cap per mission.
- **Tiers** (`simulation::tiers`), reckoned in weight, in truckloads (`TRUCK_KG` = 1 300, a truck's payload):
  Solo up to one truckload (1 player), Team from 2 to 4 (up to 3 players), Operation from 10 to 20 (up to
  10 players). The planner cuts each order into pieces (operations first, then teams, then solo for the
  rest), one order and one mission each; the publisher sets the mission's seats (`maxAssignees`) and its
  deadline from its quantity. The players of a mission share its progress (it is measured on the
  warehouses).
- **Deadlines**, from the taking (`assignmentDeadlineHours`): the time of the work for solo and team alike
  (they share it -- ddurieux): 2 h to carry between warehouses, 3 h with a refining on the way, 4 h to
  mine and shelve, 6 h to mine, refine and shelve. Team and operation missions come one in two at a
  relaxed pace instead: a day (a week for an operation) for 70 % of the reward. The tier and the pace
  open the title ("Express team · …").
- The planner no longer waits for a village's open orders to end: what they bring is in `incoming`, so
  only the gap left is ordered (an operation over a week held a village back).

## Consequences

- The size of the world is one number; the tiers stay in absolute weight, tied to the truck, not to it.
- A shortage under two truckloads gives solo missions only: teams and operations come with big shortages
  (big villages, or a bigger scale).
- Every environment must re-import its seed after clearing its profiles and stocks (they are inserted
  only when missing), as for 0014.
- The limits and paces are constants of `simulation::tiers`; liquids and gases use the same numbers until
  their trucks are known.
