//! Cluster-coordinated single-flight document refresh (design doc §5 C2,
//! strategy §8 Phase 2).
//!
//! A generic "fetch once, share the result" primitive: N instances that all
//! need to refresh the same remote document (today: an IdP's JWKS key set)
//! coordinate through one shared store so exactly one of them performs the
//! upstream fetch and the rest reuse the stored copy. The store is keyed by
//! an opaque string (for JWKS, the endpoint URL) and holds the raw document
//! body — parsing stays with the caller, so this crate needs no knowledge of
//! what the document is.
//!
//! Deliberately **server-global, not tenant-scoped**: the documents cached
//! here are public upstream material shared by every tenant (JWKS public
//! keys), so there is no per-tenant row to isolate and the DoD wrong-tenant
//! row does not apply (methodology §6).
//!
//! Freshness uses a *watermark*, not just a staleness window: callers pass
//! the `fetched_at` of the document they already hold, and a stored document
//! only counts as reusable when it is strictly newer. This is what makes a
//! key-rotation race deterministic — the instance that lost the lock race
//! sees the winner's newer document and reuses it, while an instance asking
//! again with the *winner's* watermark forces a genuine refetch.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::error::StorageError;

/// A document as produced by the caller's fetch closure.
#[derive(Debug, Clone)]
pub struct FetchedDocument {
    /// The raw document body (opaque to the store).
    pub body: String,
    /// Upstream cache lifetime, when the source declared one
    /// (e.g. HTTP `Cache-Control: max-age`).
    pub max_age_secs: Option<u64>,
}

/// A document as returned from the shared store.
#[derive(Debug, Clone)]
pub struct StoredDocument {
    /// The raw document body.
    pub body: String,
    /// Upstream cache lifetime recorded at fetch time, if any.
    pub max_age_secs: Option<u64>,
    /// When the document was fetched, on the *store's* clock. This is the
    /// caller's next watermark.
    pub fetched_at: DateTime<Utc>,
    /// Age of the document at return time, on the store's clock — zero when
    /// this call performed the fetch. Callers should shorten their local
    /// cache lifetime by this much.
    pub age: Duration,
}

/// Future returned by a [`FetchFn`].
pub type FetchFuture = Pin<Box<dyn Future<Output = Result<FetchedDocument, String>> + Send>>;

/// The caller-supplied upstream fetch, invoked at most once per
/// [`ClusterRefreshCache::refresh_with`] call, and only when no reusable
/// stored document exists. Errors are opaque strings — the store only relays
/// them.
pub type FetchFn = Box<dyn FnOnce() -> FetchFuture + Send>;

/// Error from [`ClusterRefreshCache::refresh_with`].
#[derive(Debug)]
pub enum RefreshCacheError {
    /// The fetch closure failed while this caller held the lock. Nothing was
    /// stored; the lock was released.
    Fetch(String),
    /// The coordination machinery itself failed (pool, connection, SQL).
    /// Callers should fall back to their uncoordinated path.
    Storage(StorageError),
}

impl std::fmt::Display for RefreshCacheError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Fetch(msg) => write!(f, "upstream fetch failed: {msg}"),
            Self::Storage(e) => write!(f, "storage error: {e}"),
        }
    }
}

impl std::error::Error for RefreshCacheError {}

impl From<StorageError> for RefreshCacheError {
    fn from(e: StorageError) -> Self {
        Self::Storage(e)
    }
}

/// Whether a stored document may be returned instead of fetching.
///
/// Reusable iff it is strictly newer than the caller's watermark (when one is
/// given), no older than `max_stale`, and no older than its own declared
/// `max_age` (when the upstream declared one). Shared by every backend so the
/// contract cannot drift.
pub fn stored_is_reusable(
    fetched_at: DateTime<Utc>,
    age: Duration,
    max_age_secs: Option<u64>,
    newer_than: Option<DateTime<Utc>>,
    max_stale: Duration,
) -> bool {
    if let Some(watermark) = newer_than
        && fetched_at <= watermark
    {
        return false;
    }
    if age > max_stale {
        return false;
    }
    if let Some(max_age) = max_age_secs
        && age > Duration::from_secs(max_age)
    {
        return false;
    }
    true
}

