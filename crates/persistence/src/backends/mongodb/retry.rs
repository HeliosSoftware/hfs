//! Bounded retry of transient MongoDB driver errors.
//!
//! The driver retries a retryable write once on a replica set and never on a
//! standalone server; a server that stays busy longer than that outlasts it.
//! This module adds a short, bounded, cancel-aware retry on top, shared by the
//! settings store and the `$bulk-submit` batch ingest.

use std::future::Future;
use std::time::Duration;

use mongodb::error::{Error as MongoError, ErrorKind, RETRYABLE_ERROR, RETRYABLE_WRITE_ERROR};

use crate::core::bulk_submit::CancelToken;
use crate::error::{BackendError, StorageError, StorageResult};

/// How many times an operation runs and how long it waits in between.
#[derive(Debug, Clone, Copy)]
pub(super) struct RetryPolicy {
    /// Total attempts, the first included.
    pub max_attempts: u32,
    /// Sleep before the second attempt; doubles before each later one.
    pub base: Duration,
    /// Upper bound on any single sleep.
    pub cap: Duration,
}

impl RetryPolicy {
    /// Sleep after the failure of attempt `attempt` (1-based).
    pub(super) fn backoff(&self, attempt: u32) -> Duration {
        let factor = 1u32 << (attempt - 1).min(16);
        self.base.saturating_mul(factor).min(self.cap)
    }
}

/// Settings-store policy: 25, 50, 100 ms. A brief blip, never a long stall
/// for an interactive caller.
pub(super) const SETTINGS_RETRY: RetryPolicy = RetryPolicy {
    max_attempts: 4,
    base: Duration::from_millis(25),
    cap: Duration::from_millis(100),
};

/// Bulk-ingest policy: 100, 200, 400, 800, 1000 ms — 2.5 s of sleep across
/// six attempts, enough for a cleared pool to reconnect under load (#1001).
pub(super) const BULK_INGEST_RETRY: RetryPolicy = RetryPolicy {
    max_attempts: 6,
    base: Duration::from_millis(100),
    cap: Duration::from_secs(1),
};

/// Transaction-bundle policy: at most three runs of one bundle, pausing up to
/// 200 ms and then up to 400 ms in between, each pause drawn with full jitter
/// ([`next_attempt_delay`]). The server aborts a transaction that loses a race
/// with concurrent writers (`WriteConflict`, or an eviction under cache
/// pressure) and labels the error `TransientTransactionError`; the losers of a
/// burst retrying in lockstep would only collide again (#1586).
pub(super) const BUNDLE_TRANSACTION_RETRY: RetryPolicy = RetryPolicy {
    max_attempts: 3,
    base: Duration::from_millis(200),
    cap: Duration::from_secs(2),
};

/// Default wall-clock budget for one transaction bundle, retries and pauses
/// included: the value of `MongoBackendConfig::bundle_transaction_budget` when
/// the embedder does not set one. A bundle's own runtime is unbounded by this
/// module (the REST layer's request timeout is the outer limit), so the budget
/// only stops a slow bundle from being started again once another run would
/// most likely overrun it. An embedder that serves requests under a timeout
/// should set the field to fit inside it, as the `hfs` binary does.
pub(super) const DEFAULT_BUNDLE_TRANSACTION_BUDGET: Duration = Duration::from_secs(120);

/// A uniform draw from `[0, 1)`, for jitter.
///
/// Taken from a v4 UUID, which the crate already depends on, rather than from a
/// new RNG dependency: the first 48 bits of a v4 UUID are all random (the
/// version and variant bits sit after them), and 48 bits fit a `f64` mantissa
/// exactly.
pub(super) fn jitter_fraction() -> f64 {
    let bits = (uuid::Uuid::new_v4().as_u128() >> 80) as u64;
    bits as f64 / (1u64 << 48) as f64
}

/// Full jitter: `fraction` of `backoff`, so the pause is anywhere from none at
/// all to the whole backoff. An out-of-range `fraction` is clamped rather than
/// trusted.
pub(super) fn full_jitter(backoff: Duration, fraction: f64) -> Duration {
    // `NaN` compares false to everything, so it must be caught before `clamp`.
    let fraction = if fraction.is_nan() {
        0.0
    } else {
        fraction.clamp(0.0, 1.0)
    };
    backoff.mul_f64(fraction)
}

