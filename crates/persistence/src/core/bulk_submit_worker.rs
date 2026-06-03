//! Worker-facing traits for asynchronous Bulk Data **Submit** processing.
//!
//! The synchronous ingestion engine ([`BulkSubmitProvider`] and friends) does the
//! actual create/update/delete work. This module adds the missing async layer that
//! the FHIR Bulk Data Submit operation needs: a Data Consumer accepts a submission,
//! then a background worker **claims a pending manifest under a heartbeated,
//! fencing-token-guarded lease**, fetches the referenced files, and ingests them —
//! mirroring the bulk-export worker design ([`crate::core::bulk_export_worker`]).
//!
//! [`BulkSubmitProvider`]: crate::core::bulk_submit::BulkSubmitProvider

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use helios_fhir::FhirVersion;
use serde_json::{Value, json};

use crate::core::bulk_export_output::{ExportOutputStore, ExportPartKey};
use crate::core::bulk_export_worker::{LeaseError, WorkerId};
use crate::core::bulk_submit::{
    BulkEntryOutcome, BulkProcessingOptions, BulkSubmitProvider, BulkSubmitRollbackProvider,
    StreamingBulkSubmitProvider, SubmissionId,
};
use crate::core::bulk_submit_input::{SubmitInputFetcher, submission_output_job_id};
use crate::error::StorageResult;
use crate::tenant::TenantContext;

/// A lease over a single pending manifest, held by exactly one worker at a time.
///
/// Leases expire; if the holding worker does not heartbeat before `lease_expiry`,
/// the manifest is reclaimable. The `fencing_token` is bumped on every claim so a
/// zombie worker cannot mutate a manifest another worker now owns.
#[derive(Debug, Clone)]
pub struct ManifestLease {
    /// The tenant the submission belongs to.
    pub tenant: TenantContext,
    /// The submission this manifest belongs to.
    pub submission_id: SubmissionId,
    /// The leased manifest's ID (unique within the submission).
    pub manifest_id: String,
    /// The worker holding the lease.
    pub worker_id: WorkerId,
    /// When the lease expires if not renewed.
    pub lease_expiry: DateTime<Utc>,
    /// Monotonically increasing token, bumped on every claim.
    pub fencing_token: u64,
}

/// The worker's view of a claimed manifest: everything needed to fetch + ingest it.
#[derive(Debug, Clone)]
pub struct ManifestWorkerView {
    /// The manifest ID.
    pub manifest_id: String,
    /// The remote Bulk Export Manifest URL to fetch (None means nothing to do).
    pub manifest_url: Option<String>,
    /// Base URL for resolving relative references in ingested resources.
    pub fhir_base_url: Option<String>,
    /// The kickoff `outputFormat` (MIME), used to derive the FHIR version.
    pub output_format: Option<String>,
    /// HTTP headers the Data Provider asked us to include when fetching files.
    pub file_request_headers: Vec<(String, String)>,
    /// OAuth 2.0 metadata endpoints for acquiring file-retrieval tokens.
    pub oauth_metadata_urls: Vec<String>,
    /// JWE file-encryption key descriptor, if the provider encrypts files.
    pub file_encryption_key: Option<Value>,
    /// Resume cursor: lines already processed for this manifest.
    pub last_processed_line: u64,
    /// FHIR version this submission ingests against.
    pub fhir_version: FhirVersion,
}

/// A finalized status-manifest artifact (output / error / deleted NDJSON part) to record.
#[derive(Debug, Clone)]
pub struct SubmitFileRecord {
    /// The submitted manifest this artifact relates to (for `output[].manifestUrl`).
    pub manifest_url: Option<String>,
    /// `output`, `error`, or `deleted`.
    pub file_type: String,
    /// FHIR resource type (for `output` entries).
    pub resource_type: Option<String>,
    /// 0-based part index within (submission, file_type, resource_type).
    pub part_index: u32,
    /// Encoded storage key / path in the output store.
    pub file_path: String,
    /// Number of NDJSON lines in the artifact.
    pub line_count: u64,
    /// Size of the artifact in bytes.
    pub byte_count: u64,
    /// `countSeverity` breakdown JSON (for `error` artifacts).
    pub count_severity: Option<Value>,
}

/// A persisted status-manifest artifact row, read back when building the status manifest.
#[derive(Debug, Clone)]
pub struct SubmitFileRow {
    /// The submitted manifest this artifact relates to.
    pub manifest_url: Option<String>,
    /// `output`, `error`, or `deleted`.
    pub file_type: String,
    /// FHIR resource type (for `output` entries).
    pub resource_type: Option<String>,
    /// 0-based part index.
    pub part_index: u32,
    /// Fencing token of the worker that wrote it.
    pub fencing_token: u64,
    /// Encoded storage key / path in the output store.
    pub file_path: String,
    /// Number of NDJSON lines in the artifact.
    pub line_count: u64,
    /// Size of the artifact in bytes.
    pub byte_count: u64,
    /// `countSeverity` breakdown JSON (for `error` artifacts).
    pub count_severity: Option<Value>,
}

