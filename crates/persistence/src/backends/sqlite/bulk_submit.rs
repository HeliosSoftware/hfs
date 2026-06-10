//! Bulk submit implementation for SQLite backend.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use helios_fhir::FhirVersion;
use rusqlite::params;
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
    BulkSubmitRollbackProvider, ChangeType, EntryCountSummary, ManifestStatus, NdjsonEntry,
    StreamProcessingResult, StreamingBulkSubmitProvider, SubmissionChange, SubmissionId,
    SubmissionManifest, SubmissionStatus, SubmissionSummary,
};
use crate::core::bulk_submit_worker::{
    ManifestLease, ManifestWorkerView, PollTokenTarget, SubmitClaimStrategy, SubmitFileRecord,
    SubmitFileRow, SubmitWorkerStorage,
};
use crate::error::{BackendError, BulkSubmitError, StorageError, StorageResult};
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
        .unwrap_or_default()
}

fn internal_error(message: String) -> StorageError {
    StorageError::Backend(BackendError::Internal {
        backend_name: "sqlite".to_string(),
        message,
        source: None,
    })
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
            "SELECT manifest_url, replaces_manifest_url, status, added_at, total_entries, processed_entries, failed_entries
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
        let conn = self.get_connection()?;
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
        conn.execute(
            "UPDATE bulk_manifests SET status = 'processing'
             WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3 AND manifest_id = ?4",
            params![
                tenant_id,
                &submission_id.submitter,
                &submission_id.submission_id,
                manifest_id
            ],
        )
        .map_err(|e| internal_error(format!("Failed to update manifest status: {}", e)))?;

        let mut results = Vec::new();
        let mut error_count = 0u32;

        for entry in entries {
            // Check if we've hit max errors
            if options.max_errors > 0 && error_count >= options.max_errors {
                if !options.continue_on_error {
                    return Err(StorageError::BulkSubmit(
                        BulkSubmitError::MaxErrorsExceeded {
                            submission_id: submission_id.submission_id.clone(),
                            max_errors: options.max_errors,
                        },
                    ));
                }
                // Skip remaining entries
                let skip_result = BulkEntryResult::skipped(
                    entry.line_number,
                    &entry.resource_type,
                    "max errors exceeded",
                );
                results.push(skip_result);
                continue;
            }

            // Process the entry
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

            // Store the result
            self.store_entry_result(tenant, submission_id, manifest_id, &entry_result)
                .await?;

            results.push(entry_result);
        }

        // Update manifest counts
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "UPDATE bulk_manifests SET
                total_entries = total_entries + ?1,
                processed_entries = processed_entries + ?2,
                failed_entries = failed_entries + ?3
             WHERE tenant_id = ?4 AND submitter = ?5 AND submission_id = ?6 AND manifest_id = ?7",
            params![
                results.len() as i64,
                results.iter().filter(|r| r.is_success()).count() as i64,
                error_count as i64,
                tenant_id,
                &submission_id.submitter,
                &submission_id.submission_id,
                manifest_id
            ],
        )
        .map_err(|e| internal_error(format!("Failed to update manifest counts: {}", e)))?;

        // Update submission updated_at
        conn.execute(
            "UPDATE bulk_submissions SET updated_at = ?1
             WHERE tenant_id = ?2 AND submitter = ?3 AND submission_id = ?4",
            params![
                now,
                tenant_id,
                &submission_id.submitter,
                &submission_id.submission_id
            ],
        )
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
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        let mut query =
            "SELECT line_number, resource_type, resource_id, created, outcome, operation_outcome
             FROM bulk_entry_results
             WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3 AND manifest_id = ?4"
                .to_string();

        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![
            Box::new(tenant_id.to_string()),
            Box::new(submission_id.submitter.clone()),
            Box::new(submission_id.submission_id.clone()),
            Box::new(manifest_id.to_string()),
        ];

        if let Some(outcome) = outcome_filter {
            query.push_str(" AND outcome = ?");
            params_vec.push(Box::new(outcome.to_string()));
        }

        query.push_str(" ORDER BY line_number");
        query.push_str(&format!(" LIMIT {} OFFSET {}", limit, offset));

        let params_slice: Vec<&dyn rusqlite::ToSql> =
            params_vec.iter().map(|p| p.as_ref()).collect();

        let mut stmt = conn
            .prepare(&query)
            .map_err(|e| internal_error(format!("Failed to prepare results query: {}", e)))?;

        let results: Vec<BulkEntryResult> = stmt
            .query_map(params_slice.as_slice(), |row| {
                let line_number: i64 = row.get(0)?;
                let resource_type: String = row.get(1)?;
                let resource_id: Option<String> = row.get(2)?;
                let created: Option<i32> = row.get(3)?;
                let outcome_str: String = row.get(4)?;
                let operation_outcome_bytes: Option<Vec<u8>> = row.get(5)?;

                let outcome: BulkEntryOutcome = outcome_str
                    .parse()
                    .unwrap_or(BulkEntryOutcome::ProcessingError);

                let operation_outcome =
                    operation_outcome_bytes.and_then(|b| serde_json::from_slice(&b).ok());

                Ok(BulkEntryResult {
                    line_number: line_number as u64,
                    resource_type,
                    resource_id,
                    created: created.map(|c| c != 0).unwrap_or(false),
                    outcome,
                    operation_outcome,
                })
            })
            .map_err(|e| internal_error(format!("Failed to query results: {}", e)))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(results)
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
}

