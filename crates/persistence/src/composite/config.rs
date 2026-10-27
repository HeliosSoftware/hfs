//! Configuration types for composite storage.
//!
//! This module provides configuration types for setting up composite storage
//! with multiple backends. The configuration defines:
//!
//! - Backend entries with their roles and capabilities
//! - Routing rules for directing queries to appropriate backends
//! - Synchronization settings between primary and secondary backends
//! - Cost configuration for query optimization
//!
//! # Example
//!
//! ```ignore
//! use helios_persistence::composite::{CompositeConfig, BackendRole};
//!
//! let config = CompositeConfigBuilder::new()
//!     .with_backend("sqlite", BackendRole::Primary)
//!     .with_backend("elasticsearch", BackendRole::Search)
//!     .with_sync_mode(SyncMode::Asynchronous)
//!     .build()?;
//! ```

use std::collections::HashMap;
use std::time::Duration;

use helios_fhir::FhirVersion;
use serde::{Deserialize, Serialize};

use crate::core::{BackendCapability, BackendKind};

use super::analyzer::QueryFeature;

/// Role of a backend in the composite storage.
///
/// Each backend serves a specific role in the composite architecture:
/// - **Primary**: The authoritative store for all FHIR resources (CRUD, versioning, history)
/// - **Search**: Optimized for full-text and advanced search operations
/// - **Graph**: Optimized for relationship traversal (chained searches, _has)
/// - **Terminology**: Handles code system expansion (:above, :below, :in, :not-in)
/// - **Archive**: Cold storage for historical data
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendRole {
    /// Primary storage - the single source of truth for all FHIR resources.
    /// Handles all CRUD operations, versioning, and history.
    Primary,

    /// Search optimization backend (e.g., Elasticsearch).
    /// Handles full-text search (_text, _content) and advanced text matching.
    Search,

    /// Graph query backend (e.g., Neo4j).
    /// Handles chained parameters and reverse chaining (_has).
    Graph,

    /// Terminology service.
    /// Handles code expansion for :above, :below, :in, :not-in modifiers.
    Terminology,

    /// Archive storage for cold data (e.g., S3).
    /// Used for bulk export and historical data.
    Archive,
}

impl BackendRole {
    /// Returns true if this role can be used for primary resource storage.
    pub fn is_primary(&self) -> bool {
        matches!(self, BackendRole::Primary)
    }

    /// Returns true if this is a secondary/auxiliary role.
    pub fn is_secondary(&self) -> bool {
        !self.is_primary()
    }

    /// Returns the typical capabilities associated with this role.
    pub fn typical_capabilities(&self) -> Vec<BackendCapability> {
        match self {
            BackendRole::Primary => vec![
                BackendCapability::Crud,
                BackendCapability::Versioning,
                BackendCapability::InstanceHistory,
                BackendCapability::TypeHistory,
                BackendCapability::SystemHistory,
                BackendCapability::BasicSearch,
                BackendCapability::DateSearch,
                BackendCapability::ReferenceSearch,
                BackendCapability::Transactions,
                BackendCapability::OptimisticLocking,
                BackendCapability::Include,
                BackendCapability::Revinclude,
                BackendCapability::Sorting,
                BackendCapability::OffsetPagination,
                BackendCapability::CursorPagination,
            ],
            BackendRole::Search => vec![
                BackendCapability::BasicSearch,
                BackendCapability::FullTextSearch,
                BackendCapability::Sorting,
            ],
            BackendRole::Graph => vec![
                BackendCapability::ChainedSearch,
                BackendCapability::ReverseChaining,
                BackendCapability::Include,
                BackendCapability::Revinclude,
            ],
            BackendRole::Terminology => vec![BackendCapability::TerminologySearch],
            BackendRole::Archive => vec![
                BackendCapability::Crud,
                BackendCapability::Versioning,
                BackendCapability::InstanceHistory,
            ],
        }
    }
}

/// Configuration for a single backend in the composite storage.
///
/// Note: Serde serialization is not directly supported due to complex types.
/// Use the builder pattern for configuration.
#[derive(Debug, Clone)]
pub struct BackendEntry {
    /// Unique identifier for this backend.
    pub id: String,

    /// The role this backend plays in the composite.
    pub role: BackendRole,

    /// Backend kind (sqlite, postgres, elasticsearch, etc.).
    pub kind: BackendKind,

    /// Connection string or configuration for this backend.
    pub connection: String,

