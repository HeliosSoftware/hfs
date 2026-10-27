# MongoDB `$reindex` PR2b: Overlap and Parallelise Page Preparation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship PR2b — a sub-batch extraction/insert pipeline, a per-backend rayon pool, and a driver-side id-phase prefetch — so a MongoDB `$reindex` page overlaps HFS-side extraction with MongoDB-side delete/insert instead of running the four stages (fetch, extract, delete, insert) strictly in sequence.

**Architecture:** A new `crates/persistence/src/backends/mongodb/reindex_pipeline.rs` module holds a pure `SubBatchPlanner`, a `block_in_place` + rayon `extract_range` helper, and the overlapped/serial writer bodies; `storage.rs` keeps only the `write_search_entries_page_timed` dispatcher and the two new `ReindexSource` prefetch methods. The generic driver (`crates/persistence/src/search/reindex.rs`) grows a backend-agnostic prefetch mechanism (`may_prefetch_page` / `fetch_resources_page_ahead`, both defaulted to today's serial behaviour) and two new perf phases. Every new field on `ReindexPageStats`/`PageRecord`/`Counters`/`PhaseMillis` reproduces PR0's values exactly whenever nothing is prefetched and no writer measures `db_wait` (D11).

**Tech Stack:** Rust 2024 (MSRV 1.90, let-chains), `tokio` 1.x (`block_in_place`, `spawn`, multi-thread runtime detection copied from PostgreSQL's `crates/persistence/src/backends/postgres/storage.rs:4198-4205`), `rayon` (unconditional dependency, `crates/persistence/Cargo.toml:69`), `mongodb` driver 3.x. Tests: `cargo test -p helios-persistence --lib`, MongoDB integration tests need Docker + testcontainers (`cargo test -p helios-persistence --features mongodb --test mongodb_tests <filter>`; the suite starts its own container and skips silently — with an `eprintln!` — when neither Docker nor `HFS_TEST_MONGODB_URL` is available; a skipped test is not a pass, so quote the actual `test result: ok` line, not just exit code 0).

**Spec:** `docs/superpowers/specs/2026-09-23-mongodb-reindex-rebuild-design.md` §4.4, and in full `C:\Users\DougC\Code\Helios\manual-test\archive\1403-run17-evidence\design\S3-pr2-overlap.md` §5 (§0, §2, §3 for context PR2b depends on; §5.1–§5.14 is PR2b itself), cross-checked against `S2-pr1-id-walk.md` and `S1-pr0-instrumentation.md` for the PR0/PR1 contracts PR2b extends. SHA-256 of the three design files is recorded in the spec §11 and was re-verified byte-for-byte against the files on disk before this plan was written.

## Global Constraints

- **No cargo while a bench arm is running (S4 §4.5.1, §4.12.3 "Orchestrator rule: no cargo during arms").** Before every cargo/test/build command anywhere in this plan (every Step 2/4/5 of Tasks 1–6, 7a, 8 and 9, and Step 2/4/5/6 of Tasks 7b and 10), run:
  ```bash
  test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
  ```
  If it prints and exits, stop immediately and report back instead of running `cargo`, `rustc`, a test or a build — a `SMOKE-0`/`SMOKE-1` or later arm may be mid-measurement (this plan is implemented while PR0/PR1/PR2a's own bench arms, and PR2a's own, run: B0-s during PR1, B1-s/CU-1/B1-s-1G during PR2a, P2a-0/P2a-cap during this plan), and running any of these can invalidate that arm or the host-quiet precondition its validity checks assume.
- **Baseline.** This plan targets branch `perf/1403-pr2b-mongodb-reindex-overlap`, cut from `origin/main` **after** PR0 (S1), PR1 (S2) and PR2a (S3 §4) have merged. None of those three PRs exist in the repository yet (verified: `crates/persistence/src/search/reindex_stats.rs`, `ReindexPageStats`, `crates/persistence/src/backends/mongodb/reindex_pipeline.rs` and `crates/persistence/tests/mongodb/` all do not exist at the time this plan was written, HEAD `c86d0f08b`/`0dbc51c82`).
- **Anchors are hints, not truth.** Every `file:line` cited below is at HEAD `c86d0f08b`, the same baseline S1/S2/S3 cite, and was re-verified against the actual file content at that commit while writing this plan (via `tgrep -n -F -- "<exact string>" <path>` — see each task's Files section for the exact anchors checked). PR0/PR1/PR2a will insert code around these anchors before PR2b's branch is cut, shifting every line number that follows an insertion. **Locate every anchor by the function/struct/const name given, never by the line number**, and re-run the same `tgrep -n -F` (or `grep -n` if `.tgrep/` is stale — a stale index is a correctness risk, not just a slow path, so prefer `--no-index` over trusting a possibly-stale hit) against the actual branch before editing. Where a task depends on a PR0/PR1/PR2a type or function that does not exist yet in this repository (e.g. `ReindexPageStats`, `ReindexWalkCursor`, `reindex_find_page`, `MongoBackendConfig::reindex_catch_up_margin_ms`), its exact shape is fully specified in `S1-pr0-instrumentation.md` / `S2-pr1-id-walk.md` and is treated here as already landed, given context — re-read the cited section of that file if a step's assumption about it needs checking.
- **`perf.rs` is the one exception.** S1/S2/S3 make no changes to `crates/persistence/src/perf.rs` other than doc comments (S3 §2.9), so Task 1's anchors are exact today and will still be exact after PR0/PR1/PR2a land.
- Never `git add -A` and never `git commit -a`: a local build dirties about 3,500 generated R6 spec files. Every commit below lists its exact `git add <paths>`.
- Never `cargo fmt --all` in a worktree. Format only the files a task touched: `rustfmt --edition 2024 <explicit paths>`, redirected to `/dev/null` (never `nul`).
- Clippy gate (matches `.github/workflows/ci.yml:558`, verified against the file): `cargo clippy -p helios-persistence --all-targets --all-features -- -D warnings -A clippy::items_after_test_module -A clippy::large_enum_variant -A clippy::question_mark -A clippy::collapsible_match -A clippy::collapsible_if -A clippy::field_reassign_with_default -A clippy::doc-overindented-list-items -A clippy::doc-lazy-continuation`. For `crates/hfs` changes, the same flags with `-p helios-hfs --features mongodb` in place of `-p helios-persistence --all-features`.
- If `CARGO_BUILD_JOBS` is unset and the box is memory-constrained: `export CARGO_BUILD_JOBS=4`. If `target/` fills the disk mid-task, delete `target/debug/incremental` first, never `target/` wholesale (that forces every other crate to rebuild too).
- MongoDB integration tests need Docker running; the suite starts its own single-node replica-set container (`crates/persistence/tests/mongodb_tests.rs:1-24`, `#![cfg(feature = "mongodb")]`). Never point `HFS_TEST_MONGODB_URL` at the long-lived `hfs-mongo` corpus container on port 27017.
- **The overlapped path only runs on a multi-thread Tokio runtime.** Every integration test that exercises it uses `#[tokio::test(flavor = "multi_thread", worker_threads = 4)]`; a plain `#[tokio::test]` (current-thread) exercises only the serial fallback, which is intentional for some tests (proving the fallback) and a bug in others (silently not testing the overlapped path at all) — each task says which.
- **Invariant I1** (at most one page's delete/insert in flight per walk, committed in fetch order) must hold after every task; it is what makes PR1's revisits safe. No task may spawn a second insert before the first has been joined.
- Commit messages: one line, imperative, `(#1403)` suffix where the design cites the issue, then a blank line, then:
  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  ```
- **Knob defaults** (spec §4.4, S3 §5.9): `HFS_MONGODB_REINDEX_OVERLAP` default `true`; `HFS_MONGODB_REINDEX_PREPARE_THREADS` default `0` (`0` = cores − 1 clamped to 1–4; an explicit value is clamped to at most `REINDEX_PREPARE_MAX_THREADS = 64`); `HFS_MONGODB_REINDEX_PREFETCH` default `true`, off whenever search is offloaded.
- **Overlap threshold** (S3 §5.3–§5.4): a page takes the overlapped path only when `resources.len() > REINDEX_SUBBATCH_FIRST` (32) on a multi-thread runtime with the overlap knob on; pages of 32 or fewer, and every page on a current-thread runtime, always take the serial path.
- **Sub-batch sizing** (S3 §5.5): `REINDEX_SUBBATCH_FIRST = 32` (a type's first sub-batch), `REINDEX_SUBBATCH_TARGET_DOCS = 2_500` (documents per later sub-batch), `REINDEX_SUBBATCH_MAX = 512` (resources); the serial writer only tries the rayon pool for pages of at least `REINDEX_SERIAL_POOL_MIN_PAGE = 16` resources.
- **Perf phases** (spec §4.4): `Phase::ALL`/`PHASE_COUNT` grow from 36 to 38 (`ReindexDbWait` nested under `ReindexPage`, `ReindexFetchWait` top-level).
- **Mode line** (S3 §5.9): `mongodb reindex writer configuration`, INFO, target `helios_persistence::backends::mongodb::reindex_pipeline`, fields `overlap, prefetch, prepare_threads_configured, prepare_threads, pool, multi_thread_runtime, path`, logged once per backend instance.
- **Rounds are never prefetched** (S3 §5.3/§5.10, D13): only an `Id`-phase continuation cursor may ever be prefetched; a `Round` cursor, or one that fails to parse, always returns `Ok(None)` from `fetch_resources_page_ahead`.

---

## File Structure

| File | Change |
|---|---|
| `crates/persistence/src/perf.rs` | Modify: `ReindexDbWait`, `ReindexFetchWait` phases; `ALL`; `nested_in`; `label`; `PHASE_COUNT` 36→38. |
| `crates/persistence/src/search/reindex.rs` | Modify: `ReindexPageStats` gains `db_wait`/`sub_batches`/`pool_sub_batches`/`db_wait_or_busy`/changed `accumulate` (Task 2); `ReindexSource::may_prefetch_page`/`fetch_resources_page_ahead` defaults, `PrefetchedPage`, the paging-loop rewrite (Task 8); driver fake tests. |
| `crates/persistence/src/search/reindex_stats.rs` (exists after PR0) | Modify: `PageRecord.fetch_wait`, `Counters.fetch_wait`/`add_page`, `PhaseMillis.fetch_wait_ms`/`db_wait_ms` and the two changed formulas, field-order tests (Task 3). |
| `crates/persistence/src/backends/mongodb/reindex_pipeline.rs` (new) | Create: `#![allow(dead_code)]`, constants, `SubBatchPlanner`, `PrepareEnv`/`extract_range`, `resolve_prepare_width`, `tokio_multi_thread_runtime` (Task 4); `SubBatchDocs`/`flatten_sub_batch`/`delete_filters`/`DbTask`/`InsertFailures`/`absorb_db_task`/`add_db_wait`/`merge_insert_failures`/`page_outcomes`/`fan_out`/`join_failure`/`AbortOnDrop` (Task 6); `extract_one`/`delete_page`/`insert_sub_batch`/`write_page_serial`/`log_reindex_mode_once` (Task 7a); `write_page_overlapped`, removes the `#![allow(dead_code)]` (Task 7b). |
| `crates/persistence/src/backends/mongodb/mod.rs` | Modify: `mod reindex_pipeline;` (Task 4). |
| `crates/persistence/src/backends/mongodb/backend.rs` | Modify: 3 `MongoBackendConfig` fields + defaults + `impl Default` entries + `apply_reindex_env` + `from_env` call + doc list (Task 5); 4 private `MongoBackend` fields + `#[allow(dead_code)]` accessors, allow removed (Task 7b). |
| `crates/persistence/src/backends/mongodb/storage.rs` | Modify: `pub(super)` on `insert_search_entries_chunk`/`SEARCH_INDEX_INSERT_CHUNK`, `write_search_entries_page_timed`'s temporary serial-only dispatcher (Task 7a), rewritten to the final overlap-routing dispatcher (Task 7b); `may_prefetch_page`/`fetch_resources_page_ahead`/`reindex_id_page` (Task 9). |
| `crates/hfs/src/main.rs` | Modify: `build_mongodb_config_with_env` applies the 3 knobs, plus its knob-rejection test (Task 5). |
| `README.md` | Modify: 3 knob rows after the `HFS_MONGODB_INDEX_BUILD` row (Task 5). |
| `.claude/skills/bulk-data-submit/SKILL.md`, `.agents/skills/bulk-data-submit/SKILL.md` | Modify: D11 field definitions appended to the S1 §5.6 reindex-log bullet (Task 3). |
| `crates/persistence/tests/mongodb/reindex_pipeline.rs` (exists after PR2a) | Modify: `build_test_patients` and the serial-pipeline integration test (Task 7a); `FailPoint` visibility, `rows_without_id_and_tenant`, and the remaining 5 overlapped-writer integration tests (Task 7b); `PhaseLogProbeTarget` and the run-parity/consecutive-pages tests (Task 10). |
| `crates/persistence/tests/mongodb/reindex_id_walk.rs` (exists after PR1) | Modify: `capture_walk_logs`/`walk_log_lines` become `pub(super)` (Task 10). |
| `crates/persistence/tests/mongodb_tests.rs` | Modify: `FailPoint`/`enable`/`wait_until_entered`/`off` become `pub(super)` (Task 7b). |

---

### Task 1: Perf phases — `ReindexDbWait`, `ReindexFetchWait`, `PHASE_COUNT` 38

**Files:**
- Modify: `crates/persistence/src/perf.rs` — anchors (exact, unmodified by PR0/PR1/PR2a, re-verified at HEAD `c86d0f08b`): the `Phase` enum's last variant `SubmitHeartbeatRpc` (line 142), `pub const ALL: [Phase; 36]` (line 147, ends with `Phase::SubmitHeartbeatRpc,` before its closing `];`), `pub fn nested_in(self) -> Option<Phase>` (line 188, its catch-all `_ => None` arm), `pub fn label(self) -> &'static str` (its `Phase::SubmitHeartbeatRpc => "submit_heartbeat_rpc",` arm), `const PHASE_COUNT: usize = 36;` (line 263). Existing tests `phase_all_is_indexed_in_declaration_order` and `every_phase_has_a_label_and_a_terminating_nesting_chain` (both in `#[cfg(test)] mod tests`, near line 844) already iterate `Phase::ALL` generically.

**Interfaces:**
- Produces: two new `Phase` variants, `ReindexDbWait` and `ReindexFetchWait`; `Phase::ALL` grows to length 38; `PHASE_COUNT` becomes `38`.
- Consumes: nothing new; `crate::perf::span`, `crate::perf::record_duration` (existing, used by Tasks 7a/7b).

- [ ] **Step 1: Write the failing unit test**

Add to `crates/persistence/src/perf.rs`'s existing `#[cfg(test)] mod tests`, directly after `every_phase_has_a_label_and_a_terminating_nesting_chain`:

```rust
    #[test]
    fn reindex_db_wait_and_fetch_wait_are_wired_correctly() {
        assert_eq!(Phase::ALL.len(), 38, "PHASE_COUNT and ALL must both grow to 38 (#1403)");
        assert_eq!(PHASE_COUNT, 38);
        assert_eq!(Phase::ReindexDbWait.label(), "reindex_db_wait");
        assert_eq!(Phase::ReindexDbWait.nested_in(), Some(Phase::ReindexPage));
        assert_eq!(Phase::ReindexFetchWait.label(), "reindex_fetch_wait");
        assert_eq!(Phase::ReindexFetchWait.nested_in(), None);
        assert_eq!(
            Phase::ALL[36],
            Phase::ReindexDbWait,
            "appended after SubmitHeartbeatRpc, before ReindexFetchWait"
        );
        assert_eq!(Phase::ALL[37], Phase::ReindexFetchWait);
    }
```

- [ ] **Step 2: Run the test to verify it fails**

```
cargo test -p helios-persistence --lib perf::tests::reindex_db_wait_and_fetch_wait_are_wired_correctly 2>&1 | tail -30
```

Expected failure: a compile error — `Phase::ReindexDbWait` and `Phase::ReindexFetchWait` do not exist yet.

- [ ] **Step 3: Implement**

In the `Phase` enum, immediately after `SubmitHeartbeatRpc,` (the last variant):

```rust
    /// Time a MongoDB reindex page's thread waited on its delete/insert tasks (#1403).
    ReindexDbWait,
    /// Time the reindex driver waited for a prefetched page (#1403).
    ReindexFetchWait,
```

In `pub const ALL: [Phase; 36]`, change the length to `38` and append after `Phase::SubmitHeartbeatRpc,`:

```rust
        Phase::ReindexDbWait,
        Phase::ReindexFetchWait,
```

In `nested_in`, add `ReindexDbWait` to the existing `Some(Phase::ReindexPage)` arm (which already lists `ReindexConnection | ReindexExtract | ReindexSearchDelete | ReindexFtsDelete | ReindexSearchInsert | ReindexFts | ReindexCommit | ReindexFallback`):

```rust
            Phase::ReindexConnection
            | Phase::ReindexExtract
            | Phase::ReindexSearchDelete
            | Phase::ReindexFtsDelete
            | Phase::ReindexSearchInsert
            | Phase::ReindexFts
            | Phase::ReindexCommit
            | Phase::ReindexFallback
            | Phase::ReindexDbWait => Some(Phase::ReindexPage),
```

`ReindexFetchWait` needs no new arm: it falls through to the existing `_ => None`.

In `label`, add after the `Phase::SubmitHeartbeatRpc => "submit_heartbeat_rpc",` arm:

```rust
            Phase::ReindexDbWait => "reindex_db_wait",
            Phase::ReindexFetchWait => "reindex_fetch_wait",
```

Change `const PHASE_COUNT: usize = 36;` to `const PHASE_COUNT: usize = 38;`.

- [ ] **Step 4: Run the tests to verify they pass**

```
cargo test -p helios-persistence --lib perf:: 2>&1 | tail -40
```

Expect `test result: ok` covering `reindex_db_wait_and_fetch_wait_are_wired_correctly`, `phase_all_is_indexed_in_declaration_order` and `every_phase_has_a_label_and_a_terminating_nesting_chain`, all passing.

- [ ] **Step 5: fmt, clippy, commit**

```
rustfmt --edition 2024 crates/persistence/src/perf.rs > /dev/null 2>&1
cargo clippy -p helios-persistence --all-targets --all-features -- -D warnings -A clippy::items_after_test_module -A clippy::large_enum_variant -A clippy::question_mark -A clippy::collapsible_match -A clippy::collapsible_if -A clippy::field_reassign_with_default -A clippy::doc-overindented-list-items -A clippy::doc-lazy-continuation 2>&1 | tail -40
```

```
git add crates/persistence/src/perf.rs
git commit -m "$(cat <<'EOF'
feat(persistence): add ReindexDbWait and ReindexFetchWait perf phases (#1403)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: `ReindexPageStats` contract extension (D11)

**Files:**
- Modify: `crates/persistence/src/search/reindex.rs` — anchor: the `ReindexPageStats` struct and its `impl` block, added by PR0 directly after `pub struct SkippedResource { .. }` (HEAD `c86d0f08b` `:113-118`, re-verified: `SkippedResource`'s closing brace is the line immediately before where PR0 inserts `ReindexPageStats`). At HEAD, PR0's struct does not exist yet; its exact PR0 shape (reproduced below) is `S1-pr0-instrumentation.md` §5.1(a) (lines 422-461), already verified against the file.

**Interfaces:**
- Consumes: nothing new.
- Produces:
  - `impl ReindexPageStats { pub fn db_wait_or_busy(&self) -> Duration }`
  - `ReindexPageStats` gains three `pub` fields: `db_wait: Option<Duration>`, `sub_batches: u64`, `pool_sub_batches: u64`.
  - `accumulate`'s merge order changes (db_wait computed before delete/insert are added).

**What `ReindexPageStats` looks like going into this task** (PR0's shape, for reference — do not write this, it already exists on the branch):

```rust
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReindexPageStats {
    pub extract: Duration,
    pub delete: Duration,
    pub insert: Duration,
    pub deleted_entries: u64,
    pub inserted_entries: u64,
    pub insert_commands: u64,
}

impl ReindexPageStats {
    pub fn accumulate(&mut self, other: &ReindexPageStats) {
        self.extract += other.extract;
        self.delete += other.delete;
        self.insert += other.insert;
        self.deleted_entries += other.deleted_entries;
        self.inserted_entries += other.inserted_entries;
        self.insert_commands += other.insert_commands;
    }
}
```

- [ ] **Step 1: Write the failing unit tests**

Add a new `#[cfg(test)] mod reindex_page_stats_tests` at the end of `crates/persistence/src/search/reindex.rs` (after the existing `mod tests`), since these are pure-value tests independent of the driver fakes in `mod tests`:

```rust
#[cfg(test)]
mod reindex_page_stats_tests {
    use super::ReindexPageStats;
    use std::time::Duration;

    #[test]
    fn db_wait_or_busy_falls_back_to_delete_plus_insert() {
        let stats = ReindexPageStats {
            delete: Duration::from_millis(12),
            insert: Duration::from_millis(88),
            ..ReindexPageStats::default()
        };
        assert_eq!(stats.db_wait_or_busy(), Duration::from_millis(100));

        let measured = ReindexPageStats {
            delete: Duration::from_millis(12),
            insert: Duration::from_millis(88),
            db_wait: Some(Duration::from_millis(40)),
            ..ReindexPageStats::default()
        };
        assert_eq!(measured.db_wait_or_busy(), Duration::from_millis(40));
    }

    #[test]
    fn accumulate_merges_db_wait_as_effective_waits_before_adding_delete_and_insert() {
        // (None, None) stays None.
        let mut a = ReindexPageStats::default();
        let b = ReindexPageStats::default();
        a.accumulate(&b);
        assert_eq!(a.db_wait, None);

        // Some(x) merged with a writer that never measured db_wait (its delete
        // and insert are its effective wait) adds the two effective waits,
        // computed BEFORE delete/insert themselves are added into `a`.
        let mut a = ReindexPageStats {
            db_wait: Some(Duration::from_millis(30)),
            delete: Duration::from_millis(1),
            insert: Duration::from_millis(2),
            ..ReindexPageStats::default()
        };
        let b = ReindexPageStats {
            db_wait: None,
            delete: Duration::from_millis(5),
            insert: Duration::from_millis(7),
            ..ReindexPageStats::default()
        };
        a.accumulate(&b);
        assert_eq!(a.db_wait, Some(Duration::from_millis(30 + 5 + 7)));
        assert_eq!(a.delete, Duration::from_millis(1 + 5));
        assert_eq!(a.insert, Duration::from_millis(2 + 7));

        // The reverse pairing (self=None, other=Some) is not exercised by the
        // case above, and it is the one where getting the order wrong is
        // actually observable: self's effective wait must be read as its
        // OWN delete+insert (1+2=3ms) BEFORE those fields are mutated by the
        // `+=` lines below. Doing it the wrong way around (adding delete/
        // insert into `a` first, then computing `db_wait_or_busy` from the
        // already-mutated `self`) would give 25ms (6+9+10) instead of 13ms.
        let mut a = ReindexPageStats {
            db_wait: None,
            delete: Duration::from_millis(1),
            insert: Duration::from_millis(2),
            ..ReindexPageStats::default()
        };
        let b = ReindexPageStats {
            db_wait: Some(Duration::from_millis(10)),
            delete: Duration::from_millis(5),
            insert: Duration::from_millis(7),
            ..ReindexPageStats::default()
        };
        a.accumulate(&b);
        assert_eq!(a.db_wait, Some(Duration::from_millis(13)), "3ms (a's own delete+insert) + 10ms (b's measured db_wait)");
        assert_eq!(a.delete, Duration::from_millis(6));
        assert_eq!(a.insert, Duration::from_millis(9));
    }

    #[test]
    fn accumulate_still_adds_sub_batches_and_pool_sub_batches() {
        let mut a = ReindexPageStats {
            sub_batches: 3,
            pool_sub_batches: 2,
            ..ReindexPageStats::default()
        };
        let b = ReindexPageStats {
            sub_batches: 4,
            pool_sub_batches: 1,
            ..ReindexPageStats::default()
        };
        a.accumulate(&b);
        assert_eq!(a.sub_batches, 7);
        assert_eq!(a.pool_sub_batches, 3);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```
cargo test -p helios-persistence --lib search::reindex::reindex_page_stats_tests 2>&1 | tail -30
```

Expected failure: compile errors — `db_wait`, `sub_batches`, `pool_sub_batches` are not fields of `ReindexPageStats`, and `db_wait_or_busy` does not exist.

- [ ] **Step 3: Implement**

Add the three fields to the `ReindexPageStats` struct, after `insert_commands`:

```rust
    /// Time the page's own thread spent blocked on database work: the
    /// overlapped path's join waits, or the serial path's awaited delete plus
    /// awaited insert. `None` means the writer did not measure it separately
    /// (#1403, D11).
    pub db_wait: Option<Duration>,
    /// Extraction units the page ran; 1 on MongoDB's serial path (#1403).
    pub sub_batches: u64,
    /// Of `sub_batches`, how many ran on the rayon pool rather than inline (#1403).
    pub pool_sub_batches: u64,
```

Add the new method to `impl ReindexPageStats`:

```rust
    /// Time the page's thread waited on the database: the measured wait, or,
    /// for a writer that does not measure it, its busy delete + insert time
    /// (the serial case) (#1403, D11).
    pub fn db_wait_or_busy(&self) -> Duration {
        self.db_wait.unwrap_or(self.delete + self.insert)
    }
```

Replace the body of `accumulate` with (computing `db_wait` first, from both sides' *effective* wait, before `delete`/`insert` are touched):

```rust
    pub fn accumulate(&mut self, other: &ReindexPageStats) {
        let db_wait = match (self.db_wait, other.db_wait) {
            (None, None) => None,
            _ => Some(self.db_wait_or_busy() + other.db_wait_or_busy()),
        };
        self.extract += other.extract;
        self.delete += other.delete;
        self.insert += other.insert;
        self.deleted_entries += other.deleted_entries;
        self.inserted_entries += other.inserted_entries;
        self.insert_commands += other.insert_commands;
        self.sub_batches += other.sub_batches;
        self.pool_sub_batches += other.pool_sub_batches;
        self.db_wait = db_wait;
    }
```

(`db_wait_or_busy` is computed on `self` and `other` **before** `self.delete`/`self.insert` are mutated a few lines later — reading `self.db_wait_or_busy()` up front and binding it to a local, as above, is what keeps this correct; do not inline it after the `+=` lines.)

**Fix PR0's pre-existing exhaustive-literal test.** PR0's `reindex_page_stats_accumulate_adds_every_field` (already on the branch, in this same `mod tests`) builds `ReindexPageStats { extract, delete, insert, deleted_entries, inserted_entries, insert_commands }` with every field named and no `..Default::default()`. Adding this task's three new fields makes both of its struct literals fail to compile with "missing fields `db_wait`, `sub_batches`, `pool_sub_batches`" (`#[non_exhaustive]` only affects other *crates*; within this crate the literal must still be exhaustive or use `..`). Find that test (`grep -n "fn reindex_page_stats_accumulate_adds_every_field" crates/persistence/src/search/reindex.rs`) and append `..ReindexPageStats::default()` as the last line of **both** its struct literals:

```rust
    let mut total = ReindexPageStats {
        extract: Duration::from_millis(1),
        delete: Duration::from_millis(2),
        insert: Duration::from_millis(3),
        deleted_entries: 4,
        inserted_entries: 5,
        insert_commands: 6,
        ..ReindexPageStats::default()
    };
    let other = ReindexPageStats {
        extract: Duration::from_millis(10),
        delete: Duration::from_millis(20),
        insert: Duration::from_millis(30),
        deleted_entries: 40,
        inserted_entries: 50,
        insert_commands: 60,
        ..ReindexPageStats::default()
    };
```

Its assertions are unchanged and still pass: `db_wait` stays `None` on both sides (so `accumulate`'s merge is the `(None, None) => None` arm) and `sub_batches`/`pool_sub_batches` both default to `0`, so none of the six existing assertions (`extract` through `insert_commands`) are affected.

- [ ] **Step 4: Run the tests to verify they pass**

```
cargo test -p helios-persistence --lib search::reindex:: 2>&1 | tail -60
```

Expect `test result: ok` for `reindex_page_stats_tests::*` and every pre-existing test in `search::reindex` (this module also holds PR0/PR1's driver tests — none of their assertions *read* `db_wait`/`sub_batches`/`pool_sub_batches`, so once the struct-literal fix above lands, they all pass unchanged; without that fix, `reindex_page_stats_accumulate_adds_every_field` fails to compile, not merely to assert).

- [ ] **Step 5: fmt, clippy, commit**

```
rustfmt --edition 2024 crates/persistence/src/search/reindex.rs > /dev/null 2>&1
cargo clippy -p helios-persistence --all-targets --all-features -- -D warnings -A clippy::items_after_test_module -A clippy::large_enum_variant -A clippy::question_mark -A clippy::collapsible_match -A clippy::collapsible_if -A clippy::field_reassign_with_default -A clippy::doc-overindented-list-items -A clippy::doc-lazy-continuation 2>&1 | tail -40
```

```
git add crates/persistence/src/search/reindex.rs
git commit -m "$(cat <<'EOF'
feat(persistence): extend ReindexPageStats with db_wait, sub_batches, pool_sub_batches (#1403)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: `reindex_stats.rs` D11 contract — `fetch_wait`, critical-path formulas, log field appends, docs

**Files:**
- Modify: `crates/persistence/src/search/reindex_stats.rs` (created by PR0; its shape, reproduced below for reference, is `S1-pr0-instrumentation.md` §5.2, lines 724-820 — already verified against that file) — `PageRecord`, `Counters`, `Counters::add_page`, `PhaseMillis`, `PhaseMillis::of`.
- Modify: `crates/persistence/src/search/reindex.rs` — the four `log_*` helpers added by PR0 (`log_type_finished`, `log_progress`, `log_job_finished`, `log_page`, S1 §5.1(h)) and the two `PageRecord { .. }` construction sites inside `run_reindex` (S1 §5.1(g) steps 8 and 9, the named-resources branch and the paging branch); the `TYPE_FINISHED_FIELDS`, `PROGRESS_FIELDS`, `JOB_FINISHED_FIELDS`, `PAGE_FIELDS` field-order constants in `mod tests` (S1 §8.2).
- Modify: `.claude/skills/bulk-data-submit/SKILL.md` and `.agents/skills/bulk-data-submit/SKILL.md` — the reindex log-line bullet PR0 (S1 §5.6) adds (search the file for the string `reindex job started` to find it; it does not exist before PR0).

**What `reindex_stats.rs` looks like going into this task** (PR0's shape — do not write this, it already exists on the branch):

```rust
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct PageRecord {
    pub(super) resources: u64,
    pub(super) entries: u64,
    pub(super) failed: u64,
    pub(super) fetch: Duration,
    pub(super) write: Duration,
    pub(super) writer: ReindexPageStats,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Counters {
    pub(super) resources: u64,
    pub(super) entries: u64,
    pub(super) failed: u64,
    pub(super) pages: u64,
    pub(super) fetch: Duration,
    pub(super) write: Duration,
    pub(super) yielded: Duration,
    pub(super) writer: ReindexPageStats,
}
impl Counters {
    fn add_page(&mut self, page: &PageRecord) {
        self.resources += page.resources;
        self.entries += page.entries;
        self.failed += page.failed;
        self.pages += 1;
        self.fetch += page.fetch;
        self.write += page.write;
        self.writer.accumulate(&page.writer);
    }
    fn add_yield(&mut self, d: Duration) {
        self.yielded += d;
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct PhaseMillis {
    pub(super) fetch_ms: u64,
    pub(super) write_ms: u64,
    pub(super) extract_ms: u64,
    pub(super) delete_ms: u64,
    pub(super) insert_ms: u64,
    pub(super) writer_other_ms: u64,
    pub(super) yield_ms: u64,
    pub(super) other_ms: u64,
}
impl PhaseMillis {
    pub(super) fn of(c: &Counters, elapsed: Duration) -> Self {
        let writer_busy = c.writer.extract + c.writer.delete + c.writer.insert;
        let known = c.fetch + c.write + c.yielded;
        Self {
            fetch_ms: millis(c.fetch),
            write_ms: millis(c.write),
            extract_ms: millis(c.writer.extract),
            delete_ms: millis(c.writer.delete),
            insert_ms: millis(c.writer.insert),
            writer_other_ms: millis(c.write.saturating_sub(writer_busy)),
            yield_ms: millis(c.yielded),
            other_ms: millis(elapsed.saturating_sub(known)),
        }
    }
}
```

**Interfaces:**
- Consumes: `ReindexPageStats::db_wait_or_busy` (Task 2).
- Produces:
  - `PageRecord.fetch_wait: Duration`, `Counters.fetch_wait: Duration` (added by `add_page`), `PhaseMillis.fetch_wait_ms: u64`, `PhaseMillis.db_wait_ms: u64`.
  - `PhaseMillis::of`'s two changed formulas (D11): `writer_other_ms = write − (extract + writer.db_wait_or_busy())`; `other_ms = elapsed − (fetch_wait + write + yield)`.

- [ ] **Step 1: Write the failing unit tests**

Add to `crates/persistence/src/search/reindex_stats.rs`'s `#[cfg(test)] mod tests` (create it with `use super::*; use std::time::{Duration, Instant};` if this is the first test added since PR0 — PR0 already has tests there per S1 §8.1, so more likely this appends to the existing module):

```rust
    #[test]
    fn phase_millis_uses_the_critical_path() {
        let mut c = Counters::default();
        c.write = Duration::from_millis(100);
        c.fetch = Duration::from_millis(20);
        c.fetch_wait = Duration::from_millis(5); // most of the fetch was hidden by prefetch
        c.yielded = Duration::from_millis(5);
        c.writer.extract = Duration::from_millis(30);
        c.writer.delete = Duration::from_millis(10);
        c.writer.insert = Duration::from_millis(40);
        // db_wait is unset: db_wait_or_busy() falls back to delete + insert = 50 ms.
        let m = PhaseMillis::of(&c, Duration::from_millis(130));
        assert_eq!(m.fetch_ms, 20);
        assert_eq!(m.fetch_wait_ms, 5);
        assert_eq!(m.db_wait_ms, 50);
        assert_eq!(m.writer_other_ms, 20, "write - (extract 30 + db_wait_or_busy 50) = 20");
        assert_eq!(m.other_ms, 20, "elapsed 130 - (fetch_wait 5 + write 100 + yield 5) = 20");
    }

    #[test]
    fn unmeasured_db_wait_reproduces_pr0s_writer_other() {
        // A writer that never sets db_wait (every non-MongoDB writer today,
        // and a future PostgreSQL `_timed`) must give exactly PR0's
        // writer_other_ms: write - (extract + delete + insert).
        let mut c = Counters::default();
        c.write = Duration::from_millis(10);
        c.writer.extract = Duration::from_millis(3);
        c.writer.delete = Duration::from_millis(2);
        c.writer.insert = Duration::from_millis(4);
        let m = PhaseMillis::of(&c, Duration::from_millis(10));
        assert_eq!(m.writer_other_ms, 1);
    }

    #[test]
    fn unprefetched_pages_reproduce_pr0s_other() {
        // fetch_wait == fetch (nothing prefetched) must give exactly PR0's other_ms.
        let mut c = Counters::default();
        c.fetch = Duration::from_millis(7);
        c.fetch_wait = Duration::from_millis(7);
        c.write = Duration::from_millis(3);
        c.yielded = Duration::from_millis(1);
        let m = PhaseMillis::of(&c, Duration::from_millis(20));
        assert_eq!(m.other_ms, 9);
    }

    #[test]
    fn record_page_accumulates_fetch_wait_and_db_wait() {
        let t0 = Instant::now();
        let mut stats = ReindexRunStats::new(t0, 10, 1, Duration::from_secs(60));
        stats.mark_pages_started(t0);
        let _ = stats.start_type("Patient", 10, t0);
        let page = PageRecord {
            resources: 5,
            entries: 5,
            failed: 0,
            fetch: Duration::from_millis(50),
            fetch_wait: Duration::from_millis(10),
            write: Duration::from_millis(200),
            writer: ReindexPageStats {
                db_wait: Some(Duration::from_millis(150)),
                ..ReindexPageStats::default()
            },
        };
        let recorded = stats.record_page(page, t0 + Duration::from_millis(300));
        assert_eq!(recorded.page, 1);
        let summary = stats
            .finish_type(OUTCOME_COMPLETED, t0 + Duration::from_millis(300))
            .expect("a type is open");
        assert_eq!(summary.counters.fetch_wait, Duration::from_millis(10));
        assert_eq!(summary.counters.writer.db_wait, Some(Duration::from_millis(150)));
    }
```

Add to `crates/persistence/src/search/reindex.rs`'s existing `mod tests` (which already holds `TYPE_FINISHED_FIELDS` etc. per PR0, S1 §8.2):

```rust
    #[test]
    fn field_lists_carry_d11s_appended_fetch_wait_and_db_wait() {
        assert_eq!(
            &TYPE_FINISHED_FIELDS[TYPE_FINISHED_FIELDS.len() - 4..],
            ["fetch_wait_ms", "db_wait_ms", "sub_batches", "pool_sub_batches"]
        );
        assert_eq!(
            &PROGRESS_FIELDS[PROGRESS_FIELDS.len() - 2..],
            ["fetch_wait_ms", "db_wait_ms"]
        );
        assert!(
            !PROGRESS_FIELDS.contains(&"sub_batches"),
            "L4 must not carry sub_batches or pool_sub_batches (#1403, S3 §3)"
        );
        assert_eq!(
            &JOB_FINISHED_FIELDS[JOB_FINISHED_FIELDS.len() - 4..],
            ["fetch_wait_ms", "db_wait_ms", "sub_batches", "pool_sub_batches"]
        );
        assert_eq!(
            &PAGE_FIELDS[PAGE_FIELDS.len() - 4..],
            ["fetch_wait_ms", "db_wait_ms", "sub_batches", "pool_sub_batches"]
        );
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

```
cargo test -p helios-persistence --lib search::reindex_stats:: 2>&1 | tail -40
cargo test -p helios-persistence --lib search::reindex::tests::field_lists_carry 2>&1 | tail -30
```

Expected failure: `Counters`/`PageRecord` have no field `fetch_wait`, `PhaseMillis` has no `fetch_wait_ms`/`db_wait_ms` — compile errors; and the field-list assertion fails on length/content once it does compile.

- [ ] **Step 3: Implement**

In `reindex_stats.rs`, add `pub(super) fetch_wait: Duration,` to `PageRecord` (after `fetch`) and to `Counters` (after `fetch`). In `Counters::add_page`, add `self.fetch_wait += page.fetch_wait;` next to the existing `self.fetch += page.fetch;`. Add `pub(super) fetch_wait_ms: u64,` and `pub(super) db_wait_ms: u64,` to `PhaseMillis`. Replace the body of `PhaseMillis::of`:

```rust
    pub(super) fn of(c: &Counters, elapsed: Duration) -> Self {
        let db_wait_ms_dur = c.writer.db_wait_or_busy();
        let writer_busy = c.writer.extract + db_wait_ms_dur;
        let known = c.fetch_wait + c.write + c.yielded;
        Self {
            fetch_ms: millis(c.fetch),
            fetch_wait_ms: millis(c.fetch_wait),
            write_ms: millis(c.write),
            extract_ms: millis(c.writer.extract),
            delete_ms: millis(c.writer.delete),
            insert_ms: millis(c.writer.insert),
            db_wait_ms: millis(db_wait_ms_dur),
            writer_other_ms: millis(c.write.saturating_sub(writer_busy)),
            yield_ms: millis(c.yielded),
            other_ms: millis(elapsed.saturating_sub(known)),
        }
    }
```

In `reindex.rs`, add `fetch_wait: fetch_time,` next to `fetch: fetch_time,` in **both** `PageRecord { .. }` literals inside `run_reindex` (the named-resources branch, S1 §5.1(g) step 8, and the paging branch, step 9). At this point in the plan neither branch prefetches, so `fetch_wait` is always the same value as `fetch` — Task 8 changes the paging branch's value to a real, independently-tracked wait once the driver prefetch lands; the named branch's stays `fetch_wait: fetch_time` forever (S3 §5.10: "The named-resources path never prefetches. Its `PageRecord` has `fetch_wait = fetch`").

Update the four `log_*` helpers in `reindex.rs` (added by PR0, S1 §5.1(h)) to append the D11 fields in §3's order. Each keeps its existing fields and message unchanged; only the tail changes:

```rust
fn log_type_finished(tenant: &str, job_id: &str, s: &TypeSummary) {
    let m = PhaseMillis::of(&s.counters, s.type_elapsed);
    tracing::info!(
        tenant = %tenant,
        job_id = %job_id,
        resource_type = %s.resource_type,
        outcome = %s.outcome,
        type_index = s.type_index as u64,
        type_resources = s.counters.resources,
        type_total = s.type_total,
        type_elapsed_ms = millis(s.type_elapsed),
        type_resources_per_s = per_second(s.counters.resources, s.type_elapsed),
        elapsed_ms = millis(s.elapsed),
        entries = s.counters.entries,
        failed = s.counters.failed,
        pages = s.counters.pages,
        fetch_ms = m.fetch_ms,
        write_ms = m.write_ms,
        extract_ms = m.extract_ms,
        delete_ms = m.delete_ms,
        insert_ms = m.insert_ms,
        writer_other_ms = m.writer_other_ms,
        yield_ms = m.yield_ms,
        other_ms = m.other_ms,
        deleted = s.counters.writer.deleted_entries,
        inserted = s.counters.writer.inserted_entries,
        insert_commands = s.counters.writer.insert_commands,
        fetch_wait_ms = m.fetch_wait_ms,
        db_wait_ms = m.db_wait_ms,
        sub_batches = s.counters.writer.sub_batches,
        pool_sub_batches = s.counters.writer.pool_sub_batches,
        "reindex type finished"
    );
}

fn log_progress(tenant: &str, job_id: &str, s: &ProgressSnapshot) {
    let m = PhaseMillis::of(&s.type_counters, s.type_elapsed);
    tracing::info!(
        tenant = %tenant,
        job_id = %job_id,
        resource_type = %s.resource_type,
        type_index = s.type_index as u64,
        type_resources = s.type_counters.resources,
        type_total = s.type_total,
        type_elapsed_ms = millis(s.type_elapsed),
        type_resources_per_s = per_second(s.type_counters.resources, s.type_elapsed),
        processed = s.processed,
        total = s.total,
        elapsed_ms = millis(s.elapsed),
        interval_ms = millis(s.interval),
        interval_resources = s.interval_resources,
        interval_resources_per_s = per_second(s.interval_resources, s.interval),
        entries = s.type_counters.entries,
        failed = s.type_counters.failed,
        pages = s.type_counters.pages,
        fetch_ms = m.fetch_ms,
        write_ms = m.write_ms,
        extract_ms = m.extract_ms,
        delete_ms = m.delete_ms,
        insert_ms = m.insert_ms,
        writer_other_ms = m.writer_other_ms,
        yield_ms = m.yield_ms,
        other_ms = m.other_ms,
        deleted = s.type_counters.writer.deleted_entries,
        inserted = s.type_counters.writer.inserted_entries,
        insert_commands = s.type_counters.writer.insert_commands,
        fetch_wait_ms = m.fetch_wait_ms,
        db_wait_ms = m.db_wait_ms,
        "reindex progress"
    );
}

fn log_job_finished(tenant: &str, job_id: &str, s: &JobSummary) {
    let m = PhaseMillis::of(&s.counters, s.elapsed);
    tracing::info!(
        tenant = %tenant,
        job_id = %job_id,
        outcome = %s.outcome,
        types_done = s.types_done as u64,
        types = s.types as u64,
        processed = s.counters.resources,
        total = s.total,
        elapsed_ms = millis(s.elapsed),
        resources_per_s = per_second(s.counters.resources, s.elapsed),
        entries = s.counters.entries,
        failed = s.counters.failed,
        pages = s.counters.pages,
        fetch_ms = m.fetch_ms,
        write_ms = m.write_ms,
        extract_ms = m.extract_ms,
        delete_ms = m.delete_ms,
        insert_ms = m.insert_ms,
        writer_other_ms = m.writer_other_ms,
        yield_ms = m.yield_ms,
        other_ms = m.other_ms,
        deleted = s.counters.writer.deleted_entries,
        inserted = s.counters.writer.inserted_entries,
        insert_commands = s.counters.writer.insert_commands,
        fetch_wait_ms = m.fetch_wait_ms,
        db_wait_ms = m.db_wait_ms,
        sub_batches = s.counters.writer.sub_batches,
        pool_sub_batches = s.counters.writer.pool_sub_batches,
        "reindex job finished"
    );
}

fn log_page(tenant: &str, job_id: &str, resource_type: &str, recorded: &RecordedPage, r: &PageRecord) {
    tracing::debug!(
        tenant = %tenant,
        job_id = %job_id,
        resource_type = %resource_type,
        page = recorded.page,
        resources = r.resources,
        type_elapsed_ms = millis(recorded.type_elapsed),
        entries = r.entries,
        failed = r.failed,
        fetch_ms = millis(r.fetch),
        write_ms = millis(r.write),
        extract_ms = millis(r.writer.extract),
        delete_ms = millis(r.writer.delete),
        insert_ms = millis(r.writer.insert),
        deleted = r.writer.deleted_entries,
        inserted = r.writer.inserted_entries,
        insert_commands = r.writer.insert_commands,
        fetch_wait_ms = millis(r.fetch_wait),
        db_wait_ms = millis(r.writer.db_wait_or_busy()),
        sub_batches = r.writer.sub_batches,
        pool_sub_batches = r.writer.pool_sub_batches,
        "reindex page"
    );
}
```

**Replace each of the four helpers' doc comments** with the exact text below (PR0's field-meaning prose, `Field order:` list and formula sentences are all superseded — do not leave PR0's stale formula sentences or field lists in place beside these).

`log_type_finished`:
```rust
/// Logs L3 `reindex type finished` (INFO), exactly once per L2, on every
/// path on which `run_reindex` returns (completed, cancelled or failed).
/// Field order: `tenant, job_id, resource_type, outcome, type_index,
/// type_resources, type_total, type_elapsed_ms, type_resources_per_s,
/// elapsed_ms, entries, failed, pages, fetch_ms, write_ms, extract_ms,
/// delete_ms, insert_ms, writer_other_ms, yield_ms, other_ms, deleted,
/// inserted, insert_commands, fetch_wait_ms, db_wait_ms, sub_batches,
/// pool_sub_batches`. `outcome` is this type's own exit path — a type that
/// finished all its pages is always `completed`, even if a later type or the
/// job as a whole fails or is cancelled. Every counter and phase field
/// (`entries` through `pool_sub_batches`) is scoped to this type only, since
/// its L2. `type_elapsed_ms` is this type's own clock; `elapsed_ms` is always
/// the job clock. `writer_other_ms = write − (extract + db_wait_or_busy)` and
/// `other_ms = type_elapsed − (fetch_wait + write + yield)`, both saturating
/// on `Duration`s then truncated (#1403, D11). They equal PR0's values
/// whenever nothing was prefetched and no writer set `db_wait`.
/// `fetch_wait_ms` is the driver's wait for its page (equal to `fetch_ms`
/// unless the MongoDB source prefetched it); `db_wait_ms` is
/// `writer.db_wait_or_busy()`; `sub_batches`/`pool_sub_batches` are the
/// extraction units this type's pages ran, and how many of those ran on the
/// rayon pool. `deleted`/`inserted`/`insert_commands` come from the type's
/// accumulated `ReindexPageStats` (writer-reported; zero for a writer that
/// does not measure — MongoDB-only). Fields are appended only, never
/// renamed, removed or reordered — the one sanctioned exception is this
/// PR's redefinition of `other_ms`/`writer_other_ms` on the critical path
/// (#1403, S3 D11).
```

`log_progress`:
```rust
/// Logs L4 `reindex progress` (INFO), at the first page boundary at which at
/// least `progress_interval` has passed since the previous L4, or since the
/// page loop started. Field order: `tenant, job_id, resource_type,
/// type_index, type_resources, type_total, type_elapsed_ms,
/// type_resources_per_s, processed, total, elapsed_ms, interval_ms,
/// interval_resources, interval_resources_per_s, entries, failed, pages,
/// fetch_ms, write_ms, extract_ms, delete_ms, insert_ms, writer_other_ms,
/// yield_ms, other_ms, deleted, inserted, insert_commands, fetch_wait_ms,
/// db_wait_ms`. L4 does **not** carry `sub_batches`/`pool_sub_batches`, to
/// stay within the 32-field soft budget (S3 §3). **Scope is the open type,
/// not the whole job**: `type_resources`, `type_total`, `type_elapsed_ms`,
/// `type_resources_per_s`, and every counter and phase field from `entries`
/// through `db_wait_ms`, describe only the type open since its own `reindex
/// type started` line — Observation running after Patient must never
/// inherit Patient's counts. `processed`, `total` and `elapsed_ms` are
/// job-scoped. `interval_ms`/`interval_resources`/`interval_resources_per_s`
/// are job-level, measured since the previous L4 (an interval can span a
/// type boundary). When no type is open, `resource_type` is the sentinel `-`
/// (`NO_TYPE`) and every type-scoped field is zero — no logged value is ever
/// empty. `writer_other_ms = write − (extract + db_wait_or_busy)` and
/// `other_ms = type_elapsed − (fetch_wait + write + yield)`, both saturating
/// on `Duration`s then truncated (#1403, D11): they equal PR0's values
/// whenever nothing was prefetched and no writer set `db_wait`.
/// `fetch_wait_ms` is the driver's wait for its page (equal to `fetch_ms`
/// unless prefetched); `db_wait_ms` is `writer.db_wait_or_busy()`. Fields are
/// appended only, never renamed, removed or reordered (#1403).
```

`log_job_finished`:
```rust
/// Logs L5 `reindex job finished` (INFO), once per run that logged L1, on
/// every path on which `run_reindex` returns after L1, and always **before**
/// `run_reindex` writes the terminal status (the one exception: a synchronous
/// `cancel()` may write `Cancelled` first — see [`job_outcome`], D10). Field
/// order: `tenant, job_id, outcome, types_done, types, processed, total,
/// elapsed_ms, resources_per_s, entries, failed, pages, fetch_ms, write_ms,
/// extract_ms, delete_ms, insert_ms, writer_other_ms, yield_ms, other_ms,
/// deleted, inserted, insert_commands, fetch_wait_ms, db_wait_ms,
/// sub_batches, pool_sub_batches`. `outcome` is the job's already-written
/// terminal status if one exists, otherwise the exit path's outcome
/// (`job_outcome`). `types_done` counts types whose L3 said `completed`.
/// Every counter and phase field (`entries` through `pool_sub_batches`) is
/// job-scoped (summed over every type), unlike L3/L4's type scope.
/// `elapsed_ms` is the job clock. `writer_other_ms = write − (extract +
/// db_wait_or_busy)` and `other_ms = elapsed_ms − (fetch_wait + write +
/// yield)`, both saturating on `Duration`s then truncated (#1403, D11): they
/// equal PR0's values whenever nothing was prefetched and no writer set
/// `db_wait`. `fetch_wait_ms` is the driver's wait for its page (equal to
/// `fetch_ms` unless prefetched); `db_wait_ms` is `writer.db_wait_or_busy()`;
/// `sub_batches`/`pool_sub_batches` are the extraction units the job ran, and
/// how many of those ran on the rayon pool. Fields are appended only, never
/// renamed, removed or reordered (#1403).
```

`log_page`:
```rust
/// Logs L6 `reindex page` (DEBUG; needs
/// `RUST_LOG=…,helios_persistence::search::reindex=debug`), after every page
/// or id batch. Field order: `tenant, job_id, resource_type, page, resources,
/// type_elapsed_ms, entries, failed, fetch_ms, write_ms, extract_ms,
/// delete_ms, insert_ms, deleted, inserted, insert_commands, fetch_wait_ms,
/// db_wait_ms, sub_batches, pool_sub_batches`. Every counter and phase field
/// describes only this one page — `page` is the 1-based page number within
/// the open type, `resources` is this page's own count. L6 has no derived
/// `other_ms`/`writer_other_ms` fields: `fetch_ms`/`write_ms` are the
/// driver's own timings for this page, and `extract_ms`/`delete_ms`/
/// `insert_ms`/`deleted`/`inserted`/`insert_commands` come straight from
/// this page's `ReindexPageStats` (writer-reported; zero for a writer that
/// does not measure). `fetch_wait_ms` is the driver's wait for this page
/// (equal to `fetch_ms` unless it was prefetched); `db_wait_ms` is this
/// page's `writer.db_wait_or_busy()`; `sub_batches`/`pool_sub_batches` are
/// this page's own extraction units, and how many ran on the rayon pool
/// (#1403, D11). Fields are appended only, never renamed, removed or
/// reordered (#1403).
```

Update the four field-order constants in `reindex.rs`'s `mod tests` (S1 §8.2) by appending, in this order, to the existing `&[&str]` literal:
- `TYPE_FINISHED_FIELDS`: append `"fetch_wait_ms", "db_wait_ms", "sub_batches", "pool_sub_batches"`.
- `PROGRESS_FIELDS`: append `"fetch_wait_ms", "db_wait_ms"` only.
- `JOB_FINISHED_FIELDS`: append `"fetch_wait_ms", "db_wait_ms", "sub_batches", "pool_sub_batches"`.
- `PAGE_FIELDS`: append `"fetch_wait_ms", "db_wait_ms", "sub_batches", "pool_sub_batches"`.

In `.claude/skills/bulk-data-submit/SKILL.md`, find the bullet PR0 added describing the `reindex job started` … `reindex page` log-line contract (search for the string `reindex job started`) and append this sentence to its end:

> Since #1403 (PR2b), `fetch_wait_ms` is the driver's wait for its page (equal to `fetch_ms` unless the MongoDB source prefetched it) and `db_wait_ms` is the writer's measured database wait, or its busy delete+insert time when unmeasured; `other_ms` and `writer_other_ms` are computed from these two instead of `fetch_ms`/`delete_ms`+`insert_ms`, and are numerically identical whenever nothing was prefetched and no writer measures `db_wait`.

Make the identical edit in `.agents/skills/bulk-data-submit/SKILL.md`, at the same bullet, if that mirror already carries PR0's paragraph (per the spec's file-touch list, `.claude` SKILL edits are mirrored in `.agents`).

- [ ] **Step 4: Run the tests to verify they pass**

```
cargo test -p helios-persistence --lib search::reindex_stats:: 2>&1 | tail -40
cargo test -p helios-persistence --lib search::reindex:: 2>&1 | tail -80
```

Expect `test result: ok` throughout, including every PR0/PR1 test that reads `PageRecord`/`Counters`/`PhaseMillis` (their assertions target unappended fields and are unaffected by the append-only formula changes when nothing is prefetched, per D11).

- [ ] **Step 5: fmt, clippy, commit**

```
rustfmt --edition 2024 crates/persistence/src/search/reindex.rs crates/persistence/src/search/reindex_stats.rs > /dev/null 2>&1
cargo clippy -p helios-persistence --all-targets --all-features -- -D warnings -A clippy::items_after_test_module -A clippy::large_enum_variant -A clippy::question_mark -A clippy::collapsible_match -A clippy::collapsible_if -A clippy::field_reassign_with_default -A clippy::doc-overindented-list-items -A clippy::doc-lazy-continuation 2>&1 | tail -40
```

```
git add crates/persistence/src/search/reindex.rs crates/persistence/src/search/reindex_stats.rs .claude/skills/bulk-data-submit/SKILL.md .agents/skills/bulk-data-submit/SKILL.md
git commit -m "$(cat <<'EOF'
feat(persistence): fetch_wait/db_wait critical-path formulas in reindex log lines (#1403, D11)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: Sub-batch planner and parallel extraction mechanics

**Files:**
- Create: `crates/persistence/src/backends/mongodb/reindex_pipeline.rs`
- Modify: `crates/persistence/src/backends/mongodb/mod.rs` — anchor: the alphabetical `mod` list (HEAD `c86d0f08b` `:19-32`, re-verified: `pub(crate) mod backend;` … `mod user_settings;`). Add `mod reindex_pipeline;` after `mod composite_search;` and before `mod retry;` (alphabetical order).

**Interfaces:**
- Consumes: `rayon::ThreadPool` (dependency already unconditional, `crates/persistence/Cargo.toml:69`), `tokio::task::block_in_place`/`tokio::runtime::Handle` (already available, `rt-multi-thread` feature on, `Cargo.toml:57`).
- Produces (all `pub(super)`, visible to sibling `storage.rs` and `backend.rs` per S3 §2.9 — `mod reindex_pipeline;` is a private module declaration, and Rust privacy makes a private module's `pub(super)` items visible to every module under their shared parent, exactly as `bulk_ingest.rs:99` already reaches private sibling module `storage.rs`):
  - `pub(super) const REINDEX_SUBBATCH_FIRST: usize = 32;`
  - `pub(super) const REINDEX_SUBBATCH_TARGET_DOCS: usize = 2_500;`
  - `pub(super) const REINDEX_SUBBATCH_MAX: usize = 512;`
  - `pub(super) const REINDEX_SERIAL_POOL_MIN_PAGE: usize = 16;`
  - `const REINDEX_PREPARE_MAX_THREADS: usize = 64;`
  - `pub(super) struct SubBatchPlanner` with `pub(super) fn new(len: usize, seed: Option<(u64, u64)>, min_size: usize) -> Self`, `pub(super) fn next_range(&mut self) -> Option<std::ops::Range<usize>>`, `pub(super) fn record(&mut self, resources: usize, docs: usize)`.
  - `pub(super) struct PrepareEnv<'a> { pub pool: Option<&'a rayon::ThreadPool>, pub gate: &'a tokio::sync::Semaphore }`
  - `fn extract_range<T: Send, F: Fn(usize) -> T + Sync>(env: &PrepareEnv<'_>, range: std::ops::Range<usize>, prepare: &F) -> (Vec<T>, bool)` (module-private; only this file's own writer code calls it, per Tasks 7a/7b).
  - `pub(super) fn resolve_prepare_width(configured: usize) -> usize`
  - `pub(super) fn tokio_multi_thread_runtime() -> bool`

- [ ] **Step 1: Write the failing unit tests**

Create `crates/persistence/src/backends/mongodb/reindex_pipeline.rs` with only its module doc comment and test module for now:

```rust
//! Sub-batch pipeline for MongoDB `$reindex` page preparation (#1403, PR2b).
//!
//! While sub-batch *k*'s `insert_many` runs on a spawned task, the page's own
//! thread extracts sub-batch *k+1* inside `tokio::task::block_in_place`, so
//! HFS-side extraction overlaps MongoDB-side delete/insert instead of running
//! strictly after it. This module holds the planner that sizes sub-batches,
//! the `block_in_place` + rayon extraction helper, and (once Task 6/7 land)
//! the writer bodies themselves. `storage.rs` keeps only the dispatcher and
//! the two `ReindexSource` prefetch methods.
//!
//! See `docs/superpowers/specs/2026-09-23-mongodb-reindex-rebuild-design.md`
//! §4.4 and `S3-pr2-overlap.md` §5.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planner_first_range_is_first_without_a_seed() {
        let mut planner = SubBatchPlanner::new(1000, None, 1);
        assert_eq!(planner.next_range(), Some(0..32));
    }

    #[test]
    fn planner_sizes_from_observed_docs() {
        let mut planner = SubBatchPlanner::new(1000, None, 1);
        planner.record(32, 800);
        assert_eq!(planner.next_range(), Some(0..100)); // ceil(2500 * 32 / 800)
    }

    #[test]
    fn planner_uses_the_seed_for_its_first_range() {
        let mut planner = SubBatchPlanner::new(1000, Some((1000, 24_000)), 1);
        assert_eq!(planner.next_range(), Some(0..105)); // ceil(2500 * 1000 / 24000)
    }

    #[test]
    fn planner_clamps() {
        let mut a = SubBatchPlanner::new(1000, Some((1, 1630)), 1);
        assert_eq!(a.next_range(), Some(0..2), "1,630 docs/resource: ceil(2500/1630) = 2");

        let mut b = SubBatchPlanner::new(1000, Some((1, 1630)), 4);
        assert_eq!(b.next_range(), Some(0..4), "min_size 4 lifts the same ratio to 4");

        let mut c = SubBatchPlanner::new(1000, Some((1, 0)), 1);
        assert_eq!(c.next_range(), Some(0..REINDEX_SUBBATCH_MAX), "0 documents gives MAX");

        let mut d = SubBatchPlanner::new(1000, Some((1, 100_000)), 1);
        assert_eq!(d.next_range(), Some(0..1), "100,000 docs/resource: ceil(2500/100000) = 1");

        let mut e = SubBatchPlanner::new(1000, Some((0, 500)), 1);
        assert_eq!(e.next_range(), Some(0..REINDEX_SUBBATCH_FIRST), "a seed with r == 0 is ignored");
    }

    #[test]
    fn planner_covers_every_index_exactly_once_and_in_order() {
        for len in [0usize, 1, 31, 32, 33, 1000, 12345] {
            for seed in [None, Some((1, 1630)), Some((1000, 24_000)), Some((0, 500))] {
                for min_size in [1usize, 4, 16] {
                    let mut planner = SubBatchPlanner::new(len, seed, min_size);
                    let mut covered = 0usize;
                    while let Some(range) = planner.next_range() {
                        assert_eq!(
                            range.start, covered,
                            "len={len} seed={seed:?} min_size={min_size}: ranges must tile without gaps or overlap"
                        );
                        assert!(range.end > range.start, "a range must never be empty");
                        covered = range.end;
                        planner.record(range.len(), range.len() * 25);
                    }
                    assert_eq!(covered, len, "len={len} seed={seed:?} min_size={min_size}: must cover every index");
                }
            }
        }
    }

    #[test]
    fn resolve_prepare_width_auto_explicit_and_capped() {
        let expected_auto = std::thread::available_parallelism()
            .map(|n| n.get().saturating_sub(1).clamp(1, 4))
            .unwrap_or(1);
        assert_eq!(resolve_prepare_width(0), expected_auto);
        assert_eq!(resolve_prepare_width(1), 1);
        assert_eq!(resolve_prepare_width(100), 64);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn extract_range_preserves_input_order_on_the_pool_and_inline() {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(3).build().unwrap();
        let gate = tokio::sync::Semaphore::new(1);
        let prepare = |i: usize| i * 10;

        let pooled_env = PrepareEnv { pool: Some(&pool), gate: &gate };
        let (out, on_pool) = extract_range(&pooled_env, 0..20, &prepare);
        assert!(on_pool, "a range of 20 with a pool and a free gate must run on the pool");
        assert_eq!(out, (0..20).map(|i| i * 10).collect::<Vec<_>>());

        let inline_env = PrepareEnv { pool: None, gate: &gate };
        let (out, on_pool) = extract_range(&inline_env, 0..20, &prepare);
        assert!(!on_pool);
        assert_eq!(out, (0..20).map(|i| i * 10).collect::<Vec<_>>());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn extract_range_runs_inline_when_the_gate_is_held() {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(3).build().unwrap();
        let gate = tokio::sync::Semaphore::new(1);
        let _permit = gate.try_acquire().expect("a fresh semaphore has a free permit");
        let env = PrepareEnv { pool: Some(&pool), gate: &gate };
        let calling_thread = std::thread::current().id();
        let prepare = move |_i: usize| std::thread::current().id();

        let (out, on_pool) = extract_range(&env, 0..5, &prepare);
        assert!(!on_pool, "a held gate must fall back to the calling thread");
        assert!(out.iter().all(|id| *id == calling_thread));
    }
}
```

Add `mod reindex_pipeline;` to `crates/persistence/src/backends/mongodb/mod.rs`, after `mod composite_search;`.

- [ ] **Step 2: Run the tests to verify they fail**

```
cargo test -p helios-persistence --features mongodb --lib backends::mongodb::reindex_pipeline 2>&1 | tail -40
```

Expected failure: compile errors — `SubBatchPlanner`, `PrepareEnv`, `extract_range`, `resolve_prepare_width`, `REINDEX_SUBBATCH_MAX`, `REINDEX_SUBBATCH_FIRST` do not exist yet.

- [ ] **Step 3: Implement**

Append to `crates/persistence/src/backends/mongodb/reindex_pipeline.rs`, above the `#[cfg(test)]` module:

```rust
// #1403 PR2b: every item below this line is `pub(super)` or module-private,
// and several are not called by production code until Task 7b adds
// `write_page_overlapped` (which is the first caller of `SubBatchPlanner`,
// `REINDEX_SUBBATCH_FIRST`, `REINDEX_SUBBATCH_TARGET_DOCS` and
// `REINDEX_SUBBATCH_MAX`) — until then `cargo clippy --all-targets` compiles
// this file's non-test target too, where they are otherwise unused, and
// `-D warnings` turns "never used"/"never constructed" into a hard error.
// Task 7b removes this allow once every item is used by production code.
#![allow(dead_code)]

use std::ops::Range;

/// A type's first page: too little is known yet to size from observation (#1403).
pub(super) const REINDEX_SUBBATCH_FIRST: usize = 32;
/// Own + contained index documents a sub-batch targets (#1403).
pub(super) const REINDEX_SUBBATCH_TARGET_DOCS: usize = 2_500;
/// Largest resources a single sub-batch may hold (#1403).
pub(super) const REINDEX_SUBBATCH_MAX: usize = 512;
/// PostgreSQL's `REINDEX_PREPARE_MIN_PAGE` (`postgres/storage.rs:4123`):
/// minimum page size for the serial writer to still try the rayon pool (#1403).
pub(super) const REINDEX_SERIAL_POOL_MIN_PAGE: usize = 16;
/// Ceiling on an explicit `HFS_MONGODB_REINDEX_PREPARE_THREADS` value (#1403).
const REINDEX_PREPARE_MAX_THREADS: usize = 64;

/// Sizes successive extraction sub-batches of one `$reindex` page so a
/// pipeline of extraction and insert fills and drains in about one sub-batch
/// each (#1403). `len` indices are covered exactly once, in order, by
/// [`Self::next_range`]; [`Self::record`] feeds back what was actually
/// extracted so later ranges in the *same* page size from real data instead
/// of the type-level seed.
pub(super) struct SubBatchPlanner {
    len: usize,
    next: usize,
    min_size: usize,
    /// (resources, docs) of this type's previous overlapped page, or `None`
    /// on a type's first page.
    seed: Option<(u64, u64)>,
    resources_done: u64,
    docs_done: u64,
}

impl SubBatchPlanner {
    pub(super) fn new(len: usize, seed: Option<(u64, u64)>, min_size: usize) -> Self {
        Self { len, next: 0, min_size, seed, resources_done: 0, docs_done: 0 }
    }

    /// `REINDEX_SUBBATCH_TARGET_DOCS` documents' worth of resources, at the
    /// observed `docs / resources` ratio; `MAX` when nothing has been
    /// observed yet (`docs == 0`), never zero (`div_ceil` on a positive
    /// numerator is at least 1).
    fn size_for(resources: u64, docs: u64) -> usize {
        if docs == 0 {
            return REINDEX_SUBBATCH_MAX;
        }
        let size = (REINDEX_SUBBATCH_TARGET_DOCS as u64)
            .saturating_mul(resources)
            .div_ceil(docs);
        usize::try_from(size).unwrap_or(REINDEX_SUBBATCH_MAX).min(REINDEX_SUBBATCH_MAX)
    }

    /// The next sub-batch's range, or `None` once `len` is covered. Sizes
    /// from this page's own running observation once it has one, else from
    /// the type-level seed, else `REINDEX_SUBBATCH_FIRST`; always clamps to
    /// `[min_size.clamp(1, MAX), MAX]` so a pool never starves and a range is
    /// never empty (guaranteeing termination).
    pub(super) fn next_range(&mut self) -> Option<Range<usize>> {
        if self.next >= self.len {
            return None;
        }
        let raw = if self.resources_done > 0 {
            Self::size_for(self.resources_done, self.docs_done)
        } else if let Some((r, d)) = self.seed
            && r > 0
        {
            Self::size_for(r, d)
        } else {
            REINDEX_SUBBATCH_FIRST
        };
        let size = raw.clamp(self.min_size.clamp(1, REINDEX_SUBBATCH_MAX), REINDEX_SUBBATCH_MAX);
        let end = (self.next + size).min(self.len);
        let range = self.next..end;
        self.next = end;
        Some(range)
    }

    /// Feeds back what a just-extracted sub-batch actually held, so the next
    /// call to [`Self::next_range`] sizes from this page's own data instead
    /// of the seed.
    pub(super) fn record(&mut self, resources: usize, docs: usize) {
        self.resources_done += resources as u64;
        self.docs_done += docs as u64;
    }
}

/// The pool and admission gate a page's extraction may use (#1403).
pub(super) struct PrepareEnv<'a> {
    pub pool: Option<&'a rayon::ThreadPool>,
    pub gate: &'a tokio::sync::Semaphore,
}

/// Runs `prepare` over `range` in input order, returning `true` when it ran
/// on the pool (#1403). The caller guarantees a multi-thread Tokio runtime.
///
/// Every sub-batch runs inside `block_in_place`, even with `pool: None`: a
/// task spawned from this worker (the page's insert of the *previous*
/// sub-batch) sits in this worker's LIFO slot until the worker either
/// finishes its own synchronous work or calls `block_in_place`, which hands
/// the LIFO slot and run queue to another worker
/// (`tokio-1.52.1/src/runtime/scheduler/multi_thread/worker.rs:443-449`).
/// Without it, extraction would silently never overlap the spawned insert,
/// even at P=1.
fn extract_range<T: Send, F: Fn(usize) -> T + Sync>(
    env: &PrepareEnv<'_>,
    range: Range<usize>,
    prepare: &F,
) -> (Vec<T>, bool) {
    use rayon::prelude::*;
    tokio::task::block_in_place(|| {
        if let Some(pool) = env.pool
            && range.len() >= 2
            && let Ok(permit) = env.gate.try_acquire()
        {
            let out = pool.install(|| range.into_par_iter().map(prepare).collect::<Vec<T>>());
            drop(permit);
            (out, true)
        } else {
            (range.map(prepare).collect(), false)
        }
    })
}

/// `available_parallelism − 1`, clamped to 1–4, when `configured` is `0`
/// (PostgreSQL's rule, `postgres/storage.rs:4130-4134`); otherwise `configured`
/// clamped to at most `REINDEX_PREPARE_MAX_THREADS` (#1403).
pub(super) fn resolve_prepare_width(configured: usize) -> usize {
    if configured == 0 {
        std::thread::available_parallelism()
            .map(|n| n.get().saturating_sub(1).clamp(1, 4))
            .unwrap_or(1)
    } else {
        configured.min(REINDEX_PREPARE_MAX_THREADS)
    }
}

/// Whether `block_in_place` is legal on this thread: only inside a
/// multi-thread Tokio runtime. Copy of PostgreSQL's
/// `postgres/storage.rs:4198-4205` (#1403, D6: not shared, because MongoDB's
/// caller also needs `block_in_place` for sequential extraction and a
/// configurable width, which PostgreSQL's code does not).
pub(super) fn tokio_multi_thread_runtime() -> bool {
    tokio::runtime::Handle::try_current().is_ok_and(|handle| {
        matches!(handle.runtime_flavor(), tokio::runtime::RuntimeFlavor::MultiThread)
    })
}
```

- [ ] **Step 4: Run the tests to verify they pass**

```
cargo test -p helios-persistence --features mongodb --lib backends::mongodb::reindex_pipeline 2>&1 | tail -60
```

Expect `test result: ok` for all 8 tests.

- [ ] **Step 5: fmt, clippy, commit**

```
rustfmt --edition 2024 crates/persistence/src/backends/mongodb/reindex_pipeline.rs crates/persistence/src/backends/mongodb/mod.rs > /dev/null 2>&1
cargo clippy -p helios-persistence --all-targets --all-features -- -D warnings -A clippy::items_after_test_module -A clippy::large_enum_variant -A clippy::question_mark -A clippy::collapsible_match -A clippy::collapsible_if -A clippy::field_reassign_with_default -A clippy::doc-overindented-list-items -A clippy::doc-lazy-continuation 2>&1 | tail -40
```

```
git add crates/persistence/src/backends/mongodb/reindex_pipeline.rs crates/persistence/src/backends/mongodb/mod.rs
git commit -m "$(cat <<'EOF'
feat(mongodb): sub-batch planner and block_in_place/rayon extraction for $reindex (#1403)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: Reindex knobs, per-backend pool wiring, and docs

**Files:**
- Modify: `crates/persistence/src/backends/mongodb/backend.rs` — anchors (re-verified at HEAD `c86d0f08b`): `pub struct MongoBackendConfig { .. }` (`:115-185`, ends with the `index_build` field PR1 will have followed with its own `reindex_catch_up_margin_ms` field), the default functions (`:187-209`), `impl Default for MongoBackendConfig` (`:215-230`), `pub fn new(config: MongoBackendConfig) -> StorageResult<Self>` and its struct literal (`:287-319`), `pub fn from_env() -> StorageResult<Self>` and its struct literal (`:341-385`), `impl Debug for MongoBackend` (`:104-111`, marks where the new private fields on `MongoBackend` itself go — right after it), `#[cfg(test)] mod tests` ending at `config_index_build_defaults_to_background_and_reads_env` (`:1196-1213`).
- Modify: `crates/hfs/src/main.rs` — anchor: `fn build_mongodb_config_with_env<F>(..)` (`:299-359`, re-verified), whose final `Ok(MongoBackendConfig { .. })` (`:346-358`) is, per S2 §2.9, the **one** exhaustive `MongoBackendConfig` literal in the workspace; `mod tests` (existing tests `test_build_mongodb_config_*` around `:3897-3990`, re-verified — add the new test next to `test_build_mongodb_config_reads_index_build_and_rejects_invalid_values` at `:3953`).
- Modify: `README.md` — anchor: the `HFS_MONGODB_INDEX_BUILD` row (`:336`, re-verified, the only MongoDB knob row in the file's env-var table).

**Interfaces:**
- Consumes: `resolve_prepare_width` (Task 4).
- Produces:
  - `MongoBackendConfig` gains `pub reindex_overlap: bool` (default `true`), `pub reindex_prepare_threads: usize` (default `0`), `pub reindex_prefetch: bool` (default `true`).
  - `impl MongoBackendConfig { pub fn apply_reindex_env(&mut self, env: impl Fn(&str) -> Option<String>) -> Result<(), String> }`
  - `MongoBackend` gains private fields `prepare_pool`, `prepare_gate`, `reindex_mode_logged`, `reindex_docs_per_resource`, and `pub(super)` accessors `reindex_prepare_pool(&self) -> Option<&rayon::ThreadPool>`, `reindex_prepare_gate(&self) -> &tokio::sync::Semaphore`, `reindex_docs_seed(&self, resource_type: &str) -> Option<(u64, u64)>`, `record_reindex_docs(&self, resource_type: &str, resources: u64, docs: u64)`, `reindex_mode_logged(&self) -> &std::sync::atomic::AtomicBool`.

- [ ] **Step 1: Write the failing unit tests**

Add to `crates/persistence/src/backends/mongodb/backend.rs`'s `#[cfg(test)] mod tests`, immediately before its closing `}` (after `config_index_build_defaults_to_background_and_reads_env`):

```rust
    #[test]
    fn config_reindex_pipeline_defaults() {
        let default = MongoBackendConfig::default();
        assert!(default.reindex_overlap);
        assert_eq!(default.reindex_prepare_threads, 0);
        assert!(default.reindex_prefetch);

        let from_empty: MongoBackendConfig =
            serde_json::from_str("{}").expect("every field must have a serde default");
        assert!(from_empty.reindex_overlap);
        assert_eq!(from_empty.reindex_prepare_threads, 0);
        assert!(from_empty.reindex_prefetch);
    }

    #[test]
    fn apply_reindex_env_reads_and_rejects() {
        let mut config = MongoBackendConfig::default();
        config
            .apply_reindex_env(|name| match name {
                "HFS_MONGODB_REINDEX_OVERLAP" => Some(" On ".to_string()),
                "HFS_MONGODB_REINDEX_PREPARE_THREADS" => Some("8".to_string()),
                _ => None,
            })
            .expect("valid values");
        assert!(config.reindex_overlap, "\" On \" trims and parses case-insensitively");
        assert_eq!(config.reindex_prepare_threads, 8);
        assert!(config.reindex_prefetch, "an unset variable leaves the default");

        let mut config = MongoBackendConfig { reindex_prepare_threads: 3, ..Default::default() };
        config
            .apply_reindex_env(|name| match name {
                "HFS_MONGODB_REINDEX_PREPARE_THREADS" => Some(String::new()),
                _ => None,
            })
            .expect("an empty value leaves the field unchanged");
        assert_eq!(config.reindex_prepare_threads, 3);

        let mut config = MongoBackendConfig::default();
        let err = config
            .apply_reindex_env(|name| match name {
                "HFS_MONGODB_REINDEX_PREPARE_THREADS" => Some("x".to_string()),
                _ => None,
            })
            .expect_err("a non-integer must be rejected");
        assert!(err.contains("HFS_MONGODB_REINDEX_PREPARE_THREADS"));

        let mut config = MongoBackendConfig::default();
        let err = config
            .apply_reindex_env(|name| match name {
                "HFS_MONGODB_REINDEX_OVERLAP" => Some("sideways".to_string()),
                _ => None,
            })
            .expect_err("a non-boolean must be rejected");
        assert!(err.contains("HFS_MONGODB_REINDEX_OVERLAP"));
    }
```

Add to `crates/hfs/src/main.rs`'s `mod tests`, immediately after `test_build_mongodb_config_reads_index_build_and_rejects_invalid_values`:

```rust
    #[cfg(feature = "mongodb")]
    #[test]
    fn test_build_mongodb_config_reads_reindex_pipeline_knobs_and_rejects_invalid_values() {
        let config = ServerConfig::default();

        let mongo_config = build_mongodb_config_with_env(&config, false, |name| match name {
            "HFS_MONGODB_REINDEX_OVERLAP" => Some("false".to_string()),
            "HFS_MONGODB_REINDEX_PREPARE_THREADS" => Some("2".to_string()),
            "HFS_MONGODB_REINDEX_PREFETCH" => Some("off".to_string()),
            _ => None,
        })
        .expect("valid config");
        assert!(!mongo_config.reindex_overlap);
        assert_eq!(mongo_config.reindex_prepare_threads, 2);
        assert!(!mongo_config.reindex_prefetch);

        let default_config =
            build_mongodb_config_with_env(&config, false, |_| None).expect("valid config");
        assert!(default_config.reindex_overlap);
        assert_eq!(default_config.reindex_prepare_threads, 0);
        assert!(default_config.reindex_prefetch);

        let err = build_mongodb_config_with_env(&config, false, |name| match name {
            "HFS_MONGODB_REINDEX_OVERLAP" => Some("sideways".to_string()),
            _ => None,
        })
        .expect_err("invalid value must fail startup");
        assert!(format!("{err}").contains("HFS_MONGODB_REINDEX_OVERLAP"));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

```
cargo test -p helios-persistence --features mongodb --lib backends::mongodb::backend 2>&1 | tail -40
cargo test -p helios-hfs --features mongodb build_mongodb_config_reads_reindex_pipeline_knobs 2>&1 | tail -30
```

Expected failure: compile errors — `reindex_overlap`/`reindex_prepare_threads`/`reindex_prefetch` are not fields of `MongoBackendConfig`, `apply_reindex_env` does not exist.

- [ ] **Step 3: Implement**

In `backend.rs`, add three fields to `MongoBackendConfig`, after whatever PR1 added following `index_build` (its own doc-comment marker: PR1 adds `reindex_catch_up_margin_ms` right after `index_build`; these three go after *that* field):

```rust
    /// #1403: overlap search-parameter extraction with inserts inside a
    /// reindex page (`HFS_MONGODB_REINDEX_OVERLAP`); `false` is the serial writer.
    #[serde(default = "default_reindex_overlap")]
    pub reindex_overlap: bool,
    /// #1403: extraction threads for reindex pages
    /// (`HFS_MONGODB_REINDEX_PREPARE_THREADS`); `0` = cores − 1 clamped to
    /// 1..=4, `1` = the page's own thread, `N` = `min(N, 64)`.
    #[serde(default)]
    pub reindex_prepare_threads: usize,
    /// #1403: let the reindex driver fetch the next id-phase page while it
    /// writes the current one (`HFS_MONGODB_REINDEX_PREFETCH`). Never used
    /// when search is offloaded.
    #[serde(default = "default_reindex_prefetch")]
    pub reindex_prefetch: bool,
```

Add the two default functions next to the others:

```rust
fn default_reindex_overlap() -> bool {
    true
}

fn default_reindex_prefetch() -> bool {
    true
}
```

Add the three fields to `impl Default for MongoBackendConfig`: `reindex_overlap: default_reindex_overlap(), reindex_prepare_threads: 0, reindex_prefetch: default_reindex_prefetch(),`.

Add `apply_reindex_env` to `impl MongoBackendConfig` (create the block if PR1 did not already add one for its own field's parsing — `MongoBackendConfig` has no env-parsing methods today, only `MongoBackend::from_env` reads `std::env` directly, so this is likely a new `impl MongoBackendConfig { .. }` block placed directly after the struct definition):

```rust
impl MongoBackendConfig {
    /// Applies `HFS_MONGODB_REINDEX_{OVERLAP,PREPARE_THREADS,PREFETCH}` from
    /// `env` (#1403). Each value is trimmed; an unset variable, or one empty
    /// after trimming, leaves its field unchanged. Booleans accept
    /// `true|1|yes|on` / `false|0|no|off`, case-insensitively; anything else
    /// is an `Err` naming the variable. The thread count parses as `usize`;
    /// values above 64 are accepted here and clamped later by
    /// `resolve_prepare_width`.
    pub fn apply_reindex_env(&mut self, env: impl Fn(&str) -> Option<String>) -> Result<(), String> {
        fn parse_bool(name: &str, raw: &str) -> Result<bool, String> {
            match raw.to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" | "on" => Ok(true),
                "false" | "0" | "no" | "off" => Ok(false),
                _ => Err(format!("{name} must be true or false; got {raw:?}")),
            }
        }
        if let Some(raw) = env("HFS_MONGODB_REINDEX_OVERLAP") {
            let raw = raw.trim();
            if !raw.is_empty() {
                self.reindex_overlap = parse_bool("HFS_MONGODB_REINDEX_OVERLAP", raw)?;
            }
        }
        if let Some(raw) = env("HFS_MONGODB_REINDEX_PREPARE_THREADS") {
            let raw = raw.trim();
            if !raw.is_empty() {
                self.reindex_prepare_threads = raw.parse::<usize>().map_err(|_| {
                    format!(
                        "HFS_MONGODB_REINDEX_PREPARE_THREADS must be a non-negative integer; got {raw:?}"
                    )
                })?;
            }
        }
        if let Some(raw) = env("HFS_MONGODB_REINDEX_PREFETCH") {
            let raw = raw.trim();
            if !raw.is_empty() {
                self.reindex_prefetch = parse_bool("HFS_MONGODB_REINDEX_PREFETCH", raw)?;
            }
        }
        Ok(())
    }
}
```

In `from_env`, change `let config = MongoBackendConfig { .. };` to `let mut config = MongoBackendConfig { .. };` (unchanged fields) and, immediately before `Self::new(config)`:

```rust
        config.apply_reindex_env(|n| std::env::var(n).ok()).map_err(|message| {
            StorageError::Backend(BackendError::Internal {
                backend_name: "mongodb".to_string(),
                message,
                source: None,
            })
        })?;

        Self::new(config)
```

Add the four private fields to `struct MongoBackend`, after `search_index_tx`:

```rust
    /// Rayon pool for `$reindex` sub-batch extraction; built lazily and only
    /// on a multi-thread runtime (#1403). `Err` remembers a failed build so
    /// the one warning fires once.
    prepare_pool: std::sync::OnceLock<Result<rayon::ThreadPool, String>>,
    /// The one-permit admission gate for `prepare_pool` (#1403).
    prepare_gate: tokio::sync::Semaphore,
    /// Whether `mongodb reindex writer configuration` has already been logged
    /// for this instance (#1403).
    reindex_mode_logged: std::sync::atomic::AtomicBool,
    /// `(resources, docs)` of each resource type's previous successful
    /// overlapped page, for the sub-batch planner's seed (#1403).
    reindex_docs_per_resource: std::sync::Mutex<std::collections::HashMap<String, (u64, u64)>>,
```

In `pub fn new`, add the four fields to the `Ok(Self { .. })` literal:

```rust
            prepare_pool: std::sync::OnceLock::new(),
            prepare_gate: tokio::sync::Semaphore::new(1),
            reindex_mode_logged: std::sync::atomic::AtomicBool::new(false),
            reindex_docs_per_resource: std::sync::Mutex::new(std::collections::HashMap::new()),
```

In `from_env`'s doc comment, add three lines to the "Supported variables" list, after the `HFS_MONGODB_INDEX_BUILD` bullet:

```rust
    /// - HFS_MONGODB_REINDEX_OVERLAP (default: true)
    /// - HFS_MONGODB_REINDEX_PREPARE_THREADS (default: 0 = cores − 1, 1–4)
    /// - HFS_MONGODB_REINDEX_PREFETCH (default: true)
```

Add a new `impl MongoBackend { .. }` block directly after `impl Debug for MongoBackend`. Mark it `#[allow(dead_code)]`: `reindex_docs_seed` and `record_reindex_docs` are called only by `write_page_overlapped`, which Task 7b adds — until then this block's non-test compilation (`cargo clippy --all-targets`) sees two of its five methods as never called, which `-D warnings` turns into an error. Task 7b removes this allow once `write_page_overlapped` calls both.

```rust
// #1403 PR2b: removed in Task 7b, once write_page_overlapped calls
// reindex_docs_seed and record_reindex_docs.
#[allow(dead_code)]
impl MongoBackend {
    /// The rayon pool for `$reindex` sub-batch extraction, or `None` when the
    /// resolved width is below 2 or the pool failed to build (#1403). Built
    /// lazily, once per backend instance. Callers must only call this on a
    /// multi-thread Tokio runtime.
    pub(super) fn reindex_prepare_pool(&self) -> Option<&rayon::ThreadPool> {
        if super::reindex_pipeline::resolve_prepare_width(self.config.reindex_prepare_threads) < 2 {
            return None;
        }
        self.prepare_pool
            .get_or_init(|| {
                let width = super::reindex_pipeline::resolve_prepare_width(
                    self.config.reindex_prepare_threads,
                );
                rayon::ThreadPoolBuilder::new()
                    .num_threads(width)
                    .thread_name(|i| format!("hfs-mongo-reindex-{i}"))
                    .build()
                    .map_err(|e| {
                        let message = format!(
                            "Failed to build the MongoDB reindex prepare pool; reindex sub-batches will be extracted on the calling thread: {e}"
                        );
                        tracing::warn!("{message}");
                        message
                    })
            })
            .as_ref()
            .ok()
    }

    /// The one-permit admission gate for [`Self::reindex_prepare_pool`] (#1403).
    pub(super) fn reindex_prepare_gate(&self) -> &tokio::sync::Semaphore {
        &self.prepare_gate
    }

    /// `(resources, docs)` of this type's previous successful overlapped
    /// page, if any (#1403).
    pub(super) fn reindex_docs_seed(&self, resource_type: &str) -> Option<(u64, u64)> {
        self.reindex_docs_per_resource
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(resource_type)
            .copied()
    }

    /// Records `(resources, docs)` for a successful single-type overlapped
    /// page (#1403).
    pub(super) fn record_reindex_docs(&self, resource_type: &str, resources: u64, docs: u64) {
        self.reindex_docs_per_resource
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(resource_type.to_string(), (resources, docs));
    }

    /// Whether `mongodb reindex writer configuration` has already been logged
    /// for this backend instance (#1403).
    pub(super) fn reindex_mode_logged(&self) -> &std::sync::atomic::AtomicBool {
        &self.reindex_mode_logged
    }
}
```

In `crates/hfs/src/main.rs`, replace `build_mongodb_config_with_env`'s final `Ok(MongoBackendConfig { .. })` with:

```rust
    let mut config = MongoBackendConfig {
        connection_string,
        database_name,
        max_connections,
        connect_timeout_ms,
        server_selection_timeout_ms,
        fhir_version: config.default_fhir_version,
        data_dir: config.data_dir.clone(),
        search_offloaded,
        max_included_resources,
        index_build,
        reindex_catch_up_margin_ms: MongoBackendConfig::default().reindex_catch_up_margin_ms,
        app_name: MongoBackendConfig::default().app_name,
        ..Default::default()
    };
    config
        .apply_reindex_env(&env)
        .map_err(|message| anyhow::anyhow!(message))?;
    Ok(config)
```

(`..Default::default()` now supplies `reindex_overlap`/`reindex_prepare_threads`/`reindex_prefetch` before `apply_reindex_env` overlays any that were set; this ends the literal's "every field named" property, which is exactly what S3 §5.9 calls for, since a fourth new field would otherwise force this exhaustive site to be edited again on every future addition.)

In `README.md`, insert the three rows below immediately after the `HFS_MONGODB_INDEX_BUILD` row, before the blank line and before any "For sizing MongoDB's `--wiredTigerCacheSizeGB`…" paragraph the `docs` PR (2026-09-23-1403-pr-docs) may have already added there — that paragraph is prose, not a table row, and rows placed after it would start a new header-less table that GFM renders as plain text:

```
| `HFS_MONGODB_REINDEX_OVERLAP` | `true` | Overlap search-parameter extraction with `search_index` inserts inside a `$reindex` page; `false` restores the serial writer. |
| `HFS_MONGODB_REINDEX_PREPARE_THREADS` | `0` | Extraction threads for `$reindex` pages: `0` = cores − 1 (1–4), `1` = none beyond the page's own thread. |
| `HFS_MONGODB_REINDEX_PREFETCH` | `true` | Fetch the next id-order `$reindex` page while the current one is written; never used with Elasticsearch search. |
```

Check the placement before moving on:
```
grep -n -A4 'HFS_MONGODB_INDEX_BUILD' README.md
```
Expected: the three new rows appear directly under the `HFS_MONGODB_INDEX_BUILD` row, with no blank line or prose paragraph between them.

- [ ] **Step 4: Run the tests to verify they pass**

```
cargo test -p helios-persistence --features mongodb --lib backends::mongodb::backend 2>&1 | tail -60
cargo test -p helios-hfs --features mongodb build_mongodb_config 2>&1 | tail -60
```

Expect `test result: ok` for every test in both modules, including the pre-existing `test_build_mongodb_config_*` tests (their assertions do not read the three new fields, and the switch to `..Default::default()` does not change any field they do assert on).

- [ ] **Step 5: fmt, clippy, commit**

```
rustfmt --edition 2024 crates/persistence/src/backends/mongodb/backend.rs crates/hfs/src/main.rs > /dev/null 2>&1
cargo clippy -p helios-persistence --all-targets --all-features -- -D warnings -A clippy::items_after_test_module -A clippy::large_enum_variant -A clippy::question_mark -A clippy::collapsible_match -A clippy::collapsible_if -A clippy::field_reassign_with_default -A clippy::doc-overindented-list-items -A clippy::doc-lazy-continuation 2>&1 | tail -40
cargo clippy -p helios-hfs --all-targets --features mongodb -- -D warnings -A clippy::items_after_test_module -A clippy::large_enum_variant -A clippy::question_mark -A clippy::collapsible_match -A clippy::collapsible_if -A clippy::field_reassign_with_default -A clippy::doc-overindented-list-items -A clippy::doc-lazy-continuation 2>&1 | tail -40
```

```
git add crates/persistence/src/backends/mongodb/backend.rs crates/hfs/src/main.rs README.md
git commit -m "$(cat <<'EOF'
feat(mongodb): HFS_MONGODB_REINDEX_{OVERLAP,PREPARE_THREADS,PREFETCH} knobs (#1403)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 6: Pure sub-batch writer helpers (no database)

**Files:**
- Modify: `crates/persistence/src/backends/mongodb/reindex_pipeline.rs` (Task 4's file) — insert new items directly above the existing `#[cfg(test)] mod tests` block; add new tests inside that same block, after the existing ones.

**Interfaces:**
- Consumes: `SearchIndexDocuments` (`pub(super) struct`, `storage.rs:1778-1783`, already `pub` fields, re-verified), `internal_error` (`pub(super) fn`, `storage.rs:37`, re-verified), `ReindexPageStats` (Task 2, `crate::search::reindex::ReindexPageStats`), `StoredResource` (`crate::types::StoredResource`, existing).
- Produces (all module-private unless noted):
  - `struct SubBatchDocs { own_owners: Vec<usize>, own_docs: Vec<Document>, contained_owners: Vec<usize>, contained_docs: Vec<Document> }` with `fn len(&self) -> usize`
  - `fn flatten_sub_batch(start: usize, prepared: Vec<(SearchIndexDocuments, Option<String>)>, extract_failures: &mut [Option<String>], doc_counts: &mut [usize]) -> SubBatchDocs`
  - `fn delete_filters(tenant_id: &str, resources: &[StoredResource]) -> Vec<Document>`
  - `struct DbTask<T> { stats: ReindexPageStats, result: Result<T, String> }`
  - `struct InsertFailures { own: HashMap<usize, String>, contained: HashMap<usize, String> }`
  - `fn absorb_db_task(stats: &mut ReindexPageStats, t: &ReindexPageStats)`
  - `fn add_db_wait(stats: &mut ReindexPageStats, page_wait: &mut Duration, d: Duration)`
  - `fn merge_insert_failures(into: &mut HashMap<usize, String>, f: InsertFailures)`
  - `fn page_outcomes(extract_failures: Vec<Option<String>>, insert_failures: HashMap<usize, String>, doc_counts: &[usize]) -> Vec<StorageResult<usize>>`
  - `fn fan_out(n: usize, msg: &str) -> Vec<StorageResult<usize>>`
  - `fn join_failure(e: tokio::task::JoinError, what: &str) -> String`
  - `pub(super) struct AbortOnDrop<T>(tokio::task::JoinHandle<T>)` with `fn spawn`, `async fn join`, `impl Drop`

- [ ] **Step 1: Write the failing unit tests**

Add to `reindex_pipeline.rs`'s `#[cfg(test)] mod tests`:

```rust
    use mongodb::bson::doc;
    use std::time::Duration;

    #[test]
    fn flatten_moves_documents_with_page_global_owners() {
        let own_a = doc! {"a": 1};
        let contained_a = doc! {"ac": 1};
        let own_b = doc! {"b": 1};
        let prepared = vec![
            (
                super::super::storage::SearchIndexDocuments {
                    own: vec![own_a.clone()],
                    contained: vec![contained_a.clone()],
                },
                None,
            ),
            (
                super::super::storage::SearchIndexDocuments { own: vec![own_b.clone()], contained: Vec::new() },
                Some("boom".to_string()),
            ),
        ];
        let mut extract_failures: Vec<Option<String>> = vec![None; 7];
        let mut doc_counts: Vec<usize> = vec![0; 7];

        // start = 5: this sub-batch is the third of a bigger page, so its
        // owners land at the page-wide indices 5 and 6, not 0 and 1.
        let batch = flatten_sub_batch(5, prepared, &mut extract_failures, &mut doc_counts);

        assert_eq!(batch.own_owners, vec![5, 6]);
        assert_eq!(batch.own_docs, vec![own_a, own_b]);
        assert_eq!(batch.contained_owners, vec![5]);
        assert_eq!(batch.contained_docs, vec![contained_a]);
        assert_eq!(batch.len(), 3);
        assert_eq!(doc_counts[5], 2);
        assert_eq!(doc_counts[6], 1);
        assert_eq!(doc_counts[0], 0);
        assert_eq!(extract_failures[5], None);
        assert_eq!(extract_failures[6], Some("boom".to_string()));
    }

    #[test]
    fn page_outcomes_keeps_heads_precedence() {
        let extract_failures = vec![Some("extract failed".to_string()), None, None];
        let mut insert_failures = std::collections::HashMap::new();
        insert_failures.insert(1, "insert failed".to_string());
        let doc_counts = vec![0, 0, 4];
        let outcomes = page_outcomes(extract_failures, insert_failures, &doc_counts);
        assert!(matches!(&outcomes[0], Err(e) if e.to_string().contains("extract failed")));
        assert!(matches!(&outcomes[1], Err(e) if e.to_string().contains("insert failed")));
        assert!(matches!(outcomes[2], Ok(4)));
    }

    #[test]
    fn merge_insert_failures_prefers_own_over_contained() {
        let mut own = std::collections::HashMap::new();
        own.insert(3, "own failed".to_string());
        let mut contained = std::collections::HashMap::new();
        contained.insert(3, "contained failed".to_string());
        contained.insert(4, "contained only".to_string());
        let mut into = std::collections::HashMap::new();
        merge_insert_failures(&mut into, InsertFailures { own, contained });
        assert_eq!(into.get(&3), Some(&"own failed".to_string()));
        assert_eq!(into.get(&4), Some(&"contained only".to_string()));
    }

    #[test]
    fn absorb_db_task_adds_only_the_db_fields() {
        let mut stats = ReindexPageStats {
            extract: Duration::from_millis(99),
            sub_batches: 3,
            ..ReindexPageStats::default()
        };
        let t = ReindexPageStats {
            delete: Duration::from_millis(5),
            insert: Duration::from_millis(7),
            deleted_entries: 2,
            inserted_entries: 9,
            insert_commands: 1,
            extract: Duration::from_millis(1000), // must NOT be absorbed
            sub_batches: 100,                     // must NOT be absorbed
            db_wait: Some(Duration::from_millis(1)), // must NOT be absorbed
            ..ReindexPageStats::default()
        };
        absorb_db_task(&mut stats, &t);
        assert_eq!(stats.delete, Duration::from_millis(5));
        assert_eq!(stats.insert, Duration::from_millis(7));
        assert_eq!(stats.deleted_entries, 2);
        assert_eq!(stats.inserted_entries, 9);
        assert_eq!(stats.insert_commands, 1);
        assert_eq!(stats.extract, Duration::from_millis(99), "extract must be untouched");
        assert_eq!(stats.sub_batches, 3, "sub_batches must be untouched");
        assert_eq!(stats.db_wait, None, "db_wait is tracked only via add_db_wait");
    }

    #[test]
    fn add_db_wait_turns_none_into_some() {
        let mut stats = ReindexPageStats::default();
        let mut page_wait = Duration::ZERO;
        add_db_wait(&mut stats, &mut page_wait, Duration::from_millis(10));
        assert_eq!(stats.db_wait, Some(Duration::from_millis(10)));
        assert_eq!(page_wait, Duration::from_millis(10));
        add_db_wait(&mut stats, &mut page_wait, Duration::from_millis(5));
        assert_eq!(stats.db_wait, Some(Duration::from_millis(15)));
        assert_eq!(page_wait, Duration::from_millis(15));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn abort_on_drop_aborts_a_pending_task() {
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        struct DropSignal(Option<tokio::sync::oneshot::Sender<()>>);
        impl Drop for DropSignal {
            fn drop(&mut self) {
                if let Some(tx) = self.0.take() {
                    let _ = tx.send(());
                }
            }
        }
        let guard = DropSignal(Some(tx));
        let task = AbortOnDrop::spawn(async move {
            let _guard = guard;
            std::future::pending::<()>().await;
        });
        drop(task);
        tokio::time::timeout(Duration::from_secs(1), rx)
            .await
            .expect("dropping AbortOnDrop must abort the task within 1s")
            .expect("the guard's Drop must fire, delivering the oneshot");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn join_failure_resumes_a_panic() {
        let handle = tokio::spawn(async { panic!("boom") });
        let err = handle.await.expect_err("the spawned task panicked");
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| join_failure(err, "insert")));
        assert!(result.is_err(), "join_failure must resume the panic, not swallow it");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

```
cargo test -p helios-persistence --features mongodb --lib backends::mongodb::reindex_pipeline 2>&1 | tail -60
```

Expected failure: compile errors — `flatten_sub_batch`, `page_outcomes`, `merge_insert_failures`, `InsertFailures`, `absorb_db_task`, `add_db_wait`, `AbortOnDrop`, `join_failure` do not exist yet, and `SearchIndexDocuments` is not reachable as `super::super::storage::SearchIndexDocuments` (it will be once this task's `use` line is added — see Step 3).

- [ ] **Step 3: Implement**

Add near the top of `reindex_pipeline.rs`, below the module doc comment:

```rust
use std::collections::HashMap;
use std::time::Duration;

use mongodb::bson::{Bson, Document, doc};

use crate::error::StorageResult;
use crate::search::reindex::ReindexPageStats;
use crate::types::StoredResource;

use super::storage::{SearchIndexDocuments, internal_error};
```

Add, directly above `#[cfg(test)] mod tests`:

```rust
/// One sub-batch's extracted documents, split by destination collection, with
/// page-wide resource-index owners for each document (#1403).
struct SubBatchDocs {
    own_owners: Vec<usize>,
    own_docs: Vec<Document>,
    contained_owners: Vec<usize>,
    contained_docs: Vec<Document>,
}

impl SubBatchDocs {
    fn len(&self) -> usize {
        self.own_docs.len() + self.contained_docs.len()
    }
}

/// Moves sub-batch `[start, start + prepared.len())`'s extracted documents
/// into one [`SubBatchDocs`], recording each resource's extraction failure
/// and document count at its **page-wide** index, and moving (never cloning)
/// its documents — HEAD clones every document twice; this pipeline does not
/// (#1403).
fn flatten_sub_batch(
    start: usize,
    prepared: Vec<(SearchIndexDocuments, Option<String>)>,
    extract_failures: &mut [Option<String>],
    doc_counts: &mut [usize],
) -> SubBatchDocs {
    let mut batch = SubBatchDocs {
        own_owners: Vec::new(),
        own_docs: Vec::new(),
        contained_owners: Vec::new(),
        contained_docs: Vec::new(),
    };
    for (offset, (docs, failure)) in prepared.into_iter().enumerate() {
        let i = start + offset;
        doc_counts[i] = docs.own.len() + docs.contained.len();
        extract_failures[i] = failure;
        for d in docs.own {
            batch.own_owners.push(i);
            batch.own_docs.push(d);
        }
        for d in docs.contained {
            batch.contained_owners.push(i);
            batch.contained_docs.push(d);
        }
    }
    batch
}

/// HEAD's per-type delete grouping (`storage.rs:5054-5065`), in first-seen
/// type order: one `{tenant_id, resource_type, resource_id: {$in: ids}}` per
/// distinct type in the page (#1403).
fn delete_filters(tenant_id: &str, resources: &[StoredResource]) -> Vec<Document> {
    let mut order: Vec<&str> = Vec::new();
    let mut ids_by_type: HashMap<&str, Vec<Bson>> = HashMap::new();
    for resource in resources {
        let resource_type = resource.resource_type();
        ids_by_type
            .entry(resource_type)
            .or_insert_with(|| {
                order.push(resource_type);
                Vec::new()
            })
            .push(Bson::from(resource.id()));
    }
    order
        .into_iter()
        .map(|resource_type| {
            let ids = ids_by_type.remove(resource_type).unwrap_or_default();
            doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "resource_id": { "$in": ids },
            }
        })
        .collect()
}

/// What a database task hands back to the page's own thread when joined:
/// its own delete/insert phase timing and counts, and its outcome (#1403).
struct DbTask<T> {
    stats: ReindexPageStats,
    result: Result<T, String>,
}

/// Per-resource write errors for one sub-batch's insert, kept separate by
/// destination collection so [`merge_insert_failures`] can prefer an own-row
/// failure over a contained-row one, matching HEAD's precedence (#1403).
struct InsertFailures {
    own: HashMap<usize, String>,
    contained: HashMap<usize, String>,
}

/// Adds exactly `t`'s delete/insert phase fields to `stats`. Never calls
/// [`ReindexPageStats::accumulate`] (Task 2), which would fold busy time into
/// `db_wait`: the caller tracks `db_wait` itself, from the *join* wait via
/// [`add_db_wait`], not from a task's own busy time (#1403).
fn absorb_db_task(stats: &mut ReindexPageStats, t: &ReindexPageStats) {
    stats.delete += t.delete;
    stats.insert += t.insert;
    stats.deleted_entries += t.deleted_entries;
    stats.inserted_entries += t.inserted_entries;
    stats.insert_commands += t.insert_commands;
}

/// Records `d` as time the page's thread spent waiting on a joined database
/// task, both in `stats.db_wait` and in the caller's running `page_wait`
/// total (#1403).
fn add_db_wait(stats: &mut ReindexPageStats, page_wait: &mut Duration, d: Duration) {
    stats.db_wait = Some(stats.db_wait.unwrap_or_default() + d);
    *page_wait += d;
}

/// Merges one sub-batch's insert failures into the page's running map,
/// keeping the first message recorded per resource: own-row failures first,
/// then contained-row failures, matching HEAD's `entry().or_insert`
/// precedence (`storage.rs:5130-5148`) (#1403).
fn merge_insert_failures(into: &mut HashMap<usize, String>, f: InsertFailures) {
    for (owner, msg) in f.own {
        into.entry(owner).or_insert(msg);
    }
    for (owner, msg) in f.contained {
        into.entry(owner).or_insert(msg);
    }
}

/// HEAD's exact outcome precedence (`storage.rs:5153-5162`): an extraction
/// failure beats an insert failure, which beats success (#1403).
fn page_outcomes(
    extract_failures: Vec<Option<String>>,
    mut insert_failures: HashMap<usize, String>,
    doc_counts: &[usize],
) -> Vec<StorageResult<usize>> {
    extract_failures
        .into_iter()
        .enumerate()
        .map(|(i, failure)| match failure {
            Some(msg) => Err(internal_error(msg)),
            None => match insert_failures.remove(&i) {
                Some(msg) => Err(internal_error(msg)),
                None => Ok(doc_counts[i]),
            },
        })
        .collect()
}

/// `n` copies of `Err(msg)`, for a page-level failure that could not be
/// attributed to specific resources (#1403).
fn fan_out(n: usize, msg: &str) -> Vec<StorageResult<usize>> {
    (0..n).map(|_| Err(internal_error(msg.to_string()))).collect()
}

/// Turns a joined database task's `JoinError` into a page-level failure
/// message. A panic resumes here, exactly where an un-spawned (HEAD) call
/// would itself have panicked, so it still fails the job via `reindex.rs`'s
/// `catch_unwind` instead of being silently swallowed as an ordinary `Err`
/// (#1403).
fn join_failure(e: tokio::task::JoinError, what: &str) -> String {
    if e.is_panic() {
        std::panic::resume_unwind(e.into_panic());
    }
    format!("search index {what} task ended without a result: {e}")
}

/// A spawned task aborted, not just dropped, when this wrapper goes away — so
/// a run that stops mid-page (cancellation, a timeout on the composite ingest
/// sink) never leaves an orphaned delete or insert running against the
/// database (#1403).
pub(super) struct AbortOnDrop<T>(tokio::task::JoinHandle<T>);

impl<T: Send + 'static> AbortOnDrop<T> {
    fn spawn(fut: impl std::future::Future<Output = T> + Send + 'static) -> Self {
        Self(tokio::spawn(fut))
    }

    /// Awaits the task's result. Call at most once.
    async fn join(&mut self) -> Result<T, tokio::task::JoinError> {
        (&mut self.0).await
    }
}

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}
```

Change the test's `super::super::storage::SearchIndexDocuments { .. }` literals to plain `SearchIndexDocuments { .. }` once the `use super::storage::SearchIndexDocuments;` import above is in place (the test snippet in Step 1 spells it out fully only so Step 2's failure is obviously about missing items, not a missing import).

- [ ] **Step 4: Run the tests to verify they pass**

```
cargo test -p helios-persistence --features mongodb --lib backends::mongodb::reindex_pipeline 2>&1 | tail -80
```

Expect `test result: ok` for all tests, including Task 4's 8 and this task's 7.

- [ ] **Step 5: fmt, clippy, commit**

```
rustfmt --edition 2024 crates/persistence/src/backends/mongodb/reindex_pipeline.rs > /dev/null 2>&1
cargo clippy -p helios-persistence --all-targets --all-features -- -D warnings -A clippy::items_after_test_module -A clippy::large_enum_variant -A clippy::question_mark -A clippy::collapsible_match -A clippy::collapsible_if -A clippy::field_reassign_with_default -A clippy::doc-overindented-list-items -A clippy::doc-lazy-continuation 2>&1 | tail -40
```

```
git add crates/persistence/src/backends/mongodb/reindex_pipeline.rs
git commit -m "$(cat <<'EOF'
feat(mongodb): pure sub-batch writer helpers for the overlapped $reindex path (#1403)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 7a: Dispatcher (temporary, serial-only), serial writer, and the mode line

This task is split from a single, over-large "Task 7" so a reviewer can reject the serial writer independently of the overlapped one (each has its own test cycle and its own commit). Task 7b adds `write_page_overlapped` and rewrites the dispatcher this task writes.

**Files:**
- Modify: `crates/persistence/src/backends/mongodb/storage.rs` — anchors (re-verified at HEAD `c86d0f08b`): `async fn write_search_entries_page(` (`:4996`, becomes PR0's `write_search_entries_page_timed`, whose body this task replaces), `const SEARCH_INDEX_INSERT_CHUNK: usize = 5_000;` (`:5173`), `async fn insert_search_entries_chunk(` (`:5189`, PR0 already added a trailing `stats: &mut ReindexPageStats` parameter to it).
- Modify: `crates/persistence/src/backends/mongodb/reindex_pipeline.rs` (Tasks 4/6's file) — add `delete_page`, `insert_sub_batch`, and a new `impl super::MongoBackend { .. }` block holding `extract_one`, `write_page_serial`, `log_reindex_mode_once` (Task 7b extends this same block with `write_page_overlapped`).
- Modify: `crates/persistence/tests/mongodb/reindex_pipeline.rs` (exists after PR2a, holding its own capped-fetch tests and the `create_backend_with` helper per S3 §4.5) — add `build_test_patients` and one new integration test.

**Interfaces:**
- Consumes: `PrepareEnv`, `extract_range`, `REINDEX_SERIAL_POOL_MIN_PAGE` (Task 4); `SubBatchDocs`, `flatten_sub_batch`, `delete_filters`, `DbTask`, `InsertFailures`, `absorb_db_task`, `merge_insert_failures`, `page_outcomes`, `fan_out` (Task 6); `reindex_prepare_pool`/`reindex_prepare_gate`/`reindex_mode_logged`, `config().reindex_prepare_threads` (Task 5); `crate::perf::{span, record_duration, add_rows, Phase}` (existing; `Phase::ReindexDbWait` from Task 1).
- Produces: `write_search_entries_page_timed`'s **temporary** dispatcher body (`storage.rs`, always routes to the serial writer — Task 7b replaces it with the final overlap-routing version); `fn extract_one`, `pub(super) async fn write_page_serial`, `pub(super) fn log_reindex_mode_once` (`reindex_pipeline.rs`, new `impl super::MongoBackend { .. }` block — `log_reindex_mode_once` must be `pub(super)`, not a bare private `fn`: `storage.rs`'s dispatcher, in a sibling module, calls it, and an inherent method without `pub` is private to the module that holds its `impl` block); `pub(super) const SEARCH_INDEX_INSERT_CHUNK` and `pub(super) async fn insert_search_entries_chunk` (visibility only, `storage.rs`); test helper `fn build_test_patients` (`tests/mongodb/reindex_pipeline.rs`).

- [ ] **Step 1: Write the failing integration test**

Add to `crates/persistence/tests/mongodb/reindex_pipeline.rs` (a sibling to its existing PR2a tests and `create_backend_with` helper):

```rust
fn build_test_patients(tenant: &TenantContext, prefix: &str, n: usize) -> Vec<StoredResource> {
    (0..n)
        .map(|i| {
            StoredResource::from_storage(
                "Patient",
                format!("{prefix}-{i}"),
                "1",
                tenant.tenant_id().clone(),
                json!({
                    "resourceType": "Patient",
                    "id": format!("{prefix}-{i}"),
                    "name": [{"family": format!("F{i}")}]
                }),
                chrono::Utc::now(),
                chrono::Utc::now(),
                None,
                FhirVersion::default(),
            )
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mongodb_integration_reindex_page_serial_pipeline_writes_a_large_page() {
    let Some(backend) = create_backend_with("reindex_serial_pipeline_large_page", |c| {
        c.reindex_prepare_threads = 1;
    })
    .await
    else {
        eprintln!("Skipping mongodb_integration_reindex_page_serial_pipeline_writes_a_large_page (requires Docker)");
        return;
    };

    let tenant = create_tenant("reindex-serial-pipeline-tenant");
    let page = build_test_patients(&tenant, "serialbig", 300);

    let target: &dyn ReindexTarget = &*backend;
    let mut stats = ReindexPageStats::default();
    let outcomes = target
        .write_search_entries_page_timed(&tenant, &page, &mut stats)
        .await;
    assert!(outcomes.iter().all(|o| o.is_ok()), "{outcomes:?}");
    assert_eq!(
        stats.sub_batches, 1,
        "the serial path always runs exactly one extraction pass, whatever the page size"
    );
    assert!(stats.inserted_entries as usize >= page.len());

    for resource in &page {
        assert!(
            search_index_entry_count(&backend, &tenant, "Patient", resource.id()).await > 0,
            "resource {} must have a row",
            resource.id()
        );
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```
cargo test -p helios-persistence --features mongodb --test mongodb_tests reindex_page_serial_pipeline 2>&1 | tail -60
cargo test -p helios-persistence --features mongodb --test mongodb_tests mongodb_integration_reindex_page_batches_index_writes 2>&1 | tail -40
```

Expected failure: without Docker, the new test prints its skip line and passes vacuously — **run with Docker available** for this step to be meaningful. With Docker: `mongodb_integration_reindex_page_serial_pipeline_writes_a_large_page` fails because `stats.sub_batches` stays `0` (the dispatcher still runs HEAD's untouched body, which never sets it — `write_search_entries_page_timed` does not exist as this task's `write_page_serial` yet). `mongodb_integration_reindex_page_batches_index_writes` (unmodified, pre-existing) must still pass unchanged — it is this task's regression check.

- [ ] **Step 3: Implement**

**3a. Visibility.** In `storage.rs`, change `const SEARCH_INDEX_INSERT_CHUNK: usize = 5_000;` to `pub(super) const SEARCH_INDEX_INSERT_CHUNK: usize = 5_000;`. Change `async fn insert_search_entries_chunk(` to `pub(super) async fn insert_search_entries_chunk(`.

**3b. `storage.rs`: replace the dispatcher with a temporary, serial-only version.** Task 7b replaces this body again once `write_page_overlapped` exists. Keep the existing doc comment above `write_search_entries_page_timed` (PR0's text plus PR1's `Precondition:` paragraph) word for word; replace only the method body with:

```rust
    async fn write_search_entries_page_timed(
        &self,
        tenant: &TenantContext,
        resources: &[StoredResource],
        stats: &mut ReindexPageStats,
    ) -> Vec<StorageResult<usize>> {
        if resources.is_empty() {
            return Vec::new();
        }
        let _page_span = crate::perf::span(crate::perf::Phase::ReindexPage);
        if self.is_search_offloaded() {
            return resources.iter().map(|_| Ok(0)).collect();
        }
        let db = match self.get_database().await {
            Ok(db) => db,
            Err(e) => {
                let msg = e.to_string();
                return resources
                    .iter()
                    .map(|_| Err(internal_error(msg.clone())))
                    .collect();
            }
        };
        let tenant_id = tenant.tenant_id().as_str();
        let multi_thread = super::reindex_pipeline::tokio_multi_thread_runtime();
        if resources.len() > super::reindex_pipeline::REINDEX_SUBBATCH_FIRST {
            // `overlapped` is always `false` here: Task 7b adds
            // `write_page_overlapped` and rewrites this dispatcher to choose
            // between the two paths on `self.config().reindex_overlap`.
            self.log_reindex_mode_once(multi_thread, false);
        }
        self.write_page_serial(&db, tenant_id, resources, stats, multi_thread).await
    }
```

Add `use super::reindex_pipeline;` (or qualify inline as above — either is fine; the inline form avoids a new top-level import for two uses) at the top of `storage.rs` if not already present.

**3c. `reindex_pipeline.rs`: the serial writer body.** Add, above the existing `#[cfg(test)] mod tests`:

```rust
async fn delete_page(
    own: mongodb::Collection<Document>,
    contained: mongodb::Collection<Document>,
    filters: Vec<Document>,
) -> DbTask<()> {
    let started = std::time::Instant::now();
    let mut stats = ReindexPageStats::default();
    for filter in filters {
        match own.delete_many(filter.clone()).await {
            Ok(result) => stats.deleted_entries += result.deleted_count,
            Err(e) => {
                stats.delete += started.elapsed();
                return DbTask { stats, result: Err(format!("Failed to delete search entries: {e}")) };
            }
        }
        match contained.delete_many(filter).await {
            Ok(result) => stats.deleted_entries += result.deleted_count,
            Err(e) => {
                stats.delete += started.elapsed();
                return DbTask {
                    stats,
                    result: Err(format!("Failed to delete search_index_contained entries: {e}")),
                };
            }
        }
    }
    stats.delete += started.elapsed();
    DbTask { stats, result: Ok(()) }
}

async fn insert_sub_batch(
    own: mongodb::Collection<Document>,
    contained: mongodb::Collection<Document>,
    docs: SubBatchDocs,
) -> DbTask<InsertFailures> {
    let mut stats = ReindexPageStats::default();
    let started = std::time::Instant::now();
    let own_result = super::storage::insert_search_entries_chunk(
        &own,
        &docs.own_owners,
        &docs.own_docs,
        "Failed to insert search index entries",
        &mut stats,
    )
    .await;
    stats.insert += started.elapsed();
    let own_failures = match own_result {
        Ok(failures) => failures,
        Err(msg) => return DbTask { stats, result: Err(msg) },
    };
    let mut contained_failures = HashMap::new();
    if !docs.contained_docs.is_empty() {
        let started = std::time::Instant::now();
        let contained_result = super::storage::insert_search_entries_chunk(
            &contained,
            &docs.contained_owners,
            &docs.contained_docs,
            "Failed to insert search_index_contained entries",
            &mut stats,
        )
        .await;
        stats.insert += started.elapsed();
        contained_failures = match contained_result {
            Ok(failures) => failures,
            Err(msg) => return DbTask { stats, result: Err(msg) },
        };
    }
    DbTask { stats, result: Ok(InsertFailures { own: own_failures, contained: contained_failures }) }
}

impl super::MongoBackend {
    /// Extracts one resource's search-index documents (#1403).
    fn extract_one(&self, tenant_id: &str, r: &StoredResource) -> (SearchIndexDocuments, Option<String>) {
        self.search_index_documents_checked(tenant_id, r.resource_type(), r.id(), r.content())
    }

    /// Serial writer: HEAD's command order (extract, both deletes, own
    /// chunks, then contained chunks) — the only difference is that documents
    /// are moved instead of cloned. Uses the pool only for pages of at least
    /// `REINDEX_SERIAL_POOL_MIN_PAGE` on a multi-thread runtime; the pool
    /// accessor is never called otherwise, so single-resource
    /// `write_search_entries` calls and current-thread tests never build one
    /// (#1403, S3 §5.4).
    pub(super) async fn write_page_serial(
        &self,
        db: &mongodb::Database,
        tenant_id: &str,
        resources: &[StoredResource],
        stats: &mut ReindexPageStats,
        multi_thread: bool,
    ) -> Vec<StorageResult<usize>> {
        let n = resources.len();
        let own = db.collection::<Document>(Self::SEARCH_INDEX_COLLECTION);
        let contained = db.collection::<Document>(Self::SEARCH_INDEX_CONTAINED_COLLECTION);
        let prepare = |i: usize| self.extract_one(tenant_id, &resources[i]);

        let started = std::time::Instant::now();
        let pool = if multi_thread && n >= REINDEX_SERIAL_POOL_MIN_PAGE {
            self.reindex_prepare_pool()
        } else {
            None
        };
        let (prepared, on_pool) = if pool.is_some() {
            let env = PrepareEnv { pool, gate: self.reindex_prepare_gate() };
            extract_range(&env, 0..n, &prepare)
        } else {
            ((0..n).map(&prepare).collect(), false)
        };
        let mut extract_failures: Vec<Option<String>> = vec![None; n];
        let mut doc_counts: Vec<usize> = vec![0; n];
        let batch = flatten_sub_batch(0, prepared, &mut extract_failures, &mut doc_counts);
        stats.extract += started.elapsed();
        stats.sub_batches += 1;
        stats.pool_sub_batches += u64::from(on_pool);

        let filters = delete_filters(tenant_id, resources);
        let done = delete_page(own.clone(), contained.clone(), filters).await;
        absorb_db_task(stats, &done.stats);
        if let Err(msg) = done.result {
            // Serial db_wait is the awaited delete plus the awaited insert
            // (S3 §5.12); on this early return only the delete has run.
            stats.db_wait = Some(stats.delete + stats.insert);
            return fan_out(n, &msg);
        }

        let done = insert_sub_batch(own, contained, batch).await;
        absorb_db_task(stats, &done.stats);
        stats.db_wait = Some(stats.delete + stats.insert);
        let insert_failures = match done.result {
            Ok(failures) => {
                let mut into = HashMap::new();
                merge_insert_failures(&mut into, failures);
                into
            }
            Err(msg) => return fan_out(n, &msg),
        };

        let docs: u64 = doc_counts.iter().sum::<usize>() as u64;
        crate::perf::record_duration(crate::perf::Phase::ReindexExtract, stats.extract);
        crate::perf::record_duration(crate::perf::Phase::ReindexSearchDelete, stats.delete);
        crate::perf::record_duration(crate::perf::Phase::ReindexSearchInsert, stats.insert);
        crate::perf::record_duration(crate::perf::Phase::ReindexDbWait, stats.db_wait.unwrap_or_default());
        crate::perf::add_rows(crate::perf::Phase::ReindexSearchInsert, docs);
        page_outcomes(extract_failures, insert_failures, &doc_counts)
    }

    /// Logs `mongodb reindex writer configuration` once per backend instance,
    /// the first time a page of more than `REINDEX_SUBBATCH_FIRST` resources
    /// reaches the dispatcher (#1403, S3 §5.9).
    pub(super) fn log_reindex_mode_once(&self, multi_thread: bool, overlapped: bool) {
        if self
            .reindex_mode_logged()
            .swap(true, std::sync::atomic::Ordering::Relaxed)
        {
            return;
        }
        let prepare_threads_configured = self.config().reindex_prepare_threads;
        let prepare_threads = resolve_prepare_width(prepare_threads_configured);
        let pool = if prepare_threads < 2 {
            "none"
        } else if !multi_thread {
            "unused"
        } else if self.reindex_prepare_pool().is_some() {
            "ready"
        } else {
            "unavailable"
        };
        tracing::info!(
            overlap = self.config().reindex_overlap,
            prefetch = self.config().reindex_prefetch && !self.is_search_offloaded(),
            prepare_threads_configured,
            prepare_threads,
            pool,
            multi_thread_runtime = multi_thread,
            path = if overlapped { "overlapped" } else { "serial" },
            "mongodb reindex writer configuration"
        );
    }
}
```

Add `use crate::types::StoredResource;` (if Task 6 did not already add it) and `use std::time::Duration;` at the top of `reindex_pipeline.rs` (Task 6 already added both — verify, don't duplicate).

- [ ] **Step 4: Run the tests to verify they pass**

```
cargo test -p helios-persistence --features mongodb --test mongodb_tests reindex_page_serial_pipeline 2>&1 | tail -60
cargo test -p helios-persistence --features mongodb --test mongodb_tests mongodb_integration_reindex_page_batches_index_writes 2>&1 | tail -40
```

Expect `test result: ok` for both, with Docker running.

- [ ] **Step 5: fmt, clippy, commit**

```
rustfmt --edition 2024 crates/persistence/src/backends/mongodb/storage.rs crates/persistence/src/backends/mongodb/reindex_pipeline.rs crates/persistence/tests/mongodb/reindex_pipeline.rs > /dev/null 2>&1
cargo clippy -p helios-persistence --all-targets --all-features -- -D warnings -A clippy::items_after_test_module -A clippy::large_enum_variant -A clippy::question_mark -A clippy::collapsible_match -A clippy::collapsible_if -A clippy::field_reassign_with_default -A clippy::doc-overindented-list-items -A clippy::doc-lazy-continuation 2>&1 | tail -40
```

Expect this to pass with the `#![allow(dead_code)]` (Task 4) and `#[allow(dead_code)]` (Task 5) still suppressing warnings on `SubBatchPlanner`, `add_db_wait`, `join_failure`, `AbortOnDrop`, `REINDEX_SUBBATCH_FIRST`/`_TARGET_DOCS`/`_MAX`, `reindex_docs_seed` and `record_reindex_docs` — none of them is called by this task's code. Task 7b calls all of them and removes both allows.

```
git add crates/persistence/src/backends/mongodb/storage.rs crates/persistence/src/backends/mongodb/reindex_pipeline.rs crates/persistence/tests/mongodb/reindex_pipeline.rs
git commit -m "$(cat <<'EOF'
feat(mongodb): serial $reindex writer pipeline and the mode line, routed unconditionally for now (#1403)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 7b: Overlapped writer, final dispatcher routing, and the remaining integration tests

**Files:**
- Modify: `crates/persistence/src/backends/mongodb/storage.rs` — replace the temporary dispatcher body Task 7a wrote with the final version that routes to `write_page_overlapped` or `write_page_serial`.
- Modify: `crates/persistence/src/backends/mongodb/reindex_pipeline.rs` (Task 7a's `impl super::MongoBackend { .. }` block) — add `write_page_overlapped`.
- Modify: `crates/persistence/src/backends/mongodb/backend.rs` — delete the `#[allow(dead_code)]` Task 5 added above `impl MongoBackend { .. }`.
- Modify: `crates/persistence/tests/mongodb_tests.rs` — anchor: `struct FailPoint { .. }` and its `impl` (`:10111-10187`, re-verified), `static FAILPOINT_LOCK` (`:10108`) inside `mod bulk_submit` (`:10033`) — become `pub(super)` so the new module below can use them; the existing `mongodb_integration_reindex_page_batches_index_writes` test (`:8300-8446`, re-verified) must keep passing unmodified (Task 7a's serial pipeline already carries it).
- Modify: `crates/persistence/tests/mongodb/reindex_pipeline.rs` (Task 7a's file) — add `rows_without_id_and_tenant` and the remaining 5 overlapped-writer integration tests below.

**Interfaces:**
- Consumes: `SubBatchPlanner`, `PrepareEnv`, `extract_range`, `resolve_prepare_width`, `tokio_multi_thread_runtime`, `REINDEX_SUBBATCH_FIRST` (Task 4); `SubBatchDocs`, `flatten_sub_batch`, `delete_filters`, `DbTask`, `InsertFailures`, `absorb_db_task`, `add_db_wait`, `merge_insert_failures`, `page_outcomes`, `fan_out`, `join_failure`, `AbortOnDrop` (Task 6); `extract_one`, `delete_page`, `insert_sub_batch`, `log_reindex_mode_once` (Task 7a); `reindex_prepare_pool`/`reindex_prepare_gate`/`reindex_docs_seed`/`record_reindex_docs`, `config().reindex_overlap`/`reindex_prepare_threads`/`reindex_prefetch` (Task 5); `crate::perf::{span, record_duration, add_rows, Phase}` (existing; `Phase::ReindexDbWait` from Task 1); `build_test_patients` (Task 7a).
- Produces: `write_search_entries_page_timed`'s final dispatcher body (`storage.rs`); `pub(super) async fn write_page_overlapped` (added to Task 7a's `impl super::MongoBackend { .. }` block in `reindex_pipeline.rs`); `FailPoint`/`enable`/`wait_until_entered`/`off` become `pub(super)` (`mongodb_tests.rs`); test helper `async fn rows_without_id_and_tenant` (`tests/mongodb/reindex_pipeline.rs`).

**Invariant I1 for this task:** `write_page_overlapped` never has more than one `AbortOnDrop<DbTask<InsertFailures>>` alive in `in_flight` at a time — the loop always joins the previous one (step 2c) before spawning the next (step 2d), and the trailing drain after the loop joins the last one. A reviewer checking this task should specifically verify that property.

- [ ] **Step 1: Write the failing integration tests**

Add to `crates/persistence/tests/mongodb/reindex_pipeline.rs` (a sibling to Task 7a's `build_test_patients`):

```rust
/// Reads every row of `collection`, drops `_id` and `tenant_id`, and sorts a
/// deterministic per-row string — so two backends' collections can be
/// compared regardless of insert order or `_id` values (#1403, S3 §5.13).
async fn rows_without_id_and_tenant(backend: &MongoBackend, collection: &str) -> Vec<String> {
    use futures::stream::TryStreamExt;
    let db = backend.get_database().await.expect("get_database");
    let mut rows: Vec<String> = db
        .collection::<Document>(collection)
        .find(doc! {})
        .await
        .expect("find")
        .try_collect::<Vec<Document>>()
        .await
        .expect("collect")
        .into_iter()
        .map(|mut d| {
            d.remove("_id");
            d.remove("tenant_id");
            let mut keys: Vec<&String> = d.keys().collect();
            keys.sort();
            keys.into_iter()
                .map(|k| format!("{k}={:?}", d.get(k)))
                .collect::<Vec<_>>()
                .join(",")
        })
        .collect();
    rows.sort();
    rows
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mongodb_integration_reindex_page_overlapped_matches_serial() {
    let Some(serial) = create_backend_with("reindex_overlap_matches_serial", |c| {
        c.reindex_overlap = false;
        c.reindex_prepare_threads = 1;
    })
    .await
    else {
        eprintln!("Skipping mongodb_integration_reindex_page_overlapped_matches_serial (requires Docker)");
        return;
    };
    let overlapped = create_backend_with("reindex_overlap_matches_serial", |c| {
        c.reindex_overlap = true;
        c.reindex_prepare_threads = 0;
    })
    .await
    .expect("Docker was available for the serial backend, so it must be for the overlapped one too");

    let ts = create_tenant("reindex-overlap-serial-tenant-s");
    let to = create_tenant("reindex-overlap-serial-tenant-o");

    let build_page = |tenant: &TenantContext| -> Vec<StoredResource> {
        (0..300)
            .map(|n| {
                let mut resource = json!({
                    "resourceType": "Observation",
                    "id": format!("obs-{n}"),
                    "status": "final",
                    "code": {"coding": [{"system": "http://loinc.org", "code": "1234-5"}]},
                });
                if n % 10 == 0 {
                    resource["contained"] = json!([{
                        "resourceType": "Patient",
                        "id": "p1",
                        "name": [{"family": format!("Contained{n}")}]
                    }]);
                }
                StoredResource::from_storage(
                    "Observation",
                    format!("obs-{n}"),
                    "1",
                    tenant.tenant_id().clone(),
                    resource,
                    chrono::Utc::now(),
                    chrono::Utc::now(),
                    None,
                    FhirVersion::default(),
                )
            })
            .collect()
    };

    let page_s = build_page(&ts);
    let page_o = build_page(&to);
    let target_s: &dyn ReindexTarget = &*serial;
    let target_o: &dyn ReindexTarget = &*overlapped;
    let mut stats_s = ReindexPageStats::default();
    let mut stats_o = ReindexPageStats::default();
    let outcomes_s = target_s
        .write_search_entries_page_timed(&ts, &page_s, &mut stats_s)
        .await;
    let outcomes_o = target_o
        .write_search_entries_page_timed(&to, &page_o, &mut stats_o)
        .await;

    assert_eq!(outcomes_s.len(), outcomes_o.len());
    for (a, b) in outcomes_s.iter().zip(&outcomes_o) {
        assert_eq!(a.as_ref().ok(), b.as_ref().ok());
    }
    assert_eq!(stats_s.sub_batches, 1, "300 resources is one sub-batch serially");
    assert!(stats_o.sub_batches >= 2, "300 resources must split on the overlapped path");
    assert_eq!(stats_s.inserted_entries, stats_o.inserted_entries);

    assert_eq!(
        rows_without_id_and_tenant(&serial, "search_index").await,
        rows_without_id_and_tenant(&overlapped, "search_index").await
    );
    assert_eq!(
        rows_without_id_and_tenant(&serial, "search_index_contained").await,
        rows_without_id_and_tenant(&overlapped, "search_index_contained").await
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mongodb_integration_reindex_page_overlapped_command_shape() {
    let Some(backend) = create_backend_with("reindex_overlap_command_shape", |c| {
        c.reindex_overlap = true;
        c.reindex_prepare_threads = 1; // keep the assertions independent of pool scheduling
    })
    .await
    else {
        eprintln!("Skipping mongodb_integration_reindex_page_overlapped_command_shape (requires Docker)");
        return;
    };

    let tenant = create_tenant("reindex-overlap-shape-tenant");
    let page = build_test_patients(&tenant, "shape", 40);

    let db = backend.get_database().await.unwrap();
    let profiling_enabled = db.run_command(doc! { "profile": 2_i32 }).await.is_ok();
    if !profiling_enabled {
        eprintln!(
            "mongodb_integration_reindex_page_overlapped_command_shape: server refused \
             {{profile: 2}}; skipping"
        );
        return;
    }

    let target: &dyn ReindexTarget = &*backend;
    let mut stats = ReindexPageStats::default();
    let outcomes = target
        .write_search_entries_page_timed(&tenant, &page, &mut stats)
        .await;
    assert!(outcomes.iter().all(|o| o.is_ok()), "{outcomes:?}");
    assert!(
        stats.sub_batches >= 2,
        "a type's first page caps its first sub-batch at REINDEX_SUBBATCH_FIRST=32, so 40 must split"
    );

    db.run_command(doc! { "profile": 0_i32 })
        .await
        .expect("disable profiling");
    let ns = format!("{}.search_index", db.name());
    let entries: Vec<Document> = db
        .collection::<Document>("system.profile")
        .find(doc! { "ns": ns.as_str(), "op": { "$in": ["remove", "insert"] } })
        .sort(doc! { "ts": 1 })
        .await
        .expect("read system.profile")
        .try_collect()
        .await
        .expect("collect system.profile");

    let removes: Vec<&Document> = entries
        .iter()
        .filter(|e| e.get_str("op").ok() == Some("remove"))
        .collect();
    let inserts: Vec<&Document> = entries
        .iter()
        .filter(|e| e.get_str("op").ok() == Some("insert"))
        .collect();
    assert_eq!(removes.len(), 1, "exactly one delete_many for the page: {entries:?}");
    assert_eq!(inserts.len() as u64, stats.insert_commands, "{entries:?}");
    assert!(inserts.len() >= 2);
    let remove_ts = removes[0].get_datetime("ts").expect("remove ts");
    for insert in &inserts {
        let insert_ts = insert.get_datetime("ts").expect("insert ts");
        assert!(remove_ts <= insert_ts, "the delete must happen at or before every insert");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mongodb_integration_reindex_page_overlapped_issues_no_insert_after_a_failed_one() {
    let app_name = "reindex-overlap-fail-first-insert";
    let Some(backend) = create_backend_with("reindex_overlap_fails_first_insert", |c| {
        c.reindex_overlap = true;
        c.reindex_prepare_threads = 1;
        c.app_name = app_name.to_string();
        c.connection_string = append_query_param(&c.connection_string, "retryWrites=false");
    })
    .await
    else {
        eprintln!(
            "Skipping mongodb_integration_reindex_page_overlapped_issues_no_insert_after_a_failed_one (requires Docker)"
        );
        return;
    };

    let tenant = create_tenant("reindex-overlap-fail-first-tenant");
    let page = build_test_patients(&tenant, "failfirst", 80);

    // Seed the rows with the failpoint off, so the zero-row assertion below
    // actually shows the page's delete removed something (S3 §5.13 #3).
    let target: &dyn ReindexTarget = &*backend;
    let seed = target
        .write_search_entries_page_timed(&tenant, &page, &mut ReindexPageStats::default())
        .await;
    assert!(seed.iter().all(|o| o.is_ok()), "{seed:?}");
    assert!(
        search_index_entry_count(&backend, &tenant, "Patient", page[0].id()).await > 0,
        "the seed write must have created rows, or the later zero-row assertion proves nothing"
    );

    let Some(failpoint) = FailPoint::enable(
        app_name,
        doc! { "failCommands": ["insert"], "errorCode": 2 },
        doc! { "times": 1 },
    )
    .await
    else {
        eprintln!("Skipping: enableTestCommands unavailable");
        return;
    };

    let mut stats = ReindexPageStats::default();
    let outcomes = target
        .write_search_entries_page_timed(&tenant, &page, &mut stats)
        .await;
    failpoint.off().await;

    assert!(stats.sub_batches >= 2);
    assert!(
        outcomes
            .iter()
            .all(|o| matches!(o, Err(e) if e.to_string().contains("Failed to insert search index entries"))),
        "{outcomes:?}"
    );
    for resource in &page {
        let count = search_index_entry_count(&backend, &tenant, "Patient", resource.id()).await;
        assert_eq!(
            count, 0,
            "resource {} must have zero rows: the delete removed them and sub-batch 1's insert \
             failed before any row could be re-inserted",
            resource.id()
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mongodb_integration_reindex_page_overlapped_keeps_rows_of_completed_sub_batches() {
    let app_name = "reindex-overlap-fail-later-insert";
    let Some(backend) = create_backend_with("reindex_overlap_keeps_completed_rows", |c| {
        c.reindex_overlap = true;
        c.reindex_prepare_threads = 1;
        c.app_name = app_name.to_string();
        c.connection_string = append_query_param(&c.connection_string, "retryWrites=false");
    })
    .await
    else {
        eprintln!(
            "Skipping mongodb_integration_reindex_page_overlapped_keeps_rows_of_completed_sub_batches (requires Docker)"
        );
        return;
    };

    let tenant = create_tenant("reindex-overlap-fail-later-tenant");
    let page = build_test_patients(&tenant, "faillater", 80);

    // Seed the rows with the failpoint off, so the "non-empty prefix" claim
    // below shows real rows being kept, not an already-empty collection.
    let target: &dyn ReindexTarget = &*backend;
    let seed = target
        .write_search_entries_page_timed(&tenant, &page, &mut ReindexPageStats::default())
        .await;
    assert!(seed.iter().all(|o| o.is_ok()), "{seed:?}");
    assert!(
        search_index_entry_count(&backend, &tenant, "Patient", page[0].id()).await > 0,
        "the seed write must have created rows, or the later prefix assertion proves nothing"
    );

    let Some(failpoint) = FailPoint::enable(
        app_name,
        doc! { "failCommands": ["insert"], "errorCode": 2 },
        doc! { "skip": 1 },
    )
    .await
    else {
        eprintln!("Skipping: enableTestCommands unavailable");
        return;
    };

    let mut stats = ReindexPageStats::default();
    let outcomes = target
        .write_search_entries_page_timed(&tenant, &page, &mut stats)
        .await;
    failpoint.off().await;

    assert!(outcomes.iter().all(|o| o.is_err()), "{outcomes:?}");

    let mut has_rows: Vec<bool> = Vec::with_capacity(page.len());
    for resource in &page {
        has_rows.push(search_index_entry_count(&backend, &tenant, "Patient", resource.id()).await > 0);
    }
    let with_rows = has_rows.iter().filter(|b| **b).count();
    assert!(
        with_rows > 0 && with_rows < page.len(),
        "the rows must form a non-empty, proper prefix: {has_rows:?}"
    );
    assert!(has_rows[..with_rows].iter().all(|b| *b), "{has_rows:?}");
    assert!(has_rows[with_rows..].iter().all(|b| !*b), "{has_rows:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mongodb_integration_reindex_page_overlapped_fans_out_a_delete_failure() {
    let app_name = "reindex-overlap-fail-delete";
    let Some(backend) = create_backend_with("reindex_overlap_fans_out_delete_failure", |c| {
        c.reindex_overlap = true;
        c.reindex_prepare_threads = 1;
        c.app_name = app_name.to_string();
        c.connection_string = append_query_param(&c.connection_string, "retryWrites=false");
    })
    .await
    else {
        eprintln!(
            "Skipping mongodb_integration_reindex_page_overlapped_fans_out_a_delete_failure (requires Docker)"
        );
        return;
    };

    let tenant = create_tenant("reindex-overlap-fail-delete-tenant");
    let page = build_test_patients(&tenant, "faildelete", 40);

    let target: &dyn ReindexTarget = &*backend;
    let mut seed_stats = ReindexPageStats::default();
    let seed_outcomes = target
        .write_search_entries_page_timed(&tenant, &page, &mut seed_stats)
        .await;
    assert!(seed_outcomes.iter().all(|o| o.is_ok()));
    let mut before_counts = Vec::with_capacity(page.len());
    for resource in &page {
        before_counts.push(search_index_entry_count(&backend, &tenant, "Patient", resource.id()).await);
    }

    let Some(failpoint) = FailPoint::enable(
        app_name,
        doc! { "failCommands": ["delete"], "errorCode": 2 },
        doc! { "times": 1 },
    )
    .await
    else {
        eprintln!("Skipping: enableTestCommands unavailable");
        return;
    };

    let mut stats = ReindexPageStats::default();
    let outcomes = target
        .write_search_entries_page_timed(&tenant, &page, &mut stats)
        .await;
    failpoint.off().await;

    assert!(
        outcomes
            .iter()
            .all(|o| matches!(o, Err(e) if e.to_string().contains("Failed to delete search entries"))),
        "{outcomes:?}"
    );
    assert_eq!(stats.insert_commands, 0, "no insert may be spawned once the delete has failed");
    for (resource, before) in page.iter().zip(&before_counts) {
        let after = search_index_entry_count(&backend, &tenant, "Patient", resource.id()).await;
        assert_eq!(after, *before, "resource {} row count must be unchanged", resource.id());
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```
cargo test -p helios-persistence --features mongodb --test mongodb_tests reindex_page_overlapped 2>&1 | tail -80
```

Expected failure: without Docker, every test prints its skip line and passes vacuously — **run with Docker available** for this step to be meaningful. With Docker: compile errors first (`create_backend_with`'s `configure` closure has no `reindex_overlap`/`reindex_prepare_threads` fields to set — those land in Task 5, already done — and `FailPoint`/`enable` are private to `mod bulk_submit`, not yet visible here), then, once visibility is fixed, behavioral failures because the dispatcher still runs Task 7a's serial-only body (`stats.sub_batches` stays 1, not >= 2, since `write_page_overlapped` does not exist yet).

- [ ] **Step 3: Implement**

**3a. Visibility.** In `crates/persistence/tests/mongodb_tests.rs`, inside `mod bulk_submit`, change `struct FailPoint` to `pub(super) struct FailPoint`, and its three methods `enable`, `wait_until_entered`, `off` to `pub(super) async fn`. Leave `FAILPOINT_LOCK` private (only `enable`, inside the same module, takes it). Add, near the top of `crates/persistence/tests/mongodb/reindex_pipeline.rs`, exactly `use crate::bulk_submit::FailPoint;` — `mod bulk_submit { .. }` and `#[path = "mongodb/reindex_pipeline.rs"] mod reindex_pipeline;` are both declared at `mongodb_tests.rs`'s crate root (verify with `grep -n "^mod bulk_submit" crates/persistence/tests/mongodb_tests.rs`), so `crate::bulk_submit::FailPoint` is the exact, unambiguous path from any module `reindex_pipeline.rs` declares.

**3b. `storage.rs`: replace the dispatcher with its final version.** Keep the existing doc comment above `write_search_entries_page_timed` (PR0's text plus PR1's `Precondition:` paragraph) word for word; replace only the method body with:

```rust
    async fn write_search_entries_page_timed(
        &self,
        tenant: &TenantContext,
        resources: &[StoredResource],
        stats: &mut ReindexPageStats,
    ) -> Vec<StorageResult<usize>> {
        if resources.is_empty() {
            return Vec::new();
        }
        let _page_span = crate::perf::span(crate::perf::Phase::ReindexPage);
        if self.is_search_offloaded() {
            return resources.iter().map(|_| Ok(0)).collect();
        }
        let db = match self.get_database().await {
            Ok(db) => db,
            Err(e) => {
                let msg = e.to_string();
                return resources
                    .iter()
                    .map(|_| Err(internal_error(msg.clone())))
                    .collect();
            }
        };
        let tenant_id = tenant.tenant_id().as_str();
        let multi_thread = super::reindex_pipeline::tokio_multi_thread_runtime();
        let overlapped = self.config().reindex_overlap
            && multi_thread
            && resources.len() > super::reindex_pipeline::REINDEX_SUBBATCH_FIRST;
        if resources.len() > super::reindex_pipeline::REINDEX_SUBBATCH_FIRST {
            self.log_reindex_mode_once(multi_thread, overlapped);
        }
        if overlapped {
            self.write_page_overlapped(&db, tenant_id, resources, stats).await
        } else {
            self.write_page_serial(&db, tenant_id, resources, stats, multi_thread).await
        }
    }
```

**3c. `reindex_pipeline.rs`: add `write_page_overlapped` to Task 7a's `impl super::MongoBackend { .. }` block** (directly after `extract_one`, before `write_page_serial`):

```rust
    /// Overlapped writer (#1403, design C+, S3 §5.4): while sub-batch k's
    /// insert runs on a spawned task, the page's own thread extracts
    /// sub-batch k+1 inside `block_in_place`; the page's delete runs on its
    /// own task while sub-batch 1 is extracted. Caller guarantees a
    /// multi-thread runtime, `resources.len() > REINDEX_SUBBATCH_FIRST`, and
    /// search not offloaded. Invariant I1: at most one insert is ever
    /// `in_flight` — every loop iteration joins the previous one (2c) before
    /// spawning the next (2d), and the trailing drain after the loop joins
    /// the last one.
    pub(super) async fn write_page_overlapped(
        &self,
        db: &mongodb::Database,
        tenant_id: &str,
        resources: &[StoredResource],
        stats: &mut ReindexPageStats,
    ) -> Vec<StorageResult<usize>> {
        let n = resources.len();
        let own = db.collection::<Document>(Self::SEARCH_INDEX_COLLECTION);
        let contained = db.collection::<Document>(Self::SEARCH_INDEX_CONTAINED_COLLECTION);
        let pool = self.reindex_prepare_pool();
        let env = PrepareEnv { pool, gate: self.reindex_prepare_gate() };
        let min_size = if pool.is_some() {
            resolve_prepare_width(self.config().reindex_prepare_threads)
        } else {
            1
        };
        let filters = delete_filters(tenant_id, resources);
        let page_type: Option<&str> = (filters.len() == 1).then(|| resources[0].resource_type());
        let seed = page_type.and_then(|t| self.reindex_docs_seed(t));
        let prepare = |i: usize| self.extract_one(tenant_id, &resources[i]);

        // 1. The page's delete runs on its own task while sub-batch 1 is extracted.
        let mut delete = Some(AbortOnDrop::spawn(delete_page(own.clone(), contained.clone(), filters)));
        let mut planner = SubBatchPlanner::new(n, seed, min_size);
        let mut extract_failures: Vec<Option<String>> = vec![None; n];
        let mut doc_counts: Vec<usize> = vec![0; n];
        let mut insert_failures: HashMap<usize, String> = HashMap::new();
        let mut in_flight: Option<AbortOnDrop<DbTask<InsertFailures>>> = None;
        let mut page_wait = Duration::ZERO;

        while let Some(range) = planner.next_range() {
            // 2a. Extract this sub-batch; it overlaps the delete or the previous insert.
            let started = std::time::Instant::now();
            let (prepared, on_pool) = extract_range(&env, range.clone(), &prepare);
            let batch = flatten_sub_batch(range.start, prepared, &mut extract_failures, &mut doc_counts);
            stats.extract += started.elapsed();
            stats.sub_batches += 1;
            stats.pool_sub_batches += u64::from(on_pool);
            planner.record(range.len(), batch.len());

            // 2b. No insert may start before the page's delete has finished.
            if let Some(mut task) = delete.take() {
                let waited = std::time::Instant::now();
                let joined = task.join().await;
                add_db_wait(stats, &mut page_wait, waited.elapsed());
                let done = match joined {
                    Ok(done) => done,
                    Err(e) => return fan_out(n, &join_failure(e, "delete")),
                };
                absorb_db_task(stats, &done.stats);
                if let Err(msg) = done.result {
                    return fan_out(n, &msg);
                }
            }
            // 2c. At most one insert in flight: finish the previous sub-batch's insert first.
            if let Some(mut task) = in_flight.take() {
                let waited = std::time::Instant::now();
                let joined = task.join().await;
                add_db_wait(stats, &mut page_wait, waited.elapsed());
                let done = match joined {
                    Ok(done) => done,
                    Err(e) => return fan_out(n, &join_failure(e, "insert")),
                };
                absorb_db_task(stats, &done.stats);
                match done.result {
                    Ok(failures) => merge_insert_failures(&mut insert_failures, failures),
                    Err(msg) => return fan_out(n, &msg),
                }
            }
            // 2d. Start this sub-batch's insert.
            in_flight = Some(AbortOnDrop::spawn(insert_sub_batch(own.clone(), contained.clone(), batch)));
        }
        // 3. Drain the last insert exactly as in 2c.
        if let Some(mut task) = in_flight.take() {
            let waited = std::time::Instant::now();
            let joined = task.join().await;
            add_db_wait(stats, &mut page_wait, waited.elapsed());
            let done = match joined {
                Ok(done) => done,
                Err(e) => return fan_out(n, &join_failure(e, "insert")),
            };
            absorb_db_task(stats, &done.stats);
            match done.result {
                Ok(failures) => merge_insert_failures(&mut insert_failures, failures),
                Err(msg) => return fan_out(n, &msg),
            }
        }

        // 4. Success only: remember the type's documents per resource, forward to perf.
        let docs: u64 = doc_counts.iter().sum::<usize>() as u64;
        if let Some(t) = page_type {
            self.record_reindex_docs(t, n as u64, docs);
        }
        crate::perf::record_duration(crate::perf::Phase::ReindexExtract, stats.extract);
        crate::perf::record_duration(crate::perf::Phase::ReindexSearchDelete, stats.delete);
        crate::perf::record_duration(crate::perf::Phase::ReindexSearchInsert, stats.insert);
        crate::perf::record_duration(crate::perf::Phase::ReindexDbWait, page_wait);
        crate::perf::add_rows(crate::perf::Phase::ReindexSearchInsert, docs);
        page_outcomes(extract_failures, insert_failures, &doc_counts)
    }
```

**3d. Remove both dead-code allows.** Delete Task 4's `#![allow(dead_code)]` from the top of `reindex_pipeline.rs` and Task 5's `#[allow(dead_code)]` above `impl MongoBackend { .. }` in `backend.rs`: every item they were suppressing is now called (`SubBatchPlanner`/`REINDEX_SUBBATCH_FIRST`/`_TARGET_DOCS`/`_MAX` and `add_db_wait`/`join_failure`/`AbortOnDrop` by `write_page_overlapped` above; `reindex_docs_seed`/`record_reindex_docs` by the same). Step 6's clippy run must show no `dead_code` warning once both are gone.

- [ ] **Step 4: Run the new tests and the existing profiler regression test**

```
cargo test -p helios-persistence --features mongodb --test mongodb_tests reindex_page_overlapped 2>&1 | tail -100
cargo test -p helios-persistence --features mongodb --test mongodb_tests mongodb_integration_reindex_page_batches_index_writes 2>&1 | tail -40
```

Expect `test result: ok` for both, with Docker running. Confirm each overlapped test's output does **not** contain its "Skipping" line (a skip is not a pass).

- [ ] **Step 5: Run the whole MongoDB reindex suite once**

```
cargo test -p helios-persistence --features mongodb --test mongodb_tests reindex 2>&1 | tail -150
```

- [ ] **Step 6: fmt, clippy, commit**

```
rustfmt --edition 2024 crates/persistence/src/backends/mongodb/storage.rs crates/persistence/src/backends/mongodb/reindex_pipeline.rs crates/persistence/src/backends/mongodb/backend.rs crates/persistence/tests/mongodb_tests.rs crates/persistence/tests/mongodb/reindex_pipeline.rs > /dev/null 2>&1
cargo clippy -p helios-persistence --all-targets --all-features -- -D warnings -A clippy::items_after_test_module -A clippy::large_enum_variant -A clippy::question_mark -A clippy::collapsible_match -A clippy::collapsible_if -A clippy::field_reassign_with_default -A clippy::doc-overindented-list-items -A clippy::doc-lazy-continuation 2>&1 | tail -40
```

Expect this to pass with **no** `dead_code` warning — confirming Step 3d's two removed allows were no longer needed.

```
git add crates/persistence/src/backends/mongodb/storage.rs crates/persistence/src/backends/mongodb/reindex_pipeline.rs crates/persistence/src/backends/mongodb/backend.rs crates/persistence/tests/mongodb_tests.rs crates/persistence/tests/mongodb/reindex_pipeline.rs
git commit -m "$(cat <<'EOF'
feat(mongodb): overlapped $reindex writer and the final dispatcher routing (#1403)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 8: Driver-side id-phase prefetch (backend-agnostic)

**Files:**
- Modify: `crates/persistence/src/search/reindex.rs` — anchors (re-verified at HEAD `c86d0f08b`): `pub trait ReindexSource` (`:153`), `async fn fetch_resources_page_capped` default (`:184-195`, PR2a's already-extended version continues to exist here — this task adds two more defaulted methods right after it), the paging loop (`:2211-2278` at HEAD; by the time this task runs it is PR0/PR2a's version with `page_limit`, timing and `record_and_log_page` already woven in), `crate::perf::{span, Phase}` (existing).

**Interfaces:**
- Consumes: `ResourcePage`, `StorageResult`, `TenantContext` (existing); `Phase::ReindexFetch` (existing), `Phase::ReindexFetchWait` (Task 1); `PageRecord.fetch_wait` (Task 3).
- Produces:
  - `ReindexSource::may_prefetch_page(&self, cursor: &str) -> bool` (default `false`).
  - `ReindexSource::fetch_resources_page_ahead(&self, tenant: &TenantContext, resource_type: &str, cursor: &str, limit: u32, max_bytes: u64) -> StorageResult<Option<ResourcePage>>` (default: the capped fetch, wrapped in `Some`).
  - `struct PrefetchedPage` with `fn spawn(..) -> Self` and `async fn wait(self) -> (StorageResult<Option<ResourcePage>>, Duration)`.
  - The paging loop now tracks `fetch` and `fetch_wait` separately and spawns a prefetch when `source.may_prefetch_page(next)` is true.

- [ ] **Step 1: Write the failing driver tests**

Add to `crates/persistence/src/search/reindex.rs`'s existing `mod tests`, after `PagedSource`'s `impl ReindexSource for PagedSource` block:

```rust
    /// One event of a [`PrefetchingSource`]/[`PrefetchingWriter`] run (#1403).
    /// `page` is the 1-based page number the event concerns.
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum PrefetchEvent {
        FetchStart(String, usize),
        FetchEnd(String, usize),
        AheadDeclined(String, usize),
        WriteStart(String, usize),
        WriteEnd(String, usize),
    }

    /// A scripted multi-type, multi-page `ReindexSource`. Cursors are plain
    /// `"{type}:{page}"`, or carry a caller-chosen prefix from `special`
    /// (`"hold:{type}:{page}"` makes [`Self::may_prefetch_page`] decline it;
    /// `"edge:{type}:{page}"` makes it accepted but
    /// [`Self::fetch_resources_page_ahead`] decline the actual fetch). A
    /// per-cursor gate ([`Self::gate_for`]) blocks that cursor's fetch until
    /// released; [`Self::fail_cursor`]/[`Self::panic_cursor`] make it error or
    /// panic instead of returning data (#1403).
    struct PrefetchingSource {
        events: Arc<parking_lot::Mutex<Vec<PrefetchEvent>>>,
        types: Vec<(String, Vec<Vec<String>>)>,
        special: HashMap<(String, usize), &'static str>,
        gates: parking_lot::Mutex<HashMap<String, Arc<tokio::sync::Notify>>>,
        fail: parking_lot::Mutex<std::collections::HashSet<String>>,
        panic: parking_lot::Mutex<std::collections::HashSet<String>>,
    }

    impl PrefetchingSource {
        fn new(types: Vec<(&str, Vec<Vec<&str>>)>) -> (Arc<Self>, Arc<parking_lot::Mutex<Vec<PrefetchEvent>>>) {
            Self::with_special(types, HashMap::new())
        }

        fn with_special(
            types: Vec<(&str, Vec<Vec<&str>>)>,
            special: HashMap<(String, usize), &'static str>,
        ) -> (Arc<Self>, Arc<parking_lot::Mutex<Vec<PrefetchEvent>>>) {
            let events = Arc::new(parking_lot::Mutex::new(Vec::new()));
            let types = types
                .into_iter()
                .map(|(t, pages)| {
                    (
                        t.to_string(),
                        pages.into_iter().map(|p| p.into_iter().map(String::from).collect()).collect(),
                    )
                })
                .collect();
            let source = Arc::new(Self {
                events: events.clone(),
                types,
                special,
                gates: parking_lot::Mutex::new(HashMap::new()),
                fail: parking_lot::Mutex::new(std::collections::HashSet::new()),
                panic: parking_lot::Mutex::new(std::collections::HashSet::new()),
            });
            (source, events)
        }

        fn cursor_for(&self, resource_type: &str, page: usize) -> String {
            match self.special.get(&(resource_type.to_string(), page)) {
                Some(prefix) => format!("{prefix}:{resource_type}:{page}"),
                None => format!("{resource_type}:{page}"),
            }
        }

        fn strip_prefix(cursor: &str) -> &str {
            cursor.strip_prefix("hold:").or_else(|| cursor.strip_prefix("edge:")).unwrap_or(cursor)
        }

        fn type_of(cursor: &str) -> String {
            Self::strip_prefix(cursor).rsplit_once(':').map(|(t, _)| t.to_string()).unwrap_or_default()
        }

        fn page_number(cursor: &str) -> usize {
            Self::strip_prefix(cursor).rsplit(':').next().and_then(|s| s.parse().ok()).unwrap_or(0)
        }

        fn total_pages(&self, resource_type: &str) -> usize {
            self.types.iter().find(|(t, _)| t == resource_type).map(|(_, p)| p.len()).unwrap_or(0)
        }

        fn gate_for(&self, cursor: &str) -> Arc<tokio::sync::Notify> {
            self.gates
                .lock()
                .entry(cursor.to_string())
                .or_insert_with(|| Arc::new(tokio::sync::Notify::new()))
                .clone()
        }

        fn fail_cursor(&self, cursor: &str) {
            self.fail.lock().insert(cursor.to_string());
        }

        fn panic_cursor(&self, cursor: &str) {
            self.panic.lock().insert(cursor.to_string());
        }

        async fn fetch(&self, cursor: &str) -> StorageResult<Vec<String>> {
            let resource_type = Self::type_of(cursor);
            let page = Self::page_number(cursor);
            self.events.lock().push(PrefetchEvent::FetchStart(resource_type.clone(), page));
            if self.panic.lock().contains(cursor) {
                panic!("PrefetchingSource: scripted panic for {cursor}");
            }
            // Bind the gate first: `if let Some(x) = <mutex guard>.method()` keeps
            // the `parking_lot::MutexGuard` temporary alive through the `then`
            // branch in edition 2024 (its temporaries drop only before `else`),
            // and parking_lot's guards are `!Send` without the `send_guard`
            // feature (not enabled, `Cargo.toml:63`), so awaiting inside that
            // branch would make this method's future non-`Send` — required by
            // `#[async_trait]` for `fetch_resources_page`/`_page_ahead` (#1403).
            let gate = self.gates.lock().get(cursor).cloned();
            if let Some(notify) = gate {
                notify.notified().await;
            }
            let result = if self.fail.lock().contains(cursor) {
                Err(crate::error::StorageError::Backend(crate::error::BackendError::Internal {
                    backend_name: "prefetching-source".to_string(),
                    message: "scripted fetch failure".to_string(),
                    source: None,
                }))
            } else {
                let (_, pages) = self.types.iter().find(|(t, _)| *t == resource_type).expect("known type");
                Ok(pages.get(page.saturating_sub(1)).cloned().unwrap_or_default())
            };
            self.events.lock().push(PrefetchEvent::FetchEnd(resource_type, page));
            result
        }

        fn build_page(&self, tenant: &TenantContext, resource_type: &str, page: usize, ids: Vec<String>) -> ResourcePage {
            let resources = ids
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
                .collect();
            let next_cursor = (page < self.total_pages(resource_type)).then(|| self.cursor_for(resource_type, page + 1));
            ResourcePage { resources, next_cursor, skipped: Vec::new() }
        }
    }

    #[async_trait]
    impl ReindexSource for PrefetchingSource {
        async fn list_resource_types(&self, _: &TenantContext) -> StorageResult<Vec<String>> {
            Ok(self.types.iter().map(|(t, _)| t.clone()).collect())
        }

        async fn count_resources(&self, _: &TenantContext, resource_type: &str) -> StorageResult<u64> {
            let (_, pages) = self.types.iter().find(|(t, _)| t == resource_type).expect("known type");
            Ok(pages.iter().map(|p| p.len() as u64).sum())
        }

        async fn fetch_resources_page(
            &self,
            tenant: &TenantContext,
            resource_type: &str,
            cursor: Option<&str>,
            _limit: u32,
        ) -> StorageResult<ResourcePage> {
            let (page, cursor_str) = match cursor {
                Some(c) => (Self::page_number(c), c.to_string()),
                None => (1, self.cursor_for(resource_type, 1)),
            };
            let ids = self.fetch(&cursor_str).await?;
            Ok(self.build_page(tenant, resource_type, page, ids))
        }

        fn may_prefetch_page(&self, cursor: &str) -> bool {
            !cursor.starts_with("hold:")
        }

        async fn fetch_resources_page_ahead(
            &self,
            tenant: &TenantContext,
            resource_type: &str,
            cursor: &str,
            _limit: u32,
            _max_bytes: u64,
        ) -> StorageResult<Option<ResourcePage>> {
            if cursor.starts_with("edge:") {
                let page = Self::page_number(cursor);
                self.events.lock().push(PrefetchEvent::AheadDeclined(resource_type.to_string(), page));
                return Ok(None);
            }
            let ids = self.fetch(cursor).await?;
            let page = Self::page_number(cursor);
            Ok(Some(self.build_page(tenant, resource_type, page, ids)))
        }
    }

    /// The 1-based page number a `PrefetchingSource` resource id encodes: the
    /// numeric suffix of its id, plus one (`"p3"` -> page 4). Every fixture in
    /// this test module uses ids of the form `{letters}{digits}`, one page per
    /// resource, or a fixed-size page per type, so this always agrees with
    /// [`PrefetchingSource::page_number`]'s cursor-derived numbering (#1403).
    fn resource_page_number(id: &str) -> usize {
        let digits: String = id.chars().skip_while(|c| !c.is_ascii_digit()).collect();
        digits.parse::<usize>().map(|n| n + 1).unwrap_or(0)
    }

    /// Pairs with [`PrefetchingSource`]. Overrides `write_search_entries_page`
    /// so one event covers the whole page, and asserts (`AtomicBool::swap`)
    /// that no second page's write starts before the first has returned. The
    /// page number recorded in `WriteStart`/`WriteEnd` comes from the first
    /// resource's id ([`resource_page_number`]), not from a call counter, so a
    /// test asserting pages arrived in a particular order can actually fail
    /// (#1403).
    struct PrefetchingWriter {
        events: Arc<parking_lot::Mutex<Vec<PrefetchEvent>>>,
        writing: Arc<AtomicBool>,
        /// When set to `Some((resource_type, page))`, that page's write blocks,
        /// with a 2s timeout, until `events` shows `FetchStart(resource_type,
        /// page + 1)` — proving a prefetch of the *next* page is genuinely in
        /// flight before this page's write is allowed to finish. Sets
        /// `hold_timed_out` instead of panicking on timeout, so the test can
        /// assert on it directly (#1403).
        hold_until_next_fetch: Option<(String, usize)>,
        hold_timed_out: Arc<AtomicBool>,
        /// When set to `Some((resource_type, page, notify))`, that page's
        /// write blocks on `notify` after `WriteStart` is recorded and before
        /// `WriteEnd` — lets a test hold a page's write open on purpose, to
        /// prove a job was cancelled while that page was still in flight
        /// (#1403).
        write_gate: Option<(String, usize, Arc<tokio::sync::Notify>)>,
    }

    #[async_trait]
    impl ReindexTarget for PrefetchingWriter {
        async fn delete_search_entries(&self, _: &TenantContext, _: &str, _: &str) -> StorageResult<u64> {
            Ok(0)
        }

        async fn write_search_entries(&self, _: &TenantContext, _: &StoredResource) -> StorageResult<usize> {
            Ok(1)
        }

        async fn write_search_entries_page(
            &self,
            _: &TenantContext,
            resources: &[StoredResource],
        ) -> Vec<StorageResult<usize>> {
            if resources.is_empty() {
                return Vec::new();
            }
            let resource_type = resources[0].resource_type().to_string();
            let page = resource_page_number(resources[0].id());

            assert!(
                !self.writing.swap(true, Ordering::SeqCst),
                "two pages' writes overlapped"
            );
            self.events.lock().push(PrefetchEvent::WriteStart(resource_type.clone(), page));

            if let Some((hold_type, hold_page)) = &self.hold_until_next_fetch {
                if hold_type == &resource_type && *hold_page == page {
                    let next_page = page + 1;
                    let events = self.events.clone();
                    let target_type = resource_type.clone();
                    let waited = tokio::time::timeout(Duration::from_secs(2), async move {
                        loop {
                            let seen = events.lock().iter().any(|e| {
                                matches!(e, PrefetchEvent::FetchStart(t, p) if *t == target_type && *p == next_page)
                            });
                            if seen {
                                break;
                            }
                            tokio::time::sleep(Duration::from_millis(5)).await;
                        }
                    })
                    .await;
                    if waited.is_err() {
                        self.hold_timed_out.store(true, Ordering::SeqCst);
                    }
                }
            }

            if let Some((gate_type, gate_page, notify)) = &self.write_gate {
                if gate_type == &resource_type && *gate_page == page {
                    notify.notified().await;
                }
            }

            tokio::task::yield_now().await;
            self.events.lock().push(PrefetchEvent::WriteEnd(resource_type, page));
            self.writing.store(false, Ordering::SeqCst);
            resources.iter().map(|_| Ok(1)).collect()
        }

        async fn clear_search_index(&self, _: &TenantContext) -> StorageResult<u64> {
            Ok(0)
        }
    }

    fn prefetch_fixture(
        source: Arc<PrefetchingSource>,
        events: Arc<parking_lot::Mutex<Vec<PrefetchEvent>>>,
    ) -> Arc<ReindexOperation> {
        let writer = Arc::new(PrefetchingWriter {
            events,
            writing: Arc::new(AtomicBool::new(false)),
            hold_until_next_fetch: None,
            hold_timed_out: Arc::new(AtomicBool::new(false)),
            write_gate: None,
        });
        Arc::new(ReindexOperation::with_parts(
            source,
            vec![writer as Arc<dyn ReindexTarget>],
            Arc::new(crate::search::TenantSearchRegistries::base_only()),
        ))
    }

    #[tokio::test]
    async fn prefetch_fetches_the_next_page_while_the_current_one_is_written() {
        let (source, events) =
            PrefetchingSource::new(vec![("Patient", vec![vec!["p0", "p1"], vec!["p2"]])]);
        let hold_timed_out = Arc::new(AtomicBool::new(false));
        let writer = Arc::new(PrefetchingWriter {
            events: events.clone(),
            writing: Arc::new(AtomicBool::new(false)),
            hold_until_next_fetch: Some(("Patient".to_string(), 1)),
            hold_timed_out: hold_timed_out.clone(),
            write_gate: None,
        });
        let op = Arc::new(ReindexOperation::with_parts(
            source,
            vec![writer as Arc<dyn ReindexTarget>],
            Arc::new(crate::search::TenantSearchRegistries::base_only()),
        ));
        let job = op
            .start(
                named_tenant("prefetch-overlap"),
                ReindexRequest::for_types(vec!["Patient".to_string()]).with_batch_size(2),
                None,
            )
            .await
            .expect("start");

        let progress = await_finished(&op, &job).await;
        assert_eq!(progress.status, ReindexStatus::Completed);
        assert!(
            !hold_timed_out.load(Ordering::SeqCst),
            "page 1's write must observe page 2's fetch already started within 2s: a \
             strictly serial driver never starts it until page 1's write has returned, \
             so it would time out here"
        );
    }

    #[tokio::test]
    async fn prefetch_keeps_page_writes_serial_and_in_fetch_order() {
        let (source, events) = PrefetchingSource::new(vec![(
            "Patient",
            vec![vec!["p0"], vec!["p1"], vec!["p2"], vec!["p3"], vec!["p4"]],
        )]);
        let op = prefetch_fixture(source.clone(), events.clone());
        let job = op
            .start(
                named_tenant("prefetch-serial-order"),
                ReindexRequest::for_types(vec!["Patient".to_string()]).with_batch_size(1),
                None,
            )
            .await
            .expect("start");
        let progress = await_finished(&op, &job).await;
        assert_eq!(progress.status, ReindexStatus::Completed);
        assert_eq!(progress.processed_resources, 5);

        let log = events.lock();
        let pages_in_order: Vec<usize> = log
            .iter()
            .filter_map(|e| match e {
                PrefetchEvent::WriteStart(t, p) if t == "Patient" => Some(*p),
                _ => None,
            })
            .collect();
        assert_eq!(pages_in_order, vec![1, 2, 3, 4, 5], "{log:?}");
    }

    #[tokio::test]
    async fn prefetch_never_crosses_a_resource_type() {
        let (source, events) = PrefetchingSource::new(vec![
            ("A", vec![vec!["a0"], vec!["a1"]]),
            ("B", vec![vec!["b0"], vec!["b1"]]),
        ]);
        let op = prefetch_fixture(source.clone(), events.clone());
        let job = op
            .start(
                named_tenant("prefetch-no-cross-type"),
                ReindexRequest::for_types(vec!["A".to_string(), "B".to_string()]).with_batch_size(1),
                None,
            )
            .await
            .expect("start");
        let progress = await_finished(&op, &job).await;
        assert_eq!(progress.status, ReindexStatus::Completed);

        let log = events.lock();
        let write_end_a2 = log
            .iter()
            .position(|e| matches!(e, PrefetchEvent::WriteEnd(t, 2) if t == "A"))
            .expect("WriteEnd(A,2)");
        let fetch_start_b1 = log
            .iter()
            .position(|e| matches!(e, PrefetchEvent::FetchStart(t, 1) if t == "B"))
            .expect("FetchStart(B,1)");
        assert!(fetch_start_b1 > write_end_a2, "{log:?}");
    }

    #[tokio::test]
    async fn prefetch_waits_for_the_write_when_the_source_declines_the_cursor() {
        let mut special = HashMap::new();
        special.insert(("Patient".to_string(), 2), "hold");
        let (source, events) =
            PrefetchingSource::with_special(vec![("Patient", vec![vec!["p0"], vec!["p1"]])], special);
        let op = prefetch_fixture(source.clone(), events.clone());
        let job = op
            .start(
                named_tenant("prefetch-declines-cursor"),
                ReindexRequest::for_types(vec!["Patient".to_string()]).with_batch_size(1),
                None,
            )
            .await
            .expect("start");
        let progress = await_finished(&op, &job).await;
        assert_eq!(progress.status, ReindexStatus::Completed);

        let log = events.lock();
        let write_end_1 = log
            .iter()
            .position(|e| matches!(e, PrefetchEvent::WriteEnd(t, 1) if t == "Patient"))
            .expect("WriteEnd(Patient,1)");
        let fetch_start_2 = log
            .iter()
            .position(|e| matches!(e, PrefetchEvent::FetchStart(t, 2) if t == "Patient"))
            .expect("FetchStart(Patient,2)");
        assert!(fetch_start_2 > write_end_1, "{log:?}");
    }

    #[tokio::test]
    async fn cancellation_with_prefetch_writes_nothing_after_the_page_in_flight() {
        // `ReindexOperation::cancel` writes `ReindexStatus::Cancelled`
        // synchronously (it does not wait for the task to stop), so this test
        // must not treat "status is Cancelled" as proof the prefetch was
        // aborted. It instead proves I1/cancellation two ways: (1) it holds
        // page 1's write open with `write_gate` until *after* `cancel()` has
        // been called, so the cancellation genuinely lands while a page is in
        // flight and a prefetch of page 2 is genuinely running; (2) it polls
        // `cancel_channels` — which the task's own return removes — with a
        // timeout, proving the un-awaited, gated-forever prefetch of page 2
        // was aborted rather than awaited (#1403).
        let (source, events) =
            PrefetchingSource::new(vec![("Patient", vec![vec!["p0", "p1"], vec!["p2"]])]);
        let _blocked_forever = source.gate_for("Patient:2"); // never notified
        let write_gate = Arc::new(tokio::sync::Notify::new());
        let writer = Arc::new(PrefetchingWriter {
            events: events.clone(),
            writing: Arc::new(AtomicBool::new(false)),
            hold_until_next_fetch: None,
            hold_timed_out: Arc::new(AtomicBool::new(false)),
            write_gate: Some(("Patient".to_string(), 1, write_gate.clone())),
        });
        let op = Arc::new(ReindexOperation::with_parts(
            source,
            vec![writer as Arc<dyn ReindexTarget>],
            Arc::new(crate::search::TenantSearchRegistries::base_only()),
        ));
        let job = op
            .start(
                named_tenant("prefetch-cancel"),
                ReindexRequest::for_types(vec!["Patient".to_string()]).with_batch_size(2),
                None,
            )
            .await
            .expect("start");

        // Wait for page 1's write to start (it is gated open by `write_gate`)
        // and for page 2's prefetch to start — proving a prefetch is
        // genuinely in flight — before cancelling.
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let ready = {
                    let log = events.lock();
                    log.iter().any(|e| matches!(e, PrefetchEvent::WriteStart(t, 1) if t == "Patient"))
                        && log.iter().any(|e| matches!(e, PrefetchEvent::FetchStart(t, 2) if t == "Patient"))
                };
                if ready {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("page 1's write and page 2's prefetch must both start");

        op.cancel(&job).await.expect("cancel");
        write_gate.notify_one();

        tokio::time::timeout(Duration::from_secs(2), async {
            while op.cancel_channels.read().contains_key(&job) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect(
            "the cancelled reindex task did not return: a blocked, un-awaited \
             prefetch must be aborted, not awaited",
        );

        let progress = op.get_progress(&job).await.expect("progress");
        assert_eq!(progress.status, ReindexStatus::Cancelled);
        let write_starts = events
            .lock()
            .iter()
            .filter(|e| matches!(e, PrefetchEvent::WriteStart(..)))
            .count();
        assert_eq!(write_starts, 1, "only page 1 may have been written");
    }

    #[tokio::test]
    async fn a_failed_prefetch_fails_the_run_after_the_page_in_flight_is_written() {
        let (source, events) =
            PrefetchingSource::new(vec![("Patient", vec![vec!["p0", "p1"], vec!["p2"]])]);
        source.fail_cursor("Patient:2");
        let op = prefetch_fixture(source.clone(), events.clone());
        let job = op
            .start(
                named_tenant("prefetch-fail"),
                ReindexRequest::for_types(vec!["Patient".to_string()]).with_batch_size(2),
                None,
            )
            .await
            .expect("start");
        let progress = await_finished(&op, &job).await;
        assert_eq!(progress.status, ReindexStatus::Failed);
        assert!(
            progress.error_message.as_deref().unwrap_or("").starts_with("Failed to fetch resources:"),
            "{:?}",
            progress.error_message
        );
        assert_eq!(progress.processed_resources, 2, "page 1 (2 resources) must have been written first");
    }

    #[tokio::test]
    async fn a_panicking_prefetch_fails_the_run() {
        let (source, events) =
            PrefetchingSource::new(vec![("Patient", vec![vec!["p0", "p1"], vec!["p2"]])]);
        source.panic_cursor("Patient:2");
        let op = prefetch_fixture(source.clone(), events);
        let job = op
            .start(
                named_tenant("prefetch-panic"),
                ReindexRequest::for_types(vec!["Patient".to_string()]).with_batch_size(2),
                None,
            )
            .await
            .expect("start");
        let progress = await_finished(&op, &job).await;
        assert_eq!(progress.status, ReindexStatus::Failed);
        assert_eq!(
            progress.error_message.as_deref(),
            Some("Reindex task panicked before completing")
        );
    }

    #[tokio::test]
    async fn prefetch_refetches_serially_after_the_write_when_the_ahead_fetch_declines() {
        let mut special = HashMap::new();
        special.insert(("Patient".to_string(), 2), "edge");
        let (source, events) =
            PrefetchingSource::with_special(vec![("Patient", vec![vec!["p0", "p1"], vec!["p2"]])], special);
        let op = prefetch_fixture(source.clone(), events.clone());
        let job = op
            .start(
                named_tenant("prefetch-edge"),
                ReindexRequest::for_types(vec!["Patient".to_string()]).with_batch_size(2),
                None,
            )
            .await
            .expect("start");
        let progress = await_finished(&op, &job).await;
        assert_eq!(progress.status, ReindexStatus::Completed);
        assert_eq!(progress.processed_resources, 3);

        let log = events.lock();
        let write_end_1 = log
            .iter()
            .position(|e| matches!(e, PrefetchEvent::WriteEnd(t, 1) if t == "Patient"))
            .expect("WriteEnd(Patient,1)");
        let fetch_starts_2: Vec<usize> = log
            .iter()
            .enumerate()
            .filter(|(_, e)| matches!(e, PrefetchEvent::FetchStart(t, 2) if t == "Patient"))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(fetch_starts_2.len(), 1, "exactly one serial FetchStart(2): {log:?}");
        assert!(
            fetch_starts_2[0] > write_end_1,
            "the serial re-fetch must start after page 1's write: {log:?}"
        );
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

```
cargo test -p helios-persistence --lib search::reindex::tests::prefetch 2>&1 | tail -60
cargo test -p helios-persistence --lib search::reindex::tests::a_failed_prefetch 2>&1 | tail -30
cargo test -p helios-persistence --lib search::reindex::tests::a_panicking_prefetch 2>&1 | tail -30
cargo test -p helios-persistence --lib search::reindex::tests::cancellation_with_prefetch 2>&1 | tail -30
```

Expected failure: compile errors — `ReindexSource` has no `may_prefetch_page`/`fetch_resources_page_ahead` to override (a trait impl providing methods the trait doesn't declare is a hard error), and every test hangs or fails behaviorally even once it compiles, because the driver never prefetches yet.

- [ ] **Step 3: Implement**

Add two defaulted methods to `pub trait ReindexSource`, directly after `fetch_resources_page_capped`:

```rust
    /// Whether the reindex driver may fetch the page at `cursor` — the
    /// `next_cursor` of the page it is about to write — while it writes that
    /// page (#1403). Pages are still written one at a time, in fetch order,
    /// and a prefetched page a run no longer needs (cancellation, failure) is
    /// dropped unwritten. A source answers `false` for a cursor whose fetch
    /// must observe the previous page's writes (for example one that starts a
    /// catch-up round). `false` (the default) keeps the strictly serial
    /// fetch → write loop.
    fn may_prefetch_page(&self, cursor: &str) -> bool {
        let _ = cursor;
        false
    }

    /// Fetches the page at `cursor` ahead of time, while the driver is still
    /// writing the page whose `next_cursor` it is (#1403). The driver calls
    /// it only for a cursor that [`Self::may_prefetch_page`] accepted.
    /// `Ok(Some(page))` is that page. `Ok(None)` means the source will not run
    /// this fetch ahead of the write, because it would end a walk phase (for
    /// example an empty continuation query): the driver then fetches the same
    /// cursor with [`Self::fetch_resources_page_capped`] after the in-flight
    /// write has finished. A source must not log or change any state when it
    /// returns `Ok(None)`. The default runs the ordinary capped fetch.
    async fn fetch_resources_page_ahead(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        cursor: &str,
        limit: u32,
        max_bytes: u64,
    ) -> StorageResult<Option<ResourcePage>> {
        self.fetch_resources_page_capped(tenant, resource_type, Some(cursor), limit, max_bytes)
            .await
            .map(Some)
    }
```

Add a new private type above `run_reindex`:

```rust
/// A page fetch running ahead of the page being written. Dropping it aborts
/// the fetch, so a run that stops never leaves one behind (#1403).
struct PrefetchedPage {
    handle: tokio::task::JoinHandle<(StorageResult<Option<ResourcePage>>, Duration)>,
}

impl PrefetchedPage {
    fn spawn(
        source: Arc<dyn ReindexSource>,
        tenant: TenantContext,
        resource_type: String,
        cursor: String,
        limit: u32,
        max_bytes: u64,
    ) -> Self {
        Self {
            handle: tokio::spawn(async move {
                let started = Instant::now();
                let _span = crate::perf::span(crate::perf::Phase::ReindexFetch);
                let page = source
                    .fetch_resources_page_ahead(&tenant, &resource_type, &cursor, limit, max_bytes)
                    .await;
                (page, started.elapsed())
            }),
        }
    }

    /// A panic inside the fetch resumes here, where an unprefetched fetch
    /// would itself have panicked.
    async fn wait(mut self) -> (StorageResult<Option<ResourcePage>>, Duration) {
        let _span = crate::perf::span(crate::perf::Phase::ReindexFetchWait);
        match (&mut self.handle).await {
            Ok(done) => done,
            Err(e) if e.is_panic() => std::panic::resume_unwind(e.into_panic()),
            Err(e) => (
                Err(crate::error::StorageError::Backend(crate::error::BackendError::Internal {
                    backend_name: "reindex".to_string(),
                    message: format!("page prefetch ended without a result: {e}"),
                    source: None,
                })),
                Duration::ZERO,
            ),
        }
    }
}

impl Drop for PrefetchedPage {
    fn drop(&mut self) {
        self.handle.abort();
    }
}
```

Rewrite the paging loop's cursor/fetch handling (the loop body added by PR0/PR2a around `page_limit`, keeping every other line — the cancellation check, `write_resource_batch`, `record_and_log_page`, and the trailing `match page.next_cursor` — exactly as PR0/PR2a leave them, except that the `PageRecord { .. }` literal's `fetch: fetch_time` becomes `fetch` and its `fetch_wait: fetch_time` (added by Task 3) becomes `fetch_wait`, both bound below):

```rust
    let mut prefetched: Option<PrefetchedPage> = None;
    loop {
        if cancel_rx.try_recv().is_ok() {
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
                        let fetched = source
                            .fetch_resources_page_capped(
                                &tenant,
                                resource_type,
                                cursor.as_deref(),
                                page_limit,
                                request.batch_bytes,
                            )
                            .await;
                        drop(fetch_span);
                        (fetched, ahead_fetch + serial_started.elapsed(), wait_started.elapsed())
                    }
                    Err(e) => (Err(e), ahead_fetch, wait_started.elapsed()),
                }
            }
            None => {
                let fetch_span = crate::perf::span(crate::perf::Phase::ReindexFetch);
                let fetched = source
                    .fetch_resources_page_capped(&tenant, resource_type, cursor.as_deref(), page_limit, request.batch_bytes)
                    .await;
                drop(fetch_span);
                let fetch = wait_started.elapsed();
                (fetched, fetch, fetch) // exactly equal when nothing was prefetched
            }
        };
        let page = match fetched {
            Ok(page) => page,
            Err(e) => return Err(RunExit::Failed(format!("Failed to fetch resources: {e}"))),
        };
        // Fetch the next page of THIS type while this one is written, when the source allows it.
        if let Some(next) = page.next_cursor.as_deref()
            && source.may_prefetch_page(next)
        {
            prefetched = Some(PrefetchedPage::spawn(
                source.clone(),
                tenant.clone(),
                resource_type.to_string(),
                next.to_string(),
                page_limit,
                request.batch_bytes,
            ));
        }
        // A row the source read but could not decode is a resource that
        // stays unsearchable until the row is repaired: a permanent
        // failure, recorded rather than silently dropped (#1125).
        for skipped in &page.skipped {
            record_resource_failure(
                &jobs,
                &job_id,
                &mut failures,
                resource_type,
                &skipped.resource_id,
                format!("Failed to read stored resource: {}", skipped.reason),
                false,
            );
        }

        // Rebuild the page through every writer.
        let batch_outcome = write_resource_batch(
            &tenant,
            &writers,
            &jobs,
            &job_id,
            &mut failures,
            resource_type,
            &page.resources,
            page.skipped.len() as u64,
        )
        .await;

        record_and_log_page(
            &mut stats,
            &tenant_label,
            &job_id,
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

        // Check if there are more pages
        match page.next_cursor {
            Some(next) => {
                cursor = Some(next);
                // Stand back before re-taking the write lock for the
                // next page. Only between pages: the last page has no
                // successor to hold the lock against.
                let yielded = Instant::now();
                yield_between_pages().await;
                stats.add_yield(yielded.elapsed());
            }
            None => break,
        }
    }
    if let Some(summary) = stats.finish_type(OUTCOME_COMPLETED, Instant::now()) {
        log_type_finished(&tenant_label, &job_id, &summary);
    }
```

Every line above `fetch`/`fetch_wait`'s binding and the `if let Some(next) = page.next_cursor.as_deref() && source.may_prefetch_page(next) { .. }` block is this task's new code; everything from the `for skipped in &page.skipped` loop onward is PR0's paging-path body (S1 §5.1(g) step 9), reproduced verbatim except for two edits: the `PageRecord { .. }` literal's `fetch: fetch_time` becomes `fetch` and gains `fetch_wait` (both bound by the `match prefetched.take() { .. }` above, replacing PR0's single `fetch_time` local), and the yield timing (`let yielded = Instant::now(); yield_between_pages().await; stats.add_yield(yielded.elapsed());`) and the closing `if let Some(summary) = stats.finish_type(..)` block are carried over unchanged from PR0's version of this loop. The named-resources branch (`:2167-2209` at HEAD) is untouched by this task: it never prefetches, so its `PageRecord` keeps `fetch: fetch_time, fetch_wait: fetch_time` (Task 3's binding), which stays correct.

- [ ] **Step 4: Run the tests to verify they pass**

```
cargo test -p helios-persistence --lib search::reindex:: 2>&1 | tail -150
```

Expect `test result: ok` for all 8 new tests, and every pre-existing test in `search::reindex` — in particular `cancellation_during_a_page_finishes_it_and_stops_before_the_next_fetch` (`:4018`) and `pages_are_written_with_a_gap_a_foreground_writer_can_use` (`:4201`), whose sources keep the default `may_prefetch_page = false` and so must be completely unaffected.

- [ ] **Step 5: fmt, clippy, commit**

```
rustfmt --edition 2024 crates/persistence/src/search/reindex.rs > /dev/null 2>&1
cargo clippy -p helios-persistence --all-targets --all-features -- -D warnings -A clippy::items_after_test_module -A clippy::large_enum_variant -A clippy::question_mark -A clippy::collapsible_match -A clippy::collapsible_if -A clippy::field_reassign_with_default -A clippy::doc-overindented-list-items -A clippy::doc-lazy-continuation 2>&1 | tail -40
```

```
git add crates/persistence/src/search/reindex.rs
git commit -m "$(cat <<'EOF'
feat(persistence): backend-agnostic reindex driver prefetch (may_prefetch_page / fetch_resources_page_ahead) (#1403)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 9: MongoDB's prefetch override (`may_prefetch_page`, `fetch_resources_page_ahead`, `reindex_id_page`)

**Files:**
- Modify: `crates/persistence/src/backends/mongodb/storage.rs` — anchor: `impl ReindexSource for MongoBackend` (`:4754` at HEAD; by this task PR1/PR2a have already added `fetch_resources_page_capped` to this block per S3 §4.2 item 5). PR1's `impl MongoBackend { .. }` block that holds `reindex_newest_live_last_updated`/`reindex_find_page` (added just before `:4754`, S2 §7.4) is where this task's new `reindex_id_page` method goes. PR2a's `fetch_reindex_page` (S3 §4.2 item 3, replacing S2's `fetch_resources_page` body) is where this task extracts the `IdPhase` arm from.

**Interfaces:**
- Consumes: `ReindexWalkCursor::{parse, encode}` (PR1, private to `storage.rs`), `reindex_find_page` (PR2a's extended signature, taking `max_bytes`), `reindex_page_from_docs` (PR1, `fn reindex_page_from_docs(docs: &[Document], resource_type: &str, tenant: &TenantContext, next_cursor: ReindexWalkCursor) -> StorageResult<ResourcePage>` — PR1's own conversion helper, already used by both of PR1's `fetch_resources_page` arms, S2 §7.4), `RESOURCES_IDENTITY_INDEX` (PR1, from `schema.rs`), `reindex_id_page_filter` (PR1), `config().reindex_prefetch`, `is_search_offloaded()` (existing).
- Produces:
  - `impl ReindexSource for MongoBackend { fn may_prefetch_page(&self, cursor: &str) -> bool; async fn fetch_resources_page_ahead(..) -> StorageResult<Option<ResourcePage>>; }`
  - `async fn reindex_id_page(&self, tenant: &TenantContext, resource_type: &str, floor: DateTime<Utc>, after_id: Option<&str>, limit: u32, max_bytes: u64) -> StorageResult<Option<ResourcePage>>` (private inherent method).

- [ ] **Step 1: Write the failing unit tests**

Add a new `#[cfg(test)] mod reindex_prefetch_tests` at the end of `storage.rs` (no MongoDB server needed — `MongoBackend::new`'s client is lazy, per S3 §5.13):

```rust
#[cfg(test)]
mod reindex_prefetch_tests {
    use super::*;
    use crate::backends::mongodb::MongoBackendConfig;
    use crate::search::reindex::ReindexSource;
    use crate::tenant::{TenantId, TenantPermissions};

    /// A config that can never reach a real server. `MongoBackendConfig::default()`'s
    /// connection string is `mongodb://localhost:27017` — this dev box's
    /// long-lived `hfs-mongo` corpus container, which the Global Constraints
    /// say never to touch — so every backend built in this module uses this
    /// instead, even where the code path never actually calls the database
    /// today, in case a future change moves a database call earlier (#1403).
    fn unreachable_config() -> MongoBackendConfig {
        MongoBackendConfig {
            connection_string: "mongodb://127.0.0.1:1".to_string(),
            server_selection_timeout_ms: 500,
            ..Default::default()
        }
    }

    fn id_cursor() -> String {
        ReindexWalkCursor::Id { floor: chrono::Utc::now(), after_id: "p1".to_string() }.encode()
    }

    fn round_cursor() -> String {
        ReindexWalkCursor::Round {
            round: 1,
            floor: chrono::Utc::now(),
            ceiling: chrono::Utc::now() + chrono::Duration::seconds(1),
            walked: 0,
            after_last_updated: chrono::Utc::now(),
            after_id: "p1".to_string(),
        }
        .encode()
    }

    #[test]
    fn may_prefetch_page_accepts_only_id_cursors() {
        let backend = MongoBackend::new(unreachable_config()).expect("lazy client");
        assert!(backend.may_prefetch_page(&id_cursor()));
        assert!(!backend.may_prefetch_page(&round_cursor()));
        assert!(!backend.may_prefetch_page("garbage"));

        let no_prefetch = MongoBackend::new(MongoBackendConfig {
            reindex_prefetch: false,
            ..unreachable_config()
        })
        .expect("lazy client");
        assert!(!no_prefetch.may_prefetch_page(&id_cursor()));

        let offloaded = MongoBackend::new(MongoBackendConfig {
            search_offloaded: true,
            ..unreachable_config()
        })
        .expect("lazy client");
        assert!(!offloaded.may_prefetch_page(&id_cursor()));
    }

    #[tokio::test]
    async fn fetch_ahead_declines_round_and_malformed_cursors() {
        // Both cases compile against Task 8's defaults today: the trait
        // default's `may_prefetch_page` always returns `false`, so this test
        // fails on its first assertion until Step 3 lands. Once it does, both
        // cases return before any database call — `ReindexWalkCursor::parse`
        // fails/mismatches before `get_database` is ever reached — so
        // `unreachable_config`'s bogus connection string is exercised only as
        // a defensive belt-and-suspenders, not because either case connects.
        let backend = MongoBackend::new(unreachable_config()).expect("lazy client");
        let tenant = TenantContext::new(TenantId::new("prefetch-test-tenant"), TenantPermissions::full_access());

        let result = backend
            .fetch_resources_page_ahead(&tenant, "Patient", &round_cursor(), 10, 0)
            .await
            .expect("no database error");
        assert!(result.is_none(), "a Round cursor must never be fetched ahead");

        let result = backend
            .fetch_resources_page_ahead(&tenant, "Patient", "garbage", 10, 0)
            .await
            .expect("no database error");
        assert!(result.is_none(), "a cursor that fails to parse must never be fetched ahead");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```
cargo test -p helios-persistence --features mongodb --lib backends::mongodb::storage::reindex_prefetch_tests 2>&1 | tail -40
```

Expected failure: compile errors — `may_prefetch_page` and `fetch_resources_page_ahead` are not overridden by `MongoBackend` yet, so these calls resolve to the trait's defaults, which do not read `config().reindex_prefetch` or reject `Round`/malformed cursors the way the test expects (the `may_prefetch_page` assertions fail outright since the default always returns `false`, making the first assertion `assert!(backend.may_prefetch_page(&id_cursor()))` fail).

- [ ] **Step 3: Implement**

Add the new inherent method to PR1's `impl MongoBackend { .. }` block (the one holding `reindex_newest_live_last_updated` and `reindex_find_page`, S2 §7.4, immediately before `impl ReindexSource for MongoBackend`):

```rust
    /// Runs only the id-phase continuation query (§3.3), for both the serial
    /// walk and the driver's ahead-of-time prefetch — so both paths build the
    /// same page from the same query (#1403). `Ok(None)` means the id phase is
    /// over; it logs nothing else in that case (per `ReindexSource::
    /// fetch_resources_page_ahead`'s doc contract, Task 8, a source must not
    /// log or change state when it returns `Ok(None)`), leaving the capped-page
    /// debug line and the phase transition to whichever caller runs the query
    /// when it is *not* prefetched.
    async fn reindex_id_page(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        floor: DateTime<Utc>,
        after_id: Option<&str>,
        limit: u32,
        max_bytes: u64,
    ) -> StorageResult<Option<ResourcePage>> {
        let db = self.get_database().await?;
        let resources = db.collection::<Document>(Self::RESOURCES_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();
        let found = self
            .reindex_find_page(
                &resources,
                reindex_id_page_filter(tenant_id, resource_type, floor, after_id),
                doc! { "id": 1 },
                RESOURCES_IDENTITY_INDEX,
                limit,
                max_bytes,
            )
            .await?;
        if found.docs.is_empty() {
            return Ok(None);
        }
        if max_bytes > 0 {
            tracing::debug!(
                tenant = %tenant_id,
                resource_type = %resource_type,
                rows = found.docs.len(),
                bytes = found.bytes,
                capped = found.capped,
                "mongodb reindex capped page read"
            );
        }
        let last_id = found
            .docs
            .last()
            .and_then(|d| d.get_str("id").ok())
            .map(str::to_string)
            .ok_or_else(|| internal_error("Missing id on the last row of an id-phase page".to_string()))?;
        reindex_page_from_docs(
            &found.docs,
            resource_type,
            tenant,
            ReindexWalkCursor::Id { floor, after_id: last_id },
        )
        .map(Some)
    }
```

Note two deliberate choices against the design's literal text. First, the empty check moves **before** the capped-page debug line (the design's §5.10 prose logs it unconditionally): otherwise both the ahead fetch's empty read and the serial re-fetch's empty read would each log a `rows=0` line for the same query, which is both a duplicate and a violation of the `fetch_resources_page_ahead` doc contract quoted above. Second, it calls PR1's own `reindex_page_from_docs(docs: &[Document], resource_type: &str, tenant: &TenantContext, next_cursor: ReindexWalkCursor) -> StorageResult<ResourcePage>` (S2 §7.4) instead of hand-rolling the conversion: that helper already exists, is exercised by both of PR1's own `fetch_resources_page` arms, and does exactly `docs.iter().map(|doc| parse_history_row(doc, Some(resource_type), None).map(|row| row.into_stored_resource(tenant))).collect::<StorageResult<Vec<_>>>()` then wraps the result in a `ResourcePage` — `parse_history_row` takes `&Document` and returns `StorageResult<ParsedHistoryRow>`, not `Document`/`ParsedHistoryRow` directly, so this must go through `reindex_page_from_docs` rather than being reimplemented inline.

Change the `IdPhase` arm of `fetch_reindex_page` (PR2a's version, S3 §4.2 item 3) to call this new method instead of running the query inline, and delete that arm's own capped-page debug line (now emitted by `reindex_id_page` itself, only for a non-empty read):

```rust
    WalkStep::IdPhase { floor, after_id } => {
        if let Some(page) = self
            .reindex_id_page(tenant, resource_type, floor, after_id.as_deref(), limit, max_bytes)
            .await?
        {
            return Ok(page);
        }
        tracing::info!(
            tenant = %tenant_id,
            resource_type = %resource_type,
            floor = %format_walk_instant(floor),
            "mongodb reindex id phase finished"
        );
        WalkStep::RoundStart { round: 1, floor }
    }
```

This is the arm's entire new body — nothing else in it changes. Verify PR2a's actual `IdPhase` arm (`tgrep -n -F -- "WalkStep::IdPhase" crates/persistence/src/backends/mongodb/storage.rs`) uses the same `tenant = %tenant_id, resource_type = %resource_type` field style before replacing it; if PR2a's own text differs, keep its exact field style and only replace the query-and-convert body with the `if let Some(page) = self.reindex_id_page(..)` call and delete its capped-page debug line — the log line's field *format* must stay `%`-prefixed (unquoted strings), matching PR1's `id phase finished` line and every other walk log line, since S4's parser and this plan's own Task 10 grep the unquoted `tenant=<id>` / `resource_type=<type>` form.

Add the override to `impl ReindexSource for MongoBackend`:

```rust
    fn may_prefetch_page(&self, cursor: &str) -> bool {
        self.config().reindex_prefetch
            && !self.is_search_offloaded()
            && matches!(ReindexWalkCursor::parse(cursor), Ok(ReindexWalkCursor::Id { .. }))
    }

    async fn fetch_resources_page_ahead(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        cursor: &str,
        limit: u32,
        max_bytes: u64,
    ) -> StorageResult<Option<ResourcePage>> {
        // Only an id continuation runs ahead. Everything else, including the
        // end of the id phase, is fetched serially after the page in flight
        // is written.
        let Ok(ReindexWalkCursor::Id { floor, after_id }) = ReindexWalkCursor::parse(cursor) else {
            return Ok(None);
        };
        self.reindex_id_page(tenant, resource_type, floor, Some(&after_id), limit.max(1), max_bytes)
            .await
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

```
cargo test -p helios-persistence --features mongodb --lib backends::mongodb::storage:: 2>&1 | tail -80
```

Expect `test result: ok`, including `reindex_prefetch_tests::*` and every pre-existing `reindex_walk_tests`/`reindex_page_cap_tests` module (this task changes only the `IdPhase` arm's *mechanism*, not the rows or cursor it produces, so PR1/PR2a's walk-shape and cap tests must be unaffected).

- [ ] **Step 5: fmt, clippy, commit**

```
rustfmt --edition 2024 crates/persistence/src/backends/mongodb/storage.rs > /dev/null 2>&1
cargo clippy -p helios-persistence --all-targets --all-features -- -D warnings -A clippy::items_after_test_module -A clippy::large_enum_variant -A clippy::question_mark -A clippy::collapsible_match -A clippy::collapsible_if -A clippy::field_reassign_with_default -A clippy::doc-overindented-list-items -A clippy::doc-lazy-continuation 2>&1 | tail -40
```

```
git add crates/persistence/src/backends/mongodb/storage.rs
git commit -m "$(cat <<'EOF'
feat(mongodb): id-phase reindex prefetch (may_prefetch_page, fetch_resources_page_ahead) (#1403)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 10: Cross-cutting integration tests — consecutive-version revisits, knob parity, and log-order proof

**Files:**
- Modify: `crates/persistence/tests/mongodb/reindex_id_walk.rs` (exists after PR1) — anchor: `capture_walk_logs` and `walk_log_lines` (S2 §9's log-capture helpers) become `pub(super)`, so `reindex_pipeline.rs`'s test module can share PR1's one global tracing subscriber instead of installing a second, conflicting one.
- Modify: `crates/persistence/tests/mongodb/reindex_pipeline.rs` (Tasks 7a/7b/9's file) — add `PhaseLogProbeTarget` and the three remaining §5.13 integration tests.

**Interfaces:**
- Consumes: `create_backend_with` (PR2a), `rows_without_id_and_tenant` (Task 7b), `build_test_patients` (Task 7a); `ReindexPageStats`, `may_prefetch_page` (Tasks 2, 8, 9); `capture_walk_logs()`/`walk_log_lines(needles: &[&str]) -> Vec<String>` (PR1, now `pub(super)` — `capture_walk_logs` takes no arguments and returns `()`; `walk_log_lines` returns every captured line containing **every** string in `needles`, so a needle list must include a tenant needle to isolate this test's own lines from every other walk the shared global subscriber has captured in this test binary).
- Produces: `struct ScriptedResourceSource` (test-only `ReindexSource`), `struct PhaseLogProbeTarget` (test-only `ReindexTarget` wrapper), 3 new integration tests.

- [ ] **Step 1: Write the failing integration tests**

In `crates/persistence/tests/mongodb/reindex_id_walk.rs`, change `fn capture_walk_logs` and `fn walk_log_lines` (however S2 declared them — a plain `fn`/`async fn` at module scope) to `pub(super) fn` / `pub(super) async fn` respectively, with no other change.

Add to `crates/persistence/tests/mongodb/reindex_pipeline.rs`:

```rust
/// An in-memory, scripted `ReindexSource` that always allows prefetch
/// (#1403) — used only to prove I1's ordering guarantee against a *real*
/// MongoDB writer, independent of MongoDB's own walk.
struct ScriptedResourceSource {
    resource_type: &'static str,
    pages: Vec<Vec<StoredResource>>,
}

#[async_trait::async_trait]
impl ReindexSource for ScriptedResourceSource {
    async fn list_resource_types(&self, _: &TenantContext) -> StorageResult<Vec<String>> {
        Ok(vec![self.resource_type.to_string()])
    }

    async fn count_resources(&self, _: &TenantContext, _: &str) -> StorageResult<u64> {
        Ok(self.pages.iter().map(|p| p.len() as u64).sum())
    }

    async fn fetch_resources_page(
        &self,
        _: &TenantContext,
        _: &str,
        cursor: Option<&str>,
        _: u32,
    ) -> StorageResult<ResourcePage> {
        let page = cursor.and_then(|c| c.parse::<usize>().ok()).unwrap_or(0);
        let resources = self.pages.get(page).cloned().unwrap_or_default();
        let next_cursor = (page + 1 < self.pages.len()).then(|| (page + 1).to_string());
        Ok(ResourcePage { resources, next_cursor, skipped: Vec::new() })
    }

    fn may_prefetch_page(&self, _: &str) -> bool {
        true
    }
}

async fn normalized_rows_for(
    backend: &MongoBackend,
    tenant: &TenantContext,
    resource_type: &str,
    id: &str,
) -> Vec<String> {
    use futures::stream::TryStreamExt;
    let db = backend.get_database().await.expect("get_database");
    let mut rows: Vec<String> = db
        .collection::<Document>("search_index")
        .find(doc! { "tenant_id": tenant.tenant_id().as_str(), "resource_type": resource_type, "resource_id": id })
        .await
        .expect("find")
        .try_collect::<Vec<Document>>()
        .await
        .expect("collect")
        .into_iter()
        .map(|mut d| {
            d.remove("_id");
            d.remove("tenant_id");
            let mut keys: Vec<&String> = d.keys().collect();
            keys.sort();
            keys.into_iter()
                .map(|k| format!("{k}={:?}", d.get(k)))
                .collect::<Vec<_>>()
                .join(",")
        })
        .collect();
    rows.sort();
    rows
}

async fn poll_until_finished(op: &ReindexOperation, job_id: &str) -> ReindexProgress {
    loop {
        let progress = op.get_progress(job_id).await.expect("job must exist");
        if progress.status.is_finished() {
            return progress;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mongodb_integration_reindex_resource_in_consecutive_pages_ends_with_the_later_version() {
    let Some(backend) = create_backend_with("reindex_prefetch_consecutive_versions", |c| {
        c.reindex_overlap = true;
        c.reindex_prepare_threads = 1;
    })
    .await
    else {
        eprintln!(
            "Skipping mongodb_integration_reindex_resource_in_consecutive_pages_ends_with_the_later_version (requires Docker)"
        );
        return;
    };
    // `create_backend_with` already returns `Option<Arc<MongoBackend>>` (PR2a,
    // S3 §4.5); re-wrapping it here would give `Arc<Arc<MongoBackend>>`, which
    // does not satisfy `S: ReindexableStorage` / `S: ReindexTarget` below.

    let tenant = create_tenant("reindex-prefetch-consecutive-tenant");
    let r_v2 = StoredResource::from_storage(
        "Patient",
        "r",
        "2",
        tenant.tenant_id().clone(),
        json!({"resourceType": "Patient", "id": "r", "name": [{"family": "V2"}]}),
        chrono::Utc::now(),
        chrono::Utc::now(),
        None,
        FhirVersion::default(),
    );
    let r_v1 = StoredResource::from_storage(
        "Patient",
        "r",
        "1",
        tenant.tenant_id().clone(),
        json!({"resourceType": "Patient", "id": "r", "name": [{"family": "V1"}]}),
        chrono::Utc::now(),
        chrono::Utc::now(),
        None,
        FhirVersion::default(),
    );
    let mut page1 = vec![r_v1];
    page1.extend(build_test_patients(&tenant, "others", 40));
    let source = Arc::new(ScriptedResourceSource { resource_type: "Patient", pages: vec![page1, vec![r_v2.clone()]] });

    let op = ReindexOperation::with_parts(
        source,
        vec![backend.clone() as Arc<dyn ReindexTarget>],
        backend.tenant_registries().clone(),
    );
    let job = op
        .start(
            tenant.clone(),
            ReindexRequest::for_types(vec!["Patient".to_string()]).with_batch_size(50),
            None,
        )
        .await
        .expect("start");
    let progress = poll_until_finished(&op, &job).await;
    assert_eq!(progress.status, ReindexStatus::Completed, "{progress:?}");

    // A direct rewrite from v2 alone, into a fresh tenant, is the reference:
    // R's rows in `tenant` must equal it exactly, including no duplicates.
    let fresh_tenant = create_tenant("reindex-prefetch-consecutive-fresh");
    let target: &dyn ReindexTarget = backend.as_ref();
    let mut stats = ReindexPageStats::default();
    let outcome = target
        .write_search_entries_page_timed(&fresh_tenant, std::slice::from_ref(&r_v2), &mut stats)
        .await;
    assert!(outcome[0].is_ok());

    assert_eq!(
        normalized_rows_for(&backend, &tenant, "Patient", "r").await,
        normalized_rows_for(&backend, &fresh_tenant, "Patient", "r").await,
        "R must end with v2's rows and no duplicates"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mongodb_integration_reindex_run_parity_across_knobs() {
    let configs: [(bool, bool, usize); 3] = [(false, false, 1), (true, false, 1), (true, true, 0)];
    let mut backends: Vec<Arc<MongoBackend>> = Vec::new();
    for (overlap, prefetch, threads) in configs {
        let Some(backend) = create_backend_with("reindex_run_parity_across_knobs", move |c| {
            c.reindex_overlap = overlap;
            c.reindex_prefetch = prefetch;
            c.reindex_prepare_threads = threads;
            c.reindex_catch_up_margin_ms = 1_000;
        })
        .await
        else {
            eprintln!("Skipping mongodb_integration_reindex_run_parity_across_knobs (requires Docker)");
            return;
        };
        // `backend` is already `Arc<MongoBackend>` (PR2a's `create_backend_with`
        // returns `Option<Arc<MongoBackend>>`) — push it directly.
        backends.push(backend);
    }

    let tenants: Vec<TenantContext> =
        ["k0", "k1", "k2"].iter().map(|n| create_tenant(n)).collect();
    for (backend, tenant) in backends.iter().zip(&tenants) {
        for n in 0..300 {
            backend
                .create_or_update(
                    tenant,
                    "Observation",
                    &format!("obs-{n}"),
                    json!({"resourceType": "Observation", "id": format!("obs-{n}"), "status": "final"}),
                    FhirVersion::default(),
                )
                .await
                .expect("seed");
        }
    }
    tokio::time::sleep(Duration::from_millis(1_100)).await;

    let mut entries_created = Vec::new();
    for (backend, tenant) in backends.iter().zip(&tenants) {
        let op = ReindexOperation::new(backend.clone(), backend.tenant_registries().clone());
        let job = op
            .start(
                tenant.clone(),
                ReindexRequest::for_types(vec!["Observation".to_string()]).with_batch_size(50),
                None,
            )
            .await
            .expect("start");
        let progress = tokio::time::timeout(Duration::from_secs(60), poll_until_finished(&op, &job))
            .await
            .expect("reindex did not finish within 60s");
        assert_eq!(progress.status, ReindexStatus::Completed, "{progress:?}");
        assert_eq!(progress.error_message, None);
        assert!(progress.errors.is_empty(), "{:?}", progress.errors);
        entries_created.push(progress.entries_created);
    }
    assert_eq!(entries_created[0], entries_created[1], "entries_created must match across knobs");
    assert_eq!(entries_created[1], entries_created[2], "entries_created must match across knobs");

    let rows: Vec<Vec<String>> = {
        let mut v = Vec::new();
        for backend in &backends {
            v.push(rows_without_id_and_tenant(backend, "search_index").await);
        }
        v
    };
    assert_eq!(rows[0], rows[1]);
    assert_eq!(rows[1], rows[2]);
}

/// Wraps a real `MongoBackend` writer and, after each page write returns,
/// checks whether `mongodb reindex id phase finished` is already in the
/// capture buffer — proving whether the transition ran concurrently with, or
/// only after, that page's write (#1403).
struct PhaseLogProbeTarget {
    backend: Arc<MongoBackend>,
    saw_transition_early: Arc<std::sync::atomic::AtomicBool>,
}

#[async_trait::async_trait]
impl ReindexTarget for PhaseLogProbeTarget {
    async fn delete_search_entries(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<u64> {
        self.backend.delete_search_entries(tenant, resource_type, id).await
    }

    async fn write_search_entries(&self, tenant: &TenantContext, resource: &StoredResource) -> StorageResult<usize> {
        self.backend.write_search_entries(tenant, resource).await
    }

    async fn write_search_entries_page(
        &self,
        tenant: &TenantContext,
        resources: &[StoredResource],
    ) -> Vec<StorageResult<usize>> {
        self.backend.write_search_entries_page(tenant, resources).await
    }

    async fn write_search_entries_page_timed(
        &self,
        tenant: &TenantContext,
        resources: &[StoredResource],
        stats: &mut ReindexPageStats,
    ) -> Vec<StorageResult<usize>> {
        let result = self.backend.write_search_entries_page_timed(tenant, resources, stats).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        // PR1's `capture_walk_logs`/`walk_log_lines` install ONE global
        // subscriber and ONE global buffer for the whole `mongodb_tests`
        // binary, so filtering on the message alone would also match every
        // other walk's lines (PR1's own `reindex_id_walk` tests, PR2a's tests,
        // and this test's own parity companion). A tenant needle isolates
        // this run's lines (walk lines log `tenant = %tenant_id`, which
        // `tracing_test` prints unquoted as `tenant=<id>`) (#1403, S3 §5.13).
        let tenant_needle = format!("tenant={}", tenant.tenant_id().as_str());
        if !super::reindex_id_walk::walk_log_lines(&["mongodb reindex id phase finished", &tenant_needle])
            .is_empty()
        {
            self.saw_transition_early.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        result
    }

    async fn clear_search_index(&self, tenant: &TenantContext) -> StorageResult<u64> {
        self.backend.clear_search_index(tenant).await
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mongodb_integration_reindex_prefetch_logs_id_phase_finished_after_the_last_id_page_is_written() {
    let Some(backend) = create_backend_with("reindex_prefetch_logs_id_phase_finished", |c| {
        c.reindex_prefetch = true;
        c.reindex_catch_up_margin_ms = 1_000;
    })
    .await
    else {
        eprintln!(
            "Skipping mongodb_integration_reindex_prefetch_logs_id_phase_finished_after_the_last_id_page_is_written (requires Docker)"
        );
        return;
    };
    // `backend` is already `Arc<MongoBackend>`; do not re-wrap it.
    let tenant = create_tenant("reindex-prefetch-log-order-tenant");
    for n in 0..120 {
        backend
            .create_or_update(
                &tenant,
                "Observation",
                &format!("obs-{n}"),
                json!({"resourceType": "Observation", "id": format!("obs-{n}"), "status": "final"}),
                FhirVersion::default(),
            )
            .await
            .expect("seed");
    }
    tokio::time::sleep(Duration::from_millis(1_100)).await;

    // `capture_walk_logs` returns `()`; binding it (`let _capture = ..`) is a
    // unit value binding, which clippy's `let_unit_value` rejects under
    // `-D warnings`. Call it as a plain statement instead.
    super::reindex_id_walk::capture_walk_logs();
    let saw_transition_early = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let probe = Arc::new(PhaseLogProbeTarget { backend: backend.clone(), saw_transition_early: saw_transition_early.clone() });
    let op = ReindexOperation::with_parts(
        backend.clone(),
        vec![probe as Arc<dyn ReindexTarget>],
        backend.tenant_registries().clone(),
    );
    let job = op
        .start(
            tenant.clone(),
            ReindexRequest::for_types(vec!["Observation".to_string()]).with_batch_size(50),
            None,
        )
        .await
        .expect("start");
    let progress = poll_until_finished(&op, &job).await;
    assert_eq!(progress.status, ReindexStatus::Completed, "{progress:?}");
    assert!(
        !saw_transition_early.load(std::sync::atomic::Ordering::SeqCst),
        "id phase finished must never be logged concurrently with an id-page write"
    );

    let tenant_needle = format!("tenant={}", tenant.tenant_id().as_str());
    assert_eq!(
        super::reindex_id_walk::walk_log_lines(&["mongodb reindex id phase finished", &tenant_needle]).len(),
        1
    );
    assert_eq!(
        super::reindex_id_walk::walk_log_lines(&["mongodb reindex catch-up round started", &tenant_needle]).len(),
        1
    );
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```
cargo test -p helios-persistence --features mongodb --test mongodb_tests reindex_resource_in_consecutive_pages 2>&1 | tail -60
cargo test -p helios-persistence --features mongodb --test mongodb_tests reindex_run_parity_across_knobs 2>&1 | tail -60
cargo test -p helios-persistence --features mongodb --test mongodb_tests reindex_prefetch_logs_id_phase_finished 2>&1 | tail -60
```

Expected failure: `walk_log_lines`/`capture_walk_logs` are private to `reindex_id_walk`'s module (compile error) until Step 3's visibility change lands; with Docker running, the parity and consecutive-pages tests fail behaviorally if any earlier task's invariant (I1, D13) is broken.

- [ ] **Step 3: Implement**

Apply the `reindex_id_walk.rs` visibility change described above. Add `use async_trait::async_trait;` (or the crate's existing alias), `use std::sync::Arc;`, `use std::time::Duration;` and `use helios_persistence::search::ReindexProgress;` to `reindex_pipeline.rs`'s test file if not already imported by Task 7a/7b/9's additions (`ReindexOperation, ReindexPageStats, ReindexRequest, ReindexSource, ReindexStatus, ReindexTarget, ResourcePage` are already in scope, cumulatively, from PR2a's Task 1b and Task 2 imports at this file's top — `ReindexProgress` is the one name this task's `poll_until_finished` needs that none of them added).

- [ ] **Step 4: Run the tests to verify they pass**

```
cargo test -p helios-persistence --features mongodb --test mongodb_tests reindex 2>&1 | tail -200
```

Expect `test result: ok` for the whole reindex integration suite, Tasks 7, 9 and 10's tests included.

- [ ] **Step 5: Run the whole MongoDB suite once, then the full regression list**

```
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
cargo test -p helios-persistence --features mongodb --test mongodb_tests 2>&1 | tail -60
cargo test -p helios-persistence --lib search::reindex 2>&1 | tail -60
cargo test -p helios-persistence --lib perf 2>&1 | tail -30
cargo test -p helios-persistence --features mongodb --lib reindex_pipeline 2>&1 | tail -60
cargo test -p helios-persistence --features mongodb --lib backends::mongodb 2>&1 | tail -100
cargo test -p helios-hfs --features mongodb build_mongodb_config 2>&1 | tail -40
cargo test -p helios-rest reindex 2>&1 | tail -40
cargo test -p helios-rest config 2>&1 | tail -40
cargo test -p helios-persistence --features sqlite --lib search::reindex 2>&1 | tail -40
cargo test -p helios-persistence --features postgres --test postgres_tests reindex 2>&1 | tail -60
```

The last two (SQLite and PostgreSQL reindex suites) must be completely unchanged by this plan — confirm they show the same pass count as on `origin/main` before this branch.

- [ ] **Step 6: fmt, clippy, commit**

```
rustfmt --edition 2024 crates/persistence/tests/mongodb/reindex_id_walk.rs crates/persistence/tests/mongodb/reindex_pipeline.rs > /dev/null 2>&1
cargo clippy -p helios-persistence --all-targets --all-features -- -D warnings -A clippy::items_after_test_module -A clippy::large_enum_variant -A clippy::question_mark -A clippy::collapsible_match -A clippy::collapsible_if -A clippy::field_reassign_with_default -A clippy::doc-overindented-list-items -A clippy::doc-lazy-continuation 2>&1 | tail -40
```

```
git add crates/persistence/tests/mongodb/reindex_id_walk.rs crates/persistence/tests/mongodb/reindex_pipeline.rs
git commit -m "$(cat <<'EOF'
test(mongodb): reindex prefetch consecutive-version, knob-parity, and log-order integration tests (#1403)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 11: Push and open the PR (do not merge)

**Files:** none — verification and `gh` commands only.

**Interfaces:** none.

- [ ] **Step 1: Confirm nothing is outstanding**

```bash
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
git status --short crates/persistence crates/hfs README.md
```
Expected: `git status --short` reports nothing outstanding (everything from Tasks 1–10 is already committed).

- [ ] **Step 2: Open the PR**

```bash
git push -u origin perf/1403-pr2b-mongodb-reindex-overlap
gh pr create --base main --head perf/1403-pr2b-mongodb-reindex-overlap \
  --title "perf(mongodb): overlap and parallelise \$reindex page preparation (#1403 PR2b)" \
  --body "$(cat <<'EOF'
## Summary
- Adds a sub-batch extraction/insert pipeline, a per-backend rayon pool, and
  a driver-side id-phase prefetch, so a MongoDB `$reindex` page overlaps
  HFS-side extraction with MongoDB-side delete/insert instead of running the
  four stages (fetch, extract, delete, insert) strictly in sequence.
- Three new knobs (`HFS_MONGODB_REINDEX_OVERLAP`, `_PREPARE_THREADS`,
  `_PREFETCH`), all default-on except an explicit thread count; `false`/`0`
  restores today's serial, unprefetched behaviour exactly (D11: every new
  `ReindexPageStats`/`PageRecord`/`Counters`/`PhaseMillis` field reproduces
  PR0's values whenever nothing is prefetched and no writer measures
  `db_wait`).
- Invariant I1 (at most one page's delete/insert in flight per walk,
  committed in fetch order) holds throughout, which is what keeps this safe
  alongside PR1's revisits.

## Design
- Spec: [`docs/superpowers/specs/2026-09-23-mongodb-reindex-rebuild-design.md`](../blob/main/docs/superpowers/specs/2026-09-23-mongodb-reindex-rebuild-design.md) (§4.4)
- File-level design: `manual-test/archive/1403-run17-evidence/design/S3-pr2-overlap.md` §5 — local evidence only, not committed to this repo; SHA-256 recorded in the plan this PR implements, `docs/superpowers/plans/2026-09-23-1403-pr2b-mongodb-reindex-overlap.md`.

## Gate (PR2b + CU-2b)
Pending — this PR's merge gate is Gate PR2b plus CU-2b (spec §4.7), run by the
orchestrator on this branch. Their results tables (B2-off/B2-s/CU-2b, plus
B2-s-P1-nopf/B2-s-P1 if they ran) are pasted here with `gh pr edit` once that
run completes, and the same rows are appended to
`docs/mongodb-reindex-benchmark.md` §8.2–§8.4 in a follow-up docs commit —
neither is part of this PR's test plan below, which the implementer runs
directly.

## Test plan
- [x] `cargo test -p helios-persistence --lib -- search::reindex_stats search::reindex perf`
- [x] `cargo test -p helios-persistence --features mongodb --test mongodb_tests -- reindex` (Docker; verified non-skipped, see Task 10)
- [x] `cargo test -p helios-hfs --features mongodb build_mongodb_config`
- [x] `cargo clippy -p helios-persistence --all-targets --all-features -- -D warnings` (repo's standard `-A` allowances)
- [x] SQLite and PostgreSQL reindex suites unchanged (Task 10 Step 5)

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

Do not merge until Gate PR2b and CU-2b pass (spec §4.7). Once they do, the B2-off/B2-s/CU-2b rows (and B2-s-P1-nopf/B2-s-P1, if the diagnostic pair ran) are appended to `docs/mongodb-reindex-benchmark.md` §8.2–§8.4, using that document's exact column headers — that append is the docs plan's follow-up commit, not a step of this plan.

---

## Self-review

**Spec coverage.**

- **Spec §4.4 / S3 §5.3 (recommended design overview, dispatcher routing rule):** Task 7a (temporary serial-only dispatcher) and Task 7b (`write_search_entries_page_timed`'s final dispatcher: empty/offloaded/`get_database` guards, `overlapped` decision, `log_reindex_mode_once`, route to `write_page_overlapped`/`write_page_serial`).
- **S3 §5.4 (dispatcher and overlapped writer, its numbered steps 1–4, `write_page_serial`'s 5 steps):** Task 7a (`write_page_serial`'s 5 steps) and Task 7b (`write_page_overlapped`'s steps 1–4), code-for-code against the design's listing; Invariant I1 called out explicitly in Task 7b's header for reviewer attention.
- **S3 §5.5 (sub-batch planner, constants, `size_for`/`next_range`/`record`, "why these sizes"):** Task 4, with every one of the six listed unit tests (`planner_first_range_is_first_without_a_seed`, `planner_sizes_from_observed_docs`, `planner_uses_the_seed_for_its_first_range`, `planner_clamps`'s four sub-cases, `planner_covers_every_index_exactly_once_and_in_order`, `resolve_prepare_width_auto_explicit_and_capped`).
- **S3 §5.6 (`block_in_place`, pool, gate, `PrepareEnv`/`extract_range`, `resolve_prepare_width`, `tokio_multi_thread_runtime`, D6):** Task 4 for the pure mechanics and its two `extract_range` tests; Task 5 for the per-backend pool/gate/seed-map fields and accessors that `write_page_serial` (Task 7a) and `write_page_overlapped` (Task 7b) call.
- **S3 §5.7 (error semantics table, case by case):** Task 7a's `write_page_serial` and Task 7b's `write_page_overlapped` bodies reproduce every row (fan-out on `get_database`/offloaded/delete/insert failure, `join_failure`'s panic resume, `AbortOnDrop`'s abort-on-drop); Task 7b's three failpoint integration tests (`issues_no_insert_after_a_failed_one`, `keeps_rows_of_completed_sub_batches`, `fans_out_a_delete_failure`) pin the "rows left after a page-level insert error" bullets exactly, each now seeded with the failpoint off first so the "rows removed"/"rows kept" claims are checked against real prior rows, not an already-empty collection.
- **S3 §5.8 (memory bound):** a design property of Tasks 4/7b/8 (sub-batch sizes bounded by `REINDEX_SUBBATCH_MAX`, at most one page prefetched and only in the id phase), not a separately testable unit — no dedicated test claims to cover it, matching that this section is descriptive, not prescriptive of new behaviour.
- **S3 §5.9 (knobs, config plumbing, mode line, docs):** Task 5 (three `MongoBackendConfig` fields, `apply_reindex_env`, `from_env`'s doc list and call, `build_mongodb_config_with_env` call site, README rows) and Task 7a (`log_reindex_mode_once`, the exact field list and `pool` value logic).
- **S3 §5.10 (driver prefetch, both defaulted methods, `PrefetchedPage`, the paging-loop rewrite, D13):** Task 8 (backend-agnostic defaults, type, loop rewrite — now written out in full, no elided lines — 8 driver-fake tests) and Task 9 (MongoDB's `may_prefetch_page`/`fetch_resources_page_ahead`/`reindex_id_page`, its 2 no-server tests).
- **S3 §5.11 (Invariants and interplay — I1, PR1 revisits, prefetch healing, cancellation, progress, composite deployments, manual `$reindex`, "what does not change"):** I1 is structural in Task 7b's `write_page_overlapped` (called out explicitly); the PR1-revisit healing claim is Task 10's `mongodb_integration_reindex_resource_in_consecutive_pages_ends_with_the_later_version`; cancellation-with-prefetch is Task 8's `cancellation_with_prefetch_writes_nothing_after_the_page_in_flight`, which now polls `cancel_channels` rather than treating `cancel()`'s synchronous status write as proof the task stopped; composite deployments (search-offloaded ⇒ `may_prefetch_page` false, writer returns `Ok(0)`) fall out of Task 9's `may_prefetch_page` guard and Task 7a/7b's existing offloaded guard, unchanged.
- **S3 §5.12 (instrumentation, D11's field meanings and formulas, where documented):** Task 2 (`ReindexPageStats.db_wait`/`sub_batches`/`pool_sub_batches`, `db_wait_or_busy`, `accumulate`, and PR0's pre-existing exhaustive-literal test fixed to compile), Task 3 (`PageRecord`/`Counters`/`PhaseMillis` fields, the two changed formulas, the four `log_*` helpers' appended fields in §3's exact per-line order and full replacement doc comments, the field-order test, the SKILL.md doc updates).
- **S3 §5.13 (tests, by sub-bullet):**
  - Unit tests in `reindex_pipeline.rs` (no server): planner tests and `resolve_prepare_width_*` (Task 4); `extract_range_*` (Task 4); `page_outcomes_keeps_heads_precedence`, `merge_insert_failures_prefers_own_over_contained`, `absorb_db_task_adds_only_the_db_fields`, `add_db_wait_turns_none_into_some`, `abort_on_drop_aborts_a_pending_task`, `join_failure_resumes_a_panic`, `flatten_moves_documents_with_page_global_owners` (Task 6).
  - Unit tests in `backend.rs`: `config_reindex_pipeline_defaults`, `apply_reindex_env_reads_and_rejects` (Task 5).
  - Unit test in `storage.rs`, `reindex_prefetch_tests`: `may_prefetch_page_accepts_only_id_cursors`, `fetch_ahead_declines_round_and_malformed_cursors` (Task 9, both now built on a config that can never reach a real server).
  - Unit tests in `reindex.rs`, the `ReindexPageStats` rules: `accumulate_merges_db_wait_as_effective_waits_before_adding_delete_and_insert` (now covering both the `Some/None` and `None/Some` pairings), `db_wait_or_busy_falls_back_to_delete_plus_insert` (Task 2).
  - Unit tests in `reindex.rs`, the driver fakes: all 8 `PrefetchingSource`/`PrefetchingWriter` tests plus the "existing tests unmodified" regression check (Task 8); three of the eight now genuinely prove concurrency/ordering/cancellation rather than passing vacuously (see Placeholder/soundness notes below).
  - Unit tests in `reindex_stats.rs`: `record_page_accumulates_fetch_wait_and_db_wait`, `phase_millis_uses_the_critical_path`, `unmeasured_db_wait_reproduces_pr0s_writer_other`, `unprefetched_pages_reproduce_pr0s_other`, the field-order extension including "L4 does not carry `sub_batches`" (Task 3).
  - Integration tests in `tests/mongodb/reindex_pipeline.rs`: `mongodb_integration_reindex_page_serial_pipeline_writes_a_large_page` (Task 7a); `mongodb_integration_reindex_page_overlapped_matches_serial`, `_command_shape`, `_issues_no_insert_after_a_failed_one`, `_keeps_rows_of_completed_sub_batches`, `_fans_out_a_delete_failure` (Task 7b); `mongodb_integration_reindex_resource_in_consecutive_pages_ends_with_the_later_version`, `mongodb_integration_reindex_run_parity_across_knobs`, `mongodb_integration_reindex_prefetch_logs_id_phase_finished_after_the_last_id_page_is_written` (Task 10).
  - `crates/hfs/src/main.rs` knob test: Task 5.
  - Regression runs: all listed verbatim in Task 10 Step 5, plus each task's own narrower `cargo test` in its Step 4.
- **S3 §5.14 (OBS-26 measurement, arm names, prediction formula):** explicitly out of this plan's scope — it is bench tooling (`manual-test/tools/bench-1403/`, "written by Sonnet agents" per spec §5.7) run by the orchestrator against the shipped binary, not application code; nothing here claims to implement it.
- **Spec §4.4's own summary line (sub-batch pipeline, per-backend rayon pool, the two defaulted `ReindexSource` methods, I1, the three knobs, the writer configuration INFO line, the perf phases and `PHASE_COUNT` 38, `ReindexPageStats` additions and the log field/formula changes):** every clause maps to a task above; none is unaddressed.

**Placeholder scan.** No `TBD`, `TODO`, "add error handling", "similar to Task N", or "write tests for the above" anywhere in this plan. Every checklist step carries the code to write, not a description of it, including Task 8's paging-loop snippet, which this revision writes out in full (the skipped-rows loop, `write_resource_batch` call, `record_and_log_page`/`PageRecord` literal and the closing `finish_type` block are all shown verbatim, matching PR0's paging-path body) rather than eliding them behind a comment. Two spots name a cross-reference by design rather than inventing new behaviour, and both give concrete, runnable code alongside the note, and both claims below are now literally true (they were not, in the version this revision replaces — see the corrections after each):
- Task 9's `reindex_id_page` reuses PR1's own `reindex_page_from_docs(docs: &[Document], resource_type: &str, tenant: &TenantContext, next_cursor: ReindexWalkCursor) -> StorageResult<ResourcePage>` (S2 §7.4) instead of reimplementing the conversion inline. **Correction from the prior revision:** that earlier version called a hand-rolled `parse_history_row(doc, Some(resource_type), None).into_stored_resource(tenant)` chain directly on an owned `Document`, which does not compile (`parse_history_row` takes `&Document` and returns `StorageResult<ParsedHistoryRow>`, not a type with an `.into_stored_resource` method) — the description of it as "PR1's own documented call shape" was false; it is true now that the code calls `reindex_page_from_docs` itself.
- Task 10's `PhaseLogProbeTarget` calls `walk_log_lines(&["mongodb reindex id phase finished", &tenant_needle])`/`capture_walk_logs()`, PR1's real, already-defined signatures (`fn capture_walk_logs()`, `fn walk_log_lines(needles: &[&str]) -> Vec<String>`), confirmed against `docs/superpowers/plans/2026-09-23-1403-pr1-mongodb-id-walk.md`. **Correction from the prior revision:** it called them with zero arguments and bound `capture_walk_logs()`'s `()` result to `let _capture = ..`, neither of which matches PR1's actual signatures or compiles under `-D warnings` (`clippy::let_unit_value`); the claim that "the call sites are real, not stubbed" was false. It is true now, and the tenant needle is required for correctness, not just style: PR1's global tracing buffer is shared by every walk test in the binary, so a needle-free filter on the message alone would also match other tests' lines.

**Type consistency.**
- `ReindexPageStats` (Task 2: `db_wait: Option<Duration>`, `sub_batches: u64`, `pool_sub_batches: u64`, `db_wait_or_busy(&self) -> Duration`) is the exact type `reindex_stats.rs`'s `Counters`/`PageRecord` embed (Task 3), that `reindex_pipeline.rs`'s `DbTask<T>`/`absorb_db_task`/`add_db_wait` operate on (Task 6), that `write_page_serial` (Task 7a) and `write_page_overlapped` (Task 7b) mutate as their `stats: &mut ReindexPageStats` parameter (matching `ReindexTarget::write_search_entries_page_timed`'s existing PR0 signature unchanged), and that Task 9's `reindex_prefetch_tests` never touches directly (it only calls `may_prefetch_page`/`fetch_resources_page_ahead`, which take no stats).
- `SubBatchPlanner::new(len: usize, seed: Option<(u64, u64)>, min_size: usize)` and `next_range(&mut self) -> Option<Range<usize>>` (Task 4) are called with exactly that signature, with `seed` sourced from `self.reindex_docs_seed(t)` (Task 5's accessor, returning the same `Option<(u64, u64)>`), inside `write_page_overlapped` (Task 7b) — `write_page_serial` (Task 7a) does not use `SubBatchPlanner` at all (it runs one extraction pass, not a sub-batch loop), which is why Task 4's `SubBatchPlanner`/`REINDEX_SUBBATCH_FIRST`/`_TARGET_DOCS`/`_MAX` stay behind the `#![allow(dead_code)]` through Task 7a and are only exercised, and the allow removed, in Task 7b.
- `PrepareEnv<'a> { pool: Option<&'a rayon::ThreadPool>, gate: &'a tokio::sync::Semaphore }` and `extract_range<T, F>(env: &PrepareEnv<'_>, range: Range<usize>, prepare: &F) -> (Vec<T>, bool)` (Task 4) are constructed and called identically in both `write_page_serial` (Task 7a) and `write_page_overlapped` (Task 7b), with `pool` sourced from `self.reindex_prepare_pool()` and `gate` from `self.reindex_prepare_gate()` (Task 5).
- `SubBatchDocs`/`DbTask<T>`/`InsertFailures` (Task 6) are produced by `flatten_sub_batch`/`delete_page`/`insert_sub_batch` and consumed by `absorb_db_task`/`merge_insert_failures`/`page_outcomes` (also Task 6), and `delete_page`/`insert_sub_batch` themselves are defined in Task 7a (both are called by `write_page_serial` there, and again by `write_page_overlapped` in Task 7b) using exactly those Task 6 types — no task redefines them differently.
- `MongoBackendConfig.reindex_overlap: bool`, `.reindex_prepare_threads: usize`, `.reindex_prefetch: bool` (Task 5) are read, with the same names and types, by Task 7b's dispatcher and by Task 7a's `log_reindex_mode_once`, and by Task 9's `may_prefetch_page`; `create_backend_with`'s `configure` closure (PR2a, unmodified) sets all three identically across every Task 7a/7b/9/10 test that needs a non-default value.
- `ReindexSource::may_prefetch_page(&self, cursor: &str) -> bool` and `async fn fetch_resources_page_ahead(&self, tenant: &TenantContext, resource_type: &str, cursor: &str, limit: u32, max_bytes: u64) -> StorageResult<Option<ResourcePage>>` (Task 8's trait defaults) are overridden by Task 9's `impl ReindexSource for MongoBackend` with an identical signature, and by Task 8's own `PrefetchingSource` test fake with the identical signature.
- `ReindexTarget::clear_search_index(&self, tenant: &TenantContext) -> StorageResult<u64>` (the one-argument form the trait actually declares at HEAD, and PR0/PR1/PR2a leave unchanged) is the signature every fake and wrapper in this plan implements: Task 8's `PrefetchingWriter`, Task 10's `PhaseLogProbeTarget`. Neither takes a `resource_type: Option<&str>` second argument.
- `PageRecord.fetch_wait: Duration` (Task 3) is populated as `fetch_wait: fetch_time` at both call sites when Task 3 lands (nothing prefetches yet), and Task 8 changes only the paging branch's binding to the `fetch_wait` value the loop's `(fetched, fetch, fetch_wait)` tuple computes — the named-resources branch's binding is untouched, matching S3 §5.10's explicit rule that path never prefetches.
- `create_backend_with(test_name: &str, configure: impl FnOnce(&mut MongoBackendConfig)) -> Option<Arc<MongoBackend>>` (PR2a, confirmed against `docs/superpowers/plans/2026-09-23-1403-pr2a-mongodb-batch-bytes.md`) already returns an `Arc`; every test in Tasks 7a, 7b and 10 binds its result directly (`let Some(backend) = create_backend_with(..).await else { .. };`) and never re-wraps it in a second `Arc::new(..)`, so `backends.push(backend)` and `backend.clone() as Arc<dyn ReindexTarget>` type-check against `S: ReindexSource + ReindexTarget = MongoBackend`, not `Arc<MongoBackend>`.
- `Phase::ReindexDbWait`/`Phase::ReindexFetchWait` (Task 1) are the exact variants `crate::perf::record_duration` is called with in Tasks 7a/7b (`ReindexDbWait`, both writer bodies) and Task 8 (`ReindexFetchWait`, inside `PrefetchedPage::wait`); `Phase::ALL`'s length (38) and `PHASE_COUNT` are updated together in Task 1 and never touched again.
