//! Shared schema tenancy strategy.
//!
//! In this topology, all tenants share the same database tables with a
//! `tenant_id` column used to filter data. This is the topology every backend
//! in the crate actually implements.
//!
//! Note, however, that the type in *this* module is a SQL-text generator that
//! no backend calls — backends apply the `tenant_id` discriminator directly, in
//! their own query code, not through this generator. Do not mistake this type
//! for the live code path (see issue #370).

use serde::{Deserialize, Serialize};

use crate::tenant::TenantId;

use super::{TenantResolution, TenantResolver, TenantValidationError};

/// Configuration for shared schema tenancy.
///
/// # Example
///
/// ```
/// use helios_persistence::strategy::SharedSchemaConfig;
///
/// let config = SharedSchemaConfig {
///     use_row_level_security: true,
///     tenant_column: "tenant_id".to_string(),
///     index_tenant_first: true,
///     ..Default::default()
/// };
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedSchemaConfig {
    /// Whether to use Row-Level Security (PostgreSQL only).
    ///
    /// When enabled, the database enforces tenant isolation via RLS policies,
    /// providing an additional layer of protection against application bugs.
    #[serde(default)]
    pub use_row_level_security: bool,

    /// The name of the tenant ID column in tables.
    #[serde(default = "default_tenant_column")]
    pub tenant_column: String,

    /// Whether to put tenant_id first in composite indexes.
    ///
    /// When true (recommended), indexes are created as (tenant_id, ...)
    /// which improves query performance for tenant-filtered queries.
    #[serde(default = "default_true")]
    pub index_tenant_first: bool,

    /// Maximum length for tenant IDs.
    #[serde(default = "default_max_tenant_id_length")]
    pub max_tenant_id_length: usize,

    /// Allowed characters in tenant IDs (regex pattern).
    #[serde(default = "default_tenant_id_pattern")]
    pub tenant_id_pattern: String,

    /// Whether to hash long tenant IDs.
    ///
    /// If a tenant ID exceeds `max_tenant_id_length`, it will be hashed
    /// to a shorter value. The mapping is stored for reverse lookup.
    #[serde(default)]
    pub hash_long_ids: bool,
}

fn default_tenant_column() -> String {
    "tenant_id".to_string()
}

fn default_true() -> bool {
    true
}

fn default_max_tenant_id_length() -> usize {
    64
}

fn default_tenant_id_pattern() -> String {
    r"^[a-zA-Z0-9_\-/]+$".to_string()
}

impl Default for SharedSchemaConfig {
    fn default() -> Self {
        Self {
            use_row_level_security: false,
            tenant_column: default_tenant_column(),
            index_tenant_first: true,
            max_tenant_id_length: default_max_tenant_id_length(),
            tenant_id_pattern: default_tenant_id_pattern(),
            hash_long_ids: false,
        }
    }
}

impl SharedSchemaConfig {
    /// Creates a new configuration with defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Enables Row-Level Security.
    pub fn with_rls(mut self) -> Self {
        self.use_row_level_security = true;
        self
    }

    /// Sets the tenant column name.
    pub fn with_tenant_column(mut self, column: impl Into<String>) -> Self {
        self.tenant_column = column.into();
        self
    }
}

/// Shared schema tenancy strategy implementation.
///
/// This strategy uses a single database schema with a `tenant_id` column
/// on all tables to isolate tenant data.
///
/// # Query Modification
///
/// All queries are modified to include a tenant filter:
///
/// ```sql
/// -- Original query
/// SELECT * FROM patient WHERE id = '123';
///
/// -- Modified query
/// SELECT * FROM patient WHERE tenant_id = 'acme' AND id = '123';
/// ```
///
/// # Index Strategy
///
/// For optimal performance, indexes should have `tenant_id` as the leading column:
///
/// ```sql
/// CREATE INDEX idx_patient_tenant_id ON patient (tenant_id, id);
/// CREATE INDEX idx_patient_tenant_name ON patient (tenant_id, family_name, given_name);
/// ```
///
/// # Row-Level Security (PostgreSQL)
///
/// When RLS is enabled, additional protection is provided at the database level:
///
/// ```sql
/// -- Enable RLS on table
/// ALTER TABLE patient ENABLE ROW LEVEL SECURITY;
///
/// -- Create policy
/// CREATE POLICY tenant_isolation ON patient
///     USING (tenant_id = current_setting('app.current_tenant'));
/// ```
#[derive(Debug, Clone)]
pub struct SharedSchemaStrategy {
    config: SharedSchemaConfig,
    tenant_pattern: regex::Regex,
}

