#!/usr/bin/env bash
#
# Elasticsearch refresh_interval sweep for #1749: does a long interval shorten a bulk
# import, and what does it cost an interactive write?
#
# MODE=import   (measurement A) one $bulk-submit import per run for every interval in
#               INTERVALS, REPS repetitions each, arm order rotated per round (round r
#               starts at arm r) so no interval is always first.
# MODE=latency  (measurement B and C) for every INTERVAL x LAT_WRITE_REFRESH pair: start
#               HFS, run write_latency.py (10 x POST /Basic, batch of 8, transaction of
#               662, first write of a new type + its index settings). With
#               LAT_DURING_IMPORT=1 a second pass runs while an import is in flight.
#
# Every run gets its own empty PostgreSQL 16 and Elasticsearch 8.15.0 containers
# (names hfs-sweep-*-$$, ports above 18000, removed on exit; nothing else is touched)
# and a fresh HFS started from HFS_BIN. Run nothing else on the machine meanwhile and
# report the core count with the numbers.
#
# Required environment:
#   HFS_BIN       release hfs binary built with postgres + elasticsearch
#   BUNDLE_SOURCE corpus for bulk_import.py (URL of the bulk_1k tgz, or a directory of
#                 extracted Synthea bundles)
# Optional:
#   MODE=import|latency (import)      INTERVALS="1s 5s 30s -1"     REPS=3
#   BUNDLES=100            bundles per import (sizes the "reduced corpus"; the number of
#                          resources it yields is in each run's bulk-import.txt)
#   HFS_ELASTICSEARCH_WRITE_REFRESH (wait_for)   HFS_COMPOSITE_SYNC_MODE (synchronous)
#   LAT_WRITE_REFRESH="wait_for true false"      LAT_DURING_IMPORT=0
#   BATCH_FILE / TRANSACTION_FILE  real Bundles for write_latency.py (else synthetic)
#   NEW_TYPE=RiskAssessment   BASE_PORT=18400 (uses BASE_PORT..BASE_PORT+3)
#   ES_HEAP_MB=8192    RUN_TIMEOUT_S=3600    SAMPLE_S=5
#   RESULTS_DIR (default $HOME/tmp/refresh-sweep)   WORKDIR (default $HOME/tmp/refresh-sweep-work)
#
# Output: $RESULTS_DIR/<arm>-r<N>/ with bulk-import.txt, hfs.log, refresh-before.json,
# refresh-after.json, samples.tsv, run.txt, latency-*.json; and $RESULTS_DIR/summary.tsv
# (one row per run). Never use /tmp for these: it is a RAM disk on the dev box.
set -uo pipefail

HERE="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
: "${HFS_BIN:?HFS_BIN is required}"
: "${BUNDLE_SOURCE:?BUNDLE_SOURCE is required}"
MODE="${MODE:-import}"
INTERVALS="${INTERVALS:-1s 5s 30s -1}"
REPS="${REPS:-3}"
BUNDLES="${BUNDLES:-100}"
LAT_WRITE_REFRESH="${LAT_WRITE_REFRESH:-wait_for true false}"
LAT_DURING_IMPORT="${LAT_DURING_IMPORT:-0}"
NEW_TYPE="${NEW_TYPE:-RiskAssessment}"
BASE_PORT="${BASE_PORT:-18400}"
ES_HEAP_MB="${ES_HEAP_MB:-8192}"
RUN_TIMEOUT_S="${RUN_TIMEOUT_S:-3600}"
SAMPLE_S="${SAMPLE_S:-5}"
RESULTS_DIR="${RESULTS_DIR:-$HOME/tmp/refresh-sweep}"
WORKDIR="${WORKDIR:-$HOME/tmp/refresh-sweep-work}"
WRITE_REFRESH="${HFS_ELASTICSEARCH_WRITE_REFRESH:-wait_for}"
SYNC_MODE="${HFS_COMPOSITE_SYNC_MODE:-synchronous}"
PG_PORT=$((BASE_PORT + 1)); ES_PORT=$((BASE_PORT + 2)); HFS_PORT=$((BASE_PORT + 3))
PG_NAME="hfs-sweep-pg-$$"; ES_NAME="hfs-sweep-es-$$"
ES_URL="http://127.0.0.1:$ES_PORT"
HFS_BASE="http://127.0.0.1:$HFS_PORT"

