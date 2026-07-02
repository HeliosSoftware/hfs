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
//! | `HFS_MAX_BODY_SIZE` | 10485760 | Max request body size (bytes; measured after decompression for compressed requests) |
//! | `HFS_REQUEST_TIMEOUT` | 30 | Request timeout (seconds) |
//! | `HFS_ENABLE_CORS` | true | Enable CORS |
//! | `HFS_CORS_ORIGINS` | * | Allowed CORS origins |
//! | `HFS_DEFAULT_TENANT` | default | Default tenant ID |
//!
//! ## HTTP Compression
//!
//! Request bodies sent with `Content-Encoding: gzip` (or `deflate`, `br`,
//! `zstd`) are decompressed transparently before parsing; unsupported
//! encodings are rejected with `415 Unsupported Media Type`. Responses are
//! compressed when the client advertises support via `Accept-Encoding`, with
//! `Content-Encoding` and `Vary: Accept-Encoding` set accordingly.
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

pub mod bulk_export_auth;
pub mod bulk_submit_fetcher;
pub mod bulk_submit_oauth;
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
pub mod terminology;

// Re-export commonly used types
pub use config::{MultitenancyConfig, ServerConfig, StorageBackendMode, TenantRoutingMode};
pub use error::{RestError, RestResult};
pub use middleware::auth::AuthMiddlewareState;
pub use state::AppState;
pub use tenant::{ResolvedTenant, TenantResolver, TenantSource};

use std::sync::Arc;

