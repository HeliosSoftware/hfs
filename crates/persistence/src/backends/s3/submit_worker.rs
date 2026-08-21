//! `$bulk-submit` job store for the S3 backend.
//!
//! [`super::bulk_submit`] implements the synchronous ingestion engine; this
//! module adds the other half a Data Consumer needs — [`SubmitClaimStrategy`]
//! and [`SubmitWorkerStorage`] — so a standalone S3 primary can run the
//! `$bulk-submit` REST worker instead of reporting `501`.
//!
//! # Why the job store lives on S3 rather than a sidecar
//!
//! Bulk *export* pairs S3 with an embedded SQLite sidecar for job state, and the
//! obvious move would be to do the same here. It is the wrong one: a submit job
//! store is not a separate queue alongside the data, it is the *same*
//! submission and manifest records the ingestion engine already mutates — which
//! S3 has stored under `bulk/submit/…` since the engine was written. Splitting
//! them would put one submission's truth in two stores with no transaction
//! across them.
//!
//! # Leasing without a row lock
//!
//! A lease is compare-and-swapped against the manifest object's S3 ETag:
//! `GetObject` returns body and ETag from one snapshot, and `PutObject` with
//! `If-Match` fails with `PreconditionFailed` if anything changed in between.
//! That is a genuine CAS, so two workers cannot both claim a manifest and a
//! zombie worker's write is refused once its `fencing_token` no longer matches.
//!
//! What S3 does *not* give is a query. Three lookups the worker and REST layer
//! need are cross-tenant and cannot scan tenant prefixes:
//!
//! | Lookup | Index namespace |
//! |---|---|
//! | next claimable manifest | `_system.bulk-submit/queue/` |
//! | poll token → submission | `_system.bulk-submit/tokens/` |
//! | submissions past their TTL | `_system.bulk-submit/submissions/` |
//!
//! Each entry is a small [`SubmitIndexEntry`] pointing back at the tenant-scoped
//! objects; the manifest/submission objects remain the source of truth, so a
//! stale or orphaned index entry costs a wasted read, never a wrong answer.
//!
//! The index lives outside every tenant prefix, so a bucket-per-tenant
//! configuration with no `default_system_bucket` has nowhere to put it. Such a
//! deployment does not declare `BulkSubmitRestWorker` (see
//! [`S3Backend::supports_bulk_submit_worker`]) and these methods report the
//! misconfiguration rather than writing into an arbitrary tenant's bucket —
//! exactly how the per-user settings store behaves.

use std::time::Duration as StdDuration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use helios_fhir::FhirVersion;
use uuid::Uuid;

use crate::core::bulk_export::ExportJobId;
use crate::core::bulk_export_worker::{LeaseError, WorkerId};
use crate::core::bulk_submit::{
    ManifestStatus, SubmissionId, SubmissionManifest, SubmissionStatus,
};
use crate::core::bulk_submit_worker::{
    ManifestFetchParams, ManifestLease, ManifestWorkerView, PollTokenTarget, SubmitClaimStrategy,
    SubmitFileRecord, SubmitFileRow, SubmitWorkerStorage,
};
use crate::error::{BackendError, StorageError, StorageResult};
use crate::tenant::{TenantContext, TenantId, TenantPermissions};

use super::backend::{S3Backend, TenantLocation};
use super::client::S3ClientError;
use super::keyspace::submit_index_object_id;
use super::models::{SubmissionManifestState, SubmitIndexEntry};

/// Index namespace holding one entry per manifest awaiting or under a lease.
const QUEUE_NAMESPACE: &str = "queue";
/// Index namespace mapping a poll token to its submission.
const TOKEN_NAMESPACE: &str = "tokens";
/// Index namespace holding one entry per submission, for the TTL sweep.
const REGISTRY_NAMESPACE: &str = "submissions";

/// How many queued manifests one claim attempt will race for before reporting
/// "nothing available". A CAS can lose; walking a few candidates keeps a worker
/// that keeps losing the head of the queue from starving.
const CLAIM_CANDIDATES: usize = 16;

/// Attempts at a compare-and-swap that lost its race. Counts total attempts.
const CAS_ATTEMPTS: usize = 5;

fn internal_error(message: impl Into<String>) -> StorageError {
    StorageError::Backend(BackendError::Internal {
        backend_name: "s3".to_string(),
        message: message.into(),
        source: None,
    })
}

/// Builds a `LeaseError::LeaseLost` for a submit manifest (the shared variant
/// carries an `ExportJobId`, so we encode `submission/manifest` into it).
fn lease_lost(lease: &ManifestLease) -> LeaseError {
    LeaseError::LeaseLost {
        job_id: ExportJobId::from_string(format!("{}/{}", lease.submission_id, lease.manifest_id)),
    }
}

