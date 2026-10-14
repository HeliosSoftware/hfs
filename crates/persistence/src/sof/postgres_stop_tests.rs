//! Pure tests of the PostgreSQL runner's stop rules (#1864): the statement
//! timeout from the time left, the connection's fate after a cancel, the
//! cause-aware `57014`, and the cleanup envelope. No database needed; the
//! cells that need a server are in `postgres_docker_tests.rs`.

use std::time::Duration;

use tokio_postgres::error::SqlState;

use super::*;
use crate::core::sof_runner::{RunContext, RunKind};

fn stop_with_deadline(left: Option<Duration>) -> RunStop {
    let mut ctx = RunContext::detached(RunKind::ExportJob);
    ctx.deadline = left.map(|left| tokio::time::Instant::now() + left);
    RunStop::new(&ctx)
}

/// R-b: each transaction's `SET LOCAL statement_timeout` is the time left,
/// rounded up to whole milliseconds, never 0 (which would disable the limit);
/// a deadline that already passed runs nothing; no deadline sends no `SET
/// LOCAL`.
#[tokio::test(start_paused = true)]
async fn remaining_deadline_rounds_up_and_never_reaches_zero() {
    assert_eq!(
        timeout_for_next_transaction(&stop_with_deadline(None)),
        Ok(None)
    );

    let stop = stop_with_deadline(Some(Duration::from_millis(1500)));
    assert_eq!(timeout_for_next_transaction(&stop), Ok(Some(1500)));
    tokio::time::advance(Duration::from_micros(400)).await;
    // 1499.6 ms left: rounded up, so the server never stops early.
    assert_eq!(timeout_for_next_transaction(&stop), Ok(Some(1500)));
    tokio::time::advance(Duration::from_micros(1_499_599)).await;
    // 1 µs left: still 1 ms, never 0.
    assert_eq!(timeout_for_next_transaction(&stop), Ok(Some(1)));
    tokio::time::advance(Duration::from_micros(1)).await;
    assert_eq!(
        timeout_for_next_transaction(&stop),
        Err(StopReason::DeadlineExceeded)
    );

    assert_eq!(begin_sql(None), "BEGIN READ ONLY");
    assert_eq!(
        begin_sql(Some(1500)),
        "BEGIN READ ONLY; SET LOCAL statement_timeout = 1500"
    );
    let huge = stop_with_deadline(Some(Duration::from_secs(10 * 365 * 24 * 3600)));
    assert_eq!(timeout_for_next_transaction(&huge), Ok(Some(i32::MAX)));
}

/// A9/R-m: after a cancel request, only a statement that ended with exactly
/// the cancel's own error consumed it; anything else may leave the cancel in
/// flight, so the connection is closed.
#[test]
fn only_the_exact_user_cancel_error_allows_reuse() {
    let failed = |code: &str, message: &str| DrainOutcome::Failed {
        sqlstate: Some(code.to_string()),
        message: Some(message.to_string()),
    };
    assert_eq!(
        fate_after_cancel(&failed("57014", USER_CANCEL_MESSAGE)),
        ConnectionFate::Reuse
    );
    for outcome in [
        DrainOutcome::Completed,
        DrainOutcome::TimedOut,
        // The run's own statement_timeout fired before the cancel arrived.
        failed("57014", "canceling statement due to statement timeout"),
        // A localized server (lc_messages=de_DE).
        failed("57014", "Abbruch der Anweisung wegen Benutzeranforderung"),
        failed("40001", "could not serialize access"),
        DrainOutcome::Failed {
            sqlstate: None,
            message: None,
        },
    ] {
        assert_eq!(
            fate_after_cancel(&outcome),
            ConnectionFate::Close,
            "{outcome:?}"
        );
    }
}

