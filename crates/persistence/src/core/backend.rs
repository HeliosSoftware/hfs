//! Backend abstraction for database drivers.
//!
//! This module defines the [`Backend`] trait, which provides a Diesel-inspired
//! abstraction over different database backends. Each backend implements this
//! trait to provide database-specific query building and execution.

use std::fmt::Debug;

use async_trait::async_trait;

use crate::error::BackendError;

/// Identifies the type of database backend.
///
/// This enum is used for runtime capability checks and query routing
/// in composite storage configurations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BackendKind {
    /// SQLite database (file-based or in-memory).
    Sqlite,
    /// PostgreSQL database.
    Postgres,
    /// Apache Cassandra (wide-column store).
    ///
    /// Reserved: no backend is implemented for this kind. It is accepted by
    /// composite configuration and the advisor but cannot be instantiated.
    Cassandra,
    /// MongoDB (document store).
    MongoDB,
    /// Neo4j (graph database).
    ///
    /// Reserved: no backend is implemented for this kind. It is accepted by
    /// composite configuration and the advisor but cannot be instantiated.
    Neo4j,
    /// Elasticsearch (search engine).
    Elasticsearch,
    /// AWS S3 (object storage).
    S3,
    /// Custom or unknown backend.
    Custom(&'static str),
}

impl std::fmt::Display for BackendKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackendKind::Sqlite => write!(f, "sqlite"),
            BackendKind::Postgres => write!(f, "postgres"),
            BackendKind::Cassandra => write!(f, "cassandra"),
            BackendKind::MongoDB => write!(f, "mongodb"),
            BackendKind::Neo4j => write!(f, "neo4j"),
            BackendKind::Elasticsearch => write!(f, "elasticsearch"),
            BackendKind::S3 => write!(f, "s3"),
            BackendKind::Custom(name) => write!(f, "{}", name),
        }
    }
}

