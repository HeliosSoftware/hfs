//! Sub-batch pipeline for MongoDB `$reindex` page preparation (#1403).
//!
//! While sub-batch *k*'s `insert_many` runs on a spawned task, the page's own
//! thread extracts sub-batch *k+1* inside `tokio::task::block_in_place`, so
//! HFS-side extraction overlaps MongoDB-side delete/insert instead of running
//! strictly after it. This module holds the planner that sizes sub-batches,
//! the `block_in_place` + rayon extraction helper, and, once added, the
//! writer bodies that call them. `storage.rs` keeps only the dispatcher and
//! the two `ReindexSource` prefetch methods.
//!
//! See `docs/superpowers/specs/2026-09-23-mongodb-reindex-rebuild-design.md`
//! §4.4.

// Not yet called by production code: the overlapped writer that uses these
// items lands in a later commit, which removes this allow once everything
// below is used.
#![allow(dead_code)]

use std::ops::Range;

/// A type's first page: too little is known yet to size from observation (#1403).
pub(super) const REINDEX_SUBBATCH_FIRST: usize = 32;
/// Own + contained index documents a sub-batch targets (#1403).
pub(super) const REINDEX_SUBBATCH_TARGET_DOCS: usize = 2_500;
/// Largest resources a single sub-batch may hold (#1403).
pub(super) const REINDEX_SUBBATCH_MAX: usize = 512;
/// Mirrors PostgreSQL's reindex prepare minimum page size: the smallest page
/// for which the serial writer still tries the rayon pool (#1403).
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
        Self {
            len,
            next: 0,
            min_size,
            seed,
            resources_done: 0,
            docs_done: 0,
        }
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
        usize::try_from(size)
            .unwrap_or(REINDEX_SUBBATCH_MAX)
            .min(REINDEX_SUBBATCH_MAX)
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
        let size = raw.clamp(
            self.min_size.clamp(1, REINDEX_SUBBATCH_MAX),
            REINDEX_SUBBATCH_MAX,
        );
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
/// the LIFO slot and run queue to another worker. Without it, extraction
/// would silently never overlap the spawned insert, even at a prepare width
/// of one.
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

/// `available_parallelism − 1`, clamped to 1–4, when `configured` is `0`,
/// mirroring PostgreSQL's reindex prepare width rule; otherwise `configured`
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
/// multi-thread Tokio runtime. Mirrors PostgreSQL's reindex prepare path;
/// kept as MongoDB's own copy because this backend's extraction also needs
/// `block_in_place` for the serial (non-pooled) path and a configurable
/// prepare width, which PostgreSQL's reindex path does not.
pub(super) fn tokio_multi_thread_runtime() -> bool {
    tokio::runtime::Handle::try_current().is_ok_and(|handle| {
        matches!(
            handle.runtime_flavor(),
            tokio::runtime::RuntimeFlavor::MultiThread
        )
    })
}

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
        assert_eq!(
            a.next_range(),
            Some(0..2),
            "1,630 docs/resource: ceil(2500/1630) = 2"
        );

        let mut b = SubBatchPlanner::new(1000, Some((1, 1630)), 4);
        assert_eq!(
            b.next_range(),
            Some(0..4),
            "min_size 4 lifts the same ratio to 4"
        );

        let mut c = SubBatchPlanner::new(1000, Some((1, 0)), 1);
        assert_eq!(
            c.next_range(),
            Some(0..REINDEX_SUBBATCH_MAX),
            "0 documents gives MAX"
        );

        let mut d = SubBatchPlanner::new(1000, Some((1, 100_000)), 1);
        assert_eq!(
            d.next_range(),
            Some(0..1),
            "100,000 docs/resource: ceil(2500/100000) = 1"
        );

        let mut e = SubBatchPlanner::new(1000, Some((0, 500)), 1);
        assert_eq!(
            e.next_range(),
            Some(0..REINDEX_SUBBATCH_FIRST),
            "a seed with r == 0 is ignored"
        );
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
                    assert_eq!(
                        covered, len,
                        "len={len} seed={seed:?} min_size={min_size}: must cover every index"
                    );
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
}
