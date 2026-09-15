//! Opt-in phase timing for the ingest write path (#947).
//!
//! The bulk-submit ingest path is a long chain of small steps — parse a line,
//! probe for an existing resource, write the resource row, write its history
//! copy, extract search values, write index rows, write the FTS row, write two
//! bookkeeping rows, commit the batch. Wall-clock rate alone cannot say which
//! of those to attack, and #947 measured ~84% of the per-entry budget as
//! unattributed. This module attributes it.
//!
//! Design constraints:
//!
//! * **Absent unless asked for.** The whole thing is behind `--cfg
//!   perf_phases`, *not* a cargo feature. Release artifacts are built with
//!   `cargo build --workspace --all-features --release` (`ci.yml`, the `build`
//!   job whose output the `release` job publishes and the Docker images copy),
//!   and `--all-features` enables every feature there is — so a feature could
//!   not have kept this out of a shipped binary. A `cfg` flag can, for the
//!   same reason `tokio_unstable` is one. Without it [`enabled`] is a compile
//!   time `false`, every guard folds away, and the counters are never
//!   referenced.
//!
//!   ```text
//!   RUSTFLAGS='--cfg perf_phases' cargo run --release -p helios-persistence \
//!       --example bulk_submit_bench -- --limit 25000 CarePlan.ndjson
//!   ```
//!
//! * **Zero cost when built in but switched off.** Every call site starts with
//!   one relaxed atomic load. `HFS_PERF_PHASES=1` (read once, at first use)
//!   turns collection on; with it unset the guards return `None` and no clock
//!   is read.
//! * **Process-global, lock-free.** Counters are plain `AtomicU64` pairs
//!   (nanos, hits) indexed by phase, so instrumented code can sit inside a
//!   `&self` method on a shared backend without threading a profiler handle
//!   through every signature.
//! * **Periodic, cumulative dumps.** The SQLite streaming ingest calls
//!   [`ingest_progress`] after every batch and logs the table it returns —
//!   once per [`PROGRESS_INTERVAL`] resources, cumulative since process start
//!   — as an `info` event on the `hfs_perf` target, so a server run can be
//!   read without a bench harness:
//!
//!   ```text
//!   RUSTFLAGS='--cfg perf_phases' cargo build --release --bin hfs
//!   HFS_PERF_PHASES=1 HFS_BULK_SUBMIT_DEFER_INDEXING=false ./target/release/hfs
//!   ```
//!
//! * **Explicit nesting.** Phases are recorded as measured, so a phase that
//!   encloses another double-counts by design. [`Phase::nested_in`] declares
//!   the containment, and the report renders children indented under their
//!   parent instead of pretending the columns sum to the wall clock.
#[cfg(perf_phases)]
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// How many ingested resources between two periodic reports from
/// [`ingest_progress`].
pub const PROGRESS_INTERVAL: u64 = 1_000;

