#!/usr/bin/env bash
set -euo pipefail

BASE_URL="${BASE_URL:-http://localhost:8080}"
RESULTS_DIR="${RESULTS_DIR:-subscriptions-smoke-results}"
WEBSOCAT_BIN="${WEBSOCAT_BIN:-websocat}"
TOPIC_URL_BASE="${TOPIC_URL:-http://example.org/topic/encounter-start-smoke}"
SMOKE_RUN_SUFFIX="${SMOKE_RUN_SUFFIX:-local-$(date +%s)-$$}"
FHIR_VERSION="${FHIR_VERSION:-R4}"

HTTP_DIR="$RESULTS_DIR/http"
REST_DIR="$RESULTS_DIR/rest-hook"
WS_DIR="$RESULTS_DIR/ws"
SUMMARY_FILE="$RESULTS_DIR/summary.md"

case "$FHIR_VERSION" in
  R4)
    USE_BACKPORT=1
    EXPECTED_BUNDLE_TYPE="history"
    FHIR_MIME_VERSION="4.0"
    ;;
  R4B)
    USE_BACKPORT=0
    EXPECTED_BUNDLE_TYPE="history"
    FHIR_MIME_VERSION="4.3"
    ;;
  R5)
    USE_BACKPORT=0
    EXPECTED_BUNDLE_TYPE="subscription-notification"
    FHIR_MIME_VERSION="5.0"
    ;;
  R6)
    USE_BACKPORT=0
    EXPECTED_BUNDLE_TYPE="subscription-notification"
    FHIR_MIME_VERSION="6.0"
    ;;
  *)
    fail "unsupported FHIR_VERSION: $FHIR_VERSION (expected R4, R4B, R5, or R6)"
    ;;
esac
FHIR_CT="application/fhir+json; fhirVersion=$FHIR_MIME_VERSION"
FHIR_ACCEPT="$FHIR_CT"

NOTIFICATION_TYPE_JQ='if .entry[0].resource.resourceType=="Parameters" then ([.entry[0].resource.parameter[]? | select(.name=="type") | .valueCode][0] // "") else (.entry[0].resource.type // "") end'

mkdir -p "$HTTP_DIR" "$REST_DIR" "$WS_DIR"

log() {
  echo "[subscriptions-smoke] $*"
}

fail() {
  local msg="$1"
  echo "[subscriptions-smoke] ERROR: $msg" >&2
  mkdir -p "$(dirname "$SUMMARY_FILE")"
  echo "- FAIL: $msg" >> "$SUMMARY_FILE"
  if [ -n "${HFS_LOG:-}" ] && [ -f "$HFS_LOG" ]; then
    echo "---- hfs log (tail) ----" >&2
    tail -n 120 "$HFS_LOG" >&2 || true
    echo "------------------------" >&2
  fi
  if [ -n "${WEBHOOK_LOG:-}" ] && [ -f "$WEBHOOK_LOG" ]; then
    echo "---- webhook capture log (tail) ----" >&2
    tail -n 80 "$WEBHOOK_LOG" >&2 || true
    echo "------------------------------------" >&2
  fi
  exit 1
}

pass() {
  local msg="$1"
  echo "- PASS: $msg" >> "$SUMMARY_FILE"
}

require_cmd() {
  local cmd="$1"
  if ! command -v "$cmd" >/dev/null 2>&1; then
    fail "required command not found: $cmd"
  fi
}

expect_created() {
  local status="$1"
  local operation="$2"
  local response_file="$3"
  if [ "$status" != "200" ] && [ "$status" != "201" ]; then
    echo "---- $operation response ----" >&2
    cat "$response_file" >&2 || true
    echo "----------------------------" >&2
    fail "$operation returned unexpected HTTP status: $status"
  fi
}

expect_ok() {
  local status="$1"
  local operation="$2"
  local response_file="$3"
  if [ "$status" != "200" ]; then
    echo "---- $operation response ----" >&2
    cat "$response_file" >&2 || true
    echo "----------------------------" >&2
    fail "$operation returned unexpected HTTP status: $status"
  fi
}

wait_for_value_count() {
  local file="$1"
  local jq_expr="$2"
  local expected_min="$3"
  local timeout_secs="$4"
  local label="$5"
  local count=""

  for _ in $(seq 1 "$timeout_secs"); do
    count="$( (jq -r "$jq_expr" "$file" 2>/dev/null || true) | sed '/^$/d' | wc -l | tr -d ' ' )"
    if [ "${count:-0}" -ge "$expected_min" ]; then
      return 0
    fi
    sleep 1
  done

  if [ -f "$file" ]; then
    echo "---- $label source ($file) tail ----" >&2
    tail -n 40 "$file" >&2 || true
    echo "-----------------------------------" >&2
  fi
  fail "timed out waiting for $label in $file"
}

