use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use moka::future::Cache;

use super::JtiCache;
use crate::error::AuthError;

/// In-memory JTI cache backed by moka.
///
/// Uses moka's time-to-live (TTL) to automatically evict entries
/// after tokens expire, preventing unbounded growth.
pub struct InMemoryJtiCache {
    cache: Cache<String, ()>,
}

impl InMemoryJtiCache {
    /// Create a new in-memory JTI cache.
    ///
    /// Entries are evicted after 1 hour (covers typical token lifetimes).
    /// Max capacity prevents unbounded growth.
    pub fn new() -> Self {
        let cache = Cache::builder()
            .max_capacity(100_000)
            .time_to_live(Duration::from_secs(3600))
            .build();
        Self { cache }
    }
}

impl Default for InMemoryJtiCache {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl JtiCache for InMemoryJtiCache {
    async fn check_and_store(
        &self,
        jti: &str,
        _expires_at: DateTime<Utc>,
    ) -> Result<bool, AuthError> {
        // Check if already present
        if self.cache.get(jti).await.is_some() {
            return Ok(true); // replay
        }

        // Store the JTI (TTL is set at cache level)
        self.cache.insert(jti.to_string(), ()).await;

        Ok(false) // not a replay
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration as ChronoDuration;

    #[tokio::test]
    async fn test_new_jti_not_replay() {
        let cache = InMemoryJtiCache::new();
        let expires = Utc::now() + ChronoDuration::hours(1);

        let replay = cache.check_and_store("jti-1", expires).await.unwrap();
        assert!(!replay);
    }

    #[tokio::test]
    async fn test_duplicate_jti_is_replay() {
        let cache = InMemoryJtiCache::new();
        let expires = Utc::now() + ChronoDuration::hours(1);

        let first = cache.check_and_store("jti-2", expires).await.unwrap();
        assert!(!first);

        let second = cache.check_and_store("jti-2", expires).await.unwrap();
        assert!(second);
    }

    #[tokio::test]
    async fn test_different_jtis_independent() {
        let cache = InMemoryJtiCache::new();
        let expires = Utc::now() + ChronoDuration::hours(1);

        let a = cache.check_and_store("jti-a", expires).await.unwrap();
        assert!(!a);

        let b = cache.check_and_store("jti-b", expires).await.unwrap();
        assert!(!b);
    }
}
