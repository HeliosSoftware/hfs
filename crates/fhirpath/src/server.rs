//! # FHIRPath Server Implementation
//!
//! This module provides an HTTP server for evaluating FHIRPath expressions
//! following the fhirpath-lab server API specification. The server accepts
//! FHIR Parameters resources and returns evaluation results suitable for
//! integration with fhirpath-lab and other tools.
//!
//! ## Features
//!
//! - **HTTP API**: Single POST endpoint for FHIRPath evaluation
//! - **Parse Debug Tree**: Generate AST visualizations for expressions
//! - **Variable Support**: Pass variables to expressions
//! - **Context Expressions**: Evaluate expressions with context
//! - **CORS Support**: Configurable cross-origin resource sharing
//! - **HTTP Compression**: Transparent request decompression and response compression
//! - **Health Check**: Basic health check endpoint
//!
//! ## API Endpoints
//!
//! ```text
//! POST /
//!   Body: FHIR Parameters resource with expression and resource
//!   Returns: FHIR Parameters resource with evaluation results
//!   Note: Auto-detects FHIR version from resource
//!
//! POST /r4 (if compiled with R4 feature)
//!   Body: FHIR Parameters resource with expression and R4 resource
//!   Returns: FHIR Parameters resource with evaluation results
//!
//! POST /r4b (if compiled with R4B feature)
//!   Body: FHIR Parameters resource with expression and R4B resource
//!   Returns: FHIR Parameters resource with evaluation results
//!
//! POST /r5 (if compiled with R5 feature)
//!   Body: FHIR Parameters resource with expression and R5 resource
//!   Returns: FHIR Parameters resource with evaluation results
//!
//! POST /r6 (if compiled with R6 feature)
//!   Body: FHIR Parameters resource with expression and R6 resource
//!   Returns: FHIR Parameters resource with evaluation results
//!
//! GET /health
//!   Returns: Health check status
//! ```
//!
//! ## Configuration
//!
//! The server supports configuration through both command-line arguments and
//! environment variables:
//!
//! - `FHIRPATH_SERVER_PORT` / `--port`: Server port (default: 3000)
//! - `FHIRPATH_SERVER_HOST` / `--host`: Server host (default: 127.0.0.1)
//! - `FHIRPATH_LOG_LEVEL` / `--log-level`: Log level (default: info)
//! - `FHIRPATH_ENABLE_CORS` / `--enable-cors`: Enable CORS (default: true)
//! - `FHIRPATH_CORS_ORIGINS` / `--cors-origins`: Allowed origins (default: *)
//! - `FHIRPATH_MAX_BODY_SIZE` / `--max-body-size`: Max request body size in
//!   bytes, measured after decompression (default: 10485760)
//!
//! ## HTTP Compression
//!
//! Request bodies sent with `Content-Encoding: gzip` (or `deflate`, `br`,
//! `zstd`) are decompressed transparently before parsing; unsupported
//! encodings are rejected with `415 Unsupported Media Type`. Responses are
//! compressed when the client advertises support via `Accept-Encoding`, with
//! `Content-Encoding` and `Vary: Accept-Encoding` set accordingly. Because the
//! body-size limit is enforced on the decompressed body, a small
//! highly-compressed payload cannot bypass `FHIRPATH_MAX_BODY_SIZE`.
//!
//! ## Usage Example
//!
//! ```bash
//! # Start server with defaults
//! fhirpath-server
//!
//! # Custom configuration
//! fhirpath-server --port 8080 --host 0.0.0.0
//!
//! # Test the server
//! curl -X POST http://localhost:3000 \
//!   -H "Content-Type: application/json" \
//!   -d @request.json
//! ```

use axum::{
    Router,
    extract::DefaultBodyLimit,
    routing::{get, post},
};
use clap::Parser;
use http::{HeaderValue, Method};
use std::net::SocketAddr;
use tower_http::compression::CompressionLayer;
use tower_http::cors::CorsLayer;
use tower_http::decompression::RequestDecompressionLayer;
use tower_http::trace::TraceLayer;
use tracing::{info, warn};

/// Default maximum request body size in bytes (10 MiB), measured after
/// decompression. Mirrors `HFS_MAX_BODY_SIZE` / `SOF_MAX_BODY_SIZE` /
/// `HTS_MAX_BODY_SIZE`.
const DEFAULT_MAX_BODY_SIZE: usize = 10 * 1024 * 1024;

