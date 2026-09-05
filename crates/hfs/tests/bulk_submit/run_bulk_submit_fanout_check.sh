#!/usr/bin/env bash
#
# Manual check of the $bulk-submit file fan-out on SQLite: that the clamp to
# SQLITE_MAX_FILE_CONCURRENCY is applied and announced in the log, and that a
# full ingest finishes cleanly.
#
# Origin: issue #942, "import aborts with 'database is locked' at high file
# concurrency with the full search-parameter registry".
#
# It starts two things and then stays up:
#
#   1. A static "Data Provider" (python -m http.server) serving a Bulk Export
#      Manifest and one .ndjson with 2 Patients.
#   2. HFS with HFS_BULK_SUBMIT_FILE_CONCURRENCY=8, which is the value that
#      triggers the clamp introduced by the fix.
#
# It kills no processes and frees no ports: it picks free ports >18000 and runs
# both servers under `timeout`, so they shut themselves down when the TTL runs
# out.
#
# Requirements: cargo, curl and python on PATH. On Windows the interpreter must
# be invoked as `python`: `python3` resolves to the Microsoft Store stub, which
# prints a notice and exits without starting anything.
#
#   crates/hfs/tests/bulk_submit/run_bulk_submit_fanout_check.sh
#   TTL=1800 crates/hfs/tests/bulk_submit/run_bulk_submit_fanout_check.sh
#   FILE_CONCURRENCY=1 crates/hfs/tests/bulk_submit/run_bulk_submit_fanout_check.sh
#   SKIP_BUILD=1 crates/hfs/tests/bulk_submit/run_bulk_submit_fanout_check.sh
#
set -euo pipefail

cd "$(dirname "$0")/../../../.."

TTL="${TTL:-600}"
FILE_CONCURRENCY="${FILE_CONCURRENCY:-8}"
WORKDIR="${WORKDIR:-/tmp/hfs-bulk-submit-fanout}"
# A file, not `:memory:`. An in-memory database cannot turn WAL on
# (backend.rs:557 tries, but it stays on journal_mode=memory), and without WAL
# the locks surface as table-level SQLITE_LOCKED: a degraded case that does not
# represent the real deployment the issue fixes.
DB_URL="${DB_URL:-target/hfs-bulk-submit-fanout.db}"
rm -f "$DB_URL" "$DB_URL-wal" "$DB_URL-shm"

# --- free ports >18000 ---------------------------------------------------
# This does a real bind instead of reading `netstat`: its output is not portable
# (on Linux the state is printed as LISTEN, not LISTENING, and often the binary
# is not even installed), and when the pattern does not match every port would
# look free, so the failure would only show up much later, at server startup.
# SO_REUSEADDR mirrors what both the HFS listener and http.server do, so a
# socket in TIME_WAIT does not count as taken.
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

PROVIDER_PORT="$(pick_port "${PROVIDER_PORT:-19100}")"
HFS_PORT="$(pick_port "${HFS_PORT:-18790}")"
[ "$HFS_PORT" = "$PROVIDER_PORT" ] && HFS_PORT="$(pick_port $((HFS_PORT + 1)))"

PROVIDER_URL="http://127.0.0.1:$PROVIDER_PORT"
HFS_URL="http://127.0.0.1:$HFS_PORT"

# --- provider fixture ----------------------------------------------------
rm -rf "$WORKDIR"
mkdir -p "$WORKDIR"

cat > "$WORKDIR/patients.ndjson" <<'EOF'
{"resourceType":"Patient","id":"submit-smoke-1","name":[{"family":"Submit","given":["Alpha"]}],"gender":"female"}
{"resourceType":"Patient","id":"submit-smoke-2","name":[{"family":"Submit","given":["Beta"]}],"gender":"male"}
EOF

cat > "$WORKDIR/manifest.json" <<EOF
{
  "transactionTime": "2024-01-01T00:00:00Z",
  "request": "$PROVIDER_URL/manifest.json",
  "requiresAccessToken": false,
  "output": [
    { "type": "Patient", "url": "$PROVIDER_URL/patients.ndjson", "count": 2 }
  ],
  "error": [],
  "deleted": []
}
EOF

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

HFS_BIN="target/debug/hfs"
[ -x "$HFS_BIN" ] || HFS_BIN="target/debug/hfs.exe"

# --- startup -------------------------------------------------------------
# `python -u` because when stdout is redirected the banner stays in the buffer.
echo "==> provider on $PROVIDER_URL (TTL ${TTL}s)"
( cd "$WORKDIR" && timeout "$TTL" python -u -m http.server "$PROVIDER_PORT" --bind 127.0.0.1 ) \
  > "$WORKDIR/provider.log" 2>&1 &

echo "==> HFS on $HFS_URL (HFS_BULK_SUBMIT_FILE_CONCURRENCY=$FILE_CONCURRENCY, TTL ${TTL}s)"
# HFS_BASE_URL is mandatory: it defaults to http://localhost:8080 and it is the
# origin HFS uses to build the `content-location` polling header. Without it the
# poll goes to whatever runs on 8080, not to this instance.
HFS_BASE_URL="$HFS_URL" \
HFS_BULK_SUBMIT_ENABLED=true \
HFS_BULK_SUBMIT_FILE_CONCURRENCY="$FILE_CONCURRENCY" \
HFS_LOG_LEVEL=info \
  timeout "$TTL" "$HFS_BIN" \
    --database-url "$DB_URL" --log-level info \
    --host 127.0.0.1 --port "$HFS_PORT" \
  > "$WORKDIR/hfs.log" 2>&1 &

# --- wait for HFS to answer ----------------------------------------------
for _ in $(seq 1 60); do
  if curl -sS -o /dev/null "$HFS_URL/health" 2>/dev/null; then break; fi
  sleep 1
done

if ! curl -sS -o /dev/null -w '' "$HFS_URL/health" 2>/dev/null; then
  echo "HFS did not answer on $HFS_URL/health" >&2
  tail -40 "$WORKDIR/hfs.log" >&2
  exit 1
fi

cat <<EOF

ready.

  HFS_URL      = $HFS_URL
  PROVIDER_URL = $PROVIDER_URL
  logs         = $WORKDIR/hfs.log , $WORKDIR/provider.log

Both shut themselves down in ${TTL}s. Fan-out lines from startup:

$(grep 'Bulk submit' "$WORKDIR/hfs.log" || true)
EOF
