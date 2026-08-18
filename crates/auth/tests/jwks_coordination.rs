//! `JwksCache` × `JwksCoordination` contract tests (cluster design §5 C2).
//!
//! T1: a fake coordinator drives every cache-side branch of the coordinated
//! refresh seam — shared-document reuse, watermark threading, upstream-error
//! propagation, and the availability fallback — against a wiremock IdP whose
//! hit count is the observable. The real cross-instance suites live with the
//! backends (Postgres: `rest/tests/jwks_cluster_pg.rs`; Redis:
//! `auth/tests/jwks_cluster_redis.rs`).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use helios_auth::jwks::JwksFetcher;
use helios_auth::{
    AuthError, CoordinatedJwks, JwksCache, JwksCoordination, JwksCoordinationError, JwksFetchFn,
};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TEST_KID: &str = "test-key-1";

/// The public half of the test RSA key from `bearer_token_reuse.rs`, as a
/// JWK modulus.
const TEST_JWK_N: &str = "kq6CDlGgp5pwM8F301CXez3h5CgRci9XLgM92w4Mwsui8oUngWsrWC4M3ON79jA_XyDKQ9bIdlLmuZjOersqTtotxSlbAiUM0bO4HNXM_HQTwgxuoyvcMJkPnhN03acT36g8FTcbDULvAeagcn4MDur4wzwxb3ZdSWhhiwcACtmjDWv3y9orA9-fIiM658nZF4FGA_BZ3ymNrt4Knk8uMqySApQYix-yh-9_wqVNPMNsULLFMmTziGfrFlICm_xygJepXQSjOoHTsPn0nHU_0IOXmVos8SaKISO4Agy1K2vsVYuEOiANYdviCWOdt3xt5V6brDvJ4vLzz07X0nPdEw";

fn test_jwks_body() -> String {
    serde_json::json!({
        "keys": [{
            "kid": TEST_KID,
            "kty": "RSA",
            "use": "sig",
            "alg": "RS256",
            "n": TEST_JWK_N,
            "e": "AQAB",
        }]
    })
    .to_string()
}

async fn start_jwks_server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/jwks"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(test_jwks_body(), "application/json")
                .insert_header("cache-control", "public, max-age=1200"),
        )
        .mount(&server)
        .await;
    server
}

async fn idp_hits(server: &MockServer) -> usize {
    server
        .received_requests()
        .await
        .expect("request recording enabled")
        .len()
}

enum Mode {
    /// Return a stored document without invoking the fetch closure.
    Stored {
        body: String,
        fetched_at: DateTime<Utc>,
        age: Duration,
    },
    /// Invoke the fetch closure and stamp the result with `fetched_at`.
    PassThrough { fetched_at: DateTime<Utc> },
    /// Coordination layer down.
    Unavailable,
}

/// Records every `newer_than` watermark the cache passes in.
struct FakeCoordination {
    mode: Mode,
    watermarks: Mutex<Vec<Option<DateTime<Utc>>>>,
}

