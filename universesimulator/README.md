# universesimulator

Simulates the physical economy of the universe: stocks of water, food and ores in the
warehouses of villages and cities, their production and consumption, the supply missions
that move goods hop by hop, mining missions, prices driven by distance and by supply and
demand, and the federation treasury that pays the missions.

The service **orchestrates** the existing game services (see [ADR 0003](docs/adr/0003-orchestrator-of-existing-services.md)):

| Concern | Owner |
|---|---|
| Political hierarchy (commune → agglomeration → … → federation) | social |
| Stocks (holder type `warehouse`, id = cargo_depot UUID) | inventory |
| Villages (`poi_village`) and warehouses (`cargo_depot`, linked by `poi_uuid`) | persistence |
| Missions (escrowed on the federation treasury) | mission |
| Treasury and money issuance | economie |
| Route graph, village policies and flows, warehouse capacities, mineral sources, supply orders, prices, tick | **universesimulator** (PostgreSQL) |

The first federation, **ARES**, governs planet `tarsis_3` (SandBox). Decisions are recorded in
[`docs/adr`](docs/adr/README.md).

## Villages and warehouses (ADR 0013)

A **village** is a persistence `poi_village` (e.g. `M0002`, `F0011`) and a commune in social.
Its **warehouses** are the `cargo_depot` objects whose object_data field **`poi_uuid`** holds the
village's UUID; `mining_depot` objects are ignored. The stock of a village is the sum of its
warehouses. Thresholds, flows, routes and mineral sources are per village; capacities per
warehouse. Deliveries and production go to the village's main warehouse.

## How it works

Every `SIM_TICK_SECONDS` (one replica at a time, PostgreSQL advisory lock), per federation:

1. read the warehouse stocks from inventory (and, periodically, refresh the political cache and
   attach newly linked cargo_depots);
2. apply village flows (`ratePerHour × elapsed game hours`): production to the main warehouse,
   consumption from the most stocked warehouses;
3. reconcile published missions: a completed transport leg consumes its inventory hold into
   the next warehouse and starts the next leg; a cancelled/expired mission is republished, up
   to `SIM_LEG_MAX_ATTEMPTS`;
4. recompute prices: federation index from supply and demand, local buy/sell prices;
5. plan: a village below its `minThreshold` gets a **mining order** when it has a mineral
   source, otherwise a **transport order** from the nearest village with a surplus
   (Dijkstra on the route graph), split into one **leg per hop**; without any surplus, mining
   is ordered in the nearest village holding a source;
6. publish pending legs: inventory hold on the origin, reward priced from distance, cargo value
   and urgency, mission created in the mission service, issued and **escrowed by the
   federation** (`scriptId = usim:leg:<leg>:<attempt>`);
7. treasury: mint `mintBatch` when an escrow was refused or the treasury is below its floor;
8. snapshot stocks into the history.

## API

Contract: [`openapi.yaml`](openapi.yaml) (also served at `GET /openapi.yaml`).

| Prefix | Auth | Content |
|---|---|---|
| `/healthz` | none | health |
| `/api/*` | player token | federations, entities, resources, prices, villages (aggregated stocks, warehouses), warehouses (live stocks), entity stock, paths, orders |
| `/api/internal/*` | service account, `aud=universesimulator-api` | same reads (`usim:read`), `GET /warehouses/by-poi/{uuid}`, `POST /warehouse-deliveries` (`usim:deliver`), `POST /orders` (`usim:write`) |
| `/api/admin/*` | moderation role (writes: `admin`/`supervisor`) | dashboard, network, histories, local prices, configuration (villages, village policies, flows, warehouse capacities, routes, mineral sources), orders, simulation control, `POST /import` |

### Off-mission deliveries (game server)

```http
POST /api/internal/warehouse-deliveries
{ "idempotencyKey": "drop-…", "warehouse": { "poiUuid": "…" }, "resource": { "code": "ore_fer" },
  "quantity": 500, "deliverer": { "type": "player", "id": "…" } }
→ 201 { "acceptedQuantity": 500, "rejectedQuantity": 0, "stock": 1500, "delivery": { … } }
```

The accepted quantity (capped by the warehouse capacity) is transferred from the player to the
warehouse in inventory. No payment (ADR 0012). Replays with the same key return `200`.

## Service roles

Inbound: `usim:read`, `usim:write`, `usim:deliver` (realm roles, audience `universesimulator-api`).
`svc-game` needs `usim:read` and `usim:deliver`.

Outbound, client `svc-universesimulator` (client credentials) needs:

| Service (audience) | Roles |
|---|---|
| mission (`mission-api`) | `mission:read`, `mission:write`, `mission:escrow:politics` |
| inventory (`inventory-api`) | `inventory:read`, `inventory:hold`, `inventory:transfer`, `inventory:credit` |
| social (`social-api`) | `social:politics:read`, `social:politics:write` |
| economie (`economie-api`) | `economie:wallet:ensure`, `economie:politics:read`, `economie:politics:manage`, `economie:money:issue` |

and must be listed in `INTERNAL_SERVICE_CLIENTS` of these four services.

## Local development

```bash
cp .env.example .env          # EXTERNAL_MODE=dry_run: no other service needed
make db-up                    # PostgreSQL 17 on localhost:5434
cargo run
make import-seed              # ARES world on tarsis_3 (seeds/tarsis_3-ares.json)
make tick                     # force a tick
```

`EXTERNAL_MODE=dry_run` replaces social, inventory, mission, economie and persistence with an
in-memory stand-in (state is lost on restart). Use `EXTERNAL_MODE=live` with the service URLs and
`SERVICE_CLIENT_SECRET` (or `OUTBOUND_INTERNAL_API_KEY` with the services' dev bypass) to run
against the real services.

`seeds/tarsis_3-ares.json` imports every village of SandBox (`planetPoiUuid`) that has a
cargo_depot attached through `poi_uuid`, with profiles per village type (`mining`, `factory`)
scaled by population (profile chosen by the village `type`, else by its name prefix: `M` →
mining, `F` → factory). The import report lists the depots without `poi_uuid`. The import is
kept and replayed by the tick (every 15 min and on each forced tick): a village newly linked
to a cargo_depot appears without a new import.

## Tests

```bash
make db-up
make test        # DATABASE_URL=postgres://…:5434/… cargo test
```

Unit tests cover the graph, pricing, flows, planner and auth; integration tests
(`#[sqlx::test]`, one database per test) run full ticks against the in-memory services:
hop-by-hop transport, mining, retries and failure, minting, flows and shortages, prices,
HTTP auth layers and deliveries.
