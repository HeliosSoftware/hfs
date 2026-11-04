#!/usr/bin/env bash
#
# Starts the HFS server for one benchmark leg and waits until it answers
# /metadata. Shared by the "Start HFS server" step of fhir-benchmark.yml and by
# hfs_restart (suite-lib.sh), which restarts HFS between the sync/async rounds
# of es_sync_mode=abab (#2292), so the launch command and the readiness wait
# exist once.
#
# Executed (`bash start-hfs.sh`), not sourced. HFS keeps running after this
# script exits; its pid is written to $START_HFS_PIDFILE.
#
# Inputs (environment):
#   START_HFS_PORT        required: HFS_SERVER_PORT
#   START_HFS_BACKEND     required: matrix backend (names the log, picks the
#                         readiness timeout and the sqlite diagnostics)
#   START_HFS_LOG_LEVEL   HFS_LOG_LEVEL (default info)
#   START_HFS_PG_POOL     HFS_PG_MAX_CONNECTIONS (default 32)
#   START_HFS_MONGO_POOL  HFS_MONGODB_MAX_CONNECTIONS (default 32)
#   START_HFS_BIN         server binary (default $GITHUB_WORKSPACE/hfs)
#   START_HFS_LOG_APPEND  1 = append to the log instead of truncating it (restarts)
#   START_HFS_EXPORT      1 = write HFS_PID and HFS_READY_SECONDS to $GITHUB_ENV
# HFS_STORAGE_BACKEND, HFS_DATABASE_URL, the HFS_COMPOSITE_* / HFS_ELASTICSEARCH_*
# variables and ES_CONTAINER / MONGO_CONTAINER (diagnostics only) are ambient,
# exported by "Configure backend env".
#
# Outputs: the log /tmp/hfs-bench-$START_HFS_BACKEND.log, the pid file
# /tmp/hfs-bench-$START_HFS_BACKEND.pid, and on stdout the line
# HFS_READY_SECONDS=<n>. Exit 0 = ready, 1 = not ready (diagnostics printed).
set -uo pipefail

PORT="${START_HFS_PORT:?START_HFS_PORT is required}"
BACKEND="${START_HFS_BACKEND:?START_HFS_BACKEND is required}"
BIN="${START_HFS_BIN:-${GITHUB_WORKSPACE:-.}/hfs}"
LOG="/tmp/hfs-bench-$BACKEND.log"
PIDFILE="${START_HFS_PIDFILE:-/tmp/hfs-bench-$BACKEND.pid}"

# HFS_MAX_BODY_SIZE: the Synthea import bundles run to several MB each, over
# the 10 MB default (measured post-decompression). Aidbox is configured for a
# 200 MB ceiling in the benchmark (BOX_WEB_MAX_BODY), so match that here —
# otherwise large bundles 413 and the import suite fails. There is no
# per-entry cap in HFS, only this byte limit.
#
# HFS_DATA_DIR: full search-parameter coverage is loaded at startup from
# <data_dir>/search-parameters-r4.json (2.2 MB, checked into the repo). If that
# file isn't found, search silently degrades to ~9 global params (only a
# warning is logged) and nearly every search scenario breaks. Pin it
# explicitly so coverage never depends on the process CWD.
#
# HFS_REQUEST_TIMEOUT: the default is 30s, but a 1600-entry Synthea
# transaction bundle under the import suite's 20 concurrent VUs routinely takes
# longer than that, and the timeout middleware would cut the request
# mid-transaction (observed: import p95 pinned at ~30s, ~40% of imports failing
# with the handler future cancelled before commit). The benchmark comparators
# (Aidbox/HAPI/Medplum) impose no such cap and import.js allows 900s
# client-side, so match that here for a fair comparison.
#
# HFS_PG_MAX_CONNECTIONS: set explicitly so the pool size is a recorded input
# rather than an inferred default, and a sweepable one — 32 was chosen for the
# SEARCH suite's 30 VUs, but crud drives 300. In run 32767978962 crud managed
# 261 RPS at p50 996ms; 261 RPS across 32 connections is ~122ms of service
# time per request, so ~870ms of each request was spent waiting for a
# connection rather than doing work. Postgres itself is started with
# max_connections=1000, so the pool is the binding constraint, not the server.
# The search suite runs 30 VUs; a pool smaller than that puts every request
# behind a connection queue and the measurement stops being about query cost
# at all (issue #224). 32 covers the 30 VUs while staying within a few
# multiples of the container's ~8 cores.
#
# HFS_MONGODB_MAX_CONNECTIONS: same reasoning — Mongo's own pool default is 10.
# HFS silently falls back to 10 on a non-numeric value (mongodb/backend.rs,
# config.max_connections); the value is validated in setup's "Resolve matrix
# from backend input".
if [ "${START_HFS_LOG_APPEND:-0}" = 1 ]; then
  echo "── HFS restart $(date -u +%FT%TZ) ──" >> "$LOG"
else
  : > "$LOG"
