---
name: hts-api-skill
description: >-
  Expert reference for building UI on top of the Helios Terminology Server
  (HTS) HTTP API. Covers all 42 HTS routes ($lookup, $validate-code, $expand,
  $subsumes, $translate, $closure, $batch-validate-code,
  CodeSystem/ValueSet/ConceptMap CRUD, /import, /metadata, /health,
  /metrics); FHIR content
  negotiation, OperationOutcome error rendering, and BCP-47 language handling;
  the `HFS_TERMINOLOGY_SERVER` server-side proxy pattern already used by
  `/ui/editor/expand`; and the client-surface gaps ($batch-validate-code,
  $closure, HTS CRUD) that no HFS crate covers yet. Use when authoring or
  reviewing UI work for terminology flows, adding `/ui/hts/*` pages under
  `crates/ui`, wiring a terminology admin dashboard, building an operator
  surface for HTS bootstrap/import, or planning the `/ui/hts` route inventory.
  Composes with `work-with-hts`, `work-with-ui`, and `frontend-design`.
---

# hts-api-skill

This skill is a UI-builder's expert overlay on the Helios Terminology Server
(HTS) HTTP surface. It is **self-contained**: everything a UI author needs is
here, and the source of truth for anything it does not cover is the code
itself — [crates/hts/src/server.rs](../../crates/hts/src/server.rs) is the
route table, and `crates/hts/src/operations/` holds each handler.

## 1. When to use

Apply this skill for any task that adds, changes, or reviews a UI surface that
touches CodeSystem, ValueSet, or ConceptMap, or that calls an HTS operation.
Concrete triggers:

- Adding `/ui/hts/*` pages or fragments under `crates/ui` (currently only a
  nav placeholder exists — `nav-terminology` is marked `nav-item--soon`).
- Adding a terminology admin dashboard, bootstrap-ledger view, or import
  status page.
- Wiring `$lookup`, `$validate-code`, `$expand`, `$subsumes`, `$translate`,
  `$closure`, or `$batch-validate-code` into any UI form.
- Debugging why `/ui/editor/expand` returns 204 or how ValueSet
  `X-TOO-COSTLY-THRESHOLD` should surface to the user.
- Designing OperationOutcome rendering for terminology failures.

Skill composition (co-load these when working on HTS UI):

- **Core triangle** (mandatory project siblings):
  - [.claude/skills/work-with-hts/SKILL.md](../work-with-hts/SKILL.md) —
    server-side / operator-side HTS concerns, import, config
  - [.claude/skills/work-with-ui/SKILL.md](../work-with-ui/SKILL.md) —
    `crates/ui` templates, htmx fragments, `/ui` routes, i18n, tests
  - [.claude/skills/frontend-design/SKILL.md](../frontend-design/SKILL.md) —
    palette, typography, layout, accessibility, no-SPA/no-CDN rules
- **Situational**: [.claude/skills/test-hfs/SKILL.md](../test-hfs/SKILL.md),
  [.claude/skills/run-hfs-and-hts/SKILL.md](../run-hfs-and-hts/SKILL.md),
  [.claude/skills/work-with-auth/SKILL.md](../work-with-auth/SKILL.md).
- **User-scope skills**: when personal skills for HTMX authoring, terminology
  UX, or front-end design are installed under `~/.claude/skills/`, they
  auto-load alongside — treat them as authoritative for their domain and
  defer stylistic choices to them.

Do **not** duplicate `work-with-ui`'s framework conventions here. This skill
adds only the HTS-specific surface.

## 2. Authoritative sources

- **Canonical route table:** [crates/hts/src/server.rs](../../crates/hts/src/server.rs)
  — every route HTS answers, in one `create_app` function
- **Crate README:** [crates/hts/README.md](../../crates/hts/README.md)
- **Server-side truth (routes and wiring):**
  [crates/hts/src/server.rs](../../crates/hts/src/server.rs),
  [crates/hts/src/error.rs](../../crates/hts/src/error.rs),
  [crates/hts/src/operations/](../../crates/hts/src/operations)
