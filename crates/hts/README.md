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

## Terminology Support

HTS is a terminology-engine — it does not bundle terminology data. What you can load depends entirely on the license of each terminology. This section tells you exactly what you can do, what costs money, and how to obtain each one.

> For deep background on every terminology's license model, see [`crates/hts/terminology/terminology-analysis.md`](./terminology/terminology-analysis.md).

### How Terminology Licensing Works in FHIR

The [FHIR specification](https://build.fhir.org/license.html) is explicit on this:

> "The FHIR specification itself is licensed under CC0 — Creative Commons No Rights Reserved. But code systems used within FHIR often require separate licenses. SNOMED CT requires separate licensing from IHTSDO. LOINC is available at no cost under its own license. DICOM, ICD, CPT: each requires consultation with respective governing organizations."

The [HL7 Terminology (THO) license](https://terminology.hl7.org/license.html) reinforces it:

> "HL7 Terminology contains and references intellectual property owned by third parties ('Third Party IP'). Acceptance of these License Terms does not grant any rights with respect to Third Party IP. It is the sole responsibility of each organization deploying or testing this specification to ensure their implementations comply with licensing requirements of each Third Party IP."

**What this means for HTS:** HTS is open source — the engine has no licensing cost. But the *data* you load into HTS is governed by the license of each terminology. The rules are:
1. Do not load or redistribute data whose license you have not complied with.
2. Terminologies that require a license or registration must be obtained directly from the issuing authority.
3. Only the terminologies marked ✅ in the table below can be freely loaded and redistributed.

---

### Quick Reference

| Terminology | FHIR URI | License | Cost | Action required |
|-------------|----------|---------|------|-----------------|
| [HL7 FHIR Core (THO)](#hl7-fhir-core-terminology-tho) | `http://hl7.org/fhir/...` | HL7 FHIR License | Free | None — download & import |
| [ICD-10-CM](#icd-10-cm) | `http://hl7.org/fhir/sid/icd-10-cm` | Public domain (US gov) | Free | None — download & import |
| [CVX](#cvx-vaccine-administered-codes) | `http://hl7.org/fhir/sid/cvx` | Public domain (US gov) | Free | Included in THO — no extra step |
| [SNOMED CT](#snomed-ct) | `http://snomed.info/sct` | SNOMED Affiliate License | Free in US + 50 member countries | Register with your NRC |
| [LOINC](#loinc) | `http://loinc.org` | LOINC License | Free | Register free at loinc.org |
| [RxNorm](#rxnorm) | `http://www.nlm.nih.gov/research/umls/rxnorm` | NLM Terms of Service | Free | Create free UMLS account |
| [CPT](#cpt-current-procedural-terminology) | `http://www.ama-assn.org/go/cpt` | AMA proprietary | **Paid** | Not supported — see note |
| [HCPCS Level II](#hcpcs-level-ii) | `http://www.cms.gov/Medicare/Coding/HCPCSReleaseCodeSets` | Public domain (US gov) | Free | Import not yet implemented |
| [ICD-9-CM](#icd-9-cm) | `http://hl7.org/fhir/sid/icd-9-cm` | Public domain (US gov) | Free | Retired — import not yet implemented |
| [ICD-11](#icd-11) | `http://id.who.int/icd/release/11` | CC BY-ND 3.0 IGO | Free | Import not yet implemented — WHO adoption still early |
| [NDC](#ndc-national-drug-code) | `http://hl7.org/fhir/sid/ndc` | Public domain (US gov codes) | Free | Import not yet implemented |
| MedDRA | `http://www.meddra.org` | MSSO proprietary | **Paid** | Not supported |

---

### Freely Redistributable Terminologies

The following terminologies are public domain or permissively licensed. No registration or fee is required. You may load them into HTS and redistribute them with your own products.

#### HL7 FHIR Core Terminology (THO)

HL7 FHIR NPM packages are published by [HL7 International](https://www.hl7.org) at [terminology.hl7.org](https://terminology.hl7.org) under the [HL7 FHIR License](https://build.fhir.org/license.html), which permits free use and redistribution with attribution. The FHIR *specification* itself is [CC0](https://creativecommons.org/publicdomain/zero/1.0/) (no attribution required); the THO packages ask for attribution.

**Includes:**
- All HL7-defined CodeSystems and ValueSets (`http://hl7.org/fhir/...`, `http://terminology.hl7.org/...`)
- HL7 v2 and v3 vocabulary tables
- [CVX](#cvx-vaccine-administered-codes) — vaccine codes (CDC / public domain, bundled in THO)
- [UCUM](https://ucum.org) — units of measure (used in every FHIR `Quantity` field)

**Does NOT include** third-party terminologies such as SNOMED CT, LOINC, or CPT — even when referenced by HL7 value sets. Those retain their own licenses.

**Required attribution when redistributing THO packages:**
```
This product includes content from HL7 Terminology (THO).
Copyright © Health Level Seven International.
Licensed under the HL7 FHIR License.
```

**Steps:**
1. Download the NPM package from [terminology.hl7.org/en/downloads.html](https://terminology.hl7.org/en/downloads.html)
2. Import:

```bash
hts import ./hl7.terminology.r4-6.0.0.tgz
```

Packages are available for R4, R4B, R5, and R6. Use the package that matches your FHIR version.

---

#### ICD-10-CM

ICD-10-CM (International Classification of Diseases, 10th Revision, Clinical Modification) is produced by the [U.S. CDC / NCHS](https://www.cdc.gov/nchs/icd/icd-10-cm/index.html) and is a work of the U.S. federal government. Under [17 U.S.C. § 105](https://www.law.cornell.edu/uscode/text/17/105), federal government works are not subject to copyright and are in the **public domain**. No attribution, registration, or fee is required.

> **ICD-10-CM vs WHO ICD-10:** The WHO publishes its own ICD-10 (the international version) under a separate, restricted copyright. ICD-10-CM is the US clinical modification authored by a federal agency — these are distinct. HTS imports ICD-10-CM only.

ICD-10-CM is updated annually (effective October 1) with quarterly minor updates between annual releases. Download the current release from:
- [CDC ICD-10-CM files](https://www.cdc.gov/nchs/icd/icd-10-cm/files.html)
- [CMS ICD-10 page](https://www.cms.gov/medicare/coding-billing/icd-10-codes)

```bash
# Download icd10cm_tabular_YYYY.xml from the CDC page above
hts import ./icd10cm_tabular_2026.xml
```

**What about ICD-10-PCS?**

[ICD-10-PCS](https://www.cms.gov/medicare/coding-billing/icd-10-codes) (Procedure Coding System) is the companion system for inpatient procedure codes, also maintained by CMS and also a US government work in the public domain. It uses 7-character alphanumeric codes. HTS does not currently import ICD-10-PCS, but it could be added — open an issue if you need it.

---

#### CVX (Vaccine Administered Codes)

CVX is maintained by the [CDC / NCIRD](https://www.cdc.gov/iis/code-sets/index.html) and is a US government work in the public domain. It is also published as part of the [HL7 THO packages](https://terminology.hl7.org/CodeSystem-CVX.html) — **importing the HL7 THO package automatically brings in CVX**. No separate import is needed.

A companion code set, **MVX** (vaccine manufacturer codes), is included in THO as well.

Direct source: [CDC CVX table](https://www2a.cdc.gov/vaccines/iis/iisstandards/vaccines.asp?rpt=cvx)

---

### Terminologies Requiring Registration or a License

The following terminologies are **not included** in the HTS distribution. Each one is free (or free in most countries), but requires you to register directly with the issuing authority and accept their terms before downloading. You must obtain the data yourself, then import it into HTS.

#### SNOMED CT

SNOMED CT is owned by [SNOMED International](https://www.snomed.org) and governed by an [Affiliate License](https://www.snomed.org/licensing). Every user of SNOMED CT in a product or service must be a SNOMED Affiliate or a sub-licensee of one.

**Who needs a license?**

Anyone who uses SNOMED CT in a product or service must either:
- Be a SNOMED International Affiliate themselves, OR
- Be a sub-licensee of an Affiliate.

**Software vendors** that distribute SNOMED-enabled products must be Affiliates themselves and must issue sub-licenses to their customers. If you are building a product on top of HTS that incorporates SNOMED CT, you must obtain your own Affiliate License and ensure your end users obtain sub-licenses.

**Why we cannot redistribute SNOMED CT**

SNOMED CT's license model requires every user to have their own Affiliate License or sub-license. We cannot bundle the data in a public distribution because we cannot know whether each downloader of HTS is in a member country or has a valid license.

**Cost by territory:**

| Scenario | Cost |
|----------|------|
| **SNOMED International member country** (US, UK, Canada, Australia, Germany, and [~48 others](https://www.snomed.org/snomed-ct/get-snomed)) | **Free** |
| **Least-developed country** (World Bank LDC list) | **Free** |
| Qualifying research project (any country) | **Free** |
| **Non-member country** | **Paid** — calculated on use type and World Bank income classification |

**United States:** Free via the NLM UMLS program. Register at [nlm.nih.gov/healthit/snomedct](https://www.nlm.nih.gov/healthit/snomedct/index.html).

**All other countries:** Obtain your Affiliate License through the [SNOMED Member Licensing and Distribution Service (MLDS)](https://mlds.ihtsdotools.org/) or your country's National Release Center (NRC). Find your NRC at [snomed.org/get-snomed](https://www.snomed.org/snomed-ct/get-snomed).

**Which release to download — use Snapshot, not Full:**

| Release type | Description | Compressed size |
|---|---|---|
| **Snapshot** ✅ | Current state of every concept — no history | ~1–2 GB |
| Full | Complete history of all versions | ~2–3 GB (5–8 GB uncompressed) |
| Delta | Changes since the prior release only | < 100 MB |

For most HTS deployments, the Snapshot release is the right choice. Only download the Full release if you need historical concept states.

**Steps:**
1. Register and obtain a license via [MLDS](https://mlds.ihtsdotools.org/) or your country's NRC
2. Download the RF2 Snapshot ZIP
3. Import:

```bash
hts import ./SnomedCT_InternationalRF2_PRODUCTION_20250901T120000Z.zip --format snomed-rf2
```

Use `--batch-size 200 --verbose` to monitor progress and reduce peak memory usage on large imports.

---

#### LOINC

LOINC (Logical Observation Identifiers Names and Codes) is produced by the [Regenstrief Institute](https://www.regenstrief.org) and available under the [LOINC License](https://loinc.org/kb/license/). The license is **free** and permits use and redistribution with attribution. Registration at [loinc.org](https://loinc.org) is required to download directly from Regenstrief.

**What the LOINC License allows:**
- Free commercial and non-commercial use
- Incorporation into software products
- Redistribution with the following attribution:

```
This material contains content from LOINC (http://loinc.org).
LOINC is copyright © Regenstrief Institute, Inc. and the Regenstrief LOINC Committee.
Terms of Use: https://loinc.org/license/
```

**What the LOINC License prohibits:**
- Modifying LOINC core field names or code definitions (Group 1 artifacts)
- Creating a competing clinical observation standard using LOINC content
- Derivative works that alter LOINC codes or definitions without Regenstrief's written permission

**Why we do not bundle LOINC**

Redistribution with attribution *is* legally permitted by the LOINC License. We have chosen not to bundle LOINC for the following operational reasons — not because the license prohibits it:

1. **Currency:** LOINC is updated quarterly (March, June, September, December). A bundled copy would quickly become stale. We cannot commit to releasing HTS on the same cadence as Regenstrief. Sending users to download directly guarantees they get the latest version.
2. **Regenstrief tracking:** Regenstrief uses registrations to track the global deployment of LOINC. Redirecting users to register respects this intent.
3. **Attribution placement:** If we bundle, we must ensure the attribution string is visible to end users in the right place. Redirecting users to loinc.org is simpler.

**Note for users receiving LOINC through a redistribution:** The registration requirement at loinc.org applies to *downloading from Regenstrief's site*. A user who receives LOINC via a licensed redistributor (with proper attribution in place) is not additionally required to register at loinc.org to use it. Registration at loinc.org is only needed if they want to download future updates directly from Regenstrief.

**Steps:**
1. Create a free account at [loinc.org](https://loinc.org) and accept the LOINC License
2. Download the CSV ZIP from [loinc.org/downloads](https://loinc.org/downloads/)
3. Import (format auto-detected from `LoincTable.csv` inside the ZIP):

```bash
hts import ./Loinc_2.80.zip
```

4. Include the attribution string in your product documentation

---

#### RxNorm

RxNorm is produced by the [U.S. National Library of Medicine (NLM)](https://www.nlm.nih.gov/research/umls/rxnorm/overview.html). A free UMLS account is required to download the full monthly release.

**What RxNorm is:** RxNorm provides normalized names and unique identifiers (RXCUIs) for generic and branded drugs available in the US. It solves the "same drug, 20 names" problem across pharmacy systems. Created by NLM, released monthly.

**Two tiers:**
1. **Current Prescribable Content** — a subset containing only currently prescribable drugs. No license required for use. Available via the free [RxNorm API](https://lhncbc.nlm.nih.gov/RxNav/APIs/RxNormAPIs.html).
2. **Full monthly release** — includes historical, retired, and branded content. Requires a free UMLS account and acceptance of the NLM Terms of Service.

**Public domain status and Source Restriction Levels (SRLs)**

RxNorm is not a uniform dataset — it aggregates content from multiple sources at different restriction levels:

| SRL | Meaning | Redistribution |
|-----|---------|----------------|
| SRL 0 | NLM-created content (RXCUIs, normalized names) | **Public domain** — US government work |
| SRL 1 | Openly available sources | Redistributable per source terms |
| SRL 2 | Academic/legacy restricted sources (e.g., BI98, CPM2003) | Restricted per Section 12.2 of UMLS License Agreement |
| SRL 3 | Restricted commercial sources (e.g., Micromedex) | **NOT redistributable** |
| SRL 4 | Most restricted commercial sources (e.g., First DataBank) | **NOT redistributable** |

The full RxNorm download is an **interleaved mix** of all SRL levels. You cannot naively redistribute the full RxNorm dataset because SRL 2/3/4 content from academic and commercial sources is embedded in the same files. Separating SRL 0 content requires filtering by the `SAB` and `SUPPRESS` columns in the RRF files. This is why the full release requires a UMLS account and why we cannot bundle it.

**A no-registration alternative:** The [RxNorm Current Prescribable Content](https://www.nlm.nih.gov/research/umls/rxnorm/docs/rxnormfiles.html) subset contains only currently prescribable drugs (SRL 0/1 only) and requires no license or account. It is a smaller subset of the full release.

**Required attribution:**
```
This product uses publicly available data courtesy of the U.S. National Library of Medicine (NLM),
National Institutes of Health, Department of Health and Human Services.
NLM is not responsible for the product and does not endorse or recommend this or any other product.
```

**Steps:**
1. Create a free UMLS account at [uts.nlm.nih.gov](https://uts.nlm.nih.gov) and accept the [NLM Terms of Service](https://www.nlm.nih.gov/research/umls/rxnorm/docs/termsofservice.html)
2. Download the full monthly release (RRF format) from [nlm.nih.gov/research/umls/rxnorm/docs/rxnormfiles.html](https://www.nlm.nih.gov/research/umls/rxnorm/docs/rxnormfiles.html)
3. Import (format auto-detected from `RXNCONSO.RRF`):

```bash
# From ZIP
hts import ./RxNorm_full_current.zip

# Or from an extracted RRF directory
hts import ./RxNorm_full_current/rrf/
```

---

### Not Currently Supported

#### CPT (Current Procedural Terminology)

CPT is owned by the [American Medical Association (AMA)](https://www.ama-assn.org) and is **strictly proprietary**. Unlike every other terminology in this list, CPT is not a government work and is not freely available. Any use in a software product requires a paid distribution license with annual royalties.

- AMA licensing FAQ: [ama-assn.org — CPT Licensing FAQs](https://www.ama-assn.org/practice-management/cpt/cpt-licensing-frequently-asked-questions-faqs)
- Pricing: [Standard CPT Distribution Pricing Schedule](https://compliance.ama-assn.org/hc/en-us/articles/15253095972247)
- For AI products specifically: [Licensing CPT for AI FAQs](https://www.ama-assn.org/practice-management/cpt/licensing-cpt-ai-faqs)

> **Note:** HCPCS Level I is CPT. When a payer or regulatory document refers to "HCPCS Level I codes", they mean CPT codes under AMA copyright.

HTS does not currently support CPT import. If your use case requires CPT, contact AMA to obtain a distribution license, then open an issue — the importer itself is straightforward to add once licensing is confirmed.

---

#### HCPCS Level II

HCPCS Level II codes (letter A–V followed by 4 digits) are maintained by [CMS](https://www.cms.gov/medicare/coding-billing/healthcare-common-procedure-system) and are a US government work in the public domain. They cover durable medical equipment, prosthetics, ambulance services, drugs, and biologicals not represented in CPT.

> **D-codes caveat:** The dental D-codes within the HCPCS Level II file are derived from the ADA's Current Dental Terminology (CDT). CMS publishes them as part of its government document, but the ADA maintains a copyright claim over CDT content. This is an unresolved legal ambiguity: A–V codes are clearly public domain; D-codes are disputed. Review with counsel if D-codes are relevant to your use case.

Download quarterly from: [cms.gov — HCPCS Quarterly Update](https://www.cms.gov/medicare/coding-billing/healthcare-common-procedure-system/quarterly-update)

HTS import for HCPCS Level II is not yet implemented. Open an issue if you need it.

---

#### ICD-9-CM

ICD-9-CM was the US diagnosis code set before October 1, 2015, when it was replaced by ICD-10-CM. It is a US government work in the public domain. Its only remaining use case is historical data migration and longitudinal research spanning the pre/post-2015 transition.

HTS does not currently support ICD-9-CM import. If you need it, historical code files are available from the [CMS ICD-9-CM archive](https://www.cms.gov/medicare/coding-billing/icd-10-codes/icd-9-cm-diagnosis-procedure-codes-abbreviated-and-full-code-titles).

---

#### MedDRA

MedDRA (Medical Dictionary for Regulatory Activities) is maintained by the [MSSO](https://www.meddra.org) under ICH and requires a paid annual license. It is used primarily for adverse event reporting to the FDA and EMA. HTS does not support MedDRA import. Contact [meddra.org](https://www.meddra.org) for licensing.

---

### Additional Terminologies on the World List

The [FHIR Terminology Registry (tx.fhir.org/tx-reg)](https://tx.fhir.org/tx-reg) and HL7's [External Terminologies page](https://confluence.hl7.org/spaces/TA/pages/16646186/External+Terminologies+-+Information) list dozens of systems used globally. These are beyond the initial target list but relevant for completeness.

| Terminology | Authority | FHIR URI | License | Redistribute? | Notes |
|-------------|-----------|----------|---------|---------------|-------|
| **UCUM** | Regenstrief / HL7 | `http://unitsofmeasure.org` | Free, permissive | ✅ YES | Used for physical quantities in every FHIR `Quantity` field. Included in THO packages — no separate import needed. |
| **NCI Thesaurus (NCIt)** | NCI / NIH | `http://ncicb.nci.nih.gov/xml/owl/EVS/Thesaurus.owl` | Free, public domain | ✅ YES | ~170k biomedical concepts: anatomy, genes, drugs, diseases, and more. NCI is a US federal agency. |
| **MeSH** | NLM / NIH | `http://www.nlm.nih.gov/mesh` | Free, public domain | ✅ YES | Medical Subject Headings — NLM vocabulary used for PubMed indexing. |
| **DICOM** | NEMA | `http://dicom.nema.org/resources/ontology/DCM` | Free, publicly available | ✅ YES | Used in FHIR imaging resources (`ImagingStudy`, `ImagingSelection`). NEMA makes the DICOM standard freely available. |
| **HL7 v2 tables** | HL7 | `http://terminology.hl7.org/CodeSystem/v2-...` | HL7 FHIR License | ✅ YES (with attribution) | Included in HL7 THO NPM packages. |
| **NUCC** (Provider taxonomy) | NUCC | `http://nucc.org/provider-taxonomy` | Free | ✅ YES | Used in US provider directories and `Practitioner.qualification`. |
| **NDC** | FDA | `http://hl7.org/fhir/sid/ndc` | Public domain (codes) | ⚠️ CONDITIONAL | FDA publishes the NDC directory as a government work — the 11-digit codes themselves are public domain. However, the associated drug product data includes proprietary manufacturer submissions with potential trademark concerns. The codes are freely usable; bundling the full NDC product database requires care. Import not yet implemented in HTS. |
| **ICD-11** | WHO | `http://id.who.int/icd/release/11` | [CC BY-ND 3.0 IGO](https://creativecommons.org/licenses/by-nd/3.0/igo/) | ✅ YES — with attribution, no modifications | WHO published ICD-11 under a Creative Commons Attribution-NoDerivatives 3.0 IGO license. Redistribution with attribution is permitted; the ND clause prohibits modifications or derivative works. Download from [icd.who.int](https://icd.who.int). As of 2026, few production FHIR systems have adopted ICD-11 — real-world deployment is still early. Import not yet implemented in HTS. |
| **ICD-10** (WHO international) | WHO | `http://hl7.org/fhir/sid/icd-10` | Paid / restricted | ❌ NO | The WHO's original ICD-10 has its own separate copyright. Distinct from ICD-10-CM (the US modification). WHO charges for translated versions; the English version requires direct contact with WHO. US implementers use ICD-10-CM instead. |
| **MedDRA** | MSSO (ICH) | `http://www.meddra.org` | Paid | ❌ NO | Required for drug adverse event reporting (FDA, EMA). Annual license fee — contact [meddra.org](https://www.meddra.org) for current pricing. |
| **OMOP** vocabulary | OHDSI | varies | Mixed | Mixed | OHDSI vocabularies aggregate multiple sources — some open, some licensed. Redistribution depends on the individual source vocabulary. |
| **NDFRT** | NLM | `http://hl7.org/fhir/ndfrt` | Public domain | ✅ YES | Drug terminology maintained by NLM (US gov). Being retired in favor of RxNorm + NCI. |

**Key takeaway:** The terminologies that are both globally important and freely redistributable are mostly US federal government works (ICD-10-CM, CVX, NDC codes, NCI Thesaurus, MeSH, NDFRT), HL7/FHIR-native (THO, which includes UCUM, CVX, and HL7 v2/v3 tables), or under permissive WHO licensing (ICD-11). The globally dominant clinical terminologies that are **not** freely redistributable are SNOMED CT (Affiliate license required), LOINC (redistribution allowed with attribution but not bundled by us for operational reasons), RxNorm full release (UMLS account + non-redistributable SRL 3/4 content), CPT (paid AMA license), and MedDRA (paid MSSO license).

---

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
