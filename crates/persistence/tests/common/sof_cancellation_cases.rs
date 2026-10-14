//! The shared cancellation table for the SoF runners (#1864).
//!
//! Rows are the ways a run can stop ([`CancelCase`]); columns are the four
//! runners ([`Runner`]). Each runner module of `tests/sof_cancellation.rs`
//! owns one column and observes the server-side work its own way:
//!
//! - PostgreSQL: `pg_stat_activity`, statements identified by a column-alias
//!   marker ([`marker`]) rather than by their full SQL text, so the table stays
//!   valid when the emitted SQL of a shape changes;
//! - SQLite: wall time until the producer gives its pooled connection back;
//! - MongoDB: `$currentOp`, filtered on the marker;
//! - in-process: items pulled from a scan double after the stop.
//!
//! A cell that cannot apply to a runner is listed in [`NOT_APPLICABLE`] with
//! its reason, never silently skipped. A stated exception to the stop bound
//! (plan D-14) is listed in [`STOP_BOUND_EXCEPTIONS`], with its actual bound.
//!
//! Every stop asserts the run's last item: [`SofError::Cancelled`] after a
//! cancel, [`SofError::Timeout`] with the run's budget after its deadline, and
//! nothing after a drop (nobody reads it).
//!
//! This file is `#[path]`-included by `tests/sof_cancellation.rs` and uses
//! only the public runner API (`SofRunner::run_view` and
//! `SofRunner::run_view_columnar`).

#![allow(dead_code)]

use std::time::Duration;

use futures::StreamExt;
use helios_persistence::core::sof_runner::{BatchStream, RowStream, SofError};
use serde_json::{Value, json};

/// The stop bound every cell asserts: server-side work for an abandoned run
/// ends within this long after the stop event (issue #1864, target 1 s).
pub const STOP_BOUND: Duration = Duration::from_secs(1);

/// How long a cell keeps watching after [`STOP_BOUND`] to report how long the
/// abandoned work really ran (evidence only; the assertion uses the bound).
pub const OBSERVE_FOR: Duration = Duration::from_secs(60);

/// Rows of the table: the ways a run stops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelCase {
    /// The consumer stops before the first row arrived.
    CancelBeforeFirstRow,
    /// The stop arrives while the backend is still planning the statement.
    CancelWhilePlanning,
    /// The stop arrives between two chunks (pages, cursor batches).
    CancelBetweenChunks,
    /// The stop arrives while rows are streaming.
    CancelMidStream,
    /// The run reaches its row cap.
    StopAtCap,
    /// The run's deadline passes; the run ends as a timeout.
    DeadlineEndsAsTimeout,
}

/// Columns of the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Runner {
    Postgres,
    Sqlite,
    MongoDb,
    InProcess,
}

/// Every row, in table order.
pub const CASES: [CancelCase; 6] = [
    CancelCase::CancelBeforeFirstRow,
    CancelCase::CancelWhilePlanning,
    CancelCase::CancelBetweenChunks,
    CancelCase::CancelMidStream,
    CancelCase::StopAtCap,
    CancelCase::DeadlineEndsAsTimeout,
];

/// Cells that do not apply, with the reason (plan section 8.2).
pub const NOT_APPLICABLE: &[(Runner, CancelCase, &str)] = &[
    (
        Runner::Sqlite,
        CancelCase::CancelWhilePlanning,
        "SQLite prepares a statement in microseconds; there is no planning phase to interrupt",
    ),
    (
        Runner::InProcess,
        CancelCase::CancelWhilePlanning,
        "maps to a pending scan_resources, tested as cancel_before_first_row",
    ),
    (
        Runner::Postgres,
        CancelCase::CancelWhilePlanning,
        "needs raw slow-to-plan SQL the public run_view cannot issue; lives in the in-source \
         sof/postgres_docker_tests.rs",
    ),
];

/// Stated exceptions to [`STOP_BOUND`] (plan D-14): the condition under which
/// a runner cannot stop the backend's work within the bound, and what bounds
/// it instead. Each is a condition the runner's stop does not reach (a
/// deployment, a network fault, work already running inside one resource's
/// evaluation), not a table cell.
pub const STOP_BOUND_EXCEPTIONS: &[(Runner, &str, &str)] = &[
    (
        Runner::Postgres,
        "the cancel request cannot reach the server (network fault): the producer closes the \
         connection and ends within the bound, but the statement keeps running on the server",
        "the statement's SET LOCAL statement_timeout, i.e. the run's deadline",
    ),
    (
        Runner::MongoDb,
        "the deployment refuses both the own-operation killOp (found through $currentOp with \
         allUsers: false) and the kill of a read whose client disconnected: the producer ends \
         within the bound, but the initial aggregate keeps running on the server",
        "the aggregate's maxTimeMS, i.e. the run's deadline",
    ),
    (
        Runner::InProcess,
        "a FHIRPath terminology call is in flight inside the chunk evaluation: each Rayon \
         worker stops after the resource it is evaluating, and that resource waits for the \
         call",
        "FHIRPATH_TERMINOLOGY_TIMEOUT (default 30 s; 0 = none), outside the runner",
    ),
    (
        Runner::InProcess,
        "a Rayon worker is generating one resource's rows (for example sibling forEach clauses \
         whose cross product runs to millions of rows) when the stop arrives: the stop is \
         checked once per resource, so that worker finishes the resource's CPU work; the S3 \
         reads stop at once",
        "that one resource's row generation, in the helios-sof evaluator",
    ),
];

