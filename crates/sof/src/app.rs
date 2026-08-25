//! Axum application wiring for the `sof-server` binary.
//!
//! This module builds the same router the `sof-server` binary serves, so
//! that integration tests exercise production wiring instead of a
//! hand-maintained stub. The binary (`src/server.rs`) is left with only
//! argument parsing and `main`; everything that decides *what* gets served
//! lives here.

use axum::{
    Router,
    routing::{get, post},
};
use http::{HeaderValue, Method};
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::info;

use crate::handlers;

/// Server configuration options
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Port to bind the server to
    pub port: u16,
    /// Host address to bind to
    pub host: String,
    /// Log level for the server
    pub log_level: String,
    /// Maximum request body size in bytes
    pub max_body_size: usize,
    /// Request timeout in seconds
    pub request_timeout: u64,
    /// Whether to enable CORS
    pub enable_cors: bool,
    /// Allowed CORS origins (comma-separated list, "*" for any)
    pub cors_origins: String,
    /// Allowed CORS methods (comma-separated list, "*" for any)
    pub cors_methods: String,
    /// Allowed CORS headers (comma-separated list, "*" for any)
    pub cors_headers: String,
    /// Terminology server URL for FHIRPath terminology functions (memberOf, subsumes, etc.)
    pub terminology_server: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: 8080,
            host: "127.0.0.1".to_string(),
            log_level: "info".to_string(),
            max_body_size: 10 * 1024 * 1024, // 10MB
            request_timeout: 30,
            enable_cors: true,
            cors_origins: "*".to_string(),
            cors_methods: "GET,POST,PUT,DELETE,OPTIONS".to_string(),
            cors_headers: "Accept,Accept-Language,Content-Type,Content-Language,Authorization,X-Requested-With,Content-Encoding".to_string(),
            terminology_server: None,
        }
    }
}

/// Create the axum application with all routes and configuration
/// Create the application router with default configuration
/// This is used for testing and can be used for custom server implementations
pub fn create_app() -> Router {
    let config = ServerConfig::default();
    create_app_with_config(&config)
}

/// Create the application router using the given configuration.
///
/// This is what the `sof-server` binary calls after parsing its arguments,
/// and what integration tests call (via [`create_app`] or directly) to
/// exercise the real production router.
pub fn create_app_with_config(config: &ServerConfig) -> Router {
    use axum::extract::DefaultBodyLimit;
    use std::time::Duration;
    use tower::ServiceBuilder;
    use tower_http::compression::CompressionLayer;
    use tower_http::compression::predicate::{NotForContentType, Predicate, SizeAbove};
    use tower_http::decompression::RequestDecompressionLayer;
    use tower_http::timeout::TimeoutLayer;

    // Compress responses on `Accept-Encoding`, but never re-compress Parquet
    // or ZIP output â€” both are already compressed, so HTTP-level compression
    // would only burn CPU for no size win.
    let compress_predicate = SizeAbove::new(32)
        .and(NotForContentType::const_new("application/parquet"))
        .and(NotForContentType::const_new(
            "application/vnd.apache.parquet",
        ))
        .and(NotForContentType::const_new("application/zip"));

    let mut app = Router::new()
        // FHIR endpoints
        .route("/metadata", get(handlers::capability_statement))
        // This server's own OperationDefinition for `$sql-run`. It supports a
        // subset of the guide's parameters (no `subjectCanonical`,
        // `subjectReference`, `context` or `source`), and
        // operations-capability.html#partial-operation-support requires such a
        // server to publish its own definition, with `base` naming the guide's,
        // and to cite that from its CapabilityStatement.
        .route(
            "/OperationDefinition/sof-sql-run",
            get(handlers::sql_run_operation_definition),
        )
        // `$sql-run` is invoked at the **system level only**
        // (`system=true, type=false, instance=false`). The pre-ballot
        // continuous build also offered type- and instance-level
        // `$viewdefinition-run` endpoints; those were never published and are
        // gone.
        //
        // GET is permitted whenever every supplied parameter is primitive,
        // which is what keeps the operation usable from a browser or a command
        // line. sof-server is stateless and resolves no subject by URL, so GET
        // will normally surface a 400 â€” but the route exists so clients can
        // negotiate the method correctly.
        .route(
            "/$sql-run",
            post(handlers::sql_run_handler).get(handlers::sql_run_handler),
        )
        // Health check endpoint
        .route("/health", get(handlers::health_check))
        // Add body size limit. The decompression layer below replaces the
        // request body before extractors read it, so this limit applies to
        // the *decompressed* bytes â€” a small highly-compressed payload
        // cannot bypass SOF_MAX_BODY_SIZE.
        .layer(DefaultBodyLimit::max(config.max_body_size))
        // Decompress request bodies sent with `Content-Encoding` (gzip,
        // deflate, br, zstd); unsupported encodings get 415.
        .layer(RequestDecompressionLayer::new())
        .layer(CompressionLayer::new().compress_when(compress_predicate))
        // Add request timeout
        .layer(
            ServiceBuilder::new()
                .layer(TimeoutLayer::with_status_code(
                    http::StatusCode::REQUEST_TIMEOUT,
                    Duration::from_secs(config.request_timeout),
                ))
                .into_inner(),
        );

    // Add CORS if enabled
    if config.enable_cors {
        app = app.layer(build_cors_layer(config));
    }

    // Add tracing
    app = app.layer(TraceLayer::new_for_http());

    // Observability: `/metrics` (state-free) + per-request metrics/trace span.
    app = app
        .merge(helios_observability::metrics::router())
        .layer(axum::middleware::from_fn(
            helios_observability::middleware::track,
        ));

    app
}