/// Derives the ingest FHIR version from a stored `outputFormat` MIME string.
fn fhir_version_from_output_format(output_format: Option<&str>) -> FhirVersion {
    output_format
        .and_then(|fmt| {
            fmt.split(';').find_map(|part| {
                let part = part.trim();
                part.strip_prefix("fhirVersion=")
                    .and_then(FhirVersion::from_mime_param)
            })
        })
        .unwrap_or_else(FhirVersion::default_enabled)
}

/// Rebuilds a full-access `TenantContext` from an index entry's `tenant_id`.
///
/// The worker and cleanup paths resolve submissions before any request context
/// exists, exactly as the SQL job stores do.
fn tenant_from_id(tenant_id: &str) -> TenantContext {
    TenantContext::new(TenantId::new(tenant_id), TenantPermissions::full_access())
}

impl S3Backend {
    /// Whether this configuration can host the `$bulk-submit` job store.
    ///
    /// False only in bucket-per-tenant mode with no `default_system_bucket`,
    /// where the cross-tenant worker index has nowhere to live. Mirrors
    /// [`supports_user_settings`](S3Backend::supports_user_settings), and gates
    /// the `BulkSubmitRestWorker` capability so such a deployment gets the
    /// explained `501` from `$bulk-submit` rather than a failure per request.
    /// Synchronous ingestion (`BulkSubmitIngest`) is unaffected — it is entirely
    /// tenant-scoped.
    pub fn supports_bulk_submit_worker(&self) -> bool {
        self.shared_location().is_some()
    }

    /// Resolves the bucket and keyspace holding the cross-tenant worker index.
    fn submit_index_location(&self) -> StorageResult<TenantLocation> {
        self.shared_location().ok_or_else(|| {
            internal_error(
                "the $bulk-submit worker index requires a tenant-independent bucket: set \
                 `default_system_bucket` in bucket-per-tenant mode, or disable bulk submit",
            )
        })
    }

    /// Writes (or overwrites) one worker-index entry.
    async fn put_index_entry(
        &self,
        namespace: &str,
        object_id: &str,
        entry: &SubmitIndexEntry,
    ) -> StorageResult<()> {
        let location = self.submit_index_location()?;
        let key = location.keyspace.submit_index_key(namespace, object_id);
        let payload = self.serialize_json(entry)?;
        self.put_json_object(&location.bucket, &key, &payload, None, None)
            .await?;
        Ok(())
    }

    /// Removes one worker-index entry. Idempotent.
    async fn delete_index_entry(&self, namespace: &str, object_id: &str) -> StorageResult<()> {
        let location = self.submit_index_location()?;
        let key = location.keyspace.submit_index_key(namespace, object_id);
        self.client
            .delete_object(&location.bucket, &key)
            .await
            .map_err(|e| self.map_client_error(e))
    }

    /// Reads every entry in one worker-index namespace.
    async fn list_index_entries(&self, namespace: &str) -> StorageResult<Vec<SubmitIndexEntry>> {
        self.list_index_entries_limited(namespace, usize::MAX).await
    }

    /// Reads at most `limit` entries from one worker-index namespace, oldest
    /// first.
    ///
    /// Age comes from the listing's `last_modified`, not from the entry body, so
    /// the claim path pays one `ListObjects` plus `limit` `GetObject`s per poll
    /// instead of one `GetObject` per queued manifest. For the queue that
    /// ordering *is* the enqueue order, since an entry is written once when its
    /// manifest is added and deleted when it reaches a terminal state.
    async fn list_index_entries_limited(
        &self,
        namespace: &str,
        limit: usize,
    ) -> StorageResult<Vec<SubmitIndexEntry>> {
        let location = self.submit_index_location()?;
        let prefix = location.keyspace.submit_index_prefix(namespace);

        let mut objects: Vec<_> = self
            .list_objects_all(&location.bucket, &prefix)
            .await?
            .into_iter()
            .filter(|object| object.key.ends_with(".json"))
            .collect();
        // Key order breaks ties: the keys are digests, so it is arbitrary but
        // stable, which keeps a same-millisecond batch from reshuffling between
        // polls and starving one entry.
        objects.sort_by(|a, b| (a.last_modified, &a.key).cmp(&(b.last_modified, &b.key)));
        objects.truncate(limit);

        let mut out = Vec::new();
        for object in objects {
            if let Some((entry, _)) = self
                .get_json_object::<SubmitIndexEntry>(&location.bucket, &object.key)
                .await?
            {
                out.push(entry);
            }
        }
        Ok(out)
    }

