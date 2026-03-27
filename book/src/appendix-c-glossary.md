# Appendix C — Glossary

---

**Bundle**
A FHIR resource (`resourceType: "Bundle"`) that wraps a collection of other resources. Used for REST API payloads, transactions, search results, and bulk data exports. Bundle types include `collection`, `transaction`, `batch`, and `searchset`.

**CapabilityStatement**
A FHIR resource that describes what a FHIR server can do — which resource types it supports, which interactions (CRUD, search, history) are available, and which search parameters are supported. Returned by `GET /metadata`.

**CDS Hooks**
An HL7 standard for invoking clinical decision support logic at specific points in clinical workflows (e.g., patient selection, order signing). Services expose a discovery endpoint and return **Cards** with actionable recommendations. See `helios-cds-hooks`.

**CompositeStorage**
An HFS pattern that combines two storage backends behind a single `ResourceStorage` interface: one for CRUD operations and one for search queries (e.g., SQLite + Elasticsearch). Configured via `HFS_STORAGE_BACKEND`.

**ETag**
An HTTP header used for optimistic concurrency control. HFS includes an ETag with each resource response. Clients submit the ETag in an `If-Match` header on updates to prevent lost updates.

**FHIR**
Fast Healthcare Interoperability Resources. An HL7 standard for exchanging electronic health records. Defines typed resources (Patient, Observation, etc.) with a REST API and JSON/XML serialization.

**FHIRPath**
A path-based navigation and extraction language for FHIR resources. Used in search parameter definitions, profile constraints, CDS Hooks, and SQL-on-FHIR ViewDefinitions. HFS implements FHIRPath 3.0.0-ballot via `helios-fhirpath`.

**FhirVersion**
A Rust enum (`R4`, `R4B`, `R5`, `R6`) used throughout the HFS workspace to carry FHIR version information at runtime.

**HL7**
Health Level Seven International. The organization that publishes the FHIR standard.

**maturin**
A build tool for creating Python wheels from Rust code via PyO3. Used to build and publish `pysof`.

**NDJSON**
Newline-Delimited JSON. A file format where each line is a complete, self-contained JSON object. Preferred for large FHIR bulk data exports because it can be streamed without loading the entire file into memory.

**OperationOutcome**
A FHIR resource returned by the server when an error occurs. Contains one or more `issue` entries with severity, code, and human-readable diagnostics.

**pysof**
The Python package (`pip install pysof`) providing Python bindings for `helios-sof`. Built with PyO3 and maturin. Supports CSV, JSON, NDJSON, and Parquet output with automatic multi-core parallelism.

**PyO3**
A Rust library for writing Python extension modules. Used by `pysof` to expose the `helios-sof` API to Python.

**Rayon**
A Rust data-parallelism library. Used by `pysof` to automatically parallelize ViewDefinition processing across CPU cores.

**Resource**
The fundamental unit of FHIR data. A self-describing JSON document with a `resourceType` field and version-specific fields. Examples: `Patient`, `Observation`, `MedicationRequest`.

**ResourceStorage**
The base trait in `helios-persistence` that all storage backends must implement. Covers CRUD operations (create, read, update, delete) with `TenantContext` isolation.

**SQL-on-FHIR**
An HL7 specification for flattening FHIR resources into tabular views using declarative **ViewDefinitions**. Implemented by `helios-sof`. See [sql-on-fhir.org](https://sql-on-fhir.org).

**TenantContext**
A struct passed as the first argument to every `helios-persistence` operation. Carries the tenant identifier, ensuring data is always scoped to the correct tenant at the query level.

**testcontainers**
A Rust library that spins up real Docker containers (PostgreSQL, Elasticsearch) for integration tests. Used in `helios-persistence` tests.

**ViewDefinition**
A FHIR resource type defined by the SQL-on-FHIR specification. Describes how to flatten a FHIR resource type into a relational table, specifying column names and FHIRPath expressions for each column.

**vread**
Version-specific read. A FHIR interaction (`GET /[type]/[id]/_history/[vid]`) that retrieves a specific historical version of a resource, identified by its version ID.

**workspace**
A Cargo feature that groups multiple Rust crates into a single project. All HFS crates share a workspace root at the repository root. They share a `Cargo.lock` and can reference each other as dependencies.
