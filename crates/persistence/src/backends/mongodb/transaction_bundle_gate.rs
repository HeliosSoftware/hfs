//! Admission gate for transaction Bundles (#1776, #1806).
//!
//! Every transaction Bundle is one multi-document MongoDB transaction, and
//! WiredTiger keeps its uncommitted writes in cache until commit. With more
//! large Bundles open at once than the cache holds, the server rolls back the
//! oldest ("oldest pinned transaction ID rolled back for eviction"), which
//! surfaces as `WriteConflict` / `TransientTransactionError`. Replaying such a
//! Bundle only re-collides, so the pressure scales with how many Bundle
//! transactions are open at once, and retries do not reduce it. This gate
//! bounds the Bundle entries held open per backend instance; a Bundle waits for
//! room before it starts its session, so a waiting Bundle holds no session and
//! no transaction.
//!
//! The gate counts entries, not Bundles (#1806): the cache pressure follows
//! the entries a Bundle writes, so one 1,600-entry Bundle should not weigh the
//! same as a 5-entry one. Its room is `limit × weight_entries` entries. A
//! Bundle takes its entry count from that room (at least 1, at most the whole
//! room, so a Bundle larger than the room is admitted alone and can always
//! run). Admission stays first come first served because tokio's semaphore is
//! fair, `acquire_many` included: a large waiter at the head is not overtaken
//! by smaller ones behind it.

use std::time::Duration;

use tokio::sync::{Semaphore, SemaphorePermit};
// tokio's `Instant`, not `std::time::Instant` like the rest of the retry loop,
// so that `start_paused` tests control both clocks. Both are monotonic in
// production.
use tokio::time::Instant;

use super::MongoBackendConfig;
use super::retry::{BUNDLE_TRANSACTION_RETRY, BundleElapsed, next_bundle_attempt_delay};

/// Default for `MongoBackendConfig::max_concurrent_transaction_bundles`: the
/// number of standard Bundles (see [`DEFAULT_TRANSACTION_BUNDLE_WEIGHT_ENTRIES`])
/// run at once.
///
/// Measured on the real backend path with 801-entry Bundles from 20 concurrent
/// clients, 24 Bundles, a 1 GB WiredTiger cache. No limit: 5/24 committed, 65
/// eviction rollbacks. Limit 8: 13/24, 40 rollbacks. Limit 4: 24/24, 0
/// rollbacks. Runs vary a little (another run gave 6/24 and 14/24).
/// The benchmark shape (2 GB cache, ~1,600-entry Bundles, 20 clients) at limit 4
/// was also measured: 22/24 committed, 2 eviction rollbacks, so the default
/// helps there but does not fully remove the failures.
///
/// These measurements were taken with the count-only gate (#1776), where every
/// Bundle took one slot.
///
/// The measurements live in `crates/persistence/README.md`, "Sizing the
/// WiredTiger cache for transaction Bundles".
pub(super) const DEFAULT_MAX_CONCURRENT_TRANSACTION_BUNDLES: usize = 4;

/// Default for `MongoBackendConfig::transaction_bundle_weight_entries`: the
/// entry count of one standard Bundle, the unit of
/// `max_concurrent_transaction_bundles` (#1806).
///
/// 1,000 is the smallest round size at or above the 801-entry Bundles behind
/// [`DEFAULT_MAX_CONCURRENT_TRANSACTION_BUNDLES`], so the defaults admit those
/// four at a time exactly as the count-only gate did. For Bundles of up to
/// 1,000 entries the defaults never hold more than 4,000 entries open at once
/// (the count-only gate's worst case); larger Bundles get fewer at a time.
pub(super) const DEFAULT_TRANSACTION_BUNDLE_WEIGHT_ENTRIES: usize = 1000;

/// Bounds how many transaction Bundle entries one backend runs at once.
pub(super) struct TransactionBundleGate {
    slots: Option<Semaphore>,
    limit: Option<usize>,
    capacity: Option<usize>,
    weight_entries: usize,
}

impl TransactionBundleGate {
    /// `limit` is the number of standard Bundles of `weight_entries` entries
    /// that fit at once; `0` means no limit. A limit above
    /// [`Semaphore::MAX_PERMITS`] is clamped, because `Semaphore::new` panics
    /// past it. `weight_entries == 0` counts every Bundle as one slot, the
    /// count-only gate of #1776.
    pub(super) fn new(limit: usize, weight_entries: usize) -> Self {
        if limit == 0 {
            return Self {
                slots: None,
                limit: None,
                capacity: None,
                weight_entries,
            };
        }
        let limit = limit.min(Semaphore::MAX_PERMITS);
        // `acquire_many` takes a `u32`, and a weight is capped at the capacity,
        // so the capacity must fit one.
        let capacity = if weight_entries == 0 {
            limit
        } else {
            limit.saturating_mul(weight_entries)
        }
        .min(Semaphore::MAX_PERMITS)
        .min(u32::MAX as usize);
        Self {
            slots: Some(Semaphore::new(capacity)),
            limit: Some(limit),
            capacity: Some(capacity),
            weight_entries,
        }
    }

