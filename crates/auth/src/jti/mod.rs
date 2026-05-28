pub mod memory;
#[cfg(feature = "redis")]
pub mod redis;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::error::AuthError;

/// Trait for JWT ID (jti) replay prevention.
///
/// Implementations store seen JTI values with TTLs matching
/// the token's expiration time.
#[async_trait]
pub trait JtiCache: Send + Sync + 'static {
    /// Check if a JTI has been seen before, and store it if not.
    ///
    /// Returns `true` if the JTI was already seen (replay detected).
    /// Returns `false` if the JTI is new (stored successfully).
    async fn check_and_store(
        &self,
        jti: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<bool, AuthError>;
}

/// JTI cache implementation which never treats tokens as replays.
///
/// This is intended for deployments where JWT IDs identify reusable bearer
/// access tokens rather than one-time client assertions.
#[derive(Debug, Clone, Copy, Default)]
pub struct DisabledJtiCache;

#[async_trait]
impl JtiCache for DisabledJtiCache {
    async fn check_and_store(
        &self,
        _jti: &str,
        _expires_at: DateTime<Utc>,
    ) -> Result<bool, AuthError> {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn disabled_cache_never_reports_replay() {
        let cache = DisabledJtiCache;
        let expires = Utc::now();

        assert!(!cache.check_and_store("same-jti", expires).await.unwrap());
        assert!(!cache.check_and_store("same-jti", expires).await.unwrap());
    }
}
