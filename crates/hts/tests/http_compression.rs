//! HTTP compression tests for the HTS server.
//!
//! Verifies the middleware stack assembled in `create_app`:
//!
//! - Request bodies sent with `Content-Encoding: gzip` are decompressed
//!   before parsing.
//! - Invalid compressed bodies produce a 4xx, unsupported encodings a 415.
//! - `HTS_MAX_BODY_SIZE` applies to the *decompressed* body, so a small
//!   highly-compressed payload cannot bypass the limit.
//! - Responses are gzip-compressed when the client sends
//!   `Accept-Encoding: gzip` (with `Content-Encoding` and
//!   `Vary: Accept-Encoding` set) and left identity-encoded otherwise.

#![cfg(feature = "sqlite")]

use axum::{
    Router,
    body::Body,
    http::{Request, Response, StatusCode},
};
use helios_hts::{backends::SqliteTerminologyBackend, config::HtsConfig, state::AppState};
use serde_json::{Value, json};
use tower::ServiceExt;

fn app_with_config(config: HtsConfig) -> Router {
    let backend =
        SqliteTerminologyBackend::in_memory().expect("failed to create in-memory HTS backend");
    let state = AppState::new(backend);
    helios_hts::server::create_app(&config, state)
}

fn test_app() -> Router {
    app_with_config(HtsConfig::default())
}

fn code_system_json() -> Value {
    json!({
        "resourceType": "CodeSystem",
        "url": "http://hts.test/compression-cs",
        "status": "active",
        "content": "complete",
        "concept": [
            {"code": "A", "display": "Alpha"},
            {"code": "B", "display": "Beta"}
        ]
    })
}

/// A minimal import Bundle wrapping the test CodeSystem.
fn import_bundle_json() -> Value {
    json!({
        "resourceType": "Bundle",
        "type": "collection",
        "entry": [{"resource": code_system_json()}]
    })
}

fn gzip_bytes(input: &[u8]) -> Vec<u8> {
    use flate2::{Compression, write::GzEncoder};
    use std::io::Write;

    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(input).unwrap();
    encoder.finish().unwrap()
}

fn gunzip_bytes(input: &[u8]) -> Vec<u8> {
    use flate2::read::GzDecoder;
    use std::io::Read;

    let mut decoder = GzDecoder::new(input);
    let mut output = Vec::new();
    decoder.read_to_end(&mut output).unwrap();
    output
}

async fn body_bytes(response: Response<Body>) -> Vec<u8> {
    axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec()
}

// =============================================================================
// Request decompression
// =============================================================================

#[tokio::test]
async fn gzip_request_body_is_decompressed() {
    let app = test_app();
    let body = serde_json::to_vec(&import_bundle_json()).unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/import")
                .header("content-type", "application/fhir+json")
                .header("content-encoding", "gzip")
                .body(Body::from(gzip_bytes(&body)))
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(
        response.status().is_success(),
        "gzip import failed: {}",
        response.status()
    );
    let json: Value = serde_json::from_slice(&body_bytes(response).await).unwrap();
    assert_eq!(json["code_systems"], 1, "import stats were: {json}");
}

#[tokio::test]
async fn uncompressed_request_still_works() {
    let app = test_app();
    let body = serde_json::to_vec(&import_bundle_json()).unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/import")
                .header("content-type", "application/fhir+json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(
        response.status().is_success(),
        "plain import failed: {}",
        response.status()
    );
}

#[tokio::test]
async fn invalid_gzip_request_body_is_client_error() {
    let app = test_app();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/import")
                .header("content-type", "application/fhir+json")
                .header("content-encoding", "gzip")
                .body(Body::from("this is not gzip"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(
        response.status().is_client_error(),
        "invalid gzip body must produce a 4xx, got {}",
        response.status()
    );
}

#[tokio::test]
async fn unsupported_content_encoding_is_rejected() {
    let app = test_app();
    let body = serde_json::to_vec(&import_bundle_json()).unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/import")
                .header("content-type", "application/fhir+json")
                .header("content-encoding", "compress")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
}

#[tokio::test]
async fn body_limit_applies_to_decompressed_size() {
    let app = app_with_config(HtsConfig {
        max_body_size: 1024,
        ..HtsConfig::default()
    });

    // ~64 KiB of repeated text compresses to well under the 1 KiB limit,
    // but the decompressed body must still be rejected with 413.
    let mut bundle = import_bundle_json();
    bundle["entry"][0]["resource"]["description"] = json!("a".repeat(64 * 1024));
    let raw = serde_json::to_vec(&bundle).unwrap();
    let compressed = gzip_bytes(&raw);
    assert!(
        compressed.len() < 1024,
        "test payload must compress below the limit"
    );

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/import")
                .header("content-type", "application/fhir+json")
                .header("content-encoding", "gzip")
                .body(Body::from(compressed))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn body_limit_applies_to_plain_requests() {
    let app = app_with_config(HtsConfig {
        max_body_size: 1024,
        ..HtsConfig::default()
    });

    let mut bundle = import_bundle_json();
    bundle["entry"][0]["resource"]["description"] = json!("a".repeat(64 * 1024));
    let raw = serde_json::to_vec(&bundle).unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/import")
                .header("content-type", "application/fhir+json")
                .body(Body::from(raw))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

// =============================================================================
// Response compression
// =============================================================================

#[tokio::test]
async fn response_is_gzip_compressed_on_accept_encoding() {
    let app = test_app();

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/metadata")
                .header("accept-encoding", "gzip")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers().get("content-encoding").unwrap(), "gzip");
    let vary = response
        .headers()
        .get_all("vary")
        .iter()
        .map(|v| v.to_str().unwrap().to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join(",");
    assert!(vary.contains("accept-encoding"), "Vary was: {vary}");

    let decompressed = gunzip_bytes(&body_bytes(response).await);
    let json: Value = serde_json::from_slice(&decompressed).unwrap();
    assert_eq!(json["resourceType"], "CapabilityStatement");
}

#[tokio::test]
async fn response_is_not_compressed_without_accept_encoding() {
    let app = test_app();

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/metadata")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers().get("content-encoding").is_none());
    let json: Value = serde_json::from_slice(&body_bytes(response).await).unwrap();
    assert_eq!(json["resourceType"], "CapabilityStatement");
}
