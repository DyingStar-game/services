# 0018. One job per mission: deliveries and refinings of their own

- **Status:** Proposed
- **Date:** 2026-11-11
- **Deciders:** WarpZone (playtest 2026-11-11)
- **Scope:** universesimulator (planner, publisher, seed)
- **Amends:** 0014 (the ore chain), 0007 (what a village gives away)

## Context

The ore chain gave two jobs: mine and crush, or one long mission that carries the ore, has it refined on
the way and delivers the metal (when nobody held the ore: mines it too). A player who only wants to drive,
or only to run a refinery, had nothing to take. And a mining village never gave its ore away: a village
only gives what is above its target level, which a mining village's stock, filled by missions up to that
target, never passes.

## Decision

- **A factory city stocks crushed ore.** The seed gives the factory profile a policy for `ore.gold`
  beside its ingots': short of ore, a city is supplied like any village short of anything, by a transport
  of the good as it is. That mission is a **delivery** (category `delivery`, "Deliver ... -> F0001").
- **A village refines what it holds.** A shortage of a refined resource in a village that refines and
  holds the input itself is met at home: a transport from its warehouse to its warehouse (ore taken off
  the shelves, metal shelved), published as a **refining** ("Refine ... at F0001", category `mining`, two
  hours). No new order kind: the mission service's `transport` kind already counts a load at the source
  and a drop of `deliverGoodType` at the destination, and they may be one warehouse.
- Without ore at home, the older answers stand: refining on the way from the nearest village with ore,
  else mining, refining and delivering in one mission.
- **A village keeps nothing of what it mines.** For a resource it has a source of, a village gives away
  all it holds; its policy only says how far ahead it mines. Any other village still keeps its target.
- **The same crates are not promised twice.** Nothing holds goods in `inventory` any more (0014): what a
  village owes to open transports (legs pending, held or published) is taken off what it can give, reckoned
  in the good loaded (the input of a refined resource).

## Consequences

- The chain reads mine -> deliver -> refine, each a mission of its own; the long missions remain when the
  short ones cannot be cut.
- A mining village emptied by a delivery falls under its threshold and orders mining again: the loop turns
  by itself.
- A leg counts against its origin until completed, so a little after its crates left: a village may look
  emptier than it is for the length of a trip, never fuller.
- An environment gets the cities' ore policy by re-importing the seed (policies are inserted when missing).
- A refining pays as a transport over no distance: the base reward and the share of the cargo's value.
