//! Reindex job controller surface for the `$reindex` REST handler.
//!
//! [`ReindexController`] is a small dyn-safe handle over a
//! [`helios_persistence::search::reindex::ReindexOperation`] (or any equivalent
//! orchestrator). The handler interacts with the controller, not directly with
//! the typed operation, so AppState can carry the optional dependency as an
//! `Arc<dyn ReindexController>` without leaking the `ReindexableStorage` type
//! parameter into every generic bound.

use async_trait::async_trait;
use helios_persistence::search::ReindexError;
use helios_persistence::search::reindex::{
    ReindexOperation, ReindexProgress, ReindexRequest, ReindexableStorage,
};
use helios_persistence::tenant::TenantContext;

/// Dyn-safe handle over a reindex orchestrator.
#[async_trait]
pub trait ReindexController: Send + Sync + 'static {
    /// Starts a reindex job. Returns the job ID immediately; the work runs in
    /// the background.
    async fn start(
        &self,
        tenant: TenantContext,
        request: ReindexRequest,
    ) -> Result<String, ReindexError>;

    /// Returns the current progress for a job, if it exists.
    async fn progress(&self, job_id: &str) -> Option<ReindexProgress>;

    /// Cancels a running job (idempotent: completed jobs return `Ok`).
    async fn cancel(&self, job_id: &str) -> Result<(), ReindexError>;
}

/// Blanket implementation for the persistence-layer [`ReindexOperation`].
#[async_trait]
impl<S> ReindexController for ReindexOperation<S>
where
    S: ReindexableStorage + 'static,
{
    async fn start(
        &self,
        tenant: TenantContext,
        request: ReindexRequest,
    ) -> Result<String, ReindexError> {
        ReindexOperation::start(self, tenant, request).await
    }

    async fn progress(&self, job_id: &str) -> Option<ReindexProgress> {
        ReindexOperation::get_progress(self, job_id).await
    }

    async fn cancel(&self, job_id: &str) -> Result<(), ReindexError> {
        ReindexOperation::cancel(self, job_id).await
    }
}
