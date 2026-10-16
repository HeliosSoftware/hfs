//! SQL-on-FHIR runner abstraction.
//!
//! This module defines the [`SofRunner`] trait, implemented per-backend. Two
//! strategies exist:
//!
//! - **In-DB runners** compile a `ViewDefinition` to a native query executed
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
//! `spawn_blocking`) that owns the channel's sender. The SQL runners use the
//! producer core in [`sof_producer`](super::sof_producer) (a batch channel with
//! an end marker, supervised tasks and one stop signal); the other runners
//! start their task and hand its `JoinHandle` to [`watch_row_producer`]. Either
//! way a panic or cancellation inside the task surfaces as an `Err` item on the
//! stream instead of silently looking like a clean end of stream.

use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::Stream;
use helios_sof::arrow::record_batch::RecordBatch;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use super::sof_schema::ViewSchema;
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
/// nothing. A view whose scans cannot all take the filters is an error, never
/// an unfiltered run.
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

/// What one view run asks for: the view and the filters that narrow it.
///
/// The row cap stays in [`ViewFilters::limit`].
#[derive(Debug, Clone)]
pub struct RunRequest {
    /// The raw ViewDefinition JSON (any FHIR version).
    pub view: Value,
    /// Filters (patient, group, since, limit).
    pub filters: ViewFilters,
}

impl RunRequest {
    /// A request for `view` narrowed by `filters`.
    pub fn new(view: Value, filters: ViewFilters) -> Self {
        Self { view, filters }
    }
}

/// Why a view runs, which decides its budget.
///
/// The REST layer turns the kind into the run's
/// [`deadline`](RunContext::deadline); runners read only the deadline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RunKind {
    /// A capped interactive `$viewdefinition-run` (a preview).
    Preview,
    /// A `$viewdefinition-run` without a row cap.
    UnlimitedRun,
    /// One subject (or leaf) of an export job.
    ExportJob,
    /// A ViewDefinition a SQL Query Library depends on, materialized for the
    /// query.
    SqlQueryDependency,
}

/// How a caller can stop a run: a cancel signal, an optional deadline, and the
/// run's kind.
///
/// Dropping the returned row stream also counts as a cancel. See
/// [`SofRunner::run_view`] for what each stop does to the stream.
#[derive(Debug, Clone)]
pub struct RunContext {
    /// Fired by the caller to cancel the run.
    pub cancel: CancellationToken,
    /// The instant by which the run must have ended, if any. A
    /// [`tokio::time::Instant`], so paused-clock tests can drive it.
    pub deadline: Option<tokio::time::Instant>,
    /// Why the view runs.
    pub kind: RunKind,
}

impl RunContext {
    /// A context nothing else cancels, with no deadline: the run stops only
    /// when its stream is dropped.
    pub fn detached(kind: RunKind) -> Self {
        Self {
            cancel: CancellationToken::new(),
            deadline: None,
            kind,
        }
    }
}

/// What a runner declares about one run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RunCapabilities {
    /// The rows come out in a total order. Which key that is depends on the
    /// runner and the run kind: `(last_updated, id)` for a SQL preview or
    /// unlimited run, the resource id for PostgreSQL export pages. SQL
    /// flat/`forEach` runs have one; SQL `unionAll`/`repeat` runs, MongoDB and
    /// the in-process runner do not.
    pub total_order: bool,
    /// The runner declared [`RunOutput::schema`]. `false` means the schema is
    /// empty and says nothing about the rows.
    pub declared_schema: bool,
    /// Stopping the run also stops the work the database server is doing for
    /// it, not only the client side of the run.
    pub cancels_server_side: bool,
}

/// The result of starting a view run: its declared schema, its capabilities and
/// its rows.
pub struct RunOutput {
    /// The output columns, in ViewDefinition document order.
    pub schema: ViewSchema,
    /// What the runner declares about this run.
    pub capabilities: RunCapabilities,
    /// The rows, one flat JSON object each. Every row has exactly the keys of
    /// [`schema`](Self::schema), in its order, with an explicit `null` for an
    /// empty column (see [`ViewSchema`]).
    pub rows: RowStream,
}

