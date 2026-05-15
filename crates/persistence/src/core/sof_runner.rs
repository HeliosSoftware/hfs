//! SQL-on-FHIR runner abstraction.
//!
//! This module defines the [`SofRunner`] trait, implemented by per-backend
//! **in-DB runners** that compile a [`ViewDefinition`] to SQL and execute it
//! directly inside the storage backend, skipping FHIRPath evaluation entirely.
//! Two implementations exist today: one for SQLite and one for PostgreSQL.
//! Backends advertise the capability via [`BackendCapability::InDbSofRunner`].
//!
//! There is no in-process FHIRPath fallback — if the configured backend does
//! not provide a runner, the `$viewdefinition-run` handler returns
//! `501 Not Implemented`. Inline `resource:` parameters are materialised into
//! a transient in-memory SQLite backend so they reuse the same in-DB pipeline.
//!
//! The handler layer streams the result rows directly into the HTTP response.

use std::pin::Pin;

use async_trait::async_trait;
use futures::Stream;
use serde_json::Value;

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
