//! Bulk-submit job store for composite deployments.
//!
//! `$bulk-submit` ingestion runs on the *primary* backend's engine — that is
//! where the submission, manifest, lease, and receipt state live, and where
//! the primary deliberately skips its local search indexing when search is
//! offloaded to a secondary (Elasticsearch). Nothing on that path ever told
//! the secondary, so bulk-loaded resources were readable but invisible to
//! every search (#882) — precisely the "bulk load, then search" workload the
//! composite exists for.
//!
//! [`CompositeSubmitJobs`] wraps the primary's job store and closes the gap
//! at the manifest boundary: when a manifest reaches a terminal state
//! (`finish_manifest` / `fail_manifest`), every successfully ingested entry
//! is read back from the primary and pushed through the composite's normal
//! secondary-sync machinery — the same [`SyncEvent`] path an interactive
//! create takes, honoring the configured sync mode. `rollback_change` gets
//! the mirror-image treatment so an aborted submission's reverts reach the
//! secondary too.
//!
//! Syncing at manifest completion (rather than per entry or per file) keeps
//! the ingest engine untouched and the cost linear: one read + one sync
//! event per distinct ingested resource. The trade-off is that a manifest's
//! resources become searchable when the manifest finishes, not while it
//! streams — the status endpoint's percentage is the progress signal during
//! ingestion.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use helios_fhir::FhirVersion;
use serde_json::Value;
use tokio::io::AsyncBufRead;
use tracing::warn;

use crate::core::bulk_export_worker::{LeaseError, WorkerId};
use crate::core::bulk_submit::{
    BulkEntryOutcome, BulkEntryResult, BulkProcessingOptions, BulkSubmitProvider,
    BulkSubmitRollbackProvider, ChangeType, EntryCountSummary, NdjsonEntry, StreamProcessingResult,
    StreamingBulkSubmitProvider, SubmissionChange, SubmissionId, SubmissionManifest,
    SubmissionStatus, SubmissionSummary,
};
use crate::core::bulk_submit_worker::{
    BulkSubmitJobStore, ManifestFetchParams, ManifestLease, ManifestWorkerView, PollTokenTarget,
    SubmitClaimStrategy, SubmitFileRecord, SubmitFileRow, SubmitWorkerStorage,
};
use crate::core::storage::ResourceStorage;
use crate::core::{ActivityCell, DailyResourceCount, ResourceCountDelta, SofRunner, TenantRecord};
use crate::error::{BackendError, StorageResult};
use crate::tenant::TenantContext;
use crate::types::StoredResource;

use super::storage::CompositeStorage;
use super::sync::SyncEvent;

/// One batch of ingested resources bound for the secondaries: the resource
/// type and FHIR version they share, and their `(id, content)` pairs.
type SyncGroup = ((String, FhirVersion), Vec<(String, Value)>);

/// The composite's `$bulk-submit` job store: the primary's engine for all
/// state and ingestion, plus secondary-index sync at manifest boundaries.
pub struct CompositeSubmitJobs {
    primary: Arc<dyn BulkSubmitJobStore>,
    composite: Arc<CompositeStorage>,
}

impl CompositeSubmitJobs {
    /// Wraps the primary's job store with the composite's secondary sync.
    pub fn new(primary: Arc<dyn BulkSubmitJobStore>, composite: Arc<CompositeStorage>) -> Self {
        Self { primary, composite }
    }