/// Capabilities that a backend may support.
///
/// Used for runtime capability discovery and query routing.
///
/// # What a declared capability means
///
/// Each variant asserts an **observable property of the instance that returned
/// it, as configured** — not of the backend type, and not of what the backend
/// could be configured to do with more code. A capability must never be
/// declared for aspirational, planned, or implemented-but-unwired behavior;
/// roadmap belongs in the crate README, not here.
///
/// This is the reading the trait already committed to:
/// [`Backend::supports`] and [`Backend::capabilities`] both take `&self`, and
/// the rustdoc on [`Backend`] uses `supports()` as a runtime dispatch
/// predicate. Under a "could be configured to" reading, the fallback arm of
/// such a branch is unreachable by construction and the positive arm is a
/// hazard. The same instance-scoped reading is already the house style for
/// this question elsewhere in the crate — see `S3Backend::supports_user_settings`
/// and [`crate::core::ResourceStorage::supports_tenant_registry`].
///
/// Configuration may only refine **which** member of a mutually exclusive
/// capability group an instance reports (see the tenancy variants below). It
/// must never motivate adding a new variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BackendCapability {
    /// Basic CRUD operations.
    Crud,
    /// Resource versioning (vread).
    Versioning,
    /// Instance-level history.
    InstanceHistory,
    /// Type-level history.
    TypeHistory,
    /// System-level history.
    SystemHistory,
    /// Basic search with token/string parameters.
    BasicSearch,
    /// Date range search.
    DateSearch,
    /// Quantity search with units.
    QuantitySearch,
    /// Reference search.
    ReferenceSearch,
    /// Chained search parameters.
    ///
    /// Declared by any backend on which a chained query resolves correctly in a
    /// running deployment, whether the backend joins natively or the
    /// persistence layer's backend-agnostic resolver
    /// (`crate::search::resolve_chains`) rewrites the chain into an `_id` filter
    /// that the backend's `search()` then executes. It therefore requires a
    /// working `search()` that honours `_id`; a backend without search (e.g.
    /// object-store S3) does not declare it.
    ChainedSearch,
    /// Reverse chaining (_has).
    ///
    /// Same basis as [`ChainedSearch`](BackendCapability::ChainedSearch): a
    /// deployment resolves `_has` correctly, natively or via the shared
    /// resolver, so any search-capable backend that honours `_id` declares it.
    ReverseChaining,
    /// `_include` support — resolved either inline by the backend's `search()`
    /// or by the persistence/REST layer's iterative include resolver over it.
    Include,
    /// `_revinclude` support — resolved either inline by the backend's
    /// `search()` or by the persistence/REST layer's iterative resolver over it.
    Revinclude,
    /// Full-text search (_text, _content, :text).
    FullTextSearch,
    /// Terminology operations (:above, :below, :in, :not-in).
    TerminologySearch,
    /// ACID transactions.
    Transactions,
    /// Optimistic locking (If-Match).
    OptimisticLocking,
    /// Pessimistic locking.
    PessimisticLocking,
    /// Cursor-based pagination.
    CursorPagination,
    /// Offset-based pagination.
    OffsetPagination,
    /// Sorting results.
    Sorting,
    /// Bulk export operations.
    BulkExport,
    /// Synchronous Bulk Data Submit ingestion (`BulkSubmitProvider`).
    BulkSubmitIngest,
    /// Full `$bulk-submit` REST worker/job-store support.
    BulkSubmitRestWorker,
    /// Shared-schema multitenancy: every tenant's records live in one shared
    /// table/collection/index, separated by a `tenant_id` discriminator that
    /// every query filters on.
    ///
    /// One of three mutually exclusive tenant-placement topologies — see
    /// [`SchemaPerTenant`](BackendCapability::SchemaPerTenant) and
    /// [`DatabasePerTenant`](BackendCapability::DatabasePerTenant). A
    /// conforming instance declares **exactly one** of the three.
    ///
    /// These variants describe *where records live*, not how strongly the
    /// boundary is enforced. In particular `SharedSchema` implies **no**
    /// row-level security and no database-enforced tenant boundary; isolation
    /// rests entirely on every query carrying the discriminator.
    ///
    /// Not to be confused with [`crate::tenant::TenancyModel`], which is a
    /// different axis: whether a given *resource type* is shared across
    /// tenants (terminology) or tenant-scoped (clinical data).
    SharedSchema,
    /// Schema-per-tenant multitenancy: each tenant gets a distinct named
    /// namespace (schema) inside one database, with the boundary enforced by
    /// the database's own namespace and permission model.
    ///
    /// Mutually exclusive with [`SharedSchema`](BackendCapability::SharedSchema)
    /// and [`DatabasePerTenant`](BackendCapability::DatabasePerTenant).
    ///
    /// No backend in this crate declares this today; PostgreSQL in particular
    /// issues no `CREATE SCHEMA` and no `SET search_path` for tenancy.
    SchemaPerTenant,
    /// Database-per-tenant multitenancy: each tenant gets a distinct physical
    /// database or storage container with its own connection, credential, and
    /// policy surface. A dedicated S3 bucket per tenant qualifies.
    ///
    /// Mutually exclusive with [`SharedSchema`](BackendCapability::SharedSchema)
    /// and [`SchemaPerTenant`](BackendCapability::SchemaPerTenant).
    ///
    /// An S3 backend configured for `S3TenancyMode::BucketPerTenant` is the only
    /// configuration in this crate that declares it. No SQL backend does.
    DatabasePerTenant,
    /// Backend can compile ViewDefinitions to an in-database query and run them
    /// in-DB with no in-process FHIRPath eval — SQL for the SQLite and
    /// PostgreSQL backends, an aggregation pipeline for MongoDB. A backend whose
    /// `sof_runner()` falls back to in-process evaluation (e.g. S3) does not
    /// declare it. The compiler may reject individual ViewDefinitions as
    /// uncompilable; the capability asserts the in-DB path exists and is wired,
    /// not that every ViewDefinition compiles.
    InDbSofRunner,
    /// Backend supports raw SQL queries via `$sql-query-run` (Postgres, SQLite only).
    RawSqlQuery,
}

