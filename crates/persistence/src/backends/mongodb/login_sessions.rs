//! MongoDB implementation of the web UI's [`SessionPersistence`].
//!
//! One document per session or pending login in the `login_sessions`
//! collection, keyed by the opaque id as `_id`, with a `kind` telling the two
//! apart, the JSON document as a string (like `user_settings`), a monotonic
//! `version`, and a BSON `expires_at` the sweep runs on. A standalone MongoDB
//! has no multi-document transactions, so — exactly as `user_settings` does —
//! every conditional write is a single version-conditioned update or an
//! insert whose duplicate-key error is the "already exists" answer: one of
//! two nodes writing the same row changes it, the other learns so from the
//! matched count.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use helios_auth::AuthError;
use helios_auth::session::{PersistedPending, PersistedSession, SaveOutcome, SessionPersistence};
use mongodb::bson::{Document, doc};
use serde::de::DeserializeOwned;

use super::MongoBackend;
use super::retry::retry_transient;
use super::user_settings::is_duplicate_key_error;

/// Name of the collection backing the login session store.
pub(crate) const LOGIN_SESSIONS_COLLECTION: &str = "login_sessions";

const SESSION: &str = "session";
const PENDING: &str = "pending";

/// Bound on retries when an *unconditional* session write loses the
/// version-compare race. Two nodes only ever contend on a refresh, and those
/// are conditional, so this is never approached.
const MAX_WRITE_RETRIES: usize = 8;

impl MongoBackend {
    async fn login_collection(&self) -> Result<mongodb::Collection<Document>, AuthError> {
        let db = self
            .get_database()
            .await
            .map_err(|e| store_err(e.to_string()))?;
        Ok(db.collection::<Document>(LOGIN_SESSIONS_COLLECTION))
    }

    async fn load_login_row<T: DeserializeOwned>(
        &self,
        kind: &str,
        id: &str,
    ) -> Result<Option<(T, i64)>, AuthError> {
        let collection = self.login_collection().await?;
        let row = retry_transient(|| async {
            collection.find_one(doc! { "_id": id, "kind": kind }).await
        })
        .await
        .map_err(|e| store_err(format!("read {kind}: {e}")))?;
        match row {
            None => Ok(None),
            Some(row) => {
                let data = row
                    .get_str("data")
                    .map_err(|e| store_err(format!("read {kind} data: {e}")))?;
                let document = serde_json::from_str(data)
                    .map_err(|e| store_err(format!("decode stored {kind}: {e}")))?;
                let version = row.get_i64("version").unwrap_or(0);
                Ok(Some((document, version)))
            }
        }
    }

    async fn delete_login_row(&self, kind: &str, id: &str) -> Result<bool, AuthError> {
        let collection = self.login_collection().await?;
        let result = retry_transient(|| async {
            collection
                .delete_one(doc! { "_id": id, "kind": kind })
                .await
        })
        .await
        .map_err(|e| store_err(format!("delete {kind}: {e}")))?;
        Ok(result.deleted_count > 0)
    }
}

/// The document for `id` as stored: the JSON `data` plus the sweep and audit
/// timestamps. `version` is set by the write that lands it.
fn login_document(
    id: &str,
    kind: &str,
    data: &str,
    version: i64,
    expires_at: DateTime<Utc>,
) -> Document {
    doc! {
        "_id": id,
        "kind": kind,
        "data": data,
        "version": version,
        "expires_at": bson_datetime(expires_at),
        "updated_at": bson_datetime(Utc::now()),
    }
}

fn bson_datetime(at: DateTime<Utc>) -> mongodb::bson::DateTime {
    mongodb::bson::DateTime::from_millis(at.timestamp_millis())
}

async fn current_version(
    collection: &mongodb::Collection<Document>,
    id: &str,
) -> Result<i64, AuthError> {
    let row = retry_transient(|| async {
        collection
            .find_one(doc! { "_id": id, "kind": SESSION })
            .await
    })
    .await
    .map_err(|e| store_err(format!("reload session version: {e}")))?;
    Ok(row.and_then(|d| d.get_i64("version").ok()).unwrap_or(0))
}