use axum::{Router, extract::DefaultBodyLimit};
use helios_persistence::core::{
    BundleProvider, ConditionalStorage, IncludeProvider, InstanceHistoryProvider, ResourceStorage,
    RevincludeProvider, SearchProvider, SystemHistoryProvider, TypeHistoryProvider,
};
use tower::ServiceBuilder;
use tower_http::{
    compression::CompressionLayer,
    compression::predicate::{NotForContentType, Predicate, SizeAbove},
    cors::{Any, CorsLayer},
    decompression::RequestDecompressionLayer,
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
        + IncludeProvider
        + RevincludeProvider
        + InstanceHistoryProvider
        + TypeHistoryProvider
        + SystemHistoryProvider
        + BundleProvider
        + helios_persistence::core::ExportDataProvider
        + helios_persistence::core::PatientExportProvider
        + helios_persistence::core::GroupExportProvider
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
        + IncludeProvider
        + RevincludeProvider
        + InstanceHistoryProvider
        + TypeHistoryProvider
        + SystemHistoryProvider
        + BundleProvider
        + helios_persistence::core::ExportDataProvider
        + helios_persistence::core::PatientExportProvider
        + helios_persistence::core::GroupExportProvider
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

/// The bulk-export job store, output store, and download authorizer, wired
/// into [`AppState`] by [`create_app_with_auth_and_bulk_export`].
pub struct BulkExportBundle {
    /// Job-state store (claim + worker storage + lifecycle).
    pub jobs: Arc<dyn helios_persistence::core::BulkExportJobStore>,
    /// Output store for NDJSON parts.
    pub output: Arc<dyn helios_persistence::core::ExportOutputStore>,
    /// Download authorizer.
    pub file_auth: Arc<dyn bulk_export_auth::ExportFileAuth>,
}

/// The bulk-submit job store, input fetcher, output store, and download
/// authorizer, wired into [`AppState`] by [`create_app_with_auth_and_bulk_export`].
pub struct BulkSubmitBundle {
    /// Job-state store (claim + worker storage + ingestion engine + lifecycle).
    pub jobs: Arc<dyn helios_persistence::core::BulkSubmitJobStore>,
    /// Remote input fetcher (manifest + NDJSON retrieval).
    pub fetcher: Arc<dyn helios_persistence::core::SubmitInputFetcher>,
    /// Output store for status-manifest artifacts.
    pub output: Arc<dyn helios_persistence::core::ExportOutputStore>,
    /// Download authorizer (reuses the export file-auth trait).
    pub file_auth: Arc<dyn bulk_export_auth::ExportFileAuth>,
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
        + IncludeProvider
        + RevincludeProvider
        + InstanceHistoryProvider
        + TypeHistoryProvider
        + SystemHistoryProvider
        + BundleProvider
        + helios_persistence::core::ExportDataProvider
        + helios_persistence::core::PatientExportProvider
        + helios_persistence::core::GroupExportProvider
        + Send
        + Sync
        + 'static,
{
    build_app(
        Arc::new(storage),
        config,
        auth_config,
        auth_state,
        audit_state,
        None,
        None,
    )
}

/// Like [`create_app_with_auth`], but also wires the bulk-export subsystem
/// (job store, output store, download authorizer) into the application state.
pub fn create_app_with_auth_and_bulk_export<S>(
    storage: Arc<S>,
    config: ServerConfig,
    auth_config: helios_auth::AuthConfig,
    auth_state: Option<Arc<middleware::auth::AuthMiddlewareState>>,
    audit_state: Option<Arc<helios_audit::AuditMiddlewareState>>,
    bulk_export: BulkExportBundle,
) -> Router
where
    S: ResourceStorage
        + ConditionalStorage
        + SearchProvider
        + IncludeProvider
        + RevincludeProvider
        + InstanceHistoryProvider
        + TypeHistoryProvider
        + SystemHistoryProvider
        + BundleProvider
        + helios_persistence::core::ExportDataProvider
        + helios_persistence::core::PatientExportProvider
        + helios_persistence::core::GroupExportProvider
        + Send
        + Sync
        + 'static,
{
    build_app(
        storage,
        config,
        auth_config,
        auth_state,
        audit_state,
        Some(bulk_export),
        None,
    )
}

/// Like [`create_app_with_auth`], but wires **either or both** of the bulk-export
/// and bulk-submit subsystems. Either bundle may be `None` independently, so bulk
/// submit can be enabled with bulk export disabled (and vice versa).
#[allow(clippy::too_many_arguments)]
pub fn create_app_with_auth_and_bulk<S>(
    storage: Arc<S>,
    config: ServerConfig,
    auth_config: helios_auth::AuthConfig,
    auth_state: Option<Arc<middleware::auth::AuthMiddlewareState>>,
    audit_state: Option<Arc<helios_audit::AuditMiddlewareState>>,
    bulk_export: Option<BulkExportBundle>,
    bulk_submit: Option<BulkSubmitBundle>,
) -> Router
where
    S: ResourceStorage
        + ConditionalStorage
        + SearchProvider
        + IncludeProvider
        + RevincludeProvider
        + InstanceHistoryProvider
        + TypeHistoryProvider
        + SystemHistoryProvider
        + BundleProvider
        + helios_persistence::core::ExportDataProvider
        + helios_persistence::core::PatientExportProvider
        + helios_persistence::core::GroupExportProvider
        + Send
        + Sync
        + 'static,
{
    build_app(
        storage,
        config,
        auth_config,
        auth_state,
        audit_state,
        bulk_export,
        bulk_submit,
    )
}

/// Internal app builder shared by [`create_app_with_auth`] and
/// [`create_app_with_auth_and_bulk_export`].
fn build_app<S>(
    storage: Arc<S>,
    config: ServerConfig,
    auth_config: helios_auth::AuthConfig,
    auth_state: Option<Arc<middleware::auth::AuthMiddlewareState>>,
    audit_state: Option<Arc<helios_audit::AuditMiddlewareState>>,
    bulk_export: Option<BulkExportBundle>,
    bulk_submit: Option<BulkSubmitBundle>,
) -> Router
where
    S: ResourceStorage
        + ConditionalStorage
        + SearchProvider
        + IncludeProvider
        + RevincludeProvider
        + InstanceHistoryProvider
        + TypeHistoryProvider
        + SystemHistoryProvider
        + BundleProvider
        + helios_persistence::core::ExportDataProvider
        + helios_persistence::core::PatientExportProvider
        + helios_persistence::core::GroupExportProvider
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

    // Storage arrives pre-wrapped in an Arc so we can share it with the SofRunner.
    let storage_arc = storage;

    let (app_audit_sink, app_audit_source_observer) = audit_state
        .as_ref()
        .map(|audit| {
            (
                Some(Arc::clone(&audit.sink)),
                audit.config.source_observer.clone(),
            )
        })
        .unwrap_or((None, "Device/hfs".to_string()));

    // Build the outbound auth provider before moving auth_config into the
    // app state — the subscription engine (constructed below) consumes it.
    #[cfg(feature = "subscriptions")]
    let outbound_auth_provider = auth_config.outbound_provider();

    // Create application state
    let mut state = AppState::with_auth_and_audit(
        Arc::clone(&storage_arc),
        config.clone(),
        auth_config,
        auth_state.clone(),
        app_audit_sink,
        app_audit_source_observer,
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
    }

    // Wire the bulk-export subsystem if provided.
    let state = match bulk_export {
        Some(b) => state.with_bulk_export(b.jobs, b.output, b.file_auth),
        None => state,
    };

    // Wire the bulk-submit subsystem if provided.
    let state = match bulk_submit {
        Some(b) => state.with_bulk_submit(b.jobs, b.fetcher, b.output, b.file_auth),
        None => state,
    };

    // Inject subscription engine if enabled
    #[cfg(feature = "subscriptions")]
    let state = {
        let subscriptions_enabled = std::env::var("HFS_SUBSCRIPTIONS_ENABLED")
            .map(|v| !matches!(v.trim().to_ascii_lowercase().as_str(), "false" | "0"))
            .unwrap_or(false);
        if subscriptions_enabled {
            let smtp = build_smtp_settings_from_env();
            let messaging = build_messaging_settings_from_env(&config.base_url);
            let mut supported = vec!["rest-hook".to_string(), "websocket".to_string()];
            if smtp.is_some() {
                supported.push("email".to_string());
                info!("Email subscription channel ENABLED");
            }
            if messaging.is_some() {
                supported.push("message".to_string());
                info!("FHIR Messaging subscription channel ENABLED");
            }
            let default_sub_config = helios_subscriptions::SubscriptionConfig::default();
            let sub_config = helios_subscriptions::SubscriptionConfig {
                supported_channel_types: supported,
                smtp,
                messaging,
                handshake_initial_delay: subscription_duration_ms_from_env(
                    "HFS_SUBSCRIPTION_HANDSHAKE_INITIAL_DELAY_MS",
                    default_sub_config.handshake_initial_delay,
                ),
                handshake_max_attempts: subscription_u32_from_env(
                    "HFS_SUBSCRIPTION_HANDSHAKE_MAX_ATTEMPTS",
                    default_sub_config.handshake_max_attempts,
                )
                .max(1),
                handshake_retry_initial_delay: subscription_duration_ms_from_env(
                    "HFS_SUBSCRIPTION_HANDSHAKE_RETRY_BASE_MS",
                    default_sub_config.handshake_retry_initial_delay,
                ),
                handshake_retry_max_delay: subscription_duration_ms_from_env(
                    "HFS_SUBSCRIPTION_HANDSHAKE_RETRY_MAX_MS",
                    default_sub_config.handshake_retry_max_delay,
                ),
                ..default_sub_config
            };
            // Outbound auth provider was built above (static bearer when
            // HFS_OUTBOUND_BEARER_TOKEN is set, otherwise no-op).
            let engine = helios_subscriptions::SubscriptionEngine::with_outbound_auth(
                sub_config,
                config.base_url.clone(),
                outbound_auth_provider,
            );
            info!("Subscriptions engine ENABLED");
            state.with_subscription_engine(Arc::new(engine))
        } else {
            state
        }
    };

    // Handles to the state for the console-metrics routers (public + protected),
    // merged below — the public tier outside the auth layer, the protected tier
    // behind it (see further down).
    let console_public_state = state.clone();
    let console_protected_state = state.clone();
    let console_admin_state = state.clone();

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

    // Public console metrics — only the server-global `uptime` endpoint — are
    // mounted OUTSIDE the auth/audit layers above (mirroring `/metrics`/`/health`)
    // so the console can show liveness without a bearer token. Still covered by
    // the shared CORS / timeout / body-limit / tracing stack applied below.
    let router = router.merge(routing::console_metrics::public_routes(console_public_state));

    // Tenant-scoped console metrics sit behind the same bearer-token auth as the
    // FHIR routes. The tenant is taken authoritatively from the JWT claim (see
    // TenantExtractor), so a spoofed `X-Tenant-ID` cannot widen access, and each
    // handler only ever reads the caller's own tenant. `authz_middleware` no
    // longer mis-classifies these `/console/*` paths as FHIR operations (see
    // `extract_operation`), so it is a no-op here — authentication alone is the
    // control. When auth is disabled server-wide, this tier is unprotected like
    // every other route, matching existing behaviour.
    let protected_console = routing::console_metrics::protected_routes(console_protected_state);
    let protected_console = if let Some(ref auth) = auth_state {
        protected_console
            .layer(axum::middleware::from_fn_with_state(
                auth.clone(),
                middleware::auth::authz_middleware,
            ))
            .layer(axum::middleware::from_fn_with_state(
                auth.clone(),
                middleware::auth::auth_middleware,
            ))
    } else {
        protected_console
    };
    let router = router.merge(protected_console);

    // Cross-tenant console metrics (`tenants`, `traffic`) expose data spanning
    // every tenant, so they require an administrative, system-context scope on top
    // of authentication. `admin_authz_middleware` requires `system/*.r`, rejecting
    // ordinary user-/patient-context tokens (even wildcard ones) with `403`. As
    // with the other tiers, when auth is disabled server-wide there is no
    // Principal and the middleware passes through, keeping dev-mode behaviour.
    let admin_console = routing::console_metrics::admin_routes(console_admin_state);
    let admin_console = if let Some(ref auth) = auth_state {
        admin_console
            .layer(axum::middleware::from_fn_with_state(
                auth.clone(),
                middleware::auth::admin_authz_middleware,
            ))
            .layer(axum::middleware::from_fn_with_state(
                auth.clone(),
                middleware::auth::auth_middleware,
            ))
    } else {
        admin_console
    };
    let router = router.merge(admin_console);

    // Build middleware stack
    let service_builder = ServiceBuilder::new()
        .layer(TraceLayer::new_for_http())
        .layer(TimeoutLayer::with_status_code(
            axum::http::StatusCode::REQUEST_TIMEOUT,
            std::time::Duration::from_secs(config.request_timeout),
        ));

    // Transparently decompress request bodies sent with `Content-Encoding`
    // (gzip, deflate, br, zstd) and compress responses when the client sends
    // `Accept-Encoding`. Unsupported request encodings get 415. Because the
    // decompression layer replaces the request body before any extractor or
    // handler reads it, the body-size limit below applies to the
    // *decompressed* bytes — a small highly-compressed payload cannot bypass
    // `HFS_MAX_BODY_SIZE`. Kept inside the CORS layer so 415/413 error
    // responses still carry CORS headers for browser clients.
    //
    // Never re-compress Parquet or ZIP output (SoF run/export responses) —
    // both are already compressed, so HTTP-level compression would only burn
    // CPU for no size win. Both parquet media-type identifiers are excluded:
    // `application/vnd.apache.parquet` (the spec's native media type) and
    // the legacy `application/parquet` alias, matching sof-server's
    // predicate.
    let compress_predicate = SizeAbove::new(32)
        .and(NotForContentType::const_new("application/parquet"))
        .and(NotForContentType::const_new(
            "application/vnd.apache.parquet",
        ))
        .and(NotForContentType::const_new("application/zip"));
    let router = router
        .layer(RequestDecompressionLayer::new())
        .layer(CompressionLayer::new().compress_when(compress_predicate));

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

#[cfg(feature = "subscriptions")]
fn subscription_u32_from_env(name: &str, default: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(default)
}

#[cfg(feature = "subscriptions")]
fn subscription_duration_ms_from_env(
    name: &str,
    default: std::time::Duration,
) -> std::time::Duration {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(std::time::Duration::from_millis)
        .unwrap_or(default)
}

/// Build SMTP settings for the email subscription channel from `HFS_SUBSCRIPTION_SMTP_*`
/// environment variables. Returns `None` when `HOST` or `FROM` is unset — in that
/// case the email channel is not advertised and any email Subscription is rejected.
#[cfg(feature = "subscriptions")]
fn build_smtp_settings_from_env() -> Option<helios_subscriptions::config::SmtpSettings> {
    use helios_subscriptions::config::{SmtpEncryption, SmtpSettings};

    let host = std::env::var("HFS_SUBSCRIPTION_SMTP_HOST").ok()?;
    let from_address = std::env::var("HFS_SUBSCRIPTION_SMTP_FROM").ok()?;
    if host.trim().is_empty() || from_address.trim().is_empty() {
        return None;
    }

    let port = std::env::var("HFS_SUBSCRIPTION_SMTP_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(25);
    let encryption = std::env::var("HFS_SUBSCRIPTION_SMTP_ENCRYPTION")
        .ok()
        .as_deref()
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
        .and_then(|v| match v {
            "none" => Some(SmtpEncryption::None),
            "starttls" => Some(SmtpEncryption::StartTls),
            "tls" => Some(SmtpEncryption::Tls),
            _ => None,
        })
        .unwrap_or(SmtpEncryption::StartTls);
    let username = std::env::var("HFS_SUBSCRIPTION_SMTP_USERNAME")
        .ok()
        .filter(|s| !s.is_empty());
    let password = std::env::var("HFS_SUBSCRIPTION_SMTP_PASSWORD")
        .ok()
        .filter(|s| !s.is_empty());
    let default_subject = std::env::var("HFS_SUBSCRIPTION_SMTP_DEFAULT_SUBJECT")
        .ok()
        .filter(|s| !s.is_empty());
    let timeout_secs = std::env::var("HFS_SUBSCRIPTION_SMTP_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30);

    Some(SmtpSettings {
        host,
        port,
        username,
        password,
        encryption,
        from_address,
        default_subject,
        timeout_secs,
    })
}

#[cfg(feature = "subscriptions")]
fn build_messaging_settings_from_env(
    base_url: &str,
) -> Option<helios_subscriptions::config::MessagingSettings> {
    use helios_subscriptions::config::MessagingSettings;

    let enabled = std::env::var("HFS_SUBSCRIPTION_MESSAGING_ENABLED")
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "true" | "1"))
        .unwrap_or(false);
    if !enabled {
        return None;
    }

    let source_endpoint = std::env::var("HFS_SUBSCRIPTION_MESSAGE_SOURCE_ENDPOINT")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| base_url.to_string());

    let allow_private_endpoints = std::env::var("HFS_SUBSCRIPTION_ALLOW_PRIVATE_ENDPOINTS")
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "true" | "1"))
        .unwrap_or(false);

    Some(MessagingSettings {
        source_endpoint,
        allow_private_endpoints,
    })
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
            "helios_hfs={},helios_rest={},helios_persistence={},helios_subscriptions={},tower_http=debug",
            level, level, level, level
        ))
    });

    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(filter)
        .init();
}
