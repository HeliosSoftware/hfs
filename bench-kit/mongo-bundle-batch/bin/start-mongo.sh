#!/usr/bin/env bash
# Start the single-member MongoDB replica set used by the benchmark (same flags as
# .github/scripts/fhir-bench/start-mongodb.sh and as the bench-opt-mongo container of the original runs).
# Works with Docker Desktop (WSL integration) and with docker engine inside WSL: only the `docker` CLI is used.
# Usage: bin/start-mongo.sh            (env knobs below)
#   MONGO_CONTAINER (hfs-bench-mongo)  MONGO_PORT (27017, bound to 127.0.0.1)  MONGO_WT_CACHE_GB (2)
#   MONGO_MEM_LIMIT (6g, container hard cap; the original box used 6 GB)  MONGO_IMAGE (mongo:7.0)
#   MONGO_VOLUME (hfs-bench-mongo-data)
set -euo pipefail
C=${MONGO_CONTAINER:-hfs-bench-mongo}; PORT=${MONGO_PORT:-27017}; CACHE=${MONGO_WT_CACHE_GB:-2}
MEM=${MONGO_MEM_LIMIT:-6g}; IMG=${MONGO_IMAGE:-mongo:7.0}; VOL=${MONGO_VOLUME:-hfs-bench-mongo-data}
docker rm -fv "$C" >/dev/null 2>&1 || true
docker volume rm -f "$VOL" >/dev/null 2>&1 || true   # always start from an empty data volume
docker volume create "$VOL" >/dev/null
# --memory == --memory-swap: no swap for the container (as in the original). nofile 524288 as in the original.
# oplogSize 2048 MB and transactionLifetimeLimitSeconds=900 because Bundles of ~1,600 entries are one transaction.
docker run -d --name "$C" --memory "$MEM" --memory-swap "$MEM" --ulimit nofile=524288:524288 \
  -v "$VOL:/data/db" -p "127.0.0.1:$PORT:27017" "$IMG" \
  --replSet rs0 --bind_ip_all --wiredTigerCacheSizeGB "$CACHE" --oplogSize 2048 \
  --setParameter transactionLifetimeLimitSeconds=900 >/dev/null
ev() { timeout 15 docker exec "$C" mongosh --quiet --eval "$1"; }
for i in $(seq 1 60); do ev 'db.adminCommand({ping:1}).ok' >/dev/null 2>&1 && break; sleep 2; done
ev 'try { rs.status() } catch (e) { rs.initiate({_id:"rs0",members:[{_id:0,host:"localhost:27017"}]}) }' >/dev/null 2>&1 || true
for i in $(seq 1 60); do [ "$(ev 'try{rs.status().myState}catch(e){0}' 2>/dev/null | tr -d '\r\n ')" = 1 ] && break; sleep 2; done
[ "$(ev 'try{rs.status().myState}catch(e){0}' | tr -d '\r\n ')" = 1 ] || { echo "replica set did not reach PRIMARY" >&2; docker logs --tail 50 "$C" >&2; exit 1; }
echo "MongoDB $(ev 'db.version()') PRIMARY in container $C, host port 127.0.0.1:$PORT"
echo "txn lifetime: $(ev 'db.adminCommand({getParameter:1,transactionLifetimeLimitSeconds:1}).transactionLifetimeLimitSeconds')"
echo "HFS URL form: mongodb://127.0.0.1:$PORT/<db>?replicaSet=rs0&directConnection=true  (directConnection is required: the member is registered as localhost:27017)"
