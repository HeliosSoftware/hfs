#!/usr/bin/env bash
#
# HFS_BULK_SUBMIT_DEFER_INDEXING=false vs true: which should be the default?
#
# Origin: smunini on PR #946 — "We want initial user experience to be the
# fastest import as possible. If the test discovers that
# HFS_BULK_SUBMIT_DEFER_INDEXING=true is best, we need to change the default."
#
# WHAT THIS MEASURES, AND WHY IT IS NOT JUST THE INGEST RATE
#
# Deferring indexing does not make the work disappear, it moves it. The worker
# marks the manifest terminal and only then fires the reindex, and it fires it
# fire-and-forget (`bulk_submit_worker.rs:902` -> `reindex.rs:554`,
# `tokio::spawn`). So `$bulk-submit-status` answers 200 while the search index
# is still empty: `test_deferred_indexing_defers_search_and_fires_the_hook`
# (bulk_submit_worker.rs:1512) asserts exactly that, 0 search hits right after
# a deferred ingest.
#
# Timing only the ingest would therefore reward the arm that hides its work.
# This script records two instants per arm:
#
#   t_ingest    kick-off -> poll returns 200      (the import "looks" done)
#   t_search    kick-off -> a search for the      (the data IS usable)
#               ingested resources returns the
#               full expected count
#
# t_search is the number the default should be chosen on. A user whose import
# finishes in half the time but who then searches and gets nothing has not had
# a better first experience.
#
# METHOD
#
# The skill's two documented traps are both avoided:
#   * run from the repo root so `data/search-parameters-r4.json` is picked up;
#     without it the registry falls back to five embedded parameters, index
#     volume collapses from ~14 rows per resource to ~2 and every arm looks
#     fast (`/test-hfs`, "Benchmarking discipline").
#   * arms are INTERLEAVED in one loop (false, true, false, true, ...), never
#     one run against another session's number.
#
# Arms are compared on the MINIMUM across rounds, with the median printed
# beside it. This machine hosts several worktrees, and another one's server
# starting mid-run can only ADD time to whichever arm is in flight — so the
# minimum is the best estimate of the uncontended cost, and the `other-hfs`
# column records how loaded the machine was. If min and median disagree about
# the winner the script says INCONCLUSIVE rather than pick one.
#
# Each round uses a fresh database and a fresh server, so no arm inherits the
# other's page cache state at the SQLite level.
#
# It frees no ports and touches no process it did not start: it picks free
# ports >18000 by binding them, and it stops only the server PIDs it spawned
# itself (recorded in SERVER_PIDS, also cleaned up by an EXIT trap). Servers
# are stopped between rounds on purpose — a previous arm still running its
# background reindex would compete with the arm being measured.
#
# Requirements: cargo, curl and python on PATH. On Windows the interpreter is
# invoked as `python`; `python3` resolves to the Microsoft Store stub.
#
#   crates/hfs/tests/bulk_submit/run_defer_indexing_benchmark.sh
#   ROUNDS=5 TOTAL=40000 crates/hfs/tests/bulk_submit/run_defer_indexing_benchmark.sh
#   SKIP_BUILD=1 crates/hfs/tests/bulk_submit/run_defer_indexing_benchmark.sh
#
set -euo pipefail

cd "$(dirname "$0")/../../../.."

ROUNDS="${ROUNDS:-5}"
FILES="${FILES:-4}"
PER_FILE="${PER_FILE:-2500}"
WORKDIR="${WORKDIR:-/tmp/hfs-defer-bench}"
TOTAL=$((FILES * PER_FILE))

# Counts `hfs` servers that are NOT this benchmark's, i.e. other worktrees
# doing their own work on the same disk. It only counts; it never touches them.
#
# This exists because it had to: a first attempt at this measurement had an
# identical `false` arm take 60s in round 1 and >18min in round 2, and the
# cause was six servers from two other worktrees starting within 12s of round
# 2. Without this column that run would have looked like a property of
# defer_indexing.
# The Windows branch goes through a .ps1 file rather than `powershell -Command`
# because PowerShell's `$_` does not survive being quoted inside a bash string.
foreign_servers() {
  case "$(uname -s)" in
    MINGW* | MSYS* | CYGWIN*)
      powershell -NoProfile -ExecutionPolicy Bypass -File "$WORKDIR/foreign.ps1" \
        2>/dev/null | tr -dc '0-9' ;;
    *) pgrep -c -f '[h]fs ' 2>/dev/null || echo 0 ;;
  esac
}

