#!/usr/bin/env python3
#
# Per-leg benchmark diagnostics for the step summary: tuning table, import
# completeness and throughput (resources/s), ES drain status, host load at
# crud start, "how to read this leg" guidance, the result-size cross-check +
# crud-residue caption, Mongo transaction-error heuristic, composite ES
# sync-failure count, and dead-container warnings.
#
# Called from: the `benchmark` job's "Generate step summary" step
# (fhir-benchmark.yml), via:
#   python3 "$GITHUB_WORKSPACE/.github/scripts/fhir-bench/summary_backends.py"
# runs before the Results table (after the header lines) inside that step's
# `{ ... } >> "$GITHUB_STEP_SUMMARY"` group, so its stdout lands at the same
# position this block originally printed at: after the leg header table,
# before the step's own (still inline) Results table and Environment
# paragraph — this prints unconditionally, same as before, regardless of
# whether any *.json summary file exists.
#
# Required environment (exported by the workflow step's env:):
#   BACKEND               matrix.backend
#   IN_PG_SHARED_BUFFERS  inputs.pg_shared_buffers — used only for the
#                         mongodb "How to read this leg" shared_buffers
#                         comparison below; nothing else in the calling
#                         step (including its own still-inline heredoc)
#                         reads it
# Also reads GITHUB_WORKSPACE (set by Actions on every runner, not just this
# step) and, when present, CAPACITY_NEED_MB / CAPACITY_AVAIL_MB /
# CAPACITY_WAIT_S / CAPACITY_MEM_SOURCE — $GITHUB_ENV values "Docker host
# capacity gate" writes before its own early exit, inherited automatically
# like any other $GITHUB_ENV value (no env: mapping needed for those four).
# CAPACITY_MEM_SOURCE (and host-contention.txt's host_mem_source below) name
# which of host-mem.sh's sources (docker-stats / none) that reading actually
# came from — see host-mem.sh's header for why it derives MemAvailable from
# `docker info` + `docker stats` instead of ever reading /proc/meminfo
# (run 36410157709).
#
# Input: the *.txt files "Run benchmark suites" wrote under
# bench-results/<backend>/ — runner-info.txt, import-completeness.txt,
# es-drain.txt, host-contention.txt, search-counts.txt, crud-residue.txt,
# import-mongo-txn-errors.txt, es-sync-metrics-after-drain.txt,
# containers-state.txt. Each section below is skipped, not fatal, when its
# file is absent (e.g. a leg that died before that file was ever written).
# Output: stdout — Markdown, appended to $GITHUB_STEP_SUMMARY by the caller.
import os
import re

results_dir = os.path.join(os.environ["GITHUB_WORKSPACE"], "bench-results", os.environ["BACKEND"])
backend = os.environ["BACKEND"]

# ── #1475 leg detail ────────────────────────────────────────────────
# Everything below reads files "Run benchmark suites" already wrote to
# results_dir, plus (for the Capacity gate row alone, via the ri
# fallback just below) a couple of $GITHUB_ENV values "Docker host
# capacity gate" writes even on its own early exit. So this prints
# even when a leg produced no *.json summary at all (capacity gate
# failure, a backend container that died before any suite ran) —
# exactly when a reader most needs the context.

def read_kv(path, sep):
    kv = {}
    if os.path.exists(path):
        for line in open(path):
            if sep in line:
                k, v = line.strip().split(sep, 1)
                kv[k.strip()] = v.strip()
    return kv

def import_rate(ic, bundles_ok, entries):
    """The Import line's throughput clause, or "" without the scenario clock.

    Resources/s is the import's unit, as in upstream's report and the "Compare
    legs" matrix: entries of committed Bundles over the import scenario's own
    seconds (scenario_seconds, k6 setup() excluded; suite-lib.sh
    write_import_completeness), not k6's http_reqs.rate, which counts a corpus
    GET beside every Bundle POST.
    """
    try:
        scenario = float(ic["scenario_seconds"])
    except (KeyError, ValueError):
        return ""
    if not 0 < scenario < float("inf"):
        return ""
    text = (f" — **{entries / scenario:,.0f} resources/s** ({bundles_ok / scenario:.2f} Bundles/s) "
            f"over the {scenario:,.0f} s import scenario")
    try:
        text += f", after {float(ic['setup_seconds']):,.0f} s of k6 setup() (the two seed Bundles)"
    except (KeyError, ValueError):
        pass
    return text

