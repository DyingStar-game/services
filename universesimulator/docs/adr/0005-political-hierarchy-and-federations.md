# 0005. Political hierarchy from social; federations govern supply

- **Status:** Proposed
- **Date:** 2026-10-11
- **Deciders:** David Durieux
- **Scope:** universesimulator, social
- **Builds on:** adminpanel ADR 0024

## Context

Villages and cities already exist as political entities in social (`commune` → `agglomeration`
→ … → `federation`). The first federation, ARES, controls planet `tarsis_3` (SandBox): it issues
money, manages ores and food needs, and pays the missions. Social exposed hierarchy reads and
parent changes only through player routes, and internal creation required a head player.

## Options considered

1. **Own hierarchy in the simulator** (a `parent_id` on locations). Duplicates social.
2. **Use social**, with the missing internal routes added.

## Decision

We will read and write the hierarchy in social. Social gets internal routes
`GET /politics/:id`, `GET /politics` (filters `type`, `parentId`, `name`),
`GET /politics/:id/children`, `GET /politics/:id/ancestors` and `PUT /politics/:id/parent`,
and internal `POST /politics` accepts no `headId` (a system-governed entity such as ARES).
The simulator keeps a `political_cache` mirror, refreshed at import and periodically.
A `federations` table holds, per federation entity, the governed planet, the managed resource
categories (`ore`, `food`, `water`), the monetary policy and the pricing parameters.
The world (ARES, its cities and villages, warehouses, routes) is bootstrapped by an idempotent
import (`POST /api/admin/import`), matching social entities by name.

## Consequences

Political entity names are unique in social, so the import can find existing entities.
Planning is done per federation: a federation never ships goods to another federation's
warehouses.
