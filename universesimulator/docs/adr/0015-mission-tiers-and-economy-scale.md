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
- **Tiers** (`simulation::tiers`), reckoned in truckloads of the resource (see the amendment below; first a
  fixed 1 300 kg, a truck's rated payload): Solo up to one truckload (1 player), Team from 2 to 4 (up to 3
  players), Operation from 10 to 20 (up to 10 players). The planner cuts each order into pieces (a solo, a
  team, then operations, teams and solos for the rest; at most `MAX_PIECES`), one order and one mission each; the publisher sets the mission's seats (`maxAssignees`) and its
  deadline from its quantity. The players of a mission share its progress (it is measured on the
  warehouses).
- **Deadlines**, from the taking (`assignmentDeadlineHours`): the time of the work for solo and team alike
  (they share it -- ddurieux): 2 h to carry between warehouses, 3 h with a refining on the way, 4 h to
  mine and shelve, 6 h to mine, refine and shelve. Team and operation missions come one in two at a
  relaxed pace instead: a day (a week for an operation) for 70 % of the reward. The tier and the pace
  open the title ("Express team · …").
- The planner no longer waits for a village's open orders to end: what they bring is in `incoming`, so
  only the gap left is ordered (an operation over a week held a village back).

- **Amended 2026-11-10.** The cut gives every shortage a solo mission first, and a team one when what is
  left holds it, before the operations: there is work for every crew size. The targets of the seed are
  raised instead of the scale (a bigger buffer, the same consumption), so a fresh world has team missions.
  A village's profile says whether it `refines` (the factory cities): a mission that has ore refined on the
  way names the refining village nearest to its destination (`payload.refineAt`), and is titled "Refine and
  deliver" -- a mining village asked for ingots was told to refine at home, where there is no refinery.
  `GET /api/market` gives the players each resource's price, its price a day ago and the curve between
  (the town hall's market board).
  A truckload is what a truck's bed holds in the game, not its rated payload of 1 300 kg: a fixed number
  of crates (`TRUCK_CRATES` = 16 ore crates, one layer of 4 x 4 in the mini truck's bed),
  whatever they hold. Each resource says what a crate of it typically holds (`crateKg` in the seed:
  15 kg for crushed gold ore and for ingots alike), so a truckload is 16 × `crateKg`: a solo mission of
  gold ore is up to 240 kg, a team one 480 to 960 kg, an operation 2.4 to 4.8 t. (The game does let
  players pile more on top: the truckload sizes the missions, it is not a limit.)
  Amended 2026-11-11: a kilogram of crushed ore is a kilogram of the METAL it holds, not of the crate's
  weight. The game counts a crushed crate by its metal (the barren rock weighs and is worth nothing), so
  that purity pays -- in a sale, in a mission's count -- and that refining gives as many kilograms of
  ingots as went in. A typical crate (30 % pure) holds 15 kg of gold whether crushed or refined; it was
  22 kg crushed while the rock counted.

## Consequences

- The size of the world is one number; the tiers are tied to the truck and to the resource's crate, not
  to it.
- A shortage under three truckloads gives solo missions only (one solo, then a team needs two more):
  teams and operations come with big shortages (big villages, or a bigger scale).
- Every environment must re-import its seed after clearing its profiles and stocks (they are inserted
  only when missing), as for 0014.
- The limits and paces are constants of `simulation::tiers`; a resource without a `crateKg` counts 25 kg a
  crate, liquids and gases included until their own containers are known.
- An existing database keeps 25 kg a crate, no `mined` text and its old targets until its seed is imported
  again: resources and `refines` are updated by the import, the villages' targets only when missing.
