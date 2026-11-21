//! Shared per-key sliding-window rate limiting.
//!
//! One [`RateLimiter`] owns one bucket map, so each protected surface declares
//! its own `static` limiter and surfaces never share (or steal) each other's
//! budget. A rejection carries the delta-seconds until the caller has budget
//! again, which the caller turns into a `Retry-After` header on its `429` —
//! a bare `429` tells a client to back off but not for how long, so clients
//! guess, and guessing is what the limiter is there to prevent.

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Keys are unbounded in principle (one bucket per peer address, or per poll
/// token), so a limiter's map is pruned of inactive buckets once it grows
/// past this. After a prune the threshold is twice the surviving population
/// (never below this), so the scan is amortised over the growth that
/// triggers it rather than paid on every call once the map is large.
const PRUNE_AT: usize = 10_000;

/// Which ceiling a rejected request hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LimitKind {
    /// The sliding-window limit — retry once the window rolls forward.
    Window,
    /// The per-day ceiling — retry after the UTC day boundary.
    Daily,
}

/// A rejected request, with the back-off the caller should advertise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RateLimited {
    /// Which ceiling was hit.
    pub(crate) kind: LimitKind,
    /// Delta-seconds until the caller regains budget. Always `>= 1`, since a
    /// `Retry-After: 0` reads as "retry immediately" and re-triggers the limit.
    pub(crate) retry_after_secs: u64,
}

/// Per-key state: the request instants inside the current window, plus the
/// running count for the current UTC day. The count is kept only under a
/// daily ceiling; a window-only bucket has nothing to remember once its
/// window has lapsed, and is pruned then.
struct RateState {
    window: VecDeque<Instant>,
    day: u64,
    day_count: u32,
}

/// The bucket map and the size past which the next prune runs.
struct Buckets {
    map: HashMap<String, RateState>,
    prune_at: usize,
}

/// A sliding-window rate limiter with an optional per-day ceiling.
///
/// Declare one per protected surface:
/// ```ignore
/// static POLL_LIMITER: RateLimiter = RateLimiter::new();
/// ```
///
/// A key is what the caller passes: a surface whose key carries caller-chosen
/// text (a path segment, a header) bounds it first, by hashing or by format,
/// since every distinct key is a bucket until it is pruned.
pub(crate) struct RateLimiter {
    buckets: OnceLock<Mutex<Buckets>>,
}

impl RateLimiter {
    /// Creates an empty limiter. `const` so it can back a `static`.
    pub(crate) const fn new() -> Self {
        Self {
            buckets: OnceLock::new(),
        }
    }

    /// Records a request against `key`, rejecting it when either ceiling is hit.
    ///
    /// A rejected request is *not* counted, so a client that keeps hammering
    /// never pushes its own recovery further out.
    pub(crate) fn check(
        &self,
        key: &str,
        limit: u32,
        window: Duration,
        daily_limit: u32,
    ) -> Result<(), RateLimited> {
        self.admit(key, limit, window, Some(daily_limit))
    }

    fn admit(
        &self,
        key: &str,
        limit: u32,
        window: Duration,
        daily_limit: Option<u32>,
    ) -> Result<(), RateLimited> {
        let mut buckets = self
            .buckets
            .get_or_init(|| {
                Mutex::new(Buckets {
                    map: HashMap::new(),
                    prune_at: PRUNE_AT,
                })
            })
            .lock()
            .expect("rate limiter lock");
        let now = Instant::now();
        let today = current_day();

        if buckets.map.len() > buckets.prune_at {
            buckets.map.retain(|_, state| {
                state
                    .window
                    .back()
                    .is_some_and(|t| now.duration_since(*t) <= window)
                    || (state.day == today && state.day_count > 0)
            });
            buckets.prune_at = buckets.map.len().saturating_mul(2).max(PRUNE_AT);
        }

        let state = buckets
            .map
            .entry(key.to_string())
            .or_insert_with(|| RateState {
                window: VecDeque::new(),
                day: today,
                day_count: 0,
            });

        if state.day != today {
            state.day = today;
            state.day_count = 0;
        }
        while state
            .window
            .front()
            .is_some_and(|t| now.duration_since(*t) > window)
        {
            state.window.pop_front();
        }

        if daily_limit.is_some_and(|daily_limit| state.day_count >= daily_limit) {
            return Err(RateLimited {
                kind: LimitKind::Daily,
                retry_after_secs: secs_until_utc_midnight(),
            });
        }
        if state.window.len() >= limit as usize {
            // The oldest request in the window is the first to age out, so that
            // is when a slot frees up.
            let oldest = state.window.front().copied().unwrap_or(now);
            let elapsed = now.duration_since(oldest);
            let remaining = window.saturating_sub(elapsed);
            return Err(RateLimited {
                kind: LimitKind::Window,
                retry_after_secs: ceil_secs(remaining),
            });
        }

        state.window.push_back(now);
        if daily_limit.is_some() {
            state.day_count += 1;
        }
        Ok(())
    }

