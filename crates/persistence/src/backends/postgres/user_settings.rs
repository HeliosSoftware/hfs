//! PostgreSQL implementation of the per-user [`SettingsStore`].
//!
//! Each user owns a single row in the `user_settings` table holding an opaque
//! JSONB document plus a monotonic `version` used for optimistic locking. Writes
//! run a `SELECT … FOR UPDATE` read-modify-write inside a transaction so
//! concurrent updates to the same user serialize correctly and the `If-Match`
//! precondition is checked against the live row.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::core::user_settings::{SettingsStore, StoredUserSettings, apply_merge_patch};
use crate::error::{BackendError, ConcurrencyError, StorageError, StorageResult};

use super::PostgresBackend;

impl PostgresBackend {
    /// Read-modify-write a user's settings document inside a single transaction,
    /// locking the row with `SELECT … FOR UPDATE`.
    ///
    /// `compute` receives the currently stored document (or `None` when the user
    /// has no settings yet) and returns the document to persist. The optimistic
    /// `if_match_version` precondition — where `Some(0)` asserts "does not yet
    /// exist" — is checked against the locked row before `compute` runs.
    async fn write_settings(
        &self,
        user_key: &str,
        if_match_version: Option<i64>,
        compute: impl FnOnce(Option<Value>) -> Value + Send,
    ) -> StorageResult<StoredUserSettings> {
        let mut client = self.get_client().await?;
        let txn = client
            .transaction()
            .await
            .map_err(|e| backend_err(format!("begin user_settings transaction: {e}")))?;

        let current = txn
            .query_opt(
                "SELECT version, data FROM user_settings WHERE user_key = $1 FOR UPDATE",
                &[&user_key],
            )
            .await
            .map_err(|e| backend_err(format!("read user_settings: {e}")))?;

        let (current_version, current_doc) = match &current {
            Some(row) => {
                let version: i64 = row.get(0);
                let doc: Value = row.get(1);
                (version, Some(doc))
            }
            None => (0, None),
        };

        if let Some(expected) = if_match_version
            && expected != current_version
        {
            return Err(lock_failure(user_key, expected, current_version));
        }

        let new_doc = compute(current_doc);
        let new_version = current_version + 1;
        let now = Utc::now();

        txn.execute(
            "INSERT INTO user_settings (user_key, data, version, updated_at)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (user_key)
             DO UPDATE SET data = $2, version = $3, updated_at = $4",
            &[&user_key, &new_doc, &new_version, &now],
        )
        .await
        .map_err(|e| backend_err(format!("write user_settings: {e}")))?;

        txn.commit()
            .await
            .map_err(|e| backend_err(format!("commit user_settings: {e}")))?;

        Ok(StoredUserSettings {
            user_key: user_key.to_string(),
            document: new_doc,
            version: new_version,
            updated_at: now,
        })
    }
}

#[async_trait]
impl SettingsStore for PostgresBackend {
    async fn get_settings(&self, user_key: &str) -> StorageResult<Option<StoredUserSettings>> {
        let client = self.get_client().await?;
        let row = client
            .query_opt(
                "SELECT data, version, updated_at FROM user_settings WHERE user_key = $1",
                &[&user_key],
            )
            .await
            .map_err(|e| backend_err(format!("read user_settings: {e}")))?;

        Ok(row.map(|row| {
            let document: Value = row.get(0);
            let version: i64 = row.get(1);
            let updated_at: DateTime<Utc> = row.get(2);
            StoredUserSettings {
                user_key: user_key.to_string(),
                document,
                version,
                updated_at,
            }
        }))
    }

    async fn put_settings(
        &self,
        user_key: &str,
        document: Value,
        if_match_version: Option<i64>,
    ) -> StorageResult<StoredUserSettings> {
        self.write_settings(user_key, if_match_version, move |_current| document)
            .await
    }

    async fn patch_settings(
        &self,
        user_key: &str,
        merge_patch: Value,
        if_match_version: Option<i64>,
    ) -> StorageResult<StoredUserSettings> {
        self.write_settings(user_key, if_match_version, move |current| {
            apply_merge_patch(
                current.unwrap_or_else(|| Value::Object(Default::default())),
                &merge_patch,
            )
        })
        .await
    }
}

/// Builds an `OptimisticLockFailure` for a `user_settings` write whose
/// `If-Match` precondition did not match the live version.
fn lock_failure(user_key: &str, expected: i64, actual: i64) -> StorageError {
    StorageError::Concurrency(ConcurrencyError::OptimisticLockFailure {
        resource_type: "UserSettings".to_string(),
        id: user_key.to_string(),
        expected_etag: format!("W/\"{expected}\""),
        actual_etag: Some(format!("W/\"{actual}\"")),
    })
}

fn backend_err(message: String) -> StorageError {
    StorageError::Backend(BackendError::Internal {
        backend_name: "postgres".to_string(),
        message,
        source: None,
    })
}
