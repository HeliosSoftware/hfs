# MongoDB Reindex Rebuild PR0 (Instrumentation) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add always-on, per-type and per-page wall-clock instrumentation to the reindex driver and MongoDB's writer, emitting a fixed six-line log contract, with no behaviour change to what is written, in what order, or at what page size.

**Architecture:** `run_reindex` gains a local `ReindexRunStats` accumulator (new pure-accounting module `reindex_stats.rs`) that tracks job- and type-scoped counters from synthetic `Instant`s and turns them into six `tracing` lines (`reindex job started/type started/type finished/progress/job finished/page`) at fixed field order. A new provided trait method `ReindexTarget::write_search_entries_page_timed` lets a writer report its own phase durations through an out-parameter (`ReindexPageStats`); every implementor except MongoDB inherits the default, which delegates and measures nothing. MongoDB's existing `write_search_entries_page` is renamed to `_timed`, instrumented phase-by-phase, and a new thin `write_search_entries_page` delegates to it so the driver and the composite ingest sink's direct call can never diverge.

**Tech Stack:** Rust 2024, `tokio`, `tracing 0.1.44` (module-target `helios_persistence::search::reindex`), `mongodb` driver 3.x, `parking_lot::RwLock`. Tests: `cargo test -p helios-persistence --lib` (pure unit tests, no I/O), `cargo test -p helios-persistence --features mongodb --test mongodb_tests` (testcontainers; skips silently without Docker or `HFS_TEST_MONGODB_URL`).

**Spec:** `docs/superpowers/specs/2026-09-23-mongodb-reindex-rebuild-design.md` §4.1 and §9; file-level design `manual-test/archive/1403-run17-evidence/design/S1-pr0-instrumentation.md` (SHA-256 `96f17650bdcc4d50e33049a53b08b238ad154614723d40aae701472a9e92b493`, verified against the file on disk before this plan was written).

## Global Constraints

- **No cargo while a bench arm is running (S4 §4.5.1, §4.12.3 "Orchestrator rule: no cargo during arms").** Before every cargo/test/build command anywhere in this plan (every Step 2/4/5 of Tasks 1–4, and Step 1/2 of Task 6), run:
  ```bash
  test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
  ```
  If it prints and exits, stop immediately and report back instead of running `cargo`, `rustc`, a test or a build — a `SMOKE-0`/`SMOKE-1` or later arm may be mid-measurement, and running any of these can invalidate that arm or the host-quiet precondition its validity checks assume. Task 6's Steps 1 and 2 show this line inline, since they are bundled with other setup; Tasks 1–4's Steps 2/4/5 do not repeat it inline, but it applies there identically — run it, unabbreviated, before each of those command blocks too.
