//! Server configuration for the FHIR REST API.
//!
//! This module provides configuration types for the REST server, supporting
//! both programmatic configuration and environment variable overrides.
//!
//! # Environment Variables
//!
//! | Variable | Default | Description |
//! |----------|---------|-------------|
//! | `HFS_SERVER_PORT` | 8080 | Server port |
//! | `HFS_SERVER_HOST` | 127.0.0.1 | Host to bind |
//! | `HFS_LOG_LEVEL` | info | Log level |
//! | `HFS_MAX_BODY_SIZE` | 10485760 | Max request body (bytes; applies to the decompressed body for compressed requests) |
//! | `HFS_REQUEST_TIMEOUT` | 30 | Request timeout (seconds) |
//! | `HFS_ENABLE_CORS` | true | Enable CORS |
//! | `HFS_CORS_ORIGINS` | * | Allowed origins |
//! | `HFS_CORS_METHODS` | GET,POST,PUT,PATCH,DELETE,OPTIONS | Allowed methods |
//! | `HFS_CORS_HEADERS` | Content-Type,Authorization,Accept,If-Match,If-None-Match,Prefer,Content-Encoding | Allowed headers |
//! | `HFS_DEFAULT_TENANT` | default | Default tenant ID |
//! | `HFS_BASE_URL` | http://localhost:8080 | Server base URL |
//! | `HFS_DEFAULT_FHIR_VERSION` | R4 | Default FHIR version (R4, R4B, R5, R6) |
//! | `HFS_TENANT_ROUTING_MODE` | header_only | Tenant routing mode (header_only, url_path, both) |
//! | `HFS_TENANT_STRICT_VALIDATION` | false | Error if URL and header tenant disagree |
//! | `HFS_JWT_TENANT_CLAIM` | tenant_id | JWT claim name for tenant (future use) |
//! | `HFS_TERMINOLOGY_SERVER` | (none) | HTS base URL for `:in`/`:not-in` search and FHIRPath terminology functions |
//!
//! # Example
//!
//! ```rust
//! use helios_rest::ServerConfig;
//!
//! // Create from environment
//! let config = ServerConfig::from_env();
//!
//! // Or create programmatically
//! let config = ServerConfig {
//!     port: 3000,
//!     host: "0.0.0.0".to_string(),
//!     enable_cors: true,
//!     ..Default::default()
//! };
//! ```

use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

use clap::Parser;
use helios_fhir::FhirVersion;
use helios_persistence::BackendKind;

/// Storage backend mode.
///
/// Determines which backend configuration the server uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StorageBackendMode {
    /// SQLite only (default). Zero configuration required.
    #[default]
    Sqlite,
    /// SQLite for CRUD + Elasticsearch for search.
    /// Requires a running Elasticsearch instance.
    SqliteElasticsearch,
    /// PostgreSQL only. Requires a running PostgreSQL instance.
    Postgres,
    /// PostgreSQL for CRUD + Elasticsearch for search.
    /// Requires running PostgreSQL and Elasticsearch instances.
    PostgresElasticsearch,
    /// MongoDB only. Requires a running MongoDB instance.
    MongoDB,
    /// MongoDB for CRUD + Elasticsearch for search.
    /// Requires running MongoDB and Elasticsearch instances.
    MongoDBElasticsearch,
    /// AWS S3 object storage for CRUD, versioning, history, and bulk operations.
    /// Requires AWS credentials via the standard provider chain. No search support.
    S3,
    /// AWS S3 for CRUD/history + Elasticsearch for search.
    /// Requires AWS credentials and a running Elasticsearch instance.
    S3Elasticsearch,
}

impl StorageBackendMode {
    /// Returns the primary storage backend kind for this mode.
    pub fn primary_backend_kind(self) -> BackendKind {
        match self {
            StorageBackendMode::Sqlite | StorageBackendMode::SqliteElasticsearch => {
                BackendKind::Sqlite
            }
            StorageBackendMode::Postgres | StorageBackendMode::PostgresElasticsearch => {
                BackendKind::Postgres
            }
            StorageBackendMode::MongoDB | StorageBackendMode::MongoDBElasticsearch => {
                BackendKind::MongoDB
            }
            StorageBackendMode::S3 | StorageBackendMode::S3Elasticsearch => BackendKind::S3,
        }
    }
}

impl fmt::Display for StorageBackendMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StorageBackendMode::Sqlite => write!(f, "sqlite"),
            StorageBackendMode::SqliteElasticsearch => write!(f, "sqlite-elasticsearch"),
            StorageBackendMode::Postgres => write!(f, "postgres"),
            StorageBackendMode::PostgresElasticsearch => {
                write!(f, "postgres-elasticsearch")
            }
            StorageBackendMode::MongoDB => write!(f, "mongodb"),
            StorageBackendMode::MongoDBElasticsearch => write!(f, "mongodb-elasticsearch"),
            StorageBackendMode::S3 => write!(f, "s3"),
            StorageBackendMode::S3Elasticsearch => write!(f, "s3-elasticsearch"),
        }
    }
}

impl FromStr for StorageBackendMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().replace('_', "-").as_str() {
            "sqlite" => Ok(StorageBackendMode::Sqlite),
            "sqlite-elasticsearch" | "sqlite-es" => Ok(StorageBackendMode::SqliteElasticsearch),
            "postgres" | "pg" | "postgresql" => Ok(StorageBackendMode::Postgres),
            "postgres-elasticsearch" | "postgres-es" | "pg-elasticsearch" | "pg-es" => {
                Ok(StorageBackendMode::PostgresElasticsearch)
            }
            "mongodb" | "mongo" => Ok(StorageBackendMode::MongoDB),
            "mongodb-elasticsearch" | "mongodb-es" | "mongo-elasticsearch" | "mongo-es" => {
                Ok(StorageBackendMode::MongoDBElasticsearch)
            }
            "s3" | "objectstore" => Ok(StorageBackendMode::S3),
            "s3-elasticsearch" | "s3-es" => Ok(StorageBackendMode::S3Elasticsearch),
            _ => Err(format!(
                "Invalid storage backend '{}'. Valid values: sqlite, sqlite-elasticsearch, postgres, postgres-elasticsearch, mongodb, mongodb-elasticsearch, s3, s3-elasticsearch",
                s
            )),
        }
    }
}

/// Tenant routing mode for multi-tenant deployments.
///
/// Determines how the server identifies tenants from incoming requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TenantRoutingMode {
    /// Tenant identified only from X-Tenant-ID header (default, backward compatible).
    #[default]
    HeaderOnly,
    /// Tenant identified from URL path prefix: `/{tenant}/Patient/123`.
    UrlPath,
    /// Both URL and header supported; URL takes precedence over header.
    Both,
}

impl TenantRoutingMode {
    /// Returns true if URL-based tenant routing is enabled.
    pub fn supports_url_path(&self) -> bool {
        matches!(self, TenantRoutingMode::UrlPath | TenantRoutingMode::Both)
    }

    /// Returns true if header-based tenant routing is enabled.
    pub fn supports_header(&self) -> bool {
        matches!(
            self,
            TenantRoutingMode::HeaderOnly | TenantRoutingMode::Both
        )
    }
}

impl fmt::Display for TenantRoutingMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TenantRoutingMode::HeaderOnly => write!(f, "header_only"),
            TenantRoutingMode::UrlPath => write!(f, "url_path"),
            TenantRoutingMode::Both => write!(f, "both"),
        }
    }
}

impl FromStr for TenantRoutingMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "header_only" | "headeronly" | "header" => Ok(TenantRoutingMode::HeaderOnly),
            "url_path" | "urlpath" | "url" | "path" => Ok(TenantRoutingMode::UrlPath),
            "both" | "combined" => Ok(TenantRoutingMode::Both),
            _ => Err(format!(
                "Invalid tenant routing mode '{}'. Valid values: header_only, url_path, both",
                s
            )),
        }
    }
}

/// Configuration for multi-tenant behavior.
#[derive(Debug, Clone)]
pub struct MultitenancyConfig {
    /// How tenants are identified from requests.
    pub routing_mode: TenantRoutingMode,
    /// If true, error when URL path and header specify different tenants.
    pub strict_validation: bool,
    /// JWT claim name containing tenant ID (for future JWT-based tenant resolution).
    pub jwt_tenant_claim: String,
}

