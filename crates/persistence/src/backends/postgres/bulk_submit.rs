//! Bulk submit implementation for PostgreSQL backend.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use helios_fhir::FhirVersion;
use serde_json::Value;
use std::time::Duration as StdDuration;
use tokio::io::{AsyncBufRead, AsyncBufReadExt};
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
use crate::error::{BackendError, BulkSubmitError, StorageError, StorageResult};
use crate::tenant::{TenantContext, TenantId, TenantPermissions};

use super::PostgresBackend;

fn internal_error(message: String) -> StorageError {
    StorageError::Backend(BackendError::Internal {
        backend_name: "postgres".to_string(),
        message,
        source: None,
    })
}

/// Builds a `LeaseError::LeaseLost` for a submit manifest.
fn lease_lost(lease: &ManifestLease) -> LeaseError {
    LeaseError::LeaseLost {
        job_id: ExportJobId::from_string(format!("{}/{}", lease.submission_id, lease.manifest_id)),
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

#[async_trait]
impl BulkSubmitProvider for PostgresBackend {
    async fn create_submission(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
        metadata: Option<Value>,
    ) -> StorageResult<SubmissionSummary> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        // Check for duplicate
        let rows = client
            .query(
                "SELECT 1 FROM bulk_submissions
                 WHERE tenant_id = $1 AND submitter = $2 AND submission_id = $3",
                &[
                    &tenant_id,
                    &id.submitter.as_str(),
                    &id.submission_id.as_str(),
                ],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to check duplicate: {}", e)))?;

        if !rows.is_empty() {
            return Err(StorageError::BulkSubmit(
                BulkSubmitError::DuplicateSubmission {
                    submitter: id.submitter.clone(),
                    submission_id: id.submission_id.clone(),
                },
            ));
        }

        let now = Utc::now();
        let metadata_json: Option<Value> = metadata.clone();

        client
            .execute(
                "INSERT INTO bulk_submissions
                 (tenant_id, submitter, submission_id, status, created_at, updated_at, metadata)
                 VALUES ($1, $2, $3, 'in-progress', $4, $5, $6)",
                &[
                    &tenant_id,
                    &id.submitter.as_str(),
                    &id.submission_id.as_str(),
                    &now,
                    &now,
                    &metadata_json,
                ],
            )
            .await
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
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        let rows = client
            .query(
                "SELECT status, created_at, updated_at, completed_at, metadata
                 FROM bulk_submissions
                 WHERE tenant_id = $1 AND submitter = $2 AND submission_id = $3",
                &[
                    &tenant_id,
                    &id.submitter.as_str(),
                    &id.submission_id.as_str(),
                ],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to get submission: {}", e)))?;

        if rows.is_empty() {
            return Ok(None);
        }

        let row = &rows[0];
        let status_str: String = row.get(0);
        let created_at: chrono::DateTime<Utc> = row.get(1);
        let updated_at: chrono::DateTime<Utc> = row.get(2);
        let completed_at: Option<chrono::DateTime<Utc>> = row.get(3);
        let metadata: Option<Value> = row.get(4);

        let status: SubmissionStatus = status_str
            .parse()
            .map_err(|_| internal_error(format!("Invalid status: {}", status_str)))?;

        // Get manifest count
        let manifest_row = client
            .query_one(
                "SELECT COUNT(*) FROM bulk_manifests
                 WHERE tenant_id = $1 AND submitter = $2 AND submission_id = $3",
                &[
                    &tenant_id,
                    &id.submitter.as_str(),
                    &id.submission_id.as_str(),
                ],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to count manifests: {}", e)))?;

        let manifest_count: i64 = manifest_row.get(0);

        // Get aggregated counts from entry results
        let counts_row = client
            .query_one(
                "SELECT
                    COUNT(*),
                    SUM(CASE WHEN outcome = 'success' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN outcome IN ('validation-error', 'processing-error') THEN 1 ELSE 0 END),
                    SUM(CASE WHEN outcome = 'skipped' THEN 1 ELSE 0 END)
                 FROM bulk_entry_results
                 WHERE tenant_id = $1 AND submitter = $2 AND submission_id = $3",
                &[&tenant_id, &id.submitter.as_str(), &id.submission_id.as_str()],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to count entries: {}", e)))?;

        let total: i64 = counts_row.get(0);
        let success: Option<i64> = counts_row.get(1);
        let errors: Option<i64> = counts_row.get(2);
        let skipped: Option<i64> = counts_row.get(3);

        Ok(Some(SubmissionSummary {
            id: id.clone(),
            status,
            created_at,
            updated_at,
            completed_at,
            manifest_count: manifest_count as u32,
            total_entries: total as u64,
            success_count: success.unwrap_or(0) as u64,
            error_count: errors.unwrap_or(0) as u64,
            skipped_count: skipped.unwrap_or(0) as u64,
            metadata,
        }))
    }

    async fn list_submissions(
        &self,
        tenant: &TenantContext,
        submitter: Option<&str>,
        status: Option<SubmissionStatus>,
        limit: u32,
        offset: u32,
    ) -> StorageResult<Vec<SubmissionSummary>> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        let mut sql = "SELECT submitter, submission_id FROM bulk_submissions WHERE tenant_id = $1"
            .to_string();
        let mut params: Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>> =
            vec![Box::new(tenant_id.to_string())];
        let mut param_idx = 2;

        if let Some(submitter) = submitter {
            sql.push_str(&format!(" AND submitter = ${}", param_idx));
            params.push(Box::new(submitter.to_string()));
            param_idx += 1;
        }

        if let Some(status) = status {
            sql.push_str(&format!(" AND status = ${}", param_idx));
            params.push(Box::new(status.to_string()));
        }

        sql.push_str(&format!(
            " ORDER BY created_at DESC LIMIT {} OFFSET {}",
            limit, offset
        ));

        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = params
            .iter()
            .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        let rows = client
            .query(&sql, &param_refs)
            .await
            .map_err(|e| internal_error(format!("Failed to query submissions: {}", e)))?;

        let mut results = Vec::new();
        for row in &rows {
            let submitter: String = row.get(0);
            let submission_id: String = row.get(1);
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
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        // Check current status
        let rows = client
            .query(
                "SELECT status FROM bulk_submissions
                 WHERE tenant_id = $1 AND submitter = $2 AND submission_id = $3",
                &[
                    &tenant_id,
                    &id.submitter.as_str(),
                    &id.submission_id.as_str(),
                ],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to get submission status: {}", e)))?;

        if rows.is_empty() {
            return Err(StorageError::BulkSubmit(
                BulkSubmitError::SubmissionNotFound {
                    submitter: id.submitter.clone(),
                    submission_id: id.submission_id.clone(),
                },
            ));
        }

        let current_status: String = rows[0].get(0);
        if current_status != "in-progress" {
            return Err(StorageError::BulkSubmit(BulkSubmitError::AlreadyComplete {
                submission_id: id.submission_id.clone(),
            }));
        }

        let now = Utc::now();
        client
            .execute(
                "UPDATE bulk_submissions SET status = 'complete', completed_at = $1, updated_at = $2
                 WHERE tenant_id = $3 AND submitter = $4 AND submission_id = $5",
                &[
                    &now,
                    &now,
                    &tenant_id,
                    &id.submitter.as_str(),
                    &id.submission_id.as_str(),
                ],
            )
            .await
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
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        // Check current status
        let rows = client
            .query(
                "SELECT status FROM bulk_submissions
                 WHERE tenant_id = $1 AND submitter = $2 AND submission_id = $3",
                &[
                    &tenant_id,
                    &id.submitter.as_str(),
                    &id.submission_id.as_str(),
                ],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to get submission status: {}", e)))?;

        if rows.is_empty() {
            return Err(StorageError::BulkSubmit(
                BulkSubmitError::SubmissionNotFound {
                    submitter: id.submitter.clone(),
                    submission_id: id.submission_id.clone(),
                },
            ));
        }

        let current_status: String = rows[0].get(0);
        if current_status != "in-progress" {
            return Err(StorageError::BulkSubmit(BulkSubmitError::AlreadyComplete {
                submission_id: id.submission_id.clone(),
            }));
        }

        // Count pending manifests
        let pending_row = client
            .query_one(
                "SELECT COUNT(*) FROM bulk_manifests
                 WHERE tenant_id = $1 AND submitter = $2 AND submission_id = $3
                 AND status IN ('pending', 'processing')",
                &[
                    &tenant_id,
                    &id.submitter.as_str(),
                    &id.submission_id.as_str(),
                ],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to count pending manifests: {}", e)))?;

        let pending_count: i64 = pending_row.get(0);
        let now = Utc::now();

        // Update submission status
        client
            .execute(
                "UPDATE bulk_submissions SET status = 'aborted', completed_at = $1, updated_at = $2
                 WHERE tenant_id = $3 AND submitter = $4 AND submission_id = $5",
                &[
                    &now,
                    &now,
                    &tenant_id,
                    &id.submitter.as_str(),
                    &id.submission_id.as_str(),
                ],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to abort submission: {}", e)))?;

        // Update pending manifests to failed
        client
            .execute(
                "UPDATE bulk_manifests SET status = 'failed'
                 WHERE tenant_id = $1 AND submitter = $2 AND submission_id = $3
                 AND status IN ('pending', 'processing')",
                &[
                    &tenant_id,
                    &id.submitter.as_str(),
                    &id.submission_id.as_str(),
                ],
            )
            .await
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
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        // Check submission exists and is in progress
        let rows = client
            .query(
                "SELECT status FROM bulk_submissions
                 WHERE tenant_id = $1 AND submitter = $2 AND submission_id = $3",
                &[
                    &tenant_id,
                    &submission_id.submitter.as_str(),
                    &submission_id.submission_id.as_str(),
                ],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to get submission: {}", e)))?;

        if rows.is_empty() {
            return Err(StorageError::BulkSubmit(
                BulkSubmitError::SubmissionNotFound {
                    submitter: submission_id.submitter.clone(),
                    submission_id: submission_id.submission_id.clone(),
                },
            ));
        }

        let status: String = rows[0].get(0);
        if status != "in-progress" {
            return Err(StorageError::BulkSubmit(BulkSubmitError::InvalidState {
                submission_id: submission_id.submission_id.clone(),
                expected: "in-progress".to_string(),
                actual: status,
            }));
        }

        let manifest_id = Uuid::new_v4().to_string();
        let now = Utc::now();

        client
            .execute(
                "INSERT INTO bulk_manifests
                 (tenant_id, submitter, submission_id, manifest_id, manifest_url, replaces_manifest_url, status, added_at)
                 VALUES ($1, $2, $3, $4, $5, $6, 'pending', $7)",
                &[
                    &tenant_id,
                    &submission_id.submitter.as_str(),
                    &submission_id.submission_id.as_str(),
                    &manifest_id.as_str(),
                    &manifest_url,
                    &replaces_manifest_url,
                    &now,
                ],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to add manifest: {}", e)))?;

        // Update submission updated_at
        client
            .execute(
                "UPDATE bulk_submissions SET updated_at = $1
                 WHERE tenant_id = $2 AND submitter = $3 AND submission_id = $4",
                &[
                    &now,
                    &tenant_id,
                    &submission_id.submitter.as_str(),
                    &submission_id.submission_id.as_str(),
                ],
            )
            .await
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
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        let rows = client
            .query(
                "SELECT manifest_url, replaces_manifest_url, status, added_at, total_entries, processed_entries, failed_entries, lease_expiry, bytes_processed, bytes_total, phase, files_done, files_total
                 FROM bulk_manifests
                 WHERE tenant_id = $1 AND submitter = $2 AND submission_id = $3 AND manifest_id = $4",
                &[
                    &tenant_id,
                    &submission_id.submitter.as_str(),
                    &submission_id.submission_id.as_str(),
                    &manifest_id,
                ],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to get manifest: {}", e)))?;

        if rows.is_empty() {
            return Ok(None);
        }

        let row = &rows[0];
        let manifest_url: Option<String> = row.get(0);
        let replaces_manifest_url: Option<String> = row.get(1);
        let status_str: String = row.get(2);
        let added_at: chrono::DateTime<Utc> = row.get(3);
        let total: i32 = row.get(4);
        let processed: i32 = row.get(5);
        let failed: i32 = row.get(6);
        let lease_expiry: Option<chrono::DateTime<Utc>> = row.get(7);
        let bytes_processed: i64 = row.get(8);
        let bytes_total: i64 = row.get(9);
        // Unlike `status`, an unreadable phase is not an error: it is a
        // cosmetic hint, and a row written by a newer HFS must still be
        // readable here (#953).
        let phase: Option<String> = row.get(10);
        let files_done: i64 = row.get(11);
        let files_total: i64 = row.get(12);

        let status: ManifestStatus = status_str
            .parse()
            .map_err(|_| internal_error(format!("Invalid manifest status: {}", status_str)))?;

        Ok(Some(SubmissionManifest {
            manifest_id: manifest_id.to_string(),
            manifest_url,
            replaces_manifest_url,
            status,
            added_at,
            total_entries: total as u64,
            processed_entries: processed as u64,
            failed_entries: failed as u64,
            lease_expiry,
            bytes_processed: bytes_processed.max(0) as u64,
            bytes_total: bytes_total.max(0) as u64,
            phase: phase.and_then(|p| p.parse::<ManifestPhase>().ok()),
            files_done: files_done.max(0) as u64,
            files_total: files_total.max(0) as u64,
        }))
    }

    async fn list_manifests(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
    ) -> StorageResult<Vec<SubmissionManifest>> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        let rows = client
            .query(
                "SELECT manifest_id FROM bulk_manifests
                 WHERE tenant_id = $1 AND submitter = $2 AND submission_id = $3
                 ORDER BY added_at",
                &[
                    &tenant_id,
                    &submission_id.submitter.as_str(),
                    &submission_id.submission_id.as_str(),
                ],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to query manifests: {}", e)))?;

        let mut results = Vec::new();
        for row in &rows {
            let manifest_id: String = row.get(0);
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

        // Update manifest status to processing, on a client scoped to this one
        // statement.
        //
        // `status IN ('pending', 'processing')` keeps it a promotion rather
        // than a reset: the statement runs on *every* batch, so without the
        // guard the batch that lands right after `abort_submission` moved the
        // manifest to `'failed'` would quietly put it back to `'processing'`
        // and the abort would read as if it had never happened (#968).
        {
            let client = self.get_client().await?;
            client
                .execute(
                    "UPDATE bulk_manifests SET status = 'processing'
                     WHERE tenant_id = $1 AND submitter = $2 AND submission_id = $3 AND manifest_id = $4
                       AND status IN ('pending', 'processing')",
                    &[
                        &tenant_id,
                        &submission_id.submitter.as_str(),
                        &submission_id.submission_id.as_str(),
                        &manifest_id,
                    ],
                )
                .await
                .map_err(|e| internal_error(format!("Failed to update manifest status: {}", e)))?;
        }

        let mut results = Vec::new();
        let mut error_count = 0u32;
        let mut aborted_on_max_errors = false;
        let file_url = options.file_url.as_deref().unwrap_or("");

        // One transaction per batch (#872, mirroring the SQLite batch ingest
        // from #815): entry rows, history, search-index writes, rollback
        // records, and per-line receipts commit together — one WAL flush per
        // batch instead of four-plus autocommit round trips per resource, and
        // the rollback log can never diverge from what was actually written.
        // Each entry runs under a savepoint: on Postgres a failed statement
        // aborts the whole transaction, and the savepoint contains a failure
        // to its one entry the way SQLite's per-statement independence does.
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

        for entry in entries {
            if options.max_errors > 0 && error_count >= options.max_errors {
                if !options.continue_on_error {
                    aborted_on_max_errors = true;
                    break;
                }
                let skip_result = BulkEntryResult::skipped(
                    entry.line_number,
                    &entry.resource_type,
                    "max errors exceeded",
                );
                Self::write_entry_rows_tx(
                    &txn,
                    submission_id,
                    manifest_id,
                    file_url,
                    &skip_result,
                    None,
                )
                .await?;
                results.push(skip_result);
                continue;
            }

            // The transaction buffers creates and sends them in batches, so
            // an entry's conflict can surface after its `create` returned.
            // The savepoint methods flush at the savepoint boundaries: the
            // release raises this entry's conflict inside its own savepoint,
            // and the rollback undoes only this entry's rows.
            txn.savepoint("bulk_entry").await?;
            let outcome = match self
                .ingest_entry_in_tx(&mut txn, manifest_id, &entry, options)
                .await
            {
                Ok(pair) => txn.release_savepoint("bulk_entry").await.map(|_| pair),
                Err(e) => Err(e),
            };
            let (entry_result, change) = match outcome {
                Ok(pair) => pair,
                Err(e) => {
                    txn.rollback_to_savepoint("bulk_entry")
                        .await
                        .map_err(|se| internal_error(format!("{se} after '{e}'")))?;
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

            Self::write_entry_rows_tx(
                &txn,
                submission_id,
                manifest_id,
                file_url,
                &entry_result,
                change.as_ref(),
            )
            .await?;
            results.push(entry_result);
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

        // Update manifest counts, on a fresh client for the tail statements.
        //
        // Every column here accumulates: these counters are cumulative across
        // all runs of the manifest (including resumes) and are the ones the
        // submit status endpoint reports (#969). `processed_entries` counts the
        // entries that did not fail — successes plus deliberate skips — so that
        // `processed_entries + failed_entries` equals the entries walked, and
        // `last_processed_line` advances by the entries this batch consumed.
        let now = Utc::now();
        let client = self.get_client().await?;
        client
            .execute(
                "UPDATE bulk_manifests SET
                    total_entries = total_entries + $1,
                    processed_entries = processed_entries + $2,
                    failed_entries = failed_entries + $3,
                    last_processed_line = last_processed_line + $4
                 WHERE tenant_id = $5 AND submitter = $6 AND submission_id = $7 AND manifest_id = $8",
                &[
                    &(results.len() as i32),
                    &(results.iter().filter(|r| r.is_success()).count() as i32),
                    &(error_count as i32),
                    &(results.len() as i64),
                    &tenant_id,
                    &submission_id.submitter.as_str(),
                    &submission_id.submission_id.as_str(),
                    &manifest_id,
                ],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to update manifest counts: {}", e)))?;

        // Update submission updated_at
        client
            .execute(
                "UPDATE bulk_submissions SET updated_at = $1
                 WHERE tenant_id = $2 AND submitter = $3 AND submission_id = $4",
                &[
                    &now,
                    &tenant_id,
                    &submission_id.submitter.as_str(),
                    &submission_id.submission_id.as_str(),
                ],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to update submission: {}", e)))?;

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
                i32::try_from(cursor.line_number).map_err(|_| {
                    invalid_entry_result_page(
                        "Receipt cursor line exceeds PostgreSQL INTEGER range",
                    )
                })?,
            )),
            Some(EntryResultContinuation::Offset(_)) => {
                return Err(invalid_entry_result_page(
                    "PostgreSQL receipt pages require a keyset continuation",
                ));
            }
        };
        let client = self.get_client().await?;
        let mut sql = "SELECT file_url, line_number, resource_type, resource_id, created, outcome, operation_outcome
             FROM bulk_entry_results
             WHERE tenant_id = $1 AND submitter = $2 AND submission_id = $3 AND manifest_id = $4".to_string();
        let mut params: Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>> = vec![
            Box::new(tenant.tenant_id().as_str().to_string()),
            Box::new(submission_id.submitter.clone()),
            Box::new(submission_id.submission_id.clone()),
            Box::new(manifest_id.to_string()),
        ];
        if let Some(outcome) = outcome_filter {
            sql.push_str(&format!(" AND outcome = ${}", params.len() + 1));
            params.push(Box::new(outcome.to_string()));
        }
        if let Some((file, line)) = after {
            sql.push_str(&format!(
                " AND (file_url, line_number) > (${}, ${})",
                params.len() + 1,
                params.len() + 2
            ));
            params.push(Box::new(file.to_string()));
            params.push(Box::new(line));
        }
        sql.push_str(&format!(
            " ORDER BY file_url, line_number LIMIT ${}",
            params.len() + 1
        ));
        params.push(Box::new(i64::from(limit)));
        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = params
            .iter()
            .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();
        let rows = client
            .query(&sql, &param_refs)
            .await
            .map_err(|e| internal_error(format!("Failed to query receipt page: {e}")))?;
        let entries: Vec<PagedEntryResult> = rows
            .iter()
            .map(|row| -> StorageResult<_> {
                let decode = |e| internal_error(format!("Failed to decode receipt page: {e}"));
                let file_url: String = row.try_get(0).map_err(decode)?;
                let line: i32 = row.try_get(1).map_err(decode)?;
                let line_number = u64::try_from(line).map_err(|_| {
                    internal_error("Negative stored receipt line number".to_string())
                })?;
                let resource_type = row.try_get(2).map_err(decode)?;
                let resource_id = row.try_get(3).map_err(decode)?;
                let created: Option<bool> = row.try_get(4).map_err(decode)?;
                let outcome_str: String = row.try_get(5).map_err(decode)?;
                let operation_outcome = row.try_get(6).map_err(decode)?;
                // Preserve the existing interpretation of unknown outcome labels.
                let outcome = outcome_str
                    .parse()
                    .unwrap_or(BulkEntryOutcome::ProcessingError);
                Ok(PagedEntryResult {
                    stored_identity: Some(EntryResultCursor {
                        file_url,
                        line_number,
                    }),
                    result: BulkEntryResult {
                        line_number,
                        resource_type,
                        resource_id,
                        created: created.unwrap_or(false),
                        outcome,
                        operation_outcome,
                    },
                })
            })
            .collect::<StorageResult<_>>()?;
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
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        let row = client
            .query_one(
                "SELECT
                    COUNT(*),
                    SUM(CASE WHEN outcome = 'success' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN outcome = 'validation-error' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN outcome = 'processing-error' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN outcome = 'skipped' THEN 1 ELSE 0 END)
                 FROM bulk_entry_results
                 WHERE tenant_id = $1 AND submitter = $2 AND submission_id = $3 AND manifest_id = $4",
                &[
                    &tenant_id,
                    &submission_id.submitter.as_str(),
                    &submission_id.submission_id.as_str(),
                    &manifest_id,
                ],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to count entries: {}", e)))?;

        let total: i64 = row.get(0);
        let success: Option<i64> = row.get(1);
        let validation_error: Option<i64> = row.get(2);
        let processing_error: Option<i64> = row.get(3);
        let skipped: Option<i64> = row.get(4);

        Ok(EntryCountSummary {
            total: total as u64,
            success: success.unwrap_or(0) as u64,
            validation_error: validation_error.unwrap_or(0) as u64,
            processing_error: processing_error.unwrap_or(0) as u64,
            skipped: skipped.unwrap_or(0) as u64,
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
        let mut client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();
        let txn = client
            .transaction()
            .await
            .map_err(|e| internal_error(format!("Failed to begin unindexed-mark txn: {}", e)))?;

        let mut affected = 0u64;
        for entry in entries {
            let rows = txn
                .execute(
                    "UPDATE bulk_entry_results
                     SET outcome = 'processing-error', operation_outcome = $1
                     WHERE tenant_id = $2 AND submitter = $3 AND submission_id = $4
                       AND manifest_id = $5 AND resource_type = $6 AND resource_id = $7",
                    &[
                        &entry.operation_outcome,
                        &tenant_id,
                        &submission_id.submitter.as_str(),
                        &submission_id.submission_id.as_str(),
                        &manifest_id,
                        &entry.resource_type.as_str(),
                        &entry.resource_id.as_str(),
                    ],
                )
                .await
                .map_err(|e| internal_error(format!("Failed to mark entry unindexed: {}", e)))?;
            affected += rows;
        }

        txn.commit()
            .await
            .map_err(|e| internal_error(format!("Failed to commit unindexed-mark txn: {}", e)))?;
        Ok(affected)
    }
}

impl PostgresBackend {
    /// Applies one NDJSON entry inside the batch transaction, returning the
    /// entry's result plus the rollback record to write with it.
    ///
    /// Mirrors the SQLite `ingest_entry_in_txn` exactly: read-for-upsert,
    /// import-mode-aware update, create for new ids. A soft-deleted row reads
    /// as `None` and then fails the create with `AlreadyExists` — the same
    /// outcome the storage-path create produced, recorded as this entry's
    /// error by the caller's savepoint arm.
    async fn ingest_entry_in_tx(
        &self,
        txn: &mut super::transaction::PostgresTransaction,
        manifest_id: &str,
        entry: &NdjsonEntry,
        options: &BulkProcessingOptions,
    ) -> StorageResult<(BulkEntryResult, Option<SubmissionChange>)> {
        use crate::core::Transaction;

        if let Some(id) = entry.resource_id.as_ref() {
            match txn.read(&entry.resource_type, id).await? {
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

    /// Writes an entry's rollback record and per-line receipt on the batch
    /// transaction's client, so both commit (or vanish) with the entry writes.
    async fn write_entry_rows_tx(
        txn: &super::transaction::PostgresTransaction,
        submission_id: &SubmissionId,
        manifest_id: &str,
        file_url: &str,
        result: &BulkEntryResult,
        change: Option<&SubmissionChange>,
    ) -> StorageResult<()> {
        use crate::core::Transaction;

        let client = txn.raw_client()?;
        let tenant_id = txn.tenant().tenant_id().as_str().to_string();

        let previous_content_json: Option<Value> = change
            .map(|c| c.previous_content.clone())
            .unwrap_or_default();
        let outcome_json: Option<Value> = result.operation_outcome.clone();

        // The two bookkeeping inserts are independent: pipelined on the
        // transaction's connection they cost one round trip, not two.
        let record_change = async {
            let Some(change) = change else {
                return Ok(0);
            };
            client
                .execute(
                    "INSERT INTO bulk_submission_changes
                     (tenant_id, submitter, submission_id, change_id, manifest_id, change_type, resource_type, resource_id, previous_version, new_version, previous_content, changed_at)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
                    &[
                        &tenant_id,
                        &submission_id.submitter.as_str(),
                        &submission_id.submission_id.as_str(),
                        &change.change_id.as_str(),
                        &change.manifest_id.as_str(),
                        &change.change_type.to_string().as_str(),
                        &change.resource_type.as_str(),
                        &change.resource_id.as_str(),
                        &change.previous_version,
                        &change.new_version.as_str(),
                        &previous_content_json,
                        &change.changed_at,
                    ],
                )
                .await
        };
        let submitter = submission_id.submitter.as_str();
        let sub_id_str = submission_id.submission_id.as_str();
        let line_number = result.line_number as i32;
        let result_type = result.resource_type.as_str();
        let outcome_code = result.outcome.to_string();
        let receipt_params: [&(dyn tokio_postgres::types::ToSql + Sync); 11] = [
            &tenant_id,
            &submitter,
            &sub_id_str,
            &manifest_id,
            &file_url,
            &line_number,
            &result_type,
            &result.resource_id,
            &result.created,
            &outcome_code,
            &outcome_json,
        ];
        let store_receipt = client.execute(
            // Upsert: the worker re-fetches a whole file after a transient
            // failure, and the retry must overwrite its own earlier rows
            // instead of colliding with them (#457).
            "INSERT INTO bulk_entry_results
             (tenant_id, submitter, submission_id, manifest_id, file_url, line_number, resource_type, resource_id, created, outcome, operation_outcome)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
             ON CONFLICT (tenant_id, submitter, submission_id, manifest_id, file_url, line_number)
             DO UPDATE SET resource_type = EXCLUDED.resource_type,
                           resource_id = EXCLUDED.resource_id,
                           created = EXCLUDED.created,
                           outcome = EXCLUDED.outcome,
                           operation_outcome = EXCLUDED.operation_outcome",
            &receipt_params,
        );
        let (change_res, receipt_res) = futures::future::join(record_change, store_receipt).await;
        change_res.map_err(|e| internal_error(format!("Failed to record change: {}", e)))?;
        receipt_res.map_err(|e| internal_error(format!("Failed to store entry result: {}", e)))?;

        Ok(())
    }
}

#[async_trait]
impl StreamingBulkSubmitProvider for PostgresBackend {
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
                break;
            }

            line_number += 1;
            result.lines_processed = line_number;

            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            match NdjsonEntry::parse(line_number, line) {
                Ok(entry) => {
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
impl BulkSubmitRollbackProvider for PostgresBackend {
    async fn record_change(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        change: &SubmissionChange,
    ) -> StorageResult<()> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        let previous_content_json: Option<Value> = change.previous_content.clone();

        client
            .execute(
                "INSERT INTO bulk_submission_changes
                 (tenant_id, submitter, submission_id, change_id, manifest_id, change_type, resource_type, resource_id, previous_version, new_version, previous_content, changed_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
                &[
                    &tenant_id,
                    &submission_id.submitter.as_str(),
                    &submission_id.submission_id.as_str(),
                    &change.change_id.as_str(),
                    &change.manifest_id.as_str(),
                    &change.change_type.to_string().as_str(),
                    &change.resource_type.as_str(),
                    &change.resource_id.as_str(),
                    &change.previous_version,
                    &change.new_version.as_str(),
                    &previous_content_json,
                    &change.changed_at,
                ],
            )
            .await
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
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        let sql = format!(
            "SELECT change_id, manifest_id, change_type, resource_type, resource_id, previous_version, new_version, previous_content, changed_at
             FROM bulk_submission_changes
             WHERE tenant_id = $1 AND submitter = $2 AND submission_id = $3
             ORDER BY changed_at DESC
             LIMIT {} OFFSET {}",
            limit, offset
        );

        let rows = client
            .query(
                &sql,
                &[
                    &tenant_id,
                    &submission_id.submitter.as_str(),
                    &submission_id.submission_id.as_str(),
                ],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to query changes: {}", e)))?;

        let changes: Vec<SubmissionChange> = rows
            .iter()
            .map(|row| {
                let change_id: String = row.get(0);
                let manifest_id: String = row.get(1);
                let change_type_str: String = row.get(2);
                let resource_type: String = row.get(3);
                let resource_id: String = row.get(4);
                let previous_version: Option<String> = row.get(5);
                let new_version: String = row.get(6);
                let previous_content: Option<Value> = row.get(7);
                let changed_at: chrono::DateTime<Utc> = row.get(8);

                let change_type: ChangeType = change_type_str.parse().unwrap_or(ChangeType::Create);

                SubmissionChange {
                    change_id,
                    manifest_id,
                    change_type,
                    resource_type,
                    resource_id,
                    previous_version,
                    new_version,
                    previous_content,
                    changed_at,
                }
            })
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
                match self
                    .delete(tenant, &change.resource_type, &change.resource_id)
                    .await
                {
                    Ok(()) => Ok(true),
                    Err(StorageError::Resource(crate::error::ResourceError::NotFound {
                        ..
                    })) => Ok(true),
                    Err(e) => Err(e),
                }
            }
            ChangeType::Update => {
                if let Some(ref previous_content) = change.previous_content {
                    let current = self
                        .read(tenant, &change.resource_type, &change.resource_id)
                        .await?;
                    if let Some(current) = current {
                        self.update(tenant, &current, previous_content.clone())
                            .await?;
                        Ok(true)
                    } else {
                        Ok(false)
                    }
                } else {
                    Ok(false)
                }
            }
        }
    }
}

#[async_trait]
impl SubmitClaimStrategy for PostgresBackend {
    async fn claim_next_manifest(
        &self,
        worker_id: &WorkerId,
        lease_duration: StdDuration,
    ) -> StorageResult<Option<ManifestLease>> {
        let mut client = self.get_client().await?;
        let now = Utc::now();
        let lease_expiry = now
            + chrono::Duration::from_std(lease_duration)
                .unwrap_or_else(|_| chrono::Duration::seconds(60));

        let txn = client
            .transaction()
            .await
            .map_err(|e| internal_error(format!("Failed to begin claim txn: {}", e)))?;

        let rows = txn
            .query(
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
                            AND (m.lease_expiry IS NULL OR m.lease_expiry < $1)))
                 ORDER BY m.added_at
                 LIMIT 1
                 FOR UPDATE OF m SKIP LOCKED",
                &[&now],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to select claimable manifest: {}", e)))?;

        let Some(row) = rows.first() else {
            txn.commit()
                .await
                .map_err(|e| internal_error(format!("Failed to commit claim txn: {}", e)))?;
            return Ok(None);
        };
        let tenant_id: String = row.get(0);
        let submitter: String = row.get(1);
        let submission_id: String = row.get(2);
        let manifest_id: String = row.get(3);
        let fencing_token: i64 = row.get(4);
        let new_token = fencing_token + 1;

        txn.execute(
            "UPDATE bulk_manifests
             SET status = 'processing', worker_id = $1, lease_expiry = $2, fencing_token = $3
             WHERE tenant_id = $4 AND submitter = $5 AND submission_id = $6 AND manifest_id = $7",
            &[
                &worker_id.as_str(),
                &lease_expiry,
                &new_token,
                &tenant_id,
                &submitter,
                &submission_id,
                &manifest_id,
            ],
        )
        .await
        .map_err(|e| internal_error(format!("Failed to claim manifest: {}", e)))?;

        txn.commit()
            .await
            .map_err(|e| internal_error(format!("Failed to commit claim txn: {}", e)))?;

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
        let client = self.get_client().await.map_err(LeaseError::Storage)?;
        let new_expiry = lease.renewed_expiry();
        let affected = client
            .execute(
                "UPDATE bulk_manifests SET lease_expiry = $1
                 WHERE tenant_id = $2 AND submitter = $3 AND submission_id = $4
                   AND manifest_id = $5 AND status = 'processing'
                   AND worker_id = $6 AND fencing_token = $7",
                &[
                    &new_expiry,
                    &lease.tenant.tenant_id().as_str(),
                    &lease.submission_id.submitter,
                    &lease.submission_id.submission_id,
                    &lease.manifest_id,
                    &lease.worker_id.as_str(),
                    &(lease.fencing_token as i64),
                ],
            )
            .await
            .map_err(|e| LeaseError::Storage(internal_error(format!("heartbeat failed: {e}"))))?;
        if affected == 0 {
            Err(lease_lost(lease))
        } else {
            Ok(new_expiry)
        }
    }

    async fn release(&self, lease: ManifestLease) -> StorageResult<()> {
        let client = self.get_client().await?;
        client
            .execute(
                "UPDATE bulk_manifests
                 SET status = 'pending', worker_id = NULL, lease_expiry = NULL
                 WHERE tenant_id = $1 AND submitter = $2 AND submission_id = $3
                   AND manifest_id = $4 AND worker_id = $5 AND fencing_token = $6
                   AND status = 'processing'",
                &[
                    &lease.tenant.tenant_id().as_str(),
                    &lease.submission_id.submitter,
                    &lease.submission_id.submission_id,
                    &lease.manifest_id,
                    &lease.worker_id.as_str(),
                    &(lease.fencing_token as i64),
                ],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to release manifest lease: {}", e)))?;
        Ok(())
    }
}

#[async_trait]
impl SubmitWorkerStorage for PostgresBackend {
    async fn get_manifest_for_worker(
        &self,
        lease: &ManifestLease,
    ) -> Result<ManifestWorkerView, LeaseError> {
        let client = self.get_client().await.map_err(LeaseError::Storage)?;
        let rows = client
            .query(
                "SELECT manifest_url, fhir_base_url, output_format, file_request_headers,
                        oauth_metadata_urls, file_encryption_key, last_processed_line,
                        import_directives, submission_metadata
                FROM bulk_manifests
                WHERE tenant_id = $1 AND submitter = $2 AND submission_id = $3
                  AND manifest_id = $4 AND status = 'processing'
                  AND worker_id = $5 AND fencing_token = $6",
                &[
                    &lease.tenant.tenant_id().as_str(),
                    &lease.submission_id.submitter,
                    &lease.submission_id.submission_id,
                    &lease.manifest_id,
                    &lease.worker_id.as_str(),
                    &(lease.fencing_token as i64),
                ],
            )
            .await
            .map_err(|e| LeaseError::Storage(internal_error(format!("load manifest: {e}"))))?;
        let row = rows.first().ok_or_else(|| lease_lost(lease))?;

        let manifest_url: Option<String> = row.get(0);
        let fhir_base_url: Option<String> = row.get(1);
        let output_format: Option<String> = row.get(2);
        let headers_json: Option<String> = row.get(3);
        let oauth_json: Option<String> = row.get(4);
        let encryption_json: Option<String> = row.get(5);
        let last_processed_line: i64 = row.get(6);
        let import_json: Option<String> = row.get(7);
        let metadata_json: Option<String> = row.get(8);

        let file_request_headers: Vec<(String, String)> = headers_json
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default();
        let oauth_metadata_urls: Vec<String> = oauth_json
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default();
        let file_encryption_key: Option<Value> = encryption_json
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok());
        let import_directives: Vec<(String, String)> = import_json
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default();
        let metadata: Vec<(String, String)> = metadata_json
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default();
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
            last_processed_line: last_processed_line.max(0) as u64,
            fhir_version,
        })
    }

    async fn mark_manifest_processing(&self, lease: &ManifestLease) -> Result<(), LeaseError> {
        let client = self.get_client().await.map_err(LeaseError::Storage)?;
        // Promotion only, same as the per-batch stamp: an abort landing in the
        // window between the claim and this call already moved the manifest to
        // `'failed'`, and re-marking it `'processing'` would strand it there
        // with nobody able to claim it again (#968).
        let affected = client
            .execute(
                "UPDATE bulk_manifests SET status = 'processing'
                 WHERE tenant_id = $1 AND submitter = $2 AND submission_id = $3
                   AND manifest_id = $4 AND status = 'processing'
                   AND worker_id = $5 AND fencing_token = $6",
                &[
                    &lease.tenant.tenant_id().as_str(),
                    &lease.submission_id.submitter,
                    &lease.submission_id.submission_id,
                    &lease.manifest_id,
                    &lease.worker_id.as_str(),
                    &(lease.fencing_token as i64),
                ],
            )
            .await
            .map_err(|e| LeaseError::Storage(internal_error(format!("mark processing: {e}"))))?;
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
        let client = self.get_client().await.map_err(LeaseError::Storage)?;
        // Deltas, not absolutes: the ingestion engine's per-batch bookkeeping
        // accumulates into the same columns, so an absolute `SET` here would
        // stomp its writes and walk the counters backwards on resume (#969).
        let affected = client
            .execute(
                "UPDATE bulk_manifests
                 SET processed_entries = processed_entries + $1,
                     failed_entries = failed_entries + $2,
                     last_processed_line = last_processed_line + $3
                 WHERE tenant_id = $4 AND submitter = $5 AND submission_id = $6
                   AND manifest_id = $7 AND worker_id = $8 AND fencing_token = $9",
                &[
                    &(processed_delta as i32),
                    &(failed_delta as i32),
                    &(lines_delta as i64),
                    &lease.tenant.tenant_id().as_str(),
                    &lease.submission_id.submitter,
                    &lease.submission_id.submission_id,
                    &lease.manifest_id,
                    &lease.worker_id.as_str(),
                    &(lease.fencing_token as i64),
                ],
            )
            .await
            .map_err(|e| LeaseError::Storage(internal_error(format!("update progress: {e}"))))?;
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
        let client = self.get_client().await.map_err(LeaseError::Storage)?;
        let affected = client
            .execute(
                "UPDATE bulk_manifests
                 SET bytes_processed = GREATEST(bytes_processed, $1),
                     bytes_total = GREATEST(bytes_total, $2)
                 WHERE tenant_id = $3 AND submitter = $4 AND submission_id = $5
                   AND manifest_id = $6 AND worker_id = $7 AND fencing_token = $8",
                &[
                    &(bytes_processed as i64),
                    &(bytes_total as i64),
                    &lease.tenant.tenant_id().as_str(),
                    &lease.submission_id.submitter,
                    &lease.submission_id.submission_id,
                    &lease.manifest_id,
                    &lease.worker_id.as_str(),
                    &(lease.fencing_token as i64),
                ],
            )
            .await
            .map_err(|e| LeaseError::Storage(internal_error(format!("update bytes: {e}"))))?;
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
        let client = self.get_client().await.map_err(LeaseError::Storage)?;
        // A plain overwrite, not the monotonic GREATEST the byte counters use:
        // the phases advance and their file counters restart per phase, so the
        // last write from the lease holder is the truth.
        let affected = client
            .execute(
                "UPDATE bulk_manifests
                 SET phase = $1, files_done = $2, files_total = $3
                 WHERE tenant_id = $4 AND submitter = $5 AND submission_id = $6
                   AND manifest_id = $7 AND worker_id = $8 AND fencing_token = $9",
                &[
                    &phase.to_string(),
                    &(files_done as i64),
                    &(files_total as i64),
                    &lease.tenant.tenant_id().as_str(),
                    &lease.submission_id.submitter,
                    &lease.submission_id.submission_id,
                    &lease.manifest_id,
                    &lease.worker_id.as_str(),
                    &(lease.fencing_token as i64),
                ],
            )
            .await
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
        let mut client = self.get_client().await.map_err(LeaseError::Storage)?;
        let tx = client
            .transaction()
            .await
            .map_err(|e| LeaseError::Storage(internal_error(format!("begin staging: {e}"))))?;
        let lease_token = i64::try_from(lease.fencing_token).map_err(|_| {
            LeaseError::Storage(internal_error(
                "lease fencing_token does not fit i64".to_string(),
            ))
        })?;
        let Some(state) = manifest_publication_state(&tx, lease).await? else {
            return Err(lease_lost(lease));
        };
        if state.status != "processing"
            || state.fencing_token != lease_token
            || state.worker_id.as_deref() != Some(lease.worker_id.as_str())
        {
            return Err(lease_lost(lease));
        }

        let canonical =
            canonical_publication_files(std::slice::from_ref(file)).map_err(LeaseError::Storage)?;
        validate_publication_records(state.manifest_url.as_deref(), &canonical)
            .map_err(LeaseError::Storage)?;
        let staged = read_manifest_generation_records(&tx, lease, lease_token).await?;
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
                return Err(LeaseError::Storage(publication_conflict(
                    "staged artifact identity has conflicting content",
                )));
            }
        } else {
            insert_publication_files(&tx, lease, lease_token, &canonical).await?;
        }
        tx.commit()
            .await
            .map_err(|e| LeaseError::Storage(internal_error(format!("commit staging: {e}"))))?;
        Ok(())
    }

    async fn publish_manifest_artifacts(
        &self,
        lease: &ManifestLease,
        files: &[SubmitFileRecord],
        terminal: ManifestPublicationStatus,
    ) -> Result<ManifestPublicationResult, LeaseError> {
        PostgresBackend::publish_manifest_artifacts(self, lease, files, terminal).await
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
        let client = self.get_client().await?;
        let fhir_base_url = fetch.fhir_base_url.map(|s| s.to_string());
        let output_format = fetch.output_format.map(|s| s.to_string());
        let headers_json = serde_json::to_string(fetch.file_request_headers).ok();
        let oauth_json = serde_json::to_string(fetch.oauth_metadata_urls).ok();
        let encryption_json = fetch
            .file_encryption_key
            .and_then(|v| serde_json::to_string(v).ok());
        let import_json = serde_json::to_string(fetch.import_directives).ok();
        let metadata_json = serde_json::to_string(fetch.metadata).ok();
        client
            .execute(
                "UPDATE bulk_manifests
                 SET fhir_base_url = $1, output_format = $2, file_request_headers = $3,
                     oauth_metadata_urls = $4, file_encryption_key = $5,
                     import_directives = $6, submission_metadata = $7
                 WHERE tenant_id = $8 AND submitter = $9 AND submission_id = $10
                   AND manifest_id = $11",
                &[
                    &fhir_base_url,
                    &output_format,
                    &headers_json,
                    &oauth_json,
                    &encryption_json,
                    &import_json,
                    &metadata_json,
                    &tenant.tenant_id().as_str(),
                    &id.submitter,
                    &id.submission_id,
                    &manifest_id,
                ],
            )
            .await
            .map_err(|e| internal_error(format!("set manifest fetch params: {e}")))?;
        Ok(())
    }

    async fn replace_manifest_by_url(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
        manifest_url: &str,
    ) -> StorageResult<Vec<String>> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();
        let rows = client
            .query(
                "SELECT manifest_id FROM bulk_manifests
                 WHERE tenant_id = $1 AND submitter = $2 AND submission_id = $3
                   AND manifest_url = $4 AND status != 'replaced'",
                &[&tenant_id, &id.submitter, &id.submission_id, &manifest_url],
            )
            .await
            .map_err(|e| internal_error(format!("replace lookup: {e}")))?;
        let ids: Vec<String> = rows.iter().map(|r| r.get::<_, String>(0)).collect();
        client
            .execute(
                "UPDATE bulk_manifests SET status = 'replaced'
                 WHERE tenant_id = $1 AND submitter = $2 AND submission_id = $3 AND manifest_url = $4",
                &[&tenant_id, &id.submitter, &id.submission_id, &manifest_url],
            )
            .await
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
        let client = self.get_client().await?;
        let owner = owner_subject.map(|s| s.to_string());
        client
            .execute(
                "UPDATE bulk_submissions
                 SET owner_subject = $1, request_url = $2, requires_access_token = $3
                 WHERE tenant_id = $4 AND submitter = $5 AND submission_id = $6",
                &[
                    &owner,
                    &request_url,
                    &requires_access_token,
                    &tenant.tenant_id().as_str(),
                    &id.submitter,
                    &id.submission_id,
                ],
            )
            .await
            .map_err(|e| internal_error(format!("set kickoff meta: {e}")))?;
        Ok(())
    }

    async fn ensure_poll_token(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<String> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();
        let rows = client
            .query(
                "SELECT poll_token FROM bulk_submissions
                 WHERE tenant_id = $1 AND submitter = $2 AND submission_id = $3",
                &[&tenant_id, &id.submitter, &id.submission_id],
            )
            .await
            .map_err(|e| internal_error(format!("read poll token: {e}")))?;
        if let Some(row) = rows.first() {
            let existing: Option<String> = row.get(0);
            if let Some(token) = existing {
                return Ok(token);
            }
        }
        let token = Uuid::new_v4().to_string();
        client
            .execute(
                "UPDATE bulk_submissions SET poll_token = $1
                 WHERE tenant_id = $2 AND submitter = $3 AND submission_id = $4",
                &[&token, &tenant_id, &id.submitter, &id.submission_id],
            )
            .await
            .map_err(|e| internal_error(format!("set poll token: {e}")))?;
        Ok(token)
    }

    async fn list_expired_submissions(
        &self,
        now: DateTime<Utc>,
        ttl: StdDuration,
        limit: u32,
    ) -> StorageResult<Vec<(TenantContext, SubmissionId)>> {
        let client = self.get_client().await?;
        let cutoff = now
            - chrono::Duration::from_std(ttl).unwrap_or_else(|_| chrono::Duration::seconds(86400));
        let rows = client
            .query(
                "SELECT tenant_id, submitter, submission_id FROM bulk_submissions
                 WHERE updated_at < $1 ORDER BY updated_at LIMIT $2",
                &[&cutoff, &(limit as i64)],
            )
            .await
            .map_err(|e| internal_error(format!("list expired: {e}")))?;
        Ok(rows
            .iter()
            .map(|r| {
                let tenant_id: String = r.get(0);
                let submitter: String = r.get(1);
                let submission_id: String = r.get(2);
                (
                    TenantContext::new(TenantId::new(tenant_id), TenantPermissions::full_access()),
                    SubmissionId::new(submitter, submission_id),
                )
            })
            .collect())
    }

    async fn resolve_poll_token(&self, token: &str) -> StorageResult<Option<PollTokenTarget>> {
        let client = self.get_client().await?;
        let rows = client
            .query(
                "SELECT tenant_id, submitter, submission_id, owner_subject
                 FROM bulk_submissions WHERE poll_token = $1",
                &[&token],
            )
            .await
            .map_err(|e| internal_error(format!("resolve poll token: {e}")))?;
        Ok(rows.first().map(|row| {
            let tenant_id: String = row.get(0);
            let submitter: String = row.get(1);
            let submission_id: String = row.get(2);
            let owner_subject: Option<String> = row.get(3);
            PollTokenTarget {
                tenant: TenantContext::new(
                    TenantId::new(tenant_id),
                    TenantPermissions::full_access(),
                ),
                submission_id: SubmissionId::new(submitter, submission_id),
                owner_subject,
            }
        }))
    }

    async fn clear_poll_token(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<()> {
        let client = self.get_client().await?;
        client
            .execute(
                "UPDATE bulk_submissions SET poll_token = NULL
                 WHERE tenant_id = $1 AND submitter = $2 AND submission_id = $3",
                &[
                    &tenant.tenant_id().as_str(),
                    &id.submitter,
                    &id.submission_id,
                ],
            )
            .await
            .map_err(|e| internal_error(format!("clear poll token: {e}")))?;
        Ok(())
    }

    async fn list_submit_files(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<Vec<SubmitFileRow>> {
        let client = self.get_client().await?;
        let rows = client
            .query(
                "SELECT f.manifest_url, f.file_type, f.resource_type, f.part_index,
                        f.fencing_token,
                        file_path, line_count, byte_count, count_severity,
                        f.manifest_id, m.publication_worker_id
                 FROM bulk_submit_files AS f
                 INNER JOIN bulk_manifests AS m
                   ON m.tenant_id = f.tenant_id
                      AND m.submitter = f.submitter
                      AND m.submission_id = f.submission_id
                      AND m.manifest_id = f.manifest_id
                      AND m.published_token = f.fencing_token
                 WHERE f.manifest_id IS NOT NULL
                   AND f.tenant_id = $1
                   AND f.submitter = $2
                   AND f.submission_id = $3
                   AND f.publication_excluded_reason IS NULL
                   AND m.published_token IS NOT NULL
                   AND m.publication_status IN ('completed', 'failed')
                 ORDER BY f.id",
                &[
                    &tenant.tenant_id().as_str(),
                    &id.submitter,
                    &id.submission_id,
                ],
            )
            .await
            .map_err(|e| internal_error(format!("list submit files: {e}")))?;
        rows.iter()
            .map(|row| {
                let manifest_url: Option<String> = row.get(0);
                let file_type: String = row.get(1);
                let resource_type: Option<String> = row.get(2);
                let part_index: i32 = row.get(3);
                let fencing_token: i64 = row.get(4);
                let file_path: String = row.get(5);
                let line_count: i64 = row.get(6);
                let byte_count: i64 = row.get(7);
                let count_severity: Option<String> = row.get(8);
                let manifest_id: Option<String> = row.get(9);
                let publication_worker_id: Option<String> = row.get(10);
                publication_row_from_parts(
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
                )
            })
            .collect::<StorageResult<Vec<_>>>()
    }

    async fn delete_submission_artifacts(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<()> {
        let mut client = self.get_client().await?;
        let tx = client
            .transaction()
            .await
            .map_err(|e| internal_error(format!("begin artifact cleanup: {e}")))?;
        tx.query(
            "SELECT manifest_id FROM bulk_manifests
             WHERE tenant_id = $1 AND submitter = $2 AND submission_id = $3
             ORDER BY added_at, manifest_id
             FOR UPDATE",
            &[
                &tenant.tenant_id().as_str(),
                &id.submitter,
                &id.submission_id,
            ],
        )
        .await
        .map_err(|e| internal_error(format!("lock manifest rows: {e}")))?;
        tx.execute(
            "DELETE FROM bulk_submit_files
                 WHERE tenant_id = $1 AND submitter = $2 AND submission_id = $3",
            &[
                &tenant.tenant_id().as_str(),
                &id.submitter,
                &id.submission_id,
            ],
        )
        .await
        .map_err(|e| internal_error(format!("delete artifacts: {e}")))?;
        tx.execute(
            "UPDATE bulk_manifests
                 SET published_token = NULL, publication_status = NULL,
                     publication_error_message = NULL, publication_worker_id = NULL
                 WHERE tenant_id = $1 AND submitter = $2 AND submission_id = $3",
            &[
                &tenant.tenant_id().as_str(),
                &id.submitter,
                &id.submission_id,
            ],
        )
        .await
        .map_err(|e| internal_error(format!("clear publication markers: {e}")))?;
        tx.commit()
            .await
            .map_err(|e| internal_error(format!("commit artifact cleanup: {e}")))?;
        Ok(())
    }

    async fn count_active_submissions(&self, tenant: &TenantContext) -> StorageResult<u64> {
        let client = self.get_client().await?;
        let row = client
            .query_one(
                "SELECT COUNT(*) FROM bulk_submissions s
                 WHERE s.tenant_id = $1 AND s.status = 'in-progress'
                   AND (NOT EXISTS (SELECT 1 FROM bulk_manifests m
                                    WHERE m.tenant_id = s.tenant_id
                                      AND m.submitter = s.submitter
                                      AND m.submission_id = s.submission_id)
                        OR EXISTS (SELECT 1 FROM bulk_manifests m
                                   WHERE m.tenant_id = s.tenant_id
                                     AND m.submitter = s.submitter
                                     AND m.submission_id = s.submission_id
                                     AND m.status IN ('pending', 'processing')))",
                &[&tenant.tenant_id().as_str()],
            )
            .await
            .map_err(|e| internal_error(format!("count active submissions: {e}")))?;
        let count: i64 = row.get(0);
        Ok(count.max(0) as u64)
    }

    async fn ensure_transaction_time(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<DateTime<Utc>> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();
        let rows = client
            .query(
                "SELECT transaction_time FROM bulk_submissions
                 WHERE tenant_id = $1 AND submitter = $2 AND submission_id = $3",
                &[&tenant_id, &id.submitter, &id.submission_id],
            )
            .await
            .map_err(|e| internal_error(format!("read transaction_time: {e}")))?;
        if let Some(row) = rows.first() {
            let existing: Option<DateTime<Utc>> = row.get(0);
            if let Some(dt) = existing {
                return Ok(dt);
            }
        }
        let now = Utc::now();
        client
            .execute(
                "UPDATE bulk_submissions SET transaction_time = $1
                 WHERE tenant_id = $2 AND submitter = $3 AND submission_id = $4",
                &[&now, &tenant_id, &id.submitter, &id.submission_id],
            )
            .await
            .map_err(|e| internal_error(format!("set transaction_time: {e}")))?;
        Ok(now)
    }
}

impl PostgresBackend {
    /// Atomically replaces and publishes the complete artifact set for a lease.
    ///
    /// State selection, staged-row replacement, and the terminal fencing update
    /// all run on one locked manifest row. A successful commit is the only point
    /// at which the generation becomes visible to publication queries.
    pub async fn publish_manifest_artifacts(
        &self,
        lease: &ManifestLease,
        files: &[SubmitFileRecord],
        terminal: ManifestPublicationStatus,
    ) -> Result<ManifestPublicationResult, LeaseError> {
        let canonical = canonical_publication_files(files).map_err(LeaseError::Storage)?;
        let mut client = self.get_client().await.map_err(LeaseError::Storage)?;
        let tx = client
            .transaction()
            .await
            .map_err(|e| LeaseError::Storage(internal_error(format!("begin publication: {e}"))))?;
        let Some(outcome) =
            publish_manifest_artifacts_in_tx(&tx, lease, &canonical, &terminal).await?
        else {
            return Err(lease_lost(lease));
        };
        tx.commit()
            .await
            .map_err(|e| LeaseError::Storage(internal_error(format!("commit publication: {e}"))))?;
        Ok(outcome)
    }

    /// Publishes the generation already staged for `lease`. Terminal finish and
    /// failure read the staged set inside the publication transaction, so a
    /// concurrent staging write cannot be omitted from the atomic commit.
    async fn publish_current_manifest_generation(
        &self,
        lease: &ManifestLease,
        terminal: ManifestPublicationStatus,
    ) -> Result<(), LeaseError> {
        let mut client = self.get_client().await.map_err(LeaseError::Storage)?;
        let tx = client.transaction().await.map_err(|e| {
            LeaseError::Storage(internal_error(format!("begin finish publication: {e}")))
        })?;
        let lease_token = i64::try_from(lease.fencing_token).map_err(|_| {
            LeaseError::Storage(internal_error(
                "lease fencing_token does not fit i64".to_string(),
            ))
        })?;
        let Some(state) = manifest_publication_state(&tx, lease).await? else {
            return Err(lease_lost(lease));
        };
        if state.status == "replaced" || state.fencing_token != lease_token {
            return Err(lease_lost(lease));
        }

        let staged = if state.status == "processing" {
            if state.worker_id.as_deref() != Some(lease.worker_id.as_str()) {
                return Err(lease_lost(lease));
            }
            read_manifest_generation_records(&tx, lease, lease_token).await?
        } else {
            let published_token_matches = state.published_token == Some(lease_token);
            let publisher_matches =
                state.publication_worker_id.as_deref() == Some(lease.worker_id.as_str());
            if !published_token_matches || !publisher_matches {
                return Err(lease_lost(lease));
            }
            let (terminal_status, terminal_error) = publication_status_parts(&terminal);
            if state.status != terminal_status
                || state.publication_status.as_deref() != Some(terminal_status)
                || state.publication_error_message != terminal_error.map(str::to_string)
            {
                return Err(LeaseError::Storage(publication_conflict(
                    "terminal state changed for the same publication",
                )));
            }
            read_manifest_generation_records(&tx, lease, lease_token).await?
        };
        let staged = canonical_publication_files(&staged).map_err(LeaseError::Storage)?;
        let Some(outcome) =
            publish_manifest_artifacts_in_tx(&tx, lease, &staged, &terminal).await?
        else {
            return Err(lease_lost(lease));
        };
        let _ = outcome;
        tx.commit().await.map_err(|e| {
            LeaseError::Storage(internal_error(format!("commit finish publication: {e}")))
        })?;
        Ok(())
    }
}

/// Reads the publication state with its manifest row locked. A missing identity
/// means lease loss; every query error is a storage error.
async fn manifest_publication_state(
    tx: &tokio_postgres::Transaction<'_>,
    lease: &ManifestLease,
) -> StorageResult<Option<ManifestPublicationState>> {
    let row = tx
        .query_opt(
            "SELECT manifest_url, status, fencing_token, worker_id,
                    published_token, publication_status, publication_error_message,
                    publication_worker_id
             FROM bulk_manifests
             WHERE tenant_id = $1 AND submitter = $2 AND submission_id = $3
               AND manifest_id = $4
             FOR UPDATE",
            &[
                &lease.tenant.tenant_id().as_str(),
                &lease.submission_id.submitter,
                &lease.submission_id.submission_id,
                &lease.manifest_id,
            ],
        )
        .await
        .map_err(|e| internal_error(format!("read publication state: {e}")))?;
    let Some(row) = row else {
        return Ok(None);
    };
    let manifest_url: Option<String> = row.get(0);
    let status: String = row.get(1);
    let fencing_token: i64 = row.get(2);
    let worker_id: Option<String> = row.get(3);
    let published_token: Option<i64> = row.get(4);
    let publication_status: Option<String> = row.get(5);
    let publication_error_message: Option<String> = row.get(6);
    let publication_worker_id: Option<String> = row.get(7);
    Ok(Some(ManifestPublicationState {
        manifest_url,
        status,
        fencing_token,
        worker_id,
        published_token,
        publication_status,
        publication_error_message,
        publication_worker_id,
    }))
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

async fn read_manifest_generation_records(
    tx: &tokio_postgres::Transaction<'_>,
    lease: &ManifestLease,
    fencing_token: i64,
) -> StorageResult<Vec<SubmitFileRecord>> {
    let rows = tx
        .query(
            "SELECT manifest_url, file_type, resource_type, part_index, file_path,
                    line_count, byte_count, count_severity
             FROM bulk_submit_files
             WHERE tenant_id = $1 AND submitter = $2 AND submission_id = $3
               AND manifest_id = $4 AND fencing_token = $5
               AND publication_excluded_reason IS NULL
             ORDER BY id",
            &[
                &lease.tenant.tenant_id().as_str(),
                &lease.submission_id.submitter,
                &lease.submission_id.submission_id,
                &lease.manifest_id,
                &fencing_token,
            ],
        )
        .await
        .map_err(|e| internal_error(format!("read publication rows: {e}")))?;
    let mut records = Vec::new();
    for row in rows {
        let manifest_url: Option<String> = row.get(0);
        let file_type: String = row.get(1);
        let resource_type: Option<String> = row.get(2);
        let part_index: i32 = row.get(3);
        let file_path: String = row.get(4);
        let line_count: i64 = row.get(5);
        let byte_count: i64 = row.get(6);
        let count_severity: Option<String> = row.get(7);
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
    manifest_url: Option<&str>,
    records: &[SubmitFileRecord],
) -> StorageResult<()> {
    for file in records {
        if !matches!(file.file_type.as_str(), "output" | "error" | "deleted") {
            return Err(publication_conflict(format!(
                "invalid file_type: {}",
                file.file_type
            )));
        }
        match (&file.manifest_url, manifest_url) {
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

#[allow(clippy::too_many_arguments)]
fn publication_record_from_parts(
    manifest_url: Option<String>,
    file_type: String,
    resource_type: Option<String>,
    part_index: i32,
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
        .map_err(|_| internal_error("published part_index is negative".to_string()))?;
    if fencing_token < 0 {
        return Err(internal_error(
            "published fencing_token is negative".to_string(),
        ));
    }
    let line_count = u64::try_from(line_count)
        .map_err(|_| internal_error("published line_count is negative".to_string()))?;
    let byte_count = u64::try_from(byte_count)
        .map_err(|_| internal_error("published byte_count is negative".to_string()))?;
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

#[allow(clippy::too_many_arguments)]
fn publication_row_from_parts(
    manifest_url: Option<String>,
    file_type: String,
    resource_type: Option<String>,
    part_index: i32,
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
async fn publish_manifest_artifacts_in_tx(
    tx: &tokio_postgres::Transaction<'_>,
    lease: &ManifestLease,
    canonical: &[SubmitFileRecord],
    terminal: &ManifestPublicationStatus,
) -> StorageResult<Option<ManifestPublicationResult>> {
    let lease_token = i64::try_from(lease.fencing_token)
        .map_err(|_| internal_error("lease fencing_token does not fit i64".to_string()))?;
    let Some(state) = manifest_publication_state(tx, lease).await? else {
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

        let persisted = read_manifest_generation_records(tx, lease, lease_token).await?;
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

    validate_publication_records(state.manifest_url.as_deref(), canonical)?;
    delete_staged_publication_files(tx, lease, lease_token).await?;
    insert_publication_files(tx, lease, lease_token, canonical).await?;

    let (terminal_status, terminal_error) = publication_status_parts(terminal);
    let affected = tx
        .execute(
            "UPDATE bulk_manifests
             SET status = $5, published_token = $6, publication_status = $7,
                 publication_error_message = $8, publication_worker_id = $9,
                 worker_id = NULL, lease_expiry = NULL
             WHERE tenant_id = $1 AND submitter = $2 AND submission_id = $3
               AND manifest_id = $4 AND status = 'processing'
               AND worker_id = $10 AND fencing_token = $11",
            &[
                &lease.tenant.tenant_id().as_str(),
                &lease.submission_id.submitter,
                &lease.submission_id.submission_id,
                &lease.manifest_id,
                &terminal_status,
                &lease_token,
                &terminal_status,
                &terminal_error,
                &lease.worker_id.as_str(),
                &lease.worker_id.as_str(),
                &lease_token,
            ],
        )
        .await
        .map_err(|e| internal_error(format!("publish manifest: {e}")))?;
    if affected != 1 {
        return Ok(None);
    }
    Ok(Some(ManifestPublicationResult::Published))
}

async fn delete_staged_publication_files(
    tx: &tokio_postgres::Transaction<'_>,
    lease: &ManifestLease,
    fencing_token: i64,
) -> StorageResult<()> {
    tx.execute(
        "DELETE FROM bulk_submit_files
         WHERE tenant_id = $1 AND submitter = $2 AND submission_id = $3
           AND manifest_id = $4 AND fencing_token = $5",
        &[
            &lease.tenant.tenant_id().as_str(),
            &lease.submission_id.submitter,
            &lease.submission_id.submission_id,
            &lease.manifest_id,
            &fencing_token,
        ],
    )
    .await
    .map_err(|e| internal_error(format!("delete staged files: {e}")))?;
    Ok(())
}

async fn insert_publication_files(
    tx: &tokio_postgres::Transaction<'_>,
    lease: &ManifestLease,
    fencing_token: i64,
    files: &[SubmitFileRecord],
) -> StorageResult<()> {
    for file in files {
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
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, NULL)",
            &[
                &lease.tenant.tenant_id().as_str(),
                &lease.submission_id.submitter,
                &lease.submission_id.submission_id,
                &lease.manifest_id,
                &file.manifest_url,
                &file.file_type,
                &file.resource_type,
                &(file.part_index as i32),
                &fencing_token,
                &file.file_path,
                &(file.line_count as i64),
                &(file.byte_count as i64),
                &count_severity,
                &Utc::now(),
            ],
        )
        .await
        .map_err(|e| internal_error(format!("insert publication file: {e}")))?;
    }
    Ok(())
}
