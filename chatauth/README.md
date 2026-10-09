# chat-auth

Auth adapter for the **text-chat MQTT broker**.

The broker runs `mosquitto-go-auth` in JWT *remote* mode. For every connection and
publish/subscribe, go-auth calls this service with the client's Keycloak access token
(sent as the MQTT password, forwarded here as `Authorization: Bearer <token>`). We:

- **validate** the token's RS256 signature against Keycloak's JWKS (`OIDC_JWKS_URL`),
  checking the issuer — the broker never needs the signing key, and key rotation is
  handled automatically;
- **authorize** per channel: the global chat is open to every authenticated player,
  while DMs, group chats and corporation chats require membership checked against the
  **Social** service internal API. Active mutes block publishing (reading stays open);
  active suspensions/bans block connecting.

## Topic layout

| Topic | Who may publish | Who may subscribe |
|---|---|---|
| `chat/global` | any authenticated player (not muted) | any authenticated player |
| `chat/dm/{fromId}/{toId}` | `fromId` (the verified JWT `sub`), if the pair is friends and neither is muted | the recipient only: `toId` = own id (canonical filter `chat/dm/+/{myId}`) |
| `chat/group/{groupId}` | members of the group | members of the group |
| `chat/corporation/{corporationId}` | corporation members | corporation members |

Notes:

- The client should subscribe to **all** its DMs with one filter: `chat/dm/+/{myId}`,
  then publish to `chat/dm/{me}/{recipientId}`.
- Wildcards (`+`/`#`) are rejected outside that canonical DM filter; group and
  corporation topics are concrete UUIDs only.
- Scoped channels are decided by their own rules: widening `CHAT_ALLOWED_TOPICS`
  (even to `chat/#`) can never re-open DMs or group/corporation chats.
- Membership probes fail **closed** (Social down ⇒ private channels refuse, global
  chat keeps working). Mute checks fail open (a Social outage never blocks delivery).
- Membership results are cached in memory for `CHAT_ACL_CACHE_TTL_SECONDS` (30 s).

## Endpoints (go-auth contract, response mode `json`)

| Method | Path | Meaning |
|---|---|---|
| POST | `/user` | token valid **and** player not suspended/banned? → `{"Ok":bool}` |
| POST | `/superuser` | always `{"Ok":false}` |
| POST | `/acl` | token valid **and** topic allowed for this player? → `{"Ok":bool}` |
| GET | `/health` | liveness, includes `social: bool` (configured?) |

## Social integration

`CHAT_SOCIAL_API_URL` + a Keycloak service-account secret enable the membership
probes. The client (`svc-chat` by default) calls Social's internal API with these
realm roles (see `social/src/middleware/auth.ts`):

- `social:profile:read` — `GET /api/internal/players/{id}/friendship/{otherId}`
- `social:group:read` — `GET /api/internal/players/{id}/group?groupId=`
- `social:corporation:read` — `GET /api/internal/players/{id}/corporation?corporationId=`
- `social:sanctions:read` — `GET /api/internal/players/{id}/sanctions`

The client must also be listed in Social's `INTERNAL_SERVICE_CLIENTS`.

Without Social configured, only the legacy allowlist topics work; DM/group/
corporation channels are refused.

## Configuration (env)

| Var | Default | Notes |
|---|---|---|
| `PORT` | `3000` | listen port |
| `OIDC_ISSUER` | `http://keycloak:8080/realms/dyingstar` | expected `iss` |
| `OIDC_JWKS_URL` | `${OIDC_ISSUER}/protocol/openid-connect/certs` | Keycloak public keys |
| `OIDC_TOKEN_URL` | `${OIDC_ISSUER}/protocol/openid-connect/token` | for Social client_credentials |
| `CHAT_TOPIC_ROOT` | `chat` | first topic segment |
| `CHAT_ALLOWED_TOPICS` | `chat/global,chat/general` | legacy allowlist for non-scoped topics |
| `CHAT_SOCIAL_API_URL` | *(empty = off)* | Social base URL, e.g. `http://social:3000` |
| `CHAT_SOCIAL_SERVICE_CLIENT_ID` | `svc-chat` | Keycloak service client |
| `CHAT_SOCIAL_SERVICE_CLIENT_SECRET` | *(empty)* | its secret |
| `CHAT_ACL_CACHE_TTL_SECONDS` | `30` | membership/mute cache TTL |
| `CHAT_ACL_CACHE_MAX` | `10000` | cache entry cap |

## Development

```sh
pnpm install
pnpm test        # unit tests (node:test) on the ACL rules and the cache
pnpm start
```
