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
# The script is idempotent: it creates the IdP on first run and updates it on
# subsequent runs. Username, email and avatar are mapped automatically by the
# iForged/keycloak-discord SPI — no manual mappers needed.

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
# Uses the iForged/keycloak-discord SPI which registers providerId "discord".
# The SPI hardcodes authorization/token/profile URLs and maps username, email
# and avatar automatically — we only supply client credentials and preferences.
IDP_PAYLOAD=$(cat <<EOF
{
  "alias": "${IDP_ALIAS}",
  "displayName": "Discord",
  "providerId": "discord",
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
    "defaultScope": "identify email",
    "syncMode": "IMPORT",
    "guiOrder": "1"
  }
}
EOF
)

# If the existing instance uses a different providerId (e.g. "oidc" or
# "oauth2"), delete it first — Keycloak does not allow changing providerId
# on an existing identity provider via update.
if "${KCADM}" get "identity-provider/instances/${IDP_ALIAS}" -r "${KC_REALM}" >/dev/null 2>&1; then
  CURRENT_PROVIDER_ID=$("${KCADM}" get "identity-provider/instances/${IDP_ALIAS}" \
    -r "${KC_REALM}" --fields providerId --format csv --noquotes 2>/dev/null | tail -n1 | tr -d '\r\n')
  if [[ "${CURRENT_PROVIDER_ID}" != "discord" ]]; then
    echo "Existing Discord IdP uses providerId='${CURRENT_PROVIDER_ID}', deleting before recreating as 'discord'..."
    "${KCADM}" delete "identity-provider/instances/${IDP_ALIAS}" -r "${KC_REALM}"
  fi
fi

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

# No manual mappers needed — the iForged/keycloak-discord SPI maps username,
# email and avatar automatically via extractIdentityFromProfile().

echo "Discord IdP created/updated on realm ${KC_REALM}"