impl RunOutput {
    /// An output with `schema`, `capabilities` and `rows`.
    pub fn new(schema: ViewSchema, capabilities: RunCapabilities, rows: RowStream) -> Self {
        Self {
            schema,
            capabilities,
            rows,
        }
    }

    /// An output that declares nothing: an empty schema and no capabilities.
    /// For wrappers and test doubles that only produce rows.
    pub fn from_rows(rows: RowStream) -> Self {
        Self::new(ViewSchema::default(), RunCapabilities::default(), rows)
    }
}

impl std::fmt::Debug for RunOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunOutput")
            .field("schema", &self.schema)
            .field("capabilities", &self.capabilities)
            .field("rows", &"<row stream>")
            .finish()
    }
}

/// Errors that can occur during SQL-on-FHIR view execution.
///
/// Each variant is one *kind* of failure, and every caller maps a kind to the
/// same answer: the REST layer has one mapping (`map_sof_error_to_rest` in
/// `helios-rest`), used by `$sql-run` (ViewDefinition and Library subjects)
/// and by the export job status.
///
/// | Kind | `$sql-run` answer |
/// |---|---|
/// | [`Uncompilable`](Self::Uncompilable), [`InvalidViewDefinition`](Self::InvalidViewDefinition), [`ResolutionLimit`](Self::ResolutionLimit) | 422 |
/// | [`ClientInput`](Self::ClientInput) | its `status` (422, or 400 for any other value) |
/// | [`NotFound`](Self::NotFound) | 400, OperationOutcome code `not-found` |
/// | [`Unavailable`](Self::Unavailable) | 503 with `Retry-After` |
/// | [`Timeout`](Self::Timeout) | 504, OperationOutcome code `timeout` |
/// | [`Cancelled`](Self::Cancelled) | an export job reports its own status; a `$sql-run` client has already gone or been answered by the request timeout |
/// | [`Storage`](Self::Storage), [`Backend`](Self::Backend) | 500 |
///
/// Only `Storage` and `Backend` are server faults. A kind is chosen where the
/// failure is still typed (a PostgreSQL SQLSTATE, a MongoDB error code, a
/// `BackendError`), never by matching message text.
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

    /// A batch of resources references more distinct resources than the runner
    /// may read for `resolve()`. The handler layer maps this variant to a
    /// `422 Unprocessable Entity` OperationOutcome.
    #[error("too many references to resolve: {0}")]
    ResolutionLimit(String),

    /// The request cannot be served as given, for a reason the caller can fix.
    /// `status` is the HTTP status the caller should see: `422` for a request
    /// that is well-formed but cannot be processed, any other value answers
    /// `400`.
    #[error("{message}")]
    ClientInput {
        /// The HTTP status for the caller (`422`, otherwise `400`).
        status: u16,
        /// Client-facing description of what is wrong with the request.
        message: String,
    },

    /// A `patient` or `group` reference names a resource that does not exist
    /// (or was deleted). `$sql-run` answers `400` with OperationOutcome code
    /// `not-found`.
    #[error("referenced resources not found: {}", references.join(", "))]
    NotFound {
        /// The references that did not resolve, as the caller wrote them.
        references: Vec<String>,
    },

    /// The backend cannot serve the run right now: no connection could be
    /// obtained, or the server is saturated or shutting down. A retry may
    /// succeed. Answered with `503` and `Retry-After`.
    #[error("backend unavailable: {message}")]
    Unavailable {
        /// Operator-facing detail (never sent to clients).
        message: String,
    },

    /// The run, or one of its statements, exceeded a time limit and was
    /// stopped. Answered with `504` and OperationOutcome code `timeout`.
    #[error("view run timed out: {message}")]
    Timeout {
        /// The time limit that was exceeded, when the run itself set it.
        /// `None` when a limit outside the run stopped it (for example a
        /// connection pool's session `statement_timeout`).
        budget: Option<Duration>,
        /// Operator-facing detail (never sent to clients).
        message: String,
    },

    /// The view run was cancelled (e.g. client disconnected, export job cancelled).
    #[error("view run cancelled")]
    Cancelled,

    /// An error occurred while fetching resources from the storage backend.
    #[error("storage error: {0}")]
    Storage(String),

    /// A backend-level SQL or driver error: a server fault. Build it with
    /// [`SofError::backend`] (no SQLSTATE) or
    /// [`SofError::backend_with_sqlstate`].
    #[error("backend error: {message}")]
    Backend {
        /// The PostgreSQL SQLSTATE of the error, when the backend is PostgreSQL
        /// and the server reported one. Logged by the REST layer.
        sqlstate: Option<String>,
        /// Operator-facing detail (never sent to clients).
        message: String,
    },
}

