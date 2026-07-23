//! S3 implementation of the per-user [`SettingsStore`].
//!
//! Each user owns a single JSON object holding an opaque settings document plus
//! a monotonic `version` used for optimistic locking, mirroring the row (SQLite,
//! PostgreSQL) and document (MongoDB) the other backends keep.
//!
//! # Compare-and-swap instead of a transaction
//!
//! S3 has no transactions, so the read-modify-write that SQLite and PostgreSQL
//! perform under `SELECT … FOR UPDATE` is instead made atomic by a **conditional
//! write**: the object is fetched (yielding both the body and its S3 ETag), the
//! new document is computed, and the write is issued with a precondition pinned
//! to the ETag that was read — `If-Match: {etag}` when updating, or
//! `If-None-Match: *` when creating. If a concurrent writer changed the object in
//! between, its ETag no longer matches and S3 rejects the write with a
//! precondition failure, which is then either retried (unconditional writes) or
//! surfaced as an optimistic-lock failure (conditional `If-Match` writes). This
//! is the same compare-and-swap shape as the MongoDB backend's
//! version-conditioned update, with the S3 ETag playing the role of the version
//! filter. A settings write is therefore **never** an unconditional `PutObject`.
//!
//! The S3 backend already depends on conditional `PutObject` for FHIR resource
//! create/update/delete, so this adds no new requirement on the object store.
//!
//! # Two distinct tokens
//!
//! The client-facing optimistic-lock token is the monotonic `version` (surfaced
//! as the weak ETag `W/"{version}"`); the S3 ETag is a purely internal
//! compare-and-swap token whose lifetime is a single write attempt. They are
//! never converted into one another, and a client's `If-Match: W/"6"` is never
//! forwarded to S3. The `version` lives in the object *body* — not in object
//! metadata — so that one `GetObject` returns the document and its ETag from the
//! same snapshot, and so that no two successive writes ever produce identical
//! bytes (which is what rules out an ETag ABA hazard).
//!
//! # Object naming
//!
//! The object is named by a SHA-256 digest of the user key, never by the key
//! itself: see [`settings_object_id`].

use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::core::user_settings::{SettingsStore, StoredUserSettings, apply_merge_patch};
use crate::error::{BackendError, ConcurrencyError, StorageError, StorageResult};

use super::backend::{S3Backend, TenantLocation};
use super::client::S3ClientError;

/// Bound on retries when an *unconditional* write loses the compare-and-swap
/// race with a concurrent writer. This counts total *attempts*, not retries.
///
/// This is a retry bound, **not** a guaranteed tolerance of N concurrent writers:
/// an S3 compare-and-swap has no fairness, so an unlucky writer can lose several
/// rounds in a row. With N contenders a writer typically needs at most N attempts;
/// pathological scheduling needs more, and exhausting the bound surfaces as a
/// concurrency error rather than looping forever. Contention on one user's own
/// settings document is expected to be near-zero (a user updates their own
/// settings) and each attempt costs two S3 round trips, so the bound is
/// deliberately tighter than the relational backends'.
const MAX_WRITE_ATTEMPTS: usize = 8;

/// Base unit for the backoff between compare-and-swap attempts.
///
/// The AWS SDK retries transient errors (throttling, 5xx) itself, but a failed
/// precondition is a 4xx it will never retry — so the compare-and-swap loop is on
/// its own here. Without a backoff, N writers that collide would immediately
/// re-read and re-write in lockstep, colliding again; the jittered delay breaks up
/// that herd so contending writers serialise instead of thrashing.
const RETRY_BACKOFF_BASE: Duration = Duration::from_millis(5);

