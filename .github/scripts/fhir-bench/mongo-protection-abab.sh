#!/usr/bin/env bash
# ABAB measurement of the MongoDB search protections (#2293).
#
# Runs inside the `mongodb` benchmark leg, after the normal suites, on the
# same host, corpus and commit. For each round it runs variant A, then
# variant B: restart HFS with that variant's environment and
# HFS_REQUEST_TIMEOUT=$ABAB_REQUEST_TIMEOUT_S (the production value, not the
# 900 s the normal leg uses for the import suite), then run the suites and
# record the HTTP status mix.
#
# Variants (ABAB_VARIANTS is a comma list; the first is A, the second is B):
#   off           no protection (the default of the product)
#   timeout       HFS_MONGODB_SEARCH_TIMEOUT_MS = T*1000 - 2000 (2 s margin,
#                 as bundle_transaction_budget in crates/hfs/src/main.rs)
#   gate          HFS_MONGODB_BROAD_SEARCH_CONCURRENCY = $ABAB_GATE
#   timeout+gate  both
#
# Inputs (environment):
#   ABAB_VARIANTS            e.g. "off,timeout+gate"            (required)
#   ABAB_ROUNDS              rounds, default 4
#   ABAB_GATE                broad-search concurrency, default 8
#   ABAB_REQUEST_TIMEOUT_S   HFS_REQUEST_TIMEOUT, default 30
#   ABAB_DEADLINE_EPOCH      epoch seconds after which no run may start
#                            (optional; 0 or unset = no budget)
#   ABAB_SUITES              default "crud insert search read-under-search"
#   BENCH_PORT               HFS port                             (required)
#   RESULTS_DIR              leg results dir, default bench-results/mongodb
#   IN_HFS_MONGO_MAX_CONNECTIONS  pool, default 32
#   HFS_PID                  pid of the HFS the normal run left up (stopped)
#   MONGO_CONTAINER          docker name of mongod (currentOp sampler)
#   BUNDLE_URL, RUN_ID, HFS_BIN, HFS_LOG_LEVEL, HFS_PG_MAX_CONNECTIONS,
#   BENCH_IN_VUS, BENCH_IN_DURATION  as in the "Run benchmark suites" step.
#
# Output layout (consumed by abab_report.py):
#   $RESULTS_DIR/abab/r<N>-<variant>/{crud,insert,search,read-under-search}.json
#   $RESULTS_DIR/abab/r<N>-<variant>/<suite>.log
#   $RESULTS_DIR/abab/r<N>-<variant>/status-counts.txt
#       one line per suite and status: "suite=<s> status=<code> count=<n>"
#   $RESULTS_DIR/abab/r<N>-<variant>/mongo-longops.txt   currentOp >= 30 s
#   $RESULTS_DIR/abab/r<N>-<variant>/deadline-expired.txt  log line count
#   $RESULTS_DIR/abab/r<N>-<variant>/hfs.log, env.txt
#   $RESULTS_DIR/abab/status.txt   complete=1|0, rounds_done, reason
#
# Sourced-free: run it as a command. Exit 0 even for a partial result (the
# marker is in abab/status.txt); non-zero only for bad input.

set -u

ABAB_ROUNDS="${ABAB_ROUNDS:-4}"
ABAB_GATE="${ABAB_GATE:-8}"
ABAB_REQUEST_TIMEOUT_S="${ABAB_REQUEST_TIMEOUT_S:-30}"
ABAB_DEADLINE_EPOCH="${ABAB_DEADLINE_EPOCH:-0}"
ABAB_SUITES="${ABAB_SUITES:-crud insert search read-under-search}"
RESULTS_DIR="${RESULTS_DIR:-bench-results/mongodb}"
HFS_BIN="${HFS_BIN:-./hfs}"
HFS_MONGO_POOL="${IN_HFS_MONGO_MAX_CONNECTIONS:-32}"
RUN_ID="${RUN_ID:-abab}"
BUNDLE_URL="${BUNDLE_URL:-}"
READY_TIMEOUT="${ABAB_READY_TIMEOUT_S:-300}"
LONGOPS_PERIOD_S=5
WORKSPACE="${GITHUB_WORKSPACE:-$PWD}"

die() { echo "::error::mongo-protection-abab: $*" >&2; exit 1; }

