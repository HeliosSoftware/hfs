#!/usr/bin/env bash
#
# Sourced library of "Run benchmark suites" helper functions (issue #1475's
# six-leg matrix instrumentation): the Elasticsearch drain gate, the
# per-suite ES/Mongo/composite stats snapshot, the per-suite import
# completeness and Mongo transaction-error writers, the post-suite
# search-counts cross-check, and the end-of-step container/volume state dump
# plus liveness gate. Moved out of that step's inline `run:` block so the
# step's own YAML diff stays small and these functions can be shellchecked
# directly.
#
# Sourced, never executed: keep it non-executable and never run it as
# `bash suite-lib.sh`. Also do NOT add a `set` line here. It is read with
#   source "$GITHUB_WORKSPACE/.github/scripts/fhir-bench/suite-lib.sh"
# near the top of the `benchmark` job's "Run benchmark suites" step
# (fhir-benchmark.yml), so its function bodies run in THAT step's own shell
# and inherit whatever options are already active there: GitHub's own
# default for a `run:` step (`bash -e {0}`) plus that step's own
# `set -uo pipefail` line — together, errexit + nounset + pipefail. Setting
# options in this file would instead change them for the CALLING step from
# the `source` line onward, silently altering behaviour the workflow author
# did not touch.
#
# Because it runs in the step's own shell (not a subshell), every function
# below also sees whatever plain, non-exported shell variables that step has
# already set BY THE TIME THE FUNCTION IS CALLED (not by the time it is
# defined) — in particular RESULTS_DIR (the leg's output directory) and
# ES_DRAIN_TIMEOUT_S (the ES drain deadline), both set once near the top of
# the step, and, inside its per-suite loop, SUITE_WALL (the current suite's
# wall-clock seconds, set right before each call this file's functions are
# used from — the current suite NAME is instead passed as an explicit
# argument, see "Functions" below). It also sees every real environment
# variable earlier steps already exported via $GITHUB_ENV — DOCKER_HOST_IP,
# ES_PORT, ES_PREFIX, ES_CONTAINER, PG_CONTAINER, MONGO_CONTAINER,
# TGZ_CONTAINER, PG_VOLUME, MONGO_VOLUME, ES_VOLUME, HFS_MONGODB_DATABASE,
# HFS_DATABASE_URL, HFS_COMPOSITE_SYNC_MODE, HFS_ELASTICSEARCH_WRITE_REFRESH —
# the normal way a later step's shell inherits an earlier step's $GITHUB_ENV
# writes, so none of them are re-declared here.
#
# Required environment (plain shell variables the "Run benchmark suites"
# step sets for itself near the top of its `run:` block, because a sourced
# file — unlike that step's inline `run:` block — never sees ${{ }} GitHub
# Actions expression substitution; they need not be exported, since sourcing
# runs this file's functions in that same shell):
#   BACKEND        matrix.backend
#   BENCH_PORT     matrix.port (the HFS port this leg's server listens on)
#   BENCH_RUN_ID   github.run_id — deliberately NOT the step's own $RUN_ID
#                  shell variable, which holds the possibly-user-overridden
#                  inputs.run_id tag used for k6 --tag runid=. Container and
#                  volume names, and the drain-probe identifier below, all
#                  need the actual run id, not that tag.
#
# Functions (call order/sites in the step are unchanged from before this
# move — see fhir-benchmark.yml's pointer comments for exactly where each is
# called):
#   es_drain_gate                    barrier + settle poll run before
#                                     `search` on a *-elasticsearch leg (and
#                                     again after search, if it never ran).
#                                     Writes es-drain.txt and the
#                                     es-sync-metrics-{before,after}-drain.txt
#                                     pair.
#   capture_backend_stats SUITE      per-suite ES/Mongo/composite snapshot.
#                                     Writes <suite>-esstat.txt,
#                                     <suite>-mongostat.txt and (ES legs)
#                                     <suite>-syncmetrics.txt.
#   write_import_completeness SUITE  import-completeness.txt plus the
#                                     <1000-bundles ::warning:: (import
#                                     suite only; a no-op for any other
#                                     suite).
#   write_mongo_txn_errors SUITE     import-mongo-txn-errors.txt (mongo*
#                                     legs, import suite only).
#   write_search_counts              post-suite search-counts.txt
#                                     cross-check; re-runs es_drain_gate
#                                     first if it never ran this leg.
#   write_containers_state           containers-state.txt (container
#                                     OOM/exit state + per-volume `du`) plus
#                                     the end-of-run Elasticsearch volume
#                                     fill check.
#   inspect_running CONTAINER        echoes true/false/unknown; used by
#                                     check_leg_containers_alive below.
#   check_leg_containers_alive       fails the step (GITHUB_ENV +
#                                     ::error:: + exit 1) if a backend
#                                     container died during the suites.

