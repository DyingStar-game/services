# NPC service

Runs many NPC clients in one Rust process. Each NPC opens its own WebSocket to
Horizon, authenticates with the same `player/init` handshake a Godot client
uses, then maintains a `WorldView` of the players and props in its GORC zones
and runs a trait-based action scheduler.

Idle rule: when no other player is in the NPC's zone, no actions fire and no
outbound frames are sent (other than what Horizon may pull from the socket).

## Layout

```
src/
  main.rs                 # tokio + tracing + Config + manager + admin REST
  config.rs               # ServiceConfig::from_env
  error.rs                # NpcError + IntoResponse
  horizon/                # WS client + GORC normalisation
  world/                  # WorldView (players, objects, channel merge)
  npc/                    # NpcManager, runtime, names, spec
  actions/                # Action trait + one file per behaviour
    detect_player_and_greet.rs   # IMPL
    go_to_point.rs               # IMPL
    drive_truck.rs               # stub
    check_vehicle_components.rs  # stub
    go_sell_object.rs            # stub
    conversations/
      greet.rs                   # IMPL
      small_talk.rs              # stub
  admin/                  # REST API for runtime add/remove
config/
  npcs.yaml               # roster loaded at startup
  names.txt               # display-name pool
```

## Configuration

Environment (see `.env.example`):

| Var | Default | Purpose |
|---|---|---|
| `HORIZON_URL` | `ws://server.dyingstar-game.space:7040` | Horizon WebSocket URL |
| `ADMIN_BIND` | `0.0.0.0:9400` | Admin REST bind |
| `ROSTER_PATH` | `config/npcs.yaml` | Roster file loaded at startup |
| `NAMES_PATH` | `config/names.txt` | Name pool (one per line) |
| `NPC_ACTIVE_COUNT` | all | How many of the first roster entries run at startup |
| `SPAWN_STAGGER_MS` | `200` | Delay between two roster spawns |
| `TICK_INTERVAL_MS` | `200` | Action scheduler tick |
| `RECONNECT_BACKOFF_INITIAL_MS` | `1000` | WS reconnect backoff start |
| `RECONNECT_BACKOFF_MAX_MS` | `30000` | WS reconnect backoff cap |
| `RUST_LOG` | `npc=info,tower_http=info` | Logging |

`config/npcs.yaml` (loaded at startup):

```yaml
npcs:
  - name: Alice              # optional; drawn from names.txt if omitted
    uuid: "<UUID>"           # required
    action_profile:
      - detect_player_and_greet
      - go_to_point
```

## Admin REST

Full spec: [`openapi.yaml`](openapi.yaml).

| Method | Path | Body | Result |
|---|---|---|---|
| GET | `/healthz` | — | `ok` |
| GET | `/npcs` | — | `{count, npcs:[{name, actions}]}` |
| POST | `/npcs` | `NpcSpec` JSON | `201 {name}` |
| DELETE | `/npcs/{name}` | — | `204` |
| GET | `/npcs/roster` | — | `{total, target, running}` |
| PUT | `/npcs/roster/active` | `{"count": 20}` (`null` = all) | `202 {total, target, running}` |
| POST | `/npcs/roster/scale` | `{"delta": 50}` / `{"delta": -30}` | `202 {total, target, running}` |
| POST | `/npcs/roster/reload` | — | `202 {total, target, running}` |

### Controlling how many roster NPCs run

The roster stays in memory in file order, with a *target*: the first `target`
entries should be live. A background reconciler spawns the missing ones in file
order, `SPAWN_STAGGER_MS` apart, and despawns the excess from the end of the
roster (last in, first out). The roster endpoints return as soon as the target
is set; `running` catches up over the following seconds. Scaling again mid-ramp
takes effect immediately.

- NPCs added with `POST /npcs` are not part of the roster and are never counted
  or removed by these endpoints.
- `reload` re-reads `ROSTER_PATH`: entries removed from the file are despawned,
  new ones are spawned if within the target. A changed entry keeps running on
  its old spec until it is despawned and respawned.

`scripts/npcctl` wraps these calls (`NPC_ADMIN_URL`, default
`http://localhost:9400`):

```sh
scripts/npcctl status      # {"total":300,"target":20,"running":20}
scripts/npcctl set 20      # first 20 entries
scripts/npcctl add 50      # 70
scripts/npcctl remove 30   # 40
scripts/npcctl set all
scripts/npcctl reload
```

Example:

```sh
curl -X POST http://localhost:9400/npcs \
  -H 'Content-Type: application/json' \
  -d '{"name":"Alice","uuid":"8af...","action_profile":["detect_player_and_greet"]}'
```

## Running

```sh
cp .env.example .env
make dev        # cargo run
make test       # cargo test --bins
make up         # docker/podman compose up -d
make down
```

## Adding a new action

1. Create `src/actions/my_action.rs` implementing `Action` (async-trait).
2. Add `pub mod my_action;` in `src/actions/mod.rs`.
3. Register it in `ActionRegistry::catalogue()` in `src/actions/registry.rs`.
4. List `"my_action"` under any NPC's `action_profile` to enable it.

Conversations follow the same pattern under `src/actions/conversations/`.

## Known limitations

- **Token provisioning**: NPC JWTs must be supplied externally (config or REST). No Keycloak helper yet.
- **Apartment selection**: the Godot protocol doesn't let a client choose its spawn location; NPCs are placed by Horizon.
- **Movement**: `go_to_point` assumes Horizon echoes the NPC's own position back via a `gorc_zone_enter`/`update` on the NPC's UUID; if not, it stops after the first tick.
- No clippy / fmt in CI yet.
