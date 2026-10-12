# 0010. Simulation tick: time-scaled flows, single writer

- **Status:** Proposed
- **Date:** 2026-10-11
- **Deciders:** David Durieux
- **Scope:** universesimulator

## Context

Stocks change over time (population consumes food and water, farms and wells produce), and the
simulator must react to shortages. The service may run with several replicas.

## Options considered

1. **Event-driven only**. Nothing would consume stocks between events.
2. **Periodic tick** with a single writer.

## Decision

We will run a tick every `SIM_TICK_SECONDS` (default 300). A tick runs only on the replica that
gets `pg_try_advisory_lock`, and does nothing while paused. Steps: refresh caches and read
stocks, apply flows (`rate_per_hour × elapsed hours × SIM_TIME_SCALE`, integer part credited or
debited in inventory, fraction carried), recompute prices, reconcile missions, plan new orders,
publish pending legs, apply monetary policy, snapshot stocks into `stock_history` (kept 14 days).
Admins can pause, resume or force a tick.

## Consequences

Inventory credit/debit has no idempotency key: a crash between the call and the carry update
can apply a flow twice. This is acceptable for consumption simulation. Shortages (debit limited
to the available stock) are logged.