# ── Elasticsearch drain gate (search suite, *-elasticsearch legs) ──
# In asynchronous mode a write acks once the PRIMARY commits and ONE
# background worker forwards it to Elasticsearch one at a time
# (composite/sync.rs, mpsc::channel(1000)) — so up to ~1000 events
# from crud/import can still be queued when search starts. Two
# checks, both bounded by the ONE $ES_DRAIN_TIMEOUT_S deadline set
# in the "Run benchmark suites" step (fhir-benchmark.yml), not here
# (barrier + settle together, not 600s each):
#   1. Barrier (exact). A conditional DELETE matching nothing goes
#      through conditional_delete_handler -> ensure_writes_visible
#      -> SyncManager::barrier() (composite/storage.rs,
#      composite/sync.rs): it returns only once the single sync
#      worker has processed every event queued before it, then
#      refreshes Patient. It writes nothing (204/200).
#   2. Settle (catches what the barrier cannot see — e.g. a request
#      k6 already abandoned that is still running server-side, or
#      the periodic sync-repair sweep): poll until the ES doc count
#      AND the primaries' indexing index_total/delete_total are
#      unchanged across 2 consecutive 5s polls, then _refresh.
# Completeness compares live resources in the PRIMARY against
# top-level ES docs (must_not is_contained — contained resources
# are indexed separately) and reads
# composite_secondary_sync_needs_reindex from /metrics. A primary
# count that cannot complete inside its own timeout is recorded as
# status=incomplete reason=count-timeout, distinct from a genuine
# mismatch. Never fails the leg: on any status other than "drained"
# it warns and search still runs anyway, with the reason recorded
# so the numbers are not mistaken for clean.
es_drain_gate() {
  local es="http://$DOCKER_HOST_IP:$ES_PORT"
  local poll_s=5 need_stable=2 stable=0 last="" cur="" docs="" ops=""
  local gate_start=$SECONDS t0 barrier_s=0 barrier_http=000
  local es_live=unknown primary_live=unknown missing=unknown
  local needs_reindex=unknown primary_count_s=0 status=timeout reason=""
  local settle_until min_window min_settle_until

  curl -s --max-time 30 "http://localhost:$BENCH_PORT/metrics" 2>/dev/null \
    | grep -E '^composite_secondary_sync_' > "$RESULTS_DIR/es-sync-metrics-before-drain.txt" || true

  echo "── Elasticsearch drain gate (timeout ${ES_DRAIN_TIMEOUT_S}s) ──"
  t0=$SECONDS
  barrier_http=$(curl -s -o "$RESULTS_DIR/es-drain-probe.txt" -w '%{http_code}' \
      --max-time "$ES_DRAIN_TIMEOUT_S" -X DELETE \
      "http://localhost:$BENCH_PORT/Patient?identifier=urn:hfs-bench:drain-probe%7C$BENCH_RUN_ID") || barrier_http=000
  barrier_s=$((SECONDS - t0))
  echo "  barrier probe: HTTP $barrier_http after ${barrier_s}s"

  # The barrier above shares $ES_DRAIN_TIMEOUT_S with the settle
  # poll below (one deadline across both — see where it's set, in
  # the "Run benchmark suites" step). A barrier that itself takes most of
  # that budget would otherwise leave the settle loop no time to
  # observe even $need_stable consecutive unchanged polls, and a
  # genuinely drained index would misreport as status=timeout.
  # Once the barrier ITSELF succeeds (2xx — proof the sync worker
  # was caught up as of that moment), guarantee a floor: enough
  # polls to reach need_stable, plus one initial read, plus a 5s
  # margin — even if that pushes past the nominal deadline.
  settle_until=$((gate_start + ES_DRAIN_TIMEOUT_S))
  if [ "${barrier_http:0:1}" = "2" ]; then
    min_window=$(( (need_stable + 1) * poll_s + 5 ))
    min_settle_until=$((SECONDS + min_window))
    [ "$min_settle_until" -gt "$settle_until" ] && settle_until=$min_settle_until
  fi

  while [ "$SECONDS" -lt "$settle_until" ]; do
    docs=$(curl -sf --max-time 30 "$es/_cat/count/${ES_PREFIX}_*?h=count" 2>/dev/null | tr -d '[:space:]') || docs=""
    ops=$(curl -sf --max-time 30 "$es/${ES_PREFIX}_*/_stats/indexing?filter_path=_all.primaries.indexing.index_total,_all.primaries.indexing.delete_total" 2>/dev/null \
          | jq -r '"\(._all.primaries.indexing.index_total // 0)/\(._all.primaries.indexing.delete_total // 0)"' 2>/dev/null) || ops=""
    if [ -z "$docs" ] || [ -z "$ops" ]; then
      stable=0; last=""
      echo "  t=$((SECONDS - gate_start))s Elasticsearch did not answer — retrying"
    else
      cur="$docs|$ops"
      if [ "$cur" = "$last" ]; then stable=$((stable + 1)); else stable=0; last="$cur"; fi
      echo "  t=$((SECONDS - gate_start))s docs=$docs index/delete=$ops unchanged=$stable/$need_stable"
      [ "$stable" -ge "$need_stable" ] && break
    fi
    sleep "$poll_s"
  done
  curl -s -o /dev/null --max-time 120 -X POST "$es/${ES_PREFIX}_*/_refresh?allow_no_indices=true" || true

  es_live=$(curl -sf --max-time 30 -H 'Content-Type: application/json' \
      "$es/${ES_PREFIX}_*/_count?allow_no_indices=true" \
      -d '{"query":{"bool":{"must_not":{"term":{"is_contained":true}}}}}' 2>/dev/null \
      | jq -r '.count // "unknown"' 2>/dev/null) || es_live=unknown

  t0=$SECONDS
  case "$BACKEND" in
    postgres-elasticsearch)
      # `timeout 300` only kills the local `docker exec` CLI process
      # — psql itself keeps counting inside the container and would
      # otherwise overlap the search suite that follows. Bound it
      # server-side too (290s, just under the client-side timeout)
      # with statement_timeout.
      # -q suppresses the `SET` command tag. Since psql 15,
      # SHOW_ALL_RESULTS defaults on, so every statement in a -c
      # string prints a result; without -q stdout would be
      # "SET\n<n>" (postgres:18 ships psql 18), which
      # tr -d '[:space:]' turns into "SET<n>" — never a bare
      # integer, so this always fell through to primary_live=unknown.
      primary_live=$(timeout 300 docker exec "$PG_CONTAINER" psql -U postgres -d postgres -qtAc \
          "SET statement_timeout='290s'; SELECT count(*) FROM resources WHERE NOT is_deleted" 2>/dev/null | tr -d '[:space:]') || primary_live=""
      ;;
    mongodb-elasticsearch)
      # idx_resources_type_scan (mongodb/schema.rs
      # RESOURCES_TYPE_SCAN_INDEX) covers (tenant_id, resource_type,
      # is_deleted, last_updated, id) — a valid hint for this filter.
      # maxTimeMS bounds the query server-side for the same reason
      # as statement_timeout above: `timeout 300` alone only kills
      # the local mongosh client.
      primary_live=$(timeout 300 docker exec "$MONGO_CONTAINER" mongosh --quiet "${HFS_MONGODB_DATABASE:-hfs_bench}" --eval \
          'db.resources.countDocuments({is_deleted:false},{hint:"idx_resources_type_scan",maxTimeMS:290000})' 2>/dev/null | tr -d '[:space:]') || primary_live=""
      ;;
    sqlite-elasticsearch)
      # Prefer python3 (this runner is Linux), but don't assume it's
      # there; fall back to python, else skip (recorded as unknown).
      local pybin
      pybin=$(command -v python3 || command -v python || true)
      if [ -n "$pybin" ]; then
        primary_live=$(timeout 300 "$pybin" -c '
import sqlite3, sys
c = sqlite3.connect("file:" + sys.argv[1] + "?mode=ro", uri=True, timeout=60)
print(c.execute("select count(*) from resources where is_deleted=0").fetchone()[0])
' "$HFS_DATABASE_URL" 2>/dev/null | tr -d '[:space:]') || primary_live=""
      else
        primary_live=""
      fi
      ;;
  esac
  primary_count_s=$((SECONDS - t0))
  # Reject anything that isn't a bare integer, not just empty —
  # e.g. a psql/mongosh warning line on stdout would otherwise be
  # compared byte-for-byte against $es_live below and read as a
  # (wrong) "mismatch" instead of "the count is unknown".
  [[ "$primary_live" =~ ^[0-9]+$ ]] || primary_live=unknown

  needs_reindex=$(curl -sf --max-time 30 "http://localhost:$BENCH_PORT/metrics" 2>/dev/null \
      | awk '/^composite_secondary_sync_needs_reindex/{v=$NF} END{print v+0}') || needs_reindex=unknown
  curl -s --max-time 30 "http://localhost:$BENCH_PORT/metrics" 2>/dev/null \
    | grep -E '^composite_secondary_sync_' > "$RESULTS_DIR/es-sync-metrics-after-drain.txt" || true

  if [[ "$primary_live" =~ ^[0-9]+$ ]] && [[ "$es_live" =~ ^[0-9]+$ ]]; then
    missing=$((primary_live - es_live))
  fi

  # Ordered by which signal is the strongest evidence of a real
  # problem, so e.g. a failed barrier is never reported as a plain
  # "mismatch". count-timeout vs count-failed is a heuristic: the
  # `$(docker exec ... | tr ...) || primary_live=""` fallback
  # discards the pipeline's exit status (so the inner `timeout
  # 300`'s 124 is never inspected) — elapsed time (primary_count_s)
  # stands in for "the server-side timeout fired" instead.
  if [ "${barrier_http:0:1}" != "2" ]; then
    status=incomplete; reason=barrier-failed
  elif [ "$stable" -lt "$need_stable" ]; then
    status=timeout
  elif [ "$es_live" = unknown ]; then
    status=incomplete; reason=index-count-failed
  elif [ "$primary_live" = unknown ]; then
    # The server-side bounds above (statement_timeout / maxTimeMS)
    # fire at ~290s, well under the 300s client-side `timeout`, so
    # a real server-side timeout is observed here as ~290-291s
    # elapsed, not ~299s. 285 separates that from a near-instant
    # failure (bad container, missing client) without relying on
    # the discarded exit code (see the pipefail note above).
    if [ "$primary_count_s" -ge 285 ]; then
      status=incomplete; reason=count-timeout
    else
      status=incomplete; reason=count-failed
    fi
  elif [ "$primary_live" = "$es_live" ]; then
    status=drained
  else
    status=incomplete; reason=mismatch
  fi

  {
    echo "status=$status"
    echo "reason=$reason"
    echo "barrier_http=$barrier_http"
    echo "barrier_seconds=$barrier_s"
    echo "primary_live=$primary_live"
    echo "es_live=$es_live"
    echo "missing=$missing"
    echo "primary_count_seconds=$primary_count_s"
    echo "needs_reindex_after=$needs_reindex"
    echo "gate_seconds=$((SECONDS - gate_start))"
    echo "sync_mode=${HFS_COMPOSITE_SYNC_MODE:-unknown}"
    echo "write_refresh=${HFS_ELASTICSEARCH_WRITE_REFRESH:-unknown}"
  } > "$RESULTS_DIR/es-drain.txt"
  cat "$RESULTS_DIR/es-drain.txt"
  if [ "$status" != drained ]; then
    echo "::warning::Elasticsearch drain gate status=$status${reason:+ reason=$reason} (barrier_http=$barrier_http primary_live=$primary_live es_live=$es_live) — search may run against an incomplete index (es-drain.txt)."
  fi
  return 0
}

