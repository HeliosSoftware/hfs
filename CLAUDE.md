# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Architecture Overview

### Workspace Structure

The project is a Rust workspace with 17 crates (16 default-members; `pysof` excluded from the default build):

| Crate | Description |
|-------|-------------|
| **`helios-fhir`** | Core FHIR data models (auto-generated). Supports R4, R4B, R5, R6 via feature flags. |
| **`helios-fhir-gen`** | Code generator — produces Rust structs from FHIR JSON schemas. R6 specs auto-downloaded. |
| **`helios-fhir-macro`** | Procedural macros for FHIR functionality. |
| **`helios-fhirpath`** | FHIRPath expression language — parser (chumsky), evaluator, CLI tool, and HTTP server. |
| **`helios-fhirpath-support`** | Shared support utilities for FHIRPath. |
| **`helios-serde`** | JSON and XML serialization for FHIR resources (`xml` feature flag). |
| **`helios-serde-support`** | Shared serde helpers. |
| **`helios-rest`** | FHIR RESTful API layer (Axum) — handlers, middleware, extractors, multi-tenancy routing. |
| **`helios-persistence`** | Polyglot persistence — backends (SQLite, PostgreSQL, Elasticsearch, MongoDB), composite storage, search registry, tenant isolation. |
| **`helios-hfs`** | Main FHIR server binary. Combines `helios-rest` with storage backends. |
| **`helios-sof`** | SQL-on-FHIR implementation — ViewDefinition processing, CLI and HTTP server. |
| **`helios-hts`** | FHIR Terminology Server (HTS) — CodeSystem/ValueSet/ConceptMap operations and terminology import (SNOMED, LOINC, RxNorm, ICD-10-CM). Provides the `hts` binary. |
| **`helios-auth`** | Authentication & authorization — SMART-on-FHIR / OAuth2 JWT bearer validation, JWKS, scopes, JTI replay cache. Configured via `HFS_AUTH_*`. |
| **`helios-audit`** | Audit logging — FHIR AuditEvent with IHE BALP profiles; pluggable sinks (database, file, CloudWatch, S3). Configured via `HFS_AUDIT_*`. |
| **`helios-subscriptions`** | FHIR topic-based Subscriptions engine — rest-hook, websocket, email, and messaging channels. Configured via `HFS_SUBSCRIPTION(S)_*`. |
| **`helios-cds-hooks`** | CDS Hooks protocol types and async service trait (HL7 CDS Hooks v3.0.0-ballot). Standalone library. |
| **`pysof`** | Python bindings (PyO3/maturin) for SQL-on-FHIR. Excluded from default workspace build. |

### Binaries

| Binary | Crate | Description |
|--------|-------|-------------|
| `hfs` | helios-hfs | FHIR server |
| `fhirpath-cli` | helios-fhirpath | FHIRPath expression evaluator CLI |
| `fhirpath-server` | helios-fhirpath | FHIRPath HTTP evaluation server |
| `sof-cli` | helios-sof | SQL-on-FHIR CLI tool |
| `sof-server` | helios-sof | SQL-on-FHIR HTTP server |
| `config-advisor` | helios-persistence | Storage configuration advisor |
| `hts` | helios-hts | FHIR Terminology Server (HTS) |

### Key Design Patterns

#### Version-Agnostic Abstraction
The codebase uses enum wrappers and traits to handle multiple FHIR versions:

```rust
// Example from sof crate
pub enum SofViewDefinition {
    R4(fhir::r4::ViewDefinition),
    R4B(fhir::r4b::ViewDefinition),
    R5(fhir::r5::ViewDefinition),
    R6(fhir::r6::ViewDefinition),
}
```

#### Trait-Based Processing
Core functionality is defined through traits, allowing version-independent logic:
- `ViewDefinitionTrait`, `BundleTrait`, `ResourceTrait` (SOF)
- `ResourceStorage`, `VersionedStorage`, `SearchProvider`, `Transaction` (persistence)

