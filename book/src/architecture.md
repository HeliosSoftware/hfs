# Architecture

The Helios FHIR Server is organized as a Rust workspace with modular, composable crates. Each component is designed for high performance and can be embedded directly into a data analytics pipeline.

## Workspace Structure

| Crate | Description |
|-------|-------------|
| `helios-fhir` | Core FHIR data models (auto-generated). Supports R4, R4B, R5, R6 via feature flags. |
| `helios-fhir-gen` | Code generator — produces Rust structs from FHIR JSON schemas. |
| `helios-fhir-macro` | Procedural macros for FHIR functionality. |
| `helios-fhirpath` | FHIRPath expression language — parser, evaluator, CLI tool, and HTTP server. |
| `helios-fhirpath-support` | Shared support utilities for FHIRPath. |
| `helios-serde` | JSON and XML serialization for FHIR resources. |
| `helios-serde-support` | Shared serde helpers. |
| `helios-rest` | FHIR RESTful API layer (Axum) — handlers, middleware, extractors, multi-tenancy routing. |
| `helios-persistence` | Polyglot persistence — backends (SQLite, PostgreSQL, Elasticsearch, MongoDB), composite storage, search registry, tenant isolation. |
| `helios-hfs` | Main FHIR server binary. Combines `helios-rest` with storage backends. |
| `helios-sof` | SQL-on-FHIR — ViewDefinition processing, CLI and HTTP server. |
| `helios-cds-hooks` | CDS Hooks protocol types and traits. |
| `pysof` | Python bindings (PyO3/maturin) for SQL-on-FHIR. |

## Design Principles

### Version-Agnostic Abstraction

Enum wrappers allow a single code path to handle multiple FHIR versions:

```rust
pub enum SofViewDefinition {
    R4(fhir::r4::ViewDefinition),
    R4B(fhir::r4b::ViewDefinition),
    R5(fhir::r5::ViewDefinition),
    R6(fhir::r6::ViewDefinition),
}
```

### Trait-Based Processing

Core functionality is defined through traits, enabling version-independent logic:

- `ViewDefinitionTrait`, `BundleTrait`, `ResourceTrait` (SOF)
- `ResourceStorage`, `VersionedStorage`, `SearchProvider`, `Transaction` (persistence)

### Persistence Trait Hierarchy

Storage backends implement a progressive trait hierarchy:

```
ResourceStorage
  ├── VersionedStorage
  │     ├── InstanceHistoryProvider
  │     ├── TypeHistoryProvider
  │     └── SystemHistoryProvider
  ├── SearchProvider
  │     ├── MultiTypeSearchProvider
  │     ├── ChainedSearchProvider
  │     └── IncludeProvider
  └── TransactionProvider
        └── BundleProvider
```

### Tenant-First Design

All persistence operations take a `TenantContext` as their first argument. Storage backends enforce tenant boundaries at the query level. See [Multi-Tenancy](configuration/multi-tenancy.md).

### Composite Storage

The `CompositeStorage` pattern combines backends (e.g., SQLite for CRUD + Elasticsearch for search) behind a single interface, configured via `HFS_STORAGE_BACKEND`.

## API Documentation

```bash
cargo doc --no-deps --open
```

Published crate docs are also available on [crates.io](https://crates.io/keywords/helios-fhir-server).
