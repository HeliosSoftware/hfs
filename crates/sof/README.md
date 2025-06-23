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
# Basic CSV output with headers
sof-cli --view patient-view.json --bundle patient-data.json --format csv --headers

# JSON output to file
sof-cli -v observation-view.json -b lab-results.json -f json -o output.json

# Use FHIR R5 and output NDJSON
sof-cli --view viewdef.json --bundle data.json --fhir-version R5 --format ndjson

# Read ViewDefinition from stdin, Bundle from file
cat complex-view.json | sof-cli --bundle large-dataset.json --format csv
```

#### CLI Features

- **Flexible Input**: Read ViewDefinitions and Bundles from files or stdin
- **FHIR Version Control**: Specify R4, R4B, R5, or R6 for proper parsing
- **Output Options**: Multiple formats with configurable CSV headers
- **Error Handling**: Clear, actionable error messages for debugging

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

# CSV output with headers
curl -X POST "http://localhost:8080/ViewDefinition/$run?_format=text/csv&_header=present" \
  -H "Content-Type: application/json" \
  -d '{...}'

# NDJSON output
curl -X POST http://localhost:8080/ViewDefinition/$run \
  -H "Content-Type: application/json" \
  -H "Accept: application/ndjson" \
  -d '{...}'
```

#### Parameters

The `$run` operation accepts a FHIR Parameters resource with:

- **viewResource**: The ViewDefinition to execute (inline)
- **viewReference**: Reference to a ViewDefinition (not yet supported)
- **patient**: Patient resources to transform
- **_format**: Override output format (query parameter)
- **_header**: Include CSV headers - "present" or "absent" (query parameter)

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

Run tests with:

```bash
# All tests
cargo test

# Specific FHIR version
cargo test --features R5

# Integration tests only
cargo test --test integration
```

