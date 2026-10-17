//! S3 implementation of the web UI's [`SessionPersistence`].
//!
//! One JSON object per session or pending login under
//! `_system.login-sessions/{kind}/{id}.json` in the tenant-independent bucket
//! that also holds `/_user/settings` (so, like that store, unavailable in
//! bucket-per-tenant mode with no system bucket). The monotonic `version`
//! lives in the object body; the S3 ETag is the compare-and-swap token for a
//! single write attempt, exactly as in `user_settings.rs`: a session write is
//! never an unconditional `PutObject`, so two nodes writing the same session
//! cannot lose an update — the loser's precondition fails and it reports a
//! conflict.
//!
//! A pending login is consumed by a conditional write, not a delete:
//! `DeleteObject` cannot be made conditional on every S3-compatible store, so
//! the callback that wins writes a tombstone with `If-Match` on the ETag it
//! read, and only that callback proceeds; the object is then removed.
//!
//! The sweep works from the listing's `last-modified` rather than reading
//! every object: a pending login is written once, so its age is exact, and a
//! live session is rewritten at least once per touch interval, so an object
//! older than the idle expiry plus that interval is certainly idle.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use helios_auth::AuthError;
use helios_auth::session::{
    PENDING_TTL, PersistedPending, PersistedSession, SESSION_IDLE_TTL, SaveOutcome,
    SessionPersistence, TOUCH_WRITE_INTERVAL,
};
use serde::{Deserialize, Serialize};

use super::backend::{S3Backend, TenantLocation};
use super::client::S3ClientError;
use super::user_settings::retry_backoff;

const SESSION: &str = "session";
const PENDING: &str = "pending";

/// Bound on attempts when an *unconditional* session write loses the
/// compare-and-swap race. Contention on one session is two nodes refreshing
/// at once, and those writes are conditional, so this is never approached.
const MAX_WRITE_ATTEMPTS: usize = 8;

/// A sweep lists both prefixes; one node need not do that on every login.
const SWEEP_INTERVAL: Duration = Duration::from_secs(5 * 60);

static LAST_SWEEP: Mutex<Option<Instant>> = Mutex::new(None);

/// The stored form of a session: the document plus the version the write
/// landed at (the document itself carries no version on the wire).
#[derive(Debug, Serialize, Deserialize)]
struct SessionObject {
    version: i64,
    session: PersistedSession,
}

/// The stored form of a pending login. A consumed one keeps a tombstone
/// until it is removed, so a second callback reads "already consumed" rather
/// than "still pending".
#[derive(Debug, Serialize, Deserialize)]
struct PendingObject {
    consumed: bool,
    pending: Option<PersistedPending>,
}

impl S3Backend {
    fn login_key(&self, kind: &str, id: &str) -> Result<(TenantLocation, String), AuthError> {
        let location = self
            .settings_location()
            .map_err(|e| store_err(e.to_string()))?;
        let key = location.keyspace.login_session_key(kind, id);
        Ok((location, key))
    }

    /// Writes `payload` at `key`, pinned to the generation that was read:
    /// `If-Match` on its ETag, or `If-None-Match: *` for a create. Reports
    /// whether the compare-and-swap was lost rather than failing on it.
    async fn put_login_object(
        &self,
        bucket: &str,
        key: &str,
        payload: Vec<u8>,
        etag: Option<&str>,
    ) -> Result<bool, AuthError> {
        let (if_match, if_none_match) = match etag {
            Some(etag) => (Some(etag), None),
            None => (None, Some("*")),
        };
        match self
            .client
            .put_object(
                bucket,
                key,
                payload,
                Some("application/json"),
                if_match,
                if_none_match,
            )
            .await
        {
            Ok(_) => Ok(true),
            Err(S3ClientError::PreconditionFailed) => Ok(false),
            // Some S3-compatible stores answer an `If-Match` against a
            // since-deleted object with 404 rather than 412.
            Err(S3ClientError::NotFound) if if_match.is_some() => Ok(false),
            Err(e) => Err(store_err(format!(
                "write {key}: {}",
                self.map_client_error(e)
            ))),
        }
    }
}

#[async_trait]
impl SessionPersistence for S3Backend {
    async fn load_session(&self, id: &str) -> Result<Option<PersistedSession>, AuthError> {
        let (location, key) = self.login_key(SESSION, id)?;
        let stored = self
            .get_json_object::<SessionObject>(&location.bucket, &key)
            .await
            .map_err(|e| store_err(format!("read session: {e}")))?;
        Ok(stored.map(|(object, _)| {
            let mut session = object.session;
            session.version = object.version;
            session
        }))
    }