# ── Per-suite backend snapshot — the ES/Mongo/composite analogue of
# the pgstat block (still inline in fhir-benchmark.yml). Counters are
# CUMULATIVE where noted; bound every call and never fail the leg over a
# stats capture.
capture_backend_stats() {
  local suite="$1"
  if [ -n "${ES_PORT:-}" ]; then
    local es="http://$DOCKER_HOST_IP:$ES_PORT"
    {
      echo "suite=$suite wall_seconds=$SUITE_WALL"
      echo "── cluster health (yellow expected: 1 replica, 1 node) ──"
      curl -s --max-time 30 "$es/_cluster/health?filter_path=status,active_shards,unassigned_shards" 2>/dev/null || true
      echo
      echo "── ${ES_PREFIX}_* indices by size ──"
      curl -s --max-time 30 "$es/_cat/indices/${ES_PREFIX}_*?v&h=index,health,docs.count,docs.deleted,store.size&s=store.size:desc" 2>/dev/null | head -25 || true
      echo "── node: jvm heap, indexing, write/search thread-pool rejections ──"
      curl -s --max-time 30 "$es/_nodes/stats/jvm,indices,thread_pool?filter_path=nodes.*.jvm.mem.heap_used_percent,nodes.*.indices.indexing.index_total,nodes.*.indices.indexing.index_time_in_millis,nodes.*.thread_pool.write.rejected,nodes.*.thread_pool.search.rejected" 2>/dev/null || true
      echo
      echo "── container ──"
      timeout 60 docker stats --no-stream --format '{{.Name}} mem={{.MemUsage}} cpu={{.CPUPerc}}' "$ES_CONTAINER" 2>/dev/null || true
    } > "$RESULTS_DIR/${suite}-esstat.txt" 2>&1 || echo "::warning::ES stats capture failed for $suite"
  fi
  if [ -n "${MONGO_CONTAINER:-}" ]; then
    {
      echo "suite=$suite wall_seconds=$SUITE_WALL"
      timeout 60 docker exec "$MONGO_CONTAINER" mongosh --quiet "${HFS_MONGODB_DATABASE:-hfs_bench}" --eval '
        const s = db.serverStatus(), c = s.wiredTiger.cache;
        printjson({
          wt_cache_bytes: c["bytes currently in the cache"],
          wt_cache_max_bytes: c["maximum bytes configured"],
          wt_dirty_bytes: c["tracked dirty bytes in the cache"],
          wt_app_thread_evictions: c["pages evicted by application threads"],
          opcounters: s.opcounters,
          txn: { started: s.transactions.totalStarted, committed: s.transactions.totalCommitted, aborted: s.transactions.totalAborted },
          connections: s.connections.current
        });' 2>/dev/null || true
      timeout 60 docker stats --no-stream --format '{{.Name}} mem={{.MemUsage}} cpu={{.CPUPerc}}' "$MONGO_CONTAINER" 2>/dev/null || true
    } > "$RESULTS_DIR/${suite}-mongostat.txt" 2>&1 || echo "::warning::Mongo stats capture failed for $suite"
  fi
  case "$BACKEND" in
    *-elasticsearch)
      {
        echo "suite=$suite wall_seconds=$SUITE_WALL"
        curl -s --max-time 30 "http://localhost:$BENCH_PORT/metrics" 2>/dev/null | grep -E '^composite_secondary_sync_' \
          || echo "(none recorded at this point)"
      } > "$RESULTS_DIR/${suite}-syncmetrics.txt" 2>&1 || echo "::warning::composite sync metrics capture failed for $suite"
      ;;
  esac
}

