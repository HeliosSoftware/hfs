# Patient `$everything` — Design

Issue: #966. Branch: `feat/966-patient-everything`.

## Goal

Implement the FHIR `Patient/$everything` operation (R4, R4B, R5, R6 share one
definition) on every storage backend that supports search, composed in the REST
layer over the existing per-type `SearchProvider::search` and the query-time
compartment predicate that `GET /Patient/{id}/*` already uses.

## Decision: compose in REST (Approach A)

Two shapes were considered:

- **A — compose in REST** over `SearchProvider` with a REST-owned cross-type
  cursor. No persistence changes.
- **B — push down** a new `EverythingProvider` trait with a native cross-type
  query per backend.

A was chosen because it is the repo's established shape for multi-resource
reads: compartment search (`crates/rest/src/handlers/compartment.rs`) and
chained search / `_has` (`crates/persistence/src/search/chain_resolver.rs`) both
loop plain per-type searches, and a native `ChainedSearchProvider` was built and
then deliberately left unwired (`crates/persistence/README.md`, "The native
`ChainedSearchProvider` trait is not wired into request handling"). Native
per-backend traits are reserved for storage-shape-specific work (`$export`,
`$bulk-submit`, `$reindex`, `$purge`, history, SOF runs).

If the per-type fan-out proves too slow on large corpora, the follow-up is the
SOF pattern — a native implementation per backend with the composed path as the
generic fallback — behind the same HTTP contract. Not a rewrite.

Bulk export's `PatientExportProvider` is explicitly **not** reused: its four
implementations hardcode only `subject.reference` / `patient.reference` off the
raw payload, which is narrower than the compartment, and Elasticsearch has no
implementation. That divergence is a separate issue.

## 1. HTTP contract

Routes (registered in `crates/rest/src/routing/fhir_routes.rs` before the CRUD
catch-alls, alongside `$validate` / `$purge`):

| Route | Handler |
|---|---|
| `GET\|POST /Patient/{id}/$everything` | `patient_everything_instance_handler` |
| `GET\|POST /Patient/$everything` | `patient_everything_type_handler` |

Extractors: `TenantExtractor`, `FhirVersionExtractor`, raw `Request` (method
branching), `Principal` from request extensions. POST takes a `Parameters`
body; GET takes query parameters. Both produce identical output.

Inputs (R4/R5 definition, identical set):

| Name | Type | Card. | Meaning |
|---|---|---|---|
| `start` | date | 0..1 | Lower bound on *care* dates (not `lastUpdated`) |
| `end` | date | 0..1 | Upper bound on care dates |
| `_since` | instant | 0..1 | Only resources with `meta.lastUpdated >= _since` |
| `_type` | code | 0..* | Comma-delimited resource types; repeatable |
| `_count` | integer | 0..1 | Page size; absent → single unpaged bundle |
| `_cursor` | string | 0..1 | Opaque paging token (server-issued) |

Unknown parameters → 400 `OperationOutcome`. Unknown `_type` name → 400.

Output: `searchset` Bundle built with the existing
`SearchResult::into_bundle`. `self` and `next` links; no `previous`.
`total` is set only on unpaged responses (where it is exact) and omitted when
paging. Entries carry `search.mode = match`, except supporting resources
(section 2, tier 3) which carry `search.mode = include`.

Errors: unknown Patient → 404; deleted Patient → 410; backend without
`BasicSearch` (S3 standalone) → 501 `UnsupportedCapability`, the same path
search takes today.

CapabilityStatement: `everything` is declared under
`rest.resource[Patient].operation` in `build_resource_capability`
(`crates/rest/src/handlers/capabilities.rs`), next to the per-resource
`$export` entry.

## 2. Scope resolution

Three tiers, resolved in order per patient.

**Tier 1 — the Patient.** Read by id. Always first. `_type` cannot exclude
it. `_since`, `start`, `end` do not apply.

**Tier 2 — compartment members.** Every resource type for which
`helios_fhir::get_compartment_params(version, "Patient", type)` is non-empty,
in the compartment table's order, intersected with `_type` when given. Each
type is searched with
`SearchQuery.compartment = CompartmentMembership { params, reference: "Patient/{id}" }`,
so the membership predicate is pushed down to each backend's existing query
builder. This is the same table `compartment_search_all` uses; `$everything`
and `/Patient/{id}/*` cannot disagree on membership.

Filters (tier 2 only):

- `_since` → `_lastUpdated=ge{instant}` on every per-type query.
- `start` / `end` → `ge{start}` / `le{end}` on the type's clinical date search
  parameter when it has one (`Observation.date`, `Encounter.date`,
  `Procedure.date`, `Condition.onset-date`, `MedicationRequest.authoredon`,
  …). Types with no clinical date parameter pass through unfiltered: the spec
  ties the range to care dates, and a resource without one is not excluded
  by it. The mapping is a single per-version table in the handler module.

**Tier 3 — supporting resources.** After each page of tier-2 results, walk
the returned resources' references and collect targets whose type is *not* a
Patient-compartment member (Practitioner, PractitionerRole, Organization,
Location, Medication, Substance, Device, HealthcareService, Endpoint,
Questionnaire, …). Fetch by `ResourceStorage::read`, dedup by `type/id`
within the page, emit as `include`. Compartment membership is the
discriminator, not a hand-maintained list: anything in tier 2 is skipped
because it appears as a `match`. One hop; no transitive follow. `_since`,
`start`, `end` do not apply — a supporting resource is included because a
matched resource references it.

## 3. Paging and the cross-type cursor

The walk is a fixed sequence of *segments*:
`[Patient, member_type_0, member_type_1, …]`, filtered by `_type`. A page
fills from the current segment; when a segment is exhausted the walk advances
to the next until `_count` match entries are collected or the sequence ends.

Cursor — REST-owned, opaque, base64 URL-safe JSON:

```json
{ "v": 1, "seg": 12, "inner": "<backend PageCursor or null>", "fp": "<hash>" }
```

- `v` — token version.
- `seg` — index into the segment sequence.
- `inner` — the backend's own `PageCursor` for the current segment's per-type
  search, passed through verbatim. REST never decodes it.
- `fp` — fingerprint of the scope inputs (patient id, `_type`, `_since`,
  `start`, `end`, `_count`). A cursor presented with different inputs → 400.

`next` is emitted while any segment still has results. Links are built with
the existing `replace_cursor_param` on the self link. Within a segment the
order is the backend's default sort (`lastUpdated desc, id` on every backend
today). Page boundaries may fall mid-segment; `inner` resumes exactly there.

