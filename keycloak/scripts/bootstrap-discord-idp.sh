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
    "defaultScope": "identify email openid",
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

# ── Disable "Review Profile" in the first broker login flow ──────────────────
# This prevents the "Update Account Information" popup — username and email
# are imported automatically from Discord via the mappers below.
echo "Disabling Review Profile in 'first broker login' flow..."
REVIEW_EXEC_ID=""
while IFS=',' read -r eid edisplay eprovider; do
  if [[ "${eprovider}" == "idp-review-profile" ]]; then
    REVIEW_EXEC_ID="${eid}"
    break
  fi
done < <("${KCADM}" get "authentication/flows/first%20broker%20login/executions" \
            -r "${KC_REALM}" --fields id,displayName,providerId --format csv --noquotes 2>/dev/null || true)

if [[ -n "${REVIEW_EXEC_ID}" ]]; then
  "${KCADM}" update "authentication/flows/first%20broker%20login/executions" \
    -r "${KC_REALM}" \
    -b "{\"id\": \"${REVIEW_EXEC_ID}\", \"requirement\": \"DISABLED\"}"
  echo "Review Profile execution disabled."
else
  echo "WARNING: Could not find Review Profile execution in 'first broker login' flow."
fi

# ── Relax the realm user profile ─────────────────────────────────────────────
# Discord doesn't provide firstName / lastName, so we drop them from the
# required attributes. Only username and email remain mandatory.
echo "Updating realm user profile (making firstName/lastName optional)..."
USER_PROFILE_PAYLOAD=$(cat <<'EOF'
{
  "attributes": [
    {
      "name": "username",
      "displayName": "${username}",
      "validations": {
        "length": { "min": 3, "max": 255 },
        "username-prohibited-characters": {},
        "up-username-not-idn-homograph": {}
      },
      "permissions": { "view": ["admin", "user"], "edit": ["admin", "user"] },
      "multivalued": false
    },
    {
      "name": "email",
      "displayName": "${email}",
      "validations": {
        "email": {},
        "length": { "max": 255 }
      },
      "required": { "roles": ["user"] },
      "permissions": { "view": ["admin", "user"], "edit": ["admin", "user"] },
      "multivalued": false
    },
    {
      "name": "firstName",
      "displayName": "${firstName}",
      "validations": {
        "length": { "max": 255 },
        "person-name-prohibited-characters": {}
      },
      "permissions": { "view": ["admin", "user"], "edit": ["admin", "user"] },
      "multivalued": false
    },
    {
      "name": "lastName",
      "displayName": "${lastName}",
      "validations": {
        "length": { "max": 255 },
        "person-name-prohibited-characters": {}
      },
      "permissions": { "view": ["admin", "user"], "edit": ["admin", "user"] },
      "multivalued": false
    }
  ],
  "groups": [
    {
      "name": "user-metadata",
      "displayHeader": "User metadata",
      "displayDescription": "Attributes, which refer to user metadata"
    }
  ]
}
EOF
)
echo "${USER_PROFILE_PAYLOAD}" | "${KCADM}" update "users/profile" -r "${KC_REALM}" -f -
echo "Realm user profile updated."

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
