#!/usr/bin/env bash
#
# Does a restart during the deferred reindex leave the database permanently
# unsearchable?
#
# `HFS_BULK_SUBMIT_DEFER_INDEXING=true` ingests without writing search-index or
# FTS rows and rebuilds them afterwards. The rebuild is started only after the
# manifest is already terminal, and it is started with `tokio::spawn`
# (bulk_submit_worker.rs:902 -> reindex.rs:554). Reading the code says the job
# is held in an in-memory map (reindex.rs:440), that no column on
# `bulk_manifests` records "indexes still pending" (schema.rs:736-760), and
# that nothing re-fires it at startup. This script checks that claim against
# the running server instead of trusting the reading.
#
# The sequence is:
#
#   1. ingest with defer_indexing=true
#   2. poll $bulk-submit-status until it answers 200, i.e. until the API tells
#      a client the submission is complete
#   3. stop the server at that moment — the restart a deploy, a crash or an OOM
#      would cause
#   4. start it again on the SAME database, with the SAME setting
#   5. wait, then ask how many of the ingested resources search can find
#
# Step 5 is the whole point. The resources are stored either way — the script
# reports the read-by-id result too — so a shortfall here is not lost data, it
# is data that exists and is invisible to every search until somebody notices
# and runs $reindex by hand.
#
# PASS means the index recovered on its own. FAIL means it did not.
#
# Requirements: a release `hfs` (or set HFS_BIN), curl and python. Run from the
# repo root so `data/search-parameters-r4.json` is found.
#
#   crates/hfs/tests/bulk_submit/run_defer_indexing_crash_check.sh
#   TOTAL=20000 crates/hfs/tests/bulk_submit/run_defer_indexing_crash_check.sh
#
set -euo pipefail

cd "$(dirname "$0")/../../../.."

FILES="${FILES:-2}"
PER_FILE="${PER_FILE:-2500}"
TOTAL=$((FILES * PER_FILE))
WORKDIR="${WORKDIR:-/tmp/hfs-defer-crash}"
SETTLE="${SETTLE:-90}"     # seconds to let a recovery happen after restart
DB="target/hfs-defer-crash.db"

# connect() first: on Windows SO_REUSEADDR lets a bind succeed on a port that
# is actively LISTENING, so a bind-only check would report every port free.
port_is_free() {
  python - "$1" <<'PY'
import socket, sys
port = int(sys.argv[1])
c = socket.socket(); c.settimeout(0.35)
try:
    c.connect(("127.0.0.1", port))
except OSError:
    pass
else:
    sys.exit(1)
finally:
    c.close()
s = socket.socket()
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
try:
    s.bind(("127.0.0.1", port))
except OSError:
    sys.exit(1)
finally:
    s.close()
PY
}
pick_port() { local p="$1"; while ! port_is_free "$p"; do p=$((p + 1)); done; echo "$p"; }

rm -rf "$WORKDIR"; mkdir -p "$WORKDIR"
rm -f "$DB" "$DB-wal" "$DB-shm"

# Only PIDs started here are recorded, and only these are ever stopped.
SERVER_PIDS=()
stop_own() {
  local pid
  for pid in "${SERVER_PIDS[@]:-}"; do
    [ -n "$pid" ] && kill "$pid" 2>/dev/null || true
  done
  SERVER_PIDS=()
}
trap stop_own EXIT

PROVIDER_PORT="$(pick_port "${PROVIDER_PORT:-19400}")"
PROVIDER_URL="http://127.0.0.1:$PROVIDER_PORT"
echo "==> fixture: $FILES x $PER_FILE = $TOTAL Patients"
python - "$WORKDIR" "$FILES" "$PER_FILE" "$PROVIDER_URL" <<'PY'
import json, sys, pathlib
workdir, files, per_file, provider = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), sys.argv[4]
out = []
for f in range(files):
    name = f"patients-{f}.ndjson"
    with open(pathlib.Path(workdir) / name, "w", encoding="utf-8") as fh:
        for i in range(per_file):
            fh.write(json.dumps({
                "resourceType": "Patient", "id": f"crash-{f}-{i}",
                "name": [{"family": "Crashcheck", "given": [f"F{f}", f"I{i}"]}],
                "gender": "female" if i % 2 == 0 else "male",
                "birthDate": "1980-01-01",
            }) + "\n")
    out.append({"type": "Patient", "url": f"{provider}/{name}", "count": per_file})
(pathlib.Path(workdir) / "manifest.json").write_text(json.dumps({
    "transactionTime": "2024-01-01T00:00:00Z", "request": f"{provider}/manifest.json",
    "requiresAccessToken": False, "output": out, "error": [], "deleted": [],
}, indent=2), encoding="utf-8")
PY

if [ -z "${HFS_BIN:-}" ]; then
  HFS_BIN="target/release/hfs"; [ -x "$HFS_BIN" ] || HFS_BIN="target/release/hfs.exe"
fi
[ -x "$HFS_BIN" ] || { echo "no release hfs; build it or set HFS_BIN" >&2; exit 1; }

