# NPC service

Runs many NPC clients in one Rust process. Each NPC opens its own WebSocket to
Horizon, authenticates with the same `player/init` handshake a Godot client
uses, then maintains a `WorldView` of the players and props in its GORC zones
and runs a trait-based action scheduler.

Idle rule: when no other player is in the NPC's zone, no actions fire and no
outbound frames are sent (other than what Horizon may pull from the socket).

Every NPC is a **Keycloak user** (group `npcs`, realm role `npc`) stored in
**PostgreSQL** with its login, sealed password, names, role, routine and
configuration. It logs in with its own login and password and presents the
access token to Horizon, which derives `is_npc` from the `npc` role. The admin
panel creates, spawns, edits and watches the NPCs through the admin API.

## Layout

```
src/
  main.rs                 # startup, `import-schedules` CLI, graceful shutdown
  config.rs               # Config::from_env
  error.rs                # NpcError + IntoResponse
  db/                     # PostgreSQL: npcs, patterns, tasks, events, settings, workplaces
  keycloak/               # svc-npc admin calls, per-NPC token manager (password grant + refresh)
  auth/                   # Keycloak JWT check of the admin API callers
  crypto.rs               # AES-256-GCM sealing of the NPC passwords
  roles.rs                # role templates (storekeeper, villager)
  usim.rs                 # universesimulator: main warehouse of the villages
  logs.rs                 # per-NPC ring buffer of log lines (tracing layer)
  events.rs               # key events written to `npc_events` in the background
  schedule.rs             # shift patterns + hot-swappable ScheduleStore
  schedule_import.rs      # config/schedules.yaml -> database
  horizon/                # WS client + GORC normalisation
  world/                  # WorldView (players, objects, channel merge)
  npc/                    # manager (reconciler), runtime, assign, provision, names, spec
  actions/                # Action trait + one file per behaviour
    manual.rs                    # tasks given from the admin panel
    drive_to_factory.rs          # garage kiosk -> truck -> road -> Factory
    driving/                     # route-following controller + vehicle frames
  admin/                  # admin REST API (/api/admin)
  world_data/             # villages (POI) + road graph, from the QGIS exports
  scene_layouts.rs        # kiosk/seat/door points, from the Godot export
migrations/               # embedded SQL migrations
config/
  schedules.yaml          # shift patterns, imported into the database
  roles.yaml              # storekeeper / villager templates
  routing.yaml            # planet + Mining -> Factory choices for driving
  world/<planet>/         # poi.json + roads.json (copies of the QGIS exports)
  scene_layouts/          # JSON exported by DyingStar/tools/npc/npc_layout_export.gd
```

## Configuration

Environment (see `.env.example`):

