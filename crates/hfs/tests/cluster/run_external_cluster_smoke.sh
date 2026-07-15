#!/usr/bin/env bash
set -euo pipefail

# Cluster smoke skeleton (T3 tier of docs/cluster-testing-strategy.md).
#
# Drives two hfs instances that share one Postgres, plus an nginx round-robin
# front. This skeleton only asserts behavior that is already cluster-safe on
# main (shared-Postgres CRUD visibility), calibrating the two-instance harness
# before any cluster-capable-state phase depends on it — the same
# calibrate-against-known-good principle the T2 harness uses
# (docs/cluster-testing-methodology.md §5). Phase 1+ extends this script with
# the real cross-instance assertions (SoF $export A→B, WS fan-out A→B).

BASE_URL_A="${BASE_URL_A:-http://localhost:18200}"
BASE_URL_B="${BASE_URL_B:-http://localhost:18201}"
FRONT_URL="${FRONT_URL:?FRONT_URL (nginx round-robin front) must be set}"
FHIR_VERSION="${FHIR_VERSION:-R4}"
RESULTS_DIR="${RESULTS_DIR:-cluster-smoke-results}"
SMOKE_RUN_SUFFIX="${SMOKE_RUN_SUFFIX:-local-$(date +%s)-$$}"

HTTP_DIR="$RESULTS_DIR/http"
SUMMARY_FILE="$RESULTS_DIR/summary.md"

mkdir -p "$HTTP_DIR"

log() {
  echo "[cluster-smoke] $*"
}

fail() {
  local msg="$1"
  echo "[cluster-smoke] ERROR: $msg" >&2
  mkdir -p "$(dirname "$SUMMARY_FILE")"
  echo "- FAIL: $msg" >> "$SUMMARY_FILE"
  for hfs_log in "${HFS_LOG_A:-}" "${HFS_LOG_B:-}"; do
    if [ -n "$hfs_log" ] && [ -f "$hfs_log" ]; then
      echo "---- $hfs_log (tail) ----" >&2
      tail -n 120 "$hfs_log" >&2 || true
      echo "-------------------------" >&2
    fi
  done
  exit 1
}

pass() {
  local msg="$1"
  log "PASS: $msg"
  echo "- PASS: $msg" >> "$SUMMARY_FILE"
}

case "$FHIR_VERSION" in
  R4) FHIR_MIME_VERSION="4.0" ;;
  R4B) FHIR_MIME_VERSION="4.3" ;;
  R5) FHIR_MIME_VERSION="5.0" ;;
  *) fail "unsupported FHIR_VERSION: $FHIR_VERSION (expected R4, R4B, or R5)" ;;
esac

FHIR_CT="application/fhir+json; fhirVersion=$FHIR_MIME_VERSION"

ID_SUFFIX="$(printf '%s' "$SMOKE_RUN_SUFFIX-$FHIR_VERSION" | tr -cs '[:alnum:]-' '-' | sed -e 's/^-*//' -e 's/-*$//')"
if [ -z "$ID_SUFFIX" ]; then
  ID_SUFFIX="cluster-smoke"
fi
PATIENT_ID="cluster-smoke-patient-$ID_SUFFIX"

cat > "$SUMMARY_FILE" <<EOF
## Cluster Smoke Test

