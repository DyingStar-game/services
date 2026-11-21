# 0002. Rust (axum + sqlx) and PostgreSQL, orchestrator as template

- **Status:** Proposed
- **Date:** 2026-10-11
- **Deciders:** David Durieux
- **Scope:** universesimulator

## Context

The simulator runs a periodic tick, computes shortest paths and prices, and exposes a REST API.
The team asked for Rust, like `persistence`. `persistence` uses ScyllaDB, which suits
key-value items but not the relational, transactional data of the simulator (orders, legs,
policies, history). `orchestrator` is the only Rust service on PostgreSQL.

## Options considered

1. **Copy `persistence`** (axum, ScyllaDB). Same language, wrong storage model.
2. **Copy `orchestrator`** (axum 0.8, sqlx 0.8, PostgreSQL, embedded SQL migrations).
3. **TypeScript + Drizzle** like the other game services. Rejected: Rust was requested.

## Decision

We will build `universesimulator` in Rust with axum 0.8, sqlx 0.8 (runtime-checked
`query_as`, no compile-time `query!` so the build does not need a database) and PostgreSQL 17.
Migrations are plain SQL files in `migrations/`, embedded with `sqlx::migrate!` and run at
start-up after a connection retry loop, exactly like `orchestrator/src/db/connection.rs`.
The service layout, error type, config loading, Dockerfile, compose file and Makefile follow
`orchestrator`. The REST contract is a hand-written `openapi.yaml`, served at `/openapi.yaml`.

## Consequences

No init container is needed for migrations. Integration tests use `#[sqlx::test]`, which needs
a reachable PostgreSQL (`DATABASE_URL`). Outbound calls use reqwest with rustls.
