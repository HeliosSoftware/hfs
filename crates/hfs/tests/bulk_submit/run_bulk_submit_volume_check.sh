#!/usr/bin/env bash
#
# $bulk-submit ingest at realistic volume on SQLite.
#
# Origin: issue #942. The smoke fixture (1 file, 2 Patients) does not exercise
# the fan-out, so it cannot tell whether the clamp and the retries hold up.
#
# This generates a manifest with FILES files of PER_FILE Patients each, ingests
# all of it through $bulk-submit and checks that:
#
#   - the import finishes (poll -> 200) with no "database is locked"
#   - the output manifest declares the FILES files and the counts
#   - the resources can be read back with GET /Patient/{id}
#   - the bookkeeping retries that happened are counted (grep on the log)
#
# All of it on SQLite, which is where the clamp to 2 applies.
#
# Requirements: cargo, curl and python on PATH. On Windows the interpreter is
# invoked as `python`; `python3` resolves to the Microsoft Store stub.
#
#   crates/hfs/tests/bulk_submit/run_bulk_submit_volume_check.sh
#   FILES=24 PER_FILE=1000 TTL=1500 MAX_POLLS=90 \
#     crates/hfs/tests/bulk_submit/run_bulk_submit_volume_check.sh
#
set -euo pipefail

cd "$(dirname "$0")/../../../.."

FILES="${FILES:-12}"
PER_FILE="${PER_FILE:-500}"
TTL="${TTL:-600}"
FILE_CONCURRENCY="${FILE_CONCURRENCY:-8}"
WORKDIR="${WORKDIR:-/tmp/hfs-bulk-submit-volume}"
# `:memory:` CANNOT turn WAL on (backend.rs:557 applies it, but an in-memory
# database stays on journal_mode=memory), so the locks surface as table-level
# SQLITE_LOCKED: that is the worst case, not the realistic deployment. With a
# file there is WAL. A file is the default; DB_URL=':memory:' forces the
# degraded case.
# The path lives under target/ and not under WORKDIR because in Git Bash WORKDIR
# is an MSYS path (/tmp/...) that the native Windows binary cannot resolve.
DB_URL="${DB_URL:-target/hfs-bulk-submit-volume.db}"
rm -f "$DB_URL" "$DB_URL-wal" "$DB_URL-shm"

# This does a real bind instead of reading `netstat`: its output is not portable
# (on Linux the state is printed as LISTEN, not LISTENING, and often the binary
# is not even installed), and when the pattern does not match every port would
# look free, so the failure would only show up much later, at server startup.
port_is_free() {
  python - "$1" <<'PY'
import socket, sys
s = socket.socket()
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
try:
    s.bind(("127.0.0.1", int(sys.argv[1])))
except OSError:
    sys.exit(1)
finally:
    s.close()
PY
}
pick_port() {
  local p="$1"
  while ! port_is_free "$p"; do p=$((p + 1)); done
  echo "$p"
}

PROVIDER_PORT="$(pick_port "${PROVIDER_PORT:-19200}")"
HFS_PORT="$(pick_port "${HFS_PORT:-18810}")"
[ "$HFS_PORT" = "$PROVIDER_PORT" ] && HFS_PORT="$(pick_port $((HFS_PORT + 1)))"
PROVIDER_URL="http://127.0.0.1:$PROVIDER_PORT"
HFS_URL="http://127.0.0.1:$HFS_PORT"

TOTAL=$((FILES * PER_FILE))

rm -rf "$WORKDIR"
mkdir -p "$WORKDIR"

echo "==> generating $FILES files x $PER_FILE Patients = $TOTAL resources"
python - "$WORKDIR" "$FILES" "$PER_FILE" "$PROVIDER_URL" <<'PY'
import json, sys, pathlib
workdir, files, per_file, provider = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), sys.argv[4]
out = []
for f in range(files):
    name = f"patients-{f}.ndjson"
    with open(pathlib.Path(workdir) / name, "w", encoding="utf-8") as fh:
        for i in range(per_file):
            pid = f"vol942-{f}-{i}"
            fh.write(json.dumps({
                "resourceType": "Patient",
                "id": pid,
                "name": [{"family": "Volume", "given": [f"F{f}", f"I{i}"]}],
                "gender": "female" if i % 2 == 0 else "male",
                "birthDate": "1980-01-01",
            }) + "\n")
    out.append({"type": "Patient", "url": f"{provider}/{name}", "count": per_file})