Supporting resources are resolved per page from that page's matches and
dedup'd within the page only. A resource referenced from two pages appears in
both — the same behaviour `_include` has, and spec-legal because `include`
entries do not count against `_count`.

**Unpaged default.** No `_count` → every segment is drained into one bundle,
per spec. A server ceiling `HFS_EVERYTHING_MAX_UNPAGED` (default `10000`
match entries) guards large compartments: when reached, the response switches
to paged mode at that size, carries a `next` link, and includes an
`OperationOutcome` entry with severity `information` stating that the result
was paged. Clients that want the whole set follow `next`.

## 4. Type-level walk, auth, backend matrix

**Type-level `Patient/$everything`.** The patient set is a Patient search
(no filters, backend default sort) — every patient the caller can see, which
is the spec's definition when no patient is nominated. The walk is an outer
loop over patients with the section-3 segment sequence per patient. The cursor
gains two fields:

```json
{ "v": 1, "pat": "<Patient search PageCursor>", "pid": "<current patient id>",
  "seg": 12, "inner": "...", "fp": "..." }
```

`pat` resumes the outer Patient page; `pid` pins the patient currently being
walked so a page boundary mid-patient resumes correctly. The unpaged ceiling
applies to the whole response, so type-level without `_count` on a large
tenant flips to paging immediately — expected.

