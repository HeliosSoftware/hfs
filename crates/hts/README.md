# Helios Terminology Service (HTS)

A FHIR Terminology Service built in Rust, implementing the [HL7 FHIR Terminology Service](http://hl7.org/fhir/terminology-service.html) specification. HTS runs as a standalone binary and can be wired into any HFS instance via a single environment variable.

## Features

- All six standard FHIR terminology operations: `$lookup`, `$validate-code`, `$subsumes`, `$expand`, `$translate`, `$closure`
- CRUD and search for CodeSystem, ValueSet, and ConceptMap resources
- Batch endpoint supporting `$validate-code` and `$translate` in a single request
- Bulk import CLI for major terminology distributions: SNOMED CT RF2, LOINC, ICD-10-CM, RxNorm, HL7 FHIR NPM packages
- Automatic format detection — no `--format` flag needed for most files
- SQLite backend with auto-migration on startup (no manual schema setup)
- `$expand` with lazy evaluation and materialized cache: expansions are computed once and cached across requests
- `$subsumes` via recursive CTE over a pre-materialized hierarchy table — no runtime graph traversal
- `$closure` for transitive closure over concept hierarchy and ConceptMap mappings
- Implicit ValueSet expansion: when a CodeSystem's `valueSet` URL is requested and no explicit ValueSet exists, all codes in that system are returned (FHIR R5 §4.8.7)
- Dual `/metadata` response modes: `CapabilityStatement` (default) and `TerminologyCapabilities`
- Content negotiation (JSON / XML)
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

On first start HTS creates the SQLite file (or `./data/hts.db` by default) and applies the schema automatically. No migrations or init scripts are required.

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
| `HTS_MAX_EXPANSION_SIZE` | 10000 | Maximum codes in a single ValueSet `$expand` response. Requests exceeding this limit return HTTP 422 with issue code `too-costly`. |

## Storage

### SQLite (Default)

HTS uses SQLite with a 9-table normalized schema. The schema is applied automatically at startup using `CREATE TABLE IF NOT EXISTS`, so no separate migration step is needed.

```
code_systems          — canonical CodeSystem metadata
concepts              — individual codes with display and definition
concept_hierarchy     — pre-materialized parent→child links (used by $subsumes)
concept_properties    — arbitrary FHIR properties per concept
concept_designations  — alternate names and translations per concept
value_sets            — canonical ValueSet metadata and compose rules
value_set_expansions  — materialized expansion cache (populated on first $expand)
concept_maps          — ConceptMap metadata
concept_map_mappings  — source→target code mappings with equivalence
```

```bash
# Default: file-based
./target/release/hts serve --database-url ./data/hts.db

# In-memory (useful for testing; data is lost on shutdown)
./target/release/hts serve --database-url :memory:
```

The `value_set_expansions` table acts as a write-through cache: the first `$expand` call for a given ValueSet computes and stores the expansion; subsequent calls read from the cache directly. The cache is invalidated automatically when a CodeSystem or ValueSet is updated via PUT or DELETE.

When HTS runs alongside an HFS instance sharing the same SQLite file, the two sets of tables coexist in the same file. The HFS `resources` / `resource_history` tables and the HTS normalized tables do not overlap.

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
| batch | POST | `/` |

## Search

Search results are returned as a FHIR `Bundle` of type `searchset`. Five search parameters are supported for all three resource types:

| Parameter | Type | Description |
|-----------|------|-------------|
| `url` | uri | Canonical URL |
| `version` | token | Business version |
| `name` | string | Computer-friendly name |
| `title` | string | Human-friendly title |
| `status` | token | Publication status (`active`, `draft`, `retired`, `unknown`) |

Pagination is controlled by `_count` (page size, default 20) and `_offset` (zero-based start).

```bash
# Search by canonical URL
curl "http://localhost:8090/CodeSystem?url=http://loinc.org"

# Search by status with pagination
curl "http://localhost:8090/ValueSet?status=active&_count=10&_offset=0"
```

## Capabilities Endpoint

`GET /metadata` supports two response modes via the `mode` query parameter:

| Mode | Response type | Use when |
|------|--------------|----------|
| omitted or `mode=full` | `CapabilityStatement` | General REST capabilities discovery |
| `mode=terminology` | `TerminologyCapabilities` | Terminology-specific capabilities, lists supported CodeSystem URLs and expansion settings |

```bash
# Full CapabilityStatement (default)
curl http://localhost:8090/metadata

# TerminologyCapabilities
curl "http://localhost:8090/metadata?mode=terminology"
```

## Batch Support

`POST /` accepts a FHIR Bundle of type `batch` or `transaction` and returns a `batch-response` Bundle. The following operations are supported within a batch entry:

| Entry URL | Operation |
|-----------|-----------|
| `CodeSystem/$validate-code` | Validate a code against a CodeSystem |
| `ValueSet/$validate-code` | Validate a code against a ValueSet |
| `ConceptMap/$translate` | Translate a code using a ConceptMap |

Unsupported entry operations return a `400` entry-level `OperationOutcome` without failing the overall batch.

```bash
curl -X POST http://localhost:8090/ \
  -H "Content-Type: application/fhir+json" \
  -d '{
    "resourceType": "Bundle",
    "type": "batch",
    "entry": [
      {
        "request": { "method": "POST", "url": "CodeSystem/$validate-code" },
        "resource": {
          "resourceType": "Parameters",
          "parameter": [
            {"name": "url",  "valueUri":  "http://loinc.org"},
            {"name": "code", "valueCode": "718-7"}
          ]
        }
      },
      {
        "request": { "method": "POST", "url": "ValueSet/$validate-code" },
        "resource": {
          "resourceType": "Parameters",
          "parameter": [
            {"name": "url",  "valueUri":  "http://hl7.org/fhir/ValueSet/observation-codes"},
            {"name": "code", "valueCode": "718-7"}
          ]
        }
      }
    ]
  }'
```

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
| `*tabular*.xml` | `icd10-cm` |
| `.rrf` or directory | `rxnorm` |
| `.zip` containing RF2 files (`concept_full`, `description_full`) | `snomed-rf2` |
| `.zip` containing `LoincTable.csv` | `loinc` |
| `.zip` containing `RXNCONSO.RRF` | `rxnorm` |
| `.zip` containing `*tabular*.xml` | `icd10-cm` |

`.zip` files that match none of the above patterns require `--format`.

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
      {"name": "url",  "valueUri":  "http://loinc.org"},
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
      {"name": "url",  "valueUri":  "http://loinc.org"},
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

Pagination is supported via `count` and `offset` parameters:

```bash
curl -X POST http://localhost:8090/ValueSet/\$expand \
  -H "Content-Type: application/fhir+json" \
  -d '{
    "resourceType": "Parameters",
    "parameter": [
      {"name": "url",    "valueUri":    "http://hl7.org/fhir/ValueSet/observation-codes"},
      {"name": "count",  "valueInteger": 100},
      {"name": "offset", "valueInteger": 0}
    ]
  }'
```

### Check Concept Hierarchy

```bash
# Does 73211009 (Diabetes mellitus) subsume 44054006 (Type 2 diabetes)?
curl -X POST http://localhost:8090/CodeSystem/\$subsumes \
  -H "Content-Type: application/fhir+json" \
  -d '{
    "resourceType": "Parameters",
    "parameter": [
      {"name": "system",  "valueUri":  "http://snomed.info/sct"},
      {"name": "codeA",   "valueCode": "73211009"},
      {"name": "codeB",   "valueCode": "44054006"}
    ]
  }'
```

Returns one of: `equivalent`, `subsumes`, `subsumed-by`, or `not-subsumed`.

### Translate a Code

```bash
curl -X POST http://localhost:8090/ConceptMap/\$translate \
  -H "Content-Type: application/fhir+json" \
  -d '{
    "resourceType": "Parameters",
    "parameter": [
      {"name": "url",    "valueUri":  "http://example.org/fhir/ConceptMap/icd-to-snomed"},
      {"name": "code",   "valueCode": "J06.9"},
      {"name": "system", "valueUri":  "http://hl7.org/fhir/sid/icd-10"}
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
      {"code": "red",  "display": "Red"},
      {"code": "blue", "display": "Blue"}
    ]
  }'
```

PUT automatically re-indexes the new concept set into the normalized tables. DELETE cascades to all concept, hierarchy, property, and designation rows via SQL `ON DELETE CASCADE`.

### Get CapabilityStatement

```bash
curl http://localhost:8090/metadata
```

## HFS Integration

Set `HFS_TERMINOLOGY_SERVER` on the HFS process to delegate terminology operations to a running HTS instance:

```bash
# Start HTS
HTS_DATABASE_URL=./data/hts.db cargo run --bin hts

# Start HFS with HTS delegation
HFS_TERMINOLOGY_SERVER=http://localhost:8090 cargo run --bin hfs
```

HFS propagates the URL to its embedded FHIRPath engine as `FHIRPATH_TERMINOLOGY_SERVER`, enabling:

| Feature | Delegation |
|---------|-----------|
| FHIR search `:in` modifier | `POST /ValueSet/$expand` — expands the ValueSet, then filters results |
| FHIR search `:not-in` modifier | `POST /ValueSet/$expand` — expands the ValueSet, then excludes matches |
| FHIRPath `memberOf()` | `POST /ValueSet/$validate-code` |
| FHIRPath `subsumes()` | `POST /CodeSystem/$subsumes` |

Without `HFS_TERMINOLOGY_SERVER`, these features fall back to empty results or `false`.

## Included Terminologies

The following terminologies are freely redistributable and may be bundled with the HTS distribution.

### HL7 FHIR Core Terminology

HL7 FHIR NPM packages published at [terminology.hl7.org](https://terminology.hl7.org) are licensed under the [HL7 FHIR License](https://hl7.org/fhir/license.html), which permits free use and redistribution with attribution. These packages include the FHIR-defined CodeSystems and ValueSets (e.g. `http://hl7.org/fhir/...`, `http://terminology.hl7.org/...`) as well as HL7 v2 and v3 vocabulary.

To import the latest HL7 core terminology package:

```bash
# Download from https://terminology.hl7.org/en/downloads.html
hts import ./hl7.terminology.r4-6.0.0.tgz
```

### ICD-10-CM

ICD-10-CM (International Classification of Diseases, 10th Revision, Clinical Modification) is produced by the U.S. Centers for Disease Control and Prevention (CDC) and is a work of the U.S. federal government. As such it is in the **public domain** and may be freely used and redistributed without restriction.

Annual releases are published by the CDC at [https://www.cdc.gov/nchs/icd/icd-10-cm.htm](https://www.cdc.gov/nchs/icd/icd-10-cm.htm).

To import the tabular XML:

```bash
# Download icd10cm_tabular_YYYY.xml from the CDC page above
hts import ./icd10cm_tabular_2025.xml
```

## Terminologies Requiring a License

The following terminologies are **not included** in the HTS distribution because their licenses restrict redistribution. You must obtain your own license from the issuing authority before importing them into HTS.

### SNOMED CT

SNOMED CT is owned by [SNOMED International](https://www.snomed.org) and requires a license from your country's National Release Center (NRC). Licensing terms vary by country:

- **United States** — Available at no charge to US users through the NLM UMLS program: [https://www.nlm.nih.gov/healthit/snomedct/index.html](https://www.nlm.nih.gov/healthit/snomedct/index.html)
- **Other countries** — Check your country's NRC via the SNOMED International member list: [https://www.snomed.org/snomed-ct/get-snomed](https://www.snomed.org/snomed-ct/get-snomed)

Once licensed, download the RF2 snapshot release (`.zip`) from your NRC and import it:

```bash
hts import ./SnomedCT_InternationalRF2_PRODUCTION_20240901T120000Z.zip --format snomed-rf2
```

> **Note:** SNOMED CT RF2 full releases are large (several GB). Use `--batch-size 200` and `--verbose` to monitor progress and reduce peak memory usage.

### LOINC

LOINC (Logical Observation Identifiers Names and Codes) is owned by the [Regenstrief Institute](https://www.regenstrief.org) and is available free of charge under the [LOINC License](https://loinc.org/license/). The license permits use and redistribution with attribution but prohibits creating derivative works that modify the LOINC codes or definitions.

Registration (free) is required to download. Download the CSV distribution from [https://loinc.org/downloads/](https://loinc.org/downloads/).

```bash
# The .zip file contains LoincTable.csv — format is auto-detected
hts import ./Loinc_2.78.zip
```

### RxNorm

RxNorm is produced by the U.S. National Library of Medicine (NLM) and is available at no charge under the [NLM Terms of Service](https://www.nlm.nih.gov/research/umls/rxnorm/docs/termsofservice.html). The terms require acknowledgment of NLM as the source and prohibit using RxNorm data to compete with NLM's own products.

Download the full monthly release (RRF format) from [https://www.nlm.nih.gov/research/umls/rxnorm/docs/rxnormfiles.html](https://www.nlm.nih.gov/research/umls/rxnorm/docs/rxnormfiles.html).

```bash
# The .zip contains RXNCONSO.RRF — format is auto-detected
hts import ./RxNorm_full_current.zip

# Or point directly at an extracted RRF directory
hts import ./RxNorm_full_current/rrf/
```

## Docker

Using the generic workspace Dockerfile:

```bash
# Build HTS image
docker build --build-arg BINARY_NAME=hts -t hts .

# Run with a mounted data directory
docker run -p 8090:8090 \
  -v $(pwd)/data:/app/data \
  -e HTS_SERVER_HOST=0.0.0.0 \
  hts
```

## FHIR Version Support

Build with specific FHIR versions using feature flags:

```bash
# R4 only (default)
cargo build -p helios-hts

# R5 only
cargo build -p helios-hts --no-default-features --features R5,sqlite

# All versions
cargo build -p helios-hts --features R4,R4B,R5,R6,sqlite
```

The default feature set is `R4,sqlite`. The `TerminologyCapabilities` response is typed to the active FHIR version; on non-R4 builds without an explicit `#[cfg(feature)]` match, a minimal JSON fallback is returned.
