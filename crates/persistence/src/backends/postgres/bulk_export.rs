//! Bulk export implementation for PostgreSQL backend.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::time::Duration as StdDuration;

use crate::core::bulk_export::{
    BulkExportStorage, ExpiredExportRef, ExportDataProvider, ExportFileMetadata, ExportJobId,
    ExportJobMetadata, ExportLevel, ExportProgress, ExportRequest, ExportStatus,
    GroupExportProvider, NdjsonBatch, PatientExportProvider, RawExportManifest, RawManifestEntry,
    StartExportInput, TypeExportProgress,
};
use crate::core::bulk_export_output::{ExportPartKey, FinalizedPart};
use crate::core::bulk_export_worker::{
    ExportClaimStrategy, ExportJobLease, ExportWorkerStorage, LeaseError, WorkerId, WorkerJobView,
};
use crate::error::{BackendError, BulkExportError, StorageError, StorageResult};
use crate::tenant::{TenantContext, TenantId, TenantPermissions};

use super::PostgresBackend;

fn internal_error(message: String) -> StorageError {
    StorageError::Backend(BackendError::Internal {
        backend_name: "postgres".to_string(),
        message,
        source: None,
    })
}

/// Splits a `{resource_type}-{part_index}` download segment.
fn parse_part_segment(part: &str) -> Option<(String, u32)> {
    let idx = part.rfind('-')?;
    let resource_type = &part[..idx];
    let part_index: u32 = part[idx + 1..].parse().ok()?;
    if resource_type.is_empty() {
        return None;
    }
    Some((resource_type.to_string(), part_index))
}

/// Encodes an [`ExportPartKey`] into the `file_path` column.
fn encode_part_path(key: &ExportPartKey) -> String {
    format!(
        "{}/{}/{}/{}-{}-{}",
        key.tenant_id,
        key.job_id,
        key.file_type,
        key.resource_type,
        key.part_index,
        key.fencing_token
    )
}

#[async_trait]
impl BulkExportStorage for PostgresBackend {
    async fn start_export(
        &self,
        tenant: &TenantContext,
        input: StartExportInput,
    ) -> StorageResult<ExportJobId> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        let job_id = ExportJobId::new();
        let now = Utc::now();

        let level_str = match &input.request.level {
            ExportLevel::System => "system".to_string(),
            ExportLevel::Patient => "patient".to_string(),
            ExportLevel::Group { .. } => "group".to_string(),
        };

        let group_id = input.request.group_id().map(|s| s.to_string());

        let request_json = serde_json::to_string(&input.request)
            .map_err(|e| internal_error(format!("Failed to serialize request: {}", e)))?;
        let fhir_version = input.fhir_version.as_mime_param();

        client
            .execute(
                "INSERT INTO bulk_export_jobs
                 (id, tenant_id, status, level, group_id, request_json, transaction_time,
                  created_at, owner_subject, request_url, fhir_version, fencing_token)
                 VALUES ($1, $2, 'accepted', $3, $4, $5, $6, $7, $8, $9, $10, 0)",
                &[
                    &job_id.as_str(),
                    &tenant_id,
                    &level_str.as_str(),
                    &group_id,
                    &request_json.as_str(),
                    &input.transaction_time,
                    &now,
                    &input.owner_subject,
                    &input.request_url.as_str(),
                    &fhir_version,
                ],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to create export job: {}", e)))?;

        Ok(job_id)
    }