    /// Priority for routing (lower = preferred when multiple backends can handle a query).
    pub priority: u8,

    /// Whether this backend is enabled.
    pub enabled: bool,

    /// Explicit capabilities this backend provides.
    /// If empty, derived from the backend kind and role.
    pub capabilities: Vec<BackendCapability>,

    /// Failover backend ID when this backend is unavailable.
    pub failover_to: Option<String>,

    /// Additional backend-specific configuration.
    pub options: HashMap<String, serde_json::Value>,
}

fn default_priority() -> u8 {
    100
}

impl BackendEntry {
    /// Creates a new backend entry.
    pub fn new(id: impl Into<String>, role: BackendRole, kind: BackendKind) -> Self {
        Self {
            id: id.into(),
            role,
            kind,
            connection: String::new(),
            priority: default_priority(),
            enabled: true,
            capabilities: Vec::new(),
            failover_to: None,
            options: HashMap::new(),
        }
    }

    /// Sets the connection string.
    pub fn with_connection(mut self, connection: impl Into<String>) -> Self {
        self.connection = connection.into();
        self
    }

    /// Sets the priority.
    pub fn with_priority(mut self, priority: u8) -> Self {
        self.priority = priority;
        self
    }

    /// Sets the failover backend.
    pub fn with_failover(mut self, failover_id: impl Into<String>) -> Self {
        self.failover_to = Some(failover_id.into());
        self
    }

    /// Adds explicit capabilities.
    pub fn with_capabilities(mut self, capabilities: Vec<BackendCapability>) -> Self {
        self.capabilities = capabilities;
        self
    }

    /// Returns the effective capabilities (explicit or derived from role).
    pub fn effective_capabilities(&self) -> Vec<BackendCapability> {
        if self.capabilities.is_empty() {
            self.role.typical_capabilities()
        } else {
            self.capabilities.clone()
        }
    }

    /// Checks if this backend supports a capability.
    pub fn supports(&self, capability: BackendCapability) -> bool {
        self.effective_capabilities().contains(&capability)
    }
}

/// A routing rule for directing specific queries to backends.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingRule {
    /// Rule identifier.
    pub id: String,

    /// Feature(s) that trigger this rule.
    pub triggers: Vec<QueryFeature>,

    /// Target backend ID.
    pub target_backend: String,

    /// Priority (lower = higher priority).
    #[serde(default = "default_priority")]
    pub priority: u8,

    /// Whether to fall back to primary if target is unavailable.
    #[serde(default = "default_fallback")]
    pub fallback_to_primary: bool,
}

fn default_fallback() -> bool {
    true
}

impl RoutingRule {
    /// Creates a new routing rule.
    pub fn new(id: impl Into<String>, target_backend: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            triggers: Vec::new(),
            target_backend: target_backend.into(),
            priority: default_priority(),
            fallback_to_primary: true,
        }
    }

    /// Adds a trigger feature.
    pub fn with_trigger(mut self, feature: QueryFeature) -> Self {
        self.triggers.push(feature);
        self
    }

    /// Adds multiple trigger features.
    pub fn with_triggers(mut self, features: Vec<QueryFeature>) -> Self {
        self.triggers.extend(features);
        self
    }

    /// Sets the priority.
    pub fn with_priority(mut self, priority: u8) -> Self {
        self.priority = priority;
        self
    }

    /// Sets whether to fall back to primary.
    pub fn with_fallback(mut self, fallback: bool) -> Self {
        self.fallback_to_primary = fallback;
        self
    }
}

/// Synchronization mode for secondary backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SyncMode {
    /// Update secondaries in the same transaction/operation.
    /// Higher latency but strong consistency.
    Synchronous,

    /// Update secondaries via event stream.
    /// Lower latency but eventual consistency.
    #[default]
    Asynchronous,

    /// Hybrid: sync critical data, async for the rest.
    Hybrid {
        /// Whether to sync search indexes synchronously.
        sync_for_search: bool,
    },
}

/// Retry configuration for sync operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    /// Maximum number of retry attempts.
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,

    /// Initial delay between retries.
    #[serde(with = "humantime_serde", default = "default_initial_delay")]
    pub initial_delay: Duration,

    /// Maximum delay between retries.
    #[serde(with = "humantime_serde", default = "default_max_delay")]
    pub max_delay: Duration,

    /// Backoff multiplier.
    #[serde(default = "default_backoff_multiplier")]
    pub backoff_multiplier: f64,
}