( cd "$WORKDIR" && exec python -u -m http.server "$PROVIDER_PORT" --bind 127.0.0.1 ) \
  > "$WORKDIR/provider.log" 2>&1 &
SERVER_PIDS+=("$!")
for _ in $(seq 1 30); do curl -sS -o /dev/null "$PROVIDER_URL/manifest.json" 2>/dev/null && break; sleep 1; done

PORT="$(pick_port 18950)"
URL="http://127.0.0.1:$PORT"

# Sets the global SERVER_PID. Deliberately NOT `PID=$(start_server ...)`:
# command substitution runs the function in a subshell, so the append to
# SERVER_PIDS would be lost and the EXIT trap would leave a server holding the
# database open.
SERVER_PID=""
start_server() {
  HFS_BASE_URL="$URL" \
  HFS_BULK_SUBMIT_ENABLED=true \
  HFS_BULK_SUBMIT_DEFER_INDEXING=true \
  HFS_BULK_SUBMIT_POLL_RATE_LIMIT=100000 \
    "$HFS_BIN" --database-url "$DB" --log-level info \
      --host 127.0.0.1 --port "$PORT" >> "$1" 2>&1 &
  SERVER_PID=$!
  SERVER_PIDS+=("$SERVER_PID")
  for _ in $(seq 1 90); do curl -sS -o /dev/null "$URL/health" 2>/dev/null && return 0; sleep 1; done
  echo "server did not start, see $1" >&2; return 1
}

searchable() {
  curl -sS "$URL/Patient?family=Crashcheck&_summary=count" 2>/dev/null \
    | python -c 'import json,sys
try: print(json.load(sys.stdin).get("total", 0) or 0)
except Exception: print(0)' 2>/dev/null || echo 0
}

echo "==> run 1: ingest with defer_indexing=true on $URL"
start_server "$WORKDIR/run1.log"; PID1=$SERVER_PID

cat > "$WORKDIR/submit.json" <<EOF
{ "resourceType": "Parameters", "parameter": [
  { "name": "submitter", "valueIdentifier": { "system": "http://example.org", "value": "crash" } },
  { "name": "submissionId", "valueString": "crash-1" },
  { "name": "manifestUrl", "valueUrl": "$PROVIDER_URL/manifest.json" },
  { "name": "fhirBaseUrl", "valueUrl": "$PROVIDER_URL/fhir" },
  { "name": "submissionStatus", "valueCoding": { "system": "http://hl7.org/fhir/event-status", "code": "completed" } } ] }
EOF
cat > "$WORKDIR/status.json" <<EOF
{ "resourceType": "Parameters", "parameter": [
  { "name": "submitter", "valueIdentifier": { "system": "http://example.org", "value": "crash" } },
  { "name": "submissionId", "valueString": "crash-1" } ] }
EOF

curl -sS -o /dev/null -X POST "$URL/\$bulk-submit" \
  -H 'Content-Type: application/fhir+json' --data-binary @"$WORKDIR/submit.json"
LOC=$(curl -sS -D - -o /dev/null -X POST "$URL/\$bulk-submit-status" \
  -H 'Content-Type: application/fhir+json' --data-binary @"$WORKDIR/status.json" \
  | tr -d '\r' | awk -F': ' 'tolower($1)=="content-location"{print $2}')

echo "==> polling until the API reports the submission complete (200)"
for _ in $(seq 1 1200); do
  [ "$(curl -sS -o /dev/null -w '%{http_code}' "$LOC" 2>/dev/null || echo 000)" = "200" ] && break
  sleep 1
done
AT_200=$(searchable)
echo "    status=200. searchable at that instant: $AT_200/$TOTAL"

# The restart, at the exact moment a client is told the work is done.
kill "$PID1" 2>/dev/null || true
wait "$PID1" 2>/dev/null || true
sleep 3
echo "==> server stopped immediately after the 200, and restarted on the same db"

start_server "$WORKDIR/run2.log"
echo "==> waiting ${SETTLE}s for any self-recovery"
BEST=0
for _ in $(seq 1 "$SETTLE"); do
  N=$(searchable); [ "${N:-0}" -gt "$BEST" ] && BEST=$N
  [ "${BEST:-0}" -ge "$TOTAL" ] && break
  sleep 1
done

READ_OK=$(curl -sS -o /dev/null -w '%{http_code}' "$URL/Patient/crash-0-0" 2>/dev/null || echo 000)

echo
echo "=== result ==="
echo "  searchable when the API said complete : $AT_200/$TOTAL"
echo "  searchable after restart + ${SETTLE}s       : $BEST/$TOTAL"
echo "  GET Patient/crash-0-0 after restart   : HTTP $READ_OK"
echo
if [ "${BEST:-0}" -ge "$TOTAL" ]; then
  echo "  PASS - the index recovered on its own."
else
  echo "  FAIL - $((TOTAL - BEST)) of $TOTAL resources are stored but unsearchable,"
  echo "         and stayed that way. The API had already reported the submission"
  echo "         complete, nothing records that indexing is outstanding, and no"
  echo "         reindex is re-fired at startup: only a manual \$reindex fixes it."
fi
echo
echo "logs: $WORKDIR"
