#!/usr/bin/env bash
#
# Start an ephemeral single-node Elasticsearch for the *-elasticsearch legs.
#
# Called from: the `benchmark` job's "Start ephemeral Elasticsearch" step
# (if: endsWith(matrix.backend, '-elasticsearch')).
#
# Heap from the es_heap input, already resolved to MB by "Resolve
# matrix from backend input" (setup job / resolve-matrix.sh) so this step
# never re-parses the unit suffix. The -Xlog flags match the rest of this
# repo's Elasticsearch containers: the default jvm.options otherwise writes a
# rotating gc.log into the container's writable layer — a resource
# #453 (a repo-wide outage) exhausted.
# Data lives in a labelled NAMED volume, same reaper reasoning as
# Postgres/Mongo. Health gates accept YELLOW everywhere: HFS creates
# every index with number_of_replicas=1
# (elasticsearch/backend.rs default_replicas(), applied in schema.rs),
# which cannot be satisfied on a single node.
#
# Required environment (exported by the workflow step's env:):
#   BACKEND     matrix.backend
#   RUN_ID      github.run_id
#   ES_HEAP_MB  needs.setup.outputs.es_heap_mb
#
# Also reads, already exported to $GITHUB_ENV by earlier steps:
#   ES_CONTAINER, ES_VOLUME                                "Configure backend env"
#   DOCKER_HOST_IP                                         "Determine runner / Docker host IP"
#   HFS_COMPOSITE_SYNC_MODE, HFS_ELASTICSEARCH_WRITE_REFRESH,
#   HFS_ELASTICSEARCH_REFRESH_INTERVAL                     "Configure backend env" (summary only)
#
# Outputs (via $GITHUB_ENV): ES_VOL_FREE_MB, ES_PORT, HFS_ELASTICSEARCH_NODES.
# Also appends a tuning table to $GITHUB_STEP_SUMMARY.
set -euo pipefail
ES_HEAP_MB="${ES_HEAP_MB:-1024}"
ES_IMAGE="elasticsearch:8.15.0"

docker rm -fv "$ES_CONTAINER" 2>/dev/null || true
docker volume rm -f "$ES_VOLUME" 2>/dev/null || true
docker volume create --label hfs-bench=1 \
  --label "hfs-bench-run=$RUN_ID" \
  --label "hfs-bench-leg=$BACKEND" "$ES_VOLUME" >/dev/null

# Volume filesystem free space at start, and (ES only) how full it
# already is: indices go read-only at Elasticsearch's 95%
# flood-stage watermark, so warn well before that. Never fails the
# leg — this is a signal for calibration, not a gate.
ES_VOLDF_NAME="hfs-bench-voldf-$BACKEND-$RUN_ID-es"
ES_VOL_DF=$(timeout 60 docker run --rm --name "$ES_VOLDF_NAME" \
    --label hfs-bench=1 --label "hfs-bench-run=$RUN_ID" --label "hfs-bench-leg=$BACKEND" \
    -v "$ES_VOLUME:/v" alpine:3 df -Pm /v 2>/dev/null | awk 'NR==2{print $4, $5}') || ES_VOL_DF=""
# See the Postgres step's identical note: `timeout` alone cannot be
# trusted to leave `--rm` a clean container to remove.
timeout 15 docker rm -f "$ES_VOLDF_NAME" >/dev/null 2>&1 || true
ES_VOL_FREE_MB=$(echo "$ES_VOL_DF" | awk '{print $1}')
ES_VOL_PCT_USED=$(echo "$ES_VOL_DF" | awk '{gsub("%","",$2); print $2}')
echo "ES_VOL_FREE_MB=${ES_VOL_FREE_MB:-unknown}" >> "$GITHUB_ENV"
echo "Elasticsearch volume free space: ${ES_VOL_FREE_MB:-unknown} MB (${ES_VOL_PCT_USED:-unknown}% used)"
if [ -n "$ES_VOL_PCT_USED" ] && [[ "$ES_VOL_PCT_USED" =~ ^[0-9]+$ ]] && [ "$ES_VOL_PCT_USED" -gt 85 ]; then
  echo "::warning::Elasticsearch data volume $ES_VOLUME filesystem is ${ES_VOL_PCT_USED}% used — indices go read-only at Elasticsearch's 95% flood-stage watermark."
