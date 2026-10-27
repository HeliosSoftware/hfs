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

# Pick a JSON parser once. We *test* each candidate against a known document
# rather than trusting `command -v`, because on some machines (e.g. Windows via
# Git Bash) a `python3` execution-alias stub resolves but then fails to run.
# Order: jq, then node, then python3 — whichever actually works.
JSON_PARSER=""
for candidate in jq node python3; do
  case "${candidate}" in
    jq)      echo '{"x":"ok"}' | jq -r '.x' 2>/dev/null | grep -qx ok && JSON_PARSER=jq && break ;;
    node)    echo '{"x":"ok"}' | node -e "let d='';process.stdin.on('data',c=>d+=c).on('end',()=>process.stdout.write(JSON.parse(d).x))" 2>/dev/null | grep -qx ok && JSON_PARSER=node && break ;;
    python3) echo '{"x":"ok"}' | python3 -c "import sys,json;print(json.load(sys.stdin)['x'])" 2>/dev/null | grep -qx ok && JSON_PARSER=python3 && break ;;
  esac
done
if [ -z "${JSON_PARSER}" ]; then
  echo "Need a working jq, node, or python3 to parse the token response" >&2
  exit 1
fi

# Extract a top-level string field from the JSON response using the chosen parser.
json_field() {
  local field="$1"
  case "${JSON_PARSER}" in
    jq)      echo "${RESPONSE}" | jq -r ".${field} // \"\"" ;;
    node)    echo "${RESPONSE}" | node -e "let d='';process.stdin.on('data',c=>d+=c).on('end',()=>console.log(JSON.parse(d)['${field}']??''))" ;;
    python3) echo "${RESPONSE}" | python3 -c "import sys,json; print(json.load(sys.stdin).get('${field}',''))" ;;
  esac
}

ACCESS_TOKEN=$(json_field access_token)

echo "Token obtained (expires in $(json_field expires_in) seconds)" >&2
echo "Scopes: $(json_field scope) " >&2
echo "" >&2
echo "To decode claims:" >&2
echo "  echo \"\$TOKEN\" | cut -d. -f2 | base64 -d | python3 -m json.tool" >&2
echo "" >&2

# Print the raw token to stdout so callers can do: export TOKEN=\$(./get-token.sh)
echo "${ACCESS_TOKEN}"