# Tuning table: heap, sync mode, WT cache, Mongo pool, max_parallel,
# leg timeout and the capacity gate's own numbers — all already
# recorded in runner-info.txt by "Run benchmark suites".
ri = read_kv(f"{results_dir}/runner-info.txt", ":")
if not ri:
    # A capacity-gate failure exits before "Run benchmark suites"
    # ever runs, so runner-info.txt is never written. Fall back to
    # the $GITHUB_ENV values "Docker host capacity gate" wrote
    # before its own early exit, so this leg still gets a Capacity
    # gate row instead of an empty summary.
    cap_need = os.environ.get("CAPACITY_NEED_MB")
    if cap_need is not None:
        ri = {
            "capacity_need_mb": cap_need,
            "capacity_avail_mb": os.environ.get("CAPACITY_AVAIL_MB", "unknown"),
            "capacity_wait_s": os.environ.get("CAPACITY_WAIT_S", "unknown"),
            "capacity_mem_source": os.environ.get("CAPACITY_MEM_SOURCE", "unknown"),
        }
if ri:
    print("### Leg configuration\n")
    print("| | |")
    print("|---|---|")
    print(f"| **Backend** | `{backend}` |")
    if ri.get("es_heap", "n/a") != "n/a":
        print(f"| **ES heap** | `{ri.get('es_heap', '?')}` |")
        print(f"| **ES sync mode** | `{ri.get('es_sync_mode', '?')}` |")
    if ri.get("mongo_wt_cache_gb", "n/a") != "n/a":
        print(f"| **Mongo WT cache** | `{ri.get('mongo_wt_cache_gb', '?')}` GB |")
        print(f"| **Mongo pool max** | `{ri.get('mongo_pool_max', '?')}` |")
    print(f"| **Legs at once (max_parallel)** | {ri.get('max_parallel', '?')} |")
    print(f"| **Leg timeout** | {ri.get('leg_timeout_min', '?')} min |")
    need = ri.get("capacity_need_mb")
    if need is not None:
        print(f"| **Capacity gate** | need {need} MB, available "
              f"{ri.get('capacity_avail_mb', '?')} MB "
              f"(source `{ri.get('capacity_mem_source', 'unknown')}`), "
              f"waited {ri.get('capacity_wait_s', '?')} s |")
    print()

# Import completeness (F1b): k6 caps import at 60m, so an async
# *-elasticsearch leg is expected to hit that cap — flag a leg whose
# crud/search ran on fewer than 1000 imported bundles.
ic = read_kv(f"{results_dir}/import-completeness.txt", "=")
if ic:
    try:
        imp_ok = int(float(ic.get("bundles_ok", "0")))
    except ValueError:
        imp_ok = 0
    try:
        imp_entries = int(float(ic.get("entries", "0")))
    except ValueError:
        imp_entries = 0
    imp_warn = "" if imp_ok >= 1000 else (
        " ⚠ **import incomplete — crud/search on this leg ran on a smaller corpus "
        "and are not comparable to a 1000/1000 leg** (and abandoned import requests "
        "may still have been running server-side)")
    print(f"\n**Import:** {imp_ok:,}/1000 bundles, {imp_entries:,} entries in "
          f"{ic.get('wall_seconds', '?')} s{import_rate(ic, imp_ok, imp_entries)}.{imp_warn}")

# ES drain (F2/F4): the barrier + settle + primary-vs-ES-count gate
# "Run benchmark suites" ran before the search suite.
drain = read_kv(f"{results_dir}/es-drain.txt", "=")
if drain:
    d_status = drain.get("status", "?")
    d_missing = drain.get("missing", "unknown")
    try:
        d_needs_reindex = float(drain.get("needs_reindex_after", "0"))
    except ValueError:
        d_needs_reindex = 0.0
    d_warn = " ⚠" if (d_status != "drained" or d_missing not in ("0", "unknown")
                       or d_needs_reindex > 0) else ""
    print(f"\n**ES drain:** status=`{d_status}` (barrier HTTP {drain.get('barrier_http', '?')} "
          f"after {drain.get('barrier_seconds', '?')} s); primary_live={drain.get('primary_live', '?')} "
          f"es_live={drain.get('es_live', '?')} missing={d_missing} "
          f"needs_reindex={drain.get('needs_reindex_after', '?')}.{d_warn}")

