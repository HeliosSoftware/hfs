# HTS Bundled Terminology Data

This directory holds the public-domain and permissively-licensed terminology distributions that ship inside every HTS release archive and Docker image. End users do not need to download anything — the CI release job copies this directory into each platform archive, and the `hts` Docker image bakes it in at `/app/terminology-data` (auto-imported on first run via `HTS_BOOTSTRAP_DIR`).

## Don't edit by hand

This directory is machine-managed. Refresh before each release with the download script:

```bash
./crates/hts/scripts/download-bundled-terminologies.sh ./crates/hts/terminology-data
```

The pinned versions (and their upstream landing pages) live at the top of `download-bundled-terminologies.sh`. Bump the version variables there before re-running, then `git add` and commit the refreshed files.

See [`RELEASING.md`](../../../RELEASING.md) for the full refresh workflow.

## What's in here

Each file is a distribution as published by its issuing authority. `hts import ./crates/hts/terminology-data` iterates the directory, auto-detects each file's format, and imports them all into the target database in one pass.

## Licensing

Every file in this directory carries its own license — public domain (US federal government works: ICD-10-CM, ICD-9-CM, NCI Thesaurus, MeSH, NDC), HL7 FHIR License with attribution (HL7 THO, HL7 v2 tables), or similarly permissive (UCUM, NUCC). See the `Terminology Support` section of [`crates/hts/README.md`](../README.md#terminology-support) for the per-terminology attribution text each distributor requires when redistributing.

Terminologies that require registration or a paid license (SNOMED CT, LOINC, RxNorm, CPT, MedDRA) are **not** included here — customers must obtain them from their issuing authority and import them manually.
