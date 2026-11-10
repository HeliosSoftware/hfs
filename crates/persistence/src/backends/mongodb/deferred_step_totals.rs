//! Cumulative totals of the post-commit search-row steps a backend has run
//! (#2036).
//!
//! A step logs its own duration and rows, and publishes them as metrics
//! (`helios_observability::deferred_index_metrics`), but neither can be read
//! back by the code that ran the Bundle. [`DeferredStepCounters`] keeps
//! relaxed atomic counters beside the metrics, in the style of
//! [`MongoPoolStats`](super::pool_stats::MongoPoolStats), and
//! [`MongoBackend::deferred_index_step_totals`](super::MongoBackend::deferred_index_step_totals)
//! reads them. A caller takes a snapshot before a Bundle and one after, and
//! [`DeferredIndexStepTotals::since`] gives what that Bundle's step did, without
//! parsing a log line.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// What the post-commit steps of one backend instance have done since it was
/// created, as read by
/// [`MongoBackend::deferred_index_step_totals`](super::MongoBackend::deferred_index_step_totals).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeferredIndexStepTotals {
    /// Steps that ran to their end (complete or not). A Bundle that deferred
    /// nothing runs no step, and a step whose request was dropped before it
    /// finished is not counted.
    pub steps: u64,
    /// Search rows those steps inserted (`search_index` and
    /// `search_index_contained`).
    pub rows: u64,
    /// The steps' own wall time, summed: from the step's start to its end,
    /// excluding the Bundle's transaction.
    pub elapsed: Duration,
    /// Steps that left work to the repair: cut at the Bundle deadline, or ended
    /// by a failure. A subset of `steps`.
    pub incomplete: u64,
}

impl DeferredIndexStepTotals {
    /// What happened between `earlier` and `self`: the difference of every
    /// field, saturating at zero.
    pub fn since(&self, earlier: &Self) -> Self {
        Self {
            steps: self.steps.saturating_sub(earlier.steps),
            rows: self.rows.saturating_sub(earlier.rows),
            elapsed: self.elapsed.saturating_sub(earlier.elapsed),
            incomplete: self.incomplete.saturating_sub(earlier.incomplete),
        }
    }
}

/// The atomics behind [`DeferredIndexStepTotals`].
#[derive(Debug, Default)]
pub(super) struct DeferredStepCounters {
    steps: AtomicU64,
    rows: AtomicU64,
    nanos: AtomicU64,
    incomplete: AtomicU64,
}

impl DeferredStepCounters {
    /// Adds one finished step.
    pub(super) fn record(&self, elapsed: Duration, rows: u64, incomplete: bool) {
        self.steps.fetch_add(1, Ordering::Relaxed);
        self.rows.fetch_add(rows, Ordering::Relaxed);
        // A u64 of nanoseconds holds 584 years; saturate rather than wrap.
        let nanos = u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX);
        // `fetch_update` is deprecated on newer toolchains in favour of
        // `try_update`, which is unstable on the 1.90 MSRV.
        #[allow(deprecated)]
        let _ = self
            .nanos
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |total| {
                Some(total.saturating_add(nanos))
            });
        if incomplete {
            self.incomplete.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(super) fn snapshot(&self) -> DeferredIndexStepTotals {
        DeferredIndexStepTotals {
            steps: self.steps.load(Ordering::Relaxed),
            rows: self.rows.load(Ordering::Relaxed),
            elapsed: Duration::from_nanos(self.nanos.load(Ordering::Relaxed)),
            incomplete: self.incomplete.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_empty() {
        assert_eq!(
            DeferredStepCounters::default().snapshot(),
            DeferredIndexStepTotals::default()
        );
    }

    #[test]
    fn records_steps_rows_time_and_incomplete_steps() {
        let counters = DeferredStepCounters::default();
        counters.record(Duration::from_millis(1_500), 1_000, false);
        counters.record(Duration::from_millis(500), 250, true);
        assert_eq!(
            counters.snapshot(),
            DeferredIndexStepTotals {
                steps: 2,
                rows: 1_250,
                elapsed: Duration::from_secs(2),
                incomplete: 1,
            }
        );
    }

    #[test]
    fn since_subtracts_and_saturates() {
        let counters = DeferredStepCounters::default();
        counters.record(Duration::from_secs(1), 10, false);
        let before = counters.snapshot();
        counters.record(Duration::from_secs(3), 90, true);
        let after = counters.snapshot();
        assert_eq!(
            after.since(&before),
            DeferredIndexStepTotals {
                steps: 1,
                rows: 90,
                elapsed: Duration::from_secs(3),
                incomplete: 1,
            }
        );
        // A snapshot taken the wrong way round is zero, not a wrapped number.
        assert_eq!(before.since(&after), DeferredIndexStepTotals::default());
    }

    #[test]
    fn a_huge_duration_saturates_instead_of_wrapping() {
        let counters = DeferredStepCounters::default();
        counters.record(Duration::MAX, 0, false);
        counters.record(Duration::from_secs(1), 0, false);
        assert_eq!(
            counters.snapshot().elapsed,
            Duration::from_nanos(u64::MAX),
            "the total stops at the largest representable time"
        );
    }
}