/// One measured step of the ingest write path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum Phase {
    /// `serde_json` parse of one NDJSON line into a `Value`.
    NdjsonParse = 0,
    /// The whole per-entry pipeline, from parsed entry to bookkeeping rows.
    Entry,
    /// Read-before-write: does a resource with this id already exist?
    EntryRead,
    /// `Transaction::create` in full.
    Create,
    /// The `SELECT 1 FROM resources` existence probe inside `create`.
    CreateExists,
    /// Deep clones of the parsed `serde_json::Value` on the create path: the
    /// entry's copy handed to `create`, and `create`'s own copy that receives
    /// `id`/`resourceType`. The SQLite ingest path never round-trips through
    /// the typed FHIR model (#947 item 5), so this is the whole cost of
    /// "re-materialising" the resource between parse and serialize.
    EntryClone,
    /// `serde_json::to_vec` of the resource being stored.
    Serialize,
    /// `INSERT INTO resources`.
    ResourceInsert,
    /// `INSERT INTO resource_history` — a second full copy of the blob.
    HistoryInsert,
    /// `Transaction::update` in full.
    Update,
    /// `UPDATE resources` on the update path.
    ResourceUpdate,
    /// Indexing in full: delete + extract + index rows + FTS.
    Index,
    /// `DELETE FROM search_index` (plus the FTS delete when rows were removed).
    IndexDelete,
    /// FHIRPath-driven search value extraction.
    Extract,
    /// `INSERT INTO search_index`, all rows for one resource.
    IndexInsert,
    /// The Rust-side half of an index row: normalising the value and building
    /// the 24 bound parameters, as opposed to running the statement.
    IndexMarshal,
    /// FTS content extraction plus the `resource_fts` insert.
    Fts,
    /// The two bulk bookkeeping rows per entry, together.
    Bookkeeping,
    /// `INSERT INTO bulk_submission_changes` — the rollback record.
    BookkeepingChange,
    /// `INSERT OR REPLACE INTO bulk_entry_results` — the per-line receipt.
    BookkeepingResult,
    /// `COMMIT` of one batch transaction.
    Commit,
    /// Per-batch overhead outside the entry loop (BEGIN, manifest counters).
    BatchOverhead,
    /// Fetch one page of stored resources for a reindex job.
    ReindexFetch,
    /// One writer rebuilding one reindex page in full.
    ReindexPage,
    /// Acquire the PostgreSQL connection used by a reindex page.
    ReindexConnection,
    /// Extract and marshal every resource in a PostgreSQL reindex page.
    ReindexExtract,
    /// Grouped delete from `search_index` for a PostgreSQL reindex page.
    ReindexSearchDelete,
    /// Grouped delete from `resource_fts` for a PostgreSQL reindex page.
    ReindexFtsDelete,
    /// Batched insert into `search_index` for a PostgreSQL reindex page.
    ReindexSearchInsert,
    /// Rebuild full-text rows for a PostgreSQL reindex page.
    ReindexFts,
    /// Commit a PostgreSQL reindex page.
    ReindexCommit,
    /// Retry a failed PostgreSQL reindex page through the individual path.
    ReindexFallback,
    /// Wall clock of one batch's parallel extraction (`prepare_index_batch`),
    /// as opposed to `extract`, which sums the CPU time across the pool's
    /// threads. The gap between the two is the parallel speed-up.
    PrepareBatch,
}

impl Phase {
    /// All phases, in report order.
    pub const ALL: [Phase; 33] = [
        Phase::NdjsonParse,
        Phase::Entry,
        Phase::EntryRead,
        Phase::Create,
        Phase::CreateExists,
        Phase::EntryClone,
        Phase::Serialize,
        Phase::ResourceInsert,
        Phase::HistoryInsert,
        Phase::Update,
        Phase::ResourceUpdate,
        Phase::Index,
        Phase::IndexDelete,
        Phase::Extract,
        Phase::IndexInsert,
        Phase::IndexMarshal,
        Phase::Fts,
        Phase::Bookkeeping,
        Phase::BookkeepingChange,
        Phase::BookkeepingResult,
        Phase::Commit,
        Phase::BatchOverhead,
        Phase::ReindexFetch,
        Phase::ReindexPage,
        Phase::ReindexConnection,
        Phase::ReindexExtract,
        Phase::ReindexSearchDelete,
        Phase::ReindexFtsDelete,
        Phase::ReindexSearchInsert,
        Phase::ReindexFts,
        Phase::ReindexCommit,
        Phase::ReindexFallback,
        Phase::PrepareBatch,
    ];

