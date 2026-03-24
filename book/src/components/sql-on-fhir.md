# SQL-on-FHIR

The `helios-sof` crate implements the [SQL-on-FHIR](https://sql-on-fhir.org/ig/latest/index.html) specification. It transforms FHIR resources into tabular data using ViewDefinitions and ships two executables.

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

`POST /ViewDefinition/$viewdefinition-run`

Request body parameters (JSON):

| Parameter | Description |
|-----------|-------------|
| `_format` | Output format: `csv`, `ndjson`, `json`, `parquet` |
| `header` | CSV header row: `true` / `false` |
| `viewResource` | ViewDefinition resource (JSON) |
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
| `SOF_MAX_BODY_SIZE` | 10485760 | Max request body (bytes) |
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
| Parquet | Snappy compression; follows Pathling type conventions |

### Parquet Type Mapping

| FHIR type | Arrow / Parquet type |
|-----------|----------------------|
| boolean | BOOLEAN |
| string, code, uri | UTF8 |
| integer | INT32 |
| decimal | FLOAT64 |
| dateTime, date | UTF8 |
| arrays | Arrow List |

All fields are OPTIONAL.
