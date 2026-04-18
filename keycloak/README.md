# StarDeception Keycloak

Custom Keycloak distribution for the StarDeception platform.

This repository owns:
- the **Docker image** published to `harbor.dyingstar-game.space/dyingstar/keycloak`,
- the **realm definition** (`realm/dyingstar-realm.json`) imported on every Keycloak start,
- the **Discord IdP bootstrap script** (`scripts/bootstrap-discord-idp.sh`) executed by the Helm post-install Job in the [kubernetes](../../kubernetes/keycloak) repo.

The Helm chart that consumes this image lives in `../../kubernetes/keycloak/`.

## Layout

```
docker/Dockerfile                      # FROM quay.io/keycloak/keycloak:<pinned>
realm/dyingstar-realm.json             # Imported by `start --import-realm`
scripts/bootstrap-discord-idp.sh       # kcadm.sh-based, idempotent
.github/workflows/build-and-deploy.yaml
```

## Build locally

```bash
docker build -t keycloak:dev -f docker/Dockerfile .
```

## Realm export workflow

When admins make changes through the Keycloak UI in prod/preprod, they drift
from `realm/dyingstar-realm.json`. To re-sync:

```bash
kubectl -n dyingstar-prod exec deploy/keycloak -- \
  /opt/keycloak/bin/kc.sh export --dir /tmp/export --realm dyingstar
kubectl -n dyingstar-prod cp keycloak-<pod>:/tmp/export/dyingstar-realm.json \
  realm/dyingstar-realm.json
```

Review the diff carefully before committing — the Discord IdP block should
stay empty in git (secrets are never exported into the realm JSON).

## Discord setup

1. Create an OAuth2 application on https://discord.com/developers/applications
2. Add the redirect URI for each environment:
   - prod:    `https://auth.dyingstar-game.com/realms/dyingstar/broker/discord/endpoint`
   - preprod: `https://auth-preprod.dyingstar-game.com/realms/dyingstar/broker/discord/endpoint`
   - local:   `http://<minikube-ip>:30180/realms/dyingstar/broker/discord/endpoint`
3. Store the client_id / client_secret in the cluster Secret `keycloak-discord`
   in each target namespace (or inline them in `values-dev-local.yaml` for local dev).

## Required GitHub Secrets

| Secret | Purpose |
|--------|---------|
| `HARBOR_USERNAME` / `HARBOR_PASSWORD` | Push image to Harbor |
| `KUBERNETES_REPO_TOKEN` | Fine-grained PAT to trigger `repository_dispatch` on the kubernetes repo |
