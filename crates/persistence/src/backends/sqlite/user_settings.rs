//! SQLite implementation of the per-user [`SettingsStore`].
//!
//! Each user owns a single row in the `user_settings` table holding an opaque
//! JSON document plus a monotonic `version` used for optimistic locking. Writes
//! perform a read-modify-write inside a transaction so concurrent updates to the
//! same user cannot lose data or skip the `If-Match` precondition check.

use async_trait::async_trait;
use chrono::Utc;
use rusqlite::{OptionalExtension, params};
use serde_json::Value;

use crate::core::user_settings::{SettingsStore, StoredUserSettings, apply_merge_patch};
use crate::error::{BackendError, ConcurrencyError, StorageError, StorageResult};

use super::SqliteBackend;

impl SqliteBackend {
    /// Read-modify-write a user's settings document inside a single transaction.
    ///
    /// `compute` receives the currently stored document (or `None` when the user
    /// has no settings yet) and returns the document to persist. The optimistic
    /// `if_match_version` precondition — where `Some(0)` asserts "does not yet
    /// exist" — is checked against the live row before `compute` runs.
    fn write_settings(
        &self,
        user_key: &str,
        if_match_version: Option<i64>,
        compute: impl FnOnce(Option<Value>) -> Value,
    ) -> StorageResult<StoredUserSettings> {
        let mut conn = self.get_connection()?;
        let txn = conn
            .transaction()
            .map_err(|e| backend_err(format!("begin user_settings transaction: {e}")))?;

        let current: Option<(i64, Vec<u8>)> = txn
            .query_row(
                "SELECT version, data FROM user_settings WHERE user_key = ?1",
                [user_key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|e| backend_err(format!("read user_settings: {e}")))?;

        let current_version = current.as_ref().map(|(v, _)| *v).unwrap_or(0);
        if let Some(expected) = if_match_version
            && expected != current_version
        {
            return Err(lock_failure(user_key, expected, current_version));
        }

        let current_doc = match &current {
            Some((_, data)) => Some(
                serde_json::from_slice(data)
                    .map_err(|e| backend_err(format!("decode stored user_settings: {e}")))?,
            ),
            None => None,
        };

        let new_doc = compute(current_doc);
        let new_version = current_version + 1;
        let now = Utc::now();
        let updated_at = now.to_rfc3339();
        let data = serde_json::to_vec(&new_doc)
            .map_err(|e| backend_err(format!("encode user_settings: {e}")))?;

        txn.execute(
            "INSERT INTO user_settings (user_key, data, version, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(user_key) DO UPDATE SET data = ?2, version = ?3, updated_at = ?4",
            params![user_key, data, new_version, updated_at],
        )
        .map_err(|e| backend_err(format!("write user_settings: {e}")))?;

        txn.commit()
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
impl SettingsStore for SqliteBackend {
    async fn get_settings(&self, user_key: &str) -> StorageResult<Option<StoredUserSettings>> {
        let conn = self.get_connection()?;
        let row: Option<(Vec<u8>, i64, String)> = conn
            .query_row(
                "SELECT data, version, updated_at FROM user_settings WHERE user_key = ?1",
                [user_key],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|e| backend_err(format!("read user_settings: {e}")))?;

        match row {
            None => Ok(None),
            Some((data, version, updated_at)) => {
                let document = serde_json::from_slice(&data)
                    .map_err(|e| backend_err(format!("decode stored user_settings: {e}")))?;
                let updated_at = chrono::DateTime::parse_from_rfc3339(&updated_at)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());
                Ok(Some(StoredUserSettings {
                    user_key: user_key.to_string(),
                    document,
                    version,
                    updated_at,
                }))
            }
        }
    }

    async fn put_settings(
        &self,
        user_key: &str,
        document: Value,
        if_match_version: Option<i64>,
    ) -> StorageResult<StoredUserSettings> {
        self.write_settings(user_key, if_match_version, move |_current| document)
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
        backend_name: "sqlite".to_string(),
        message,
        source: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn backend() -> SqliteBackend {
        let backend = SqliteBackend::in_memory().expect("in-memory backend");
        backend.init_schema().expect("init schema");
        backend
    }

    #[tokio::test]
    async fn get_returns_none_for_unknown_user() {
        let backend = backend();
        assert!(backend.get_settings("ghost").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn put_then_get_round_trips_document() {
        let backend = backend();
        let doc = json!({"theme": "dark", "recentQueries": {"Patient": ["name=smith"]}});
        let stored = backend.put_settings("u1", doc.clone(), None).await.unwrap();
        assert_eq!(stored.version, 1);

        let fetched = backend.get_settings("u1").await.unwrap().unwrap();
        assert_eq!(fetched.document, doc);
        assert_eq!(fetched.version, 1);
    }

    #[tokio::test]
    async fn version_increments_on_each_write() {
        let backend = backend();
        backend
            .put_settings("u1", json!({"a": 1}), None)
            .await
            .unwrap();
        let second = backend
            .put_settings("u1", json!({"a": 2}), None)
            .await
            .unwrap();
        assert_eq!(second.version, 2);
    }

    #[tokio::test]
    async fn patch_merges_and_deletes_keys() {
        let backend = backend();
        backend
            .put_settings(
                "u1",
                json!({"theme": "dark", "defaultTenant": "acme"}),
                None,
            )
            .await
            .unwrap();
        let patched = backend
            .patch_settings("u1", json!({"theme": "light", "defaultTenant": null}), None)
            .await
            .unwrap();
        assert_eq!(patched.document, json!({"theme": "light"}));
        assert_eq!(patched.version, 2);
    }

    #[tokio::test]
    async fn patch_on_missing_user_creates_document() {
        let backend = backend();
        let patched = backend
            .patch_settings("u1", json!({"theme": "dark"}), None)
            .await
            .unwrap();
        assert_eq!(patched.document, json!({"theme": "dark"}));
        assert_eq!(patched.version, 1);
    }

    #[tokio::test]
    async fn stale_if_match_is_rejected() {
        let backend = backend();
        backend
            .put_settings("u1", json!({"a": 1}), None)
            .await
            .unwrap(); // version 1
        let err = backend
            .put_settings("u1", json!({"a": 2}), Some(0))
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            StorageError::Concurrency(ConcurrencyError::OptimisticLockFailure { .. })
        ));
    }

    #[tokio::test]
    async fn matching_if_match_succeeds() {
        let backend = backend();
        // Some(0) asserts "does not exist yet" for the first write.
        backend
            .put_settings("u1", json!({"a": 1}), Some(0))
            .await
            .unwrap();
        let updated = backend
            .put_settings("u1", json!({"a": 2}), Some(1))
            .await
            .unwrap();
        assert_eq!(updated.version, 2);
    }

    #[tokio::test]
    async fn get_settings_surfaces_decode_error_for_corrupt_row() {
        let backend = backend();
        // Write a row whose `data` blob is not valid JSON, bypassing the store.
        {
            let conn = backend.get_connection().unwrap();
            conn.execute(
                "INSERT INTO user_settings (user_key, data, version, updated_at) \
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![
                    "corrupt",
                    b"not json".as_slice(),
                    1_i64,
                    "2020-01-01T00:00:00Z"
                ],
            )
            .unwrap();
        }
        let err = backend.get_settings("corrupt").await.unwrap_err();
        assert!(matches!(err, StorageError::Backend(_)));
    }

    #[tokio::test]
    async fn get_settings_tolerates_unparseable_timestamp() {
        let backend = backend();
        // Valid JSON document but a malformed `updated_at`: read must not fail.
        {
            let conn = backend.get_connection().unwrap();
            conn.execute(
                "INSERT INTO user_settings (user_key, data, version, updated_at) \
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params!["odd-ts", b"{}".as_slice(), 3_i64, "not-a-timestamp"],
            )
            .unwrap();
        }
        let stored = backend.get_settings("odd-ts").await.unwrap().unwrap();
        assert_eq!(stored.version, 3);
        assert_eq!(stored.document, json!({}));
    }
}
