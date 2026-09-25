# MongoDB `$reindex` PR3: Concurrent Write Streams on Disjoint Id Ranges Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let the MongoDB deferred search-index rebuild split one resource type into up to `HFS_REINDEX_WRITE_STREAMS` contiguous id ranges, walk and write them concurrently (one insert stream each), then run the catch-up rounds once after every range has been written — so MongoDB's cache misses are served at queue depth K instead of 1.

**Architecture:** The backend-agnostic driver (`crates/persistence/src/search/reindex.rs`) gains a defaulted `ReindexSource::plan_type_walk` returning `TypeWalkPlan::{Single, Ranges { ranges, catch_up }}`. Its per-type paging loop moves, verbatim, into `walk_range` over a shared `RangeWalk` context, so a multi-range type runs one `walk_range` per range on a `tokio::task::JoinSet` and then one catch-up `walk_range`. MongoDB (`crates/persistence/src/backends/mongodb/storage.rs`) overrides `plan_type_walk` (standalone only: connection budget, size limit, covered `skip` probes of `idx_resources_identity`) and adds two tags to its in-memory `v2` cursor grammar: `v2|r|<floor>|<lo>|<hi>|<after_id>` (one range) and `v2|d|<floor>` (catch-up start). Every other backend inherits `Single`, and `HFS_REINDEX_WRITE_STREAMS` defaults to 1, so nothing changes unless an operator opts in.

**Tech Stack:** Rust 2024 (MSRV 1.90, let-chains), `tokio` 1.x (`task::JoinSet`; `select!` is already used in `reindex.rs`), `parking_lot`, `mongodb` driver 3.7.0 (`find_one(..).sort(..).skip(u64).projection(..).hint(..)`, `count_documents(..).hint(..)`), `tracing`, `clap` (server config). Tests: `cargo test -p helios-persistence --lib`; MongoDB integration tests through testcontainers (`cargo test -p helios-persistence --features mongodb --test mongodb_tests <filter>`).

**Spec:** `docs/superpowers/specs/2026-09-23-mongodb-reindex-rebuild-design.md` §4.5, §4.7, §5.5, §7, §8, §9 — and, binding at file level, `C:\Users\DougC\Code\Helios\manual-test\archive\1403-run17-evidence\design\S5-followups-docs.md` §0 and §1.1–§1.12 (the detailed design this plan follows), plus `S4-measurement.md` §4.8 (log contract), §4.10.1 (V12, V13), §4.10.9 (Gate PR3), §4.10.11 (re-runs) and the arm rows `K1-x`/`K4-x`/`K2-x`/`CU-3-ref`/`CU-3`/`P3-prov` (S4 §4.4). Where S5 names something that did not exist when it was written, this plan follows the code at PR2b's head `c23193e3b`; every difference is listed under **Drift from S5**.

## Global Constraints

- **Branch.** `perf/1403-pr3-mongodb-write-streams`, cut from PR2b's final head (PR #1519, branch `perf/1403-pr2b-mongodb-reindex-overlap`) in the **main checkout** `C:\Users\DougC\Code\Helios\hfs` — not a worktree. PR0 #1510, PR1 #1512, PR2a #1516 and PR2b #1519 are **unmerged**: never assume `main` contains any of them, and never rebase onto `main` in this plan.
- **Baseline and anchors.** This plan was written against `c23193e3b`. Line numbers below are hints at that commit; PR2b may gain review commits before this branch is cut. **Locate every anchor by the function/struct/const name given**, never by line number (`git grep -n -F -- "<name>" <path>`; after an edit, `tgrep -n -F --no-index -- "<name>" <path>`).
- **No cargo during a bench arm.** Before **every** `cargo` command in this plan (`cargo fmt` included, and each command of a multi-command block) run
  ```bash
  test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
  ```
  If it prints, stop and report instead of building.
- **Cargo hygiene.** Every cargo/test command runs in the **foreground** (never `run_in_background`, never Monitor; never end a turn waiting for a notification). Prefix each with `CARGO_BUILD_JOBS=4` (`cargo fmt` too). One cargo process at a time.
- **Disk.** If `C:` drops below 70 GB free (`df -h /c`), run `cargo clean -p helios-rest` (its integration-test binaries are ~46 GB). Never delete `target/` wholesale. Check it at the start of every task and right before any `helios-rest` build (Task 9 Step 1 spells both out).
- **Formatting.** `cargo fmt -- <explicit touched files>`, from the main checkout. Never `cargo fmt --all`.
- **Staging.** Stage explicit paths only. Never `git add -A`, never `git commit -a`: any build dirties ~3,500 R6 fixture files.
- **Commits.** One-line imperative subject ending `(#1403)`, a blank line, then exactly:
  ```
  Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01Y61t4nTxKCT8WeE5NbxaVc
  ```
  Commits are signed automatically (the repo requires signed commits); never pass `--no-gpg-sign`.
- **Comments and test names** describe behaviour or name code. They never cite plan labels (PR0–PR4, S1–S5, `§`, D-numbers, I1, "Task N", `file:line`). `#1403` is fine.
- **MongoDB integration tests** run only through the suite's testcontainers harness (`mongo:5.0.6`, pinned by testcontainers-modules). Never set `HFS_TEST_MONGODB_URL`, never start a MongoDB container or server by hand, and never point anything at `hfs-mongo` or `localhost:27017`. A skipped test is not a pass: quote the `test result: ok. N passed` line and confirm no `Skipping` line for the tests you ran.
- **Reuse test helpers; never copy one.** An implementer sees one task at a time, so the existing helpers are listed here. Before writing any test helper, search for one that already does the job (`git grep -n -F -- "fn <name>" crates/persistence`); if it is private, make it `pub(super)` and call it, as this plan does, rather than writing a near-copy.
  - `crates/persistence/tests/mongodb_tests.rs` (reached by `use super::*`): `create_backend`, `create_tenant`, `build_backend` (and `build_backend_with_pool`, Task 6), `search_index_entry_count`, `shared_mongo`, `build_test_database_name`, `repo_data_dir`, `TEST_BACKEND_MAX_POOL`; `super::bulk_submit::seed`.
  - `crates/persistence/tests/mongodb/reindex_pipeline.rs`: `create_backend_with` (and `create_backend_with_pool`, Task 6), `create_id_phase_backend`, `settle_into_id_phase`, `walk_capped`, `seed_provenance`, `RecordingSource`/`RecordingWriter`, `PhaseLogProbeTarget` (`pub(super)` from Task 7).
  - `crates/persistence/tests/mongodb/reindex_id_walk.rs`: `capture_walk_logs`, `walk_log_lines`, `wait_for_terminal` (already `pub(super)`); `WalkFixture`, `seed_walk_fixture`, `backdate_fixture`, `snapshot` (`pub(super)` from Task 5).
  - `crates/persistence/src/search/reindex.rs` `mod tests`: `named_tenant`, `await_finished`, `await_automatic_idle`, `controlled_operation`, `capture_contract`, `assert_contract`, `PagedSource`, `RecordingTarget`, `recording_operation`; Task 4 adds `RangedSource`, `RangedWriter`, `ranged_operation`, `streams_request`, `range_pages`, `new_seq`.
- **Runtimes.** Integration tests that run a rebuild through MongoDB's writer use `#[tokio::test(flavor = "multi_thread", worker_threads = 4)]` (the overlapped writer needs `block_in_place`). The driver-fake tests in `reindex.rs` use plain `#[tokio::test]`: the log-contract capture there (`capture_contract`) is a thread-local default subscriber, and a current-thread runtime runs every `JoinSet` stream on the test thread.
- **Clippy gates.** The first after every task that touches `crates/persistence`; all three in the final task, each after its own lock check:
  ```bash
  test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
  CARGO_BUILD_JOBS=4 cargo clippy -p helios-persistence --all-targets --all-features -- -D warnings -A clippy::items_after_test_module -A clippy::large_enum_variant -A clippy::question_mark -A clippy::collapsible_match -A clippy::collapsible_if -A clippy::field_reassign_with_default -A clippy::doc-overindented-list-items -A clippy::doc-lazy-continuation
  test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
  CARGO_BUILD_JOBS=4 cargo clippy -p helios-hfs --features mongodb -- -D warnings -A clippy::items_after_test_module -A clippy::large_enum_variant -A clippy::question_mark -A clippy::collapsible_match -A clippy::collapsible_if -A clippy::field_reassign_with_default -A clippy::doc-overindented-list-items -A clippy::doc-lazy-continuation
  test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
  CARGO_BUILD_JOBS=4 cargo clippy -p helios-hfs --no-default-features --features R4,mongodb -- -D warnings -A clippy::items_after_test_module -A clippy::large_enum_variant -A clippy::question_mark -A clippy::collapsible_match -A clippy::collapsible_if -A clippy::field_reassign_with_default -A clippy::doc-overindented-list-items -A clippy::doc-lazy-continuation
  ```
- **Knob** (spec §4.5, S5 §1.7, D11): `HFS_REINDEX_WRITE_STREAMS`, default `1`, clamped to `1..=16` (`REINDEX_MAX_WRITE_STREAMS`). Manual `POST $reindex` and automatic retries of named resources (`GenerationScope::Resources`) always use 1. `min_resources_per_stream` is `50_000` (`DEFAULT_MIN_RESOURCES_PER_STREAM`) with **no** env var and no hook setter. `concurrent_runs` is the automatic coordinator's `limits.max_concurrency` (= `HFS_BULK_SUBMIT_WORKER_CONCURRENCY`).
- **Plan rule** (S5 §1.4): `k = min(streams, max(1, (max_connections − 2) / (2 × max(concurrent_runs, 1))), max(1, n / min_resources_per_stream))`, where `n` is the type's document count on `idx_resources_identity` (tombstones included). Search offloaded ⇒ `Single`. Boundaries come from chained covered `skip` probes and affect balance only.
- **Cursor grammar** (S5 §1.5, D15), backend-private and in memory only: `v2|r|<floor>|<lo>|<hi>|<after_id>` (an empty field is an open bound, or "before the range's first page" for `after_id`) and `v2|d|<floor>`. `v2|s|…` and every other unknown tag stay `SearchError::InvalidCursor`.
- **Invariants.** Streams own disjoint contiguous id ranges; at most one page's delete/insert in flight per stream, in fetch order; a range ends only on an empty query; the catch-up (`v2|d|`) starts only after the `JoinSet` has drained, and the driver never returns early from, aborts, or drops that `JoinSet` before it is drained. No `parking_lot` guard is ever held across an `.await`; the only nested lock order is stats → jobs, and only after every stream has drained.
- **Memory.** A multi-range type never runs uncapped: with `batch_bytes == 0` its pages are capped at `REINDEX_MULTI_STREAM_BATCH_BYTES = 33_554_432`, with one warning per run.
- **Log contract.** The bench parses the lines in the **Bench contract** table exactly: never rename, reorder or re-target any of them.
- **This plan file** is committed once, in Task 1 Step 1, as PR0–PR2b's plans were. Implementers never edit it afterwards: track progress in the SDD workspace, not by ticking its boxes.
- **The final task** writes `pr-body.md` into this plan's SDD workspace. It does not push and does not open a PR.

## Measured context (why PR3 is being written)

