# Helios Terminology Server (HTS)

A FHIR Terminology Server built in Rust, implementing the [HL7 FHIR Terminology Service](http://hl7.org/fhir/terminology-service.html) specification. HTS runs as a standalone binary and can be wired into any of the other Helios Software binaries via a single environment variable:

- [HFS](../hfs/README.md) - `HFS_TERMINOLOGY_SERVER` enables `:in`/`:not-in` search modifiers and FHIRPath `memberOf()`/`subsumes()` ([details](../hfs/README.md#configuration))
- [FHIRPath CLI and Server](../fhirpath/README.md) - `FHIRPATH_TERMINOLOGY_SERVER` powers terminology-aware FHIRPath evaluation ([details](../fhirpath/README.md#terminology-service-integration))
- [SOF CLI and Server](../sof/README.md) - `SOF_TERMINOLOGY_SERVER` enables FHIRPath terminology functions inside ViewDefinitions ([details](../sof/README.md#configuration))

It can also be used standalone as a general-purpose FHIR terminology service, independent of any other Helios Software component.

HTS supports both SQLite and PostgreSQL as database backends - see [Storage Backends](#storage-backends) for details.

### Terminology Data

HTS is a terminology engine - it does not include terminology data by default. Each terminology must be imported from its issuing authority, and you are responsible for obtaining a license and the source data before importing.

For terminologies where redistribution is permitted, the latest available data is included directly in each release build - no separate download needed.

## Features

### Terminology Operations

All six standard [FHIR Terminology Service](http://hl7.org/fhir/terminology-service.html) operations:

| Operation | Spec | Description |
|-----------|------|-------------|
| `$lookup` | [CodeSystem/$lookup](https://hl7.org/fhir/codesystem-operation-lookup.html) | Look up display name and properties for a code |
| `$validate-code` | [CodeSystem/$validate-code](https://hl7.org/fhir/codesystem-operation-validate-code.html) | Validate a code against a CodeSystem or ValueSet |
| `$subsumes` | [CodeSystem/$subsumes](https://hl7.org/fhir/codesystem-operation-subsumes.html) | Test concept hierarchy (subsumes / subsumed-by / equivalent / not-subsumed) via recursive CTE - no runtime graph traversal |
| `$expand` | [ValueSet/$expand](https://hl7.org/fhir/valueset-operation-expand.html) | Expand a ValueSet with lazy evaluation and materialized cache (computed once, cached across requests) |
| `$translate` | [ConceptMap/$translate](https://hl7.org/fhir/conceptmap-operation-translate.html) | Translate a code using a ConceptMap |
| `$closure` | [ConceptMap/$closure](https://hl7.org/fhir/conceptmap-operation-closure.html) | Compute transitive closure over a concept hierarchy and ConceptMap mappings |

- [CRUD and search](https://hl7.org/fhir/http.html) for CodeSystem, ValueSet, and ConceptMap resources
- [Batch](https://hl7.org/fhir/http.html#transaction) endpoint supporting `$validate-code` and `$translate` in a single request
- Implicit ValueSet expansion: when a CodeSystem's `valueSet` URL is requested and no explicit ValueSet exists, all codes in that system are returned ([FHIR R5 §4.8.7](https://hl7.org/fhir/codesystem.html#implicit))
- Dual `/metadata` response modes: `CapabilityStatement` (default) and `TerminologyCapabilities`
- Content negotiation (JSON / XML)
- CORS support

### Terminologies

| Terminology | Authority | Import | License / How to obtain |
|-------------|-----------|--------|--------------------------|
| [HL7 FHIR Core (THO)](https://terminology.hl7.org) | [HL7 International](https://www.hl7.org) | ✅ Bundled | Free - redistribution with attribution |
| [ICD-10-CM](https://www.cdc.gov/nchs/icd/icd-10-cm/index.html) | [U.S. CDC / NCHS](https://www.cdc.gov) | ✅ Bundled | Public domain (US federal government work) |
| [ICD-9-CM](https://www.cms.gov/medicare/coding-billing/icd-10-codes/icd-9-cm-diagnosis-procedure-codes-abbreviated-and-full-code-titles) | [U.S. NCHS / CMS](https://www.cms.gov) | ✅ Bundled | Public domain - retired 2015, legacy data only |
| [UCUM](https://ucum.org) | [Regenstrief Institute](https://www.regenstrief.org) | ✅ Bundled | Free, permissive - also in the THO package |
| [NCI Thesaurus (NCIt)](https://evs.nci.nih.gov) | [U.S. National Cancer Institute](https://www.cancer.gov) | ✅ Bundled | Public domain |
| [MeSH](https://www.nlm.nih.gov/mesh/) | [U.S. National Library of Medicine](https://www.nlm.nih.gov) | ✅ Bundled | Public domain |
| [DICOM](https://www.dicomstandard.org) | [NEMA](https://www.nema.org) | ✅ Bundled | Free, publicly available |
| [HL7 v2 tables](https://terminology.hl7.org) | [HL7 International](https://www.hl7.org) | ✅ Bundled | HL7 FHIR License (free with attribution) - also in the THO package |
| [NUCC Provider Taxonomy](https://www.nucc.org) | [NUCC](https://www.nucc.org) | ✅ Bundled | Free |
| [NDC](https://www.fda.gov/drugs/drug-approvals-and-databases/national-drug-code-directory) | [U.S. FDA](https://www.fda.gov) | ✅ Bundled | Public domain (US federal government work) |
| [SNOMED CT](https://www.snomed.org) | [SNOMED International](https://www.snomed.org) | 🔑 License required | Free in [~50 member countries](https://www.snomed.org/snomed-ct/get-snomed); paid elsewhere. [Register via MLDS](https://mlds.ihtsdotools.org/) or your [National Release Center](https://www.snomed.org/snomed-ct/get-snomed). US users: [nlm.nih.gov/healthit/snomedct](https://www.nlm.nih.gov/healthit/snomedct/index.html). |
| [LOINC](https://loinc.org) | [Regenstrief Institute](https://www.regenstrief.org) | 🔑 License required | Free - [create a free account at loinc.org](https://loinc.org/download/) to download. |
| [RxNorm](https://www.nlm.nih.gov/research/umls/rxnorm/overview.html) | [U.S. National Library of Medicine](https://www.nlm.nih.gov) | 🔑 License required | Free - [create a free UMLS account at uts.nlm.nih.gov](https://uts.nlm.nih.gov) and accept the [NLM Terms of Service](https://www.nlm.nih.gov/research/umls/rxnorm/docs/termsofservice.html). |
| [HCPCS Level II](https://www.cms.gov/medicare/coding-billing/healthcare-common-procedure-system) | [U.S. CMS](https://www.cms.gov) | 🚧 Not yet | Public domain (US gov). |
| [ICD-11](https://icd.who.int) | [WHO](https://www.who.int) | 🚧 Not yet | Free ([CC BY-ND 3.0 IGO](https://creativecommons.org/licenses/by-nd/3.0/igo/)). |
| [CPT](https://www.ama-assn.org/practice-management/cpt) | [AMA](https://www.ama-assn.org) | 🚧 Not yet | Proprietary - paid AMA license required. [Contact AMA for licensing](https://www.ama-assn.org/practice-management/cpt/cpt-licensing-frequently-asked-questions-faqs). |
| [MedDRA](https://www.meddra.org) | [MSSO](https://www.meddra.org) | 🚧 Not yet | Proprietary - paid MSSO license required. [Contact MedDRA](https://www.meddra.org). |

**Legend:** ✅ Bundled - no separate download needed. 🔑 License required - freely available, but registration or terms acceptance required. 🚧 Not yet - importer not yet implemented; open an issue.

- Bulk import CLI for all supported terminologies - see [Supported Terminologies](#supported-terminologies-1) for per-terminology instructions
- Automatic format detection - no `--format` flag needed for most files
- SQLite and PostgreSQL backends with auto-migration on startup (no manual schema setup)

## Quick Start

### Using Release Binaries

Pre-built binaries are available on the [GitHub Releases](https://github.com/HeliosSoftware/hfs/releases) page. Download the appropriate archive for your platform and extract it.

> **Windows users:** Add `.exe` to the binary name (e.g., `hts.exe`).

```bash
# Import a terminology package
./hts import ./hl7.terminology.r4-6.0.0.tgz

# Start the server (R4, SQLite, port 8090)
./hts run

# Verify
curl http://localhost:8090/health
curl http://localhost:8090/metadata
```

### Using Docker Images

Pre-built multi-arch Docker images (amd64/arm64) are available on GitHub Container Registry.

```bash
# Import a terminology package via Docker
docker run \
  -v hts-data:/data \
  -v $(pwd)/terminology:/terminology \
  ghcr.io/heliossoftware/hts:latest \
  import /terminology/hl7.terminology.r4-6.0.0.tgz

# With persistent SQLite storage
docker run -p 8090:8090 \
  -v hts-data:/data \
  -e HTS_DATABASE_URL=/data/hts.db \
  ghcr.io/heliossoftware/hts:latest
```

See [Environment Variables](#environment-variables) for all available configuration options.

### Building From Source

#### Prerequisites

1. **Install [Rust](https://www.rust-lang.org/tools/install)**
    ```bash
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
    ```

2. **Install [LLD](https://lld.llvm.org/)**

    Linux (Ubuntu/Debian):
    ```bash
    sudo apt install clang lld
    ```

    Windows:

      Download a pre-build binary from [llvm-project's GitHub page](https://github.com/llvm/llvm-project/releases).

    macOS:

      LLD is not required for macOS.

3. **Configure config.toml**

    Create or modify `~/.cargo/config.toml`:
    ```toml
    [target.x86_64-unknown-linux-gnu]
    linker = "clang"
    rustflags = ["-C", "link-arg=-fuse-ld=lld", "-C", "link-arg=-Wl,-zstack-size=8388608"]

    [target.aarch64-apple-darwin]
    linker = "clang"
    rustflags = [
      "-C", "link-arg=-Wl,-dead_strip",
      "-C", "link-arg=-undefined",
      "-C", "link-arg=dynamic_lookup"
    ]

    [target.x86_64-pc-windows-msvc]
    linker = "lld-link.exe"
    rustflags = ["-C", "link-arg=/STACK:8388608"]
    ```

4. **Memory-constrained builds** (optional):

    **Tip**: If you run out of memory during compilation on Linux, especially on high CPU core count machines, limit parallel jobs to 4 (or less):
    ```bash
    export CARGO_BUILD_JOBS=4
    ```

#### Build and Install

```bash
# Clone the repository
git clone https://github.com/HeliosSoftware/hfs.git
cd hfs

# Build with default features (R4 + SQLite)
cargo build --release -p helios-hts

# Build with all FHIR versions
cargo build --release -p helios-hts --features R4,R4B,R5,R6,sqlite

# Import terminologies of interest
./target/release/hts import ...

# Run from build output
./target/release/hts
```

## Usage

### `hts import`

Bulk-import a terminology package from the filesystem into the HTS database. Typically run before `hts run` so the server starts with terminology data already loaded.

If the target SQLite database does not yet exist, `hts import` creates the file (default: `./data/hts.db`) and applies the schema automatically - no prior `hts run` is required.

```bash
# HL7 FHIR NPM package (.tgz from https://terminology.hl7.org/en/downloads.html)
hts import ./hl7.terminology.r4-6.0.0.tgz

# SNOMED CT RF2 ZIP (requires NRC license)
hts import ./SnomedCT_InternationalRF2_*.zip --format snomed-rf2

# LOINC CSV ZIP (requires free registration at loinc.org)
hts import ./Loinc_*.zip --format loinc

# ICD-10-CM tabular XML (free, from cms.gov)
hts import ./icd10cm_tabular_2025.xml

# ICD-9-CM pipe-delimited text (free, from cms.gov)
hts import ./CMS32_DESC_LONG_DX.txt --format icd9-cm

# RxNorm RRF folder (requires free NLM terms-of-service)
hts import ./RxNorm_full_current/rrf/

# UCUM (free, from github.com/ucum-org/ucum/releases)
hts import ./ucum-essence.xml

# NCI Thesaurus (free, from evs.nci.nih.gov)
hts import ./Thesaurus.txt --format nci-thesaurus

# MeSH (free, from nlm.nih.gov)
hts import ./mesh2025.xml

# DICOM Part 16 code table (free, from dicomstandard.org - export as CSV)
hts import ./dicom-codes.csv --format dicom

# HL7 v2 tables XML (free with attribution)
hts import ./hl7-v2-tables.xml --format hl7-v2-tables

# NUCC Provider Taxonomy (free, from nucc.org)
hts import ./nucc_taxonomy_240.csv

# NDC Directory (free, public domain - from accessdata.fda.gov/cder/ndctext.zip)
hts import ./ndctext.zip

# Dry run - parse without writing to database
hts import ./package.tgz --dry-run --verbose
```

```
Usage: hts import [OPTIONS] <PATH>

Arguments:
  <PATH>  Path to the terminology package file or directory

Options:
      --format <FORMAT>            Terminology format (auto-detected when omitted)
                                   [possible values: hl7-npm, snomed-rf2, loinc, icd10-cm,
                                    icd9-cm, rxnorm, ndc, ucum, nci-thesaurus, mesh, dicom,
                                    hl7-v2-tables, nucc]
      --database-url <URL>         Database URL [env: HTS_DATABASE_URL=] [default: ./data/hts.db]
      --storage-backend <BACKEND>  Storage backend [env: HTS_STORAGE_BACKEND=] [default: sqlite]
      --log-level <LOG_LEVEL>      Log level [env: HTS_LOG_LEVEL=] [default: info]
      --batch-size <N>             Resources per import batch [default: 500]
      --dry-run                    Parse only - no database writes
      --verbose                    Emit per-batch progress to stderr
  -h, --help                       Print help
```

#### Format Auto-Detection

| Extension / pattern | Detected format |
|---------------------|-----------------|
| `.tgz` / `.tar.gz` | `hl7-npm` |
| `*tabular*.xml` | `icd10-cm` |
| `*ucum*.xml` or `*essence*.xml` | `ucum` |
| `*mesh*.xml` or `desc*.xml` | `mesh` |
| `*thesaurus*.txt` | `nci-thesaurus` |
| `*nucc*.csv` or `*taxonomy*.csv` | `nucc` |
| `.rrf` or directory | `rxnorm` |
| `.zip` containing RF2 files (`concept_full`, `description_full`) | `snomed-rf2` |
| `.zip` containing `LoincTable.csv` | `loinc` |
| `.zip` containing `RXNCONSO.RRF` | `rxnorm` |
| `.zip` containing `*tabular*.xml` | `icd10-cm` |
| `.zip` containing `*thesaurus*.txt` | `nci-thesaurus` |
| `.zip` containing `*ucum*.xml` | `ucum` |
| `.zip` containing `*dicom*.csv` or `*dcm*.csv` | `dicom` |
| `.zip` containing `*nucc*.csv` | `nucc` |
| `product.txt` (exact name) | `ndc` |
| file/path containing `ndctext` | `ndc` |
| `.zip` containing `product.txt` | `ndc` |

`.zip` files that match none of the above patterns require `--format`.

#### Exit Codes

| Code | Meaning |
|------|---------|
| `0` | Success - all resources imported |
| `1` | Fatal error - import aborted |
| `2` | Success with non-fatal errors - some records skipped |

### `hts run`

Start the FHIR Terminology HTTP server. This is the default command when no subcommand is given.

```bash
# Run with default settings (R4, SQLite, port 8090)
hts run

# Equivalent - run is the default
hts

# Specify a different port
hts run --port 9090

# Custom database path
hts run --database-url ./my-terminology.db

# Enable debug logging
hts run --log-level debug
```

If `hts import` has not been run first, HTS creates the SQLite file (or `./data/hts.db` by default) and applies the schema automatically on startup. No migrations or init scripts are required.

```
Usage: hts run [OPTIONS]

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
                                   [default: 3500]
  -h, --help                       Print help
```

## Configuration

### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `HTS_SERVER_PORT` | 8090 | Server port |
| `HTS_SERVER_HOST` | 127.0.0.1 | Host to bind |
| `HTS_LOG_LEVEL` | info | Log level (error, warn, info, debug, trace) |
| `HTS_DATABASE_URL` | ./data/hts.db | SQLite database file path |
| `HTS_STORAGE_BACKEND` | sqlite | Storage backend (`sqlite` or `postgres`) |
| `HTS_ENABLE_CORS` | true | Enable CORS |
| `HTS_CORS_ORIGINS` | * | Allowed CORS origins |
| `HTS_MAX_EXPANSION_SIZE` | 3500 | Maximum codes in a single ValueSet `$expand` response. Requests exceeding this limit return HTTP 422 with issue code `too-costly`. |

## Storage Backends

### SQLite (Default)

HTS uses SQLite with a 9-table normalized schema. The schema is applied automatically at startup using `CREATE TABLE IF NOT EXISTS`, so no separate migration step is needed.

```bash
# Default: file-based
hts run --database-url ./data/hts.db

# In-memory (useful for testing; data is lost on shutdown)
hts run --database-url :memory:
```

#### Schema

```
code_systems          - canonical CodeSystem metadata
concepts              - individual codes with display and definition
concept_hierarchy     - pre-materialized parent→child links (used by $subsumes)
concept_properties    - arbitrary FHIR properties per concept
concept_designations  - alternate names and translations per concept
value_sets            - canonical ValueSet metadata and compose rules
value_set_expansions  - materialized expansion cache (populated on first $expand)
concept_maps          - ConceptMap metadata
concept_map_mappings  - source→target code mappings with equivalence
```

The `value_set_expansions` table acts as a write-through cache: the first `$expand` call for a given ValueSet computes and stores the expansion; subsequent calls read from the cache directly. The cache is invalidated automatically when a CodeSystem or ValueSet is updated via PUT or DELETE.

### PostgreSQL

```bash
hts run --storage-backend postgres --database-url "postgresql://user:pass@localhost/hts"
```

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
| FHIR search `:in` modifier | `POST /ValueSet/$expand` - expands the ValueSet, then filters results |
| FHIR search `:not-in` modifier | `POST /ValueSet/$expand` - expands the ValueSet, then excludes matches |
| FHIRPath `memberOf()` | `POST /ValueSet/$validate-code` |
| FHIRPath `subsumes()` | `POST /CodeSystem/$subsumes` |

Without `HFS_TERMINOLOGY_SERVER`, these features fall back to empty results or `false`.

## Terminology Support

HTS is the engine - terminology data is not bundled. Each terminology has its own license, and you must obtain the data from its issuing authority before importing it.

> **Note:** HTS has no licensing cost. The data you load is governed by each terminology's own license - make sure you've accepted the relevant terms before importing.

---

### HL7 FHIR Core Terminology (THO)

Published by [HL7 International](https://www.hl7.org) under the [HL7 FHIR License](https://build.fhir.org/license.html). Free to use and redistribute with attribution. Includes all HL7-defined CodeSystems and ValueSets, HL7 v2/v3 vocabulary, CVX vaccine codes, and UCUM units.

```bash
hts import ./hl7.terminology.r4-6.0.0.tgz
```

Packages for R4, R4B, R5, and R6 are available at [terminology.hl7.org/en/downloads.html](https://terminology.hl7.org/en/downloads.html).

**Required attribution when redistributing:**
```
This product includes content from HL7 Terminology (THO).
Copyright © Health Level Seven International. Licensed under the HL7 FHIR License.
```

---

### ICD-10-CM

Produced by the [U.S. CDC / NCHS](https://www.cdc.gov/nchs/icd/icd-10-cm/index.html). A US federal government work - public domain, no license or registration required. Updated annually (effective October 1).

Download `icd10cm_tabular_YYYY.xml` from the [CDC ICD-10-CM files page](https://www.cdc.gov/nchs/icd/icd-10-cm/files.html) or the [CMS ICD-10 page](https://www.cms.gov/medicare/coding-billing/icd-10-codes).

```bash
hts import ./icd10cm_tabular_2026.xml
```

> **ICD-10-CM vs WHO ICD-10:** HTS imports ICD-10-CM (the US clinical modification, public domain). The WHO's ICD-10 is a separate, restricted publication.

---

### ICD-9-CM

Produced by the [U.S. NCHS / CMS](https://www.cms.gov/medicare/coding-billing/icd-10-codes/icd-9-cm-diagnosis-procedure-codes-abbreviated-and-full-code-titles). A US federal government work - public domain, no license or registration required. **Retired October 1, 2015** (replaced by ICD-10-CM); use for historical data and legacy EHR migration only.

Download the pipe-delimited code files (`CMS32_DESC_LONG_DX.txt` or similar) from the [CMS ICD-9-CM archive](https://www.cms.gov/medicare/coding-billing/icd-10-codes/icd-9-cm-diagnosis-procedure-codes-abbreviated-and-full-code-titles).

```bash
# From a raw pipe-delimited text file (--format required; .txt is ambiguous)
hts import ./CMS32_DESC_LONG_DX.txt --format icd9-cm

# From a ZIP containing the text file (auto-detected if file is named *_DESC_LONG_DX*.txt)
hts import ./ICD9CM_2015.zip --format icd9-cm
```

Codes are stored with decimal points (`001.0`, `E800.0`). Hierarchy is inferred from code structure - 3-digit categories are top-level, subcategories hang beneath them. No chapter groupers are imported (CMS flat files do not include them).

---

### SNOMED CT

Owned by [SNOMED International](https://www.snomed.org) and licensed under the [SNOMED Affiliate License](https://www.snomed.org/licensing). Free in the US and ~50 member countries; paid elsewhere.

**How to get it:**
- **United States:** Register at [nlm.nih.gov/healthit/snomedct](https://www.nlm.nih.gov/healthit/snomedct/index.html)
- **Other member countries:** Register via [MLDS](https://mlds.ihtsdotools.org/) or your [National Release Center](https://www.snomed.org/snomed-ct/get-snomed)

Download the **Snapshot** ZIP (not Full or Delta) - it contains the current state of all concepts without historical versions.

```bash
hts import ./SnomedCT_InternationalRF2_PRODUCTION_20250901T120000Z.zip --format snomed-rf2
```

For large imports, add `--batch-size 200 --verbose` to monitor progress.

---

### LOINC

Produced by the [Regenstrief Institute](https://www.regenstrief.org) under the [LOINC License](https://loinc.org/kb/license/). Free for commercial and non-commercial use; redistribution allowed with attribution. Registration at [loinc.org](https://loinc.org) is required to download.

```bash
hts import ./Loinc_2.80.zip   # format auto-detected from LoincTable.csv inside the ZIP
```

**Required attribution when redistributing:**
```
This material contains content from LOINC (http://loinc.org).
LOINC is copyright © Regenstrief Institute, Inc. and the Regenstrief LOINC Committee.
Terms of Use: https://loinc.org/license/
```

---

### RxNorm

Produced by the [U.S. National Library of Medicine (NLM)](https://www.nlm.nih.gov/research/umls/rxnorm/overview.html). Provides normalized names and identifiers for US drugs. A free UMLS account is required to download the full monthly release.

**Two options:**
- **Current Prescribable Content** - no account needed; smaller subset of actively prescribable drugs. Download from [nlm.nih.gov/research/umls/rxnorm/docs/rxnormfiles.html](https://www.nlm.nih.gov/research/umls/rxnorm/docs/rxnormfiles.html).
- **Full monthly release** - complete dataset including historical and branded content. Requires a free UMLS account at [uts.nlm.nih.gov](https://uts.nlm.nih.gov) and acceptance of the [NLM Terms of Service](https://www.nlm.nih.gov/research/umls/rxnorm/docs/termsofservice.html).

```bash
hts import ./RxNorm_full_current.zip        # from ZIP
hts import ./RxNorm_full_current/rrf/       # or from extracted RRF directory
```

**Required attribution:**
```
This product uses publicly available data courtesy of the U.S. National Library of Medicine (NLM),
National Institutes of Health, Department of Health and Human Services.
```

---

> **Note:** UCUM and HL7 v2 tables are already included in the HL7 THO NPM package. If you've already run `hts import <tgz>`, no separate import is needed for those two.
