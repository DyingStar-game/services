# 0011. Keycloak authentication in Rust

- **Status:** Proposed
- **Date:** 2026-10-11
- **Deciders:** David Durieux
- **Scope:** universesimulator, kubernetes/keycloak-managed

## Context

The TypeScript services authenticate players with Keycloak JWTs and services with service
account tokens (`aud`, `azp` allow-list, `service-account-*`, capability roles), with dev
bypass headers. No Rust service authenticates yet.

## Options considered

1. **No authentication**, like the other Rust services. Not acceptable: the API moves goods.
2. **Port the TypeScript middleware** to axum extractors.

## Decision

We will verify RS256 JWTs with `jsonwebtoken` against the realm JWKS (cached, reloaded on an
unknown `kid`), checking `iss` and `aud`. Extractors: `Player` (`sub` is a UUID), `Admin`
(`admin` or `supervisor` realm role) and `Service` (`aud = universesimulator-api`, `azp` in
`INTERNAL_SERVICE_CLIENTS`, `preferred_username` starting with `service-account-`, roles from
the realm and `resource_access[aud]`). Capability roles: `usim:read`, `usim:write`,
`usim:deliver`. Dev bypasses (`X-Player-Id`, `X-Internal-Key`) are refused in production.

Outbound, the simulator uses the client `svc-universesimulator` (client credentials, token cached
until 30 s before expiry, `X-Internal-Key` fallback in dev) with the roles `mission:read`,
`mission:write`, `mission:escrow:politics`, `inventory:read`, `inventory:hold`,
`inventory:transfer`, `inventory:credit`, `social:politics:read`, `social:politics:write`,
`economie:wallet:ensure`, `economie:politics:read`, `economie:politics:manage`,
`economie:money:issue`. `svc-game` gets `usim:read` and `usim:deliver`.

## Consequences

Keycloak realm imports, service client declarations and the role bootstrap job must be updated
(kubernetes repository), and `svc-universesimulator` added to the internal client allow-lists of
mission, inventory, social and economie.
