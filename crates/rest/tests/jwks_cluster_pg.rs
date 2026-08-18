//! T2 cluster suite — Postgres-coordinated JWKS refresh (C2).
//!
//! Two independently constructed `JwksCache`s play "instance A" and
//! "instance B" (fresh handles, never a cloned `Arc`), each wired the way
//! `hfs` wires them in cluster mode: a fresh `PostgresBackend` →
//! `cluster_refresh_cache()` →
//! `StoreJwksCoordination` → `set_coordination`. One wiremock IdP's hit
//! count is the observable: N concurrent refreshes must produce exactly one
//! upstream fetch.
//!
//! Deterministic without sleeps: the watermark (not wall-clock staleness)
//! decides reuse-vs-refetch, the per-instance rate limiter is disabled
//! (`min_refresh_interval = 0`), and mock responses carry no Cache-Control
//! header so the background refresh task sleeps 75% of the 3600s default TTL
//! and never interferes.
//!
//! Tenant-isolation DoD row: N/A — JWKS documents are server-global public
//! key material shared by every tenant; the `cluster_refresh_cache` table
//! deliberately has no tenant column (methodology §6).

#![cfg(feature = "postgres")]

use std::path::PathBuf;
use std::sync::Arc;

use helios_auth::{JwksCache, JwksCoordination};
use helios_persistence::backends::postgres::{PostgresBackend, PostgresConfig};
use helios_persistence::core::ResourceStorage;
use helios_rest::StoreJwksCoordination;

use testcontainers::ImageExt;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use tokio::sync::OnceCell;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KID_1: &str = "rotation-key-1";
const KID_2: &str = "rotation-key-2";

/// RSA public modulus reused from `auth/tests/bearer_token_reuse.rs` (these
/// tests never validate tokens, only key installation).
const TEST_JWK_N: &str = "kq6CDlGgp5pwM8F301CXez3h5CgRci9XLgM92w4Mwsui8oUngWsrWC4M3ON79jA_XyDKQ9bIdlLmuZjOersqTtotxSlbAiUM0bO4HNXM_HQTwgxuoyvcMJkPnhN03acT36g8FTcbDULvAeagcn4MDur4wzwxb3ZdSWhhiwcACtmjDWv3y9orA9-fIiM658nZF4FGA_BZ3ymNrt4Knk8uMqySApQYix-yh-9_wqVNPMNsULLFMmTziGfrFlICm_xygJepXQSjOoHTsPn0nHU_0IOXmVos8SaKISO4Agy1K2vsVYuEOiANYdviCWOdt3xt5V6brDvJ4vLzz07X0nPdEw";

struct SharedPg {
    host: String,
    port: u16,
    /// Kept alive for the test binary's lifetime; a `static` is never
    /// dropped, so CI reaps the container by its `github.run_id` label.
    _container: testcontainers::ContainerAsync<Postgres>,
}

static SHARED_PG: OnceCell<SharedPg> = OnceCell::const_new();

async fn shared_pg() -> &'static SharedPg {
    SHARED_PG
        .get_or_init(|| async {
            let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
            let container = Postgres::default()
                .with_label("github.run_id", &run_id)
                .start()
                .await
                .expect("failed to start PostgreSQL container");
            let port = container
                .get_host_port_ipv4(5432)
                .await
                .expect("failed to get host port");
            let host = container
                .get_host()
                .await
                .expect("failed to get host")
                .to_string();

            let backend = PostgresBackend::new(pg_config(&host, port))
                .await
                .expect("failed to create PostgresBackend");
            backend
                .init_schema()
                .await
                .expect("failed to initialize schema");

            SharedPg {
                host,
                port,
                _container: container,
            }
        })
        .await
}

fn pg_config(host: &str, port: u16) -> PostgresConfig {
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("data"))
        .unwrap_or_else(|| PathBuf::from("data"));
    PostgresConfig {
        host: host.to_string(),
        port,
        dbname: "postgres".to_string(),
        user: "postgres".to_string(),
        password: Some("postgres".to_string()),
        max_connections: 5,
        data_dir: Some(data_dir),
        ..Default::default()
    }
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

/// One simulated instance: a fresh `JwksCache` over a fresh backend handle,
/// wired exactly as `hfs` wires it in database coordination mode.
async fn instance(jwks_url: &str) -> Arc<JwksCache> {
    let backend = PostgresBackend::new(pg_config(&shared_pg().await.host, shared_pg().await.port))
        .await
        .expect("failed to create PostgresBackend");
    let store = backend
        .cluster_refresh_cache()
        .expect("postgres backs a cluster refresh cache");
    let cache = Arc::new(JwksCache::new(jwks_url, 0));
    assert!(
        cache.set_coordination(
            Arc::new(StoreJwksCoordination::new(store)) as Arc<dyn JwksCoordination>
        )
    );
    cache
}

/// DoD row: exclusivity (single flight) — two instances booting
/// concurrently perform exactly one JWKS fetch between them, and both serve
/// the key (strategy §8 Phase 2: "N handles trigger JWKS refresh under the
/// coordinator lock → exactly one upstream fetch").
#[tokio::test]
async fn jwks_cluster_boot_herd_single_upstream_fetch() {
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

/// Visibility/durability: a later instance (fresh backend handle, fresh
/// cache) boots without contacting the IdP at all.
#[tokio::test]
async fn jwks_cluster_late_joiner_reuses_stored_document() {
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

/// Rotation: after the IdP rotates its key, two instances racing the
/// unknown-kid refresh perform exactly one additional fetch, and both
/// resolve the new kid — the loser adopts the winner's newer document via
/// the watermark instead of refetching or failing.
#[tokio::test]
async fn jwks_cluster_rotation_single_additional_fetch() {
    let server = MockServer::start().await;
    mount_jwks(&server, KID_1).await;
    let url = format!("{}/jwks", server.uri());

    let a = instance(&url).await;
    let b = instance(&url).await;
    a.initial_fetch().await.expect("instance A boot");
    b.initial_fetch().await.expect("instance B boot");

    // Rotate: the IdP now serves only KID_2. reset() also clears the
    // request log, so the count below is the post-rotation delta.
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
