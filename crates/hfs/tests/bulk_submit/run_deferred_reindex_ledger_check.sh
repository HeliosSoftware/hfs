#!/usr/bin/env bash
#
# Does a deferred search-index rebuild that covers several manifests clear the
# `index_pending` marker of every one of them? (#1213)
#
# With `HFS_BULK_SUBMIT_DEFER_INDEXING=true`, each manifest records in
# `bulk_manifests.index_pending` that it still owes a rebuild (#1125). The
# reindex coordinator merges the resource types of every manifest that
# finishes while a generation is pending into one generation. Before #1213 it
# kept only the last manifest's id, so the generation cleared one marker and
# left the rest set. Every restart then found them, ran another full rebuild
# of data that was already indexed, and cleared one more.
#
# The script checks both ways into a merged generation, against a running
# server and the real SQLite ledger:
#
#   phase 1, live path
#     three manifests go into ONE submission back to back. When every family
#     is searchable, no manifest may still have index_pending = 1. If the log
#     shows the manifests never merged into a pending generation, the phase
#     is INCONCLUSIVE rather than PASS: the merge is what is under test.
#
#   phase 2, restart path (the startup resume loop enqueues every owed
#   manifest back to back, so they merge by construction)
#     two more manifests are ingested and the server is SIGKILLed the instant
#     $bulk-submit-status answers 200, before their rebuild can finish. If the
#     rebuild beat the kill, the markers are set again by hand, which is the
#     exact state a crash leaves behind. The server restarts on the same
#     database, must log the resume line for those manifests, and must end
#     with no marker set. A third start must log no resume line at all.
#
# PASS means every marker was cleared and a restart owes nothing. FAIL prints
# the rows still marked.
#
# The fixture is synthetic Patients generated here, a few thousand of them:
# enough for each rebuild to take a noticeable moment, far short of a corpus.
#
# Requirements: cargo (or HFS_BIN), curl and python3. The script changes to the
# repo root so `data/search-parameters-r4.json` is found. It starts and stops
# only its own processes.
#
#   crates/hfs/tests/bulk_submit/run_deferred_reindex_ledger_check.sh
#   SKIP_BUILD=1 PER_MANIFEST=500 crates/hfs/tests/bulk_submit/run_deferred_reindex_ledger_check.sh
#   HFS_BIN=target/release/hfs crates/hfs/tests/bulk_submit/run_deferred_reindex_ledger_check.sh
#
# Knobs: PER_MANIFEST (2000), PHASE1_MANIFESTS (3), PHASE2_MANIFESTS (2),
# WORKDIR (/tmp/hfs-1213-ledger), TIMEOUT seconds per wait (600),
# PROVIDER_PORT (19500), HFS_PORT (18960), SKIP_BUILD, HFS_BIN.
#
set -euo pipefail

cd "$(dirname "$0")/../../../.."

PER_MANIFEST="${PER_MANIFEST:-2000}"
PHASE1_MANIFESTS="${PHASE1_MANIFESTS:-3}"
PHASE2_MANIFESTS="${PHASE2_MANIFESTS:-2}"
WORKDIR="${WORKDIR:-/tmp/hfs-1213-ledger}"
TIMEOUT="${TIMEOUT:-600}"
DB="$WORKDIR/qa.db"
PY="$(command -v python3 || command -v python)"