- **No behaviour change.** What is written, in what order, at what page size, per-resource error attribution, retries, `$reindex-status`'s output, `ReindexProgress`, index specs and `SCHEMA_VERSION` are all unchanged (S1 §1). Every existing test must keep passing unmodified — the default `_timed` delegates, so no existing `ReindexTarget` double needs to change.
- **Always-on timing is the record.** `std::time::Instant` timing runs unconditionally inside `run_reindex`; the existing `cfg(perf_phases)` machinery and the driver's `ReindexFetch` span are untouched. MongoDB additionally forwards to `crate::perf::record_duration`/`add_rows` and opens a `ReindexPage` span — these compile to nothing (or a stubbed no-op) outside a `--cfg perf_phases` build (S1 D1).
- **Writer-phase timing is MongoDB-only in this PR.** PostgreSQL, SQLite and Elasticsearch inherit the trait's default `_timed` and report zero writer phases; that is correct for PR0 (S1 D8) — do not add overrides for them.
- **The delegation invariant is load-bearing and tested.** MongoDB's `write_search_entries_page` MUST delegate to `write_search_entries_page_timed` with a throwaway `ReindexPageStats`, never the reverse and never two independent bodies — the composite ingest sink (`ingest_index_sink.rs:890`) calls the untimed method directly and must see exactly what the driver sees. Task 4's test calls only through `&dyn ReindexTarget` to prove dynamic dispatch reaches the override, not the trait's zero-measuring default (S1 §10 "default-method trap").
- **The six log lines are a fixed contract.** Messages are exactly `reindex job started` / `reindex type started` / `reindex type finished` / `reindex progress` / `reindex job finished` / `reindex page`; field order within a line is fixed and pinned by tests; strings print unquoted via `%`; `*_ms` fields are whole milliseconds truncated (never rounded) with the subtraction done on `Duration`s *before* truncation; `*_per_s` fields are rounded to one decimal, `0.0` when nothing elapsed; no logged value is ever empty (`resource_type` is the sentinel `-` when no type is open). Fields may be appended later, never renamed, removed or reordered. Soft budget: 32 fields per line including `message` (L4 is 29 in this PR) — there is no hard limit in the workspace's locked `tracing-core 0.1.36`, but keep the budget anyway (S1 D14).
- **Target and env.** All six lines use the default module target `helios_persistence::search::reindex` (an `hfs_*` target would be silenced by the benchmark harnesses' `hfs=warn`). The five INFO lines need no environment variable; the DEBUG `reindex page` line needs `RUST_LOG=...,helios_persistence::search::reindex=debug`.
- **`ReindexPageStats` is `#[non_exhaustive]`, `Copy + Default`, and every field/method added must be read somewhere** — `cargo clippy -D warnings` rejects dead code.
- **TDD per task**, in this order: write the failing test with real assertions (never a stub), run it and confirm the *expected* failure (compile error or a named assertion), implement, run green, then the crate-wide checks below, then commit with explicit paths.
- **Compile-wide checks**, run at least once per task before its final commit:
  ```
  cargo check -p helios-persistence --all-features --tests
  cargo check -p helios-hfs
  ```
- **The crate's real CI clippy gate** (`.github/workflows/ci.yml:558`, scoped to this package) — run before every task's final commit:
  ```
  cargo clippy -p helios-persistence --all-features --tests -- -D warnings -A clippy::items_after_test_module -A clippy::large_enum_variant -A clippy::question_mark -A clippy::collapsible_match -A clippy::collapsible_if -A clippy::field_reassign_with_default -A clippy::doc-overindented-list-items -A clippy::doc-lazy-continuation
  ```
- **Format only the files you touched**: `rustfmt --edition 2024 <explicit paths>`. Never `cargo fmt --all` — it fails from this worktree and a build dirties ~3,500 generated R6 spec files. `rustfmt` on `search/mod.rs` also reformats every out-of-line submodule `mod.rs` declares; if `git status --short crates/persistence` lists a file you did not intend to touch, `git checkout -- <path>` it before committing.
- **Stage explicit paths only** — `git add <path> <path> ...`, never `-A` or `-a`, and never `git commit -a`.
- **MongoDB integration tests need Docker + testcontainers**: `cargo test -p helios-persistence --features mongodb --test mongodb_tests <filter>`. They skip silently (`eprintln!` + `return`) without Docker or `HFS_TEST_MONGODB_URL`. libtest captures `eprintln!` output from a *passing* test, so a skip's `eprintln!` line is invisible in the normal summary and the console still shows `test result: ok. 1 passed` — the same line a real pass shows. A skip is therefore never treated as a pass: run with `--nocapture`, save the output to a file, and `grep -q '^Skipping'` it — a match means the step is unverified (report it and get Docker running before treating the task as verified), not that it is green. Before running one, also confirm Docker itself is reachable: `docker info >/dev/null 2>&1 || echo "Docker not reachable"`.
- **Windows Git Bash**: redirect genuinely-discarded output to `/dev/null`, never `nul` (e.g. `docker info >/dev/null 2>&1`). Never redirect a `cargo` command's stderr to `/dev/null` — that is where compile errors and every clippy diagnostic print, and this plan's "expected" text for each step depends on reading them. If a command's output is too long to read comfortably, pipe it instead of discarding it, e.g. `2>&1 | tail -n 100` or, for clippy, `2>&1 | grep -E '^(warning|error)' -A 6 | head -n 120`. Set `CARGO_BUILD_JOBS=4` if memory is tight; if `target/` fills the disk, delete `target/debug/incremental` first.
- **Branch**: stay on `perf/1403-mongodb-reindex-rebuild` (already checked out; already `origin/main` `c86d0f08b` plus the spec commit, no source files touched yet — verified by `git diff --stat c86d0f08b HEAD` showing only the spec markdown). Do not rebase, do not push except in the final task, do not merge.
- **Commit messages** end with a blank line then exactly:
  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  ```

---

## File Structure

| File | Responsibility |
|---|---|
| Modify `crates/persistence/src/search/reindex.rs` | `ReindexPageStats` + `write_search_entries_page_timed` provided trait method (Task 1); `REINDEX_PROGRESS_INTERVAL`, `ReindexOperation.progress_interval` + test seam, `BatchOutcome`, full `run_reindex` instrumentation, private `log_*`/`exit_outcome`/`job_outcome`/`log_job_end`/`record_and_log_page` helpers, the capture-contract test harness and driver-log tests (Task 3). |
| Create, then modify, `crates/persistence/src/search/reindex_stats.rs` | Pure wall-clock accounting, no tracing/I-O: `OUTCOME_*`/`NO_TYPE` consts, `millis`/`per_second`, `PageRecord`, `Counters`, `PhaseMillis`, `TypeStarted`, `RecordedPage`, `ProgressSnapshot`, `TypeSummary`, `JobSummary`, `ReindexRunStats`, behind a temporary `#![allow(dead_code)]` (Task 2); that `allow` is deleted once `run_reindex` reads every item (Task 3). |
| Modify `crates/persistence/src/search/mod.rs` | Declare `mod reindex_stats;`; export `ReindexPageStats` (Task 2). |
| Modify `crates/persistence/src/backends/mongodb/storage.rs` | Rename `write_search_entries_page` → `write_search_entries_page_timed` (+ `stats` out-parameter, phase timing); new thin delegate `write_search_entries_page`; `insert_search_entries_chunk` gains a `stats` parameter (Task 4). |
| Modify `crates/persistence/src/perf.rs` | Doc-comment-only updates: `ReindexExtract`/`ReindexSearchDelete`/`ReindexSearchInsert` now also fire for MongoDB (Task 4). |
| Modify `crates/persistence/tests/mongodb_tests.rs` | New integration test `mongodb_integration_reindex_page_reports_phase_stats`, called only through `&dyn ReindexTarget` (Task 4). |
| Modify `.claude/skills/bulk-data-submit/SKILL.md` | One bullet documenting the six log lines (Task 5). |
| Modify `.agents/skills/bulk-data-submit/SKILL.md` | The same bullet, mirrored per the repo's cross-copy rule (Task 5). |

---

### Task 1: `ReindexPageStats` and the provided `write_search_entries_page_timed` method

**Files:**
- Modify: `crates/persistence/src/search/reindex.rs` — insert a new struct after `SkippedResource` closes (at HEAD `c86d0f08b`, that's the `}` on line 118; line 123 is `pub struct ResourceRef {`'s own opening line), just before the `ResourceRef` doc comment; insert a new trait method inside `#[async_trait] pub trait ReindexTarget { ... }` (the block spanning lines 247–324), directly after `write_search_entries_page` closes (its own closing `}` is line 323; the trait's closing `}` is line 324).
- Test: `crates/persistence/src/search/reindex.rs`, existing `mod tests` block (starts at line 2456).

**Interfaces:**
- Produces:
  - `#[non_exhaustive] #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)] pub struct ReindexPageStats { pub extract: Duration, pub delete: Duration, pub insert: Duration, pub deleted_entries: u64, pub inserted_entries: u64, pub insert_commands: u64 }` with `pub fn accumulate(&mut self, other: &ReindexPageStats)`.
  - `async fn write_search_entries_page_timed(&self, tenant: &TenantContext, resources: &[StoredResource], stats: &mut ReindexPageStats) -> Vec<StorageResult<usize>>` — provided method on `ReindexTarget`, default delegates to `write_search_entries_page` and leaves `stats` untouched.
- Consumes: nothing new (`Duration`, `HashMap` already imported at the top of `reindex.rs`).

- [ ] **Step 1: Write the failing tests**

Add these two tests inside the existing `mod tests { ... }` block in `crates/persistence/src/search/reindex.rs` (put them near the top of the module, e.g. directly before the `struct ControlledBackend` block):

```rust
#[test]
fn reindex_page_stats_accumulate_adds_every_field() {
    let mut total = ReindexPageStats {
        extract: Duration::from_millis(1),
        delete: Duration::from_millis(2),
        insert: Duration::from_millis(3),
        deleted_entries: 4,
        inserted_entries: 5,
        insert_commands: 6,
    };
    let other = ReindexPageStats {
        extract: Duration::from_millis(10),
        delete: Duration::from_millis(20),
        insert: Duration::from_millis(30),
        deleted_entries: 40,
        inserted_entries: 50,
        insert_commands: 60,
    };
    total.accumulate(&other);
    assert_eq!(total.extract, Duration::from_millis(11));
    assert_eq!(total.delete, Duration::from_millis(22));
    assert_eq!(total.insert, Duration::from_millis(33));
    assert_eq!(total.deleted_entries, 44);
    assert_eq!(total.inserted_entries, 55);
    assert_eq!(total.insert_commands, 66);
}

/// The default `_timed` must behave exactly like calling
/// `write_search_entries_page` directly: same outcomes, same side effects,
/// and it must leave `stats` untouched — proving the plumbing before any
/// writer (MongoDB, in Task 4) overrides it (#1403).
#[tokio::test]
async fn default_timed_page_write_delegates_and_measures_nothing() {
    // Two fresh targets: `RecordingTarget` is stateful (it records what it
    // wrote), so the untimed and timed calls each need their own instance to
    // compare like for like.
    let first = RecordingTarget {
        permanent: BTreeSet::from(["p1".to_string()]),
        ..Default::default()
    };
    let second = RecordingTarget {
        permanent: BTreeSet::from(["p1".to_string()]),
        ..Default::default()
    };
    let tenant = named_tenant("default-timed-delegates");
    let resources: Vec<StoredResource> = ["p0", "p1", "p2"]
        .iter()
        .map(|id| {
            StoredResource::new(
                "Patient",
                id,
                tenant.tenant_id().clone(),
                serde_json::json!({"resourceType": "Patient", "id": id}),
                helios_fhir::FhirVersion::default(),
            )
        })
        .collect();

    let untimed = first.write_search_entries_page(&tenant, &resources).await;
    let target: &dyn ReindexTarget = &second;
    let mut stats = ReindexPageStats::default();
    let timed = target
        .write_search_entries_page_timed(&tenant, &resources, &mut stats)
        .await;

    let simplify = |v: Vec<StorageResult<usize>>| {
        v.into_iter()
            .map(|r| r.as_ref().ok().copied())
            .collect::<Vec<_>>()
    };
    assert_eq!(simplify(untimed), vec![Some(1), None, Some(1)]);
    assert_eq!(simplify(timed), vec![Some(1), None, Some(1)]);
    assert_eq!(
        first.written.lock().clone(),
        vec!["p0".to_string(), "p1".to_string(), "p2".to_string()]
    );
    assert_eq!(
        second.written.lock().clone(),
        vec!["p0".to_string(), "p1".to_string(), "p2".to_string()]
    );
    assert_eq!(stats, ReindexPageStats::default());
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test -p helios-persistence --lib -- search::reindex::tests::reindex_page_stats_accumulate_adds_every_field search::reindex::tests::default_timed_page_write_delegates_and_measures_nothing
```
Expected: compile error — `ReindexPageStats` and `write_search_entries_page_timed` are not found.

- [ ] **Step 3: Implement**

Insert after `SkippedResource`'s closing brace (before the `ResourceRef` doc comment):

```rust
/// What one writer measured while rebuilding one page, reported through
/// [`ReindexTarget::write_search_entries_page_timed`] (#1403). A writer adds to
/// it as each phase completes, so a page that fails part-way still reports the
/// phases it finished. A writer that does not measure leaves it zero; the driver
/// still times the whole call itself.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReindexPageStats {
    /// Search-parameter extraction and building the page's index entries.
    pub extract: Duration,
    /// Removing the page's stale entries (every delete command of the page,
    /// including a failed one).
    pub delete: Duration,
    /// Writing the page's new entries (every insert command of the page,
    /// including a failed one).
    pub insert: Duration,
    /// Stale entries the successful deletes removed.
    pub deleted_entries: u64,
    /// Entries handed to insert commands that were issued, including any the
    /// backend then rejected.
    pub inserted_entries: u64,
    /// Insert commands issued, including any that failed.
    pub insert_commands: u64,
}

impl ReindexPageStats {
    /// Adds every duration and count of `other` to this one.
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

Inside the `#[async_trait] pub trait ReindexTarget { ... }` block, directly after `write_search_entries_page`'s closing brace (and still inside the trait block, so `async_trait` boxes it and `Arc<dyn ReindexTarget>` stays object-safe):

```rust
    /// Like [`Self::write_search_entries_page`], also reporting where the
    /// writer's time went (#1403). The reindex driver calls this one; `stats`
    /// arrives zeroed and the writer adds to it as each phase completes. The
    /// default measures nothing and delegates, so a writer that does not
    /// override it behaves exactly as before. A writer that overrides this
    /// MUST implement `write_search_entries_page` as a delegate to it (with a
    /// throwaway `ReindexPageStats`), so the driver's path and direct callers
    /// (the composite ingest sink) can never diverge.
    async fn write_search_entries_page_timed(
        &self,
        tenant: &TenantContext,
        resources: &[StoredResource],
        stats: &mut ReindexPageStats,
    ) -> Vec<StorageResult<usize>> {
        let _ = stats;
        self.write_search_entries_page(tenant, resources).await
    }
```

Add one sentence at the end of `write_search_entries_page`'s doc comment: "The reindex driver calls [`Self::write_search_entries_page_timed`], whose default delegates here."

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cargo test -p helios-persistence --lib -- search::reindex::tests::reindex_page_stats_accumulate_adds_every_field search::reindex::tests::default_timed_page_write_delegates_and_measures_nothing
```
Expected: `test result: ok. 2 passed`.

- [ ] **Step 5: Full-crate compile check, clippy, fmt, commit**

```bash
cargo check -p helios-persistence --all-features --tests
cargo check -p helios-hfs
cargo clippy -p helios-persistence --all-features --tests -- -D warnings -A clippy::items_after_test_module -A clippy::large_enum_variant -A clippy::question_mark -A clippy::collapsible_match -A clippy::collapsible_if -A clippy::field_reassign_with_default -A clippy::doc-overindented-list-items -A clippy::doc-lazy-continuation
rustfmt --edition 2024 crates/persistence/src/search/reindex.rs
git add crates/persistence/src/search/reindex.rs
git commit -m "$(cat <<'EOF'
feat(persistence): ReindexPageStats and write_search_entries_page_timed (#1403 PR0)

Adds the out-parameter every ReindexTarget implementor can report writer
phase timing through. The default delegates and measures nothing, so no
existing implementor or caller changes; MongoDB's override lands in a
later commit.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: `reindex_stats.rs` — pure wall-clock accounting

**Files:**
- Create: `crates/persistence/src/search/reindex_stats.rs`
- Modify: `crates/persistence/src/search/mod.rs` — add `mod reindex_stats;` after `pub mod reindex;` (line 77); add `ReindexPageStats` to the `pub use reindex::{...}` list (lines 116–120) in alphabetical position, immediately after `ReindexOperation`.

**Interfaces:**
- Consumes: `super::reindex::ReindexPageStats` (Task 1).
- Produces (all `pub(super)`, used only from `reindex.rs`):
  - Consts: `OUTCOME_COMPLETED: &str = "completed"`, `OUTCOME_CANCELLED: &str = "cancelled"`, `OUTCOME_FAILED: &str = "failed"`, `NO_TYPE: &str = "-"`.
  - `fn millis(d: Duration) -> u64`, `fn per_second(count: u64, elapsed: Duration) -> f64`.
  - `struct PageRecord { resources: u64, entries: u64, failed: u64, fetch: Duration, write: Duration, writer: ReindexPageStats }` (`Copy`).
  - `struct Counters { resources, entries, failed, pages: u64, fetch, write, yielded: Duration, writer: ReindexPageStats }` (`Copy`) with private `add_page(&mut self, page: &PageRecord)` and `add_yield(&mut self, d: Duration)`.
  - `struct PhaseMillis { fetch_ms, write_ms, extract_ms, delete_ms, insert_ms, writer_other_ms, yield_ms, other_ms: u64 }` with `fn of(c: &Counters, elapsed: Duration) -> Self`.
  - `struct TypeStarted { resource_type: String, type_index: usize, types: usize, type_total: u64, elapsed: Duration }`.
  - `struct RecordedPage { page: u64, type_elapsed: Duration, progress: Option<ProgressSnapshot> }`.
  - `struct ProgressSnapshot { resource_type: String, type_index: usize, type_total: u64, type_elapsed: Duration, type_counters: Counters, processed: u64, total: u64, elapsed: Duration, interval: Duration, interval_resources: u64 }`.
  - `struct TypeSummary { resource_type: String, outcome: &'static str, type_index: usize, type_total: u64, type_elapsed: Duration, elapsed: Duration, counters: Counters }`.
  - `struct JobSummary { outcome: &'static str, types_done: usize, types: usize, total: u64, elapsed: Duration, counters: Counters }`.
  - `struct ReindexRunStats` with `fn new(started: Instant, total: u64, types: usize, progress_every: Duration) -> Self`, `fn total(&self) -> u64`, `fn types(&self) -> usize`, `fn mark_pages_started(&mut self, now: Instant)`, `fn start_type(&mut self, resource_type: &str, type_total: u64, now: Instant) -> TypeStarted`, `fn record_page(&mut self, page: PageRecord, now: Instant) -> RecordedPage`, `fn add_yield(&mut self, d: Duration)`, `fn finish_type(&mut self, outcome: &'static str, now: Instant) -> Option<TypeSummary>`, `fn finish_job(&self, outcome: &'static str, now: Instant) -> JobSummary`.

- [ ] **Step 1: Write the failing unit tests**

Create `crates/persistence/src/search/reindex_stats.rs` containing only the test module for now:

```rust
//! Always-on wall-clock accounting of one `run_reindex` call (#1403). Local to
//! the run, so concurrent jobs never mix (unlike `crate::perf`'s
//! process-global counters). Logged by `reindex.rs`; the field contract
//! (`reindex job started` ... `reindex page`) is documented on its log
//! helpers and in the design for #1403.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_page_adds_to_the_open_type_and_the_job() {
        let t0 = Instant::now();
        let mut stats = ReindexRunStats::new(t0, 10, 1, Duration::from_secs(60));
        stats.mark_pages_started(t0);
        stats.start_type("Patient", 10, t0);

        let page1 = PageRecord {
            resources: 3,
            entries: 3,
            fetch: Duration::from_millis(5),
            write: Duration::from_millis(7),
            ..Default::default()
        };
        let recorded1 = stats.record_page(page1, t0 + Duration::from_secs(1));
        assert_eq!(recorded1.page, 1);
        assert_eq!(recorded1.type_elapsed, Duration::from_secs(1));

        let page2 = PageRecord {
            resources: 4,
            entries: 4,
            fetch: Duration::from_millis(6),
            write: Duration::from_millis(8),
            ..Default::default()
        };
        let recorded2 = stats.record_page(page2, t0 + Duration::from_secs(2));
        assert_eq!(recorded2.page, 2);
        assert_eq!(recorded2.type_elapsed, Duration::from_secs(2));

        let summary = stats
            .finish_type(OUTCOME_COMPLETED, t0 + Duration::from_secs(2))
            .unwrap();
        assert_eq!(summary.counters.resources, 7);
        assert_eq!(summary.counters.entries, 7);
        assert_eq!(summary.counters.pages, 2);
        assert_eq!(summary.counters.fetch, Duration::from_millis(11));
        assert_eq!(summary.counters.write, Duration::from_millis(15));

        let job = stats.finish_job(OUTCOME_COMPLETED, t0 + Duration::from_secs(2));
        assert_eq!(job.counters.resources, 7);
        assert_eq!(job.counters.pages, 2);
    }

    #[test]
    fn finish_type_reports_both_clocks_and_closes_the_type() {
        let t0 = Instant::now();
        let mut stats = ReindexRunStats::new(t0, 5, 1, Duration::from_secs(60));
        stats.mark_pages_started(t0 + Duration::from_secs(1));
        stats.start_type("Patient", 5, t0 + Duration::from_secs(1));
        let summary = stats
            .finish_type(OUTCOME_COMPLETED, t0 + Duration::from_secs(4))
            .unwrap();
        assert_eq!(summary.type_elapsed, Duration::from_secs(3));
        assert_eq!(summary.elapsed, Duration::from_secs(4));
        assert!(
            stats
                .finish_type(OUTCOME_COMPLETED, t0 + Duration::from_secs(5))
                .is_none()
        );
    }

    #[test]
    fn types_done_counts_only_completed_types() {
        let t0 = Instant::now();
        let mut stats = ReindexRunStats::new(t0, 2, 2, Duration::from_secs(60));
        stats.mark_pages_started(t0);
        stats.start_type("Patient", 1, t0);
        stats.finish_type(OUTCOME_COMPLETED, t0 + Duration::from_secs(1));
        stats.start_type("Observation", 1, t0 + Duration::from_secs(1));
        stats.finish_type(OUTCOME_FAILED, t0 + Duration::from_secs(2));
        let job = stats.finish_job(OUTCOME_FAILED, t0 + Duration::from_secs(2));
        assert_eq!(job.types_done, 1);
    }

    #[test]
    fn progress_is_due_once_per_interval() {
        let t0 = Instant::now();
        let mut stats = ReindexRunStats::new(t0, 100, 1, Duration::from_secs(60));
        stats.mark_pages_started(t0);
        stats.start_type("Patient", 100, t0);
        let rec = |n: u64| PageRecord { resources: n, ..Default::default() };

        assert!(
            stats
                .record_page(rec(10), t0 + Duration::from_secs(30))
                .progress
                .is_none()
        );

        let snap = stats
            .record_page(rec(10), t0 + Duration::from_secs(61))
            .progress
            .unwrap();
        assert_eq!(snap.interval, Duration::from_secs(61));
        assert_eq!(snap.interval_resources, 20);
        assert_eq!(snap.elapsed, Duration::from_secs(61));

        assert!(
            stats
                .record_page(rec(5), t0 + Duration::from_secs(62))
                .progress
                .is_none()
        );

        let snap2 = stats
            .record_page(rec(5), t0 + Duration::from_secs(122))
            .progress
            .unwrap();
        assert_eq!(snap2.interval, Duration::from_secs(61));
        assert_eq!(snap2.interval_resources, 10);
    }

    #[test]
    fn first_interval_starts_when_pages_start() {
        let t0 = Instant::now();
        let mut stats = ReindexRunStats::new(t0, 100, 1, Duration::from_secs(60));
        stats.mark_pages_started(t0 + Duration::from_secs(90));
        stats.start_type("Patient", 100, t0 + Duration::from_secs(90));
        let rec = |n: u64| PageRecord { resources: n, ..Default::default() };

        assert!(
            stats
                .record_page(rec(1), t0 + Duration::from_secs(120))
                .progress
                .is_none()
        );

        let snap = stats
            .record_page(rec(1), t0 + Duration::from_secs(151))
            .progress
            .unwrap();
        assert_eq!(snap.interval, Duration::from_secs(61));
        assert_eq!(snap.elapsed, Duration::from_secs(151));
    }

    #[test]
    fn zero_interval_reports_after_every_page() {
        let t0 = Instant::now();
        let mut stats = ReindexRunStats::new(t0, 10, 1, Duration::ZERO);
        stats.mark_pages_started(t0);
        stats.start_type("Patient", 10, t0);
        let rec = PageRecord { resources: 1, ..Default::default() };
        assert!(stats.record_page(rec, t0).progress.is_some());
        assert!(
            stats
                .record_page(rec, t0 + Duration::from_millis(1))
                .progress
                .is_some()
        );
    }

    #[test]
    fn progress_snapshot_is_scoped_to_the_open_type() {
        let t0 = Instant::now();
        let mut stats = ReindexRunStats::new(t0, 9, 2, Duration::ZERO);
        stats.mark_pages_started(t0);
        stats.start_type("Patient", 5, t0);
        stats.record_page(PageRecord { resources: 2, ..Default::default() }, t0);
        stats.record_page(PageRecord { resources: 3, ..Default::default() }, t0);
        stats.finish_type(OUTCOME_COMPLETED, t0);

        stats.start_type("Observation", 4, t0);
        let snap = stats
            .record_page(PageRecord { resources: 4, ..Default::default() }, t0)
            .progress
            .unwrap();
        assert_eq!(snap.resource_type, "Observation");
        assert_eq!(snap.type_index, 2);
        assert_eq!(snap.type_counters.pages, 1);
        assert_eq!(snap.type_counters.resources, 4);
        assert_eq!(snap.processed, 9);

        stats.finish_type(OUTCOME_COMPLETED, t0);
        let recorded = stats.record_page(PageRecord { resources: 1, ..Default::default() }, t0);
        assert_eq!(recorded.page, 0);
        assert_eq!(recorded.type_elapsed, Duration::ZERO);
        let snap2 = recorded.progress.unwrap();
        assert_eq!(snap2.resource_type, NO_TYPE);
        assert_eq!(snap2.type_counters, Counters::default());
        assert_eq!(snap2.processed, 10);
    }

    #[test]
    fn phase_millis_subtract_before_truncating_and_saturate() {
        let mut c = Counters { write: Duration::from_millis(10), ..Default::default() };
        c.writer.extract = Duration::from_millis(3);
        c.writer.delete = Duration::from_millis(2);
        c.writer.insert = Duration::from_millis(4);
        assert_eq!(PhaseMillis::of(&c, Duration::from_millis(10)).writer_other_ms, 1);

        let mut c2 = Counters { write: Duration::from_millis(5), ..Default::default() };
        c2.writer.extract = Duration::from_millis(10);
        assert_eq!(PhaseMillis::of(&c2, Duration::from_millis(5)).writer_other_ms, 0);

        let c3 = Counters {
            fetch: Duration::from_millis(20),
            write: Duration::from_millis(20),
            yielded: Duration::from_millis(20),
            ..Default::default()
        };
        assert_eq!(PhaseMillis::of(&c3, Duration::from_millis(10)).other_ms, 0);

        let mut c4 = Counters {
            write: Duration::from_nanos(2_500_000),
            ..Default::default()
        };
        c4.writer.extract = Duration::from_nanos(1_600_000);
        assert_eq!(
            PhaseMillis::of(&c4, Duration::from_nanos(2_500_000)).writer_other_ms,
            0,
            "truncating extract and write separately before subtracting would wrongly give 1"
        );

        let c5 = Counters { fetch: Duration::from_nanos(1_999_999), ..Default::default() };
        assert_eq!(PhaseMillis::of(&c5, Duration::from_nanos(1_999_999)).fetch_ms, 1);
    }

    #[test]
    fn per_second_rounds_and_never_divides_by_zero() {
        assert_eq!(per_second(3, Duration::from_secs(2)), 1.5);
        assert_eq!(per_second(1000, Duration::ZERO), 0.0);
        assert_eq!(per_second(2, Duration::from_secs(3)), 0.7);
        assert_eq!(per_second(1, Duration::from_secs(3)), 0.3);
    }

    #[test]
    fn yield_adds_to_the_type_and_the_job() {
        let t0 = Instant::now();
        let mut stats = ReindexRunStats::new(t0, 1, 1, Duration::from_secs(60));
        stats.mark_pages_started(t0);
        stats.start_type("Patient", 1, t0);
        stats.add_yield(Duration::from_millis(5));
        let summary = stats.finish_type(OUTCOME_COMPLETED, t0).unwrap();
        assert_eq!(summary.counters.yielded, Duration::from_millis(5));
        let job = stats.finish_job(OUTCOME_COMPLETED, t0);
        assert_eq!(job.counters.yielded, Duration::from_millis(5));
    }
}
```

Declare the module in `crates/persistence/src/search/mod.rs`, directly after `pub mod reindex;`:

```rust
mod reindex_stats;
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test -p helios-persistence --lib -- search::reindex_stats
```
Expected: compile error — `ReindexRunStats`, `PageRecord`, `Counters`, `PhaseMillis`, `OUTCOME_COMPLETED`, `NO_TYPE`, `per_second`, `Instant`, `Duration` not found (the test module's `use super::*;` has nothing to import yet).

- [ ] **Step 3: Implement**

Insert above the test module in `reindex_stats.rs`, directly under the `//!` module doc added in Step 1 and above every `use`:

```rust
#![allow(dead_code)] // #1403 PR0: removed in Task 3, once run_reindex reads every item
```

Nothing in non-test code calls into this module until Task 3 wires it into `run_reindex`. `cargo clippy --tests` also compiles the non-test lib as a dependency of the integration tests, so without this `allow`, `millis`, `per_second`, every struct here and every field on it (including `OUTCOME_CANCELLED`, which no Task 2 test uses even under `#[cfg(test)]`) trips `dead_code` and `-D warnings` fails deterministically. Plain `allow`, not `cfg_attr(not(test))`, because `OUTCOME_CANCELLED` is unused even in the `cfg(test)` build. Then continue with the rest of the module:

```rust
use std::time::{Duration, Instant};

use super::reindex::ReindexPageStats;

pub(super) const OUTCOME_COMPLETED: &str = "completed";
pub(super) const OUTCOME_CANCELLED: &str = "cancelled";
pub(super) const OUTCOME_FAILED: &str = "failed";
/// Logged as `resource_type` when no type is open: no logged value is ever empty.
pub(super) const NO_TYPE: &str = "-";

/// Whole milliseconds, truncated.
pub(super) fn millis(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

/// `count / elapsed` per second, rounded to one decimal; 0.0 when `elapsed` is zero.
pub(super) fn per_second(count: u64, elapsed: Duration) -> f64 {
    if elapsed.is_zero() {
        return 0.0;
    }
    ((count as f64 / elapsed.as_secs_f64()) * 10.0).round() / 10.0
}

/// What one page cost the driver and its writers, before it is folded into a
/// type's or the job's running [`Counters`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct PageRecord {
    pub(super) resources: u64,
    pub(super) entries: u64,
    pub(super) failed: u64,
    pub(super) fetch: Duration,
    pub(super) write: Duration,
    pub(super) writer: ReindexPageStats,
}

/// Running totals over some scope (a type, or the whole job).
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

/// [`Counters`]' phase fields, in whole milliseconds, with the two derived
/// fields (`writer_other_ms`, `other_ms`) computed on `Duration`s before
/// truncation.
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
        let writer_other = c.write.saturating_sub(writer_busy);
        let busy = c.fetch + c.write + c.yielded;
        let other = elapsed.saturating_sub(busy);
        Self {
            fetch_ms: millis(c.fetch),
            write_ms: millis(c.write),
            extract_ms: millis(c.writer.extract),
            delete_ms: millis(c.writer.delete),
            insert_ms: millis(c.writer.insert),
            writer_other_ms: millis(writer_other),
            yield_ms: millis(c.yielded),
            other_ms: millis(other),
        }
    }
}

pub(super) struct TypeStarted {
    pub(super) resource_type: String,
    pub(super) type_index: usize,
    pub(super) types: usize,
    pub(super) type_total: u64,
    /// Job clock at the type's start.
    pub(super) elapsed: Duration,
}

pub(super) struct RecordedPage {
    /// 1-based page number within the open type; 0 when no type is open.
    pub(super) page: u64,
    /// Time since the open type started; zero when no type is open.
    pub(super) type_elapsed: Duration,
    pub(super) progress: Option<ProgressSnapshot>,
}

pub(super) struct ProgressSnapshot {
    /// `NO_TYPE` when no type is open (type fields are then zero).
    pub(super) resource_type: String,
    pub(super) type_index: usize,
    pub(super) type_total: u64,
    pub(super) type_elapsed: Duration,
    pub(super) type_counters: Counters,
    pub(super) processed: u64,
    pub(super) total: u64,
    pub(super) elapsed: Duration,
    pub(super) interval: Duration,
    pub(super) interval_resources: u64,
}

pub(super) struct TypeSummary {
    pub(super) resource_type: String,
    pub(super) outcome: &'static str,
    pub(super) type_index: usize,
    pub(super) type_total: u64,
    pub(super) type_elapsed: Duration,
    pub(super) elapsed: Duration,
    pub(super) counters: Counters,
}

pub(super) struct JobSummary {
    pub(super) outcome: &'static str,
    pub(super) types_done: usize,
    pub(super) types: usize,
    pub(super) total: u64,
    pub(super) elapsed: Duration,
    pub(super) counters: Counters,
}

struct OpenType {
    resource_type: String,
    type_index: usize,
    type_total: u64,
    started: Instant,
    counters: Counters,
}

/// Local, single-job accounting: nothing here is a process-global, unlike
/// `crate::perf`'s counters, so concurrent jobs never mix.
pub(super) struct ReindexRunStats {
    started: Instant,
    total: u64,
    types: usize,
    progress_every: Duration,
    last_progress_at: Instant,
    last_progress_resources: u64,
    job: Counters,
    types_started: usize,
    types_done: usize,
    current: Option<OpenType>,
}

impl ReindexRunStats {
    pub(super) fn new(started: Instant, total: u64, types: usize, progress_every: Duration) -> Self {
        Self {
            started,
            total,
            types,
            progress_every,
            last_progress_at: started,
            last_progress_resources: 0,
            job: Counters::default(),
            types_started: 0,
            types_done: 0,
            current: None,
        }
    }

    pub(super) fn total(&self) -> u64 {
        self.total
    }

    pub(super) fn types(&self) -> usize {
        self.types
    }

    pub(super) fn mark_pages_started(&mut self, now: Instant) {
        self.last_progress_at = now;
        self.last_progress_resources = self.job.resources;
    }

    pub(super) fn start_type(
        &mut self,
        resource_type: &str,
        type_total: u64,
        now: Instant,
    ) -> TypeStarted {
        debug_assert!(
            self.current.is_none(),
            "a type must be finished before the next one starts"
        );
        self.types_started += 1;
        let type_index = self.types_started;
        self.current = Some(OpenType {
            resource_type: resource_type.to_string(),
            type_index,
            type_total,
            started: now,
            counters: Counters::default(),
        });
        TypeStarted {
            resource_type: resource_type.to_string(),
            type_index,
            types: self.types,
            type_total,
            elapsed: now.saturating_duration_since(self.started),
        }
    }

    pub(super) fn record_page(&mut self, page: PageRecord, now: Instant) -> RecordedPage {
        self.job.add_page(&page);
        let (page_number, type_elapsed) = if let Some(open) = self.current.as_mut() {
            open.counters.add_page(&page);
            (
                open.counters.pages,
                now.saturating_duration_since(open.started),
            )
        } else {
            (0, Duration::ZERO)
        };

        let progress = if now.saturating_duration_since(self.last_progress_at) >= self.progress_every {
            let interval = now.saturating_duration_since(self.last_progress_at);
            let interval_resources = self.job.resources.saturating_sub(self.last_progress_resources);
            let (resource_type, type_index, type_total, type_elapsed_snap, type_counters) =
                match &self.current {
                    Some(open) => (
                        open.resource_type.clone(),
                        open.type_index,
                        open.type_total,
                        now.saturating_duration_since(open.started),
                        open.counters,
                    ),
                    None => (NO_TYPE.to_string(), 0, 0, Duration::ZERO, Counters::default()),
                };
            self.last_progress_at = now;
            self.last_progress_resources = self.job.resources;
            Some(ProgressSnapshot {
                resource_type,
                type_index,
                type_total,
                type_elapsed: type_elapsed_snap,
                type_counters,
                processed: self.job.resources,
                total: self.total,
                elapsed: now.saturating_duration_since(self.started),
                interval,
                interval_resources,
            })
        } else {
            None
        };

        RecordedPage { page: page_number, type_elapsed, progress }
    }

    pub(super) fn add_yield(&mut self, d: Duration) {
        self.job.add_yield(d);
        if let Some(open) = self.current.as_mut() {
            open.counters.add_yield(d);
        }
    }

    pub(super) fn finish_type(&mut self, outcome: &'static str, now: Instant) -> Option<TypeSummary> {
        let open = self.current.take()?;
        if outcome == OUTCOME_COMPLETED {
            self.types_done += 1;
        }
        Some(TypeSummary {
            resource_type: open.resource_type,
            outcome,
            type_index: open.type_index,
            type_total: open.type_total,
            type_elapsed: now.saturating_duration_since(open.started),
            elapsed: now.saturating_duration_since(self.started),
            counters: open.counters,
        })
    }

    pub(super) fn finish_job(&self, outcome: &'static str, now: Instant) -> JobSummary {
        JobSummary {
            outcome,
            types_done: self.types_done,
            types: self.types,
            total: self.total,
            elapsed: now.saturating_duration_since(self.started),
            counters: self.job,
        }
    }
}
```

In `crates/persistence/src/search/mod.rs`, add `ReindexPageStats` to the `pub use reindex::{...}` list, alphabetically after `ReindexOperation`:

```rust
pub use reindex::{
    DEFERRED_REINDEX_BATCH_SIZE, DeferredReindexLedger, ReindexOnFinish, ReindexOperation,
    ReindexPageStats, ReindexProgress, ReindexProgressError, ReindexRequest, ReindexSource,
    ReindexStatus, ReindexTarget, ReindexableStorage, ResourcePage, ResourceRef, SkippedResource,
};
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cargo test -p helios-persistence --lib -- search::reindex_stats
```
Expected: `test result: ok. 10 passed`.

- [ ] **Step 5: Full-crate compile check, clippy, fmt, commit**

```bash
cargo check -p helios-persistence --all-features --tests
cargo check -p helios-hfs
cargo clippy -p helios-persistence --all-features --tests -- -D warnings -A clippy::items_after_test_module -A clippy::large_enum_variant -A clippy::question_mark -A clippy::collapsible_match -A clippy::collapsible_if -A clippy::field_reassign_with_default -A clippy::doc-overindented-list-items -A clippy::doc-lazy-continuation
rustfmt --edition 2024 crates/persistence/src/search/reindex_stats.rs crates/persistence/src/search/mod.rs
git status --short crates/persistence
```
`rustfmt` on `mod.rs` reformats every out-of-line submodule it declares — if the last command lists anything besides `reindex_stats.rs`, `mod.rs` and (whitespace-only) `reindex.rs`, `git checkout --` the unexpected file before committing.

```bash
git add crates/persistence/src/search/reindex_stats.rs crates/persistence/src/search/mod.rs
git commit -m "$(cat <<'EOF'
feat(persistence): pure wall-clock accounting for the reindex log lines (#1403 PR0)

reindex_stats.rs is process-local (unlike crate::perf's process-global
counters) and untested against tracing or I/O: ReindexRunStats turns
page records into job- and type-scoped Counters and derived PhaseMillis,
tested with synthetic Instants. Not yet wired into run_reindex.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: Wire the six log lines into `run_reindex`

**Files:**
- Modify: `crates/persistence/src/search/reindex.rs` — `REINDEX_PROGRESS_INTERVAL` constant (near line 2000, after `REINDEX_PAGE_YIELD`); `ReindexOperation` struct (line 999) + `with_parts` (line 1038) + a `#[cfg(test)]` seam; `start_tracked` (line 1118); `write_resource_batch` (line 1938) return type; `run_reindex` (line 2020) full instrumentation; new private helpers; new imports.
- Modify: `crates/persistence/src/search/reindex_stats.rs` — delete the `#![allow(dead_code)]` line Task 2 added at the top of the module (this task is what makes every item in it read from `run_reindex`).
- Test: `crates/persistence/src/search/reindex.rs`, existing `mod tests` (line 2456).

**Interfaces:**
- Consumes: `reindex_stats::{ReindexRunStats, PageRecord, PhaseMillis, TypeStarted, RecordedPage, ProgressSnapshot, TypeSummary, JobSummary, OUTCOME_COMPLETED, OUTCOME_CANCELLED, OUTCOME_FAILED, millis, per_second}` (Task 2; `Counters` is not imported directly — it is reached only through `TypeSummary.counters`, `ProgressSnapshot.type_counters` and `JobSummary.counters`); `ReindexTarget::write_search_entries_page_timed` (Task 1, the provided trait method — not a method on `ReindexPageStats` itself).
- Produces:
  - `const REINDEX_PROGRESS_INTERVAL: Duration = Duration::from_secs(60);`
  - `ReindexOperation.progress_interval: Duration` (new last field) + `#[cfg(test)] pub(crate) fn with_progress_interval(mut self, every: Duration) -> Self`.
  - `struct BatchOutcome { entries: u64, failed: u64, write: Duration, writer: ReindexPageStats }` (private), replacing `write_resource_batch`'s `()` return.
  - `run_reindex(..., progress_interval: Duration)` — one new trailing parameter; the six log lines now fire from inside it.
  - Private: `fn exit_outcome(outcome: &Result<(), RunExit>) -> &'static str`, `fn job_outcome(jobs: &Arc<RwLock<HashMap<String, ReindexProgress>>>, job_id: &str, path_outcome: &'static str) -> &'static str`, `fn log_job_end(...)`, `fn record_and_log_page(...)`, `fn log_job_started(...)`, `fn log_type_started(...)`, `fn log_type_finished(...)`, `fn log_progress(...)`, `fn log_job_finished(...)`, `fn log_page(...)`.

- [ ] **Step 1a: Write the capture harness and the two plain unit tests**

Add the capture-contract harness and two plain unit tests to the existing `mod tests { ... }` block in `reindex.rs` (place them near the end of the module, after the existing `RecordingTarget`/`recording_operation`/`await_finished` helpers at lines 4230–4305, since the driver tests added in Step 1b use them):

```rust
// --- #1403 PR0: the six-line log contract -------------------------------

const CONTRACT_MESSAGES: &[&str] = &[
    "reindex job started",
    "reindex type started",
    "reindex type finished",
    "reindex progress",
    "reindex job finished",
    "reindex page",
];

const JOB_STARTED_FIELDS: &[&str] = &[
    "tenant", "job_id", "types", "total", "batch_size", "batch_bytes",
    "bulk_index_rebuild", "clear_existing", "resource_scoped", "writers", "setup_ms",
];
const TYPE_STARTED_FIELDS: &[&str] =
    &["tenant", "job_id", "resource_type", "type_index", "types", "type_total", "elapsed_ms"];
const TYPE_FINISHED_FIELDS: &[&str] = &[
    "tenant", "job_id", "resource_type", "outcome", "type_index", "type_resources",
    "type_total", "type_elapsed_ms", "type_resources_per_s", "elapsed_ms", "entries",
    "failed", "pages", "fetch_ms", "write_ms", "extract_ms", "delete_ms", "insert_ms",
    "writer_other_ms", "yield_ms", "other_ms", "deleted", "inserted", "insert_commands",
];
const PROGRESS_FIELDS: &[&str] = &[
    "tenant", "job_id", "resource_type", "type_index", "type_resources", "type_total",
    "type_elapsed_ms", "type_resources_per_s", "processed", "total", "elapsed_ms",
    "interval_ms", "interval_resources", "interval_resources_per_s", "entries", "failed",
    "pages", "fetch_ms", "write_ms", "extract_ms", "delete_ms", "insert_ms",
    "writer_other_ms", "yield_ms", "other_ms", "deleted", "inserted", "insert_commands",
];
const JOB_FINISHED_FIELDS: &[&str] = &[
    "tenant", "job_id", "outcome", "types_done", "types", "processed", "total",
    "elapsed_ms", "resources_per_s", "entries", "failed", "pages", "fetch_ms", "write_ms",
    "extract_ms", "delete_ms", "insert_ms", "writer_other_ms", "yield_ms", "other_ms",
    "deleted", "inserted", "insert_commands",
];
const PAGE_FIELDS: &[&str] = &[
    "tenant", "job_id", "resource_type", "page", "resources", "type_elapsed_ms",
    "entries", "failed", "fetch_ms", "write_ms", "extract_ms", "delete_ms", "insert_ms",
    "deleted", "inserted", "insert_commands",
];

/// One captured `reindex ...` event: field names in printed (macro) order,
/// `message` excluded, plus each field's `{value:?}` text (a `%`-recorded
/// string arrives unquoted; a bare `&str` would arrive quoted, but this
/// module logs none).
#[derive(Debug, Clone)]
struct ContractEvent {
    level: tracing::Level,
    message: String,
    names: Vec<&'static str>,
    values: HashMap<&'static str, String>,
}

/// New code; shaped like `core/bulk_submit_worker.rs`'s `CaptureEvents` but
/// not shared with it, because this one keeps per-field values and filters to
/// this module's six contract messages instead of collecting flat text.
struct CaptureContract {
    events: Arc<std::sync::Mutex<Vec<ContractEvent>>>,
}

impl tracing::Subscriber for CaptureContract {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        if event.metadata().target() != "helios_persistence::search::reindex" {
            return;
        }
        struct Visitor {
            values: HashMap<&'static str, String>,
            message: String,
        }
        impl tracing::field::Visit for Visitor {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                if field.name() == "message" {
                    self.message = format!("{value:?}");
                } else {
                    self.values.insert(field.name(), format!("{value:?}"));
                }
            }
        }
        let mut visitor = Visitor { values: HashMap::new(), message: String::new() };
        event.record(&mut visitor);
        if !CONTRACT_MESSAGES.contains(&visitor.message.as_str()) {
            return;
        }
        let names: Vec<&'static str> = event
            .metadata()
            .fields()
            .iter()
            .map(|f| f.name())
            .filter(|n| *n != "message")
            .collect();
        self.events.lock().unwrap().push(ContractEvent {
            level: *event.metadata().level(),
            message: visitor.message,
            names,
            values: visitor.values,
        });
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

fn capture_contract() -> (tracing::subscriber::DefaultGuard, Arc<std::sync::Mutex<Vec<ContractEvent>>>) {
    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let guard = tracing::subscriber::set_default(CaptureContract { events: events.clone() });
    (guard, events)
}

/// Every captured event's field names match the documented order for its
/// message, and its level is DEBUG for `reindex page`, INFO for the rest. A
/// lost subscriber (an empty capture) fails loudly rather than passing
/// vacuously.
fn assert_contract(events: &[ContractEvent]) {
    assert!(!events.is_empty(), "no reindex contract events were captured");
    for event in events {
        let (expected_names, expected_level): (&[&str], tracing::Level) = match event.message.as_str() {
            "reindex job started" => (JOB_STARTED_FIELDS, tracing::Level::INFO),
            "reindex type started" => (TYPE_STARTED_FIELDS, tracing::Level::INFO),
            "reindex type finished" => (TYPE_FINISHED_FIELDS, tracing::Level::INFO),
            "reindex progress" => (PROGRESS_FIELDS, tracing::Level::INFO),
            "reindex job finished" => (JOB_FINISHED_FIELDS, tracing::Level::INFO),
            "reindex page" => (PAGE_FIELDS, tracing::Level::DEBUG),
            other => panic!("unexpected contract message {other:?}"),
        };
        assert_eq!(event.names, expected_names, "{}", event.message);
        assert_eq!(event.level, expected_level, "{}", event.message);
    }
}

#[test]
fn exit_outcome_maps_every_exit() {
    assert_eq!(exit_outcome(&Ok(())), OUTCOME_COMPLETED);
    assert_eq!(exit_outcome(&Err(RunExit::Cancelled)), OUTCOME_CANCELLED);
    assert_eq!(
        exit_outcome(&Err(RunExit::Failed("boom".to_string()))),
        OUTCOME_FAILED
    );
}

#[test]
fn job_outcome_prefers_a_terminal_status_already_written() {
    let jobs: Arc<RwLock<HashMap<String, ReindexProgress>>> = Arc::new(RwLock::new(HashMap::new()));
    let mut progress = ReindexProgress::new("j");
    progress.status = ReindexStatus::InProgress;
    jobs.write().insert("j".to_string(), progress);

    assert_eq!(job_outcome(&jobs, "j", OUTCOME_COMPLETED), OUTCOME_COMPLETED);

    jobs.write().get_mut("j").unwrap().status = ReindexStatus::Cancelled;
    assert_eq!(job_outcome(&jobs, "j", OUTCOME_COMPLETED), OUTCOME_CANCELLED);

    jobs.write().get_mut("j").unwrap().status = ReindexStatus::Completed;
    assert_eq!(job_outcome(&jobs, "j", OUTCOME_FAILED), OUTCOME_COMPLETED);

    jobs.write().remove("j");
    assert_eq!(job_outcome(&jobs, "j", OUTCOME_FAILED), OUTCOME_FAILED);
}
```

Nothing here calls `run_reindex` directly, so these two tests compile only once Step 3d (which adds the `exit_outcome`/`job_outcome` functions) exists, same as the rest of this module's new tests; they are written now, together with the harness, to keep Steps 1a/1b as one self-contained "write every test" pass before Steps 3a–3d implement against them.

- [ ] **Step 1b: Write the five driver-log tests**

Add the `MeasuringTarget` double and the five `#[tokio::test]` driver-log tests, directly after the two tests from Step 1a, in the same `mod tests { ... }` block:

```rust
/// Injects fixed writer-phase durations and counts per call, without
/// sleeping, so the driver's saturating-subtraction formulas can be checked
/// against exact numbers instead of timing noise.
#[derive(Default)]
struct MeasuringTarget;

#[async_trait]
impl ReindexTarget for MeasuringTarget {
    async fn delete_search_entries(&self, _: &TenantContext, _: &str, _: &str) -> StorageResult<u64> {
        Ok(0)
    }
    async fn write_search_entries(&self, _: &TenantContext, _: &StoredResource) -> StorageResult<usize> {
        Ok(1)
    }
    async fn clear_search_index(&self, _: &TenantContext) -> StorageResult<u64> {
        Ok(0)
    }
    async fn write_search_entries_page(
        &self,
        tenant: &TenantContext,
        resources: &[StoredResource],
    ) -> Vec<StorageResult<usize>> {
        let mut stats = ReindexPageStats::default();
        self.write_search_entries_page_timed(tenant, resources, &mut stats).await
    }
    async fn write_search_entries_page_timed(
        &self,
        _: &TenantContext,
        resources: &[StoredResource],
        stats: &mut ReindexPageStats,
    ) -> Vec<StorageResult<usize>> {
        stats.extract += Duration::from_millis(3);
        stats.deleted_entries += 2;
        stats.inserted_entries += 5;
        stats.insert_commands += 1;
        resources.iter().map(|_| Ok(1)).collect()
    }
}

/// The regression test for S1's major review defect: L4 (`reindex progress`)
/// must be scoped to the *open type*, not the whole job — Observation
/// running after Patient must not carry Patient's rows into its own quartiles.
#[tokio::test]
async fn a_run_logs_job_type_progress_and_page_lines_with_the_documented_fields() {
    let (_guard, events) = capture_contract();
    let source = Arc::new(PagedSource::new(5));
    let op = Arc::new(
        ReindexOperation::with_parts(
            source,
            vec![Arc::new(MeasuringTarget)],
            Arc::new(crate::search::TenantSearchRegistries::base_only()),
        )
        .with_progress_interval(Duration::ZERO),
    );
    let tenant = named_tenant("driver-log-contract");
    let tenant_id = tenant.tenant_id().as_str().to_string();
    let job_id = op
        .start(
            tenant,
            ReindexRequest::for_types(vec!["Patient".to_string(), "Observation".to_string()])
                .with_batch_size(2),
            None,
        )
        .await
        .unwrap();
    let progress = await_finished(&op, &job_id).await;
    assert_eq!(progress.status, ReindexStatus::Completed);

    let events = events.lock().unwrap().clone();
    assert_contract(&events);

    let messages: Vec<&str> = events.iter().map(|e| e.message.as_str()).collect();
    assert_eq!(
        messages,
        vec![
            "reindex job started",
            "reindex type started",
            "reindex page", "reindex progress",
            "reindex page", "reindex progress",
            "reindex page", "reindex progress",
            "reindex type finished",
            "reindex type started",
            "reindex page", "reindex progress",
            "reindex page", "reindex progress",
            "reindex page", "reindex progress",
            "reindex type finished",
            "reindex job finished",
        ]
    );

    let pages: Vec<&ContractEvent> = events.iter().filter(|e| e.message == "reindex page").collect();
    assert_eq!(pages.len(), 6);
    let first_type_pages: Vec<&str> = pages[..3].iter().map(|e| e.values["page"].as_str()).collect();
    assert_eq!(first_type_pages, vec!["1", "2", "3"]);
    let first_type_resources: Vec<&str> =
        pages[..3].iter().map(|e| e.values["resources"].as_str()).collect();
    assert_eq!(first_type_resources, vec!["2", "2", "1"]);

    let obs_progress: Vec<&ContractEvent> = events
        .iter()
        .filter(|e| {
            e.message == "reindex progress" && e.values.get("resource_type").map(String::as_str) == Some("Observation")
        })
        .collect();
    assert_eq!(obs_progress.len(), 3);
    assert_eq!(
        obs_progress.iter().map(|e| e.values["pages"].as_str()).collect::<Vec<_>>(),
        vec!["1", "2", "3"]
    );
    assert_eq!(
        obs_progress.iter().map(|e| e.values["type_resources"].as_str()).collect::<Vec<_>>(),
        vec!["2", "4", "5"]
    );
    assert_eq!(
        obs_progress.iter().map(|e| e.values["extract_ms"].as_str()).collect::<Vec<_>>(),
        vec!["3", "6", "9"]
    );
    assert_eq!(
        obs_progress.iter().map(|e| e.values["inserted"].as_str()).collect::<Vec<_>>(),
        vec!["5", "10", "15"]
    );
    assert_eq!(
        obs_progress.iter().map(|e| e.values["insert_commands"].as_str()).collect::<Vec<_>>(),
        vec!["1", "2", "3"]
    );
    assert_eq!(
        obs_progress.iter().map(|e| e.values["processed"].as_str()).collect::<Vec<_>>(),
        vec!["7", "9", "10"]
    );
    assert_eq!(
        obs_progress.iter().map(|e| e.values["interval_resources"].as_str()).collect::<Vec<_>>(),
        vec!["2", "2", "1"]
    );

    for tf in events.iter().filter(|e| e.message == "reindex type finished") {
        assert_eq!(tf.values["outcome"], "completed");
        assert_eq!(tf.values["type_resources"], "5");
        assert_eq!(tf.values["type_total"], "5");
        assert_eq!(tf.values["pages"], "3");
        assert_eq!(tf.values["entries"], "5");
        assert_eq!(tf.values["failed"], "0");
        assert_eq!(tf.values["extract_ms"], "9");
        assert_eq!(tf.values["deleted"], "6");
        assert_eq!(tf.values["inserted"], "15");
        assert_eq!(tf.values["insert_commands"], "3");
        assert_eq!(
            tf.values["writer_other_ms"], "0",
            "the double injects durations without sleeping, so subtraction must saturate exactly at 0"
        );
        let yield_ms: u64 = tf.values["yield_ms"].parse().unwrap();
        assert!(yield_ms >= 10, "expected >= 10 from two 5 ms yields, got {yield_ms}");
    }

    let l2: Vec<&ContractEvent> = events.iter().filter(|e| e.message == "reindex type started").collect();
    assert_eq!(l2[0].values["type_index"], "1");
    assert_eq!(l2[1].values["type_index"], "2");
    for e in &l2 {
        assert_eq!(e.values["types"], "2");
        assert_eq!(e.values["type_total"], "5");
    }

    let l1 = events.iter().find(|e| e.message == "reindex job started").unwrap();
    assert_eq!(l1.values["types"], "2");
    assert_eq!(l1.values["total"], "10");
    assert_eq!(l1.values["batch_size"], "2");
    assert_eq!(l1.values["batch_bytes"], "0");
    assert_eq!(l1.values["bulk_index_rebuild"], "false");
    assert_eq!(l1.values["clear_existing"], "false");
    assert_eq!(l1.values["resource_scoped"], "false");
    assert_eq!(l1.values["writers"], "1");
    assert_eq!(l1.values["tenant"], tenant_id);
    assert_eq!(l1.values["job_id"], job_id);

    let l5 = events.iter().find(|e| e.message == "reindex job finished").unwrap();
    assert_eq!(l5.values["outcome"], "completed");
    assert_eq!(l5.values["types_done"], "2");
    assert_eq!(l5.values["types"], "2");
    assert_eq!(l5.values["processed"], "10");
    assert_eq!(l5.values["total"], "10");
    assert_eq!(l5.values["pages"], "6");
    assert_eq!(l5.values["entries"], "10");
    assert_eq!(l5.values["extract_ms"], "18");
    assert_eq!(l5.values["deleted"], "12");
    assert_eq!(l5.values["inserted"], "30");
    assert_eq!(l5.values["insert_commands"], "6");
    assert_eq!(l5.values["processed"].parse::<u64>().unwrap(), progress.processed_resources);

    let mut last_elapsed: Option<u64> = None;
    for e in &events {
        if let Some(v) = e.values.get("elapsed_ms") {
            let ms: u64 = v.parse().unwrap();
            if let Some(prev) = last_elapsed {
                assert!(ms >= prev, "elapsed_ms decreased: {prev} -> {ms}");
            }
            last_elapsed = Some(ms);
        }
    }
}

#[tokio::test]
async fn a_writer_that_does_not_measure_logs_zero_writer_phases() {
    let (_guard, events) = capture_contract();
    let source = Arc::new(TimedPageSource::new(3));
    let target = Arc::new(RecordingTarget::default());
    let op = recording_operation(source, target);
    let job_id = op
        .start(named_tenant("undefault-writer"), ReindexRequest::default(), None)
        .await
        .unwrap();
    await_finished(&op, &job_id).await;

    let events = events.lock().unwrap().clone();
    assert_contract(&events);
    let l3 = events.iter().find(|e| e.message == "reindex type finished").unwrap();
    assert_eq!(l3.values["extract_ms"], "0");
    assert_eq!(l3.values["delete_ms"], "0");
    assert_eq!(l3.values["insert_ms"], "0");
    assert_eq!(l3.values["deleted"], "0");
    assert_eq!(l3.values["inserted"], "0");
    assert_eq!(l3.values["insert_commands"], "0");
    assert!(l3.values.contains_key("write_ms"));
    assert!(
        !events.iter().any(|e| e.message == "reindex progress"),
        "the default 60 s interval must not fire for a run this short"
    );
}

#[tokio::test]
async fn a_run_scoped_to_named_resources_logs_one_page_per_id_batch() {
    let (_guard, events) = capture_contract();
    let source = Arc::new(PagedSource::new(10));
    let target = Arc::new(RecordingTarget::default());
    let op = recording_operation(source, target);
    let job_id = op
        .start(
            named_tenant("named-resources-log"),
            ReindexRequest::for_resources([
                ResourceRef::new("Patient", "p7"),
                ResourceRef::new("Patient", "p2"),
                ResourceRef::new("Patient", "p2"),
                ResourceRef::new("Patient", "deleted"),
            ])
            .with_batch_size(2),
            None,
        )
        .await
        .unwrap();
    await_finished(&op, &job_id).await;

    let events = events.lock().unwrap().clone();
    assert_contract(&events);
    let l1 = events.iter().find(|e| e.message == "reindex job started").unwrap();
    assert_eq!(l1.values["resource_scoped"], "true");
    assert_eq!(l1.values["total"], "3");
    assert_eq!(l1.values["types"], "1");
    let l2 = events.iter().find(|e| e.message == "reindex type started").unwrap();
    assert_eq!(l2.values["type_total"], "3");
    let pages: Vec<&ContractEvent> = events.iter().filter(|e| e.message == "reindex page").collect();
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0].values["resources"], "2");
    assert_eq!(pages[1].values["resources"], "1");
    let l3 = events.iter().find(|e| e.message == "reindex type finished").unwrap();
    assert_eq!(l3.values["type_resources"], "3");
    assert_eq!(l3.values["pages"], "2");
    assert_eq!(l3.values["entries"], "2");
    assert_eq!(l3.values["failed"], "0");
    let yield_ms: u64 = l3.values["yield_ms"].parse().unwrap();
    assert!(yield_ms >= 5);
    let l5 = events.iter().find(|e| e.message == "reindex job finished").unwrap();
    assert_eq!(l5.values["processed"], "3");
}

#[tokio::test]
async fn a_failed_page_closes_its_type_and_the_job_as_failed() {
    let (_guard, events) = capture_contract();
    let source: Arc<dyn ReindexSource> = Arc::new(FailingPageSource);
    let target: Arc<dyn ReindexTarget> = Arc::new(CountingTarget::default());
    let op = Arc::new(ReindexOperation::with_parts(
        source,
        vec![target],
        Arc::new(crate::search::TenantSearchRegistries::base_only()),
    ));
    let job_id = op
        .start(named_tenant("failed-page-log"), ReindexRequest::default(), None)
        .await
        .unwrap();
    await_finished(&op, &job_id).await;

    let events = events.lock().unwrap().clone();
    assert_contract(&events);
    // Filter to INFO before indexing: `FailingPageSource` fails the fetch, so
    // no `reindex page`/`reindex progress` DEBUG or extra INFO line is
    // expected here, but the filter is applied for the same reason the
    // cancelled-run test below needs it — indexing raw `events` would silently
    // break if a future change added a DEBUG line before L3/L5.
    let info: Vec<&ContractEvent> = events.iter().filter(|e| e.level == tracing::Level::INFO).collect();
    let messages: Vec<&str> = info.iter().map(|e| e.message.as_str()).collect();
    assert_eq!(
        messages,
        vec!["reindex job started", "reindex type started", "reindex type finished", "reindex job finished"]
    );
    assert_eq!(info[1].values["type_total"], "1");
    assert_eq!(info[2].values["outcome"], "failed");
    assert_eq!(info[2].values["type_resources"], "0");
    assert_eq!(info[2].values["pages"], "0");
    assert_eq!(info[3].values["outcome"], "failed");
    assert_eq!(info[3].values["types_done"], "0");
    assert_eq!(info[3].values["processed"], "0");
    assert_eq!(info[3].values["total"], "1");
}

#[tokio::test]
async fn a_cancelled_run_closes_the_open_type_as_cancelled() {
    let (_guard, events) = capture_contract();
    let (backend, mut controlled_events) = ControlledBackend::new(Vec::new(), 0);
    let source = Arc::new(PagedSource::new(9));
    let source_port: Arc<dyn ReindexSource> = source.clone();
    let writers: Vec<Arc<dyn ReindexTarget>> = vec![backend.clone()];
    let op = Arc::new(
        ReindexOperation::with_parts(
            source_port,
            writers,
            Arc::new(crate::search::TenantSearchRegistries::base_only()),
        )
        .with_progress_interval(Duration::from_secs(3600)),
    );
    let tenant = named_tenant("cancelled-run-log");
    let tenant_id = tenant.tenant_id().to_string();

    let job_id = op
        .start(
            tenant,
            ReindexRequest::for_types(vec!["Patient".to_string()]).with_batch_size(2),
            None,
        )
        .await
        .expect("start the reindex");

    assert_eq!(
        next_controlled_event(&mut controlled_events).await,
        ControlledEvent::Write { tenant: tenant_id, resource_type: "Patient".to_string() }
    );

    op.cancel(&job_id).await.expect("cancel the job");
    backend.write_gate.add_permits(2);
    tokio::time::timeout(Duration::from_secs(2), async {
        while op.cancel_channels.read().contains_key(&job_id) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the cancelled reindex task did not return");

    let events = events.lock().unwrap().clone();
    assert_contract(&events);
    // This run writes one page (`resources=2`), so the DEBUG `reindex page`
    // line (L6) is captured alongside the four INFO lines. Filter to INFO
    // before asserting order and indexing by position, or `events[2]` lands
    // on L6 (which has no `outcome` field) instead of L3, and `HashMap`
    // indexing panics.
    let info: Vec<&ContractEvent> = events.iter().filter(|e| e.level == tracing::Level::INFO).collect();
    let messages: Vec<&str> = info.iter().map(|e| e.message.as_str()).collect();
    assert_eq!(
        messages,
        vec!["reindex job started", "reindex type started", "reindex type finished", "reindex job finished"]
    );
    assert_eq!(info[2].values["outcome"], "cancelled");
    assert_eq!(info[2].values["type_resources"], "2");
    assert_eq!(info[2].values["pages"], "1");
    assert_eq!(info[3].values["outcome"], "cancelled");
    let page_events: Vec<&ContractEvent> = events.iter().filter(|e| e.message == "reindex page").collect();
    assert_eq!(page_events.len(), 1);
    assert_eq!(page_events[0].values["page"], "1");
    assert_eq!(page_events[0].values["resources"], "2");
    assert!(!events.iter().any(|e| e.message == "reindex progress"));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test -p helios-persistence --lib -- search::reindex::tests
```
Expected: compile error — `exit_outcome`, `job_outcome`, `with_progress_interval` and the `reindex_stats` re-exports used above are not found; `ReindexOperation::with_parts` does not have a `.with_progress_interval` yet.

- [ ] **Step 3a: Constant, `ReindexOperation` field and seam, `start_tracked`**

**(a) Constant.** After `REINDEX_PAGE_YIELD`:

```rust
/// How often, at most, a running reindex logs `reindex progress` (#1403).
/// Checked at page boundaries, so a page that outlasts it shows up as a
/// longer `interval_ms`.
const REINDEX_PROGRESS_INTERVAL: Duration = Duration::from_secs(60);
```

**(b) `ReindexOperation`.** Add a last field to the struct:

```rust
    /// Cadence of `reindex progress` lines (#1403); [`REINDEX_PROGRESS_INTERVAL`]
    /// outside tests.
    progress_interval: Duration,
```

In `with_parts`, initialize it (`new` delegates to `with_parts`, so both constructors get it):

```rust
            progress_interval: REINDEX_PROGRESS_INTERVAL,
```

Add the test seam, inside `impl ReindexOperation`:

```rust
    /// Overrides the progress-line cadence; tests only.
    #[cfg(test)]
    pub(crate) fn with_progress_interval(mut self, every: Duration) -> Self {
        self.progress_interval = every;
        self
    }
```

**(c) `start_tracked`.** Clone the field alongside the other clones before the spawn:

```rust
        let progress_interval = self.progress_interval;
```

Do not yet change the `run_reindex(...)` call inside `tokio::spawn` — that call gains the new argument in Step 3c, in the same edit that adds the matching parameter to `run_reindex`'s signature. Adding the argument here, before the signature exists to accept it, would leave the crate uncompilable at the Step 3b checkpoint below (`this function takes N arguments but N+1 arguments were supplied`). Until Step 3c, `progress_interval` here is an unused local (`dead_code` warning under plain `cargo test`, not a compile error).

- [ ] **Step 3b: `BatchOutcome` and `write_resource_batch`**

**(d) `write_resource_batch`.** Declare a new private struct just above it:

```rust
/// What one batch cost and produced, for the run's accounting (#1403).
#[derive(Debug, Default)]
struct BatchOutcome {
    /// Sum of entries over resources at least one writer wrote (what
    /// `entries_created` advanced by).
    entries: u64,
    /// Writer `Err`s recorded for this batch, over all writers.
    failed: u64,
    /// Time inside `write_search_entries_page_timed`, summed over writers.
    write: Duration,
    /// What the writers measured, accumulated over writers.
    writer: ReindexPageStats,
}
```

Change its return type from `()` to `BatchOutcome`, and its body to the code below. Keep the existing doc comment and `#[allow(clippy::too_many_arguments)]` attribute directly above the signature, word for word except the one changed sentence noted after the listing — do not drop them, or clippy `-D warnings` fails with `too_many_arguments` (8 parameters):

```rust
/// Rewrites one batch of resources through every writer and advances the
/// job's counters by `resources.len() + extra_processed`.
///
/// Page-at-a-time so a writer can wrap it in one transaction; each writer
/// reports a per-resource outcome for error attribution. The entry count for
/// progress comes from the writers' own extraction — the driver no longer
/// extracts a second time just to count. `extra_processed` accounts for rows
/// of the batch that were read but not written (skipped, or deleted since
/// they were named).
#[allow(clippy::too_many_arguments)]
async fn write_resource_batch(
    tenant: &TenantContext,
    writers: &[Arc<dyn ReindexTarget>],
    jobs: &Arc<RwLock<HashMap<String, ReindexProgress>>>,
    job_id: &str,
    failures: &mut ResourceFailureLog,
    resource_type: &str,
    resources: &[StoredResource],
    extra_processed: u64,
) -> BatchOutcome {
    let mut wrote_any: Vec<bool> = vec![false; resources.len()];
    let mut entry_counts: Vec<u64> = vec![0; resources.len()];
    let mut batch = BatchOutcome::default();
    if !resources.is_empty() {
        for writer in writers {
            let mut page_stats = ReindexPageStats::default();
            let started = Instant::now();
            let outcomes = writer
                .write_search_entries_page_timed(tenant, resources, &mut page_stats)
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
                            jobs,
                            job_id,
                            failures,
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

    let mut jobs_guard = jobs.write();
    if let Some(progress) = jobs_guard.get_mut(job_id) {
        progress.processed_resources += resources.len() as u64 + extra_processed;
        progress.entries_created += entries;
    }
    drop(jobs_guard);

    batch
}
```

Update the doc comment: "...and returns the batch's accounting for the run's log lines (#1403)."

**Checkpoint.** Steps 1a/1b's tests already call `exit_outcome`, `job_outcome` and `MeasuringTarget`'s `_timed` override, none of which exist until Step 3d, so `cargo test --lib` (which compiles `#[cfg(test)]` code) still shows the same missing-symbol compile error as Step 2's red run — that is expected, not a regression. What Step 3b's own new code can actually be checked against here is the **non-test** lib, which excludes `#[cfg(test)] mod tests` entirely and so never sees those not-yet-defined symbols:
```bash
cargo check -p helios-persistence --lib
```
Expected: this succeeds (both callers of `write_resource_batch` discard its return value as a bare `.await;` statement, which type-checks unchanged against the new `BatchOutcome` return type). `BatchOutcome`'s fields are not yet read anywhere outside tests, which is a `dead_code` warning, not an error, since this command carries no `-D warnings`. Step 3c's `run_reindex` wiring (via `record_and_log_page`) is what reads them; Task 3's Step 5 clippy gate (which does carry `-D warnings`, and also compiles the test module via `--tests`) runs only after Steps 3c and 3d are both done.

- [ ] **Step 3c: Wire `run_reindex`**

**(e) `run_reindex`.**

1. Add the last parameter `progress_interval: Duration` to the signature. In the same edit, pass `progress_interval` (the local `start_tracked` bound in Step 3a) as the new **last** argument of the `run_reindex(...)` call inside `tokio::spawn` — the signature and its one call site change together, so the crate compiles both before this step (old signature, old call) and after it (new signature, new call), never in between.
2. Before `perf_run`, add `let run_started = Instant::now();`.
3. Just before `let mut total_resources` (the count-loop setup), declare `let mut type_totals: HashMap<String, u64> = HashMap::new();`. Change the named-resources arm's body:
   ```rust
   if let Some(named) = &named_resources {
       total_resources = named.values().map(|ids| ids.len() as u64).sum();
       for (resource_type, ids) in named {
           type_totals.insert(resource_type.clone(), ids.len() as u64);
       }
   } else {
   ```
   Change the count loop's `Ok(count)` arm from an expression to a block:
   ```rust
   Ok(count) => {
       total_resources += count;
       type_totals.insert(resource_type.clone(), count);
   }
   ```
4. After the "Update total" block and before `if request.clear_existing`, add:
   ```rust
   let tenant_label = tenant.tenant_id().as_str().to_string();
   let mut stats = ReindexRunStats::new(run_started, total_resources, resource_types.len(), progress_interval);
   log_job_started(
       &tenant_label,
       &job_id,
       &stats,
       request.batch_size,
       request.batch_bytes,
       request.bulk_index_rebuild,
       request.clear_existing,
       named_resources.is_some(),
       writers.len(),
       run_started.elapsed(),
   );
   ```
   Pass scalar fields, never `&request` — `request.resource_types` was already moved earlier, so `request` is partially moved by this point.
5. On the `clear_search_index` failure and the `begin_bulk_index_rebuild` failure (each returns right after calling `mark_failed`), insert `log_job_end(&stats, &jobs, &tenant_label, &job_id, OUTCOME_FAILED);` immediately before that `mark_failed(...)` call:
   ```rust
   if request.clear_existing {
       for writer in &writers {
           if let Err(e) = writer.clear_search_index(&tenant).await {
               log_job_end(&stats, &jobs, &tenant_label, &job_id, OUTCOME_FAILED);
               mark_failed(&jobs, &job_id, format!("Failed to clear search index: {e}"));
               return;
           }
       }
   }
   ```
   ```rust
   if request.bulk_index_rebuild {
       for writer in &writers {
           if let Err(e) = writer.begin_bulk_index_rebuild().await {
               log_job_end(&stats, &jobs, &tenant_label, &job_id, OUTCOME_FAILED);
               mark_failed(
                   &jobs,
                   &job_id,
                   format!("Failed to enter bulk index rebuild: {e}"),
               );
               return;
           }
       }
   }
   ```
6. Immediately before `let mut failures = ResourceFailureLog::new(&job_id, &tenant);`, add `stats.mark_pages_started(Instant::now());`. The `async { ... }` block that follows is not `move`, so it borrows `stats` mutably exactly as it already borrows `failures`.
7. After `failures.start_type(resource_type);`, add:
   ```rust
   let type_total = type_totals.get(resource_type.as_str()).copied().unwrap_or(0);
   log_type_started(&tenant_label, &job_id, &stats.start_type(resource_type, type_total, Instant::now()));
   ```
8. **Named path.** Replace the body of the `if batch_index > 0 { yield_between_pages().await; }` arm with:
   ```rust
   let yielded = Instant::now();
   yield_between_pages().await;
   stats.add_yield(yielded.elapsed());
   ```
   Add `let fetch_started = Instant::now();` immediately before the fetch span is opened, and `let fetch_time = fetch_started.elapsed();` immediately after the span is dropped (the perf span itself stays). Bind the batch call as `let batch_outcome = write_resource_batch(...).await;`, then add:
   ```rust
   record_and_log_page(
       &mut stats,
       &tenant_label,
       &job_id,
       resource_type,
       PageRecord {
           resources: batch.len() as u64,
           entries: batch_outcome.entries,
           failed: batch_outcome.failed,
           fetch: fetch_time,
           write: batch_outcome.write,
           writer: batch_outcome.writer,
       },
   );
   ```
   (`batch` here is the existing chunk variable of `for (batch_index, batch) in ...`.) Before the loop's `continue;`, add:
   ```rust
   if let Some(summary) = stats.finish_type(OUTCOME_COMPLETED, Instant::now()) {
       log_type_finished(&tenant_label, &job_id, &summary);
   }
   ```
9. **Paging path.** Replace the whole `// Process resources in batches` block (the `let mut cursor` through the `loop { ... }`'s closing `}`, still inside the type's `for` body) with:
   ```rust
   // Process resources in batches
   let mut cursor: Option<String> = None;
   loop {
       // Check for cancellation
       if cancel_rx.try_recv().is_ok() {
           return Err(RunExit::Cancelled);
       }

       // Fetch a page of resources
       let fetch_started = Instant::now();
       let fetch_span = crate::perf::span(crate::perf::Phase::ReindexFetch);
       let fetched = source
           .fetch_resources_page_capped(
               &tenant,
               resource_type,
               cursor.as_deref(),
               request.batch_size,
               request.batch_bytes,
           )
           .await;
       drop(fetch_span);
       let fetch_time = fetch_started.elapsed();
       let page = match fetched {
           Ok(page) => page,
           Err(e) => {
               return Err(RunExit::Failed(format!("Failed to fetch resources: {e}")));
           }
       };

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
               fetch: fetch_time,
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
   `page.next_cursor` is read (`.len()`) before the `match page.next_cursor { ... }` that moves it, exactly as `resources: (page.resources.len() + page.skipped.len())` above already reads `page.resources`/`page.skipped` before the batch call consumes them by reference only (`write_resource_batch` takes `&page.resources`, so `page` itself is still intact when `record_and_log_page` and then `match page.next_cursor` run).
10. Immediately after `failures.finish_type();` (right after the `async { ... }.await` block, at the top level of `run_reindex`), add:
    ```rust
    if let Some(summary) = stats.finish_type(exit_outcome(&outcome), Instant::now()) {
        log_type_finished(&tenant_label, &job_id, &summary);
    }
    ```
11. On the `end_bulk_index_rebuild` failure, insert `log_job_end(&stats, &jobs, &tenant_label, &job_id, OUTCOME_FAILED);` immediately before its `mark_failed(...)` call:
    ```rust
    if request.bulk_index_rebuild {
        for writer in &writers {
            if let Err(e) = writer.end_bulk_index_rebuild().await {
                log_job_end(&stats, &jobs, &tenant_label, &job_id, OUTCOME_FAILED);
                mark_failed(
                    &jobs,
                    &job_id,
                    format!("Failed to leave bulk index rebuild: {e}"),
                );
                return;
            }
        }
    }
    ```
12. Rewrite `match outcome { ... }`'s two error arms as blocks:
    ```rust
    match outcome {
        Err(RunExit::Cancelled) => {
            log_job_end(&stats, &jobs, &tenant_label, &job_id, OUTCOME_CANCELLED);
            return mark_cancelled(&jobs, &job_id);
        }
        Err(RunExit::Failed(msg)) => {
            log_job_end(&stats, &jobs, &tenant_label, &job_id, OUTCOME_FAILED);
            return mark_failed(&jobs, &job_id, msg);
        }
        Ok(()) => {}
    }
    ```
13. After the perf-summary block and before the completion write (`// Mark as completed`), add `log_job_end(&stats, &jobs, &tenant_label, &job_id, OUTCOME_COMPLETED);`.

**Rule that falls out of this:** `run_reindex` always logs L5 **before** it writes the terminal status, with one exception — `cancel()` writes `Cancelled` synchronously before the task has returned, so a test of the cancel path must wait for the cancellation channel to be released (as the existing `cancellation_during_a_page_finishes_it_and_stops_before_the_next_fetch` test already does), not for the terminal status alone.

- [ ] **Step 3d: Private helpers, imports, and removing Task 2's temporary allow**

**(f) Private helpers.** Add after `yield_between_pages`, so every line they emit gets the module target:

```rust
fn exit_outcome(outcome: &Result<(), RunExit>) -> &'static str {
    match outcome {
        Ok(()) => OUTCOME_COMPLETED,
        Err(RunExit::Cancelled) => OUTCOME_CANCELLED,
        Err(RunExit::Failed(_)) => OUTCOME_FAILED,
    }
}

fn job_outcome(
    jobs: &Arc<RwLock<HashMap<String, ReindexProgress>>>,
    job_id: &str,
    path_outcome: &'static str,
) -> &'static str {
    match jobs.read().get(job_id).map(|p| p.status) {
        Some(ReindexStatus::Completed) => OUTCOME_COMPLETED,
        Some(ReindexStatus::Cancelled) => OUTCOME_CANCELLED,
        Some(ReindexStatus::Failed) => OUTCOME_FAILED,
        _ => path_outcome,
    }
}

fn log_job_end(
    stats: &ReindexRunStats,
    jobs: &Arc<RwLock<HashMap<String, ReindexProgress>>>,
    tenant: &str,
    job_id: &str,
    path_outcome: &'static str,
) {
    let outcome = job_outcome(jobs, job_id, path_outcome);
    log_job_finished(tenant, job_id, &stats.finish_job(outcome, Instant::now()));
}

fn record_and_log_page(
    stats: &mut ReindexRunStats,
    tenant: &str,
    job_id: &str,
    resource_type: &str,
    record: PageRecord,
) {
    let recorded = stats.record_page(record, Instant::now());
    log_page(tenant, job_id, resource_type, &recorded, &record);
    if let Some(p) = &recorded.progress {
        log_progress(tenant, job_id, p);
    }
}

/// Logs L1 `reindex job started` (INFO), once per run, after counting and
/// before `clear_existing` or `begin_bulk_index_rebuild`. Field order:
/// `tenant, job_id, types, total, batch_size, batch_bytes, bulk_index_rebuild,
/// clear_existing, resource_scoped, writers, setup_ms`. `types`/`total` come
/// from `stats` (job-scoped, fixed for the run); the rest mirror the
/// request's shape so each arm's configuration is visible in the log.
/// `setup_ms` is the time from entry into `run_reindex` to the end of
/// counting. Fields are appended only, never renamed, removed or reordered
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
        "reindex job started"
    );
}

/// Logs L2 `reindex type started` (INFO), once per type, when its `for`
/// iteration begins. Field order: `tenant, job_id, resource_type, type_index,
/// types, type_total, elapsed_ms`. `type_index` is the type's 1-based
/// position in the run's order; `type_total` is that type's resource count
/// from the initial count (or the number of distinct named ids). `elapsed_ms`
/// is the job clock (time since entry into `run_reindex`) at this line, never
/// the type's own clock. Fields are appended only, never renamed, removed or
/// reordered (#1403).
fn log_type_started(tenant: &str, job_id: &str, s: &TypeStarted) {
    tracing::info!(
        tenant = %tenant,
        job_id = %job_id,
        resource_type = %s.resource_type,
        type_index = s.type_index as u64,
        types = s.types as u64,
        type_total = s.type_total,
        elapsed_ms = millis(s.elapsed),
        "reindex type started"
    );
}

/// Logs L3 `reindex type finished` (INFO), exactly once per L2, on every
/// path on which `run_reindex` returns (completed, cancelled or failed).
/// Field order: `tenant, job_id, resource_type, outcome, type_index,
/// type_resources, type_total, type_elapsed_ms, type_resources_per_s,
/// elapsed_ms, entries, failed, pages, fetch_ms, write_ms, extract_ms,
/// delete_ms, insert_ms, writer_other_ms, yield_ms, other_ms, deleted,
/// inserted, insert_commands`. `outcome` is this type's own exit path — a
/// type that finished all its pages is always `completed`, even if a later
/// type or the job as a whole fails or is cancelled. Every counter and phase
/// field (`entries` through `insert_commands`) is scoped to this type only,
/// since its L2. `type_elapsed_ms` is this type's own clock; `elapsed_ms` is
/// always the job clock. `writer_other_ms = write_ms − (extract_ms +
/// delete_ms + insert_ms)`, and `other_ms = type_elapsed − (fetch + write +
/// yield)`: both are computed by subtracting on `Duration`s, saturating at
/// zero, and truncating to whole milliseconds only afterward (never
/// truncate-then-subtract). `deleted`/`inserted`/`insert_commands` come from
/// the type's accumulated `ReindexPageStats` (writer-reported; zero for a
/// writer that does not measure — MongoDB-only in PR0). Fields are appended
/// only, never renamed, removed or reordered — the one sanctioned exception
/// is PR2b's redefinition of `other_ms`/`writer_other_ms` on the critical
/// path (S3 D11), documented here when that PR lands (#1403).
fn log_type_finished(tenant: &str, job_id: &str, s: &TypeSummary) {
    let phases = PhaseMillis::of(&s.counters, s.type_elapsed);
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
        fetch_ms = phases.fetch_ms,
        write_ms = phases.write_ms,
        extract_ms = phases.extract_ms,
        delete_ms = phases.delete_ms,
        insert_ms = phases.insert_ms,
        writer_other_ms = phases.writer_other_ms,
        yield_ms = phases.yield_ms,
        other_ms = phases.other_ms,
        deleted = s.counters.writer.deleted_entries,
        inserted = s.counters.writer.inserted_entries,
        insert_commands = s.counters.writer.insert_commands,
        "reindex type finished"
    );
}

/// Logs L4 `reindex progress` (INFO), at the first page boundary at which at
/// least `progress_interval` has passed since the previous L4, or since the
/// page loop started. Field order: `tenant, job_id, resource_type,
/// type_index, type_resources, type_total, type_elapsed_ms,
/// type_resources_per_s, processed, total, elapsed_ms, interval_ms,
/// interval_resources, interval_resources_per_s, entries, failed, pages,
/// fetch_ms, write_ms, extract_ms, delete_ms, insert_ms, writer_other_ms,
/// yield_ms, other_ms, deleted, inserted, insert_commands`. **Scope is the
/// open type, not the whole job**: `type_resources`, `type_total`,
/// `type_elapsed_ms`, `type_resources_per_s`, and every counter and phase
/// field from `entries` through `insert_commands`, describe only the type
/// open since its own `reindex type started` line — Observation running
/// after Patient must never inherit Patient's counts. `processed`, `total`
/// and `elapsed_ms` are job-scoped. `interval_ms`/`interval_resources`/
/// `interval_resources_per_s` are job-level, measured since the previous L4
/// (an interval can span a type boundary). When no type is open,
/// `resource_type` is the sentinel `-` (`NO_TYPE`) and every type-scoped
/// field is zero — no logged value is ever empty. `writer_other_ms` and
/// `other_ms` use the same saturating-subtract-then-truncate formulas as L3
/// (see [`log_type_finished`]). Fields are appended only, never renamed,
/// removed or reordered (#1403).
fn log_progress(tenant: &str, job_id: &str, s: &ProgressSnapshot) {
    let phases = PhaseMillis::of(&s.type_counters, s.type_elapsed);
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
        fetch_ms = phases.fetch_ms,
        write_ms = phases.write_ms,
        extract_ms = phases.extract_ms,
        delete_ms = phases.delete_ms,
        insert_ms = phases.insert_ms,
        writer_other_ms = phases.writer_other_ms,
        yield_ms = phases.yield_ms,
        other_ms = phases.other_ms,
        deleted = s.type_counters.writer.deleted_entries,
        inserted = s.type_counters.writer.inserted_entries,
        insert_commands = s.type_counters.writer.insert_commands,
        "reindex progress"
    );
}

