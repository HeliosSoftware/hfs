//! Returning freed heap to the OS after heavy work.
//!
//! glibc keeps freed memory in its per-thread arenas: after importing
//! transaction Bundles `hfs` held 4–10 GB at idle, which the host later
//! swapped out (#2020), and the dashboard's reconcile queries on a large
//! SQLite database grew it until it was killed (#2106). `malloc_trim(0)`
//! hands the free pages back. It walks every arena, so it runs on a blocking
//! thread, at most one at a time.

/// A request body at least this large is followed by a trim.
pub const TRIM_AFTER_BYTES: usize = 8 * 1024 * 1024;

/// A SQL Query run that materialized at least this many rows is followed by a
/// trim; smaller runs leave too little behind to be worth walking every arena.
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

#[cfg(all(target_os = "linux", target_env = "gnu"))]
mod imp {
    use std::sync::atomic::{AtomicBool, Ordering};

    static TRIMMING: AtomicBool = AtomicBool::new(false);
    static PENDING: AtomicBool = AtomicBool::new(false);

    pub(super) fn release_free_memory() -> bool {
        if TRIMMING.swap(true, Ordering::AcqRel) {
            // Running now: ask it to go around once more when it finishes.
            PENDING.store(true, Ordering::Release);
            return true;
        }
        tokio::task::spawn_blocking(|| {
            loop {
                PENDING.store(false, Ordering::Release);
                let started = std::time::Instant::now();
                // SAFETY: `malloc_trim` only releases free pages held by the
                // allocator; it touches no live allocation.
                let released = unsafe { libc::malloc_trim(0) } != 0;
                tracing::debug!(
                    released,
                    duration_ms = started.elapsed().as_millis() as u64,
                    "malloc_trim released free heap"
                );
                TRIMMING.store(false, Ordering::Release);
                // A request that arrived after the walk began set PENDING; take
                // the flag back over unless a new trim already started.
                if !PENDING.load(Ordering::Acquire) || TRIMMING.swap(true, Ordering::AcqRel) {
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

    #[tokio::test]
    async fn a_trim_is_scheduled_only_where_glibc_provides_it() {
        let scheduled = release_free_memory();
        assert_eq!(
            scheduled,
            cfg!(all(target_os = "linux", target_env = "gnu"))
        );
    }
}
