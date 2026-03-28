[![Helios FHIR Server](https://github.com/HeliosSoftware/hfs/blob/main/github-banner.png)](https://heliossoftware.com)

# Helios FHIR Server

The Helios FHIR Server is an implementation of the [HL7® FHIR®](https://hl7.org/fhir) standard, built in Rust for high performance and optimized for clinical analytics workloads. It provides modular components that can be run as standalone command-line tools, integrated as microservices, or embedded directly into your data analytics pipeline.

## Why Helios FHIR Server?

- **🚀 Blazing Fast**: Built in Rust for maximum performance and minimal resource usage
- **📊 Analytics-First**: Optimized for clinical data analytics and research workloads
- **🔧 Modular Design**: Use only what you need - from FHIRPath expressions to full server capabilities
- **🌐 Multi-Version Support**: Work with R4, R4B, R5, and R6 data in the same application
- **🛠️ Developer Friendly**: Excellent error messages, comprehensive documentation, and CLI tools

## What People Build with the Helios FHIR Server

- **Clinical Research Platforms**: Transform FHIR data into research-ready datasets using SQL-on-FHIR
- **Real-time Analytics Dashboards**: Process streaming FHIR data for operational insights
- **Data Quality Tools**: Validate and profile FHIR data using FHIRPath expressions
- **ETL Pipelines**: Extract and transform FHIR data for data warehouses and lakes
- **Healthcare APIs**: Build high-performance FHIR-compliant REST APIs
- **Healthcare Analytics**: Analyze patient cohorts at scale


# Quick Start

The Helios FHIR Server includes several components:

- **`hfs`** — the main FHIR server
- **`fhirpath-cli`** and **`fhirpath-server`** — FHIRPath evaluation
- **`sof-cli`** and **`sof-server`** — SQL-on-FHIR transformation
- **[`pysof`](https://pypi.org/project/pysof/)** — Python bindings for SQL-on-FHIR

See [Core Components](#core-components) for details on each.

The server supports SQLite, PostgreSQL, Elasticsearch, and S3 in various configurations — see [Storage Backends](#storage-backends) for setup options.

## Using Release Binaries

Pre-built binaries are available on the [GitHub Releases](https://github.com/HeliosSoftware/hfs/releases) page. Download the appropriate archive for your platform and extract it.

> **Windows users:** Add `.exe` to all binary names (e.g., `hfs.exe`, `fhirpath-cli.exe`, `sof-cli.exe`).

The following are independent examples showing how to run each executable — pick whichever ones apply to your use case:

```bash
# FHIR server (access at http://localhost:8080/metadata)
./hfs

# Evaluate a FHIRPath expression
echo '{"resourceType": "Patient", "id": "123"}' | ./fhirpath-cli 'Patient.id'

# Transform FHIR Bundle to CSV using SQL-on-FHIR
./sof-cli --view examples/patient-view.json --bundle examples/patients.json

# Transform NDJSON file to CSV using SQL-on-FHIR
./sof-cli --view examples/patient-view.json --bundle examples/patients.ndjson

# SQL-on-FHIR HTTP server (POST to http://localhost:8080/ViewDefinition/$viewdefinition-run)
./sof-server

# FHIRPath HTTP server (POST expressions to http://localhost:3000/fhirpath)
./fhirpath-server
```

## Using Docker Images

Pre-built multi-arch Docker images (amd64/arm64) are available on GitHub Container Registry.

```bash
# FHIR Server (default: R4, in-memory SQLite, port 8080)
docker run -p 8080:8080 ghcr.io/heliossoftware/hfs:latest

# With persistent SQLite storage
docker run -p 8080:8080 -v hfs-data:/data -e HFS_DATABASE_URL=/data/fhir.db ghcr.io/heliossoftware/hfs:latest

# With PostgreSQL
docker run -p 8080:8080 \
  -e HFS_STORAGE_BACKEND=postgres \
  -e HFS_DATABASE_URL="postgresql://user:pass@host:5432/fhir" \
  ghcr.io/heliossoftware/hfs:latest

# FHIRPath Server (port 3000)
docker run -p 3000:3000 ghcr.io/heliossoftware/fhirpath-server:latest

# SQL-on-FHIR Server (port 8080)
docker run -p 8080:8080 ghcr.io/heliossoftware/sof-server:latest
```

See [Environment Variables](#environment-variables) for all available configuration options.

## Building From Source

### Prerequisites

1. **Install [Rust](https://www.rust-lang.org/tools/install)**
    ```bash
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
    ```

2. **Install [LLD](https://lld.llvm.org/)**

    Linux (Ubuntu/Debian):
    ```bash
    sudo apt install clang lld
    ```

    Windows:

      Download a pre-build binary from [llvm-project's GitHub page](https://github.com/llvm/llvm-project/releases).

    macOS:

      LLD is not required for macOS.


3. **Configure config.toml**

    Create or modify `~/.cargo/config.toml`:
    ```toml
    [target.x86_64-unknown-linux-gnu]
    linker = "clang"
    rustflags = ["-C", "link-arg=-fuse-ld=lld", "-C", "link-arg=-Wl,-zstack-size=8388608"]

    [target.aarch64-apple-darwin]
    linker = "clang"
    rustflags = [
      "-C", "link-arg=-Wl,-dead_strip",
      "-C", "link-arg=-undefined",
      "-C", "link-arg=dynamic_lookup"
    ]

    [target.x86_64-pc-windows-msvc]
    linker = "lld-link.exe"
    rustflags = ["-C", "link-arg=/STACK:8388608"]
    ```

4. **Memory-constrained builds** (optional):

💡 **Tip**: If you run out of memory during compilation on Linux, especially on high CPU core count machines, limit parallel jobs to 4 (or less):
    ```bash
    export CARGO_BUILD_JOBS=4
    ```

### Build and Install

```bash
# Clone the repository
git clone https://github.com/HeliosSoftware/hfs.git
cd hfs

# Build (R4 only by default). Uses workspace default-members and skips the Python bindings crate (pysof).
cargo build --release

# Or build with all FHIR versions
cargo build --release --features R4,R4B,R5,R6

# Build all workspace members (including pysof)
cargo build --workspace --release

```

## Storage Backends

The Helios FHIR Server supports multiple storage backend configurations. Choose a configuration based on your search requirements and deployment scale.

### Available Configurations

| Configuration | Search Capability | Use Case |
|---|---|---|
| **SQLite** (default) | Built-in FTS5 full-text search | Development, testing, small deployments |
| **SQLite + Elasticsearch** | Elasticsearch-powered search with relevance scoring | Production deployments needing robust search |
| **PostgreSQL** | Built-in full-text search (tsvector/tsquery) | Production OLTP deployments |
| **PostgreSQL + Elasticsearch** | Elasticsearch-powered search with PostgreSQL CRUD | Production deployments needing RDBMS + robust search |
| **S3** | Object storage for CRUD, versioning, history, and bulk operations (no search) | Archival, bulk analytics, cost-effective storage |
| **S3 + Elasticsearch** | Elasticsearch-powered search with S3 CRUD | Large-scale storage with full FHIR search |

### Running the Server

```bash
# SQLite (default) — no external dependencies
./hfs

# SQLite + Elasticsearch
HFS_STORAGE_BACKEND=sqlite-elasticsearch \
HFS_ELASTICSEARCH_NODES=http://localhost:9200 \
  ./hfs

# PostgreSQL
HFS_STORAGE_BACKEND=postgres \
HFS_DATABASE_URL="postgresql://user:pass@localhost:5432/fhir" \
  ./hfs

# PostgreSQL + Elasticsearch
HFS_STORAGE_BACKEND=postgres-elasticsearch \
HFS_DATABASE_URL="postgresql://user:pass@localhost:5432/fhir" \
HFS_ELASTICSEARCH_NODES=http://localhost:9200 \
  ./hfs

# S3 (requires AWS credentials via standard provider chain:
# https://docs.aws.amazon.com/sdkref/latest/guide/standardized-credentials.html)
HFS_STORAGE_BACKEND=s3 \
HFS_S3_BUCKET=my-fhir-bucket \
AWS_PROFILE=your-aws-profile \
AWS_REGION=us-east-1 \
  ./hfs

# S3 + Elasticsearch
HFS_STORAGE_BACKEND=s3-elasticsearch \
HFS_S3_BUCKET=my-fhir-bucket \
HFS_ELASTICSEARCH_NODES=http://localhost:9200 \
AWS_PROFILE=your-aws-profile \
AWS_REGION=us-east-1 \
  ./hfs
```

### Environment Variables

| Variable | Default | Description |
|---|---|---|
| `HFS_STORAGE_BACKEND` | `sqlite` | Backend mode: `sqlite`, `sqlite-elasticsearch`, `postgres`, `postgres-elasticsearch`, `s3`, or `s3-elasticsearch` |
| `HFS_SERVER_PORT` | `8080` | Server port |
| `HFS_SERVER_HOST` | `127.0.0.1` | Host to bind |
| `HFS_DATABASE_URL` | `fhir.db` | Database URL (SQLite path or PostgreSQL connection string) |
| `HFS_DEFAULT_FHIR_VERSION` | `R4` | FHIR version (R4, R4B, R5, R6) |
| `HFS_LOG_LEVEL` | `info` | Log level (error, warn, info, debug, trace) |
| `HFS_ELASTICSEARCH_NODES` | `http://localhost:9200` | Comma-separated ES node URLs |
| `HFS_ELASTICSEARCH_INDEX_PREFIX` | `hfs` | ES index name prefix |
| `HFS_ELASTICSEARCH_USERNAME` | *(none)* | ES basic auth username |
| `HFS_ELASTICSEARCH_PASSWORD` | *(none)* | ES basic auth password |
| `HFS_S3_BUCKET` | `hfs` | S3 bucket name (prefix-per-tenant mode) |
| `HFS_S3_REGION` | *(AWS provider chain)* | AWS region override |
| `HFS_S3_PREFIX` | *(none)* | Optional key prefix prepended to all S3 object keys |
| `HFS_S3_VALIDATE_BUCKETS` | `true` | Validate bucket access on startup |

For detailed backend setup instructions (building from source, Docker commands, and search offloading architecture), see the [persistence crate documentation](crates/persistence/README.md#building--running-storage-backends).

# Architecture Overview

The Helios FHIR Server is organized as a Rust workspace with modular components that can be used independently or together. Each component is designed for high performance and can be embedded directly into your data analytics pipeline.

## Core Components

### 1. [`helios-hfs`](crates/hfs) - Main Server Application
- **Executable:**
  - `hfs` - The main Helios FHIR Server application.

### 2. [`helios-fhir`](crates/fhir) - FHIR Data Models
Generated from FHIR StructureDefinitions, type-safe Rust representations of all FHIR resources and data types.
- Supports FHIR R4, R4B, R5, and R6 via feature flags
- JSON serialization/deserialization with full FHIR compliance
- Precision decimal handling for clinical accuracy
- Default: R4 (use `--all-features` for all versions)

### 3. [`helios-fhir-gen`](crates/fhir-gen) - Code Generator
Generates the FHIR data models from official HL7 specifications.
- Transforms FHIR StructureDefinitions into Rust types
- Automatically downloads the latest R6 specs from the HL7 build server
- See [Code Generation](#code-generation) section and [helios-fhir-gen README](crates/fhir-gen/README.md) for usage details

### 4. [`helios-fhirpath`](crates/fhirpath) - FHIRPath Expression Engine
Complete implementation of the [FHIRPath 3.0.0-ballot specification](https://hl7.org/fhirpath/2025Jan/).
- **Executables:**
  - `fhirpath-cli` - Evaluate FHIRPath expressions from the command line
  - `fhirpath-server` - HTTP server for FHIRPath evaluation
- Parser built with chumsky for excellent error messages
- Comprehensive function library with version-aware type checking
- Auto-detects FHIR version from input data

### 5. [`helios-sof`](crates/sof) - SQL-on-FHIR Implementation
Transform FHIR resources into tabular data using [ViewDefinitions](https://sql-on-fhir.org/ig/latest/index.html).
- **Executables:**
  - `sof-cli` - Command-line tool for batch transformations
  - `sof-server` - HTTP server with `ViewDefinition/$viewdefinition-run` operation
- Supports multiple input formats: JSON, NDJSON, and FHIR Bundles from local/cloud storage
- Supports multiple output formats: CSV, JSON, NDJSON, and Parquet

### 6. [`pysof`](crates/pysof) - Python Bindings
Python bindings for SQL-on-FHIR using PyO3, bringing high-performance FHIR data transformation to Python.

**Key Capabilities:**
- **ViewDefinition Processing**: Transform FHIR resources into tabular formats using ViewDefinitions
- **Multiple Output Formats**: Export to CSV, JSON, NDJSON, and Parquet formats
- **Streaming Support**: Efficiently process large FHIR bundles without loading everything into memory
- **Auto Version Detection**: Automatically detects and handles R4, R4B, R5, and R6 FHIR versions
- **Type-Safe Interface**: Leverages Rust's type safety while providing a Pythonic API
- **High Performance**: Native Rust performance with minimal Python overhead

**Python API Example:**
```python
import pysof

# Transform FHIR bundle to CSV using a ViewDefinition
result = pysof.run_view_definition(
    view_definition=view_def_json,
    bundle=fhir_bundle_json,
    format="csv"
)

# Process individual resources
result = pysof.run_view_definition(
    view_definition=view_def_json,
    resources=[patient1, patient2],
    format="parquet"
)
```

**Distribution:**
- Cross-platform wheel distribution for Linux, Windows, and macOS available on [PyPi](https://pypi.org/project/pysof/)

### 7. [`helios-cds-hooks`](crates/cds-hooks) - CDS Hooks Protocol Types
Rust types and traits for building [CDS Hooks](https://cds-hooks.hl7.org/) clinical decision support services.
- Complete protocol types for discovery, requests, responses, cards, suggestions, and feedback
- Strongly-typed context structs for all 10 hooks in the CDS Hooks Library
- Async `CdsHooksService` trait compatible with any Rust web framework

### 8. [`helios-fhir-macro`](crates/fhir-macro) - Procedural Macros
Helper macros for code generation used by other components.

### 9. [`helios-fhirpath-support`](crates/fhirpath-support) - Shared Utilities
Common types and traits for FHIRPath evaluation.

### 10. [`helios-persistence`](crates/persistence) - Polyglot Persistence Layer
Storage backend abstraction supporting multiple database technologies optimized for different FHIR workloads.

## Design Principles

- **Version Agnostic**: All components use enum wrappers to seamlessly handle multiple FHIR versions
- **Modular Architecture**: Each component can be used standalone or integrated
- **Type Safety**: Leverages Rust's type system for compile-time correctness
- **Performance**: Built for high-throughput clinical analytics workloads

# Features

## FHIR Version Support
- ✅ **FHIR R4** (4.0.1) - Default
- ✅ **FHIR R4B** (4.3.0)
- ✅ **FHIR R5** (5.0.0)
- ✅ **FHIR R6** (6.0.0-ballot2)

## FHIRPath Expression Language
- Complete implementation of [FHIRPath 3.0.0-ballot specification](https://hl7.org/fhirpath/2025Jan/)
- 100+ built-in functions across all categories
- HTTP server for integration with FHIRPath Lab
- Command-line tool for testing and development

## SQL-on-FHIR
- ViewDefinition-based transformation to tabular formats
- Multiple input formats: JSON, NDJSON (newline-delimited), and FHIR Bundles
- Multiple output formats: CSV, JSON, NDJSON, Parquet
- Streaming support for large datasets
- HTTP API with `$viewdefinition-run` operation
- Cloud storage support: S3, GCS, Azure Blob Storage

## FHIR REST API
- Full CRUD operations
- Search with chained parameters
- History and versioning
- Batch/transaction support

# Development

## Building from Source
```bash
# Build with default features (R4 only)
# Note: pysof (Python bindings) is excluded by workspace default-members
cargo build

# Build with all FHIR versions
cargo build --all-features

# Build specific component
cargo build -p helios-fhirpath

# Build Python bindings (requires Python 3.11)
# Option A: Rust-only build of the crate
cargo build -p pysof

# Option B (recommended): build via maturin into a virtual env
cd crates/pysof
uv venv --python 3.11
uv sync
uv run maturin develop --release
# Build distributable artifacts for pysof
uv run maturin build --release -o dist   # wheels
uv run maturin sdist -o dist             # source distribution
# Build everything except Python bindings (alternative)
cargo build --workspace --exclude pysof
```

## Running Tests

**Docker Required:** PostgreSQL and Elasticsearch integration tests use [testcontainers](https://testcontainers.com/) to spin up real database instances in Docker. Make sure Docker is installed and running to execute the full test suite. To skip these tests (e.g., if Docker is unavailable), add `-- --skip postgres_integration --skip es_integration` to your test command.

```bash
# Run all tests (R4 only by default)
cargo test

# Run tests for all FHIR versions
cargo test --all-features

# Run tests without Docker (skips PostgreSQL and Elasticsearch integration tests)
cargo test --all-features -- --skip postgres_integration --skip es_integration

# Run specific test
cargo test test_name_pattern

# Run with output
cargo test -- --nocapture
```

## Code Generation
To regenerate FHIR models from HL7 specifications:
```bash
# This will download the latest R6 (build) specifications from https://build.fhir.org/
# Note the lack of use of --all-features and the lack of skip-r6-download here.
cargo build -p helios-fhir-gen --features R4,R4B,R5,R6
# This will generate all FHIR code models (r4.rs, r4b.rs, r5, and r6) 
./target/debug/helios-fhir-gen --all
# Format the generated files accordingly
cargo fmt --all
```

## Code Documentation

Published crate documentation is available on [crates.io](https://crates.io/keywords/helios-fhir-server). To generate and view documentation locally:

```bash
cargo doc --no-deps --open
```

# Roadmap

See our [Roadmap](ROADMAP.md) for current development priorities and planned features.

# Contributing

Please see our [Contributing Guidelines](CONTRIBUTING.md) for details.

# License

The Helios FHIR Server is licensed under the [MIT License](LICENSE).

# Support

- **Issues**: [GitHub Issues](https://github.com/HeliosSoftware/hfs/issues)
- **Website**: [heliossoftware.com](https://heliossoftware.com)

---

HL7® and FHIR® are registered trademarks of Health Level Seven International.