/// The cluster-coordinated refresh store.
///
/// Single-flight contract: for a given `key`, at most one caller cluster-wide
/// runs its fetch closure at a time, and a caller that finds a reusable
/// stored document (per [`stored_is_reusable`]) never invokes its closure at
/// all.
#[async_trait]
pub trait ClusterRefreshCache: Send + Sync {
    /// Refreshes the document under `key` with cluster-wide single-flight.
    ///
    /// Under an exclusive cluster-wide lock on `key`: if the stored document
    /// is reusable for this caller ([`stored_is_reusable`] with `newer_than`
    /// and `max_stale`), returns it without invoking `fetch`; otherwise
    /// invokes `fetch`, persists the result, and returns it with
    /// `age == 0`. A fetch failure releases the lock without storing
    /// anything and surfaces as [`RefreshCacheError::Fetch`].
    async fn refresh_with(
        &self,
        key: &str,
        newer_than: Option<DateTime<Utc>>,
        max_stale: Duration,
        fetch: FetchFn,
    ) -> Result<StoredDocument, RefreshCacheError>;
}

/// Test support: an in-memory [`ClusterRefreshCache`] implementing the same
/// freshness and single-flight contract as the database backends.
///
/// NOT a cluster-safe production backend — two instances would each have
/// their own map. It exists so trait consumers can be unit-tested without a
/// database and so the contract has a T1 reference model. Single-flight is
/// modeled by holding the map mutex across the fetch await.
pub mod testing {
    use tokio::sync::Mutex;

    use super::*;

    struct StoredEntry {
        body: String,
        max_age_secs: Option<u64>,
        fetched_at: DateTime<Utc>,
    }

    /// See [module docs](self).
    #[derive(Default)]
    pub struct InMemoryClusterRefreshCache {
        entries: Mutex<HashMap<String, StoredEntry>>,
    }

    impl InMemoryClusterRefreshCache {
        /// Creates an empty store.
        pub fn new() -> Self {
            Self::default()
        }

        /// Test-only: seeds a document with an explicit (typically past)
        /// `fetched_at`, so staleness paths can be exercised without sleeps.
        pub async fn insert_backdated(
            &self,
            key: &str,
            body: &str,
            max_age_secs: Option<u64>,
            fetched_at: DateTime<Utc>,
        ) {
            self.entries.lock().await.insert(
                key.to_string(),
                StoredEntry {
                    body: body.to_string(),
                    max_age_secs,
                    fetched_at,
                },
            );
        }
    }

