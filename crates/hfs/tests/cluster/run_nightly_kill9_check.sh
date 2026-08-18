#!/usr/bin/env bash
# Nightly T3 kill-9 recovery check (A1). SoF export is one of the two
# worst-blast-radius durable paths: this check kill -9s the instance holding
# an export lease mid-run and asserts the surviving instance reclaims the
# orphaned lease after expiry and completes the job — no lost or torn job,
# no lock held by the corpse.
#
# Deliberately slow (waits out a real ~60s lease expiry), so it runs in the
# nightly schedule only, never in `cargo test`, and outside coverage.
#
# Sequence (deterministic about *which* instance holds the lease):
#   1. stop B gracefully → only A's workers can claim
#   2. seed enough rows that the export cannot finish before the kill lands
#   3. kick off on A; wait until the cluster_jobs row is claimed (running)
#   4. kill -9 A — the lease is now orphaned
#   5. restart B; it reclaims after lease expiry and completes
#   6. poll + download via B and verify the data survived intact

set -euo pipefail

: "${PG_CONTAINER:?PG_CONTAINER is required}"
: "${HFS_PID_A:?HFS_PID_A is required}"
: "${HFS_PID_B:?HFS_PID_B is required}"
: "${BASE_URL_A:?BASE_URL_A is required}"
: "${BASE_URL_B:?BASE_URL_B is required}"
: "${HFS_PORT_B:?HFS_PORT_B is required}"
: "${HFS_BINARY:?HFS_BINARY is required}"
: "${CLUSTER_ENV_FILE:?CLUSTER_ENV_FILE is required}"
FHIR_VERSION="${FHIR_VERSION:-R4}"
RESULTS_DIR="${RESULTS_DIR:-cluster-smoke-results}"
SEED_ROWS="${SEED_ROWS:-200000}"

mkdir -p "$RESULTS_DIR"
SUMMARY_FILE="$RESULTS_DIR/nightly-summary.md"
HTTP_DIR="$RESULTS_DIR/nightly-http"
mkdir -p "$HTTP_DIR"
echo "## Nightly kill-9 recovery (A1)" > "$SUMMARY_FILE"

log() { echo "[kill9-check] $*"; }
fail() {
  echo "[kill9-check] ERROR: $*" >&2
  echo "- FAIL: $*" >> "$SUMMARY_FILE"
  exit 1
}
pass() {
  log "OK: $*"
  echo "- PASS: $*" >> "$SUMMARY_FILE"
}

case "$FHIR_VERSION" in
  R4) FHIR_CT="application/fhir+json; fhirVersion=4.0" ;;
  R4B) FHIR_CT="application/fhir+json; fhirVersion=4.3" ;;
  R5) FHIR_CT="application/fhir+json; fhirVersion=5.0" ;;
  R6) FHIR_CT="application/fhir+json; fhirVersion=6.0" ;;
  *) fail "unknown FHIR_VERSION '$FHIR_VERSION'" ;;
esac
case "$FHIR_VERSION" in
  R4) FHIR_MIME="4.0" ;;
  R4B) FHIR_MIME="4.3" ;;
  R5) FHIR_MIME="5.0" ;;
  R6) FHIR_MIME="6.0" ;;
esac

TENANT="$(grep '^HFS_DEFAULT_TENANT=' "$CLUSTER_ENV_FILE" | head -1 | cut -d= -f2-)"
[ -n "$TENANT" ] || fail "could not read HFS_DEFAULT_TENANT from $CLUSTER_ENV_FILE"

pg() {
  docker exec "$PG_CONTAINER" psql -U helios -d helios -tA -c "$1"
}

# --- 1. Stop B gracefully so instance A must claim the job ------------------
if kill -0 "$HFS_PID_B" 2>/dev/null; then
  kill -INT "$HFS_PID_B" 2>/dev/null || true
  for _ in $(seq 1 50); do
    kill -0 "$HFS_PID_B" 2>/dev/null || break
    sleep 0.2
  done
  kill -9 "$HFS_PID_B" 2>/dev/null || true
fi
pass "instance B stopped; only A's workers remain"