impl SharedSchemaStrategy {
    /// Creates a new shared schema strategy with the given configuration.
    pub fn new(config: SharedSchemaConfig) -> Result<Self, regex::Error> {
        let tenant_pattern = regex::Regex::new(&config.tenant_id_pattern)?;
        Ok(Self {
            config,
            tenant_pattern,
        })
    }

    /// Returns the configuration.
    pub fn config(&self) -> &SharedSchemaConfig {
        &self.config
    }

    /// Returns the tenant column name.
    pub fn tenant_column(&self) -> &str {
        &self.config.tenant_column
    }

    /// Returns whether RLS is enabled.
    pub fn uses_rls(&self) -> bool {
        self.config.use_row_level_security
    }

    /// Generates SQL for setting the current tenant (for RLS).
    ///
    /// This should be executed at the beginning of each request/transaction.
    pub fn set_tenant_sql(&self, tenant_id: &TenantId) -> String {
        format!(
            "SET LOCAL app.current_tenant = '{}'",
            self.escape_sql_string(tenant_id.as_str())
        )
    }

    /// Generates SQL for clearing the current tenant.
    pub fn clear_tenant_sql(&self) -> String {
        "RESET app.current_tenant".to_string()
    }

    /// Generates a WHERE clause fragment for tenant filtering.
    pub fn tenant_filter_sql(&self, table_alias: Option<&str>) -> String {
        match table_alias {
            Some(alias) => format!("{}.{} = $tenant_id", alias, self.config.tenant_column),
            None => format!("{} = $tenant_id", self.config.tenant_column),
        }
    }

    /// Escapes a string for safe inclusion in SQL.
    fn escape_sql_string(&self, s: &str) -> String {
        s.replace('\'', "''")
    }

    /// Normalizes a tenant ID (handles hashing if needed).
    fn normalize_tenant_id(&self, tenant_id: &TenantId) -> String {
        let id = tenant_id.as_str();

        if self.config.hash_long_ids && id.len() > self.config.max_tenant_id_length {
            // Use a simple hash for long IDs
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};

            let mut hasher = DefaultHasher::new();
            id.hash(&mut hasher);
            format!("h_{:016x}", hasher.finish())
        } else {
            id.to_string()
        }
    }
}

impl TenantResolver for SharedSchemaStrategy {
    fn resolve(&self, tenant_id: &TenantId) -> TenantResolution {
        TenantResolution::SharedSchema {
            tenant_id: self.normalize_tenant_id(tenant_id),
        }
    }

    fn validate(&self, tenant_id: &TenantId) -> Result<(), TenantValidationError> {
        let id = tenant_id.as_str();

        // Check length
        if !self.config.hash_long_ids && id.len() > self.config.max_tenant_id_length {
            return Err(TenantValidationError {
                tenant_id: id.to_string(),
                reason: format!(
                    "tenant ID exceeds maximum length of {} characters",
                    self.config.max_tenant_id_length
                ),
            });
        }

        // Check pattern
        if !self.tenant_pattern.is_match(id) {
            return Err(TenantValidationError {
                tenant_id: id.to_string(),
                reason: format!(
                    "tenant ID does not match required pattern: {}",
                    self.config.tenant_id_pattern
                ),
            });
        }

        Ok(())
    }

    fn system_tenant(&self) -> TenantResolution {
        TenantResolution::SharedSchema {
            tenant_id: crate::tenant::SYSTEM_TENANT.to_string(),
        }
    }
}

/// Builder for creating table DDL with tenant support.
#[derive(Debug)]
#[allow(dead_code)]
pub struct TenantAwareTableBuilder {
    table_name: String,
    tenant_column: String,
    columns: Vec<ColumnDef>,
    indexes: Vec<IndexDef>,
    use_rls: bool,
}

