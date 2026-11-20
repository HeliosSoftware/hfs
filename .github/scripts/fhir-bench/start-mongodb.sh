#!/usr/bin/env bash
#
# Start an ephemeral single-member MongoDB replica set for the mongodb /
# mongodb-elasticsearch legs.
#
# Called from: the `benchmark` job's "Start ephemeral MongoDB (single-member
# replica set)" step (if: startsWith(matrix.backend, 'mongodb')).
#
# Single-member replica set: HFS runs each FHIR transaction Bundle as
# ONE multi-document transaction, which MongoDB only supports on a
# replica set. The member is registered as localhost:27017 —
# unreachable from the runner — so the client URL below uses
# directConnection=true (skips replica-set topology discovery), the
# same pattern as inferno-us-core.yml's Mongo wiring.
#
# Explicit engine caps, because this runs on the shared 4-CPU
# Docker host:
#   --wiredTigerCacheSizeGB  mongod's own default is ~50% of (host RAM
#                            - 1GB), which risks an OOM on this host
#                            (crates/persistence/tests/mongodb_tests.rs
#                            has a regression test for that failure
#                            mode).
#   transactionLifetimeLimitSeconds=900  mongod's default is 60s; import
#                            bundles run into the thousands of entries
#                            and already need HFS_REQUEST_TIMEOUT=900
#                            on the Postgres leg.
#   --oplogSize 2048         default is ~5% of free disk (~8GB here).
# Data lives in a labelled NAMED volume for the same reaper reason as
# Postgres's. The mongo image also declares an anonymous /data/configdb
# volume; that one is deliberately left unnamed — the workflow's "Stop
# ephemeral MongoDB" step's `docker rm -fv` and docker-host-gc's
# anonymous-volume sweep both already remove it, so a named/labelled
# volume for it would add reaper surface for no reclaim benefit.
#
# Required environment (exported by the workflow step's env:):
#   BACKEND                        matrix.backend
#   RUN_ID                         github.run_id
#   IN_MONGO_WT_CACHE_GB           inputs.mongo_wt_cache_gb
#   IN_HFS_MONGO_MAX_CONNECTIONS   inputs.hfs_mongo_max_connections
#
# Optional environment (not set by the workflow; for local runs):
#   IN_MONGO_IMAGE                 official mongo:<version> tag; empty = mongo:9.0.8
#
# Two settings for every image other than mongo:7.x (#2605):
#   -e MONGO_TCMALLOC_PER_CPU_CACHE_SIZE_BYTES=10485760
#                            mongod 8.x / 9.x size tcmalloc's per-CPU
#                            caches from the container's memory limit.
#                            This container has no memory limit (no --memory: the shared
#                            host's page cache is the point), so mongod
#                            sizes those caches at 0 and every malloc/free
#                            takes tcmalloc's central lock (SERVER-109273,
#                            https://jira.mongodb.org/browse/SERVER-109273):
#                            search -31% and sql-run -65% against 7.0.48.
#                            The variable gives the caches a real size and
#                            keeps MongoDB's per-CPU mode (no startup
#                            warning); `-e GLIBC_TUNABLES=` (set empty) only
#                            works around it by moving tcmalloc to
#                            per-thread caches (measured fallback).
#   --wiredTigerCollectionConfigString / --wiredTigerIndexConfigString
#     memory_page_max=16MB   8.3 and 9.0 stalled the import for 60 s or more
#                            with default pages; 16 MB pages removed the
#                            stalls with no insert p99 penalty (E-73, E-74
#                            in #2180).
# mongo:7.x gets neither: it ships the older tcmalloc (not affected) and
# never stalled. mongo-tcmalloc.sh reports which allocator state mongod
# ended up in; its verdict goes into the tuning table below.
#
# Also reads, already exported to $GITHUB_ENV by earlier steps:
#   MONGO_CONTAINER, MONGO_VOLUME, HFS_MONGODB_DATABASE   "Configure backend env"
#   DOCKER_HOST_IP                                        "Resolve Docker host and runner IP"
#
# Outputs (via $GITHUB_ENV): MONGO_VOL_FREE_MB, MONGO_PORT, HFS_DATABASE_URL,
# MONGO_TXN_LIFETIME_S. Also appends a tuning table to $GITHUB_STEP_SUMMARY.
set -euo pipefail

MONGO_WT_CACHE_GB="${IN_MONGO_WT_CACHE_GB:-3.5}"
if ! [[ "$MONGO_WT_CACHE_GB" =~ ^[0-9]+(\.[0-9]+)?$ ]] \
   || ! awk -v v="$MONGO_WT_CACHE_GB" 'BEGIN { exit !(v >= 0.25 && v <= 6) }'; then
  echo "::error::mongo_wt_cache_gb must be a number in 0.25..6 (got '$MONGO_WT_CACHE_GB'); the Docker host is shared with all CI"
  exit 1
