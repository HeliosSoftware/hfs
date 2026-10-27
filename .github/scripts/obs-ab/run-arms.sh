#!/usr/bin/env bash
#
# Same-session observability A/B driver (issue #297), extended with
# version arms (issue #298).
#
# Runs the k6 tx-benchmark scenarios against the HTS server once per
# (round × arm), restarting the server between arms so each arm's
# HELIOS_OBS_MODE / RUST_LOG takes effect (the arm switch is read once into a
# process-global OnceLock, so it can only change across a restart). The
# terminology database is imported ONCE by the caller and left untouched: only
# the app process is restarted, so every arm measures the same warmed data with
# no cross-run drift.
#
# ── Version arms ────────────────────────────────────────────────────────────
# An arm is normally an env-var switch on ONE binary. The `baseline` arm is
# different: it runs a *second binary*, built by the caller from an older git
# ref (the workflow's `baseline_ref` input, e.g. v0.2.0), and passed here as
# BASELINE_BIN. That is what lets a run separate two questions #298 has to keep
# apart — how much of the regression the current branch already recovered
# (baseline→default) versus how much was never observability's fault at all
# (off→default, measured on one binary).
#
# A version arm breaks the "restart the app, keep the database" invariant,
# because a different binary runs a DIFFERENT startup migration set against the
# shared database. Concretely, between v0.2.0 and HEAD the HTS schema gained
# `authority_rank` on code_systems/value_sets and the `concepts_search_fts`
# index, and both binaries run migrations + prebuild_concepts_fts on EVERY boot
# (crates/hts/src/backends/sqlite/mod.rs). An old binary would leave HEAD's
# extra index stale, and a new binary would then re-migrate on top — so arm N+1
# would measure a database that arm N mutated. DB_SNAPSHOT closes that hole:
# when set, every arm is restored from one pristine post-import copy before its
# server starts, so all arms see byte-identical data regardless of order.
#
# Restoring resets the file, but not the comparison: the discarded warm-up pass
# below re-primes the OS page cache and the in-process memo caches for every arm
# identically, which is exactly what it already had to do after each restart.
#
# The confound a version arm CANNOT remove: the database is imported once, by
# the CURRENT binary's importer. `baseline` therefore measures old serving code
# over a newly-imported corpus, not the corpus v0.2.0 would have built for
# itself (#261 changed which of the duplicate THO CodeSystems win, for one). It
# isolates per-request serving cost with the data held constant — which is the
# question — and is NOT a reproduction of the v0.2.0 release.
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
#                     (e.g. "off default full full+probe", or
#                      "baseline off default" for a version comparison)
#   ROUNDS            repeats per arm (integer >= 1)
#   TESTS_CSV         comma-separated test IDs (e.g. "LK01,VC01,EX01")
#   VUS_CSV           comma-separated VU levels (e.g. "1" or "1,10,50")
#   DURATION          measured duration per scenario (e.g. "30s")
#   WARMUP_DURATION   discarded warm-up duration per test (e.g. "10s")
#   LOG_DIR           directory for per-arm server logs (e.g. "/tmp")
#   HTS_DATABASE_URL, HTS_STORAGE_BACKEND  already exported by the caller
#
# Required ONLY when ARMS contains a version arm (`baseline`); the run aborts
# up front if either is missing, rather than reporting meaningless numbers:
#   BASELINE_BIN      path to the hts binary built from `baseline_ref`
#   DB_SNAPSHOT       pristine post-import DB copy, restored before every arm
#                     (sqlite only; see "Version arms" above)
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
#
# `baseline` is a VERSION arm: it runs BASELINE_BIN instead, and sets no
# HELIOS_OBS_MODE at all — the ref it is built from (v0.2.0) predates the
# switch, so the variable would be ignored anyway, and leaving it unset is the
# honest reproduction of how that build shipped.
arm_obs_mode() {
  case "$1" in
    off)        echo "off" ;;
    default)    echo "default" ;;
    no-span)    echo "no-span" ;;
    full)       echo "full" ;;
    full+probe) echo "full" ;;
    baseline)   echo "" ;;
    *)          echo "" ;;
  esac
}
# Which binary an arm runs. Version arms are the only ones that differ.
arm_binary() {
  case "$1" in
    baseline) printf '%s' "${BASELINE_BIN:-}" ;;
    *)        printf '%s' "$HTS_BIN" ;;
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
    # A pre-#292 build logs no obs_mode= line; asserting one would fail every
    # baseline arm. The binary-identity check in start_server covers it instead.
    baseline)   echo "" ;;
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