impl Default for MultitenancyConfig {
    fn default() -> Self {
        Self {
            routing_mode: TenantRoutingMode::HeaderOnly,
            strict_validation: false,
            jwt_tenant_claim: "tenant_id".to_string(),
        }
    }
}

impl MultitenancyConfig {
    /// Creates a new MultitenancyConfig from environment variables.
    pub fn from_env() -> Self {
        let routing_mode = std::env::var("HFS_TENANT_ROUTING_MODE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or_default();

        let strict_validation = std::env::var("HFS_TENANT_STRICT_VALIDATION")
            .map(|s| s.to_lowercase() == "true" || s == "1")
            .unwrap_or(false);

        let jwt_tenant_claim =
            std::env::var("HFS_JWT_TENANT_CLAIM").unwrap_or_else(|_| "tenant_id".to_string());

        Self {
            routing_mode,
            strict_validation,
            jwt_tenant_claim,
        }
    }
}

/// Configuration for the bulk data export subsystem.
#[derive(Debug, Clone)]
pub struct BulkExportConfig {
    /// Master switch — when `false`, the `$export` endpoints return `501`.
    pub enabled: bool,
    /// Output store: `local-fs` or `s3`.
    pub output_backend: String,
    /// Local-FS output root directory.
    pub output_dir: Option<String>,
    /// S3 bucket for output (required when `output_backend = s3`).
    pub s3_bucket: Option<String>,
    /// Manifest access-token posture: `auto`, `true`, or `false`.
    pub requires_access_token: String,
    /// Pre-signed download-URL lifetime, in seconds.
    pub file_url_ttl_secs: u64,
    /// How long output files are retained after job completion, in seconds.
    pub output_ttl_secs: u64,
    /// Maximum jobs this pod runs concurrently.
    pub worker_concurrency: u32,
    /// When `true`, this pod does not run in-process workers.
    pub disable_local_worker: bool,
    /// Cap on simultaneous in-flight jobs per tenant.
    pub max_concurrent_per_tenant: u32,
    /// Resources per `fetch_export_batch` call.
    pub batch_size: u32,
    /// Initial lease length issued at claim, in seconds.
    pub lease_duration_secs: u64,
    /// Worker heartbeat cadence, in seconds.
    pub heartbeat_interval_secs: u64,
    /// How often the cleanup task scans for expired outputs, in seconds.
    pub cleanup_interval_secs: u64,
    /// Group export `_since` toggle (`include` / `exclude`).
    ///
    /// When `exclude`, patients whose `Group.member.period.start` is *after*
    /// the request's `_since` are filtered out of the export — implementing
    /// the IG's optional "do not return resources from before the patient
    /// joined the cohort" behavior.
    pub since_newly_added: String,
}

impl Default for BulkExportConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            output_backend: "local-fs".to_string(),
            output_dir: None,
            s3_bucket: None,
            requires_access_token: "auto".to_string(),
            file_url_ttl_secs: 3600,
            output_ttl_secs: 86400,
            worker_concurrency: 2,
            disable_local_worker: false,
            max_concurrent_per_tenant: 4,
            batch_size: 1000,
            lease_duration_secs: 60,
            heartbeat_interval_secs: 20,
            cleanup_interval_secs: 300,
            since_newly_added: "include".to_string(),
        }
    }
}

impl BulkExportConfig {
    /// Loads bulk-export configuration from `HFS_BULK_EXPORT_*` env vars.
    pub fn from_env() -> Self {
        fn env_bool(key: &str, default: bool) -> bool {
            std::env::var(key)
                .map(|s| {
                    let s = s.to_lowercase();
                    s == "true" || s == "1"
                })
                .unwrap_or(default)
        }
        fn env_u64(key: &str, default: u64) -> u64 {
            std::env::var(key)
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(default)
        }
        fn env_u32(key: &str, default: u32) -> u32 {
            std::env::var(key)
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(default)
        }
        let d = Self::default();
        Self {
            enabled: env_bool("HFS_BULK_EXPORT_ENABLED", d.enabled),
            output_backend: std::env::var("HFS_BULK_EXPORT_OUTPUT_BACKEND")
                .unwrap_or(d.output_backend),
            output_dir: std::env::var("HFS_BULK_EXPORT_OUTPUT_DIR").ok(),
            s3_bucket: std::env::var("HFS_BULK_EXPORT_S3_BUCKET").ok(),
            requires_access_token: std::env::var("HFS_BULK_EXPORT_REQUIRES_ACCESS_TOKEN")
                .unwrap_or(d.requires_access_token),
            file_url_ttl_secs: env_u64("HFS_BULK_EXPORT_FILE_URL_TTL", d.file_url_ttl_secs),
            output_ttl_secs: env_u64("HFS_BULK_EXPORT_OUTPUT_TTL", d.output_ttl_secs),
            worker_concurrency: env_u32("HFS_BULK_EXPORT_WORKER_CONCURRENCY", d.worker_concurrency),
            disable_local_worker: env_bool(
                "HFS_BULK_EXPORT_DISABLE_LOCAL_WORKER",
                d.disable_local_worker,
            ),
            max_concurrent_per_tenant: env_u32(
                "HFS_BULK_EXPORT_MAX_CONCURRENT_PER_TENANT",
                d.max_concurrent_per_tenant,
            ),
            batch_size: env_u32("HFS_BULK_EXPORT_BATCH_SIZE", d.batch_size),
            lease_duration_secs: env_u64("HFS_BULK_EXPORT_LEASE_DURATION", d.lease_duration_secs),
            heartbeat_interval_secs: env_u64(
                "HFS_BULK_EXPORT_HEARTBEAT_INTERVAL",
                d.heartbeat_interval_secs,
            ),
            cleanup_interval_secs: env_u64(
                "HFS_BULK_EXPORT_CLEANUP_INTERVAL",
                d.cleanup_interval_secs,
            ),
            since_newly_added: std::env::var("HFS_BULK_EXPORT_SINCE_NEWLY_ADDED")
                .unwrap_or(d.since_newly_added),
        }
    }

