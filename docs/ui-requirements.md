# Helios FHIR Server — Web UI Requirements

**Status:** Draft v1 · **Date:** 2026-06-01 · **Owner:** Helios Software
**Target binary:** `hfs` (the Helios FHIR Server) only
**Purpose of this document:** Capture, comprehensively, every capability the
`hfs` server exposes so that they can be driven through a web-based user
interface. This document is the input to the next step — generating wireframes
in Claude Design. It is written to be design-ready: it enumerates *what the
server can do*, *who needs to do it*, and *what the UI must surface* to make
each capability usable.

---

## 1. Overview

The Helios FHIR Server (`hfs`) is a multi-version, multi-tenant FHIR R4/R4B/R5/R6
server with a full RESTful API, advanced search, conditional operations,
versioning, batch/transaction processing, asynchronous Bulk Data Export, optional
SMART-on-FHIR authentication, and pluggable storage backends. Today it is
operated entirely over HTTP and configured through environment variables. There
is no graphical interface.

This project adds a **web UI** that makes the server's capabilities usable
directly from the browser, serving four distinct user types (see §3). The UI is a
client of the same HTTP API external integrators use; it must not require any
capability the server does not expose over HTTP.

### 1.1 Reference specifications

The UI and this document follow the published FHIR specifications. Designers and
implementers should reference them for resource shapes, search semantics, and
operation contracts:

- FHIR (all versions): https://hl7.org/fhir/
- RESTful API: https://hl7.org/fhir/http.html
- Search: https://hl7.org/fhir/search.html
- Bundle / transaction: https://hl7.org/fhir/bundle.html, https://hl7.org/fhir/http.html#transaction
- Bulk Data Access IG ($export): https://hl7.org/fhir/uv/bulkdata/
- SMART on FHIR: https://hl7.org/fhir/smart-app-launch/
- SQL-on-FHIR (ViewDefinition): https://sql-on-fhir.org/ig/latest/
- CapabilityStatement: https://hl7.org/fhir/capabilitystatement.html

---

## 2. Goals and non-goals

### 2.1 Goals
- Provide a browser UI that exposes the **full** RESTful, search, history,
  bulk-export, transaction, multi-tenancy, terminology, and SQL-on-FHIR surface
  of the `hfs` server.
- Support **full read-write** workflows: create, read, update, patch, delete,
  conditional operations, and batch/transaction bundles.
- Be **persona-aware**: surface technical detail (raw JSON/XML, query strings,
  capability inspection) to developers and admins, while offering
  friendlier, form- and table-driven flows for clinical and analyst users.
- Be **version-aware** (R4/R4B/R5/R6) and **tenant-aware** in every screen.
- Degrade gracefully when optional capabilities (auth, terminology server,
  XML, subscriptions, specific storage backends) are disabled.

### 2.2 Non-goals
- The UI does not implement FHIR logic itself; it is a thin client of the
  `hfs` HTTP API.
- The UI does not configure the server's environment variables at runtime
  (configuration remains deployment-time), but it **reads** server configuration
  and capabilities to adapt itself (see §4).
- Standalone UIs for the separate `sof-server` or `hts` binaries are out of
  scope. Terminology and SQL-on-FHIR are surfaced only as they relate to `hfs`
  (see §11, §13).

---

## 3. User personas

The UI serves four personas. A given deployment may grant a user one or more
roles. The design SHOULD support a unified application whose density and
vocabulary adapt to the active persona/role rather than four separate apps.

| # | Persona | Mental model | Primary needs |
|---|---------|--------------|---------------|
| P1 | **FHIR developer / integrator** | "Postman for FHIR" / API console | Raw resource view/edit (JSON & XML), hand-built and assisted search queries, capability/metadata inspection, request/response transparency (headers, status, ETags), batch/transaction authoring, content-negotiation control. |
| P2 | **Server administrator / ops** | Operational console | Tenant selection & isolation, health/liveness/readiness, capability statement, bulk-export job monitoring & cancellation, audit visibility, server/version/backend awareness, auth/SMART status. |
| P3 | **Clinical / data end-user** | Record browser | Find patients and related records via friendly forms; browse resources in readable form (not raw JSON); navigate references and compartments; create/edit through guided forms. |
| P4 | **Data analyst** | Query & extract tool | Powerful search & filtering, tabular results, SQL-on-FHIR ViewDefinition runs, and Bulk Data Export to NDJSON for downstream analysis. |

Design implication: most screens have a **"raw" mode** (P1/P2) and a
**"friendly" mode** (P3/P4). Both modes are backed by the same API calls.

---

## 4. Cross-cutting / global requirements

These apply to the whole application and frame every screen.

### 4.1 Server & capability discovery
- On load, the UI MUST fetch `GET /metadata` (CapabilityStatement) and
  `GET /$versions` to discover: supported FHIR versions, default version,
  enabled resource types and their interactions, advertised search parameters,
  enabled operations (e.g. `$export`), supported formats (JSON, and XML iff
  enabled), and security/SMART configuration.
- The UI MUST adapt to what the server advertises: hide interactions the server
  does not list, show only enabled resource types, expose only advertised search
  parameters by default, and reflect `conditionalCreate/Update/Delete`,
  `updateCreate`, `versioning`, and `searchInclude`/`searchRevInclude` support.
  (Exception: the current server's CapabilityStatement does **not** advertise
  resource-specific search parameters or modifiers — see §7.9 — so the Search
  Builder catalog must be sourced from the bundled FHIR `SearchParameter`
  definitions rather than `/metadata`.)
- The UI MUST surface `GET /health`, `GET /_liveness`, `GET /_readiness` status
  (primarily for P2) — e.g. a status indicator with backend name and timestamp.