wait_for_health() {
  local url="$1"
  local timeout_secs="$2"
  for _ in $(seq 1 "$timeout_secs"); do
    if curl -sf "$url" >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  return 1
}

WEBHOOK_PID=""
WS_PID=""
WS_INPUT_FIFO=""
WS_INPUT_OPEN=0
cleanup() {
  if [ -n "${WS_PID:-}" ]; then
    kill "$WS_PID" 2>/dev/null || true
    wait "$WS_PID" 2>/dev/null || true
  fi
  if [ "${WS_INPUT_OPEN:-0}" -eq 1 ]; then
    exec 3>&- || true
    WS_INPUT_OPEN=0
  fi
  if [ -n "${WS_INPUT_FIFO:-}" ]; then
    rm -f "$WS_INPUT_FIFO"
  fi
  if [ -n "${WEBHOOK_PID:-}" ]; then
    kill "$WEBHOOK_PID" 2>/dev/null || true
    wait "$WEBHOOK_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

SUFFIX_SAFE="$(printf '%s' "$SMOKE_RUN_SUFFIX" | tr -cs '[:alnum:]-' '-' | sed -e 's/^-*//' -e 's/-*$//')"
if [ -z "$SUFFIX_SAFE" ]; then
  SUFFIX_SAFE="smoke"
fi
ID_SUFFIX="$(printf '%s' "$SUFFIX_SAFE" | cut -c1-24)"
TOPIC_URL="${TOPIC_URL_BASE}-${SUFFIX_SAFE}"

cat > "$SUMMARY_FILE" <<EOF
## Subscriptions Smoke Test

- Base URL: \`$BASE_URL\`
- Topic URL: \`$TOPIC_URL\`
- FHIR Version: \`$FHIR_VERSION\`
- Run Suffix: \`$SUFFIX_SAFE\`

EOF

require_cmd curl
require_cmd jq
require_cmd python3

if ! command -v "$WEBSOCAT_BIN" >/dev/null 2>&1 && [ ! -x "$WEBSOCAT_BIN" ]; then
  fail "websocket client not found: $WEBSOCAT_BIN"
fi

log "Starting local webhook capture service"
WEBHOOK_CAPTURE_DIR="$REST_DIR/capture"
WEBHOOK_LOG="$REST_DIR/webhook_capture.log"
mkdir -p "$WEBHOOK_CAPTURE_DIR"

webhook_started=0
for _ in $(seq 1 10); do
  WEBHOOK_PORT="$((20000 + (RANDOM % 15000)))"
  python3 ./crates/hfs/tests/subscriptions/webhook_capture.py \
    --host 127.0.0.1 \
    --port "$WEBHOOK_PORT" \
    --out-dir "$WEBHOOK_CAPTURE_DIR" > "$WEBHOOK_LOG" 2>&1 &
  WEBHOOK_PID="$!"

  if wait_for_health "http://127.0.0.1:$WEBHOOK_PORT/health" 3; then
    webhook_started=1
    break
  fi
  kill "$WEBHOOK_PID" 2>/dev/null || true
  wait "$WEBHOOK_PID" 2>/dev/null || true
  WEBHOOK_PID=""
done

if [ "$webhook_started" -ne 1 ]; then
  fail "unable to start webhook capture service"
fi
pass "webhook capture started on port $WEBHOOK_PORT"

TOPIC_ID="topic-smoke-$ID_SUFFIX"
REST_SUB_ID="sub-rest-$ID_SUFFIX"
WS_SUB_ID="sub-ws-$ID_SUFFIX"
REST_ENCOUNTER_ID="enc-rest-$ID_SUFFIX"
WS_ENCOUNTER_ID="enc-ws-$ID_SUFFIX"

if [ "$USE_BACKPORT" -eq 1 ]; then
  TOPIC_CREATE_ENDPOINT="Basic"
  cat > "$HTTP_DIR/topic.request.json" <<EOF
{
  "resourceType": "Basic",
  "id": "$TOPIC_ID",
  "code": {
    "coding": [{
      "system": "http://hl7.org/fhir/fhir-types",
      "code": "SubscriptionTopic"
    }]
  },
  "extension": [{
    "url": "http://hl7.org/fhir/5.0/StructureDefinition/extension-SubscriptionTopic.url",
    "valueUri": "$TOPIC_URL"
  }, {
    "url": "http://hl7.org/fhir/5.0/StructureDefinition/extension-SubscriptionTopic.title",
    "valueString": "Subscriptions Smoke Encounter Topic"
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
else
  TOPIC_CREATE_ENDPOINT="SubscriptionTopic"
  cat > "$HTTP_DIR/topic.request.json" <<EOF
{
  "resourceType": "SubscriptionTopic",
  "id": "$TOPIC_ID",
  "url": "$TOPIC_URL",
  "status": "active",
  "resourceTrigger": [{
    "resource": "Encounter",
    "supportedInteraction": ["create"]
  }]
}
EOF
fi

TOPIC_STATUS="$(curl -sS -o "$HTTP_DIR/topic.response.json" -w "%{http_code}" \
  -X POST "$BASE_URL/$TOPIC_CREATE_ENDPOINT" \
  -H "Content-Type: $FHIR_CT" \
  -H "Accept: $FHIR_ACCEPT" \
  --data-binary @"$HTTP_DIR/topic.request.json")"
expect_created "$TOPIC_STATUS" "create SubscriptionTopic" "$HTTP_DIR/topic.response.json"
pass "created topic for FHIR $FHIR_VERSION"

if [ "$USE_BACKPORT" -eq 1 ]; then
  cat > "$HTTP_DIR/rest-subscription.request.json" <<EOF
{
  "resourceType": "Subscription",
  "id": "$REST_SUB_ID",
  "status": "requested",
  "reason": "R4 backport rest-hook subscriptions smoke test",
  "meta": {
    "profile": [
      "http://hl7.org/fhir/uv/subscriptions-backport/StructureDefinition/backport-subscription"
    ]
  },
  "criteria": "$TOPIC_URL",
  "channel": {
    "type": "rest-hook",
    "endpoint": "http://127.0.0.1:$WEBHOOK_PORT/webhook",
    "payload": "application/fhir+json",
    "_payload": {
      "extension": [{
        "url": "http://hl7.org/fhir/uv/subscriptions-backport/StructureDefinition/backport-payload-content",
        "valueCode": "id-only"
      }]
    }
  }
}
EOF
else
  cat > "$HTTP_DIR/rest-subscription.request.json" <<EOF
{
  "resourceType": "Subscription",
  "id": "$REST_SUB_ID",
  "status": "requested",
  "reason": "Native rest-hook subscriptions smoke test",
  "topic": "$TOPIC_URL",
  "channelType": {
    "system": "http://terminology.hl7.org/CodeSystem/subscription-channel-type",
    "code": "rest-hook"
  },
  "endpoint": "http://127.0.0.1:$WEBHOOK_PORT/webhook",
  "contentType": "application/fhir+json",
  "content": "id-only",
  "parameter": [{
    "name": "Authorization",
    "value": "Bearer smoke-token"
  }]
}
EOF
fi

REST_SUB_STATUS="$(curl -sS -o "$HTTP_DIR/rest-subscription.response.json" -w "%{http_code}" \
  -X POST "$BASE_URL/Subscription" \
  -H "Content-Type: $FHIR_CT" \
  -H "Accept: $FHIR_ACCEPT" \
  --data-binary @"$HTTP_DIR/rest-subscription.request.json")"
expect_created "$REST_SUB_STATUS" "create rest-hook Subscription" "$HTTP_DIR/rest-subscription.response.json"

REST_CAPTURE_FILE="$WEBHOOK_CAPTURE_DIR/bodies.ndjson"
wait_for_value_count "$REST_CAPTURE_FILE" \
  "$NOTIFICATION_TYPE_JQ | select(.==\"handshake\")" \
  1 30 "rest-hook handshake notification"

cat > "$HTTP_DIR/rest-encounter.request.json" <<EOF
{
  "resourceType": "Encounter",
  "id": "$REST_ENCOUNTER_ID",
  "status": "in-progress"
}
EOF

REST_ENCOUNTER_STATUS="$(curl -sS -o "$HTTP_DIR/rest-encounter.response.json" -w "%{http_code}" \
  -X POST "$BASE_URL/Encounter" \
  -H "Content-Type: $FHIR_CT" \
  -H "Accept: $FHIR_ACCEPT" \
  --data-binary @"$HTTP_DIR/rest-encounter.request.json")"
expect_created "$REST_ENCOUNTER_STATUS" "create Encounter for rest-hook smoke" "$HTTP_DIR/rest-encounter.response.json"

wait_for_value_count "$REST_CAPTURE_FILE" \
  "$NOTIFICATION_TYPE_JQ | select(.==\"event-notification\")" \
  1 30 "rest-hook event-notification"

jq -c "select(($NOTIFICATION_TYPE_JQ)==\"handshake\")" \
  "$REST_CAPTURE_FILE" | head -n 1 > "$REST_DIR/handshake.json"
jq -c "select(($NOTIFICATION_TYPE_JQ)==\"event-notification\")" \
  "$REST_CAPTURE_FILE" | head -n 1 > "$REST_DIR/event-notification.json"

[ -s "$REST_DIR/handshake.json" ] || fail "missing captured rest-hook handshake bundle"
[ -s "$REST_DIR/event-notification.json" ] || fail "missing captured rest-hook event bundle"

jq -e --arg expected "$EXPECTED_BUNDLE_TYPE" \
  '.resourceType=="Bundle" and .type==$expected' "$REST_DIR/handshake.json" >/dev/null \
  || fail "rest-hook handshake bundle shape mismatch"
jq -e --arg expected_type "handshake" \
  "($NOTIFICATION_TYPE_JQ)==\$expected_type" \
  "$REST_DIR/handshake.json" >/dev/null || fail "rest-hook handshake type missing"
jq -e --arg expected "$EXPECTED_BUNDLE_TYPE" --arg expected_type "event-notification" --arg focus "Encounter/$REST_ENCOUNTER_ID" \
  ".resourceType==\"Bundle\"
   and .type==\$expected
   and (($NOTIFICATION_TYPE_JQ)==\$expected_type)
   and any(.entry[]?; (.request.url // \"\") == \$focus)" \
  "$REST_DIR/event-notification.json" >/dev/null || fail "rest-hook event bundle missing expected focus"

pass "rest-hook smoke assertions passed (handshake + event-notification)"

if [ "$USE_BACKPORT" -eq 1 ]; then
  cat > "$HTTP_DIR/ws-subscription.request.json" <<EOF
{
  "resourceType": "Subscription",
  "id": "$WS_SUB_ID",
  "status": "requested",
  "reason": "R4 backport websocket subscriptions smoke test",
  "meta": {
    "profile": [
      "http://hl7.org/fhir/uv/subscriptions-backport/StructureDefinition/backport-subscription"
    ]
  },
  "criteria": "$TOPIC_URL",
  "channel": {
    "type": "websocket",
    "payload": "application/fhir+json",
    "_payload": {
      "extension": [{
        "url": "http://hl7.org/fhir/uv/subscriptions-backport/StructureDefinition/backport-payload-content",
        "valueCode": "id-only"
      }]
    }
  }
}
EOF
else
  cat > "$HTTP_DIR/ws-subscription.request.json" <<EOF
{
  "resourceType": "Subscription",
  "id": "$WS_SUB_ID",
  "status": "requested",
  "reason": "Native websocket subscriptions smoke test",
  "topic": "$TOPIC_URL",
  "channelType": {
    "system": "http://terminology.hl7.org/CodeSystem/subscription-channel-type",
    "code": "websocket"
  },
  "contentType": "application/fhir+json",
  "content": "id-only"
}
EOF
fi

WS_SUB_STATUS="$(curl -sS -o "$HTTP_DIR/ws-subscription.response.json" -w "%{http_code}" \
  -X POST "$BASE_URL/Subscription" \
  -H "Content-Type: $FHIR_CT" \
  -H "Accept: $FHIR_ACCEPT" \
  --data-binary @"$HTTP_DIR/ws-subscription.request.json")"
expect_created "$WS_SUB_STATUS" "create websocket Subscription" "$HTTP_DIR/ws-subscription.response.json"

WS_TOKEN_STATUS="$(curl -sS -o "$WS_DIR/token.response.json" -w "%{http_code}" \
  -H "Accept: $FHIR_ACCEPT" \
  "$BASE_URL/Subscription/$WS_SUB_ID/\$get-ws-binding-token")"
expect_ok "$WS_TOKEN_STATUS" "get websocket binding token" "$WS_DIR/token.response.json"

TOKEN="$(jq -r '.parameter[]? | select(.name=="token") | .valueString // empty' "$WS_DIR/token.response.json")"
WS_URL="$(jq -r '.parameter[]? | select(.name=="websocket-url") | .valueUrl // empty' "$WS_DIR/token.response.json")"
EXPIRATION="$(jq -r '.parameter[]? | select(.name=="expiration") | .valueDateTime // empty' "$WS_DIR/token.response.json")"

[ -n "$TOKEN" ] || fail "token missing from \$get-ws-binding-token response"
[ -n "$WS_URL" ] || fail "websocket-url missing from \$get-ws-binding-token response"
[ -n "$EXPIRATION" ] || fail "expiration missing from \$get-ws-binding-token response"
pass "websocket binding token response includes token, expiration, and websocket-url"

WS_FRAMES="$WS_DIR/frames.ndjson"
: > "$WS_FRAMES"
WS_INPUT_FIFO="$WS_DIR/ws-input.fifo"
rm -f "$WS_INPUT_FIFO"
mkfifo "$WS_INPUT_FIFO"
exec 3<>"$WS_INPUT_FIFO"
WS_INPUT_OPEN=1

log "Connecting websocket client"
timeout 45s "$WEBSOCAT_BIN" "$WS_URL" < "$WS_INPUT_FIFO" > "$WS_FRAMES" 2> "$WS_DIR/websocat.stderr" &
WS_PID="$!"
printf 'bind-with-token %s\n' "$TOKEN" >&3

wait_for_value_count "$WS_FRAMES" \
  "$NOTIFICATION_TYPE_JQ | select(.==\"handshake\")" \
  1 30 "websocket handshake frame"

cat > "$HTTP_DIR/ws-encounter.request.json" <<EOF
{
  "resourceType": "Encounter",
  "id": "$WS_ENCOUNTER_ID",
  "status": "in-progress"
}
EOF

WS_ENCOUNTER_STATUS="$(curl -sS -o "$HTTP_DIR/ws-encounter.response.json" -w "%{http_code}" \
  -X POST "$BASE_URL/Encounter" \
  -H "Content-Type: $FHIR_CT" \
  -H "Accept: $FHIR_ACCEPT" \
  --data-binary @"$HTTP_DIR/ws-encounter.request.json")"
expect_created "$WS_ENCOUNTER_STATUS" "create Encounter for websocket smoke" "$HTTP_DIR/ws-encounter.response.json"

wait_for_value_count "$WS_FRAMES" \
  "$NOTIFICATION_TYPE_JQ | select(.==\"event-notification\")" \
  1 30 "websocket event-notification frame"

jq -c "select(($NOTIFICATION_TYPE_JQ)==\"handshake\")" \
  "$WS_FRAMES" | head -n 1 > "$WS_DIR/handshake.json"
jq -c "select(($NOTIFICATION_TYPE_JQ)==\"event-notification\")" \
  "$WS_FRAMES" | head -n 1 > "$WS_DIR/event-notification.json"

[ -s "$WS_DIR/handshake.json" ] || fail "missing websocket handshake frame"
[ -s "$WS_DIR/event-notification.json" ] || fail "missing websocket event-notification frame"

jq -e --arg expected "$EXPECTED_BUNDLE_TYPE" \
  '.resourceType=="Bundle" and .type==$expected' "$WS_DIR/handshake.json" >/dev/null \
  || fail "websocket handshake bundle shape mismatch"
jq -e --arg expected_type "handshake" \
  "($NOTIFICATION_TYPE_JQ)==\$expected_type" \
  "$WS_DIR/handshake.json" >/dev/null || fail "websocket handshake type missing"
jq -e --arg expected "$EXPECTED_BUNDLE_TYPE" --arg expected_type "event-notification" --arg focus "Encounter/$WS_ENCOUNTER_ID" \
  ".resourceType==\"Bundle\"
   and .type==\$expected
   and (($NOTIFICATION_TYPE_JQ)==\$expected_type)
   and any(.entry[]?; (.request.url // \"\") == \$focus)" \
  "$WS_DIR/event-notification.json" >/dev/null || fail "websocket event bundle missing expected focus"

pass "websocket smoke assertions passed (token + bind + handshake + event-notification)"

if [ -n "${WS_PID:-}" ]; then
  kill "$WS_PID" 2>/dev/null || true
  wait "$WS_PID" 2>/dev/null || true
  WS_PID=""
fi
if [ "${WS_INPUT_OPEN:-0}" -eq 1 ]; then
  exec 3>&- || true
  WS_INPUT_OPEN=0
fi
rm -f "$WS_INPUT_FIFO"

REST_TOTAL="$(wc -l < "$REST_CAPTURE_FILE" 2>/dev/null | tr -d ' ')"
WS_TOTAL="$(wc -l < "$WS_FRAMES" 2>/dev/null | tr -d ' ')"

cat >> "$SUMMARY_FILE" <<EOF

- Rest-hook notifications captured: $REST_TOTAL
- WebSocket frames captured: $WS_TOTAL

All subscriptions smoke checks completed successfully.
EOF

log "Subscriptions smoke test completed successfully"
