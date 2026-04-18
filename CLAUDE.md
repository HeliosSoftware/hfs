# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build and Development Commands

**Note:** Build times can exceed 10 minutes, especially for full workspace builds with all features or when building the FHIR generator due to large generated files.

### Building
```bash
# Build default (R4 only)
cargo build

# Build with all FHIR versions
cargo build --features R4,R4B,R5,R6

# Build specific crate
cargo build -p helios-sof
cargo build -p helios-fhirpath
cargo build -p helios-rest
cargo build -p helios-persistence --features postgres,elasticsearch

# Build pysof (excluded from default workspace build, requires Python)
cd crates/pysof && uv run maturin develop --release
```

### Running Binaries
```bash
# HFS FHIR server (default: R4, SQLite, port 8080)
cargo run --bin hfs

# FHIRPath CLI
cargo run --bin fhirpath-cli -- -e "Patient.name.family" -r patient.json

# FHIRPath HTTP server (default port 3000)
cargo run --bin fhirpath-server

# SQL-on-FHIR CLI
cargo run --bin sof-cli -- --view view.json --bundle data.json --format csv

# SQL-on-FHIR HTTP server (default port 8080)
cargo run --bin sof-server

# Persistence config advisor
cargo run --bin config-advisor
```

### Testing
```bash
# Run all tests (default R4)
cargo test

# Test with all FHIR versions
cargo test --features R4,R4B,R5,R6

# Test specific crate
cargo test -p helios-sof
cargo test -p helios-fhirpath
cargo test -p helios-persistence

# Run single test
cargo test test_name_pattern

# Run tests in specific file
cargo test --test test_file_name

# Show test output
cargo test -- --nocapture

# pysof Python tests (from crates/pysof/)
cd crates/pysof && uv run pytest python-tests/ -v
```

### Linting and Formatting
```bash
# Format code
cargo fmt --all

# Lint code (with CI-compatible flags)
# Note: --all-features is safe here — it enables the `skip-r6-download` feature
# in all crates (fhir, fhir-gen, hfs), preventing R6 spec downloads from build.fhir.org.
cargo clippy --all-targets --all-features -- -D warnings \
  -A clippy::items_after_test_module \
  -A clippy::large_enum_variant \
  -A clippy::question_mark \
  -A clippy::collapsible_match \
  -A clippy::collapsible_if \
  -A clippy::field_reassign_with_default \
  -A clippy::doc-overindented-list-items \
  -A clippy::doc-lazy-continuation

# Check types without building
cargo check
```

### Before Completing Code Changes
Before declaring a plan complete after significant code changes, always run:
1. `cargo fmt --all` - Format all code
2. `cargo clippy` with the CI flags shown above - Fix any linting issues
3. `cargo test` for affected crates - Ensure tests pass

### Documentation
```bash
# Generate and view docs
cargo doc --no-deps --open
```

### FHIR Code Generation
```bash
# Generate FHIR models for all versions
cargo build -p helios-fhir-gen --features R6
./target/debug/fhir_gen --all

# Note: R6 specification files are auto-downloaded from HL7 build server
# Note: Building fhir-gen can take 5-10 minutes due to large generated files
```

## Architecture Overview

### Workspace Structure

The project is a Rust workspace with 12 crates (`pysof` excluded from default-members):

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

## HFS Server Configuration

### Running the Server
```bash
# Default (R4, SQLite, port 8080)
cargo run --bin hfs

# With PostgreSQL
HFS_STORAGE_BACKEND=postgres HFS_DATABASE_URL="postgresql://user:pass@localhost/fhir" cargo run --bin hfs

# With SQLite + Elasticsearch
HFS_STORAGE_BACKEND=sqlite-es HFS_ELASTICSEARCH_NODES="http://localhost:9200" cargo run --bin hfs

# With S3 (requires --features s3)
HFS_STORAGE_BACKEND=s3 HFS_S3_BUCKET=my-bucket cargo run --bin hfs --features s3

# With environment overrides
HFS_SERVER_PORT=3000 HFS_LOG_LEVEL=debug cargo run --bin hfs
```

