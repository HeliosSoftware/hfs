#!/usr/bin/env bash
set -euo pipefail

BASE_URL="${BASE_URL:?BASE_URL is required}"
FULL_TOKEN="${FULL_TOKEN:?FULL_TOKEN is required}"
READONLY_TOKEN="${READONLY_TOKEN:?READONLY_TOKEN is required}"
AUDIT_FILE="${AUDIT_FILE:?AUDIT_FILE is required}"
RESULTS_DIR="${RESULTS_DIR:-audit-results}"

mkdir -p "$RESULTS_DIR/http" "$RESULTS_DIR/events" "$RESULTS_DIR/payloads"
RANGES_FILE="$RESULTS_DIR/interaction_ranges.tsv"
CONTEXT_FILE="$RESULTS_DIR/context.json"

echo -e "name\tmethod\tpath\texpected_status\tactual_status\tstart_line\tend_line" > "$RANGES_FILE"

line_count() {
  if [ -f "$AUDIT_FILE" ]; then
    wc -l < "$AUDIT_FILE" | tr -d ' '
  else
    echo "0"
  fi
}

status_matches() {
  local actual="$1"
  local allowed_csv="$2"
  IFS=',' read -r -a allowed <<< "$allowed_csv"
  for code in "${allowed[@]}"; do
    if [ "$actual" = "$code" ]; then
      return 0
    fi
  done
  return 1
}

capture_event_window() {
  local start_line="$1"
  local current
  current="$(line_count)"

  # Wait for at least one new event (audit logging is asynchronous)
  for _ in $(seq 1 100); do
    if [ "$current" -gt "$start_line" ]; then
      break
    fi
    sleep 0.1
    current="$(line_count)"
  done

  # If nothing new appeared, return early (validator will fail this interaction)
  if [ "$current" -le "$start_line" ]; then
    echo "$current"
    return
  fi

  # Give the sink a short settle window to flush sibling events for the same request
  local stable_loops=0
  local prev="$current"
  for _ in $(seq 1 40); do
    sleep 0.1
    current="$(line_count)"
    if [ "$current" = "$prev" ]; then
      stable_loops=$((stable_loops + 1))
      if [ "$stable_loops" -ge 3 ]; then
        break
      fi
    else
      stable_loops=0
      prev="$current"
    fi
  done

  echo "$current"
}

run_call() {
  local name="$1"
  local method="$2"
  local path="$3"
  local expected_status="$4"
  local token="$5"
  local content_type="${6:-}"
  local data_file="${7:-}"

  local start_line end_line status
  local body_file="$RESULTS_DIR/http/${name}.body"
  local status_file="$RESULTS_DIR/http/${name}.status"

  start_line="$(line_count)"

  local -a curl_args
  curl_args=(-sS -o "$body_file" -w "%{http_code}" -X "$method" "${BASE_URL}${path}")

  if [ -n "$token" ]; then
    curl_args+=(-H "Authorization: Bearer $token")
  fi
  if [ -n "$content_type" ]; then
    curl_args+=(-H "Content-Type: $content_type")
  fi
  if [ -n "$data_file" ]; then
    curl_args+=(--data-binary "@$data_file")
  fi

  status="$(curl "${curl_args[@]}")"
  printf "%s\n" "$status" > "$status_file"

  if ! status_matches "$status" "$expected_status"; then
    echo "ERROR: $name expected HTTP $expected_status but got $status" >&2
    echo "---- response body ($name) ----" >&2
    cat "$body_file" >&2 || true
    echo "---- end response body ----" >&2
    exit 1
  fi

  end_line="$(capture_event_window "$start_line")"

  if [ "$end_line" -gt "$start_line" ]; then
    sed -n "$((start_line + 1)),$((end_line))p" "$AUDIT_FILE" > "$RESULTS_DIR/events/${name}.ndjson"
  else
    : > "$RESULTS_DIR/events/${name}.ndjson"
  fi

  printf "%s\t%s\t%s\t%s\t%s\t%s\t%s\n" \
    "$name" "$method" "$path" "$expected_status" "$status" "$start_line" "$end_line" >> "$RANGES_FILE"
}

# Give startup audit emission a chance to flush before first interaction windowing
sleep 1

RUN_TAG="audit-$(date +%s)"

cat > "$RESULTS_DIR/payloads/create_patient.json" <<JSON
{
  "resourceType": "Patient",
  "name": [{"family": "${RUN_TAG}", "given": ["External", "Audit"]}],
  "active": true
}
JSON

run_call "missing_token_search" "GET" "/Patient" "401" ""
run_call "invalid_token_search" "GET" "/Patient" "401" "not.a.jwt"

run_call "create_patient" "POST" "/Patient" "201" "$FULL_TOKEN" "application/fhir+json" "$RESULTS_DIR/payloads/create_patient.json"
PATIENT_ID="$(jq -r '.id // empty' "$RESULTS_DIR/http/create_patient.body")"
if [ -z "$PATIENT_ID" ]; then
  echo "ERROR: create_patient response missing id" >&2
  cat "$RESULTS_DIR/http/create_patient.body" >&2 || true
  exit 1
fi

cat > "$RESULTS_DIR/payloads/create_observation.json" <<JSON
{
  "resourceType": "Observation",
  "status": "final",
  "code": {"text": "External Audit Observation"},
  "subject": {"reference": "Patient/${PATIENT_ID}"}
}
JSON