#### Persistence Trait Hierarchy
Storage backends implement a progressive trait hierarchy:
```
ResourceStorage → VersionedStorage → InstanceHistoryProvider → TypeHistoryProvider → SystemHistoryProvider
ResourceStorage → SearchProvider → MultiTypeSearchProvider / ChainedSearchProvider / IncludeProvider
ResourceStorage → TransactionProvider → BundleProvider
```

#### Tenant-First Design
All persistence operations take a `TenantContext` as the first argument, ensuring data isolation. Every storage backend enforces tenant boundaries at the query level.

#### Composite Storage
The `CompositeStorage` pattern combines backends (e.g., SQLite for CRUD + Elasticsearch for search) behind a single interface. Configured via `HFS_STORAGE_BACKEND`.

## Project Skills

Detailed operational guidance lives in project skills under `.claude/skills/`.
Use those skills instead of expanding this always-loaded file:

- `/run-hfs-server` - HFS server runtime, storage backends, multi-tenancy, compression, and API endpoints.
- `/work-with-fhirpath` - FHIRPath CLI, server, expressions, terminology integration, and tests.
- `/work-with-sof` - SQL-on-FHIR, ViewDefinition processing, `sof-cli`, `sof-server`, and parquet output.
- `/work-with-pysof` - Python bindings under `crates/pysof`, maturin setup, API usage, and pysof tests.
- `/test-hfs` - Test strategy, testcontainers, persistence integration tests, and shared test data.
- `/work-with-hts` - Terminology server configuration, APIs, bootstrap sync, and terminology imports.
- `/work-with-auth` - Authentication/authorization, SMART-on-FHIR, JWT/JWKS, scopes, JTI cache, and `HFS_AUTH_*` config.
- `/work-with-audit` - FHIR AuditEvent logging, IHE BALP, audit sinks, and `HFS_AUDIT_*` config.
- `/work-with-subscriptions` - Topic-based Subscriptions engine, channels (rest-hook/websocket/email/messaging), and config.
- `/work-with-cds-hooks` - CDS Hooks protocol types and async service trait for clinical decision support.
- `/bulk-data-export` - FHIR Bulk Data Access `$export` jobs, manifests, output storage, and behavior notes.
- `/bulk-data-submit` - FHIR Bulk Data Submit `$bulk-submit` ingestion, status, OAuth, JWE, and worker settings.
- `/docker-and-release` - Docker image builds and release workflow.

## Environment Setup

### LLD Linker Configuration
Add to `~/.cargo/config.toml`:
```toml
[target.x86_64-unknown-linux-gnu]
linker = "clang"
rustflags = ["-C", "link-arg=-fuse-ld=lld"]
```

### Memory-Constrained Builds
```bash
export CARGO_BUILD_JOBS=4
```

### Debugging Tips
- Use `cargo test -- --nocapture` to see println! output
- Enable trace logging: `RUST_LOG=trace cargo run`
- FHIRPath expressions can be tested independently via CLI
- HFS server: `HFS_LOG_LEVEL=debug cargo run --bin hfs`

## Important Notes

- Default FHIR version is R4 when no features specified
- **FHIR version feature assumption:** Code MAY assume that at least one FHIR version feature is enabled at compile time, and SHOULD assume R4 is enabled when relying on `FhirVersion::default()` (which is gated on `feature = "R4"`). Avoid adding cfg-ladder fallbacks for the "no version enabled" case — that build target is not supported. Single-version minimal builds (e.g. R4B-only) are supported, but functions that need a default value should require R4 explicitly rather than enumerating versions in `#[cfg]` arms.
- The project follows standard Rust conventions
- `pysof` is excluded from default workspace members — `cargo build` from root skips it
- Server returns appropriate HTTP status codes and FHIR OperationOutcomes for errors
- Minimum supported Rust version: 1.90 (edition 2024)