/// Logs L5 `reindex job finished` (INFO), once per run that logged L1, on
/// every path on which `run_reindex` returns after L1, and always **before**
/// `run_reindex` writes the terminal status (the one exception: a synchronous
/// `cancel()` may write `Cancelled` first — see [`job_outcome`], D10). Field
/// order: `tenant, job_id, outcome, types_done, types, processed, total,
/// elapsed_ms, resources_per_s, entries, failed, pages, fetch_ms, write_ms,
/// extract_ms, delete_ms, insert_ms, writer_other_ms, yield_ms, other_ms,
/// deleted, inserted, insert_commands`. `outcome` is the job's already-
/// written terminal status if one exists, otherwise the exit path's outcome
/// (`job_outcome`). `types_done` counts types whose L3 said `completed`.
/// Every counter and phase field (`entries` through `insert_commands`) is
/// job-scoped (summed over every type), unlike L3/L4's type scope.
/// `elapsed_ms` is the job clock. `writer_other_ms = write_ms − (extract_ms +
/// delete_ms + insert_ms)` and `other_ms = elapsed_ms − (fetch + write +
/// yield)`, both saturating-subtract-then-truncate on `Duration`s, as in L3.
/// Fields are appended only, never renamed, removed or reordered (#1403).
fn log_job_finished(tenant: &str, job_id: &str, s: &JobSummary) {
    let phases = PhaseMillis::of(&s.counters, s.elapsed);
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
        fetch_ms = phases.fetch_ms,
        write_ms = phases.write_ms,
        extract_ms = phases.extract_ms,
        delete_ms = phases.delete_ms,
        insert_ms = phases.insert_ms,
        writer_other_ms = phases.writer_other_ms,
        yield_ms = phases.yield_ms,
        other_ms = phases.other_ms,
        deleted = s.counters.writer.deleted_entries,
        inserted = s.counters.writer.inserted_entries,
        insert_commands = s.counters.writer.insert_commands,
        "reindex job finished"
    );
}

