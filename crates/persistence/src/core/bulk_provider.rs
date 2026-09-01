//! Provider-side storage for the Bulk Data Submit workspace.
//!
//! When HFS acts as a Bulk Submit **Data Provider** (the `/ui/bulk-import`
//! workspace), it needs a durable record of the submissions it has sent: name,
//! recipient, submitter identity, auth configuration, the manifests submitted,
//! the recipient's poll URL and latest verdict, and a bounded activity log.
//!
//! That record used to ride in the per-user UI settings document, which was the
//! wrong box (#766, #772): invisible to every other operator of the tenant,
//! capped with the settings document, and maintained through merge patches
//! whose delete-then-write shape could destroy a submission outright. This
//! store is the correction — **tenant-scoped**, shared by all of a tenant's
//! operators, written as whole documents under optimistic versioning.
//!
//! The document body is opaque JSON owned by the UI layer (the same
//! `Submission` shape it always serialized); the store persists, versions, and
//! enumerates it. Implemented by every standalone primary backend, mirroring
//! [`SettingsStore`](crate::core::user_settings::SettingsStore).

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::error::StorageResult;
use crate::tenant::TenantContext;

/// One stored provider-side submission.
#[derive(Debug, Clone)]
pub struct StoredProviderSubmission {
    /// The submission's id (unique within the tenant).
    pub id: String,
    /// The opaque submission document.
    pub document: Value,
    /// Monotonic version, bumped on every write; the optimistic-locking token.
    pub version: i64,
    /// Timestamp of the most recent write.
    pub updated_at: DateTime<Utc>,
}

/// Storage abstraction for provider-side Bulk Submit submissions.
#[async_trait]
pub trait BulkProviderStore: Send + Sync {
    /// Every submission of the tenant, unordered.
    async fn list_provider_submissions(
        &self,
        tenant: &TenantContext,
    ) -> StorageResult<Vec<StoredProviderSubmission>>;

    /// One submission by id, or `None`.
    async fn get_provider_submission(
        &self,
        tenant: &TenantContext,
        id: &str,
    ) -> StorageResult<Option<StoredProviderSubmission>>;

    /// Replaces (or creates) a submission's whole document.
    ///
    /// When `if_match_version` is `Some`, the write only succeeds against that
    /// stored version — `Some(0)` asserts the submission does not yet exist —
    /// otherwise a
    /// [`ConcurrencyError::OptimisticLockFailure`](crate::error::ConcurrencyError::OptimisticLockFailure)
    /// is returned. `None` writes unconditionally.
    async fn put_provider_submission(
        &self,
        tenant: &TenantContext,
        id: &str,
        document: Value,
        if_match_version: Option<i64>,
    ) -> StorageResult<StoredProviderSubmission>;

    /// Deletes a submission, returning whether it existed. Idempotent.
    async fn delete_provider_submission(
        &self,
        tenant: &TenantContext,
        id: &str,
    ) -> StorageResult<bool>;
}
