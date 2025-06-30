# SQL-on-FHIR Implementation

This crate provides a complete implementation of the SQL-on-FHIR specification for Rust, enabling the transformation of FHIR resources into tabular data using declarative ViewDefinitions. It supports all major FHIR versions (R4, R4B, R5, R6) through a version-agnostic abstraction layer.

## Overview

The `sof` crate implements the [HL7 FHIR SQL-on-FHIR Implementation Guide](https://build.fhir.org/ig/HL7/sql-on-fhir-v2/), providing:

- **ViewDefinition Processing** - Transform FHIR resources into tabular data using declarative configuration
- **Multi-Version Support** - Works seamlessly with R4, R4B, R5, and R6 FHIR specifications
- **FHIRPath Integration** - Complex data extraction using FHIRPath expressions
- **Multiple Output Formats** - CSV, JSON, NDJSON, and planned Parquet support
- **Command Line Interface** - Ready-to-use CLI tool for batch processing
- **Server Implementation** - HTTP API for on-demand transformations (planned)

## Executables

This crate provides two executable targets:

### `sof-cli` - Command Line Interface

A full-featured command-line tool for running ViewDefinition transformations:

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
```

#### CLI Features

- **Flexible Input**: Read ViewDefinitions and Bundles from files or stdin (but not both from stdin)
- **Output Formats**: CSV (with/without headers), JSON, and other supported formats
- **Output Options**: Write to stdout (default) or specified file with `-o`
- **FHIR Version Support**: R4 by default; other versions (R4B, R5, R6) require compilation with feature flags
- **Error Handling**: Clear, actionable error messages for debugging

#### Command Line Options

```
-v, --view <VIEW>              Path to ViewDefinition JSON file (or use stdin if not provided)
-b, --bundle <BUNDLE>          Path to FHIR Bundle JSON file (or use stdin if not provided)
-f, --format <FORMAT>          Output format [default: csv]
    --no-headers               Exclude CSV headers (only for CSV format)
-o, --output <OUTPUT>          Output file path (defaults to stdout)
    --fhir-version <VERSION>   FHIR version to use [default: R4] [possible values: R4*]
-h, --help                     Print help

* Additional FHIR versions (R4B, R5, R6) available when compiled with corresponding features
```

### `sof-server` - HTTP Server

A high-performance HTTP server providing SQL-on-FHIR ViewDefinition transformation capabilities:

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
- **Multi-format Output**: Support for CSV, JSON, and NDJSON responses
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

The `$run` operation accepts parameters either as query parameters or in a FHIR Parameters resource.
All parameters follow the SQL-on-FHIR specification order.

**Note on GET vs POST**: Per FHIR specification, GET operations cannot use complex datatypes. Therefore:
- **GET requests**: Can only use simple parameters (_format, header, _count, _page, _since)
- **POST requests**: Can use all parameters including complex types (viewReference, viewResource, patient, group, source, resource)

Parameter table:

| Name | Type | Use | Scope | Min | Max | Documentation |
|------|------|-----|-------|-----|-----|---------------|
| _format | code | in | type, instance | 1 | 1 | Output format - json, ndjson, csv, parquet |
| header | boolean | in | type, instance | 0 | 1 | This parameter only applies to `text/csv` requests. `true` (default) - return headers in the response, `false` - do not return headers. |
| viewReference | Reference | in | type, instance | 0 | * | Reference(s) to ViewDefinition(s) to be used for data transformation. (not yet supported) |
| viewResource | ViewDefinition | in | type | 0 | * | ViewDefinition(s) to be used for data transformation. |
| patient | Reference | in | type, instance | 0 | * | Filter resources by patient. |
| group | Reference | in | type, instance | 0 | * | Filter resources by group. (not yet supported) |
| source | string | in | type, instance | 0 | 1 | If provided, the source of FHIR data to be transformed into a tabular projection. (not yet supported) |
| _count | integer | in | type, instance | 0 | 1 | Limits the number of results, equivalent to the FHIR search `_count` parameter. (1-10000) |
| _page | integer | in | type, instance | 0 | 1 | Page number for paginated results, equivalent to the FHIR search `_page` parameter. (1-based) |
| _since | instant | in | type, instance | 0 | 1 | Return resources that have been modified after the supplied time. (RFC3339 format, validates format only) |
| resource | Resource | in | type, instance | 0 | * | Collection of FHIR resources to be transformed into a tabular projection. |

##### Query Parameters

All parameters except `viewReference`, `viewResource`, `patient`, `group`, `source` and `resource` can be provided as query parameters:

- **_format**: Output format (required if not in Accept header)
  - `application/json` or `json` - JSON array output (default)
  - `text/csv` or `csv` - CSV output
  - `application/ndjson` or `ndjson` - Newline-delimited JSON
- **header**: Control CSV headers (only applies to CSV format)
  - `true` - Include headers (default for CSV)
  - `false` - Exclude headers
- **_count**: Limit results (1-10000)
- **_page**: Page number (1-based)
- **_since**: Filter by modification time (RFC3339 format)

##### Body Parameters

For POST requests, parameters can be provided in a FHIR Parameters resource:

- **_format**: As valueCode or valueString (overrides query params and Accept header)
- **header**: As valueBoolean (overrides query params)
- **viewReference**: As valueReference (not yet supported)
- **viewResource**: As resource (inline ViewDefinition)
- **patient**: As valueReference
- **group**: As valueReference (not yet supported)
- **source**: As valueString (not yet supported)
- **_count**: As valueInteger
- **_page**: As valueInteger
- **_since**: As valueInstant
- **resource**: As resource (can be repeated)

##### Parameter Precedence

When the same parameter is specified in multiple places, the precedence order is:
1. Parameters in request body (highest priority)
2. Query parameters
3. Accept header (for format only, lowest priority)

##### Examples

```bash
# Paginated results - first 50 records as CSV
curl -X POST "http://localhost:8080/ViewDefinition/$run?_count=50&_page=1&_format=text/csv" \
  -H "Content-Type: application/json" \
  -d '{...}'

# Get page 3 of results (records 201-300)
curl -X POST "http://localhost:8080/ViewDefinition/$run?_count=100&_page=3" \
  -H "Content-Type: application/json" \
  -d '{...}'

# CSV without headers, limited to 20 results
curl -X POST "http://localhost:8080/ViewDefinition/$run?_format=text/csv&header=false&_count=20" \
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
```

##### GET Endpoint - Limited by FHIR Specification

**Important**: Per the FHIR specification, GET operations can only use simple input parameters. Complex datatypes like Reference, Identifier, and Resource cannot be passed as URL parameters.

The GET endpoint exists but has severe limitations:

```bash
# GET /ViewDefinition/$run - WILL FAIL due to FHIR constraints
# Cannot use viewReference, patient, group parameters in GET
curl "http://localhost:8080/ViewDefinition/$run?_format=json&_count=50"
```

**Supported GET parameters** (simple types only):
- `_format` - Output format
- `header` - CSV header control
- `source` - Data source (not implemented)
- `_count` - Result limit
- `_page` - Page number
- `_since` - Modification time filter

**Unsupported GET parameters** (complex types - use POST instead):
- `viewReference` - Reference type (not allowed in GET)
- `patient` - Reference type (not allowed in GET)
- `group` - Reference type (not allowed in GET)
- `viewResource` - Resource type (only in POST body)
- `resource` - Resource type (only in POST body)

**Recommendation**: Always use POST for the $run operation as it supports all parameters.

## Core Features

### ViewDefinition Processing

Transform FHIR resources using declarative ViewDefinitions:

```rust
use sof::{SofViewDefinition, SofBundle, ContentType, run_view_definition};

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
use sof::ContentType;

// CSV without headers
let csv = run_view_definition(view, bundle, ContentType::Csv)?;

// CSV with headers  
let csv_headers = run_view_definition(view, bundle, ContentType::CsvWithHeader)?;

// Pretty-printed JSON array
let json = run_view_definition(view, bundle, ContentType::Json)?;

// Newline-delimited JSON (streaming friendly)
let ndjson = run_view_definition(view, bundle, ContentType::NdJson)?;

// Apache Parquet (planned)
let parquet = run_view_definition(view, bundle, ContentType::Parquet)?;
```

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
use sof::SofError;

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
use sof::{SofViewDefinition, SofBundle, ContentType, run_view_definition};
use std::fs;

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
use sof::{SofError, run_view_definition};

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