    /// Queue-entry id for one manifest of one submission.
    fn queue_object_id(tenant: &TenantContext, id: &SubmissionId, manifest_id: &str) -> String {
        submit_index_object_id(&[
            tenant.tenant_id().as_str(),
            &id.submitter,
            &id.submission_id,
            manifest_id,
        ])
    }

    /// Registry-entry id for one submission.
    fn registry_object_id(tenant: &TenantContext, id: &SubmissionId) -> String {
        submit_index_object_id(&[
            tenant.tenant_id().as_str(),
            &id.submitter,
            &id.submission_id,
        ])
    }

    /// Adds a manifest to the claim queue.
    ///
    /// Called from `add_manifest` for every manifest that names a
    /// `manifestUrl`. A deployment that cannot host the index (bucket-per-tenant
    /// with no system bucket) silently skips this: ingestion still works, and
    /// the `BulkSubmitRestWorker` capability it never declared is what tells a
    /// caller the worker is unavailable.
    pub(crate) async fn enqueue_manifest(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
        manifest: &SubmissionManifest,
    ) -> StorageResult<()> {
        if !self.supports_bulk_submit_worker() {
            return Ok(());
        }
        self.put_index_entry(
            QUEUE_NAMESPACE,
            &Self::queue_object_id(tenant, id, &manifest.manifest_id),
            &SubmitIndexEntry {
                tenant_id: tenant.tenant_id().as_str().to_string(),
                submitter: id.submitter.clone(),
                submission_id: id.submission_id.clone(),
                manifest_id: Some(manifest.manifest_id.clone()),
                added_at: Some(manifest.added_at),
                updated_at: None,
                status: None,
            },
        )
        .await
    }

    /// Removes a manifest from the claim queue once it reaches a terminal state.
    pub(crate) async fn dequeue_manifest(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
        manifest_id: &str,
    ) -> StorageResult<()> {
        if !self.supports_bulk_submit_worker() {
            return Ok(());
        }
        self.delete_index_entry(
            QUEUE_NAMESPACE,
            &Self::queue_object_id(tenant, id, manifest_id),
        )
        .await
    }

    /// Records the submission's latest activity and status.
    ///
    /// This entry backs two lookups that have no tenant in hand or would
    /// otherwise mean walking the whole `bulk/submit/` tree: the TTL sweep
    /// ([`SubmitWorkerStorage::list_expired_submissions`]) and the per-tenant
    /// concurrency cap ([`SubmitWorkerStorage::count_active_submissions`]).
    pub(crate) async fn touch_submit_registry(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
        status: SubmissionStatus,
    ) -> StorageResult<()> {
        if !self.supports_bulk_submit_worker() {
            return Ok(());
        }
        self.put_index_entry(
            REGISTRY_NAMESPACE,
            &Self::registry_object_id(tenant, id),
            &SubmitIndexEntry {
                tenant_id: tenant.tenant_id().as_str().to_string(),
                submitter: id.submitter.clone(),
                submission_id: id.submission_id.clone(),
                manifest_id: None,
                added_at: None,
                updated_at: Some(Utc::now()),
                status: Some(status),
            },
        )
        .await
    }

    /// Loads a manifest state together with the ETag a conditional write must
    /// be made against.
    ///
    /// One `GetObject` returns body and ETag from the same snapshot; a
    /// `HeadObject` + `GetObject` pair could straddle a concurrent write and
    /// pair one generation's state with another's ETag.
    async fn load_manifest_for_cas(
        &self,
        location: &TenantLocation,
        id: &SubmissionId,
        manifest_id: &str,
    ) -> StorageResult<Option<(SubmissionManifestState, Option<String>)>> {
        let key =
            location
                .keyspace
                .submit_manifest_key(&id.submitter, &id.submission_id, manifest_id);
        Ok(self
            .get_json_object::<SubmissionManifestState>(&location.bucket, &key)
            .await?
            .map(|(state, metadata)| (state, metadata.etag)))
    }

    /// Writes a manifest state back only if it has not changed since it was
    /// read. `Ok(false)` means the compare-and-swap lost.
    async fn save_manifest_if_unchanged(
        &self,
        location: &TenantLocation,
        id: &SubmissionId,
        state: &SubmissionManifestState,
        etag: Option<&str>,
    ) -> StorageResult<bool> {
        let key = location.keyspace.submit_manifest_key(
            &id.submitter,
            &id.submission_id,
            &state.manifest.manifest_id,
        );
        let payload = self.serialize_json(state)?;
        // No ETag means the object was absent when read, so the write must fail
        // if anyone created it since.
        let (if_match, if_none_match) = match etag {
            Some(etag) => (Some(etag), None),
            None => (None, Some("*")),
        };
        match self
            .client
            .put_object(
                &location.bucket,
                &key,
                payload,
                Some("application/json"),
                if_match,
                if_none_match,
            )
            .await
        {
            Ok(_) => Ok(true),
            Err(S3ClientError::PreconditionFailed) => Ok(false),
            Err(e) => Err(self.map_client_error(e)),
        }
    }

