# Helios FHIR Server (HFS)

A high-performance FHIR server built in Rust.

An open test server is available at https://hfs.heliossoftware.com/ for experimentation and evaluation.

## Features

- Full FHIR RESTful API support 
- Multiple FHIR version support
- Pluggable storage backends (SQLite, PostgreSQL, MongoDB)
- **Bulk Data Export (`$export`)** — system / patient / group, asynchronous,
  with an embedded single-instance default and a multi-instance
  PostgreSQL + S3 topology (see *Bulk Data Export* below)
- Content negotiation (JSON)
- Conditional operations with ETag support
- Multi-tenant support via X-Tenant-ID header
- CORS support

## Bulk Data Export

HFS implements the [FHIR Bulk Data Access IG](https://build.fhir.org/ig/HL7/bulk-data/)
`$export` operation. Single-instance (zero config) wires SQLite job state +
local-FS output + an in-process worker pool; multi-instance switches to a
PostgreSQL job store and an S3-compatible output store with pre-signed
download URLs.

```bash
# Single-instance (default)
cargo run --bin hfs
curl -H 'Prefer: respond-async' \
  http://localhost:8080/Patient/\$export
```

The full configuration surface (`HFS_BULK_EXPORT_*` env vars, single- vs
multi-instance recipes, parameter behavior) is documented in `CLAUDE.md`.
A docker-compose stack for the multi-instance topology lives at
`docker/bulk-export/docker-compose.yml`, and a manual Inferno Bulk Data IG
v2.0.0 conformance workflow at `.github/workflows/inferno-bulk-data.yml`.

## Installation

### From Source

```bash
# Clone the repository
git clone https://github.com/HeliosSoftware/hfs.git
cd hfs

# Build with default features (R4 + SQLite)
cargo build --release -p helios-hfs

# Build with all FHIR versions
cargo build --release -p helios-hfs --features R4,R4B,R5,R6,sqlite
```

## Usage

### Running the Server

```bash
# Run with default settings (R4, SQLite, port 8080)
./target/release/hfs

# Specify a different port
./target/release/hfs --port 3000

# Use an in-memory database
./target/release/hfs --database-url :memory:

# Enable debug logging
./target/release/hfs --log-level debug
```

### Command Line Options

```
Usage: hfs [OPTIONS]

Options:
      --port <PORT>              Server port [env: HFS_SERVER_PORT=] [default: 8080]
      --host <HOST>              Host to bind [env: HFS_SERVER_HOST=] [default: 127.0.0.1]
      --log-level <LOG_LEVEL>    Log level (error, warn, info, debug, trace)
                                 [env: HFS_LOG_LEVEL=] [default: info]
      --database-url <URL>       Database connection URL [env: DATABASE_URL=]
      --data-dir <PATH>          Path to FHIR data directory containing search parameter
                                 definitions [env: HFS_DATA_DIR=] [default: ./data]
      --max-body-size <BYTES>    Maximum request body size [env: HFS_MAX_BODY_SIZE=]
                                 [default: 10485760]
      --request-timeout <SECS>   Request timeout in seconds [env: HFS_REQUEST_TIMEOUT=]
                                 [default: 30]
      --enable-cors              Enable CORS [env: HFS_ENABLE_CORS=] [default: true]
      --cors-origins <ORIGINS>   Allowed CORS origins [env: HFS_CORS_ORIGINS=] [default: *]
  -h, --help                     Print help
  -V, --version                  Print version
```

## Configuration

### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `HFS_SERVER_PORT` | 8080 | Server port |
| `HFS_SERVER_HOST` | 127.0.0.1 | Host to bind |
| `HFS_LOG_LEVEL` | info | Log level (error, warn, info, debug, trace) |
| `DATABASE_URL` | fhir.db | Database connection string |
| `HFS_DATA_DIR` | ./data | Path to FHIR data directory (search parameters) |
| `HFS_MAX_BODY_SIZE` | 10485760 | Max request body size (bytes) |
| `HFS_REQUEST_TIMEOUT` | 30 | Request timeout (seconds) |
| `HFS_ENABLE_CORS` | true | Enable CORS |
| `HFS_CORS_ORIGINS` | * | Allowed CORS origins |
| `HFS_CORS_METHODS` | GET,POST,PUT,DELETE,OPTIONS | Allowed HTTP methods |
| `HFS_CORS_HEADERS` | Content-Type,Authorization,X-Requested-With | Allowed headers |
| `HFS_DEFAULT_TENANT` | default | Default tenant ID |
| `HFS_TERMINOLOGY_SERVER` | (none) | Terminology server URL for `:in`/`:not-in` modifiers and FHIRPath `memberOf()`/`subsumes()` |

## FHIR Version Support

Build with specific FHIR versions using feature flags:

```bash
# R4 only (default)
cargo build -p helios-hfs --features R4,sqlite

# R5 only
cargo build -p helios-hfs --no-default-features --features R5,sqlite

# Multiple versions
cargo build -p helios-hfs --features R4,R4B,R5,R6,sqlite
```

## Database Backends

### SQLite (Default)

```bash
cargo build -p helios-hfs --features sqlite

# Run with file-based database
./target/release/hfs --database-url ./data/fhir.db

# Run with in-memory database
./target/release/hfs --database-url :memory:
```

### PostgreSQL

```bash
cargo build -p helios-hfs --no-default-features --features R4,postgres

./target/release/hfs --database-url "postgresql://user:pass@localhost/fhir"
```

### MongoDB

```bash
cargo build -p helios-hfs --no-default-features --features R4,mongodb

./target/release/hfs --database-url "mongodb://localhost:27017/fhir"
```

## API Endpoints

| Interaction | Method | URL |
|------------|--------|-----|
| capabilities | GET | `/metadata` |
| read | GET | `/[type]/[id]` |
| vread | GET | `/[type]/[id]/_history/[vid]` |
| update | PUT | `/[type]/[id]` |
| patch | PATCH | `/[type]/[id]` |
| delete | DELETE | `/[type]/[id]` |
| create | POST | `/[type]` |
| search | GET/POST | `/[type]?params` or `/[type]/_search` |
| history (instance) | GET | `/[type]/[id]/_history` |
| history (type) | GET | `/[type]/_history` |
| history (system) | GET | `/_history` |
| batch/transaction | POST | `/` |
| health | GET | `/health` |

## Examples

### Create a Patient

```bash
curl -X POST http://localhost:8080/Patient \
  -H "Content-Type: application/fhir+json" \
  -d '{
    "resourceType": "Patient",
    "name": [{"family": "Smith", "given": ["John"]}],
    "birthDate": "1970-01-01"
  }'
```

### Read a Patient

```bash
curl http://localhost:8080/Patient/123
```

### Search for Patients

```bash
curl "http://localhost:8080/Patient?family=Smith"
```

### Get CapabilityStatement

```bash
curl http://localhost:8080/metadata
```

## Batch and Transaction Support

HFS supports both batch and transaction bundles for processing multiple operations in a single request.

> **Note:** Transaction bundles require ACID transaction support. The default SQLite backend fully supports transactions. If using other backends, check the capability matrix in the persistence crate documentation.

### Transaction Bundle (Atomic)

All operations succeed or all fail. Entries are processed in FHIR-specified order: DELETE → POST → PUT → GET.

```bash
curl -X POST http://localhost:8080/ \
  -H "Content-Type: application/fhir+json" \
  -d '{
    "resourceType": "Bundle",
    "type": "transaction",
    "entry": [
      {
        "fullUrl": "urn:uuid:patient-1",
        "resource": {
          "resourceType": "Patient",
          "name": [{"family": "Smith"}]
        },
        "request": {
          "method": "POST",
          "url": "Patient"
        }
      },
      {
        "resource": {
          "resourceType": "Observation",
          "subject": {"reference": "urn:uuid:patient-1"},
          "code": {"text": "Blood Pressure"}
        },
        "request": {
          "method": "POST",
          "url": "Observation"
        }
      }
    ]
  }'
```

The `urn:uuid:patient-1` reference is automatically resolved to the actual Patient ID after creation.

### Batch Bundle (Independent)

Each operation is processed independently; failures don't affect other entries.

```bash
curl -X POST http://localhost:8080/ \
  -H "Content-Type: application/fhir+json" \
  -d '{
    "resourceType": "Bundle",
    "type": "batch",
    "entry": [
      {
        "request": {
          "method": "GET",
          "url": "Patient/123"
        }
      },
      {
        "request": {
          "method": "DELETE",
          "url": "Patient/456"
        }
      }
    ]
  }'
```

### Current Limitations

The following FHIR bundle features are not yet implemented:
- Conditional reference resolution (`Patient?identifier=12345`)
- PATCH method in bundles
- Prefer header handling (`return=minimal`, etc.)

## Search Parameter Configuration

HFS loads FHIR SearchParameter definitions from JSON bundle files to enable comprehensive search functionality. By default, these files are expected in a `data/` directory relative to the working directory or executable.

### Data Directory Structure

```
data/
├── search-parameters-r4.json   # FHIR R4 SearchParameters (HL7 spec)
├── search-parameters-r4b.json  # FHIR R4B SearchParameters (HL7 spec)
├── search-parameters-r5.json   # FHIR R5 SearchParameters (HL7 spec)
├── search-parameters-r6.json   # FHIR R6 SearchParameters (auto-downloaded at build time)
└── *.json                      # Custom SearchParameter files (see below)
```

### Search Parameter Loading

On startup, HFS loads SearchParameters in this order:
1. **Minimal fallback** - Built-in `_id`, `_lastUpdated`, `_tag`, `_profile`, `_security` (always available)
2. **Spec file** - Loads from the appropriate `search-parameters-*.json` based on configured FHIR version
3. **Custom files** - Loads any additional `.json` files in the data directory (not matching `search-parameters-*.json`)
4. **Stored parameters** - Loads any custom SearchParameters POSTed to the server

### Custom SearchParameter Files

You can add custom SearchParameters by placing JSON files in the data directory. Each file can contain:
- A single SearchParameter resource
- An array of SearchParameter resources
- A FHIR Bundle containing SearchParameter resources

Example custom SearchParameter file (`data/custom-search-params.json`):
```json
{
  "resourceType": "SearchParameter",
  "id": "patient-mrn",
  "url": "http://example.org/fhir/SearchParameter/patient-mrn",
  "name": "mrn",
  "status": "active",
  "code": "mrn",
  "base": ["Patient"],
  "type": "token",
  "expression": "Patient.identifier.where(type.coding.code='MR')"
}
```

Or as a Bundle:
```json
{
  "resourceType": "Bundle",
  "type": "collection",
  "entry": [
    {
      "resource": {
        "resourceType": "SearchParameter",
        "url": "http://example.org/fhir/SearchParameter/patient-mrn",
        ...
      }
    }
  ]
}
```

### Custom Data Directory

Specify a custom location for the data files:

```bash
# Via command line
./target/release/hfs --data-dir /opt/hfs/data

# Via environment variable
HFS_DATA_DIR=/opt/hfs/data ./target/release/hfs
```

If the spec file is missing, HFS logs a warning and continues with minimal fallback parameters. This ensures the server can start even without the full spec files, though search functionality will be limited.

### R6 Automatic Download

When building with the R6 feature enabled, the `search-parameters-r6.json` file is automatically downloaded from the HL7 build server during compilation. The download is skipped if:
- The file already exists and is less than 24 hours old
- The `DOCS_RS` environment variable is set (for docs.rs builds)

## Multi-Tenant Support

Use the `X-Tenant-ID` header to isolate data between tenants:

```bash
curl -H "X-Tenant-ID: clinic-a" http://localhost:8080/Patient
curl -H "X-Tenant-ID: clinic-b" http://localhost:8080/Patient
```

