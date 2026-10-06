//! Admission gate for transaction Bundles (#1776).
//!
//! Every transaction Bundle is one multi-document MongoDB transaction, and
//! WiredTiger keeps its uncommitted writes in cache until commit. With more
//! large Bundles open at once than the cache holds, the server rolls back the
//! oldest ("oldest pinned transaction ID rolled back for eviction"), which
//! surfaces as `WriteConflict` / `TransientTransactionError`. Replaying such a
//! Bundle only re-collides, so the pressure scales with how many Bundle
//! transactions are open at once, and retries do not reduce it. This gate
//! bounds that number per backend instance; a Bundle waits for a slot before
//! it starts its session, so a waiting Bundle holds no session and no
//! transaction.

use tokio::sync::{Semaphore, SemaphorePermit};

/// Default for `MongoBackendConfig::max_concurrent_transaction_bundles`.
///
/// Measured on the real backend path with 801-entry Bundles from 20 concurrent
/// clients, 24 Bundles, a 1 GB WiredTiger cache. No limit: 5/24 committed, 65
/// eviction rollbacks. Limit 8: 13/24, 40 rollbacks. Limit 4: 24/24, 0
/// rollbacks. Runs vary a little (another run gave 6/24 and 14/24).
/// The benchmark shape (2 GB cache, ~1,600-entry Bundles, 20 clients) at limit 4
/// was also measured: 22/24 committed, 2 eviction rollbacks, so the default
/// helps there but does not fully remove the failures.
/// The measurements live in `crates/persistence/README.md`, "Sizing the
/// WiredTiger cache for transaction Bundles".
pub(super) const DEFAULT_MAX_CONCURRENT_TRANSACTION_BUNDLES: usize = 4;

/// Bounds how many transaction Bundles one backend runs at once.
pub(super) struct TransactionBundleGate {
    slots: Option<Semaphore>,
    limit: Option<usize>,
}

impl TransactionBundleGate {
    /// `0` means no limit. A limit above [`Semaphore::MAX_PERMITS`] is clamped,
    /// because `Semaphore::new` panics past it.
    pub(super) fn new(limit: usize) -> Self {
        if limit == 0 {
            return Self {
                slots: None,
                limit: None,
            };
        }
        let limit = limit.min(Semaphore::MAX_PERMITS);
        Self {
            slots: Some(Semaphore::new(limit)),
            limit: Some(limit),
        }
    }

    /// The effective limit, or `None` when the gate is off.
    pub(super) fn limit(&self) -> Option<usize> {
        self.limit
    }

    /// Waits for a slot; `None` when there is no limit. The slot is held until
    /// the returned permit drops.
    ///
    /// Waiters are served first come first served (the semaphore is fair), and
    /// the wait is cancel-safe: dropping the future, as the request timeout
    /// does, gives up its place in the queue.
    pub(super) async fn admit(&self) -> Option<SemaphorePermit<'_>> {
        // The semaphore is never closed, so `acquire` cannot fail.
        self.slots.as_ref()?.acquire().await.ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    /// Runs 6 tasks that each hold a slot for 100 ms; returns `(max in
    /// flight, elapsed)`.
    async fn run_six(gate: Arc<TransactionBundleGate>) -> (usize, Duration) {
        let in_flight = Arc::new(AtomicUsize::new(0));
        let max = Arc::new(AtomicUsize::new(0));
        let started = tokio::time::Instant::now();
        let mut handles = Vec::new();
        for _ in 0..6 {
            let (gate, in_flight, max) = (gate.clone(), in_flight.clone(), max.clone());
            handles.push(tokio::spawn(async move {
                let _permit = gate.admit().await;
                let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                max.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(100)).await;
                in_flight.fetch_sub(1, Ordering::SeqCst);
            }));
        }
        for h in handles {
            h.await.expect("task finished");
        }
        (max.load(Ordering::SeqCst), started.elapsed())
    }

    #[tokio::test(start_paused = true)]
    async fn limit_bounds_concurrency() {
        let gate = Arc::new(TransactionBundleGate::new(2));
        assert_eq!(gate.limit(), Some(2));
        let (max, elapsed) = run_six(gate).await;
        assert_eq!(max, 2);
        assert_eq!(elapsed, Duration::from_millis(300));
    }

    #[tokio::test(start_paused = true)]
    async fn zero_means_no_limit() {
        let gate = Arc::new(TransactionBundleGate::new(0));
        assert_eq!(gate.limit(), None);
        assert!(gate.admit().await.is_none());
        let (max, elapsed) = run_six(gate).await;
        assert_eq!(max, 6);
        assert_eq!(elapsed, Duration::from_millis(100));
    }

    #[tokio::test(start_paused = true)]
    async fn waiters_are_admitted_first_come_first_served() {
        let gate = Arc::new(TransactionBundleGate::new(1));
        let order = Arc::new(Mutex::new(Vec::new()));
        let held = gate.admit().await.expect("limited gate yields a permit");
        let mut handles = Vec::new();
        for i in 0..3 {
            let (gate, order) = (gate.clone(), order.clone());
            handles.push(tokio::spawn(async move {
                let _permit = gate.admit().await;
                order.lock().unwrap().push(i);
            }));
            tokio::task::yield_now().await;
        }
        drop(held);
        for h in handles {
            h.await.expect("waiter finished");
        }
        assert_eq!(*order.lock().unwrap(), vec![0, 1, 2]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_dropped_waiter_gives_up_its_place() {
        let gate = TransactionBundleGate::new(1);
        let held = gate.admit().await.expect("permit");
        let waited = tokio::time::timeout(Duration::from_millis(10), gate.admit()).await;
        assert!(
            waited.is_err(),
            "the only slot is held, so the wait times out"
        );
        drop(held);
        let fresh = tokio::time::timeout(Duration::ZERO, gate.admit()).await;
        assert!(
            matches!(fresh, Ok(Some(_))),
            "the abandoned waiter must not keep the slot"
        );
    }

    #[test]
    fn oversized_limit_is_clamped() {
        let gate = TransactionBundleGate::new(usize::MAX);
        assert_eq!(gate.limit(), Some(Semaphore::MAX_PERMITS));
    }
}
