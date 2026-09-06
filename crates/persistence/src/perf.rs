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
//! * **Explicit nesting.** Phases are recorded as measured, so a phase that
//!   encloses another double-counts by design. [`Phase::nested_in`] declares
//!   the containment, and the report renders children indented under their
//!   parent instead of pretending the columns sum to the wall clock.
#[cfg(perf_phases)]
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

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
    /// The two bulk bookkeeping rows per entry.
    Bookkeeping,
    /// `COMMIT` of one batch transaction.
    Commit,
    /// Per-batch overhead outside the entry loop (BEGIN, manifest counters).
    BatchOverhead,
}

impl Phase {
    /// All phases, in report order.
    pub const ALL: [Phase; 19] = [
        Phase::NdjsonParse,
        Phase::Entry,
        Phase::EntryRead,
        Phase::Create,
        Phase::CreateExists,
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
        Phase::Commit,
        Phase::BatchOverhead,
    ];

    /// The phase this one is measured inside of, if any. Drives the report's
    /// indentation, and warns the reader that the two overlap.
    pub fn nested_in(self) -> Option<Phase> {
        match self {
            Phase::EntryRead | Phase::Create | Phase::Update | Phase::Bookkeeping => {
                Some(Phase::Entry)
            }
            Phase::ResourceUpdate => Some(Phase::Update),
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
            Phase::Bookkeeping => "bookkeeping",
            Phase::Commit => "commit",
            Phase::BatchOverhead => "batch_overhead",
        }
    }
}

const PHASE_COUNT: usize = 19;

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

/// Zeroes every counter (between benchmark phases).
pub fn reset() {
    for idx in 0..PHASE_COUNT {
        NANOS[idx].store(0, Ordering::Relaxed);
        HITS[idx].store(0, Ordering::Relaxed);
        ROWS[idx].store(0, Ordering::Relaxed);
    }
}

/// Renders the snapshot as a table: per-resource cost and share of `wall` for
/// each phase, children indented under the phase that encloses them.
pub fn report(resources: u64, wall: Duration) -> String {
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
    for totals in snapshot() {
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

    #[test]
    fn phase_all_is_indexed_in_declaration_order() {
        // The report and `snapshot()` index by `phase as usize`; ALL must line
        // up with the discriminants or every row would be mislabelled.
        for (i, phase) in Phase::ALL.iter().enumerate() {
            assert_eq!(*phase as usize, i, "{} out of order", phase.label());
        }
    }
}
