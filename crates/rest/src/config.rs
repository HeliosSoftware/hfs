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
//! | `HFS_MAX_BODY_SIZE` | 10485760 | Max request body (bytes) |
//! | `HFS_REQUEST_TIMEOUT` | 30 | Request timeout (seconds) |
//! | `HFS_ENABLE_CORS` | true | Enable CORS |
//! | `HFS_CORS_ORIGINS` | * | Allowed origins |
//! | `HFS_CORS_METHODS` | GET,POST,PUT,PATCH,DELETE,OPTIONS | Allowed methods |
//! | `HFS_CORS_HEADERS` | Content-Type,Authorization,Accept,If-Match,If-None-Match,Prefer | Allowed headers |
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
        default_value = "Content-Type,Authorization,Accept,If-Match,If-None-Match,If-None-Exist,If-Modified-Since,Prefer,X-Tenant-ID"
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

    /// URL of the Helios Terminology Service (HTS) for terminology operations.
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
            cors_headers: "Content-Type,Authorization,Accept,If-Match,If-None-Match,If-None-Exist,If-Modified-Since,Prefer,X-Tenant-ID".to_string(),
            default_tenant: "default".to_string(),
            base_url: "http://localhost:8080".to_string(),
            database_url: None,
            enable_request_id: true,
            return_gone: true,
            enable_versioning: true,
            require_if_match: false,
            default_fhir_version: FhirVersion::default(),
            data_dir: None,
            default_page_size: 20,
            max_page_size: 1000,
            storage_backend: "sqlite".to_string(),
            elasticsearch_nodes: "http://localhost:9200".to_string(),
            elasticsearch_index_prefix: "hfs".to_string(),
            elasticsearch_username: None,
            elasticsearch_password: None,
            terminology_server: None,
            multitenancy: MultitenancyConfig::default(),
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
            default_fhir_version: FhirVersion::default(),
            data_dir: None,
            default_page_size: 10,
            max_page_size: 100,
            storage_backend: "sqlite".to_string(),
            elasticsearch_nodes: "http://localhost:9200".to_string(),
            elasticsearch_index_prefix: "hfs".to_string(),
            elasticsearch_username: None,
            elasticsearch_password: None,
            terminology_server: None,
            multitenancy: MultitenancyConfig::default(),
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
