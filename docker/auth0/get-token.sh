#!/usr/bin/env bash
# Obtain a SMART Backend Services token from Auth0 using the client credentials flow.
#
# Required environment variables:
#   AUTH0_DOMAIN         e.g. dev-abc123.us.auth0.com
#   AUTH0_CLIENT_ID      Client ID from Application → Settings
#   AUTH0_CLIENT_SECRET  Client Secret from Application → Settings
#
# Optional:
#   AUTH0_AUDIENCE       API Identifier (default: https://fhir.example.com)
#   AUTH0_SCOPE          Space-separated SMART scopes (default: system/*.cruds)
#
# Usage:
#   export TOKEN=$(AUTH0_DOMAIN=... AUTH0_CLIENT_ID=... AUTH0_CLIENT_SECRET=... ./get-token.sh)

set -euo pipefail

: "${AUTH0_DOMAIN:?AUTH0_DOMAIN is required (e.g. dev-abc123.us.auth0.com)}"
: "${AUTH0_CLIENT_ID:?AUTH0_CLIENT_ID is required}"
: "${AUTH0_CLIENT_SECRET:?AUTH0_CLIENT_SECRET is required}"

AUDIENCE="${AUTH0_AUDIENCE:-https://fhir.example.com}"
SCOPE="${AUTH0_SCOPE:-system/*.cruds}"
TOKEN_ENDPOINT="https://${AUTH0_DOMAIN}/oauth/token"

echo "Requesting token from ${TOKEN_ENDPOINT}" >&2
echo "Audience: ${AUDIENCE}" >&2
echo "Scope: ${SCOPE}" >&2

RESPONSE=$(curl -sf -X POST "${TOKEN_ENDPOINT}" \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d "grant_type=client_credentials" \
  --data-urlencode "client_id=${AUTH0_CLIENT_ID}" \
  --data-urlencode "client_secret=${AUTH0_CLIENT_SECRET}" \
  --data-urlencode "audience=${AUDIENCE}" \
  --data-urlencode "scope=${SCOPE}")

ACCESS_TOKEN=$(echo "${RESPONSE}" | python3 -c "import sys,json; print(json.load(sys.stdin)['access_token'])")

echo "Token obtained (expires in $(echo "${RESPONSE}" | python3 -c "import sys,json; print(json.load(sys.stdin).get('expires_in','?'))") seconds)" >&2
echo "Scopes granted: $(echo "${RESPONSE}" | python3 -c "import sys,json; print(json.load(sys.stdin).get('scope','(none)'))") " >&2
echo "" >&2
echo "To decode claims:" >&2
echo "  echo \"\$TOKEN\" | cut -d. -f2 | base64 -d | python3 -m json.tool" >&2
echo "" >&2

echo "${ACCESS_TOKEN}"
