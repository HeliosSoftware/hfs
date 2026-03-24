# Architecture Overview

The Helios FHIR Server is organized as a Rust workspace with 13 modular crates. Each crate has a single, well-defined responsibility and can be used independently or composed together.

---

## Workspace Structure and Crate Map

| Crate | Binary / Package | Description |
|-------|-----------------|-------------|
| `helios-fhir` | *(library)* | Core FHIR data models. Auto-generated from HL7 StructureDefinition JSON. Supports R4, R4B, R5, R6 via feature flags. |
| `helios-fhir-gen` | `helios-fhir-gen` | Code generator — produces Rust structs from FHIR JSON schemas. Run when updating FHIR spec versions. |
| `helios-fhir-macro` | *(proc-macro)* | Procedural macros: `FhirSerde` (serialization) and `FhirPath` (evaluation integration). |
| `helios-fhirpath` | `fhirpath-cli`, `fhirpath-server` | FHIRPath 3.0.0-ballot implementation — parser (chumsky), evaluator, CLI, and HTTP server. |
| `helios-fhirpath-support` | *(library)* | Bridge between the FHIRPath evaluator and FHIR types. Provides `EvaluationResult`, `EvaluationError`, `IntoEvaluationResult`. |
| `helios-serde` | *(library)* | JSON (always) and XML (`xml` feature) serialization for FHIR resources. |
| `helios-serde-support` | *(library)* | Shared serde helpers used by `helios-serde` and `helios-fhir`. |
| `helios-rest` | *(library)* | FHIR RESTful API layer built on Axum — handlers, middleware, extractors, multi-tenancy routing. |
| `helios-persistence` | `config-advisor` | Polyglot persistence — backends (SQLite, PostgreSQL, Elasticsearch, MongoDB, S3), composite storage, search registry, tenant isolation. |
| `helios-hfs` | `hfs` | Main FHIR server binary. Wires `helios-rest` with storage backends and configuration. |
| `helios-sof` | `sof-cli`, `sof-server` | SQL-on-FHIR — ViewDefinition processing, CLI, and HTTP server. |
| `helios-cds-hooks` | *(library)* | CDS Hooks protocol types and `CdsHooksService` trait. |
| `pysof` | `pysof` (PyPI) | Python bindings for SQL-on-FHIR via PyO3 and maturin. Excluded from the default workspace build. |

All crates share the same version number. `pysof` is excluded from the default `cargo build` because it requires Python and maturin.

---

## Crate Deep Dives

### helios-fhir

The core FHIR data models library. Approximately 95% of its code is auto-generated from official HL7 StructureDefinition JSON schemas by `helios-fhir-gen`. The remaining 5% is hand-coded infrastructure:

- `Element<T>` — generic wrapper for FHIR primitives (value + optional extension)
- `DecimalElement` / `PreciseDecimal` — high-precision decimal handling
- `FhirVersion` — enum (`R4`, `R4B`, `R5`, `R6`) used throughout the workspace
- Choice type enums for polymorphic elements (e.g., `value[x]`)
- The `Resource` enum aggregating all resource types for a given version

Feature flags gate which versions are compiled:

```toml
[features]
default = ["R4"]
R4  = ["helios-fhir/R4"]
R4B = ["helios-fhir/R4B"]
R5  = ["helios-fhir/R5"]
R6  = ["helios-fhir/R6"]
```

### helios-fhirpath

A complete FHIRPath 3.0.0-ballot implementation. The architecture separates:

- **Parser** (`parser.rs`) — converts FHIRPath expressions into an AST using [chumsky](https://github.com/zesterer/chumsky), producing excellent error messages
- **Evaluator** (`evaluator.rs`) — walks the AST against FHIR resources with a runtime context
- **Type system** (`fhir_type_hierarchy.rs`) — manages FHIR and System type namespaces with version-aware resource type checking
- **Function modules** — each FHIRPath function category in its own `*.rs` file

The `EvaluationContext` is the primary runtime object:

```rust
use helios_fhirpath::evaluator::EvaluationContext;
use helios_fhir::FhirVersion;

// Version auto-detected from resources
let context = EvaluationContext::new(fhir_resources);

// Explicit version
let context = EvaluationContext::new_with_version(fhir_resources, FhirVersion::R5);
```

### helios-sof

ViewDefinition-based transformation engine. Version-agnostic through enum wrappers:

```rust
pub enum SofViewDefinition {
    R4(fhir::r4::ViewDefinition),
    R4B(fhir::r4b::ViewDefinition),
    R5(fhir::r5::ViewDefinition),
    R6(fhir::r6::ViewDefinition),
}
```

The same pattern is used for `SofBundle` and `SofCapabilityStatement`. Core function: `run_view_definition()`.

### helios-fhir-gen

The code generator that reads HL7 StructureDefinition JSON schemas and writes `r4.rs`, `r4b.rs`, `r5.rs`, `r6.rs` into `crates/fhir/src/`. You only need to run it when updating the FHIR spec version or changing code generation behavior. See [Code Generation](ch10-codegen.md).

### pysof

Python bindings built with [PyO3](https://pyo3.rs/) and [maturin](https://maturin.rs/). Exposes the `helios-sof` API to Python with automatic multithreading via Rayon (5–7× speedup on multi-core machines). Published to [PyPI](https://pypi.org/project/pysof/). See [Python Bindings](ch09-pysof.md).

---

## Design Principles

### Version-Agnostic Abstraction

Enum wrappers allow a single code path to handle all FHIR versions without runtime overhead:

```rust
pub enum SofViewDefinition {
    R4(fhir::r4::ViewDefinition),
    R4B(fhir::r4b::ViewDefinition),
    R5(fhir::r5::ViewDefinition),
    R6(fhir::r6::ViewDefinition),
}
```

Traits abstract over the enum variants so callers don't need to pattern-match:

```rust
// Works for any FHIR version
impl ViewDefinitionTrait for SofViewDefinition { ... }
```

### Persistence Trait Hierarchy

Storage backends implement a progressive trait hierarchy. You only implement the traits for the capabilities your backend supports:

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

Key traits:
- `ResourceStorage` — CRUD: create, read, update, delete
- `VersionedStorage` — ETag versioning, vread
- `SearchProvider` — parameterized search with modifiers
- `TransactionProvider` — atomic multi-resource transactions

### Tenant-First Design

Every persistence operation accepts a `TenantContext` as its first argument. Storage backends enforce tenant boundaries at the SQL/query level — there is no application-level post-filtering. This makes multi-tenancy correct by construction.

### Composite Storage

The `CompositeStorage` pattern combines two backends behind a single `ResourceStorage` interface:
- One backend handles **writes and CRUD** (SQLite, PostgreSQL, MongoDB, S3)
- The other handles **search queries** (Elasticsearch)

This allows you to use best-of-breed storage for each concern without changing any application code. Configured via `HFS_STORAGE_BACKEND` (e.g., `sqlite-elasticsearch`, `postgres-es`).