# ── Import completeness (F1a), all legs ──────────────────────────
# k6 caps import at 60m (import.js maxDuration); an async
# *-elasticsearch leg is expected to hit that cap (one sequential
# sync worker, two ES requests per resource). A truncated corpus
# makes crud/search look better than a full 1000/1000 leg, so flag
# it rather than let the numbers pass as comparable.
write_import_completeness() {
  local suite="$1"
  if [ "$suite" = import ] && [ -f "$RESULTS_DIR/import.json" ]; then
    local IMP_OK IMP_ITERS IMP_ENTRIES IMP_OK_INT
    IMP_OK=$(jq -r '.metrics.checks.passes // 0' "$RESULTS_DIR/import.json" 2>/dev/null) || IMP_OK=0
    IMP_ITERS=$(jq -r '.metrics.iterations.count // 0' "$RESULTS_DIR/import.json" 2>/dev/null) || IMP_ITERS=0
    IMP_ENTRIES=$(jq -r '.metrics.bundle_size.count // 0' "$RESULTS_DIR/import.json" 2>/dev/null) || IMP_ENTRIES=0
    {
      echo "bundles_ok=$IMP_OK"
      echo "iterations=$IMP_ITERS"
      echo "entries=$IMP_ENTRIES"
      echo "wall_seconds=$SUITE_WALL"
    } > "$RESULTS_DIR/import-completeness.txt"
    IMP_OK_INT="${IMP_OK%.*}"
    case "$IMP_OK_INT" in ''|*[!0-9]*) IMP_OK_INT=0 ;; esac
    if [ "$IMP_OK_INT" -lt 1000 ]; then
      echo "::warning::import completed ${IMP_OK}/1000 bundles (${IMP_ENTRIES} entries) — k6's 60-min cap or failed bundles. crud/search on this leg ran on a smaller corpus and are not comparable to a 1000/1000 leg."
    fi
  elif [ "$suite" = import ]; then
    # k6 died or was killed before writing any summary at all — the
    # most incomplete import there is. Without this branch the
    # completeness file (and its warning) are silently skipped
    # entirely, and crud/search look like any other leg.
    printf 'bundles_ok=0\niterations=0\nentries=0\nwall_seconds=%s\nnote=no-k6-summary\n' \
      "$SUITE_WALL" > "$RESULTS_DIR/import-completeness.txt"
    echo "::warning::import produced no k6 summary (import.json missing — k6 died or was killed) — crud/search on this leg are not comparable to any leg that completed an import."
  fi
}

