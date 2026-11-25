# weather — DyingStar planetary weather service

Replayable weather for every body of a system, published to Horizon through its generic `ds_bridge`
plugin as `weather_system` objects, plus an HTTP API for maps. The model is a **pure function of
(data, seed, time)**: no database, restart-safe, any instant (past or future) answers instantly.

- `crates/weather-core` — the model: climate derivation (energy balance, air column, circulation
  regime, dust thresholds), orbit and spin twins of the game, deterministic hashing, the synoptic
  systems (depressions, anticyclones, dust storms, rain fronts) and the point sampler.
- `crates/ds-weather-proto` — the wire contract: `ds_bridge` envelope, `weather_system` object,
  `weather_system_def.json` (lockstep with `horizonserver/ds_genericprops/props/` and the game's
  `items_def/`).
- `crates/weather-service` — the process: `/ws` (ds_bridge), `/health`, `/metrics`,
  `/api/v1/bodies`, `/api/v1/{body}/climate|systems|point|fields|forecast`.
- `crates/weather-atmosphere` — an atmosphere model, **stateful** (see below): dry hybrid
  sigma-pressure primitive equations on an icosahedral hexagonal grid (~240 km), Held-Suarez
  relaxation with seasons, column physics as components (ground and day-night, two dust classes,
  dust sea, lid, dry convection, game events), bit-exact restart files, a guard with automatic
  roll-back, outputs as 16-bit PNG tables. Bodies come from the same data through `weather-core`.
  Example for offline runs: `cargo run --release -p weather-atmosphere --example offline_run --
  --body Sandbox --out out/sandbox --spinup-days 5 --record-days 10` (options in
  `examples/offline_run.rs`).
- `data/climate/<system>.json` — the knobs the celestial data does not carry (seed, dust grains, lift
  bias, thermal inertia, humidity, wind and dust caps), by body name.
- `data/atmosphere/` — settings of the atmosphere model (`defaut.toml` for every body, `<body>.toml`
  for what the data does not give) and the rasters they use; see its `README.md`.

Science, conventions and simplifications: the weather R&D report of 2026-10-06 (not in this repository yet)
(to be published in technical-docs with the PR).

## Run

```
cargo test --workspace
SYSTEM_JSON=../resourcesDynamic/data/system/tarsis.json cargo run -p weather-service
curl localhost:9400/api/v1/bodies
curl "localhost:9400/api/v1/SandBox/point?lon=-39.5&lat=24.7"
curl "localhost:9400/api/v1/SandBox/fields?layer=pressure&w=90&h=45"
```

Docker (context = `services/`): `docker build -f weather/docker/Dockerfile -t weather:dev .`

## Environment

| Variable | Default | Meaning |
|---|---|---|
| `PORT` | 9400 | one port for ws + http |
| `SYSTEM_JSON` | `data/system/tarsis.json` | celestial data (resourcesDynamic's file) |
| `CLIMATE_JSON` | `data/climate/tarsis.json` | weather knobs per body |
| `CELESTIAL_HTTP` | `http://service-resourcesdynamic:3001` | to resolve the planets' Horizon uuids |
| `TICK_S` | 30 | publish period |
| `KEYFRAME_DT_S`, `KEYFRAMES` | 600, 6 | the future positions each system carries |
| `ATMOSPHERE_BODIES` | *(empty = off)* | bodies that run the atmosphere model, e.g. `Sandbox` or `Sandbox,Gaea` |
| `ATMOSPHERE_DATA_DIR` | `data/atmosphere` | its settings files and rasters |
| `DATABASE_URL` | *(unset)* | PostgreSQL for its checkpoints (`postgres://user:pass@host:5432/db`); wins over the folder |
| `ATMOSPHERE_STATE_DIR` | *(unset)* | without `DATABASE_URL`: checkpoints as files, one folder per body (persistent volume) |
| `ATMOSPHERE_THREADS` | 2 | threads per body (1 to 4) |
| `ATMOSPHERE_CHECKPOINT_S` | 300 | a checkpoint at most every N wall-clock seconds |
| `ATMOSPHERE_MAX_CATCHUP_DAYS` | 30 | after a stop, catch up at most N model days (older time is skipped) |
| `ATMOSPHERE_SPINUP_DAYS` | *(settings: 250)* | spin-up of a cold start, model days; short values for smoke tests only |
| `ATMOSPHERE_SET` | *(empty)* | settings overrides, `key=value;key=value` (e.g. `grid.level=4`) |

## The atmosphere model (stateful, off by default)

With `ATMOSPHERE_BODIES` empty, the service is exactly the pure (data, seed, time) service above.
When a body is listed, the service also keeps that body's atmosphere **in memory** (a dedicated
thread per body). The atmosphere is chaotic, so this part is no longer a pure function of time: it
is a state plus checkpoints. The synoptic systems, their ds_bridge publication and every endpoint
above are unchanged (the model is not published to the game yet).

1. At start: the newest healthy checkpoint is loaded; without one, a **cold start** runs the spin-up
   (250 model days by default, about an hour for Sandbox on 2 threads) as fast as it can — status
   `warming`.
