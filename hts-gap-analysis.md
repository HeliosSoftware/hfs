# FHIR Terminology Service Completeness Analysis

Compared against [build.fhir.org/terminology-service.html](https://build.fhir.org/terminology-service.html).

---

## Operations ([spec §operations](https://build.fhir.org/terminology-service.html#operations)) — SHALL support all 6

| Operation | Spec Requirement | HTS Status | Gaps |
|-----------|-----------------|------------|------|
| `CodeSystem/$lookup` | SHALL | **Implemented (GET+POST)** | `expression` param (post-coordination) returns 501 ([spec §lookup](https://build.fhir.org/terminology-service.html#lookup)) |
| `CodeSystem/$validate-code` | SHALL | **Implemented (GET+POST)** | Uses `url` instead of spec's `system` param name ([spec §validation](https://build.fhir.org/terminology-service.html#validation)) |
| `CodeSystem/$subsumes` | SHALL | **Implemented (GET+POST)** | Single `system` param by design — no cross-code-system error needed ([spec §subsumes](https://build.fhir.org/terminology-service.html#subsumes)) |
| `ValueSet/$expand` | SHALL | **Implemented (GET+POST)** | No `too-costly` error for huge expansions (SHOULD); no hierarchical tree mode; no `date` param ([spec §expand](https://build.fhir.org/terminology-service.html#expand)) |
| `ValueSet/$validate-code` | SHALL | **Implemented (GET+POST)** | No `CodeableConcept` input support; no instance-level ([spec §validation](https://build.fhir.org/terminology-service.html#validation)) |
| `ConceptMap/$translate` | SHALL | **Implemented (GET+POST)** | No instance-level; `source`/`target` params extracted but backend filtering unverified ([spec §translate](https://build.fhir.org/terminology-service.html#translate)) |
| `$closure` | Optional | **Implemented (POST only)** | Beyond spec requirements — good to have |

**Summary:** All 6 mandatory operations are present with both GET and POST support. Remaining gaps are in advanced parameter handling and instance-level invocation.

---

## RESTful Interactions ([spec §restfulapi](https://build.fhir.org/terminology-service.html#restfulapi)) — SHALL

| Requirement | HTS Status | Notes |
|-------------|------------|-------|
| READ CodeSystem/ValueSet/ConceptMap | **Implemented** | Full CRUD (POST, GET, PUT, DELETE) with ETag/versioning |
| SEARCH CodeSystem/ValueSet/ConceptMap | **Not implemented** | No `GET /CodeSystem?url=X` style search endpoints |
| Search param: `url` | **Missing** | |
| Search param: `version` | **Missing** | |
| Search param: `name` | **Missing** | |
| Search param: `title` | **Missing** | |
| Search param: `status` | **Missing** | |

**This is the biggest gap.** The spec says servers SHALL support READ + SEARCH with all 5 search parameters for each resource type.

---

## CapabilityStatement & TerminologyCapabilities ([spec §restfulapi](https://build.fhir.org/terminology-service.html#restfulapi))

| Requirement | HTS Status | Notes |
|-------------|------------|-------|
| `GET /metadata` returns TerminologyCapabilities | **Implemented** | Lists code systems, operations, subsumption support |
| `GET /metadata?mode=terminology` | **Not implemented** | No `mode` query parameter support; always returns TerminologyCapabilities |
| `GET /metadata` (no mode / mode=full) returns CapabilityStatement | **Missing** | Spec says default should be CapabilityStatement; HTS always returns TerminologyCapabilities |
| `capabilitystatement-supported-system` extension | **Not implemented** | Required for advertising external code system support ([spec §externals](https://build.fhir.org/terminology-service.html#externals)) |

---

## Format Support ([spec §restfulapi](https://build.fhir.org/terminology-service.html#restfulapi))

| Requirement | HTS Status | Notes |
|-------------|------------|-------|
| JSON | **Implemented** | |
| XML | **Not implemented** | Spec says SHALL support both |

---

## Instance-Level Operations ([spec §operations](https://build.fhir.org/terminology-service.html#operations))

| Operation | Spec | HTS |
|-----------|------|-----|
| `ValueSet/[id]/$expand` | SHALL | **Missing** |
| `ValueSet/[id]/$validate-code` | SHALL | **Missing** |
| `ConceptMap/[id]/$translate` | SHALL | **Missing** |
| `CodeSystem/[id]/$lookup` | Optional | Missing |

---

## Batch Support ([spec §batch](https://build.fhir.org/terminology-service.html#batch), [§batch2](https://build.fhir.org/terminology-service.html#batch2))

| Requirement | HTS Status |
|-------------|------------|
| Batch Bundle with multiple `$validate-code` | **Not implemented** |
| Batch Bundle with multiple `$translate` | **Not implemented** |

Note: `POST /import` exists for bulk-loading CodeSystem/ValueSet/ConceptMap resources, but this is not the same as FHIR batch operation execution.

---

## Security ([spec §security](https://build.fhir.org/terminology-service.html#security))

| Requirement | Level | HTS Status |
|-------------|-------|------------|
| SSL/TLS | SHOULD | Not enforced (deployment concern) |
| Authentication/Authorization | MAY | Not implemented |

---

## Other Gaps

| Feature | Status | Spec Reference |
|---------|--------|----------------|
| Implicit value sets (CodeSystem.valueSet → all codes) | Not implemented | [§externals](https://build.fhir.org/terminology-service.html#externals) |
| `date` parameter for point-in-time evaluation | Not implemented on any operation | [§expand](https://build.fhir.org/terminology-service.html#expand), [§validation](https://build.fhir.org/terminology-service.html#validation) |
| `displayLanguage` filtering in responses | Param accepted in `$lookup` but designations not actually filtered | [§lookup](https://build.fhir.org/terminology-service.html#lookup), [§standard-props](https://build.fhir.org/terminology-service.html#standard-props) |
| Hierarchical expansion (tree mode) | Not implemented (metadata explicitly sets `hierarchical: false`) | [§expand](https://build.fhir.org/terminology-service.html#expand) |
| `CodeableConcept` input for `$validate-code` | Not implemented (string codes only) | [§validation](https://build.fhir.org/terminology-service.html#validation) |

---

## Scorecard

| Category | Score | Notes |
|----------|-------|-------|
| **Core Operations** | 6/6 present, ~75% param coverage | All exist with GET+POST; gaps in advanced params |
| **RESTful Interactions** | READ done, SEARCH missing | 5 mandatory search params × 3 resource types = 0/15 |
| **Capability Reporting** | Partial | TerminologyCapabilities exists; CapabilityStatement and mode switching missing |
| **Format Support** | 1/2 | JSON only, XML missing (SHALL) |
| **Instance-Level Ops** | 0/3 | None of the required instance-level operation endpoints |
| **Batch** | 0/2 | No batch bundle support |

## Priority Recommendations

1. **Search endpoints** (`/CodeSystem?url=...` etc.) — biggest SHALL-level gap ([§restfulapi](https://build.fhir.org/terminology-service.html#restfulapi))
2. **Instance-level operations** (`/ValueSet/[id]/$expand`, etc.) — required by spec ([§operations](https://build.fhir.org/terminology-service.html#operations))
3. **XML format support** — SHALL requirement, though rarely used in practice ([§restfulapi](https://build.fhir.org/terminology-service.html#restfulapi))
4. **`mode` parameter on `/metadata`** — to properly distinguish CapabilityStatement vs TerminologyCapabilities ([§restfulapi](https://build.fhir.org/terminology-service.html#restfulapi))
5. **`displayLanguage` filtering** — parameter is accepted but silently ignored; should filter or drop the param ([§lookup](https://build.fhir.org/terminology-service.html#lookup))
