//! SQL-on-FHIR runner abstraction.
//!
//! This module defines the [`SofRunner`] trait, implemented per-backend. Two
//! strategies exist:
//!
//! - **In-DB runners** compile a [`ViewDefinition`] to a native query executed
//!   inside the storage backend, skipping FHIRPath evaluation entirely: SQLite
//!   and PostgreSQL compile to SQL, MongoDB compiles to an aggregation pipeline.
//! - **In-process runners** stream the resources out of a backend that has no
//!   query engine (S3 object storage, and S3-primary composites) and evaluate
//!   the view with the `helios-sof` FHIRPath engine
//!   ([`InProcessSofRunner`](crate::sof::in_process::InProcessSofRunner)).
//!
//! If the configured backend provides no runner at all, the
//! `$viewdefinition-run` handler returns `501 Not Implemented`. Inline
//! `resource:` parameters are materialised into a transient in-memory SQLite
//! backend so they reuse the same in-DB pipeline.
//!
//! The handler layer streams the result rows directly into the HTTP response.
//!
//! Every runner streams rows from a spawned task (`tokio::spawn` or
//! `spawn_blocking`) that owns the channel's sender. Runners must start that
//! task and hand its `JoinHandle` to [`watch_row_producer`] so a panic or
//! cancellation inside the task surfaces as an `Err` item on the stream
//! instead of silently looking like a clean end of stream.
//!
//! The SQL in-DB runners send consecutive rows in batches
//! ([`row_batch_channel`]) instead of one channel message per row; the
//! returned [`RowStream`] flattens them, so consumers still see one item per
//! row, in the same order, with every error item in its original position.

use std::pin::Pin;

use async_trait::async_trait;
use futures::{Stream, StreamExt as _};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_stream::wrappers::ReceiverStream;
use tracing::warn;

use crate::tenant::TenantContext;

/// Filters that narrow which resources are processed by a view run.
///
/// Per the SQL-on-FHIR v2 spec, `patient` and `group` are `0..*` — supplying
/// multiple values must include resources matching ANY of them (union of the
/// corresponding compartments).
///
/// `patient`, `group` and `_since` apply to every scan of the resources table:
/// each `unionAll` branch, each `repeat` seed and join-back. A non-empty
/// `group` that resolves to no Patient members, with no `patient`, selects
/// nothing. The SQL runners lower the filters structurally into every scan the
/// emitter writes (`crate::sof::emit::ResourcePredicates`), so no part of a
/// view runs unfiltered.
#[derive(Debug, Clone, Default)]
pub struct ViewFilters {
    /// Restrict to resources belonging to these patients (FHIR references,
    /// e.g. `Patient/123`). Multiple values are unioned: a resource that
    /// matches any reference is included.
    pub patient: Vec<String>,

    /// Restrict to resources belonging to these groups (FHIR references,
    /// e.g. `Group/abc`). Multiple values are unioned.
    pub group: Vec<String>,

    /// Include only resources last-modified at or after this instant (RFC 3339).
    /// Every runner compares inclusively, and a resource with no last-modified
    /// time is excluded.
    pub since: Option<chrono::DateTime<chrono::Utc>>,

    /// Maximum number of output rows to return (across all pages).
    ///
    /// The SQL in-DB runners apply a value representable as `i64` as one
    /// final SQL `LIMIT`, returning exactly the first `n` rows of the
    /// unlimited result; larger values are enforced only by the client-side
    /// cap. See [`SofRunner`] for the ordering contract.
    pub limit: Option<usize>,
}

/// A single output row from a view run.
///
/// Each row is a flat JSON object whose keys come from the ViewDefinition's `select`
/// columns. Nested columns are dot-joined by convention (`name.family`).
pub type ViewRow = Value;

