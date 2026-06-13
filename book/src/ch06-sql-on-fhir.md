# SQL-on-FHIR

The `helios-sof` crate implements the [SQL-on-FHIR specification](https://build.fhir.org/ig/FHIR/sql-on-fhir-v2). It transforms FHIR resources into flat tabular data using declarative **ViewDefinitions** and ships two executables (`sof-cli` and `sof-server`).

---

## What Is SQL-on-FHIR and ViewDefinitions?

FHIR data is deeply nested and polymorphic — a single `Patient` resource might have an array of names, each with an array of given names, all with multiple use codes. SQL-on-FHIR solves the problem of getting this data into flat tabular form (rows and columns) that SQL engines, pandas, and BI tools can consume natively.

A **ViewDefinition** is a JSON document that describes how to flatten a FHIR resource type into a table. It specifies:
- Which resource type to flatten (`from.resourceType`)
- Which columns to extract, each with a name and a FHIRPath expression (`select[].column[]`)
- Optional `forEach` clauses to iterate over repeated elements

---

## Writing Your First ViewDefinition

Here is a complete ViewDefinition that extracts key demographics from `Patient` resources:

```json
{
  "resourceType": "ViewDefinition",
  "id": "patient-demographics",
  "name": "PatientDemographics",
  "title": "Basic Patient Demographics",
  "description": "Flattened patient demographic data",
  "from": {
    "resourceType": "Patient"
  },
  "select": [
    {
      "column": [
        {"name": "id",           "path": "getResourceKey()"},
        {"name": "birth_date",   "path": "birthDate"},
        {"name": "gender",       "path": "gender"},
        {"name": "first_name",   "path": "name.where(use='official').given.first()"},
        {"name": "last_name",    "path": "name.where(use='official').family"},
        {"name": "ssn",          "path": "identifier.where(system='http://hl7.org/fhir/sid/us-ssn').value"},
        {"name": "email",        "path": "telecom.where(system='email').value"},
        {"name": "phone",        "path": "telecom.where(system='phone' and use='mobile').value"},
        {"name": "address_line", "path": "address.where(use='home').line.join(', ')"},
        {"name": "city",         "path": "address.where(use='home').city"},
        {"name": "state",        "path": "address.where(use='home').state"},
        {"name": "postal_code",  "path": "address.where(use='home').postalCode"}
      ]
    }
  ]
}
```

Save this as `patient-view.json` and run:

```bash
sof-cli --view patient-view.json --bundle patients.json
```

Output (CSV):
```
id,birth_date,gender,first_name,last_name,ssn,email,phone,...
p1,1980-04-14,male,John,Smith,,john@example.com,555-1234,...
```

---

## Using sof-cli for Batch Transforms

```
sof-cli [OPTIONS]
```

### Core flags

| Flag | Description |
|------|-------------|
| `-v, --view <FILE>` | Path to ViewDefinition JSON (or stdin) |
| `-b, --bundle <FILE>` | Path to FHIR Bundle or NDJSON file (or stdin) |
| `-s, --source <URL>` | URL-based data source (local, http, s3, gs, azure) |
| `-f, --format <FMT>` | Output format: `csv`, `json`, `ndjson`, `parquet` (default: `csv`) |
| `--no-headers` | Omit CSV header row |
| `-o, --output <FILE>` | Write to file instead of stdout |
| `--since <RFC3339>` | Filter resources modified after this time |
| `--limit <N>` | Limit results to N rows (1–10000) |
| `--fhir-version <VER>` | FHIR version: `R4`, `R4B`, `R5`, `R6` (default: `R4`) |

### Parquet-specific flags

| Flag | Default | Description |
|------|---------|-------------|
| `--parquet-row-group-size <MB>` | 256 | Row group size in MB (64–1024) |
| `--parquet-page-size <KB>` | 1024 | Page size in KB (64–8192) |
| `--parquet-compression <ALG>` | snappy | `none`, `snappy`, `gzip`, `lz4`, `brotli`, `zstd` |
| `--max-file-size <MB>` | 1000 | Max file size; creates numbered files when exceeded |

### Streaming NDJSON flags

| Flag | Default | Description |
|------|---------|-------------|
| `--chunk-size <N>` | 1000 | Resources per chunk for streaming NDJSON |
| `--skip-invalid` | false | Skip invalid JSON lines instead of failing |

### Examples

```bash
# CSV from a Bundle file
sof-cli -v patient-view.json -b patients.json

# CSV without headers
sof-cli -v view.json -b data.json --no-headers

# JSON output to file
sof-cli -v obs-view.json -b lab-results.json -f json -o output.json

# NDJSON (one row object per line)
sof-cli -v view.json -b data.json -f ndjson

# Parquet with Zstd compression
sof-cli -v view.json -b data.json -f parquet --parquet-compression zstd -o output.parquet

# Large NDJSON — streaming mode
sof-cli -v view.json -b large-data.ndjson --chunk-size 500

# Filter by time window
sof-cli -v view.json -b data.json --since 2024-01-01T00:00:00Z --limit 500

# Read ViewDefinition from stdin
cat view.json | sof-cli -b data.json -f csv
```

---

## Output Formats

| Format | Description |
|--------|-------------|
| `csv` | Comma-separated values with optional header row |
| `json` | Pretty-printed JSON array of row objects |
| `ndjson` | One JSON row object per line (newline-delimited) |
| `parquet` | Columnar binary format; Snappy compression by default |

### Parquet Type Mapping

| FHIR type | Arrow / Parquet type |
|-----------|----------------------|
| `boolean` | `BOOLEAN` |
| `string`, `code`, `uri` | `UTF8` |
| `integer` | `INT32` |
| `decimal` | `FLOAT64` |
| `dateTime`, `date` | `UTF8` |
| Arrays | Arrow `List` type |

All fields are `OPTIONAL`. Snappy compression is the default.

---

## Using the sof-server HTTP API

```bash
# Start with defaults (port 8080)
sof-server

# Custom port
SOF_SERVER_PORT=9090 sof-server
```

### Endpoint

`POST /ViewDefinition/$viewdefinition-run`

Request body (JSON):

| Parameter | Type | Description |
|-----------|------|-------------|
| `_format` | string | Output format: `csv`, `ndjson`, `json`, `parquet` |
| `header` | boolean | Include CSV header row (`true` / `false`) |
| `viewResource` | object | ViewDefinition resource |
| `resource` | array | FHIR resources to transform |
| `patient` | string | Filter by patient reference |
| `_limit` | integer | Limit results (1–10000) |
| `_since` | string | Filter by modification time (RFC3339) |

Parameter precedence: **request body > query params > `Accept` header**

### Example

```bash
curl -X POST http://localhost:8080/ViewDefinition/\$viewdefinition-run \
  -H "Content-Type: application/json" \
  -d '{
    "_format": "csv",
    "viewResource": {
      "resourceType": "ViewDefinition",
      "from": {"resourceType": "Patient"},
      "select": [{"column": [
        {"name": "id", "path": "getResourceKey()"},
        {"name": "family", "path": "name.first().family"}
      ]}]
    },
    "resource": [
      {"resourceType": "Patient", "id": "p1", "name": [{"family": "Smith"}]},
      {"resourceType": "Patient", "id": "p2", "name": [{"family": "Jones"}]}
    ]
  }'
```

### Server environment variables

| Variable | Default | Description |
|----------|---------|-------------|
| `SOF_SERVER_PORT` | `8080` | Server port |
| `SOF_SERVER_HOST` | `127.0.0.1` | Host to bind |
| `SOF_LOG_LEVEL` | `info` | Log level |
| `SOF_MAX_BODY_SIZE` | `10485760` | Max request body (bytes; applies to the decompressed body for compressed requests) |
| `SOF_REQUEST_TIMEOUT` | `30` | Request timeout (seconds) |
| `SOF_ENABLE_CORS` | `true` | Enable CORS |
| `SOF_CORS_ORIGINS` | `*` | Allowed CORS origins |

---

## Reading from Cloud Storage

The `--source` flag (aliased `-s`) accepts URL-based data sources. This is distinct from `--bundle` (`-b`), which accepts local file paths only.

### Supported protocols

| Protocol | Example |
|----------|---------|
| Local (absolute) | `sof-cli -v view.json -s /data/patients.ndjson` |
| Local (relative) | `sof-cli -v view.json -s ./data/patients.json` |
| File URI | `sof-cli -v view.json -s file:///data/patients.json` |
| HTTP/HTTPS | `sof-cli -v view.json -s https://example.com/fhir/bundle.json` |
| Amazon S3 | `sof-cli -v view.json -s s3://my-bucket/fhir-data/patients.ndjson` |
| Google Cloud Storage | `sof-cli -v view.json -s gs://my-bucket/fhir-data/patients.json` |
| Azure Blob Storage | `sof-cli -v view.json -s azure://my-container/fhir-data/patients.ndjson` |

```bash
# S3 example
sof-cli -v patient-view.json \
  -s s3://my-fhir-bucket/exports/patients.ndjson \
  -f parquet -o patients.parquet

# GCS example
sof-cli -v obs-view.json \
  -s gs://my-fhir-bucket/observations.ndjson \
  -f csv -o observations.csv

# Azure example
sof-cli -v view.json \
  -s azure://my-container/data/bundle.json \
  -f json
```
