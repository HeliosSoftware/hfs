//! # SQL-on-FHIR Server Implementation
//!
//! This module provides a stateless HTTP server implementation for the [SQL-on-FHIR
//! specification](https://sql-on-fhir.org/ig/latest),
//! enabling HTTP-based access to ViewDefinition transformation capabilities.  Use this module
//! if you need a stateless, simple web service for SQL-on-FHIR implementations.  Should you
//! need to perform SQL-on-FHIR transformations using server-stored ViewDefinitions and
//! server-stored FHIR data, use the full capabilities of the Helios FHIR Server in the [hfs](../hfs/index.html) module.
//!
//! ## Features
//!
//! - **HTTP API**: RESTful endpoints for ViewDefinition execution
//! - **CapabilityStatement**: Discovery endpoint for server capabilities
//! - **ViewDefinition Runner**: Synchronous execution of ViewDefinitions
//! - **Multi-format Output**: Support for CSV, JSON, and NDJSON responses
//! - **FHIR Version Support**: Handle requests for any supported FHIR version
//! - **Error Handling**: Comprehensive HTTP error responses with FHIR OperationOutcome
//! - **Configurable CORS**: Full control over CORS origins, methods, and headers
//! - **Parquet Support**: Advanced Parquet configuration with automatic file splitting
//! - **Streaming Response**: Chunked transfer encoding for large datasets
//! - **ZIP Archive**: Automatic ZIP packaging when multiple Parquet files are generated
//!
//! ## API Endpoints
//!
//! ```text
//! GET /metadata
//!   Returns: CapabilityStatement
//!
//! POST /ViewDefinition/$viewdefinition-run
//!   Body: Parameters resource containing ViewDefinition and data
//!   Query Parameters (except viewReference, viewResource, patient, group, resource):
//!     _format: Output format - application/json, application/x-ndjson, text/csv, application/octet-stream (parquet)
//!     header: CSV header control - true (default), false (only applies to CSV format)
//!     source: Data source (type: string) - Not yet supported
//!     _limit: Limits the number of results (1-10000)
//!     _since: Return resources modified after this time (RFC3339 format, validates format only)
//!     maxFileSize: Maximum Parquet file size in MB (10-10000) - splits into multiple files if exceeded
//!     rowGroupSize: Parquet row group size in MB (64-1024, default: 256)
//!     pageSize: Parquet page size in KB (64-8192, default: 1024)
//!     compression: Parquet compression (none, snappy, gzip, lz4, brotli, zstd, default: snappy)
//!   Body Parameters (in FHIR Parameters resource):
//!     _format: Output format (type: code or string)
//!     header: CSV header control (type: boolean)
//!     viewReference: Reference(s) to ViewDefinition(s) (type: Reference) - Not yet supported
//!     viewResource: ViewDefinition(s) to use (type: ViewDefinition)
//!     patient: Filter by patient (type: Reference)
//!     group: Filter by group (type: Reference) - Not yet supported
//!     source: Data source (type: string) - Not yet supported
//!     _limit: Result limit (type: integer)
//!     _since: Modification time filter (type: instant)
//!     resource: FHIR resources to transform (type: Resource)
//!     maxFileSize: Maximum Parquet file size in MB (type: integer)
//!     rowGroupSize: Parquet row group size in MB (type: integer)
//!     pageSize: Parquet page size in KB (type: integer)
//!     compression: Parquet compression algorithm (type: code or string)
//!   Returns: Transformed data in requested format
//!
//! ```
//!
//! ## Configuration
//!
//! The server supports configuration through both command-line arguments and environment variables:
//!
//! - `SOF_SERVER_PORT` / `--port`: Server port (default: 8080)
//! - `SOF_SERVER_HOST` / `--host`: Server host (default: 127.0.0.1)
//! - `SOF_LOG_LEVEL` / `--log-level`: Log level (default: info)
//! - `SOF_MAX_BODY_SIZE` / `--max-body-size`: Max request size in bytes (default: 10MB).
//!   Applies to the decompressed body for compressed requests.
//! - `SOF_REQUEST_TIMEOUT` / `--request-timeout`: Request timeout in seconds (default: 30)
//! - `SOF_ENABLE_CORS` / `--enable-cors`: Enable CORS (default: true)
//! - `SOF_CORS_ORIGINS` / `--cors-origins`: Allowed origins, comma-separated (default: *)
//! - `SOF_CORS_METHODS` / `--cors-methods`: Allowed methods, comma-separated (default: *)
//! - `SOF_CORS_HEADERS` / `--cors-headers`: Allowed headers, comma-separated (default: *)
//! - `SOF_TERMINOLOGY_SERVER` / `--terminology-server`: Terminology server URL for FHIRPath functions
//!
//! ## HTTP Compression
//!
//! Request bodies sent with `Content-Encoding: gzip` (or `deflate`, `br`,
//! `zstd`) are decompressed transparently before parsing; unsupported
//! encodings are rejected with `415 Unsupported Media Type`. Responses are
//! compressed when the client advertises support via `Accept-Encoding` —
//! except `application/parquet` and `application/zip` outputs, which are
//! already compressed and are returned as-is.
//!
//! ## CORS Configuration Examples
//!
//! ```bash
//! # Allow any origin (default)
//! sof-server --enable-cors true
//!
//! # Allow specific origins
//! sof-server --cors-origins "https://example.com,https://app.example.com"
//!
//! # Allow specific methods
//! sof-server --cors-methods "GET,POST,OPTIONS"
//!
//! # Allow specific headers
//! sof-server --cors-headers "Content-Type,Authorization,X-Requested-With"
//!
//! # Production configuration
//! SOF_ENABLE_CORS=true \
//! SOF_CORS_ORIGINS="https://app.example.com" \
//! SOF_CORS_METHODS="GET,POST,OPTIONS" \
//! SOF_CORS_HEADERS="Content-Type,Authorization" \
//! sof-server
//! ```

