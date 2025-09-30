# SQL-on-FHIR Implementation

This crate provides a complete implementation of the SQL-on-FHIR specification for Rust, enabling the transformation of FHIR resources into tabular data using declarative ViewDefinitions. It supports all major FHIR versions (R4, R4B, R5, R6) through a version-agnostic abstraction layer.

## Overview

The `sof` crate implements the [HL7 FHIR SQL-on-FHIR Implementation Guide](https://build.fhir.org/ig/FHIR/sql-on-fhir-v2), providing:

- **ViewDefinition Processing** - Transform FHIR resources into tabular data using declarative configuration
- **Multi-Version Support** - Works seamlessly with R4, R4B, R5, and R6 FHIR specifications
- **FHIRPath Integration** - Complex data extraction using FHIRPath expressions
- **Multiple Output Formats** - CSV, JSON, NDJSON, and Parquet support
- **Command Line Interface** - Ready-to-use CLI tool for batch processing
- **Server Implementation** - HTTP API for on-demand transformations (planned)

## Executables

This crate provides two executable targets:

### `sof-cli` - Command Line Interface

A full-featured command-line (CLI) tool for running ViewDefinition transformations.
The CLI tool accepts FHIR ViewDefinition and Bundle resources as input (either from
files or stdin) and applies the SQL-on-FHIR transformation to produce structured
output in formats like CSV, JSON, or other supported content types.


```bash
# Basic CSV output (includes headers by default)
sof-cli --view patient-view.json --bundle patient-data.json --format csv

# CSV output without headers
sof-cli --view patient-view.json --bundle patient-data.json --format csv --no-headers

# JSON output to file
sof-cli -v observation-view.json -b lab-results.json -f json -o output.json

# Read ViewDefinition from stdin, Bundle from file
cat view-definition.json | sof-cli --bundle patient-data.json --format csv

# Read Bundle from stdin, ViewDefinition from file
cat patient-bundle.json | sof-cli --view view-definition.json --format json

# Load data using --source parameter (supports multiple protocols)
sof-cli -v view-definition.json -s file:///path/to/bundle.json -f csv
sof-cli -v view-definition.json -s https://example.com/fhir/bundle.json -f json
sof-cli -v view-definition.json -s s3://my-bucket/fhir-data/bundle.json -f csv
sof-cli -v view-definition.json -s gs://my-bucket/fhir-data/bundle.json -f json
sof-cli -v view-definition.json -s azure://my-container/fhir-data/bundle.json -f ndjson

# Filter resources modified after a specific date
sof-cli -v view-definition.json -b patient-data.json --since 2024-01-01T00:00:00Z -f csv

# Limit results to first 100 rows
sof-cli -v view-definition.json -b patient-data.json --limit 100

# Combine filters: recent resources limited to 50 results
sof-cli -v view-definition.json -b patient-data.json --since 2024-01-01T00:00:00Z --limit 50
```

#### CLI Features

- **Flexible Input**:
  - Read ViewDefinitions from file (`-v`) or stdin
  - Read Bundles from file (`-b`), stdin, or external sources (`-s`)
  - Use `-s/--source` to load from URLs: `file://`, `http(s)://`, `s3://`, `gs://`, `azure://`
  - Cannot read both ViewDefinition and Bundle from stdin simultaneously
- **Output Formats**: CSV (with/without headers), JSON (pretty-printed array), NDJSON (newline-delimited), Parquet (columnar binary format)
- **Output Options**: Write to stdout (default) or specified file with `-o`
- **Result Filtering**:
  - Filter resources by modification time with `--since` (RFC3339 format)
  - Limit number of results with `--limit` (1-10000)
- **FHIR Version Support**: R4 by default; other versions (R4B, R5, R6) require compilation with feature flags
- **Error Handling**: Clear, actionable error messages for debugging

#### Command Line Options

```
-v, --view <VIEW>              Path to ViewDefinition JSON file (or use stdin if not provided)
-b, --bundle <BUNDLE>          Path to FHIR Bundle JSON file (or use stdin if not provided)
-s, --source <SOURCE>          URL or path to FHIR data source (see Data Sources below)
-f, --format <FORMAT>          Output format (csv, json, ndjson, parquet) [default: csv]
    --no-headers               Exclude CSV headers (only for CSV format)
-o, --output <OUTPUT>          Output file path (defaults to stdout)
    --since <SINCE>            Filter resources modified after this time (RFC3339 format)
    --limit <LIMIT>            Limit the number of results (1-10000)
    --fhir-version <VERSION>   FHIR version to use [default: R4]
    --parquet-row-group-size <MB> Row group size for Parquet (64-1024MB) [default: 256]
    --parquet-page-size <KB>   Page size for Parquet (64-8192KB) [default: 1024]
    --parquet-compression <ALG> Compression for Parquet [default: snappy]
                              Options: none, snappy, gzip, lz4, brotli, zstd
    --max-file-size <MB>       Maximum file size for Parquet output (10-10000MB)
                              When exceeded, creates numbered files (e.g., output_001.parquet)
-h, --help                     Print help

* Additional FHIR versions (R4B, R5, R6) available when compiled with corresponding features
```

#### Data Sources

The CLI provides two ways to specify FHIR data:
- `-b/--bundle`: Direct path to a local file (simple, no protocol prefix needed)
- `-s/--source`: URL-based loading with protocol support (more flexible)

The `--source` parameter supports loading FHIR data from various sources:

##### Local Files
```bash
# Using --bundle (simpler for local files)
sof-cli -v view.json -b /path/to/bundle.json

# Using --source with file:// protocol
sof-cli -v view.json -s file:///path/to/bundle.json
```

##### HTTP/HTTPS URLs
```bash
sof-cli -v view.json -s https://example.com/fhir/bundle.json
```

##### AWS S3
```bash
# Set AWS credentials
export AWS_ACCESS_KEY_ID=your_access_key
export AWS_SECRET_ACCESS_KEY=your_secret_key
export AWS_REGION=us-east-1

# Load from S3 bucket
sof-cli -v view.json -s s3://my-bucket/fhir-data/bundle.json -f csv
```

##### Google Cloud Storage
```bash
# Option 1: Service account credentials
export GOOGLE_SERVICE_ACCOUNT=/path/to/service-account.json

# Option 2: Application Default Credentials
gcloud auth application-default login

# Load from GCS bucket
sof-cli -v view.json -s gs://my-bucket/fhir-data/bundle.json -f json
```

##### Azure Blob Storage
```bash
# Option 1: Storage account credentials
export AZURE_STORAGE_ACCOUNT=myaccount
export AZURE_STORAGE_ACCESS_KEY=mykey

# Option 2: Azure managed identity (when running in Azure)
# No environment variables needed

# Load from Azure container
sof-cli -v view.json -s azure://my-container/fhir-data/bundle.json -f ndjson
```

The source can contain:
- A FHIR Bundle
- A single FHIR resource (will be wrapped in a Bundle)
- An array of FHIR resources (will be wrapped in a Bundle)

#### Output Formats

The CLI supports multiple output formats via the `-f/--format` parameter:

- **csv** (default) - Comma-separated values format
  - Includes headers by default
  - Use `--no-headers` flag to exclude column headers
  - All values are properly quoted according to CSV standards
  
- **json** - JSON array format
  - Pretty-printed for readability
  - Each row is a JSON object with column names as keys
  - Suitable for further processing with JSON tools
  
- **ndjson** - Newline-delimited JSON format
  - One JSON object per line
  - Streaming-friendly format
  - Ideal for processing large datasets
  
- **parquet** - Apache Parquet columnar format
  - Efficient binary format for analytics workloads
  - Automatic schema inference from data
  - Configurable compression (snappy, gzip, lz4, brotli, zstd, none)
  - Optimized for large datasets with automatic chunking
  - Configurable row group and page sizes for performance tuning

### `sof-server` - HTTP Server

A high-performance HTTP server providing SQL-on-FHIR ViewDefinition transformation capabilities with
advanced Parquet support and streaming for large datasets. Use this server if you need a stateless,
simple web service for SQL-on-FHIR implementations. Should you need to perform SQL-on-FHIR
transformations using server-stored ViewDefinitions and server-stored FHIR data, use the full
capabilities of the Helios FHIR Server in [hfs](../hfs).


```bash
# Start server with defaults
sof-server

# Custom configuration via command line
sof-server --port 3000 --host 0.0.0.0 --log-level debug

# Custom configuration via environment variables
SOF_SERVER_PORT=3000 SOF_SERVER_HOST=0.0.0.0 sof-server

# Check server health
curl http://localhost:8080/health

# Get CapabilityStatement
curl http://localhost:8080/metadata
```

#### Configuration

The server can be configured using either command-line arguments or environment variables. Command-line arguments take precedence when both are provided.

##### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `SOF_SERVER_PORT` | Server port | `8080` |
| `SOF_SERVER_HOST` | Server host address | `127.0.0.1` |
| `SOF_LOG_LEVEL` | Log level (error, warn, info, debug, trace) | `info` |
| `SOF_MAX_BODY_SIZE` | Maximum request body size in bytes | `10485760` (10MB) |
| `SOF_REQUEST_TIMEOUT` | Request timeout in seconds | `30` |
| `SOF_ENABLE_CORS` | Enable CORS (true/false) | `true` |
| `SOF_CORS_ORIGINS` | Allowed CORS origins (comma-separated, * for any) | `*` |
| `SOF_CORS_METHODS` | Allowed CORS methods (comma-separated, * for any) | `GET,POST,PUT,DELETE,OPTIONS` |
| `SOF_CORS_HEADERS` | Allowed CORS headers (comma-separated, * for any) | Common headers¹ |

##### Command-Line Arguments

| Argument | Short | Description | Default |
|----------|-------|-------------|---------|
| `--port` | `-p` | Server port | `8080` |
| `--host` | `-H` | Server host address | `127.0.0.1` |
| `--log-level` | `-l` | Log level | `info` |
| `--max-body-size` | `-m` | Max request body (bytes) | `10485760` |
| `--request-timeout` | `-t` | Request timeout (seconds) | `30` |
| `--enable-cors` | `-c` | Enable CORS | `true` |
| `--cors-origins` | | Allowed origins (comma-separated) | `*` |
| `--cors-methods` | | Allowed methods (comma-separated) | `GET,POST,PUT,DELETE,OPTIONS` |
| `--cors-headers` | | Allowed headers (comma-separated) | Common headers¹ |

##### Examples

```bash
# Production configuration with environment variables
export SOF_SERVER_HOST=0.0.0.0
export SOF_SERVER_PORT=8080
export SOF_LOG_LEVEL=warn
export SOF_MAX_BODY_SIZE=52428800  # 50MB
export SOF_REQUEST_TIMEOUT=60
export SOF_ENABLE_CORS=false
sof-server

# Development configuration
sof-server --log-level debug --enable-cors true

# CORS configuration for specific frontend
sof-server --cors-origins "http://localhost:3000,http://localhost:3001" \
           --cors-methods "GET,POST,OPTIONS" \
           --cors-headers "Content-Type,Authorization"

# Disable CORS for internal services
sof-server --enable-cors false

# Show all configuration options
sof-server --help
```

##### Cloud Storage Configuration

When using the `source` parameter with cloud storage URLs, ensure the appropriate credentials are configured:

**AWS S3** (`s3://` URLs):
```bash
export AWS_ACCESS_KEY_ID=your_access_key
export AWS_SECRET_ACCESS_KEY=your_secret_key
export AWS_REGION=us-east-1
sof-server
```

**Google Cloud Storage** (`gs://` URLs):
```bash
# Option 1: Service account
export GOOGLE_SERVICE_ACCOUNT=/path/to/service-account.json
sof-server

# Option 2: Application Default Credentials
gcloud auth application-default login
sof-server
```

**Azure Blob Storage** (`azure://` URLs):
```bash
# Option 1: Storage account credentials
export AZURE_STORAGE_ACCOUNT=myaccount
export AZURE_STORAGE_ACCESS_KEY=mykey
sof-server

# Option 2: Use managed identity when running in Azure
sof-server
```

##### CORS Configuration

The server provides flexible CORS (Cross-Origin Resource Sharing) configuration to control which web applications can access the API:

- **Origins**: Specify which domains can access the server
  - Use `*` to allow any origin (default)
  - Provide comma-separated list for specific origins: `https://app1.com,https://app2.com`
  
- **Methods**: Control which HTTP methods are allowed
  - Default: `GET,POST,PUT,DELETE,OPTIONS`
  - Use `*` to allow any method
  - Provide comma-separated list: `GET,POST,OPTIONS`
  
- **Headers**: Specify which headers clients can send
  - Default: Common headers¹
  - Use `*` to allow any header
  - Provide comma-separated list: `Content-Type,Authorization,X-Custom-Header`

**Important Security Notes**:
1. When using wildcard (`*`) for origins, credentials (cookies, auth headers) are automatically disabled for security
2. To enable credentials, you must specify exact origins, not wildcards
3. In production, always specify exact origins rather than using `*` to prevent unauthorized access

```bash
# Development (permissive, no credentials)
sof-server  # Uses default wildcard origin

# Production CORS configuration (with credentials)
export SOF_CORS_ORIGINS="https://app.example.com"
export SOF_CORS_METHODS="GET,POST,OPTIONS"
export SOF_CORS_HEADERS="Content-Type,Authorization"
sof-server
```

¹ Default headers: `Accept,Accept-Language,Content-Type,Content-Language,Authorization,X-Requested-With`

#### Server Features

- **HTTP API**: RESTful endpoints for ViewDefinition execution
- **CapabilityStatement**: Discovery endpoint for server capabilities
- **ViewDefinition Runner**: Synchronous execution of ViewDefinitions
- **Multi-format Output**: Support for CSV, JSON, NDJSON, and Parquet responses
- **Advanced Parquet Support**:
  - Configurable compression, row group size, and page size
  - Automatic file splitting when size limits are exceeded
  - ZIP archive generation for multi-file outputs
- **Streaming Response**: Chunked transfer encoding for large datasets
- **FHIR Compliance**: Proper OperationOutcome error responses
- **Configurable CORS**: Fine-grained control over cross-origin requests with support for specific origins, methods, and headers

#### API Endpoints

##### GET /metadata
Returns the server's CapabilityStatement describing supported operations:

```bash
curl http://localhost:8080/metadata
```

##### POST /ViewDefinition/$run
Execute a ViewDefinition transformation:

```bash
# JSON output (default)
curl -X POST http://localhost:8080/ViewDefinition/$run \
  -H "Content-Type: application/json" \
  -d '{
    "resourceType": "Parameters",
    "parameter": [{
      "name": "viewResource",
      "resource": {
        "resourceType": "ViewDefinition",
        "status": "active",
        "resource": "Patient",
        "select": [{
          "column": [{
            "name": "id",
            "path": "id"
          }, {
            "name": "gender", 
            "path": "gender"
          }]
        }]
      }
    }, {
      "name": "patient",
      "resource": {
        "resourceType": "Patient",
        "id": "example",
        "gender": "male"
      }
    }]
  }'

# CSV output (includes headers by default)
curl -X POST "http://localhost:8080/ViewDefinition/$run?_format=text/csv" \
  -H "Content-Type: application/json" \
  -d '{...}'

# CSV output without headers
curl -X POST "http://localhost:8080/ViewDefinition/$run?_format=text/csv&header=false" \
  -H "Content-Type: application/json" \
  -d '{...}'

# NDJSON output
curl -X POST http://localhost:8080/ViewDefinition/$run \
  -H "Content-Type: application/json" \
  -H "Accept: application/ndjson" \
  -d '{...}'
```

#### Parameters

The `$run` POST operation accepts parameters either as query parameters or in a FHIR Parameters resource.

Parameter table:

| Name | Type | Use | Scope | Min | Max | Documentation |
|------|------|-----|-------|-----|-----|---------------|
| _format | code | in | type, instance | 1 | 1 | Output format - `application/json`, `application/ndjson`, `text/csv`, `application/parquet` |
| header | boolean | in | type, instance | 0 | 1 | This parameter only applies to `text/csv` requests. `true` (default) - return headers in the response, `false` - do not return headers. |
| maxFileSize | integer | in | type, instance | 0 | 1 | Maximum Parquet file size in MB (10-10000). When exceeded, generates multiple files in a ZIP archive. |
| rowGroupSize | integer | in | type, instance | 0 | 1 | Parquet row group size in MB (64-1024, default: 256) |
| pageSize | integer | in | type, instance | 0 | 1 | Parquet page size in KB (64-8192, default: 1024) |
| compression | code | in | type, instance | 0 | 1 | Parquet compression: none, snappy (default), gzip, lz4, brotli, zstd |
| viewReference | Reference | in | type, instance | 0 | 1 | Reference to ViewDefinition to be used for data transformation. (not yet supported) |
| viewResource | ViewDefinition | in | type | 0 | 1 | ViewDefinition to be used for data transformation. |
| patient | Reference | in | type, instance | 0 | * | Filter resources by patient. |
| group | Reference | in | type, instance | 0 | * | Filter resources by group. (not yet supported) |
| source | string | in | type, instance | 0 | 1 | URL or path to FHIR data source. Supports file://, http(s)://, s3://, gs://, and azure:// protocols. |
| _limit | integer | in | type, instance | 0 | 1 | Limits the number of results. (1-10000) |
| _since | instant | in | type, instance | 0 | 1 | Return resources that have been modified after the supplied time. (RFC3339 format, validates format only) |
| resource | Resource | in | type, instance | 0 | * | Collection of FHIR resources to be transformed into a tabular projection. |

##### Query Parameters

All parameters except `viewReference`, `viewResource`, `patient`, `group`, and `resource` can be provided as POST query parameters:

- **_format**: Output format (required if not in Accept header)
  - `application/json` - JSON array output (default)
  - `text/csv` - CSV output
  - `application/ndjson` - Newline-delimited JSON
  - `application/parquet` - Parquet file 
- **header**: Control CSV headers (only applies to CSV format)
  - `true` - Include headers (default for CSV)
  - `false` - Exclude headers
- **source**: URL to FHIR data (file://, http://, s3://, gs://, azure://)
- **_limit**: Limit results (1-10000)
- **_since**: Filter by modification time (RFC3339 format)
- **maxFileSize**: Maximum Parquet file size in MB (10-10000)
- **rowGroupSize**: Parquet row group size in MB (64-1024)
- **pageSize**: Parquet page size in KB (64-8192)
- **compression**: Parquet compression algorithm

##### Body Parameters

For POST requests, parameters can be provided in a FHIR Parameters resource:

- **_format**: As valueCode or valueString (overrides query params and Accept header)
- **header**: As valueBoolean (overrides query params)
- **viewReference**: As valueReference (not yet supported)
- **viewResource**: As resource (inline ViewDefinition)
- **patient**: As valueReference
- **group**: As valueReference (not yet supported)
- **source**: As valueString (URL to external FHIR data)
- **_limit**: As valueInteger
- **_since**: As valueInstant
- **resource**: As resource (can be repeated)
- **maxFileSize**: As valueInteger (for Parquet output)
- **rowGroupSize**: As valueInteger (for Parquet output)
- **pageSize**: As valueInteger (for Parquet output)
- **compression**: As valueCode or valueString (for Parquet output)

##### Parameter Precedence

When the same parameter is specified in multiple places, the precedence order is:
1. Parameters in request body (highest priority)
2. Query parameters
3. Accept header (for format only, lowest priority)

##### Response Headers

The server automatically sets appropriate response headers based on the output format and size:

**Standard Response Headers:**
- `Content-Type`: Based on format parameter
  - `application/json` for JSON output
  - `text/csv` for CSV output
  - `application/ndjson` for NDJSON output
  - `application/parquet` for single Parquet file
  - `application/zip` for multiple Parquet files

**Streaming Response Headers (for large files):**
- `Transfer-Encoding: chunked` - Automatically set for files > 10MB
- `Content-Disposition: attachment; filename="..."` - Suggests filename for downloads
  - Single Parquet: `filename="data.parquet"`
  - Multiple Parquet (ZIP): `filename="data.zip"`

**Note:** The `Transfer-Encoding: chunked` header is automatically managed by the server. Clients don't need to set any special headers to receive chunked responses - they will automatically receive data in chunks if the response is large.

##### Examples

```bash
# Limit results - first 50 records as CSV
curl -X POST "http://localhost:8080/ViewDefinition/$run?_limit=50&_format=text/csv" \
  -H "Content-Type: application/json" \
  -d '{...}'

# CSV without headers, limited to 20 results
curl -X POST "http://localhost:8080/ViewDefinition/$run?_format=text/csv&header=false&_limit=20" \
  -H "Content-Type: application/json" \
  -d '{...}'

# Using header parameter in request body (overrides query params)
curl -X POST "http://localhost:8080/ViewDefinition/$run?_format=text/csv" \
  -H "Content-Type: application/json" \
  -d '{
    "resourceType": "Parameters",
    "parameter": [{
      "name": "header",
      "valueBoolean": false
    }, {
      "name": "viewResource",
      "resource": {...}
    }]
  }'

# Filter by modification time (requires resources with lastUpdated metadata)
curl -X POST "http://localhost:8080/ViewDefinition/$run?_since=2024-01-01T00:00:00Z" \
  -H "Content-Type: application/json" \
  -d '{...}'

# Load data from S3 bucket
curl -X POST "http://localhost:8080/ViewDefinition/$run?source=s3://my-bucket/bundle.json" \
  -H "Content-Type: application/json" \
  -d '{
    "resourceType": "Parameters",
    "parameter": [{
      "name": "viewResource",
      "resource": {...}
    }]
  }'

# Load data from Azure with filtering
curl -X POST "http://localhost:8080/ViewDefinition/$run?source=azure://container/data.json&_limit=100" \
  -H "Content-Type: application/json" \
  -d '{...}'

# Generate Parquet with custom compression and row group size
curl -X POST "http://localhost:8080/ViewDefinition/$run?_format=application/parquet&compression=zstd&rowGroupSize=512" \
  -H "Content-Type: application/json" \
  -d '{...}' \
  --output result.parquet

# Generate large Parquet with file splitting (returns ZIP if multiple files)
curl -X POST "http://localhost:8080/ViewDefinition/$run?_format=application/parquet&maxFileSize=100" \
  -H "Content-Type: application/json" \
  -d '{...}' \
  --output result.zip

# Using Parquet parameters in request body
curl -X POST "http://localhost:8080/ViewDefinition/$run" \
  -H "Content-Type: application/json" \
  -d '{
    "resourceType": "Parameters",
    "parameter": [{
      "name": "_format",
      "valueCode": "parquet"
    }, {
      "name": "maxFileSize",
      "valueInteger": 500
    }, {
      "name": "compression",
      "valueCode": "brotli"
    }, {
      "name": "viewResource",
      "resource": {...}
    }]
  }' \
  --output result.zip
```

## Core Features

### ViewDefinition Processing

Transform FHIR resources using declarative ViewDefinitions:

```rust
use helios_sof::{SofViewDefinition, SofBundle, ContentType, run_view_definition};

// Parse ViewDefinition and Bundle
let view_definition: fhir::r4::ViewDefinition = serde_json::from_str(view_json)?;
let bundle: fhir::r4::Bundle = serde_json::from_str(bundle_json)?;

// Wrap in version-agnostic containers
let sof_view = SofViewDefinition::R4(view_definition);
let sof_bundle = SofBundle::R4(bundle);

// Transform to CSV with headers
let csv_output = run_view_definition(
    sof_view,
    sof_bundle,
    ContentType::CsvWithHeader
)?;
```

### Multi-Version FHIR Support

Seamlessly work with any supported FHIR version:

```rust
// Version-agnostic processing
match fhir_version {
    FhirVersion::R4 => {
        let view = SofViewDefinition::R4(parse_r4_viewdef(json)?);
        let bundle = SofBundle::R4(parse_r4_bundle(json)?);
        run_view_definition(view, bundle, format)?
    },
    FhirVersion::R5 => {
        let view = SofViewDefinition::R5(parse_r5_viewdef(json)?);
        let bundle = SofBundle::R5(parse_r5_bundle(json)?);
        run_view_definition(view, bundle, format)?
    },
    // ... other versions
}
```

### Advanced ViewDefinition Features

#### forEach Iteration

Process collections with automatic row generation:

```json
{
  "resourceType": "ViewDefinition",
  "resource": "Patient",
  "select": [{
    "forEach": "name",
    "column": [{
      "name": "family_name",
      "path": "family"
    }, {
      "name": "given_name", 
      "path": "given.first()"
    }]
  }]
}
```

#### Constants and Variables

Define reusable values for complex expressions:

```json
{
  "constant": [{
    "name": "loinc_system",
    "valueString": "http://loinc.org"
  }],
  "select": [{
    "where": [{
      "path": "code.coding.where(system = %loinc_system).exists()"
    }],
    "column": [{
      "name": "loinc_code",
      "path": "code.coding.where(system = %loinc_system).code"
    }]
  }]
}
```

#### Where Clauses

Filter resources using FHIRPath expressions:

```json
{
  "where": [{
    "path": "status = 'final'"
  }, {
    "path": "effective.exists()"
  }, {
    "path": "value.exists()"
  }]
}
```

#### Union Operations

Combine multiple select statements:

```json
{
  "select": [{
    "unionAll": [{
      "column": [{"name": "type", "path": "'observation'"}]
    }, {
      "column": [{"name": "type", "path": "'condition'"}]
    }]
  }]
}
```

### Output Formats

Multiple output formats for different integration needs:

```rust
use helios_sof::ContentType;

// CSV without headers
let csv = run_view_definition(view, bundle, ContentType::Csv)?;

// CSV with headers  
let csv_headers = run_view_definition(view, bundle, ContentType::CsvWithHeader)?;

// Pretty-printed JSON array
let json = run_view_definition(view, bundle, ContentType::Json)?;

// Newline-delimited JSON (streaming friendly)
let ndjson = run_view_definition(view, bundle, ContentType::NdJson)?;

// Apache Parquet (columnar binary format)
let parquet = run_view_definition(view, bundle, ContentType::Parquet)?;
```

### Parquet Export

The SOF implementation supports Apache Parquet format for efficient columnar data storage and analytics:

- **Automatic Schema Inference**: Column types are automatically determined from the data
- **FHIR Type Mapping**:
  - `boolean` → BOOLEAN
  - `string`/`code`/`uri` → UTF8
  - `integer` → INT32
  - `decimal` → FLOAT64
  - `dateTime`/`date` → UTF8
  - Arrays → List types with nullable elements
- **Optimized for Large Datasets**:
  - Automatic chunking into optimal batch sizes (100K-500K rows)
  - Memory-efficient streaming for datasets > 1GB
  - Configurable row group size (default: 256MB, range: 64-1024MB)
  - Configurable page size (default: 1MB, range: 64KB-8MB)
- **Compression Options**:
  - `snappy` (default): Fast compression with good ratios
  - `gzip`: Maximum compatibility, good compression
  - `lz4`: Fastest compression/decompression
  - `zstd`: Balanced speed and compression ratio
  - `brotli`: Best compression ratio
  - `none`: No compression for maximum speed
- **Null Handling**: All fields are OPTIONAL to accommodate FHIR's nullable nature
- **Complex Types**: Objects and nested structures are serialized as JSON strings

Example usage:
```bash
# CLI export with default settings (256MB row groups, snappy compression)
sof-cli --view view.json --bundle data.json --format parquet -o output.parquet

# Optimize for smaller files with better compression
sof-cli --view view.json --bundle data.json --format parquet \
  --parquet-compression zstd \
  --parquet-row-group-size 128 \
  -o output.parquet

# Maximize compression for archival
sof-cli --view view.json --bundle data.json --format parquet \
  --parquet-compression brotli \
  --parquet-row-group-size 512 \
  --parquet-page-size 2048 \
  -o output.parquet

# Fast processing with minimal compression
sof-cli --view view.json --bundle data.json --format parquet \
  --parquet-compression lz4 \
  --parquet-row-group-size 64 \
  -o output.parquet

# Split large datasets into multiple files (500MB each)
sof-cli --view view.json --bundle large-data.json --format parquet \
  --max-file-size 500 \
  -o output.parquet
# Creates: output.parquet (first 500MB)
#          output_002.parquet (next 500MB)
#          output_003.parquet (remaining data)

# Server API - single Parquet file
curl -X POST "http://localhost:8080/ViewDefinition/$run?_format=application/parquet" \
  -H "Content-Type: application/json" \
  -d '{"resourceType": "Parameters", ...}' \
  --output result.parquet

# Server API - with file splitting (returns ZIP archive if multiple files)
curl -X POST "http://localhost:8080/ViewDefinition/$run?_format=application/parquet&maxFileSize=100" \
  -H "Content-Type: application/json" \
  -d '{"resourceType": "Parameters", ...}' \
  --output result.zip

# Server API - optimized settings for large datasets
curl -X POST "http://localhost:8080/ViewDefinition/$run?_format=application/parquet&compression=zstd&rowGroupSize=512&maxFileSize=500" \
  -H "Content-Type: application/json" \
  -d '{"resourceType": "Parameters", ...}' \
  --output result.zip
```

#### Performance Guidelines

- **Row Group Size**: Larger row groups (256-512MB) improve compression and columnar efficiency but require more memory during processing
- **Page Size**: Smaller pages (64-512KB) enable fine-grained reads and better predicate pushdown; larger pages (1-8MB) reduce metadata overhead
- **Compression**:
  - Use `snappy` or `lz4` for real-time processing
  - Use `zstd` for balanced storage and query performance
  - Use `brotli` or `gzip` for long-term storage where space is critical
- **Large Datasets**: The implementation automatically chunks data to prevent memory issues, processing in batches optimized for the configured row group size
- **File Splitting**: When `--max-file-size` or `maxFileSize` is specified:
  - Files are split when they exceed the specified size in MB
  - Each file contains complete row groups and is independently queryable
  - CLI: Files are named with sequential numbering: `base.parquet`, `base_002.parquet`, `base_003.parquet`, etc.
  - Server: Multiple files are automatically packaged into a ZIP archive for convenient download
  - Ideal for distributed processing systems that parallelize across files
- **Streaming Response** (Server only):
  - Files larger than 10MB are automatically streamed using chunked transfer encoding
  - Reduces memory usage on both server and client
  - Multiple Parquet files are streamed as a ZIP archive with proper content disposition headers
  - Enables processing of gigabyte-scale datasets without memory constraints
  - Response headers for streaming:
    - `Transfer-Encoding: chunked` - Automatically set by the server for streaming responses
    - `Content-Type: application/parquet` or `application/zip` - Based on single or multi-file output
    - `Content-Disposition: attachment; filename="data.parquet"` or `filename="data.zip"` - For convenient file downloads
  - Chunked responses use 64KB chunks for optimal network efficiency

## Performance

### Multi-Threading

The SQL-on-FHIR implementation leverages multi-core processors for optimal performance through parallel resource processing:

- **Automatic Parallelization**: FHIR resources in bundles are processed in parallel using `rayon`
- **5-7x Performance Improvement**: Benchmarks show 5-7x speedup for typical workloads on multi-core systems
- **Zero Configuration**: Parallel processing is always enabled with intelligent work distribution
- **Thread Pool Control**: Optionally control thread count via `RAYON_NUM_THREADS` environment variable

```bash
# Use all available CPU cores (default)
sof-cli --view view.json --bundle large-bundle.json

# Limit to 4 threads for resource-constrained environments
RAYON_NUM_THREADS=4 sof-cli --view view.json --bundle large-bundle.json

# Server with custom thread pool
RAYON_NUM_THREADS=8 sof-server
```

#### Performance Benchmarks

Typical performance improvements with multi-threading:

| Bundle Size | Sequential Time | Parallel Time | Speedup |
|-------------|----------------|---------------|---------|
| 10 patients | 22.7ms | 8.3ms | 2.7x |
| 50 patients | 113.8ms | 16.1ms | 7.1x |
| 100 patients | 229.4ms | 35.7ms | 6.4x |
| 500 patients | 1109ms | 152ms | 7.3x |

The parallel processing ensures:
- Each FHIR resource is processed independently on available threads
- Column ordering is maintained consistently across parallel operations
- Thread-safe evaluation contexts for FHIRPath expressions
- Efficient load balancing through work-stealing algorithms

## Architecture

### Version-Agnostic Design

The crate uses trait abstractions to provide uniform processing across FHIR versions:

```rust
// Core traits for version independence
pub trait ViewDefinitionTrait {
    fn resource(&self) -> Option<&str>;
    fn select(&self) -> Option<&[Self::Select]>;
    fn where_clauses(&self) -> Option<&[Self::Where]>;
    fn constants(&self) -> Option<&[Self::Constant]>;
}

pub trait BundleTrait {
    type Resource: ResourceTrait;
    fn entries(&self) -> Vec<&Self::Resource>;
}
```

### Processing Pipeline

1. **Input Validation** - Verify ViewDefinition structure and FHIR version compatibility
2. **Constant Extraction** - Parse constants/variables for use in expressions
3. **Resource Filtering** - Apply where clauses to filter input resources
4. **Row Generation** - Process select statements with forEach support
5. **Output Formatting** - Convert to requested format (CSV, JSON, etc.)

### Error Handling

Comprehensive error types for different failure scenarios:

```rust
use helios_sof::SofError;

match run_view_definition(view, bundle, format) {
    Ok(output) => println!("Success: {} bytes", output.len()),
    Err(SofError::InvalidViewDefinition(msg)) => {
        eprintln!("ViewDefinition error: {}", msg);
    },
    Err(SofError::FhirPathError(msg)) => {
        eprintln!("FHIRPath evaluation error: {}", msg);
    },
    Err(SofError::UnsupportedContentType(format)) => {
        eprintln!("Unsupported format: {}", format);
    },
    Err(e) => eprintln!("Other error: {}", e),
}
```

## Feature Flags

Enable support for specific FHIR versions:

```toml
[dependencies]
sof = { version = "1.0", features = ["R4", "R5"] }

# Or enable all versions
sof = { version = "1.0", features = ["R4", "R4B", "R5", "R6"] }
```

Available features:
- `R4` - FHIR 4.0.1 support (default)
- `R4B` - FHIR 4.3.0 support  
- `R5` - FHIR 5.0.0 support
- `R6` - FHIR 6.0.0 support

## Integration Examples

### Batch Processing Pipeline

```rust
use helios_sof::{SofViewDefinition, SofBundle, ContentType, run_view_definition};
use helios_std::fs;

fn process_directory(view_path: &str, data_dir: &str) -> Result<(), Box<dyn std::error::Error>> {
    let view_def = fs::read_to_string(view_path)?;
    let view: fhir::r4::ViewDefinition = serde_json::from_str(&view_def)?;
    let sof_view = SofViewDefinition::R4(view);
    
    for entry in fs::read_dir(data_dir)? {
        let bundle_path = entry?.path();
        let bundle_json = fs::read_to_string(&bundle_path)?;
        let bundle: fhir::r4::Bundle = serde_json::from_str(&bundle_json)?;
        let sof_bundle = SofBundle::R4(bundle);
        
        let output = run_view_definition(
            sof_view.clone(),
            sof_bundle,
            ContentType::CsvWithHeader
        )?;
        
        let output_path = bundle_path.with_extension("csv");
        fs::write(output_path, output)?;
    }
    
    Ok(())
}
```

### Custom Error Handling

```rust
use helios_sof::{SofError, run_view_definition};

fn safe_transform(view: SofViewDefinition, bundle: SofBundle) -> Option<Vec<u8>> {
    match run_view_definition(view, bundle, ContentType::Json) {
        Ok(output) => Some(output),
        Err(SofError::InvalidViewDefinition(msg)) => {
            log::error!("ViewDefinition validation failed: {}", msg);
            None
        },
        Err(SofError::FhirPathError(msg)) => {
            log::warn!("FHIRPath evaluation issue: {}", msg);
            None
        },
        Err(e) => {
            log::error!("Unexpected error: {}", e);
            None
        }
    }
}
```

## Testing

The crate includes comprehensive tests covering:

- **ViewDefinition Validation** - Structure and logic validation
- **FHIRPath Integration** - Expression evaluation and error handling
- **Multi-Version Compatibility** - Cross-version processing
- **Output Format Validation** - Correct CSV, JSON, and NDJSON generation
- **Edge Cases** - Empty results, null values, complex nested structures
- **Query Parameter Validation** - Pagination, filtering, and format parameters
- **Error Handling** - Proper FHIR OperationOutcome responses for invalid parameters

Run tests with:

```bash
# All tests
cargo test

# Specific FHIR version
cargo test --features R5

# Integration tests only
cargo test --test integration
```