| Var | Default | Purpose |
|---|---|---|
| `HORIZON_URL` | `ws://server.dyingstar-game.space:7040` | Horizon WebSocket URL |
| `ADMIN_BIND` | `0.0.0.0:9400` | Admin REST bind |
| `DATABASE_URL` | `postgres://npc:npc@localhost:5432/npc` | Roster, schedules, tasks, events |
| `KEYCLOAK_URL` / `KEYCLOAK_REALM` | `http://keycloak:8080` / `dyingstar` | Token endpoint + Admin REST |
| `NPC_LOGIN_CLIENT_ID` / `_SECRET` | `npc-login` | Client the NPCs log in with (password grant) |
| `SERVICE_CLIENT_ID` / `_SECRET` | `svc-npc` | Manages the NPC users, reads universesimulator |
| `NPC_PASSWORD_KEY` | — (required) | Base64 of 32 bytes sealing the passwords |
| `OIDC_ISSUER` / `OIDC_JWKS_URL` / `OIDC_AUDIENCE` | from `KEYCLOAK_URL` | Admin API tokens |
| `AUTH_DEV_BYPASS` | `false` | Dev: `X-Player-Id` / `X-Player-Roles` headers instead of a token |
| `UNIVERSESIMULATOR_URL` | — | Main warehouse of a village (storekeepers) |
| `NPC_EVENTS_RETENTION_DAYS` | `7` | Events kept in the database |
| `NPC_LOG_BUFFER_LINES` | `500` | Log lines kept in memory per NPC |
| `ROLES_PATH` | `config/roles.yaml` | Role templates |
| `SCHEDULES_PATH` | `config/schedules.yaml` | Imported into the database |
| `SPAWN_STAGGER_MS` | `200` | Delay between two spawns |
| `TICK_INTERVAL_MS` | `200` | Action scheduler tick |
| `RECONNECT_BACKOFF_INITIAL_MS` | `1000` | Reconnect backoff start |
| `RECONNECT_BACKOFF_MAX_MS` | `30000` | Reconnect backoff cap |
| `WORLD_DATA_DIR` | `config/world` | Villages + roads per planet |
| `ROUTING_CONFIG` | `config/routing.yaml` | Planet and Mining → Factory choices |
| `SCENE_LAYOUTS_DIR` | `config/scene_layouts` | Kiosk / door / seat points per scene |
| `DRIVE_TICK_MS` | `100` | Control period while an NPC drives |
| `RUST_LOG` | `npc=info,tower_http=info` | Logging |

## NPC lifecycle

- **Create** (`POST /api/admin/npcs {count}`): a generated name (first + last from
  the `fake` crate's locale lists, mostly French, then English, German,
  Italian, Brazilian; unique), a Keycloak user `npc-<8 hex>` in group `npcs` with a random 32-char
  password, then the row keyed by the Keycloak user id (= JWT `sub` = Horizon
  player id). A failed insert deletes the account again.
- **Run**: an NPC is connected when the **global switch** is on
  (`PUT /api/admin/settings {npcsEnabled}`) and its own `enabled` is set. A
  reconciler applies every change (spawns staggered, despawns, and hot-pushes
  edited configurations to running NPCs without reconnecting).
- **Tokens**: an access token is reused while it has 10 min left, then
  refreshed; the password grant only runs when both are gone. A password
  Keycloak rejects (expired by the realm policy, out of step) is reset through
  the admin API and stored, once; repeated failures back off up to 10 min.
- **First spawn**: Godot places the NPC in a habitation. Once its position is
  known, an `unassigned` NPC in a Mining village (`Mxxxx`) becomes one of its
  **five storekeepers** (`magasinier_1..5`, positions local to the village's
  cargo_depot from `config/roles.yaml`) — the warehouse comes from
  universesimulator (main warehouse), else from the nearest replicated
  cargo_depot, and is cached per village — or a **villager** once the five
  slots are taken. Concurrent spawns are serialised per village. No warehouse
  found: the NPC strolls and the assignment is retried at its next session.
- **Village workplaces**: at startup and every 10 min, the main warehouse of
  every Mining village is read from universesimulator into the cache, before
  any NPC spawns there (admin panel › Villages). A manual workplace is never
  replaced; storekeepers already assigned keep their place. Without
  `UNIVERSESIMULATOR_URL` (or `SERVICE_CLIENT_SECRET`), only a depot an NPC
  sees can fill it, so a village stays "not found yet" until one spawns.
- **Manual tasks** (`POST /api/admin/npcs/{id}/tasks`): `go_to_position`,
  `run_action` (any catalogue action, outside the schedule) or `wait`; they
  pre-empt the schedule, and `forceOnline` connects an off-shift NPC.