/// The pause before running a transaction bundle again, or `None` when it must
/// not run again: `attempts_made` runs have already used the policy's
/// `max_attempts`, or `elapsed` plus the pause plus another run as long as the
/// `last_attempt` would pass the `budget`.
///
/// `fraction` is the jitter draw ([`jitter_fraction`]); it is a parameter so the
/// decision is deterministic under test.
pub(super) fn next_attempt_delay(
    policy: &RetryPolicy,
    attempts_made: u32,
    elapsed: Duration,
    last_attempt: Duration,
    budget: Duration,
    fraction: f64,
) -> Option<Duration> {
    if attempts_made >= policy.max_attempts {
        return None;
    }
    let delay = full_jitter(policy.backoff(attempts_made), fraction);
    if elapsed.saturating_add(delay).saturating_add(last_attempt) > budget {
        return None;
    }
    Some(delay)
}

/// An operation's final result and how many times it ran.
pub(super) struct Attempted<T> {
    pub result: Result<T, MongoError>,
    pub attempts: u32,
}

/// True when a MongoDB error is transient and safe to retry: one the driver has
/// itself labelled retryable, or a fast network/connection failure (e.g. a
/// connection reset by a momentarily overloaded server). The driver retries such
/// errors once on a replica set and never on a standalone; a server that stays
/// busy longer than that outlasts the single retry, so we add a short bounded
/// retry on top. Non-transient errors (duplicate key, bad command, decode) are
/// never retried here.
///
/// A `ServerSelection` timeout is deliberately *not* treated as transient: it
/// already means the driver waited its full `server_selection_timeout` and found
/// no usable server, so a fast backoff-retry would just pay that wait again
/// (blocking the caller for minutes against a genuinely-down server) without
/// improving the odds. Such an error is surfaced promptly instead.
///
/// An `InsertMany` error carrying per-document `write_errors` is never
/// transient whatever labels ride on it: the server executed the command and
/// reported per-document outcomes, which the caller must attribute.
pub(super) fn is_transient_mongo_error(err: &MongoError) -> bool {
    if let ErrorKind::InsertMany(insert_many) = err.kind.as_ref()
        && insert_many.write_errors.is_some()
    {
        return false;
    }
    err.contains_label(RETRYABLE_ERROR)
        || err.contains_label(RETRYABLE_WRITE_ERROR)
        || matches!(
            err.kind.as_ref(),
            ErrorKind::Io(_) | ErrorKind::ConnectionPoolCleared { .. }
        )
}

/// Runs `op` until it succeeds, fails with a non-transient error, or
/// `policy.max_attempts` is reached. A tripped `cancel` token ends the loop
/// before the next sleep, so an aborted submission never pays the backoff.
pub(super) async fn retry_transient_with<T, F, Fut>(
    policy: &RetryPolicy,
    cancel: Option<&CancelToken>,
    what: &str,
    mut op: F,
) -> Attempted<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, MongoError>>,
{
    let mut attempts: u32 = 1;
    loop {
        match op().await {
            Ok(value) => {
                return Attempted {
                    result: Ok(value),
                    attempts,
                };
            }
            Err(err) if attempts < policy.max_attempts && is_transient_mongo_error(&err) => {
                if cancel.is_some_and(CancelToken::is_cancelled) {
                    return Attempted {
                        result: Err(err),
                        attempts,
                    };
                }
                let backoff = policy.backoff(attempts);
                tracing::warn!(
                    attempt = attempts,
                    max_attempts = policy.max_attempts,
                    backoff_ms = backoff.as_millis() as u64,
                    "transient mongodb error during {what}; retrying: {err}"
                );
                tokio::time::sleep(backoff).await;
                attempts += 1;
            }
            Err(err) => {
                return Attempted {
                    result: Err(err),
                    attempts,
                };
            }
        }
    }
}

/// The settings store's shape: the fast policy, no cancellation.
///
/// The settings-store writes are already safe to re-run: reads are pure, and a
/// re-executed insert/update is caught by the version-conditioned filter and the
/// duplicate-key path in `MongoBackend::write_settings`, so a retry after a
/// lost acknowledgement cannot double-apply.
pub(super) async fn retry_transient<T, F, Fut>(op: F) -> Result<T, MongoError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, MongoError>>,
{
    retry_transient_with(&SETTINGS_RETRY, None, "user_settings", op)
        .await
        .result
}

