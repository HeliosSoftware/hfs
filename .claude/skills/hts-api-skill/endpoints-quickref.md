# HTS endpoints quick reference

All 42 method/path pairs registered by the HTS binary, grouped by resource
family. Full parameter matrices and behavior notes live in
[../../edson/docs/hts-details.md](../../edson/docs/hts-details.md).

Legend:
- **Body shape**: `-` (no body) / `Params` (FHIR `Parameters` resource) /
  `Bundle` (FHIR Bundle) / `Raw` (raw JSON, non-Parameters)
- **`Accept`**: `+xml` means XML response is supported;
  `json-only` means the handler ignores XML negotiation
- **Auth**: none at HTS itself — see [SKILL.md §5](SKILL.md#5-auth-gaps-the-ui-must-handle)
- Instance operation routes are registered **before** generic `/{id}` CRUD
  so the operation suffix is not captured as an id
  ([edson/docs/hts-details.md §Route ordering](../../edson/docs/hts-details.md))

Route count: **42**
(5 utility + 13 CodeSystem + 14 ValueSet + 10 ConceptMap; matches the total
in [edson/docs/hts-details.md](../../edson/docs/hts-details.md) §Executive
summary).

---

## Utility, conformance, batch, import (5)

| Method | Path | Body | `Accept` | Purpose | Notes |
|---|---|---|---|---|---|
| GET | `/health` | – | json-only | Process health | Always 200; does **not** check database |
| GET | `/metadata` | – | +xml | CapabilityStatement | `mode=terminology` → TerminologyCapabilities; other values fall back to CapabilityStatement |
| GET | `/metrics` | – | text/plain | Prometheus text | 503 with plain-text body if recorder skipped |
| POST | `/` | Bundle | +xml | Root batch/transaction | Accepts `batch` / `transaction`; **not atomic**; only three entry URLs supported (see below) |
| POST | `/import` | Raw JSON Bundle | json-only | FHIR Bundle import | Returns non-FHIR JSON summary; 200 / 207 / 400 / 500 |

Root batch entry `request.url` values recognized:
`CodeSystem/$validate-code`, `ValueSet/$validate-code`,
`ConceptMap/$translate`. `entry.request.method` is ignored.

---

## CodeSystem (13)

### Operations

| Method | Path | Body | Purpose |
|---|---|---|---|
| GET | `/CodeSystem/$lookup` | – | Type-level lookup by `system` + `code` |
| POST | `/CodeSystem/$lookup` | Params | Structured lookup (property=*, supplements, dates) |
| GET | `/CodeSystem/$validate-code` | – | Scalar `url`/`system` + `code` validation |
| POST | `/CodeSystem/$validate-code` | Params | Coding / CodeableConcept validation |
| GET | `/CodeSystem/$subsumes` | – | `system` + `codeA` + `codeB` |
| POST | `/CodeSystem/$subsumes` | Params | Structured `codingA` / `codingB` |
| GET | `/CodeSystem/{id}/$lookup` | – | Instance lookup (id overrides `system`) |
| POST | `/CodeSystem/{id}/$lookup` | Params | Instance lookup |

CRUD:

| Method | Path | Body | Purpose |
|---|---|---|---|
| GET | `/CodeSystem` | – | Search (`url`, `version`, `name`, `title`, `status`, `_count`, `_offset`, `_summary`) |
| POST | `/CodeSystem` | Raw resource | Create; returns 201 + `Location` + ETag |
| GET | `/CodeSystem/{id}` | – | Read; 404 on missing/soft-deleted |
| PUT | `/CodeSystem/{id}` | Raw resource | Update; `If-Match` supported (412 on mismatch) |
| DELETE | `/CodeSystem/{id}` | – | Soft-delete; returns 204 |

Read/update return XML when `_format=xml` or an XML `Accept` is sent
(`application/fhir+xml; charset=utf-8`); search and delete are JSON-only.

Notable: no CodeSystem instance-level `$validate-code` route.

---

## ValueSet (14)

### Operations

| Method | Path | Body | Purpose |
|---|---|---|---|
| GET | `/ValueSet/$expand` | – | Expand by `url` (canonical, `url|version` supported) |
| POST | `/ValueSet/$expand` | Params | Expand + inline `valueSet` and full param matrix |
| GET | `/ValueSet/$validate-code` | – | Membership validation |
| POST | `/ValueSet/$validate-code` | Params | Coding / CodeableConcept / inline ValueSet |
| POST | `/ValueSet/$batch-validate-code` | Params | Bulk validation — principal `tx-resource` + repeated `validation` |
| GET | `/ValueSet/{id}/$expand` | – | Instance expansion |
| POST | `/ValueSet/{id}/$expand` | Params | Instance expansion |
| GET | `/ValueSet/{id}/$validate-code` | – | Instance validation |
| POST | `/ValueSet/{id}/$validate-code` | Params | Instance validation |

CRUD:

| Method | Path | Body | Purpose |
|---|---|---|---|
| GET | `/ValueSet` | – | Search (same params as CodeSystem search) |
| POST | `/ValueSet` | Raw resource | Create |
| GET | `/ValueSet/{id}` | – | Read |
| PUT | `/ValueSet/{id}` | Raw resource | Update |
| DELETE | `/ValueSet/{id}` | – | Soft-delete |

Read/update return XML when `_format=xml` or an XML `Accept` is sent
(`application/fhir+xml; charset=utf-8`); search and delete are JSON-only.

Notable: `POST /ValueSet/$batch-validate-code` is **code-only**, not listed
in the crate README operation tables, and not advertised in
CapabilityStatement.

---

## ConceptMap (10)

### Operations

| Method | Path | Body | Purpose |
|---|---|---|---|
| GET | `/ConceptMap/$translate` | – | Scalar `code` / `system` / `targetCode` translation |
| POST | `/ConceptMap/$translate` | Params | Coding / CodeableConcept translation, forward + reverse |
| POST | `/ConceptMap/$closure` | Params | Stateless closure edges over supplied Codings |
| GET | `/ConceptMap/{id}/$translate` | – | Instance translation |
| POST | `/ConceptMap/{id}/$translate` | Params | Instance translation |

CRUD:

| Method | Path | Body | Purpose |
|---|---|---|---|
| GET | `/ConceptMap` | – | Search |
| POST | `/ConceptMap` | Raw resource | Create |
| GET | `/ConceptMap/{id}` | – | Read |
| PUT | `/ConceptMap/{id}` | Raw resource | Update |
| DELETE | `/ConceptMap/{id}` | – | Soft-delete |

Read/update return XML when `_format=xml` or an XML `Accept` is sent
(`application/fhir+xml; charset=utf-8`); search and delete are JSON-only.

Notable: `$closure` has no GET variant and is **stateless** — prior requests
are not persisted or merged (see [SKILL.md §10](SKILL.md#10-gaps-and-known-drift-call-out-in-review)).

---

## Response type by route

| Category | Content-Type |
|---|---|
| `$expand` (success) | `application/fhir+json` (explicit) |
| Other operations, CRUD read/update | `application/json` |
| XML negotiated | `application/fhir+xml; charset=utf-8` |
| Errors | JSON `OperationOutcome` — even when XML was requested |
| `/health`, search, `/import` | JSON only |
| `/metrics` | Prometheus `text/plain` |

---

## Status-code map for operations

Standard `HtsError` → HTTP mapping applies to most 4xx/5xx responses.

| Status | Meaning in HTS |
|---:|---|
| 200 | Success. Also: `$validate-code` / `$translate` "no match" with `result=false` |
| 201 | Create returned resource + `Location` + ETag |
| 204 | Delete success. Also: HFS-side `/ui/editor/expand` degrade path |
| 207 | `/import` only: 200 = clean, 207 = partial success (non-fatal errors in `errors[]`), 400 = bad JSON / non-Bundle root, 500 = backend failure |
| 400 | `InvalidRequest`, `VsInvalid` |
| 404 | `NotFound` — unknown resource/concept/supplement |
| 408 | Request timeout (30s middleware) |
| 412 | `PreconditionFailed` — `If-Match` mismatch |
| 413 | Body over `HTS_MAX_BODY_SIZE` (post-decompression) |
| 415 | Unsupported request encoding or media type |
| 422 | `TooCostly` (expansion), cyclic reference |
| 500 | `Internal` / `StorageError` |
| 501 | `NotSupported` (e.g. `$lookup` with `expression`) |

---

## Client surface availability (workspace-internal)

Which HTS operations already have a callable client somewhere in the HFS
workspace? Confirmed by
[Terminology client inventory](bc3e226b-e68f-4dc9-ad90-1a064a63bc37).

| Operation | `helios-rest` | `helios-fhirpath` | `crates/ui` |
|---|:---:|:---:|:---:|
| `$expand` | yes (POST, canonical + inline is-a/generalizes) | yes (GET) | yes (GET proxy) |
| `$lookup` | – | yes (POST) | – |
| `$validate-code` (CS) | – | yes (POST) | – |
| `$validate-code` (VS) | yes (via validation cache) | yes (POST) | – |
| `$subsumes` | – | yes (POST) | – |
| `$translate` | – | yes (POST) | – |
| `$closure` | – | – | – |
| `$batch-validate-code` | – | – | – |
| CRUD + search | – | – | – |
| `/import` | – | – | – |

Everything without a check-mark must be added fresh when the corresponding UI
page ships. Prefer wrapping calls in a new module of the UI crate that
extends the [`crates/rest/src/terminology.rs`](../../crates/rest/src/terminology.rs)
client, not a browser-side reqwest.

---

## Recognized-but-unimplemented parameters (partial list)

Details in
[edson/docs/hts-details.md §CodeSystem APIs](../../edson/docs/hts-details.md)
and §ValueSet APIs:

- `$lookup` `expression` — 501
- `$lookup` GET `useSupplement` — string-coerced ineffective
- `$expand` `includeDefinition` — advertised in TerminologyCapabilities but
  not read by the handler
- `$expand` `excludeNotForUI`, `excludePostCoordinated`, `contextDirection` —
  not advertised and not implemented
- `$translate` dependency, `ConceptMap.version`, lowercase `targetsystem` —
  not accepted
- Search `_id`, `_sort`, chained params, modifiers, `_include`, `_elements`,
  accurate `_total`, pagination links — not implemented

Do not build UI affordances for these until the server implements them.
