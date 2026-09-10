//! Bulk submit implementation for SQLite backend.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use helios_fhir::FhirVersion;
use rusqlite::{TransactionBehavior, params};
use serde_json::Value;
use std::time::Duration as StdDuration;
use tokio::io::{AsyncBufRead, AsyncBufReadExt};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::core::ResourceStorage;
use crate::core::bulk_export::ExportJobId;
use crate::core::bulk_export_worker::{LeaseError, WorkerId};
use crate::core::bulk_submit::{
    BulkEntryOutcome, BulkEntryResult, BulkProcessingOptions, BulkSubmitProvider,
    BulkSubmitRollbackProvider, CANCELLED_ABORT_REASON, ChangeType, EntryCountSummary,
    EntryResultContinuation, EntryResultCursor, EntryResultPage, ManifestPhase, ManifestStatus,
    NdjsonEntry, PagedEntryResult, StreamProcessingResult, StreamingBulkSubmitProvider,
    SubmissionChange, SubmissionId, SubmissionManifest, SubmissionStatus, SubmissionSummary,
    UnindexedEntry, invalid_entry_result_page,
};
use crate::core::bulk_submit_publication::{
    ManifestPublicationResult, ManifestPublicationStatus, canonical_publication_files,
};
use crate::core::bulk_submit_worker::{
    ManifestFetchParams, ManifestLease, ManifestWorkerView, PollTokenTarget, SubmitClaimStrategy,
    SubmitFileRecord, SubmitFileRow, SubmitWorkerStorage,
};
use crate::error::{
    BackendError, BulkSubmitError, StorageError, StorageResult, classify_sqlite_error,
};
use crate::tenant::{TenantContext, TenantId, TenantPermissions};

use super::SqliteBackend;

/// Process-local lock serializing manifest claims for the single-instance SQLite
/// job store (SQLite has no `SELECT … FOR UPDATE SKIP LOCKED`).
static SUBMIT_CLAIM_LOCK: Mutex<()> = Mutex::const_new(());

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

fn internal_error(message: String) -> StorageError {
    StorageError::Backend(BackendError::Internal {
        backend_name: "sqlite".to_string(),
        message,
        source: None,
    })
}

/// Retry budget for a write guarded by `lease`.
///
/// Half the lease so the retries always end while the lease is still ours:
/// each attempt can itself block for up to the connection's `busy_timeout`,
/// so a budget counted in *attempts* rather than wall-clock time could run
/// well past the lease, let it lapse, and let a second worker claim the
/// manifest under us (#942).
fn lease_retry_budget(lease: &ManifestLease) -> StdDuration {
    lease.lease_duration / 2
}

/// Bounded retry for the manifest bookkeeping writes — the `bulk_manifests`
/// lease, progress and lifecycle `UPDATE`s, and the `bulk_submit_files`
/// insert — each of which runs outside the ingest batch's own transaction.
///
/// Every ingest batch holds SQLite's single write lock for its whole
/// extraction + insert span, so a bookkeeping write queued behind such a hold
/// can outlast `busy_timeout` and fail with `SQLITE_BUSY` even though the
/// database is healthy. Aborting the whole manifest over one contended
/// bookkeeping write is disproportionate (#942): these writes are safe to
/// reissue — an attempt that fails busy/locked never acquired the write lock,
/// so the statement was rolled back and changed nothing — and every one of
/// them is guarded by the lease's `worker_id`/`fencing_token`. Each attempt
/// checks out a fresh pooled connection so no pool slot is held across the
/// backoff sleep.
///
/// `budget` bounds the *elapsed time* spent retrying, not the number of
/// attempts, because a single attempt can block for a whole `busy_timeout`.
/// The first attempt always runs, so a zero budget still issues the write
/// once. Every caller holds a lease and so passes [`lease_retry_budget`].
///
/// Only busy/locked — classified as [`BackendError::Unavailable`] by
/// [`classify_sqlite_error`] — is retried; every other error surfaces
/// immediately. Call sites that report [`LeaseError`] must therefore classify
/// with [`classify_sqlite_error`] *before* wrapping, or the busy never
/// reaches this loop.
///
/// # Write paths deliberately left outside this helper
///
/// Two classes of write still abort on a busy, because retrying them is not
/// obviously safe and needs its own design rather than being folded in here:
///
/// * The manifest **claim** (`claim_next_manifest`): a `SELECT` followed by an
///   `UPDATE` that are not one atomic statement, and the `UPDATE` matches on
///   the *old* fencing token, so a retry has to re-read the row rather than
///   reissue the same statement. A lost claim is also self-healing — the
///   manifest stays `pending` and the next poll picks it up.
/// * The **per-entry** `bulk_submit_entries` inserts inside a batch
///   transaction, and the manifest counter/`updated_at` writes that ride that
///   same transaction. There the busy can surface on `COMMIT`, which leaves
///   the transaction open, so a retry must roll back and replay the whole
///   batch, not just the failing statement.
async fn retry_bookkeeping_on_busy<T>(
    what: &str,
    budget: StdDuration,
    mut attempt: impl FnMut() -> StorageResult<T>,
) -> StorageResult<T> {
    // Tokio's clock, not `std`'s, so the budget can be exercised under
    // `tokio::time::pause()`. Identical behaviour on a live runtime.
    let started = tokio::time::Instant::now();
    let mut backoff = StdDuration::from_millis(50);
    let mut attempt_no = 1u32;
    loop {
        match attempt() {
            Err(StorageError::Backend(BackendError::Unavailable { message, .. }))
                if started.elapsed() + backoff < budget =>
            {
                tracing::warn!(
                    attempt = attempt_no,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    budget_ms = budget.as_millis() as u64,
                    backoff_ms = backoff.as_millis() as u64,
                    "sqlite busy during {what}; retrying: {message}"
                );
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(StdDuration::from_secs(1));
                attempt_no += 1;
            }
            other => return other,
        }
    }
}

#[async_trait]
impl BulkSubmitProvider for SqliteBackend {
    async fn create_submission(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
        metadata: Option<Value>,
    ) -> StorageResult<SubmissionSummary> {
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        // Check for duplicate
        let exists: bool = conn
            .query_row(
                "SELECT 1 FROM bulk_submissions
                 WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3",
                params![tenant_id, &id.submitter, &id.submission_id],
                |_| Ok(true),
            )
            .unwrap_or(false);

        if exists {
            return Err(StorageError::BulkSubmit(
                BulkSubmitError::DuplicateSubmission {
                    submitter: id.submitter.clone(),
                    submission_id: id.submission_id.clone(),
                },
            ));
        }

        let now = Utc::now();
        let now_str = now.to_rfc3339();
        let metadata_bytes = metadata.as_ref().and_then(|m| serde_json::to_vec(m).ok());

        conn.execute(
            "INSERT INTO bulk_submissions
             (tenant_id, submitter, submission_id, status, created_at, updated_at, metadata)
             VALUES (?1, ?2, ?3, 'in-progress', ?4, ?5, ?6)",
            params![
                tenant_id,
                &id.submitter,
                &id.submission_id,
                now_str,
                now_str,
                metadata_bytes
            ],
        )
        .map_err(|e| internal_error(format!("Failed to create submission: {}", e)))?;

        Ok(SubmissionSummary {
            id: id.clone(),
            status: SubmissionStatus::InProgress,
            created_at: now,
            updated_at: now,
            completed_at: None,
            manifest_count: 0,
            total_entries: 0,
            success_count: 0,
            error_count: 0,
            skipped_count: 0,
            metadata,
        })
    }

    async fn get_submission(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<Option<SubmissionSummary>> {
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        let result = conn.query_row(
            "SELECT status, created_at, updated_at, completed_at, metadata
             FROM bulk_submissions
             WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3",
            params![tenant_id, &id.submitter, &id.submission_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<Vec<u8>>>(4)?,
                ))
            },
        );

