#!/usr/bin/env bash
#
# Bulk-import suite for one benchmark leg: loads the same Synthea corpus the
# `import` suite loads through HFS's `$bulk-submit` into a FRESH, empty
# database, and records resources/s from kick-off until the deferred
# search-index rebuild has finished. The work is bulk_import.py (same
# directory); this wrapper owns the backend-specific parts:
#   - a fresh database per backend family (a sqlite file under RUNNER_TEMP, a
#     postgres database `hfs_bulk_import`, the mongo database `<db>_bulk_import`,
#     the Elasticsearch index prefix `hfsbulk`), so the leg's main database,
#     and with it the crud/search corpus, result-size counts and crud residue,
#     is never touched;
#   - a SECOND HFS on BENCH_PORT+100 (8181-8186) with the leg's environment
#     plus that database, and the leg-budget deadline;
#   - pg_stat_statements / Elasticsearch snapshots, and dropping the bulk
#     database or indices afterwards.
# It starts no containers, so the Docker host capacity gate model is unchanged.
#
# Called from: the `benchmark` job's "Run benchmark suites" step
# (fhir-benchmark.yml), after write_search_counts and before the container
# state dump, so the liveness gate covers it. Never fails the step: every
# outcome (complete, timeout, error, unsupported, skipped) is a bulk-import.txt
# that compare_legs.py and summary_backends.py render; the exit code is 0.
#
# Required environment:
#   BACKEND        matrix.backend (sqlite | postgres | mongodb | <each>-elasticsearch)
#   BENCH_PORT     the leg's HFS port; the bulk HFS listens on BENCH_PORT+100
#   RESULTS_DIR    bench-results/<backend>
#   BUNDLE_URL     the tgz bundle server, http://<docker host>:<port>
# Optional environment:
#   BENCH_RUN_ID, BENCH_LEG_START_EPOCH + BENCH_LEG_TIMEOUT_MIN (the leg budget:
#   the deadline is leg start + timeout - BULK_IMPORT_LEG_RESERVE_S; without
#   them there is no deadline), BENCH_IN_PG_MAX_CONNECTIONS,
#   IN_HFS_MONGO_MAX_CONNECTIONS, PG_CONTAINER, MONGO_CONTAINER, ES_PORT,
#   DOCKER_HOST_IP, GITHUB_WORKSPACE, and everything HFS reads (HFS_STORAGE_BACKEND,
#   HFS_DATABASE_URL, HFS_ELASTICSEARCH_NODES, HFS_COMPOSITE_SYNC_MODE, any
#   HFS_BULK_SUBMIT_*): all inherited from the step / $GITHUB_ENV.
#   BULK_IMPORT_TIMEOUT_S           cap from kick-off until searchable (default 3600)
#   BULK_IMPORT_LEG_RESERVE_S       budget kept back for the rest of the leg (default 1200)
#   BULK_IMPORT_BUNDLES             rotation bundles to convert (default 1000)
#   BULK_IMPORT_MAX_LINES_PER_FILE  NDJSON part size (default 50000)
#   BULK_IMPORT_WORKDIR             scratch (default ${RUNNER_TEMP:-/tmp}/hfs-bench-bulk-import-$BACKEND)
#   BULK_IMPORT_SOURCE              corpus (a directory or a tgz URL); overrides BUNDLE_URL
#   HFS_BIN                         the hfs binary (default $GITHUB_WORKSPACE/hfs)
#
# Outputs, in RESULTS_DIR: bulk-import.txt (the result), bulk-import-hfs.log
# (tail of the bulk HFS log), bulk-import-pgstat.txt (postgres family),
# bulk-import-esstat.txt (*-elasticsearch). The workflow also tees this
# script's stdout to bulk-import.log.
#
# Local run (sqlite, a directory of extracted Synthea bundles):
#   BACKEND=sqlite HFS_STORAGE_BACKEND=sqlite BENCH_PORT=18471 \
#   RESULTS_DIR=/tmp/bulk/results BUNDLE_URL=/path/to/bulk_1k BULK_IMPORT_BUNDLES=30 \
#   BULK_IMPORT_WORKDIR=/tmp/bulk/work GITHUB_WORKSPACE=$PWD HFS_BIN=/path/to/hfs \
#   bash .github/scripts/fhir-bench/bulk-import.sh
set -uo pipefail

