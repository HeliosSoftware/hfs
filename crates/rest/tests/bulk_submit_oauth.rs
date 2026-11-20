//! Integration test for the bulk-submit outbound SMART Backend Services client.
//!
//! Stands up a real ephemeral mock IdP (SMART discovery + token endpoint) and
//! verifies that [`JwtClientCredentialsTokenProvider`] discovers the token
//! endpoint, exchanges a `private_key_jwt` client assertion for an access token,
//! and caches it (a second request is served from cache); and that the token
//! only ever travels to the token endpoint's origin or to an origin listed in
//! `HFS_BULK_SUBMIT_TRUSTED_PROVIDERS`.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use std::sync::Mutex;

use axum::{
    Json, Router,
    routing::{get, post},
};
use helios_persistence::core::{FileTokenError, FileTokenProvider};
use helios_rest::bulk_submit_oauth::{JwtClientCredentialsTokenProvider, TrustedProviders};
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

    let file = format!("{base}/bulk/Patient.ndjson");
    let t1 = provider.token(&discovery, "system/*.rs", &file).await;
    assert_eq!(t1.as_deref(), Ok("tok-abc"));

    let t2 = provider.token(&discovery, "system/*.rs", &file).await;
    assert_eq!(t2.as_deref(), Ok("tok-abc"));

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
    let err = provider
        .token(
            &discovery,
            "system/*.rs",
            "http://127.0.0.1:1/manifest.json",
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, FileTokenError::Unavailable(ref reason) if reason.contains("metadata GET")),
        "{err}"
    );
}

// ---- destination policy --------------------------------------------------

/// A mock IdP whose metadata names `token_endpoint` (its own `/token` when
/// `None`). Returns its origin and the metadata/token hit counters.
async fn spawn_idp(token_endpoint: Option<String>) -> (String, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let metadata_hits = Arc::new(AtomicUsize::new(0));
    let token_hits = Arc::new(AtomicUsize::new(0));
    let endpoint = token_endpoint.unwrap_or_else(|| format!("{base}/token"));
    let app = Router::new()
        .route(
            "/.well-known/smart-configuration",
            get({
                let hits = metadata_hits.clone();
                move || {
                    let endpoint = endpoint.clone();
                    hits.fetch_add(1, Ordering::SeqCst);
                    async move { Json(json!({ "token_endpoint": endpoint })) }
                }
            }),
        )
        .route(
            "/token",
            post({
                let hits = token_hits.clone();
                move || {
                    hits.fetch_add(1, Ordering::SeqCst);
                    async move { Json(json!({ "access_token": "tok", "expires_in": 300 })) }
                }
            }),
        );
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (base, metadata_hits, token_hits)
}

/// An origin nobody listens on, distinct from every mock IdP by its port.
async fn other_origin() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    format!("http://{}", listener.local_addr().unwrap())
}

fn provider(trusted: &[&str]) -> Arc<JwtClientCredentialsTokenProvider> {
    JwtClientCredentialsTokenProvider::new_with_trusted_providers(
        "client-1",
        TEST_ES384_PKCS8,
        "ES384",
        TrustedProviders::parse(trusted).expect("test origins parse"),
    )
    .expect("parse ES384 key")
}