#[derive(Debug)]
#[allow(dead_code)]
struct ColumnDef {
    name: String,
    data_type: String,
    nullable: bool,
}

#[derive(Debug)]
#[allow(dead_code)]
struct IndexDef {
    name: String,
    columns: Vec<String>,
    unique: bool,
}

#[allow(dead_code)]
impl TenantAwareTableBuilder {
    /// Creates a new table builder.
    pub fn new(table_name: impl Into<String>, config: &SharedSchemaConfig) -> Self {
        Self {
            table_name: table_name.into(),
            tenant_column: config.tenant_column.clone(),
            columns: Vec::new(),
            indexes: Vec::new(),
            use_rls: config.use_row_level_security,
        }
    }

    /// Adds a column to the table.
    pub fn column(
        mut self,
        name: impl Into<String>,
        data_type: impl Into<String>,
        nullable: bool,
    ) -> Self {
        self.columns.push(ColumnDef {
            name: name.into(),
            data_type: data_type.into(),
            nullable,
        });
        self
    }

    /// Adds an index (tenant_id will be prepended automatically).
    pub fn index(mut self, name: impl Into<String>, columns: Vec<&str>, unique: bool) -> Self {
        self.indexes.push(IndexDef {
            name: name.into(),
            columns: columns.into_iter().map(String::from).collect(),
            unique,
        });
        self
    }

