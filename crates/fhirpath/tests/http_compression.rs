//! HTTP compression conformance tests for the FHIRPath server.
//!
//! Verifies the middleware stack assembled in `create_app`:
//!
//! - Request bodies sent with `Content-Encoding: gzip`/`deflate` are
//!   decompressed before parsing.
//! - Invalid compressed bodies produce a 4xx, unsupported encodings a 415.
//! - `FHIRPATH_MAX_BODY_SIZE` applies to the *decompressed* body, so a small
//!   highly-compressed payload cannot bypass the limit.
//! - Responses are gzip-compressed when the client sends
//!   `Accept-Encoding: gzip` (with `Content-Encoding` and
//!   `Vary: Accept-Encoding` set) and left identity-encoded otherwise.
//! - CORS preflight allows the `Content-Encoding` request header.

use axum::http::{Method, StatusCode};
use axum_test::TestServer;
use helios_fhirpath::server::{ServerConfig, create_app};
use serde_json::{Value, json};

fn create_test_server(config: ServerConfig) -> TestServer {
    let app = create_app(&config);
    TestServer::new(app).expect("Failed to create test server")
}

/// A valid fhirpath-lab evaluation request: `Patient.name.family` over a
/// minimal Patient resource.
fn evaluation_request() -> Value {
    json!({
        "resourceType": "Parameters",
        "parameter": [
            {
                "name": "expression",
                "valueString": "Patient.name.family"
            },
            {
                "name": "resource",
                "resource": {
                    "resourceType": "Patient",
                    "name": [{ "family": "Compressed" }]
                }
            }
        ]
    })
}

fn gzip_bytes(input: &[u8]) -> Vec<u8> {
    use flate2::{Compression, write::GzEncoder};
    use std::io::Write;

    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(input).unwrap();
    encoder.finish().unwrap()
}

fn deflate_bytes(input: &[u8]) -> Vec<u8> {
    use flate2::{Compression, write::ZlibEncoder};
    use std::io::Write;

    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
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

// =============================================================================
// Request decompression
// =============================================================================

#[tokio::test]
async fn test_gzip_request_body_is_decompressed() {
    let server = create_test_server(ServerConfig::default());
    let body = serde_json::to_vec(&evaluation_request()).unwrap();

    let response = server
        .post("/")
        .add_header("content-encoding", "gzip")
        .content_type("application/json")
        .bytes(gzip_bytes(&body).into())
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let json: Value = response.json();
    assert_eq!(json["resourceType"], "Parameters");
}

#[tokio::test]
async fn test_deflate_request_body_is_decompressed() {
    let server = create_test_server(ServerConfig::default());
    let body = serde_json::to_vec(&evaluation_request()).unwrap();

    let response = server
        .post("/")
        .add_header("content-encoding", "deflate")
        .content_type("application/json")
        .bytes(deflate_bytes(&body).into())
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let json: Value = response.json();
    assert_eq!(json["resourceType"], "Parameters");
}

#[tokio::test]
async fn test_uncompressed_request_still_works() {
    let server = create_test_server(ServerConfig::default());

    let response = server
        .post("/")
        .content_type("application/json")
        .json(&evaluation_request())
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let json: Value = response.json();
    assert_eq!(json["resourceType"], "Parameters");
}

#[tokio::test]
async fn test_invalid_gzip_request_body_is_client_error() {
    let server = create_test_server(ServerConfig::default());

    let response = server
        .post("/")
        .add_header("content-encoding", "gzip")
        .content_type("application/json")
        .bytes(b"this is not gzip".to_vec().into())
        .await;

    assert!(
        response.status_code().is_client_error(),
        "invalid gzip body must produce a 4xx, got {}",
        response.status_code()
    );
}

#[tokio::test]
async fn test_unsupported_content_encoding_is_rejected() {
    let server = create_test_server(ServerConfig::default());
    let body = serde_json::to_vec(&evaluation_request()).unwrap();

    let response = server
        .post("/")
        .add_header("content-encoding", "compress")
        .content_type("application/json")
        .bytes(body.into())
        .await;

    assert_eq!(response.status_code(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
}

#[tokio::test]
async fn test_body_limit_applies_to_decompressed_size() {
    let config = ServerConfig {
        max_body_size: 1024,
        ..ServerConfig::default()
    };
    let server = create_test_server(config);

    // ~64 KiB of repeated text compresses to well under the 1 KiB limit,
    // but the decompressed body must still be rejected with 413.
    let mut request = evaluation_request();
    request["parameter"][0]["valueString"] = json!(format!("'{}'", "a".repeat(64 * 1024)));
    let raw = serde_json::to_vec(&request).unwrap();
    let compressed = gzip_bytes(&raw);
    assert!(
        compressed.len() < 1024,
        "test payload must compress below the limit"
    );

    let response = server
        .post("/")
        .add_header("content-encoding", "gzip")
        .content_type("application/json")
        .bytes(compressed.into())
        .await;

    assert_eq!(response.status_code(), StatusCode::PAYLOAD_TOO_LARGE);
}

// =============================================================================
// Response compression
// =============================================================================

#[tokio::test]
async fn test_response_is_gzip_compressed_on_accept_encoding() {
    let server = create_test_server(ServerConfig::default());

    let response = server
        .get("/health")
        .add_header("accept-encoding", "gzip")
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    assert_eq!(response.headers().get("content-encoding").unwrap(), "gzip");
    let vary = response
        .headers()
        .get_all("vary")
        .iter()
        .map(|v| v.to_str().unwrap().to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join(",");
    assert!(vary.contains("accept-encoding"), "Vary was: {vary}");

    let decompressed = gunzip_bytes(response.as_bytes());
    let json: Value = serde_json::from_slice(&decompressed).unwrap();
    assert_eq!(json["status"], "ok");
}

#[tokio::test]
async fn test_response_is_not_compressed_without_accept_encoding() {
    let server = create_test_server(ServerConfig::default());

    let response = server.get("/health").await;

    assert_eq!(response.status_code(), StatusCode::OK);
    assert!(response.headers().get("content-encoding").is_none());
    let json: Value = response.json();
    assert_eq!(json["status"], "ok");
}

// =============================================================================
// CORS
// =============================================================================

#[tokio::test]
async fn test_cors_preflight_allows_content_encoding() {
    let server = create_test_server(ServerConfig::default());

    let response = server
        .method(Method::OPTIONS, "/")
        .add_header("origin", "http://example.com")
        .add_header("access-control-request-method", "POST")
        .add_header("access-control-request-headers", "content-encoding")
        .await;

    assert!(
        response.status_code().is_success(),
        "preflight failed: {}",
        response.status_code()
    );
    let allow_headers = response
        .headers()
        .get("access-control-allow-headers")
        .expect("preflight response must list allowed headers")
        .to_str()
        .unwrap()
        .to_ascii_lowercase();
    assert!(
        allow_headers.contains("content-encoding"),
        "Access-Control-Allow-Headers was: {allow_headers}"
    );
}
