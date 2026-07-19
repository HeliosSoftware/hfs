---
name: run-hfs-server
description: Run, configure, or debug the main HFS FHIR server. Use for hfs binary startup, HFS environment variables, storage backend selection, multi-tenancy routing, HFS REST endpoints, request/response compression, S3 storage, and server behavior flags.
---

# HFS Server

Use this when working with the `helios-hfs` binary or HFS runtime configuration.

## Running

```bash
# Default: R4, SQLite, port 8080
cargo run --bin hfs

# PostgreSQL
HFS_STORAGE_BACKEND=postgres HFS_DATABASE_URL="postgresql://user:pass@localhost/fhir" cargo run --bin hfs

# SQLite plus Elasticsearch
HFS_STORAGE_BACKEND=sqlite-es HFS_ELASTICSEARCH_NODES="http://localhost:9200" cargo run --bin hfs

# S3, requires the s3 feature
HFS_STORAGE_BACKEND=s3 HFS_S3_BUCKET=my-bucket cargo run --bin hfs --features s3

# Environment overrides
HFS_SERVER_PORT=3000 HFS_LOG_LEVEL=debug cargo run --bin hfs
```

## Server Environment

| Variable | Default | Description |
|---|---|---|
| `HFS_SERVER_PORT` | `8080` | Server port |
| `HFS_SERVER_HOST` | `127.0.0.1` | Host to bind |
| `HFS_LOG_LEVEL` | `info` | Log level: error, warn, info, debug, trace |
| `HFS_BASE_URL` | `http://localhost:8080` | Base URL for Location headers and Bundle links |
| `HFS_DATA_DIR` | `./data` | FHIR data directory, including search parameters |
| `HFS_SEARCH_PARAM_CACHE_TTL` | `3600` | Seconds between refreshes of the in-memory SearchParameter registry from storage; a param POSTed to one cluster node becomes visible to others within this interval. `0` disables the refresh. |

## Limits

| Variable | Default | Description |
|---|---|---|
| `HFS_MAX_BODY_SIZE` | `10485760` | Max request body size in bytes, applied after decompression |
| `HFS_REQUEST_TIMEOUT` | `30` | Request timeout in seconds |
| `HFS_DEFAULT_PAGE_SIZE` | `20` | Default search result page size |
| `HFS_MAX_PAGE_SIZE` | `1000` | Maximum search result page size |

## Compression

`hfs`, `sof-server`, and `hts` accept gzip, deflate, br, and zstd request bodies via `Content-Encoding`. Unsupported encodings return `415`. Responses are compressed when the client sends `Accept-Encoding`; `Content-Encoding` and `Vary: Accept-Encoding` are set. Body-size limits are enforced on decompressed bodies. SOF does not recompress `application/parquet` or `application/zip`.

## CORS

| Variable | Default | Description |
|---|---|---|
| `HFS_ENABLE_CORS` | `true` | Enable CORS |
| `HFS_CORS_ORIGINS` | `*` | Allowed origins |
| `HFS_CORS_METHODS` | `GET,POST,PUT,PATCH,DELETE,OPTIONS` | Allowed methods |
| `HFS_CORS_HEADERS` | `Content-Type,Authorization,Accept,...` | Allowed headers |

## Storage

| Variable | Default | Description |
|---|---|---|
| `HFS_STORAGE_BACKEND` | `sqlite` | Storage mode |
| `HFS_DATABASE_URL` | none | Database connection string |
| `HFS_ELASTICSEARCH_NODES` | `http://localhost:9200` | Comma-separated Elasticsearch node URLs |
| `HFS_ELASTICSEARCH_INDEX_PREFIX` | `hfs` | Elasticsearch index prefix |
| `HFS_ELASTICSEARCH_USERNAME` | none | Elasticsearch basic auth username |
| `HFS_ELASTICSEARCH_PASSWORD` | none | Elasticsearch basic auth password |
| `HFS_COMPOSITE_SYNC_MODE` | `asynchronous` | ES-backed composite write sync mode: asynchronous, synchronous, or hybrid |

Use `HFS_COMPOSITE_SYNC_MODE=synchronous` when callers need read-your-write search semantics, such as integration tests or bulk loads that immediately search.

## Storage Backends

| Mode | Value |
|---|---|
| SQLite | `sqlite` |
| SQLite plus Elasticsearch | `sqlite-elasticsearch` or `sqlite-es` |
| PostgreSQL | `postgres`, `pg`, or `postgresql` |
| PostgreSQL plus Elasticsearch | `postgres-elasticsearch` or `pg-es` |
| S3 | `s3` |
| S3 plus Elasticsearch | `s3-elasticsearch` or `s3-es` |

