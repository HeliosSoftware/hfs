//! HTTP compression conformance tests.
//!
//! Verifies the middleware stack assembled in `create_app_with_config`:
//!
//! - Request bodies sent with `Content-Encoding: gzip`/`deflate` are
//!   decompressed before parsing.
//! - Invalid compressed bodies produce a 4xx, unsupported encodings a 415.
//! - `HFS_MAX_BODY_SIZE` applies to the *decompressed* body, so a small
//!   highly-compressed payload cannot bypass the limit.
//! - Responses are gzip-compressed when the client sends
//!   `Accept-Encoding: gzip` (with `Content-Encoding` and
//!   `Vary: Accept-Encoding` set) and left identity-encoded otherwise.
//! - CORS preflight allows the `Content-Encoding` request header.

use std::path::PathBuf;

use axum::http::{Method, StatusCode};
use axum_test::TestServer;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_rest::ServerConfig;
use serde_json::{Value, json};

/// Creates a test server running the full middleware stack.
fn create_test_server(config: ServerConfig) -> TestServer {
    // Configure with data directory to load spec SearchParameters
    // CARGO_MANIFEST_DIR for this test is crates/rest
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("data"))
        .unwrap_or_else(|| PathBuf::from("data"));

    let backend_config = SqliteBackendConfig {
        data_dir: Some(data_dir),
        ..Default::default()
    };
    let backend = SqliteBackend::with_config(":memory:", backend_config)
        .expect("Failed to create SQLite backend");
    backend.init_schema().expect("Failed to init schema");

    let app = helios_rest::create_app_with_config(backend, config);
    TestServer::new(app).expect("Failed to create test server")
}

fn test_config() -> ServerConfig {
    ServerConfig {
        default_tenant: "test-tenant".to_string(),
        ..ServerConfig::for_testing()
    }
}

fn patient_json() -> Value {
    json!({
        "resourceType": "Patient",
        "name": [{"family": "Compressed"}],
        "active": true
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

// =============================================================================
// Request decompression
// =============================================================================

#[tokio::test]
async fn test_gzip_request_body_is_decompressed() {
    let server = create_test_server(test_config());
    let body = serde_json::to_vec(&patient_json()).unwrap();

    let response = server
        .post("/Patient")
        .add_header("x-tenant-id", "test-tenant")
        .add_header("content-encoding", "gzip")
        .content_type("application/fhir+json")
        .bytes(gzip_bytes(&body).into())
        .await;

    assert_eq!(response.status_code(), StatusCode::CREATED);
    let json: Value = response.json();
    assert_eq!(json["resourceType"], "Patient");
    assert_eq!(json["name"][0]["family"], "Compressed");
}

#[tokio::test]
async fn test_uncompressed_request_still_works() {
    let server = create_test_server(test_config());

    let response = server
        .post("/Patient")
        .add_header("x-tenant-id", "test-tenant")
        .content_type("application/fhir+json")
        .json(&patient_json())
        .await;

    assert_eq!(response.status_code(), StatusCode::CREATED);
}

#[tokio::test]
async fn test_invalid_gzip_request_body_is_client_error() {
    let server = create_test_server(test_config());

    let response = server
        .post("/Patient")
        .add_header("x-tenant-id", "test-tenant")
        .add_header("content-encoding", "gzip")
        .content_type("application/fhir+json")
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
    let server = create_test_server(test_config());
    let body = serde_json::to_vec(&patient_json()).unwrap();

    let response = server
        .post("/Patient")
        .add_header("x-tenant-id", "test-tenant")
        .add_header("content-encoding", "compress")
        .content_type("application/fhir+json")
        .bytes(body.into())
        .await;

    assert_eq!(response.status_code(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
}

#[tokio::test]
async fn test_body_limit_applies_to_decompressed_size() {
    let config = ServerConfig {
        max_body_size: 1024,
        ..test_config()
    };
    let server = create_test_server(config);

    // ~64 KiB of repeated text compresses to well under the 1 KiB limit,
    // but the decompressed body must still be rejected with 413.
    let mut patient = patient_json();
    patient["name"][0]["family"] = json!("a".repeat(64 * 1024));
    let raw = serde_json::to_vec(&patient).unwrap();
    let compressed = gzip_bytes(&raw);
    assert!(
        compressed.len() < 1024,
        "test payload must compress below the limit"
    );

    let response = server
        .post("/Patient")
        .add_header("x-tenant-id", "test-tenant")
        .add_header("content-encoding", "gzip")
        .content_type("application/fhir+json")
        .bytes(compressed.into())
        .await;

    assert_eq!(response.status_code(), StatusCode::PAYLOAD_TOO_LARGE);
}

// =============================================================================
// Response compression
// =============================================================================

#[tokio::test]
async fn test_response_is_gzip_compressed_on_accept_encoding() {
    let server = create_test_server(test_config());

    let response = server
        .get("/metadata")
        .add_header("x-tenant-id", "test-tenant")
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
    assert_eq!(json["resourceType"], "CapabilityStatement");
}

/// Parquet output is already compressed — the response-compression layer
/// must skip it (for both parquet media-type identifiers; HFS emits the
/// spec-native `application/vnd.apache.parquet`).
#[tokio::test]
async fn test_parquet_response_is_not_http_compressed() {
    let server = create_test_server(test_config());

    let body = json!({
        "resourceType": "Parameters",
        "parameter": [
            {
                "name": "viewResource",
                "resource": {
                    "resourceType": "ViewDefinition",
                    "status": "active",
                    "resource": "Patient",
                    "select": [{"column": [{"name": "id", "path": "id", "type": "id"}]}]
                }
            },
            {
                "name": "resource",
                "resource": {"resourceType": "Patient", "id": "pq-1", "active": true}
            }
        ]
    });

    let response = server
        .post("/ViewDefinition/$viewdefinition-run?_format=parquet")
        .add_header("x-tenant-id", "test-tenant")
        .add_header("accept-encoding", "gzip")
        .json(&body)
        .await;

    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "{}",
        response.text()
    );
    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "application/vnd.apache.parquet"
    );
    // Parquet is already compressed — the gzip layer must skip it.
    assert!(
        response.headers().get("content-encoding").is_none(),
        "parquet response must not be re-compressed"
    );
    assert!(
        response.as_bytes().starts_with(b"PAR1"),
        "body must be raw parquet bytes"
    );
}

#[tokio::test]
async fn test_response_is_not_compressed_without_accept_encoding() {
    let server = create_test_server(test_config());

    let response = server
        .get("/metadata")
        .add_header("x-tenant-id", "test-tenant")
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    assert!(response.headers().get("content-encoding").is_none());
    let json: Value = response.json();
    assert_eq!(json["resourceType"], "CapabilityStatement");
}

// =============================================================================
// CORS
// =============================================================================

#[tokio::test]
async fn test_cors_preflight_allows_content_encoding() {
    // Default config: CORS enabled with the explicit default header list.
    let config = ServerConfig {
        default_tenant: "test-tenant".to_string(),
        ..ServerConfig::default()
    };
    let server = create_test_server(config);

    let response = server
        .method(Method::OPTIONS, "/Patient")
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
