# 0019. Inspections: a mission that moves no goods

- **Status:** Proposed
- **Date:** 2026-11-11
- **Deciders:** WarpZone (playtest 2026-11-11)
- **Scope:** universesimulator, mission (`visit`), inventory (POIs), game (star chart)

## Context

Every mission came from a shortage: mine, carry, refine. A player without a truck or a tool had nothing to
do, and the corporation's other subsidiaries gave no work. The first job that moves no goods: the
corporation's security has a place looked at, 5 to 40 km from a village.

## Decision

- **Sites.** A village with a position gets `SITES_PER_VILLAGE` (4) inspection sites, made once: a quarter
  turn apart around it, each between 5 and 40 km away, both drawn from the village's id (the same village
  always gets the same sites). A site is a point at the village's distance from the planet's centre and a
  POI of 1 500 m in `inventory`, wide enough for the relief. Table `inspection_sites` (migration 0011).
- **One mission a village.** Each tick, a village without an open inspection gets one on its site
  inspected the longest ago. A mission that ends frees its site; cancelled or expired, the site keeps its
  turn.
- **The mission** is the mission service's `visit` on the site's POI: being there is enough ("just be on
  the spot", WarpZone). Category `exploration`, giver `{faction, division: security}`, one player, three
  hours, `autoComplete`: paid on arrival, nothing to turn in. It is offered in its village. Its reward is
  the base reward of a leg, once more per 10 km.
- **Its place travels with it.** The objective's `locationTo` carries the site's name, planet (`scene`),
  position and radius: the game draws the zone on its star chart for the player who took the mission.
- Nothing here reads a stock nor makes an order: the step is beside the supply chain, and shares only the
  treasury (an escrow it cannot pay mints, as for a leg).

## Consequences

- Around forty more places in `inventory` per ten villages, made once; none per mission.
- The altitude of a site is its village's: a relief of more than 1 500 m between them would put the zone
  out of reach. Raise `SITE_RADIUS_M`, or ask the game for the ground's height, if a planet needs it.
- A site may fall in a lake or on a cliff: the simulator knows no terrain. The zone is wide; a site that
  proves unreachable is to be moved by hand (its row), not by code.
- How many sites, how far and how often are constants: data of the entity when a second one needs others.
