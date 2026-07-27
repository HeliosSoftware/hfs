//! Database-per-tenant tenancy: SQL/name generation only, not wired into any
//! backend (see issue #370).
//!
//! The types here generate per-tenant database names and connection strings
//! from a tenant ID. They execute nothing and connect to nothing; no backend
//! consumes them. In this design each tenant *would* have a completely separate
//! database, but that topology is not implemented anywhere. Every backend in
//! the crate instead separates tenants by a `tenant_id` discriminator within
//! shared tables (the one per-tenant *container* that exists is the S3
//! backend's `BucketPerTenant` mode, in `backends::s3`).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::tenant::TenantId;

use super::{TenantResolution, TenantResolver, TenantValidationError};

/// Configuration for database-per-tenant strategy.
///
/// # Example
///
/// ```
/// use helios_persistence::strategy::DatabasePerTenantConfig;
///
/// let config = DatabasePerTenantConfig {
///     connection_template: "postgres://user:pass@{host}/{tenant}_db".to_string(),
///     pool_per_tenant: true,
///     max_pools: Some(100),
///     ..Default::default()
/// };
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabasePerTenantConfig {
    /// Connection string template with placeholders.
    ///
    /// Supported placeholders:
    /// - `{tenant}` - The tenant ID (sanitized)
    /// - `{tenant_hash}` - Hash of tenant ID (for long IDs)
    /// - `{host}` - Database host (from host_resolver or default)
    /// - `{port}` - Database port
    /// - `{user}` - Database user
    /// - `{password}` - Database password
    #[serde(default = "default_connection_template")]
    pub connection_template: String,

    /// Whether to maintain a separate connection pool per tenant.
    ///
    /// When true, each tenant gets its own pool for better isolation.
    /// When false, connections are created on-demand (less resource usage).
    #[serde(default = "default_true")]
    pub pool_per_tenant: bool,

    /// Maximum number of tenant pools to maintain.
    ///
    /// If exceeded, least recently used pools are closed.
    /// Only applies when `pool_per_tenant` is true.
    #[serde(default)]
    pub max_pools: Option<usize>,

    /// Minimum connections per tenant pool.
    #[serde(default = "default_min_connections")]
    pub min_connections_per_pool: u32,

    /// Maximum connections per tenant pool.
    #[serde(default = "default_max_connections")]
    pub max_connections_per_pool: u32,

    /// Connection idle timeout.
    #[serde(default = "default_idle_timeout")]
    pub idle_timeout_secs: u64,

    /// Maximum length for tenant IDs in database names.
    #[serde(default = "default_max_tenant_id_length")]
    pub max_tenant_id_length: usize,

    /// Allowed characters in tenant IDs (regex pattern).
    #[serde(default = "default_tenant_id_pattern")]
    pub tenant_id_pattern: String,

    /// Whether to auto-create databases for new tenants.
    #[serde(default)]
    pub auto_create_database: bool,

    /// Database name prefix.
    #[serde(default = "default_database_prefix")]
    pub database_prefix: String,

    /// Database name suffix.
    #[serde(default)]
    pub database_suffix: String,

    /// Default host for database connections.
    #[serde(default = "default_host")]
    pub default_host: String,

    /// Default port for database connections.
    #[serde(default = "default_port")]
    pub default_port: u16,

    /// System database name for administrative operations.
    #[serde(default = "default_system_database")]
    pub system_database: String,
}

fn default_connection_template() -> String {
    "postgres://{user}:{password}@{host}:{port}/{tenant}_db".to_string()
}

fn default_true() -> bool {
    true
}

fn default_min_connections() -> u32 {
    1
}

fn default_max_connections() -> u32 {
    10
}

fn default_idle_timeout() -> u64 {
    300 // 5 minutes
}

fn default_max_tenant_id_length() -> usize {
    32
}

fn default_tenant_id_pattern() -> String {
    r"^[a-zA-Z][a-zA-Z0-9_]*$".to_string()
}

fn default_database_prefix() -> String {
    "tenant_".to_string()
}

fn default_host() -> String {
    "localhost".to_string()
}