impl std::fmt::Display for BackendCapability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            BackendCapability::Crud => "crud",
            BackendCapability::Versioning => "versioning",
            BackendCapability::InstanceHistory => "instance-history",
            BackendCapability::TypeHistory => "type-history",
            BackendCapability::SystemHistory => "system-history",
            BackendCapability::BasicSearch => "basic-search",
            BackendCapability::DateSearch => "date-search",
            BackendCapability::QuantitySearch => "quantity-search",
            BackendCapability::ReferenceSearch => "reference-search",
            BackendCapability::ChainedSearch => "chained-search",
            BackendCapability::ReverseChaining => "reverse-chaining",
            BackendCapability::Include => "include",
            BackendCapability::Revinclude => "revinclude",
            BackendCapability::FullTextSearch => "full-text-search",
            BackendCapability::TerminologySearch => "terminology-search",
            BackendCapability::Transactions => "transactions",
            BackendCapability::OptimisticLocking => "optimistic-locking",
            BackendCapability::PessimisticLocking => "pessimistic-locking",
            BackendCapability::CursorPagination => "cursor-pagination",
            BackendCapability::OffsetPagination => "offset-pagination",
            BackendCapability::Sorting => "sorting",
            BackendCapability::BulkExport => "bulk-export",
            BackendCapability::BulkSubmitIngest => "bulk-submit-ingest",
            BackendCapability::BulkSubmitRestWorker => "bulk-submit-rest-worker",
            BackendCapability::SharedSchema => "shared-schema",
            BackendCapability::SchemaPerTenant => "schema-per-tenant",
            BackendCapability::DatabasePerTenant => "database-per-tenant",
            BackendCapability::InDbSofRunner => "indb-sof-runner",
            BackendCapability::RawSqlQuery => "raw-sql-query",
        };
        write!(f, "{}", name)
    }
}

/// Configuration for a database backend.
#[derive(Debug, Clone)]
pub struct BackendConfig {
    /// Connection string or URL.
    pub connection_string: String,
    /// Maximum number of connections in the pool.
    pub max_connections: u32,
    /// Minimum number of idle connections.
    pub min_connections: u32,
    /// Connection timeout in milliseconds.
    pub connect_timeout_ms: u64,
    /// Idle connection timeout in milliseconds.
    pub idle_timeout_ms: Option<u64>,
    /// Maximum connection lifetime in milliseconds.
    pub max_lifetime_ms: Option<u64>,
}

impl Default for BackendConfig {
    fn default() -> Self {
        Self {
            connection_string: String::new(),
            max_connections: 10,
            min_connections: 1,
            connect_timeout_ms: 5000,
            idle_timeout_ms: Some(600_000),   // 10 minutes
            max_lifetime_ms: Some(1_800_000), // 30 minutes
        }
    }
}

impl BackendConfig {
    /// Creates a new configuration with the given connection string.
    pub fn new(connection_string: impl Into<String>) -> Self {
        Self {
            connection_string: connection_string.into(),
            ..Default::default()
        }
    }

    /// Sets the maximum number of connections.
    pub fn with_max_connections(mut self, max: u32) -> Self {
        self.max_connections = max;
        self
    }

    /// Sets the connection timeout.
    pub fn with_connect_timeout_ms(mut self, timeout: u64) -> Self {
        self.connect_timeout_ms = timeout;
        self
    }
}

/// A database backend that can execute storage operations.
///
/// This trait is inspired by Diesel's `Backend` trait, providing a common
/// abstraction over different database drivers. Each backend implementation
/// provides its own connection type and query builder.
///
/// # Design
///
/// The `Backend` trait is designed to be object-safe where possible, allowing
/// for dynamic dispatch in composite storage scenarios. However, some operations
/// require associated types for type safety.
///
/// # Example
///
/// ```ignore
/// use helios_persistence::core::{Backend, BackendKind, BackendCapability};
///
/// // Check backend capabilities at runtime
/// if backend.supports(BackendCapability::ChainedSearch) {
///     // Use chained search
/// } else {
///     // Fall back to multiple queries
/// }
/// ```
#[async_trait]
pub trait Backend: Send + Sync + Debug {
    /// The type of raw connection used by this backend.
    type Connection: Send;

