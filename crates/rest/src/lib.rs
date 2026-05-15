//! # helios-rest - FHIR RESTful API Implementation
//!
//! This crate provides a complete implementation of the [FHIR RESTful API](https://hl7.org/fhir/http.html)
//! specification for the Helios FHIR Server. It implements all standard FHIR interactions
//! including CRUD operations, search, versioning, conditional operations, and batch/transaction
//! processing.
//!
//! ## Features
//!
//! - **Full CRUD Support**: Create, Read, Update, Delete operations for all FHIR resource types
//! - **Versioning**: Full version history with vread and history interactions
//! - **Conditional Operations**: Conditional create, update, delete, and patch
//! - **Search**: Type-level and system-level search with modifiers and chaining
//! - **Batch/Transaction**: Bundle processing with atomic transaction support
//! - **Content Negotiation**: JSON and XML format support with proper MIME types
//! - **Multi-Tenant**: Built-in tenant isolation for multi-tenant deployments
//!
//! ## FHIR Version Support
//!
//! This crate supports multiple FHIR versions through feature flags:
//!
//! - `R4` - FHIR R4 (4.0.1) - Default
//! - `R4B` - FHIR R4B (4.3.0)
//! - `R5` - FHIR R5 (5.0.0)
//! - `R6` - FHIR R6 (6.0.0-ballot)
//!
//! ## Backend Support
//!
//! Storage backends are configured through feature flags:
//!
//! - `sqlite` - SQLite backend (default, great for development)
//! - `postgres` - PostgreSQL backend (recommended for production)
//! - `mongodb` - MongoDB backend
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use helios_rest::{create_app, ServerConfig};
//! use helios_persistence::backends::sqlite::SqliteBackend;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     // Create a storage backend
//!     let backend = SqliteBackend::new("fhir.db")?;
//!     backend.init_schema()?;
//!
//!     // Configure the server
//!     let config = ServerConfig::default();
//!
//!     // Create the Axum application
//!     let app = create_app(backend, config);
//!
//!     // Start the server
//!     let listener = tokio::net::TcpListener::bind("127.0.0.1:8080").await?;
//!     axum::serve(listener, app).await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! ## API Endpoints
//!
//! The server exposes the following endpoints:
//!
//! | Interaction | HTTP Method | URL Pattern |
//! |------------|-------------|-------------|
//! | read | GET | `/[type]/[id]` |
//! | vread | GET | `/[type]/[id]/_history/[vid]` |
//! | update | PUT | `/[type]/[id]` |
//! | patch | PATCH | `/[type]/[id]` |
//! | delete | DELETE | `/[type]/[id]` |
//! | create | POST | `/[type]` |
//! | search | GET/POST | `/[type]?params` or `/[type]/_search` |
//! | capabilities | GET | `/metadata` |
//! | history (instance) | GET | `/[type]/[id]/_history` |
//! | history (type) | GET | `/[type]/_history` |
//! | history (system) | GET | `/_history` |
//! | batch/transaction | POST | `/` |
//!
//! ## HTTP Headers
//!
//! The server supports standard FHIR HTTP headers:
//!
//! - `Accept` - Content negotiation (application/fhir+json, application/fhir+xml)
//! - `Content-Type` - Request body format
//! - `ETag` / `If-Match` - Optimistic locking for updates
//! - `If-None-Match` - Conditional read
//! - `If-None-Exist` - Conditional create
//! - `If-Modified-Since` - Conditional read by date
//! - `Prefer` - Response preference (return=minimal, return=representation, return=OperationOutcome)
//! - `X-Tenant-ID` - Multi-tenant identification
//!
//! ## Error Handling
//!
//! All errors are returned as FHIR [OperationOutcome](https://hl7.org/fhir/operationoutcome.html)
//! resources with appropriate HTTP status codes:
//!
//! | HTTP Status | FHIR Issue Code | Description |
//! |-------------|-----------------|-------------|
//! | 400 | invalid | Bad request / validation error |
//! | 404 | not-found | Resource not found |
//! | 409 | conflict | Version conflict |
//! | 410 | deleted | Resource was deleted |
//! | 412 | conflict | Precondition failed |
//! | 415 | not-supported | Unsupported media type |
//! | 422 | processing | Unprocessable entity |
//! | 500 | exception | Internal server error |
//!
//! ## Configuration
//!
//! The server is configured via environment variables:
//!
//! | Variable | Default | Description |
//! |----------|---------|-------------|
//! | `HFS_SERVER_PORT` | 8080 | Server port |
//! | `HFS_SERVER_HOST` | 127.0.0.1 | Host to bind |
//! | `HFS_LOG_LEVEL` | info | Log level (error, warn, info, debug, trace) |
//! | `HFS_MAX_BODY_SIZE` | 10485760 | Max request body size (bytes) |
//! | `HFS_REQUEST_TIMEOUT` | 30 | Request timeout (seconds) |
//! | `HFS_ENABLE_CORS` | true | Enable CORS |
//! | `HFS_CORS_ORIGINS` | * | Allowed CORS origins |
//! | `HFS_DEFAULT_TENANT` | default | Default tenant ID |
//!
//! ## Architecture
//!
//! The crate is organized into several modules:
//!
//! - [`error`] - Error types and OperationOutcome generation
//! - [`config`] - Server configuration
//! - [`state`] - Application state (storage, configuration)
//! - [`handlers`] - HTTP request handlers for each interaction
//! - [`middleware`] - Axum middleware (tenant, content negotiation, conditional headers)
//! - [`extractors`] - Axum extractors for FHIR-specific data
//! - [`responses`] - Response formatting and header generation
//! - [`routing`] - Route configuration