use crate::handlers::{evaluate_fhirpath, health_check};

/// Server configuration
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Port to bind the server to
    pub port: u16,
    /// Host address to bind to
    pub host: String,
    /// Log level for the server
    pub log_level: String,
    /// Whether to enable CORS
    pub enable_cors: bool,
    /// Allowed CORS origins (comma-separated list, "*" for any)
    pub cors_origins: String,
    /// Allowed CORS methods (comma-separated list, "*" for any)
    pub cors_methods: String,
    /// Allowed CORS headers (comma-separated list, "*" for any)
    pub cors_headers: String,
    /// Maximum request body size in bytes, measured after decompression
    pub max_body_size: usize,
    /// Terminology server URL (defaults based on FHIR version)
    pub terminology_server: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: 3000,
            host: "127.0.0.1".to_string(),
            log_level: "info".to_string(),
            enable_cors: true,
            cors_origins: "*".to_string(),
            cors_methods: "GET,POST,OPTIONS".to_string(),
            cors_headers:
                "Accept,Accept-Language,Content-Type,Content-Language,Authorization,Content-Encoding"
                    .to_string(),
            max_body_size: DEFAULT_MAX_BODY_SIZE,
            terminology_server: None,
        }
    }
}

/// Command-line arguments for the server
#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "FHIRPath HTTP server",
    long_about = "HTTP server providing FHIRPath expression evaluation for fhirpath-lab integration\n\nEnvironment variables:\n  FHIRPATH_SERVER_PORT - Server port (default: 3000)\n  FHIRPATH_SERVER_HOST - Server host (default: 127.0.0.1)\n  FHIRPATH_LOG_LEVEL - Log level: error, warn, info, debug, trace (default: info)\n  FHIRPATH_ENABLE_CORS - Enable CORS: true/false (default: true)\n  FHIRPATH_CORS_ORIGINS - Allowed origins (comma-separated, * for any) (default: *)\n  FHIRPATH_CORS_METHODS - Allowed methods (comma-separated, * for any) (default: GET,POST,OPTIONS)\n  FHIRPATH_CORS_HEADERS - Allowed headers (comma-separated, * for any) (default: common headers)\n  FHIRPATH_MAX_BODY_SIZE - Max request body size in bytes, measured after decompression (default: 10485760)\n  FHIRPATH_TERMINOLOGY_SERVER - Terminology server URL (default: version-specific test servers)"
)]
pub struct ServerArgs {
    /// Port to bind the server to
    #[arg(short, long, env = "FHIRPATH_SERVER_PORT", default_value_t = 3000)]
    pub port: u16,

    /// Host address to bind to
    #[arg(
        short = 'H',
        long,
        env = "FHIRPATH_SERVER_HOST",
        default_value = "127.0.0.1"
    )]
    pub host: String,

    /// Log level (error, warn, info, debug, trace)
    #[arg(short, long, env = "FHIRPATH_LOG_LEVEL", default_value = "info")]
    pub log_level: String,

    /// Enable CORS
    #[arg(
        short = 'c',
        long,
        env = "FHIRPATH_ENABLE_CORS",
        default_value_t = true
    )]
    pub enable_cors: bool,

    /// Allowed CORS origins (comma-separated list, "*" for any)
    #[arg(long, env = "FHIRPATH_CORS_ORIGINS", default_value = "*")]
    pub cors_origins: String,

    /// Allowed CORS methods (comma-separated list, "*" for any)
    #[arg(
        long,
        env = "FHIRPATH_CORS_METHODS",
        default_value = "GET,POST,OPTIONS"
    )]
    pub cors_methods: String,

    /// Allowed CORS headers (comma-separated list, "*" for any)
    #[arg(
        long,
        env = "FHIRPATH_CORS_HEADERS",
        default_value = "Accept,Accept-Language,Content-Type,Content-Language,Authorization,Content-Encoding"
    )]
    pub cors_headers: String,

    /// Maximum request body size in bytes, measured after decompression
    #[arg(
        long,
        env = "FHIRPATH_MAX_BODY_SIZE",
        default_value_t = DEFAULT_MAX_BODY_SIZE
    )]
    pub max_body_size: usize,

    /// Terminology server URL (defaults based on FHIR version)
    #[arg(long, env = "FHIRPATH_TERMINOLOGY_SERVER")]
    pub terminology_server: Option<String>,
}

