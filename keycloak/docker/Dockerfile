# Custom Keycloak image for StarDeception.
#
# - Bakes in the realm export (imported on start via `--import-realm`).
# - Bakes in the Discord IdP bootstrap script invoked by the post-install Job.
# - Pre-builds Keycloak with `kc.sh build --db=postgres` for fast container starts.
#
# Pinned to a specific upstream release; bump together with the chart's
# appVersion in ../../kubernetes/keycloak/Chart.yaml.

ARG KEYCLOAK_VERSION=26.6

FROM quay.io/keycloak/keycloak:${KEYCLOAK_VERSION} AS builder

ENV KC_DB=postgres \
    KC_HEALTH_ENABLED=true \
    KC_METRICS_ENABLED=true \
    KC_HTTP_RELATIVE_PATH=/

# Pre-build the optimized server distribution.
RUN /opt/keycloak/bin/kc.sh build

FROM quay.io/keycloak/keycloak:${KEYCLOAK_VERSION}

# Copy the optimized distribution from the builder stage.
COPY --from=builder /opt/keycloak/ /opt/keycloak/

# Realm export imported on every start (idempotent with `--import-realm`).
COPY --chown=1000:0 realm/dyingstar-realm.json /opt/keycloak/data/import/dyingstar-realm.json

# Bootstrap script invoked by the Helm post-install Job to register / update
# the Discord identity provider through kcadm.sh.
COPY --chown=1000:0 --chmod=0755 scripts/bootstrap-discord-idp.sh /opt/keycloak/scripts/bootstrap-discord-idp.sh

USER 1000

ENTRYPOINT ["/opt/keycloak/bin/kc.sh"]
CMD ["start", "--optimized", "--import-realm"]