    /// [`RateLimiter::check`] without a per-day ceiling.
    pub(crate) fn check_window(
        &self,
        key: &str,
        limit: u32,
        window: Duration,
    ) -> Result<(), RateLimited> {
        self.admit(key, limit, window, None)
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.buckets
            .get()
            .map_or(0, |b| b.lock().expect("rate limiter lock").map.len())
    }

    #[cfg(test)]
    fn prune_threshold(&self) -> usize {
        self.buckets
            .get()
            .map_or(PRUNE_AT, |b| b.lock().expect("rate limiter lock").prune_at)
    }
}

/// Days since the Unix epoch, i.e. the current UTC day.
fn current_day() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or_default()
}

/// Delta-seconds until the next UTC day boundary, when a daily ceiling resets.
fn secs_until_utc_midnight() -> u64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    (86_400 - (now % 86_400)).max(1)
}

/// Rounds a back-off up to whole seconds, never below 1.
fn ceil_secs(d: Duration) -> u64 {
    let secs = d.as_secs() + u64::from(d.subsec_nanos() > 0);
    secs.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_requests_up_to_the_window_limit() {
        let limiter = RateLimiter::new();
        for _ in 0..3 {
            assert!(
                limiter
                    .check_window("k", 3, Duration::from_secs(60))
                    .is_ok()
            );
        }
        let err = limiter
            .check_window("k", 3, Duration::from_secs(60))
            .expect_err("fourth request is over the limit");
        assert_eq!(err.kind, LimitKind::Window);
    }

    #[test]
    fn window_rejection_advertises_a_usable_retry_after() {
        let limiter = RateLimiter::new();
        limiter
            .check_window("k", 1, Duration::from_secs(60))
            .expect("first request");
        let err = limiter
            .check_window("k", 1, Duration::from_secs(60))
            .expect_err("second request is over the limit");
        // The single recorded request has barely aged, so the advertised
        // back-off is the near-full window — and never 0.
        assert!(
            (1..=60).contains(&err.retry_after_secs),
            "retry_after_secs out of range: {}",
            err.retry_after_secs
        );
    }

    #[test]
    fn buckets_are_independent_per_key() {
        let limiter = RateLimiter::new();
        limiter
            .check_window("a", 1, Duration::from_secs(60))
            .expect("first request for a");
        assert!(
            limiter
                .check_window("a", 1, Duration::from_secs(60))
                .is_err()
        );
        assert!(
            limiter
                .check_window("b", 1, Duration::from_secs(60))
                .is_ok()
        );
    }

    #[test]
    fn limiters_are_independent_of_each_other() {
        let one = RateLimiter::new();
        let two = RateLimiter::new();
        one.check_window("k", 1, Duration::from_secs(60))
            .expect("first request");
        assert!(one.check_window("k", 1, Duration::from_secs(60)).is_err());
        assert!(two.check_window("k", 1, Duration::from_secs(60)).is_ok());
    }

    #[test]
    fn a_lapsed_window_frees_the_budget() {
        let limiter = RateLimiter::new();
        // A zero-length window ages every recorded request out immediately.
        for _ in 0..5 {
            assert!(limiter.check_window("k", 1, Duration::ZERO).is_ok());
        }
    }

    #[test]
    fn daily_ceiling_rejects_after_the_window_has_room() {
        let limiter = RateLimiter::new();
        for _ in 0..2 {
            limiter
                .check("k", 100, Duration::from_secs(60), 2)
                .expect("within both ceilings");
        }
        let err = limiter
            .check("k", 100, Duration::from_secs(60), 2)
            .expect_err("third request is over the daily ceiling");
        assert_eq!(err.kind, LimitKind::Daily);
        assert!(err.retry_after_secs >= 1 && err.retry_after_secs <= 86_400);
    }

    #[test]
    fn rejected_requests_do_not_extend_the_back_off() {
        let limiter = RateLimiter::new();
        limiter
            .check_window("k", 1, Duration::from_secs(60))
            .expect("first request");
        let first = limiter.check_window("k", 1, Duration::from_secs(60));
        let second = limiter.check_window("k", 1, Duration::from_secs(60));
        // Both rejections point at the same recorded request, so the advertised
        // back-off shrinks (or holds) rather than resetting to a full window.
        assert!(
            second.unwrap_err().retry_after_secs <= first.unwrap_err().retry_after_secs,
            "a rejected request must not be counted"
        );
    }

    /// Past the prune threshold, a window-only bucket whose window has lapsed
    /// is dropped: an unknown poll token or a one-off peer does not hold its
    /// bucket until the UTC day ends.
    #[test]
    fn window_only_buckets_are_dropped_once_their_window_lapses() {
        let limiter = RateLimiter::new();
        let window = Duration::from_millis(1);
        for i in 0..=PRUNE_AT {
            limiter
                .check_window(&format!("k{i}"), 1, window)
                .expect("first request for a fresh key");
        }
        assert_eq!(limiter.len(), PRUNE_AT + 1);
        std::thread::sleep(Duration::from_millis(20));

        limiter
            .check_window("trigger", 1, window)
            .expect("the triggering request");
        assert_eq!(limiter.len(), 1, "only the triggering bucket survives");
    }

    /// A bucket under a daily ceiling keeps its count for the day, so the
    /// ceiling cannot be reset by growing the map past the threshold.
    #[test]
    fn daily_buckets_survive_the_prune_within_the_day() {
        let limiter = RateLimiter::new();
        let window = Duration::from_millis(1);
        limiter
            .check("spent", 100, window, 1)
            .expect("the one request of the day");
        for i in 0..=PRUNE_AT {
            limiter
                .check(&format!("k{i}"), 100, window, 100)
                .expect("first request for a fresh key");
        }
        std::thread::sleep(Duration::from_millis(20));

        limiter
            .check("trigger", 100, window, 100)
            .expect("the triggering request");
        assert_eq!(limiter.len(), PRUNE_AT + 3, "every daily bucket survives");
        let err = limiter
            .check("spent", 100, window, 1)
            .expect_err("the daily ceiling still holds");
        assert_eq!(err.kind, LimitKind::Daily);
    }

    /// After a prune the threshold follows the surviving population, so a
    /// map that legitimately holds more than `PRUNE_AT` live buckets is not
    /// scanned on every call.
    #[test]
    fn the_prune_threshold_grows_with_the_surviving_population() {
        let limiter = RateLimiter::new();
        let window = Duration::from_secs(60);
        for i in 0..=PRUNE_AT {
            limiter
                .check_window(&format!("k{i}"), 1, window)
                .expect("first request for a fresh key");
        }
        assert_eq!(limiter.prune_threshold(), PRUNE_AT);
        limiter
            .check_window("trigger", 1, window)
            .expect("the triggering request");
        assert_eq!(limiter.len(), PRUNE_AT + 2, "nothing had lapsed");
        assert_eq!(limiter.prune_threshold(), 2 * (PRUNE_AT + 1));
    }

    #[test]
    fn ceil_secs_never_returns_zero() {
        assert_eq!(ceil_secs(Duration::ZERO), 1);
        assert_eq!(ceil_secs(Duration::from_millis(1)), 1);
        assert_eq!(ceil_secs(Duration::from_millis(1_001)), 2);
        assert_eq!(ceil_secs(Duration::from_secs(30)), 30);
    }
}
