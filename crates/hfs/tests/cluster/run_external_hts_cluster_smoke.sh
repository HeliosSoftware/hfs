#!/usr/bin/env bash
set -euo pipefail

# T3 wiring check for C3 (HTS terminology cache cross-instance invalidation):
# import a CodeSystem via HTS instance A, prove a *pre-warmed stale* cache on
# instance B is invalidated by the shared terminology epoch after an update
# via A — not just that a cold read on B sees fresh data (which would be true
# even without C3, since both instances share one Postgres). A separate
# script from run_external_cluster_smoke.sh: HTS is its own binary with its
# own lifecycle and no FHIR-resource-shaped helpers to share.

HTS_BASE_URL_A="${HTS_BASE_URL_A:?HTS_BASE_URL_A must be set}"
HTS_BASE_URL_B="${HTS_BASE_URL_B:?HTS_BASE_URL_B must be set}"
RESULTS_DIR="${RESULTS_DIR:-cluster-smoke-results}"
SMOKE_RUN_SUFFIX="${SMOKE_RUN_SUFFIX:-local-$(date +%s)-$$}"

HTTP_DIR="$RESULTS_DIR/hts-http"
SUMMARY_FILE="$RESULTS_DIR/summary.md"

mkdir -p "$HTTP_DIR"

log() {
  echo "[hts-cluster-smoke] $*"
}

fail() {
  local msg="$1"
  echo "[hts-cluster-smoke] ERROR: $msg" >&2
  mkdir -p "$(dirname "$SUMMARY_FILE")"
  echo "- FAIL (HTS): $msg" >> "$SUMMARY_FILE"
  for hts_log in "${HTS_LOG_A:-}" "${HTS_LOG_B:-}"; do
    if [ -n "$hts_log" ] && [ -f "$hts_log" ]; then
      echo "---- $hts_log (tail) ----" >&2
      tail -n 120 "$hts_log" >&2 || true
      echo "-------------------------" >&2
    fi
  done
  exit 1
}

pass() {
  local msg="$1"
  log "PASS: $msg"
  echo "- PASS (HTS): $msg" >> "$SUMMARY_FILE"
}

ID_SUFFIX="$(printf '%s' "$SMOKE_RUN_SUFFIX-hts" | tr -cs '[:alnum:]-' '-' | sed -e 's/^-*//' -e 's/-*$//')"
[ -n "$ID_SUFFIX" ] || ID_SUFFIX="hts-cluster-smoke"

CS_URL="http://hts-cluster-smoke.example/cs-$ID_SUFFIX"

# --- 6a. Health: both HTS instances answer -----------------------------------

for name in A B; do
  case "$name" in
    A) url="$HTS_BASE_URL_A" ;;
    B) url="$HTS_BASE_URL_B" ;;
  esac
  status="$(curl -sS -o "$HTTP_DIR/health-$name.txt" -w "%{http_code}" "$url/health")" \
    || fail "health check request to HTS $name ($url) failed"
  if [ "$status" != "200" ]; then
    fail "HTS health check on $name returned HTTP $status, expected 200"
  fi
done
pass "HTS health endpoint answers on instance A and instance B"

# --- 6b. Create via A, warm B's caches with the pre-update display ----------

CS_V1="$HTTP_DIR/cs-v1.json"
cat > "$CS_V1" <<EOF
{
  "resourceType": "CodeSystem",
  "url": "$CS_URL",
  "version": "1.0",
  "name": "HtsClusterSmokeCS",
  "status": "active",
  "content": "complete",
  "concept": [{"code": "widget", "display": "Old Display"}]
}
EOF

status="$(curl -sS -o "$HTTP_DIR/cs-create.json" -w "%{http_code}" \
  -X POST "$HTS_BASE_URL_A/CodeSystem" \
  -H "Content-Type: application/fhir+json" --data-binary @"$CS_V1")" \
  || fail "CodeSystem create via HTS instance A failed"
