#!/usr/bin/env bash
#
# Same-session observability A/B driver (issue #297).
#
# Runs the k6 tx-benchmark scenarios against the HTS server once per
# (round × arm), restarting the server between arms so each arm's
# HELIOS_OBS_MODE / RUST_LOG takes effect (the arm switch is read once into a
# process-global OnceLock, so it can only change across a restart). The
# terminology database is imported ONCE by the caller and left untouched: only
# the app process is restarted, so every arm measures the same warmed data with
# no cross-run drift.
#
# Two properties the whole comparison rests on, enforced here:
#   1. Interleaving. Arms are rotated per round (round-robin, not blocked
#      off×N,default×N,…) so slow within-session drift on the shared runner
#      spreads across all arms as honest variance instead of aliasing onto the
#      arm axis. The reporter pairs deltas WITHIN a round.
#   2. Warm start. Every restart leaves the in-process memo caches
#      (hierarchy/closure/$lookup/expansion) cold. A discarded warm-up pass
#      that exercises the exact measured request population primes them before
#      the measured pass, so an arm's first scenarios are not taxed with
#      cache-fill cost that has nothing to do with instrumentation.
#
# Results are namespaced per arm+round:
#   results/${RUN_ID}__${arm-slug}__r${round}/hts/${TEST}_vu${VU}.json
# The RUN_ID itself carries the arm+round, which also namespaces k6's internal
# handleSummary() output (results/${RUN_ID}/hts/benchmark/) — without that,
# successive arms silently overwrite each other's k6 summaries.
#
# Required environment (exported by the workflow):
#   HTS_BIN            path to the hts binary
#   PORT               server port for this backend leg
#   BACKEND            sqlite | postgres (for log/artifact naming)
#   RUN_ID            github.run_id (the base run identifier)
#   ARMS              space-separated arm list, in ladder order
#                     (e.g. "off default full full+probe")
#   ROUNDS            repeats per arm (integer >= 1)
#   TESTS_CSV         comma-separated test IDs (e.g. "LK01,VC01,EX01")
#   VUS_CSV           comma-separated VU levels (e.g. "1" or "1,10,50")
#   DURATION          measured duration per scenario (e.g. "30s")
#   WARMUP_DURATION   discarded warm-up duration per test (e.g. "10s")
#   LOG_DIR           directory for per-arm server logs (e.g. "/tmp")
#   HTS_DATABASE_URL, HTS_STORAGE_BACKEND  already exported by the caller
#
# Run from the tx-benchmark checkout (working-directory), so k6 script paths
# (k6/<FAMILY>/<TEST>.js) and results/ resolve correctly.
set -uo pipefail

# Fail the job if a whole arm never came up, but only AFTER every salvageable
# arm has run and its artifacts exist — one broken arm must not discard the
# others' good data.
ANY_ARM_FAILED=0

# ── arm → (HELIOS_OBS_MODE, RUST_LOG) ───────────────────────────────────────
# `full+probe` is Full plus the developer probe stdout logs re-enabled via
# RUST_LOG; every other arm leaves RUST_LOG unset so HTS_LOG_LEVEL=info stands.
arm_obs_mode() {
  case "$1" in
    off)        echo "off" ;;
    default)    echo "default" ;;
    no-span)    echo "no-span" ;;
    full)       echo "full" ;;
    full+probe) echo "full" ;;
    *)          echo "" ;;
  esac
}
arm_rust_log() {
  case "$1" in
    full+probe) echo "info,hts::probe=debug" ;;
    *)          echo "" ;;
  esac
}
# The ObsMode Debug string the server logs at boot, for the active-arm assertion.
arm_expected_mode_log() {
  case "$1" in
    off)        echo "Off" ;;
    default)    echo "Default" ;;
    no-span)    echo "NoSpan" ;;
    full)       echo "Full" ;;
    full+probe) echo "Full" ;;
    *)          echo "" ;;
  esac
}
# Filesystem-safe slug — NEVER let RUST_LOG (which contains `hts::probe`, i.e.
# colons) reach a path; colons in result paths have already bitten this repo.
arm_slug() {
  printf '%s' "$1" | tr '+' '-'
}

# ── server lifecycle ────────────────────────────────────────────────────────
SERVER_PID=""

wait_port_free() {
  for _ in $(seq 1 100); do
    if ! fuser "${PORT}/tcp" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.2
  done
  echo "WARN: port ${PORT} still bound after 20s"
  return 1
}

