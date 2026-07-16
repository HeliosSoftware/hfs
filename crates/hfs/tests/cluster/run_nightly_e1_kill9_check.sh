#!/usr/bin/env bash
# Nightly T3 kill-9 recovery check (E1). Composite secondary-backend sync
# is the other worst-blast-radius durable path (alongside A1): this check
# kill -9s the instance
# whose worker is mid-drain of the composite_sync_outbox and asserts the
# surviving instance reclaims the orphaned rows and the secondary
# (Elasticsearch) index converges — no silently lost propagation.
#
# Unlike A1 (one long-running export job, deterministically claimed once),
# each composite-sync row's apply is a single fast Elasticsearch write, so
# there is no single "claim" event to wait for. Instead this seeds a large
# batch (so the single-worker drain takes real wall-clock time) and races
# the kill against the drain, polling for a healthy queued/applying backlog
# before pulling the trigger — the same "seed enough that it can't finish
# before the kill lands" principle A1 uses, adapted to E1's many-small-rows
# shape. Deliberately slow; nightly schedule only, never `cargo test`.
#
# Sequence:
#   1. stop instance B (of the dedicated postgres-elasticsearch pair) →
#      only A's worker drains the outbox
#   2. POST a large transaction Bundle of Patients via A — durably enqueues
#      one composite_sync_outbox row per resource; the Bundle write itself
#      returns fast, it does not wait for the Elasticsearch applies
#   3. poll composite_sync_outbox until a healthy backlog is still
#      queued/applying (proves the single worker hasn't already drained it)
#   4. kill -9 A — rows the worker had claimed are now orphaned mid-apply
#   5. restart B; its worker reclaims the orphaned rows (bumped fencing
#      token, new worker_id) and drains the rest
#   6. poll until every row from this batch reads applied
#   7. verify via Elasticsearch search through B that the full patient count
#      is present — proves no lost propagation, not just "marked applied"

set -euo pipefail

: "${PG_CONTAINER:?PG_CONTAINER is required}"
: "${HFS_PID_A:?HFS_PID_A is required}"
: "${HFS_PID_B:?HFS_PID_B is required}"
: "${BASE_URL_A:?BASE_URL_A is required}"
: "${BASE_URL_B:?BASE_URL_B is required}"
: "${HFS_PORT_B:?HFS_PORT_B is required}"
: "${HFS_BINARY:?HFS_BINARY is required}"
: "${CLUSTER_ENV_FILE:?CLUSTER_ENV_FILE is required}"
RESULTS_DIR="${RESULTS_DIR:-cluster-smoke-results}"
SEED_ROWS="${SEED_ROWS:-2000}"

mkdir -p "$RESULTS_DIR"
SUMMARY_FILE="$RESULTS_DIR/nightly-e1-summary.md"
HTTP_DIR="$RESULTS_DIR/nightly-e1-http"
mkdir -p "$HTTP_DIR"
echo "## Nightly kill-9 recovery (E1)" > "$SUMMARY_FILE"

log() { echo "[e1-kill9-check] $*"; }
fail() {
  echo "[e1-kill9-check] ERROR: $*" >&2
  echo "- FAIL: $*" >> "$SUMMARY_FILE"
  exit 1
}
pass() {
  log "OK: $*"
  echo "- PASS: $*" >> "$SUMMARY_FILE"
}

FHIR_CT="application/fhir+json; fhirVersion=4.0"

TENANT="$(grep '^HFS_DEFAULT_TENANT=' "$CLUSTER_ENV_FILE" | head -1 | cut -d= -f2-)"
[ -n "$TENANT" ] || fail "could not read HFS_DEFAULT_TENANT from $CLUSTER_ENV_FILE"

pg() {
  docker exec "$PG_CONTAINER" psql -U helios -d helios -tA -c "$1"
}

FAMILY="E1Kill$(date +%s)"

# --- 1. Stop B gracefully so instance A must drain the outbox ---------------
if kill -0 "$HFS_PID_B" 2>/dev/null; then
  kill -INT "$HFS_PID_B" 2>/dev/null || true
  for _ in $(seq 1 50); do
    kill -0 "$HFS_PID_B" 2>/dev/null || break
    sleep 0.2
  done
  kill -9 "$HFS_PID_B" 2>/dev/null || true
fi
pass "instance B (postgres-elasticsearch pair) stopped; only A's worker remains"

# --- 2. POST a large transaction Bundle of Patients via A -------------------
# One Bundle entry per Patient; each entry's create() call durably enqueues
# its own composite_sync_outbox row (BulkSync — a genuinely different code
# path — is not involved here). The Bundle write itself is fast: it commits
# to the primary and returns before any Elasticsearch apply happens.
BUNDLE_FILE="$HTTP_DIR/seed-bundle.json"
jq -n --arg family "$FAMILY" --argjson n "$SEED_ROWS" '
  {
    resourceType: "Bundle",
    type: "transaction",
    entry: [
      range(0; $n) | {
        resource: {
          resourceType: "Patient",
          id: ("e1kill-" + (. | tostring)),
          name: [{family: $family, given: [("Patient" + (. | tostring))]}]
        },
        request: {method: "PUT", url: ("Patient/e1kill-" + (. | tostring))}
      }
    ]
  }' > "$BUNDLE_FILE"