impl SofError {
    /// A [`SofError::Backend`] without a SQLSTATE.
    pub fn backend(message: impl Into<String>) -> Self {
        Self::Backend {
            sqlstate: None,
            message: message.into(),
        }
    }

    /// A [`SofError::Backend`] that keeps the PostgreSQL SQLSTATE `sqlstate`.
    pub fn backend_with_sqlstate(sqlstate: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Backend {
            sqlstate: Some(sqlstate.into()),
            message: message.into(),
        }
    }

    /// Lowers a classified storage [`BackendError`](crate::error::BackendError)
    /// to its run kind: unavailable, refused or exhausted connections become
    /// [`SofError::Unavailable`], a server-side time limit becomes
    /// [`SofError::Timeout`] (with no run budget), everything else
    /// [`SofError::Backend`].
    pub fn from_backend_error(err: crate::error::BackendError) -> Self {
        use crate::error::BackendError as B;
        match err {
            B::Unavailable { .. } | B::ConnectionFailed { .. } | B::PoolExhausted { .. } => {
                Self::Unavailable {
                    message: err.to_string(),
                }
            }
            B::Timeout { .. } => Self::Timeout {
                budget: None,
                message: err.to_string(),
            },
            other => Self::backend(other.to_string()),
        }
    }

    /// Converts a storage error from a resource read or scan (#2048).
    ///
    /// A [`StorageError::Backend`](crate::error::StorageError::Backend) keeps
    /// its run kind through [`Self::from_backend_error`]: a transient failure
    /// (`Unavailable`, `ConnectionFailed`, `PoolExhausted`; see
    /// [`StorageError::is_unavailable`](crate::error::StorageError::is_unavailable))
    /// becomes [`SofError::Unavailable`], so the one REST mapping answers `503`
    /// with `Retry-After` rather than `500`. This covers what a backend reports
    /// as unavailable besides an exhausted retry: a dispatch failure or
    /// timeout, throttling, and an S3 misconfiguration (access denied, a bucket
    /// that is missing or not accessible). Every other storage error becomes
    /// [`SofError::Storage`] with the error's text.
    pub fn from_storage(err: crate::error::StorageError) -> Self {
        match err {
            crate::error::StorageError::Backend(backend) => Self::from_backend_error(backend),
            other => Self::Storage(other.to_string()),
        }
    }
}

/// Why a run stopped before its natural end.
///
/// A runner takes the kind of a stop it initiated from this reason, never from
/// the error the backend reports for the interrupted work (a PostgreSQL
/// `57014`, a SQLite interrupt).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StopReason {
    /// The consumer dropped the row stream. Nobody reads the stream any more,
    /// so the run ends without an error item.
    Dropped,
    /// The caller fired [`RunContext::cancel`].
    Cancelled,
    /// [`RunContext::deadline`] passed.
    DeadlineExceeded,
}

impl StopReason {
    /// The error item a stop ends the stream with: none for
    /// [`Dropped`](Self::Dropped), [`SofError::Cancelled`] for a cancel, and
    /// [`SofError::Timeout`] carrying `budget` (the time the run had when it
    /// started) for a deadline.
    pub fn into_error(self, budget: Option<Duration>) -> Option<SofError> {
        match self {
            Self::Dropped => None,
            Self::Cancelled => Some(SofError::Cancelled),
            Self::DeadlineExceeded => Some(SofError::Timeout {
                budget,
                message: match budget {
                    Some(budget) => format!("the run's deadline passed (budget {budget:?})"),
                    None => "the run's deadline passed".to_string(),
                },
            }),
        }
    }
}