# --- 2. Seed enough rows that the export cannot finish instantly ------------
# Inserted directly into the resources table (export reads it; no search
# indexing needed), so seeding is seconds instead of minutes of HTTP PUTs.
pg "INSERT INTO resources (tenant_id, resource_type, id, version_id, data, last_updated, is_deleted, fhir_version)
    SELECT '$TENANT', 'Patient', 'kill9-' || g, '1',
           jsonb_build_object('resourceType', 'Patient', 'id', 'kill9-' || g,
                              'name', jsonb_build_array(jsonb_build_object('family', 'Kill' || g))),
           now(), false, '$FHIR_MIME'
    FROM generate_series(1, $SEED_ROWS) g
    ON CONFLICT DO NOTHING" > /dev/null \
  || fail "failed to seed $SEED_ROWS patients"
pass "$SEED_ROWS patients seeded for tenant $TENANT"

# --- 3. Kick off the export on A and wait until a worker claims it ----------
EXPORT_BODY="$HTTP_DIR/kickoff.json"
cat > "$EXPORT_BODY" <<EOF
{
  "resourceType": "Parameters",
  "parameter": [{
    "name": "view",
    "part": [
      {"name": "name", "valueString": "patients"},
      {"name": "viewResource", "resource": {
        "resourceType": "ViewDefinition",
        "resource": "Patient",
        "status": "active",
        "select": [{"column": [{"path": "id", "name": "patient_id", "type": "string"}]}]
      }}
    ]
  }]
}
EOF

KICKOFF_HEADERS="$HTTP_DIR/kickoff-headers.txt"
status="$(curl -sS -o "$HTTP_DIR/kickoff.out" -w "%{http_code}" -D "$KICKOFF_HEADERS" \
  -X POST "$BASE_URL_A/ViewDefinition/\$viewdefinition-export" \
  -H "Content-Type: $FHIR_CT" -H "Accept: $FHIR_CT" -H "Prefer: respond-async" \
  --data-binary @"$EXPORT_BODY")" || fail "kickoff curl failed"
[ "$status" = "202" ] || fail "kickoff on A returned HTTP $status, expected 202"

STATUS_URL="$(awk 'tolower($1) == "content-location:" {print $2}' "$KICKOFF_HEADERS" | tr -d '\r' | tail -1)"
JOB_ID="$(echo "$STATUS_URL" | sed 's|.*/export/\([^/]*\)/status.*|\1|')"
[ -n "$JOB_ID" ] || fail "could not derive the job id from '$STATUS_URL'"
log "export job $JOB_ID kicked off on A"

CLAIMED=""
for _ in $(seq 1 120); do
  state="$(pg "SELECT status FROM cluster_jobs WHERE id = '$JOB_ID'")"
  case "$state" in
    running) CLAIMED=1; break ;;
    queued) sleep 0.5 ;;
    completed) fail "export completed before the kill could land — raise SEED_ROWS" ;;
    *) fail "unexpected cluster_jobs status '$state' while waiting for the claim" ;;
  esac
done
[ -n "$CLAIMED" ] || fail "job was not claimed within 60s"
ORPHAN_WORKER="$(pg "SELECT worker_id FROM cluster_jobs WHERE id = '$JOB_ID'")"
pass "job claimed by worker $ORPHAN_WORKER on instance A"

# --- 4. kill -9 the lease holder --------------------------------------------
kill -9 "$HFS_PID_A" 2>/dev/null || fail "could not kill instance A"
state="$(pg "SELECT status FROM cluster_jobs WHERE id = '$JOB_ID'")"
[ "$state" = "running" ] || fail "expected the orphaned job to still read running, got '$state'"
pass "instance A killed with -9; the corpse still holds the lease"

# --- 5. Restart B; it must reclaim the orphaned lease after expiry ----------
# The env list is single-line KEY=VALUE pairs with no spaces (URLs/idents).
# shellcheck disable=SC2046
env $(grep -v '^\s*$' "$CLUSTER_ENV_FILE") \
  "$HFS_BINARY" --log-level info --port "$HFS_PORT_B" --host 0.0.0.0 \
  >> "$RESULTS_DIR/hfs-b.log" 2>&1 &
