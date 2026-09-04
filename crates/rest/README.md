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
- The same applies after a **server upgrade that adds a parameter to the
  built-in set**, not just after an operator edits one. Resources written
  before the upgrade were extracted under the old definitions and have no index
  rows for the new parameter, so it matches nothing on that older data until a
  `$reindex` runs. Three current cases: `_source` (`meta.source`), which no
  version of the server indexed before [#523]; `_language`
  (`Resource.language`) on R5/R6; and the `ViewDefinition` SearchParameters
  from the SQL-on-FHIR IG (`url`, `name`, `status`, `date`, `context`, …)
  added in [#570] — `ViewDefinition` resources created before that upgrade are
  not searchable by those parameters until `POST /ViewDefinition/$reindex` (or
  a system-wide `$reindex`) runs. None of these return an error on stale
  data — they simply under-match, which is the failure mode a reindex exists
  to clear.

[#523]: https://github.com/HeliosSoftware/hfs/issues/523
[#570]: https://github.com/HeliosSoftware/hfs/issues/570

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
| `HFS_BATCH_MAX_CONCURRENCY` | 16 | Ceiling on concurrent entries within one `batch` Bundle (see below) |
| `HFS_ENABLE_CORS` | true | Enable CORS |
| `HFS_DEFAULT_TENANT` | default | Default tenant ID |
| `HFS_DATABASE_URL` | - | Database connection string |
| `HFS_TENANT_ROUTING_MODE` | header_only | Tenant routing mode |
| `HFS_TENANT_STRICT_VALIDATION` | false | Error on tenant mismatch |
| `HFS_JWT_TENANT_CLAIM` | tenant_id | JWT claim name (future) |

### Batch entry concurrency

Entries of a `batch` Bundle execute concurrently, bounded by whatever the
storage backend declares it tolerates (`ResourceStorage::bulk_write_concurrency`)
capped by `HFS_BATCH_MAX_CONCURRENCY`. The setting is a **ceiling**: it can
lower a backend's declared tolerance but never raise it, because only the
backend knows what its connection pool absorbs. Raise throughput by raising the
backend's own limit — pool size, for instance — not this one.

The bound exists so a large bundle finishes inside `HFS_REQUEST_TIMEOUT`. At a
bound of 1 the wall clock is the *sum* of every entry's storage round trips, so
against a latency-bound backend a few hundred entries will exceed the timeout
and the client receives a 408 with no body — while the entries that already
committed stay committed.

Points worth knowing before raising it:

- **It multiplies per concurrent request.** Twenty simultaneous batches at a
  bound of 16 put 320 storage operations in flight. This is a per-request bound,
  not a server-wide budget.
- **Entries are not ordered relative to one another.** FHIR states that a server
  may process batch entries in any order, and at a bound above 1 it does. Two
  entries writing the same id, or an entry reading what another entry in the
  same bundle writes, resolve by timing. Put ordered work in a `transaction`, or
  in separate requests.
- **Response entries stay positional** regardless: response entry *i* always
  answers request entry *i*.
- **A bundle that writes a `StructureDefinition` drops to sequential**
  automatically, because later entries validate against profiles earlier entries
  register.
- **Auditing amplifies with it.** Per-entry audit events are emitted as the
  entries complete, so a database or S3 audit sink sees the same event count
  compressed into a much shorter window.

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
- **Poll pacing**: an in-progress poll advertises `Retry-After`
  (`HFS_BULK_SUBMIT_RETRY_AFTER`); a client that ignores it and hammers the poll URL
  is throttled with `429` plus a `Retry-After` pointing at the end of the rate window
  (`HFS_BULK_SUBMIT_POLL_RATE_LIMIT` / `_POLL_RATE_WINDOW`). Buckets are per client
  (principal, else peer address) per poll token.
- **Backends**: available on `sqlite`, `postgres`, and their `-elasticsearch`
  composites; other backends return `501`. Job state reuses the FHIR-resource
  backend — no separate job store to configure.
- **Status manifest**: emits `output[]`, `outcome[]`, and `deleted[]` arrays (each
  entry carries `url`, `count`, and `fileSize`), plus `requiresAccessToken`,
  `transactionTime`, and `link[]`.
- **Status pagination**: entries are split across pages of
  `HFS_BULK_SUBMIT_MANIFEST_PAGE_SIZE` (default `1000`); when more remain, `link[]`
  carries one `{"relation": "next", "url": ".../bulk-submit-status/{token}?page=N"}`
  entry and every other manifest field repeats identically on each page. Pages are
  fetched from the same status URL with `?page=N` (1-based); an out-of-range page is
  `404` and a malformed one `400`. Set the page size to `0` to disable pagination.

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
| `HFS_BULK_SUBMIT_MANIFEST_PAGE_SIZE` | `1000` | Max `output` + `outcome` + `deleted` entries per status-manifest page; further pages are chained by `link[]` `next`. `0` disables pagination. |
| `HFS_BULK_SUBMIT_POLL_RATE_LIMIT` | `10` | Status polls allowed per client, per submission, per rate window. `0` disables poll rate limiting. |
| `HFS_BULK_SUBMIT_POLL_RATE_WINDOW` | `60` | Sliding window for the poll rate limit, seconds. |
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
| `HFS_BULK_SUBMIT_DECRYPTION_KEY` | *(none)* | P-256/P-384 private key(s) for `ECDH-ES*` JWE key management — PEM (PKCS#8/SEC1) or a JWK / JWK Set. |

For protected provider files (`requiresAccessToken`), HFS acquires a read-scoped
token via SMART Backend Services (`client_credentials` + `private_key_jwt`) when
`HFS_BULK_SUBMIT_CLIENT_ID` and `HFS_BULK_SUBMIT_PRIVATE_KEY` are set.

#### Encrypted submissions (`fileEncryptionKey`)

JWE decryption is built unconditionally — the old `bulk-submit-jwe` feature is a
deprecated no-op. When a submission carries a `fileEncryptionKey`, both the
manifest and every output/deleted file are decrypted by the
[`jwe`](src/jwe.rs) module (pure RustCrypto, no OpenSSL):

| Layer | Supported |
|-------|-----------|
| `alg` | `dir`, `A128KW`/`A192KW`/`A256KW`, `A128GCMKW`/`A192GCMKW`/`A256GCMKW`, `ECDH-ES`, `ECDH-ES+A128KW`/`+A192KW`/`+A256KW` |
| `enc` | `A128GCM`/`A192GCM`/`A256GCM`, `A128CBC-HS256`/`A192CBC-HS384`/`A256CBC-HS512` |
| Serialization | compact, flattened JSON, general JSON |
| Compression | `zip: "DEF"` |

Deliberately unsupported, each rejected with an error naming the reason:

- **`RSA-OAEP` / `RSA-OAEP-256`.** The only pure-Rust RSA implementation (the
  `rsa` crate) carries [RUSTSEC-2023-0071] — the Marvin Attack, key recovery
  through a decryption timing sidechannel — with no fixed release. Rather than
  take a knowingly vulnerable RSA implementation into a server that handles PHI,
  the RSA arms are refused; use the `ECDH-ES` family to deliver a
  content-encryption key asymmetrically. RSA private keys are likewise rejected
  by `HFS_BULK_SUBMIT_DECRYPTION_KEY` rather than silently ignored.
- **`RSA1_5`.** RFC 8017 §7.2 padding-oracle exposure; deprecated for JOSE by
  RFC 8725.
- **`PBES2-*`.** Password-based — the submit flow has no shared password.

Any other unknown `alg` or `enc` is rejected with the offending value in the
message.

[RUSTSEC-2023-0071]: https://rustsec.org/advisories/RUSTSEC-2023-0071

`fileEncryptionKey.value` is accepted as base64url key material, an `oct` JWK, or
— matching the spec's "JSON Web Encryption structure to deliver a Content
Encryption Key" — a JWE that wraps the CEK. That last form, and any file using
`ECDH-ES*` directly, needs a local P-256/P-384 private key in
`HFS_BULK_SUBMIT_DECRYPTION_KEY`; `dir` and the `A*KW` families work from the
provider-supplied symmetric key alone and need no configuration.

A file that arrives in cleartext despite a `fileEncryptionKey` is rejected. A
cleartext *manifest* is accepted with a warning, since manifests carry URLs
rather than PHI and providers commonly leave them unencrypted.

### SQL-on-FHIR Async Export

Separate from Bulk Data Export, the SQL-on-FHIR `$sql-export` and
`$sql-export` operations run asynchronously and write their tabular output
to a dedicated *export sink*, configured via `HFS_EXPORT_*`. The whole subsystem
is gated by `HFS_SOF_ENABLED` (which also enables `$sql-run`); when
enabled, the storage backend must provide an in-DB SOF runner (`sqlite` or
`postgres`).

| Variable | Default | Description |
|----------|---------|-------------|
| `HFS_SOF_ENABLED` | `true` | Master switch for SQL-on-FHIR operations (`$sql-run`, `$sql-export`). |
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

`$sql-run` and `$sql-export` both accept a repeating `context` parameter
carrying ViewDefinitions or SQLView Libraries inline, matched to the
subject's dependency graph by canonical URL at any depth. Resolution is
**server-first**: for each dependency, storage is checked before `context`,
so `context` only applies to URLs the server cannot resolve on its own — a
`context` entry that duplicates an artifact the server already has is
silently ignored (there is no channel to attach a warning to a streamed
`$sql-run`/`$sql-export` response).

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

All advertised URLs use `HFS_BASE_URL`. Set it to the public origin and any
reverse-proxy prefix, such as `https://fhir.example.com/fhir`. URL-routed
requests append the selected tenant after that prefix. Header-routed requests
keep the configured base unchanged. Forwarding headers never override it.

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

### Entry Methods

`Bundle.entry.request.method` is matched **case-sensitively** in both bundle types.
It is a `code` with a required binding to `http://hl7.org/fhir/ValueSet/http-verb`,
whose concepts are `caseSensitive: true` and uppercase in every supported FHIR
version — so a lowercase `"post"` is invalid instance data and is refused with
`400`, not silently accepted. `GET`, `POST`, `PUT` and `DELETE` dispatch; `PATCH`
and `HEAD` are refused as described under Current Limitations.

### Conditional Operations in Bundles

- `ifMatch` — **supported.** ETag for optimistic locking on `PUT` **and `DELETE`**
  entries, in both `batch` and `transaction` bundles. Parsed as the comma-separated
  list RFC 9110 §13.1.1 defines: satisfied when any supplied entity-tag matches.
- `ifNoneMatch` — **parsed and ignored.** `parse_bundle_entry` populates
  `BundleEntry.if_none_match`; no handler or backend reads it.
- `ifNoneExist` — **supported** on `POST` entries, in both `batch` and
  `transaction` bundles, on every backend that implements `ConditionalStorage`
  (SQLite, PostgreSQL, MongoDB; S3's implementation is a stub and answers `501`
  per entry). The value is passed to storage verbatim, as the `If-None-Exist`
  header is. No match creates (`201`); one match answers `200` with the existing
  resource and its `location`, so a `urn:uuid` reference to that entry resolves to
  the match; several matches answer `412 multiple-matches`. In a transaction the
  criteria are resolved inside the open transaction, so two entries with the same
  criteria in one bundle yield one resource. When search is offloaded to a
  secondary backend (composite SQLite/PostgreSQL + Elasticsearch) the local index
  is empty, so a transaction `ifNoneExist` entry is refused with `501` and the
  bundle fails at that entry rather than creating a duplicate.

Conditional interactions expressed in the entry URL (`PUT [type]?[criteria]`,
`DELETE [type]?[criteria]`):

- In a `batch`, they are **resolved** with the status mapping the resource
  endpoints use. `PUT`: one match updates (`200`, `location` = `[type]/[id]`), no
  match creates (`201`), several matches `412`. `DELETE`: deleted or no match
  `204`, several matches `412` (`/metadata` elects `conditionalDelete: "single"`).
  The criteria are percent-decoded like a request URL's query, with repeated
  parameters kept. A bundle carrying a conditional entry runs its entries
  serially, because the backends resolve criteria as read-then-write rather than
  compare-and-swap. `ifMatch` on a conditional entry is `400`: it names a
  version of an instance the server has yet to resolve. Criteria on a `POST` are
  `400`; a conditional create is expressed through `ifNoneExist`.
- In a `transaction`, they are **resolved inside the open transaction** (#859).
  The criteria are percent-decoded and parsed with the search parser against the
  tenant's search-parameter registry before anything executes, so a modifier the
  parameter's type does not define, an unknown parameter, or a result-shaping `_`
  parameter (`_count`, `_sort`, `_include`, `_has`, …) is a `400` for the whole
  bundle rather than a silent "matches nothing". The typed criteria reach the
  backend on `BundleEntry.criteria` and are resolved through the
  `ConditionalTransaction` trait, the search surface design discussion #28
  proposed. Every conditional entry is resolved against the transaction's
  starting view **before any entry is written**, per the R4 transaction
  processing rules, and the outcome is pinned: `PUT` updates the match (`200`,
  `location` = the updated version) or creates (`201`); `DELETE` deletes the match
  (`204`, `location` = the deleted version, so the AuditEvent and a composite's
  secondaries can name it) or is a no-op `204`. Several matches fail the whole
  bundle with `412 multiple-matches`. Per R4 §3.1.0.11.2, a resolved identity that
  another entry also addresses — an instance-addressed `PUT`/`DELETE`, or another
  conditional entry resolving to the same resource — fails the bundle with `400`
  naming both entries. Because resolution precedes execution, a conditional entry
  does not see a sibling `POST`'s write (unlike `ifNoneExist`, which resolves in
  entry order); a conditional `PUT` that matched can be referenced by `urn:uuid`
  from any entry, whatever its position. `ifMatch` on a conditional entry and
  criteria on a `POST` are `400`, as in a batch. A control parameter on an
  instance URL (`PUT Patient/123?_format=json`) is dropped; the entry addresses
  the instance either way.
- A backend answers `BundleProvider::supports_conditional_in_transaction`. When it
  is `false` — search offloaded to a secondary (composite SQLite/PostgreSQL +
  Elasticsearch), whose local index is empty — a transaction carrying URL criteria
  or `ifNoneExist` is declined intact with `501` before anything executes, rather
  than failing at the entry. MongoDB's session-scoped matcher evaluates criteria in
  application memory and understands plain `name=value` only: a criterion with a
  modifier, chain, comparison prefix, or comma OR-list fails the bundle with the
  same `501` text (#709 owns an index-backed matcher).

Note that `/metadata` advertises `conditionalCreate`, `conditionalUpdate` and
`conditionalDelete` for every resource type regardless of backend; gating it per
backend is #514.

### Current Limitations

The following FHIR transaction features are not yet implemented:
- **Conditional reference resolution** - References like `Patient?identifier=12345` are not resolved
- **PATCH method** - PATCH operations in bundles return 501 Not Implemented, in both `batch` (per entry) and `transaction` (whole bundle). Send the patch to the instance endpoint instead
- **HEAD entries** - refused with 405. `HEAD` is a legal `http-verb` code and is served on the instance-read route, but not inside a Bundle
- **Prefer header** - `return=minimal` and `return=OperationOutcome` not honored
- **Duplicate detection** - Same resource appearing twice in a transaction is not detected

## HTTP Headers

The server supports standard FHIR HTTP headers:

| Header | Purpose |
|--------|---------|
| `Accept` | Content negotiation |
| `Content-Type` | Request body format |
| `ETag` / `If-Match` | Optimistic locking on `PUT`, `PATCH` and `DELETE` |
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