case "$MODE" in import|latency) ;; *) echo "MODE must be import or latency" >&2; exit 2 ;; esac
[ -x "$HFS_BIN" ] || { echo "HFS_BIN $HFS_BIN is not executable" >&2; exit 2; }
for port in "$BASE_PORT" "$PG_PORT" "$ES_PORT" "$HFS_PORT"; do
  if ss -ltn "( sport = :$port )" 2>/dev/null | grep -q LISTEN; then
    echo "port $port is in use; set BASE_PORT" >&2; exit 2
  fi
done
mkdir -p "$RESULTS_DIR" "$WORKDIR"

SAMPLER_PID=""; HFS_PID=""; IMPORT_PID=""
stop_run() {
  [ -n "$SAMPLER_PID" ] && kill "$SAMPLER_PID" 2>/dev/null
  [ -n "$IMPORT_PID" ] && kill "$IMPORT_PID" 2>/dev/null
  [ -n "$HFS_PID" ] && kill "$HFS_PID" 2>/dev/null && wait "$HFS_PID" 2>/dev/null
  SAMPLER_PID=""; HFS_PID=""; IMPORT_PID=""
  timeout 120 docker rm -fv "$PG_NAME" "$ES_NAME" >/dev/null 2>&1 || true
}
trap stop_run EXIT

start_services() {
  timeout 300 docker run -d --name "$PG_NAME" -p "127.0.0.1:$PG_PORT:5432" \
    -e POSTGRES_PASSWORD=postgres postgres:16 -c max_connections=200 >/dev/null || return 1
  timeout 300 docker run -d --name "$ES_NAME" -p "127.0.0.1:$ES_PORT:9200" \
    -e discovery.type=single-node -e xpack.security.enabled=false \
    -e "ES_JAVA_OPTS=-Xms${ES_HEAP_MB}m -Xmx${ES_HEAP_MB}m" elasticsearch:8.15.0 >/dev/null || return 1
  local i
  for i in $(seq 1 90); do
    docker exec "$PG_NAME" pg_isready -U postgres >/dev/null 2>&1 \
      && curl -fs "$ES_URL/_cluster/health?wait_for_status=yellow&timeout=5s" >/dev/null 2>&1 && return 0
    sleep 2
  done
  echo "services did not become ready" >&2; return 1
}

hfs_env() {  # $1 = refresh interval, $2 = write refresh
  export HFS_STORAGE_BACKEND=postgres-elasticsearch
  export HFS_DATABASE_URL="postgresql://postgres:postgres@127.0.0.1:$PG_PORT/postgres"
  export HFS_ELASTICSEARCH_NODES="$ES_URL"
  export HFS_ELASTICSEARCH_REFRESH_INTERVAL="$1"
  export HFS_ELASTICSEARCH_WRITE_REFRESH="$2"
  export HFS_COMPOSITE_SYNC_MODE="$SYNC_MODE"
  # The import settings recommended in crates/rest/README.md; override from the caller.
  export HFS_BULK_SUBMIT_DEFER_INDEXING="${HFS_BULK_SUBMIT_DEFER_INDEXING:-false}"
  export HFS_ELASTICSEARCH_REINDEX_REFRESH="${HFS_ELASTICSEARCH_REINDEX_REFRESH:-false}"
  export HFS_ELASTICSEARCH_BULK_CONCURRENCY="${HFS_ELASTICSEARCH_BULK_CONCURRENCY:-4}"
  export HFS_SERVER_HOST=127.0.0.1 HFS_SERVER_PORT="$HFS_PORT" HFS_BASE_URL="$HFS_BASE"
  export HFS_LOG_LEVEL=info HFS_REQUEST_TIMEOUT=900 HFS_MAX_BODY_SIZE=209715200
  export HFS_PG_MAX_CONNECTIONS="${HFS_PG_MAX_CONNECTIONS:-32}"
  export HFS_BULK_SUBMIT_OUTPUT_DIR="$WORKDIR/submit"
  export HFS_BULK_SUBMIT_POLL_RATE_LIMIT=0
  export HFS_DATA_DIR="$WORKDIR/data"
  mkdir -p "$HFS_BULK_SUBMIT_OUTPUT_DIR" "$HFS_DATA_DIR"
}

refresh_stats() {  # $1 = output file; refresh.total and total_time_in_millis for hfs*
  curl -fs "$ES_URL/hfs*/_stats/refresh" > "$1" 2>/dev/null || echo '{}' > "$1"
}
stat_of() {  # $1 = file, $2 = field of _all.total.refresh
  python3 -I - "$1" "$2" <<'PY' 2>/dev/null || echo ""
import json, sys
d = json.load(open(sys.argv[1]))
print(d.get("_all", {}).get("total", {}).get("refresh", {}).get(sys.argv[2], ""))
PY
}