// Enforce documentation
#![warn(missing_docs)]
#![warn(rustdoc::missing_crate_level_docs)]

pub mod config;
pub mod error;
pub mod export;
pub mod extractors;
pub mod fhir_types;
pub mod handlers;
pub mod middleware;
pub mod responses;
pub mod routing;
pub mod state;
pub mod tenant;

// Re-export commonly used types
pub use config::{MultitenancyConfig, ServerConfig, StorageBackendMode, TenantRoutingMode};
pub use error::{RestError, RestResult};
pub use middleware::auth::AuthMiddlewareState;
pub use state::AppState;
pub use tenant::{ResolvedTenant, TenantResolver, TenantSource};

use std::sync::Arc;

use axum::Router;
use helios_persistence::core::{
    BundleProvider, ConditionalStorage, InstanceHistoryProvider, ResourceStorage, SearchProvider,
};
use tower::ServiceBuilder;
use tower_http::{
    cors::{Any, CorsLayer},
    timeout::TimeoutLayer,
    trace::TraceLayer,
};
use tracing::info;

/// Creates the Axum application with default configuration.
///
/// This is a convenience function that creates the app with default settings.
/// For more control, use [`create_app_with_config`].
///
/// # Arguments
///
/// * `storage` - The storage backend to use
///
/// # Example
///
/// ```rust,ignore
/// use helios_rest::create_app;
/// use helios_persistence::backends::sqlite::SqliteBackend;
///
/// let backend = SqliteBackend::in_memory()?;
/// let app = create_app(backend);
/// ```
pub fn create_app<S>(storage: S) -> Router
where
    S: ResourceStorage
        + ConditionalStorage
        + SearchProvider
        + InstanceHistoryProvider
        + BundleProvider
        + Send
        + Sync
        + 'static,
{
    create_app_with_config(storage, ServerConfig::default())
}

/// Creates the Axum application with custom configuration.
///
/// This function sets up the complete FHIR REST API with all handlers,
/// middleware, and configuration.
///
/// # Arguments
///
/// * `storage` - The storage backend to use
/// * `config` - Server configuration
///
/// # Example
///
/// ```rust,ignore
/// use helios_rest::{create_app_with_config, ServerConfig};
/// use helios_persistence::backends::sqlite::SqliteBackend;
///
/// let backend = SqliteBackend::in_memory()?;
/// let config = ServerConfig {
///     port: 3000,
///     enable_cors: true,
///     ..Default::default()
/// };
/// let app = create_app_with_config(backend, config);
/// ```
pub fn create_app_with_config<S>(storage: S, config: ServerConfig) -> Router
where
    S: ResourceStorage
        + ConditionalStorage
        + SearchProvider
        + InstanceHistoryProvider
        + BundleProvider
        + Send
        + Sync
        + 'static,
{
    create_app_with_auth(storage, config, helios_auth::AuthConfig::default(), None)
}