HERE="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

BULK_PORT=$((BENCH_PORT + 100))
# RUNNER_TEMP is emptied by the runner at job start and end, so a crashed leg
# leaves no multi-GB corpus behind.
WORKDIR="${BULK_IMPORT_WORKDIR:-${RUNNER_TEMP:-/tmp}/hfs-bench-bulk-import-$BACKEND}"
BULK_PG_DB=hfs_bulk_import
BULK_ES_PREFIX=hfsbulk
HFS_BIN="${HFS_BIN:-${GITHUB_WORKSPACE:-.}/hfs}"

mkdir -p "$RESULTS_DIR"
rm -f "$RESULTS_DIR/bulk-import.txt"
rm -rf "$WORKDIR"
mkdir -p "$WORKDIR/submit"

kill_port() { fuser -k "$BULK_PORT/tcp" >/dev/null 2>&1 || true; }
kill_port
trap kill_port EXIT

# A minimal bulk-import.txt, only when bulk_import.py wrote none.
fallback_result() {
  [ -f "$RESULTS_DIR/bulk-import.txt" ] && return 0
  {
    echo "status=$1"
    echo "reason=$2"
    echo "phase=start"
    echo "backend=$BACKEND"
  } > "$RESULTS_DIR/bulk-import.txt"
}

# The deadline: the leg's job timeout, minus what the rest of the leg still needs.
DEADLINE=0
if [ -n "${BENCH_LEG_START_EPOCH:-}" ] && [ -n "${BENCH_LEG_TIMEOUT_MIN:-}" ]; then
  DEADLINE=$((BENCH_LEG_START_EPOCH + BENCH_LEG_TIMEOUT_MIN * 60 - ${BULK_IMPORT_LEG_RESERVE_S:-1200}))
fi
TIMEOUT_S="${BULK_IMPORT_TIMEOUT_S:-3600}"

# A fresh database per backend family: one explicit case per family and never
# an if/else, the same rule "Configure backend env" follows. The corpus must
# not be doubled in the database crud/search measure.
MIN_FREE_GB=6
case "$BACKEND" in
  sqlite|sqlite-elasticsearch)
    export HFS_DATABASE_URL="$WORKDIR/bulk-import.db"
    # Bulk sqlite database plus ~3 GB of NDJSON and ~120 MB of receipts, next
    # to the leg's own sqlite file.
    MIN_FREE_GB=20
    ;;
  postgres|postgres-elasticsearch)
    if ! timeout 120 docker exec "${PG_CONTAINER:-}" psql -U postgres -d postgres -v ON_ERROR_STOP=1 -q \
         -c "DROP DATABASE IF EXISTS $BULK_PG_DB WITH (FORCE)" -c "CREATE DATABASE $BULK_PG_DB"; then
      fallback_result error "could not create database $BULK_PG_DB"
      exit 0
    fi
    export HFS_DATABASE_URL="${HFS_DATABASE_URL%/*}/$BULK_PG_DB"
    ;;
  mongodb|mongodb-elasticsearch)
    export HFS_MONGODB_DATABASE="${HFS_MONGODB_DATABASE:-hfs_bench}_bulk_import"
    ;;
  *)
    fallback_result unsupported "no fresh-database wiring for backend $BACKEND"
    exit 0
    ;;
esac
case "$BACKEND" in
  *-elasticsearch)
    # An index prefix the leg's own `${ES_PREFIX}_*` (hfs_*) patterns do not match.
    export HFS_ELASTICSEARCH_INDEX_PREFIX="$BULK_ES_PREFIX"
    # The deferred rebuild writes with the bench's refresh=wait_for unless told
    # otherwise; HFS's documented import setting is no per-request refresh
    # (bulk-data-submit SKILL.md, "Rebuild knobs").
    export HFS_ELASTICSEARCH_REINDEX_REFRESH="${HFS_ELASTICSEARCH_REINDEX_REFRESH:-false}"
    ;;
esac

