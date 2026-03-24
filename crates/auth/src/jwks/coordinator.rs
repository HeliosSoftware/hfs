use redis::AsyncCommands;
use std::time::Duration;
use tracing::{debug, info, warn};

use crate::error::AuthError;

/// Redis-based coordinator for JWKS refresh across multiple HFS instances.
///
/// Uses a Redis lock to ensure only one instance performs the JWKS fetch
/// at a time, preventing thundering herd on the IdP's JWKS endpoint.
pub struct JwksCoordinator {
    client: redis::Client,
    lock_key: String,
    lock_ttl: Duration,
}

impl JwksCoordinator {
    /// Create a new coordinator.
    ///
    /// * `redis_url` — Redis connection string
    /// * `lock_ttl` — How long the refresh lock is held (default: 30s)
    pub fn new(redis_url: &str, lock_ttl: Duration) -> Result<Self, AuthError> {
        let client = redis::Client::open(redis_url)
            .map_err(|e| AuthError::InternalError(format!("Redis connection error: {}", e)))?;
        Ok(Self {
            client,
            lock_key: "hfs:jwks:refresh_lock".to_string(),
            lock_ttl,
        })
    }

    /// Try to acquire the refresh lock.
    ///
    /// Returns `true` if this instance is the leader and should perform the refresh.
    pub async fn try_acquire_lock(&self) -> Result<bool, AuthError> {
        let mut conn = self
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| AuthError::InternalError(format!("Redis connection error: {}", e)))?;

        let acquired: bool = redis::cmd("SET")
            .arg(&self.lock_key)
            .arg("1")
            .arg("NX")
            .arg("EX")
            .arg(self.lock_ttl.as_secs())
            .query_async(&mut conn)
            .await
            .map(|v: Option<String>| v.is_some())
            .map_err(|e| AuthError::InternalError(format!("Redis SET error: {}", e)))?;

        if acquired {
            debug!("Acquired JWKS refresh lock");
        } else {
            debug!("Another instance holds the JWKS refresh lock");
        }

        Ok(acquired)
    }

    /// Store serialized JWKS keys in Redis for other instances to read.
    pub async fn store_keys(&self, keys_json: &str) -> Result<(), AuthError> {
        let mut conn = self
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| AuthError::InternalError(format!("Redis connection error: {}", e)))?;

        // Store keys with a TTL longer than the lock to allow non-leaders to read
        let _: () = conn
            .set_ex("hfs:jwks:keys", keys_json, self.lock_ttl.as_secs() * 4)
            .await
            .map_err(|e| AuthError::InternalError(format!("Redis SET error: {}", e)))?;

        info!("Stored JWKS keys in Redis");
        Ok(())
    }

    /// Read cached JWKS keys from Redis (for non-leader instances).
    pub async fn load_keys(&self) -> Result<Option<String>, AuthError> {
        let mut conn = self
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| AuthError::InternalError(format!("Redis connection error: {}", e)))?;

        let keys: Option<String> = conn
            .get("hfs:jwks:keys")
            .await
            .map_err(|e| AuthError::InternalError(format!("Redis GET error: {}", e)))?;

        Ok(keys)
    }
}
