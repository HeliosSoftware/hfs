use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use jsonwebtoken::DecodingKey;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use super::coordination::{FetchedJwks, JwksCoordination, JwksCoordinationError, JwksFetchFn};
use super::fetcher::JwksFetcher;
use crate::error::AuthError;

/// Default cache TTL when no Cache-Control header is present. Also the
/// staleness cap for reusing a cluster-stored document.
const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(3600);

/// Caches JWKS keys with Cache-Control awareness and background refresh.
///
/// Cheaply cloneable — all state is shared behind `Arc`s, so a clone is
/// another handle onto the same cache (used by the background refresh task).
///
/// When a [`JwksCoordination`] is attached ([`set_coordination`](Self::set_coordination)),
/// every refresh goes through cluster-wide single-flight: one instance
/// fetches from the IdP, the rest adopt the shared document. Coordination
/// failures fall back to a direct fetch — auth availability outranks the
/// dedupe optimization.
#[derive(Clone)]
pub struct JwksCache {
    keys: Arc<RwLock<HashMap<String, DecodingKey>>>,
    jwks_url: String,
    fetcher: JwksFetcher,
    expires_at: Arc<RwLock<Instant>>,
    last_refresh: Arc<RwLock<Instant>>,
    min_refresh_interval: Duration,
    /// Set at most once, after construction (the server wraps the cache in
    /// an `Arc` before storage — the coordination backend — exists).
    coordination: Arc<OnceLock<Arc<dyn JwksCoordination>>>,
    /// `fetched_at` of the coordinated document currently installed; the
    /// watermark passed on the next coordinated refresh.
    stored_watermark: Arc<RwLock<Option<DateTime<Utc>>>>,
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
            coordination: Arc::new(OnceLock::new()),
            stored_watermark: Arc::new(RwLock::new(None)),
        }
    }

    /// Attach a cluster coordination backend. Settable at most once; returns
    /// whether this call installed it. Call before `initial_fetch()` so even
    /// the boot fetch is coordinated.
    pub fn set_coordination(&self, coordination: Arc<dyn JwksCoordination>) -> bool {
        self.coordination.set(coordination).is_ok()
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

    /// Refresh the cache — the single refresh seam (the boot fetch, unknown
    /// `kid` misses, and the background task all come through here).
    ///
    /// With coordination attached the fetch is cluster-wide single-flight;
    /// without it (or when the coordination layer is unavailable) it is a
    /// direct IdP fetch.
    async fn refresh(&self) -> Result<(), AuthError> {
        let (keys, ttl) = match self.coordination.get() {
            Some(coordination) => self.coordinated_refresh(coordination.as_ref()).await?,
            None => self.direct_refresh().await?,
        };

        {
            let mut cached = self.keys.write().await;
            *cached = keys;
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

    /// Direct (uncoordinated) fetch + parse.
    async fn direct_refresh(&self) -> Result<(HashMap<String, DecodingKey>, Duration), AuthError> {
        let raw = self.fetcher.fetch_raw(&self.jwks_url).await?;
        let keys = JwksFetcher::parse_document(&raw.body)?;
        Ok((keys, raw.max_age.unwrap_or(DEFAULT_CACHE_TTL)))
    }

    /// Cluster-coordinated fetch: at most one instance hits the IdP; the
    /// rest adopt the shared document, shortening their local TTL by its
    /// age. `Unavailable` (shared store down) falls back to a direct fetch.
    async fn coordinated_refresh(
        &self,
        coordination: &dyn JwksCoordination,
    ) -> Result<(HashMap<String, DecodingKey>, Duration), AuthError> {
        let watermark = *self.stored_watermark.read().await;

        let fetcher = self.fetcher.clone();
        let url = self.jwks_url.clone();
        let fetch: JwksFetchFn = Box::new(move || {
            Box::pin(async move {
                let raw = fetcher.fetch_raw(&url).await?;
                Ok(FetchedJwks {
                    body: raw.body,
                    max_age: raw.max_age,
                })
            })
        });

        match coordination
            .refresh(&self.jwks_url, watermark, DEFAULT_CACHE_TTL, fetch)
            .await
        {
            Ok(doc) => {
                let keys = JwksFetcher::parse_document(&doc.body)?;
                if doc.age > Duration::ZERO {
                    info!(
                        age_secs = doc.age.as_secs(),
                        "Reused shared JWKS document from the cluster store"
                    );
                }
                // Shorten the local lifetime by the document's age; floor at
                // 1s so a boundary-aged document cannot spin the background
                // refresh loop.
                let ttl = doc
                    .max_age
                    .unwrap_or(DEFAULT_CACHE_TTL)
                    .saturating_sub(doc.age)
                    .max(Duration::from_secs(1));
                *self.stored_watermark.write().await = Some(doc.fetched_at);
                Ok((keys, ttl))
            }
            Err(JwksCoordinationError::Fetch(e)) => Err(e),
            Err(JwksCoordinationError::Unavailable(msg)) => {
                warn!(
                    error = %msg,
                    "JWKS coordination unavailable; falling back to a direct fetch"
                );
                let result = self.direct_refresh().await?;
                // A recovering store may hold an older document than the one
                // we just fetched — advance the watermark so it is never
                // installed over these keys.
                *self.stored_watermark.write().await = Some(Utc::now());
                Ok(result)
            }
        }
    }

    /// Spawn a background task that refreshes keys before they expire.
    fn spawn_background_refresh(&self) {
        let cache = self.clone();
        tokio::spawn(async move {
            loop {
                // Sleep until shortly before expiry
                let sleep_duration = {
                    let expires = cache.expires_at.read().await;
                    let now = Instant::now();
                    if *expires > now {
                        let remaining = *expires - now;
                        // Refresh at 75% of TTL to avoid edge cases
                        remaining.mul_f64(0.75)
                    } else {
                        cache.min_refresh_interval
                    }
                };

                tokio::time::sleep(sleep_duration).await;

                debug!(url = %cache.jwks_url, "Background JWKS refresh triggered");
                if let Err(e) = cache.refresh().await {
                    warn!(error = %e, "Background JWKS refresh failed, will retry");
                }
            }
        });
    }
}