    /// The phase this one is measured inside of, if any. Drives the report's
    /// indentation, and warns the reader that the two overlap.
    pub fn nested_in(self) -> Option<Phase> {
        match self {
            Phase::EntryRead | Phase::Create | Phase::Update | Phase::Bookkeeping => {
                Some(Phase::Entry)
            }
            Phase::ResourceUpdate => Some(Phase::Update),
            Phase::BookkeepingChange | Phase::BookkeepingResult => Some(Phase::Bookkeeping),
            Phase::EntryClone => Some(Phase::Create),
            // Indentation shows the create path, which is what a bulk load
            // runs. `serialize` and `index` are also reached from `update`,
            // and their counters cover both.
            Phase::CreateExists
            | Phase::Serialize
            | Phase::ResourceInsert
            | Phase::HistoryInsert
            | Phase::Index => Some(Phase::Create),
            Phase::IndexDelete | Phase::Extract | Phase::IndexInsert | Phase::Fts => {
                Some(Phase::Index)
            }
            Phase::IndexMarshal => Some(Phase::IndexInsert),
            Phase::ReindexConnection
            | Phase::ReindexExtract
            | Phase::ReindexSearchDelete
            | Phase::ReindexFtsDelete
            | Phase::ReindexSearchInsert
            | Phase::ReindexFts
            | Phase::ReindexCommit
            | Phase::ReindexFallback => Some(Phase::ReindexPage),
            _ => None,
        }
    }

    /// The label used in reports.
    pub fn label(self) -> &'static str {
        match self {
            Phase::NdjsonParse => "ndjson_parse",
            Phase::Entry => "entry (total)",
            Phase::EntryRead => "entry_read",
            Phase::Create => "create",
            Phase::Update => "update",
            Phase::CreateExists => "create_exists_probe",
            Phase::Serialize => "serialize",
            Phase::ResourceInsert => "resources_insert",
            Phase::HistoryInsert => "history_insert",
            Phase::ResourceUpdate => "resources_update",
            Phase::Index => "index (total)",
            Phase::IndexDelete => "index_delete",
            Phase::Extract => "extract",
            Phase::IndexInsert => "search_index_insert",
            Phase::IndexMarshal => "  (of which: marshal)",
            Phase::Fts => "fts",
            Phase::Bookkeeping => "bookkeeping (total)",
            Phase::BookkeepingChange => "submission_changes_insert",
            Phase::BookkeepingResult => "entry_results_insert",
            Phase::EntryClone => "entry_clone",
            Phase::Commit => "commit",
            Phase::BatchOverhead => "batch_overhead",
            Phase::ReindexFetch => "reindex_fetch",
            Phase::ReindexPage => "reindex_page (total)",
            Phase::ReindexConnection => "reindex_connection",
            Phase::ReindexExtract => "reindex_extract",
            Phase::ReindexSearchDelete => "reindex_search_delete",
            Phase::ReindexFtsDelete => "reindex_fts_delete",
            Phase::ReindexSearchInsert => "reindex_search_insert",
            Phase::ReindexFts => "reindex_fts",
            Phase::ReindexCommit => "reindex_commit",
            Phase::ReindexFallback => "reindex_fallback",
            Phase::PrepareBatch => "prepare_batch (wall)",
        }
    }
}

const PHASE_COUNT: usize = 33;

#[allow(clippy::declare_interior_mutable_const)]
const ZERO: AtomicU64 = AtomicU64::new(0);
static NANOS: [AtomicU64; PHASE_COUNT] = [ZERO; PHASE_COUNT];
static HITS: [AtomicU64; PHASE_COUNT] = [ZERO; PHASE_COUNT];
/// Rows written, for the phases where "how many" is the interesting number
/// (index rows per resource, above all).
static ROWS: [AtomicU64; PHASE_COUNT] = [ZERO; PHASE_COUNT];

#[cfg(perf_phases)]
static ENABLED: AtomicBool = AtomicBool::new(false);
#[cfg(perf_phases)]
static ENABLED_INIT: AtomicUsize = AtomicUsize::new(0);