fi
MONGO_IMAGE="${IN_MONGO_IMAGE:-mongo:9.0.8}"
# Official image only: the flags below (and mongosh inside the
# container) are what mongo:<tag> ships, and a free-text image name
# would otherwise be a way to run anything on the shared Docker host.
if ! [[ "$MONGO_IMAGE" =~ ^mongo:[0-9][0-9A-Za-z._-]*$ ]]; then
  echo "::error::IN_MONGO_IMAGE must be an official mongo:<version> tag, e.g. mongo:9.0.8 (got '$MONGO_IMAGE')"
  exit 1
fi
MONGO_PAGE_MAX=16MB
MONGO_TCMALLOC_PER_CPU_BYTES=10485760
MONGO_ENV_ARGS=()
MONGO_WT_ARGS=()
if [[ "$MONGO_IMAGE" =~ ^mongo:7($|[.-]) ]]; then
  MONGO_IS_7X=1
else
  MONGO_IS_7X=0
  MONGO_ENV_ARGS=(-e "MONGO_TCMALLOC_PER_CPU_CACHE_SIZE_BYTES=$MONGO_TCMALLOC_PER_CPU_BYTES")
  MONGO_WT_ARGS=(
    --wiredTigerCollectionConfigString "memory_page_max=$MONGO_PAGE_MAX"
    --wiredTigerIndexConfigString "memory_page_max=$MONGO_PAGE_MAX"
  )
fi
MONGO_TXN_LIFETIME_S=900
MONGO_OPLOG_MB=2048

docker rm -fv "$MONGO_CONTAINER" 2>/dev/null || true
docker volume rm -f "$MONGO_VOLUME" 2>/dev/null || true
docker volume create --label hfs-bench=1 \
  --label "hfs-bench-run=$RUN_ID" \
  --label "hfs-bench-leg=$BACKEND" "$MONGO_VOLUME" >/dev/null

# Volume filesystem free space at start (see the Postgres step for
# why this is the volume's own filesystem, not `docker system df`'s
# numbers). Never fails the leg.
MONGO_VOLDF_NAME="hfs-bench-voldf-$BACKEND-$RUN_ID-mongo"
MONGO_VOL_DF=$(timeout 60 docker run --rm --name "$MONGO_VOLDF_NAME" \
    --label hfs-bench=1 --label "hfs-bench-run=$RUN_ID" --label "hfs-bench-leg=$BACKEND" \
    -v "$MONGO_VOLUME:/v" alpine:3 df -Pm /v 2>/dev/null | awk 'NR==2{print $4}') || MONGO_VOL_DF=""
# See the Postgres step's identical note: `timeout` alone cannot be
# trusted to leave `--rm` a clean container to remove.
timeout 15 docker rm -f "$MONGO_VOLDF_NAME" >/dev/null 2>&1 || true
echo "MONGO_VOL_FREE_MB=${MONGO_VOL_DF:-unknown}" >> "$GITHUB_ENV"
echo "Mongo volume free space: ${MONGO_VOL_DF:-unknown} MB"

docker run -d \
  --name "$MONGO_CONTAINER" \
  --label hfs-bench=1 \
  --label hfs-ci=true \
  --label "hfs-bench-run=$RUN_ID" \
  --label "hfs-bench-leg=$BACKEND" \
  --log-driver local \
  -v "$MONGO_VOLUME:/data/db" \
  -p 0:27017 \
  ${MONGO_ENV_ARGS[@]+"${MONGO_ENV_ARGS[@]}"} \
  "$MONGO_IMAGE" \
  --replSet rs0 \
  --bind_ip_all \
  --wiredTigerCacheSizeGB "$MONGO_WT_CACHE_GB" \
  --oplogSize "$MONGO_OPLOG_MB" \
  --setParameter "transactionLifetimeLimitSeconds=$MONGO_TXN_LIFETIME_S" \
  ${MONGO_WT_ARGS[@]+"${MONGO_WT_ARGS[@]}"} >/dev/null

# 10s, not 60s: this is called from wall-clock-deadline loops below,
# and a mongod that accepts TCP but never answers (host memory
# pressure) would otherwise let a single call eat most of the
# loop's own budget before the deadline check ever runs again.
mongo_eval() { timeout 10 docker exec "$MONGO_CONTAINER" mongosh --quiet --eval "$1"; }

