//! Schema-per-tenant tenancy: SQL/name generation only, not wired into any
//! backend (see issue #370).
//!
//! The types here generate PostgreSQL schema names and `search_path` SQL from a
//! tenant ID. They execute nothing and connect to nothing; no backend consumes
//! them. In this design each tenant *would* have a separate PostgreSQL schema,
//! but that topology is not implemented anywhere. Every backend in the crate
//! instead separates tenants by a `tenant_id` discriminator within shared
//! tables.

use serde::{Deserialize, Serialize};

use crate::tenant::TenantId;

use super::{TenantResolution, TenantResolver, TenantValidationError};

/// Configuration for schema-per-tenant tenancy.
///
/// # Example
///
/// ```
/// use helios_persistence::strategy::SchemaPerTenantConfig;
///
/// let config = SchemaPerTenantConfig {
///     schema_prefix: "tenant_".to_string(),
///     shared_schema: "shared".to_string(),
///     auto_create_schema: true,
///     ..Default::default()
/// };
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaPerTenantConfig {
    /// Prefix for tenant schema names.
    ///
    /// The full schema name is `{prefix}{tenant_id}`.
    #[serde(default = "default_schema_prefix")]
    pub schema_prefix: String,

    /// Name of the shared schema for system resources.
    #[serde(default = "default_shared_schema")]
    pub shared_schema: String,

    /// Whether to automatically create schemas for new tenants.
    #[serde(default = "default_true")]
    pub auto_create_schema: bool,

    /// Whether to use the public schema for the system tenant.
    #[serde(default = "default_true")]
    pub system_uses_public: bool,

    /// Maximum schema name length (PostgreSQL limit is 63).
    #[serde(default = "default_max_schema_length")]
    pub max_schema_length: usize,

    /// Characters allowed in schema names (derived from tenant IDs).
    #[serde(default = "default_schema_pattern")]
    pub schema_pattern: String,

    /// Whether to drop schema on tenant deletion.
    #[serde(default)]
    pub drop_on_delete: bool,

    /// Template schema to copy when creating new tenants.
    ///
    /// If set, new tenant schemas are created by copying this template.
    pub template_schema: Option<String>,
}

fn default_schema_prefix() -> String {
    "tenant_".to_string()
}

fn default_shared_schema() -> String {
    "shared".to_string()
}

fn default_true() -> bool {
    true
}

fn default_max_schema_length() -> usize {
    63 // PostgreSQL identifier limit
}

fn default_schema_pattern() -> String {
    r"^[a-z][a-z0-9_]*$".to_string()
}

impl Default for SchemaPerTenantConfig {
    fn default() -> Self {
        Self {
            schema_prefix: default_schema_prefix(),
            shared_schema: default_shared_schema(),
            auto_create_schema: true,
            system_uses_public: true,
            max_schema_length: default_max_schema_length(),
            schema_pattern: default_schema_pattern(),
            drop_on_delete: false,
            template_schema: None,
        }
    }
}

impl SchemaPerTenantConfig {
    /// Creates a new configuration with defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the schema prefix.
    pub fn with_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.schema_prefix = prefix.into();
        self
    }

    /// Sets the shared schema name.
    pub fn with_shared_schema(mut self, schema: impl Into<String>) -> Self {
        self.shared_schema = schema.into();
        self
    }

    /// Sets the template schema.
    pub fn with_template(mut self, template: impl Into<String>) -> Self {
        self.template_schema = Some(template.into());
        self
    }

    /// Enables dropping schemas on tenant deletion.
    pub fn with_drop_on_delete(mut self) -> Self {
        self.drop_on_delete = true;
        self
    }
}

/// Schema-per-tenant tenancy strategy — SQL/name generation only.
///
/// <div class="warning">
///
/// This type is not connected to any storage backend and provides no isolation
/// guarantee. It generates schema names and `search_path` SQL; it opens no
/// connections and no backend consumes it. See issue #370.
///
/// </div>
///
/// As a design, this strategy would use PostgreSQL schemas to isolate tenant
/// data — each tenant with its own schema, the connection's `search_path` set
/// to include it — but that behavior is not deployed. The following describes
/// the intended design, not runtime behavior.
///
/// # Schema Naming
///
/// Tenant IDs are converted to valid PostgreSQL schema names:
/// - Prefixed with the configured prefix (default: `tenant_`)
/// - Converted to lowercase
/// - Hierarchical separators (`/`) replaced with underscores
/// - Invalid characters removed
///
/// # Search Path
///
/// For each request, the search_path is set to:
/// ```sql
/// SET search_path TO tenant_acme, shared, public;
/// ```
///
/// This allows:
/// - Tenant-specific tables in the tenant schema
/// - Shared resources (CodeSystems, etc.) in the shared schema
/// - Extension functions in public
///
/// # Schema Creation
///
/// Schemas can be created:
/// - Automatically on first access (if `auto_create_schema` is true)
/// - From a template schema (copying structure)
/// - Manually via migrations
#[derive(Debug, Clone)]
pub struct SchemaPerTenantStrategy {
    config: SchemaPerTenantConfig,
    schema_pattern: regex::Regex,
}