- **Existing UI HTS touchpoint:**
  [crates/ui/src/editor.rs](../../crates/ui/src/editor.rs) — the
  `/ui/editor/expand` proxy handler
- **Available library clients** (server-side reuse candidates):
  [crates/fhirpath/src/terminology_client.rs](../../crates/fhirpath/src/terminology_client.rs),
  [crates/rest/src/terminology.rs](../../crates/rest/src/terminology.rs)
- **Local run recipe:** [run-hts-server](../run-hts-server/SKILL.md), or
  [run-hfs-and-hts](../run-hfs-and-hts/SKILL.md) to wire both servers together
- **In-skill deep tables:** [endpoints-quickref.md](endpoints-quickref.md),
  [ui-design-map.md](ui-design-map.md)

## 3. Operation cheat-sheet

One row per operation family. Full route matrix in
[endpoints-quickref.md](endpoints-quickref.md); the parameters each handler
actually reads are in `crates/hts/src/operations/`.

| Op | Primary route | UI surface |
|---|---|---|
| CodeSystem `$lookup` | `GET/POST /CodeSystem/$lookup`, `/{id}/$lookup` | Concept inspector: display, definition, designations, properties, parent/child |
| CodeSystem `$validate-code` | `GET/POST /CodeSystem/$validate-code` | Single-code validator with display / active / abstract / supplement toggles |
| CodeSystem `$subsumes` | `GET/POST /CodeSystem/$subsumes` | Two-code relationship viewer (`equivalent`, `subsumes`, `subsumed-by`, `not-subsumed`) |
| ValueSet `$expand` | `GET/POST /ValueSet/$expand`, `/{id}/$expand` | Expansion browser with filter, flat/tree, pagination, language, designations, supplements, `tx-resource`; 422 `too-costly` needs an `X-TOO-COSTLY-THRESHOLD` escape hatch |
| ValueSet `$validate-code` | `GET/POST /ValueSet/$validate-code`, `/{id}/$validate-code` | Membership validator with `tx-resource`, version pins, language |
| ValueSet `$batch-validate-code` | `POST /ValueSet/$batch-validate-code` | Bulk validation grid with inherited defaults + per-row overrides |
| ConceptMap `$translate` | `GET/POST /ConceptMap/$translate`, `/{id}/$translate` | Forward/reverse translation with match grid, equivalence/relationship column |
| ConceptMap `$closure` | `POST /ConceptMap/$closure` | Stateless hierarchy edge builder — must be labeled non-persistent |
| Root batch | `POST /` | Bundle workbench limited to the three supported entry URLs (see §7 for design, §10 for behavior) |
| Import | `POST /import` | JSON Bundle upload with counts + non-fatal error list (no XML) |
| Metadata | `GET /metadata` | CapabilityStatement / TerminologyCapabilities switcher |
| Health | `GET /health` | Process status only — **not** a dependency-readiness probe |
| Metrics | `GET /metrics` | Prometheus text; hide from admin UI, expose in monitoring integration |
| CRUD + search | `GET/POST/PUT/DELETE /{Resource}[/{id}]` | Browser + editor per resource; search is exact-match only |

## 4. Common client concerns

### 4.1 GET versus POST parameter shape

GET operation routes coerce every query pair to `valueString`. They **cannot**
represent structured `Coding`, `CodeableConcept`, embedded resource, boolean,
or integer values with native FHIR types. Any form that needs one of those
must POST a `Parameters` body. Repeatable string params work under GET via
repeated keys.

POST routes read a `Parameters` body and support `valueXxx`, `part`, and
embedded `resource`. Query parameters on POST are used only for `_format`;
they are **not** merged with the body.

### 4.2 Content negotiation

`_format` takes precedence over `Accept`. Parsing is substring-based and does
not honor `q` weights, so any XML mention in `Accept` selects XML. XML is
response-only — POST bodies must be JSON. `$expand` sets
`application/fhir+json` explicitly; other JSON responses use plain
`application/json`. XML uses `application/fhir+xml; charset=utf-8`. Search,
`/health`, and `/import` are JSON-only. **Errors are always JSON
`OperationOutcome` even when XML was requested.**