    /// The effective limit, or `None` when the gate is off.
    pub(super) fn limit(&self) -> Option<usize> {
        self.limit
    }

    /// The room in entries (permits), or `None` when the gate is off.
    pub(super) fn capacity(&self) -> Option<usize> {
        self.capacity
    }

    /// The permits a Bundle of `entries` entries takes: 1 when the weight is
    /// `0` (every Bundle is one slot), otherwise its entry count, at least 1
    /// and at most the whole capacity so that it can always be admitted.
    pub(super) fn weight(&self, entries: usize) -> u32 {
        let Some(capacity) = self.capacity else {
            return 1;
        };
        if self.weight_entries == 0 {
            return 1;
        }
        u32::try_from(entries.max(1).min(capacity)).unwrap_or(u32::MAX)
    }

    /// Waits for room for a Bundle of `entries` entries. The room is held
    /// until the returned [`Admission`] drops; with no limit it holds none.
    ///
    /// Waiters are served first come first served (the semaphore is fair, also
    /// for `acquire_many`: a large waiter at the head holds back smaller ones
    /// behind it, so it is never starved), and the wait is cancel-safe:
    /// dropping the future, as the request timeout does, gives up its place in
    /// the queue.
    pub(super) async fn admit(&self, entries: usize) -> Admission<'_> {
        let called = Instant::now();
        // The semaphore is never closed, so `acquire_many` cannot fail.
        let permit = match self.slots.as_ref() {
            Some(slots) => slots.acquire_many(self.weight(entries)).await.ok(),
            None => None,
        };
        Admission {
            _permit: permit,
            called,
            admitted: Instant::now(),
        }
    }
}

/// A Bundle's place in the gate: the room it holds, and the two clocks its
/// replays are bounded by (#1806). Dropping it frees the room.
pub(super) struct Admission<'a> {
    _permit: Option<SemaphorePermit<'a>>,
    /// Taken immediately before waiting for room.
    called: Instant,
    /// Taken immediately after the room was granted.
    admitted: Instant,
}