# Host load (F9), every leg — not just Postgres ones (see the
# Environment paragraph further below, which stays Postgres-only).
contention_path = f"{results_dir}/host-contention.txt"
if os.path.exists(contention_path):
    hc = re.search(
        r"suite=crud phase=start host_loadavg=(\S+).*host_containers=(\d+).*"
        r"host_mem_avail_mb=(\S+).*host_mem_source=(\S+)",
        open(contention_path).read())
    if hc:
        print(f"\n**Host load at crud start:** loadavg {hc.group(1)}, {hc.group(2)} containers, "
              f"{hc.group(3)} MB MemAvailable on the Docker host (source `{hc.group(4)}`).")

# "How to read this leg" (F4): what the ES/Mongo numbers actually
# measure, so they are not misread as directly comparable to a bare
# sqlite/postgres leg.
if backend.endswith("-elasticsearch"):
    if ri.get("es_sync_mode") == "synchronous":
        print("\n**How to read this leg.** Every search is answered by Elasticsearch; the primary's "
              "own search index is not written. In `synchronous` mode a write returns only after "
              "Elasticsearch has indexed it — bundles go through a per-resource-type `_bulk` request "
              "with `refresh=wait_for` against a 200ms `refresh_interval`, so **import/crud latency "
              "on this leg includes Elasticsearch indexing plus that refresh wait**, not just the "
              "primary. Every update and delete also runs a `_delete_by_query` with a forced refresh "
              "across all of the tenant's indices. Cluster health `yellow` is expected (1 replica per "
              "index, 1 node). Compare search latency with another leg only if both imported the same "
              "entry count and the ES drain above says `drained`.")
    else:
        print("\n**How to read this leg.** Every search is answered by Elasticsearch; the primary's "
              "own search index is not written. Writes commit on the primary first. In `asynchronous` "
              "mode ONE background worker then forwards each resource to Elasticsearch individually "
              "(an index-exists check plus an index request per resource) through a 1,000-event queue "
              "that blocks writers when full, so **import and crud throughput on this leg is that "
              "worker's rate, not the primary's**. Every update and delete also runs a "
              "`_delete_by_query` with a forced refresh across all of the tenant's indices. Cluster "
              "health `yellow` is expected (1 replica per index, 1 node). Compare search latency with "
              "another leg only if both imported the same entry count and the ES drain above says "
              "`drained`. **This worker's rate is expected to make import hit k6's 60-minute cap "
              "before the corpus finishes, leaving the index incomplete** (see the Import and ES "
              "drain lines above / `es-drain.txt`; run 36412022609 imported 313/1000 bundles and "
              "the drain gate reported status=incomplete with 194,664 resources never indexed).")
if backend.startswith("mongodb"):
    pg_shared_buffers = os.environ.get("IN_PG_SHARED_BUFFERS") or "2GB"
    print(f"\n**How to read this leg.** MongoDB 7.0 single-member replica set (directConnection); "
          f"WiredTiger cache **{ri.get('mongo_wt_cache_gb', '?')} GB** (Postgres: shared_buffers "
          f"{pg_shared_buffers}; both also use the host page cache). Each FHIR "
          "transaction bundle is one multi-document transaction (lifetime "
          f"{ri.get('mongo_txn_lifetime_s', '900')} s); an aborted transaction fails that bundle's "
          "`Bundle import` check — see Checks ✗ on the import row (in Results, below) and "
          "`txn.aborted` in `import-mongostat.txt`.")

# Result-size cross-check (F5): a composite search that silently
# degrades to matching on the parameter name alone (or a Mongo sa/eb
# mistranslation) changes the result-set size, and nothing else
# would show it.
sc_path = f"{results_dir}/search-counts.txt"
if os.path.exists(sc_path):
    sc_lines = [l.rstrip("\n") for l in open(sc_path) if l.strip()]
    sc_rows = [l.split("|") for l in sc_lines[1:] if l.count("|") == 3]
    if sc_rows:
        print("\n### Result-size cross-check\n")
        print("| Query | Total | HTTP | Seconds |")
        print("|---|---:|---:|---:|")
        for q, total, http, secs in sc_rows:
            print(f"| `{q}` | {total} | {http} | {secs} |")

        # Comparability caption: crud.js creates 9 resources per
        # iteration and deletes them in the same one; prewarm.js
        # runs the identical script at low concurrency first. A
        # failed delete check on EITHER leaves a live row this
        # table's totals cannot tell apart from import.
        # crud-residue.txt (written by "Run benchmark suites" from
        # the upstream k6/crud.js check names) is that count, per
        # resource type, created minus deleted.
        cr = read_kv(f"{results_dir}/crud-residue.txt", "=")
        try:
            residue_total = sum(int(v) for v in cr.values())
        except ValueError:
            residue_total = 0
        patient_residue = cr.get("Patient", "0")
        print(f"\n_totals include {residue_total} crud/prewarm leftovers "
              f"(Patient: {patient_residue}) and any import shortfall; a composite "
              "total equal to the Observation total means the name-only fallback; "
              "seconds after a 000 row overlap the previous query (HFS keeps "
              "executing after curl's --max-time)._")