#[tokio::test]
async fn token_travels_to_the_token_endpoints_origin() {
    let (idp, _, token_hits) = spawn_idp(None).await;
    let discovery = vec![format!("{idp}/.well-known/smart-configuration")];
    let token = provider(&[])
        .token(
            &discovery,
            "system/*.rs",
            &format!("{idp}/bulk/manifest.json"),
        )
        .await;
    assert_eq!(token.as_deref(), Ok("tok"));
    assert_eq!(token_hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn token_is_withheld_from_a_destination_on_another_origin() {
    let (idp, metadata_hits, token_hits) = spawn_idp(None).await;
    let elsewhere = other_origin().await;
    let discovery = vec![format!("{idp}/.well-known/smart-configuration")];
    let err = provider(&[])
        .token(
            &discovery,
            "system/*.rs",
            &format!("{elsewhere}/bulk/manifest.json"),
        )
        .await
        .unwrap_err();
    assert_eq!(
        err,
        FileTokenError::UntrustedDestination {
            destination: elsewhere.clone(),
            token_origin: idp.clone(),
        }
    );
    assert!(err.to_string().contains(&elsewhere), "{err}");
    // Discovery ran, but no client assertion was spent on a token nobody may receive.
    assert_eq!(metadata_hits.load(Ordering::SeqCst), 1);
    assert_eq!(token_hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_trusted_provider_origin_may_receive_the_token() {
    let (idp, _, token_hits) = spawn_idp(None).await;
    let files = other_origin().await;
    let discovery = vec![format!("{idp}/.well-known/smart-configuration")];
    let provider = provider(&[&idp, &files]);

    let token = provider
        .token(
            &discovery,
            "system/*.rs",
            &format!("{files}/bulk/Patient.ndjson"),
        )
        .await;
    assert_eq!(token.as_deref(), Ok("tok"));
    assert_eq!(token_hits.load(Ordering::SeqCst), 1);

    // The list extends the destinations; it does not drop the endpoint's own origin.
    let token = provider
        .token(
            &discovery,
            "system/*.rs",
            &format!("{idp}/bulk/manifest.json"),
        )
        .await;
    assert_eq!(token.as_deref(), Ok("tok"));

    // And an origin outside it is still refused.
    let stranger = other_origin().await;
    let err = provider
        .token(
            &discovery,
            "system/*.rs",
            &format!("{stranger}/bulk/Patient.ndjson"),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, FileTokenError::UntrustedDestination { .. }),
        "{err}"
    );
}

#[tokio::test]
async fn a_configured_list_refuses_an_unlisted_metadata_url_before_discovery() {
    let (idp, metadata_hits, token_hits) = spawn_idp(None).await;
    let trusted = other_origin().await;
    let discovery = vec![format!("{idp}/.well-known/smart-configuration")];
    let err = provider(&[&trusted])
        .token(
            &discovery,
            "system/*.rs",
            &format!("{trusted}/bulk/manifest.json"),
        )
        .await
        .unwrap_err();
    assert_eq!(
        err,
        FileTokenError::UntrustedProvider {
            origin: idp.clone()
        }
    );
    assert!(
        err.to_string()
            .contains("HFS_BULK_SUBMIT_TRUSTED_PROVIDERS"),
        "{err}"
    );
    assert_eq!(metadata_hits.load(Ordering::SeqCst), 0);
    assert_eq!(token_hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_configured_list_refuses_an_unlisted_token_endpoint() {
    // The listed metadata host hands discovery to a token endpoint outside the list.
    let (real_idp, _, real_token_hits) = spawn_idp(None).await;
    let (front, front_metadata_hits, _) = spawn_idp(Some(format!("{real_idp}/token"))).await;
    let discovery = vec![format!("{front}/.well-known/smart-configuration")];
    let err = provider(&[&front])
        .token(
            &discovery,
            "system/*.rs",
            &format!("{front}/bulk/manifest.json"),
        )
        .await
        .unwrap_err();
    assert_eq!(
        err,
        FileTokenError::UntrustedProvider {
            origin: real_idp.clone()
        }
    );
    assert_eq!(front_metadata_hits.load(Ordering::SeqCst), 1);
    assert_eq!(real_token_hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_metadata_document_cannot_redirect_a_token_to_its_own_host() {
    // The attack: metadata on the submitter's host names a token endpoint
    // where HFS is a registered client, and the manifest lives on that host.
    let (real_idp, _, real_token_hits) = spawn_idp(None).await;
    let (attacker, _, _) = spawn_idp(Some(format!("{real_idp}/token"))).await;
    let discovery = vec![format!("{attacker}/.well-known/smart-configuration")];
    let err = provider(&[])
        .token(
            &discovery,
            "system/*.rs",
            &format!("{attacker}/bulk/manifest.json"),
        )
        .await
        .unwrap_err();
    assert_eq!(
        err,
        FileTokenError::UntrustedDestination {
            destination: attacker.clone(),
            token_origin: real_idp.clone(),
        }
    );
    assert_eq!(real_token_hits.load(Ordering::SeqCst), 0);
}