**Auth.** SMART scopes are enforced where they are today: `authz_middleware`
(`crates/rest/src/middleware/auth.rs`) checks the path's resource type and
operation before the handler runs. `Principal` carries no SMART `patient`
launch context, so type-level `$everything` does not narrow to a patient
context; that is deferred until launch context exists. No new scope
semantics.

**Audit.** The existing `audit_middleware` records one `AuditEvent` per
request. The handler attaches `helios_audit::AuditResponseContext` with
`resource_type = "Patient"`, `resource_id` and `patient_reference` so the
event names the patient — the same enrichment `create`/`delete` do. Not one
event per member type.

**Backend matrix.** The handler is generic over
`S: ResourceStorage + SearchProvider`; support falls out of capabilities.

| Backend | Support | Note |
|---|---|---|
| SQLite, PostgreSQL, MongoDB, Elasticsearch | Full | compartment predicate already in each query builder |
| Composite | Full | search routes to the search-capable member, reads to the primary — as compartment `*` does today |
| S3 standalone | 501 `UnsupportedCapability` | consistent with all other search on S3 |

## 5. Testing

**Unit** (`crates/rest/src/handlers/everything.rs`, no storage):

- Cursor round-trip; fingerprint mismatch → 400; unknown version / tampered
  token → 400.
- Segment sequencing is deterministic per FHIR version and `_type`; unknown
  `_type` → 400.
- Clinical-date mapping resolves the expected parameter per type and version;
  unmapped types produce no filter.
- Supporting-resource discriminator: compartment-member references skipped,
  non-member references collected, duplicates within a page collapsed.

**Router integration** (`crates/rest/tests/patient_everything.rs`, SQLite
in-memory, real Axum router — the `compartment_definition_endpoint.rs`
harness). Seed one patient with resources across ~6 member types plus a
referenced Practitioner and Organization, and a control patient. Assert:

- instance-level: Patient first, all member resources as `match`,
  Practitioner/Organization as `include`, nothing from the control patient;
- `_type=Observation,Encounter` restricts tier 2 only; Patient still present;
- `_since` excludes older resources; `start`/`end` filter Observation by
  `date` and leave a date-less type untouched;
- `_count=3` yields `next`; following it to exhaustion reproduces exactly the
  unpaged set — no duplicates, no gaps — including across a segment boundary;
- unpaged ceiling set to 5 via env flips to paging with the `information`
  outcome;
- type-level walks both patients and resumes correctly mid-patient across a
  page boundary;
- unknown patient → 404; GET and POST `Parameters` produce identical bundles;
- CapabilityStatement lists `everything` under Patient.

**Backend integration** (testcontainers, gated as the existing suites are):
REST-level tests for PostgreSQL and MongoDB in `crates/rest/tests/`
(fixtures copied from `sof_conformance_postgres.rs` and
`mongodb_include_iterate.rs`), and a persistence-level Elasticsearch test
that pins compartment predicate + `_lastUpdated` + cursor pass-through, the
three primitives the handler composes. Same seed, instance-level with
`_count=2`, walk to exhaustion, compare the id set with SQLite's. S3
standalone 501 is exercised by the existing `From<BackendError>` → 501
mapping and verified manually (`HFS_STORAGE_BACKEND=s3`) — there is no
REST-level MinIO fixture today.

**Out of scope for tests:** performance on the 11M-resource corpus. That is a
manual-matrix row after shipping and the trigger for the native-path
follow-up.

## Non-goals

- `previous` links.
- Transitive (multi-hop) supporting-resource resolution.
- Reworking `PatientExportProvider` to use the compartment table.
- A native per-backend implementation (deferred; see Decision).
- Encounter / EpisodeOfCare `$everything`.