use axum::{
    Router,
    routing::{get, post},
};
use http::{HeaderValue, Method};
use std::net::SocketAddr;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::{info, warn};

mod error;
mod handlers;
mod models;
mod parquet_zip;

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

/// Main server entry point
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Parse command line arguments first to get log level
    let config = parse_args();

    // Initialize observability: logging/tracing (+ optional OTLP), uptime, and
    // the Prometheus metrics recorder.
    helios_observability::uptime::init();
    helios_observability::telemetry::init("sof-server", &config.log_level);
    helios_observability::metrics::init("sof-server");

    // Propagate SOF_TERMINOLOGY_SERVER to FHIRPATH_TERMINOLOGY_SERVER so that any
    // FHIRPath evaluation (memberOf, subsumes, etc.) delegates terminology
    // operations to the configured HTS instance.
    if let Some(ref ts_url) = config.terminology_server {
        if std::env::var("FHIRPATH_TERMINOLOGY_SERVER").is_err() {
            // SAFETY: called before any threads are spawned by the tokio runtime.
            unsafe {
                std::env::set_var("FHIRPATH_TERMINOLOGY_SERVER", ts_url);
            }
            info!(url = %ts_url, "SOF_TERMINOLOGY_SERVER wired to FHIRPath context");
        }
    }

    info!("Starting SQL-on-FHIR server...");
    info!("Configuration: {:?}", config);

    // Build the application router with configuration
    let app = create_app_with_config(&config);

    // Parse the host address
    let host: std::net::IpAddr = config.host.parse().unwrap_or_else(|_| {
        warn!("Invalid host address '{}', using 127.0.0.1", config.host);
        "127.0.0.1".parse().unwrap()
    });

    // Create the server address
    let addr = SocketAddr::from((host, config.port));
    info!("Server listening on {}", addr);

    // Create the server
    let listener = tokio::net::TcpListener::bind(addr).await?;

    // Start the server
    axum::serve(listener, app).await?;

    Ok(())
}