fi

docker run -d \
  --name "$ES_CONTAINER" \
  --label hfs-bench=1 \
  --label hfs-ci=true \
  --label "hfs-bench-run=$RUN_ID" \
  --label "hfs-bench-leg=$BACKEND" \
  --log-driver local \
  -v "$ES_VOLUME:/usr/share/elasticsearch/data" \
  -p 0:9200 \
  -e "discovery.type=single-node" \
  -e "xpack.security.enabled=false" \
  -e "ES_JAVA_OPTS=-Xms${ES_HEAP_MB}m -Xmx${ES_HEAP_MB}m -Xlog:disable -Xlog:all=warning:stderr" \
  "$ES_IMAGE" >/dev/null

echo "Waiting for Elasticsearch..."
ES_PORT=""
ES_READY_T0=$SECONDS
while :; do
  ES_PORT=$(timeout 30 docker port "$ES_CONTAINER" 9200 2>/dev/null | head -1 | sed 's/.*://') || ES_PORT=""
  # --max-time 5, not 30: this loop enforces its own 120s wall-clock
  # deadline below, so one slow/unresponsive curl must not itself
  # eat most of that budget before the deadline is checked again.
  if [ -n "$ES_PORT" ] && curl -sf --max-time 5 "http://$DOCKER_HOST_IP:$ES_PORT/_cluster/health?wait_for_status=yellow&timeout=1s" -o /dev/null 2>/dev/null; then
    echo "Elasticsearch ready on $DOCKER_HOST_IP:$ES_PORT after $((SECONDS - ES_READY_T0))s"
    break
  fi
  if [ $((SECONDS - ES_READY_T0)) -ge 120 ]; then
    echo "ERROR: Elasticsearch did not become ready within 120s"
    timeout 60 docker inspect -f 'OOMKilled={{.State.OOMKilled}} ExitCode={{.State.ExitCode}} Status={{.State.Status}}' "$ES_CONTAINER" || true
    timeout 60 docker logs --tail 100 "$ES_CONTAINER" 2>&1 || true
    exit 1
  fi
  sleep 2
done

echo "ES_PORT=$ES_PORT" >> "$GITHUB_ENV"
echo "HFS_ELASTICSEARCH_NODES=http://$DOCKER_HOST_IP:$ES_PORT" >> "$GITHUB_ENV"
ES_VERSION=$(curl -sf --max-time 30 "http://$DOCKER_HOST_IP:$ES_PORT/" | jq -r '.version.number' 2>/dev/null) || ES_VERSION='?'
{
  echo "### Elasticsearch tuning ($BACKEND)"
  echo ""
  echo "| setting | value |"
  echo "|---|---|"
  echo "| image | \`$ES_IMAGE\` (reports $ES_VERSION), single node |"
  echo "| JVM heap (-Xms = -Xmx) | \`${ES_HEAP_MB}m\` |"
  echo "| HFS_COMPOSITE_SYNC_MODE | \`${HFS_COMPOSITE_SYNC_MODE:-}\` |"
  echo "| HFS_ELASTICSEARCH_WRITE_REFRESH | \`${HFS_ELASTICSEARCH_WRITE_REFRESH:-}\` |"
  echo "| HFS_ELASTICSEARCH_REFRESH_INTERVAL | \`${HFS_ELASTICSEARCH_REFRESH_INTERVAL:-}\` |"
  echo "| HFS_COMPOSITE_SYNC_REPAIR_INTERVAL | \`60\` |"
  echo "| index replicas | 1 (hardcoded default in HFS) — cluster **yellow** on one node is expected |"
  echo "| data volume free space (start) | \`${ES_VOL_FREE_MB:-unknown} MB (${ES_VOL_PCT_USED:-unknown}% used)\` |"
} >> "$GITHUB_STEP_SUMMARY"
