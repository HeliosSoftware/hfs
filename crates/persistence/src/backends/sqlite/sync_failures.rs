//! SQLite-backed "needs reindex" ledger for failed secondary syncs (#1334).
//!
//! When SQLite is a composite's primary, a secondary that refuses a change
//! after retries is recorded in `secondary_sync_failures` (schema v34), next
//! to the resources it concerns, so the record survives a restart. See
//! [`crate::composite::sync_failures`].

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rusqlite::params;

use crate::composite::sync_failures::{
    SecondarySyncFailure, SecondarySyncFailureLedger, SyncFailureKey, SyncFailureReport,
    SyncOperation,
};
use crate::error::{BackendError, StorageError, StorageResult};

use super::SqliteBackend;

fn backend_err(message: String) -> StorageError {
    StorageError::Backend(BackendError::Internal {
        backend_name: "sqlite".to_string(),
        message,
        source: None,
    })
}

fn parse_time(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

#[async_trait]
impl SecondarySyncFailureLedger for SqliteBackend {
    async fn record_sync_failure(&self, report: &SyncFailureReport) -> StorageResult<bool> {
        let report = report.clone();
        self.run_blocking(move |conn| {
            // Timestamps are fixed-width RFC 3339 (UTC, microseconds), so
            // they sort as text.
            let failed_at = report
                .failed_at
                .to_rfc3339_opts(chrono::SecondsFormat::Micros, true);
            let total: i64 = conn
                .query_row(
                    "INSERT INTO secondary_sync_failures
                        (tenant_id, resource_type, resource_id, backend_id, operation,
                         first_failed_at, last_failed_at, last_error, attempts)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7, ?8)
                     ON CONFLICT (tenant_id, resource_type, resource_id, backend_id)
                     DO UPDATE SET operation = excluded.operation,
                                   last_failed_at = excluded.last_failed_at,
                                   last_error = excluded.last_error,
                                   attempts = attempts + excluded.attempts
                     RETURNING attempts",
                    params![
                        report.key.tenant_id,
                        report.key.resource_type,
                        report.key.resource_id,
                        report.key.backend_id,
                        report.operation.as_str(),
                        failed_at,
                        report.error,
                        i64::from(report.attempts),
                    ],
                    |row| row.get(0),
                )
                .map_err(|e| backend_err(format!("record secondary sync failure: {e}")))?;
            // A fresh row holds exactly this report's attempts; a folded one
            // holds more.
            Ok(total == i64::from(report.attempts))
        })
        .await
    }

    async fn clear_sync_failure(&self, key: &SyncFailureKey) -> StorageResult<bool> {
        let key = key.clone();
        self.run_blocking(move |conn| {
            let removed = conn
                .execute(
                    "DELETE FROM secondary_sync_failures
                     WHERE tenant_id = ?1 AND resource_type = ?2
                       AND resource_id = ?3 AND backend_id = ?4",
                    params![
                        key.tenant_id,
                        key.resource_type,
                        key.resource_id,
                        key.backend_id
                    ],
                )
                .map_err(|e| backend_err(format!("clear secondary sync failure: {e}")))?;
            Ok(removed > 0)
        })
        .await
    }

    async fn list_sync_failures(&self, limit: usize) -> StorageResult<Vec<SecondarySyncFailure>> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        self.run_blocking(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT tenant_id, resource_type, resource_id, backend_id, operation,
                            first_failed_at, last_failed_at, last_error, attempts
                     FROM secondary_sync_failures
                     ORDER BY last_failed_at, tenant_id, resource_type, resource_id, backend_id
                     LIMIT ?1",
                )
                .map_err(|e| backend_err(format!("list secondary sync failures: {e}")))?;
            let rows = stmt
                .query_map([limit], |row| {
                    let operation: String = row.get(4)?;
                    let first_failed_at: String = row.get(5)?;
                    let last_failed_at: String = row.get(6)?;
                    let attempts: i64 = row.get(8)?;
                    Ok(SecondarySyncFailure {
                        key: SyncFailureKey {
                            tenant_id: row.get(0)?,
                            resource_type: row.get(1)?,
                            resource_id: row.get(2)?,
                            backend_id: row.get(3)?,
                        },
                        operation: SyncOperation::from_stored(&operation),
                        first_failed_at: parse_time(&first_failed_at),
                        last_failed_at: parse_time(&last_failed_at),
                        last_error: row.get(7)?,
                        attempts: attempts.max(0) as u64,
                    })
                })
                .map_err(|e| backend_err(format!("list secondary sync failures: {e}")))?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| backend_err(format!("read secondary sync failure: {e}")))
        })
        .await
    }

    async fn count_sync_failures(&self) -> StorageResult<u64> {
        self.run_blocking(|conn| {
            let count: i64 = conn
                .query_row("SELECT COUNT(*) FROM secondary_sync_failures", [], |row| {
                    row.get(0)
                })
                .map_err(|e| backend_err(format!("count secondary sync failures: {e}")))?;
            Ok(count.max(0) as u64)
        })
        .await
    }
}
