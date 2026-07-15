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

# --- 5. Subscriptions WS fan-out (Phase 3, #170): socket on B, write on A ---
# The one bug no single-process tier can catch (strategy §8, B1): a WebSocket
# client bound to instance B must receive the notification for a matching
# resource write served by instance A, via the pg-notify fan-out. Along the
# way this also proves B2 (binding token minted on A, redeemed on B) and B3
# (the Subscription created via A activates and answers $status via B) — and
# the sticky-session negative: the single-use token must NOT redeem again on
# another instance.

WEBSOCAT_BIN="${WEBSOCAT_BIN:?WEBSOCAT_BIN must point at a websocat binary for check 5}"
command -v jq >/dev/null 2>&1 || fail "check 5 requires jq"

WS_DIR="$RESULTS_DIR/ws"
mkdir -p "$WS_DIR"

WS_TOPIC_ID="smoke-ws-topic-$ID_SUFFIX"
WS_SUB_ID="smoke-ws-sub-$ID_SUFFIX"
WS_TOPIC_URL="http://example.org/cluster-smoke/topic/encounter-$ID_SUFFIX"
WS_ENCOUNTER_ID="smoke-ws-encounter-$ID_SUFFIX"

# The manager builds notifications per the FHIR version: R4 uses the backport
# IG shapes (Basic topic + criteria/channel), R4B+ the native resources.
if [ "$FHIR_VERSION" = "R4" ]; then
  EXPECTED_WS_BUNDLE_TYPE="history"
  cat > "$HTTP_DIR/ws-topic.request.json" <<EOF
{
  "resourceType": "Basic",
  "id": "$WS_TOPIC_ID",
  "code": {
    "coding": [{
      "system": "http://hl7.org/fhir/fhir-types",
      "code": "SubscriptionTopic"
    }]
  },
  "extension": [{
    "url": "http://hl7.org/fhir/5.0/StructureDefinition/extension-SubscriptionTopic.url",
    "valueUri": "$WS_TOPIC_URL"
  }, {
    "url": "http://hl7.org/fhir/4.3/StructureDefinition/extension-SubscriptionTopic.resourceTrigger",
    "extension": [{
      "url": "resource",
      "valueUri": "http://hl7.org/fhir/StructureDefinition/Encounter"
    }, {
      "url": "supportedInteraction",
      "valueCode": "create"
    }]
  }]
}
EOF
  WS_TOPIC_ENDPOINT="Basic/$WS_TOPIC_ID"
  cat > "$HTTP_DIR/ws-subscription.request.json" <<EOF
{
  "resourceType": "Subscription",
  "id": "$WS_SUB_ID",
  "status": "requested",
  "reason": "cluster smoke websocket fan-out",
  "criteria": "$WS_TOPIC_URL",
  "channel": {
    "type": "websocket",
    "payload": "application/fhir+json"
  }
}
EOF
else
  if [ "$FHIR_VERSION" = "R4B" ]; then
    EXPECTED_WS_BUNDLE_TYPE="history"
  else
    EXPECTED_WS_BUNDLE_TYPE="subscription-notification"
  fi
  cat > "$HTTP_DIR/ws-topic.request.json" <<EOF
{
  "resourceType": "SubscriptionTopic",
  "id": "$WS_TOPIC_ID",
  "url": "$WS_TOPIC_URL",
  "status": "active",
  "resourceTrigger": [{
    "resource": "Encounter",
    "supportedInteraction": ["create"]
  }]
}
EOF
  WS_TOPIC_ENDPOINT="SubscriptionTopic/$WS_TOPIC_ID"
  cat > "$HTTP_DIR/ws-subscription.request.json" <<EOF
{
  "resourceType": "Subscription",
  "id": "$WS_SUB_ID",
  "status": "requested",
  "topic": "$WS_TOPIC_URL",
  "channelType": { "code": "websocket" },
  "contentType": "application/fhir+json",
  "content": "id-only"
}
EOF
fi

NOTIFICATION_TYPE_JQ='if .entry[0].resource.resourceType=="Parameters" then ([.entry[0].resource.parameter[]? | select(.name=="type") | .valueCode][0] // "") else (.entry[0].resource.type // "") end'

wait_for_ws_frame() {
  local jq_filter="$1" timeout_secs="$2" label="$3"
  for _ in $(seq 1 "$timeout_secs"); do
    if [ "$( (jq -r "$NOTIFICATION_TYPE_JQ | select(.==\"$jq_filter\")" "$WS_FRAMES" 2>/dev/null || true) | sed '/^$/d' | wc -l | tr -d ' ')" -ge 1 ]; then
      return 0
    fi
    sleep 1
  done
  echo "---- websocket frames ($WS_FRAMES) ----" >&2
  cat "$WS_FRAMES" >&2 || true
  echo "---- websocat stderr ----" >&2
  cat "$WS_DIR/websocat.stderr" >&2 || true
  fail "timed out waiting for the websocket $label frame"
}

# Topic + Subscription created via instance A directly.
status="$(curl -sS -o "$HTTP_DIR/ws-topic.response.json" -w "%{http_code}" \
  -X PUT "$BASE_URL_A/$WS_TOPIC_ENDPOINT" \
  -H "Content-Type: $FHIR_CT" --data-binary @"$HTTP_DIR/ws-topic.request.json")"
case "$status" in
  200|201) ;;
  *) cat "$HTTP_DIR/ws-topic.response.json" >&2 || true
     fail "topic create via A returned HTTP $status" ;;