/// Logs L6 `reindex page` (DEBUG; needs
/// `RUST_LOG=…,helios_persistence::search::reindex=debug`), after every page
/// or id batch. Field order: `tenant, job_id, resource_type, page, resources,
/// type_elapsed_ms, entries, failed, fetch_ms, write_ms, extract_ms,
/// delete_ms, insert_ms, deleted, inserted, insert_commands`. Every counter
/// and phase field describes only this one page — `page` is the 1-based page
/// number within the open type, `resources` is this page's own count. L6 has
/// no derived fields: `fetch_ms`/`write_ms` are the driver's own timings for
/// this page, and `extract_ms`/`delete_ms`/`insert_ms`/`deleted`/`inserted`/
/// `insert_commands` come straight from this page's `ReindexPageStats`
/// (writer-reported; zero for a writer that does not measure). Fields are
/// appended only, never renamed, removed or reordered (#1403).
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
        "reindex page"
    );
}
```

**(g) Imports.** Add near the top of `reindex.rs`:

```rust
use super::reindex_stats::{
    JobSummary, OUTCOME_CANCELLED, OUTCOME_COMPLETED, OUTCOME_FAILED, PageRecord, PhaseMillis,
    ProgressSnapshot, RecordedPage, ReindexRunStats, TypeStarted, TypeSummary, millis,
    per_second,
};
```
(`NO_TYPE` is not imported — only `reindex_stats.rs` needs it, and clippy `-D warnings` rejects unused imports. `Instant`, `Duration` and `HashMap` are already imported.)

**(h) Delete Task 2's temporary allow.** Delete the `#![allow(dead_code)] // #1403 PR0: removed in Task 3, once run_reindex reads every item` line at the top of `crates/persistence/src/search/reindex_stats.rs`. This step's wiring is what makes every `pub(super)` item in that module read from `reindex.rs`, so the allow is no longer needed; leaving it in would hide a real dead-code regression in a later PR.

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cargo test -p helios-persistence --lib -- search::reindex::tests
```
Expected: every existing test in the module still passes (the default `_timed` delegates, so no double needed a change), plus the 7 new tests: `test result: ok. N passed` where N is the module's previous count plus 7.

- [ ] **Step 5: Full-crate compile check, clippy, fmt, commit**

```bash
cargo check -p helios-persistence --all-features --tests
cargo check -p helios-hfs
cargo clippy -p helios-persistence --all-features --tests -- -D warnings -A clippy::items_after_test_module -A clippy::large_enum_variant -A clippy::question_mark -A clippy::collapsible_match -A clippy::collapsible_if -A clippy::field_reassign_with_default -A clippy::doc-overindented-list-items -A clippy::doc-lazy-continuation
rustfmt --edition 2024 crates/persistence/src/search/reindex.rs crates/persistence/src/search/reindex_stats.rs
git add crates/persistence/src/search/reindex.rs crates/persistence/src/search/reindex_stats.rs
git commit -m "$(cat <<'EOF'
feat(persistence): wire the six reindex log lines into run_reindex (#1403 PR0)

Job/type/progress/page timing now flows through ReindexRunStats into
fixed-format tracing lines at helios_persistence::search::reindex: L1-L5
at INFO, L6 (per page) at DEBUG. Progress and page counters are scoped to
the open type, not the whole job -- Observation running after another
type must not inherit its rows. No writer's behaviour changes; every
implementor except MongoDB (next commit) reports zero writer phases.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: MongoDB writer phase timing

**Files:**
- Modify: `crates/persistence/src/backends/mongodb/storage.rs` — imports (`use std::collections::{HashMap, HashSet};` at line 3 gains a sibling `use std::time::Instant;`; the `use crate::search::reindex::{ReindexSource, ReindexTarget, ResourcePage};` import gains `ReindexPageStats`); rename `write_search_entries_page` (lines 4996–5164, inside `impl ReindexTarget for MongoBackend` at line 4868) to `write_search_entries_page_timed` with instrumentation; add a new thin `write_search_entries_page`; `insert_search_entries_chunk` (lines 5189–5218) gains a `stats` parameter.
- Modify: `crates/persistence/src/perf.rs` — doc comments only, on the `ReindexExtract` (line 119), `ReindexSearchDelete` (line 121) and `ReindexSearchInsert` (line 125) `Phase` variants.
- Test: `crates/persistence/tests/mongodb_tests.rs` — new test after `mongodb_integration_reindex_page_counts_contained_entries` (ends at line 8536).

**Interfaces:**
- Consumes: `ReindexPageStats` (Task 1); existing `crate::perf::{span, record_duration, add_rows, Phase}`.
- Produces:
  - `impl ReindexTarget for MongoBackend { async fn write_search_entries_page_timed(&self, tenant: &TenantContext, resources: &[StoredResource], stats: &mut ReindexPageStats) -> Vec<StorageResult<usize>>; async fn write_search_entries_page(&self, tenant: &TenantContext, resources: &[StoredResource]) -> Vec<StorageResult<usize>> }`.
  - `async fn insert_search_entries_chunk(collection: &mongodb::Collection<Document>, owners: &[usize], docs: &[Document], error_context: &str, stats: &mut ReindexPageStats) -> Result<HashMap<usize, String>, String>`.

- [ ] **Step 1: Write the failing integration test**

Append to `crates/persistence/tests/mongodb_tests.rs`, directly after `mongodb_integration_reindex_page_counts_contained_entries`:

```rust
/// #1403 PR0: MongoDB is the only writer that measures its own phases in this
/// PR. Every call goes through `&dyn ReindexTarget` to prove dynamic dispatch
/// reaches MongoDB's override rather than the trait's zero-measuring default
/// (S1 "default-method trap").
#[tokio::test]
async fn mongodb_integration_reindex_page_reports_phase_stats() {
    use helios_persistence::search::{ReindexPageStats, ReindexTarget};
    use helios_persistence::types::StoredResource;

    let Some(backend) = create_backend_with_full_registry("reindex_page_phase_stats").await else {
        eprintln!(
            "Skipping mongodb_integration_reindex_page_reports_phase_stats (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    let tenant = create_tenant("reindex-phase-stats-tenant");
    let target: &dyn ReindexTarget = &backend;

    // Step 1: seed 8 Patients.
    let patients: Vec<StoredResource> = (0..8)
        .map(|i| {
            StoredResource::from_storage(
                "Patient",
                &format!("phase-stats-{i}"),
                "1",
                tenant.tenant_id().clone(),
                json!({
                    "resourceType": "Patient",
                    "id": format!("phase-stats-{i}"),
                    "name": [{"family": "Stats"}],
                    "gender": "female",
                    "birthDate": "1990-01-01"
                }),
                chrono::Utc::now(),
                chrono::Utc::now(),
                None,
                FhirVersion::default(),
            )
        })
        .collect();
    let mut first = ReindexPageStats::default();
    let outcomes = target
        .write_search_entries_page_timed(&tenant, &patients, &mut first)
        .await;
    assert!(outcomes.iter().all(|r| r.is_ok()), "{outcomes:?}");
    assert_eq!(first.deleted_entries, 0, "the database is unique per test");
    assert!(first.inserted_entries > 0);
    assert_eq!(first.insert_commands, 1);
    assert!(first.extract > std::time::Duration::ZERO);
    assert!(first.delete > std::time::Duration::ZERO);
    assert!(first.insert > std::time::Duration::ZERO);

    // Step 2: rewrite the same page.
    let mut second = ReindexPageStats::default();
    let outcomes = target
        .write_search_entries_page_timed(&tenant, &patients, &mut second)
        .await;
    assert!(outcomes.iter().all(|r| r.is_ok()), "{outcomes:?}");
    assert_eq!(second.deleted_entries, first.inserted_entries);
    assert_eq!(second.inserted_entries, first.inserted_entries);
    let mut total_after_rewrite = 0u64;
    for p in &patients {
        total_after_rewrite += search_index_entry_count(&backend, &tenant, "Patient", p.id()).await;
    }
    assert_eq!(total_after_rewrite, first.inserted_entries);

    // Step 3: contained.
    let with_contained = StoredResource::from_storage(
        "Observation",
        "phase-stats-contained",
        "1",
        tenant.tenant_id().clone(),
        json!({
            "resourceType": "Observation",
            "id": "phase-stats-contained",
            "status": "final",
            "contained": [{
                "resourceType": "Patient",
                "id": "inner",
                "name": [{"family": "Contained"}]
            }],
            "subject": {"reference": "#inner"},
            "code": {"coding": [{"system": "http://loinc.org", "code": "1234-5"}]}
        }),
        chrono::Utc::now(),
        chrono::Utc::now(),
        None,
        FhirVersion::default(),
    );
    let mut third = ReindexPageStats::default();
    let outcomes = target
        .write_search_entries_page_timed(&tenant, std::slice::from_ref(&with_contained), &mut third)
        .await;
    assert!(outcomes.iter().all(|r| r.is_ok()), "{outcomes:?}");
    let own_rows =
        search_index_entry_count(&backend, &tenant, "Observation", "phase-stats-contained").await;
    let client = raw_test_client(&backend.config().connection_string)
        .await
        .expect("failed to connect MongoDB client for search_index_contained assertions");
    let database = client.database(&backend.config().database_name);
    let contained_rows = database
        .collection::<Document>("search_index_contained")
        .count_documents(doc! {
            "tenant_id": tenant.tenant_id().as_str(),
            "resource_type": "Observation",
            "resource_id": "phase-stats-contained",
        })
        .await
        .expect("failed to count search_index_contained rows");
    assert!(contained_rows > 0);
    assert_eq!(third.inserted_entries, own_rows + contained_rows);
    assert_eq!(third.insert_commands, 2);

    // Step 4: offloaded.
    let Some(offloaded) =
        create_backend_with_search_offloaded("reindex_page_phase_stats_offloaded", true).await
    else {
        eprintln!("Skipping the offloaded step (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let offloaded_target: &dyn ReindexTarget = &offloaded;
    let mut offloaded_stats = ReindexPageStats::default();
    let outcomes = offloaded_target
        .write_search_entries_page_timed(&tenant, &patients, &mut offloaded_stats)
        .await;
    assert!(outcomes.iter().all(|r| matches!(r, Ok(0))), "{outcomes:?}");
    assert_eq!(offloaded_stats, ReindexPageStats::default());
}
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
docker info >/dev/null 2>&1 || [ -n "$HFS_TEST_MONGODB_URL" ] || { echo "Docker/HFS_TEST_MONGODB_URL unavailable: this step cannot be verified"; exit 1; }
cargo test -p helios-persistence --features mongodb --test mongodb_tests -- reindex_page_reports_phase_stats --nocapture 2>&1 | tee /tmp/pr0-mongo-phase-stats-red.log
! grep -q '^Skipping' /tmp/pr0-mongo-phase-stats-red.log
```
The last line's assertion is the point: a skip prints `Skipping mongodb_integration_reindex_page_reports_phase_stats (...)` and libtest still reports `test result: ok. 1 passed` for it, the same line a real pass shows — a skip is not red, so `! grep -q '^Skipping' ...` must itself succeed (no `Skipping` line found) before this step counts as verified. If it fails (a `Skipping` line is present), stop, get Docker running, and re-run before proceeding.
Expected (Docker reachable, no skip): compiles fine — Task 1's default `_timed` already exists on the trait — but the test binary panics at `assert!(first.inserted_entries > 0)`, because the trait's default measures nothing until MongoDB overrides it. The captured log's panic message names that line; the two asserts after it never run, since the panic unwinds the test.

- [ ] **Step 3: Implement**

In `storage.rs`, add the import and change the existing one:

```rust
use std::time::Instant;
```
(placed after the existing `use std::collections::{HashMap, HashSet};`)

```rust
use crate::search::reindex::{ReindexPageStats, ReindexSource, ReindexTarget, ResourcePage};
```

Inside `#[async_trait] impl ReindexTarget for MongoBackend { ... }`, replace the whole `write_search_entries_page` method (keep its existing doc comment, and every inline comment already inside the body, word for word — only the additions below are new, per S1 §5.4 "Only these additions are made") with:

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

        // Honors `is_search_offloaded()`, matching the guards in
        // `delete_search_entries` and `write_search_entries`/`clear_search_index`
        // above: a search-offloaded backend keeps no index of its own and must
        // issue no commands here.
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

        struct Prepared {
            docs: SearchIndexDocuments,
            failure: Option<String>,
        }
        let extract_started = Instant::now();
        let prepared: Vec<Prepared> = resources
            .iter()
            .map(|resource| {
                let (docs, failure) = self.search_index_documents_checked(
                    tenant_id,
                    resource.resource_type(),
                    resource.id(),
                    resource.content(),
                );
                Prepared { docs, failure }
            })
            .collect();
        let extract_time = extract_started.elapsed();
        stats.extract += extract_time;
        crate::perf::record_duration(crate::perf::Phase::ReindexExtract, extract_time);

        let collection = db.collection::<Document>(MongoBackend::SEARCH_INDEX_COLLECTION);
        let contained_collection =
            db.collection::<Document>(MongoBackend::SEARCH_INDEX_CONTAINED_COLLECTION);

        // ONE delete per distinct resource_type in the page (a production
        // page is single-type — `fetch_resources_page` filters on one type —
        // so this is one command; grouping keeps a hypothetical
        // heterogeneous slice correct too), run against both collections so
        // stale contained rows don't outlive the page they belonged to
        // (#1160 Task 4). A failure on either delete means stale rows may
        // remain for the whole page, so it fans out to every resource.
        let mut ids_by_type: HashMap<&str, Vec<Bson>> = HashMap::new();
        for resource in resources {
            ids_by_type
                .entry(resource.resource_type())
                .or_default()
                .push(Bson::from(resource.id()));
        }
        let delete_started = Instant::now();
        for (resource_type, ids) in ids_by_type {
            let filter = doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "resource_id": { "$in": ids },
            };
            match collection.delete_many(filter.clone()).await {
                Ok(result) => stats.deleted_entries += result.deleted_count,
                Err(e) => {
                    stats.delete += delete_started.elapsed();
                    let msg = format!("Failed to delete search entries: {e}");
                    return resources
                        .iter()
                        .map(|_| Err(internal_error(msg.clone())))
                        .collect();
                }
            }
            match contained_collection.delete_many(filter).await {
                Ok(result) => stats.deleted_entries += result.deleted_count,
                Err(e) => {
                    stats.delete += delete_started.elapsed();
                    let msg = format!("Failed to delete search_index_contained entries: {e}");
                    return resources
                        .iter()
                        .map(|_| Err(internal_error(msg.clone())))
                        .collect();
                }
            }
        }
        let delete_time = delete_started.elapsed();
        stats.delete += delete_time;
        crate::perf::record_duration(crate::perf::Phase::ReindexSearchDelete, delete_time);

        // Flatten every resource's own documents into one insert, chunked at
        // SEARCH_INDEX_INSERT_CHUNK, tracking which resource each document
        // belongs to so an unordered write error attributes back to just
        // that resource instead of failing the whole page.
        let mut own_owners: Vec<usize> =
            Vec::with_capacity(prepared.iter().map(|p| p.docs.own.len()).sum());
        let mut own_docs: Vec<Document> = Vec::with_capacity(own_owners.capacity());
        for (i, p) in prepared.iter().enumerate() {
            for d in &p.docs.own {
                own_owners.push(i);
                own_docs.push(d.clone());
            }
        }

        let insert_started = Instant::now();
        let own_result = insert_search_entries_chunk(
            &collection,
            &own_owners,
            &own_docs,
            "Failed to insert search index entries",
            stats,
        )
        .await;
        let mut insert_time = insert_started.elapsed();
        stats.insert += insert_time;
        let mut insert_failures = match own_result {
            Ok(failures) => failures,
            Err(msg) => {
                return resources
                    .iter()
                    .map(|_| Err(internal_error(msg.clone())))
                    .collect();
            }
        };

        // Same flatten-and-chunked-insert for contained rows, into their own
        // collection. A failed contained insert attributes back to its
        // resource exactly like a failed own insert; if a resource already
        // has an own-row failure recorded, that one wins (matching the
        // "first write error found" semantics `insert_search_entries_chunk`
        // already uses within one collection).
        let mut contained_owners: Vec<usize> =
            Vec::with_capacity(prepared.iter().map(|p| p.docs.contained.len()).sum());
        let mut contained_docs: Vec<Document> = Vec::with_capacity(contained_owners.capacity());
        for (i, p) in prepared.iter().enumerate() {
            for d in &p.docs.contained {
                contained_owners.push(i);
                contained_docs.push(d.clone());
            }
        }

        if !contained_docs.is_empty() {
            let contained_started = Instant::now();
            let contained_result = insert_search_entries_chunk(
                &contained_collection,
                &contained_owners,
                &contained_docs,
                "Failed to insert search_index_contained entries",
                stats,
            )
            .await;
            let contained_time = contained_started.elapsed();
            stats.insert += contained_time;
            insert_time += contained_time;
            match contained_result {
                Ok(failures) => {
                    for (owner, msg) in failures {
                        insert_failures.entry(owner).or_insert(msg);
                    }
                }
                Err(msg) => {
                    return resources
                        .iter()
                        .map(|_| Err(internal_error(msg.clone())))
                        .collect();
                }
            }
        }

        crate::perf::record_duration(crate::perf::Phase::ReindexSearchInsert, insert_time);
        crate::perf::add_rows(
            crate::perf::Phase::ReindexSearchInsert,
            (own_docs.len() + contained_docs.len()) as u64,
        );

        prepared
            .into_iter()
            .enumerate()
            .map(|(i, p)| match p.failure {
                Some(msg) => Err(internal_error(msg)),
                None => match insert_failures.remove(&i) {
                    Some(msg) => Err(internal_error(msg)),
                    None => Ok(p.docs.own.len() + p.docs.contained.len()),
                },
            })
            .collect()
    }

    /// Delegates to [`Self::write_search_entries_page_timed`] with a
    /// throwaway `ReindexPageStats`, so the two cannot diverge (#1403).
    async fn write_search_entries_page(
        &self,
        tenant: &TenantContext,
        resources: &[StoredResource],
    ) -> Vec<StorageResult<usize>> {
        let mut stats = ReindexPageStats::default();
        self.write_search_entries_page_timed(tenant, resources, &mut stats)
            .await
    }
