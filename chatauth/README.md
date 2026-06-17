# chat-auth

Tiny auth adapter for the **text-chat MQTT broker**.

The broker runs `mosquitto-go-auth` in JWT *remote* mode. For every connection and
publish/subscribe, go-auth calls this service with the client's Keycloak access token
(sent as the MQTT password, forwarded here as `Authorization: Bearer <token>`). We:

- **validate** the token's RS256 signature against Keycloak's JWKS (`OIDC_JWKS_URL`),
  checking the issuer — the broker never needs the signing key, and key rotation is
  handled automatically;
- **authorize** topics. For now any authenticated player may use the chat topics
  (`CHAT_ALLOWED_TOPICS`, default `chat/#`). This will tighten per JWT claims
  (group/alliance) once those gameplay systems exist.

## Endpoints (go-auth contract, response mode `json`)

| Method | Path | Meaning |
|---|---|---|
| POST | `/user` | token valid? → `{"Ok":bool}` |
| POST | `/superuser` | always `{"Ok":false}` |
| POST | `/acl` | token valid **and** topic allowed? → `{"Ok":bool}` |
| GET | `/health` | liveness |

## Configuration (env)

| Var | Default | Notes |
|---|---|---|
| `PORT` | `3000` | listen port |
| `OIDC_ISSUER` | `http://keycloak:8080/realms/dyingstar` | expected `iss` |
| `OIDC_JWKS_URL` | `${OIDC_ISSUER}/protocol/openid-connect/certs` | Keycloak public keys |
| `CHAT_ALLOWED_TOPICS` | `chat/#` | comma-separated MQTT topic filters |