    /// Returns the kind of backend.
    fn kind(&self) -> BackendKind;

    /// Returns a human-readable name for this backend.
    fn name(&self) -> &'static str;

    /// Checks whether **this instance, as configured** supports the given
    /// capability.
    ///
    /// See [`BackendCapability`] for what a declared capability asserts. Both
    /// this method and [`capabilities`](Backend::capabilities) must be derived
    /// from a single declaration per backend, so the two answers cannot drift
    /// apart.
    fn supports(&self, capability: BackendCapability) -> bool;

    /// Returns every capability **this instance, as configured** supports.
    ///
    /// A backend may additionally expose a constructor-free associated
    /// `declared_capabilities()` accessor, but only when its capability set is
    /// invariant across all valid configurations. `S3Backend` is the
    /// counterexample: its tenant-placement topology depends on the configured
    /// `S3TenancyMode`, so it exposes a mode-parameterised
    /// `declared_capabilities_for(&S3TenancyMode)` instead.
    fn capabilities(&self) -> Vec<BackendCapability>;

    /// Acquires a connection from the pool.
    async fn acquire(&self) -> Result<Self::Connection, BackendError>;

    /// Returns the connection back to the pool.
    async fn release(&self, conn: Self::Connection);

    /// Checks if the backend is healthy and accepting connections.
    async fn health_check(&self) -> Result<(), BackendError>;

    /// Initializes the database schema if needed.
    async fn initialize(&self) -> Result<(), BackendError>;

    /// Runs any pending migrations.
    async fn migrate(&self) -> Result<(), BackendError>;
}

/// Extension trait for backends that support connection pooling statistics.
pub trait BackendPoolStats {
    /// Returns the current number of active connections.
    fn active_connections(&self) -> u32;

    /// Returns the current number of idle connections.
    fn idle_connections(&self) -> u32;

    /// Returns the maximum pool size.
    fn max_connections(&self) -> u32;

    /// Returns the number of connections waiting to be acquired.
    fn pending_connections(&self) -> u32;
}

/// Marker trait for backends that support ACID transactions.
pub trait TransactionalBackend: Backend {}

/// Marker trait for backends that support full-text search.
pub trait FullTextBackend: Backend {}

/// Marker trait for backends optimized for graph queries.
pub trait GraphBackend: Backend {}

/// Marker trait for backends optimized for time-series data.
pub trait TimeSeriesBackend: Backend {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backend_kind_display() {
        assert_eq!(BackendKind::Sqlite.to_string(), "sqlite");
        assert_eq!(BackendKind::Postgres.to_string(), "postgres");
        assert_eq!(BackendKind::Custom("custom-db").to_string(), "custom-db");
    }

    #[test]
    fn test_backend_capability_display() {
        assert_eq!(BackendCapability::Crud.to_string(), "crud");
        assert_eq!(
            BackendCapability::ChainedSearch.to_string(),
            "chained-search"
        );
        assert_eq!(
            BackendCapability::FullTextSearch.to_string(),
            "full-text-search"
        );
        assert_eq!(
            BackendCapability::BulkSubmitIngest.to_string(),
            "bulk-submit-ingest"
        );
        assert_eq!(
            BackendCapability::BulkSubmitRestWorker.to_string(),
            "bulk-submit-rest-worker"
        );
    }

    #[test]
    fn test_backend_config_default() {
        let config = BackendConfig::default();
        assert_eq!(config.max_connections, 10);
        assert_eq!(config.min_connections, 1);
        assert_eq!(config.connect_timeout_ms, 5000);
    }

    #[test]
    fn test_backend_config_builder() {
        let config = BackendConfig::new("postgres://localhost/db")
            .with_max_connections(20)
            .with_connect_timeout_ms(10000);

        assert_eq!(config.connection_string, "postgres://localhost/db");
        assert_eq!(config.max_connections, 20);
        assert_eq!(config.connect_timeout_ms, 10000);
    }
}
