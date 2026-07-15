//! [`ClusterRefreshCache`] implementation for PostgreSQL.
//!
//! Single-flight is a per-key transaction-scoped advisory lock
//! (`pg_advisory_xact_lock`): commit or rollback releases it, so there is no
//! manual-unlock hazard (contrast the session-level schema-init lock in
//! `schema.rs`, which must be released explicitly). The caller's fetch
//! closure runs *inside* the transaction — the connection is held for the
//! duration of the upstream fetch, which is bounded by the caller's HTTP
//! timeout and only happens on rotation/boot events.
//!
//! Freshness math uses the database clock on both write (`NOW()`) and read
//! (`now()` selected alongside the row), so instance clock skew is
//! irrelevant.

use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::core::cluster_refresh_cache::{
    ClusterRefreshCache, FetchFn, RefreshCacheError, StoredDocument, stored_is_reusable,
};
use crate::error::{BackendError, StorageError, StorageResult};

/// Advisory lock seed mixed into the per-key hash (ASCII "HFSRFRSH"), keeping
/// these locks disjoint from other advisory-lock users (e.g. the schema-init
/// lock).
const REFRESH_CACHE_LOCK_SEED: i64 = 0x4846_5352_4652_5348;

/// Pool-sharing [`ClusterRefreshCache`] handle for PostgreSQL.
///
/// Obtained via `ResourceStorage::cluster_refresh_cache()` on
/// [`super::PostgresBackend`] (the same seam shape as `cluster_job_store()`):
/// the backend is not `Clone`, so the store is a cheap handle over the shared
/// connection pool rather than an impl on the backend itself.
pub struct PgClusterRefreshCache {
    pool: deadpool_postgres::Pool,
}

impl PgClusterRefreshCache {
    /// Creates a store handle over the backend's connection pool.
    pub(crate) fn new(pool: deadpool_postgres::Pool) -> Self {
        Self { pool }
    }

    async fn get_client(&self) -> StorageResult<deadpool_postgres::Client> {
        self.pool.get().await.map_err(|e| {
            StorageError::Backend(BackendError::ConnectionFailed {
                backend_name: "postgres".to_string(),
                message: e.to_string(),
            })
        })
    }
}

fn internal_error(message: String) -> StorageError {
    StorageError::Backend(BackendError::Internal {
        backend_name: "postgres".to_string(),
        message,
        source: None,
    })
}

#[async_trait]
impl ClusterRefreshCache for PgClusterRefreshCache {
    async fn refresh_with(
        &self,
        key: &str,
        newer_than: Option<DateTime<Utc>>,
        max_stale: Duration,
        fetch: FetchFn,
    ) -> Result<StoredDocument, RefreshCacheError> {
        let mut client = self.get_client().await?;
        let txn = client
            .transaction()
            .await
            .map_err(|e| internal_error(format!("Failed to begin refresh txn: {}", e)))?;

        // Exclusive cluster-wide lock for this key; released on commit or on
        // the rollback implied by dropping the transaction on any early
        // return below.
        txn.execute(
            "SELECT pg_advisory_xact_lock(hashtextextended($1, $2))",
            &[&key, &REFRESH_CACHE_LOCK_SEED],
        )
        .await
        .map_err(|e| internal_error(format!("Failed to acquire refresh lock: {}", e)))?;

        let row = txn
            .query_opt(
                "SELECT body, max_age_secs, fetched_at, now()
                 FROM cluster_refresh_cache WHERE cache_key = $1",
                &[&key],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to read stored document: {}", e)))?;

        if let Some(row) = row {
            let body: String = row.get(0);
            let max_age_secs: Option<i64> = row.get(1);
            let fetched_at: DateTime<Utc> = row.get(2);
            let db_now: DateTime<Utc> = row.get(3);
            let age = (db_now - fetched_at).to_std().unwrap_or(Duration::ZERO);
            let max_age_secs = max_age_secs.map(|v| v.max(0) as u64);

            if stored_is_reusable(fetched_at, age, max_age_secs, newer_than, max_stale) {
                txn.commit()
                    .await
                    .map_err(|e| internal_error(format!("Failed to commit refresh txn: {}", e)))?;
                return Ok(StoredDocument {
                    body,
                    max_age_secs,
                    fetched_at,
                    age,
                });
            }
        }

        // Stale or absent — this caller is the single flight. A fetch error
        // returns before commit; the dropped transaction rolls back and
        // releases the lock without storing anything.
        let fetched = fetch().await.map_err(RefreshCacheError::Fetch)?;

        let max_age_param: Option<i64> = fetched
            .max_age_secs
            .map(|v| i64::try_from(v).unwrap_or(i64::MAX));
        let row = txn
            .query_one(
                "INSERT INTO cluster_refresh_cache (cache_key, body, max_age_secs, fetched_at)
                 VALUES ($1, $2, $3, NOW())
                 ON CONFLICT (cache_key) DO UPDATE
                 SET body = EXCLUDED.body,
                     max_age_secs = EXCLUDED.max_age_secs,
                     fetched_at = NOW()
                 RETURNING fetched_at",
                &[&key, &fetched.body, &max_age_param],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to store fetched document: {}", e)))?;
        let fetched_at: DateTime<Utc> = row.get(0);

        txn.commit()
            .await
            .map_err(|e| internal_error(format!("Failed to commit refresh txn: {}", e)))?;

        Ok(StoredDocument {
            body: fetched.body,
            max_age_secs: fetched.max_age_secs,
            fetched_at,
            age: Duration::ZERO,
        })
    }
}
