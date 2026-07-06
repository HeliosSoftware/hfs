# IG Publisher compatibility

**Status:** Enhancement to `helios-hts` for upstream contribution to
[HeliosSoftware/hfs](https://github.com/HeliosSoftware/hfs).

**Motivation:** [HL7 FHIR IG Publisher](https://github.com/HL7/fhir-ig-publisher)
(2.2.x) validates Implementation Guide artifacts against a terminology server via
`-tx` / `IG_PUBLISHER_TX`. Without these changes, Publisher validation reported
dozens of false-positive errors even when codes were loaded and curl-based
`$validate-code` calls succeeded.

**Validated consumer:** Atrius IG (`AtriusIGDraft`) — `./_build.sh` probes HTS via
`-tx http://127.0.0.1:9091`. Clear the Publisher TX disk cache
(`input-cache/txcache`) after HTS behaviour changes so stale false negatives are
not replayed.

---

## Summary

Seven independent gaps were fixed:

| Gap | Symptom in Publisher `qa.txt` | Fix location |
|-----|-------------------------------|--------------|
| Parameter name / shape mismatch | Composition: *Must provide one of: code, coding (valueCoding), or codeableConcept*; Library: spurious validate failures | `operations/params.rs`, `operations/validate_code.rs` |
| `version=current` on SNOMED | Library: *Unknown code '…' in CodeSystem 'http://snomed.info/sct' version 'current'* | `backends/mod.rs`, `backends/{sqlite,postgres}/code_system.rs` |
| Empty LOINC stub wins over full import | Library: *Unknown code '85354-9' in CodeSystem 'http://loinc.org'* despite LOINC loaded | `backends/{sqlite,postgres}/code_system.rs`, `import/loinc_csv.rs` |
| BCP-13 / `mimetypes` ValueSet | Library `contentType`: *Unknown code 'text/cql' in ValueSet 'http://hl7.org/fhir/ValueSet/mimetypes'* | `bcp13.rs`, `operations/validate_code.rs`, `backends/{sqlite,postgres}/value_set.rs` |
| Inline ValueSet + CodeableConcept | Composition `type`: same *Must provide one of…* error when Publisher sends inline `valueSet` + `codeableConcept` without `url` | `operations/validate_code.rs` (`process_inline_vs_validate_code`) |
| Handler cache pins false negatives | Transient failures cached for process lifetime; Publisher disk cache amplifies stale results | `operations/validate_code.rs` (`validate_code_cache_put`) |
| UCUM composed units (`mg`, etc.) | ActivityDefinition dosage: *Unknown code 'mg' in CodeSystem 'http://unitsofmeasure.org'* despite UCUM import | `ucum_validate.rs`, `backends/{sqlite,postgres}/code_system.rs` |

After deploying a **rebuilt** `hts` binary, clearing Publisher TX cache, and keeping
HTS up through the full IG validation phase, Atrius IG Publisher errors dropped
from **64 → 0** (with UCUM composed-unit validation and CQL SNOMED fix).

---

## 1. `$validate-code` parameter aliases (IG Publisher)

FHIR defines in-parameter names `coding` and `codeableConcept`. IG Publisher
(and some HAPI tooling paths) often send the **value-type names** as the
parameter `name`:

| FHIR spec name | Publisher alias | Payload field |
|----------------|-----------------|---------------|
| `url` (CodeSystem bare-code path) | `system` | `valueUri` |
| `coding` | `valueCoding` | `valueCoding` |
| `codeableConcept` | `valueCodeableConcept` | `valueCodeableConcept` |

The FHIR R4 `$validate-code` operation defines the CodeSystem canonical as in-parameter
`url`. IG Publisher (and some HAPI paths) send **`system` + `code`** instead — e.g.
Library `dataRequirement` validation against LOINC. HTS accepts `system` as an alias
for `url` on **CodeSystem Path 1** (bare `code` only). When both are present, `url`
wins. If neither is present, the handler returns `400` with *Missing required parameter:
url or system*.

This is an intentional deviation from strict spec-only parsing to match real Publisher
traffic; it is covered by integration test `system_param_alias_validates_code` (replacing
the former `system_param_rejected_with_400`, which asserted spec-only rejection).

### Code changes

**`src/operations/params.rs`**

- `extract_coding_full("coding")` also accepts a parameter named `valueCoding`.
- `extract_codeable_concept("codeableConcept")` also accepts `valueCodeableConcept`.
- `find_codeable_concept_param()` — resolves CC payload from either name (used
  when echoing display/version in ValueSet validate responses).
- `coding_entries_from_codeable_concept()` — normalizes `coding` as either a
  **JSON array** (spec-correct) or a **single object** (seen in some tooling
  examples and Publisher payloads).

**`src/operations/validate_code.rs`**

- **CodeSystem Path 1** (bare `code`): accepts `system` as an alias for `url`
  when identifying the CodeSystem canonical URL. Publisher Library
  `dataRequirement` validation uses `system` + `code`, not `url` + `code`.
- **ValueSet Path 3** (`codeableConcept`): uses `find_codeable_concept_param` and
  `coding_entries_from_codeable_concept` for display/version extraction.

### Tests

- `extract_coding_accepts_value_coding_alias`
- `extract_codeable_concept_accepts_value_codeable_concept_alias`
- `extract_codeable_concept_accepts_single_coding_object`
- `system_param_alias_validates_code` — `system` + `code` returns `200` / `result=true`
  (replaces `system_param_rejected_with_400`, which encoded pre-Publisher strictness)

### Manual verification

```bash
# CodeSystem — Publisher-style system + code
curl -s -X POST 'http://127.0.0.1:9091/CodeSystem/$validate-code' \
  -H 'Content-Type: application/fhir+json' \
  -d '{"resourceType":"Parameters","parameter":[
    {"name":"system","valueUri":"http://loinc.org"},
    {"name":"code","valueCode":"85354-9"}
  ]}'

# ValueSet — Publisher-style valueCodeableConcept + single-object coding
curl -s -X POST 'http://127.0.0.1:9091/ValueSet/$validate-code' \
  -H 'Content-Type: application/fhir+json' \
  -d '{"resourceType":"Parameters","parameter":[
    {"name":"url","valueUri":"https://example.org/ValueSet/example"},
    {"name":"valueCodeableConcept","valueCodeableConcept":{
      "coding":{"system":"http://snomed.info/sct","code":"394659003"}
    }}
  ]}'
```

Both should return `"result": true` when the code is loaded.

---

## 2. SNOMED `version=current` resolution

CQL-generated Library `dataRequirement` entries validate SNOMED codes with
`version=current`. VSAC stubs and some NPM packages also store a CodeSystem row
with `version: "current"` and `content: not-present` (no concepts).

### Behaviour

When `$validate-code` (or backend `validate_code`) receives `version=current`:

1. **`fetch_versions` / Postgres resolve query** orders candidates so that:
   - `content = complete` or `supplement` ranks before `fragment`, `example`, `not-present`
   - rows **with concepts** rank before empty stubs
   - then highest `version` string, then stable `id`
2. **`resolve_code_system*`** treats `current` as “pick the first candidate after
   ordering” instead of literal string equality.

### Code changes

- `src/backends/mod.rs` — `code_system_version_is_current()`
- `src/backends/sqlite/code_system.rs` — `fetch_versions`, `resolve_code_system_uncached`
- `src/backends/postgres/code_system.rs` — `resolve_code_system` ORDER BY + `current` arm

### Tests

- `validate_code_version_current_resolves_loaded_edition` (SQLite integration test)

---

## 3. LOINC import version labeling

LOINC ZIP layouts use a top-level folder such as `Loinc_2.82/LoincTable/Loinc.csv`.
Previously HTS imported concepts but left `CodeSystem.version` unset (`NULL`).
Multiple rows for `http://loinc.org` (HL7 stub, consult-minimal pack, full import)
then competed during validation; an empty or minimal row could win.

### Behaviour

On LOINC CSV/ZIP import:

1. Parse release version from the first `Loinc_<ver>/` path segment (e.g. `2.82`).
2. Store it on `CodeSystemMeta.version` for the imported edition.
3. Combined with improved `fetch_versions` ordering, the full import is preferred
   over empty stubs when version is unpinned or `current`.

### Code changes

- `src/import/loinc_csv.rs` — `loinc_release_version_from_entry()`, `LoincArchivePaths.release_version`

### Tests

- `loinc_release_version_from_entry_parses_top_level_folder`
- `import_loinc_nested_zip_layout` — asserts `version = 2.77` from fixture path

### Operational note

If an existing database already has the full LOINC import but `version` was wrong,
you can `UPDATE code_systems SET version='2.82' WHERE url='http://loinc.org' AND …`
or delete stale stub rows — reimport is optional once ordering + version label are
correct.

---

## 4. BCP-13 / `mimetypes` ValueSet

The FHIR core `http://hl7.org/fhir/ValueSet/mimetypes` composes all codes from
`urn:ietf:bcp:13` — an unbounded code system. tx.fhir.org accepts any
syntactically valid `type/subtype` rather than materialising every MIME type.

### Publisher behaviour

IG Publisher validates Library `contentType` bindings by POSTing to
`ValueSet/$validate-code` with:

- **`code`** (bare code, no `system`) — e.g. `text/cql`, `application/elm+json`
- **`valueSet`** — inline copy of the core `mimetypes` ValueSet from the FHIR
  package (compose: `include[{ system: urn:ietf:bcp:13 }]`)
- **No top-level `url` parameter** — triggers the inline ValueSet path

A URL-only BCP-13 short-circuit is insufficient; the inline path must also detect
mimetypes compose and validate MIME syntax instead of expanding an empty expansion.

### Behaviour

1. **`is_mimetypes_valueset_url`** — matches `http://hl7.org/fhir/ValueSet/mimetypes`
   (optional `|version` suffix).
2. **`is_valid_mime_type`** — RFC 2045 `type/subtype` syntax check (supports
   structured suffixes like `application/elm+json`; ignores `;` parameters).
3. **URL-based `$validate-code`** — short-circuit before empty expansion /
   `inferSystem` ambiguity when `url` is the mimetypes canonical.
4. **Inline `$validate-code`** — when inline `valueSet` is mimetypes (by URL or
   BCP-13-only compose), validate each coding attempt via BCP-13 syntax.
5. **`detect_bad_vs_import`** — skip for mimetypes VS (no false unresolved-import
   issues on the unbounded compose).

### Code changes

- `src/bcp13.rs` — new module: constants, syntax validation, `validate_mimetypes_code`
- `src/lib.rs` — export `bcp13` module
- `src/operations/validate_code.rs` — inline + URL mimetypes paths
- `src/backends/{sqlite,postgres}/value_set.rs` — URL short-circuit in expand path

### Tests

- `bcp13::tests::accepts_common_ig_mime_types`, `rejects_obviously_invalid_mime_types`
- `bcp13::tests::mimetypes_validate_success_without_system`
- `vs_inline_mimetypes_publisher_shape_validates_text_cql` (integration)
- `vs_inline_mimetypes_rejects_invalid_syntax` (integration)

### Manual verification

```bash
# URL-based (no BCP-13 CS loaded locally)
curl -s -X POST 'http://127.0.0.1:9091/ValueSet/$validate-code' \
  -H 'Content-Type: application/fhir+json' \
  -d '{"resourceType":"Parameters","parameter":[
    {"name":"url","valueUri":"http://hl7.org/fhir/ValueSet/mimetypes"},
    {"name":"code","valueCode":"text/cql"}
  ]}'

# Inline — Publisher Library contentType shape (no url param)
curl -s -X POST 'http://127.0.0.1:9091/ValueSet/$validate-code' \
  -H 'Content-Type: application/fhir+json' \
  -d '{"resourceType":"Parameters","parameter":[
    {"name":"code","valueCode":"text/cql"},
    {"name":"valueSet","resource":{
      "resourceType":"ValueSet",
      "url":"http://hl7.org/fhir/ValueSet/mimetypes",
      "version":"4.0.1",
      "status":"active",
      "compose":{"include":[{"system":"urn:ietf:bcp:13"}]}
    }}
  ]}'
```

Both should return `"result": true` and `"system": "urn:ietf:bcp:13"`.

---

## 5. Inline ValueSet + CodeableConcept

Composition examples bind `Composition.type` against a profile ValueSet. Publisher
sends:

- **`valueCodeableConcept`** (alias for `codeableConcept`) with SNOMED coding
- **`valueSet`** — inline ValueSet resource with `compose.include` SNOMED concepts
- **No top-level `url` parameter**

Previously `process_inline_vs_validate_code` only accepted `coding` or bare `code`,
returning *Must provide one of: code, coding…* for CodeableConcept payloads.

### Behaviour

1. Accept `codeableConcept` / `valueCodeableConcept` on the inline path.
2. Iterate codings **last-to-first** (IG “last match wins” convention).
3. Expand inline compose and check membership (same as existing inline flow).

### Code changes

- `src/operations/validate_code.rs` — `process_inline_vs_validate_code`

### Tests

- `vs_inline_codeable_concept_publisher_shape_validates` (integration)

### Manual verification

```bash
curl -s -X POST 'http://127.0.0.1:9091/ValueSet/$validate-code' \
  -H 'Content-Type: application/fhir+json' \
  -d '{"resourceType":"Parameters","parameter":[
    {"name":"valueCodeableConcept","valueCodeableConcept":{
      "coding":[{
        "system":"http://snomed.info/sct",
        "code":"371530004",
        "display":"Clinical consultation report"
      }]
    }},
    {"name":"valueSet","resource":{
      "resourceType":"ValueSet",
      "url":"https://example.org/fhir/ValueSet/composition-type",
      "status":"active",
      "compose":{"include":[{
        "system":"http://snomed.info/sct",
        "concept":[{"code":"371530004"}]
      }]}
    }}
  ]}'
```

Requires SNOMED loaded locally. Should return `"result": true`.

---

## 6. Handler cache policy

The in-process `$validate-code` handler cache previously stored all responses.
False negatives from bootstrap timing, stale request shapes, or empty expansions
were pinned for the process lifetime; Publisher also caches TX responses on disk.

### Behaviour

`validate_code_cache_put` now caches **only `result=true`** responses. Failed
validations are always re-evaluated on the next request.

### Code changes

- `src/operations/validate_code.rs` — `validate_code_cache_put`

---

## 7. UCUM composed units

The UCUM `ucum-essence.xml` import stores **atomic** codes (prefixes, base units,
named units). Composed expressions such as `mg` (milli + gram) are valid UCUM but
are not individual rows in the essence file. tx.fhir.org validates them
structurally via a UCUM parser.

### Publisher behaviour

ActivityDefinition examples bind `dosage.doseQuantity.code` to UCUM (`mg` with
system `http://unitsofmeasure.org`). Publisher calls
`CodeSystem/$validate-code` with `url` + `code`.

### Behaviour

When a concept lookup fails for `http://unitsofmeasure.org`, HTS falls back to
`octofhir-ucum` structural validation before returning *Unknown code*.

### Code changes

- `src/ucum_validate.rs` — UCUM URL detection and composed-unit validation
- `src/backends/{sqlite,postgres}/code_system.rs` — fallback on unknown concept
- `Cargo.toml` — `octofhir-ucum` dependency

### Tests

- `ucum_validate::tests::accepts_composed_mg`
- `cs_ucum_composed_mg_validates_when_not_in_essence_table` (integration)

### Manual verification

```bash
curl -s -X POST 'http://127.0.0.1:9091/CodeSystem/$validate-code' \
  -H 'Content-Type: application/fhir+json' \
  -d '{"resourceType":"Parameters","parameter":[
    {"name":"url","valueUri":"http://unitsofmeasure.org"},
    {"name":"code","valueCode":"mg"}
  ]}'
```

Should return `"result": true` when UCUM 2.2 is loaded (even though `mg` is not
an essence row).

---

## Build and runtime requirements

These fixes exist only in the **compiled binary**. Rebuild without a stale sandbox
target directory:

```bash
cd /path/to/atrius-hfs
unset CARGO_TARGET_DIR
cargo build --release --bin hts
./target/release/hts run --port 9091
```

Wait for log line `HTS listening address=127.0.0.1:9091` (~5 minutes on a large
SNOMED+LOINC database) **before** running IG Publisher validation. HTS must stay
up for the entire `./_build.sh` validation phase (~30s of terminology traffic).

---

## Files touched

| File | Change |
|------|--------|
| `src/bcp13.rs` | BCP-13 MIME syntax validation for mimetypes ValueSet |
| `src/ucum_validate.rs` | UCUM composed-unit structural validation |
| `src/lib.rs` | Export `bcp13`, `ucum_validate` modules |
| `src/operations/params.rs` | Publisher parameter aliases; single-object `coding` |
| `src/operations/validate_code.rs` | Inline mimetypes + CC; cache policy; `system`/`url` alias |
| `src/backends/mod.rs` | `code_system_version_is_current()` |
| `src/backends/sqlite/code_system.rs` | `version=current`; `fetch_versions` ordering; test |
| `src/backends/postgres/code_system.rs` | Same ordering + `current` |
| `src/backends/sqlite/value_set.rs` | Mimetypes URL short-circuit in expand |
| `src/backends/postgres/value_set.rs` | Same |
| `src/import/loinc_csv.rs` | Release version from ZIP path |
| `docs/ig-publisher-compatibility.md` | This document |
| `README.md` | Link to this document |

---

## Related docs

- [ig-publisher-compatibility-pr.md](./ig-publisher-compatibility-pr.md) — suggested upstream PR title, summary, and test plan
- [fork-ecl-fts-typeahead-expand.md](./fork-ecl-fts-typeahead-expand.md) — SNOMED typeahead `$expand` (separate Atrius fork change)
- [expand-paths-architecture.md](./expand-paths-architecture.md) — ValueSet `$expand` internals