# ── Mongo transaction-abort count (F8), mongo legs, after import ──
# A bundle is one multi-document transaction over sequential
# entries with no whole-transaction retry (mongodb/storage.rs) —
# any abort fails the bundle. 20 concurrent transactions against a
# capped WiredTiger cache can plausibly hit these.
write_mongo_txn_errors() {
  local suite="$1"
  if [ "$suite" = import ]; then
    case "$BACKEND" in
      mongodb*)
        local MONGO_TXN_ERR
        MONGO_TXN_ERR=$(grep -ciE 'WriteConflict|TransientTransactionError|TransactionTooLargeForCache|NoSuchTransaction|Commit failed' \
            "/tmp/hfs-bench-$BACKEND.log" 2>/dev/null) || MONGO_TXN_ERR=0
        echo "$MONGO_TXN_ERR" > "$RESULTS_DIR/import-mongo-txn-errors.txt"
        ;;
    esac
  fi
}

# ── Result-size cross-check (F5), all legs ─────────────────────────
# Runs AFTER search so it cannot warm caches for the measured suite.
# k6's search checks are status-only, so a composite search that
# silently degrades to matching on the parameter name alone (or a
# Mongo sa/eb range mistranslation) would otherwise pass unnoticed —
# different result-set sizes change latency and nothing else would
# show it. Single-quoted so `$gt100`/`$gt140` reach the query string
# literally instead of being shell-expanded.
#
# On an ES leg run WITHOUT 'search' in tests (e.g. tests=prewarm,import),
# the suite loop never called es_drain_gate, so up to ~1000 queued
# async sync events could still be unprocessed — these counts would
# then read low and look like a cross-leg mismatch that isn't one.
# Run the gate here too when it hasn't already run this leg.
write_search_counts() {
  if [ -n "${ES_PORT:-}" ] && [ ! -f "$RESULTS_DIR/es-drain.txt" ]; then
    es_drain_gate || echo '::warning::drain gate errored'
  fi
  {
    echo "query|total|http|seconds"
    # shellcheck disable=SC2016 # single-quoted deliberately: $gt100/$gt140
    # must reach the query string literally, not shell-expand.
    for Q in 'Patient?_summary=count' 'Observation?_summary=count' 'Encounter?_summary=count' \
             'Patient?name:contains=an&_summary=count' 'Observation?date=ge2020-01-01&_summary=count' \
             'Observation?date=sa2020-01-01&_summary=count' \
             'Observation?code=8302-2,29463-7&_summary=count' 'Observation?value-quantity=gt100&_summary=count' \
             'Observation?value-quantity=sa100&_summary=count' \
             'Observation?code-value-quantity=http://loinc.org%7C8867-4$gt100&_summary=count' \
             'Observation?combo-code-value-quantity=8480-6$gt140&_summary=count' 'Encounter?class=AMB,EMER&_summary=count'; do
      local SC_T0 SC_OUT SC_TOTAL SC_HTTP
      SC_T0=$SECONDS
      SC_OUT=$(curl -s --max-time 30 -w '\n%{http_code}' "http://localhost:$BENCH_PORT/$Q") || SC_OUT=$'\n000'
      SC_TOTAL=$(printf '%s\n' "$SC_OUT" | head -n -1 | jq -r '.total // "?"' 2>/dev/null) || SC_TOTAL="?"
      SC_HTTP=$(printf '%s\n' "$SC_OUT" | tail -n1)
      echo "$Q|$SC_TOTAL|$SC_HTTP|$((SECONDS - SC_T0))"
    done
  } > "$RESULTS_DIR/search-counts.txt" || echo "::warning::search-counts cross-check failed"
}

