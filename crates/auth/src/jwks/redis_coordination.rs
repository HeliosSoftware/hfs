//! Redis-backed [`JwksCoordination`] (cluster design §5 C2).
//!
//! Resurrected from the pre-#205 `JwksCoordinator` (leader lock + shared key
//! store; commit `9c90d835^`) and adapted to the [`JwksCoordination`]
//! contract. Two keys per JWKS URL:
//!
//! - `hfs:jwks:lock:<url>` — the single-flight lock (`SET NX EX`, holder
//!   token compared on release so an expired lock is never freed for a new
//!   holder);
//! - `hfs:jwks:doc:<url>` — the stored document (JSON `{body, max_age_secs,
//!   fetched_at}`) with a TTL covering the staleness window.
//!
//! A caller that loses the lock race polls the document key until the
//! holder's fresh document appears; if the holder dies (lock TTL passes with
//! no document), the caller fetches directly as a last resort — single-flight
//! degrades gracefully rather than blocking auth.
//!
//! `fetched_at` uses the *instance* clock (Redis has no `RETURNING now()`
//! equivalent woven into `SET`), so watermark comparisons tolerate clock skew
//! in the worst case by performing one extra fetch — never by serving stale
//! keys.

use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use super::coordination::{CoordinatedJwks, JwksCoordination, JwksCoordinationError, JwksFetchFn};

/// How long the single-flight lock is held before a presumed-dead holder is
/// bypassed. Comfortably above the fetcher's 10s HTTP timeout.
const DEFAULT_LOCK_TTL: Duration = Duration::from_secs(30);

/// How often a lock loser polls for the holder's document.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Compare-and-delete so an expired lock reacquired by someone else is never
/// released by the previous holder.
const RELEASE_SCRIPT: &str = r#"
if redis.call('get', KEYS[1]) == ARGV[1] then
    return redis.call('del', KEYS[1])
else
    return 0
end
"#;

#[derive(Serialize, Deserialize)]
struct StoredDoc {
    body: String,
    max_age_secs: Option<u64>,
    fetched_at: DateTime<Utc>,
}

/// See [module docs](self).
pub struct RedisJwksCoordination {
    client: redis::Client,
    lock_ttl: Duration,
}

impl RedisJwksCoordination {
    /// Creates a coordinator over the given Redis URL. Connections are
    /// established lazily per call, so a down Redis surfaces as
    /// [`JwksCoordinationError::Unavailable`] (fallback), not a boot failure.
    pub fn new(redis_url: &str) -> Result<Self, JwksCoordinationError> {
        Self::with_lock_ttl(redis_url, DEFAULT_LOCK_TTL)
    }

    /// [`new`](Self::new) with an explicit lock TTL (tests).
    pub fn with_lock_ttl(
        redis_url: &str,
        lock_ttl: Duration,
    ) -> Result<Self, JwksCoordinationError> {
        let client = redis::Client::open(redis_url)
            .map_err(|e| JwksCoordinationError::Unavailable(format!("invalid Redis URL: {e}")))?;
        Ok(Self { client, lock_ttl })
    }

    async fn conn(&self) -> Result<redis::aio::MultiplexedConnection, JwksCoordinationError> {
        self.client
            .get_multiplexed_async_connection()
            .await
            .map_err(unavailable)
    }

    async fn try_acquire(
        &self,
        conn: &mut redis::aio::MultiplexedConnection,
        url: &str,
        token: &str,
    ) -> Result<bool, JwksCoordinationError> {
        let acquired: Option<String> = redis::cmd("SET")
            .arg(lock_key(url))
            .arg(token)
            .arg("NX")
            .arg("EX")
            .arg(self.lock_ttl.as_secs().max(1))
            .query_async(conn)
            .await
            .map_err(unavailable)?;
        Ok(acquired.is_some())
    }

    async fn release(&self, conn: &mut redis::aio::MultiplexedConnection, url: &str, token: &str) {
        if let Err(e) = redis::Script::new(RELEASE_SCRIPT)
            .key(lock_key(url))
            .arg(token)
            .invoke_async::<()>(conn)
            .await
        {
            // The lock self-expires; a failed release only delays peers.
            warn!(error = %e, "Failed to release the JWKS refresh lock");
        }
    }

    /// Fetches via the caller's closure and stores the result (best-effort —
    /// a failed store still returns the fetched document).
    async fn fetch_and_store(
        &self,
        conn: &mut redis::aio::MultiplexedConnection,
        url: &str,
        max_stale: Duration,
        fetch: JwksFetchFn,
    ) -> Result<CoordinatedJwks, JwksCoordinationError> {
        let fetched = fetch().await.map_err(JwksCoordinationError::Fetch)?;
        let fetched_at = Utc::now();
        let stored = StoredDoc {
            body: fetched.body.clone(),
            max_age_secs: fetched.max_age.map(|d| d.as_secs()),
            fetched_at,
        };
        let doc_ttl = max_stale.as_secs().max(self.lock_ttl.as_secs() * 4).max(1);
        match serde_json::to_string(&stored) {
            Ok(json) => {
                if let Err(e) = conn.set_ex::<_, _, ()>(doc_key(url), json, doc_ttl).await {
                    warn!(error = %e, "Failed to store the fetched JWKS document in Redis");
                }
            }
            Err(e) => warn!(error = %e, "Failed to serialize the fetched JWKS document"),
        }
        Ok(CoordinatedJwks {
            body: fetched.body,
            max_age: fetched.max_age,
            fetched_at,
            age: Duration::ZERO,
        })
    }
}

