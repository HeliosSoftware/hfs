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
    fn backoff(&self, attempt: u32) -> Duration {
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
