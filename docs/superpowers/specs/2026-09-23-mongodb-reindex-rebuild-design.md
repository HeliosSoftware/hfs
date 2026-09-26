# MongoDB `$reindex` rebuild: id-order walk, byte-bounded pages, overlapped preparation

Issue: [#1403](https://github.com/HeliosSoftware/hfs/issues/1403) (on standalone MongoDB the post-import search-index rebuild took 17 h 16 m for 18.96 M resources, nearly twice the 9 h 10 m ingest).

Status: approved design, 2026-09-23. The implementation plan covers PR0, PR1, PR2a, PR2b, PR-docs and the bench tooling. PR3 and PR4 each get their own plan only when their trigger fires (and, for PR4, after Step 0 passes), re-checked against main at that time. Section files S1–S5 (§11) hold the file-level detail; "S4 §4.10.3" and the like point into them.

## 1. Summary

The rebuild is slow only while it writes Observation, and the cause is key order, not collection size. The walk pages each type by `(last_updated, id)`, so every `search_index` key ending in the random-UUID `resource_id` lands at a random leaf; Observation's random slice (~20–25 GB) far exceeds the effective WiredTiger cache (~6.1 GB), so each insert waits on page-in reads at queue depth 1.

The fix (Approach A: id walk, byte cap and overlap, with K streams and an index reshape as conditional follow-ups) ships as PRs, each gated on a byte-scaled bench (OBS-26) that must first reproduce run 17: **PR0** timing; **PR1** id-order walk with catch-up rounds; **PR2a** MongoDB honours the page byte cap, default 32 MiB; **PR2b** extraction overlapped with inserts; conditional **PR3** (K streams) and **PR4** (generation-4 indexes). One full-corpus run (T3-full) confirms it.

## 2. Root cause

### 2.1 Measured

Sources: mongod's slow log, an FTDC decode, a read-only corpus survey and a code trace at `c86d0f08b`; the hot path is unchanged since 0.2.3.

| Phase (UTC, 2026-09-19/20) | Resources | Wall | Rate |
|---|---|---|---|
| 07:15–10:26, the 16 types before Observation | 8.72 M | 3 h 11 m | ~760/s |
| 10:26–23:38, Observation | 7.70 M | 13 h 13 m (76.5%) | ~162/s |
| 23:38–00:31, the 7 types after Observation | 2.54 M | 53 m | ~800/s |

- **Per-type working set.** Every `search_index` secondary key leads with `tenant_id, resource_type`, so each type has its own key range; bytes read per row fell 64x at the Observation→Procedure boundary.
- **Key order.** Each Observation touches ~37 random leaves (token 15–16, token_display 13–14, quantity ~3.4, date, uri, composite, resource 1 each); a row enters 4.56 (partial) indexes.
- **Continuous decay.** A 5,000-row `insert_many` went from ~250 ms to a ~1.6 s plateau, ~80% `timeReadingMicros`. Reads per row rose from 1.66 KB (Q1) to 14.72 KB (Q4); Q4/Q1 0.317; write-back 4,775 GB.
- **Serial HFS.** One operation in flight; for resident types HFS-side work is ~60% of wall time.
- **Deletes** cost 4.6% of wall; 99.96% removed nothing, but the first Observation delete removed 2,812 rows.
- **Page memory.** MongoDB ignores `batch_bytes`; one Provenance page is ~1.63 M index documents.
- **Environment.** The VM exhausted its 4 GiB swap at 19:07Z (121.4 GiB of swap I/O); the page cache masked ~14% of early reads.

The id walk cuts random touches per Observation from ~37 to ~7; its residual (~5–7 GB) sits at the effective cache line. Cost model (Observation backtest −14.3%, unmeasured): id walk alone 8.79 h, plus overlap 5.18 h. Judge-adjusted for random-fetch cost and the Observation resident floor: id walk alone ~9.4–11.5 h, plus overlap ≤ ~6.5 h.

### 2.2 Corrections to the issue text

| Issue text | Evidence |
|---|---|
| ~500/s until 13:55, then ~116/s | 13:55 is a status-sample boundary; Observation decays continuously from 10:26. The one event is a 218 s checkpoint stall at 11:52Z |
| every insert paid for eviction | Cache-miss reads: app threads evicted 50 pages in 20 h; tickets peaked at 2 of 16 |
| 12 B-tree indexes per document | 4.56 per Observation row |
| working set exceeds the cache 7:1 | Only the active type's random slice must fit |
| HFS lightly loaded | Serial, one op in flight |
| file concurrency 16 | 1, the default: the ~575/s ingest was sequential |

### 2.3 Ruled out

Pagination and page size (a keyset page reads 1,000 keys in 22 ms); write concern, tickets and locks; a code regression; reordering types (disjoint key ranges); bulk-load (shared multi-tenant collection, no ledger, ≥ 2 h sorted build, model 9.98 h); a larger cache as the fix (model: 31 GB still ~8.3 h, so it becomes a sizing rule); skipping the pre-insert delete (2,812 real rows).

## 3. Goals and non-goals

Goals:

- The full-corpus rebuild takes ≤ 7.5 h on run 17's VM, never longer than run 17's 9 h 10 m ingest (9.17 h, S4 T1); Observation stops decaying (Q4/Q1 ≥ 0.70).
- On a quiescent type, index contents equal HEAD's walk exactly. Under concurrent writes, stale or orphan rows occur only within §4.2's documented residuals and assumptions A1/A2.
- Automatic rebuilds are bounded by bytes under the 32 MiB server default. Manual `$reindex`, and runs with the variable set to 0, stay count-bounded (at most two pages with prefetch on).
- Numbers come from the shipping build; operators get a measured sizing rule.

Non-goals: a persisted ledger or resume after restart (#1283); the Elasticsearch composite rebuild (#1250), which keeps one serial walk; indexing during ingest (#1159, #1242); steady-state search beyond what PR4 incidentally improves.

## 4. Design by PR

### 4.1 PR0: instrumentation, no behaviour change

Always-on `Instant` timing inside `run_reindex` (~20 clock reads per page) is the measurement of record; the `cfg(perf_phases)` spans stay. New defaulted `ReindexTarget` method `write_search_entries_page_timed(&self, tenant, resources, stats: &mut ReindexPageStats) -> Vec<StorageResult<usize>>` (default: calls `write_search_entries_page`, stats untouched). `ReindexPageStats` (`#[non_exhaustive]`, `Copy + Default`) holds `extract`, `delete`, `insert`, `deleted_entries`, `inserted_entries`, `insert_commands`. MongoDB overrides `_timed` and makes `write_search_entries_page` delegate to it, as any override must. Other backends report zero writer phases.

**Log contract** (target `helios_persistence::search::reindex`, every backend). Fixed messages; fields in the order below; strings unquoted, never empty (`-` when no type is open); `*_ms` truncated whole ms; `*_per_s` one decimal. Fields may be appended, never renamed or removed; meanings never change except PR2b's sanctioned change (§4.4). Soft budget: 32 fields per line, including `message` (L4 is 29 in PR0 and 31 after PR2b; L3 is 31 after PR3). `P` below stands for `entries, failed, pages, fetch_ms, write_ms, extract_ms, delete_ms, insert_ms, writer_other_ms, yield_ms, other_ms, deleted, inserted, insert_commands`.

| Line | When | Fields, in order |
|---|---|---|
| L1 `reindex job started` | INFO, after counting | `tenant, job_id, types, total, batch_size, batch_bytes, bulk_index_rebuild, clear_existing, resource_scoped, writers, setup_ms` |
| L2 `reindex type started` | INFO | `tenant, job_id, resource_type, type_index, types, type_total, elapsed_ms` |
| L3 `reindex type finished` | INFO, once per L2 on every return path | `tenant, job_id, resource_type, outcome, type_index, type_resources, type_total, type_elapsed_ms, type_resources_per_s, elapsed_ms,` P |
| L4 `reindex progress` | INFO, first page boundary ≥ 60 s after the last | `tenant, job_id, resource_type, type_index, type_resources, type_total, type_elapsed_ms, type_resources_per_s, processed, total, elapsed_ms, interval_ms, interval_resources, interval_resources_per_s,` P |
| L5 `reindex job finished` | INFO, once per L1, before the terminal status write | `tenant, job_id, outcome, types_done, types, processed, total, elapsed_ms, resources_per_s,` P |
| L6 `reindex page` | DEBUG, every page | `tenant, job_id, resource_type, page, resources, type_elapsed_ms, entries, failed, fetch_ms, write_ms, extract_ms, delete_ms, insert_ms, deleted, inserted, insert_commands` |

Scopes: L3/L4 counters cover the open type, L5's the job, L6's one page. `elapsed_ms` is the job clock, `type_elapsed_ms` the type clock. `other_ms` = `type_elapsed − (fetch + write + yield)` on L3 and L4, and `elapsed − (fetch + write + yield)` on L5; from PR2b, `fetch_wait` replaces `fetch` in both. An unclosed L1 or L2 (panic, process death) is a truncated run.

Arms set `RUST_LOG=info,hfs=warn,hfs_perf=info,helios_persistence::search::reindex=debug`. P2a-0 and P2a-cap append `,helios_persistence::backends::mongodb::storage=debug`, so G2a.6 can read the capped-page line; every other arm and T3-full use the value above.

### 4.2 PR1: id-order walk with catch-up rounds

PR1 changes only MongoDB's reindex source (`fetch_resources_page`), plus one config field, `MongoBackendConfig::reindex_catch_up_margin_ms` (and the one exhaustive literal in `crates/hfs/src/main.rs`), two index-name constants in `schema.rs`, a doc-comment precondition and a docs section. No trait, driver, index key, generation or persisted-state change. Two phases partition each type's live resources by `last_updated`:

1. **Id phase:** `last_updated < floor`, keyset on `id`, hinted to `idx_resources_identity` (mandatory: otherwise the planner can pick `idx_resources_type_scan` plus a blocking sort, #1021).
2. **Catch-up rounds 1–3:** `[round floor, ceiling)`, keyset on `(last_updated, id)`, hinted to `idx_resources_type_scan`.

- `floor = min(newest_live + 1 ms, t0 − margin)`, where `t0` is the HFS clock at the type's first call and `newest_live` comes from a covered probe.
- Each round's `ceiling = max(now + margin, newest_live + 1 ms)` from a fresh probe (WARN when the second term wins).
- Round 1 always runs; round k ≥ 2 only if the previous round took ≥ `margin/2`; at most 3 (WARN at the cap).
- `margin` defaults to 120,000 ms (60 s skew + 60 s commit lag), clamped to 1 s–24 h.

**Invariants.**

- A phase ends only on an empty query and the next starts in the same call: every non-empty page gets a successor query after its write (that query heals races), and the driver sees one trailing empty page per walk.
- A type nobody writes to is written exactly once: `processed == total`.
- Given A1 (writer clocks agree within `margin/2`) and A2 (a write is visible within `margin/2` of its stamp), a resource updated while its id-phase page is written is re-read by round 1 from its newest version.
- Ids are unique within a page (round pages are de-duplicated keeping the newest). The pre-insert delete stays.

Documented residuals: in rounds, a racing write stamped at or below the page cursor is not revisited (HEAD's limit); a write during a round-3 page can stay stale (cap WARN); a delete racing a page leaves orphans that search never returns.

**Cursor grammar** (backend-private, in memory only):

```
v2|i|<floor>|<after_id>                                          Id
v2|c|<round>|<floor>|<ceiling>|<walked>|<after_lu>|<after_id>    Round, round in 1..=3
```

Instants are RFC 3339 UTC with milliseconds; the id is the last field. Anything else, HEAD's format included, is `SearchError::InvalidCursor` and fails the run.

**Walk lines** (target `helios_persistence::backends::mongodb::storage`; fields start `tenant, resource_type`; a parser attributes one to a type only between its L2 and L3 in a job with `resource_scoped=false`), messages prefixed `mongodb reindex`: INFO `walk started` (`t0, newest_live, floor`); INFO `id phase finished` (`floor`); INFO `catch-up round started` (`round, floor, ceiling`); INFO `catch-up round finished` (`round, floor, ceiling, walked`); WARN `found live resources stamped in the future` (`round, newest_live, ceiling`); DEBUG `catch-up complete` (`rounds`); WARN `catch-up stopped at its round limit` (`rounds, last_ceiling`).

### 4.3 PR2a: honour `batch_bytes` on MongoDB

MongoDB overrides `fetch_resources_page_capped` with PostgreSQL's strict rule: a page never exceeds `max_bytes` unless it holds one resource, measured as each row's raw BSON length. A capped page is non-empty, so it continues its phase from the last row taken; `max_bytes == 0` is PR1's page. DEBUG `mongodb reindex capped page read` (`tenant, resource_type, rows, bytes, capped`). The REST handler saturates `batchSize` to `u32::MAX` instead of truncating it modulo 2³² (today 4294967296 becomes 0); the driver uses `batch_size.max(1)`.

**Default:** `HFS_REINDEX_BATCH_BYTES` = `33554432` (32 MiB) in the server; `ReindexRequest` and `AutomaticRunOptions` stay 0, so manual `$reindex` is unchanged. The default takes effect on the SQLite (overshoot by one) and PostgreSQL (strict) sources as well as MongoDB; Elasticsearch and S3 page by count only. Checks: the existing SQLite and PostgreSQL capped-fetch tests, plus a `crates/hfs` test that the automatic hook receives 33554432 when the variable is unset. In the corpus only Provenance (~108 KB per resource) gets smaller pages. Six stale doc descriptions are rewritten. PR2a closes issue A.

### 4.4 PR2b: overlap and parallelise page preparation

- **Sub-batch pipeline** (new `mongodb/reindex_pipeline.rs`): while sub-batch k (~2,500 index documents) inserts on a spawned task, the page's thread extracts k+1 in `block_in_place`; the delete runs during the first extraction. Pages of ≤ 32 resources and current-thread runtimes stay serial.
- **Parallel extraction** on a per-backend rayon pool of `available_parallelism − 1`, clamped to 1–4 (the #1199 pattern).
- **Id-phase prefetch** through two defaulted `ReindexSource` methods: `may_prefetch_page(&self, cursor: &str) -> bool` (default `false`; MongoDB: `true` only for an `Id` continuation with prefetch on and search not offloaded) and `fetch_resources_page_ahead(&self, tenant, resource_type, cursor, limit: u32, max_bytes: u64) -> StorageResult<Option<ResourcePage>>` (default: the capped fetch in `Some`). MongoDB runs only the id query and returns `Ok(None)` when it is empty; the driver then re-fetches serially after the write, so phase ends still follow the phase's last write.

**Invariant I1:** at most one page's delete/insert in flight per walk (stream), in fetch order, so PR1's revisits stay safe. Rounds are never prefetched. Memory per walk: at most two pages of resources and two sub-batches of documents.

| Knob | Default | Meaning |
|---|---|---|
| `HFS_MONGODB_REINDEX_OVERLAP` | `true` | `false` is the serial writer |
| `HFS_MONGODB_REINDEX_PREPARE_THREADS` | `0` | `0` = cores − 1 (1–4); `1` = no pool |
| `HFS_MONGODB_REINDEX_PREFETCH` | `true` | id-phase prefetch; off when search is offloaded |

Invalid values fail startup. INFO `mongodb reindex writer configuration` (target `helios_persistence::backends::mongodb::reindex_pipeline`; `overlap, prefetch, prepare_threads_configured, prepare_threads, pool, multi_thread_runtime, path`) logs once. New perf phases `ReindexDbWait` (nested under `ReindexPage`) and `ReindexFetchWait`; `PHASE_COUNT` 38.

**Contract additions.** `ReindexPageStats` gains `db_wait: Option<Duration>` (read through `db_wait_or_busy()`, which falls back to `delete + insert`), `sub_batches`, `pool_sub_batches`. L3, L5 and L6 append `fetch_wait_ms, db_wait_ms, sub_batches, pool_sub_batches`; L4 appends `fetch_wait_ms, db_wait_ms`. Sanctioned change: `fetch_wait` replaces `fetch` in both `other_ms` formulas (§4.1), and `writer_other_ms = write − (extract + db_wait_or_busy)`; both equal PR0's when nothing is prefetched and `db_wait` is unset.

### 4.5 PR3 (conditional): K insert streams on disjoint id ranges

**Trigger:** in B2-s, Observation's mongod write-busy fraction ≥ 0.85, **or** the half-cache probe's Q4/Q1 < 0.70 (§5.5). Otherwise the numbers go on #1403.

- Defaulted `ReindexSource::plan_type_walk` returns `Single` or `Ranges { ranges, catch_up }`. MongoDB, standalone only: `k = min(streams, (max_connections − 2) / (2 × concurrent_runs), n / 50,000)`; boundaries from covered `skip` probes affect balance, never correctness.
- New `v2` tags: `v2|r|<floor>|<lo>|<hi>|<after_id>` (`IdRange`) and `v2|d|<floor>` (`IdPhaseDone`).
- Correctness rests on: streams own disjoint contiguous id ranges (each keeps its own append points); I1 and the trailing empty page hold per stream; the catch-up runs once, from `IdPhaseDone`, only after every stream has drained. Memory is at most K × (one written + one prefetched page), each capped at 32 MiB.
- `HFS_REINDEX_WRITE_STREAMS`, default 1, clamped 1–16; manual `$reindex` always uses 1. L1 appends `write_streams`, L3 `streams, plan_ms`; new INFO lines `mongodb reindex streams planned` and `mongodb reindex id range finished` (fields in S5 §1).

### 4.6 PR4 (conditional): generation-4 index reshape

**Trigger:** the half-cache probe's Q4/Q1 < 0.70, **or** slow Observation-scoped reads after a rebuild reproduced through HFS searches (issue B).

**Step 0, before code:** `$indexStats` and explains on `hfs-mongo`, then hide-and-re-explain on a kept bench database. Go only if plans on `idx_search_resource` move to `idx_search_composite` at ≤ 1.1x the keys and only `:text`/`:code-text` use the display index.

**Catalog:** generation 3 → 4. `idx_search_token_v3` (`tenant_id, resource_type, param_name, value_token_code, value_token_system, value_token_display, resource_id`, partial on `value_token_code`) replaces `idx_search_token_v2` and `idx_search_token_display_v2`; `idx_search_resource`, a strict prefix of the composite, is removed. `:text` and `:code-text` gain `value_token_code: {$ne: null}`. Upgrade all instances together; rollback needs `search-index-gen3-rollback.mongosh.js` first.

### 4.7 Sequence

| PR | Depends on | Gate |
|---|---|---|
| PR0 | — | SMOKE-1; Gate 0 on its merge |
| PR1 | PR0 | Gate PR1 + CU-1 |
| PR2a | PR1 (same fetch function) | Gate PR2a |
| PR2b | PR2a | Gate PR2b + CU-2b |
| PR3 | PR2b and its trigger | Gate PR3 + CU-3 |
| PR4 | PR2b (and PR3 if it merges), its trigger, Step 0 | Gate PR4 + upgrade arm |
| PR-docs | — | none; any time after SMOKE-1 |

Merge order: PR0 → PR1 → PR2a → PR2b → [PR3] → [PR4]. B1-s-1G does not block PR1's merge but finishes before the PR2b verdict. Each branch is rebased on `origin/main` before its arm; nothing merges before its gate passes or its escalation is resolved.

## 5. Measurement

### 5.1 OBS-26 setup

- **Input:** a real `$bulk-submit` of `Observation.part00`–`04` (byte-exact, the first 26% of run 17's Observation order) and `Procedure.part00` as control: 2.4 M resources.
- **Cache scaled in bytes:** run 17's 7,671,382,016 B × 2.0 / 7.70 → 1,900 MiB (`--wiredTigerCacheSizeGB 1.85546875`), keeping run 17's slice/cache ratios. Memory `M = ceil((C + max(1536, C)) / 64) × 64` MiB (3,840), `--memory-swap` equal, so no swap.
- **Container and builds:** hfs-mongo's image by ID (mongo 7.0.40), fresh volumes, slowms 0 during the rebuild only; plain release builds (`R4,sqlite,mongodb`, never `RUSTFLAGS` or `perf_phases`); one arm on the box at a time.
- **Batch bytes:** every bench arm sets `HFS_REINDEX_BATCH_BYTES` explicitly: 0, except 33554432 in P2a-cap. T3-full inherits the shipped default.
- **Metrics:** S1's lines, FTDC, per-index `collStats`, the slow log, host samples; quartiles cut by cumulative inserts in the walk block (L2 → `id phase finished`).
- **Correctness:** census and sample-document hashes, `processed == total`, equal `entriesCreated`, `errorCount 0`. CU arms add 500 PUTs during the Observation walk, a second submission and a manual `$reindex`, then verify by search.
- **Validity** (else INVALID): no OOM, swap, page-cache masking or checkpoint > 120 s; quiet host; clean run; full contract; configured cache and knobs. From PR1 on, in write-free obs26-profile arms (B1-s, B1-s-1G, B2-off, B2-s, the diagnostic pair, K and G4 arms): floor = `newest_live + 1 ms` and round 1 walks 0 (V12). In CU arms, round 1 is expected to walk the updated resources.

Arms, in order: SMOKE-0/1; **B0-s** (main + PR0); **B1-s**, CU-1, half-cache B1-s-1G (+PR1); **P2a-0/P2a-cap** (+PR2a, Provenance, 4,096 MiB); **B2-off/B2-s**, CU-2b (+PR2b, knobs off/default); conditional K, CU-3, P3-prov (PR3) and G4 arms (PR4); T3-full. Knob pairs share a binary.

### 5.2 Gate 0 (B0-s must reproduce run 17)

| Metric | Band | Run 17 |
|---|---|---|
| Obs Q4 KB read / inserted row | 7.5–25 | 14.72 |
| Obs Q4 miss ratio | 3.4–13.6% | 6.82% |
| Obs Q4/Q1 rate | ≤ 0.45 | 0.317 |
| Observation walk ms/resource | 5.0–9.3 | 6.16 |
| Obs Q4 iowait | ≥ 0.6 cores | 1.20 |
| Procedure walk ms/resource | ≤ 1.3 | ≈0.72 |
| Obs first-decile ms/resource (`R`) | ≤ 3.4 | first hour 2.47 |

Selected rows (full table: S4 §4.10.3). Too resident: retry at 0.75x cache; too miss-bound: 1.25x; two recalibrations, then escalate. The 5.0 ms floor stops a faster swap-free baseline from failing a correct PR1.

### 5.3 Gate PR1 (B1-s vs B0-s; S4 §4.10.4)

| Id | Metric | Threshold | Kind |
|---|---|---|---|
| G1.1 | Observation walk ms/resource | ≤ max(0.50 × B0-s, R + 0.8) and ≤ 3.4 | GATE |
| G1.2 | Obs Q4 `search_index` KB read into cache / row | ≤ 0.25 × B0-s | GATE |
| G1.3 | Obs Q4 `search_index` miss ratio | ≤ 1.5% | GATE |
| G1.4 | Obs Q4 KB read / row, global | ≤ 3.0 | GATE |
| G1.5 | Obs Q4/Q1 | ≥ 0.70 | GATE |
| G1.6, G1.10 | Correctness; CU-1 | exact; all checks | GATE |
| G1.11 | no `plan_unexpected` flag | — | GATE |
| G1.7 | Procedure walk ms/resource | ≤ 1.15 × B0-s | GUARDRAIL |
| G1.8, G1.9 | Obs fetch share; p95 fetch server ms per page | ≤ 20%; ≤ 400 ms | GUARDRAIL |

Locality but fetch-bound escalates (prefetch is the remedy); no locality and not fetch-bound is REJECT, confirmed by the orchestrator. Expected: B1-s ≈ 2.7–3.1 ms, B0-s ≈ 6.2.

### 5.4 Gates PR2a and PR2b (S4 §4.10.7–8)

| Id | Metric | Threshold | Kind |
|---|---|---|---|
| G2a.1–3 | Provenance memory growth; pages; max `inserted` per page | ≤ 0.5x; ≥ 2x; ≤ 0.5x uncapped | GATE |
| G2a.5 | Correctness: hashes, equal `entriesCreated`, `errorCount 0` | exact | GATE |
| G2a.6 | Every capped page | `bytes ≤ 33554432` or `rows=1` | GATE |
| G2a.4 | Provenance wall | ≤ 1.10 × uncapped | GUARDRAIL |
| G2b.1–2 | Procedure and Observation speedup over B2-off | ≥ 0.8 × predicted | GATE |
| G2b.3–4 | Obs Q4 `search_index` KB read / row; Q4/Q1 | ≤ 1.2 × B2-off; ≥ 0.70 | GATE |
| G2b.5, G2b.9 | Correctness; CU-2b | exact | GATE |
| G2b.6–7 | HFS peak memory; foreground p99 | ≤ +1.5 GiB; ≤ 1.5 × B2-off | GUARDRAIL |
| G2b.8 | B2-off vs B1-s walk ms/resource | Observation ±10%; Procedure ±15% | GUARDRAIL |

Predicted speedup per type from B2-off's L6 pages: `Σ(F+E+D+I+W+Y) / Σ(Fa + max(E/P, D+I) + W + Y)`, `Fa = 0` for prefetched pages; Procedure's ~40:60 HFS:MongoDB split caps it near 0.6x. Observation's is REPORT while its Q4 is ≥ 85% delete + insert. If G2b.1, G2b.2 or G2b.3 fails, or G2b.7 flags, the diagnostic pair (B2-s-P1-nopf, B2-s-P1) runs (REPORT).

### 5.5 Scale probe and conditional gates

**B1-s-1G** (half cache, ≈15 M Observations on run 17's cache): Q4/Q1 ≥ 0.70, PR4 only on search grounds; 0.40–0.70, K and PR4 arms at half cache; < 0.40, PR3 and PR4 both required before any customer-scale claim. It fixes the sizing rule `wiredTigerCacheSizeGB ≥ (k × N_M + 1) / 0.8` (`N_M` = millions of resources in the largest type): at a 1,900 MiB bench cache, k = 0.742 GiB/M if only B1-s passes, 0.371 if the probe passes too.

- **PR3** (K4 vs K1, `HFS_MONGODB_MAX_CONNECTIONS=18`; S4 §4.10.9). GATE: Observation ≤ 0.75 × K1 (G3.1), write tickets out ≥ 1.8 (G3.2), cross-connection overlap > 0.50 (G3.3), exact correctness (G3.4), CU-3 (G3.10), Provenance peak ≤ 3 GiB (G3.14). GUARDRAIL, selected: app-thread eviction ≤ 0.1%, checkpoint ≤ 60 s, bytes per row ≤ 1.25 × K1. An HFS-bound miss escalates.
- **PR4** (vs parent at the same cache; S4 §4.10.10). GATE: ≤ 3.1 keys per row, today 4.56 (G4.1); bytes written per row ≤ 0.75x (G4.2); `totalIndexSize` ≤ 0.85x (G4.3); half-cache Q4/Q1 ≥ parent + 0.10 if the probe triggered PR4 (G4.4); census (G4.5); identical query results (G4.6), also after the upgrade build (GU.3); if PR3 merged, write-busy ≤ 0.85 × PR3's (G4.8). GUARDRAIL: query p50 ≤ 1.2x; upgrade build ≤ 3 h per 100 M rows.

**Re-runs:** a GATE metric within ±10% of its threshold gets one candidate re-run; a flipped verdict re-runs the baseline and judges on means. The budget is in §7.

### 5.6 T3-full

Once, on the final main SHA and a fresh `hfs-mongo`, like-for-like with run 17 in VM, image, cache and manifest; knobs at shipped defaults plus the sizing doc's settings. Preconditions: #940 posted, PR4 Step 0 done if triggered, the user's answer on resetting hfs-mongo (§7), vhdx compacted. Binary: the T0 feature list (`R4,R4B,R5,R6,sqlite,postgres,mongodb,elasticsearch,s3,ui,subscriptions,cloudwatch,otel`), release, no `RUSTFLAGS`.

| Check | Threshold | Kind | Run 17 |
|---|---|---|---|
| Rebuild wall | PASS ≤ 7.5 h; PARTIAL 7.5–9.17 h (orchestrator decides PR3/PR4); FAIL > 9.17 h | GATE | 17.27 h |
| Obs Q4/Q1 | ≥ 0.70 | GUARDRAIL | 0.317 |
| Correctness | `errorCount 0`; `processed == total`; FTDC inserts within 1% of `entriesCreated`; Observation and Procedure entries per resource within 0.05% of the last bench arm whose source passes the baseline path rule against T3's SHA (REPORT if none) | GATE | — |
| Obs Q4 KB read / row; bytes written | ≤ 3.0; ≤ 2,000 GB | REPORT | 14.72; 4,775 GB |
| Ingest wall; swap I/O | 9 h 10 m ± 10%; < 30 GiB flags an environmental share | REPORT | 9 h 10 m; 121.4 GiB |

### 5.7 Tooling and budget

Tools: `manual-test/tools/bench-1403/` (Python 3.12, outside the repo, written by Sonnet agents); outputs in `manual-test/bench-1403/`. `compare_arms.py` holds every threshold as data and exits PASS/FAIL/RERUN/ESCALATE/REJECT. No cargo runs while `ARM_RUNNING.lock` exists. The committed record is `docs/mongodb-reindex-benchmark.md`; PR bodies paste their gate tables.

Machine time: ≈24 h of mandatory arms, 3–5 h reserve, ≤ ≈41 h conditional, ≈18 h for T3-full.

## 6. Docs and issue hygiene

- **PR-docs** (docs only): the benchmark doc; new `docs/mongodb/bulk-import-sizing.md` (per-type working set, effective cache at 80%, the rule with k ≈ 3 GiB/M for 0.2.x, container memory, `.wslconfig` with `swap=0`, no resume after restart); pointers from `search-indexes.md` and the READMEs; cache and memory flags in `MANUAL_TESTING_MATRIX.md` and the test-hfs SKILL; a corrected bulk-data-submit SKILL paragraph on the decay. The knobs section lands with PR2a/PR3, and "watching a rebuild" names only log lines already on main.
- **With each PR:** its own doc text (PR1's walk section in `search-indexes.md`, knob rows, PR4's upgrade and rollback). A provisional k is never merged; `.claude` SKILL edits are mirrored in `.agents`.
- **#1403:** a correcting comment now, shown to the user first (findings, §2.2's corrections, a verdict per suggested approach, the plan, the bench), using §2.1's figures (Q1 1.66 → Q4 14.72 KB read per row; Q4 iowait 1.20 cores). OBS-26 results follow as comments.
- **Filed now:** issue A, "MongoDB reindex source ignores HFS_REINDEX_BATCH_BYTES" (PR2a closes it), and a follow-up for a MongoDB `fetch_resources_by_ids` `$in` override (twin of PostgreSQL #1460).
- **Held:** issue B (slow Observation reads after a rebuild) until reproduced through HFS searches. No archive upload; no comment on #1283.

## 7. Decisions log

Terms: T4 is the user's manual UI pass; `hfs-mongo` is the long-lived MongoDB container that holds the T4 corpus.

User, 2026-09-23:

- Approach A (§1) and all five design sections approved; the 32 MiB server default (§4.3); the #1403 comment and issue filing in §6.
- T4 is finished; the orchestrator may stop `hfs-mongo` and the running HFS whenever the campaign needs; no overnight window.
- Pre-registered re-runs: per gate at most 2 candidate and 1 baseline re-run for canary or near-threshold causes; INVALID and memory-adjustment re-runs do not count; beyond that, ask.
- **Still open, asked near the campaign's end:** may `hfs-mongo`'s data be deleted and its vhdx compacted before T3-full? Irreversible; PR4 Step 0 runs first if triggered.

Orchestrator: no env var for the catch-up margin (only tests change it); `HFS_REINDEX_WRITE_STREAMS` defaults to 1, and a failed K4 means PR3 does not merge.

Engineering: always-on timing, because `perf_phases` builds differ in codegen and never ship; writer phases via an out-parameter method, so no implementor or caller changes; no switch back to the old walk (rollback is the previous binary); an unknown cursor fails the run, since restarting the type could loop; PR3 adds `v2` tags, not `v3`, since cursors live only in memory; PR4 uses new index names, since MongoDB rejects a changed key spec under an existing name. PR2b is C+, not a prepare/commit trait split: within 2–5% of its speed, with at most two ~2,500-document sub-batches in flight instead of two prepared pages, and one defaulted trait method instead of two.

## 8. Risks

- **Random `resources` fetch (PR1)** competes with foreground reads on a live server. OBS-26 does not gate this: it reports resources KB read into cache per fetched resource and fg p99 as a proxy. G1.8/G1.9 bound only the rebuild's own fetch cost.
- **A1/A2 violations** (slow bulk-ingest batches, secondary reads) leave stale rows, as today's skew does; a rebuild overlapping an import of the same type chases it through rounds (cap WARN).
- **PR2b:** up to four extraction threads beside mongod (the threads knob caps it); prefetch widens the delete race to about two page writes (orphans never surface); an extraction panic on the overlapped path loses the page's old rows (the generation retries once).
- **The bench cannot swap** and is 26% scale: T3-full and the probe cover the gap.
- **PR4:** `$ne: null` index eligibility is unverified on MongoDB 5.0.6; rollback is one-way without the script.

## 9. Tests

- **PR0:** `reindex_stats` unit tests; the field-order capture tests in `reindex.rs`; the MongoDB integration test called through `&dyn ReindexTarget` (S1 §8).
- **PR1:** `mod reindex_walk_tests` in `storage.rs` and `tests/mongodb/reindex_id_walk.rs` T1–T12; a manual MongoDB 7.0 run, since CI runs 5.0.6 (S2 §9).
- **PR2a/PR2b:** `mod reindex_page_cap_tests`; `reindex_pipeline` unit tests; the prefetch driver fakes; multi-thread integration tests in `tests/mongodb/reindex_pipeline.rs` (S3 §4.5, §5.13).
- **PR3/PR4:** S5 §1.10 and §2.8, carried into their own plans.

## 10. Out of scope

Beyond §3's non-goals and §2.3: cache sizing in code; a synthetic replay; concurrent reindexes of the same types; writer phases for other backends; streams for manual `$reindex`; catalog or generation changes outside PR4; `SCHEMA_VERSION`; the T7 failures (#1473, #1474).

## 11. Evidence

`C:\Users\DougC\Code\Helios\manual-test\archive\1403-run17-evidence\` (local, not uploaded):

- `1403-causes.md` (pass 1), `1403-council-*.md`, `slow.jsonl` (238 MB slow log), `hfs-mongodb.run17.log`, `i1403.json`, `explain.out` (run 17's measured plan).
- `pass2/result.json`: investigations, conflict resolutions C1–C10, cost model, judge verdicts.
- `pass2/ftdc_decode.py` (validated decoder); `pass2/ftdc/` (raw `metrics.*`, `build_series.py`, `per_minute.csv` for Gate 0); `pass2/slowlog/`, `pass2/corpus/`, `pass2/env/`, `pass2/model/`, `pass2/resolve-C*/`.
- `design/`: the five approved section files, copied there from the session scratchpad before the plan is written. SHA256: S1-pr0-instrumentation.md `96f17650bdcc4d50e33049a53b08b238ad154614723d40aae701472a9e92b493`; S2-pr1-id-walk.md `ad782e257989392cef99e2ecbea58a036b37caa3530c1da3710cc92c805e8a4e`; S3-pr2-overlap.md `3918083534a81ae613af7b0da159338ba947bbd026eb74b1bfa817260dae2289`; S4-measurement.md `7e5e6ac5ea57757f79722db889a9fb0e3181455fe3e301ebe4800f9a82e5bcdf`; S5-followups-docs.md `a17a2ec7f4a60b001cbd9a63e4b76c87df69165153ebc88b2ba3d6fa09b659cc`.
