# helios-persistence

[Polyglot persistence layer](https://github.com/HeliosSoftware/hfs/discussions/28) for the Helios FHIR Server.

## Overview

Traditional FHIR server implementations force all resources into a single database technology, creating inevitable trade-offs. A patient lookup by identifier, a population cohort query, relationship traversals through care teams, and semantic similarity searches for clinical trial matching all have fundamentally different performance characteristics, yet they're typically crammed into one system optimized for none of them.

**Polyglot persistence** is an architectural approach where different types of data and operations are routed to the storage technologies best suited for how that data will be accessed. Rather than accepting compromise, this pattern leverages specialized storage systems optimized for specific workloads:

| Workload               | Optimal Technology | Why                                          |
| ---------------------- | ------------------ | -------------------------------------------- |
| ACID transactions      | PostgreSQL         | Strong consistency guarantees                |
| Document storage       | MongoDB            | Natural alignment with FHIR's resource model |
| Relationship traversal | Neo4j              | Efficient graph queries for references       |
| Full-text search       | Elasticsearch      | Optimized inverted indexes                   |
| Semantic search        | Vector databases   | Embedding similarity for clinical matching   |
| Bulk analytics & ML    | Object Storage     | Cost-effective columnar storage              |

## Polyglot Query Example

Consider a complex clinical query that combines multiple access patterns:

```
GET /Observation?patient.name:contains=smith&_text=cardiac&code:below=http://loinc.org|8867-4&_include=Observation:patient
```

This query requires:

1. **Chained search** (`patient.name:contains=smith`) - Find observations where the referenced patient's name contains "smith"
2. **Full-text search** (`_text=cardiac`) - Search narrative text for "cardiac"
3. **Terminology subsumption** (`code:below=LOINC|8867-4`) - Find codes that are descendants of heart rate
4. **Reference resolution** (`_include=Observation:patient`) - Include the referenced Patient resources

In a polyglot architecture, the `CompositeStorage` routes each component to its optimal backend:

```rust
// Conceptual flow - CompositeStorage coordinates backends
async fn search(&self, query: SearchQuery) -> SearchResult {
    // 1. Route chained search to graph database (efficient traversal)
    let patient_refs = self.neo4j.find_patients_by_name("smith").await?;

    // 2. Route full-text to Elasticsearch (optimized inverted index)
    let text_matches = self.elasticsearch.text_search("cardiac").await?;

    // 3. Route terminology query to terminology service + primary store
    let code_matches = self.postgres.codes_below("8867-4").await?;

    // 4. Intersect results and fetch from primary storage
    let observation_ids = intersect(patient_refs, text_matches, code_matches);
    let observations = self.postgres.batch_read(observation_ids).await?;

    // 5. Resolve _include from primary storage
    let patients = self.postgres.resolve_references(&observations, "patient").await?;

    SearchResult { resources: observations, included: patients }
}
```

No single database excels at all four operations. PostgreSQL would struggle with the graph traversal, Neo4j isn't optimized for full-text search, and Elasticsearch can't efficiently handle terminology hierarchies. Polyglot persistence lets each system do what it does best.

## Architecture

```
helios-persistence/
├── src/
│   ├── lib.rs           # Main entry point and re-exports
│   ├── error.rs         # Comprehensive error types
│   ├── tenant/          # Multitenancy support
│   │   ├── id.rs        # Hierarchical TenantId
│   │   ├── context.rs   # TenantContext (required for all operations)
│   │   ├── permissions.rs # Fine-grained TenantPermissions
│   │   └── tenancy.rs   # TenancyModel configuration
│   ├── types/           # Core domain types
│   │   ├── stored_resource.rs    # Resource with persistence metadata
│   │   ├── search_params.rs      # Full FHIR search parameter model
│   │   ├── search_capabilities.rs # Search capability reporting
│   │   └── pagination.rs         # Cursor and offset pagination
│   ├── core/            # Storage trait hierarchy
│   │   ├── backend.rs      # Backend abstraction with capabilities
│   │   ├── storage.rs      # ResourceStorage (CRUD)
│   │   ├── versioned.rs    # VersionedStorage (vread, If-Match)
│   │   ├── history.rs      # History providers (instance/type/system)
│   │   ├── search.rs       # Search providers (basic, chained, include)
│   │   ├── transaction.rs  # ACID transactions with bundle support
│   │   ├── capabilities.rs # Runtime capability discovery
│   │   ├── bulk_export.rs  # FHIR Bulk Data Export job/data traits
│   │   ├── bulk_export_output.rs # ExportOutputStore trait
│   │   ├── bulk_export_worker.rs # Bulk export worker runtime and leasing traits
│   │   └── bulk_submit.rs  # FHIR Bulk Submit traits
│   ├── search/          # Search parameter infrastructure
│   │   ├── registry.rs     # SearchParameterRegistry (in-memory cache)
│   │   ├── loader.rs       # SearchParameterLoader (R4 standard params)
│   │   ├── extractor.rs    # FHIRPath-based value extraction
│   │   ├── converters.rs   # Type conversion utilities
│   │   ├── writer.rs       # Search index writer
│   │   ├── reindex.rs      # Reindexing operations
│   │   └── errors.rs       # Search-specific error types
│   ├── strategy/        # Tenancy isolation strategies
│   │   ├── shared_schema.rs       # tenant_id column + optional RLS
│   │   ├── schema_per_tenant.rs   # PostgreSQL search_path isolation
│   │   └── database_per_tenant.rs # Complete database isolation
│   ├── backends/        # Backend implementations
│   │   ├── sqlite/      # Reference implementation (complete)
│   │   │   ├── backend.rs      # SqliteBackend with connection pooling
│   │   │   ├── storage.rs      # ResourceStorage implementation
│   │   │   ├── transaction.rs  # TransactionProvider implementation
│   │   │   ├── schema.rs       # Schema migrations (v1-v6)
│   │   │   ├── search_impl.rs  # SearchProvider implementation
│   │   │   ├── bulk_export.rs  # BulkExportStorage implementation
│   │   │   ├── bulk_submit.rs  # BulkSubmitProvider implementation
│   │   │   └── search/         # Search query building
│   │   │       ├── query_builder.rs      # SQL query construction
│   │   │       ├── chain_builder.rs      # Chained parameter resolution
│   │   │       ├── filter_parser.rs      # _filter parameter parsing
│   │   │       ├── fts.rs                # FTS5 full-text search
│   │   │       ├── modifier_handlers.rs  # Search modifier logic
│   │   │       ├── strategy.rs           # Query strategy selection
│   │   │       ├── writer.rs             # Index writing
│   │   │       └── parameter_handlers/   # Type-specific handlers
│   │   │           ├── string.rs, token.rs, date.rs, number.rs
│   │   │           ├── quantity.rs, reference.rs, uri.rs, composite.rs
│   │   ├── postgres/       # PostgreSQL primary backend
│   │   │   ├── backend.rs      # PostgresBackend with connection pooling
│   │   │   ├── storage.rs      # ResourceStorage implementation
│   │   │   ├── transaction.rs  # TransactionProvider implementation
│   │   │   ├── schema.rs       # Schema DDL with migrations
│   │   │   ├── search_impl.rs  # SearchProvider implementation
│   │   │   ├── bulk_export.rs  # BulkExportStorage implementation
│   │   │   ├── bulk_submit.rs  # BulkSubmitProvider implementation
│   │   │   └── search/         # Search query building
│   │   │       ├── query_builder.rs  # SQL with $N params, ILIKE, TIMESTAMPTZ
│   │   │       └── writer.rs        # Search index writer
│   │   ├── mongodb/       # MongoDB primary backend
│   │   │   ├── backend.rs      # MongoBackend + MongoBackendConfig
│   │   │   ├── schema.rs       # Schema/index bootstrap helpers
│   │   │   ├── search_impl.rs  # SearchProvider implementation
│   │   │   ├── storage.rs      # ResourceStorage/history/versioning implementation
│   │   │   └── mod.rs          # Module wiring and re-exports
│   │   ├── elasticsearch/  # Search-optimized secondary backend
│   │   │   ├── backend.rs      # ElasticsearchBackend with config
│   │   │   ├── storage.rs      # ResourceStorage for sync support
│   │   │   ├── schema.rs       # Index mappings and templates
│   │   │   ├── search_impl.rs  # SearchProvider, TextSearchProvider
│   │   │   └── search/         # ES Query DSL translation
│   │   │       ├── query_builder.rs      # FHIR SearchQuery → ES Query DSL
│   │   │       ├── fts.rs                # Full-text search queries
│   │   │       ├── modifier_handlers.rs  # :missing and other modifiers
│   │   │       └── parameter_handlers/   # Type-specific handlers
│   │   │           ├── string.rs, token.rs, date.rs, number.rs
│   │   │           ├── quantity.rs, reference.rs, uri.rs, composite.rs
│   │   └── s3/               # AWS S3 object-storage backend
│   │       ├── backend.rs        # S3Backend with connection management
│   │       ├── config.rs         # S3BackendConfig, S3TenancyMode
│   │       ├── client.rs         # S3Api trait and AwsS3Client implementation
│   │       ├── keyspace.rs       # S3Keyspace key-path generation
│   │       ├── models.rs         # HistoryIndexEvent, SubmissionState
│   │       ├── storage.rs        # ResourceStorage implementation
│   │       ├── bundle.rs         # Batch/transaction bundle processing
│   │       ├── bulk_export.rs    # ExportDataProvider implementation
│   │       ├── output_store.rs   # S3OutputStore for bulk export files
│   │       ├── bulk_submit.rs    # BulkSubmitProvider implementation
│   │       └── tests.rs          # Integration tests
│   ├── composite/       # Multi-backend coordination
│   │   ├── config.rs       # CompositeConfig and builder
│   │   ├── analyzer.rs     # Query feature detection
│   │   ├── router.rs       # Query routing logic
│   │   ├── cost.rs         # Cost-based optimization
│   │   ├── merger.rs       # Result merging strategies
│   │   ├── sync.rs         # Backend synchronization
│   │   ├── health.rs       # Health monitoring
│   │   └── storage.rs      # CompositeStorage implementation
│   └── advisor/         # Configuration advisor HTTP API
│       ├── server.rs       # Axum HTTP server
│       ├── handlers.rs     # API endpoint handlers
│       ├── analysis.rs     # Configuration analysis
│       ├── suggestions.rs  # Optimization suggestions
│       └── main.rs         # Advisor binary entry point
└── tests/               # Integration tests
    ├── common/          # Shared test utilities
    │   ├── harness.rs      # Test harness setup
    │   ├── fixtures.rs     # FHIR resource fixtures
    │   ├── assertions.rs   # Custom test assertions
    │   └── capabilities.rs # Capability test helpers
    ├── crud/            # CRUD operation tests
    │   ├── create_tests.rs, read_tests.rs, update_tests.rs
    │   ├── delete_tests.rs, conditional_tests.rs
    ├── search/          # Search parameter tests
    │   ├── string_tests.rs, token_tests.rs, date_tests.rs
    │   ├── number_tests.rs, quantity_tests.rs, reference_tests.rs
    │   ├── chained_tests.rs, include_tests.rs
    │   ├── modifier_tests.rs, pagination_tests.rs
    ├── versioning/      # Version history tests
    │   ├── vread_tests.rs, history_tests.rs
    │   └── optimistic_locking_tests.rs
    ├── transactions/    # Transaction tests
    │   ├── basic_tests.rs, bundle_tests.rs, rollback_tests.rs
    ├── multitenancy/    # Tenant isolation tests
    │   ├── isolation_tests.rs, cross_tenant_tests.rs
    ├── composite_routing_tests.rs   # Query routing tests
    ├── composite_polyglot_tests.rs  # Multi-backend tests
    ├── sqlite_tests.rs              # SQLite backend tests
    ├── postgres_tests.rs            # PostgreSQL backend tests
    ├── mongodb_tests.rs             # MongoDB backend tests
    └── elasticsearch_tests.rs       # Elasticsearch backend tests
```

### Trait Hierarchy

The storage layer uses a progressive trait hierarchy inspired by Diesel:

```
Backend (connection management, capabilities)
    │
    ├── ResourceStorage (create, read, update, delete)
    │       │
    │       └── VersionedStorage (vread, update_with_match)
    │               │
    │               └── HistoryProvider (instance, type, system history)
    │
    ├── SearchProvider (search, search_count)
    │       │
    │       ├── IncludeProvider (_include resolution)
    │       ├── RevincludeProvider (_revinclude resolution)
    │       └── ChainedSearchProvider (chained parameters, _has)
    │
    └── TransactionProvider (begin, commit, rollback)
```

## Features

- **Multiple Backends**: SQLite, PostgreSQL, Cassandra, MongoDB, Neo4j, Elasticsearch, S3
- **Multitenancy**: Three isolation strategies with type-level enforcement
- **Full FHIR Search**: All parameter types, modifiers, chaining, \_include/\_revinclude
- **Versioning**: Complete resource history with optimistic locking
- **Transactions**: ACID transactions with FHIR bundle support
- **Capability Discovery**: Runtime introspection of backend capabilities

## Multitenancy

All storage operations require a `TenantContext`, ensuring tenant isolation at the type level. There is no way to bypass this requirement—the compiler enforces it.

### Tenancy Strategies

| Strategy                | Isolation                         | Use Case                                     |
| ----------------------- | --------------------------------- | -------------------------------------------- |
| **Shared Schema**       | `tenant_id` column + optional RLS | Multi-tenant SaaS with shared infrastructure |
| **Schema-per-Tenant**   | PostgreSQL schemas                | Logical isolation with shared database       |
| **Database-per-Tenant** | Separate databases                | Complete isolation for compliance            |

### Hierarchical Tenants

```rust
use helios_persistence::tenant::TenantId;

let parent = TenantId::new("acme");
let child = TenantId::new("acme/research");
let grandchild = TenantId::new("acme/research/oncology");

assert!(child.is_descendant_of(&parent));
assert!(grandchild.is_descendant_of(&parent));
assert_eq!(grandchild.root().as_str(), "acme");
```

### Permission Control

```rust
use helios_persistence::tenant::{TenantPermissions, Operation};

// Read-only access
let read_only = TenantPermissions::read_only();

// Custom permissions with compartment restrictions
let custom = TenantPermissions::builder()
    .allow_operations(vec![Operation::Read, Operation::Search])
    .allow_resource_types(vec!["Patient", "Observation"])
    .restrict_to_compartment("Patient", "123")
    .build();
```

## Search

Build search queries with full FHIR search support:

```rust
use helios_persistence::types::{
    SearchQuery, SearchParameter, SearchParamType, SearchValue,
    SearchModifier, SortDirective, IncludeDirective, IncludeType,
};

// Simple search
let query = SearchQuery::new("Patient")
    .with_parameter(SearchParameter {
        name: "name".to_string(),
        param_type: SearchParamType::String,
        modifier: Some(SearchModifier::Contains),
        values: vec![SearchValue::eq("smith")],
        chain: vec![],
    })
    .with_sort(SortDirective::parse("-_lastUpdated"))
    .with_count(20);

// With _include
let query_with_include = SearchQuery::new("Observation")
    .with_include(IncludeDirective {
        include_type: IncludeType::Include,
        source_type: "Observation".to_string(),
        search_param: "patient".to_string(),
        target_type: Some("Patient".to_string()),
        iterate: false,
    });
```

## Backend Capability Matrix

The matrix below shows which FHIR operations each backend supports. This reflects the actual implementation status, not aspirational goals.

> **Note:** Documentation links reference [build.fhir.org](https://build.fhir.org), which contains the current FHIR development version. Some features marked as planned are new and may be labeled "Trial Use" in the specification.

**Legend:** ✓ Implemented | ◐ Partial | ○ Planned | ✗ Not planned | † Requires external service
| Feature                                                                     | SQLite | PostgreSQL | MongoDB | Cassandra | Neo4j | Elasticsearch | S3  |
| --------------------------------------------------------------------------- | ------ | ---------- | ------- | --------- | ----- | ------------- | --- |
| **Core Operations**                                                         |
| [CRUD](https://build.fhir.org/http.html#crud)                               | ✓      | ✓          | ✓       | ○         | ○     | ✓             | ✓   |
| [Versioning (vread)](https://build.fhir.org/http.html#vread)                | ✓      | ✓          | ✓       | ○         | ○     | ○             | ✓   |
| [Optimistic Locking](https://build.fhir.org/http.html#concurrency)          | ✓      | ✓          | ✓       | ○         | ○     | ✗             | ✓   |
| [Instance History](https://build.fhir.org/http.html#history)                | ✓      | ✓          | ○       | ✗         | ○     | ✗             | ✓   |
| [Type History](https://build.fhir.org/http.html#history)                    | ✓      | ✓          | ○       | ✗         | ○     | ✗             | ✓   |
| [System History](https://build.fhir.org/http.html#history)                  | ✓      | ✓          | ○       | ✗         | ○     | ✗             | ✓   |
| [Batch Bundles](https://build.fhir.org/http.html#batch)                     | ✓      | ✓          | ✓       | ○         | ○     | ○             | ✓   |
| [Transaction Bundles](https://build.fhir.org/http.html#transaction)         | ✓      | ✓          | ✓       | ✗         | ○     | ✗             | ◐   |
| [Conditional Operations](https://build.fhir.org/http.html#cond-update)      | ✓      | ✓          | ✓       | ✗         | ○     | ○             | ✗   |
| [Conditional Patch](https://build.fhir.org/http.html#patch)                 | ✓      | ✓          | ○       | ✗         | ○     | ○             | ✗   |
| [Delete History](https://build.fhir.org/http.html#delete)                   | ✓      | ✓          | ○       | ✗         | ○     | ✗             | ✗   |
| **Multitenancy**                                                            |
| Shared Schema                                                               | ✓      | ✓          | ✓       | ○         | ○     | ✓             | ✓   |
| Schema-per-Tenant                                                           | ✗      | ○          | ○       | ✗         | ✗     | ✗             | ✗   |
| Database-per-Tenant                                                         | ✓      | ○          | ○       | ○         | ○     | ○             | ✓   |
| Row-Level Security                                                          | ✗      | ○          | ✗       | ✗         | ✗     | ✗             | ✗   |
| **[Search Parameters](https://build.fhir.org/search.html#ptypes)**          |
| [String](https://build.fhir.org/search.html#string)                         | ✓      | ✓          | ✓       | ✗         | ○     | ✓             | ✗   |
| [Token](https://build.fhir.org/search.html#token)                           | ✓      | ✓          | ✓       | ○         | ○     | ✓             | ✗   |
| [Reference](https://build.fhir.org/search.html#reference)                   | ✓      | ✓          | ✓       | ✗         | ○     | ✓             | ✗   |
| [Date](https://build.fhir.org/search.html#date)                             | ✓      | ✓          | ✓       | ○         | ○     | ✓             | ○   |
| [Number](https://build.fhir.org/search.html#number)                         | ✓      | ✓          | ✓       | ✗         | ○     | ✓             | ○   |
| [Quantity](https://build.fhir.org/search.html#quantity)                     | ✓      | ✓          | ○       | ✗         | ✗     | ✓             | ○   |
| [URI](https://build.fhir.org/search.html#uri)                               | ✓      | ✓          | ✓       | ○         | ○     | ✓             | ○   |
| [Composite](https://build.fhir.org/search.html#composite)                   | ✓      | ○          | ○       | ✗         | ○     | ✓             | ✗   |
| **[Search Modifiers](https://build.fhir.org/search.html#modifiers)**        |
| [:exact](https://build.fhir.org/search.html#modifiers)                      | ✓      | ✓          | ○       | ○         | ○     | ✓             | ○   |
| [:contains](https://build.fhir.org/search.html#modifiers)                   | ✓      | ✓          | ○       | ✗         | ○     | ✓             | ✗   |
| [:text](https://build.fhir.org/search.html#modifiers) (full-text)           | ✓      | ◐          | ○       | ✗         | ✗     | ✓             | ✗   |
| [:not](https://build.fhir.org/search.html#modifiers)                        | ✓      | ○          | ○       | ✗         | ○     | ✓             | ○   |
| [:missing](https://build.fhir.org/search.html#modifiers)                    | ✓      | ○          | ○       | ✗         | ○     | ✓             | ○   |
| [:above / :below](https://build.fhir.org/search.html#modifiers)             | ✗      | †○         | †○      | ✗         | ○     | ✓             | ✗   |
| [:in / :not-in](https://build.fhir.org/search.html#modifiers)               | ✗      | †○         | †○      | ✗         | ○     | †○            | ✗   |
| [:of-type](https://build.fhir.org/search.html#modifiers)                    | ✓      | ○          | ○       | ✗         | ○     | ✓             | ✗   |
| [:text-advanced](https://build.fhir.org/search.html#modifiertextadvanced)   | ✓      | †○         | †○      | ✗         | ✗     | ✓             | ✗   |
| **[Special Parameters](https://build.fhir.org/search.html#all)**            |
| [\_text](https://build.fhir.org/search.html#_text) (narrative search)       | ✓      | ◐          | ○       | ✗         | ✗     | ✓             | ✗   |
| [\_content](https://build.fhir.org/search.html#_content) (full content)     | ✓      | ◐          | ○       | ✗         | ✗     | ✓             | ✗   |
| [\_filter](https://build.fhir.org/search.html#_filter) (advanced filtering) | ✓      | ○          | ○       | ✗         | ○     | ○             | ✗   |
| **Advanced Search**                                                         |
| [Chained Parameters](https://build.fhir.org/search.html#chaining)           | ✓      | ◐          | ○       | ✗         | ○     | ✗             | ✗   |
| [Reverse Chaining (\_has)](https://build.fhir.org/search.html#has)          | ✓      | ◐          | ○       | ✗         | ○     | ✗             | ✗   |
| [\_include](https://build.fhir.org/search.html#include)                     | ✓      | ✓          | ✓       | ✗         | ○     | ✓             | ✗   |
| [\_revinclude](https://build.fhir.org/search.html#revinclude)               | ✓      | ✓          | ✓       | ✗         | ○     | ✓             | ✗   |
| **[Pagination](https://build.fhir.org/http.html#paging)**                   |
| Offset                                                                      | ✓      | ✓          | ✓       | ✗         | ○     | ✓             | ✗   |
| Cursor (keyset)                                                             | ✓      | ✓          | ✓       | ○         | ○     | ✓             | ○   |
| **[Sorting](https://build.fhir.org/search.html#sort)**                      |
| Single field                                                                | ✓      | ✓          | ✓       | ✗         | ○     | ✓             | ✗   |
| Multiple fields                                                             | ✓      | ✓          | ✓       | ✗         | ○     | ✓             | ✗   |
| **[Bulk Operations](https://hl7.org/fhir/uv/bulkdata/)**                    |
| [Bulk Export](https://hl7.org/fhir/uv/bulkdata/export.html)                 | ✓      | ✓          | ○       | ○         | ○     | ○             | ◐   |
| [Bulk Submit](https://hackmd.io/@argonaut/rJoqHZrPle)                       | ✓      | ✓          | ○       | ○         | ○     | ○             | ✓   |

The S3 backend is intentionally storage-focused (CRUD/version/history/bulk submit) and does not act as a full FHIR search engine. For bulk export, S3 can feed system-level batches through `ExportDataProvider` and can store output files through `S3OutputStore`, but job state belongs to SQLite or PostgreSQL. Patient-level and Group-level export compartment enumeration are not supported by S3 as the resource store. For query-heavy deployments, use a DB/search backend as primary query engine and compose S3 as archive/history/output storage.

### Primary/Secondary Role Matrix

Backends can serve as primary (CRUD, versioning, transactions) or secondary (optimized for specific query patterns). When a secondary search backend is configured, the primary backend's search indexing is automatically disabled to avoid data duplication.

| Configuration              | Primary    | Secondary              | Status                          | Use Case                                |
| -------------------------- | ---------- | ---------------------- | ------------------------------- | --------------------------------------- |
| SQLite alone               | SQLite     | —                      | ✓ Implemented                   | Development, testing, small deployments |
| SQLite + Elasticsearch     | SQLite     | Elasticsearch (search) | ✓ Implemented                   | Small prod with robust search           |
| PostgreSQL alone           | PostgreSQL | —                      | ✓ Implemented                   | Production OLTP                         |
| PostgreSQL + Elasticsearch | PostgreSQL | Elasticsearch (search) | ✓ Implemented                   | OLTP + advanced search                  |
| PostgreSQL + Neo4j         | PostgreSQL | Neo4j (graph)          | Planned                         | Graph-heavy queries                     |
| Cassandra alone            | Cassandra  | —                      | Planned                         | High write throughput                   |
| Cassandra + Elasticsearch  | Cassandra  | Elasticsearch (search) | Planned                         | Write-heavy + search                    |
| MongoDB alone              | MongoDB    | —                      | ✓ Implemented                   | Document-centric                        |
| MongoDB + Elasticsearch    | MongoDB    | Elasticsearch (search) | ✓ Implemented                   | Document-centric + offloaded search     |
| S3 alone                   | S3         | —                      | ✓ Implemented (storage-focused) | Archival/history storage                |
| S3 + Elasticsearch         | S3         | Elasticsearch (search) | ✓ Implemented                   | Large-scale + search                    |

### Backend Selection Guide

| Use Case              | Recommended Backend | Rationale                                  |
| --------------------- | ------------------- | ------------------------------------------ |
| Development & Testing | SQLite              | Zero configuration, in-memory mode         |
| Production OLTP       | PostgreSQL          | ACID transactions, JSONB, mature ecosystem |
| Document-centric      | MongoDB             | Natural FHIR alignment, flexible schema    |
| Graph queries         | Neo4j               | Efficient relationship traversal           |
| Full-text search      | Elasticsearch       | Optimized inverted indexes, analyzers      |
| Bulk analytics        | S3 + Parquet        | Cost-effective, columnar, ML-ready         |
| High write throughput | Cassandra           | Distributed writes, eventual consistency   |

### Feature Flags

| Feature            | Description                 | Driver         |
| ------------------ | --------------------------- | -------------- |
| `sqlite` (default) | SQLite (in-memory and file) | rusqlite       |
| `postgres`         | PostgreSQL with JSONB       | tokio-postgres |
| `cassandra`        | Apache Cassandra            | cdrs-tokio     |
| `mongodb`          | MongoDB document store      | mongodb        |
| `neo4j`            | Neo4j graph database        | neo4rs         |
| `elasticsearch`    | Elasticsearch search        | elasticsearch  |
| `s3`               | AWS S3 object storage       | aws-sdk-s3     |

## Building & Running Storage Backends

This section covers building the `hfs` binary with specific backend support and setting up the required infrastructure.

### SQLite (Default)

Zero-configuration setup — no external dependencies required.

```bash
# Build with default SQLite backend
cargo build --bin hfs --release

# Run
./target/release/hfs
```

SQLite handles all CRUD operations, versioning, history, and search using its built-in FTS5 full-text search engine. Data is stored in `fhir.db` by default.

### SQLite + Elasticsearch

SQLite handles CRUD, versioning, history, and transactions. Elasticsearch handles all search operations with:

- Full-text search with relevance scoring (`_text`, `_content`)
- All FHIR search parameter types (string, token, date, number, quantity, reference, URI, composite)
- Advanced text search with stemming, boolean operators, and proximity matching (`:text-advanced`)
- Cursor-based pagination via `search_after`

**Prerequisites:** A running Elasticsearch 8.x instance.

```bash
# Build with Elasticsearch support
cargo build --bin hfs --features sqlite,elasticsearch --release

# Start Elasticsearch (example using Docker)
docker run -d --name es -p 9200:9200 \
  -e "discovery.type=single-node" \
  -e "xpack.security.enabled=false" \
  elasticsearch:8.15.0

# Start the server
HFS_STORAGE_BACKEND=sqlite-elasticsearch \
HFS_ELASTICSEARCH_NODES=http://localhost:9200 \
  ./target/release/hfs
```

### PostgreSQL

Full-featured relational backend for production deployments with JSONB storage, full-text search, and advanced multi-tenant isolation strategies.

- Full CRUD operations with ACID transactions
- Full-text search via PostgreSQL's tsvector/tsquery
- All FHIR search parameter types (string, token, date, number, quantity, reference, URI, composite)
- Chained parameters and reverse chaining (`_has`)
- `_include` and `_revinclude` resolution
- Multi-tenant support (shared schema, schema-per-tenant, database-per-tenant)

**Prerequisites:** A running PostgreSQL instance (14+).

```bash
# Build with PostgreSQL support
cargo build --bin hfs --features postgres --release

# Start PostgreSQL (example using Docker)
docker run -d --name pg -p 5432:5432 \
  -e POSTGRES_USER=hfs \
  -e POSTGRES_PASSWORD=hfs \
  -e POSTGRES_DB=fhir \
  postgres:16

# Start the server
HFS_STORAGE_BACKEND=postgres \
HFS_DATABASE_URL="postgresql://hfs:hfs@localhost:5432/fhir" \
  ./target/release/hfs
```

### PostgreSQL + Elasticsearch

PostgreSQL handles CRUD, versioning, history, and transactions with ACID guarantees. Elasticsearch handles all search operations. Combines PostgreSQL's production-grade storage with Elasticsearch's search capabilities.

- Full CRUD operations with ACID transactions via PostgreSQL
- Full-text search with relevance scoring (`_text`, `_content`) via Elasticsearch
- All FHIR search parameter types (string, token, date, number, quantity, reference, URI, composite)
- Advanced text search with stemming, boolean operators, and proximity matching (`:text-advanced`)
- Multi-tenant support (shared schema, schema-per-tenant, database-per-tenant)

**Prerequisites:** Running PostgreSQL (14+) and Elasticsearch 8.x instances.

```bash
# Build with PostgreSQL and Elasticsearch support
cargo build --bin hfs --features postgres,elasticsearch --release

# Start PostgreSQL (example using Docker)
docker run -d --name pg -p 5432:5432 \
  -e POSTGRES_USER=hfs \
  -e POSTGRES_PASSWORD=hfs \
  -e POSTGRES_DB=fhir \
  postgres:16

# Start Elasticsearch (example using Docker)
docker run -d --name es -p 9200:9200 \
  -e "discovery.type=single-node" \
  -e "xpack.security.enabled=false" \
  elasticsearch:8.15.0

# Start the server
HFS_STORAGE_BACKEND=postgres-elasticsearch \
HFS_DATABASE_URL="postgresql://hfs:hfs@localhost:5432/fhir" \
HFS_ELASTICSEARCH_NODES=http://localhost:9200 \
  ./target/release/hfs
```

### MongoDB

MongoDB provides document-centric primary storage with full FHIR capabilities including CRUD, versioning, history, search, and transactions.

- Full CRUD operations with document-native resource storage
- Versioning and history providers (`vread`, instance/type/system history)
- Transaction bundles with urn:uuid reference resolution (requires replica set)
- Native search (string, token, reference, date, number, URI parameters)
- `_include` and `_revinclude` resolution
- Conditional create, update, and delete operations
- Cursor and offset pagination with multi-field sorting
- Shared-schema multitenancy with strict tenant filtering
- Optimistic locking with ETag support

**Prerequisites:** A running MongoDB instance. Use standalone for basic deployments or replica set/sharded topology for transaction bundle support.

```bash
# Build with MongoDB support
cargo build --bin hfs --features mongodb --release

# Start MongoDB (example using Docker)
docker run -d --name mongo -p 27017:27017 \
  mongo:8.0

# Start the server
HFS_STORAGE_BACKEND=mongodb \
HFS_DATABASE_URL="mongodb://localhost:27017" \
HFS_MONGODB_DATABASE=helios \
  ./target/release/hfs
```

MongoDB runtime configuration also supports:

- `HFS_MONGODB_URL` or `HFS_MONGODB_URI` as preferred connection-string inputs
- `HFS_MONGODB_DATABASE` to select the database name (default: `helios`)
- `HFS_MONGODB_MAX_CONNECTIONS` to control the driver pool size (default: `10`)
- `HFS_MONGODB_CONNECT_TIMEOUT_MS` to control the connection timeout (default: `5000`)

### MongoDB + Elasticsearch

MongoDB remains the canonical write/read store while Elasticsearch owns delegated search execution. This mode mirrors the existing SQLite + Elasticsearch and PostgreSQL + Elasticsearch composite patterns.

- MongoDB handles CRUD, versioning, history, and conditional write behavior
- Elasticsearch handles delegated search queries, including full-text search
- MongoDB search index population is automatically disabled via `search_offloaded`
- Composite routing preserves MongoDB as the source of truth for reads and writes

**Prerequisites:** Running MongoDB and Elasticsearch 8.x instances.

```bash
# Build with MongoDB and Elasticsearch support
cargo build --bin hfs --features mongodb,elasticsearch --release

# Start MongoDB (example using Docker)
docker run -d --name mongo -p 27017:27017 \
  mongo:8.0

# Start Elasticsearch (example using Docker)
docker run -d --name es -p 9200:9200 \
  -e "discovery.type=single-node" \
  -e "xpack.security.enabled=false" \
  elasticsearch:8.15.0

# Start the server
HFS_STORAGE_BACKEND=mongodb-elasticsearch \
HFS_DATABASE_URL="mongodb://localhost:27017" \
HFS_MONGODB_DATABASE=helios \
HFS_ELASTICSEARCH_NODES=http://localhost:9200 \
  ./target/release/hfs
```

### S3 + Elasticsearch

S3 handles CRUD, versioning, history, and bulk-submit artifacts. Elasticsearch handles all search operations. For bulk export, this topology can use S3 as the resource data provider for system-level exports and `S3OutputStore` as the output-file store; export job state still lives in the configured SQLite or PostgreSQL bulk-export job store.

- CRUD persistence via S3 objects (current pointer + immutable history versions)
- Versioning (`vread`, optimistic locking via version checks)
- Instance, type, and system history via immutable history objects
- Batch bundles and best-effort transaction bundles
- Bulk export data provider for system-level exports
- Optional S3 bulk-export output files via `S3OutputStore`
- Bulk submit with rollback change log
- Full-text search with relevance scoring (`_text`, `_content`) via Elasticsearch
- All FHIR search parameter types (string, token, date, number, quantity, reference, URI, composite)
- Advanced text search with stemming, boolean operators, and proximity matching (`:text-advanced`)
- Tenant isolation (`PrefixPerTenant` or `BucketPerTenant`)

**Prerequisites:** An AWS S3 bucket (or S3-compatible service) and a running Elasticsearch 8.x instance.

```bash
# Build with S3 and Elasticsearch support
cargo build --bin hfs --features s3,elasticsearch --release

# Start Elasticsearch (example using Docker)
docker run -d --name es -p 9200:9200 \
  -e "discovery.type=single-node" \
  -e "xpack.security.enabled=false" \
  elasticsearch:8.15.0

# Start the server (AWS S3)
HFS_STORAGE_BACKEND=s3-elasticsearch \
HFS_S3_BUCKET=my-fhir-bucket \
HFS_ELASTICSEARCH_NODES=http://localhost:9200 \
  ./target/release/hfs
```

#### S3 Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `HFS_S3_BUCKET` | `hfs` | S3 bucket name |
| `HFS_S3_REGION` | (provider chain) | AWS region override |
| `HFS_S3_PREFIX` | (none) | Optional global key prefix |
| `HFS_S3_VALIDATE_BUCKETS` | `true` | Validate buckets on startup via `HeadBucket` |

AWS credentials are resolved via the standard AWS provider chain (`AWS_ACCESS_KEY_ID`/`AWS_SECRET_ACCESS_KEY`, EC2 instance metadata, shared credentials file, SSO, etc.).

#### S3-Compatible Endpoints (MinIO, etc.)

For S3-compatible services, configure the endpoint programmatically via `S3BackendConfig`:

```rust
use helios_persistence::backends::s3::{S3BackendConfig, S3TenancyMode};

let config = S3BackendConfig {
    tenancy_mode: S3TenancyMode::PrefixPerTenant {
        bucket: "minio-bucket".to_string(),
    },
    endpoint_url: Some("http://127.0.0.1:9000".to_string()),
    allow_http: true,
    force_path_style: true,
    ..Default::default()
};
```

When `endpoint_url` is set, the backend automatically defaults `force_path_style` to `true` and `region` to `us-east-1` if not otherwise specified.

#### Key Differences from SQLite/PG + ES

Unlike SQLite and PostgreSQL, the S3 backend has no built-in search parameter registry. When composing S3 + Elasticsearch, the ES backend creates its own standalone registry with minimal embedded search parameters (`_id`, `_lastUpdated`, `_tag`, `_profile`, `_security`). For full search capability, use `with_shared_registry()` with parameters loaded from spec files.

```rust
use std::collections::HashMap;
use std::sync::Arc;
use helios_persistence::backends::elasticsearch::{ElasticsearchBackend, ElasticsearchConfig};
use helios_persistence::backends::s3::{S3Backend, S3BackendConfig};
use helios_persistence::composite::{CompositeConfig, CompositeStorage, DynStorage, DynSearchProvider};
use helios_persistence::core::BackendKind;

// Create S3 backend
let s3_config = S3BackendConfig::default();
let s3 = Arc::new(S3Backend::from_env_async(s3_config).await?);

// Create ES backend (standalone registry — S3 has no registry to share)
let es_config = ElasticsearchConfig::default();
let es = Arc::new(ElasticsearchBackend::new(es_config)?);

// Build composite
let composite_config = CompositeConfig::builder()
    .primary("s3", BackendKind::S3)
    .search_backend("es", BackendKind::Elasticsearch)
    .build()?;

let mut backends = HashMap::new();
backends.insert("s3".to_string(), s3.clone() as DynStorage);
backends.insert("es".to_string(), es.clone() as DynStorage);

let mut search_providers = HashMap::new();
search_providers.insert("s3".to_string(), s3.clone() as DynSearchProvider);
search_providers.insert("es".to_string(), es.clone() as DynSearchProvider);

let composite = CompositeStorage::new(composite_config, backends)?
    .with_search_providers(search_providers)
    .with_full_primary(s3);
```

### How Search Offloading Works

When `HFS_STORAGE_BACKEND` is set to `sqlite-elasticsearch`, `postgres-elasticsearch`, `mongodb-elasticsearch`, or `s3-elasticsearch`, the server:

1. Creates the primary backend (SQLite, PostgreSQL, MongoDB, or S3). For SQLite/PG/MongoDB, search indexing is **disabled**; S3 has no search indexing to disable.
2. Creates an Elasticsearch backend. For SQLite/PG/MongoDB, it shares the primary backend's search parameter registry; for S3, it creates its own standalone registry.
3. Wraps both in a `CompositeStorage` that routes:
   - All **writes** (create, update, delete, conditional ops, transactions) → primary backend, then syncs to ES
   - All **reads** (read, vread, history) → primary backend
   - All **search** operations → Elasticsearch

This is controlled by the `search_offloaded` flag on the primary backend, which the composite layer sets automatically when a search secondary is configured.

### Composite Usage

```rust
use std::collections::HashMap;
use std::sync::Arc;
use helios_persistence::composite::{CompositeConfig, CompositeStorage, DynStorage, DynSearchProvider};
use helios_persistence::core::BackendKind;
use helios_persistence::backends::sqlite::SqliteBackend;
use helios_persistence::backends::elasticsearch::{ElasticsearchBackend, ElasticsearchConfig};

// Create backends
let mut sqlite = SqliteBackend::new("fhir.db")?;
sqlite.set_search_offloaded(true);  // Disable SQLite search indexing
let sqlite = Arc::new(sqlite);

let es = Arc::new(ElasticsearchBackend::with_shared_registry(
    ElasticsearchConfig::default(),
    sqlite.search_registry().clone(),
)?);

// Build composite
let config = CompositeConfig::builder()
    .primary("sqlite", BackendKind::Sqlite)
    .search_backend("es", BackendKind::Elasticsearch)
    .build()?;

let mut backends = HashMap::new();
backends.insert("sqlite".to_string(), sqlite.clone() as DynStorage);
backends.insert("es".to_string(), es.clone() as DynStorage);

let mut search_providers = HashMap::new();
search_providers.insert("sqlite".to_string(), sqlite.clone() as DynSearchProvider);
search_providers.insert("es".to_string(), es.clone() as DynSearchProvider);

// with_full_primary() enables delegation of ConditionalStorage, VersionedStorage,
// InstanceHistoryProvider, and BundleProvider through the composite layer.
let composite = CompositeStorage::new(config, backends)?
    .with_search_providers(search_providers)
    .with_full_primary(sqlite);
```

## S3 Backend

The S3 backend is a storage-focused persistence backend using AWS S3 object storage. It handles CRUD, versioning/history, and bulk-submit workflows but is intentionally not a FHIR search engine. For bulk export, S3 participates in two narrower roles: `S3Backend` can provide resource batches for system-level exports, and `S3OutputStore` can store finalized NDJSON output files. Bulk-export job state, progress, manifests, leases, and file metadata are not stored in S3; they live in SQLite or PostgreSQL.

### Scope

**Primary responsibilities:**
- CRUD persistence of resources
- Versioning (`vread`, `list_versions`, optimistic conflict checks)
- Instance/type/system history via immutable history objects plus history index events
- Batch bundles and best-effort transaction bundles (non-atomic with compensating rollback)
- Bulk export resource data provider for system-level exports
- Bulk export output storage through `S3OutputStore` when configured separately from job state
- Bulk submit (ingest + raw artifact persistence + rollback change log)
- Tenant isolation (`PrefixPerTenant` or `BucketPerTenant`)

**Explicit non-goals:** Advanced FHIR search semantics (date/number/quantity comparisons, chained query planning, `_has`, include/revinclude fanout, cursor keyset queries).

### Configuration

```rust
use helios_persistence::backends::s3::{S3BackendConfig, S3TenancyMode};

let config = S3BackendConfig {
    tenancy_mode: S3TenancyMode::PrefixPerTenant {
        bucket: "hfs".to_string(),
    },
    prefix: None,
    region: None,
    validate_buckets_on_startup: true,
    bulk_submit_batch_size: 100,
    ..Default::default()
};
```

| Option | Default | Description |
|--------|---------|-------------|
| `tenancy_mode` | `PrefixPerTenant { bucket: "hfs" }` | Tenant-to-bucket mapping strategy |
| `prefix` | `None` | Optional global key prefix applied before backend keys |
| `region` | `None` | AWS region override (falls back to provider chain) |
| `validate_buckets_on_startup` | `true` | Validate configured buckets with `HeadBucket` on startup |
| `bulk_submit_batch_size` | `100` | Default ingestion batch size for bulk submit processing |

### Tenancy Modes

| Mode | Description |
|------|-------------|
| **PrefixPerTenant** | All tenants share one bucket with tenant-specific key prefixes |
| **BucketPerTenant** | Each tenant maps to a specific bucket via an explicit tenant→bucket map |

### Object Model

Resource objects:

| Object | Key Pattern |
|--------|-------------|
| Current pointer | `.../resources/{type}/{id}/current.json` |
| Immutable history version | `.../resources/{type}/{id}/_history/{version}.json` |
| Type history event | `.../history/type/{type}/{ts}_{id}_{version}_{suffix}.json` |
| System history event | `.../history/system/{ts}_{type}_{id}_{version}_{suffix}.json` |

Bulk export output objects:

| Object | Key Pattern |
|--------|-------------|
| Finalized NDJSON part | `{tenant_id}/exports/{job_id}/{file_type}-{resource_type}-{part_index}-{fencing_token}.ndjson` |

Bulk-export job state is deliberately not an S3 object model. SQLite and PostgreSQL store the job row, progress, leases/fencing tokens, file metadata, and raw manifest rows. `S3OutputStore` stores only finalized output parts and deletes every object under `{tenant_id}/exports/{job_id}/` during cancellation or retention cleanup. The REST layer assembles the client-facing manifest from the job store plus `ExportOutputStore::download_url`.

Bulk submit objects:

| Object | Key Pattern |
|--------|-------------|
| Submission state | `.../bulk/submit/{submitter}/{submission_id}/state.json` |
| Manifest | `.../bulk/submit/{submitter}/{submission_id}/manifests/{manifest_id}.json` |
| Raw input | `.../bulk/submit/{submitter}/{submission_id}/raw/{manifest_id}/line-{line}.ndjson` |
| Results | `.../bulk/submit/{submitter}/{submission_id}/results/{manifest_id}/line-{line}.json` |
| Change log | `.../bulk/submit/{submitter}/{submission_id}/changes/{change_id}.json` |

### Consistency and Transaction Notes

- The backend never creates buckets — startup/runtime bucket checks use `HeadBucket` only.
- Optimistic locking relies on version checks plus S3 preconditions (`If-Match`, `If-None-Match`) where applicable.
- Transaction bundle behavior is best-effort: entries are applied sequentially, rollback is attempted in reverse order on failure, but rollback is not guaranteed under concurrent writes or partial failures.

### AWS Credentials and Region

Uses the AWS SDK for Rust ([`aws_sdk_s3`](https://docs.rs/aws-sdk-s3/latest/aws_sdk_s3/)) with standard provider chain:
- Region may be provided in config or via `AWS_REGION`
- Environment credentials (`AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, optional `AWS_SESSION_TOKEN`) are supported by provider chain behavior

## Endpoint Modes

### AWS S3 mode

Use this mode when connecting to AWS-managed S3 endpoints.

- `endpoint_url = None`
- `allow_http = false` (default)
- `force_path_style = false` (default)

```rust
use helios_persistence::backends::s3::{S3BackendConfig, S3TenancyMode};

let config = S3BackendConfig {
    tenancy_mode: S3TenancyMode::PrefixPerTenant {
        bucket: "my-aws-bucket".to_string(),
    },
    endpoint_url: None,
    force_path_style: false,
    allow_http: false,
    ..Default::default()
};
```

### S3-compatible endpoint mode (MinIO, etc.)

Use this mode for custom endpoints.

- set `endpoint_url`
- set `allow_http=true` for local `http://...` endpoints
- path-style is defaulted on when endpoint mode is active
- region falls back to `us-east-1` when not provided

```rust
use helios_persistence::backends::s3::{S3BackendConfig, S3TenancyMode};

let config = S3BackendConfig {
    tenancy_mode: S3TenancyMode::PrefixPerTenant {
        bucket: "minio-bucket".to_string(),
    },
    endpoint_url: Some("http://127.0.0.1:9000".to_string()),
    allow_http: true,
    force_path_style: true,
    ..Default::default()
};
```

Notes:

- `http://` endpoints are rejected unless `allow_http=true`.
- AWS behavior is unchanged when `endpoint_url` is not set.
- Buckets are never created by production backend code; startup validation uses `HeadBucket`.

## MinIO Integration Tests

MinIO parity tests live in:

- `crates/persistence/tests/minio_s3_tests.rs`

The suite is opt-in and env-gated:

- `RUN_MINIO_S3_TESTS=1`

Optional overrides:

- `MINIO_IMAGE` (default: `minio/minio`)
- `MINIO_TAG` (default: `RELEASE.2025-02-28T09-55-16Z`)
- `MINIO_ROOT_USER` (default: `minioadmin`)
- `MINIO_ROOT_PASSWORD` (default: `minioadmin`)
- `HFS_MINIO_TEST_BUCKET` (if unset, tests auto-generate a unique bucket)

Example:

MinIO Testing

```bash
RUN_MINIO_S3_TESTS=1 \
cargo test -p helios-persistence --features s3 --test minio_s3_tests
```

S3 Testing

Note: Make sure aws sso login is set up and running before executing S3 Testing

```bash
export RUN_AWS_S3_TESTS=1
export HFS_S3_TEST_BUCKET="your-existing-bucket-name"
export AWS_REGION="us-east-1"   # or your bucket’s region
cargo test -p helios-persistence --test s3_tests --features s3
```



## Implementation Status

### Phase 1: Core Types ✓

- [x] Error types with comprehensive variants
- [x] Tenant types (TenantId, TenantContext, TenantPermissions)
- [x] Stored resource types with versioning metadata
- [x] Search parameter types (all FHIR parameter types)
- [x] Pagination types (cursor and offset)

### Phase 2: Core Traits ✓

- [x] Backend trait with capability discovery
- [x] ResourceStorage trait (CRUD operations)
- [x] VersionedStorage trait (vread, If-Match)
- [x] History provider traits (instance, type, system)
- [x] Search provider traits (basic, chained, \_include, terminology)
- [x] Transaction traits (ACID, bundles)
- [x] Capabilities trait (CapabilityStatement generation)

### Phase 3: Tenancy Strategies ✓

- [x] Shared schema strategy with RLS support
- [x] Schema-per-tenant strategy with PostgreSQL search_path
- [x] Database-per-tenant strategy with pool management

### Phase 4: SQLite Backend ✓

- [x] Connection pooling (r2d2)
- [x] Schema migrations
- [x] ResourceStorage implementation
- [x] VersionedStorage implementation
- [x] History providers (instance, type, system)
- [x] TransactionProvider implementation
- [x] Conditional operations (conditional create/update/delete)

#### Transaction & Batch Support ◐

FHIR [transaction](https://build.fhir.org/http.html#transaction) and [batch](https://build.fhir.org/http.html#batch) bundle processing.

> **Backend Support:** Transaction bundles require ACID support. SQLite supports transactions. Cassandra, Elasticsearch, and S3 do not support transactions (batch only). See the capability matrix above.

**Implemented Features:**

- [x] **Transaction bundles** - Atomic all-or-nothing processing with automatic rollback on failure
- [x] **Batch bundles** - Independent entry processing (failures don't affect other entries)
- [x] **Processing order** - Entries processed per FHIR spec: DELETE → POST → PUT/PATCH → GET
- [x] **Reference resolution** - `urn:uuid:` references automatically resolved to assigned IDs after creates
- [x] **fullUrl support** - Track temporary identifiers for intra-bundle references
- [x] **Conditional headers** - If-Match, If-None-Match, If-None-Exist in bundle entries
- [x] **Error responses** - Transaction failures return OperationOutcome with failing entry index
- [x] **Response ordering** - Results returned in original request entry order

**Not Yet Implemented:**

| Gap                              | Description                                                          | Spec Reference                                                       |
| -------------------------------- | -------------------------------------------------------------------- | -------------------------------------------------------------------- |
| Conditional reference resolution | References like `Patient?identifier=12345` should resolve via search | [Transaction](https://build.fhir.org/http.html#trules)               |
| PATCH method                     | PATCH operations in bundle entries return 501                        | [Patch](https://build.fhir.org/http.html#patch)                      |
| Duplicate resource detection     | Same resource appearing twice in transaction should fail             | [Transaction](https://build.fhir.org/http.html#trules)               |
| Prefer header handling           | `return=minimal`, `return=representation`, `return=OperationOutcome` | [Prefer](https://build.fhir.org/http.html#return)                    |
| History bundle acceptance        | Servers SHOULD accept history bundles for replay                     | [History](https://build.fhir.org/http.html#history)                  |
| Version-specific references      | `resolve-as-version-specific` extension support                      | [References](https://build.fhir.org/http.html#trules)                |
| lastModified in response         | Bundle entry responses should include lastModified                   | [Transaction](https://build.fhir.org/http.html#transaction-response) |

#### SQLite Search Implementation ✓

The SQLite backend includes a complete FHIR search implementation using pre-computed indexes:

**Search Parameter Registry & Extraction:**

- [x] `SearchParameterRegistry` - In-memory cache of active SearchParameter definitions
- [x] `SearchParameterLoader` - Loads embedded R4 standard parameters at startup
- [x] `SearchParameterExtractor` - FHIRPath-based value extraction using `helios-fhirpath`
- [x] Dynamic SearchParameter handling - POST/PUT/DELETE to SearchParameter updates the registry

**Search Index & Query:**

- [x] Pre-computed `search_index` table for fast queries
- [x] All 8 parameter type handlers (string, token, date, number, quantity, reference, URI, composite)
- [x] Modifier support (:exact, :contains, :missing, :not, :identifier, :below, :above)
- [x] Prefix support for date/number/quantity (eq, ne, gt, lt, ge, le, sa, eb, ap)
- [x] `_include` and `_revinclude` resolution
- [x] Cursor-based and offset pagination
- [x] Single-field sorting

**Full-Text Search (FTS5):**

- [x] `resource_fts` FTS5 virtual table for full-text indexing
- [x] Narrative text extraction from `text.div` with HTML stripping
- [x] Full content extraction from all resource string values
- [x] `_text` parameter - searches narrative content
- [x] `_content` parameter - searches all resource text
- [x] `:text-advanced` modifier - advanced FTS5-based search with:
  - Porter stemming (e.g., "run" matches "running")
  - Boolean operators (AND, OR, NOT)
  - Phrase matching ("heart failure")
  - Prefix search (cardio\*)
  - Proximity matching (NEAR operator)
- [x] Porter stemmer tokenization for improved search quality
- [x] Automatic FTS indexing on resource create/update/delete

**Chained Parameters & Reverse Chaining:**

- [x] N-level forward chains (e.g., `Observation?subject.organization.name=Hospital`)
- [x] Nested reverse chains / `_has` (e.g., `Patient?_has:Observation:subject:code=1234-5`)
- [x] Type modifiers for ambiguous references (e.g., `subject:Patient.name=Smith`)
- [x] SQL-based chain resolution using efficient nested subqueries
- [x] Registry-based type inference with fallback heuristics
- [x] Configurable depth limits (default: 4, max: 8)

**Reindexing:**

- [x] `ReindexableStorage` trait for backend-agnostic reindexing
- [x] `ReindexOperation` with background task execution
- [x] Progress tracking and cancellation support
- [ ] `$reindex` HTTP endpoint (planned for server layer)

**Capability Reporting:**

- [x] `SearchCapabilityProvider` implementation
- [x] Runtime capability discovery from registry

**Bulk Operations:**

- [x] `BulkExportStorage` trait implementation (FHIR Bulk Data Access IG)
  - System-level export (`/$export`)
  - Patient-level export (`/Patient/$export`)
  - Group-level export (`/Group/[id]/$export`)
  - Job lifecycle management (pending, in-progress, completed, failed, cancelled)
  - Streaming NDJSON batch generation
  - Type filtering and \_since parameter support
- [x] `BulkSubmitProvider` trait implementation (FHIR Bulk Submit)
  - Submission lifecycle management
  - Manifest creation and management
  - Entry processing with validation
  - Rollback support for failed submissions
- [x] Schema migration v5 to v6 with 7 new tables for bulk operations

### Phase 5: Elasticsearch Backend ✓

- [x] Backend structure with connection management and health checks
- [x] Index schema and mappings (nested objects for multi-value search params)
- [x] ResourceStorage implementation for composite sync support
- [x] Search query translation (FHIR SearchQuery → ES Query DSL)
- [x] All 8 parameter type handlers (string, token, date, number, quantity, reference, URI, composite)
- [x] Full-text search (`_text`, `_content`, `:text-advanced`)
- [x] Modifier support (:exact, :contains, :text, :not, :missing, :above, :below, :of-type)
- [x] `_include` and `_revinclude` resolution
- [x] Cursor-based (`search_after`) and offset pagination
- [x] Multi-field sorting
- [x] Search offloading: when Elasticsearch is the search secondary, the primary backend skips search index population

### Phase 5b: PostgreSQL Backend ✓

- [x] Connection pooling (deadpool-postgres)
- [x] Schema migrations with JSONB storage
- [x] ResourceStorage implementation (CRUD)
- [x] VersionedStorage implementation (vread, If-Match)
- [x] History providers (instance, type, system)
- [x] TransactionProvider with configurable isolation levels
- [x] Conditional operations (conditional create/update/delete)
- [x] SearchProvider with all parameter types
- [x] ChainedSearchProvider and reverse chaining (\_has)
- [x] Full-text search (tsvector/tsquery)
- [x] `_include` and `_revinclude` resolution
- [x] BulkExportStorage and BulkSubmitProvider
- [x] Search offloading support
- [x] ReindexableStorage implementation

### Phase 5c: S3 Backend ✓

- [x] S3BackendConfig with PrefixPerTenant and BucketPerTenant tenancy modes
- [x] ResourceStorage implementation (CRUD via S3 objects)
- [x] VersionedStorage implementation (vread, optimistic locking)
- [x] History providers (instance, type, system via immutable history objects)
- [x] Batch and best-effort transaction bundles
- [x] ExportDataProvider implementation for system-level bulk export
- [x] S3OutputStore implementation for bulk-export NDJSON output files
- [x] BulkSubmitProvider implementation (ingest, raw artifacts, rollback change log)

### Phase 5+: Additional Backends (Planned)

- [ ] Cassandra backend (wide-column, partition keys)
- [x] MongoDB Phase 1 scaffold (module wiring, config, Backend trait baseline)
- [x] MongoDB Phase 2 core storage parity (CRUD/count/read_batch/create_or_update, tenant isolation, soft-delete, schema bootstrap)
- [x] MongoDB Phase 3 versioning/history plus best-effort session-backed consistency
- [x] MongoDB Phase 4 native search, pagination/sorting, and conditional create/update/delete
- [x] MongoDB Phase 5 composite MongoDB + Elasticsearch integration and runtime wiring
- [x] MongoDB Phase 6 runtime wiring verification, documentation sync, and release-readiness validation
- [ ] Neo4j backend (graph queries, Cypher)

### Phase 6: Composite Storage ✓

- [x] Query analysis and feature detection
- [x] Multi-backend coordination with primary-secondary model
- [x] Cost-based query routing
- [x] Result merging strategies
- [x] Secondary backend synchronization
- [x] Health monitoring
- [x] Configuration Advisor HTTP API
- [x] Full primary delegation via `with_full_primary()` — CompositeStorage now implements `ConditionalStorage`, `VersionedStorage`, `InstanceHistoryProvider`, and `BundleProvider` by delegating to the primary backend

## Composite Storage

The composite storage layer enables polyglot persistence by coordinating multiple database backends for optimal FHIR resource storage and querying.

### Design Principles

1. **Single Source of Truth**: One primary backend handles all FHIR resource CRUD operations, versioning, and history. This is the authoritative store.

2. **Feature-Based Routing**: Queries are automatically routed based on detected features (chained search, full-text, terminology) to appropriate backends.

3. **Eventual Consistency**: Secondary backends may lag behind primary (configurable sync/async modes with documented consistency guarantees).

4. **Graceful Degradation**: If a secondary backend is unavailable, the system falls back to primary with potentially degraded performance.

### Valid Backend Configurations

| Configuration      | Primary    | Secondary(s)  | Status        | Use Case                                |
| ------------------ | ---------- | ------------- | ------------- | --------------------------------------- |
| SQLite-only        | SQLite     | None          | ✓ Implemented | Development, testing, small deployments |
| SQLite + ES        | SQLite     | Elasticsearch | ✓ Implemented | Small prod with robust search           |
| PostgreSQL-only    | PostgreSQL | None          | ✓ Implemented | Production OLTP                         |
| PostgreSQL + ES    | PostgreSQL | Elasticsearch | ✓ Implemented | OLTP + advanced search                  |
| PostgreSQL + Neo4j | PostgreSQL | Neo4j         | Planned       | Graph-heavy queries                     |
| MongoDB-only       | MongoDB    | None          | ✓ Implemented | Document-centric primary                |
| MongoDB + ES       | MongoDB    | Elasticsearch | ✓ Implemented | Document-centric + search               |
| S3 alone           | S3         | —             | ✓ Implemented | Archival/history storage                |
| S3 + ES            | S3         | Elasticsearch | ✓ Implemented | Large-scale + search                    |

### Quick Start

```rust
use helios_persistence::composite::{
    CompositeConfigBuilder, BackendRole, SyncMode,
};
use helios_persistence::core::BackendKind;

// Development configuration (SQLite-only)
let dev_config = CompositeConfigBuilder::new()
    .primary("sqlite", BackendKind::Sqlite)
    .build()?;

// Production configuration (PostgreSQL + Elasticsearch)
let prod_config = CompositeConfigBuilder::new()
    .primary("pg", BackendKind::Postgres)
    .search_backend("es", BackendKind::Elasticsearch)
    .sync_mode(SyncMode::Asynchronous)
    .build()?;
```

### Query Routing

Queries are automatically analyzed and routed to optimal backends:

| Feature                | Detection                 | Routed To           |
| ---------------------- | ------------------------- | ------------------- |
| Basic search           | Standard parameters       | Primary             |
| Chained parameters     | `patient.name=Smith`      | Graph backend       |
| Full-text              | `_text`, `_content`       | Search backend      |
| Terminology            | `:above`, `:below`, `:in` | Terminology backend |
| Writes                 | All mutations             | Primary only        |
| \_include/\_revinclude | Include directives        | Primary             |

```rust
use helios_persistence::composite::{QueryAnalyzer, QueryFeature};
use helios_persistence::types::SearchQuery;

let analyzer = QueryAnalyzer::new();

// Analyze a complex query
let query = SearchQuery::new("Observation")
    .with_parameter(/* _text=cardiac */);

let analysis = analyzer.analyze(&query);
println!("Features: {:?}", analysis.features);
println!("Complexity: {}", analysis.complexity_score);
```

### Result Merging Strategies

When queries span multiple backends, results are merged using configurable strategies:

| Strategy              | Behavior                                       | Use Case             |
| --------------------- | ---------------------------------------------- | -------------------- |
| **Intersection**      | Results must match all backends (AND)          | Restrictive queries  |
| **Union**             | Results from any backend (OR)                  | Inclusive queries    |
| **PrimaryEnriched**   | Primary results with metadata from secondaries | Standard search      |
| **SecondaryFiltered** | Filter secondary results through primary       | Search-heavy queries |

### Synchronization Modes

| Mode             | Latency  | Consistency  | Use Case                            |
| ---------------- | -------- | ------------ | ----------------------------------- |
| **Synchronous**  | Higher   | Strong       | Critical data requiring consistency |
| **Asynchronous** | Lower    | Eventual     | Read-heavy workloads                |
| **Hybrid**       | Balanced | Configurable | Search indexes sync, others async   |

```rust
use helios_persistence::composite::SyncMode;

// Synchronous: All secondaries updated in same transaction
let sync = SyncMode::Synchronous;

// Asynchronous: Update via event stream
let async_mode = SyncMode::Asynchronous;

// Hybrid: Sync for search indexes, async for others
let hybrid = SyncMode::Hybrid { sync_for_search: true };
```

### Cost-Based Optimization

The cost estimator uses benchmark-derived costs to make routing decisions:

```rust
use helios_persistence::composite::{CostEstimator, QueryCost};
use helios_persistence::types::SearchQuery;

let estimator = CostEstimator::with_defaults();
let query = SearchQuery::new("Patient");

// Estimate cost for each backend
let costs = estimator.estimate_all(&query, &config);
for (backend_id, cost) in costs {
    println!("{}: total={}, latency={}ms",
        backend_id, cost.total, cost.estimated_latency_ms);
}

// Get cheapest backend
let best = estimator.cheapest_backend(&query, &config.backends);
```

### Health Monitoring

The health monitor tracks backend availability and triggers failover:

```rust
use helios_persistence::composite::{HealthMonitor, HealthConfig};
use std::time::Duration;

let config = HealthConfig {
    check_interval: Duration::from_secs(30),
    timeout: Duration::from_secs(5),
    failure_threshold: 3,  // Mark unhealthy after 3 failures
    success_threshold: 2,  // Mark healthy after 2 successes
};

let monitor = HealthMonitor::new(config);

// Check backend health
if monitor.is_healthy("primary") {
    // Use backend
}

// Get aggregate status
let status = monitor.all_status();
println!("Healthy: {}/{}", status.healthy_count(), status.backends.len());
```

### Configuration Advisor

The configuration advisor is an HTTP API for analyzing and optimizing composite storage configurations.

#### Running the Advisor

```bash
# Build with advisor feature
cargo build -p helios-persistence --features advisor --bin config-advisor

# Run the advisor
./target/debug/config-advisor

# With custom settings
ADVISOR_HOST=0.0.0.0 ADVISOR_PORT=9000 ./target/debug/config-advisor
```

#### API Endpoints

| Endpoint           | Method | Description                         |
| ------------------ | ------ | ----------------------------------- |
| `/health`          | GET    | Health check                        |
| `/backends`        | GET    | List available backend types        |
| `/backends/{kind}` | GET    | Get capabilities for a backend type |
| `/analyze`         | POST   | Analyze a configuration             |
| `/validate`        | POST   | Validate a configuration            |
| `/suggest`         | POST   | Get optimization suggestions        |
| `/simulate`        | POST   | Simulate query routing              |

#### Example: Analyze Configuration

```bash
curl -X POST http://localhost:8081/analyze \
  -H "Content-Type: application/json" \
  -d '{
    "config": {
      "backends": [
        {"id": "primary", "role": "Primary", "kind": "Sqlite"}
      ]
    }
  }'
```

#### Example: Get Suggestions

```bash
curl -X POST http://localhost:8081/suggest \
  -H "Content-Type: application/json" \
  -d '{
    "config": {
      "backends": [
        {"id": "primary", "role": "Primary", "kind": "Sqlite"}
      ]
    },
    "workload": {
      "read_ratio": 0.8,
      "write_ratio": 0.2,
      "fulltext_search_ratio": 0.3,
      "queries_per_day": 10000
    }
  }'
```

### Example Configurations

#### Development (SQLite-only)

```rust
let config = CompositeConfigBuilder::new()
    .primary("sqlite", BackendKind::Sqlite)
    .build()?;
```

#### Production with Full-Text Search

```rust
let config = CompositeConfigBuilder::new()
    .primary("pg", BackendKind::Postgres)
    .search_backend("es", BackendKind::Elasticsearch)
    .sync_mode(SyncMode::Asynchronous)
    .build()?;
```

#### Graph-Heavy Workloads

```rust
let config = CompositeConfigBuilder::new()
    .primary("pg", BackendKind::Postgres)
    .graph_backend("neo4j", BackendKind::Neo4j)
    .sync_mode(SyncMode::Hybrid { sync_for_search: false })
    .build()?;
```

#### Large-Scale Archival

```rust
let config = CompositeConfigBuilder::new()
    .primary("s3", BackendKind::S3)
    .search_backend("es", BackendKind::Elasticsearch)
    .sync_mode(SyncMode::Synchronous)
    .build()?;
```

### Troubleshooting

**Query not routing to expected backend:**

- Enable debug logging: `RUST_LOG=helios_persistence::composite=debug`
- Use the analyzer to inspect detected features: `analyzer.analyze(&query)`
- Check backend capabilities match required features

**High sync lag:**

- Reduce batch size in SyncConfig
- Increase sync workers
- Consider synchronous mode for critical data

**Failover not triggering:**

- Check health check interval isn't too long
- Verify failure threshold is appropriate
- Ensure failover_to targets are configured

**Cost estimates seem wrong:**

- Run Criterion benchmarks to calibrate costs
- Use `with_benchmarks()` on CostEstimator
- Check feature multipliers in CostConfig

## License

MIT
