//! # SQL-on-FHIR Server Implementation
//!
//! This module provides a stateless HTTP server implementation for the [SQL-on-FHIR
//! specification](http://hl7.org/fhir/uv/sql-on-fhir),
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
//! GET /OperationDefinition/sof-sql-run
//!   Returns: this server's $sql-run OperationDefinition, declaring the
//!            subset of parameters it supports (base = the guide's definition)
//!
//! POST /$sql-run
//!   Body: Parameters resource containing the subject and data
//!   Query Parameters (except the subject trio, patient, group, resource):
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
//!     subjectCanonical: Canonical URL of the subject - not supported (stateless)
//!     subjectReference: Literal location of the subject - not supported (stateless)
//!     subjectResource: The ViewDefinition to execute, supplied inline
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

use helios_sof::app::{ServerConfig, create_app_with_config};
use std::net::SocketAddr;
use tracing::{info, warn};

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
