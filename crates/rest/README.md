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
- **Bulk Data Submit**: Asynchronous [FHIR Bulk Data Access **Submit**](https://build.fhir.org/ig/HL7/bulk-data/en/submit.html) `$bulk-submit` — HFS as Data Consumer fetches and ingests a provider's manifest/NDJSON, with poll, status manifest, and cancel

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
| bulk submit kick-off | POST | `/$bulk-submit` |
| bulk submit status kick-off | POST | `/$bulk-submit-status` |
| bulk submit poll / manifest | GET | `/bulk-submit-status/[poll_token]` |
| bulk submit cancel | DELETE | `/bulk-submit-status/[poll_token]` |
| bulk submit file download | GET | `/bulk-submit-file/[poll_token]/[part]` |
| purge (instance) | DELETE | `/[type]/[id]/$purge` |
| purge (type) | POST | `/[type]/$purge` |
| reindex (system) | POST | `/$reindex` |
| reindex (type) | POST | `/[type]/$reindex` |
| reindex status | GET | `/$reindex-status/[job_id]` |
| reindex cancel | DELETE | `/$reindex-status/[job_id]` |

All `$export` kick-offs require `Prefer: respond-async` and return `202 Accepted`
with a `Content-Location` status URL. See [Bulk Data Export](#bulk-data-export)
for configuration; the storage-layer job/output internals are documented in the
[helios-persistence README](../persistence/README.md).

### Administrative Operations

`$purge` and `$reindex` are **not** part of the FHIR specification. Both are
gated on their own operation scope — `system/purge` and `system/reindex` — and
**not** on ordinary resource scopes, following the `system/bulk-submit`
precedent. A token that may soft-delete a Patient must not thereby be able to
destroy it irrecoverably, and a token that may write a resource must not be able
to rebuild the whole tenant's search index. As with `system/bulk-submit`, a
`system/*.<perm>` wildcard also grants them.

**`$purge`** permanently deletes a resource and all of its versions. This is not
the FHIR `delete` interaction: an ordinary `DELETE` is a *soft* delete that
writes a tombstone version, keeps the resource in `_history`, and answers reads
with `410 Gone`. `$purge` removes the bytes, which is what erasure requirements
(GDPR Article 17 and similar) actually need. It is irreversible.

- `AuditEvent` can **never** be purged, whatever scope the caller holds. A caller
  who can erase the audit trail can erase the evidence of their own actions.
- On a composite deployment the purge targets the *composite*, so it reaches the
  Elasticsearch secondary too — a resource purged only from the primary would
  remain searchable, with its full content, in the index. Secondaries are purged
  first; if any of them fails the primary is left untouched and the operation
  reports `500` with a failure `AuditEvent` (outcome `8`) naming the backend, so
  the purge stays retryable rather than destroying the system of record while
  leaving a searchable copy behind.

**`$reindex`** rebuilds the search index from the stored resources, which is
needed after a `SearchParameter` is added or changed — existing resources were
indexed under the old definition and will not match the new one until they are
re-extracted. Kick-off returns `202 Accepted` with a job id; the rebuild runs in
the background and is polled via `/$reindex-status/[job_id]`.

- It writes to **every** search index in the deployment, including the
  Elasticsearch secondary. Rebuilding only the primary on a deployment where
  Elasticsearch serves search would rebuild an index nothing queries.
- Job state is held **in memory on the node that accepted the kick-off**, so
  `/$reindex-status/[job_id]` returns `404` from any other node. In a multi-node
  deployment, poll the node you kicked off against.
- The `s3` backend standalone has no search index of any kind, so `$reindex`
  there returns `501`. Every other backend and composite supports it.

Both operations emit BALP `AuditEvent`s — purge on completion or failure,
reindex at start and at its terminal state (complete / cancel / fail, outcome
`0` / `4` / `8`) — each attributed to the requesting principal.

## Configuration

The server is configured via environment variables:

| Variable | Default | Description |
|----------|---------|-------------|
| `HFS_SERVER_PORT` | 8080 | Server port |
| `HFS_SERVER_HOST` | 127.0.0.1 | Host to bind |
| `HFS_LOG_LEVEL` | info | Log level |
| `HFS_MAX_BODY_SIZE` | 10485760 | Max request body (bytes; applies to the decompressed body for compressed requests) |
| `HFS_REQUEST_TIMEOUT` | 30 | Request timeout (seconds) |
| `HFS_ENABLE_CORS` | true | Enable CORS |
| `HFS_DEFAULT_TENANT` | default | Default tenant ID |
| `HFS_DATABASE_URL` | - | Database connection string |
| `HFS_TENANT_ROUTING_MODE` | header_only | Tenant routing mode |
| `HFS_TENANT_STRICT_VALIDATION` | false | Error on tenant mismatch |
| `HFS_JWT_TENANT_CLAIM` | tenant_id | JWT claim name (future) |

### HTTP Compression

Request bodies sent with `Content-Encoding: gzip` (also `deflate`, `br`,
`zstd`) are decompressed transparently before parsing; unsupported encodings
are rejected with `415 Unsupported Media Type`. Responses are compressed when
the client advertises support via `Accept-Encoding`, with `Content-Encoding`
and `Vary: Accept-Encoding` set accordingly.

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

### Bulk Data Submit

HFS implements the HL7 FHIR Bulk Data Access **Submit** operation
([`submit.html`](https://build.fhir.org/ig/HL7/bulk-data/en/submit.html)) in the
**Data Consumer** role: a Data Provider `POST`s `$bulk-submit` referencing a Bulk
Export Manifest, HFS asynchronously fetches the manifest and NDJSON files, ingests
them, and exposes results through a status manifest.

- **Endpoints**: `POST /$bulk-submit` (kick-off, `200`), `POST /$bulk-submit-status`
  (`202` + `Content-Location`), `GET`/`DELETE /bulk-submit-status/{poll_token}`
  (poll / cancel), and `GET /bulk-submit-file/{poll_token}/{part}` (HFS-hosted
  status artifacts). See the [API Endpoints](#api-endpoints) table.
- **Scope**: every surface requires the `system/bulk-submit` SMART scope when auth is
  enabled; status, cancel, and file surfaces also enforce submission ownership.
- **Backends**: available on `sqlite`, `postgres`, and their `-elasticsearch`
  composites; other backends return `501`. Job state reuses the FHIR-resource
  backend — no separate job store to configure.
- **Status manifest**: emits `output[]`, `outcome[]`, and `deleted[]` arrays (each
  entry carries `url`, `count`, and `fileSize`), plus `requiresAccessToken`,
  `transactionTime`, and an always-present `link[]` (single, non-paginated).

Configured via `HFS_BULK_SUBMIT_*` environment variables:

| Variable | Default | Description |
|----------|---------|-------------|
| `HFS_BULK_SUBMIT_ENABLED` | `true` | Master switch — when `false`, all `$bulk-submit` endpoints return `501`. |
| `HFS_BULK_SUBMIT_OUTPUT_BACKEND` | `local-fs` | Status-artifact store: `local-fs` or `s3`. |
| `HFS_BULK_SUBMIT_OUTPUT_DIR` | `${HFS_DATA_DIR}/submit` | Local-FS artifact root. |
| `HFS_BULK_SUBMIT_S3_BUCKET` | *(none)* | S3 bucket — required when `OUTPUT_BACKEND=s3`. |
| `HFS_BULK_SUBMIT_REQUIRES_ACCESS_TOKEN` | `auto` | Manifest posture: `auto` / `true` / `false`. **`false` is invalid with `local-fs`.** |
| `HFS_BULK_SUBMIT_FILE_URL_TTL` | `3600` | Pre-signed artifact-URL lifetime, seconds. |
| `HFS_BULK_SUBMIT_OUTPUT_TTL` | `86400` | Artifact retention after completion, seconds. |
| `HFS_BULK_SUBMIT_RETRY_AFTER` | `120` | `Retry-After` (seconds) advertised on an in-progress status poll. |
| `HFS_BULK_SUBMIT_WORKER_CONCURRENCY` | `2` | In-process submit-worker pool size. |
| `HFS_BULK_SUBMIT_DISABLE_LOCAL_WORKER` | `false` | Disable in-pod workers. |
| `HFS_BULK_SUBMIT_MAX_CONCURRENT_PER_TENANT` | `4` | Per-tenant active-submission cap (kick-off returns `429` if exceeded). |
| `HFS_BULK_SUBMIT_BATCH_SIZE` | `1000` | Resources per ingestion batch. |
| `HFS_BULK_SUBMIT_LEASE_DURATION` | `60` | Initial manifest lease length, seconds. Must exceed the heartbeat interval. |
| `HFS_BULK_SUBMIT_HEARTBEAT_INTERVAL` | `20` | Worker heartbeat cadence, seconds. |
| `HFS_BULK_SUBMIT_CLEANUP_INTERVAL` | `300` | Cleanup-task scan interval, seconds. |
| `HFS_BULK_SUBMIT_CLIENT_ID` | *(none)* | OAuth `client_id` for fetching protected provider files. |
| `HFS_BULK_SUBMIT_PRIVATE_KEY` | *(none)* | PEM key for the `private_key_jwt` client assertion. |
| `HFS_BULK_SUBMIT_SIGNING_ALG` | `ES384` | Client-assertion signing algorithm: `ES384` or `RS384`. |
| `HFS_BULK_SUBMIT_OUTBOUND_SCOPE` | `system/*.rs` | Read scope requested for file-retrieval tokens (never `system/bulk-submit`). |

For protected provider files (`requiresAccessToken`), HFS acquires a read-scoped
token via SMART Backend Services (`client_credentials` + `private_key_jwt`) when
`HFS_BULK_SUBMIT_CLIENT_ID` and `HFS_BULK_SUBMIT_PRIVATE_KEY` are set. JWE-encrypted
files (`fileEncryptionKey`) are supported for `dir` + `A128GCM`/`A256GCM` when built
with the `bulk-submit-jwe` feature.

### SQL-on-FHIR Async Export

Separate from Bulk Data Export, the SQL-on-FHIR `$viewdefinition-export` and
`$sqlquery-export` operations run asynchronously and write their tabular output
to a dedicated *export sink*, configured via `HFS_EXPORT_*`. The whole subsystem
is gated by `HFS_SOF_ENABLED` (which also enables `$viewdefinition-run`); when
enabled, the storage backend must provide an in-DB SOF runner (`sqlite` or
`postgres`).

| Variable | Default | Description |
|----------|---------|-------------|
| `HFS_SOF_ENABLED` | `true` | Master switch for SQL-on-FHIR operations (`$viewdefinition-run`/`-export`, `$sqlquery-*`). |
| `HFS_EXPORT_SINK` | `fs` | Output sink for finished shards: `fs` (local filesystem) or `s3`. |
| `HFS_EXPORT_DIR` | `./exports` | Root directory for the `fs` sink. |
| `HFS_EXPORT_S3_BUCKET` | *(none)* | S3 bucket — required when `HFS_EXPORT_SINK=s3`. |
| `HFS_EXPORT_S3_REGION` | *(AWS chain)* | AWS region override for the `s3` sink. |
| `HFS_EXPORT_PRESIGN_TTL_SECS` | `86400` | Pre-signed download-URL lifetime for the `s3` sink, seconds (spec requires ≥ 24h). |
| `HFS_EXPORT_MAX_CONCURRENCY` | `4` | Maximum concurrent export jobs. |
| `HFS_EXPORT_SHARD_ROWS` | `500000` | Target rows per output shard; larger result sets are split across files. |
| `HFS_EXPORT_CONTROLLER` | `memory` | Job-controller backend (`memory`, in-process; `kafka`/`sqs` reserved for future use). |
| `HFS_EXPORT_OUTPUT_TTL` | `86400` | Retention for a finished job's output and bookkeeping, seconds. After this the cleanup reaper deletes the shards and drops the job, so later polls/downloads return `404`. Aligns with the manifest's advertised 24h `Expires`. |
| `HFS_EXPORT_CLEANUP_INTERVAL` | `300` | How often the cleanup reaper scans for expired jobs, seconds (clamped to ≥ 1). |

Cancelling a job (`DELETE` on the status URL) or a mid-run failure deletes that
job's already-written partial shards immediately; the reaper above reclaims
*completed* jobs once they age past `HFS_EXPORT_OUTPUT_TTL`.

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
