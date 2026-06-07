//! Bulk submit implementation for PostgreSQL backend.

use async_trait::async_trait;
use chrono::Utc;
use helios_fhir::FhirVersion;
use serde_json::Value;
use tokio::io::{AsyncBufRead, AsyncBufReadExt};
use uuid::Uuid;

use crate::core::ResourceStorage;
use crate::core::bulk_submit::{
    BulkEntryOutcome, BulkEntryResult, BulkProcessingOptions, BulkSubmitProvider,
    BulkSubmitRollbackProvider, ChangeType, EntryCountSummary, ManifestStatus, NdjsonEntry,
    StreamProcessingResult, StreamingBulkSubmitProvider, SubmissionChange, SubmissionId,
    SubmissionManifest, SubmissionStatus, SubmissionSummary,
};
use crate::error::{BackendError, BulkSubmitError, StorageError, StorageResult};
use crate::tenant::TenantContext;

use super::PostgresBackend;

fn internal_error(message: String) -> StorageError {
    StorageError::Backend(BackendError::Internal {
        backend_name: "postgres".to_string(),
        message,
        source: None,
    })
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
                "SELECT manifest_url, replaces_manifest_url, status, added_at, total_entries, processed_entries, failed_entries
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
        let total: i64 = row.get(4);
        let processed: i64 = row.get(5);
        let failed: i64 = row.get(6);

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
        let client = self.get_client().await?;
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

        // Update manifest status to processing
        client
            .execute(
                "UPDATE bulk_manifests SET status = 'processing'
                 WHERE tenant_id = $1 AND submitter = $2 AND submission_id = $3 AND manifest_id = $4",
                &[
                    &tenant_id,
                    &submission_id.submitter.as_str(),
                    &submission_id.submission_id.as_str(),
                    &manifest_id,
                ],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to update manifest status: {}", e)))?;

        let mut results = Vec::new();
        let mut error_count = 0u32;

        for entry in entries {
            if options.max_errors > 0 && error_count >= options.max_errors {
                if !options.continue_on_error {
                    return Err(StorageError::BulkSubmit(
                        BulkSubmitError::MaxErrorsExceeded {
                            submission_id: submission_id.submission_id.clone(),
                            max_errors: options.max_errors,
                        },
                    ));
                }
                let skip_result = BulkEntryResult::skipped(
                    entry.line_number,
                    &entry.resource_type,
                    "max errors exceeded",
                );
                results.push(skip_result);
                continue;
            }

            let result = self
                .process_single_entry(tenant, submission_id, manifest_id, &entry, options)
                .await;

            let entry_result = match result {
                Ok(r) => r,
                Err(e) => {
                    error_count += 1;
                    BulkEntryResult::processing_error(
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
                    )
                }
            };

            if entry_result.is_error() {
                error_count += 1;
            }

            self.store_entry_result(tenant, submission_id, manifest_id, &entry_result)
                .await?;

            results.push(entry_result);
        }

        // Update manifest counts
        let now = Utc::now();
        client
            .execute(
                "UPDATE bulk_manifests SET
                    total_entries = total_entries + $1,
                    processed_entries = processed_entries + $2,
                    failed_entries = failed_entries + $3
                 WHERE tenant_id = $4 AND submitter = $5 AND submission_id = $6 AND manifest_id = $7",
                &[
                    &(results.len() as i64),
                    &(results.iter().filter(|r| r.is_success()).count() as i64),
                    &(error_count as i64),
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

    async fn get_entry_results(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_id: &str,
        outcome_filter: Option<BulkEntryOutcome>,
        limit: u32,
        offset: u32,
    ) -> StorageResult<Vec<BulkEntryResult>> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        let mut sql =
            "SELECT line_number, resource_type, resource_id, created, outcome, operation_outcome
             FROM bulk_entry_results
             WHERE tenant_id = $1 AND submitter = $2 AND submission_id = $3 AND manifest_id = $4"
                .to_string();

        let mut params: Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>> = vec![
            Box::new(tenant_id.to_string()),
            Box::new(submission_id.submitter.clone()),
            Box::new(submission_id.submission_id.clone()),
            Box::new(manifest_id.to_string()),
        ];

        if let Some(outcome) = outcome_filter {
            sql.push_str(" AND outcome = $5");
            params.push(Box::new(outcome.to_string()));
        }

        sql.push_str(&format!(
            " ORDER BY line_number LIMIT {} OFFSET {}",
            limit, offset
        ));

        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = params
            .iter()
            .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        let rows = client
            .query(&sql, &param_refs)
            .await
            .map_err(|e| internal_error(format!("Failed to query results: {}", e)))?;

        let results: Vec<BulkEntryResult> = rows
            .iter()
            .map(|row| {
                let line_number: i64 = row.get(0);
                let resource_type: String = row.get(1);
                let resource_id: Option<String> = row.get(2);
                let created: Option<bool> = row.get(3);
                let outcome_str: String = row.get(4);
                let operation_outcome: Option<Value> = row.get(5);

                let outcome: BulkEntryOutcome = outcome_str
                    .parse()
                    .unwrap_or(BulkEntryOutcome::ProcessingError);

                BulkEntryResult {
                    line_number: line_number as u64,
                    resource_type,
                    resource_id,
                    created: created.unwrap_or(false),
                    outcome,
                    operation_outcome,
                }
            })
            .collect();

        Ok(results)
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
}

impl PostgresBackend {
    /// Process a single NDJSON entry.
    async fn process_single_entry(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_id: &str,
        entry: &NdjsonEntry,
        options: &BulkProcessingOptions,
    ) -> StorageResult<BulkEntryResult> {
        let resource_id = entry.resource_id.as_ref();

        if let Some(id) = resource_id {
            let existing = self.read(tenant, &entry.resource_type, id).await;

            match existing {
                Ok(Some(current)) => {
                    if !options.allow_updates {
                        return Ok(BulkEntryResult::skipped(
                            entry.line_number,
                            &entry.resource_type,
                            "updates not allowed",
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
                    self.record_change(tenant, submission_id, &change).await?;

                    let updated = self
                        .update(tenant, &current, entry.resource.clone())
                        .await?;

                    Ok(BulkEntryResult::success(
                        entry.line_number,
                        &entry.resource_type,
                        updated.id(),
                        false,
                    ))
                }
                Ok(None)
                | Err(StorageError::Resource(crate::error::ResourceError::Gone { .. })) => {
                    let created = self
                        .create(
                            tenant,
                            &entry.resource_type,
                            entry.resource.clone(),
                            FhirVersion::default_enabled(),
                        )
                        .await?;

                    let change = SubmissionChange::create(
                        manifest_id,
                        &entry.resource_type,
                        created.id(),
                        created.version_id(),
                    );
                    self.record_change(tenant, submission_id, &change).await?;

                    Ok(BulkEntryResult::success(
                        entry.line_number,
                        &entry.resource_type,
                        created.id(),
                        true,
                    ))
                }
                Err(e) => Err(e),
            }
        } else {
            let created = self
                .create(
                    tenant,
                    &entry.resource_type,
                    entry.resource.clone(),
                    FhirVersion::default_enabled(),
                )
                .await?;

            let change = SubmissionChange::create(
                manifest_id,
                &entry.resource_type,
                created.id(),
                created.version_id(),
            );
            self.record_change(tenant, submission_id, &change).await?;

            Ok(BulkEntryResult::success(
                entry.line_number,
                &entry.resource_type,
                created.id(),
                true,
            ))
        }
    }

    /// Store an entry result in the database.
    async fn store_entry_result(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_id: &str,
        result: &BulkEntryResult,
    ) -> StorageResult<()> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        let outcome_json: Option<Value> = result.operation_outcome.clone();

        client
            .execute(
                "INSERT INTO bulk_entry_results
                 (tenant_id, submitter, submission_id, manifest_id, line_number, resource_type, resource_id, created, outcome, operation_outcome)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
                &[
                    &tenant_id,
                    &submission_id.submitter.as_str(),
                    &submission_id.submission_id.as_str(),
                    &manifest_id,
                    &(result.line_number as i64),
                    &result.resource_type.as_str(),
                    &result.resource_id,
                    &result.created,
                    &result.outcome.to_string().as_str(),
                    &outcome_json,
                ],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to store entry result: {}", e)))?;

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
                        result.counts.increment(error_result.outcome);

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