For UI defaults, send `Accept: application/fhir+json` and parse JSON only.

### 4.3 Language handling

Explicit `displayLanguage` beats `Accept-Language`. BCP-47 matching runs
exact → separator-insensitive → primary language. No global English fallback.
Handlers that honor language: `$lookup`, `$expand`, both `$validate-code`
variants. `$subsumes`, `$translate`, `$closure`, and `$batch-validate-code`
do **not** uniformly read the header — set `displayLanguage` explicitly there.

### 4.4 Compression, body limits, timeouts

Supported request encodings: `gzip`, `deflate`, `br`, `zstd`. Unsupported →
415. Decompressed payload > `HTS_MAX_BODY_SIZE` (default 10 MiB) → 413.
Per-request timeout is 30 seconds → 408. Expansion beyond
`HTS_MAX_EXPANSION_SIZE` (CLI default 3,500) → 422 with `too-costly`. The UI
can inject a request-scoped ceiling via the `X-TOO-COSTLY-THRESHOLD` header.

### 4.5 Error model — always parse OperationOutcome

`HtsError` variants map to HTTP + FHIR `OperationOutcome`:

| Variant | HTTP | issue.code |
|---|---:|---|
| `NotFound` | 404 | `not-found` |
| `NotSupported` | 501 | `not-supported` |
| `InvalidRequest` | 400 | `invalid` |
| `VsInvalid` | 400 | `invalid` |
| `PreconditionFailed` | 412 | `conflict` |
| `TooCostly` | 422 | `too-costly` |
| `Internal` | 500 | `exception` |
| `StorageError` | 500 | `exception` |

Cyclic ValueSet reference → 422 (grouped with `TooCostly`); invalid display
language, ValueSet-version checks, and unknown CodeSystem-version issues →
400 (grouped with `InvalidRequest`).

Special IG-compatible variants exist for invalid display language, ValueSet
version checks, unknown CodeSystem versions, and cyclic ValueSet references.
Pre-handler infrastructure failures (malformed JSON, 408, 413, 415) may
**not** use the standard OperationOutcome shape — the UI must gracefully
render both.

Critical UI rule: for `$validate-code` and `$translate`, "not valid /
no match" is **HTTP 200** with `result=false`; only surface it as an error
when transport status is 4xx/5xx.

## 5. Auth gaps the UI must handle

HTS has **no built-in authentication or authorization middleware** — there is
no auth layer in `create_app`. Any HTS deployment relies on a reverse proxy,
service mesh, or private network boundary.

UI implications:

- Destructive HTS routes (`POST/PUT/DELETE` on CodeSystem/ValueSet/ConceptMap,
  `POST /import`, `POST /`) are unauthenticated at the server. Editor and
  admin pages **must** be gated by deployment-level auth before production;
  render a top-of-page warning banner in dev-mode builds.
- CORS default is `HTS_ENABLE_CORS=true` with origin `*`. Direct
  browser-to-HTS calls will succeed cross-origin but they leak the HTS URL.
  Prefer the server-side proxy pattern in §6.
- When HTS is reached through HFS (via `HFS_TERMINOLOGY_SERVER` for search
  modifier pre-processing or validation), only HFS routes carry auth. There
  is no HFS FHIR route that transparently proxies HTS operations today
  (confirmed by [HTS auth-gap review](22d3fbbc-ff61-4c26-a7ea-a4726aaba59e)).

## 6. UI integration in `crates/ui`

Findings from
[UI HTS surface audit](5f4aacfd-6d52-4268-9ef2-fa8aeb1d402e) and
[Terminology client inventory](bc3e226b-e68f-4dc9-ad90-1a064a63bc37) drive this section. Follow
[work-with-ui](../work-with-ui/SKILL.md) for the framework conventions; the
notes below are the HTS-specific additions.

### 6.1 Add an `hts` module

- Declare `mod hts;` alongside the other UI modules in
  [crates/ui/src/lib.rs](../../crates/ui/src/lib.rs) (see the existing
  `editor`, `conformance`, `bulk_import` modules for shape).
