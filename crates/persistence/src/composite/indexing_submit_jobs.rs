//! Bulk-submit job store that indexes into search while a manifest ingests
//! (#1127, selected by `HFS_BULK_SUBMIT_DEFER_INDEXING=false` on a deployment
//! whose search is offloaded to a secondary — #1242).
//!
//! [`IndexingSubmitJobs`] wraps another job store — normally the composite's
//! [`CompositeSubmitJobs`](super::CompositeSubmitJobs), so resource operations
//! and rollbacks still reach the secondaries — and changes these things:
//!
//! - every ingest ([`BulkSubmitProvider::process_entries`] and
//!   [`StreamingBulkSubmitProvider::process_ndjson_stream`]) runs with an
//!   [`IngestIndexSink`] attached as one more batch observer, so each
//!   committed batch is handed to the search index as soon as it is durable;
//! - [`SubmitWorkerStorage::sync_ingested`] no longer re-reads and re-syncs the
//!   whole manifest: it drains the sink (bounded), marks what the sink could
//!   not index as `processing-error`, and reports the rejected types with
//!   [`IngestSyncReport::indexed_during_ingest`] set, so the deferred reindex
//!   rebuilds only those types — or nothing;
//! - [`SubmitWorkerStorage::checkpoint_after_file`] reaches the inner store,
//!   whose primary may keep a WAL to fold back between files;
//! - [`SubmitWorkerStorage::mark_manifest_processing`] starts a fresh sink
//!   run for the manifest, so a claim never inherits what an earlier run that
//!   ended without draining recorded;
//! - [`BulkSubmitProvider::abort_submission`] and
//!   [`BulkSubmitRollbackProvider::rollback_change`] first cancel the
//!   submission's queued sink writes and wait (bounded) for the ones under
//!   way, so no write lands in search after the removal.
//!
//! Everything else delegates to the inner store unchanged.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use helios_fhir::FhirVersion;
use serde_json::Value;
use tokio::io::AsyncBufRead;
use tracing::warn;

use crate::core::bulk_export_worker::{LeaseError, WorkerId};
use crate::core::bulk_submit::{
    BatchCommitObserver, BulkEntryOutcome, BulkEntryResult, BulkProcessingOptions,
    BulkSubmitProvider, BulkSubmitRollbackProvider, EntryCountSummary, EntryResultContinuation,
    EntryResultPage, ManifestPhase, NdjsonEntry, StreamProcessingResult,
    StreamingBulkSubmitProvider, SubmissionChange, SubmissionId, SubmissionManifest,
    SubmissionStatus, SubmissionSummary, UnindexedEntry,
};
use crate::core::bulk_submit_publication::{ManifestPublicationResult, ManifestPublicationStatus};
use crate::core::bulk_submit_worker::{
    BulkSubmitJobStore, IngestSyncReport, ManifestFetchParams, ManifestLease, ManifestWorkerView,
    PollTokenTarget, SubmitClaimStrategy, SubmitFileRecord, SubmitFileRow, SubmitWorkerStorage,
};
use crate::core::storage::ResourceStorage;
use crate::core::{ActivityCell, DailyResourceCount, ResourceCountDelta, SofRunner, TenantRecord};
use crate::error::{BackendError, StorageResult};
use crate::tenant::TenantContext;
use crate::types::StoredResource;

use super::ingest_index_sink::IngestIndexSink;

/// A job store whose ingests index into search batch by batch. See the
/// [module docs](self).
pub struct IndexingSubmitJobs {
    inner: Arc<dyn BulkSubmitJobStore>,
    sink: Arc<IngestIndexSink>,
}

impl IndexingSubmitJobs {
    /// Wraps `inner`, attaching `sink` to every ingest it runs.
    pub fn new(inner: Arc<dyn BulkSubmitJobStore>, sink: Arc<IngestIndexSink>) -> Self {
        Self { inner, sink }
    }

