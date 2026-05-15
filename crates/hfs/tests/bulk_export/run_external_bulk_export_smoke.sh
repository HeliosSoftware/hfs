#!/usr/bin/env bash
set -euo pipefail

BASE_URL="${BASE_URL:-http://localhost:8080}"
FHIR_VERSION="${FHIR_VERSION:-R4}"
RESULTS_DIR="${RESULTS_DIR:-bulk-export-smoke-results}"
SMOKE_RUN_SUFFIX="${SMOKE_RUN_SUFFIX:-local-$(date +%s)-$$}"
BULK_EXPORT_EXPECTATION="${BULK_EXPORT_EXPECTATION:-full}"
EXPECT_REQUIRES_ACCESS_TOKEN="${EXPECT_REQUIRES_ACCESS_TOKEN:-true}"

HTTP_DIR="$RESULTS_DIR/http"
MANIFEST_DIR="$RESULTS_DIR/manifests"
NDJSON_DIR="$RESULTS_DIR/ndjson"
SUMMARY_FILE="$RESULTS_DIR/summary.md"

mkdir -p "$HTTP_DIR" "$MANIFEST_DIR" "$NDJSON_DIR"

log() {
  echo "[bulk-export-smoke] $*"
}

fail() {
  local msg="$1"
  echo "[bulk-export-smoke] ERROR: $msg" >&2
  mkdir -p "$(dirname "$SUMMARY_FILE")"
  echo "- FAIL: $msg" >> "$SUMMARY_FILE"
  if [ -n "${HFS_LOG:-}" ] && [ -f "$HFS_LOG" ]; then
    echo "---- hfs log (tail) ----" >&2
    tail -n 160 "$HFS_LOG" >&2 || true
    echo "------------------------" >&2
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

expect_status() {
  local actual="$1"
  local expected="$2"
  local operation="$3"
  local response_file="$4"
  if [ "$actual" != "$expected" ]; then
    echo "---- $operation response ----" >&2
    cat "$response_file" >&2 || true
    echo "----------------------------" >&2
    fail "$operation returned HTTP $actual, expected $expected"
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

case "$FHIR_VERSION" in
  R4) FHIR_MIME_VERSION="4.0" ;;
  R4B) FHIR_MIME_VERSION="4.3" ;;
  R5) FHIR_MIME_VERSION="5.0" ;;
  *) fail "unsupported FHIR_VERSION: $FHIR_VERSION (expected R4, R4B, or R5)" ;;
esac

FHIR_CT="application/fhir+json; fhirVersion=$FHIR_MIME_VERSION"
FHIR_ACCEPT="$FHIR_CT"
ID_SUFFIX="$(printf '%s' "$SMOKE_RUN_SUFFIX-$FHIR_VERSION" | tr -cs '[:alnum:]-' '-' | sed -e 's/^-*//' -e 's/-*$//')"
if [ -z "$ID_SUFFIX" ]; then
  ID_SUFFIX="bulk-smoke"
fi

PATIENT_A="bulk-smoke-patient-a-$ID_SUFFIX"
PATIENT_B="bulk-smoke-patient-b-$ID_SUFFIX"
PATIENT_C="bulk-smoke-patient-c-$ID_SUFFIX"
OBS_A="bulk-smoke-observation-a-$ID_SUFFIX"
OBS_B="bulk-smoke-observation-b-$ID_SUFFIX"
OBS_C="bulk-smoke-observation-c-$ID_SUFFIX"
GROUP_ID="bulk-smoke-group-$ID_SUFFIX"

