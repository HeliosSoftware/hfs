# helios-rest

FHIR RESTful API implementation for the Helios FHIR Server.

## Overview

This crate provides a complete implementation of the [FHIR RESTful API](https://hl7.org/fhir/http.html) specification, including:

- **Full CRUD Support**: Create, Read, Update, Delete for all FHIR resource types
- **Versioning**: Version history with vread and history interactions
- **Conditional Operations**: Conditional create, update, delete, and patch
- **Search**: Type-level and system-level search with modifiers
- **Batch/Transaction**: Bundle processing with atomic transaction support
- **Content Negotiation**: JSON format support with proper MIME types
- **Multi-Tenant**: Built-in tenant isolation for multi-tenant deployments
- **Bulk Data Export**: Asynchronous [FHIR Bulk Data Access](https://hl7.org/fhir/uv/bulkdata/export.html) `$export` (system / Patient / Group) with poll, manifest, download, and cancel

## Quick Start

```rust
use helios_rest::{create_app_with_config, ServerConfig};
use helios_persistence::backends::sqlite::SqliteBackend;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Create a storage backend
    let backend = SqliteBackend::new("fhir.db")?;
    backend.init_schema()?;

    // Configure the server
    let config = ServerConfig::default();

    // Create the Axum application
    let app = create_app_with_config(backend, config.clone());

    // Start the server
    let listener = tokio::net::TcpListener::bind(config.socket_addr()).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
```

## Running the Server

```bash
# Run with default settings (SQLite, port 8080)
cargo run --bin rest-server

# Configure via environment variables
HFS_SERVER_PORT=3000 HFS_LOG_LEVEL=debug cargo run --bin rest-server

# Or via command line arguments
cargo run --bin rest-server -- --port 3000 --log-level debug
```

## API Endpoints

| Interaction | Method | URL Pattern |
|------------|--------|-------------|
| read | GET | `/[type]/[id]` |
| vread | GET | `/[type]/[id]/_history/[vid]` |
| update | PUT | `/[type]/[id]` |
| patch | PATCH | `/[type]/[id]` |
| delete | DELETE | `/[type]/[id]` |
| create | POST | `/[type]` |
| search | GET/POST | `/[type]?params` or `/[type]/_search` |
| capabilities | GET | `/metadata` |
| history (instance) | GET | `/[type]/[id]/_history` |
| history (type) | GET | `/[type]/_history` |
| history (system) | GET | `/_history` |
| batch/transaction | POST | `/` |
| bulk export (system) | GET/POST | `/$export` |
| bulk export (patient) | GET/POST | `/Patient/$export` |
| bulk export (group) | GET/POST | `/Group/[id]/$export` |
| export status / manifest | GET | `/export-status/[job_id]` |
| export cancel + delete | DELETE | `/export-status/[job_id]` |
| export file download | GET | `/export-file/[job_id]/[type]-[part]` |

All `$export` kick-offs require `Prefer: respond-async` and return `202 Accepted`
with a `Content-Location` status URL. See [Bulk Data Export](#bulk-data-export)
for configuration; the storage-layer job/output internals are documented in the
[helios-persistence README](../persistence/README.md).

## Configuration

The server is configured via environment variables:

| Variable | Default | Description |
|----------|---------|-------------|
| `HFS_SERVER_PORT` | 8080 | Server port |
| `HFS_SERVER_HOST` | 127.0.0.1 | Host to bind |
| `HFS_LOG_LEVEL` | info | Log level |
| `HFS_MAX_BODY_SIZE` | 10485760 | Max request body (bytes) |
| `HFS_REQUEST_TIMEOUT` | 30 | Request timeout (seconds) |
| `HFS_ENABLE_CORS` | true | Enable CORS |
| `HFS_DEFAULT_TENANT` | default | Default tenant ID |
| `HFS_DATABASE_URL` | - | Database connection string |
| `HFS_TENANT_ROUTING_MODE` | header_only | Tenant routing mode |
| `HFS_TENANT_STRICT_VALIDATION` | false | Error on tenant mismatch |
| `HFS_JWT_TENANT_CLAIM` | tenant_id | JWT claim name (future) |

### Bulk Data Export

The `$export` subsystem is configured via `HFS_BULK_EXPORT_*` environment
variables. Job state reuses the same storage backend that holds FHIR resources
(SQLite or PostgreSQL) — there is no separate job-store connection to configure.
Bulk export is available on the `sqlite`, `postgres`, `sqlite-elasticsearch`, and
`postgres-elasticsearch` backends.

| Variable | Default | Description |
|----------|---------|-------------|
| `HFS_BULK_EXPORT_ENABLED` | `true` | Master switch — when `false`, all `$export` endpoints return `501`. |
| `HFS_BULK_EXPORT_OUTPUT_BACKEND` | `local-fs` | Output store: `local-fs` or `s3`. |
| `HFS_BULK_EXPORT_OUTPUT_DIR` | `${HFS_DATA_DIR}/exports` | Local-FS output root. |
| `HFS_BULK_EXPORT_S3_BUCKET` | *(none)* | S3 bucket — required when `OUTPUT_BACKEND=s3`. |
| `HFS_BULK_EXPORT_S3_ENDPOINT` | *(AWS)* | S3-compatible endpoint URL (e.g. MinIO). |
| `HFS_BULK_EXPORT_S3_FORCE_PATH_STYLE` | `false` | Path-style addressing for S3-compatible providers. |
| `HFS_BULK_EXPORT_S3_REGION` | *(falls back to `HFS_S3_REGION`, then AWS chain)* | AWS region override for export output. |
| `HFS_BULK_EXPORT_REQUIRES_ACCESS_TOKEN` | `auto` | Manifest posture: `auto` / `true` / `false`. **`false` is invalid with `local-fs`** (no pre-signed URLs). |
| `HFS_BULK_EXPORT_FILE_URL_TTL` | `3600` | Pre-signed download-URL lifetime, seconds. |
| `HFS_BULK_EXPORT_OUTPUT_TTL` | `86400` | Output retention after job completion, seconds. |
| `HFS_BULK_EXPORT_WORKER_CONCURRENCY` | `2` | In-process worker pool size. |
| `HFS_BULK_EXPORT_DISABLE_LOCAL_WORKER` | `false` | Disable in-pod workers (use a separate exporter deployment). |
| `HFS_BULK_EXPORT_MAX_CONCURRENT_PER_TENANT` | `4` | Per-tenant active-job cap (kick-off returns `429` if exceeded). |
| `HFS_BULK_EXPORT_BATCH_SIZE` | `1000` | Resources per export batch. |
| `HFS_BULK_EXPORT_LEASE_DURATION` | `60` | Initial lease length, seconds. Must be greater than the heartbeat interval. |
| `HFS_BULK_EXPORT_HEARTBEAT_INTERVAL` | `20` | Worker heartbeat cadence, seconds. |
| `HFS_BULK_EXPORT_CLEANUP_INTERVAL` | `300` | Cleanup-task scan interval, seconds. |
| `HFS_BULK_EXPORT_SINCE_NEWLY_ADDED` | `include` | Group-export `_since` toggle: `include` or `exclude`. |

A runnable multi-instance stack (HFS + PostgreSQL + MinIO + Keycloak) is provided
as a compose example in [`docker/bulk-export/`](../../docker/bulk-export/README.md).

## Multi-Tenancy

The server supports multiple methods for tenant identification, configurable via the `HFS_TENANT_ROUTING_MODE` environment variable.

### Tenant Routing Modes

| Mode | Description |
|------|-------------|
| `header_only` | Tenant from X-Tenant-ID header (default, backward compatible) |
| `url_path` | Tenant from URL path prefix: `/{tenant}/Patient/123` |
| `both` | Both supported; URL takes precedence over header |

### Resolution Priority

When multiple sources provide tenant information, they are resolved in this priority order:

1. **URL path prefix** (highest) - `/{tenant}/...`
2. **X-Tenant-ID header**
3. **JWT token claim** (future)
4. **Default tenant** (lowest) - from configuration

### Strict Validation

When `HFS_TENANT_STRICT_VALIDATION=true`, the server returns an error if the URL path and X-Tenant-ID header specify different tenants. This helps catch configuration issues early.

### Examples

```bash
# Header-based (default mode)
curl -H "X-Tenant-ID: acme" http://localhost:8080/Patient/123

# URL-based (requires HFS_TENANT_ROUTING_MODE=url_path or both)
curl http://localhost:8080/acme/Patient/123

# With URL routing, the CapabilityStatement includes the tenant in the base URL
curl http://localhost:8080/acme/metadata
# Returns implementation.url: "http://localhost:8080/acme"
```

### URL-Based Routing Setup

To enable URL-based tenant routing:

```bash
# URL paths only (header ignored)
HFS_TENANT_ROUTING_MODE=url_path cargo run --bin hfs

# Both URL and header (URL takes precedence)
HFS_TENANT_ROUTING_MODE=both cargo run --bin hfs
```

When using `url_path` or `both` mode, routes are structured as:
- `/{tenant}/Patient/123` - Read patient in tenant
- `/{tenant}/metadata` - Tenant-specific CapabilityStatement
- `/health` - Health check (not tenant-scoped)
- `/_liveness` - Liveness probe (not tenant-scoped)

## Features

Enable different FHIR versions and backends via Cargo features:

```toml
[dependencies]
helios-rest = { version = "0.1", features = ["R4", "sqlite"] }
```

### FHIR Versions
- `R4` (default) - FHIR R4 (4.0.1)
- `R4B` - FHIR R4B (4.3.0)
- `R5` - FHIR R5 (5.0.0)
- `R6` - FHIR R6 (6.0.0-ballot)

### Backends
- `sqlite` (default) - SQLite (great for development)
- `postgres` - PostgreSQL (recommended for production)
- `mongodb` - MongoDB

## Batch and Transaction Bundles

The server supports FHIR [batch](https://hl7.org/fhir/http.html#batch) and [transaction](https://hl7.org/fhir/http.html#transaction) bundles via `POST /`.

> **Backend Support:** Transaction bundles require a backend with ACID transaction support. SQLite fully supports transactions. Some backends (Cassandra, Elasticsearch, S3) only support batch bundles. See the persistence crate documentation for the full capability matrix.

### Transaction (Atomic)

All entries succeed or all fail together. Entries are processed in FHIR-specified order:
1. DELETE operations
2. POST (create) operations
3. PUT/PATCH (update) operations
4. GET (read) operations

**Reference Resolution:** `urn:uuid:` references in resources are automatically resolved to assigned IDs after creates. This allows referencing newly-created resources within the same transaction.

```bash
curl -X POST http://localhost:8080/ \
  -H "Content-Type: application/fhir+json" \
  -d '{
    "resourceType": "Bundle",
    "type": "transaction",
    "entry": [
      {
        "fullUrl": "urn:uuid:new-patient",
        "resource": {"resourceType": "Patient", "name": [{"family": "Smith"}]},
        "request": {"method": "POST", "url": "Patient"}
      },
      {
        "resource": {
          "resourceType": "Observation",
          "subject": {"reference": "urn:uuid:new-patient"}
        },
        "request": {"method": "POST", "url": "Observation"}
      }
    ]
  }'
```

### Batch (Independent)

Each entry is processed independently. Failures in one entry don't affect others.

```bash
curl -X POST http://localhost:8080/ \
  -H "Content-Type: application/fhir+json" \
  -d '{
    "resourceType": "Bundle",
    "type": "batch",
    "entry": [
      {"request": {"method": "GET", "url": "Patient/123"}},
      {"request": {"method": "DELETE", "url": "Patient/456"}}
    ]
  }'
```

### Conditional Operations in Bundles

Bundle entries support conditional headers:
- `ifMatch` - ETag for optimistic locking on updates
- `ifNoneMatch` - Prevent overwrites (`*` for conditional create)
- `ifNoneExist` - Search query for conditional create

### Current Limitations

The following FHIR transaction features are not yet implemented:
- **Conditional reference resolution** - References like `Patient?identifier=12345` are not resolved
- **PATCH method** - PATCH operations in bundles return 501 Not Implemented
- **Prefer header** - `return=minimal` and `return=OperationOutcome` not honored
- **Duplicate detection** - Same resource appearing twice in a transaction is not detected

## HTTP Headers

The server supports standard FHIR HTTP headers:

| Header | Purpose |
|--------|---------|
| `Accept` | Content negotiation |
| `Content-Type` | Request body format |
| `ETag` / `If-Match` | Optimistic locking |
| `If-None-Match` | Conditional read |
| `If-None-Exist` | Conditional create |
| `If-Modified-Since` | Conditional read by date |
| `Prefer` | Response preference |
| `X-Tenant-ID` | Multi-tenant identification |

## Error Handling

All errors are returned as FHIR OperationOutcome resources:

```json
{
  "resourceType": "OperationOutcome",
  "issue": [{
    "severity": "error",
    "code": "not-found",
    "details": {
      "text": "Resource Patient/123 not found"
    }
  }]
}
```

## Testing

Tests use a JSON-driven specification format:

```bash
# Run all tests
cargo test -p helios-rest

# Run with specific backend
cargo test -p helios-rest --features sqlite
```

### Test Specifications

Tests are defined in JSON files under `tests/specs/`:

```json
{
  "name": "Patient Read Tests",
  "tests": [
    {
      "name": "read_existing_patient",
      "request": {
        "method": "GET",
        "path": "/Patient/123"
      },
      "expect": {
        "status": 200,
        "body": {
          "resourceType": "Patient",
          "id": "123"
        }
      }
    }
  ]
}
```

## Architecture

```
src/
├── lib.rs          # Crate entry point
├── config.rs       # Server configuration
├── error.rs        # Error types → OperationOutcome
├── state.rs        # Application state
├── handlers/       # HTTP request handlers
├── middleware/     # Axum middleware
├── extractors/     # Axum extractors
├── responses/      # Response formatting
├── routing/        # Route configuration
└── tenant/         # Multi-source tenant resolution
    ├── mod.rs      # Module exports
    ├── source.rs   # TenantSource enum
    ├── resolver.rs # TenantResolver and extractors
    └── validation.rs # Strict mode validation
```

## License

MIT
