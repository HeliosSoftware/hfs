//! SQLite implementation of the provider-side [`BulkProviderStore`].
//!
//! One row per `(tenant_id, id)` in `bulk_provider_submissions`, holding the
//! opaque submission document plus a monotonic `version` for optimistic
//! locking — the same shape as the per-user settings store, scoped to the
//! tenant instead of a user (#772).

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rusqlite::{OptionalExtension, params};
use serde_json::Value;

use crate::core::bulk_provider::{BulkProviderStore, StoredProviderSubmission};
use crate::error::{BackendError, ConcurrencyError, StorageError, StorageResult};
use crate::tenant::TenantContext;

use super::SqliteBackend;

fn backend_err(message: String) -> StorageError {
    StorageError::Backend(BackendError::Internal {
        backend_name: "sqlite".to_string(),
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

fn decode_row(
    id: String,
    data: Vec<u8>,
    version: i64,
    updated_at: String,
) -> StorageResult<StoredProviderSubmission> {
    let document: Value = serde_json::from_slice(&data)
        .map_err(|e| backend_err(format!("decode stored provider submission: {e}")))?;
    let updated_at = DateTime::parse_from_rfc3339(&updated_at)
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    Ok(StoredProviderSubmission {
        id,
        document,
        version,
        updated_at,
    })
}

#[async_trait]
impl BulkProviderStore for SqliteBackend {
    async fn list_provider_submissions(
        &self,
        tenant: &TenantContext,
    ) -> StorageResult<Vec<StoredProviderSubmission>> {
        let conn = self.get_connection()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, data, version, updated_at FROM bulk_provider_submissions
                 WHERE tenant_id = ?1",
            )
            .map_err(|e| backend_err(format!("scan provider submissions: {e}")))?;
        let rows = stmt
            .query_map([tenant.tenant_id().as_str()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(|e| backend_err(format!("scan provider submissions: {e}")))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| backend_err(format!("scan provider submissions: {e}")))?;
        rows.into_iter()
            .map(|(id, data, version, updated_at)| decode_row(id, data, version, updated_at))
            .collect()
    }

    async fn get_provider_submission(
        &self,
        tenant: &TenantContext,
        id: &str,
    ) -> StorageResult<Option<StoredProviderSubmission>> {
        let conn = self.get_connection()?;
        let row = conn
            .query_row(
                "SELECT data, version, updated_at FROM bulk_provider_submissions
                 WHERE tenant_id = ?1 AND id = ?2",
                params![tenant.tenant_id().as_str(), id],
                |row| {
                    Ok((
                        row.get::<_, Vec<u8>>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| backend_err(format!("read provider submission: {e}")))?;
        row.map(|(data, version, updated_at)| decode_row(id.to_string(), data, version, updated_at))
            .transpose()
    }

    async fn put_provider_submission(
        &self,
        tenant: &TenantContext,
        id: &str,
        document: Value,
        if_match_version: Option<i64>,
    ) -> StorageResult<StoredProviderSubmission> {
        let mut conn = self.get_connection()?;
        // IMMEDIATE, not the DEFERRED default: this transaction reads (the
        // version below) before it writes. Under WAL a deferred transaction
        // takes a read snapshot on that first read and then fails the
        // read-to-write upgrade with SQLITE_BUSY_SNAPSHOT if another
        // connection committed in between — and the busy handler is *not*
        // invoked for that code, so the configured busy_timeout does not
        // cover it. With a bulk ingest committing continuously, that race
        // lost essentially every time (#791). Taking the write lock up
        // front does.
        let txn = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| backend_err(format!("begin provider submission transaction: {e}")))?;

        let current_version: i64 = txn
            .query_row(
                "SELECT version FROM bulk_provider_submissions
                 WHERE tenant_id = ?1 AND id = ?2",
                params![tenant.tenant_id().as_str(), id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| backend_err(format!("read provider submission version: {e}")))?
            .unwrap_or(0);
        if let Some(expected) = if_match_version
            && expected != current_version
        {
            return Err(lock_failure(id, expected, current_version));
        }

        let new_version = current_version + 1;
        let now = Utc::now();
        let data = serde_json::to_vec(&document)
            .map_err(|e| backend_err(format!("encode provider submission: {e}")))?;
        txn.execute(
            "INSERT INTO bulk_provider_submissions (tenant_id, id, data, version, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(tenant_id, id) DO UPDATE SET data = ?3, version = ?4, updated_at = ?5",
            params![
                tenant.tenant_id().as_str(),
                id,
                data,
                new_version,
                now.to_rfc3339()
            ],
        )
        .map_err(|e| backend_err(format!("write provider submission: {e}")))?;
        txn.commit()
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
        let conn = self.get_connection()?;
        let affected = conn
            .execute(
                "DELETE FROM bulk_provider_submissions WHERE tenant_id = ?1 AND id = ?2",
                params![tenant.tenant_id().as_str(), id],
            )
            .map_err(|e| backend_err(format!("delete provider submission: {e}")))?;
        Ok(affected > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tenant::{TenantId, TenantPermissions};
    use serde_json::json;

    fn tenant(id: &str) -> TenantContext {
        TenantContext::new(TenantId::new(id), TenantPermissions::full_access())
    }

    fn backend() -> SqliteBackend {
        let backend = SqliteBackend::in_memory().expect("in-memory sqlite");
        backend.init_schema().expect("init schema");
        backend
    }

    #[tokio::test]
    async fn create_read_update_delete_under_versioning() {
        let b = backend();
        let t = tenant("alpha");

        // Some(0) creates; the stored version is 1.
        let stored = b
            .put_provider_submission(&t, "s1", json!({"name": "one"}), Some(0))
            .await
            .unwrap();
        assert_eq!(stored.version, 1);

        // Some(0) against an existing entry is a conflict.
        let err = b
            .put_provider_submission(&t, "s1", json!({"name": "dupe"}), Some(0))
            .await
            .unwrap_err();
        assert!(matches!(err, StorageError::Concurrency(_)), "{err:?}");

        // A stale version is a conflict; the matching version writes.
        let err = b
            .put_provider_submission(&t, "s1", json!({"name": "stale"}), Some(9))
            .await
            .unwrap_err();
        assert!(matches!(err, StorageError::Concurrency(_)), "{err:?}");
        let stored = b
            .put_provider_submission(&t, "s1", json!({"name": "two"}), Some(1))
            .await
            .unwrap();
        assert_eq!(stored.version, 2);

        let read = b.get_provider_submission(&t, "s1").await.unwrap().unwrap();
        assert_eq!(read.document["name"], "two");
        assert_eq!(read.version, 2);

        assert!(b.delete_provider_submission(&t, "s1").await.unwrap());
        assert!(!b.delete_provider_submission(&t, "s1").await.unwrap());
        assert!(b.get_provider_submission(&t, "s1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn listing_is_tenant_scoped() {
        let b = backend();
        b.put_provider_submission(&tenant("alpha"), "a1", json!({"n": 1}), None)
            .await
            .unwrap();
        b.put_provider_submission(&tenant("alpha"), "a2", json!({"n": 2}), None)
            .await
            .unwrap();
        b.put_provider_submission(&tenant("beta"), "b1", json!({"n": 3}), None)
            .await
            .unwrap();

        let mut alpha: Vec<String> = b
            .list_provider_submissions(&tenant("alpha"))
            .await
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();
        alpha.sort();
        assert_eq!(alpha, ["a1", "a2"]);
        assert_eq!(
            b.list_provider_submissions(&tenant("beta"))
                .await
                .unwrap()
                .len(),
            1
        );
    }
}