esac
status="$(curl -sS -o "$HTTP_DIR/ws-subscription.response.json" -w "%{http_code}" \
  -X PUT "$BASE_URL_A/Subscription/$WS_SUB_ID" \
  -H "Content-Type: $FHIR_CT" --data-binary @"$HTTP_DIR/ws-subscription.request.json")"
case "$status" in
  200|201) ;;
  *) cat "$HTTP_DIR/ws-subscription.response.json" >&2 || true
     fail "websocket Subscription create via A returned HTTP $status" ;;
esac

# B3/lifecycle propagation: instance B (which never saw the writes) must
# answer $status with the activated subscription.
SUB_STATUS=""
for _ in $(seq 1 30); do
  SUB_STATUS="$(curl -sS "$BASE_URL_B/Subscription/$WS_SUB_ID/\$status" 2>/dev/null \
    | jq -r 'if .resourceType=="Parameters" then ([.parameter[]? | select(.name=="status") | .valueCode][0] // "") else (.status // "") end' 2>/dev/null || true)"
  [ "$SUB_STATUS" = "active" ] && break
  sleep 1
done
if [ "$SUB_STATUS" != "active" ]; then
  fail "subscription created via A never became active on instance B (last status: '$SUB_STATUS')"
fi
pass "subscription created via instance A is active via instance B (lifecycle fan-out)"

# B2: token minted on A, socket bound on B.
status="$(curl -sS -o "$WS_DIR/token.response.json" -w "%{http_code}" \
  "$BASE_URL_A/Subscription/$WS_SUB_ID/\$get-ws-binding-token")"
[ "$status" = "200" ] || { cat "$WS_DIR/token.response.json" >&2 || true; fail "\$get-ws-binding-token via A returned HTTP $status"; }
WS_TOKEN="$(jq -r '.parameter[]? | select(.name=="token") | .valueString // empty' "$WS_DIR/token.response.json")"
[ -n "$WS_TOKEN" ] || fail "binding token missing from \$get-ws-binding-token response"

WS_URL_B="ws://${BASE_URL_B#http://}/ws/subscriptions/bind"
WS_FRAMES="$WS_DIR/frames.ndjson"
: > "$WS_FRAMES"
WS_INPUT_FIFO="$WS_DIR/ws-input.fifo"
rm -f "$WS_INPUT_FIFO"
mkfifo "$WS_INPUT_FIFO"
exec 3<>"$WS_INPUT_FIFO"

log "connecting websocket client to instance B ($WS_URL_B)"
timeout 90s "$WEBSOCAT_BIN" "$WS_URL_B" < "$WS_INPUT_FIFO" > "$WS_FRAMES" 2> "$WS_DIR/websocat.stderr" &
WS_PID="$!"
printf 'bind-with-token %s\n' "$WS_TOKEN" >&3

wait_for_ws_frame "handshake" 30 "handshake"
pass "binding token minted on A bound a socket on B (handshake received)"

# B1: the write lands on A; the socket lives on B.
cat > "$HTTP_DIR/ws-encounter.request.json" <<EOF
{
  "resourceType": "Encounter",
  "id": "$WS_ENCOUNTER_ID",
  "status": "in-progress"
}
EOF
status="$(curl -sS -o "$HTTP_DIR/ws-encounter.response.json" -w "%{http_code}" \
  -X PUT "$BASE_URL_A/Encounter/$WS_ENCOUNTER_ID" \
  -H "Content-Type: $FHIR_CT" --data-binary @"$HTTP_DIR/ws-encounter.request.json")"
case "$status" in
  200|201) ;;
  *) cat "$HTTP_DIR/ws-encounter.response.json" >&2 || true
     fail "Encounter create via A returned HTTP $status" ;;
esac

wait_for_ws_frame "event-notification" 45 "event-notification"
jq -c "select(($NOTIFICATION_TYPE_JQ)==\"event-notification\")" "$WS_FRAMES" \
  | head -n 1 > "$WS_DIR/event-notification.json"
jq -e --arg expected "$EXPECTED_WS_BUNDLE_TYPE" --arg focus "Encounter/$WS_ENCOUNTER_ID" \
  '.resourceType=="Bundle"
   and .type==$expected
   and any(.entry[]?; (.request.url // "") == $focus)' \
  "$WS_DIR/event-notification.json" >/dev/null \
  || { cat "$WS_DIR/event-notification.json" >&2 || true; fail "websocket event bundle missing expected shape/focus"; }
pass "encounter written to A delivered to the socket on B (WS fan-out)"

kill "$WS_PID" 2>/dev/null || true
wait "$WS_PID" 2>/dev/null || true
exec 3>&- || true
rm -f "$WS_INPUT_FIFO"

# Sticky-session negative: the token was consumed on B — presenting it again
# (to instance A this time) must be rejected, not silently rebound.
WS_URL_A="ws://${BASE_URL_A#http://}/ws/subscriptions/bind"
WS_REPLAY_FRAMES="$WS_DIR/replay-frames.ndjson"
: > "$WS_REPLAY_FRAMES"
if printf 'bind-with-token %s\n' "$WS_TOKEN" \
  | timeout 15s "$WEBSOCAT_BIN" "$WS_URL_A" > "$WS_REPLAY_FRAMES" 2> "$WS_DIR/websocat-replay.stderr"; then
  :
fi
if grep -q '"resourceType"' "$WS_REPLAY_FRAMES"; then
  cat "$WS_REPLAY_FRAMES" >&2
  fail "consumed binding token was accepted again on instance A (redeem-once violated)"
fi
pass "consumed binding token rejected on re-use against instance A (redeem-once)"

log "all cluster smoke checks passed"
echo "" >> "$SUMMARY_FILE"
echo "All checks passed." >> "$SUMMARY_FILE"
