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

/// Asks the allocator to return free memory to the OS, on a blocking thread,
/// unless a trim is already running. Returns whether one was scheduled; on
/// platforms without glibc's `malloc_trim` it does nothing.
pub fn release_free_memory() -> bool {
    imp::release_free_memory()
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
mod imp {
    use std::sync::atomic::{AtomicBool, Ordering};

    static TRIMMING: AtomicBool = AtomicBool::new(false);

    pub(super) fn release_free_memory() -> bool {
        if TRIMMING.swap(true, Ordering::AcqRel) {
            return false;
        }
        tokio::task::spawn_blocking(|| {
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
    async fn a_trim_is_scheduled_only_where_glibc_provides_it() {
        let scheduled = release_free_memory();
        assert_eq!(
            scheduled,
            cfg!(all(target_os = "linux", target_env = "gnu"))
        );
    }
}