write_summary_header() {
  cat > "$SUMMARY_FILE" <<EOF
## Bulk Export Smoke Test

- Base URL: \`$BASE_URL\`
- FHIR version: \`$FHIR_VERSION\`
- Expectation: \`$BULK_EXPORT_EXPECTATION\`
- Run suffix: \`$SMOKE_RUN_SUFFIX\`

EOF
}

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
      -H "Accept: $FHIR_ACCEPT" \
      "$@" \
      --data-binary @"$body_file"
  else
    curl -sS -o "$output_file" -w "%{http_code}" \
      -X "$method" "$url" \
      -H "Accept: $FHIR_ACCEPT" \
      "$@"
  fi
}

seed_resource() {
  local resource_type="$1"
  local id="$2"
  local file="$3"
  local response="$HTTP_DIR/put-$resource_type-$id.json"
  local status
  status="$(curl_json PUT "$BASE_URL/$resource_type/$id" "$file" "$response")"
  expect_created "$status" "PUT $resource_type/$id" "$response"
}

seed_data() {
  log "Seeding Patients, Observations, and Group"

  cat > "$HTTP_DIR/patient-a.json" <<EOF
{"resourceType":"Patient","id":"$PATIENT_A","name":[{"family":"BulkSmoke","given":["Alpha"]}],"gender":"female","birthDate":"1980-01-01"}
EOF
  cat > "$HTTP_DIR/patient-b.json" <<EOF
{"resourceType":"Patient","id":"$PATIENT_B","name":[{"family":"BulkSmoke","given":["Beta"]}],"gender":"male","birthDate":"1981-02-02"}
EOF
  cat > "$HTTP_DIR/patient-c.json" <<EOF
{"resourceType":"Patient","id":"$PATIENT_C","name":[{"family":"BulkSmoke","given":["Gamma"]}],"gender":"other","birthDate":"1982-03-03"}
EOF
  cat > "$HTTP_DIR/observation-a.json" <<EOF
{"resourceType":"Observation","id":"$OBS_A","status":"final","code":{"text":"Bulk smoke A"},"subject":{"reference":"Patient/$PATIENT_A"},"effectiveDateTime":"2024-01-01T00:00:00Z","valueString":"alpha"}
EOF
  cat > "$HTTP_DIR/observation-b.json" <<EOF
{"resourceType":"Observation","id":"$OBS_B","status":"final","code":{"text":"Bulk smoke B"},"subject":{"reference":"Patient/$PATIENT_B"},"effectiveDateTime":"2024-01-02T00:00:00Z","valueString":"beta"}
EOF
  cat > "$HTTP_DIR/observation-c.json" <<EOF
{"resourceType":"Observation","id":"$OBS_C","status":"final","code":{"text":"Bulk smoke C"},"subject":{"reference":"Patient/$PATIENT_C"},"effectiveDateTime":"2024-01-03T00:00:00Z","valueString":"gamma"}
EOF
  cat > "$HTTP_DIR/group.json" <<EOF
{"resourceType":"Group","id":"$GROUP_ID","type":"person","actual":true,"name":"Bulk Export Smoke Group","member":[{"entity":{"reference":"Patient/$PATIENT_A"}},{"entity":{"reference":"Patient/$PATIENT_B"}}]}
EOF

  seed_resource Patient "$PATIENT_A" "$HTTP_DIR/patient-a.json"
  seed_resource Patient "$PATIENT_B" "$HTTP_DIR/patient-b.json"
  seed_resource Patient "$PATIENT_C" "$HTTP_DIR/patient-c.json"
  seed_resource Observation "$OBS_A" "$HTTP_DIR/observation-a.json"
  seed_resource Observation "$OBS_B" "$HTTP_DIR/observation-b.json"
  seed_resource Observation "$OBS_C" "$HTTP_DIR/observation-c.json"
  seed_resource Group "$GROUP_ID" "$HTTP_DIR/group.json"
  pass "seeded FHIR resources through REST"
}

assert_metadata_advertises_export() {
  local response="$HTTP_DIR/metadata.json"
  local status
  status="$(curl_json GET "$BASE_URL/metadata" "" "$response")"
  expect_status "$status" "200" "GET /metadata" "$response"
  jq -e '
    [.rest[]?.operation[]?.name] as $names
    | ($names | index("export"))
    and ($names | index("patient-export"))
    and ($names | index("group-export"))
  ' "$response" >/dev/null || fail "CapabilityStatement does not advertise all bulk export operations"
  pass "CapabilityStatement advertises bulk export operations"
}

assert_no_bulk_export_endpoint() {
  local response="$HTTP_DIR/export-unavailable.response"
  local status
  status="$(curl -sS -o "$response" -w "%{http_code}" \
    -H "Prefer: respond-async" \
    -H "Accept: $FHIR_ACCEPT" \
    "$BASE_URL/\$export?_type=Patient")"
  case "$status" in
    400|404|501|500)
      pass "bulk export endpoint is unavailable as expected (HTTP $status)"
      ;;
    *)
      cat "$response" >&2 || true
      fail "expected bulk export endpoint to be unavailable, got HTTP $status"
      ;;
  esac
}

assert_requires_respond_async() {
  local response="$HTTP_DIR/export-missing-prefer.response"
  local status
  status="$(curl -sS -o "$response" -w "%{http_code}" \
    -H "Accept: $FHIR_ACCEPT" \
    "$BASE_URL/\$export?_type=Patient")"
  expect_status "$status" "400" "GET /\$export without Prefer" "$response"
  pass "kickoff requires Prefer: respond-async"
}

kickoff_get() {
  local label="$1"
  local path="$2"
  local response="$HTTP_DIR/$label-kickoff.response"
  local headers="$HTTP_DIR/$label-kickoff.headers"
  local status
  status="$(curl -sS -D "$headers" -o "$response" -w "%{http_code}" \
    -H "Prefer: respond-async" \
    -H "Accept: $FHIR_ACCEPT" \
    "$BASE_URL$path")"
  expect_status "$status" "202" "$label kickoff" "$response"
  local content_location
  content_location="$(awk 'BEGIN{IGNORECASE=1} /^Content-Location:/ {sub(/\r$/, "", $0); sub(/^[^:]+:[[:space:]]*/, "", $0); print; exit}' "$headers")"
  if [ -z "$content_location" ]; then
    fail "$label kickoff did not return Content-Location"
  fi
  printf '%s\n' "$content_location"
}

kickoff_patient_post() {
  local label="$1"
  local patient_ref="$2"
  local body="$HTTP_DIR/$label-parameters.json"
  local response="$HTTP_DIR/$label-kickoff.response"
  local headers="$HTTP_DIR/$label-kickoff.headers"
  cat > "$body" <<EOF
{"resourceType":"Parameters","parameter":[{"name":"_type","valueString":"Patient,Observation"},{"name":"patient","valueReference":{"reference":"$patient_ref"}}]}
EOF
  local status
  status="$(curl -sS -D "$headers" -o "$response" -w "%{http_code}" \
    -X POST "$BASE_URL/Patient/\$export" \
    -H "Prefer: respond-async" \
    -H "Content-Type: $FHIR_CT" \
    -H "Accept: $FHIR_ACCEPT" \
    --data-binary @"$body")"
  expect_status "$status" "202" "$label kickoff" "$response"
  local content_location
  content_location="$(awk 'BEGIN{IGNORECASE=1} /^Content-Location:/ {sub(/\r$/, "", $0); sub(/^[^:]+:[[:space:]]*/, "", $0); print; exit}' "$headers")"
  if [ -z "$content_location" ]; then
    fail "$label kickoff did not return Content-Location"
  fi
  printf '%s\n' "$content_location"
}

poll_manifest() {
  local label="$1"
  local status_url="$2"
  local manifest="$MANIFEST_DIR/$label-manifest.json"
  local response="$HTTP_DIR/$label-status.response"
  local status=""

  for i in $(seq 1 90); do
    status="$(curl -sS -o "$response" -w "%{http_code}" \
      -H "Accept: application/json" \
      "$status_url")"
    if [ "$status" = "200" ]; then
      cp "$response" "$manifest"
      printf '%s\n' "$manifest"
      return 0
    fi
    if [ "$status" != "202" ]; then
      cat "$response" >&2 || true
      fail "$label status returned HTTP $status before completion"
    fi
    sleep 2
  done

  fail "$label export did not complete before timeout"
}

expect_export_failure() {
  local label="$1"
  local path="$2"
  local response="$HTTP_DIR/$label-expected-failure.response"
  local headers="$HTTP_DIR/$label-expected-failure.headers"
  local status
  status="$(curl -sS -D "$headers" -o "$response" -w "%{http_code}" \
    -H "Prefer: respond-async" \
    -H "Accept: $FHIR_ACCEPT" \
    "$BASE_URL$path")"

  case "$status" in
    400|404|500|501)
      pass "$label failed immediately as expected (HTTP $status)"
      return 0
      ;;
    202)
      ;;
    *)
      cat "$response" >&2 || true
      fail "$label returned unexpected HTTP $status for expected-negative export"
      ;;
  esac

  local status_url
  status_url="$(awk 'BEGIN{IGNORECASE=1} /^Content-Location:/ {sub(/\r$/, "", $0); sub(/^[^:]+:[[:space:]]*/, "", $0); print; exit}' "$headers")"
  if [ -z "$status_url" ]; then
    fail "$label expected-negative kickoff returned 202 without Content-Location"
  fi

  for _ in $(seq 1 45); do
    status="$(curl -sS -o "$response" -w "%{http_code}" \
      -H "Accept: application/json" \
      "$status_url")"
    case "$status" in
      202)
        sleep 2
        ;;
      500|501|400|404)
        pass "$label reached expected failure state (HTTP $status)"
        return 0
        ;;
      200)
        cat "$response" >&2 || true
        fail "$label unexpectedly completed successfully"
        ;;
      *)
        cat "$response" >&2 || true
        fail "$label reached unexpected status HTTP $status"
        ;;
    esac
  done

  fail "$label expected-negative export did not fail before timeout"
}

download_outputs() {
  local label="$1"
  local manifest="$2"
  local merged="$NDJSON_DIR/$label-all.ndjson"
  : > "$merged"

  local output_count
  output_count="$(jq '.output | length' "$manifest")"
  if [ "$output_count" -lt 1 ]; then
    fail "$label manifest has no output files"
  fi

  local requires_token
  requires_token="$(jq -r '.requiresAccessToken' "$manifest")"
  if [ "$requires_token" != "$EXPECT_REQUIRES_ACCESS_TOKEN" ]; then
    fail "$label manifest requiresAccessToken=$requires_token, expected $EXPECT_REQUIRES_ACCESS_TOKEN"
  fi

  local idx=0
  while [ "$idx" -lt "$output_count" ]; do
    local url
    local resource_type
    local file
    url="$(jq -r ".output[$idx].url" "$manifest")"
    resource_type="$(jq -r ".output[$idx].type" "$manifest")"
    file="$NDJSON_DIR/$label-$idx-$resource_type.ndjson"
    curl -sS -o "$file" "$url"
    if [ -s "$file" ]; then
      while IFS= read -r line; do
        [ -z "$line" ] && continue
        printf '%s\n' "$line" | jq -e --arg rt "$resource_type" '.resourceType == $rt' >/dev/null \
          || fail "$label output $idx contains invalid JSON or wrong resourceType"
      done < "$file"
      cat "$file" >> "$merged"
    fi
    idx=$((idx + 1))
  done

  if [ ! -s "$merged" ]; then
    fail "$label downloaded outputs were empty"
  fi
  printf '%s\n' "$merged"
}

assert_ids() {
  local label="$1"
  local ndjson="$2"
  shift 2
  local expected
  for expected in "$@"; do
    jq -e --arg id "$expected" 'select(.id == $id)' "$ndjson" >/dev/null \
      || fail "$label output missing resource id $expected"
  done
}

assert_absent_ids() {
  local label="$1"
  local ndjson="$2"
  shift 2
  local unexpected
  for unexpected in "$@"; do
    if jq -e --arg id "$unexpected" 'select(.id == $id)' "$ndjson" >/dev/null; then
      fail "$label output unexpectedly included resource id $unexpected"
    fi
  done
}

assert_type_counts() {
  local label="$1"
  local manifest="$2"
  local patient_min="$3"
  local observation_min="$4"
  local patient_count
  local observation_count
  patient_count="$(jq '[.output[] | select(.type == "Patient") | .count // 0] | add // 0' "$manifest")"
  observation_count="$(jq '[.output[] | select(.type == "Observation") | .count // 0] | add // 0' "$manifest")"
  if [ "$patient_count" -lt "$patient_min" ]; then
    fail "$label manifest Patient count $patient_count is below expected minimum $patient_min"
  fi
  if [ "$observation_count" -lt "$observation_min" ]; then
    fail "$label manifest Observation count $observation_count is below expected minimum $observation_min"
  fi
}

run_full_lifecycle() {
  assert_metadata_advertises_export
  assert_requires_respond_async
  seed_data

  local status_url manifest ndjson

  log "Running system export"
  status_url="$(kickoff_get system "/\$export?_type=Patient,Observation")"
  manifest="$(poll_manifest system "$status_url")"
  assert_type_counts system "$manifest" 3 3
  ndjson="$(download_outputs system "$manifest")"
  assert_ids system "$ndjson" "$PATIENT_A" "$PATIENT_B" "$PATIENT_C" "$OBS_A" "$OBS_B" "$OBS_C"
  pass "system export completed and downloaded expected resources"

  log "Running patient export"
  status_url="$(kickoff_patient_post patient "Patient/$PATIENT_A")"
  manifest="$(poll_manifest patient "$status_url")"
  assert_type_counts patient "$manifest" 1 1
  ndjson="$(download_outputs patient "$manifest")"
  assert_ids patient "$ndjson" "$PATIENT_A" "$OBS_A"
  assert_absent_ids patient "$ndjson" "$PATIENT_B" "$PATIENT_C" "$OBS_B" "$OBS_C"
  pass "patient export scoped to requested patient"

  log "Running group export"
  status_url="$(kickoff_get group "/Group/$GROUP_ID/\$export?_type=Patient,Observation")"
  manifest="$(poll_manifest group "$status_url")"
  assert_type_counts group "$manifest" 2 2
  ndjson="$(download_outputs group "$manifest")"
  assert_ids group "$ndjson" "$PATIENT_A" "$PATIENT_B" "$OBS_A" "$OBS_B"
  assert_absent_ids group "$ndjson" "$PATIENT_C" "$OBS_C"
  pass "group export scoped to group members"

  local delete_response="$HTTP_DIR/system-delete.response"
  local delete_status
  delete_status="$(curl -sS -o "$delete_response" -w "%{http_code}" -X DELETE "$status_url")"
  expect_status "$delete_status" "202" "DELETE final export status URL" "$delete_response"

  local gone_response="$HTTP_DIR/final-status-after-delete.response"
  local gone_status
  gone_status="$(curl -sS -o "$gone_response" -w "%{http_code}" "$status_url")"
  expect_status "$gone_status" "404" "GET final export status URL after delete" "$gone_response"
  pass "export delete endpoint accepted cleanup request and removed status URL"
}

run_expected_negative() {
  seed_data
  expect_export_failure system "/\$export?_type=Patient,Observation"
}

main() {
  require_cmd curl
  require_cmd jq
  write_summary_header

  case "$BULK_EXPORT_EXPECTATION" in
    full)
      run_full_lifecycle
      ;;
    unsupported)
      run_expected_negative
      ;;
    endpoint-unavailable)
      assert_no_bulk_export_endpoint
      ;;
    *)
      fail "unknown BULK_EXPORT_EXPECTATION: $BULK_EXPORT_EXPECTATION"
      ;;
  esac

  echo "" >> "$SUMMARY_FILE"
  echo "All bulk export smoke checks completed for expectation \`$BULK_EXPORT_EXPECTATION\`." >> "$SUMMARY_FILE"
  log "Bulk export smoke test completed successfully"
}

main "$@"
