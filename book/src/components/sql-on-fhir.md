# SQL-on-FHIR

The `helios-sof` crate implements the [SQL-on-FHIR](http://hl7.org/fhir/uv/sql-on-fhir/index.html) specification. It transforms FHIR resources into tabular data using ViewDefinitions and ships two executables.

## CLI — `sof-cli`

```bash
# Transform a FHIR Bundle to CSV (default)
sof-cli --view examples/patient-view.json --bundle examples/patients.json

# From NDJSON
sof-cli --view examples/patient-view.json --bundle examples/patients.ndjson

# Output formats
sof-cli --view view.json --bundle data.json --format csv
sof-cli --view view.json --bundle data.json --format ndjson
sof-cli --view view.json --bundle data.json --format json
sof-cli --view view.json --bundle data.json --format parquet
```

## HTTP Server — `sof-server`

```bash
# Start with defaults (port 8080)
cargo run --bin sof-server
```

### Endpoint

`POST /$sql-run`

Request body parameters (JSON):

| Parameter | Description |
|-----------|-------------|
| `_format` | Output format: `csv`, `ndjson`, `json`, `parquet` |
| `header` | CSV header row: `true` / `false` |
| `subjectResource` | ViewDefinition resource (JSON) |
| `resource` | FHIR resources to transform |
| `patient` | Filter by patient reference |
| `_limit` | Limit results (1–10000) |
| `_since` | Filter by modification time |

Parameter precedence: request body > query params > `Accept` header.

### Server Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `SOF_SERVER_PORT` | 8080 | Server port |
| `SOF_SERVER_HOST` | 127.0.0.1 | Host to bind |
| `SOF_LOG_LEVEL` | info | Log level |
| `SOF_MAX_BODY_SIZE` | 10485760 | Max request body (bytes; applies to the decompressed body for compressed requests) |
| `SOF_REQUEST_TIMEOUT` | 30 | Request timeout (seconds) |
| `SOF_ENABLE_CORS` | true | Enable CORS |
| `SOF_CORS_ORIGINS` | `*` | Allowed origins |

## Input Formats

- FHIR Bundle (JSON)
- NDJSON (newline-delimited JSON)
- Cloud storage: S3, GCS, Azure Blob Storage

## Output Formats

| Format | Notes |
|--------|-------|
| CSV | Default |
| JSON | Array of row objects |
| NDJSON | One row object per line |
| Parquet | Snappy compression; types follow the SQL-on-FHIR 3.0.0-ballot default mappings |

### Parquet Type Mapping

| FHIR type | Arrow / Parquet type |
|-----------|----------------------|
| boolean | BOOLEAN |
| integer, positiveInt, unsignedInt | INT32 |
| integer64 | INT64 |
| decimal | UTF8 (lexical form preserved) |
| instant | TIMESTAMP (µs, UTC) |
| base64Binary | BINARY |
| date, dateTime, time | UTF8 |
| string, code, uri, other | UTF8 |
| collection columns | Arrow List of the element type |

A column `tag` named `ansi/type` overrides the default (for example `DATE` on a `date` column gives a native date); an unknown or incompatible value is rejected. Columns with no declared `type` are inferred from the values. See the [`helios-sof` README](https://github.com/HeliosSoftware/hfs/blob/main/crates/sof/README.md#column-type-mapping) for the accepted values and the per-emitter comparison. Decimal columns were previously FLOAT64; add `ansi/type: DOUBLE PRECISION` to keep that.

All fields are OPTIONAL.
