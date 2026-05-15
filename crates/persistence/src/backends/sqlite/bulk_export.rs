//! Bulk export implementation for SQLite backend.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rusqlite::params;
use serde_json::Value;
use std::time::Duration as StdDuration;
use tokio::sync::Mutex;

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

use super::SqliteBackend;

/// Process-local lock serializing `claim_next` for the single-instance
/// SQLite job store (SQLite has no `SELECT … FOR UPDATE SKIP LOCKED`).
static CLAIM_LOCK: Mutex<()> = Mutex::const_new(());

/// Parses an RFC3339 timestamp column into a UTC `DateTime`.
fn parse_dt(s: &str) -> StorageResult<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| internal_error(format!("invalid timestamp '{s}': {e}")))
}

/// Parses an optional RFC3339 timestamp column.
fn parse_dt_opt(s: Option<String>) -> Option<DateTime<Utc>> {
    s.and_then(|s| {
        DateTime::parse_from_rfc3339(&s)
            .ok()
            .map(|dt| dt.with_timezone(&Utc))
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

fn internal_error(message: String) -> StorageError {
    StorageError::Backend(BackendError::Internal {
        backend_name: "sqlite".to_string(),
        message,
        source: None,
    })
}

#[async_trait]
impl BulkExportStorage for SqliteBackend {
    async fn start_export(
        &self,
        tenant: &TenantContext,
        input: StartExportInput,
    ) -> StorageResult<ExportJobId> {
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        let job_id = ExportJobId::new();
        let now = Utc::now().to_rfc3339();
        let transaction_time = input.transaction_time.to_rfc3339();

        let level_str = match &input.request.level {
            ExportLevel::System => "system".to_string(),
            ExportLevel::Patient => "patient".to_string(),
            ExportLevel::Group { .. } => "group".to_string(),
        };

        let group_id = input.request.group_id().map(|s| s.to_string());

        let request_json = serde_json::to_string(&input.request)
            .map_err(|e| internal_error(format!("Failed to serialize request: {}", e)))?;

        conn.execute(
            "INSERT INTO bulk_export_jobs
             (id, tenant_id, status, level, group_id, request_json, transaction_time,
              created_at, owner_subject, request_url, fhir_version, fencing_token)
             VALUES (?1, ?2, 'accepted', ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 0)",
            params![
                job_id.as_str(),
                tenant_id,
                level_str,
                group_id,
                request_json,
                transaction_time,
                now,
                input.owner_subject,
                input.request_url,
                input.fhir_version.as_mime_param(),
            ],
        )
        .map_err(|e| internal_error(format!("Failed to create export job: {}", e)))?;

        Ok(job_id)
    }

    async fn get_export_status(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
    ) -> StorageResult<ExportProgress> {
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        let (status_str, level_str, group_id, transaction_time, started_at, completed_at, error_message, current_type):
            (String, String, Option<String>, String, Option<String>, Option<String>, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT status, level, group_id, transaction_time, started_at, completed_at, error_message, current_type
                 FROM bulk_export_jobs
                 WHERE id = ?1 AND tenant_id = ?2",
                params![job_id.as_str(), tenant_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?,
                          row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?)),
            )
            .map_err(|e| {
                if matches!(e, rusqlite::Error::QueryReturnedNoRows) {
                    StorageError::BulkExport(BulkExportError::JobNotFound {
                        job_id: job_id.to_string(),
                    })
                } else {
                    internal_error(format!("Failed to get export status: {}", e))
                }
            })?;

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

        let transaction_time = chrono::DateTime::parse_from_rfc3339(&transaction_time)
            .map_err(|e| internal_error(format!("Invalid transaction_time: {}", e)))?
            .with_timezone(&Utc);

        let started_at = started_at.and_then(|s| {
            chrono::DateTime::parse_from_rfc3339(&s)
                .ok()
                .map(|dt| dt.with_timezone(&Utc))
        });

        let completed_at = completed_at.and_then(|s| {
            chrono::DateTime::parse_from_rfc3339(&s)
                .ok()
                .map(|dt| dt.with_timezone(&Utc))
        });

        // Get per-type progress
        let mut stmt = conn
            .prepare(
                "SELECT resource_type, total_count, exported_count, error_count, cursor_state
                 FROM bulk_export_progress
                 WHERE job_id = ?1",
            )
            .map_err(|e| internal_error(format!("Failed to prepare progress query: {}", e)))?;

        let type_progress: Vec<TypeExportProgress> = stmt
            .query_map(params![job_id.as_str()], |row| {
                Ok(TypeExportProgress {
                    resource_type: row.get(0)?,
                    total_count: row.get::<_, Option<i64>>(1)?.map(|v| v as u64),
                    exported_count: row.get::<_, i64>(2)? as u64,
                    error_count: row.get::<_, i64>(3)? as u64,
                    cursor_state: row.get(4)?,
                })
            })
            .map_err(|e| internal_error(format!("Failed to query progress: {}", e)))?
            .filter_map(|r| r.ok())
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
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        // Check current status
        let current_status: String = conn
            .query_row(
                "SELECT status FROM bulk_export_jobs WHERE id = ?1 AND tenant_id = ?2",
                params![job_id.as_str(), tenant_id],
                |row| row.get(0),
            )
            .map_err(|e| {
                if matches!(e, rusqlite::Error::QueryReturnedNoRows) {
                    StorageError::BulkExport(BulkExportError::JobNotFound {
                        job_id: job_id.to_string(),
                    })
                } else {
                    internal_error(format!("Failed to get export status: {}", e))
                }
            })?;

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

        let now = Utc::now().to_rfc3339();
        conn.execute(
            "UPDATE bulk_export_jobs SET status = 'cancelled', completed_at = ?1 WHERE id = ?2",
            params![now, job_id.as_str()],
        )
        .map_err(|e| internal_error(format!("Failed to cancel export: {}", e)))?;

        Ok(())
    }

    async fn delete_export(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
    ) -> StorageResult<()> {
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        // Check exists
        let exists: bool = conn
            .query_row(
                "SELECT 1 FROM bulk_export_jobs WHERE id = ?1 AND tenant_id = ?2",
                params![job_id.as_str(), tenant_id],
                |_| Ok(true),
            )
            .unwrap_or(false);

        if !exists {
            return Err(StorageError::BulkExport(BulkExportError::JobNotFound {
                job_id: job_id.to_string(),
            }));
        }

        // Delete job (cascades to progress and files due to foreign keys)
        conn.execute(
            "DELETE FROM bulk_export_jobs WHERE id = ?1 AND tenant_id = ?2",
            params![job_id.as_str(), tenant_id],
        )
        .map_err(|e| internal_error(format!("Failed to delete export: {}", e)))?;

        Ok(())
    }

    async fn get_export_manifest(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
    ) -> StorageResult<RawExportManifest> {
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        let (status_str, transaction_time, request_url, error_message, completed_at): (
            String,
            String,
            String,
            Option<String>,
            Option<String>,
        ) = conn
            .query_row(
                "SELECT status, transaction_time, request_url, error_message, completed_at
                 FROM bulk_export_jobs WHERE id = ?1 AND tenant_id = ?2",
                params![job_id.as_str(), tenant_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .map_err(|e| {
                if matches!(e, rusqlite::Error::QueryReturnedNoRows) {
                    StorageError::BulkExport(BulkExportError::JobNotFound {
                        job_id: job_id.to_string(),
                    })
                } else {
                    internal_error(format!("Failed to get export job: {}", e))
                }
            })?;

        let status: ExportStatus = status_str
            .parse()
            .map_err(|_| internal_error(format!("Invalid status in database: {}", status_str)))?;

        // Get output/error files.
        let mut stmt = conn
            .prepare(
                "SELECT resource_type, resource_count, file_type, part_index, fencing_token
                 FROM bulk_export_files
                 WHERE job_id = ?1
                 ORDER BY file_type, resource_type, part_index",
            )
            .map_err(|e| internal_error(format!("Failed to prepare files query: {}", e)))?;

        let rows: Vec<(String, i64, String, i64, i64)> = stmt
            .query_map(params![job_id.as_str()], |row| {
                Ok((
                    row.get(0)?,
                    row.get::<_, Option<i64>>(1)?.unwrap_or(0),
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })
            .map_err(|e| internal_error(format!("Failed to query files: {}", e)))?
            .filter_map(|r| r.ok())
            .collect();

        let mut output = Vec::new();
        let mut errors = Vec::new();
        for (resource_type, count, file_type, part_index, fencing_token) in rows {
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
                count: count as u64,
            };
            if file_type == "error" {
                errors.push(entry);
            } else {
                output.push(entry);
            }
        }

        Ok(RawExportManifest {
            transaction_time: parse_dt(&transaction_time)?,
            request_url,
            status,
            error_message,
            completed_at: parse_dt_opt(completed_at),
            output,
            errors,
        })
    }

    async fn list_exports(
        &self,
        tenant: &TenantContext,
        include_completed: bool,
    ) -> StorageResult<Vec<ExportProgress>> {
        // Collect IDs first, then drop the connection before calling async methods
        let job_ids: Vec<String> = {
            let conn = self.get_connection()?;
            let tenant_id = tenant.tenant_id().as_str();

            let query = if include_completed {
                "SELECT id FROM bulk_export_jobs WHERE tenant_id = ?1 ORDER BY created_at DESC"
            } else {
                "SELECT id FROM bulk_export_jobs WHERE tenant_id = ?1 AND status IN ('accepted', 'in-progress') ORDER BY created_at DESC"
            };

            let mut stmt = conn
                .prepare(query)
                .map_err(|e| internal_error(format!("Failed to prepare list query: {}", e)))?;

            stmt.query_map(params![tenant_id], |row| row.get(0))
                .map_err(|e| internal_error(format!("Failed to query exports: {}", e)))?
                .filter_map(|r| r.ok())
                .collect()
        };

        let mut results = Vec::new();
        for id in job_ids {
            let job_id = ExportJobId::from_string(id);
            if let Ok(progress) = self.get_export_status(tenant, &job_id).await {
                results.push(progress);
            }
        }

        Ok(results)
    }

    async fn get_export_job_metadata(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
    ) -> StorageResult<ExportJobMetadata> {
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        let (status_str, level_str, group_id, owner_subject, transaction_time, completed_at, request_url): (
            String,
            String,
            Option<String>,
            Option<String>,
            String,
            Option<String>,
            String,
        ) = conn
            .query_row(
                "SELECT status, level, group_id, owner_subject, transaction_time, completed_at, request_url
                 FROM bulk_export_jobs WHERE id = ?1 AND tenant_id = ?2",
                params![job_id.as_str(), tenant_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                    ))
                },
            )
            .map_err(|e| {
                if matches!(e, rusqlite::Error::QueryReturnedNoRows) {
                    StorageError::BulkExport(BulkExportError::JobNotFound {
                        job_id: job_id.to_string(),
                    })
                } else {
                    internal_error(format!("Failed to get export job metadata: {}", e))
                }
            })?;

        let status: ExportStatus = status_str
            .parse()
            .map_err(|_| internal_error(format!("Invalid status in database: {}", status_str)))?;
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
            transaction_time: parse_dt(&transaction_time)?,
            completed_at: parse_dt_opt(completed_at),
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

        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        let (file_type, resource_count, fencing_token, owner_subject): (
            String,
            i64,
            i64,
            Option<String>,
        ) = conn
            .query_row(
                "SELECT f.file_type, f.resource_count, f.fencing_token, j.owner_subject
                 FROM bulk_export_files f
                 JOIN bulk_export_jobs j ON j.id = f.job_id
                 WHERE f.job_id = ?1 AND j.tenant_id = ?2
                   AND f.resource_type = ?3 AND f.part_index = ?4",
                params![job_id.as_str(), tenant_id, resource_type, part_index as i64],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .map_err(|e| {
                if matches!(e, rusqlite::Error::QueryReturnedNoRows) {
                    StorageError::BulkExport(BulkExportError::JobNotFound {
                        job_id: format!("{job_id}/{part}"),
                    })
                } else {
                    internal_error(format!("Failed to get export file metadata: {}", e))
                }
            })?;

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
            line_count: resource_count as u64,
            job_owner_subject: owner_subject,
        })
    }

    async fn count_active_exports(&self, tenant: &TenantContext) -> StorageResult<u64> {
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM bulk_export_jobs
                 WHERE tenant_id = ?1 AND status IN ('accepted', 'in-progress')",
                params![tenant_id],
                |row| row.get(0),
            )
            .map_err(|e| internal_error(format!("Failed to count active exports: {}", e)))?;
        Ok(count as u64)
    }

    async fn list_expired_exports(
        &self,
        now: DateTime<Utc>,
        output_ttl: StdDuration,
        limit: u32,
    ) -> StorageResult<Vec<ExpiredExportRef>> {
        let conn = self.get_connection()?;
        let cutoff = (now
            - chrono::Duration::from_std(output_ttl)
                .unwrap_or_else(|_| chrono::Duration::seconds(0)))
        .to_rfc3339();

        let mut stmt = conn
            .prepare(
                "SELECT tenant_id, id FROM bulk_export_jobs
                 WHERE status IN ('complete', 'error', 'cancelled')
                   AND completed_at IS NOT NULL AND completed_at < ?1
                 ORDER BY completed_at LIMIT ?2",
            )
            .map_err(|e| internal_error(format!("Failed to prepare expired query: {}", e)))?;

        let rows: Vec<(String, String)> = stmt
            .query_map(params![cutoff, limit], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|e| internal_error(format!("Failed to query expired exports: {}", e)))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(rows
            .into_iter()
            .map(|(tenant_id, id)| ExpiredExportRef {
                tenant: TenantContext::new(
                    TenantId::new(tenant_id),
                    TenantPermissions::full_access(),
                ),
                job_id: ExportJobId::from_string(id),
            })
            .collect())
    }
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
impl ExportClaimStrategy for SqliteBackend {
    async fn claim_next(
        &self,
        worker_id: &WorkerId,
        lease_duration: StdDuration,
    ) -> StorageResult<Option<ExportJobLease>> {
        let _guard = CLAIM_LOCK.lock().await;
        let conn = self.get_connection()?;
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        let lease_expiry = now
            + chrono::Duration::from_std(lease_duration)
                .unwrap_or_else(|_| chrono::Duration::seconds(60));
        let lease_expiry_str = lease_expiry.to_rfc3339();

        // Find one eligible job: accepted, or in-progress with an expired lease.
        let row: Option<(String, String, i64)> = conn
            .query_row(
                "SELECT id, tenant_id, fencing_token FROM bulk_export_jobs
                 WHERE status = 'accepted'
                    OR (status = 'in-progress' AND (lease_expiry IS NULL OR lease_expiry < ?1))
                 ORDER BY created_at LIMIT 1",
                params![now_str],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .ok();

        let Some((job_id, tenant_id, fencing_token)) = row else {
            return Ok(None);
        };
        let new_token = fencing_token + 1;

        conn.execute(
            "UPDATE bulk_export_jobs
             SET status = 'in-progress', worker_id = ?1, lease_expiry = ?2,
                 heartbeat_at = ?3, fencing_token = ?4,
                 started_at = COALESCE(started_at, ?3)
             WHERE id = ?5",
            params![
                worker_id.as_str(),
                lease_expiry_str,
                now_str,
                new_token,
                job_id
            ],
        )
        .map_err(|e| internal_error(format!("Failed to claim export job: {}", e)))?;

        Ok(Some(ExportJobLease {
            job_id: ExportJobId::from_string(job_id),
            tenant: TenantContext::new(TenantId::new(tenant_id), TenantPermissions::full_access()),
            worker_id: worker_id.clone(),
            lease_expiry,
            fencing_token: new_token as u64,
        }))
    }

    async fn heartbeat(&self, lease: &ExportJobLease) -> Result<DateTime<Utc>, LeaseError> {
        let conn = self.get_connection().map_err(LeaseError::Storage)?;
        let now = Utc::now();
        let new_expiry = now + chrono::Duration::seconds(60);
        let affected = conn
            .execute(
                "UPDATE bulk_export_jobs
                 SET lease_expiry = ?1, heartbeat_at = ?2
                 WHERE id = ?3 AND worker_id = ?4 AND fencing_token = ?5",
                params![
                    new_expiry.to_rfc3339(),
                    now.to_rfc3339(),
                    lease.job_id.as_str(),
                    lease.worker_id.as_str(),
                    lease.fencing_token as i64
                ],
            )
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
        let conn = self.get_connection()?;
        conn.execute(
            "UPDATE bulk_export_jobs
             SET status = 'accepted', worker_id = NULL, lease_expiry = NULL
             WHERE id = ?1 AND worker_id = ?2 AND fencing_token = ?3
               AND status = 'in-progress'",
            params![
                lease.job_id.as_str(),
                lease.worker_id.as_str(),
                lease.fencing_token as i64
            ],
        )
        .map_err(|e| internal_error(format!("Failed to release lease: {}", e)))?;
        Ok(())
    }
}

#[async_trait]
impl ExportWorkerStorage for SqliteBackend {
    async fn get_export_job_for_worker(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
        worker_id: &WorkerId,
        fencing_token: u64,
    ) -> Result<WorkerJobView, LeaseError> {
        let conn = self.get_connection().map_err(LeaseError::Storage)?;
        let tenant_id = tenant.tenant_id().as_str();

        let (request_json, level_str, group_id, transaction_time, fhir_version): (
            String,
            String,
            Option<String>,
            String,
            String,
        ) = conn
            .query_row(
                "SELECT request_json, level, group_id, transaction_time, fhir_version
                 FROM bulk_export_jobs
                 WHERE id = ?1 AND tenant_id = ?2 AND worker_id = ?3 AND fencing_token = ?4",
                params![
                    job_id.as_str(),
                    tenant_id,
                    worker_id.as_str(),
                    fencing_token as i64
                ],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => LeaseError::LeaseLost {
                    job_id: job_id.clone(),
                },
                other => LeaseError::Storage(internal_error(format!(
                    "Failed to load worker job: {other}"
                ))),
            })?;

        let request: ExportRequest = serde_json::from_str(&request_json).map_err(|e| {
            LeaseError::Storage(internal_error(format!("Failed to parse request_json: {e}")))
        })?;
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
        let fhir_version =
            helios_fhir::FhirVersion::from_mime_param(&fhir_version).unwrap_or_default();
        let transaction_time = parse_dt(&transaction_time).map_err(LeaseError::Storage)?;

        // Load persisted per-type progress for resume.
        let mut stmt = conn
            .prepare(
                "SELECT resource_type, total_count, exported_count, error_count, cursor_state
                 FROM bulk_export_progress WHERE job_id = ?1",
            )
            .map_err(|e| LeaseError::Storage(internal_error(format!("prepare progress: {e}"))))?;
        let type_progress: Vec<TypeExportProgress> = stmt
            .query_map(params![job_id.as_str()], |row| {
                Ok(TypeExportProgress {
                    resource_type: row.get(0)?,
                    total_count: row.get::<_, Option<i64>>(1)?.map(|v| v as u64),
                    exported_count: row.get::<_, i64>(2)? as u64,
                    error_count: row.get::<_, i64>(3)? as u64,
                    cursor_state: row.get(4)?,
                })
            })
            .map_err(|e| LeaseError::Storage(internal_error(format!("query progress: {e}"))))?
            .filter_map(|r| r.ok())
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
        let conn = self.get_connection().map_err(LeaseError::Storage)?;
        let now = Utc::now().to_rfc3339();
        let affected = conn
            .execute(
                "UPDATE bulk_export_jobs
                 SET status = 'in-progress', started_at = COALESCE(started_at, ?1)
                 WHERE id = ?2 AND tenant_id = ?3 AND worker_id = ?4 AND fencing_token = ?5",
                params![
                    now,
                    job_id.as_str(),
                    tenant.tenant_id().as_str(),
                    worker_id.as_str(),
                    fencing_token as i64
                ],
            )
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
        let conn = self.get_connection().map_err(LeaseError::Storage)?;
        let affected = conn
            .execute(
                "INSERT INTO bulk_export_progress
                   (job_id, resource_type, total_count, exported_count, error_count, cursor_state)
                 SELECT ?1, ?2, ?3, ?4, ?5, ?6
                 WHERE EXISTS (
                     SELECT 1 FROM bulk_export_jobs
                     WHERE id = ?1 AND tenant_id = ?7 AND worker_id = ?8 AND fencing_token = ?9
                 )
                 ON CONFLICT(job_id, resource_type) DO UPDATE SET
                   total_count = excluded.total_count,
                   exported_count = excluded.exported_count,
                   error_count = excluded.error_count,
                   cursor_state = excluded.cursor_state",
                params![
                    job_id.as_str(),
                    progress.resource_type,
                    progress.total_count.map(|v| v as i64),
                    progress.exported_count as i64,
                    progress.error_count as i64,
                    progress.cursor_state,
                    tenant.tenant_id().as_str(),
                    worker_id.as_str(),
                    fencing_token as i64,
                ],
            )
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
        let conn = self.get_connection().map_err(LeaseError::Storage)?;
        let file_path = encode_part_path(&part.key);
        let affected = conn
            .execute(
                "INSERT INTO bulk_export_files
                   (job_id, resource_type, file_type, file_path, resource_count, byte_count,
                    part_index, fencing_token)
                 SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8
                 WHERE EXISTS (
                     SELECT 1 FROM bulk_export_jobs
                     WHERE id = ?1 AND tenant_id = ?9 AND worker_id = ?10 AND fencing_token = ?11
                 )
                 ON CONFLICT(job_id, file_type, resource_type, part_index) DO UPDATE SET
                   file_path = excluded.file_path,
                   resource_count = excluded.resource_count,
                   byte_count = excluded.byte_count,
                   fencing_token = excluded.fencing_token",
                params![
                    job_id.as_str(),
                    part.resource_type,
                    file_type,
                    file_path,
                    part.line_count as i64,
                    part.size_bytes as i64,
                    part.key.part_index as i64,
                    part.key.fencing_token as i64,
                    tenant.tenant_id().as_str(),
                    worker_id.as_str(),
                    fencing_token as i64,
                ],
            )
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
        let conn = self.get_connection().map_err(LeaseError::Storage)?;
        let now = Utc::now().to_rfc3339();
        let affected = conn
            .execute(
                "UPDATE bulk_export_jobs
                 SET status = 'complete', completed_at = ?1
                 WHERE id = ?2 AND tenant_id = ?3 AND worker_id = ?4 AND fencing_token = ?5",
                params![
                    now,
                    job_id.as_str(),
                    tenant.tenant_id().as_str(),
                    worker_id.as_str(),
                    fencing_token as i64
                ],
            )
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
        let conn = self.get_connection().map_err(LeaseError::Storage)?;
        let now = Utc::now().to_rfc3339();
        let affected = conn
            .execute(
                "UPDATE bulk_export_jobs
                 SET status = 'error', error_message = ?1, completed_at = ?2
                 WHERE id = ?3 AND tenant_id = ?4 AND worker_id = ?5 AND fencing_token = ?6",
                params![
                    error_message,
                    now,
                    job_id.as_str(),
                    tenant.tenant_id().as_str(),
                    worker_id.as_str(),
                    fencing_token as i64
                ],
            )
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
impl ExportDataProvider for SqliteBackend {
    async fn list_export_types(
        &self,
        tenant: &TenantContext,
        request: &ExportRequest,
    ) -> StorageResult<Vec<String>> {
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        // If specific types are requested, validate and return them
        if !request.resource_types.is_empty() {
            // Verify the types exist in the database
            let mut valid_types = Vec::new();
            for rt in &request.resource_types {
                let exists: bool = conn
                    .query_row(
                        "SELECT 1 FROM resources WHERE tenant_id = ?1 AND resource_type = ?2 AND is_deleted = 0 LIMIT 1",
                        params![tenant_id, rt],
                        |_| Ok(true),
                    )
                    .unwrap_or(false);
                if exists {
                    valid_types.push(rt.clone());
                }
            }
            return Ok(valid_types);
        }

        // Otherwise, get all types with data
        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT resource_type FROM resources
                 WHERE tenant_id = ?1 AND is_deleted = 0
                 ORDER BY resource_type",
            )
            .map_err(|e| internal_error(format!("Failed to prepare types query: {}", e)))?;

        let types: Vec<String> = stmt
            .query_map(params![tenant_id], |row| row.get(0))
            .map_err(|e| internal_error(format!("Failed to query types: {}", e)))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(types)
    }

    async fn count_export_resources(
        &self,
        tenant: &TenantContext,
        request: &ExportRequest,
        resource_type: &str,
    ) -> StorageResult<u64> {
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        let mut query = "SELECT COUNT(*) FROM resources WHERE tenant_id = ?1 AND resource_type = ?2 AND is_deleted = 0".to_string();
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![
            Box::new(tenant_id.to_string()),
            Box::new(resource_type.to_string()),
        ];

        // Apply _since filter if present
        if let Some(since) = request.since {
            query.push_str(" AND last_updated >= ?3");
            params_vec.push(Box::new(since.to_rfc3339()));
        }

        let params_slice: Vec<&dyn rusqlite::ToSql> =
            params_vec.iter().map(|p| p.as_ref()).collect();

        let count: i64 = conn
            .query_row(&query, params_slice.as_slice(), |row| row.get(0))
            .map_err(|e| internal_error(format!("Failed to count resources: {}", e)))?;

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
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        let mut query = "SELECT id, data, last_updated FROM resources WHERE tenant_id = ?1 AND resource_type = ?2 AND is_deleted = 0".to_string();
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![
            Box::new(tenant_id.to_string()),
            Box::new(resource_type.to_string()),
        ];

        // Apply _since filter if present
        if let Some(since) = request.since {
            query.push_str(" AND last_updated >= ?");
            params_vec.push(Box::new(since.to_rfc3339()));
        }

        // Apply cursor (keyset pagination)
        if let Some(cursor) = cursor {
            // Cursor format: "last_updated|id"
            let parts: Vec<&str> = cursor.splitn(2, '|').collect();
            if parts.len() == 2 {
                query.push_str(" AND (last_updated, id) > (?, ?)");
                params_vec.push(Box::new(parts[0].to_string()));
                params_vec.push(Box::new(parts[1].to_string()));
            }
        }

        query.push_str(" ORDER BY last_updated, id");
        query.push_str(&format!(" LIMIT {}", batch_size + 1)); // Fetch one extra to detect if there's more

        let params_slice: Vec<&dyn rusqlite::ToSql> =
            params_vec.iter().map(|p| p.as_ref()).collect();

        let mut stmt = conn
            .prepare(&query)
            .map_err(|e| internal_error(format!("Failed to prepare batch query: {}", e)))?;

        let rows: Vec<(String, Vec<u8>, String)> = stmt
            .query_map(params_slice.as_slice(), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| internal_error(format!("Failed to query batch: {}", e)))?
            .filter_map(|r| r.ok())
            .collect();

        let has_more = rows.len() > batch_size as usize;
        let rows = if has_more {
            &rows[..batch_size as usize]
        } else {
            &rows[..]
        };

        let mut lines = Vec::new();
        let mut last_cursor = None;

        for (id, data, last_updated) in rows {
            let resource: Value = serde_json::from_slice(data)
                .map_err(|e| internal_error(format!("Failed to parse resource: {}", e)))?;
            let line = serde_json::to_string(&resource)
                .map_err(|e| internal_error(format!("Failed to serialize resource: {}", e)))?;
            lines.push(line);
            last_cursor = Some(format!("{}|{}", last_updated, id));
        }

        Ok(NdjsonBatch {
            lines,
            next_cursor: if has_more { last_cursor } else { None },
            is_last: !has_more,
        })
    }
}

#[async_trait]
impl PatientExportProvider for SqliteBackend {
    async fn list_patient_ids(
        &self,
        tenant: &TenantContext,
        request: &ExportRequest,
        cursor: Option<&str>,
        batch_size: u32,
    ) -> StorageResult<(Vec<String>, Option<String>)> {
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        let mut query = "SELECT id FROM resources WHERE tenant_id = ?1 AND resource_type = 'Patient' AND is_deleted = 0".to_string();
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(tenant_id.to_string())];

        if let Some(since) = request.since {
            query.push_str(" AND last_updated >= ?");
            params_vec.push(Box::new(since.to_rfc3339()));
        }

        if let Some(cursor) = cursor {
            query.push_str(" AND id > ?");
            params_vec.push(Box::new(cursor.to_string()));
        }

        query.push_str(" ORDER BY id");
        query.push_str(&format!(" LIMIT {}", batch_size + 1));

        let params_slice: Vec<&dyn rusqlite::ToSql> =
            params_vec.iter().map(|p| p.as_ref()).collect();

        let mut stmt = conn
            .prepare(&query)
            .map_err(|e| internal_error(format!("Failed to prepare patient ids query: {}", e)))?;

        let ids: Vec<String> = stmt
            .query_map(params_slice.as_slice(), |row| row.get(0))
            .map_err(|e| internal_error(format!("Failed to query patient ids: {}", e)))?
            .filter_map(|r| r.ok())
            .collect();

        let has_more = ids.len() > batch_size as usize;
        let ids = if has_more {
            ids[..batch_size as usize].to_vec()
        } else {
            ids
        };

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

        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        // For Patient resources, just filter by the IDs
        if resource_type == "Patient" {
            let placeholders: Vec<String> = (0..patient_ids.len())
                .map(|i| format!("?{}", i + 3))
                .collect();
            let mut query = format!(
                "SELECT id, data, last_updated FROM resources
                 WHERE tenant_id = ?1 AND resource_type = ?2 AND id IN ({}) AND is_deleted = 0",
                placeholders.join(",")
            );

            let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![
                Box::new(tenant_id.to_string()),
                Box::new(resource_type.to_string()),
            ];
            for id in patient_ids {
                params_vec.push(Box::new(id.clone()));
            }

            if let Some(cursor) = cursor {
                let parts: Vec<&str> = cursor.splitn(2, '|').collect();
                if parts.len() == 2 {
                    query.push_str(" AND (last_updated, id) > (?, ?)");
                    params_vec.push(Box::new(parts[0].to_string()));
                    params_vec.push(Box::new(parts[1].to_string()));
                }
            }

            query.push_str(" ORDER BY last_updated, id");
            query.push_str(&format!(" LIMIT {}", batch_size + 1));

            let params_slice: Vec<&dyn rusqlite::ToSql> =
                params_vec.iter().map(|p| p.as_ref()).collect();

            let mut stmt = conn.prepare(&query).map_err(|e| {
                internal_error(format!("Failed to prepare compartment query: {}", e))
            })?;

            let rows: Vec<(String, Vec<u8>, String)> = stmt
                .query_map(params_slice.as_slice(), |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })
                .map_err(|e| internal_error(format!("Failed to query compartment: {}", e)))?
                .filter_map(|r| r.ok())
                .collect();

            let has_more = rows.len() > batch_size as usize;
            let rows = if has_more {
                &rows[..batch_size as usize]
            } else {
                &rows[..]
            };

            let mut lines = Vec::new();
            let mut last_cursor = None;

            for (id, data, last_updated) in rows {
                let resource: Value = serde_json::from_slice(data)
                    .map_err(|e| internal_error(format!("Failed to parse resource: {}", e)))?;
                let line = serde_json::to_string(&resource)
                    .map_err(|e| internal_error(format!("Failed to serialize resource: {}", e)))?;
                lines.push(line);
                last_cursor = Some(format!("{}|{}", last_updated, id));
            }

            return Ok(NdjsonBatch {
                lines,
                next_cursor: if has_more { last_cursor } else { None },
                is_last: !has_more,
            });
        }

        // For other resources, we need to use the search index to find resources
        // that reference these patients via subject/patient parameters
        let patient_refs: Vec<String> = patient_ids
            .iter()
            .map(|id| format!("Patient/{}", id))
            .collect();
        let placeholders: Vec<String> = (0..patient_refs.len())
            .map(|i| format!("?{}", i + 4))
            .collect();

        let mut query = format!(
            "SELECT DISTINCT r.id, r.data, r.last_updated
             FROM resources r
             INNER JOIN search_index si ON r.tenant_id = si.tenant_id
                AND r.resource_type = si.resource_type
                AND r.id = si.resource_id
             WHERE r.tenant_id = ?1
                AND r.resource_type = ?2
                AND r.is_deleted = 0
                AND si.param_name IN ('subject', 'patient')
                AND si.value_reference IN ({})",
            placeholders.join(",")
        );

        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![
            Box::new(tenant_id.to_string()),
            Box::new(resource_type.to_string()),
        ];
        // Placeholder for since filter slot
        let since_value = request.since.map(|s| s.to_rfc3339());
        if since_value.is_some() {
            params_vec.push(Box::new(since_value.clone().unwrap()));
        }
        for patient_ref in &patient_refs {
            params_vec.push(Box::new(patient_ref.clone()));
        }

        if request.since.is_some() {
            query = query.replace(
                "r.is_deleted = 0",
                "r.is_deleted = 0 AND r.last_updated >= ?3",
            );
        }

        if let Some(cursor) = cursor {
            let parts: Vec<&str> = cursor.splitn(2, '|').collect();
            if parts.len() == 2 {
                query.push_str(" AND (r.last_updated, r.id) > (?, ?)");
                params_vec.push(Box::new(parts[0].to_string()));
                params_vec.push(Box::new(parts[1].to_string()));
            }
        }

        query.push_str(" ORDER BY r.last_updated, r.id");
        query.push_str(&format!(" LIMIT {}", batch_size + 1));

        let params_slice: Vec<&dyn rusqlite::ToSql> =
            params_vec.iter().map(|p| p.as_ref()).collect();

        let mut stmt = conn
            .prepare(&query)
            .map_err(|e| internal_error(format!("Failed to prepare compartment query: {}", e)))?;

        let rows: Vec<(String, Vec<u8>, String)> = stmt
            .query_map(params_slice.as_slice(), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| internal_error(format!("Failed to query compartment: {}", e)))?
            .filter_map(|r| r.ok())
            .collect();

        let has_more = rows.len() > batch_size as usize;
        let rows = if has_more {
            &rows[..batch_size as usize]
        } else {
            &rows[..]
        };

        let mut lines = Vec::new();
        let mut last_cursor = None;

        for (id, data, last_updated) in rows {
            let resource: Value = serde_json::from_slice(data)
                .map_err(|e| internal_error(format!("Failed to parse resource: {}", e)))?;
            let line = serde_json::to_string(&resource)
                .map_err(|e| internal_error(format!("Failed to serialize resource: {}", e)))?;
            lines.push(line);
            last_cursor = Some(format!("{}|{}", last_updated, id));
        }

        Ok(NdjsonBatch {
            lines,
            next_cursor: if has_more { last_cursor } else { None },
            is_last: !has_more,
        })
    }
}

#[async_trait]
impl GroupExportProvider for SqliteBackend {
    async fn get_group_members(
        &self,
        tenant: &TenantContext,
        group_id: &str,
    ) -> StorageResult<Vec<String>> {
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        // Get the Group resource
        let data: Vec<u8> = conn
            .query_row(
                "SELECT data FROM resources WHERE tenant_id = ?1 AND resource_type = 'Group' AND id = ?2 AND is_deleted = 0",
                params![tenant_id, group_id],
                |row| row.get(0),
            )
            .map_err(|e| {
                if matches!(e, rusqlite::Error::QueryReturnedNoRows) {
                    StorageError::BulkExport(BulkExportError::GroupNotFound {
                        group_id: group_id.to_string(),
                    })
                } else {
                    internal_error(format!("Failed to get group: {}", e))
                }
            })?;

        let group: Value = serde_json::from_slice(&data)
            .map_err(|e| internal_error(format!("Failed to parse group: {}", e)))?;

        // Extract member references from Group.member[].entity.reference
        let mut members = Vec::new();
        if let Some(member_array) = group.get("member").and_then(|m| m.as_array()) {
            for member in member_array {
                if let Some(entity) = member.get("entity") {
                    if let Some(reference) = entity.get("reference").and_then(|r| r.as_str()) {
                        members.push(reference.to_string());
                    }
                }
            }
        }

        Ok(members)
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
            for reference in members {
                if let Some(pid) = reference.strip_prefix("Patient/") {
                    if seen_patients.insert(pid.to_string()) {
                        patient_ids.push(pid.to_string());
                    }
                } else if let Some(nested) = reference.strip_prefix("Group/") {
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
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();
        let data: Vec<u8> = conn
            .query_row(
                "SELECT data FROM resources
                 WHERE tenant_id = ?1 AND resource_type = 'Group'
                   AND id = ?2 AND is_deleted = 0",
                params![tenant_id, group_id],
                |row| row.get(0),
            )
            .map_err(|e| {
                if matches!(e, rusqlite::Error::QueryReturnedNoRows) {
                    StorageError::BulkExport(BulkExportError::GroupNotFound {
                        group_id: group_id.to_string(),
                    })
                } else {
                    internal_error(format!("Failed to get group: {}", e))
                }
            })?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ResourceStorage;
    use crate::tenant::{TenantId, TenantPermissions};
    use helios_fhir::FhirVersion;
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

    /// Wraps an `ExportRequest` in a `StartExportInput` with default kickoff metadata.
    fn test_input(request: ExportRequest) -> StartExportInput {
        StartExportInput {
            request,
            transaction_time: Utc::now(),
            request_url: "http://localhost/$export".to_string(),
            owner_subject: Some("test-subject".to_string()),
            fhir_version: FhirVersion::default(),
        }
    }

    #[tokio::test]
    async fn test_start_export() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let request = ExportRequest::system().with_types(vec!["Patient".to_string()]);
        let job_id = backend
            .start_export(&tenant, test_input(request))
            .await
            .unwrap();

        assert!(!job_id.as_str().is_empty());

        let progress = backend.get_export_status(&tenant, &job_id).await.unwrap();
        assert_eq!(progress.status, ExportStatus::Accepted);
    }

    #[tokio::test]
    async fn test_cancel_export() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let job_id = backend
            .start_export(&tenant, test_input(ExportRequest::system()))
            .await
            .unwrap();

        backend.cancel_export(&tenant, &job_id).await.unwrap();

        let progress = backend.get_export_status(&tenant, &job_id).await.unwrap();
        assert_eq!(progress.status, ExportStatus::Cancelled);
    }

    #[tokio::test]
    async fn test_list_exports() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let _job_id1 = backend
            .start_export(&tenant, test_input(ExportRequest::system()))
            .await
            .unwrap();
        let _job_id2 = backend
            .start_export(&tenant, test_input(ExportRequest::patient()))
            .await
            .unwrap();

        let exports = backend.list_exports(&tenant, false).await.unwrap();
        assert_eq!(exports.len(), 2);
    }

    #[tokio::test]
    async fn test_count_active_exports() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        for _ in 0..3 {
            backend
                .start_export(&tenant, test_input(ExportRequest::system()))
                .await
                .unwrap();
        }
        assert_eq!(backend.count_active_exports(&tenant).await.unwrap(), 3);
    }

    #[tokio::test]
    async fn test_get_export_job_metadata() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let job_id = backend
            .start_export(&tenant, test_input(ExportRequest::patient()))
            .await
            .unwrap();

        let meta = backend
            .get_export_job_metadata(&tenant, &job_id)
            .await
            .unwrap();
        assert_eq!(meta.status, ExportStatus::Accepted);
        assert_eq!(meta.owner_subject.as_deref(), Some("test-subject"));
        assert!(matches!(meta.level, ExportLevel::Patient));

        let missing = backend
            .get_export_job_metadata(&tenant, &ExportJobId::from_string("nope"))
            .await;
        assert!(missing.is_err());
    }

    #[tokio::test]
    async fn test_claim_and_worker_lifecycle() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let job_id = backend
            .start_export(&tenant, test_input(ExportRequest::system()))
            .await
            .unwrap();

        let worker = WorkerId::new("worker-1");
        let lease = backend
            .claim_next(&worker, StdDuration::from_secs(60))
            .await
            .unwrap()
            .expect("a job should be claimable");
        assert_eq!(lease.job_id, job_id);
        assert_eq!(lease.fencing_token, 1);

        // A second claim finds nothing (the only job is now in-progress).
        assert!(
            backend
                .claim_next(&worker, StdDuration::from_secs(60))
                .await
                .unwrap()
                .is_none()
        );

        // Worker can load, progress, finish.
        backend
            .mark_export_in_progress(&tenant, &job_id, &worker, lease.fencing_token)
            .await
            .unwrap();
        backend
            .update_export_type_progress(
                &tenant,
                &job_id,
                &worker,
                lease.fencing_token,
                &TypeExportProgress::new("Patient"),
            )
            .await
            .unwrap();
        backend
            .finish_export_job(&tenant, &job_id, &worker, lease.fencing_token)
            .await
            .unwrap();

        let progress = backend.get_export_status(&tenant, &job_id).await.unwrap();
        assert_eq!(progress.status, ExportStatus::Complete);
    }

    #[tokio::test]
    async fn test_stale_worker_fenced_out() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let job_id = backend
            .start_export(&tenant, test_input(ExportRequest::system()))
            .await
            .unwrap();

        let worker_a = WorkerId::new("worker-a");
        let lease_a = backend
            .claim_next(&worker_a, StdDuration::from_millis(1))
            .await
            .unwrap()
            .unwrap();

        // Lease expires; worker B reclaims, bumping the fencing token.
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let worker_b = WorkerId::new("worker-b");
        let lease_b = backend
            .claim_next(&worker_b, StdDuration::from_secs(60))
            .await
            .unwrap()
            .unwrap();
        assert!(lease_b.fencing_token > lease_a.fencing_token);

        // Worker A's stale mutations are all rejected as LeaseLost.
        assert!(matches!(
            backend
                .mark_export_in_progress(&tenant, &job_id, &worker_a, lease_a.fencing_token)
                .await,
            Err(LeaseError::LeaseLost { .. })
        ));
        assert!(matches!(
            backend
                .update_export_type_progress(
                    &tenant,
                    &job_id,
                    &worker_a,
                    lease_a.fencing_token,
                    &TypeExportProgress::new("Patient"),
                )
                .await,
            Err(LeaseError::LeaseLost { .. })
        ));
        assert!(matches!(
            backend
                .finish_export_job(&tenant, &job_id, &worker_a, lease_a.fencing_token)
                .await,
            Err(LeaseError::LeaseLost { .. })
        ));

        // Worker B can still operate.
        backend
            .finish_export_job(&tenant, &job_id, &worker_b, lease_b.fencing_token)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_since_newly_added_exclude_filters_late_joiners() {
        use crate::core::bulk_export_output::{ExportPartKey, ExportPartWriter};
        let _ = ExportPartKey::output("t", ExportJobId::new(), "x", 0, 0); // import sanity

        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // A Group with two members: one joined before _since (period.start =
        // 2024-01-01), one joined after (period.start = 2026-06-01).
        backend
            .create(
                &tenant,
                "Group",
                json!({
                    "resourceType": "Group", "id": "g-cohort",
                    "member": [
                        {
                            "entity": {"reference": "Patient/p-old"},
                            "period": {"start": "2024-01-01T00:00:00Z"}
                        },
                        {
                            "entity": {"reference": "Patient/p-new"},
                            "period": {"start": "2026-06-01T00:00:00Z"}
                        }
                    ]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let members = backend
            .get_group_members_with_periods(&tenant, "g-cohort")
            .await
            .unwrap();
        assert_eq!(members.len(), 2);
        assert!(members.iter().all(|(_, p)| p.is_some()));

        // Worker-level filter logic: with exclude=true and _since=2025,
        // p-new (joined 2026) should be filtered out; p-old kept.
        let since = chrono::DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let kept: Vec<String> = members
            .iter()
            .filter_map(|(reference, period_start)| {
                let pid = reference.strip_prefix("Patient/")?;
                match period_start {
                    Some(start) if *start > since => None,
                    _ => Some(pid.to_string()),
                }
            })
            .collect();
        assert_eq!(kept, vec!["p-old".to_string()]);

        // Drop reference to silence the unused-import allowance.
        let _ = ExportPartWriter::new(Box::pin(Vec::<u8>::new()));
    }

    #[tokio::test]
    async fn test_resolve_nested_groups_with_cycle_guard() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // g1 -> [Patient/p1, Group/g2]; g2 -> [Patient/p2, Group/g1 (cycle)]
        backend
            .create(
                &tenant,
                "Group",
                json!({
                    "resourceType": "Group", "id": "g1",
                    "member": [
                        {"entity": {"reference": "Patient/p1"}},
                        {"entity": {"reference": "Group/g2"}}
                    ]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create(
                &tenant,
                "Group",
                json!({
                    "resourceType": "Group", "id": "g2",
                    "member": [
                        {"entity": {"reference": "Patient/p2"}},
                        {"entity": {"reference": "Group/g1"}}
                    ]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let mut ids = backend
            .resolve_group_patient_ids(&tenant, "g1")
            .await
            .unwrap();
        ids.sort();
        // Both patients resolved exactly once; the cycle did not loop forever.
        assert_eq!(ids, vec!["p1".to_string(), "p2".to_string()]);
    }

    #[tokio::test]
    async fn test_list_export_types() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create some resources
        backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "name": [{"family": "Test"}]}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        backend
            .create(
                &tenant,
                "Observation",
                json!({"resourceType": "Observation", "status": "final"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let request = ExportRequest::system();
        let types = backend.list_export_types(&tenant, &request).await.unwrap();

        assert!(types.contains(&"Patient".to_string()));
        assert!(types.contains(&"Observation".to_string()));
    }

    #[tokio::test]
    async fn test_fetch_export_batch() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create some resources
        for i in 0..5 {
            backend
                .create(
                    &tenant,
                    "Patient",
                    json!({"resourceType": "Patient", "name": [{"family": format!("Patient{}", i)}]}),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }

        let request = ExportRequest::system();
        let batch = backend
            .fetch_export_batch(&tenant, &request, "Patient", None, 3)
            .await
            .unwrap();

        assert_eq!(batch.lines.len(), 3);
        assert!(!batch.is_last);
        assert!(batch.next_cursor.is_some());

        // Fetch next batch
        let batch2 = backend
            .fetch_export_batch(
                &tenant,
                &request,
                "Patient",
                batch.next_cursor.as_deref(),
                3,
            )
            .await
            .unwrap();

        assert_eq!(batch2.lines.len(), 2);
        assert!(batch2.is_last);
    }

    #[tokio::test]
    async fn test_delete_export() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let job_id = backend
            .start_export(&tenant, test_input(ExportRequest::system()))
            .await
            .unwrap();

        backend.delete_export(&tenant, &job_id).await.unwrap();

        // Should fail to get status now
        let result = backend.get_export_status(&tenant, &job_id).await;
        assert!(matches!(
            result,
            Err(StorageError::BulkExport(
                BulkExportError::JobNotFound { .. }
            ))
        ));
    }
}