/// Whether phase collection is on. Set by `HFS_PERF_PHASES` (`1`/`true`), read
/// once per process; [`set_enabled`] overrides it for in-process harnesses.
#[cfg(perf_phases)]
#[inline]
pub fn enabled() -> bool {
    if ENABLED_INIT.load(Ordering::Relaxed) == 0 {
        init_from_env();
    }
    ENABLED.load(Ordering::Relaxed)
}

/// Constant `false` in a build without `--cfg perf_phases`, which is every
/// build that is not explicitly a profiling one. Each call site is
/// `if !enabled() { return None; }`, so this folds the guard, the clock read,
/// and the counter update out of the binary — there is nothing left to switch
/// on, and `HFS_PERF_PHASES` is not read or even present in the executable.
#[cfg(not(perf_phases))]
#[inline(always)]
pub fn enabled() -> bool {
    false
}

#[cfg(perf_phases)]
#[cold]
fn init_from_env() {
    let on = std::env::var("HFS_PERF_PHASES")
        .map(|v| {
            let v = v.trim().to_ascii_lowercase();
            v == "1" || v == "true" || v == "yes" || v == "on"
        })
        .unwrap_or(false);
    ENABLED.store(on, Ordering::Relaxed);
    ENABLED_INIT.store(1, Ordering::Relaxed);
}

/// Turns collection on or off explicitly (benchmark harnesses, tests).
#[cfg(perf_phases)]
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
    ENABLED_INIT.store(1, Ordering::Relaxed);
}

/// No-op without `--cfg perf_phases`: there is no switch to throw, because
/// there are no call sites left to record. A harness that calls this and then
/// finds [`snapshot`] all zeros was built without the flag — see the module
/// docs for the invocation.
#[cfg(not(perf_phases))]
pub fn set_enabled(_on: bool) {}

/// A running phase measurement. Adds its elapsed time to the phase on drop.
pub struct Span {
    phase: Phase,
    start: Instant,
}

impl Drop for Span {
    fn drop(&mut self) {
        // Re-check the switch. A span can outlive it — collection is turned
        // off while this one is in flight — and "stop collecting" has to mean
        // that, or a sample lands after the caller believes recording has
        // stopped. The load is only reached when a `Span` exists at all, which
        // already implies collection was on.
        if !enabled() {
            return;
        }
        let idx = self.phase as usize;
        NANOS[idx].fetch_add(self.start.elapsed().as_nanos() as u64, Ordering::Relaxed);
        HITS[idx].fetch_add(1, Ordering::Relaxed);
    }
}

/// Starts timing `phase`, or returns `None` when collection is off (the guard
/// is `Option<Span>` so the disabled path reads no clock).
#[inline]
pub fn span(phase: Phase) -> Option<Span> {
    if !enabled() {
        return None;
    }
    Some(Span {
        phase,
        start: Instant::now(),
    })
}

/// Adds `rows` to a phase's row counter (index rows written, entries in a
/// batch, …). Cheap enough to leave unguarded, but guarded anyway.
#[inline]
pub fn add_rows(phase: Phase, rows: u64) {
    if !enabled() {
        return;
    }
    ROWS[phase as usize].fetch_add(rows, Ordering::Relaxed);
}

/// Resources the streaming ingest has reported through [`ingest_progress`],
/// process-wide and cumulative like every other counter here.
static INGESTED: AtomicU64 = AtomicU64::new(0);
/// Wall-clock nanoseconds spent inside the streaming ingest, summed over the
/// batches reported so far. Not "time since the first batch": that would count
/// the gaps between files (manifest fetch, download, lease bookkeeping) and
/// dilute every phase's share of a number the phases were never inside of.
static INGEST_WALL_NANOS: AtomicU64 = AtomicU64::new(0);
/// Counter values when the first streaming ingest began. Every phase counter
/// is process-global and the server has usually done indexed writes before
/// the first manifest arrives — seeding the spec SearchParameters alone is
/// ~1,400 autocommitted, indexed creates — so a report against the raw totals
/// charges that start-up work to the ingest, and a phase can show more time
/// than the ingest wall it is supposed to be a share of. Reports subtract
/// this baseline instead.
static INGEST_BASELINE: parking_lot::Mutex<Option<Vec<PhaseTotals>>> =
    parking_lot::Mutex::new(None);