### 4.2 FHIR version selection
- A global control lets the user pick the active FHIR version among those the
  server enables (R4 / R4B / R5 / R6). The default comes from `$versions`.
- The selected version MUST flow into requests via the `fhirVersion` media-type
  parameter (`Accept: application/fhir+json; fhirVersion=4.0`) and/or `_format`.
- Resource type lists, search parameters, and forms MUST reflect the active
  version (these differ across versions).

### 4.3 Multi-tenancy
- A global tenant selector MUST be present. Behavior depends on the server's
  routing mode (`header_only`, `url_path`, `both`):
  - **header_only:** send `X-Tenant-ID: <tenant>` on every request.
  - **url_path:** prefix request paths with `/<tenant>`.
  - **both:** send both; respect server's strict-validation behavior.
- All data views (search, browse, history, export, transactions) are scoped to
  the active tenant. Switching tenant MUST re-scope the whole UI and never leak
  data across tenants.
- Where the server returns tenant-aware base URLs (in CapabilityStatement and
  Bundle links), the UI MUST use them for follow-up navigation.

### 4.4 Authentication & authorization (SMART on FHIR)
- The UI MUST read `GET /.well-known/smart-configuration` to detect whether auth
  is enabled and to obtain `authorization_endpoint`, `token_endpoint`,
  `scopes_supported`, and capabilities.
- When auth is enabled, the UI MUST support a bearer-token / SMART authorization
  flow and attach `Authorization: Bearer <token>` to requests. When disabled,
  the UI operates unauthenticated.
- The UI MUST handle `401` (login required) and `403` (insufficient SMART scope)
  gracefully, explaining which scope is missing where possible, and SHOULD hide
  or disable actions the current scopes do not permit (e.g. write actions under a
  read-only scope, per-resource-type scopes like `patient/Observation.read`).
- Exempt endpoints (`/metadata`, `/health`, `/_liveness`, `/_readiness`,
  `/.well-known/smart-configuration`, `/$versions`) are reachable without auth.

### 4.5 Content negotiation & format control
- The UI defaults to `application/fhir+json`. When the server advertises XML
  (`feature=xml`), a per-view toggle MAY let P1 users view/edit
  `application/fhir+xml`.
- A developer-facing control SHOULD allow setting `_format` and `Accept`
  explicitly. NDJSON is used by Bulk Export (§8).
- The UI MUST display the negotiated `Content-Type` (including `fhirVersion`).

### 4.6 Request/response transparency (developer mode)
- For P1/P2, the UI SHOULD expose, per request: method, full URL (incl. query
  string), request headers/body, response status, response headers (notably
  `ETag`, `Location`, `Content-Location`, `Last-Modified`), and timing.
- A "copy as cURL" affordance SHOULD be available for any request the UI makes.

### 4.7 Prefer header control
- Write screens SHOULD let advanced users choose `Prefer: return=minimal |
  representation | OperationOutcome` and `Prefer: handling=strict | lenient`.
  Sensible defaults: `return=representation`, `handling=lenient`.

### 4.8 Error handling
- All server errors are returned as FHIR `OperationOutcome`. The UI MUST parse
  and present issues legibly: severity, code, human-readable details, and the
  FHIRPath `expression` (path) when present.
- The UI MUST map and explain key HTTP statuses meaningfully: 400 invalid,
  401 unauthorized, 403 forbidden (scope), 404 not-found, 405 not-allowed,
  406/415 negotiation, 409/412 version/precondition conflicts and
  multiple-matches, 410 Gone (deleted), 422 unprocessable, 501 not-implemented,
  500 exception.

### 4.9 Responsiveness, accessibility, theming
- Desktop-first (P1/P2 are power users) but responsive to tablet.
- WCAG 2.1 AA: keyboard navigation, focus order, ARIA on data tables and forms,
  sufficient contrast. Light/dark themes.
- Long-running and large-payload operations MUST show progress and never block
  the UI thread.

---

## 5. Information architecture (proposed top-level navigation)

The wireframe should organize the application around these areas. Each maps to
server capabilities detailed in §6–§13.

1. **Dashboard / Home** — server identity, version(s), active tenant, health,
   backend, enabled capabilities at a glance.
2. **Explore / Resources** — per-resource-type browse, search, and detail.
3. **Search Builder** — assisted construction of FHIR search queries.
4. **Resource Editor** — create / update / patch / delete a resource (JSON, XML,
   or guided form).
5. **History & Versions** — instance/type/system history and version read.
6. **Compartments** — patient-centric (and other) compartment navigation.
7. **Batch / Transaction** — author and submit Bundles.
8. **Bulk Export** — kick off, monitor, download, cancel `$export` jobs.
9. **SQL-on-FHIR** — author/run ViewDefinitions, view/export tabular output.
10. **Capability & Conformance** — CapabilityStatement, `$versions`,
    SMART configuration, search-parameter catalog.
11. **Admin / Ops** — tenants, health, auth/SMART status, audit, (read-only)
    server configuration & storage backend info.
12. **Terminology** (Conditional link-out) — present only when
    `HFS_TERMINOLOGY_SERVER` is configured.

---

## 6. RESTful resource interactions (CRUD)

The server exposes the standard FHIR REST interactions. The editor and detail
screens MUST support all of them.

### 6.1 Read
- `GET /{type}/{id}` — read current version. Render in friendly form (P3) and
  raw JSON/XML (P1). Show `ETag`, `Last-Modified`.
