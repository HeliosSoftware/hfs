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
