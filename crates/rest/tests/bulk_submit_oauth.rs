//! Integration test for the bulk-submit outbound SMART Backend Services client.
//!
//! Stands up a real ephemeral mock IdP (SMART discovery + token endpoint) and
//! verifies that [`JwtClientCredentialsTokenProvider`] discovers the token
//! endpoint, exchanges a `private_key_jwt` client assertion for an access token,
//! and caches it (a second request is served from cache).

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use std::sync::Mutex;

use axum::{
    Json, Router,
    routing::{get, post},
};
use helios_persistence::core::FileTokenProvider;
use helios_rest::bulk_submit_oauth::JwtClientCredentialsTokenProvider;
use serde_json::json;

// Test-only PKCS#8 P-384 private key (ES384). Not used anywhere else.
const TEST_ES384_PKCS8: &str = "-----BEGIN PRIVATE KEY-----
MIG2AgEAMBAGByqGSM49AgEGBSuBBAAiBIGeMIGbAgEBBDDh4/zIrHrW0+fKypcL
B1a8x4n3Dvci3RzRNgH6YJqqmtBwLd3Irnx9daxwlD/5U7ahZANiAAQKn/bdkrzR
KkRi8EMcS3BFQ2HtRQlxzLdjZjos4eQkZZUpoZt3tOWMUR4rWAvE8g9Rh5Ro9UUE
JR3ki5aX+jyNPExMQbJF0j0KlaKh5y8MEV6vq8GaqVmsrabIn7CeLUA=
-----END PRIVATE KEY-----
";

#[tokio::test]
async fn test_outbound_client_credentials_discovery_and_cache() {
    let token_hits = Arc::new(AtomicUsize::new(0));
    let last_form: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));

    // --- Stand up a mock IdP on an ephemeral port. ---
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base = format!("http://{addr}");

    let disc_base = base.clone();
    let hits = token_hits.clone();
    let form_capture = last_form.clone();
    let app = Router::new()
        .route(
            "/.well-known/smart-configuration",
            get(move || {
                let disc_base = disc_base.clone();
                async move { Json(json!({ "token_endpoint": format!("{disc_base}/token") })) }
            }),
        )
        .route(
            "/token",
            post(move |body: String| {
                let hits = hits.clone();
                let form_capture = form_capture.clone();
                async move {
                    hits.fetch_add(1, Ordering::SeqCst);
                    *form_capture.lock().unwrap() = body;
                    Json(json!({ "access_token": "tok-abc", "expires_in": 300 }))
                }
            }),
        );
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // --- Provider acquires + caches a read-scoped token. ---
    let provider = JwtClientCredentialsTokenProvider::new("client-1", TEST_ES384_PKCS8, "ES384")
        .expect("parse ES384 key");
    let discovery = vec![format!("{base}/.well-known/smart-configuration")];

    let t1 = provider.token(&discovery, "system/*.rs").await;
    assert_eq!(t1.as_deref(), Some("tok-abc"));

    let t2 = provider.token(&discovery, "system/*.rs").await;
    assert_eq!(t2.as_deref(), Some("tok-abc"));

    // The second call is served from cache → token endpoint hit exactly once.
    assert_eq!(token_hits.load(Ordering::SeqCst), 1);

    // The posted form carried the client_credentials grant, the requested READ
    // scope (not system/bulk-submit), and a 3-segment private_key_jwt assertion.
    let form = last_form.lock().unwrap().clone();
    let pairs: std::collections::HashMap<_, _> = url::form_urlencoded::parse(form.as_bytes())
        .into_owned()
        .collect();
    assert_eq!(
        pairs.get("grant_type").map(String::as_str),
        Some("client_credentials")
    );
    assert_eq!(pairs.get("scope").map(String::as_str), Some("system/*.rs"));
    assert_eq!(
        pairs.get("client_assertion_type").map(String::as_str),
        Some("urn:ietf:params:oauth:client-assertion-type:jwt-bearer")
    );
    let assertion = pairs
        .get("client_assertion")
        .expect("client_assertion present");
    assert_eq!(
        assertion.split('.').count(),
        3,
        "client_assertion must be a compact JWS (header.payload.signature)"
    );
    // The assertion payload binds iss/sub to the client_id and aud to the token endpoint.
    use base64::Engine;
    let payload_b64 = assertion.split('.').nth(1).unwrap();
    let payload_json = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .expect("base64url payload");
    let claims: serde_json::Value = serde_json::from_slice(&payload_json).unwrap();
    assert_eq!(claims["iss"], "client-1");
    assert_eq!(claims["sub"], "client-1");
    assert_eq!(claims["aud"], format!("{base}/token"));
    assert!(claims["jti"].is_string());
}

#[tokio::test]
async fn test_outbound_unparseable_key_rejected() {
    assert!(JwtClientCredentialsTokenProvider::new("c", "not-a-key", "ES384").is_none());
}

#[tokio::test]
async fn test_outbound_discovery_failure_returns_none() {
    let provider =
        JwtClientCredentialsTokenProvider::new("client-1", TEST_ES384_PKCS8, "ES384").unwrap();
    // Unreachable metadata URL → no token.
    let discovery = vec!["http://127.0.0.1:1/.well-known/smart-configuration".to_string()];
    assert!(provider.token(&discovery, "system/*.rs").await.is_none());
}
