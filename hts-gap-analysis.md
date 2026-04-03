# FHIR Terminology Service Completeness Analysis

Compared against [build.fhir.org/terminology-service.html](https://build.fhir.org/terminology-service.html).

Last updated: 2026-04-02

---

## Operations (SHALL support all 6)

| Operation | Spec Requirement | HTS Status | Gaps |
|-----------|-----------------|------------|------|
| `CodeSystem/$lookup` | SHALL | **Implemented** | `expression` param (post-coordination) returns 501; `displayLanguage` param is extracted but filtering is backend-dependent |
| `CodeSystem/$validate-code` | SHALL | **Implemented** | Uses `url` instead of spec's `system` param name — `system` is actively rejected with 400 |
| `CodeSystem/$subsumes` | SHALL | **Implemented** | No cross-code-system error handling (spec: SHALL return error) |
| `ValueSet/$expand` | SHALL | **Implemented** | No `too-costly` error for huge expansions (SHOULD); no hierarchical tree mode; no `date` param |
| `ValueSet/$validate-code` | SHALL | **Implemented** | No `CodeableConcept` input support (params.rs only handles primitive value types); no instance-level (`/ValueSet/[id]/$validate-code`) |
| `ConceptMap/$translate` | SHALL | **Implemented** | `source`/`target` params are captured and forwarded to backend (no longer missing); no instance-level |
| `$closure` | Optional | **Implemented** | POST only (no GET) — acceptable for optional operation |

**GET support:** All 6 mandatory operations now support both GET (query params) and POST (FHIR Parameters body). `$closure` is POST-only.

---

## RESTful Interactions (SHALL)

| Requirement | HTS Status | Notes |
|-------------|------------|-------|
| READ CodeSystem/ValueSet/ConceptMap | **Implemented** | Full CRUD (POST, GET, PUT, DELETE) with ETag/versioning |
| SEARCH CodeSystem/ValueSet/ConceptMap | **Implemented** | `GET /CodeSystem?url=X`, `GET /ValueSet?name=Y`, etc. |
| Search param: `url` | **Implemented** | Exact match on canonical URL |
| Search param: `version` | **Implemented** | Exact match on version string |
| Search param: `name` | **Implemented** | Exact match on computer name |
| Search param: `title` | **Implemented** | Exact match on human title |
| Search param: `status` | **Implemented** | Exact match on status |

Results returned as a FHIR `Bundle` of type `searchset` with `total` and `entry[]`. Pagination via `_count` / `_offset`.

---

## CapabilityStatement & TerminologyCapabilities

| Requirement | HTS Status | Notes |
|-------------|------------|-------|
| `GET /metadata` returns TerminologyCapabilities | **Implemented** | Lists code systems, operations, subsumption support |
| `GET /metadata?mode=terminology` | **Not implemented** | Spec distinguishes `mode=full` (CapabilityStatement) vs `mode=terminology` (TerminologyCapabilities). HTS ignores the `mode` param entirely |
| `GET /metadata` (no mode / mode=full) returns CapabilityStatement | **Missing** | HTS always returns TerminologyCapabilities; a full CapabilityStatement is never returned |
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

| Requirement | HTS Status | Notes |
|-------------|------------|-------|
| `POST /import` — FHIR Bundle ingestion | **Implemented** | Accepts collection/transaction/batch bundles; imports CodeSystem → ValueSet → ConceptMap in order; returns 207 on partial failure |
| Standard FHIR batch bundle (`POST /`) with terminology operations | **Not implemented** | The spec allows batch bundles containing multiple `$validate-code` / `$translate` calls via the normal FHIR batch endpoint |

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
| `displayLanguage` filtering in responses | Param extracted in `$lookup`, actual filtering is backend-dependent |
| Hierarchical expansion (tree mode) | Not implemented |
| Cross-code-system subsumption error | Not implemented (SHALL) |
| `CodeableConcept` / `valueCoding` input | Not implemented — params.rs only handles primitive value types (valueCode, valueString, valueUri, etc.) |
| `expression` param in `$lookup` (post-coordination) | Returns 501 Not Supported |

---

## Scorecard

| Category | Score | Notes |
|----------|-------|-------|
| **Core Operations** | 6/6 present, ~80% param coverage | All exist; GET+POST both supported; `source`/`target` in translate now captured |
| **RESTful Interactions** | READ + SEARCH done | 5 search params × 3 resource types = 15/15 ✅ |
| **Capability Reporting** | Partial | TerminologyCapabilities exists; CapabilityStatement and `mode` switching missing |
| **Format Support** | 1/2 | JSON only, XML missing (SHALL) |
| **Instance-Level Ops** | 0/3 | None of the required instance-level operation endpoints |
| **Batch** | Partial | `/import` handles Bundle ingestion; standard FHIR batch endpoint missing |

## Priority Recommendations

1. ~~**Search endpoints**~~ — ✅ implemented (Phase 1)
2. **Instance-level operations** (`/ValueSet/[id]/$expand`, etc.) — required by spec
3. **XML format support** — SHALL requirement, though rarely used in practice
4. **`mode` parameter on `/metadata`** — to properly distinguish CapabilityStatement vs TerminologyCapabilities
5. **`CodeableConcept`/`valueCoding` param support** — needed for full `$validate-code` compliance
6. **Cross-code-system error in `$subsumes`** — SHALL-level requirement