stop_server() {
  local pid="${1:-}"
  [ -z "$pid" ] && return 0
  kill "$pid" 2>/dev/null || true
  # Block until the process actually exits — a fire-and-forget kill is how a
  # previous arm's server ends up still holding the port when the next binds.
  for _ in $(seq 1 100); do
    kill -0 "$pid" 2>/dev/null || break
    sleep 0.2
  done
  kill -9 "$pid" 2>/dev/null || true
  fuser -k "${PORT}/tcp" 2>/dev/null || true
  wait_port_free || true
}

# start_server <arm> <logfile>; sets SERVER_PID. Returns non-zero on a failed
# readiness/active-arm check (caller isolates the arm rather than aborting).
start_server() {
  local arm="$1" logfile="$2"
  local obs rustlog expected started_epoch
  obs="$(arm_obs_mode "$arm")"
  rustlog="$(arm_rust_log "$arm")"
  expected="$(arm_expected_mode_log "$arm")"
  started_epoch=$(date +%s)

  echo "  starting server: HELIOS_OBS_MODE=${obs} RUST_LOG='${rustlog}'  -> ${logfile}"
  # RUST_LOG (when set) overrides HTS_LOG_LEVEL in the server's EnvFilter. It
  # must be genuinely UNSET for the non-probe arms, not set to "" — a set-but-
  # empty RUST_LOG makes the tracing EnvFilter enable nothing, which suppresses
  # the startup `obs_mode=` line this script greps to confirm the active arm.
  # `env -u RUST_LOG` guarantees it is absent (even if inherited); the probe arm
  # sets it explicitly.
  if [ -n "$rustlog" ]; then
    HTS_SERVER_PORT="$PORT" \
    HTS_LOG_LEVEL="info" \
    HELIOS_OBS_MODE="$obs" \
    RUST_LOG="$rustlog" \
      "$HTS_BIN" >"$logfile" 2>&1 &
  else
    HTS_SERVER_PORT="$PORT" \
    HTS_LOG_LEVEL="info" \
    HELIOS_OBS_MODE="$obs" \
      env -u RUST_LOG "$HTS_BIN" >"$logfile" 2>&1 &
  fi
  SERVER_PID=$!

  # Readiness — generous bound: inside the arm loop a cold start (and, on
  # sqlite pre-#295, an FTS rebuild) can take minutes.
  local ready=0
  for _ in $(seq 1 150); do
    if curl -sf "http://localhost:${PORT}/health" >/dev/null 2>&1; then
      ready=1
      break
    fi
    kill -0 "$SERVER_PID" 2>/dev/null || { echo "  ERROR: server process exited during startup"; break; }
    sleep 2
  done
  if [ "$ready" -ne 1 ]; then
    echo "  ERROR: server for arm '${arm}' did not become ready"
    tail -50 "$logfile" || true
    return 1
  fi

  # Zombie / wrong-arm guard: confirm we are talking to the process we just
  # started (a survivor on the port would serve every request of this arm under
  # the WRONG mode). /health carries uptime_seconds; it must be smaller than the
  # time we spent waiting.
  local elapsed uptime
  elapsed=$(( $(date +%s) - started_epoch + 15 ))
  uptime=$(curl -sf "http://localhost:${PORT}/health" 2>/dev/null \
           | jq -r '.uptime_seconds // empty' 2>/dev/null || true)
  if [ -n "$uptime" ] && [ "$uptime" -gt "$elapsed" ] 2>/dev/null; then
    echo "  ERROR: /health uptime_seconds=${uptime}s exceeds ${elapsed}s — a stale server may be answering on port ${PORT}"
    return 1
  fi

  # Active-arm assertion: the server stamps its resolved ObsMode into the
  # startup line (main.rs). If it does not match, the arm is inert (e.g. a
  # server built without the arm switch) and its numbers are meaningless.
  if [ -n "$expected" ]; then
    # Strip ANSI colour escapes before matching: a server writing to this file
    # may still colourize (older builds, or a forced-colour env), which would
    # otherwise split `obs_mode=Off` with escape sequences and fail the match.
    local plainlog
    plainlog="$(sed -E 's/\x1b\[[0-9;]*m//g' "$logfile")"
    if ! printf '%s' "$plainlog" | grep -Eq "obs_mode=${expected}\b"; then
      echo "  ERROR: server did not report obs_mode=${expected}; arm '${arm}' is not active."
      printf '%s' "$plainlog" | grep -i 'obs_mode' | head -3 \
        || echo "    (no obs_mode line found in startup log)"
      return 1
    fi
    echo "  active arm confirmed: obs_mode=${expected}"
  fi
  return 0
}

