#!/usr/bin/env bash
#
# Gap 2 of issue #942: check that the SQLite fan-out restriction is NOT applied
# when the primary backend is PostgreSQL, which is the `_ => configured` arm of
# `effective_file_concurrency` in crates/rest/src/config.rs.
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

# Same real probe as the other scripts: `netstat` is not portable, and bind()
# alone is not a valid liveness test on Windows, where SO_REUSEADDR permits
# binding a port that is actively LISTENING.
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
HFS_PORT="${HFS_PORT:-18795}"
while ! port_is_free "$HFS_PORT"; do HFS_PORT=$((HFS_PORT + 1)); done

HFS_BIN="target/debug/hfs"
[ -x "$HFS_BIN" ] || HFS_BIN="target/debug/hfs.exe"

echo "==> HFS on PostgreSQL at 127.0.0.1:$HFS_PORT (requested fan-out: $FILE_CONCURRENCY)"

# DEFER_INDEXING is pinned off, against the `true` default (#946): the point of
# the check is that PostgreSQL honours the configured fan-out under the full
# write path, index writes included.
HFS_BASE_URL="http://127.0.0.1:$HFS_PORT" \
HFS_STORAGE_BACKEND=postgres \
HFS_DATABASE_URL="postgres://helios:helios@127.0.0.1:$PG_PORT/helios" \
HFS_BULK_SUBMIT_ENABLED=true \
HFS_BULK_SUBMIT_FILE_CONCURRENCY="$FILE_CONCURRENCY" \
HFS_BULK_SUBMIT_DEFER_INDEXING=false \
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
# On SQLite the server emits a WARN carrying the `configured=` and `effective=`
# fields, because the file fan-out is not supported there and is forced to 1.
# Matching those two fields instead of the English sentence keeps this check
# independent of the wording. On PostgreSQL the line must be absent.
RESTRICTION_LINE=$(grep 'configured=' "$LOG" | grep 'effective=' || true)
if [ -n "$RESTRICTION_LINE" ]; then
  echo "RESULT: FAIL - the SQLite fan-out restriction was applied with PostgreSQL"
  echo "  $RESTRICTION_LINE"
  exit 1
fi
if grep -q "file_concurrency=$FILE_CONCURRENCY" "$LOG"; then
  echo "RESULT: OK - not restricted, effective fan-out = $FILE_CONCURRENCY"
else
  echo "RESULT: INCONCLUSIVE - file_concurrency=$FILE_CONCURRENCY was not seen"
  exit 1
fi
