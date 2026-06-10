//! Helios FHIR Server Persistence Layer
//!
//! This crate provides a polyglot persistence layer for storing and retrieving FHIR resources.
//! It supports multiple database backends via feature flags and provides configurable
//! multitenancy with full FHIR search capabilities.
//!
//! # Features
//!
//! - **Multiple Backends**: SQLite, PostgreSQL, Cassandra, MongoDB, Neo4j, Elasticsearch, S3
//! - **Multitenancy**: Three isolation strategies (shared schema, schema-per-tenant, database-per-tenant)
//! - **Full FHIR Search**: All parameter types, modifiers, chaining, _include/_revinclude
//! - **Versioning**: Full resource history with optimistic locking
//! - **Transactions**: ACID transactions with bundle support
//!
//! # Backend Features
//!
//! Enable backends with feature flags in `Cargo.toml`:
//!
//! ```toml
//! [dependencies]
//! helios-persistence = { version = "0.1", features = ["postgres", "R4"] }
//! ```
//!
//! Available backend features:
//! - `sqlite` (default) - SQLite with in-memory and file modes
//! - `postgres` - PostgreSQL with JSONB storage
//! - `cassandra` - Apache Cassandra via cdrs-tokio
//! - `mongodb` - MongoDB document storage
//! - `neo4j` - Neo4j graph database
//! - `elasticsearch` - Elasticsearch for full-text search
//! - `s3` - AWS S3 object storage
//!
//! FHIR version features:
//! - `R4`, `R4B`, `R5`, `R6`
//!
//! # Architecture
//!
//! The persistence layer is organized into several modules:
//!
//! - [`tenant`] - Multi-tenant support with mandatory tenant context
//! - [`types`] - Core types for stored resources and search
//! - [`error`] - Error types for all operations
//! - [`core`] - Storage traits and abstractions
//! - [`strategy`] - Tenancy isolation strategies (shared schema, schema-per-tenant, database-per-tenant)
//! - [`backends`] - Backend implementations (SQLite, PostgreSQL, etc.)
//!
//! # Quick Start
//!
//! ```no_run
//! use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
//! use helios_persistence::types::StoredResource;
//! use helios_fhir::FhirVersion;
//! use serde_json::json;
//!
//! // Create a tenant context (required for all operations)
//! let tenant = TenantContext::new(
//!     TenantId::new("my-organization"),
//!     TenantPermissions::full_access(),
//! );
//!
//! // Create a stored resource
//! let resource = StoredResource::new(
//!     "Patient",
//!     "patient-123",
//!     tenant.tenant_id().clone(),
//!     json!({
//!         "resourceType": "Patient",
//!         "id": "patient-123",
//!         "name": [{"family": "Smith", "given": ["John"]}]
//!     }),
//!     FhirVersion::default(),
//! );
//!
//! // The resource includes persistence metadata
//! assert_eq!(resource.version_id(), "1");
//! assert_eq!(resource.url(), "Patient/patient-123");
//! ```
//!
//! # Multitenancy
//!
//! All storage operations require a [`TenantContext`](tenant::TenantContext), ensuring
//! tenant isolation at the type level. There is no way to bypass this requirement.
//!
//! ```
//! use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions, Operation};
//!
//! // Full access tenant
//! let admin_ctx = TenantContext::new(
//!     TenantId::new("acme"),
//!     TenantPermissions::full_access(),
//! );
//!
//! // Read-only tenant
//! let reader_ctx = TenantContext::new(
//!     TenantId::new("acme"),
//!     TenantPermissions::read_only(),
//! );
//!
//! // Check permissions
//! assert!(admin_ctx.check_permission(Operation::Create, "Patient").is_ok());
//! assert!(reader_ctx.check_permission(Operation::Create, "Patient").is_err());
//! ```
//!
//! # Search
//!
//! Build search queries with full FHIR search support:
//!
//! ```
//! use helios_persistence::types::{
//!     SearchQuery, SearchParameter, SearchParamType, SearchValue,
//!     SearchModifier, SortDirective, IncludeDirective, IncludeType,
//! };
//!
//! // Simple search
//! let query = SearchQuery::new("Patient")
//!     .with_parameter(SearchParameter {
//!         name: "name".to_string(),
//!         param_type: SearchParamType::String,
//!         modifier: Some(SearchModifier::Contains),
//!         values: vec![SearchValue::eq("smith")],
//!         chain: vec![],
//!         components: vec![],
//!     })
//!     .with_sort(SortDirective::parse("-_lastUpdated"))
//!     .with_count(20);
//!
//! // With _include
//! let query_with_include = SearchQuery::new("Observation")
//!     .with_include(IncludeDirective {
//!         include_type: IncludeType::Include,
//!         source_type: "Observation".to_string(),
//!         search_param: "patient".to_string(),
//!         target_type: Some("Patient".to_string()),
//!         iterate: false,
//!     });
//! ```

#![warn(missing_docs)]
#![warn(rustdoc::missing_crate_level_docs)]
// The Elasticsearch index mapping is built with a single large `json!` literal;
// the added `_contained` fields push it past the default macro recursion limit.
#![recursion_limit = "256"]

pub mod advisor;
pub mod backends;
pub mod composite;
pub mod core;
pub mod error;
pub mod search;
pub mod strategy;
pub mod tenant;
pub mod types;

/// Default FHIR version for backend configuration fields.
///
/// Used as the `#[serde(default = "...")]` source for `fhir_version` fields on the
/// backend `Config` structs. Unlike `#[serde(default)]` — which would require the
/// `Default` impl for `FhirVersion` that is gated on `feature = "R4"` — this
/// resolves to [`helios_fhir::FhirVersion::default_enabled`], which is available in
/// any single-version-minimal build (e.g. R4B-only).
pub(crate) fn default_fhir_version() -> helios_fhir::FhirVersion {
    helios_fhir::FhirVersion::default_enabled()
}

// Re-export commonly used types at crate root
pub use error::{StorageError, StorageResult};
pub use tenant::{TenantContext, TenantId, TenantPermissions};
pub use types::{Pagination, SearchQuery, StoredResource};

// Re-export core traits
pub use core::{
    Backend, BackendKind, CapabilityProvider, ResourceStorage, SearchProvider, Transaction,
    TransactionProvider, VersionedStorage,
};

// Re-export tenancy strategies
pub use strategy::{
    DatabasePerTenantConfig, DatabasePerTenantStrategy, IsolationLevel, SchemaPerTenantConfig,
    SchemaPerTenantStrategy, SharedSchemaConfig, SharedSchemaStrategy, TenancyStrategy,
    TenantResolution, TenantResolver,
};

/// Crate version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Crate name.
pub const NAME: &str = env!("CARGO_PKG_NAME");