echo "Waiting for mongod..."
MONGO_PING_T0=$SECONDS
until mongo_eval 'db.adminCommand({ ping: 1 }).ok' >/dev/null 2>&1; do
  if [ $((SECONDS - MONGO_PING_T0)) -ge 60 ]; then
    echo "ERROR: mongod did not answer within 60s"
    timeout 60 docker logs --tail 100 "$MONGO_CONTAINER" 2>&1 || true
    exit 1
  fi
  sleep 2
done

mongo_eval 'try { rs.status(); } catch (e) { rs.initiate({ _id: "rs0", members: [{ _id: 0, host: "localhost:27017" }] }); }' >/dev/null 2>&1 || true

echo "Waiting for replica-set PRIMARY..."
MONGO_PORT=""
MONGO_PRIMARY_T0=$SECONDS
while :; do
  STATE=$(mongo_eval 'try { rs.status().myState } catch (e) { 0 }' 2>/dev/null | tr -d '\r\n ' || true)
  if [ "$STATE" = "1" ]; then
    MONGO_PORT=$(timeout 30 docker port "$MONGO_CONTAINER" 27017 2>/dev/null | head -1 | sed 's/.*://') || MONGO_PORT=""
    if [ -n "$MONGO_PORT" ] && timeout 2 bash -c "cat < /dev/null > /dev/tcp/$DOCKER_HOST_IP/$MONGO_PORT" 2>/dev/null; then
      echo "MongoDB PRIMARY reachable on $DOCKER_HOST_IP:$MONGO_PORT after $((SECONDS - MONGO_PRIMARY_T0))s"
      break
    fi
  fi
  if [ $((SECONDS - MONGO_PRIMARY_T0)) -ge 120 ]; then
    echo "ERROR: MongoDB replica set did not become a reachable PRIMARY within 120s"
    timeout 60 docker logs --tail 100 "$MONGO_CONTAINER" 2>&1 || true
    exit 1
  fi
  sleep 2
done

# Prove the caps took effect rather than trusting the flags. A single
# mongo_eval right after PRIMARY comes up can hit mongod still
# settling — reading that hiccup as "the setting is wrong" would be
# a false failure, so retry up to 5 times, 2s apart, and only treat
# it as an error once it is still unreadable after that (or once a
# value WAS read and definitely disagrees).
mongo_read_int() {
  local expr="$1" attempt out
  for attempt in 1 2 3 4 5; do
    out=$(mongo_eval "$expr" 2>/dev/null | tr -d '\r\n ') || out=""
    if [[ "$out" =~ ^[0-9]+$ ]]; then
      echo "$out"
      return 0
    fi
    [ "$attempt" -lt 5 ] && sleep 2
  done
  echo '?'
  return 1
}
WT_MAX_BYTES=$(mongo_read_int 'Number(db.serverStatus().wiredTiger.cache["maximum bytes configured"])') || true
TXN_LIFETIME=$(mongo_read_int 'Number(db.adminCommand({ getParameter: 1, transactionLifetimeLimitSeconds: 1 }).transactionLifetimeLimitSeconds)') || true
MONGO_SERVER_VERSION=$(mongo_eval 'db.version()' 2>/dev/null | tr -d '\r' | tail -n 1) || true
[[ "$MONGO_SERVER_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || MONGO_SERVER_VERSION=unknown
if [ "$MONGO_SERVER_VERSION" != "unknown" ] && [ "$MONGO_SERVER_VERSION" != "${MONGO_IMAGE#mongo:}" ]; then
  echo "::warning::mongod reports $MONGO_SERVER_VERSION but the image is $MONGO_IMAGE"
fi
echo "mongod version: $MONGO_SERVER_VERSION"
echo "WiredTiger cache max: $WT_MAX_BYTES bytes; transactionLifetimeLimitSeconds: $TXN_LIFETIME"
if [ "$TXN_LIFETIME" = '?' ]; then
  echo "::error::could not read transactionLifetimeLimitSeconds after 5 attempts (2s apart) — cannot confirm import transactions won't abort at mongod's 60s default"
  exit 1
fi
if [ "$TXN_LIFETIME" != "$MONGO_TXN_LIFETIME_S" ]; then
  echo "::error::transactionLifetimeLimitSeconds is '$TXN_LIFETIME', expected $MONGO_TXN_LIFETIME_S — import transactions would abort at mongod's 60s default"
  exit 1
fi

# What mongod parsed for the WiredTiger config strings (empty = none
# passed, the mongo:7.x case). Read back, not echoed from the flags.
MONGO_WT_CONFIG_STRINGS=$(mongo_eval 'const w = (db.adminCommand({ getCmdLineOpts: 1 }).parsed.storage || {}).wiredTiger || {}; print("collection=" + ((w.collectionConfig || {}).configString || "") + " index=" + ((w.indexConfig || {}).configString || ""))' 2>/dev/null | tr -d '\r' | tail -n 1) || MONGO_WT_CONFIG_STRINGS="?"
echo "WiredTiger config strings: ${MONGO_WT_CONFIG_STRINGS:-?}"
if [ "$MONGO_IS_7X" = 0 ] \
   && [ "$MONGO_WT_CONFIG_STRINGS" != "collection=memory_page_max=$MONGO_PAGE_MAX index=memory_page_max=$MONGO_PAGE_MAX" ]; then
  echo "::warning::mongod did not report memory_page_max=$MONGO_PAGE_MAX for collections and indexes (got '${MONGO_WT_CONFIG_STRINGS:-?}'); imports on 8.3/9.0 may stall"
fi

# Allocator state at startup (mongo-tcmalloc.sh never fails). The full
# report goes to the step log; its key fields go into the table below.
TCM_REPORT=$(bash "$(dirname "${BASH_SOURCE[0]}")/mongo-tcmalloc.sh" 2>&1) || true
echo "tcmalloc check (mongo-tcmalloc.sh):"
printf '%s\n' "$TCM_REPORT" | sed 's/^/  /'
tcm_field() { printf '%s\n' "$TCM_REPORT" | sed -n "s/^$1: //p" | tail -n 1; }
TCM_VERDICT=$(tcm_field verdict)
TCM_MAX_PER_CPU=$(tcm_field maxPerCPUCacheSizeBytes)
TCM_CPU_CACHE=$(tcm_field cpuCache)
TCM_ENV_SIZE=$(tcm_field container_MONGO_TCMALLOC_PER_CPU_CACHE_SIZE_BYTES)
TCM_ENV_GLIBC=$(tcm_field container_GLIBC_TUNABLES)
case "$TCM_VERDICT" in
  AFFECTED*) echo "::warning::mongod's tcmalloc per-CPU caches have no capacity (SERVER-109273): reads under concurrency will be several times slower; see start-mongodb.sh" ;;
  "")        echo "::warning::mongo-tcmalloc.sh gave no verdict; allocator state unknown" ;;