- **Logs**: each runtime runs in a tracing span `npc{id, name}`; its lines (and
  its actions') land in a per-NPC ring buffer (`GET …/logs?since=`). Key events
  (spawn, connection, action changes, role, tasks, credentials) go to
  `npc_events`, kept 7 days.
- **Delete**: despawn, delete the Keycloak user, delete the row.

## Admin REST

Full spec: [`openapi.yaml`](openapi.yaml). Everything is under `/api/admin`
and needs a staff token (moderator/admin/supervisor; writes admin or
supervisor), as forwarded by the adminpanel BFF. Public: `GET /healthz`.

| Method | Path | Purpose |
|---|---|---|
| GET / PUT | `/settings` | Global switch + counts |
| GET / POST | `/npcs` | Roster with live status (filters `village`, `role`, `enabled`, `online`, `q`) / create N |
| GET / PATCH / DELETE | `/npcs/{id}` | Detail (routine, tasks, state, workplace) / edit / delete |
| POST | `/npcs/{id}/spawn` · `/despawn` · `/reassign` · `/reset-credentials` | One NPC |
| POST | `/npcs/bulk` | `{action: spawn\|despawn\|delete, ids \| count, village?, role?}` |
| GET | `/npcs/{id}/logs` · `/npcs/{id}/events` · `/events` | Logs and events |
| GET / POST / DELETE | `/npcs/{id}/tasks[/{taskId}]` | Manual tasks |
| GET | `/task-kinds` | What the task dialog offers |
| GET / POST / PUT / DELETE | `/schedule/patterns[/{name}]` | Shift patterns (hot-reloaded) |
| POST | `/schedule/import` | Import `config/schedules.yaml` (`{mode: missing\|overwrite, yaml?}`) |
| GET / PUT | `/schedule/activity-actions` | Activity → allowed actions |
| GET | `/schedule/legend` | Grid letters |
| GET / PUT / DELETE | `/villages[/{code}/workplace]` | Villages, storekeepers, workplace override |
| GET | `/debug/world` · `/debug/routes/{from}/{to}` · `/health/keycloak` | Debug views, Keycloak orphans |

`scripts/npcctl` wraps the common calls (`NPC_ADMIN_URL`, `NPC_TOKEN`):

```sh
scripts/npcctl status        # global switch + counts
scripts/npcctl create 10     # 10 new NPCs
scripts/npcctl on            # connect the enabled NPCs
scripts/npcctl spawn 20      # enable 20 more
scripts/npcctl despawn 5
scripts/npcctl import        # config/schedules.yaml -> database (missing only)
```

## Running

```sh
cp .env.example .env     # set the Keycloak secrets and NPC_PASSWORD_KEY
make dev                 # cargo run (needs DATABASE_URL)
make test                # cargo test; DB tests need NPC_TEST_DATABASE_URL
make up                  # docker/podman compose: service + PostgreSQL
make down
```

The database tests create a fresh database per test on the server named by
`NPC_TEST_DATABASE_URL` (e.g. `postgres://postgres:test@localhost:5432/postgres`)
and are skipped without it.

In Kubernetes the service runs on node 2 with its own CNPG database (chart
`kubernetes/service-npc`); one replica only, so no NPC is ever logged in twice.

## Driving to the Factory village (`drive_to_factory`)

An NPC spawned in a Mining village (`Mxxxx`) walks to the village garage,
presses the truck kiosk, opens the driver's door, boards, drives **by the road**
to its Factory village (`Fxxxx`), parks and gets out. The service decides and
steers everything; the game server only validates the same actions a player
sends (`kiosk_spawn`, `vehicle_door`, `enter_vehicle`, `vehicle_input`, …).

Enable it per NPC:

```yaml
    action_profile: [drive_to_factory]
    drive_to_factory:          # optional
      destination: F0003       # default: routing.yaml choice for its village
      max_speed_kmh: 40        # default: routing.yaml max_speed_kmh
```

Static data it needs, all under `config/`:

- **Villages and roads** — `world/<planet>/poi.json` and `roads.json` are copies
  of `DyingStar/assets/qgis/export/<planet>_poi.json` and
  `<planet>_roads_buffered.json`. Copy them again after a QGIS re-export. The
  road graph is inferred at startup (shared vertices within `road_snap_m` =
  intersections). On tarsis_3 the road layer is made of separate regional
  networks: a Mining village with no Factory on its own network gets none
  (`GET /npcs/villages` lists them with `"factory": null`).
- **Routing** — `routing.yaml`: planet, speed caps, and `factory_for` to pin a
  Mining village to a Factory; otherwise the Factory nearest *by road* is used.
- **Scene layouts** — exported from Godot into `scene_layouts/`:
  `garage.json` (kiosk, where the truck appears), `truck.json` (driver seat and
  door, wheelbase, steering lock) and the village layouts
  (`ares_village_mining.json`, `ares_city_factory.json`: where their garages
  stand and the ground footprint of every building). Regenerate them after
  changing those scenes, with the project's double-precision editor binary:

  ```sh
  cd DyingStar
  <godot> --headless --path . -s res://tools/npc/npc_layout_export.gd -- out=<services>/npc/config/scene_layouts
  ```

  The "Identifier not found: GameOrchestrator" errors it prints are expected
  (no autoloads under `-s`); the exporter reads the scenes' stored values
  instead of running their scripts.

The road can pass several hundred metres from a garage, so a trip has three
parts, all planned here:

1. **Out of the village**: straight out of the garage door (the truck spawns
   facing it), then round the village's buildings to the nearest road point —
   a grid A* over the building footprints, placed with the replicated
   `poi_village` pose (`actions/driving/offroad.rs`).
2. **The road**, along the road graph.
3. **Into the destination village**: as soon as the Factory village is in view
   (villages replicate up to 10 km away), the trip is re-planned to leave the
   road at the point nearest one of its garages and wind round the buildings to
   park in front of it — at the garage's `NpcParking` marker if the scene has
   one (a `Marker3D`, -Z = heading), else 15 m out of its door.

### Lanes and traffic (`actions/driving/{lanes,obstacles,traffic}.rs`)

The truck keeps **right**, its centre 1.25 m from the right edge on a two-way
road (1.75 m right of the centreline on a 6 m road), or in the outer lane of a
highway. Two trucks (2.19 m wide) crossing on a 6 m road then keep 1.3 m
between them. Road width and
lanes come from `roads.json`. Every tick (ADR `docs/adr/0012` at the
repository root), the driver:

1. **looks**: the replicated vehicles, people on foot, crates, containers and
   mining rocks within 250 m. It skips its own truck and what rides in it.
   Sizes come from each scene's exported `footprint` (the exporter above), else
   a default per type. Velocity is `velocity_local` for vehicles, and is
   estimated from positions for the rest;
2. **decides**, in route coordinates (`traffic.rs`, mode in brackets):
   - *nothing ahead* → cruise in the lane (`cruise`);
   - *a vehicle ahead, about as fast* → follow 5 m + 1.5 s × speed behind (`follow`);
   - *a vehicle much slower (> 3 m/s)* → overtake in the other lane (`overtake`);
   - *something standing on the road* → steer round it through the widest free
     gap (`avoid`).

   On a two-way road, the truck only moves into the other lane when nothing is
   coming within the time it needs to pass. Vehicles are only seen up to
   200 m, so it never starts a pass longer than that;
   - *oncoming in our lane, or someone crossing* → squeeze right, slow, stop (`yield`);
   - *no way through* → stop 10 m short and wait (`blocked`), counting from
     when the truck has stopped:
     - if someone is within 25 m of the blockage (a person, or a vehicle with
       someone aboard), the truck honks at 10 s and 40 s and waits **60 s**;
     - if nobody is there, it waits **5 s**.

     Then it goes round it **off the road** (`bypass`): an A* detour, on the
     side with more room, keeping clear of everything within 40 m, driven at
     8 km/h, then back onto the route. If the truck gets stuck on that side (a
     cutting's wall, a tunnel), it tries the other side. If both fail, it gives
     the trip up (`error: "road blocked"`), parks and gets out;
3. **drives**: the controller follows the lane offset blended with the
   manoeuvre, under the speed cap and stop point decided.

   The truck's wheels turn slowly: 0.2 rad/s out from centre and 1 rad/s back,
   from `truck.json`'s `steer_speed` and `steer_return_speed`. The controller
   steers at those rates, and slows down whenever the wheels lag the angle
   wanted, so it does not cut the corners.

Terrain itself (slopes, cutting walls) is not modelled. A truck stuck with
nothing in sight backs out and retries, then is reset.

Progress is on the NPC's blackboard under `drive_to_factory`, also shown by
`GET /npcs/world`: phase, villages, truck, metres driven and remaining, lateral
position, and `traffic` (`mode`, `lead`, `lead_gap_m`, `blocked_s`,
`attended`, `bypass_attempts`).

## Mining-village archetype (250 inhabitants)

`config/village/mining_village.yaml` describes a mining village once: its buildings (from the
village sketch), its roles and headcounts, a dwelling for each NPC, and the coefficients of
its resource budget. `scripts/village_gen.py` expands it, with the rotas of
`config/schedules.yaml` (`equipe_*`, `jour_*`, `soir_*`, `enfant`, `retraite`, `refugie`):

```bash
python3 scripts/village_gen.py --check                 # validate, write nothing
python3 scripts/village_gen.py --village-uuid <uuid>   # regenerate the outputs
```

- `config/npcs.mining_village.yaml` and `config/npcs.farm_village.yaml` are the generated rosters
  (250 and 200 NPCs, each with its own home). The service no longer loads YAML rosters (NPCs
  live in the database); they remain a design reference for staffing and budgets.
- `config/village/budget.<archetype>.generated.md` gives staffing per hour, the load of each building,
  water, food and ore per day, truck trips and stock levels.
- `config/village/usim_profile.generated.json` holds the values copied into
  `universesimulator/seeds/tarsis_3-ares.json`.

`scripts/village_geology.py` finds the outcrop rock under every village of tarsis_3, using
the DyingStar repository's biome export, and writes `config/world/tarsis_3/village_geology.json`.
The generator uses it for each mining village's ore. With `--write-seed`, it also writes the
universesimulator seed: resources, profiles of mining villages, factories, railway cities and
spaceports, and route links.

The farm villages (sealed greenhouses above 4,800 m, `config/village/farm_village.yaml`) do
not exist in QGIS yet: `scripts/village_farms.py` proposes them from the heightmap, the biomes
and the roads (`config/world/tarsis_3/farm_villages.json`, plus a `.geojson` to import into
QGIS). The food is the consumables of the narrative design (`config/food/consumables.yaml`),
eaten by diet and made from the farm produce.

```bash
python3 scripts/village_geology.py --dyingstar ../../DyingStar   # after a map re-export
python3 scripts/village_farms.py --dyingstar ../../DyingStar     # after a map or food change
python3 scripts/village_gen.py --write-seed
```

Positions are placeholders until the village scene exports markers. Decisions: ADR
`docs/adr/0002`–`0010` at the repository root.

## Adding a new action

1. Create `src/actions/my_action.rs` implementing `Action` (async-trait).
2. Add `pub mod my_action;` in `src/actions/mod.rs`.
3. Register it in `ActionRegistry::catalogue()` in `src/actions/registry.rs`.
4. Add `"my_action"` to an NPC's `actionProfile` (admin panel), or to a role
   template in `config/roles.yaml`.

Conversations follow the same pattern under `src/actions/conversations/`.

## Known limitations

- **Single replica**: two service instances would log the same NPCs in twice; scaling out needs per-NPC leases.
- **Global switch**: one switch for the single `HORIZON_URL`, not per Horizon server.
- **Apartment selection**: the Godot protocol doesn't let a client choose its spawn location; NPCs are placed by Horizon.
- **Movement**: `go_to_point` assumes Horizon echoes the NPC's own position back via a `gorc_zone_enter`/`update` on the NPC's UUID; if not, it stops after the first tick.
- No clippy / fmt in CI yet.
