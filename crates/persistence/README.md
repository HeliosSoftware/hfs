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
│   ├── backends/        # Backend implementations
│   │   ├── sqlite/      # Reference implementation (complete)
│   │   │   ├── backend.rs      # SqliteBackend with connection pooling
│   │   │   ├── storage.rs      # ResourceStorage implementation
│   │   │   ├── transaction.rs  # TransactionProvider implementation
│   │   │   ├── schema.rs       # Schema migrations (v1 to current)
│   │   │   ├── search_impl.rs  # SearchProvider implementation
│   │   │   ├── bulk_export.rs  # BulkExportStorage implementation
│   │   │   ├── bulk_submit.rs  # BulkSubmitProvider implementation
│   │   │   └── search/         # Search query building
│   │   │       ├── query_builder.rs      # SQL query construction
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
    │       └── RevincludeProvider (_revinclude resolution)
    │
    └── TransactionProvider (begin, commit, rollback)
```

## Features

- **Multiple Backends**: SQLite, PostgreSQL, Cassandra, MongoDB, Neo4j, Elasticsearch, S3
- **Multitenancy**: Shared-schema isolation via a `tenant_id` discriminator on every backend, with a mandatory `TenantContext` on every tenant-scoped operation (the S3 backend additionally offers a bucket-per-tenant mode, `HFS_S3_TENANCY=bucket-per-tenant` in the `hfs` binary — see S3 tenancy below)
- **Full FHIR Search**: All parameter types, modifiers, chaining, \_include/\_revinclude
- **Versioning**: Complete resource history with optimistic locking
- **Transactions**: ACID transactions with FHIR bundle support
- **Capability Discovery**: Runtime introspection of backend capabilities

## Multitenancy

Every tenant-scoped storage operation requires a `TenantContext` as its first argument, so the compiler guarantees tenant scope is *available* at the call site (a few operations are intentionally cross-tenant — `count_by_tenant`, the tenant-registry calls — and take none). This is a guarantee about call sites, not about the SQL each backend emits: the `tenant_id` predicate is hand-written per query, and the control on it is review. A database-enforced layer (PostgreSQL RLS) is tracked in [#381](https://github.com/HeliosSoftware/hfs/issues/381).

### Tenant Isolation

There is one isolation model — **shared schema with a `tenant_id`
discriminator** — chosen deliberately in [discussion #28](https://github.com/HeliosSoftware/hfs/discussions/28).
All tenants' records live in one set of tables, collections, or indices;
isolation is logical and enforced at the query level. No backend gives a tenant
its own database schema, its own database, or a Row-Level Security policy. Each
backend applies the discriminator in its own idiom:

| Backend           | How the discriminator is applied                                          |
| ----------------- | ------------------------------------------------------------------------- |
| **SQLite**        | `tenant_id` column; composite indexes lead with `tenant_id`               |
| **PostgreSQL**    | `tenant_id` column; `PRIMARY KEY (tenant_id, resource_type, id)`          |
| **MongoDB**       | `tenant_id` field on every document                                       |
| **Elasticsearch** | Per-tenant index (`{prefix}_{tenant}_{type}`) **and** a `tenant_id` filter |
| **S3**            | Tenant-scoped key prefix (`prefix-per-tenant`, the default) or a dedicated bucket per tenant (`bucket-per-tenant`, see below) |

The S3 backend is the one place HFS offers a genuine per-tenant *physical*
boundary: `S3TenancyMode::BucketPerTenant` gives each mapped tenant its own
bucket, with its own IAM/policy surface. A tenant absent from the bucket map
is refused with `TenantError::UnmappedTenant` (on request *and* on
`register_tenant`, so the registry cannot name a tenant that has no bucket);
only `__system__` falls back, to the optional `default_system_bucket`, which
also hosts the cross-tenant control-plane state. The `hfs` binary selects the
mode with `HFS_S3_TENANCY=bucket-per-tenant`, maps tenants with
`HFS_S3_TENANT_BUCKETS=tenant=bucket,…` and names the system bucket with
`HFS_S3_SYSTEM_BUCKET` (#1598; the `/run-hfs-server` skill has the rules).
Every other backend is shared-schema.
Note that Elasticsearch gives each tenant its own index, but within a single
cluster and credential, so that is a naming boundary rather than a physical
one — the term filter is what actually isolates.

Schema-per-tenant and database-per-tenant are **not** offered for the SQL
backends, so neither can be cited to satisfy an isolation, compliance, or
data-residency requirement. Unwired SQL generators for them were removed in
[#370](https://github.com/HeliosSoftware/hfs/issues/370), after the capability
advertisements that referenced them were corrected in
[#369](https://github.com/HeliosSoftware/hfs/issues/369). A backend reports
what it actually does through `Backend::capabilities()`.

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

For a capability-by-capability narrative of FHIR Search against the [spec](https://build.fhir.org/search.html) — including the REST-layer vs. backend boundary and a roadmap of known gaps — see [`docs/search-spec-assessment.md`](docs/search-spec-assessment.md).

> **Note:** Documentation links reference [build.fhir.org](https://build.fhir.org), which contains the current FHIR development version. Some features marked as planned are new and may be labeled "Trial Use" in the specification.

**Legend:** ✓ Implemented | ◐ Partial | ○ Planned | ✗ Not planned | † Requires external service | ‡ Mode-dependent — see the notes below the table | — Backend not present in this tree

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
| [Conditional Patch](https://build.fhir.org/http.html#patch)                 | ✓      | ✓          | ✓       | ✗         | ○     | ○             | ✗   |
| [Delete History](https://build.fhir.org/http.html#delete)                   | ✓      | ✓          | ○       | ✗         | ○     | ✗             | ✗   |
| Per-User Settings (`/_user/settings`)                                       | ✓      | ✓          | ✓       | ✗         | ✗     | ✗             | ✓   |
| **Multitenancy**                                                            |
| Shared Schema                                                               | ✓      | ✓          | ✓       | —         | —     | ✓             | ‡✓  |
| Schema-per-Tenant                                                           | ✗      | ✗          | ✗       | —         | —     | ✗             | ✗   |
| Database-per-Tenant                                                         | ✗      | ✗          | ○       | —         | —     | ○             | ‡✓  |
| Row-Level Security                                                          | ✗      | ○          | ✗       | —         | —     | ✗             | ✗   |
| **[Search Parameters](https://build.fhir.org/search.html#ptypes)**          |
| [String](https://build.fhir.org/search.html#string)                         | ✓      | ✓          | ✓       | ✗         | ○     | ✓             | ✗   |
| [Token](https://build.fhir.org/search.html#token)                           | ✓      | ✓          | ✓       | ○         | ○     | ✓             | ✗   |
| [Reference](https://build.fhir.org/search.html#reference)                   | ✓      | ✓          | ✓       | ✗         | ○     | ✓             | ✗   |
| [Date](https://build.fhir.org/search.html#date)                             | ✓      | ✓          | ✓       | ○         | ○     | ✓             | ○   |
| [Number](https://build.fhir.org/search.html#number)                         | ✓      | ✓          | ✓       | ✗         | ○     | ✓             | ○   |
| [Quantity](https://build.fhir.org/search.html#quantity)                     | ✓      | ✓          | ✓       | ✗         | ✗     | ✓             | ○   |
| [URI](https://build.fhir.org/search.html#uri)                               | ✓      | ✓          | ✓       | ○         | ○     | ✓             | ○   |
| [Composite](https://build.fhir.org/search.html#composite)                   | ✓      | ✓          | ✓       | ✗         | ○     | ✓             | ✗   |
| **[Search Modifiers](https://build.fhir.org/search.html#modifiers)**        |
| [:exact](https://build.fhir.org/search.html#modifiers)                      | ✓      | ✓          | ✓       | ○         | ○     | ✓             | ○   |
| [:contains](https://build.fhir.org/search.html#modifiers)                   | ✓      | ✓          | ✓       | ✗         | ○     | ✓             | ✗   |
| [:text](https://build.fhir.org/search.html#modifiers) (full-text)           | ✓      | ◐          | ○       | ✗         | ✗     | ✓             | ✗   |
| [:not](https://build.fhir.org/search.html#modifiers)                        | ✓      | ✓          | ○       | ✗         | ○     | ✓             | ○   |
| [:missing](https://build.fhir.org/search.html#modifiers)                    | ✓      | ✓          | ○       | ✗         | ○     | ✓             | ○   |
| [:above / :below](https://build.fhir.org/search.html#modifiers)             | ◐      | ◐          | ◐       | ✗         | ○     | ◐             | ✗   |
| [:in / :not-in](https://build.fhir.org/search.html#modifiers)               | ✗      | †○         | †○      | ✗         | ○     | †○            | ✗   |
| [:of-type](https://build.fhir.org/search.html#modifiers)                    | ✓      | ✓          | ○       | ✗         | ○     | ✓             | ✗   |
| [:text-advanced](https://build.fhir.org/search.html#modifiertextadvanced)   | ✓      | †○         | †○      | ✗         | ✗     | ✓             | ✗   |
| **[Special Parameters](https://build.fhir.org/search.html#all)**            |
| [\_text](https://build.fhir.org/search.html#_text) (narrative search)       | ✓      | ✓          | ○       | ✗         | ✗     | ✓             | ✗   |
| [\_content](https://build.fhir.org/search.html#_content) (full content)     | ✓      | ✓          | ○       | ✗         | ✗     | ✓             | ✗   |
| [\_filter](https://build.fhir.org/search.html#_filter) (advanced filtering) | ✓      | ○          | ○       | ✗         | ○     | ○             | ✗   |
| **Advanced Search**                                                         |
| [Chained Parameters](https://build.fhir.org/search.html#chaining)           | ✓      | ✓          | ◐       | ✗         | ○     | ◐             | ✗   |
| [Reverse Chaining (\_has)](https://build.fhir.org/search.html#has)          | ✓      | ✓          | ◐       | ✗         | ○     | ◐             | ✗   |
| [\_include](https://build.fhir.org/search.html#include)                     | ✓      | ✓          | ✓       | ✗         | ○     | ✓             | ✗   |
| [\_revinclude](https://build.fhir.org/search.html#revinclude)               | ✓      | ✓          | ✓       | ✗         | ○     | ✓             | ✗   |
| **[Pagination](https://build.fhir.org/http.html#paging)**                   |
| Offset                                                                      | ✓      | ✓          | ✓       | ✗         | ○     | ✓             | ✗   |
| Cursor (keyset)                                                             | ✓      | ✓          | ✓       | ○         | ○     | ✓             | ○   |
| **[Sorting](https://build.fhir.org/search.html#sort)**                      |
| Single field                                                                | ✓      | ✓          | ✓       | ✗         | ○     | ✓             | ✗   |
| Multiple fields                                                             | ✓      | ✓          | ◐       | ✗         | ○     | ✓             | ✗   |
| **[Bulk Operations](https://hl7.org/fhir/uv/bulkdata/)**                    |
| [Bulk Export](https://hl7.org/fhir/uv/bulkdata/export.html)                 | ✓      | ✓          | ○       | ○         | ○     | ○             | ◐   |
| [Bulk Submit ingest](https://build.fhir.org/ig/HL7/bulk-data/en/submit.html)                | ✓      | ✓          | ✓       | ○         | ○     | ○             | ✓   |
| [Bulk Submit REST worker](https://build.fhir.org/ig/HL7/bulk-data/en/submit.html)           | ✓      | ✓          | ✓       | ✗         | ✗     | ✗             | ‡✓  |

**Notes on partial cells:**

- **Sorting** — SQLite and PostgreSQL sort by any indexed search parameter (string, token,
  date, number, quantity, reference, URI) via a correlated subquery into the search index, taking
  the min value ascending / max descending for multi-valued params; `_id`/`_lastUpdated` sort on
  the resources table directly. Cursor (keyset) pagination is consistent with the active sort: the
  sort key value is encoded into the opaque cursor and the keyset comparison runs on it, so deep
  paging preserves the sort order. A multi-field `_sort` returns a single page (no cursor). MongoDB
  sorts by `_id`/`_lastUpdated`, or by up to 15 indexed search parameters (each key in its own
  direction, a missing value last for that key, ties broken by id); it cannot combine a
  search-parameter sort with `_id`/`_lastUpdated`/`_score` or with cursor pagination, hence ◐ for
  multiple fields.
- **`:above` / `:below`** — two mechanisms (◐ = both, conditional on context): (1) hierarchical
  **URI** prefix matching is native to SQLite, PostgreSQL, and Elasticsearch (no external service);
  (2) **token/code** hierarchy (e.g. `code:below=http://snomed.info/sct|73211009`) is resolved at
  the REST layer — the code and its descendants (`:below`, `is-a`) or ancestors (`:above`,
  `generalizes`) are expanded via the terminology server's `$expand`, then matched as a plain token
  OR list on any backend. Token hierarchy therefore requires `HFS_TERMINOLOGY_SERVER`.
- **`:in` / `:not-in`** — handled at the REST layer: `:in` is expanded against a terminology
  server before the query reaches the backend; `:not-in` returns `501 Not Implemented`. No
  backend resolves these natively.
- **Chained / `_has`** — chained and reverse-chained parameters are resolved by the persistence
  layer's backend-agnostic resolver (`search::resolve_chains`): each hop is one plain `search()`
  against the backend, and the result is folded into an `_id` filter
  that the backend's ordinary `search()` executes. Because the rewrite happens above the backend,
  `GET /Observation?subject.name=Smith` and `?_has:Observation:subject:code=…` work end-to-end on
  every search-capable backend — SQLite, PostgreSQL, Elasticsearch, and MongoDB alike. A backend
  may answer a specific shape faster through a `SearchProvider` hook, reusing its ordinary search
  matching: standalone SQLite reads a `_has` hop's references from its index in one statement
  (`search_reference_values`), and PostgreSQL evaluates
  `Patient?_has:Observation:patient:code=system|code` inside `search()`
  on R4 with the denormalized index layout (`supports_native_reverse_chains`).