impl SchemaPerTenantStrategy {
    /// Creates a new schema-per-tenant strategy with the given configuration.
    pub fn new(config: SchemaPerTenantConfig) -> Result<Self, regex::Error> {
        let schema_pattern = regex::Regex::new(&config.schema_pattern)?;
        Ok(Self {
            config,
            schema_pattern,
        })
    }

    /// Returns the configuration.
    pub fn config(&self) -> &SchemaPerTenantConfig {
        &self.config
    }

    /// Returns the shared schema name.
    pub fn shared_schema(&self) -> &str {
        &self.config.shared_schema
    }

    /// Converts a tenant ID to a schema name.
    pub fn tenant_to_schema(&self, tenant_id: &TenantId) -> String {
        let normalized = self.normalize_tenant_id(tenant_id.as_str());
        format!("{}{}", self.config.schema_prefix, normalized)
    }

    /// Normalizes a tenant ID to a valid schema name component.
    fn normalize_tenant_id(&self, id: &str) -> String {
        id.to_lowercase()
            .replace(['/', '-'], "_")
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect()
    }

    /// Generates SQL to set the search_path for a tenant.
    pub fn set_search_path_sql(&self, tenant_id: &TenantId) -> String {
        let schema = self.tenant_to_schema(tenant_id);
        format!(
            "SET search_path TO {}, {}, public",
            self.escape_identifier(&schema),
            self.escape_identifier(&self.config.shared_schema)
        )
    }

    /// Generates SQL to set the search_path for the system tenant.
    pub fn set_system_search_path_sql(&self) -> String {
        if self.config.system_uses_public {
            format!(
                "SET search_path TO {}, public",
                self.escape_identifier(&self.config.shared_schema)
            )
        } else {
            format!(
                "SET search_path TO {}",
                self.escape_identifier(&self.config.shared_schema)
            )
        }
    }

    /// Generates SQL to reset the search_path.
    pub fn reset_search_path_sql(&self) -> String {
        "RESET search_path".to_string()
    }

    /// Generates SQL to create a schema for a tenant.
    pub fn create_schema_sql(&self, tenant_id: &TenantId) -> String {
        let schema = self.tenant_to_schema(tenant_id);

        if let Some(ref template) = self.config.template_schema {
            // Create from template (PostgreSQL 15+)
            format!(
                "CREATE SCHEMA IF NOT EXISTS {} TEMPLATE {}",
                self.escape_identifier(&schema),
                self.escape_identifier(template)
            )
        } else {
            format!(
                "CREATE SCHEMA IF NOT EXISTS {}",
                self.escape_identifier(&schema)
            )
        }
    }

    /// Generates SQL to drop a schema for a tenant.
    pub fn drop_schema_sql(&self, tenant_id: &TenantId, cascade: bool) -> String {
        let schema = self.tenant_to_schema(tenant_id);
        let cascade_str = if cascade { " CASCADE" } else { "" };
        format!(
            "DROP SCHEMA IF EXISTS {}{}",
            self.escape_identifier(&schema),
            cascade_str
        )
    }

    /// Generates SQL to check if a schema exists.
    pub fn schema_exists_sql(&self, tenant_id: &TenantId) -> String {
        let schema = self.tenant_to_schema(tenant_id);
        format!(
            "SELECT EXISTS(SELECT 1 FROM information_schema.schemata WHERE schema_name = '{}')",
            self.escape_sql_string(&schema)
        )
    }

    /// Generates SQL to list all tenant schemas.
    pub fn list_tenant_schemas_sql(&self) -> String {
        format!(
            "SELECT schema_name FROM information_schema.schemata WHERE schema_name LIKE '{}%' ORDER BY schema_name",
            self.escape_sql_string(&self.config.schema_prefix)
        )
    }