/// Resolution of a poll token to its owning submission, for REST status/cancel/file auth.
#[derive(Debug, Clone)]
pub struct PollTokenTarget {
    /// The tenant the submission belongs to.
    pub tenant: TenantContext,
    /// The submission identified by the token.
    pub submission_id: SubmissionId,
    /// The OAuth subject that kicked off the submission (for ownership checks).
    pub owner_subject: Option<String>,
}

/// Strategy for atomically claiming the next available pending manifest.
///
/// Each backend reaches for its native primitive — `SELECT … FOR UPDATE SKIP LOCKED`
/// on Postgres, a process-local mutex on SQLite.
#[async_trait]
pub trait SubmitClaimStrategy: Send + Sync {
    /// Atomically transitions one eligible manifest (`pending`, or `processing` with
    /// an expired lease) to held-by-this-worker, bumping the fencing token. Returns
    /// `Ok(None)` when no manifest is available.
    async fn claim_next_manifest(
        &self,
        worker_id: &WorkerId,
        lease_duration: Duration,
    ) -> StorageResult<Option<ManifestLease>>;

    /// Renews a lease the worker still holds; returns the new expiry, or
    /// `LeaseError::LeaseLost` if the manifest was reclaimed.
    async fn heartbeat(&self, lease: &ManifestLease) -> Result<DateTime<Utc>, LeaseError>;

    /// Releases a lease early (graceful shutdown). Best-effort.
    async fn release(&self, lease: ManifestLease) -> StorageResult<()>;
}

/// Worker-owned mutations of manifest/submission state.
///
/// The fenced methods (those taking a [`ManifestLease`]) verify `worker_id` +
/// `fencing_token`: a guarded mutation affecting zero rows returns
/// `LeaseError::LeaseLost`, so a zombie worker cannot corrupt progress, file rows,
/// or terminal status after its manifest has been reclaimed.
#[async_trait]
pub trait SubmitWorkerStorage: Send + Sync {
    /// Loads the claimed manifest's fetch parameters and resume cursor. Fenced.
    async fn get_manifest_for_worker(
        &self,
        lease: &ManifestLease,
    ) -> Result<ManifestWorkerView, LeaseError>;

    /// Marks the manifest `processing`. Fenced.
    async fn mark_manifest_processing(&self, lease: &ManifestLease) -> Result<(), LeaseError>;

    /// Idempotent update of per-manifest progress (counts + resume cursor). Fenced.
    async fn update_manifest_progress(
        &self,
        lease: &ManifestLease,
        processed_entries: u64,
        failed_entries: u64,
        last_processed_line: u64,
    ) -> Result<(), LeaseError>;

    /// Idempotent upsert of a finalized status-manifest artifact row. Fenced.
    async fn record_submit_file(
        &self,
        lease: &ManifestLease,
        file: &SubmitFileRecord,
    ) -> Result<(), LeaseError>;

    /// Marks the manifest `completed`. Fenced.
    async fn finish_manifest(&self, lease: &ManifestLease) -> Result<(), LeaseError>;

    /// Marks the manifest `failed` with a message. Fenced.
    async fn fail_manifest(
        &self,
        lease: &ManifestLease,
        error_message: &str,
    ) -> Result<(), LeaseError>;

    // ---- REST-facing (unfenced) submission/poll-token/artifact lifecycle ----

    /// Persists the remote-fetch parameters for a manifest that the kickoff handler
    /// added via [`crate::core::bulk_submit::BulkSubmitProvider::add_manifest`].
    ///
    /// `file_request_headers` / `oauth_metadata_urls` are stored as JSON arrays;
    /// `file_encryption_key` as a JSON object.
    #[allow(clippy::too_many_arguments)]
    async fn set_manifest_fetch_params(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
        manifest_id: &str,
        fhir_base_url: Option<&str>,
        output_format: Option<&str>,
        file_request_headers: &[(String, String)],
        oauth_metadata_urls: &[String],
        file_encryption_key: Option<&Value>,
    ) -> StorageResult<()>;

    /// Marks a previously-submitted manifest (identified by its submitted
    /// `manifest_url`) as `replaced`, for `replacesManifestUrl` handling. Returns the
    /// `manifest_id`s that were superseded (so the caller can roll back their changes).
    async fn replace_manifest_by_url(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
        manifest_url: &str,
    ) -> StorageResult<Vec<String>>;

    /// Persists kickoff metadata (owner subject, request URL, access-token posture).
    async fn set_submission_kickoff_meta(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
        owner_subject: Option<&str>,
        request_url: &str,
        requires_access_token: bool,
    ) -> StorageResult<()>;

