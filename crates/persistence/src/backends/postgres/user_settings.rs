//! PostgreSQL implementation of the per-user [`SettingsStore`].
//!
//! Each user owns a single row in the `user_settings` table holding an opaque
//! JSONB document plus a monotonic `version` used for optimistic locking. Writes
//! take a transaction-scoped, two-part `pg_advisory_xact_lock` — this module's
//! [`USER_SETTINGS_LOCK_NAMESPACE`] paired with `hashtext(user_key)` — before
//! the read-modify-write, then run a `SELECT … FOR UPDATE` inside that same
//! transaction so concurrent updates to the same user serialize correctly and
//! the `If-Match` precondition is checked against the live row — the advisory
//! lock is what makes that true even for a user's very first write, where
//! `FOR UPDATE` has no row yet to lock (see [`PostgresBackend::write_settings`]).

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::core::user_settings::{
    SettingsStore, StoredUserSettings, apply_merge_patch, purge_tenant_subtree,
};
use crate::error::{BackendError, ConcurrencyError, StorageError, StorageResult};

use super::PostgresBackend;

/// First key of the two-part [`pg_advisory_xact_lock`][pg-advisory] this module
/// takes in [`PostgresBackend::write_settings`].
///
/// PostgreSQL keeps the single-bigint-key form (used by the migration lock in
/// [`schema::MIGRATION_LOCK_KEY`](super::schema)) and the two-int4-key form in
/// entirely separate lock spaces, so `user_settings`' locks can never collide
/// with the migration lock regardless of this constant's value. What this
/// namespace guards against is a *future* two-int4-key advisory lock elsewhere
/// in HFS: pairing every `user_settings` lock with this fixed first key
/// reserves it as this module's own sub-space of the two-key form, so a later
/// caller's second key — whatever it hashes to — can never land on a key this
/// module also holds. Arbitrary but must stay stable across releases: changing
/// it only changes which in-flight locks a rolling deploy's old and new
/// instances fail to recognize as the same key, which matters only while an
/// upgrade is in progress.
const USER_SETTINGS_LOCK_NAMESPACE: i32 = 0x4855_5354; // "HUST" (HFS User SeTtings)

impl PostgresBackend {
    /// Read-modify-write a user's settings document inside a single transaction,
    /// locking the row with `SELECT … FOR UPDATE`.
    ///
    /// `compute` receives the currently stored document (or `None` when the user
    /// has no settings yet) and returns the document to persist. The optimistic
    /// `if_match_version` precondition — where `Some(0)` asserts "does not yet
    /// exist" — is checked against the locked row before `compute` runs.
    ///
    /// Before that `SELECT`, this takes a transaction-scoped
    /// [`pg_advisory_xact_lock`][pg-advisory] keyed on
    /// `(USER_SETTINGS_LOCK_NAMESPACE, hashtext(user_key))`: `FOR UPDATE`
    /// only blocks on a row that already exists, so two concurrent writers
    /// creating the *same* user's first document both read `None`, both compute
    /// `new_version = 1`, and the `INSERT … ON CONFLICT DO UPDATE` loser
    /// silently clobbers the winner instead of hitting the `if_match_version`
    /// check. The advisory lock closes that gap by serializing the whole
    /// read-modify-write — including the case with no row to lock — per user
    /// key. It is released automatically on commit or rollback (the `_xact`
    /// variant), never needs an explicit unlock, and is cheap because
    /// PostgreSQL hashes the lock id into an in-memory table rather than
    /// touching disk. Under `READ COMMITTED` (this pool's default), the
    /// `SELECT` that runs once the lock is acquired sees a fresh snapshot that
    /// already includes whatever the previous holder committed, so the second
    /// writer correctly observes the row the first one just created.
    ///
    /// [pg-advisory]: https://www.postgresql.org/docs/current/explicit-locking.html#ADVISORY-LOCKS
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