impl Admission<'_> {
    /// How long the Bundle waited for room.
    pub(super) fn queued(&self) -> Duration {
        self.admitted.duration_since(self.called)
    }

    /// Time since the call reached the gate (the wait included) and since
    /// admission (the wait excluded).
    pub(super) fn elapsed(&self) -> BundleElapsed {
        BundleElapsed {
            since_call: self.called.elapsed(),
            since_admission: self.admitted.elapsed(),
        }
    }

    /// The delay before the next replay, or `None` when the Bundle gives up.
    /// Reads the budget and deadline from `config` (#1806), so the deadline
    /// wiring is testable here.
    pub(super) fn next_attempt_delay(
        &self,
        config: &MongoBackendConfig,
        attempts_made: u32,
        last_attempt: Duration,
        fraction: f64,
    ) -> Option<Duration> {
        next_bundle_attempt_delay(
            &BUNDLE_TRANSACTION_RETRY,
            attempts_made,
            self.elapsed(),
            last_attempt,
            config.bundle_transaction_budget,
            config.bundle_transaction_deadline,
            fraction,
        )
    }

    /// Whether this holds room in a limited gate.
    #[cfg(test)]
    pub(super) fn is_gated(&self) -> bool {
        self._permit.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Runs 6 Bundles of `entries` entries that each hold their room for
    /// 100 ms; returns `(max in flight, elapsed)`.
    async fn run_six(gate: Arc<TransactionBundleGate>, entries: usize) -> (usize, Duration) {
        let in_flight = Arc::new(AtomicUsize::new(0));
        let max = Arc::new(AtomicUsize::new(0));
        let started = tokio::time::Instant::now();
        let mut handles = Vec::new();
        for _ in 0..6 {
            let (gate, in_flight, max) = (gate.clone(), in_flight.clone(), max.clone());
            handles.push(tokio::spawn(async move {
                let _permit = gate.admit(entries).await;
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
        let gate = Arc::new(TransactionBundleGate::new(2, 0));
        assert_eq!(gate.limit(), Some(2));
        let (max, elapsed) = run_six(gate, 1_000).await;
        assert_eq!(max, 2);
        assert_eq!(elapsed, Duration::from_millis(300));
    }

    #[tokio::test(start_paused = true)]
    async fn zero_means_no_limit() {
        let gate = Arc::new(TransactionBundleGate::new(0, 1000));
        assert_eq!(gate.limit(), None);
        assert_eq!(gate.capacity(), None);
        assert!(!gate.admit(5).await.is_gated());
        let (max, elapsed) = run_six(gate, 5).await;
        assert_eq!(max, 6);
        assert_eq!(elapsed, Duration::from_millis(100));
    }

    #[tokio::test(start_paused = true)]
    async fn small_bundles_share_a_standard_slot() {
        let gate = Arc::new(TransactionBundleGate::new(2, 10));
        let (max, elapsed) = run_six(gate, 5).await;
        assert_eq!(max, 4);
        assert_eq!(elapsed, Duration::from_millis(200));
    }

    #[tokio::test(start_paused = true)]
    async fn a_bundle_larger_than_a_standard_one_takes_more_room() {
        let gate = Arc::new(TransactionBundleGate::new(4, 10));
        let (max, elapsed) = run_six(gate, 20).await;
        assert_eq!(max, 2);
        assert_eq!(elapsed, Duration::from_millis(300));
    }

    #[tokio::test(start_paused = true)]
    async fn an_oversized_bundle_is_capped_and_admitted_alone() {
        let gate = TransactionBundleGate::new(2, 10);
        assert_eq!(gate.weight(1_000_000), 20);
        let held = tokio::time::timeout(Duration::ZERO, gate.admit(1_000_000))
            .await
            .expect("an oversized Bundle must be admitted into an idle gate");
        assert!(held.is_gated());
        let waited = tokio::time::timeout(Duration::from_millis(10), gate.admit(1)).await;
        assert!(waited.is_err(), "the oversized Bundle runs alone");
        drop(held);
        let fresh = tokio::time::timeout(Duration::ZERO, gate.admit(1)).await;
        assert!(matches!(fresh, Ok(ref admission) if admission.is_gated()));
    }

    #[tokio::test(start_paused = true)]
    async fn a_large_waiter_is_not_overtaken_by_smaller_ones() {
        let gate = Arc::new(TransactionBundleGate::new(1, 10));
        let order = Arc::new(Mutex::new(Vec::new()));
        let held = gate.admit(5).await;
        assert!(held.is_gated());

        let a = {
            let (gate, order) = (gate.clone(), order.clone());
            tokio::spawn(async move {
                let _permit = gate.admit(10).await;
                order.lock().unwrap().push("large");
            })
        };
        tokio::task::yield_now().await;
        let b = {
            let (gate, order) = (gate.clone(), order.clone());
            tokio::spawn(async move {
                let _permit = gate.admit(1).await;
                order.lock().unwrap().push("small");
            })
        };
        for _ in 0..4 {
            tokio::task::yield_now().await;
        }
        assert!(
            order.lock().unwrap().is_empty(),
            "the small Bundle must queue behind the large waiter"
        );

        drop(held);
        a.await.expect("large finished");
        b.await.expect("small finished");
        assert_eq!(*order.lock().unwrap(), vec!["large", "small"]);
    }

    #[tokio::test(start_paused = true)]
    async fn waiters_are_admitted_first_come_first_served() {
        let gate = Arc::new(TransactionBundleGate::new(1, 0));
        let order = Arc::new(Mutex::new(Vec::new()));
        let held = gate.admit(1).await;
        assert!(held.is_gated());
        let mut handles = Vec::new();
        for i in 0..3 {
            let (gate, order) = (gate.clone(), order.clone());
            handles.push(tokio::spawn(async move {
                let _permit = gate.admit(1).await;
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
        let gate = TransactionBundleGate::new(1, 0);
        let held = gate.admit(1).await;
        assert!(held.is_gated());
        let waited = tokio::time::timeout(Duration::from_millis(10), gate.admit(1)).await;
        assert!(
            waited.is_err(),
            "the only slot is held, so the wait times out"
        );
        drop(held);
        let fresh = tokio::time::timeout(Duration::ZERO, gate.admit(1)).await;
        assert!(
            matches!(fresh, Ok(ref admission) if admission.is_gated()),
            "the abandoned waiter must not keep the slot"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn the_two_clocks_start_at_the_call_and_at_admission() {
        let gate = Arc::new(TransactionBundleGate::new(1, 0));
        let held = gate.admit(1).await;
        let (admitted_tx, admitted_rx) = tokio::sync::oneshot::channel();
        let (read_tx, read_rx) = tokio::sync::oneshot::channel::<()>();
        let waiter = {
            let gate = gate.clone();
            tokio::spawn(async move {
                let admission = gate.admit(1).await;
                admitted_tx.send(()).expect("test is waiting");
                read_rx.await.expect("test signals the read");
                (admission.queued(), admission.elapsed())
            })
        };
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_millis(300)).await;
        drop(held);
        admitted_rx.await.expect("waiter admitted");
        tokio::time::advance(Duration::from_millis(50)).await;
        read_tx.send(()).expect("waiter is waiting");
        let (queued, elapsed) = waiter.await.expect("waiter finished");
        assert_eq!(queued, Duration::from_millis(300));
        assert_eq!(elapsed.since_call, Duration::from_millis(350));
        assert_eq!(elapsed.since_admission, Duration::from_millis(50));
    }

    #[tokio::test(start_paused = true)]
    async fn a_queued_bundle_keeps_its_replays_only_under_a_deadline() {
        let gate = Arc::new(TransactionBundleGate::new(1, 0));
        let held = gate.admit(1).await;
        let (admitted_tx, admitted_rx) = tokio::sync::oneshot::channel();
        let (read_tx, read_rx) = tokio::sync::oneshot::channel::<()>();
        let waiter = {
            let gate = gate.clone();
            tokio::spawn(async move {
                let admission = gate.admit(1).await;
                admitted_tx.send(()).expect("test is waiting");
                read_rx.await.expect("test signals the read");
                let delay = |deadline: Option<Duration>| {
                    let config = MongoBackendConfig {
                        bundle_transaction_budget: Duration::from_secs(2),
                        bundle_transaction_deadline: deadline,
                        ..Default::default()
                    };
                    admission.next_attempt_delay(&config, 1, Duration::from_millis(100), 0.0)
                };
                (
                    delay(Some(Duration::from_secs(60))),
                    delay(None),
                    delay(Some(Duration::from_secs(3))),
                )
            })
        };
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(3)).await;
        drop(held);
        admitted_rx.await.expect("waiter admitted");
        tokio::time::advance(Duration::from_millis(100)).await;
        read_tx.send(()).expect("waiter is waiting");
        let (with_room, without_deadline, past_deadline) = waiter.await.expect("waiter finished");
        // The 3 s queue exceeds the 2 s budget, but the budget counts from admission.
        assert_eq!(with_room, Some(Duration::ZERO));
        // Without a deadline the wait counts against the budget, as in #1776.
        assert_eq!(without_deadline, None);
        // 3.1 s since the call plus 0.1 s of the last attempt passes the deadline.
        assert_eq!(past_deadline, None);
    }

    #[tokio::test(start_paused = true)]
    async fn an_ungated_admission_has_no_queue_time() {
        let gate = TransactionBundleGate::new(0, 1000);
        let admission = gate.admit(5).await;
        tokio::time::advance(Duration::from_millis(20)).await;
        assert_eq!(admission.queued(), Duration::ZERO);
        let elapsed = admission.elapsed();
        assert_eq!(elapsed.since_call, elapsed.since_admission);
        assert_eq!(elapsed.since_call, Duration::from_millis(20));
    }

    #[test]
    fn capacity_is_limit_times_weight() {
        let gate = TransactionBundleGate::new(4, 1000);
        assert_eq!(gate.capacity(), Some(4000));
        assert_eq!(gate.weight(801), 801);
        assert_eq!(gate.weight(5000), 4000);
    }

    #[test]
    fn zero_weight_entries_counts_bundles() {
        let gate = TransactionBundleGate::new(2, 0);
        assert_eq!(gate.capacity(), Some(2));
        assert_eq!(gate.weight(0), 1);
        assert_eq!(gate.weight(1_000_000), 1);
    }

    #[test]
    fn an_empty_bundle_weighs_one() {
        assert_eq!(TransactionBundleGate::new(2, 10).weight(0), 1);
    }

    #[test]
    fn oversized_limit_is_clamped() {
        let capacity = Semaphore::MAX_PERMITS.min(u32::MAX as usize);
        let gate = TransactionBundleGate::new(usize::MAX, 0);
        assert_eq!(gate.limit(), Some(Semaphore::MAX_PERMITS));
        assert_eq!(gate.capacity(), Some(capacity));

        let gate = TransactionBundleGate::new(usize::MAX, usize::MAX);
        assert_eq!(gate.limit(), Some(Semaphore::MAX_PERMITS));
        assert_eq!(gate.capacity(), Some(capacity));
        assert_eq!(gate.weight(usize::MAX) as usize, capacity);
    }
}
