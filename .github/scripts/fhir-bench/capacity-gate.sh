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
# Both MemTotal and every MemAvailable poll go through host-mem.sh, not a
# direct `docker info`/`docker run ... /proc/meminfo` read — see that
# script's header for why (run 36410157709: this host's own /proc/meminfo
# read from inside a plain container disagreed with `docker info` by more
# than 5x, most likely an LXC/VM-like daemon host whose /proc/meminfo is
# virtualised, e.g. lxcfs). The source host-mem.sh actually trusted for a
# given poll is logged on that poll's line and recorded as
# CAPACITY_MEM_SOURCE alongside the other CAPACITY_* outputs below.
#
# Required environment (exported by the workflow step's env:):
#   BACKEND                  matrix.backend
#   IN_PG_SHARED_BUFFERS     inputs.pg_shared_buffers
#   IN_MONGO_WT_CACHE_GB     inputs.mongo_wt_cache_gb
#   ES_HEAP_MB               needs.setup.outputs.es_heap_mb
#
# Also reads host-mem.sh (same directory) for MemTotal/MemAvailable.
#
# Outputs: CAPACITY_NEED_MB / CAPACITY_AVAIL_MB / CAPACITY_WAIT_S /
# CAPACITY_MEM_SOURCE appended to $GITHUB_ENV (read by "Run benchmark
# suites" for runner-info.txt, and by summary_backends.py as its fallback
# Capacity gate row for a leg that failed this gate before
# runner-info.txt was ever written).
set -euo pipefail

SCRIPT_DIR="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
HOST_MEM_SH="$SCRIPT_DIR/host-mem.sh"

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

# read_host_mem: runs host-mem.sh and splits its one guaranteed output
# line into HOSTMEM_SOURCE / HOSTMEM_TOTAL_MB / HOSTMEM_AVAIL_MB (each
# "unknown"/"none" on any failure, including host-mem.sh itself being
# unreadable — a case host-mem.sh's own `set -uo pipefail` guarding
# cannot cover).
read_host_mem() {
  local line
  line=$(bash "$HOST_MEM_SH" 2>/dev/null) || line=""
  HOSTMEM_SOURCE=$(printf '%s\n' "$line" | sed -n 's/^source=\([^ ]*\) .*/\1/p')
  HOSTMEM_TOTAL_MB=$(printf '%s\n' "$line" | sed -n 's/.*total_mb=\([^ ]*\) avail_mb=.*/\1/p')
  HOSTMEM_AVAIL_MB=$(printf '%s\n' "$line" | sed -n 's/.*avail_mb=\(.*\)$/\1/p')
  [ -n "$HOSTMEM_SOURCE" ] || HOSTMEM_SOURCE=none
  [ -n "$HOSTMEM_TOTAL_MB" ] || HOSTMEM_TOTAL_MB=unknown
  [ -n "$HOSTMEM_AVAIL_MB" ] || HOSTMEM_AVAIL_MB=unknown
}

read_host_mem
MEM_TOTAL_MB="$HOSTMEM_TOTAL_MB"
case "$MEM_TOTAL_MB" in ''|*[!0-9]*) MEM_TOTAL_MB=0 ;; esac
echo "Docker host MemTotal: ${MEM_TOTAL_MB}MB (source=${HOSTMEM_SOURCE})"

# A host-mem.sh miss reads as MEM_TOTAL_MB=0 (source=none), which would
# otherwise always trip the impossible-fit check below and tell the
# user to lower their inputs when the real problem is that memory
# couldn't be read at all. Skip straight to the poll loop instead — it
# re-reads memory through the same script, so a transient miss alone
# doesn't fail the leg.
if [ "$MEM_TOTAL_MB" -eq 0 ]; then
  echo "::warning::could not determine Docker host MemTotal (host-mem.sh source=${HOSTMEM_SOURCE}) — skipping the impossible-fit check; the poll below still guards capacity"
elif [ $(( CAPACITY_NEED_MB + 2048 )) -gt "$MEM_TOTAL_MB" ]; then
  # Record what this leg needed even though no suite will run, so
  # the summary (which reads these from $GITHUB_ENV when
  # runner-info.txt was never written) can still show a Capacity
  # gate row instead of nothing.
  {
    echo "CAPACITY_NEED_MB=$CAPACITY_NEED_MB"
    echo "CAPACITY_AVAIL_MB=skipped"
    echo "CAPACITY_WAIT_S=0"
    echo "CAPACITY_MEM_SOURCE=$HOSTMEM_SOURCE"
  } >> "$GITHUB_ENV"
  echo "::error::backend=$BACKEND needs ~${CAPACITY_NEED_MB}MB (plus a 2048MB margin), but the Docker host only reports ${MEM_TOTAL_MB}MB total RAM (source=${HOSTMEM_SOURCE}). Lower es_heap / mongo_wt_cache_gb / pg_shared_buffers, or pick a lighter backend — this combination can never fit, even with the whole host free."
  exit 1
fi

echo "── Waiting for MemAvailable >= $(( CAPACITY_NEED_MB + 2048 ))MB (poll 60s, timeout 10min) ──"
CAPACITY_START=$SECONDS
CAPACITY_AVAIL_MB=""
CAPACITY_MEM_SOURCE=""
CAPACITY_OK=0
while :; do
  read_host_mem
  CAPACITY_MEM_SOURCE="$HOSTMEM_SOURCE"
  CAPACITY_AVAIL_MB="$HOSTMEM_AVAIL_MB"
  case "$CAPACITY_AVAIL_MB" in ''|*[!0-9]*) CAPACITY_AVAIL_MB="" ;; esac
  CAPACITY_WAIT_S=$((SECONDS - CAPACITY_START))
  if [ -n "$CAPACITY_AVAIL_MB" ] && [ "$CAPACITY_AVAIL_MB" -ge $(( CAPACITY_NEED_MB + 2048 )) ]; then
    CAPACITY_OK=1
    echo "  t=${CAPACITY_WAIT_S}s MemAvailable=${CAPACITY_AVAIL_MB}MB (source=${CAPACITY_MEM_SOURCE}) — capacity OK"
    break
  fi
  # source=none reads the same as any other unknown MemAvailable above
  # (CAPACITY_AVAIL_MB blanked by the numeric guard) — keep waiting and
  # let the 600s budget below be the only thing that fails the leg.
  echo "  t=${CAPACITY_WAIT_S}s MemAvailable=${CAPACITY_AVAIL_MB:-unknown}MB (source=${CAPACITY_MEM_SOURCE}), need $(( CAPACITY_NEED_MB + 2048 ))MB — waiting"
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
  echo "CAPACITY_MEM_SOURCE=${CAPACITY_MEM_SOURCE:-none}"
} >> "$GITHUB_ENV"

if [ "$CAPACITY_OK" -ne 1 ]; then
  echo "::error::Docker host still short of memory for $BACKEND after ${CAPACITY_WAIT_S}s (needed $(( CAPACITY_NEED_MB + 2048 ))MB, last MemAvailable=${CAPACITY_AVAIL_MB:-unknown}MB). Failing this leg rather than risk an OOM on the shared host."
  exit 1
fi
