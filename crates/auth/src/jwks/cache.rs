use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use jsonwebtoken::DecodingKey;
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, info, warn};

use super::fetcher::JwksFetcher;
use crate::config::redact_url;
use crate::error::AuthError;

/// Default cache TTL when no Cache-Control header is present.
const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(3600);

/// Caches JWKS keys with Cache-Control awareness and background refresh.
///
/// An unknown `kid` triggers at most one fetch per `min_refresh_interval`,
/// whatever the outcome: the interval counts from the attempt, not from the
/// last success, and lookups that arrive while a fetch is in flight wait for
/// it instead of starting their own. The `kid` comes from an unverified
/// bearer, so otherwise every such request would cost the identity provider
/// one fetch.
pub struct JwksCache {
    keys: Arc<RwLock<HashMap<String, DecodingKey>>>,
    jwks_url: String,
    fetcher: JwksFetcher,
    expires_at: Arc<RwLock<Instant>>,
    /// When a fetch was last started, by a lookup or by the background task.
    last_attempt: Arc<RwLock<Instant>>,
    /// Held for the duration of a lookup-triggered fetch.
    refresh_gate: Mutex<()>,
    /// Bumped when a lookup-triggered fetch ends, however it ends (success,
    /// failure, or the request that ran it being dropped). A lookup that saw
    /// it change while waiting on `refresh_gate` rides on that fetch.
    fetches_ended: AtomicU64,
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
            last_attempt: Arc::new(RwLock::new(
                Instant::now() - Duration::from_secs(min_refresh_interval_secs + 1),
            )),
            refresh_gate: Mutex::new(()),
            fetches_ended: AtomicU64::new(0),
            min_refresh_interval: Duration::from_secs(min_refresh_interval_secs),
        }
    }

    /// Perform the initial JWKS fetch. Must be called before serving requests.
    ///
    /// Also spawns a background task to refresh keys before expiry.
    pub async fn initial_fetch(&self) -> Result<(), AuthError> {
        // Redacted: a JWKS URL may carry userinfo or a query credential (#2149).
        info!(url = %redact_url(&self.jwks_url), "Performing initial JWKS fetch");
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

    /// Attempt a refresh: join a fetch in flight, else respect the interval.
    ///
    /// The gate is taken before the interval check. `last_attempt` is armed
    /// when a fetch starts, so checking it first would turn away the lookups
    /// that arrive mid-fetch, including legitimate tokens naming a key the
    /// identity provider just rotated in; waiting instead lets them read the
    /// keys that fetch brings back. Comparing `fetches_ended` rather than the
    /// clock also coalesces concurrent lookups when the interval is zero.
    async fn try_refresh(&self) -> Result<(), AuthError> {
        let seen = self.fetches_ended.load(Ordering::Acquire);
        let _flight = self.refresh_gate.lock().await;
        if self.fetches_ended.load(Ordering::Acquire) != seen {
            debug!("JWKS fetch finished while waiting, reading its keys");
            return Ok(());
        }
        if self.attempted_within_interval().await {
            debug!("JWKS refresh rate-limited, skipping");
            return Ok(());
        }
        struct Ended<'a>(&'a AtomicU64);
        impl Drop for Ended<'_> {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::Release);
            }
        }
        let _ended = Ended(&self.fetches_ended);
        self.refresh().await
    }

    async fn attempted_within_interval(&self) -> bool {
        let last = self.last_attempt.read().await;
        Instant::now().duration_since(*last) < self.min_refresh_interval
    }

    /// Fetch keys from the JWKS endpoint and update the cache. The attempt
    /// arms the refresh interval before its outcome is known, so a failing
    /// identity provider is asked again no sooner than a healthy one.
    async fn refresh(&self) -> Result<(), AuthError> {
        {
            let mut last = self.last_attempt.write().await;
            *last = Instant::now();
        }
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

        info!(ttl_secs = ttl.as_secs(), "JWKS cache refreshed");
        Ok(())
    }

    /// Spawn a background task that refreshes keys before they expire.
    fn spawn_background_refresh(&self) {
        let keys = Arc::clone(&self.keys);
        let expires_at = Arc::clone(&self.expires_at);
        let last_attempt = Arc::clone(&self.last_attempt);
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

                debug!(url = %redact_url(&url), "Background JWKS refresh triggered");
                {
                    let mut l = last_attempt.write().await;
                    *l = Instant::now();
                }
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