fn default_max_retries() -> u32 {
    3
}

fn default_initial_delay() -> Duration {
    Duration::from_millis(100)
}

fn default_max_delay() -> Duration {
    Duration::from_secs(5)
}

fn default_backoff_multiplier() -> f64 {
    2.0
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: default_max_retries(),
            initial_delay: default_initial_delay(),
            max_delay: default_max_delay(),
            backoff_multiplier: default_backoff_multiplier(),
        }
    }
}

/// Synchronization configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConfig {
    /// Sync mode: synchronous, asynchronous, or hybrid.
    #[serde(default)]
    pub mode: SyncMode,

    /// Maximum acceptable read lag in milliseconds.
    /// Reads may wait up to this long for sync to complete.
    #[serde(default = "default_max_read_lag")]
    pub max_read_lag_ms: u64,

    /// Batch size for async sync operations.
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,

    /// Retry configuration for failed sync operations.
    #[serde(default)]
    pub retry: RetryConfig,
}

fn default_max_read_lag() -> u64 {
    500
}

fn default_batch_size() -> usize {
    100
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            mode: SyncMode::default(),
            max_read_lag_ms: default_max_read_lag(),
            batch_size: default_batch_size(),
            retry: RetryConfig::default(),
        }
    }
}

/// Cost weights for query optimization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostWeights {
    /// Weight for latency in cost calculation.
    #[serde(default = "default_latency_weight")]
    pub latency: f64,

    /// Weight for resource usage.
    #[serde(default = "default_resource_weight")]
    pub resource_usage: f64,

    /// Weight for result quality (relevance scoring).
    #[serde(default = "default_quality_weight")]
    pub quality: f64,
}

fn default_latency_weight() -> f64 {
    0.5
}

fn default_resource_weight() -> f64 {
    0.3
}

fn default_quality_weight() -> f64 {
    0.2
}

impl Default for CostWeights {
    fn default() -> Self {
        Self {
            latency: default_latency_weight(),
            resource_usage: default_resource_weight(),
            quality: default_quality_weight(),
        }
    }
}

/// Cost configuration for query optimization.
///
/// Costs are derived from Criterion benchmarks and stored as defaults.
/// These can be overridden based on deployment-specific performance characteristics.
#[derive(Debug, Clone)]
pub struct CostConfig {
    /// Base costs per backend kind (in arbitrary units).
    /// Derived from benchmark measurements.
    pub base_costs: HashMap<BackendKind, f64>,

    /// Cost multipliers per query feature.
    pub feature_multipliers: HashMap<QueryFeature, f64>,

    /// Weights for combining cost components.
    pub weights: CostWeights,
}

impl Default for CostConfig {
    fn default() -> Self {
        let mut base_costs = HashMap::new();
        // Default costs based on typical performance characteristics
        // These should be updated from benchmark results
        base_costs.insert(BackendKind::Sqlite, 1.0);
        base_costs.insert(BackendKind::Postgres, 1.2);
        base_costs.insert(BackendKind::Elasticsearch, 0.8);
        base_costs.insert(BackendKind::Neo4j, 1.5);
        base_costs.insert(BackendKind::S3, 2.0);

        let mut feature_multipliers = HashMap::new();
        // Default multipliers - higher means more expensive
        feature_multipliers.insert(QueryFeature::BasicSearch, 1.0);
        feature_multipliers.insert(QueryFeature::ChainedSearch, 3.0);
        feature_multipliers.insert(QueryFeature::ReverseChaining, 3.5);
        feature_multipliers.insert(QueryFeature::FullTextSearch, 1.5);
        feature_multipliers.insert(QueryFeature::TerminologySearch, 2.0);

        Self {
            base_costs,
            feature_multipliers,
            weights: CostWeights::default(),
        }
    }
}

/// Health check configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthConfig {
    /// Interval between health checks.
    #[serde(with = "humantime_serde", default = "default_health_interval")]
    pub check_interval: Duration,

    /// Timeout for health check operations.
    #[serde(with = "humantime_serde", default = "default_health_timeout")]
    pub timeout: Duration,

    /// Number of consecutive failures before marking unhealthy.
    #[serde(default = "default_failure_threshold")]
    pub failure_threshold: u32,

    /// Number of consecutive successes before marking healthy.
    #[serde(default = "default_success_threshold")]
    pub success_threshold: u32,
}

fn default_health_interval() -> Duration {
    Duration::from_secs(30)
}