- Instance A: \`$BASE_URL_A\`
- Instance B: \`$BASE_URL_B\`
- Front (nginx): \`$FRONT_URL\`
- FHIR version: \`$FHIR_VERSION\`
- Run suffix: \`$SMOKE_RUN_SUFFIX\`

EOF

curl_json() {
  local method="$1"
  local url="$2"
  local body_file="$3"
  local output_file="$4"
  shift 4

  if [ -n "$body_file" ]; then
    curl -sS -o "$output_file" -w "%{http_code}" \
      -X "$method" "$url" \
      -H "Content-Type: $FHIR_CT" \
      -H "Accept: $FHIR_CT" \
      "$@" \
      --data-binary @"$body_file"
  else
    curl -sS -o "$output_file" -w "%{http_code}" \
      -X "$method" "$url" \
      -H "Accept: $FHIR_CT" \
      "$@"
  fi
}

# --- 1. Health: both instances and the front answer -------------------------

for name in A B front; do
  case "$name" in
    A) url="$BASE_URL_A" ;;
    B) url="$BASE_URL_B" ;;
    front) url="$FRONT_URL" ;;
  esac
  status="$(curl -sS -o "$HTTP_DIR/health-$name.txt" -w "%{http_code}" "$url/health")" \
    || fail "health check request to $name ($url) failed"
  if [ "$status" != "200" ]; then
    fail "health check on $name returned HTTP $status, expected 200"
  fi
done
pass "health endpoint answers on instance A, instance B, and the nginx front"

# --- 2. Round-robin: the front alternates between both upstreams ------------
# nginx stamps X-Hfs-Upstream with \$upstream_addr; over several requests we
# must observe at least two distinct upstream addresses.

: > "$HTTP_DIR/upstreams.txt"
for i in $(seq 1 8); do
  curl -sS -D - -o /dev/null "$FRONT_URL/health" \
    | tr -d '\r' \
    | awk -F': ' 'tolower($1) == "x-hfs-upstream" { print $2 }' \
    >> "$HTTP_DIR/upstreams.txt" \
    || fail "round-robin probe $i via the front failed"
done
DISTINCT_UPSTREAMS="$(sort -u "$HTTP_DIR/upstreams.txt" | sed '/^$/d' | wc -l | tr -d ' ')"
if [ "$DISTINCT_UPSTREAMS" -lt 2 ]; then
  cat "$HTTP_DIR/upstreams.txt" >&2 || true
  fail "front only reached $DISTINCT_UPSTREAMS distinct upstream(s) over 8 requests, expected 2"
fi
pass "nginx front round-robins across both instances ($DISTINCT_UPSTREAMS distinct upstreams)"

# --- 3. Cross-instance visibility (calibration): write on A, read on B ------
# Plain CRUD on a shared Postgres primary is already cluster-safe; this proves
# the harness (two processes, one database) is wired correctly.

PATIENT_FILE="$HTTP_DIR/patient.json"
cat > "$PATIENT_FILE" <<EOF
{
  "resourceType": "Patient",
  "id": "$PATIENT_ID",
  "name": [{"family": "ClusterSmoke", "given": ["Skeleton"]}]
}
EOF

status="$(curl_json PUT "$BASE_URL_A/Patient/$PATIENT_ID" "$PATIENT_FILE" "$HTTP_DIR/put-a.json")"
if [ "$status" != "200" ] && [ "$status" != "201" ]; then
  cat "$HTTP_DIR/put-a.json" >&2 || true
  fail "PUT Patient on instance A returned HTTP $status, expected 200/201"
fi

status="$(curl_json GET "$BASE_URL_B/Patient/$PATIENT_ID" "" "$HTTP_DIR/get-b.json")"
if [ "$status" != "200" ]; then
  cat "$HTTP_DIR/get-b.json" >&2 || true
  fail "GET Patient on instance B returned HTTP $status, expected 200 (created via A)"
fi
if ! grep -q "\"$PATIENT_ID\"" "$HTTP_DIR/get-b.json"; then
  cat "$HTTP_DIR/get-b.json" >&2 || true
  fail "Patient read via instance B does not contain id $PATIENT_ID"
fi
pass "Patient created via instance A is readable via instance B"

status="$(curl_json GET "$FRONT_URL/Patient/$PATIENT_ID" "" "$HTTP_DIR/get-front.json")"
if [ "$status" != "200" ]; then
  cat "$HTTP_DIR/get-front.json" >&2 || true
  fail "GET Patient via the front returned HTTP $status, expected 200"
fi
pass "Patient is readable through the nginx front"

# --- 4. SoF $viewdefinition-export across instances (Phase 1, #169) ---------
# Jobs live on the shared cluster job store (HFS_JOB_STORE_BACKEND=database +
# HFS_EXPORT_CONTROLLER=database in the workflow), and shards in a shared
# export dir, so: kick off on A, poll the status URL through the front
# (either instance answers), and download the shard via B explicitly.

EXPORT_BODY="$HTTP_DIR/export-kickoff.json"
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

KICKOFF_HEADERS="$HTTP_DIR/export-kickoff-headers.txt"
status="$(curl -sS -o "$HTTP_DIR/export-kickoff.out" -w "%{http_code}" \
  -D "$KICKOFF_HEADERS" \
  -X POST "$BASE_URL_A/ViewDefinition/\$viewdefinition-export" \
  -H "Content-Type: $FHIR_CT" -H "Accept: $FHIR_CT" -H "Prefer: respond-async" \
  --data-binary @"$EXPORT_BODY")" || fail "SoF export kickoff curl failed"
if [ "$status" != "202" ]; then
  cat "$HTTP_DIR/export-kickoff.out" >&2 || true
  fail "SoF export kickoff on instance A returned HTTP $status, expected 202"
fi

STATUS_URL="$(awk 'tolower($1) == "content-location:" {print $2}' "$KICKOFF_HEADERS" | tr -d '\r' | tail -1)"
[ -n "$STATUS_URL" ] || fail "SoF export kickoff response has no Content-Location header"
case "$STATUS_URL" in
  "$FRONT_URL"*) ;;
  *) fail "status URL '$STATUS_URL' is not front-based (HFS_BASE_URL should be the front)" ;;
esac

# Poll through the front until the 303 redirect to the result URL.
RESULT_URL=""
for _ in $(seq 1 60); do
  POLL_HEADERS="$HTTP_DIR/export-poll-headers.txt"
  status="$(curl -sS -o "$HTTP_DIR/export-poll.out" -w "%{http_code}" \
    -D "$POLL_HEADERS" -H "Accept: $FHIR_CT" "$STATUS_URL")" || fail "SoF export poll curl failed"
  case "$status" in
    202) sleep 1 ;;
    303)
      RESULT_URL="$(awk 'tolower($1) == "location:" {print $2}' "$POLL_HEADERS" | tr -d '\r' | tail -1)"
      break
      ;;
    *)
      cat "$HTTP_DIR/export-poll.out" >&2 || true
      fail "SoF export poll via the front returned HTTP $status, expected 202/303"
      ;;
  esac
done
[ -n "$RESULT_URL" ] || fail "SoF export did not complete within 60s of polling via the front"
pass "SoF export kicked off on A completed while polling through the front"

status="$(curl_json GET "$RESULT_URL" "" "$HTTP_DIR/export-manifest.json")"
if [ "$status" != "200" ]; then
  cat "$HTTP_DIR/export-manifest.json" >&2 || true
  fail "SoF export manifest fetch returned HTTP $status, expected 200"
fi

# The manifest carries several valueUri params: a top-level `location` /
# `cancelUrl` (both point at the status endpoint) and, inside each `output`
# part, the shard's download `location`. Only the shard URL serves bytes —
# the status URLs answer 303 — so filter the status endpoints out rather
# than taking the first match.
DOWNLOAD_URL="$(grep -o '"valueUri"[[:space:]]*:[[:space:]]*"[^"]*"' "$HTTP_DIR/export-manifest.json" \
  | sed 's/.*"\(http[^"]*\)".*/\1/' | grep -v '/status$' | head -1)"
[ -n "$DOWNLOAD_URL" ] || { cat "$HTTP_DIR/export-manifest.json" >&2; fail "SoF export manifest has no output location"; }

# Rewrite the front-based download URL to target instance B directly: the
# shard was (possibly) written by the other instance's worker, so a 200 here
# proves the output storage is genuinely shared.
DOWNLOAD_PATH="${DOWNLOAD_URL#"$FRONT_URL"}"
status="$(curl -sS -o "$HTTP_DIR/export-shard.ndjson" -w "%{http_code}" "$BASE_URL_B$DOWNLOAD_PATH")" || fail "SoF export download curl failed"
if [ "$status" != "200" ]; then
  cat "$HTTP_DIR/export-shard.ndjson" >&2 || true
  fail "SoF export shard download via instance B returned HTTP $status, expected 200"
fi
if ! grep -q "\"$PATIENT_ID\"" "$HTTP_DIR/export-shard.ndjson"; then
  cat "$HTTP_DIR/export-shard.ndjson" >&2 || true
  fail "SoF export shard downloaded via B does not contain patient $PATIENT_ID"
fi
pass "SoF export shard downloaded via instance B contains the exported patient"

log "all cluster smoke checks passed"
echo "" >> "$SUMMARY_FILE"
echo "All checks passed." >> "$SUMMARY_FILE"