```

`write_search_entries` (unchanged) still calls `write_search_entries_page`, which now delegates.

Change `insert_search_entries_chunk` to take and update `stats`:

```rust
async fn insert_search_entries_chunk(
    collection: &mongodb::Collection<Document>,
    owners: &[usize],
    docs: &[Document],
    error_context: &str,
    stats: &mut ReindexPageStats,
) -> Result<HashMap<usize, String>, String> {
    let mut insert_failures: HashMap<usize, String> = HashMap::new();
    let mut offset = 0usize;
    for chunk in docs.chunks(SEARCH_INDEX_INSERT_CHUNK) {
        stats.insert_commands += 1;
        stats.inserted_entries += chunk.len() as u64;
        match collection.insert_many(chunk).ordered(false).await {
            Ok(_) => {}
            Err(e) => match e.kind.as_ref() {
                MongoErrorKind::InsertMany(insert_many) => {
                    let Some(write_errors) = insert_many.write_errors.as_ref() else {
                        return Err(format!("{error_context}: {e}"));
                    };
                    for write_error in write_errors {
                        let owner = owners[offset + write_error.index];
                        insert_failures
                            .entry(owner)
                            .or_insert_with(|| format!("{error_context}: {}", write_error.message));
                    }
                }
                _ => return Err(format!("{error_context}: {e}")),
            },
        }
        offset += chunk.len();
    }
    Ok(insert_failures)
}
```

Update its doc comment: "...and counts each command it issues, and the documents in it, into `stats` (#1403)."

In `perf.rs`, update the three doc comments (code is unchanged):

```rust
    /// Extract and marshal every resource in a PostgreSQL or MongoDB reindex page.
    ReindexExtract,
    /// Grouped delete from `search_index` (MongoDB: and `search_index_contained`)
    /// for a PostgreSQL or MongoDB reindex page.
    ReindexSearchDelete,
    /// Grouped delete from `resource_fts` for a PostgreSQL reindex page.
    ReindexFtsDelete,
    /// Batched insert into `search_index` (MongoDB: and `search_index_contained`)
    /// for a PostgreSQL or MongoDB reindex page.
    ReindexSearchInsert,
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
docker info >/dev/null 2>&1 || [ -n "$HFS_TEST_MONGODB_URL" ] || { echo "Docker/HFS_TEST_MONGODB_URL unavailable: this step cannot be verified"; exit 1; }
cargo test -p helios-persistence --features mongodb --test mongodb_tests -- reindex_page --nocapture 2>&1 | tee /tmp/pr0-mongo-phase-stats-green.log
! grep -q '^Skipping' /tmp/pr0-mongo-phase-stats-green.log
```
Expected: `test result: ok.` including both `mongodb_integration_reindex_page_reports_phase_stats` and the pre-existing `mongodb_integration_reindex_page_counts_contained_entries` (unchanged behaviour — same reported counts as before), and no `Skipping` line in the log — a skip would make `test result: ok.` vacuous, not a real green.

- [ ] **Step 5: Full-crate compile check, clippy, fmt, commit**

```bash
cargo check -p helios-persistence --all-features --tests
cargo check -p helios-hfs
cargo clippy -p helios-persistence --all-features --tests -- -D warnings -A clippy::items_after_test_module -A clippy::large_enum_variant -A clippy::question_mark -A clippy::collapsible_match -A clippy::collapsible_if -A clippy::field_reassign_with_default -A clippy::doc-overindented-list-items -A clippy::doc-lazy-continuation
rustfmt --edition 2024 crates/persistence/src/backends/mongodb/storage.rs crates/persistence/src/perf.rs crates/persistence/tests/mongodb_tests.rs
git add crates/persistence/src/backends/mongodb/storage.rs crates/persistence/src/perf.rs crates/persistence/tests/mongodb_tests.rs
git commit -m "$(cat <<'EOF'
feat(mongodb): report reindex writer-phase timing through ReindexPageStats (#1403 PR0)

write_search_entries_page is renamed to write_search_entries_page_timed
and instrumented phase by phase (extract/delete/insert), forwarding to
crate::perf as PostgreSQL already does. A new thin write_search_entries_page
delegates to it with a throwaway ReindexPageStats, so the driver's path and
the composite ingest sink's direct call can never diverge. No command,
argument, order or returned result changes.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: SKILL doc bullets (`.claude` and `.agents`)

**Files:**
- Modify: `.claude/skills/bulk-data-submit/SKILL.md` — insert one bullet immediately after the bullet beginning "A deferred rebuild generation whose resource errors include *transient* ones..." (currently line 149).
- Modify: `.agents/skills/bulk-data-submit/SKILL.md` — insert the identical bullet immediately after the equivalent bullet beginning "A clean automatic generation ends `Completed`..." (currently line 134), per S1 §5.6's cross-copy rule: every PR in this campaign that edits the `.claude` copy makes the same edit in the `.agents` copy wherever that paragraph exists there.

**Interfaces:** none — documentation only.

- [ ] **Step 1: Insert the bullet in the `.claude` copy**

In `.claude/skills/bulk-data-submit/SKILL.md`, directly after the line ending "...A SQLite row the source cannot parse is recorded as a permanent error for that resource instead of silently ending the rebuild of its type." (the bullet at line 149), add:

```markdown
- The rebuild logs `reindex job started` / `reindex type started` / `reindex type finished` / `reindex job finished` at INFO, `reindex progress` every ≥60 s, and `reindex page` at DEBUG (`RUST_LOG=…,helios_persistence::search::reindex=debug`). Per-type lines carry type-scoped fetch/write/extract/delete/insert/yield milliseconds and `deleted`/`inserted`/`insert_commands` in the release build; join to the generation by `job_id` (#1403). The field contract is on the log helpers in `search/reindex.rs`.
```

- [ ] **Step 2: Insert the identical bullet in the `.agents` copy**

In `.agents/skills/bulk-data-submit/SKILL.md`, directly after the bullet ending "...Independently queued work that arrived during the retry still runs as a new generation with its own retry budget. Cancellation does not retry the cancelled active types, but independently queued pending types still run after the cancelled task has stopped writing." (line 134), add the same bullet, verbatim:

```markdown
- The rebuild logs `reindex job started` / `reindex type started` / `reindex type finished` / `reindex job finished` at INFO, `reindex progress` every ≥60 s, and `reindex page` at DEBUG (`RUST_LOG=…,helios_persistence::search::reindex=debug`). Per-type lines carry type-scoped fetch/write/extract/delete/insert/yield milliseconds and `deleted`/`inserted`/`insert_commands` in the release build; join to the generation by `job_id` (#1403). The field contract is on the log helpers in `search/reindex.rs`.
```

- [ ] **Step 3: Verify the two copies are byte-identical for this bullet**

```bash
diff <(grep -F "The rebuild logs \`reindex job started\`" .claude/skills/bulk-data-submit/SKILL.md) \
     <(grep -F "The rebuild logs \`reindex job started\`" .agents/skills/bulk-data-submit/SKILL.md)
```
Expected: no output.

- [ ] **Step 4: Commit**

```bash
git add .claude/skills/bulk-data-submit/SKILL.md .agents/skills/bulk-data-submit/SKILL.md
git commit -m "$(cat <<'EOF'
docs(bulk-data-submit): document the reindex instrumentation log lines (#1403 PR0)

Mirrors the same bullet into both the .claude and .agents copies per the
repo's cross-copy rule for this skill.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 6: Pre-arm smoke check and opening the PR

**Files:** none — verification and `gh` commands only.

**Interfaces:** none.

- [ ] **Step 1: Full crate verification, one more time, across every task's changes together**

```bash
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
cargo check -p helios-persistence --all-features --tests
cargo check -p helios-hfs
cargo clippy -p helios-persistence --all-features --tests -- -D warnings -A clippy::items_after_test_module -A clippy::large_enum_variant -A clippy::question_mark -A clippy::collapsible_match -A clippy::collapsible_if -A clippy::field_reassign_with_default -A clippy::doc-overindented-list-items -A clippy::doc-lazy-continuation
cargo test -p helios-persistence --lib -- search::reindex_stats search::reindex
docker info >/dev/null 2>&1 || [ -n "$HFS_TEST_MONGODB_URL" ] || { echo "Docker/HFS_TEST_MONGODB_URL unavailable: this step cannot be verified"; exit 1; }
cargo test -p helios-persistence --features mongodb --test mongodb_tests -- reindex_page --nocapture 2>&1 | tee /tmp/pr0-mongo-phase-stats-task6.log
! grep -q '^Skipping' /tmp/pr0-mongo-phase-stats-task6.log
git status --short crates/persistence .claude/skills/bulk-data-submit/SKILL.md .agents/skills/bulk-data-submit/SKILL.md
```
Expected: clean clippy; every `reindex_stats`/`reindex`/`reindex_page` test green (quote the actual `test result:` lines, not exit codes); no `Skipping` line in the Mongo log; `git status --short` reports nothing outstanding (everything from Tasks 1–5 is already committed).

- [ ] **Step 2: Pre-arm smoke check (S1 §8.3 and §10 — feeds the OBS-26 runbook that S4 owns)**

This is the guard against a writer-phase wiring regression (S1 §10's "default-method trap": an override of `_timed` whose `write_search_entries_page` does not delegate to it, or a build that silently picked up the trait default) reaching a multi-hour OBS-26 baseline undetected. Run every command below from the repo root, checking the lock first:
```bash
test ! -e /c/Users/DougC/Code/Helios/manual-test/bench-1403/ARM_RUNNING.lock || { echo "OBS-26 arm running: stop and report"; exit 1; }
```

Build the exact frozen bench binary S4 §4.12.3 builds for every arm — `--no-default-features` (the default `ui` feature is not part of the bench feature set), `--locked`, no `RUSTFLAGS` — into an isolated target dir so this never touches `hfs\target\release\hfs.exe` (the 0.2.3 binary the #940 row still uses, S4 §4.12.3):
```bash
CARGO_BUILD_JOBS=4 CARGO_TARGET_DIR=/c/Users/DougC/Code/Helios/target-pr0-smoke \
  cargo build --locked --release -p helios-hfs --bin hfs --no-default-features --features R4,sqlite,mongodb
```

Start a disposable MongoDB on a port distinct from S4's bench ports (27018/18403/8080), and the built binary against it:
```bash
docker run -d --rm --name hfs1403-pr0-smoke -p 27118:27017 mongo:7.0
mkdir -p /tmp/pr0-smoke
HFS_STORAGE_BACKEND=mongodb \
HFS_MONGODB_URL=mongodb://localhost:27118 \
HFS_MONGODB_DATABASE=pr0smoke \
HFS_SERVER_PORT=8099 \
RUST_LOG=info,hfs=warn,hfs_perf=info,helios_persistence::search::reindex=debug \
/c/Users/DougC/Code/Helios/target-pr0-smoke/release/hfs.exe > /tmp/pr0-smoke/server.log 2>&1 &
HFS_SMOKE_PID=$!
```

Poll for readiness, create 5 Patients, and kick a manual reindex:
```bash
for i in $(seq 1 60); do curl -sf http://localhost:8099/metadata >/dev/null 2>&1 && break; sleep 2; done
for i in 1 2 3 4 5; do
  curl -s -X POST -H 'Content-Type: application/fhir+json' \
    -d '{"resourceType":"Patient","name":[{"family":"Smoke"}]}' \
    http://localhost:8099/Patient >/dev/null
done
kickoff=$(curl -s -X POST 'http://localhost:8099/$reindex')
job=$(printf '%s' "$kickoff" | grep -oE '"valueString"[[:space:]]*:[[:space:]]*"[^"]*"' | head -1 | sed -E 's/.*"([^"]*)"$/\1/')
echo "job=$job"
```
(`$reindex`'s 202 body is `Parameters{parameter:[{"name":"jobId","valueString":<job>}, {"name":"status","valueString":"queued"}]}` — `crates/rest/src/handlers/reindex.rs` around its `StatusCode::ACCEPTED` response — so the first `valueString` in the body is the job id.)

Poll until the job is terminal, then check the log:
```bash
for i in $(seq 1 60); do
  st=$(curl -s "http://localhost:8099/\$reindex-status/$job" | grep -oE '"valueCode"[[:space:]]*:[[:space:]]*"[^"]*"' | head -1 | sed -E 's/.*"([^"]*)"$/\1/')
  [ "$st" = "completed" ] && break
  sleep 2
done
grep 'reindex type finished' /tmp/pr0-smoke/server.log | grep -E 'inserted=[1-9]' | grep -E 'insert_commands=[1-9]'
```
Expected: at least one match (`grep`'s own exit code 0). If `inserted=0`/`insert_commands=0` here, or no `reindex type finished` line appears at all, stop and re-check Task 4 before running anything longer.

Clean up unconditionally, whether the check passed or failed:
```bash
kill "$HFS_SMOKE_PID" 2>/dev/null
docker stop hfs1403-pr0-smoke
rm -rf /c/Users/DougC/Code/Helios/target-pr0-smoke
```

- [ ] **Step 3: Open the PR (do not merge)**

```bash
git push -u origin perf/1403-mongodb-reindex-rebuild
gh pr create --base main --head perf/1403-mongodb-reindex-rebuild \
  --title "perf(mongodb): reindex instrumentation, no behaviour change (#1403 PR0)" \
  --body "$(cat <<'EOF'
## Summary
- Adds always-on wall-clock timing to `run_reindex` (job/type/progress/page)
  and a new `ReindexTarget::write_search_entries_page_timed` provided method
  so MongoDB's writer reports its own extract/delete/insert phases and counts.
- No behaviour change: what is written, in what order, at what page size,
  error attribution, retries, `$reindex-status`'s output and index specs are
  all unchanged. Every other `ReindexTarget` implementor inherits the
  default, which delegates and leaves stats untouched.
- Six fixed-format log lines under `helios_persistence::search::reindex`
  (`reindex job started` / `type started` / `type finished` / `progress` /
  `job finished` at INFO, `reindex page` at DEBUG) give OBS-26 and the
  eventual T3-full run the per-type, per-page numbers Gate 0 needs. This is
  PR0 of the #1403 campaign; it ships no walk change, no byte cap and no
  overlap (those are PR1 / PR2a / PR2b).

## Design
- Spec: [`docs/superpowers/specs/2026-09-23-mongodb-reindex-rebuild-design.md`](../blob/main/docs/superpowers/specs/2026-09-23-mongodb-reindex-rebuild-design.md) (§4.1, §9)
- File-level design: `manual-test/archive/1403-run17-evidence/design/S1-pr0-instrumentation.md` — local evidence only, not committed to this repo; SHA-256 `96f17650bdcc4d50e33049a53b08b238ad154614723d40aae701472a9e92b493` (recorded in the plan this PR implements, `docs/superpowers/plans/2026-09-23-1403-pr0-reindex-instrumentation.md`).

## Gate (SMOKE-1)
Pending — this PR's merge gate is SMOKE-1 (spec §4.7; S4 §4.4), run by the orchestrator on this branch. Its `results/SMOKE-1__*.md` table is pasted here with `gh pr edit` once that run completes; it is not part of this PR's test plan below, which the implementer runs directly.

## Test plan
- [x] `cargo test -p helios-persistence --lib -- search::reindex_stats search::reindex`
- [x] `cargo test -p helios-persistence --features mongodb --test mongodb_tests -- reindex_page` (Docker; verified non-skipped, see Task 4/6)
- [x] `cargo clippy -p helios-persistence --all-features --tests -- -D warnings` (repo's standard `-A` allowances)
- [x] Pre-arm smoke check: a tiny rebuild's `reindex type finished` line shows `inserted > 0` and `insert_commands > 0`

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

Do not merge. This PR's merge gate is SMOKE-1 (spec §4.7, S4 §4.4), which the orchestrator runs on this branch after it opens; Gate 0 (B0-s, spec §5.2) runs on `main` + this PR afterward and does not wait for PR1.

---

## Self-Review

**Review response (this revision, 2026-09-23).** Every blocker, major and minor issue from the code review was applied; none was rejected.
- Blockers: the cancelled-run test (and, defensively, the failed-page test) now filters to `tracing::Level::INFO` before indexing by position, so the DEBUG `reindex page` line can no longer land on `events[2]`/`events[3]` and panic a `HashMap` index (Task 3 Step 1b). `reindex_stats.rs` now carries a scoped `#![allow(dead_code)]` from the moment Task 2 creates it, deleted by name in Task 3 Step 3d once `run_reindex` reads every item — `cargo clippy --tests -D warnings` no longer fails deterministically on Task 2's own commit.
- Majors: every `2>/dev/null` was removed from every cargo/test/build/clippy invocation (Global Constraints, and every task's verification steps) so compile errors and clippy diagnostics are visible, per the "expected" text each step already asks the implementer to read. MongoDB integration test steps (Task 4 Steps 2 and 4, Task 6 Step 1) now preflight Docker/`HFS_TEST_MONGODB_URL`, run with `--nocapture`, and `grep -q '^Skipping'` the saved log so a skip fails the step instead of reading as a pass. A new Global Constraint adds the `ARM_RUNNING.lock` check (S4 §4.5.1, §4.12.3) ahead of every cargo/test/build command in the plan. Task 6 Step 2 is rewritten with concrete, runnable commands matching S4 §4.12.3's frozen bench build (`--no-default-features --locked`, an isolated `CARGO_TARGET_DIR`, no `RUSTFLAGS`) instead of the previous default-featured build that would have touched `hfs\target\release\hfs.exe` and silently added `ui`; it now stands up a disposable Mongo container, a Patient, a real `$reindex` kick-off/poll cycle, and a concrete `grep` check, with explicit cleanup. All six `log_*` helpers (Task 3 Step 3d) now carry the doc comments that Task 2's module doc, Task 3's SKILL bullet reference, and S1 §4's field-meaning table promised but that did not previously exist anywhere in the plan.
- Minors: the MongoDB `write_search_entries_page_timed` replacement listing (Task 4 Step 3) now keeps every existing inline rationale comment word for word, and moves `_page_span` to directly after the empty-resources guard, before the offloaded guard, matching S1 §5.4 exactly (previously it sat after the offloaded guard). The `write_resource_batch` replacement (Task 3 Step 3b) now keeps its existing doc comment and `#[allow(clippy::too_many_arguments)]`. The four prose-only edits to `run_reindex` (Task 3 Step 3c, formerly step (e) items 3/5/9/11) now show literal code, including the full paging-path block with both accounting formulas (`resources = page.resources.len() + page.skipped.len()`, `failed = page.skipped.len() + batch_outcome.failed`). Task 6's closing note and PR body no longer claim Gate 0 runs "after PR1 lands"; they cite SMOKE-1 as this PR's actual merge gate (spec §4.7, S4 §4.4) and add a pending `## Gate (SMOKE-1)` section, a markdown link for the spec, and mark the S1 design file as uncommitted local evidence with its SHA-256. The three line-number hints that didn't match HEAD `c86d0f08b` are corrected (`SkippedResource` closes at 118, not 123; `write_search_entries_page` closes at 323 and the trait at 324, not 322/323; `search/mod.rs`'s `pub use reindex::{...}` is at 116–120, not 113–117) — verified directly against the files in this revision. Task 3's Interfaces now say `ReindexTarget::write_search_entries_page_timed` (a trait method, not a method on the `ReindexPageStats` struct) and drop `Counters` from its `Consumes` list with a note on how it is actually reached. Task 4 Step 2's "expected" text no longer implies asserts run after a panic. Task 3's Steps 1 and 3 are split into 1a/1b and 3a/3b/3c/3d, each independently reviewable. Steps 1a/1b's tests call `exit_outcome`/`job_outcome`/`MeasuringTarget`'s `_timed` override, which are not defined until Step 3d, so no green `cargo test` is achievable before 3d completes — claiming one at 3b would have been dishonest, and an earlier draft of this revision did exactly that before being corrected. Step 3b instead gets a real, achievable checkpoint: `cargo check -p helios-persistence --lib` (no `--tests`, so `#[cfg(test)] mod tests` and its not-yet-defined symbols are excluded entirely), confirming Step 3b's own new code type-checks. Step 3a also fixes a genuine compile hazard the split would otherwise have introduced: `start_tracked`'s call site does not pass the new `progress_interval` argument to `run_reindex` until Step 3c changes the signature in the same edit, so the crate never sits in an uncompilable "extra argument, old signature" state. Step 3c (`run_reindex` wiring) gets no standalone checkpoint, since its helpers and imports (Step 3d) are added afterward; Step 4, after 3d, is the first point at which everything — including the test module — compiles and every new test passes.
- Nothing was rejected: every issue's evidence (checked against the actual files in this repo and the S1/S4 design sections during this revision) held up.

**Spec coverage.**
- Spec §4.1 "always-on `Instant` timing... measurement of record": Task 3 Steps 3a (`ReindexOperation`'s new field) and 3c (`run_started`, item 2), Global Constraints.
- Spec §4.1 new `write_search_entries_page_timed` + `ReindexPageStats` (`#[non_exhaustive]`): Task 1.
- Spec §4.1 log contract table (L1–L6, exact field order, soft 32-field budget, sentinel `-`): Task 3 Step 3d (the six `log_*` functions, now each with its own field-order/scope/formula doc comment) and the `assert_contract` harness (Step 1a) that pins field order at runtime.
- Spec §4.1 "MongoDB overrides `_timed` and makes `write_search_entries_page` delegate to it, as any override must": Task 4, guarded by the `&dyn ReindexTarget` calls in the Task 4 integration test (now with a Docker/skip preflight so that guard is actually exercised) and the default-only Task 1 test.
- Spec §6 "the knobs section lands with PR2a/PR3" / SKILL edits mirrored `.claude`→`.agents`: Task 5.
- Spec §9 "What PR0 lets us measure" (quartile rates, Gate 0 signature, `t_hfs`, delete cost, yield overhead): all fed by the L3/L4/L6 fields Task 3 emits; no separate task needed since it's a read of the log contract, not new code.
- S1 §2 "What HEAD has today" facts (line numbers, existing behaviour of `write_resource_batch`, `run_reindex`, MongoDB's writer): verified against the actual file at HEAD `c86d0f08b` while drafting Tasks 1, 3 and 4, and re-verified in this revision for the three corrected line-number hints (see the anchors quoted in each task's **Files** section); no further material drift found.
- S1 §3 D1–D15 (engineering decisions): D1/D2 → Task 1; D3/D4 → Task 3's `REINDEX_PROGRESS_INTERVAL`/`reindex page`; D5 (`$reindex-status` untouched) → no task touches it, confirmed by Task 3 not modifying `ReindexProgress`/`to_parameters`; D6 (target, `%` strings, sentinel) → the `log_*` bodies and their new doc comments; D7 (no `generation` field) → the field lists omit it; D8 (MongoDB-only) → Global Constraints and Task 3 (every other target reports zero); D9 (type- vs job-scoped counters, the "major defect" regression) → Task 3's first driver-log test, its doc comment on `log_progress` now states the scope rule explicitly; D10 (L5 outcome race) → `job_outcome`/`log_job_end` and their doc comments, tested in `job_outcome_prefers_a_terminal_status_already_written`; D11 (truncated runs, no synthetic L5) → Task 3 Step 3c items 5/11/12/13 only log on paths that return, panics stay uncovered by design; D12 (fetch not split) → `fetch_ms` stays one phase in `PageRecord`; D13 (`insert_commands` counted, deletes not) → `ReindexPageStats`/`insert_search_entries_chunk`; D14 (32-field soft budget) → Global Constraints, verified by field-count arithmetic during drafting (L4 = 28 data fields + `message` = 29); D15 (`RUST_LOG` value) → Task 6 Step 2 (now using the exact recommended value against a real running server).
- S1 §5 code changes: §5.1(a)–(g) → Task 1 + Task 3; §5.1(h) (field-meaning doc comments on the `log_*` helpers, which PR2b needs to update) → Task 3 Step 3d, now written out in full instead of only referenced; §5.2 → Task 2, including its "every field and function must be read somewhere" clippy rule → Task 2's temporary `#![allow(dead_code)]`, deleted in Task 3 Step 3d; §5.3 → Task 2; §5.4 → Task 4, including "only these additions are made" → the restored inline rationale comments and the corrected `_page_span` placement; §5.5 → Task 4; §5.6 → Task 5. §5's "Files that change" list matches this plan's File Structure table exactly (the table now also lists `reindex_stats.rs` under Task 3, for the allow-removal), including the one new MongoDB integration test and the two SKILL copies; `docs/mongodb-reindex-benchmark.md` is confirmed out of scope for PR0 (S1 §5, spec §6) and no task creates it.
- S1 §8 tests: §8.1 (10 unit tests) → Task 2 Step 1; §8.2 (2 plain unit tests + capture harness + 5 driver-log tests) → Task 3 Steps 1a/1b; §8.3 (4-step MongoDB integration test, `&dyn ReindexTarget`, offloaded skip) → Task 4 Step 1, its skip-verification behaviour → Task 4 Steps 2/4; §8.3's pre-arm smoke check → Task 6 Step 2, now runnable end to end; §8.4 commands → reused verbatim (minus `2>/dev/null`) across every task's verification steps and Global Constraints.
- S1 §9/§10 risks (busy vs. wall time, default-method trap, composite deployments, capture-subscriber runtime requirement, partial move of `request`, L5 race, truncated runs): each is either a design constraint already respected by the code as written (partial move → Task 3 Step 3c item 4's "never `&request`" note; current-thread runtime → Task 3's tests are plain `#[tokio::test]`, matching S1's stated requirement) or explicitly called out in Global Constraints / the relevant task.
- S4 §4.5.1 / §4.12.3 "no cargo during arms" (`ARM_RUNNING.lock`): a new Global Constraint, checked ahead of every cargo/test/build command in the plan.
- S4 §4.12.3 build procedure (frozen bench feature set, `--no-default-features --locked`, isolated `CARGO_TARGET_DIR`, `RUSTFLAGS` never set, `hfs\target\release\hfs.exe` never touched): Task 6 Step 2.
- S4 §4.4/§4.7 (SMOKE-1 is PR0's actual merge gate; Gate 0 runs on `main` + PR0 afterward, not gated on PR1): Task 6 Step 3's closing note and the PR body's new `## Gate (SMOKE-1)` section.

**Placeholder scan.** No "TBD", "TODO", "add error handling", "similar to Task N", or "write tests for the above" appears anywhere above. Every checkbox step that changes behaviour shows the literal code — including, after this revision, the four edits to `run_reindex` that were previously prose only (Task 3 Step 3c items 3/5/9/11: the count-loop block, the two `clear_search_index`/`begin_bulk_index_rebuild` failure inserts, the full paging-path block, and the `end_bulk_index_rebuild` failure insert) and the `write_resource_batch`/MongoDB listings that previously dropped existing doc comments, attributes or inline rationale comments. Every type and function referenced (`ReindexPageStats`, `ReindexRunStats`, `Counters`, `PhaseMillis`, `BatchOutcome`, `TenantSearchRegistries::base_only`, `PagedSource`, `RecordingTarget`, `ControlledBackend`, `FailingPageSource`, `CountingTarget`, `TimedPageSource`, `recording_operation`, `await_finished`, `named_tenant`, `next_controlled_event`, `create_backend_with_full_registry`, `create_backend_with_search_offloaded`, `search_index_entry_count`, `raw_test_client`, `create_tenant`) is either defined in this plan's own tasks or already exists at HEAD `c86d0f08b` — confirmed by reading each one's definition in the repository before citing it (`crates/persistence/src/search/reindex.rs`, `crates/persistence/tests/mongodb_tests.rs`).

**Type consistency.** `ReindexPageStats` (Task 1: `extract, delete, insert: Duration; deleted_entries, inserted_entries, insert_commands: u64`) is the exact shape Task 2's `PageRecord`/`Counters` embed, Task 3's `log_page`/`log_type_finished`/etc. read (`.writer.deleted_entries` etc.), and Task 4's MongoDB override populates field-by-field — no task invents a different field name or type for it. `BatchOutcome { entries, failed: u64; write: Duration; writer: ReindexPageStats }` (Task 3) is what `record_and_log_page`'s `PageRecord` literal is built from in both the named and paging paths, now shown as literal code in both places. `ReindexRunStats::record_page(&mut self, page: PageRecord, now: Instant) -> RecordedPage` and `::finish_type(&mut self, outcome: &'static str, now: Instant) -> Option<TypeSummary>` (Task 2) are called with exactly that signature from Task 3's `record_and_log_page`/Step 3c item 10/`log_job_end`. `write_search_entries_page_timed(&self, tenant: &TenantContext, resources: &[StoredResource], stats: &mut ReindexPageStats) -> Vec<StorageResult<usize>>` (Task 1's trait default, on `ReindexTarget` — not a method of the `ReindexPageStats` struct, corrected in Task 3's Interfaces this revision) is the signature Task 3's `MeasuringTarget` and Task 4's `MongoBackend` both override identically. `insert_search_entries_chunk`'s new trailing `stats: &mut ReindexPageStats` parameter (Task 4) matches both of its call sites, updated in the same task. `Counters` (Task 2) is never imported directly into `reindex.rs`; it is reached only through `TypeSummary.counters`, `ProgressSnapshot.type_counters` and `JobSummary.counters`, which Task 3's import list correctly reflects.