fn default_health_timeout() -> Duration {
    Duration::from_secs(5)
}

fn default_failure_threshold() -> u32 {
    3
}

fn default_success_threshold() -> u32 {
    2
}

impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            check_interval: default_health_interval(),
            timeout: default_health_timeout(),
            failure_threshold: default_failure_threshold(),
            success_threshold: default_success_threshold(),
        }
    }
}

/// Complete composite storage configuration.
#[derive(Debug, Clone)]
pub struct CompositeConfig {
    /// All configured backends.
    pub backends: Vec<BackendEntry>,

    /// Custom routing rules (override automatic detection).
    pub routing_rules: Vec<RoutingRule>,

    /// Synchronization settings.
    pub sync_config: SyncConfig,

    /// Cost model configuration.
    pub cost_config: CostConfig,

    /// Health check settings.
    pub health_config: HealthConfig,

    /// The FHIR version this composite serves — under `hfs`, the server's
    /// default version, which is the one its search index is built for.
    ///
    /// Conditional criteria the composite resolves itself (it has a dedicated
    /// search backend) judge a `:[type]` qualifier against this version.
    /// `None` keeps the version-less fallback: a type of any FHIR version
    /// enabled in the build passes (#1384).
    pub fhir_version: Option<FhirVersion>,
}

impl CompositeConfig {
    /// Creates a new empty configuration.
    pub fn new() -> Self {
        Self {
            backends: Vec::new(),
            routing_rules: Vec::new(),
            sync_config: SyncConfig::default(),
            cost_config: CostConfig::default(),
            health_config: HealthConfig::default(),
            fhir_version: None,
        }
    }

    /// Creates a builder for constructing configuration.
    pub fn builder() -> CompositeConfigBuilder {
        CompositeConfigBuilder::new()
    }

    /// Returns the primary backend entry.
    pub fn primary(&self) -> Option<&BackendEntry> {
        self.backends
            .iter()
            .find(|b| b.role.is_primary() && b.enabled)
    }

    /// Returns the primary backend ID.
    pub fn primary_id(&self) -> Option<&str> {
        self.primary().map(|b| b.id.as_str())
    }

    /// Returns all secondary (non-primary) backends.
    pub fn secondaries(&self) -> impl Iterator<Item = &BackendEntry> {
        self.backends
            .iter()
            .filter(|b| b.role.is_secondary() && b.enabled)
    }

    /// Returns a backend by ID.
    pub fn backend(&self, id: &str) -> Option<&BackendEntry> {
        self.backends.iter().find(|b| b.id == id)
    }

    /// Returns backends with a specific role.
    pub fn backends_with_role(&self, role: BackendRole) -> impl Iterator<Item = &BackendEntry> {
        self.backends
            .iter()
            .filter(move |b| b.role == role && b.enabled)
    }

    /// Returns backends that support a specific capability.
    pub fn backends_with_capability(
        &self,
        capability: BackendCapability,
    ) -> impl Iterator<Item = &BackendEntry> {
        self.backends
            .iter()
            .filter(move |b| b.enabled && b.supports(capability))
    }

    /// Validates the configuration and returns any errors.
    pub fn validate(&self) -> Result<Vec<ConfigWarning>, ConfigError> {
        let mut warnings = Vec::new();

        // Must have exactly one primary
        let primaries: Vec<_> = self
            .backends
            .iter()
            .filter(|b| b.role.is_primary() && b.enabled)
            .collect();

        if primaries.is_empty() {
            return Err(ConfigError::NoPrimaryBackend);
        }
        if primaries.len() > 1 {
            return Err(ConfigError::MultiplePrimaryBackends(
                primaries.iter().map(|b| b.id.clone()).collect(),
            ));
        }

        // Check for duplicate IDs
        let mut seen_ids = std::collections::HashSet::new();
        for backend in &self.backends {
            if !seen_ids.insert(&backend.id) {
                return Err(ConfigError::DuplicateBackendId(backend.id.clone()));
            }
        }

        // Check failover references
        for backend in &self.backends {
            if let Some(ref failover_id) = backend.failover_to {
                if self.backend(failover_id).is_none() {
                    return Err(ConfigError::InvalidFailoverReference {
                        backend_id: backend.id.clone(),
                        failover_id: failover_id.clone(),
                    });
                }
            }
        }

        // Check routing rule references
        for rule in &self.routing_rules {
            if self.backend(&rule.target_backend).is_none() {
                return Err(ConfigError::InvalidRoutingTarget {
                    rule_id: rule.id.clone(),
                    target_id: rule.target_backend.clone(),
                });
            }
        }

        // Warnings for potential issues
        if self.secondaries().count() == 0 {
            warnings.push(ConfigWarning::NoSecondaryBackends);
        }

        // Check for redundant capabilities
        let search_backends: Vec<_> = self
            .backends_with_capability(BackendCapability::FullTextSearch)
            .collect();
        if search_backends.len() > 1 {
            warnings.push(ConfigWarning::RedundantCapability {
                capability: BackendCapability::FullTextSearch,
                backends: search_backends.iter().map(|b| b.id.clone()).collect(),
            });
        }

        Ok(warnings)
    }
}