/// Creates the Axum application with custom configuration and optional authentication.
///
/// When `auth_state` is `Some`, authentication and authorization middleware
/// are added to the middleware stack.
pub fn create_app_with_auth<S>(
    storage: S,
    config: ServerConfig,
    auth_config: helios_auth::AuthConfig,
    auth_state: Option<Arc<middleware::auth::AuthMiddlewareState>>,
) -> Router
where
    S: ResourceStorage
        + ConditionalStorage
        + SearchProvider
        + InstanceHistoryProvider
        + BundleProvider
        + Send
        + Sync
        + 'static,
{
    info!(
        "Creating REST API server with backend: {}",
        storage.backend_name()
    );
    if auth_state.is_some() {
        info!("Authentication is ENABLED");
    }

    // Wrap storage in Arc so we can share it with the SofRunner
    let storage_arc = Arc::new(storage);

    // Create application state
    let mut state = AppState::with_auth(
        Arc::clone(&storage_arc),
        config.clone(),
        auth_config,
        auth_state.clone(),
    );

    // Wire SQL-on-FHIR runner and export controller. The SOF runtime path is
    // in-DB SQL only — backends without a SOF runner can't serve
    // `$viewdefinition-run` and the handler returns 501 if SOF is enabled
    // without one.
    if config.sof_enabled {
        let Some(runner) = storage_arc.sof_runner() else {
            // Hard config error — surfaced as a startup panic so misconfiguration
            // doesn't silently disable a feature the operator asked for.
            panic!(
                "HFS_SOF_ENABLED=true but storage backend '{}' does not provide an in-DB SOF \
                 runner; either disable SOF or use a backend that supports it (sqlite, postgres)",
                storage_arc.backend_name()
            );
        };
        info!(
            runner = runner.runner_name(),
            fhir_version = ?config.default_fhir_version,
            "Using in-DB SofRunner"
        );

        // Keep a clone for the export controller before moving runner into state.
        let runner_for_export = Arc::clone(&runner);
        state = state.with_sof_runner(runner);

        // Wire the export job controller.
        use crate::export::{ExportJobController, FilesystemSink, InMemoryController};
        let controller: Arc<dyn ExportJobController> = {
            let max_concurrency = Some(config.export_max_concurrency);
            let shard_rows = Some(config.export_shard_rows);

            #[cfg(feature = "s3")]
            if config.export_sink.to_lowercase() == "s3" {
                use crate::export::S3Sink;
                let bucket = config
                    .export_s3_bucket
                    .clone()
                    .unwrap_or_else(|| "hfs-exports".to_string());
                let region = config.export_s3_region.clone();
                let ttl = config.export_presign_ttl_secs;

                info!(bucket = %bucket, "Export controller: InMemory + S3Sink");

                match tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(S3Sink::from_config(
                        bucket.clone(),
                        region,
                        String::new(),
                        ttl,
                    ))
                }) {
                    Ok(sink) => Arc::new(InMemoryController::with_shard_rows(
                        runner_for_export,
                        sink,
                        max_concurrency,
                        shard_rows,
                    )),
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            dir = %config.export_dir,
                            "S3 export sink init failed — falling back to FilesystemSink"
                        );
                        let sink = FilesystemSink::new(&config.export_dir, &config.base_url);
                        Arc::new(InMemoryController::with_shard_rows(
                            runner_for_export,
                            sink,
                            max_concurrency,
                            shard_rows,
                        ))
                    }
                }
            } else {
                info!(dir = %config.export_dir, "Export controller: InMemory + FilesystemSink");
                let sink = FilesystemSink::new(&config.export_dir, &config.base_url);
                Arc::new(InMemoryController::with_shard_rows(
                    runner_for_export,
                    sink,
                    max_concurrency,
                    shard_rows,
                ))
            }

            #[cfg(not(feature = "s3"))]
            {
                info!(dir = %config.export_dir, "Export controller: InMemory + FilesystemSink");
                let sink = FilesystemSink::new(&config.export_dir, &config.base_url);
                Arc::new(InMemoryController::with_shard_rows(
                    runner_for_export,
                    sink,
                    max_concurrency,
                    shard_rows,
                ))
            }
        };
        state = state.with_export_controller(controller);

        // Wire raw SQL query runner when explicitly enabled + URL provided.
        if config.sof_sql_query_enabled {
            if let Some(ref url) = config.sof_readonly_url {
                use helios_persistence::core::raw_sql::RawSqlRunner;
                let is_pg = url.starts_with("postgres://") || url.starts_with("postgresql://");

                // PostgreSQL raw runner (only when postgres feature is compiled in).
                #[cfg(feature = "postgres")]
                if is_pg {
                    use helios_persistence::raw_sql::PgRawRunner;
                    info!(url = %url, "Raw SQL runner: PgRawRunner");
                    state = state.with_raw_sql_runner(
                        Arc::new(PgRawRunner::new(url.clone())) as Arc<dyn RawSqlRunner>
                    );
                }

                // SQLite raw runner (only when sqlite feature is compiled in).
                #[cfg(feature = "sqlite")]
                if !is_pg {
                    use helios_persistence::raw_sql::SqliteRawRunner;
                    info!(url = %url, "Raw SQL runner: SqliteRawRunner");
                    state =
                        state
                            .with_raw_sql_runner(Arc::new(SqliteRawRunner::new(url.clone()))
                                as Arc<dyn RawSqlRunner>);
                }

                if state.raw_sql_runner().is_none() {
                    tracing::warn!(
                        url = %url,
                        "HFS_SOF_READONLY_URL set but no matching backend feature \
                         is compiled in; $sqlquery-run will return 501"
                    );
                }
            }
        }
    }

    // Build the router with all FHIR routes
    let router = routing::fhir_routes::create_routes(state);

    // Apply auth middleware if enabled (outermost = runs first)
    let router = if let Some(ref auth) = auth_state {
        router
            .layer(axum::middleware::from_fn_with_state(
                auth.clone(),
                middleware::auth::authz_middleware,
            ))
            .layer(axum::middleware::from_fn_with_state(
                auth.clone(),
                middleware::auth::auth_middleware,
            ))
    } else {
        router
    };

    // Build middleware stack
    let service_builder = ServiceBuilder::new()
        .layer(TraceLayer::new_for_http())
        .layer(TimeoutLayer::with_status_code(
            axum::http::StatusCode::REQUEST_TIMEOUT,
            std::time::Duration::from_secs(config.request_timeout),
        ));

    // Add CORS if enabled
    let router = if config.enable_cors {
        let cors = build_cors_layer(&config);
        router.layer(cors)
    } else {
        router
    };

    // Apply remaining middleware
    router.layer(service_builder)
}