        match result {
            Ok((status_str, created_at, updated_at, completed_at, metadata_bytes)) => {
                let status: SubmissionStatus = status_str
                    .parse()
                    .map_err(|_| internal_error(format!("Invalid status: {}", status_str)))?;

                let created_at = chrono::DateTime::parse_from_rfc3339(&created_at)
                    .map_err(|e| internal_error(format!("Invalid created_at: {}", e)))?
                    .with_timezone(&Utc);

                let updated_at = chrono::DateTime::parse_from_rfc3339(&updated_at)
                    .map_err(|e| internal_error(format!("Invalid updated_at: {}", e)))?
                    .with_timezone(&Utc);

                let completed_at = completed_at.and_then(|s| {
                    chrono::DateTime::parse_from_rfc3339(&s)
                        .ok()
                        .map(|dt| dt.with_timezone(&Utc))
                });

                let metadata = metadata_bytes.and_then(|b| serde_json::from_slice(&b).ok());

                // Get manifest count
                let manifest_count: i32 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM bulk_manifests
                         WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3",
                        params![tenant_id, &id.submitter, &id.submission_id],
                        |row| row.get(0),
                    )
                    .unwrap_or(0);

                // Get aggregated counts from entry results
                let (total, success, errors, skipped): (i64, i64, i64, i64) = conn
                    .query_row(
                        "SELECT
                            COUNT(*),
                            SUM(CASE WHEN outcome = 'success' THEN 1 ELSE 0 END),
                            SUM(CASE WHEN outcome IN ('validation-error', 'processing-error') THEN 1 ELSE 0 END),
                            SUM(CASE WHEN outcome = 'skipped' THEN 1 ELSE 0 END)
                         FROM bulk_entry_results
                         WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3",
                        params![tenant_id, &id.submitter, &id.submission_id],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                    )
                    .unwrap_or((0, 0, 0, 0));

                Ok(Some(SubmissionSummary {
                    id: id.clone(),
                    status,
                    created_at,
                    updated_at,
                    completed_at,
                    manifest_count: manifest_count as u32,
                    total_entries: total as u64,
                    success_count: success as u64,
                    error_count: errors as u64,
                    skipped_count: skipped as u64,
                    metadata,
                }))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(internal_error(format!("Failed to get submission: {}", e))),
        }
    }

    async fn list_submissions(
        &self,
        tenant: &TenantContext,
        submitter: Option<&str>,
        status: Option<SubmissionStatus>,
        limit: u32,
        offset: u32,
    ) -> StorageResult<Vec<SubmissionSummary>> {
        // Collect IDs first, then drop the connection before calling async methods
        let ids: Vec<(String, String)> = {
            let conn = self.get_connection()?;
            let tenant_id = tenant.tenant_id().as_str();

            let (query, params): (String, Vec<String>) = {
                let mut query =
                    "SELECT submitter, submission_id FROM bulk_submissions WHERE tenant_id = ?1"
                        .to_string();
                let mut params = vec![tenant_id.to_string()];

                if let Some(submitter) = submitter {
                    query.push_str(" AND submitter = ?2");
                    params.push(submitter.to_string());
                }

                if let Some(status) = status {
                    let param_num = params.len() + 1;
                    query.push_str(&format!(" AND status = ?{}", param_num));
                    params.push(status.to_string());
                }

                query.push_str(" ORDER BY created_at DESC");
                query.push_str(&format!(" LIMIT {} OFFSET {}", limit, offset));

                (query, params)
            };

            let mut stmt = conn
                .prepare(&query)
                .map_err(|e| internal_error(format!("Failed to prepare list query: {}", e)))?;

            let params_refs: Vec<&dyn rusqlite::ToSql> =
                params.iter().map(|s| s as &dyn rusqlite::ToSql).collect();

            stmt.query_map(params_refs.as_slice(), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| internal_error(format!("Failed to query submissions: {}", e)))?
            .filter_map(|r| r.ok())
            .collect()
        };

        let mut results = Vec::new();
        for (submitter, submission_id) in ids {
            let sub_id = SubmissionId::new(submitter, submission_id);
            if let Some(summary) = self.get_submission(tenant, &sub_id).await? {
                results.push(summary);
            }
        }

        Ok(results)
    }

    async fn complete_submission(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<SubmissionSummary> {
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        // Check current status
        let current_status: String = conn
            .query_row(
                "SELECT status FROM bulk_submissions
                 WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3",
                params![tenant_id, &id.submitter, &id.submission_id],
                |row| row.get(0),
            )
            .map_err(|e| {
                if matches!(e, rusqlite::Error::QueryReturnedNoRows) {
                    StorageError::BulkSubmit(BulkSubmitError::SubmissionNotFound {
                        submitter: id.submitter.clone(),
                        submission_id: id.submission_id.clone(),
                    })
                } else {
                    internal_error(format!("Failed to get submission status: {}", e))
                }
            })?;

        if current_status != "in-progress" {
            return Err(StorageError::BulkSubmit(BulkSubmitError::AlreadyComplete {
                submission_id: id.submission_id.clone(),
            }));
        }

        let now = Utc::now().to_rfc3339();
        conn.execute(
            "UPDATE bulk_submissions SET status = 'complete', completed_at = ?1, updated_at = ?2
             WHERE tenant_id = ?3 AND submitter = ?4 AND submission_id = ?5",
            params![now, now, tenant_id, &id.submitter, &id.submission_id],
        )
        .map_err(|e| internal_error(format!("Failed to complete submission: {}", e)))?;

        self.get_submission(tenant, id)
            .await?
            .ok_or_else(|| internal_error("Submission disappeared".to_string()))
    }

    async fn abort_submission(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
        _reason: &str,
    ) -> StorageResult<u64> {
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        // Check current status
        let current_status: String = conn
            .query_row(
                "SELECT status FROM bulk_submissions
                 WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3",
                params![tenant_id, &id.submitter, &id.submission_id],
                |row| row.get(0),
            )
            .map_err(|e| {
                if matches!(e, rusqlite::Error::QueryReturnedNoRows) {
                    StorageError::BulkSubmit(BulkSubmitError::SubmissionNotFound {
                        submitter: id.submitter.clone(),
                        submission_id: id.submission_id.clone(),
                    })
                } else {
                    internal_error(format!("Failed to get submission status: {}", e))
                }
            })?;

        if current_status != "in-progress" {
            return Err(StorageError::BulkSubmit(BulkSubmitError::AlreadyComplete {
                submission_id: id.submission_id.clone(),
            }));
        }

        // Count pending manifests
        let pending_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM bulk_manifests
                 WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3
                 AND status IN ('pending', 'processing')",
                params![tenant_id, &id.submitter, &id.submission_id],
                |row| row.get(0),
            )
            .unwrap_or(0);

        let now = Utc::now().to_rfc3339();

        // Update submission status
        conn.execute(
            "UPDATE bulk_submissions SET status = 'aborted', completed_at = ?1, updated_at = ?2
             WHERE tenant_id = ?3 AND submitter = ?4 AND submission_id = ?5",
            params![now, now, tenant_id, &id.submitter, &id.submission_id],
        )
        .map_err(|e| internal_error(format!("Failed to abort submission: {}", e)))?;

        // Update pending manifests to failed
        conn.execute(
            "UPDATE bulk_manifests SET status = 'failed'
             WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3
             AND status IN ('pending', 'processing')",
            params![tenant_id, &id.submitter, &id.submission_id],
        )
        .map_err(|e| internal_error(format!("Failed to update manifests: {}", e)))?;

        Ok(pending_count as u64)
    }

    async fn add_manifest(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_url: Option<&str>,
        replaces_manifest_url: Option<&str>,
    ) -> StorageResult<SubmissionManifest> {
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        // Check submission exists and is in progress
        let status: String = conn
            .query_row(
                "SELECT status FROM bulk_submissions
                 WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3",
                params![
                    tenant_id,
                    &submission_id.submitter,
                    &submission_id.submission_id
                ],
                |row| row.get(0),
            )
            .map_err(|e| {
                if matches!(e, rusqlite::Error::QueryReturnedNoRows) {
                    StorageError::BulkSubmit(BulkSubmitError::SubmissionNotFound {
                        submitter: submission_id.submitter.clone(),
                        submission_id: submission_id.submission_id.clone(),
                    })
                } else {
                    internal_error(format!("Failed to get submission: {}", e))
                }
            })?;

        if status != "in-progress" {
            return Err(StorageError::BulkSubmit(BulkSubmitError::InvalidState {
                submission_id: submission_id.submission_id.clone(),
                expected: "in-progress".to_string(),
                actual: status,
            }));
        }

        let manifest_id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let now_str = now.to_rfc3339();

        conn.execute(
            "INSERT INTO bulk_manifests
             (tenant_id, submitter, submission_id, manifest_id, manifest_url, replaces_manifest_url, status, added_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', ?7)",
            params![
                tenant_id,
                &submission_id.submitter,
                &submission_id.submission_id,
                manifest_id,
                manifest_url,
                replaces_manifest_url,
                now_str
            ],
        )
        .map_err(|e| internal_error(format!("Failed to add manifest: {}", e)))?;

        // Update submission updated_at
        conn.execute(
            "UPDATE bulk_submissions SET updated_at = ?1
             WHERE tenant_id = ?2 AND submitter = ?3 AND submission_id = ?4",
            params![
                now_str,
                tenant_id,
                &submission_id.submitter,
                &submission_id.submission_id
            ],
        )
        .map_err(|e| internal_error(format!("Failed to update submission: {}", e)))?;

        Ok(SubmissionManifest {
            manifest_id,
            manifest_url: manifest_url.map(String::from),
            replaces_manifest_url: replaces_manifest_url.map(String::from),
            status: ManifestStatus::Pending,
            added_at: now,
            total_entries: 0,
            processed_entries: 0,
            failed_entries: 0,
            lease_expiry: None,
            bytes_processed: 0,
            bytes_total: 0,
            phase: None,
            files_done: 0,
            files_total: 0,
        })
    }

    async fn get_manifest(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_id: &str,
    ) -> StorageResult<Option<SubmissionManifest>> {
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        let result = conn.query_row(
            "SELECT manifest_url, replaces_manifest_url, status, added_at, total_entries, processed_entries, failed_entries, lease_expiry, bytes_processed, bytes_total, phase, files_done, files_total
             FROM bulk_manifests
             WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3 AND manifest_id = ?4",
            params![tenant_id, &submission_id.submitter, &submission_id.submission_id, manifest_id],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, Option<String>>(10)?,
                    row.get::<_, i64>(11)?,
                    row.get::<_, i64>(12)?,
                ))
            },
        );

        match result {
            Ok((
                manifest_url,
                replaces_manifest_url,
                status_str,
                added_at,
                total,
                processed,
                failed,
                lease_expiry,
                bytes_processed,
                bytes_total,
                phase,
                files_done,
                files_total,
            )) => {
                let status: ManifestStatus = status_str.parse().map_err(|_| {
                    internal_error(format!("Invalid manifest status: {}", status_str))
                })?;

                let added_at = chrono::DateTime::parse_from_rfc3339(&added_at)
                    .map_err(|e| internal_error(format!("Invalid added_at: {}", e)))?
                    .with_timezone(&Utc);

                Ok(Some(SubmissionManifest {
                    manifest_id: manifest_id.to_string(),
                    manifest_url,
                    replaces_manifest_url,
                    status,
                    added_at,
                    total_entries: total as u64,
                    processed_entries: processed as u64,
                    failed_entries: failed as u64,
                    bytes_processed: bytes_processed.max(0) as u64,
                    bytes_total: bytes_total.max(0) as u64,
                    lease_expiry: lease_expiry.and_then(|s| {
                        chrono::DateTime::parse_from_rfc3339(&s)
                            .ok()
                            .map(|d| d.with_timezone(&Utc))
                    }),
                    // Cosmetic hint only: an unrecognized value (a newer worker,
                    // or a hand-edited row) degrades to "no phase", never an error.
                    phase: phase.and_then(|s| s.parse::<ManifestPhase>().ok()),
                    files_done: files_done.max(0) as u64,
                    files_total: files_total.max(0) as u64,
                }))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(internal_error(format!("Failed to get manifest: {}", e))),
        }
    }

    async fn list_manifests(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
    ) -> StorageResult<Vec<SubmissionManifest>> {
        // Collect IDs first, then drop the connection before calling async methods
        let manifest_ids: Vec<String> = {
            let conn = self.get_connection()?;
            let tenant_id = tenant.tenant_id().as_str();

            let mut stmt = conn
                .prepare(
                    "SELECT manifest_id FROM bulk_manifests
                     WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3
                     ORDER BY added_at",
                )
                .map_err(|e| internal_error(format!("Failed to prepare list query: {}", e)))?;

            stmt.query_map(
                params![
                    tenant_id,
                    &submission_id.submitter,
                    &submission_id.submission_id
                ],
                |row| row.get(0),
            )
            .map_err(|e| internal_error(format!("Failed to query manifests: {}", e)))?
            .filter_map(|r| r.ok())
            .collect()
        };

        let mut results = Vec::new();
        for manifest_id in manifest_ids {
            if let Some(manifest) = self
                .get_manifest(tenant, submission_id, &manifest_id)
                .await?
            {
                results.push(manifest);
            }
        }

        Ok(results)
    }

    async fn process_entries(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_id: &str,
        entries: Vec<NdjsonEntry>,
        options: &BulkProcessingOptions,
    ) -> StorageResult<Vec<BulkEntryResult>> {
        let tenant_id = tenant.tenant_id().as_str();
        let batch_prologue = crate::perf::span(crate::perf::Phase::BatchOverhead);

        // Verify manifest exists
        if self
            .get_manifest(tenant, submission_id, manifest_id)
            .await?
            .is_none()
        {
            return Err(StorageError::BulkSubmit(
                BulkSubmitError::ManifestNotFound {
                    submission_id: submission_id.submission_id.clone(),
                    manifest_id: manifest_id.to_string(),
                },
            ));
        }

        let mut results = Vec::new();
        let mut error_count = 0u32;
        let mut aborted_on_max_errors = false;
        let file_url = options.file_url.as_deref().unwrap_or("");

        // One IMMEDIATE transaction per batch (#813): entry rows, history,
        // search-index writes, rollback records, and per-line receipts all
        // land on a single fsync instead of four-plus autocommits per
        // resource — and because they commit together, the rollback log can
        // never diverge from what was actually written (#815 review). The
        // connection is held across sync statement runs only — batch inputs
        // arrived as a Vec — so #646/#711's held-across-awaits deadlock
        // stays out.
        let mut txn = <Self as crate::core::TransactionProvider>::begin_transaction(
            self,
            tenant,
            crate::core::TransactionOptions {
                fhir_version: Some(FhirVersion::default_enabled()),
                defer_search_indexing: options.defer_indexing,
                ..Default::default()
            },
        )
        .await?;

        // Manifest status, on the batch transaction's own connection. It used
        // to be a standalone autocommit statement before the transaction
        // opened: a second write-lock acquisition and a second fsync per
        // batch, for a flag whose only reader is a status poller. Riding the
        // batch costs neither, and cannot report "processing" for work that
        // then rolls back.
        //
        // `status IN ('pending', 'processing')` keeps it a promotion rather
        // than a reset: the statement runs on *every* batch, so without the
        // guard the batch that lands right after `abort_submission` moved the
        // manifest to `'failed'` would quietly put it back to `'processing'`
        // and the abort would read as if it had never happened (#968).
        txn.with_connection(|conn| {
            conn.prepare_cached(
                "UPDATE bulk_manifests SET status = 'processing'
                 WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3 AND manifest_id = ?4
                   AND status IN ('pending', 'processing')",
            )
            .map_err(|e| internal_error(format!("prepare manifest status update: {e}")))?
            .execute(params![
                tenant_id,
                &submission_id.submitter,
                &submission_id.submission_id,
                manifest_id
            ])
            .map_err(|e| internal_error(format!("Failed to update manifest status: {}", e)))?;
            Ok(())
        })?;
        drop(batch_prologue);

        for entry in entries {
            // Check if we've hit max errors
            if options.max_errors > 0 && error_count >= options.max_errors {
                if !options.continue_on_error {
                    aborted_on_max_errors = true;
                    break;
                }
                // Skip remaining entries
                let skip_result = BulkEntryResult::skipped(
                    entry.line_number,
                    &entry.resource_type,
                    "max errors exceeded",
                );
                write_entry_rows(
                    &txn,
                    submission_id,
                    manifest_id,
                    file_url,
                    &skip_result,
                    None,
                )?;
                results.push(skip_result);
                continue;
            }

            let _entry_span = crate::perf::span(crate::perf::Phase::Entry);
            let result = self
                .ingest_entry_in_txn(&mut txn, manifest_id, &entry, options)
                .await;

            let (entry_result, change) = match result {
                Ok(outcome) => outcome,
                Err(e) => {
                    // Not counted here: the result built below is an error
                    // result, and the shared tally right after this match
                    // counts it. Counting in both places charged an ingest
                    // failure twice — invisible while the worker overwrote
                    // `failed_entries` with its own absolute total, and no
                    // longer (#969).
                    let failed = BulkEntryResult::processing_error(
                        entry.line_number,
                        &entry.resource_type,
                        serde_json::json!({
                            "resourceType": "OperationOutcome",
                            "issue": [{
                                "severity": "error",
                                "code": "exception",
                                "diagnostics": e.to_string()
                            }]
                        }),
                    );
                    (failed, None)
                }
            };

            if entry_result.is_error() {
                error_count += 1;
            }

            {
                let _span = crate::perf::span(crate::perf::Phase::Bookkeeping);
                write_entry_rows(
                    &txn,
                    submission_id,
                    manifest_id,
                    file_url,
                    &entry_result,
                    change.as_ref(),
                )?;
            }
            drop(_entry_span);
            results.push(entry_result);
        }

        // Manifest counters and the submission's timestamp, on the batch
        // transaction. These were two autocommit statements after the commit:
        // two more write-lock acquisitions and two more fsyncs per batch, and
        // a window in which the entries were durable but the counts that
        // describe them were not. Byte progress still rides this write — the
        // reason it is here at all is that a standalone flush (the lease
        // keeper's) starves against back-to-back batch transactions on SQLite,
        // and MAX keeps a late keeper flush from regressing it.
        //
        // The entry counters accumulate: they are cumulative across every run
        // of the manifest, and this batch — which owns them — is the only
        // writer that knows what it committed. `processed_entries` counts the
        // entries that did not fail, successes plus deliberate skips, so
        // processed + failed is the number of entries walked (#969).
        {
            let _span = crate::perf::span(crate::perf::Phase::BatchOverhead);
            let now = Utc::now().to_rfc3339();
            let (consumed, bytes_total) = options
                .byte_progress
                .as_ref()
                .map(|bp| {
                    (
                        bp.consumed.load(std::sync::atomic::Ordering::Relaxed) as i64,
                        bp.total.load(std::sync::atomic::Ordering::Relaxed) as i64,
                    )
                })
                .unwrap_or((0, 0));
            let total = results.len() as i64;
            // `processed_entries` means resources written to the store, so skips
            // are excluded and surface through their receipts (#954).
            // `last_processed_line` is a line cursor, not an outcome tally, so it
            // advances by every entry the batch walked (#969).
            let succeeded = results.iter().filter(|r| r.is_success()).count() as i64;
            txn.with_connection(|conn| {
                conn.prepare_cached(
                    "UPDATE bulk_manifests SET
                        total_entries = total_entries + ?1,
                        processed_entries = processed_entries + ?2,
                        failed_entries = failed_entries + ?3,
                        last_processed_line = last_processed_line + ?1,
                        bytes_processed = MAX(bytes_processed, ?8),
                        bytes_total = MAX(bytes_total, ?9)
                     WHERE tenant_id = ?4 AND submitter = ?5 AND submission_id = ?6 AND manifest_id = ?7",
                )
                .map_err(|e| internal_error(format!("prepare manifest counts update: {e}")))?
                .execute(params![
                    total,
                    succeeded,
                    error_count as i64,
                    tenant_id,
                    &submission_id.submitter,
                    &submission_id.submission_id,
                    manifest_id,
                    consumed,
                    bytes_total
                ])
                .map_err(|e| internal_error(format!("Failed to update manifest counts: {}", e)))?;

                conn.prepare_cached(
                    "UPDATE bulk_submissions SET updated_at = ?1
                     WHERE tenant_id = ?2 AND submitter = ?3 AND submission_id = ?4",
                )
                .map_err(|e| internal_error(format!("prepare submission touch: {e}")))?
                .execute(params![
                    now,
                    tenant_id,
                    &submission_id.submitter,
                    &submission_id.submission_id
                ])
                .map_err(|e| internal_error(format!("Failed to update submission: {}", e)))?;
                Ok(())
            })?;
        }

        crate::core::Transaction::commit(Box::new(txn)).await?;

        if aborted_on_max_errors {
            return Err(StorageError::BulkSubmit(
                BulkSubmitError::MaxErrorsExceeded {
                    submission_id: submission_id.submission_id.clone(),
                    max_errors: options.max_errors,
                },
            ));
        }

        Ok(results)
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
        if limit == 0 {
            return Err(invalid_entry_result_page(
                "Receipt page limit must be greater than zero",
            ));
        }
        let after = match continuation {
            None => None,
            Some(EntryResultContinuation::Keyset(cursor)) => Some((
                cursor.file_url.as_str(),
                i64::try_from(cursor.line_number).map_err(|_| {
                    invalid_entry_result_page("Receipt cursor line exceeds SQLite INTEGER range")
                })?,
            )),
            Some(EntryResultContinuation::Offset(_)) => {
                return Err(invalid_entry_result_page(
                    "SQLite receipt pages require a keyset continuation",
                ));
            }
        };
        let conn = self.get_connection()?;
        let mut query = "SELECT file_url, line_number, resource_type, resource_id, created, outcome, operation_outcome
             FROM bulk_entry_results
             WHERE tenant_id = ? AND submitter = ? AND submission_id = ? AND manifest_id = ?".to_string();
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![
            Box::new(tenant.tenant_id().as_str().to_string()),
            Box::new(submission_id.submitter.clone()),
            Box::new(submission_id.submission_id.clone()),
            Box::new(manifest_id.to_string()),
        ];
        if let Some(outcome) = outcome_filter {
            query.push_str(" AND outcome = ?");
            params_vec.push(Box::new(outcome.to_string()));
        }
        if let Some((file, line)) = after {
            query.push_str(" AND (file_url, line_number) > (?, ?)");
            params_vec.push(Box::new(file.to_string()));
            params_vec.push(Box::new(line));
        }
        query.push_str(" ORDER BY file_url, line_number LIMIT ?");
        params_vec.push(Box::new(i64::from(limit)));
        let params_slice: Vec<&dyn rusqlite::ToSql> =
            params_vec.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn.prepare(&query)?;
        let mut rows = stmt.query(params_slice.as_slice())?;
        let mut entries = Vec::new();
        while let Some(row) = rows.next()? {
            let file_url: String = row.get(0)?;
            let line: i64 = row.get(1)?;
            let line_number = u64::try_from(line)
                .map_err(|_| internal_error("Negative stored receipt line number".to_string()))?;
            let resource_type = row.get(2)?;
            let resource_id = row.get(3)?;
            let created: Option<i32> = row.get(4)?;
            let outcome_str: String = row.get(5)?;
            let operation_outcome_bytes: Option<Vec<u8>> = row.get(6)?;
            let operation_outcome = operation_outcome_bytes
                .map(|b| serde_json::from_slice(&b))
                .transpose()?;
            let outcome = outcome_str
                .parse()
                .unwrap_or(BulkEntryOutcome::ProcessingError);
            entries.push(PagedEntryResult {
                stored_identity: Some(EntryResultCursor {
                    file_url,
                    line_number,
                }),
                result: BulkEntryResult {
                    line_number,
                    resource_type,
                    resource_id,
                    created: created.is_some_and(|value| value != 0),
                    outcome,
                    operation_outcome,
                },
            });
        }
        let next = if entries.len() == limit as usize {
            entries
                .last()
                .and_then(|entry| entry.stored_identity.clone())
                .map(EntryResultContinuation::Keyset)
        } else {
            None
        };
        Ok(EntryResultPage { entries, next })
    }

    async fn get_entry_counts(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_id: &str,
    ) -> StorageResult<EntryCountSummary> {
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        let (total, success, validation_error, processing_error, skipped): (i64, i64, i64, i64, i64) = conn
            .query_row(
                "SELECT
                    COUNT(*),
                    SUM(CASE WHEN outcome = 'success' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN outcome = 'validation-error' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN outcome = 'processing-error' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN outcome = 'skipped' THEN 1 ELSE 0 END)
                 FROM bulk_entry_results
                 WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3 AND manifest_id = ?4",
                params![tenant_id, &submission_id.submitter, &submission_id.submission_id, manifest_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .unwrap_or((0, 0, 0, 0, 0));

        Ok(EntryCountSummary {
            total: total as u64,
            success: success as u64,
            validation_error: validation_error as u64,
            processing_error: processing_error as u64,
            skipped: skipped as u64,
        })
    }

    async fn mark_entries_unindexed(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_id: &str,
        entries: &[UnindexedEntry],
    ) -> StorageResult<u64> {
        if entries.is_empty() {
            return Ok(0);
        }
        let mut conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str().to_string();
        let tx = conn
            .transaction()
            .map_err(|e| internal_error(format!("Failed to begin unindexed-mark txn: {}", e)))?;
        let mut affected = 0u64;
        {
            let mut stmt = tx
                .prepare(
                    "UPDATE bulk_entry_results
                     SET outcome = 'processing-error', operation_outcome = ?1
                     WHERE tenant_id = ?2 AND submitter = ?3 AND submission_id = ?4
                       AND manifest_id = ?5 AND resource_type = ?6 AND resource_id = ?7",
                )
                .map_err(|e| internal_error(format!("Failed to prepare unindexed-mark: {}", e)))?;
            for entry in entries {
                let outcome_bytes = serde_json::to_vec(&entry.operation_outcome).map_err(|e| {
                    internal_error(format!("Failed to serialize operation outcome: {}", e))
                })?;
                let n = stmt
                    .execute(params![
                        outcome_bytes,
                        tenant_id,
                        &submission_id.submitter,
                        &submission_id.submission_id,
                        manifest_id,
                        &entry.resource_type,
                        &entry.resource_id,
                    ])
                    .map_err(|e| {
                        internal_error(format!("Failed to mark entry unindexed: {}", e))
                    })?;
                affected += n as u64;
            }
        }
        tx.commit()
            .map_err(|e| internal_error(format!("Failed to commit unindexed-mark txn: {}", e)))?;
        Ok(affected)
    }
}

/// One entry's bookkeeping rows, on the batch transaction's own connection so
/// they commit (or vanish) with the resource writes they describe (#815
/// review): the rollback record when the entry mutated something, and the
/// per-line receipt keyed by the file it came from (#457 — line numbers
/// restart per file, so the file is part of the identity). OR REPLACE: the
/// worker re-fetches a whole file after a transient failure, and the retry
/// must overwrite its own earlier rows instead of colliding with them.
fn write_entry_rows(
    txn: &super::transaction::SqliteTransaction,
    submission_id: &SubmissionId,
    manifest_id: &str,
    file_url: &str,
    result: &BulkEntryResult,
    change: Option<&SubmissionChange>,
) -> StorageResult<()> {
    use crate::core::Transaction;

    let tenant_id = txn.tenant().tenant_id().as_str().to_string();
    txn.with_connection(|conn| {
        if let Some(change) = change {
            let previous_content_bytes = change
                .previous_content
                .as_ref()
                .and_then(|c| serde_json::to_vec(c).ok());
            conn.prepare_cached(
                "INSERT INTO bulk_submission_changes
                 (tenant_id, submitter, submission_id, change_id, manifest_id, change_type, resource_type, resource_id, previous_version, new_version, previous_content, changed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            )
            .map_err(|e| internal_error(format!("prepare change insert: {e}")))?
            .execute(params![
                tenant_id,
                &submission_id.submitter,
                &submission_id.submission_id,
                &change.change_id,
                &change.manifest_id,
                change.change_type.to_string(),
                &change.resource_type,
                &change.resource_id,
                &change.previous_version,
                &change.new_version,
                previous_content_bytes,
                change.changed_at.to_rfc3339()
            ])
            .map_err(|e| internal_error(format!("Failed to record change: {}", e)))?;
        }

        let outcome_bytes = result
            .operation_outcome
            .as_ref()
            .and_then(|o| serde_json::to_vec(o).ok());
        conn.prepare_cached(
            "INSERT OR REPLACE INTO bulk_entry_results
             (tenant_id, submitter, submission_id, manifest_id, file_url, line_number, resource_type, resource_id, created, outcome, operation_outcome)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        )
        .map_err(|e| internal_error(format!("prepare entry-result insert: {e}")))?
        .execute(params![
            tenant_id,
            &submission_id.submitter,
            &submission_id.submission_id,
            manifest_id,
            file_url,
            result.line_number as i64,
            &result.resource_type,
            &result.resource_id,
            if result.created { Some(1) } else { Some(0) },
            result.outcome.to_string(),
            outcome_bytes
        ])
        .map_err(|e| internal_error(format!("Failed to store entry result: {}", e)))?;
        Ok(())
    })
}

impl SqliteBackend {
    /// One NDJSON entry inside the batch transaction (#813): read, then
    /// update (import-mode aware) or create. Returns the entry's receipt and
    /// the rollback record its mutation warrants; both land on the batch
    /// transaction alongside the write itself. Same outcomes as the former
    /// per-entry autocommit path, minus its per-entry fsyncs.
    async fn ingest_entry_in_txn(
        &self,
        txn: &mut super::transaction::SqliteTransaction,
        manifest_id: &str,
        entry: &NdjsonEntry,
        options: &BulkProcessingOptions,
    ) -> StorageResult<(BulkEntryResult, Option<SubmissionChange>)> {
        use crate::core::Transaction;

        if let Some(id) = entry.resource_id.as_ref() {
            let existing = {
                let _span = crate::perf::span(crate::perf::Phase::EntryRead);
                txn.read(&entry.resource_type, id).await?
            };
            match existing {
                Some(current) => {
                    if !options.allow_updates {
                        return Ok((
                            BulkEntryResult::skipped(
                                entry.line_number,
                                &entry.resource_type,
                                "updates not allowed",
                            ),
                            None,
                        ));
                    }

                    let change = SubmissionChange::update(
                        manifest_id,
                        &entry.resource_type,
                        id,
                        current.version_id(),
                        (current.version_id().parse::<i32>().unwrap_or(0) + 1).to_string(),
                        current.content().clone(),
                    );

                    // Update the resource, honoring the submission's import mode.
                    let content = options.content_for_update(current.content(), &entry.resource);
                    let updated = txn.update(&current, content).await?;

                    Ok((
                        BulkEntryResult::success(
                            entry.line_number,
                            &entry.resource_type,
                            updated.id(),
                            false,
                        ),
                        Some(change),
                    ))
                }
                // A soft-deleted row reads as None and then fails the create
                // with AlreadyExists — the same outcome the storage-path
                // create produced, recorded as this entry's error.
                None => {
                    let created = txn
                        .create(&entry.resource_type, entry.resource.clone())
                        .await?;
                    let change = SubmissionChange::create(
                        manifest_id,
                        &entry.resource_type,
                        created.id(),
                        created.version_id(),
                    );

                    Ok((
                        BulkEntryResult::success(
                            entry.line_number,
                            &entry.resource_type,
                            created.id(),
                            true,
                        ),
                        Some(change),
                    ))
                }
            }
        } else {
            let created = txn
                .create(&entry.resource_type, entry.resource.clone())
                .await?;
            let change = SubmissionChange::create(
                manifest_id,
                &entry.resource_type,
                created.id(),
                created.version_id(),
            );

            Ok((
                BulkEntryResult::success(
                    entry.line_number,
                    &entry.resource_type,
                    created.id(),
                    true,
                ),
                Some(change),
            ))
        }
    }
}

#[async_trait]
impl StreamingBulkSubmitProvider for SqliteBackend {
    async fn process_ndjson_stream(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_id: &str,
        resource_type: &str,
        mut reader: Box<dyn AsyncBufRead + Send + Unpin>,
        options: &BulkProcessingOptions,
    ) -> StorageResult<StreamProcessingResult> {
        let mut result = StreamProcessingResult::new();
        let mut line_number = 0u64;
        let mut batch = Vec::new();

        // An ingest cancelled before it read anything persists nothing.
        if options.is_cancelled() {
            return Ok(result.aborted(CANCELLED_ABORT_REASON));
        }

        loop {
            let mut line = String::new();
            let bytes_read = reader
                .read_line(&mut line)
                .await
                .map_err(|e| internal_error(format!("Failed to read line: {}", e)))?;

            if bytes_read == 0 {
                // End of stream
                break;
            }

            line_number += 1;
            result.lines_processed = line_number;

            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            // Parse the line
            let parsed = {
                let _span = crate::perf::span(crate::perf::Phase::NdjsonParse);
                NdjsonEntry::parse(line_number, line)
            };
            match parsed {
                Ok(entry) => {
                    // Validate resource type matches
                    if entry.resource_type != resource_type {
                        let error_result = BulkEntryResult::validation_error(
                            line_number,
                            &entry.resource_type,
                            serde_json::json!({
                                "resourceType": "OperationOutcome",
                                "issue": [{
                                    "severity": "error",
                                    "code": "invalid",
                                    "diagnostics": format!("Expected resource type {}, got {}", resource_type, entry.resource_type)
                                }]
                            }),
                        );
                        // Rejected here, so no batch will charge it to the
                        // manifest's counters; the worker adds it (#969).
                        result.counts.increment(error_result.outcome);
                        result.unbatched_errors += 1;

                        if !options.continue_on_error
                            && (options.max_errors == 0
                                || result.counts.error_count() >= options.max_errors as u64)
                        {
                            return Ok(result.aborted("max errors exceeded"));
                        }
                        continue;
                    }

                    batch.push(entry);
                }
                Err(e) => {
                    result.counts.increment(BulkEntryOutcome::ValidationError);
                    result.unbatched_errors += 1;

                    if !options.continue_on_error
                        && (options.max_errors == 0
                            || result.counts.error_count() >= options.max_errors as u64)
                    {
                        return Ok(result.aborted(format!("Parse error: {}", e)));
                    }
                }
            }

            // Process batch if it's full
            if batch.len() >= options.batch_size as usize {
                let batch_results = self
                    .process_entries(
                        tenant,
                        submission_id,
                        manifest_id,
                        std::mem::take(&mut batch),
                        options,
                    )
                    .await?;

                for r in batch_results {
                    result.counts.increment(r.outcome);
                }

                // Check if we need to abort
                if !options.continue_on_error
                    && options.max_errors > 0
                    && result.counts.error_count() >= options.max_errors as u64
                {
                    return Ok(result.aborted("max errors exceeded"));
                }

                // Abort is cooperative: a claimed manifest checks between
                // batches, so an aborted submission stops here with its partial
                // counts intact instead of running to the end (#968).
                if options.is_cancelled() {
                    return Ok(result.aborted(CANCELLED_ABORT_REASON));
                }
            }
        }

        // Process remaining entries
        if !batch.is_empty() {
            let batch_results = self
                .process_entries(tenant, submission_id, manifest_id, batch, options)
                .await?;

            for r in batch_results {
                result.counts.increment(r.outcome);
            }
        }

        Ok(result)
    }
}

#[async_trait]
impl BulkSubmitRollbackProvider for SqliteBackend {
    async fn record_change(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        change: &SubmissionChange,
    ) -> StorageResult<()> {
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        let previous_content_bytes = change
            .previous_content
            .as_ref()
            .and_then(|c| serde_json::to_vec(c).ok());

        conn.execute(
            "INSERT INTO bulk_submission_changes
             (tenant_id, submitter, submission_id, change_id, manifest_id, change_type, resource_type, resource_id, previous_version, new_version, previous_content, changed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                tenant_id,
                &submission_id.submitter,
                &submission_id.submission_id,
                &change.change_id,
                &change.manifest_id,
                change.change_type.to_string(),
                &change.resource_type,
                &change.resource_id,
                &change.previous_version,
                &change.new_version,
                previous_content_bytes,
                change.changed_at.to_rfc3339()
            ],
        )
        .map_err(|e| internal_error(format!("Failed to record change: {}", e)))?;

        Ok(())
    }

    async fn list_changes(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        limit: u32,
        offset: u32,
    ) -> StorageResult<Vec<SubmissionChange>> {
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        let mut stmt = conn
            .prepare(&format!(
                "SELECT change_id, manifest_id, change_type, resource_type, resource_id, previous_version, new_version, previous_content, changed_at
                 FROM bulk_submission_changes
                 WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3
                 ORDER BY changed_at DESC
                 LIMIT {} OFFSET {}",
                limit, offset
            ))
            .map_err(|e| internal_error(format!("Failed to prepare changes query: {}", e)))?;

        let changes: Vec<SubmissionChange> = stmt
            .query_map(
                params![
                    tenant_id,
                    &submission_id.submitter,
                    &submission_id.submission_id
                ],
                |row| {
                    let change_id: String = row.get(0)?;
                    let manifest_id: String = row.get(1)?;
                    let change_type_str: String = row.get(2)?;
                    let resource_type: String = row.get(3)?;
                    let resource_id: String = row.get(4)?;
                    let previous_version: Option<String> = row.get(5)?;
                    let new_version: String = row.get(6)?;
                    let previous_content_bytes: Option<Vec<u8>> = row.get(7)?;
                    let changed_at_str: String = row.get(8)?;

                    let change_type: ChangeType =
                        change_type_str.parse().unwrap_or(ChangeType::Create);
                    let previous_content =
                        previous_content_bytes.and_then(|b| serde_json::from_slice(&b).ok());
                    let changed_at = chrono::DateTime::parse_from_rfc3339(&changed_at_str)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now());

                    Ok(SubmissionChange {
                        change_id,
                        manifest_id,
                        change_type,
                        resource_type,
                        resource_id,
                        previous_version,
                        new_version,
                        previous_content,
                        changed_at,
                    })
                },
            )
            .map_err(|e| internal_error(format!("Failed to query changes: {}", e)))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(changes)
    }

    async fn rollback_change(
        &self,
        tenant: &TenantContext,
        _submission_id: &SubmissionId,
        change: &SubmissionChange,
    ) -> StorageResult<bool> {
        match change.change_type {
            ChangeType::Create => {
                // Delete the created resource
                match self
                    .delete(tenant, &change.resource_type, &change.resource_id)
                    .await
                {
                    Ok(()) => Ok(true),
                    Err(StorageError::Resource(crate::error::ResourceError::NotFound {
                        ..
                    })) => {
                        // Already deleted
                        Ok(true)
                    }
                    Err(e) => Err(e),
                }
            }
            ChangeType::Update => {
                // Restore the previous content
                if let Some(ref previous_content) = change.previous_content {
                    // Read current to get version for update
                    let current = self
                        .read(tenant, &change.resource_type, &change.resource_id)
                        .await?;
                    if let Some(current) = current {
                        self.update(tenant, &current, previous_content.clone())
                            .await?;
                        Ok(true)
                    } else {
                        // Resource no longer exists
                        Ok(false)
                    }
                } else {
                    // No previous content to restore
                    Ok(false)
                }
            }
        }
    }
}

#[async_trait]
impl SubmitClaimStrategy for SqliteBackend {
    async fn claim_next_manifest(
        &self,
        worker_id: &WorkerId,
        lease_duration: StdDuration,
    ) -> StorageResult<Option<ManifestLease>> {
        let _guard = SUBMIT_CLAIM_LOCK.lock().await;
        let conn = self.get_connection()?;
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        let lease_expiry = now
            + chrono::Duration::from_std(lease_duration)
                .unwrap_or_else(|_| chrono::Duration::seconds(60));
        let lease_expiry_str = lease_expiry.to_rfc3339();

        // Find one eligible manifest with a fetchable URL: pending, or processing
        // with an expired lease. Only manifests of non-terminal submissions count.
        let row: Option<(String, String, String, String, i64)> = conn
            .query_row(
                "SELECT m.tenant_id, m.submitter, m.submission_id, m.manifest_id, m.fencing_token
                 FROM bulk_manifests m
                 JOIN bulk_submissions s
                   ON s.tenant_id = m.tenant_id AND s.submitter = m.submitter
                      AND s.submission_id = m.submission_id
                 -- `complete` is admitted alongside `in-progress`: it means the
                 -- submitter will send no further manifests, not that already
                 -- registered ones should be dropped. `aborted` stays excluded.
                 WHERE m.manifest_url IS NOT NULL
                   AND s.status IN ('in-progress', 'complete')
                   AND (m.status = 'pending'
                        OR (m.status = 'processing'
                            AND (m.lease_expiry IS NULL OR m.lease_expiry < ?1)))
                 ORDER BY m.added_at LIMIT 1",
                params![now_str],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?,
                    ))
                },
            )
            .ok();

        let Some((tenant_id, submitter, submission_id, manifest_id, fencing_token)) = row else {
            return Ok(None);
        };
        let new_token = fencing_token + 1;

        conn.execute(
            "UPDATE bulk_manifests
             SET status = 'processing', worker_id = ?1, lease_expiry = ?2, fencing_token = ?3
             WHERE tenant_id = ?4 AND submitter = ?5 AND submission_id = ?6 AND manifest_id = ?7",
            params![
                worker_id.as_str(),
                lease_expiry_str,
                new_token,
                tenant_id,
                submitter,
                submission_id,
                manifest_id
            ],
        )
        .map_err(|e| internal_error(format!("Failed to claim manifest: {}", e)))?;

        Ok(Some(ManifestLease {
            tenant: TenantContext::new(TenantId::new(tenant_id), TenantPermissions::full_access()),
            submission_id: SubmissionId::new(submitter, submission_id),
            manifest_id,
            worker_id: worker_id.clone(),
            lease_expiry,
            lease_duration,
            fencing_token: new_token as u64,
        }))
    }

    async fn heartbeat(&self, lease: &ManifestLease) -> Result<DateTime<Utc>, LeaseError> {
        let new_expiry = lease.renewed_expiry();
        // The heartbeat competes with the ingest batches for the write lock, so
        // it is exactly the write that must not give up on the first
        // SQLITE_BUSY: dropping it is what lets the lease lapse under a healthy
        // database (#942). Absolute expiry guarded by worker_id +
        // fencing_token, so a retry is idempotent and a stale lease still
        // loses.
        let affected = retry_bookkeeping_on_busy("heartbeat", lease_retry_budget(lease), || {
            let conn = self.get_connection()?;
            conn.execute(
                "UPDATE bulk_manifests SET lease_expiry = ?1
                 WHERE tenant_id = ?2 AND submitter = ?3 AND submission_id = ?4
                   AND manifest_id = ?5 AND status = 'processing'
                   AND worker_id = ?6 AND fencing_token = ?7",
                params![
                    new_expiry.to_rfc3339(),
                    lease.tenant.tenant_id().as_str(),
                    lease.submission_id.submitter,
                    lease.submission_id.submission_id,
                    lease.manifest_id,
                    lease.worker_id.as_str(),
                    lease.fencing_token as i64
                ],
            )
            .map_err(|e| StorageError::Backend(classify_sqlite_error("heartbeat failed", e)))
        })
        .await
        .map_err(LeaseError::Storage)?;
        if affected == 0 {
            Err(lease_lost(lease))
        } else {
            Ok(new_expiry)
        }
    }

    async fn release(&self, lease: ManifestLease) -> StorageResult<()> {
        let conn = self.get_connection()?;
        conn.execute(
            "UPDATE bulk_manifests
             SET status = 'pending', worker_id = NULL, lease_expiry = NULL
             WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3 AND manifest_id = ?4
               AND worker_id = ?5 AND fencing_token = ?6 AND status = 'processing'",
            params![
                lease.tenant.tenant_id().as_str(),
                lease.submission_id.submitter,
                lease.submission_id.submission_id,
                lease.manifest_id,
                lease.worker_id.as_str(),
                lease.fencing_token as i64
            ],
        )
        .map_err(|e| internal_error(format!("Failed to release manifest lease: {}", e)))?;
        Ok(())
    }
}