[ -n "${ABAB_VARIANTS:-}" ] || die "ABAB_VARIANTS is required"
[ -n "${BENCH_PORT:-}" ] || die "BENCH_PORT is required"
case "$ABAB_ROUNDS$ABAB_GATE$ABAB_REQUEST_TIMEOUT_S$ABAB_DEADLINE_EPOCH" in
  *[!0-9]*) die "ABAB_ROUNDS, ABAB_GATE, ABAB_REQUEST_TIMEOUT_S and ABAB_DEADLINE_EPOCH must be integers" ;;
esac
[ "$ABAB_ROUNDS" -ge 1 ] || die "ABAB_ROUNDS must be >= 1"
[ "$ABAB_GATE" -ge 1 ] || die "ABAB_GATE must be >= 1"
[ "$ABAB_REQUEST_TIMEOUT_S" -gt 2 ] || die "ABAB_REQUEST_TIMEOUT_S must be > 2 (the search deadline is T*1000-2000 ms)"

IFS=',' read -r VAR_A VAR_B EXTRA_VAR <<< "$ABAB_VARIANTS"
[ -n "${VAR_A:-}" ] && [ -n "${VAR_B:-}" ] && [ -z "${EXTRA_VAR:-}" ] \
  || die "ABAB_VARIANTS must name exactly two variants, got '$ABAB_VARIANTS'"
for V in "$VAR_A" "$VAR_B"; do
  case "$V" in off|timeout|gate|timeout+gate) ;; *) die "unknown variant '$V'" ;; esac
done
[ "$VAR_A" != "$VAR_B" ] || die "variants A and B must differ"

SEARCH_TIMEOUT_MS=$(( ABAB_REQUEST_TIMEOUT_S * 1000 - 2000 ))

if [ "$ABAB_GATE" -ge "$HFS_MONGO_POOL" ]; then
  echo "::warning::ABAB gate ($ABAB_GATE) is not below the Mongo pool ($HFS_MONGO_POOL); the broad-search gate cannot bite (HFS logs the same warning)"
fi

ABAB_DIR="$RESULTS_DIR/abab"
mkdir -p "$ABAB_DIR"

# Locate a suite script: the repo's own k6 dir first, then upstream's.
suite_script() {
  local own="$WORKSPACE/.github/scripts/fhir-bench/k6/$1.js"
  if [ -f "$own" ]; then echo "$own"; else echo "k6/$1.js"; fi
}

EXTRA_ARGS=""
[ -n "${BENCH_IN_VUS:-}" ]      && EXTRA_ARGS="$EXTRA_ARGS --vus $BENCH_IN_VUS"
[ -n "${BENCH_IN_DURATION:-}" ] && EXTRA_ARGS="$EXTRA_ARGS --duration $BENCH_IN_DURATION"

NEW_HFS_PID=""
SAMPLER_PID=""

stop_sampler() {
  if [ -n "$SAMPLER_PID" ]; then
    kill "$SAMPLER_PID" 2>/dev/null || true
    wait "$SAMPLER_PID" 2>/dev/null || true
    SAMPLER_PID=""
  fi
}

# Stop an HFS pid and wait until it is gone so the port is free again.
stop_pid() {
  local pid="$1"
  [ -n "$pid" ] || return 0
  kill "$pid" 2>/dev/null || return 0
  for _ in $(seq 1 60); do
    kill -0 "$pid" 2>/dev/null || return 0
    sleep 1
  done
  kill -9 "$pid" 2>/dev/null || true
}

# shellcheck disable=SC2329  # invoked via trap below
cleanup() {
  stop_sampler
  stop_pid "$NEW_HFS_PID"
  NEW_HFS_PID=""
}
trap cleanup EXIT