/// Sender half of a runner's row channel.
pub type RowSender = mpsc::Sender<Result<ViewRow, SofError>>;

/// A pinned, heap-allocated, `Send + 'static` stream of Arrow record batches.
///
/// Returned by [`SofRunner::run_view_columnar`]. Every batch has the same
/// schema; column names and order are the view's output columns.
pub type BatchStream = Pin<Box<dyn Stream<Item = Result<RecordBatch, SofError>> + Send + 'static>>;

/// Sender half of a runner's record-batch channel.
pub type BatchSender = mpsc::Sender<Result<RecordBatch, SofError>>;

/// Watches a spawned row producer and reports its death on the channel.
///
/// Generic over the channel's item: [`RowSender`] for row streams,
/// [`BatchSender`] for columnar ones.
///
/// `producer` is the `JoinHandle` of the task (`tokio::spawn` or
/// `spawn_blocking`) that owns the channel's original `Sender`; `tx` is a
/// clone of that sender kept outside the task. If the task ends with a
/// `JoinError` (panic or cancellation), an
/// `Err(SofError::Backend { message: "row producer failed: …", .. })` is sent through `tx`
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
                .send(Err(SofError::backend(format!("row producer failed: {e}"))))
                .await;
        }
    });
}

/// Abstraction over in-process and in-DB SQL-on-FHIR execution strategies.
///
/// For row order, collection cells and preview limits by backend and view
/// shape, see the [HFS stored-data ordering contract].
///
/// [HFS stored-data ordering contract]: https://heliossoftware.github.io/hfs/ch06-sql-on-fhir.html#hfs-stored-data-ordering
///
/// # Object safety
///
/// `SofRunner` is object-safe and intended for use as `Arc<dyn SofRunner>`. The
/// [`run_view`](Self::run_view) method returns a heap-allocated
/// [`RowStream`] to avoid associated types that would break object safety.
///
/// # Threading
///
/// Implementors must be `Send + Sync` so that the runner can be stored in `AppState`
/// and shared across request tasks.
#[async_trait]
pub trait SofRunner: Send + Sync {
    /// Start a view run and return its schema, capabilities and row stream.
    ///
    /// # Arguments
    ///
    /// * `tenant` — The tenant context; all resource access is scoped to this tenant.
    /// * `request` — The raw ViewDefinition JSON (any FHIR version) and its
    ///   filters (patient, group, since, limit).
    /// * `ctx` — The run's cancel signal, deadline and kind.
    ///
    /// # Returns
    ///
    /// A [`RunOutput`]: the declared [`ViewSchema`] (the view's columns in
    /// ViewDefinition document order, known before the first row), the run's
    /// [`RunCapabilities`], and a [`RowStream`] that yields one flat JSON
    /// object per output row. Callers should honour the `filters.limit` cap or
    /// impose their own.
    ///
    /// Every row is normalized at the runner boundary: it has exactly the keys
    /// of the schema, in schema order, with an explicit `null` for an empty
    /// column, whatever order the runner's compiled query produces its columns
    /// in (a `select` after a `unionAll`, say). A runner permutes its compiled
    /// columns into schema order once per run; the emitted query, and so the
    /// row order, does not depend on it.
    ///
    /// # Dropping the stream
    ///
    /// Dropping the row stream stops the run: the runner stops the work the
    /// backend is doing for it (see [`RunCapabilities::cancels_server_side`])
    /// and gives its connection back. A run that reaches its
    /// [`ViewFilters::limit`] stops the same way. Neither adds an error item.
    ///
    /// A cancel, a deadline and a drop are one stop signal, and the target is
    /// that the backend's work ends within one second of it. A runner that
    /// stops server work bounds its cleanup from the stop: PostgreSQL sends a
    /// cancel request while it still holds the connection, drains the
    /// statement and rolls back, all inside a one-second cleanup envelope, and
    /// a phase that stalls closes the connection instead. The drain timeout
    /// only decides whether the connection is reused or closed; it never
    /// extends the cleanup past the envelope. Three stops are bounded
    /// by something other than the runner, and are documented as exceptions:
    /// a FHIRPath terminology call in flight inside an in-process evaluation
    /// (`FHIRPATH_TERMINOLOGY_TIMEOUT`), a MongoDB deployment that refuses
    /// both the own-operation `killOp` and the disconnect kill (`maxTimeMS`,
    /// the deadline), and a PostgreSQL cancel request that cannot reach the
    /// server (`SET LOCAL statement_timeout`, the deadline; the connection is
    /// still closed within the envelope).
    ///
    /// # Errors
    ///
    /// Returns [`SofError::Uncompilable`] synchronously (before the stream is
    /// polled) when this runner cannot handle the given ViewDefinition. A
    /// failure after the stream was returned is an `Err` item on the stream,
    /// after which the stream ends. Either way the error carries its kind
    /// (see [`SofError`]): for example a backend that is unavailable is
    /// [`SofError::Unavailable`] and a server-side time limit is
    /// [`SofError::Timeout`], never a [`SofError::Backend`] string. A stream
    /// never ends early without an error item, so a partial row set cannot
    /// look complete.
    ///
    /// A stop that is not a drop ends the stream with an error item whose
    /// kind comes from the [`StopReason`]: [`SofError::Cancelled`] after
    /// [`RunContext::cancel`], [`SofError::Timeout`] (with the run's budget)
    /// after the deadline.
    ///
    /// # Deadlines
    ///
    /// [`RunContext::deadline`] is absolute: a runner never renews it per
    /// statement or page. The SQL runners enforce it on the server where they
    /// can (PostgreSQL runs every statement under `SET LOCAL
    /// statement_timeout` from the time left; SQLite checks it from a
    /// progress handler), and on the client for the steps the server cannot
    /// see (connection checkout, delivery to a slow consumer). A run whose
    /// deadline has already passed ends as a timeout without running.
    async fn run_view(
        &self,
        tenant: &TenantContext,
        request: RunRequest,
        ctx: RunContext,
    ) -> Result<RunOutput, SofError>;