NEW_B_PID=$!
echo "$NEW_B_PID" > "$RESULTS_DIR/nightly-b.pid"
for i in $(seq 1 45); do
  kill -0 "$NEW_B_PID" 2>/dev/null || { tail -50 "$RESULTS_DIR/hfs-b.log" >&2; fail "restarted B exited"; }
  if curl -sf "$BASE_URL_B/health" > /dev/null 2>&1; then
    break
  fi
  [ "$i" = "45" ] && fail "restarted B did not become healthy"
  sleep 2
done
pass "instance B restarted"

# The lease is ~60s; B's workers poll every ~2s after that. Allow generous
# slack for the reclaimed run over the seeded rows.
RECOVERED=""
for _ in $(seq 1 300); do
  state="$(pg "SELECT status FROM cluster_jobs WHERE id = '$JOB_ID'")"
  case "$state" in
    completed) RECOVERED=1; break ;;
    failed) fail "reclaimed job failed: $(pg "SELECT error_message FROM cluster_jobs WHERE id = '$JOB_ID'")" ;;
    *) sleep 1 ;;
  esac
done
[ -n "$RECOVERED" ] || fail "job did not complete within 300s of the kill"

NEW_WORKER="$(pg "SELECT worker_id FROM cluster_jobs WHERE id = '$JOB_ID'")"
[ "$NEW_WORKER" != "$ORPHAN_WORKER" ] || fail "the job completed under the dead worker's id — no reclaim happened"
NEW_TOKEN="$(pg "SELECT fencing_token FROM cluster_jobs WHERE id = '$JOB_ID'")"
[ "$NEW_TOKEN" -ge 2 ] || fail "expected a bumped fencing token after the reclaim, got $NEW_TOKEN"
pass "job reclaimed by $NEW_WORKER (fencing token $NEW_TOKEN) and completed"

# --- 6. The result is fully servable from B ----------------------------------
status="$(curl -sS -o "$HTTP_DIR/poll.out" -w "%{http_code}" -D "$HTTP_DIR/poll-headers.txt" \
  -H "Accept: $FHIR_CT" "$BASE_URL_B$(echo "$STATUS_URL" | sed 's|^[a-z]*://[^/]*||')")" \
  || fail "status poll via B failed"
[ "$status" = "303" ] || fail "status poll via B returned HTTP $status, expected 303"
RESULT_PATH="$(awk 'tolower($1) == "location:" {print $2}' "$HTTP_DIR/poll-headers.txt" | tr -d '\r' | tail -1 | sed 's|^[a-z]*://[^/]*||')"

status="$(curl -sS -o "$HTTP_DIR/manifest.json" -w "%{http_code}" -H "Accept: $FHIR_CT" "$BASE_URL_B$RESULT_PATH")" \
  || fail "manifest fetch via B failed"
[ "$status" = "200" ] || fail "manifest fetch via B returned HTTP $status, expected 200"

DOWNLOAD_PATH="$(grep -o '"valueUri"[[:space:]]*:[[:space:]]*"[^"]*"' "$HTTP_DIR/manifest.json" \
  | sed 's/.*"\(http[^"]*\)".*/\1/' | grep -v '/status$' | head -1 | sed 's|^[a-z]*://[^/]*||')"
[ -n "$DOWNLOAD_PATH" ] || fail "manifest has no output location"

status="$(curl -sS -o "$HTTP_DIR/shard.ndjson" -w "%{http_code}" "$BASE_URL_B$DOWNLOAD_PATH")" \
  || fail "shard download via B failed"
[ "$status" = "200" ] || fail "shard download via B returned HTTP $status, expected 200"

LINES="$(wc -l < "$HTTP_DIR/shard.ndjson" | tr -d ' ')"
[ "$LINES" -ge "$SEED_ROWS" ] || fail "shard has $LINES rows, expected >= $SEED_ROWS — the reclaimed run lost data"
grep -q '"kill9-1"' "$HTTP_DIR/shard.ndjson" || fail "shard is missing seeded patients"
pass "shard downloaded via B with $LINES rows — nothing lost across the kill"

log "nightly kill-9 recovery check passed"
echo "" >> "$SUMMARY_FILE"
echo "Kill-9 recovery check passed." >> "$SUMMARY_FILE"