- `HEAD /{type}/{id}` — headers only (developer utility).
- Conditional read: support `If-None-Match` (ETag) and `If-Modified-Since`;
  surface `304 Not Modified` (e.g. "unchanged since last load").
- Deleted resources return `410 Gone` — the UI MUST present this distinctly from
  `404` (e.g. "this resource was deleted") and offer to view history / re-create.
- Subsetting: support `_summary` (`true|false|text|data|count`) and `_elements`
  on read; show a "SUBSETTED" indicator when the server tags the result.

### 6.2 Create
- `POST /{type}` — create with server-assigned id. Show resulting `201 Created`
  with `Location` and `ETag`.
- **Conditional create** via `If-None-Exist: <search params>` — the UI MUST let
  users express the uniqueness query; explain "created" vs "matched existing".
- Note: `AuditEvent` is immutable and cannot be created/updated/deleted through
  these interactions — the UI MUST disable write actions for it.

### 6.3 Update
- `PUT /{type}/{id}` — update or create (updateCreate/upsert). Show `200` vs
  `201`.
- **Optimistic locking** via `If-Match: W/"<versionId>"` — the editor SHOULD send
  the version it loaded and handle `409 Conflict` / `412 Precondition Failed`
  with a clear "edited concurrently — reload/merge" flow.
- **Conditional update** via `PUT /{type}?<search params>`; handle `412` on
  multiple matches.

### 6.4 Patch
- `PATCH /{type}/{id}` with selectable patch format:
  - JSON Patch (RFC 6902) — `application/json-patch+json`
  - JSON Merge Patch (RFC 7386) — `application/merge-patch+json`
  - FHIRPath Patch — `application/fhir+json` with a `Parameters` resource
- The UI SHOULD provide an editor appropriate to each (e.g. add/remove/replace
  op builder for JSON Patch; a diff/merge editor for Merge Patch). Support
  `If-Match`.

### 6.5 Delete
- `DELETE /{type}/{id}` — soft delete; history preserved. Show `204`/`200`.
- **Conditional delete** via `DELETE /{type}?<search params>` (single-match
  per capability). The UI MUST confirm destructive actions and show what matched.

### 6.6 Response/Prefer handling
- Honor §4.7 `Prefer` controls; when `return=minimal`, present a confirmation
  with headers only; when `return=OperationOutcome`, show the outcome.

---

## 7. Search

Search is central for P1, P3, and P4. Two entry points: the **Search Builder**
(assisted) and a **raw query** input (developer). Both target
`GET /{type}?...` and `POST /{type}/_search` (form-encoded).

### 7.1 Parameter types
Support building and editing all FHIR search parameter types `hfs` implements:
**token, string, reference, date, quantity, number, uri, composite, special**.
The common cross-resource parameters `hfs` indexes are `_id` (token),
`_lastUpdated` (date), `_tag` (token), `_security` (token), `_profile` (uri), and
`_source` (uri). The full-text parameters `_text` and `_content` are implemented
**locally on the default SQLite backend** via FTS5 — they do **not** require
Elasticsearch and do **not** return `501` when ES is absent. (The graceful
`501 Not Implemented` fallback for unavailable text search exists **only on the
MongoDB backend**; the SQLite path has **no** no-FTS5 guard, so on a SQLite build
compiled without FTS5 these queries hit a missing-table error rather than
degrading to a clean empty result. Bundled SQLite normally ships with FTS5, so
this is an edge case — but do not rely on "matches nothing" as the failure mode.
Their spec type is `special`, though the CapabilityStatement currently advertises
them as `string` — see §7.9.) `_filter` is also implemented on the SQLite backend
(an undocumented, unadvertised extra; note a `_filter` that fails to parse is
**silently dropped** with only a logged warning, not rejected). `hfs` does **not**
implement `_query`, `_list`, `_contained` / `_containedType`, or `_score` — but
note it **silently ignores** them (returns an unfiltered result, even under
`Prefer: handling=strict`) rather than rejecting them. The builder MUST NOT offer
the unimplemented params, and MUST NOT rely on the server to reject them.