    /// Pushes every successfully ingested entry of the leased manifest
    /// through the composite's secondary sync.
    ///
    /// Best-effort by design, mirroring the interactive write path: the
    /// primary commit is the source of truth, a secondary that misses an
    /// event is repaired by `$reindex`, and a terminal-state transition must
    /// not be blocked by a search-index hiccup. Distinct resources are
    /// synced once even when a manifest touched them on several lines.
    async fn sync_ingested(&self, lease: &ManifestLease) {
        let mut seen: std::collections::HashSet<(String, String)> =
            std::collections::HashSet::new();
        let limit = 1000u32;
        let mut offset = 0u32;
        loop {
            let batch = match self
                .primary
                .get_entry_results(
                    &lease.tenant,
                    &lease.submission_id,
                    &lease.manifest_id,
                    Some(BulkEntryOutcome::Success),
                    limit,
                    offset,
                )
                .await
            {
                Ok(b) => b,
                Err(e) => {
                    warn!(
                        submission = %lease.submission_id,
                        manifest = %lease.manifest_id,
                        error = %e,
                        "secondary sync: failed to page entry results; \
                         remaining entries await $reindex"
                    );
                    return;
                }
            };
            let n = batch.len() as u32;
            // One batch per (type, FHIR version) per page, so the secondary
            // takes a page of ingested resources as one write rather than one
            // synchronous event each — under Elasticsearch `refresh=wait_for`
            // that is one refresh wait per page instead of per resource.
            let mut by_type: Vec<SyncGroup> = Vec::new();
            for entry in batch {
                let Some(resource_id) = entry.resource_id else {
                    continue;
                };
                if !seen.insert((entry.resource_type.clone(), resource_id.clone())) {
                    continue;
                }
                let Some(stored) = self
                    .read_ingested(lease, &entry.resource_type, &resource_id)
                    .await
                else {
                    continue;
                };
                let key = (entry.resource_type.clone(), stored.fhir_version());
                let group = match by_type.iter_mut().find(|(k, _)| *k == key) {
                    Some(group) => group,
                    None => {
                        by_type.push((key, Vec::new()));
                        by_type.last_mut().expect("just pushed")
                    }
                };
                group.1.push((resource_id, stored.content().clone()));
            }
            for ((resource_type, fhir_version), resources) in by_type {
                self.composite
                    .sync_creates_to_secondaries(
                        &lease.tenant,
                        &resource_type,
                        fhir_version,
                        resources,
                    )
                    .await;
            }
            if n < limit {
                return;
            }
            offset += limit;
        }
    }

    /// Reads one ingested resource back from the primary for syncing. A
    /// resource deleted (or rolled back) since ingestion is reflected as a
    /// delete on the secondaries instead, and `None` is returned.
    async fn read_ingested(
        &self,
        lease: &ManifestLease,
        resource_type: &str,
        resource_id: &str,
    ) -> Option<StoredResource> {
        match self
            .primary
            .read(&lease.tenant, resource_type, resource_id)
            .await
        {
            Ok(Some(stored)) => Some(stored),
            Ok(None) | Err(_) => {
                let _ = self
                    .composite
                    .sync_to_secondaries(SyncEvent::Delete {
                        resource_type: resource_type.to_string(),
                        resource_id: resource_id.to_string(),
                        tenant_id: lease.tenant.tenant_id().clone(),
                    })
                    .await;
                None
            }
        }
    }
}

// ---------------------------------------------------------------------------
// ResourceStorage — delegated to the composite, so a direct resource
// operation through this store syncs secondaries exactly like the app's.
// ---------------------------------------------------------------------------