fn default_port() -> u16 {
    5432
}

fn default_system_database() -> String {
    "helios_system".to_string()
}

impl Default for DatabasePerTenantConfig {
    fn default() -> Self {
        Self {
            connection_template: default_connection_template(),
            pool_per_tenant: true,
            max_pools: Some(100),
            min_connections_per_pool: default_min_connections(),
            max_connections_per_pool: default_max_connections(),
            idle_timeout_secs: default_idle_timeout(),
            max_tenant_id_length: default_max_tenant_id_length(),
            tenant_id_pattern: default_tenant_id_pattern(),
            auto_create_database: false,
            database_prefix: default_database_prefix(),
            database_suffix: String::new(),
            default_host: default_host(),
            default_port: default_port(),
            system_database: default_system_database(),
        }
    }
}

impl DatabasePerTenantConfig {
    /// Creates a new configuration with defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the connection template.
    pub fn with_connection_template(mut self, template: impl Into<String>) -> Self {
        self.connection_template = template.into();
        self
    }

    /// Enables auto-creation of databases.
    pub fn with_auto_create(mut self) -> Self {
        self.auto_create_database = true;
        self
    }

    /// Sets the maximum number of pools.
    pub fn with_max_pools(mut self, max: usize) -> Self {
        self.max_pools = Some(max);
        self
    }

    /// Sets the database prefix.
    pub fn with_database_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.database_prefix = prefix.into();
        self
    }

    /// Disables per-tenant pools (use on-demand connections).
    pub fn without_pool_per_tenant(mut self) -> Self {
        self.pool_per_tenant = false;
        self
    }
}

/// Database-per-tenant tenancy strategy — SQL/name generation only.
///
/// <div class="warning">
///
/// This type is not connected to any storage backend and provides no isolation
/// guarantee. It generates per-tenant database names and connection strings; it
/// opens no connections and no backend consumes it. See issue #370.
///
/// </div>
///
/// As a design, this strategy would provide isolation by maintaining separate
/// databases for each tenant — the most resource-intensive topology — but that
/// behavior is not deployed. The following describes the intended design, not
/// runtime behavior.
///
/// # Connection Management
///
/// When `pool_per_tenant` is enabled, each tenant gets a dedicated
/// connection pool. Pools are created lazily and can be evicted
/// based on LRU when `max_pools` is exceeded.
///
/// # Database Naming
///
/// Database names are generated from tenant IDs:
///
/// ```text
/// tenant_id: "acme"
/// database:  "tenant_acme_db"  (with default prefix/suffix)
/// ```
///
/// # Example
///
/// ```
/// use helios_persistence::strategy::{DatabasePerTenantConfig, DatabasePerTenantStrategy};
/// use helios_persistence::tenant::TenantId;
///
/// let config = DatabasePerTenantConfig::default();
/// let strategy = DatabasePerTenantStrategy::new(config).unwrap();
///
/// // Get connection info for a tenant
/// let connection = strategy.connection_string(
///     &TenantId::new("acme"),
///     "user",
///     "password",
/// );
/// ```
#[derive(Debug)]
pub struct DatabasePerTenantStrategy {
    config: DatabasePerTenantConfig,
    tenant_pattern: regex::Regex,
    /// Cache of tenant database info with last access time.
    pool_access_times: Arc<RwLock<HashMap<String, Instant>>>,
}

impl Clone for DatabasePerTenantStrategy {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            tenant_pattern: regex::Regex::new(&self.config.tenant_id_pattern)
                .expect("pattern was valid in original"),
            pool_access_times: Arc::clone(&self.pool_access_times),
        }
    }
}

