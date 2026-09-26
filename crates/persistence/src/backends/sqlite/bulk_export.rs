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
    abandoned_export_message, export_lease_expiry,
};
use crate::error::{
    BackendError, BulkExportError, QueryErrorExt, StorageError, StorageResult,
    classify_sqlite_error,
};
use crate::tenant::{TenantContext, TenantId, TenantPermissions};
use crate::types::StoredResource;

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

/// Builds one NDJSON export line from a `resources` row.
///
/// The stored blob carries no server `meta`: `versionId` and `lastUpdated`
/// live in their own columns. Merge them in the same way the REST read paths
/// do, so exported resources match `GET /<Type>/<id>` and consumers can derive
/// their next `_since` from the downloaded parts (#1273).
fn export_line(data: &[u8], version_id: &str, last_updated: &str) -> StorageResult<String> {
    let resource: Value = serde_json::from_slice(data)
        .map_err(|e| internal_error(format!("Failed to parse resource: {}", e)))?;
    let resource = StoredResource::merge_meta(resource, version_id, parse_dt(last_updated)?);
    serde_json::to_string(&resource)
        .map_err(|e| internal_error(format!("Failed to serialize resource: {}", e)))
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

/// Appends the `_since` / `_until` window to an export query and binds the
/// bounds.
///
/// Both are inclusive, matching the S3 backend's `last_modified() < since` /
/// `> until` skips. Every export query path uses this, so a job's count and its
/// emitted rows cannot disagree about the window.
///
/// The placeholders are anonymous on purpose. SQLite gives each `?` one more
/// than the highest index used so far, so a query that binds only the upper
/// bound cannot mis-number it the way a hard-coded `?3` would.
fn push_export_window(
    query: &mut String,
    params: &mut Vec<Box<dyn rusqlite::ToSql>>,
    request: &ExportRequest,
) {
    if let Some(since) = request.since {
        query.push_str(" AND last_updated >= ?");
        params.push(Box::new(since.to_rfc3339()));
    }
    if let Some(until) = request.until {
        query.push_str(" AND last_updated <= ?");
        params.push(Box::new(until.to_rfc3339()));
    }
}

/// Wraps a *non-driver* failure (serde, chrono, an enum parse of a column)
/// as [`BackendError::Internal`].
///
/// Driver failures must NOT come through here: a `rusqlite::Error` carries a
/// result code, and flattening it into a string threw away `SQLITE_BUSY` /
/// `SQLITE_LOCKED`, so a kick-off that merely lost a race with a background
/// index rebuild surfaced as a 500 instead of a retryable 503 (#1185). Use
/// [`QueryErrorExt::or_query_error`] (or [`classify_sqlite_error`] where the
/// error is already unwrapped) for anything that came out of rusqlite.
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
        .or_query_error("Failed to create export job")?;

        Ok(job_id)
    }

    async fn get_export_status(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
    ) -> StorageResult<ExportProgress> {
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        let (status_str, level_str, group_id, transaction_time, started_at, completed_at, error_message, current_type, types_done, types_total):
            (String, String, Option<String>, String, Option<String>, Option<String>, Option<String>, Option<String>, i64, i64) = conn
            .query_row(
                "SELECT status, level, group_id, transaction_time, started_at, completed_at, error_message, current_type, types_done, types_total
                 FROM bulk_export_jobs
                 WHERE id = ?1 AND tenant_id = ?2",
                params![job_id.as_str(), tenant_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?,
                          row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?,
                          row.get(8)?, row.get(9)?)),
            )
            .map_err(|e| {
                if matches!(e, rusqlite::Error::QueryReturnedNoRows) {
                    StorageError::BulkExport(BulkExportError::JobNotFound {
                        job_id: job_id.to_string(),
                    })
                } else {
                    StorageError::Backend(classify_sqlite_error("Failed to get export status", e))
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
            .or_query_error("Failed to prepare progress query")?;

        // `query_map` only binds the parameters — the first `sqlite3_step`,
        // and with it a `SQLITE_BUSY` from a rebuild holding the write lock,
        // lands on the per-row `Result` below. Discarding those with
        // `filter_map(|r| r.ok())` reported an empty read as success (#1185),
        // so every row iteration in this file is collected through its error.
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
            .or_query_error("Failed to query progress")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .or_query_error("Failed to read progress rows")?;

        Ok(ExportProgress {
            job_id: job_id.clone(),
            status,
            level,
            transaction_time,
            started_at,
            completed_at,
            type_progress,
            current_type,
            types_done: types_done as u32,
            types_total: types_total as u32,
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
                    StorageError::Backend(classify_sqlite_error("Failed to get export status", e))
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
        .or_query_error("Failed to cancel export")?;

        Ok(())
    }

    async fn delete_export(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
    ) -> StorageResult<()> {
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        // Check exists. A driver failure here is not an absent job: swallowing
        // it would answer 404 for a row that is merely locked (#1185), so only
        // an empty result means "gone".
        let exists: bool = match conn.query_row(
            "SELECT 1 FROM bulk_export_jobs WHERE id = ?1 AND tenant_id = ?2",
            params![job_id.as_str(), tenant_id],
            |_| Ok(true),
        ) {
            Ok(found) => found,
            Err(rusqlite::Error::QueryReturnedNoRows) => false,
            Err(e) => {
                return Err(StorageError::Backend(classify_sqlite_error(
                    "Failed to look up export job",
                    e,
                )));
            }
        };

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
        .or_query_error("Failed to delete export")?;

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
                    StorageError::Backend(classify_sqlite_error("Failed to get export job", e))
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
            .or_query_error("Failed to prepare files query")?;

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
            .or_query_error("Failed to query files")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .or_query_error("Failed to read export file rows")?;

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
                .or_query_error("Failed to prepare list query")?;

            stmt.query_map(params![tenant_id], |row| row.get(0))
                .or_query_error("Failed to query exports")?
                .collect::<rusqlite::Result<Vec<_>>>()
                .or_query_error("Failed to read export rows")?
        };

        let mut results = Vec::new();
        for id in job_ids {
            let job_id = ExportJobId::from_string(id);
            // A job deleted between listing the ids and reading its status is
            // a benign race, and drops out of the list. Anything else must
            // not: swallowing a busy database here returned a short list as
            // if those exports had never existed (#1185).
            match self.get_export_status(tenant, &job_id).await {
                Ok(progress) => results.push(progress),
                Err(StorageError::BulkExport(BulkExportError::JobNotFound { .. })) => {}
                Err(e) => return Err(e),
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
                    StorageError::Backend(classify_sqlite_error("Failed to get export job metadata", e))
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
                    StorageError::Backend(classify_sqlite_error(
                        "Failed to get export file metadata",
                        e,
                    ))
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
            .or_query_error("Failed to count active exports")?;
        Ok(count as u64)
    }

    async fn count_exports_by_status(
        &self,
        tenant: &TenantContext,
        status: ExportStatus,
    ) -> StorageResult<u64> {
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM bulk_export_jobs WHERE tenant_id = ?1 AND status = ?2",
                params![tenant_id, status.to_string()],
                |row| row.get(0),
            )
            .or_query_error("Failed to count exports by status")?;
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
            .or_query_error("Failed to prepare expired query")?;

        let rows: Vec<(String, String)> = stmt
            .query_map(params![cutoff, limit], |row| Ok((row.get(0)?, row.get(1)?)))
            .or_query_error("Failed to query expired exports")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .or_query_error("Failed to read expired export rows")?;

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
        max_attempts: u32,
    ) -> StorageResult<Option<ExportJobLease>> {
        let _guard = CLAIM_LOCK.lock().await;
        let mut conn = self.get_connection()?;
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        let lease_expiry = export_lease_expiry(now, lease_duration);
        let lease_expiry_str = lease_expiry.to_rfc3339();

        // Lock-free eligibility probe, in autocommit. The claim itself needs an
        // IMMEDIATE transaction, which takes SQLite's single write lock the
        // moment it begins — so without this the *idle* poll queued behind
        // every long writer too. Against a search-index rebuild (a ~500ms lock
        // with a 5ms gap) a claim poll waited ~10s on average and sometimes
        // blew past `busy_timeout`, parking a worker thread and logging an
        // error for an empty queue. In WAL mode a reader never blocks, so an
        // empty queue now costs one uncontended SELECT (#1185 is the same
        // pathology on the kick-off insert).
        //
        // The race with the transaction below is benign and needs no handling:
        // a job that appears right after the probe is claimed by the next poll
        // (2s later), exactly as before this transaction existed; a job that
        // disappears leaves the scan's own SELECT empty, which is the
        // already-existing `break None` path. The in-transaction SELECT stays
        // authoritative, so bump-and-wipe atomicity is untouched.
        let eligible = match conn.query_row(
            "SELECT 1 FROM bulk_export_jobs
             WHERE status = 'accepted'
                OR (status = 'in-progress' AND (lease_expiry IS NULL OR lease_expiry < ?1))
             LIMIT 1",
            params![now_str],
            |_| Ok(()),
        ) {
            Ok(()) => true,
            Err(rusqlite::Error::QueryReturnedNoRows) => false,
            // Never `.ok()` here: a busy database is not an empty queue (#1185).
            Err(e) => {
                return Err(StorageError::Backend(classify_sqlite_error(
                    "Failed to probe for an eligible export job",
                    e,
                )));
            }
        };
        if !eligible {
            return Ok(None);
        }

        // The whole scan runs inside one IMMEDIATE transaction. The token bump
        // and the wipe of a re-claimed job's half-written state have to land
        // together: a reader that saw the new token but the old progress rows
        // would resume a run that is in the middle of being restarted (#1041).
        // `CLAIM_LOCK` only serializes claims within this process.
        let txn = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .or_query_error("Failed to begin claim txn")?;

        // Each turn either claims a job or retires one whose attempts are
        // spent. Retiring moves the job out of the eligible set, so the scan
        // makes progress on every turn and a caller is never left empty-handed
        // while another job is still claimable (#1041).
        let claimed = loop {
            // Find one eligible job: accepted, or in-progress with an expired lease.
            // Only an empty result means "nothing to do": discarding the
            // error here made a `SQLITE_BUSY` from a concurrent index rebuild
            // look like an idle queue, so the worker parked instead of
            // retrying and the export never started (#1185). The worker loop
            // already logs and backs off on `Err`.
            let row: Option<(String, String, String, i64, i64)> = match txn.query_row(
                "SELECT id, tenant_id, status, fencing_token, attempts FROM bulk_export_jobs
                 WHERE status = 'accepted'
                    OR (status = 'in-progress' AND (lease_expiry IS NULL OR lease_expiry < ?1))
                 ORDER BY created_at LIMIT 1",
                params![now_str],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            ) {
                Ok(found) => Some(found),
                Err(rusqlite::Error::QueryReturnedNoRows) => None,
                Err(e) => {
                    return Err(StorageError::Backend(classify_sqlite_error(
                        "Failed to select an eligible export job",
                        e,
                    )));
                }
            };

            let Some((job_id, tenant_id, status, fencing_token, attempts)) = row else {
                break None;
            };
            let new_token = fencing_token + 1;
            let attempt = attempts + 1;

            if attempt > i64::from(max_attempts) {
                // Every worker that held this job lost its lease before
                // finishing. Hand it to no one else: fail it terminally so the
                // status poll ends in an error the client can act on, and so
                // the job stops occupying one of the tenant's export slots.
                let attempts_made = u32::try_from(attempts).unwrap_or(u32::MAX);
                txn.execute(
                    "UPDATE bulk_export_jobs
                     SET status = 'error', error_message = ?1, completed_at = ?2,
                         current_type = NULL, worker_id = NULL, lease_expiry = NULL
                     WHERE id = ?3",
                    params![abandoned_export_message(attempts_made), now_str, job_id],
                )
                .or_query_error("Failed to abandon export job")?;
                tracing::warn!(
                    job_id = %job_id,
                    attempts = attempts_made,
                    "export job abandoned: its lease expired on every attempt"
                );
                // No wipe: the job is terminal, and the output TTL sweep
                // reclaims its rows and its artifacts together.
                continue;
            }

            // A re-claimed job restarts from scratch. The worker resumes
            // *within* a type from `cursor_state` but always restarts
            // `part_index` at 0, so keeping the previous attempt's rows would
            // let the resumed run overwrite parts 0..n of the half-finished
            // type — silently dropping every resource the dead worker had
            // already written, with the job still ending `complete` (#1041).
            // Types that did finish would also be exported twice, inflating
            // `exported_count`. A job still `accepted` never wrote anything.
            //
            // Only the rows go. The artifacts stay: unlinking them would pull
            // the `.tmp` file out from under a zombie worker, whose
            // `finalize_part` would then fail its rename with a plain
            // `StorageError` instead of `LeaseLost` — and that makes the
            // worker loop emit a spurious `failed` audit event for a job now
            // running under someone else's lease. The periodic TTL cleanup
            // drops the job's whole output prefix anyway.
            if status == "in-progress" {
                txn.execute(
                    "DELETE FROM bulk_export_progress WHERE job_id = ?1",
                    params![job_id],
                )
                .or_query_error("Failed to clear reclaimed progress")?;
                txn.execute(
                    "DELETE FROM bulk_export_files WHERE job_id = ?1",
                    params![job_id],
                )
                .or_query_error("Failed to clear reclaimed file rows")?;
                tracing::info!(
                    job_id = %job_id,
                    attempt,
                    "reclaimed export job: discarding the previous attempt's progress"
                );
            }

            txn.execute(
                "UPDATE bulk_export_jobs
                 SET status = 'in-progress', worker_id = ?1, lease_expiry = ?2,
                     heartbeat_at = ?3, fencing_token = ?4, attempts = ?5,
                     started_at = COALESCE(started_at, ?3)
                 WHERE id = ?6",
                params![
                    worker_id.as_str(),
                    lease_expiry_str,
                    now_str,
                    new_token,
                    attempt,
                    job_id
                ],
            )
            .or_query_error("Failed to claim export job")?;

            break Some(ExportJobLease {
                job_id: ExportJobId::from_string(job_id),
                tenant: TenantContext::new(
                    TenantId::new(tenant_id),
                    TenantPermissions::full_access(),
                ),
                worker_id: worker_id.clone(),
                lease_expiry,
                fencing_token: new_token as u64,
                lease_duration,
            });
        };

        // Committed on both paths: an empty scan may still have retired jobs.
        txn.commit().or_query_error("Failed to commit claim txn")?;

        Ok(claimed)
    }

    async fn heartbeat(&self, lease: &ExportJobLease) -> Result<DateTime<Utc>, LeaseError> {
        let conn = self.get_connection().map_err(LeaseError::Storage)?;
        let now = Utc::now();
        // Renew by the duration the job was claimed under, not by a constant
        // this backend picked: a deployment that raises
        // `HFS_BULK_EXPORT_LEASE_DURATION` for slow batches would otherwise see
        // every heartbeat shrink the lease back to 60s, and the job be
        // reclaimed mid-run (#1152).
        let new_expiry = export_lease_expiry(now, lease.lease_duration);
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
            .or_query_error("heartbeat failed")
            .map_err(LeaseError::Storage)?;
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
        .or_query_error("Failed to release lease")?;
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

        let (request_json, level_str, group_id, transaction_time, fhir_version, owner_subject): (
            String,
            String,
            Option<String>,
            String,
            String,
            Option<String>,
        ) = conn
            .query_row(
                "SELECT request_json, level, group_id, transaction_time, fhir_version, owner_subject
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
                        row.get(5)?,
                    ))
                },
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => LeaseError::LeaseLost {
                    job_id: job_id.clone(),
                },
                other => LeaseError::Storage(StorageError::Backend(classify_sqlite_error(
                    "Failed to load worker job",
                    other,
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
        let fhir_version = helios_fhir::FhirVersion::from_mime_param(&fhir_version)
            .unwrap_or_else(helios_fhir::FhirVersion::default_enabled);
        let transaction_time = parse_dt(&transaction_time).map_err(LeaseError::Storage)?;

        // Load persisted per-type progress for resume.
        let mut stmt = conn
            .prepare(
                "SELECT resource_type, total_count, exported_count, error_count, cursor_state
                 FROM bulk_export_progress WHERE job_id = ?1",
            )
            .or_query_error("prepare progress")
            .map_err(LeaseError::Storage)?;
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
            .or_query_error("query progress")
            .map_err(LeaseError::Storage)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .or_query_error("read progress rows")
            .map_err(LeaseError::Storage)?;

        Ok(WorkerJobView {
            request,
            level,
            transaction_time,
            fhir_version,
            type_progress,
            owner_subject,
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
            .or_query_error("mark_in_progress")
            .map_err(LeaseError::Storage)?;
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
            .or_query_error("update_type_progress")
            .map_err(LeaseError::Storage)?;
        if affected == 0 {
            Err(LeaseError::LeaseLost {
                job_id: job_id.clone(),
            })
        } else {
            Ok(())
        }
    }

    async fn set_export_current_type(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
        worker_id: &WorkerId,
        fencing_token: u64,
        current_type: Option<&str>,
        types_done: u32,
        types_total: u32,
    ) -> Result<(), LeaseError> {
        let conn = self.get_connection().map_err(LeaseError::Storage)?;
        let affected = conn
            .execute(
                "UPDATE bulk_export_jobs
                 SET current_type = ?1, types_done = ?2, types_total = ?3
                 WHERE id = ?4 AND tenant_id = ?5 AND worker_id = ?6 AND fencing_token = ?7",
                params![
                    current_type,
                    types_done as i64,
                    types_total as i64,
                    job_id.as_str(),
                    tenant.tenant_id().as_str(),
                    worker_id.as_str(),
                    fencing_token as i64
                ],
            )
            .or_query_error("set_export_current_type")
            .map_err(LeaseError::Storage)?;
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
            .or_query_error("record_export_file")
            .map_err(LeaseError::Storage)?;
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
                 SET status = 'complete', completed_at = ?1, current_type = NULL
                 WHERE id = ?2 AND tenant_id = ?3 AND worker_id = ?4 AND fencing_token = ?5",
                params![
                    now,
                    job_id.as_str(),
                    tenant.tenant_id().as_str(),
                    worker_id.as_str(),
                    fencing_token as i64
                ],
            )
            .or_query_error("finish_job")
            .map_err(LeaseError::Storage)?;
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
                 SET status = 'error', error_message = ?1, completed_at = ?2, current_type = NULL
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
            .or_query_error("fail_job")
            .map_err(LeaseError::Storage)?;
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
                // Only an empty result means "this type has no data". Treating a
                // driver failure as absence would quietly drop a requested type
                // from the export and still report success (#1185).
                let exists: bool = match conn.query_row(
                    "SELECT 1 FROM resources WHERE tenant_id = ?1 AND resource_type = ?2 AND is_deleted = 0 LIMIT 1",
                    params![tenant_id, rt],
                    |_| Ok(true),
                ) {
                    Ok(found) => found,
                    Err(rusqlite::Error::QueryReturnedNoRows) => false,
                    Err(e) => {
                        return Err(StorageError::Backend(classify_sqlite_error(
                            "Failed to probe resource type",
                            e,
                        )));
                    }
                };
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
            .or_query_error("Failed to prepare types query")?;

        let types: Vec<String> = stmt
            .query_map(params![tenant_id], |row| row.get(0))
            .or_query_error("Failed to query types")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .or_query_error("Failed to read resource type rows")?;

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

        push_export_window(&mut query, &mut params_vec, request);

        let params_slice: Vec<&dyn rusqlite::ToSql> =
            params_vec.iter().map(|p| p.as_ref()).collect();

        let count: i64 = conn
            .query_row(&query, params_slice.as_slice(), |row| row.get(0))
            .or_query_error("Failed to count resources")?;

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

        let mut query = "SELECT id, data, last_updated, version_id FROM resources WHERE tenant_id = ?1 AND resource_type = ?2 AND is_deleted = 0".to_string();
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![
            Box::new(tenant_id.to_string()),
            Box::new(resource_type.to_string()),
        ];

        push_export_window(&mut query, &mut params_vec, request);

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
            .or_query_error("Failed to prepare batch query")?;

        let rows: Vec<(String, Vec<u8>, String, String)> = stmt
            .query_map(params_slice.as_slice(), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .or_query_error("Failed to query batch")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .or_query_error("Failed to read batch rows")?;

        let has_more = rows.len() > batch_size as usize;
        let rows = if has_more {
            &rows[..batch_size as usize]
        } else {
            &rows[..]
        };

        let mut lines = Vec::new();
        let mut last_cursor = None;

        for (id, data, last_updated, version_id) in rows {
            lines.push(export_line(data, version_id, last_updated)?);
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

        // `_since` only, deliberately: this selects WHICH patients are in scope,
        // not which of their resources are exported. Bounding it above by
        // `_until` would drop a patient whose own record was touched after the
        // window and take their in-window compartment resources with them. The
        // Patient resource itself is still bounded, by the compartment fetch.
        // S3 makes the same distinction (`backends/s3/bulk_export.rs:216`).
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
            .or_query_error("Failed to prepare patient ids query")?;

        let ids: Vec<String> = stmt
            .query_map(params_slice.as_slice(), |row| row.get(0))
            .or_query_error("Failed to query patient ids")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .or_query_error("Failed to read patient id rows")?;

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
                "SELECT id, data, last_updated, version_id FROM resources
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

            // Same `_since` / `_until` window as the non-Patient branch below.
            // Anonymous `?` placeholders are correct even though the id list is
            // numbered: SQLite gives a bare `?` one more than the highest index
            // used so far, so these bind after the ids and before the cursor's
            // own two, matching the order they are pushed in.
            push_export_window(&mut query, &mut params_vec, request);

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

            let mut stmt = conn
                .prepare(&query)
                .or_query_error("Failed to prepare compartment query")?;

            let rows: Vec<(String, Vec<u8>, String, String)> = stmt
                .query_map(params_slice.as_slice(), |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })
                .or_query_error("Failed to query compartment")?
                .collect::<rusqlite::Result<Vec<_>>>()
                .or_query_error("Failed to read compartment rows")?;

            let has_more = rows.len() > batch_size as usize;
            let rows = if has_more {
                &rows[..batch_size as usize]
            } else {
                &rows[..]
            };

            let mut lines = Vec::new();
            let mut last_cursor = None;

            for (id, data, last_updated, version_id) in rows {
                lines.push(export_line(data, version_id, last_updated)?);
                last_cursor = Some(format!("{}|{}", last_updated, id));
            }

            return Ok(NdjsonBatch {
                lines,
                next_cursor: if has_more { last_cursor } else { None },
                is_last: !has_more,
            });
        }

        // For other resource types, find resources whose payload references one
        // of the patients via `subject.reference` or `patient.reference`. We
        // read the JSON payload directly (json_extract over the `data` column)
        // rather than the search_index, so this is correct even when search is
        // offloaded to a secondary backend (sqlite-elasticsearch), which leaves
        // the local search_index empty.
        let patient_refs: Vec<String> = patient_ids
            .iter()
            .map(|id| format!("Patient/{}", id))
            .collect();

        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![
            Box::new(tenant_id.to_string()),
            Box::new(resource_type.to_string()),
        ];
        let mut query = "SELECT id, data, last_updated, version_id FROM resources \
             WHERE tenant_id = ? AND resource_type = ? AND is_deleted = 0"
            .to_string();

        push_export_window(&mut query, &mut params_vec, request);

        let placeholders: Vec<&str> = patient_refs.iter().map(|_| "?").collect();
        let in_list = placeholders.join(",");
        query.push_str(&format!(
            " AND (json_extract(data, '$.subject.reference') IN ({in_list}) \
               OR json_extract(data, '$.patient.reference') IN ({in_list}))"
        ));
        // The IN-list params appear twice (subject + patient), so bind twice.
        for patient_ref in &patient_refs {
            params_vec.push(Box::new(patient_ref.clone()));
        }
        for patient_ref in &patient_refs {
            params_vec.push(Box::new(patient_ref.clone()));
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

        let mut stmt = conn
            .prepare(&query)
            .or_query_error("Failed to prepare compartment query")?;

        let rows: Vec<(String, Vec<u8>, String, String)> = stmt
            .query_map(params_slice.as_slice(), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .or_query_error("Failed to query compartment")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .or_query_error("Failed to read compartment rows")?;

        let has_more = rows.len() > batch_size as usize;
        let rows = if has_more {
            &rows[..batch_size as usize]
        } else {
            &rows[..]
        };

        let mut lines = Vec::new();
        let mut last_cursor = None;

        for (id, data, last_updated, version_id) in rows {
            lines.push(export_line(data, version_id, last_updated)?);
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
                    StorageError::Backend(classify_sqlite_error("Failed to get group", e))
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
                    StorageError::Backend(classify_sqlite_error("Failed to get group", e))
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

    /// Claim cap for tests that are not exercising the cap itself.
    const TEST_MAX_ATTEMPTS: u32 = 3;

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
    async fn count_exports_by_status_splits_accepted_and_in_progress_per_tenant() {
        let backend = create_test_backend();
        let tenant_a = create_test_tenant();
        let tenant_b = TenantContext::new(
            TenantId::new("other-tenant"),
            TenantPermissions::full_access(),
        );

        backend
            .start_export(&tenant_a, test_input(ExportRequest::system()))
            .await
            .unwrap();
        backend
            .start_export(&tenant_a, test_input(ExportRequest::system()))
            .await
            .unwrap();

        // Move one of tenant A's jobs to in-progress via the real worker path.
        let worker = WorkerId::new("worker-1");
        let lease = backend
            .claim_next(&worker, StdDuration::from_secs(60), TEST_MAX_ATTEMPTS)
            .await
            .unwrap()
            .expect("a job should be claimable");
        backend
            .mark_export_in_progress(&tenant_a, &lease.job_id, &worker, lease.fencing_token)
            .await
            .unwrap();

        assert_eq!(
            backend
                .count_exports_by_status(&tenant_a, ExportStatus::Accepted)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            backend
                .count_exports_by_status(&tenant_a, ExportStatus::InProgress)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            backend
                .count_exports_by_status(&tenant_a, ExportStatus::Complete)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            backend
                .count_exports_by_status(&tenant_b, ExportStatus::Accepted)
                .await
                .unwrap(),
            0
        );

        // The concurrency-cap aggregate is unaffected by the new method.
        assert_eq!(backend.count_active_exports(&tenant_a).await.unwrap(), 2);
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
            .claim_next(&worker, StdDuration::from_secs(60), TEST_MAX_ATTEMPTS)
            .await
            .unwrap()
            .expect("a job should be claimable");
        assert_eq!(lease.job_id, job_id);
        assert_eq!(lease.fencing_token, 1);

        // A second claim finds nothing (the only job is now in-progress).
        assert!(
            backend
                .claim_next(&worker, StdDuration::from_secs(60), TEST_MAX_ATTEMPTS)
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
            .claim_next(&worker_a, StdDuration::from_millis(1), TEST_MAX_ATTEMPTS)
            .await
            .unwrap()
            .unwrap();

        // Lease expires; worker B reclaims, bumping the fencing token.
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let worker_b = WorkerId::new("worker-b");
        let lease_b = backend
            .claim_next(&worker_b, StdDuration::from_secs(60), TEST_MAX_ATTEMPTS)
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

    /// Reads back the lease timestamps that a claim or heartbeat persisted.
    fn persisted_lease_row(
        backend: &SqliteBackend,
        job_id: &ExportJobId,
    ) -> (DateTime<Utc>, DateTime<Utc>) {
        let conn = backend.get_connection().unwrap();
        let (expiry, heartbeat): (String, String) = conn
            .query_row(
                "SELECT lease_expiry, heartbeat_at FROM bulk_export_jobs WHERE id = ?1",
                params![job_id.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        (parse_dt(&expiry).unwrap(), parse_dt(&heartbeat).unwrap())
    }

    /// Claims the single job of a fresh backend under `lease_duration`.
    async fn claim_one(
        backend: &SqliteBackend,
        worker: &WorkerId,
        lease_duration: StdDuration,
    ) -> ExportJobLease {
        backend
            .claim_next(worker, lease_duration, TEST_MAX_ATTEMPTS)
            .await
            .unwrap()
            .expect("a job should be claimable")
    }

    /// Claims and repeated heartbeats preserve short, default, and long lease
    /// durations in both stored timestamps (#1152).
    #[tokio::test]
    async fn test_claim_duration_controls_repeated_heartbeats() {
        for seconds in [30, 60, 180] {
            let backend = create_test_backend();
            let tenant = create_test_tenant();
            let job_id = backend
                .start_export(&tenant, test_input(ExportRequest::system()))
                .await
                .unwrap();
            let configured = StdDuration::from_secs(seconds);
            let worker = WorkerId::new(format!("worker-{seconds}"));
            let lease = claim_one(&backend, &worker, configured).await;
            let expected = chrono::Duration::seconds(seconds as i64);

            assert_eq!(lease.lease_duration, configured);
            let (claimed_expiry, claimed_heartbeat) = persisted_lease_row(&backend, &job_id);
            assert_eq!(lease.lease_expiry, claimed_expiry);
            assert_eq!(claimed_expiry - claimed_heartbeat, expected);

            for renewal in 1..=2 {
                let returned = backend.heartbeat(&lease).await.unwrap();
                let (persisted_expiry, heartbeat_at) = persisted_lease_row(&backend, &job_id);
                assert_eq!(
                    returned, persisted_expiry,
                    "renewal {renewal} for {seconds}s must return its stored expiry"
                );
                assert_eq!(
                    persisted_expiry - heartbeat_at,
                    expected,
                    "renewal {renewal} must preserve the {seconds}s claim duration"
                );
            }
        }
    }

    /// Renewing by the configured duration must not weaken fencing: a
    /// heartbeat from a worker whose job was reclaimed still loses its lease.
    #[tokio::test]
    async fn test_heartbeat_on_a_stolen_lease_is_lease_lost() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let job_id = backend
            .start_export(&tenant, test_input(ExportRequest::system()))
            .await
            .unwrap();

        let worker_a = WorkerId::new("worker-a");
        let lease_a = claim_one(&backend, &worker_a, StdDuration::from_secs(30)).await;

        // Expire only the row still owned by worker A. This avoids a timing
        // dependency while proving the update itself is fenced.
        let past = Utc::now() - chrono::Duration::seconds(1);
        let affected = backend
            .get_connection()
            .unwrap()
            .execute(
                "UPDATE bulk_export_jobs SET lease_expiry = ?1
                 WHERE id = ?2 AND worker_id = ?3 AND fencing_token = ?4",
                params![
                    past.to_rfc3339(),
                    job_id.as_str(),
                    worker_a.as_str(),
                    lease_a.fencing_token as i64
                ],
            )
            .unwrap();
        assert_eq!(affected, 1, "the test must expire exactly worker A's row");

        let worker_b = WorkerId::new("worker-b");
        let lease_b = claim_one(&backend, &worker_b, StdDuration::from_secs(180)).await;
        assert!(lease_b.fencing_token > lease_a.fencing_token);
        assert_eq!(lease_b.lease_duration, StdDuration::from_secs(180));
        let row_after_steal = persisted_lease_row(&backend, &job_id);

        assert!(matches!(
            backend.heartbeat(&lease_a).await,
            Err(LeaseError::LeaseLost { job_id: lost }) if lost == job_id
        ));
        assert_eq!(
            persisted_lease_row(&backend, &job_id),
            row_after_steal,
            "the stale heartbeat must change neither lease timestamp"
        );

        let returned = backend.heartbeat(&lease_b).await.unwrap();
        let (persisted_expiry, heartbeat_at) = persisted_lease_row(&backend, &job_id);
        assert_eq!(returned, persisted_expiry);
        assert_eq!(
            persisted_expiry - heartbeat_at,
            chrono::Duration::seconds(180),
            "the new owner's duration controls its renewal"
        );
    }

    /// Counts a job's rows in the two tables a re-claim wipes; no trait
    /// surfaces them as raw counts.
    fn attempt_row_counts(backend: &SqliteBackend, job_id: &ExportJobId) -> (i64, i64) {
        let conn = backend.get_connection().unwrap();
        let progress = conn
            .query_row(
                "SELECT COUNT(*) FROM bulk_export_progress WHERE job_id = ?1",
                params![job_id.as_str()],
                |row| row.get(0),
            )
            .unwrap();
        let files = conn
            .query_row(
                "SELECT COUNT(*) FROM bulk_export_files WHERE job_id = ?1",
                params![job_id.as_str()],
                |row| row.get(0),
            )
            .unwrap();
        (progress, files)
    }

    /// Records one finalized output part the way the worker does after a flush.
    #[allow(clippy::too_many_arguments)]
    async fn record_output_part(
        backend: &SqliteBackend,
        tenant: &TenantContext,
        job_id: &ExportJobId,
        worker: &WorkerId,
        fencing_token: u64,
        resource_type: &str,
        part_index: u32,
        line_count: u64,
    ) {
        let part = FinalizedPart {
            key: ExportPartKey::output(
                tenant.tenant_id().as_str(),
                job_id.clone(),
                resource_type,
                part_index,
                fencing_token,
            ),
            resource_type: resource_type.to_string(),
            line_count,
            size_bytes: line_count * 120,
        };
        backend
            .record_export_file(tenant, job_id, worker, fencing_token, &part, "output")
            .await
            .unwrap();
    }

    /// Persists per-type progress the way the worker does between batches.
    async fn record_type_progress(
        backend: &SqliteBackend,
        tenant: &TenantContext,
        job_id: &ExportJobId,
        worker: &WorkerId,
        fencing_token: u64,
        progress: TypeExportProgress,
    ) {
        backend
            .update_export_type_progress(tenant, job_id, worker, fencing_token, &progress)
            .await
            .unwrap();
    }

    /// Re-claiming a job whose lease expired mid-run drops everything the dead
    /// worker wrote, in the same transaction that bumps the fencing token, so
    /// the new lease starts from a clean slate (#1041).
    #[tokio::test]
    async fn test_reclaim_discards_the_previous_attempts_rows() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let job_id = backend
            .start_export(&tenant, test_input(ExportRequest::system()))
            .await
            .unwrap();

        let worker_a = WorkerId::new("worker-a");
        let lease_a = backend
            .claim_next(&worker_a, StdDuration::from_millis(1), TEST_MAX_ATTEMPTS)
            .await
            .unwrap()
            .unwrap();
        backend
            .mark_export_in_progress(&tenant, &job_id, &worker_a, lease_a.fencing_token)
            .await
            .unwrap();
        let mut patient = TypeExportProgress::new("Patient");
        patient.exported_count = 200;
        patient.cursor_state = Some("page-3".to_string());
        record_type_progress(
            &backend,
            &tenant,
            &job_id,
            &worker_a,
            lease_a.fencing_token,
            patient,
        )
        .await;
        record_output_part(
            &backend,
            &tenant,
            &job_id,
            &worker_a,
            lease_a.fencing_token,
            "Patient",
            0,
            100,
        )
        .await;
        record_output_part(
            &backend,
            &tenant,
            &job_id,
            &worker_a,
            lease_a.fencing_token,
            "Patient",
            1,
            100,
        )
        .await;
        assert_eq!(
            attempt_row_counts(&backend, &job_id),
            (1, 2),
            "the first attempt wrote progress and file rows"
        );

        // The lease lapses and worker B takes over.
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let worker_b = WorkerId::new("worker-b");
        let lease_b = backend
            .claim_next(&worker_b, StdDuration::from_secs(60), TEST_MAX_ATTEMPTS)
            .await
            .unwrap()
            .expect("an expired lease makes the job claimable again");
        assert_eq!(lease_b.job_id, job_id);
        assert!(
            lease_b.fencing_token > lease_a.fencing_token,
            "the re-claim still bumps the fencing token"
        );

        assert_eq!(
            attempt_row_counts(&backend, &job_id),
            (0, 0),
            "the re-claim wipes the previous attempt's progress and file rows"
        );
        let view = backend
            .get_export_job_for_worker(&tenant, &job_id, &worker_b, lease_b.fencing_token)
            .await
            .unwrap();
        assert!(
            view.type_progress.is_empty(),
            "the new attempt resumes from nothing, not from a cursor whose parts are gone"
        );
        assert!(
            backend
                .get_export_manifest(&tenant, &job_id)
                .await
                .unwrap()
                .output
                .is_empty(),
            "no part of the abandoned attempt survives into the manifest"
        );
    }

    /// The wipe is only for re-claims: an `accepted` job never wrote anything,
    /// and its first claim goes through the same code path untouched.
    #[tokio::test]
    async fn test_first_claim_has_nothing_to_discard() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let job_id = backend
            .start_export(&tenant, test_input(ExportRequest::system()))
            .await
            .unwrap();
        assert_eq!(attempt_row_counts(&backend, &job_id), (0, 0));

        let worker = WorkerId::new("worker-1");
        let lease = backend
            .claim_next(&worker, StdDuration::from_secs(60), TEST_MAX_ATTEMPTS)
            .await
            .unwrap()
            .expect("an accepted job is claimable");
        assert_eq!(lease.job_id, job_id);
        assert_eq!(lease.fencing_token, 1);
        assert_eq!(attempts_of(&backend, &job_id), 1);

        // Rows written under the fresh lease stay put — the wipe runs before
        // them, not after.
        record_type_progress(
            &backend,
            &tenant,
            &job_id,
            &worker,
            lease.fencing_token,
            TypeExportProgress::new("Patient"),
        )
        .await;
        record_output_part(
            &backend,
            &tenant,
            &job_id,
            &worker,
            lease.fencing_token,
            "Patient",
            0,
            10,
        )
        .await;
        assert_eq!(attempt_row_counts(&backend, &job_id), (1, 1));

        backend
            .finish_export_job(&tenant, &job_id, &worker, lease.fencing_token)
            .await
            .unwrap();
        let manifest = backend.get_export_manifest(&tenant, &job_id).await.unwrap();
        assert_eq!(manifest.output.len(), 1);
    }

    /// The reason the wipe exists. The worker resumes *within* a type from
    /// `cursor_state` but always restarts `part_index` at 0, and
    /// `record_export_file` upserts on `(job, file_type, resource_type,
    /// part_index)`. Keeping the first attempt's rows would therefore let the
    /// second attempt's parts overwrite them row by row: the manifest would
    /// list attempt 2's post-cursor parts under attempt 1's indexes, the
    /// pre-cursor resources would vanish, and the job would still end
    /// `complete` — silent data loss (#1041). After the wipe a manifest can
    /// only ever describe one attempt.
    #[tokio::test]
    async fn test_reclaim_cannot_mix_parts_from_two_attempts() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let job_id = backend
            .start_export(&tenant, test_input(ExportRequest::system()))
            .await
            .unwrap();

        // Attempt 1: Observation runs to the end, Patient stops mid-type with
        // two parts written and a cursor pointing past them.
        let worker_a = WorkerId::new("worker-a");
        let lease_a = backend
            .claim_next(&worker_a, StdDuration::from_millis(1), TEST_MAX_ATTEMPTS)
            .await
            .unwrap()
            .unwrap();
        backend
            .mark_export_in_progress(&tenant, &job_id, &worker_a, lease_a.fencing_token)
            .await
            .unwrap();
        let mut observation = TypeExportProgress::new("Observation");
        observation.exported_count = 50;
        record_type_progress(
            &backend,
            &tenant,
            &job_id,
            &worker_a,
            lease_a.fencing_token,
            observation,
        )
        .await;
        record_output_part(
            &backend,
            &tenant,
            &job_id,
            &worker_a,
            lease_a.fencing_token,
            "Observation",
            0,
            50,
        )
        .await;
        let mut patient = TypeExportProgress::new("Patient");
        patient.exported_count = 200;
        patient.cursor_state = Some("after-patient-200".to_string());
        record_type_progress(
            &backend,
            &tenant,
            &job_id,
            &worker_a,
            lease_a.fencing_token,
            patient,
        )
        .await;
        for part_index in 0..2 {
            record_output_part(
                &backend,
                &tenant,
                &job_id,
                &worker_a,
                lease_a.fencing_token,
                "Patient",
                part_index,
                100,
            )
            .await;
        }

        // Worker A dies; worker B re-claims the job.
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let worker_b = WorkerId::new("worker-b");
        let lease_b = backend
            .claim_next(&worker_b, StdDuration::from_secs(60), TEST_MAX_ATTEMPTS)
            .await
            .unwrap()
            .unwrap();

        // Nothing of attempt 1 is left for attempt 2's part 0 to overwrite.
        assert_eq!(
            attempt_row_counts(&backend, &job_id),
            (0, 0),
            "attempt 2 must not inherit attempt 1's rows"
        );
        let view = backend
            .get_export_job_for_worker(&tenant, &job_id, &worker_b, lease_b.fencing_token)
            .await
            .unwrap();
        assert!(
            view.type_progress.is_empty(),
            "no surviving cursor, so attempt 2 re-exports Patient from the start"
        );

        // Attempt 2 re-exports both types from scratch and finishes.
        record_output_part(
            &backend,
            &tenant,
            &job_id,
            &worker_b,
            lease_b.fencing_token,
            "Patient",
            0,
            300,
        )
        .await;
        record_output_part(
            &backend,
            &tenant,
            &job_id,
            &worker_b,
            lease_b.fencing_token,
            "Observation",
            0,
            50,
        )
        .await;
        backend
            .finish_export_job(&tenant, &job_id, &worker_b, lease_b.fencing_token)
            .await
            .unwrap();

        let manifest = backend.get_export_manifest(&tenant, &job_id).await.unwrap();
        assert_eq!(manifest.status, ExportStatus::Complete);
        assert_eq!(
            manifest.output.len(),
            2,
            "one part per type, all from attempt 2"
        );
        assert!(
            manifest
                .output
                .iter()
                .all(|entry| entry.key.fencing_token == lease_b.fencing_token),
            "every manifest entry belongs to the attempt that finished the job"
        );
        let patient_entry = manifest
            .output
            .iter()
            .find(|entry| entry.resource_type == "Patient")
            .expect("Patient part present");
        assert_eq!(
            patient_entry.count, 300,
            "the manifest reports attempt 2's whole Patient export, not a post-cursor remainder \
             sitting on top of attempt 1's rows"
        );
    }

    /// The wipe is scoped to the job being re-claimed: another job of the same
    /// tenant keeps its progress and file rows, however the DELETEs are
    /// written.
    #[tokio::test]
    async fn test_reclaim_leaves_other_jobs_rows_alone() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // `bystander` is created (and so claimed) first: the claim scan orders
        // by `created_at`, and its long lease keeps it out of the later scan.
        let bystander = backend
            .start_export(&tenant, test_input(ExportRequest::system()))
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let reclaimed = backend
            .start_export(&tenant, test_input(ExportRequest::system()))
            .await
            .unwrap();

        let worker_a = WorkerId::new("worker-a");
        let lease_bystander = backend
            .claim_next(&worker_a, StdDuration::from_secs(60), TEST_MAX_ATTEMPTS)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(lease_bystander.job_id, bystander);
        record_type_progress(
            &backend,
            &tenant,
            &bystander,
            &worker_a,
            lease_bystander.fencing_token,
            TypeExportProgress::new("Patient"),
        )
        .await;
        record_output_part(
            &backend,
            &tenant,
            &bystander,
            &worker_a,
            lease_bystander.fencing_token,
            "Patient",
            0,
            7,
        )
        .await;

        let worker_b = WorkerId::new("worker-b");
        let lease_b = backend
            .claim_next(&worker_b, StdDuration::from_millis(1), TEST_MAX_ATTEMPTS)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(lease_b.job_id, reclaimed);
        record_type_progress(
            &backend,
            &tenant,
            &reclaimed,
            &worker_b,
            lease_b.fencing_token,
            TypeExportProgress::new("Patient"),
        )
        .await;
        record_output_part(
            &backend,
            &tenant,
            &reclaimed,
            &worker_b,
            lease_b.fencing_token,
            "Patient",
            0,
            9,
        )
        .await;

        // Only `reclaimed` has an expired lease, so only its rows go.
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let worker_c = WorkerId::new("worker-c");
        let lease_c = backend
            .claim_next(&worker_c, StdDuration::from_secs(60), TEST_MAX_ATTEMPTS)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(lease_c.job_id, reclaimed);

        assert_eq!(attempt_row_counts(&backend, &reclaimed), (0, 0));
        assert_eq!(
            attempt_row_counts(&backend, &bystander),
            (1, 1),
            "a concurrent job's rows are not collateral damage"
        );
        let bystander_manifest = backend
            .get_export_manifest(&tenant, &bystander)
            .await
            .unwrap();
        assert_eq!(bystander_manifest.output.len(), 1);
        assert_eq!(bystander_manifest.output[0].count, 7);
    }

    /// Reads a job's raw claim counter, which no trait surfaces.
    fn attempts_of(backend: &SqliteBackend, job_id: &ExportJobId) -> i64 {
        backend
            .get_connection()
            .unwrap()
            .query_row(
                "SELECT attempts FROM bulk_export_jobs WHERE id = ?1",
                params![job_id.as_str()],
                |row| row.get(0),
            )
            .unwrap()
    }

    /// A job whose lease expires mid-run is reclaimable, so one that keeps
    /// dying the same way used to be handed to worker after worker forever:
    /// never terminal, `error_message` never set, the status poll answering
    /// `202` indefinitely, and one of the tenant's export slots held the whole
    /// time (#1041). The claim cap retires it instead.
    #[tokio::test]
    async fn test_claim_cap_retires_a_job_that_never_finishes() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let job_id = backend
            .start_export(&tenant, test_input(ExportRequest::system()))
            .await
            .unwrap();

        // Two claims, each losing its lease before finishing.
        for attempt in 1..=2 {
            let worker = WorkerId::new(format!("worker-{attempt}"));
            let lease = backend
                .claim_next(&worker, StdDuration::from_millis(1), 2)
                .await
                .unwrap()
                .expect("claimable while attempts remain");
            assert_eq!(lease.fencing_token, attempt as u64, "fencing still bumps");
            assert_eq!(attempts_of(&backend, &job_id), attempt, "attempts counted");
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }

        // The third claim would exceed the cap, so the job is retired rather
        // than handed out again.
        let worker = WorkerId::new("worker-3");
        assert!(
            backend
                .claim_next(&worker, StdDuration::from_secs(60), 2)
                .await
                .unwrap()
                .is_none(),
            "a job past its attempt cap must not be claimable"
        );

        let progress = backend.get_export_status(&tenant, &job_id).await.unwrap();
        assert_eq!(progress.status, ExportStatus::Error);
        assert_eq!(progress.error_message, Some(abandoned_export_message(2)));
        assert!(
            progress.completed_at.is_some(),
            "a retired job is terminal, so it has a completion time"
        );

        // And it stays retired: a later claim does not resurrect it.
        assert!(
            backend
                .claim_next(&worker, StdDuration::from_secs(60), 2)
                .await
                .unwrap()
                .is_none()
        );
    }

    /// Retiring a job is not the end of the scan — the claim that spends the
    /// last attempt still hands back the next eligible job, so one stuck job
    /// cannot stall a worker that has other work waiting.
    #[tokio::test]
    async fn test_claim_cap_still_returns_the_next_eligible_job() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let stuck = backend
            .start_export(&tenant, test_input(ExportRequest::system()))
            .await
            .unwrap();
        // The claim scan orders by `created_at`; a beat between the two
        // inserts keeps `stuck` ahead of `fresh` on a coarse system clock.
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let fresh = backend
            .start_export(&tenant, test_input(ExportRequest::system()))
            .await
            .unwrap();

        let worker = WorkerId::new("worker-1");
        let lease = backend
            .claim_next(&worker, StdDuration::from_millis(1), 1)
            .await
            .unwrap()
            .expect("the older job is claimed first");
        assert_eq!(lease.job_id, stuck);
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;

        let lease = backend
            .claim_next(&worker, StdDuration::from_secs(60), 1)
            .await
            .unwrap()
            .expect("the scan must go past the retired job, not stop at it");
        assert_eq!(lease.job_id, fresh);

        assert_eq!(
            backend
                .get_export_status(&tenant, &stuck)
                .await
                .unwrap()
                .status,
            ExportStatus::Error
        );
        assert_eq!(
            backend
                .get_export_status(&tenant, &fresh)
                .await
                .unwrap()
                .status,
            ExportStatus::InProgress
        );
    }

    /// The cap only ever sees jobs that come back for another claim: a job
    /// that runs to completion on its first attempt is untouched by it.
    #[tokio::test]
    async fn test_claim_cap_leaves_a_completed_job_alone() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let job_id = backend
            .start_export(&tenant, test_input(ExportRequest::system()))
            .await
            .unwrap();

        let worker = WorkerId::new("worker-1");
        let lease = backend
            .claim_next(&worker, StdDuration::from_secs(60), 1)
            .await
            .unwrap()
            .expect("job claimable");
        backend
            .finish_export_job(&tenant, &job_id, &worker, lease.fencing_token)
            .await
            .unwrap();

        let progress = backend.get_export_status(&tenant, &job_id).await.unwrap();
        assert_eq!(progress.status, ExportStatus::Complete);
        assert_eq!(progress.error_message, None);
        assert_eq!(attempts_of(&backend, &job_id), 1);

        // A complete job is not eligible, so no later scan can retire it.
        assert!(
            backend
                .claim_next(&worker, StdDuration::from_secs(60), 1)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            backend
                .get_export_status(&tenant, &job_id)
                .await
                .unwrap()
                .status,
            ExportStatus::Complete
        );
    }

    #[tokio::test]
    async fn test_set_export_current_type_persists_and_clears() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let job_id = backend
            .start_export(&tenant, test_input(ExportRequest::system()))
            .await
            .unwrap();
        let worker = WorkerId::new("worker-1");
        let lease = backend
            .claim_next(&worker, StdDuration::from_secs(60), TEST_MAX_ATTEMPTS)
            .await
            .unwrap()
            .expect("job claimable");

        backend
            .set_export_current_type(
                &tenant,
                &job_id,
                &worker,
                lease.fencing_token,
                Some("Patient"),
                1,
                3,
            )
            .await
            .unwrap();

        let progress = backend.get_export_status(&tenant, &job_id).await.unwrap();
        assert_eq!(progress.current_type, Some("Patient".to_string()));
        assert_eq!(progress.types_done, 1);
        assert_eq!(progress.types_total, 3);

        // The terminal update clears the marker but keeps the counters — they
        // describe how much of the job ran, which stays true after it ends.
        backend
            .finish_export_job(&tenant, &job_id, &worker, lease.fencing_token)
            .await
            .unwrap();
        let progress = backend.get_export_status(&tenant, &job_id).await.unwrap();
        assert_eq!(progress.current_type, None);
        assert_eq!(progress.types_done, 1);
        assert_eq!(progress.types_total, 3);
    }

    #[tokio::test]
    async fn test_set_export_current_type_is_fenced() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let job_id = backend
            .start_export(&tenant, test_input(ExportRequest::system()))
            .await
            .unwrap();
        let worker = WorkerId::new("worker-1");
        let lease = backend
            .claim_next(&worker, StdDuration::from_secs(60), TEST_MAX_ATTEMPTS)
            .await
            .unwrap()
            .expect("job claimable");

        let stale_token = lease.fencing_token + 1;
        assert!(matches!(
            backend
                .set_export_current_type(
                    &tenant,
                    &job_id,
                    &worker,
                    stale_token,
                    Some("Patient"),
                    1,
                    3,
                )
                .await,
            Err(LeaseError::LeaseLost { .. })
        ));

        // The status is unchanged by the rejected mutation.
        let progress = backend.get_export_status(&tenant, &job_id).await.unwrap();
        assert_eq!(progress.current_type, None);
        assert_eq!(progress.types_done, 0);
        assert_eq!(progress.types_total, 0);
    }

    #[tokio::test]
    async fn test_fail_export_job_clears_current_type() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let job_id = backend
            .start_export(&tenant, test_input(ExportRequest::system()))
            .await
            .unwrap();
        let worker = WorkerId::new("worker-1");
        let lease = backend
            .claim_next(&worker, StdDuration::from_secs(60), TEST_MAX_ATTEMPTS)
            .await
            .unwrap()
            .expect("job claimable");

        backend
            .set_export_current_type(
                &tenant,
                &job_id,
                &worker,
                lease.fencing_token,
                Some("Patient"),
                0,
                1,
            )
            .await
            .unwrap();
        backend
            .fail_export_job(&tenant, &job_id, &worker, lease.fencing_token, "boom")
            .await
            .unwrap();

        let progress = backend.get_export_status(&tenant, &job_id).await.unwrap();
        assert_eq!(progress.current_type, None);
        assert_eq!(progress.error_message, Some("boom".to_string()));
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
    async fn test_patient_compartment_uses_resource_payload_not_search_index() {
        // Regression: when search is offloaded (sqlite-elasticsearch), the local
        // search_index is empty, so compartment lookups must read the resource
        // payload directly. Here we force-offload to guarantee no search_index
        // rows exist, then confirm the Observation is still found via its
        // subject.reference.
        let mut backend = SqliteBackend::in_memory().unwrap();
        backend.init_schema().unwrap();
        backend.set_search_offloaded(true);
        let tenant = create_test_tenant();

        backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "id": "p1"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create(
                &tenant,
                "Observation",
                json!({
                    "resourceType": "Observation", "id": "o1", "status": "final",
                    "subject": {"reference": "Patient/p1"}
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let request = ExportRequest::patient();
        let batch = backend
            .fetch_patient_compartment_batch(
                &tenant,
                &request,
                "Observation",
                &["p1".to_string()],
                None,
                100,
            )
            .await
            .unwrap();
        assert_eq!(
            batch.lines.len(),
            1,
            "Observation should be found via subject.reference"
        );
        assert!(batch.lines[0].contains("\"o1\""));
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

    /// Exported lines carry the server `meta.versionId` / `meta.lastUpdated`
    /// from the row's columns, on every fetch path, while client `meta`
    /// members survive (#1273). The stored blob holds neither field, so before
    /// this fix they never reached the NDJSON output.
    #[tokio::test]
    async fn exported_lines_carry_server_meta() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let tag = json!([{"system": "http://example.org/tags", "code": "keep-me"}]);
        let created = backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "id": "p1", "meta": {"tag": tag.clone()}}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        // A second version, so `versionId` is not the trivial "1".
        backend
            .update(
                &tenant,
                &created,
                json!({"resourceType": "Patient", "id": "p1", "meta": {"tag": tag.clone()}, "active": true}),
            )
            .await
            .unwrap();
        pin_last_updated(&backend, "p1", "2026-03-04T05:06:07.123456+02:00");

        backend
            .create(
                &tenant,
                "Observation",
                json!({"resourceType": "Observation", "id": "o1", "status": "final",
                       "code": {"text": "x"}, "subject": {"reference": "Patient/p1"}}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        pin_last_updated(&backend, "o1", "2026-03-05T00:00:00+00:00");

        let parse = |line: &str| -> Value { serde_json::from_str(line).unwrap() };
        let ids = vec!["p1".to_string()];

        let system = backend
            .fetch_export_batch(&tenant, &ExportRequest::system(), "Patient", None, 10)
            .await
            .unwrap();
        let compartment_patient = backend
            .fetch_patient_compartment_batch(
                &tenant,
                &ExportRequest::patient(),
                "Patient",
                &ids,
                None,
                10,
            )
            .await
            .unwrap();
        for batch in [&system, &compartment_patient] {
            assert_eq!(batch.lines.len(), 1);
            let meta = &parse(&batch.lines[0])["meta"];
            assert_eq!(meta["versionId"], "2");
            assert_eq!(meta["lastUpdated"], "2026-03-04T03:06:07.123Z");
            assert_eq!(meta["tag"], tag, "client meta members are preserved");
        }

        let compartment_other = backend
            .fetch_patient_compartment_batch(
                &tenant,
                &ExportRequest::patient(),
                "Observation",
                &ids,
                None,
                10,
            )
            .await
            .unwrap();
        assert_eq!(compartment_other.lines.len(), 1);
        let meta = &parse(&compartment_other.lines[0])["meta"];
        assert_eq!(meta["versionId"], "1");
        assert_eq!(meta["lastUpdated"], "2026-03-05T00:00:00.000Z");
    }

    /// Pins a stored resource's `last_updated` so a window test does not depend
    /// on wall-clock timing.
    fn pin_last_updated(backend: &SqliteBackend, id: &str, rfc3339: &str) {
        let conn = backend.get_connection().unwrap();
        conn.execute(
            "UPDATE resources SET last_updated = ?1 WHERE id = ?2",
            rusqlite::params![rfc3339, id],
        )
        .unwrap();
    }

    async fn seed_patient_at(backend: &SqliteBackend, tenant: &TenantContext, at: &str) -> String {
        let stored = backend
            .create(
                tenant,
                "Patient",
                json!({"resourceType": "Patient"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        let id = stored.id().to_string();
        pin_last_updated(backend, &id, at);
        id
    }

    fn instant(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    /// The `Patient` branch of the compartment fetch applies `_since`, the way
    /// the non-Patient branch below it always has. Before this fix a group
    /// export with `_since` emitted every member, however stale.
    #[tokio::test]
    async fn since_bounds_the_patient_branch_of_the_compartment_fetch() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let stale = seed_patient_at(&backend, &tenant, "2026-01-01T00:00:00+00:00").await;
        let fresh = seed_patient_at(&backend, &tenant, "2026-07-01T00:00:00+00:00").await;

        // Both are named explicitly, as a group export or an explicit `patient`
        // parameter would: the id list reaching this method is NOT pre-filtered.
        let ids = vec![stale.clone(), fresh.clone()];
        let request = ExportRequest::patient().with_since(instant("2026-06-01T00:00:00Z"));

        let batch = backend
            .fetch_patient_compartment_batch(&tenant, &request, "Patient", &ids, None, 10)
            .await
            .unwrap();

        assert_eq!(
            batch.lines.len(),
            1,
            "the stale patient must be filtered out"
        );
        assert!(
            batch.lines[0].contains(&fresh),
            "only the patient modified inside the window survives"
        );
    }

    /// `_since` is inclusive here too, matching every other bound.
    #[tokio::test]
    async fn since_is_inclusive_in_the_patient_branch() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let on_bound = seed_patient_at(&backend, &tenant, "2026-06-01T00:00:00+00:00").await;
        let ids = vec![on_bound.clone()];
        let request = ExportRequest::patient().with_since(instant("2026-06-01T00:00:00Z"));

        let batch = backend
            .fetch_patient_compartment_batch(&tenant, &request, "Patient", &ids, None, 10)
            .await
            .unwrap();

        assert_eq!(
            batch.lines.len(),
            1,
            "a patient exactly on the bound is included"
        );
    }

    /// The new bound and the keyset cursor coexist: paging a filtered set
    /// neither drops nor repeats a row. This is the case the numbered
    /// placeholders in this branch could break — the bound binds after the id
    /// list and before the cursor's own two.
    #[tokio::test]
    async fn since_and_cursor_page_the_patient_branch_without_loss() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let _stale = seed_patient_at(&backend, &tenant, "2026-01-01T00:00:00+00:00").await;
        let a = seed_patient_at(&backend, &tenant, "2026-07-01T00:00:00+00:00").await;
        let b = seed_patient_at(&backend, &tenant, "2026-07-02T00:00:00+00:00").await;
        let c = seed_patient_at(&backend, &tenant, "2026-07-03T00:00:00+00:00").await;

        let ids = vec![_stale.clone(), a.clone(), b.clone(), c.clone()];
        let request = ExportRequest::patient().with_since(instant("2026-06-01T00:00:00Z"));

        let first = backend
            .fetch_patient_compartment_batch(&tenant, &request, "Patient", &ids, None, 2)
            .await
            .unwrap();
        assert_eq!(first.lines.len(), 2);
        assert!(!first.is_last);

        let second = backend
            .fetch_patient_compartment_batch(
                &tenant,
                &request,
                "Patient",
                &ids,
                first.next_cursor.as_deref(),
                2,
            )
            .await
            .unwrap();

        let mut seen: Vec<String> = Vec::new();
        for line in first.lines.iter().chain(second.lines.iter()) {
            for id in [&a, &b, &c] {
                if line.contains(id.as_str()) {
                    seen.push(id.clone());
                }
            }
        }
        seen.sort();
        seen.dedup();
        assert_eq!(
            seen.len(),
            3,
            "all three in-window patients appear exactly once"
        );
        assert!(
            !first
                .lines
                .iter()
                .chain(second.lines.iter())
                .any(|l| l.contains(_stale.as_str())),
            "the stale patient never appears on any page"
        );
    }

    /// `_until` excludes a resource modified after the bound — and the count
    /// agrees with what the fetch emits, so a job's total cannot promise rows
    /// the export never writes.
    #[tokio::test]
    async fn until_excludes_resources_modified_after_the_bound() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let early = seed_patient_at(&backend, &tenant, "2026-01-01T00:00:00+00:00").await;
        let _late = seed_patient_at(&backend, &tenant, "2026-03-01T00:00:00+00:00").await;

        let request = ExportRequest::system().with_until(instant("2026-02-01T00:00:00Z"));

        let count = backend
            .count_export_resources(&tenant, &request, "Patient")
            .await
            .unwrap();
        let batch = backend
            .fetch_export_batch(&tenant, &request, "Patient", None, 10)
            .await
            .unwrap();

        assert_eq!(count, 1, "count must apply the upper bound");
        assert_eq!(batch.lines.len(), 1, "fetch must apply the upper bound");
        assert_eq!(
            count as usize,
            batch.lines.len(),
            "count and fetch must agree"
        );
        assert!(
            batch.lines[0].contains(&early),
            "the surviving row is the early one"
        );
    }

    /// The bound is inclusive, matching S3's `last_modified() > until` skip: a
    /// resource sitting exactly on `_until` is exported.
    #[tokio::test]
    async fn until_is_inclusive() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        seed_patient_at(&backend, &tenant, "2026-02-01T00:00:00+00:00").await;

        let request = ExportRequest::system().with_until(instant("2026-02-01T00:00:00Z"));

        let batch = backend
            .fetch_export_batch(&tenant, &request, "Patient", None, 10)
            .await
            .unwrap();
        assert_eq!(
            batch.lines.len(),
            1,
            "a resource exactly on the bound is included"
        );
    }

    /// `_since` and `_until` together produce a bounded window: rows on either
    /// side are dropped and only the middle one survives.
    #[tokio::test]
    async fn since_and_until_together_bound_the_window() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let _before = seed_patient_at(&backend, &tenant, "2025-12-01T00:00:00+00:00").await;
        let inside = seed_patient_at(&backend, &tenant, "2026-01-15T00:00:00+00:00").await;
        let _after = seed_patient_at(&backend, &tenant, "2026-03-01T00:00:00+00:00").await;

        let request = ExportRequest::system()
            .with_since(instant("2026-01-01T00:00:00Z"))
            .with_until(instant("2026-02-01T00:00:00Z"));

        let count = backend
            .count_export_resources(&tenant, &request, "Patient")
            .await
            .unwrap();
        let batch = backend
            .fetch_export_batch(&tenant, &request, "Patient", None, 10)
            .await
            .unwrap();

        assert_eq!(count, 1);
        assert_eq!(batch.lines.len(), 1);
        assert!(
            batch.lines[0].contains(&inside),
            "only the in-window row survives"
        );
    }

    /// An unbounded request is unchanged by the window plumbing — the bound is
    /// opt-in, so nothing regresses for callers that pass neither parameter.
    #[tokio::test]
    async fn no_bounds_exports_everything() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        seed_patient_at(&backend, &tenant, "2025-12-01T00:00:00+00:00").await;
        seed_patient_at(&backend, &tenant, "2026-03-01T00:00:00+00:00").await;

        let request = ExportRequest::system();
        let count = backend
            .count_export_resources(&tenant, &request, "Patient")
            .await
            .unwrap();
        assert_eq!(count, 2);
    }

    /// The patient-compartment path applies the bound too. Exercised through
    /// the `Patient` branch, which builds its own query separate from
    /// `fetch_export_batch`.
    #[tokio::test]
    async fn until_bounds_the_patient_compartment_fetch() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let early = seed_patient_at(&backend, &tenant, "2026-01-01T00:00:00+00:00").await;
        let late = seed_patient_at(&backend, &tenant, "2026-03-01T00:00:00+00:00").await;

        let request = ExportRequest::patient().with_until(instant("2026-02-01T00:00:00Z"));
        let ids = vec![early.clone(), late.clone()];

        let batch = backend
            .fetch_patient_compartment_batch(&tenant, &request, "Patient", &ids, None, 10)
            .await
            .unwrap();

        assert_eq!(
            batch.lines.len(),
            1,
            "the compartment fetch applies the upper bound"
        );
        assert!(batch.lines[0].contains(&early));
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

    /// #1185: the kick-off is one small insert, and a search-index rebuild
    /// holding SQLite's single write lock makes it wait out `busy_timeout`
    /// and fail with `SQLITE_BUSY`. That is a transient condition worth
    /// retrying, so it has to reach REST as `Unavailable` — which renders as
    /// 503 with `Retry-After`. Flattened into `Internal` it answered 500, and
    /// nothing retried a database that was merely busy.
    #[tokio::test]
    async fn a_busy_database_makes_the_kickoff_retryable_not_a_fault() {
        use crate::backends::sqlite::SqliteBackendConfig;

        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("kickoff-busy.db");
        let backend = SqliteBackend::with_config(
            &db_path,
            SqliteBackendConfig {
                max_connections: 2,
                busy_timeout_ms: 20,
                ..Default::default()
            },
        )
        .unwrap();
        backend.init_schema().unwrap();
        let tenant = create_test_tenant();

        // A raw connection outside the pool stands in for the rebuild: the
        // lock that matters is SQLite's file-level write lock, not a pool
        // slot, so `BEGIN IMMEDIATE` reproduces the contention exactly.
        let rebuild = rusqlite::Connection::open(&db_path).unwrap();
        rebuild.execute_batch("BEGIN IMMEDIATE;").unwrap();

        let err = backend
            .start_export(&tenant, test_input(ExportRequest::system()))
            .await
            .expect_err("the insert cannot land while the write lock is held");
        rebuild.execute_batch("ROLLBACK;").unwrap();

        match err {
            StorageError::Backend(BackendError::Unavailable { message, .. }) => {
                assert!(
                    message.contains("Failed to create export job"),
                    "the context must survive classification: {message}"
                );
            }
            other => panic!("a busy database must stay retryable, got {other:?}"),
        }
    }

    /// An idle poll must not queue behind a long writer. `claim_next` scans
    /// inside an IMMEDIATE transaction, which grabs SQLite's write lock as it
    /// begins, so a search-index rebuild used to park every empty poll for
    /// seconds — and past `busy_timeout` it came back `Unavailable`, which the
    /// worker loop logs as an error and backs off 5s from. The lock-free
    /// eligibility probe answers an empty queue as a WAL read, which no writer
    /// blocks.
    #[tokio::test]
    async fn an_idle_claim_does_not_wait_behind_a_writer() {
        use crate::backends::sqlite::SqliteBackendConfig;

        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("idle-claim.db");
        let backend = SqliteBackend::with_config(
            &db_path,
            SqliteBackendConfig {
                max_connections: 2,
                // The production default: the point is that the poll returns
                // long before it, not that the timeout is short.
                busy_timeout_ms: 30_000,
                ..Default::default()
            },
        )
        .unwrap();
        backend.init_schema().unwrap();

        // Same stand-in for the rebuild as the kick-off test above: what
        // matters is SQLite's file-level write lock, not a pool slot.
        let rebuild = rusqlite::Connection::open(&db_path).unwrap();
        rebuild.execute_batch("BEGIN IMMEDIATE;").unwrap();

        let worker = WorkerId::new("worker-idle");
        let started = std::time::Instant::now();
        let claimed = backend
            .claim_next(&worker, StdDuration::from_secs(60), TEST_MAX_ATTEMPTS)
            .await;
        let elapsed = started.elapsed();
        rebuild.execute_batch("ROLLBACK;").unwrap();

        assert!(
            matches!(&claimed, Ok(None)),
            "an empty queue reads as empty even while a writer holds the lock: {claimed:?}"
        );
        assert!(
            elapsed < StdDuration::from_secs(5),
            "the idle poll waited {elapsed:?} on the write lock"
        );
    }

    /// A row the driver cannot hand over must fail the read, not quietly drop
    /// out of it. `query_map` binds the parameters and nothing more — the
    /// first `sqlite3_step` happens on the per-row `Result` — so the old
    /// `filter_map(|r| r.ok())` turned a failed read into an empty one, and
    /// `$export-status` answered 200 with no progress instead of saying it
    /// could not read it (#1185).
    #[tokio::test]
    async fn an_unreadable_progress_row_fails_the_status_read() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();
        let job_id = backend
            .start_export(&tenant, test_input(ExportRequest::system()))
            .await
            .unwrap();

        // `exported_count` has INTEGER affinity, so a non-numeric string is
        // stored as TEXT and reading it back as an `i64` fails the same way a
        // driver error on that row would.
        backend
            .get_connection()
            .unwrap()
            .execute(
                "INSERT INTO bulk_export_progress
                 (job_id, resource_type, total_count, exported_count, error_count)
                 VALUES (?1, 'Patient', 1, 'not-a-number', 0)",
                params![job_id.as_str()],
            )
            .unwrap();

        let err = backend
            .get_export_status(&tenant, &job_id)
            .await
            .expect_err("an unreadable row must not read as an absent one");
        assert!(
            matches!(err, StorageError::Backend(_)),
            "unexpected error: {err:?}"
        );
    }
}
