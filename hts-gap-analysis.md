# FHIR Terminology Service Completeness Analysis

Compared against [build.fhir.org/terminology-service.html](https://build.fhir.org/terminology-service.html).

---

## Operations (SHALL support all 6)

| Operation | Spec Requirement | HTS Status | Gaps |
|-----------|-----------------|------------|------|
| `CodeSystem/$lookup` | SHALL | **Implemented** | `expression` param (post-coordination) returns 501; GET not supported (POST only) |
| `CodeSystem/$validate-code` | SHALL | **Implemented** | Uses `url` instead of spec's `system` param name; GET not supported |
| `CodeSystem/$subsumes` | SHALL | **Implemented** | No cross-code-system error handling (spec: SHALL return error); GET not supported |
| `ValueSet/$expand` | SHALL | **Implemented** | No `too-costly` error for huge expansions (SHOULD); no hierarchical tree mode; no `date` param; GET not supported |
| `ValueSet/$validate-code` | SHALL | **Implemented** | No `CodeableConcept` input support; no instance-level (`/ValueSet/[id]/$validate-code`); GET not supported |
| `ConceptMap/$translate` | SHALL | **Implemented** | No `source`/`target` value set filtering; no instance-level; GET not supported |
| `$closure` | Optional | **Implemented** | Beyond spec requirements — good to have |

**Summary:** All 6 mandatory operations are present. Main gap is **GET support** — spec implies both GET and POST for all operations, HTS only supports POST.

---

## RESTful Interactions (SHALL)

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

## CapabilityStatement & TerminologyCapabilities

| Requirement | HTS Status | Notes |
|-------------|------------|-------|
| `GET /metadata` returns TerminologyCapabilities | **Implemented** | Lists code systems, operations, subsumption support |
| `GET /metadata?mode=terminology` | **Not verified** | Spec distinguishes `mode=full` (CapabilityStatement) vs `mode=terminology` (TerminologyCapabilities) |
| `GET /metadata` (no mode / mode=full) returns CapabilityStatement | **Likely missing** | HTS returns TerminologyCapabilities for `/metadata`; spec says a full CapabilityStatement should be the default |
| `capabilitystatement-supported-system` extension | **Not implemented** | Required for advertising external code system support |

---

## Format Support

| Requirement | HTS Status | Notes |
|-------------|------------|-------|
| JSON | **Implemented** | |
| XML | **Not implemented** | Spec says SHALL support both |

---

## Instance-Level Operations

| Operation | Spec | HTS |
|-----------|------|-----|
| `ValueSet/[id]/$expand` | SHALL | **Missing** |
| `ValueSet/[id]/$validate-code` | SHALL | **Missing** |
| `ConceptMap/[id]/$translate` | SHALL | **Missing** |
| `CodeSystem/[id]/$lookup` | Optional | Missing |

---

## Batch Support

| Requirement | HTS Status |
|-------------|------------|
| Batch Bundle with multiple `$validate-code` | **Not implemented** |
| Batch Bundle with multiple `$translate` | **Not implemented** |

---

## Security

| Requirement | Level | HTS Status |
|-------------|-------|------------|
| SSL/TLS | SHOULD | Not enforced (deployment concern) |
| Authentication/Authorization | MAY | Not implemented |

---

## Other Gaps

| Feature | Status |
|---------|--------|
| NamingSystem resource | Not implemented (not strictly required) |
| Implicit value sets (CodeSystem.valueSet → all codes) | Not implemented |
| `date` parameter for point-in-time evaluation | Not implemented |
| `displayLanguage` filtering in responses | Param accepted in `$lookup` but designations not filtered |
| Hierarchical expansion (tree mode) | Not implemented |
| Cross-code-system subsumption error | Not implemented (SHALL) |

---

## Scorecard

| Category | Score | Notes |
|----------|-------|-------|
| **Core Operations** | 6/6 present, ~70% param coverage | All exist but with gaps in parameters and GET support |
| **RESTful Interactions** | READ done, SEARCH missing | 5 mandatory search params × 3 resource types = 0/15 |
| **Capability Reporting** | Partial | TerminologyCapabilities exists, CapabilityStatement and mode switching missing |
| **Format Support** | 1/2 | JSON only, XML missing (SHALL) |
| **Instance-Level Ops** | 0/3 | None of the required instance-level operation endpoints |
| **Batch** | 0/2 | No batch bundle support |

## Priority Recommendations

1. **Search endpoints** (`/CodeSystem?url=...` etc.) — biggest SHALL-level gap
2. **Instance-level operations** (`/ValueSet/[id]/$expand`, etc.) — required by spec
3. **GET support for operations** — spec expects both GET and POST
4. **XML format support** — SHALL requirement, though rarely used in practice
5. **`mode` parameter on `/metadata`** — to properly distinguish CapabilityStatement vs TerminologyCapabilities
