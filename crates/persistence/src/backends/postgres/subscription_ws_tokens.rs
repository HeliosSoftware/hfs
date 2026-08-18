//! [`WsBindingTokenStore`] implementation for PostgreSQL.
//!
//! Redeem-once by construction: redeeming is a single
//! `DELETE … RETURNING`, so exactly one of N racing redeemers gets the row.
//! Expiry is judged on the database clock (`expires_at > now()` evaluated in
//! the same statement), so instance clock skew is irrelevant and tests can
//! exercise the expired path with a zero TTL — no sleeps.

use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::core::ws_binding_tokens::WsBindingTokenStore;
use crate::error::{BackendError, StorageError, StorageResult};
use crate::tenant::TenantContext;

/// Pool-sharing [`WsBindingTokenStore`] handle for PostgreSQL.
///
/// Obtained via `ResourceStorage::ws_binding_token_store()` on
/// [`super::PostgresBackend`] (the same seam shape as `cluster_job_store()`).
pub struct PgWsBindingTokenStore {
    pool: deadpool_postgres::Pool,
}

impl PgWsBindingTokenStore {
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
impl WsBindingTokenStore for PgWsBindingTokenStore {
    async fn mint(
        &self,
        tenant: &TenantContext,
        subscription_id: &str,
        ttl: Duration,
    ) -> StorageResult<(String, DateTime<Utc>)> {
        let client = self.get_client().await?;

        // Lazy cleanup, mirroring the in-memory manager: tokens are ~30s
        // lived and low-volume, so piggybacking on mint keeps the table
        // tiny without a dedicated reaper.
        client
            .execute(
                "DELETE FROM subscription_ws_tokens WHERE expires_at < NOW()",
                &[],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to purge expired tokens: {}", e)))?;

        let token = uuid::Uuid::new_v4().to_string();
        let ttl_secs = ttl.as_secs_f64();
        let row = client
            .query_one(
                "INSERT INTO subscription_ws_tokens
                     (token, tenant_id, subscription_id, expires_at)
                 VALUES ($1, $2, $3, NOW() + $4 * INTERVAL '1 second')
                 RETURNING expires_at",
                &[
                    &token,
                    &tenant.tenant_id().as_str(),
                    &subscription_id,
                    &ttl_secs,
                ],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to mint binding token: {}", e)))?;
        Ok((token, row.get(0)))
    }

    async fn redeem(&self, token: &str) -> StorageResult<Option<(String, String)>> {
        let client = self.get_client().await?;
        let row = client
            .query_opt(
                "DELETE FROM subscription_ws_tokens
                 WHERE token = $1
                 RETURNING tenant_id, subscription_id, (expires_at > NOW()) AS live",
                &[&token],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to redeem binding token: {}", e)))?;
        Ok(row.and_then(|row| {
            let live: bool = row.get(2);
            live.then(|| (row.get(0), row.get(1)))
        }))
    }
}