if [ "$status" != "201" ]; then
  cat "$HTTP_DIR/cs-create.json" >&2 || true
  fail "CodeSystem create via HTS instance A returned HTTP $status, expected 201"
fi
CS_ID="$(jq -r '.id // empty' "$HTTP_DIR/cs-create.json")"
[ -n "$CS_ID" ] || fail "CodeSystem create response has no id"

LOOKUP_PATH="/CodeSystem/\$lookup?system=$CS_URL&code=widget&version=1.0"
status="$(curl -sS -o "$HTTP_DIR/lookup-warm-b.json" -w "%{http_code}" \
  --get "$HTS_BASE_URL_B$LOOKUP_PATH")" || fail "warm lookup via HTS instance B failed"
if [ "$status" != "200" ]; then
  cat "$HTTP_DIR/lookup-warm-b.json" >&2 || true
  fail "warm lookup via HTS instance B returned HTTP $status, expected 200"
fi
WARM_DISPLAY="$(jq -r '.parameter[]? | select(.name=="display") | .valueString // empty' "$HTTP_DIR/lookup-warm-b.json")"
[ "$WARM_DISPLAY" = "Old Display" ] || fail "warm lookup via B returned display '$WARM_DISPLAY', expected 'Old Display'"
pass "CodeSystem created via A is readable via B (warms B's caches with the pre-update display)"

# --- 6c. Update via A, assert B serves the fresh display ---------------------
# B never saw the update directly — only the shared terminology_epoch check
# on B's next request should invalidate its stale handler- and backend-layer
# lookup caches.

CS_V1_UPDATED="$HTTP_DIR/cs-v1-updated.json"
cat > "$CS_V1_UPDATED" <<EOF
{
  "resourceType": "CodeSystem",
  "id": "$CS_ID",
  "url": "$CS_URL",
  "version": "1.0",
  "name": "HtsClusterSmokeCS",
  "status": "active",
  "content": "complete",
  "concept": [{"code": "widget", "display": "New Display"}]
}
EOF

status="$(curl -sS -o "$HTTP_DIR/cs-update.json" -w "%{http_code}" \
  -X PUT "$HTS_BASE_URL_A/CodeSystem/$CS_ID" \
  -H "Content-Type: application/fhir+json" --data-binary @"$CS_V1_UPDATED")" \
  || fail "CodeSystem update via HTS instance A failed"
if [ "$status" != "200" ]; then
  cat "$HTTP_DIR/cs-update.json" >&2 || true
  fail "CodeSystem update via HTS instance A returned HTTP $status, expected 200"
fi

# B's epoch check is memoized (~1s in production, per EpochGuard::new's
# default — the T2 unit/integration tests use a zero memo window for
# determinism, but this is a real process on its production default), so
# poll for a few seconds rather than asserting on the very next request.
FRESH_DISPLAY=""
for _ in $(seq 1 10); do
  status="$(curl -sS -o "$HTTP_DIR/lookup-fresh-b.json" -w "%{http_code}" \
    --get "$HTS_BASE_URL_B$LOOKUP_PATH")" || fail "post-update lookup via HTS instance B failed"
  if [ "$status" != "200" ]; then
    cat "$HTTP_DIR/lookup-fresh-b.json" >&2 || true
    fail "post-update lookup via HTS instance B returned HTTP $status, expected 200"
  fi
  FRESH_DISPLAY="$(jq -r '.parameter[]? | select(.name=="display") | .valueString // empty' "$HTTP_DIR/lookup-fresh-b.json")"
  [ "$FRESH_DISPLAY" = "New Display" ] && break
  sleep 1
done
if [ "$FRESH_DISPLAY" != "New Display" ]; then
  cat "$HTTP_DIR/lookup-fresh-b.json" >&2 || true
  fail "post-update lookup via B returned display '$FRESH_DISPLAY', expected 'New Display' (stale cache was not invalidated within 10s)"
fi
pass "CodeSystem updated via A invalidates B's pre-warmed stale lookup caches (terminology epoch)"

log "all HTS cluster smoke checks passed"