/// Marks the start of the streaming ingest: the first call snapshots the
/// counters as the baseline that [`ingest_progress`] reports against; later
/// calls are no-ops, so concurrent or successive files share one baseline
/// and the report stays cumulative over the run. [`reset`] clears it.
///
/// Split on the cfg rather than guarded with `if !enabled()` so that a build
/// without `--cfg perf_phases` compiles only the empty stub: there is then no
/// baseline snapshot in the binary, and nothing for a coverage run to see as
/// an unreachable, never-hit body.
#[cfg(perf_phases)]
pub fn mark_ingest_start() {
    if !enabled() {
        return;
    }
    let mut baseline = INGEST_BASELINE.lock();
    if baseline.is_none() {
        *baseline = Some(snapshot());
    }
}

/// No-op without `--cfg perf_phases`.
#[cfg(not(perf_phases))]
pub fn mark_ingest_start() {}

/// Records one ingested batch — `resources` entries walked, `wall` the time
/// from the end of the previous batch (or the start of the stream) to the end
/// of this one, so that it covers line reading and parsing as well as the
/// batch transaction. Returns the cumulative [`report`] whenever the running
/// total crosses a multiple of [`PROGRESS_INTERVAL`], for the caller to log;
/// `None` otherwise, and always `None` when collection is off.
///
/// Cumulative by design: the number an operator reads at 50k resources is the
/// average over the whole run, which is what the issue-body tables in #947
/// quote, and a phase that is quadratic in the import (the FTS delete scan
/// was) shows up as a share that keeps climbing between dumps.
///
/// Split on the cfg (like [`mark_ingest_start`]) so a non-profiling build
/// carries only the `None` stub, keeping the boundary arithmetic and the
/// report render out of a coverage build that could never reach them.
#[cfg(perf_phases)]
pub fn ingest_progress(resources: u64, wall: Duration) -> Option<String> {
    if !enabled() {
        return None;
    }
    let wall_total = INGEST_WALL_NANOS.fetch_add(wall.as_nanos() as u64, Ordering::Relaxed)
        + wall.as_nanos() as u64;
    let before = INGESTED.fetch_add(resources, Ordering::Relaxed);
    let after = before + resources;
    if after / PROGRESS_INTERVAL == before / PROGRESS_INTERVAL {
        return None;
    }
    let wall_total = Duration::from_nanos(wall_total);
    let baseline = INGEST_BASELINE.lock();
    Some(match baseline.as_ref() {
        Some(before) => report_since(before, after, wall_total),
        None => report(after, wall_total),
    })
}

/// Always `None` without `--cfg perf_phases`.
#[cfg(not(perf_phases))]
pub fn ingest_progress(_resources: u64, _wall: Duration) -> Option<String> {
    None
}

/// Feeds one finished batch to [`ingest_progress`] and logs the breakdown it
/// returns on the `hfs_perf` target. Kept here, cfg-split, so the caller in
/// the ingest loop is one unconditional call with no `perf`-only branch of its
/// own for a coverage build to leave unhit.
#[cfg(perf_phases)]
pub fn log_ingest_progress(resource_type: &str, resources: u64, wall: Duration) {
    if let Some(report) = ingest_progress(resources, wall) {
        tracing::info!(
            target: "hfs_perf",
            resource_type,
            process_global = true,
            "ingest phase breakdown (cumulative)\n{report}"
        );
    }
}

/// No-op without `--cfg perf_phases`.
#[cfg(not(perf_phases))]
pub fn log_ingest_progress(_resource_type: &str, _resources: u64, _wall: Duration) {}