S3 requires `--features s3`:

```bash
cargo build -p helios-hfs --features s3
HFS_STORAGE_BACKEND=s3 HFS_S3_BUCKET=my-bucket HFS_S3_REGION=us-east-1 cargo run --bin hfs --features s3
```

| Variable | Default | Description |
|---|---|---|
| `HFS_S3_BUCKET` | `hfs` | S3 bucket name in prefix-per-tenant mode |
| `HFS_S3_REGION` | AWS chain | AWS region override |
| `HFS_S3_ENDPOINT` | AWS | S3-compatible endpoint URL, such as MinIO |
| `HFS_S3_FORCE_PATH_STYLE` | `false` | Required by MinIO and most S3-compatible providers |
| `HFS_S3_ALLOW_HTTP` | `true` | Allow insecure HTTP endpoint URLs when `HFS_S3_ENDPOINT` is set |
| `HFS_S3_VALIDATE_BUCKETS` | `true` | Validate bucket existence on startup |

The standard AWS credential chain applies. For S3-compatible endpoints, set `HFS_S3_ENDPOINT` and `HFS_S3_FORCE_PATH_STYLE=true`. One HFS process shares a single AWS credential chain, so a MinIO primary store and real-AWS bulk-export output store cannot be combined in the same process.

### S3 requires conditional writes

The S3 backend depends on **conditional `PutObject`** (`If-Match` and
`If-None-Match: *`) for correctness — not only for `/_user/settings`, but for
FHIR resource create, update, and delete, which use it for optimistic locking.

AWS S3, MinIO, Cloudflare R2, and recent Ceph RGW support it. **Google Cloud
Storage's S3-interop API and Backblaze B2 do not**, and are therefore unsupported
as an HFS primary store. A store that *rejects* the precondition headers fails
loudly; one that silently *ignores* them would turn every concurrent write into a
lost update with no error anywhere, so verify support before pointing HFS at an
unfamiliar S3-compatible provider.

### S3 key prefixes (IAM)

The backend writes under these prefixes inside `HFS_S3_BUCKET` (each below the
optional `HFS_S3_PREFIX`):

| Prefix | Contents |
|---|---|
| `{tenant}/resources/`, `{tenant}/history/` | FHIR resources and history |
| `{tenant}/bulk/submit/` | Bulk-submit staging |
| `tenants/` | Tenant registry |
| `_system.user-settings/` | Per-user UI settings (`/_user/settings`) |

The last two are **cross-tenant** and sit outside any tenant prefix. A
least-privilege bucket policy scoped only to the FHIR prefixes will pass startup
validation (which only issues `HeadBucket`) and then return `AccessDenied` — a
500 on every affected request. Grant the policy these prefixes too.

Note also that a **bucket-wide** lifecycle rule (expiration or Glacier
transition) will apply to `_system.user-settings/` as well: expiry silently resets
users' preferences, and a Glacier transition makes them unreadable. Scope lifecycle
rules to the FHIR prefixes.

## Per-user UI settings

`GET`/`PUT`/`PATCH /_user/settings` stores an opaque, per-user JSON document (UI
theme, default tenant, recent queries). It is keyed by user only, never by tenant.
Responses carry a weak `ETag` (`W/"{version}"`); clients may send `If-Match` on a
write for optimistic concurrency.

Supported on the **SQLite, PostgreSQL, MongoDB, and S3** backends. Elasticsearch is
search-only and never a standalone primary, so an Elasticsearch-only deployment
gets an explained `501 Not Implemented`. On S3 the store is also unavailable (and
reports the same `501`) in bucket-per-tenant mode with no default system bucket,
since there is nowhere tenant-independent to keep a user-global document.

When authentication is disabled, every caller resolves to the same fallback user
key (`local|default`) and therefore **shares one settings document**.

## Multi-tenancy

| Variable | Default | Description |
|---|---|---|
| `HFS_DEFAULT_TENANT` | `default` | Default tenant ID |
| `HFS_TENANT_ROUTING_MODE` | `header_only` | `header_only`, `url_path`, or `both` |
| `HFS_TENANT_STRICT_VALIDATION` | `false` | Error if URL and header tenant disagree |
| `HFS_JWT_TENANT_CLAIM` | `tenant_id` | JWT claim name for tenant, future use |

