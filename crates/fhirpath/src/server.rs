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
//! - **Health Check**: Basic health check endpoint
//!
//! ## API Endpoints
//!
//! ```text
//! POST /
//!   Body: FHIR Parameters resource with expression and resource
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
    routing::{get, post},
};
use clap::Parser;
use http::{HeaderValue, Method};
use std::net::SocketAddr;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::{info, warn};

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
            cors_headers: "Accept,Accept-Language,Content-Type,Content-Language,Authorization".to_string(),
        }
    }
}

/// Command-line arguments for the server
#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "FHIRPath HTTP server",
    long_about = "HTTP server providing FHIRPath expression evaluation for fhirpath-lab integration\n\nEnvironment variables:\n  FHIRPATH_SERVER_PORT - Server port (default: 3000)\n  FHIRPATH_SERVER_HOST - Server host (default: 127.0.0.1)\n  FHIRPATH_LOG_LEVEL - Log level: error, warn, info, debug, trace (default: info)\n  FHIRPATH_ENABLE_CORS - Enable CORS: true/false (default: true)\n  FHIRPATH_CORS_ORIGINS - Allowed origins (comma-separated, * for any) (default: *)\n  FHIRPATH_CORS_METHODS - Allowed methods (comma-separated, * for any) (default: GET,POST,OPTIONS)\n  FHIRPATH_CORS_HEADERS - Allowed headers (comma-separated, * for any) (default: common headers)"
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
    #[arg(short = 'c', long, env = "FHIRPATH_ENABLE_CORS", default_value_t = true)]
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
        default_value = "Accept,Accept-Language,Content-Type,Content-Language,Authorization"
    )]
    pub cors_headers: String,
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
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| filter.into()),
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
fn create_app(config: &ServerConfig) -> Router {
    let mut app = Router::new()
        // Main evaluation endpoint
        .route("/", post(evaluate_fhirpath))
        // Health check endpoint
        .route("/health", get(health_check));

    // Add CORS if enabled
    if config.enable_cors {
        app = app.layer(build_cors_layer(config));
    }

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
    use axum::http::StatusCode;
    use axum_test::TestServer;

    #[tokio::test]
    async fn test_health_check() {
        let config = ServerConfig::default();
        let app = create_app(&config);
        let server = TestServer::new(app).unwrap();

        let response = server.get("/health").await;

        assert_eq!(response.status_code(), StatusCode::OK);

        let json: serde_json::Value = response.json();
        assert_eq!(json["status"], "ok");
        assert_eq!(json["service"], "fhirpath-server");
    }
}