    /// Validates the bulk-export configuration.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();
        if !matches!(self.output_backend.as_str(), "local-fs" | "s3") {
            errors.push(format!(
                "HFS_BULK_EXPORT_OUTPUT_BACKEND '{}' invalid (expected local-fs|s3)",
                self.output_backend
            ));
        }
        if self.output_backend == "s3" && self.s3_bucket.is_none() {
            errors.push("HFS_BULK_EXPORT_S3_BUCKET is required when OUTPUT_BACKEND=s3".to_string());
        }
        if !matches!(
            self.requires_access_token.as_str(),
            "auto" | "true" | "false"
        ) {
            errors.push(format!(
                "HFS_BULK_EXPORT_REQUIRES_ACCESS_TOKEN '{}' invalid (expected auto|true|false)",
                self.requires_access_token
            ));
        }
        // local-fs has no pre-signed-URL capability.
        if self.output_backend == "local-fs" && self.requires_access_token == "false" {
            errors.push(
                "HFS_BULK_EXPORT_REQUIRES_ACCESS_TOKEN=false is invalid with OUTPUT_BACKEND=local-fs"
                    .to_string(),
            );
        }
        if self.file_url_ttl_secs == 0 {
            errors.push("HFS_BULK_EXPORT_FILE_URL_TTL must be > 0".to_string());
        }
        if self.output_ttl_secs == 0 {
            errors.push("HFS_BULK_EXPORT_OUTPUT_TTL must be > 0".to_string());
        }
        if self.worker_concurrency == 0 {
            errors.push("HFS_BULK_EXPORT_WORKER_CONCURRENCY must be >= 1".to_string());
        }
        if self.max_concurrent_per_tenant == 0 {
            errors.push("HFS_BULK_EXPORT_MAX_CONCURRENT_PER_TENANT must be >= 1".to_string());
        }
        if self.batch_size == 0 {
            errors.push("HFS_BULK_EXPORT_BATCH_SIZE must be >= 1".to_string());
        }
        if self.heartbeat_interval_secs == 0 {
            errors.push("HFS_BULK_EXPORT_HEARTBEAT_INTERVAL must be > 0".to_string());
        }
        if self.lease_duration_secs <= self.heartbeat_interval_secs {
            errors.push(
                "HFS_BULK_EXPORT_LEASE_DURATION must be greater than HEARTBEAT_INTERVAL"
                    .to_string(),
            );
        }
        if !matches!(self.since_newly_added.as_str(), "include" | "exclude") {
            errors.push(format!(
                "HFS_BULK_EXPORT_SINCE_NEWLY_ADDED '{}' invalid (expected include|exclude)",
                self.since_newly_added
            ));
        }
        if self.cleanup_interval_secs == 0 {
            errors.push("HFS_BULK_EXPORT_CLEANUP_INTERVAL must be > 0".to_string());
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// Bulk Data **Submit** (`$bulk-submit`) configuration, loaded from
/// `HFS_BULK_SUBMIT_*` environment variables.
///
/// HFS acts as the Data Consumer: it accepts submissions, fetches the referenced
/// manifests/files, ingests them, and serves a status manifest whose
/// `output`/`error`/`deleted` artifacts are written to the output store below.
#[derive(Debug, Clone)]
pub struct BulkSubmitConfig {
    /// Master switch — when `false`, the `$bulk-submit` endpoints return `501`.
    pub enabled: bool,
    /// Output store for status artifacts: `local-fs` or `s3`.
    pub output_backend: String,
    /// Local-FS output root directory.
    pub output_dir: Option<String>,
    /// S3 bucket for status artifacts (required when `output_backend = s3`).
    pub s3_bucket: Option<String>,
    /// Manifest access-token posture: `auto`, `true`, or `false`.
    pub requires_access_token: String,
    /// Pre-signed download-URL lifetime, in seconds.
    pub file_url_ttl_secs: u64,
    /// How long status artifacts are retained after completion, in seconds.
    pub output_ttl_secs: u64,
    /// Maximum manifests this pod ingests concurrently.
    pub worker_concurrency: u32,
    /// When `true`, this pod does not run in-process submit workers.
    pub disable_local_worker: bool,
    /// Cap on simultaneous in-flight submissions per tenant.
    pub max_concurrent_per_tenant: u32,
    /// Resources per ingestion batch.
    pub batch_size: u32,
    /// Initial lease length issued at manifest claim, in seconds.
    pub lease_duration_secs: u64,
    /// Worker heartbeat cadence, in seconds.
    pub heartbeat_interval_secs: u64,
    /// How often the cleanup task scans for expired submissions, in seconds.
    pub cleanup_interval_secs: u64,
    /// When `true`, reject a new submission while one is in-progress (`429`).
    pub block_concurrent_submission: bool,
    /// OAuth `client_id` HFS presents when fetching protected provider files.
    pub client_id: Option<String>,
    /// PEM private key HFS signs its `private_key_jwt` client assertion with.
    pub private_key: Option<String>,
    /// Signing algorithm for the client assertion (`ES384` or `RS384`).
    pub signing_alg: String,
    /// Read scope requested for the outbound file-retrieval token.
    pub outbound_scope: String,
}

impl Default for BulkSubmitConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            output_backend: "local-fs".to_string(),
            output_dir: None,
            s3_bucket: None,
            requires_access_token: "auto".to_string(),
            file_url_ttl_secs: 3600,
            output_ttl_secs: 86400,
            worker_concurrency: 2,
            disable_local_worker: false,
            max_concurrent_per_tenant: 4,
            batch_size: 1000,
            lease_duration_secs: 60,
            heartbeat_interval_secs: 20,
            cleanup_interval_secs: 300,
            block_concurrent_submission: false,
            client_id: None,
            private_key: None,
            signing_alg: "ES384".to_string(),
            outbound_scope: "system/*.rs".to_string(),
        }
    }
}

impl BulkSubmitConfig {
    /// Loads bulk-submit configuration from `HFS_BULK_SUBMIT_*` env vars.
    pub fn from_env() -> Self {
        fn env_bool(key: &str, default: bool) -> bool {
            std::env::var(key)
                .map(|s| {
                    let s = s.to_lowercase();
                    s == "true" || s == "1"
                })
                .unwrap_or(default)
        }
        fn env_u64(key: &str, default: u64) -> u64 {
            std::env::var(key)
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(default)
        }
        fn env_u32(key: &str, default: u32) -> u32 {
            std::env::var(key)
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(default)
        }
        let d = Self::default();
        Self {
            enabled: env_bool("HFS_BULK_SUBMIT_ENABLED", d.enabled),
            output_backend: std::env::var("HFS_BULK_SUBMIT_OUTPUT_BACKEND")
                .unwrap_or(d.output_backend),
            output_dir: std::env::var("HFS_BULK_SUBMIT_OUTPUT_DIR").ok(),
            s3_bucket: std::env::var("HFS_BULK_SUBMIT_S3_BUCKET").ok(),
            requires_access_token: std::env::var("HFS_BULK_SUBMIT_REQUIRES_ACCESS_TOKEN")
                .unwrap_or(d.requires_access_token),
            file_url_ttl_secs: env_u64("HFS_BULK_SUBMIT_FILE_URL_TTL", d.file_url_ttl_secs),
            output_ttl_secs: env_u64("HFS_BULK_SUBMIT_OUTPUT_TTL", d.output_ttl_secs),
            worker_concurrency: env_u32("HFS_BULK_SUBMIT_WORKER_CONCURRENCY", d.worker_concurrency),
            disable_local_worker: env_bool(
                "HFS_BULK_SUBMIT_DISABLE_LOCAL_WORKER",
                d.disable_local_worker,
            ),
            max_concurrent_per_tenant: env_u32(
                "HFS_BULK_SUBMIT_MAX_CONCURRENT_PER_TENANT",
                d.max_concurrent_per_tenant,
            ),
            batch_size: env_u32("HFS_BULK_SUBMIT_BATCH_SIZE", d.batch_size),
            lease_duration_secs: env_u64("HFS_BULK_SUBMIT_LEASE_DURATION", d.lease_duration_secs),
            heartbeat_interval_secs: env_u64(
                "HFS_BULK_SUBMIT_HEARTBEAT_INTERVAL",
                d.heartbeat_interval_secs,
            ),
            cleanup_interval_secs: env_u64(
                "HFS_BULK_SUBMIT_CLEANUP_INTERVAL",
                d.cleanup_interval_secs,
            ),
            block_concurrent_submission: env_bool(
                "HFS_BULK_SUBMIT_BLOCK_CONCURRENT_SUBMISSION",
                d.block_concurrent_submission,
            ),
            client_id: std::env::var("HFS_BULK_SUBMIT_CLIENT_ID").ok(),
            private_key: std::env::var("HFS_BULK_SUBMIT_PRIVATE_KEY").ok(),
            signing_alg: std::env::var("HFS_BULK_SUBMIT_SIGNING_ALG").unwrap_or(d.signing_alg),
            outbound_scope: std::env::var("HFS_BULK_SUBMIT_OUTBOUND_SCOPE")
                .unwrap_or(d.outbound_scope),
        }
    }

