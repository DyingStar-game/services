# Architecture Decision Records

This folder records the significant decisions taken on the `universesimulator` service,
using lightweight ADRs ([Michael Nygard format](https://cognitect.com/blog/2011/11/15/documenting-architecture-decisions)),
with the same conventions as the admin panel (`adminpanel/docs/adr`).

## Conventions

- One file per decision: `NNNN-short-title-in-kebab-case.md` (4-digit sequence, never reused).
- Start from [`template.md`](./template.md).
- Statuses: `Proposed` → `Accepted` | `Rejected`; later `Deprecated` or `Superseded by NNNN`.
- An accepted ADR is not rewritten: write a new ADR that supersedes it.
- An accepted ADR is binding. Reread the ADRs of an area before working in it.
- Written in English, like the rest of the contributor documentation.

## Index

| # | Title | Status | Date |
|---|-------|--------|------|
| [0001](./0001-record-architecture-decisions.md) | Record architecture decisions | Proposed | 2026-10-11 |
| [0002](./0002-rust-axum-sqlx-postgresql.md) | Rust (axum + sqlx) and PostgreSQL, orchestrator as template | Proposed | 2026-10-11 |
| [0003](./0003-orchestrator-of-existing-services.md) | universesimulator orchestrates; data ownership split | Proposed | 2026-10-11 |
| [0004](./0004-warehouses-are-pois-and-inventory-holders.md) | Warehouses are persistence POIs and inventory holders | Proposed | 2026-10-11 |
| [0005](./0005-political-hierarchy-and-federations.md) | Political hierarchy from social; federations govern supply | Proposed | 2026-10-11 |
| [0006](./0006-route-graph-and-per-hop-legs.md) | Weighted warehouse route graph, Dijkstra, per-hop legs | Proposed | 2026-10-11 |
| [0007](./0007-supply-orders-published-to-mission.md) | Supply orders published to the mission service | Proposed | 2026-10-11 |
| [0008](./0008-pricing-distance-supply-demand.md) | Pricing: supply/demand index, local urgency, distance freight | Proposed | 2026-10-11 |
| [0009](./0009-treasury-escrow-and-monetary-policy.md) | Rewards escrowed from the federation treasury; minting | Proposed | 2026-10-11 |
| [0010](./0010-simulation-tick.md) | Simulation tick: time-scaled flows, single writer | Proposed | 2026-10-11 |
| [0011](./0011-keycloak-auth-in-rust.md) | Keycloak authentication in Rust | Proposed | 2026-10-11 |
| [0012](./0012-off-mission-warehouse-deliveries.md) | Off-mission warehouse deliveries from the game server | Proposed | 2026-10-11 |
| [0013](./0013-villages-aggregate-their-warehouses.md) | Villages are the simulation nodes; they aggregate their warehouses | Proposed | 2026-10-12 |