    /// Applies `mutate` to the leased manifest under compare-and-swap, after
    /// verifying the lease still belongs to this worker.
    ///
    /// This is the fence: a worker whose manifest was reclaimed sees a different
    /// `worker_id`/`fencing_token` and gets `LeaseLost` instead of corrupting
    /// the new holder's progress.
    async fn fenced_mutate<F>(&self, lease: &ManifestLease, mutate: F) -> Result<(), LeaseError>
    where
        F: Fn(&mut SubmissionManifestState),
    {
        let location = self
            .tenant_location(&lease.tenant)
            .map_err(LeaseError::Storage)?;

        for _ in 0..CAS_ATTEMPTS {
            let loaded = self
                .load_manifest_for_cas(&location, &lease.submission_id, &lease.manifest_id)
                .await
                .map_err(LeaseError::Storage)?;
            let Some((mut state, etag)) = loaded else {
                return Err(lease_lost(lease));
            };
            if !holds_lease(&state, lease) {
                return Err(lease_lost(lease));
            }

            mutate(&mut state);
            let written = self
                .save_manifest_if_unchanged(
                    &location,
                    &lease.submission_id,
                    &state,
                    etag.as_deref(),
                )
                .await
                .map_err(LeaseError::Storage)?;
            if written {
                return Ok(());
            }
        }

        Err(LeaseError::Storage(internal_error(format!(
            "manifest {} of submission {} could not be updated after {CAS_ATTEMPTS} \
             compare-and-swap attempts",
            lease.manifest_id, lease.submission_id
        ))))
    }

    /// Applies `mutate` to a submission state under compare-and-swap, returning
    /// what `mutate` produced.
    async fn submission_cas<T, F>(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
        mutate: F,
    ) -> StorageResult<T>
    where
        F: Fn(&mut super::models::SubmissionState) -> T,
    {
        let location = self.tenant_location(tenant)?;
        let key = location
            .keyspace
            .submit_state_key(&id.submitter, &id.submission_id);

        for _ in 0..CAS_ATTEMPTS {
            let Some((mut state, metadata)) = self
                .get_json_object::<super::models::SubmissionState>(&location.bucket, &key)
                .await?
            else {
                return Err(StorageError::BulkSubmit(
                    crate::error::BulkSubmitError::SubmissionNotFound {
                        submitter: id.submitter.clone(),
                        submission_id: id.submission_id.clone(),
                    },
                ));
            };

            let outcome = mutate(&mut state);
            let payload = self.serialize_json(&state)?;
            match self
                .client
                .put_object(
                    &location.bucket,
                    &key,
                    payload,
                    Some("application/json"),
                    metadata.etag.as_deref(),
                    None,
                )
                .await
            {
                Ok(_) => return Ok(outcome),
                Err(S3ClientError::PreconditionFailed) => continue,
                Err(e) => return Err(self.map_client_error(e)),
            }
        }

        Err(internal_error(format!(
            "submission {id} could not be updated after {CAS_ATTEMPTS} compare-and-swap attempts"
        )))
    }
}

/// Whether `state`'s lease fields still name the holder `lease` describes.
fn holds_lease(state: &SubmissionManifestState, lease: &ManifestLease) -> bool {
    state.worker_id.as_deref() == Some(lease.worker_id.as_str())
        && state.fencing_token == lease.fencing_token
}