    /// Validates the bulk-submit configuration.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();
        if !matches!(self.output_backend.as_str(), "local-fs" | "s3") {
            errors.push(format!(
                "HFS_BULK_SUBMIT_OUTPUT_BACKEND '{}' invalid (expected local-fs|s3)",
                self.output_backend
            ));
        }
        if self.output_backend == "s3" && self.s3_bucket.is_none() {
            errors.push("HFS_BULK_SUBMIT_S3_BUCKET is required when OUTPUT_BACKEND=s3".to_string());
        }
        if !matches!(
            self.requires_access_token.as_str(),
            "auto" | "true" | "false"
        ) {
            errors.push(format!(
                "HFS_BULK_SUBMIT_REQUIRES_ACCESS_TOKEN '{}' invalid (expected auto|true|false)",
                self.requires_access_token
            ));
        }
        if self.output_backend == "local-fs" && self.requires_access_token == "false" {
            errors.push(
                "HFS_BULK_SUBMIT_REQUIRES_ACCESS_TOKEN=false is invalid with OUTPUT_BACKEND=local-fs"
                    .to_string(),
            );
        }
        if !matches!(self.signing_alg.as_str(), "ES384" | "RS384") {
            errors.push(format!(
                "HFS_BULK_SUBMIT_SIGNING_ALG '{}' invalid (expected ES384|RS384)",
                self.signing_alg
            ));
        }
        if self.worker_concurrency == 0 {
            errors.push("HFS_BULK_SUBMIT_WORKER_CONCURRENCY must be >= 1".to_string());
        }
        if self.max_concurrent_per_tenant == 0 {
            errors.push("HFS_BULK_SUBMIT_MAX_CONCURRENT_PER_TENANT must be >= 1".to_string());
        }
        if self.batch_size == 0 {
            errors.push("HFS_BULK_SUBMIT_BATCH_SIZE must be >= 1".to_string());
        }
        if self.heartbeat_interval_secs == 0 {
            errors.push("HFS_BULK_SUBMIT_HEARTBEAT_INTERVAL must be > 0".to_string());
        }
        if self.lease_duration_secs <= self.heartbeat_interval_secs {
            errors.push(
                "HFS_BULK_SUBMIT_LEASE_DURATION must be greater than HEARTBEAT_INTERVAL"
                    .to_string(),
            );
        }
        if self.cleanup_interval_secs == 0 {
            errors.push("HFS_BULK_SUBMIT_CLEANUP_INTERVAL must be > 0".to_string());
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// Server configuration for the FHIR REST API.
///
/// This struct can be constructed from environment variables using [`ServerConfig::from_env`],
/// from command line arguments using [`ServerConfig::parse`], or programmatically.
#[derive(Debug, Clone, Parser)]
#[command(name = "rest-server")]
#[command(about = "FHIR RESTful API Server")]
pub struct ServerConfig {
    /// Port to listen on.
    #[arg(short, long, env = "HFS_SERVER_PORT", default_value = "8080")]
    pub port: u16,

    /// Host address to bind to.
    #[arg(long, env = "HFS_SERVER_HOST", default_value = "127.0.0.1")]
    pub host: String,

    /// Log level (error, warn, info, debug, trace).
    #[arg(long, env = "HFS_LOG_LEVEL", default_value = "info")]
    pub log_level: String,

    /// Maximum request body size in bytes.
    ///
    /// For requests sent with `Content-Encoding`, the limit applies to the
    /// *decompressed* body, so a small highly-compressed payload cannot
    /// bypass it.
    #[arg(long, env = "HFS_MAX_BODY_SIZE", default_value = "10485760")]
    pub max_body_size: usize,

    /// Request timeout in seconds.
    #[arg(long, env = "HFS_REQUEST_TIMEOUT", default_value = "30")]
    pub request_timeout: u64,

    /// Enable CORS.
    #[arg(long, env = "HFS_ENABLE_CORS", default_value = "true")]
    pub enable_cors: bool,

    /// Allowed CORS origins (comma-separated, or * for all).
    #[arg(long, env = "HFS_CORS_ORIGINS", default_value = "*")]
    pub cors_origins: String,

    /// Allowed CORS methods (comma-separated, or * for all).
    #[arg(
        long,
        env = "HFS_CORS_METHODS",
        default_value = "GET,POST,PUT,PATCH,DELETE,OPTIONS"
    )]
    pub cors_methods: String,

    /// Allowed CORS headers (comma-separated, or * for all).
    #[arg(
        long,
        env = "HFS_CORS_HEADERS",
        default_value = "Content-Type,Authorization,Accept,If-Match,If-None-Match,If-None-Exist,If-Modified-Since,Prefer,X-Tenant-ID,Content-Encoding"
    )]
    pub cors_headers: String,

    /// Default tenant ID for requests without X-Tenant-ID header.
    #[arg(long, env = "HFS_DEFAULT_TENANT", default_value = "default")]
    pub default_tenant: String,

    /// Base URL for the server (used in Location headers and Bundle links).
    #[arg(long, env = "HFS_BASE_URL", default_value = "http://localhost:8080")]
    pub base_url: String,

    /// Database connection string.
    #[arg(long, env = "HFS_DATABASE_URL")]
    pub database_url: Option<String>,

    /// Enable request ID tracking.
    #[arg(long, env = "HFS_ENABLE_REQUEST_ID", default_value = "true")]
    pub enable_request_id: bool,

    /// Return deleted resources with 410 Gone instead of 404 Not Found.
    #[arg(long, env = "HFS_RETURN_GONE", default_value = "true")]
    pub return_gone: bool,

    /// Enable versioning (ETag support).
    #[arg(long, env = "HFS_ENABLE_VERSIONING", default_value = "true")]
    pub enable_versioning: bool,

    /// Require If-Match header for updates.
    #[arg(long, env = "HFS_REQUIRE_IF_MATCH", default_value = "false")]
    pub require_if_match: bool,

    /// Default FHIR version for operations that need it before request parsing
    /// (e.g., tenant resolution, resource type detection).
    #[arg(
        long,
        env = "HFS_DEFAULT_FHIR_VERSION",
        value_enum,
        default_value = "R4"
    )]
    pub default_fhir_version: FhirVersion,

    /// Directory containing FHIR data files (e.g., search-parameters-r4.json).
    /// Defaults to ./data or the directory containing the executable.
    #[arg(long, env = "HFS_DATA_DIR")]
    pub data_dir: Option<PathBuf>,

    /// Default page size for search results.
    #[arg(long, env = "HFS_DEFAULT_PAGE_SIZE", default_value = "20")]
    pub default_page_size: usize,

    /// Maximum page size for search results.
    #[arg(long, env = "HFS_MAX_PAGE_SIZE", default_value = "1000")]
    pub max_page_size: usize,

    /// Storage backend mode: sqlite (default), sqlite-elasticsearch, postgres,
    /// postgres-elasticsearch, mongodb, mongodb-elasticsearch, s3, or s3-elasticsearch.
    #[arg(long, env = "HFS_STORAGE_BACKEND", default_value = "sqlite")]
    pub storage_backend: String,

    /// Elasticsearch node URLs (comma-separated).
    /// Used when storage_backend is sqlite-elasticsearch, postgres-elasticsearch,
    /// or mongodb-elasticsearch.
    #[arg(
        long,
        env = "HFS_ELASTICSEARCH_NODES",
        default_value = "http://localhost:9200"
    )]
    pub elasticsearch_nodes: String,

    /// Elasticsearch index name prefix.
    #[arg(long, env = "HFS_ELASTICSEARCH_INDEX_PREFIX", default_value = "hfs")]
    pub elasticsearch_index_prefix: String,

    /// Elasticsearch basic auth username (optional).
    #[arg(long, env = "HFS_ELASTICSEARCH_USERNAME")]
    pub elasticsearch_username: Option<String>,

    /// Elasticsearch basic auth password (optional).
    #[arg(long, env = "HFS_ELASTICSEARCH_PASSWORD")]
    pub elasticsearch_password: Option<String>,

    /// Enable SQL-on-FHIR operations ($viewdefinition-run, $viewdefinition-export).
    /// When enabled, the configured storage backend MUST provide an in-DB
    /// SOF runner (sqlite or postgres) — there is no in-process fallback.
    #[arg(long, env = "HFS_SOF_ENABLED", default_value = "true")]
    pub sof_enabled: bool,

    /// Export sink type: "fs" (default, local filesystem) or "s3" (AWS S3).
    #[arg(long, env = "HFS_EXPORT_SINK", default_value = "fs")]
    pub export_sink: String,

    /// Root directory for filesystem export sink.
    #[arg(long, env = "HFS_EXPORT_DIR", default_value = "./exports")]
    pub export_dir: String,

    /// S3 bucket name for S3 export sink.
    #[arg(long, env = "HFS_EXPORT_S3_BUCKET")]
    pub export_s3_bucket: Option<String>,

    /// S3 region for S3 export sink (defaults to AWS credential-chain region).
    #[arg(long, env = "HFS_EXPORT_S3_REGION")]
    pub export_s3_region: Option<String>,

    /// Pre-signed URL TTL (seconds) for S3 export sink.
    ///
    /// Defaults to 24 hours: the SQL-on-FHIR spec requires `output.location`
    /// download URLs to remain valid for at least 24 hours after export
    /// completion (matching the `Expires` header on the completion poll).
    #[arg(long, env = "HFS_EXPORT_PRESIGN_TTL_SECS", default_value = "86400")]
    pub export_presign_ttl_secs: u64,

    /// Maximum concurrent export jobs.
    #[arg(long, env = "HFS_EXPORT_MAX_CONCURRENCY", default_value = "4")]
    pub export_max_concurrency: usize,

    /// Target rows per output shard for `$viewdefinition-export`.
    /// Large result sets are split into multiple files of this size.
    #[arg(long, env = "HFS_EXPORT_SHARD_ROWS", default_value = "500000")]
    pub export_shard_rows: usize,

    /// Export job controller backend: "memory" (default, in-process).
    /// Future values: "kafka", "sqs".
    #[arg(long, env = "HFS_EXPORT_CONTROLLER", default_value = "memory")]
    pub export_controller: String,

    /// Retention (seconds) for a finished export job's output and bookkeeping.
    ///
    /// Defaults to 24 hours, matching the `Expires` header on the completion
    /// poll. After this period the cleanup reaper deletes the output shards and
    /// drops the job, so subsequent polls/downloads return 404.
    #[arg(long, env = "HFS_EXPORT_OUTPUT_TTL", default_value = "86400")]
    pub export_output_ttl_secs: u64,

    /// How often (seconds) the cleanup reaper scans for expired export jobs.
    #[arg(long, env = "HFS_EXPORT_CLEANUP_INTERVAL", default_value = "300")]
    pub export_cleanup_interval_secs: u64,

    /// Maximum rows returned by `$sqlquery-run`.
    #[arg(long, env = "HFS_SOF_SQLQUERY_MAX_ROWS", default_value = "100000")]
    pub sof_sqlquery_max_rows: usize,

    /// Maximum rows materialized per depends-on ViewDefinition by `$sqlquery-run`.
    #[arg(
        long,
        env = "HFS_SOF_SQLQUERY_MAX_SOURCE_ROWS_PER_VD",
        default_value = "1000000"
    )]
    pub sof_sqlquery_max_source_rows_per_vd: usize,

    /// Maximum number of depends-on ViewDefinitions a single SQLQuery Library may declare.
    #[arg(long, env = "HFS_SOF_SQLQUERY_MAX_VDS", default_value = "16")]
    pub sof_sqlquery_max_vds: usize,

    /// Hard timeout (seconds) for `$sqlquery-run` queries.
    #[arg(long, env = "HFS_SOF_SQLQUERY_TIMEOUT_SECS", default_value = "30")]
    pub sof_sqlquery_timeout_secs: u64,

    /// URL of the Helios Terminology Server (HTS) for terminology operations.
    ///
    /// When set, HFS delegates the following operations to the HTS:
    /// - FHIR search `:in` modifier  → `POST /ValueSet/$expand`
    /// - FHIR search `:not-in` modifier → `POST /ValueSet/$expand` (expansion-based)
    /// - FHIRPath `memberOf()` → `POST /ValueSet/$validate-code` (via env var passthrough)
    /// - FHIRPath `subsumes()` → `POST /CodeSystem/$subsumes` (via env var passthrough)
    ///
    /// Leave unset (default: none) to disable terminology integration.
    /// Example: `http://localhost:8090`
    #[arg(long, env = "HFS_TERMINOLOGY_SERVER")]
    pub terminology_server: Option<String>,

    /// Multitenancy configuration (loaded from environment variables).
    #[arg(skip)]
    pub multitenancy: MultitenancyConfig,

    /// Bulk data export configuration (loaded from environment variables).
    #[arg(skip)]
    pub bulk_export: BulkExportConfig,

    /// Bulk data submit configuration (loaded from environment variables).
    #[arg(skip)]
    pub bulk_submit: BulkSubmitConfig,
}