cat > "$RESULTS_DIR/payloads/update_patient.json" <<JSON
{
  "resourceType": "Patient",
  "id": "${PATIENT_ID}",
  "active": true,
  "name": [{"family": "${RUN_TAG}", "given": ["Updated"]}]
}
JSON

cat > "$RESULTS_DIR/payloads/patch_patient.json" <<JSON
{
  "active": false
}
JSON

cat > "$RESULTS_DIR/payloads/search_post.form" <<FORM
name=${RUN_TAG}
FORM

run_call "read_patient" "GET" "/Patient/${PATIENT_ID}" "200" "$FULL_TOKEN"
run_call "head_patient" "HEAD" "/Patient/${PATIENT_ID}" "200" "$FULL_TOKEN"
run_call "search_patient_get" "GET" "/Patient?family=${RUN_TAG}" "200" "$FULL_TOKEN"
run_call "search_patient_post" "POST" "/Patient/_search" "200" "$FULL_TOKEN" "application/x-www-form-urlencoded" "$RESULTS_DIR/payloads/search_post.form"
run_call "history_type" "GET" "/Patient/_history" "200" "$FULL_TOKEN"
run_call "history_system" "GET" "/_history" "200" "$FULL_TOKEN"
run_call "history_instance" "GET" "/Patient/${PATIENT_ID}/_history" "200" "$FULL_TOKEN"

run_call "create_observation" "POST" "/Observation" "201" "$FULL_TOKEN" "application/fhir+json" "$RESULTS_DIR/payloads/create_observation.json"
OBSERVATION_ID="$(jq -r '.id // empty' "$RESULTS_DIR/http/create_observation.body")"
if [ -z "$OBSERVATION_ID" ]; then
  echo "ERROR: create_observation response missing id" >&2
  cat "$RESULTS_DIR/http/create_observation.body" >&2 || true
  exit 1
fi

run_call "search_subject_query" "GET" "/Observation?subject=Patient/${PATIENT_ID}" "200" "$FULL_TOKEN"
run_call "search_patient_query" "GET" "/Observation?patient=Patient/${PATIENT_ID}" "200" "$FULL_TOKEN"
run_call "search_unresolved_query" "GET" "/Observation?code=1234-5" "200" "$FULL_TOKEN"

run_call "update_patient_put" "PUT" "/Patient/${PATIENT_ID}" "200" "$FULL_TOKEN" "application/fhir+json" "$RESULTS_DIR/payloads/update_patient.json"
run_call "patch_patient" "PATCH" "/Patient/${PATIENT_ID}" "200" "$FULL_TOKEN" "application/merge-patch+json" "$RESULTS_DIR/payloads/patch_patient.json"

run_call "readonly_denied_create" "POST" "/Observation" "403" "$READONLY_TOKEN" "application/fhir+json" "$RESULTS_DIR/payloads/create_observation.json"
run_call "options_execute" "OPTIONS" "/Patient" "405" "$FULL_TOKEN"

cat > "$RESULTS_DIR/payloads/batch_bundle.json" <<JSON
{
  "resourceType": "Bundle",
  "type": "batch",
  "entry": [
    {
      "request": {"method": "GET", "url": "Patient/${PATIENT_ID}"}
    },
    {
      "request": {"method": "POST", "url": "Observation"},
      "resource": {
        "resourceType": "Observation",
        "status": "final",
        "code": {"text": "Batch Observation"},
        "subject": {"reference": "Patient/${PATIENT_ID}"}
      }
    }
  ]
}
JSON

cat > "$RESULTS_DIR/payloads/transaction_bundle.json" <<JSON
{
  "resourceType": "Bundle",
  "type": "transaction",
  "entry": [
    {
      "request": {"method": "POST", "url": "Observation"},
      "resource": {
        "resourceType": "Observation",
        "status": "final",
        "code": {"text": "Transaction Observation"},
        "subject": {"reference": "Patient/${PATIENT_ID}"}
      }
    },
    {
      "request": {"method": "GET", "url": "Patient/${PATIENT_ID}"}
    }
  ]
}
JSON

run_call "batch_bundle" "POST" "/" "200" "$FULL_TOKEN" "application/fhir+json" "$RESULTS_DIR/payloads/batch_bundle.json"
run_call "transaction_bundle" "POST" "/" "200" "$FULL_TOKEN" "application/fhir+json" "$RESULTS_DIR/payloads/transaction_bundle.json"

run_call "delete_observation" "DELETE" "/Observation/${OBSERVATION_ID}" "204" "$FULL_TOKEN"
run_call "delete_patient" "DELETE" "/Patient/${PATIENT_ID}" "204" "$FULL_TOKEN"

# Persist context for the validator/report generator
cat > "$CONTEXT_FILE" <<JSON
{
  "base_url": "${BASE_URL}",
  "audit_file": "${AUDIT_FILE}",
  "patient_id": "${PATIENT_ID}",
  "observation_id": "${OBSERVATION_ID}",
  "run_tag": "${RUN_TAG}",
  "ranges_file": "${RANGES_FILE}"
}
JSON

echo "Interaction suite completed: patient=${PATIENT_ID}, observation=${OBSERVATION_ID}"