# Print "<status> <count>" per HTTP status, summed over routes, from
# /metrics. Our own scrapes (route="/metrics") are left out.
metrics_by_status() {
  curl -sf --max-time 10 "http://127.0.0.1:$BENCH_PORT/metrics" 2>/dev/null \
    | awk '
        /^http_requests_total\{/ {
          if ($0 ~ /route="\/metrics"/) next
          if (match($0, /status="[0-9]+"/)) {
            s = substr($0, RSTART + 8, RLENGTH - 9)
            n[s] += $NF
          }
        }
        END { for (s in n) printf "%s %d\n", s, n[s] }'
}

# status_delta <suite> <before> <after> -> lines "suite=.. status=.. count=.."
status_delta() {
  awk -v suite="$1" '
      NR == FNR { before[$1] = $2; next }
      { d = $2 - before[$1]; if (d > 0) printf "suite=%s status=%s count=%d\n", suite, $1, d }
    ' "$2" "$3" | sort
}

start_longops_sampler() {
  local out="$1"
  : > "$out"
  [ -n "${MONGO_CONTAINER:-}" ] || { echo "# MONGO_CONTAINER unset: no currentOp sampling" > "$out"; return 0; }
  (
    while true; do
      ts=$(date -u +%FT%TZ)
      ops=$(docker exec "$MONGO_CONTAINER" mongosh --quiet --eval '
        JSON.stringify(db.getSiblingDB("admin").currentOp({active: true, secs_running: {$gte: 30}}).inprog
          .filter(o => o.ns && !o.ns.startsWith("admin.") && !o.ns.startsWith("local."))
          .map(o => ({ns: o.ns, op: o.op, secs: o.secs_running, cmd: JSON.stringify(o.command || {}).slice(0, 200)})))' 2>/dev/null)
      case "$ops" in ""|"[]") ;; *) echo "$ts $ops" >> "$out" ;; esac
      sleep "$LONGOPS_PERIOD_S"
    done
  ) &
  SAMPLER_PID=$!
}

start_hfs() {
  local variant="$1" logfile="$2" timeout_ms="" gate=""
  case "$variant" in
    timeout)      timeout_ms="$SEARCH_TIMEOUT_MS" ;;
    gate)         gate="$ABAB_GATE" ;;
    timeout+gate) timeout_ms="$SEARCH_TIMEOUT_MS"; gate="$ABAB_GATE" ;;
  esac
  local -a envv=(
    -u HFS_MONGODB_SEARCH_TIMEOUT_MS
    -u HFS_MONGODB_BROAD_SEARCH_CONCURRENCY
    HFS_SERVER_HOST=127.0.0.1
    HFS_SERVER_PORT="$BENCH_PORT"
    HFS_LOG_LEVEL="${HFS_LOG_LEVEL:-info}"
    HFS_MAX_BODY_SIZE=209715200
    HFS_REQUEST_TIMEOUT="$ABAB_REQUEST_TIMEOUT_S"
    HFS_PG_MAX_CONNECTIONS="${HFS_PG_MAX_CONNECTIONS:-32}"
    HFS_MONGODB_MAX_CONNECTIONS="$HFS_MONGO_POOL"
    HFS_DATA_DIR="$WORKSPACE/data"
  )
  [ -z "$timeout_ms" ] || envv+=(HFS_MONGODB_SEARCH_TIMEOUT_MS="$timeout_ms")
  [ -z "$gate" ] || envv+=(HFS_MONGODB_BROAD_SEARCH_CONCURRENCY="$gate")
  {
    echo "variant=$variant"
    echo "HFS_REQUEST_TIMEOUT=$ABAB_REQUEST_TIMEOUT_S"
    echo "HFS_MONGODB_MAX_CONNECTIONS=$HFS_MONGO_POOL"
    echo "HFS_MONGODB_SEARCH_TIMEOUT_MS=${timeout_ms:-unset}"
    echo "HFS_MONGODB_BROAD_SEARCH_CONCURRENCY=${gate:-unset}"
  } > "$(dirname "$logfile")/env.txt"
  env "${envv[@]}" "$HFS_BIN" > "$logfile" 2>&1 &
  NEW_HFS_PID=$!
}

wait_ready() {
  local _
  for _ in $(seq 1 "$READY_TIMEOUT"); do
    kill -0 "$NEW_HFS_PID" 2>/dev/null || { echo "HFS exited during startup"; return 1; }
    curl -sf --max-time 3 -o /dev/null "http://127.0.0.1:$BENCH_PORT/metadata" && return 0
    sleep 1
  done
  echo "HFS not ready after ${READY_TIMEOUT}s"
  return 1
}

