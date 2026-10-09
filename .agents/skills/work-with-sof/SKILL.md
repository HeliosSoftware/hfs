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
| integer | INT32 |
| decimal | FLOAT64 |
| dateTime, date | UTF8 |

Arrays map to Arrow List types. All fields are OPTIONAL. Snappy compression is the default.

## Testing

- Unit tests live in `src/` files.
- Integration tests live in `tests/`.
- ViewDefinition examples are embedded in test files.
- For version-independent logic, use enum wrappers such as `SofViewDefinition` and traits such as `ViewDefinitionTrait`, `BundleTrait`, and `ResourceTrait`.