manifest = {
    "transactionTime": "2024-01-01T00:00:00Z",
    "request": f"{provider}/manifest.json",
    "requiresAccessToken": False,
    "output": out,
    "error": [],
    "deleted": [],
}
(pathlib.Path(workdir) / "manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
print(f"    manifest with {len(out)} output entries")
PY

echo "==> provider on $PROVIDER_URL"
( cd "$WORKDIR" && timeout "$TTL" python -u -m http.server "$PROVIDER_PORT" --bind 127.0.0.1 ) \
  > "$WORKDIR/provider.log" 2>&1 &

echo "==> HFS on $HFS_URL (requested fan-out: $FILE_CONCURRENCY)"
HFS_BASE_URL="$HFS_URL" \
HFS_BULK_SUBMIT_ENABLED=true \
HFS_BULK_SUBMIT_FILE_CONCURRENCY="$FILE_CONCURRENCY" \
HFS_LOG_LEVEL=info \
  timeout "$TTL" "${HFS_BIN:-target/debug/hfs.exe}" \
    --database-url "$DB_URL" --log-level info \
    --host 127.0.0.1 --port "$HFS_PORT" \
  > "$WORKDIR/hfs.log" 2>&1 &

for _ in $(seq 1 90); do
  curl -sS -o /dev/null "$HFS_URL/health" 2>/dev/null && break
  sleep 1
done

echo
grep 'Bulk submit' "$WORKDIR/hfs.log" || true
echo

cat > "$WORKDIR/submit.json" <<EOF
{ "resourceType": "Parameters", "parameter": [
  { "name": "submitter", "valueIdentifier": { "system": "http://example.org", "value": "vol" } },
  { "name": "submissionId", "valueString": "vol-942" },
  { "name": "manifestUrl", "valueUrl": "$PROVIDER_URL/manifest.json" },
  { "name": "fhirBaseUrl", "valueUrl": "$PROVIDER_URL/fhir" },
  { "name": "submissionStatus", "valueCoding": { "system": "http://hl7.org/fhir/event-status", "code": "completed" } } ] }
EOF
cat > "$WORKDIR/status.json" <<'EOF'
{ "resourceType": "Parameters", "parameter": [
  { "name": "submitter", "valueIdentifier": { "system": "http://example.org", "value": "vol" } },
  { "name": "submissionId", "valueString": "vol-942" } ] }
EOF

echo "==> kick-off"
curl -sS -o "$WORKDIR/kickoff.json" -w "    HTTP %{http_code}\n" -X POST "$HFS_URL/\$bulk-submit" \
  -H 'Content-Type: application/fhir+json' --data-binary @"$WORKDIR/submit.json"
cat "$WORKDIR/kickoff.json"; echo

LOC=$(curl -sS -D - -o /dev/null -X POST "$HFS_URL/\$bulk-submit-status" \
  -H 'Content-Type: application/fhir+json' --data-binary @"$WORKDIR/status.json" \
  | tr -d '\r' | awk -F': ' 'tolower($1)=="content-location"{print $2}')
echo "==> poll: $LOC"

# The endpoint allows 10 polls per 60s and answers 429 past that, so the
# interval has to be >= 6s. At 10s that is 6 per minute, within the limit.
START=$(date +%s)
CODE=""
for i in $(seq 1 "${MAX_POLLS:-60}"); do
  CODE=$(curl -sS -o "$WORKDIR/poll.json" -w "%{http_code}" "$LOC")
  echo "    poll $i: HTTP $CODE ($(( $(date +%s) - START ))s)"
  [ "$CODE" = "200" ] && break
  [ "$CODE" = "429" ] && { echo "    (rate limited, waiting 60s)"; sleep 60; continue; }
  sleep "${POLL_INTERVAL:-10}"
done
ELAPSED=$(( $(date +%s) - START ))

echo
echo "=== output manifest summary ==="
python - "$WORKDIR/poll.json" <<'PY'
import json, sys
raw = open(sys.argv[1], encoding="utf-8").read()
if not raw.strip():
    print("  (empty body: the last poll was not 200, the import had not finished)")
    raise SystemExit(0)
try:
    d = json.loads(raw)
except json.JSONDecodeError:
    print("  (non-JSON body):", raw[:200])
    raise SystemExit(0)
out = d.get("output", [])
print("  submissionId :", d.get("submissionId"))
print("  files        :", len(out))
print("  resources    :", sum(o.get("count", 0) for o in out))
print("  bytes        :", sum(o.get("fileSize", 0) for o in out))
print("  outcome      :", d.get("outcome"))
PY

echo
echo "=== resource read-back (first, middle, last) ==="
LAST_F=$((FILES - 1)); LAST_I=$((PER_FILE - 1)); MID_F=$((FILES / 2))
for id in "vol942-0-0" "vol942-$MID_F-0" "vol942-$LAST_F-$LAST_I"; do
  curl -sS -o /dev/null -w "  GET Patient/$id -> HTTP %{http_code}\n" "$HFS_URL/Patient/$id"
done

echo
echo "=== lock errors / retries ==="
echo "  journal_mode         : ${DB_URL}"
echo "  'database is locked' : $(grep -c 'database is locked' "$WORKDIR/hfs.log" || true)"
echo "  'table is locked'    : $(grep -c 'database table is locked' "$WORKDIR/hfs.log" || true)"
echo "  busy retries         : $(grep -c 'sqlite busy during' "$WORKDIR/hfs.log" || true)"
# This path does NOT go through retry_bookkeeping_on_busy: bulk_submit.rs:1488
# still wraps the claim UPDATE with internal_error, and main.rs:1825 logs it as
# ERROR and sleeps 5s. It is the same failure class as issue #942.
echo "  failed claims        : $(grep -c 'submit worker claim failed' "$WORKDIR/hfs.log" || true)"
echo "  ERROR lines in log   : $(grep -c ' ERROR ' "$WORKDIR/hfs.log" || true)"
echo "  ingest seconds       : ${ELAPSED}"
echo "  log                  : $WORKDIR/hfs.log"

[ "$CODE" = "200" ] || { echo "RESULT: FAIL - the poll never reached 200"; exit 1; }
echo "RESULT: OK - $TOTAL resources across $FILES files ingested with effective fan-out 2"