    /// Generates PostgreSQL DDL for the table.
    pub fn to_postgres_ddl(&self) -> String {
        let mut ddl = String::new();

        // CREATE TABLE
        ddl.push_str(&format!(
            "CREATE TABLE IF NOT EXISTS {} (\n",
            self.table_name
        ));
        ddl.push_str(&format!(
            "    {} VARCHAR(64) NOT NULL,\n",
            self.tenant_column
        ));

        for col in &self.columns {
            let null_str = if col.nullable { "" } else { " NOT NULL" };
            ddl.push_str(&format!(
                "    {} {}{},\n",
                col.name, col.data_type, null_str
            ));
        }

        // Remove trailing comma and close
        ddl.truncate(ddl.len() - 2);
        ddl.push_str("\n);\n\n");

        // CREATE INDEXES (with tenant_id first)
        for idx in &self.indexes {
            let unique_str = if idx.unique { "UNIQUE " } else { "" };
            let columns: Vec<_> = std::iter::once(self.tenant_column.as_str())
                .chain(idx.columns.iter().map(|s| s.as_str()))
                .collect();
            ddl.push_str(&format!(
                "CREATE {}INDEX IF NOT EXISTS {} ON {} ({});\n",
                unique_str,
                idx.name,
                self.table_name,
                columns.join(", ")
            ));
        }

        // RLS if enabled
        if self.use_rls {
            ddl.push_str(&format!(
                "\nALTER TABLE {} ENABLE ROW LEVEL SECURITY;\n",
                self.table_name
            ));
            ddl.push_str(&format!(
                "CREATE POLICY tenant_isolation ON {} USING ({} = current_setting('app.current_tenant'));\n",
                self.table_name, self.tenant_column
            ));
        }

        ddl
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shared_schema_config_default() {
        let config = SharedSchemaConfig::default();
        assert_eq!(config.tenant_column, "tenant_id");
        assert!(!config.use_row_level_security);
        assert!(config.index_tenant_first);
    }

    #[test]
    fn test_shared_schema_config_builder() {
        let config = SharedSchemaConfig::new()
            .with_rls()
            .with_tenant_column("org_id");

        assert!(config.use_row_level_security);
        assert_eq!(config.tenant_column, "org_id");
    }

    #[test]
    fn test_shared_schema_strategy_creation() {
        let config = SharedSchemaConfig::default();
        let strategy = SharedSchemaStrategy::new(config).unwrap();
        assert_eq!(strategy.tenant_column(), "tenant_id");
    }

    #[test]
    fn test_tenant_resolution() {
        let strategy = SharedSchemaStrategy::new(SharedSchemaConfig::default()).unwrap();
        let resolution = strategy.resolve(&TenantId::new("acme"));

        match resolution {
            TenantResolution::SharedSchema { tenant_id } => {
                assert_eq!(tenant_id, "acme");
            }
            _ => panic!("expected SharedSchema resolution"),
        }
    }

    #[test]
    fn test_tenant_validation_valid() {
        let strategy = SharedSchemaStrategy::new(SharedSchemaConfig::default()).unwrap();
        assert!(strategy.validate(&TenantId::new("acme")).is_ok());
        assert!(strategy.validate(&TenantId::new("acme/research")).is_ok());
        assert!(strategy.validate(&TenantId::new("tenant_123")).is_ok());
    }

    #[test]
    fn test_tenant_validation_invalid_pattern() {
        let strategy = SharedSchemaStrategy::new(SharedSchemaConfig::default()).unwrap();
        let result = strategy.validate(&TenantId::new("tenant with spaces"));
        assert!(result.is_err());
    }

    #[test]
    fn test_tenant_validation_too_long() {
        let config = SharedSchemaConfig {
            max_tenant_id_length: 10,
            ..Default::default()
        };
        let strategy = SharedSchemaStrategy::new(config).unwrap();
        let result = strategy.validate(&TenantId::new("this-is-a-very-long-tenant-id"));
        assert!(result.is_err());
    }

    #[test]
    fn test_set_tenant_sql() {
        let strategy = SharedSchemaStrategy::new(SharedSchemaConfig::default()).unwrap();
        let sql = strategy.set_tenant_sql(&TenantId::new("acme"));
        assert_eq!(sql, "SET LOCAL app.current_tenant = 'acme'");
    }

    #[test]
    fn test_set_tenant_sql_escapes() {
        let strategy = SharedSchemaStrategy::new(SharedSchemaConfig::default()).unwrap();
        let sql = strategy.set_tenant_sql(&TenantId::new("o'brien"));
        assert_eq!(sql, "SET LOCAL app.current_tenant = 'o''brien'");
    }

    #[test]
    fn test_tenant_filter_sql() {
        let strategy = SharedSchemaStrategy::new(SharedSchemaConfig::default()).unwrap();

        let filter = strategy.tenant_filter_sql(None);
        assert_eq!(filter, "tenant_id = $tenant_id");

        let filter_aliased = strategy.tenant_filter_sql(Some("p"));
        assert_eq!(filter_aliased, "p.tenant_id = $tenant_id");
    }

    #[test]
    fn test_table_builder() {
        let config = SharedSchemaConfig::default();
        let ddl = TenantAwareTableBuilder::new("patient", &config)
            .column("id", "VARCHAR(64)", false)
            .column("family_name", "TEXT", true)
            .index("idx_patient_id", vec!["id"], true)
            .to_postgres_ddl();

        assert!(ddl.contains("CREATE TABLE IF NOT EXISTS patient"));
        assert!(ddl.contains("tenant_id VARCHAR(64) NOT NULL"));
        assert!(ddl.contains("id VARCHAR(64) NOT NULL"));
        assert!(ddl.contains("CREATE UNIQUE INDEX"));
        assert!(ddl.contains("(tenant_id, id)"));
    }

    #[test]
    fn test_table_builder_with_rls() {
        let config = SharedSchemaConfig::new().with_rls();
        let ddl = TenantAwareTableBuilder::new("patient", &config)
            .column("id", "VARCHAR(64)", false)
            .to_postgres_ddl();

        assert!(ddl.contains("ENABLE ROW LEVEL SECURITY"));
        assert!(ddl.contains("CREATE POLICY tenant_isolation"));
    }

    #[test]
    fn test_system_tenant_resolution() {
        let strategy = SharedSchemaStrategy::new(SharedSchemaConfig::default()).unwrap();
        let resolution = strategy.system_tenant();

        match resolution {
            TenantResolution::SharedSchema { tenant_id } => {
                assert_eq!(tenant_id, crate::tenant::SYSTEM_TENANT);
            }
            _ => panic!("expected SharedSchema resolution"),
        }
    }
}