- **Composite** — SQLite, PostgreSQL, MongoDB, and Elasticsearch evaluate component values
  within one composite instance. SQLite and PostgreSQL unfolded rows share a `composite_group`
  and retain per-type component ordinals in `composite_slot`; predicates for repeated types also
  require the corresponding slot. Thus `A$B`, `B$A`, and `A$A` preserve their declared positions
  in ordinary and contained search, including counts and paging. Contained matches belong to one
  contained entity and group; top-level composite subqueries exclude contained rows. MongoDB uses
  the same group/slot identity with value-index probes. Elasticsearch uses one nested object per
  instance with inline component values; this SQL change does not expand its repeated-type support.
  See [`docs/search-spec-assessment.md`](docs/search-spec-assessment.md).

#### SQL composite slots and upgrade recovery

SQLite schema **38** and PostgreSQL schema **48** add a nullable `composite_slot` without assigning
positions to old rows. New unfolded SQL rows preserve the extractor's ordinal among components of
one type (Token–Quantity–Token uses slots 1, 1, 2). Repeated Token, Number, Quantity, String and Date
components are supported by SQLite and PostgreSQL unfolded readers, up to 255 components of one
type. PostgreSQL's Legacy ordinary reader also accepts valid transitional folded Token/Number
rows. Its Denormalized ordinary reader retains the existing folded columns: at most two Token or
two Number components and one component of each other family. Consequently Token–Quantity–Token
is representable; repeated String/Date/Quantity or a third Token/Number is unsupported for ordinary
or `_contained=both` Denormalized searches. These shapes return a named 400, and `$reindex` does
not make them representable. `_contained=true` uses unfolded rows in either PostgreSQL layout.

Ordinary composite `:missing=true|false` checks only parameter presence, so it needs neither slots
nor a representable positional shape and bypasses these SQL legacy guards. Contained `:missing`
remains unsupported.

For positional matching of a supported repeated-type parameter, SQL conservatively refuses a search
if ambiguous old unfolded rows exist for that tenant, searched resource type, parameter and requested
top-level or contained scope. The named 400 asks for `$reindex` even when the requested value has no matches or
`_count=0`. Valid folded rows and distinct-type composites remain usable. MongoDB's corresponding
guard is value-dependent: it refuses when a component's value probe encounters an old matching row.

Upgrade **all writers** before rebuilding each affected tenant with `POST /$reindex` and
`clearExisting=false` (the default with no body). Use the tenant-wide route rather than a
type-limited rebuild: contained resources are indexed under their container's type. Poll
`GET /$reindex-status/{job_id}` on the **same server node** until `status=completed` and
`errorCount=0`. Keep old writers from reintroducing missing slots during or after recovery.

SQL startup creates sparse indexes for the legacy probes, so the first upgrade can take longer on
a large index. SQLite creates ordinary and contained probes; PostgreSQL creates the contained
probe in migration 48 and the three family-aware ordinary probes only when initializing a Legacy
layout. PostgreSQL builds these under the startup advisory lock with transaction-local statement
timeouts disabled, restoring the session timeout afterward. Fresh Denormalized databases do not
create the ordinary Legacy probes. Existing ambiguous rows stay nullable until the rebuild.