/// Parse command line arguments for server configuration
fn parse_args() -> ServerConfig {
    use clap::Parser;

    #[derive(Parser, Debug)]
    #[command(
        author,
        version,
        about = "SQL-on-FHIR HTTP server",
        long_about = "HTTP server providing SQL-on-FHIR ViewDefinition transformation capabilities\n\nEnvironment variables:\n  SOF_SERVER_PORT - Server port (default: 8080)\n  SOF_SERVER_HOST - Server host (default: 127.0.0.1)\n  SOF_LOG_LEVEL - Log level: error, warn, info, debug, trace (default: info)\n  SOF_MAX_BODY_SIZE - Maximum request body size in bytes (default: 10485760)\n  SOF_REQUEST_TIMEOUT - Request timeout in seconds (default: 30)\n  SOF_ENABLE_CORS - Enable CORS: true/false (default: true)\n  SOF_CORS_ORIGINS - Allowed origins (comma-separated, * for any) (default: *)\n  SOF_CORS_METHODS - Allowed methods (comma-separated, * for any) (default: GET,POST,PUT,DELETE,OPTIONS)\n  SOF_CORS_HEADERS - Allowed headers (comma-separated, * for any) (default: common headers)\n\nNote: When using wildcard (*) origins, credentials are disabled for security."
    )]
    struct Args {
        /// Port to bind the server to
        #[arg(short, long, env = "SOF_SERVER_PORT", default_value_t = 8080)]
        port: u16,

        /// Host address to bind to
        #[arg(
            short = 'H',
            long,
            env = "SOF_SERVER_HOST",
            default_value = "127.0.0.1"
        )]
        host: String,

        /// Log level (error, warn, info, debug, trace)
        #[arg(short, long, env = "SOF_LOG_LEVEL", default_value = "info")]
        log_level: String,

        /// Maximum request body size in bytes
        #[arg(
            short = 'm',
            long,
            env = "SOF_MAX_BODY_SIZE",
            default_value_t = 10_485_760
        )]
        max_body_size: usize,

        /// Request timeout in seconds
        #[arg(short = 't', long, env = "SOF_REQUEST_TIMEOUT", default_value_t = 30)]
        request_timeout: u64,

        /// Enable CORS
        #[arg(short = 'c', long, env = "SOF_ENABLE_CORS", default_value_t = true)]
        enable_cors: bool,

        /// Allowed CORS origins (comma-separated list, "*" for any)
        #[arg(long, env = "SOF_CORS_ORIGINS", default_value = "*")]
        cors_origins: String,

        /// Allowed CORS methods (comma-separated list, "*" for any)
        #[arg(
            long,
            env = "SOF_CORS_METHODS",
            default_value = "GET,POST,PUT,DELETE,OPTIONS"
        )]
        cors_methods: String,

        /// Allowed CORS headers (comma-separated list, "*" for any)
        #[arg(
            long,
            env = "SOF_CORS_HEADERS",
            default_value = "Accept,Accept-Language,Content-Type,Content-Language,Authorization,X-Requested-With,Content-Encoding"
        )]
        cors_headers: String,

        /// Terminology server URL for FHIRPath terminology functions (memberOf, subsumes, etc.)
        #[arg(long, env = "SOF_TERMINOLOGY_SERVER")]
        terminology_server: Option<String>,
    }

    let args = Args::parse();

    ServerConfig {
        port: args.port,
        host: args.host,
        log_level: args.log_level,
        max_body_size: args.max_body_size,
        request_timeout: args.request_timeout,
        enable_cors: args.enable_cors,
        cors_origins: args.cors_origins,
        cors_methods: args.cors_methods,
        cors_headers: args.cors_headers,
        terminology_server: args.terminology_server,
    }
}

/// Create the axum application with all routes and configuration
/// Create the application router with default configuration
/// This is used for testing and can be used for custom server implementations
pub fn create_app() -> Router {
    let config = ServerConfig::default();
    create_app_with_config(&config)
}