fi
HFS_SERVER_HOST="127.0.0.1" \
HFS_SERVER_PORT="$PORT" \
HFS_LOG_LEVEL="${START_HFS_LOG_LEVEL:-info}" \
HFS_MAX_BODY_SIZE="209715200" \
HFS_REQUEST_TIMEOUT="900" \
HFS_PG_MAX_CONNECTIONS="${START_HFS_PG_POOL:-32}" \
HFS_MONGODB_MAX_CONNECTIONS="${START_HFS_MONGO_POOL:-32}" \
HFS_DATA_DIR="${GITHUB_WORKSPACE:-.}/data" \
  "$BIN" >> "$LOG" 2>&1 < /dev/null &
HFS_PID=$!
echo "$HFS_PID" > "$PIDFILE"
if [ "${START_HFS_EXPORT:-0}" = 1 ] && [ -n "${GITHUB_ENV:-}" ]; then
  echo "HFS_PID=$HFS_PID" >> "$GITHUB_ENV"
fi

# Cold start is dominated by conformance seeding — 1,377 SearchParameters
# written one at a time, measured at 27s (postgres) / 38s (sqlite) on an empty
# database. The old 120s gate left barely 3x headroom on a shared runner, and
# when the sqlite leg of run 32740257894 blew through it the step reported 120s
# of silence and nothing else. 300s plus a progress line every 10s so a slow
# start is distinguishable from a hung one.
echo "Waiting for HFS to become ready..."
READY_TIMEOUT=300
# *-elasticsearch legs seed the SearchParameters bundle THROUGH the composite
# (the same "Composite storage initialized: ... + Elasticsearch (search)" path
# checked in "Assert search parameters loaded"), which is slower than seeding
# the primary alone.
if [[ "$BACKEND" == *-elasticsearch ]]; then READY_TIMEOUT=600; fi
START_TS=$SECONDS
READY=0
while [ $((SECONDS - START_TS)) -lt "$READY_TIMEOUT" ]; do
  if curl -sf --max-time 10 "http://localhost:$PORT/metadata" -o /dev/null 2>/dev/null; then
    echo "HFS ready after $((SECONDS - START_TS))s"
    echo "HFS_READY_SECONDS=$((SECONDS - START_TS))"
    if [ "${START_HFS_EXPORT:-0}" = 1 ] && [ -n "${GITHUB_ENV:-}" ]; then
      echo "HFS_READY_SECONDS=$((SECONDS - START_TS))" >> "$GITHUB_ENV"
    fi
    READY=1
    break
  fi
  if ! kill -0 "$HFS_PID" 2>/dev/null; then
    echo "ERROR: HFS process $HFS_PID exited after $((SECONDS - START_TS))s"
    break
  fi
  ELAPSED=$((SECONDS - START_TS))
  if [ $((ELAPSED % 10)) -lt 2 ] && [ "$ELAPSED" -gt 0 ]; then
    echo "  ... still waiting (${ELAPSED}s) — last log line: $(tail -n 1 "$LOG" 2>/dev/null || echo '(no log yet)')"
  fi
  sleep 2
done

if [ "$READY" -ne 1 ]; then
  echo "ERROR: HFS did not become ready within ${READY_TIMEOUT}s"
  echo "── server log ──"
  cat "$LOG" || true
  echo "── process state ──"
  ps -o pid,stat,etime,rss,args -p "$HFS_PID" 2>/dev/null || echo "process $HFS_PID is gone"
  echo "── open files / sockets (may need lsof) ──"
  ls -l "/proc/$HFS_PID/fd" 2>/dev/null | tail -n 20 || true
  echo "── kernel stack ──"
  cat "/proc/$HFS_PID/stack" 2>/dev/null || echo "(stack unavailable)"
  echo "── wait channel ──"
  cat "/proc/$HFS_PID/wchan" 2>/dev/null && echo || true
  if [[ "$BACKEND" == "sqlite" || "$BACKEND" == "sqlite-elasticsearch" ]]; then
    echo "── sqlite database state ──"
    ls -la "${HFS_DATABASE_URL:-}"* 2>/dev/null || echo "(no database files)"
  fi
  if [ -n "${ES_CONTAINER:-}" ]; then
    echo "── elasticsearch ($ES_CONTAINER) ──"
    curl -s --max-time 30 "http://${DOCKER_HOST_IP:-localhost}:${ES_PORT:-9200}/_cluster/health?pretty" \
      || echo "::error::Elasticsearch on port ${ES_PORT:-?} is NOT answering — HFS may have been retrying bootstrap writes against a dead search backend."
    timeout 60 docker inspect -f 'OOMKilled={{.State.OOMKilled}} ExitCode={{.State.ExitCode}} Status={{.State.Status}}' "$ES_CONTAINER" || true
    timeout 60 docker logs --tail 30 "$ES_CONTAINER" 2>&1 || true
  fi
  if [ -n "${MONGO_CONTAINER:-}" ]; then
    echo "── mongodb ($MONGO_CONTAINER) ──"
    timeout 60 docker inspect -f 'OOMKilled={{.State.OOMKilled}} ExitCode={{.State.ExitCode}} Status={{.State.Status}}' "$MONGO_CONTAINER" || true
    timeout 60 docker logs --tail 30 "$MONGO_CONTAINER" 2>&1 || true
  fi
  exit 1
fi
exit 0