# Restore the pristine post-import database, so an arm can never inherit schema
# or index state left behind by a different binary version (see "Version arms").
# No-op unless DB_SNAPSHOT is set, i.e. unless a version arm is in the ladder.
# Must run while no server holds the file — callers invoke it after stop_server.
restore_db() {
  [ -z "${DB_SNAPSHOT:-}" ] && return 0
  if [ ! -f "$DB_SNAPSHOT" ]; then
    echo "  ERROR: DB_SNAPSHOT=${DB_SNAPSHOT} does not exist"
    return 1
  fi
  # The -wal/-shm sidecars belong to the OLD database file. Leaving them next to
  # a restored copy is corruption, not staleness: sqlite would replay a WAL
  # against a file it was never written for.
  rm -f "${HTS_DATABASE_URL}-wal" "${HTS_DATABASE_URL}-shm"
  cp -f "$DB_SNAPSHOT" "$HTS_DATABASE_URL" || {
    echo "  ERROR: failed to restore ${HTS_DATABASE_URL} from ${DB_SNAPSHOT}"
    return 1
  }
  return 0
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
  local obs rustlog expected started_epoch bin
  obs="$(arm_obs_mode "$arm")"
  rustlog="$(arm_rust_log "$arm")"
  expected="$(arm_expected_mode_log "$arm")"
  bin="$(arm_binary "$arm")"
  started_epoch=$(date +%s)

  if [ -z "$bin" ] || [ ! -x "$bin" ]; then
    echo "  ERROR: arm '${arm}' has no runnable binary (got '${bin}')."
    echo "         A 'baseline' arm requires BASELINE_BIN — set the workflow's baseline_ref input."
    return 1
  fi

  echo "  starting server: bin=${bin} HELIOS_OBS_MODE=${obs} RUST_LOG='${rustlog}'  -> ${logfile}"
  # RUST_LOG (when set) overrides HTS_LOG_LEVEL in the server's EnvFilter. It
  # must be genuinely UNSET for the non-probe arms, not set to "" — a set-but-
  # empty RUST_LOG makes the tracing EnvFilter enable nothing, which suppresses
  # the startup `obs_mode=` line this script greps to confirm the active arm.
  # `env -u RUST_LOG` guarantees it is absent (even if inherited); the probe arm
  # sets it explicitly.
  #
  # An empty $obs means a version arm, which must run with HELIOS_OBS_MODE
  # genuinely absent rather than set-but-empty — the build it targets predates
  # the variable, and an empty value is not what production would have seen.
  if [ -n "$rustlog" ]; then
    HTS_SERVER_PORT="$PORT" \
    HTS_LOG_LEVEL="info" \
    HELIOS_OBS_MODE="$obs" \
    RUST_LOG="$rustlog" \
      "$bin" >"$logfile" 2>&1 &
  elif [ -n "$obs" ]; then
    HTS_SERVER_PORT="$PORT" \
    HTS_LOG_LEVEL="info" \
    HELIOS_OBS_MODE="$obs" \
      env -u RUST_LOG "$bin" >"$logfile" 2>&1 &
  else
    HTS_SERVER_PORT="$PORT" \
    HTS_LOG_LEVEL="info" \
      env -u RUST_LOG -u HELIOS_OBS_MODE "$bin" >"$logfile" 2>&1 &
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

  # Binary-identity guard: confirm the process we started is running the binary
  # this arm asked for. This is the ONLY assertion a version arm gets — a
  # pre-#292 build emits no obs_mode= line for the check below to match — and it
  # is the one that matters most there, because a `baseline` arm silently
  # running the current binary would report "no regression" and be believed.
  local actual_exe
  actual_exe="$(readlink -f "/proc/${SERVER_PID}/exe" 2>/dev/null || true)"
  if [ -n "$actual_exe" ] && [ "$actual_exe" != "$(readlink -f "$bin")" ]; then
    echo "  ERROR: arm '${arm}' expected binary $(readlink -f "$bin") but pid ${SERVER_PID} is running ${actual_exe}"
    return 1
  fi

  # Zombie / wrong-arm guard: confirm we are talking to the process we just
  # started (a survivor on the port would serve every request of this arm under
  # the WRONG mode). Two independent checks, because neither alone covers every
  # build: the port-owner check works on any version, while uptime_seconds is
  # absent from /health on pre-v0.2.1 builds and simply skips there.
  local port_pids
  port_pids="$(fuser -n tcp "$PORT" 2>/dev/null | tr -s ' ' '\n' | grep -E '^[0-9]+$' || true)"
  if [ -n "$port_pids" ] && ! printf '%s\n' "$port_pids" | grep -qx "$SERVER_PID"; then
    echo "  ERROR: port ${PORT} is held by pid(s) $(echo "$port_pids" | tr '\n' ' ')— not the server we started (${SERVER_PID})"
    return 1
  fi

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

# Fail before the first import-expensive round rather than mid-ladder: a version
# arm without its binary, or without the snapshot that keeps arms independent,
# produces numbers that look fine and mean nothing.
HAS_VERSION_ARM=0
for a in "${ARM_ARR[@]}"; do
  [ "$a" = "baseline" ] && HAS_VERSION_ARM=1
done
if [ "$HAS_VERSION_ARM" -eq 1 ]; then
  if [ -z "${BASELINE_BIN:-}" ] || [ ! -x "${BASELINE_BIN:-}" ]; then
    echo "ERROR: arm list contains 'baseline' but BASELINE_BIN is unset or not executable."
    echo "       Set the workflow's baseline_ref input (e.g. v0.2.0)."
    exit 1
  fi
  if [ -z "${DB_SNAPSHOT:-}" ]; then
    echo "ERROR: arm list contains 'baseline' but DB_SNAPSHOT is unset — arms would"
    echo "       inherit each other's schema migrations. Refusing to produce numbers."
    exit 1
  fi
  echo "Version arm active: baseline -> ${BASELINE_BIN}"
  echo "DB restored from ${DB_SNAPSHOT} before every arm."
fi

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
    # Between stopping the last server and starting this one is the only window
    # in which the database file is unheld and can be safely rolled back.
    if ! restore_db; then
      echo "  arm '${arm}' round ${round} SKIPPED — could not restore the pristine DB"
      ANY_ARM_FAILED=1
      continue
    fi
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