# A real probe, not a netstat scrape: netstat output is not portable and a
# non-matching pattern would make every port look free.
#
# connect() FIRST, and that order matters. On Windows SO_REUSEADDR lets a
# second socket bind a port that is actively LISTENING, so a bind-only test
# reports every port free. That is not theoretical: it made a run of this
# benchmark silently talk to a leftover provider from an earlier run and
# ingest its 20000-resource fixture instead of its own 800.
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

rm -rf "$WORKDIR"
mkdir -p "$WORKDIR"

cat > "$WORKDIR/foreign.ps1" <<'PS1'
# Counts only. Nothing here selects a process to act on.
@(Get-CimInstance Win32_Process -Filter "Name='hfs.exe'" |
    Where-Object { $_.CommandLine -notlike '*hfs-defer-bench*' }).Count
PS1

# Only PIDs this script started are ever recorded here, and only these are ever
# stopped. Nothing else on the machine is inspected, matched by name, or killed.
SERVER_PIDS=()
stop_own_servers() {
  local pid
  for pid in "${SERVER_PIDS[@]:-}"; do
    [ -n "$pid" ] || continue
    kill "$pid" 2>/dev/null || true
  done
  SERVER_PIDS=()
}
trap stop_own_servers EXIT

# --- fixture -------------------------------------------------------------
# One shared corpus for every arm and round: the two arms must ingest byte
# identical input or the comparison measures the fixture, not the setting.
# `family` is what the search probe queries, so it is the same for all.
echo "==> generating $FILES files x $PER_FILE Patients = $TOTAL resources"
PROVIDER_PORT="$(pick_port "${PROVIDER_PORT:-19300}")"
PROVIDER_URL="http://127.0.0.1:$PROVIDER_PORT"
python - "$WORKDIR" "$FILES" "$PER_FILE" "$PROVIDER_URL" <<'PY'
import json, sys, pathlib
workdir, files, per_file, provider = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), sys.argv[4]
out = []
for f in range(files):
    name = f"patients-{f}.ndjson"
    with open(pathlib.Path(workdir) / name, "w", encoding="utf-8") as fh:
        for i in range(per_file):
            fh.write(json.dumps({
                "resourceType": "Patient",
                "id": f"defer-{f}-{i}",
                "identifier": [{"system": "http://example.org/mrn", "value": f"MRN{f}-{i}"}],
                "name": [{"family": "Deferbench", "given": [f"F{f}", f"I{i}"]}],
                "telecom": [{"system": "phone", "value": f"555-{f:03d}-{i:04d}"}],
                "gender": "female" if i % 2 == 0 else "male",
                "birthDate": "1980-01-01",
                "address": [{"city": "Springfield", "state": "IL", "postalCode": "62701"}],
            }) + "\n")
    out.append({"type": "Patient", "url": f"{provider}/{name}", "count": per_file})
(pathlib.Path(workdir) / "manifest.json").write_text(json.dumps({
    "transactionTime": "2024-01-01T00:00:00Z",
    "request": f"{provider}/manifest.json",
    "requiresAccessToken": False,
    "output": out, "error": [], "deleted": [],
}, indent=2), encoding="utf-8")
print(f"    manifest with {len(out)} output entries")
PY

# --- build ---------------------------------------------------------------
# RELEASE, not debug. In a debug build the index extraction is disproportionately
# slow, which inflates exactly the work the deferred arm skips and would bias
# the comparison towards `true`.
if [ "${SKIP_BUILD:-0}" = "1" ]; then
  echo "==> SKIP_BUILD=1: reusing the already built binary"
else
  echo "==> building hfs (release)"
  case "$(uname -s)" in
    MINGW* | MSYS* | CYGWIN*)
      cargo rustc --release -p helios-hfs --bin hfs -- -C link-arg=/STACK:33554432 ;;
    *)
      cargo build --release -p helios-hfs ;;
  esac
fi
if [ -z "${HFS_BIN:-}" ]; then
  HFS_BIN="target/release/hfs"
  [ -x "$HFS_BIN" ] || HFS_BIN="target/release/hfs.exe"
fi

echo "==> provider on $PROVIDER_URL"
# `-u`: stdout is block buffered when redirected, so without it the log is
# empty until the process ends and a startup failure is invisible.
( cd "$WORKDIR" && exec python -u -m http.server "$PROVIDER_PORT" --bind 127.0.0.1 ) \
  > "$WORKDIR/provider.log" 2>&1 &
