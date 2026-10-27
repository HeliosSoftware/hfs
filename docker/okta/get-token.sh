#!/usr/bin/env bash
# Obtain a SMART Backend Services token from Okta using the client credentials flow.
#
# Required environment variables:
#   OKTA_DOMAIN          e.g. dev-12345678.okta.com
#   OKTA_AUTH_SERVER_ID  e.g. aus1abc2defGHIJK  (from Security → API → Authorization Servers)
#   OKTA_CLIENT_ID       Client ID of your API Services app
#   OKTA_CLIENT_SECRET   Client secret of your API Services app
#
# Optional:
#   OKTA_SCOPE           Space-separated SMART scopes (default: system/*.cruds)
#
# Usage:
#   export TOKEN=$(OKTA_DOMAIN=... OKTA_AUTH_SERVER_ID=... OKTA_CLIENT_ID=... OKTA_CLIENT_SECRET=... ./get-token.sh)

set -euo pipefail

: "${OKTA_DOMAIN:?OKTA_DOMAIN is required (e.g. dev-12345678.okta.com)}"
: "${OKTA_AUTH_SERVER_ID:?OKTA_AUTH_SERVER_ID is required (e.g. aus1abc2defGHIJK)}"
: "${OKTA_CLIENT_ID:?OKTA_CLIENT_ID is required}"
: "${OKTA_CLIENT_SECRET:?OKTA_CLIENT_SECRET is required}"

SCOPE="${OKTA_SCOPE:-system/*.cruds}"
TOKEN_ENDPOINT="https://${OKTA_DOMAIN}/oauth2/${OKTA_AUTH_SERVER_ID}/v1/token"

echo "Requesting token from ${TOKEN_ENDPOINT}" >&2
echo "Scope: ${SCOPE}" >&2

RESPONSE=$(curl -sf -X POST "${TOKEN_ENDPOINT}" \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -u "${OKTA_CLIENT_ID}:${OKTA_CLIENT_SECRET}" \
  --data-urlencode "grant_type=client_credentials" \
  --data-urlencode "scope=${SCOPE}")

ACCESS_TOKEN=$(echo "${RESPONSE}" | python3 -c "import sys,json; print(json.load(sys.stdin)['access_token'])")

echo "Token obtained (expires in $(echo "${RESPONSE}" | python3 -c "import sys,json; print(json.load(sys.stdin).get('expires_in','?'))") seconds)" >&2
echo "Scopes granted: $(echo "${RESPONSE}" | python3 -c "import sys,json; print(json.load(sys.stdin).get('scope','(none)'))") " >&2
echo "" >&2
echo "To decode claims:" >&2
echo "  echo \"\$TOKEN\" | cut -d. -f2 | base64 -d | python3 -m json.tool" >&2
echo "" >&2

echo "${ACCESS_TOKEN}"
