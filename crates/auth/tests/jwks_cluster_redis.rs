//! T2 cluster twin — Redis-coordinated JWKS refresh (C2).
//!
//! The Redis twin of `rest/tests/jwks_cluster_pg.rs`: two independently
//! constructed `JwksCache`s, each with its own fresh `RedisJwksCoordination`
//! (never a cloned handle), over one shared Redis container and one wiremock
//! IdP whose hit count is the observable.
//!
//! Gated behind `RUN_REDIS_CLUSTER_TESTS=1` — the Redis seam gets the
//! identical assertions as the CI-default DB backend, runnable on demand
//! without adding Redis to every CI run — see
//! `.github/workflows/redis-cluster-tests.yml`.
//!
//! Tenant-isolation DoD row: N/A — JWKS documents are server-global public
//! key material; there is no tenant dimension.

#![cfg(feature = "redis")]

use std::sync::Arc;

use helios_auth::{JwksCache, JwksCoordination, RedisJwksCoordination};
use testcontainers::ImageExt;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::redis::{REDIS_PORT, Redis};
use tokio::sync::OnceCell;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KID_1: &str = "rotation-key-1";
const KID_2: &str = "rotation-key-2";

/// RSA public modulus reused from `bearer_token_reuse.rs` (any valid JWK
/// works — these tests never validate tokens, only key installation).
const TEST_JWK_N: &str = "kq6CDlGgp5pwM8F301CXez3h5CgRci9XLgM92w4Mwsui8oUngWsrWC4M3ON79jA_XyDKQ9bIdlLmuZjOersqTtotxSlbAiUM0bO4HNXM_HQTwgxuoyvcMJkPnhN03acT36g8FTcbDULvAeagcn4MDur4wzwxb3ZdSWhhiwcACtmjDWv3y9orA9-fIiM658nZF4FGA_BZ3ymNrt4Knk8uMqySApQYix-yh-9_wqVNPMNsULLFMmTziGfrFlICm_xygJepXQSjOoHTsPn0nHU_0IOXmVos8SaKISO4Agy1K2vsVYuEOiANYdviCWOdt3xt5V6brDvJ4vLzz07X0nPdEw";

fn redis_tests_enabled() -> bool {
    std::env::var("RUN_REDIS_CLUSTER_TESTS").is_ok_and(|v| v == "1")
}

struct SharedRedis {
    url: String,
    /// Never dropped (a `static` outlives the process); the container is
    /// reaped in CI by its `github.run_id` label, like the Postgres one.
    _container: testcontainers::ContainerAsync<Redis>,
}

static SHARED_REDIS: OnceCell<SharedRedis> = OnceCell::const_new();

async fn shared_redis() -> &'static SharedRedis {
    SHARED_REDIS
        .get_or_init(|| async {
            let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
            let container = Redis::default()
                .with_label("github.run_id", &run_id)
                .start()
                .await
                .expect("failed to start Redis container");
            let port = container
                .get_host_port_ipv4(REDIS_PORT)
                .await
                .expect("failed to get Redis host port");
            let host = container
                .get_host()
                .await
                .expect("failed to get Redis host")
                .to_string();
            SharedRedis {
                url: format!("redis://{host}:{port}"),
                _container: container,
            }
        })
        .await
}

fn jwks_body(kid: &str) -> serde_json::Value {
    serde_json::json!({
        "keys": [{
            "kid": kid,
            "kty": "RSA",
            "use": "sig",
            "alg": "RS256",
            "n": TEST_JWK_N,
            "e": "AQAB",
        }]
    })
}

async fn mount_jwks(server: &MockServer, kid: &str) {
    Mock::given(method("GET"))
        .and(path("/jwks"))
        .respond_with(ResponseTemplate::new(200).set_body_json(jwks_body(kid)))
        .mount(server)
        .await;
}

async fn idp_hits(server: &MockServer) -> usize {
    server
        .received_requests()
        .await
        .expect("request recording enabled")
        .len()
}

/// One simulated instance: a fresh `JwksCache` with its own freshly
/// constructed Redis coordinator (shares nothing but the Redis server).
async fn instance(jwks_url: &str) -> Arc<JwksCache> {
    let redis = shared_redis().await;
    let coordination = RedisJwksCoordination::new(&redis.url).expect("valid Redis URL");
    let cache = Arc::new(JwksCache::new(jwks_url, 0));
    assert!(cache.set_coordination(Arc::new(coordination) as Arc<dyn JwksCoordination>));
    cache
}