# The bulk HFS: the leg's environment plus these.
export HFS_SERVER_HOST=127.0.0.1
export HFS_SERVER_PORT="$BULK_PORT"
# The status Content-Location is built from it; without it HFS advertises
# http://localhost:8080 (see any leg's server log).
export HFS_BASE_URL="http://127.0.0.1:$BULK_PORT"
export HFS_LOG_LEVEL=info   # the rebuild lines bulk_import.py follows are INFO
export HFS_DATA_DIR="${HFS_DATA_DIR:-${GITHUB_WORKSPACE:-.}/data}"
# The default ${HFS_DATA_DIR}/submit is inside the checkout.
export HFS_BULK_SUBMIT_OUTPUT_DIR="$WORKDIR/submit"
export HFS_BULK_SUBMIT_POLL_RATE_LIMIT=0
# 'Start HFS server' sets these two inline, not via $GITHUB_ENV, so they are not
# inherited; the 30 s default would 408 the verify count queries (up to 11 s on
# the main sqlite leg in run 37782202746) or a status poll on a busy database.
export HFS_REQUEST_TIMEOUT="${HFS_REQUEST_TIMEOUT:-900}"
export HFS_MAX_BODY_SIZE="${HFS_MAX_BODY_SIZE:-209715200}"
export HFS_PG_MAX_CONNECTIONS="${BENCH_IN_PG_MAX_CONNECTIONS:-32}"
export HFS_MONGODB_MAX_CONNECTIONS="${IN_HFS_MONGO_MAX_CONNECTIONS:-32}"

# Backstop: bulk_import.py honours the deadline itself; this only catches a hang.
NOW=$(date +%s)
if [ "$DEADLINE" -gt 0 ]; then
  BACKSTOP=$((DEADLINE - NOW + 300))
else
  BACKSTOP=$((TIMEOUT_S + 1800))
fi
[ "$BACKSTOP" -lt 120 ] && BACKSTOP=120

echo "bulk-import: backend=$BACKEND port=$BULK_PORT workdir=$WORKDIR timeout=${TIMEOUT_S}s deadline_epoch=$DEADLINE backstop=${BACKSTOP}s"
timeout -k 60 "$BACKSTOP" python3 "$HERE/bulk_import.py" \
  --source "${BULK_IMPORT_SOURCE:-${BUNDLE_URL:-}}" \
  --hfs-bin "$HFS_BIN" \
  --port "$BULK_PORT" \
  --workdir "$WORKDIR" \
  --results-dir "$RESULTS_DIR" \
  --hfs-log "$WORKDIR/hfs-bulk-import.log" \
  --bundles "${BULK_IMPORT_BUNDLES:-1000}" \
  --max-lines-per-file "${BULK_IMPORT_MAX_LINES_PER_FILE:-50000}" \
  --timeout-s "$TIMEOUT_S" \
  --deadline-epoch "$DEADLINE" \
  --min-free-gb "$MIN_FREE_GB" \
  --submission-id "hfs-bench-${BENCH_RUN_ID:-local}-$BACKEND"
RC=$?
kill_port
fallback_result error "bulk_import.py exited $RC without a result"

# Snapshots, before the bulk database goes away.
case "$BACKEND" in
  postgres|postgres-elasticsearch)
    if [ -n "${PG_CONTAINER:-}" ]; then
      {
        echo "suite=bulk-import"
        timeout 60 docker exec "$PG_CONTAINER" psql -U postgres -d postgres \
          -c "SELECT round((sum(total_exec_time)/1000.0)::numeric, 1) AS pg_exec_total_s,
                     round((sum(total_plan_time)/1000.0)::numeric, 1) AS pg_plan_total_s,
                     sum(calls) AS calls
              FROM pg_stat_statements
              WHERE dbid = (SELECT oid FROM pg_database WHERE datname = '$BULK_PG_DB')" \
          -c "SELECT round((total_exec_time/1000.0)::numeric, 1) AS exec_s, calls,
                     round(mean_exec_time::numeric, 3) AS mean_ms, rows,
                     left(regexp_replace(query, '\s+', ' ', 'g'), 110) AS query
              FROM pg_stat_statements
              WHERE dbid = (SELECT oid FROM pg_database WHERE datname = '$BULK_PG_DB')
              ORDER BY total_exec_time DESC LIMIT 15" \
          -c "SELECT pg_size_pretty(pg_database_size('$BULK_PG_DB')) AS bulk_database_size"
      } > "$RESULTS_DIR/bulk-import-pgstat.txt" 2>&1 \
        || echo "::warning::bulk-import pgstat capture failed"
      timeout 60 docker exec "$PG_CONTAINER" psql -U postgres -d postgres -q \
        -c "SELECT pg_stat_statements_reset()" >/dev/null 2>&1 || true
    fi
    ;;