# Every SAMPLE_S seconds: HFS RSS/CPU (pid listening on HFS_PORT) and the two containers' CPU.
sampler() {  # $1 = samples file
  echo -e "t\thfs_rss_kb\thfs_cpu\tes_cpu\tpg_cpu" > "$1"
  local t0 pid rss cpu es pg
  t0=$(date +%s)
  while true; do
    pid=$(fuser -n tcp "$HFS_PORT" 2>/dev/null | tr -d ' ')
    rss=""; cpu=""
    [ -n "$pid" ] && read -r rss cpu < <(ps -o rss=,pcpu= -p "$pid" 2>/dev/null)
    read -r es pg < <(timeout 20 docker stats --no-stream --format '{{.Name}} {{.CPUPerc}}' "$ES_NAME" "$PG_NAME" 2>/dev/null \
      | awk -v e="$ES_NAME" '{g=$2; sub("%","",g); if ($1==e) es=g; else pg=g} END {print es, pg}')
    echo -e "$(( $(date +%s) - t0 ))\t${rss:-}\t${cpu:-}\t${es:-}\t${pg:-}" >> "$1"
    sleep "$SAMPLE_S"
  done
}

start_hfs() {  # $1 = log file
  "$HFS_BIN" > "$1" 2>&1 &
  HFS_PID=$!
  local i
  for i in $(seq 1 120); do
    curl -fs "$HFS_BASE/metadata" >/dev/null 2>&1 && return 0
    kill -0 "$HFS_PID" 2>/dev/null || return 1
    sleep 1
  done
  return 1
}

# peak RSS (MB) and mean/max CPU per process from samples.tsv
sample_summary() {
  awk -F'\t' 'NR>1 {
      if ($2+0 > rss) rss=$2+0
      if ($3!="") {hs+=$3; hn++; if ($3+0>hm) hm=$3+0}
      if ($4!="") {es+=$4; en++; if ($4+0>em) em=$4+0}
      if ($5!="") {ps+=$5; pn++; if ($5+0>pm) pm=$5+0}
    } END {
      printf "peak_hfs_rss_mb=%.0f\nhfs_cpu_mean=%.1f\nhfs_cpu_max=%.1f\nes_cpu_mean=%.1f\nes_cpu_max=%.1f\npg_cpu_mean=%.1f\npg_cpu_max=%.1f\n",
        rss/1024, hn?hs/hn:0, hm, en?es/en:0, em, pn?ps/pn:0, pm }' "$1"
}

kv() { grep -m1 "^$2=" "$1" 2>/dev/null | cut -d= -f2-; }

run_import_arm() {  # $1 = arm name, $2 = interval, $3 = run dir
  local out="$3" before after rc
  rm -rf "$WORKDIR/submit" "$WORKDIR/data" "$WORKDIR/bi"; mkdir -p "$out"
  hfs_env "$2" "$WRITE_REFRESH"
  refresh_stats "$out/refresh-before.json"
  sampler "$out/samples.tsv" & SAMPLER_PID=$!
  : > "$out/hfs.log"
  timeout -k 60 $((RUN_TIMEOUT_S + 900)) python3 "$HERE/bulk_import.py" \
    --source "$BUNDLE_SOURCE" --hfs-bin "$HFS_BIN" --port "$HFS_PORT" \
    --workdir "$WORKDIR/bi" --results-dir "$out" --hfs-log "$out/hfs.log" \
    --bundles "$BUNDLES" --timeout-s "$RUN_TIMEOUT_S" --searchable-wait-s 600 \
    --submission-id "refresh-sweep-$1"
  rc=$?
  refresh_stats "$out/refresh-after.json"
  kill "$SAMPLER_PID" 2>/dev/null; SAMPLER_PID=""
  {
    echo "arm=$1"; echo "interval=$2"; echo "write_refresh=$WRITE_REFRESH"; echo "cores=$(nproc)"
    echo "bulk_import_rc=$rc"
    echo "refresh_total_before=$(stat_of "$out/refresh-before.json" total)"
    echo "refresh_total_after=$(stat_of "$out/refresh-after.json" total)"
    echo "refresh_millis_before=$(stat_of "$out/refresh-before.json" total_time_in_millis)"
    echo "refresh_millis_after=$(stat_of "$out/refresh-after.json" total_time_in_millis)"
    echo "es_write_rejected=$(curl -fs "$ES_URL/_nodes/stats/thread_pool" 2>/dev/null \
      | python3 -I -c 'import json,sys; d=json.load(sys.stdin); print(sum(n["thread_pool"]["write"]["rejected"] for n in d["nodes"].values()))' 2>/dev/null)"
    echo "rejected_pages_logged=$(grep -c 'rejected permanently' "$out/hfs.log" 2>/dev/null)"
    sample_summary "$out/samples.tsv"
  } > "$out/run.txt"
  local bi="$out/bulk-import.txt"
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$1" "$2" "$(kv "$bi" status)" \
    "$(kv "$bi" ingest_seconds)" "$(kv "$bi" total_seconds)" "$(kv "$bi" searchable_seconds)" \
    "$(kv "$bi" resources_per_s)" "$(kv "$bi" resources_ok)" \
    "$(( $(kv "$out/run.txt" refresh_total_after) - $(kv "$out/run.txt" refresh_total_before) ))" \
    "$(kv "$out/run.txt" peak_hfs_rss_mb)" "$(kv "$out/run.txt" rejected_pages_logged)" >> "$RESULTS_DIR/summary.tsv"
}

