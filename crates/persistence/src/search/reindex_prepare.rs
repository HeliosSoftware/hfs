//! Bounded CPU parallelism for `$reindex` page preparation, shared by the
//! writers that extract search values on a rayon pool (#1403, #1250).
//!
//! A writer owns a lazily built [`rayon::ThreadPool`] and a one-permit
//! admission gate; [`extract_range`] runs one page's (or sub-batch's)
//! extraction on that pool in input order, or inline when the pool is absent,
//! the range is tiny, or another page already holds the gate. MongoDB's
//! sub-batch pipeline and the Elasticsearch page writer (the composite
//! deployments' search target) both use it; PostgreSQL keeps its own copy
//! (`backends/postgres/search/writer.rs`), which predates this module.

use std::ops::Range;

/// Explicit `*_REINDEX_PREPARE_THREADS` values above this are clamped.
pub(crate) const REINDEX_PREPARE_MAX_THREADS: usize = 64;

/// The pool and its admission gate for one extraction run.
pub(crate) struct PrepareEnv<'a> {
    /// `None` runs every range inline.
    pub pool: Option<&'a rayon::ThreadPool>,
    /// One permit: a second page arriving while the first is on the pool
    /// extracts inline instead of queueing behind it.
    pub gate: &'a tokio::sync::Semaphore,
}

/// Runs `prepare` over `range` in input order, returning `true` when it ran
/// on the pool (#1403). The caller guarantees a multi-thread Tokio runtime
/// (see [`tokio_multi_thread_runtime`]).
///
/// Every range runs inside `block_in_place`, even with `pool: None`: a task
/// spawned from this worker (a previous sub-batch's insert, or a prefetch)
/// sits in this worker's LIFO slot until the worker either finishes its own
/// synchronous work or calls `block_in_place`, which hands the LIFO slot and
/// run queue to another worker. Without it, extraction would silently never
/// overlap the spawned work, even at a prepare width of one.
pub(crate) fn extract_range<T: Send, F: Fn(usize) -> T + Sync>(
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

/// `available_parallelism − 1`, clamped to 1–4, when `configured` is `0`,
/// mirroring PostgreSQL's reindex prepare width rule; otherwise `configured`
/// clamped to at most [`REINDEX_PREPARE_MAX_THREADS`] (#1403).
pub(crate) fn resolve_prepare_width(configured: usize) -> usize {
    if configured == 0 {
        std::thread::available_parallelism()
            .map(|n| n.get().saturating_sub(1).clamp(1, 4))
            .unwrap_or(1)
    } else {
        configured.min(REINDEX_PREPARE_MAX_THREADS)
    }
}

/// Whether this thread is running a multi-thread Tokio runtime, which is
/// what lets `block_in_place` hand this worker's run queue to another worker
/// instead of just blocking it. `block_in_place` panics only on a
/// current-thread runtime; called with no runtime running at all, it just
/// runs its closure inline — the writers never call it in that case, since
/// they gate the pooled extraction path behind this check first.
pub(crate) fn tokio_multi_thread_runtime() -> bool {
    tokio::runtime::Handle::try_current().is_ok_and(|handle| {
        matches!(
            handle.runtime_flavor(),
            tokio::runtime::RuntimeFlavor::MultiThread
        )
    })
}

/// Builds a writer's prepare pool once, or remembers why it could not be
/// built so the warning fires once per backend instance. `None` when the
/// resolved width is below 2 (nothing to gain from a pool) or the build
/// failed.
pub(crate) fn prepare_pool<'a>(
    slot: &'a std::sync::OnceLock<Result<rayon::ThreadPool, String>>,
    configured_threads: usize,
    thread_prefix: &'static str,
    backend_label: &'static str,
) -> Option<&'a rayon::ThreadPool> {
    let width = resolve_prepare_width(configured_threads);
    if width < 2 {
        return None;
    }
    slot.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(width)
            .thread_name(move |i| format!("{thread_prefix}-{i}"))
            .build()
            .map_err(|e| {
                let message = format!(
                    "Failed to build the {backend_label} reindex prepare pool; reindex pages will be extracted on the calling thread: {e}"
                );
                tracing::warn!("{message}");
                message
            })
    })
    .as_ref()
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(3)
            .build()
            .unwrap();
        let gate = tokio::sync::Semaphore::new(1);
        let prepare = |i: usize| i * 10;

        let pooled_env = PrepareEnv {
            pool: Some(&pool),
            gate: &gate,
        };
        let (out, on_pool) = extract_range(&pooled_env, 0..20, &prepare);
        assert!(
            on_pool,
            "a range of 20 with a pool and a free gate must run on the pool"
        );
        assert_eq!(out, (0..20).map(|i| i * 10).collect::<Vec<_>>());

        let inline_env = PrepareEnv {
            pool: None,
            gate: &gate,
        };
        let (out, on_pool) = extract_range(&inline_env, 0..20, &prepare);
        assert!(!on_pool);
        assert_eq!(out, (0..20).map(|i| i * 10).collect::<Vec<_>>());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn extract_range_runs_inline_when_the_gate_is_held() {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(3)
            .build()
            .unwrap();
        let gate = tokio::sync::Semaphore::new(1);
        let _permit = gate
            .try_acquire()
            .expect("a fresh semaphore has a free permit");
        let env = PrepareEnv {
            pool: Some(&pool),
            gate: &gate,
        };
        let calling_thread = std::thread::current().id();
        let prepare = move |_i: usize| std::thread::current().id();

        let (out, on_pool) = extract_range(&env, 0..5, &prepare);
        assert!(!on_pool, "a held gate must fall back to the calling thread");
        assert!(out.iter().all(|id| *id == calling_thread));
    }

    #[test]
    fn prepare_pool_is_none_below_width_two_and_built_once_otherwise() {
        let slot = std::sync::OnceLock::new();
        assert!(prepare_pool(&slot, 1, "hfs-test-reindex", "test").is_none());
        assert!(slot.get().is_none(), "a width of 1 never builds a pool");

        let slot = std::sync::OnceLock::new();
        let first = prepare_pool(&slot, 2, "hfs-test-reindex", "test").expect("built");
        let second = prepare_pool(&slot, 2, "hfs-test-reindex", "test").expect("reused");
        assert!(std::ptr::eq(first, second), "the pool is built once");
        assert_eq!(first.current_num_threads(), 2);
    }
}