```bash
# Via header, default
curl -H "X-Tenant-ID: clinic-a" http://localhost:8080/Patient

# Via URL path, requires HFS_TENANT_ROUTING_MODE=url_path or both
curl http://localhost:8080/clinic-a/Patient
```

## Clustering

| Variable | Default | Description |
|---|---|---|
| `HFS_CLUSTER` | `false` | Declares this process one of N instances: flips cluster-safe defaults and fails fast on unsafe combinations |
| `HFS_JOB_STORE_BACKEND` | `memory` (`database` under `HFS_CLUSTER`) | Unified async-job store selector (SoF export, reindex); parsed today, wired as the phases land |
| `HFS_SUBSCRIPTIONS_FANOUT` | `memory` (`pg-notify` under `HFS_CLUSTER`) | Subscriptions cluster delivery (Phase 3): shared counters/tokens/outbox on the Postgres primary + LISTEN/NOTIFY fan-out; non-Postgres primaries warn and stay per-instance |

`HFS_CLUSTER=true` refuses to boot on: a SQLite primary (F1), `local-fs` bulk export/submit output (F2), `HFS_AUDIT_BACKEND=file` (F4), an explicit `memory` job store, (with SoF enabled) a `memory` export controller / `fs` export sink (A1), or (with subscriptions enabled) an explicit `memory` fanout or a non-Postgres primary (B). The former C1 jti check went away with the jti subsystem (#205).

JWKS refresh coordination (C2, warn-only — never a refusal): `HFS_AUTH_JWKS_COORDINATION` = `local` | `database` | `redis`; unset resolves to `database` under `HFS_CLUSTER` (Postgres primary; other primaries warn and stay per-instance). `redis` needs the `redis` build feature plus `HFS_AUTH_REDIS_URL`.

Composite secondary-backend sync durability (E1, Phase 4, warn-only): when a composite storage secondary is present (e.g. `HFS_STORAGE_BACKEND=postgres-elasticsearch`) and the primary is Postgres, `CompositeStorage` wires a durable `composite_sync_outbox` automatically — capability-based, no env var. A composite secondary on a non-Postgres primary under `HFS_CLUSTER` warns and keeps the pre-existing in-memory-channel behavior rather than refusing boot.

HTS terminology-server clustering (bootstrap lock D1, cache invalidation C3) is a standalone `hts`-binary concern with its own `HTS_*` env var, not `HFS_CLUSTER`-gated — see `/work-with-hts`.

Operator guide: book chapter "Running HFS in a Cluster" (`book/src/ch15-cluster-deployment.md`).

## Behavior

| Variable | Default | Description |
|---|---|---|
| `HFS_DEFAULT_FHIR_VERSION` | `R4` | Default FHIR version: R4, R4B, R5, R6 |
| `HFS_ENABLE_REQUEST_ID` | `true` | Enable request ID tracking |
| `HFS_RETURN_GONE` | `true` | Return 410 Gone for deleted resources instead of 404 |
| `HFS_ENABLE_VERSIONING` | `true` | Enable ETag versioning |
| `HFS_REQUIRE_IF_MATCH` | `false` | Require If-Match header for updates |

## API Endpoints

| Interaction | Method | URL |
|---|---|---|
| capabilities | GET | `/metadata` |
| read | GET | `/[type]/[id]` |
| vread | GET | `/[type]/[id]/_history/[vid]` |
| update | PUT | `/[type]/[id]` |
| patch | PATCH | `/[type]/[id]` |
| delete | DELETE | `/[type]/[id]` |
| create | POST | `/[type]` |
| search | GET/POST | `/[type]?params` or `/[type]/_search` |
| history, instance | GET | `/[type]/[id]/_history` |
| history, type | GET | `/[type]/_history` |
| history, system | GET | `/_history` |
| batch/transaction | POST | `/` |
| health | GET | `/health` |
| purge, instance | DELETE | `/[type]/[id]/$purge` |
| purge, type | POST | `/[type]/$purge` |
| reindex | POST | `/$reindex`, `/[type]/$reindex` |
| reindex status / cancel | GET/DELETE | `/$reindex-status/[job_id]` |

`$purge` (permanent, irreversible deletion including history) and `$reindex`
(rebuild the search index) are administrative, non-FHIR operations. They require
the `system/purge` / `system/reindex` scopes, never ordinary resource scopes.
`AuditEvent` can never be purged. `$reindex` returns `501` on the `s3` backend
standalone (no search index exists to rebuild) and its job state is per-process,
so poll the node you kicked off against. See the
[helios-rest README](../../../crates/rest/README.md#administrative-operations).
