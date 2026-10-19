//! Returning freed heap to the OS after heavy work.
//!
//! glibc keeps freed memory in its per-thread arenas: after importing
//! transaction Bundles `hfs` held 4–10 GB at idle, which the host later
//! swapped out (#2020), and the dashboard's reconcile queries on a large
//! SQLite database grew it until it was killed (#2106). `malloc_trim(0)`
//! hands the free pages back. It walks every arena, so it runs on a blocking
//! thread, at most one at a time.
//!
//! The SQL-on-FHIR SQL Query run (#2158) is the same case: it loads every
//! dependency row into an in-memory engine, and the freed tables stayed
//! resident, so a run that loaded enough rows is followed by a trim too.

/// A request body at least this large is followed by a trim.
pub const TRIM_AFTER_BYTES: usize = 8 * 1024 * 1024;

/// A SQL Query run that materialized at least this many rows is followed by a
/// trim; smaller runs leave too little behind to be worth walking every arena.
///
/// The value is a judgment call, not a measurement: a typical flattened row
/// (a dozen columns of short text and numbers) takes on the order of a few
/// hundred bytes to a kilobyte in the engine, so 50,000 rows is roughly
/// 25-50 MB, about where a trim pays for walking every arena.
pub const TRIM_AFTER_ROWS: usize = 50_000;

/// Whether a run that handled `rows` rows should be followed by a trim.
pub fn should_trim_after_rows(rows: usize) -> bool {
    rows >= TRIM_AFTER_ROWS
}

/// Trims when dropped if the rows counted so far reach [`TRIM_AFTER_ROWS`].
/// Declare it before the buffers it should outlive, so they are freed first.
#[derive(Debug, Default)]
pub struct TrimOnDrop {
    rows: usize,
}

impl TrimOnDrop {
    /// A guard that starts with `rows` rows already counted.
    pub fn new(rows: usize) -> Self {
        Self { rows }
    }

    /// Counts more rows handled by the run.
    pub fn add_rows(&mut self, rows: usize) {
        self.rows = self.rows.saturating_add(rows);
    }

    /// Rows counted so far.
    pub fn rows(&self) -> usize {
        self.rows
    }
}

impl Drop for TrimOnDrop {
    fn drop(&mut self) {
        if should_trim_after_rows(self.rows) {
            release_free_memory();
        }
    }
}

/// Asks the allocator to return free memory to the OS, on a blocking thread.
/// If a trim is already running, one more is queued to run right after it, so
/// memory freed while it was walking the arenas is not left behind. Returns
/// whether a trim will run; on platforms without glibc's `malloc_trim` it does
/// nothing and returns false.
pub fn release_free_memory() -> bool {
    imp::release_free_memory()
}

/// The at-most-one-trim state machine, separate from the allocator call so
/// its interleavings can be driven step by step in tests.
///
/// A requester stores `pending` *before* it tries to claim `trimming`, and the
/// running trimmer clears `trimming` *before* it reads `pending`. With
/// sequentially consistent operations on both sides, either the requester
/// claims the trim itself or the trimmer sees its `pending` store: a request
/// is never lost.
#[cfg_attr(not(all(target_os = "linux", target_env = "gnu")), allow(dead_code))]
struct TrimGate {
    trimming: std::sync::atomic::AtomicBool,
    pending: std::sync::atomic::AtomicBool,
}

#[cfg_attr(not(all(target_os = "linux", target_env = "gnu")), allow(dead_code))]
impl TrimGate {
    const fn new() -> Self {
        Self {
            trimming: std::sync::atomic::AtomicBool::new(false),
            pending: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Records a request. True if the caller now owns the trim and must run
    /// it; false if a trim is running and will go around again for it.
    fn request(&self) -> bool {
        use std::sync::atomic::Ordering::SeqCst;
        self.pending.store(true, SeqCst);
        !self.trimming.swap(true, SeqCst)
    }

    /// Called by the trim owner at the top of each pass.
    fn begin_pass(&self) {
        self.pending
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }

    /// Called by the trim owner after a pass. True if it must run another
    /// pass (it still owns the trim); false once it has released it.
    fn finish_pass(&self) -> bool {
        use std::sync::atomic::Ordering::SeqCst;
        self.trimming.store(false, SeqCst);
        // A request that came in during the pass left `pending` set; take the
        // trim back unless a new requester already did.
        self.pending.load(SeqCst) && !self.trimming.swap(true, SeqCst)
    }
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
mod imp {
    use super::TrimGate;

    static GATE: TrimGate = TrimGate::new();

    pub(super) fn release_free_memory() -> bool {
        if !GATE.request() {
            return true;
        }
        tokio::task::spawn_blocking(|| {
            loop {
                GATE.begin_pass();
                let started = std::time::Instant::now();
                // SAFETY: `malloc_trim` only releases free pages held by the
                // allocator; it touches no live allocation.
                let released = unsafe { libc::malloc_trim(0) } != 0;
                tracing::debug!(
                    released,
                    duration_ms = started.elapsed().as_millis() as u64,
                    "malloc_trim released free heap"
                );
                if !GATE.finish_pass() {
                    break;
                }
            }
        });
        true
    }
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
mod imp {
    pub(super) fn release_free_memory() -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_row_gate_trims_only_runs_that_materialized_enough() {
        assert!(!should_trim_after_rows(0));
        assert!(!should_trim_after_rows(TRIM_AFTER_ROWS - 1));
        assert!(should_trim_after_rows(TRIM_AFTER_ROWS));
        let mut g = TrimOnDrop::new(TRIM_AFTER_ROWS - 10);
        g.add_rows(5);
        assert!(!should_trim_after_rows(g.rows()));
        g.add_rows(5);
        assert!(should_trim_after_rows(g.rows()));
        g.add_rows(usize::MAX);
        assert_eq!(g.rows(), usize::MAX);
    }

    #[test]
    fn a_request_made_while_a_trim_runs_is_not_lost() {
        let g = TrimGate::new();
        // First request claims the trim; the pass starts.
        assert!(g.request());
        g.begin_pass();
        // A second request arrives mid-pass: it must not start a second trim.
        assert!(!g.request());
        // The pass ends: the owner sees the request and keeps the trim.
        assert!(g.finish_pass());
        g.begin_pass();
        // Nothing arrived during the second pass: the owner releases.
        assert!(!g.finish_pass());
        // Now free again: a new request owns the trim.
        assert!(g.request());
    }

    #[test]
    fn a_request_between_the_release_and_the_check_is_not_lost() {
        let g = TrimGate::new();
        assert!(g.request());
        g.begin_pass();
        // Interleaving: the owner released `trimming` but has not read
        // `pending` yet when a requester arrives. The requester claims the
        // trim itself, so the owner must stand down (one trim task only).
        g.trimming.store(false, std::sync::atomic::Ordering::SeqCst);
        assert!(g.request());
        assert!(
            !(g.pending.load(std::sync::atomic::Ordering::SeqCst)
                && !g.trimming.swap(true, std::sync::atomic::Ordering::SeqCst))
        );
        // And a request whose claim failed before the release is seen by the
        // owner: covered by the previous test.
    }

    #[tokio::test]
    async fn a_trim_is_scheduled_only_where_glibc_provides_it() {
        let scheduled = release_free_memory();
        assert_eq!(
            scheduled,
            cfg!(all(target_os = "linux", target_env = "gnu"))
        );
    }
}
