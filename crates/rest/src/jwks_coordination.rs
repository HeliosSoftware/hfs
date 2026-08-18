//! Bridges `helios-auth`'s [`JwksCoordination`] to `helios-persistence`'s
//! [`ClusterRefreshCache`] (cluster design §5 C2).
//!
//! The two traits are deliberately near-twins that cannot see each other:
//! auth stays free of database dependencies and persistence stays free of
//! auth. This crate depends on both, so the adapter lives here; the server
//! binary attaches it to the `JwksCache` when the database coordination mode
//! is selected (`HFS_AUTH_JWKS_COORDINATION`).

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use helios_auth::{
    AuthError, CoordinatedJwks, JwksCoordination, JwksCoordinationError, JwksFetchFn,
};
use helios_persistence::core::cluster_refresh_cache::{
    ClusterRefreshCache, FetchFn, FetchedDocument, RefreshCacheError,
};

/// [`JwksCoordination`] over a backend's shared refresh store (obtained from
/// `ResourceStorage::cluster_refresh_cache()`).
pub struct StoreJwksCoordination {
    store: Arc<dyn ClusterRefreshCache>,
}

impl StoreJwksCoordination {
    /// Wraps a shared refresh store.
    pub fn new(store: Arc<dyn ClusterRefreshCache>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl JwksCoordination for StoreJwksCoordination {
    async fn refresh(
        &self,
        jwks_url: &str,
        newer_than: Option<DateTime<Utc>>,
        max_stale: Duration,
        fetch: JwksFetchFn,
    ) -> Result<CoordinatedJwks, JwksCoordinationError> {
        // Auth's fetch closure -> the store's document closure. Errors cross
        // the boundary as strings (the store is deliberately auth-agnostic)
        // and are re-wrapped as JwksFetchError on the way back.
        let store_fetch: FetchFn = Box::new(move || {
            Box::pin(async move {
                let fetched = fetch().await.map_err(|e| e.to_string())?;
                Ok(FetchedDocument {
                    body: fetched.body,
                    max_age_secs: fetched.max_age.map(|d| d.as_secs()),
                })
            })
        });

        match self
            .store
            .refresh_with(jwks_url, newer_than, max_stale, store_fetch)
            .await
        {
            Ok(doc) => Ok(CoordinatedJwks {
                body: doc.body,
                max_age: doc.max_age_secs.map(Duration::from_secs),
                fetched_at: doc.fetched_at,
                age: doc.age,
            }),
            Err(RefreshCacheError::Fetch(msg)) => {
                Err(JwksCoordinationError::Fetch(AuthError::JwksFetchError(msg)))
            }
            Err(RefreshCacheError::Storage(e)) => {
                Err(JwksCoordinationError::Unavailable(e.to_string()))
            }
        }
    }
}