impl Default for CompositeConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for constructing [`CompositeConfig`].
#[derive(Debug, Default)]
pub struct CompositeConfigBuilder {
    backends: Vec<BackendEntry>,
    routing_rules: Vec<RoutingRule>,
    sync_config: SyncConfig,
    cost_config: CostConfig,
    health_config: HealthConfig,
    fhir_version: Option<FhirVersion>,
}

impl CompositeConfigBuilder {
    /// Creates a new builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a backend entry.
    pub fn with_backend(mut self, backend: BackendEntry) -> Self {
        self.backends.push(backend);
        self
    }

    /// Adds a primary backend.
    pub fn primary(mut self, id: impl Into<String>, kind: BackendKind) -> Self {
        self.backends
            .push(BackendEntry::new(id, BackendRole::Primary, kind));
        self
    }

    /// Adds a search backend.
    pub fn search_backend(mut self, id: impl Into<String>, kind: BackendKind) -> Self {
        self.backends
            .push(BackendEntry::new(id, BackendRole::Search, kind));
        self
    }

    /// Adds a graph backend.
    pub fn graph_backend(mut self, id: impl Into<String>, kind: BackendKind) -> Self {
        self.backends
            .push(BackendEntry::new(id, BackendRole::Graph, kind));
        self
    }

    /// Adds a terminology backend.
    pub fn terminology_backend(mut self, id: impl Into<String>, kind: BackendKind) -> Self {
        self.backends
            .push(BackendEntry::new(id, BackendRole::Terminology, kind));
        self
    }

    /// Adds a routing rule.
    pub fn with_routing_rule(mut self, rule: RoutingRule) -> Self {
        self.routing_rules.push(rule);
        self
    }

    /// Sets the sync mode.
    pub fn sync_mode(mut self, mode: SyncMode) -> Self {
        self.sync_config.mode = mode;
        self
    }

    /// Sets the FHIR version the composite serves (see
    /// [`CompositeConfig::fhir_version`]).
    pub fn fhir_version(mut self, fhir_version: FhirVersion) -> Self {
        self.fhir_version = Some(fhir_version);
        self
    }

    /// Sets the sync configuration.
    pub fn with_sync_config(mut self, config: SyncConfig) -> Self {
        self.sync_config = config;
        self
    }

    /// Sets the cost configuration.
    pub fn with_cost_config(mut self, config: CostConfig) -> Self {
        self.cost_config = config;
        self
    }

    /// Sets the health configuration.
    pub fn with_health_config(mut self, config: HealthConfig) -> Self {
        self.health_config = config;
        self
    }

    /// Builds the configuration, validating it first.
    pub fn build(self) -> Result<CompositeConfig, ConfigError> {
        let config = CompositeConfig {
            backends: self.backends,
            routing_rules: self.routing_rules,
            sync_config: self.sync_config,
            cost_config: self.cost_config,
            health_config: self.health_config,
            fhir_version: self.fhir_version,
        };

        // Validate and ignore warnings for build
        let _ = config.validate()?;
        Ok(config)
    }

    /// Builds the configuration and returns warnings.
    pub fn build_with_warnings(self) -> Result<(CompositeConfig, Vec<ConfigWarning>), ConfigError> {
        let config = CompositeConfig {
            backends: self.backends,
            routing_rules: self.routing_rules,
            sync_config: self.sync_config,
            cost_config: self.cost_config,
            health_config: self.health_config,
            fhir_version: self.fhir_version,
        };

        let warnings = config.validate()?;
        Ok((config, warnings))
    }
}

