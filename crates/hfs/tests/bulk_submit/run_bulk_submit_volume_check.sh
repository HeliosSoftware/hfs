#!/usr/bin/env bash
#
# $bulk-submit ingest at realistic volume on SQLite.
#
# Origin: issue #942. The smoke fixture (1 file, 2 Patients) is far too small to
# put the writer under pressure, so it cannot tell whether the forced fan-out of
# 1 and the busy retries hold up.
#
# This generates a manifest with FILES files of PER_FILE Patients each, ingests
# all of it through $bulk-submit and ASSERTS that:
#
#   - the import finishes (poll -> 200)
#   - the log contains no "database is locked"
#   - the log contains no "database table is locked"
#   - the three sampled resources read back with GET /Patient/{id} -> HTTP 200
#
# and reports, without failing on them:
#
#   - the output manifest's declared files, counts and bytes
#   - the busy retries that happened ("sqlite busy during", grep on the log).
#     A non-zero count is the expected good case: it means a writer hit a busy
#     database and the bounded retry absorbed it instead of aborting the import.
#
# All of it on SQLite, where the file fan-out is not supported: whatever
# HFS_BULK_SUBMIT_FILE_CONCURRENCY asks for, the effective value is 1.
#
# Requirements: cargo, curl and python on PATH. On Windows the interpreter is
# invoked as `python`; `python3` resolves to the Microsoft Store stub.
#
#   crates/hfs/tests/bulk_submit/run_bulk_submit_volume_check.sh
#   FILES=24 PER_FILE=1000 TTL=1500 MAX_POLLS=90 \
#     crates/hfs/tests/bulk_submit/run_bulk_submit_volume_check.sh
#   SKIP_BUILD=1 crates/hfs/tests/bulk_submit/run_bulk_submit_volume_check.sh
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

# This probes the port instead of reading `netstat`: its output is not portable
# (on Linux the state is printed as LISTEN, not LISTENING, and often the binary
# is not even installed), and when the pattern does not match every port would
# look free, so the failure would only show up much later, at server startup.
#
# connect() runs FIRST because bind() alone is not a valid liveness test on
# Windows: there SO_REUSEADDR permits binding a port that is actively
# LISTENING, so a bind-only check reports every port free and the script
# happily attaches to somebody else's server.
port_is_free() {
  python - "$1" <<'PY'
import socket, sys
port = int(sys.argv[1])

# Something already accepting connections here?
c = socket.socket()
c.settimeout(0.35)
try:
    c.connect(("127.0.0.1", port))
except OSError:
    pass            # nothing listening
else:
    sys.exit(1)     # taken
finally:
    c.close()

# Also reject a port that is bound but not yet listening. SO_REUSEADDR here
# keeps a socket lingering in TIME_WAIT from counting as taken.
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

# --- build ---------------------------------------------------------------
# On Windows/MSVC the debug hfs binary overflows the main thread stack while
# building the SearchParameter registry, so it is relinked with a 32 MB stack.
# That is only a link flag: it does not change the code.
# SKIP_BUILD=1 avoids recompiling: on Windows the linker cannot replace
# target/debug/hfs.exe while another instance holds it open ("Access is
# denied", os error 5), and killing that instance is not an option.
if [ "${SKIP_BUILD:-0}" = "1" ]; then
  echo "==> SKIP_BUILD=1: reusing the already built binary"
else
  echo "==> building hfs (debug)"
  case "$(uname -s)" in
    MINGW* | MSYS* | CYGWIN*)
      cargo rustc -p helios-hfs --bin hfs -- -C link-arg=/STACK:33554432
      ;;
    *)
      cargo build -p helios-hfs
      ;;
  esac
fi

# An explicit HFS_BIN still wins; otherwise probe both names so the script runs
# unchanged on Linux/macOS and on Windows.
if [ -z "${HFS_BIN:-}" ]; then
  HFS_BIN="target/debug/hfs"
  [ -x "$HFS_BIN" ] || HFS_BIN="target/debug/hfs.exe"
fi

echo "==> provider on $PROVIDER_URL"
( cd "$WORKDIR" && timeout "$TTL" python -u -m http.server "$PROVIDER_PORT" --bind 127.0.0.1 ) \
  > "$WORKDIR/provider.log" 2>&1 &

