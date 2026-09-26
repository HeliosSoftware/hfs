#!/usr/bin/env bash
#
# $bulk-submit on a composite (primary + Elasticsearch) deployment must leave
# its resources SEARCHABLE, not merely readable by id.
#
# Origin: issue #1021. `$bulk-submit` ingestion runs on the primary's engine,
# and the primary deliberately skips its own indexing when search is offloaded
# to Elasticsearch. #882 wrapped the primary's job store so a finished manifest
# syncs into the secondary — but only on sqlite-es and pg-es. mongo-es and
# s3-es kept passing the raw primary, each with a comment asserting the
# composite's search half was "fed by the primary's own indexing hooks", which
# on those deployments are exactly what is turned off. The result: 15.27M of
# 15.28M bulk-imported resources readable by id and invisible to every search,
# with `GET` by id passing every smoke test.
#
# This asserts, on mongo-es:
#
#   - the import finishes (poll -> 200)
#   - every imported resource is readable by id
#   - the searchable count EQUALS the imported count
#   - Elasticsearch's own doc count equals it too
#   - $reindex is advertised in /metadata and its OperationDefinition resolves
#
# The search assertion is the point: a version that only checked GET-by-id
# passed throughout the bug.
#
# Requirements: docker, curl, python3, cargo.
#
#   crates/hfs/tests/bulk_submit/run_composite_es_index_check.sh
#   COUNT=500 SKIP_BUILD=1 crates/hfs/tests/bulk_submit/run_composite_es_index_check.sh
#   DEFER=true crates/hfs/tests/bulk_submit/run_composite_es_index_check.sh
#
set -euo pipefail

cd "$(dirname "$0")/../../../.."

COUNT="${COUNT:-200}"
# Both settings must end searchable. `false` syncs at manifest completion
# through the composite wrapper; `true` (the default) defers to the
# post-manifest reindex. The bug made `false` index nothing at all.
DEFER="${DEFER:-false}"
WORKDIR="${WORKDIR:-/tmp/hfs-composite-es-check}"
HFS_PORT="${HFS_PORT:-8930}"
PROVIDER_PORT="${PROVIDER_PORT:-8931}"
MONGO_PORT="${MONGO_PORT:-27019}"
ES_PORT="${ES_PORT:-9201}"
MONGO_NAME="hfs1021-check-mongo"
ES_NAME="hfs1021-check-es"

cleanup() {
  [ -n "${HFS_PID:-}" ] && kill "$HFS_PID" 2>/dev/null || true
  [ -n "${PROVIDER_PID:-}" ] && kill "$PROVIDER_PID" 2>/dev/null || true
  docker rm -f "$MONGO_NAME" "$ES_NAME" >/dev/null 2>&1 || true
}
trap cleanup EXIT

rm -rf "$WORKDIR"; mkdir -p "$WORKDIR"

echo "==> starting MongoDB (replica set) and Elasticsearch"
docker rm -f "$MONGO_NAME" "$ES_NAME" >/dev/null 2>&1 || true
# The replica-set member host must match the published port, or the driver
# rediscovers the advertised host:port and every operation dies with
# ReplicaSetNoPrimary.
docker run -d --name "$MONGO_NAME" -p "$MONGO_PORT:$MONGO_PORT" mongo:7.0 \
  --port "$MONGO_PORT" --replSet rscheck --bind_ip_all >/dev/null
docker run -d --name "$ES_NAME" -p "$ES_PORT:9200" \
  -e discovery.type=single-node -e xpack.security.enabled=false \
  -e "ES_JAVA_OPTS=-Xms1g -Xmx1g" elasticsearch:8.15.0 >/dev/null

until docker exec "$MONGO_NAME" mongosh --port "$MONGO_PORT" --quiet \
        --eval 'db.adminCommand({ping:1}).ok' 2>/dev/null | grep -q 1; do sleep 2; done
docker exec "$MONGO_NAME" mongosh --port "$MONGO_PORT" --quiet \
  --eval "rs.initiate({_id:'rscheck',members:[{_id:0,host:'localhost:$MONGO_PORT'}]})" >/dev/null
