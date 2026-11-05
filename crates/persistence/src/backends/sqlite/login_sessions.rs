//! SQLite implementation of the web UI's [`SessionPersistence`].
//!
//! One row per session or pending login in the `login_sessions` table: the
//! opaque id, a `kind` telling the two apart, the JSON document, a monotonic
//! `version` for conditional writes, and the fixed-width RFC 3339 `expires_at`
//! the sweep runs on. The store never reads a session under a pending id or
//! the reverse, so a stray cookie value cannot be promoted across kinds.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use helios_auth::AuthError;
use helios_auth::session::{
    PersistedPending, PersistedSession, SaveOutcome, SessionPersistence, store_timestamp,
};
use rusqlite::{OptionalExtension, params};
use serde::de::DeserializeOwned;

use super::SqliteBackend;

const SESSION: &str = "session";
const PENDING: &str = "pending";

impl SqliteBackend {
    fn load_login_row<T: DeserializeOwned>(
        &self,
        kind: &str,
        id: &str,
    ) -> Result<Option<(T, i64)>, AuthError> {
        let conn = self
            .get_connection()
            .map_err(|e| store_err(e.to_string()))?;
        let row: Option<(String, i64)> = conn
            .query_row(
                "SELECT data, version FROM login_sessions WHERE id = ?1 AND kind = ?2",
                params![id, kind],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|e| store_err(format!("read {kind}: {e}")))?;
        match row {
            None => Ok(None),
            Some((data, version)) => {
                let document = serde_json::from_str(&data)
                    .map_err(|e| store_err(format!("decode stored {kind}: {e}")))?;
                Ok(Some((document, version)))
            }
        }
    }

    fn delete_login_row(&self, kind: &str, id: &str) -> Result<bool, AuthError> {
        let conn = self
            .get_connection()
            .map_err(|e| store_err(e.to_string()))?;
        let removed = conn
            .execute(
                "DELETE FROM login_sessions WHERE id = ?1 AND kind = ?2",
                params![id, kind],
            )
            .map_err(|e| store_err(format!("delete {kind}: {e}")))?;
        Ok(removed > 0)
    }
}

#[async_trait]
impl SessionPersistence for SqliteBackend {
    async fn load_session(&self, id: &str) -> Result<Option<PersistedSession>, AuthError> {
        Ok(self
            .load_login_row::<PersistedSession>(SESSION, id)?
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
        let mut conn = self
            .get_connection()
            .map_err(|e| store_err(e.to_string()))?;
        // IMMEDIATE for the same read-then-write/WAL reason as
        // `write_settings` in user_settings.rs.
        let txn = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| store_err(format!("begin session write: {e}")))?;
        let current: Option<i64> = txn
            .query_row(
                "SELECT version FROM login_sessions WHERE id = ?1 AND kind = ?2",
                params![session.id, SESSION],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| store_err(format!("read session version: {e}")))?;
        let current = current.unwrap_or(0);
        if let Some(expected) = if_version
            && expected != current
        {
            return Ok(SaveOutcome::Conflict { current });
        }
        let version = current + 1;
        let data = serde_json::to_string(session)
            .map_err(|e| store_err(format!("encode session: {e}")))?;
        txn.execute(
            "INSERT INTO login_sessions (id, kind, data, version, expires_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
                 kind = ?2, data = ?3, version = ?4, expires_at = ?5, updated_at = ?6",
            params![
                session.id,
                SESSION,
                data,
                version,
                store_timestamp(session.expires_at()),
                store_timestamp(Utc::now()),
            ],
        )
        .map_err(|e| store_err(format!("write session: {e}")))?;
        txn.commit()
            .map_err(|e| store_err(format!("commit session write: {e}")))?;
        Ok(SaveOutcome::Saved(version))
    }

    async fn delete_session(&self, id: &str) -> Result<(), AuthError> {
        self.delete_login_row(SESSION, id).map(|_| ())
    }

    async fn load_pending(&self, id: &str) -> Result<Option<PersistedPending>, AuthError> {
        Ok(self
            .load_login_row::<PersistedPending>(PENDING, id)?
            .map(|(pending, _)| pending))
    }