impl ServerConfig {
    /// Parses the storage backend mode from the string field.
    pub fn storage_backend_mode(&self) -> Result<StorageBackendMode, String> {
        self.storage_backend.parse()
    }
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
            cors_methods: "GET,POST,PUT,PATCH,DELETE,OPTIONS".to_string(),
            cors_headers: "Content-Type,Authorization,Accept,If-Match,If-None-Match,If-None-Exist,If-Modified-Since,Prefer,X-Tenant-ID,Content-Encoding".to_string(),
            default_tenant: "default".to_string(),
            base_url: "http://localhost:8080".to_string(),
            database_url: None,
            enable_request_id: true,
            return_gone: true,
            enable_versioning: true,
            require_if_match: false,
            default_fhir_version: FhirVersion::default_enabled(),
            data_dir: None,
            default_page_size: 20,
            max_page_size: 1000,
            storage_backend: "sqlite".to_string(),
            elasticsearch_nodes: "http://localhost:9200".to_string(),
            elasticsearch_index_prefix: "hfs".to_string(),
            elasticsearch_username: None,
            elasticsearch_password: None,
            sof_enabled: true,
            export_sink: "fs".to_string(),
            export_dir: "./exports".to_string(),
            export_s3_bucket: None,
            export_s3_region: None,
            export_presign_ttl_secs: 86_400,
            export_max_concurrency: 4,
            export_shard_rows: 500_000,
            export_controller: "memory".to_string(),
            export_output_ttl_secs: 86_400,
            export_cleanup_interval_secs: 300,
            sof_sqlquery_max_rows: 100_000,
            sof_sqlquery_max_source_rows_per_vd: 1_000_000,
            sof_sqlquery_max_vds: 16,
            sof_sqlquery_timeout_secs: 30,
            terminology_server: None,
            multitenancy: MultitenancyConfig::default(),
            bulk_export: BulkExportConfig::default(),
            bulk_submit: BulkSubmitConfig::default(),
        }
    }
}

impl ServerConfig {
    /// Creates a new ServerConfig from environment variables.
    ///
    /// This is a convenience method that parses environment variables without
    /// requiring command line arguments.
    pub fn from_env() -> Self {
        // Try to parse from environment, falling back to defaults
        let mut config = Self::try_parse().unwrap_or_default();
        // Load multitenancy config from environment
        config.multitenancy = MultitenancyConfig::from_env();
        // Load bulk export config from environment
        config.bulk_export = BulkExportConfig::from_env();
        // Load bulk submit config from environment
        config.bulk_submit = BulkSubmitConfig::from_env();
        config
    }

    /// Returns the socket address to bind to.
    pub fn socket_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    /// Returns the full base URL for the server.
    pub fn full_base_url(&self) -> &str {
        &self.base_url
    }

    /// Validates the configuration and returns errors if any.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();

        if self.port == 0 {
            errors.push("Port cannot be 0".to_string());
        }

        if self.max_body_size == 0 {
            errors.push("Max body size cannot be 0".to_string());
        }

        if self.request_timeout == 0 {
            errors.push("Request timeout cannot be 0".to_string());
        }

        if self.default_page_size == 0 {
            errors.push("Default page size cannot be 0".to_string());
        }

        if self.default_page_size > self.max_page_size {
            errors.push("Default page size cannot exceed max page size".to_string());
        }

        if let Err(mut bulk_errors) = self.bulk_export.validate() {
            errors.append(&mut bulk_errors);
        }