# ── Container / volume final state (end of step) ───────────────────
# k6 runs with `|| true` throughout, so a mid-run OOM would otherwise
# only ever show up as mysteriously bad numbers, never as a failed
# step. This is the step's own final action: every result file above
# is already written, so a fail-fast exit here loses nothing.
write_containers_state() {
  : > "$RESULTS_DIR/containers-state.txt"
  local c CS_OUT
  for c in ${PG_CONTAINER:-} ${MONGO_CONTAINER:-} ${ES_CONTAINER:-} ${TGZ_CONTAINER:-}; do
    # Tell a Docker daemon hiccup apart from proof the container is
    # actually gone (Docker's own "no such container"), same
    # distinction inspect_running() draws below — "Generate step
    # summary" reads "gone" as died and "unknown" as a neutral note,
    # not a death.
    if CS_OUT=$(timeout 60 docker inspect -f "$c OOMKilled={{.State.OOMKilled}} Status={{.State.Status}} ExitCode={{.State.ExitCode}} StartedAt={{.State.StartedAt}}" "$c" 2>&1); then
      printf '%s\n' "$CS_OUT" >> "$RESULTS_DIR/containers-state.txt"
    elif printf '%s' "$CS_OUT" | grep -qi 'no such'; then
      echo "$c inspect-failed gone" >> "$RESULTS_DIR/containers-state.txt"
    else
      echo "$c inspect-failed unknown" >> "$RESULTS_DIR/containers-state.txt"
    fi
  done
  local VOLDU_I=0 v VOLDU_NAME VOLDU_MB
  for v in ${PG_VOLUME:-} ${MONGO_VOLUME:-} ${ES_VOLUME:-}; do
    VOLDU_I=$((VOLDU_I + 1))
    VOLDU_NAME="hfs-bench-voldu-$BACKEND-$BENCH_RUN_ID-$VOLDU_I"
    # The outer `timeout 150` only kills the local docker CLI, not a
    # container still running on the daemon — an outer-timeout kill
    # would otherwise leave `du` running with this volume's :ro mount
    # still open, and the later `docker volume rm -f` in "Stop
    # ephemeral ..." would then fail and leak the volume until the
    # next run's reaper. The inner `timeout -s KILL 120 du` bounds du
    # ITSELF, well inside the outer timeout, so (with --rm) the
    # container always exits and is removed on its own; the distinct
    # per-volume name avoids a stale container from one volume
    # blocking the next volume's `--name`.
    VOLDU_MB=$(timeout 150 docker run --rm --name "$VOLDU_NAME" \
        --label hfs-bench=1 --label "hfs-bench-run=$BENCH_RUN_ID" --label "hfs-bench-leg=$BACKEND" \
        -v "$v:/v:ro" alpine:3 timeout -s KILL 120 du -sm /v 2>/dev/null | awk '{print $1}') || VOLDU_MB=""
    docker rm -f "$VOLDU_NAME" >/dev/null 2>&1 || true
    echo "$v du_mb=${VOLDU_MB:-unknown}" >> "$RESULTS_DIR/containers-state.txt"
  done
  # Elasticsearch indices go read-only at the 95% flood-stage
  # watermark; the only check so far ran at leg start, before ES had
  # written anything. Check again now that import/crud/search have
  # actually filled the volume.
  if [ -n "${ES_VOLUME:-}" ]; then
    local ES_VOLDF_END_NAME ES_VOL_PCT_USED_END
    ES_VOLDF_END_NAME="hfs-bench-voldf-$BACKEND-$BENCH_RUN_ID-esend"
    ES_VOL_PCT_USED_END=$(timeout 60 docker run --rm --name "$ES_VOLDF_END_NAME" \
        --label hfs-bench=1 --label "hfs-bench-run=$BENCH_RUN_ID" --label "hfs-bench-leg=$BACKEND" \
        -v "$ES_VOLUME:/v" alpine:3 df -Pm /v 2>/dev/null | awk 'NR==2{gsub("%","",$5); print $5}') || ES_VOL_PCT_USED_END=""
    # See the Postgres step's identical note: `timeout` alone cannot be
    # trusted to leave `--rm` a clean container to remove.
    timeout 15 docker rm -f "$ES_VOLDF_END_NAME" >/dev/null 2>&1 || true
    if [[ "$ES_VOL_PCT_USED_END" =~ ^[0-9]+$ ]] && [ "$ES_VOL_PCT_USED_END" -gt 85 ]; then
      echo "::warning::Elasticsearch data volume $ES_VOLUME filesystem is ${ES_VOL_PCT_USED_END}% used at end of run — indices go read-only at Elasticsearch's 95% flood-stage watermark."
    fi
  fi
  cat "$RESULTS_DIR/containers-state.txt"
}