impl From<ServerArgs> for ServerConfig {
    fn from(args: ServerArgs) -> Self {
        ServerConfig {
            port: args.port,
            host: args.host,
            log_level: args.log_level,
            enable_cors: args.enable_cors,
            cors_origins: args.cors_origins,
            cors_methods: args.cors_methods,
            cors_headers: args.cors_headers,
            max_body_size: args.max_body_size,
            terminology_server: args.terminology_server,
        }
    }
}

/// Run the FHIRPath server
pub async fn run_server(config: ServerConfig) -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    let filter = format!(
        "fhirpath_server={},tower_http={}",
        config.log_level, config.log_level
    );
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| filter.into()),
        )
        .init();

    info!("Starting FHIRPath server...");
    info!("Configuration: {:?}", config);

    // Build the application
    let app = create_app(&config);

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

/// Create the axum application with all routes
pub fn create_app(config: &ServerConfig) -> Router {
    let mut app = Router::new()
        // Main evaluation endpoint (auto-detects version)
        .route("/", post(evaluate_fhirpath))
        // Health check endpoint
        .route("/health", get(health_check));

    // Add version-specific endpoints based on enabled features
    #[cfg(feature = "R4")]
    {
        app = app.route("/r4", post(crate::handlers::evaluate_fhirpath_r4));
    }

    #[cfg(feature = "R4B")]
    {
        app = app.route("/r4b", post(crate::handlers::evaluate_fhirpath_r4b));
    }

    #[cfg(feature = "R5")]
    {
        app = app.route("/r5", post(crate::handlers::evaluate_fhirpath_r5));
    }

    #[cfg(feature = "R6")]
    {
        app = app.route("/r6", post(crate::handlers::evaluate_fhirpath_r6));
    }

    // Transparently decompress request bodies sent with `Content-Encoding`
    // (gzip, deflate, br, zstd) and compress responses when the client sends
    // `Accept-Encoding`. Unsupported request encodings get 415. Because the
    // decompression layer replaces the request body before any extractor or
    // handler reads it, the body-size limit below applies to the
    // *decompressed* bytes — a small highly-compressed payload cannot bypass
    // `FHIRPATH_MAX_BODY_SIZE`. Kept inside the CORS layer so 415/413 error
    // responses still carry CORS headers for browser clients.
    app = app
        .layer(RequestDecompressionLayer::new())
        .layer(CompressionLayer::new());

    // Add CORS if enabled
    if config.enable_cors {
        app = app.layer(build_cors_layer(config));
    }

    // Raise the body-size limit from axum's 2 MiB default to the configured
    // ceiling so `FHIRPATH_MAX_BODY_SIZE` / `--max-body-size` actually takes
    // effect. Without this, axum's `DefaultBodyLimit` extractor rejects any
    // request > 2 MiB with 413 before the handler runs. Mirrors the pattern
    // used in `crates/rest/src/lib.rs` and `crates/sof/src/server.rs`.
    app = app.layer(DefaultBodyLimit::max(config.max_body_size));

    // Add tracing
    app = app.layer(TraceLayer::new_for_http());

    app
}

