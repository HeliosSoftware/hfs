//! PostgreSQL implementation of the web UI's [`SessionPersistence`].
//!
//! One row per session or pending login in the `login_sessions` table: the
//! opaque id, a `kind` telling the two apart, the JSONB document, a monotonic
//! `version` for conditional writes, and a native `expires_at` the sweep runs
//! on. Every conditional write is a single statement — `INSERT … ON CONFLICT
//! DO NOTHING` for "must not exist yet", `UPDATE … WHERE version = $expected`
//! for "at this version" — so two nodes writing the same row need no lock to
//! be serialised: exactly one of them changes a row, and the other learns it
//! from the row count.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use helios_auth::AuthError;
use helios_auth::session::{PersistedPending, PersistedSession, SaveOutcome, SessionPersistence};
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::PostgresBackend;

const SESSION: &str = "session";
const PENDING: &str = "pending";

impl PostgresBackend {
    async fn load_login_row<T: DeserializeOwned>(
        &self,
        kind: &str,
        id: &str,
    ) -> Result<Option<(T, i64)>, AuthError> {
        let client = self
            .get_client()
            .await
            .map_err(|e| store_err(e.to_string()))?;
        let row = client
            .query_opt(
                "SELECT data, version FROM login_sessions WHERE id = $1 AND kind = $2",
                &[&id, &kind],
            )
            .await
            .map_err(|e| store_err(format!("read {kind}: {e}")))?;
        match row {
            None => Ok(None),
            Some(row) => {
                let data: Value = row.get(0);
                let version: i64 = row.get(1);
                let document = serde_json::from_value(data)
                    .map_err(|e| store_err(format!("decode stored {kind}: {e}")))?;
                Ok(Some((document, version)))
            }
        }
    }

    async fn delete_login_row(&self, kind: &str, id: &str) -> Result<bool, AuthError> {
        let client = self
            .get_client()
            .await
            .map_err(|e| store_err(e.to_string()))?;
        let removed = client
            .execute(
                "DELETE FROM login_sessions WHERE id = $1 AND kind = $2",
                &[&id, &kind],
            )
            .await
            .map_err(|e| store_err(format!("delete {kind}: {e}")))?;
        Ok(removed > 0)
    }
}

#[async_trait]
impl SessionPersistence for PostgresBackend {
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
        let client = self
            .get_client()
            .await
            .map_err(|e| store_err(e.to_string()))?;
        let data =
            serde_json::to_value(session).map_err(|e| store_err(format!("encode session: {e}")))?;
        let expires_at = session.expires_at();
        let now = Utc::now();
        let written = match if_version {
            None => client
                .query_opt(
                    "INSERT INTO login_sessions (id, kind, data, version, expires_at, updated_at)
                     VALUES ($1, $2, $3, 1, $4, $5)
                     ON CONFLICT (id) DO UPDATE SET
                         kind = EXCLUDED.kind, data = EXCLUDED.data,
                         version = login_sessions.version + 1,
                         expires_at = EXCLUDED.expires_at, updated_at = EXCLUDED.updated_at
                     RETURNING version",
                    &[&session.id, &SESSION, &data, &expires_at, &now],
                )
                .await
                .map_err(|e| store_err(format!("write session: {e}")))?,
            Some(0) => client
                .query_opt(
                    "INSERT INTO login_sessions (id, kind, data, version, expires_at, updated_at)
                     VALUES ($1, $2, $3, 1, $4, $5)
                     ON CONFLICT (id) DO NOTHING
                     RETURNING version",
                    &[&session.id, &SESSION, &data, &expires_at, &now],
                )
                .await
                .map_err(|e| store_err(format!("create session: {e}")))?,
            Some(expected) => client
                .query_opt(
                    "UPDATE login_sessions
                     SET data = $3, version = version + 1, expires_at = $4, updated_at = $5
                     WHERE id = $1 AND kind = $2 AND version = $6
                     RETURNING version",
                    &[&session.id, &SESSION, &data, &expires_at, &now, &expected],
                )
                .await
                .map_err(|e| store_err(format!("update session: {e}")))?,
        };
        if let Some(row) = written {
            return Ok(SaveOutcome::Saved(row.get(0)));
        }
        let current = client
            .query_opt(
                "SELECT version FROM login_sessions WHERE id = $1 AND kind = $2",
                &[&session.id, &SESSION],
            )
            .await
            .map_err(|e| store_err(format!("read session version: {e}")))?
            .map(|row| row.get::<_, i64>(0))
            .unwrap_or(0);
        Ok(SaveOutcome::Conflict { current })
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
        let client = self
            .get_client()
            .await
            .map_err(|e| store_err(e.to_string()))?;
        let data = serde_json::to_value(pending)
            .map_err(|e| store_err(format!("encode pending login: {e}")))?;
        client
            .execute(
                "INSERT INTO login_sessions (id, kind, data, version, expires_at, updated_at)
                 VALUES ($1, $2, $3, 1, $4, $5)
                 ON CONFLICT (id) DO UPDATE SET
                     kind = EXCLUDED.kind, data = EXCLUDED.data, version = 1,
                     expires_at = EXCLUDED.expires_at, updated_at = EXCLUDED.updated_at",
                &[
                    &pending.id,
                    &PENDING,
                    &data,
                    &pending.expires_at(),
                    &Utc::now(),
                ],
            )
            .await
            .map_err(|e| store_err(format!("write pending login: {e}")))?;
        Ok(())
    }

    async fn delete_pending(&self, id: &str) -> Result<bool, AuthError> {
        self.delete_login_row(PENDING, id).await
    }

    async fn sweep(&self, now: DateTime<Utc>) -> Result<u64, AuthError> {
        let client = self
            .get_client()
            .await
            .map_err(|e| store_err(e.to_string()))?;
        client
            .execute("DELETE FROM login_sessions WHERE expires_at < $1", &[&now])
            .await
            .map_err(|e| store_err(format!("sweep: {e}")))
    }
}

fn store_err(message: String) -> AuthError {
    AuthError::InternalError(format!("postgres login_sessions: {message}"))
}