# Only the actual backend (primary/ES) containers gate the leg —
# the tgz corpus server dying mid-run is logged above but does not,
# by itself, invalidate this leg's numbers. A container that is not
# Running (OOM-killed, exited, or gone entirely) fails the gate.
#
# `docker inspect` failing is ambiguous by itself — it also fails on
# a daemon hiccup, which must NOT be read as "died". Tell them apart
# by stderr: Docker's own "No such container" is unambiguous proof
# the container is gone (never coming back), so it fails the gate
# immediately; any other failure is a possible hiccup and gets one
# retry before being read as "unknown" (warn only, not counted as
# died).
inspect_running() {
  local out
  if out=$(timeout 60 docker inspect -f '{{.State.Running}}' "$1" 2>&1); then
    # `2>&1` above also captures a Docker CLI stderr warning printed
    # ahead of the value (e.g. "DOCKER_HOST environment variable
    # overrides the active context") — an unfiltered `echo "$out"`
    # would then return "<warning>\ntrue" (never == "true") and read
    # a live container as died. Only the last line is the actual
    # `{{.State.Running}}` value.
    printf '%s\n' "$out" | tail -n1
  elif printf '%s' "$out" | grep -qi 'no such'; then
    echo "false"
  else
    echo "unknown"
  fi
}

check_leg_containers_alive() {
  local c RUNNING
  LEG_DIED=false
  for c in ${PG_CONTAINER:-} ${MONGO_CONTAINER:-} ${ES_CONTAINER:-}; do
    RUNNING=$(inspect_running "$c")
    if [ "$RUNNING" = unknown ]; then
      sleep 2
      RUNNING=$(inspect_running "$c")
    fi
    if [ "$RUNNING" = unknown ]; then
      echo "::warning::could not inspect container $c to confirm it is still running (Docker host hiccup?) — not counted as died"
    elif [ "$RUNNING" != "true" ]; then
      LEG_DIED=true
    fi
  done
  if [ "$LEG_DIED" = true ]; then
    echo "LEG_CONTAINER_DIED=true" >> "$GITHUB_ENV"
    echo "::error::A backend container for leg '$BACKEND' died or was OOM-killed during the suites — this leg's numbers are not valid (containers-state.txt)."
    exit 1
  fi
}
