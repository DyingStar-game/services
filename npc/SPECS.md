List of task a NPC can play (actions):

 - reception_players
 - depot_store_reception_to_temp_area
    reception_deposited_item (bool), if not null, there is a box on the reception deposit
    pnj_box_temp_zones (array {"zone_0: 1}), 1 = number of boxes in the temp zone

 - depot_store_item

    Moves boxes from the temp zone onto the warehouse shelves — the step after
    depot_store_reception_to_temp_area. Priority 94 (just below the reception
    action's 95): a fresh deposit on the counter always wins. The two actions
    use separate "box in hands" blackboard keys and each refuses to start while
    the other one's box is in hands, so an interrupted carry always resumes in
    the right flow.

    Triggers while AtWork whenever the depot's `pnj_box_temp_zones` lists a box
    that is replicated into view and not parented to a player (nobody carrying
    it). Before grabbing anything the NPC checks that some shelf has a free
    slot — a box grabbed with nowhere to put it down would stay in its hands
    forever. It then walks to the matching `pnj_zones.TempZone` place, picks
    the box up (grab confirmed by the box reparenting under the NPC), carries
    it to the NEAREST usable shelf, and drops it into the lowest free slot
    (drop confirmed by the box reparenting off the NPC; the slot is re-checked
    after the walk and re-picked if someone filled it meanwhile).

    ### Shelf contract

    Shelves replicate as objects: `object_type: "shelf"`, `parent_id` = depot
    uuid, `position`/`rotation` depot-local (radians, yaw in `rotation.y`),
    `scenename`, and ONE slot property:

    - `shelf_slots_occupied`: array of booleans, one per slot, `true` =
      occupied. Server-maintained: flipped when a box lands in / leaves a
      slot's detection area.

    The slot GEOMETRY is deliberately NOT replicated (it is static per scene
    and only the NPC needs it): it ships as JSON files exported from the
    Godot editor, loaded by the NPC service at startup from
    `SLOT_LAYOUTS_DIR` (default `config/slot_layouts`). The files mirror the
    Godot scene tree — the layout for
    `res://scenes/_universe/props/furniture/metal_shelf.tscn` lives at
    `config/slot_layouts/scenes/_universe/props/furniture/metal_shelf.json`
    (the relative path, `.json` -> `.tscn`, is the lookup key):

    ```json
    {
      "schema_version": 1,
      "scenename": "scenes/_universe/props/furniture/metal_shelf.tscn",
      "object_type": "shelf",
      "front": [0.0, 0.0, 1.0],
      "slots": [[-1.25, 0.35, 0.42], [-0.75, 0.35, 0.42], ...]
    }
    ```

    - `slots[i]` is the centre of slot `i`, LOCAL TO THE SCENE ROOT NODE, in
      metres, index-matched with the replicated `shelf_slots_occupied` —
      export them from the scene's slot markers, ordered bottom row first,
      left-to-right as seen when facing the front.
    - `front` is the access-face direction in the scene's local frame (the
      side the NPC approaches from; default +Z). Its length scales the
      standing distance: the NPC stands at `slot + front × stand_distance_m`,
      so `[2.5, 0, 0]` means "approach from +X, 2.5× the configured
      distance".
    - Depot-local point = `shelf.position + rotate_y(shelf.rotation.y) * local`
      (Godot yaw: `Vector3(1,0,0).rotated(UP, PI/2) == Vector3(0,0,-1)`).
    - A shelf whose scene has no layout file (or whose
      `shelf_slots_occupied` length mismatches the layout's slot count) is
      treated as "not usable yet" and silently skipped — the action stays
      inert-but-safe until the layout is exported.
    - The temp-zone shelf — same scene and `object_type` as the warehouse
      shelves — is NEVER a drop target: it is where the box comes from.
      Recognized by its `name` (`shelf_cargo_temp_zone` prefix, top-level or
      nested under `object_data`), and as a fallback geometrically: any shelf
      standing within 2.5 m (ground plane) of a `pnj_zones.TempZone` place
      (`target` point or rectangle centre) is treated as the temp rack even
      when no `name` replicates.
    - The NPC stands `stand_distance_m` metres (per-NPC config, default 1.0)
      in front of the target slot's column along `front`, at the shelf's
      floor height, and aims its head at the slot point before releasing
      the box.

    NOTE: the current
    `config/slot_layouts/scenes/_universe/props/furniture/metal_shelf.json`
    holds PLACEHOLDER positions (6 columns x 4 levels) — re-export the real
    marker positions from Godot.

    Assumption (server side): the depot removes a box's uuid from
    `pnj_box_temp_zones.zone_<n>` when the box leaves the temp deposit area,
    symmetric to how it adds it on entry. The NPC only reads that property.

 - depot_transfer_to_loading_dock
 - depot_transfert_to_reception
 - going_to_work
 - go_to_home
 - waiting
 - work_break
 - validate_mission
 - pay_players
 - load_into_vehicle
 - drive_to


## Planetary shift schedule (25 h day)

The five cargo-depot NPCs follow a rotating shift rota tied to the planet's
rotation, not to real wall-clock cadence. Three pieces layer on top of the
existing action scheduler:

### Planetary clock (`src/clock.rs`)

Because 1 h real = 1 h game, wall-clock time *is* planetary time. resourcesDynamic
derives a planet's rotation phase with **no epoch offset**, so:

```
planetary_hour = (unix_seconds / 3600) mod rotation_h
rota_day       = floor(unix_seconds / (rotation_h · 3600)) mod ROTA_DAYS
```

No state is stored — every instance and restart agrees. `rotation_h` per planet
comes from resourcesDynamic (below); it is **not** hardcoded (the sandbox depot
is on `tarsis_4`). Test hooks `CLOCK_OVERRIDE_UNIX` (freeze) and `CLOCK_TIME_SCALE`
(speed up) make a full rota testable in minutes.

### Rotation source (`src/resources_dynamic.rs`)

At startup (and on a periodic refresh) the service fetches
`GET {RESOURCES_DYNAMIC_HTTP}/api/planets` and builds a `name/internal_name →
{uuid, rotation_h}` map. Each NPC's planet comes from `place_where_work.planet`
(default `PLANET_INTERNAL_NAME`). If resourcesDynamic is unreachable, NPCs fall
back to `ROTATION_H_FALLBACK` (25.0) and keep running.

### Schedules (`src/schedule.rs`, `config/schedules.yaml`)

Each NPC references a pattern by name via its roster `schedule:` field. A pattern
is a grid of planetary hours — one line per rota-day, **exactly 25 chars**, one
char per hour:

```
T travail (work)   t transmission (handoff)   r repas (meal)
a activités obligatoires (mandatory)   . off-shift
```

Each hour maps to an **activity**, and each activity to a set of allowed action
names (code-default, YAML-overridable via `activity_actions`):

| activity        | char | allowed actions                                             |
|-----------------|------|-------------------------------------------------------------|
| Work / Transmission | `T` `t` | `going_to_work`, depot actions, `reception_players`    |
| Meal            | `r`  | `go_to_meal`                                                |
| Mandatory       | `a`  | `mandatory_activity`                                        |
| Off             | `.`  | `go_home`                                                   |

`detect_player_and_greet` is always allowed (ambient). The gate is applied in
`ActionRegistry::pick`: on each tick the runtime computes the allowed set from
`(schedule, clock.now())` and only those actions are eligible — layered on top of
the NPC's static `action_profile` and each action's `can_run`.

### MVP activity actions

`go_home`, `go_to_meal` and `mandatory_activity` are one generic behaviour
(`src/actions/go_to_place.rs`): walk to a place, wait for arrival, then idle
there for the window. `go_to_meal`/`mandatory_activity` target `meal_place` /
`mandatory_place` from the roster; `go_home` needs no config — the runtime
remembers the NPC's **spawn position** as home (an NPC spawns in its
habitation), so it returns there (an optional `home:` overrides this).
`going_to_work` was generalised to resume work from *any* settled status
(home, meal, mandatory), so it is the universal "return to work" action — the
schedule only makes it eligible during `T`/`t` hours, so it never fights the
break behaviours. Transmission (`t`) shares Work's action set for MVP (the box
flow keeps moving during a handoff); richer handoff logic is deferred.

### Ambient strolling (`take_a_walk`)

`src/actions/take_a_walk.rs` makes an NPC wander: it draws a point uniformly
from the horizontal disc of radius `radius_m` (default **100 m**) centred on
wherever it currently stands, asks the server to walk there
(`npc_go_to_position`), and on `npc_arrived` stands still for `pause_s`
(default **30 s**) before drawing the next point — indefinitely.

The order is expressed in the NPC's own frame (its `parent_id`, empty = root),
the same frame Horizon echoes its position back in, so no coordinate conversion
is involved. The action stands down until that position is known, since there is
nothing to draw a point around. Radius and pause are tuned per NPC from the
roster:

```yaml
    action_profile:
      - take_a_walk
    take_a_walk: { radius_m: 100.0, pause_s: 30, max_distance_m: null }
```

`max_distance_m` leashes the stroll to that many metres around the NPC's
**spawn position** — the one the runtime already remembers as home
(`home_position` on the blackboard, shared with `go_home`). Points drawn outside
the leash are re-drawn a few times and, failing that, pulled back onto its
boundary circle, which also walks an NPC that somehow ended up outside it back
in. `null`/unset = no limit. If the NPC has been re-parented since spawn, the
anchor is reconciled through the root frame; when that chain is not in view the
NPC keeps walking *unleashed* and says so with a warning, rather than being
leashed to a plausible-looking wrong spot.

Like the other travel actions it is polled by the scheduler rather than looping
inside `run` (so greeting still runs while it is en route), it uses the same
`TripWatch` retry (a walk unanswered for 120 s is dispatched again, toward a
*fresh* point since the last one may be unreachable), and it moves through two
statuses: `Walking` (en route) then `WalkPause` (standing at the point reached).

Priority is **20** — below greeting (100) and every scheduled behaviour, above
plain `go_to_point` (10). Each hop is drawn around the NPC's *current* position,
so an unleashed stroll is a random walk that drifts well beyond `radius_m` from
where it started — set `max_distance_m` to bound it. Note that `take_a_walk` belongs
to no activity set, so a scheduled NPC only ever runs it if the schedule's
`activity_actions` / `always_allowed` lists it (an unscheduled NPC just needs it
in its `action_profile`).

### Unanswered trips

Every walk is fire-and-forget: `npc_go_to_position` goes out and the NPC leaves
its travelling status only when the server replicates `npc_arrived` back. When
that answer never comes — the order was dropped, or the server could not path
out of the habitation the NPC spawned in — the NPC waited for it in silence for
the rest of the window, looking exactly like one with nothing to do.

`src/actions/trip.rs::TripWatch` closes that hole for both travel actions: a
walk unanswered for `ARRIVAL_TIMEOUT` (120 s) is logged as a warning and
dispatched again, restarting the watchdog. A real `npc_arrived` still wins over
it. A trip the server genuinely cannot complete therefore retries every two
minutes with a `waiting_s` + `my_parent` warning, instead of failing silently.

### Observability

`GET /npcs/schedule` (admin API) reports each NPC's planet, rotation period,
planetary day/hour and current scheduled activity — for verifying the rota as
the day drifts against real time.

`GET /npcs/world` reports, per NPC, its FSM `status`, `my_parent` /
`my_position` / `npc_arrived`, how many objects and players it sees, and the
full replicated property set of its depot. Between them those answer the two
questions an idle NPC raises: *which status is it stuck in* (an NPC waiting on
an arrival notice reads `GoingToWork` with `npc_arrived: false`) and *what does
it actually see* on the object its behaviour is driven by.


## Reception coordination (who works the counter)

When several magasiniers are on shift at the same depot, exactly **one** works
the reception counter (`depot_store_reception_to_temp_area`) while the others
stock the shelves (`depot_store_item`). The single reception NPC also *helps*
stock, but only when reception is idle — no player waiting
(`reception_players`), no box on the counter (`pnj_reception_deposited_item`),
and it is not mid-carry or owed a walk back to its spot. Alone on shift, that
one NPC is the reception NPC and so does both, reception first.

### Stocker takeover

The split is a preference, not a lock. A box that has sat on the counter for
`STOCKER_GRACE` (6 s) without being collected is taken by a **stocker** too: by
then the reception NPC has either set off (it acts on the next tick) or is
mid-carry, at a meal, or absent. Within that window the stocker also refuses to
start a new shelf carry — a carry runs far longer than the grace, so beginning
one would leave the box waiting anyway. The grace is what keeps the two from
walking to the same box in the normal case, and the *expiry* is what stops a box
nobody accepts (a refused type stays on the counter for good) from halting
stocking forever.

The waiting time comes from `ObjectState::property_age`, which stamps a
replicated property when its **value** changes rather than when it was last
received. The session loop keeps applying frames while an action runs, so the
age stays true even across a 60 s shelf carry.

### Server-arbitrated claim (`reception_owner`)

The decision is authoritative and race-free: the depot replicates a single
`reception_owner` property (a player uuid, `""` = free), maintained by the Godot
game server (`cargo_depot.gd`). An NPC takes reception by sending
`{action: "claim_reception", uuid: <depot>}`; the server grants it only when the
counter is free (single-threaded, so the read-check-set is atomic) and clears it
on `{action: "release_reception", …}`, on disconnect, or when the owner leaves.
Each NPC reads `reception_owner` to pick its role
(`src/depot.rs::role_from_owner`): owner == me → reception; anyone else (or empty
= not yet claimed) → stocker. The role gates live in the two depot actions'
`can_run`, always *after* their in-flight-carry checks so a role flip mid-carry
never strands a box.

`src/actions/depot_reception_coordinator.rs` drives the claim and voices it: it
says "Réception libre ?" and claims when the counter is free, "Je prends la
réception." once it owns it, and "Ok, je m'occupe des rayons." once it has
**heard** another magasinier claim it — NPCs now retain a peer's replicated
`conversation` line (`WorldView::heard_from`), so they genuinely hear each other
(dialog is otherwise outbound-only). Nearby players see the whole exchange.

### Deterministic fallback

If the depot does not replicate `reception_owner` at all (an older game server),
NPCs fall back to a no-communication election over the shared schedule
(`DepotWorkforce::role_for`): among co-workers on shift now, the one whose
current Work/Transmission block started earliest (tiebreak: lowest uuid) is the
reception NPC. This keeps the NPC service deployable before the game-server
change ships.

### Stale claims

A `reception_owner` naming a magasinier who is **not on shift right now** is
treated as stale (`DepotWorkforce::is_off_shift_worker`) and the election above
runs instead — otherwise one NPC walking off to a meal, a mandatory activity or
home with the counter still in its name makes every co-worker read itself as a
stocker, and a box deposited at the reception is never collected. An owner the
roster does not know (a player, an NPC from another process) keeps its claim.

Both ends of that are covered: an NPC that stops working hands the counter back
even when it never claimed it — the game server may grant the role on its own,
and a role we never claimed is exactly the one we would otherwise never release.
The role each NPC resolves is logged whenever it changes (`"depot role"`), since
a silent flip is the hardest depot stall to diagnose; `GET /npcs/world` shows the
depot's replicated properties (including `reception_owner`) per NPC.

### Cross-service touch points

- Game server (GDScript): `DyingStar/scenes/_universe/structures/industrial/cargo_depot.gd`
  (`reception_owner` + claim/release/clear), `scenes/player/player_server.gd`
  (the `claim_reception` / `release_reception` action arms), `server/server.gd`
  (clear the owner on disconnect).
- Horizon: `ds_genericprops/props/cargo_depot_def.json` (replicate
  `reception_owner` on channel 0) and `startup_items.json` (seed `""`).
- Protocol: `network-protocol/schemas/client-to-horizon/player.action.schema.json`
  documents the two verbs. NOTE: a full end-to-end claim-race test needs the
  Godot game server (the claim is arbitrated there), so it is not part of the
  schema mock-server e2e harness.


