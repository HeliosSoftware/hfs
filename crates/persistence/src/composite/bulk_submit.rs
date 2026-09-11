//! Bulk-submit job store for composite deployments.
//!
//! `$bulk-submit` ingestion runs on the *primary* backend's engine — that is
//! where the submission, manifest, lease, and receipt state live, and where
//! the primary deliberately skips its local search indexing when search is
//! offloaded to a secondary (Elasticsearch). Nothing on that path used to tell
//! the secondary, so bulk-loaded resources were readable but invisible to
//! every search (#882) — precisely the "bulk load, then search" workload the
//! composite exists for.
//!
//! [`CompositeSubmitJobs`] wraps the primary's job store and closes the gap as
//! [`SubmitWorkerStorage::sync_ingested`]: the worker calls it as an explicit
//! step **before** the manifest's receipt is written (#1007), not inside
//! `finish_manifest`/`fail_manifest` as it used to. Every successfully
//! ingested entry is read back from the primary and pushed through the
//! composite's normal secondary-sync machinery — the same [`SyncEvent`] batch
//! path an interactive `create_many` takes, honoring the configured sync
//! mode. A resource a secondary rejects after its retries is not silently
//! dropped: [`BulkSubmitProvider::mark_entries_unindexed`] flips its entry
//! results to `processing-error` with an OperationOutcome naming the
//! resource and the `$reindex` repair, so the receipt reflects what is
//! actually searchable instead of claiming `success` for it. `finish_manifest`
//! and `fail_manifest` themselves only delegate to the primary now.
//! `rollback_change` gets the mirror-image treatment so an aborted
//! submission's reverts reach the secondary too.
//!
//! Syncing before the receipt (rather than per entry or per file) keeps the
//! ingest engine untouched and the cost linear: one read + one batched sync
//! event per distinct ingested resource. The trade-off is that a manifest's
//! resources become searchable just before its receipt is written, not while
//! it streams — the status endpoint's percentage is the progress signal
//! during ingestion.
//!
//! After the sync, [`SubmitWorkerStorage::sync_ingested`] also runs a
//! best-effort **index drift check** (#1007): for every resource type the
//! manifest ingested, it compares the primary's tenant-wide
//! [`ResourceStorage::count`] against each secondary's. The check only runs
//! when [`CompositeStorage::syncs_search_synchronously`] is `true` — under
//! asynchronous sync a secondary's count reflects whatever had already
//! drained from its queue, not this manifest's sync, so the comparison would
//! be against a moving target rather than a real discrepancy; the check is
//! skipped entirely in that case. It never retries: a `count` failure or a
//! transient mismatch is logged and left for `$reindex`, the same repair the
//! rejected-resource path above already points operators to. A count
//! disagreement does not fail the manifest or any entry — it is surfaced as
//! a `warning` in the receipt's `error` artifact (see
//! [`crate::core::bulk_submit_worker::DefaultSubmitWorker`]).

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
    BulkSubmitRollbackProvider, ChangeType, EntryCountSummary, EntryResultContinuation,
    EntryResultPage, ManifestPhase, NdjsonEntry, StreamProcessingResult,
    StreamingBulkSubmitProvider, SubmissionChange, SubmissionId, SubmissionManifest,
    SubmissionStatus, SubmissionSummary, UnindexedEntry, entry_result_pages,
};
use crate::core::bulk_submit_publication::{ManifestPublicationResult, ManifestPublicationStatus};
use crate::core::bulk_submit_worker::{
    BulkSubmitJobStore, IndexDrift, IngestSyncReport, ManifestFetchParams, ManifestLease,
    ManifestWorkerView, PollTokenTarget, SubmitClaimStrategy, SubmitFileRecord, SubmitFileRow,
    SubmitWorkerStorage,
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

    /// Consumes a stream of successfully-ingested receipt pages — built by
    /// the caller from either the primary's live paginated cursor or, in
    /// tests, a scripted continuation sequence — pushing each page's
    /// resources through the composite's secondary sync in batches grouped
    /// by `(resource type, FHIR version)`, then marks whatever a secondary
    /// rejected after retries as `processing-error` on the primary (#1007).
    ///
    /// Distinct resources are synced once even when a manifest touched them
    /// on several lines; a resource deleted since ingestion (`read_ingested`)
    /// is reflected as a delete on the secondaries and does not count toward
    /// `synced`. A page fetch failure propagates instead of being swallowed:
    /// the receipt must not be written against an incomplete sync.
    async fn sync_ingested_pages(
        &self,
        lease: &ManifestLease,
        pages: impl futures::Stream<Item = StorageResult<EntryResultPage>>,
    ) -> Result<IngestSyncReport, LeaseError> {
        use futures::StreamExt;
        let mut seen: std::collections::HashSet<(String, String)> =
            std::collections::HashSet::new();
        // Attempted (not merely seen): a resource deleted since ingestion is
        // reflected as a delete on the secondaries instead (`read_ingested`)
        // and never enters a sync batch, so it must not count toward `synced`.
        let mut attempted = 0u64;
        // `(resource_type, resource_id)` -> the first backend that rejected it
        // and its error, for the entries that must be marked unindexed. An id
        // rejected by several secondaries is reported once, naming whichever
        // backend's rejection was seen first.
        let mut rejected: std::collections::HashMap<(String, String), (String, String)> =
            std::collections::HashMap::new();
        // Every resource type with at least one `success` receipt in this
        // manifest, for the post-sync drift check below. Collected here so
        // the check does not have to re-read the manifest.
        let mut types_seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        futures::pin_mut!(pages);
        while let Some(page) = pages.next().await {
            let page = page.map_err(LeaseError::Storage)?;
            // One batch per (type, FHIR version) per page, so the secondary
            // takes a page of ingested resources as one write rather than one
            // synchronous event each — under Elasticsearch `refresh=wait_for`
            // that is one refresh wait per page instead of per resource.
            let mut by_type: Vec<SyncGroup> = Vec::new();
            for entry in page.entries.into_iter().map(|entry| entry.result) {
                types_seen.insert(entry.resource_type.clone());
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
                attempted += resources.len() as u64;
                let statuses = self
                    .composite
                    .sync_creates_to_secondaries(
                        &lease.tenant,
                        &resource_type,
                        fhir_version,
                        resources,
                    )
                    .await;
                for status in statuses {
                    for resource_id in status.failed_resource_ids {
                        rejected
                            .entry((resource_type.clone(), resource_id))
                            .or_insert_with(|| {
                                (
                                    status.backend_id.clone(),
                                    status
                                        .error
                                        .clone()
                                        .unwrap_or_else(|| "unknown error".to_string()),
                                )
                            });
                    }
                }
            }
        }

        let unindexed = rejected.len() as u64;
        if unindexed > 0 {
            let entries: Vec<UnindexedEntry> = rejected
                .into_iter()
                .map(|((resource_type, resource_id), (backend_id, error))| {
                    let operation_outcome = serde_json::json!({
                        "resourceType": "OperationOutcome",
                        "issue": [{
                            "severity": "error",
                            "code": "incomplete",
                            "diagnostics": format!(
                                "{resource_type}/{resource_id} was stored but could not be \
                                 indexed for search on {backend_id}: {error}. Run \
                                 POST /{resource_type}/$reindex to repair."
                            )
                        }]
                    });
                    UnindexedEntry {
                        resource_type,
                        resource_id,
                        operation_outcome,
                    }
                })
                .collect();
            self.primary
                .mark_entries_unindexed(
                    &lease.tenant,
                    &lease.submission_id,
                    &lease.manifest_id,
                    &entries,
                )
                .await
                .map_err(LeaseError::Storage)?;
        }

        // Post-sync drift check (#1007): compare each ingested type's
        // tenant-wide count on the primary against every secondary's. Only
        // meaningful when the sync above was synchronous — otherwise a
        // secondary's count reflects whatever had already drained from its
        // queue, not this manifest's sync, and the comparison would be
        // against a moving target rather than a real discrepancy. Skipped
        // entirely in that case, with no retry: a transient miss here is the
        // same miss `$reindex` repairs, and this check must not slow down or
        // fail closing the manifest.
        let mut drift = Vec::new();
        if !self.composite.syncs_search_synchronously() {
            tracing::info!(
                submission = %lease.submission_id,
                manifest = %lease.manifest_id,
                "index drift check skipped: secondary sync is asynchronous"
            );
        } else {
            for resource_type in &types_seen {
                let primary_count = match self
                    .composite
                    .primary()
                    .count(&lease.tenant, Some(resource_type.as_str()))
                    .await
                {
                    Ok(count) => count,
                    Err(e) => {
                        warn!(
                            submission = %lease.submission_id,
                            manifest = %lease.manifest_id,
                            resource_type,
                            error = %e,
                            "bulk-submit: could not read the primary's count for the index \
                             drift check; skipping this type"
                        );
                        continue;
                    }
                };
                for (backend_id, secondary) in self.composite.secondaries() {
                    let search_count = match secondary
                        .count(&lease.tenant, Some(resource_type.as_str()))
                        .await
                    {
                        Ok(count) => count,
                        Err(e) => {
                            warn!(
                                submission = %lease.submission_id,
                                manifest = %lease.manifest_id,
                                resource_type,
                                backend_id,
                                error = %e,
                                "bulk-submit: could not read a secondary's count for the \
                                 index drift check; skipping this pair"
                            );
                            continue;
                        }
                    };
                    if primary_count != search_count {
                        drift.push(IndexDrift {
                            resource_type: resource_type.clone(),
                            backend_id: backend_id.clone(),
                            primary_count,
                            search_count,
                        });
                    }
                }
            }
        }

        Ok(IngestSyncReport {
            synced: attempted.saturating_sub(unindexed),
            unindexed,
            drift,
        })
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

    async fn get_entry_results_page(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_id: &str,
        outcome_filter: Option<BulkEntryOutcome>,
        limit: u32,
        continuation: Option<&EntryResultContinuation>,
    ) -> StorageResult<EntryResultPage> {
        self.primary
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
        self.primary
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
        self.primary
            .mark_entries_unindexed(tenant, submission_id, manifest_id, entries)
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

    async fn add_manifest_progress(
        &self,
        lease: &ManifestLease,
        processed_delta: u64,
        failed_delta: u64,
        lines_delta: u64,
    ) -> Result<(), LeaseError> {
        self.primary
            .add_manifest_progress(lease, processed_delta, failed_delta, lines_delta)
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

    async fn update_manifest_phase(
        &self,
        lease: &ManifestLease,
        phase: ManifestPhase,
        files_done: u64,
        files_total: u64,
    ) -> Result<(), LeaseError> {
        self.primary
            .update_manifest_phase(lease, phase, files_done, files_total)
            .await
    }

    async fn record_submit_file(
        &self,
        lease: &ManifestLease,
        file: &SubmitFileRecord,
    ) -> Result<(), LeaseError> {
        self.primary.record_submit_file(lease, file).await
    }

    async fn publish_manifest_artifacts(
        &self,
        lease: &ManifestLease,
        files: &[SubmitFileRecord],
        terminal: ManifestPublicationStatus,
    ) -> Result<ManifestPublicationResult, LeaseError> {
        // The worker calls `sync_ingested` itself, as an explicit step before
        // the receipt is built (#1007) — nothing left to do here but delegate.
        self.primary
            .publish_manifest_artifacts(lease, files, terminal)
            .await
    }

    async fn finish_manifest(&self, lease: &ManifestLease) -> Result<(), LeaseError> {
        // Ditto: nothing left to do here but delegate.
        self.primary.finish_manifest(lease).await
    }

    async fn fail_manifest(
        &self,
        lease: &ManifestLease,
        error_message: &str,
    ) -> Result<(), LeaseError> {
        // Ditto: the worker syncs before calling this on the fetch-error path too.
        self.primary.fail_manifest(lease, error_message).await
    }

    async fn sync_ingested(&self, lease: &ManifestLease) -> Result<IngestSyncReport, LeaseError> {
        let pages = entry_result_pages(|continuation| async move {
            self.primary
                .get_entry_results_page(
                    &lease.tenant,
                    &lease.submission_id,
                    &lease.manifest_id,
                    Some(BulkEntryOutcome::Success),
                    1000,
                    continuation.as_ref(),
                )
                .await
        });
        self.sync_ingested_pages(lease, pages).await
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
    use std::collections::{HashMap, HashSet};

    /// Records every write the composite syncs into the "secondary", and
    /// rejects `create`/`create_many` for any id in `reject` — the shape of a
    /// secondary search index that could not accept a bulk-submitted resource
    /// (mapping conflict, timeout, …) even after retries.
    ///
    /// `received` tracks every distinct id `create`/`create_or_update` was
    /// called with, by resource type, whether or not it was accepted —
    /// `count` reports its size unless `count_override` is set. This keeps a
    /// rejection test's own drift check honest: the primary really does hold
    /// a rejected resource (only its *search* indexing failed), so a
    /// realistic "index" double must say it heard about it too, or the new
    /// drift check would misreport the very rejection `mark_entries_unindexed`
    /// already names.
    struct SpySecondary {
        events: Arc<Mutex<Vec<String>>>,
        reject: std::collections::HashSet<String>,
        received: Arc<Mutex<HashMap<String, HashSet<String>>>>,
        count_override: Option<u64>,
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
            self.received
                .lock()
                .entry(resource_type.to_string())
                .or_default()
                .insert(id.clone());
            if self.reject.contains(&id) {
                return Err(crate::error::StorageError::Backend(
                    crate::error::BackendError::Internal {
                        backend_name: "spy-secondary".to_string(),
                        message: format!("secondary rejected {resource_type}/{id}"),
                        source: None,
                    },
                ));
            }
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
            self.received
                .lock()
                .entry(resource_type.to_string())
                .or_default()
                .insert(id.to_string());
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
            resource_type: Option<&str>,
        ) -> StorageResult<u64> {
            if let Some(n) = self.count_override {
                return Ok(n);
            }
            let ty = resource_type.unwrap_or("");
            Ok(self
                .received
                .lock()
                .get(ty)
                .map(|ids| ids.len())
                .unwrap_or(0) as u64)
        }
    }

    fn tenant() -> TenantContext {
        TenantContext::new(TenantId::new("t1"), TenantPermissions::full_access())
    }

    /// Builds a primary + composite + spy-secondary harness. `reject` names
    /// the ids the spy secondary refuses, for the rejection-handling tests;
    /// pass an empty set for the happy path. Synchronous sync mode, so no
    /// background worker is needed and events apply before the call returns.
    fn harness(
        reject: std::collections::HashSet<String>,
    ) -> (
        Arc<SqliteBackend>,
        CompositeSubmitJobs,
        Arc<Mutex<Vec<String>>>,
    ) {
        harness_with(
            reject,
            None,
            crate::composite::config::SyncMode::Synchronous,
        )
    }

    /// Like [`harness`], with the secondary's `count` overridable (for the
    /// index-drift tests) and the sync mode chosen by the caller (for the
    /// asynchronous-mode skip test).
    fn harness_with(
        reject: std::collections::HashSet<String>,
        count_override: Option<u64>,
        sync_mode: crate::composite::config::SyncMode,
    ) -> (
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
            .sync_mode(sync_mode)
            .build()
            .unwrap();
        let mut backends = HashMap::new();
        backends.insert("sqlite".to_string(), sqlite.clone() as DynStorage);
        backends.insert(
            "es".to_string(),
            Arc::new(SpySecondary {
                events: events.clone(),
                reject,
                received: Arc::new(Mutex::new(HashMap::new())),
                count_override,
            }) as DynStorage,
        );
        // No sync worker started: events apply synchronously, which is what
        // the assertions need.
        let composite = Arc::new(CompositeStorage::new(config, backends).unwrap());
        let jobs =
            CompositeSubmitJobs::new(sqlite.clone() as Arc<dyn BulkSubmitJobStore>, composite);
        (sqlite, jobs, events)
    }

    mod scripted_pages {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/bulk_submit/scripted_pages.rs"
        ));
    }

    /// #986: `sync_ingested_pages` must not treat an empty page carrying a
    /// continuation as end of traversal — it has to keep fetching until
    /// `next` is `None`.
    #[tokio::test]
    async fn sync_consumer_reads_beyond_an_empty_page_with_continuation() {
        let (sqlite, jobs, events) = harness(HashSet::new());
        let tenant = tenant();
        let sub = SubmissionId::generate("scripted-sync");
        sqlite.create_submission(&tenant, &sub, None).await.unwrap();
        sqlite
            .add_manifest(&tenant, &sub, Some("http://provider/m.json"), None)
            .await
            .unwrap();
        let lease = sqlite
            .claim_next_manifest(
                &WorkerId::new("scripted"),
                std::time::Duration::from_secs(60),
            )
            .await
            .unwrap()
            .unwrap();
        for id in ["after-empty", "exclusive-late"] {
            sqlite
                .create(
                    &tenant,
                    "Patient",
                    json!({"resourceType":"Patient", "id":id}),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }
        let (pages, calls) = scripted_pages::pages();
        jobs.sync_ingested_pages(&lease, pages).await.unwrap();
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 3);
        let mut actual = events.lock().clone();
        actual.sort();
        assert_eq!(
            actual,
            vec![
                "create Patient/after-empty",
                "create Patient/exclusive-late"
            ]
        );
    }

    /// #882/#1007: `sync_ingested` pushes every ingested resource into the
    /// secondary — through the raw primary engine, nothing ever did — as its
    /// own worker step, and `finish_manifest` no longer does this itself.
    #[tokio::test]
    async fn sync_ingested_pushes_every_ingested_resource_to_the_secondary() {
        let (sqlite, jobs, events) = harness(HashSet::new());
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

        let report = jobs.sync_ingested(&lease).await.unwrap();
        assert_eq!(
            report,
            IngestSyncReport {
                synced: 2,
                unindexed: 0,
                drift: Vec::new(),
            }
        );

        let seen = events.lock().clone();
        assert!(
            seen.contains(&"create Patient/p-sync-1".to_string()),
            "ingested resource must reach the secondary, got {seen:?}"
        );
        assert!(
            seen.contains(&"create Patient/p-sync-2".to_string()),
            "every distinct ingested resource syncs, got {seen:?}"
        );

        // The sync already happened as its own step; `finish_manifest` must
        // not sync again.
        let synced_event_count = events.lock().len();
        jobs.finish_manifest(&lease).await.unwrap();
        assert_eq!(
            events.lock().len(),
            synced_event_count,
            "finish_manifest must not add new sync events (#1007)"
        );

        // And the manifest actually finished on the primary.
        let manifests = sqlite.list_manifests(&tenant, &sub).await.unwrap();
        assert!(manifests[0].status.is_terminal());
    }

    /// #1007: a secondary that rejects a resource after retries must not
    /// leave its entry result reading `success` — it becomes
    /// `processing-error` with an OperationOutcome naming the repair.
    #[tokio::test]
    async fn sync_ingested_marks_rejected_resources_as_processing_error() {
        let reject: HashSet<String> = ["p-reject-1".to_string()].into_iter().collect();
        let (sqlite, jobs, _events) = harness(reject);
        let tenant = tenant();
        let sub = SubmissionId::generate("reject-test");
        sqlite.create_submission(&tenant, &sub, None).await.unwrap();
        sqlite
            .add_manifest(&tenant, &sub, Some("http://provider/m.json"), None)
            .await
            .unwrap();
        let lease = sqlite
            .claim_next_manifest(
                &WorkerId::new("w-reject"),
                std::time::Duration::from_secs(60),
            )
            .await
            .unwrap()
            .expect("claimable manifest");

        sqlite
            .process_entries(
                &tenant,
                &sub,
                &lease.manifest_id,
                vec![
                    NdjsonEntry::new(
                        1,
                        "Patient",
                        json!({"resourceType": "Patient", "id": "p-ok-1"}),
                    ),
                    NdjsonEntry::new(
                        2,
                        "Patient",
                        json!({"resourceType": "Patient", "id": "p-reject-1"}),
                    ),
                ],
                &BulkProcessingOptions::new(),
            )
            .await
            .unwrap();

        let report = jobs.sync_ingested(&lease).await.unwrap();
        assert_eq!(
            report,
            IngestSyncReport {
                synced: 1,
                unindexed: 1,
                drift: Vec::new(),
            }
        );

        let page = sqlite
            .get_entry_results_page(&tenant, &sub, &lease.manifest_id, None, 10, None)
            .await
            .unwrap();
        let results: Vec<_> = page.entries.into_iter().map(|e| e.result).collect();
        let rejected = results
            .iter()
            .find(|r| r.resource_id.as_deref() == Some("p-reject-1"))
            .expect("rejected entry present");
        assert_eq!(rejected.outcome, BulkEntryOutcome::ProcessingError);
        let oo = rejected
            .operation_outcome
            .as_ref()
            .expect("operation outcome recorded");
        assert_eq!(oo["issue"][0]["code"], "incomplete");
        let diagnostics = oo["issue"][0]["diagnostics"].as_str().unwrap();
        assert!(diagnostics.contains("Patient/p-reject-1"));
        assert!(diagnostics.contains("$reindex"));

        let ok = results
            .iter()
            .find(|r| r.resource_id.as_deref() == Some("p-ok-1"))
            .expect("accepted entry present");
        assert_eq!(ok.outcome, BulkEntryOutcome::Success);

        let counts = sqlite
            .get_entry_counts(&tenant, &sub, &lease.manifest_id)
            .await
            .unwrap();
        assert_eq!(counts.success, 1);
        assert_eq!(counts.processing_error, 1);
    }

    /// #1007: a tenant-wide count mismatch between the primary and a
    /// secondary, discovered right after this manifest's sync, is reported
    /// per resource type. With the spy's real received count (no override),
    /// the same sync reports no drift — the spy genuinely holds what the
    /// primary holds.
    #[tokio::test]
    async fn sync_ingested_reports_count_drift_per_type() {
        async fn ingest_two_patients(
            sqlite: &SqliteBackend,
            tenant: &TenantContext,
            sub: &SubmissionId,
        ) -> ManifestLease {
            sqlite.create_submission(tenant, sub, None).await.unwrap();
            sqlite
                .add_manifest(tenant, sub, Some("http://provider/m.json"), None)
                .await
                .unwrap();
            let lease = sqlite
                .claim_next_manifest(
                    &WorkerId::new("w-drift"),
                    std::time::Duration::from_secs(60),
                )
                .await
                .unwrap()
                .expect("claimable manifest");
            sqlite
                .process_entries(
                    tenant,
                    sub,
                    &lease.manifest_id,
                    vec![
                        NdjsonEntry::new(
                            1,
                            "Patient",
                            json!({"resourceType": "Patient", "id": "p-drift-1"}),
                        ),
                        NdjsonEntry::new(
                            2,
                            "Patient",
                            json!({"resourceType": "Patient", "id": "p-drift-2"}),
                        ),
                    ],
                    &BulkProcessingOptions::new(),
                )
                .await
                .unwrap();
            lease
        }

        // Overridden: the secondary claims only 1 Patient, disagreeing with
        // the primary's real count of 2.
        let (sqlite, jobs, _events) = harness_with(
            HashSet::new(),
            Some(1),
            crate::composite::config::SyncMode::Synchronous,
        );
        let tenant = tenant();
        let sub = SubmissionId::generate("drift-override");
        let lease = ingest_two_patients(&sqlite, &tenant, &sub).await;
        let report = jobs.sync_ingested(&lease).await.unwrap();
        assert_eq!(
            report.drift,
            vec![IndexDrift {
                resource_type: "Patient".to_string(),
                backend_id: "es".to_string(),
                primary_count: 2,
                search_count: 1,
            }]
        );

        // No override: the spy reports the ids it actually received, which
        // matches the primary — no drift.
        let (sqlite, jobs, _events) = harness_with(
            HashSet::new(),
            None,
            crate::composite::config::SyncMode::Synchronous,
        );
        let sub = SubmissionId::generate("drift-clean");
        let lease = ingest_two_patients(&sqlite, &tenant, &sub).await;
        let report = jobs.sync_ingested(&lease).await.unwrap();
        assert!(
            report.drift.is_empty(),
            "a secondary that truly received every ingested resource must not report drift, \
             got {:?}",
            report.drift
        );
    }

    /// #1007: under asynchronous sync, a secondary's count taken right after
    /// `sync_ingested` reflects whatever had already drained from the async
    /// queue, not this manifest's sync — so the drift check must not run at
    /// all, even when the secondary's (overridden) count would otherwise
    /// disagree with the primary's.
    #[tokio::test]
    async fn drift_check_is_skipped_when_sync_is_asynchronous() {
        let (sqlite, jobs, _events) = harness_with(
            HashSet::new(),
            Some(0),
            crate::composite::config::SyncMode::Asynchronous,
        );
        let tenant = tenant();
        let sub = SubmissionId::generate("drift-async");
        sqlite.create_submission(&tenant, &sub, None).await.unwrap();
        sqlite
            .add_manifest(&tenant, &sub, Some("http://provider/m.json"), None)
            .await
            .unwrap();
        let lease = sqlite
            .claim_next_manifest(
                &WorkerId::new("w-drift-async"),
                std::time::Duration::from_secs(60),
            )
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
                    json!({"resourceType": "Patient", "id": "p-async-1"}),
                )],
                &BulkProcessingOptions::new(),
            )
            .await
            .unwrap();

        let report = jobs.sync_ingested(&lease).await.unwrap();
        assert!(
            report.drift.is_empty(),
            "asynchronous sync must skip the drift check entirely, got {:?}",
            report.drift
        );
    }

    /// A rolled-back create is deleted from the secondary too.
    #[tokio::test]
    async fn rollback_of_a_create_deletes_from_the_secondary() {
        let (sqlite, jobs, events) = harness(HashSet::new());
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