#[async_trait]
impl SubmitClaimStrategy for S3Backend {
    async fn claim_next_manifest(
        &self,
        worker_id: &WorkerId,
        lease_duration: StdDuration,
    ) -> StorageResult<Option<ManifestLease>> {
        if !self.supports_bulk_submit_worker() {
            return Ok(None);
        }

        let now = Utc::now();
        let lease_expiry = now
            + chrono::Duration::from_std(lease_duration)
                .unwrap_or_else(|_| chrono::Duration::seconds(60));

        let mut queued = self
            .list_index_entries_limited(QUEUE_NAMESPACE, CLAIM_CANDIDATES)
            .await?;
        // The listing already ordered these by write time; re-sort on the
        // recorded `added_at`, which is the manifest's own arrival time and so
        // survives an index entry being rewritten.
        queued.sort_by_key(|entry| entry.added_at.unwrap_or(now));

        for entry in queued {
            let Some(manifest_id) = entry.manifest_id.clone() else {
                continue;
            };
            let tenant = tenant_from_id(&entry.tenant_id);
            let id = SubmissionId::new(entry.submitter.clone(), entry.submission_id.clone());
            let Ok(location) = self.tenant_location(&tenant) else {
                continue;
            };

            let Some((mut state, etag)) = self
                .load_manifest_for_cas(&location, &id, &manifest_id)
                .await?
            else {
                // The manifest is gone; the queue entry is stale.
                self.dequeue_manifest(&tenant, &id, &manifest_id).await?;
                continue;
            };

            // Eligible: never started, or held by a worker that stopped
            // heartbeating. Anything terminal leaves the queue instead.
            let eligible = match state.manifest.status {
                ManifestStatus::Pending => true,
                ManifestStatus::Processing => state.lease_expiry.is_none_or(|expiry| expiry < now),
                ManifestStatus::Completed | ManifestStatus::Failed | ManifestStatus::Replaced => {
                    self.dequeue_manifest(&tenant, &id, &manifest_id).await?;
                    false
                }
            };
            if !eligible || state.manifest.manifest_url.is_none() {
                continue;
            }

            // `complete` is admitted alongside `in-progress`: it means the
            // submitter will send no further manifests, not that already
            // registered ones should be dropped. `aborted` stays excluded.
            let submission_status = self
                .get_json_object::<super::models::SubmissionState>(
                    &location.bucket,
                    &location
                        .keyspace
                        .submit_state_key(&id.submitter, &id.submission_id),
                )
                .await?
                .map(|(state, _)| state.summary.status);
            let admissible = matches!(
                submission_status,
                Some(crate::core::bulk_submit::SubmissionStatus::InProgress)
                    | Some(crate::core::bulk_submit::SubmissionStatus::Complete)
            );
            if !admissible {
                continue;
            }

            let new_token = state.fencing_token + 1;
            state.manifest.status = ManifestStatus::Processing;
            state.worker_id = Some(worker_id.as_str().to_string());
            state.lease_expiry = Some(lease_expiry);
            state.fencing_token = new_token;

            // Losing here means another worker claimed it between our read and
            // our write: move on to the next candidate rather than retrying,
            // since it is no longer available anyway.
            if !self
                .save_manifest_if_unchanged(&location, &id, &state, etag.as_deref())
                .await?
            {
                continue;
            }

            return Ok(Some(ManifestLease {
                tenant,
                submission_id: id,
                manifest_id,
                worker_id: worker_id.clone(),
                lease_expiry,
                fencing_token: new_token,
            }));
        }

        Ok(None)
    }

    async fn heartbeat(&self, lease: &ManifestLease) -> Result<DateTime<Utc>, LeaseError> {
        let new_expiry = Utc::now() + chrono::Duration::seconds(60);
        self.fenced_mutate(lease, |state| {
            state.lease_expiry = Some(new_expiry);
        })
        .await?;
        Ok(new_expiry)
    }

    async fn release(&self, lease: ManifestLease) -> StorageResult<()> {
        // Best-effort: a lease we no longer hold has already been reclaimed by
        // someone else, and there is nothing to give back.
        let _ = self
            .fenced_mutate(&lease, |state| {
                if state.manifest.status == ManifestStatus::Processing {
                    state.manifest.status = ManifestStatus::Pending;
                    state.worker_id = None;
                    state.lease_expiry = None;
                }
            })
            .await;
        Ok(())
    }
}

#[async_trait]
impl SubmitWorkerStorage for S3Backend {
    async fn get_manifest_for_worker(
        &self,
        lease: &ManifestLease,
    ) -> Result<ManifestWorkerView, LeaseError> {
        let location = self
            .tenant_location(&lease.tenant)
            .map_err(LeaseError::Storage)?;
        let (state, _) = self
            .load_manifest_for_cas(&location, &lease.submission_id, &lease.manifest_id)
            .await
            .map_err(LeaseError::Storage)?
            .ok_or_else(|| lease_lost(lease))?;
        if !holds_lease(&state, lease) {
            return Err(lease_lost(lease));
        }

        Ok(ManifestWorkerView {
            manifest_id: lease.manifest_id.clone(),
            manifest_url: state.manifest.manifest_url.clone(),
            fhir_base_url: state.fhir_base_url.clone(),
            fhir_version: fhir_version_from_output_format(state.output_format.as_deref()),
            output_format: state.output_format,
            file_request_headers: state.file_request_headers,
            oauth_metadata_urls: state.oauth_metadata_urls,
            file_encryption_key: state.file_encryption_key,
            import_directives: state.import_directives,
            metadata: state.submission_metadata,
            last_processed_line: state.last_processed_line,
        })
    }

    async fn mark_manifest_processing(&self, lease: &ManifestLease) -> Result<(), LeaseError> {
        self.fenced_mutate(lease, |state| {
            state.manifest.status = ManifestStatus::Processing;
        })
        .await
    }

