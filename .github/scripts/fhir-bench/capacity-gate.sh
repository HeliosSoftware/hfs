#!/usr/bin/env bash
#
# Docker host capacity gate for one benchmark leg.
#
# Called from: the `benchmark` job's "Docker host capacity gate" step.
#
# `max_parallel`'s clamp (1..2, see the `setup` job / resolve-matrix.sh) only
# bounds THIS run's own legs — it cannot see the rest of CI sharing the same
# 4-CPU / 11 GB Docker host, which is exactly what has OOM-killed a mongod
# here before. NEED_MB is a rough per-backend model, not a real cgroup
# budget:
#   postgres family: shared_buffers, +1.5G for the rest of the server
#     (autovacuum workers' maintenance_work_mem, connections), or a flat
#     3584 when pg_shared_buffers isn't a plain "<N>GB" value (e.g.
#     "auto" — resolved for real in "Start ephemeral Postgres").
#   mongo family: --wiredTigerCacheSizeGB + ~1G mongod/OS overhead.
#   sqlite family: 512 (HFS itself; no extra container).
#   *-elasticsearch legs ADD heap*2 (heap plus JVM off-heap/direct
#     memory, ballparked at another heap's worth) + 512 (ES process
#     overhead).
# A combination that could never fit even with the WHOLE host free
# fails immediately, naming the inputs to lower. Otherwise poll
# MemAvailable (host-wide, not per-container — this is what protects
# OTHER CI too) every 60s for up to 10 minutes; still short after that,
# fail the leg rather than risk taking down a neighbour's container.
#
# Required environment (exported by the workflow step's env:):
#   BACKEND                  matrix.backend
#   RUN_ID                   github.run_id
#   IN_PG_SHARED_BUFFERS     inputs.pg_shared_buffers
#   IN_MONGO_WT_CACHE_GB     inputs.mongo_wt_cache_gb
#   ES_HEAP_MB               needs.setup.outputs.es_heap_mb
#
# Outputs: CAPACITY_NEED_MB / CAPACITY_AVAIL_MB / CAPACITY_WAIT_S appended to
# $GITHUB_ENV (read by "Run benchmark suites" for runner-info.txt, and by
# summary_backends.py as its fallback Capacity gate row for a leg that
# failed this gate before runner-info.txt was ever written).
set -euo pipefail

case "$BACKEND" in
  postgres|postgres-elasticsearch)
    PG_SHARED_BUFFERS="${IN_PG_SHARED_BUFFERS:-2GB}"
    # A plain or fractional GB value (e.g. "2GB", "1.5GB") is parsed
    # here via awk (bash arithmetic can't do fractions); anything
    # else — "auto" or a bad value — falls back to the flat 3584
    # estimate instead of handing non-numeric text to bash
    # arithmetic, which aborts this step with a raw "arithmetic
    # syntax error" rather than naming the input (the pre-#1475
    # shm-size calc in the YAML's "Start ephemeral Postgres" step has
    # the same shape and the same gap, just later in the run).
    if [[ "$PG_SHARED_BUFFERS" =~ ^([0-9]+(\.[0-9]+)?)GB$ ]]; then
      NEED_PRIMARY_MB=$(awk -v g="${BASH_REMATCH[1]}" 'BEGIN { printf "%d", g * 1024 + 1536 }')
    else
      NEED_PRIMARY_MB=3584
    fi
    ;;
  mongodb|mongodb-elasticsearch)
    MONGO_WT_CACHE_GB="${IN_MONGO_WT_CACHE_GB:-2}"
    # WT cache can be fractional (0.25..6) — let awk do the math.
    NEED_PRIMARY_MB=$(awk -v g="$MONGO_WT_CACHE_GB" 'BEGIN { printf "%d", g * 1024 + 1024 }')
    ;;
  sqlite|sqlite-elasticsearch)
    NEED_PRIMARY_MB=512
    ;;
  *)
    echo "::error::Docker host capacity gate has no memory model for backend '$BACKEND'"
    exit 1
    ;;
esac

NEED_ES_MB=0
case "$BACKEND" in
  *-elasticsearch)
    NEED_ES_MB=$(( ${ES_HEAP_MB:-1024} * 2 + 512 ))
    ;;
esac

CAPACITY_NEED_MB=$(( NEED_PRIMARY_MB + NEED_ES_MB ))
echo "Capacity need for $BACKEND: primary=${NEED_PRIMARY_MB}MB elasticsearch=${NEED_ES_MB}MB total=${CAPACITY_NEED_MB}MB"

MEM_TOTAL_BYTES=$(timeout 60 docker info --format '{{.MemTotal}}' 2>/dev/null) || MEM_TOTAL_BYTES=0
case "$MEM_TOTAL_BYTES" in ''|*[!0-9]*) MEM_TOTAL_BYTES=0 ;; esac
MEM_TOTAL_MB=$(( MEM_TOTAL_BYTES / 1024 / 1024 ))
echo "Docker host MemTotal: ${MEM_TOTAL_MB}MB"