- **Value escaping (gap).** FHIR uses a backslash to escape the special
  characters `, | $ \` inside a search value. `hfs` performs **no** unescaping, so
  a literal comma always splits OR-values and a literal `|` always splits a token
  into `system|code`. The Search Builder MUST handle escaping itself and SHOULD
  warn when a user-entered token system URL or string value contains an unescaped
  special character, because the server will otherwise mis-split it (see §18).

### 7.2 Modifiers
Expose type-appropriate modifiers in the builder. The groupings below reflect
what `hfs` actually parses and validates (`SearchModifier::is_valid_for` in
`crates/persistence/src/types/search_params.rs`). As of the search-modifier
spec-compliance work, these now align with the published FHIR spec except for the
few items called out under "Deviations" below.
- **All types:** `:missing`
- **String:** `:exact`, `:contains`, `:text`, `:text-advanced`
- **Token:** `:not`, `:text`, `:in`, `:above`, `:below`, `:of-type`, `:code`,
  `:code-text`, `:text-advanced`
- **Reference:** `:identifier`, `:[Type]` (target type), `:contains`, `:text`,
  `:above`, `:below`, `:code-text`
- **Uri:** `:contains`, `:above`, `:below`
- `_include` / `_revinclude` modifier: `:iterate`
- **Not supported — do not offer (or clearly mark unavailable):** `:not-in` is
  parsed (token only) but **rejected at runtime with `501 Not Implemented`**
  (negated value-set filtering is unimplemented). There is **no** `:code-only`
  modifier — only `:code`.
- Note: the text/terminology modifiers depend on a backend that can satisfy
  them. `:text-advanced` (token) uses the SQLite FTS5 full-text index; `:text`
  and `:code-text` match against indexed display text and also run on the default
  backend. The terminology-dependent token modifiers — `:in`, and `:above`/
  `:below` **against code hierarchies** — **delegate to a terminology server**
  (see §11) only when one is configured. (Reference and uri `:above`/`:below` are
  resolved locally via URL/path-prefix hierarchy and need no terminology server.)
  **Important:** when no terminology server is configured, token `:in`/`:above`/
  `:below` are **silently ignored** — the server does NOT return `501`; it falls
  through and still returns `200`. The exact mechanism is messy: `:in` is searched
  as the ValueSet *URL treated as a literal code* (so it matches nothing in
  practice), and the SQLite token handler has **no** `:above`/`:below` branch at
  all, so those effectively no-op / degrade to roughly exact-code matching. A
  failed terminology `$expand` (server configured but the call errors) likewise
  **fails open** — the parameter is dropped and the search continues unfiltered.
  Only `:not-in` returns
  `501` (always — see below). The UI therefore MUST gate these modifiers on
  *configuration detection* (§11), not on catching a `501`, and SHOULD indicate
  when a terminology server is required and whether one is configured.
- **Advertised vs. validated — caution:** the served `GET /metadata` does **not**
  advertise modifiers at all, and lists no resource-specific search params. The
  REST CapabilityStatement (`crates/rest/src/handlers/capabilities.rs`) is static:
  for *every* resource type it returns only the seven common search params with no
  `modifier` field. The richer per-type modifier set (`modifiers_for_type` in the
  SQLite backend) is computed but **never wired into the HTTP response**.
  Consequently the UI CANNOT source its search-parameter or modifier catalog from
  `/metadata` (contra §4.1/§12); it must drive the Search Builder from the bundled
  FHIR `SearchParameter` definitions and the modifier/prefix groupings in this
  section instead. See §7.9 for the full advertisement gap.
- **`:of-type` spelling:** the parser accepts **both** `:of-type` (the spec /
  build.fhir.org spelling) and the legacy R4 `:ofType`, and the
  CapabilityStatement advertises `:of-type`. The UI SHOULD emit `:of-type`.
- **Deviations from the FHIR spec** (the UI should follow `hfs`, but designers
  should be aware — every other grouping above now matches the spec):
  - `:code` — `hfs` exposes a `:code` token modifier; the published spec defines
    no `:code` modifier (its token modifiers are `:text`, `:not`, `:above`,
    `:below`, `:in`, `:not-in`, `:of-type`, `:identifier`, `:code-text`,
    `:text-advanced`). Treat `:code` as an `hfs` extension.
  - `:contains` — `hfs` accepts and executes it on **reference** and **uri** as
    well as string; the spec defines `:contains` for **string** only.
  - `:text-advanced` — `hfs` accepts it on **string** or **token**; the spec
    defines it for **reference** and **token**. `hfs` thus deviates in *both*
    directions (wrongly accepts it on string, wrongly rejects it on reference), so
    the UI SHOULD offer `:text-advanced` on **token only** — the safe intersection
    of spec and `hfs`.
  - `:not-in` — the spec restricts it to **token** (as `hfs` does), but `hfs`
    returns `501` for it regardless (see above).

### 7.3 Prefixes
For number/date/quantity values, expose prefixes: `eq, ne, gt, lt, ge, le, sa,
eb, ap`. Per the FHIR spec these apply to any ordered type (number, date,
quantity), and `sa`/`eb` (starts-after / ends-before) are likewise spec-valid on
all three. The builder SHOULD therefore scope prefixes by parameter type
client-side (e.g. no prefixes on string/token/uri params).
- **Caution — `hfs` does NO prefix validation.** `SearchPrefix::is_valid_for`
  exists but is **never invoked on the request path** (only in unit tests). The
  server therefore neither rejects a nonsensical prefix (e.g. `name=gt2020` on a
  string param is *not* a `400`) nor enforces any type-scoping. In particular
  `sa`/`eb` on number/quantity are **not** rejected and actually **execute** — the
  number/quantity handlers implement them. (The internal `is_valid_for` table
  still restricts `sa`/`eb` to date, but that table is dead code today, so it has
  no runtime effect.)
- **Implication for the UI:** do your own prefix/type scoping client-side and do
  **not** rely on the server to reject malformed prefix usage; a raw query that
  misuses a prefix will generally return `200` (with the prefix either applied or
  ignored), not an error. See §18.

### 7.4 Result controls
- `_count` (default 20, server max 1000), pagination via `_offset`
  (offset-based) and `_cursor` (opaque, server-proprietary keyset cursor; used in
  `next`/`previous` Bundle links — the UI MUST follow these links rather than
  reconstruct them). Note `_cursor` works only for single-field sorts; a
  multi-field `_sort` disables cursor paging and the result comes back as a single
  page with no `next` link.
- **Pagination links (gap):** search Bundles carry `self`, plus `next`/`previous`
  when applicable, but **never `first` or `last`**. The UI MUST drive paging from
  `next`/`previous` only and SHOULD NOT offer jump-to-first/last page controls.
- `_sort` (comma-separated; `-` prefix = descending; multi-field supported).
  **Caution:** an unsupported sort field (composite/special/unresolved) is
  **silently ignored** — the server falls back to sorting by `id` with no error or
  warning. The UI MUST NOT assume a requested sort was honored; only `_id`,
  `_lastUpdated`, and indexed typed params sort reliably.
- `_total` (`accurate|estimate|none`) — show `Bundle.total` when present. Note
  `estimate` currently runs the same exact `COUNT(*)` as `accurate` (no cheaper
  estimate path), so it carries the same cost.
- `_summary` and `_elements` for subsetting result entries.

### 7.5 Includes
- `_include` and `_revinclude`, including wildcard `*` and `:iterate`.
- Included resources arrive as Bundle entries with `search.mode=include`; the UI
  MUST visually distinguish match vs include entries.

### 7.6 Chained & reverse-chained search
- Forward chaining (`subject.name=...`) and `_has` reverse chaining (including
  nested `_has`) are supported. `hfs` resolves them application-side via
  iterative searches (rewritten as an `_id` filter), so they work uniformly
  across all backends rather than being backend-dependent. **Depth limits:**
  reverse `_has` is capped at 4 (exceeding it returns a `400`-class parse error),
  but this cap is **hardcoded, not configurable** by any env var. Forward chaining
  currently has **no depth cap** in the active resolver. The builder SHOULD allow
  composing chains and `_has`; there is no configurable forward limit to surface,
  and the reverse limit cannot be changed by deployment.

### 7.7 Results presentation
- **P1:** raw Bundle (JSON/XML), with each entry's `fullUrl`, `search.mode`,
  and response metadata.
- **P3/P4:** sortable, paginated **table** with column selection (driven by
  `_elements`), row → detail navigation, and reference links that navigate to
  the referenced resource.
- Show the exact query string used, with "copy as cURL" and "open as raw query".
- `_summary=count` SHOULD render as a count-only result.
- Paging controls bind to the Bundle's `next`/`previous` links only; `first` and
  `last` links are not emitted by the server (§7.4), so omit those affordances.

### 7.8 Compartment search
- `GET /{compartmentType}/{id}/{targetType}` (e.g. `Patient/123/Observation`).
  Supported compartments: Patient, Encounter, Practitioner, Device,
  RelatedPerson. The all-types form `/{compartmentType}/{id}/*` is **not
  implemented** (returns `400`).
- Surface in §9 Compartments and as a "related resources" affordance on a
  resource detail view; the server validates compartment membership.
- **Caution — under-inclusive:** for target types that belong to a compartment
  via multiple reference params (e.g. AllergyIntolerance via
  `patient`/`recorder`/`asserter`), `hfs` applies only the **first** membership
  param, not the spec's OR across all of them — some legitimately-in-compartment
  resources may be missed. The UI SHOULD NOT present compartment results as
  exhaustive for such types.

### 7.9 CapabilityStatement advertisement gap (discovery caveat)
The served `GET /metadata` is **static** and identical for every resource type
(`crates/rest/src/handlers/capabilities.rs`):
- `searchParam` lists only the seven common params (`_id`, `_lastUpdated`,
  `_tag`, `_profile`, `_security`, `_text`, `_content`) — **no resource-specific
  search params and no `modifier` field** on any param.
- `searchInclude` / `searchRevInclude` are advertised as `["*"]` unconditionally,
  not validated against real target types.
- `_text` / `_content` are advertised as type `string` (the spec type is
  `special`).

The backend computes far richer per-type capabilities (hundreds of loaded FHIR
`SearchParameter` definitions; a per-type modifier set via `modifiers_for_type`),
but **none of it reaches the HTTP CapabilityStatement**. Implication for the UI:
the §4.1/§12 guidance to "expose only advertised search parameters" and build a
"search-parameter catalog per resource type" from `/metadata` **cannot** be
satisfied by the current server — a `/metadata`-driven catalog would show seven
params and no modifiers for every type. Until the server advertises real params,
the Search Builder MUST source its parameter and modifier catalog from the
bundled FHIR `SearchParameter` definitions (and the groupings in §7.2–§7.3),
treating those as the source of truth. This should be tracked as a server-side
gap (see §18).

---

## 8. History & versioning

### 8.1 Versioning model
- All resources are versioned; `meta.versionId` drives weak ETags
  (`W/"<versionId>"`). The UI MUST display the current version and use it for
  optimistic locking on write.

### 8.2 History interactions

History is implemented end-to-end. Every version is written to a
`resource_history` table; the backends implement the history-provider traits
(`InstanceHistoryProvider → TypeHistoryProvider → SystemHistoryProvider`) with
working retrieval, pagination, and `_since` filtering; and the REST read
handlers are now wired to those providers (`CompositeStorage` delegates type/
system history to its primary, so the `-elasticsearch` variants work too).

Per-endpoint behavior:

| Endpoint | HTTP method | Behavior |
|----------|-------------|----------|
| Instance history `/{type}/{id}/_history` | GET | Returns a `type: history` Bundle (404 if the resource never existed) |
| Type history `/{type}/_history` | GET | `type: history` Bundle across the type |
| System history `/_history` | GET | `type: history` Bundle across all types |
| Version read (vread) `/{type}/{id}/_history/{vid}` | GET | Returns the resource at that version (404 if the version is unknown) |
| Delete instance history `/{type}/{id}/_history` (R6 Trial Use) | DELETE | Deletes the instance's history |
| Delete a version `/{type}/{id}/_history/{vid}` (R6 Trial Use) | DELETE | Deletes a single version |

Supported query params on the read endpoints: `_count` (capped at the server
max page size) and `_since` (an RFC3339 instant; a malformed value returns
`400`). Each history Bundle entry carries `request` (method + url), `response`
(`status`, weak `etag`, `lastModified`), and — for non-delete versions — the
resource body.

- Design implication: the wireframe SHOULD include a **History & Versions**
  screen as a real feature:
  - a chronological list of an instance's versions (from instance history),
  - version-read (vread) detail for any past version,
  - diff-between-versions,
  - type- and system-level history feeds (P1/P2), and
  - the R6 delete-history / delete-version actions behind destructive-action
    confirmation.
- The CapabilityStatement advertises `readHistory`, the `vread`,
  `history-instance`, `history-type`, and `history-system` interactions, and
  these now match runtime behavior.

---

## 9. Compartments

- A patient-centric (and general compartment) navigation: given a resource
  (e.g. a Patient), browse all related resources by target type via
  `/{compartmentType}/{id}/{targetType}`.
- Primary value to P3 (clinical record view) and P4 (scoped extraction).

---

## 10. Batch & transaction

- `POST /` with a Bundle of `type: batch` or `type: transaction`.
- **Batch:** entries processed independently; partial success allowed.
- **Transaction:** atomic all-or-nothing (ACID on SQLite/PostgreSQL; eventual on
  MongoDB — the UI MAY note backend-dependent atomicity).
- Entry request methods: GET, POST, PUT, PATCH, DELETE.
- UI requirements:
  - A **Bundle authoring** screen: add entries (method + URL + body),
    reorder, and validate locally before submit. Support conditional
    references / `If-None-Exist` semantics within entries.
  - A **results view** mapping each entry to its response `status`, `location`,
    `etag`, `lastModified`, and `outcome` (OperationOutcome on error).
  - Honor `Prefer: return=minimal|representation` for response verbosity.
  - Per-entry authorization may apply under SMART; surface per-entry `403`.

---

## 11. Terminology integration

- The server can delegate terminology-dependent token search modifiers
  (`:in`, `:above`, `:below` against code hierarchies) and FHIRPath terminology
  functions to an external terminology server (HTS) **only when
  `HFS_TERMINOLOGY_SERVER` is configured**. (`:not-in` is **never** delegated — it
  returns `501` regardless of configuration; see §7.2.)
- **Critical behavior:** when no terminology server is configured, the server does
  **not** reject `:in`/`:above`/`:below` — it **silently ignores** them and
  returns a `200` with literal-match (effectively wrong) results. The UI therefore
  CANNOT rely on an HTTP error to detect unavailability; it MUST gate these
  modifiers on configuration/capability detection.
- UI requirements:
  - The UI MUST detect whether terminology delegation is available (via server
    capability/config signaling) and **only then**:
    - enable the value-set–backed search modifiers in the Search Builder, and
    - show a **link-out to the configured terminology server (HTS)** in
      navigation (§5, item 12) and contextually near terminology-dependent
      controls.
  - When no terminology server is configured, these modifiers and the link MUST
    be hidden or disabled with an explanatory tooltip (since the server will
    otherwise accept them and return misleading results).

---

## 12. Capability, conformance & metadata

A dedicated area (and underpinning for §4.1) that exposes:
- **CapabilityStatement** (`GET /metadata`): rendered human-readably (per-resource
  interactions, conditional-op support, versioning, search params, formats,
  security/CORS) and as raw JSON/XML. Version-specific per active FHIR version.
- **Supported versions** (`GET /$versions`): list with default highlighted.
- **SMART configuration** (`GET /.well-known/smart-configuration`): endpoints,
  scopes, capabilities.
- **Search-parameter catalog**: browsable list of search parameters per resource
  type (feeds the Search Builder). Note the served CapabilityStatement advertises
  only the seven common params with no modifiers (§7.9), so this catalog must be
  sourced from the bundled FHIR `SearchParameter` definitions, not `/metadata`.

---

## 13. SQL-on-FHIR (ViewDefinition & SQLQuery)

> SQL-on-FHIR is integrated directly into the `hfs` server on the
> `feat/sof-integration` branch (richer than the standalone `helios-sof` /
> `sof-server` binary). The endpoints below run against resources held by the
> server (resolving ViewDefinitions/Libraries by stored id, canonical `url=`,
> or absolute URL) and/or against inline resources. The UI MUST adapt via the
> `$sql-on-fhir-capabilities` endpoint, mirroring §4.1.

### 13.1 SoF capability discovery
- `GET /$sql-on-fhir-capabilities` returns a `Parameters` resource declaring
  which SoF features are available. The UI MUST read these flags and adapt:
  - `supportsViewDefinitionRun` (always true when SoF is enabled)
  - `supportsViewDefinitionExport` (runtime-gated on a wired export controller)
  - `supportsSqlQueryRun` (always true)
  - `supportsInDbRunner` (true when an in-DB runner, not the in-process fallback)
  - `supportsRelativeReference` / `supportsCanonicalReference` /
    `supportsAbsoluteReference` (which ViewDefinition reference forms resolve)
  - `supportedFormat`: `ndjson`, `json`, `csv`, `parquet`, `fhir`
- These same SoF operations are also advertised on the main CapabilityStatement
  (`rest[0].operation` + a SoF extension block), so §12 MUST surface them.

### 13.2 `$viewdefinition-run` (synchronous tabular run)
Invocable at three levels:
- System: `POST|GET /$viewdefinition-run`
- Type (anonymous): `POST|GET /ViewDefinition/$viewdefinition-run`
- Instance (stored): `POST|GET /ViewDefinition/{id}/$viewdefinition-run`

GET is permitted when the ViewDefinition is supplied via a `viewReference` query
parameter (or, for instance level, inferred from the URL id) with no body.
Parameters mirror the SoF IG / sof-server: `_format` (`csv`/`ndjson`/`json`/
`parquet`/`fhir`), `header` (CSV header on/off), `viewResource` (inline
ViewDefinition), `viewReference` (relative/canonical/absolute), `resource`
(inline resources), `patient` (filter by patient reference), `_limit`
(1–10000), `_since`.

### 13.3 `$sqlquery-run` (SQL over views)
- System: `POST /$sqlquery-run`
- Type: `POST /Library/$sqlquery-run`
- Instance: `POST /Library/{id}/$sqlquery-run`
- Runs a SQLQuery `Library` whose `depends-on` ViewDefinitions are materialized
  into a (SQLite-backed) query engine; returns tabular output. Output may also
  be `fhir`-formatted. The UI MUST let users author/select a SQLQuery Library and
  run it, then preview/download results.

### 13.4 `$viewdefinition-export` (asynchronous tabular export)
Available only when an export controller is wired (`supportsViewDefinitionExport`).
- Kick-off: `POST /$viewdefinition-export` (system),
  `POST /ViewDefinition/$viewdefinition-export` (type),
  `POST /ViewDefinition/{id}/$viewdefinition-export` (instance) — supports
  multi-view exports.
- Status / cancel: `GET /export/{job_id}/status` (DELETE cancels).
- Result: `GET /export/{job_id}/result` (reached via a `303` redirect from
  status on completion).
- Download: `GET /export/{job_id}/{filename}`.
- This is a **separate async job flow from Bulk Data `$export` (§14)** — the UI
  SHOULD present SoF export jobs within the SQL-on-FHIR area but MAY share the
  jobs-list UX pattern.

### 13.5 UI requirements
- A **ViewDefinition authoring** screen: edit the ViewDefinition JSON and/or a
  guided builder for `select` / `column` / `forEach` / `where`; choose target
  FHIR version, output format, and reference form (inline `viewResource`,
  stored `ViewDefinition/{id}`, or canonical/absolute `viewReference`).
- A **SQLQuery authoring** screen for `$sqlquery-run` Libraries.
- A **run & results** view: tabular preview of CSV/JSON output with row counts,
  with **download** as CSV / NDJSON / JSON / Parquet (and `fhir` where
  applicable) — primary value to P4. Errors shown as OperationOutcome.
- An **export jobs** view for `$viewdefinition-export` (kick-off → poll status →
  follow 303 to result → download), shown when `supportsViewDefinitionExport`.
- The UI MUST hide/disable any SoF sub-feature the `$sql-on-fhir-capabilities`
  flags report as unavailable.

---

## 14. Bulk Data Export (`$export`)

Available when `HFS_BULK_EXPORT_ENABLED=true` (default) and on supported
backends (sqlite, postgres, and their `-elasticsearch` variants today; others
return `501`). The UI MUST detect availability via CapabilityStatement
(`instantiates` bulk-data, `$export` operations) and adapt.

### 14.1 Kick-off
- System: `GET|POST /$export`
- Patient: `GET|POST /Patient/$export`
- Group: `GET|POST /Group/{id}/$export`
- All kick-offs REQUIRE `Prefer: respond-async`. The server returns
  `202 Accepted` with a `Content-Location` status URL.
- Supported parameters the UI MUST let users set:
  - `_type` (resource types to include; repeatable/comma-separated)
  - `_since`, `_until` (FHIR instants)
  - `_elements` (subsetting)
  - `_typeFilter` (per-type query filter, `ResourceType?param=value`) — note the
    server rejects result-control params (`_sort`, `_include`, `_revinclude`,
    `_count`, `_elements`) *inside* a `_typeFilter` with `400`.
  - `_outputFormat` (default `application/fhir+ndjson`)
  - `patient` (POST-only, for patient/group exports; server validates membership)
- Unsupported params (`includeAssociatedData`, `organizeOutputBy`,
  `allowPartialManifests`): under `Prefer: handling=strict` the server returns
  `400`; otherwise they are ignored with a warning. The UI SHOULD warn before
  sending these.
- Per-tenant concurrency cap: kick-off may return `429` when exceeded — the UI
  MUST surface this and the active-job count.

### 14.2 Monitor
- `GET /export-status/{job_id}` returns progress and, on completion, the manifest
  (`transactionTime`, `request`, `output[]`, `error[]`).
- The UI MUST provide a **jobs list** (per tenant) with live status/polling, and
  a **job detail** view showing the manifest, output files, and any errors.

### 14.3 Download
- `GET /export-file/{job_id}/{part}` returns an NDJSON file (HFS-served output).
- For `local-fs` output the files are HFS-served; for `s3` output the manifest
  may contain pre-signed URLs. The UI MUST handle both: list each `output[]`
  entry by type and offer download.

### 14.4 Cancel / delete
- `DELETE /export-status/{job_id}` cancels a running job and deletes output.
  Confirm destructive action.

---

## 15. Subscriptions

When built/enabled, the server exposes:
- `GET /Subscription/{id}/$status` — status (SubscriptionStatus/Parameters)
- `GET /Subscription/{id}/$events` — recent events as a Bundle
- `GET /Subscription/{id}/$get-ws-binding-token` — WebSocket binding token
- `GET /ws/subscriptions/bind` — WebSocket notification channel

UI requirements (only when the feature is advertised): a Subscription management
view to inspect status, view recent events, and (for developers) obtain a
WebSocket token and observe live notifications. Hidden when the feature is off.

---

## 16. Admin & operations (P2)

A consolidated operations area:
- **Tenants:** active-tenant selector, routing-mode awareness, and (where
  applicable) a list of known tenants. Strict-validation behavior surfaced.
- **Health:** `GET /health`, `/_liveness`, `/_readiness` with backend name and
  timestamps; a clear up/down indicator.
- **Auth/SMART status:** whether auth is enabled, configured endpoints, current
  token/scopes (P2/P1).
- **Audit visibility:** the server can record audit events (file/database
  backends, configurable). When an audit query surface is available, the UI
  SHOULD provide a read-only audit log view; otherwise present audit
  configuration status only. *(Audit query endpoint availability TBD — treat as
  Planned if no read API exists.)*
- **Server/config info (read-only):** active FHIR versions & default, storage
  backend in use, bulk-export configuration (enabled, output backend, retention,
  concurrency caps). Runtime reconfiguration is **out of scope** (§2.2).

---

## 17. Resource editing experience (detail for §6)

- **Three editing modalities**, switchable per resource:
  1. **Raw JSON** editor with schema-aware validation hints (P1).
  2. **Raw XML** editor when the server advertises XML (P1, Conditional).
  3. **Guided form** generated from the resource's elements for the active FHIR
     version (P3) — required fields, cardinality, value sets where terminology is
     available, references with type-aware pickers.
- Reference fields SHOULD offer search-as-you-type against the referenced type
  (using §7 search) and render as navigable links in read mode.
- Choice-type elements (`value[x]`) MUST be handled (note: server-side search on
  choice types uses concrete field names — see internal limitations).
- Validation: the server performs basic validation (resourceType present, type
  matches URL); there is **no** `$validate` operation (§18). The UI SHOULD do
  client-side structural checks and rely on `OperationOutcome` for server-side
  errors.

---

## 18. Known limitations the UI must account for

These are server-side gaps; the design must not promise more than the server
delivers, but may scaffold UI ahead of them (clearly marked).

- **History & vread:** now fully wired end-to-end — instance/type/system history
  and vread return proper Bundles/resources, including on the `-elasticsearch`
  composite variants. R6 Trial Use delete-history / delete-version also work
  (§8.2).
- **Chained / `_has` search:** resolved application-side (uniform across
  backends); nested `_has` supported. Reverse `_has` is capped at 4 (hardcoded,
  **not** configurable); forward chaining currently has **no** depth cap (§7.6).
- **CapabilityStatement under-advertises search:** `GET /metadata` advertises only
  seven common search params (and no modifiers) for every resource type; the UI
  must source its search catalog elsewhere (§7.9).
- **Silent-ignore search behaviors:** several unsupported or misconfigured search
  inputs return `200` with wrong/unfiltered results instead of an error —
  `:in`/`:above`/`:below` with no terminology server (§11), `_query`/`_list`/
  `_score`/`_contained` (§7.1), a `_filter` that fails to parse (§7.1), and
  unsupported `_sort` fields (§7.4). The UI must not treat a `200` as confirmation
  the filter/sort was applied.
- **No prefix/parameter-type validation:** `hfs` never validates search prefixes
  against parameter type, so misuse (e.g. `gt` on a string param, or `sa`/`eb` on
  number/quantity) is neither rejected nor reliably honored — the UI must scope
  prefixes client-side (§7.3).
- **No search-value escaping:** the server does not unescape `\,` `\|` `\$` `\\`
  in search values, so literal commas/pipes in user input are mis-split into
  OR-values / token `system|code`. The UI must escape on its own (§7.1).
- **Search pagination links:** Bundles never include `first`/`last` links — paging
  must use `next`/`previous` only (§7.4).
- **No `$validate`, `$everything`, `$graph`, `$document` operations.**
- **No GraphQL endpoint.**
- **No user-defined custom `$operations` registration.**
- **Subscription delivery to external endpoints** is not implemented in the base
  server (the Subscription resource and `$status`/`$events`/WebSocket exist when
  the feature is enabled).
- **mTLS / OAuth server:** TLS termination and the OAuth/OIDC provider are
  external; the server only validates tokens and points to an external provider
  via SMART discovery.
- **SQL-on-FHIR endpoints** are integrated into `hfs` on the
  `feat/sof-integration` branch (§13); availability of each sub-feature is
  reported by `$sql-on-fhir-capabilities`.
- **S3-only backend has no search**; some backends don't support bulk export.
  The UI MUST adapt to advertised capabilities rather than assume all features.

---

## 19. Non-functional requirements

- **Performance:** searches and lists must paginate; never load unbounded result
  sets. Respect server `_count` max (1000). Large bodies respect
  `HFS_MAX_BODY_SIZE`.
- **Resilience:** handle `429` (export concurrency), `503`/timeouts
  (`HFS_REQUEST_TIMEOUT`), and transient backend errors with retry/backoff where
  safe (idempotent reads only).
- **Statelessness:** the UI holds no server data of its own beyond session
  (active tenant, version, token, recent queries). It is a pure API client.
- **Observability for users:** every mutating action shows the resulting status,
  ids, and version; destructive actions require confirmation.
- **Internationalization-ready** copy; dates rendered in the user's locale while
  preserving FHIR instant precision in raw views.

---

## 20. Open questions / decisions to revisit

1. **SQL-on-FHIR merge timeline** — endpoint shapes are settled on
   `feat/sof-integration` (§13); confirm when they merge to `main` and that
   bulk `$export` (§14) and SoF coexist post-merge (the branch predates the
   bulk-export merge). Confirm whether the async `$viewdefinition-export`
   controller will be wired by default (`supportsViewDefinitionExport`).
2. **Audit query API** — does/will `hfs` expose a read endpoint for audit events,
   or is audit visibility config-only? (§16)
3. **Tenant enumeration** — is there an API to list known tenants, or is the
   tenant set deployment-known only? (§4.3, §16)
4. **Single adaptive app vs role-segmented entry points** — confirmed direction
   is one adaptive app (§3); validate during wireframing.

---

*End of requirements. Next step: feed this document into Claude Design to
produce wireframes for the screens in §5.*