impl DatabasePerTenantStrategy {
    /// Creates a new database-per-tenant strategy with the given configuration.
    pub fn new(config: DatabasePerTenantConfig) -> Result<Self, regex::Error> {
        let tenant_pattern = regex::Regex::new(&config.tenant_id_pattern)?;
        Ok(Self {
            config,
            tenant_pattern,
            pool_access_times: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Returns the configuration.
    pub fn config(&self) -> &DatabasePerTenantConfig {
        &self.config
    }

    /// Generates the database name for a tenant.
    pub fn database_name(&self, tenant_id: &TenantId) -> String {
        let sanitized = self.sanitize_tenant_id(tenant_id);
        format!(
            "{}{}{}",
            self.config.database_prefix, sanitized, self.config.database_suffix
        )
    }

    /// Generates a connection string for a tenant.
    pub fn connection_string(&self, tenant_id: &TenantId, user: &str, password: &str) -> String {
        self.connection_string_with_host(tenant_id, user, password, None)
    }

    /// Generates a connection string for a tenant with a specific host.
    pub fn connection_string_with_host(
        &self,
        tenant_id: &TenantId,
        user: &str,
        password: &str,
        host: Option<&str>,
    ) -> String {
        let sanitized = self.sanitize_tenant_id(tenant_id);
        let db_name = self.database_name(tenant_id);
        let host = host.unwrap_or(&self.config.default_host);

        self.config
            .connection_template
            .replace("{tenant}", &sanitized)
            .replace("{tenant_hash}", &self.hash_tenant_id(tenant_id))
            .replace("{host}", host)
            .replace("{port}", &self.config.default_port.to_string())
            .replace("{user}", user)
            .replace("{password}", password)
            .replace("{database}", &db_name)
    }

    /// Generates SQL for creating a tenant database.
    pub fn create_database_sql(&self, tenant_id: &TenantId) -> String {
        let db_name = self.database_name(tenant_id);
        format!(
            "CREATE DATABASE {} WITH ENCODING 'UTF8'",
            self.quote_identifier(&db_name)
        )
    }

    /// Generates SQL for dropping a tenant database.
    pub fn drop_database_sql(&self, tenant_id: &TenantId) -> String {
        let db_name = self.database_name(tenant_id);
        format!(
            "DROP DATABASE IF EXISTS {}",
            self.quote_identifier(&db_name)
        )
    }

    /// Generates SQL for checking if a tenant database exists.
    pub fn database_exists_sql(&self, tenant_id: &TenantId) -> String {
        let db_name = self.database_name(tenant_id);
        format!(
            "SELECT 1 FROM pg_database WHERE datname = '{}'",
            self.escape_sql_string(&db_name)
        )
    }

    /// Records access to a tenant's pool for LRU tracking.
    pub fn record_pool_access(&self, tenant_id: &TenantId) {
        let mut times = self.pool_access_times.write();
        times.insert(tenant_id.as_str().to_string(), Instant::now());
    }

    /// Returns tenants that should be evicted based on LRU.
    pub fn tenants_to_evict(&self) -> Vec<String> {
        let times = self.pool_access_times.read();
        let max_pools = self.config.max_pools.unwrap_or(usize::MAX);

        if times.len() <= max_pools {
            return Vec::new();
        }

        let mut entries: Vec<_> = times.iter().collect();
        entries.sort_by_key(|(_, time)| *time);

        let to_evict = times.len() - max_pools;
        entries
            .into_iter()
            .take(to_evict)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Removes a tenant from the access tracking.
    pub fn remove_pool_tracking(&self, tenant_id: &str) {
        let mut times = self.pool_access_times.write();
        times.remove(tenant_id);
    }

    /// Returns tenants with pools that have exceeded idle timeout.
    pub fn idle_tenants(&self) -> Vec<String> {
        let times = self.pool_access_times.read();
        let timeout = Duration::from_secs(self.config.idle_timeout_secs);
        let now = Instant::now();

        times
            .iter()
            .filter(|(_, last_access)| now.duration_since(**last_access) > timeout)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Sanitizes a tenant ID for use in database names.
    fn sanitize_tenant_id(&self, tenant_id: &TenantId) -> String {
        let id = tenant_id.as_str();

        // Replace hierarchy separators with underscores
        let sanitized = id.replace(['/', '-'], "_");

        // Truncate if too long
        if sanitized.len() > self.config.max_tenant_id_length {
            // Use hash for long IDs
            self.hash_tenant_id(tenant_id)
        } else {
            sanitized.to_lowercase()
        }
    }

    /// Generates a hash for a tenant ID.
    fn hash_tenant_id(&self, tenant_id: &TenantId) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        tenant_id.as_str().hash(&mut hasher);
        format!("t_{:016x}", hasher.finish())
    }

    /// Quotes an identifier for safe use in SQL.
    fn quote_identifier(&self, id: &str) -> String {
        format!("\"{}\"", id.replace('"', "\"\""))
    }

    /// Escapes a string for safe inclusion in SQL.
    fn escape_sql_string(&self, s: &str) -> String {
        s.replace('\'', "''")
    }
}

impl TenantResolver for DatabasePerTenantStrategy {
    fn resolve(&self, tenant_id: &TenantId) -> TenantResolution {
        self.record_pool_access(tenant_id);
        TenantResolution::Database {
            connection: self.database_name(tenant_id),
        }
    }

    fn validate(&self, tenant_id: &TenantId) -> Result<(), TenantValidationError> {
        let id = tenant_id.as_str();

        // For database names, we're more restrictive
        // Check that the base name (before hierarchy) matches pattern
        let base_name = id.split('/').next().unwrap_or(id);

        if !self.tenant_pattern.is_match(base_name) {
            return Err(TenantValidationError {
                tenant_id: id.to_string(),
                reason: format!(
                    "tenant ID does not match required pattern for database names: {}",
                    self.config.tenant_id_pattern
                ),
            });
        }

        // Check length after sanitization
        let sanitized = self.sanitize_tenant_id(tenant_id);
        if sanitized.len() > 63 {
            // PostgreSQL limit
            return Err(TenantValidationError {
                tenant_id: id.to_string(),
                reason: "sanitized tenant ID would exceed database name length limit (63 chars)"
                    .to_string(),
            });
        }

        Ok(())
    }

    fn system_tenant(&self) -> TenantResolution {
        TenantResolution::Database {
            connection: self.config.system_database.clone(),
        }
    }
}

/// Manager for database-per-tenant operations.
///
/// Provides utilities for creating, dropping, and managing tenant databases.
#[derive(Debug)]
#[allow(dead_code)]
pub struct TenantDatabaseManager {
    strategy: DatabasePerTenantStrategy,
    admin_user: String,
    admin_password: String,
}

#[allow(dead_code)]
impl TenantDatabaseManager {
    /// Creates a new database manager.
    pub fn new(
        strategy: DatabasePerTenantStrategy,
        admin_user: impl Into<String>,
        admin_password: impl Into<String>,
    ) -> Self {
        Self {
            strategy,
            admin_user: admin_user.into(),
            admin_password: admin_password.into(),
        }
    }

    /// Returns the admin connection string (connects to system database).
    pub fn admin_connection_string(&self) -> String {
        self.strategy
            .config
            .connection_template
            .replace("{tenant}", "system")
            .replace("{tenant_hash}", "system")
            .replace("{host}", &self.strategy.config.default_host)
            .replace("{port}", &self.strategy.config.default_port.to_string())
            .replace("{user}", &self.admin_user)
            .replace("{password}", &self.admin_password)
            .replace("{database}", &self.strategy.config.system_database)
    }

    /// Returns SQL statements for provisioning a new tenant.
    pub fn provision_tenant_sql(&self, tenant_id: &TenantId) -> Vec<String> {
        let db_name = self.strategy.database_name(tenant_id);
        let quoted_db = self.strategy.quote_identifier(&db_name);

        vec![
            // Create database
            format!("CREATE DATABASE {} WITH ENCODING 'UTF8'", quoted_db),
            // Note: Additional setup (tables, roles) would be done after connecting to the new DB
        ]
    }

    /// Returns SQL statements for deprovisioning a tenant.
    pub fn deprovision_tenant_sql(&self, tenant_id: &TenantId) -> Vec<String> {
        let db_name = self.strategy.database_name(tenant_id);
        let quoted_db = self.strategy.quote_identifier(&db_name);

        vec![
            // Terminate existing connections
            format!(
                "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = '{}'",
                self.strategy.escape_sql_string(&db_name)
            ),
            // Drop database
            format!("DROP DATABASE IF EXISTS {}", quoted_db),
        ]
    }

    /// Returns SQL to list all tenant databases.
    pub fn list_tenant_databases_sql(&self) -> String {
        let prefix = &self.strategy.config.database_prefix;
        format!(
            "SELECT datname FROM pg_database WHERE datname LIKE '{}%' ORDER BY datname",
            self.strategy.escape_sql_string(prefix)
        )
    }

    /// Returns SQL to get database statistics for a tenant.
    pub fn database_stats_sql(&self, tenant_id: &TenantId) -> String {
        let db_name = self.strategy.database_name(tenant_id);
        format!(
            r#"
            SELECT
                pg_database_size('{}') as size_bytes,
                (SELECT count(*) FROM pg_stat_activity WHERE datname = '{}') as active_connections
            "#,
            self.strategy.escape_sql_string(&db_name),
            self.strategy.escape_sql_string(&db_name)
        )
    }
}

/// Information about a tenant's database.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct TenantDatabaseInfo {
    /// The tenant ID.
    pub tenant_id: String,
    /// The database name.
    pub database_name: String,
    /// Size in bytes (if known).
    pub size_bytes: Option<u64>,
    /// Number of active connections (if known).
    pub active_connections: Option<u32>,
    /// Last access time (if tracked).
    pub last_access: Option<Instant>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_database_per_tenant_config_default() {
        let config = DatabasePerTenantConfig::default();
        assert!(config.pool_per_tenant);
        assert_eq!(config.max_pools, Some(100));
        assert_eq!(config.database_prefix, "tenant_");
    }

    #[test]
    fn test_database_per_tenant_config_builder() {
        let config = DatabasePerTenantConfig::new()
            .with_max_pools(50)
            .with_database_prefix("db_")
            .with_auto_create();

        assert_eq!(config.max_pools, Some(50));
        assert_eq!(config.database_prefix, "db_");
        assert!(config.auto_create_database);
    }

    #[test]
    fn test_database_per_tenant_strategy_creation() {
        let config = DatabasePerTenantConfig::default();
        let strategy = DatabasePerTenantStrategy::new(config).unwrap();
        assert_eq!(strategy.config().database_prefix, "tenant_");
    }

    #[test]
    fn test_database_name_generation() {
        let strategy = DatabasePerTenantStrategy::new(DatabasePerTenantConfig::default()).unwrap();

        let db_name = strategy.database_name(&TenantId::new("acme"));
        assert_eq!(db_name, "tenant_acme");

        // Hierarchical tenant
        let db_name = strategy.database_name(&TenantId::new("acme/research"));
        assert_eq!(db_name, "tenant_acme_research");
    }

    #[test]
    fn test_tenant_resolution() {
        let strategy = DatabasePerTenantStrategy::new(DatabasePerTenantConfig::default()).unwrap();
        let resolution = strategy.resolve(&TenantId::new("acme"));

        match resolution {
            TenantResolution::Database { connection } => {
                assert_eq!(connection, "tenant_acme");
            }
            _ => panic!("expected Database resolution"),
        }
    }

    #[test]
    fn test_tenant_validation_valid() {
        let strategy = DatabasePerTenantStrategy::new(DatabasePerTenantConfig::default()).unwrap();
        assert!(strategy.validate(&TenantId::new("acme")).is_ok());
        assert!(strategy.validate(&TenantId::new("Acme123")).is_ok());
        assert!(strategy.validate(&TenantId::new("tenant_one")).is_ok());
    }

    #[test]
    fn test_tenant_validation_invalid_pattern() {
        let strategy = DatabasePerTenantStrategy::new(DatabasePerTenantConfig::default()).unwrap();
        // Starts with number (not allowed by default pattern)
        let result = strategy.validate(&TenantId::new("123acme"));
        assert!(result.is_err());
    }

    #[test]
    fn test_connection_string_generation() {
        let config = DatabasePerTenantConfig {
            connection_template: "postgres://{user}:{password}@{host}:{port}/{tenant}_db"
                .to_string(),
            default_host: "db.example.com".to_string(),
            default_port: 5432,
            ..Default::default()
        };
        let strategy = DatabasePerTenantStrategy::new(config).unwrap();

        let conn = strategy.connection_string(&TenantId::new("acme"), "admin", "secret");
        assert!(conn.contains("admin:secret"));
        assert!(conn.contains("db.example.com:5432"));
        assert!(conn.contains("acme_db"));
    }

    #[test]
    fn test_create_database_sql() {
        let strategy = DatabasePerTenantStrategy::new(DatabasePerTenantConfig::default()).unwrap();
        let sql = strategy.create_database_sql(&TenantId::new("acme"));
        assert!(sql.contains("CREATE DATABASE"));
        assert!(sql.contains("tenant_acme"));
    }

    #[test]
    fn test_drop_database_sql() {
        let strategy = DatabasePerTenantStrategy::new(DatabasePerTenantConfig::default()).unwrap();
        let sql = strategy.drop_database_sql(&TenantId::new("acme"));
        assert!(sql.contains("DROP DATABASE IF EXISTS"));
        assert!(sql.contains("tenant_acme"));
    }

    #[test]
    fn test_system_tenant_resolution() {
        let strategy = DatabasePerTenantStrategy::new(DatabasePerTenantConfig::default()).unwrap();
        let resolution = strategy.system_tenant();

        match resolution {
            TenantResolution::Database { connection } => {
                assert_eq!(connection, "helios_system");
            }
            _ => panic!("expected Database resolution"),
        }
    }

    #[test]
    fn test_pool_access_tracking() {
        let strategy = DatabasePerTenantStrategy::new(DatabasePerTenantConfig::default()).unwrap();

        strategy.record_pool_access(&TenantId::new("tenant1"));
        strategy.record_pool_access(&TenantId::new("tenant2"));

        // Should have tracked both
        let times = strategy.pool_access_times.read();
        assert!(times.contains_key("tenant1"));
        assert!(times.contains_key("tenant2"));
    }

    #[test]
    fn test_tenants_to_evict() {
        let config = DatabasePerTenantConfig {
            max_pools: Some(2),
            ..Default::default()
        };
        let strategy = DatabasePerTenantStrategy::new(config).unwrap();

        // Add 3 tenants (exceeds max of 2)
        strategy.record_pool_access(&TenantId::new("tenant1"));
        std::thread::sleep(std::time::Duration::from_millis(10));
        strategy.record_pool_access(&TenantId::new("tenant2"));
        std::thread::sleep(std::time::Duration::from_millis(10));
        strategy.record_pool_access(&TenantId::new("tenant3"));

        let to_evict = strategy.tenants_to_evict();
        assert_eq!(to_evict.len(), 1);
        assert_eq!(to_evict[0], "tenant1"); // Oldest should be evicted
    }

    #[test]
    fn test_tenant_database_manager() {
        let strategy = DatabasePerTenantStrategy::new(DatabasePerTenantConfig::default()).unwrap();
        let manager = TenantDatabaseManager::new(strategy, "admin", "password");

        let provision_sql = manager.provision_tenant_sql(&TenantId::new("newcorp"));
        assert!(!provision_sql.is_empty());
        assert!(provision_sql[0].contains("CREATE DATABASE"));

        let deprovision_sql = manager.deprovision_tenant_sql(&TenantId::new("oldcorp"));
        assert!(deprovision_sql.len() >= 2);
        assert!(deprovision_sql.iter().any(|s| s.contains("DROP DATABASE")));
    }

    #[test]
    fn test_long_tenant_id_hashing() {
        let config = DatabasePerTenantConfig {
            max_tenant_id_length: 10,
            ..Default::default()
        };
        let strategy = DatabasePerTenantStrategy::new(config).unwrap();

        // Long tenant ID should be hashed
        let long_id = TenantId::new("this_is_a_very_long_tenant_identifier");
        let db_name = strategy.database_name(&long_id);

        // Should use hash format
        assert!(db_name.starts_with("tenant_t_"));
        assert!(db_name.len() <= 64); // PostgreSQL limit
    }
}