/// Returns the jittered backoff to wait before compare-and-swap attempt `attempt`
/// (0-based), growing exponentially.
///
/// The jitter must be drawn *independently per writer*, not read from the clock:
/// two tasks that lose the same compare-and-swap receive their errors within
/// microseconds of each other, so clock-derived "noise" would hand them nearly
/// identical delays and they would simply collide again. A v4 UUID gives an
/// independent draw without taking on a random-number dependency (`uuid` is
/// already a dependency of this crate).
fn retry_backoff(attempt: usize) -> Duration {
    let exponential = RETRY_BACKOFF_BASE * (1 << attempt.min(5)) as u32;
    // Full jitter: sleep for a uniformly random point in [0, exponential).
    let span = u64::try_from(exponential.as_nanos()).unwrap_or(u64::MAX);
    let draw = Uuid::new_v4().as_u128() as u64;
    Duration::from_nanos(draw % span)
}

/// The stored form of a user's settings: the opaque document plus the metadata
/// needed to reconstruct a [`StoredUserSettings`].
///
/// `user_key` is persisted verbatim even though it is not part of the object's
/// name. It is a tripwire: every read asserts it matches the key that was
/// requested, so any future regression in the key derivation surfaces as an error
/// rather than silently handing one user another user's settings. It also keeps
/// the objects debuggable, since the key itself is only a digest.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SettingsObject {
    /// The opaque key identifying the owning user.
    user_key: String,
    /// The opaque settings document. Always a JSON object.
    document: Value,
    /// Monotonic version, bumped on every write.
    version: i64,
    /// Timestamp of the most recent write.
    updated_at: DateTime<Utc>,
}