#[async_trait]
impl ResourceStorage for CompositeSubmitJobs {
    fn backend_name(&self) -> &'static str {
        self.composite.backend_name()
    }

    fn is_cluster_shared(&self) -> bool {
        self.composite.is_cluster_shared()
    }

    async fn readiness_check(&self) -> Result<(), BackendError> {
        self.composite.readiness_check().await
    }

    async fn create(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        fhir_version: FhirVersion,
    ) -> StorageResult<StoredResource> {
        self.composite
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
        self.composite
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
        self.composite
            .create_or_update(tenant, resource_type, id, resource, fhir_version)
            .await
    }

    async fn read(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<Option<StoredResource>> {
        self.composite.read(tenant, resource_type, id).await
    }

    async fn update(
        &self,
        tenant: &TenantContext,
        current: &StoredResource,
        resource: Value,
    ) -> StorageResult<StoredResource> {
        self.composite.update(tenant, current, resource).await
    }

    async fn delete(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<()> {
        self.composite.delete(tenant, resource_type, id).await
    }

    async fn exists(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<bool> {
        self.composite.exists(tenant, resource_type, id).await
    }

    async fn read_batch(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        ids: &[&str],
    ) -> StorageResult<Vec<StoredResource>> {
        self.composite.read_batch(tenant, resource_type, ids).await
    }

    async fn count(
        &self,
        tenant: &TenantContext,
        resource_type: Option<&str>,
    ) -> StorageResult<u64> {
        self.composite.count(tenant, resource_type).await
    }

    fn sof_runner(&self) -> Option<Arc<dyn SofRunner>> {
        self.composite.sof_runner()
    }

    async fn count_by_types(
        &self,
        tenant: &TenantContext,
        resource_types: &[&str],
    ) -> StorageResult<Vec<(String, u64)>> {
        self.composite.count_by_types(tenant, resource_types).await
    }

    async fn count_by_day(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        since: DateTime<Utc>,
    ) -> StorageResult<Vec<DailyResourceCount>> {
        self.composite
            .count_by_day(tenant, resource_type, since)
            .await
    }

    async fn count_deltas_by_bucket(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        since: DateTime<Utc>,
        bucket_seconds: i64,
    ) -> StorageResult<Vec<ResourceCountDelta>> {
        self.composite
            .count_deltas_by_bucket(tenant, resource_type, since, bucket_seconds)
            .await
    }

    async fn activity_histogram(
        &self,
        tenant: &TenantContext,
        since: DateTime<Utc>,
    ) -> StorageResult<Vec<ActivityCell>> {
        self.composite.activity_histogram(tenant, since).await
    }

    async fn count_all_types(&self, tenant: &TenantContext) -> StorageResult<Vec<(String, u64)>> {
        self.composite.count_all_types(tenant).await
    }

    async fn count_by_tenant(&self) -> StorageResult<Vec<(String, u64)>> {
        self.composite.count_by_tenant().await
    }

    fn bulk_write_concurrency(&self) -> usize {
        self.composite.bulk_write_concurrency()
    }

    fn supports_tenant_registry(&self) -> bool {
        self.composite.supports_tenant_registry()
    }

    async fn list_tenants(&self) -> StorageResult<Vec<TenantRecord>> {
        self.composite.list_tenants().await
    }

    async fn get_tenant(&self, id: &str) -> StorageResult<Option<TenantRecord>> {
        self.composite.get_tenant(id).await
    }

    fn ensure_canonical_tenant_id(&self, id: &str) -> StorageResult<()> {
        self.composite.ensure_canonical_tenant_id(id)
    }

    async fn register_tenant(
        &self,
        id: &str,
        display_name: Option<&str>,
    ) -> StorageResult<TenantRecord> {
        self.composite.register_tenant(id, display_name).await
    }

    async fn deregister_tenant(&self, id: &str) -> StorageResult<bool> {
        self.composite.deregister_tenant(id).await
    }

    async fn purge_tenant_data(&self, id: &str) -> StorageResult<u64> {
        self.composite.purge_tenant_data(id).await
    }
}

// ---------------------------------------------------------------------------
// Job-store traits — delegated to the primary, which owns all submit state.
// ---------------------------------------------------------------------------

#[async_trait]
impl BulkSubmitProvider for CompositeSubmitJobs {
    async fn create_submission(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
        metadata: Option<Value>,
    ) -> StorageResult<SubmissionSummary> {
        self.primary.create_submission(tenant, id, metadata).await
    }

    async fn get_submission(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<Option<SubmissionSummary>> {
        self.primary.get_submission(tenant, id).await
    }

    async fn list_submissions(
        &self,
        tenant: &TenantContext,
        submitter: Option<&str>,
        status: Option<SubmissionStatus>,
        limit: u32,
        offset: u32,
    ) -> StorageResult<Vec<SubmissionSummary>> {
        self.primary
            .list_submissions(tenant, submitter, status, limit, offset)
            .await
    }

    async fn complete_submission(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<SubmissionSummary> {
        self.primary.complete_submission(tenant, id).await
    }

    async fn abort_submission(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
        reason: &str,
    ) -> StorageResult<u64> {
        self.primary.abort_submission(tenant, id, reason).await
    }

    async fn add_manifest(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_url: Option<&str>,
        replaces_manifest_url: Option<&str>,
    ) -> StorageResult<SubmissionManifest> {
        self.primary
            .add_manifest(tenant, submission_id, manifest_url, replaces_manifest_url)
            .await
    }

    async fn get_manifest(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_id: &str,
    ) -> StorageResult<Option<SubmissionManifest>> {
        self.primary
            .get_manifest(tenant, submission_id, manifest_id)
            .await
    }

    async fn list_manifests(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
    ) -> StorageResult<Vec<SubmissionManifest>> {
        self.primary.list_manifests(tenant, submission_id).await
    }

    async fn process_entries(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_id: &str,
        entries: Vec<NdjsonEntry>,
        options: &BulkProcessingOptions,
    ) -> StorageResult<Vec<BulkEntryResult>> {
        self.primary
            .process_entries(tenant, submission_id, manifest_id, entries, options)
            .await
    }

    async fn get_entry_results(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_id: &str,
        outcome_filter: Option<BulkEntryOutcome>,
        limit: u32,
        offset: u32,
    ) -> StorageResult<Vec<BulkEntryResult>> {
        self.primary
            .get_entry_results(
                tenant,
                submission_id,
                manifest_id,
                outcome_filter,
                limit,
                offset,
            )
            .await
    }

    async fn get_entry_counts(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_id: &str,
    ) -> StorageResult<EntryCountSummary> {
        self.primary
            .get_entry_counts(tenant, submission_id, manifest_id)
            .await
    }
}

#[async_trait]
impl StreamingBulkSubmitProvider for CompositeSubmitJobs {
    async fn process_ndjson_stream(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_id: &str,
        resource_type: &str,
        reader: Box<dyn AsyncBufRead + Send + Unpin>,
        options: &BulkProcessingOptions,
    ) -> StorageResult<StreamProcessingResult> {
        self.primary
            .process_ndjson_stream(
                tenant,
                submission_id,
                manifest_id,
                resource_type,
                reader,
                options,
            )
            .await
    }
}

#[async_trait]
impl BulkSubmitRollbackProvider for CompositeSubmitJobs {
    async fn record_change(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        change: &SubmissionChange,
    ) -> StorageResult<()> {
        self.primary
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
        self.primary
            .list_changes(tenant, submission_id, limit, offset)
            .await
    }

    async fn rollback_change(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        change: &SubmissionChange,
    ) -> StorageResult<bool> {
        let rolled_back = self
            .primary
            .rollback_change(tenant, submission_id, change)
            .await?;
        if rolled_back {
            // Mirror the primary's revert on the secondaries: a rolled-back
            // create is a delete; a rolled-back update restores the previous
            // content, which the primary now holds.
            let event = match change.change_type {
                ChangeType::Create => SyncEvent::Delete {
                    resource_type: change.resource_type.clone(),
                    resource_id: change.resource_id.clone(),
                    tenant_id: tenant.tenant_id().clone(),
                },
                _ => match self
                    .primary
                    .read(tenant, &change.resource_type, &change.resource_id)
                    .await
                {
                    Ok(Some(stored)) => SyncEvent::Create {
                        resource_type: change.resource_type.clone(),
                        resource_id: change.resource_id.clone(),
                        content: stored.content().clone(),
                        tenant_id: tenant.tenant_id().clone(),
                        fhir_version: stored.fhir_version(),
                    },
                    _ => SyncEvent::Delete {
                        resource_type: change.resource_type.clone(),
                        resource_id: change.resource_id.clone(),
                        tenant_id: tenant.tenant_id().clone(),
                    },
                },
            };
            if let Err(e) = self.composite.sync_to_secondaries(event).await {
                warn!(
                    resource_type = %change.resource_type,
                    resource_id = %change.resource_id,
                    error = %e,
                    "secondary sync of a rollback failed; repair via $reindex"
                );
            }
        }
        Ok(rolled_back)
    }
}

#[async_trait]
impl SubmitClaimStrategy for CompositeSubmitJobs {
    async fn claim_next_manifest(
        &self,
        worker_id: &WorkerId,
        lease_duration: std::time::Duration,
    ) -> StorageResult<Option<ManifestLease>> {
        self.primary
            .claim_next_manifest(worker_id, lease_duration)
            .await
    }

    async fn heartbeat(&self, lease: &ManifestLease) -> Result<DateTime<Utc>, LeaseError> {
        self.primary.heartbeat(lease).await
    }

    async fn release(&self, lease: ManifestLease) -> StorageResult<()> {
        self.primary.release(lease).await
    }
}

#[async_trait]
impl SubmitWorkerStorage for CompositeSubmitJobs {
    async fn get_manifest_for_worker(
        &self,
        lease: &ManifestLease,
    ) -> Result<ManifestWorkerView, LeaseError> {
        self.primary.get_manifest_for_worker(lease).await
    }

    async fn mark_manifest_processing(&self, lease: &ManifestLease) -> Result<(), LeaseError> {
        self.primary.mark_manifest_processing(lease).await
    }

    async fn update_manifest_progress(
        &self,
        lease: &ManifestLease,
        processed_entries: u64,
        failed_entries: u64,
        last_processed_line: u64,
    ) -> Result<(), LeaseError> {
        self.primary
            .update_manifest_progress(
                lease,
                processed_entries,
                failed_entries,
                last_processed_line,
            )
            .await
    }

    async fn update_manifest_bytes(
        &self,
        lease: &ManifestLease,
        bytes_processed: u64,
        bytes_total: u64,
    ) -> Result<(), LeaseError> {
        self.primary
            .update_manifest_bytes(lease, bytes_processed, bytes_total)
            .await
    }

    async fn record_submit_file(
        &self,
        lease: &ManifestLease,
        file: &SubmitFileRecord,
    ) -> Result<(), LeaseError> {
        self.primary.record_submit_file(lease, file).await
    }

    async fn finish_manifest(&self, lease: &ManifestLease) -> Result<(), LeaseError> {
        // Sync before finishing: once the manifest is terminal the lease is
        // gone, and syncing under the live lease keeps a reclaim from racing
        // a half-finished sweep with a second ingestion of the same files.
        self.sync_ingested(lease).await;
        self.primary.finish_manifest(lease).await
    }

    async fn fail_manifest(
        &self,
        lease: &ManifestLease,
        error_message: &str,
    ) -> Result<(), LeaseError> {
        // A failed manifest still committed its successful entries on the
        // primary — search must agree with what reads will return.
        self.sync_ingested(lease).await;
        self.primary.fail_manifest(lease, error_message).await
    }

    async fn set_manifest_fetch_params(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
        manifest_id: &str,
        params: ManifestFetchParams<'_>,
    ) -> StorageResult<()> {
        self.primary
            .set_manifest_fetch_params(tenant, id, manifest_id, params)
            .await
    }

    async fn replace_manifest_by_url(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
        manifest_url: &str,
    ) -> StorageResult<Vec<String>> {
        self.primary
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
        self.primary
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
        self.primary.ensure_poll_token(tenant, id).await
    }

    async fn resolve_poll_token(&self, token: &str) -> StorageResult<Option<PollTokenTarget>> {
        self.primary.resolve_poll_token(token).await
    }

    async fn clear_poll_token(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<()> {
        self.primary.clear_poll_token(tenant, id).await
    }

    async fn list_submit_files(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<Vec<SubmitFileRow>> {
        self.primary.list_submit_files(tenant, id).await
    }

    async fn delete_submission_artifacts(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<()> {
        self.primary.delete_submission_artifacts(tenant, id).await
    }

    async fn count_active_submissions(&self, tenant: &TenantContext) -> StorageResult<u64> {
        self.primary.count_active_submissions(tenant).await
    }

    async fn list_expired_submissions(
        &self,
        now: DateTime<Utc>,
        ttl: std::time::Duration,
        limit: u32,
    ) -> StorageResult<Vec<(TenantContext, SubmissionId)>> {
        self.primary.list_expired_submissions(now, ttl, limit).await
    }

    async fn ensure_transaction_time(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<DateTime<Utc>> {
        self.primary.ensure_transaction_time(tenant, id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::sqlite::SqliteBackend;
    use crate::composite::config::CompositeConfig;
    use crate::composite::storage::DynStorage;
    use crate::core::BackendKind;
    use crate::tenant::{TenantId, TenantPermissions};
    use parking_lot::Mutex;
    use serde_json::json;
    use std::collections::HashMap;

    /// Records every write the composite syncs into the "secondary".
    struct SpySecondary {
        events: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl ResourceStorage for SpySecondary {
        fn backend_name(&self) -> &'static str {
            "spy-secondary"
        }

        async fn create(
            &self,
            _tenant: &TenantContext,
            resource_type: &str,
            resource: Value,
            fhir_version: FhirVersion,
        ) -> StorageResult<StoredResource> {
            let id = resource
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("?")
                .to_string();
            self.events
                .lock()
                .push(format!("create {resource_type}/{id}"));
            Ok(StoredResource::new(
                resource_type,
                id,
                TenantId::new("t1"),
                resource,
                fhir_version,
            ))
        }

        async fn create_or_update(
            &self,
            _tenant: &TenantContext,
            resource_type: &str,
            id: &str,
            resource: Value,
            fhir_version: FhirVersion,
        ) -> StorageResult<(StoredResource, bool)> {
            self.events
                .lock()
                .push(format!("upsert {resource_type}/{id}"));
            Ok((
                StoredResource::new(
                    resource_type,
                    id,
                    TenantId::new("t1"),
                    resource,
                    fhir_version,
                ),
                true,
            ))
        }

        async fn read(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _id: &str,
        ) -> StorageResult<Option<StoredResource>> {
            Ok(None)
        }

        async fn update(
            &self,
            _tenant: &TenantContext,
            current: &StoredResource,
            resource: Value,
        ) -> StorageResult<StoredResource> {
            self.events.lock().push(format!(
                "update {}/{}",
                current.resource_type(),
                current.id()
            ));
            Ok(StoredResource::new(
                current.resource_type(),
                current.id(),
                TenantId::new("t1"),
                resource,
                current.fhir_version(),
            ))
        }

        async fn delete(
            &self,
            _tenant: &TenantContext,
            resource_type: &str,
            id: &str,
        ) -> StorageResult<()> {
            self.events
                .lock()
                .push(format!("delete {resource_type}/{id}"));
            Ok(())
        }

        async fn count(
            &self,
            _tenant: &TenantContext,
            _resource_type: Option<&str>,
        ) -> StorageResult<u64> {
            Ok(0)
        }
    }

    fn tenant() -> TenantContext {
        TenantContext::new(TenantId::new("t1"), TenantPermissions::full_access())
    }

    fn harness() -> (
        Arc<SqliteBackend>,
        CompositeSubmitJobs,
        Arc<Mutex<Vec<String>>>,
    ) {
        let sqlite = Arc::new(SqliteBackend::in_memory().unwrap());
        sqlite.init_schema().unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let config = CompositeConfig::builder()
            .primary("sqlite", BackendKind::Sqlite)
            .search_backend("es", BackendKind::Elasticsearch)
            // Synchronous: no background worker runs in a unit test, and the
            // assertions need events applied before the call returns.
            .sync_mode(crate::composite::config::SyncMode::Synchronous)
            .build()
            .unwrap();
        let mut backends = HashMap::new();
        backends.insert("sqlite".to_string(), sqlite.clone() as DynStorage);
        backends.insert(
            "es".to_string(),
            Arc::new(SpySecondary {
                events: events.clone(),
            }) as DynStorage,
        );
        // No sync worker started: events apply synchronously, which is what
        // the assertions need.
        let composite = Arc::new(CompositeStorage::new(config, backends).unwrap());
        let jobs =
            CompositeSubmitJobs::new(sqlite.clone() as Arc<dyn BulkSubmitJobStore>, composite);
        (sqlite, jobs, events)
    }

    /// #882: a finished manifest pushes every ingested resource into the
    /// secondary — through the raw primary engine, nothing ever did.
    #[tokio::test]
    async fn finished_manifest_syncs_ingested_resources_to_the_secondary() {
        let (sqlite, jobs, events) = harness();
        let tenant = tenant();
        let sub = SubmissionId::generate("sync-test");
        sqlite.create_submission(&tenant, &sub, None).await.unwrap();
        sqlite
            .add_manifest(&tenant, &sub, Some("http://provider/m.json"), None)
            .await
            .unwrap();
        let lease = sqlite
            .claim_next_manifest(&WorkerId::new("w-sync"), std::time::Duration::from_secs(60))
            .await
            .unwrap()
            .expect("claimable manifest");

        // Ingest through the primary engine, exactly as the worker does.
        sqlite
            .process_entries(
                &tenant,
                &sub,
                &lease.manifest_id,
                vec![
                    NdjsonEntry::new(
                        1,
                        "Patient",
                        json!({"resourceType": "Patient", "id": "p-sync-1", "name": [{"family": "Synced"}]}),
                    ),
                    NdjsonEntry::new(
                        2,
                        "Patient",
                        json!({"resourceType": "Patient", "id": "p-sync-2"}),
                    ),
                ],
                &BulkProcessingOptions::new(),
            )
            .await
            .unwrap();

        assert!(
            events.lock().is_empty(),
            "nothing syncs while the manifest is still streaming"
        );

        jobs.finish_manifest(&lease).await.unwrap();

        let seen = events.lock().clone();
        assert!(
            seen.contains(&"create Patient/p-sync-1".to_string()),
            "ingested resource must reach the secondary, got {seen:?}"
        );
        assert!(
            seen.contains(&"create Patient/p-sync-2".to_string()),
            "every distinct ingested resource syncs, got {seen:?}"
        );

        // And the manifest actually finished on the primary.
        let manifests = sqlite.list_manifests(&tenant, &sub).await.unwrap();
        assert!(manifests[0].status.is_terminal());
    }

    /// A rolled-back create is deleted from the secondary too.
    #[tokio::test]
    async fn rollback_of_a_create_deletes_from_the_secondary() {
        let (sqlite, jobs, events) = harness();
        let tenant = tenant();
        let sub = SubmissionId::generate("rollback-test");
        sqlite.create_submission(&tenant, &sub, None).await.unwrap();
        sqlite
            .add_manifest(&tenant, &sub, Some("http://provider/m.json"), None)
            .await
            .unwrap();
        let lease = sqlite
            .claim_next_manifest(&WorkerId::new("w-rb"), std::time::Duration::from_secs(60))
            .await
            .unwrap()
            .expect("claimable manifest");
        sqlite
            .process_entries(
                &tenant,
                &sub,
                &lease.manifest_id,
                vec![NdjsonEntry::new(
                    1,
                    "Patient",
                    json!({"resourceType": "Patient", "id": "p-rb-1"}),
                )],
                &BulkProcessingOptions::new(),
            )
            .await
            .unwrap();
        jobs.finish_manifest(&lease).await.unwrap();
        events.lock().clear();

        let changes = sqlite.list_changes(&tenant, &sub, 10, 0).await.unwrap();
        assert_eq!(changes.len(), 1);
        let rolled = jobs
            .rollback_change(&tenant, &sub, &changes[0])
            .await
            .unwrap();
        assert!(rolled);

        let seen = events.lock().clone();
        assert!(
            seen.contains(&"delete Patient/p-rb-1".to_string()),
            "a rolled-back create must delete from the secondary, got {seen:?}"
        );
    }
}
