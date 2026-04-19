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
pub mod extractors;
pub mod fhir_types;
pub mod handlers;
pub mod middleware;
pub mod responses;
pub mod routing;
pub mod state;
pub mod tenant;
pub mod terminology;

// Re-export commonly used types
pub use config::{MultitenancyConfig, ServerConfig, StorageBackendMode, TenantRoutingMode};
pub use error::{RestError, RestResult};
pub use middleware::auth::AuthMiddlewareState;
pub use state::AppState;
pub use tenant::{ResolvedTenant, TenantResolver, TenantSource};

use std::sync::Arc;

use axum::{extract::DefaultBodyLimit, Router};
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
    create_app_with_auth(
        storage,
        config,
        helios_auth::AuthConfig::default(),
        None,
        None,
    )
}

/// Creates the Axum application with custom configuration and optional authentication.
///
/// When `auth_state` is `Some`, authentication and authorization middleware
/// are added to the middleware stack.
///
/// When `audit_state` is `Some`, audit middleware is added to record FHIR
/// operation events as `AuditEvent` resources.
pub fn create_app_with_auth<S>(
    storage: S,
    config: ServerConfig,
    auth_config: helios_auth::AuthConfig,
    auth_state: Option<Arc<middleware::auth::AuthMiddlewareState>>,
    audit_state: Option<Arc<helios_audit::AuditMiddlewareState>>,
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

    let (app_audit_sink, app_audit_source_observer) = audit_state
        .as_ref()
        .map(|audit| {
            (
                Some(Arc::clone(&audit.sink)),
                audit.config.source_observer.clone(),
            )
        })
        .unwrap_or((None, "Device/hfs".to_string()));

    // Create application state
    let state = AppState::with_auth_and_audit(
        Arc::new(storage),
        config.clone(),
        auth_config,
        auth_state.clone(),
        app_audit_sink,
        app_audit_source_observer,
    );

    // Inject subscription engine if enabled
    #[cfg(feature = "subscriptions")]
    let state = {
        let subscriptions_enabled = std::env::var("HFS_SUBSCRIPTIONS_ENABLED")
            .map(|v| !matches!(v.trim().to_ascii_lowercase().as_str(), "false" | "0"))
            .unwrap_or(false);
        if subscriptions_enabled {
            let sub_config = helios_subscriptions::SubscriptionConfig::default();
            let engine =
                helios_subscriptions::SubscriptionEngine::new(sub_config, config.base_url.clone());
            info!("Subscriptions engine ENABLED");
            state.with_subscription_engine(Arc::new(engine))
        } else {
            state
        }
    };

    // Build the router with all FHIR routes
    let router = routing::fhir_routes::create_routes(state);

    // Apply audit middleware if enabled (inner layer = runs after auth)
    let router = if let Some(audit) = audit_state {
        router.layer(axum::middleware::from_fn_with_state(
            audit,
            helios_audit::middleware::audit_middleware,
        ))
    } else {
        router
    };

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

    // Raise the body-size limit from axum's 2 MiB default to the configured
    // ceiling. Without this, `HFS_MAX_BODY_SIZE` / `--max-body-size` has no
    // effect on the REST router: individual batch/transaction handlers read
    // their body via `axum::body::to_bytes(..., config.max_body_size)`, but
    // axum's `DefaultBodyLimit` extractor runs first and rejects any request
    // > 2 MiB with 413 "length limit exceeded" before the handler is called.
    // Mirrors the pattern already used in `crates/sof/src/server.rs`.
    let router = router.layer(DefaultBodyLimit::max(config.max_body_size));

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