/// Whether a cell is listed in [`NOT_APPLICABLE`].
pub fn not_applicable(runner: Runner, case: CancelCase) -> Option<&'static str> {
    NOT_APPLICABLE
        .iter()
        .find(|(r, c, _)| *r == runner && *c == case)
        .map(|(_, _, reason)| *reason)
}

/// A unique column alias that identifies one run's statements on the server
/// (`pg_stat_activity.query`, `$currentOp`), whatever SQL the runner emits.
pub fn marker(prefix: &str) -> String {
    format!("{prefix}_{}", uuid::Uuid::new_v4().simple())
}

/// A Patient whose `name` and `address` arrays have `n` entries each. A
/// `forEach` over both ([`cartesian_view`]) yields `n * n` rows for it, and a
/// SQL runner sorts all of them before it can return the first one, so the
/// statement is busy for a while before any row reaches the consumer.
pub fn cartesian_patient(id: &str, n: usize) -> Value {
    let names: Vec<Value> = (0..n)
        .map(|i| json!({"family": format!("f{i:05}")}))
        .collect();
    let addresses: Vec<Value> = (0..n)
        .map(|i| json!({"city": format!("c{i:05}")}))
        .collect();
    json!({
        "resourceType": "Patient",
        "id": id,
        "name": names,
        "address": addresses,
    })
}

/// `name` × `address` over Patient, with the marker as the alias of the first
/// column so the statement can be found on the server.
pub fn cartesian_view(marker: &str) -> Value {
    json!({
        "resourceType": "ViewDefinition",
        "resource": "Patient",
        "status": "active",
        "select": [
            {"forEach": "name", "column": [{"name": marker, "path": "family"}]},
            {"forEach": "address", "column": [{"name": "city", "path": "city"}]}
        ]
    })
}

/// How long the abandoned work kept running, observed by polling `still_running`
/// every 20 ms from the stop event until it reports `false` (or [`OBSERVE_FOR`]
/// passes). Returns `(running_at_bound, ran_for)`: whether it was still running
/// at [`STOP_BOUND`], and for how long it ran in total.
pub async fn observe_after_stop<F, Fut>(
    stopped_at: std::time::Instant,
    mut still_running: F,
) -> (bool, Duration)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let mut running_at_bound = None;
    loop {
        let running = still_running().await;
        let elapsed = stopped_at.elapsed();
        if running_at_bound.is_none() && elapsed >= STOP_BOUND {
            running_at_bound = Some(running);
        }
        if !running {
            return (running_at_bound.unwrap_or(false), elapsed);
        }
        if elapsed >= OBSERVE_FOR {
            return (running_at_bound.unwrap_or(true), elapsed);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// How a stopped run ended, read from its stream.
#[derive(Debug)]
pub struct Drained {
    /// Rows read before the end.
    pub rows: usize,
    /// The last item, if it was an error.
    pub error: Option<SofError>,
    /// Whether the stream ended within the wait.
    pub ended: bool,
}

/// Reads `rows` to its end (or until `wait` passes), counting rows and
/// keeping the error item that ends it.
pub async fn drain(mut rows: RowStream, wait: Duration) -> Drained {
    let mut drained = Drained {
        rows: 0,
        error: None,
        ended: false,
    };
    let read = async {
        while let Some(item) = rows.next().await {
            match item {
                Ok(_) => drained.rows += 1,
                Err(e) => drained.error = Some(e),
            }
        }
    };
    drained.ended = tokio::time::timeout(wait, read).await.is_ok();
    drained
}

/// A columnar run's batches as a [`RowStream`] for [`drain`]: one item per
/// record batch (its row count), errors in place. [`Drained::rows`] then
/// counts batches.
pub fn batches_as_items(batches: BatchStream) -> RowStream {
    Box::pin(batches.map(|item| item.map(|batch| Value::from(batch.num_rows()))))
}

/// Asserts that a run stopped by its cancel token ended with
/// [`SofError::Cancelled`] as its last item.
pub fn assert_cancelled(cell: &str, drained: &Drained) {
    assert!(
        drained.ended,
        "{cell}: the stream did not end after the cancel"
    );
    assert!(
        matches!(drained.error, Some(SofError::Cancelled)),
        "{cell}: expected a Cancelled item, got {:?}",
        drained.error
    );
}

/// Asserts that a run stopped by its deadline ended with
/// [`SofError::Timeout`] carrying a budget close to `budget`.
pub fn assert_timed_out(cell: &str, drained: &Drained, budget: Duration) {
    assert!(
        drained.ended,
        "{cell}: the stream did not end after the deadline"
    );
    match &drained.error {
        Some(SofError::Timeout {
            budget: Some(got), ..
        }) => assert!(
            *got <= budget && budget - *got < Duration::from_millis(500),
            "{cell}: budget {got:?}, expected about {budget:?}"
        ),
        other => panic!("{cell}: expected a Timeout with the run's budget, got {other:?}"),
    }
}
