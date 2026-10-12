# 0006. Weighted warehouse route graph, Dijkstra, per-hop legs

- **Status:** Proposed
- **Date:** 2026-10-11
- **Deciders:** David Durieux
- **Scope:** universesimulator

## Context

To supply a remote mining village from a city, goods go through intermediate towns. A single
long mission is hard for a player or an NPC; several short hops are easier and let different
carriers take each part.

## Options considered

1. **Hierarchy only**: route = up to the lowest common ancestor, then down. Simple but ignores
   real roads and shortcuts.
2. **Weighted graph** of warehouses (`travel_routes`, with distance and cost), shortest path
   with Dijkstra.

## Decision

We will store a weighted, optionally bidirectional route graph between warehouses and compute
paths with a hand-written Dijkstra (binary heap) over the active warehouses of a federation.
A supply order follows the path and is split into one leg per hop. Supplier selection runs one
Dijkstra from the destination and picks the closest warehouse with enough surplus.

## Consequences

Routes must be entered (import or admin). An unreachable warehouse cannot be supplied and is
reported. The graph is loaded per tick; it is small (hundreds of nodes).