until docker exec "$MONGO_NAME" mongosh --port "$MONGO_PORT" --quiet \
        --eval 'db.hello().isWritablePrimary' 2>/dev/null | grep -q true; do sleep 2; done
until curl -sf "localhost:$ES_PORT/_cluster/health" >/dev/null 2>&1; do sleep 3; done
echo "    up"

echo "==> generating $COUNT Patients"
python3 - "$WORKDIR" "$COUNT" "http://127.0.0.1:$PROVIDER_PORT" <<'PY'
import json, sys, pathlib
workdir, count, provider = sys.argv[1], int(sys.argv[2]), sys.argv[3]
with open(pathlib.Path(workdir) / "patients.ndjson", "w") as fh:
    for i in range(count):
        fh.write(json.dumps({
            "resourceType": "Patient", "id": f"ces-{i}",
            "name": [{"family": "CompositeCheck", "given": [f"P{i}"]}],
            "gender": "female" if i % 2 == 0 else "male",
        }) + "\n")
pathlib.Path(workdir, "manifest.json").write_text(json.dumps({
    "transactionTime": "2024-01-01T00:00:00Z",
    "request": f"{provider}/manifest.json",
    "requiresAccessToken": False,
    "output": [{"type": "Patient", "url": f"{provider}/patients.ndjson", "count": count}],
    "error": [], "deleted": [],
}))
PY

if [ -z "${SKIP_BUILD:-}" ]; then
  echo "==> building hfs (mongodb,elasticsearch)"
  cargo build --release -p helios-hfs --features mongodb,elasticsearch
fi

( cd "$WORKDIR" && python3 -u -m http.server "$PROVIDER_PORT" --bind 127.0.0.1 >/dev/null 2>&1 ) &
PROVIDER_PID=$!
sleep 2

echo "==> starting hfs in mongo-es mode (HFS_BULK_SUBMIT_DEFER_INDEXING=$DEFER)"
HFS_STORAGE_BACKEND=mongo-es \
HFS_MONGODB_URL="mongodb://localhost:$MONGO_PORT/?replicaSet=rscheck" \
HFS_MONGODB_DATABASE=helios_check \
HFS_ELASTICSEARCH_NODES="http://localhost:$ES_PORT" \
HFS_COMPOSITE_SYNC_MODE=synchronous \
HFS_BULK_SUBMIT_DEFER_INDEXING="$DEFER" \
HFS_SERVER_PORT="$HFS_PORT" \
HFS_BASE_URL="http://localhost:$HFS_PORT" \
HFS_UI_ENABLED=false \
HFS_BULK_SUBMIT_OUTPUT_DIR="$WORKDIR/out" \
  ./target/release/hfs > "$WORKDIR/hfs.log" 2>&1 &
HFS_PID=$!
until curl -sf "localhost:$HFS_PORT/metadata" -o /dev/null 2>/dev/null; do sleep 3; done
echo "    up"

cat > "$WORKDIR/submit.json" <<EOF
{ "resourceType": "Parameters", "parameter": [
  { "name": "submitter", "valueIdentifier": { "system": "http://example.org", "value": "ces" } },
  { "name": "submissionId", "valueString": "composite-es-check" },
  { "name": "manifestUrl", "valueUrl": "http://127.0.0.1:$PROVIDER_PORT/manifest.json" },
  { "name": "fhirBaseUrl", "valueUrl": "http://127.0.0.1:$PROVIDER_PORT/fhir" },
  { "name": "submissionStatus", "valueCoding": { "system": "http://hl7.org/fhir/event-status", "code": "completed" } } ] }
EOF
cat > "$WORKDIR/status.json" <<'EOF'
{ "resourceType": "Parameters", "parameter": [
  { "name": "submitter", "valueIdentifier": { "system": "http://example.org", "value": "ces" } },
  { "name": "submissionId", "valueString": "composite-es-check" } ] }
EOF