    async fn save_session(
        &self,
        session: &PersistedSession,
        if_version: Option<i64>,
    ) -> Result<SaveOutcome, AuthError> {
        let (location, key) = self.login_key(SESSION, &session.id)?;
        let bucket = location.bucket.as_str();

        for attempt in 0..MAX_WRITE_ATTEMPTS {
            if attempt > 0 {
                tokio::time::sleep(retry_backoff(attempt - 1)).await;
            }
            let existing = self
                .get_json_object::<SessionObject>(bucket, &key)
                .await
                .map_err(|e| store_err(format!("read session: {e}")))?;
            let current = existing.as_ref().map(|(o, _)| o.version).unwrap_or(0);
            if let Some(expected) = if_version
                && expected != current
            {
                return Ok(SaveOutcome::Conflict { current });
            }
            let etag = match &existing {
                Some((_, metadata)) => Some(metadata.etag.clone().ok_or_else(|| {
                    store_err(format!(
                        "S3 returned no ETag for session object {key}; refusing an unconditional write"
                    ))
                })?),
                None => None,
            };
            let next = current + 1;
            let payload = self
                .serialize_json(&SessionObject {
                    version: next,
                    session: session.clone(),
                })
                .map_err(|e| store_err(e.to_string()))?;
            if self
                .put_login_object(bucket, &key, payload, etag.as_deref())
                .await?
            {
                return Ok(SaveOutcome::Saved(next));
            }
            if if_version.is_some() {
                let current = self
                    .get_json_object::<SessionObject>(bucket, &key)
                    .await
                    .map_err(|e| store_err(format!("read session: {e}")))?
                    .map(|(o, _)| o.version)
                    .unwrap_or(0);
                return Ok(SaveOutcome::Conflict { current });
            }
        }
        Err(store_err(
            "session write lost the compare-and-swap too many times".to_string(),
        ))
    }

    async fn delete_session(&self, id: &str) -> Result<(), AuthError> {
        let (location, key) = self.login_key(SESSION, id)?;
        self.client
            .delete_object(&location.bucket, &key)
            .await
            .map_err(|e| store_err(format!("delete session: {}", self.map_client_error(e))))?;
        Ok(())
    }

    async fn load_pending(&self, id: &str) -> Result<Option<PersistedPending>, AuthError> {
        let (location, key) = self.login_key(PENDING, id)?;
        let stored = self
            .get_json_object::<PendingObject>(&location.bucket, &key)
            .await
            .map_err(|e| store_err(format!("read pending login: {e}")))?;
        Ok(stored.and_then(|(object, _)| {
            if object.consumed {
                None
            } else {
                object.pending
            }
        }))
    }

    async fn save_pending(&self, pending: &PersistedPending) -> Result<(), AuthError> {
        let (location, key) = self.login_key(PENDING, &pending.id)?;
        let payload = self
            .serialize_json(&PendingObject {
                consumed: false,
                pending: Some(pending.clone()),
            })
            .map_err(|e| store_err(e.to_string()))?;
        self.client
            .put_object(
                &location.bucket,
                &key,
                payload,
                Some("application/json"),
                None,
                None,
            )
            .await
            .map_err(|e| store_err(format!("write pending login: {}", self.map_client_error(e))))?;
        Ok(())
    }

    async fn delete_pending(&self, id: &str) -> Result<bool, AuthError> {
        let (location, key) = self.login_key(PENDING, id)?;
        let bucket = location.bucket.as_str();
        let Some((object, metadata)) = self
            .get_json_object::<PendingObject>(bucket, &key)
            .await
            .map_err(|e| store_err(format!("read pending login: {e}")))?
        else {
            return Ok(false);
        };
        if object.consumed {
            return Ok(false);
        }
        let etag = metadata.etag.ok_or_else(|| {
            store_err(format!(
                "S3 returned no ETag for pending login {key}; cannot consume it exactly once"
            ))
        })?;
        let tombstone = self
            .serialize_json(&PendingObject {
                consumed: true,
                pending: None,
            })
            .map_err(|e| store_err(e.to_string()))?;
        if !self
            .put_login_object(bucket, &key, tombstone, Some(&etag))
            .await?
        {
            return Ok(false);
        }
        // The tombstone did the consuming; the delete only tidies up, and
        // the sweep removes it anyway if this fails.
        if let Err(e) = self.client.delete_object(bucket, &key).await {
            tracing::debug!(error = ?e, key, "consumed pending login not removed yet");
        }
        Ok(true)
    }

    async fn sweep(&self, now: DateTime<Utc>) -> Result<u64, AuthError> {
        {
            let mut last = LAST_SWEEP
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if last.is_some_and(|at| at.elapsed() < SWEEP_INTERVAL) {
                return Ok(0);
            }
            *last = Some(Instant::now());
        }
        let location = self
            .settings_location()
            .map_err(|e| store_err(e.to_string()))?;
        let bucket = location.bucket.as_str();
        let mut removed = 0u64;
        for (kind, ttl) in [
            (SESSION, SESSION_IDLE_TTL + TOUCH_WRITE_INTERVAL),
            (PENDING, PENDING_TTL),
        ] {
            let ttl = chrono::TimeDelta::from_std(ttl).unwrap_or(chrono::TimeDelta::MAX);
            let prefix = location.keyspace.login_sessions_prefix(kind);
            let items = self
                .list_objects_all(bucket, &prefix)
                .await
                .map_err(|e| store_err(format!("list {kind} logins: {e}")))?;
            for item in items {
                let Some(modified) = item.last_modified else {
                    continue;
                };
                if modified + ttl >= now {
                    continue;
                }
                self.client
                    .delete_object(bucket, &item.key)
                    .await
                    .map_err(|e| {
                        store_err(format!("sweep {kind}: {}", self.map_client_error(e)))
                    })?;
                removed += 1;
            }
        }
        Ok(removed)
    }
}

fn store_err(message: String) -> AuthError {
    AuthError::InternalError(format!("s3 login_sessions: {message}"))
}