    async fn update_manifest_progress(
        &self,
        lease: &ManifestLease,
        processed_entries: u64,
        failed_entries: u64,
        last_processed_line: u64,
    ) -> Result<(), LeaseError> {
        self.fenced_mutate(lease, |state| {
            state.manifest.processed_entries = processed_entries;
            state.manifest.failed_entries = failed_entries;
            state.last_processed_line = last_processed_line;
        })
        .await
    }

    async fn record_submit_file(
        &self,
        lease: &ManifestLease,
        file: &SubmitFileRecord,
    ) -> Result<(), LeaseError> {
        let location = self
            .tenant_location(&lease.tenant)
            .map_err(LeaseError::Storage)?;
        let (state, _) = self
            .load_manifest_for_cas(&location, &lease.submission_id, &lease.manifest_id)
            .await
            .map_err(LeaseError::Storage)?
            .ok_or_else(|| lease_lost(lease))?;
        if !holds_lease(&state, lease) {
            return Err(lease_lost(lease));
        }

        let row = SubmitFileRow {
            manifest_url: file.manifest_url.clone(),
            file_type: file.file_type.clone(),
            resource_type: file.resource_type.clone(),
            part_index: file.part_index,
            fencing_token: lease.fencing_token,
            file_path: file.file_path.clone(),
            line_count: file.line_count,
            byte_count: file.byte_count,
            count_severity: file.count_severity.clone(),
        };
        let key = location.keyspace.submit_file_key(
            &lease.submission_id.submitter,
            &lease.submission_id.submission_id,
            &file.file_type,
            file.resource_type.as_deref(),
            file.part_index,
            lease.fencing_token,
        );
        // The key carries the artifact's full identity, so an overwrite is the
        // idempotent retry this method promises.
        let payload = self
            .serialize_json(&SubmitFileRowRecord::from(&row))
            .map_err(LeaseError::Storage)?;
        self.put_json_object(&location.bucket, &key, &payload, None, None)
            .await
            .map_err(LeaseError::Storage)?;
        Ok(())
    }

    async fn finish_manifest(&self, lease: &ManifestLease) -> Result<(), LeaseError> {
        self.fenced_mutate(lease, |state| {
            state.manifest.status = ManifestStatus::Completed;
            state.worker_id = None;
            state.lease_expiry = None;
        })
        .await?;
        self.dequeue_manifest(&lease.tenant, &lease.submission_id, &lease.manifest_id)
            .await
            .map_err(LeaseError::Storage)
    }

    async fn fail_manifest(
        &self,
        lease: &ManifestLease,
        error_message: &str,
    ) -> Result<(), LeaseError> {
        let message = error_message.to_string();
        self.fenced_mutate(lease, move |state| {
            state.manifest.status = ManifestStatus::Failed;
            state.error_message = Some(message.clone());
            state.worker_id = None;
            state.lease_expiry = None;
        })
        .await?;
        self.dequeue_manifest(&lease.tenant, &lease.submission_id, &lease.manifest_id)
            .await
            .map_err(LeaseError::Storage)
    }

    async fn set_manifest_fetch_params(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
        manifest_id: &str,
        params: ManifestFetchParams<'_>,
    ) -> StorageResult<()> {
        let location = self.tenant_location(tenant)?;

        for _ in 0..CAS_ATTEMPTS {
            let Some((mut state, etag)) = self
                .load_manifest_for_cas(&location, id, manifest_id)
                .await?
            else {
                return Err(StorageError::BulkSubmit(
                    crate::error::BulkSubmitError::ManifestNotFound {
                        submission_id: id.submission_id.clone(),
                        manifest_id: manifest_id.to_string(),
                    },
                ));
            };

            state.fhir_base_url = params.fhir_base_url.map(str::to_string);
            state.output_format = params.output_format.map(str::to_string);
            state.file_request_headers = params.file_request_headers.to_vec();
            state.oauth_metadata_urls = params.oauth_metadata_urls.to_vec();
            state.file_encryption_key = params.file_encryption_key.cloned();
            state.import_directives = params.import_directives.to_vec();
            state.submission_metadata = params.metadata.to_vec();

            if self
                .save_manifest_if_unchanged(&location, id, &state, etag.as_deref())
                .await?
            {
                return Ok(());
            }
        }

        Err(internal_error(format!(
            "manifest {manifest_id} of submission {id} could not be updated after \
             {CAS_ATTEMPTS} compare-and-swap attempts"
        )))
    }