#[async_trait]
impl SubmitWorkerStorage for SqliteBackend {
    async fn get_manifest_for_worker(
        &self,
        lease: &ManifestLease,
    ) -> Result<ManifestWorkerView, LeaseError> {
        let conn = self.get_connection().map_err(LeaseError::Storage)?;
        type Row = (
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            i64,
            Option<String>,
            Option<String>,
        );
        let row: Row = conn
            .query_row(
                "SELECT manifest_url, fhir_base_url, output_format, file_request_headers,
                        oauth_metadata_urls, file_encryption_key, last_processed_line,
                        import_directives, submission_metadata
                 FROM bulk_manifests
                 WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3
                   AND manifest_id = ?4 AND status = 'processing'
                   AND worker_id = ?5 AND fencing_token = ?6",
                params![
                    lease.tenant.tenant_id().as_str(),
                    lease.submission_id.submitter,
                    lease.submission_id.submission_id,
                    lease.manifest_id,
                    lease.worker_id.as_str(),
                    lease.fencing_token as i64
                ],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                        r.get(7)?,
                        r.get(8)?,
                    ))
                },
            )
            .map_err(|error| {
                if matches!(error, rusqlite::Error::QueryReturnedNoRows) {
                    lease_lost(lease)
                } else {
                    LeaseError::Storage(StorageError::Backend(classify_sqlite_error(
                        "get manifest for worker",
                        error,
                    )))
                }
            })?;

        let (
            manifest_url,
            fhir_base_url,
            output_format,
            headers_json,
            oauth_json,
            encryption_json,
            last_processed_line,
            import_json,
            metadata_json,
        ) = row;

        let decode_json = |field: &str, json: Option<&str>| -> StorageResult<Value> {
            match json {
                Some(text) => serde_json::from_str(text)
                    .map_err(|error| internal_error(format!("decode manifest {field}: {error}"))),
                None => Ok(Value::Array(Vec::new())),
            }
        };
        let file_request_headers: Vec<(String, String)> = serde_json::from_value(decode_json(
            "file_request_headers",
            headers_json.as_deref(),
        )?)
        .map_err(|error| {
            internal_error(format!("invalid manifest file_request_headers: {error}"))
        })?;
        let oauth_metadata_urls: Vec<String> =
            serde_json::from_value(decode_json("oauth_metadata_urls", oauth_json.as_deref())?)
                .map_err(|error| {
                    internal_error(format!("invalid manifest oauth_metadata_urls: {error}"))
                })?;
        let file_encryption_key: Option<Value> = match encryption_json.as_deref() {
            Some(text) => Some(serde_json::from_str(text).map_err(|error| {
                internal_error(format!("decode manifest file_encryption_key: {error}"))
            })?),
            None => None,
        };
        let import_directives: Vec<(String, String)> =
            serde_json::from_value(decode_json("import_directives", import_json.as_deref())?)
                .map_err(|error| {
                    internal_error(format!("invalid manifest import_directives: {error}"))
                })?;
        let metadata: Vec<(String, String)> = serde_json::from_value(decode_json(
            "submission_metadata",
            metadata_json.as_deref(),
        )?)
        .map_err(|error| {
            internal_error(format!("invalid manifest submission_metadata: {error}"))
        })?;
        let fhir_version = fhir_version_from_output_format(output_format.as_deref());

        Ok(ManifestWorkerView {
            manifest_id: lease.manifest_id.clone(),
            manifest_url,
            fhir_base_url,
            output_format,
            file_request_headers,
            oauth_metadata_urls,
            file_encryption_key,
            import_directives,
            metadata,
            last_processed_line: u64::try_from(last_processed_line).map_err(|_| {
                internal_error("manifest last_processed_line does not fit u64".to_string())
            })?,
            fhir_version,
        })
    }

    async fn mark_manifest_processing(&self, lease: &ManifestLease) -> Result<(), LeaseError> {
        // Idempotent status write guarded by worker_id + fencing_token: a busy
        // retry re-applies the same value, and a stale lease still loses (#942).
        // Promotion only, same as the per-batch stamp: an abort landing in the
        // window between the claim and this call already moved the manifest to
        // `'failed'`, and re-marking it `'processing'` would strand it there
        // with nobody able to claim it again (#968). That also makes the retry
        // safe past an abort — the replay matches nothing and reads as a lost
        // lease, which is what a cancelled manifest should look like here.
        let affected = retry_bookkeeping_on_busy(
            "mark manifest processing",
            lease_retry_budget(lease),
            || {
                let conn = self.get_connection()?;
                conn.execute(
                    "UPDATE bulk_manifests SET status = 'processing'
                 WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3
                   AND manifest_id = ?4 AND status = 'processing'
                   AND worker_id = ?5 AND fencing_token = ?6",
                    params![
                        lease.tenant.tenant_id().as_str(),
                        lease.submission_id.submitter,
                        lease.submission_id.submission_id,
                        lease.manifest_id,
                        lease.worker_id.as_str(),
                        lease.fencing_token as i64
                    ],
                )
                .map_err(|e| StorageError::Backend(classify_sqlite_error("mark processing", e)))
            },
        )
        .await
        .map_err(LeaseError::Storage)?;
        if affected == 0 {
            Err(lease_lost(lease))
        } else {
            Ok(())
        }
    }

    async fn add_manifest_progress(
        &self,
        lease: &ManifestLease,
        processed_delta: u64,
        failed_delta: u64,
        lines_delta: u64,
    ) -> Result<(), LeaseError> {
        // Deltas, not absolutes: the batch bookkeeping in `process_entries`
        // accumulates into the same columns, so assigning here would stomp it
        // and walk the counters backwards on resume (#969).
        //
        // That makes this the one retried bookkeeping write that is *not*
        // idempotent, so the fencing token alone would not make a reissue safe
        // — applying the delta twice would inflate the counters even under a
        // valid lease. What keeps the retry correct is the narrower argument in
        // `retry_bookkeeping_on_busy`: only busy/locked is retried, and an
        // attempt that fails busy/locked never took the write lock, so the
        // statement rolled back and added nothing. Hence `classify_sqlite_error`
        // rather than `internal_error` — misclassify the busy and this write
        // stops being retried at all (#942).
        let affected = retry_bookkeeping_on_busy(
            "manifest progress update",
            lease_retry_budget(lease),
            || {
                let conn = self.get_connection()?;
                conn.execute(
                    "UPDATE bulk_manifests
                 SET processed_entries = processed_entries + ?1,
                     failed_entries = failed_entries + ?2,
                     last_processed_line = last_processed_line + ?3
                 WHERE tenant_id = ?4 AND submitter = ?5 AND submission_id = ?6
                   AND manifest_id = ?7 AND worker_id = ?8 AND fencing_token = ?9",
                    params![
                        processed_delta as i64,
                        failed_delta as i64,
                        lines_delta as i64,
                        lease.tenant.tenant_id().as_str(),
                        lease.submission_id.submitter,
                        lease.submission_id.submission_id,
                        lease.manifest_id,
                        lease.worker_id.as_str(),
                        lease.fencing_token as i64
                    ],
                )
                .map_err(|e| StorageError::Backend(classify_sqlite_error("update progress", e)))
            },
        )
        .await
        .map_err(LeaseError::Storage)?;
        if affected == 0 {
            Err(lease_lost(lease))
        } else {
            Ok(())
        }
    }

    async fn update_manifest_bytes(
        &self,
        lease: &ManifestLease,
        bytes_processed: u64,
        bytes_total: u64,
    ) -> Result<(), LeaseError> {
        // MAX() keeps the write monotonic, so a busy retry is idempotent and
        // the worker_id + fencing_token guard still fences stale leases (#942).
        let affected =
            retry_bookkeeping_on_busy("manifest bytes update", lease_retry_budget(lease), || {
                let conn = self.get_connection()?;
                conn.execute(
                    "UPDATE bulk_manifests
                 SET bytes_processed = MAX(bytes_processed, ?1),
                     bytes_total = MAX(bytes_total, ?2)
                 WHERE tenant_id = ?3 AND submitter = ?4 AND submission_id = ?5
                   AND manifest_id = ?6 AND worker_id = ?7 AND fencing_token = ?8",
                    params![
                        bytes_processed as i64,
                        bytes_total as i64,
                        lease.tenant.tenant_id().as_str(),
                        lease.submission_id.submitter,
                        lease.submission_id.submission_id,
                        lease.manifest_id,
                        lease.worker_id.as_str(),
                        lease.fencing_token as i64
                    ],
                )
                .map_err(|e| StorageError::Backend(classify_sqlite_error("update bytes", e)))
            })
            .await
            .map_err(LeaseError::Storage)?;
        if affected == 0 {
            Err(lease_lost(lease))
        } else {
            Ok(())
        }
    }

    async fn update_manifest_phase(
        &self,
        lease: &ManifestLease,
        phase: ManifestPhase,
        files_done: u64,
        files_total: u64,
    ) -> Result<(), LeaseError> {
        let conn = self.get_connection().map_err(LeaseError::Storage)?;
        // Plain overwrite, unlike the monotonic MAX() the byte counters need:
        // the phase walks forward and back within a run (sizing a file, then
        // downloading it), and the fence below makes the lease holder the only
        // writer.
        let affected = conn
            .execute(
                "UPDATE bulk_manifests
                 SET phase = ?1, files_done = ?2, files_total = ?3
                 WHERE tenant_id = ?4 AND submitter = ?5 AND submission_id = ?6
                   AND manifest_id = ?7 AND worker_id = ?8 AND fencing_token = ?9",
                params![
                    phase.to_string(),
                    files_done as i64,
                    files_total as i64,
                    lease.tenant.tenant_id().as_str(),
                    lease.submission_id.submitter,
                    lease.submission_id.submission_id,
                    lease.manifest_id,
                    lease.worker_id.as_str(),
                    lease.fencing_token as i64
                ],
            )
            .map_err(|e| LeaseError::Storage(internal_error(format!("update phase: {e}"))))?;
        if affected == 0 {
            Err(lease_lost(lease))
        } else {
            Ok(())
        }
    }

    async fn record_submit_file(
        &self,
        lease: &ManifestLease,
        file: &SubmitFileRecord,
    ) -> Result<(), LeaseError> {
        let staged =
            retry_bookkeeping_on_busy("record submit file", lease_retry_budget(lease), || {
                let mut conn = self.get_connection()?;
                let tx = conn
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(|e| {
                        StorageError::Backend(classify_sqlite_error("begin staging", e))
                    })?;
                let lease_token = i64::try_from(lease.fencing_token).map_err(|_| {
                    internal_error("lease fencing_token does not fit i64".to_string())
                })?;
                let Some(state) = manifest_publication_state(&tx, lease)? else {
                    return Ok(None);
                };
                if state.status == "replaced"
                    || state.status != "processing"
                    || state.fencing_token != lease_token
                    || state.worker_id.as_deref() != Some(lease.worker_id.as_str())
                {
                    return Ok(None);
                }
                let canonical = canonical_publication_files(std::slice::from_ref(file))?;
                validate_publication_records(&state, &canonical)?;
                let staged = read_manifest_generation_records(&tx, lease, lease_token)?;
                let identity = |record: &SubmitFileRecord| {
                    (
                        record.file_type.clone(),
                        record.resource_type.clone(),
                        record.part_index,
                    )
                };
                if let Some(existing) = staged
                    .iter()
                    .find(|record| identity(record) == identity(file))
                {
                    if existing != file {
                        return Err(publication_conflict(
                            "staged artifact identity has conflicting content",
                        ));
                    }
                    tx.commit()
                        .map_err(|e| {
                            StorageError::Backend(classify_sqlite_error("commit staged replay", e))
                        })
                        .map(|()| Some(()))?;
                    return Ok(Some(()));
                }

                let count_severity = match &file.count_severity {
                    Some(value) => Some(serde_json::to_string(value).map_err(|error| {
                        internal_error(format!("encode count_severity: {error}"))
                    })?),
                    None => None,
                };
                tx.execute(
                    "INSERT INTO bulk_submit_files
                     (tenant_id, submitter, submission_id, manifest_id, manifest_url, file_type,
                      resource_type, part_index, fencing_token, file_path, line_count,
                      byte_count, count_severity, created_at, publication_excluded_reason)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, NULL)",
                    params![
                        lease.tenant.tenant_id().as_str(),
                        lease.submission_id.submitter,
                        lease.submission_id.submission_id,
                        lease.manifest_id,
                        file.manifest_url,
                        file.file_type,
                        file.resource_type,
                        file.part_index,
                        lease_token,
                        file.file_path,
                        file.line_count,
                        file.byte_count,
                        count_severity,
                        Utc::now().to_rfc3339(),
                    ],
                )
                .map_err(|e| {
                    StorageError::Backend(classify_sqlite_error("stage submit file", e))
                })?;
                tx.commit()
                    .map_err(|e| StorageError::Backend(classify_sqlite_error("commit staging", e)))
                    .map(|()| Some(()))
            })
            .await
            .map_err(LeaseError::Storage)?;
        if staged.is_none() {
            return Err(lease_lost(lease));
        }
        Ok(())
    }

    async fn publish_manifest_artifacts(
        &self,
        lease: &ManifestLease,
        files: &[SubmitFileRecord],
        terminal: ManifestPublicationStatus,
    ) -> Result<ManifestPublicationResult, LeaseError> {
        SqliteBackend::publish_manifest_artifacts(self, lease, files, terminal).await
    }

    async fn checkpoint_after_file(&self) {
        // Fold the WAL back into the database at a file boundary (#978). The
        // passive auto-checkpoint yields to the back-to-back batch writers and
        // lets the WAL grow into the multi-gigabyte range over a long ingest,
        // which slows every read and doubles disk use; a TRUNCATE checkpoint
        // between files reclaims it while no batch holds the write lock. A
        // busy return (a reader still in a WAL frame) is fine — the next file
        // boundary tries again — so this is best-effort and never fails the
        // ingest. Runs on a blocking thread so the checkpoint's I/O does not
        // stall an async worker.
        let backend = self.clone();
        let _ = tokio::task::spawn_blocking(move || {
            if let Ok(conn) = backend.get_connection() {
                let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
            }
        })
        .await;
    }

    async fn finish_manifest(&self, lease: &ManifestLease) -> Result<(), LeaseError> {
        // The helper's fence (`status = 'processing'` plus this lease's worker
        // and fencing token) is what makes a finish that lands after
        // `abort_submission` moved the manifest to `'failed'` a `LeaseLost`
        // no-op instead of a silent rewrite back to `'completed'` (#968).
        self.publish_current_manifest_generation(lease, ManifestPublicationStatus::Completed)
            .await
    }

    async fn fail_manifest(
        &self,
        lease: &ManifestLease,
        error_message: &str,
    ) -> Result<(), LeaseError> {
        // Same fence as `finish_manifest`: an aborted manifest's outcome belongs
        // to the abort, so a late worker verdict gets `LeaseLost` instead (#968).
        self.publish_current_manifest_generation(
            lease,
            ManifestPublicationStatus::Failed {
                error_message: error_message.to_string(),
            },
        )
        .await
    }

    async fn set_manifest_fetch_params(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
        manifest_id: &str,
        fetch: ManifestFetchParams<'_>,
    ) -> StorageResult<()> {
        let conn = self.get_connection()?;
        let headers_json = serde_json::to_string(fetch.file_request_headers).ok();
        let oauth_json = serde_json::to_string(fetch.oauth_metadata_urls).ok();
        let encryption_json = fetch
            .file_encryption_key
            .and_then(|v| serde_json::to_string(v).ok());
        let import_json = serde_json::to_string(fetch.import_directives).ok();
        let metadata_json = serde_json::to_string(fetch.metadata).ok();
        conn.execute(
            "UPDATE bulk_manifests
             SET fhir_base_url = ?1, output_format = ?2, file_request_headers = ?3,
                 oauth_metadata_urls = ?4, file_encryption_key = ?5,
                 import_directives = ?6, submission_metadata = ?7
             WHERE tenant_id = ?8 AND submitter = ?9 AND submission_id = ?10
               AND manifest_id = ?11",
            params![
                fetch.fhir_base_url,
                fetch.output_format,
                headers_json,
                oauth_json,
                encryption_json,
                import_json,
                metadata_json,
                tenant.tenant_id().as_str(),
                id.submitter,
                id.submission_id,
                manifest_id
            ],
        )
        .map_err(|e| internal_error(format!("set manifest fetch params: {e}")))?;
        Ok(())
    }

    async fn replace_manifest_by_url(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
        manifest_url: &str,
    ) -> StorageResult<Vec<String>> {
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();
        let mut stmt = conn
            .prepare(
                "SELECT manifest_id FROM bulk_manifests
                 WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3
                   AND manifest_url = ?4 AND status != 'replaced'",
            )
            .map_err(|e| internal_error(format!("prepare replace lookup: {e}")))?;
        let ids: Vec<String> = stmt
            .query_map(
                params![tenant_id, id.submitter, id.submission_id, manifest_url],
                |r| r.get::<_, String>(0),
            )
            .map_err(|e| internal_error(format!("query replace lookup: {e}")))?
            .filter_map(|r| r.ok())
            .collect();
        conn.execute(
            "UPDATE bulk_manifests SET status = 'replaced'
             WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3 AND manifest_url = ?4",
            params![tenant_id, id.submitter, id.submission_id, manifest_url],
        )
        .map_err(|e| internal_error(format!("mark replaced: {e}")))?;
        Ok(ids)
    }

    async fn set_submission_kickoff_meta(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
        owner_subject: Option<&str>,
        request_url: &str,
        requires_access_token: bool,
    ) -> StorageResult<()> {
        let conn = self.get_connection()?;
        conn.execute(
            "UPDATE bulk_submissions
             SET owner_subject = ?1, request_url = ?2, requires_access_token = ?3
             WHERE tenant_id = ?4 AND submitter = ?5 AND submission_id = ?6",
            params![
                owner_subject,
                request_url,
                requires_access_token as i64,
                tenant.tenant_id().as_str(),
                id.submitter,
                id.submission_id
            ],
        )
        .map_err(|e| internal_error(format!("set kickoff meta: {e}")))?;
        Ok(())
    }

    async fn ensure_poll_token(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<String> {
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();
        let existing: Option<String> = conn
            .query_row(
                "SELECT poll_token FROM bulk_submissions
                 WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3",
                params![tenant_id, id.submitter, id.submission_id],
                |r| r.get::<_, Option<String>>(0),
            )
            .map_err(|e| internal_error(format!("read poll token: {e}")))?;
        if let Some(token) = existing {
            return Ok(token);
        }
        let token = Uuid::new_v4().to_string();
        conn.execute(
            "UPDATE bulk_submissions SET poll_token = ?1
             WHERE tenant_id = ?2 AND submitter = ?3 AND submission_id = ?4",
            params![token, tenant_id, id.submitter, id.submission_id],
        )
        .map_err(|e| internal_error(format!("set poll token: {e}")))?;
        Ok(token)
    }

    async fn list_expired_submissions(
        &self,
        now: DateTime<Utc>,
        ttl: StdDuration,
        limit: u32,
    ) -> StorageResult<Vec<(TenantContext, SubmissionId)>> {
        let conn = self.get_connection()?;
        let cutoff = (now
            - chrono::Duration::from_std(ttl).unwrap_or_else(|_| chrono::Duration::seconds(86400)))
        .to_rfc3339();
        let mut stmt = conn
            .prepare(
                "SELECT tenant_id, submitter, submission_id FROM bulk_submissions
                 WHERE updated_at < ?1 ORDER BY updated_at LIMIT ?2",
            )
            .map_err(|e| internal_error(format!("prepare expired: {e}")))?;
        let rows = stmt
            .query_map(params![cutoff, limit], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| internal_error(format!("query expired: {e}")))?;
        let mut out = Vec::new();
        for row in rows {
            let (tenant_id, submitter, submission_id) =
                row.map_err(|e| internal_error(format!("row expired: {e}")))?;
            out.push((
                TenantContext::new(TenantId::new(tenant_id), TenantPermissions::full_access()),
                SubmissionId::new(submitter, submission_id),
            ));
        }
        Ok(out)
    }

    async fn resolve_poll_token(&self, token: &str) -> StorageResult<Option<PollTokenTarget>> {
        let conn = self.get_connection()?;
        let row: Option<(String, String, String, Option<String>)> = conn
            .query_row(
                "SELECT tenant_id, submitter, submission_id, owner_subject
                 FROM bulk_submissions WHERE poll_token = ?1",
                params![token],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .ok();
        Ok(row.map(
            |(tenant_id, submitter, submission_id, owner_subject)| PollTokenTarget {
                tenant: TenantContext::new(
                    TenantId::new(tenant_id),
                    TenantPermissions::full_access(),
                ),
                submission_id: SubmissionId::new(submitter, submission_id),
                owner_subject,
            },
        ))
    }

    async fn clear_poll_token(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<()> {
        let conn = self.get_connection()?;
        conn.execute(
            "UPDATE bulk_submissions SET poll_token = NULL
             WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3",
            params![tenant.tenant_id().as_str(), id.submitter, id.submission_id],
        )
        .map_err(|e| internal_error(format!("clear poll token: {e}")))?;
        Ok(())
    }

    async fn list_submit_files(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<Vec<SubmitFileRow>> {
        let conn = self.get_connection()?;
        let mut stmt = conn
            .prepare(
                "SELECT f.manifest_url, f.file_type, f.resource_type, f.part_index, f.fencing_token,
                        f.file_path, f.line_count, f.byte_count, f.count_severity,
                        f.manifest_id, m.publication_worker_id
                 FROM bulk_submit_files AS f
                 INNER JOIN bulk_manifests AS m
                   ON m.tenant_id = f.tenant_id
                      AND m.submitter = f.submitter
                      AND m.submission_id = f.submission_id
                      AND m.manifest_id = f.manifest_id
                      AND m.published_token = f.fencing_token
                 WHERE f.manifest_id IS NOT NULL
                   AND f.tenant_id = ?1
                   AND f.submitter = ?2
                   AND f.submission_id = ?3
                   AND f.publication_excluded_reason IS NULL
                   AND m.published_token IS NOT NULL
                   AND m.publication_status IN ('completed', 'failed')
                 ORDER BY f.id",
            )
            .map_err(|e| internal_error(format!("prepare list files: {e}")))?;
        let rows = stmt
            .query_map(
                params![tenant.tenant_id().as_str(), id.submitter, id.submission_id],
                |r| {
                    Ok((
                        r.get::<_, Option<String>>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, i64>(4)?,
                        r.get::<_, String>(5)?,
                        r.get::<_, i64>(6)?,
                        r.get::<_, i64>(7)?,
                        r.get::<_, Option<String>>(8)?,
                        r.get::<_, Option<String>>(9)?,
                        r.get::<_, Option<String>>(10)?,
                    ))
                },
            )
            .map_err(|e| internal_error(format!("query list files: {e}")))?;
        let mut out = Vec::new();
        for row in rows {
            let (
                manifest_url,
                file_type,
                resource_type,
                part_index,
                fencing_token,
                file_path,
                line_count,
                byte_count,
                count_severity,
                manifest_id,
                publication_worker_id,
            ) = row.map_err(|e| internal_error(format!("row list files: {e}")))?;
            out.push(publication_row_from_parts(
                manifest_url,
                file_type,
                resource_type,
                part_index,
                fencing_token,
                file_path,
                line_count,
                byte_count,
                count_severity,
                manifest_id,
                publication_worker_id.is_none(),
            )?);
        }
        Ok(out)
    }

    async fn delete_submission_artifacts(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<()> {
        let mut conn = self.get_connection()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| {
                StorageError::Backend(classify_sqlite_error("begin artifact cleanup", e))
            })?;
        tx.execute(
            "DELETE FROM bulk_submit_files
             WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3",
            params![tenant.tenant_id().as_str(), id.submitter, id.submission_id],
        )
        .map_err(|e| {
            StorageError::Backend(classify_sqlite_error("delete submission artifacts", e))
        })?;
        tx.execute(
            "UPDATE bulk_manifests
             SET published_token = NULL, publication_status = NULL,
                 publication_error_message = NULL, publication_worker_id = NULL
             WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3",
            params![tenant.tenant_id().as_str(), id.submitter, id.submission_id],
        )
        .map_err(|e| {
            StorageError::Backend(classify_sqlite_error("clear publication markers", e))
        })?;
        tx.commit().map_err(|e| {
            StorageError::Backend(classify_sqlite_error("commit artifact cleanup", e))
        })?;
        Ok(())
    }

    async fn count_active_submissions(&self, tenant: &TenantContext) -> StorageResult<u64> {
        let conn = self.get_connection()?;
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM bulk_submissions s
                 WHERE s.tenant_id = ?1 AND s.status = 'in-progress'
                   AND (NOT EXISTS (SELECT 1 FROM bulk_manifests m
                                    WHERE m.tenant_id = s.tenant_id
                                      AND m.submitter = s.submitter
                                      AND m.submission_id = s.submission_id)
                        OR EXISTS (SELECT 1 FROM bulk_manifests m
                                   WHERE m.tenant_id = s.tenant_id
                                     AND m.submitter = s.submitter
                                     AND m.submission_id = s.submission_id
                                     AND m.status IN ('pending', 'processing')))",
                params![tenant.tenant_id().as_str()],
                |r| r.get(0),
            )
            .map_err(|e| internal_error(format!("count active submissions: {e}")))?;
        Ok(count.max(0) as u64)
    }

    async fn ensure_transaction_time(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<DateTime<Utc>> {
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();
        let existing: Option<String> = conn
            .query_row(
                "SELECT transaction_time FROM bulk_submissions
                 WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3",
                params![tenant_id, id.submitter, id.submission_id],
                |r| r.get::<_, Option<String>>(0),
            )
            .map_err(|e| internal_error(format!("read transaction_time: {e}")))?;
        if let Some(ts) = existing.as_deref() {
            if let Ok(dt) = DateTime::parse_from_rfc3339(ts) {
                return Ok(dt.with_timezone(&Utc));
            }
        }
        let now = Utc::now();
        conn.execute(
            "UPDATE bulk_submissions SET transaction_time = ?1
             WHERE tenant_id = ?2 AND submitter = ?3 AND submission_id = ?4",
            params![now.to_rfc3339(), tenant_id, id.submitter, id.submission_id],
        )
        .map_err(|e| internal_error(format!("set transaction_time: {e}")))?;
        Ok(now)
    }
}

impl SqliteBackend {
    /// Atomically replaces and publishes the complete artifact set for a lease.
    ///
    /// The entire attempt—state selection, staged-row replacement, and
    /// terminal fencing update—runs in one immediate transaction. A
    /// successful commit is the only point at which a new generation becomes
    /// visible to future publication queries.
    pub async fn publish_manifest_artifacts(
        &self,
        lease: &ManifestLease,
        files: &[SubmitFileRecord],
        terminal: ManifestPublicationStatus,
    ) -> Result<ManifestPublicationResult, LeaseError> {
        let canonical = canonical_publication_files(files).map_err(LeaseError::Storage)?;
        let outcome = retry_bookkeeping_on_busy(
            "publish manifest artifacts",
            lease_retry_budget(lease),
            || {
                let mut conn = self.get_connection()?;
                let tx = conn
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(|e| {
                        StorageError::Backend(classify_sqlite_error("begin publication", e))
                    })?;
                let Some(outcome) =
                    publish_manifest_artifacts_in_tx(&tx, lease, &canonical, &terminal)?
                else {
                    return Ok(None);
                };
                tx.commit()
                    .map_err(|e| {
                        StorageError::Backend(classify_sqlite_error("commit publication", e))
                    })
                    .map(|()| Some(outcome))
            },
        )
        .await
        .map_err(LeaseError::Storage)?;
        outcome.ok_or_else(|| lease_lost(lease))
    }

    /// Publishes the generation already staged for `lease`. Terminal finish and
    /// failure read the staged set inside the publication transaction, so a
    /// concurrent staging write cannot be omitted from the atomic commit.
    async fn publish_current_manifest_generation(
        &self,
        lease: &ManifestLease,
        terminal: ManifestPublicationStatus,
    ) -> Result<(), LeaseError> {
        let outcome = retry_bookkeeping_on_busy(
            "finish manifest publication",
            lease_retry_budget(lease),
            || {
                let mut conn = self.get_connection()?;
                let tx = conn
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(|e| {
                        StorageError::Backend(classify_sqlite_error("begin finish publication", e))
                    })?;
                let lease_token = i64::try_from(lease.fencing_token).map_err(|_| {
                    internal_error("lease fencing_token does not fit i64".to_string())
                })?;
                let Some(state) = manifest_publication_state(&tx, lease)? else {
                    return Ok(None);
                };
                if state.status == "replaced" || state.fencing_token != lease_token {
                    return Ok(None);
                }

                let staged = if state.status == "processing" {
                    if state.worker_id.as_deref() != Some(lease.worker_id.as_str()) {
                        return Ok(None);
                    }
                    read_manifest_generation_records(&tx, lease, lease_token)?
                } else {
                    let published_token_matches = state.published_token == Some(lease_token);
                    let publisher_matches =
                        state.publication_worker_id.as_deref() == Some(lease.worker_id.as_str());
                    if !published_token_matches || !publisher_matches {
                        return Ok(None);
                    }
                    let (terminal_status, terminal_error) = publication_status_parts(&terminal);
                    if state.status != terminal_status
                        || state.publication_status.as_deref() != Some(terminal_status)
                        || state.publication_error_message != terminal_error.map(str::to_string)
                    {
                        return Err(publication_conflict(
                            "terminal state changed for the same publication",
                        ));
                    }
                    read_manifest_generation_records(&tx, lease, lease_token)?
                };
                let staged = canonical_publication_files(&staged)?;
                let Some(outcome) =
                    publish_manifest_artifacts_in_tx(&tx, lease, &staged, &terminal)?
                else {
                    return Ok(None);
                };
                tx.commit()
                    .map_err(|e| {
                        StorageError::Backend(classify_sqlite_error("commit finish publication", e))
                    })
                    .map(|()| Some(outcome))
            },
        )
        .await
        .map_err(LeaseError::Storage)?;
        outcome.map(|_| ()).ok_or_else(|| lease_lost(lease))
    }
}

#[derive(Debug)]
struct ManifestPublicationState {
    manifest_url: Option<String>,
    status: String,
    fencing_token: i64,
    worker_id: Option<String>,
    published_token: Option<i64>,
    publication_status: Option<String>,
    publication_error_message: Option<String>,
    publication_worker_id: Option<String>,
}

/// Reads the full tenant/submission/manifest publication identity. A missing
/// identity is a lost lease; every SQLite failure is a storage error.
fn manifest_publication_state(
    tx: &rusqlite::Transaction<'_>,
    lease: &ManifestLease,
) -> StorageResult<Option<ManifestPublicationState>> {
    let state = match tx.query_row(
        "SELECT manifest_url, status, fencing_token, worker_id,
                published_token, publication_status, publication_error_message,
                publication_worker_id
         FROM bulk_manifests
         WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3
           AND manifest_id = ?4",
        params![
            lease.tenant.tenant_id().as_str(),
            lease.submission_id.submitter,
            lease.submission_id.submission_id,
            lease.manifest_id,
        ],
        |row| {
            Ok(ManifestPublicationState {
                manifest_url: row.get(0)?,
                status: row.get(1)?,
                fencing_token: row.get(2)?,
                worker_id: row.get(3)?,
                published_token: row.get(4)?,
                publication_status: row.get(5)?,
                publication_error_message: row.get(6)?,
                publication_worker_id: row.get(7)?,
            })
        },
    ) {
        Ok(state) => state,
        Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
        Err(error) => {
            return Err(StorageError::Backend(classify_sqlite_error(
                "read publication state",
                error,
            )));
        }
    };
    Ok(Some(state))
}

fn publication_conflict(message: impl Into<String>) -> StorageError {
    internal_error(format!("manifest publication conflict: {}", message.into()))
}

fn publication_status_parts(terminal: &ManifestPublicationStatus) -> (&'static str, Option<&str>) {
    match terminal {
        ManifestPublicationStatus::Completed => ("completed", None),
        ManifestPublicationStatus::Failed { error_message } => {
            ("failed", Some(error_message.as_str()))
        }
    }
}

/// Reads the artifact generation stored for one exact fencing token. The caller
/// supplies the marker token for replay or the lease token for staged work.
fn read_manifest_generation_records(
    conn: &rusqlite::Connection,
    lease: &ManifestLease,
    fencing_token: i64,
) -> StorageResult<Vec<SubmitFileRecord>> {
    let mut stmt = conn
        .prepare(
            "SELECT manifest_url, file_type, resource_type, part_index, file_path,
                    line_count, byte_count, count_severity
             FROM bulk_submit_files
             WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3
               AND manifest_id = ?4 AND fencing_token = ?5
               AND publication_excluded_reason IS NULL
             ORDER BY id",
        )
        .map_err(|e| StorageError::Backend(classify_sqlite_error("prepare publication rows", e)))?;
    let rows = stmt
        .query_map(
            params![
                lease.tenant.tenant_id().as_str(),
                lease.submission_id.submitter,
                lease.submission_id.submission_id,
                lease.manifest_id,
                fencing_token,
            ],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, Option<String>>(7)?,
                ))
            },
        )
        .map_err(|e| StorageError::Backend(classify_sqlite_error("query publication rows", e)))?;
    let mut records = Vec::new();
    for row in rows {
        let (
            manifest_url,
            file_type,
            resource_type,
            part_index,
            file_path,
            line_count,
            byte_count,
            count_severity,
        ) = row
            .map_err(|e| StorageError::Backend(classify_sqlite_error("read publication row", e)))?;
        records.push(publication_record_from_parts(
            manifest_url,
            file_type,
            resource_type,
            part_index,
            fencing_token,
            file_path,
            line_count,
            byte_count,
            count_severity,
        )?);
    }
    Ok(records)
}