/// One phase's totals.
#[derive(Debug, Clone, Copy)]
pub struct PhaseTotals {
    /// The phase these totals belong to.
    pub phase: Phase,
    /// Accumulated time in the phase.
    pub elapsed: Duration,
    /// How many times the phase ran.
    pub hits: u64,
    /// Rows the phase reported writing, when it reports any.
    pub rows: u64,
}

/// Reads every phase's totals.
pub fn snapshot() -> Vec<PhaseTotals> {
    Phase::ALL
        .iter()
        .map(|&phase| {
            let idx = phase as usize;
            PhaseTotals {
                phase,
                elapsed: Duration::from_nanos(NANOS[idx].load(Ordering::Relaxed)),
                hits: HITS[idx].load(Ordering::Relaxed),
                rows: ROWS[idx].load(Ordering::Relaxed),
            }
        })
        .collect()
}

/// Zeroes every counter (between benchmark phases), including the ingest
/// progress behind [`ingest_progress`].
pub fn reset() {
    for idx in 0..PHASE_COUNT {
        NANOS[idx].store(0, Ordering::Relaxed);
        HITS[idx].store(0, Ordering::Relaxed);
        ROWS[idx].store(0, Ordering::Relaxed);
    }
    INGESTED.store(0, Ordering::Relaxed);
    INGEST_WALL_NANOS.store(0, Ordering::Relaxed);
    *INGEST_BASELINE.lock() = None;
}

/// Renders the snapshot as a table: per-resource cost and share of `wall` for
/// each phase, children indented under the phase that encloses them.
pub fn report(resources: u64, wall: Duration) -> String {
    report_totals(&snapshot(), resources, wall)
}

/// Renders only work recorded after `before`.
///
/// Counters are process-global. This subtraction is useful for a benchmark
/// that runs one reindex job at a time, but concurrent jobs contribute to the
/// same delta and cannot be separated by job id.
pub fn report_since(before: &[PhaseTotals], resources: u64, wall: Duration) -> String {
    let after = snapshot();
    let delta: Vec<PhaseTotals> = after
        .iter()
        .enumerate()
        .map(|(index, totals)| {
            let old = before.get(index).copied().unwrap_or(PhaseTotals {
                phase: totals.phase,
                elapsed: Duration::ZERO,
                hits: 0,
                rows: 0,
            });
            PhaseTotals {
                phase: totals.phase,
                elapsed: totals.elapsed.saturating_sub(old.elapsed),
                hits: totals.hits.saturating_sub(old.hits),
                rows: totals.rows.saturating_sub(old.rows),
            }
        })
        .collect();
    report_totals(&delta, resources, wall)
}

