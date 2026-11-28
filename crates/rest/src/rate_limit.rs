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

/// How often a full map (see [`RateLimiter::with_max_keys`]) is scanned for
/// buckets to drop. Between scans a new key is refused straight away, so a
/// stream of new keys against a full map costs one scan per interval, not one
/// per request.
const FULL_PRUNE_INTERVAL: Duration = Duration::from_secs(1);

/// The bucket map, the size past which the next prune runs, and when the last
/// one ran.
struct Buckets {
    map: HashMap<String, RateState>,
    prune_at: usize,
    last_prune: Option<Instant>,
}

impl Buckets {
    /// Drops the buckets with nothing left to remember, then hands back the
    /// memory they held: `retain` keeps the map's capacity, so a map that once
    /// held a burst's buckets would otherwise stay that size.
    fn prune(&mut self, now: Instant, window: Duration, today: u64) {
        self.map.retain(|_, state| {
            state
                .window
                .back()
                .is_some_and(|t| now.duration_since(*t) <= window)
                || (state.day == today && state.day_count > 0)
        });
        self.prune_at = self.map.len().saturating_mul(2).max(PRUNE_AT);
        self.map.shrink_to(self.prune_at);
        self.last_prune = Some(now);
    }

    /// Whether a prune is due: the map outgrew its threshold, or it holds more
    /// than the floor and a whole window has passed since the last prune, so
    /// the buckets of a burst that has lapsed go even when no new burst
    /// pushes the map past the threshold the old one set.
    fn prune_due(&self, now: Instant, window: Duration) -> bool {
        self.map.len() > self.prune_at
            || (self.map.len() > PRUNE_AT
                && self
                    .last_prune
                    .is_none_or(|at| now.duration_since(at) > window))
    }
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
    max_keys: Option<usize>,
}

impl RateLimiter {
    /// Creates an empty limiter. `const` so it can back a `static`.
    pub(crate) const fn new() -> Self {
        Self {
            buckets: OnceLock::new(),
            max_keys: None,
        }
    }

    /// Creates an empty limiter that holds at most `max_keys` buckets. When
    /// the map is full and a prune frees nothing, a request under a new key
    /// is rejected for a window instead of adding a bucket, so a surface whose
    /// keys a caller can invent cannot grow the map without bound.
    pub(crate) const fn with_max_keys(max_keys: usize) -> Self {
        Self {
            buckets: OnceLock::new(),
            max_keys: Some(max_keys),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Buckets> {
        self.buckets
            .get_or_init(|| {
                Mutex::new(Buckets {
                    map: HashMap::new(),
                    prune_at: PRUNE_AT,
                    last_prune: None,
                })
            })
            .lock()
            .expect("rate limiter lock")
    }

    /// Whether a request under `key` would be admitted by
    /// [`RateLimiter::check_window`], without recording one or creating a
    /// bucket. For a ceiling that is charged only after the request turns out
    /// to deserve it.
    pub(crate) fn peek_window(
        &self,
        key: &str,
        limit: u32,
        window: Duration,
    ) -> Result<(), RateLimited> {
        let buckets = self.lock();
        let now = Instant::now();
        let Some(state) = buckets.map.get(key) else {
            return Ok(());
        };
        let live: Vec<Instant> = state
            .window
            .iter()
            .copied()
            .filter(|t| now.duration_since(*t) <= window)
            .collect();
        if live.len() >= limit as usize {
            let oldest = live.first().copied().unwrap_or(now);
            return Err(RateLimited {
                kind: LimitKind::Window,
                retry_after_secs: ceil_secs(window.saturating_sub(now.duration_since(oldest))),
            });
        }
        Ok(())
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
        let mut buckets = self.lock();
        let now = Instant::now();
        let today = current_day();

        if buckets.prune_due(now, window) {
            buckets.prune(now, window, today);
        }
        if let Some(max_keys) = self.max_keys
            && buckets.map.len() >= max_keys
            && !buckets.map.contains_key(key)
        {
            if buckets
                .last_prune
                .is_none_or(|at| now.duration_since(at) >= FULL_PRUNE_INTERVAL)
            {
                buckets.prune(now, window, today);
            }
            if buckets.map.len() >= max_keys {
                return Err(RateLimited {
                    kind: LimitKind::Window,
                    retry_after_secs: ceil_secs(window),
                });
            }
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
    fn capacity(&self) -> usize {
        self.buckets
            .get()
            .map_or(0, |b| b.lock().expect("rate limiter lock").map.capacity())
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

    /// The buckets of a burst that has lapsed are dropped, and their memory
    /// handed back, once a window has passed, even though the map never grows
    /// past the threshold the burst left behind (#2680).
    #[test]
    fn a_lapsed_burst_is_pruned_and_its_memory_returned() {
        let limiter = RateLimiter::new();
        let window = Duration::from_millis(500);
        for i in 0..(2 * PRUNE_AT + 10) {
            limiter
                .check_window(&format!("k{i}"), 1, window)
                .expect("first request for a fresh key");
        }
        let peak = limiter.capacity();
        assert!(peak >= 2 * PRUNE_AT, "capacity {peak}");
        assert!(limiter.prune_threshold() > 2 * PRUNE_AT);
        std::thread::sleep(Duration::from_millis(600));

        limiter
            .check_window("after", 1, window)
            .expect("a request after the burst");
        assert_eq!(limiter.len(), 1, "the lapsed burst is gone");
        assert_eq!(limiter.prune_threshold(), PRUNE_AT);
        assert!(
            limiter.capacity() < peak,
            "capacity {} did not shrink from {peak}",
            limiter.capacity()
        );
    }

    /// A limiter with a key cap refuses a new key while it is full of live
    /// buckets, and keeps serving the keys it holds.
    #[test]
    fn a_full_limiter_refuses_new_keys() {
        let limiter = RateLimiter::with_max_keys(3);
        let window = Duration::from_secs(60);
        for key in ["a", "b", "c"] {
            limiter
                .check_window(key, 2, window)
                .expect("room for a key");
        }
        let err = limiter
            .check_window("d", 2, window)
            .expect_err("the map is full");
        assert_eq!(err.kind, LimitKind::Window);
        assert!((1..=60).contains(&err.retry_after_secs));
        assert_eq!(limiter.len(), 3, "the refused key got no bucket");
        limiter
            .check_window("a", 2, window)
            .expect("a held key keeps its budget");
    }

    /// `peek_window` answers as `check_window` would, without recording a
    /// request or creating a bucket.
    #[test]
    fn peek_window_records_nothing() {
        let limiter = RateLimiter::new();
        let window = Duration::from_secs(60);
        for _ in 0..3 {
            limiter
                .peek_window("k", 2, window)
                .expect("nothing recorded");
        }
        assert_eq!(limiter.len(), 0);
        limiter.check_window("k", 2, window).expect("first");
        limiter.peek_window("k", 2, window).expect("one left");
        limiter.check_window("k", 2, window).expect("second");
        let err = limiter
            .peek_window("k", 2, window)
            .expect_err("the budget is spent");
        assert_eq!(err.kind, LimitKind::Window);
    }

    #[test]
    fn ceil_secs_never_returns_zero() {
        assert_eq!(ceil_secs(Duration::ZERO), 1);
        assert_eq!(ceil_secs(Duration::from_millis(1)), 1);
        assert_eq!(ceil_secs(Duration::from_millis(1_001)), 2);
        assert_eq!(ceil_secs(Duration::from_secs(30)), 30);
    }
}