- Register routes on the mount router **before** `.merge(assets)`. Extractor
  order for a typical page: `State<WebState>`, `RequestLocale`,
  `RequestVersion`, `RequestTenant`.
- The nav-item `nav-terminology` in
  [crates/ui/templates/layouts/base.html](../../crates/ui/templates/layouts/base.html)
  is currently `nav-item--soon`. Remove the `--soon` variant and point it at
  the new `/ui/hts` root when landing your first page.

### 6.2 Reuse the `HFS_TERMINOLOGY_SERVER` proxy pattern

Do not ship a browser-side HTS client. The workspace already has three
in-process clients:

- [helios_rest::terminology::TerminologyServiceClient](../../crates/rest/src/terminology.rs)
  — `$expand` only (used for search `:in` / `:below` / `:above` modifiers)
- [helios_fhirpath::TerminologyClient](../../crates/fhirpath/src/terminology_client.rs)
  — broadest coverage: `$expand`, `$lookup`, `$validate-code` (CS + VS),
  `$subsumes`, `$translate`
- [helios_rest::validation::RemoteTerminologyProvider](../../crates/rest/src/validation.rs)
  — cached `$validate-code`, only used inside validation

Server-side handlers in `crates/ui/src/hts/*.rs` should either:

1. Reuse `helios_fhirpath::TerminologyClient` for lookup / validate / subsumes
   / translate — it already handles `Parameters` serialization, timeouts,
   and error propagation.
2. Copy the [`editor::expand`](../../crates/ui/src/editor.rs) reqwest
   pattern (no proxy, short timeout, degrade to 204) when a bespoke shape is
   needed.

For `$batch-validate-code` and `$closure` there is **no existing client
anywhere in HFS**. New UI code must add a fresh reqwest call — mirror the
`editor::expand` shape and gate it behind `WebState.terminology.is_some()`.

### 6.3 HTMX fragment vs full-page

`crates/ui` uses two dual-mode patterns:

- **`HxRequest` branching** (see `status` in `lib.rs`): one handler returns
  a full-page template on plain GET and a fragment when `HX-Request` is set.
- **Dedicated fragment routes** (dominant): separate routes for full page
  vs. swap target, e.g. `GET /ui/editor` (page) and `POST /ui/editor/render`
  (fragment).

Prefer the dedicated-fragment pattern for HTS forms — expansion, validation,
translation results are the natural swap targets. Fragment templates go in
`crates/ui/templates/partials/hts-<op>-<part>.html`; page shells in
`crates/ui/templates/pages/hts-<screen>.html` and extend
`layouts/base.html`. The `AutoVaryLayer` on the mount router adds
`Vary: HX-Request` automatically.

### 6.4 i18n keys

- Catalogs live at workspace root `locales/<locale>/main.ftl` (not inside
  `crates/ui`); shipped locales are `en` (source), `es`, `de`.
- Use `hts-*` key prefixes for stability: `hts-lookup-title`,
  `hts-expand-filter-placeholder`, `hts-validate-result-ok`,
  `hts-error-not-found`, `hts-op-outcome-code-invalid`, etc.
- All non-English locales must mirror the `en` key set (enforced by
  `crates/ui/src/i18n.rs`).
- Server-rendered JS strings ride on `data-msg-*` attributes on the page
  root — do not add a new browser-side JSON API for translated strings.

### 6.5 Error rendering — no shared toast component

`crates/ui` uses `role="alert"` inline banners and `notice notice--warn`
partials, not a shared toast. For HTS:

- Server handler parses `OperationOutcome` and reduces it to a typed
  `HtsUiError { severity, code, diagnostics, expression }` before passing
  to the template.
- Fragment templates render a `partials/hts-outcome.html` alert block with
  the FHIR `severity` mapped to `alert--error | alert--warning | alert--info`.
- On `result=false` (200 body), render a neutral "no match" state — this is
  not an error condition.

### 6.6 Tenant + language propagation