fn validate_publication_records(
    state: &ManifestPublicationState,
    records: &[SubmitFileRecord],
) -> StorageResult<()> {
    for file in records {
        if !matches!(file.file_type.as_str(), "output" | "error" | "deleted") {
            return Err(publication_conflict(format!(
                "invalid file_type: {}",
                file.file_type
            )));
        }
        match (&file.manifest_url, &state.manifest_url) {
            (Some(file_url), Some(manifest_url)) if file_url == manifest_url => {}
            _ => {
                return Err(publication_conflict(
                    "artifact manifest_url does not match the leased manifest",
                ));
            }
        }
    }
    Ok(())
}

/// Converts a persisted publication row to the shared semantic value. Corrupt
/// numeric columns and JSON are storage errors, never silently filtered rows.
#[allow(clippy::too_many_arguments)]
fn publication_record_from_parts(
    manifest_url: Option<String>,
    file_type: String,
    resource_type: Option<String>,
    part_index: i64,
    fencing_token: i64,
    file_path: String,
    line_count: i64,
    byte_count: i64,
    count_severity: Option<String>,
) -> StorageResult<SubmitFileRecord> {
    let count_severity = match count_severity {
        Some(text) => Some(serde_json::from_str::<Value>(&text).map_err(|error| {
            internal_error(format!("decode published count_severity: {error}"))
        })?),
        None => None,
    };
    let part_index = u32::try_from(part_index)
        .map_err(|_| internal_error("published part_index does not fit u32".to_string()))?;
    if fencing_token < 0 {
        return Err(internal_error(
            "published fencing_token is negative".to_string(),
        ));
    }
    let line_count = u64::try_from(line_count)
        .map_err(|_| internal_error("published line_count does not fit u64".to_string()))?;
    let byte_count = u64::try_from(byte_count)
        .map_err(|_| internal_error("published byte_count does not fit u64".to_string()))?;
    Ok(SubmitFileRecord {
        manifest_url,
        file_type,
        resource_type,
        part_index,
        file_path,
        line_count,
        byte_count,
        count_severity,
    })
}