/// Build CORS layer from configuration
///
/// This function creates a CORS middleware layer based on the server configuration.
/// It supports flexible CORS configuration:
///
/// - **Origins**: Use "*" for any origin, or provide a comma-separated list of allowed origins
/// - **Methods**: Use "*" for any method, or provide a comma-separated list (e.g., "GET,POST,OPTIONS")
/// - **Headers**: Use "*" for any header, or provide a comma-separated list of allowed headers
///
/// # Examples
///
/// ```text
/// # Allow any origin, method, and header (without credentials)
/// cors_origins = "*"
/// cors_methods = "*"
/// cors_headers = "*"
///
/// # Allow specific origins (with credentials)
/// cors_origins = "https://example.com,https://app.example.com"
///
/// # Allow specific methods
/// cors_methods = "GET,POST,OPTIONS"
///
/// # Allow specific headers
/// cors_headers = "Content-Type,Authorization,X-Requested-With"
/// ```
///
/// Note: When using wildcards (*), credentials are disabled for security.
/// To use credentials, specify exact origins, methods, and headers.
fn build_cors_layer(config: &ServerConfig) -> CorsLayer {
    use tower_http::cors::{AllowHeaders, AllowMethods, AllowOrigin};

    let mut cors = CorsLayer::new();

    // Check if we're using wildcards
    let using_wildcard_origin = config.cors_origins == "*";
    let using_wildcard_methods = config.cors_methods == "*";
    let using_wildcard_headers = config.cors_headers == "*";
    let using_any_wildcard =
        using_wildcard_origin || using_wildcard_methods || using_wildcard_headers;

    // Configure origins
    if using_wildcard_origin {
        cors = cors.allow_origin(AllowOrigin::any());
    } else {
        let origins: Vec<HeaderValue> = config
            .cors_origins
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .filter_map(|s| HeaderValue::from_str(s).ok())
            .collect();
        cors = cors.allow_origin(origins);
    }

    // Configure methods
    if using_wildcard_methods {
        cors = cors.allow_methods(AllowMethods::any());
    } else {
        let methods: Vec<Method> = config
            .cors_methods
            .split(',')
            .map(|s| s.trim().to_uppercase())
            .filter(|s| !s.is_empty())
            .filter_map(|s| Method::from_bytes(s.as_bytes()).ok())
            .collect();
        cors = cors.allow_methods(methods);
    }

    // Configure headers
    if using_wildcard_headers {
        cors = cors.allow_headers(AllowHeaders::any());
    } else {
        let headers: Vec<http::HeaderName> = config
            .cors_headers
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.parse().ok())
            .collect();
        cors = cors.allow_headers(headers);
    }

    // Only allow credentials if not using wildcards
    if !using_any_wildcard {
        cors = cors.allow_credentials(true);
    } else {
        // Log a warning if wildcards are used
        info!("CORS: Using wildcards, credentials are disabled for security");
    }

    cors
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use axum_test::TestServer;

    #[tokio::test]
    async fn test_health_check() {
        let config = ServerConfig::default();
        let app = create_app_with_config(&config);
        let server = TestServer::new(app).unwrap();

        let response = server.get("/health").await;

        assert_eq!(response.status_code(), StatusCode::OK);

        let json: serde_json::Value = response.json();
        assert_eq!(json["status"], "ok");
        assert_eq!(json["service"], "sof-server");
    }

    // â”€â”€ Unsupported `_format` â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    /// Spec (operations-common, Output Formats): an unsupported `_format`
    /// value SHALL be rejected with 400 Bad Request + OperationOutcome â€”
    /// for the body parameter as well as the query parameter. (The stub
    /// suite in `tests/` used to entrench 415 for the body path.)
    #[tokio::test]
    async fn test_unsupported_body_format_returns_400() {
        let server = TestServer::new(create_app()).unwrap();

        let mut body = run_request_body();
        body["parameter"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"name": "_format", "valueCode": "text/plain"}));

        let response = server.post("/$sql-run").json(&body).await;

        assert_eq!(
            response.status_code(),
            StatusCode::BAD_REQUEST,
            "unsupported body _format must be 400, got {}: {}",
            response.status_code(),
            response.text()
        );
        let json: serde_json::Value = response.json();
        assert_eq!(json["resourceType"], "OperationOutcome");
    }

    // â”€â”€ HTTP compression â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    /// A minimal valid `$viewdefinition-run` Parameters body.
    fn run_request_body() -> serde_json::Value {
        serde_json::json!({
            "resourceType": "Parameters",
            "parameter": [
                {
                    "name": "subjectResource",
                    "resource": {
                        "resourceType": "ViewDefinition",
                        "status": "active",
                        "resource": "Patient",
                        "select": [{
                            "column": [
                                {"name": "id", "path": "id"},
                                {"name": "gender", "path": "gender"}
                            ]
                        }]
                    }
                },
                {
                    "name": "resource",
                    "resource": {
                        "resourceType": "Patient",
                        "id": "example",
                        "gender": "male"
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

    fn gunzip_bytes(input: &[u8]) -> Vec<u8> {
        use flate2::read::GzDecoder;
        use std::io::Read;

        let mut decoder = GzDecoder::new(input);
        let mut output = Vec::new();
        decoder.read_to_end(&mut output).unwrap();
        output
    }

    #[tokio::test]
    async fn test_gzip_request_body_is_decompressed() {
        let server = TestServer::new(create_app()).unwrap();
        let body = serde_json::to_vec(&run_request_body()).unwrap();

        let response = server
            .post("/$sql-run")
            .add_header("content-encoding", "gzip")
            .add_header("accept", "application/json")
            .content_type("application/json")
            .bytes(gzip_bytes(&body).into())
            .await;

        assert_eq!(response.status_code(), StatusCode::OK);
        let json: serde_json::Value = response.json();
        assert_eq!(json[0]["id"], "example");
    }

    #[tokio::test]
    async fn test_deflate_request_body_is_decompressed() {
        use flate2::{Compression, write::ZlibEncoder};
        use std::io::Write;

        let server = TestServer::new(create_app()).unwrap();
        let body = serde_json::to_vec(&run_request_body()).unwrap();
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&body).unwrap();
        let compressed = encoder.finish().unwrap();

        let response = server
            .post("/$sql-run")
            .add_header("content-encoding", "deflate")
            .add_header("accept", "application/json")
            .content_type("application/json")
            .bytes(compressed.into())
            .await;

        assert_eq!(response.status_code(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_uncompressed_request_still_works() {
        let server = TestServer::new(create_app()).unwrap();

        let response = server
            .post("/$sql-run")
            .add_header("accept", "application/json")
            .json(&run_request_body())
            .await;

        assert_eq!(response.status_code(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_invalid_gzip_request_body_is_client_error() {
        let server = TestServer::new(create_app()).unwrap();

        let response = server
            .post("/$sql-run")
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
        let server = TestServer::new(create_app()).unwrap();
        let body = serde_json::to_vec(&run_request_body()).unwrap();

        let response = server
            .post("/$sql-run")
            .add_header("content-encoding", "compress")
            .content_type("application/json")
            .bytes(body.into())
            .await;

        assert_eq!(response.status_code(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    #[tokio::test]
    async fn test_response_is_gzip_compressed_on_accept_encoding() {
        let server = TestServer::new(create_app()).unwrap();

        let response = server
            .get("/metadata")
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
        let json: serde_json::Value = serde_json::from_slice(&decompressed).unwrap();
        assert_eq!(json["resourceType"], "CapabilityStatement");
    }

    #[tokio::test]
    async fn test_response_is_not_compressed_without_accept_encoding() {
        let server = TestServer::new(create_app()).unwrap();

        let response = server.get("/metadata").await;

        assert_eq!(response.status_code(), StatusCode::OK);
        assert!(response.headers().get("content-encoding").is_none());
        let json: serde_json::Value = response.json();
        assert_eq!(json["resourceType"], "CapabilityStatement");
    }

    #[tokio::test]
    async fn test_parquet_response_is_not_http_compressed() {
        let server = TestServer::new(create_app()).unwrap();
        let mut body = run_request_body();
        body["parameter"].as_array_mut().unwrap().insert(
            0,
            serde_json::json!({"name": "_format", "valueCode": "application/parquet"}),
        );

        let response = server
            .post("/$sql-run")
            .add_header("accept-encoding", "gzip")
            .json(&body)
            .await;

        assert_eq!(response.status_code(), StatusCode::OK);
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "application/vnd.apache.parquet"
        );
        // Parquet is already compressed â€” the gzip layer must skip it.
        assert!(response.headers().get("content-encoding").is_none());
        assert!(response.as_bytes().starts_with(b"PAR1"));
    }

    #[tokio::test]
    async fn test_body_limit_applies_to_decompressed_size() {
        let config = ServerConfig {
            max_body_size: 1024,
            ..Default::default()
        };
        let server = TestServer::new(create_app_with_config(&config)).unwrap();

        // ~64 KiB of repeated text compresses to well under the 1 KiB limit,
        // but the decompressed body must still be rejected with 413.
        let mut body = run_request_body();
        body["parameter"][1]["resource"]["name"] =
            serde_json::json!([{"family": "a".repeat(64 * 1024)}]);
        let raw = serde_json::to_vec(&body).unwrap();
        let compressed = gzip_bytes(&raw);
        assert!(
            compressed.len() < 1024,
            "test payload must compress below the limit"
        );

        let response = server
            .post("/$sql-run")
            .add_header("content-encoding", "gzip")
            .content_type("application/json")
            .bytes(compressed.into())
            .await;

        assert_eq!(response.status_code(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    // =========================================================================
    // SoF v2 Common Operation Behavior (spec PR #365): `fhir` output format,
    // Binary-envelope representation, and FHIR-XML rejection.
    // =========================================================================

    #[tokio::test]
    async fn test_fhir_format_returns_parameters() {
        let server = TestServer::new(create_app()).unwrap();
        let mut body = run_request_body();
        body["parameter"].as_array_mut().unwrap().insert(
            0,
            serde_json::json!({"name": "_format", "valueCode": "fhir"}),
        );

        let response = server.post("/$sql-run").json(&body).await;

        assert_eq!(
            response.status_code(),
            StatusCode::OK,
            "{}",
            response.text()
        );
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "application/fhir+json"
        );
        let v: serde_json::Value = response.json();
        assert_eq!(v["resourceType"], "Parameters");
        let rows = v["parameter"].as_array().expect("parameter array");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["name"], "row");
        let parts = rows[0]["part"].as_array().expect("row parts");
        assert!(
            parts
                .iter()
                .any(|p| p["name"] == "gender" && p["valueString"] == "male"),
            "row must carry the gender part: {v}"
        );
    }

    #[tokio::test]
    async fn test_accept_fhir_json_without_format_selects_fhir() {
        let server = TestServer::new(create_app()).unwrap();
        let response = server
            .post("/$sql-run")
            .add_header("accept", "application/fhir+json")
            .json(&run_request_body())
            .await;

        assert_eq!(
            response.status_code(),
            StatusCode::OK,
            "{}",
            response.text()
        );
        let v: serde_json::Value = response.json();
        assert_eq!(
            v["resourceType"], "Parameters",
            "Accept: application/fhir+json must select the fhir format: {v}"
        );
    }

    #[tokio::test]
    async fn test_accept_fhir_json_with_csv_format_returns_binary_envelope() {
        use base64::Engine as _;
        let server = TestServer::new(create_app()).unwrap();
        let mut body = run_request_body();
        body["parameter"].as_array_mut().unwrap().insert(
            0,
            serde_json::json!({"name": "_format", "valueCode": "csv"}),
        );

        let response = server
            .post("/$sql-run")
            .add_header("accept", "application/fhir+json")
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
            "application/fhir+json"
        );
        let v: serde_json::Value = response.json();
        assert_eq!(v["resourceType"], "Binary");
        assert_eq!(v["contentType"], "text/csv");
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(v["data"].as_str().expect("Binary.data"))
            .expect("Binary.data must be base64");
        let csv = String::from_utf8(decoded).expect("decoded csv is utf8");
        assert!(csv.contains("male"), "decoded csv: {csv}");
    }

    #[tokio::test]
    async fn test_accept_fhir_xml_returns_406() {
        let server = TestServer::new(create_app()).unwrap();
        let response = server
            .post("/$sql-run")
            .add_header("accept", "application/fhir+xml")
            .json(&run_request_body())
            .await;

        assert_eq!(
            response.status_code(),
            StatusCode::NOT_ACCEPTABLE,
            "{}",
            response.text()
        );
        let v: serde_json::Value = response.json();
        assert_eq!(v["resourceType"], "OperationOutcome");
    }
}