### Environment Variables

#### Server
| Variable | Default | Description |
|----------|---------|-------------|
| `HFS_SERVER_PORT` | 8080 | Server port |
| `HFS_SERVER_HOST` | 127.0.0.1 | Host to bind |
| `HFS_LOG_LEVEL` | info | Log level (error, warn, info, debug, trace) |
| `HFS_BASE_URL` | http://localhost:8080 | Base URL for Location headers and Bundle links |
| `HFS_DATA_DIR` | ./data | Path to FHIR data directory (search parameters) |

#### Limits
| Variable | Default | Description |
|----------|---------|-------------|
| `HFS_MAX_BODY_SIZE` | 10485760 | Max request body size (bytes) |
| `HFS_REQUEST_TIMEOUT` | 30 | Request timeout (seconds) |
| `HFS_DEFAULT_PAGE_SIZE` | 20 | Default search result page size |
| `HFS_MAX_PAGE_SIZE` | 1000 | Maximum search result page size |

#### CORS
| Variable | Default | Description |
|----------|---------|-------------|
| `HFS_ENABLE_CORS` | true | Enable CORS |
| `HFS_CORS_ORIGINS` | * | Allowed origins |
| `HFS_CORS_METHODS` | GET,POST,PUT,PATCH,DELETE,OPTIONS | Allowed methods |
| `HFS_CORS_HEADERS` | Content-Type,Authorization,Accept,... | Allowed headers |

#### Storage
| Variable | Default | Description |
|----------|---------|-------------|
| `HFS_STORAGE_BACKEND` | sqlite | Storage mode (see table below) |
| `HFS_DATABASE_URL` | (none) | Database connection string |
| `HFS_ELASTICSEARCH_NODES` | http://localhost:9200 | Elasticsearch node URLs (comma-separated) |
| `HFS_ELASTICSEARCH_INDEX_PREFIX` | hfs | Elasticsearch index name prefix |
| `HFS_ELASTICSEARCH_USERNAME` | (none) | Elasticsearch basic auth username |
| `HFS_ELASTICSEARCH_PASSWORD` | (none) | Elasticsearch basic auth password |

#### Multi-tenancy
| Variable | Default | Description |
|----------|---------|-------------|
| `HFS_DEFAULT_TENANT` | default | Default tenant ID |
| `HFS_TENANT_ROUTING_MODE` | header_only | Tenant routing: `header_only`, `url_path`, `both` |
| `HFS_TENANT_STRICT_VALIDATION` | false | Error if URL and header tenant disagree |
| `HFS_JWT_TENANT_CLAIM` | tenant_id | JWT claim name for tenant (future use) |

#### Behavior
| Variable | Default | Description |
|----------|---------|-------------|
| `HFS_DEFAULT_FHIR_VERSION` | R4 | Default FHIR version (R4, R4B, R5, R6) |
| `HFS_ENABLE_REQUEST_ID` | true | Enable request ID tracking |
| `HFS_RETURN_GONE` | true | Return 410 Gone for deleted resources (vs 404) |
| `HFS_ENABLE_VERSIONING` | true | Enable ETag versioning |
| `HFS_REQUIRE_IF_MATCH` | false | Require If-Match header for updates |

### Storage Backends

| Mode | Value | Description |
|------|-------|-------------|
| SQLite (default) | `sqlite` | Zero-config, file or in-memory |
| SQLite + Elasticsearch | `sqlite-elasticsearch` or `sqlite-es` | SQLite for CRUD, ES for search |
| PostgreSQL | `postgres` or `pg` or `postgresql` | PostgreSQL only |
| PostgreSQL + Elasticsearch | `postgres-elasticsearch` or `pg-es` | PG for CRUD, ES for search |
| S3 | `s3` | AWS S3 object storage for CRUD, versioning, history, bulk ops (no search) |
| S3 + Elasticsearch | `s3-elasticsearch` or `s3-es` | S3 for CRUD, ES for search |