/// A pinned, heap-allocated, `Send + 'static` stream of view rows.
///
/// Streams returned by runners must own all their state (e.g. via cloned
/// `Arc`s or owned `Vec`s) so that the caller can move them across tasks
/// — for example, into an HTTP response body. The previous `'a` lifetime
/// turned out to be unused by every implementation and prevented streaming
/// responses, so it was removed.
pub type RowStream = Pin<Box<dyn Stream<Item = Result<ViewRow, SofError>> + Send + 'static>>;

/// Errors that can occur during SQL-on-FHIR view execution.
#[derive(Debug, thiserror::Error)]
pub enum SofError {
    /// The ViewDefinition contains constructs that this runner cannot compile or execute.
    ///
    /// The `reason` field describes which construct is unsupported. The handler layer
    /// maps this variant to a `422 Unprocessable Entity` OperationOutcome.
    #[error("view definition is not compilable by this runner: {reason}")]
    Uncompilable {
        /// Human-readable description of the unsupported construct.
        reason: String,
    },

    /// The ViewDefinition JSON is structurally invalid (missing required fields, wrong types).
    #[error("invalid view definition: {0}")]
    InvalidViewDefinition(String),

    /// An error occurred while fetching resources from the storage backend.
    #[error("storage error: {0}")]
    Storage(String),

    /// A backend-level SQL or driver error.
    #[error("backend error: {0}")]
    Backend(String),

    /// The view run was cancelled (e.g. client disconnected, export job cancelled).
    #[error("view run cancelled")]
    Cancelled,
}

/// Sender half of a runner's row channel.
pub type RowSender = mpsc::Sender<Result<ViewRow, SofError>>;

/// Consecutive output rows, in result order, sent as one channel message.
pub type RowBatch = Vec<ViewRow>;

/// Sender half of a batching runner's row channel ([`row_batch_channel`]).
pub type RowBatchSender = mpsc::Sender<Result<RowBatch, SofError>>;

/// Most rows a batching runner puts in one [`RowBatch`].
pub const ROW_BATCH_MAX_ROWS: usize = 256;

/// Batches a batching runner may queue ahead of its consumer. One is
/// enough to keep producer and consumer overlapping (the producer fills the
/// next batch while one waits and the consumer works through another), and
/// keeps a stream's buffered rows — at most three batches: queued, being
/// filled, being consumed — close to the per-row channel's 256.
const ROW_BATCH_DEPTH: usize = 1;

/// Creates a batching runner's row channel: the producer sends
/// `Ok(batch)` and `Err(error)` messages, and the returned [`RowStream`]
/// yields each batch's rows one by one, then any error, in send order.
///
/// A producer must send a partial batch before an error item and at the end
/// of its stream, so neither the order of rows and errors nor the end of a
/// short result is delayed by batching. The sender works with
/// [`watch_row_producer`] like a per-row sender does.
pub fn row_batch_channel() -> (RowBatchSender, RowStream) {
    let (tx, rx) = mpsc::channel::<Result<RowBatch, SofError>>(ROW_BATCH_DEPTH);
    let rows = ReceiverStream::new(rx).flat_map(|message| {
        let (rows, error) = match message {
            Ok(rows) => (rows, None),
            Err(error) => (RowBatch::new(), Some(error)),
        };
        futures::stream::iter(rows.into_iter().map(Ok).chain(error.map(Err)))
    });
    (tx, Box::pin(rows))
}

/// Watches a spawned row producer and reports its death on the channel.
///
/// `producer` is the `JoinHandle` of the task (`tokio::spawn` or
/// `spawn_blocking`) that owns the channel's original `Sender`; `tx` is a
/// clone of that sender kept outside the task — a per-row [`RowSender`] or
/// a [`RowBatchSender`]. If the task ends with a
/// `JoinError` (panic or cancellation), an
/// `Err(SofError::Backend("row producer failed: …"))` is sent through `tx`
/// and a `warn!` names the runner; if it ends normally, nothing is sent.
/// Because `tx` stays alive until the outcome is known, the receiver only
/// observes end-of-stream after the producer's fate has been reported —
/// a partial row set can never look like a complete one.
pub fn watch_row_producer<T: Send + 'static>(
    runner: &'static str,
    tx: mpsc::Sender<Result<T, SofError>>,
    producer: JoinHandle<()>,
) {
    tokio::spawn(async move {
        if let Err(e) = producer.await {
            warn!(
                runner,
                error = %e,
                "SoF row producer ended before finishing its stream"
            );
            let _ = tx
                .send(Err(SofError::Backend(format!("row producer failed: {e}"))))
                .await;
        }
    });
}

