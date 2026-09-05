#!/usr/bin/env bash
#
# Gap 2 of issue #942: check that the clamp is NOT applied when the primary
# backend is PostgreSQL, which is the `_ => configured` arm of
# `effective_file_concurrency` (crates/rest/src/config.rs:737).
#
# Requires a PostgreSQL listening on PG_PORT. Start one with:
#   docker run -d --name hfs-bulk-submit-pg \
#     -e POSTGRES_USER=helios -e POSTGRES_PASSWORD=helios -e POSTGRES_DB=helios \
#     -p 127.0.0.1:18432:5432 postgres:16-alpine
#
# The binary must be built with the `postgres` feature, which is NOT in the
# helios-hfs defaults (crates/hfs/Cargo.toml:17):
#   cargo build -p helios-hfs --features helios-hfs/postgres
# On Windows the debug binary also overflows the main thread stack while
# building the SearchParameter registry, so it has to be relinked:
#   cargo rustc -p helios-hfs --bin hfs --features helios-hfs/postgres \
#     -- -C link-arg=/STACK:33554432
#
# Requirements: docker (or your own PostgreSQL), cargo and python on PATH.
#
set -euo pipefail

cd "$(dirname "$0")/../../../.."

PG_PORT="${PG_PORT:-18432}"
TTL="${TTL:-75}"
FILE_CONCURRENCY="${FILE_CONCURRENCY:-8}"
LOG="${LOG:-/tmp/hfs-bulk-submit-postgres.log}"

# Same real bind as the other scripts: `netstat` is not portable.
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
HFS_PORT="${HFS_PORT:-18795}"
while ! port_is_free "$HFS_PORT"; do HFS_PORT=$((HFS_PORT + 1)); done

HFS_BIN="target/debug/hfs"
[ -x "$HFS_BIN" ] || HFS_BIN="target/debug/hfs.exe"

echo "==> HFS on PostgreSQL at 127.0.0.1:$HFS_PORT (requested fan-out: $FILE_CONCURRENCY)"

HFS_BASE_URL="http://127.0.0.1:$HFS_PORT" \
HFS_STORAGE_BACKEND=postgres \
HFS_DATABASE_URL="postgres://helios:helios@127.0.0.1:$PG_PORT/helios" \
HFS_BULK_SUBMIT_ENABLED=true \
HFS_BULK_SUBMIT_FILE_CONCURRENCY="$FILE_CONCURRENCY" \
HFS_LOG_LEVEL=info \
  timeout "$TTL" "$HFS_BIN" --log-level info --host 127.0.0.1 --port "$HFS_PORT" \
  > "$LOG" 2>&1 || true

echo
echo "=== selected backend ==="
grep -o 'storage_backend=[a-z]*' "$LOG" | head -1 || echo "(not found)"

echo
echo "=== bulk submit lines ==="
grep 'Bulk submit' "$LOG" || echo "(none)"

echo
if grep -q 'fan-out clamped' "$LOG"; then
  echo "RESULT: FAIL - the clamp was applied with PostgreSQL"
  exit 1
fi
if grep -q "file_concurrency=$FILE_CONCURRENCY" "$LOG"; then
  echo "RESULT: OK - no clamp, effective fan-out = $FILE_CONCURRENCY"
else
  echo "RESULT: INCONCLUSIVE - file_concurrency=$FILE_CONCURRENCY was not seen"
  exit 1
fi
