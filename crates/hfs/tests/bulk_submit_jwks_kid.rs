//! The two bulk-submit signing sites and the JWKS endpoint agree on one `kid`
//! (#529).
//!
//! HFS signs `private_key_jwt` client assertions in two places that share no
//! code: the web UI's Bulk Import page (`helios-ui`, `signing_kid`) and the
//! bulk-submit consumer plus the JWKS handler (`helios-rest`,
//! `derive_public_jwk`). A recipient finds the right key by matching the
//! assertion's `kid` header to the key set, so a disagreement would break
//! verification silently. Only this crate sees both, so the check lives here
//! and drives each one through its public surface: the UI's "test
//! authentication" route and `GET /.well-known/bulk-submit-jwks.json`.
//!
//! One test, run top to bottom: it sets the process-wide
//! `HFS_BULK_SUBMIT_PRIVATE_KEY` the UI reads, and nothing else in this binary
//! may run beside it.

#![cfg(all(feature = "ui", feature = "sqlite"))]

use std::sync::{Arc, Mutex};

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::SqliteBackend;
use helios_rest::{ServerConfig, config::BulkSubmitConfig};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

// A throwaway P-384 PKCS#8 key (ES384), generated solely for tests. It is the
// key `helios-rest`'s `bulk_submit_oauth` unit tests use, so the thumbprint
// below is the value they pin.
const TEST_ES384_PEM: &str = "-----BEGIN PRIVATE KEY-----
MIG2AgEAMBAGByqGSM49AgEGBSuBBAAiBIGeMIGbAgEBBDDL2uAdbDMvTFQ/r+Tz
45fPr+tTiZPaq7mKcF2g1fmOCRMSdkSMLaRBzWbIwOE8GxOhZANiAASjv7NSBHxr
k5K6g75uWMf/NsVu7ay2lO8vuu9EC12UBjDQ+297+9I7+IlSzRmoWKUT0fVrOeoB
gMMPIjEueYREkCGj6cILnLaP5hAey+Q7JjRgVO6Xqf0u2uCO6yn8UTU=
-----END PRIVATE KEY-----
";

// RFC 7638 thumbprint of `TEST_ES384_PEM`, computed outside either crate
// (`openssl ec -text`, then Python `hashlib.sha256` over
// `{"crv":"P-384","kty":"EC","x":"<x>","y":"<y>"}`).
const TEST_ES384_KID: &str = "RIm6zOzFR3cMAG9PzzAubmfag2C2EKZa4w_Xnb_fuQM";

/// Decodes unpadded base64url, enough to read a JWT header segment.
fn b64url_decode(segment: &str) -> Vec<u8> {
    let (mut out, mut acc, mut bits) = (Vec::new(), 0u32, 0u32);
    for c in segment.bytes() {
        let value = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            other => panic!("not base64url: {other:#x}"),
        };
        acc = (acc << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    out
}

fn percent_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

/// A stand-in authorization server whose `/token` endpoint keeps the client
/// assertion it was sent. Returns the token URL and the capture.
async fn mock_token_endpoint() -> (String, Arc<Mutex<Option<String>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen: Arc<Mutex<Option<String>>> = Arc::default();
    let capture = Arc::clone(&seen);
    let app = Router::new().route(
        "/token",
        axum::routing::post(move |body: String| {
            let capture = Arc::clone(&capture);
            async move {
                // A JWT is base64url and dots, which a form encoder leaves alone.
                *capture.lock().unwrap() = body
                    .split('&')
                    .find_map(|pair| pair.strip_prefix("client_assertion="))
                    .map(str::to_string);
                axum::Json(json!({ "access_token": "tok" }))
            }
        }),
    );
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}/token"), seen)
}

/// The `kid` the UI's signing path puts in the assertion header.
async fn kid_from_the_ui_assertion() -> Option<String> {
    let backend = Arc::new({
        let backend = SqliteBackend::in_memory().expect("in-memory sqlite");
        backend.init_schema().expect("init schema");
        backend
    });
    let ui = helios_ui::mount_with_conformance_source(
        Router::new(),
        "9.9.9",
        None,
        helios_ui::NlSearch::default(),
        None,
        Some(backend.clone()),
        "default".to_string(),
        Arc::new(helios_ui::StaticConformanceSource::empty()),
        FhirVersion::default_enabled(),
        None,
        "http://localhost:9".to_string(),
        Some(backend),
    );

    let (token_url, seen) = mock_token_endpoint().await;
    let response = ui
        .oneshot(
            Request::post("/ui/bulk-import/test-auth")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(format!(
                    "client_id=alice&token_url={}",
                    percent_encode(&token_url)
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let assertion = seen
        .lock()
        .unwrap()
        .clone()
        .expect("the UI posted a client assertion to the token endpoint");
    let header_segment = assertion.split('.').next().expect("JWT header segment");
    let header: Value = serde_json::from_slice(&b64url_decode(header_segment)).unwrap();
    assert_eq!(header["alg"], "ES384");
    header["kid"].as_str().map(str::to_string)
}

/// The `kid` of the key `GET /.well-known/bulk-submit-jwks.json` serves.
async fn kid_from_the_jwks_endpoint() -> String {
    let backend = SqliteBackend::in_memory().expect("in-memory sqlite");
    backend.init_schema().expect("init schema");
    let app = helios_rest::create_app_with_config(
        backend,
        ServerConfig {
            bulk_submit: BulkSubmitConfig {
                private_key: Some(TEST_ES384_PEM.to_string()),
                signing_alg: "ES384".to_string(),
                ..BulkSubmitConfig::default()
            },
            ..ServerConfig::for_testing()
        },
    );
    let response = app
        .oneshot(
            Request::get("/.well-known/bulk-submit-jwks.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    let keys = body["keys"].as_array().expect("`keys` array");
    assert_eq!(keys.len(), 1, "{body}");
    keys[0]["kid"].as_str().expect("`kid`").to_string()
}

#[tokio::test]
async fn the_ui_signer_and_the_jwks_endpoint_agree_on_the_kid() {
    // The UI reads its key from the process environment, as the server does.
    unsafe {
        std::env::set_var("HFS_BULK_SUBMIT_PRIVATE_KEY", TEST_ES384_PEM);
        std::env::remove_var("HFS_BULK_SUBMIT_SIGNING_ALG");
    }

    let from_ui = kid_from_the_ui_assertion().await;
    let from_jwks = kid_from_the_jwks_endpoint().await;

    unsafe { std::env::remove_var("HFS_BULK_SUBMIT_PRIVATE_KEY") };

    assert_eq!(
        from_ui.as_deref(),
        Some(from_jwks.as_str()),
        "the assertion's kid must name the key the endpoint serves"
    );
    // And both are the independently computed RFC 7638 thumbprint.
    assert_eq!(from_jwks, TEST_ES384_KID);
}