    /// Execute a ViewDefinition and return its rows as Arrow record batches,
    /// built straight from database values without the JSON step.
    ///
    /// `Ok(None)` means this runner has no columnar path, or not for this
    /// view; the caller must use [`run_view`](Self::run_view). The default
    /// returns `Ok(None)`, so existing runners and wrappers keep working, and
    /// a wrapper that does not forward this method silently takes the row path.
    ///
    /// Everything [`run_view`](Self::run_view) states about stopping, errors
    /// and deadlines holds here too, with `ctx` as the run's context (#1864):
    /// dropping the batch stream, a cancel or the deadline stops the backend's
    /// work within the same bound; a stop that is not a drop ends the stream
    /// with its typed error item; a failure carries its [`SofError`] kind; and
    /// the stream never ends early without an error item. The columns of every
    /// batch are the view's output columns in ViewDefinition document order
    /// (the order of [`RunOutput::schema`]), whatever order the compiled query
    /// produces them in.
    ///
    /// [`SofError::Uncompilable`] is returned synchronously, as for `run_view`.
    /// `Ok(Some(stream))` that ends without a batch means the view produced
    /// no rows; its schema is not known then.
    async fn run_view_columnar(
        &self,
        _tenant: &TenantContext,
        _request: RunRequest,
        _ctx: RunContext,
    ) -> Result<Option<BatchStream>, SofError> {
        Ok(None)
    }

    /// Returns a human-readable name for this runner (used in logs and diagnostics).
    fn runner_name(&self) -> &'static str;