status="$(curl -sS -o "$HTTP_DIR/seed-bundle.out" -w "%{http_code}" \
  -X POST "$BASE_URL_A/" \
  -H "Content-Type: $FHIR_CT" -H "Accept: $FHIR_CT" \
  --data-binary @"$BUNDLE_FILE")" || fail "seed bundle POST failed"
[ "$status" = "200" ] || fail "seed bundle via A returned HTTP $status, expected 200"
pass "$SEED_ROWS patients created via A (transaction Bundle), family=$FAMILY"

# --- 3. Wait for a healthy queued/applying backlog to exist ------------------
# The single worker drains sequentially; with SEED_ROWS large enough this
# takes real wall-clock time. Poll briefly for a nonzero backlog rather than
# assuming — if the worker somehow already finished, fail loudly (raise
# SEED_ROWS) rather than silently proving nothing.
BACKLOG=""
for _ in $(seq 1 30); do
  count="$(pg "SELECT count(*) FROM composite_sync_outbox WHERE status IN ('queued','applying')")"
  if [ "$count" -gt 0 ]; then
    BACKLOG="$count"
    break
  fi
  sleep 0.2
done
[ -n "$BACKLOG" ] || fail "composite_sync_outbox already fully drained before the kill could land — raise SEED_ROWS"
pass "$BACKLOG rows still queued/applying — kill window confirmed"

# --- 4. kill -9 the instance whose worker holds any in-flight leases --------
kill -9 "$HFS_PID_A" 2>/dev/null || fail "could not kill instance A"
ORPHANED="$(pg "SELECT count(*) FROM composite_sync_outbox WHERE status = 'applying'")"
pass "instance A killed with -9; $ORPHANED row(s) left applying under the corpse's lease"

# --- 5. Restart B; it must reclaim orphaned rows and drain the rest ---------
# shellcheck disable=SC2046
env $(grep -v '^\s*$' "$CLUSTER_ENV_FILE") \
  "$HFS_BINARY" --log-level info --port "$HFS_PORT_B" --host 0.0.0.0 \
  >> "$RESULTS_DIR/hfs-e1-b.log" 2>&1 &
NEW_B_PID=$!
echo "$NEW_B_PID" > "$RESULTS_DIR/nightly-e1-b.pid"
for i in $(seq 1 45); do
  kill -0 "$NEW_B_PID" 2>/dev/null || { tail -50 "$RESULTS_DIR/hfs-e1-b.log" >&2; fail "restarted B exited"; }
  if curl -sf "$BASE_URL_B/health" > /dev/null 2>&1; then
    break
  fi
  [ "$i" = "45" ] && fail "restarted B did not become healthy"
  sleep 2
done
pass "instance B restarted"

# --- 6. Wait for every row from this batch to reach 'applied' ---------------
# Lease is 60s; B's worker polls on a 1s floor after that. Generous slack for
# draining the remaining backlog plus the reclaim.
DRAINED=""
for _ in $(seq 1 300); do
  remaining="$(pg "SELECT count(*) FROM composite_sync_outbox WHERE status IN ('queued','applying')")"
  failed_count="$(pg "SELECT count(*) FROM composite_sync_outbox WHERE status = 'failed'")"
  if [ "$failed_count" -gt 0 ]; then
    fail "$failed_count composite_sync_outbox row(s) reached terminal failure"
  fi
  if [ "$remaining" = "0" ]; then
    DRAINED=1
    break
  fi
  sleep 1
done
[ -n "$DRAINED" ] || fail "composite_sync_outbox did not fully drain within 300s of the kill"
pass "composite_sync_outbox fully drained (0 queued/applying) after the kill"

# --- 7. Verify convergence: Elasticsearch actually has every patient -------
# Through B, which is now serving search off the same Elasticsearch index —
# proves the applies genuinely landed, not just that the outbox rows say so.
COUNT=""
for _ in $(seq 1 30); do
  status="$(curl -sS -o "$HTTP_DIR/search-count.json" -w "%{http_code}" \
    -H "Accept: $FHIR_CT" \
    "$BASE_URL_B/Patient?family=$FAMILY&_summary=count")" \
    || fail "search-count via B failed"
  [ "$status" = "200" ] || fail "search-count via B returned HTTP $status, expected 200"
  COUNT="$(jq -r '.total // 0' "$HTTP_DIR/search-count.json")"
  [ "$COUNT" = "$SEED_ROWS" ] && break
  sleep 1
done
[ "$COUNT" = "$SEED_ROWS" ] || fail "Elasticsearch search via B found $COUNT/$SEED_ROWS patients — propagation lost"
pass "Elasticsearch search via B found all $SEED_ROWS patients — no lost propagation across the kill"

log "nightly E1 kill-9 recovery check passed"
echo "" >> "$SUMMARY_FILE"
echo "E1 kill-9 recovery check passed." >> "$SUMMARY_FILE"