write_status() { # <complete 0|1> <rounds_done> <reason>
  {
    echo "complete=$1"
    echo "rounds_done=$2"
    echo "rounds_requested=$ABAB_ROUNDS"
    echo "variants=$ABAB_VARIANTS"
    echo "reason=$3"
  } > "$ABAB_DIR/status.txt"
}

# Stop the HFS the normal run left up; its port is needed.
stop_pid "${HFS_PID:-}"

MAX_RUN_S=0
ROUNDS_DONE=0
INCOMPLETE_REASON=""

run_variant() { # <round> <variant>  -> 0 ok, 1 failed
  local round="$1" variant="$2" dir suite script t0 sc_before sc_after
  dir="$ABAB_DIR/r${round}-${variant}"
  mkdir -p "$dir"
  : > "$dir/status-counts.txt"
  echo "=== ABAB round $round variant $variant ==="

  start_hfs "$variant" "$dir/hfs.log"
  if ! wait_ready; then
    echo "$variant: HFS failed to start" > "$dir/FAILED"
    stop_pid "$NEW_HFS_PID"; NEW_HFS_PID=""
    return 1
  fi

  start_longops_sampler "$dir/mongo-longops.txt"
  for suite in $ABAB_SUITES; do
    script=$(suite_script "$suite")
    sc_before=$(mktemp); sc_after=$(mktemp)
    metrics_by_status > "$sc_before"
    t0=$(date +%s)
    # shellcheck disable=SC2086  # EXTRA_ARGS is a deliberate word-split flag list
    ( cd "$WORKSPACE" && k6 run \
        --no-usage-report \
        --tag runid="$RUN_ID" \
        --tag fhirimpl=hfs \
        --tag abab_variant="$variant" \
        --env BASE_URL="http://localhost:$BENCH_PORT" \
        --env BUNDLE_URL="$BUNDLE_URL" \
        --env FHIRIMPL=hfs \
        --summary-trend-stats "avg,min,med,p(90),p(95),p(99),max,count" \
        --summary-export "$dir/${suite}.json" \
        $EXTRA_ARGS \
        "$script" 2>&1 | tee "$dir/${suite}.log" ) || true
    echo "suite=$suite wall_s=$(( $(date +%s) - t0 ))" >> "$dir/walls.txt"
    metrics_by_status > "$sc_after"
    status_delta "$suite" "$sc_before" "$sc_after" >> "$dir/status-counts.txt"
    rm -f "$sc_before" "$sc_after"
  done
  stop_sampler

  # Server-side deadline expirations (504 source); /metrics cannot tell a
  # deadline 504 from a mongod maxTimeMS one.
  grep -c 'MongoDB search deadline expired' "$dir/hfs.log" > "$dir/deadline-expired.txt" 2>/dev/null \
    || echo 0 > "$dir/deadline-expired.txt"

  stop_pid "$NEW_HFS_PID"; NEW_HFS_PID=""
  return 0
}

for round in $(seq 1 "$ABAB_ROUNDS"); do
  for variant in "$VAR_A" "$VAR_B"; do
    if [ "$ABAB_DEADLINE_EPOCH" -gt 0 ]; then
      now=$(date +%s)
      # Runs are of about the same length; the longest one so far is the
      # estimate (0 before the first run, which always starts).
      if [ $(( now + MAX_RUN_S )) -ge "$ABAB_DEADLINE_EPOCH" ]; then
        INCOMPLETE_REASON="time budget: round $round $variant needs ~${MAX_RUN_S}s, $(( ABAB_DEADLINE_EPOCH - now ))s left"
        break 2
      fi
    fi
    t_run=$(date +%s)
    if ! run_variant "$round" "$variant"; then
      INCOMPLETE_REASON="round $round $variant: HFS failed to start"
      break 2
    fi
    d=$(( $(date +%s) - t_run ))
    [ "$d" -gt "$MAX_RUN_S" ] && MAX_RUN_S="$d"
  done
  ROUNDS_DONE="$round"
done

if [ -n "$INCOMPLETE_REASON" ]; then
  echo "::warning::ABAB incomplete: $INCOMPLETE_REASON ($ROUNDS_DONE of $ABAB_ROUNDS rounds finished)"
  write_status 0 "$ROUNDS_DONE" "$INCOMPLETE_REASON"
else
  write_status 1 "$ROUNDS_DONE" "ok"
fi
exit 0
