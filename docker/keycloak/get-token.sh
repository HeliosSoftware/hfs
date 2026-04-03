#!/usr/bin/env bash
# Obtain a SMART Backend Services token from the local Keycloak dev instance.
#
# Usage:
#   ./get-token.sh                     # full-access client (system/*.cruds)
#   ./get-token.sh hfs-readonly-client # read-only client (system/Patient.rs)
#
# The access_token is printed to stdout; all other output goes to stderr.
# Example: export TOKEN=$(./get-token.sh)

set -euo pipefail

KEYCLOAK_URL="${KEYCLOAK_URL:-http://localhost:8180}"
REALM="${REALM:-fhir}"
CLIENT_ID="${1:-hfs-backend-client}"

case "${CLIENT_ID}" in
  hfs-backend-client)  CLIENT_SECRET="${CLIENT_SECRET:-hfs-backend-secret}" ;;
  hfs-readonly-client) CLIENT_SECRET="${CLIENT_SECRET:-hfs-readonly-secret}" ;;
  *)                   CLIENT_SECRET="${CLIENT_SECRET:?CLIENT_SECRET must be set for custom client IDs}" ;;
esac

TOKEN_ENDPOINT="${KEYCLOAK_URL}/realms/${REALM}/protocol/openid-connect/token"

echo "Requesting token from ${TOKEN_ENDPOINT} (client: ${CLIENT_ID})" >&2

RESPONSE=$(curl -sf -X POST "${TOKEN_ENDPOINT}" \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d "grant_type=client_credentials" \
  --data-urlencode "client_id=${CLIENT_ID}" \
  --data-urlencode "client_secret=${CLIENT_SECRET}")

ACCESS_TOKEN=$(echo "${RESPONSE}" | python3 -c "import sys,json; print(json.load(sys.stdin)['access_token'])")

echo "Token obtained (expires in $(echo "${RESPONSE}" | python3 -c "import sys,json; print(json.load(sys.stdin).get('expires_in','?'))") seconds)" >&2
echo "Scopes: $(echo "${RESPONSE}" | python3 -c "import sys,json; print(json.load(sys.stdin).get('scope','(none)'))") " >&2
echo "" >&2
echo "To decode claims:" >&2
echo "  echo \"\$TOKEN\" | cut -d. -f2 | base64 -d | python3 -m json.tool" >&2
echo "" >&2

# Print the raw token to stdout so callers can do: export TOKEN=\$(./get-token.sh)
echo "${ACCESS_TOKEN}"
