#!/usr/bin/env bash
#
# Resolve the `backend` workflow_dispatch input into a job matrix for
# fhir-benchmark.yml, and validate the free-text tuning inputs in one place
# before the ~13-minute build runs — once, not once per leg; its single
# binary serves every leg in the resolved matrix.
#
# Called from: the `setup` job's "Resolve matrix from backend input" step.
#
# Free-text/choice inputs reach this script only through environment
# variables, never inlined as ${{ }} inside the step's `run:` (a value
# containing `"` or `$(...)` would otherwise be substituted into the script
# text before bash runs it, and validated only after — same risk the pg_*
# inputs already carry, limited today to people with write access).
#
# Required environment (exported by the workflow step's env:):
#   IN_BACKEND                      inputs.backend
#   IN_MAX_PARALLEL                 inputs.max_parallel
#   IN_ES_HEAP                      inputs.es_heap
#   IN_ES_SYNC_MODE                 inputs.es_sync_mode
#   IN_MONGO_WT_CACHE_GB            inputs.mongo_wt_cache_gb
#   IN_HFS_MONGO_MAX_CONNECTIONS    inputs.hfs_mongo_max_connections
#
# Outputs (via $GITHUB_OUTPUT): matrix, max_parallel, es_heap_mb,
# leg_timeout_min.
#
# This combines GitHub's default `bash -e {0}` step wrapping with the
# original step's own `set -uo pipefail` line — the net effect the step ran
# under was `-euo pipefail`. A script invoked as `bash <script>` does NOT
# inherit the calling step's shell options, so this script sets that
# combination explicitly to reproduce the same behavior.
set -euo pipefail
FAIL=0
BACKEND="${IN_BACKEND:-core}"

port_for() {
  case "$1" in
    sqlite)                 echo 8081 ;;
    postgres)               echo 8082 ;;
    mongodb)                echo 8083 ;;
    sqlite-elasticsearch)   echo 8084 ;;
    postgres-elasticsearch) echo 8085 ;;
    mongodb-elasticsearch)  echo 8086 ;;
  esac
}

case "$BACKEND" in
  core)          LEGS="sqlite postgres" ;;
  all)           LEGS="sqlite postgres mongodb sqlite-elasticsearch postgres-elasticsearch mongodb-elasticsearch" ;;
  elasticsearch) LEGS="sqlite-elasticsearch postgres-elasticsearch mongodb-elasticsearch" ;;
  sqlite|postgres|mongodb|sqlite-elasticsearch|postgres-elasticsearch|mongodb-elasticsearch)
                 LEGS="$BACKEND" ;;
  *)
    echo "::error::Unknown backend selection '$BACKEND'"
    FAIL=1
    LEGS="" ;;
esac

ITEMS=""
for b in $LEGS; do
  ITEMS="${ITEMS:+$ITEMS,}{\"backend\":\"$b\",\"port\":$(port_for "$b")}"
done
MATRIX="{\"include\":[$ITEMS]}"
LEG_COUNT=$(echo "$LEGS" | wc -w)