    /// Idempotently mints + stores a poll token on the submission (UNIQUE index
    /// guards collisions). Returns the existing token if one is already set.
    async fn ensure_poll_token(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<String>;

    /// Resolves a poll token to its submission, or `None` after deletion/unknown.
    async fn resolve_poll_token(&self, token: &str) -> StorageResult<Option<PollTokenTarget>>;

    /// Clears the poll token so subsequent [`Self::resolve_poll_token`] returns `None`.
    async fn clear_poll_token(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<()>;

    /// Lists the recorded status-manifest artifact rows for a submission.
    async fn list_submit_files(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<Vec<SubmitFileRow>>;

    /// Deletes the `bulk_submit_files` rows for a submission (idempotent).
    ///
    /// **Note:** the persistence layer holds no reference to the output store, so
    /// removing the underlying NDJSON objects is orchestrated by the caller (REST
    /// DELETE handler / TTL cleanup task), which lists files via
    /// [`Self::list_submit_files`], deletes them from the output store, then calls
    /// this to drop the rows.
    async fn delete_submission_artifacts(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<()>;

    /// Counts non-terminal submissions for a tenant (per-tenant concurrency cap).
    async fn count_active_submissions(&self, tenant: &TenantContext) -> StorageResult<u64>;

    /// Lists submissions whose `updated_at` is older than `now - ttl`, across all
    /// tenants, for the periodic cleanup task. Returns `(tenant, submission_id)`
    /// pairs (bounded by `limit`) so the caller can delete their output-store
    /// artifacts and rows.
    async fn list_expired_submissions(
        &self,
        now: DateTime<Utc>,
        ttl: Duration,
        limit: u32,
    ) -> StorageResult<Vec<(TenantContext, SubmissionId)>>;

    /// Records a transaction time on the submission when its status manifest is first
    /// finalized (idempotent — only sets if currently unset).
    async fn ensure_transaction_time(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<DateTime<Utc>>;
}

/// Marker trait composing the submit job-state surfaces a worker + REST layer needs.
///
/// Only the SQLite and Postgres backends implement this; it is held as an
/// `Arc<dyn BulkSubmitJobStore>` and selected at bootstrap by `HFS_BULK_SUBMIT_BACKEND`.
pub trait BulkSubmitJobStore:
    BulkSubmitProvider
    + StreamingBulkSubmitProvider
    + BulkSubmitRollbackProvider
    + SubmitWorkerStorage
    + SubmitClaimStrategy
{
}

impl<T> BulkSubmitJobStore for T where
    T: BulkSubmitProvider
        + StreamingBulkSubmitProvider
        + BulkSubmitRollbackProvider
        + SubmitWorkerStorage
        + SubmitClaimStrategy
{
}

/// The default in-process submit worker.
///
/// Binds a [`BulkSubmitJobStore`] (job state + claim + worker storage + ingestion
/// engine), a [`SubmitInputFetcher`] (remote manifest + NDJSON fetch), and an
/// [`ExportOutputStore`] (where status-manifest artifacts go), and drives a claimed
/// manifest to completion: fetch → ingest each `output` file via the existing
/// `process_ndjson_stream` engine → emit `output`/`error` artifacts → finish.
pub struct DefaultSubmitWorker<Js: ?Sized, Fetcher: ?Sized, Os: ?Sized> {
    jobs: Arc<Js>,
    fetcher: Arc<Fetcher>,
    output: Arc<Os>,
    #[allow(dead_code)]
    worker_id: WorkerId,
}

impl<Js, Fetcher, Os> DefaultSubmitWorker<Js, Fetcher, Os>
where
    Js: BulkSubmitJobStore + ?Sized,
    Fetcher: SubmitInputFetcher + ?Sized,
    Os: ExportOutputStore + ?Sized,
{
    /// Creates a new worker bound to the given job store, fetcher, and output store.
    pub fn new(jobs: Arc<Js>, fetcher: Arc<Fetcher>, output: Arc<Os>, worker_id: WorkerId) -> Self {
        Self {
            jobs,
            fetcher,
            output,
            worker_id,
        }
    }

    /// Drives a single claimed manifest to a terminal state.
    ///
    /// Returns `Ok(())` for both successful ingestion and *recorded* manifest-level
    /// failures (a bad remote manifest fails only that manifest, not the worker).
    /// Only `LeaseError::LeaseLost` aborts early; storage errors propagate.
    pub async fn run_job(&self, lease: ManifestLease) -> StorageResult<()> {
        let view = match self.jobs.get_manifest_for_worker(&lease).await {
            Ok(v) => v,
            Err(LeaseError::LeaseLost { .. }) => return Ok(()),
            Err(LeaseError::Storage(e)) => return Err(e),
        };
        if let Err(LeaseError::Storage(e)) = self.jobs.mark_manifest_processing(&lease).await {
            return Err(e);
        }

        let Some(manifest_url) = view.manifest_url.clone() else {
            // Nothing to fetch (status-only submission) — treat as done.
            let _ = self.jobs.finish_manifest(&lease).await;
            return Ok(());
        };

        // 1. Fetch the remote Bulk Export Manifest.
        let manifest = match self
            .fetcher
            .fetch_manifest(
                &manifest_url,
                &view.file_request_headers,
                &view.oauth_metadata_urls,
            )
            .await
        {
            Ok(m) => m,
            Err(e) => {
                self.record_manifest_error(
                    &lease,
                    &manifest_url,
                    &format!("failed to fetch manifest: {e}"),
                )
                .await?;
                let _ = self.jobs.fail_manifest(&lease, &e.to_string()).await;
                return Ok(());
            }
        };

        let opts = BulkProcessingOptions::new();
        let mut processed: u64 = 0;
        let mut failed: u64 = 0;

        // 2. Ingest each output file via the existing streaming engine.
        for file in &manifest.output {
            if let Err(LeaseError::LeaseLost { .. }) = self.jobs.heartbeat(&lease).await {
                return Ok(());
            }
            let resource_type = file
                .resource_type
                .clone()
                .unwrap_or_else(|| "Resource".into());
            let stream = match self
                .fetcher
                .open_file_stream(
                    &file.url,
                    &view.file_request_headers,
                    manifest.requires_access_token,
                    &view.oauth_metadata_urls,
                    view.file_encryption_key.as_ref(),
                )
                .await
            {
                Ok(s) => s,
                Err(e) => {
                    self.record_manifest_error(
                        &lease,
                        &manifest_url,
                        &format!("failed to fetch file {}: {e}", file.url),
                    )
                    .await?;
                    failed += 1;
                    continue;
                }
            };

            match self
                .jobs
                .process_ndjson_stream(
                    &lease.tenant,
                    &lease.submission_id,
                    &lease.manifest_id,
                    &resource_type,
                    stream,
                    &opts,
                )
                .await
            {
                Ok(result) => {
                    processed += result.counts.success + result.counts.skipped;
                    failed += result.counts.error_count();
                }
                Err(e) => {
                    self.record_manifest_error(
                        &lease,
                        &manifest_url,
                        &format!("failed to ingest file {}: {e}", file.url),
                    )
                    .await?;
                    failed += 1;
                }
            }

            if let Err(LeaseError::Storage(e)) = self
                .jobs
                .update_manifest_progress(&lease, processed, failed, processed + failed)
                .await
            {
                return Err(e);
            }
        }

        // 2b. Process `deleted` files — transaction Bundles / resource refs to remove.
        let mut deleted_refs: Vec<String> = Vec::new();
        for file in &manifest.deleted {
            if let Err(LeaseError::LeaseLost { .. }) = self.jobs.heartbeat(&lease).await {
                return Ok(());
            }
            match self
                .fetcher
                .open_file_stream(
                    &file.url,
                    &view.file_request_headers,
                    manifest.requires_access_token,
                    &view.oauth_metadata_urls,
                    view.file_encryption_key.as_ref(),
                )
                .await
            {
                Ok(reader) => {
                    self.process_deleted_stream(&lease, reader, &mut deleted_refs)
                        .await;
                }
                Err(e) => {
                    self.record_manifest_error(
                        &lease,
                        &manifest_url,
                        &format!("failed to fetch deleted file {}: {e}", file.url),
                    )
                    .await?;
                }
            }
        }
        if !deleted_refs.is_empty() {
            self.write_deleted_artifact(&lease, &manifest_url, &deleted_refs)
                .await?;
        }

        // 3. Emit per-type `output` receipts and an aggregated `error` artifact.
        self.write_result_artifacts(&lease, &manifest_url, view.fhir_version, failed)
            .await?;

        // 4. Mark the manifest complete.
        if let Err(LeaseError::Storage(e)) = self.jobs.finish_manifest(&lease).await {
            return Err(e);
        }
        Ok(())
    }

    /// Reads back this manifest's entry results and writes `output` receipts
    /// (grouped by resource type) plus a single aggregated `error` artifact.
    async fn write_result_artifacts(
        &self,
        lease: &ManifestLease,
        manifest_url: &str,
        _fhir_version: FhirVersion,
        failed_count: u64,
    ) -> StorageResult<()> {
        let job_id = submission_output_job_id(&lease.submission_id);
        let tenant_id = lease.tenant.tenant_id().as_str().to_string();

        // Page through all entry results for this manifest.
        let mut all = Vec::new();
        let limit = 1000u32;
        let mut offset = 0u32;
        loop {
            let batch = self
                .jobs
                .get_entry_results(
                    &lease.tenant,
                    &lease.submission_id,
                    &lease.manifest_id,
                    None,
                    limit,
                    offset,
                )
                .await?;
            let n = batch.len() as u32;
            all.extend(batch);
            if n < limit {
                break;
            }
            offset += limit;
        }

        // Partition successes (by type) and errors.
        let mut by_type: std::collections::BTreeMap<String, Vec<String>> =
            std::collections::BTreeMap::new();
        let mut error_lines: Vec<String> = Vec::new();
        let mut severity: std::collections::BTreeMap<String, u64> =
            std::collections::BTreeMap::new();

        for entry in &all {
            match entry.outcome {
                BulkEntryOutcome::Success => {
                    if let Some(id) = &entry.resource_id {
                        let line = json!({"reference": format!("{}/{}", entry.resource_type, id)})
                            .to_string();
                        by_type
                            .entry(entry.resource_type.clone())
                            .or_default()
                            .push(line);
                    }
                }
                BulkEntryOutcome::ValidationError | BulkEntryOutcome::ProcessingError => {
                    let oo = entry
                        .operation_outcome
                        .clone()
                        .unwrap_or_else(|| default_error_outcome(entry));
                    tally_severity(&oo, &mut severity);
                    error_lines.push(oo.to_string());
                }
                BulkEntryOutcome::Skipped => {}
            }
        }

        // The streaming engine counts parse / wrong-type failures but does not
        // persist them as per-line entry results. Surface any such uncaptured
        // failures as a summary OperationOutcome so the status manifest's `error`
        // array reflects them (partial success).
        let recorded_errors = error_lines.len() as u64;
        if failed_count > recorded_errors {
            let uncaptured = failed_count - recorded_errors;
            let oo = json!({
                "resourceType": "OperationOutcome",
                "issue": [{
                    "severity": "error",
                    "code": "processing",
                    "diagnostics": format!(
                        "{uncaptured} submitted resource(s) could not be parsed or did not \
                         match the declared resource type"
                    )
                }]
            });
            tally_severity(&oo, &mut severity);
            error_lines.push(oo.to_string());
        }

        // Write one `output` part per resource type.
        for (idx, (resource_type, lines)) in by_type.iter().enumerate() {
            let key = ExportPartKey::output(
                tenant_id.clone(),
                job_id.clone(),
                resource_type.clone(),
                idx as u32,
                lease.fencing_token,
            );
            let part = self.write_part(&key, lines).await?;
            self.jobs
                .record_submit_file(
                    lease,
                    &SubmitFileRecord {
                        manifest_url: Some(manifest_url.to_string()),
                        file_type: "output".to_string(),
                        resource_type: Some(resource_type.clone()),
                        part_index: idx as u32,
                        file_path: key.part_segment(),
                        line_count: part.0,
                        byte_count: part.1,
                        count_severity: None,
                    },
                )
                .await
                .map_err(lease_err_to_storage)?;
        }

        // Write a single aggregated `error` part (if any).
        if !error_lines.is_empty() {
            let key = ExportPartKey::error(
                tenant_id.clone(),
                job_id.clone(),
                "OperationOutcome",
                0,
                lease.fencing_token,
            );
            let part = self.write_part(&key, &error_lines).await?;
            let count_severity = Value::Object(
                severity
                    .into_iter()
                    .map(|(k, v)| (k, Value::from(v)))
                    .collect(),
            );
            self.jobs
                .record_submit_file(
                    lease,
                    &SubmitFileRecord {
                        manifest_url: Some(manifest_url.to_string()),
                        file_type: "error".to_string(),
                        resource_type: Some("OperationOutcome".to_string()),
                        part_index: 0,
                        file_path: key.part_segment(),
                        line_count: part.0,
                        byte_count: part.1,
                        count_severity: Some(count_severity),
                    },
                )
                .await
                .map_err(lease_err_to_storage)?;
        }
        Ok(())
    }

    /// Applies deletions from a `deleted` NDJSON stream (transaction Bundles or
    /// bare resources), collecting `Type/id` references actually removed.
    async fn process_deleted_stream(
        &self,
        lease: &ManifestLease,
        reader: Box<dyn tokio::io::AsyncBufRead + Send + Unpin>,
        refs: &mut Vec<String>,
    ) {
        use tokio::io::AsyncBufReadExt;
        let mut lines = reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Ok(val) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if val.get("resourceType").and_then(|v| v.as_str()) == Some("Bundle") {
                if let Some(entries) = val.get("entry").and_then(|e| e.as_array()) {
                    for e in entries {
                        if let Some(url) = e
                            .get("request")
                            .and_then(|r| r.get("url"))
                            .and_then(|u| u.as_str())
                        {
                            if let Some((ty, id)) = url.split_once('/') {
                                if self.jobs.delete(&lease.tenant, ty, id).await.is_ok() {
                                    refs.push(format!("{ty}/{id}"));
                                }
                            }
                        }
                    }
                }
            } else if let (Some(ty), Some(id)) = (
                val.get("resourceType").and_then(|v| v.as_str()),
                val.get("id").and_then(|v| v.as_str()),
            ) {
                if self.jobs.delete(&lease.tenant, ty, id).await.is_ok() {
                    refs.push(format!("{ty}/{id}"));
                }
            }
        }
    }

    /// Writes the `deleted` receipt artifact listing removed resource references.
    async fn write_deleted_artifact(
        &self,
        lease: &ManifestLease,
        manifest_url: &str,
        refs: &[String],
    ) -> StorageResult<()> {
        let job_id = submission_output_job_id(&lease.submission_id);
        let tenant_id = lease.tenant.tenant_id().as_str().to_string();
        let lines: Vec<String> = refs
            .iter()
            .map(|r| json!({ "reference": r }).to_string())
            .collect();
        let key = ExportPartKey {
            tenant_id,
            job_id,
            resource_type: "Bundle".to_string(),
            file_type: "deleted".to_string(),
            part_index: 0,
            fencing_token: lease.fencing_token,
        };
        let (line_count, byte_count) = self.write_part(&key, &lines).await?;
        self.jobs
            .record_submit_file(
                lease,
                &SubmitFileRecord {
                    manifest_url: Some(manifest_url.to_string()),
                    file_type: "deleted".to_string(),
                    resource_type: Some("Bundle".to_string()),
                    part_index: 0,
                    file_path: key.part_segment(),
                    line_count,
                    byte_count,
                    count_severity: None,
                },
            )
            .await
            .map_err(lease_err_to_storage)?;
        Ok(())
    }

    /// Records a single manifest-level `error` OperationOutcome artifact.
    async fn record_manifest_error(
        &self,
        lease: &ManifestLease,
        manifest_url: &str,
        message: &str,
    ) -> StorageResult<()> {
        let job_id = submission_output_job_id(&lease.submission_id);
        let tenant_id = lease.tenant.tenant_id().as_str().to_string();
        let oo = json!({
            "resourceType": "OperationOutcome",
            "issue": [{
                "severity": "error",
                "code": "processing",
                "diagnostics": message
            }]
        })
        .to_string();
        // Use a high part index space for manifest-level errors to avoid clashing
        // with per-result error parts (which use index 0).
        let key = ExportPartKey::error(
            tenant_id,
            job_id,
            "OperationOutcome",
            1_000_000 + (manifest_url.len() as u32 % 1000),
            lease.fencing_token,
        );
        let part = self.write_part(&key, std::slice::from_ref(&oo)).await?;
        self.jobs
            .record_submit_file(
                lease,
                &SubmitFileRecord {
                    manifest_url: Some(manifest_url.to_string()),
                    file_type: "error".to_string(),
                    resource_type: Some("OperationOutcome".to_string()),
                    part_index: key.part_index,
                    file_path: key.part_segment(),
                    line_count: part.0,
                    byte_count: part.1,
                    count_severity: Some(json!({"error": 1})),
                },
            )
            .await
            .map_err(lease_err_to_storage)?;
        Ok(())
    }

    /// Writes NDJSON `lines` to a new output-store part, returning `(line_count, byte_count)`.
    async fn write_part(&self, key: &ExportPartKey, lines: &[String]) -> StorageResult<(u64, u64)> {
        let mut writer = self.output.open_writer(key).await?;
        for line in lines {
            writer.write_line(line).await.map_err(|e| {
                crate::error::StorageError::Backend(crate::error::BackendError::Internal {
                    backend_name: "bulk-submit-output".to_string(),
                    message: format!("write artifact: {e}"),
                    source: None,
                })
            })?;
        }
        let finalized = self.output.finalize_part(key, writer).await?;
        Ok((finalized.line_count, finalized.size_bytes))
    }
}

/// Converts a fenced [`LeaseError`] into a plain storage error for non-fatal paths.
fn lease_err_to_storage(e: LeaseError) -> crate::error::StorageError {
    match e {
        LeaseError::Storage(s) => s,
        LeaseError::LeaseLost { job_id } => {
            crate::error::StorageError::Backend(crate::error::BackendError::Internal {
                backend_name: "bulk-submit".to_string(),
                message: format!("lease lost for {job_id}"),
                source: None,
            })
        }
    }
}

/// Builds a fallback OperationOutcome when an entry result lacks one.
fn default_error_outcome(entry: &crate::core::bulk_submit::BulkEntryResult) -> Value {
    json!({
        "resourceType": "OperationOutcome",
        "issue": [{
            "severity": "error",
            "code": "processing",
            "diagnostics": format!(
                "{} error on {} line {}",
                entry.outcome, entry.resource_type, entry.line_number
            )
        }]
    })
}

/// Tallies issue severities from an OperationOutcome into `acc` (for `countSeverity`).
fn tally_severity(oo: &Value, acc: &mut std::collections::BTreeMap<String, u64>) {
    if let Some(issues) = oo.get("issue").and_then(|v| v.as_array()) {
        for issue in issues {
            let sev = issue
                .get("severity")
                .and_then(|v| v.as_str())
                .unwrap_or("error");
            *acc.entry(sev.to_string()).or_insert(0) += 1;
        }
    } else {
        *acc.entry("error".to_string()).or_insert(0) += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::local_fs::LocalFsOutputStore;
    use crate::backends::sqlite::SqliteBackend;
    use crate::core::bulk_submit::BulkSubmitProvider;
    use crate::core::bulk_submit_input::{RemoteFile, RemoteManifest};
    use crate::tenant::{TenantContext, TenantId, TenantPermissions};
    use std::time::Duration as StdDuration;

    /// A fetcher that returns a fixed manifest and serves NDJSON from memory.
    struct MockFetcher {
        files: std::collections::HashMap<String, Vec<u8>>,
        manifest: RemoteManifest,
    }

    #[async_trait]
    impl SubmitInputFetcher for MockFetcher {
        async fn fetch_manifest(
            &self,
            _url: &str,
            _headers: &[(String, String)],
            _oauth: &[String],
        ) -> StorageResult<RemoteManifest> {
            Ok(self.manifest.clone())
        }

        async fn open_file_stream(
            &self,
            url: &str,
            _headers: &[(String, String)],
            _requires_access_token: bool,
            _oauth: &[String],
            _encryption_key: Option<&Value>,
        ) -> StorageResult<Box<dyn tokio::io::AsyncBufRead + Send + Unpin>> {
            let data = self.files.get(url).cloned().unwrap_or_default();
            Ok(Box::new(tokio::io::BufReader::new(std::io::Cursor::new(
                data,
            ))))
        }
    }

    fn tenant() -> TenantContext {
        TenantContext::new(TenantId::new("t1"), TenantPermissions::full_access())
    }

    #[tokio::test]
    async fn test_worker_ingests_output_and_records_artifacts() {
        let backend = Arc::new(SqliteBackend::in_memory().unwrap());
        backend.init_schema().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let output = Arc::new(LocalFsOutputStore::new(
            tmp.path().to_path_buf(),
            "http://localhost:8080",
        ));

        let tenant = tenant();
        let sub_id = SubmissionId::generate("mock-system");
        backend
            .create_submission(&tenant, &sub_id, None)
            .await
            .unwrap();
        backend
            .add_manifest(
                &tenant,
                &sub_id,
                Some("http://provider/manifest.json"),
                None,
            )
            .await
            .unwrap();

        let ndjson = concat!(
            "{\"resourceType\":\"Patient\",\"id\":\"p1\",\"name\":[{\"family\":\"A\"}]}\n",
            "{\"resourceType\":\"Patient\",\"name\":[{\"family\":\"B\"}]}\n"
        );
        let mut files = std::collections::HashMap::new();
        files.insert(
            "http://provider/patient.ndjson".to_string(),
            ndjson.as_bytes().to_vec(),
        );
        let fetcher = Arc::new(MockFetcher {
            files,
            manifest: RemoteManifest {
                requires_access_token: false,
                output: vec![RemoteFile {
                    resource_type: Some("Patient".to_string()),
                    url: "http://provider/patient.ndjson".to_string(),
                    count: Some(2),
                }],
                deleted: vec![],
            },
        });

        let worker = DefaultSubmitWorker::new(
            backend.clone(),
            fetcher,
            output,
            WorkerId::new("test-worker"),
        );

        let lease = backend
            .claim_next_manifest(&WorkerId::new("test-worker"), StdDuration::from_secs(60))
            .await
            .unwrap()
            .expect("claimable manifest");
        worker.run_job(lease).await.unwrap();

        let manifests = backend.list_manifests(&tenant, &sub_id).await.unwrap();
        let manifest_id = manifests[0].manifest_id.clone();

        // Both Patients ingested (one with id, one assigned a new id).
        let counts = backend
            .get_entry_counts(&tenant, &sub_id, &manifest_id)
            .await
            .unwrap();
        assert_eq!(counts.success, 2);

        // Manifest marked completed.
        assert_eq!(
            manifests[0].status,
            crate::core::bulk_submit::ManifestStatus::Completed
        );

        // An `output` artifact for Patient was recorded.
        let files = backend.list_submit_files(&tenant, &sub_id).await.unwrap();
        assert!(
            files
                .iter()
                .any(|f| f.file_type == "output" && f.resource_type.as_deref() == Some("Patient"))
        );
    }

    /// A fetcher whose `fetch_manifest` always fails (unreachable / bad manifest).
    struct FailingManifestFetcher;
    #[async_trait]
    impl SubmitInputFetcher for FailingManifestFetcher {
        async fn fetch_manifest(
            &self,
            _url: &str,
            _h: &[(String, String)],
            _o: &[String],
        ) -> StorageResult<RemoteManifest> {
            Err(crate::error::StorageError::Backend(
                crate::error::BackendError::Internal {
                    backend_name: "test".into(),
                    message: "unreachable manifest".into(),
                    source: None,
                },
            ))
        }
        async fn open_file_stream(
            &self,
            _url: &str,
            _h: &[(String, String)],
            _r: bool,
            _o: &[String],
            _k: Option<&Value>,
        ) -> StorageResult<Box<dyn tokio::io::AsyncBufRead + Send + Unpin>> {
            Ok(Box::new(tokio::io::BufReader::new(std::io::Cursor::new(
                Vec::new(),
            ))))
        }
    }

    async fn seed(backend: &Arc<SqliteBackend>, tenant: &TenantContext) -> SubmissionId {
        let sub_id = SubmissionId::generate("mock-system");
        backend
            .create_submission(tenant, &sub_id, None)
            .await
            .unwrap();
        backend
            .add_manifest(tenant, &sub_id, Some("http://provider/manifest.json"), None)
            .await
            .unwrap();
        sub_id
    }

    #[tokio::test]
    async fn test_worker_fails_manifest_on_fetch_error() {
        let backend = Arc::new(SqliteBackend::in_memory().unwrap());
        backend.init_schema().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let output = Arc::new(LocalFsOutputStore::new(
            tmp.path().to_path_buf(),
            "http://x",
        ));
        let tenant = tenant();
        let sub_id = seed(&backend, &tenant).await;

        let worker = DefaultSubmitWorker::new(
            backend.clone(),
            Arc::new(FailingManifestFetcher),
            output,
            WorkerId::new("w"),
        );
        let lease = backend
            .claim_next_manifest(&WorkerId::new("w"), StdDuration::from_secs(60))
            .await
            .unwrap()
            .unwrap();
        // A bad manifest fails only that manifest (worker returns Ok).
        worker.run_job(lease).await.unwrap();

        let manifests = backend.list_manifests(&tenant, &sub_id).await.unwrap();
        assert_eq!(
            manifests[0].status,
            crate::core::bulk_submit::ManifestStatus::Failed
        );
        // A manifest-level error artifact was recorded.
        let files = backend.list_submit_files(&tenant, &sub_id).await.unwrap();
        assert!(files.iter().any(|f| f.file_type == "error"));
    }

    #[tokio::test]
    async fn test_worker_partial_success_on_invalid_ndjson() {
        let backend = Arc::new(SqliteBackend::in_memory().unwrap());
        backend.init_schema().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let output = Arc::new(LocalFsOutputStore::new(
            tmp.path().to_path_buf(),
            "http://x",
        ));
        let tenant = tenant();
        let sub_id = seed(&backend, &tenant).await;

        // One valid Patient, one malformed JSON line.
        let ndjson = "{\"resourceType\":\"Patient\",\"id\":\"ok\"}\nnot-json\n";
        let mut files = std::collections::HashMap::new();
        files.insert(
            "http://provider/p.ndjson".to_string(),
            ndjson.as_bytes().to_vec(),
        );
        let fetcher = Arc::new(MockFetcher {
            files,
            manifest: RemoteManifest {
                requires_access_token: false,
                output: vec![RemoteFile {
                    resource_type: Some("Patient".to_string()),
                    url: "http://provider/p.ndjson".to_string(),
                    count: Some(2),
                }],
                deleted: vec![],
            },
        });
        let worker = DefaultSubmitWorker::new(backend.clone(), fetcher, output, WorkerId::new("w"));
        let lease = backend
            .claim_next_manifest(&WorkerId::new("w"), StdDuration::from_secs(60))
            .await
            .unwrap()
            .unwrap();
        worker.run_job(lease).await.unwrap();

        let manifests = backend.list_manifests(&tenant, &sub_id).await.unwrap();
        let counts = backend
            .get_entry_counts(&tenant, &sub_id, &manifests[0].manifest_id)
            .await
            .unwrap();
        // Partial success: one ingested; the malformed line is counted as a
        // failure on the manifest and surfaced as a summary error artifact, and
        // the manifest still completes.
        assert_eq!(counts.success, 1);
        assert!(manifests[0].failed_entries >= 1);
        assert_eq!(
            manifests[0].status,
            crate::core::bulk_submit::ManifestStatus::Completed
        );
        let files = backend.list_submit_files(&tenant, &sub_id).await.unwrap();
        assert!(files.iter().any(|f| f.file_type == "error"));
        assert!(files.iter().any(|f| f.file_type == "output"));
    }
}
