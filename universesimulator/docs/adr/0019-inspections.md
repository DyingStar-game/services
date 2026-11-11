# 0019. Inspections: a mission that moves no goods

- **Status:** Proposed
- **Date:** 2026-11-11
- **Deciders:** WarpZone (playtest 2026-11-11)
- **Scope:** universesimulator, mission (`visit`), inventory (POIs), game (server: the ground; client: star chart)

## Context

Every mission came from a shortage: mine, carry, refine. A player without a truck or a tool had nothing to
do, and the corporation's other subsidiaries gave no work. The first job that moves no goods: the
corporation's security has a place looked at, 5 to 40 km from a village.

## Decision

- **Sites.** A village with a position gets `SITES_PER_VILLAGE` (4) inspection sites, made once: a quarter
  turn apart around it, each between 5 and 40 km away, both drawn from the village's id (the same village
  always gets the same sites). Table `inspection_sites` (migration 0011).
- **The game server lays them on the ground.** The simulator knows no relief (the villages alone stand
  over 3.4 km of it): it makes a site at its village's distance from the planet's centre, and gives it no
  mission. The game server, which has the terrain, asks now and then for the sites not laid
  (`GET /api/internal/inspection-sites/ungrounded`) and answers where the ground is under each
  (`PUT /api/internal/inspection-sites/{id}/ground`, role `usim:deliver`), once per site (`grounded_at`,
  migration 0012). Only then does the site get its POI in `inventory`, 500 m around that point, and its
  missions. A site that had a POI is moved with it; a mission open on a site not laid is cancelled.
- **One mission a village.** Each tick, a village without an open inspection gets one on its site
  inspected the longest ago. A mission that ends frees its site; cancelled or expired, the site keeps its
  turn.
- **The mission** is the mission service's `visit` on the site's POI: being there is enough ("just be on
  the spot", WarpZone). Category `exploration`, giver `{faction, division: security}`, one player, three
  hours, `autoComplete`: paid on arrival, nothing to turn in. It is offered in its village. Its reward is
  2 000 credits, and the base reward of a leg once more per 10 km: the drive is the player's time.
- **Its place travels with it.** The objective's `locationTo` carries the site's name, planet (`scene`),
  position and radius: the game draws the zone on its star chart for the player who took the mission.
  The POI itself is private: public, the zones of every village's inspection were on everybody's chart.
- Nothing here reads a stock nor makes an order: the step is beside the supply chain, and shares only the
  treasury (an escrow it cannot pay mints, as for a leg).

## Consequences

- Around forty more places in `inventory` per ten villages, made once; none per mission.
- No inspection is offered while no game server runs for the planet: nothing lays its sites.
- The ground is read once: a terrain reworked later leaves the site where it was (clear `grounded_at`
  to have it laid again).
- A site may fall in a lake or on a cliff: nothing checks that it can be driven to. The zone is wide; a site that
  proves unreachable is to be moved by hand (its row), not by code.
- How many sites, how far and how often are constants: data of the entity when a second one needs others.