    /// Escapes a SQL identifier (schema name, table name, etc.).
    fn escape_identifier(&self, id: &str) -> String {
        format!("\"{}\"", id.replace('"', "\"\""))
    }

    /// Escapes a string for safe inclusion in SQL.
    fn escape_sql_string(&self, s: &str) -> String {
        s.replace('\'', "''")
    }

    /// Validates that a schema name is valid.
    fn validate_schema_name(&self, schema: &str) -> Result<(), TenantValidationError> {
        if schema.len() > self.config.max_schema_length {
            return Err(TenantValidationError {
                tenant_id: schema.to_string(),
                reason: format!(
                    "schema name exceeds maximum length of {} characters",
                    self.config.max_schema_length
                ),
            });
        }

        if !self.schema_pattern.is_match(schema) {
            return Err(TenantValidationError {
                tenant_id: schema.to_string(),
                reason: format!(
                    "schema name does not match required pattern: {}",
                    self.config.schema_pattern
                ),
            });
        }

        Ok(())
    }
}

impl TenantResolver for SchemaPerTenantStrategy {
    fn resolve(&self, tenant_id: &TenantId) -> TenantResolution {
        TenantResolution::Schema {
            schema_name: self.tenant_to_schema(tenant_id),
        }
    }

    fn validate(&self, tenant_id: &TenantId) -> Result<(), TenantValidationError> {
        let schema = self.tenant_to_schema(tenant_id);
        self.validate_schema_name(&schema)
    }

    fn system_tenant(&self) -> TenantResolution {
        TenantResolution::Schema {
            schema_name: self.config.shared_schema.clone(),
        }
    }
}

/// Manages schema lifecycle operations.
#[derive(Debug)]
#[allow(dead_code)]
pub struct SchemaManager<'a> {
    strategy: &'a SchemaPerTenantStrategy,
}

#[allow(dead_code)]
impl<'a> SchemaManager<'a> {
    /// Creates a new schema manager.
    pub fn new(strategy: &'a SchemaPerTenantStrategy) -> Self {
        Self { strategy }
    }

    /// Generates DDL to create the shared schema.
    pub fn create_shared_schema_ddl(&self) -> String {
        format!(
            "CREATE SCHEMA IF NOT EXISTS {}",
            self.strategy
                .escape_identifier(&self.strategy.config.shared_schema)
        )
    }

    /// Generates DDL to create a table in a specific schema.
    #[allow(dead_code)]
    pub fn create_table_ddl(&self, schema: &str, table_ddl: &str) -> String {
        // Prepend SET search_path to ensure table is created in correct schema
        format!(
            "SET search_path TO {};\n{}",
            self.strategy.escape_identifier(schema),
            table_ddl
        )
    }