echo "==> kick-off"
curl -sS -o /dev/null -w '    HTTP %{http_code}\n' -X POST "localhost:$HFS_PORT/\$bulk-submit" \
  -H 'Content-Type: application/fhir+json' --data-binary @"$WORKDIR/submit.json"
TOKEN=$(curl -sS -D - -o /dev/null -X POST "localhost:$HFS_PORT/\$bulk-submit-status" \
  -H 'Content-Type: application/fhir+json' --data-binary @"$WORKDIR/status.json" \
  | tr -d '\r' | awk -F': ' 'tolower($1)=="content-location"{print $2}' | xargs basename)

for i in $(seq 1 "${MAX_POLLS:-30}"); do
  CODE=$(curl -sS -o "$WORKDIR/poll.json" -w "%{http_code}" "localhost:$HFS_PORT/bulk-submit-status/$TOKEN")
  echo "    poll $i: HTTP $CODE"
  [ "$CODE" = "200" ] && break
  sleep 8
done
[ "$CODE" = "200" ] || { echo "FAIL: submission never reached 200"; exit 1; }

# Deferred indexing rebuilds after the manifest is already terminal, so give the
# fire-and-forget reindex a moment before counting.
sleep 10
curl -s "localhost:$ES_PORT/_refresh" -o /dev/null

echo
echo "=== results ==="
MONGO_COUNT=$(docker exec "$MONGO_NAME" mongosh --port "$MONGO_PORT" helios_check --quiet \
  --eval 'db.resources.countDocuments({resource_type:"Patient", id:{$regex:"^ces-"}})')
READ_CODE=$(curl -s -o /dev/null -w '%{http_code}' "localhost:$HFS_PORT/Patient/ces-7")
SEARCH_COUNT=$(curl -s "localhost:$HFS_PORT/Patient?family=CompositeCheck&_summary=count" \
  | python3 -c 'import json,sys; print(json.load(sys.stdin).get("total") or 0)')
ES_COUNT=$(curl -s "localhost:$ES_PORT/hfs_default_patient/_count" \
  | python3 -c 'import json,sys; print(json.load(sys.stdin).get("count",0))' 2>/dev/null || echo 0)

echo "  imported into MongoDB : $MONGO_COUNT"
echo "  GET /Patient/ces-7    : $READ_CODE"
echo "  searchable            : $SEARCH_COUNT"
echo "  Elasticsearch docs    : $ES_COUNT"

FAILED=0
[ "$MONGO_COUNT" = "$COUNT" ] || { echo "FAIL: MongoDB holds $MONGO_COUNT of $COUNT"; FAILED=1; }
[ "$READ_CODE" = "200" ]      || { echo "FAIL: read by id returned $READ_CODE"; FAILED=1; }
[ "$SEARCH_COUNT" = "$COUNT" ] || { echo "FAIL: $SEARCH_COUNT of $COUNT searchable — this is #1021"; FAILED=1; }
[ "$ES_COUNT" = "$COUNT" ]    || { echo "FAIL: Elasticsearch holds $ES_COUNT of $COUNT"; FAILED=1; }

# #1021 also asked that the recovery path be discoverable.
OPS=$(curl -s "localhost:$HFS_PORT/metadata" \
  | python3 -c 'import json,sys; print(" ".join(o["name"] for o in json.load(sys.stdin)["rest"][0].get("operation",[])))')
case "$OPS" in
  *reindex*) echo "  /metadata advertises  : reindex" ;;
  *) echo "FAIL: \$reindex is not advertised in CapabilityStatement"; FAILED=1 ;;
esac
DEF_CODE=$(curl -s -o /dev/null -w '%{http_code}' "localhost:$HFS_PORT/OperationDefinition/hfs-reindex")
[ "$DEF_CODE" = "200" ] || { echo "FAIL: OperationDefinition/hfs-reindex returned $DEF_CODE"; FAILED=1; }

echo
[ "$FAILED" = "0" ] && echo "PASS" || { echo "FAILED"; exit 1; }