echo "==> HFS on $HFS_URL (requested fan-out: $FILE_CONCURRENCY, effective on SQLite: 1)"
# DEFER_INDEXING is pinned off, against the `true` default (#946), on purpose:
# this check exists to put the SQLite writer under pressure and count the busy
# retries that absorb it. Deferring indexing removes the search-index and FTS
# writes from the ingest path, which is most of that pressure, so running the
# default here would quietly stop testing what the script was written to test.
HFS_BASE_URL="$HFS_URL" \
HFS_BULK_SUBMIT_ENABLED=true \
HFS_BULK_SUBMIT_FILE_CONCURRENCY="$FILE_CONCURRENCY" \
HFS_BULK_SUBMIT_DEFER_INDEXING=false \
HFS_LOG_LEVEL=info \
  timeout "$TTL" "$HFS_BIN" \
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
READ_FAILURES=0
# `|| true` inside the substitution because a connection failure makes curl
# exit non-zero, which under `set -e` would abort before the assertion runs.
# curl still writes "000" for %{http_code} in that case, so it is counted.
read_back() {
  local id="$1"
  local code
  code=$(curl -sS -o /dev/null -w '%{http_code}' "$HFS_URL/Patient/$id" || true)
  echo "  GET Patient/$id -> HTTP $code"
  [ "$code" = "200" ] || READ_FAILURES=$((READ_FAILURES + 1))
}
read_back "vol942-0-0"
read_back "vol942-$MID_F-0"
read_back "vol942-$LAST_F-$LAST_I"

echo
echo "=== lock errors / retries ==="
# `grep -c` exits 1 on zero matches, so every count is guarded with `|| true`.
#
# The retry's own WARN line embeds the rusqlite text, so it reads
# "sqlite busy during <what>; retrying: ...: database is locked". Those lines
# are the fix WORKING and must not count as failures, hence the `grep -v`:
# what has to be zero is a lock that was *surfaced*, not one that was absorbed.
BUSY_RETRIES=$(grep -c 'sqlite busy during' "$WORKDIR/hfs.log" || true)
LOCKED=$(grep 'database is locked' "$WORKDIR/hfs.log" \
  | grep -vc 'sqlite busy during' || true)
TABLE_LOCKED=$(grep 'database table is locked' "$WORKDIR/hfs.log" \
  | grep -vc 'sqlite busy during' || true)
# A failed claim is the same failure class as issue #942: the worker loop
# reports it as an ERROR and sleeps before trying again, so it never fails the
# poll. Counted here only to keep it visible.
FAILED_CLAIMS=$(grep -c 'submit worker claim failed' "$WORKDIR/hfs.log" || true)
ERROR_LINES=$(grep -c ' ERROR ' "$WORKDIR/hfs.log" || true)

echo "  database file        : ${DB_URL}"
echo "  'database is locked' : $LOCKED"
echo "  'table is locked'    : $TABLE_LOCKED"
echo "  busy retries         : $BUSY_RETRIES (non-zero is fine: the retry absorbed the contention)"
echo "  failed claims        : $FAILED_CLAIMS"
echo "  ERROR lines in log   : $ERROR_LINES"
echo "  ingest seconds       : ${ELAPSED}"
echo "  log                  : $WORKDIR/hfs.log"

echo
FAILURES=0
if [ "$CODE" != "200" ]; then
  echo "  FAIL: the poll never reached 200 (last HTTP ${CODE:-none})"
  FAILURES=$((FAILURES + 1))
fi
if [ "$LOCKED" != "0" ]; then
  echo "  FAIL: $LOCKED 'database is locked' line(s) in the log"
  FAILURES=$((FAILURES + 1))
fi
if [ "$TABLE_LOCKED" != "0" ]; then
  echo "  FAIL: $TABLE_LOCKED 'database table is locked' line(s) in the log"
  FAILURES=$((FAILURES + 1))
fi
if [ "$READ_FAILURES" != "0" ]; then
  echo "  FAIL: $READ_FAILURES of 3 read-backs did not return HTTP 200"
  FAILURES=$((FAILURES + 1))
fi

if [ "$FAILURES" -ne 0 ]; then
  echo "RESULT: FAIL - $FAILURES check(s) failed; see $WORKDIR/hfs.log"
  exit 1
fi
echo "RESULT: OK - $TOTAL resources across $FILES files ingested with effective fan-out 1, no lock errors, $BUSY_RETRIES busy retries"
