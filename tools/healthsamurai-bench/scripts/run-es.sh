#!/bin/bash
# HFS postgres-elasticsearch (synchronous), upstream suite order, 3 rounds.
# Per round: fresh ES container+volume, fresh Postgres database, one server at a time.
# A drain gate runs between import and search (outside the search timing), as in HFS CI.
export MSYS2_ARG_CONV_EXCL="*"
BENCH="${BENCH:?set BENCH to the upstream benchmark clone (with upstream-e6a4941.patch applied and the compose files copied in)}"
cd "$BENCH" || exit 1
OUT="${OUT:-$BENCH/results-local-es}"
mkdir -p "$OUT"
F="-f docker-compose.yaml -f docker-compose.hfs.yaml -f ${ES_COMPOSE:-docker-compose.hfses.yaml}"
ROUNDS="${ROUNDS:-1 2 3}"
S=hfses

ts() { date -u +%H:%M:%S; }

reset_all() {
  echo "[$(ts)] reset: stop $S, recreate ES volume, recreate database"
  docker compose $F up -d --wait postgres prometheus tgz
  docker compose $F stop $S $S-checker
  docker compose $F rm -sf $S $S-checker elasticsearch
  docker volume ls -q --filter name=esdata | while read v; do docker volume rm -f "$v"; done
  docker stop perf-hfs perf-wso2 2>/dev/null
  docker exec perf-postgres psql -U postgres -c "DROP DATABASE IF EXISTS $S WITH (FORCE)" -c "CREATE DATABASE $S"
  docker compose $F up -d --wait elasticsearch $S $S-checker
}

running() { [ "$(docker inspect -f '{{.State.Running}}' "perf-$1" 2>/dev/null)" = "true" ]; }

snap() { docker stats --no-stream --format '{{.Name}} cpu={{.CPUPerc}} mem={{.MemUsage}}' perf-$S perf-es perf-postgres >> "$OUT/R$1-stats.log"; }

drain_gate() {
  round=$1; L="$OUT/R$round-$S-drain.log"
  {
    echo "[$(ts)] drain gate start"
    code=$(curl -s -o /dev/null -w '%{http_code}' -X DELETE "http://localhost:13131/Patient?identifier=urn:hfs-bench:drain-probe%7Cr$round")
    echo "probe DELETE http=$code"
    prev=""; same=0; t0=$SECONDS; status=timeout
    while [ $((SECONDS - t0)) -lt 600 ]; do
      cur=$(docker exec perf-es curl -s "localhost:9200/_cat/count/hfs_*?h=count" | tr -d '[:space:]')
      idx=$(docker exec perf-es curl -s "localhost:9200/hfs_*/_stats/indexing" | grep -o '"index_total":[0-9]*' | head -1)
      key="$cur|$idx"
      if [ "$key" = "$prev" ]; then same=$((same + 1)); else same=0; fi
      prev=$key
      echo "[$(ts)] es_count=$cur $idx stable_polls=$same"
      if [ $same -ge 2 ]; then status=drained; break; fi
      sleep 5
    done
    docker exec perf-es curl -s -X POST "localhost:9200/hfs_*/_refresh" > /dev/null
    escount=$(docker exec perf-es curl -s "localhost:9200/_cat/count/hfs_*?h=count" | tr -d '[:space:]')
    pgcount=$(docker exec perf-postgres psql -U postgres -d $S -Atc "select count(*) from resources where not is_deleted" 2>&1)
    echo "status=$status es_count=$escount pg_count=$pgcount"
    echo "needs_reindex: $(curl -s localhost:13131/metrics | grep composite_secondary_sync_needs_reindex | tr '\n' ' ')"
    echo "es_health: $(docker exec perf-es curl -s 'localhost:9200/_cluster/health' | tr -d '\n')"
    echo "es_readonly_blocks: $(docker exec perf-es curl -s 'localhost:9200/hfs_*/_settings/index.blocks.read_only_allow_delete' | head -c 300)"
    echo "[$(ts)] drain gate end"
  } > "$L" 2>&1
  tail -6 "$L"
}

for round in $ROUNDS; do
  reset_all
  ok=0; for i in 1 2 3 4 5 6; do if docker logs perf-$S 2>&1 | grep -q "Composite storage initialized"; then ok=1; break; fi; sleep 10; done
  if [ $ok -ne 1 ]; then
    echo "[$(ts)] round $round ABORT: composite storage not initialized (HFS did not start with Elasticsearch)"
    continue
  fi
  for t in prewarm crud import search; do
    if ! running $S || ! running es; then
      echo "[$(ts)] round $round $t SKIPPED: container not running (hfs oom=$(docker inspect -f '{{.State.OOMKilled}}' perf-$S) es oom=$(docker inspect -f '{{.State.OOMKilled}}' perf-es))"
      continue
    fi
    if [ "$t" = search ]; then
      if ! grep -q "checks_failed......: 0.00%" "$OUT/R$round-$S-import.log"; then
        echo "[$(ts)] round $round search SKIPPED: import had failures"; continue
      fi
      drain_gate "$round"
    fi
    echo "[$(ts)] round $round $t start"
    ./runner.sh -t "/k6/$t.js" -s $S -id "R$round-$S" $F > "$OUT/R$round-$S-$t.log" 2>&1
    echo "[$(ts)] round $round $t exit=$? hfs_oom=$(docker inspect -f '{{.State.OOMKilled}}' perf-$S) es_oom=$(docker inspect -f '{{.State.OOMKilled}}' perf-es)"
    snap "$round"
  done
done
echo "[$(ts)] ALL DONE"