    async fn replace_manifest_by_url(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
        manifest_url: &str,
    ) -> StorageResult<Vec<String>> {
        let location = self.tenant_location(tenant)?;
        let mut superseded = Vec::new();

        for candidate in self.list_manifest_states(&location, id).await? {
            if candidate.manifest.manifest_url.as_deref() != Some(manifest_url) {
                continue;
            }
            let manifest_id = candidate.manifest.manifest_id.clone();
            if candidate.manifest.status != ManifestStatus::Replaced {
                superseded.push(manifest_id.clone());
            }

            // Re-read under compare-and-swap: a worker may be mid-manifest, and
            // writing the copy listed above would roll its progress back.
            for _ in 0..CAS_ATTEMPTS {
                let Some((mut state, etag)) = self
                    .load_manifest_for_cas(&location, id, &manifest_id)
                    .await?
                else {
                    break;
                };
                state.manifest.status = ManifestStatus::Replaced;
                state.worker_id = None;
                state.lease_expiry = None;
                if self
                    .save_manifest_if_unchanged(&location, id, &state, etag.as_deref())
                    .await?
                {
                    break;
                }
            }

            // A replaced manifest is no longer work.
            self.dequeue_manifest(tenant, id, &manifest_id).await?;
        }

        Ok(superseded)
    }

    async fn set_submission_kickoff_meta(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
        owner_subject: Option<&str>,
        request_url: &str,
        requires_access_token: bool,
    ) -> StorageResult<()> {
        let owner_subject = owner_subject.map(str::to_string);
        let request_url = request_url.to_string();
        let status = self
            .submission_cas(tenant, id, move |state| {
                state.owner_subject = owner_subject.clone();
                state.request_url = Some(request_url.clone());
                state.requires_access_token = Some(requires_access_token);
                state.summary.status
            })
            .await?;
        self.touch_submit_registry(tenant, id, status).await
    }