#[async_trait]
impl SessionPersistence for MongoBackend {
    async fn load_session(&self, id: &str) -> Result<Option<PersistedSession>, AuthError> {
        Ok(self
            .load_login_row::<PersistedSession>(SESSION, id)
            .await?
            .map(|(mut session, version)| {
                session.version = version;
                session
            }))
    }

    async fn save_session(
        &self,
        session: &PersistedSession,
        if_version: Option<i64>,
    ) -> Result<SaveOutcome, AuthError> {
        let collection = self.login_collection().await?;
        let data = serde_json::to_string(session)
            .map_err(|e| store_err(format!("encode session: {e}")))?;
        let expires_at = session.expires_at();

        for _ in 0..=MAX_WRITE_RETRIES {
            let current = match if_version {
                Some(expected) => expected,
                None => current_version(&collection, &session.id).await?,
            };
            let next = current + 1;

            if current == 0 {
                let inserted = retry_transient(|| async {
                    collection
                        .insert_one(login_document(
                            &session.id,
                            SESSION,
                            &data,
                            next,
                            expires_at,
                        ))
                        .await
                })
                .await;
                match inserted {
                    Ok(_) => return Ok(SaveOutcome::Saved(next)),
                    Err(e) if is_duplicate_key_error(&e) => {
                        if if_version.is_some() {
                            let current = current_version(&collection, &session.id).await?;
                            return Ok(SaveOutcome::Conflict { current });
                        }
                        continue;
                    }
                    Err(e) => return Err(store_err(format!("create session: {e}"))),
                }
            }

            let result = retry_transient(|| async {
                collection
                    .update_one(
                        doc! { "_id": &session.id, "kind": SESSION, "version": current },
                        doc! { "$set": {
                            "data": &data,
                            "version": next,
                            "expires_at": bson_datetime(expires_at),
                            "updated_at": bson_datetime(Utc::now()),
                        }},
                    )
                    .await
            })
            .await
            .map_err(|e| store_err(format!("update session: {e}")))?;
            if result.matched_count == 1 {
                return Ok(SaveOutcome::Saved(next));
            }
            if if_version.is_some() {
                let current = current_version(&collection, &session.id).await?;
                return Ok(SaveOutcome::Conflict { current });
            }
        }
        Err(store_err(
            "session write lost the version race too many times".to_string(),
        ))
    }

    async fn delete_session(&self, id: &str) -> Result<(), AuthError> {
        self.delete_login_row(SESSION, id).await.map(|_| ())
    }

    async fn load_pending(&self, id: &str) -> Result<Option<PersistedPending>, AuthError> {
        Ok(self
            .load_login_row::<PersistedPending>(PENDING, id)
            .await?
            .map(|(pending, _)| pending))
    }

    async fn save_pending(&self, pending: &PersistedPending) -> Result<(), AuthError> {
        let collection = self.login_collection().await?;
        let data = serde_json::to_string(pending)
            .map_err(|e| store_err(format!("encode pending login: {e}")))?;
        let document = login_document(&pending.id, PENDING, &data, 1, pending.expires_at());
        retry_transient(|| async {
            collection
                .replace_one(doc! { "_id": &pending.id }, document.clone())
                .upsert(true)
                .await
        })
        .await
        .map_err(|e| store_err(format!("write pending login: {e}")))?;
        Ok(())
    }

    async fn delete_pending(&self, id: &str) -> Result<bool, AuthError> {
        self.delete_login_row(PENDING, id).await
    }

    async fn sweep(&self, now: DateTime<Utc>) -> Result<u64, AuthError> {
        let collection = self.login_collection().await?;
        let result = retry_transient(|| async {
            collection
                .delete_many(doc! { "expires_at": { "$lt": bson_datetime(now) } })
                .await
        })
        .await
        .map_err(|e| store_err(format!("sweep: {e}")))?;
        Ok(result.deleted_count)
    }
}

fn store_err(message: String) -> AuthError {
    AuthError::InternalError(format!("mongodb login_sessions: {message}"))
}