The S3 backend is intentionally storage-focused (CRUD/version/history and the full `$bulk-submit` surface) and does not act as a full FHIR search engine; the one conditional interaction it serves is an identifier-scoped conditional create (`_id` and `identifier` criteria, decided by reading the stored objects, #1435). For bulk export, S3 can feed system-level batches through `ExportDataProvider` and can store output files through `S3OutputStore`, but export job state belongs to SQLite or PostgreSQL. `$bulk-submit` is different: S3 hosts its own job state, since the submission and manifest objects the ingestion engine writes *are* the job state — leases are compare-and-swapped against those objects' ETags, with a small cross-tenant index for claim/poll-token/TTL lookups. Patient-level and Group-level export on S3 enumerate the compartment by reading every current object of each type and applying the Patient compartment parameters in memory (`PatientCompartmentMatcher`), which is correct but scans the whole type per batch. For query-heavy deployments, use a DB/search backend as primary query engine and compose S3 as archive/history/output storage.

**Multitenancy notes.** The four Multitenancy rows describe *where a tenant's records physically live*, not how strongly the boundary is enforced; a deployment sits in exactly one of the first three.

- **‡ The S3 `$bulk-submit` REST worker needs somewhere cross-tenant to write.** Its claim queue and
  poll-token index live outside every tenant prefix, because a worker claims manifests and resolves
  poll tokens with no tenant in hand. Every configuration has such a place except `BucketPerTenant`
  with no `default_system_bucket`, which does not declare the capability and reports `501` from
  `$bulk-submit` — the same axis `supports_user_settings` answers. Synchronous *ingest* is
  tenant-scoped throughout and is available on every configuration. See
  `S3Backend::supports_bulk_submit_worker`.
- **‡ S3 tenancy is a property of the deployment, not the backend type.** A `PrefixPerTenant` instance (one shared bucket, tenant-scoped key prefixes) is *Shared Schema*; a `BucketPerTenant` instance (a dedicated bucket per tenant) is *Database-per-Tenant*. An instance declares exactly one — see `S3Backend::declared_capabilities_for`.
- **○ means no backend implements it today.** Unwired SQL generators for these topologies (shared-schema + RLS, schema-per-tenant `search_path`, database-per-tenant pools) once lived in `src/strategy/`; they were removed in issue #370. Do not read a `○` cell as a deployable isolation guarantee.
- **✗ for PostgreSQL is a decision, not a gap.** Design discussion #28 chose shared-schema for PostgreSQL and declined the per-schema and per-database topologies; advertising them anyway was the defect issue #369 corrected.
- **— Cassandra and Neo4j have no backend in this tree** (`src/backends/mod.rs`), so they have no tenant placement to report.
- **Row-Level Security has no `BackendCapability` counterpart**, so the contract test cannot police it. PostgreSQL is `○` because no backend issues `ALTER TABLE … ENABLE ROW LEVEL SECURITY`; a database-enforced layer is tracked in [#381](https://github.com/HeliosSoftware/hfs/issues/381).

### Polling `_history` with `_since`

`_since` keeps the history rows whose `last_updated` is at or after it, on every backend. A client
that polls `_history?_since=` and starts each window where its previous poll ended can miss
entries for good, in two ways.

**Commit-time gap.** On every backend a write takes its `last_updated` from the HFS process clock
before other readers can see it, and concurrent writes can land out of stamp order. For one
ordinary write the gap is short, but it is not zero. On S3, `StoredResource::new` stamps the
resource and only then do the current, history and index PUTs run, and a batch Bundle runs up to
16 entries at once (`bulk_write_concurrency` 32, capped by `HFS_BATCH_MAX_CONCURRENCY`), so a later
stamp can land while an earlier one is still being PUT. On SQLite, `update` stamps before
`BEGIN IMMEDIATE`, which can wait up to the 30 s busy timeout for the write lock. A transaction
Bundle is the long case, on SQLite, PostgreSQL and MongoDB: it stamps each entry when it stages the
entry, and other readers see the rows only once the Bundle commits, which for a large Bundle can be
much later (minutes on SQLite). Either way, a poll can have covered a `last_updated` whose row only
becomes visible afterwards. S3 refuses transaction Bundles, and Elasticsearch serves no history (a
composite serves `_history` from its primary).

How long the gap can be depends on who bounds the write:

- Every backend: overlap by at least `HFS_REQUEST_TIMEOUT` (default 30 s), because each write is
  stamped before it becomes visible and concurrent writes can land out of order. On PostgreSQL and
  MongoDB that is enough, since a transaction Bundle still running at the timeout is rolled back
  (the timeout layer drops the handler; PostgreSQL's transaction drop spawns the `ROLLBACK`, and
  MongoDB's dropped `ClientSession` aborts its transaction; the MongoDB Bundle deadline, the
  timeout less 2 s, only stops replays and admission waits before that).
- SQLite: the timeout does not reliably bound a transaction Bundle. The path between
  `BEGIN IMMEDIATE` and `COMMIT` is mostly synchronous `rusqlite` calls behind a mutex, with few
  suspension points (a PATCH entry's validation can be one), so the timeout layer usually cannot
  drop the handler and the Bundle runs to its commit however long that takes (about 2 minutes for
  the largest Synthea patient, per the `HFS_MAX_BODY_SIZE` notes). An update can also wait
  up to 30 s for the write lock after it is stamped, and the request timeout cannot cut that wait
  short. Overlap by at least 30 s and by the longest transaction Bundle any client sends. This is
  from reading the code, not from a test that runs a Bundle past a short timeout.
- `$bulk-submit`: the worker is not a request, so no request timeout bounds it. On SQLite and
  PostgreSQL it commits each batch as one transaction. On MongoDB `bulk_ingest.rs` takes one
  `Utc::now()` per batch while planning, stamps every plan with it, and then writes the batch as
  separate commands (resources, then `resource_history`), so a batch's history rows appear after
  their `last_updated` by up to the batch's write time plus retries. S3 writes each entry through
  its ordinary `create`/`update`, so there is no batch-wide gap beyond the request-timeout
  overlap. Overlap by more than the longest batch takes.
- Several HFS instances sharing one store each stamp from their own clock: add the largest clock
  difference between them.

Start each window earlier than the newest `response.lastModified` seen, by the largest of these.

**Page limit.** The REST history handlers answer one page, newest first, with no `link`/`next`
(`HistoryQuery` takes no cursor, although the backends do compute one). `_count` defaults to
`HFS_DEFAULT_PAGE_SIZE` (20) and is capped at `HFS_MAX_PAGE_SIZE` (default 1000). On SQLite,
PostgreSQL and MongoDB the Bundle `total` is just the number of entries returned (their
`PageInfo.total` is `None`, so the handler falls back to the entry count); S3 sets it to every
match before paging, so there a `total` above the entry count means the page was cut off. When a
window holds more versions than `_count`, its oldest entries are dropped and a later window can
never reach them. `build_history_params` lowers a `_count` above the cap to `HFS_MAX_PAGE_SIZE`
without saying so, so the effective page size is the `_count` sent or `HFS_MAX_PAGE_SIZE`,
whichever is smaller. A client should pass `_count` up to the cap, treat a response with that many
entries as cut off (a response to a larger `_count` stops at the cap, so comparing against `_count`
misses it), and poll often enough to stay under it, or accept the limit; history paging is not
implemented.

Other client rules:

- dedupe on `(resourceType, id, versionId)`, since `_since` is inclusive and the windows overlap.
  A `DELETE` entry has no `resource`: take the type and id from `fullUrl` and the version from
  `response.etag`;
- percent-encode the `_since` instant (`+` as `%2B`) or send it in UTC with a `Z` suffix: HFS
  writes `lastModified` with `to_rfc3339()`, which ends in `+00:00`, and an unencoded `+` decodes
  to a space and gets `400 Invalid _since value`.

The client-facing copy of this note is in the book's CLI reference
(`book/src/appendix-a-cli.md`, "Polling history with `_since`").

### Primary/Secondary Role Matrix

Backends can serve as primary (CRUD, versioning, and — where the backend supports it — transactions) or secondary (optimized for specific query patterns). When a secondary search backend is configured, the primary backend's search indexing is automatically disabled to avoid data duplication.

**Not every primary can offer transactions.** A FHIR `transaction` Bundle is all-or-nothing, which requires the primary to be able to unwind. SQLite and PostgreSQL provide it through `TransactionProvider`, and MongoDB through a server-side session transaction. S3 cannot: an object store has no atomic multi-object operation, so an S3-primary deployment declines transaction Bundles with 501 `not-supported` and advertises only `batch` in its CapabilityStatement (#489). This is the trait separation design discussion #28 describes — a backend without transaction support still handles batches, where entries are independent and partial success is the contract.

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
| S3 alone                   | S3         | —                      | ✓ Implemented (storage-focused, batch only) | Archival/history storage                |
| S3 + Elasticsearch         | S3         | Elasticsearch (search) | ✓ Implemented (batch only, no transactions) | Large-scale + search                    |

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

**Schema version.** Startup records the SQLite schema version in the `schema_version` table. A build refuses to start on a database file that records a newer schema version than its own, one a newer HFS release has already upgraded: startup fails before any migration or startup index step runs, the recorded version and the indexes stay as they were, and the error names the database file and both versions and says the database was written by a newer HFS. The recorded version is never lowered, and a crash while it is written no longer leaves the table empty. There is no override. HFS releases from before this check (#2365) still start on such a database and run their own startup index steps on it.

For a first-page search on a single parameter, SQLite reads at most 2,001 index rows of that parameter to decide
whether its key list is short enough to drive the query from, instead of letting the planner walk `resources` in
`last_updated` order. Set `HFS_SQLITE_SEARCH_PROBE=0` (`false`, `off` and `no` also work) to turn the probe off and
always use the planner's plan; it is read once per process and defaults to on. Results are the same either way.

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

Full-featured relational backend for production deployments with JSONB storage and full-text search.

- Full CRUD operations with ACID transactions
- Full-text search via PostgreSQL's tsvector/tsquery
- All FHIR search parameter types (string, token, date, number, quantity, reference, URI, composite)
- Chained parameters and reverse chaining (`_has`)
- `_include` and `_revinclude` resolution
- Multi-tenant support (shared schema via a `tenant_id` discriminator column)

**Prerequisites:** A running PostgreSQL instance (14+; startup refuses an older server before
changing the schema).

**Schema version.** Startup records the PostgreSQL schema version in the `schema_version` table. A
build refuses to start on a database that records a newer schema version than its own, one a newer
HFS release has already upgraded: startup fails under the migration lock before anything is
changed, the recorded version and the indexes stay as they were, and the error names the database
and both versions and says the database was written by a newer HFS. The recorded version is never
lowered. There is no override. HFS releases from before this check (#2367) still start on such a
database and run their own startup index steps on it.

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

The cross-tenant `count_by_tenant` aggregate (Tenants UI, `/admin/tenants`, console tenant
metrics) runs in its own read-only transaction under
`HFS_PG_COUNT_BY_TENANT_STATEMENT_TIMEOUT_MS` (default: `25000`; must be 1 to 2147483647),
applied with `SET LOCAL statement_timeout` so the pooled session keeps
`HFS_PG_STATEMENT_TIMEOUT_MS` for everything else. The count runs under the smaller of the two.
Past the budget the call fails with `BackendError::Timeout` (`504` on the REST endpoints while
the budget, plus the pool checkout and the handler's earlier work, is below `HFS_REQUEST_TIMEOUT`). The session stays owned by the count until
PostgreSQL ends it, even if the caller is dropped first (#1826, #1911).

Every indexed write confirms its tenant's SearchParameter overlay with a cheap
`SELECT COUNT(*), MAX(last_updated)` fingerprint probe. `HFS_PG_SP_FINGERPRINT_TTL_MS`
(default: `0`, every write probes) lets ordinary writes reuse an overlay validated within
that many milliseconds (for example `1000`) and skip the probe. A SearchParameter written by
this process invalidates the cache immediately; one committed by another HFS process is seen by
ordinary writes within the TTL. Bulk submit always probes, whatever the TTL.

Schema v49 adds the partial index `idx_resources_live_tenant ON resources (tenant_id) WHERE
is_deleted = FALSE`, so the count is an index-only scan of a narrow index instead of the wider
`idx_resources_reindex_id`. The first startup on v49 builds it with `CREATE INDEX CONCURRENTLY`
under the migration lock: writes to `resources` continue, but the build waits for transactions
older than it (long exports, sessions idle in a transaction) and blocks other DDL and `VACUUM` on
the table while it runs. v49 is recorded only once the catalog shows a valid index of the expected
shape; an interrupted build is dropped and rebuilt on the next startup. Measured on a
~19M-resource corpus (PostgreSQL 16): 12.3 s and 125.3 MiB to build, and the warm count went from
about 2.43 s to 1.00 s (0.60 s once `VACUUM` had set the visibility map). Operators who want the
build out of the startup path can run it beforehand; v49 then finds it and only records the
version:

```sql
CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_resources_live_tenant
  ON resources (tenant_id) WHERE is_deleted = FALSE;
```

Schema v50 adds `idx_search_date_recent_end` on `search_index (tenant_id, resource_type, param_name,
last_updated DESC, resource_id) INCLUDE (value_date, value_date_end) WHERE value_date_end IS NOT NULL`,
which lets date `gt`/`ge` searches walk the newest-first order and stop at the first qualifying rows
(Observation, Encounter and Patient `gt`/`ge`: 0.07-0.11 ms on a 7M-row `search_index`; 66 MB). It is
built the same way as the v49 index (`CREATE INDEX CONCURRENTLY` under the migration lock, recorded only
once the index is valid, an interrupted build is dropped and rebuilt on the next startup) and runs no
`ANALYZE`: the planner keeps using the table's per-column statistics, which the index does not change.
The index is additive and has no opt-out. To keep the build out of the startup path, run it beforehand;
v50 then finds it and only records the version:

```sql
CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_search_date_recent_end
  ON search_index (tenant_id, resource_type, param_name, last_updated DESC, resource_id ASC)
  INCLUDE (value_date, value_date_end) WHERE value_date_end IS NOT NULL;
```

The newest-first search fast path is only valid when no `search_index` row of the searched parameter
has `last_updated IS NULL` (contained-resource memberships are written that way). Each fast-path
statement used to re-run the whole search predicate in an `index_eligible` CTE to prove it. The
backend now learns, per tenant, which `(resource type, parameter)` pairs have such rows with one
`SELECT DISTINCT ... WHERE is_contained` and caches the answer for 30 s (plus a 10 s grace window
served while one request refreshes it). A cached "NULL rows exist" answer, any probe failure, or a
write by this process that creates contained rows keeps the original SQL, so the cache can only
remove the probe, never add results. Writes by other processes are noticed after the TTL, so a
deployment with several writers that create contained resources against one database can turn the
cache off with `HFS_PG_ELIGIBLE_CACHE=0` (default on; `0`, `false`, `off` and `no` disable it).

### PostgreSQL + Elasticsearch

PostgreSQL handles CRUD, versioning, history, and transactions with ACID guarantees. Elasticsearch handles all search operations. Combines PostgreSQL's production-grade storage with Elasticsearch's search capabilities.

- Full CRUD operations with ACID transactions via PostgreSQL
- Full-text search with relevance scoring (`_text`, `_content`) via Elasticsearch
- All FHIR search parameter types (string, token, date, number, quantity, reference, URI, composite)
- Advanced text search with stemming, boolean operators, and proximity matching (`:text-advanced`)
- Multi-tenant support (shared schema via a `tenant_id` discriminator column)

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
- Transaction bundles with urn:uuid reference resolution (requires replica set; see
  [cache sizing](#sizing-the-wiredtiger-cache-for-transaction-bundles))
- Native search (string, token, reference, date, number, quantity, URI and composite parameters;
  `_text`/`_content` and most modifiers beyond `:exact`/`:contains` are not yet supported;
  chained/`_has` work via the REST-layer resolver)
- `_include` and `_revinclude` resolution
- Conditional create, update, and delete operations
- Cursor and offset pagination; sorting by `_id`/`_lastUpdated` or by up to 15 indexed search
  parameters (a search-parameter sort is offset-paged and cannot be combined with
  `_id`/`_lastUpdated`/`_score` or with cursor pagination)
- Shared-schema multitenancy with strict tenant filtering
- Optimistic locking with ETag support

MongoDB records supported missing parameters on each resource. See
[missing-parameter metadata](../../docs/mongodb/search-indexes.md#missing-parameter-metadata)
for the index, compatibility behavior, rollout and definition maintenance.

**Prerequisites:** A running MongoDB instance. A replica set (one member is enough) or a sharded cluster is needed for transaction Bundles, and it is what makes each single-resource create, update, delete and restore one transaction: the resource, its history row and its search-index entries commit together. On a standalone server those writes run without a transaction, so the `resources` and `resource_history` writes are separate commands and a failure between them can leave a version without its history row; each backend logs a warning the first time it sees a standalone server and again each time a replica set turns into a standalone server, and an info line when a standalone server turns into a replica set. Before each such write HFS asks the server (`hello`) and never treats a failed check as a standalone answer (#2258): a dropped connection, a cleared connection pool or a retryable error is tried up to 3 more times (after 25, 50 and 100 ms); a check that still fails or finds no usable server answers `503` with `Retry-After`, a server deadline `504`, any other failure `500`, and nothing is written. A member started with `--replSet` that has no replica-set configuration yet answers `503` as well; it is not taken for a standalone server. The check uses the `hello` command, which MongoDB has from 4.2.10 and 4.4.2 on (and in every 5.0 or later release); on an older server every single-resource write answers `500`.

**Schema version.** Each boot creates the indexes the running build needs and then records its MongoDB schema version in the `schema_version` collection. A build refuses to start on a database that records a newer schema version than its own, one a newer HFS release has already upgraded: startup fails before anything is written, the recorded version stays as it was, and the error names both versions and says the database was written by a newer HFS. The recorded version is never lowered, also when two builds start at once. There is no override. HFS releases from before this check (#2352) still start on such a database and record their own, lower version.

To roll back to an older build on purpose, follow [Rolling back to an older build](../../docs/mongodb/schema-version.md#rolling-back-to-an-older-build). For an older build from #2352 on, this includes lowering the recorded version by hand. The page also lists what each schema version adds and what an older build leaves behind.

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
- `HFS_MONGODB_CONNECT_TIMEOUT_MS` to control the TCP handshake timeout (default: `5000`)
- `HFS_MONGODB_SERVER_SELECTION_TIMEOUT_MS` to control how long an operation waits for a
  usable server before failing (default: `15000`). This — not the connect timeout — is what
  bounds how quickly an unreachable MongoDB surfaces an error. The request then answers `503`
  with `Retry-After`, and the message names only the operation, for example
  `Failed to execute MongoDB search: the database is temporarily unavailable; retry the request`.
  A dropped connection or a cleared connection pool answers the same way. The driver's error,
  which lists the topology and each server address, is logged at `warn` (#2303).
- `HFS_MONGODB_MAX_CONCURRENT_TRANSACTION_BUNDLES` to limit how many standard-size transaction
  Bundles run at once per server (default: `4`; `0` means no limit). See the sizing section below.
- `HFS_MONGODB_TRANSACTION_BUNDLE_WEIGHT_ENTRIES` for the entry count of one standard Bundle
  (default: `1000`; `0` counts every Bundle as one slot).
- `HFS_MONGODB_TRANSACTION_BUNDLE_DEFERRED_INDEX_ENTRIES` for the entry count from which a
  transaction Bundle writes its search-index entries after its commit instead of inside the
  transaction (default: `auto`, the admission room, 4,000 entries; `off` or `0` never defers).
  Ignored on `mongodb-elasticsearch`. See
  [Deferred search indexing](#deferred-search-indexing-for-large-mongodb-transaction-bundles).
- `HFS_MONGODB_DEFERRED_INDEX_REPAIR_INTERVAL` (`hfs` binary) for the seconds between passes of
  the task that finishes search-index entries a large Bundle left unwritten (default: `60`; `0`
  turns the task off). It also runs once at startup. A pass takes only ledger documents older
  than the larger of 60 s and the Bundle deadline (`HFS_REQUEST_TIMEOUT` less 2 s).
- `HFS_MONGODB_COUNT_BY_TENANT_MAX_TIME_MS` for the server execution budget (`maxTimeMS`) of
  the cross-tenant `count_by_tenant` aggregate, and of no other command (default: `25000`; must
  be 1 to 2147483647). Past it the call fails with `BackendError::Timeout`. It does not bound
  server selection, waiting for a pooled connection or socket time.
- `HFS_MONGODB_SOF_CURSOR_BATCH_SIZE` for the documents per cursor batch of a SQL-on-FHIR scan
  (default: `512`; must be 1 to 100000). Lower means lower first-row latency on selective views,
  higher means fewer round trips against a remote MongoDB. It applies to the in-database
  aggregate and to the patient/group fallback `find`.

#### Single-resource write and read errors

A create, update, delete or restore of one resource runs as one MongoDB transaction on a replica
set or a sharded cluster. These are the answers for its failures:

- **A lost race on the resource document of an update, a delete or a restore** (`WriteConflict`,
  112) answers `409` (`VersionConflict`) or `404`. A PUT without `If-Match` reads again and
  retries up to 3 times (4 attempts in all); a DELETE without `If-Match` tries once more.
- **An abort without a conflicting writer** runs the write again from its first read, up to 4
  attempts with pauses of 100, 200 and 400 ms. That covers a chunk migration (`StaleConfig`), a
  `WriteConflict` that names a catalog change and, since #2269, a `WriteConflict` on the resource's
  `search_index` rows. Other writers touch those rows without changing the resource's version (a
  large Bundle's deferred-rows batch, `$reindex`), so that conflict does not say another write
  won. Since #2300 it also covers a `WriteConflict` on a create's resource insert, history row or
  commit and on a restore's history row or commit, which used to answer `500`: a create carries no
  version to lose, and a restore already holds the resource document by then. Every run checks the
  precondition again, so a real winner still answers `409` or `404`: a create whose id another
  write stored in the meantime answers `409` (`AlreadyExists`), and a PUT to a new id that another
  write created in the meantime reads again and updates it. When the attempts run out the answer
  is `503` with `Retry-After`.
- **A rollback for eviction** (WiredTiger cache pressure) answers `503` with `Retry-After` at
  once; the same write would meet the same cache.
- **A commit whose outcome is unknown** (`UnknownTransactionCommitResult`, or a write-concern
  error) is sent again up to twice, as a transaction Bundle's commit is (#2270). The write itself
  never runs again, because it may already be applied. If the outcome stays unknown, the answer
  is `500` with the code `exception` and a message that says the change may have been stored and
  should be checked before retrying; there is no `Retry-After`. It is the same status a
  transaction Bundle gives for an unknown commit (`TransactionError::CommitOutcomeUnknown`), and
  it is not `503` because a blind resubmit could store a POST a second time.
- **A failed read** (`read`, `vread` and the existence check, #2271) answers `503` with
  `Retry-After` when its connection dropped, its connection pool was cleared or it found no
  usable server. It also answers `503` when the server refused it with a code that means the node
  could not serve it just now (#2302). Either the node is not (or no longer) the primary, is
  recovering or is shutting down: NotWritablePrimary (10107), NotPrimaryNoSecondaryOk (13435),
  LegacyNotPrimary (10058), InterruptedDueToReplStateChange (11602), NotPrimaryOrSecondary
  (13436), PrimarySteppedDown (189), InterruptedAtShutdown (11600) and ShutdownInProgress (91).
  Or the server gave one of the other codes the driver retries a read on: HostUnreachable (6),
  HostNotFound (7), NetworkTimeout (89), SocketException (9001) and
  ReadConcernMajorityNotAvailableYet (134). The driver retries a read once on each of these codes
  except LegacyNotPrimary (10058), so such a read has already outlasted that retry. The message
  names the read only; the driver error is logged at `warn`. A server deadline (MaxTimeMSExpired
  50, ExceededTimeLimit 262) answers `504`; any other driver error answers `500`, with the driver
  error kept as its source. Since #2301 the same rules cover the other single-document reads: the
  reads a single-resource write makes outside its transaction (on a standalone server, and the
  re-reads after a lost race), the tenant registry lookup, `delete_with_match`'s load, `$purge`'s
  existence check and the `$reindex` walk's resource-type list and newest-resource probe. The read
  is not retried in HFS: the driver already retries it once (`retryReads`), so a primary election
  that outlasts that retry reaches the client as this `503`.
- **A failed query or purge** (searches, counts, history queries, `$purge`, the tenant purge and
  `$reindex`'s clear step) answers `503` with `Retry-After` on the same state-change and
  retryable-read codes since #2334; it used to answer `500`. The message names the operation
  only, and the driver error goes to the same `warn` line. A server deadline (50, 262) still
  answers `504`. The driver retries the initial command of such a read once (never a cursor's
  `getMore`), and never a `delete_many` or `update_many`, so a purge can stop partway. Sending it
  again does no harm: each delete removes only what is left, and nothing is written twice. But a
  retry does not always finish the job. A retried tenant purge
  (`DELETE /admin/tenants/{id}?purge=true`) finishes it only with the #2309 fix (PR #2358);
  without it the retry answers `404` once the tenant is deregistered and has no live resources.
  An instance `$purge` that stopped after its `resources` and `resource_history` deletes answers
  `404` to a retry, because it finds no trace of the resource, and leaves its search rows,
  contained search rows, bulk-submit previous content and sync-failure rows behind (#2146). A
  transaction Bundle entry that meets one of these codes aborts the attempt, and the Bundle runs
  again as after any transient abort. A failed commit is still decided by the commit's own labels,
  so a commit whose outcome is unknown is re-sent, never replayed.
- **A failed cursor or bulk-data read** (#2335) is classified the same way: a history query's
  later batches (`getMore`), the tenant list, the `$reindex` page and by-id reads, the bulk export
  data reads (type list, counts, pages, patient list, Group members) and the `$bulk-submit`
  job-store reads (submission, status, poll-token lookup, manifests, counters, receipts, change
  log, claim scan).
  They used to answer `500` whatever the cause, with the driver text in the message. A bulk
  export run that meets such a failure leaves its job to be claimed again (#2384, see `HFS_BULK_EXPORT_MAX_ATTEMPTS` in the REST README), and the `$bulk-submit` claim loop
  still waits and claims again: only the error's kind and text change.

#### Sizing the WiredTiger cache for transaction Bundles

A FHIR transaction Bundle is one MongoDB multi-document transaction, so it needs a replica set
or a sharded cluster, not a standalone server. The Bundle's whole write set (its resources, their
history rows and their search-index entries) must fit in WiredTiger's cache while the
transaction runs, and each Bundle running at the same time needs room of its own. Since #1975 a
Bundle of at least `HFS_MONGODB_TRANSACTION_BUNDLE_DEFERRED_INDEX_ENTRIES` entries (4,000 by
default) does not write its search-index entries inside the transaction, which is most of its
write set; see
[Deferred search indexing for large MongoDB transaction Bundles](#deferred-search-indexing-for-large-mongodb-transaction-bundles).
The measurements below predate that change and describe Bundles under the threshold.

**How a Bundle sends its creates (#1977).** Creating a resource (a POST, or a PUT or conditional
PUT that creates) no longer starts with a read of `resources` to see whether the id is taken; a
PUT still looks up its target to choose between create and update. The unique
`idx_resources_identity` refuses a reused id, live or deleted, and the Bundle fails at that entry
with the same `400` ("resource already exists") as before. The creates are kept in memory and sent
in chunks: a chunk goes out once it holds 100 entries, 16 MiB of resource payload (counted as BSON
once for the `resources` document and once for its history copy, not the memory held) or 5,000
search rows, and a
resource of 1 MiB or more is sent on its own. These bounds are fixed in code, not configurable. A
chunk is one ordered `insert_many` into `resources`, one into `resource_history` and, for natively
indexed resources, one delete of any old rows of the created ids from each of `search_index` and
`search_index_contained`, followed by one `insert_many` of the new rows into each that has any. A
chunk is also sent before any entry that reads what the Bundle wrote (a GET, PUT, PATCH, DELETE,
or a POST with `ifNoneExist`), before an entry's failure is reported, before the deferred-index
ledger and before the commit. HFS runs a Bundle's entries in the order DELETE, POST, PUT or PATCH,
GET, so its plain POSTs are normally one run. A run of plain POSTs within the bounds sends at most
6 commands per 100 entries, where it sent 6 to 7 per entry before (3 per entry on
`mongodb-elasticsearch` and above the deferral threshold, which now send 2 per 100); a bound can
cut a chunk short. A PUT or conditional PUT that creates, and a POST with `ifNoneExist`, make the
Bundle send what it holds first, so such creates are sent one at a time and save only the
existence read. The Bundle writes the same documents and index keys, so this shortens how long it
runs and holds the admission room. For the same reason the cache ceiling below is not expected to
move; that is inferred. Neither the speed-up nor the ceiling has been measured on the benchmark.

Past what the cache can hold, MongoDB aborts the oldest transaction ("oldest pinned transaction
ID rolled back for eviction"). HFS replays such aborts (#1641, #1700), except for a Bundle that
runs alone (see below), and returns `503` with `Retry-After` when the replays run out, but
replays cannot fix this: the Bundles that collide again run into the same cache limit. A Bundle
that is too large for the cache fails on every replay.

So HFS admits transaction Bundles while their entries fit in
`HFS_MONGODB_MAX_CONCURRENT_TRANSACTION_BUNDLES` x `HFS_MONGODB_TRANSACTION_BUNDLE_WEIGHT_ENTRIES`
(default 4 x 1,000 = 4,000 entries) per backend instance. A Bundle counts its entry count, at
least 1; one larger than the whole room is admitted alone, so it can always run. The rest wait
in arrival order (FIFO) without holding a session or a transaction, and a large Bundle at the
head of the queue is not overtaken by smaller ones behind it. The time spent waiting counts
toward the request timeout. A limit of `0` removes the gate; a weight of `0` counts every Bundle
as one slot, which is the plain count-based gate of #1776. In the `hfs` binary the backend
instance is the MongoDB primary that serves Bundles. The weighting is per backend instance, not
per tenant.

The limit counts standard Bundles of `HFS_MONGODB_TRANSACTION_BUNDLE_WEIGHT_ENTRIES` entries,
not Bundles. With the default weight, many small transaction Bundles can be open at once, each
with its own session and transaction, bounded by the entry room and the connection pool rather
than by the limit. For a hard cap on concurrent Bundles (for example limit `1` to serialise
them), set `HFS_MONGODB_TRANSACTION_BUNDLE_WEIGHT_ENTRIES=0`.

The default weight of 1,000 is the smallest round size at or above the 801-entry Bundles
measured below, so those are admitted four at a time exactly as before. For Bundles of up to
1,000 entries the defaults never admit more entries at once than the count-based gate did.
Bundles of about 1,600 entries now run two at a time instead of four, and small Bundles share a
slot.

Measured with the in-repo harness: 801-entry Bundles sent by 20 concurrent clients, a debug
build, MongoDB 7.0 as a single-member replica set. The table was measured with the count-based
gate of #1776, which is the same as `HFS_MONGODB_TRANSACTION_BUNDLE_WEIGHT_ENTRIES=0`:

| WiredTiger cache | Bundles | Limit | Committed | WiredTiger eviction rollbacks |
|---|---|---|---|---|
| 1 GB | 24 | 4 (default) | 24/24 | 0 |
| 1 GB | 24 | none (`0`) | 5/24 | 65 |
| 1 GB | 24 | 8 | 13/24 | 40 |
| 0.25 GB | 12 | 1 | 12/12 | 0 |
| 2 GB, ~1,600 entries | 24 | 4 (default) | 22/24 | 2 |

Rows 1-3 use the harness defaults (`HFS_TEST_BUNDLE_LOAD_WT_CACHE_GB=1`,
`HFS_TEST_BUNDLE_LOAD_BUNDLES=24`, `HFS_TEST_BUNDLE_LOAD_COPIES=3`) with
`HFS_TEST_BUNDLE_LOAD_LIMIT=4`, `0` and `8`. Row 4 is
`HFS_TEST_BUNDLE_LOAD_WT_CACHE_GB=0.25 HFS_TEST_BUNDLE_LOAD_BUNDLES=12 HFS_TEST_BUNDLE_LOAD_LIMIT=1`.
Row 5 is `HFS_TEST_BUNDLE_LOAD_WT_CACHE_GB=2 HFS_TEST_BUNDLE_LOAD_COPIES=6` at the default
limit. Results vary a little between runs (another run of row 2 gave 6/24, and of row 3 14/24).

Under the weighted default, rows 1, 2 and 4 admit the same Bundles at once. Row 3 (limit 8)
would admit nine 801-entry Bundles at once, and row 5 would run its ~1,600-entry Bundles two at
a time; neither was re-measured. Row 1 was re-run with the weighted default (limit 4, weight 1,000)
and gave the same result: 24/24 committed, 0 rollbacks (`ok=24 transient=0 wt_rollbacks=0`,
`wall_s=222.3`, `p50_s=137.8`).

Measured with the HFS benchmark and no admission limit (20 at once), 1,000 Synthea transaction
Bundles of about 1,600 entries each: about 53% of the Bundles imported with
`--wiredTigerCacheSizeGB 2`, and about 82% with `--wiredTigerCacheSizeGB 4`. The same Bundle
shape was measured only with the harness, with the count-based gate (row 5 above, 22/24: four
~1,600-entry Bundles at once). The weighted default runs them two at a time and has not been
measured for this shape. At 2 GB the count-based gate helped but did not fully remove the
failures, so give such Bundles more cache or a lower limit. That row ran without a request
timeout, with a median Bundle time of about 380 s, far above the 120 s replay budget.

If Bundles still fail with `503`, lower the limit (`1` for a 256 MB cache with ~800-entry
Bundles). Raise the limit or the weight only with a larger cache. Keep `HFS_REQUEST_TIMEOUT` long
enough for queued imports, since a Bundle waiting for its turn is still bounded by it.
Otherwise, size `--wiredTigerCacheSizeGB` for the entries the gate admits at once, the limit ×
`HFS_MONGODB_TRANSACTION_BUNDLE_WEIGHT_ENTRIES` (4,000 by default), or for the largest single
Bundle when that is bigger, since a Bundle larger than the room runs alone. Or split very large
Bundles into smaller ones.

A single Bundle can also be too large for the cache on its own, and the gate cannot help with that:
a Bundle larger than the room already runs alone. When one transaction's write set does not fit in
WiredTiger's cache, MongoDB rolls it back with error 388 (`TransactionTooLargeForCache`,
"transaction is too large and will not fit in the storage engine cache"), on one of the Bundle's
writes or on its commit. The commit also writes the transaction's oplog entry, which can push a
Bundle just under the ceiling over it (#1991). HFS neither replays such a Bundle nor re-sends its
commit, since every attempt meets the same cache, and answers `500` with a `too-costly`
OperationOutcome that names the WiredTiger cache and the Bundle's entry count and says that
resubmitting it unchanged will fail the same way. The driver's text stays in the server log (#1837);
a commit-time 388 is logged with the prefix `Commit failed:`. Size `--wiredTigerCacheSizeGB` for the
largest transaction Bundle you import, or split such Bundles. A 2 GB cache is too small for the
largest Bundles of the full Synthea corpus: in the HFS benchmark (MongoDB with
`--wiredTigerCacheSizeGB 2`, 20 virtual users, 1,000 Synthea transaction Bundles), 8 Bundles of
6,400-7,600 entries each failed this way. With the search-index rows deferred, a lone Bundle's
ceiling rises toward that of the `mongodb-elasticsearch` leg, whose benchmark Bundles aborted at
28,700-31,800 entries at 2 GB. The first benchmark with the deferral (run 39657850695) landed lower,
at 19,200-21,100 entries. The ignored ceiling test of the
[deferred-indexing section](#deferred-search-indexing-for-large-mongodb-transaction-bundles) shows
that a deferred create's transaction wrote each resource's `missing_params` and its
`idx_resources_missing_params` keys (15.19 per resource on its fixture), which an offloaded backend
does not write, and that a deferred Bundle without them reached the offloaded ceiling. We infer
that the benchmark's gap has the same cause: the test's deferred/offloaded ratio before this change
was 0.64-0.65 at 0.5 GB and 0.64 at 2 GB, and the benchmark's four Bundles that failed on both legs
gave 0.62-0.70 (the native abort entry over the offloaded one). A deferred create writes neither
now.
Measured with that test (one synthetic Bundle of a 267-entry US Core fixture repeated, in a
dedicated `mongo:7.0` container on a shared host; a harness result, not the benchmark), the entry at
which a larger Bundle's transaction aborted was, over three runs of each cell (two for `offloaded`
before this change at 2 GB):

| Variant | 0.5 GB cache | 2 GB cache |
|---|---:|---:|
| `deferred`, before this change | 5,729-5,780 | 22,898-22,972 |
| `deferred_no_metadata`, before this change | 9,085-9,211 | not run |
| `deferred`, after | 8,871-8,933 | 35,346-35,481 |
| `offloaded`, before this change | 8,936-9,040 | 35,878-35,921 |
| `offloaded`, after | 9,011-9,036 | 35,884-35,943 |

These are the entries at which a Bundle larger than the ceiling aborted: an upper bound on the size
of a Bundle that can commit, not the largest one measured to commit, and accurate to about 100
entries (one create chunk). A deferred Bundle now reaches 0.98-0.99 of the offloaded ceiling
(0.982-0.991 at 0.5 GB and 0.985-0.987 at 2 GB, each ratio within one run). We infer that the
remaining 0.9-1.8% is the `search_rows_pending` marker, about 62 bytes per `resources` document; the
ledger is written after the last entry, and every aborted run left `ledger=0`, so it does not move
the abort entry. One Bundle was run to commit near the ceiling: at 2 GB, 28,035 entries (105 copies,
79% of the deferred abort entry above), `deferred` committed it in two runs (351.1 s and 347.5 s), each with `rows=447930`,
`ledger=0` and `markers=0`, while `inline` aborted at entry 7,349 in the first (see Ceiling test).
That shows that a Bundle of that size commits here, not where the ceiling lies. The fixture's
ceilings sit above the benchmark's, and the benchmark's `mongodb` leg has not been run with this
change: its result for Bundles under 30,000 entries is open. A Bundle above the offloaded ceiling
(about 30,000 entries in the benchmark, where its corpus has four of 33,562-49,027) still fails on
its resources and history alone.

MongoDB sends 388 only when the transaction holds more than 75% of the cache's dirty bytes
(`transactionTooLargeForCacheThreshold`). When other dirty data shares the cache, the same rollback
arrives as `WriteConflict` (112, labelled `TransientTransactionError`, "oldest pinned transaction ID
rolled back for eviction", or on 7.0 also "transaction rolled back because of cache overflow"), or as
`TemporarilyUnavailable` (365) at commit. A Bundle that takes the
whole room (its entries are at least the limit x the weight, 4,000 by default) runs alone, so that
rollback is mostly its own write set meeting the cache; in the benchmark 49 of 50 such replays
failed again at nearly the same entry while holding the whole room. HFS does not replay it. It
answers `500` `too-costly` as for 388, with the entry count and the same advice, but without saying
that an unchanged resubmit will fail: the room bounds only this instance's transaction Bundles,
while single-resource writes, batch Bundles, other HFS instances and other clients share the same
cache, so a resubmit is unlikely to succeed but can (#1993). It logs one `WARN`, "transaction bundle
rolled back for eviction while running alone; not retrying", with `lone=true`, `entries`,
`failed_entry` (absent at commit), `error_code` and `attempt` (#1895). With chunked creates
(#1977), `failed_entry`, and the "Entry N" of the log-only reasons, is the entry whose document the
server refused when the error names one, and otherwise the first entry of the chunk being sent, so
an abort position reads up to 99 entries before the entry the server was writing. The
`Transaction failed` `ERROR` line names the cause: `write set too large` for 388,
`lone eviction rollback` here. Bundles that shared the room, and plain write conflicts, are still
replayed. With a weight of `0` or no limit no Bundle counts as alone. Raising the limit x the
weight above the largest Bundle restores the replays, at the cost of sharing the cache. The room is
per backend instance: Bundles sent to another HFS instance on the same MongoDB are not seen.

Two caveats. In `hfs` the replay budget (`bundle_transaction_budget`: `HFS_REQUEST_TIMEOUT` less
2 s, capped at 120 s) counts from admission. The whole call, wait included, stays within
`HFS_REQUEST_TIMEOUT` less 2 s (`bundle_transaction_deadline`), so no replay starts that would run
into the `408`. Under the default 30 s timeout both are 28 s, so a Bundle that waited most of it
still gets no replay on a transient abort and returns `503`. With a longer timeout for queued
imports, the wait no longer uses up the 120 s of replays. A wait that outlasts the request
timeout still ends in `408`. And while a cancelled (timed-out) Bundle's server-side abort is
still in flight, one more transaction than the limit may briefly be open.

To re-measure on your own hardware, run the ignored harness (it starts its own MongoDB
container; `HFS_TEST_BUNDLE_LOAD_LIMIT`, `HFS_TEST_BUNDLE_LOAD_WEIGHT_ENTRIES`,
`HFS_TEST_BUNDLE_LOAD_WT_CACHE_GB` and `HFS_TEST_BUNDLE_LOAD_BUNDLES` vary the run;
`HFS_TEST_BUNDLE_LOAD_LIMIT=0` reproduces the pre-#1776 behaviour,
`HFS_TEST_BUNDLE_LOAD_WEIGHT_ENTRIES=0` the count-based gate of #1776,
`HFS_TEST_BUNDLE_LOAD_DEFERRED_INDEX_ENTRIES` sets the threshold for writing a Bundle's search
rows after its commit (#1975; the harness default is `off`, so it keeps measuring the write set
inside the transaction, and `auto` or a number turns the deferral on), and
`HFS_TEST_BUNDLE_LOAD_EXPECT_ALL_OK=1` fails the run if any Bundle ends transient or too-costly):

```bash
cargo test -p helios-persistence --features mongodb --test mongodb_tests \
  transaction_bundle_load -- --ignored --nocapture
```

### MongoDB + Elasticsearch

MongoDB remains the canonical write/read store while Elasticsearch owns delegated search execution. This mode mirrors the existing SQLite + Elasticsearch and PostgreSQL + Elasticsearch composite patterns.

- MongoDB handles CRUD, versioning, history, and conditional write behavior
- Elasticsearch handles delegated search queries, including full-text search
- MongoDB search index population is automatically disabled via `search_offloaded`
- Composite routing preserves MongoDB as the source of truth for reads and writes

> **MongoDB + Elasticsearch note:** with search offloaded, an in-transaction `ifNoneExist` is resolved by scanning the raw `resources` documents inside the transaction session. Only `_id`, `_lastUpdated` and plain `identifier` values (`code`, `|code`, or `system|code`, with a nonempty code) are evaluated; `|code` matches any system, same as Mongo direct search. Every other shape is rejected so the bundle rolls back instead of matching the wrong set (#1394).

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

S3 handles CRUD, versioning, history, and the whole `$bulk-submit` surface (ingestion and job state alike). Elasticsearch handles all search operations, and — since the S3 primary keeps no counts — the Home dashboard's totals, per-type breakdown and write marker (#1280); it keeps no history, so the dashboard's creation-time series is empty on this topology. For bulk export, this topology can use S3 as the resource data provider for system-level exports and `S3OutputStore` as the output-file store; export job state still lives in the configured SQLite or PostgreSQL bulk-export job store.

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
| `HFS_S3_TENANCY` | `prefix-per-tenant` | `prefix-per-tenant` or `bucket-per-tenant` |
| `HFS_S3_BUCKET` | `hfs` | The shared bucket (prefix-per-tenant only) |
| `HFS_S3_TENANT_BUCKETS` | (none) | Bucket-per-tenant, required: `tenant=bucket,…` including `HFS_DEFAULT_TENANT` |
| `HFS_S3_SYSTEM_BUCKET` | (none) | Bucket-per-tenant, optional: `default_system_bucket` |
| `HFS_S3_REGION` | (provider chain) | AWS region override |
| `HFS_S3_PREFIX` | (none) | Optional global key prefix |
| `HFS_S3_VALIDATE_BUCKETS` | `true` | Validate buckets on startup via `HeadBucket` |
| `HFS_S3_KEYSPACE_LAYOUT` | `v1` | `v1` (flat) or `v2` (sharded by id hash, no `history/system/`): the layout **new, empty tenants** get; fixed per tenant by a marker object once chosen (see S3 keyspace layout) |
| `HFS_S3_SCAN_CONCURRENCY` | `32` | The most `GetObject` requests S3 reads may have in flight at once, shared by every request on the S3 store: SQL-on-FHIR scans and by-id reads (`$sql-run`, previews, SQL Query, `$sql-export`), type counts, conditional-create and SearchParameter scans, and `_history`. A whole number of at least 1. Writes (startup seeding, bulk writes) are unaffected |

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

### Deferred search indexing for large MongoDB transaction Bundles

On the native `mongodb` backend a transaction Bundle normally writes the search-index entries of
every resource (about 18.6 `search_index` rows per entry) inside its transaction, and those rows
are most of what WiredTiger holds in its cache until the commit. A Bundle of at least
`HFS_MONGODB_TRANSACTION_BUNDLE_DEFERRED_INDEX_ENTRIES` entries (#1975) therefore writes only its
resources and their history inside the transaction, and writes the search-index entries right
after the commit, before the response. This is the same ordering the Elasticsearch composites
use, where the search index is written after the primary commits. It lifts the Bundle size at
which MongoDB rolls a lone Bundle back for cache eviction (see
[Sizing the WiredTiger cache for transaction Bundles](#sizing-the-wiredtiger-cache-for-transaction-bundles)).

**When it applies.**

- Only on the native `mongodb` backend, and only for transaction Bundles. Never while search is
  offloaded (`mongodb-elasticsearch`): the variable is ignored there, nothing is deferred, no
  ledger is written and `hfs` starts no repair task.
- The rule is `entries >= N`. `HFS_MONGODB_TRANSACTION_BUNDLE_DEFERRED_INDEX_ENTRIES` sets `N`
  (`MongoBackendConfig::transaction_bundle_deferred_index_entries`). The default, `auto`, is the
  admission room in entries, the limit x the weight (4 x 1,000 = 4,000), which is also the size
  from which a Bundle runs alone. With the gate off (limit `0`) or counting Bundles (weight `0`)
  `auto` is 4,000. `off` or `0` never defers, and a plain integer sets `N` directly. Anything else
  fails startup with an error that names the variable.
- `auto` follows the admission room, so shrinking the room shrinks `N`: a limit of `1` with the
  default weight defers every Bundle of 1,000 entries or more. Set the variable explicitly to keep
  a threshold you chose.
- Bundles below `N`, and every Bundle when the deferral is off, run exactly as before.
- `hfs` logs `N` at startup (the `deferred_index_min_entries` field of "MongoDB transaction
  Bundles of at least this many entries write their search rows after the commit"; `None` means
  never).

**What the transaction writes.** The `resources` documents and the history rows, as always. Each
`resources` document whose rows are deferred also carries a transient field,
`search_rows_pending: <bundle id>`, until its rows are written. The transaction adds a ledger, the
`search_index_pending` collection, with one document per 5,000 resources (50,000 resources write
ten), each naming its resources as `Type/id` together with `tenant_id`, `bundle_id`, `chunk`,
`chunks`, `entries`, `created_at`, `not_before` and `attempts`. The ledger goes in after the last
chunk of creates (#1977) and before the commit, and it commits or aborts with the Bundle, so an
aborted or replayed attempt leaves neither a ledger nor markers. A failure to write the ledger is a
server-side failure of the Bundle and answers `500`, with nothing applied (a cache failure still
ends as `500` `too-costly`, and a transient abort is replayed as before); no entry is blamed for
it. The transaction does not insert `search_index` or `search_index_contained` rows, and it skips
the deletes of old rows that an inline Bundle sends once per chunk of creates (or per update).

A resource of a deferred type that the Bundle creates also carries no `missing_params` and no
`missing_params_fp`
([missing-parameter metadata](../../docs/mongodb/search-indexes.md#missing-parameter-metadata)) from
the transaction (the `ifNoneExist` target types stay inline and keep their metadata in the
transaction, see Conditional entries), and the transaction does not run the search extraction for
it: the post-commit step (or the repair) writes the rows, `missing_params` and the fingerprint from
one extraction, in the update that clears the marker. The array adds one multikey key per absent
search parameter to `idx_resources_missing_params` (15.19 per resource, measured on the ceiling
fixture's 267 entries) and the fingerprint one key to `idx_resources_missing_params_fp`, and
WiredTiger holds every one of them in cache until the commit. Without them a create writes what an
offloaded backend's create writes (the document, its history, one `null` key in
`idx_resources_missing_params`), plus the marker of about 60 bytes (measured: 1,736 bytes per
`resources` document against 1,674). An update, a restore or a delete of an existing resource still
writes its `missing_params` in the transaction, and the step rewrites it after the SearchParameter
reload; every such update rewrites all of that resource's keys in `idx_resources_missing_params`,
because `last_updated` is part of the index key. The collection and its `not_before` index
(`idx_search_index_pending_due`) are created at schema initialisation (schema version 13).

**The step after the commit.** While it still holds its admission (so the next large Bundle does
not compete with these inserts for cache), the request reloads the SearchParameter cache if the
Bundle changed SearchParameters, and then rewrites the rows:

- The rows and `missing_params` are rebuilt from the resources' current stored state, with the
  builder inline writes and `$reindex` use, not from the request. A resource deleted by the Bundle
  gets no rows.
- The batches run on the Bundle's own session, which is causally consistent: the first batch reads
  at or after the Bundle's commit, so a marker cannot be missed because the commit is not yet
  visible to it (for example under `readConcernLevel=majority` with `w:1`). A batch that finds no
  marker was finished by another rewriter.
- Resources are taken in batches of up to 256 of one type. Each batch is its own small
  transaction (about 5,000 rows) that selects the resources still carrying the marker, deletes
  their old rows and inserts the new ones. It then makes one update per resource that removes the
  marker and writes `missing_params` and its fingerprint from the same extraction as the rows
  (a created resource has none until then; an updated one is rewritten): `[]` for a deleted
  resource, and removed when the extraction failed, as on every other write path. A batch that
  fails is tried up to five times (25 ms x the attempt number between tries). When all batches of
  a ledger document are done, the document is deleted.
- A batch that runs out of its five attempts stops the step: the rest of that ledger document and
  every later one are left to the repair, so a stalled MongoDB costs one batch's attempts, not
  one per document.
- The step starts no new attempt after the Bundle's own deadline (`bundle_transaction_deadline`:
  `HFS_REQUEST_TIMEOUT` less 2 s, counted from the call), and an attempt still running at the
  deadline is cut, so a committed Bundle is answered instead of timing out with `408` and being
  resubmitted. The cut attempt's transaction is aborted by dropping the session, not awaited.
- It never turns a committed Bundle into an error. Whatever it did not finish stays in the ledger
  for the repair, and cancelling the request mid-step aborts only the batch in flight, since the
  batches before it have committed.
- It logs one `INFO` line per Bundle, "transaction bundle search rows written after commit", with
  `tenant`, `bundle_id`, `entries`, `resources`, `indexed`, `rows`, `batches`, `retries` and `ms`.
  If anything is left it also logs a `WARN`, "transaction bundle search rows not all written after
  commit; the deferred index repair will finish them", with `left_chunks`, `stopped_at_deadline`
  and `last_error`.

The rows are written in small transactions rather than outside any transaction, because they must
not end up stale or duplicated next to a concurrent write. The marker is removed, and
`missing_params` rewritten, in the same transaction that rewrites the rows, so each rewriter (this
step, a repair, another instance) writes the resource document, and MongoDB's write conflict on
that document serializes them: a loser starts again from the state the winner committed.

**Consistency.**

- The client that submitted the Bundle can search it once it has the response, unless the `WARN`
  above reports rows left to the repair (a deadline cut, a failed batch): the response is still the
  committed Bundle, and the missing rows follow within the repair's lag.
- Other clients can read the resources at once, by id, version and history, and a search that uses
  only `_id` and `_lastUpdated` finds them at once, since those are resolved on the resource
  documents. A search on any other parameter can miss them until their rows are written. For an
  updated or deleted resource the old rows stay until its batch runs, so such a search can briefly
  match its previous content. The lag is inferred, not measured with this change: about 2 s for
  a 6,000-entry Bundle and about 10 s for a 30,000-entry one, from the 57,346 rows/s the
  `$reindex` writer reached in a benchmark run and 18.6 rows per entry. After a crash or a
  deadline cut it is the repair's lag: in `hfs`, its minimum age (the larger of 60 s and the
  Bundle deadline) plus up to one repair interval plus the time to write the rows.
- A supported `:missing=true` search resolves on the resource document's `missing_params`. A
  resource of a deferred type that the Bundle created has none until its batch commits (the
  `ifNoneExist` target types keep theirs). It still carries the `search_rows_pending` marker
  then, and `:missing=true` leaves a marked resource without `missing_params` out, like every other
  search that reads rows, rather than answering it from rows that are not there yet (which would
  list it as missing every parameter, so that `Patient?family:missing=true` found a Patient that
  has a family). A conditional criterion with `:missing=true` from another client misses it the
  same way during the lag. An updated resource keeps the `missing_params` its
  transaction wrote, so it is answered at once, from its new content. `:missing=false` checks the
  rows, so like any other parameter it can miss the resources until their rows are written, and
  for an updated resource it sees the old rows. `:missing=true` reads rows too, and lags the same
  way, until `idx_resources_missing_params` is built, for a parameter the metadata route does not
  support, and for a resource whose extraction failed (it has no `missing_params`). The submitting
  client is not affected unless the step was cut at the Bundle deadline or a batch failed (see the
  first bullet): the step writes the metadata before the response.
- Conditional operations and conditional references from other clients resolve through the search
  rows, so during that lag they can miss a resource of a just-committed Bundle and, for example,
  create a duplicate. The Elasticsearch composites have the same exposure.
- A client write to a resource of a just-committed Bundle can lose the write conflict with the
  step's batch and answer `409` (`VersionConflict`); sending it again once the batch has
  committed succeeds. Its rows never end stale or duplicated.
- SearchParameters created or changed in the same Bundle apply to its deferred rows and to the
  `missing_params` the step writes, because the cache is reloaded after the commit and before the
  step. Rows and `missing_params` written inside a transaction are built before that reload: those
  of an inline Bundle, of the `ifNoneExist` types of a deferred Bundle, and, until its batch
  rewrites it, a deferred update's `missing_params`. A deferred create has none to rewrite.

**Conditional entries.** A POST entry with `ifNoneExist` is matched inside the Bundle's own
transaction through the search rows, so it must see an earlier entry of the same Bundle. A
deferred Bundle therefore still writes the rows of every resource whose type is the target of one
of its own `ifNoneExist` entries inside the transaction (no marker, no ledger entry), and defers
the other types. Matching is exactly what an inline Bundle does, no criterion is refused that an
inline Bundle accepts, and a Bundle whose resources are all of such types writes no ledger. URL
conditionals (`PUT Patient?identifier=...`) and conditional references are resolved before the
Bundle's first write, against rows that are already committed, so the deferral does not change
them.

**Repair.** `hfs` runs `MongoBackend::repair_deferred_search_index(min_age, max_chunks)` once at
startup and then every `HFS_MONGODB_DEFERRED_INDEX_REPAIR_INTERVAL` seconds (default `60`; `0`
turns the task off, and the ledger is still written). Each pass takes up to 20 ledger documents
(100,000 resources) whose `not_before` is at least `min_age` old, oldest first, across tenants.
In `hfs` `min_age` is the larger of 60 s and the Bundle deadline (`HFS_REQUEST_TIMEOUT` less 2 s,
898 s with a 900 s timeout), logged at startup as `min_age_secs`. A live request starts nothing
after that deadline, counted from its call, and the call precedes the document's `not_before`, so
a document older than `min_age` has no live writer and one instance's repair never duplicates a
step that is still running.

- The repair claims each document before working on it, by moving its `not_before` to now only
  while it still holds the value the pass read. A second repair (another instance, or an
  overlapping call) that read the same document finds the claim taken and skips it: that is the
  `skipped` field of `DeferredIndexRepairReport`, which is not counted in `examined`. A claimer
  that crashes leaves a document that is due again after `min_age`.
- It runs the same marker-gated batches with no deadline. A batch that runs out of attempts does
  not stop the document: the healthy batches are still written (up to three failed batches in a
  row, so a MongoDB that is down does not cost every attempt of every batch), and the document is
  kept, with `not_before` set to now, `last_error` (at most 1,024 characters) and `attempts`
  increased, and is retried after another `min_age`.
- A document with no `refs` array, or with refs that cannot be read, is kept the same way, after
  its readable refs are indexed. Documents are never dropped before all of their resources are
  indexed. Every instance may run the repair.
- Correctness does not depend on the claim or on `min_age`: the rewrite is gated on the marker and
  serialized by the write conflict on the resource document, so a repair that raced a live step
  would only repeat work.

An application that embeds `MongoBackend` without the `hfs` binary must call
`MongoBackend::repair_deferred_search_index` periodically itself (`hfs` does so at startup and
every interval), or set the threshold to `off`. Otherwise the rows a crash or a deadline cut left
unwritten are never written.

- A pass that found work logs "Deferred search index repair pass finished" at `INFO` (at `WARN` if
  any document failed) with `examined`, `skipped`, `resources`, `reindexed`, `rows`, `cleared`,
  `failed`, `remaining` and `pass_ms`. Each failing document logs a `WARN`, "deferred search index
  repair could not finish a ledger chunk", with `tenant`, `bundle_id`, `chunk`, `attempts` and
  `error`. A pass that could not run at all logs "Repair of deferred MongoDB search rows could not
  run".
- At startup, if nothing was due but the ledger is not empty, it logs "Deferred search index
  ledger holds chunks younger than the repair's minimum age; the next pass takes them".
- To see what is owed, run `db.search_index_pending.countDocuments()` (or call
  `MongoBackend::count_deferred_search_index_chunks()`). It is `0` when every deferred Bundle is
  fully indexed.

**Tenant purge.** `purge_tenant_data` deletes the tenant's ledger documents first, then its
`resources`, then its `search_index`, `search_index_contained` and `resource_history` rows, so a
post-commit step or a repair that runs during the purge leaves none of the tenant's rows behind
(#2259):

- A repair pass that claims after the ledger delete finds none of the tenant's documents. A pass
  that claimed one before it runs its batches, which the next two bullets cover.
- A batch that has cleared a resource's marker holds that document until it commits, so the
  `resources` delete waits for the commit and the row deletes that follow remove what the batch
  wrote.
- A batch that read a resource the purge then deleted cannot clear its marker (its update loses the
  write conflict on the deleted document), so it aborts and its retry finds nothing to index.
  Neither the step nor the repair inserts a ledger document or a resource.

Client writes to the tenant during the purge, and a `$reindex` of its types, are not ordered
against it. A purge that fails part-way can leave rows, history or ledger documents without their
resources. Retrying `DELETE /admin/tenants/{id}?purge=true` removes them: with `?purge=true` the
endpoint runs the purge even once the tenant is deregistered and has no live resources, and answers
`200` (#2309). `resources_removed` then counts only what the retry removed, so it is `0` when the
first attempt had already deleted the resources. Calling `purge_tenant_data` again does the same.

**Out of scope.** Batch Bundles, single create/update/delete writes and `$bulk-submit` are
unchanged. A Bundle whose resources and history alone do not fit in the cache still fails with
`500` `too-costly`, as described above.

**Ceiling test.** An ignored test starts its own `mongo:7.0` replica set with a small WiredTiger
cache and sends one lone synthetic Bundle (`COPIES` copies of a 267-entry fixture, so 15 copies are
4,005 entries) through one backend per variant, each on a database of its own:

- `inline`: the deferral off; expected too large for the cache.
- `deferred`: the default threshold; expected to commit, its rows written after the commit.
- `deferred_no_metadata`: `deferred` with `missing_metadata=false`, so no resource carries
  `missing_params` or its fingerprint and `idx_resources_missing_params` is not built.
- `offloaded`: search offloaded, so the transaction holds only the resources and their history.

```bash
cargo test -p helios-persistence --features mongodb --test mongodb_tests \
  deferred_indexing_raises_the_lone_bundle_ceiling -- --ignored --nocapture
```

Before the variants it runs a write-set probe: one copy of the fixture (267 entries) through
`inline`, `deferred_txn` (the deferral with its post-commit step cut by a 1 ms deadline, so the
documents are as the transaction wrote them), `deferred_no_metadata` and `offloaded`, counting
index keys per resource with `validate`. It prints one `DEFERRED_CEILING_PROBE` line per mode
(`missing_keys_per_resource` and `fp_keys_per_resource` are the keys of
`idx_resources_missing_params` and `idx_resources_missing_params_fp`, `resource_keys_per_resource`
all keys of `resources`), and then one `DEFERRED_CEILING` line per variant with `outcome`,
`cause`, `abort_entry` (the entry the transaction ended at, `commit`, or `na` for a committed
Bundle), `secs`, `entries_per_s` (`na` when there is no abort entry to divide by), the resources,
rows, ledger documents and markers left, and the server's eviction-rollback delta.

`abort_entry` is accurate to about one create chunk (100 entries): a failed chunk is charged to the
entry the server names, or else to its first entry, so it can read up to 99 entries low. A 388
while the ledger is written reads as `abort_entry=<entries>`, because `ledger_attempt_error` reuses
`entry_attempt_error(entries, entries, ..)`.

Knobs (defaults in parentheses): `HFS_TEST_DEFERRED_CEILING_WT_CACHE_GB` (`0.5`),
`HFS_TEST_DEFERRED_CEILING_COPIES` (`15`), `HFS_TEST_DEFERRED_CEILING_VARIANTS` (`inline,deferred`),
`HFS_TEST_DEFERRED_CEILING_PROBE` (`true`), `HFS_TEST_DEFERRED_CEILING_TXN_LIFETIME_SECS` (`900`,
mongod's `transactionLifetimeLimitSeconds`), `HFS_TEST_DEFERRED_CEILING_MONGODB_URL` (an external
replica set; no container is started), `HFS_TEST_DEFERRED_CEILING_EXPECT_INLINE_ABORT` (`true`;
`false` accepts any inline outcome) and `HFS_TEST_DEFERRED_CEILING_EXPECT_COMMIT` (`true`; `false`
accepts any outcome of the other variants, to find where they abort). An aborted Bundle must leave
no resource, ledger or marker whatever the knobs. Speed depends on the load of the host. On a shared
12-core host with a debug build, a committing 4,005-entry Bundle ran at 48-54 entries per second
(74.1 s, 79.9 s and 83.7 s), and the variants that aborted ran at 51-401 entries per second up to
the abort, so an aborting variant took about 90-245 s at 2 GB. A 28,035-entry Bundle that committed
at 2 GB, rows included, ran at 79.9 and 80.7 entries per second (351.1 s and 347.5 s). Set `TXN_LIFETIME_SECS` to `7200`
for a run that may last longer than the 900 s default, or a variant ends at the server's transaction
lifetime instead of at the cache.

Recorded runs, each cell three times (`offloaded` before this change at 2 GB twice), on a shared
12-core host with a debug build. The third `deferred` run before this change at 2 GB used
`HFS_TEST_DEFERRED_CEILING_COPIES=100` (26,700 entries), which still aborts before its end. At 0.5 GB:
`HFS_TEST_DEFERRED_CEILING_COPIES=40` (10,680 entries),
`HFS_TEST_DEFERRED_CEILING_EXPECT_COMMIT=false`, and
`HFS_TEST_DEFERRED_CEILING_VARIANTS=inline,deferred,deferred_no_metadata,offloaded` for the runs
before this change or `inline,deferred,offloaded` for those after it; the probe ran with them, at
its default. At 2 GB: `HFS_TEST_DEFERRED_CEILING_WT_CACHE_GB=2`,
`HFS_TEST_DEFERRED_CEILING_COPIES=150` (40,050 entries),
`HFS_TEST_DEFERRED_CEILING_VARIANTS=deferred,offloaded`,
`HFS_TEST_DEFERRED_CEILING_EXPECT_INLINE_ABORT=false`,
`HFS_TEST_DEFERRED_CEILING_EXPECT_COMMIT=false`,
`HFS_TEST_DEFERRED_CEILING_TXN_LIFETIME_SECS=7200` and `HFS_TEST_DEFERRED_CEILING_PROBE=false`.
The probe, per resource (the same in both 0.5 GB runs of each mode):

```text
mode                 with_missing_params  missing_keys  fp_keys  resource_keys  avg_resource_bytes
inline                               267         15.19     1.00          21.19                2034
deferred_txn                           0          1.00     0.00           6.00                1736
deferred_no_metadata                   0          0.00     0.00           5.00                1736
offloaded                              0          1.00     0.00           6.00                1674
```

and before this change `deferred_txn` had `with_missing_params=267`, `missing_keys=15.19`,
`fp_keys=1.00`, `resource_keys=21.19` and `avg_resource_bytes=2096`. The variants (the abort entry
of each run; `inline` is the same code before and after, six runs):

```text
variant                        cache  entries  outcome    abort_entry
inline                         0.5      10680  too_large  1846-1851 (six runs)
deferred, before this change   0.5      10680  too_large  5778, 5780, 5729
deferred_no_metadata, before   0.5      10680  too_large  9211, 9178, 9085
deferred, after                0.5      10680  too_large  8911, 8871, 8933
offloaded, before              0.5      10680  too_large  9040, 8936, 8963
offloaded, after               0.5      10680  too_large  9022, 9036, 9011
deferred, before this change   2        40050  too_large  22967, 22972, 22898 (of 26700)
deferred, after                2        40050  too_large  35481, 35346, 35406
offloaded, before              2        40050  too_large  35878, 35921
offloaded, after               2        40050  too_large  35943, 35891, 35884
```

Every abort was `write_set_too_large`, with `resources=0`, `ledger=0` and `markers=0` after it.
`deferred_no_metadata` stands in for a deferred Bundle that writes no `missing_params` (it also
skips the one `null` key per resource that the offloaded write set has, 5 keys per resource
against 6). Before this change it reached 9,085-9,211 against the offloaded 8,936-9,040, which is
the evidence that `missing_params` and its keys were the difference. A run at the defaults (15
copies, 4,005 entries, 0.5 GB, the asserting configuration) gave `inline` too large for the cache at
entry 1,851 and `deferred` committed, with `resources=4005`, `rows=63990`, `ledger=0` and
`markers=0`.

The Bundle that commits near the ceiling, at 2 GB with 105 copies (28,035 entries; 79% of
the deferred abort entry above, so the abort entry of `deferred` is not reached):

```bash
HFS_TEST_DEFERRED_CEILING_WT_CACHE_GB=2 HFS_TEST_DEFERRED_CEILING_COPIES=105 \
HFS_TEST_DEFERRED_CEILING_VARIANTS=inline,deferred HFS_TEST_DEFERRED_CEILING_PROBE=false \
HFS_TEST_DEFERRED_CEILING_EXPECT_INLINE_ABORT=true HFS_TEST_DEFERRED_CEILING_EXPECT_COMMIT=true \
HFS_TEST_DEFERRED_CEILING_TXN_LIFETIME_SECS=7200 \
cargo test -p helios-persistence --features mongodb --test mongodb_tests \
  deferred_indexing_raises_the_lone_bundle_ceiling -- --ignored --nocapture
```

The test passed, in 553 s, and printed the lines below. A second run, of `deferred` only,
committed the same Bundle in 347.5 s (80.7 entries per second), again with `resources=28035`, `rows=447930`, `ledger=0` and `markers=0`.

```text
DEFERRED_CEILING variant=inline wt_cache_gb=2 entries=28035 outcome=too_large cause=write_set_too_large abort_entry=7349 secs=77.4 entries_per_s=95.0 resources=0 rows=0 ledger=0 markers=0 wt_rollbacks=1
DEFERRED_CEILING variant=deferred wt_cache_gb=2 entries=28035 outcome=ok cause=na abort_entry=na secs=351.1 entries_per_s=79.9 resources=28035 rows=447930 ledger=0 markers=0 wt_rollbacks=0
```

So at 2 GB the deferred Bundle committed with its rows written and nothing left behind, where the
inline one aborted at about a quarter of its size. 28,035 entries is under the 30,000 of #1975's
target, but this is one synthetic Bundle in a harness, not the benchmark's `mongodb` leg, which has
not been run.

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

The S3 backend is a storage-focused persistence backend using AWS S3 object storage. It handles CRUD, versioning/history, and the whole `$bulk-submit` surface, but is intentionally not a FHIR search engine. For bulk export, S3 participates in two narrower roles: `S3Backend` can provide resource batches for system-level exports, and `S3OutputStore` can store finalized NDJSON output files; bulk-*export* job state, progress, manifests, leases, and file metadata are not stored in S3, and live in SQLite or PostgreSQL. `$bulk-submit` job state *is* stored in S3, alongside the submission objects it belongs to.

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
| `default_keyspace_layout` | `KeyspaceLayout::V1` | Layout given to new, empty tenants (`V1` flat, `V2` sharded); an existing tenant keeps what its marker or data says |
| `scan_concurrency` | `32` | Most `GetObject` requests the read fan-outs (SQL-on-FHIR scans and by-id reads, type counts, conditional-create and SearchParameter scans, `_history`) may have in flight at once, shared by every request on the backend; at least 1. Writes keep their own fan-out |

### Tenancy Modes

| Mode | Description |
|------|-------------|
| **PrefixPerTenant** | All tenants share one bucket with tenant-specific key prefixes |
| **BucketPerTenant** | Each tenant maps to a specific bucket via an explicit tenant→bucket map; an unmapped tenant is refused (`TenantError::UnmappedTenant`). In the `hfs` binary: `HFS_S3_TENANCY=bucket-per-tenant` + `HFS_S3_TENANT_BUCKETS` (+ optional `HFS_S3_SYSTEM_BUCKET`), #1598 |

### Object Model

Resource objects:

| Object | Key Pattern |
|--------|-------------|
| Current pointer | `.../resources/{type}/{id}/current.json` |
| Immutable history version | `.../resources/{type}/{id}/_history/{version}.json` |
| Type history event | `.../history/type/{type}/{ts}_{id}_{version}_{suffix}.json` |
| System history event | `.../history/system/{ts}_{type}_{id}_{version}_{suffix}.json` |

Under keyspace layout `V2` the resource and type-history keys gain a shard segment and the system history index is not written:

| Object | `V2` key pattern |
|--------|-------------|
| Current pointer | `.../resources/{type}/{h2}/{id}/current.json` |
| Immutable history version | `.../resources/{type}/{h2}/{id}/_history/{version}.json` |
| Type history event | `.../history/type/{type}/{h2}/{ts}_{id}_{version}_{suffix}.json` |
| System history event | *(none: system history lists `history/type/`)* |
| Layout marker | `.../history/_keyspace.json` (`{"layout":2}`) |

`h2` is the first two hex characters of `sha256(id)`. The layout is per tenant and immutable. See `HFS_S3_KEYSPACE_LAYOUT` above.

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

### Bulk Submit Result Receipts

When a manifest finishes, its worker turns the stored per-entry results into `output` receipts — one
NDJSON part per resource type, alphabetically ordered with a dense part index — plus a single
aggregated `error` part. Those parts are built through temporary spool files rather than in-memory
vectors, so a submission with millions of entries no longer grows the worker's heap with its whole
receipt set. All backends share the same path.

- Memory for receipt construction is `O(Pmax + Lmax + B + M)`. `Pmax` is the largest materialized
  entry page in bytes; `Lmax` is the largest serialized entry in bytes; `B` is fixed I/O workspace,
  including the local output writer; `M` is retained metadata in bytes for resource types, issue
  severities, artifacts, and input files. Requesting at most 1000 results limits the row count,
  not page or entry bytes. This change adds no per-entry size cap.
- Up to 8 spool files stay open (64 KiB writer buffer each; the underlying Tokio file copy buffer and
  the fixed replay chunk are each bounded by the same 64 KiB). The least recently written spool is
  flushed and closed before another opens, so a manifest with more resource types than that never
  widens the handle set. Replay opens one reader at a time, after every writer is closed.
- Replay copies each spool to the output store in fixed 64 KiB chunks instead of re-serializing or
  parsing rows, and assigns the part's line and byte counters from the spool bookkeeping those bytes
  were counted with. A spool that does not replay exactly its counted bytes fails the manifest
  rather than publishing counts that disagree with the artifact.
- Spools live in a per-run temporary directory (`TMPDIR` picks its parent on Unix) that is removed when the
  run ends, including on lease loss and cancellation. An abrupt process kill can leave one behind,
  and disk use grows with the manifest's receipt bytes (rows plus one newline each) while it runs.

This bounds the receipt path only, and is not a whole-process bound: a `deleted` file's references
are still collected in memory, the S3 **primary** backend loads a manifest's stored results as one
object, the S3 output store writes each part to a scratch file and then reads that whole file back
into memory for the upload, and composite `finish_manifest` synchronization (including the
`mongo-es`/`s3-es` wrappers) still materializes the resource set. The REST artifact download handler
also reads the requested part into memory before returning it.

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

- `MINIO_IMAGE` (default: `ghcr.io/coollabsio/minio`)
- `MINIO_TAG` (default: `RELEASE.2025-10-15T17-29-55Z`)
- `MINIO_ROOT_USER` (default: `minioadmin`)
- `MINIO_ROOT_PASSWORD` (default: `minioadmin`)
- `HFS_MINIO_TEST_BUCKET` (if unset, tests auto-generate a unique bucket)

The default GHCR image replaces the Quay image after anonymous pulls from Quay
started returning 401 in CI ([run 36024486722](https://github.com/HeliosSoftware/hfs/actions/runs/36024486722)).

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

### Phase 3: Tenant Isolation

- [x] Shared-schema isolation via a `tenant_id` discriminator, with a mandatory
      `TenantContext` on every tenant-scoped `ResourceStorage` operation
- [x] Per-tenant physical isolation for S3 (`S3TenancyMode::BucketPerTenant`)
- [ ] Row-Level Security as a PostgreSQL database-enforced layer — not
      implemented; tracked in [#381](https://github.com/HeliosSoftware/hfs/issues/381)
- Schema-per-tenant and database-per-tenant for SQL backends: **not planned**
  (design discussion #28). Unwired SQL generators removed in #370.

### Phase 4: SQLite Backend ✓

- [x] Connection pooling (r2d2)
- [x] Schema migrations
- [x] ResourceStorage implementation
- [x] VersionedStorage implementation
- [x] History providers (instance, type, system)
- [x] TransactionProvider implementation
- [x] Conditional operations (conditional create/update/delete)

#### Transaction Support ◐

FHIR [transaction](https://build.fhir.org/http.html#transaction) bundle processing, via `BundleProvider`.

> **Backend Support:** Transaction bundles require ACID support. SQLite supports transactions. Cassandra, Elasticsearch, and S3 do not support transactions (batch only). See the capability matrix above.

> **Where batch lives:** [batch](https://build.fhir.org/http.html#batch) Bundles are *not* processed by this crate. Each entry is authorized individually against the request's SMART scopes and emits its own audit event, neither of which the persistence tier can see, so the REST layer executes entries directly against `ResourceStorage` — with bounded concurrency taken from `bulk_write_concurrency()`. `BundleProvider` carried an unreachable `process_batch` until #501 removed it. Batch therefore works on **every** backend that implements `ResourceStorage`, independent of transaction support.

**Implemented Features:**

- [x] **Transaction bundles** - Atomic all-or-nothing processing with automatic rollback on failure
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

**Number and quantity prefix semantics:**

- `eq`/`ne` bound the *implicit-precision range* of the search value as
  written: the number of significant decimals sets the range's width, so
  `value=100` matches `[99.5, 100.5)` while `value=100.0` matches the
  narrower `[99.95, 100.05)`. `ne` matches everything outside that range.
- `gt`, `lt`, `ge`, `le`, `sa`, `eb` compare against the exact search value
  and ignore its written precision, per the [FHIR number search
  spec](https://hl7.org/fhir/R4/search.html#number): *"When a comparison
  prefix in the set gt, lt, ge, le, sa & eb is provided, the implicit
  precision of the number is ignored, and they are treated as if they have
  arbitrarily high precision."* So `value-quantity=gt60` and
  `value-quantity=gt60.0` are equivalent. On SQLite, PostgreSQL, and
  Elasticsearch, quantity comparators additionally apply the rule after UCUM
  unit conversion (e.g. `gt60|http://unitsofmeasure.org|kg` and
  `gt60000|http://unitsofmeasure.org|g` are equivalent). MongoDB does not
  perform UCUM conversion: it compares the raw stored quantity value against
  the search value as written, and filters on the unit code verbatim, so on
  MongoDB `gt60000|http://unitsofmeasure.org|g` matches nothing against data
  stored as `kg`.
- The eq/ne-vs-comparator precision rule itself (implicit-precision range for
  `eq`/`ne`, exact value for the other comparators) is unified across
  SQLite, PostgreSQL, Elasticsearch, and MongoDB. The exception is `ap`
  (approximately equal), whose tolerance is not unified across backends.
- Date parameters are unaffected by this rule and keep their own
  calendar-precision range comparison.

**Date prefix semantics (range targets, #1391):**

- Every indexed date value is a range `[start, end)`. A point value such as
  `birthDate` or `effectiveDateTime` runs to the end of its own precision
  (`2020-03` is `[2020-03-01, 2020-04-01)`). A `Period`, and a `Timing`'s
  `repeat.boundsPeriod`, is **one** range from its `start` to the end of its
  `end` at that value's precision. A missing `start` or `end` is unbounded,
  stored as the supported-range limit (year 1 / year 9999). One rule for all
  four backends: a Period whose `start` or `end` is not a valid FHIR
  date/dateTime (as the search grammar reads it, so minute precision is fine
  but `2024-03-15T10Z` or `+0530` is not) is not indexed at all, rather than
  read as open. Before #1391 each end was indexed as its own point and a
  backend could be lenient about the format.
- With search range `[s, e)` and indexed range `[ts, te)`, all four backends
  apply the FHIR rules for range targets: `eq` `s ≤ ts ∧ te ≤ e`, `ne` its
  negation, `gt` `te > e`, `lt` `ts < s`, `ge` `gt ∨ eq`, `le` `lt ∨ eq`,
  `sa` `ts ≥ e`, `eb` `te ≤ s`. So `date=2020` no longer finds a Period that
  merely starts or ends in 2020, and `date=gt2030` finds a Period with no end.
- `ap` matches when the ranges overlap once the search range is widened by a
  margin that follows its precision, shared by all backends: 1 year, 1 month,
  1 day, 10 minutes, or 10 seconds for year, month, day, minute and
  second-or-finer values.
- Descending `_sort` on a date parameter orders by the largest range end.
- SQLite stores the range as two normalized text columns, `value_date_start`
  (added by the v39 migration) and `value_date_end`, in the same
  `YYYY-MM-DD HH:MM:SS.SSS` UTC form, and compares both as plain `TEXT` with no
  SQL function per row. Two partial indexes cover them:
  `idx_search_date_start (…, value_date_start, value_date_end, resource_key)`
  and `idx_search_date_end (…, value_date_end, value_date_start, resource_key)`,
  both covering for the `resource_key IN (…)` subquery. The query still
  adds the implied `end > start-bound` term to every group that bounds the
  start, so `eq`, `gt` and `sa` seek. Before v39 the start was normalized with
  `strftime` on every row, which made `lt`, `ge`, `le`, `ne`, `eb` and `ap`
  slower than before #1391 on very large date slices (#1506). The v39 backfill
  runs in batches of 10 000 rows, one transaction per batch, and resumes after
  a crash.
- `_lastUpdated` and the date components of composite parameters are still
  compared as points.
- SQLite rows indexed before `value_date_precision` existed (or by the fallback
  indexer) have no recorded precision, and a padded `T00:00:00` value cannot
  say whether it was a year, month, day or second. The v35 backfill does not
  guess: such a row gets a one-second range, the migration logs how many it
  filled, and `ne`/`ap` on them can differ until `$reindex`.
- **After upgrading, run `$reindex`.** The SQLite (v35, v39) and PostgreSQL (v43)
  migrations fill in the range end (and, on SQLite, the start) of existing
  rows, so point values keep working, but a Period indexed before the upgrade
  is still two point rows until the resource is reindexed. MongoDB and
  Elasticsearch documents indexed before the upgrade have no range end at all,
  so they fail every condition on it (`eq`, `ne`, `gt`, `ge`, `le`, `eb`,
  `ap`) until reindexed. Reindexing can also change which resources match: the
  two point rows of an old Period matched on either end, while the single range
  row follows the range-target rules (a Period starting before `2000` and
  ending inside it no longer matches `eq2000`).
- On a very large date slice `le` and `ne` match most rows of the parameter,
  so their cost follows the size of the result, and they return more rows than
  before #1391 for the same data (a year-precision value now covers the whole
  year). They still run faster than before #1391 (#1506):
  - the two ranges of `ne`, `le` and `ge` are one `SELECT` each, joined by
    `UNION ALL`, so each reads its covering index alone; joined by `OR`,
    SQLite de-duplicated every row by rowid and, for `ne`, read each row from
    the table;
  - a search on one parameter uses that parameter's `search_index` subquery as
    the whole filter, without the `SELECT DISTINCT` wrapper that probed
    `idx_search_composite` once per matching key;
  - a filtered one-row page (`_count=0` or `1`) reads its matches by rowid
    instead of probing `idx_resources_live_type` first.

  On 2M date rows (debug build) `le2000` takes 2.3 s against 4.6 s before
  #1391 and `ne2000` 2.7 s against 5.4 s with `_count=1`; with `_count=100`,
  1.2 s against 2.6 s and 1.6 s against 3.1 s.
- **PostgreSQL v43 does not hold a long lock.** It adds `value_date_end` and
  the statistics object in short transactions (2 s `lock_timeout`, retried),
  backfills in committed batches of 2,000 heap blocks selected by `ctid` range
  (resumable: only rows with a `NULL` end are touched), builds each index with
  `CREATE INDEX CONCURRENTLY` under a temporary name and swaps it in with a
  short `DROP`/`RENAME`, so date searches always have an index. The schema
  version is written only after the last step, and every step is idempotent, so
  a crash resumes on the next startup. Like v49, the concurrent builds wait for
  older transactions (a long export delays startup) and block other DDL and
  `VACUUM` while they run. The closing `ANALYZE` is best-effort and bounded
  (30 s): if it fails or times out, v43 is still recorded, a warning says
  autovacuum will fill in the statistics, and the session keeps its own
  `statement_timeout`. Each batch is a TID range scan, which needs PostgreSQL
  14 — on 13 every batch reads the whole table, so the backfill grows with the
  square of its size — and 13 cannot run v42's reference index either, so
  startup refuses servers older than 14 before changing anything.
- **Elasticsearch mappings are reconciled at startup.** `hfs` runs the
  backend's initialization (index template, nested-object limit, mapping
  reconcile) once before serving in every `*-elasticsearch` mode, so a
  version-1 index is brought up to date, and the "reindex needed" warning is
  logged once per startup pass. If Elasticsearch is unreachable at startup the
  failure is logged and the server still starts; indices are then reconciled on
  their first write, with one warning per index. If the cluster answers but one
  startup step fails (for example the index template), that step is logged on
  its own and the other two still run.

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
- [x] Parameter type handlers (string, token, date, number, quantity, reference, URI, composite —
      each composite instance is one nested object with inline component values, matched by a
      single nested query)
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
- [x] SearchProvider with all parameter types including composite (string, token, reference, date,
      number, quantity, URI, composite; supports `:exact`/`:contains`/`:not`/`:missing`/`:of-type`
      and URI `:above`/`:below`; the `:text-advanced` modifier is not yet implemented). Composite
      search works end-to-end (REST → registry-resolved components → grouped index → query).
- [x] Chained search and reverse chaining (\_has), through the shared resolver
- [x] Full-text search (tsvector/tsquery)
- [x] `_sort` by `_id`/`_lastUpdated` and any indexed search parameter via a search_index
      correlated subquery (first-page and offset paths)
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

#### Search visibility on Elasticsearch-backed composites

When Elasticsearch is the search secondary, a write can return before the
resource is findable via search. Two independent delays contribute:

1. **Composite forwarding.** In `Asynchronous` mode the composite returns as
   soon as the primary commits and forwards the index write to Elasticsearch on
   a background worker. `Synchronous` (or `Hybrid { sync_for_search: true }`)
   puts the document in Elasticsearch before the write returns.
2. **Elasticsearch refresh.** An indexed document only becomes searchable at the
   next index refresh. By default that happens on a timer
   (`ElasticsearchConfig::refresh_interval`, set when an index is created and
   reconciled onto every existing `{index_prefix}_*` index at startup, which
   overwrites a value set by hand with `PUT _settings`). With `WaitFor` the
   interval is the price of every write: a long one makes each interactive
   write wait for the next refresh (#1749).
   `ElasticsearchConfig::write_refresh`
   sets the `refresh` parameter on each index/delete request instead:
   `WriteRefreshPolicy::False` (default) leaves it to the timer, `WaitFor`
   blocks the write until the affected shards refresh, and `True` forces a
   refresh per write (expensive; low-volume deployments only).

Read-after-write search therefore requires **both** a synchronous sync mode and
`write_refresh: WaitFor`; either alone still leaves a window in which an
immediate follow-up search misses the write. The `hfs` binary exposes these as
`HFS_COMPOSITE_SYNC_MODE`, `HFS_ELASTICSEARCH_REFRESH_INTERVAL`, and
`HFS_ELASTICSEARCH_WRITE_REFRESH` (see the
[hfs README](../hfs/README.md#environment-variables)).

That window applies to *client* searches. Server-side lookups that resolve a
write against existing content do not inherit it: a transaction Bundle's
conditional references (`"reference": "Organization?identifier=…"`) and the
composite's conditional create/update/delete (`If-None-Exist`,
`PUT [type]?[criteria]`) first call
`SearchProvider::ensure_writes_visible` for the types they name. The composite
drains its asynchronous sync queue up to that point, then the Elasticsearch
backend refreshes those indices (a no-op under `WaitFor`/`True`, where the
write was already searchable when it returned). A resource the server has
acknowledged is therefore always found by such a lookup, on every sync mode
and every `write_refresh` setting, and the cost is paid only by requests that
carry such criteria (#1047).

#### Very large resources on Elasticsearch-backed composites

Every indexed search-parameter value is a nested object in the resource's
Elasticsearch document, and Elasticsearch rejects the **whole document** once it
holds more than `index.mapping.nested_objects.limit` of them. The resource stays
stored in the primary and readable by id, but is absent from every search.
Elasticsearch's own default of 10000 is exceeded by real data: 458 of the 11,704
Synthea `Provenance` resources carry more than 10000 `target` references (the
largest, 28,192).

`ElasticsearchConfig::nested_objects_limit` (default 50000) is written into the
index template, so new indices get it, and is raised during backend
initialization on existing indices that are below it. The setting is dynamic,
so resources that already indexed need no reindex. Resources rejected before the
raise can be reindexed with `POST /{type}/$reindex`, but raising the limit alone
does not guarantee that repair: `$reindex` sends the same documents through the
`_bulk` path described below. Before #1125, on `sqlite-elasticsearch`, every
500-document `Provenance` request failed as a whole at the transport level
(`backend unavailable: elasticsearch`, `retryable: true`), so `$reindex` indexed
none of them. Treat a rebuild as a repair only once `GET /$reindex-status/{job_id}`
reports `errorCount` 0 for the type. The `hfs` binary exposes the limit as
`HFS_ELASTICSEARCH_NESTED_OBJECTS_LIMIT`.

A search that pins many ids — a chained or `_has` search whose terminal hop
resolved a wide set, sent to Elasticsearch as `_id` values — meets a second
ceiling, `index.max_terms_count` (Elasticsearch's default 65,536 values per
`terms` query). `ElasticsearchConfig::max_terms_count` (default 65536) is written
into the index template, and the query builder splits an id list longer than it
into several `terms` clauses ORed under one `bool.should`, so such a search
succeeds instead of being refused (#1548); when Elasticsearch still refuses a
query for a limit it names, the `400` names that limit. The `hfs` binary exposes
it as `HFS_ELASTICSEARCH_MAX_TERMS_COUNT`.

#### Bulk writes and rebuilds on Elasticsearch-backed composites

`create_many` and `ReindexTarget::write_search_entries_page` put documents on the
wire through one `_bulk` path. Each request is capped at 500 operations **and**
at `ElasticsearchConfig::bulk_max_bytes` (default 10 MiB); a single document
larger than the byte cap is sent alone. The cap exists because the client's
`ElasticsearchConfig::request_timeout_ms` (default 30000) covers the whole
request: 500 Synthea `Provenance` resources average ~108 KB each, and the ~54 MB
body that produced timed out on every attempt.

Failures are classified by shape:

- **Transport timeout, `413`, or a proxy's `408`/`504`**: the chunk is split in
  half and each half is resent, recursively, down to a single document, so an
  oversized chunk is never resent unchanged nor reported as hundreds of
  rejected resources. A single document still answered `413` is permanent; one
  that still times out is transient, and on the client's own timeout the rest
  of the page is failed as transient too rather than split further (the
  cluster is stalled, and every halving would wait out another timeout).
- **`429`**, for the whole request or per item: retried with bounded exponential
  back-off, resending only the rejected items.
- **Connection error, `5xx` for the whole request, or a `429` that outlasts its
  retries**: every document of that request fails as transient.
- **Per-document `4xx`** (for example the nested-object limit above): permanent,
  so a rebuild that has only those is not retried.
- **`ensure_index` transport failures**: transient (`BackendError::Unavailable`),
  not `Internal`.

A client-side timeout does not cancel the `_bulk` Elasticsearch is already
executing, so a resource reported as failed may still end up indexed; confirm
with a count before rebuilding again.

`ElasticsearchConfig::reindex_refresh` sets the `refresh` parameter for rebuild
writes separately from `write_refresh`; `None` (default) follows `write_refresh`.

On a primary whose search is offloaded (`set_search_offloaded(true)`), the
reindex writers `write_search_entries_on`, `write_search_entries_page` and
`clear_search_index` are no-ops, as `begin/end_bulk_index_rebuild` already were,
and `hfs` wires only Elasticsearch as the reindex target on
`sqlite-elasticsearch`. Before #1125 the rebuild also wrote the SQLite
`search_index`/FTS rows that no query there reads, and because the matching
delete was already a no-op the rows accumulated on every run (≥ 11 KB per
resource). A SQLite row `fetch_resources_page` cannot parse is reported as a
per-resource error and no longer ends the rebuild of its type.

A manifest ingested with indexing deferred records that it still owes a
rebuild, in `bulk_manifests.index_pending` (SQLite schema v30), inside the
transaction that publishes it. `SubmitWorkerStorage::list_manifests_awaiting_reindex`
is what a restarted server scans to re-fire those rebuilds, and the marker is
cleared once a generation finishes; backends that do not implement the three
defaulted methods simply never resume, as before #1125.

The `hfs` binary exposes these as `HFS_ELASTICSEARCH_REQUEST_TIMEOUT_MS`,
`HFS_ELASTICSEARCH_BULK_MAX_BYTES`, `HFS_ELASTICSEARCH_BULK_CONCURRENCY`,
`HFS_ELASTICSEARCH_CONTAINED_SWEEP_CONCURRENCY` and
`HFS_ELASTICSEARCH_REINDEX_REFRESH`, and the deferred rebuild's page as
`HFS_REINDEX_BATCH_SIZE` plus `HFS_REINDEX_BATCH_BYTES` (see the
[hfs README](../hfs/README.md#environment-variables)).

Measured on `sqlite-elasticsearch` with a 228,580-resource Synthea cut
(Elasticsearch 8.15, 4 GB heap): the rebuild completes in one generation with
every Provenance indexed, against `main` losing 2,500 of them and failing
twice. With `HFS_ELASTICSEARCH_WRITE_REFRESH=wait_for` the rebuild takes 806 s;
adding `HFS_ELASTICSEARCH_REINDEX_REFRESH=false` takes it to 145 s
(1,576 resources/s), which is the recommended pair.

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