    /// Generates SQL to migrate all tenant schemas.
    ///
    /// This creates a DO block that applies the migration to each tenant schema.
    #[allow(dead_code)]
    pub fn migrate_all_schemas_sql(&self, migration_sql: &str) -> String {
        format!(
            r#"
DO $$
DECLARE
    schema_name TEXT;
BEGIN
    FOR schema_name IN
        SELECT s.schema_name
        FROM information_schema.schemata s
        WHERE s.schema_name LIKE '{}%'
    LOOP
        EXECUTE format('SET search_path TO %I', schema_name);
        {}
    END LOOP;
END $$;
"#,
            self.strategy
                .escape_sql_string(&self.strategy.config.schema_prefix),
            migration_sql.replace('\'', "''")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema_per_tenant_config_default() {
        let config = SchemaPerTenantConfig::default();
        assert_eq!(config.schema_prefix, "tenant_");
        assert_eq!(config.shared_schema, "shared");
        assert!(config.auto_create_schema);
    }

    #[test]
    fn test_schema_per_tenant_config_builder() {
        let config = SchemaPerTenantConfig::new()
            .with_prefix("org_")
            .with_shared_schema("common")
            .with_template("template_tenant")
            .with_drop_on_delete();

        assert_eq!(config.schema_prefix, "org_");
        assert_eq!(config.shared_schema, "common");
        assert_eq!(config.template_schema, Some("template_tenant".to_string()));
        assert!(config.drop_on_delete);
    }

    #[test]
    fn test_tenant_to_schema() {
        let strategy = SchemaPerTenantStrategy::new(SchemaPerTenantConfig::default()).unwrap();

        assert_eq!(
            strategy.tenant_to_schema(&TenantId::new("acme")),
            "tenant_acme"
        );
        assert_eq!(
            strategy.tenant_to_schema(&TenantId::new("Acme-Corp")),
            "tenant_acme_corp"
        );
        assert_eq!(
            strategy.tenant_to_schema(&TenantId::new("acme/research")),
            "tenant_acme_research"
        );
    }

    #[test]
    fn test_tenant_resolution() {
        let strategy = SchemaPerTenantStrategy::new(SchemaPerTenantConfig::default()).unwrap();
        let resolution = strategy.resolve(&TenantId::new("acme"));

        match resolution {
            TenantResolution::Schema { schema_name } => {
                assert_eq!(schema_name, "tenant_acme");
            }
            _ => panic!("expected Schema resolution"),
        }
    }

    #[test]
    fn test_set_search_path_sql() {
        let strategy = SchemaPerTenantStrategy::new(SchemaPerTenantConfig::default()).unwrap();
        let sql = strategy.set_search_path_sql(&TenantId::new("acme"));
        assert_eq!(
            sql,
            "SET search_path TO \"tenant_acme\", \"shared\", public"
        );
    }

    #[test]
    fn test_create_schema_sql() {
        let strategy = SchemaPerTenantStrategy::new(SchemaPerTenantConfig::default()).unwrap();
        let sql = strategy.create_schema_sql(&TenantId::new("acme"));
        assert_eq!(sql, "CREATE SCHEMA IF NOT EXISTS \"tenant_acme\"");
    }

    #[test]
    fn test_create_schema_sql_with_template() {
        let config = SchemaPerTenantConfig::new().with_template("tenant_template");
        let strategy = SchemaPerTenantStrategy::new(config).unwrap();
        let sql = strategy.create_schema_sql(&TenantId::new("acme"));
        assert!(sql.contains("TEMPLATE"));
        assert!(sql.contains("tenant_template"));
    }

    #[test]
    fn test_drop_schema_sql() {
        let strategy = SchemaPerTenantStrategy::new(SchemaPerTenantConfig::default()).unwrap();

        let sql = strategy.drop_schema_sql(&TenantId::new("acme"), false);
        assert_eq!(sql, "DROP SCHEMA IF EXISTS \"tenant_acme\"");

        let sql_cascade = strategy.drop_schema_sql(&TenantId::new("acme"), true);
        assert_eq!(sql_cascade, "DROP SCHEMA IF EXISTS \"tenant_acme\" CASCADE");
    }

    #[test]
    fn test_schema_exists_sql() {
        let strategy = SchemaPerTenantStrategy::new(SchemaPerTenantConfig::default()).unwrap();
        let sql = strategy.schema_exists_sql(&TenantId::new("acme"));
        assert!(sql.contains("information_schema.schemata"));
        assert!(sql.contains("tenant_acme"));
    }

    #[test]
    fn test_list_tenant_schemas_sql() {
        let strategy = SchemaPerTenantStrategy::new(SchemaPerTenantConfig::default()).unwrap();
        let sql = strategy.list_tenant_schemas_sql();
        assert!(sql.contains("LIKE 'tenant_%'"));
    }

    #[test]
    fn test_system_tenant_resolution() {
        let strategy = SchemaPerTenantStrategy::new(SchemaPerTenantConfig::default()).unwrap();
        let resolution = strategy.system_tenant();

        match resolution {
            TenantResolution::Schema { schema_name } => {
                assert_eq!(schema_name, "shared");
            }
            _ => panic!("expected Schema resolution"),
        }
    }

    #[test]
    fn test_schema_manager_create_shared() {
        let strategy = SchemaPerTenantStrategy::new(SchemaPerTenantConfig::default()).unwrap();
        let manager = SchemaManager::new(&strategy);
        let ddl = manager.create_shared_schema_ddl();
        assert!(ddl.contains("CREATE SCHEMA IF NOT EXISTS"));
        assert!(ddl.contains("shared"));
    }

    #[test]
    fn test_tenant_validation_valid() {
        let strategy = SchemaPerTenantStrategy::new(SchemaPerTenantConfig::default()).unwrap();
        assert!(strategy.validate(&TenantId::new("acme")).is_ok());
        assert!(strategy.validate(&TenantId::new("acme-corp")).is_ok());
    }

    #[test]
    fn test_escape_identifier() {
        let strategy = SchemaPerTenantStrategy::new(SchemaPerTenantConfig::default()).unwrap();
        let escaped = strategy.escape_identifier("test\"schema");
        assert_eq!(escaped, "\"test\"\"schema\"");
    }
}