#### S3 Backend
Requires building with the `s3` feature:
```bash
cargo build -p helios-hfs --features s3
HFS_STORAGE_BACKEND=s3 HFS_S3_BUCKET=my-bucket HFS_S3_REGION=us-east-1 cargo run --bin hfs --features s3
```

| Variable | Default | Description |
|----------|---------|-------------|
| `HFS_S3_BUCKET` | `hfs` | S3 bucket name (prefix-per-tenant mode) |
| `HFS_S3_REGION` | (AWS chain) | AWS region override |
| `HFS_S3_VALIDATE_BUCKETS` | `true` | Validate bucket existence on startup |

Standard AWS credential chain applies (`AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, instance profiles, etc.). For S3-compatible endpoints (e.g., MinIO), configure `S3BackendConfig` directly with `endpoint_url` and `force_path_style`.

### Multi-tenancy
```bash
# Via header (default)
curl -H "X-Tenant-ID: clinic-a" http://localhost:8080/Patient

# Via URL path (requires HFS_TENANT_ROUTING_MODE=url_path or both)
curl http://localhost:8080/clinic-a/Patient
```

### API Endpoints

| Interaction | Method | URL |
|------------|--------|-----|
| capabilities | GET | `/metadata` |
| read | GET | `/[type]/[id]` |
| vread | GET | `/[type]/[id]/_history/[vid]` |
| update | PUT | `/[type]/[id]` |
| patch | PATCH | `/[type]/[id]` |
| delete | DELETE | `/[type]/[id]` |
| create | POST | `/[type]` |
| search | GET/POST | `/[type]?params` or `/[type]/_search` |
| history (instance) | GET | `/[type]/[id]/_history` |
| history (type) | GET | `/[type]/_history` |
| history (system) | GET | `/_history` |
| batch/transaction | POST | `/` |
| health | GET | `/health` |

## FHIRPath CLI and Server

### CLI Usage
```bash
# Basic expression evaluation
fhirpath-cli -e "Patient.name.family" -r patient.json

# With context expression
fhirpath-cli -c "Patient.name" -e "family" -r patient.json

# With variables
fhirpath-cli -e "value > %threshold" -r observation.json --var threshold=5.0

# Parse debug tree (no resource needed)
fhirpath-cli -e "Patient.name.given.first()" --parse-debug-tree

# Read from stdin
cat patient.json | fhirpath-cli -e "Patient.name.family" -r -

# Specify FHIR version
fhirpath-cli --fhir-version R5 -e "Patient.name.family" -r patient.json
```

### FHIRPath Server

The FHIRPath server provides an HTTP API for expression evaluation, compatible with fhirpath-lab.

```bash
# Start with defaults (port 3000)
cargo run --bin fhirpath-server

# Custom configuration
FHIRPATH_SERVER_PORT=8080 FHIRPATH_SERVER_HOST=0.0.0.0 cargo run --bin fhirpath-server
```

#### Endpoints
| Method | URL | Description |
|--------|-----|-------------|
| POST | `/` | Evaluate FHIRPath (auto-detects FHIR version) |
| POST | `/r4`, `/r4b`, `/r5`, `/r6` | Version-specific evaluation |
| GET | `/health` | Health check |

#### Environment Variables
| Variable | Default | Description |
|----------|---------|-------------|
| `FHIRPATH_SERVER_PORT` | 3000 | Server port |
| `FHIRPATH_SERVER_HOST` | 127.0.0.1 | Host to bind |
| `FHIRPATH_LOG_LEVEL` | info | Log level |
| `FHIRPATH_ENABLE_CORS` | true | Enable CORS |
| `FHIRPATH_CORS_ORIGINS` | * | Allowed origins |
| `FHIRPATH_TERMINOLOGY_SERVER` | (none) | Terminology server URL |

## SOF Server Configuration

### Environment Variables
| Variable | Default | Description |
|----------|---------|-------------|
| `SOF_SERVER_PORT` | 8080 | Server port |
| `SOF_SERVER_HOST` | 127.0.0.1 | Host to bind |
| `SOF_LOG_LEVEL` | info | Log level |
| `SOF_MAX_BODY_SIZE` | 10485760 | Max request body size (bytes) |
| `SOF_REQUEST_TIMEOUT` | 30 | Request timeout (seconds) |
| `SOF_ENABLE_CORS` | true | Enable CORS |
| `SOF_CORS_ORIGINS` | * | Allowed origins |
| `SOF_TERMINOLOGY_SERVER` | (none) | Terminology server URL for FHIRPath functions (memberOf, subsumes) |

### API Endpoints
- `GET /metadata` - Returns CapabilityStatement
- `GET /health` - Health check endpoint
- `POST /ViewDefinition/$viewdefinition-run` - Execute ViewDefinition transformation
  - Parameters (in request body or query):
    - `_format` - Output format (csv, ndjson, json, parquet)
    - `header` - CSV header control (true/false)
    - `viewResource` - ViewDefinition resource
    - `resource` - FHIR resources to transform
    - `patient` - Filter by patient reference
    - `_limit` - Limit results (1-10000)
    - `_since` - Filter by modification time
  - Parameter precedence: Request body > Query params > Accept header

### Parquet Export
```bash
# CLI
cargo run --bin sof-cli -- --view view.json --bundle data.json --format parquet

# Server
curl -X POST http://localhost:8080/ViewDefinition/\$viewdefinition-run \
  -H "Content-Type: application/json" \
  -d '{"_format": "parquet", "viewResource": {...}, "resource": [...]}'
```

Parquet type mapping follows Pathling conventions: boolean->BOOLEAN, string/code/uri->UTF8, integer->INT32, decimal->FLOAT64, dateTime/date->UTF8. Arrays map to Arrow List types. All fields are OPTIONAL. Snappy compression by default.

## Python Bindings (pysof)

Python bindings for SQL-on-FHIR via PyO3/maturin. Published to [PyPI](https://pypi.org/project/pysof/).

### Installation
```bash
pip install pysof
```

### Development Setup
```bash
cd crates/pysof
uv venv --python 3.11
uv sync --group dev
uv run maturin develop --release

# Verify
uv run python -c "import pysof; print(pysof.get_version()); print(pysof.get_supported_fhir_versions())"
```

### Testing
```bash
cd crates/pysof
uv run pytest python-tests/ -v            # Python tests (58 tests)
cargo test                                  # Rust tests (17 tests)
```

### Key API
```python
import pysof

# Basic transformation
result = pysof.run_view_definition(view_definition, bundle, "csv")

# With options
result = pysof.run_view_definition_with_options(view, bundle, "json", limit=10, fhir_version="R4")

# Streaming large NDJSON files
for chunk in pysof.ChunkedProcessor(view, "patients.ndjson", chunk_size=500):
    process(chunk["rows"])

# File-to-file (most memory efficient)
stats = pysof.process_ndjson_to_file(view, "input.ndjson", "output.csv", "csv")
```

## FHIR Code Generation
```bash
# Build the generator (requires R6 feature for latest schemas)
cargo build -p helios-fhir-gen --features R6

# Generate all FHIR version models
./target/debug/fhir_gen --all

# Note: R6 specs auto-downloaded from HL7 build server
# Note: Build can take 5-10 minutes due to large generated files
```

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

## Testing Patterns

### FHIRPath Tests
- Test cases in `crates/fhirpath/tests/`
- Official FHIR test cases from `fhir-test-cases` repository

### SQL-on-FHIR Tests
- Unit tests in `src/` files
- Integration tests in `tests/` directory

### Persistence Tests
- Integration tests use **testcontainers** for PostgreSQL and Elasticsearch (Docker required)
- Use `tokio::sync::OnceCell` for shared containers across tests (1 container per test binary)
- Data isolation via unique prefixes/tenant IDs (UUID-based) instead of separate containers
- ES containers: cap JVM heap with `ES_JAVA_OPTS=-Xms256m -Xmx256m`

### pysof Tests
- Python tests: `cd crates/pysof && uv run pytest python-tests/ -v` (58 tests)
- Rust tests: `cd crates/pysof && cargo test` (17 tests)

### Test Data
- FHIR examples in `crates/fhir/tests/data/`
- Search parameter definitions in `data/search-parameters-{r4,r4b,r5,r6}.json`
- ViewDefinition examples in test files

## HTS Server Configuration

### Running HTS
```bash
# Default (SQLite, port 8090)
cargo run --bin hts

# Custom database path and port
HTS_DATABASE_URL=./my-terminology.db HTS_SERVER_PORT=9090 cargo run --bin hts
```

### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `HTS_SERVER_PORT` | 8090 | Server port |
| `HTS_SERVER_HOST` | 127.0.0.1 | Host to bind |
| `HTS_LOG_LEVEL` | info | Log level (error, warn, info, debug, trace) |
| `HTS_DATABASE_URL` | ./data/hts.db | Database location — SQLite file path, or `postgresql://user:pass@host/db` for Postgres |
| `HTS_STORAGE_BACKEND` | sqlite | Storage backend (`sqlite` default; `postgres` when built with `--features postgres`) |
| `HTS_ENABLE_CORS` | true | Enable CORS |
| `HTS_CORS_ORIGINS` | * | Allowed CORS origins |

### API Endpoints

| Operation | Method | URL |
|-----------|--------|-----|
| health | GET | `/health` |
| capabilities | GET | `/metadata` |
| $lookup | POST | `/CodeSystem/$lookup` |
| $validate-code (CS) | POST | `/CodeSystem/$validate-code` |
| $subsumes | POST | `/CodeSystem/$subsumes` |
| $expand | POST | `/ValueSet/$expand` |
| $validate-code (VS) | POST | `/ValueSet/$validate-code` |
| $translate | POST | `/ConceptMap/$translate` |
| $closure | POST | `/ConceptMap/$closure` |
| import bundle | POST | `/import` |
| CRUD | GET/POST/PUT/DELETE | `/CodeSystem/:id`, `/ValueSet/:id`, `/ConceptMap/:id` |

### Quick Examples
```bash
# Import a FHIR Bundle containing CodeSystem/ValueSet/ConceptMap resources
curl -X POST http://localhost:8090/import \
  -H "Content-Type: application/fhir+json" \
  -d @bundle.json

# Lookup a concept
curl -X POST http://localhost:8090/CodeSystem/\$lookup \
  -H "Content-Type: application/fhir+json" \
  -d '{"resourceType":"Parameters","parameter":[{"name":"url","valueUri":"http://example.org/cs"},{"name":"code","valueCode":"ABC"}]}'

# Expand a value set
curl -X POST http://localhost:8090/ValueSet/\$expand \
  -H "Content-Type: application/fhir+json" \
  -d '{"resourceType":"Parameters","parameter":[{"name":"url","valueUri":"http://example.org/vs"}]}'
```

### HFS Integration
Set `HFS_TERMINOLOGY_SERVER=http://localhost:8090` on the HFS process to enable:
- FHIR search `:in` modifier (expands ValueSet, filters by code)
- FHIRPath `memberOf()` / `subsumes()` delegation via `FHIRPATH_TERMINOLOGY_SERVER`

### Bulk Import CLI (`hts import`)

Load terminology packages directly from the filesystem without going through the HTTP API.

```bash
# HL7 FHIR NPM package (.tgz from https://terminology.hl7.org/en/downloads.html)
cargo run --bin hts -- import ./hl7.terminology.r4-6.0.0.tgz

# SNOMED CT RF2 ZIP  ⚠️  requires NRC license
cargo run --bin hts -- import ./SnomedCT_InternationalRF2_*.zip --format snomed-rf2

# LOINC CSV ZIP  ⚠️  requires free Regenstrief registration at loinc.org
cargo run --bin hts -- import ./Loinc_*.zip --format loinc

# ICD-10-CM tabular XML (free, no license needed — from cms.gov)
cargo run --bin hts -- import ./icd10cm_tabular_2025.xml

# RxNorm RRF folder  ⚠️  requires free NLM terms-of-service at nlm.nih.gov
cargo run --bin hts -- import ./RxNorm_full_current/rrf/

# Common flags
cargo run --bin hts -- import ./package.tgz \
  --database-url ./data/hts.db \   # SQLite file (default: ./data/hts.db)
  --batch-size 1000 \              # resources per batch (default: 500)
  --dry-run \                      # parse only — no DB writes
  --verbose                        # debug output to stderr
```

#### Format auto-detection

| Extension / pattern | Detected format |
|---------------------|-----------------|
| `.tgz` / `.tar.gz` | `hl7-npm` |
| `*_tabular*.xml` | `icd10-cm` |
| `.rrf` or directory | `rxnorm` |
| `.zip` containing RF2 files | `snomed-rf2` |
| `.zip` containing `LoincTable.csv` | `loinc` |
| `.zip` containing `RXNCONSO.RRF` | `rxnorm` |

`.zip` files that cannot be auto-detected require `--format`.

#### Exit codes

| Code | Meaning |
|------|---------|
| `0` | Success — all resources imported |
| `1` | Fatal error — import aborted |
| `2` | Success with non-fatal errors — some records skipped |

---

## Docker

Generic Dockerfile supporting all server binaries via `BINARY_NAME` build arg:

```bash
# Build HFS server image
docker build --build-arg BINARY_NAME=hfs -t hfs .

# Build SOF server image
docker build --build-arg BINARY_NAME=sof-server -t sof-server .

# Build FHIRPath server image
docker build --build-arg BINARY_NAME=fhirpath-server -t fhirpath-server .
```

Base image: `debian:bookworm-slim`. Runs as non-root user `hfs`. Default exposed port: 8080. Server host vars (`HFS_SERVER_HOST`, `SOF_SERVER_HOST`, `FHIRPATH_SERVER_HOST`) are set to `0.0.0.0` inside the container.

**Note:** The Dockerfile expects the binary and data files to be pre-staged in the build context (CI builds the binary separately and copies it in).

## Release Process

Uses `cargo-release` for workspace-wide version bumps. All crates share the same version.

```bash
# Dry run
cargo release patch --dry-run

# Execute (bumps versions, commits, tags, publishes to crates.io, pushes)
cargo release patch --execute
```

After the tag is pushed, GitHub Actions automatically:
- Builds release artifacts
- Creates a GitHub Release
- Builds pysof wheels for Linux, Windows, macOS
- Publishes pysof to PyPI

See `RELEASING.md` for full details.

## Common Development Tasks

### Adding a New FHIRPath Function
1. Add function implementation in appropriate module under `crates/fhirpath/src/`
2. Update parser if needed in `parser.rs`
3. Add test cases covering the function
4. Update feature matrix in README.md

### Working with ViewDefinitions
1. ViewDefinition JSON goes through version-specific parsing
2. Wrapped in `SofViewDefinition` enum for version-agnostic processing
3. Use `run_view_definition()` for transformation

### Adding a New REST Endpoint
1. Add handler in `crates/rest/src/handlers/`
2. Register route in `crates/rest/src/routes.rs`
3. Add tests covering the endpoint

### Implementing a New Storage Backend
1. Implement `ResourceStorage` trait (and optionally `VersionedStorage`, `SearchProvider`, `TransactionProvider`)
2. All operations must take `TenantContext` for tenant isolation
3. Register in composite storage if combining with other backends
4. Use `CapabilityProvider` to advertise supported interactions

### Debugging Tips
- Use `cargo test -- --nocapture` to see println! output
- Enable trace logging: `RUST_LOG=trace cargo run`
- FHIRPath expressions can be tested independently via CLI
- HFS server: `HFS_LOG_LEVEL=debug cargo run --bin hfs`

## Important Notes

- Default FHIR version is R4 when no features specified
- The project follows standard Rust conventions
- `pysof` is excluded from default workspace members — `cargo build` from root skips it
- Server returns appropriate HTTP status codes and FHIR OperationOutcomes for errors
- Minimum supported Rust version: 1.90 (edition 2024)
- When committing changes that only touch documentation (README.md, CLAUDE.md, etc.) or other non-compiled files, include `[skip ci]` in the commit message to avoid unnecessary CI builds
