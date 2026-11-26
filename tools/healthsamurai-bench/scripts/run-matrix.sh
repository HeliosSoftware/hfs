#!/bin/bash
# HFS (postgres) vs WSO2, upstream suite order, 3 rounds, alternating which server goes first,
# clean database before every server run, only one server running at a time.
# Logs go to results-local/ in the benchmark clone.
export MSYS2_ARG_CONV_EXCL="*"
BENCH="${BENCH:?set BENCH to the upstream benchmark clone (with upstream-e6a4941.patch applied and the compose files copied in)}"
cd "$BENCH" || exit 1
OUT="$BENCH/results-local"
mkdir -p "$OUT"
F="-f docker-compose.yaml -f docker-compose.hfs.yaml"
ROUNDS="${ROUNDS:-1 2 3}"

stop_server() {
  docker compose $F stop "$1" "$1-checker"
}

reset_db() {
  s=$1
  echo "[$(date -u +%H:%M:%S)] reset database for $s"
  docker compose $F up -d --wait postgres prometheus tgz
  docker compose $F stop "$s" "$s-checker"
  docker exec perf-postgres psql -U postgres -c "DROP DATABASE IF EXISTS $s WITH (FORCE)" -c "CREATE DATABASE $s"
  docker compose $F up -d --wait "$s" "$s-checker"
}

running() {
  [ "$(docker inspect -f '{{.State.Running}}' "perf-$1" 2>/dev/null)" = "true" ]
}

run_server() {
  round=$1; s=$2; other=$3
  stop_server "$other"
  reset_db "$s"
  for t in prewarm crud import search; do
    if ! running "$s"; then
      echo "[$(date -u +%H:%M:%S)] round $round $s $t SKIPPED: container not running (oom=$(docker inspect -f '{{.State.OOMKilled}}' perf-$s))"
      continue
    fi
    if [ "$t" = search ] && ! grep -q "checks_failed......: 0.00%" "$OUT/R$round-$s-import.log"; then
      echo "[$(date -u +%H:%M:%S)] round $round $s search SKIPPED: import had failures"
      continue
    fi
    echo "[$(date -u +%H:%M:%S)] round $round $s $t start"
    ./runner.sh -t "/k6/$t.js" -s "$s" -id "R$round-$s" $F > "$OUT/R$round-$s-$t.log" 2>&1
    echo "[$(date -u +%H:%M:%S)] round $round $s $t exit=$? oom=$(docker inspect -f '{{.State.OOMKilled}}' perf-$s)"
  done
}

for round in $ROUNDS; do
  if [ $((round % 2)) -eq 1 ]; then ORDER="hfs wso2"; else ORDER="wso2 hfs"; fi
  for s in $ORDER; do
    if [ "$s" = hfs ]; then other=wso2; else other=hfs; fi
    run_server "$round" "$s" "$other"
  done
done
echo "[$(date -u +%H:%M:%S)] ALL DONE"