fn create_app_with_config(config: &ServerConfig) -> Router {
    use axum::extract::DefaultBodyLimit;
    use std::time::Duration;
    use tower::ServiceBuilder;
    use tower_http::compression::CompressionLayer;
    use tower_http::compression::predicate::{NotForContentType, Predicate, SizeAbove};
    use tower_http::decompression::RequestDecompressionLayer;
    use tower_http::timeout::TimeoutLayer;

    // Compress responses on `Accept-Encoding`, but never re-compress Parquet
    // or ZIP output — both are already compressed, so HTTP-level compression
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
        // SQL-on-FHIR capabilities (audit item #11): the spec-defined
        // `GET /$sql-on-fhir-capabilities` endpoint returning a Parameters
        // resource that enumerates which SoF features this server supports.
        // sof-server is stateless so most of the reference-resolution
        // capabilities are false; the truthful capability block lets
        // clients negotiate without trial-and-error.
        .route(
            "/$sql-on-fhir-capabilities",
            get(handlers::sof_capabilities),
        )
        // Per spec, GET is permitted for simple invocations (no
        // viewResource/resource body). sof-server is stateless and rejects
        // viewReference, so GET will normally surface a 400/501 — but the
        // route exists so clients can negotiate the method correctly.
        //
        // The SoF v2 OperationDefinition lists three valid endpoints:
        //   - [base]/$viewdefinition-run                            (system-level)
        //   - [base]/CanonicalResource/$viewdefinition-run          (type-level)
        //   - [base]/CanonicalResource/[id]/$viewdefinition-run     (instance-level)
        //
        // sof-server is stateless, so instance-level (which infers the
        // ViewDefinition from a stored {id}) is rejected with a clear 400
        // by `instance_level_not_supported`. The system- and type-level
        // endpoints both route to the same handler — they differ only in
        // URL shape (the type-level path is `CanonicalResource =
        // ViewDefinition`).
        .route(
            "/$viewdefinition-run",
            post(handlers::run_view_definition_handler).get(handlers::run_view_definition_handler),
        )
        .route(
            "/ViewDefinition/$viewdefinition-run",
            post(handlers::run_view_definition_handler).get(handlers::run_view_definition_handler),
        )
        .route(
            "/ViewDefinition/{id}/$viewdefinition-run",
            post(handlers::instance_level_not_supported)
                .get(handlers::instance_level_not_supported),
        )
        // Health check endpoint
        .route("/health", get(handlers::health_check))
        // Add body size limit. The decompression layer below replaces the
        // request body before extractors read it, so this limit applies to
        // the *decompressed* bytes — a small highly-compressed payload
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

    // ── Unsupported `_format` ─────────────────────────────────────────────

    /// Spec (operations-common, Output Formats): an unsupported `_format`
    /// value SHALL be rejected with 400 Bad Request + OperationOutcome —
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

        let response = server
            .post("/ViewDefinition/$viewdefinition-run")
            .json(&body)
            .await;

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

    // ── HTTP compression ──────────────────────────────────────────────────

    /// A minimal valid `$viewdefinition-run` Parameters body.
    fn run_request_body() -> serde_json::Value {
        serde_json::json!({
            "resourceType": "Parameters",
            "parameter": [
                {
                    "name": "viewResource",
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
            .post("/ViewDefinition/$viewdefinition-run")
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
            .post("/ViewDefinition/$viewdefinition-run")
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
            .post("/ViewDefinition/$viewdefinition-run")
            .add_header("accept", "application/json")
            .json(&run_request_body())
            .await;

        assert_eq!(response.status_code(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_invalid_gzip_request_body_is_client_error() {
        let server = TestServer::new(create_app()).unwrap();

        let response = server
            .post("/ViewDefinition/$viewdefinition-run")
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
            .post("/ViewDefinition/$viewdefinition-run")
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
            .post("/ViewDefinition/$viewdefinition-run")
            .add_header("accept-encoding", "gzip")
            .json(&body)
            .await;

        assert_eq!(response.status_code(), StatusCode::OK);
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "application/vnd.apache.parquet"
        );
        // Parquet is already compressed — the gzip layer must skip it.
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
            .post("/ViewDefinition/$viewdefinition-run")
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

        let response = server
            .post("/ViewDefinition/$viewdefinition-run")
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
            .post("/ViewDefinition/$viewdefinition-run")
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
            .post("/ViewDefinition/$viewdefinition-run")
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
            .post("/ViewDefinition/$viewdefinition-run")
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