/// Build CORS layer from configuration
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
        info!("CORS: Using wildcards, credentials are disabled for security");
    }

    cors
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use serde_json::json;
    use tower::ServiceExt; // for oneshot

    #[tokio::test]
    async fn test_health_check() {
        let config = ServerConfig::default();
        let app = create_app(&config);

        // Create a request to the health endpoint
        let request = Request::builder()
            .uri("/health")
            .body(Body::empty())
            .unwrap();

        // Send the request and get the response
        let response = app.oneshot(request).await.unwrap();

        // Check the status code
        assert_eq!(response.status(), StatusCode::OK);

        // Check the response body
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["status"], "ok");
        assert_eq!(json["service"], "fhirpath-server");
        assert_eq!(json["version"], env!("CARGO_PKG_VERSION"));
    }

    #[tokio::test]
    async fn test_create_app_with_cors() {
        let mut config = ServerConfig::default();
        config.enable_cors = true;
        config.cors_origins = "http://localhost:3000".to_string();

        let app = create_app(&config);

        // Test that the app can be created with CORS configuration
        let request = Request::builder()
            .method("OPTIONS")
            .uri("/")
            .header("Origin", "http://localhost:3000")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        // CORS preflight should return OK
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[test]
    fn test_server_config_default() {
        let config = ServerConfig::default();
        assert_eq!(config.port, 3000);
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.log_level, "info");
        assert!(config.enable_cors);
        assert_eq!(config.cors_origins, "*");
        assert_eq!(config.cors_methods, "GET,POST,OPTIONS");
        assert_eq!(
            config.cors_headers,
            "Accept,Accept-Language,Content-Type,Content-Language,Authorization,Content-Encoding"
        );
        assert_eq!(config.max_body_size, 10 * 1024 * 1024);
    }

    #[test]
    fn test_server_args_to_config() {
        let args = ServerArgs {
            port: 8080,
            host: "0.0.0.0".to_string(),
            log_level: "debug".to_string(),
            enable_cors: false,
            cors_origins: "http://example.com".to_string(),
            cors_methods: "GET,POST".to_string(),
            cors_headers: "Content-Type".to_string(),
            max_body_size: 2048,
            terminology_server: None,
        };

        let config: ServerConfig = args.into();
        assert_eq!(config.port, 8080);
        assert_eq!(config.host, "0.0.0.0");
        assert_eq!(config.log_level, "debug");
        assert!(!config.enable_cors);
        assert_eq!(config.cors_origins, "http://example.com");
        assert_eq!(config.cors_methods, "GET,POST");
        assert_eq!(config.cors_headers, "Content-Type");
        assert_eq!(config.max_body_size, 2048);
    }

    #[tokio::test]
    async fn test_cors_wildcard_configuration() {
        let mut config = ServerConfig::default();
        config.enable_cors = true;
        config.cors_origins = "*".to_string();
        config.cors_methods = "*".to_string();
        config.cors_headers = "*".to_string();

        let app = create_app(&config);

        // Test with any origin
        let request = Request::builder()
            .method("OPTIONS")
            .uri("/")
            .header("Origin", "http://any-origin.com")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_cors_specific_origins() {
        let mut config = ServerConfig::default();
        config.enable_cors = true;
        config.cors_origins = "http://localhost:3000,http://localhost:4000".to_string();

        let app = create_app(&config);

        // Test with allowed origin
        let request = Request::builder()
            .method("OPTIONS")
            .uri("/")
            .header("Origin", "http://localhost:3000")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_cors_disabled() {
        let mut config = ServerConfig::default();
        config.enable_cors = false;

        let app = create_app(&config);

        // Without CORS, OPTIONS request should still work but without CORS headers
        let request = Request::builder()
            .method("OPTIONS")
            .uri("/")
            .header("Origin", "http://localhost:3000")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        // The response status depends on route handling, but it should not crash
        assert!(
            response.status() == StatusCode::OK
                || response.status() == StatusCode::METHOD_NOT_ALLOWED
        );
    }

    #[tokio::test]
    async fn test_post_endpoint_exists() {
        let config = ServerConfig::default();
        let app = create_app(&config);

        // Create a minimal Parameters resource for testing
        let parameters = json!({
            "resourceType": "Parameters",
            "parameter": []
        });

        let request = Request::builder()
            .method("POST")
            .uri("/")
            .header("Content-Type", "application/json")
            .body(Body::from(parameters.to_string()))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        // We expect it to process the request (might fail due to missing parameters, but endpoint exists)
        assert!(
            response.status() == StatusCode::OK || response.status() == StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn test_build_cors_layer_with_empty_values() {
        let mut config = ServerConfig::default();
        config.cors_origins = "".to_string();
        config.cors_methods = "".to_string();
        config.cors_headers = "".to_string();

        // Should not panic
        let _cors_layer = build_cors_layer(&config);
    }

    #[test]
    fn test_build_cors_layer_with_mixed_values() {
        let mut config = ServerConfig::default();
        config.cors_origins = "http://localhost:3000,http://localhost:4000".to_string();
        config.cors_methods = "GET,POST,PUT".to_string();
        config.cors_headers = "*".to_string();

        // Should not panic and handle mixed wildcard/specific values
        let _cors_layer = build_cors_layer(&config);
    }

    #[test]
    fn test_build_cors_layer_credentials_logic() {
        // Test that credentials are disabled when using wildcards
        let mut config = ServerConfig::default();
        config.cors_origins = "*".to_string();
        let _cors_layer = build_cors_layer(&config);
        // No direct way to test this without inspecting the layer internals
        // but the code should execute without panic

        // Test that credentials can be enabled with specific origins
        config.cors_origins = "http://localhost:3000".to_string();
        config.cors_methods = "GET,POST".to_string();
        config.cors_headers = "Content-Type".to_string();
        let _cors_layer = build_cors_layer(&config);
    }

    #[tokio::test]
    async fn test_invalid_host_address_handling() {
        // The run_server function handles invalid host addresses
        // We can't easily test run_server directly, but we can test
        // that the server creation doesn't panic with various configs
        let mut config = ServerConfig::default();
        config.host = "invalid-host-@#$".to_string();

        // This would normally be handled in run_server with a fallback to 127.0.0.1
        // Just verify the app can be created
        let _app = create_app(&config);
    }

    #[cfg(feature = "R4")]
    #[tokio::test]
    async fn test_r4_endpoint_exists() {
        let config = ServerConfig::default();
        let app = create_app(&config);

        // Create a minimal Parameters resource for testing
        let parameters = json!({
            "resourceType": "Parameters",
            "parameter": []
        });

        let request = Request::builder()
            .method("POST")
            .uri("/r4")
            .header("Content-Type", "application/json")
            .body(Body::from(parameters.to_string()))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        // We expect it to process the request (might fail due to missing parameters, but endpoint exists)
        assert!(
            response.status() == StatusCode::OK || response.status() == StatusCode::BAD_REQUEST
        );
    }

    #[cfg(feature = "R4B")]
    #[tokio::test]
    async fn test_r4b_endpoint_exists() {
        let config = ServerConfig::default();
        let app = create_app(&config);

        let parameters = json!({
            "resourceType": "Parameters",
            "parameter": []
        });

        let request = Request::builder()
            .method("POST")
            .uri("/r4b")
            .header("Content-Type", "application/json")
            .body(Body::from(parameters.to_string()))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert!(
            response.status() == StatusCode::OK || response.status() == StatusCode::BAD_REQUEST
        );
    }

    #[cfg(feature = "R5")]
    #[tokio::test]
    async fn test_r5_endpoint_exists() {
        let config = ServerConfig::default();
        let app = create_app(&config);

        let parameters = json!({
            "resourceType": "Parameters",
            "parameter": []
        });

        let request = Request::builder()
            .method("POST")
            .uri("/r5")
            .header("Content-Type", "application/json")
            .body(Body::from(parameters.to_string()))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert!(
            response.status() == StatusCode::OK || response.status() == StatusCode::BAD_REQUEST
        );
    }

    #[cfg(feature = "R6")]
    #[tokio::test]
    async fn test_r6_endpoint_exists() {
        let config = ServerConfig::default();
        let app = create_app(&config);

        let parameters = json!({
            "resourceType": "Parameters",
            "parameter": []
        });

        let request = Request::builder()
            .method("POST")
            .uri("/r6")
            .header("Content-Type", "application/json")
            .body(Body::from(parameters.to_string()))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert!(
            response.status() == StatusCode::OK || response.status() == StatusCode::BAD_REQUEST
        );
    }

    #[tokio::test]
    async fn test_version_endpoints_conditional_compilation() {
        // Test that endpoints are only available when features are enabled
        let config = ServerConfig::default();
        let _app = create_app(&config);

        // Test R4 endpoint
        #[cfg(not(feature = "R4"))]
        {
            let request = Request::builder()
                .method("POST")
                .uri("/r4")
                .body(Body::empty())
                .unwrap();
            let response = _app.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }

        // Test R4B endpoint
        #[cfg(not(feature = "R4B"))]
        {
            let request = Request::builder()
                .method("POST")
                .uri("/r4b")
                .body(Body::empty())
                .unwrap();
            let response = _app.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }

        // Test R5 endpoint
        #[cfg(not(feature = "R5"))]
        {
            let request = Request::builder()
                .method("POST")
                .uri("/r5")
                .body(Body::empty())
                .unwrap();
            let response = _app.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }

        // Test R6 endpoint
        #[cfg(not(feature = "R6"))]
        {
            let request = Request::builder()
                .method("POST")
                .uri("/r6")
                .body(Body::empty())
                .unwrap();
            let response = _app.oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }
    }
}