esac
case "$BACKEND" in
  *-elasticsearch)
    if [ -n "${ES_PORT:-}" ]; then
      timeout 60 curl -s --max-time 30 \
        "http://$DOCKER_HOST_IP:$ES_PORT/_cat/indices/${BULK_ES_PREFIX}_*?v&h=index,docs.count,store.size&s=store.size:desc" \
        > "$RESULTS_DIR/bulk-import-esstat.txt" 2>&1 \
        || echo "::warning::bulk-import ES snapshot failed"
    fi
    ;;
esac
if [ -f "$WORKDIR/hfs-bulk-import.log" ]; then
  tail -c 20000000 "$WORKDIR/hfs-bulk-import.log" > "$RESULTS_DIR/bulk-import-hfs.log" 2>/dev/null || true
fi

# Cleanup: the bulk database, every container-side byte it took.
case "$BACKEND" in
  postgres|postgres-elasticsearch)
    if [ -n "${PG_CONTAINER:-}" ]; then
      timeout 120 docker exec "$PG_CONTAINER" psql -U postgres -d postgres -q \
        -c "DROP DATABASE IF EXISTS $BULK_PG_DB WITH (FORCE)" >/dev/null 2>&1 \
        || echo "::warning::could not drop database $BULK_PG_DB"
      timeout 120 docker exec "$PG_CONTAINER" psql -U postgres -d postgres -q \
        -c "CHECKPOINT" >/dev/null 2>&1 || true
    fi
    ;;
  mongodb|mongodb-elasticsearch)
    if [ -n "${MONGO_CONTAINER:-}" ]; then
      timeout 120 docker exec "$MONGO_CONTAINER" mongosh --quiet "$HFS_MONGODB_DATABASE" \
        --eval 'db.dropDatabase()' >/dev/null 2>&1 \
        || echo "::warning::could not drop mongo database $HFS_MONGODB_DATABASE"
    fi
    ;;
esac
case "$BACKEND" in
  *-elasticsearch)
    if [ -n "${ES_PORT:-}" ]; then
      ES_URL="http://$DOCKER_HOST_IP:$ES_PORT"
      # ES 8 refuses wildcard deletes (action.destructive_requires_name), so
      # list the indices and delete them by name, in batches.
      ES_INDICES=()
      mapfile -t ES_INDICES < <(timeout 60 curl -s --max-time 30 \
        "$ES_URL/_cat/indices/${BULK_ES_PREFIX}_*?h=index" 2>/dev/null | tr -d '\r' | grep -v '^$' || true)
      ES_BATCH=""
      ES_N=0
      for IDX in "${ES_INDICES[@]+"${ES_INDICES[@]}"}"; do
        ES_BATCH="${ES_BATCH:+$ES_BATCH,}$IDX"
        ES_N=$((ES_N + 1))
        if [ "$ES_N" -ge 20 ]; then
          timeout 90 curl -s --max-time 60 -X DELETE "$ES_URL/$ES_BATCH" >/dev/null 2>&1 || true
          ES_BATCH=""
          ES_N=0
        fi
      done
      if [ -n "$ES_BATCH" ]; then
        timeout 90 curl -s --max-time 60 -X DELETE "$ES_URL/$ES_BATCH" >/dev/null 2>&1 || true
      fi
    fi
    ;;
esac
rm -rf "$WORKDIR"

if [ -f "$RESULTS_DIR/bulk-import.txt" ]; then
  echo "── bulk-import.txt ──"
  grep -v '^env_' "$RESULTS_DIR/bulk-import.txt" || true
fi
exit 0
