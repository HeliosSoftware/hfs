//! PostgreSQL-backed "needs reindex" ledger for failed secondary syncs (#1334).
//!
//! The PostgreSQL counterpart of the SQLite ledger: rows in
//! `secondary_sync_failures` (schema v41). See
//! [`crate::composite::sync_failures`].

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::composite::sync_failures::{
    SecondarySyncFailure, SecondarySyncFailureLedger, SyncFailureKey, SyncFailureReport,
    SyncOperation,
};
use crate::error::{BackendError, StorageError, StorageResult};

use super::PostgresBackend;

fn backend_err(message: String) -> StorageError {
    StorageError::Backend(BackendError::Internal {
        backend_name: "postgres".to_string(),
        message,
        source: None,
    })
}

#[async_trait]
impl SecondarySyncFailureLedger for PostgresBackend {
    async fn record_sync_failure(&self, report: &SyncFailureReport) -> StorageResult<bool> {
        let client = self.get_client().await?;
        let attempts = i64::from(report.attempts);
        let row = client
            .query_one(
                "INSERT INTO secondary_sync_failures
                    (tenant_id, resource_type, resource_id, backend_id, operation,
                     first_failed_at, last_failed_at, last_error, attempts)
                 VALUES ($1, $2, $3, $4, $5, $6, $6, $7, $8)
                 ON CONFLICT (tenant_id, resource_type, resource_id, backend_id)
                 DO UPDATE SET operation = EXCLUDED.operation,
                               last_failed_at = EXCLUDED.last_failed_at,
                               last_error = EXCLUDED.last_error,
                               attempts = secondary_sync_failures.attempts + EXCLUDED.attempts
                 RETURNING (xmax = 0) AS inserted",
                &[
                    &report.key.tenant_id,
                    &report.key.resource_type,
                    &report.key.resource_id,
                    &report.key.backend_id,
                    &report.operation.as_str(),
                    &report.failed_at,
                    &report.error,
                    &attempts,
                ],
            )
            .await
            .map_err(|e| backend_err(format!("record secondary sync failure: {e}")))?;
        Ok(row.get(0))
    }

    async fn clear_sync_failure(&self, key: &SyncFailureKey) -> StorageResult<bool> {
        let client = self.get_client().await?;
        let removed = client
            .execute(
                "DELETE FROM secondary_sync_failures
                 WHERE tenant_id = $1 AND resource_type = $2
                   AND resource_id = $3 AND backend_id = $4",
                &[
                    &key.tenant_id,
                    &key.resource_type,
                    &key.resource_id,
                    &key.backend_id,
                ],
            )
            .await
            .map_err(|e| backend_err(format!("clear secondary sync failure: {e}")))?;
        Ok(removed > 0)
    }

    async fn list_sync_failures(&self, limit: usize) -> StorageResult<Vec<SecondarySyncFailure>> {
        let client = self.get_client().await?;
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let rows = client
            .query(
                "SELECT tenant_id, resource_type, resource_id, backend_id, operation,
                        first_failed_at, last_failed_at, last_error, attempts
                 FROM secondary_sync_failures
                 ORDER BY last_failed_at, tenant_id, resource_type, resource_id, backend_id
                 LIMIT $1",
                &[&limit],
            )
            .await
            .map_err(|e| backend_err(format!("list secondary sync failures: {e}")))?;
        Ok(rows
            .into_iter()
            .map(|row| {
                let operation: String = row.get(4);
                let first_failed_at: DateTime<Utc> = row.get(5);
                let last_failed_at: DateTime<Utc> = row.get(6);
                let attempts: i64 = row.get(8);
                SecondarySyncFailure {
                    key: SyncFailureKey {
                        tenant_id: row.get(0),
                        resource_type: row.get(1),
                        resource_id: row.get(2),
                        backend_id: row.get(3),
                    },
                    operation: SyncOperation::from_stored(&operation),
                    first_failed_at,
                    last_failed_at,
                    last_error: row.get(7),
                    attempts: attempts.max(0) as u64,
                }
            })
            .collect())
    }

    async fn count_sync_failures(&self) -> StorageResult<u64> {
        let client = self.get_client().await?;
        let row = client
            .query_one("SELECT COUNT(*) FROM secondary_sync_failures", &[])
            .await
            .map_err(|e| backend_err(format!("count secondary sync failures: {e}")))?;
        let count: i64 = row.get(0);
        Ok(count.max(0) as u64)
    }
}