impl SqliteBackend {
    /// Process a single NDJSON entry.
    async fn process_single_entry(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_id: &str,
        entry: &NdjsonEntry,
        options: &BulkProcessingOptions,
    ) -> StorageResult<BulkEntryResult> {
        // Check if resource has an ID
        let resource_id = entry.resource_id.as_ref();

        if let Some(id) = resource_id {
            // Check if resource exists
            let existing = self.read(tenant, &entry.resource_type, id).await;

            match existing {
                Ok(Some(current)) => {
                    // Resource exists - update if allowed
                    if !options.allow_updates {
                        return Ok(BulkEntryResult::skipped(
                            entry.line_number,
                            &entry.resource_type,
                            "updates not allowed",
                        ));
                    }

                    // Record change for rollback
                    let change = SubmissionChange::update(
                        manifest_id,
                        &entry.resource_type,
                        id,
                        current.version_id(),
                        (current.version_id().parse::<i32>().unwrap_or(0) + 1).to_string(),
                        current.content().clone(),
                    );
                    self.record_change(tenant, submission_id, &change).await?;

                    // Update the resource
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
                    // Resource doesn't exist - create it
                    // Use default FHIR version for bulk operations
                    let created = self
                        .create(
                            tenant,
                            &entry.resource_type,
                            entry.resource.clone(),
                            FhirVersion::default_enabled(),
                        )
                        .await?;

                    // Record change for rollback
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
            // No ID - create new resource
            // Use default FHIR version for bulk operations
            let created = self
                .create(
                    tenant,
                    &entry.resource_type,
                    entry.resource.clone(),
                    FhirVersion::default_enabled(),
                )
                .await?;

            // Record change for rollback
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
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        let outcome_bytes = result
            .operation_outcome
            .as_ref()
            .and_then(|o| serde_json::to_vec(o).ok());

        conn.execute(
            "INSERT INTO bulk_entry_results
             (tenant_id, submitter, submission_id, manifest_id, line_number, resource_type, resource_id, created, outcome, operation_outcome)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                tenant_id,
                &submission_id.submitter,
                &submission_id.submission_id,
                manifest_id,
                result.line_number as i64,
                &result.resource_type,
                &result.resource_id,
                if result.created { Some(1) } else { Some(0) },
                result.outcome.to_string(),
                outcome_bytes
            ],
        )
        .map_err(|e| internal_error(format!("Failed to store entry result: {}", e)))?;

        Ok(())
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
            match NdjsonEntry::parse(line_number, line) {
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
                 WHERE m.manifest_url IS NOT NULL
                   AND s.status = 'in-progress'
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
            fencing_token: new_token as u64,
        }))
    }