fn report_totals(totals_by_phase: &[PhaseTotals], resources: u64, wall: Duration) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{:<28} {:>12} {:>12} {:>10} {:>10}\n",
        "phase", "total", "per-resource", "share", "hits"
    ));
    out.push_str(&format!(
        "{:<28} {:>12} {:>12.3} {:>9.1}% {:>10}\n",
        "WALL",
        format!("{:.2}s", wall.as_secs_f64()),
        wall.as_secs_f64() * 1000.0 / resources.max(1) as f64,
        100.0,
        resources
    ));
    for totals in totals_by_phase {
        if totals.hits == 0 {
            continue;
        }
        let depth = {
            let mut d = 0;
            let mut p = totals.phase;
            while let Some(parent) = p.nested_in() {
                d += 1;
                p = parent;
            }
            d
        };
        let name = format!("{}{}", "  ".repeat(depth), totals.phase.label());
        let secs = totals.elapsed.as_secs_f64();
        out.push_str(&format!(
            "{:<28} {:>12} {:>12.3} {:>9.1}% {:>10}",
            name,
            format!("{:.2}s", secs),
            secs * 1000.0 / resources.max(1) as f64,
            secs / wall.as_secs_f64().max(f64::EPSILON) * 100.0,
            totals.hits
        ));
        if totals.rows > 0 {
            out.push_str(&format!(
                "  rows={} ({:.2}/resource)",
                totals.rows,
                totals.rows as f64 / resources.max(1) as f64
            ));
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The counters are process-global by design, and `cargo test` runs a
    /// crate's tests as threads of one process. So while these tests have
    /// collection switched on, *every other test in the crate* that performs
    /// an indexed write also lands hits and rows in the same counters — this
    /// module's first version asserted `rows == 14` and CI duly reported 15.
    ///
    /// Two rules follow, and both matter: the tests here take this lock so no
    /// two of them disagree about whether collection is on, and they assert on
    /// *deltas* they caused rather than on absolute totals they do not own.
    ///
    /// The exact-equality assertions below hold only because `Span::drop`
    /// re-checks the switch: with collection off, a span another thread
    /// started while it was on cannot land a sample afterwards.
    #[cfg(perf_phases)]
    static SWITCH: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

    #[cfg(perf_phases)]
    #[test]
    fn disabled_collection_records_nothing() {
        let _guard = SWITCH.lock();
        set_enabled(false);
        let before = snapshot()[Phase::Commit as usize].hits;
        {
            let _s = span(Phase::Commit);
            std::thread::sleep(Duration::from_millis(1));
        }
        // Exact, not a bound: with the switch held off, no thread in the
        // process can be recording.
        assert_eq!(snapshot()[Phase::Commit as usize].hits, before);
    }

    #[cfg(perf_phases)]
    #[test]
    fn enabled_collection_accumulates_time_and_rows() {
        let _guard = SWITCH.lock();
        let idx = Phase::IndexInsert as usize;
        set_enabled(true);
        let before = snapshot()[idx];
        {
            let _s = span(Phase::IndexInsert);
            std::thread::sleep(Duration::from_millis(2));
        }
        add_rows(Phase::IndexInsert, 14);
        let after = snapshot()[idx];
        set_enabled(false);

        // `>=`, because a concurrent test writing search-index rows adds to
        // these same counters while the switch is on.
        assert!(
            after.hits > before.hits,
            "hits {} -> {}",
            before.hits,
            after.hits
        );
        assert!(
            after.rows >= before.rows + 14,
            "rows {} -> {}",
            before.rows,
            after.rows
        );
        assert!(
            after.elapsed >= before.elapsed + Duration::from_millis(1),
            "elapsed {:?} -> {:?}",
            before.elapsed,
            after.elapsed
        );
    }

    /// `reset()` zeroes every counter. Run under the switch lock and with
    /// collection off, so nothing else can be writing while it is checked.
    #[cfg(perf_phases)]
    #[test]
    fn reset_zeroes_every_counter() {
        let _guard = SWITCH.lock();
        set_enabled(true);
        {
            let _s = span(Phase::Commit);
        }
        add_rows(Phase::IndexInsert, 3);
        set_enabled(false);
        reset();
        assert!(
            snapshot()
                .iter()
                .all(|t| t.hits == 0 && t.rows == 0 && t.elapsed == Duration::ZERO)
        );
    }

    /// The property release artifacts depend on: without `--cfg perf_phases`
    /// nothing records, and `set_enabled` cannot change that. `ci.yml` builds
    /// them with `--all-features`, so this must not be reachable through any
    /// feature combination.
    #[cfg(not(perf_phases))]
    #[test]
    fn without_the_cfg_nothing_records_even_when_switched_on() {
        set_enabled(true);
        {
            let _s = span(Phase::Commit);
            std::thread::sleep(Duration::from_millis(1));
        }
        add_rows(Phase::IndexInsert, 14);
        assert!(!enabled());
        assert!(
            snapshot()
                .iter()
                .all(|t| t.hits == 0 && t.rows == 0 && t.elapsed == Duration::ZERO),
            "a build without --cfg perf_phases must record nothing"
        );
    }

    #[cfg(perf_phases)]
    #[test]
    fn ingest_progress_reports_once_per_interval() {
        let _guard = SWITCH.lock();
        set_enabled(true);
        reset();
        let step = Duration::from_millis(10);
        assert!(ingest_progress(PROGRESS_INTERVAL / 2, step).is_none());
        // Crosses the first boundary: cumulative resources and wall.
        let report = ingest_progress(PROGRESS_INTERVAL / 2, step).expect("boundary crossed");
        assert!(
            report.contains(&format!("{:>10}", PROGRESS_INTERVAL)),
            "{report}"
        );
        assert!(report.contains("0.02s"), "{report}");
        // Inside the next interval: nothing.
        assert!(ingest_progress(1, step).is_none());
        // A batch larger than the interval still reports exactly once.
        assert!(ingest_progress(PROGRESS_INTERVAL * 3, step).is_some());
        set_enabled(false);
        reset();
    }

    #[cfg(perf_phases)]
    #[test]
    fn ingest_progress_reports_against_the_baseline() {
        let _guard = SWITCH.lock();
        set_enabled(true);
        reset();
        // Start-up work before the ingest: must not appear in the report.
        {
            let _span = span(Phase::ReindexFtsDelete);
            add_rows(Phase::ReindexFtsDelete, 5);
        }
        mark_ingest_start();
        {
            let _span = span(Phase::ReindexFtsDelete);
            add_rows(Phase::ReindexFtsDelete, 2);
        }
        let report =
            ingest_progress(PROGRESS_INTERVAL, Duration::from_secs(1)).expect("boundary crossed");
        set_enabled(false);
        reset();
        assert!(report.contains("rows=2 "), "{report}");
        assert!(!report.contains("rows=7 "), "{report}");
    }

    #[cfg(not(perf_phases))]
    #[test]
    fn ingest_progress_is_silent_without_the_cfg() {
        set_enabled(true);
        assert!(ingest_progress(PROGRESS_INTERVAL * 10, Duration::from_secs(1)).is_none());
    }

    #[test]
    fn phase_all_is_indexed_in_declaration_order() {
        // The report and `snapshot()` index by `phase as usize`; ALL must line
        // up with the discriminants or every row would be mislabelled.
        for (i, phase) in Phase::ALL.iter().enumerate() {
            assert_eq!(*phase as usize, i, "{} out of order", phase.label());
        }
    }

    #[test]
    fn every_phase_has_a_label_and_a_terminating_nesting_chain() {
        // Exercises `label()` and `nested_in()` for every variant (the assert
        // message in the test above only formats `label()` on failure, so it
        // never actually runs those arms). Also guards the nesting chain
        // against a cycle, which would hang `report_totals`.
        for phase in Phase::ALL {
            assert!(!phase.label().is_empty(), "{phase:?} has an empty label");
            let mut p = phase;
            let mut depth = 0;
            while let Some(parent) = p.nested_in() {
                p = parent;
                depth += 1;
                assert!(depth < PHASE_COUNT, "{phase:?} nests without terminating");
            }
        }
    }

    #[cfg(perf_phases)]
    #[test]
    fn report_since_contains_only_the_counter_delta() {
        let _guard = SWITCH.lock();
        set_enabled(true);
        reset();
        add_rows(Phase::ReindexSearchInsert, 7);
        let before = snapshot();
        {
            let _span = span(Phase::ReindexSearchInsert);
            add_rows(Phase::ReindexSearchInsert, 11);
        }
        {
            let _span = span(Phase::ReindexCommit);
        }
        let report = report_since(&before, 2, Duration::from_secs(1));
        set_enabled(false);
        assert!(report.contains("reindex_search_insert"));
        assert!(report.contains("rows=11 (5.50/resource)"));
        assert!(report.contains("reindex_commit"));
    }
}