    async fn ensure_poll_token(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<String> {
        let minted = Uuid::new_v4().to_string();
        // The CAS is what makes this idempotent: a concurrent caller that got
        // there first has already written its token, and we return that one.
        let token = self
            .submission_cas(tenant, id, |state| {
                state
                    .poll_token
                    .get_or_insert_with(|| minted.clone())
                    .clone()
            })
            .await?;

        self.put_index_entry(
            TOKEN_NAMESPACE,
            &submit_index_object_id(&[&token]),
            &SubmitIndexEntry {
                tenant_id: tenant.tenant_id().as_str().to_string(),
                submitter: id.submitter.clone(),
                submission_id: id.submission_id.clone(),
                manifest_id: None,
                added_at: None,
                updated_at: Some(Utc::now()),
                status: None,
            },
        )
        .await?;
        Ok(token)
    }

    async fn resolve_poll_token(&self, token: &str) -> StorageResult<Option<PollTokenTarget>> {
        if !self.supports_bulk_submit_worker() {
            return Ok(None);
        }
        let location = self.submit_index_location()?;
        let key = location
            .keyspace
            .submit_index_key(TOKEN_NAMESPACE, &submit_index_object_id(&[token]));
        let Some((entry, _)) = self
            .get_json_object::<SubmitIndexEntry>(&location.bucket, &key)
            .await?
        else {
            return Ok(None);
        };

        let tenant = tenant_from_id(&entry.tenant_id);
        let id = SubmissionId::new(entry.submitter.clone(), entry.submission_id.clone());

        // The submission object is the source of truth for both the token and
        // the owner: an index entry left behind by a cleared token must not keep
        // resolving, and the owner must not be read from the index, which the
        // ownership check would then be trusting to be in sync.
        let tenant_location = self.tenant_location(&tenant)?;
        let state_key = tenant_location
            .keyspace
            .submit_state_key(&id.submitter, &id.submission_id);
        let Some((state, _)) = self
            .get_json_object::<super::models::SubmissionState>(&tenant_location.bucket, &state_key)
            .await?
        else {
            return Ok(None);
        };
        if state.poll_token.as_deref() != Some(token) {
            return Ok(None);
        }

        Ok(Some(PollTokenTarget {
            tenant,
            submission_id: id,
            owner_subject: state.owner_subject,
        }))
    }

    async fn clear_poll_token(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<()> {
        let cleared = self
            .submission_cas(tenant, id, |state| state.poll_token.take())
            .await?;
        if let Some(token) = cleared {
            self.delete_index_entry(TOKEN_NAMESPACE, &submit_index_object_id(&[&token]))
                .await?;
        }
        Ok(())
    }

    async fn list_submit_files(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<Vec<SubmitFileRow>> {
        let location = self.tenant_location(tenant)?;
        let prefix = location
            .keyspace
            .submit_files_prefix(&id.submitter, &id.submission_id);

        let mut rows = Vec::new();
        for object in self.list_objects_all(&location.bucket, &prefix).await? {
            if !object.key.ends_with(".json") {
                continue;
            }
            if let Some((record, _)) = self
                .get_json_object::<SubmitFileRowRecord>(&location.bucket, &object.key)
                .await?
            {
                rows.push(SubmitFileRow::from(record));
            }
        }
        // S3 lists lexicographically, which is not the order the parts were
        // written; sort so the status manifest is stable across polls.
        rows.sort_by(|a, b| {
            (
                &a.file_type,
                &a.resource_type,
                a.part_index,
                a.fencing_token,
            )
                .cmp(&(
                    &b.file_type,
                    &b.resource_type,
                    b.part_index,
                    b.fencing_token,
                ))
        });
        Ok(rows)
    }

    async fn delete_submission_artifacts(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<()> {
        let location = self.tenant_location(tenant)?;
        let prefix = location
            .keyspace
            .submit_files_prefix(&id.submitter, &id.submission_id);
        for object in self.list_objects_all(&location.bucket, &prefix).await? {
            self.client
                .delete_object(&location.bucket, &object.key)
                .await
                .map_err(|e| self.map_client_error(e))?;
        }
        if self.supports_bulk_submit_worker() {
            self.delete_index_entry(REGISTRY_NAMESPACE, &Self::registry_object_id(tenant, id))
                .await?;
        }
        Ok(())
    }

    async fn count_active_submissions(&self, tenant: &TenantContext) -> StorageResult<u64> {
        if !self.supports_bulk_submit_worker() {
            return Ok(0);
        }
        // Counted from the worker index rather than by listing `bulk/submit/`,
        // whose prefix also contains every archived NDJSON line and entry
        // result. Submissions written before the index existed carry no entry
        // and so do not count toward the cap; that under-counts an old backlog
        // rather than wrongly rejecting new work.
        Ok(self
            .list_index_entries(REGISTRY_NAMESPACE)
            .await?
            .into_iter()
            .filter(|entry| {
                entry.tenant_id == tenant.tenant_id().as_str()
                    && entry.status == Some(SubmissionStatus::InProgress)
            })
            .count() as u64)
    }

    async fn list_expired_submissions(
        &self,
        now: DateTime<Utc>,
        ttl: StdDuration,
        limit: u32,
    ) -> StorageResult<Vec<(TenantContext, SubmissionId)>> {
        if !self.supports_bulk_submit_worker() {
            return Ok(Vec::new());
        }
        let cutoff = now
            - chrono::Duration::from_std(ttl).unwrap_or_else(|_| chrono::Duration::seconds(86_400));

        let mut entries: Vec<SubmitIndexEntry> = self
            .list_index_entries(REGISTRY_NAMESPACE)
            .await?
            .into_iter()
            .filter(|entry| entry.updated_at.is_some_and(|updated| updated < cutoff))
            .collect();
        entries.sort_by_key(|entry| entry.updated_at);
        entries.truncate(limit as usize);

        Ok(entries
            .into_iter()
            .map(|entry| {
                (
                    tenant_from_id(&entry.tenant_id),
                    SubmissionId::new(entry.submitter, entry.submission_id),
                )
            })
            .collect())
    }

    async fn ensure_transaction_time(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<DateTime<Utc>> {
        let now = Utc::now();
        self.submission_cas(tenant, id, |state| {
            *state.transaction_time.get_or_insert(now)
        })
        .await
    }
}

/// Serialisable form of a [`SubmitFileRow`], which is a worker-layer type with
/// no `serde` impls of its own.
#[derive(serde::Serialize, serde::Deserialize)]
struct SubmitFileRowRecord {
    manifest_url: Option<String>,
    file_type: String,
    resource_type: Option<String>,
    part_index: u32,
    fencing_token: u64,
    file_path: String,
    line_count: u64,
    byte_count: u64,
    count_severity: Option<serde_json::Value>,
}

impl From<&SubmitFileRow> for SubmitFileRowRecord {
    fn from(row: &SubmitFileRow) -> Self {
        Self {
            manifest_url: row.manifest_url.clone(),
            file_type: row.file_type.clone(),
            resource_type: row.resource_type.clone(),
            part_index: row.part_index,
            fencing_token: row.fencing_token,
            file_path: row.file_path.clone(),
            line_count: row.line_count,
            byte_count: row.byte_count,
            count_severity: row.count_severity.clone(),
        }
    }
}

impl From<SubmitFileRowRecord> for SubmitFileRow {
    fn from(record: SubmitFileRowRecord) -> Self {
        Self {
            manifest_url: record.manifest_url,
            file_type: record.file_type,
            resource_type: record.resource_type,
            part_index: record.part_index,
            fencing_token: record.fencing_token,
            file_path: record.file_path,
            line_count: record.line_count,
            byte_count: record.byte_count,
            count_severity: record.count_severity,
        }
    }
}