    /// The sink every ingest reports its committed batches to.
    pub fn sink(&self) -> &Arc<IngestIndexSink> {
        &self.sink
    }

    /// The caller's options plus the sink, appended after whatever observers
    /// the caller attached (the dashboard counts keep working).
    fn with_sink(&self, options: &BulkProcessingOptions) -> BulkProcessingOptions {
        options
            .clone()
            .with_batch_observer(Arc::clone(&self.sink) as Arc<dyn BatchCommitObserver>)
    }

    /// Cancels the submission's queued sink writes and waits (bounded) for
    /// the ones under way, before the inner store removes or reverts what
    /// they would index.
    async fn settle_submission(&self, tenant: &TenantContext, id: &SubmissionId) {
        if !self.sink.cancel_pending(tenant, id).await {
            warn!(
                submission = %id,
                "bulk-submit: search index writes for the submission were still under way \
                 when it was aborted or rolled back; a late one may need POST /$reindex"
            );
        }
    }

    /// Drains the sink for the leased manifest and turns what it could not
    /// index into `processing-error` entry results (#1007, #1127).
    async fn drain_ingested(&self, lease: &ManifestLease) -> Result<IngestSyncReport, LeaseError> {
        let drain = self
            .sink
            .drain(&lease.tenant, &lease.submission_id, &lease.manifest_id)
            .await;
        let rejected_types = drain.rejected_types();
        let unindexed = drain.rejected.len() as u64;
        if unindexed > 0 {
            warn!(
                submission = %lease.submission_id,
                manifest = %lease.manifest_id,
                unindexed,
                timed_out = drain.timed_out,
                types = ?rejected_types,
                "bulk-submit: resources ingested but not indexed for search during ingest; \
                 marking them unindexed and leaving their types to the deferred reindex"
            );
            let entries: Vec<UnindexedEntry> = drain
                .rejected
                .into_iter()
                .map(|rejected| {
                    let operation_outcome = serde_json::json!({
                        "resourceType": "OperationOutcome",
                        "issue": [{
                            "severity": "error",
                            "code": "incomplete",
                            "diagnostics": format!(
                                "{}/{} was stored but could not be indexed for search during \
                                 ingest: {}. Run POST /{}/$reindex to repair.",
                                rejected.resource_type,
                                rejected.resource_id,
                                rejected.reason,
                                rejected.resource_type,
                            )
                        }]
                    });
                    UnindexedEntry {
                        resource_type: rejected.resource_type,
                        resource_id: rejected.resource_id,
                        operation_outcome,
                    }
                })
                .collect();
            self.inner
                .mark_entries_unindexed(
                    &lease.tenant,
                    &lease.submission_id,
                    &lease.manifest_id,
                    &entries,
                )
                .await
                .map_err(LeaseError::Storage)?;
        }
        Ok(IngestSyncReport {
            synced: drain.written,
            unindexed,
            drift: Vec::new(),
            rejected_types,
            indexed_during_ingest: true,
        })
    }
}

