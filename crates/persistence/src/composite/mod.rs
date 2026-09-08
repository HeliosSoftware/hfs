//! Composite Storage for Polyglot Persistence
//!
//! This module provides a composite storage layer that coordinates multiple database
//! backends for optimal FHIR resource storage and querying.
//!
//! # Overview
//!
//! Traditional FHIR server implementations force all resources into a single database
//! technology. The composite storage layer enables **polyglot persistence** where
//! different types of operations are routed to the storage technologies best suited
//! for them:
//!
//! | Operation | Optimal Backend | Why |
//! |-----------|-----------------|-----|
//! | CRUD + History | PostgreSQL/SQLite | ACID guarantees |
//! | Full-text search | Elasticsearch | Optimized inverted indexes |
//! | Terminology expansion | Terminology Service | Dedicated code hierarchies |
//! | Bulk analytics | S3 + Parquet | Cost-effective columnar storage |
//!
//! Note that [`BackendKind`](crate::core::BackendKind) also names `Cassandra`
//! and `Neo4j`, and the builder accepts them, but no backend is implemented for
//! either — see [`backends`](crate::backends). A graph role configured with
//! `BackendKind::Neo4j` cannot be instantiated at runtime.
//!
//! # Design Principles
//!
//! 1. **Single Source of Truth**: One primary backend handles all FHIR resource CRUD
//!    operations, versioning, and history. This is the authoritative store.
//!
//! 2. **Feature-Based Routing**: Queries are automatically routed based on detected
//!    features (chained search, full-text, terminology) to appropriate backends.
//!
//! 3. **Eventual Consistency**: Secondary backends may lag behind primary (configurable
//!    sync/async modes with documented consistency guarantees).
//!
//! 4. **Graceful Degradation**: If a secondary backend is unavailable, the system
//!    falls back to primary with potentially degraded performance.
//!
//! # Valid Configurations
//!
//! | Configuration | Primary | Secondary(s) | Use Case |
//! |---------------|---------|--------------|----------|
//! | SQLite-only | SQLite | None | Development, small deployments |
//! | SQLite + ES | SQLite | Elasticsearch | Production with robust text search |
//! | S3 + ES | S3 | Elasticsearch | Large-scale, cheap storage |
//! | PostgreSQL + ES | PostgreSQL | Elasticsearch | Production with robust text search |
//!
//! # Example
//!
//! ```ignore
//! use helios_persistence::composite::{
//!     CompositeConfig, CompositeConfigBuilder, BackendRole, SyncMode,
//! };
//! use helios_persistence::core::BackendKind;
//!
//! // SQLite-only (development)
//! let simple = CompositeConfig::builder()
//!     .primary("sqlite", BackendKind::Sqlite)
//!     .build()?;
//!
//! // PostgreSQL + Elasticsearch (production)
//! let production = CompositeConfig::builder()
//!     .primary("pg", BackendKind::Postgres)
//!     .search_backend("es", BackendKind::Elasticsearch)
//!     .sync_mode(SyncMode::Asynchronous)
//!     .build()?;
//! ```
//!
//! # Query Routing
//!
//! Queries are automatically routed based on detected features:
//!
//! | Feature | Detection | Routed To |
//! |---------|-----------|-----------|
//! | Basic search | Standard parameters | Primary |
//! | Chained parameters | `patient.name=Smith` | Graph |
//! | Full-text | `_text`, `_content` | Search |
//! | Terminology | `:above`, `:below`, `:in` | Terminology |
//! | Writes | All mutations | Primary only |
//!
//! # Module Structure
//!
//! - [`config`] - Configuration types and builder
//! - [`analyzer`] - Query feature detection
//! - [`router`] - Query routing logic
//! - [`storage`] - CompositeStorage implementation (Phase 2)
//! - [`merger`] - Result merging strategies (Phase 2)
//! - [`sync`] - Secondary synchronization (Phase 2)
//! - [`cost`] - Cost-based optimization (Phase 3)
//! - [`health`] - Health monitoring (Phase 3)

pub mod analyzer;
pub mod bulk_submit;
pub mod config;
pub mod cost;
pub mod health;
pub mod merger;
pub mod router;
pub mod storage;
pub mod sync;

// Re-export main types
pub use analyzer::{
    QueryAnalysis, QueryAnalyzer, QueryFeature, detect_query_features, features_to_capabilities,
};
pub use bulk_submit::CompositeSubmitJobs;
pub use config::{
    BackendEntry, BackendRole, CompositeConfig, CompositeConfigBuilder, ConfigError, ConfigWarning,
    CostConfig, CostWeights, HealthConfig, RetryConfig, RoutingRule, SyncConfig, SyncMode,
};
pub use merger::{MergeOptions, RelevanceMerger, ResultMerger, WeightedResult};
pub use router::{
    BackendType, ExecutionStep, MergeStrategy, QueryPart, QueryRouter, QueryRouting,
    RoutingDecision, RoutingError, decompose_query, route_query,
};
pub use storage::{BackendHealth, CompositeStorage, DynSearchProvider, DynStorage};
pub use sync::{
    BackendSyncStatus, ReconciliationResult, SyncEvent, SyncManager, SyncReconciler, SyncStatus,
};

// Phase 3: Cost estimation and health monitoring
pub use cost::{
    BenchmarkMeasurement, BenchmarkOperation, BenchmarkResults, CostBreakdown, CostComparison,
    CostEstimator, EstimatedCount, QueryCost,
};
pub use health::{
    BackendHealthStatus, ComponentHealth, CompositeHealthStatus, HealthCheckResponse,
    HealthCheckResult, HealthMonitor,
};