        if let Err(mut submit_errors) = self.bulk_submit.validate() {
            errors.append(&mut submit_errors);
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Creates a configuration suitable for testing.
    ///
    /// This uses ephemeral port 0 and disables features that might interfere
    /// with tests.
    pub fn for_testing() -> Self {
        Self {
            port: 0, // Let OS assign port
            host: "127.0.0.1".to_string(),
            log_level: "debug".to_string(),
            max_body_size: 10 * 1024 * 1024,
            request_timeout: 5, // Shorter timeout for tests
            enable_cors: false,
            cors_origins: "*".to_string(),
            cors_methods: "*".to_string(),
            cors_headers: "*".to_string(),
            default_tenant: "test-tenant".to_string(),
            base_url: "http://localhost:0".to_string(),
            database_url: None,
            enable_request_id: false,
            return_gone: true,
            enable_versioning: true,
            require_if_match: false,
            default_fhir_version: FhirVersion::default_enabled(),
            data_dir: None,
            default_page_size: 10,
            max_page_size: 100,
            storage_backend: "sqlite".to_string(),
            elasticsearch_nodes: "http://localhost:9200".to_string(),
            elasticsearch_index_prefix: "hfs".to_string(),
            elasticsearch_username: None,
            elasticsearch_password: None,
            sof_enabled: true,
            export_sink: "fs".to_string(),
            export_dir: "./exports".to_string(),
            export_s3_bucket: None,
            export_s3_region: None,
            export_presign_ttl_secs: 86_400,
            export_max_concurrency: 4,
            export_shard_rows: 500_000,
            export_controller: "memory".to_string(),
            export_output_ttl_secs: 86_400,
            export_cleanup_interval_secs: 300,
            sof_sqlquery_max_rows: 100_000,
            sof_sqlquery_max_source_rows_per_vd: 1_000_000,
            sof_sqlquery_max_vds: 16,
            sof_sqlquery_timeout_secs: 30,
            terminology_server: None,
            multitenancy: MultitenancyConfig::default(),
            bulk_export: BulkExportConfig::default(),
            bulk_submit: BulkSubmitConfig::default(),
        }
    }

    /// Returns the multitenancy configuration.
    pub fn multitenancy(&self) -> &MultitenancyConfig {
        &self.multitenancy
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = ServerConfig::default();
        assert_eq!(config.port, 8080);
        assert_eq!(config.host, "127.0.0.1");
        assert!(config.enable_cors);
    }

    #[test]
    fn test_socket_addr() {
        let config = ServerConfig {
            port: 3000,
            host: "0.0.0.0".to_string(),
            ..Default::default()
        };
        assert_eq!(config.socket_addr(), "0.0.0.0:3000");
    }

    #[test]
    fn test_validate_valid() {
        let config = ServerConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_invalid_port() {
        let config = ServerConfig {
            port: 0,
            ..Default::default()
        };
        let result = config.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().iter().any(|e| e.contains("Port")));
    }

    #[test]
    fn test_validate_invalid_page_sizes() {
        let config = ServerConfig {
            default_page_size: 100,
            max_page_size: 50,
            ..Default::default()
        };
        let result = config.validate();
        assert!(result.is_err());
    }

    #[test]
    fn test_for_testing() {
        let config = ServerConfig::for_testing();
        assert_eq!(config.port, 0);
        assert!(!config.enable_cors);
        assert_eq!(config.default_tenant, "test-tenant");
    }

    #[test]
    fn test_tenant_routing_mode_parse() {
        assert_eq!(
            "header_only".parse::<TenantRoutingMode>().unwrap(),
            TenantRoutingMode::HeaderOnly
        );
        assert_eq!(
            "url_path".parse::<TenantRoutingMode>().unwrap(),
            TenantRoutingMode::UrlPath
        );
        assert_eq!(
            "both".parse::<TenantRoutingMode>().unwrap(),
            TenantRoutingMode::Both
        );
        assert_eq!(
            "HEADER".parse::<TenantRoutingMode>().unwrap(),
            TenantRoutingMode::HeaderOnly
        );
        assert!("invalid".parse::<TenantRoutingMode>().is_err());
    }

    #[test]
    fn test_tenant_routing_mode_display() {
        assert_eq!(TenantRoutingMode::HeaderOnly.to_string(), "header_only");
        assert_eq!(TenantRoutingMode::UrlPath.to_string(), "url_path");
        assert_eq!(TenantRoutingMode::Both.to_string(), "both");
    }

    #[test]
    fn test_tenant_routing_mode_supports() {
        assert!(TenantRoutingMode::HeaderOnly.supports_header());
        assert!(!TenantRoutingMode::HeaderOnly.supports_url_path());

        assert!(!TenantRoutingMode::UrlPath.supports_header());
        assert!(TenantRoutingMode::UrlPath.supports_url_path());

        assert!(TenantRoutingMode::Both.supports_header());
        assert!(TenantRoutingMode::Both.supports_url_path());
    }

    #[test]
    fn test_storage_backend_mode_parse() {
        assert_eq!(
            "sqlite".parse::<StorageBackendMode>().unwrap(),
            StorageBackendMode::Sqlite
        );
        assert_eq!(
            "sqlite-elasticsearch"
                .parse::<StorageBackendMode>()
                .unwrap(),
            StorageBackendMode::SqliteElasticsearch
        );
        assert_eq!(
            "sqlite-es".parse::<StorageBackendMode>().unwrap(),
            StorageBackendMode::SqliteElasticsearch
        );
        assert_eq!(
            "sqlite_elasticsearch"
                .parse::<StorageBackendMode>()
                .unwrap(),
            StorageBackendMode::SqliteElasticsearch
        );
        assert_eq!(
            "postgres".parse::<StorageBackendMode>().unwrap(),
            StorageBackendMode::Postgres
        );
        assert_eq!(
            "pg".parse::<StorageBackendMode>().unwrap(),
            StorageBackendMode::Postgres
        );
        assert_eq!(
            "postgresql".parse::<StorageBackendMode>().unwrap(),
            StorageBackendMode::Postgres
        );
        assert_eq!(
            "POSTGRES".parse::<StorageBackendMode>().unwrap(),
            StorageBackendMode::Postgres
        );
        assert_eq!(
            "postgres-elasticsearch"
                .parse::<StorageBackendMode>()
                .unwrap(),
            StorageBackendMode::PostgresElasticsearch
        );
        assert_eq!(
            "postgres-es".parse::<StorageBackendMode>().unwrap(),
            StorageBackendMode::PostgresElasticsearch
        );
        assert_eq!(
            "pg-elasticsearch".parse::<StorageBackendMode>().unwrap(),
            StorageBackendMode::PostgresElasticsearch
        );
        assert_eq!(
            "pg-es".parse::<StorageBackendMode>().unwrap(),
            StorageBackendMode::PostgresElasticsearch
        );
        assert_eq!(
            "postgres_elasticsearch"
                .parse::<StorageBackendMode>()
                .unwrap(),
            StorageBackendMode::PostgresElasticsearch
        );
        assert_eq!(
            "mongodb".parse::<StorageBackendMode>().unwrap(),
            StorageBackendMode::MongoDB
        );
        assert_eq!(
            "mongo".parse::<StorageBackendMode>().unwrap(),
            StorageBackendMode::MongoDB
        );
        assert_eq!(
            "MONGODB".parse::<StorageBackendMode>().unwrap(),
            StorageBackendMode::MongoDB
        );
        assert_eq!(
            "mongodb-elasticsearch"
                .parse::<StorageBackendMode>()
                .unwrap(),
            StorageBackendMode::MongoDBElasticsearch
        );
        assert_eq!(
            "mongo-es".parse::<StorageBackendMode>().unwrap(),
            StorageBackendMode::MongoDBElasticsearch
        );
        assert_eq!(
            "mongodb_elasticsearch"
                .parse::<StorageBackendMode>()
                .unwrap(),
            StorageBackendMode::MongoDBElasticsearch
        );
        assert_eq!(
            "s3".parse::<StorageBackendMode>().unwrap(),
            StorageBackendMode::S3
        );
        assert_eq!(
            "objectstore".parse::<StorageBackendMode>().unwrap(),
            StorageBackendMode::S3
        );
        assert_eq!(
            "S3".parse::<StorageBackendMode>().unwrap(),
            StorageBackendMode::S3
        );
        assert_eq!(
            "s3-elasticsearch".parse::<StorageBackendMode>().unwrap(),
            StorageBackendMode::S3Elasticsearch
        );
        assert_eq!(
            "s3-es".parse::<StorageBackendMode>().unwrap(),
            StorageBackendMode::S3Elasticsearch
        );
        assert_eq!(
            "S3-ES".parse::<StorageBackendMode>().unwrap(),
            StorageBackendMode::S3Elasticsearch
        );
        assert!("invalid".parse::<StorageBackendMode>().is_err());
    }

    #[test]
    fn test_storage_backend_mode_display() {
        assert_eq!(StorageBackendMode::Sqlite.to_string(), "sqlite");
        assert_eq!(
            StorageBackendMode::SqliteElasticsearch.to_string(),
            "sqlite-elasticsearch"
        );
        assert_eq!(StorageBackendMode::Postgres.to_string(), "postgres");
        assert_eq!(
            StorageBackendMode::PostgresElasticsearch.to_string(),
            "postgres-elasticsearch"
        );
        assert_eq!(StorageBackendMode::MongoDB.to_string(), "mongodb");
        assert_eq!(
            StorageBackendMode::MongoDBElasticsearch.to_string(),
            "mongodb-elasticsearch"
        );
        assert_eq!(StorageBackendMode::S3.to_string(), "s3");
        assert_eq!(
            StorageBackendMode::S3Elasticsearch.to_string(),
            "s3-elasticsearch"
        );
    }

    #[test]
    fn test_storage_backend_mode_primary_backend_kind() {
        assert_eq!(
            StorageBackendMode::Sqlite.primary_backend_kind(),
            BackendKind::Sqlite
        );
        assert_eq!(
            StorageBackendMode::SqliteElasticsearch.primary_backend_kind(),
            BackendKind::Sqlite
        );
        assert_eq!(
            StorageBackendMode::Postgres.primary_backend_kind(),
            BackendKind::Postgres
        );
        assert_eq!(
            StorageBackendMode::PostgresElasticsearch.primary_backend_kind(),
            BackendKind::Postgres
        );
        assert_eq!(
            StorageBackendMode::MongoDB.primary_backend_kind(),
            BackendKind::MongoDB
        );
        assert_eq!(
            StorageBackendMode::MongoDBElasticsearch.primary_backend_kind(),
            BackendKind::MongoDB
        );
        assert_eq!(
            StorageBackendMode::S3.primary_backend_kind(),
            BackendKind::S3
        );
        assert_eq!(
            StorageBackendMode::S3Elasticsearch.primary_backend_kind(),
            BackendKind::S3
        );
    }

    #[test]
    fn test_storage_backend_mode_from_config() {
        let config = ServerConfig::default();
        assert_eq!(
            config.storage_backend_mode().unwrap(),
            StorageBackendMode::Sqlite
        );
    }

    #[test]
    fn test_multitenancy_config_default() {
        let config = MultitenancyConfig::default();
        assert_eq!(config.routing_mode, TenantRoutingMode::HeaderOnly);
        assert!(!config.strict_validation);
        assert_eq!(config.jwt_tenant_claim, "tenant_id");
    }

    // ── validate() – max_body_size == 0 ───────────────────────────

    #[test]
    fn test_validate_max_body_size_zero() {
        let config = ServerConfig {
            max_body_size: 0,
            ..Default::default()
        };
        let result = config.validate();
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| e.contains("body size")));
    }

    // ── validate() – request_timeout == 0 ────────────────────────

    #[test]
    fn test_validate_request_timeout_zero() {
        let config = ServerConfig {
            request_timeout: 0,
            ..Default::default()
        };
        let result = config.validate();
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| e.contains("timeout")));
    }

    // ── validate() – default_page_size == 0 ──────────────────────

    #[test]
    fn test_validate_default_page_size_zero() {
        let config = ServerConfig {
            default_page_size: 0,
            max_page_size: 100,
            ..Default::default()
        };
        let result = config.validate();
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| e.contains("page size")));
    }

    // ── validate() – multiple errors at once ─────────────────────

    #[test]
    fn test_validate_multiple_errors() {
        let config = ServerConfig {
            max_body_size: 0,
            request_timeout: 0,
            default_page_size: 0,
            ..Default::default()
        };
        let result = config.validate();
        assert!(result.is_err());
        let errors = result.unwrap_err();
        // At least the three errors above should be present
        assert!(errors.len() >= 3);
    }

    // ── full_base_url() ───────────────────────────────────────────

    #[test]
    fn test_full_base_url() {
        let config = ServerConfig {
            base_url: "https://fhir.example.com".to_string(),
            ..Default::default()
        };
        assert_eq!(config.full_base_url(), "https://fhir.example.com");
    }

    #[test]
    fn test_full_base_url_default() {
        let config = ServerConfig::default();
        assert_eq!(config.full_base_url(), "http://localhost:8080");
    }

    // ── multitenancy() accessor ───────────────────────────────────

    #[test]
    fn test_multitenancy_accessor() {
        let config = ServerConfig::default();
        let mt = config.multitenancy();
        assert_eq!(mt.routing_mode, TenantRoutingMode::HeaderOnly);
    }

    // ── StorageBackendMode::default() ─────────────────────────────

    #[test]
    fn test_storage_backend_mode_default() {
        let mode = StorageBackendMode::default();
        assert_eq!(mode, StorageBackendMode::Sqlite);
    }

    // ── TenantRoutingMode::default() ──────────────────────────────

    #[test]
    fn test_tenant_routing_mode_default() {
        let mode = TenantRoutingMode::default();
        assert_eq!(mode, TenantRoutingMode::HeaderOnly);
    }

    // ── Alias parsing variations ──────────────────────────────────

    #[test]
    fn test_tenant_routing_mode_aliases() {
        // headeronly / header
        assert_eq!(
            "headeronly".parse::<TenantRoutingMode>().unwrap(),
            TenantRoutingMode::HeaderOnly
        );
        assert_eq!(
            "header".parse::<TenantRoutingMode>().unwrap(),
            TenantRoutingMode::HeaderOnly
        );
        // urlpath / url / path
        assert_eq!(
            "urlpath".parse::<TenantRoutingMode>().unwrap(),
            TenantRoutingMode::UrlPath
        );
        assert_eq!(
            "url".parse::<TenantRoutingMode>().unwrap(),
            TenantRoutingMode::UrlPath
        );
        assert_eq!(
            "path".parse::<TenantRoutingMode>().unwrap(),
            TenantRoutingMode::UrlPath
        );
        // combined
        assert_eq!(
            "combined".parse::<TenantRoutingMode>().unwrap(),
            TenantRoutingMode::Both
        );
    }

    #[test]
    fn test_tenant_routing_mode_invalid() {
        assert!("unknown_mode".parse::<TenantRoutingMode>().is_err());
    }

    // ── MultitenancyConfig struct field behaviour (without env) ──

    #[test]
    fn test_multitenancy_config_strict_validation_field() {
        // Test the struct directly, avoiding env-var parallelism issues.
        let config = MultitenancyConfig {
            routing_mode: TenantRoutingMode::UrlPath,
            strict_validation: true,
            jwt_tenant_claim: "custom_claim".to_string(),
        };
        assert_eq!(config.routing_mode, TenantRoutingMode::UrlPath);
        assert!(config.strict_validation);
        assert_eq!(config.jwt_tenant_claim, "custom_claim");
    }

    #[test]
    fn test_multitenancy_config_from_env_routing_mode_parsed() {
        // Parse the routing mode value the same way from_env does, without
        // touching global env state.
        let result: Result<TenantRoutingMode, _> = "url_path".parse();
        assert_eq!(result.unwrap(), TenantRoutingMode::UrlPath);
    }

    #[test]
    fn test_multitenancy_strict_validation_string_parsing() {
        // Mirror the logic inside MultitenancyConfig::from_env.
        let parse_strict = |s: &str| -> bool { s.to_lowercase() == "true" || s == "1" };
        assert!(parse_strict("true"));
        assert!(parse_strict("TRUE"));
        assert!(parse_strict("1"));
        assert!(!parse_strict("false"));
        assert!(!parse_strict("0"));
        assert!(!parse_strict("yes"));
    }

    // ── storage_backend_mode() – invalid value ────────────────────

    #[test]
    fn test_storage_backend_mode_invalid_returns_error() {
        let config = ServerConfig {
            storage_backend: "unknown_backend".to_string(),
            ..Default::default()
        };
        assert!(config.storage_backend_mode().is_err());
    }

    // ── BulkExportConfig::validate ────────────────────────────────

    #[test]
    fn test_bulk_export_config_default_is_valid() {
        assert!(BulkExportConfig::default().validate().is_ok());
    }

    #[test]
    fn test_bulk_export_config_s3_output_requires_bucket() {
        let cfg = BulkExportConfig {
            output_backend: "s3".to_string(),
            s3_bucket: None,
            ..BulkExportConfig::default()
        };
        let errs = cfg.validate().unwrap_err();
        assert!(errs.iter().any(|e| e.contains("S3_BUCKET")));
    }

    #[test]
    fn test_bulk_export_config_local_fs_requires_access_token() {
        let cfg = BulkExportConfig {
            output_backend: "local-fs".to_string(),
            requires_access_token: "false".to_string(),
            ..BulkExportConfig::default()
        };
        let errs = cfg.validate().unwrap_err();
        assert!(errs.iter().any(|e| e.contains("local-fs")));
    }

    #[test]
    fn test_bulk_export_config_lease_must_exceed_heartbeat() {
        let cfg = BulkExportConfig {
            lease_duration_secs: 10,
            heartbeat_interval_secs: 20,
            ..BulkExportConfig::default()
        };
        let errs = cfg.validate().unwrap_err();
        assert!(errs.iter().any(|e| e.contains("LEASE_DURATION")));
    }

    #[test]
    fn test_bulk_export_config_invalid_since_newly_added() {
        let cfg = BulkExportConfig {
            since_newly_added: "maybe".to_string(),
            ..BulkExportConfig::default()
        };
        let errs = cfg.validate().unwrap_err();
        assert!(errs.iter().any(|e| e.contains("SINCE_NEWLY_ADDED")));
    }

    // ── BulkSubmitConfig::validate ────────────────────────────────

    #[test]
    fn test_bulk_submit_config_default_is_valid() {
        assert!(BulkSubmitConfig::default().validate().is_ok());
    }

    #[test]
    fn test_bulk_submit_config_invalid_output_backend() {
        let cfg = BulkSubmitConfig {
            output_backend: "gcs".to_string(),
            ..BulkSubmitConfig::default()
        };
        let errs = cfg.validate().unwrap_err();
        assert!(errs.iter().any(|e| e.contains("OUTPUT_BACKEND")));
    }

    #[test]
    fn test_bulk_submit_config_s3_output_requires_bucket() {
        let cfg = BulkSubmitConfig {
            output_backend: "s3".to_string(),
            s3_bucket: None,
            ..BulkSubmitConfig::default()
        };
        let errs = cfg.validate().unwrap_err();
        assert!(errs.iter().any(|e| e.contains("S3_BUCKET")));
    }

    #[test]
    fn test_bulk_submit_config_s3_output_with_bucket_ok() {
        let cfg = BulkSubmitConfig {
            output_backend: "s3".to_string(),
            s3_bucket: Some("my-bucket".to_string()),
            ..BulkSubmitConfig::default()
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn test_bulk_submit_config_invalid_requires_access_token() {
        let cfg = BulkSubmitConfig {
            requires_access_token: "maybe".to_string(),
            ..BulkSubmitConfig::default()
        };
        let errs = cfg.validate().unwrap_err();
        assert!(errs.iter().any(|e| e.contains("REQUIRES_ACCESS_TOKEN")));
    }

    #[test]
    fn test_bulk_submit_config_local_fs_requires_access_token_false_invalid() {
        let cfg = BulkSubmitConfig {
            output_backend: "local-fs".to_string(),
            requires_access_token: "false".to_string(),
            ..BulkSubmitConfig::default()
        };
        let errs = cfg.validate().unwrap_err();
        assert!(errs.iter().any(|e| e.contains("local-fs")));
    }

    #[test]
    fn test_bulk_submit_config_invalid_signing_alg() {
        let cfg = BulkSubmitConfig {
            signing_alg: "HS256".to_string(),
            ..BulkSubmitConfig::default()
        };
        let errs = cfg.validate().unwrap_err();
        assert!(errs.iter().any(|e| e.contains("SIGNING_ALG")));
    }

    #[test]
    fn test_bulk_submit_config_rs384_signing_alg_ok() {
        let cfg = BulkSubmitConfig {
            signing_alg: "RS384".to_string(),
            ..BulkSubmitConfig::default()
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn test_bulk_submit_config_zero_worker_concurrency() {
        let cfg = BulkSubmitConfig {
            worker_concurrency: 0,
            ..BulkSubmitConfig::default()
        };
        let errs = cfg.validate().unwrap_err();
        assert!(errs.iter().any(|e| e.contains("WORKER_CONCURRENCY")));
    }

    #[test]
    fn test_bulk_submit_config_zero_max_concurrent_per_tenant() {
        let cfg = BulkSubmitConfig {
            max_concurrent_per_tenant: 0,
            ..BulkSubmitConfig::default()
        };
        let errs = cfg.validate().unwrap_err();
        assert!(errs.iter().any(|e| e.contains("MAX_CONCURRENT_PER_TENANT")));
    }

    #[test]
    fn test_bulk_submit_config_zero_batch_size() {
        let cfg = BulkSubmitConfig {
            batch_size: 0,
            ..BulkSubmitConfig::default()
        };
        let errs = cfg.validate().unwrap_err();
        assert!(errs.iter().any(|e| e.contains("BATCH_SIZE")));
    }

    #[test]
    fn test_bulk_submit_config_zero_heartbeat_interval() {
        let cfg = BulkSubmitConfig {
            heartbeat_interval_secs: 0,
            ..BulkSubmitConfig::default()
        };
        let errs = cfg.validate().unwrap_err();
        assert!(errs.iter().any(|e| e.contains("HEARTBEAT_INTERVAL")));
    }

    #[test]
    fn test_bulk_submit_config_lease_must_exceed_heartbeat() {
        let cfg = BulkSubmitConfig {
            lease_duration_secs: 10,
            heartbeat_interval_secs: 20,
            ..BulkSubmitConfig::default()
        };
        let errs = cfg.validate().unwrap_err();
        assert!(errs.iter().any(|e| e.contains("LEASE_DURATION")));
    }

    #[test]
    fn test_bulk_submit_config_zero_cleanup_interval() {
        let cfg = BulkSubmitConfig {
            cleanup_interval_secs: 0,
            ..BulkSubmitConfig::default()
        };
        let errs = cfg.validate().unwrap_err();
        assert!(errs.iter().any(|e| e.contains("CLEANUP_INTERVAL")));
    }

    #[test]
    fn test_bulk_submit_config_collects_multiple_errors() {
        let cfg = BulkSubmitConfig {
            output_backend: "bogus".to_string(),
            signing_alg: "bogus".to_string(),
            worker_concurrency: 0,
            batch_size: 0,
            ..BulkSubmitConfig::default()
        };
        let errs = cfg.validate().unwrap_err();
        assert!(errs.len() >= 4);
    }

    #[test]
    fn test_server_config_validate_propagates_bulk_submit_errors() {
        let mut config = ServerConfig::default();
        config.bulk_submit.signing_alg = "nope".to_string();
        let errs = config.validate().unwrap_err();
        assert!(errs.iter().any(|e| e.contains("SIGNING_ALG")));
    }

    // ── display for StorageBackendMode ────────────────────────────

    #[test]
    fn test_storage_backend_mode_display_all_variants() {
        // Already partially tested, but ensure every variant round-trips
        for (variant, expected) in [
            (StorageBackendMode::Sqlite, "sqlite"),
            (
                StorageBackendMode::SqliteElasticsearch,
                "sqlite-elasticsearch",
            ),
            (StorageBackendMode::Postgres, "postgres"),
            (
                StorageBackendMode::PostgresElasticsearch,
                "postgres-elasticsearch",
            ),
            (StorageBackendMode::MongoDB, "mongodb"),
            (
                StorageBackendMode::MongoDBElasticsearch,
                "mongodb-elasticsearch",
            ),
            (StorageBackendMode::S3, "s3"),
            (StorageBackendMode::S3Elasticsearch, "s3-elasticsearch"),
        ] {
            assert_eq!(variant.to_string(), expected);
            assert_eq!(expected.parse::<StorageBackendMode>().unwrap(), variant);
        }
    }
}