# ── k6 passes ───────────────────────────────────────────────────────────────
IFS=',' read -ra TESTS <<< "$TESTS_CSV"
IFS=',' read -ra VUS <<< "$VUS_CSV"

script_for() {  # <TEST> -> k6 script path, or empty if absent
  local t="${1// /}" fam
  fam="${t:0:2}"
  local s="k6/${fam}/${t}.js"
  [ -f "$s" ] && printf '%s' "$s" || printf ''
}

# Discarded warm-up: touch the exact measured request population once at VU=1 so
# the per-URL memo caches are primed identically for every arm before the
# measured pass. Output goes to a throwaway RUN_ID that is deleted afterwards.
warmup_pass() {
  local warm_run="warmup-${1}"
  rm -rf "results/${warm_run}"
  mkdir -p "results/${warm_run}/hts/benchmark"
  for TEST in "${TESTS[@]}"; do
    local s; s="$(script_for "$TEST")"
    [ -z "$s" ] && continue
    k6 run \
      --env BASE_URL="http://localhost:${PORT}" \
      --env SERVER_NAME=hts \
      --env RUN_ID="$warm_run" \
      --env TEST_ID="${TEST// /}" \
      --env VUS=1 \
      --duration "$WARMUP_DURATION" \
      --vus 1 \
      "$s" >/dev/null 2>&1 || true
  done
  rm -rf "results/${warm_run}"
}

# Measured pass: namespaced per arm+round so nothing (ours or k6's internal
# handleSummary) collides across arms.
measured_pass() {
  local arm="$1" round="$2" slug run_arm
  slug="$(arm_slug "$arm")"
  run_arm="${RUN_ID}__${slug}__r${round}"
  mkdir -p "results/${run_arm}/hts/benchmark"
  for TEST in "${TESTS[@]}"; do
    TEST="${TEST// /}"
    local s; s="$(script_for "$TEST")"
    [ -z "$s" ] && { echo "    skip ${TEST} — script not found"; continue; }
    for VU in "${VUS[@]}"; do
      VU="${VU// /}"
      echo "    ${arm} r${round}: ${TEST} vu${VU}"
      k6 run \
        --env BASE_URL="http://localhost:${PORT}" \
        --env SERVER_NAME=hts \
        --env RUN_ID="$run_arm" \
        --env TEST_ID="$TEST" \
        --env VUS="$VU" \
        --duration "$DURATION" \
        --vus "$VU" \
        --summary-export "results/${run_arm}/hts/${TEST}_vu${VU}.json" \
        "$s" || true
    done
  done
}

# ── main loop: rounds × (rotated) arms ──────────────────────────────────────
read -ra ARM_ARR <<< "$ARMS"
NARMS=${#ARM_ARR[@]}

echo "Arms: ${ARMS}"
echo "Rounds: ${ROUNDS}  Tests: ${TESTS_CSV}  VUs: ${VUS_CSV}  Duration: ${DURATION}"
echo ""

for (( round=1; round<=ROUNDS; round++ )); do
  echo "═══ round ${round}/${ROUNDS} ═══"
  # Rotate the arm order by (round-1) so no arm is permanently first
  # (first-after-restart pays the most cold-start; rotation shares it).
  for (( j=0; j<NARMS; j++ )); do
    idx=$(( (j + round - 1) % NARMS ))
    arm="${ARM_ARR[$idx]}"
    slug="$(arm_slug "$arm")"
    logfile="${LOG_DIR}/hts-bench-${BACKEND}-${slug}-r${round}.log"

    stop_server "$SERVER_PID"; SERVER_PID=""
    if ! start_server "$arm" "$logfile"; then
      echo "  arm '${arm}' round ${round} FAILED to start/verify — recording and continuing"
      ANY_ARM_FAILED=1
      continue
    fi
    warmup_pass "${slug}-r${round}"
    measured_pass "$arm" "$round"
  done
done

stop_server "$SERVER_PID"; SERVER_PID=""

echo ""
if [ "$ANY_ARM_FAILED" -ne 0 ]; then
  echo "One or more arms failed to start/verify; see per-arm logs. Reporting on whatever completed."
fi
# Do not exit non-zero here: the report + artifact-upload steps must still run
# so partial data and logs are preserved. The workflow inspects ARM_FAILED.
echo "ARM_FAILED=${ANY_ARM_FAILED}" >> "${GITHUB_ENV:-/dev/null}"