/// Abstraction over in-process and in-DB SQL-on-FHIR execution strategies.
///
/// # Object safety
///
/// `SofRunner` is object-safe and intended for use as `Arc<dyn SofRunner>`. The
/// [`run_view`] method returns a heap-allocated [`RowStream`] to avoid associated
/// types that would break object safety.
///
/// # Threading
///
/// Implementors must be `Send + Sync` so that the runner can be stored in `AppState`
/// and shared across request tasks.
///
/// # Ordering (SQLite and PostgreSQL runners)
///
/// Every SQL-runner result has a total, deterministic order (all keys
/// ascending):
///
/// - ordinary and expanded views: resource `last_updated`, resource id, then
///   every expansion occurrence ordinal;
/// - `unionAll` and `repeat`: the first visible column, as before, with
///   explicit NULL placement (PostgreSQL `NULLS LAST`, SQLite `NULLS FIRST`),
///   then deterministic resource, branch and occurrence/traversal
///   tie-breakers.
///
/// [`ViewFilters::limit`] (when representable as `i64`) becomes one final SQL
/// `LIMIT` applied after every filter, expansion, union and recursion, so a
/// limited run returns exactly the first `n` rows of the unlimited run.
///
/// Compatibility: rows that previously tied may arrive in a different, now
/// fixed, order — in unlimited runs, in `$sql-export` shards (contiguous
/// slices of the stream) and in SQLQuery dependency insertion order. The
/// ordering and the final `LIMIT` never change which rows are produced. Each
/// backend's order is deterministic, but PostgreSQL and SQLite are not
/// guaranteed to agree with each other (collation, NULL placement). The
/// MongoDB and in-process runners are not covered by this contract.
///
/// The same change also corrects SQL-runner results where they disagreed
/// with the in-process evaluator:
///
/// - `%rowIndex` under `repeat` and indexed `forEach: "<chain>[N]"` now
///   matches the evaluator (indexed scopes are always `0`);
/// - an indexed `forEach` drops the enclosing row when its selection is
///   absent or rejected by a trailing `where(crit)` (FHIRPath indexes first,
///   then filters), and `forEachOrNull` yields the empty context instead —
///   rows previously emitted for such selections are gone;
/// - `_since` and Patient/Group runtime filters apply to every `unionAll`
///   branch, every `repeat` seed and the `repeat` resource rejoin, not just
///   one;
/// - SQL-runner buffered and tabular formats (JSON, CSV, Parquet, Arrow, and
///   CSV/Parquet export shards) carry every declared column, even when
///   PostgreSQL omits a NULL-valued key from the leading rows.
///
/// The canonical per-shape key list lives with the emitter:
/// [the ordering contract](crate::sof::emit#structured-composition-and-the-ordering-contract).
///
/// # Dropping the stream (SQLite and PostgreSQL runners)
///
/// A consumer that stops early (client disconnect, cancelled or failed
/// export job) only has to drop the [`RowStream`]. The SQL runners notice
/// promptly — also before the first row, while the database is still
/// sorting — and stop the statement instead of letting it run to the end:
///
/// - on PostgreSQL the server-side query is cancelled (a cancel request for
///   that backend, sent while the runner still holds the pooled
///   connection), and the statement is drained to its end before the
///   connection goes back to the pool; a connection is reused only when
///   its statement ended with the cancel request's own error (not, e.g.,
///   `statement_timeout`'s), and is otherwise — or past a bounded time —
///   closed, so a cancel can never reach another borrower's statement;
/// - on SQLite the statement is interrupted (`sqlite3_interrupt`), and the
///   interrupt is disarmed before the connection is reused or returned to
///   the pool, so it can only ever hit that statement.
///
/// A statement that completes normally is never cancelled, and the
/// deliberate stop is not reported as an error. The MongoDB and in-process
/// runners only stop once their next send finds the consumer gone.
#[async_trait]
pub trait SofRunner: Send + Sync {
    /// Execute a ViewDefinition and return a stream of output rows.
    ///
    /// # Arguments
    ///
    /// * `tenant` — The tenant context; all resource access is scoped to this tenant.
    /// * `view_definition` — The raw ViewDefinition JSON (any FHIR version).
    /// * `filters` — Optional filters (patient, group, since, limit).
    ///
    /// # Returns
    ///
    /// A [`RowStream`] that yields one flat JSON object per output row. The stream
    /// may be infinite in theory; callers should honour the `filters.limit` cap or
    /// impose their own. SQL in-DB runners yield rows in the order described
    /// under [Ordering](SofRunner#ordering-sqlite-and-postgresql-runners).
    ///
    /// # Errors
    ///
    /// Returns [`SofError::Uncompilable`] synchronously (before the stream is polled)
    /// when this runner cannot handle the given ViewDefinition. The handler layer
    /// must catch this and either fall back to the in-process runner or return `422`.
    async fn run_view(
        &self,
        tenant: &TenantContext,
        view_definition: Value,
        filters: ViewFilters,
    ) -> Result<RowStream, SofError>;

