//! MongoDB implementation of the per-user [`SettingsStore`].
//!
//! Each user owns a single document in the `user_settings` collection holding an
//! opaque JSON settings document (stored as a JSON string, mirroring the SQLite
//! backend's JSON blob) plus a monotonic `version` used for optimistic locking.
//!
//! MongoDB standalone deployments do not support multi-document transactions, so
//! rather than a `SELECT … FOR UPDATE` read-modify-write (as SQLite/PostgreSQL
//! use) writes here are made atomic by a **version-conditioned update**: the
//! current document is read, the new document computed, then persisted with a
//! filter pinned to the version that was read. If a concurrent writer bumped the
//! version in between, the update matches nothing and the write is retried
//! (unconditional writes) or surfaced as an optimistic-lock failure (conditional
//! `If-Match` writes). A `Some(0)` precondition asserts the document does not yet
//! exist; the unique index on `user_key` turns a lost insert race into the same
//! lock failure. This yields the same read-modify-write + monotonic-version
//! semantics as the relational stores without requiring a replica set.

use std::future::Future;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use mongodb::bson::{Document, doc};
use mongodb::error::Error as MongoError;
use serde_json::Value;

use crate::core::user_settings::{SettingsStore, StoredUserSettings, apply_merge_patch};
use crate::error::{BackendError, ConcurrencyError, StorageError, StorageResult};

use super::MongoBackend;

/// Name of the collection backing the per-user settings store.
pub(crate) const USER_SETTINGS_COLLECTION: &str = "user_settings";

/// Bound on retries when an *unconditional* write loses the version-compare race
/// with a concurrent writer. A single unconditional writer can be beaten at most
/// once per other concurrent writer before it wins, so this caps the tolerated
/// concurrent-writer count. Contention on a single user's document is expected to
/// be near-zero (a user updates their own settings), so this generous bound is
/// never approached in practice; exceeding it surfaces as a concurrency error
/// rather than looping forever.
const MAX_WRITE_RETRIES: usize = 32;

impl MongoBackend {
    /// Read-modify-write a user's settings document using a version-conditioned
    /// update so concurrent writers cannot lose data or skip the `If-Match`
    /// precondition check.
    ///
    /// `compute` receives the currently stored document (or `None` when the user
    /// has no settings yet) and returns the document to persist. It may be called
    /// more than once when an unconditional write retries after losing a race.
    async fn write_settings(
        &self,
        user_key: &str,
        if_match_version: Option<i64>,
        compute: impl Fn(Option<Value>) -> Value + Send,
    ) -> StorageResult<StoredUserSettings> {
        let db = self.get_database().await?;
        let collection = db.collection::<Document>(USER_SETTINGS_COLLECTION);

        for _ in 0..=MAX_WRITE_RETRIES {
            let existing = retry_transient(|| async {
                collection.find_one(doc! { "user_key": user_key }).await
            })
            .await
            .map_err(|e| backend_err(format!("read user_settings: {e}")))?;

            let current_version = existing
                .as_ref()
                .and_then(|d| d.get_i64("version").ok())
                .unwrap_or(0);

            if let Some(expected) = if_match_version
                && expected != current_version
            {
                return Err(lock_failure(user_key, expected, current_version));
            }

            let current_doc = match &existing {
                Some(d) => Some(decode_document(d)?),
                None => None,
            };

            let new_doc = compute(current_doc);
            let new_version = current_version + 1;
            let now = Utc::now();
            let data = serde_json::to_string(&new_doc)
                .map_err(|e| backend_err(format!("encode user_settings: {e}")))?;
            let now_bson = mongodb::bson::DateTime::from_millis(now.timestamp_millis());

            if current_version == 0 {
                // No document yet: insert. A unique index on `user_key` makes a
                // concurrent insert fail with a duplicate-key error, which we
                // treat as a lost race (retry) or a lock failure (conditional).
                match retry_transient(|| async {
                    collection
                        .insert_one(doc! {
                            "user_key": user_key,
                            "data": &data,
                            "version": new_version,
                            "updated_at": now_bson,
                        })
                        .await
                })
                .await
                {
                    Ok(_) => {
                        return Ok(StoredUserSettings {
                            user_key: user_key.to_string(),
                            document: new_doc,
                            version: new_version,
                            updated_at: now,
                        });
                    }
                    Err(e) if is_duplicate_key_error(&e) => {
                        if let Some(expected) = if_match_version {
                            let actual = reload_version(&collection, user_key).await?;
                            return Err(lock_failure(user_key, expected, actual));
                        }
                        continue;
                    }
                    Err(e) => {
                        return Err(backend_err(format!("insert user_settings: {e}")));
                    }
                }
            }

            // A document exists: update only if its version is unchanged since we
            // read it, so a concurrent writer's change is never clobbered.
            let result = retry_transient(|| async {
                collection
                    .update_one(
                        doc! { "user_key": user_key, "version": current_version },
                        doc! { "$set": {
                            "data": &data,
                            "version": new_version,
                            "updated_at": now_bson,
                        }},
                    )
                    .await
            })
            .await
            .map_err(|e| backend_err(format!("write user_settings: {e}")))?;

            if result.matched_count == 1 {
                return Ok(StoredUserSettings {
                    user_key: user_key.to_string(),
                    document: new_doc,
                    version: new_version,
                    updated_at: now,
                });
            }

            // Version moved under us. A conditional write fails the precondition;
            // an unconditional write retries against the new state.
            if let Some(expected) = if_match_version {
                let actual = reload_version(&collection, user_key).await?;
                return Err(lock_failure(user_key, expected, actual));
            }
        }

        Err(StorageError::Concurrency(
            ConcurrencyError::OptimisticLockFailure {
                resource_type: "UserSettings".to_string(),
                id: user_key.to_string(),
                expected_etag: "W/\"*\"".to_string(),
                actual_etag: None,
            },
        ))
    }
}

