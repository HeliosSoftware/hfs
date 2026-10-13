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
- `POST /$sql-run`: execute ViewDefinition transformation.

`$sql-run` accepts these parameters in the request body or query:

- `_format`: csv, ndjson, json, or parquet.
- `header`: CSV header control, true or false.
- `subjectResource`: ViewDefinition resource.
- `resource`: FHIR resources to transform.
- `patient`: filter by patient reference.
- `group`: filter by Group reference (members' compartments).
- `_limit`: limit results, 1 to 10000.
- `_since`: filter by modification time.

On the HFS server, `patient`/`group`/`_since` also apply to every dependency view of a Library (SQLQuery/SQLView) subject, and `_limit` caps only the final rows.

When changing HFS stored-data SQL row order, collection cells or preview
limits, read the
[stored-data ordering contract](../../../book/src/ch06-sql-on-fhir.md#hfs-stored-data-ordering).

Parameter precedence is request body, then query params, then Accept header.

## Parquet Export

```bash
# CLI
cargo run --bin sof-cli -- --view view.json --bundle data.json --format parquet

# Server
curl -X POST http://localhost:8080/\$sql-run \
  -H "Content-Type: application/json" \
  -d '{"_format": "parquet", "subjectResource": {...}, "resource": [...]}'
```

Parquet type mapping follows Pathling conventions:

| SOF type | Parquet type |
|---|---|
| boolean | BOOLEAN |
| string, code, uri | UTF8 |
| integer, positiveInt, unsignedInt | INT32 |
| integer64 | INT64 |
| decimal | FLOAT64 |
| dateTime, date | UTF8 |

Declared column `type` wins; columns without one are inferred from the first 100 rows only, and a later value that does not fit (e.g. a non-integer or an integer above `i32::MAX` in an Int32 column) becomes null. `collection: true` and arrays map to `List<UTF8>`. All fields are OPTIONAL. Snappy compression is the default.

Parquet/Arrow are encoded incrementally (`helios_sof::columnar_stream`): schema fixed up front, 8,192-row batches, row groups cut by encoded bytes (`row_group_size_mb`), and the `$sql-run` response streams with an aborted body on a mid-stream error. File splitting by size is per batch, so a file can overshoot by one batch plus the footer. Streamed responses have no `Content-Length` (chunked), the timeout layer does not cover the body, Arrow stays compressible, and a client disconnect stops the encoder and drops the `RowStream` (test: `streaming_columnar_client_disconnect_drops_the_row_stream`). Values that do not fit a declared type become null; the Postgres runner's text cells are parsed by the builders (unit-tested, not run against a live Postgres). Peak-memory evidence: `docs/sof-parquet-streaming-benchmark.md` (`cargo run -p helios-sof --example parquet_peak_memory`). Row groups are cut at `row_group_size_mb` encoded bytes (REST/CLI minimum 64 MB), so per-request peak memory is about 3x that (~190 MB at the minimum). The first split file keeps the plain name and later files are `_002`, `_003` (no `_001`).

## Testing

- Unit tests live in `src/` files.
- Integration tests live in `tests/`.
- ViewDefinition examples are embedded in test files.
- For version-independent logic, use enum wrappers such as `SofViewDefinition` and traits such as `ViewDefinitionTrait`, `BundleTrait`, and `ResourceTrait`.
