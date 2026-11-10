# 0016. The issuer's kind is data: ARES is a corporation

- **Status:** Proposed
- **Date:** 2026-11-10
- **Deciders:** WarpZone, with ddurieux and The_Moye (Discord, 2026-11-10), on the lore team's ruling (Theofrastus)
- **Scope:** universesimulator, game
- **Amends:** 0005 (which calls every entity governing supply a federation)

## Context

The lore team ruled that ARES is a corporation, with ten subsidiaries, and that nothing should present it
as a political body: politics is not written yet. The simulator holds ARES in a `federations` table, social
holds it as a political entity of type `federation`, and its missions carry `issuerType: politics`. The game
showed the players "politics" and "political entity". Other corporations, federations and organisations
will come.

## Options considered

1. **Wording only** -- show the name and drop the type. Nothing says what an entity is.
2. **A kind per entity, as data, and its wording in the game** -- the entity says what it is, the game names
   it in the player's language.
3. **Rename `federations` and add a type to social's political entities** -- right in the end, but a change
   across services for names players never read.

## Decision

We will take option 2.

- A row of `federations` has a `kind` (migration 0009): a slug (`corporation`, `federation`,
  `organisation`...), `federation` when never said. The seed sets ARES's to `corporation`; an import
  updates it.
- Every mission the simulator publishes says who orders it in its objective's payload:
  `payload.issuer = { name, kind }`. The mission service is not changed: it keeps the issuer as an id.
- The game names the kinds it knows in the player's language and shows "ARES · Corporation"; a kind it does
  not know shows the name alone. It never shows the issuer type `politics`.
- The kind is a description for players, not a type the services act on. In particular `corporation` here
  is not the mission service's issuer type `corporation`, which is a group made by players (a CEO, ranks,
  members): the two stay distinct in the data, whatever word the game shows for each.

## Consequences

- A new kind needs no code in the simulator: a seed value, and a translation in the game.
- The names `federations`, `politics` and `political` remain in tables, routes and roles. Option 3 stays
  open; nothing here stands in its way.
- Social still creates ARES as a political entity of type `federation`, with that type's default offices
  if it creates any: to be looked at when social learns organisations.
- ARES's subsidiaries are not entities yet: the game holds them as its brand table, the simulator names two
  of them as the missions' giver division.