# A `docker info` hiccup reads as MEM_TOTAL_MB=0, which would
# otherwise always trip the impossible-fit check below and tell the
# user to lower their inputs when the real problem is that the
# daemon couldn't be read. Skip straight to the poll loop instead —
# it re-reads memory through a separate `docker run`, so a
# transient `docker info` failure alone doesn't fail the leg.
if [ "$MEM_TOTAL_MB" -eq 0 ]; then
  echo "::warning::could not read Docker host MemTotal (docker info failed) — skipping the impossible-fit check; the poll below still guards capacity"
elif [ $(( CAPACITY_NEED_MB + 2048 )) -gt "$MEM_TOTAL_MB" ]; then
  # Record what this leg needed even though no suite will run, so
  # the summary (which reads these from $GITHUB_ENV when
  # runner-info.txt was never written) can still show a Capacity
  # gate row instead of nothing.
  {
    echo "CAPACITY_NEED_MB=$CAPACITY_NEED_MB"
    echo "CAPACITY_AVAIL_MB=skipped"
    echo "CAPACITY_WAIT_S=0"
  } >> "$GITHUB_ENV"
  echo "::error::backend=$BACKEND needs ~${CAPACITY_NEED_MB}MB (plus a 2048MB margin), but the Docker host only reports ${MEM_TOTAL_MB}MB total RAM. Lower es_heap / mongo_wt_cache_gb / pg_shared_buffers, or pick a lighter backend — this combination can never fit, even with the whole host free."
  exit 1
fi

echo "── Waiting for MemAvailable >= $(( CAPACITY_NEED_MB + 2048 ))MB (poll 60s, timeout 10min) ──"
CAPACITY_START=$SECONDS
CAPACITY_AVAIL_MB=""
CAPACITY_OK=0
while :; do
  # shellcheck disable=SC2016 # single-quoted deliberately: $2 is awk's field
  # reference, evaluated inside the container, not a shell variable here.
  CAPACITY_AVAIL_MB=$(timeout 60 docker run --rm --name "hfs-bench-mem-$BACKEND-$RUN_ID" \
      --label hfs-bench=1 --label "hfs-bench-run=$RUN_ID" --label "hfs-bench-leg=$BACKEND" \
      alpine:3 awk '/^MemAvailable:/{print int($2 / 1024)}' /proc/meminfo 2>/dev/null) || CAPACITY_AVAIL_MB=""
  # `timeout 60` only kills the local docker CLI, not a container
  # still starting on the daemon — force it gone so the same
  # `--name` doesn't collide on the next poll (every 60s).
  docker rm -f "hfs-bench-mem-$BACKEND-$RUN_ID" >/dev/null 2>&1 || true
  CAPACITY_WAIT_S=$((SECONDS - CAPACITY_START))
  if [ -n "$CAPACITY_AVAIL_MB" ] && [ "$CAPACITY_AVAIL_MB" -ge $(( CAPACITY_NEED_MB + 2048 )) ]; then
    CAPACITY_OK=1
    echo "  t=${CAPACITY_WAIT_S}s MemAvailable=${CAPACITY_AVAIL_MB}MB — capacity OK"
    break
  fi
  echo "  t=${CAPACITY_WAIT_S}s MemAvailable=${CAPACITY_AVAIL_MB:-unknown}MB, need $(( CAPACITY_NEED_MB + 2048 ))MB — waiting"
  # Never sleep past the 600s budget: a plain `sleep 60` here could
  # carry the last iteration well beyond it (e.g. wait_s=590 -> next
  # check at 650s). Cap the sleep to whatever is actually left.
  CAPACITY_REMAIN_S=$((600 - CAPACITY_WAIT_S))
  if [ "$CAPACITY_REMAIN_S" -le 0 ]; then
    break
  fi
  CAPACITY_SLEEP_S=$CAPACITY_REMAIN_S
  [ "$CAPACITY_SLEEP_S" -gt 60 ] && CAPACITY_SLEEP_S=60
  sleep "$CAPACITY_SLEEP_S"
done

{
  echo "CAPACITY_NEED_MB=$CAPACITY_NEED_MB"
  echo "CAPACITY_AVAIL_MB=${CAPACITY_AVAIL_MB:-unknown}"
  echo "CAPACITY_WAIT_S=$CAPACITY_WAIT_S"
} >> "$GITHUB_ENV"

if [ "$CAPACITY_OK" -ne 1 ]; then
  echo "::error::Docker host still short of memory for $BACKEND after ${CAPACITY_WAIT_S}s (needed $(( CAPACITY_NEED_MB + 2048 ))MB, last MemAvailable=${CAPACITY_AVAIL_MB:-unknown}MB). Failing this leg rather than risk an OOM on the shared host."
  exit 1
fi