2. Then the model follows the wall clock: one model day = the body's solar day (Sandbox: 90 551.5 s).
   After a stop it catches up in accelerated time (status `catching_up`). It advances by chunks of
   1/24 model day (~1 h for Sandbox), so the tables change once per chunk.
3. After each chunk the guard accepted (finite values, pressure and wind bounds), the tables are
   rendered into memory; at most every `ATMOSPHERE_CHECKPOINT_S` a checkpoint is saved into one of
   two rolling slots. A guard failure rolls back to the last model-day boundary with a smaller time
   step; the third failure in a row stops the model (status `failed`) until the next restart.

Endpoints:

- `GET /api/v1/{body}/atmosphere` — status (`warming`, `catching_up`, `running`, `failed`), model
  time and day, grid, last guard events, last checkpoint, and the list of tables with their range
  (404 when the model is not enabled for that body).
- `GET /api/v1/{body}/atmosphere/tables/{name}.png` — a table: 16-bit grayscale PNG, equirectangular
  (longitude −180 → 180, latitude 90 → −90), `value = min + (max − min) × pixel / 65535`;
  `{name}.json` gives `min`, `max`, units and model time. Names: `pmer`, `ps`, `u_bas`, `v_bas`,
  `t_bas`, `u_mil`, `v_mil`, `t_mil`, `poussiere`, and the physics "ingredients" (`flux_chaleur`,
  `couche_limite`, `tension_vent`, `soulevement`, `couvercle`, `t_sol`…).
- `/metrics`: `weather_atmosphere_model_day`, `weather_atmosphere_max_wind_m_s`,
  `weather_atmosphere_lag_s`, `weather_atmosphere_rollbacks_total` (label `body`).

**State storage.** Checkpoints go through a `StateStore` trait (`weather-atmosphere/src/live.rs`),
two rolling slots per body, each state 1 to 10 MB. The store is chosen at start:

1. `DATABASE_URL` set → **PostgreSQL** (CNPG in the cluster, like the other services; sqlx with
   rustls). The embedded migration (`crates/weather-service/migrations/`) creates the table at
   start; the connection is retried for 2.5 minutes while the database comes up.
2. else `ATMOSPHERE_STATE_DIR` set → **files**, `<dir>/<body>/checkpoint_<a|b>.bin` + `.toml`
   (a persistent volume in a cluster).
3. neither, with `ATMOSPHERE_BODIES` set → the atmosphere mode **does not start** (status
   `failed` with the reason on `/api/v1/{body}/atmosphere`); the rest of the service runs.

```sql
CREATE TABLE atmosphere_checkpoint (
    body        TEXT             NOT NULL,          -- body name in lower case
    slot        SMALLINT         NOT NULL,          -- rolling slot, 0 or 1
    model_time  DOUBLE PRECISION NOT NULL,          -- model time of the state (s)
    healthy     BOOLEAN          NOT NULL,          -- accepted by the guard (only these are loaded)
    saved_at    TIMESTAMPTZ      NOT NULL DEFAULT now(),
    meta        JSONB            NOT NULL,          -- chunk, spin-up done, pacing anchor
    state       BYTEA            NOT NULL,          -- the full state, bit-exact
    PRIMARY KEY (body, slot)
);
```

**A lost state is not serious**: without a checkpoint (empty table, database reset, files lost),
the model simply makes a cold start and spins up again (about an hour for Sandbox), while the rest
of the service keeps serving. Players would only see the atmosphere start over from a new state.

**Known limits.** The model's seasons and sun start at its own t = 0 (northern spring equinox), not
at the body's real orbital position (TODO in `crates/weather-service/src/atmosphere.rs`). Restart
files are bit-exact on one platform; the math library (sin, exp, pow…) differs between platforms,
so a checkpoint resumed on another OS continues a slightly different (equally valid) weather.

Quick try (from `services/weather`, a short spin-up only for a test):

```
ATMOSPHERE_BODIES=Sandbox ATMOSPHERE_SPINUP_DAYS=0.5 ATMOSPHERE_STATE_DIR=/tmp/atmo \
SYSTEM_JSON=../resourcesDynamic/data/system/tarsis.json cargo run -p weather-service
curl localhost:9400/api/v1/Sandbox/atmosphere
curl -o t_bas.png localhost:9400/api/v1/Sandbox/atmosphere/tables/t_bas.png
```

Tests: `cargo test --workspace` runs the fast ones (Williamson cases, conservation, restarts bit for
bit, guard, resting atmosphere over the relief, settings, checkpoints). Slow ones are `#[ignore]`d:
`cargo test -p weather-atmosphere -- --ignored`. The PostgreSQL store's test needs a database:
`docker run --rm -d -p 55432:5432 -e POSTGRES_PASSWORD=x postgres:16`, then
`TEST_DATABASE_URL=postgres://postgres:x@localhost:55432/postgres cargo test -p weather-service -- --ignored`. The bit-for-bit regression against the prototype
runs only with the MSVC toolchain on Windows (its fingerprints come from that math library).
