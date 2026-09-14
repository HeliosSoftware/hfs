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

use std::pin::Pin;

use async_trait::async_trait;
use futures::Stream;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::warn;

use crate::tenant::TenantContext;

/// Filters that narrow which resources are processed by a view run.
///
/// Per the SQL-on-FHIR v2 spec, `patient` and `group` are `0..*` — supplying
/// multiple values must include resources matching ANY of them (union of the
/// corresponding compartments).
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

/// Watches a spawned row producer and reports its death on the channel.
///
/// `producer` is the `JoinHandle` of the task (`tokio::spawn` or
/// `spawn_blocking`) that owns the channel's original `Sender`; `tx` is a
/// clone of that sender kept outside the task. If the task ends with a
/// `JoinError` (panic or cancellation), an
/// `Err(SofError::Backend("row producer failed: …"))` is sent through `tx`
/// and a `warn!` names the runner; if it ends normally, nothing is sent.
/// Because `tx` stays alive until the outcome is known, the receiver only
/// observes end-of-stream after the producer's fate has been reported —
/// a partial row set can never look like a complete one.
pub fn watch_row_producer(runner: &'static str, tx: RowSender, producer: JoinHandle<()>) {
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
    /// impose their own.
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
}