    /// A runner for one export job, or `None` when this runner has nothing to
    /// share between the subjects of a job.
    ///
    /// An export job runs many views one after another. The job has no
    /// job-wide snapshot: each view, and each page of a paged view, reads
    /// storage as it is at that read, so a resource created or deleted while
    /// the job runs may or may not appear in it.
    ///
    /// - The PostgreSQL runner returns a clone that reads each view in keyset
    ///   pages over the resource id, one statement (in its own `READ ONLY`
    ///   transaction) per page, so no statement sorts or holds the entire
    ///   type; its rows come out in bytewise id order (not
    ///   `(last_updated, id)`).
    /// - The in-process runner (S3) returns a runner whose scans go through a
    ///   per-job cache, so the views of a job share one read of a type, but
    ///   only for a type of 20,000 resources or fewer, keyed by
    ///   `(tenant, type, cutoff)`: views with different `_since` cutoffs never
    ///   share a read. A larger type is read again by each view.
    ///
    /// Every run of the job still takes its own [`RunContext`]: the job's
    /// budget is the runs' deadline, so a shared runner never extends it.
    fn for_export_job(&self) -> Option<Arc<dyn SofRunner>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;
    use tokio_stream::StreamExt;
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
                Err(SofError::Backend { message: m, .. }) => {
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
                Err(SofError::Backend { message: m, .. }) => {
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

    /// A classified storage error keeps its kind as a run error: a transient
    /// condition stays retryable (`Unavailable`), a server-side time limit is
    /// a `Timeout` with no run budget, and only the rest is a `Backend` fault.
    #[test]
    fn backend_errors_lower_to_their_run_kind() {
        use crate::error::BackendError;
        let name = || "postgres".to_string();
        for err in [
            BackendError::Unavailable {
                backend_name: name(),
                message: "shutting down".into(),
            },
            BackendError::ConnectionFailed {
                backend_name: name(),
                message: "refused".into(),
            },
            BackendError::PoolExhausted {
                backend_name: name(),
            },
        ] {
            let lowered = SofError::from_backend_error(err);
            assert!(
                matches!(lowered, SofError::Unavailable { .. }),
                "{lowered:?}"
            );
        }
        let lowered = SofError::from_backend_error(BackendError::Timeout {
            backend_name: name(),
            message: "57014".into(),
        });
        assert!(
            matches!(lowered, SofError::Timeout { budget: None, ref message } if message.contains("57014")),
            "{lowered:?}"
        );
        let lowered = SofError::from_backend_error(BackendError::Internal {
            backend_name: name(),
            message: "boom".into(),
            source: None,
        });
        assert!(
            matches!(lowered, SofError::Backend { sqlstate: None, ref message } if message.contains("boom")),
            "{lowered:?}"
        );
    }

    /// A transient backend failure keeps its unavailability so the handler can
    /// answer 503; every other storage error stays a 500 kind with the same
    /// text as before (#2048, expressed through #1864's typed kinds).
    #[test]
    fn storage_errors_keep_their_unavailability() {
        use crate::error::{BackendError, StorageError, TenantError};

        let transient = [
            StorageError::Backend(BackendError::Unavailable {
                backend_name: "s3".to_string(),
                message: "x".to_string(),
            }),
            StorageError::Backend(BackendError::ConnectionFailed {
                backend_name: "s3".to_string(),
                message: "x".to_string(),
            }),
            StorageError::Backend(BackendError::PoolExhausted {
                backend_name: "s3".to_string(),
            }),
        ];
        for err in transient {
            assert!(err.is_unavailable());
            let text = err.to_string();
            match SofError::from_storage(err) {
                SofError::Unavailable { message } => {
                    assert!(message.contains(&text), "lost the text: {message}")
                }
                other => panic!("expected SofError::Unavailable, got {other:?}"),
            }
        }

        let internal = StorageError::Backend(BackendError::Internal {
            backend_name: "s3".to_string(),
            message: "boom".to_string(),
            source: None,
        });
        let text = internal.to_string();
        match SofError::from_storage(internal) {
            SofError::Backend {
                sqlstate: None,
                message,
            } => assert_eq!(message, text),
            other => panic!("expected SofError::Backend, got {other:?}"),
        }

        let tenant = StorageError::Tenant(TenantError::InvalidTenant {
            tenant_id: crate::tenant::TenantId::new("t"),
        });
        let text = tenant.to_string();
        match SofError::from_storage(tenant) {
            SofError::Storage(m) => assert_eq!(m, text),
            other => panic!("expected SofError::Storage, got {other:?}"),
        }
    }
}
