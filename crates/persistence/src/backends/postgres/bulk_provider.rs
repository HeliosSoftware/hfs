//! PostgreSQL implementation of the provider-side [`BulkProviderStore`].
//!
//! One row per `(tenant_id, id)` in `bulk_provider_submissions`, holding the
//! opaque submission document as JSONB plus a monotonic `version` for
//! optimistic locking. Writes run a `SELECT … FOR UPDATE` read-modify-write
//! inside a transaction, mirroring the per-user settings store (#772).

use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;

use crate::core::bulk_provider::{BulkProviderStore, StoredProviderSubmission};
use crate::error::{BackendError, ConcurrencyError, StorageError, StorageResult};
use crate::tenant::TenantContext;

use super::PostgresBackend;

fn backend_err(message: String) -> StorageError {
    StorageError::Backend(BackendError::Internal {
        backend_name: "postgres".to_string(),
        message,
        source: None,
    })
}

fn lock_failure(id: &str, expected: i64, actual: i64) -> StorageError {
    StorageError::Concurrency(ConcurrencyError::OptimisticLockFailure {
        resource_type: "BulkProviderSubmission".to_string(),
        id: id.to_string(),
        expected_etag: format!("W/\"{expected}\""),
        actual_etag: Some(format!("W/\"{actual}\"")),
    })
}

#[async_trait]
impl BulkProviderStore for PostgresBackend {
    async fn list_provider_submissions(
        &self,
        tenant: &TenantContext,
    ) -> StorageResult<Vec<StoredProviderSubmission>> {
        let client = self.get_client().await?;
        let rows = client
            .query(
                "SELECT id, data, version, updated_at FROM bulk_provider_submissions
                 WHERE tenant_id = $1",
                &[&tenant.tenant_id().as_str()],
            )
            .await
            .map_err(|e| backend_err(format!("scan provider submissions: {e}")))?;
        Ok(rows
            .into_iter()
            .map(|row| StoredProviderSubmission {
                id: row.get(0),
                document: row.get(1),
                version: row.get(2),
                updated_at: row.get(3),
            })
            .collect())
    }

    async fn get_provider_submission(
        &self,
        tenant: &TenantContext,
        id: &str,
    ) -> StorageResult<Option<StoredProviderSubmission>> {
        let client = self.get_client().await?;
        let row = client
            .query_opt(
                "SELECT data, version, updated_at FROM bulk_provider_submissions
                 WHERE tenant_id = $1 AND id = $2",
                &[&tenant.tenant_id().as_str(), &id],
            )
            .await
            .map_err(|e| backend_err(format!("read provider submission: {e}")))?;
        Ok(row.map(|row| StoredProviderSubmission {
            id: id.to_string(),
            document: row.get(0),
            version: row.get(1),
            updated_at: row.get(2),
        }))
    }

    async fn put_provider_submission(
        &self,
        tenant: &TenantContext,
        id: &str,
        document: Value,
        if_match_version: Option<i64>,
    ) -> StorageResult<StoredProviderSubmission> {
        let mut client = self.get_client().await?;
        let txn = client
            .transaction()
            .await
            .map_err(|e| backend_err(format!("begin provider submission transaction: {e}")))?;

        let current_version: i64 = txn
            .query_opt(
                "SELECT version FROM bulk_provider_submissions
                 WHERE tenant_id = $1 AND id = $2 FOR UPDATE",
                &[&tenant.tenant_id().as_str(), &id],
            )
            .await
            .map_err(|e| backend_err(format!("read provider submission version: {e}")))?
            .map(|row| row.get(0))
            .unwrap_or(0);
        if let Some(expected) = if_match_version
            && expected != current_version
        {
            return Err(lock_failure(id, expected, current_version));
        }

        let new_version = current_version + 1;
        let now = Utc::now();
        txn.execute(
            "INSERT INTO bulk_provider_submissions (tenant_id, id, data, version, updated_at)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (tenant_id, id)
             DO UPDATE SET data = $3, version = $4, updated_at = $5",
            &[
                &tenant.tenant_id().as_str(),
                &id,
                &document,
                &new_version,
                &now,
            ],
        )
        .await
        .map_err(|e| backend_err(format!("write provider submission: {e}")))?;
        txn.commit()
            .await
            .map_err(|e| backend_err(format!("commit provider submission: {e}")))?;

        Ok(StoredProviderSubmission {
            id: id.to_string(),
            document,
            version: new_version,
            updated_at: now,
        })
    }

    async fn delete_provider_submission(
        &self,
        tenant: &TenantContext,
        id: &str,
    ) -> StorageResult<bool> {
        let client = self.get_client().await?;
        let affected = client
            .execute(
                "DELETE FROM bulk_provider_submissions WHERE tenant_id = $1 AND id = $2",
                &[&tenant.tenant_id().as_str(), &id],
            )
            .await
            .map_err(|e| backend_err(format!("delete provider submission: {e}")))?;
        Ok(affected > 0)
    }
}