# Mongo transaction-abort count (F8): this counts matching HFS log
# LINES across the whole log (startup, prewarm and import), not
# bundles — a heuristic, not an exact bundle-failure count (a
# WriteConflict retried into a VersionConflict, for instance, won't
# match this grep). The exact per-bundle failure count is the
# import row's Checks ✗ above.
mte_path = f"{results_dir}/import-mongo-txn-errors.txt"
if os.path.exists(mte_path):
    try:
        mte_n = int(open(mte_path).read().strip() or "0")
    except ValueError:
        mte_n = 0
    if mte_n > 0:
        print(f"\n⚠ **Mongo transaction errors (heuristic):** {mte_n} HFS log line(s) matching "
              "WriteConflict/TransientTransactionError/TransactionTooLargeForCache/NoSuchTransaction/"
              "commit-failed — bundle failures are the import row's Checks ✗ (in Results, below), "
              "not this count.")

# Final ES sync failures (spec §15d): composite_secondary_sync_failures_total
# (observability/composite_metrics.rs SECONDARY_SYNC_FAILURES) is a
# counter that only ever goes up (record_secondary_sync_failure ->
# increment(1)) — a later successful reindex by the sync-repair
# sweep does NOT decrement it. So a nonzero count here is cumulative
# over the whole leg, not necessarily still outstanding at the end;
# what's still outstanding is composite_secondary_sync_needs_reindex
# (the gauge already shown on the ES drain line above), captured
# into es-sync-metrics-after-drain.txt by the drain gate.
esf_path = f"{results_dir}/es-sync-metrics-after-drain.txt"
if os.path.exists(esf_path):
    # One line per {backend,operation} label combination — sum all
    # of them, not just the first match, or a failure recorded
    # under one operation (e.g. "update") would be missed while
    # "create" reads zero.
    esf_vals = re.findall(r"^composite_secondary_sync_failures_total\S*\s+(\S+)",
                           open(esf_path).read(), re.M)
    esf_n = 0.0
    for v in esf_vals:
        try:
            esf_n += float(v)
        except ValueError:
            pass
    if esf_n > 0:
        print(f"\n⚠ **Composite sync:** {esf_n:g} final ES sync failure(s) recorded "
              "(cumulative — includes any the sync-repair sweep later fixed; what is "
              "still outstanding is `needs_reindex` in the ES drain line).")

# Dead-container warning: a backend container that died or was
# OOM-killed mid-suite invalidates this leg's numbers outright. The
# tgz corpus server is excluded — "Run benchmark suites" deliberately
# does not gate the leg on it (only the primary/ES containers do).
# "inspect-failed gone" (Docker's own "no such container" — see
# suite-lib.sh's write_containers_state, which writes this file) is
# unambiguous proof of death; "inspect-failed
# unknown" is a possible daemon hiccup and gets a neutral note
# instead of being folded into the same warning.
cs_path = f"{results_dir}/containers-state.txt"
if os.path.exists(cs_path):
    cs_lines = [l.strip() for l in open(cs_path) if not l.startswith("hfs-bench-tgz-")]
    cs_bad = [l for l in cs_lines
              if re.search(r"OOMKilled=true|Status=(exited|dead)|inspect-failed gone", l)]
    cs_unknown = [l for l in cs_lines if "inspect-failed unknown" in l]
    if cs_bad:
        print("\n⚠ **A backend container died during the run — do not use these numbers:** "
              + "; ".join(f"`{b}`" for b in cs_bad))
    if cs_unknown:
        print("\n_Could not confirm final state for: "
              + "; ".join(f"`{b}`" for b in cs_unknown)
              + " (Docker host hiccup at inspect time — not counted as died)._")
