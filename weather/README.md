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
- `data/climate/<system>.json` — the knobs the celestial data does not carry (seed, dust grains, lift
  bias, thermal inertia, humidity, wind and dust caps), by body name.

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
