# 0001. Record architecture decisions

- **Status:** Proposed
- **Date:** 2026-10-11
- **Deciders:** David Durieux
- **Scope:** universesimulator

## Context

`universesimulator` is a new service that drives the physical economy of the game across
several existing services (social, inventory, persistence, mission, economie). Its decisions
cut across service boundaries and must stay discoverable for the people who work on those
services.

## Options considered

1. **No written record** — decisions live in pull requests. Cheap, but lost quickly.
2. **ADRs in the service folder**, same format as the admin panel. Small, versioned with the code.

## Decision

We will record significant decisions as ADRs in `universesimulator/docs/adr`, using the
admin panel conventions (Nygard format, English, 4-digit numbering, index in `README.md`).
Decisions that change another service are recorded here and referenced from that service's
README.

## Consequences

Every cross-service change made for the simulator has a written rationale. ADRs must be kept
in sync with the index.