    async fn heartbeat(&self, lease: &ManifestLease) -> Result<DateTime<Utc>, LeaseError> {
        let conn = self.get_connection().map_err(LeaseError::Storage)?;
        let now = Utc::now();
        let new_expiry = now + chrono::Duration::seconds(60);
        let affected = conn
            .execute(
                "UPDATE bulk_manifests SET lease_expiry = ?1
                 WHERE tenant_id = ?2 AND submitter = ?3 AND submission_id = ?4
                   AND manifest_id = ?5 AND worker_id = ?6 AND fencing_token = ?7",
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
            .map_err(|e| LeaseError::Storage(internal_error(format!("heartbeat failed: {e}"))))?;
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
        );
        let row: Row = conn
            .query_row(
                "SELECT manifest_url, fhir_base_url, output_format, file_request_headers,
                        oauth_metadata_urls, file_encryption_key, last_processed_line
                 FROM bulk_manifests
                 WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3
                   AND manifest_id = ?4 AND worker_id = ?5 AND fencing_token = ?6",
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
                    ))
                },
            )
            .map_err(|_| lease_lost(lease))?;

        let (
            manifest_url,
            fhir_base_url,
            output_format,
            headers_json,
            oauth_json,
            encryption_json,
            last_processed_line,
        ) = row;

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
        let fhir_version = fhir_version_from_output_format(output_format.as_deref());

        Ok(ManifestWorkerView {
            manifest_id: lease.manifest_id.clone(),
            manifest_url,
            fhir_base_url,
            output_format,
            file_request_headers,
            oauth_metadata_urls,
            file_encryption_key,
            last_processed_line: last_processed_line.max(0) as u64,
            fhir_version,
        })
    }

    async fn mark_manifest_processing(&self, lease: &ManifestLease) -> Result<(), LeaseError> {
        let conn = self.get_connection().map_err(LeaseError::Storage)?;
        let affected = conn
            .execute(
                "UPDATE bulk_manifests SET status = 'processing'
                 WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3
                   AND manifest_id = ?4 AND worker_id = ?5 AND fencing_token = ?6",
                params![
                    lease.tenant.tenant_id().as_str(),
                    lease.submission_id.submitter,
                    lease.submission_id.submission_id,
                    lease.manifest_id,
                    lease.worker_id.as_str(),
                    lease.fencing_token as i64
                ],
            )
            .map_err(|e| LeaseError::Storage(internal_error(format!("mark processing: {e}"))))?;
        if affected == 0 {
            Err(lease_lost(lease))
        } else {
            Ok(())
        }
    }

    async fn update_manifest_progress(
        &self,
        lease: &ManifestLease,
        processed_entries: u64,
        failed_entries: u64,
        last_processed_line: u64,
    ) -> Result<(), LeaseError> {
        let conn = self.get_connection().map_err(LeaseError::Storage)?;
        let affected = conn
            .execute(
                "UPDATE bulk_manifests
                 SET processed_entries = ?1, failed_entries = ?2, last_processed_line = ?3
                 WHERE tenant_id = ?4 AND submitter = ?5 AND submission_id = ?6
                   AND manifest_id = ?7 AND worker_id = ?8 AND fencing_token = ?9",
                params![
                    processed_entries as i64,
                    failed_entries as i64,
                    last_processed_line as i64,
                    lease.tenant.tenant_id().as_str(),
                    lease.submission_id.submitter,
                    lease.submission_id.submission_id,
                    lease.manifest_id,
                    lease.worker_id.as_str(),
                    lease.fencing_token as i64
                ],
            )
            .map_err(|e| LeaseError::Storage(internal_error(format!("update progress: {e}"))))?;
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
        let conn = self.get_connection().map_err(LeaseError::Storage)?;
        // Fence: only record if we still hold the lease.
        let holds: bool = conn
            .query_row(
                "SELECT 1 FROM bulk_manifests
                 WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3
                   AND manifest_id = ?4 AND worker_id = ?5 AND fencing_token = ?6",
                params![
                    lease.tenant.tenant_id().as_str(),
                    lease.submission_id.submitter,
                    lease.submission_id.submission_id,
                    lease.manifest_id,
                    lease.worker_id.as_str(),
                    lease.fencing_token as i64
                ],
                |_| Ok(true),
            )
            .unwrap_or(false);
        if !holds {
            return Err(lease_lost(lease));
        }

        let count_severity = file
            .count_severity
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok());
        conn.execute(
            "INSERT INTO bulk_submit_files
             (tenant_id, submitter, submission_id, manifest_url, file_type, resource_type,
              part_index, fencing_token, file_path, line_count, byte_count, count_severity,
              created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                lease.tenant.tenant_id().as_str(),
                lease.submission_id.submitter,
                lease.submission_id.submission_id,
                file.manifest_url,
                file.file_type,
                file.resource_type,
                file.part_index as i64,
                lease.fencing_token as i64,
                file.file_path,
                file.line_count as i64,
                file.byte_count as i64,
                count_severity,
                Utc::now().to_rfc3339()
            ],
        )
        .map_err(|e| LeaseError::Storage(internal_error(format!("record submit file: {e}"))))?;
        Ok(())
    }

    async fn finish_manifest(&self, lease: &ManifestLease) -> Result<(), LeaseError> {
        let conn = self.get_connection().map_err(LeaseError::Storage)?;
        let affected = conn
            .execute(
                "UPDATE bulk_manifests SET status = 'completed', worker_id = NULL, lease_expiry = NULL
                 WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3
                   AND manifest_id = ?4 AND worker_id = ?5 AND fencing_token = ?6",
                params![
                    lease.tenant.tenant_id().as_str(),
                    lease.submission_id.submitter,
                    lease.submission_id.submission_id,
                    lease.manifest_id,
                    lease.worker_id.as_str(),
                    lease.fencing_token as i64
                ],
            )
            .map_err(|e| LeaseError::Storage(internal_error(format!("finish manifest: {e}"))))?;
        if affected == 0 {
            Err(lease_lost(lease))
        } else {
            Ok(())
        }
    }

    async fn fail_manifest(
        &self,
        lease: &ManifestLease,
        _error_message: &str,
    ) -> Result<(), LeaseError> {
        let conn = self.get_connection().map_err(LeaseError::Storage)?;
        let affected = conn
            .execute(
                "UPDATE bulk_manifests SET status = 'failed', worker_id = NULL, lease_expiry = NULL
                 WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3
                   AND manifest_id = ?4 AND worker_id = ?5 AND fencing_token = ?6",
                params![
                    lease.tenant.tenant_id().as_str(),
                    lease.submission_id.submitter,
                    lease.submission_id.submission_id,
                    lease.manifest_id,
                    lease.worker_id.as_str(),
                    lease.fencing_token as i64
                ],
            )
            .map_err(|e| LeaseError::Storage(internal_error(format!("fail manifest: {e}"))))?;
        if affected == 0 {
            Err(lease_lost(lease))
        } else {
            Ok(())
        }
    }

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
    ) -> StorageResult<()> {
        let conn = self.get_connection()?;
        let headers_json = serde_json::to_string(file_request_headers).ok();
        let oauth_json = serde_json::to_string(oauth_metadata_urls).ok();
        let encryption_json = file_encryption_key.and_then(|v| serde_json::to_string(v).ok());
        conn.execute(
            "UPDATE bulk_manifests
             SET fhir_base_url = ?1, output_format = ?2, file_request_headers = ?3,
                 oauth_metadata_urls = ?4, file_encryption_key = ?5
             WHERE tenant_id = ?6 AND submitter = ?7 AND submission_id = ?8 AND manifest_id = ?9",
            params![
                fhir_base_url,
                output_format,
                headers_json,
                oauth_json,
                encryption_json,
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
                "SELECT manifest_url, file_type, resource_type, part_index, fencing_token,
                        file_path, line_count, byte_count, count_severity
                 FROM bulk_submit_files
                 WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3
                 ORDER BY id",
            )
            .map_err(|e| internal_error(format!("prepare list files: {e}")))?;
        let rows = stmt
            .query_map(
                params![tenant.tenant_id().as_str(), id.submitter, id.submission_id],
                |r| {
                    let count_severity: Option<String> = r.get(8)?;
                    Ok(SubmitFileRow {
                        manifest_url: r.get(0)?,
                        file_type: r.get(1)?,
                        resource_type: r.get(2)?,
                        part_index: r.get::<_, i64>(3)? as u32,
                        fencing_token: r.get::<_, i64>(4)? as u64,
                        file_path: r.get(5)?,
                        line_count: r.get::<_, i64>(6)? as u64,
                        byte_count: r.get::<_, i64>(7)? as u64,
                        count_severity: count_severity
                            .as_deref()
                            .and_then(|s| serde_json::from_str(s).ok()),
                    })
                },
            )
            .map_err(|e| internal_error(format!("query list files: {e}")))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| internal_error(format!("row list files: {e}")))?);
        }
        Ok(out)
    }

    async fn delete_submission_artifacts(
        &self,
        tenant: &TenantContext,
        id: &SubmissionId,
    ) -> StorageResult<()> {
        let conn = self.get_connection()?;
        conn.execute(
            "DELETE FROM bulk_submit_files
             WHERE tenant_id = ?1 AND submitter = ?2 AND submission_id = ?3",
            params![tenant.tenant_id().as_str(), id.submitter, id.submission_id],
        )
        .map_err(|e| internal_error(format!("delete artifacts: {e}")))?;
        Ok(())
    }

    async fn count_active_submissions(&self, tenant: &TenantContext) -> StorageResult<u64> {
        let conn = self.get_connection()?;
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM bulk_submissions
                 WHERE tenant_id = ?1 AND status = 'in-progress'",
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tenant::{TenantId, TenantPermissions};
    use serde_json::json;

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
            .update_manifest_progress(&lease, 5, 1, 6)
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

        backend
            .record_submit_file(
                &lease,
                &SubmitFileRecord {
                    manifest_url: Some("http://example.com/manifest.json".to_string()),
                    file_type: "error".to_string(),
                    resource_type: None,
                    part_index: 0,
                    file_path: "tenant/sub/error-0.ndjson".to_string(),
                    line_count: 3,
                    byte_count: 120,
                    count_severity: Some(json!({"error": 3})),
                },
            )
            .await
            .unwrap();

        let files = backend
            .list_submit_files(&tenant, &lease.submission_id)
            .await
            .unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].file_type, "error");
        assert_eq!(files[0].line_count, 3);

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
    }
}
