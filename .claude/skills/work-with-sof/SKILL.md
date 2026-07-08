---
name: work-with-sof
description: Work on SQL-on-FHIR, ViewDefinition processing, sof-cli, sof-server, SOF HTTP configuration, output formats, or parquet export. Use for helios-sof changes, ViewDefinitionTrait behavior, transformation tests, and SOF server endpoints.
---

# SQL-on-FHIR

Use this when working in `helios-sof`, `sof-cli`, `sof-server`, or ViewDefinition transformation code.

## Server Environment

| Variable | Default | Description |
|---|---|---|
| `SOF_SERVER_PORT` | `8080` | Server port |
| `SOF_SERVER_HOST` | `127.0.0.1` | Host to bind |
| `SOF_LOG_LEVEL` | `info` | Log level |
| `SOF_MAX_BODY_SIZE` | `10485760` | Max request body size in bytes, applied after decompression |
| `SOF_REQUEST_TIMEOUT` | `30` | Request timeout in seconds |
| `SOF_ENABLE_CORS` | `true` | Enable CORS |
| `SOF_CORS_ORIGINS` | `*` | Allowed origins |
| `SOF_TERMINOLOGY_SERVER` | none | Terminology server URL for FHIRPath memberOf and subsumes |

## API Endpoints

- `GET /metadata`: return CapabilityStatement.
- `GET /health`: health check.
- `POST /ViewDefinition/$viewdefinition-run`: execute ViewDefinition transformation.

`$viewdefinition-run` accepts these parameters in the request body or query:

- `_format`: csv, ndjson, json, or parquet.
- `header`: CSV header control, true or false.
- `viewResource`: ViewDefinition resource.
- `resource`: FHIR resources to transform.
- `patient`: filter by patient reference.
- `_limit`: limit results, 1 to 10000.
- `_since`: filter by modification time.

Parameter precedence is request body, then query params, then Accept header.

## Parquet Export

```bash
# CLI
cargo run --bin sof-cli -- --view view.json --bundle data.json --format parquet

# Server
curl -X POST http://localhost:8080/ViewDefinition/\$viewdefinition-run \
  -H "Content-Type: application/json" \
  -d '{"_format": "parquet", "viewResource": {...}, "resource": [...]}'
```

Parquet type mapping follows Pathling conventions:

| SOF type | Parquet type |
|---|---|
| boolean | BOOLEAN |
| string, code, uri | UTF8 |
| integer | INT32 |
| decimal | FLOAT64 |
| dateTime, date | UTF8 |

Arrays map to Arrow List types. All fields are OPTIONAL. Snappy compression is the default.

## Async Export in the HFS Server

The `hfs` binary embeds SQL-on-FHIR via `helios-rest` and exposes the async
`$viewdefinition-export` and `$sqlquery-export` operations. These run in the
background and write tabular shards to an *export sink*, configured with the
`HFS_EXPORT_*` variables below (distinct from the standalone `sof-server`
`SOF_*` vars above, and from Bulk Data Export's `HFS_BULK_EXPORT_*`).

The subsystem is gated by `HFS_SOF_ENABLED`, and the configured storage backend
must provide an in-DB SOF runner (`sqlite` or `postgres`) — there is no
in-process fallback.

| Variable | Default | Description |
|---|---|---|
| `HFS_SOF_ENABLED` | `true` | Master switch for SQL-on-FHIR ops (`$viewdefinition-run`/`-export`, `$sqlquery-*`) |
| `HFS_EXPORT_SINK` | `fs` | Output sink: `fs` (local filesystem) or `s3` |
| `HFS_EXPORT_DIR` | `./exports` | Root directory for the `fs` sink |
| `HFS_EXPORT_S3_BUCKET` | *(none)* | S3 bucket — required when `HFS_EXPORT_SINK=s3` |
| `HFS_EXPORT_S3_REGION` | *(AWS chain)* | AWS region override for the `s3` sink |
| `HFS_EXPORT_PRESIGN_TTL_SECS` | `86400` | Pre-signed download-URL lifetime for `s3`, seconds (spec requires ≥ 24h) |
| `HFS_EXPORT_MAX_CONCURRENCY` | `4` | Maximum concurrent export jobs |
| `HFS_EXPORT_SHARD_ROWS` | `500000` | Target rows per output shard; larger results split across files |
| `HFS_EXPORT_CONTROLLER` | `memory` | Job-controller backend (`memory` in-process; `kafka`/`sqs` reserved) |
| `HFS_EXPORT_OUTPUT_TTL` | `86400` | Retention (seconds) for a finished job's output + bookkeeping; the cleanup reaper then deletes shards and drops the job (later polls/downloads → `404`) |
| `HFS_EXPORT_CLEANUP_INTERVAL` | `300` | Cleanup-reaper scan interval, seconds (clamped to ≥ 1) |

Cancelling a job (`DELETE` on the status URL) or a mid-run failure deletes that
job's partial shards immediately; the reaper reclaims *completed* jobs once they
age past `HFS_EXPORT_OUTPUT_TTL`. Full `HFS_EXPORT_*` reference lives in the
[helios-rest README](../../../crates/rest/README.md#sql-on-fhir-async-export).

## Testing

- Unit tests live in `src/` files.
- Integration tests live in `tests/`.
- ViewDefinition examples are embedded in test files.
- For version-independent logic, use enum wrappers such as `SofViewDefinition` and traits such as `ViewDefinitionTrait`, `BundleTrait`, and `ResourceTrait`.