SERVER_PIDS+=("$!")
for _ in $(seq 1 30); do
  curl -sS -o /dev/null "$PROVIDER_URL/manifest.json" 2>/dev/null && break
  sleep 1
done

# --- one arm -------------------------------------------------------------
# Prints "<t_ingest> <t_search> <indexed_rows>" on stdout; everything else
# goes to stderr so the caller can capture the numbers cleanly.
run_arm() {
  local defer="$1" round="$2"
  local tag="${defer}-r${round}"
  local db="target/hfs-defer-bench-$tag.db"
  local port; port="$(pick_port 18900)"
  local url="http://127.0.0.1:$port"
  local log="$WORKDIR/hfs-$tag.log"

  rm -f "$db" "$db-wal" "$db-shm"

  # The status poll is rate limited to 10 hits per 60s by default
  # (config.rs:704), which would force a >=6s polling interval and quantise
  # t_ingest to 6s — a tenth of the whole measurement. Raising the budget
  # buys 1s resolution. It is set identically for both arms and throttles
  # nothing on the ingest path.
  HFS_BASE_URL="$url" \
  HFS_BULK_SUBMIT_ENABLED=true \
  HFS_BULK_SUBMIT_DEFER_INDEXING="$defer" \
  HFS_BULK_SUBMIT_POLL_RATE_LIMIT=100000 \
  HFS_LOG_LEVEL=info \
    "$HFS_BIN" \
      --database-url "$db" --log-level info \
      --host 127.0.0.1 --port "$port" \
    > "$log" 2>&1 &
  local server_pid=$!
  SERVER_PIDS+=("$server_pid")

  local up=0
  for _ in $(seq 1 90); do
    curl -sS -o /dev/null "$url/health" 2>/dev/null && { up=1; break; }
    sleep 1
  done
  [ "$up" = "1" ] || { echo "server did not come up, see $log" >&2; return 1; }

  cat > "$WORKDIR/submit-$tag.json" <<EOF
{ "resourceType": "Parameters", "parameter": [
  { "name": "submitter", "valueIdentifier": { "system": "http://example.org", "value": "bench" } },
  { "name": "submissionId", "valueString": "defer-$tag" },
  { "name": "manifestUrl", "valueUrl": "$PROVIDER_URL/manifest.json" },
  { "name": "fhirBaseUrl", "valueUrl": "$PROVIDER_URL/fhir" },
  { "name": "submissionStatus", "valueCoding": { "system": "http://hl7.org/fhir/event-status", "code": "completed" } } ] }
EOF
  cat > "$WORKDIR/status-$tag.json" <<EOF
{ "resourceType": "Parameters", "parameter": [
  { "name": "submitter", "valueIdentifier": { "system": "http://example.org", "value": "bench" } },
  { "name": "submissionId", "valueString": "defer-$tag" } ] }
EOF

  # Sub-second, because the deferred arm's ingest is expected to be short
  # enough that whole seconds would be a coarse ruler for it.
  local start; start=$(date +%s.%N)
  elapsed() { python -c "import sys;print(f'{float(sys.argv[2])-float(sys.argv[1]):.1f}')" \
    "$start" "$(date +%s.%N)"; }

  curl -sS -o /dev/null -X POST "$url/\$bulk-submit" \
    -H 'Content-Type: application/fhir+json' --data-binary @"$WORKDIR/submit-$tag.json"
  local loc; loc=$(curl -sS -D - -o /dev/null -X POST "$url/\$bulk-submit-status" \
    -H 'Content-Type: application/fhir+json' --data-binary @"$WORKDIR/status-$tag.json" \
    | tr -d '\r' | awk -F': ' 'tolower($1)=="content-location"{print $2}')
  [ -n "$loc" ] || { echo "no Content-Location, see $log" >&2; return 1; }

  # 202 while manifests are still processing, 200 once all are terminal
  # (bulk_submit.rs:815 and :964).
  local t_ingest=-1
  for _ in $(seq 1 1200); do
    local code; code=$(curl -sS -o /dev/null -w '%{http_code}' "$loc" 2>/dev/null || echo 000)
    if [ "$code" = "200" ]; then t_ingest=$(elapsed); break; fi
    sleep 1
  done

  # Searchability: a plain indexed search. Under `false` it is already
  # satisfied when the ingest finishes; under `true` it only passes once the
  # fire-and-forget reindex has rebuilt the index.
  local t_search=-1 rows=0
  for _ in $(seq 1 1200); do
    rows=$(curl -sS "$url/Patient?family=Deferbench&_summary=count" 2>/dev/null \
      | python -c 'import json,sys
try: print(json.load(sys.stdin).get("total", 0) or 0)
except Exception: print(0)' 2>/dev/null || echo 0)
    if [ "${rows:-0}" -ge "$TOTAL" ]; then t_search=$(elapsed); break; fi
    sleep 1
  done

  # Contention markers, captured before the server is stopped. `busy` is the
  # #942 symptom; `foreign` is how loaded the machine was during this arm.
  local foreign; foreign=$(foreign_servers)
  local busy; busy=$(grep -c 'database is locked' "$log" 2>/dev/null || true)
  busy="${busy:-0}"

  # Stops only the PID started just above. Doing it here and not at exit is
  # deliberate: a server still running its background reindex would compete
  # with the next arm for the same cores.
  kill "$server_pid" 2>/dev/null || true
  wait "$server_pid" 2>/dev/null || true
  sleep 1

  echo "$t_ingest $t_search $rows $busy $foreign"
}

# --- interleaved rounds --------------------------------------------------
echo
echo "==> $ROUNDS interleaved rounds, $TOTAL resources each, release build"
FALSE_ING=(); FALSE_SEA=(); TRUE_ING=(); TRUE_SEA=()
for r in $(seq 1 "$ROUNDS"); do
  for arm in false true; do
    echo "    round $r, defer_indexing=$arm ..."
    read -r ti ts rows busy foreign <<< "$(run_arm "$arm" "$r")"
    echo "        t_ingest=${ti}s  t_search=${ts}s  searchable=${rows}/${TOTAL}" \
         " busy=${busy} other-hfs=${foreign}"
    if [ "$arm" = "false" ]; then FALSE_ING+=("$ti"); FALSE_SEA+=("$ts")
    else TRUE_ING+=("$ti"); TRUE_SEA+=("$ts"); fi
  done
done

echo
echo "=== results over $ROUNDS rounds ($TOTAL resources each) ==="
python - "$TOTAL" "${FALSE_ING[*]}" "${FALSE_SEA[*]}" "${TRUE_ING[*]}" "${TRUE_SEA[*]}" <<'PY'
import statistics, sys
total = int(sys.argv[1])

def vals(s):
    return [float(x) for x in s.split() if x not in ("", "-1")]

fi, fs, ti, ts = (vals(sys.argv[i]) for i in (2, 3, 4, 5))

# MINIMUM, not just median. Competing load on this machine can only ADD time
# to an arm, never remove it, so across rounds the minimum is the best
# estimate of the uncontended cost and the statistic least damaged by another
# worktree's server starting mid-run. The median is printed beside it: if the
# two disagree about which arm wins, the run was too noisy to conclude from.
def line(label, a, b):
    if not a or not b:
        print(f"  {label:<24} INCOMPLETE (false={a} true={b})")
        return None
    amin, bmin = min(a), min(b)
    amed, bmed = statistics.median(a), statistics.median(b)
    win = "true" if bmin < amin else "false"
    print(f"  {label:<24} false: min={amin:6.1f}s med={amed:6.1f}s   "
          f"true: min={bmin:6.1f}s med={bmed:6.1f}s   -> {win} faster "
          f"({max(amin,bmin)/min(amin,bmin):.2f}x)")
    return (amin, bmin, amed, bmed)

a = line("t_ingest (poll->200)", fi, ti)
b = line("t_search (usable)", fs, ts)
print()
if not a or not b:
    print("  INCONCLUSIVE: a probe never completed; see the logs.")
else:
    fs_min, ts_min, fs_med, ts_med = b
    print(f"  ingest rate    false={total/a[0]:7.0f}/s   true={total/a[1]:7.0f}/s")
    print(f"  time-to-usable false={total/fs_min:7.0f}/s   true={total/ts_min:7.0f}/s")
    print()
    if (ts_min < fs_min) != (ts_med < fs_med):
        print("  INCONCLUSIVE: min and median disagree on the winner, so the run")
        print("  was too noisy. Re-run when the machine is quiet (watch the")
        print("  other-hfs column above).")
    elif ts_min < fs_min:
        print("  => `true` reaches a USABLE database sooner: change the default.")
    else:
        print("  => `true` finishes the INGEST sooner but the database becomes")
        print("     usable LATER, by "
              f"{ts_min - fs_min:.1f}s ({ts_min/fs_min:.2f}x). `false` is the")
        print("     better first experience; keep it as the default.")
PY

echo
echo "logs: $WORKDIR"
