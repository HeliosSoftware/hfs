//! Cluster-wide coordination of JWKS refreshes (cluster design §5 C2).
//!
//! Per-instance JWKS refresh is functionally correct — every node fetches
//! the same public keys — but N instances behind a load balancer all hit the
//! IdP's JWKS endpoint on boot and on every key rotation. A
//! [`JwksCoordination`] implementation provides cluster-wide single-flight:
//! at most one instance performs the upstream fetch for a given URL, and the
//! rest reuse the fetched document from a shared store.
//!
//! This crate defines only the trait; implementations live where the shared
//! infrastructure lives (the Postgres-backed adapter in `helios-rest` over
//! `helios-persistence`'s `ClusterRefreshCache`, and the Redis coordinator in
//! this crate behind the `redis` feature).
//!
//! Freshness is watermark-based: the caller passes the `fetched_at` of the
//! document it already holds, and a stored document only counts as reusable
//! when strictly newer. That is what lets the loser of a rotation race adopt
//! the winner's document while a caller re-asking with the winner's own
//! watermark forces a genuine refetch.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::error::AuthError;

/// A JWKS document as produced by the caller's fetch closure.
#[derive(Debug, Clone)]
pub struct FetchedJwks {
    /// The raw JWKS JSON document body.
    pub body: String,
    /// Upstream cache lifetime (`Cache-Control: max-age`), when declared.
    pub max_age: Option<Duration>,
}

/// A JWKS document as returned from the coordination store.
#[derive(Debug, Clone)]
pub struct CoordinatedJwks {
    /// The raw JWKS JSON document body — parse locally with
    /// [`JwksFetcher::parse_document`](super::JwksFetcher::parse_document).
    pub body: String,
    /// Upstream cache lifetime recorded at fetch time, if any.
    pub max_age: Option<Duration>,
    /// When the document was fetched, on the store's clock. This is the
    /// caller's next watermark.
    pub fetched_at: DateTime<Utc>,
    /// Age of the document at return time — zero when this call performed
    /// the fetch. Callers should shorten their local cache lifetime by this
    /// much.
    pub age: Duration,
}

/// Future returned by a [`JwksFetchFn`].
pub type JwksFetchFuture = Pin<Box<dyn Future<Output = Result<FetchedJwks, AuthError>> + Send>>;

/// The caller-supplied upstream fetch, invoked at most once per
/// [`JwksCoordination::refresh`] call, and only when no reusable stored
/// document exists.
pub type JwksFetchFn = Box<dyn FnOnce() -> JwksFetchFuture + Send>;

/// Error from [`JwksCoordination::refresh`].
#[derive(Debug)]
pub enum JwksCoordinationError {
    /// The upstream fetch itself failed while this caller held the lock —
    /// a real IdP failure, propagated as-is.
    Fetch(AuthError),
    /// The coordination layer failed (shared store unreachable). Callers
    /// should fall back to a direct, uncoordinated fetch: auth availability
    /// outranks the dedupe optimization.
    Unavailable(String),
}

impl std::fmt::Display for JwksCoordinationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Fetch(e) => write!(f, "JWKS fetch failed: {e}"),
            Self::Unavailable(msg) => write!(f, "JWKS coordination unavailable: {msg}"),
        }
    }
}

impl std::error::Error for JwksCoordinationError {}

/// Cluster-wide single-flight JWKS refresh.
///
/// Contract: under an exclusive cluster-wide lock on `jwks_url`, if the
/// shared store holds a document strictly newer than `newer_than` (when
/// given), no older than `max_stale`, and no older than its own declared
/// `max_age`, return it WITHOUT invoking `fetch`; otherwise invoke `fetch`,
/// persist the result, and return it with `age == 0`.
#[async_trait]
pub trait JwksCoordination: Send + Sync {
    /// See the trait docs for the single-flight contract.
    async fn refresh(
        &self,
        jwks_url: &str,
        newer_than: Option<DateTime<Utc>>,
        max_stale: Duration,
        fetch: JwksFetchFn,
    ) -> Result<CoordinatedJwks, JwksCoordinationError>;
}
