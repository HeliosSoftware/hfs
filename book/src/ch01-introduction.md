# Introduction

## What is HL7 FHIR?

**FHIR** (Fast Healthcare Interoperability Resources) is a standard developed by [HL7 International](https://hl7.org/fhir) for exchanging electronic health records. It defines a collection of **resources** — typed, self-describing JSON (or XML) documents representing clinical and administrative data such as patients, observations, medications, encounters, and more.

Each resource has:
- A `resourceType` field identifying its type (e.g., `"Patient"`, `"Observation"`)
- A stable REST API shape: create, read, update, delete, search, history
- A versioned content model with support for extensions

FHIR is used by EHR vendors, national health networks, payers, and research platforms. It is the foundation for standards such as US Core, Da Vinci, and SMART on FHIR.

---

## What is the Helios FHIR Server?

The **Helios FHIR Server (HFS)** is a production-grade implementation of the HL7® FHIR® standard, written in Rust for high performance and optimized for clinical analytics workloads. It provides modular components that can be run as standalone command-line tools, integrated as microservices, or embedded directly into a data analytics pipeline.

**Why Helios?**

- **Blazing Fast** — Built in Rust for maximum performance and minimal resource usage
- **Analytics-First** — Optimized for clinical data analytics and research workloads
- **Modular Design** — Use only what you need, from FHIRPath expressions to a full FHIR server
- **Multi-Version Support** — Work with R4, R4B, R5, and R6 data in the same application
- **Developer Friendly** — Excellent error messages, comprehensive tooling, and CLI tools

The project ships five standalone tools:

| Component | Binary / Package | Description |
|-----------|-----------------|-------------|
| FHIR REST server | `hfs` | Full FHIR-compliant REST API |
| FHIRPath engine | `fhirpath-cli` / `fhirpath-server` | Expression evaluation, CLI and HTTP |
| SQL-on-FHIR | `sof-cli` / `sof-server` | ViewDefinition-based tabular transforms |
| Python bindings | `pysof` (PyPI) | SQL-on-FHIR from Python |
| CDS Hooks types | `helios-cds-hooks` (library) | Protocol types for clinical decision support |

---

## Who Is This For?

HFS is designed for three primary audiences:

**Clinical researchers and data engineers**
Building ETL pipelines that extract FHIR data from EHR systems, transform it into tabular formats (CSV, Parquet), and load it into analytics platforms or data warehouses.

**Healthcare application developers**
Building FHIR-compliant REST APIs, integrating CDS Hooks services into EHR workflows, or embedding FHIRPath evaluation into validation pipelines.

**Platform and infrastructure engineers**
Deploying FHIR servers with polyglot storage backends (SQLite, PostgreSQL, MongoDB, S3, Elasticsearch) and multi-tenant data isolation.

You do not need to be a Rust programmer to use HFS — most workflows are covered by the CLI tools and the Python `pysof` package. The Rust API is available for teams embedding HFS into larger systems.

---

## What Can You Build With HFS?

**Clinical Research Platforms**
Transform FHIR data exported from EHR systems into research-ready datasets using SQL-on-FHIR ViewDefinitions. Export to CSV, JSON, NDJSON, or Parquet for downstream analysis in Python, R, or SQL.

**Real-Time Analytics Dashboards**
Stream FHIR resources through the `sof-server` HTTP API for operational insights, filtered by patient, time window (`_since`), and resource type.

**Data Quality and Validation Tools**
Write FHIRPath expressions that encode clinical invariants and run them over patient cohorts using `fhirpath-cli` or the HTTP server.

**ETL Pipelines**
Use `pysof` to process large NDJSON exports in parallel with chunked streaming. Write to Parquet files for efficient columnar storage. Automate with standard Python tooling.

**High-Performance FHIR APIs**
Deploy `hfs` backed by PostgreSQL + Elasticsearch for a production FHIR server with full search, history, versioning, and multi-tenancy. Switch storage backends with a single environment variable.

**CDS Hooks Services**
Implement the CDS Hooks standard using `helios-cds-hooks` types and traits, compatible with any Rust async web framework.

---

## FHIR Version Support

| Version | Label | Status in HFS |
|---------|-------|---------------|
| FHIR R4 (4.0.1) | Normative | ✅ Default |
| FHIR R4B (4.3.0) | Normative | ✅ Supported |
| FHIR R5 (5.0.0) | Current standard | ✅ Supported |
| FHIR R6 (6.0.0-ballot2) | Latest ballot | ✅ Supported |

All four versions are available via Cargo feature flags. The default build compiles R4 only; use `--features R4,R4B,R5,R6` to enable all versions.

---

*HL7® and FHIR® are registered trademarks of Health Level Seven International.*