esac

{
  echo "MONGO_PORT=$MONGO_PORT"
  echo "HFS_DATABASE_URL=mongodb://$DOCKER_HOST_IP:$MONGO_PORT/?replicaSet=rs0&directConnection=true"
  # Recorded so "Run benchmark suites" can put it in runner-info.txt
  # — this step's own local $MONGO_TXN_LIFETIME_S isn't visible there.
  echo "MONGO_TXN_LIFETIME_S=$MONGO_TXN_LIFETIME_S"
} >> "$GITHUB_ENV"
{
  echo "### MongoDB tuning ($BACKEND)"
  echo ""
  echo "| setting | value |"
  echo "|---|---|"
  echo "| image | \`$MONGO_IMAGE\` — single-member replica set \`rs0\`, directConnection |"
  echo "| mongod version (db.version()) | \`$MONGO_SERVER_VERSION\` |"
  if [ "$MONGO_IS_7X" = 1 ]; then
    echo "| extra container env | none (mongo:7.x ships the older tcmalloc) |"
  else
    echo "| extra container env | \`MONGO_TCMALLOC_PER_CPU_CACHE_SIZE_BYTES=$MONGO_TCMALLOC_PER_CPU_BYTES\` (SERVER-109273; see start-mongodb.sh) |"
  fi
  echo "| tcmalloc (startup) | ${TCM_VERDICT:-unknown}; maxPerCPUCacheSizeBytes \`${TCM_MAX_PER_CPU:-?}\`; cpuCache \`${TCM_CPU_CACHE:-?}\`; container GLIBC_TUNABLES \`${TCM_ENV_GLIBC:-?}\`, MONGO_TCMALLOC_PER_CPU_CACHE_SIZE_BYTES \`${TCM_ENV_SIZE:-?}\` |"
  echo "| WiredTiger config strings (mongod parsed) | \`${MONGO_WT_CONFIG_STRINGS:-?}\` |"
  echo "| --wiredTigerCacheSizeGB | \`$MONGO_WT_CACHE_GB\` (mongod reports $WT_MAX_BYTES bytes) |"
  echo "| transactionLifetimeLimitSeconds | \`$TXN_LIFETIME\` |"
  echo "| --oplogSize | \`${MONGO_OPLOG_MB}MB\` |"
  echo "| HFS_MONGODB_MAX_CONNECTIONS | \`${IN_HFS_MONGO_MAX_CONNECTIONS:-32}\` |"
  echo "| HFS_MONGODB_DATABASE | \`$HFS_MONGODB_DATABASE\` |"
  echo "| data volume free space (start) | \`${MONGO_VOL_DF:-unknown} MB\` |"
} >> "$GITHUB_STEP_SUMMARY"