/// Configuration errors.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ConfigError {
    /// No primary backend configured.
    #[error("no primary backend configured - exactly one primary backend is required")]
    NoPrimaryBackend,

    /// Multiple primary backends configured.
    #[error("multiple primary backends configured: {0:?} - only one primary is allowed")]
    MultiplePrimaryBackends(Vec<String>),

    /// Duplicate backend ID.
    #[error("duplicate backend ID: {0}")]
    DuplicateBackendId(String),

    /// Invalid failover reference.
    #[error("backend '{backend_id}' references non-existent failover backend '{failover_id}'")]
    InvalidFailoverReference {
        /// The backend with the invalid reference.
        backend_id: String,
        /// The non-existent failover ID.
        failover_id: String,
    },

    /// Invalid routing rule target.
    #[error("routing rule '{rule_id}' targets non-existent backend '{target_id}'")]
    InvalidRoutingTarget {
        /// The routing rule ID.
        rule_id: String,
        /// The non-existent target ID.
        target_id: String,
    },
}

/// Configuration warnings (non-fatal issues).
#[derive(Debug, Clone)]
pub enum ConfigWarning {
    /// No secondary backends configured.
    NoSecondaryBackends,

    /// Redundant capability across multiple backends.
    RedundantCapability {
        /// The redundant capability.
        capability: BackendCapability,
        /// Backends providing this capability.
        backends: Vec<String>,
    },
}

impl std::fmt::Display for ConfigWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigWarning::NoSecondaryBackends => {
                write!(
                    f,
                    "no secondary backends configured - using primary for all operations"
                )
            }
            ConfigWarning::RedundantCapability {
                capability,
                backends,
            } => {
                write!(
                    f,
                    "capability {:?} is provided by multiple backends: {:?}",
                    capability, backends
                )
            }
        }
    }
}

/// Serde module for Duration with humantime format.
mod humantime_serde {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&humantime::format_duration(*duration).to_string())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        humantime::parse_duration(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backend_role_capabilities() {
        let primary_caps = BackendRole::Primary.typical_capabilities();
        assert!(primary_caps.contains(&BackendCapability::Crud));
        assert!(primary_caps.contains(&BackendCapability::Versioning));

        let search_caps = BackendRole::Search.typical_capabilities();
        assert!(search_caps.contains(&BackendCapability::FullTextSearch));

        let graph_caps = BackendRole::Graph.typical_capabilities();
        assert!(graph_caps.contains(&BackendCapability::ChainedSearch));
    }

    #[test]
    fn test_config_builder_minimal() {
        let config = CompositeConfigBuilder::new()
            .primary("sqlite", BackendKind::Sqlite)
            .build()
            .unwrap();

        assert_eq!(config.backends.len(), 1);
        assert!(config.primary().is_some());
        assert_eq!(config.primary_id(), Some("sqlite"));
    }

    #[test]
    fn test_config_builder_with_secondaries() {
        let config = CompositeConfigBuilder::new()
            .primary("pg", BackendKind::Postgres)
            .search_backend("es", BackendKind::Elasticsearch)
            .graph_backend("neo4j", BackendKind::Neo4j)
            .build()
            .unwrap();

        assert_eq!(config.backends.len(), 3);
        assert_eq!(config.secondaries().count(), 2);
    }

    #[test]
    fn test_config_validation_no_primary() {
        let result = CompositeConfigBuilder::new()
            .search_backend("es", BackendKind::Elasticsearch)
            .build();

        assert!(matches!(result, Err(ConfigError::NoPrimaryBackend)));
    }

    #[test]
    fn test_config_validation_multiple_primaries() {
        let result = CompositeConfigBuilder::new()
            .primary("pg1", BackendKind::Postgres)
            .primary("pg2", BackendKind::Postgres)
            .build();

        assert!(matches!(
            result,
            Err(ConfigError::MultiplePrimaryBackends(_))
        ));
    }

    #[test]
    fn test_backend_entry_effective_capabilities() {
        let entry = BackendEntry::new("test", BackendRole::Search, BackendKind::Elasticsearch);
        let caps = entry.effective_capabilities();
        assert!(caps.contains(&BackendCapability::FullTextSearch));

        // With explicit capabilities
        let entry_explicit =
            BackendEntry::new("test", BackendRole::Search, BackendKind::Elasticsearch)
                .with_capabilities(vec![BackendCapability::BasicSearch]);
        let caps_explicit = entry_explicit.effective_capabilities();
        assert!(caps_explicit.contains(&BackendCapability::BasicSearch));
        assert!(!caps_explicit.contains(&BackendCapability::FullTextSearch));
    }
}
