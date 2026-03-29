#!/usr/bin/env bash
# Obtain a SMART Backend Services token from Microsoft Entra ID using the client credentials flow.
#
# Required environment variables:
#   ENTRA_TENANT_ID      Directory (tenant) ID from the Azure portal
#   ENTRA_CLIENT_ID      Application (client) ID of the backend service app registration
#   ENTRA_CLIENT_SECRET  Client secret of the backend service app registration
#   ENTRA_API_CLIENT_ID  Application (client) ID of the HFS FHIR API app registration
#
# Optional:
#   ENTRA_SCOPE          OAuth2 scope (default: api://<ENTRA_API_CLIENT_ID>/.default)
#
# Usage:
#   export TOKEN=$(ENTRA_TENANT_ID=... ENTRA_CLIENT_ID=... ENTRA_CLIENT_SECRET=... ENTRA_API_CLIENT_ID=... ./get-token.sh)

set -euo pipefail

: "${ENTRA_TENANT_ID:?ENTRA_TENANT_ID is required (e.g. xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx)}"
: "${ENTRA_CLIENT_ID:?ENTRA_CLIENT_ID is required}"
: "${ENTRA_CLIENT_SECRET:?ENTRA_CLIENT_SECRET is required}"
: "${ENTRA_API_CLIENT_ID:?ENTRA_API_CLIENT_ID is required}"

SCOPE="${ENTRA_SCOPE:-api://${ENTRA_API_CLIENT_ID}/.default}"
TOKEN_ENDPOINT="https://login.microsoftonline.com/${ENTRA_TENANT_ID}/oauth2/v2.0/token"

echo "Requesting token from ${TOKEN_ENDPOINT}" >&2
echo "Scope: ${SCOPE}" >&2

RESPONSE=$(curl -sf -X POST "${TOKEN_ENDPOINT}" \
  -H "Content-Type: application/x-www-form-urlencoded" \
  --data-urlencode "grant_type=client_credentials" \
  --data-urlencode "client_id=${ENTRA_CLIENT_ID}" \
  --data-urlencode "client_secret=${ENTRA_CLIENT_SECRET}" \
  --data-urlencode "scope=${SCOPE}")

ACCESS_TOKEN=$(echo "${RESPONSE}" | python3 -c "import sys,json; print(json.load(sys.stdin)['access_token'])")

echo "Token obtained (expires in $(echo "${RESPONSE}" | python3 -c "import sys,json; print(json.load(sys.stdin).get('expires_in','?'))") seconds)" >&2
echo "" >&2
echo "To decode claims:" >&2
echo "  echo \"\$TOKEN\" | cut -d. -f2 | base64 -d | python3 -m json.tool" >&2
echo "" >&2

echo "${ACCESS_TOKEN}"