- B2-s (PR2b on): Observation mongod write-busy 0.79–0.83 (MongoDB is the bottleneck, but under trigger T1's 0.85), HFS 0.65 cores, 158 ms per insert command.
- Half-cache probe B1-s-1G: Observation Q4/Q1 = 0.629 — trigger T2 (S5 §1.1) fired, in the 0.40–0.70 band. Per S4 §4.10.9 ("only the probe fired") the K arms run at **C_K = C_bench/2**; per spec §5.5 the PR4 arms are also due (outside this plan).

## Drift from S5 (checked against `c23193e3b`) and how this plan resolves it

| # | S5 says | Code at `c23193e3b` | This plan |
|---|---|---|---|
| 1 | `file:line` anchors at `c86d0f08b` | Every anchor moved (PR0–PR2b landed) | Anchors located by name; `c23193e3b` lines given as hints |
| 2 | `write_resource_batch(.., &mut failures, ..)` and `record_resource_failure(.., &mut failures, ..)` take `&parking_lot::Mutex<ResourceFailureLog>` | 8- and 7-argument free functions taking `&mut ResourceFailureLog` | Both take `ctx: &RangeWalk` (tenant, writers, jobs, job id and the locked failure log all live there); their `too_many_arguments` allows go |
| 3 | `record_and_log_page(&mut stats, ..)` → `record_and_log_page_shared(ctx, rt, record)` | `record_and_log_page` exists as named | Replaced; the old function is deleted (its two callers move to the new one) |
| 4 | `walk_range(.., should_stop: &mut (dyn FnMut() -> bool + Send))` is "PR2b's loop moved verbatim" | PR2b's loop is inline in `run_reindex` (`:2846-2984`) with `PrefetchedPage`, the `Ok(None)` serial re-fetch and `page_limit = request.batch_size.max(1)` | Moved verbatim with S5's substitutions; `page_limit` becomes `ctx.batch_size`. The named-resources loop moves to `walk_named_resources`, unchanged apart from `ctx` |
| 5 | `run_reindex` passes request fields to the per-type code | `request.resource_types` is moved out of `request` (`match (&named_resources, request.resource_types)`), a partial move that forbids `&request` | That match takes `request.resource_types.clone()`, so `walk_type` can take `&ReindexRequest` |
| 6 | `ReindexRunStats::set_type_plan`, `TypeSummary.{streams, plan}` | Neither exists | Added; `set_type_plan` lands with its first caller (Task 4) so no commit carries dead code |
| 7 | L3 appends `streams, plan_ms` after PR2b's four fields | L3 has 28 fields; the test `field_lists_append_fetch_wait_and_db_wait` pins its **last** four | That test's slice moves to `[len-6..len-2]`; a new test pins the last two (30 fields, 31 with `message`) |
| 8 | `ReindexWalkCursor::IdRange { floor, lo, hi, after_id }` flat | `ReindexWalkCursor::{Id { floor, after_id: String }, Round { .. }}` | `IdRange { range: ReindexIdRange { floor, lo, hi }, after_id: Option<String> }` — same encoding; the struct keeps the page method at 7 arguments |
| 9 | `IdPhaseDone` logs `id phase finished`, then continues as `RoundStart { 1, floor }` | The `IdPhase` arm logs `id phase finished` itself on an empty query | The `IdPhase` arm now steps to `WalkStep::IdPhaseDone { floor }`, the only place that line is logged |
| 10 | `reindex_walk_floor` extracted from PR1's `Start`; new `reindex_margin(&self)` | `Start` computes `t0`, `newest_live`, the floor and logs `walk started` inline; `margin` is a local of `fetch_reindex_page` | Both added as S5 says; `Start` and `plan_type_walk` share `reindex_walk_floor` |
| 11 | The `IdRange` page goes through `reindex_find_page` inside `fetch_reindex_page` | PR2b added `reindex_id_page(..) -> Option<ResourcePage>` (the id phase, serial and ahead) | New sibling `reindex_id_range_page(..) -> ResourcePage` (never `None`: a range ends on an empty page), used by the `IdRange` step and by `fetch_resources_page_ahead` |
| 12 | `build_backend_with_pool(config, pool)` next to `build_backend` | `build_backend(config)` caps every pool at `TEST_BACKEND_MAX_POOL = 4` | `build_backend` becomes a call to `build_backend_with_pool(config, TEST_BACKEND_MAX_POOL)` |
| 13 | PR3 integration tests go into `tests/mongodb/reindex_id_walk.rs`; PR1's `seed_submission` | `reindex_id_walk.rs` is 2,031 lines; the submission helper is `super::bulk_submit::seed(&backend, &tenant) -> (SubmissionId, String)` | New sibling `tests/mongodb/reindex_streams.rs`. `WalkFixture` (+`live`), `seed_walk_fixture`, `backdate_fixture`, `snapshot` become `pub(super)` (as PR2b did for `capture_walk_logs`); `PhaseLogProbeTarget` in `reindex_pipeline.rs` becomes `pub(super)` for reuse |
| 14 | Test 18 (offloaded ⇒ `Single`) is an integration test | The offload check runs before any database call | A Docker-free unit test, beside two more early-return unit tests (one stream; a pool budget of 1) |
| 15 | Test 19 explains "the probe" | The probe is private to `plan_type_walk` | The test turns on the profiler, plans, and explains the profiled `find`s that carry `skip` (the pattern of `mongodb_reindex_id_walk_pages_plan_without_a_blocking_sort`) |
| 16 | `crates/rest/src/config.rs`: add `reindex_write_streams: 1,` after `reindex_batch_bytes: 0,` | `reindex_batch_bytes` defaults to `32 * 1024 * 1024` (PR2a) | Added after `reindex_batch_bytes: 32 * 1024 * 1024,` in `Default` and `for_testing` |
| 17 | `crates/hfs/src/main.rs:1942` builds the hook inline | PR2a split it into `build_automatic_reindex_hook(op, config, ledger)` | `.with_write_streams(..)` goes there; `ReindexOnFinish::write_streams()` (mirroring PR2a's `batch_bytes()`) lets a test read it. The S3 branch's `ReindexOnFinish::new(op)` stays untouched (its `ops.reindex` is `None`) |
| 18 | `docs/mongodb/bulk-import-sizing.md` gets the knob line | That doc belongs to the separate, unmerged PR-docs and is absent here; the recommended K exists only after the K arms | Not touched: the recommended K goes there after adoption (a provisional value is never merged, spec §6). `docs/mongodb/search-indexes.md` — which now has PR1's "How `$reindex` walks a type" section — gains one paragraph |
| 19 | The plan line for a budget-only `Single` "with streams=1" | S5's step order applies the budget before counting | That line logs `resources=0` (not counted); documented on `log_streams_planned` |
| 20 | The clamp WARN uses `≥` | The bench's `STREAM_CLAMP_WARN_RE` accepts any text between `needs` and `using` | `≥`, verbatim from S4/S5 |
| 21 | Test 15 seeds 5,000 Observations in two tenants that share ids | `seed_walk_fixture(.., n, ..)` names Observations `obs-NNN` in every tenant; the test rebuilds tenant A three times | Tenant A gets 2,000 Observations and tenant B 200 (B's ids are A's first 200, so ids are still shared). Four ranges of 500 still exercise every split and byte-capped path while three rebuilds stay within CI time; the assertions are S5's |
| 22 | The driver passes `request.write_streams` through, and a cancel is first seen inside the streams | `ReindexRequest` is `pub` and deserialisable, so `write_streams` can bypass the setter's clamp; `plan_type_walk` (a count plus K−1 skip probes) cannot be interrupted | Beyond S5, pending controller confirmation: `walk_type` clamps to `REINDEX_MAX_WRITE_STREAMS` itself and rejects an empty `Ranges` before recording the plan; `walk_ranges` returns `Cancelled` before spawning when a cancel arrived during planning. Each is one line with its own Task 4 test |
| 23 | New integration-test helpers | `create_backend_with` (`reindex_pipeline.rs`) already builds a configurable backend, at the suite pool cap | Task 6 adds `create_backend_with_pool` beside it, `create_backend_with` delegates to it, and `create_streams_backend` is a one-line call of it |

## Bench contract (what `manual-test/tools/bench-1403/analyze_arm.py` parses)

| Line | Target | Level | Message | Fields, in order |
|---|---|---|---|---|
| L1 | `helios_persistence::search::reindex` | INFO | `reindex job started` | PR0's 11 fields, then **`write_streams`** (`PR3_APPENDED["L1"]`) |
| L3 | `helios_persistence::search::reindex` | INFO | `reindex type finished` | PR0's 24 + PR2b's `fetch_wait_ms, db_wait_ms, sub_batches, pool_sub_batches`, then **`streams, plan_ms`** (`PR3_APPENDED["L3"]`; V13 reads Observation's `streams`) |
| new | `helios_persistence::backends::mongodb::storage` | INFO | `mongodb reindex streams planned` | `tenant, resource_type, requested, allowed, resources, streams, plan_ms` (`WALK_CONTRACT["streams_planned"]`) |
| new | `helios_persistence::backends::mongodb::storage` | INFO | `mongodb reindex id range finished` | `tenant, resource_type, floor, lo, hi`; an open bound prints `*` (`WALK_CONTRACT["id_range_finished"]`) |
| new | `helios_persistence::backends::mongodb::storage` | WARN | `HFS_REINDEX_WRITE_STREAMS={requested} needs HFS_MONGODB_MAX_CONNECTIONS ≥ {2·requested·concurrent_runs+2}; using {k}` | then `tenant, resource_type` (`STREAM_CLAMP_WARN_RE`; any occurrence in a K arm fails V13) |
| kept | `…::mongodb::storage` | INFO | `mongodb reindex walk started` | unchanged; a multi-range type logs it once, from the plan (V12 reads `floor`/`newest_live`) |
| kept | `…::mongodb::storage` | INFO | `mongodb reindex id phase finished` | unchanged; a multi-range type logs it once, from the catch-up's first call, after every range page is written (the walk block of S4 §4.9.1 ends there) |
| knob | — | — | env `HFS_REINDEX_WRITE_STREAMS` | `KNOBS["HFS_REINDEX_WRITE_STREAMS"]` |

Values are bare tokens (`%` formatting); ids and RFC 3339 instants contain no spaces.

## Open items for the controller (outside this plan's code; nothing here blocks Tasks 1–9)

- **P3-prov cannot exercise K streams as specified.** The bench's Provenance manifest holds 11,704 resources (full corpus: 11,705). With `min_resources_per_stream` fixed at 50,000 and no override (S5 §1.4 step 5, §1.7), `plan_type_walk` returns `Single` for Provenance at any K, so P3-prov runs one walk with `HFS_REINDEX_BATCH_BYTES=0` — uncapped, because the automatic 32 MiB cap applies only to a split type — and its L3 `streams` is 1. G3.14 then measures PR2b's single walk, not K streams. This plan implements S5 as written.
- **Bench gaps for `--gate pr3`.** `compare_arms.py` injects `cu1.pass`/`cu2b.pass` from `--cu-run` only for `pr1`/`pr2b`, so G3.10 (`cu3.pass`) resolves to TOOL_ERROR; and `analyze_arm.py` never writes `db_wait_over_wall_share` or `hfs_cores_share_of_logical_cores`, which `pr3_verdict`'s inconclusive/HFS-bound override (S4 §4.10.9) reads, so that override can never fire. The log lines it parses already match this plan (table above).
- **The covered-probe check on MongoDB 7.0.** `mongodb_plan_type_walk_boundary_probes_are_covered` asserts `totalDocsExamined == 0` only on 7.0 or later, but the testcontainers harness is pinned to `mongo:5.0.6` and implementers may not start a server by hand or touch `hfs-mongo`. On 5.0.6 the test asserts the plan (the identity index, no `SORT` stage) and prints it. Whether that suffices for PR3, or who runs the 7.0 check and against which server, is the controller's call.
- **Three one-line guards beyond S5 (Drift 22).** `walk_type` clamps `write_streams` to 16 itself and rejects an empty `Ranges` before recording the plan; `walk_ranges` returns `Cancelled` before spawning when a cancel arrived during planning. Each has its own Task 4 test (`write_streams_above_the_maximum_plan_at_the_maximum`, `a_plan_without_ranges_fails_the_type`, `cancel_during_planning_fetches_no_range`). To drop one, delete its line and its test before Task 4 runs.

---

## File Structure

| File | Change | Task |
|---|---|---|
| `crates/persistence/src/search/reindex.rs` | `REINDEX_MAX_WRITE_STREAMS`, `DEFAULT_MIN_RESOURCES_PER_STREAM`, `TypeWalkPlan`, `TypeWalkRequest`, `ReindexSource::plan_type_walk` (default `Single`), three `ReindexRequest` fields + setters, `AutomaticRunOptions.write_streams`, `GenerationScope::request(options, concurrent_runs)`, `ReindexOnFinish::{with_write_streams, write_streams}` | 1 |
| same | L1 `write_streams`, L3 `streams`/`plan_ms`, the field-order test constants | 2 |
| same | `RangeWalk`, `walk_range`, `walk_named_resources`, `walk_type`, `record_and_log_page_shared`; `write_resource_batch`/`record_resource_failure` take `&RangeWalk`; `run_reindex` uses them — no behaviour change | 3 |
| same | `REINDEX_MULTI_STREAM_BATCH_BYTES`, `RangeWalk.{stop, cap_warned}`, planning in `walk_type`, `walk_ranges`, `multi_stream_page_bytes`; the `RangedSource`/`RangedWriter` driver fakes and their tests | 4 |
| `crates/persistence/src/search/reindex_stats.rs` | `TypeSummary.{streams, plan}` (Task 2); `ReindexRunStats::set_type_plan` (Task 4) | 2, 4 |
| `crates/persistence/src/search/mod.rs` | re-export the four new public items | 1 |
| `crates/persistence/src/backends/mongodb/storage.rs` | `ReindexIdRange`, `ReindexWalkCursor::{IdRange, IdPhaseDone}` and their `WalkStep` arms, `reindex_id_range_page_filter`, `reindex_id_range_page`, `reindex_walk_floor`, `reindex_margin`, prefetch of `IdRange` (Task 5); `reindex_stream_budget`, `reindex_streams_for_size`, `reindex_id_range_cursors`, `log_streams_planned`, `reindex_range_boundaries`, the `plan_type_walk` override (Task 6) | 5, 6 |
| `crates/persistence/src/backends/mongodb/backend.rs` | `reindex_streams_clamp_warned: AtomicBool` + accessor | 6 |
| `crates/persistence/tests/mongodb_tests.rs` | `mod reindex_streams;` (Task 5); `build_backend_with_pool` (Task 6) | 5, 6 |
| `crates/persistence/tests/mongodb/reindex_id_walk.rs` | `WalkFixture` (+`live`), `seed_walk_fixture`, `backdate_fixture`, `snapshot` → `pub(super)` | 5 |
| `crates/persistence/tests/mongodb/reindex_pipeline.rs` | `create_backend_with_pool`, which `create_backend_with` now calls (Task 6); `PhaseLogProbeTarget` and its fields → `pub(super)` (Task 7) | 6, 7 |
| `crates/persistence/tests/mongodb/reindex_streams.rs` (new) | cursor tests (5), plan tests (6), whole-rebuild tests (7) | 5, 6, 7 |
| `crates/rest/src/config.rs` | `reindex_write_streams` (`HFS_REINDEX_WRITE_STREAMS`, default 1) + test | 8 |
| `crates/hfs/src/main.rs` | `build_automatic_reindex_hook` sets `.with_write_streams(..)` + test | 8 |
| `README.md`, `crates/hfs/README.md`, `book/src/configuration/environment-variables.md`, `docs/mongodb/search-indexes.md` | knob rows; one "write streams" paragraph | 8 |
| `.claude/skills/bulk-data-submit/SKILL.md`, `.agents/skills/bulk-data-submit/SKILL.md` | the rebuild-knobs bullet gains the new knob; the rebuild-log bullet gains `write_streams`, `streams`, `plan_ms` and the `other_ms` caveat (the same edit in both copies) | 8 |
| `docs/superpowers/plans/2026-09-25-1403-pr3-mongodb-write-streams.md` | this plan, committed as it stands | 1 |

---

### Task 1: Walk-plan types, request fields, and the automatic hook's write streams

**Files:**
- Modify: `crates/persistence/src/search/reindex.rs` — after `pub struct SkippedResource` (`:115-124`); in `trait ReindexSource` after `fetch_resources_page_ahead` (`:283-294`); `pub struct ReindexRequest` (`:468-508`), `fn default_batch_size` (`:510-512`), `impl Default for ReindexRequest` (`:514-526`), `impl ReindexRequest` (`:528-600`); `pub(crate) struct AutomaticRunOptions` and its `Default` (`:945-965`); `GenerationScope::request` (`:1500-1512`) and its one caller in `AutomaticReindexCoordinator::run_tenant` (`:1800-1802`); `impl ReindexOnFinish` after `pub fn batch_bytes` (`:3133-3135`); `mod tests` (after `fn test_reindex_request`, `:6073`).
- Modify: `crates/persistence/src/search/mod.rs` — the `pub use reindex::{..}` block (`:117-121`).

**Interfaces:**
- Consumes: nothing new.
- Produces (public, re-exported from `helios_persistence::search`):
  - `pub const REINDEX_MAX_WRITE_STREAMS: u32 = 16;`
  - `pub const DEFAULT_MIN_RESOURCES_PER_STREAM: u64 = 50_000;`
  - `pub enum TypeWalkPlan { Single, Ranges { ranges: Vec<String>, catch_up: String } }` (`Debug, Clone, PartialEq, Eq`)
  - `pub struct TypeWalkRequest { pub streams: u32, pub min_resources_per_stream: u64, pub concurrent_runs: u32 }` (`Debug, Clone, Copy, PartialEq, Eq`)
  - `async fn ReindexSource::plan_type_walk(&self, tenant: &TenantContext, resource_type: &str, request: TypeWalkRequest) -> StorageResult<TypeWalkPlan>` — default `Ok(TypeWalkPlan::Single)`
  - `ReindexRequest.{write_streams: u32, min_resources_per_stream: u64, concurrent_runs: u32}` (serde defaults 1 / 50,000 / 1) and `with_write_streams(u32)`, `with_min_resources_per_stream(u64)`, `with_concurrent_runs(usize)`
  - `ReindexOnFinish::with_write_streams(self, u32) -> Self`, `ReindexOnFinish::write_streams(&self) -> u32`
- Produces (crate-private): `AutomaticRunOptions.write_streams: u32`; `GenerationScope::request(&self, options: AutomaticRunOptions, concurrent_runs: usize) -> ReindexRequest`.

- [ ] **Step 1: Cut the branch**

```bash
cd /c/Users/DougC/Code/Helios/hfs
git fetch origin
git status --short -uno -- crates docs book README.md .claude .agents
```
Expected: no output. If anything is listed, stop and report: uncommitted work must not ride into this branch.

```bash
git rev-parse perf/1403-pr2b-mongodb-reindex-overlap origin/perf/1403-pr2b-mongodb-reindex-overlap
git merge-base --is-ancestor c23193e3b perf/1403-pr2b-mongodb-reindex-overlap && echo "contains the planned baseline"
```
Expected: the two hashes are equal, then `contains the planned baseline`. If the hashes differ, stop and report (the controller decides which head is PR2b's final one).

```bash
git switch -c perf/1403-pr3-mongodb-write-streams perf/1403-pr2b-mongodb-reindex-overlap
df -h /c
```
Expected: at least 70 GB available on `C:`; otherwise run the lock check, then `CARGO_BUILD_JOBS=4 cargo clean -p helios-rest`.

Commit this plan on the new branch, as PR0–PR2b's plans were committed, so the PR body's reference to it resolves. It is untracked in the main checkout, so the switch carries it over:

```bash
git status --short -- docs/superpowers/plans/2026-09-25-1403-pr3-mongodb-write-streams.md
```
Expected: `?? docs/superpowers/plans/2026-09-25-1403-pr3-mongodb-write-streams.md`. If the file shows as tracked or is absent, stop and report.

```bash
git add docs/superpowers/plans/2026-09-25-1403-pr3-mongodb-write-streams.md
git commit -m "$(cat <<'EOF'
docs(plan): implementation plan for concurrent reindex write streams (#1403)

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Y61t4nTxKCT8WeE5NbxaVc
EOF
)"
```
From here on, never edit this file.

- [ ] **Step 2: Write the failing tests**

Append to `mod tests` in `crates/persistence/src/search/reindex.rs`, right after `fn test_reindex_request`:

```rust
    #[test]
    fn write_stream_fields_default_clamp_and_deserialize() {
        let request = ReindexRequest::default();
        assert_eq!(request.write_streams, 1);
        assert_eq!(
            request.min_resources_per_stream,
            DEFAULT_MIN_RESOURCES_PER_STREAM
        );
        assert_eq!(request.concurrent_runs, 1);

        // A request serialized before these fields existed still deserializes.
        let legacy: ReindexRequest = serde_json::from_value(
            serde_json::json!({"resource_types": null, "search_param_urls": null}),
        )
        .unwrap();
        assert_eq!(legacy.write_streams, 1);
        assert_eq!(legacy.min_resources_per_stream, 50_000);
        assert_eq!(legacy.concurrent_runs, 1);

        let streams = |n: u32| ReindexRequest::default().with_write_streams(n).write_streams;
        assert_eq!(streams(0), 1);
        assert_eq!(streams(4), 4);
        assert_eq!(streams(40), REINDEX_MAX_WRITE_STREAMS);
        assert_eq!(
            ReindexRequest::default()
                .with_min_resources_per_stream(0)
                .min_resources_per_stream,
            1
        );
        let runs = |n: usize| {
            ReindexRequest::default()
                .with_concurrent_runs(n)
                .concurrent_runs
        };
        assert_eq!(runs(0), 1);
        assert_eq!(runs(3), 3);
        assert_eq!(runs(usize::MAX), u32::MAX);
    }

    #[test]
    fn resource_scoped_generations_ignore_write_streams() {
        let options = AutomaticRunOptions {
            write_streams: 4,
            ..AutomaticRunOptions::default()
        };
        let types = GenerationScope::Types(vec!["Observation".to_string()]).request(options, 2);
        assert_eq!(types.write_streams, 4);
        assert_eq!(types.concurrent_runs, 2);
        assert_eq!(
            types.min_resources_per_stream,
            DEFAULT_MIN_RESOURCES_PER_STREAM
        );

        let resources = GenerationScope::Resources(vec![ResourceRef::new("Observation", "o1")])
            .request(options, 2);
        assert_eq!(resources.write_streams, 1);
        assert_eq!(resources.concurrent_runs, 1);
    }

    #[tokio::test]
    async fn plan_type_walk_defaults_to_the_single_walk() {
        let plan = PagedSource::new(3)
            .plan_type_walk(
                &named_tenant("default-plan"),
                "Patient",
                TypeWalkRequest {
                    streams: 4,
                    min_resources_per_stream: 1,
                    concurrent_runs: 1,
                },
            )
            .await
            .unwrap();
        assert_eq!(plan, TypeWalkPlan::Single);
    }

    #[test]
    fn the_automatic_hook_clamps_its_write_streams() {
        let (backend, _events) = ControlledBackend::new(Vec::new(), 0);
        let op = controlled_operation(backend);
        assert_eq!(ReindexOnFinish::new(op.clone()).write_streams(), 1);
        assert_eq!(
            ReindexOnFinish::new(op.clone())
                .with_write_streams(0)
                .write_streams(),
            1
        );
        assert_eq!(
            ReindexOnFinish::new(op.clone())
                .with_write_streams(4)
                .write_streams(),
            4
        );
        assert_eq!(
            ReindexOnFinish::new(op)
                .with_write_streams(99)
                .write_streams(),
            REINDEX_MAX_WRITE_STREAMS
        );
    }
```

- [ ] **Step 3: Run the tests to verify they fail**

```bash
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
CARGO_BUILD_JOBS=4 cargo test -p helios-persistence --lib search::reindex:: 2>&1 | tail -40
```
Expected: compile errors naming `write_streams`, `DEFAULT_MIN_RESOURCES_PER_STREAM`, `TypeWalkRequest`, `TypeWalkPlan` and `plan_type_walk`.

- [ ] **Step 4: Implement**

(a) Directly after `pub struct SkippedResource { .. }`, add:

```rust
/// Most concurrent write streams one resource type's rebuild may use;
/// `HFS_REINDEX_WRITE_STREAMS` is clamped to it (#1403).
pub const REINDEX_MAX_WRITE_STREAMS: u32 = 16;

/// Fewest resources, tombstones included, that each write stream of a type
/// must cover, so a small type keeps its single walk (#1403).
pub const DEFAULT_MIN_RESOURCES_PER_STREAM: u64 = 50_000;

/// How the rebuild walks one resource type (#1403).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeWalkPlan {
    /// One walk from a `None` cursor: the only plan before #1403.
    Single,
    /// Disjoint id ranges walked concurrently, one write stream each, then
    /// one catch-up walk that starts only after every range has finished.
    Ranges {
        /// The first cursor of each range. Never empty.
        ranges: Vec<String>,
        /// The cursor the catch-up walk starts from.
        catch_up: String,
    },
}

/// What the driver asks of [`ReindexSource::plan_type_walk`] (#1403).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypeWalkRequest {
    /// Write streams the run asked for; at least 2 whenever the driver plans.
    pub streams: u32,
    /// Fewest resources each stream must cover.
    pub min_resources_per_stream: u64,
    /// Automatic rebuilds that may run at once, sharing the source's
    /// connection pool.
    pub concurrent_runs: u32,
}
```

(b) In `pub trait ReindexSource`, directly after the default `fetch_resources_page_ahead`, add:

```rust
    /// Splits one type's walk into concurrent id ranges (#1403). The default
    /// keeps the single walk, as does any source that cannot split. A source
    /// that splits returns disjoint ranges that together cover every resource
    /// its single walk would reach, apart from writes the catch-up picks up,
    /// plus the cursor the catch-up walk starts from. The driver walks every
    /// range to its end before it starts the catch-up, and it calls this only
    /// when the run asks for more than one write stream.
    async fn plan_type_walk(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        request: TypeWalkRequest,
    ) -> StorageResult<TypeWalkPlan> {
        let _ = (tenant, resource_type, request);
        Ok(TypeWalkPlan::Single)
    }
```

(c) In `pub struct ReindexRequest`, after the `batch_bytes` field, add:

```rust
    /// Concurrent write streams per resource type, `1..=`
    /// [`REINDEX_MAX_WRITE_STREAMS`] (#1403). Above 1 the driver asks the
    /// source to split each type into id ranges
    /// ([`ReindexSource::plan_type_walk`]); a source that does not split,
    /// and a run scoped to named resources, walk each type once.
    /// `POST $reindex` always uses 1.
    #[serde(default = "default_write_streams")]
    pub write_streams: u32,

    /// Fewest resources each write stream of a type must cover (#1403).
    #[serde(default = "default_min_resources_per_stream")]
    pub min_resources_per_stream: u64,

    /// Automatic rebuilds that may run at once, so a source that plans
    /// streams can divide its connection pool between them (#1403).
    #[serde(default = "default_concurrent_runs")]
    pub concurrent_runs: u32,
```

After `fn default_batch_size() -> u32 { .. }`, add:

```rust
fn default_write_streams() -> u32 {
    1
}

fn default_min_resources_per_stream() -> u64 {
    DEFAULT_MIN_RESOURCES_PER_STREAM
}

fn default_concurrent_runs() -> u32 {
    1
}
```

In `impl Default for ReindexRequest`, after `batch_bytes: 0,`, add:

```rust
            write_streams: default_write_streams(),
            min_resources_per_stream: default_min_resources_per_stream(),
            concurrent_runs: default_concurrent_runs(),
```

In `impl ReindexRequest`, after `pub fn with_batch_bytes`, add:

```rust
    /// Sets the write streams per type (see [`Self::write_streams`]), clamped
    /// to `1..=`[`REINDEX_MAX_WRITE_STREAMS`].
    pub fn with_write_streams(mut self, streams: u32) -> Self {
        self.write_streams = streams.clamp(1, REINDEX_MAX_WRITE_STREAMS);
        self
    }

    /// Sets the fewest resources per write stream (see
    /// [`Self::min_resources_per_stream`]); at least 1.
    pub fn with_min_resources_per_stream(mut self, resources: u64) -> Self {
        self.min_resources_per_stream = resources.max(1);
        self
    }

    /// Sets how many automatic rebuilds may run at once (see
    /// [`Self::concurrent_runs`]); at least 1, saturating at `u32::MAX`.
    pub fn with_concurrent_runs(mut self, runs: usize) -> Self {
        self.concurrent_runs = u32::try_from(runs).unwrap_or(u32::MAX).max(1);
        self
    }
```

(d) In `pub(crate) struct AutomaticRunOptions`, after `batch_bytes`, add:

```rust
    /// `ReindexRequest::write_streams` for the type-scoped runs the hook starts.
    pub(crate) write_streams: u32,
```

and in its `Default`, after `batch_bytes: 0,`, add `write_streams: 1,`.

(e) Replace `GenerationScope::request` with:

```rust
    fn request(&self, options: AutomaticRunOptions, concurrent_runs: usize) -> ReindexRequest {
        match self {
            Self::Types(types) => ReindexRequest::for_types(types.clone())
                .with_bulk_index_rebuild(options.bulk_index_rebuild)
                .with_write_streams(options.write_streams)
                .with_concurrent_runs(concurrent_runs),
            // Dropping and rebuilding a writer's value indexes costs a pass
            // over its whole index: out of proportion for a handful of ids.
            // A retry of named resources fetches them by id, so there is no
            // type walk to split either: it keeps one write stream (#1403).
            Self::Resources(resources) => ReindexRequest::for_resources(resources.clone()),
        }
        .with_batch_size(options.batch_size)
        .with_batch_bytes(options.batch_bytes)
    }
```

and in `run_tenant` change `scope.request(options)` to `scope.request(options, limits.max_concurrency)`.

(f) In `impl ReindexOnFinish`, after `pub fn batch_bytes`, add:

```rust
    /// Concurrent write streams per resource type for this hook's type-scoped
    /// rebuilds (`HFS_REINDEX_WRITE_STREAMS`), clamped to
    /// `1..=`[`REINDEX_MAX_WRITE_STREAMS`] (#1403). Only a source that splits
    /// a type (standalone MongoDB) uses more than one.
    pub fn with_write_streams(mut self, streams: u32) -> Self {
        self.options.write_streams = streams.clamp(1, REINDEX_MAX_WRITE_STREAMS);
        self
    }

    /// The write streams this hook's type-scoped rebuilds ask for (#1403);
    /// exposed for the same reason as [`Self::batch_bytes`].
    pub fn write_streams(&self) -> u32 {
        self.options.write_streams
    }
```

(g) In `crates/persistence/src/search/mod.rs`, replace the `pub use reindex::{..};` block with:

```rust
pub use reindex::{
    DEFAULT_MIN_RESOURCES_PER_STREAM, DEFERRED_REINDEX_BATCH_SIZE, DeferredReindexLedger,
    REINDEX_MAX_WRITE_STREAMS, ReindexOnFinish, ReindexOperation, ReindexPageStats,
    ReindexProgress, ReindexProgressError, ReindexRequest, ReindexSource, ReindexStatus,
    ReindexTarget, ReindexableStorage, ResourcePage, ResourceRef, SkippedResource, TypeWalkPlan,
    TypeWalkRequest,
};
```

- [ ] **Step 5: Run the tests to verify they pass**

```bash
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
CARGO_BUILD_JOBS=4 cargo test -p helios-persistence --lib search::reindex:: 2>&1 | tail -40
```
Expected: `test result: ok.` including the four new tests; no other test changes outcome.

- [ ] **Step 6: fmt, clippy, commit**

```bash
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
CARGO_BUILD_JOBS=4 cargo fmt -- crates/persistence/src/search/reindex.rs crates/persistence/src/search/mod.rs
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
CARGO_BUILD_JOBS=4 cargo clippy -p helios-persistence --all-targets --all-features -- -D warnings -A clippy::items_after_test_module -A clippy::large_enum_variant -A clippy::question_mark -A clippy::collapsible_match -A clippy::collapsible_if -A clippy::field_reassign_with_default -A clippy::doc-overindented-list-items -A clippy::doc-lazy-continuation 2>&1 | tail -30
git add crates/persistence/src/search/reindex.rs crates/persistence/src/search/mod.rs
git commit -m "$(cat <<'EOF'
feat(persistence): walk-plan types and write-stream fields for the reindex driver (#1403)

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Y61t4nTxKCT8WeE5NbxaVc
EOF
)"
```

---

### Task 2: Log `write_streams` on L1 and `streams`/`plan_ms` on L3

**Files:**
- Modify: `crates/persistence/src/search/reindex_stats.rs` — `pub(super) struct TypeSummary` (`:148-156`), `struct OpenType` (`:167-173`), `ReindexRunStats::start_type` (`:221-250`), `ReindexRunStats::finish_type` (`:318-338`), `mod tests`.
- Modify: `crates/persistence/src/search/reindex.rs` — `fn log_job_started` (`:2241-2276`) and its one call in `run_reindex` (`:2710-2722`); `fn log_type_finished` (`:2303-2357`); in `mod tests`: `JOB_STARTED_FIELDS` (`:6194`), `TYPE_FINISHED_FIELDS` (`:6216`), `fn field_lists_append_fetch_wait_and_db_wait` (`:6442`).

**Interfaces:**
- Consumes: `ReindexRequest.write_streams` (Task 1).
- Produces: `TypeSummary.{streams: u32, plan: Duration}` (`pub(super)`), defaulting to 1 and zero from `start_type`; `fn log_job_started(.., writers: usize, setup: Duration, write_streams: u32)`.

- [ ] **Step 1: Write the failing tests**

In `crates/persistence/src/search/reindex_stats.rs`, `mod tests`, add:

```rust
    #[test]
    fn a_type_reports_one_stream_and_no_plan_time_by_default() {
        let t0 = Instant::now();
        let mut stats = ReindexRunStats::new(t0, 1, 1, Duration::from_secs(60));
        stats.start_type("Patient", 1, t0);
        let summary = stats
            .finish_type(OUTCOME_COMPLETED, t0 + Duration::from_secs(1))
            .unwrap();
        assert_eq!(summary.streams, 1);
        assert_eq!(summary.plan, Duration::ZERO);
    }
```

In `crates/persistence/src/search/reindex.rs`, `mod tests`:

- append `"write_streams",` as the last entry of `JOB_STARTED_FIELDS` (after `"setup_ms",`);
- append `"streams",` and `"plan_ms",` as the last two entries of `TYPE_FINISHED_FIELDS` (after `"pool_sub_batches",`);
- in `fn field_lists_append_fetch_wait_and_db_wait`, change the first assertion's slice from `&TYPE_FINISHED_FIELDS[TYPE_FINISHED_FIELDS.len() - 4..]` to `&TYPE_FINISHED_FIELDS[TYPE_FINISHED_FIELDS.len() - 6..TYPE_FINISHED_FIELDS.len() - 2]`;
- add these two tests after `fn field_lists_append_fetch_wait_and_db_wait`:

```rust
    #[test]
    fn field_lists_append_write_streams_streams_and_plan_ms() {
        assert_eq!(JOB_STARTED_FIELDS.last(), Some(&"write_streams"));
        assert_eq!(
            &TYPE_FINISHED_FIELDS[TYPE_FINISHED_FIELDS.len() - 2..],
            ["streams", "plan_ms"]
        );
        // 30 fields plus `message` stays inside a line's 32-field budget.
        assert_eq!(TYPE_FINISHED_FIELDS.len(), 30);
    }

    #[tokio::test]
    async fn a_single_walk_type_logs_one_stream_and_no_plan_time() {
        let (_guard, events) = capture_contract();
        let source = Arc::new(PagedSource::new(3));
        let op = Arc::new(ReindexOperation::with_parts(
            source,
            vec![Arc::new(MeasuringTarget)],
            Arc::new(crate::search::TenantSearchRegistries::base_only()),
        ));
        let job_id = op
            .start(
                named_tenant("single-walk-streams"),
                ReindexRequest::for_types(vec!["Patient".to_string()]).with_batch_size(2),
                None,
            )
            .await
            .unwrap();
        let progress = await_finished(&op, &job_id).await;
        assert_eq!(progress.status, ReindexStatus::Completed);

        let events = events.lock().unwrap().clone();
        assert_contract(&events);
        let job_started = events
            .iter()
            .find(|e| e.message == "reindex job started")
            .expect("reindex job started");
        assert_eq!(job_started.values["write_streams"], "1");
        let type_finished = events
            .iter()
            .find(|e| e.message == "reindex type finished")
            .expect("reindex type finished");
        assert_eq!(type_finished.values["streams"], "1");
        assert_eq!(type_finished.values["plan_ms"], "0");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
CARGO_BUILD_JOBS=4 cargo test -p helios-persistence --lib search::reindex 2>&1 | tail -40
```
Expected: compile error `no field streams on type TypeSummary` (and `plan`).

- [ ] **Step 3: Implement**

(a) `reindex_stats.rs` — add to `pub(super) struct TypeSummary`, after `counters`:

```rust
    /// Write streams the type was walked with: 1 unless the source split it
    /// into id ranges (#1403).
    pub(super) streams: u32,
    /// Time spent planning the type's streams; zero when it was not planned
    /// (#1403).
    pub(super) plan: Duration,
```

add to `struct OpenType`, after `counters: Counters,`:

```rust
    streams: u32,
    plan: Duration,
```

in `start_type`, inside `Some(OpenType { .. })`, after `counters: Counters::default(),` add:

```rust
            streams: 1,
            plan: Duration::ZERO,
```

in `finish_type`, inside `Some(TypeSummary { .. })`, after `counters: open.counters,` add:

```rust
            streams: open.streams,
            plan: open.plan,
```

(b) `reindex.rs` — replace `fn log_job_started` (doc comment included) with:

```rust
/// Logs L1 `reindex job started` (INFO), once per run, after counting and
/// before `clear_existing` or `begin_bulk_index_rebuild`. Field order:
/// `tenant, job_id, types, total, batch_size, batch_bytes, bulk_index_rebuild,
/// clear_existing, resource_scoped, writers, setup_ms, write_streams`.
/// `types`/`total` come from `stats` (job-scoped, fixed for the run); the rest
/// mirror the request's shape so each arm's configuration is visible in the
/// log. `setup_ms` is the time from entry into `run_reindex` to the end of
/// counting. `write_streams` is the write streams per type the run asked for;
/// the streams a type actually used are its `reindex type finished` line's
/// `streams`. Fields are appended only, never renamed, removed or reordered
/// (#1403).
#[allow(clippy::too_many_arguments)]
fn log_job_started(
    tenant: &str,
    job_id: &str,
    stats: &ReindexRunStats,
    batch_size: u32,
    batch_bytes: u64,
    bulk_index_rebuild: bool,
    clear_existing: bool,
    resource_scoped: bool,
    writers: usize,
    setup: Duration,
    write_streams: u32,
) {
    tracing::info!(
        tenant = %tenant,
        job_id = %job_id,
        types = stats.types() as u64,
        total = stats.total(),
        batch_size = batch_size as u64,
        batch_bytes = batch_bytes,
        bulk_index_rebuild = bulk_index_rebuild,
        clear_existing = clear_existing,
        resource_scoped = resource_scoped,
        writers = writers as u64,
        setup_ms = millis(setup),
        write_streams = u64::from(write_streams),
        "reindex job started"
    );
}
```

and in `run_reindex`'s `log_job_started(..)` call add `request.write_streams,` as the last argument (after `run_started.elapsed(),`).

(c) `reindex.rs` — in `fn log_type_finished`, append after `pool_sub_batches = s.counters.writer.pool_sub_batches,`:

```rust
        streams = u64::from(s.streams),
        plan_ms = millis(s.plan),
```

and in its doc comment: extend the field-order list's end to `…, sub_batches, pool_sub_batches, streams, plan_ms`, and add after the sentence that defines `sub_batches`/`pool_sub_batches`:

```rust
/// `streams` is the number of write streams the type was walked with (1
/// unless the source split it into id ranges) and `plan_ms` the time spent
/// planning them (0 when it was not planned). With `streams` above 1 every
/// phase field (`fetch_ms` through `pool_sub_batches`) is summed over the
/// streams while `type_elapsed_ms` stays wall time, so `other_ms` saturates
/// to 0 and must not be read; `(fetch_wait_ms + write_ms + yield_ms) /
/// type_elapsed_ms` is then the type's effective concurrency.
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
CARGO_BUILD_JOBS=4 cargo test -p helios-persistence --lib search::reindex 2>&1 | tail -40
```
Expected: `test result: ok.` — including `a_run_logs_job_type_progress_and_page_lines_with_the_documented_fields` and every other `assert_contract` test, which now check the appended fields too.

- [ ] **Step 5: fmt, clippy, commit**

```bash
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
CARGO_BUILD_JOBS=4 cargo fmt -- crates/persistence/src/search/reindex.rs crates/persistence/src/search/reindex_stats.rs
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
CARGO_BUILD_JOBS=4 cargo clippy -p helios-persistence --all-targets --all-features -- -D warnings -A clippy::items_after_test_module -A clippy::large_enum_variant -A clippy::question_mark -A clippy::collapsible_match -A clippy::collapsible_if -A clippy::field_reassign_with_default -A clippy::doc-overindented-list-items -A clippy::doc-lazy-continuation 2>&1 | tail -30
git add crates/persistence/src/search/reindex.rs crates/persistence/src/search/reindex_stats.rs
git commit -m "$(cat <<'EOF'
feat(persistence): log write_streams on the job line and streams/plan_ms on the type line (#1403)

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Y61t4nTxKCT8WeE5NbxaVc
EOF
)"
```

---

### Task 3: Move the page loop into `walk_range` over a shared `RangeWalk` (no behaviour change)

A pure refactor: every existing driver test is the regression test, so there is no new failing test. The new functions are `Send` by construction (the spawned `run_reindex` future would not compile otherwise).

**Files:**
- Modify: `crates/persistence/src/search/reindex.rs` — `fn record_resource_failure` (`:2059-2073`), `fn write_resource_batch` and its doc comment (`:2088-2170`), `fn record_and_log_page` (`:2227-2239`), a new block between `impl Drop for PrefetchedPage` and `run_reindex`'s doc comment (`:2586-2592`), and inside `async fn run_reindex` (`:2593-3050`).

**Interfaces:**
- Consumes: `PrefetchedPage::spawn(source: Arc<dyn ReindexSource>, tenant: TenantContext, resource_type: String, cursor: String, limit: u32, max_bytes: u64)` and `PrefetchedPage::wait(self)` (PR2b, unchanged); `ResourceFailureLog::{new, start_type, record, finish_type}`; `ReindexRunStats::{record_page, add_yield, start_type, finish_type}`.
- Produces (all private to `reindex.rs`):
  - `struct RangeWalk { tenant: TenantContext, tenant_label: String, job_id: String, source: Arc<dyn ReindexSource>, writers: Vec<Arc<dyn ReindexTarget>>, jobs: Arc<RwLock<HashMap<String, ReindexProgress>>>, batch_size: u32, failures: parking_lot::Mutex<ResourceFailureLog>, stats: parking_lot::Mutex<ReindexRunStats> }`
  - `fn record_resource_failure(ctx: &RangeWalk, resource_type: &str, resource_id: &str, error: String, retryable: bool)`
  - `async fn write_resource_batch(ctx: &RangeWalk, resource_type: &str, resources: &[StoredResource], extra_processed: u64) -> BatchOutcome`
  - `fn record_and_log_page_shared(ctx: &RangeWalk, resource_type: &str, record: PageRecord)`
  - `async fn walk_range(ctx: &RangeWalk, resource_type: &str, start: Option<String>, page_bytes: u64, should_stop: &mut (dyn FnMut() -> bool + Send)) -> Result<(), RunExit>`
  - `async fn walk_named_resources(ctx: &RangeWalk, resource_type: &str, ids: &[String], cancel_rx: &mut mpsc::Receiver<()>) -> Result<(), RunExit>`
  - `async fn walk_type(ctx: &Arc<RangeWalk>, resource_type: &str, request: &ReindexRequest, cancel_rx: &mut mpsc::Receiver<()>) -> Result<(), RunExit>`

- [ ] **Step 1: Record the baseline**

```bash
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
CARGO_BUILD_JOBS=4 cargo test -p helios-persistence --lib search::reindex 2>&1 | tail -5
```
Expected: `test result: ok. N passed; 0 failed`. Write `N` down; Step 3 must report the same `N`.

- [ ] **Step 2: Refactor**

(a) Replace `fn record_resource_failure` (keep its one-line doc comment) with:

```rust
/// Records a per-resource error against the job and in the log.
fn record_resource_failure(
    ctx: &RangeWalk,
    resource_type: &str,
    resource_id: &str,
    error: String,
    retryable: bool,
) {
    ctx.failures
        .lock()
        .record(resource_type, resource_id, &error, retryable);
    push_error(
        &ctx.jobs,
        &ctx.job_id,
        resource_type,
        resource_id,
        error,
        retryable,
    );
}
```

(b) Replace `fn write_resource_batch` and its doc comment with:

```rust
/// Rewrites one batch of resources through every writer and advances the
/// job's counters by `resources.len() + extra_processed`.
///
/// Page-at-a-time so a writer can wrap it in one transaction; each writer
/// reports a per-resource outcome for error attribution. The entry count for
/// progress comes from the writers' own extraction — the driver no longer
/// extracts a second time just to count. `extra_processed` accounts for rows
/// of the batch that were read but not written (skipped, or deleted since
/// they were named), and returns the batch's accounting for the run's log
/// lines (#1403).
async fn write_resource_batch(
    ctx: &RangeWalk,
    resource_type: &str,
    resources: &[StoredResource],
    extra_processed: u64,
) -> BatchOutcome {
    let mut wrote_any: Vec<bool> = vec![false; resources.len()];
    let mut entry_counts: Vec<u64> = vec![0; resources.len()];
    let mut batch = BatchOutcome::default();
    if !resources.is_empty() {
        for writer in &ctx.writers {
            let mut page_stats = ReindexPageStats::default();
            let started = Instant::now();
            let outcomes = writer
                .write_search_entries_page_timed(&ctx.tenant, resources, &mut page_stats)
                .await;
            batch.write += started.elapsed();
            batch.writer.accumulate(&page_stats);
            for (i, outcome) in outcomes.into_iter().enumerate() {
                match outcome {
                    Ok(written) => {
                        wrote_any[i] = true;
                        entry_counts[i] = entry_counts[i].max(written as u64);
                    }
                    Err(e) => {
                        batch.failed += 1;
                        record_resource_failure(
                            ctx,
                            resource_type,
                            resources[i].id(),
                            format!("Failed to rebuild index entries: {e}"),
                            is_transient_error(&e),
                        )
                    }
                }
            }
        }
    }

    let entries: u64 = wrote_any
        .iter()
        .zip(&entry_counts)
        .filter(|(w, _)| **w)
        .map(|(_, e)| e)
        .sum();
    batch.entries = entries;

    let mut jobs_guard = ctx.jobs.write();
    if let Some(progress) = jobs_guard.get_mut(&ctx.job_id) {
        progress.processed_resources += resources.len() as u64 + extra_processed;
        progress.entries_created += entries;
    }
    drop(jobs_guard);

    batch
}
```

(c) Replace `fn record_and_log_page` with:

```rust
/// Folds one page into the run's statistics, then logs its `reindex page`
/// line and, when one is due, a `reindex progress` line — after the
/// statistics lock is released, so concurrent walks never log under it
/// (#1403).
fn record_and_log_page_shared(ctx: &RangeWalk, resource_type: &str, record: PageRecord) {
    let recorded = ctx.stats.lock().record_page(record, Instant::now());
    log_page(
        &ctx.tenant_label,
        &ctx.job_id,
        resource_type,
        &recorded,
        &record,
    );
    if let Some(p) = &recorded.progress {
        log_progress(&ctx.tenant_label, &ctx.job_id, p);
    }
}
```

(d) Between `impl Drop for PrefetchedPage { .. }` and the doc comment of `async fn run_reindex`, add:

```rust
/// What one walk of a resource type needs, behind an `Arc` so the concurrent
/// walks of one type can share it (#1403). The run's failure log and
/// statistics sit behind short-lived locks: no guard is held across an
/// `.await`.
struct RangeWalk {
    tenant: TenantContext,
    tenant_label: String,
    job_id: String,
    source: Arc<dyn ReindexSource>,
    writers: Vec<Arc<dyn ReindexTarget>>,
    jobs: Arc<RwLock<HashMap<String, ReindexProgress>>>,
    /// Resources per page: the request's `batch_size`, at least 1.
    batch_size: u32,
    failures: parking_lot::Mutex<ResourceFailureLog>,
    stats: parking_lot::Mutex<ReindexRunStats>,
}

/// Rebuilds exactly the named resources of `resource_type`, fetched in
/// batches of the run's page size; an id deleted since it was named is
/// simply absent, and counts as processed with nothing to index (#1125).
async fn walk_named_resources(
    ctx: &RangeWalk,
    resource_type: &str,
    ids: &[String],
    cancel_rx: &mut mpsc::Receiver<()>,
) -> Result<(), RunExit> {
    for (batch_index, batch) in ids.chunks(ctx.batch_size as usize).enumerate() {
        if cancel_rx.try_recv().is_ok() {
            return Err(RunExit::Cancelled);
        }
        // Between two batches only, never before the first or after the
        // last: the gap exists to let a foreground writer take the lock, and
        // there is nothing to yield to once this run has stopped writing.
        if batch_index > 0 {
            let yielded = Instant::now();
            yield_between_pages().await;
            ctx.stats.lock().add_yield(yielded.elapsed());
        }
        let fetch_started = Instant::now();
        let fetch_span = crate::perf::span(crate::perf::Phase::ReindexFetch);
        let fetched = ctx
            .source
            .fetch_resources_by_ids(&ctx.tenant, resource_type, batch)
            .await;
        drop(fetch_span);
        let fetch_time = fetch_started.elapsed();
        let resources = match fetched {
            Ok(resources) => resources,
            Err(e) => {
                return Err(RunExit::Failed(format!("Failed to fetch resources: {e}")));
            }
        };
        let missing = (batch.len() as u64).saturating_sub(resources.len() as u64);
        let batch_outcome = write_resource_batch(ctx, resource_type, &resources, missing).await;
        record_and_log_page_shared(
            ctx,
            resource_type,
            PageRecord {
                resources: batch.len() as u64,
                entries: batch_outcome.entries,
                failed: batch_outcome.failed,
                fetch: fetch_time,
                fetch_wait: fetch_time,
                write: batch_outcome.write,
                writer: batch_outcome.writer,
            },
        );
    }
    Ok(())
}

/// Pages one walk of `resource_type` from `start` (`None` = the type's first
/// page) until the source returns no next cursor, rebuilding every page
/// through every writer (#1403). `should_stop` is asked once per page, before
/// the page is fetched or taken from the prefetch, so a page that has been
/// fetched is always written in full. The next page of this walk may be
/// fetched while this one is written, when the source allows it; the walk's
/// pages are still written one at a time, in fetch order. `page_bytes` caps a
/// page by bytes of stored content (`0` = count only).
async fn walk_range(
    ctx: &RangeWalk,
    resource_type: &str,
    start: Option<String>,
    page_bytes: u64,
    should_stop: &mut (dyn FnMut() -> bool + Send),
) -> Result<(), RunExit> {
    let mut cursor: Option<String> = start;
    let mut prefetched: Option<PrefetchedPage> = None;
    loop {
        if should_stop() {
            return Err(RunExit::Cancelled); // dropping `prefetched` aborts it
        }
        let wait_started = Instant::now();
        let (fetched, fetch, fetch_wait) = match prefetched.take() {
            Some(next) => {
                let (ahead, ahead_fetch) = next.wait().await;
                match ahead {
                    Ok(Some(page)) => (Ok(page), ahead_fetch, wait_started.elapsed()),
                    Ok(None) => {
                        // The source would not fetch this cursor ahead of the write
                        // (it ends a phase). The previous page's write has finished,
                        // so fetch it now.
                        let serial_started = Instant::now();
                        let fetch_span = crate::perf::span(crate::perf::Phase::ReindexFetch);
                        let fetched = ctx
                            .source
                            .fetch_resources_page_capped(
                                &ctx.tenant,
                                resource_type,
                                cursor.as_deref(),
                                ctx.batch_size,
                                page_bytes,
                            )
                            .await;
                        drop(fetch_span);
                        (
                            fetched,
                            ahead_fetch + serial_started.elapsed(),
                            wait_started.elapsed(),
                        )
                    }
                    Err(e) => (Err(e), ahead_fetch, wait_started.elapsed()),
                }
            }
            None => {
                let fetch_span = crate::perf::span(crate::perf::Phase::ReindexFetch);
                let fetched = ctx
                    .source
                    .fetch_resources_page_capped(
                        &ctx.tenant,
                        resource_type,
                        cursor.as_deref(),
                        ctx.batch_size,
                        page_bytes,
                    )
                    .await;
                drop(fetch_span);
                let fetch = wait_started.elapsed();
                (fetched, fetch, fetch) // exactly equal when nothing was prefetched
            }
        };
        let page = match fetched {
            Ok(page) => page,
            Err(e) => {
                return Err(RunExit::Failed(format!("Failed to fetch resources: {e}")));
            }
        };
        // Fetch the next page of THIS walk while this one is written, when the source allows it.
        if let Some(next) = page.next_cursor.as_deref()
            && ctx.source.may_prefetch_page(next)
        {
            prefetched = Some(PrefetchedPage::spawn(
                ctx.source.clone(),
                ctx.tenant.clone(),
                resource_type.to_string(),
                next.to_string(),
                ctx.batch_size,
                page_bytes,
            ));
        }
        // A row the source read but could not decode is a resource that
        // stays unsearchable until the row is repaired: a permanent
        // failure, recorded rather than silently dropped (#1125).
        for skipped in &page.skipped {
            record_resource_failure(
                ctx,
                resource_type,
                &skipped.resource_id,
                format!("Failed to read stored resource: {}", skipped.reason),
                false,
            );
        }

        // Rebuild the page through every writer.
        let batch_outcome = write_resource_batch(
            ctx,
            resource_type,
            &page.resources,
            page.skipped.len() as u64,
        )
        .await;

        record_and_log_page_shared(
            ctx,
            resource_type,
            PageRecord {
                resources: (page.resources.len() + page.skipped.len()) as u64,
                entries: batch_outcome.entries,
                failed: page.skipped.len() as u64 + batch_outcome.failed,
                fetch,
                fetch_wait,
                write: batch_outcome.write,
                writer: batch_outcome.writer,
            },
        );

        match page.next_cursor {
            Some(next) => {
                cursor = Some(next);
                // Stand back before re-taking the write lock for the next
                // page. Only between pages of one walk: the last page has no
                // successor to hold the lock against.
                let yielded = Instant::now();
                yield_between_pages().await;
                ctx.stats.lock().add_yield(yielded.elapsed());
            }
            None => return Ok(()),
        }
    }
}

/// Walks every page of `resource_type` (#1403).
async fn walk_type(
    ctx: &Arc<RangeWalk>,
    resource_type: &str,
    request: &ReindexRequest,
    cancel_rx: &mut mpsc::Receiver<()>,
) -> Result<(), RunExit> {
    walk_range(
        ctx,
        resource_type,
        None,
        request.batch_bytes,
        &mut || cancel_rx.try_recv().is_ok(),
    )
    .await
}
```

(e) In `run_reindex`:

1. Change `let resource_types = match (&named_resources, request.resource_types) {` to `let resource_types = match (&named_resources, request.resource_types.clone()) {`.
2. Replace everything from `stats.mark_pages_started(Instant::now());` down to and including the block
   ```rust
       failures.finish_type();
       if let Some(summary) = stats.finish_type(exit_outcome(&outcome), Instant::now()) {
           log_type_finished(&tenant_label, &job_id, &summary);
       }
   ```
   with:

```rust
    stats.mark_pages_started(Instant::now());
    let ctx = Arc::new(RangeWalk {
        tenant: tenant.clone(),
        tenant_label: tenant_label.clone(),
        job_id: job_id.clone(),
        source: source.clone(),
        writers: writers.clone(),
        jobs: jobs.clone(),
        batch_size: request.batch_size.max(1),
        failures: parking_lot::Mutex::new(ResourceFailureLog::new(&job_id, &tenant)),
        stats: parking_lot::Mutex::new(stats),
    });
    let outcome: Result<(), RunExit> = async {
        // Process each resource type
        for resource_type in &resource_types {
            // Check for cancellation
            if cancel_rx.try_recv().is_ok() {
                return Err(RunExit::Cancelled);
            }

            // Update current resource type
            {
                let mut jobs_guard = jobs.write();
                if let Some(progress) = jobs_guard.get_mut(&job_id) {
                    progress.current_resource_type = Some(resource_type.clone());
                }
            }
            ctx.failures.lock().start_type(resource_type);
            let type_total = type_totals
                .get(resource_type.as_str())
                .copied()
                .unwrap_or(0);
            let type_started = ctx
                .stats
                .lock()
                .start_type(resource_type, type_total, Instant::now());
            log_type_started(&tenant_label, &job_id, &type_started);

            match named_resources
                .as_ref()
                .and_then(|named| named.get(resource_type))
            {
                Some(ids) => walk_named_resources(&ctx, resource_type, ids, &mut cancel_rx).await?,
                None => walk_type(&ctx, resource_type, &request, &mut cancel_rx).await?,
            }
            let summary = ctx
                .stats
                .lock()
                .finish_type(OUTCOME_COMPLETED, Instant::now());
            if let Some(summary) = summary {
                log_type_finished(&tenant_label, &job_id, &summary);
            }
        }

        Ok(())
    }
    .await;
    ctx.failures.lock().finish_type();
    let summary = ctx
        .stats
        .lock()
        .finish_type(exit_outcome(&outcome), Instant::now());
    if let Some(summary) = summary {
        log_type_finished(&tenant_label, &job_id, &summary);
    }
```

3. After that point there are four `log_job_end(&stats, ...)` calls (the end-bulk-rebuild failure, `Err(RunExit::Cancelled)`, `Err(RunExit::Failed(msg))`, and the final completed one). Change each `&stats` to `&ctx.stats.lock()`. The two earlier `log_job_end(&stats, ..)` calls (clear-existing and begin-bulk-rebuild failures) run before `ctx` exists and stay as they are.

- [ ] **Step 3: Run the tests to verify nothing changed**

```bash
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
CARGO_BUILD_JOBS=4 cargo test -p helios-persistence --lib search::reindex 2>&1 | tail -5
```
Expected: `test result: ok. N passed; 0 failed` with the same `N` as Step 1 (this filter includes the SQLite `page_limit_tests`, the prefetch fakes and the log-contract tests).

- [ ] **Step 4: fmt, clippy, commit**

```bash
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
CARGO_BUILD_JOBS=4 cargo fmt -- crates/persistence/src/search/reindex.rs
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
CARGO_BUILD_JOBS=4 cargo clippy -p helios-persistence --all-targets --all-features -- -D warnings -A clippy::items_after_test_module -A clippy::large_enum_variant -A clippy::question_mark -A clippy::collapsible_match -A clippy::collapsible_if -A clippy::field_reassign_with_default -A clippy::doc-overindented-list-items -A clippy::doc-lazy-continuation 2>&1 | tail -30
git add crates/persistence/src/search/reindex.rs
git commit -m "$(cat <<'EOF'
refactor(persistence): move the reindex page loop into walk_range over a shared RangeWalk (#1403)

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Y61t4nTxKCT8WeE5NbxaVc
EOF
)"
```

---

### Task 4: Walk a split type's id ranges concurrently, then one catch-up

**Files:**
- Modify: `crates/persistence/src/search/reindex.rs` — `struct RangeWalk` and `fn walk_type` (Task 3); a new `walk_ranges`, `multi_stream_page_bytes`, `REINDEX_MULTI_STREAM_BATCH_BYTES`; the `Arc::new(RangeWalk { .. })` in `run_reindex`; `mod tests` (new fakes and tests, placed after `fn cancellation_during_a_page_finishes_it_and_stops_before_the_next_fetch`).
- Modify: `crates/persistence/src/search/reindex_stats.rs` — `impl ReindexRunStats` (after `fn add_yield`), `mod tests`.

**Interfaces:**
- Consumes: `TypeWalkPlan`, `TypeWalkRequest`, `ReindexSource::plan_type_walk`, `ReindexRequest.{write_streams, min_resources_per_stream, concurrent_runs, batch_bytes}` (Task 1); `RangeWalk`, `walk_range`, `walk_type` (Task 3).
- Produces:
  - `const REINDEX_MULTI_STREAM_BATCH_BYTES: u64 = 33_554_432;`
  - `RangeWalk.{stop: AtomicBool, cap_warned: AtomicBool}`
  - `async fn walk_ranges(ctx: &Arc<RangeWalk>, resource_type: &str, ranges: Vec<String>, catch_up: String, page_bytes: u64, cancel_rx: &mut mpsc::Receiver<()>) -> Result<(), RunExit>`
  - `fn multi_stream_page_bytes(ctx: &RangeWalk, batch_bytes: u64) -> u64`
  - `ReindexRunStats::set_type_plan(&mut self, streams: u32, plan: Duration)` (`pub(super)`)
  - Test fakes (in `mod tests`): `enum RangedPlan { Ranges, Single, NoRanges, Fail, Panic }`, `struct RangedSource` (records every `TypeWalkRequest` it is asked to plan in `requests`, and can hold `plan_type_walk` on `plan_gate`), `struct RangedWriter`, `fn range_pages(prefix: &str, pages: usize, per_page: usize) -> Vec<Vec<String>>`, `fn ranged_operation(source: Arc<RangedSource>, writer: Arc<RangedWriter>) -> Arc<ReindexOperation>`, `fn streams_request(streams: u32) -> ReindexRequest`, `fn new_seq() -> Arc<AtomicU64>`.
- Error texts (tests assert them): `"Failed to plan the walk of {resource_type}: {e}"`, `"The walk plan of {resource_type} has no id ranges"`, `"reindex stream ended without a result: {e}"`; a stream panic surfaces as the existing `"Reindex task panicked before completing"`.

- [ ] **Step 1: Write the failing tests**

In `crates/persistence/src/search/reindex_stats.rs`, `mod tests`, add:

```rust
    #[test]
    fn set_type_plan_is_reported_when_the_type_finishes() {
        let t0 = Instant::now();
        let mut stats = ReindexRunStats::new(t0, 1, 1, Duration::from_secs(60));
        stats.set_type_plan(4, Duration::from_millis(7)); // no type open: ignored
        stats.start_type("Observation", 1, t0);
        stats.set_type_plan(3, Duration::from_millis(12));
        let summary = stats
            .finish_type(OUTCOME_COMPLETED, t0 + Duration::from_secs(1))
            .unwrap();
        assert_eq!(summary.streams, 3);
        assert_eq!(summary.plan, Duration::from_millis(12));
    }
```

In `crates/persistence/src/search/reindex.rs`, `mod tests`, after `fn cancellation_during_a_page_finishes_it_and_stops_before_the_next_fetch`, add the fakes and tests:

```rust
    // --- #1403: concurrent write streams over id ranges ------------------

    /// How [`RangedSource::plan_type_walk`] answers (#1403).
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum RangedPlan {
        /// One range per entry of `ranges`, then the catch-up.
        Ranges,
        /// The single walk.
        Single,
        /// A `Ranges` plan with no range: a broken source.
        NoRanges,
        /// A planning error.
        Fail,
        /// Panics: the driver must not plan at all.
        Panic,
    }

    /// A scripted source for the multi-stream driver tests (#1403).
    /// `ranges[r]` holds range `r`'s pages of ids, walked from cursor
    /// `"r:{r}:1"`; `catch_up` holds the catch-up walk's pages, from
    /// `"catchup:1"`. The page after a walk's last is empty with no next
    /// cursor, as a source that ends a walk on an empty query returns it. A
    /// `None` cursor — the single walk — returns every range id in one page.
    /// Every fetch, serial or ahead, is recorded as `(seq, cursor, max_bytes)`
    /// from a sequence shared with [`RangedWriter`], so a test can order
    /// fetches against page writes. Every plan request is kept in
    /// `requests`, and `plan_gate`, when set, holds `plan_type_walk` until a
    /// permit is added.
    struct RangedSource {
        ranges: Vec<Vec<Vec<String>>>,
        catch_up: Vec<Vec<String>>,
        plan: RangedPlan,
        prefetch: bool,
        seq: Arc<std::sync::atomic::AtomicU64>,
        fetches: parking_lot::Mutex<Vec<(u64, String, u64)>>,
        fail: parking_lot::Mutex<std::collections::HashSet<String>>,
        panic: parking_lot::Mutex<std::collections::HashSet<String>>,
        plans: std::sync::atomic::AtomicUsize,
        requests: parking_lot::Mutex<Vec<TypeWalkRequest>>,
        plan_gate: Option<Arc<Semaphore>>,
    }

    /// `pages` pages of `per_page` ids each: `"{prefix}0"`, `"{prefix}1"`, …
    fn range_pages(prefix: &str, pages: usize, per_page: usize) -> Vec<Vec<String>> {
        (0..pages)
            .map(|p| {
                (0..per_page)
                    .map(|i| format!("{prefix}{}", p * per_page + i))
                    .collect()
            })
            .collect()
    }

    impl RangedSource {
        fn new(
            ranges: Vec<Vec<Vec<String>>>,
            catch_up: Vec<Vec<String>>,
            seq: Arc<std::sync::atomic::AtomicU64>,
        ) -> Self {
            Self {
                ranges,
                catch_up,
                plan: RangedPlan::Ranges,
                prefetch: false,
                seq,
                fetches: parking_lot::Mutex::new(Vec::new()),
                fail: parking_lot::Mutex::new(std::collections::HashSet::new()),
                panic: parking_lot::Mutex::new(std::collections::HashSet::new()),
                plans: std::sync::atomic::AtomicUsize::new(0),
                requests: parking_lot::Mutex::new(Vec::new()),
                plan_gate: None,
            }
        }

        fn with_plan(mut self, plan: RangedPlan) -> Self {
            self.plan = plan;
            self
        }

        fn with_prefetch(mut self) -> Self {
            self.prefetch = true;
            self
        }

        fn fail_cursor(&self, cursor: &str) {
            self.fail.lock().insert(cursor.to_string());
        }

        fn panic_cursor(&self, cursor: &str) {
            self.panic.lock().insert(cursor.to_string());
        }

        fn fetched(&self) -> Vec<(u64, String, u64)> {
            self.fetches.lock().clone()
        }

        fn fetched_cursors(&self, prefix: &str) -> Vec<String> {
            self.fetched()
                .into_iter()
                .map(|(_, cursor, _)| cursor)
                .filter(|cursor| cursor.starts_with(prefix))
                .collect()
        }

        fn total(&self) -> u64 {
            self.ranges
                .iter()
                .flatten()
                .map(|page| page.len() as u64)
                .sum()
        }

        fn page(&self, cursor: Option<&str>) -> (Vec<String>, Option<String>) {
            let Some(cursor) = cursor else {
                return (
                    self.ranges.iter().flatten().flatten().cloned().collect(),
                    None,
                );
            };
            let (pages, prefix, page) = if let Some(page) = cursor.strip_prefix("catchup:") {
                (&self.catch_up, "catchup:".to_string(), page)
            } else {
                let rest = cursor
                    .strip_prefix("r:")
                    .unwrap_or_else(|| panic!("unexpected cursor {cursor}"));
                let (range, page) = rest.split_once(':').expect("r:{range}:{page}");
                let range: usize = range.parse().expect("range index");
                (&self.ranges[range], format!("r:{range}:"), page)
            };
            let page: usize = page.parse().expect("page number");
            match pages.get(page - 1) {
                Some(ids) => (ids.clone(), Some(format!("{prefix}{}", page + 1))),
                None => (Vec::new(), None),
            }
        }
    }

    #[async_trait]
    impl ReindexSource for RangedSource {
        async fn list_resource_types(&self, _: &TenantContext) -> StorageResult<Vec<String>> {
            Ok(vec!["Observation".to_string()])
        }

        async fn count_resources(&self, _: &TenantContext, _: &str) -> StorageResult<u64> {
            Ok(self.total())
        }

        async fn fetch_resources_page(
            &self,
            tenant: &TenantContext,
            resource_type: &str,
            cursor: Option<&str>,
            limit: u32,
        ) -> StorageResult<ResourcePage> {
            self.fetch_resources_page_capped(tenant, resource_type, cursor, limit, 0)
                .await
        }

        async fn fetch_resources_page_capped(
            &self,
            tenant: &TenantContext,
            resource_type: &str,
            cursor: Option<&str>,
            _limit: u32,
            max_bytes: u64,
        ) -> StorageResult<ResourcePage> {
            let label = cursor.unwrap_or("<start>").to_string();
            let seq = self.seq.fetch_add(1, Ordering::SeqCst);
            self.fetches.lock().push((seq, label.clone(), max_bytes));
            if self.panic.lock().contains(&label) {
                panic!("RangedSource: scripted panic at {label}");
            }
            if self.fail.lock().contains(&label) {
                return Err(crate::error::BackendError::Internal {
                    backend_name: "ranged-source".into(),
                    message: format!("scripted fetch failure at {label}"),
                    source: None,
                }
                .into());
            }
            tokio::task::yield_now().await;
            let (ids, next_cursor) = self.page(cursor);
            Ok(ResourcePage {
                resources: ids
                    .into_iter()
                    .map(|id| {
                        StoredResource::new(
                            resource_type,
                            &id,
                            tenant.tenant_id().clone(),
                            serde_json::json!({"resourceType": resource_type, "id": id}),
                            helios_fhir::FhirVersion::default(),
                        )
                    })
                    .collect(),
                next_cursor,
                skipped: Vec::new(),
            })
        }

        fn may_prefetch_page(&self, cursor: &str) -> bool {
            self.prefetch && cursor.starts_with("r:")
        }

        async fn plan_type_walk(
            &self,
            _: &TenantContext,
            _: &str,
            request: TypeWalkRequest,
        ) -> StorageResult<TypeWalkPlan> {
            self.requests.lock().push(request);
            self.plans.fetch_add(1, Ordering::SeqCst);
            if let Some(gate) = &self.plan_gate {
                gate.acquire()
                    .await
                    .expect("ranged plan gate remains open")
                    .forget();
            }
            match self.plan {
                RangedPlan::Ranges => Ok(TypeWalkPlan::Ranges {
                    ranges: (0..self.ranges.len()).map(|r| format!("r:{r}:1")).collect(),
                    catch_up: "catchup:1".to_string(),
                }),
                RangedPlan::Single => Ok(TypeWalkPlan::Single),
                RangedPlan::NoRanges => Ok(TypeWalkPlan::Ranges {
                    ranges: Vec::new(),
                    catch_up: "catchup:1".to_string(),
                }),
                RangedPlan::Fail => Err(crate::error::BackendError::Unavailable {
                    backend_name: "ranged-source".into(),
                    message: "scripted plan failure".into(),
                }
                .into()),
                RangedPlan::Panic => panic!("plan_type_walk must not be called"),
            }
        }
    }

    /// Pairs with [`RangedSource`]: records each page write as `(seq, ids)`
    /// on the shared sequence when the write finishes, can hold each write on
    /// `gate` (one permit per page) or for `delay`, and rejects the ids in
    /// `reject` as permanent failures (#1403).
    struct RangedWriter {
        seq: Arc<std::sync::atomic::AtomicU64>,
        writes: parking_lot::Mutex<Vec<(u64, Vec<String>)>>,
        started: std::sync::atomic::AtomicUsize,
        ended: std::sync::atomic::AtomicUsize,
        gate: Option<Arc<Semaphore>>,
        delay: Duration,
        reject: BTreeSet<String>,
    }

    impl RangedWriter {
        fn new(seq: Arc<std::sync::atomic::AtomicU64>) -> Self {
            Self {
                seq,
                writes: parking_lot::Mutex::new(Vec::new()),
                started: std::sync::atomic::AtomicUsize::new(0),
                ended: std::sync::atomic::AtomicUsize::new(0),
                gate: None,
                delay: Duration::ZERO,
                reject: BTreeSet::new(),
            }
        }

        fn page_writes(&self) -> Vec<(u64, Vec<String>)> {
            self.writes.lock().clone()
        }
    }

    #[async_trait]
    impl ReindexTarget for RangedWriter {
        async fn delete_search_entries(
            &self,
            _: &TenantContext,
            _: &str,
            _: &str,
        ) -> StorageResult<u64> {
            Ok(0)
        }

        async fn write_search_entries(
            &self,
            _: &TenantContext,
            _: &StoredResource,
        ) -> StorageResult<usize> {
            Ok(1)
        }

        async fn clear_search_index(&self, _: &TenantContext) -> StorageResult<u64> {
            Ok(0)
        }

        async fn write_search_entries_page(
            &self,
            _: &TenantContext,
            resources: &[StoredResource],
        ) -> Vec<StorageResult<usize>> {
            self.started.fetch_add(1, Ordering::SeqCst);
            if let Some(gate) = &self.gate {
                gate.acquire()
                    .await
                    .expect("ranged write gate remains open")
                    .forget();
            }
            if !self.delay.is_zero() {
                tokio::time::sleep(self.delay).await;
            }
            let ids: Vec<String> = resources.iter().map(|r| r.id().to_string()).collect();
            let seq = self.seq.fetch_add(1, Ordering::SeqCst);
            self.writes.lock().push((seq, ids));
            self.ended.fetch_add(1, Ordering::SeqCst);
            resources
                .iter()
                .map(|r| {
                    if self.reject.contains(r.id()) {
                        Err(crate::error::BackendError::Internal {
                            backend_name: "ranged-writer".into(),
                            message: format!("scripted rejection of {}", r.id()),
                            source: None,
                        }
                        .into())
                    } else {
                        Ok(1)
                    }
                })
                .collect()
        }
    }

    fn ranged_operation(
        source: Arc<RangedSource>,
        writer: Arc<RangedWriter>,
    ) -> Arc<ReindexOperation> {
        Arc::new(ReindexOperation::with_parts(
            source,
            vec![writer as Arc<dyn ReindexTarget>],
            Arc::new(crate::search::TenantSearchRegistries::base_only()),
        ))
    }

    fn streams_request(streams: u32) -> ReindexRequest {
        ReindexRequest::for_types(vec!["Observation".to_string()])
            .with_batch_size(2)
            .with_write_streams(streams)
    }

    fn new_seq() -> Arc<std::sync::atomic::AtomicU64> {
        Arc::new(std::sync::atomic::AtomicU64::new(0))
    }

    #[tokio::test]
    async fn write_streams_one_never_plans() {
        let seq = new_seq();
        let source = Arc::new(
            RangedSource::new(
                vec![range_pages("a", 2, 2), range_pages("b", 2, 2)],
                Vec::new(),
                seq.clone(),
            )
            .with_plan(RangedPlan::Panic),
        );
        let op = ranged_operation(source.clone(), Arc::new(RangedWriter::new(seq)));
        let job = op
            .start(named_tenant("streams-one"), streams_request(1), None)
            .await
            .unwrap();
        let progress = await_finished(&op, &job).await;
        assert_eq!(progress.status, ReindexStatus::Completed, "{progress:?}");
        assert_eq!(progress.total_resources, 8);
        assert_eq!(progress.processed_resources, 8);
        assert_eq!(source.plans.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn single_plan_walks_the_type_once_under_write_streams() {
        let source = Arc::new(PagedSource::new(9));
        let target = Arc::new(RecordingTarget::default());
        let op = recording_operation(source.clone(), target.clone());
        let job = op
            .start(
                named_tenant("streams-single-plan"),
                ReindexRequest::for_types(vec!["Patient".to_string()])
                    .with_batch_size(2)
                    .with_write_streams(4),
                None,
            )
            .await
            .unwrap();
        let progress = await_finished(&op, &job).await;
        assert_eq!(progress.status, ReindexStatus::Completed, "{progress:?}");
        assert_eq!(progress.total_resources, 9);
        assert_eq!(progress.processed_resources, 9);
        assert_eq!(source.pages.load(Ordering::SeqCst), 5, "one walk, no double walk");
        assert_eq!(target.written.lock().len(), 9);
    }

    #[tokio::test]
    async fn ranges_run_then_catch_up_runs_last() {
        let seq = new_seq();
        let source = Arc::new(RangedSource::new(
            vec![
                range_pages("a", 2, 2),
                range_pages("b", 2, 2),
                range_pages("c", 2, 2),
            ],
            vec![vec!["z0".to_string()]],
            seq.clone(),
        ));
        let writer = Arc::new(RangedWriter::new(seq));
        let op = ranged_operation(source.clone(), writer.clone());
        let job = op
            .start(named_tenant("streams-order"), streams_request(3), None)
            .await
            .unwrap();
        let progress = await_finished(&op, &job).await;
        assert_eq!(progress.status, ReindexStatus::Completed, "{progress:?}");
        assert_eq!(progress.total_resources, 12);
        assert_eq!(
            progress.processed_resources, 13,
            "the catch-up's row is processed on top of the ranges'"
        );
        assert_eq!(source.plans.load(Ordering::SeqCst), 1);

        let last_range_write = writer
            .page_writes()
            .iter()
            .filter(|(_, ids)| !ids[0].starts_with('z'))
            .map(|(seq, _)| *seq)
            .max()
            .expect("range pages were written");
        let catch_up_fetches: Vec<u64> = source
            .fetched()
            .into_iter()
            .filter(|(_, cursor, _)| cursor.starts_with("catchup:"))
            .map(|(seq, _, _)| seq)
            .collect();
        assert_eq!(catch_up_fetches.len(), 2, "one page, then the empty one");
        assert!(
            catch_up_fetches.iter().all(|seq| *seq > last_range_write),
            "the catch-up must fetch only after every range page is written: \
             {catch_up_fetches:?} vs last range write {last_range_write}"
        );
        let mut written: Vec<String> = writer
            .page_writes()
            .into_iter()
            .flat_map(|(_, ids)| ids)
            .collect();
        written.sort();
        written.dedup();
        assert_eq!(written.len(), 13, "every id written exactly once");
    }

    #[tokio::test]
    async fn failed_range_fails_job_and_skips_catch_up() {
        let seq = new_seq();
        let source = Arc::new(RangedSource::new(
            vec![
                range_pages("a", 2, 2),
                range_pages("b", 2, 2),
                range_pages("c", 2, 2),
            ],
            vec![vec!["z0".to_string()]],
            seq.clone(),
        ));
        source.fail_cursor("r:1:2");
        let op = ranged_operation(source.clone(), Arc::new(RangedWriter::new(seq)));
        let job = op
            .start(named_tenant("streams-range-fails"), streams_request(3), None)
            .await
            .unwrap();
        let progress = await_finished(&op, &job).await;
        assert_eq!(progress.status, ReindexStatus::Failed, "{progress:?}");
        let message = progress.error_message.as_deref().unwrap_or("");
        assert!(
            message.starts_with("Failed to fetch resources:")
                && message.contains("scripted fetch failure at r:1:2"),
            "{message}"
        );
        assert!(source.fetched_cursors("catchup:").is_empty());
    }

    #[tokio::test]
    async fn first_real_error_wins_over_cancelled_streams() {
        let seq = new_seq();
        let source = Arc::new(RangedSource::new(
            vec![
                range_pages("a", 1, 2),
                range_pages("b", 40, 2),
                range_pages("c", 40, 2),
            ],
            vec![vec!["z0".to_string()]],
            seq.clone(),
        ));
        source.fail_cursor("r:0:1");
        let op = ranged_operation(source.clone(), Arc::new(RangedWriter::new(seq)));
        let job = op
            .start(named_tenant("streams-first-error"), streams_request(3), None)
            .await
            .unwrap();
        let progress = await_finished(&op, &job).await;
        assert_eq!(progress.status, ReindexStatus::Failed, "{progress:?}");
        let message = progress.error_message.as_deref().unwrap_or("");
        assert!(message.contains("scripted fetch failure at r:0:1"), "{message}");
        let fetched = source.fetched_cursors("r:");
        assert!(
            !fetched.iter().any(|c| c == "r:1:41" || c == "r:2:41"),
            "the other streams must stop at a page boundary, long before their end: {fetched:?}"
        );
        assert!(source.fetched_cursors("catchup:").is_empty());
    }

    #[tokio::test]
    async fn cancel_during_streams_finishes_in_flight_pages_and_skips_catch_up() {
        let seq = new_seq();
        let source = Arc::new(RangedSource::new(
            vec![
                range_pages("a", 2, 2),
                range_pages("b", 2, 2),
                range_pages("c", 2, 2),
            ],
            vec![vec!["z0".to_string()]],
            seq.clone(),
        ));
        let gate = Arc::new(Semaphore::new(0));
        let writer = Arc::new(RangedWriter {
            gate: Some(gate.clone()),
            ..RangedWriter::new(seq)
        });
        let op = ranged_operation(source.clone(), writer.clone());
        let job = op
            .start(named_tenant("streams-cancel"), streams_request(3), None)
            .await
            .unwrap();

        // Every stream is inside its first page's write, held by the gate.
        tokio::time::timeout(Duration::from_secs(2), async {
            while writer.started.load(Ordering::SeqCst) < 3 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("all three streams must reach their first page write");

        op.cancel(&job).await.expect("cancel");
        gate.add_permits(3);

        // The task releases its cancellation channel when it returns: the
        // only signal that every stream has stopped for good.
        tokio::time::timeout(Duration::from_secs(2), async {
            while op.cancel_channels.read().contains_key(&job) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the cancelled reindex task did not return");

        assert_eq!(
            writer.ended.load(Ordering::SeqCst),
            3,
            "each page in flight is written in full"
        );
        assert_eq!(
            source.fetched_cursors("r:").len(),
            3,
            "no stream fetches past the page it was writing"
        );
        assert!(source.fetched_cursors("catchup:").is_empty());
        let progress = op.get_progress(&job).await.expect("progress");
        assert_eq!(progress.status, ReindexStatus::Cancelled);
        assert_eq!(progress.processed_resources, 6);
    }

    #[tokio::test]
    async fn stream_panic_fails_job_like_a_serial_panic() {
        let seq = new_seq();
        let source = Arc::new(RangedSource::new(
            vec![
                range_pages("a", 3, 2),
                range_pages("b", 3, 2),
                range_pages("c", 3, 2),
            ],
            vec![vec!["z0".to_string()]],
            seq.clone(),
        ));
        source.panic_cursor("r:1:1");
        let writer = Arc::new(RangedWriter {
            delay: Duration::from_millis(20),
            ..RangedWriter::new(seq)
        });
        let op = ranged_operation(source.clone(), writer.clone());
        let job = op
            .start(named_tenant("streams-panic"), streams_request(3), None)
            .await
            .unwrap();
        let progress = await_finished(&op, &job).await;
        assert_eq!(progress.status, ReindexStatus::Failed, "{progress:?}");
        assert_eq!(
            progress.error_message.as_deref(),
            Some("Reindex task panicked before completing")
        );
        let started = writer.started.load(Ordering::SeqCst);
        assert!(started >= 1, "another stream was writing when range b panicked");
        assert_eq!(
            writer.ended.load(Ordering::SeqCst),
            started,
            "the driver must let the other streams finish their pages before failing"
        );
        assert!(source.fetched_cursors("catchup:").is_empty());
    }

    #[tokio::test]
    async fn resource_failure_attribution_under_streams() {
        let seq = new_seq();
        let source = Arc::new(RangedSource::new(
            vec![
                range_pages("a", 2, 2),
                range_pages("b", 2, 2),
                range_pages("c", 2, 2),
            ],
            Vec::new(),
            seq.clone(),
        ));
        let writer = Arc::new(RangedWriter {
            reject: BTreeSet::from(["c1".to_string()]),
            ..RangedWriter::new(seq)
        });
        let op = ranged_operation(source, writer);
        let job = op
            .start(named_tenant("streams-attribution"), streams_request(3), None)
            .await
            .unwrap();
        let progress = await_finished(&op, &job).await;
        assert_eq!(progress.status, ReindexStatus::Completed, "{progress:?}");
        assert_eq!(progress.errors.len(), 1, "{:?}", progress.errors);
        assert_eq!(progress.errors[0].resource_type, "Observation");
        assert_eq!(progress.errors[0].resource_id, "c1");
        assert!(!progress.errors[0].retryable);
    }

    #[tokio::test]
    async fn multi_stream_types_cap_uncapped_pages() {
        async fn fetched_caps(plan: RangedPlan, batch_bytes: u64, tenant: &str) -> Vec<u64> {
            let seq = new_seq();
            let source = Arc::new(
                RangedSource::new(
                    vec![range_pages("a", 2, 2), range_pages("b", 2, 2)],
                    vec![vec!["z0".to_string()]],
                    seq.clone(),
                )
                .with_plan(plan),
            );
            let op = ranged_operation(source.clone(), Arc::new(RangedWriter::new(seq)));
            let job = op
                .start(
                    named_tenant(tenant),
                    streams_request(4).with_batch_bytes(batch_bytes),
                    None,
                )
                .await
                .unwrap();
            let progress = await_finished(&op, &job).await;
            assert_eq!(progress.status, ReindexStatus::Completed, "{progress:?}");
            source
                .fetched()
                .into_iter()
                .map(|(_, _, max_bytes)| max_bytes)
                .collect()
        }

        let ranged = fetched_caps(RangedPlan::Ranges, 0, "streams-cap-ranges").await;
        assert!(
            !ranged.is_empty() && ranged.iter().all(|b| *b == 33_554_432),
            "range and catch-up pages of a split type are never uncapped: {ranged:?}"
        );
        let single = fetched_caps(RangedPlan::Single, 0, "streams-cap-single").await;
        assert_eq!(single, vec![0], "a single walk keeps the run's own cap");
        let explicit = fetched_caps(RangedPlan::Ranges, 4096, "streams-cap-explicit").await;
        assert!(explicit.iter().all(|b| *b == 4096), "{explicit:?}");
    }

    #[tokio::test]
    async fn type_finished_line_reports_streams_and_plan_ms() {
        let (_guard, events) = capture_contract();
        let seq = new_seq();
        let source = Arc::new(RangedSource::new(
            vec![
                range_pages("a", 2, 2),
                range_pages("b", 2, 2),
                range_pages("c", 2, 2),
            ],
            vec![vec!["z0".to_string()]],
            seq.clone(),
        ));
        let op = ranged_operation(source, Arc::new(RangedWriter::new(seq)));
        let job = op
            .start(named_tenant("streams-contract"), streams_request(4), None)
            .await
            .unwrap();
        let progress = await_finished(&op, &job).await;
        assert_eq!(progress.status, ReindexStatus::Completed, "{progress:?}");

        let events = events.lock().unwrap().clone();
        assert_contract(&events);
        let job_started = events
            .iter()
            .find(|e| e.message == "reindex job started")
            .expect("reindex job started");
        assert_eq!(job_started.values["write_streams"], "4");
        let type_finished = events
            .iter()
            .find(|e| e.message == "reindex type finished")
            .expect("reindex type finished");
        assert_eq!(type_finished.values["streams"], "3");
        assert!(type_finished.values.contains_key("plan_ms"));
        let pages = events.iter().filter(|e| e.message == "reindex page").count();
        assert_eq!(type_finished.values["pages"], pages.to_string());
    }

    #[tokio::test]
    async fn a_plan_without_ranges_fails_the_type() {
        let seq = new_seq();
        let source = Arc::new(
            RangedSource::new(vec![range_pages("a", 1, 2)], Vec::new(), seq.clone())
                .with_plan(RangedPlan::NoRanges),
        );
        let op = ranged_operation(source.clone(), Arc::new(RangedWriter::new(seq)));
        let job = op
            .start(named_tenant("streams-no-ranges"), streams_request(4), None)
            .await
            .unwrap();
        let progress = await_finished(&op, &job).await;
        assert_eq!(progress.status, ReindexStatus::Failed, "{progress:?}");
        assert_eq!(
            progress.error_message.as_deref(),
            Some("The walk plan of Observation has no id ranges")
        );
        assert!(source.fetched().is_empty(), "nothing is fetched");
    }

    #[tokio::test]
    async fn a_failed_plan_fails_the_job() {
        let seq = new_seq();
        let source = Arc::new(
            RangedSource::new(vec![range_pages("a", 1, 2)], Vec::new(), seq.clone())
                .with_plan(RangedPlan::Fail),
        );
        let op = ranged_operation(source.clone(), Arc::new(RangedWriter::new(seq)));
        let job = op
            .start(named_tenant("streams-plan-fails"), streams_request(4), None)
            .await
            .unwrap();
        let progress = await_finished(&op, &job).await;
        assert_eq!(progress.status, ReindexStatus::Failed, "{progress:?}");
        let message = progress.error_message.as_deref().unwrap_or("");
        assert!(
            message.starts_with("Failed to plan the walk of Observation: backend unavailable"),
            "{message}"
        );
        assert!(source.fetched().is_empty(), "nothing is fetched");
    }

    #[tokio::test]
    async fn ranges_prefetch_their_own_next_page_and_write_in_fetch_order() {
        let seq = new_seq();
        let source = Arc::new(
            RangedSource::new(
                vec![
                    range_pages("a", 3, 2),
                    range_pages("b", 3, 2),
                    range_pages("c", 3, 2),
                ],
                Vec::new(),
                seq.clone(),
            )
            .with_prefetch(),
        );
        let writer = Arc::new(RangedWriter::new(seq));
        let op = ranged_operation(source.clone(), writer.clone());
        let job = op
            .start(named_tenant("streams-prefetch"), streams_request(3), None)
            .await
            .unwrap();
        let progress = await_finished(&op, &job).await;
        assert_eq!(progress.status, ReindexStatus::Completed, "{progress:?}");
        assert_eq!(progress.processed_resources, 18);

        let cursors: Vec<String> = source
            .fetched()
            .into_iter()
            .map(|(_, cursor, _)| cursor)
            .collect();
        let unique: BTreeSet<&String> = cursors.iter().collect();
        assert_eq!(unique.len(), cursors.len(), "a page was fetched twice: {cursors:?}");
        for prefix in ["a", "b", "c"] {
            let firsts: Vec<String> = writer
                .page_writes()
                .into_iter()
                .filter(|(_, ids)| ids[0].starts_with(prefix))
                .map(|(_, ids)| ids[0].clone())
                .collect();
            assert_eq!(
                firsts,
                vec![
                    format!("{prefix}0"),
                    format!("{prefix}2"),
                    format!("{prefix}4")
                ],
                "range {prefix} must be written in fetch order"
            );
        }
    }

    #[tokio::test]
    async fn the_automatic_hook_plans_with_its_write_streams_and_concurrency() {
        let seq = new_seq();
        let source = Arc::new(RangedSource::new(
            vec![range_pages("a", 1, 2), range_pages("b", 1, 2)],
            Vec::new(),
            seq.clone(),
        ));
        let writer = Arc::new(RangedWriter::new(seq));
        let op = ranged_operation(source.clone(), writer.clone());
        // The production path: hook -> coordinator -> `GenerationScope::request`
        // -> the driver's `TypeWalkRequest`.
        let hook = ReindexOnFinish::with_max_concurrency(op.clone(), 2).with_write_streams(4);
        hook.reindex_types(
            &named_tenant("streams-automatic"),
            vec!["Observation".to_string()],
        )
        .await;

        tokio::time::timeout(Duration::from_secs(2), async {
            while source.plans.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the automatic generation must plan the type");
        await_automatic_idle(&op).await;

        assert_eq!(
            source.requests.lock().clone(),
            vec![TypeWalkRequest {
                streams: 4,
                min_resources_per_stream: DEFAULT_MIN_RESOURCES_PER_STREAM,
                concurrent_runs: 2,
            }]
        );
        let mut written: Vec<String> = writer
            .page_writes()
            .into_iter()
            .flat_map(|(_, ids)| ids)
            .collect();
        written.sort();
        assert_eq!(written, ["a0", "a1", "b0", "b1"], "the generation ran to its end");
    }

    #[tokio::test]
    async fn write_streams_above_the_maximum_plan_at_the_maximum() {
        let seq = new_seq();
        let source = Arc::new(RangedSource::new(
            vec![range_pages("a", 1, 2), range_pages("b", 1, 2)],
            Vec::new(),
            seq.clone(),
        ));
        let op = ranged_operation(source.clone(), Arc::new(RangedWriter::new(seq)));
        // A deserialized request bypasses `with_write_streams`' clamp.
        let mut request = streams_request(1);
        request.write_streams = 40;
        let job = op
            .start(named_tenant("streams-over-max"), request, None)
            .await
            .unwrap();
        let progress = await_finished(&op, &job).await;
        assert_eq!(progress.status, ReindexStatus::Completed, "{progress:?}");
        let planned: Vec<u32> = source.requests.lock().iter().map(|r| r.streams).collect();
        assert_eq!(planned, vec![REINDEX_MAX_WRITE_STREAMS]);
    }

    #[tokio::test]
    async fn cancel_during_planning_fetches_no_range() {
        let seq = new_seq();
        let gate = Arc::new(Semaphore::new(0));
        let source = Arc::new(RangedSource {
            plan_gate: Some(gate.clone()),
            ..RangedSource::new(
                vec![range_pages("a", 2, 2), range_pages("b", 2, 2)],
                vec![vec!["z0".to_string()]],
                seq.clone(),
            )
        });
        let op = ranged_operation(source.clone(), Arc::new(RangedWriter::new(seq)));
        let job = op
            .start(named_tenant("streams-cancel-plan"), streams_request(2), None)
            .await
            .unwrap();

        tokio::time::timeout(Duration::from_secs(2), async {
            while source.plans.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the driver must reach plan_type_walk");
        op.cancel(&job).await.expect("cancel");
        gate.add_permits(1);

        // The task releases its cancellation channel when it returns.
        tokio::time::timeout(Duration::from_secs(2), async {
            while op.cancel_channels.read().contains_key(&job) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the cancelled reindex task did not return");

        // On this current-thread runtime the streams' own first stop check
        // would also catch the cancel; the check before spawning matters on
        // a multi-thread runtime, where a spawned stream can fetch before the
        // driver polls the cancellation. This pins the observable outcome.
        assert!(source.fetched().is_empty(), "{:?}", source.fetched());
        let progress = op.get_progress(&job).await.expect("progress");
        assert_eq!(progress.status, ReindexStatus::Cancelled);
        assert_eq!(progress.processed_resources, 0);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
CARGO_BUILD_JOBS=4 cargo test -p helios-persistence --lib search::reindex 2>&1 | tail -40
```
Expected: compile error `no method named set_type_plan found for struct ReindexRunStats`. (Once it compiles, `ranges_run_then_catch_up_runs_last` and the other range tests fail: `walk_type` still ignores `write_streams`.)

- [ ] **Step 3: Implement**

(a) `reindex_stats.rs`, in `impl ReindexRunStats` after `pub(super) fn add_yield`:

```rust
    /// Records how the open type is walked: its write streams and the time
    /// spent planning them (#1403). Does nothing when no type is open.
    pub(super) fn set_type_plan(&mut self, streams: u32, plan: Duration) {
        if let Some(open) = self.current.as_mut() {
            open.streams = streams;
            open.plan = plan;
        }
    }
```

(b) `reindex.rs`, above `struct RangeWalk`:

```rust
/// Byte cap of a split type's pages when the run's `batch_bytes` is 0 (count
/// only): each of K concurrent walks holds up to two pages, so those pages
/// are never left uncapped (#1403). Equal to the server default of
/// `HFS_REINDEX_BATCH_BYTES`.
const REINDEX_MULTI_STREAM_BATCH_BYTES: u64 = 33_554_432;
```

In `struct RangeWalk`, after `stats`, add:

```rust
    /// Raised to stop the concurrent walks of a split type at their next page
    /// boundary: on cancellation, or once one of them has failed.
    stop: AtomicBool,
    /// Whether this run has already warned that it capped a split type's
    /// pages because `batch_bytes` was 0.
    cap_warned: AtomicBool,
```

and in `run_reindex`'s `Arc::new(RangeWalk { .. })`, after `stats: parking_lot::Mutex::new(stats),` add:

```rust
        stop: AtomicBool::new(false),
        cap_warned: AtomicBool::new(false),
```

(c) Replace `async fn walk_type` (Task 3's version) with:

```rust
/// Walks every page of `resource_type` (#1403): one walk when the run asks
/// for a single write stream or the source keeps the type whole; otherwise
/// the source's id ranges concurrently, one stream each, then one catch-up
/// walk once every range has finished.
async fn walk_type(
    ctx: &Arc<RangeWalk>,
    resource_type: &str,
    request: &ReindexRequest,
    cancel_rx: &mut mpsc::Receiver<()>,
) -> Result<(), RunExit> {
    // `ReindexRequest` is public and deserialisable, so its setter's clamp
    // can be bypassed: clamp again here.
    let write_streams = request.write_streams.min(REINDEX_MAX_WRITE_STREAMS);
    if write_streams <= 1 {
        return walk_range(
            ctx,
            resource_type,
            None,
            request.batch_bytes,
            &mut || cancel_rx.try_recv().is_ok(),
        )
        .await;
    }
    let plan_started = Instant::now();
    let plan = ctx
        .source
        .plan_type_walk(
            &ctx.tenant,
            resource_type,
            TypeWalkRequest {
                streams: write_streams,
                min_resources_per_stream: request.min_resources_per_stream,
                concurrent_runs: request.concurrent_runs,
            },
        )
        .await
        .map_err(|e| RunExit::Failed(format!("Failed to plan the walk of {resource_type}: {e}")))?;
    // A broken plan fails before it is recorded, so the type's L3 line never
    // reports `streams=0`.
    let streams = match &plan {
        TypeWalkPlan::Single => 1,
        TypeWalkPlan::Ranges { ranges, .. } if ranges.is_empty() => {
            return Err(RunExit::Failed(format!(
                "The walk plan of {resource_type} has no id ranges"
            )));
        }
        TypeWalkPlan::Ranges { ranges, .. } => u32::try_from(ranges.len()).unwrap_or(u32::MAX),
    };
    ctx.stats
        .lock()
        .set_type_plan(streams, plan_started.elapsed());
    match plan {
        TypeWalkPlan::Single => {
            walk_range(
                ctx,
                resource_type,
                None,
                request.batch_bytes,
                &mut || cancel_rx.try_recv().is_ok(),
            )
            .await
        }
        TypeWalkPlan::Ranges { ranges, catch_up } => {
            let page_bytes = multi_stream_page_bytes(ctx, request.batch_bytes);
            walk_ranges(ctx, resource_type, ranges, catch_up, page_bytes, cancel_rx).await
        }
    }
}

/// A split type's page byte cap: the run's own, or
/// [`REINDEX_MULTI_STREAM_BATCH_BYTES`] when the run asked for none, with one
/// warning per run (#1403).
fn multi_stream_page_bytes(ctx: &RangeWalk, batch_bytes: u64) -> u64 {
    if batch_bytes > 0 {
        return batch_bytes;
    }
    if !ctx.cap_warned.swap(true, Ordering::SeqCst) {
        tracing::warn!(
            tenant = %ctx.tenant_label,
            job_id = %ctx.job_id,
            "HFS_REINDEX_BATCH_BYTES=0 with concurrent streams; capping rebuild pages at 32 MiB"
        );
    }
    REINDEX_MULTI_STREAM_BATCH_BYTES
}

/// Walks every range of a split type concurrently, one stream each, then the
/// catch-up (#1403).
///
/// Every stream is drained before this returns: the set is never dropped,
/// aborted or left early, because dropping it would abort a stream between a
/// page's delete and its insert. A cancellation, a failure or a panic raises
/// `stop`, which each stream checks at its next page boundary, so a page
/// that has been fetched is always written in full. Then, in order of
/// precedence: a panic resumes here (the job's own unwind guard fails it, as
/// for a single walk); the first failure's message fails the type; a
/// cancellation cancels it. Only when every range completed does the
/// catch-up walk run, so it starts after the last range page is written. A
/// cancel that arrived while the type was being planned returns before any
/// stream is spawned.
async fn walk_ranges(
    ctx: &Arc<RangeWalk>,
    resource_type: &str,
    ranges: Vec<String>,
    catch_up: String,
    page_bytes: u64,
    cancel_rx: &mut mpsc::Receiver<()>,
) -> Result<(), RunExit> {
    if cancel_rx.try_recv().is_ok() {
        return Err(RunExit::Cancelled);
    }
    ctx.stop.store(false, Ordering::SeqCst);
    let mut set: tokio::task::JoinSet<Result<(), RunExit>> = tokio::task::JoinSet::new();
    for cursor in ranges {
        let ctx = ctx.clone();
        let resource_type = resource_type.to_string();
        set.spawn(async move {
            let mut stop = || ctx.stop.load(Ordering::SeqCst);
            walk_range(&ctx, &resource_type, Some(cursor), page_bytes, &mut stop).await
        });
    }

    let mut cancelled = false;
    let mut failed: Option<String> = None;
    let mut panicked: Option<Box<dyn std::any::Any + Send + 'static>> = None;
    loop {
        tokio::select! {
            biased;
            // A closed channel yields `None`, which is not a cancellation.
            Some(()) = cancel_rx.recv(), if !cancelled => {
                cancelled = true;
                ctx.stop.store(true, Ordering::SeqCst);
            }
            joined = set.join_next() => match joined {
                None => break,
                Some(Ok(Ok(()))) | Some(Ok(Err(RunExit::Cancelled))) => {}
                Some(Ok(Err(RunExit::Failed(message)))) => {
                    if failed.is_none() {
                        failed = Some(message);
                    }
                    ctx.stop.store(true, Ordering::SeqCst);
                }
                Some(Err(e)) if e.is_panic() => {
                    if panicked.is_none() {
                        panicked = Some(e.into_panic());
                    }
                    ctx.stop.store(true, Ordering::SeqCst);
                }
                Some(Err(e)) => {
                    if failed.is_none() {
                        failed = Some(format!("reindex stream ended without a result: {e}"));
                    }
                    ctx.stop.store(true, Ordering::SeqCst);
                }
            }
        }
    }

    if let Some(payload) = panicked {
        std::panic::resume_unwind(payload);
    }
    if let Some(message) = failed {
        return Err(RunExit::Failed(message));
    }
    if cancelled {
        return Err(RunExit::Cancelled);
    }
    walk_range(
        ctx,
        resource_type,
        Some(catch_up),
        page_bytes,
        &mut || cancel_rx.try_recv().is_ok(),
    )
    .await
}
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
CARGO_BUILD_JOBS=4 cargo test -p helios-persistence --lib search::reindex 2>&1 | tail -40
```
Expected: `test result: ok.` — the sixteen new driver tests (from `write_streams_one_never_plans` to `cancel_during_planning_fetches_no_range`), the new stats test, and every existing test (in particular `cancellation_during_a_page_finishes_it_and_stops_before_the_next_fetch`, which runs the one-stream path). Run the same command twice more; the stream tests must pass every time (report any flake instead of retrying past it).

- [ ] **Step 5: fmt, clippy, commit**

```bash
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
CARGO_BUILD_JOBS=4 cargo fmt -- crates/persistence/src/search/reindex.rs crates/persistence/src/search/reindex_stats.rs
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
CARGO_BUILD_JOBS=4 cargo clippy -p helios-persistence --all-targets --all-features -- -D warnings -A clippy::items_after_test_module -A clippy::large_enum_variant -A clippy::question_mark -A clippy::collapsible_match -A clippy::collapsible_if -A clippy::field_reassign_with_default -A clippy::doc-overindented-list-items -A clippy::doc-lazy-continuation 2>&1 | tail -30
git add crates/persistence/src/search/reindex.rs crates/persistence/src/search/reindex_stats.rs
git commit -m "$(cat <<'EOF'
feat(persistence): walk a split type's id ranges concurrently, then one catch-up (#1403)

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Y61t4nTxKCT8WeE5NbxaVc
EOF
)"
```

---

### Task 5: MongoDB id-range and id-phase-done cursors

**Files:**
- Modify: `crates/persistence/src/backends/mongodb/storage.rs` — the `impl MongoBackend` walk block (`reindex_newest_live_last_updated` `:4788`, `reindex_id_page` `:4871-4923`, `fetch_reindex_page` `:4926-5130`); `enum WalkStep` and `impl From<ReindexWalkCursor> for WalkStep` (`:5133-5178`); `impl ReindexSource for MongoBackend`'s `may_prefetch_page` and `fetch_resources_page_ahead` (`:5264-5306`); `enum ReindexWalkCursor` and its `impl` (`:5572-5690`); after `fn reindex_id_page_filter` (`:5753-5770`); `mod reindex_walk_tests` (`:6188`) and `mod reindex_prefetch_tests` (`:6568-6670`).
- Modify: `crates/persistence/tests/mongodb/reindex_id_walk.rs` — `struct WalkFixture` (`:24-27`), `async fn seed_walk_fixture` (`:34`), `async fn backdate_fixture` (`:137`), `async fn snapshot` (`:390`).
- Modify: `crates/persistence/tests/mongodb_tests.rs` — after `mod reindex_pipeline;` (`:826-827`).
- Create: `crates/persistence/tests/mongodb/reindex_streams.rs`.

**Interfaces:**
- Consumes: `reindex_find_page(&self, resources: &Collection<Document>, filter: Document, sort: Document, hint: &str, limit: u32, max_bytes: u64) -> StorageResult<ReindexFoundPage>`, `log_capped_page_read`, `reindex_page_from_docs(docs: &[Document], resource_type: &str, tenant: &TenantContext, next_cursor: ReindexWalkCursor) -> StorageResult<ResourcePage>`, `reindex_newest_live_last_updated(&self, resources: &Collection<Document>, tenant_id: &str, resource_type: &str)`, `reindex_catch_up_margin(u64)`, `reindex_catch_up_floor(t0, newest_live, margin)`, `format_walk_instant`, `chrono_to_bson`, `RESOURCES_IDENTITY_INDEX` (all existing, private to `storage.rs`).
- Produces (private to `storage.rs`):
  - `struct ReindexIdRange { floor: DateTime<Utc>, lo: Option<String>, hi: Option<String> }` with `fn admits(&self, after_id: Option<&str>) -> bool`
  - `ReindexWalkCursor::IdRange { range: ReindexIdRange, after_id: Option<String> }` ⇄ `v2|r|<floor>|<lo>|<hi>|<after_id>`; `ReindexWalkCursor::IdPhaseDone { floor: DateTime<Utc> }` ⇄ `v2|d|<floor>`
  - `WalkStep::IdRange { range, after_id }`, `WalkStep::IdPhaseDone { floor }`
  - `fn reindex_id_range_page_filter(tenant_id: &str, resource_type: &str, floor: DateTime<Utc>, lo: Option<&str>, hi: Option<&str>, after_id: Option<&str>) -> Document`
  - `async fn reindex_id_range_page(&self, tenant: &TenantContext, resource_type: &str, range: &ReindexIdRange, after_id: Option<&str>, limit: u32, max_bytes: u64) -> StorageResult<ResourcePage>`
  - `fn reindex_margin(&self) -> chrono::Duration`; `async fn reindex_walk_floor(&self, resources: &Collection<Document>, tenant_id: &str, resource_type: &str) -> StorageResult<DateTime<Utc>>`
- Produces (tests): `pub(super)` on `WalkFixture` + its `live` field, `seed_walk_fixture`, `backdate_fixture`, `snapshot`; the new module `reindex_streams` with `RANGE_FLOOR`, `range_cursor(lo, hi)`, `ids_of(&ResourcePage)`, `walk_ids(backend, tenant, cursor, limit)`.

- [ ] **Step 1: Write the failing unit tests**

In `storage.rs`, `mod reindex_walk_tests`, after `fn cursor_rejects_foreign_and_malformed_tokens`, add:

```rust
    // --- Id-range and id-phase-done cursors (#1403) ---

    fn id_range(lo: Option<&str>, hi: Option<&str>) -> ReindexIdRange {
        ReindexIdRange {
            floor: ts("2026-01-01T00:00:00.000Z"),
            lo: lo.map(str::to_string),
            hi: hi.map(str::to_string),
        }
    }

    #[test]
    fn id_range_cursor_round_trips() {
        for (lo, hi, after_id) in [
            (None, None, None),
            (None, Some("m"), Some("a-1")),
            (Some("A.1"), Some("m"), Some("Zz-9.x")),
            (Some("m"), None, None),
            (Some("m"), None, Some("m")),
        ] {
            let cursor = ReindexWalkCursor::IdRange {
                range: id_range(lo, hi),
                after_id: after_id.map(str::to_string),
            };
            assert_eq!(ReindexWalkCursor::parse(&cursor.encode()).unwrap(), cursor);
        }
        assert_eq!(
            ReindexWalkCursor::IdRange {
                range: id_range(None, Some("obs-020")),
                after_id: None,
            }
            .encode(),
            "v2|r|2026-01-01T00:00:00.000Z||obs-020|"
        );
    }

    #[test]
    fn id_range_cursor_rejects_bad_bounds_and_shapes() {
        let floor = "2026-01-01T00:00:00.000Z";
        let bad: Vec<String> = vec![
            format!("v2|r|{floor}|m|m|"),  // lo == hi
            format!("v2|r|{floor}|t|m|"),  // lo > hi
            format!("v2|r|{floor}|m|t|a"), // after_id below lo
            format!("v2|r|{floor}|m|t|t"), // after_id == hi
            format!("v2|r|{floor}||m|z"),  // after_id above hi
            format!("v2|r|{floor}|m|t"),   // three fields
            format!("v2|r|{floor}"),       // one field
            "v2|r|not-a-time|||".to_string(),
            "v2|r||||".to_string(), // empty floor
        ];
        for cursor in bad {
            match ReindexWalkCursor::parse(&cursor) {
                Err(StorageError::Search(SearchError::InvalidCursor { .. })) => {}
                other => panic!("expected InvalidCursor for {cursor:?}, got {other:?}"),
            }
        }
    }

    #[test]
    fn id_phase_done_cursor_round_trips_and_rejects_extra_fields() {
        let cursor = ReindexWalkCursor::IdPhaseDone {
            floor: ts("2026-01-01T00:00:00.123Z"),
        };
        assert_eq!(cursor.encode(), "v2|d|2026-01-01T00:00:00.123Z");
        assert_eq!(ReindexWalkCursor::parse(&cursor.encode()).unwrap(), cursor);
        for bad in [
            "v2|d",
            "v2|d|",
            "v2|d|not-a-time",
            "v2|d|2026-01-01T00:00:00.000Z|x",
        ] {
            match ReindexWalkCursor::parse(bad) {
                Err(StorageError::Search(SearchError::InvalidCursor { .. })) => {}
                other => panic!("expected InvalidCursor for {bad:?}, got {other:?}"),
            }
        }
    }

    #[test]
    fn id_range_page_filter_shapes() {
        let floor = ts("2026-01-01T00:00:00.000Z");
        let id_of = |lo: Option<&str>, hi: Option<&str>, after_id: Option<&str>| {
            reindex_id_range_page_filter("t1", "Observation", floor, lo, hi, after_id)
                .get_document("id")
                .ok()
                .cloned()
        };
        assert_eq!(id_of(None, None, None), None);
        assert_eq!(id_of(Some("m"), None, None), Some(doc! { "$gte": "m" }));
        assert_eq!(id_of(None, Some("t"), None), Some(doc! { "$lt": "t" }));
        assert_eq!(
            id_of(Some("m"), Some("t"), None),
            Some(doc! { "$gte": "m", "$lt": "t" })
        );
        assert_eq!(id_of(None, None, Some("p")), Some(doc! { "$gt": "p" }));
        assert_eq!(id_of(Some("m"), None, Some("p")), Some(doc! { "$gt": "p" }));
        assert_eq!(
            id_of(None, Some("t"), Some("p")),
            Some(doc! { "$gt": "p", "$lt": "t" })
        );
        assert_eq!(
            id_of(Some("m"), Some("t"), Some("p")),
            Some(doc! { "$gt": "p", "$lt": "t" })
        );

        let full = reindex_id_range_page_filter("t1", "Observation", floor, Some("m"), None, None);
        assert_eq!(full.get_str("tenant_id").unwrap(), "t1");
        assert_eq!(full.get_str("resource_type").unwrap(), "Observation");
        assert!(!full.get_bool("is_deleted").unwrap());
        assert_eq!(
            full.get_document("last_updated").unwrap(),
            &doc! { "$lt": chrono_to_bson(floor) }
        );
    }
```

In `mod reindex_prefetch_tests`, after `fn round_cursor`, add:

```rust
    fn id_range_cursor() -> String {
        ReindexWalkCursor::IdRange {
            range: ReindexIdRange {
                floor: chrono::Utc::now(),
                lo: Some("a".to_string()),
                hi: Some("m".to_string()),
            },
            after_id: Some("b".to_string()),
        }
        .encode()
    }

    fn id_phase_done_cursor() -> String {
        ReindexWalkCursor::IdPhaseDone {
            floor: chrono::Utc::now(),
        }
        .encode()
    }
```

In `fn may_prefetch_page_accepts_only_id_cursors`, add after `assert!(!backend.may_prefetch_page("garbage"));`:

```rust
        assert!(backend.may_prefetch_page(&id_range_cursor()));
        assert!(!backend.may_prefetch_page(&id_phase_done_cursor()));
```

after `assert!(!no_prefetch.may_prefetch_page(&id_cursor()));`:

```rust
        assert!(!no_prefetch.may_prefetch_page(&id_range_cursor()));
```

and after `assert!(!offloaded.may_prefetch_page(&id_cursor()));`:

```rust
        assert!(!offloaded.may_prefetch_page(&id_range_cursor()));
```

In `fn fetch_ahead_declines_round_and_malformed_cursors`, add before its end:

```rust
        let result = backend
            .fetch_resources_page_ahead(&tenant, "Patient", &id_phase_done_cursor(), 10, 0)
            .await
            .expect("no database error");
        assert!(
            result.is_none(),
            "the catch-up's first cursor must never be fetched ahead"
        );
```

- [ ] **Step 2: Run the unit tests to verify they fail**

```bash
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
CARGO_BUILD_JOBS=4 cargo test -p helios-persistence --features mongodb --lib backends::mongodb::storage:: 2>&1 | tail -40
```
Expected: compile errors naming `ReindexIdRange`, `IdRange`, `IdPhaseDone` and `reindex_id_range_page_filter`.

- [ ] **Step 3: Implement the cursor grammar and the filter**

(a) Directly above `enum ReindexWalkCursor`, add:

```rust
/// One write stream's slice of a type's id phase (#1403): live resources
/// stamped before `floor` whose id lies in `[lo, hi)`; an unset bound is open.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ReindexIdRange {
    floor: DateTime<Utc>,
    lo: Option<String>,
    hi: Option<String>,
}

impl ReindexIdRange {
    /// Whether the bounds are ordered and `after_id` — the last id a page of
    /// this range took — lies inside them.
    fn admits(&self, after_id: Option<&str>) -> bool {
        if let (Some(lo), Some(hi)) = (&self.lo, &self.hi)
            && lo >= hi
        {
            return false;
        }
        let Some(after_id) = after_id else {
            return true;
        };
        self.lo.as_deref().is_none_or(|lo| lo <= after_id)
            && self.hi.as_deref().is_none_or(|hi| after_id < hi)
    }
}
```

(b) In `enum ReindexWalkCursor`, after the `Round { .. }` variant, add:

```rust
    /// A write stream's position in its id range (#1403); `after_id` is
    /// `None` before the range's first page.
    IdRange {
        range: ReindexIdRange,
        after_id: Option<String>,
    },
    /// Every id range of a split type has been written; the catch-up rounds
    /// start from `floor` (#1403).
    IdPhaseDone { floor: DateTime<Utc> },
```

(c) In `fn encode`, after the `Round` arm, add:

```rust
            ReindexWalkCursor::IdRange { range, after_id } => format!(
                "v2|r|{}|{}|{}|{}",
                format_walk_instant(range.floor),
                range.lo.as_deref().unwrap_or(""),
                range.hi.as_deref().unwrap_or(""),
                after_id.as_deref().unwrap_or("")
            ),
            ReindexWalkCursor::IdPhaseDone { floor } => {
                format!("v2|d|{}", format_walk_instant(*floor))
            }
```

(d) In `fn parse`, add two arms before `_ => Err(invalid()),`:

```rust
            "r" => {
                let fields: Vec<&str> = rest.splitn(4, '|').collect();
                let [floor, lo, hi, after_id] = fields[..] else {
                    return Err(invalid());
                };
                let floor = DateTime::parse_from_rfc3339(floor)
                    .map_err(|_| invalid())?
                    .with_timezone(&Utc);
                let set = |s: &str| (!s.is_empty()).then(|| s.to_string());
                let range = ReindexIdRange {
                    floor,
                    lo: set(lo),
                    hi: set(hi),
                };
                let after_id = set(after_id);
                if !range.admits(after_id.as_deref()) {
                    return Err(invalid());
                }
                Ok(ReindexWalkCursor::IdRange { range, after_id })
            }
            "d" => {
                if rest.contains('|') {
                    return Err(invalid());
                }
                let floor = DateTime::parse_from_rfc3339(rest)
                    .map_err(|_| invalid())?
                    .with_timezone(&Utc);
                Ok(ReindexWalkCursor::IdPhaseDone { floor })
            }
```

(e) After `fn reindex_id_page_filter`, add:

```rust
/// One write stream's filter (#1403): the id phase's predicate plus its
/// range's bounds. Once a page has been returned (`after_id` set),
/// `$gt: after_id` replaces `$gte: lo`; an unset bound adds no operator, and
/// with no operator at all the `id` key is omitted.
fn reindex_id_range_page_filter(
    tenant_id: &str,
    resource_type: &str,
    floor: DateTime<Utc>,
    lo: Option<&str>,
    hi: Option<&str>,
    after_id: Option<&str>,
) -> Document {
    let mut filter = doc! {
        "tenant_id": tenant_id,
        "resource_type": resource_type,
        "is_deleted": false,
        "last_updated": { "$lt": chrono_to_bson(floor) },
    };
    let mut id = Document::new();
    match (after_id, lo) {
        (Some(after_id), _) => {
            id.insert("$gt", after_id);
        }
        (None, Some(lo)) => {
            id.insert("$gte", lo);
        }
        (None, None) => {}
    }
    if let Some(hi) = hi {
        id.insert("$lt", hi);
    }
    if !id.is_empty() {
        filter.insert("id", id);
    }
    filter
}
```

(f) Replace `enum WalkStep`'s doc comment and add two variants, so it reads:

```rust
/// One step of the walk inside a single call (#1403); never leaves the
/// call — only a `ReindexWalkCursor` does, as an encoded cursor.
enum WalkStep {
    Start,
    IdPhase {
        floor: DateTime<Utc>,
        after_id: Option<String>,
    },
    /// The id phase is over: log it, then start round 1.
    IdPhaseDone {
        floor: DateTime<Utc>,
    },
    /// One page of one write stream's id range.
    IdRange {
        range: ReindexIdRange,
        after_id: Option<String>,
    },
    RoundStart {
        round: u8,
        floor: DateTime<Utc>,
    },
    Round {
        round: u8,
        floor: DateTime<Utc>,
        ceiling: DateTime<Utc>,
        walked: u64,
        after: Option<(DateTime<Utc>, String)>,
    },
}
```

and in `impl From<ReindexWalkCursor> for WalkStep`, add:

```rust
            ReindexWalkCursor::IdRange { range, after_id } => {
                WalkStep::IdRange { range, after_id }
            }
            ReindexWalkCursor::IdPhaseDone { floor } => WalkStep::IdPhaseDone { floor },
```

- [ ] **Step 4: Implement the walk steps, the range page and the prefetch**

(a) In the `impl MongoBackend` block that holds `reindex_newest_live_last_updated`, add after that method:

```rust
    /// The walk's catch-up margin: the configured one, clamped (#1403).
    fn reindex_margin(&self) -> chrono::Duration {
        reindex_catch_up_margin(self.config().reindex_catch_up_margin_ms)
    }

    /// Fixes a type's floor when its walk starts — `min(newest live
    /// last_updated + 1 ms, now − margin)` — and logs `mongodb reindex walk
    /// started` (#1403). A single walk calls it from its first page; a split
    /// type calls it once, from its plan, before the ranges are cut.
    async fn reindex_walk_floor(
        &self,
        resources: &Collection<Document>,
        tenant_id: &str,
        resource_type: &str,
    ) -> StorageResult<DateTime<Utc>> {
        let t0 = Utc::now();
        let newest_live = self
            .reindex_newest_live_last_updated(resources, tenant_id, resource_type)
            .await?;
        let floor = reindex_catch_up_floor(t0, newest_live, self.reindex_margin());
        tracing::info!(
            tenant = %tenant_id,
            resource_type = %resource_type,
            t0 = %format_walk_instant(t0),
            newest_live = %newest_live.map(format_walk_instant).unwrap_or_else(|| "none".to_string()),
            floor = %format_walk_instant(floor),
            "mongodb reindex walk started"
        );
        Ok(floor)
    }
```

(b) After `async fn reindex_id_page`, add:

```rust
    /// One page of one write stream's id range (#1403), for both the serial
    /// walk and the driver's ahead-of-time prefetch. A range never moves on
    /// to another phase — the driver starts the catch-up once every range has
    /// finished — so an empty query simply ends it: that call logs `mongodb
    /// reindex id range finished` and returns an empty page with no next
    /// cursor. The last row taken is the last row scanned: a row that fails
    /// to decode fails the whole page.
    async fn reindex_id_range_page(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        range: &ReindexIdRange,
        after_id: Option<&str>,
        limit: u32,
        max_bytes: u64,
    ) -> StorageResult<ResourcePage> {
        let db = self.get_database().await?;
        let resources = db.collection::<Document>(Self::RESOURCES_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();
        let found = self
            .reindex_find_page(
                &resources,
                reindex_id_range_page_filter(
                    tenant_id,
                    resource_type,
                    range.floor,
                    range.lo.as_deref(),
                    range.hi.as_deref(),
                    after_id,
                ),
                doc! { "id": 1 },
                RESOURCES_IDENTITY_INDEX,
                limit,
                max_bytes,
            )
            .await?;
        let Some(last) = found.docs.last() else {
            tracing::info!(
                tenant = %tenant_id,
                resource_type = %resource_type,
                floor = %format_walk_instant(range.floor),
                lo = %range.lo.as_deref().unwrap_or("*"),
                hi = %range.hi.as_deref().unwrap_or("*"),
                "mongodb reindex id range finished"
            );
            return Ok(ResourcePage::default());
        };
        if max_bytes > 0 {
            log_capped_page_read(tenant_id, resource_type, &found);
        }
        let last_id = last.get_str("id").map(str::to_string).map_err(|_| {
            internal_error("Missing id on the last row of an id-range page".to_string())
        })?;
        reindex_page_from_docs(
            &found.docs,
            resource_type,
            tenant,
            ReindexWalkCursor::IdRange {
                range: range.clone(),
                after_id: Some(last_id),
            },
        )
    }
```

(c) In `async fn fetch_reindex_page`:

1. Replace `let margin = reindex_catch_up_margin(self.config().reindex_catch_up_margin_ms);` with `let margin = self.reindex_margin();`.
2. Replace the whole `WalkStep::Start => { .. }` arm with:
   ```rust
                   WalkStep::Start => WalkStep::IdPhase {
                       floor: self
                           .reindex_walk_floor(&resources, tenant_id, resource_type)
                           .await?,
                       after_id: None,
                   },
   ```
3. Replace the whole `WalkStep::IdPhase { floor, after_id } => { .. }` arm with:
   ```rust
                   WalkStep::IdPhase { floor, after_id } => {
                       if let Some(page) = self
                           .reindex_id_page(
                               tenant,
                               resource_type,
                               floor,
                               after_id.as_deref(),
                               limit,
                               max_bytes,
                           )
                           .await?
                       {
                           return Ok(page);
                       }
                       WalkStep::IdPhaseDone { floor }
                   }
                   WalkStep::IdPhaseDone { floor } => {
                       tracing::info!(
                           tenant = %tenant_id,
                           resource_type = %resource_type,
                           floor = %format_walk_instant(floor),
                           "mongodb reindex id phase finished"
                       );
                       WalkStep::RoundStart { round: 1, floor }
                   }
                   WalkStep::IdRange { range, after_id } => {
                       return self
                           .reindex_id_range_page(
                               tenant,
                               resource_type,
                               &range,
                               after_id.as_deref(),
                               limit,
                               max_bytes,
                           )
                           .await;
                   }
   ```
4. Append to `fetch_reindex_page`'s doc comment:
   ```rust
    /// An id-range cursor walks one write stream's slice of the id phase and
    /// ends on an empty query without moving on; an id-phase-done cursor logs
    /// the end of the id phase and starts catch-up round 1 in the same call.
   ```

(d) In `impl ReindexSource for MongoBackend`, replace `may_prefetch_page` and `fetch_resources_page_ahead` (doc comments included) with:

```rust
    /// Only an id-phase or id-range continuation may run ahead of the write
    /// in flight (#1403): each reads live resources stamped strictly before a
    /// floor fixed when the walk (or the plan) started, so it observes
    /// nothing the page being written could change. A catch-up round reads up
    /// to "now", so running it early could race the very writes it is meant
    /// to pick up; the catch-up's first cursor and a cursor that fails to
    /// parse are rejected the same way. `reindex_prefetch` and search offload
    /// gate it off entirely.
    fn may_prefetch_page(&self, cursor: &str) -> bool {
        self.config().reindex_prefetch
            && !self.is_search_offloaded()
            && matches!(
                ReindexWalkCursor::parse(cursor),
                Ok(ReindexWalkCursor::Id { .. } | ReindexWalkCursor::IdRange { .. })
            )
    }

    async fn fetch_resources_page_ahead(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        cursor: &str,
        limit: u32,
        max_bytes: u64,
    ) -> StorageResult<Option<ResourcePage>> {
        match ReindexWalkCursor::parse(cursor) {
            // The end of the id phase is fetched serially, after the page in
            // flight is written.
            Ok(ReindexWalkCursor::Id { floor, after_id }) => {
                self.reindex_id_page(
                    tenant,
                    resource_type,
                    floor,
                    Some(&after_id),
                    limit.max(1),
                    max_bytes,
                )
                .await
            }
            // A range never moves on to another phase, so every one of its
            // pages — the empty one that ends it included — may run ahead.
            Ok(ReindexWalkCursor::IdRange { range, after_id }) => self
                .reindex_id_range_page(
                    tenant,
                    resource_type,
                    &range,
                    after_id.as_deref(),
                    limit.max(1),
                    max_bytes,
                )
                .await
                .map(Some),
            _ => Ok(None),
        }
    }
```

- [ ] **Step 5: Run the unit tests to verify they pass**

```bash
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
CARGO_BUILD_JOBS=4 cargo test -p helios-persistence --features mongodb --lib backends::mongodb::storage:: 2>&1 | tail -40
```
Expected: `test result: ok.` — the four new cursor/filter tests, the extended prefetch tests, and every existing `reindex_walk_tests`, `reindex_page_cap_tests` and `reindex_prefetch_tests` test unchanged (in particular `cursor_rejects_foreign_and_malformed_tokens`, which still rejects `v2|s|…`).

- [ ] **Step 6: Write the integration tests**

(a) In `crates/persistence/tests/mongodb/reindex_id_walk.rs`, make these `pub(super)`: `struct WalkFixture` and its field `live` (`pub(super) struct WalkFixture { pub(super) live: .., tombstones: .. }`), `async fn seed_walk_fixture`, `async fn backdate_fixture`, `async fn snapshot`. Change nothing else.

(b) In `crates/persistence/tests/mongodb_tests.rs`, after

```rust
/// #1499: MongoDB honours `HFS_REINDEX_BATCH_BYTES` (PR2a).
#[path = "mongodb/reindex_pipeline.rs"]
mod reindex_pipeline;
```

add:

```rust
/// #1403: concurrent write streams over disjoint id ranges.
#[path = "mongodb/reindex_streams.rs"]
mod reindex_streams;
```

(c) Create `crates/persistence/tests/mongodb/reindex_streams.rs`:

```rust
//! #1403: concurrent write streams over disjoint id ranges for the MongoDB
//! `$reindex` rebuild — the range and catch-up cursors, the stream plan, and
//! whole rebuilds split across streams. A `#[path]`-included child module of
//! `mongodb_tests.rs`, like `reindex_id_walk.rs`: `use super::*` reaches the
//! parent's harness (`create_backend`, `create_tenant`, `shared_mongo`, …).

use super::*;

use std::collections::BTreeSet;

use helios_persistence::search::{ReindexSource, ResourcePage};

use super::reindex_id_walk::{
    backdate_fixture, capture_walk_logs, seed_walk_fixture, walk_log_lines,
};

/// Floor of the hand-built range cursors below: later than every backdated
/// fixture row, so the whole fixture falls in the id phase.
const RANGE_FLOOR: &str = "2021-01-01T00:00:00.000Z";

/// A range cursor spelled out, pinning the backend's grammar
/// `v2|r|<floor>|<lo>|<hi>|<after_id>`: an empty bound is open, and an empty
/// `after_id` starts the range.
fn range_cursor(lo: &str, hi: &str) -> String {
    format!("v2|r|{RANGE_FLOOR}|{lo}|{hi}|")
}

fn ids_of(page: &ResourcePage) -> Vec<String> {
    page.resources.iter().map(|r| r.id().to_string()).collect()
}

/// Pages one Observation walk from `cursor` to its end, checking the walk's
/// page contract on the way: every page with a next cursor holds at least
/// one resource, and the walk ends on one empty page with none. Returns every
/// id, in fetch order.
async fn walk_ids(
    backend: &MongoBackend,
    tenant: &TenantContext,
    cursor: &str,
    limit: u32,
) -> Vec<String> {
    let mut ids = Vec::new();
    let mut cursor = cursor.to_string();
    for _ in 0..500 {
        let page = backend
            .fetch_resources_page_capped(tenant, "Observation", Some(&cursor), limit, 0)
            .await
            .unwrap();
        let page_ids = ids_of(&page);
        match page.next_cursor {
            Some(next) => {
                assert!(
                    !page_ids.is_empty(),
                    "a page with a next cursor must hold a resource"
                );
                ids.extend(page_ids);
                cursor = next;
            }
            None => {
                assert!(page_ids.is_empty(), "a walk ends on one empty page");
                return ids;
            }
        }
    }
    panic!("the walk from {cursor} did not end within 500 pages");
}

#[tokio::test]
async fn mongodb_id_range_cursors_partition_the_id_phase() {
    let Some(backend) = create_backend("reindex_streams_range_cursors").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("streams-range-cursors");
    let fixture = seed_walk_fixture(&backend, &tenant, 60, "extra").await;
    backdate_fixture(&backend, &tenant, &fixture).await;
    capture_walk_logs();

    let bounds = [("", "obs-020"), ("obs-020", "obs-040"), ("obs-040", "")];
    let mut seen = BTreeSet::new();
    for (lo, hi) in bounds {
        let ids = walk_ids(&backend, &tenant, &range_cursor(lo, hi), 7).await;
        assert!(!ids.is_empty(), "[{lo}, {hi}) is empty");
        assert!(
            ids.windows(2).all(|w| w[0] < w[1]),
            "[{lo}, {hi}) is not in id order: {ids:?}"
        );
        for id in ids {
            assert!(lo.is_empty() || id.as_str() >= lo, "{id} is below {lo}");
            assert!(hi.is_empty() || id.as_str() < hi, "{id} is at or above {hi}");
            assert!(seen.insert(id.clone()), "{id} came back from two ranges");
        }
    }
    assert_eq!(&seen, &fixture.live["Observation"]);

    let needle = format!("tenant={}", tenant.tenant_id().as_str());
    let finished = walk_log_lines(&["mongodb reindex id range finished", &needle]);
    assert_eq!(finished.len(), 3, "{finished:?}");
    for (line, (lo, hi)) in finished.iter().zip(bounds) {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        let lo = format!("lo={}", if lo.is_empty() { "*" } else { lo });
        let hi = format!("hi={}", if hi.is_empty() { "*" } else { hi });
        assert!(tokens.contains(&lo.as_str()), "{line}");
        assert!(tokens.contains(&hi.as_str()), "{line}");
        assert!(
            tokens.contains(&format!("floor={RANGE_FLOOR}").as_str()),
            "{line}"
        );
    }
    assert!(
        walk_log_lines(&["mongodb reindex id phase finished", &needle]).is_empty(),
        "a range never ends the id phase itself"
    );
}

#[tokio::test]
async fn mongodb_id_range_ahead_fetch_matches_the_serial_fetch() {
    let Some(backend) = create_backend("reindex_streams_range_ahead").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("streams-range-ahead");
    let fixture = seed_walk_fixture(&backend, &tenant, 30, "extra").await;
    backdate_fixture(&backend, &tenant, &fixture).await;

    let mut cursor = range_cursor("obs-010", "");
    for _ in 0..50 {
        let serial = backend
            .fetch_resources_page_capped(&tenant, "Observation", Some(&cursor), 4, 0)
            .await
            .unwrap();
        let ahead = backend
            .fetch_resources_page_ahead(&tenant, "Observation", &cursor, 4, 0)
            .await
            .unwrap()
            .expect("a range page is always fetched ahead, the empty last one included");
        assert_eq!(ids_of(&ahead), ids_of(&serial));
        assert_eq!(ahead.next_cursor, serial.next_cursor);
        match serial.next_cursor {
            Some(next) => cursor = next,
            None => return,
        }
    }
    panic!("the range did not end within 50 pages");
}

#[tokio::test]
async fn mongodb_id_phase_done_cursor_runs_the_catch_up_from_its_floor() {
    let Some(backend) = create_backend("reindex_streams_phase_done").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("streams-phase-done");
    let fixture = seed_walk_fixture(&backend, &tenant, 30, "extra").await;
    backdate_fixture(&backend, &tenant, &fixture).await;
    capture_walk_logs();

    // The backdated fixture stamps Observation `i` at second `i % 3`, so a
    // catch-up from second 1 walks exactly the Observations of seconds 1 and 2.
    let walked: BTreeSet<String> =
        walk_ids(&backend, &tenant, "v2|d|2020-01-01T00:00:01.000Z", 50)
            .await
            .into_iter()
            .collect();
    let expected: BTreeSet<String> = fixture.live["Observation"]
        .iter()
        .filter(|id| {
            let i: usize = id.trim_start_matches("obs-").parse().unwrap();
            i % 3 != 0
        })
        .cloned()
        .collect();
    assert_eq!(walked, expected);

    let needle = format!("tenant={}", tenant.tenant_id().as_str());
    let finished = walk_log_lines(&["mongodb reindex id phase finished", &needle]);
    assert_eq!(finished.len(), 1, "{finished:?}");
    assert!(
        finished[0]
            .split_whitespace()
            .any(|t| t == "floor=2020-01-01T00:00:01.000Z"),
        "{}",
        finished[0]
    );
    assert_eq!(
        walk_log_lines(&["mongodb reindex catch-up round started", &needle, "round=1"]).len(),
        1
    );
    assert!(
        walk_log_lines(&["mongodb reindex walk started", &needle]).is_empty(),
        "a catch-up cursor never restarts the walk"
    );
}
```

- [ ] **Step 7: Run the integration tests**

```bash
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
CARGO_BUILD_JOBS=4 cargo test -p helios-persistence --features mongodb --test mongodb_tests reindex_streams 2>&1 | tail -40
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
CARGO_BUILD_JOBS=4 cargo test -p helios-persistence --features mongodb --test mongodb_tests reindex_id_walk 2>&1 | tail -40
```
Expected: `test result: ok. 3 passed` for the first (no `Skipping` line), and the whole existing id-walk suite still passing for the second (its single walks now log `walk started` through `reindex_walk_floor` and `id phase finished` through `IdPhaseDone`).

- [ ] **Step 8: fmt, clippy, commit**

```bash
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
CARGO_BUILD_JOBS=4 cargo fmt -- crates/persistence/src/backends/mongodb/storage.rs crates/persistence/tests/mongodb_tests.rs crates/persistence/tests/mongodb/reindex_id_walk.rs crates/persistence/tests/mongodb/reindex_streams.rs
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
CARGO_BUILD_JOBS=4 cargo clippy -p helios-persistence --all-targets --all-features -- -D warnings -A clippy::items_after_test_module -A clippy::large_enum_variant -A clippy::question_mark -A clippy::collapsible_match -A clippy::collapsible_if -A clippy::field_reassign_with_default -A clippy::doc-overindented-list-items -A clippy::doc-lazy-continuation 2>&1 | tail -30
git add crates/persistence/src/backends/mongodb/storage.rs crates/persistence/tests/mongodb_tests.rs crates/persistence/tests/mongodb/reindex_id_walk.rs crates/persistence/tests/mongodb/reindex_streams.rs
git commit -m "$(cat <<'EOF'
feat(mongodb): id-range and id-phase-done reindex cursors (#1403)

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Y61t4nTxKCT8WeE5NbxaVc
EOF
)"
```

---

### Task 6: MongoDB plans write streams (`plan_type_walk`)

**Files:**
- Modify: `crates/persistence/src/backends/mongodb/backend.rs` — `pub struct MongoBackend` (after `reindex_mode_logged`, `:108-110`), `MongoBackend::new` (`:470-481`), the `impl MongoBackend` accessor block (after `fn reindex_mode_logged`, `:181-183`).
- Modify: `crates/persistence/src/backends/mongodb/storage.rs` — the `use crate::search::reindex::{..}` import (`:29`); after `fn log_capped_page_read` (`:4773-4782`); the walk `impl MongoBackend` block (after `reindex_id_range_page`, Task 5); `impl ReindexSource for MongoBackend` (after `fetch_resources_page_ahead`); after `fn reindex_id_range_page_filter` (Task 5); `mod reindex_prefetch_tests` (`fn unreachable_config`); a new `mod reindex_streams_tests` at the end of the file.
- Modify: `crates/persistence/tests/mongodb_tests.rs` — `async fn build_backend` (`:1198-1253`).
- Modify: `crates/persistence/tests/mongodb/reindex_pipeline.rs` — `async fn create_backend_with` (`:31-43`).
- Modify: `crates/persistence/tests/mongodb/reindex_streams.rs`.

**Interfaces:**
- Consumes: Task 5's `ReindexIdRange`, `ReindexWalkCursor::{IdRange, IdPhaseDone}`, `reindex_walk_floor`; Task 1's `TypeWalkPlan`, `TypeWalkRequest`.
- Produces:
  - `MongoBackend.reindex_streams_clamp_warned: std::sync::atomic::AtomicBool`; `pub(super) fn reindex_streams_clamp_warned(&self) -> &std::sync::atomic::AtomicBool`
  - `fn reindex_stream_budget(max_connections: u32, concurrent_runs: u32) -> u32`
  - `fn reindex_streams_for_size(resources: u64, min_per_stream: u64) -> u32`
  - `fn reindex_id_range_cursors(floor: DateTime<Utc>, boundaries: &[String]) -> Vec<String>`
  - `fn log_streams_planned(tenant_id: &str, resource_type: &str, request: TypeWalkRequest, allowed: u32, resources: u64, streams: usize, plan: std::time::Duration)`
  - `async fn reindex_range_boundaries(&self, resources: &Collection<Document>, tenant_id: &str, resource_type: &str, counted: u64, streams: u32) -> StorageResult<Vec<String>>`
  - `impl ReindexSource for MongoBackend { async fn plan_type_walk(..) }`
  - tests: `async fn build_backend_with_pool(config: MongoBackendConfig, max_pool: u32) -> Option<MongoBackend>` (`mongodb_tests.rs`); `pub(super) async fn create_backend_with_pool(test_name: &str, pool: u32, configure: impl FnOnce(&mut MongoBackendConfig)) -> Option<Arc<MongoBackend>>` (`reindex_pipeline.rs`, which `create_backend_with` now calls); `const STREAMS_TEST_POOL: u32 = 10`, `async fn create_streams_backend(test_name: &str) -> Option<Arc<MongoBackend>>` (`reindex_streams.rs`, one call of `create_backend_with_pool`).

- [ ] **Step 1: Write the failing unit tests**

In `storage.rs`, `mod reindex_prefetch_tests`, change `fn unreachable_config()` to `pub(super) fn unreachable_config()`. Then add at the end of `storage.rs`:

```rust
#[cfg(test)]
mod reindex_streams_tests {
    //! Docker-free tests for #1403's stream plan: the pure budget, size and
    //! range rules, and the plan's early returns, none of which reaches the
    //! database.

    use super::reindex_prefetch_tests::unreachable_config;
    use super::*;
    use crate::backends::mongodb::MongoBackendConfig;
    use crate::search::reindex::ReindexSource;
    use crate::tenant::{TenantId, TenantPermissions};

    fn tenant() -> TenantContext {
        TenantContext::new(
            TenantId::new("streams-unit-tenant"),
            TenantPermissions::full_access(),
        )
    }

    fn request(streams: u32) -> TypeWalkRequest {
        TypeWalkRequest {
            streams,
            min_resources_per_stream: 1,
            concurrent_runs: 1,
        }
    }

    #[test]
    fn reindex_stream_budget_cases() {
        assert_eq!(reindex_stream_budget(0, 1), 1);
        assert_eq!(reindex_stream_budget(1, 1), 1);
        assert_eq!(reindex_stream_budget(4, 1), 1);
        assert_eq!(reindex_stream_budget(6, 1), 2);
        assert_eq!(reindex_stream_budget(10, 1), 4);
        assert_eq!(reindex_stream_budget(10, 2), 2);
        assert_eq!(reindex_stream_budget(18, 2), 4);
        assert!(reindex_stream_budget(u32::MAX, 0) >= 16);
    }

    #[test]
    fn reindex_streams_for_size_cases() {
        assert_eq!(reindex_streams_for_size(0, 50_000), 1);
        assert_eq!(reindex_streams_for_size(99_999, 50_000), 1);
        assert_eq!(reindex_streams_for_size(100_000, 50_000), 2);
        assert_eq!(reindex_streams_for_size(400_000, 50_000), 8);
        assert_eq!(reindex_streams_for_size(10, 0), 10);
        assert_eq!(reindex_streams_for_size(u64::MAX, 1), u32::MAX);
    }

    #[test]
    fn id_range_cursors_cover_the_boundaries_in_order() {
        let floor = DateTime::parse_from_rfc3339("2026-01-01T00:00:00.000Z")
            .unwrap()
            .with_timezone(&Utc);
        let range = |lo: Option<&str>, hi: Option<&str>| ReindexWalkCursor::IdRange {
            range: ReindexIdRange {
                floor,
                lo: lo.map(str::to_string),
                hi: hi.map(str::to_string),
            },
            after_id: None,
        };
        let parsed: Vec<ReindexWalkCursor> =
            reindex_id_range_cursors(floor, &["m".to_string(), "t".to_string()])
                .iter()
                .map(|c| ReindexWalkCursor::parse(c).unwrap())
                .collect();
        assert_eq!(
            parsed,
            vec![
                range(None, Some("m")),
                range(Some("m"), Some("t")),
                range(Some("t"), None)
            ]
        );
        assert_eq!(
            reindex_id_range_cursors(floor, &[]),
            vec![range(None, None).encode()]
        );
    }

    #[tokio::test]
    async fn plan_type_walk_is_single_when_search_is_offloaded() {
        let backend = MongoBackend::new(MongoBackendConfig {
            search_offloaded: true,
            ..unreachable_config()
        })
        .expect("lazy client");
        let plan = backend
            .plan_type_walk(&tenant(), "Observation", request(4))
            .await
            .expect("no database call");
        assert_eq!(plan, TypeWalkPlan::Single);
    }

    #[tokio::test]
    async fn plan_type_walk_is_single_for_one_stream() {
        let backend = MongoBackend::new(unreachable_config()).expect("lazy client");
        let plan = backend
            .plan_type_walk(&tenant(), "Observation", request(1))
            .await
            .expect("no database call");
        assert_eq!(plan, TypeWalkPlan::Single);
    }

    #[tokio::test]
    async fn plan_type_walk_fits_the_pool_before_any_query() {
        let backend = MongoBackend::new(MongoBackendConfig {
            max_connections: 4,
            ..unreachable_config()
        })
        .expect("lazy client");
        assert!(
            !backend
                .reindex_streams_clamp_warned()
                .load(std::sync::atomic::Ordering::Relaxed)
        );
        for _ in 0..2 {
            let plan = backend
                .plan_type_walk(&tenant(), "Observation", request(4))
                .await
                .expect("a pool of 4 admits one stream, decided before any query");
            assert_eq!(plan, TypeWalkPlan::Single);
        }
        assert!(
            backend
                .reindex_streams_clamp_warned()
                .load(std::sync::atomic::Ordering::Relaxed)
        );
    }
}
```

- [ ] **Step 2: Run the unit tests to verify they fail**

```bash
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
CARGO_BUILD_JOBS=4 cargo test -p helios-persistence --features mongodb --lib backends::mongodb::storage::reindex_streams_tests 2>&1 | tail -40
```
Expected: compile errors naming `reindex_stream_budget`, `reindex_streams_for_size`, `reindex_id_range_cursors`, `TypeWalkRequest`/`TypeWalkPlan` (not imported yet) and `reindex_streams_clamp_warned`.

- [ ] **Step 3: Implement**

(a) `backend.rs` — in `pub struct MongoBackend`, after `reindex_mode_logged`, add:

```rust
    /// Whether the `HFS_REINDEX_WRITE_STREAMS` clamp warning has already been
    /// logged for this instance (#1403).
    reindex_streams_clamp_warned: std::sync::atomic::AtomicBool,
```

in `MongoBackend::new`'s `Ok(Self { .. })`, after `reindex_mode_logged: ..,` add:

```rust
            reindex_streams_clamp_warned: std::sync::atomic::AtomicBool::new(false),
```

and after `pub(super) fn reindex_mode_logged(&self)`, add:

```rust
    /// Whether the `HFS_REINDEX_WRITE_STREAMS` clamp warning has already been
    /// logged for this backend instance (#1403).
    pub(super) fn reindex_streams_clamp_warned(&self) -> &std::sync::atomic::AtomicBool {
        &self.reindex_streams_clamp_warned
    }
```

(b) `storage.rs` — change the import to

```rust
use crate::search::reindex::{
    ReindexPageStats, ReindexSource, ReindexTarget, ResourcePage, TypeWalkPlan, TypeWalkRequest,
};
```

(c) After `fn log_capped_page_read`, add:

```rust
/// Logs INFO `mongodb reindex streams planned` (#1403). Fields, in order:
/// `tenant, resource_type, requested, allowed, resources, streams, plan_ms`.
/// `allowed` is what the connection pool admits; `resources` is the type's
/// count on `idx_resources_identity`, tombstones included (0 when the pool
/// alone settled on one stream, before counting); `streams` is the number of
/// ranges the plan returns, 1 for a single walk; `plan_ms` is the planning
/// time in whole milliseconds, truncated.
fn log_streams_planned(
    tenant_id: &str,
    resource_type: &str,
    request: TypeWalkRequest,
    allowed: u32,
    resources: u64,
    streams: usize,
    plan: std::time::Duration,
) {
    tracing::info!(
        tenant = %tenant_id,
        resource_type = %resource_type,
        requested = u64::from(request.streams),
        allowed = u64::from(allowed),
        resources = resources,
        streams = streams as u64,
        plan_ms = u64::try_from(plan.as_millis()).unwrap_or(u64::MAX),
        "mongodb reindex streams planned"
    );
}
```

(d) After `fn reindex_id_range_page_filter`, add:

```rust
/// Streams one rebuild may run so that every concurrent automatic rebuild
/// together keeps 2 connections per stream (one write, one prefetch) and
/// leaves 2 for foreground requests (#1403). Never below 1.
fn reindex_stream_budget(max_connections: u32, concurrent_runs: u32) -> u32 {
    let per_stream = 2u32.saturating_mul(concurrent_runs.max(1));
    (max_connections.saturating_sub(2) / per_stream).max(1)
}

/// Most streams a type of `resources` rows (tombstones included) can use when
/// each must cover at least `min_per_stream` of them; never below 1 (#1403).
fn reindex_streams_for_size(resources: u64, min_per_stream: u64) -> u32 {
    u32::try_from(resources / min_per_stream.max(1))
        .unwrap_or(u32::MAX)
        .max(1)
}

/// Encodes the ranges `boundaries` cut a type into (#1403): `[.., b1)`,
/// `[b1, b2)`, …, `[bm, ..)`, each over live resources stamped before
/// `floor`; no boundary is one unbounded range. Every id falls in exactly one.
fn reindex_id_range_cursors(floor: DateTime<Utc>, boundaries: &[String]) -> Vec<String> {
    let mut cursors = Vec::with_capacity(boundaries.len() + 1);
    let mut lo: Option<String> = None;
    for boundary in boundaries {
        cursors.push(
            ReindexWalkCursor::IdRange {
                range: ReindexIdRange {
                    floor,
                    lo: lo.clone(),
                    hi: Some(boundary.clone()),
                },
                after_id: None,
            }
            .encode(),
        );
        lo = Some(boundary.clone());
    }
    cursors.push(
        ReindexWalkCursor::IdRange {
            range: ReindexIdRange { floor, lo, hi: None },
            after_id: None,
        }
        .encode(),
    );
    cursors
}
```

(e) In the walk `impl MongoBackend` block, after `async fn reindex_id_range_page`, add:

```rust
    /// The first id of every range but the first (#1403): chained, covered
    /// probes of `idx_resources_identity`, each skipping one range's worth of
    /// keys past the previous boundary, so a whole plan reads about one pass
    /// of the type's identity keys (a single `skip` from the start for each
    /// boundary would read about half as many again at four streams). Fewer
    /// boundaries come back only when the type shrank since it was counted.
    /// Boundaries decide balance only: whatever they are, every id falls in
    /// exactly one range. `distinct` with a hint needs MongoDB 7.1, so it is
    /// not used.
    async fn reindex_range_boundaries(
        &self,
        resources: &Collection<Document>,
        tenant_id: &str,
        resource_type: &str,
        counted: u64,
        streams: u32,
    ) -> StorageResult<Vec<String>> {
        let step = counted / u64::from(streams.max(1));
        let mut boundaries: Vec<String> = Vec::new();
        for _ in 1..streams {
            let (filter, skip) = match boundaries.last() {
                None => (
                    doc! { "tenant_id": tenant_id, "resource_type": resource_type },
                    step,
                ),
                Some(previous) => (
                    doc! {
                        "tenant_id": tenant_id,
                        "resource_type": resource_type,
                        "id": { "$gt": previous.as_str() },
                    },
                    step.saturating_sub(1),
                ),
            };
            let found = resources
                .find_one(filter)
                .sort(doc! { "id": 1 })
                .skip(skip)
                .projection(doc! { "_id": 0, "id": 1 })
                .hint(Hint::Name(RESOURCES_IDENTITY_INDEX.to_string()))
                .await
                .map_err(|e| internal_error(format!("Failed to probe a reindex range boundary: {e}")))?;
            match found.as_ref().and_then(|doc| doc.get_str("id").ok()) {
                Some(id) => boundaries.push(id.to_string()),
                None => break,
            }
        }
        Ok(boundaries)
    }
```

(f) In `impl ReindexSource for MongoBackend`, after `fetch_resources_page_ahead`, add:

```rust
    /// Splits a type into up to `request.streams` contiguous id ranges, one
    /// write stream each (#1403). Only standalone MongoDB splits: with search
    /// offloaded to Elasticsearch the walk stays whole. The streams fit the
    /// connection pool — 2 connections per stream (one write, one prefetch)
    /// for each concurrent rebuild, plus 2 for foreground requests; the first
    /// time the pool lowers the request, one WARN says so — and each covers
    /// at least `request.min_resources_per_stream` resources. The floor is
    /// fixed once, before the ranges are cut; every range walks the live
    /// resources stamped before it, and the catch-up cursor runs the catch-up
    /// rounds from that floor once every range has been written.
    async fn plan_type_walk(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        request: TypeWalkRequest,
    ) -> StorageResult<TypeWalkPlan> {
        let tenant_id = tenant.tenant_id().as_str();
        if self.is_search_offloaded() {
            tracing::debug!(
                tenant = %tenant_id,
                resource_type = %resource_type,
                "mongodb reindex streams skipped"
            );
            return Ok(TypeWalkPlan::Single);
        }
        if request.streams <= 1 {
            return Ok(TypeWalkPlan::Single);
        }
        let started = std::time::Instant::now();
        let allowed = reindex_stream_budget(self.config().max_connections, request.concurrent_runs);
        let streams = request.streams.min(allowed);
        if streams < request.streams
            && !self
                .reindex_streams_clamp_warned()
                .swap(true, std::sync::atomic::Ordering::Relaxed)
        {
            let needed = 2 * u64::from(request.streams) * u64::from(request.concurrent_runs.max(1)) + 2;
            tracing::warn!(
                tenant = %tenant_id,
                resource_type = %resource_type,
                "HFS_REINDEX_WRITE_STREAMS={} needs HFS_MONGODB_MAX_CONNECTIONS ≥ {}; using {}",
                request.streams,
                needed,
                streams
            );
        }
        if streams <= 1 {
            log_streams_planned(tenant_id, resource_type, request, allowed, 0, 1, started.elapsed());
            return Ok(TypeWalkPlan::Single);
        }

        let db = self.get_database().await?;
        let resources: Collection<Document> = db.collection(MongoBackend::RESOURCES_COLLECTION);
        let counted = resources
            .count_documents(doc! { "tenant_id": tenant_id, "resource_type": resource_type })
            .hint(Hint::Name(RESOURCES_IDENTITY_INDEX.to_string()))
            .await
            .map_err(|e| internal_error(format!("Failed to count resources for a reindex plan: {e}")))?;
        let streams = streams.min(reindex_streams_for_size(
            counted,
            request.min_resources_per_stream,
        ));
        if streams <= 1 {
            log_streams_planned(
                tenant_id,
                resource_type,
                request,
                allowed,
                counted,
                1,
                started.elapsed(),
            );
            return Ok(TypeWalkPlan::Single);
        }

        let floor = self
            .reindex_walk_floor(&resources, tenant_id, resource_type)
            .await?;
        let boundaries = self
            .reindex_range_boundaries(&resources, tenant_id, resource_type, counted, streams)
            .await?;
        let ranges = reindex_id_range_cursors(floor, &boundaries);
        log_streams_planned(
            tenant_id,
            resource_type,
            request,
            allowed,
            counted,
            ranges.len(),
            started.elapsed(),
        );
        Ok(TypeWalkPlan::Ranges {
            ranges,
            catch_up: ReindexWalkCursor::IdPhaseDone { floor }.encode(),
        })
    }
```

- [ ] **Step 4: Run the unit tests to verify they pass**

```bash
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
CARGO_BUILD_JOBS=4 cargo test -p helios-persistence --features mongodb --lib backends::mongodb:: 2>&1 | tail -40
```
Expected: `test result: ok.` — the six new `reindex_streams_tests` and every other MongoDB unit test.

- [ ] **Step 5: Write the integration tests**

(a) In `crates/persistence/tests/mongodb_tests.rs`, replace `async fn build_backend(mut config: MongoBackendConfig) -> Option<MongoBackend> {` and its first two lines

```rust
async fn build_backend(mut config: MongoBackendConfig) -> Option<MongoBackend> {
    const MAX_ATTEMPTS: u32 = 3;
    config.max_connections = config.max_connections.min(TEST_BACKEND_MAX_POOL);
```

with

```rust
async fn build_backend(config: MongoBackendConfig) -> Option<MongoBackend> {
    build_backend_with_pool(config, TEST_BACKEND_MAX_POOL).await
}

/// [`build_backend`] with the pool capped at `max_pool` instead of
/// [`TEST_BACKEND_MAX_POOL`]. The `$reindex` write-stream tests pass a larger
/// pool: a rebuild's stream budget is `(max_connections - 2) / 2`, so at the
/// suite-wide cap every plan would be a single walk (#1403).
async fn build_backend_with_pool(
    mut config: MongoBackendConfig,
    max_pool: u32,
) -> Option<MongoBackend> {
    const MAX_ATTEMPTS: u32 = 3;
    config.max_connections = config.max_connections.min(max_pool);
```

leaving the rest of the old body (from `// Generation-2 indexes are built after boot by default;` on) as the body of `build_backend_with_pool`. The existing doc comment stays above `build_backend`, unchanged.

(a2) In `crates/persistence/tests/mongodb/reindex_pipeline.rs`, do not copy `create_backend_with` for a larger pool. Replace its signature line and body

```rust
async fn create_backend_with(
    test_name: &str,
    configure: impl FnOnce(&mut MongoBackendConfig),
) -> Option<Arc<MongoBackend>> {
    let connection_string = shared_mongo::connection_string().await?;
    let mut config = MongoBackendConfig {
        connection_string,
        database_name: build_test_database_name(test_name),
        data_dir: Some(repo_data_dir()),
        ..Default::default()
    };
    configure(&mut config);
    build_backend(config).await.map(Arc::new)
}
```

(its doc comment stays above it, unchanged) with

```rust
async fn create_backend_with(
    test_name: &str,
    configure: impl FnOnce(&mut MongoBackendConfig),
) -> Option<Arc<MongoBackend>> {
    create_backend_with_pool(test_name, TEST_BACKEND_MAX_POOL, configure).await
}

/// [`create_backend_with`] with the connection pool set to, and capped at,
/// `pool` instead of the suite-wide [`TEST_BACKEND_MAX_POOL`]; the `$reindex`
/// write-stream tests need room for more than one stream (#1403).
pub(super) async fn create_backend_with_pool(
    test_name: &str,
    pool: u32,
    configure: impl FnOnce(&mut MongoBackendConfig),
) -> Option<Arc<MongoBackend>> {
    let connection_string = shared_mongo::connection_string().await?;
    let mut config = MongoBackendConfig {
        connection_string,
        database_name: build_test_database_name(test_name),
        data_dir: Some(repo_data_dir()),
        max_connections: pool,
        ..Default::default()
    };
    configure(&mut config);
    build_backend_with_pool(config, pool).await.map(Arc::new)
}
```

For every existing caller nothing changes: `build_backend` already capped the default pool of 10 at `TEST_BACKEND_MAX_POOL`, and a `configure` closure still runs last.

(b) In `crates/persistence/tests/mongodb/reindex_streams.rs`, change the imports to:

```rust
use super::*;

use std::collections::BTreeSet;

use helios_persistence::search::{ReindexSource, ResourcePage, TypeWalkPlan, TypeWalkRequest};

use super::reindex_id_walk::{
    backdate_fixture, capture_walk_logs, seed_walk_fixture, walk_log_lines,
};
use super::reindex_pipeline::create_backend_with_pool;
```

and add after `fn walk_ids`:

```rust
/// Pool of the write-stream tests' backends: `(10 - 2) / 2` admits four
/// streams for one rebuild, where the suite-wide cap of 4 admits one.
const STREAMS_TEST_POOL: u32 = 10;

async fn create_streams_backend(test_name: &str) -> Option<Arc<MongoBackend>> {
    create_backend_with_pool(test_name, STREAMS_TEST_POOL, |_| {}).await
}

/// The field names that follow `message` on a captured log line, in order.
fn field_names_after(line: &str, message: &str) -> Vec<String> {
    let (_, rest) = line
        .split_once(message)
        .unwrap_or_else(|| panic!("{line:?} does not carry {message:?}"));
    rest.split_whitespace()
        .filter_map(|token| token.split_once('=').map(|(name, _)| name.to_string()))
        .collect()
}

async fn server_major_version(db: &mongodb::Database) -> u32 {
    let info = db.run_command(doc! { "buildInfo": 1_i32 }).await.unwrap();
    info.get_str("version")
        .ok()
        .and_then(|v| v.split('.').next())
        .and_then(|major| major.parse().ok())
        .unwrap_or(0)
}

#[tokio::test]
async fn mongodb_plan_type_walk_ranges_cover_the_id_phase() {
    let Some(backend) = create_streams_backend("reindex_streams_plan_cover").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("streams-plan-cover");
    let fixture = seed_walk_fixture(&backend, &tenant, 400, "extra").await;
    backdate_fixture(&backend, &tenant, &fixture).await;

    let plan = backend
        .plan_type_walk(
            &tenant,
            "Observation",
            TypeWalkRequest {
                streams: 4,
                min_resources_per_stream: 50,
                concurrent_runs: 1,
            },
        )
        .await
        .unwrap();
    let (ranges, catch_up) = match plan {
        TypeWalkPlan::Ranges { ranges, catch_up } => (ranges, catch_up),
        TypeWalkPlan::Single => panic!("400 Observations at 50 per stream must plan four ranges"),
    };
    assert_eq!(ranges.len(), 4, "{ranges:?}");
    assert!(ranges.iter().all(|c| c.starts_with("v2|r|")), "{ranges:?}");
    assert!(catch_up.starts_with("v2|d|"), "{catch_up}");

    // A 1-byte cap takes exactly one resource per page; every page of a range
    // continues that range, and each range ends on one empty page.
    let mut seen = BTreeSet::new();
    for (i, range) in ranges.iter().enumerate() {
        let mut cursor = range.clone();
        let mut in_range: Vec<String> = Vec::new();
        let mut ended = false;
        for _ in 0..1_000 {
            let page = backend
                .fetch_resources_page_capped(&tenant, "Observation", Some(&cursor), 100, 1)
                .await
                .unwrap();
            let page_ids = ids_of(&page);
            match page.next_cursor {
                Some(next) => {
                    assert_eq!(page_ids.len(), 1, "range {i}: {page_ids:?}");
                    in_range.extend(page_ids);
                    cursor = next;
                }
                None => {
                    assert!(page_ids.is_empty(), "range {i} ends on one empty page");
                    ended = true;
                    break;
                }
            }
        }
        assert!(ended, "range {i} did not end within 1,000 pages");
        assert!(!in_range.is_empty(), "range {i} is empty");
        assert!(in_range.windows(2).all(|w| w[0] < w[1]), "range {i} is not in id order");
        for id in in_range {
            assert!(seen.insert(id.clone()), "{id} is in two ranges");
        }
    }
    assert_eq!(&seen, &fixture.live["Observation"]);
}

#[tokio::test]
async fn mongodb_plan_type_walk_fits_the_pool_and_the_type_size() {
    capture_walk_logs();
    let request = TypeWalkRequest {
        streams: 4,
        min_resources_per_stream: 50,
        concurrent_runs: 1,
    };

    // The suite-wide pool of 4 admits (4 - 2) / 2 = 1 stream.
    let Some(small_pool) = create_backend("reindex_streams_small_pool").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("streams-small-pool");
    seed_walk_fixture(&small_pool, &tenant, 200, "extra").await;
    for _ in 0..2 {
        assert_eq!(
            small_pool
                .plan_type_walk(&tenant, "Observation", request)
                .await
                .unwrap(),
            TypeWalkPlan::Single
        );
    }
    let needle = format!("tenant={}", tenant.tenant_id().as_str());
    let warned = walk_log_lines(&[
        "HFS_REINDEX_WRITE_STREAMS=4 needs HFS_MONGODB_MAX_CONNECTIONS ≥ 10; using 1",
        &needle,
    ]);
    assert_eq!(warned.len(), 1, "the clamp warns once per backend: {warned:?}");
    assert_eq!(
        warned[0].split_whitespace().nth(1),
        Some("WARN"),
        "{}",
        warned[0]
    );
    let planned = walk_log_lines(&["mongodb reindex streams planned", &needle]);
    assert_eq!(planned.len(), 2, "{planned:?}");
    for line in &planned {
        assert_eq!(
            field_names_after(line, "mongodb reindex streams planned"),
            [
                "tenant",
                "resource_type",
                "requested",
                "allowed",
                "resources",
                "streams",
                "plan_ms"
            ]
        );
        let tokens: Vec<&str> = line.split_whitespace().collect();
        for expected in ["requested=4", "allowed=1", "resources=0", "streams=1"] {
            assert!(tokens.contains(&expected), "{line}");
        }
    }

    // A pool of 10 shared by two rebuilds admits (10 - 2) / (2 * 2) = 2 streams.
    let Some(pool_ten) = create_streams_backend("reindex_streams_pool_ten").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("streams-pool-ten");
    seed_walk_fixture(&pool_ten, &tenant, 200, "extra").await;
    let plan = pool_ten
        .plan_type_walk(
            &tenant,
            "Observation",
            TypeWalkRequest {
                concurrent_runs: 2,
                ..request
            },
        )
        .await
        .unwrap();
    assert!(
        matches!(&plan, TypeWalkPlan::Ranges { ranges, .. } if ranges.len() == 2),
        "{plan:?}"
    );
    // A type too small for two streams of 1,000 keeps its single walk.
    let small_type = pool_ten
        .plan_type_walk(
            &tenant,
            "Observation",
            TypeWalkRequest {
                min_resources_per_stream: 1_000,
                ..request
            },
        )
        .await
        .unwrap();
    assert_eq!(small_type, TypeWalkPlan::Single);
    let needle = format!("tenant={}", tenant.tenant_id().as_str());
    let planned = walk_log_lines(&["mongodb reindex streams planned", &needle]);
    assert_eq!(planned.len(), 2, "{planned:?}");
    let first: Vec<&str> = planned[0].split_whitespace().collect();
    for expected in ["allowed=2", "resources=200", "streams=2"] {
        assert!(first.contains(&expected), "{}", planned[0]);
    }
    let second: Vec<&str> = planned[1].split_whitespace().collect();
    for expected in ["allowed=4", "resources=200", "streams=1"] {
        assert!(second.contains(&expected), "{}", planned[1]);
    }
}

#[tokio::test]
async fn mongodb_plan_type_walk_boundary_probes_are_covered() {
    use futures::stream::TryStreamExt;

    let Some(backend) = create_streams_backend("reindex_streams_probe_plan").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("streams-probe-plan");
    seed_walk_fixture(&backend, &tenant, 300, "extra").await;

    let db = backend.get_database().await.unwrap();
    // The suite's testcontainers mongod is a standalone that allows
    // profiling; a refusal is a failure, not a skip.
    db.run_command(doc! { "profile": 2_i32 })
        .await
        .expect("the test mongod must accept {profile: 2}");
    let plan = backend
        .plan_type_walk(
            &tenant,
            "Observation",
            TypeWalkRequest {
                streams: 4,
                min_resources_per_stream: 50,
                concurrent_runs: 1,
            },
        )
        .await
        .unwrap();
    let _ = db.run_command(doc! { "profile": 0_i32 }).await;
    assert!(
        matches!(&plan, TypeWalkPlan::Ranges { ranges, .. } if ranges.len() == 4),
        "{plan:?}"
    );

    let probes: Vec<Document> = db
        .collection::<Document>("system.profile")
        .find(doc! {
            "ns": format!("{}.resources", db.name()),
            "command.find": "resources",
            "command.skip": { "$exists": true },
        })
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(probes.len(), 3, "one probe per boundary: {probes:?}");

    let major = server_major_version(&db).await;
    for entry in &probes {
        let command = entry.get_document("command").unwrap();
        assert_eq!(command.get_str("hint").ok(), Some("idx_resources_identity"));
        let mut inner = Document::new();
        for key in ["find", "filter", "sort", "skip", "limit", "projection", "hint"] {
            if let Some(value) = command.get(key) {
                inner.insert(key, value.clone());
            }
        }
        let explain = db
            .run_command(doc! { "explain": inner, "verbosity": "executionStats" })
            .await
            .unwrap();
        let winning = explain
            .get_document("queryPlanner")
            .and_then(|qp| qp.get_document("winningPlan"))
            .unwrap()
            .clone();
        let mut names = Vec::new();
        collect_index_names(&winning, &mut names);
        assert!(
            !names.is_empty() && names.iter().all(|n| n == "idx_resources_identity"),
            "{names:?}"
        );
        assert!(!contains_stage_named(&winning, "SORT"), "a probe must not sort: {winning:?}");
        if major >= 7 {
            let docs_examined = explain
                .get_document("executionStats")
                .and_then(|s| {
                    s.get_i64("totalDocsExamined")
                        .or_else(|_| s.get_i32("totalDocsExamined").map(i64::from))
                })
                .unwrap();
            assert_eq!(docs_examined, 0, "a probe must be covered: {explain:?}");
        } else {
            eprintln!("MongoDB {major}: probe plan, not asserted covered below 7.0: {winning:?}");
        }
    }
}
```

- [ ] **Step 6: Run the integration tests**

```bash
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
CARGO_BUILD_JOBS=4 cargo test -p helios-persistence --features mongodb --test mongodb_tests reindex_streams 2>&1 | tail -40
```
Expected: `test result: ok. 6 passed` (Task 5's three and these three), no `Skipping` line. The harness runs `mongo:5.0.6`, so `mongodb_plan_type_walk_boundary_probes_are_covered` prints `MongoDB 5: probe plan, not asserted covered below 7.0: …` three times; paste those three lines into your report. Do not try to reach a 7.0 server: the covered-probe check on 7.0 is an open item for the controller.

- [ ] **Step 7: fmt, clippy, commit**

```bash
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
CARGO_BUILD_JOBS=4 cargo fmt -- crates/persistence/src/backends/mongodb/storage.rs crates/persistence/src/backends/mongodb/backend.rs crates/persistence/tests/mongodb_tests.rs crates/persistence/tests/mongodb/reindex_pipeline.rs crates/persistence/tests/mongodb/reindex_streams.rs
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
CARGO_BUILD_JOBS=4 cargo clippy -p helios-persistence --all-targets --all-features -- -D warnings -A clippy::items_after_test_module -A clippy::large_enum_variant -A clippy::question_mark -A clippy::collapsible_match -A clippy::collapsible_if -A clippy::field_reassign_with_default -A clippy::doc-overindented-list-items -A clippy::doc-lazy-continuation 2>&1 | tail -30
git add crates/persistence/src/backends/mongodb/storage.rs crates/persistence/src/backends/mongodb/backend.rs crates/persistence/tests/mongodb_tests.rs crates/persistence/tests/mongodb/reindex_pipeline.rs crates/persistence/tests/mongodb/reindex_streams.rs
git commit -m "$(cat <<'EOF'
feat(mongodb): plan concurrent reindex write streams on disjoint id ranges (#1403)

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Y61t4nTxKCT8WeE5NbxaVc
EOF
)"
```

---

### Task 7: Whole-rebuild tests: parity with one stream, the catch-up, and log order

Tests only: they exercise Tasks 4 and 6 together through a real MongoDB writer. Each test must fail if the behaviour it names regresses (the parity test compares against a cleared single-stream rebuild; the catch-up test's resource is invisible to every range; the log-order test's probe fires if `id phase finished` appears during any range page write).

**Files:**
- Modify: `crates/persistence/tests/mongodb/reindex_pipeline.rs` — `struct PhaseLogProbeTarget` (`:1628-1631`).
- Modify: `crates/persistence/tests/mongodb/reindex_streams.rs`.

**Interfaces:**
- Consumes: `seed_walk_fixture`, `backdate_fixture`, `snapshot`, `wait_for_terminal`, `capture_walk_logs`, `walk_log_lines` (`reindex_id_walk.rs`); `PhaseLogProbeTarget { backend: Arc<MongoBackend>, saw_transition_early: Arc<AtomicBool> }` (`reindex_pipeline.rs`); `super::bulk_submit::seed`, `search_index_entry_count` (`mongodb_tests.rs`); `create_backend_with_pool` (`reindex_pipeline.rs`, Task 6); `create_streams_backend`, `field_names_after`, `ids_of` (Tasks 5–6).
- Produces: `struct RangeCountingSource` (a `ReindexSource` wrapper counting what `v2|r|` fetches return, recording plans, and running an optional mutation on a chosen range's first fetch).

- [ ] **Step 1: Share `PhaseLogProbeTarget`**

In `reindex_pipeline.rs`, change

```rust
struct PhaseLogProbeTarget {
    backend: Arc<MongoBackend>,
    saw_transition_early: Arc<std::sync::atomic::AtomicBool>,
}
```

to

```rust
pub(super) struct PhaseLogProbeTarget {
    pub(super) backend: Arc<MongoBackend>,
    pub(super) saw_transition_early: Arc<std::sync::atomic::AtomicBool>,
}
```

- [ ] **Step 2: Write the tests**

In `reindex_streams.rs`, change the imports to:

```rust
use super::*;

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use async_trait::async_trait;
use helios_persistence::error::StorageResult;
use helios_persistence::search::{
    ReindexOperation, ReindexRequest, ReindexSource, ReindexStatus, ReindexTarget, ResourcePage,
    TypeWalkPlan, TypeWalkRequest,
};
use helios_persistence::types::StoredResource;

use super::reindex_id_walk::{
    backdate_fixture, capture_walk_logs, seed_walk_fixture, snapshot, wait_for_terminal,
    walk_log_lines,
};
use super::reindex_pipeline::create_backend_with_pool;
```

and append:

```rust
/// A mutation run from inside a source fetch.
type Mutation = Box<dyn Fn() -> futures::future::BoxFuture<'static, ()> + Send + Sync>;

/// Delegates every `ReindexSource` method to a real backend, and counts what
/// the id ranges return: fetches whose cursor carries the backend's range tag
/// `v2|r|`, ahead fetches included when they return a page. Keeps every plan
/// it hands the driver, and can run a mutation once, right after the first
/// fetch of one range returns and before the driver gets that page (#1403).
struct RangeCountingSource {
    inner: Arc<MongoBackend>,
    range_fetches: AtomicU64,
    range_resources: AtomicU64,
    plans: std::sync::Mutex<Vec<TypeWalkPlan>>,
    trigger: Option<(usize, Mutation)>,
    fired: AtomicBool,
}

impl RangeCountingSource {
    fn new(inner: Arc<MongoBackend>) -> Self {
        Self {
            inner,
            range_fetches: AtomicU64::new(0),
            range_resources: AtomicU64::new(0),
            plans: std::sync::Mutex::new(Vec::new()),
            trigger: None,
            fired: AtomicBool::new(false),
        }
    }

    /// Runs `mutation` once, after the first fetch of range `range` (the
    /// cursor the plan handed out for it) returns.
    fn with_trigger(mut self, range: usize, mutation: Mutation) -> Self {
        self.trigger = Some((range, mutation));
        self
    }

    fn count(&self, cursor: Option<&str>, page: &ResourcePage) {
        if cursor.is_some_and(|c| c.starts_with("v2|r|")) {
            self.range_fetches.fetch_add(1, Ordering::SeqCst);
            self.range_resources
                .fetch_add(page.resources.len() as u64, Ordering::SeqCst);
        }
    }

    fn range_start(&self, range: usize) -> Option<String> {
        match self.plans.lock().unwrap().last() {
            Some(TypeWalkPlan::Ranges { ranges, .. }) => ranges.get(range).cloned(),
            _ => None,
        }
    }
}

#[async_trait]
impl ReindexSource for RangeCountingSource {
    async fn list_resource_types(&self, tenant: &TenantContext) -> StorageResult<Vec<String>> {
        self.inner.list_resource_types(tenant).await
    }

    async fn count_resources(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
    ) -> StorageResult<u64> {
        self.inner.count_resources(tenant, resource_type).await
    }

    async fn fetch_resources_page(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        cursor: Option<&str>,
        limit: u32,
    ) -> StorageResult<ResourcePage> {
        let page = self
            .inner
            .fetch_resources_page(tenant, resource_type, cursor, limit)
            .await?;
        self.count(cursor, &page);
        Ok(page)
    }

    async fn fetch_resources_page_capped(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        cursor: Option<&str>,
        limit: u32,
        max_bytes: u64,
    ) -> StorageResult<ResourcePage> {
        let page = self
            .inner
            .fetch_resources_page_capped(tenant, resource_type, cursor, limit, max_bytes)
            .await?;
        self.count(cursor, &page);
        if let Some((range, mutation)) = &self.trigger
            && cursor.is_some()
            && cursor.map(str::to_string) == self.range_start(*range)
            && !self.fired.swap(true, Ordering::SeqCst)
        {
            mutation().await;
        }
        Ok(page)
    }

    async fn fetch_resources_by_ids(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        ids: &[String],
    ) -> StorageResult<Vec<StoredResource>> {
        self.inner
            .fetch_resources_by_ids(tenant, resource_type, ids)
            .await
    }

    fn may_prefetch_page(&self, cursor: &str) -> bool {
        self.inner.may_prefetch_page(cursor)
    }

    async fn fetch_resources_page_ahead(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        cursor: &str,
        limit: u32,
        max_bytes: u64,
    ) -> StorageResult<Option<ResourcePage>> {
        let page = self
            .inner
            .fetch_resources_page_ahead(tenant, resource_type, cursor, limit, max_bytes)
            .await?;
        if let Some(page) = &page {
            self.count(Some(cursor), page);
        }
        Ok(page)
    }

    async fn plan_type_walk(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        request: TypeWalkRequest,
    ) -> StorageResult<TypeWalkPlan> {
        let plan = self
            .inner
            .plan_type_walk(tenant, resource_type, request)
            .await?;
        self.plans.lock().unwrap().push(plan.clone());
        Ok(plan)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mongodb_reindex_write_streams_match_single_stream() {
    let Some(backend) = create_streams_backend("reindex_streams_match_single").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant_a = create_tenant("streams-match-a");
    let tenant_b = create_tenant("streams-match-b");
    let fixture_a = seed_walk_fixture(&backend, &tenant_a, 2_000, "extra").await;
    backdate_fixture(&backend, &tenant_a, &fixture_a).await;
    let fixture_b = seed_walk_fixture(&backend, &tenant_b, 200, "extra").await;
    backdate_fixture(&backend, &tenant_b, &fixture_b).await;
    let live_observations = fixture_a.live["Observation"].len() as u64;

    let plan = backend
        .plan_type_walk(
            &tenant_a,
            "Observation",
            TypeWalkRequest {
                streams: 4,
                min_resources_per_stream: 500,
                concurrent_runs: 1,
            },
        )
        .await
        .unwrap();
    match &plan {
        TypeWalkPlan::Ranges { ranges, catch_up } => {
            assert_eq!(ranges.len(), 4, "{plan:?}");
            assert!(ranges.iter().all(|c| c.starts_with("v2|r|")), "{ranges:?}");
            assert!(catch_up.starts_with("v2|d|"), "{catch_up}");
        }
        TypeWalkPlan::Single => {
            panic!("2,000 Observations at 500 per stream must plan four ranges")
        }
    }

    let db = backend.get_database().await.unwrap();
    let tenant_b_before = snapshot(&db, "streams-match-b", false).await;

    let mut snapshots = Vec::new();
    for (label, streams, batch_bytes) in [
        ("one stream", 1, 0),
        ("four streams", 4, 0),
        ("four streams, 4 KiB pages", 4, 4096),
    ] {
        let source = Arc::new(RangeCountingSource::new(backend.clone()));
        let op = ReindexOperation::with_parts(
            source.clone(),
            vec![backend.clone() as Arc<dyn ReindexTarget>],
            backend.tenant_registries().clone(),
        );
        let job = op
            .start(
                tenant_a.clone(),
                ReindexRequest::for_types(["Observation"])
                    .with_batch_size(100)
                    .with_batch_bytes(batch_bytes)
                    .with_write_streams(streams)
                    .with_min_resources_per_stream(500)
                    .clear_existing(),
                None,
            )
            .await
            .unwrap();
        let progress = wait_for_terminal(&op, &job).await;
        assert_eq!(progress.status, ReindexStatus::Completed, "{label}: {progress:?}");
        assert!(progress.errors.is_empty(), "{label}: {:?}", progress.errors);
        if streams > 1 {
            assert_eq!(
                source.range_resources.load(Ordering::SeqCst),
                live_observations,
                "{label}: the ranges must return every live Observation exactly once"
            );
            let plans = source.plans.lock().unwrap().clone();
            assert!(
                matches!(plans.as_slice(), [TypeWalkPlan::Ranges { ranges, .. }] if ranges.len() == 4),
                "{label}: {plans:?}"
            );
        } else {
            assert_eq!(
                source.range_fetches.load(Ordering::SeqCst),
                0,
                "{label}: a single walk never uses a range cursor"
            );
        }
        snapshots.push((label, snapshot(&db, "streams-match-a", false).await));
    }

    let (_, baseline) = &snapshots[0];
    assert!(
        !baseline.0.is_empty(),
        "the single-stream rebuild must have written search_index rows"
    );
    for (label, rows) in &snapshots[1..] {
        assert_eq!(rows, baseline, "{label}: rows differ from the single-stream rebuild");
    }
    assert_eq!(
        snapshot(&db, "streams-match-b", false).await,
        tenant_b_before,
        "tenant B's rows must be untouched"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mongodb_streams_catch_up_indexes_a_resource_created_during_the_ranges() {
    use helios_persistence::core::{BulkProcessingOptions, BulkSubmitProvider, NdjsonEntry};

    let Some(backend) = create_streams_backend("reindex_streams_catch_up").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("streams-catch-up");
    let fixture = seed_walk_fixture(&backend, &tenant, 800, "extra").await;
    backdate_fixture(&backend, &tenant, &fixture).await;
    let (submission, manifest) = super::bulk_submit::seed(&backend, &tenant).await;
    let live_observations = fixture.live["Observation"].len() as u64;

    let mutation_backend = backend.clone();
    let mutation_tenant = tenant.clone();
    let mutation: Mutation = Box::new(move || {
        let backend = mutation_backend.clone();
        let tenant = mutation_tenant.clone();
        let submission = submission.clone();
        let manifest = manifest.clone();
        Box::pin(async move {
            backend
                .process_entries(
                    &tenant,
                    &submission,
                    &manifest,
                    vec![NdjsonEntry::new(
                        1,
                        "Observation",
                        json!({
                            "resourceType": "Observation",
                            "id": "--created-mid-walk",
                            "status": "final",
                            "code": { "coding": [{ "system": "http://loinc.org", "code": "8867-4" }] },
                        }),
                    )],
                    &BulkProcessingOptions::new().with_defer_indexing(true),
                )
                .await
                .unwrap();
            // A deferred create writes no search rows of its own, so rows
            // found at the end can only come from the rebuild.
            assert_eq!(
                search_index_entry_count(&backend, &tenant, "Observation", "--created-mid-walk")
                    .await,
                0
            );
        }) as futures::future::BoxFuture<'static, ()>
    });
    let source =
        Arc::new(RangeCountingSource::new(backend.clone()).with_trigger(1, mutation));
    let op = ReindexOperation::with_parts(
        source.clone(),
        vec![backend.clone() as Arc<dyn ReindexTarget>],
        backend.tenant_registries().clone(),
    );
    let job = op
        .start(
            tenant.clone(),
            ReindexRequest::for_types(["Observation"])
                .with_batch_size(50)
                .with_write_streams(4)
                .with_min_resources_per_stream(100),
            None,
        )
        .await
        .unwrap();
    let progress = wait_for_terminal(&op, &job).await;
    assert_eq!(progress.status, ReindexStatus::Completed, "{progress:?}");
    assert!(source.fired.load(Ordering::SeqCst), "the create ran during range 1");
    assert!(source.range_fetches.load(Ordering::SeqCst) > 0);
    assert_eq!(
        source.range_resources.load(Ordering::SeqCst),
        live_observations,
        "the ranges walk exactly the Observations stamped before the floor"
    );
    assert!(
        search_index_entry_count(&backend, &tenant, "Observation", "--created-mid-walk").await > 0,
        "the catch-up must index a resource created while the ranges ran"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mongodb_streams_log_id_phase_finished_after_every_range_page_is_written() {
    let Some(backend) = create_streams_backend("reindex_streams_log_order").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("streams-log-order");
    let fixture = seed_walk_fixture(&backend, &tenant, 400, "extra").await;
    backdate_fixture(&backend, &tenant, &fixture).await;

    capture_walk_logs();
    let saw_transition_early = Arc::new(AtomicBool::new(false));
    let probe = Arc::new(super::reindex_pipeline::PhaseLogProbeTarget {
        backend: backend.clone(),
        saw_transition_early: saw_transition_early.clone(),
    });
    let op = ReindexOperation::with_parts(
        backend.clone(),
        vec![probe as Arc<dyn ReindexTarget>],
        backend.tenant_registries().clone(),
    );
    let job = op
        .start(
            tenant.clone(),
            ReindexRequest::for_types(["Observation"])
                .with_batch_size(50)
                .with_write_streams(4)
                .with_min_resources_per_stream(50),
            None,
        )
        .await
        .unwrap();
    let progress = wait_for_terminal(&op, &job).await;
    assert_eq!(progress.status, ReindexStatus::Completed, "{progress:?}");
    assert!(
        !saw_transition_early.load(Ordering::SeqCst),
        "id phase finished must never be logged while a range page is being written"
    );

    let needle = format!("tenant={}", tenant.tenant_id().as_str());
    assert_eq!(
        walk_log_lines(&["mongodb reindex walk started", &needle]).len(),
        1,
        "the plan fixes the floor once for the whole type"
    );
    assert_eq!(
        walk_log_lines(&["mongodb reindex id phase finished", &needle]).len(),
        1
    );
    let planned = walk_log_lines(&["mongodb reindex streams planned", &needle]);
    assert_eq!(planned.len(), 1, "{planned:?}");
    let tokens: Vec<&str> = planned[0].split_whitespace().collect();
    for expected in ["requested=4", "allowed=4", "streams=4"] {
        assert!(tokens.contains(&expected), "{}", planned[0]);
    }
    let ranges = walk_log_lines(&["mongodb reindex id range finished", &needle]);
    assert_eq!(ranges.len(), 4, "{ranges:?}");
    for line in &ranges {
        assert_eq!(
            field_names_after(line, "mongodb reindex id range finished"),
            ["tenant", "resource_type", "floor", "lo", "hi"]
        );
    }
    let open_lo = ranges
        .iter()
        .filter(|l| l.split_whitespace().any(|t| t == "lo=*"))
        .count();
    let open_hi = ranges
        .iter()
        .filter(|l| l.split_whitespace().any(|t| t == "hi=*"))
        .count();
    assert_eq!((open_lo, open_hi), (1, 1), "{ranges:?}");
}
```

- [ ] **Step 3: Run the tests**

```bash
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
CARGO_BUILD_JOBS=4 cargo test -p helios-persistence --features mongodb --test mongodb_tests reindex_streams 2>&1 | tail -40
```
Expected: `test result: ok. 9 passed` and no `Skipping` line. If one of the three new tests fails, that is a defect in Tasks 4–6 (or in the test), not a flake: report the failing assertion and stop.

- [ ] **Step 4: Run the whole MongoDB reindex suite**

```bash
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
CARGO_BUILD_JOBS=4 cargo test -p helios-persistence --features mongodb --test mongodb_tests reindex 2>&1 | tail -60
```
Expected: `test result: ok.` for every `reindex_id_walk`, `reindex_pipeline` and `reindex_streams` test.

- [ ] **Step 5: fmt, clippy, commit**

```bash
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
CARGO_BUILD_JOBS=4 cargo fmt -- crates/persistence/tests/mongodb/reindex_pipeline.rs crates/persistence/tests/mongodb/reindex_streams.rs
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
CARGO_BUILD_JOBS=4 cargo clippy -p helios-persistence --all-targets --all-features -- -D warnings -A clippy::items_after_test_module -A clippy::large_enum_variant -A clippy::question_mark -A clippy::collapsible_match -A clippy::collapsible_if -A clippy::field_reassign_with_default -A clippy::doc-overindented-list-items -A clippy::doc-lazy-continuation 2>&1 | tail -30
git add crates/persistence/tests/mongodb/reindex_pipeline.rs crates/persistence/tests/mongodb/reindex_streams.rs
git commit -m "$(cat <<'EOF'
test(mongodb): write-stream rebuild parity, catch-up and log-order tests (#1403)

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Y61t4nTxKCT8WeE5NbxaVc
EOF
)"
```

---

### Task 8: `HFS_REINDEX_WRITE_STREAMS` in the server, the hook and the docs

**Files:**
- Modify: `crates/rest/src/config.rs` — after `pub reindex_batch_bytes: u64,` (`:1325-1326`); `impl Default for ServerConfig` (`:1602`); `fn for_testing` (`:1872`); `mod tests` (after `fn test_elasticsearch_rebuild_knob_defaults`, `:2471-2483`).
- Modify: `crates/hfs/src/main.rs` — `fn build_automatic_reindex_hook` (`:1941-1958`); `mod tests` (after `fn test_automatic_reindex_hook_gets_the_server_default_batch_bytes_when_unset`, `:3888-3911`).
- Modify: `README.md:309`, `crates/hfs/README.md:113`, `book/src/configuration/environment-variables.md:65` (each after its `HFS_REINDEX_BATCH_BYTES` row); `docs/mongodb/search-indexes.md` (after the paragraph that starts "The floor is the earlier of two times", `:61`).
- Modify: `.claude/skills/bulk-data-submit/SKILL.md` (the bullet that starts `- **Rebuild knobs (#1125).**`, `:145`, and the bullet that starts `- The rebuild logs \`reindex job started\``, `:150`) and `.agents/skills/bulk-data-submit/SKILL.md` (the same two bullets, `:142` and `:135`): the same edit in both copies.

**Interfaces:**
- Consumes: `ReindexOnFinish::{with_write_streams, write_streams}` (Task 1).
- Produces: `ServerConfig.reindex_write_streams: u32` (`--reindex-write-streams`, env `HFS_REINDEX_WRITE_STREAMS`, default 1).

- [ ] **Step 1: Write the failing tests**

`crates/rest/src/config.rs`, `mod tests`, after `fn test_elasticsearch_rebuild_knob_defaults`:

```rust
    #[test]
    fn test_reindex_write_streams_default_and_flag() {
        let parsed = ServerConfig::try_parse_from(["rest-server"]).unwrap();
        for config in [parsed, ServerConfig::default(), ServerConfig::for_testing()] {
            assert_eq!(config.reindex_write_streams, 1);
        }
        let parsed =
            ServerConfig::try_parse_from(["rest-server", "--reindex-write-streams", "4"]).unwrap();
        assert_eq!(parsed.reindex_write_streams, 4);
        assert!(parsed.validate().is_ok());
        assert!(
            ServerConfig::try_parse_from(["rest-server", "--reindex-write-streams", "many"])
                .is_err()
        );
    }
```

`crates/hfs/src/main.rs`, `mod tests`, after `fn test_automatic_reindex_hook_gets_the_server_default_batch_bytes_when_unset`:

```rust
    #[cfg(feature = "sqlite")]
    #[test]
    fn test_automatic_reindex_hook_gets_the_write_streams_setting() {
        use clap::Parser;

        let backend = Arc::new(
            create_sqlite_backend(&ServerConfig {
                database_url: Some(":memory:".to_string()),
                ..Default::default()
            })
            .unwrap(),
        );
        let registries = backend.tenant_registries().clone();
        let op = Arc::new(ReindexOperation::new(backend, registries));

        for (args, expected) in [
            (vec!["rest-server"], 1),
            (vec!["rest-server", "--reindex-write-streams", "4"], 4),
            (vec!["rest-server", "--reindex-write-streams", "40"], 16),
        ] {
            let config = ServerConfig::try_parse_from(args.clone()).unwrap();
            let hook = build_automatic_reindex_hook(op.clone(), &config, None);
            assert_eq!(hook.write_streams(), expected, "{args:?}");
        }
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
CARGO_BUILD_JOBS=4 cargo test -p helios-rest --lib config::tests::test_reindex_write_streams 2>&1 | tail -20
```
Expected: compile error `no field reindex_write_streams on type ServerConfig`.

- [ ] **Step 3: Implement**

(a) `crates/rest/src/config.rs`, after `pub reindex_batch_bytes: u64,`:

```rust

    /// Concurrent insert streams per resource type in the automatic
    /// post-import rebuild, standalone MongoDB only (#1403). Each stream walks
    /// its own contiguous id range, then one catch-up pass runs. `1` (the
    /// default) is the single walk. Values above 16 are clamped, and the
    /// backend lowers it to fit `HFS_MONGODB_MAX_CONNECTIONS` (2 connections
    /// per stream per concurrent rebuild, plus 2). A type split across
    /// streams never pages uncapped: with `HFS_REINDEX_BATCH_BYTES=0` its
    /// pages are capped at 32 MiB. Ignored by other backends and by
    /// `POST $reindex`.
    #[arg(long, env = "HFS_REINDEX_WRITE_STREAMS", default_value = "1")]
    pub reindex_write_streams: u32,
```

and add `reindex_write_streams: 1,` right after `reindex_batch_bytes: 32 * 1024 * 1024,` in both `impl Default for ServerConfig` and `fn for_testing`.

(b) `crates/hfs/src/main.rs`, in `build_automatic_reindex_hook`, change

```rust
    .with_batch_bytes(config.reindex_batch_bytes)
    .with_bulk_index_rebuild(config.bulk_submit.bulk_index_rebuild);
```

to

```rust
    .with_batch_bytes(config.reindex_batch_bytes)
    .with_write_streams(config.reindex_write_streams)
    .with_bulk_index_rebuild(config.bulk_submit.bulk_index_rebuild);
```

(Leave the S3 branch's `ReindexOnFinish::new(op)` alone: its `ops.reindex` is always `None`.)

(c) `README.md` and `book/src/configuration/environment-variables.md` — after the `HFS_REINDEX_BATCH_BYTES` row, add:

```markdown
| `HFS_REINDEX_WRITE_STREAMS` | `1` | Concurrent insert streams per resource type in the automatic rebuild, standalone MongoDB only (ignored by other backends, when Elasticsearch serves search, and by `POST $reindex`). Each stream walks its own contiguous id range; one catch-up pass follows. Clamped to 16, and lowered to fit `HFS_MONGODB_MAX_CONNECTIONS`: 2 connections per stream for each concurrent rebuild (`HFS_BULK_SUBMIT_WORKER_CONCURRENCY`), plus 2. A type split across streams never pages uncapped: with `HFS_REINDEX_BATCH_BYTES=0` its pages are capped at 32 MiB |
```

`crates/hfs/README.md` — after its `HFS_REINDEX_BATCH_BYTES` row (this table ends each description with a period), add:

```markdown
| `HFS_REINDEX_WRITE_STREAMS` | `1` | Concurrent insert streams per resource type in the automatic rebuild, standalone MongoDB only (ignored by other backends, when Elasticsearch serves search, and by `POST $reindex`). Each stream walks its own contiguous id range; one catch-up pass follows. Clamped to 16, and lowered to fit `HFS_MONGODB_MAX_CONNECTIONS`: 2 connections per stream for each concurrent rebuild (`HFS_BULK_SUBMIT_WORKER_CONCURRENCY`), plus 2. A type split across streams never pages uncapped: with `HFS_REINDEX_BATCH_BYTES=0` its pages are capped at 32 MiB. |
```

(d) `docs/mongodb/search-indexes.md`, in "How `$reindex` walks a type (#1403)", after the paragraph that starts "The floor is the earlier of two times", add:

```markdown
With `HFS_REINDEX_WRITE_STREAMS` above 1, the rebuild that follows a fast-load import splits a type's id phase into up to that many contiguous id ranges and writes them concurrently, one insert stream each, so every stream keeps its own append points in each index; interleaving ids across streams would scatter the keys again. The floor is fixed once for the whole type. The range boundaries come from covered probes of `idx_resources_identity` and only affect balance: every id falls in exactly one range. The catch-up rounds run once, after every range has been written. A type with fewer than 50,000 resources per stream, a deployment where Elasticsearch serves search, `POST $reindex`, and a retry of named resources keep one walk. Each stream uses up to 2 connections for each concurrent rebuild, and 2 more are left for requests: when `HFS_MONGODB_MAX_CONNECTIONS` is below `2 × streams × HFS_BULK_SUBMIT_WORKER_CONCURRENCY + 2`, the rebuild runs fewer streams and warns once. A split type's pages are never uncapped: with `HFS_REINDEX_BATCH_BYTES=0` they are capped at 32 MiB, since each stream holds up to two pages. The log shows `mongodb reindex streams planned` for each type it plans and `mongodb reindex id range finished` for each range.
```

(e) Both bulk-data-submit SKILL copies (`.claude/skills/…` and `.agents/skills/…`), the same two edits in each:

- In the bullet that starts `- **Rebuild knobs (#1125).**`, after the sentence that ends `the Elasticsearch and S3 sources page by count only.`, insert:

  ```markdown
  `HFS_REINDEX_WRITE_STREAMS` (default `1`, clamped to 16; standalone MongoDB only, and never for `POST $reindex`) splits each large type of the automatic rebuild into that many contiguous id ranges written concurrently, then runs one catch-up pass (#1403). It is lowered to fit `HFS_MONGODB_MAX_CONNECTIONS` (2 connections per stream for each concurrent rebuild, plus 2) with one warning, and a split type's pages are capped at 32 MiB even when `HFS_REINDEX_BATCH_BYTES=0`.
  ```

- At the end of the bullet that starts `- The rebuild logs \`reindex job started\``, append:

  ```markdown
  `reindex job started` carries `write_streams` (the streams per type the run asked for); `reindex type finished` carries `streams` (the streams the type used) and `plan_ms`. With `streams` above 1 the phase fields are summed over the streams while `type_elapsed_ms` stays wall time, so `other_ms` must not be read; `(fetch_wait_ms + write_ms + yield_ms) / type_elapsed_ms` is the type's effective concurrency. MongoDB also logs `mongodb reindex streams planned` per planned type and `mongodb reindex id range finished` per range.
  ```

Check both copies carry both additions:

```bash
for f in .claude/skills/bulk-data-submit/SKILL.md .agents/skills/bulk-data-submit/SKILL.md; do
  grep -c -F -e 'splits each large type of the automatic rebuild' -e 'so `other_ms` must not be read' "$f"
done
```
Expected: `2`, then `2`.

- [ ] **Step 4: Run the tests to verify they pass**

```bash
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
CARGO_BUILD_JOBS=4 cargo test -p helios-rest --lib config::tests 2>&1 | tail -20
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
CARGO_BUILD_JOBS=4 cargo test -p helios-hfs test_automatic_reindex_hook 2>&1 | tail -20
```
Expected: `test result: ok.` for both, including `test_reindex_write_streams_default_and_flag`, `test_automatic_reindex_hook_gets_the_write_streams_setting` and PR2a's `…_batch_bytes_when_unset`.

- [ ] **Step 5: fmt, clippy, commit**

```bash
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
CARGO_BUILD_JOBS=4 cargo fmt -- crates/rest/src/config.rs crates/hfs/src/main.rs
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
CARGO_BUILD_JOBS=4 cargo clippy -p helios-hfs --features mongodb -- -D warnings -A clippy::items_after_test_module -A clippy::large_enum_variant -A clippy::question_mark -A clippy::collapsible_match -A clippy::collapsible_if -A clippy::field_reassign_with_default -A clippy::doc-overindented-list-items -A clippy::doc-lazy-continuation 2>&1 | tail -30
git add crates/rest/src/config.rs crates/hfs/src/main.rs README.md crates/hfs/README.md book/src/configuration/environment-variables.md docs/mongodb/search-indexes.md .claude/skills/bulk-data-submit/SKILL.md .agents/skills/bulk-data-submit/SKILL.md
git commit -m "$(cat <<'EOF'
feat(config): HFS_REINDEX_WRITE_STREAMS for the automatic MongoDB rebuild (#1403)

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Y61t4nTxKCT8WeE5NbxaVc
EOF
)"
```

---

### Task 9: Full verification and the PR body (no push, no PR)

**Files:**
- Create: `<SDD workspace>/pr-body.md` (git-ignored; resolved in Step 4).

**Interfaces:** none.

- [ ] **Step 1: Regression suites**

Run each command on its own, in the foreground, each after its own lock check. First check the disk:

```bash
df -h /c
```
If `C:` has less than 70 GB available, run the lock check, then `CARGO_BUILD_JOBS=4 cargo clean -p helios-rest`, before going on.

```bash
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
CARGO_BUILD_JOBS=4 cargo test -p helios-persistence --lib 2>&1 | tail -15
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
CARGO_BUILD_JOBS=4 cargo test -p helios-persistence --features mongodb --lib backends::mongodb 2>&1 | tail -15
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
CARGO_BUILD_JOBS=4 cargo test -p helios-persistence --features mongodb --test mongodb_tests 2>&1 | tail -30
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
CARGO_BUILD_JOBS=4 cargo test -p helios-persistence --features postgres --test postgres_tests reindex 2>&1 | tail -30
df -h /c
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
CARGO_BUILD_JOBS=4 cargo test -p helios-rest reindex 2>&1 | tail -20
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
CARGO_BUILD_JOBS=4 cargo test -p helios-rest --lib config 2>&1 | tail -20
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
CARGO_BUILD_JOBS=4 cargo test -p helios-hfs test_automatic_reindex_hook 2>&1 | tail -20
```
Before `cargo test -p helios-rest reindex` (it builds every `helios-rest` integration binary, ~46 GB), the second `df -h /c` must again show at least 70 GB available; otherwise run the lock check and `CARGO_BUILD_JOBS=4 cargo clean -p helios-rest` first. Expected: every test command ends `test result: ok.`; the whole `mongodb_tests` binary passes with no `Skipping` line for the `reindex_*` tests; the PostgreSQL reindex suite (a backend that inherits the default `Single` plan) passes unchanged.

- [ ] **Step 2: Clippy gates and formatting**

Run the three commands of **Clippy gates** (Global Constraints), each after its lock check, then:

```bash
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
CARGO_BUILD_JOBS=4 cargo fmt -- --check crates/persistence/src/search/reindex.rs crates/persistence/src/search/reindex_stats.rs crates/persistence/src/search/mod.rs crates/persistence/src/backends/mongodb/storage.rs crates/persistence/src/backends/mongodb/backend.rs crates/persistence/tests/mongodb_tests.rs crates/persistence/tests/mongodb/reindex_id_walk.rs crates/persistence/tests/mongodb/reindex_pipeline.rs crates/persistence/tests/mongodb/reindex_streams.rs crates/rest/src/config.rs crates/hfs/src/main.rs
git status --short -uno -- crates docs book README.md .claude .agents
git log --oneline perf/1403-pr2b-mongodb-reindex-overlap..HEAD
```
Expected: clippy clean ×3; `cargo fmt --check` prints nothing; `git status` prints nothing (every change committed); the log shows nine commits: the plan's, then one for each of Tasks 1–8.

- [ ] **Step 3: Plan-label scan of the new code**

```bash
git diff perf/1403-pr2b-mongodb-reindex-overlap..HEAD -- crates docs README.md book .claude .agents ':(exclude)docs/superpowers' | grep -n -E '^\+.*(PR[0-4]\b|\bS[1-5]\b|§|\bD[0-9]+\b|\bI1\b|Task [0-9]|\.rs:[0-9]+)' || echo "no plan labels"
```
Expected: `no plan labels`. (The plan itself, under `docs/superpowers`, is excluded: it is made of those labels.) A hit that names Amazon S3 as a storage backend is not a plan label; anything else is. Fix and amend nothing: make a new commit if anything is found.

- [ ] **Step 4: Write `pr-body.md` into the SDD workspace**

```bash
WS=$(bash /c/Users/DougC/.claude/plugins/cache/claude-plugins-official/superpowers/6.3.0/skills/subagent-driven-development/scripts/sdd-workspace docs/superpowers/plans/2026-09-25-1403-pr3-mongodb-write-streams.md)
echo "$WS"
```

Write `$WS/pr-body.md` with this content (tick each Test plan box only for a command you ran in Step 1–2 and saw pass):

````markdown
## Summary
- Adds concurrent write streams to the MongoDB deferred search-index rebuild (#1403): a type is split into up to `HFS_REINDEX_WRITE_STREAMS` contiguous id ranges, each walked and written by its own insert stream, and one catch-up walk runs once every range has been written.
- Standalone MongoDB only. Every other backend, deployments where Elasticsearch serves search, `POST $reindex`, and retries of named resources keep the single walk. The default, `1`, leaves today's behaviour unchanged.
- Streams fit the pool and the type: `min(HFS_REINDEX_WRITE_STREAMS, (HFS_MONGODB_MAX_CONNECTIONS − 2) / (2 × HFS_BULK_SUBMIT_WORKER_CONCURRENCY), resources / 50,000)`; lowering the request warns once.
- Correctness: disjoint ranges keep their own append points; at most one page's delete/insert is in flight per stream, in fetch order; each range ends on an empty query; the catch-up rounds start only after the last range page is written, so a write that misses its range is picked up exactly as a single walk's round 1 would pick it up. A split type's pages are never uncapped (32 MiB when `HFS_REINDEX_BATCH_BYTES=0`).

## Operator notes
- New `HFS_REINDEX_WRITE_STREAMS` (default `1`, clamped to 16), standalone MongoDB only.
- Give it `HFS_MONGODB_MAX_CONNECTIONS ≥ 2 × streams × HFS_BULK_SUBMIT_WORKER_CONCURRENCY + 2`, more if ingest overlaps the rebuild; otherwise the rebuild runs fewer streams and logs `HFS_REINDEX_WRITE_STREAMS=… needs HFS_MONGODB_MAX_CONNECTIONS ≥ …; using …` once.
- New log lines: `mongodb reindex streams planned` (per planned type) and `mongodb reindex id range finished` (per range); `reindex job started` gains `write_streams`, and `reindex type finished` gains `streams` and `plan_ms`. With `streams` above 1 that line's phase fields are summed over the streams.
- The recommended K goes into `docs/mongodb/bulk-import-sizing.md` after the K arms; the default stays 1.

## Design
- Spec: `docs/superpowers/specs/2026-09-23-mongodb-reindex-rebuild-design.md` §4.5.
- File-level design: `manual-test/archive/1403-run17-evidence/design/S5-followups-docs.md` §1 (local evidence, not committed); the plan this PR implements is `docs/superpowers/plans/2026-09-25-1403-pr3-mongodb-write-streams.md`.
- Trigger: B2-s write-busy 0.79–0.83 (T1 not met); half-cache probe Observation Q4/Q1 0.629 (T2 met).

## Gate (PR3 + CU-3)
Pending — run by the orchestrator on this branch: `K1-x`, `K4-x` (and `K2-x` if K4 passes G3.1) at C_K = C_bench/2, `CU-3-ref` + `CU-3`, then `P3-prov` at the adopted K; every arm with `HFS_MONGODB_MAX_CONNECTIONS=18`. Results tables are pasted here with `gh pr edit` once they exist. If K4-x fails, this PR does not merge and the numbers go on #1403.

## Test plan
- [ ] `cargo test -p helios-persistence --lib`
- [ ] `cargo test -p helios-persistence --features mongodb --lib backends::mongodb`
- [ ] `cargo test -p helios-persistence --features mongodb --test mongodb_tests` (Docker; `reindex_streams` not skipped)
- [ ] `cargo test -p helios-persistence --features postgres --test postgres_tests reindex`
- [ ] `cargo test -p helios-rest reindex` and `cargo test -p helios-rest --lib config`
- [ ] `cargo test -p helios-hfs test_automatic_reindex_hook`
- [ ] clippy: `-p helios-persistence --all-targets --all-features`; `-p helios-hfs --features mongodb`; `-p helios-hfs --no-default-features --features R4,mongodb` (repo `-A` list, `-D warnings`)

🤖 Generated with [Claude Code](https://claude.com/claude-code)

https://claude.ai/code/session_01Y61t4nTxKCT8WeE5NbxaVc
````

Do not `git add` it (the workspace is git-ignored), do not push, and do not open a PR. Report the workspace path, the branch head hash, and the Test plan boxes you ticked.

---

## Self-review

**Spec coverage.**
- Spec §4.5 / S5 §1.4 (plan: offload, one stream, budget with a once-per-backend WARN, count on `idx_resources_identity`, size limit, floor via the extracted helper, chained covered probes, plan line, `Ranges { ranges, catch_up }`): Task 6 (`plan_type_walk`, `reindex_stream_budget`, `reindex_streams_for_size`, `reindex_range_boundaries`, `reindex_id_range_cursors`, `log_streams_planned`) with unit tests for the budget table (S5 test 13), the size rule, the ranges, and the three early returns (S5 test 18 and two more), and integration tests 16, 17 and 19.
- S5 §1.5 (cursors `r`/`d`, parse validation, filter, dispatch inside `fetch_reindex_page`, a range ends on an empty query with `id range finished`, `IdPhaseDone` logs `id phase finished` then round 1 in the same call, prefetch of `IdRange` including its last page, `IdPhaseDone` never prefetched): Task 5, unit tests 12 and 14 plus the prefetch extensions, and three integration tests with hand-built cursors.
- S5 §1.6 (types, trait method, `RangeWalk`, `walk_range`, `record_and_log_page_shared`, per-type flow, page-bytes cap with one warning, stop reset, spawn, drain without abort, outcome precedence, catch-up only on `Ok`, locks, counters, L1/L3 fields, `set_type_plan`): Tasks 1–4; driver tests 1–8, 10 and 11 of S5 §1.10 plus `a_plan_without_ranges_fails_the_type`, `a_failed_plan_fails_the_job`, `ranges_prefetch_their_own_next_page_and_write_in_fetch_order`, and the three beyond-S5 tests (Drift 22).
- The production path to more than one stream (hook → coordinator → `GenerationScope::request` → `TypeWalkRequest`) runs end to end in `the_automatic_hook_plans_with_its_write_streams_and_concurrency` (Task 4), so a wrong `concurrent_runs` or a dropped `write_streams` fails a unit test instead of a K arm.
- S5 §1.7 (request fields and setters, `AutomaticRunOptions`, `ReindexOnFinish::with_write_streams`, `GenerationScope::request(options, concurrent_runs)` with `limits.max_concurrency`, rest config and test 21, hfs wiring): Tasks 1 and 8; S5 test 9 is `resource_scoped_generations_ignore_write_streams`.
- S5 §1.8 (files): all listed files are touched, except the sizing doc (drift 18) and with the integration tests in a new sibling file (drift 13).
- S5 §1.9 (attribution, failure, cancel, panic, memory, pool, tenancy): tests 8, 4/5, 6, 7 and 10; every count, probe and page filter carries `tenant_id` (Tasks 5–6), and test 15 checks another tenant's rows stay untouched.
- S5 §1.10 integration tests 15, 16, 17, 19 and 20: Tasks 6–7 (`mongodb_reindex_write_streams_match_single_stream`, `mongodb_plan_type_walk_ranges_cover_the_id_phase`, `mongodb_plan_type_walk_fits_the_pool_and_the_type_size`, `mongodb_plan_type_walk_boundary_probes_are_covered`, `mongodb_streams_catch_up_indexes_a_resource_created_during_the_ranges`), plus the log-order proof reusing `PhaseLogProbeTarget`.
- S5 §1.11 / S4 §4.10.9 (gate, pool 18, C_K, adoption, INCONCLUSIVE rule): no code; the bench contract table pins every line and field the analyser and V13 read, and the PR body names the arms. S4 §4.10.11's re-run rules belong to the orchestrator.
- Spec §8 risks (memory under K streams, pool pressure): the automatic 32 MiB cap (Task 4, test 10) and the connection budget (Task 6); §9 tests: as above.

**Placeholder scan.** No "TBD", "TODO", "similar to Task N" or undescribed steps; every code step carries the code. Two instructions describe an edit instead of repeating code, each precise: Task 3 Step 2(e)3 (four `&stats` → `&ctx.stats.lock()` substitutions in named statements) and Task 5 Step 6(a) (four `pub(super)` visibility changes, named). Task 8 Step 3(e) gives the exact sentences and names the bullet each goes into, in both SKILL copies. Every `cargo` command in a code block follows its own lock check (checked mechanically after the review fixes).

**Type consistency.**
- `TypeWalkPlan::Ranges { ranges: Vec<String>, catch_up: String }` and `TypeWalkRequest { streams: u32, min_resources_per_stream: u64, concurrent_runs: u32 }` (Task 1) are what `walk_type` builds and matches (Task 4), what `RangedSource` and `RangeCountingSource` return, and what MongoDB's override returns (Task 6).
- `ReindexWalkCursor::IdRange { range: ReindexIdRange, after_id: Option<String> }` and `IdPhaseDone { floor }` (Task 5) are what `reindex_id_range_cursors` encodes and `plan_type_walk` returns (Task 6); the tests build them with the same field names.
- `walk_range(ctx: &RangeWalk, resource_type: &str, start: Option<String>, page_bytes: u64, should_stop: &mut (dyn FnMut() -> bool + Send))` (Task 3) is called with that signature from `walk_type` and `walk_ranges` (Task 4); `RangeWalk` gains `stop`/`cap_warned` in Task 4, and its one constructor in `run_reindex` is updated in the same task.
- `TypeSummary.{streams: u32, plan: Duration}` (Task 2) are set by `set_type_plan(streams: u32, plan: Duration)` (Task 4) and logged as `streams`/`plan_ms` (Task 2).
- `log_job_started(.., setup: Duration, write_streams: u32)` (Task 2) is called once, with `request.write_streams` (Task 1's field).
- `build_backend_with_pool(config: MongoBackendConfig, max_pool: u32) -> Option<MongoBackend>` (Task 6, `mongodb_tests.rs`) is called by `create_backend_with_pool(test_name, pool, configure) -> Option<Arc<MongoBackend>>` (Task 6, `reindex_pipeline.rs`), which `create_backend_with` and `create_streams_backend` both call, so no backend builder is copied; `create_backend` still returns a bare `MongoBackend`, and every `seed_walk_fixture(&backend, ..)` call works for both through deref.
- `RangedSource.requests: parking_lot::Mutex<Vec<TypeWalkRequest>>` and `plan_gate: Option<Arc<Semaphore>>` (Task 4) are set in `RangedSource::new` and read only by Task 4's tests.
- `ReindexOnFinish::write_streams(&self) -> u32` (Task 1) is what Task 8's hfs test reads.
