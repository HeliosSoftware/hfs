# Helios Terminology Service (HTS)

A FHIR Terminology Service built in Rust, implementing the [HL7 FHIR Terminology Service](http://hl7.org/fhir/terminology-service.html) specification.

## Features

- FHIR Terminology operations (`$lookup`, `$validate-code`, `$expand`, `$subsumes`, `$translate`, `$closure`)
- CRUD for CodeSystem, ValueSet, and ConceptMap resources
- Search across terminology resources
- Batch/transaction bundle support
- Bulk import CLI for standard terminology distributions (SNOMED CT, LOINC, ICD-10-CM, RxNorm, HL7 NPM packages)
- Automatic format detection for import files
- SQLite storage backend (PostgreSQL planned)
- CORS support

## Installation

### From Source

```bash
# Clone the repository
git clone https://github.com/HeliosSoftware/hfs.git
cd hfs

# Build with default features (R4 + SQLite)
cargo build --release -p helios-hts

# Build with all FHIR versions
cargo build --release -p helios-hts --features R4,R4B,R5,R6,sqlite
```

## Usage

### Running the Server

```bash
# Run with default settings (R4, SQLite, port 8090)
./target/release/hts

# Explicit serve subcommand (equivalent to above)
./target/release/hts serve

# Specify a different port
./target/release/hts serve --port 9090

# Custom database path
./target/release/hts serve --database-url ./my-terminology.db

# Enable debug logging
./target/release/hts serve --log-level debug
```

### Command Line Options

```
Usage: hts [COMMAND]

Commands:
  serve   Run the FHIR Terminology HTTP server (default when no subcommand given)
  import  Bulk-import a terminology package from the filesystem

Options:
  -h, --help     Print help
  -V, --version  Print version
```

#### `hts serve`

```
Usage: hts serve [OPTIONS]

Options:
      --port <PORT>                Server port [env: HTS_SERVER_PORT=] [default: 8090]
      --host <HOST>                Host to bind [env: HTS_SERVER_HOST=] [default: 127.0.0.1]
      --log-level <LOG_LEVEL>      Log level (error, warn, info, debug, trace)
                                   [env: HTS_LOG_LEVEL=] [default: info]
      --database-url <URL>         Database URL [env: HTS_DATABASE_URL=] [default: ./data/hts.db]
      --storage-backend <BACKEND>  Storage backend [env: HTS_STORAGE_BACKEND=] [default: sqlite]
      --enable-cors                Enable CORS [env: HTS_ENABLE_CORS=] [default: true]
      --cors-origins <ORIGINS>     Allowed CORS origins [env: HTS_CORS_ORIGINS=] [default: *]
      --max-expansion-size <N>     Max codes in a ValueSet expansion [env: HTS_MAX_EXPANSION_SIZE=]
                                   [default: 10000]
  -h, --help                       Print help
```

#### `hts import`

```
Usage: hts import [OPTIONS] <PATH>

Arguments:
  <PATH>  Path to the terminology package file or directory

Options:
      --format <FORMAT>          Terminology format (auto-detected when omitted)
                                 [possible values: hl7-npm, snomed-rf2, loinc, icd10-cm, rxnorm]
      --database-url <URL>       SQLite database file [env: HTS_DATABASE_URL=] [default: ./data/hts.db]
      --log-level <LOG_LEVEL>    Log level [env: HTS_LOG_LEVEL=] [default: info]
      --batch-size <N>           Resources per import batch [default: 500]
      --dry-run                  Parse only — no database writes
      --verbose                  Emit per-batch progress to stderr
  -h, --help                     Print help
```

## Configuration

### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `HTS_SERVER_PORT` | 8090 | Server port |
| `HTS_SERVER_HOST` | 127.0.0.1 | Host to bind |
| `HTS_LOG_LEVEL` | info | Log level (error, warn, info, debug, trace) |
| `HTS_DATABASE_URL` | ./data/hts.db | SQLite database file path |
| `HTS_STORAGE_BACKEND` | sqlite | Storage backend (`sqlite`; `postgres` planned) |
| `HTS_ENABLE_CORS` | true | Enable CORS |
| `HTS_CORS_ORIGINS` | * | Allowed CORS origins |
| `HTS_MAX_EXPANSION_SIZE` | 10000 | Maximum codes in a single ValueSet `$expand` response |

## API Endpoints

### Terminology Operations

| Operation | Method | URL |
|-----------|--------|-----|
| $lookup (type) | GET/POST | `/CodeSystem/$lookup` |
| $lookup (instance) | GET/POST | `/CodeSystem/{id}/$lookup` |
| $validate-code (CodeSystem) | GET/POST | `/CodeSystem/$validate-code` |
| $subsumes | GET/POST | `/CodeSystem/$subsumes` |
| $expand (type) | GET/POST | `/ValueSet/$expand` |
| $expand (instance) | GET/POST | `/ValueSet/{id}/$expand` |
| $validate-code (ValueSet, type) | GET/POST | `/ValueSet/$validate-code` |
| $validate-code (ValueSet, instance) | GET/POST | `/ValueSet/{id}/$validate-code` |
| $translate (type) | GET/POST | `/ConceptMap/$translate` |
| $translate (instance) | GET/POST | `/ConceptMap/{id}/$translate` |
| $closure | POST | `/ConceptMap/$closure` |

### CRUD & Search

| Interaction | Method | URL |
|-------------|--------|-----|
| search | GET | `/CodeSystem`, `/ValueSet`, `/ConceptMap` |
| create | POST | `/CodeSystem`, `/ValueSet`, `/ConceptMap` |
| read | GET | `/CodeSystem/{id}`, `/ValueSet/{id}`, `/ConceptMap/{id}` |
| update | PUT | `/CodeSystem/{id}`, `/ValueSet/{id}`, `/ConceptMap/{id}` |
| delete | DELETE | `/CodeSystem/{id}`, `/ValueSet/{id}`, `/ConceptMap/{id}` |

### Utility

| Operation | Method | URL |
|-----------|--------|-----|
| health | GET | `/health` |
| capabilities | GET | `/metadata` |
| import bundle | POST | `/import` |
| batch/transaction | POST | `/` |

## Examples

### Import a Terminology Package

```bash
# HL7 FHIR NPM package (.tgz from https://terminology.hl7.org/en/downloads.html)
hts import ./hl7.terminology.r4-6.0.0.tgz

# SNOMED CT RF2 ZIP (requires NRC license)
hts import ./SnomedCT_InternationalRF2_*.zip --format snomed-rf2

# LOINC CSV ZIP (requires free registration at loinc.org)
hts import ./Loinc_*.zip --format loinc

# ICD-10-CM tabular XML (free, from cms.gov)
hts import ./icd10cm_tabular_2025.xml

# RxNorm RRF folder (requires free NLM terms-of-service)
hts import ./RxNorm_full_current/rrf/

# Dry run — parse without writing to database
hts import ./package.tgz --dry-run --verbose
```

#### Format Auto-Detection

| Extension / pattern | Detected format |
|---------------------|-----------------|
| `.tgz` / `.tar.gz` | `hl7-npm` |
| `*_tabular*.xml` | `icd10-cm` |
| `.rrf` or directory | `rxnorm` |
| `.zip` containing RF2 files | `snomed-rf2` |
| `.zip` containing `LoincTable.csv` | `loinc` |
| `.zip` containing `RXNCONSO.RRF` | `rxnorm` |

`.zip` files that cannot be auto-detected require `--format`.

#### Exit Codes

| Code | Meaning |
|------|---------|
| `0` | Success — all resources imported |
| `1` | Fatal error — import aborted |
| `2` | Success with non-fatal errors — some records skipped |

### Import a FHIR Bundle via HTTP

```bash
curl -X POST http://localhost:8090/import \
  -H "Content-Type: application/fhir+json" \
  -d @bundle.json
```

### Lookup a Concept

```bash
curl -X POST http://localhost:8090/CodeSystem/\$lookup \
  -H "Content-Type: application/fhir+json" \
  -d '{
    "resourceType": "Parameters",
    "parameter": [
      {"name": "url", "valueUri": "http://loinc.org"},
      {"name": "code", "valueCode": "718-7"}
    ]
  }'
```

### Validate a Code

```bash
curl -X POST http://localhost:8090/CodeSystem/\$validate-code \
  -H "Content-Type: application/fhir+json" \
  -d '{
    "resourceType": "Parameters",
    "parameter": [
      {"name": "url", "valueUri": "http://loinc.org"},
      {"name": "code", "valueCode": "718-7"}
    ]
  }'
```

### Expand a ValueSet

```bash
curl -X POST http://localhost:8090/ValueSet/\$expand \
  -H "Content-Type: application/fhir+json" \
  -d '{
    "resourceType": "Parameters",
    "parameter": [
      {"name": "url", "valueUri": "http://hl7.org/fhir/ValueSet/observation-codes"}
    ]
  }'
```

### Translate a Code

```bash
curl -X POST http://localhost:8090/ConceptMap/\$translate \
  -H "Content-Type: application/fhir+json" \
  -d '{
    "resourceType": "Parameters",
    "parameter": [
      {"name": "url", "valueUri": "http://example.org/fhir/ConceptMap/icd-to-snomed"},
      {"name": "code", "valueCode": "J06.9"},
      {"name": "system", "valueUri": "http://hl7.org/fhir/sid/icd-10"}
    ]
  }'
```

### Create a CodeSystem

```bash
curl -X POST http://localhost:8090/CodeSystem \
  -H "Content-Type: application/fhir+json" \
  -d '{
    "resourceType": "CodeSystem",
    "url": "http://example.org/cs/colors",
    "name": "Colors",
    "status": "active",
    "content": "complete",
    "concept": [
      {"code": "red", "display": "Red"},
      {"code": "blue", "display": "Blue"}
    ]
  }'
```

### Get CapabilityStatement

```bash
curl http://localhost:8090/metadata
```

## HFS Integration

Set `HFS_TERMINOLOGY_SERVER=http://localhost:8090` on the HFS process to enable:
- FHIR search `:in` and `:not-in` modifiers (expands ValueSet, filters by code)
- FHIRPath `memberOf()` / `subsumes()` delegation via `FHIRPATH_TERMINOLOGY_SERVER`

```bash
# Start HTS
HTS_DATABASE_URL=./data/hts.db cargo run --bin hts

# Start HFS with terminology integration
HFS_TERMINOLOGY_SERVER=http://localhost:8090 cargo run --bin hfs
```

## FHIR Version Support

Build with specific FHIR versions using feature flags:

```bash
# R4 only (default)
cargo build -p helios-hts --features R4,sqlite

# Multiple versions
cargo build -p helios-hts --features R4,R4B,R5,R6,sqlite
```