# max_parallel: job outputs are strings, so the benchmark job's
# strategy.max-parallel reads this through fromJSON(). Clamped to
# 1..2, not higher: the heaviest leg (postgres-elasticsearch, at
# default inputs) needs about 6 GB (the capacity gate's own formula:
# 2048+1536 primary, 1024*2+512 ES = 6144MB, so 8 GB free with its
# 2 GB margin), so 2 concurrent legs already use most of the shared
# 4-CPU / 11 GB Docker host that the rest of CI also runs on.
MAX_PARALLEL="${IN_MAX_PARALLEL:-1}"
case "$MAX_PARALLEL" in
  *[!0-9]*)
    echo "::error::max_parallel must be a positive integer (got '$MAX_PARALLEL')"
    FAIL=1
    MAX_PARALLEL=1 ;;
  *)
    # Reject an overlong digit string BEFORE the 10# arithmetic below
    # — bash arithmetic on a huge value doesn't error, it silently
    # wraps, which could land back in 1..2 and pass clamping that
    # was meant to reject it. 3 digits (<=999) is well inside a safe
    # range for the arithmetic and the `-gt 2` clamp that follows.
    if [ "${#MAX_PARALLEL}" -gt 3 ]; then
      echo "::error::max_parallel must be a positive integer, at most 3 digits (got '$MAX_PARALLEL')"
      FAIL=1
      MAX_PARALLEL=1
    else
      # 10# forces base-10 so a leading zero ("01") isn't read as octal.
      MAX_PARALLEL=$((10#$MAX_PARALLEL))
      if [ "$MAX_PARALLEL" -lt 1 ]; then
        echo "::error::max_parallel must be >= 1 (got '$IN_MAX_PARALLEL')"
        FAIL=1
        MAX_PARALLEL=1
      elif [ "$MAX_PARALLEL" -gt 2 ]; then
        echo "::warning::max_parallel=$MAX_PARALLEL clamped to 2 (shared 4-CPU / 11 GB Docker host)"
        MAX_PARALLEL=2
      fi
    fi
    ;;
esac

# es_heap: -Xms = -Xmx, 256m..4g. Emitted as es_heap_mb so later
# steps never have to re-parse the unit suffix.
ES_HEAP="${IN_ES_HEAP:-1g}"
ES_HEAP_MB=""
if [[ "$ES_HEAP" =~ ^([0-9]+)([mMgG])$ ]]; then
  ES_HEAP_MB=$((10#${BASH_REMATCH[1]}))
  case "${BASH_REMATCH[2]}" in g|G) ES_HEAP_MB=$((ES_HEAP_MB * 1024)) ;; esac
else
  echo "::error::es_heap must match ^[0-9]+[mMgG]\$ (got '$ES_HEAP')"
  FAIL=1
fi
if [ -n "$ES_HEAP_MB" ] && { [ "$ES_HEAP_MB" -lt 256 ] || [ "$ES_HEAP_MB" -gt 4096 ]; }; then
  echo "::error::es_heap must be 256m..4g (got '$ES_HEAP')"
  FAIL=1
fi

# es_sync_mode: defence in depth — workflow_dispatch's UI already
# restricts this to the choice options, but an API dispatch can send
# anything. Default here matches the workflow input's default
# (synchronous): async import cannot finish the 1000-bundle corpus
# inside k6's 60-minute cap (run 36412022609 — 313/1000 bundles, ES
# drain status=incomplete, 194,664 resources never indexed).
ES_SYNC_MODE="${IN_ES_SYNC_MODE:-synchronous}"
case "$ES_SYNC_MODE" in
  asynchronous|synchronous) ;;
  *)
    echo "::error::es_sync_mode must be 'asynchronous' or 'synchronous' (got '$ES_SYNC_MODE')"
    FAIL=1 ;;
esac

# mongo_wt_cache_gb: decimal, 0.25..6 (parity with Postgres's
# shared_buffers default; mongod's own default risks a host OOM).
MONGO_WT_CACHE_GB="${IN_MONGO_WT_CACHE_GB:-2}"
if ! [[ "$MONGO_WT_CACHE_GB" =~ ^[0-9]+(\.[0-9]+)?$ ]] \
   || ! awk -v v="$MONGO_WT_CACHE_GB" 'BEGIN { exit !(v >= 0.25 && v <= 6) }'; then
  echo "::error::mongo_wt_cache_gb must be a number in 0.25..6 (got '$MONGO_WT_CACHE_GB')"
  FAIL=1
fi

# hfs_mongo_max_connections: positive integer, 1..1000, no leading
# zeros. HFS silently falls back to 10 on a non-numeric value —
# reject it here instead. This is the ONLY validation of
# hfs_mongo_max_connections: "Start HFS server" re-derives
# HFS_MONGO_POOL from the same raw input (env:
# IN_HFS_MONGO_MAX_CONNECTIONS) without re-checking it, trusting
# this step to have already rejected a bad value before the
# ~13-minute build runs on every leg, including sqlite/postgres.
HFS_MONGO_POOL="${IN_HFS_MONGO_MAX_CONNECTIONS:-32}"
# At most 4 digits (matched BEFORE the -gt 1000 test below): an
# overlong digit string handed straight to `[ -gt ]` risks "value
# too great for base"/an unparseable-integer failure there instead
# of the clear error this step is supposed to give.
if ! [[ "$HFS_MONGO_POOL" =~ ^[1-9][0-9]{0,3}$ ]]; then
  echo "::error::hfs_mongo_max_connections must be a positive integer with no leading zeros, at most 4 digits (got '$HFS_MONGO_POOL')"
  FAIL=1
elif [ "$HFS_MONGO_POOL" -gt 1000 ]; then
  echo "::error::hfs_mongo_max_connections must be 1..1000 (got '$HFS_MONGO_POOL')"
  FAIL=1
fi

if [ "$FAIL" -ne 0 ]; then
  exit 1
fi

{
  echo "matrix=$MATRIX"
  echo "max_parallel=$MAX_PARALLEL"
  echo "es_heap_mb=$ES_HEAP_MB"
} >> "$GITHUB_OUTPUT"
echo "Backend matrix: $MATRIX"
echo "::notice title=Benchmark legs::backend=$BACKEND -> $LEG_COUNT leg(s) [$LEGS], max_parallel=$MAX_PARALLEL. A leg takes ~30-130 min; legs beyond max_parallel queue."

# The benchmark job's timeout-minutes, and the base the "Reap stale
# benchmark containers and verify Docker host storage" step (in the
# `benchmark` job, fhir-benchmark.yml) sizes its age guard from. Must stay <= 175:
# .github/actions/docker-host-gc (input container-max-age-min,
# default 180; used by ci.yml / ci-extended / hts / keycloak-smoke
# / ui-tests-matrix) reaps any `hfs-ci=true` container whose
# `.State.StartedAt` is >= 180 min old, and GitHub gives a
# cancelled job up to 5 more minutes to exit. Do not raise this
# without re-checking that reaper's default.
echo "leg_timeout_min=150" >> "$GITHUB_OUTPUT"
