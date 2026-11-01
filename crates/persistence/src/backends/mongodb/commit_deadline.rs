//! A deadline for the commit re-sends of a write whose commit outcome is unknown
//! (#2320).
//!
//! A commit that comes back unknown is sent again up to twice
//! (`BUNDLE_COMMIT_RETRIES`, `SINGLE_WRITE_COMMIT_RETRIES`). The driver gives
//! each re-send `w: majority` with a 10 s `wtimeout` (mongodb 3.7
//! `commit_transaction.rs`, `update_for_retry`), so a re-send whose majority
//! write stalls takes about 10 s. Nothing bounded them, and a request that ran
//! into `HFS_REQUEST_TIMEOUT` was answered `408`, which hides that the write may
//! have applied. A re-send now starts only if it can end before the write's
//! deadline; otherwise the commit is reported as unknown, the answer that tells
//! the client to read before it retries.
//!
//! A transaction Bundle's deadline is `bundle_transaction_deadline`, counted from
//! the call. A single-resource write has no admission gate to carry a clock, and
//! its attempt functions are shared by every way into a write, so its deadline
//! (`single_write_deadline`, counted from when the write reaches the backend)
//! travels in a task-local, as the search deadline does
//! ([`search_deadline`](super::search_deadline)): the outermost scope wins.

use std::{future::Future, time::Duration};

use tokio::time::Instant;

tokio::task_local! {
    static SINGLE_WRITE_DEADLINE: Instant;
}

/// The instant `deadline` after `start`, or `None` when there is no deadline or
/// it is too far away to be represented (which is no deadline either).
pub(super) fn deadline_after(start: Instant, deadline: Option<Duration>) -> Option<Instant> {
    deadline.and_then(|deadline| start.checked_add(deadline))
}

/// Runs `work`, a single-resource write, with the deadline `deadline` from now
/// for its commit re-sends.
///
/// Inside an active scope `work` just runs, so a scoped write called from
/// another scoped write would keep the outer write's clock. No scoped write
/// calls another today; the rule is defensive. With no deadline, or one that
/// cannot be represented, `work` runs without a scope.
pub(super) async fn single_write_scope<T>(
    deadline: Option<Duration>,
    work: impl Future<Output = T>,
) -> T {
    if SINGLE_WRITE_DEADLINE.try_with(|_| ()).is_ok() {
        return work.await;
    }
    match deadline_after(Instant::now(), deadline) {
        Some(at) => SINGLE_WRITE_DEADLINE.scope(at, work).await,
        None => work.await,
    }
}

/// The deadline of the single-resource write being run, if it has one.
pub(super) fn single_write_deadline_at() -> Option<Instant> {
    SINGLE_WRITE_DEADLINE.try_with(|at| *at).ok()
}

/// Whether a re-send fits in `left`: it is expected to take as long as the
/// slowest commit so far (`slowest`), and `after` must still be left once it
/// ends. No deadline (`left` is `None`) always fits.
pub(super) fn resend_fits(left: Option<Duration>, slowest: Duration, after: Duration) -> bool {
    left.is_none_or(|left| slowest.saturating_add(after) <= left)
}

/// What a commit loop needs to decide whether another re-send fits before its
/// deadline: the deadline, what has to follow the loop, and how long the slowest
/// commit of the loop took so far.
#[derive(Debug, Clone, Copy)]
pub(super) struct ResendWindow {
    deadline: Option<Instant>,
    after: Duration,
    slowest: Duration,
}

impl ResendWindow {
    /// No deadline: every re-send the budget allows is sent.
    pub(super) fn unbounded() -> Self {
        Self::until(None, Duration::ZERO)
    }

    /// Re-sends must end `after` before `deadline`; `after` is what runs once
    /// the loop is over (a single write's settling read, nothing for a Bundle).
    pub(super) fn until(deadline: Option<Instant>, after: Duration) -> Self {
        Self {
            deadline,
            after,
            slowest: Duration::ZERO,
        }
    }

    /// Records how long one `commitTransaction` call took.
    pub(super) fn record_commit(&mut self, took: Duration) {
        self.slowest = self.slowest.max(took);
    }

    /// The time left until the deadline, or `None` without one.
    pub(super) fn time_left(&self) -> Option<Duration> {
        self.deadline
            .map(|at| at.saturating_duration_since(Instant::now()))
    }