- `/ui/editor/expand` currently does **not** propagate tenant or
  `Accept-Language` to HTS (confirmed at
  [crates/ui/src/editor.rs](../../crates/ui/src/editor.rs)). This is a bug
  for multi-tenant deployments. When authoring new HTS handlers:
  - Read `RequestTenant` and add `X-Tenant-ID` if the deployment terminates
    tenancy at HTS.
  - Read the negotiated locale via `RequestLocale` and either send it as
    `displayLanguage` on the operation call or forward `Accept-Language`
    upstream.

### 6.7 Existing UI HTS surface (starting inventory)

| Route | Handler | Behavior |
|---|---|---|
| `GET /ui/editor` | `editor::page` | Full-page editor shell |
| `POST /ui/editor/render` | `editor::render_body` | Editor body fragment: applies one structural mutation to the posted document, revalidates against embedded core packs, and re-renders |
| `GET /ui/editor/expand` | `editor::expand` | Server-side `$expand` proxy; JSON `{codes:[…]}`; 204 on failure |

Everything else in [ui-design-map.md](ui-design-map.md) is greenfield.

### 6.8 UI patterns, as shipped

The console exists now, so the patterns below are best read from the code that
implements them rather than restated here:

1. **Operation workbench** — one input partial per operation, swapped into a
   shared region: `crates/hts-ui/templates/partials/hts-cs-*-input.html`
2. **Resource browser** — a filter toolbar over a `.data-table`, rows fetched
   from a `/rows` fragment: `templates/pages/cs-browser.html`
3. **Concept renderer** — `templates/partials/hts-concept-identity.html`
4. **Click-to-load pagination**, because `Bundle.total` echoes the page size
   rather than the store size: `templates/partials/hts-cs-rows.html`
5. **Chrome parity with HFS** — same stylesheet, same topbar, same nav
   vocabulary: `templates/layouts/base.html` beside `crates/ui`'s

Extracting a shared `helios-ui-chrome` crate is deferred as its own piece of
work; until then `tests/chrome_parity.rs` guards the duplication.
Per-operation field matrices remain in [ui-design-map.md](ui-design-map.md).

## 7. UI design map (summary)

Recommended page tree — details, inputs, outputs, and edge cases per operation
in [ui-design-map.md](ui-design-map.md):

- `/ui/hts` — dashboard (health, backend, uptime, bundled data summary)
- `/ui/hts/code-systems` — browser + editor; per-instance `$lookup`,
  `$validate-code`, `$subsumes`
- `/ui/hts/value-sets` — browser + editor; per-instance `$expand`,
  `$validate-code`, `$batch-validate-code`
- `/ui/hts/concept-maps` — browser + editor; per-instance `$translate`,
  `$closure` workbench
- `/ui/hts/import` — Bundle upload + non-fatal error list
- `/ui/hts/batch` — Bundle workbench limited to the three supported
  `entry.request.url` values: `CodeSystem/$validate-code`,
  `ValueSet/$validate-code`, `ConceptMap/$translate`. Label transaction
  bundles non-atomic.
- `/ui/hts/diagnostics` — CapabilityStatement viewer with the
  `mode=terminology` toggle; optional Prometheus deep-link

## 8. Local run recipe

For starting `hts` alone, or `hfs` + `hts` together wired via
`HFS_TERMINOLOGY_SERVER` / `FHIRPATH_TERMINOLOGY_SERVER`, see
[.claude/skills/run-hts-server/SKILL.md](../run-hts-server/SKILL.md) and
[.claude/skills/run-hfs-and-hts/SKILL.md](../run-hfs-and-hts/SKILL.md) —
both cover the bundled `crates/hts/terminology-data` seed set via
`HTS_BOOTSTRAP_DIR`. The full Rust + MinGW toolchain setup for this
Windows machine lives at
[run-hts-server](../run-hts-server/SKILL.md).

Sanity: `Invoke-WebRequest http://127.0.0.1:8090/metadata?mode=terminology`
should return a TerminologyCapabilities resource.

## 9. Verification checklist for UI PRs

Before requesting review on any `/ui/hts/*` change:

- [ ] `Accept: application/fhir+json` set on every server-side HTS call
- [ ] `OperationOutcome` parsed for every non-2xx and rendered via
      `partials/hts-outcome.html`; pre-handler failures (408/413/415/malformed
      JSON) handled with a generic banner
- [ ] `result=false` on `$validate-code` / `$translate` rendered as a neutral
      "no match" state, not an error
- [ ] `displayLanguage` explicitly sent for `$subsumes`, `$translate`,
      `$closure`, and `$batch-validate-code` (they do **not** read
      `Accept-Language` uniformly)
- [ ] Tenant + locale propagated to HTS via header or explicit param
- [ ] Structured Coding / CodeableConcept inputs use POST; only bare
      `system|code` uses GET
- [ ] Expansion pagination uses `count`/`offset`; tree mode ignores paging;
      `X-TOO-COSTLY-THRESHOLD` surfaced when the user requests larger pages
- [ ] `$closure` UI clearly labeled stateless / non-persistent
- [ ] Root batch UI restricted to the three supported entry URLs; transaction
      bundles labeled non-atomic
- [ ] `/import` UI JSON-only (no XML); errors surfaced from the non-FHIR
      summary payload
- [ ] Search UI does not advertise fields not implemented by HTS (no `_sort`,
      `_include`, chained params, or claim to accurate `_total`)
- [ ] Loading + empty + error states for every fragment; htmx `hx-target`
      degrades correctly when `WebState.terminology` is `None`
- [ ] i18n keys added under `hts-*` in `locales/en/main.ftl` and mirrored
      to `locales/es/main.ftl`, `locales/de/main.ftl`
- [ ] Fragment templates carry no `<!doctype>` or `<html>` (see the tests in
      `crates/ui/tests/router_http.rs`)
- [ ] `nav-terminology` no longer `nav-item--soon` if any HTS page ships
- [ ] Playwright test added under `crates/ui/tests` (framework conventions
      in [work-with-ui](../work-with-ui/SKILL.md))
- [ ] Auth-gap warning banner rendered in non-production builds when the UI
      exposes destructive HTS routes

## 10. Gaps and known drift (call out in review)

Verified against the handlers in `crates/hts/src/operations/`.
Any UI that surfaces these must reflect the reality, not the advertised metadata:

- `$versions` is advertised at server-level in CapabilityStatement but has
  **no route** — do not add a UI action for it.
- `$batch-validate-code`, `/import`, root batch, `/metrics` are routed but
  **not advertised** in CapabilityStatement.
- `TerminologyCapabilities.expansion.parameter` advertises
  `includeDefinition`, but the current `$expand` handler does not read it.
- Search `total` is a **page count**, not the full match total. Do not label
  it "total matches". Search has no pagination links.
- `_sort`, `_include`, `_id`, `_elements`, chained params, modifiers, and
  accurate `_total` are not implemented.
- Root `transaction` bundles are **not atomic**.
- `$closure` is stateless — prior requests are not persisted or merged.
- GET operation inputs are strings only.
- POST operation query parameters are not merged with body Parameters.
- Errors do not honor XML negotiation.
- `/health` does not check database readiness.
- `:not-in` search modifier is not implemented (HFS returns 501).

## 11. References

- [endpoints-quickref.md](endpoints-quickref.md) — grouped 42-route matrix
- [ui-design-map.md](ui-design-map.md) — per-operation UI surfaces
- [crates/hts/src/server.rs](../../crates/hts/src/server.rs) — the route table
  reference (parameter matrices, error mapping, bundled data, source map)
- [crates/hts/README.md](../../crates/hts/README.md) — crate README
- [.claude/skills/work-with-hts/SKILL.md](../work-with-hts/SKILL.md)
- [.claude/skills/work-with-ui/SKILL.md](../work-with-ui/SKILL.md)
- [.claude/skills/frontend-design/SKILL.md](../frontend-design/SKILL.md)
- [.claude/skills/test-hfs/SKILL.md](../test-hfs/SKILL.md)
- [run-hts-server](../run-hts-server/SKILL.md) — local setup