#[async_trait]
impl SettingsStore for MongoBackend {
    async fn get_settings(&self, user_key: &str) -> StorageResult<Option<StoredUserSettings>> {
        let db = self.get_database().await?;
        let collection = db.collection::<Document>(USER_SETTINGS_COLLECTION);
        let row =
            retry_transient(|| async { collection.find_one(doc! { "user_key": user_key }).await })
                .await
                .map_err(|e| backend_err(format!("read user_settings: {e}")))?;

        match row {
            None => Ok(None),
            Some(d) => {
                let document = decode_document(&d)?;
                let version = d
                    .get_i64("version")
                    .map_err(|e| backend_err(format!("read user_settings version: {e}")))?;
                let updated_at = d
                    .get_datetime("updated_at")
                    .map(|dt| {
                        chrono::DateTime::<Utc>::from_timestamp_millis(dt.timestamp_millis())
                            .unwrap_or_else(Utc::now)
                    })
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
        self.write_settings(user_key, if_match_version, move |_current| document.clone())
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

    async fn delete_settings(&self, user_key: &str) -> StorageResult<bool> {
        let db = self.get_database().await?;
        let collection = db.collection::<Document>(USER_SETTINGS_COLLECTION);
        let result = retry_transient(|| async {
            collection.delete_one(doc! { "user_key": user_key }).await
        })
        .await
        .map_err(|e| backend_err(format!("delete user_settings: {e}")))?;
        Ok(result.deleted_count > 0)
    }
}

/// Decodes the JSON `data` field of a stored `user_settings` document.
fn decode_document(doc: &Document) -> StorageResult<Value> {
    let data = doc
        .get_str("data")
        .map_err(|e| backend_err(format!("read user_settings data: {e}")))?;
    serde_json::from_str(data).map_err(|e| backend_err(format!("decode stored user_settings: {e}")))
}

/// Reloads the live version for a user, used to report the actual version in an
/// optimistic-lock failure. A vanished document reads back as version 0.
async fn reload_version(
    collection: &mongodb::Collection<Document>,
    user_key: &str,
) -> StorageResult<i64> {
    let row =
        retry_transient(|| async { collection.find_one(doc! { "user_key": user_key }).await })
            .await
            .map_err(|e| backend_err(format!("reload user_settings version: {e}")))?;
    Ok(row.and_then(|d| d.get_i64("version").ok()).unwrap_or(0))
}

fn is_duplicate_key_error(err: &MongoError) -> bool {
    err.to_string().contains("E11000")
}

/// Bound on retries when a MongoDB operation fails with a *transient* error.
/// Four attempts with exponential backoff (25/50/100 ms) covers a brief server
/// blip without stalling a genuine outage for long.
const MAX_TRANSIENT_RETRIES: u32 = 4;

/// True when a MongoDB error is transient and safe to retry: one the driver has
/// itself labelled retryable, or a fast network/connection failure (e.g. a
/// connection reset by a momentarily overloaded server). The driver retries such
/// errors once; a server that stays busy longer than that outlasts the single
/// retry, so we add a short bounded retry on top. Non-transient errors
/// (duplicate key, bad command, decode) are never retried here.
///
/// A `ServerSelection` timeout is deliberately *not* treated as transient: it
/// already means the driver waited its full `server_selection_timeout` and found
/// no usable server, so a fast backoff-retry would just pay that wait again
/// (blocking the caller for minutes against a genuinely-down server) without
/// improving the odds. Such an error is surfaced promptly instead.
fn is_transient_mongo_error(err: &MongoError) -> bool {
    use mongodb::error::{ErrorKind, RETRYABLE_ERROR, RETRYABLE_WRITE_ERROR};

    err.contains_label(RETRYABLE_ERROR)
        || err.contains_label(RETRYABLE_WRITE_ERROR)
        || matches!(
            err.kind.as_ref(),
            ErrorKind::Io(_) | ErrorKind::ConnectionPoolCleared { .. }
        )
}

/// Runs a MongoDB operation, retrying it on a [transient error](is_transient_mongo_error)
/// with exponential backoff.
///
/// The settings-store writes are already safe to re-run: reads are pure, and a
/// re-executed insert/update is caught by the version-conditioned filter and the
/// duplicate-key path in [`MongoBackend::write_settings`], so a retry after a
/// lost acknowledgement cannot double-apply.
async fn retry_transient<T, F, Fut>(mut op: F) -> Result<T, MongoError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, MongoError>>,
{
    let mut attempt: u32 = 1;
    loop {
        match op().await {
            Ok(value) => return Ok(value),
            Err(err) if attempt < MAX_TRANSIENT_RETRIES && is_transient_mongo_error(&err) => {
                let backoff = Duration::from_millis(25u64 << (attempt - 1));
                tokio::time::sleep(backoff).await;
                attempt += 1;
            }
            Err(err) => return Err(err),
        }
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
        backend_name: "mongodb".to_string(),
        message,
        source: None,
    })
}