/// Derives the S3 object name for a user key: the lowercase hex SHA-256 of the
/// key's UTF-8 bytes.
///
/// The user key is built from JWT claims that the server does not constrain: it
/// may contain `/`, `\`, `..`, or be empty, and it is unbounded in length.
/// Hashing — rather than escaping or sanitising — is what makes the mapping
/// safe:
///
/// - **Injective in practice.** A lossy transform (such as the keyspace's
///   `sanitize`, which maps both `/` and space to `_`) would let two distinct
///   principals collide on one object, letting one user read and overwrite
///   another's settings.
/// - **Path-safe.** The digest is 64 characters of `[0-9a-f]`, so no user key can
///   introduce a `/` or a `..` segment and escape the settings namespace into the
///   FHIR resource keyspace.
/// - **Fixed length.** S3 keys are limited to 1024 bytes; an unbounded `sub`
///   claim could otherwise exceed it.
/// - **No identifiers in key names.** The issuer and subject (often an email) stay
///   out of S3 access logs, bucket inventories, and listings.
///
/// The digest is over raw bytes, so no Unicode normalisation is applied: two
/// principals differing only in normalisation form get two documents, which is
/// the fail-safe direction.
pub(crate) fn settings_object_id(user_key: &str) -> String {
    let digest = Sha256::digest(user_key.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

impl S3Backend {
    /// Resolves the bucket and object key holding `user_key`'s settings.
    fn settings_key(&self, user_key: &str) -> StorageResult<(TenantLocation, String)> {
        let location = self.settings_location()?;
        let key = location
            .keyspace
            .user_settings_key(&settings_object_id(user_key));
        Ok((location, key))
    }

    /// Fetches the stored settings object together with the S3 ETag it must be
    /// compare-and-swapped against.
    ///
    /// A single `GetObject` returns the body and the ETag from the same snapshot;
    /// a `HeadObject` + `GetObject` pair could straddle a concurrent write and
    /// pair one generation's version with another's ETag.
    async fn load_settings(
        &self,
        bucket: &str,
        key: &str,
        user_key: &str,
    ) -> StorageResult<Option<(SettingsObject, Option<String>)>> {
        let Some((stored, metadata)) = self
            .get_json_object::<SettingsObject>(bucket, key)
            .await
            .map_err(|e| context("read user_settings", e))?
        else {
            return Ok(None);
        };

        // Tripwire: the object's name is a digest of the user key, so this can
        // only fire if the key derivation regressed. Fail rather than return
        // another user's document.
        if stored.user_key != user_key {
            return Err(StorageError::Backend(BackendError::Internal {
                backend_name: "s3".to_string(),
                message: format!(
                    "user_settings object {key} belongs to a different user than the one requested"
                ),
                source: None,
            }));
        }

        Ok(Some((stored, metadata.etag)))
    }

    /// Reloads the live version for a user, used to report the actual version in
    /// an optimistic-lock failure. A vanished object reads back as version 0.
    async fn reload_settings_version(
        &self,
        bucket: &str,
        key: &str,
        user_key: &str,
    ) -> StorageResult<i64> {
        Ok(self
            .load_settings(bucket, key, user_key)
            .await?
            .map(|(stored, _)| stored.version)
            .unwrap_or(0))
    }

    /// Read-modify-write a user's settings document using a conditional write, so
    /// that no write is silently lost and no writer can skip the `If-Match`
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
        let (location, key) = self.settings_key(user_key)?;
        let bucket = location.bucket.as_str();

        for attempt in 0..MAX_WRITE_ATTEMPTS {
            // Back off before re-reading, so writers that just collided do not
            // re-collide in lockstep. Skipped on the first (uncontended) attempt.
            if attempt > 0 {
                tokio::time::sleep(retry_backoff(attempt - 1)).await;
            }

            let existing = self.load_settings(bucket, &key, user_key).await?;
            let current_version = existing
                .as_ref()
                .map(|(stored, _)| stored.version)
                .unwrap_or(0);

            // The caller's precondition is checked against the version in the
            // body, in-process — never against the S3 ETag, which is a different
            // token space.
            if let Some(expected) = if_match_version
                && expected != current_version
            {
                return Err(lock_failure(user_key, expected, current_version));
            }

            let current_doc = existing.as_ref().map(|(stored, _)| stored.document.clone());
            let now = Utc::now();
            let new_object = SettingsObject {
                user_key: user_key.to_string(),
                document: compute(current_doc),
                version: current_version + 1,
                updated_at: now,
            };
            let payload = self.serialize_json(&new_object)?;

            // Pin the write to the exact object generation that was just read: an
            // existing object must still carry the ETag we saw, and a create must
            // find no object at all. Never an unconditional put — that is what a
            // silent lost update would look like.
            let etag = match &existing {
                Some((_, Some(etag))) => Some(etag.clone()),
                Some((_, None)) => {
                    return Err(StorageError::Backend(BackendError::Internal {
                        backend_name: "s3".to_string(),
                        message: format!(
                            "S3 returned no ETag for existing user_settings object {key}; \
                             refusing an unconditional write"
                        ),
                        source: None,
                    }));
                }
                None => None,
            };
            let (if_match, if_none_match) = match etag.as_deref() {
                Some(etag) => (Some(etag), None),
                None => (None, Some("*")),
            };

            // Call the client directly rather than `put_json_object`: that helper
            // routes errors through `map_client_error`, which folds
            // `PreconditionFailed` into a `QueryError` (a mapping the resource
            // paths depend on to synthesize `AlreadyExists`/`VersionConflict`).
            // The compare-and-swap outcome must stay distinguishable here.
            let result = self
                .client
                .put_object(
                    bucket,
                    &key,
                    payload,
                    Some("application/json"),
                    if_match,
                    if_none_match,
                )
                .await;

            let err = match result {
                Ok(_) => {
                    return Ok(StoredUserSettings {
                        user_key: user_key.to_string(),
                        document: new_object.document,
                        version: new_object.version,
                        updated_at: now,
                    });
                }
                Err(err) => err,
            };

            // Did we lose the compare-and-swap? A failed precondition always means
            // yes. A `NotFound` means yes only when we asserted `If-Match`: some
            // S3-compatible stores answer an `If-Match` against a since-deleted
            // object with 404 rather than 412. A `NotFound` on a create
            // (`If-None-Match: *`) instead means the bucket is missing, which is a
            // genuine error and must not be retried as contention.
            let lost_race = matches!(err, S3ClientError::PreconditionFailed)
                || (matches!(err, S3ClientError::NotFound) && if_match.is_some());

            if !lost_race {
                return Err(context("write user_settings", self.map_client_error(err)));
            }

            // A conditional write must not retry: the caller asked to write only
            // if the version was still `expected`, and it no longer is.
            if let Some(expected) = if_match_version {
                let actual = self.reload_settings_version(bucket, &key, user_key).await?;
                return Err(lock_failure(user_key, expected, actual));
            }

            // An unconditional write re-reads and retries against the new state.
            // No write is ever *silently* lost: every attempt is pinned to the
            // generation it read. Note this does not make a `put_settings` a
            // merge — a PUT replaces the document by definition, so it is
            // last-writer-wins over the winner's *content*, while a
            // `patch_settings` re-applies its merge onto the winner.
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
impl SettingsStore for S3Backend {
    async fn get_settings(&self, user_key: &str) -> StorageResult<Option<StoredUserSettings>> {
        let (location, key) = self.settings_key(user_key)?;
        let Some((stored, _etag)) = self.load_settings(&location.bucket, &key, user_key).await?
        else {
            return Ok(None);
        };

        Ok(Some(StoredUserSettings {
            user_key: user_key.to_string(),
            document: stored.document,
            version: stored.version,
            updated_at: stored.updated_at,
        }))
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
        let (location, key) = self.settings_key(user_key)?;

        // Probe first so the return value distinguishes "removed one" from
        // "there was nothing there" — `DeleteObject` is idempotent and reports
        // success either way. The probe also runs the owner tripwire in
        // `load_settings`, so a body whose `user_key` disagrees with the object
        // name surfaces as an error rather than being silently deleted.
        let existed = self
            .load_settings(&location.bucket, &key, user_key)
            .await?
            .is_some();

        self.client
            .delete_object(&location.bucket, &key)
            .await
            .map_err(|e| context("delete user_settings", self.map_client_error(e)))?;

        Ok(existed)
    }
}

/// Builds an `OptimisticLockFailure` for a `user_settings` write whose `If-Match`
/// precondition did not match the live version.
fn lock_failure(user_key: &str, expected: i64, actual: i64) -> StorageError {
    StorageError::Concurrency(ConcurrencyError::OptimisticLockFailure {
        resource_type: "UserSettings".to_string(),
        id: user_key.to_string(),
        expected_etag: format!("W/\"{expected}\""),
        actual_etag: Some(format!("W/\"{actual}\"")),
    })
}

/// Prefixes the operation name onto a backend error message.
///
/// Every `BackendError` this module can raise is labelled, not just the internal
/// ones: the failures an operator actually hits in production are transport
/// failures — a missing bucket, throttling, and above all `AccessDenied` on the
/// settings prefix (easy to hit with a least-privilege bucket policy written
/// before this namespace existed) — and those arrive as `Unavailable`. An
/// unlabelled "internal error while processing request" is not something anyone
/// can act on at 3am.
///
/// Backend errors from this module identify the failing object by its key — an
/// opaque digest — rather than by the raw user key, which is built from the
/// authenticated issuer and subject and is therefore personally identifying.
/// (`OptimisticLockFailure` still carries the user key in its `id`, matching the
/// SQLite, PostgreSQL, and MongoDB stores, whose `id` is likewise the user key.)
fn context(operation: &str, err: StorageError) -> StorageError {
    match err {
        StorageError::Backend(BackendError::Internal {
            backend_name,
            message,
            source,
        }) => StorageError::Backend(BackendError::Internal {
            backend_name,
            message: format!("{operation}: {message}"),
            source,
        }),
        // A corrupt or undecodable settings object arrives as a serialization
        // error; it needs the operation label just as much as an internal one.
        StorageError::Backend(BackendError::SerializationError { message }) => {
            StorageError::Backend(BackendError::SerializationError {
                message: format!("{operation}: {message}"),
            })
        }
        // The transport failures: missing bucket, throttling, timeouts, and
        // access denied. These are the ones an operator has to diagnose.
        StorageError::Backend(BackendError::Unavailable {
            backend_name,
            message,
        }) => StorageError::Backend(BackendError::Unavailable {
            backend_name,
            message: format!("{operation}: {message}"),
        }),
        StorageError::Backend(BackendError::QueryError { message }) => {
            StorageError::Backend(BackendError::QueryError {
                message: format!("{operation}: {message}"),
            })
        }
        other => other,
    }
}
