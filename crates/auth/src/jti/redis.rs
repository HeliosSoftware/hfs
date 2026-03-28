use async_trait::async_trait;
use chrono::{DateTime, Utc};

use super::JtiCache;
use crate::error::AuthError;

/// Redis-backed JTI cache for distributed deployments.
///
/// Uses `SET NX EX` for atomic check-and-store with TTL.
pub struct RedisJtiCache {
    client: redis::Client,
}

impl RedisJtiCache {
    /// Create a new Redis JTI cache.
    pub fn new(redis_url: &str) -> Result<Self, AuthError> {
        let client = redis::Client::open(redis_url)
            .map_err(|e| AuthError::InternalError(format!("Redis connection error: {}", e)))?;
        Ok(Self { client })
    }

    fn key(jti: &str) -> String {
        format!("hfs:jti:{}", jti)
    }
}

#[async_trait]
impl JtiCache for RedisJtiCache {
    async fn check_and_store(
        &self,
        jti: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<bool, AuthError> {
        let mut conn = self
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| AuthError::InternalError(format!("Redis connection error: {}", e)))?;

        let now = Utc::now();
        let ttl_secs = if expires_at > now {
            (expires_at - now).num_seconds().max(1) as u64
        } else {
            60 // expired tokens get a brief TTL
        };

        let key = Self::key(jti);

        // SET key value NX EX ttl — returns true if the key was set (new),
        // false if it already existed (replay).
        let was_set: bool = redis::cmd("SET")
            .arg(&key)
            .arg("1")
            .arg("NX")
            .arg("EX")
            .arg(ttl_secs)
            .query_async(&mut conn)
            .await
            .map(|v: Option<String>| v.is_some())
            .map_err(|e| AuthError::InternalError(format!("Redis SET error: {}", e)))?;

        Ok(!was_set) // true = replay (key already existed)
    }
}
