# 0020. Breakdowns: ARES Machinery has a machine repaired

- **Status:** Proposed
- **Date:** 2026-11-11
- **Deciders:** WarpZone (playtest 2026-11-11)
- **Scope:** universesimulator, mission (`manual`), game (server: the machines; client: diagnosis and gauges)

## Context

After the inspections (ADR 0019), a second job that moves no goods, for another subsidiary: a machine of
a village breaks down, sparks, and cannot be used until a player has diagnosed and repaired it. The
repair is a small game in the client, there to stand for the work: nothing of it is simulated.

## Decision

- **The game server lists the machines.** The simulator knows the villages and their depots, not what
  stands in them. The game server says now and then which machines it has loaded on a planet
  (`PUT /api/internal/machines`, role `usim:deliver`: `{scene, machines: [{id, kind, position}]}`), its
  objects' own ids (a screen of an object that has several gets an id made of both). Each is given to the nearest active village within 3 km; one far from all is left
  out. `kind` is a slug of the game's (crusher, refinery, dispenser, terminal, kiosk, screen, lift, teleporter):
  the simulator
  only names it in the mission's text, "machine" when it does not know it. Table `machines`
  (migration 0014).
- **The simulator breaks them.** Each tick, while fewer than `SIM_BREAKDOWNS_OPEN` (2) machines of the
  entity are broken, one more breaks: a machine listed in the last day, in an active village where
  nothing is broken, the one repaired the longest ago. Table `machine_breakdowns`. The count is the
  rate: set high so that there is always a repair to take while the feature is played, 0 to stop.
- **The answer says what is broken.** The same call answers the breakdowns open on the planet
  (`broken: [{machineId, breakdownId, missionId}]`): the game server makes those machines unusable.
- **Clients read it too.** `GET /api/breakdowns` (`{items: [{machineId, kind}]}`) is what a client
  draws as broken: sparks, and the screen covered. Not replicated machine by machine: most machines'
  network definitions carry no state of their own, and a breakdown is the same for every kind of
  machine. The game server alone decides what a broken machine answers.
- **The mission** is the mission service's `manual` objective, which its issuer confirms: category
  `divers`, giver `{faction, division: machinery}`, one player, three hours, 1 500 credits,
  `autoComplete`. It is offered in the machine's village and names the machine (`payload.machineId`,
  `machineKind`).
- **Repaired by the game.** When the player who holds the mission has repaired the machine, the game
  server confirms the objective with the mission service (it has `mission:progress`; the simulator has
  not), which pays, then tells the simulator (`PUT /api/internal/breakdowns/{id}/repaired`). A tick that
  reads the mission as done repairs the machine too, should that second call be lost.
- **It stays broken.** A mission given up or expired is offered again on the same breakdown; nothing
  repairs a machine but a player (WarpZone).

## Consequences

- A broken crusher or refinery stops the missions that need it in that village until somebody repairs
  it: wanted, and the reason one machine at most is broken per village.
- No machine breaks while no game server runs for the planet, and a machine not listed for a day is
  left alone; one already broken stays so.
- The mission service is unchanged. The game server needs nothing new in Keycloak.
- Which machines break, how often and what a repair pays are constants and one setting: data of the
  entity when a second one needs others.
