---
name: work-with-hts
description: Work on the HFS Terminology Server. Use for hts runtime configuration, HTS APIs, terminology import, bootstrap sync, CodeSystem, ValueSet, ConceptMap, SNOMED, LOINC, RxNorm, ICD-10-CM, and HFS terminology integration.
---

# HTS

Use this when working on the `hts` binary or terminology server behavior.

## Running

```bash
# Default: SQLite, port 8090
cargo run --bin hts

# Custom database path and port
HTS_DATABASE_URL=./my-terminology.db HTS_SERVER_PORT=9090 cargo run --bin hts
```

## Environment

| Variable | Default | Description |
|---|---|---|
| `HTS_SERVER_PORT` | `8090` | Server port |
| `HTS_SERVER_HOST` | `127.0.0.1` | Host to bind |
| `HTS_LOG_LEVEL` | `info` | Log level: error, warn, info, debug, trace |
| `HTS_DATABASE_URL` | `./data/hts.db` | SQLite file path or PostgreSQL connection URL |
| `HTS_STORAGE_BACKEND` | `sqlite` | Storage backend; postgres when built with `--features postgres` |
| `HTS_BOOTSTRAP_DIR` | none | Directory of terminology files imported on startup |
| `HTS_MAX_BODY_SIZE` | `10485760` | Max request body size in bytes, applied after decompression |
| `HTS_ENABLE_CORS` | `true` | Enable CORS |
| `HTS_CORS_ORIGINS` | `*` | Allowed CORS origins |

## Bootstrap Directory Sync

When `HTS_BOOTSTRAP_DIR` points at a directory, HTS synchronizes recognized files into the database on every startup. Each recognized file is hashed with SHA-256 and recorded in a `bootstrap_imports` ledger keyed by file name. A file imports only when the hash is absent or changed.

- Adding a new file imports it on the next restart.
- Replacing a file with a newer terminology release re-imports it. Imports upsert on canonical `(url, version)`, so a new version coexists with the old and a same-version re-import refreshes in place.
- Unchanged files are skipped without reparsing.
- RxNorm RRF directory signatures are derived from immediate children's `(name, size)` instead of full content hashing to avoid rereading multi-gigabyte folders on every boot.

## API Endpoints

| Operation | Method | URL |
|---|---|---|
| health | GET | `/health` |
| capabilities | GET | `/metadata` |
| lookup | POST | `/CodeSystem/$lookup` |
| validate code, CodeSystem | POST | `/CodeSystem/$validate-code` |
| subsumes | POST | `/CodeSystem/$subsumes` |
| expand | POST | `/ValueSet/$expand` |
| validate code, ValueSet | POST | `/ValueSet/$validate-code` |
| translate | POST | `/ConceptMap/$translate` |
| closure | POST | `/ConceptMap/$closure` |
| import bundle | POST | `/import` |
| CRUD | GET/POST/PUT/DELETE | `/CodeSystem/:id`, `/ValueSet/:id`, `/ConceptMap/:id` |

## Quick Examples

```bash
# Import a FHIR Bundle containing CodeSystem, ValueSet, or ConceptMap resources
curl -X POST http://localhost:8090/import \
  -H "Content-Type: application/fhir+json" \
  -d @bundle.json

# Lookup a concept
curl -X POST http://localhost:8090/CodeSystem/\$lookup \
  -H "Content-Type: application/fhir+json" \
  -d '{"resourceType":"Parameters","parameter":[{"name":"url","valueUri":"http://example.org/cs"},{"name":"code","valueCode":"ABC"}]}'

# Expand a value set
curl -X POST http://localhost:8090/ValueSet/\$expand \
  -H "Content-Type: application/fhir+json" \
  -d '{"resourceType":"Parameters","parameter":[{"name":"url","valueUri":"http://example.org/vs"}]}'
```

## HFS Integration

Set `HFS_TERMINOLOGY_SERVER=http://localhost:8090` on HFS to enable:

- FHIR search `:in` modifier, using ValueSet expansion to filter by code.
- FHIRPath `memberOf()` and `subsumes()` delegation through `FHIRPATH_TERMINOLOGY_SERVER`.

## Bulk Import CLI

Load terminology packages directly from the filesystem:

```bash
# HL7 FHIR NPM package
cargo run --bin hts -- import ./hl7.terminology.r4-6.0.0.tgz

# SNOMED CT RF2 ZIP, requires NRC license
cargo run --bin hts -- import ./SnomedCT_InternationalRF2_*.zip --format snomed-rf2

# LOINC CSV ZIP, requires free Regenstrief registration
cargo run --bin hts -- import ./Loinc_*.zip --format loinc

# ICD-10-CM tabular XML
cargo run --bin hts -- import ./icd10cm_tabular_2025.xml

# RxNorm RRF folder, requires free NLM terms of service
cargo run --bin hts -- import ./RxNorm_full_current/rrf/

# Common flags
cargo run --bin hts -- import ./package.tgz \
  --database-url ./data/hts.db \
  --batch-size 1000 \
  --dry-run \
  --verbose
```

LOINC language translations under `AccessoryFiles/LinguisticVariants/`, such as `frFR28LinguisticVariant.csv`, are imported automatically as FHIR `concept.designation` entries tagged with BCP-47 language values such as `fr-FR` and `de-DE`.

## Format Auto-detection

| Extension or pattern | Detected format |
|---|---|
| `.tgz` or `.tar.gz` | `hl7-npm` |
| `*_tabular*.xml` | `icd10-cm` |
| `.rrf` or directory | `rxnorm` |
| `.zip` containing RF2 files | `snomed-rf2` |
| `.zip` containing `LoincTable.csv` | `loinc` |
| `.zip` containing `RXNCONSO.RRF` | `rxnorm` |

Zip files that cannot be auto-detected require `--format`.

## Exit Codes

| Code | Meaning |
|---|---|
| `0` | Success; all resources imported |
| `1` | Fatal error; import aborted |
| `2` | Success with non-fatal errors; some records skipped |