    /// Returns a human-readable name for this runner (used in logs and diagnostics).
    fn runner_name(&self) -> &'static str;
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;
    use tokio_stream::wrappers::ReceiverStream;

    use super::*;

    /// Creates a row channel and its `ReceiverStream` wrapper, mirroring the
    /// buffer size and stream construction every real runner uses.
    fn channel() -> (RowSender, RowStream) {
        let (tx, rx) = mpsc::channel(8);
        (tx, Box::pin(ReceiverStream::new(rx)))
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn panicking_producer_yields_error_item_then_end_of_stream() {
        let (tx, mut stream) = channel();
        let producer_tx = tx.clone();
        let producer = tokio::task::spawn_blocking(move || {
            for i in 0..3 {
                if producer_tx.blocking_send(Ok(json!({"n": i}))).is_err() {
                    return;
                }
            }
            panic!("boom");
        });
        watch_row_producer("test-runner", tx, producer);

        tokio::time::timeout(Duration::from_secs(10), async {
            for i in 0..3 {
                let item = stream.next().await.expect("expected row");
                assert!(matches!(&item, Ok(v) if *v == json!({"n": i})));
            }
            let error_item = stream.next().await.expect("expected error item");
            match error_item {
                Err(SofError::Backend(m)) => {
                    assert!(m.contains("row producer failed"), "unexpected message: {m}");
                }
                other => panic!("expected SofError::Backend, got {other:?}"),
            }
            assert!(stream.next().await.is_none());
        })
        .await
        .expect("stream consumption must not hang");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancelled_producer_yields_error_item() {
        let (tx, mut stream) = channel();
        let producer_tx = tx.clone();
        let producer = tokio::spawn(async move {
            let _ = producer_tx.send(Ok(json!({"n": 0}))).await;
            // Hold `producer_tx` open while pending so the channel closes
            // only when the task is aborted, not on its own.
            std::future::pending::<()>().await;
        });
        let abort_handle = producer.abort_handle();
        watch_row_producer("test-runner", tx, producer);

        tokio::time::timeout(Duration::from_secs(10), async {
            let first = stream.next().await.expect("expected first row");
            assert!(matches!(&first, Ok(v) if *v == json!({"n": 0})));

            abort_handle.abort();

            let error_item = stream.next().await.expect("expected error item");
            match error_item {
                Err(SofError::Backend(m)) => {
                    assert!(m.contains("cancelled"), "unexpected message: {m}");
                }
                other => panic!("expected SofError::Backend, got {other:?}"),
            }
            assert!(stream.next().await.is_none());
        })
        .await
        .expect("stream consumption must not hang");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn clean_producer_ends_stream_without_error() {
        let (tx, mut stream) = channel();
        let producer_tx = tx.clone();
        let producer = tokio::task::spawn_blocking(move || {
            for i in 0..2 {
                if producer_tx.blocking_send(Ok(json!({"n": i}))).is_err() {
                    return;
                }
            }
        });
        watch_row_producer("test-runner", tx, producer);

        tokio::time::timeout(Duration::from_secs(10), async {
            let first = stream.next().await;
            assert!(matches!(&first, Some(Ok(v)) if *v == json!({"n": 0})));
            let second = stream.next().await;
            assert!(matches!(&second, Some(Ok(v)) if *v == json!({"n": 1})));
            let end = stream.next().await;
            assert!(end.is_none());
        })
        .await
        .expect("stream consumption must not hang");
    }

    /// Batches flatten into one item per row, in send order, with each error
    /// item between the rows sent before and after it; empty batches vanish.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn batched_rows_flatten_in_order_with_errors_in_place() {
        let (tx, mut stream) = row_batch_channel();
        let producer = tokio::spawn(async move {
            let rows = |range: std::ops::Range<i32>| range.map(|n| json!({"n": n})).collect();
            tx.send(Ok(rows(0..300))).await.unwrap();
            tx.send(Ok(Vec::new())).await.unwrap();
            tx.send(Ok(rows(300..301))).await.unwrap();
            tx.send(Err(SofError::Backend("mid".into()))).await.unwrap();
            tx.send(Ok(rows(301..303))).await.unwrap();
            tx.send(Err(SofError::Backend("last".into())))
                .await
                .unwrap();
        });

        let items = tokio::time::timeout(Duration::from_secs(10), async {
            let mut items = Vec::new();
            while let Some(item) = stream.next().await {
                items.push(item.map_err(|e| e.to_string()));
            }
            items
        })
        .await
        .expect("stream consumption must not hang");
        producer.await.unwrap();

        let mut expected: Vec<Result<Value, String>> =
            (0..301).map(|n| Ok(json!({"n": n}))).collect();
        expected.push(Err("backend error: mid".into()));
        expected.extend((301..303).map(|n| Ok(json!({"n": n}))));
        expected.push(Err("backend error: last".into()));
        assert_eq!(items, expected);
    }

    /// The batched channel keeps `watch_row_producer`'s guarantee: rows sent
    /// before a panic, then the producer-failed error, then end of stream.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn panicking_batch_producer_yields_rows_then_error_item() {
        let (tx, mut stream) = row_batch_channel();
        let producer_tx = tx.clone();
        let producer = tokio::task::spawn_blocking(move || {
            let batch = (0..3).map(|n| json!({"n": n})).collect();
            if producer_tx.blocking_send(Ok(batch)).is_err() {
                return;
            }
            panic!("boom");
        });
        watch_row_producer("test-runner", tx, producer);

        tokio::time::timeout(Duration::from_secs(10), async {
            for i in 0..3 {
                let item = stream.next().await.expect("expected row");
                assert!(matches!(&item, Ok(v) if *v == json!({"n": i})));
            }
            match stream.next().await.expect("expected error item") {
                Err(SofError::Backend(m)) => {
                    assert!(m.contains("row producer failed"), "unexpected message: {m}");
                }
                other => panic!("expected SofError::Backend, got {other:?}"),
            }
            assert!(stream.next().await.is_none());
        })
        .await
        .expect("stream consumption must not hang");
    }
}
