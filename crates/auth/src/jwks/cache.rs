use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use jsonwebtoken::DecodingKey;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use super::fetcher::JwksFetcher;
use crate::error::AuthError;

/// Default cache TTL when no Cache-Control header is present.
const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(3600);

/// Caches JWKS keys with Cache-Control awareness and background refresh.
pub struct JwksCache {
    keys: Arc<RwLock<HashMap<String, DecodingKey>>>,
    jwks_url: String,
    fetcher: JwksFetcher,
    expires_at: Arc<RwLock<Instant>>,
    last_refresh: Arc<RwLock<Instant>>,
    min_refresh_interval: Duration,
}

impl JwksCache {
    /// Create a new JWKS cache.
    ///
    /// Does not fetch keys — call `initial_fetch()` before use.
    pub fn new(jwks_url: &str, min_refresh_interval_secs: u64) -> Self {
        Self {
            keys: Arc::new(RwLock::new(HashMap::new())),
            jwks_url: jwks_url.to_string(),
            fetcher: JwksFetcher::new(),
            expires_at: Arc::new(RwLock::new(Instant::now())),
            last_refresh: Arc::new(RwLock::new(
                Instant::now() - Duration::from_secs(min_refresh_interval_secs + 1),
            )),
            min_refresh_interval: Duration::from_secs(min_refresh_interval_secs),
        }
    }

    /// Perform the initial JWKS fetch. Must be called before serving requests.
    ///
    /// Also spawns a background task to refresh keys before expiry.
    pub async fn initial_fetch(&self) -> Result<(), AuthError> {
        info!(url = %self.jwks_url, "Performing initial JWKS fetch");
        self.refresh().await?;
        self.spawn_background_refresh();
        Ok(())
    }

    /// Get the decoding key for a given key ID.
    ///
    /// If the `kid` is not found, triggers a rate-limited refresh and retries.
    pub async fn get_key(&self, kid: &str) -> Result<DecodingKey, AuthError> {
        // Try cached keys first
        {
            let keys = self.keys.read().await;
            if let Some(key) = keys.get(kid) {
                return Ok(key.clone());
            }
        }

        // Unknown kid — try refreshing (rate-limited)
        debug!(kid, "Key not found in cache, attempting refresh");
        self.try_refresh().await?;

        // Retry after refresh
        let keys = self.keys.read().await;
        keys.get(kid).cloned().ok_or_else(|| AuthError::UnknownKid {
            kid: kid.to_string(),
        })
    }

    /// Attempt a refresh, respecting the rate limit.
    async fn try_refresh(&self) -> Result<(), AuthError> {
        let now = Instant::now();
        {
            let last = self.last_refresh.read().await;
            if now.duration_since(*last) < self.min_refresh_interval {
                debug!("JWKS refresh rate-limited, skipping");
                return Ok(());
            }
        }
        self.refresh().await
    }

    /// Fetch keys from the JWKS endpoint and update the cache.
    async fn refresh(&self) -> Result<(), AuthError> {
        let response = self.fetcher.fetch(&self.jwks_url).await?;

        let ttl = response.max_age.unwrap_or(DEFAULT_CACHE_TTL);

        {
            let mut keys = self.keys.write().await;
            *keys = response.keys;
        }
        {
            let mut expires = self.expires_at.write().await;
            *expires = Instant::now() + ttl;
        }
        {
            let mut last = self.last_refresh.write().await;
            *last = Instant::now();
        }

        info!(ttl_secs = ttl.as_secs(), "JWKS cache refreshed");
        Ok(())
    }

    /// Spawn a background task that refreshes keys before they expire.
    fn spawn_background_refresh(&self) {
        let keys = Arc::clone(&self.keys);
        let expires_at = Arc::clone(&self.expires_at);
        let last_refresh = Arc::clone(&self.last_refresh);
        let url = self.jwks_url.clone();
        let fetcher = self.fetcher.clone();
        let min_interval = self.min_refresh_interval;

        tokio::spawn(async move {
            loop {
                // Sleep until shortly before expiry
                let sleep_duration = {
                    let expires = expires_at.read().await;
                    let now = Instant::now();
                    if *expires > now {
                        let remaining = *expires - now;
                        // Refresh at 75% of TTL to avoid edge cases
                        remaining.mul_f64(0.75)
                    } else {
                        min_interval
                    }
                };

                tokio::time::sleep(sleep_duration).await;

                debug!(url = %url, "Background JWKS refresh triggered");
                match fetcher.fetch(&url).await {
                    Ok(response) => {
                        let ttl = response.max_age.unwrap_or(DEFAULT_CACHE_TTL);
                        {
                            let mut k = keys.write().await;
                            *k = response.keys;
                        }
                        {
                            let mut e = expires_at.write().await;
                            *e = Instant::now() + ttl;
                        }
                        {
                            let mut l = last_refresh.write().await;
                            *l = Instant::now();
                        }
                        info!(ttl_secs = ttl.as_secs(), "Background JWKS refresh complete");
                    }
                    Err(e) => {
                        warn!(error = %e, "Background JWKS refresh failed, will retry");
                    }
                }
            }
        });
    }
}