    #[async_trait]
    impl ClusterRefreshCache for InMemoryClusterRefreshCache {
        async fn refresh_with(
            &self,
            key: &str,
            newer_than: Option<DateTime<Utc>>,
            max_stale: Duration,
            fetch: FetchFn,
        ) -> Result<StoredDocument, RefreshCacheError> {
            // Held across the fetch await: that IS the single-flight model.
            let mut entries = self.entries.lock().await;

            if let Some(entry) = entries.get(key) {
                let age = (Utc::now() - entry.fetched_at)
                    .to_std()
                    .unwrap_or(Duration::ZERO);
                if stored_is_reusable(
                    entry.fetched_at,
                    age,
                    entry.max_age_secs,
                    newer_than,
                    max_stale,
                ) {
                    return Ok(StoredDocument {
                        body: entry.body.clone(),
                        max_age_secs: entry.max_age_secs,
                        fetched_at: entry.fetched_at,
                        age,
                    });
                }
            }

            let fetched = fetch().await.map_err(RefreshCacheError::Fetch)?;
            let fetched_at = Utc::now();
            entries.insert(
                key.to_string(),
                StoredEntry {
                    body: fetched.body.clone(),
                    max_age_secs: fetched.max_age_secs,
                    fetched_at,
                },
            );
            Ok(StoredDocument {
                body: fetched.body,
                max_age_secs: fetched.max_age_secs,
                fetched_at,
                age: Duration::ZERO,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::testing::InMemoryClusterRefreshCache;
    use super::*;

    const HOUR: Duration = Duration::from_secs(3600);

    /// A fetch closure returning `body` and counting its invocations.
    fn counting_fetch(body: &str, max_age_secs: Option<u64>, hits: &Arc<AtomicUsize>) -> FetchFn {
        let body = body.to_string();
        let hits = Arc::clone(hits);
        Box::new(move || {
            Box::pin(async move {
                hits.fetch_add(1, Ordering::SeqCst);
                Ok(FetchedDocument { body, max_age_secs })
            })
        })
    }

    fn failing_fetch(msg: &str, hits: &Arc<AtomicUsize>) -> FetchFn {
        let msg = msg.to_string();
        let hits = Arc::clone(hits);
        Box::new(move || {
            Box::pin(async move {
                hits.fetch_add(1, Ordering::SeqCst);
                Err(msg)
            })
        })
    }

    /// No stored document → fetch, store, return with age zero.
    #[tokio::test]
    async fn t1_absent_fetches_and_stores() {
        let store = InMemoryClusterRefreshCache::new();
        let hits = Arc::new(AtomicUsize::new(0));

        let doc = store
            .refresh_with("k", None, HOUR, counting_fetch("v1", Some(600), &hits))
            .await
            .unwrap();
        assert_eq!(doc.body, "v1");
        assert_eq!(doc.max_age_secs, Some(600));
        assert_eq!(doc.age, Duration::ZERO);
        assert_eq!(hits.load(Ordering::SeqCst), 1);

        // A second caller with no watermark reuses it without fetching.
        let doc2 = store
            .refresh_with("k", None, HOUR, counting_fetch("v2", None, &hits))
            .await
            .unwrap();
        assert_eq!(doc2.body, "v1");
        assert_eq!(doc2.fetched_at, doc.fetched_at);
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    /// A caller holding the stored document itself (watermark == fetched_at)
    /// forces a refetch; a caller with an older watermark reuses.
    #[tokio::test]
    async fn t1_watermark_distinguishes_own_doc_from_newer() {
        let store = InMemoryClusterRefreshCache::new();
        let hits = Arc::new(AtomicUsize::new(0));

        let first = store
            .refresh_with("k", None, HOUR, counting_fetch("v1", None, &hits))
            .await
            .unwrap();

        // "I already have v1; give me something newer" → refetch.
        let second = store
            .refresh_with(
                "k",
                Some(first.fetched_at),
                HOUR,
                counting_fetch("v2", None, &hits),
            )
            .await
            .unwrap();
        assert_eq!(second.body, "v2");
        assert_eq!(hits.load(Ordering::SeqCst), 2);
        assert!(second.fetched_at > first.fetched_at);

        // A caller still on the v1 watermark sees v2 as newer → reuse.
        let third = store
            .refresh_with(
                "k",
                Some(first.fetched_at),
                HOUR,
                counting_fetch("v3", None, &hits),
            )
            .await
            .unwrap();
        assert_eq!(third.body, "v2");
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }

    /// A document older than `max_stale` is not reusable.
    #[tokio::test]
    async fn t1_backdated_beyond_max_stale_refetches() {
        let store = InMemoryClusterRefreshCache::new();
        let hits = Arc::new(AtomicUsize::new(0));
        store
            .insert_backdated("k", "old", None, Utc::now() - chrono::Duration::hours(2))
            .await;

        let doc = store
            .refresh_with("k", None, HOUR, counting_fetch("new", None, &hits))
            .await
            .unwrap();
        assert_eq!(doc.body, "new");
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    /// A document older than its own declared max_age is not reusable, even
    /// inside the max_stale window.
    #[tokio::test]
    async fn t1_backdated_beyond_own_max_age_refetches() {
        let store = InMemoryClusterRefreshCache::new();
        let hits = Arc::new(AtomicUsize::new(0));
        store
            .insert_backdated(
                "k",
                "old",
                Some(60), // declared lifetime: 60s
                Utc::now() - chrono::Duration::seconds(120),
            )
            .await;

        let doc = store
            .refresh_with("k", None, HOUR, counting_fetch("new", None, &hits))
            .await
            .unwrap();
        assert_eq!(doc.body, "new");
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    /// A fetch failure surfaces as `Fetch`, stores nothing, and does not
    /// poison the key: the next caller's fetch succeeds.
    #[tokio::test]
    async fn t1_fetch_error_propagates_and_releases() {
        let store = InMemoryClusterRefreshCache::new();
        let hits = Arc::new(AtomicUsize::new(0));

        let err = store
            .refresh_with("k", None, HOUR, failing_fetch("idp down", &hits))
            .await
            .unwrap_err();
        assert!(matches!(err, RefreshCacheError::Fetch(ref m) if m == "idp down"));

        let doc = store
            .refresh_with("k", None, HOUR, counting_fetch("v1", None, &hits))
            .await
            .unwrap();
        assert_eq!(doc.body, "v1");
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }

    /// Keys are independent documents.
    #[tokio::test]
    async fn t1_keys_are_independent() {
        let store = InMemoryClusterRefreshCache::new();
        let hits = Arc::new(AtomicUsize::new(0));

        store
            .refresh_with("a", None, HOUR, counting_fetch("va", None, &hits))
            .await
            .unwrap();
        let doc_b = store
            .refresh_with("b", None, HOUR, counting_fetch("vb", None, &hits))
            .await
            .unwrap();
        assert_eq!(doc_b.body, "vb");
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }
}
