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
//!
//! ## API Endpoints
//!
//! ```text
//! GET /metadata
//!   Returns: CapabilityStatement
//!
//! POST /ViewDefinition/$run
//!   Body: Parameters resource containing ViewDefinition and data
//!   Query Parameters (in specification order):
//!     _format: Output format - application/json, application/ndjson, text/csv, application/parquet
//!     header: CSV header control - true (default), false
//!     source: Data source (type: string) - Not yet supported
//!     _count: Limits the number of results (1-10000)
//!     _page: Page number for paginated results (1-based)
//!     _since: Return resources modified after this time (RFC3339)
//!   Body Parameters (in FHIR Parameters resource):
//!     _format: Output format (type: code)
//!     header: CSV header control (type: boolean)
//!     viewReference: Reference(s) to ViewDefinition(s) (type: Reference) - Not yet supported
//!     viewResource: ViewDefinition(s) to use (type: ViewDefinition)
//!     patient: Filter by patient (type: Reference)
//!     group: Filter by group (type: Reference)
//!     source: Data source (type: string) - Not yet supported
//!     _count: Result limit (type: integer)
//!     _page: Page number (type: integer)
//!     _since: Modification time filter (type: instant)
//!     resource: FHIR resources to transform (type: Resource)
//!   Returns: Transformed data in requested format
//!
//! GET /ViewDefinition/$run
//!   IMPORTANT: Per FHIR specification, GET operations cannot use complex parameters.
//!   This endpoint is limited and typically requires POST for practical use.
//!   
//!   Supported Query Parameters (simple types only):
//!     _format: Output format - application/json, application/ndjson, text/csv, application/parquet
//!     header: CSV header control - true (default), false
//!     source: Data source (type: string) - Not yet supported
//!     _count: Limits the number of results (1-10000)
//!     _page: Page number for paginated results (1-based)
//!     _since: Return resources modified after this time (RFC3339)
//!   
//!   Unsupported Parameters in GET (complex types - use POST instead):
//!     viewReference: Cannot pass Reference types in GET
//!     viewResource: Cannot pass a FHIR resource in GET
//!     patient: Cannot pass Reference types in GET
//!     group: Cannot pass Reference types in GET
//!   
//!   Returns: Error message explaining GET limitations
//! ```
//!
//! ## Configuration
//!
//! The server supports configuration through both command-line arguments and environment variables:
//!
//! - `SOF_SERVER_PORT` / `--port`: Server port (default: 8080)
//! - `SOF_SERVER_HOST` / `--host`: Server host (default: 127.0.0.1)
//! - `SOF_LOG_LEVEL` / `--log-level`: Log level (default: info)
//! - `SOF_MAX_BODY_SIZE` / `--max-body-size`: Max request size in bytes (default: 10MB)
//! - `SOF_REQUEST_TIMEOUT` / `--request-timeout`: Request timeout in seconds (default: 30)
//! - `SOF_ENABLE_CORS` / `--enable-cors`: Enable CORS (default: true)
//! - `SOF_CORS_ORIGINS` / `--cors-origins`: Allowed origins, comma-separated (default: *)
//! - `SOF_CORS_METHODS` / `--cors-methods`: Allowed methods, comma-separated (default: *)
//! - `SOF_CORS_HEADERS` / `--cors-headers`: Allowed headers, comma-separated (default: *)
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
            cors_headers: "Accept,Accept-Language,Content-Type,Content-Language,Authorization,X-Requested-With".to_string(),
        }
    }
}

/// Main server entry point
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Parse command line arguments first to get log level
    let config = parse_args();

    // Initialize tracing subscriber for logging with configured level
    let filter = format!(
        "sof_server={},tower_http={}",
        config.log_level, config.log_level
    );
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| filter.into()),
        )
        .init();

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
            default_value = "Accept,Accept-Language,Content-Type,Content-Language,Authorization,X-Requested-With"
        )]
        cors_headers: String,
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
    }
}

/// Create the axum application with all routes and configuration
fn create_app_with_config(config: &ServerConfig) -> Router {
    use axum::extract::DefaultBodyLimit;
    use std::time::Duration;
    use tower::ServiceBuilder;
    use tower_http::timeout::TimeoutLayer;

    let mut app = Router::new()
        // FHIR endpoints
        .route("/metadata", get(handlers::capability_statement))
        .route(
            "/ViewDefinition/$run",
            post(handlers::run_view_definition_handler)
                .get(handlers::run_view_definition_get_handler),
        )
        // Health check endpoint
        .route("/health", get(handlers::health_check))
        // Add body size limit
        .layer(DefaultBodyLimit::max(config.max_body_size))
        // Add request timeout
        .layer(
            ServiceBuilder::new()
                .layer(TimeoutLayer::new(Duration::from_secs(
                    config.request_timeout,
                )))
                .into_inner(),
        );

    // Add CORS if enabled
    if config.enable_cors {
        app = app.layer(build_cors_layer(config));
    }

    // Add tracing
    app = app.layer(TraceLayer::new_for_http());

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

    #[tokio::test]
    async fn test_get_with_source_parameter_not_implemented() {
        let config = ServerConfig::default();
        let app = create_app_with_config(&config);
        let server = TestServer::new(app).unwrap();

        let response = server
            .get("/ViewDefinition/$run")
            .add_query_param("source", "my-data-source")
            .add_query_param("_format", "application/json")
            .await;

        assert_eq!(response.status_code(), StatusCode::NOT_IMPLEMENTED);

        let json: serde_json::Value = response.json();
        assert_eq!(json["resourceType"], "OperationOutcome");
        assert!(
            json["issue"][0]["details"]["text"]
                .as_str()
                .unwrap()
                .contains("The source parameter is not supported in this stateless implementation")
        );
    }

    #[tokio::test]
    async fn test_get_rejects_all_complex_parameters() {
        let config = ServerConfig::default();
        let app = create_app_with_config(&config);

        // Test viewReference
        let server = TestServer::new(app.clone()).unwrap();
        let response = server
            .get("/ViewDefinition/$run")
            .add_query_param("viewReference", "ViewDefinition/123")
            .await;
        assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
        let json: serde_json::Value = response.json();
        assert!(
            json["issue"][0]["details"]["text"]
                .as_str()
                .unwrap()
                .contains("viewReference")
        );

        // Test patient
        let server = TestServer::new(app.clone()).unwrap();
        let response = server
            .get("/ViewDefinition/$run")
            .add_query_param("patient", "Patient/456")
            .await;
        assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
        let json: serde_json::Value = response.json();
        assert!(
            json["issue"][0]["details"]["text"]
                .as_str()
                .unwrap()
                .contains("patient")
        );

        // Test group
        let server = TestServer::new(app.clone()).unwrap();
        let response = server
            .get("/ViewDefinition/$run")
            .add_query_param("group", "Group/789")
            .await;
        assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
        let json: serde_json::Value = response.json();
        assert!(
            json["issue"][0]["details"]["text"]
                .as_str()
                .unwrap()
                .contains("group")
        );

        // Test source - now returns NOT_IMPLEMENTED instead of BAD_REQUEST
        let server = TestServer::new(app.clone()).unwrap();
        let response = server
            .get("/ViewDefinition/$run")
            .add_query_param("source", "my-source")
            .add_query_param("_format", "application/json")
            .await;
        assert_eq!(response.status_code(), StatusCode::NOT_IMPLEMENTED);
        let json: serde_json::Value = response.json();
        assert!(
            json["issue"][0]["details"]["text"]
                .as_str()
                .unwrap()
                .contains("source parameter is not supported")
        );
    }
}