/// Maps an operation's final driver error to a `StorageError`: a transient
/// error that outlived its retries is `Unavailable`, anything else `Internal`.
pub(super) fn exhausted(context: &str, attempts: u32, err: &MongoError) -> StorageError {
    let message = if attempts == 1 {
        format!("{context}: {err}")
    } else {
        format!("{context}: {err} (after {attempts} attempts)")
    };
    StorageError::Backend(if is_transient_mongo_error(err) {
        BackendError::Unavailable {
            backend_name: "mongodb".to_string(),
            message,
        }
    } else {
        BackendError::Internal {
            backend_name: "mongodb".to_string(),
            message,
            source: None,
        }
    })
}

/// [`exhausted`] applied to an [`Attempted`].
pub(super) fn or_exhausted<T>(context: &str, attempted: Attempted<T>) -> StorageResult<T> {
    let Attempted { result, attempts } = attempted;
    result.map_err(|err| exhausted(context, attempts, &err))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn io_error() -> MongoError {
        MongoError::from(std::io::Error::from(std::io::ErrorKind::TimedOut))
    }

    fn custom_error() -> MongoError {
        MongoError::custom("not transient")
    }

    #[test]
    fn io_and_pool_errors_are_transient_custom_is_not() {
        assert!(is_transient_mongo_error(&io_error()));
        assert!(!is_transient_mongo_error(&custom_error()));
    }

    #[tokio::test(start_paused = true)]
    async fn a_transient_error_is_retried_with_the_policy_backoff() {
        let calls = Arc::new(AtomicU32::new(0));
        let started = tokio::time::Instant::now();
        let attempted = retry_transient_with(&BULK_INGEST_RETRY, None, "test", {
            let calls = calls.clone();
            move || {
                let calls = calls.clone();
                async move {
                    if calls.fetch_add(1, Ordering::SeqCst) < 2 {
                        Err(io_error())
                    } else {
                        Ok(42)
                    }
                }
            }
        })
        .await;
        assert_eq!(attempted.result.unwrap(), 42);
        assert_eq!(attempted.attempts, 3);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        // 100 ms before attempt 2, 200 ms before attempt 3.
        assert_eq!(started.elapsed(), Duration::from_millis(300));
    }

    #[tokio::test(start_paused = true)]
    async fn a_non_transient_error_is_returned_on_the_first_attempt() {
        let started = tokio::time::Instant::now();
        let attempted = retry_transient_with(&BULK_INGEST_RETRY, None, "test", || async {
            Err::<(), _>(custom_error())
        })
        .await;
        assert!(attempted.result.is_err());
        assert_eq!(attempted.attempts, 1);
        assert_eq!(started.elapsed(), Duration::ZERO);
    }

    #[tokio::test(start_paused = true)]
    async fn exhaustion_returns_the_last_error_after_max_attempts() {
        let started = tokio::time::Instant::now();
        let attempted = retry_transient_with(&BULK_INGEST_RETRY, None, "test", || async {
            Err::<(), _>(io_error())
        })
        .await;
        assert!(attempted.result.is_err());
        assert_eq!(attempted.attempts, BULK_INGEST_RETRY.max_attempts);
        // 100 + 200 + 400 + 800 + 1000 (capped) ms.
        assert_eq!(started.elapsed(), Duration::from_millis(2500));
    }

    #[tokio::test(start_paused = true)]
    async fn the_settings_policy_sleeps_25_50_100() {
        let started = tokio::time::Instant::now();
        let attempted = retry_transient_with(&SETTINGS_RETRY, None, "test", || async {
            Err::<(), _>(io_error())
        })
        .await;
        assert_eq!(attempted.attempts, 4);
        assert_eq!(started.elapsed(), Duration::from_millis(175));
    }

    #[tokio::test(start_paused = true)]
    async fn a_tripped_cancel_token_stops_before_the_next_sleep() {
        let cancel = CancelToken::new();
        let started = tokio::time::Instant::now();
        let attempted = retry_transient_with(&BULK_INGEST_RETRY, Some(&cancel), "test", {
            let cancel = cancel.clone();
            move || {
                cancel.cancel();
                async { Err::<(), _>(io_error()) }
            }
        })
        .await;
        assert!(attempted.result.is_err());
        assert_eq!(attempted.attempts, 1, "no second attempt once cancelled");
        assert_eq!(
            started.elapsed(),
            Duration::ZERO,
            "no backoff sleep once cancelled"
        );
    }

    #[test]
    fn the_bundle_policy_is_three_attempts_backing_off_from_200ms_to_2s() {
        assert_eq!(BUNDLE_TRANSACTION_RETRY.max_attempts, 3);
        assert_eq!(
            BUNDLE_TRANSACTION_RETRY.backoff(1),
            Duration::from_millis(200)
        );
        assert_eq!(
            BUNDLE_TRANSACTION_RETRY.backoff(2),
            Duration::from_millis(400)
        );
        // Doubling stops at the cap.
        assert_eq!(BUNDLE_TRANSACTION_RETRY.backoff(5), Duration::from_secs(2));
        assert_eq!(BUNDLE_TRANSACTION_RETRY.backoff(30), Duration::from_secs(2));
    }

    #[test]
    fn full_jitter_stays_between_zero_and_the_backoff() {
        let backoff = Duration::from_millis(400);
        assert_eq!(full_jitter(backoff, 0.0), Duration::ZERO);
        assert_eq!(full_jitter(backoff, 0.5), Duration::from_millis(200));
        assert!(full_jitter(backoff, 0.999_999) < backoff);
        // Out-of-range input never escapes the bounds.
        assert_eq!(full_jitter(backoff, 1.0), backoff);
        assert_eq!(full_jitter(backoff, 7.0), backoff);
        assert_eq!(full_jitter(backoff, -1.0), Duration::ZERO);
        assert_eq!(full_jitter(backoff, f64::NAN), Duration::ZERO);
    }

    #[test]
    fn the_jitter_fraction_is_in_the_unit_interval_and_actually_varies() {
        let draws: Vec<f64> = (0..256).map(|_| jitter_fraction()).collect();
        assert!(draws.iter().all(|f| (0.0..1.0).contains(f)), "{draws:?}");
        assert!(
            draws.windows(2).any(|pair| pair[0] != pair[1]),
            "256 draws were all identical: {draws:?}"
        );
    }

    #[test]
    fn a_bundle_is_retried_only_while_attempts_remain() {
        let none_used = Duration::ZERO;
        let last = Duration::from_millis(50);
        let budget = DEFAULT_BUNDLE_TRANSACTION_BUDGET;
        // After attempts 1 and 2 another attempt is allowed; after the third it is not.
        assert!(
            next_attempt_delay(&BUNDLE_TRANSACTION_RETRY, 1, none_used, last, budget, 0.5)
                .is_some()
        );
        assert!(
            next_attempt_delay(&BUNDLE_TRANSACTION_RETRY, 2, none_used, last, budget, 0.5)
                .is_some()
        );
        assert!(
            next_attempt_delay(&BUNDLE_TRANSACTION_RETRY, 3, none_used, last, budget, 0.5)
                .is_none()
        );
    }

    #[test]
    fn the_delay_is_the_jittered_backoff_of_the_attempt_just_made() {
        let last = Duration::from_millis(50);
        let delay = |attempts_made, fraction| {
            next_attempt_delay(
                &BUNDLE_TRANSACTION_RETRY,
                attempts_made,
                Duration::ZERO,
                last,
                DEFAULT_BUNDLE_TRANSACTION_BUDGET,
                fraction,
            )
            .unwrap()
        };
        assert_eq!(delay(1, 0.0), Duration::ZERO);
        assert_eq!(delay(1, 0.5), Duration::from_millis(100));
        assert_eq!(delay(2, 0.5), Duration::from_millis(200));
        assert!(delay(2, 0.999_999) < Duration::from_millis(400));
    }

    #[test]
    fn the_default_budget_is_120_seconds() {
        assert_eq!(DEFAULT_BUNDLE_TRANSACTION_BUDGET, Duration::from_secs(120));
    }

    /// The budget is the caller's, not a constant: under the default 30 s
    /// request timeout the `hfs` binary hands in 28 s, and a replay that would
    /// start at 18 s after an 18 s attempt is refused — it could not finish
    /// before the timeout layer answers 408, so the backend gives up in time to
    /// answer 503 `Retry-After` instead.
    #[test]
    fn a_28_second_budget_refuses_a_replay_at_18_seconds_after_an_18_second_attempt() {
        let decide = |budget: Duration| {
            next_attempt_delay(
                &BUNDLE_TRANSACTION_RETRY,
                1,
                Duration::from_secs(18),
                Duration::from_secs(18),
                budget,
                0.0,
            )
        };
        // 18 s elapsed + 18 s for another run = 36 s.
        assert!(decide(Duration::from_secs(28)).is_none());
        // The same attempt fits the default budget.
        assert!(decide(DEFAULT_BUNDLE_TRANSACTION_BUDGET).is_some());
        // And a shorter attempt fits the 28 s one: 18 + 10 = 28, exactly on it.
        assert!(
            next_attempt_delay(
                &BUNDLE_TRANSACTION_RETRY,
                1,
                Duration::from_secs(18),
                Duration::from_secs(10),
                Duration::from_secs(28),
                0.0,
            )
            .is_some()
        );
        // A zero budget (a request timeout of 2 s or less) never replays.
        assert!(decide(Duration::ZERO).is_none());
    }

    #[test]
    fn a_bundle_is_not_retried_when_another_attempt_would_overrun_the_budget() {
        let budget = Duration::from_secs(120);
        let at = |elapsed_s: u64, last_s: u64| {
            next_attempt_delay(
                &BUNDLE_TRANSACTION_RETRY,
                1,
                Duration::from_secs(elapsed_s),
                Duration::from_secs(last_s),
                budget,
                0.0,
            )
        };
        assert!(at(50, 30).is_some(), "80 s of 120 s");
        assert!(at(90, 30).is_some(), "exactly on the budget still fits");
        assert!(at(91, 30).is_none(), "121 s of 120 s");
        assert!(at(100, 30).is_none());
    }

    #[test]
    fn the_sleep_is_counted_against_the_budget_too() {
        // 119 s used + a 1 s attempt fits only if the (here maximal, 200 ms)
        // pause before it is ignored — it must not be.
        let delay = next_attempt_delay(
            &BUNDLE_TRANSACTION_RETRY,
            1,
            Duration::from_secs(119),
            Duration::from_secs(1),
            Duration::from_secs(120),
            1.0,
        );
        assert!(delay.is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn the_bundle_backoff_sleeps_are_bounded_by_the_policy() {
        // Worst case (no jitter taken off): 200 ms then 400 ms.
        let started = tokio::time::Instant::now();
        for attempts_made in 1..BUNDLE_TRANSACTION_RETRY.max_attempts {
            let delay = next_attempt_delay(
                &BUNDLE_TRANSACTION_RETRY,
                attempts_made,
                started.elapsed(),
                Duration::ZERO,
                DEFAULT_BUNDLE_TRANSACTION_BUDGET,
                1.0,
            )
            .unwrap();
            tokio::time::sleep(delay).await;
        }
        assert_eq!(started.elapsed(), Duration::from_millis(600));

        // Real draws never exceed it, and never sleep negative time.
        let started = tokio::time::Instant::now();
        for attempts_made in 1..BUNDLE_TRANSACTION_RETRY.max_attempts {
            let delay = next_attempt_delay(
                &BUNDLE_TRANSACTION_RETRY,
                attempts_made,
                started.elapsed(),
                Duration::ZERO,
                DEFAULT_BUNDLE_TRANSACTION_BUDGET,
                jitter_fraction(),
            )
            .unwrap();
            tokio::time::sleep(delay).await;
        }
        assert!(started.elapsed() <= Duration::from_millis(600));
    }

    #[test]
    fn exhausted_maps_transient_to_unavailable_and_other_to_internal() {
        let transient = exhausted("insert batch resources", 6, &io_error());
        match transient {
            StorageError::Backend(BackendError::Unavailable { message, .. }) => {
                assert!(message.starts_with("insert batch resources: "));
                assert!(message.ends_with(" (after 6 attempts)"), "{message}");
            }
            other => panic!("expected Unavailable, got {other:?}"),
        }
        let internal = exhausted("insert batch resources", 1, &custom_error());
        match internal {
            StorageError::Backend(BackendError::Internal { message, .. }) => {
                assert!(message.starts_with("insert batch resources: "));
                assert!(!message.contains("attempts"), "{message}");
            }
            other => panic!("expected Internal, got {other:?}"),
        }
    }
}