impl FakeCoordination {
    fn new(mode: Mode) -> Arc<Self> {
        Arc::new(Self {
            mode,
            watermarks: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait]
impl JwksCoordination for FakeCoordination {
    async fn refresh(
        &self,
        _jwks_url: &str,
        newer_than: Option<DateTime<Utc>>,
        _max_stale: Duration,
        fetch: JwksFetchFn,
    ) -> Result<CoordinatedJwks, JwksCoordinationError> {
        self.watermarks.lock().unwrap().push(newer_than);
        match &self.mode {
            Mode::Stored {
                body,
                fetched_at,
                age,
            } => Ok(CoordinatedJwks {
                body: body.clone(),
                max_age: None,
                fetched_at: *fetched_at,
                age: *age,
            }),
            Mode::PassThrough { fetched_at } => {
                let fetched = fetch().await.map_err(JwksCoordinationError::Fetch)?;
                Ok(CoordinatedJwks {
                    body: fetched.body,
                    max_age: fetched.max_age,
                    fetched_at: *fetched_at,
                    age: Duration::ZERO,
                })
            }
            Mode::Unavailable => Err(JwksCoordinationError::Unavailable(
                "shared store down".to_string(),
            )),
        }
    }
}

/// A shared document from the coordinator installs keys without any IdP
/// fetch — the whole point of C2.
#[tokio::test]
async fn reused_shared_document_installs_keys_without_idp_fetch() {
    let server = start_jwks_server().await;
    let coordination = FakeCoordination::new(Mode::Stored {
        body: test_jwks_body(),
        fetched_at: Utc::now() - chrono::Duration::seconds(120),
        age: Duration::from_secs(120),
    });

    let cache = JwksCache::new(&format!("{}/jwks", server.uri()), 0);
    assert!(cache.set_coordination(coordination));
    cache.initial_fetch().await.expect("coordinated boot fetch");

    assert_eq!(idp_hits(&server).await, 0, "the IdP must not be contacted");
    cache
        .get_key(TEST_KID)
        .await
        .expect("key from the shared document");
}

/// `set_coordination` is settable exactly once.
#[tokio::test]
async fn coordination_is_set_once() {
    let cache = JwksCache::new("http://unused.example/jwks", 0);
    let first = FakeCoordination::new(Mode::Unavailable);
    let second = FakeCoordination::new(Mode::Unavailable);
    assert!(cache.set_coordination(first));
    assert!(!cache.set_coordination(second));
}

/// The cache passes no watermark on its first refresh and the stored
/// document's `fetched_at` on the next one.
#[tokio::test]
async fn watermark_threads_between_refreshes() {
    let server = start_jwks_server().await;
    let stamp = Utc::now() - chrono::Duration::seconds(5);
    let coordination = FakeCoordination::new(Mode::PassThrough { fetched_at: stamp });

    let cache = JwksCache::new(&format!("{}/jwks", server.uri()), 0);
    assert!(cache.set_coordination(Arc::clone(&coordination) as Arc<dyn JwksCoordination>));
    cache.initial_fetch().await.expect("boot fetch");

    // An unknown kid triggers a second (rate-limit-free) refresh.
    // (`.err()` because `DecodingKey` is not `Debug`.)
    let err = cache
        .get_key("no-such-kid")
        .await
        .err()
        .expect("an unknown kid must not resolve");
    assert!(matches!(err, AuthError::UnknownKid { .. }));

    let watermarks = coordination.watermarks.lock().unwrap().clone();
    assert_eq!(watermarks.len(), 2);
    assert_eq!(watermarks[0], None, "first refresh has nothing to compare");
    assert_eq!(
        watermarks[1],
        Some(stamp),
        "second refresh passes the stored document's fetched_at"
    );
}

/// An IdP failure under the coordination lock propagates as the fetch error.
#[tokio::test]
async fn upstream_error_under_coordination_propagates() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/jwks"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    let coordination = FakeCoordination::new(Mode::PassThrough {
        fetched_at: Utc::now(),
    });

    let cache = JwksCache::new(&format!("{}/jwks", server.uri()), 0);
    assert!(cache.set_coordination(coordination));
    let err = cache.initial_fetch().await.unwrap_err();
    assert!(matches!(err, AuthError::JwksFetchError(_)));
}

/// A dead coordination layer falls back to a direct IdP fetch — auth
/// availability outranks the dedupe optimization.
#[tokio::test]
async fn unavailable_coordination_falls_back_to_direct_fetch() {
    let server = start_jwks_server().await;
    let coordination = FakeCoordination::new(Mode::Unavailable);

    let cache = JwksCache::new(&format!("{}/jwks", server.uri()), 0);
    assert!(cache.set_coordination(coordination));
    cache.initial_fetch().await.expect("fallback boot fetch");

    assert_eq!(idp_hits(&server).await, 1, "exactly one direct fetch");
    cache
        .get_key(TEST_KID)
        .await
        .expect("key from direct fetch");
}

/// `fetch_raw` preserves the body and Cache-Control lifetime;
/// `parse_document` turns the body into usable decoding keys.
#[tokio::test]
async fn fetch_raw_and_parse_document_round_trip() {
    let server = start_jwks_server().await;
    let fetcher = JwksFetcher::new();

    let raw = fetcher
        .fetch_raw(&format!("{}/jwks", server.uri()))
        .await
        .expect("raw fetch");
    assert_eq!(raw.max_age, Some(Duration::from_secs(1200)));

    let keys = JwksFetcher::parse_document(&raw.body).expect("parse");
    assert!(keys.contains_key(TEST_KID));
}