#[async_trait]
impl ResourceStorage for IndexingSubmitJobs {
    fn backend_name(&self) -> &'static str {
        self.inner.backend_name()
    }

    fn is_cluster_shared(&self) -> bool {
        self.inner.is_cluster_shared()
    }

    async fn readiness_check(&self) -> Result<(), BackendError> {
        self.inner.readiness_check().await
    }

    async fn create(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        fhir_version: FhirVersion,
    ) -> StorageResult<StoredResource> {
        self.inner
            .create(tenant, resource_type, resource, fhir_version)
            .await
    }

    async fn create_many(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resources: Vec<Value>,
        fhir_version: FhirVersion,
    ) -> Vec<StorageResult<StoredResource>> {
        self.inner
            .create_many(tenant, resource_type, resources, fhir_version)
            .await
    }

    async fn create_or_update(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        resource: Value,
        fhir_version: FhirVersion,
    ) -> StorageResult<(StoredResource, bool)> {
        self.inner
            .create_or_update(tenant, resource_type, id, resource, fhir_version)
            .await
    }

    async fn read(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<Option<StoredResource>> {
        self.inner.read(tenant, resource_type, id).await
    }

    async fn update(
        &self,
        tenant: &TenantContext,
        current: &StoredResource,
        resource: Value,
    ) -> StorageResult<StoredResource> {
        self.inner.update(tenant, current, resource).await
    }

    async fn delete(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<()> {
        self.inner.delete(tenant, resource_type, id).await
    }

    async fn exists(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<bool> {
        self.inner.exists(tenant, resource_type, id).await
    }

    async fn read_batch(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        ids: &[&str],
    ) -> StorageResult<Vec<StoredResource>> {
        self.inner.read_batch(tenant, resource_type, ids).await
    }

    async fn count(
        &self,
        tenant: &TenantContext,
        resource_type: Option<&str>,
    ) -> StorageResult<u64> {
        self.inner.count(tenant, resource_type).await
    }

    fn sof_runner(&self) -> Option<Arc<dyn SofRunner>> {
        self.inner.sof_runner()
    }

    async fn count_by_types(
        &self,
        tenant: &TenantContext,
        resource_types: &[&str],
    ) -> StorageResult<Vec<(String, u64)>> {
        self.inner.count_by_types(tenant, resource_types).await
    }

    async fn count_by_day(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        since: DateTime<Utc>,
    ) -> StorageResult<Vec<DailyResourceCount>> {
        self.inner.count_by_day(tenant, resource_type, since).await
    }

    async fn count_deltas_by_bucket(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        since: DateTime<Utc>,
        bucket_seconds: i64,
    ) -> StorageResult<Vec<ResourceCountDelta>> {
        self.inner
            .count_deltas_by_bucket(tenant, resource_type, since, bucket_seconds)
            .await
    }

    async fn count_deltas_by_type_and_bucket(
        &self,
        tenant: &TenantContext,
        resource_types: &[&str],
        since: DateTime<Utc>,
        bucket_seconds: i64,
    ) -> StorageResult<Vec<(String, ResourceCountDelta)>> {
        self.inner
            .count_deltas_by_type_and_bucket(tenant, resource_types, since, bucket_seconds)
            .await
    }

    async fn activity_histogram(
        &self,
        tenant: &TenantContext,
        since: DateTime<Utc>,
    ) -> StorageResult<Vec<ActivityCell>> {
        self.inner.activity_histogram(tenant, since).await
    }

    async fn count_all_types(&self, tenant: &TenantContext) -> StorageResult<Vec<(String, u64)>> {
        self.inner.count_all_types(tenant).await
    }

    fn supports_type_counts(&self) -> bool {
        self.inner.supports_type_counts()
    }

    async fn latest_write_marker(
        &self,
        tenant: &TenantContext,
        recent_since: Option<DateTime<Utc>>,
    ) -> StorageResult<Option<crate::core::WriteMarker>> {
        self.inner.latest_write_marker(tenant, recent_since).await
    }

    async fn count_by_tenant(&self) -> StorageResult<Vec<(String, u64)>> {
        self.inner.count_by_tenant().await
    }

    fn bulk_write_concurrency(&self) -> usize {
        self.inner.bulk_write_concurrency()
    }

    fn supports_tenant_registry(&self) -> bool {
        self.inner.supports_tenant_registry()
    }

    async fn list_tenants(&self) -> StorageResult<Vec<TenantRecord>> {
        self.inner.list_tenants().await
    }

    async fn get_tenant(&self, id: &str) -> StorageResult<Option<TenantRecord>> {
        self.inner.get_tenant(id).await
    }

    fn ensure_canonical_tenant_id(&self, id: &str) -> StorageResult<()> {
        self.inner.ensure_canonical_tenant_id(id)
    }

    async fn register_tenant(
        &self,
        id: &str,
        display_name: Option<&str>,
    ) -> StorageResult<TenantRecord> {
        self.inner.register_tenant(id, display_name).await
    }

    async fn deregister_tenant(&self, id: &str) -> StorageResult<bool> {
        self.inner.deregister_tenant(id).await
    }

    async fn purge_tenant_data(&self, id: &str) -> StorageResult<u64> {
        self.inner.purge_tenant_data(id).await
    }
}

// ---------------------------------------------------------------------------
// Job-store traits — delegated to the inner store, which owns all submit
// state; only the ingest entry points, the sync and the checkpoint differ.
// ---------------------------------------------------------------------------

#[async_trait]
impl BulkSubmitProvider for IndexingSubmitJobs {
    async fn create_submission(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
        metadata: Option<Value>,
    ) -> StorageResult<SubmissionSummary> {
        self.inner.create_submission(tenant, id, metadata).await
    }

    async fn get_submission(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<Option<SubmissionSummary>> {
        self.inner.get_submission(tenant, id).await
    }

    /// Forwarded rather than left to the trait default, which would answer it
    /// with a whole [`Self::get_submission`]. The lease keeper polls this on
    /// every heartbeat (#1138), so falling through would put a manifest-wide
    /// aggregate on the path that keeps the lease alive — the one #1127 exists
    /// to protect. Backends that answer it with a single-row read, as
    /// PostgreSQL does, only do so if the call reaches them.
    async fn get_submission_status(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<Option<SubmissionStatus>> {
        self.inner.get_submission_status(tenant, id).await
    }

    async fn list_submissions(
        &self,
        tenant: &TenantContext,
        submitter: Option<&str>,
        status: Option<SubmissionStatus>,
        limit: u32,
        offset: u32,
    ) -> StorageResult<Vec<SubmissionSummary>> {
        self.inner
            .list_submissions(tenant, submitter, status, limit, offset)
            .await
    }

    async fn complete_submission(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<()> {
        self.inner.complete_submission(tenant, id).await
    }

    async fn abort_submission(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
        reason: &str,
    ) -> StorageResult<u64> {
        self.settle_submission(tenant, id).await;
        self.inner.abort_submission(tenant, id, reason).await
    }

    async fn add_manifest(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_url: Option<&str>,
        replaces_manifest_url: Option<&str>,
    ) -> StorageResult<SubmissionManifest> {
        self.inner
            .add_manifest(tenant, submission_id, manifest_url, replaces_manifest_url)
            .await
    }

    async fn get_manifest(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_id: &str,
    ) -> StorageResult<Option<SubmissionManifest>> {
        self.inner
            .get_manifest(tenant, submission_id, manifest_id)
            .await
    }

    async fn list_manifests(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
    ) -> StorageResult<Vec<SubmissionManifest>> {
        self.inner.list_manifests(tenant, submission_id).await
    }

    async fn process_entries(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_id: &str,
        entries: Vec<NdjsonEntry>,
        options: &BulkProcessingOptions,
    ) -> StorageResult<Vec<BulkEntryResult>> {
        let options = self.with_sink(options);
        self.inner
            .process_entries(tenant, submission_id, manifest_id, entries, &options)
            .await
    }

    async fn get_entry_results_page(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_id: &str,
        outcome_filter: Option<BulkEntryOutcome>,
        limit: u32,
        continuation: Option<&EntryResultContinuation>,
    ) -> StorageResult<EntryResultPage> {
        self.inner
            .get_entry_results_page(
                tenant,
                submission_id,
                manifest_id,
                outcome_filter,
                limit,
                continuation,
            )
            .await
    }

    async fn get_entry_counts(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_id: &str,
    ) -> StorageResult<EntryCountSummary> {
        self.inner
            .get_entry_counts(tenant, submission_id, manifest_id)
            .await
    }

    async fn mark_entries_unindexed(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_id: &str,
        entries: &[UnindexedEntry],
    ) -> StorageResult<u64> {
        self.inner
            .mark_entries_unindexed(tenant, submission_id, manifest_id, entries)
            .await
    }
}

#[async_trait]
impl StreamingBulkSubmitProvider for IndexingSubmitJobs {
    async fn process_ndjson_stream(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_id: &str,
        resource_type: &str,
        reader: Box<dyn AsyncBufRead + Send + Unpin>,
        options: &BulkProcessingOptions,
    ) -> StorageResult<StreamProcessingResult> {
        let options = self.with_sink(options);
        self.inner
            .process_ndjson_stream(
                tenant,
                submission_id,
                manifest_id,
                resource_type,
                reader,
                &options,
            )
            .await
    }
}

#[async_trait]
impl BulkSubmitRollbackProvider for IndexingSubmitJobs {
    async fn record_change(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        change: &SubmissionChange,
    ) -> StorageResult<()> {
        self.inner
            .record_change(tenant, submission_id, change)
            .await
    }

    async fn list_changes(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        limit: u32,
        offset: u32,
    ) -> StorageResult<Vec<SubmissionChange>> {
        self.inner
            .list_changes(tenant, submission_id, limit, offset)
            .await
    }

    async fn rollback_change(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        change: &SubmissionChange,
    ) -> StorageResult<bool> {
        // The inner store mirrors the revert on the secondaries when it is a
        // composite job store; a queued sink write must not undo it.
        self.settle_submission(tenant, submission_id).await;
        self.inner
            .rollback_change(tenant, submission_id, change)
            .await
    }
}

#[async_trait]
impl SubmitClaimStrategy for IndexingSubmitJobs {
    async fn claim_next_manifest(
        &self,
        worker_id: &WorkerId,
        lease_duration: std::time::Duration,
    ) -> StorageResult<Option<ManifestLease>> {
        self.inner
            .claim_next_manifest(worker_id, lease_duration)
            .await
    }

    async fn heartbeat(&self, lease: &ManifestLease) -> Result<DateTime<Utc>, LeaseError> {
        self.inner.heartbeat(lease).await
    }

    async fn release(&self, lease: ManifestLease) -> StorageResult<()> {
        self.inner.release(lease).await
    }
}

#[async_trait]
impl SubmitWorkerStorage for IndexingSubmitJobs {
    async fn get_manifest_for_worker(
        &self,
        lease: &ManifestLease,
    ) -> Result<ManifestWorkerView, LeaseError> {
        self.inner.get_manifest_for_worker(lease).await
    }

    async fn mark_manifest_processing(&self, lease: &ManifestLease) -> Result<(), LeaseError> {
        // Every run of a claimed manifest starts here.
        self.sink
            .start_run(&lease.tenant, &lease.submission_id, &lease.manifest_id);
        self.inner.mark_manifest_processing(lease).await
    }

    async fn add_manifest_progress(
        &self,
        lease: &ManifestLease,
        processed_delta: u64,
        failed_delta: u64,
        lines_delta: u64,
    ) -> Result<(), LeaseError> {
        self.inner
            .add_manifest_progress(lease, processed_delta, failed_delta, lines_delta)
            .await
    }

    async fn update_manifest_bytes(
        &self,
        lease: &ManifestLease,
        bytes_processed: u64,
        bytes_total: u64,
    ) -> Result<(), LeaseError> {
        self.inner
            .update_manifest_bytes(lease, bytes_processed, bytes_total)
            .await
    }

    async fn update_manifest_phase(
        &self,
        lease: &ManifestLease,
        phase: ManifestPhase,
        files_done: u64,
        files_total: u64,
    ) -> Result<(), LeaseError> {
        self.inner
            .update_manifest_phase(lease, phase, files_done, files_total)
            .await
    }

    async fn record_submit_file(
        &self,
        lease: &ManifestLease,
        file: &SubmitFileRecord,
    ) -> Result<(), LeaseError> {
        self.inner.record_submit_file(lease, file).await
    }

    async fn publish_manifest_artifacts(
        &self,
        lease: &ManifestLease,
        files: &[SubmitFileRecord],
        terminal: ManifestPublicationStatus,
    ) -> Result<ManifestPublicationResult, LeaseError> {
        self.inner
            .publish_manifest_artifacts(lease, files, terminal)
            .await
    }

    async fn finish_manifest(&self, lease: &ManifestLease) -> Result<(), LeaseError> {
        self.inner.finish_manifest(lease).await
    }

    async fn fail_manifest(
        &self,
        lease: &ManifestLease,
        error_message: &str,
    ) -> Result<(), LeaseError> {
        self.inner.fail_manifest(lease, error_message).await
    }

    async fn sync_ingested(&self, lease: &ManifestLease) -> Result<IngestSyncReport, LeaseError> {
        self.drain_ingested(lease).await
    }

    async fn checkpoint_after_file(&self) {
        self.inner.checkpoint_after_file().await;
    }

    async fn set_manifest_fetch_params(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
        manifest_id: &str,
        params: ManifestFetchParams<'_>,
    ) -> StorageResult<()> {
        self.inner
            .set_manifest_fetch_params(tenant, id, manifest_id, params)
            .await
    }

    async fn replace_manifest_by_url(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
        manifest_url: &str,
    ) -> StorageResult<Vec<String>> {
        self.inner
            .replace_manifest_by_url(tenant, id, manifest_url)
            .await
    }

    async fn set_submission_kickoff_meta(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
        owner_subject: Option<&str>,
        request_url: &str,
        requires_access_token: bool,
    ) -> StorageResult<()> {
        self.inner
            .set_submission_kickoff_meta(
                tenant,
                id,
                owner_subject,
                request_url,
                requires_access_token,
            )
            .await
    }

    async fn ensure_poll_token(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<String> {
        self.inner.ensure_poll_token(tenant, id).await
    }

    async fn resolve_poll_token(&self, token: &str) -> StorageResult<Option<PollTokenTarget>> {
        self.inner.resolve_poll_token(token).await
    }

    async fn clear_poll_token(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<()> {
        self.inner.clear_poll_token(tenant, id).await
    }

    async fn list_submit_files(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<Vec<SubmitFileRow>> {
        self.inner.list_submit_files(tenant, id).await
    }

    async fn delete_submission_artifacts(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<()> {
        self.inner.delete_submission_artifacts(tenant, id).await
    }

    async fn count_active_submissions(&self, tenant: &TenantContext) -> StorageResult<u64> {
        self.inner.count_active_submissions(tenant).await
    }

    async fn list_expired_submissions(
        &self,
        now: DateTime<Utc>,
        ttl: std::time::Duration,
        limit: u32,
    ) -> StorageResult<Vec<(TenantContext, SubmissionId)>> {
        self.inner.list_expired_submissions(now, ttl, limit).await
    }

    async fn ensure_transaction_time(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<DateTime<Utc>> {
        self.inner.ensure_transaction_time(tenant, id).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use serde_json::json;

    use super::*;
    use crate::backends::sqlite::SqliteBackend;
    use crate::composite::ingest_index_sink::IngestIndexSinkConfig;
    use crate::composite::ingest_index_sink::test_support::SpyTarget;
    use crate::core::bulk_submit::BatchCommitted;
    use crate::search::ReindexTarget;
    use crate::tenant::{TenantId, TenantPermissions};

    fn tenant() -> TenantContext {
        TenantContext::new(TenantId::new("t1"), TenantPermissions::full_access())
    }

    struct Harness {
        sqlite: Arc<SqliteBackend>,
        jobs: IndexingSubmitJobs,
        target: Arc<SpyTarget>,
        sub: SubmissionId,
        lease: ManifestLease,
    }

    async fn harness(target: SpyTarget) -> Harness {
        let sqlite = Arc::new(SqliteBackend::in_memory().unwrap());
        sqlite.init_schema().unwrap();
        let target = Arc::new(target);
        let sink = Arc::new(IngestIndexSink::new(
            sqlite.clone() as Arc<dyn ResourceStorage>,
            vec![target.clone() as Arc<dyn ReindexTarget>],
            IngestIndexSinkConfig::default(),
        ));
        let jobs = IndexingSubmitJobs::new(sqlite.clone() as Arc<dyn BulkSubmitJobStore>, sink);
        let tenant = tenant();
        let sub = SubmissionId::generate("index-during-ingest");
        sqlite.create_submission(&tenant, &sub, None).await.unwrap();
        sqlite
            .add_manifest(&tenant, &sub, Some("http://provider/m.json"), None)
            .await
            .unwrap();
        let lease = sqlite
            .claim_next_manifest(&WorkerId::new("w-idx"), std::time::Duration::from_secs(60))
            .await
            .unwrap()
            .expect("claimable manifest");
        Harness {
            sqlite,
            jobs,
            target,
            sub,
            lease,
        }
    }

    fn patients(ids: &[&str]) -> Vec<NdjsonEntry> {
        ids.iter()
            .enumerate()
            .map(|(i, id)| {
                NdjsonEntry::new(
                    i as u64 + 1,
                    "Patient",
                    json!({"resourceType": "Patient", "id": id}),
                )
            })
            .collect()
    }

    /// `complete_submission` is forwarded to the inner store with the trait's
    /// status-only `()` result (#1194): the stored status becomes Complete,
    /// and a second completion is rejected by the store, not swallowed by the
    /// wrapper.
    #[tokio::test]
    async fn complete_submission_is_forwarded_to_the_inner_store() {
        let h = harness(SpyTarget::default()).await;

        h.jobs.complete_submission(&tenant(), &h.sub).await.unwrap();
        assert_eq!(
            h.sqlite
                .get_submission_status(&tenant(), &h.sub)
                .await
                .unwrap(),
            Some(SubmissionStatus::Complete)
        );
        assert!(
            h.jobs.complete_submission(&tenant(), &h.sub).await.is_err(),
            "completing twice must surface the store's rejection"
        );
    }

    #[tokio::test]
    async fn committed_batches_are_indexed_and_the_sync_only_drains_them() {
        let h = harness(SpyTarget::default()).await;
        h.jobs
            .process_entries(
                &tenant(),
                &h.sub,
                &h.lease.manifest_id,
                patients(&["p1", "p2", "p3"]),
                &BulkProcessingOptions::new().with_batch_size(2),
            )
            .await
            .unwrap();

        let report = h.jobs.sync_ingested(&h.lease).await.unwrap();
        assert_eq!(
            report,
            IngestSyncReport {
                synced: 3,
                unindexed: 0,
                drift: Vec::new(),
                rejected_types: Vec::new(),
                indexed_during_ingest: true,
            }
        );
        let mut ids = h.target.ids();
        ids.sort();
        assert_eq!(ids, vec!["p1", "p2", "p3"]);
        assert!(
            h.target.pages.load(Ordering::SeqCst) >= 1,
            "the sink wrote pages"
        );
    }

    #[tokio::test]
    async fn a_rejected_resource_becomes_processing_error_and_names_its_type() {
        let h = harness(SpyTarget {
            reject: ["p-bad".to_string()].into_iter().collect(),
            ..Default::default()
        })
        .await;
        h.jobs
            .process_entries(
                &tenant(),
                &h.sub,
                &h.lease.manifest_id,
                patients(&["p-ok", "p-bad"]),
                &BulkProcessingOptions::new(),
            )
            .await
            .unwrap();

        let report = h.jobs.sync_ingested(&h.lease).await.unwrap();
        assert_eq!(report.unindexed, 1);
        assert_eq!(report.synced, 1);
        assert_eq!(report.rejected_types, vec!["Patient".to_string()]);
        assert!(report.indexed_during_ingest);

        let page = h
            .sqlite
            .get_entry_results_page(&tenant(), &h.sub, &h.lease.manifest_id, None, 10, None)
            .await
            .unwrap();
        let results: Vec<_> = page.entries.into_iter().map(|e| e.result).collect();
        let bad = results
            .iter()
            .find(|r| r.resource_id.as_deref() == Some("p-bad"))
            .expect("rejected entry present");
        assert_eq!(bad.outcome, BulkEntryOutcome::ProcessingError);
        let diagnostics = bad.operation_outcome.as_ref().unwrap()["issue"][0]["diagnostics"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(diagnostics.contains("Patient/p-bad"), "{diagnostics}");
        assert!(diagnostics.contains("$reindex"), "{diagnostics}");
        let ok = results
            .iter()
            .find(|r| r.resource_id.as_deref() == Some("p-ok"))
            .expect("accepted entry present");
        assert_eq!(ok.outcome, BulkEntryOutcome::Success);
    }

    #[tokio::test]
    async fn the_stream_path_indexes_too_and_keeps_the_callers_observers() {
        struct Counting(AtomicUsize);
        #[async_trait]
        impl BatchCommitObserver for Counting {
            async fn batch_committed(&self, batch: &BatchCommitted<'_>) {
                self.0.fetch_add(batch.results.len(), Ordering::SeqCst);
            }
        }

        let h = harness(SpyTarget::default()).await;
        let counting = Arc::new(Counting(AtomicUsize::new(0)));
        let ndjson = (1..=5)
            .map(|i| format!(r#"{{"resourceType":"Patient","id":"s{i}"}}"#))
            .collect::<Vec<_>>()
            .join("\n");
        let options = BulkProcessingOptions::new()
            .with_batch_size(2)
            .with_batch_observer(counting.clone() as Arc<dyn BatchCommitObserver>);
        h.jobs
            .process_ndjson_stream(
                &tenant(),
                &h.sub,
                &h.lease.manifest_id,
                "Patient",
                Box::new(std::io::Cursor::new(ndjson.into_bytes())),
                &options,
            )
            .await
            .unwrap();
        assert_eq!(
            options.batch_observers.len(),
            1,
            "the caller's options are untouched"
        );
        assert_eq!(counting.0.load(Ordering::SeqCst), 5);

        let report = h.jobs.sync_ingested(&h.lease).await.unwrap();
        assert_eq!(report.synced, 5);
        assert_eq!(h.target.ids().len(), 5);
    }

    #[tokio::test]
    async fn a_new_run_of_the_manifest_does_not_inherit_an_earlier_runs_rejections() {
        let h = harness(SpyTarget {
            reject: ["p-bad".to_string()].into_iter().collect(),
            ..Default::default()
        })
        .await;
        // A first run ingests a resource the index rejects, then ends without
        // draining (lease lost, abort, error).
        h.jobs.mark_manifest_processing(&h.lease).await.unwrap();
        h.jobs
            .process_entries(
                &tenant(),
                &h.sub,
                &h.lease.manifest_id,
                patients(&["p-bad"]),
                &BulkProcessingOptions::new(),
            )
            .await
            .unwrap();

        h.jobs.mark_manifest_processing(&h.lease).await.unwrap();
        h.jobs
            .process_entries(
                &tenant(),
                &h.sub,
                &h.lease.manifest_id,
                patients(&["p-ok"]),
                &BulkProcessingOptions::new(),
            )
            .await
            .unwrap();
        let report = h.jobs.sync_ingested(&h.lease).await.unwrap();
        assert_eq!(report.unindexed, 0, "{report:?}");
        assert!(report.rejected_types.is_empty(), "{report:?}");
        assert_eq!(report.synced, 1);
    }

    #[tokio::test]
    async fn a_manifest_that_ingested_nothing_drains_to_an_empty_report() {
        let h = harness(SpyTarget::default()).await;
        let report = h.jobs.sync_ingested(&h.lease).await.unwrap();
        assert_eq!(
            report,
            IngestSyncReport {
                indexed_during_ingest: true,
                ..Default::default()
            }
        );
    }
}
