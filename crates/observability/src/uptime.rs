//! Process uptime tracking.
//!
//! [`init`] is called once at server startup to record the process start time.
//! The accessors are cheap and lock-free, suitable for calling on every
//! `/health` or `/metrics` request.

use std::sync::OnceLock;
use std::time::Instant;

static START_INSTANT: OnceLock<Instant> = OnceLock::new();
static STARTED_AT: OnceLock<chrono::DateTime<chrono::Utc>> = OnceLock::new();

/// Record the process start time. Idempotent — the first call wins, so calling
/// it more than once (e.g. from tests) is harmless.
pub fn init() {
    let _ = START_INSTANT.set(Instant::now());
    let _ = STARTED_AT.set(chrono::Utc::now());
}

/// Seconds elapsed since [`init`] was called. Returns `0.0` if `init` was never
/// called (e.g. in a unit test that exercises a handler directly).
pub fn uptime_seconds() -> f64 {
    START_INSTANT
        .get()
        .map(|start| start.elapsed().as_secs_f64())
        .unwrap_or(0.0)
}

/// RFC 3339 timestamp of when the process started, or an empty string if
/// [`init`] was never called.
pub fn started_at_rfc3339() -> String {
    STARTED_AT.get().map(|t| t.to_rfc3339()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uptime_is_non_negative_after_init() {
        init();
        assert!(uptime_seconds() >= 0.0);
        assert!(!started_at_rfc3339().is_empty());
    }
}
