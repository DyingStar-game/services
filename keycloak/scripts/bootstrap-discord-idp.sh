#!/usr/bin/env bash
#
# bootstrap-discord-idp.sh — register or update the Discord identity provider
# on the configured Keycloak realm. Invoked by the Helm post-install/-upgrade
# Job defined in ../../kubernetes/keycloak/templates/job-bootstrap-discord.yaml
#
# Required environment:
#   KC_SERVER_URL              e.g. http://keycloak:8080/
#   KC_REALM                   e.g. dyingstar
#   KEYCLOAK_ADMIN             admin username (master realm)
#   KEYCLOAK_ADMIN_PASSWORD    admin password
#   DISCORD_CLIENT_ID          Discord OAuth2 client id
#   DISCORD_CLIENT_SECRET      Discord OAuth2 client secret
#
# The script is idempotent: it creates the IdP and mappers on first run and
# updates them on subsequent runs.

set -euo pipefail

: "${KC_SERVER_URL:?KC_SERVER_URL is required}"
: "${KC_REALM:?KC_REALM is required}"
: "${KEYCLOAK_ADMIN:?KEYCLOAK_ADMIN is required}"
: "${KEYCLOAK_ADMIN_PASSWORD:?KEYCLOAK_ADMIN_PASSWORD is required}"
: "${DISCORD_CLIENT_ID:?DISCORD_CLIENT_ID is required}"
: "${DISCORD_CLIENT_SECRET:?DISCORD_CLIENT_SECRET is required}"

KCADM=/opt/keycloak/bin/kcadm.sh
IDP_ALIAS="discord"

echo "Logging in to ${KC_SERVER_URL} as ${KEYCLOAK_ADMIN}..."
"${KCADM}" config credentials \
  --server "${KC_SERVER_URL%/}" \
  --realm master \
  --user "${KEYCLOAK_ADMIN}" \
  --password "${KEYCLOAK_ADMIN_PASSWORD}"

# ── Identity provider ────────────────────────────────────────────────────────
IDP_PAYLOAD=$(cat <<EOF
{
  "alias": "${IDP_ALIAS}",
  "displayName": "Discord",
  "providerId": "oidc",
  "enabled": true,
  "trustEmail": true,
  "storeToken": false,
  "addReadTokenRoleOnCreate": false,
  "linkOnly": false,
  "firstBrokerLoginFlowAlias": "first broker login",
  "config": {
    "clientId": "${DISCORD_CLIENT_ID}",
    "clientSecret": "${DISCORD_CLIENT_SECRET}",
    "clientAuthMethod": "client_secret_post",
    "authorizationUrl": "https://discord.com/api/oauth2/authorize",
    "tokenUrl": "https://discord.com/api/oauth2/token",
    "userInfoUrl": "https://discord.com/api/users/@me",
    "defaultScope": "identify email",
    "syncMode": "IMPORT",
    "useJwksUrl": "false",
    "validateSignature": "false",
    "backchannelSupported": "false",
    "loginHint": "false",
    "guiOrder": "1"
  }
}
EOF
)

if "${KCADM}" get "identity-provider/instances/${IDP_ALIAS}" -r "${KC_REALM}" >/dev/null 2>&1; then
  echo "Updating Discord IdP on realm ${KC_REALM}..."
  echo "${IDP_PAYLOAD}" | "${KCADM}" update "identity-provider/instances/${IDP_ALIAS}" -r "${KC_REALM}" -f -
else
  echo "Creating Discord IdP on realm ${KC_REALM}..."
  echo "${IDP_PAYLOAD}" | "${KCADM}" create "identity-provider/instances" -r "${KC_REALM}" -f -
fi

# ── Mappers (username, email, avatar) ────────────────────────────────────────
upsert_mapper() {
  local name="$1"
  local payload="$2"
  local existing_id=""
  # List existing mappers and find the one matching $name.
  # UBI-micro has no awk/grep — use pure bash string matching.
  while IFS=',' read -r mid mname; do
    if [[ "${mname}" == "${name}" ]]; then
      existing_id="${mid}"
      break
    fi
  done < <("${KCADM}" get "identity-provider/instances/${IDP_ALIAS}/mappers" \
              -r "${KC_REALM}" --fields id,name --format csv --noquotes 2>/dev/null || true)
  if [[ -n "${existing_id}" ]]; then
    echo "Updating mapper '${name}' (${existing_id})..."
    echo "${payload}" | "${KCADM}" update "identity-provider/instances/${IDP_ALIAS}/mappers/${existing_id}" -r "${KC_REALM}" -f -
  else
    echo "Creating mapper '${name}'..."
    echo "${payload}" | "${KCADM}" create "identity-provider/instances/${IDP_ALIAS}/mappers" -r "${KC_REALM}" -f -
  fi
}

upsert_mapper "discord-username" "$(cat <<EOF
{
  "name": "discord-username",
  "identityProviderAlias": "${IDP_ALIAS}",
  "identityProviderMapper": "oidc-username-idp-mapper",
  "config": {
    "syncMode": "INHERIT",
    "template": "\${CLAIM.username}"
  }
}
EOF
)"

upsert_mapper "discord-email" "$(cat <<EOF
{
  "name": "discord-email",
  "identityProviderAlias": "${IDP_ALIAS}",
  "identityProviderMapper": "oidc-user-attribute-idp-mapper",
  "config": {
    "syncMode": "INHERIT",
    "claim": "email",
    "user.attribute": "email"
  }
}
EOF
)"

upsert_mapper "discord-avatar" "$(cat <<EOF
{
  "name": "discord-avatar",
  "identityProviderAlias": "${IDP_ALIAS}",
  "identityProviderMapper": "oidc-user-attribute-idp-mapper",
  "config": {
    "syncMode": "INHERIT",
    "claim": "avatar",
    "user.attribute": "discord_avatar"
  }
}
EOF
)"

echo "discord IdP created/updated on realm ${KC_REALM}"