# connect() first: on Windows SO_REUSEADDR lets a bind succeed on a port that
# is actively LISTENING, so a bind-only check would report every port free.
port_is_free() {
  "$PY" - "$1" <<'PY'
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

# Only PIDs started here are recorded, and only these are ever stopped.
OWN_PIDS=()
stop_own() {
  local pid
  for pid in "${OWN_PIDS[@]:-}"; do
    [ -n "$pid" ] && kill "$pid" 2>/dev/null || true
  done
  for pid in "${OWN_PIDS[@]:-}"; do
    [ -n "$pid" ] && wait "$pid" 2>/dev/null || true
  done
  OWN_PIDS=()
}
trap stop_own EXIT

# --- build ---------------------------------------------------------------
if [ -z "${HFS_BIN:-}" ]; then
  if [ "${SKIP_BUILD:-0}" = "1" ]; then
    echo "==> SKIP_BUILD=1: reusing the already built binary"
  else
    echo "==> building hfs (debug)"
    cargo build -p helios-hfs --bin hfs
  fi
  HFS_BIN="target/debug/hfs"; [ -x "$HFS_BIN" ] || HFS_BIN="target/debug/hfs.exe"
fi
[ -x "$HFS_BIN" ] || { echo "no hfs binary at $HFS_BIN; build it or set HFS_BIN" >&2; exit 2; }

rm -rf "$WORKDIR"; mkdir -p "$WORKDIR"

# --- fixture -------------------------------------------------------------
# One manifest per family, one NDJSON file each. Ids and families never
# collide across manifests, so each family count proves its own manifest's
# rebuild ran.
PROVIDER_PORT="$(pick_port "${PROVIDER_PORT:-19500}")"
PROVIDER_URL="http://127.0.0.1:$PROVIDER_PORT"
TOTAL=$(( (PHASE1_MANIFESTS + PHASE2_MANIFESTS) * PER_MANIFEST ))
echo "==> fixture: $((PHASE1_MANIFESTS + PHASE2_MANIFESTS)) manifests x $PER_MANIFEST Patients = $TOTAL"
"$PY" - "$WORKDIR" "$PER_MANIFEST" "$PROVIDER_URL" "$PHASE1_MANIFESTS" "$PHASE2_MANIFESTS" <<'PY'
import json, sys, pathlib
workdir, per, provider = pathlib.Path(sys.argv[1]), int(sys.argv[2]), sys.argv[3]
phases = {1: int(sys.argv[4]), 2: int(sys.argv[5])}
for phase, count in phases.items():
    for m in range(count):
        tag = f"p{phase}{chr(ord('a') + m)}"
        family = f"Ledger{tag.upper()}"
        name = f"patients-{tag}.ndjson"
        with open(workdir / name, "w", encoding="utf-8") as fh:
            for i in range(per):
                fh.write(json.dumps({
                    "resourceType": "Patient",
                    "id": f"ledger-{tag}-{i}",
                    "identifier": [{"system": "http://example.org/mrn", "value": f"MRN-{tag}-{i}"}],
                    "name": [{"family": family, "given": [tag, f"I{i}"]}],
                    "telecom": [{"system": "phone", "value": f"555-{m:03d}-{i:04d}"}],
                    "gender": "female" if i % 2 == 0 else "male",
                    "birthDate": "1980-01-01",
                    "address": [{"city": "Springfield", "state": "IL", "postalCode": "62701"}],
                }) + "\n")
        (workdir / f"manifest-{tag}.json").write_text(json.dumps({
            "transactionTime": "2024-01-01T00:00:00Z",
            "request": f"{provider}/manifest-{tag}.json",
            "requiresAccessToken": False,
            "output": [{"type": "Patient", "url": f"{provider}/{name}", "count": per}],
            "error": [], "deleted": [],
        }, indent=2), encoding="utf-8")
PY

( cd "$WORKDIR" && exec "$PY" -u -m http.server "$PROVIDER_PORT" --bind 127.0.0.1 ) \
  > "$WORKDIR/provider.log" 2>&1 &
OWN_PIDS+=("$!")
for _ in $(seq 1 30); do
  curl -sS -o /dev/null "$PROVIDER_URL/manifest-p1a.json" 2>/dev/null && break; sleep 0.5
done

PORT="$(pick_port "${HFS_PORT:-18960}")"
URL="http://127.0.0.1:$PORT"
echo "==> provider on $PROVIDER_URL, hfs on $URL, db $DB"

# Sets the global SERVER_PID. Deliberately NOT `PID=$(start_server ...)`:
# command substitution runs the function in a subshell, so the append to
# OWN_PIDS would be lost and the EXIT trap would leave a server holding the
# database open.
SERVER_PID=""
start_server() {
  HFS_BASE_URL="$URL" \
  HFS_AUTH_ENABLED=false \
  HFS_BULK_SUBMIT_ENABLED=true \
  HFS_BULK_SUBMIT_DEFER_INDEXING=true \
  HFS_BULK_SUBMIT_POLL_RATE_LIMIT=100000 \
  HFS_BULK_SUBMIT_RETRY_AFTER=1 \
    "$HFS_BIN" --database-url "$DB" --log-level info \
      --host 127.0.0.1 --port "$PORT" >> "$1" 2>&1 &
  SERVER_PID=$!
  OWN_PIDS+=("$SERVER_PID")
  for _ in $(seq 1 180); do
    curl -sS -o /dev/null "$URL/health" 2>/dev/null && return 0
    kill -0 "$SERVER_PID" 2>/dev/null || break
    sleep 0.5
  done
  echo "server did not start, see $1" >&2; return 1
}

stop_server() { # $1 = signal
  kill "-$1" "$SERVER_PID" 2>/dev/null || true
  wait "$SERVER_PID" 2>/dev/null || true
}

searchable() { # $1 = family
  curl -sS "$URL/Patient?family=$1&_summary=count" 2>/dev/null \
    | "$PY" -c 'import json,sys
try: print(json.load(sys.stdin).get("total", 0) or 0)
except Exception: print(0)' 2>/dev/null || echo 0
}

# Prints "manifest_id|submission_id|index_pending" per row, oldest first.
# $1 = optional SQL filter on submission_id.
ledger_rows() {
  "$PY" - "$DB" "${1:-}" <<'PY'
import sqlite3, sys
db, sub = sys.argv[1], sys.argv[2]
con = sqlite3.connect(db, timeout=30)
q = "SELECT manifest_id, submission_id, index_pending FROM bulk_manifests"
args = ()
if sub:
    q += " WHERE submission_id = ?"; args = (sub,)
for row in con.execute(q + " ORDER BY added_at", args):
    print("|".join(str(c) for c in row))
PY
}
pending_count() { ledger_rows "${1:-}" | awk -F'|' '$3 == 1' | wc -l | tr -d ' '; }

submit() { # $1 submission, $2 manifest tag, $3 "last" to complete the submission
  local status_part=""
  if [ "${3:-}" = "last" ]; then
    status_part=', { "name": "submissionStatus", "valueCoding": { "system": "http://hl7.org/fhir/event-status", "code": "completed" } }'
  fi
  local code
  code=$(curl -sS -o "$WORKDIR/kickoff-$2.json" -w '%{http_code}' -X POST "$URL/\$bulk-submit" \
    -H 'Content-Type: application/fhir+json' --data-binary @- <<EOF
{ "resourceType": "Parameters", "parameter": [
  { "name": "submitter", "valueIdentifier": { "system": "http://example.org", "value": "ledger" } },
  { "name": "submissionId", "valueString": "$1" },
  { "name": "manifestUrl", "valueUrl": "$PROVIDER_URL/manifest-$2.json" },
  { "name": "fhirBaseUrl", "valueUrl": "$PROVIDER_URL/fhir" }$status_part ] }
EOF
  )
  [ "$code" = "200" ] || { echo "kick-off of manifest-$2 answered $code:" >&2; cat "$WORKDIR/kickoff-$2.json" >&2; exit 2; }
}

await_submission() { # $1 submission
  local loc
  loc=$(curl -sS -D - -o /dev/null -X POST "$URL/\$bulk-submit-status" \
    -H 'Content-Type: application/fhir+json' --data-binary @- <<EOF | tr -d '\r' | awk -F': ' 'tolower($1)=="content-location"{print $2}'
{ "resourceType": "Parameters", "parameter": [
  { "name": "submitter", "valueIdentifier": { "system": "http://example.org", "value": "ledger" } },
  { "name": "submissionId", "valueString": "$1" } ] }
EOF
  )
  [ -n "$loc" ] || { echo "no Content-Location for the status of $1" >&2; exit 2; }
  local deadline=$((SECONDS + TIMEOUT))
  while [ $SECONDS -lt $deadline ]; do
    [ "$(curl -sS -o /dev/null -w '%{http_code}' "$loc" 2>/dev/null || echo 000)" = "200" ] && return 0
    sleep 0.2
  done
  echo "submission $1 never reached status 200" >&2; exit 2
}

await_searchable() { # families...
  local deadline=$((SECONDS + TIMEOUT)) family n done_all
  while [ $SECONDS -lt $deadline ]; do
    done_all=1
    for family in "$@"; do
      n=$(searchable "$family")
      [ "${n:-0}" -ge "$PER_MANIFEST" ] || { done_all=0; break; }
    done
    [ "$done_all" = 1 ] && return 0
    sleep 1
  done
  return 1
}

# The marker is cleared right after the generation's last page is written, so
# give it a short moment after search is complete.
await_ledger_drained() { # $1 = optional submission filter
  local deadline=$((SECONDS + 60))
  while [ $SECONDS -lt $deadline ]; do
    [ "$(pending_count "${1:-}")" = "0" ] && return 0
    sleep 1
  done
  return 1
}

tags() { # $1 phase, $2 count
  local letters=abcdefghijklmnopqrstuvwxyz i
  for ((i = 0; i < $2; i++)); do printf 'p%s%s ' "$1" "${letters:i:1}"; done
}
families() { local t; for t in "$@"; do printf 'Ledger%s ' "$(echo "$t" | tr '[:lower:]' '[:upper:]')"; done; }

RESULT=PASS
P2_VERDICT=PASS
fail() { echo "  FAIL - $*"; RESULT=FAIL; }
fail2() { fail "$@"; P2_VERDICT=FAIL; }

# One line per generation: its number and every manifest it covered.
show_generations() { # $1 = log
  sed -n '/deferred reindex generation started/ s/.*\(generation=[0-9]*\).*\(manifests={[^}]*}\).*/\1 \2/p' "$1" \
    | sed 's/^/      /'
}

# --- phase 1: live path --------------------------------------------------
P1_TAGS=($(tags 1 "$PHASE1_MANIFESTS"))
P1_FAMILIES=($(families "${P1_TAGS[@]}"))
echo
echo "==> phase 1: ${#P1_TAGS[@]} manifests into submission ledger-1, back to back"
start_server "$WORKDIR/run1.log"
for ((i = 0; i < ${#P1_TAGS[@]}; i++)); do
  if [ $i -eq $((${#P1_TAGS[@]} - 1)) ]; then submit ledger-1 "${P1_TAGS[$i]}" last
  else submit ledger-1 "${P1_TAGS[$i]}"; fi
done
await_submission ledger-1
echo "    status=200; waiting until ${P1_FAMILIES[*]} are all searchable"
await_searchable "${P1_FAMILIES[@]}" || fail "phase 1 families never became fully searchable"
MERGES_P1=$(grep -c 'merged deferred reindex work into the pending generation' "$WORKDIR/run1.log" || true)
GENS_P1=$(grep -c 'deferred reindex generation started' "$WORKDIR/run1.log" || true)
P1_VERDICT=PASS
if await_ledger_drained ledger-1; then
  if [ "${MERGES_P1:-0}" -eq 0 ]; then
    P1_VERDICT=INCONCLUSIVE
    echo "  INCONCLUSIVE - every marker cleared, but no manifest merged into a pending"
    echo "                 generation this run, so the live merge was not exercised."
    echo "                 On SQLite a manifest's publish usually waits for the writer"
    echo "                 until the running rebuild ends. Phase 2 always merges."
  fi
else
  P1_VERDICT=FAIL
  fail "phase 1 left manifests marked as owing a rebuild:"
  ledger_rows ledger-1 | awk -F'|' '$3 == 1 {print "         " $0}'
fi
echo "    generations started: $GENS_P1, merges logged: $MERGES_P1, pending rows: $(pending_count ledger-1)"
show_generations "$WORKDIR/run1.log"

# --- phase 2: restart path -----------------------------------------------
P2_TAGS=($(tags 2 "$PHASE2_MANIFESTS"))
P2_FAMILIES=($(families "${P2_TAGS[@]}"))
echo
echo "==> phase 2: ${#P2_TAGS[@]} manifests into submission ledger-2, SIGKILL at status 200"
for ((i = 0; i < ${#P2_TAGS[@]}; i++)); do
  if [ $i -eq $((${#P2_TAGS[@]} - 1)) ]; then submit ledger-2 "${P2_TAGS[$i]}" last
  else submit ledger-2 "${P2_TAGS[$i]}"; fi
done
await_submission ledger-2
stop_server KILL
P2_PENDING=$(pending_count ledger-2)
FALLBACK=no
if [ "$P2_PENDING" -lt "${#P2_TAGS[@]}" ]; then
  # The rebuild beat the kill. Put back the state a crash mid-rebuild leaves.
  FALLBACK=yes
  "$PY" - "$DB" <<'PY'
import sqlite3, sys
con = sqlite3.connect(sys.argv[1], timeout=30)
con.execute("UPDATE bulk_manifests SET index_pending = 1 WHERE submission_id = 'ledger-2'")
con.commit()
PY
  P2_PENDING=$(pending_count ledger-2)
fi
echo "    killed; ledger-2 rows pending: $P2_PENDING (markers re-set by hand: $FALLBACK)"
TOTAL_PENDING=$(pending_count)
[ "$TOTAL_PENDING" = "$P2_PENDING" ] \
  || fail2 "expected only ledger-2 to be pending before the restart, found $TOTAL_PENDING rows"

echo "==> restart 1 on the same db: the resume loop must pick up $P2_PENDING manifests"
start_server "$WORKDIR/run2.log"
RESUME_LINE=""
for _ in $(seq 1 60); do
  RESUME_LINE=$(grep 'resuming search-index rebuilds left outstanding by an earlier run' "$WORKDIR/run2.log" || true)
  [ -n "$RESUME_LINE" ] && break
  sleep 0.5
done
RESUMED=$(echo "$RESUME_LINE" | grep -o 'manifests=[0-9]*' | head -1 | cut -d= -f2)
if [ "${RESUMED:-}" = "$P2_PENDING" ]; then
  echo "    resume line logged with manifests=$RESUMED"
else
  fail2 "restart 1 should log the resume line with manifests=$P2_PENDING, got '${RESUMED:-none}'"
fi
await_searchable "${P2_FAMILIES[@]}" || fail2 "phase 2 families never became fully searchable"
if await_ledger_drained; then
  echo "    every marker cleared after the resumed rebuild"
else
  fail2 "the resumed rebuild left manifests marked as owing a rebuild:"
  ledger_rows | awk -F'|' '$3 == 1 {print "         " $0}'
fi
show_generations "$WORKDIR/run2.log"
stop_server TERM

echo "==> restart 2 on the same db: nothing is owed, so no resume line"
start_server "$WORKDIR/run3.log"
sleep 5
if grep -q 'resuming search-index rebuilds left outstanding by an earlier run' "$WORKDIR/run3.log"; then
  fail2 "restart 2 resumed rebuilds that were already done:"
  grep 'resuming search-index rebuilds' "$WORKDIR/run3.log" | sed 's/^/         /'
else
  echo "    no resume line"
fi
stop_server TERM

echo
echo "=== result ==="
echo "  phase 1 (live merge)       : $P1_VERDICT"
echo "  phase 2 (restart + resume) : $P2_VERDICT"
echo "  final ledger:"
ledger_rows | sed 's/^/    /'
echo
echo "  $RESULT"
echo "logs: $WORKDIR"
[ "$RESULT" = PASS ]