/// C6/R-o: a server-side `57014` that no local stop explains is classified
/// by the deadline: a timeout (and the connection is reused) when the run's
/// `SET LOCAL` or the session's own timeout explains it, a backend fault with
/// its SQLSTATE (and the connection closed) otherwise.
#[test]
fn a_server_side_57014_is_classified_by_the_deadline() {
    let canceled = Some(&SqlState::QUERY_CANCELED);
    let budget = Some(Duration::from_secs(3));

    let (error, fate) = classify_failure(canceled, "x".into(), false, StatementLimit::Session);
    assert!(
        matches!(error, SofError::Timeout { budget: None, .. }),
        "{error:?}"
    );
    assert_eq!(fate, ConnectionFate::Reuse);

    let (error, fate) = classify_failure(
        canceled,
        "x".into(),
        false,
        StatementLimit::RunDeadline {
            passed: true,
            budget,
        },
    );
    assert!(
        matches!(error, SofError::Timeout { budget: Some(b), .. } if b == Duration::from_secs(3)),
        "{error:?}"
    );
    assert_eq!(fate, ConnectionFate::Reuse);

    let (error, fate) = classify_failure(
        canceled,
        "x".into(),
        false,
        StatementLimit::RunDeadline {
            passed: false,
            budget,
        },
    );
    assert!(
        matches!(&error, SofError::Backend { sqlstate: Some(s), .. } if s == "57014"),
        "{error:?}"
    );
    assert_eq!(fate, ConnectionFate::Close);

    let (error, fate) = classify_failure(
        Some(&SqlState::UNDEFINED_TABLE),
        "x".into(),
        false,
        StatementLimit::Session,
    );
    assert!(
        matches!(&error, SofError::Backend { sqlstate: Some(s), .. } if s == "42P01"),
        "{error:?}"
    );
    assert_eq!(fate, ConnectionFate::Reuse);

    let (error, fate) = classify_failure(
        Some(&SqlState::TOO_MANY_CONNECTIONS),
        "x".into(),
        false,
        StatementLimit::Session,
    );
    assert!(matches!(error, SofError::Unavailable { .. }), "{error:?}");
    assert_eq!(fate, ConnectionFate::Reuse);

    let (_, fate) = classify_failure(None, "closed".into(), true, StatementLimit::Session);
    assert_eq!(
        fate,
        ConnectionFate::Close,
        "a closed connection is never reused"
    );
}

/// A5: a server fault keeps its SQLSTATE on the error and in the run's log.
#[test]
#[tracing_test::traced_test]
fn backend_error_keeps_its_sqlstate() {
    let (error, _) = classify_failure(
        Some(&SqlState::UNDEFINED_TABLE),
        "query execution failed: ERROR (SQLSTATE 42P01): relation \"nope\" does not exist".into(),
        false,
        StatementLimit::Session,
    );
    assert!(
        matches!(&error, SofError::Backend { sqlstate: Some(s), .. } if s == "42P01"),
        "{error:?}"
    );
    log_run_failure(&error);
    assert!(logs_contain("SoF PostgreSQL run failed"));
    assert!(logs_contain("sqlstate=\"42P01\"") || logs_contain("sqlstate=42P01"));
}

/// R-n: the cleanup phases fit the envelope, and a deadline that reaches a
/// server-bounded step (the backstop) always closes the connection.
#[test]
fn the_cleanup_envelope_bounds_every_phase() {
    assert!(CANCEL_REQUEST_TIMEOUT + CANCEL_DRAIN_TIMEOUT + ROLLBACK_TIMEOUT <= CLEANUP_ENVELOPE);
    assert!(CLEANUP_ENVELOPE <= Duration::from_secs(1));
    assert_eq!(
        backstop_fate(StopReason::DeadlineExceeded, ConnectionFate::Reuse),
        ConnectionFate::Close
    );
    assert_eq!(
        backstop_fate(StopReason::Cancelled, ConnectionFate::Reuse),
        ConnectionFate::Reuse
    );
    assert_eq!(
        backstop_fate(StopReason::Dropped, ConnectionFate::Close),
        ConnectionFate::Close
    );
}