latency_pass() {  # $1 = out file, $2 = extra args...
  local out="$1"; shift
  python3 "$HERE/write_latency.py" --base-url "$HFS_BASE" --es-url "$ES_URL" \
    ${BATCH_FILE:+--batch-file "$BATCH_FILE"} ${TRANSACTION_FILE:+--transaction-file "$TRANSACTION_FILE"} \
    --timeout-s 300 --out "$out" "$@" > /dev/null
}

run_latency_arm() {  # $1 = arm name ("<interval>-<write_refresh>"), $2 = interval, $3 = run dir, $4 = write refresh
  local out="$3"
  rm -rf "$WORKDIR/submit" "$WORKDIR/data" "$WORKDIR/bi"; mkdir -p "$out"
  hfs_env "$2" "$4"
  start_hfs "$out/hfs.log" || { echo "HFS did not start ($1)" >&2; return 1; }
  # Mode C runs only on the idle pass: it needs a type never written on this server.
  latency_pass "$out/latency-idle.json" --label "$1 idle" --new-type "$NEW_TYPE"
  if [ "$LAT_DURING_IMPORT" = "1" ]; then
    python3 "$HERE/bulk_import.py" --source "$BUNDLE_SOURCE" --base-url "$HFS_BASE" \
      --workdir "$WORKDIR/bi" --results-dir "$out" --hfs-log "$out/hfs.log" \
      --bundles "$BUNDLES" --timeout-s "$RUN_TIMEOUT_S" --submission-id "refresh-sweep-lat-$1" \
      > "$out/bulk-import.out" 2>&1 & IMPORT_PID=$!
    sleep 30  # let the import get going before probing
    latency_pass "$out/latency-import.json" --label "$1 during import"
    wait "$IMPORT_PID" 2>/dev/null; IMPORT_PID=""
  fi
  printf '%s\t%s\t%s\n' "$1" "$2" "$4" >> "$RESULTS_DIR/summary.tsv"
}

# arms: "<name>|<interval>|<write refresh>"
ARMS=()
if [ "$MODE" = "import" ]; then
  for iv in $INTERVALS; do ARMS+=("$iv|$iv|$WRITE_REFRESH"); done
else
  for iv in $INTERVALS; do for wr in $LAT_WRITE_REFRESH; do ARMS+=("$iv-$wr|$iv|$wr"); done; done
fi
NARMS=${#ARMS[@]}
if [ "$MODE" = "import" ]; then
  echo -e "arm\tinterval\tstatus\tingest_s\ttotal_s\tsearchable_s\tresources_per_s\tresources_ok\trefreshes\tpeak_hfs_rss_mb\trejected_pages" > "$RESULTS_DIR/summary.tsv"
else
  echo -e "arm\tinterval\twrite_refresh" > "$RESULTS_DIR/summary.tsv"
fi
echo "refresh sweep: mode=$MODE arms=$NARMS reps=$REPS cores=$(nproc) results=$RESULTS_DIR"

for (( round = 1; round <= REPS; round++ )); do
  for (( j = 0; j < NARMS; j++ )); do
    idx=$(( (j + round - 1) % NARMS ))
    IFS='|' read -r name interval wr <<< "${ARMS[$idx]}"
    dir="$RESULTS_DIR/${name}-r${round}"
    echo "== round $round arm $name (interval $interval, write_refresh $wr)"
    if start_services; then
      if [ "$MODE" = "import" ]; then run_import_arm "$name" "$interval" "$dir"
      else run_latency_arm "$name" "$interval" "$dir" "$wr"; fi
    else
      echo "arm $name round $round: services failed" >&2
    fi
    stop_run
  done
done
echo "summary: $RESULTS_DIR/summary.tsv"