        // `hashtext` folds the user key into an `int4` lock id computed by the
        // server, so every HFS instance — even a different version mid rolling
        // deploy — hashes a given `user_key` to the same id; see
        // `USER_SETTINGS_LOCK_NAMESPACE` for why it is paired with a fixed
        // first key. Collisions between unrelated users are possible but
        // harmless: they only cause writes to two different users to
        // serialize against each other on rare hash collisions, never a
        // correctness issue.
        txn.execute(
            "SELECT pg_advisory_xact_lock($1, hashtext($2))",
            &[&USER_SETTINGS_LOCK_NAMESPACE, &user_key],
        )
        .await
        .map_err(|e| backend_err(format!("lock user_settings: {e}")))?;

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

    /// Sweeps `tenant_id` out of every settings document **using the caller's
    /// transaction**, returning how many documents changed.
    ///
    /// Takes the transaction so the sweep commits atomically with the resource
    /// deletes in
    /// [`purge_tenant_data`](crate::core::ResourceStorage::purge_tenant_data):
    /// an offboarding must not be able to half-apply, leaving a tenant's saved
    /// queries behind after its records are gone. `FOR UPDATE` serialises against
    /// a concurrent `/_user/settings` write, which locks the same rows.
    ///
    /// Deliberately does *not* take [`write_settings`](Self::write_settings)'s
    /// per-user `pg_advisory_xact_lock` before its bulk `FOR UPDATE`: doing so
    /// would need one lock per existing row, for no benefit — a purge has no
    /// "row does not exist yet" case to protect against, since it only ever
    /// touches rows a `SELECT` already found. Skipping it also rules out a
    /// deadlock between the two paths. A writer only ever waits on its own
    /// resources, in order (its advisory lock, then its own row); a purge only
    /// ever waits on rows locked by another transaction. So the one lock a
    /// blocked writer can be holding while it waits — its advisory lock — is
    /// never something a purge waits on, which is what would be required to
    /// close a cycle.
    ///
    /// The edit is done on a parsed `Value` via the shared
    /// [`purge_tenant_subtree`] rather than with `jsonb #-`, for two reasons: all
    /// four backends then erase byte-identically, and a JSONB text path would
    /// need care for tenant ids containing `.` or `/`, both of which
    /// `admin_tenants::validate_tenant_id` permits.
    pub(crate) async fn purge_tenant_settings_in_txn(
        txn: &deadpool_postgres::Transaction<'_>,
        tenant_id: &str,
    ) -> StorageResult<u64> {
        let rows = txn
            .query(
                "SELECT user_key, data, version FROM user_settings FOR UPDATE",
                &[],
            )
            .await
            .map_err(|e| backend_err(format!("scan user_settings: {e}")))?;

        let mut changed = 0u64;
        for row in rows {
            let user_key: String = row.get(0);
            let mut document: Value = row.get(1);
            let version: i64 = row.get(2);
            if !purge_tenant_subtree(&mut document, tenant_id) {
                continue;
            }
            txn.execute(
                "UPDATE user_settings SET data = $2, version = $3, updated_at = $4 \
                 WHERE user_key = $1",
                &[&user_key, &document, &(version + 1), &Utc::now()],
            )
            .await
            .map_err(|e| backend_err(format!("purge user_settings: {e}")))?;
            changed += 1;
        }
        Ok(changed)
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

    async fn delete_settings(&self, user_key: &str) -> StorageResult<bool> {
        let client = self.get_client().await?;
        let removed = client
            .execute(
                "DELETE FROM user_settings WHERE user_key = $1",
                &[&user_key],
            )
            .await
            .map_err(|e| backend_err(format!("delete user_settings: {e}")))?;
        Ok(removed > 0)
    }

    async fn purge_tenant_settings(&self, tenant_id: &str) -> StorageResult<u64> {
        let mut client = self.get_client().await?;
        let txn = client
            .transaction()
            .await
            .map_err(|e| backend_err(format!("begin user_settings purge: {e}")))?;
        let changed = Self::purge_tenant_settings_in_txn(&txn, tenant_id).await?;
        txn.commit()
            .await
            .map_err(|e| backend_err(format!("commit user_settings purge: {e}")))?;
        Ok(changed)
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