    /// The longest commit of the loop so far.
    pub(super) fn slowest(&self) -> Duration {
        self.slowest
    }

    /// Whether one more re-send fits.
    ///
    /// The re-send carries the same write concern the driver gives every
    /// re-sent commit (`w: majority`, `wtimeout` 10 s), so a stall costs it
    /// what it cost the slowest commit so far. The estimate can be short after
    /// a fast failure that follows a slow one, and when server selection takes
    /// longer than before. The clock is read only when there is a deadline.
    pub(super) fn another_fits(&self) -> bool {
        resend_fits(self.time_left(), self.slowest, self.after)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    #[test]
    fn resend_fits_compares_the_slowest_commit_and_the_reserve_with_the_time_left() {
        let s = Duration::from_secs;
        // No deadline: always.
        assert!(resend_fits(None, s(3600), s(3600)));
        // Equal fits; a millisecond short does not.
        assert!(resend_fits(Some(s(3)), s(2), s(1)));
        assert!(!resend_fits(Some(ms(2_999)), s(2), s(1)));
        // Nothing to wait for and nothing left fits; a commit that took time does not.
        assert!(resend_fits(
            Some(Duration::ZERO),
            Duration::ZERO,
            Duration::ZERO
        ));
        assert!(!resend_fits(Some(Duration::ZERO), ms(1), Duration::ZERO));
        // The sum saturates instead of overflowing.
        assert!(!resend_fits(
            Some(Duration::MAX - ms(1)),
            Duration::MAX,
            Duration::MAX
        ));
        assert!(resend_fits(
            Some(Duration::MAX),
            Duration::MAX,
            Duration::MAX
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn a_window_counts_the_slowest_commit_not_the_last() {
        let mut window = ResendWindow::until(Some(Instant::now() + ms(5_000)), ms(500));
        assert_eq!(window.time_left(), Some(ms(5_000)));
        assert_eq!(window.slowest(), Duration::ZERO);

        window.record_commit(ms(2_000));
        tokio::time::advance(ms(2_000)).await;
        assert!(window.another_fits(), "3 s left cover 2 s plus 0.5 s");

        // A fast failure after a slow commit does not lower the estimate.
        window.record_commit(ms(100));
        assert_eq!(window.slowest(), ms(2_000));
        tokio::time::advance(ms(600)).await;
        assert_eq!(window.time_left(), Some(ms(2_400)));
        assert!(!window.another_fits(), "2.4 s left do not cover 2.5 s");
    }

    #[tokio::test(start_paused = true)]
    async fn an_unbounded_window_always_fits() {
        let mut window = ResendWindow::unbounded();
        assert_eq!(window.time_left(), None);
        window.record_commit(Duration::from_secs(3600));
        tokio::time::advance(Duration::from_secs(3600)).await;
        assert!(window.another_fits());
    }

    #[tokio::test(start_paused = true)]
    async fn single_write_scope_sets_the_deadline_once_and_the_outer_clock_wins() {
        assert_eq!(single_write_deadline_at(), None);
        let start = Instant::now();

        single_write_scope(Some(ms(5_000)), async {
            assert_eq!(single_write_deadline_at(), Some(start + ms(5_000)));
            tokio::time::advance(ms(1_000)).await;
            // A write called from this one keeps this write's clock.
            single_write_scope(Some(ms(1_000)), async {
                assert_eq!(single_write_deadline_at(), Some(start + ms(5_000)));
            })
            .await;
            single_write_scope(None, async {
                assert_eq!(single_write_deadline_at(), Some(start + ms(5_000)));
            })
            .await;
        })
        .await;

        assert_eq!(single_write_deadline_at(), None, "the scope ended");
    }

    #[tokio::test(start_paused = true)]
    async fn single_write_scope_without_a_deadline_or_with_an_unrepresentable_one_sets_none() {
        single_write_scope(None, async {
            assert_eq!(single_write_deadline_at(), None);
        })
        .await;
        single_write_scope(Some(Duration::MAX), async {
            assert_eq!(single_write_deadline_at(), None);
        })
        .await;
        assert_eq!(deadline_after(Instant::now(), Some(Duration::MAX)), None);
        assert_eq!(deadline_after(Instant::now(), None), None);
    }
}
