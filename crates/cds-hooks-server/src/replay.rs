//! Replay protection for the `jti` claim. A nonce is accepted once within its
//! token's lifetime; the second call carrying it is refused. The store has to
//! be shared by every instance of the service, or a replayed token lands on
//! another node and passes: the in-memory store is for one process and for
//! tests, the Postgres store is for a deployment.
//!
//! The store holds nonces and expiry times, nothing that names a client or a
//! patient.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod};
use tokio_postgres::NoTls;

/// Seconds a nonce stays refused after its token expired, matching the clock
/// skew the validator tolerates on `exp`.
pub const LEEWAY_SECS: u64 = 60;
/// Calls between two sweeps of expired nonces.
const PURGE_EVERY: u64 = 1_000;

#[derive(Debug, thiserror::Error)]
#[error("replay store: {0}")]
pub struct ReplayError(String);

#[async_trait]
pub trait ReplayStore: Send + Sync {
    /// Records the nonce; `Ok(true)` when it had not been seen within its
    /// lifetime, `Ok(false)` when it had. `exp` is the token's expiry, in
    /// seconds since the epoch.
    async fn first_use(&self, nonce: &str, exp: u64) -> Result<bool, ReplayError>;
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// One process's memory: enough for one instance and for tests.
#[derive(Default)]
pub struct MemoryReplayStore {
    seen: Mutex<HashMap<String, u64>>,
}

impl MemoryReplayStore {
    /// How many nonces are kept before expired ones are swept.
    const PURGE_AT: usize = 10_000;

    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl ReplayStore for MemoryReplayStore {
    async fn first_use(&self, nonce: &str, exp: u64) -> Result<bool, ReplayError> {
        let now = now_secs();
        let mut seen = self.seen.lock().expect("replay cache");
        if seen.len() >= Self::PURGE_AT {
            seen.retain(|_, exp| *exp + LEEWAY_SECS > now);
        }
        if let Some(exp) = seen.get(nonce)
            && *exp + LEEWAY_SECS > now
        {
            return Ok(false);
        }
        seen.insert(nonce.to_string(), exp);
        Ok(true)
    }
}

/// A table in Postgres shared by every instance: `cds_hooks_jti (nonce, exp)`.
/// Created on connect when missing.
pub struct PostgresReplayStore {
    pool: Pool,
    calls: AtomicU64,
}

impl PostgresReplayStore {
    pub async fn connect(url: &str) -> Result<Self, ReplayError> {
        let config = tokio_postgres::Config::from_str(url)
            .map_err(|e| ReplayError(format!("database url: {e}")))?;
        let manager = Manager::from_config(
            config,
            NoTls,
            ManagerConfig {
                recycling_method: RecyclingMethod::Fast,
            },
        );
        let pool = Pool::builder(manager)
            .max_size(4)
            .build()
            .map_err(|e| ReplayError(format!("pool: {e}")))?;
        let client = pool.get().await.map_err(|e| ReplayError(e.to_string()))?;
        client
            .batch_execute(
                "CREATE TABLE IF NOT EXISTS cds_hooks_jti (nonce TEXT PRIMARY KEY, exp BIGINT NOT NULL)",
            )
            .await
            .map_err(|e| ReplayError(e.to_string()))?;
        Ok(Self {
            pool,
            calls: AtomicU64::new(0),
        })
    }
}

#[async_trait]
impl ReplayStore for PostgresReplayStore {
    async fn first_use(&self, nonce: &str, exp: u64) -> Result<bool, ReplayError> {
        let now = now_secs() as i64;
        let leeway = LEEWAY_SECS as i64;
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| ReplayError(e.to_string()))?;
        if self
            .calls
            .fetch_add(1, Ordering::Relaxed)
            .is_multiple_of(PURGE_EVERY)
        {
            client
                .execute(
                    "DELETE FROM cds_hooks_jti WHERE exp + $1 <= $2",
                    &[&leeway, &now],
                )
                .await
                .map_err(|e| ReplayError(e.to_string()))?;
        }
        // Inserted, or an expired row taken over: first use. A live row: a replay.
        let rows = client
            .execute(
                "INSERT INTO cds_hooks_jti (nonce, exp) VALUES ($1, $2) \
                 ON CONFLICT (nonce) DO UPDATE SET exp = EXCLUDED.exp \
                 WHERE cds_hooks_jti.exp + $3 <= $4",
                &[&nonce, &(exp as i64), &leeway, &now],
            )
            .await
            .map_err(|e| ReplayError(e.to_string()))?;
        Ok(rows == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn memory_store_refuses_a_live_nonce_and_forgets_an_expired_one() {
        let store = MemoryReplayStore::new();
        let live = now_secs() + 300;
        assert!(store.first_use("a", live).await.unwrap());
        assert!(!store.first_use("a", live).await.unwrap());
        assert!(store.first_use("b", live).await.unwrap());

        let long_gone = now_secs().saturating_sub(LEEWAY_SECS + 10);
        assert!(store.first_use("c", long_gone).await.unwrap());
        assert!(store.first_use("c", live).await.unwrap());
    }

    /// Runs against the database `CDS_HOOKS_TEST_DATABASE_URL` names; skipped otherwise.
    #[tokio::test]
    async fn postgres_store_refuses_a_live_nonce_across_two_connections() {
        let Ok(url) = std::env::var("CDS_HOOKS_TEST_DATABASE_URL") else {
            eprintln!("CDS_HOOKS_TEST_DATABASE_URL not set; skipping");
            return;
        };
        let one = PostgresReplayStore::connect(&url).await.unwrap();
        let two = PostgresReplayStore::connect(&url).await.unwrap();
        let nonce = format!("test-{}", uuid::Uuid::new_v4());
        let live = now_secs() + 300;
        assert!(one.first_use(&nonce, live).await.unwrap());
        assert!(!two.first_use(&nonce, live).await.unwrap());
        assert!(!one.first_use(&nonce, live).await.unwrap());

        let stale = format!("test-{}", uuid::Uuid::new_v4());
        let long_gone = now_secs().saturating_sub(LEEWAY_SECS + 10);
        assert!(one.first_use(&stale, long_gone).await.unwrap());
        assert!(two.first_use(&stale, live).await.unwrap());
        assert!(!one.first_use(&stale, live).await.unwrap());
    }
}