/// Builds the CORS layer based on configuration.
fn build_cors_layer(config: &ServerConfig) -> CorsLayer {
    let mut cors = CorsLayer::new();

    // Configure origins
    if config.cors_origins == "*" {
        cors = cors.allow_origin(Any);
    } else {
        let origins: Vec<_> = config
            .cors_origins
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();
        cors = cors.allow_origin(origins);
    }

    // Configure methods
    if config.cors_methods == "*" {
        cors = cors.allow_methods(Any);
    } else {
        let methods: Vec<_> = config
            .cors_methods
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();
        cors = cors.allow_methods(methods);
    }

    // Configure headers
    if config.cors_headers == "*" {
        cors = cors.allow_headers(Any);
    } else {
        let headers: Vec<_> = config
            .cors_headers
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();
        cors = cors.allow_headers(headers);
    }

    cors
}

/// Initializes the tracing subscriber for logging.
///
/// This should be called once at application startup.
///
/// # Arguments
///
/// * `level` - The log level (error, warn, info, debug, trace)
pub fn init_logging(level: &str) {
    use tracing_subscriber::{EnvFilter, fmt, prelude::*};

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new(format!(
            "helios_hfs={},helios_rest={},helios_persistence={},tower_http=debug",
            level, level, level
        ))
    });

    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(filter)
        .init();
}