/// Converts a visible artifact row to the shared consumer value. All semantic
/// decoding and range checks are shared with the publication reader.
#[allow(clippy::too_many_arguments)]
fn publication_row_from_parts(
    manifest_url: Option<String>,
    file_type: String,
    resource_type: Option<String>,
    part_index: i64,
    fencing_token: i64,
    file_path: String,
    line_count: i64,
    byte_count: i64,
    count_severity: Option<String>,
    manifest_id: Option<String>,
    legacy_locator: bool,
) -> StorageResult<SubmitFileRow> {
    let record = publication_record_from_parts(
        manifest_url,
        file_type,
        resource_type,
        part_index,
        fencing_token,
        file_path,
        line_count,
        byte_count,
        count_severity,
    )?;
    Ok(SubmitFileRow {
        manifest_url: record.manifest_url,
        file_type: record.file_type,
        resource_type: record.resource_type,
        part_index: record.part_index,
        fencing_token: fencing_token as u64,
        manifest_id,
        legacy_locator,
        file_path: record.file_path,
        line_count: record.line_count,
        byte_count: record.byte_count,
        count_severity: record.count_severity,
    })
}

/// Returns `None` for lease loss and `Some` for a committed publication. All
/// writes remain protected by the transaction's rollback-on-drop behavior.
fn publish_manifest_artifacts_in_tx(
    tx: &rusqlite::Transaction<'_>,
    lease: &ManifestLease,
    canonical: &[SubmitFileRecord],
    terminal: &ManifestPublicationStatus,
) -> StorageResult<Option<ManifestPublicationResult>> {
    let lease_token = i64::try_from(lease.fencing_token)
        .map_err(|_| internal_error("lease fencing_token does not fit i64".to_string()))?;
    let Some(state) = manifest_publication_state(tx, lease)? else {
        return Ok(None);
    };
    if state.status == "replaced" || state.fencing_token != lease_token {
        return Ok(None);
    }

    let published_token_matches = state.published_token == Some(lease_token);
    let publisher_matches =
        state.publication_worker_id.as_deref() == Some(lease.worker_id.as_str());
    if published_token_matches {
        if !publisher_matches {
            return Ok(None);
        }
        let (terminal_status, terminal_error) = publication_status_parts(terminal);
        if state.publication_status.as_deref() != Some(terminal_status)
            || state.status != terminal_status
            || state.publication_error_message != terminal_error.map(str::to_string)
        {
            return Err(publication_conflict(
                "terminal state changed for the same publication",
            ));
        }

        let persisted = read_manifest_generation_records(tx, lease, lease_token)?;
        let persisted = canonical_publication_files(&persisted)?;
        if persisted != canonical {
            return Err(publication_conflict(
                "artifact payload changed for the same publication",
            ));
        }
        return Ok(Some(ManifestPublicationResult::AlreadyPublished));
    }

    // Fresh work must still be owned by this exact lease. An older publication
    // marker is allowed and remains selected until this transaction replaces it.
    if state.status != "processing" || state.worker_id.as_deref() != Some(lease.worker_id.as_str())
    {
        return Ok(None);
    }

    validate_publication_records(&state, canonical)?;

    tx.execute(
        "DELETE FROM bulk_submit_files
         WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3
           AND manifest_id = ?4 AND fencing_token = ?5",
        params![
            lease.tenant.tenant_id().as_str(),
            lease.submission_id.submitter,
            lease.submission_id.submission_id,
            lease.manifest_id,
            lease_token,
        ],
    )
    .map_err(|e| StorageError::Backend(classify_sqlite_error("delete staged files", e)))?;

    for file in canonical {
        let count_severity = match &file.count_severity {
            Some(value) => Some(
                serde_json::to_string(value)
                    .map_err(|error| internal_error(format!("encode count_severity: {error}")))?,
            ),
            None => None,
        };
        tx.execute(
            "INSERT INTO bulk_submit_files
             (tenant_id, submitter, submission_id, manifest_id, manifest_url, file_type,
              resource_type, part_index, fencing_token, file_path, line_count, byte_count,
              count_severity, created_at, publication_excluded_reason)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, NULL)",
            params![
                lease.tenant.tenant_id().as_str(),
                lease.submission_id.submitter,
                lease.submission_id.submission_id,
                lease.manifest_id,
                file.manifest_url,
                file.file_type,
                file.resource_type,
                file.part_index,
                lease_token,
                file.file_path,
                file.line_count,
                file.byte_count,
                count_severity,
                Utc::now().to_rfc3339(),
            ],
        )
        .map_err(|e| StorageError::Backend(classify_sqlite_error("insert publication file", e)))?;
    }

    let (terminal_status, terminal_error) = publication_status_parts(terminal);
    let affected = tx
        .execute(
            "UPDATE bulk_manifests
             SET status = ?5, published_token = ?6, publication_status = ?7,
                 publication_error_message = ?8, publication_worker_id = ?9,
                 worker_id = NULL, lease_expiry = NULL
             WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3
               AND manifest_id = ?4 AND status = 'processing'
               AND worker_id = ?10 AND fencing_token = ?11",
            params![
                lease.tenant.tenant_id().as_str(),
                lease.submission_id.submitter,
                lease.submission_id.submission_id,
                lease.manifest_id,
                terminal_status,
                lease_token,
                terminal_status,
                terminal_error,
                lease.worker_id.as_str(),
                lease.worker_id.as_str(),
                lease_token,
            ],
        )
        .map_err(|e| StorageError::Backend(classify_sqlite_error("publish manifest", e)))?;
    if affected != 1 {
        return Ok(None);
    }
    Ok(Some(ManifestPublicationResult::Published))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::bulk_submit::CancelToken;
    use crate::tenant::{TenantId, TenantPermissions};
    use serde_json::json;

    mod paging_contract {
        use crate as persistence;
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/bulk_submit/paging_contract.rs"
        ));
    }

    mod consumer_contract {
        use crate as persistence;
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/bulk_submit/consumer_contract.rs"
        ));
    }

    mod publication {
        use super::*;
        use crate::core::bulk_submit_publication::ManifestPublicationResult;
        use crate::core::bulk_submit_publication::ManifestPublicationStatus;

        fn publication_file(
            manifest_url: &str,
            file_type: &str,
            resource_type: Option<&str>,
            part_index: u32,
            file_path: &str,
            count_severity: Option<Value>,
        ) -> SubmitFileRecord {
            SubmitFileRecord {
                manifest_url: Some(manifest_url.to_string()),
                file_type: file_type.to_string(),
                resource_type: resource_type.map(str::to_string),
                part_index,
                file_path: file_path.to_string(),
                line_count: 1,
                byte_count: 16,
                count_severity,
            }
        }

        async fn claim(
            backend: &SqliteBackend,
            manifest_url: &str,
        ) -> (TenantContext, SubmissionId, ManifestLease) {
            let tenant = create_test_tenant();
            let submission = SubmissionId::generate("publication");
            backend
                .create_submission(&tenant, &submission, None)
                .await
                .unwrap();
            backend
                .add_manifest(&tenant, &submission, Some(manifest_url), None)
                .await
                .unwrap();
            let lease = backend
                .claim_next_manifest(&WorkerId::new("publisher"), StdDuration::from_secs(60))
                .await
                .unwrap()
                .unwrap();
            (tenant, submission, lease)
        }

        pub(super) fn publication_row_count(
            backend: &SqliteBackend,
            tenant: &TenantContext,
            submission: &SubmissionId,
            manifest_id: &str,
        ) -> i64 {
            backend
                .get_connection()
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM bulk_submit_files
                     WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3
                       AND manifest_id = ?4",
                    params![
                        tenant.tenant_id().as_str(),
                        submission.submitter,
                        submission.submission_id,
                        manifest_id,
                    ],
                    |row| row.get(0),
                )
                .unwrap()
        }

        #[allow(clippy::type_complexity)] // Exact persisted columns for rollback assertions.
        pub(super) fn publication_marker(
            backend: &SqliteBackend,
            tenant: &TenantContext,
            submission: &SubmissionId,
            manifest_id: &str,
        ) -> (
            String,
            Option<i64>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
        ) {
            backend
                .get_connection()
                .unwrap()
                .query_row(
                    "SELECT status, published_token, publication_status,
                            publication_error_message, publication_worker_id, worker_id
                     FROM bulk_manifests
                     WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3
                       AND manifest_id = ?4",
                    params![
                        tenant.tenant_id().as_str(),
                        submission.submitter,
                        submission.submission_id,
                        manifest_id,
                    ],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                            row.get(5)?,
                        ))
                    },
                )
                .unwrap()
        }

        #[tokio::test]
        async fn complete_publication_replays_exactly_and_rejects_changes() {
            let backend = create_test_backend();
            let manifest_url = "http://provider/complete.json";
            let (tenant, submission, lease) = claim(&backend, manifest_url).await;
            let output = publication_file(
                manifest_url,
                "output",
                Some("Patient"),
                0,
                "output/patient-0.ndjson",
                None,
            );
            let error = publication_file(
                manifest_url,
                "error",
                Some("OperationOutcome"),
                0,
                "error/outcome-0.ndjson",
                Some(json!({"error": 1})),
            );

            assert_eq!(
                backend
                    .publish_manifest_artifacts(
                        &lease,
                        &[output.clone(), error.clone()],
                        ManifestPublicationStatus::Completed,
                    )
                    .await
                    .unwrap(),
                ManifestPublicationResult::Published
            );
            let (
                status,
                published_token,
                publication_status,
                publication_error,
                publication_worker,
                worker,
            ) = publication_marker(&backend, &tenant, &submission, &lease.manifest_id);
            assert_eq!(status, "completed");
            assert_eq!(published_token, Some(lease.fencing_token as i64));
            assert_eq!(publication_status.as_deref(), Some("completed"));
            assert_eq!(publication_error, None);
            assert_eq!(
                publication_worker.as_deref(),
                Some(lease.worker_id.as_str())
            );
            assert_eq!(worker, None);
            assert_eq!(
                publication_row_count(&backend, &tenant, &submission, &lease.manifest_id),
                2
            );

            assert_eq!(
                backend
                    .publish_manifest_artifacts(
                        &lease,
                        &[error.clone(), output.clone()],
                        ManifestPublicationStatus::Completed,
                    )
                    .await
                    .unwrap(),
                ManifestPublicationResult::AlreadyPublished
            );

            let mut changed_payload = output.clone();
            changed_payload.file_path = "output/patient-changed.ndjson".to_string();
            assert!(matches!(
                backend
                    .publish_manifest_artifacts(
                        &lease,
                        &[changed_payload, error.clone()],
                        ManifestPublicationStatus::Completed,
                    )
                    .await,
                Err(LeaseError::Storage(_))
            ));
            let mut changed_status = error.clone();
            changed_status.count_severity = Some(json!({"error": 2}));
            assert!(matches!(
                backend
                    .publish_manifest_artifacts(
                        &lease,
                        &[output.clone(), changed_status],
                        ManifestPublicationStatus::Failed {
                            error_message: "changed".to_string(),
                        },
                    )
                    .await,
                Err(LeaseError::Storage(_))
            ));
            assert_eq!(
                publication_row_count(&backend, &tenant, &submission, &lease.manifest_id),
                2
            );
            let (status, _, publication_status, _, _, _) =
                publication_marker(&backend, &tenant, &submission, &lease.manifest_id);
            assert_eq!(status, "completed");
            assert_eq!(publication_status.as_deref(), Some("completed"));
        }

        #[tokio::test]
        async fn replaced_manifest_process_entries_does_not_revive_publication() {
            let backend = create_test_backend();
            let tenant = create_test_tenant();
            let submission = SubmissionId::generate("replacement-guard");
            let manifest_url = "http://provider/replacement-guard.json";
            backend
                .create_submission(&tenant, &submission, None)
                .await
                .unwrap();
            let manifest = backend
                .add_manifest(&tenant, &submission, Some(manifest_url), None)
                .await
                .unwrap();
            let lease = backend
                .claim_next_manifest(&WorkerId::new("publisher"), StdDuration::from_secs(60))
                .await
                .unwrap()
                .unwrap();
            backend
                .replace_manifest_by_url(&tenant, &submission, manifest_url)
                .await
                .unwrap();

            backend
                .process_entries(
                    &tenant,
                    &submission,
                    &manifest.manifest_id,
                    Vec::new(),
                    &BulkProcessingOptions::new(),
                )
                .await
                .unwrap();
            let manifest = backend
                .get_manifest(&tenant, &submission, &manifest.manifest_id)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                manifest.status,
                ManifestStatus::Replaced,
                "an empty ingest must not revive a replaced manifest"
            );
            assert!(
                matches!(
                    backend
                        .publish_manifest_artifacts(
                            &lease,
                            &[],
                            ManifestPublicationStatus::Completed,
                        )
                        .await,
                    Err(LeaseError::LeaseLost { .. })
                ),
                "the old lease must lose publication after replacement"
            );
        }

        #[tokio::test]
        async fn empty_failed_publication_replays_exactly_and_rejects_new_message() {
            let backend = create_test_backend();
            let manifest_url = "http://provider/empty.json";
            let (tenant, submission, lease) = claim(&backend, manifest_url).await;
            assert_eq!(
                backend
                    .publish_manifest_artifacts(
                        &lease,
                        &[],
                        ManifestPublicationStatus::Failed {
                            error_message: "remote manifest unavailable".to_string(),
                        },
                    )
                    .await
                    .unwrap(),
                ManifestPublicationResult::Published
            );
            let (
                status,
                published_token,
                publication_status,
                publication_error,
                publication_worker,
                _,
            ) = publication_marker(&backend, &tenant, &submission, &lease.manifest_id);
            assert_eq!(status, "failed");
            assert_eq!(published_token, Some(lease.fencing_token as i64));
            assert_eq!(publication_status.as_deref(), Some("failed"));
            assert_eq!(
                publication_error.as_deref(),
                Some("remote manifest unavailable")
            );
            assert_eq!(
                publication_worker.as_deref(),
                Some(lease.worker_id.as_str())
            );
            assert_eq!(
                publication_row_count(&backend, &tenant, &submission, &lease.manifest_id),
                0
            );

            assert_eq!(
                backend
                    .publish_manifest_artifacts(
                        &lease,
                        &[],
                        ManifestPublicationStatus::Failed {
                            error_message: "remote manifest unavailable".to_string(),
                        },
                    )
                    .await
                    .unwrap(),
                ManifestPublicationResult::AlreadyPublished
            );
            assert!(matches!(
                backend
                    .publish_manifest_artifacts(
                        &lease,
                        &[],
                        ManifestPublicationStatus::Failed {
                            error_message: "changed".to_string(),
                        },
                    )
                    .await,
                Err(LeaseError::Storage(_))
            ));
            let (status, _, publication_status, publication_error, _, _) =
                publication_marker(&backend, &tenant, &submission, &lease.manifest_id);
            assert_eq!(status, "failed");
            assert_eq!(publication_status.as_deref(), Some("failed"));
            assert_eq!(
                publication_error.as_deref(),
                Some("remote manifest unavailable")
            );
        }

        #[tokio::test]
        async fn stale_wrong_worker_and_replaced_leases_are_lost() {
            let backend = create_test_backend();
            let terminal = ManifestPublicationStatus::Completed;

            let stale_url = "http://provider/stale.json";
            let (_, _, stale) = claim(&backend, stale_url).await;
            backend
                .get_connection()
                .unwrap()
                .execute(
                    "UPDATE bulk_manifests SET lease_expiry = '1970-01-01T00:00:00Z'
                     WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3
                       AND manifest_id = ?4",
                    params![
                        create_test_tenant().tenant_id().as_str(),
                        stale.submission_id.submitter,
                        stale.submission_id.submission_id,
                        stale.manifest_id,
                    ],
                )
                .unwrap();
            let _fresh = backend
                .claim_next_manifest(&WorkerId::new("takeover"), StdDuration::from_secs(60))
                .await
                .unwrap()
                .unwrap();
            assert!(matches!(
                backend
                    .publish_manifest_artifacts(&stale, &[], terminal.clone())
                    .await,
                Err(LeaseError::LeaseLost { .. })
            ));

            let wrong_url = "http://provider/wrong-worker.json";
            let (tenant, submission, lease) = claim(&backend, wrong_url).await;
            backend
                .get_connection()
                .unwrap()
                .execute(
                    "UPDATE bulk_manifests SET worker_id = 'interloper'
                     WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3
                       AND manifest_id = ?4",
                    params![
                        tenant.tenant_id().as_str(),
                        submission.submitter,
                        submission.submission_id,
                        lease.manifest_id,
                    ],
                )
                .unwrap();
            assert!(matches!(
                backend
                    .publish_manifest_artifacts(&lease, &[], terminal.clone())
                    .await,
                Err(LeaseError::LeaseLost { .. })
            ));

            let replaced_url = "http://provider/replaced.json";
            let (tenant, submission, lease) = claim(&backend, replaced_url).await;
            backend
                .get_connection()
                .unwrap()
                .execute(
                    "UPDATE bulk_manifests SET status = 'replaced'
                     WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3
                       AND manifest_id = ?4",
                    params![
                        tenant.tenant_id().as_str(),
                        submission.submitter,
                        submission.submission_id,
                        lease.manifest_id,
                    ],
                )
                .unwrap();
            assert!(matches!(
                backend
                    .publish_manifest_artifacts(&lease, &[], terminal)
                    .await,
                Err(LeaseError::LeaseLost { .. })
            ));
            assert_eq!(
                publication_row_count(&backend, &tenant, &submission, &lease.manifest_id),
                0
            );
        }

        #[tokio::test]
        async fn guard_change_after_insert_rolls_back_artifacts_and_marker() {
            let backend = create_test_backend();
            let manifest_url = "http://provider/injected.json";
            let (tenant, submission, lease) = claim(&backend, manifest_url).await;
            let file = publication_file(
                manifest_url,
                "output",
                Some("Patient"),
                0,
                "output/patient-0.ndjson",
                None,
            );
            backend
                .get_connection()
                .unwrap()
                .execute(
                    "CREATE TRIGGER publication_guard_change
                     AFTER INSERT ON bulk_submit_files
                     BEGIN
                       UPDATE bulk_manifests
                       SET worker_id = 'interloper', fencing_token = 999
                       WHERE tenant_id = NEW.tenant_id
                         AND submitter = NEW.submitter
                         AND submission_id = NEW.submission_id
                         AND manifest_id = NEW.manifest_id;
                     END",
                    [],
                )
                .unwrap();

            assert!(matches!(
                backend
                    .publish_manifest_artifacts(
                        &lease,
                        &[file],
                        ManifestPublicationStatus::Completed,
                    )
                    .await,
                Err(LeaseError::LeaseLost { .. })
            ));
            assert_eq!(
                publication_row_count(&backend, &tenant, &submission, &lease.manifest_id),
                0
            );
            let (
                status,
                published_token,
                publication_status,
                publication_error,
                publication_worker,
                worker,
            ) = publication_marker(&backend, &tenant, &submission, &lease.manifest_id);
            assert_eq!(status, "processing");
            assert_eq!(published_token, None);
            assert_eq!(publication_status, None);
            assert_eq!(publication_error, None);
            assert_eq!(publication_worker, None);
            assert_eq!(worker.as_deref(), Some(lease.worker_id.as_str()));
        }

        async fn stage_output(
            backend: &SqliteBackend,
            lease: &ManifestLease,
            manifest_url: &str,
            file_path: &str,
        ) {
            backend
                .record_submit_file(
                    lease,
                    &publication_file(manifest_url, "output", Some("Patient"), 0, file_path, None),
                )
                .await
                .unwrap();
        }

        fn visible_paths(rows: &[SubmitFileRow]) -> Vec<String> {
            rows.iter().map(|row| row.file_path.clone()).collect()
        }

        #[tokio::test]
        async fn visible_artifacts_are_scoped_by_manifest_tenant_and_marker() {
            let backend = create_test_backend();
            let first_tenant = TenantContext::new(
                TenantId::new("scope-tenant-one"),
                TenantPermissions::full_access(),
            );
            let second_tenant = TenantContext::new(
                TenantId::new("scope-tenant-two"),
                TenantPermissions::full_access(),
            );
            let submission = SubmissionId::new("scope-submitter", "shared-submission");
            let manifest_id = "shared-manifest-id";

            for tenant in [&first_tenant, &second_tenant] {
                backend
                    .create_submission(tenant, &submission, None)
                    .await
                    .unwrap();
                let manifest = backend
                    .add_manifest(
                        tenant,
                        &submission,
                        Some("http://provider/shared.json"),
                        None,
                    )
                    .await
                    .unwrap();
                backend
                    .get_connection()
                    .unwrap()
                    .execute(
                        "UPDATE bulk_manifests SET manifest_id = ?1, added_at = ?5
                         WHERE tenant_id = ?2 AND submitter = ?3 AND submission_id = ?4",
                        params![
                            manifest_id,
                            tenant.tenant_id().as_str(),
                            submission.submitter,
                            submission.submission_id,
                            if tenant.tenant_id() == first_tenant.tenant_id() {
                                "1970-01-01T00:00:00Z"
                            } else {
                                "2000-01-01T00:00:00Z"
                            }
                        ],
                    )
                    .unwrap();
                if tenant.tenant_id() == second_tenant.tenant_id() {
                    backend
                        .get_connection()
                        .unwrap()
                        .execute(
                            "UPDATE bulk_manifests SET added_at = '2000-01-01T00:00:00Z'
                             WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3",
                            params![
                                tenant.tenant_id().as_str(),
                                submission.submitter,
                                submission.submission_id
                            ],
                        )
                        .unwrap();
                }
                let _ = manifest;
            }

            let first = backend
                .claim_next_manifest(&WorkerId::new("scope-first"), StdDuration::from_secs(60))
                .await
                .unwrap()
                .unwrap();
            let second = backend
                .claim_next_manifest(&WorkerId::new("scope-second"), StdDuration::from_secs(60))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(first.manifest_id, manifest_id);
            assert_eq!(second.manifest_id, manifest_id);
            assert_eq!(first.fencing_token, second.fencing_token);

            stage_output(
                &backend,
                &first,
                "http://provider/shared.json",
                "tenant-one/output.ndjson",
            )
            .await;
            assert!(
                backend
                    .list_submit_files(&first_tenant, &first.submission_id)
                    .await
                    .unwrap()
                    .is_empty()
            );
            assert!(
                backend
                    .list_submit_files(&second_tenant, &second.submission_id)
                    .await
                    .unwrap()
                    .is_empty()
            );

            // A token without a complete publication marker is still invisible,
            // even when the row itself has the expected manifest and fence.
            stage_output(
                &backend,
                &second,
                "http://provider/shared.json",
                "tenant-two/output.ndjson",
            )
            .await;
            backend
                .get_connection()
                .unwrap()
                .execute(
                    "UPDATE bulk_manifests SET published_token = 1
                     WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3
                       AND manifest_id = ?4",
                    params![
                        second_tenant.tenant_id().as_str(),
                        second.submission_id.submitter,
                        second.submission_id.submission_id,
                        manifest_id
                    ],
                )
                .unwrap();
            assert!(
                backend
                    .list_submit_files(&second_tenant, &second.submission_id)
                    .await
                    .unwrap()
                    .is_empty()
            );
            backend
                .get_connection()
                .unwrap()
                .execute(
                    "UPDATE bulk_manifests SET published_token = NULL
                     WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3
                       AND manifest_id = ?4",
                    params![
                        second_tenant.tenant_id().as_str(),
                        second.submission_id.submitter,
                        second.submission_id.submission_id,
                        manifest_id
                    ],
                )
                .unwrap();

            assert_eq!(
                backend
                    .publish_manifest_artifacts(
                        &first,
                        &[publication_file(
                            "http://provider/shared.json",
                            "output",
                            Some("Patient"),
                            0,
                            "tenant-one/output.ndjson",
                            None,
                        )],
                        ManifestPublicationStatus::Completed,
                    )
                    .await
                    .unwrap(),
                ManifestPublicationResult::Published
            );
            assert_eq!(
                visible_paths(
                    &backend
                        .list_submit_files(&first_tenant, &first.submission_id)
                        .await
                        .unwrap()
                ),
                ["tenant-one/output.ndjson"]
            );
            assert!(
                backend
                    .list_submit_files(&second_tenant, &second.submission_id)
                    .await
                    .unwrap()
                    .is_empty()
            );

            assert_eq!(
                backend
                    .publish_manifest_artifacts(
                        &second,
                        &[publication_file(
                            "http://provider/shared.json",
                            "output",
                            Some("Patient"),
                            0,
                            "tenant-two/output.ndjson",
                            None,
                        )],
                        ManifestPublicationStatus::Completed,
                    )
                    .await
                    .unwrap(),
                ManifestPublicationResult::Published
            );
            assert_eq!(
                visible_paths(
                    &backend
                        .list_submit_files(&second_tenant, &second.submission_id)
                        .await
                        .unwrap()
                ),
                ["tenant-two/output.ndjson"]
            );
            assert_eq!(
                visible_paths(
                    &backend
                        .list_submit_files(&first_tenant, &first.submission_id)
                        .await
                        .unwrap()
                ),
                ["tenant-one/output.ndjson"]
            );
        }

        #[tokio::test]
        async fn new_same_manifest_generation_hides_old_but_preserves_rows() {
            let backend = create_test_backend();
            let tenant = create_test_tenant();
            let submission = SubmissionId::generate("generations");
            backend
                .create_submission(&tenant, &submission, None)
                .await
                .unwrap();
            let manifest_url = "http://provider/generation.json";
            backend
                .add_manifest(&tenant, &submission, Some(manifest_url), None)
                .await
                .unwrap();

            let old = backend
                .claim_next_manifest(&WorkerId::new("generation-old"), StdDuration::from_secs(60))
                .await
                .unwrap()
                .unwrap();
            stage_output(&backend, &old, manifest_url, "generation/old.ndjson").await;
            backend.finish_manifest(&old).await.unwrap();
            assert_eq!(
                visible_paths(
                    &backend
                        .list_submit_files(&tenant, &submission)
                        .await
                        .unwrap()
                ),
                ["generation/old.ndjson"]
            );

            // Simulate the same manifest becoming reclaimable while retaining
            // the token-one publication marker. The marker continues to select
            // generation one until generation two commits.
            backend
                .get_connection()
                .unwrap()
                .execute(
                    "UPDATE bulk_manifests SET status = 'pending', worker_id = NULL,
                            lease_expiry = NULL
                     WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3
                       AND manifest_id = ?4",
                    params![
                        tenant.tenant_id().as_str(),
                        submission.submitter,
                        submission.submission_id,
                        old.manifest_id
                    ],
                )
                .unwrap();
            let new = backend
                .claim_next_manifest(&WorkerId::new("generation-new"), StdDuration::from_secs(60))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(new.manifest_id, old.manifest_id);
            assert!(new.fencing_token > old.fencing_token);
            stage_output(&backend, &new, manifest_url, "generation/new.ndjson").await;
            assert_eq!(
                visible_paths(
                    &backend
                        .list_submit_files(&tenant, &submission)
                        .await
                        .unwrap()
                ),
                ["generation/old.ndjson"]
            );

            backend.finish_manifest(&new).await.unwrap();
            let rows = backend
                .list_submit_files(&tenant, &submission)
                .await
                .unwrap();
            assert_eq!(visible_paths(&rows), ["generation/new.ndjson"]);
            assert_eq!(
                publication_row_count(&backend, &tenant, &submission, &new.manifest_id),
                2
            );
        }

        #[tokio::test]
        async fn same_token_and_artifact_identity_are_independent_by_manifest() {
            let backend = create_test_backend();
            let tenant = create_test_tenant();
            let submission = SubmissionId::generate("independent");
            backend
                .create_submission(&tenant, &submission, None)
                .await
                .unwrap();
            let first_url = "http://provider/independent-one.json";
            let second_url = "http://provider/independent-two.json";
            backend
                .add_manifest(&tenant, &submission, Some(first_url), None)
                .await
                .unwrap();
            backend
                .add_manifest(&tenant, &submission, Some(second_url), None)
                .await
                .unwrap();

            let first = backend
                .claim_next_manifest(
                    &WorkerId::new("independent-one"),
                    StdDuration::from_secs(60),
                )
                .await
                .unwrap()
                .unwrap();
            stage_output(&backend, &first, first_url, "independent/first.ndjson").await;
            backend.finish_manifest(&first).await.unwrap();

            let second = backend
                .claim_next_manifest(
                    &WorkerId::new("independent-two"),
                    StdDuration::from_secs(60),
                )
                .await
                .unwrap()
                .unwrap();
            stage_output(&backend, &second, second_url, "independent/second.ndjson").await;
            assert_eq!(
                visible_paths(
                    &backend
                        .list_submit_files(&tenant, &submission)
                        .await
                        .unwrap()
                ),
                ["independent/first.ndjson"]
            );

            backend.finish_manifest(&second).await.unwrap();
            let rows = backend
                .list_submit_files(&tenant, &submission)
                .await
                .unwrap();
            assert_eq!(
                visible_paths(&rows),
                ["independent/first.ndjson", "independent/second.ndjson"]
            );
            // Publication is per manifest. The same lease identity is valid
            // independently under each manifest, and both published sets stay
            // visible to the consumer.
            assert_eq!(
                publication_row_count(&backend, &tenant, &submission, &first.manifest_id),
                1
            );
            assert_eq!(
                publication_row_count(&backend, &tenant, &submission, &second.manifest_id),
                1
            );
            assert!(rows.iter().all(|row| row.fencing_token == 1));
            assert!(
                rows.iter()
                    .all(|row| row.resource_type.as_deref() == Some("Patient")
                        && row.part_index == 0)
            );
        }

        #[tokio::test]
        async fn expired_lease_can_publish_when_not_reclaimed() {
            let backend = create_test_backend();
            let manifest_url = "http://provider/expired.json";
            let (tenant, submission, lease) = claim(&backend, manifest_url).await;
            stage_output(&backend, &lease, manifest_url, "expired/output.ndjson").await;
            backend
                .get_connection()
                .unwrap()
                .execute(
                    "UPDATE bulk_manifests SET lease_expiry = '1970-01-01T00:00:00Z'
                     WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3
                       AND manifest_id = ?4",
                    params![
                        tenant.tenant_id().as_str(),
                        submission.submitter,
                        submission.submission_id,
                        lease.manifest_id
                    ],
                )
                .unwrap();

            backend.finish_manifest(&lease).await.unwrap();
            assert_eq!(
                visible_paths(
                    &backend
                        .list_submit_files(&tenant, &submission)
                        .await
                        .unwrap()
                ),
                ["expired/output.ndjson"]
            );
        }

        #[tokio::test]
        async fn cleanup_blocks_exact_publication_replay() {
            let backend = create_test_backend();
            let manifest_url = "http://provider/cleanup-replay.json";
            let (tenant, submission, lease) = claim(&backend, manifest_url).await;
            stage_output(&backend, &lease, manifest_url, "cleanup/output.ndjson").await;
            backend.finish_manifest(&lease).await.unwrap();
            assert_eq!(
                publication_row_count(&backend, &tenant, &submission, &lease.manifest_id),
                1
            );

            backend
                .delete_submission_artifacts(&tenant, &submission)
                .await
                .unwrap();
            assert!(
                backend
                    .list_submit_files(&tenant, &submission)
                    .await
                    .unwrap()
                    .is_empty()
            );
            let (
                status,
                published_token,
                publication_status,
                publication_error,
                publication_worker,
                _,
            ) = publication_marker(&backend, &tenant, &submission, &lease.manifest_id);
            assert_eq!(status, "completed");
            assert_eq!(published_token, None);
            assert_eq!(publication_status, None);
            assert_eq!(publication_error, None);
            assert_eq!(publication_worker, None);

            assert!(matches!(
                backend.finish_manifest(&lease).await,
                Err(LeaseError::LeaseLost { .. })
            ));
            assert!(
                backend
                    .list_submit_files(&tenant, &submission)
                    .await
                    .unwrap()
                    .is_empty()
            );
        }

        #[tokio::test]
        async fn publication_busy_timeout_retry_and_concurrent_replay() {
            use rusqlite::Connection;
            use std::sync::mpsc::channel;

            use crate::backends::sqlite::SqliteBackendConfig;

            let dir = tempfile::tempdir().unwrap();
            let config = SqliteBackendConfig {
                max_connections: 2,
                busy_timeout_ms: 20,
                ..Default::default()
            };
            let backend =
                SqliteBackend::with_config(dir.path().join("publication-busy.db"), config).unwrap();
            backend.init_schema().unwrap();
            let manifest_url = "http://provider/busy-publication.json";
            let (tenant, submission, lease) = claim(&backend, manifest_url).await;
            let output = publication_file(
                manifest_url,
                "output",
                Some("Patient"),
                0,
                "output/patient-0.ndjson",
                None,
            );
            let error = publication_file(
                manifest_url,
                "error",
                Some("OperationOutcome"),
                0,
                "error/outcome-0.ndjson",
                Some(json!({"error": 1})),
            );
            let set = [output.clone(), error.clone()];

            let (held_tx, held_rx) = channel();
            let lock_path = dir.path().join("publication-busy.db");
            let lock = std::thread::spawn(move || {
                let conn = Connection::open(lock_path).unwrap();
                conn.execute_batch("BEGIN IMMEDIATE").unwrap();
                held_tx.send(()).unwrap();
                std::thread::sleep(std::time::Duration::from_millis(150));
                conn.execute_batch("COMMIT").unwrap();
            });
            held_rx.recv().unwrap();

            let started = std::time::Instant::now();
            let (first, second) = tokio::join!(
                backend.publish_manifest_artifacts(
                    &lease,
                    &set,
                    ManifestPublicationStatus::Completed,
                ),
                backend.publish_manifest_artifacts(
                    &lease,
                    &set,
                    ManifestPublicationStatus::Completed,
                ),
            );
            let results = [first.unwrap(), second.unwrap()];
            assert!(started.elapsed() >= std::time::Duration::from_millis(50));
            assert_eq!(
                results
                    .iter()
                    .filter(|result| **result == ManifestPublicationResult::Published)
                    .count(),
                1
            );
            assert_eq!(
                results
                    .iter()
                    .filter(|result| **result == ManifestPublicationResult::AlreadyPublished)
                    .count(),
                1
            );
            lock.join().unwrap();
            let (
                status,
                published_token,
                publication_status,
                publication_error,
                publication_worker,
                _,
            ) = publication_marker(&backend, &tenant, &submission, &lease.manifest_id);
            assert_eq!(status, "completed");
            assert_eq!(published_token, Some(lease.fencing_token as i64));
            assert_eq!(publication_status.as_deref(), Some("completed"));
            assert_eq!(publication_error, None);
            assert_eq!(
                publication_worker.as_deref(),
                Some(lease.worker_id.as_str())
            );
            assert_eq!(
                publication_row_count(&backend, &tenant, &submission, &lease.manifest_id),
                2
            );
            assert_eq!(
                visible_paths(
                    &backend
                        .list_submit_files(&tenant, &submission)
                        .await
                        .unwrap()
                ),
                ["error/outcome-0.ndjson", "output/patient-0.ndjson"]
            );
        }

        #[tokio::test]
        async fn publication_cleanup_marker_fault_rolls_back_after_delete() {
            let backend = create_test_backend();
            let manifest_url = "http://provider/cleanup-fault.json";
            let (tenant, submission, lease) = claim(&backend, manifest_url).await;
            let output = publication_file(
                manifest_url,
                "output",
                Some("Patient"),
                0,
                "output/patient-0.ndjson",
                None,
            );
            let error = publication_file(
                manifest_url,
                "error",
                Some("OperationOutcome"),
                0,
                "error/outcome-0.ndjson",
                Some(json!({"error": 1})),
            );
            assert_eq!(
                backend
                    .publish_manifest_artifacts(
                        &lease,
                        &[output.clone(), error.clone()],
                        ManifestPublicationStatus::Completed,
                    )
                    .await
                    .unwrap(),
                ManifestPublicationResult::Published
            );
            let marker_before =
                publication_marker(&backend, &tenant, &submission, &lease.manifest_id);
            backend
                .get_connection()
                .unwrap()
                .execute(
                    "CREATE TRIGGER publication_cleanup_marker_fault
                     BEFORE UPDATE OF published_token ON bulk_manifests
                     WHEN (OLD.published_token IS NOT NULL AND NEW.published_token IS NULL)
                     BEGIN
                       SELECT RAISE(ABORT, 'cleanup marker fault');
                     END",
                    [],
                )
                .unwrap();

            let cleanup_error = backend
                .delete_submission_artifacts(&tenant, &submission)
                .await
                .unwrap_err();
            assert!(
                cleanup_error
                    .to_string()
                    .contains("clear publication markers"),
                "cleanup must fail while clearing the marker, got {cleanup_error}"
            );
            assert_eq!(
                publication_row_count(&backend, &tenant, &submission, &lease.manifest_id),
                2
            );
            let marker_after =
                publication_marker(&backend, &tenant, &submission, &lease.manifest_id);
            assert_eq!(marker_before, marker_after);
            assert_eq!(
                visible_paths(
                    &backend
                        .list_submit_files(&tenant, &submission)
                        .await
                        .unwrap()
                ),
                ["error/outcome-0.ndjson", "output/patient-0.ndjson"]
            );

            backend
                .get_connection()
                .unwrap()
                .execute("DROP TRIGGER publication_cleanup_marker_fault", [])
                .unwrap();
            backend
                .delete_submission_artifacts(&tenant, &submission)
                .await
                .unwrap();
            assert_eq!(
                publication_row_count(&backend, &tenant, &submission, &lease.manifest_id),
                0
            );
            assert!(
                backend
                    .list_submit_files(&tenant, &submission)
                    .await
                    .unwrap()
                    .is_empty()
            );
            assert!(matches!(
                backend
                    .publish_manifest_artifacts(
                        &lease,
                        &[output, error],
                        ManifestPublicationStatus::Completed,
                    )
                    .await,
                Err(LeaseError::LeaseLost { .. })
            ));
        }

        #[tokio::test]
        async fn replaced_manifest_cannot_preflight_or_revive_processing() {
            let backend = create_test_backend();
            let manifest_url = "http://provider/replaced-guard.json";
            let (tenant, submission, lease) = claim(&backend, manifest_url).await;
            backend
                .get_connection()
                .unwrap()
                .execute(
                    "UPDATE bulk_manifests SET status = 'replaced'
                     WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3
                       AND manifest_id = ?4",
                    params![
                        tenant.tenant_id().as_str(),
                        submission.submitter,
                        submission.submission_id,
                        lease.manifest_id
                    ],
                )
                .unwrap();

            assert!(matches!(
                backend.heartbeat(&lease).await,
                Err(LeaseError::LeaseLost { .. })
            ));
            assert!(matches!(
                backend.get_manifest_for_worker(&lease).await,
                Err(LeaseError::LeaseLost { .. })
            ));
            assert!(matches!(
                backend.mark_manifest_processing(&lease).await,
                Err(LeaseError::LeaseLost { .. })
            ));
            let (status, _, _, _, _, worker) =
                publication_marker(&backend, &tenant, &submission, &lease.manifest_id);
            assert_eq!(status, "replaced");
            assert_eq!(worker.as_deref(), Some(lease.worker_id.as_str()));
        }
    }

    #[tokio::test]
    async fn bulk_submit_worker_exact_artifacts_across_pages() {
        consumer_contract::worker_receipts(
            std::sync::Arc::new(create_test_backend()),
            &create_test_tenant(),
        )
        .await;
    }

    #[tokio::test]
    async fn bulk_submit_composite_deduplicates_all_pages_on_finish_and_failure() {
        for (fail, secondary_failure) in
            [(false, false), (true, false), (false, true), (true, true)]
        {
            consumer_contract::composite_receipts(
                std::sync::Arc::new(create_test_backend()),
                &create_test_tenant(),
                crate::core::BackendKind::Sqlite,
                fail,
                secondary_failure,
            )
            .await;
        }
    }

    #[async_trait]
    impl paging_contract::ReceiptFixture for SqliteBackend {
        async fn seed_receipts(
            &self,
            tenant: &TenantContext,
            submission: &SubmissionId,
            manifest: &str,
            rows: &[paging_contract::ReceiptRow],
        ) {
            let conn = self.get_connection().unwrap();
            let tid = tenant.tenant_id().as_str();
            conn.execute("INSERT OR IGNORE INTO bulk_submissions (tenant_id,submitter,submission_id,status,created_at,updated_at) VALUES (?1,?2,?3,'complete',datetime('now'),datetime('now'))", params![tid, submission.submitter, submission.submission_id]).unwrap();
            conn.execute("INSERT OR IGNORE INTO bulk_manifests (tenant_id,submitter,submission_id,manifest_id,status,added_at) VALUES (?1,?2,?3,?4,'completed',datetime('now'))", params![tid, submission.submitter, submission.submission_id, manifest]).unwrap();
            for row in rows {
                conn.execute("INSERT INTO bulk_entry_results (tenant_id,submitter,submission_id,manifest_id,file_url,line_number,resource_type,resource_id,outcome) VALUES (?1,?2,?3,?4,?5,?6,'Patient',?7,?8)", params![tid, submission.submitter, submission.submission_id, manifest, row.file, row.line, row.id, row.outcome]).unwrap();
            }
        }
    }

    #[tokio::test]
    async fn bulk_submit_exact_keyset_pages() {
        paging_contract::exact_sql_pages(&create_test_backend(), &create_test_tenant(), i64::MAX)
            .await;
    }

    #[tokio::test]
    async fn bulk_submit_corrupt_receipt_is_an_error_not_a_short_page() {
        use paging_contract::ReceiptFixture;
        let backend = create_test_backend();
        let tenant = create_test_tenant();
        let sub = SubmissionId::generate("corrupt-receipt");
        backend
            .seed_receipts(
                &tenant,
                &sub,
                "manifest",
                &[paging_contract::ReceiptRow {
                    file: String::new(),
                    line: 0,
                    id: "p".to_string(),
                    outcome: "processing-error",
                }],
            )
            .await;
        backend
            .get_connection()
            .unwrap()
            .execute(
                "UPDATE bulk_entry_results SET operation_outcome = ?1",
                params![b"not-json".to_vec()],
            )
            .unwrap();
        assert!(
            backend
                .get_entry_results_page(&tenant, &sub, "manifest", None, 10, None)
                .await
                .is_err()
        );
        backend.get_connection().unwrap().execute("UPDATE bulk_entry_results SET operation_outcome = NULL, line_number = 'not-a-number'", []).unwrap();
        assert!(
            backend
                .get_entry_results_page(&tenant, &sub, "manifest", None, 10, None)
                .await
                .is_err()
        );
    }

    fn create_test_backend() -> SqliteBackend {
        let backend = SqliteBackend::in_memory().unwrap();
        backend.init_schema().unwrap();
        backend
    }

    fn create_test_tenant() -> TenantContext {
        TenantContext::new(
            TenantId::new("test-tenant"),
            TenantPermissions::full_access(),
        )
    }

    /// #815 review: the batch-transaction write path must index exactly like
    /// the direct path — FTS content for `_text`/`_content`, contained
    /// resources for `_contained` — and the rollback record plus per-line
    /// receipt must be committed by the same transaction as the resource.
    #[tokio::test]
    async fn ingested_entries_index_like_direct_writes_and_commit_their_bookkeeping() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let sub_id = SubmissionId::generate("test-system");
        backend
            .create_submission(&tenant, &sub_id, None)
            .await
            .unwrap();
        let manifest = backend
            .add_manifest(&tenant, &sub_id, Some("https://x/manifest.json"), None)
            .await
            .unwrap();

        let entry = NdjsonEntry::parse(
            1,
            r#"{"resourceType":"Patient","id":"fts-1","name":[{"family":"Ftsable","given":["Search"]}],"contained":[{"resourceType":"Organization","id":"org1","name":"Contained Org"}]}"#,
        )
        .unwrap();
        let results = backend
            .process_entries(
                &tenant,
                &sub_id,
                &manifest.manifest_id,
                vec![entry],
                &BulkProcessingOptions::new().with_file_url("https://x/f.ndjson"),
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].is_success());

        let conn = backend.get_connection().unwrap();
        let fts_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM resource_fts WHERE tenant_id = ?1 AND resource_id = 'fts-1'",
                [tenant.tenant_id().as_str()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(fts_rows, 1, "_text/_content index row written");

        let contained_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM search_index
                 WHERE tenant_id = ?1 AND resource_id = 'fts-1' AND is_contained = 1",
                [tenant.tenant_id().as_str()],
                |row| row.get(0),
            )
            .unwrap();
        assert!(contained_rows > 0, "_contained index rows written");

        let change_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM bulk_submission_changes WHERE tenant_id = ?1",
                [tenant.tenant_id().as_str()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(change_rows, 1, "rollback record committed with the write");

        let receipt_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM bulk_entry_results WHERE tenant_id = ?1",
                [tenant.tenant_id().as_str()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(receipt_rows, 1, "per-line receipt committed with the write");
    }

    #[tokio::test]
    async fn test_create_submission() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let sub_id = SubmissionId::generate("test-system");
        let summary = backend
            .create_submission(&tenant, &sub_id, None)
            .await
            .unwrap();

        assert_eq!(summary.status, SubmissionStatus::InProgress);
        assert_eq!(summary.manifest_count, 0);
    }

    #[tokio::test]
    async fn test_duplicate_submission() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let sub_id = SubmissionId::new("test-system", "sub-123");
        backend
            .create_submission(&tenant, &sub_id, None)
            .await
            .unwrap();

        let result = backend.create_submission(&tenant, &sub_id, None).await;
        assert!(matches!(
            result,
            Err(StorageError::BulkSubmit(
                BulkSubmitError::DuplicateSubmission { .. }
            ))
        ));
    }

    #[tokio::test]
    async fn test_add_manifest() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let sub_id = SubmissionId::generate("test-system");
        backend
            .create_submission(&tenant, &sub_id, None)
            .await
            .unwrap();

        let manifest = backend
            .add_manifest(
                &tenant,
                &sub_id,
                Some("http://example.com/data.ndjson"),
                None,
            )
            .await
            .unwrap();

        assert_eq!(manifest.status, ManifestStatus::Pending);
        assert_eq!(
            manifest.manifest_url,
            Some("http://example.com/data.ndjson".to_string())
        );
    }

    #[tokio::test]
    async fn test_process_entries() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let sub_id = SubmissionId::generate("test-system");
        backend
            .create_submission(&tenant, &sub_id, None)
            .await
            .unwrap();

        let manifest = backend
            .add_manifest(&tenant, &sub_id, None, None)
            .await
            .unwrap();

        let entries = vec![
            NdjsonEntry::new(
                1,
                "Patient",
                json!({"resourceType": "Patient", "name": [{"family": "Test1"}]}),
            ),
            NdjsonEntry::new(
                2,
                "Patient",
                json!({"resourceType": "Patient", "name": [{"family": "Test2"}]}),
            ),
        ];

        let options = BulkProcessingOptions::new();
        let results = backend
            .process_entries(&tenant, &sub_id, &manifest.manifest_id, entries, &options)
            .await
            .unwrap();

        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.is_success()));
        assert!(results.iter().all(|r| r.created));
    }

    /// #457: a manifest with several output files restarts line numbers in
    /// each file, so the stored entry-result key must include the file — the
    /// old key collided on every file after the first.
    /// #947: the manifest's status, its entry counters and the submission's
    /// timestamp used to be three autocommit statements around the batch —
    /// three extra write-lock acquisitions and fsyncs per batch, and a window
    /// where the entries were durable but the counts describing them were not.
    /// They now ride the batch transaction, and must still land.
    #[tokio::test]
    async fn batch_bookkeeping_lands_with_the_entries_it_describes() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let sub_id = SubmissionId::generate("test-system");
        backend
            .create_submission(&tenant, &sub_id, None)
            .await
            .unwrap();
        let manifest = backend
            .add_manifest(&tenant, &sub_id, None, None)
            .await
            .unwrap();

        let entries = (1..=3)
            .map(|i| {
                NdjsonEntry::new(
                    i,
                    "Patient",
                    json!({"resourceType": "Patient", "id": format!("p{i}")}),
                )
            })
            .collect();
        backend
            .process_entries(
                &tenant,
                &sub_id,
                &manifest.manifest_id,
                entries,
                &BulkProcessingOptions::new(),
            )
            .await
            .unwrap();

        let stored = backend
            .get_manifest(&tenant, &sub_id, &manifest.manifest_id)
            .await
            .unwrap()
            .expect("manifest");
        assert_eq!(stored.status, ManifestStatus::Processing);
        assert_eq!(stored.total_entries, 3);
        assert_eq!(stored.processed_entries, 3);
        assert_eq!(stored.failed_entries, 0);

        // And the resources really are there — the counters describe committed work.
        let counts = backend
            .get_entry_counts(&tenant, &sub_id, &manifest.manifest_id)
            .await
            .unwrap();
        assert_eq!(counts.total, 3);
        assert_eq!(counts.success, 3);
    }

    #[tokio::test]
    async fn test_process_entries_from_multiple_files_do_not_collide() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let sub_id = SubmissionId::generate("test-system");
        backend
            .create_submission(&tenant, &sub_id, None)
            .await
            .unwrap();
        let manifest = backend
            .add_manifest(&tenant, &sub_id, None, None)
            .await
            .unwrap();

        for (file, family) in [
            ("http://provider/Patient.ndjson", "FromPatients"),
            ("http://provider/Practitioner.ndjson", "FromPractitioners"),
        ] {
            // Both files start at line 1 — the collision of the old key.
            let entries = vec![NdjsonEntry::new(
                1,
                "Patient",
                json!({"resourceType": "Patient", "name": [{"family": family}]}),
            )];
            let options = BulkProcessingOptions::new().with_file_url(file);
            let results = backend
                .process_entries(&tenant, &sub_id, &manifest.manifest_id, entries, &options)
                .await
                .unwrap_or_else(|e| panic!("file {file} must ingest: {e}"));
            assert!(results.iter().all(|r| r.is_success()), "{file}");
        }

        let counts = backend
            .get_entry_counts(&tenant, &sub_id, &manifest.manifest_id)
            .await
            .unwrap();
        assert_eq!(counts.success, 2, "one stored entry result per file");
    }

    #[tokio::test]
    async fn test_complete_submission() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let sub_id = SubmissionId::generate("test-system");
        backend
            .create_submission(&tenant, &sub_id, None)
            .await
            .unwrap();

        let summary = backend.complete_submission(&tenant, &sub_id).await.unwrap();
        assert_eq!(summary.status, SubmissionStatus::Complete);
        assert!(summary.completed_at.is_some());
    }

    #[tokio::test]
    async fn test_abort_submission() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let sub_id = SubmissionId::generate("test-system");
        backend
            .create_submission(&tenant, &sub_id, None)
            .await
            .unwrap();

        backend
            .add_manifest(&tenant, &sub_id, None, None)
            .await
            .unwrap();

        let cancelled = backend
            .abort_submission(&tenant, &sub_id, "test abort")
            .await
            .unwrap();
        assert_eq!(cancelled, 1);

        let summary = backend
            .get_submission(&tenant, &sub_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(summary.status, SubmissionStatus::Aborted);
    }

    #[tokio::test]
    async fn test_rollback_create() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let sub_id = SubmissionId::generate("test-system");
        backend
            .create_submission(&tenant, &sub_id, None)
            .await
            .unwrap();

        let manifest = backend
            .add_manifest(&tenant, &sub_id, None, None)
            .await
            .unwrap();

        let entries = vec![NdjsonEntry::new(
            1,
            "Patient",
            json!({"resourceType": "Patient", "id": "rollback-test", "name": [{"family": "Test"}]}),
        )];

        let options = BulkProcessingOptions::new();
        let _results = backend
            .process_entries(&tenant, &sub_id, &manifest.manifest_id, entries, &options)
            .await
            .unwrap();

        // Verify resource was created
        let patient = backend
            .read(&tenant, "Patient", "rollback-test")
            .await
            .unwrap();
        assert!(patient.is_some());

        // Rollback
        let changes = backend.list_changes(&tenant, &sub_id, 10, 0).await.unwrap();
        assert_eq!(changes.len(), 1);

        let rolled_back = backend
            .rollback_change(&tenant, &sub_id, &changes[0])
            .await
            .unwrap();
        assert!(rolled_back);

        // Verify resource was deleted
        let patient = backend.read(&tenant, "Patient", "rollback-test").await;
        assert!(patient.is_err()); // Should be Gone
    }

    #[tokio::test]
    async fn test_entry_counts() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let sub_id = SubmissionId::generate("test-system");
        backend
            .create_submission(&tenant, &sub_id, None)
            .await
            .unwrap();

        let manifest = backend
            .add_manifest(&tenant, &sub_id, None, None)
            .await
            .unwrap();

        let entries = vec![
            NdjsonEntry::new(
                1,
                "Patient",
                json!({"resourceType": "Patient", "name": [{"family": "Test1"}]}),
            ),
            NdjsonEntry::new(
                2,
                "Patient",
                json!({"resourceType": "Patient", "name": [{"family": "Test2"}]}),
            ),
        ];

        let options = BulkProcessingOptions::new();
        backend
            .process_entries(&tenant, &sub_id, &manifest.manifest_id, entries, &options)
            .await
            .unwrap();

        let counts = backend
            .get_entry_counts(&tenant, &sub_id, &manifest.manifest_id)
            .await
            .unwrap();

        assert_eq!(counts.total, 2);
        assert_eq!(counts.success, 2);
        assert_eq!(counts.error_count(), 0);
    }

    async fn seed_claimable(backend: &SqliteBackend, tenant: &TenantContext) -> SubmissionId {
        let sub_id = SubmissionId::generate("worker-system");
        backend
            .create_submission(tenant, &sub_id, None)
            .await
            .unwrap();
        backend
            .add_manifest(
                tenant,
                &sub_id,
                Some("http://example.com/manifest.json"),
                None,
            )
            .await
            .unwrap();
        sub_id
    }

    #[tokio::test]
    async fn test_claim_heartbeat_finish() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();
        let sub_id = seed_claimable(&backend, &tenant).await;
        let worker = WorkerId::new("w1");

        let lease = backend
            .claim_next_manifest(&worker, StdDuration::from_secs(60))
            .await
            .unwrap()
            .expect("a manifest should be claimable");
        assert_eq!(lease.submission_id, sub_id);
        assert_eq!(lease.fencing_token, 1);

        // A second claim finds nothing (the only manifest is now processing w/ fresh lease).
        assert!(
            backend
                .claim_next_manifest(&WorkerId::new("w2"), StdDuration::from_secs(60))
                .await
                .unwrap()
                .is_none()
        );

        backend.heartbeat(&lease).await.unwrap();
        backend.mark_manifest_processing(&lease).await.unwrap();
        backend
            .add_manifest_progress(&lease, 5, 1, 6)
            .await
            .unwrap();
        backend.finish_manifest(&lease).await.unwrap();

        // After completion nothing is claimable.
        assert!(
            backend
                .claim_next_manifest(&worker, StdDuration::from_secs(60))
                .await
                .unwrap()
                .is_none()
        );
    }

    /// Both writers into a manifest's progress columns add rather than assign,
    /// so a reclaimed manifest never reports less progress than it had (#969).
    #[tokio::test]
    async fn test_progress_counters_only_move_forward() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();
        let sub_id = seed_claimable(&backend, &tenant).await;

        let lease = backend
            .claim_next_manifest(&WorkerId::new("w1"), StdDuration::from_secs(60))
            .await
            .unwrap()
            .unwrap();

        // The worker's own contributions accumulate across calls...
        backend
            .add_manifest_progress(&lease, 5, 1, 6)
            .await
            .unwrap();
        backend
            .add_manifest_progress(&lease, 2, 0, 2)
            .await
            .unwrap();
        assert_eq!(
            backend
                .get_manifest_for_worker(&lease)
                .await
                .unwrap()
                .last_processed_line,
            8
        );

        // ...and the ingestion engine's per-batch bookkeeping adds on top of
        // them instead of replacing them.
        backend
            .process_entries(
                &tenant,
                &sub_id,
                &lease.manifest_id,
                vec![
                    NdjsonEntry::new(1, "Patient", json!({"resourceType": "Patient"})),
                    NdjsonEntry::new(2, "Patient", json!({"resourceType": "Patient"})),
                ],
                &BulkProcessingOptions::new(),
            )
            .await
            .unwrap();

        let manifests = backend.list_manifests(&tenant, &sub_id).await.unwrap();
        assert_eq!(manifests[0].processed_entries, 9);
        assert_eq!(manifests[0].failed_entries, 1);
        assert_eq!(manifests[0].total_entries, 2);
        assert_eq!(
            backend
                .get_manifest_for_worker(&lease)
                .await
                .unwrap()
                .last_processed_line,
            10
        );
    }

    /// Lines the stream rejects never reach a batch, so the manifest's
    /// counters do not move for them — they are reported back for the worker
    /// to add instead (#969).
    #[tokio::test]
    async fn test_stream_reports_the_errors_no_batch_counted() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();
        let sub_id = seed_claimable(&backend, &tenant).await;
        let manifests = backend.list_manifests(&tenant, &sub_id).await.unwrap();
        let manifest_id = manifests[0].manifest_id.clone();

        // One ingestable Patient, one unparseable line, one resource of the
        // wrong type for this file.
        let ndjson = concat!(
            "{\"resourceType\":\"Patient\"}\n",
            "not-json\n",
            "{\"resourceType\":\"Observation\"}\n"
        );
        let result = backend
            .process_ndjson_stream(
                &tenant,
                &sub_id,
                &manifest_id,
                "Patient",
                Box::new(tokio::io::BufReader::new(std::io::Cursor::new(
                    ndjson.as_bytes().to_vec(),
                ))),
                &BulkProcessingOptions::new(),
            )
            .await
            .unwrap();
        assert_eq!(result.counts.error_count(), 2);
        assert_eq!(result.unbatched_errors, 2);

        let manifests = backend.list_manifests(&tenant, &sub_id).await.unwrap();
        assert_eq!(manifests[0].total_entries, 1);
        assert_eq!(manifests[0].processed_entries, 1);
        assert_eq!(
            manifests[0].failed_entries, 0,
            "a batch only ever counts what it committed"
        );
    }

    #[tokio::test]
    async fn test_fencing_blocks_zombie_writer() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();
        seed_claimable(&backend, &tenant).await;

        // Claim with a zero-duration lease so it is immediately reclaimable.
        let stale = backend
            .claim_next_manifest(&WorkerId::new("old"), StdDuration::from_secs(0))
            .await
            .unwrap()
            .unwrap();
        // A new worker reclaims it (bumps the fencing token).
        let fresh = backend
            .claim_next_manifest(&WorkerId::new("new"), StdDuration::from_secs(60))
            .await
            .unwrap()
            .unwrap();
        assert!(fresh.fencing_token > stale.fencing_token);

        // The stale lease can no longer mutate the manifest.
        assert!(matches!(
            backend.heartbeat(&stale).await,
            Err(LeaseError::LeaseLost { .. })
        ));
        assert!(matches!(
            backend.finish_manifest(&stale).await,
            Err(LeaseError::LeaseLost { .. })
        ));
        // The fresh lease still works.
        backend.finish_manifest(&fresh).await.unwrap();
    }

    #[tokio::test]
    async fn test_poll_token_lifecycle() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();
        let sub_id = SubmissionId::generate("poll-system");
        backend
            .create_submission(&tenant, &sub_id, None)
            .await
            .unwrap();

        let token = backend.ensure_poll_token(&tenant, &sub_id).await.unwrap();
        // Idempotent: same token returned.
        assert_eq!(
            token,
            backend.ensure_poll_token(&tenant, &sub_id).await.unwrap()
        );

        let resolved = backend.resolve_poll_token(&token).await.unwrap().unwrap();
        assert_eq!(resolved.submission_id, sub_id);

        backend.clear_poll_token(&tenant, &sub_id).await.unwrap();
        assert!(backend.resolve_poll_token(&token).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_record_and_delete_artifacts() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();
        seed_claimable(&backend, &tenant).await;
        let lease = backend
            .claim_next_manifest(&WorkerId::new("w1"), StdDuration::from_secs(60))
            .await
            .unwrap()
            .unwrap();

        let file = SubmitFileRecord {
            manifest_url: Some("http://example.com/manifest.json".to_string()),
            file_type: "error".to_string(),
            resource_type: None,
            part_index: 0,
            file_path: "tenant/sub/error-0.ndjson".to_string(),
            line_count: 3,
            byte_count: 120,
            count_severity: Some(json!({"error": 3})),
        };
        backend.record_submit_file(&lease, &file).await.unwrap();
        backend.record_submit_file(&lease, &file).await.unwrap();

        // Staged work is publication-private: only the generation marker selects it.
        let files = backend
            .list_submit_files(&tenant, &lease.submission_id)
            .await
            .unwrap();
        assert!(files.is_empty());
        let staged_count: i64 = backend
            .get_connection()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM bulk_submit_files
                 WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3",
                params![
                    tenant.tenant_id().as_str(),
                    lease.submission_id.submitter,
                    lease.submission_id.submission_id
                ],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(staged_count, 1);

        backend.finish_manifest(&lease).await.unwrap();
        let files = backend
            .list_submit_files(&tenant, &lease.submission_id)
            .await
            .unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].file_type, "error");
        assert_eq!(files[0].line_count, 3);
        assert_eq!(files[0].count_severity, Some(json!({"error": 3})));

        backend
            .delete_submission_artifacts(&tenant, &lease.submission_id)
            .await
            .unwrap();
        assert!(
            backend
                .list_submit_files(&tenant, &lease.submission_id)
                .await
                .unwrap()
                .is_empty()
        );
        let (status, published_token, publication_status, publication_error, publication_worker, _) =
            publication::publication_marker(
                &backend,
                &tenant,
                &lease.submission_id,
                &lease.manifest_id,
            );
        assert_eq!(status, "completed");
        assert_eq!(published_token, None);
        assert_eq!(publication_status, None);
        assert_eq!(publication_error, None);
        assert_eq!(publication_worker, None);
    }

    /// Wraps a reader so that `token` is tripped the first time the ingest
    /// actually reads from the stream.
    ///
    /// That makes "cancelled mid-manifest" deterministic: the pre-loop check
    /// still sees an un-cancelled token, the first batch is read and committed,
    /// and the between-batches check then sees the cancellation — no sleeps and
    /// no cross-task race.
    struct CancelOnFirstRead<R> {
        inner: R,
        token: CancelToken,
        tripped: bool,
    }

    impl<R: tokio::io::AsyncRead + Unpin> tokio::io::AsyncRead for CancelOnFirstRead<R> {
        fn poll_read(
            self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
            buf: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            let this = self.get_mut();
            if !this.tripped {
                this.tripped = true;
                this.token.cancel();
            }
            std::pin::Pin::new(&mut this.inner).poll_read(cx, buf)
        }
    }

    /// Six one-per-line Patients, enough for three batches of two.
    fn six_patient_lines() -> Vec<u8> {
        (1..=6)
            .map(|i| format!("{{\"resourceType\":\"Patient\",\"id\":\"cancel-{i}\"}}\n"))
            .collect::<String>()
            .into_bytes()
    }

    /// #968: a token already tripped when the ingest starts stops it before it
    /// reads or writes anything.
    #[tokio::test]
    async fn cancelled_before_start_persists_nothing() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let sub_id = SubmissionId::generate("test-system");
        backend
            .create_submission(&tenant, &sub_id, None)
            .await
            .unwrap();
        let manifest = backend
            .add_manifest(&tenant, &sub_id, None, None)
            .await
            .unwrap();

        let cancel = CancelToken::new();
        cancel.cancel();
        let options = BulkProcessingOptions::new()
            .with_batch_size(2)
            .with_cancel(cancel);

        let reader = Box::new(tokio::io::BufReader::new(std::io::Cursor::new(
            six_patient_lines(),
        )));
        let result = backend
            .process_ndjson_stream(
                &tenant,
                &sub_id,
                &manifest.manifest_id,
                "Patient",
                reader,
                &options,
            )
            .await
            .unwrap();

        assert!(result.aborted, "a cancelled ingest reports itself aborted");
        assert_eq!(
            result.abort_reason.as_deref(),
            Some(CANCELLED_ABORT_REASON),
            "the abort reason distinguishes cancellation from an error budget"
        );
        assert_eq!(result.lines_processed, 0, "no line was even read");
        assert_eq!(result.counts.total, 0);

        // Nothing was written: neither resources nor per-line receipts.
        let counts = backend
            .get_entry_counts(&tenant, &sub_id, &manifest.manifest_id)
            .await
            .unwrap();
        assert_eq!(counts.total, 0, "no entry result was persisted");
        assert!(
            backend
                .read(&tenant, "Patient", "cancel-1")
                .await
                .unwrap()
                .is_none(),
            "no resource was ingested"
        );
    }

    /// #968: cancelling mid-manifest stops at the next batch boundary, keeping
    /// the batches already committed and skipping the rest.
    #[tokio::test]
    async fn cancelled_mid_stream_keeps_committed_batches_and_stops() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let sub_id = SubmissionId::generate("test-system");
        backend
            .create_submission(&tenant, &sub_id, None)
            .await
            .unwrap();
        let manifest = backend
            .add_manifest(&tenant, &sub_id, None, None)
            .await
            .unwrap();

        let cancel = CancelToken::new();
        let options = BulkProcessingOptions::new()
            .with_batch_size(2)
            .with_cancel(cancel.clone());

        let reader = Box::new(tokio::io::BufReader::new(CancelOnFirstRead {
            inner: std::io::Cursor::new(six_patient_lines()),
            token: cancel,
            tripped: false,
        }));
        let result = backend
            .process_ndjson_stream(
                &tenant,
                &sub_id,
                &manifest.manifest_id,
                "Patient",
                reader,
                &options,
            )
            .await
            .unwrap();

        assert!(result.aborted);
        assert_eq!(result.abort_reason.as_deref(), Some(CANCELLED_ABORT_REASON));
        assert_eq!(
            result.counts.success, 2,
            "the batch already committed when the token tripped is kept"
        );
        assert_eq!(
            result.lines_processed, 2,
            "the remaining four lines were never read"
        );

        let counts = backend
            .get_entry_counts(&tenant, &sub_id, &manifest.manifest_id)
            .await
            .unwrap();
        assert_eq!(counts.total, 2, "the partial counts are durable");
        assert!(
            backend
                .read(&tenant, "Patient", "cancel-2")
                .await
                .unwrap()
                .is_some(),
            "the first batch really landed"
        );
        assert!(
            backend
                .read(&tenant, "Patient", "cancel-3")
                .await
                .unwrap()
                .is_none(),
            "nothing after the cancellation point was ingested"
        );
    }

    /// #968: `abort_submission` fails in-flight manifests without clearing the
    /// lease, so the worker's late verdict must lose rather than resurrect the
    /// manifest as `completed`.
    #[tokio::test]
    async fn abort_beats_a_late_finish_manifest() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();
        let sub_id = seed_claimable(&backend, &tenant).await;

        let lease = backend
            .claim_next_manifest(&WorkerId::new("w1"), StdDuration::from_secs(60))
            .await
            .unwrap()
            .expect("a manifest should be claimable");
        backend.mark_manifest_processing(&lease).await.unwrap();

        // The submitter aborts while the worker still holds a valid lease.
        backend
            .abort_submission(&tenant, &sub_id, "user cancelled")
            .await
            .unwrap();

        // The worker's verdicts arrive too late and change nothing.
        assert!(
            matches!(
                backend.finish_manifest(&lease).await,
                Err(LeaseError::LeaseLost { .. })
            ),
            "a finish after an abort must not win"
        );
        let stored = backend
            .get_manifest(&tenant, &sub_id, &lease.manifest_id)
            .await
            .unwrap()
            .expect("manifest");
        assert_eq!(
            stored.status,
            ManifestStatus::Failed,
            "the abort's verdict stands"
        );

        assert!(
            matches!(
                backend.fail_manifest(&lease, "worker gave up").await,
                Err(LeaseError::LeaseLost { .. })
            ),
            "a late failure verdict is equally a no-op"
        );
        let stored = backend
            .get_manifest(&tenant, &sub_id, &lease.manifest_id)
            .await
            .unwrap()
            .expect("manifest");
        assert_eq!(stored.status, ManifestStatus::Failed);
    }

    fn sqlite_busy() -> StorageError {
        StorageError::Backend(BackendError::Unavailable {
            backend_name: "sqlite".to_string(),
            message: "database is locked".to_string(),
        })
    }

    /// #942: a bookkeeping write that hits SQLITE_BUSY (classified as
    /// `Unavailable`) is retried and succeeds once the contention clears,
    /// instead of aborting the manifest. `start_paused` auto-advances the
    /// backoff sleeps.
    #[tokio::test(start_paused = true)]
    async fn busy_bookkeeping_write_is_retried_until_it_succeeds() {
        let attempts = std::sync::atomic::AtomicU32::new(0);
        let result = retry_bookkeeping_on_busy("test write", StdDuration::from_secs(30), || {
            let n = attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n < 2 {
                Err(sqlite_busy())
            } else {
                Ok(7usize)
            }
        })
        .await;
        assert_eq!(result.unwrap(), 7);
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    /// #942: sustained contention is still surfaced — the retry loop is
    /// bounded, and the final error is the classified busy error.
    #[tokio::test(start_paused = true)]
    async fn busy_bookkeeping_write_gives_up_when_the_budget_runs_out() {
        let attempts = std::sync::atomic::AtomicU32::new(0);
        let started = tokio::time::Instant::now();
        // 50 + 100 + 200 ms of backoff fit; the next 400 ms would not.
        let result: StorageResult<()> =
            retry_bookkeeping_on_busy("test write", StdDuration::from_millis(500), || {
                attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Err(sqlite_busy())
            })
            .await;
        assert!(matches!(
            result,
            Err(StorageError::Backend(BackendError::Unavailable { .. }))
        ));
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 4);
        // The point of the budget: it never overruns what the caller allowed.
        assert!(started.elapsed() < StdDuration::from_millis(500));
    }

    /// #942: the budget is wall-clock, not a number of attempts, so a caller
    /// with no time to spare still issues the write exactly once rather than
    /// skipping it.
    #[tokio::test(start_paused = true)]
    async fn a_zero_budget_still_attempts_the_write_once() {
        let attempts = std::sync::atomic::AtomicU32::new(0);
        let result: StorageResult<()> =
            retry_bookkeeping_on_busy("test write", StdDuration::ZERO, || {
                attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Err(sqlite_busy())
            })
            .await;
        assert!(result.is_err());
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    /// #942: the reason the budget exists — a lease-guarded write must finish
    /// retrying well inside the lease, so another worker cannot claim the
    /// manifest while we are still backing off.
    #[test]
    fn lease_retry_budget_is_half_the_lease() {
        let lease = ManifestLease {
            tenant: create_test_tenant(),
            submission_id: SubmissionId::new("s", "m"),
            manifest_id: "manifest-1".to_string(),
            worker_id: WorkerId::new("w1"),
            lease_expiry: Utc::now(),
            lease_duration: StdDuration::from_secs(60),
            fencing_token: 1,
        };
        assert_eq!(lease_retry_budget(&lease), StdDuration::from_secs(30));
        assert!(lease_retry_budget(&lease) < lease.lease_duration);
    }

    /// #942: only busy/locked retries — any other error surfaces on the
    /// first attempt, exactly as before.
    #[tokio::test]
    async fn non_busy_bookkeeping_error_is_not_retried() {
        let attempts = std::sync::atomic::AtomicU32::new(0);
        let result: StorageResult<()> =
            retry_bookkeeping_on_busy("test write", StdDuration::from_secs(30), || {
                attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Err(internal_error("constraint violation".to_string()))
            })
            .await;
        assert!(matches!(
            result,
            Err(StorageError::Backend(BackendError::Internal { .. }))
        ));
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    /// #978: `checkpoint_after_file` truncates the WAL back into the database.
    /// A file-backed WAL grows with each write until a checkpoint folds it in;
    /// after the call the `-wal` file is reclaimed (0 bytes on TRUNCATE).
    #[tokio::test]
    async fn checkpoint_after_file_truncates_the_wal() {
        use crate::core::bulk_submit_worker::SubmitWorkerStorage;

        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("wal-test.db");
        let backend = SqliteBackend::open(&db_path).unwrap();
        backend.init_schema().unwrap();
        let tenant = create_test_tenant();

        // Write enough resources to grow the WAL past its initial size.
        for i in 0..500 {
            backend
                .create(
                    &tenant,
                    "Patient",
                    json!({"resourceType": "Patient", "id": format!("wal-{i}")}),
                    FhirVersion::default_enabled(),
                )
                .await
                .unwrap();
        }

        let wal_path = db_path.with_extension("db-wal");
        let before = std::fs::metadata(&wal_path).map(|m| m.len()).unwrap_or(0);
        assert!(
            before > 0,
            "the WAL should have grown before the checkpoint"
        );

        backend.checkpoint_after_file().await;

        let after = std::fs::metadata(&wal_path).map(|m| m.len()).unwrap_or(0);
        assert!(
            after < before,
            "TRUNCATE checkpoint should reclaim the WAL: before={before} after={after}"
        );
    }

    /// #1007: `mark_entries_unindexed` flips only the named `(type, id)`
    /// entry results to `processing-error`, leaving the rest untouched, and
    /// is a no-op on an empty entry list.
    #[tokio::test]
    async fn mark_entries_unindexed_flips_only_the_named_resources() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let sub_id = SubmissionId::generate("test-system");
        backend
            .create_submission(&tenant, &sub_id, None)
            .await
            .unwrap();
        let manifest = backend
            .add_manifest(&tenant, &sub_id, None, None)
            .await
            .unwrap();

        let entries: Vec<NdjsonEntry> = ["sqlite-unidx-1", "sqlite-unidx-2", "sqlite-unidx-3"]
            .iter()
            .enumerate()
            .map(|(i, id)| {
                NdjsonEntry::new(
                    (i + 1) as u64,
                    "Patient",
                    json!({"resourceType": "Patient", "id": id}),
                )
            })
            .collect();
        let results = backend
            .process_entries(
                &tenant,
                &sub_id,
                &manifest.manifest_id,
                entries,
                &BulkProcessingOptions::new(),
            )
            .await
            .unwrap();
        assert!(results.iter().all(|r| r.is_success()));

        // An empty entry list touches nothing.
        assert_eq!(
            backend
                .mark_entries_unindexed(&tenant, &sub_id, &manifest.manifest_id, &[])
                .await
                .unwrap(),
            0
        );

        let oo = json!({
            "resourceType": "OperationOutcome",
            "issue": [{
                "severity": "error",
                "code": "incomplete",
                "diagnostics": "Patient/sqlite-unidx-2 was stored but could not be indexed for \
                                search on es: timeout. Run POST /Patient/$reindex to repair."
            }]
        });
        let changed = backend
            .mark_entries_unindexed(
                &tenant,
                &sub_id,
                &manifest.manifest_id,
                &[UnindexedEntry {
                    resource_type: "Patient".to_string(),
                    resource_id: "sqlite-unidx-2".to_string(),
                    operation_outcome: oo.clone(),
                }],
            )
            .await
            .unwrap();
        assert_eq!(changed, 1);

        let page = backend
            .get_entry_results_page(&tenant, &sub_id, &manifest.manifest_id, None, 10, None)
            .await
            .unwrap();
        for paged in &page.entries {
            let result = &paged.result;
            if result.resource_id.as_deref() == Some("sqlite-unidx-2") {
                assert_eq!(result.outcome, BulkEntryOutcome::ProcessingError);
                assert_eq!(result.operation_outcome.as_ref(), Some(&oo));
            } else {
                assert_eq!(result.outcome, BulkEntryOutcome::Success);
            }
        }
    }
}