    async fn get_export_status(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
    ) -> StorageResult<ExportProgress> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        let rows = client
            .query(
                "SELECT status, level, group_id, transaction_time, started_at, completed_at, error_message, current_type
                 FROM bulk_export_jobs
                 WHERE id = $1 AND tenant_id = $2",
                &[&job_id.as_str(), &tenant_id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to get export status: {}", e)))?;

        if rows.is_empty() {
            return Err(StorageError::BulkExport(BulkExportError::JobNotFound {
                job_id: job_id.to_string(),
            }));
        }

        let row = &rows[0];
        let status_str: String = row.get(0);
        let level_str: String = row.get(1);
        let group_id: Option<String> = row.get(2);
        let transaction_time: chrono::DateTime<Utc> = row.get(3);
        let started_at: Option<chrono::DateTime<Utc>> = row.get(4);
        let completed_at: Option<chrono::DateTime<Utc>> = row.get(5);
        let error_message: Option<String> = row.get(6);
        let current_type: Option<String> = row.get(7);

        let status: ExportStatus = status_str
            .parse()
            .map_err(|_| internal_error(format!("Invalid status in database: {}", status_str)))?;

        let level = match level_str.as_str() {
            "system" => ExportLevel::System,
            "patient" => ExportLevel::Patient,
            "group" => ExportLevel::Group {
                group_id: group_id.unwrap_or_default(),
            },
            _ => {
                return Err(internal_error(format!(
                    "Invalid level in database: {}",
                    level_str
                )));
            }
        };

        // Get per-type progress
        let progress_rows = client
            .query(
                "SELECT resource_type, total_count, exported_count, error_count, cursor_state
                 FROM bulk_export_progress
                 WHERE job_id = $1",
                &[&job_id.as_str()],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to query progress: {}", e)))?;

        let type_progress: Vec<TypeExportProgress> = progress_rows
            .iter()
            .map(|r| TypeExportProgress {
                resource_type: r.get(0),
                total_count: r.get::<_, Option<i32>>(1).map(|v| v as u64),
                exported_count: r.get::<_, i32>(2) as u64,
                error_count: r.get::<_, i32>(3) as u64,
                cursor_state: r.get(4),
            })
            .collect();

        Ok(ExportProgress {
            job_id: job_id.clone(),
            status,
            level,
            transaction_time,
            started_at,
            completed_at,
            type_progress,
            current_type,
            error_message,
        })
    }

    async fn cancel_export(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
    ) -> StorageResult<()> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        let rows = client
            .query(
                "SELECT status FROM bulk_export_jobs WHERE id = $1 AND tenant_id = $2",
                &[&job_id.as_str(), &tenant_id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to get export status: {}", e)))?;

        if rows.is_empty() {
            return Err(StorageError::BulkExport(BulkExportError::JobNotFound {
                job_id: job_id.to_string(),
            }));
        }

        let current_status: String = rows[0].get(0);
        let status: ExportStatus = current_status.parse().map_err(|_| {
            internal_error(format!("Invalid status in database: {}", current_status))
        })?;

        if status.is_terminal() {
            return Err(StorageError::BulkExport(BulkExportError::InvalidJobState {
                job_id: job_id.to_string(),
                expected: "accepted or in-progress".to_string(),
                actual: current_status,
            }));
        }

        let now = Utc::now();
        client
            .execute(
                "UPDATE bulk_export_jobs SET status = 'cancelled', completed_at = $1 WHERE id = $2",
                &[&now, &job_id.as_str()],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to cancel export: {}", e)))?;

        Ok(())
    }

    async fn delete_export(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
    ) -> StorageResult<()> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        let result = client
            .execute(
                "DELETE FROM bulk_export_jobs WHERE id = $1 AND tenant_id = $2",
                &[&job_id.as_str(), &tenant_id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to delete export: {}", e)))?;

        if result == 0 {
            return Err(StorageError::BulkExport(BulkExportError::JobNotFound {
                job_id: job_id.to_string(),
            }));
        }

        Ok(())
    }

    async fn get_export_manifest(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
    ) -> StorageResult<RawExportManifest> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        let job_rows = client
            .query(
                "SELECT status, transaction_time, request_url, error_message, completed_at
                 FROM bulk_export_jobs WHERE id = $1 AND tenant_id = $2",
                &[&job_id.as_str(), &tenant_id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to get export job: {}", e)))?;
        let job_row = job_rows.first().ok_or_else(|| {
            StorageError::BulkExport(BulkExportError::JobNotFound {
                job_id: job_id.to_string(),
            })
        })?;
        let status_str: String = job_row.get(0);
        let transaction_time: DateTime<Utc> = job_row.get(1);
        let request_url: String = job_row.get(2);
        let error_message: Option<String> = job_row.get(3);
        let completed_at: Option<DateTime<Utc>> = job_row.get(4);
        let status: ExportStatus = status_str
            .parse()
            .map_err(|_| internal_error(format!("Invalid status in database: {}", status_str)))?;

        let rows = client
            .query(
                "SELECT resource_type, resource_count, file_type, part_index, fencing_token
                 FROM bulk_export_files
                 WHERE job_id = $1
                 ORDER BY file_type, resource_type, part_index",
                &[&job_id.as_str()],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to query files: {}", e)))?;

        let mut output = Vec::new();
        let mut errors = Vec::new();
        for row in &rows {
            let resource_type: String = row.get(0);
            let count: Option<i32> = row.get(1);
            let file_type: String = row.get(2);
            let part_index: i32 = row.get(3);
            let fencing_token: i64 = row.get(4);
            let key = ExportPartKey {
                tenant_id: tenant_id.to_string(),
                job_id: job_id.clone(),
                resource_type: resource_type.clone(),
                file_type: file_type.clone(),
                part_index: part_index as u32,
                fencing_token: fencing_token as u64,
            };
            let entry = RawManifestEntry {
                resource_type,
                key,
                count: count.unwrap_or(0) as u64,
            };
            if file_type == "error" {
                errors.push(entry);
            } else {
                output.push(entry);
            }
        }

        Ok(RawExportManifest {
            transaction_time,
            request_url,
            status,
            error_message,
            completed_at,
            output,
            errors,
        })
    }

    async fn get_export_job_metadata(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
    ) -> StorageResult<ExportJobMetadata> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();
        let rows = client
            .query(
                "SELECT status, level, group_id, owner_subject, transaction_time,
                        completed_at, request_url
                 FROM bulk_export_jobs WHERE id = $1 AND tenant_id = $2",
                &[&job_id.as_str(), &tenant_id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to get export job metadata: {}", e)))?;
        let row = rows.first().ok_or_else(|| {
            StorageError::BulkExport(BulkExportError::JobNotFound {
                job_id: job_id.to_string(),
            })
        })?;
        let status_str: String = row.get(0);
        let level_str: String = row.get(1);
        let group_id: Option<String> = row.get(2);
        let owner_subject: Option<String> = row.get(3);
        let transaction_time: DateTime<Utc> = row.get(4);
        let completed_at: Option<DateTime<Utc>> = row.get(5);
        let request_url: String = row.get(6);
        let status: ExportStatus = status_str
            .parse()
            .map_err(|_| internal_error(format!("Invalid status: {}", status_str)))?;
        let level = match level_str.as_str() {
            "system" => ExportLevel::System,
            "patient" => ExportLevel::Patient,
            "group" => ExportLevel::Group {
                group_id: group_id.unwrap_or_default(),
            },
            _ => return Err(internal_error(format!("Invalid level: {}", level_str))),
        };
        Ok(ExportJobMetadata {
            job_id: job_id.clone(),
            status,
            level,
            owner_subject,
            transaction_time,
            completed_at,
            request_url,
        })
    }

    async fn get_export_file_metadata(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
        part: &str,
    ) -> StorageResult<ExportFileMetadata> {
        let (resource_type, part_index) = parse_part_segment(part).ok_or_else(|| {
            StorageError::BulkExport(BulkExportError::JobNotFound {
                job_id: format!("{job_id}/{part}"),
            })
        })?;
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();
        let rows = client
            .query(
                "SELECT f.file_type, f.resource_count, f.fencing_token, j.owner_subject
                 FROM bulk_export_files f
                 JOIN bulk_export_jobs j ON j.id = f.job_id
                 WHERE f.job_id = $1 AND j.tenant_id = $2
                   AND f.resource_type = $3 AND f.part_index = $4",
                &[
                    &job_id.as_str(),
                    &tenant_id,
                    &resource_type.as_str(),
                    &(part_index as i32),
                ],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to get file metadata: {}", e)))?;
        let row = rows.first().ok_or_else(|| {
            StorageError::BulkExport(BulkExportError::JobNotFound {
                job_id: format!("{job_id}/{part}"),
            })
        })?;
        let file_type: String = row.get(0);
        let resource_count: Option<i32> = row.get(1);
        let fencing_token: i64 = row.get(2);
        let owner_subject: Option<String> = row.get(3);
        let key = ExportPartKey {
            tenant_id: tenant_id.to_string(),
            job_id: job_id.clone(),
            resource_type: resource_type.clone(),
            file_type: file_type.clone(),
            part_index,
            fencing_token: fencing_token as u64,
        };
        Ok(ExportFileMetadata {
            key,
            resource_type,
            file_type,
            line_count: resource_count.unwrap_or(0) as u64,
            job_owner_subject: owner_subject,
        })
    }

    async fn count_active_exports(&self, tenant: &TenantContext) -> StorageResult<u64> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();
        let row = client
            .query_one(
                "SELECT COUNT(*) FROM bulk_export_jobs
                 WHERE tenant_id = $1 AND status IN ('accepted', 'in-progress')",
                &[&tenant_id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to count active exports: {}", e)))?;
        let count: i64 = row.get(0);
        Ok(count as u64)
    }

    async fn list_expired_exports(
        &self,
        now: DateTime<Utc>,
        output_ttl: StdDuration,
        limit: u32,
    ) -> StorageResult<Vec<ExpiredExportRef>> {
        let client = self.get_client().await?;
        let cutoff = now
            - chrono::Duration::from_std(output_ttl)
                .unwrap_or_else(|_| chrono::Duration::seconds(0));
        let rows = client
            .query(
                "SELECT tenant_id, id FROM bulk_export_jobs
                 WHERE status IN ('complete', 'error', 'cancelled')
                   AND completed_at IS NOT NULL AND completed_at < $1
                 ORDER BY completed_at LIMIT $2",
                &[&cutoff, &(limit as i64)],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to query expired exports: {}", e)))?;
        Ok(rows
            .iter()
            .map(|row| {
                let tenant_id: String = row.get(0);
                let id: String = row.get(1);
                ExpiredExportRef {
                    tenant: TenantContext::new(
                        TenantId::new(tenant_id),
                        TenantPermissions::full_access(),
                    ),
                    job_id: ExportJobId::from_string(id),
                }
            })
            .collect())
    }

    async fn list_exports(
        &self,
        tenant: &TenantContext,
        include_completed: bool,
    ) -> StorageResult<Vec<ExportProgress>> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        let query = if include_completed {
            "SELECT id FROM bulk_export_jobs WHERE tenant_id = $1 ORDER BY created_at DESC"
        } else {
            "SELECT id FROM bulk_export_jobs WHERE tenant_id = $1 AND status IN ('accepted', 'in-progress') ORDER BY created_at DESC"
        };

        let rows = client
            .query(query, &[&tenant_id])
            .await
            .map_err(|e| internal_error(format!("Failed to query exports: {}", e)))?;

        let mut results = Vec::new();
        for row in &rows {
            let id: String = row.get(0);
            let job_id = ExportJobId::from_string(id);
            if let Ok(progress) = self.get_export_status(tenant, &job_id).await {
                results.push(progress);
            }
        }

        Ok(results)
    }
}

#[async_trait]
impl ExportClaimStrategy for PostgresBackend {
    async fn claim_next(
        &self,
        worker_id: &WorkerId,
        lease_duration: StdDuration,
    ) -> StorageResult<Option<ExportJobLease>> {
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
                "SELECT id, tenant_id, fencing_token FROM bulk_export_jobs
                 WHERE status = 'accepted'
                    OR (status = 'in-progress' AND (lease_expiry IS NULL OR lease_expiry < $1))
                 ORDER BY created_at
                 LIMIT 1
                 FOR UPDATE SKIP LOCKED",
                &[&now],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to select claimable job: {}", e)))?;

        let Some(row) = rows.first() else {
            txn.commit()
                .await
                .map_err(|e| internal_error(format!("Failed to commit claim txn: {}", e)))?;
            return Ok(None);
        };
        let job_id: String = row.get(0);
        let tenant_id: String = row.get(1);
        let fencing_token: i64 = row.get(2);
        let new_token = fencing_token + 1;

        txn.execute(
            "UPDATE bulk_export_jobs
             SET status = 'in-progress', worker_id = $1, lease_expiry = $2,
                 heartbeat_at = $3, fencing_token = $4,
                 started_at = COALESCE(started_at, $3)
             WHERE id = $5",
            &[
                &worker_id.as_str(),
                &lease_expiry,
                &now,
                &new_token,
                &job_id.as_str(),
            ],
        )
        .await
        .map_err(|e| internal_error(format!("Failed to claim export job: {}", e)))?;

        txn.commit()
            .await
            .map_err(|e| internal_error(format!("Failed to commit claim txn: {}", e)))?;

        Ok(Some(ExportJobLease {
            job_id: ExportJobId::from_string(job_id),
            tenant: TenantContext::new(TenantId::new(tenant_id), TenantPermissions::full_access()),
            worker_id: worker_id.clone(),
            lease_expiry,
            fencing_token: new_token as u64,
        }))
    }

    async fn heartbeat(&self, lease: &ExportJobLease) -> Result<DateTime<Utc>, LeaseError> {
        let client = self.get_client().await.map_err(LeaseError::Storage)?;
        let now = Utc::now();
        let new_expiry = now + chrono::Duration::seconds(60);
        let affected = client
            .execute(
                "UPDATE bulk_export_jobs
                 SET lease_expiry = $1, heartbeat_at = $2
                 WHERE id = $3 AND worker_id = $4 AND fencing_token = $5",
                &[
                    &new_expiry,
                    &now,
                    &lease.job_id.as_str(),
                    &lease.worker_id.as_str(),
                    &(lease.fencing_token as i64),
                ],
            )
            .await
            .map_err(|e| LeaseError::Storage(internal_error(format!("heartbeat failed: {e}"))))?;
        if affected == 0 {
            Err(LeaseError::LeaseLost {
                job_id: lease.job_id.clone(),
            })
        } else {
            Ok(new_expiry)
        }
    }

    async fn release(&self, lease: ExportJobLease) -> StorageResult<()> {
        let client = self.get_client().await?;
        client
            .execute(
                "UPDATE bulk_export_jobs
                 SET status = 'accepted', worker_id = NULL, lease_expiry = NULL
                 WHERE id = $1 AND worker_id = $2 AND fencing_token = $3
                   AND status = 'in-progress'",
                &[
                    &lease.job_id.as_str(),
                    &lease.worker_id.as_str(),
                    &(lease.fencing_token as i64),
                ],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to release lease: {}", e)))?;
        Ok(())
    }
}

#[async_trait]
impl ExportWorkerStorage for PostgresBackend {
    async fn get_export_job_for_worker(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
        worker_id: &WorkerId,
        fencing_token: u64,
    ) -> Result<WorkerJobView, LeaseError> {
        let client = self.get_client().await.map_err(LeaseError::Storage)?;
        let tenant_id = tenant.tenant_id().as_str();
        let rows = client
            .query(
                "SELECT request_json, level, group_id, transaction_time, fhir_version
                 FROM bulk_export_jobs
                 WHERE id = $1 AND tenant_id = $2 AND worker_id = $3 AND fencing_token = $4",
                &[
                    &job_id.as_str(),
                    &tenant_id,
                    &worker_id.as_str(),
                    &(fencing_token as i64),
                ],
            )
            .await
            .map_err(|e| LeaseError::Storage(internal_error(format!("load worker job: {e}"))))?;
        let row = rows.first().ok_or_else(|| LeaseError::LeaseLost {
            job_id: job_id.clone(),
        })?;
        let request_json: String = row.get(0);
        let level_str: String = row.get(1);
        let group_id: Option<String> = row.get(2);
        let transaction_time: DateTime<Utc> = row.get(3);
        let fhir_version_str: String = row.get(4);

        let request: ExportRequest = serde_json::from_str(&request_json)
            .map_err(|e| LeaseError::Storage(internal_error(format!("parse request_json: {e}"))))?;
        let level = match level_str.as_str() {
            "system" => ExportLevel::System,
            "patient" => ExportLevel::Patient,
            "group" => ExportLevel::Group {
                group_id: group_id.unwrap_or_default(),
            },
            _ => {
                return Err(LeaseError::Storage(internal_error(format!(
                    "Invalid level: {level_str}"
                ))));
            }
        };
        let fhir_version = helios_fhir::FhirVersion::from_mime_param(&fhir_version_str)
            .unwrap_or_else(helios_fhir::FhirVersion::default_enabled);

        let progress_rows = client
            .query(
                "SELECT resource_type, total_count, exported_count, error_count, cursor_state
                 FROM bulk_export_progress WHERE job_id = $1",
                &[&job_id.as_str()],
            )
            .await
            .map_err(|e| LeaseError::Storage(internal_error(format!("query progress: {e}"))))?;
        let type_progress: Vec<TypeExportProgress> = progress_rows
            .iter()
            .map(|r| TypeExportProgress {
                resource_type: r.get(0),
                total_count: r.get::<_, Option<i32>>(1).map(|v| v as u64),
                exported_count: r.get::<_, i32>(2) as u64,
                error_count: r.get::<_, i32>(3) as u64,
                cursor_state: r.get(4),
            })
            .collect();

        Ok(WorkerJobView {
            request,
            level,
            transaction_time,
            fhir_version,
            type_progress,
        })
    }

    async fn mark_export_in_progress(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
        worker_id: &WorkerId,
        fencing_token: u64,
    ) -> Result<(), LeaseError> {
        let client = self.get_client().await.map_err(LeaseError::Storage)?;
        let now = Utc::now();
        let affected = client
            .execute(
                "UPDATE bulk_export_jobs
                 SET status = 'in-progress', started_at = COALESCE(started_at, $1)
                 WHERE id = $2 AND tenant_id = $3 AND worker_id = $4 AND fencing_token = $5",
                &[
                    &now,
                    &job_id.as_str(),
                    &tenant.tenant_id().as_str(),
                    &worker_id.as_str(),
                    &(fencing_token as i64),
                ],
            )
            .await
            .map_err(|e| LeaseError::Storage(internal_error(format!("mark_in_progress: {e}"))))?;
        if affected == 0 {
            Err(LeaseError::LeaseLost {
                job_id: job_id.clone(),
            })
        } else {
            Ok(())
        }
    }

    async fn update_export_type_progress(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
        worker_id: &WorkerId,
        fencing_token: u64,
        progress: &TypeExportProgress,
    ) -> Result<(), LeaseError> {
        let client = self.get_client().await.map_err(LeaseError::Storage)?;
        let affected = client
            .execute(
                "INSERT INTO bulk_export_progress
                   (job_id, resource_type, total_count, exported_count, error_count, cursor_state)
                 SELECT $1, $2, $3, $4, $5, $6
                 WHERE EXISTS (
                     SELECT 1 FROM bulk_export_jobs
                     WHERE id = $1 AND tenant_id = $7 AND worker_id = $8 AND fencing_token = $9
                 )
                 ON CONFLICT (job_id, resource_type) DO UPDATE SET
                   total_count = EXCLUDED.total_count,
                   exported_count = EXCLUDED.exported_count,
                   error_count = EXCLUDED.error_count,
                   cursor_state = EXCLUDED.cursor_state",
                &[
                    &job_id.as_str(),
                    &progress.resource_type.as_str(),
                    &progress.total_count.map(|v| v as i32),
                    &(progress.exported_count as i32),
                    &(progress.error_count as i32),
                    &progress.cursor_state,
                    &tenant.tenant_id().as_str(),
                    &worker_id.as_str(),
                    &(fencing_token as i64),
                ],
            )
            .await
            .map_err(|e| {
                LeaseError::Storage(internal_error(format!("update_type_progress: {e}")))
            })?;
        if affected == 0 {
            Err(LeaseError::LeaseLost {
                job_id: job_id.clone(),
            })
        } else {
            Ok(())
        }
    }

    async fn record_export_file(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
        worker_id: &WorkerId,
        fencing_token: u64,
        part: &FinalizedPart,
        file_type: &str,
    ) -> Result<(), LeaseError> {
        let client = self.get_client().await.map_err(LeaseError::Storage)?;
        let file_path = encode_part_path(&part.key);
        let affected = client
            .execute(
                "INSERT INTO bulk_export_files
                   (job_id, resource_type, file_type, file_path, resource_count, byte_count,
                    part_index, fencing_token)
                 SELECT $1, $2, $3, $4, $5, $6, $7, $8
                 WHERE EXISTS (
                     SELECT 1 FROM bulk_export_jobs
                     WHERE id = $1 AND tenant_id = $9 AND worker_id = $10 AND fencing_token = $11
                 )
                 ON CONFLICT (job_id, file_type, resource_type, part_index) DO UPDATE SET
                   file_path = EXCLUDED.file_path,
                   resource_count = EXCLUDED.resource_count,
                   byte_count = EXCLUDED.byte_count,
                   fencing_token = EXCLUDED.fencing_token",
                &[
                    &job_id.as_str(),
                    &part.resource_type.as_str(),
                    &file_type,
                    &file_path.as_str(),
                    &(part.line_count as i32),
                    &(part.size_bytes as i64),
                    &(part.key.part_index as i32),
                    &(part.key.fencing_token as i64),
                    &tenant.tenant_id().as_str(),
                    &worker_id.as_str(),
                    &(fencing_token as i64),
                ],
            )
            .await
            .map_err(|e| LeaseError::Storage(internal_error(format!("record_export_file: {e}"))))?;
        if affected == 0 {
            Err(LeaseError::LeaseLost {
                job_id: job_id.clone(),
            })
        } else {
            Ok(())
        }
    }

    async fn finish_export_job(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
        worker_id: &WorkerId,
        fencing_token: u64,
    ) -> Result<(), LeaseError> {
        let client = self.get_client().await.map_err(LeaseError::Storage)?;
        let now = Utc::now();
        let affected = client
            .execute(
                "UPDATE bulk_export_jobs
                 SET status = 'complete', completed_at = $1
                 WHERE id = $2 AND tenant_id = $3 AND worker_id = $4 AND fencing_token = $5",
                &[
                    &now,
                    &job_id.as_str(),
                    &tenant.tenant_id().as_str(),
                    &worker_id.as_str(),
                    &(fencing_token as i64),
                ],
            )
            .await
            .map_err(|e| LeaseError::Storage(internal_error(format!("finish_job: {e}"))))?;
        if affected == 0 {
            Err(LeaseError::LeaseLost {
                job_id: job_id.clone(),
            })
        } else {
            Ok(())
        }
    }

    async fn fail_export_job(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
        worker_id: &WorkerId,
        fencing_token: u64,
        error_message: &str,
    ) -> Result<(), LeaseError> {
        let client = self.get_client().await.map_err(LeaseError::Storage)?;
        let now = Utc::now();
        let affected = client
            .execute(
                "UPDATE bulk_export_jobs
                 SET status = 'error', error_message = $1, completed_at = $2
                 WHERE id = $3 AND tenant_id = $4 AND worker_id = $5 AND fencing_token = $6",
                &[
                    &error_message,
                    &now,
                    &job_id.as_str(),
                    &tenant.tenant_id().as_str(),
                    &worker_id.as_str(),
                    &(fencing_token as i64),
                ],
            )
            .await
            .map_err(|e| LeaseError::Storage(internal_error(format!("fail_job: {e}"))))?;
        if affected == 0 {
            Err(LeaseError::LeaseLost {
                job_id: job_id.clone(),
            })
        } else {
            Ok(())
        }
    }
}

#[async_trait]
impl ExportDataProvider for PostgresBackend {
    async fn list_export_types(
        &self,
        tenant: &TenantContext,
        request: &ExportRequest,
    ) -> StorageResult<Vec<String>> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        if !request.resource_types.is_empty() {
            let mut valid_types = Vec::new();
            for rt in &request.resource_types {
                let row = client
                    .query_one(
                        "SELECT EXISTS(SELECT 1 FROM resources WHERE tenant_id = $1 AND resource_type = $2 AND is_deleted = FALSE LIMIT 1)",
                        &[&tenant_id, &rt.as_str()],
                    )
                    .await
                    .map_err(|e| internal_error(format!("Failed to check type: {}", e)))?;

                let exists: bool = row.get(0);
                if exists {
                    valid_types.push(rt.clone());
                }
            }
            return Ok(valid_types);
        }

        let rows = client
            .query(
                "SELECT DISTINCT resource_type FROM resources
                 WHERE tenant_id = $1 AND is_deleted = FALSE
                 ORDER BY resource_type",
                &[&tenant_id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to query types: {}", e)))?;

        let types: Vec<String> = rows.iter().map(|r| r.get(0)).collect();
        Ok(types)
    }

    async fn count_export_resources(
        &self,
        tenant: &TenantContext,
        request: &ExportRequest,
        resource_type: &str,
    ) -> StorageResult<u64> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        let (sql, params): (
            String,
            Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>>,
        ) = if let Some(since) = request.since {
            (
                "SELECT COUNT(*) FROM resources WHERE tenant_id = $1 AND resource_type = $2 AND is_deleted = FALSE AND last_updated >= $3".to_string(),
                vec![
                    Box::new(tenant_id.to_string()),
                    Box::new(resource_type.to_string()),
                    Box::new(since),
                ],
            )
        } else {
            (
                "SELECT COUNT(*) FROM resources WHERE tenant_id = $1 AND resource_type = $2 AND is_deleted = FALSE".to_string(),
                vec![
                    Box::new(tenant_id.to_string()),
                    Box::new(resource_type.to_string()),
                ],
            )
        };

        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = params
            .iter()
            .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        let row = client
            .query_one(&sql, &param_refs)
            .await
            .map_err(|e| internal_error(format!("Failed to count resources: {}", e)))?;

        let count: i64 = row.get(0);
        Ok(count as u64)
    }

    async fn fetch_export_batch(
        &self,
        tenant: &TenantContext,
        request: &ExportRequest,
        resource_type: &str,
        cursor: Option<&str>,
        batch_size: u32,
    ) -> StorageResult<NdjsonBatch> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        let mut sql = "SELECT id, data, last_updated FROM resources WHERE tenant_id = $1 AND resource_type = $2 AND is_deleted = FALSE".to_string();
        let mut params: Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>> = vec![
            Box::new(tenant_id.to_string()),
            Box::new(resource_type.to_string()),
        ];
        let mut param_idx = 3;

        if let Some(since) = request.since {
            sql.push_str(&format!(" AND last_updated >= ${}", param_idx));
            params.push(Box::new(since));
            param_idx += 1;
        }

        if let Some(cursor) = cursor {
            let parts: Vec<&str> = cursor.splitn(2, '|').collect();
            if parts.len() == 2 {
                if let Ok(dt) = DateTime::parse_from_rfc3339(parts[0]) {
                    sql.push_str(&format!(
                        " AND (last_updated, id) > (${}, ${})",
                        param_idx,
                        param_idx + 1
                    ));
                    params.push(Box::new(dt.with_timezone(&Utc)));
                    params.push(Box::new(parts[1].to_string()));
                }
            }
        }

        sql.push_str(&format!(
            " ORDER BY last_updated, id LIMIT {}",
            batch_size + 1
        ));

        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = params
            .iter()
            .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        let rows = client
            .query(&sql, &param_refs)
            .await
            .map_err(|e| internal_error(format!("Failed to query batch: {}", e)))?;

        let has_more = rows.len() > batch_size as usize;
        let rows_to_process = if has_more {
            &rows[..batch_size as usize]
        } else {
            &rows[..]
        };

        let mut lines = Vec::new();
        let mut last_cursor = None;

        for row in rows_to_process {
            let id: String = row.get(0);
            let resource: Value = row.get(1);
            let last_updated: chrono::DateTime<Utc> = row.get(2);

            let line = serde_json::to_string(&resource)
                .map_err(|e| internal_error(format!("Failed to serialize resource: {}", e)))?;
            lines.push(line);
            last_cursor = Some(format!("{}|{}", last_updated.to_rfc3339(), id));
        }

        Ok(NdjsonBatch {
            lines,
            next_cursor: if has_more { last_cursor } else { None },
            is_last: !has_more,
        })
    }
}

#[async_trait]
impl PatientExportProvider for PostgresBackend {
    async fn list_patient_ids(
        &self,
        tenant: &TenantContext,
        request: &ExportRequest,
        cursor: Option<&str>,
        batch_size: u32,
    ) -> StorageResult<(Vec<String>, Option<String>)> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        let mut sql = "SELECT id FROM resources WHERE tenant_id = $1 AND resource_type = 'Patient' AND is_deleted = FALSE".to_string();
        let mut params: Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>> =
            vec![Box::new(tenant_id.to_string())];
        let mut param_idx = 2;

        if let Some(since) = request.since {
            sql.push_str(&format!(" AND last_updated >= ${}", param_idx));
            params.push(Box::new(since));
            param_idx += 1;
        }

        if let Some(cursor) = cursor {
            sql.push_str(&format!(" AND id > ${}", param_idx));
            params.push(Box::new(cursor.to_string()));
        }

        sql.push_str(&format!(" ORDER BY id LIMIT {}", batch_size + 1));

        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = params
            .iter()
            .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        let rows = client
            .query(&sql, &param_refs)
            .await
            .map_err(|e| internal_error(format!("Failed to query patient ids: {}", e)))?;

        let mut ids: Vec<String> = rows.iter().map(|r| r.get(0)).collect();

        let has_more = ids.len() > batch_size as usize;
        if has_more {
            ids.truncate(batch_size as usize);
        }

        let next_cursor = if has_more { ids.last().cloned() } else { None };

        Ok((ids, next_cursor))
    }

    async fn fetch_patient_compartment_batch(
        &self,
        tenant: &TenantContext,
        request: &ExportRequest,
        resource_type: &str,
        patient_ids: &[String],
        cursor: Option<&str>,
        batch_size: u32,
    ) -> StorageResult<NdjsonBatch> {
        if patient_ids.is_empty() {
            return Ok(NdjsonBatch::empty());
        }

        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        if resource_type == "Patient" {
            // For Patient resources, just filter by the IDs using ANY($3::text[])
            let mut sql = "SELECT id, data, last_updated FROM resources
                 WHERE tenant_id = $1 AND resource_type = $2 AND id = ANY($3::text[]) AND is_deleted = FALSE".to_string();

            let mut params: Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>> = vec![
                Box::new(tenant_id.to_string()),
                Box::new(resource_type.to_string()),
                Box::new(patient_ids.to_vec()),
            ];
            let param_idx = 4;

            if let Some(cursor) = cursor {
                let parts: Vec<&str> = cursor.splitn(2, '|').collect();
                if parts.len() == 2 {
                    if let Ok(dt) = DateTime::parse_from_rfc3339(parts[0]) {
                        sql.push_str(&format!(
                            " AND (last_updated, id) > (${}, ${})",
                            param_idx,
                            param_idx + 1
                        ));
                        params.push(Box::new(dt.with_timezone(&Utc)));
                        params.push(Box::new(parts[1].to_string()));
                    }
                }
            }

            sql.push_str(&format!(
                " ORDER BY last_updated, id LIMIT {}",
                batch_size + 1
            ));

            let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = params
                .iter()
                .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
                .collect();

            let rows = client
                .query(&sql, &param_refs)
                .await
                .map_err(|e| internal_error(format!("Failed to query compartment: {}", e)))?;

            let has_more = rows.len() > batch_size as usize;
            let rows_slice = if has_more {
                &rows[..batch_size as usize]
            } else {
                &rows[..]
            };

            let mut lines = Vec::new();
            let mut last_cursor = None;

            for row in rows_slice {
                let id: String = row.get(0);
                let resource: Value = row.get(1);
                let last_updated: chrono::DateTime<Utc> = row.get(2);

                let line = serde_json::to_string(&resource)
                    .map_err(|e| internal_error(format!("Failed to serialize: {}", e)))?;
                lines.push(line);
                last_cursor = Some(format!("{}|{}", last_updated.to_rfc3339(), id));
            }

            return Ok(NdjsonBatch {
                lines,
                next_cursor: if has_more { last_cursor } else { None },
                is_last: !has_more,
            });
        }

        // For other resource types, find resources whose JSONB payload
        // references one of the patients via `subject.reference` or
        // `patient.reference`. We read the payload directly rather than the
        // search_index, so this is correct even when search is offloaded to a
        // secondary backend (postgres-elasticsearch), which leaves the local
        // search_index empty.
        let patient_refs: Vec<String> = patient_ids
            .iter()
            .map(|id| format!("Patient/{}", id))
            .collect();

        let mut sql = "SELECT id, data, last_updated FROM resources
             WHERE tenant_id = $1
                AND resource_type = $2
                AND is_deleted = FALSE
                AND ((data #>> '{subject,reference}') = ANY($3::text[])
                  OR (data #>> '{patient,reference}') = ANY($3::text[]))"
            .to_string();

        let mut params: Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>> = vec![
            Box::new(tenant_id.to_string()),
            Box::new(resource_type.to_string()),
            Box::new(patient_refs),
        ];
        let mut param_idx = 4;

        if let Some(since) = request.since {
            sql.push_str(&format!(" AND last_updated >= ${}", param_idx));
            params.push(Box::new(since));
            param_idx += 1;
        }

        if let Some(cursor) = cursor {
            let parts: Vec<&str> = cursor.splitn(2, '|').collect();
            if parts.len() == 2 {
                if let Ok(dt) = DateTime::parse_from_rfc3339(parts[0]) {
                    sql.push_str(&format!(
                        " AND (last_updated, id) > (${}, ${})",
                        param_idx,
                        param_idx + 1
                    ));
                    params.push(Box::new(dt.with_timezone(&Utc)));
                    params.push(Box::new(parts[1].to_string()));
                }
            }
        }

        sql.push_str(&format!(
            " ORDER BY last_updated, id LIMIT {}",
            batch_size + 1
        ));

        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = params
            .iter()
            .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        let rows = client
            .query(&sql, &param_refs)
            .await
            .map_err(|e| internal_error(format!("Failed to query compartment: {}", e)))?;

        let has_more = rows.len() > batch_size as usize;
        let rows_slice = if has_more {
            &rows[..batch_size as usize]
        } else {
            &rows[..]
        };

        let mut lines = Vec::new();
        let mut last_cursor = None;

        for row in rows_slice {
            let id: String = row.get(0);
            let resource: Value = row.get(1);
            let last_updated: chrono::DateTime<Utc> = row.get(2);

            let line = serde_json::to_string(&resource)
                .map_err(|e| internal_error(format!("Failed to serialize: {}", e)))?;
            lines.push(line);
            last_cursor = Some(format!("{}|{}", last_updated.to_rfc3339(), id));
        }

        Ok(NdjsonBatch {
            lines,
            next_cursor: if has_more { last_cursor } else { None },
            is_last: !has_more,
        })
    }
}

#[async_trait]
impl GroupExportProvider for PostgresBackend {
    async fn get_group_members(
        &self,
        tenant: &TenantContext,
        group_id: &str,
    ) -> StorageResult<Vec<String>> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        let rows = client
            .query(
                "SELECT data FROM resources WHERE tenant_id = $1 AND resource_type = 'Group' AND id = $2 AND is_deleted = FALSE",
                &[&tenant_id, &group_id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to fetch group: {}", e)))?;

        if rows.is_empty() {
            return Ok(Vec::new());
        }

        let data: Value = rows[0].get(0);

        // Extract member references from the Group resource
        let mut member_refs = Vec::new();
        if let Some(members) = data.get("member").and_then(|m| m.as_array()) {
            for member in members {
                if let Some(reference) = member
                    .get("entity")
                    .and_then(|e| e.get("reference"))
                    .and_then(|r| r.as_str())
                {
                    member_refs.push(reference.to_string());
                }
            }
        }

        Ok(member_refs)
    }

    async fn resolve_group_patient_ids(
        &self,
        tenant: &TenantContext,
        group_id: &str,
    ) -> StorageResult<Vec<String>> {
        // Flatten nested Groups iteratively, guarding against membership
        // cycles with a visited set.
        use std::collections::HashSet;
        let mut visited_groups: HashSet<String> = HashSet::new();
        let mut seen_patients: HashSet<String> = HashSet::new();
        let mut patient_ids: Vec<String> = Vec::new();
        let mut worklist: Vec<String> = vec![group_id.to_string()];

        while let Some(gid) = worklist.pop() {
            if !visited_groups.insert(gid.clone()) {
                continue; // cycle / already processed
            }
            let members = self.get_group_members(tenant, &gid).await?;
            for member_ref in &members {
                if let Some(id) = member_ref.strip_prefix("Patient/") {
                    if seen_patients.insert(id.to_string()) {
                        patient_ids.push(id.to_string());
                    }
                } else if let Some(nested) = member_ref.strip_prefix("Group/") {
                    worklist.push(nested.to_string());
                }
            }
        }

        Ok(patient_ids)
    }

    async fn get_group_members_with_periods(
        &self,
        tenant: &TenantContext,
        group_id: &str,
    ) -> StorageResult<Vec<(String, Option<DateTime<Utc>>)>> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();
        let rows = client
            .query(
                "SELECT data FROM resources
                 WHERE tenant_id = $1 AND resource_type = 'Group'
                   AND id = $2 AND is_deleted = false",
                &[&tenant_id, &group_id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to get group: {}", e)))?;
        let row = rows.first().ok_or_else(|| {
            StorageError::BulkExport(BulkExportError::GroupNotFound {
                group_id: group_id.to_string(),
            })
        })?;
        let data: Vec<u8> = row.get(0);
        let group: Value = serde_json::from_slice(&data)
            .map_err(|e| internal_error(format!("Failed to parse group: {}", e)))?;
        let mut out = Vec::new();
        if let Some(arr) = group.get("member").and_then(|m| m.as_array()) {
            for member in arr {
                let Some(reference) = member
                    .get("entity")
                    .and_then(|e| e.get("reference"))
                    .and_then(|r| r.as_str())
                else {
                    continue;
                };
                let period_start = member
                    .get("period")
                    .and_then(|p| p.get("start"))
                    .and_then(|s| s.as_str())
                    .and_then(|s| {
                        DateTime::parse_from_rfc3339(s)
                            .ok()
                            .map(|dt| dt.with_timezone(&Utc))
                    });
                out.push((reference.to_string(), period_start));
            }
        }
        Ok(out)
    }
}