/// DoD row: exclusivity (single flight) — two instances booting
/// concurrently against one IdP perform exactly one JWKS fetch between
/// them, and both serve the key.
#[tokio::test]
async fn redis_cluster_jwks_boot_herd_single_upstream_fetch() {
    if !redis_tests_enabled() {
        eprintln!("skipping: set RUN_REDIS_CLUSTER_TESTS=1 to run the Redis cluster twin");
        return;
    }
    let server = MockServer::start().await;
    mount_jwks(&server, KID_1).await;
    let url = format!("{}/jwks", server.uri());

    let a = instance(&url).await;
    let b = instance(&url).await;

    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let (barrier_a, barrier_b) = (Arc::clone(&barrier), barrier);
    let (cache_a, cache_b) = (Arc::clone(&a), Arc::clone(&b));
    let task_a = tokio::spawn(async move {
        barrier_a.wait().await;
        cache_a.initial_fetch().await
    });
    let task_b = tokio::spawn(async move {
        barrier_b.wait().await;
        cache_b.initial_fetch().await
    });
    task_a.await.unwrap().expect("instance A boot fetch");
    task_b.await.unwrap().expect("instance B boot fetch");

    assert_eq!(
        idp_hits(&server).await,
        1,
        "exactly one upstream fetch across both booting instances"
    );
    a.get_key(KID_1).await.expect("key on instance A");
    b.get_key(KID_1).await.expect("key on instance B");
}

/// A late joiner reuses the stored document without contacting the IdP.
#[tokio::test]
async fn redis_cluster_jwks_late_joiner_reuses_stored_document() {
    if !redis_tests_enabled() {
        eprintln!("skipping: set RUN_REDIS_CLUSTER_TESTS=1 to run the Redis cluster twin");
        return;
    }
    let server = MockServer::start().await;
    mount_jwks(&server, KID_1).await;
    let url = format!("{}/jwks", server.uri());

    let first = instance(&url).await;
    first.initial_fetch().await.expect("first instance boot");
    assert_eq!(idp_hits(&server).await, 1);

    let late = instance(&url).await;
    late.initial_fetch().await.expect("late joiner boot");
    assert_eq!(
        idp_hits(&server).await,
        1,
        "the late joiner must reuse the stored document"
    );
    late.get_key(KID_1).await.expect("key on the late joiner");
}

/// DoD row, rotation: after the IdP rotates its key, two instances racing
/// the unknown-kid refresh perform exactly one additional fetch, and both
/// resolve the new kid (the loser adopts the winner's document via the
/// watermark).
#[tokio::test]
async fn redis_cluster_jwks_rotation_single_additional_fetch() {
    if !redis_tests_enabled() {
        eprintln!("skipping: set RUN_REDIS_CLUSTER_TESTS=1 to run the Redis cluster twin");
        return;
    }
    let server = MockServer::start().await;
    mount_jwks(&server, KID_1).await;
    let url = format!("{}/jwks", server.uri());

    let a = instance(&url).await;
    let b = instance(&url).await;
    a.initial_fetch().await.expect("instance A boot");
    b.initial_fetch().await.expect("instance B boot");

    // Rotate: the IdP now serves only KID_2. reset() also clears the
    // request log, so counts below are post-rotation deltas.
    server.reset().await;
    mount_jwks(&server, KID_2).await;

    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let (barrier_a, barrier_b) = (Arc::clone(&barrier), barrier);
    let (cache_a, cache_b) = (Arc::clone(&a), Arc::clone(&b));
    let task_a = tokio::spawn(async move {
        barrier_a.wait().await;
        cache_a.get_key(KID_2).await.map(|_| ())
    });
    let task_b = tokio::spawn(async move {
        barrier_b.wait().await;
        cache_b.get_key(KID_2).await.map(|_| ())
    });
    task_a
        .await
        .unwrap()
        .unwrap_or_else(|e| panic!("instance A must resolve the rotated kid: {e}"));
    task_b
        .await
        .unwrap()
        .unwrap_or_else(|e| panic!("instance B must resolve the rotated kid: {e}"));

    assert_eq!(
        idp_hits(&server).await,
        1,
        "exactly one additional upstream fetch for the rotation"
    );
}