    async fn save_pending(&self, pending: &PersistedPending) -> Result<(), AuthError> {
        let conn = self
            .get_connection()
            .map_err(|e| store_err(e.to_string()))?;
        let data = serde_json::to_string(pending)
            .map_err(|e| store_err(format!("encode pending login: {e}")))?;
        conn.execute(
            "INSERT INTO login_sessions (id, kind, data, version, expires_at, updated_at)
             VALUES (?1, ?2, ?3, 1, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET
                 kind = ?2, data = ?3, version = 1, expires_at = ?4, updated_at = ?5",
            params![
                pending.id,
                PENDING,
                data,
                store_timestamp(pending.expires_at()),
                store_timestamp(Utc::now()),
            ],
        )
        .map_err(|e| store_err(format!("write pending login: {e}")))?;
        Ok(())
    }

    async fn delete_pending(&self, id: &str) -> Result<bool, AuthError> {
        self.delete_login_row(PENDING, id)
    }

    async fn sweep(&self, now: DateTime<Utc>) -> Result<u64, AuthError> {
        let conn = self
            .get_connection()
            .map_err(|e| store_err(e.to_string()))?;
        let removed = conn
            .execute(
                "DELETE FROM login_sessions WHERE expires_at < ?1",
                params![store_timestamp(now)],
            )
            .map_err(|e| store_err(format!("sweep: {e}")))?;
        Ok(removed as u64)
    }
}

fn store_err(message: String) -> AuthError {
    AuthError::InternalError(format!("sqlite login_sessions: {message}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeDelta;
    use helios_auth::SessionPrincipal;

    fn backend() -> SqliteBackend {
        let backend = SqliteBackend::in_memory().expect("in-memory backend");
        backend.init_schema().expect("init schema");
        backend
    }

    fn session(id: &str) -> PersistedSession {
        let now = Utc::now();
        PersistedSession {
            id: id.to_string(),
            principal: SessionPrincipal {
                subject: "demo-sub".to_string(),
                issuer: "https://idp".to_string(),
                name: Some("Demo User".to_string()),
                preferred_username: None,
                email: None,
                picture: None,
            },
            access_token: "at-1".to_string(),
            access_expires_at: now + TimeDelta::minutes(5),
            refresh_token: Some("rt-1".to_string()),
            id_token: None,
            last_seen: now,
            created_at: now,
            version: 0,
        }
    }

    fn pending(id: &str) -> PersistedPending {
        PersistedPending {
            id: id.to_string(),
            state: "state-1".to_string(),
            code_verifier: "verifier-1".to_string(),
            next: "/ui/resources".to_string(),
            started_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn a_session_round_trips_with_its_version() {
        let backend = backend();
        assert!(backend.load_session("s1").await.unwrap().is_none());

        let saved = backend.save_session(&session("s1"), Some(0)).await.unwrap();
        assert_eq!(saved, SaveOutcome::Saved(1));

        let loaded = backend.load_session("s1").await.unwrap().unwrap();
        assert_eq!(loaded.version, 1);
        assert_eq!(loaded.access_token, "at-1");
        assert_eq!(loaded.principal.display(), "Demo User");

        let mut newer = loaded.clone();
        newer.access_token = "at-2".to_string();
        assert_eq!(
            backend.save_session(&newer, Some(1)).await.unwrap(),
            SaveOutcome::Saved(2)
        );
        assert_eq!(
            backend
                .load_session("s1")
                .await
                .unwrap()
                .unwrap()
                .access_token,
            "at-2"
        );
    }

    #[tokio::test]
    async fn a_stale_version_is_a_conflict_and_writes_nothing() {
        let backend = backend();
        backend.save_session(&session("s1"), Some(0)).await.unwrap();
        let mut stale = session("s1");
        stale.access_token = "at-stale".to_string();
        assert_eq!(
            backend.save_session(&stale, Some(0)).await.unwrap(),
            SaveOutcome::Conflict { current: 1 }
        );
        assert_eq!(
            backend
                .load_session("s1")
                .await
                .unwrap()
                .unwrap()
                .access_token,
            "at-1"
        );
        // Unconditional writes still land.
        assert_eq!(
            backend.save_session(&stale, None).await.unwrap(),
            SaveOutcome::Saved(2)
        );
    }

    #[tokio::test]
    async fn delete_session_is_idempotent() {
        let backend = backend();
        backend.save_session(&session("s1"), None).await.unwrap();
        backend.delete_session("s1").await.unwrap();
        assert!(backend.load_session("s1").await.unwrap().is_none());
        backend.delete_session("s1").await.unwrap();
    }

    #[tokio::test]
    async fn a_pending_login_is_consumed_exactly_once() {
        let backend = backend();
        backend.save_pending(&pending("p1")).await.unwrap();
        let loaded = backend.load_pending("p1").await.unwrap().unwrap();
        assert_eq!(loaded.state, "state-1");
        assert_eq!(loaded.next, "/ui/resources");

        assert!(backend.delete_pending("p1").await.unwrap());
        assert!(!backend.delete_pending("p1").await.unwrap());
        assert!(backend.load_pending("p1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn kinds_do_not_cross() {
        let backend = backend();
        backend.save_pending(&pending("x")).await.unwrap();
        assert!(backend.load_session("x").await.unwrap().is_none());
        backend.delete_session("x").await.unwrap();
        assert!(backend.load_pending("x").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn sweep_drops_only_what_has_expired() {
        let backend = backend();
        let mut idle = session("idle");
        idle.last_seen = Utc::now() - TimeDelta::hours(9);
        backend.save_session(&idle, None).await.unwrap();
        backend.save_session(&session("live"), None).await.unwrap();
        let mut old = pending("old");
        old.started_at = Utc::now() - TimeDelta::minutes(11);
        backend.save_pending(&old).await.unwrap();
        backend.save_pending(&pending("fresh")).await.unwrap();

        assert_eq!(backend.sweep(Utc::now()).await.unwrap(), 2);
        assert!(backend.load_session("idle").await.unwrap().is_none());
        assert!(backend.load_session("live").await.unwrap().is_some());
        assert!(backend.load_pending("old").await.unwrap().is_none());
        assert!(backend.load_pending("fresh").await.unwrap().is_some());
    }
}
