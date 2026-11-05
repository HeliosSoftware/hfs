#!/usr/bin/env bash
# Apply the Helios login branding to an Auth0 tenant's Universal Login.
#
# Auth0 hosts the login page, so there is no theme directory to mount (unlike
# docker/keycloak/themes/helios). This script is the repo-side equivalent: it
# pushes theme.json and the tenant branding settings through the Management API.
# Safe to re-run; every call overwrites the previous state.
#
# Required environment variables:
#   AUTH0_DOMAIN               e.g. dev-abc123.us.auth0.com
#
# Management API credentials, first match wins:
#   AUTH0_MGMT_TOKEN           a Management API access token, or
#   AUTH0_MGMT_CLIENT_ID +     an M2M application authorized for the Auth0
#   AUTH0_MGMT_CLIENT_SECRET   Management API (read:branding update:branding
#                              create:branding read:prompts update:prompts), or
#   (neither)                  a logged-in Auth0 CLI (`auth0 login`) on PATH
#
# Optional:
#   HELIOS_BRANDING_ASSET_BASE Base HTTPS URL that the logo and font paths in
#                              theme.json resolve against. Auth0 fetches them
#                              itself, so they must be public. Default: this
#                              repository's main branch on raw.githubusercontent.com
#
# Usage:
#   AUTH0_DOMAIN=... ./apply-branding.sh            # apply
#   AUTH0_DOMAIN=... ./apply-branding.sh --dry-run  # print the payloads only

set -euo pipefail

: "${AUTH0_DOMAIN:?AUTH0_DOMAIN is required (e.g. dev-abc123.us.auth0.com)}"

DRY_RUN=false
[[ "${1:-}" == "--dry-run" ]] && DRY_RUN=true

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ASSET_BASE="${HELIOS_BRANDING_ASSET_BASE:-https://raw.githubusercontent.com/HeliosSoftware/hfs/main}"
ASSET_BASE="${ASSET_BASE%/}"

# Keep these in sync with theme.json (and crates/ui/assets/app.css).
LOGO_URL="${ASSET_BASE}/crates/ui/assets/logo.png"
FONT_URL="${ASSET_BASE}/docker/keycloak/themes/helios/login/resources/fonts/figtree-latin.woff2"

THEME=$(sed "s|__ASSET_BASE__|${ASSET_BASE}|g" "${HERE}/theme.json")

BRANDING=$(cat <<EOF
{
  "colors": { "primary": "#33b8ff", "page_background": "#f5f5f5" },
  "logo_url": "${LOGO_URL}",
  "font": { "url": "${FONT_URL}" }
}
EOF
)

# The "Log in to <tenant> to continue to <app>" subtitle would otherwise show
# the tenant's generated name (dev-xxxx).
LOGIN_TEXT='{ "login": { "description": "Log in to continue to the Helios FHIR Server." } }'
LOGIN_ID_TEXT='{ "login-id": { "description": "Log in to continue to the Helios FHIR Server." } }'

if $DRY_RUN; then
  printf 'PATCH /api/v2/branding\n%s\n\n' "$BRANDING"
  printf 'POST|PATCH /api/v2/branding/themes\n%s\n\n' "$THEME"
  printf 'PUT /api/v2/prompts/login/custom-text/en\n%s\n\n' "$LOGIN_TEXT"
  printf 'PUT /api/v2/prompts/login-id/custom-text/en\n%s\n' "$LOGIN_ID_TEXT"
  exit 0
fi

# --- Management API transport ------------------------------------------------

if [[ -z "${AUTH0_MGMT_TOKEN:-}" && -n "${AUTH0_MGMT_CLIENT_ID:-}" ]]; then
  : "${AUTH0_MGMT_CLIENT_SECRET:?AUTH0_MGMT_CLIENT_SECRET is required with AUTH0_MGMT_CLIENT_ID}"
  echo "Requesting a Management API token for ${AUTH0_DOMAIN}" >&2
  RESPONSE=$(curl -sf -X POST "https://${AUTH0_DOMAIN}/oauth/token" \
    -H "Content-Type: application/x-www-form-urlencoded" \
    --data-urlencode "grant_type=client_credentials" \
    --data-urlencode "client_id=${AUTH0_MGMT_CLIENT_ID}" \
    --data-urlencode "client_secret=${AUTH0_MGMT_CLIENT_SECRET}" \
    --data-urlencode "audience=https://${AUTH0_DOMAIN}/api/v2/")
  AUTH0_MGMT_TOKEN=$(printf '%s' "$RESPONSE" | sed -n 's/.*"access_token":"\([^"]*\)".*/\1/p')
  [[ -n "$AUTH0_MGMT_TOKEN" ]] || { echo "No access_token in response: $RESPONSE" >&2; exit 1; }
fi

# mgmt METHOD PATH [JSON] — prints the response body, fails on HTTP errors.
mgmt() {
  local method=$1 path=$2 body=${3:-}
  if [[ -n "${AUTH0_MGMT_TOKEN:-}" ]]; then
    curl -sS --fail-with-body -X "$method" "https://${AUTH0_DOMAIN}/api/v2/${path}" \
      -H "Authorization: Bearer ${AUTH0_MGMT_TOKEN}" \
      -H "Content-Type: application/json" \
      ${body:+--data "$body"}
  elif command -v auth0 >/dev/null; then
    auth0 api "$(tr '[:upper:]' '[:lower:]' <<<"$method")" "$path" --no-input ${body:+--data "$body"}
  else
    echo "No Management API credentials: set AUTH0_MGMT_TOKEN or AUTH0_MGMT_CLIENT_ID/SECRET, or log in with the Auth0 CLI" >&2
    exit 1
  fi
}

# --- Apply --------------------------------------------------------------------

EXPERIENCE=$(mgmt GET prompts)
if ! grep -Eq '"universal_login_experience" *: *"new"' <<<"$EXPERIENCE"; then
  echo "Warning: the tenant is not on the new Universal Login experience; the theme only applies to it." >&2
  echo "         Switch it in Dashboard → Branding → Universal Login, then re-run." >&2
fi

echo "Updating tenant branding (logo, primary color, font)" >&2
mgmt PATCH branding "$BRANDING" >/dev/null

# GET branding/themes/default returns 404 until a theme exists.
THEME_ID=$(mgmt GET branding/themes/default 2>/dev/null \
  | sed -n 's/.*"themeId" *: *"\([^"]*\)".*/\1/p' || true)
if [[ -n "$THEME_ID" ]]; then
  echo "Updating theme ${THEME_ID}" >&2
  mgmt PATCH "branding/themes/${THEME_ID}" "$THEME" >/dev/null
else
  echo "Creating the Helios theme" >&2
  mgmt POST branding/themes "$THEME" >/dev/null
fi

echo "Setting the login subtitle text" >&2
mgmt PUT prompts/login/custom-text/en "$LOGIN_TEXT" >/dev/null
mgmt PUT prompts/login-id/custom-text/en "$LOGIN_ID_TEXT" >/dev/null

echo "Done. Open the login page: https://${AUTH0_DOMAIN}/authorize?client_id=<web client id>&response_type=code&scope=openid&redirect_uri=<callback>" >&2