fn unavailable(e: redis::RedisError) -> JwksCoordinationError {
    JwksCoordinationError::Unavailable(e.to_string())
}

fn lock_key(url: &str) -> String {
    format!("hfs:jwks:lock:{url}")
}

fn doc_key(url: &str) -> String {
    format!("hfs:jwks:doc:{url}")
}

fn reusable(doc: &StoredDoc, newer_than: Option<DateTime<Utc>>, max_stale: Duration) -> bool {
    if let Some(watermark) = newer_than
        && doc.fetched_at <= watermark
    {
        return false;
    }
    let age = (Utc::now() - doc.fetched_at).to_std().unwrap_or_default();
    if age > max_stale {
        return false;
    }
    if let Some(max_age) = doc.max_age_secs
        && age > Duration::from_secs(max_age)
    {
        return false;
    }
    true
}

fn to_coordinated(doc: StoredDoc) -> CoordinatedJwks {
    let age = (Utc::now() - doc.fetched_at).to_std().unwrap_or_default();
    CoordinatedJwks {
        body: doc.body,
        max_age: doc.max_age_secs.map(Duration::from_secs),
        fetched_at: doc.fetched_at,
        age,
    }
}

async fn read_doc(
    conn: &mut redis::aio::MultiplexedConnection,
    url: &str,
) -> Result<Option<StoredDoc>, JwksCoordinationError> {
    let raw: Option<String> = conn.get(doc_key(url)).await.map_err(unavailable)?;
    Ok(raw.and_then(|json| match serde_json::from_str(&json) {
        Ok(doc) => Some(doc),
        Err(e) => {
            // Treat a corrupt document as absent; the caller refetches.
            warn!(error = %e, "Ignoring an unparseable stored JWKS document");
            None
        }
    }))
}

#[async_trait]
impl JwksCoordination for RedisJwksCoordination {
    async fn refresh(
        &self,
        jwks_url: &str,
        newer_than: Option<DateTime<Utc>>,
        max_stale: Duration,
        fetch: JwksFetchFn,
    ) -> Result<CoordinatedJwks, JwksCoordinationError> {
        let mut conn = self.conn().await?;
        let deadline = Instant::now() + self.lock_ttl;
        // Distinguishes this holder from a later one after lock expiry.
        let token = format!(
            "{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );

        loop {
            // Fast path: a document someone else already refreshed.
            if let Some(doc) = read_doc(&mut conn, jwks_url).await?
                && reusable(&doc, newer_than, max_stale)
            {
                return Ok(to_coordinated(doc));
            }

            if self.try_acquire(&mut conn, jwks_url, &token).await? {
                // Double-check after winning: the previous holder may have
                // stored between our read and our acquire.
                if let Some(doc) = read_doc(&mut conn, jwks_url).await?
                    && reusable(&doc, newer_than, max_stale)
                {
                    self.release(&mut conn, jwks_url, &token).await;
                    return Ok(to_coordinated(doc));
                }
                let result = self
                    .fetch_and_store(&mut conn, jwks_url, max_stale, fetch)
                    .await;
                self.release(&mut conn, jwks_url, &token).await;
                return result;
            }

            // Someone else is fetching — wait for their document.
            if Instant::now() >= deadline {
                // The holder outlived its lock TTL without storing anything;
                // presume it dead and fetch directly (last-writer-wins on the
                // store) rather than blocking auth forever.
                debug!(url = %jwks_url, "JWKS lock holder timed out; fetching directly");
                return self
                    .fetch_and_store(&mut conn, jwks_url, max_stale, fetch)
                    .await;
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_doc_round_trips_through_json() {
        let doc = StoredDoc {
            body: r#"{"keys":[]}"#.to_string(),
            max_age_secs: Some(1200),
            fetched_at: Utc::now(),
        };
        let json = serde_json::to_string(&doc).unwrap();
        let back: StoredDoc = serde_json::from_str(&json).unwrap();
        assert_eq!(back.body, doc.body);
        assert_eq!(back.max_age_secs, doc.max_age_secs);
        assert_eq!(back.fetched_at, doc.fetched_at);
    }

    #[test]
    fn reusable_applies_watermark_staleness_and_own_max_age() {
        let hour = Duration::from_secs(3600);
        let fresh = StoredDoc {
            body: String::new(),
            max_age_secs: None,
            fetched_at: Utc::now(),
        };
        assert!(reusable(&fresh, None, hour));
        // Not newer than the caller's own document.
        assert!(!reusable(&fresh, Some(fresh.fetched_at), hour));
        assert!(reusable(
            &fresh,
            Some(fresh.fetched_at - chrono::Duration::seconds(1)),
            hour
        ));

        let old = StoredDoc {
            body: String::new(),
            max_age_secs: None,
            fetched_at: Utc::now() - chrono::Duration::hours(2),
        };
        assert!(!reusable(&old, None, hour));

        let past_own_lifetime = StoredDoc {
            body: String::new(),
            max_age_secs: Some(60),
            fetched_at: Utc::now() - chrono::Duration::seconds(120),
        };
        assert!(!reusable(&past_own_lifetime, None, hour));
    }
}
