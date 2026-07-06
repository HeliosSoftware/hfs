# Pull request: HTS IG Publisher `$validate-code` compatibility

Suggested branch: `feat/hts-ig-publisher-validate-code` (from `upstream/main`)

Suggested title:

```
feat(hts): IG Publisher $validate-code compatibility
```

---

## Summary

HL7 FHIR IG Publisher (2.2.x) validates Implementation Guides against a terminology
server via `-tx`. HTS previously reported dozens of false-positive validation errors
even when terminology was loaded and manual `$validate-code` probes succeeded.

This PR aligns HTS with Publisher request shapes and tx.fhir.org behaviour for:

- Parameter name aliases (`valueCoding`, `valueCodeableConcept`, and **`system`** for CodeSystem URL)
- SNOMED `version=current` resolution
- LOINC import version labeling and stub ordering
- BCP-13 / inline `mimetypes` ValueSet validation
- Inline ValueSet + `codeableConcept` (Composition bindings)
- Handler cache policy (only cache `result=true`)
- UCUM composed units (`mg`, etc.) via structural validation

Validated against Atrius IG: Publisher errors **64 → 0** after rebuild, TX cache clear,
and HTS kept up through validation.

Full design notes: [ig-publisher-compatibility.md](./ig-publisher-compatibility.md)

---

## CodeSystem `system` parameter (spec vs Publisher)

**FHIR spec:** CodeSystem `$validate-code` bare-code form uses in-parameter `url` for the
CodeSystem canonical.

**IG Publisher:** Often sends `system` + `code` (e.g. Library `dataRequirement` / LOINC
validation), not `url` + `code`.

**HTS behaviour:** On CodeSystem Path 1 (bare `code`), accept `system` as an alias for
`url`. Prefer `url` when both are present. Return `400` only when neither is supplied.

**Test change:** Replaced `system_param_rejected_with_400` (asserted spec-only rejection)
with `system_param_alias_validates_code` (asserts `200` and `result=true` for Publisher
shape). This documents the intentional compatibility choice for upstream review.

---

## Test plan

- [ ] `cargo fmt --check`
- [ ] `cargo clippy -p helios-hts -- -D warnings`
- [ ] `cargo test -p helios-hts`
- [ ] Manual probes (HTS on `:9091`):
  - [ ] CodeSystem: `system` + `code` (LOINC `85354-9`)
  - [ ] CodeSystem: UCUM `mg` on `http://unitsofmeasure.org`
  - [ ] ValueSet: inline `mimetypes` + `code=text/cql` (no `url`)
  - [ ] ValueSet: inline VS + `valueCodeableConcept` (no `url`)
- [ ] Optional: IG Publisher build with `-tx http://127.0.0.1:9091` and cleared
  `input-cache/txcache`

Key automated tests:

| Area | Test |
|------|------|
| `system` alias | `system_param_alias_validates_code` |
| Parameter aliases | `extract_coding_accepts_value_coding_alias`, etc. |
| Inline mimetypes | `vs_inline_mimetypes_publisher_shape_validates_text_cql` |
| Inline CC | `vs_inline_codeable_concept_publisher_shape_validates` |
| UCUM composed | `cs_ucum_composed_mg_validates_when_not_in_essence_table`, `ucum_validate::tests::*` |
| SNOMED `current` | `validate_code_version_current_resolves_loaded_edition` |

---

## Files touched (high level)

See [ig-publisher-compatibility.md § Files touched](./ig-publisher-compatibility.md#files-touched).

**Exclude from PR** (formatting-only, no behaviour change): `ecl/*`, `sqlite/mod.rs` if
present as unrelated diffs.

---

## Breaking changes

None for production API consumers. **Test expectation change only:** integration test
`system_param_rejected_with_400` removed in favour of `system_param_alias_validates_code`
because Publisher compatibility requires accepting `system` on the bare-code path.
