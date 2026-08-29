# HTS UI design map

> **Reuse strategy:** the authoritative component-reuse plan (unified operation
> workbench, resource browser, and concept renderer) is best read from the
> shipped templates under `crates/hts-ui/templates/`.
> This file remains the per-operation field / fragment matrix.

Per-operation UI surface guidance for the future `/ui/hts/*` pages under
[crates/ui](../../crates/ui). Each block gives:

- **Page / fragment**: which route and swap target
- **Inputs**: form fields (typed, not just `valueString`)
- **Output shape**: what the server returns and how to render it
- **htmx boundaries**: which fragment routes to define
- **Edge cases**: quirks the UI must render correctly

Framework conventions (Askama layout, HxRequest, i18n) are in
[work-with-ui](../work-with-ui/SKILL.md). This document is HTS-specific.

---

## 1. Dashboard — `/ui/hts`

| | |
|---|---|
| Sources | `GET /health`, `GET /metadata`, `GET /metadata?mode=terminology` |
| Fragment | `partials/hts-dashboard-cards.html` polled every 15 s via `hx-trigger` |
| Inputs | none |
| Output | Cards: `status`, `backend`, `uptime_seconds`, `version` (from `/health`); `fhirVersion` and advertised expansion/validation parameters (from `/metadata?mode=terminology`). Do **not** advertise the numeric expansion limit or CORS state — HTS exposes neither over HTTP. |
| Edge cases | `/health` is not a readiness probe — do not label it "ready"; show a "backend" pill (sqlite / postgres) not a URL; `TerminologyCapabilities.expansion.parameter` advertises parameter names, not the `HTS_MAX_EXPANSION_SIZE` value |

## 2. CodeSystem `$lookup` — `/ui/hts/code-systems/{id}/lookup`

> **Slice B (2026-08-18) — shipped.** Implemented in `crates/hts-ui/src/code_systems.rs`
> and the `hts-cs-lookup-input.html` + `hts-cs-workbench-result.html` partials.
> The embedded route is `POST /ui/hts/code-systems/{id}/lookup` (per §7.3 detail-page
> workbench); the tab GET at the same path returns the input partial. The
> `useSupplement` field is deferred to the Slice E standalone workbench where
> the extra chrome fits.

| | |
|---|---|
| Route | `POST /ui/hts/code-systems/{id}/lookup` (fragment) |
| HTS calls | `POST /CodeSystem/{id}/$lookup` (default), `POST /CodeSystem/$lookup` when no id |
| Inputs | `code`, `version`, `displayLanguage`, repeatable `property` (checkbox list plus `*`), `date`, repeatable `useSupplement` |
| Output | `Parameters`: `name`, `version`, `display`, `definition`, repeatable `property`, repeatable `designation`, `used-supplement`, hierarchy via `property=parent` / `property=child` |
| Fragment | `partials/hts-cs-workbench-result.html` (shared across CS ops) — grouped panels for designations + properties |
| Edge cases | `expression` param → 501 (do **not** expose in the form); no top-level `subsumedBy` — read hierarchy from `property=parent/child`; on 404 render neutral "unknown concept" state |

## 3. CodeSystem `$validate-code` — `/ui/hts/code-systems/{id}/validate`

> **Slice B (2026-08-18) — shipped, with divergences from the standalone-page
> design below.** Because HTS has no CS instance-level `$validate-code` route
> (hts-details.md), the Validate tab resolves the CS canonical URL from the
> same read that backs the detail page and POSTs to the type-level
> `/CodeSystem/$validate-code`. The Slice B UI exposes `code` and `Coding`
> modes only; `CodeableConcept` is deferred to Slice E (§7.6 standalone
> operations workbench), where the extra field-set fits.

| | |
|---|---|
| Route (Slice B, detail-embedded) | `POST /ui/hts/code-systems/{id}/validate` |
| Route (Slice E, standalone) | `POST /ui/hts/code-systems/validate` |
| HTS calls | `POST /CodeSystem/$validate-code` (both routes; canonical URL comes from the CS read in Slice B) |
| Slice B inputs | Mode selector `code` / `Coding`, `code`, `display`, `coding.system`, `coding.code`, `coding.display`, `displayLanguage` |
| Slice E inputs (future) | Adds `CodeableConcept`, `version`/`systemVersion`, `lenient-display-validation`, `abstract`, `date`, `activeOnly`, `useSupplement`, `force-system-version`, `system-version`, `check-system-version` |
| Output | `Parameters`: `result` (bool), plus optional `code`, `system`, `version`, `display`, `inactive`, `status`, `message`, `issues`, `normalized-code`, unknown-system markers |
| Fragment | `partials/hts-cs-workbench-result.html` (shared) — result badge (true/false) + labelled properties + embedded issues via `partials/hts-outcome.html` |
| Edge cases | Coding must POST (GET can't carry it); `result=false` is HTTP **200**, not an error; render `issues` as OperationOutcome via shared partial |

## 4. CodeSystem `$subsumes` — `/ui/hts/code-systems/{id}/subsumes`

> **Slice B (2026-08-18) — shipped.** Both codes are pinned to the current
> CS's canonical URL server-side (HTS requires codeA and codeB to share a
> system, see `hts-details.md §$subsumes`), so the form asks only for
> `codeA` / `codeB` / optional `version`. The `system` value is echoed as
> a read-only note above the inputs so operators can see the scope.

| | |
|---|---|
| Route (Slice B) | `POST /ui/hts/code-systems/{id}/subsumes` |
| HTS calls | `POST /CodeSystem/$subsumes` |
| Inputs | `codeA`, `codeB`, optional `version` (system pinned server-side to the current CS's canonical URL) |
| Output | `Parameters`: `outcome` = `equivalent` / `subsumes` / `subsumed-by` / `not-subsumed` |
| Fragment | `partials/hts-cs-workbench-result.html` (shared) — outcome sentence via Fluent (`hts-cs-subsumes-outcome-*`) |
| Edge cases | Does **not** uniformly read `Accept-Language`; pass `displayLanguage` explicitly if the UI needs localized display of A / B |

## 5. ValueSet `$expand` — `/ui/hts/value-sets/{id}/expand`

> **As shipped:** the instance-scoped Expand tab only. The three-way source
> selector (canonical / instance / inline JSON) and the repeatable
> `designation[]` filter defer to Slice E's standalone Operations
> workbench (§7.6). The `Validate` tab in the earlier §6 draft below is
> also deferred to Slice E (see §6 note).

| | |
|---|---|
| Routes (Slice C) | `GET /ui/hts/value-sets/{id}/expand` (page), `POST /ui/hts/value-sets/{id}/expand` (fragment) |
| HTS calls | `POST /ValueSet/{id}/$expand` (Slice C — instance-only). `POST /ValueSet/$expand` with inline `valueSet` or canonical `url` defers to Slice E. |
| Inputs (Slice C — 14 of 15 params) | `filter`, `count`, `offset`, `displayLanguage`, `activeOnly`, `includeDesignations` (boolean only — chip multi-select for `designation[]` defers to Slice E), `useSupplement[]`, `date`, `property[]`, `tx-resource[]`, `system-version[]`, `check-system-version[]`, `force-system-version[]`, `default-valueset-version`, and a per-request `threshold` hidden form field that maps to the `X-TOO-COSTLY-THRESHOLD` request header. Advanced fields sit inside a collapsed `<details>` panel; the always-visible controls are `filter`, `count`, `offset`, `displayLanguage`, `activeOnly`, `includeDesignations`, `useSupplement[]`, and the tree/flat toggle. |
| Tree/flat mapping | `tree` ⇒ `hierarchical=true`; `flat` ⇒ `excludeNested=true`. No "auto" third state; Slice C never emits both parameters in the same request. |
| Output | Expanded `ValueSet`: `expansion.identifier`, `timestamp`, `total`, `offset`, `contains[]`, `parameter[]`, warnings. |
| Fragment | `partials/hts-vs-expand-result.html` (per-op partial mirroring Slice B's `hts-cs-workbench-result.html`; the abstract `hts-concept` renderer stays aspirational). Flat mode = table with per-row `code`, `display`, `system`, `designation` popover; tree mode = nested list with `role="tree"`. |
| Pager rule | *Flat*: `expansion.total - expansion.offset - contains.len()` (hides `[Load more]` when `≤ 0` or when `expansion.total` missing → falls back to Slice B's terminal-page heuristic `rows.len() < requested`). *Tree*: pager hidden; metadata line renders `showing full tree {N}`. |
| Threshold storage | Per-request hidden form field named `threshold`; the Advanced `<details>` numeric input and the too-costly banner's "Raise" action both write to the same field. No cookies, no session store. Values above the build-time `HTS_UI_MAX_EXPANSION_SIZE_HINT` ceiling render a warning and are NOT attached as the request header. |
| nojs | Tree toggle becomes a plain form GET-submit; results render flat-only regardless of toggle position. |
| Edge cases | Text filter is server-side; tree mode ignores paging (HTS behavior); ECL is passed via `compose.include.filter[property=constraint]` (SQLite only); implicit ValueSets via `?fhir_vs=isa/{code}` need explicit UI opt-in (deferred); `includeDefinition` param is advertised but **ignored** by HTS — do not add a UI toggle. |

## 6. ValueSet `$validate-code` — deferred to Slice E

> **As shipped:** the `Validate` tab was dropped from §7.4 during the Opus 4.7
> advisor triage (F9). VS `$validate-code` reaches its UI through Slice
> E's standalone Operations workbench at
> `/ui/hts/operations?op=validate-code&resource=ValueSet`. The dedicated
> `/ui/hts/value-sets/{id}/validate` route in the earlier draft below
> does NOT ship in Slice C.

| | |
|---|---|
| Slice E route | `POST /ui/hts/operations/validate-code` (resource selector = `ValueSet`) |
| HTS calls | `POST /ValueSet/{id}/$validate-code` or `POST /ValueSet/$validate-code` |
| Inputs (Slice E scope) | ValueSet source (canonical URL / id / inline JSON), input-mode selector (`code` / `Coding` / `CodeableConcept`), `system`/`version`/`systemVersion`, `display`, `valueSetVersion`, `displayLanguage`, `date`, `activeOnly`, `abstract`, `lenient-display-validation`, repeatable `useSupplement`, repeatable `tx-resource`, `default-valueset-version`, `system-version`, `force-system-version`, `check-system-version` |
| Output | `Parameters`: `result`, normalized concept fields, optional embedded `issues` OperationOutcome |
| Fragment | Shared operations-workbench result partial (Slice E design) |
| Edge cases | Membership failure is HTTP **200** with `result=false` → render neutral state, NOT the error partial (mirrors Slice B's CS Validate contract in §7.3.1 and §7.5's ConceptMap analog); `issues` may embed OperationOutcome — parse via shared partial. |

## 7. UI-fabricated batch validate — `/ui/hts/operations` (op = **`batch-validate`**)

> **As shipped:** renamed this op from `batch-validate-code` to **`batch-validate`** (F18 triage) because
> HTS's `$batch-validate-code` route is intentionally not used — the surface is a UI-fabricated
> fan-out over parallel `$validate-code` calls. Transport is client-side polling (F1 = D), not
> OOB streaming, to preserve the "only vendored htmx" invariant.

| | |
|---|---|
| v1 Routes | `POST /ui/hts/operations/batch-validate` (seed the skeleton table); `GET /ui/hts/operations/batch-validate/row/{i}` (per-row polling target — the seeded row's `hx-trigger="load"` GETs this); `GET /ui/hts/operations/batch-validate/progress` (progress-counter poll target) |
| Transport | **Client-side polling (F1 = D).** Each seeded row has its own `hx-get` + `hx-trigger="load"` fetching a per-row endpoint; the progress counter is a separate `hx-trigger="every 1s"` element. No OOB choreography, no SSE, no chunked-transfer, no vendored htmx extension. |
| HTS calls | N × `POST /ValueSet[/{id}]/$validate-code` (VS mode) or `POST /CodeSystem/$validate-code` (CS mode) — fan-out only; the HTS `$batch-validate-code` route is not used. |
| Concurrency | Server-side bound = `HTS_UI_BATCH_FANOUT_CONCURRENCY: usize = 8` (build-time constant alongside `HTS_UI_MAX_EXPANSION_SIZE_HINT`; Phase 1.5 may expose via `/metadata?mode=terminology`). |
| Inputs (v1) | Repeatable inline row editor (dynamic add-row `hx-get` returning a new empty row partial); each row carries `code`, `system`, `display?` OR a `Coding` / `CodeableConcept` structured input (per §7.6.1 F4 widening); a **`Target ValueSet`** canonical URL input (renamed from HTS's "principal `tx-resource`" wording to avoid conflating semantics); resource-family tab strip (CS / VS, per §7.6.1 F5). CSV / JSON import defers to Phase 2 (§7.11). |
| Seeded shell | `<tbody id="hts-batch-results">` with one `<tr id="hts-batch-row-{i}" aria-busy="true">` skeleton per input row; each carries `hx-get="/ui/hts/operations/batch-validate/row/{i}?…"` + `hx-trigger="load"`. Progress element `<p id="hts-batch-progress" aria-live="polite" aria-atomic="true">` polls every 1 s and announces `{n} of {m} completed`, then final `{m} completed, {k} failed`. |
| Fragment | `partials/hts-vs-batch-row.html` per row (Slice E per-op family — §7.6.1 F11 = A defers cross-slice refactor to Phase 3). |
| Fallback | Pre-flight validation failure → page-level `partials/hts-outcome.html`; per-row upstream 5xx → row-scoped `OperationOutcome` with `severity=error, code=exception`; per-row 5s timeout → row-scoped `OperationOutcome` with `severity=warning, code=timeout`. |
| Focus rule | On Submit, focus lands on the first `aria-busy` skeleton row (not the Submit button); subsequent row swaps do NOT move focus. |
| nojs | Form POSTs synchronously; server fans out (still bounded by `HTS_UI_BATCH_FANOUT_CONCURRENCY`), waits for all rows, pre-renders full table. Same URL contract; no client-side polling. |
| Edge cases | Individual `$validate-code` failures embed as row-scoped `OperationOutcome` (HTTP 200 outer); do **not** emulate the batch `Parameters[validation]` shape — v1 abandons the batch route contract entirely. Cancel affordance not in v1 (§7.11 v2). |

## 8. ConceptMap `$translate` — `/ui/hts/concept-maps/{id}/translate`

| | |
|---|---|
| Route | `POST /ui/hts/concept-maps/{id}/translate` |
| HTS calls | `POST /ConceptMap/{id}/$translate` or `POST /ConceptMap/$translate` |
| Inputs | Map source (canonical / id), forward/reverse toggle, source input-mode (`code`+`system` / `Coding` / `CodeableConcept`), target constraints (`targetCode` for reverse, `targetSystem`, `source`, `target`), `date` |
| Output | `Parameters`: repeatable `match` groups, each with `concept`, R4/R4B `equivalence` **or** R5/R6 `relationship`, `originMap` (forward) / `source` (reverse); trailing `result` |
| Fragment | `partials/hts-translate-matches.html` — match grid with columns `code`, `system`, `display`, `equivalence`/`relationship`, `origin` |
| Edge cases | Version-of-ConceptMap, dependency, and lowercase `targetsystem` are **not** accepted — do not add form fields; no matches returns HTTP 200 with `result=false`; equivalence vs. relationship column depends on compiled FHIR version — read from response, do not assume |

## 9. ConceptMap `$closure` — `/ui/hts/concept-maps/closure`

| | |
|---|---|
| Route | `POST /ui/hts/concept-maps/closure` |
| HTS calls | `POST /ConceptMap/$closure` |
| Inputs | Closure `name`, repeatable Coding rows (`system` + `code`) |
| Output | `Parameters` → `return.resource` = `ConceptMap` with `equivalence=subsumes` relationships |
| Fragment | `partials/hts-closure-graph.html` — simple edge list; optional SVG DAG |
| Edge cases | Implementation is **stateless** — the UI must clearly label this "not a durable closure session"; the `version` parameter is accepted but never surfaced in the response — do not depend on it; initial name-only request returns an empty ConceptMap by design |

## 10. Root batch workbench — `/ui/hts/batch` (v2 — **deferred**)

> **v1 status:** deferred — the root batch page is not exposed in the nav (which is limited to three entry URLs); the
> field / fragment matrix below stays as a v2 reference sketch.

| | |
|---|---|
| Route | `POST /ui/hts/batch` |
| HTS calls | `POST /` with FHIR Bundle |
| Inputs | Bundle editor (`type` = `batch` / `transaction`); entry-add wizard restricted to the three supported URLs: `CodeSystem/$validate-code`, `ValueSet/$validate-code`, `ConceptMap/$translate` |
| Output | `batch-response` Bundle with per-entry status + `resource` |
| Fragment | `partials/hts-batch-result.html` — per-entry cards with status pill and inline OperationOutcome |
| Edge cases | `transaction` is **not atomic** — banner must say so; `entry.request.method` is ignored by HTS — hide it from the form; unsupported entry URLs produce entry-level 400 OperationOutcome — surface, do not abort UI |

## 11. Import — `/ui/hts/import`

| | |
|---|---|
| Route | `POST /ui/hts/import` |
| HTS calls | `POST /import` (raw JSON Bundle) |
| Inputs | File upload (JSON only) or paste-in Bundle editor; import-mode label ("bundle-import — non-FHIR summary response") |
| Output | Non-FHIR JSON: `{ code_systems, value_sets, concept_maps, concepts, errors[] }` |
| Fragment | `partials/hts-import-summary.html` — count cards + non-fatal error list |
| Edge cases | XML upload is **not** supported; response is not a FHIR resource — do not run OperationOutcome parsing on it; HTTP 207 = partial success (errors present); 400 = bad JSON / non-Bundle root; bundled data catalog lives at `crates/hts/terminology-data/`, and the import path that consumes it is `crates/hts/src/import/` |

## 12. Bootstrap ledger — `/ui/hts/bootstrap`

| | |
|---|---|
| Route | `GET /ui/hts/bootstrap` |
| HTS calls | Read the `bootstrap_imports` table via a new UI-side read handler (no HTS route exposes this) |
| Inputs | none |
| Output | Table: filename, size, hash, mtime, languages selected, last import timestamp |
| Fragment | `partials/hts-bootstrap-table.html` |
| Edge cases | The UI crate is architecturally forbidden from opening HTS's database directly (`crates/ui/src/lib.rs` module doc: storage stays behind the ordinary API). Ship this page only after HTS grows an admin route (e.g. `GET /admin/bootstrap`) that returns the ledger as JSON — do not add a direct-SQL path. Must respect tenancy if a future HTS gains multi-tenancy |

## 13. Search browsers — `/ui/hts/{code-systems, value-sets, concept-maps}`

| | |
|---|---|
| Route | `GET /ui/hts/{resource}` |
| HTS calls | `GET /{Resource}?url=&version=&name=&title=&status=&_count=&_offset=&_summary=true` |
| Inputs | Facet form (`url`, `version`, `name`, `title`, `status`), `_count`, page controls |
| Output | `searchset` Bundle — `entry[]` and page `total` |
| Fragment | `partials/hts-<resource>-search-results.html` |
| Edge cases | `total` is a **page count**, not total matches — label it "on this page (n)"; no pagination links → compute next/prev from `_offset` + `_count`; `_sort`, `_include`, chained params, `_id`, `_elements`, modifiers, and accurate `_total` are **not** implemented — do not offer them in the form; ConceptMap ignores `_summary=true` — do not render the toggle on that tab, or disable it |

## 14. Metadata / diagnostics — `/ui/hts/diagnostics`

| | |
|---|---|
| Route | `GET /ui/hts/diagnostics` |
| HTS calls | `GET /metadata`, `GET /metadata?mode=terminology`, optional `GET /metrics` |
| Inputs | Mode toggle (CapabilityStatement / TerminologyCapabilities), search-parameter viewer |
| Output | Rendered capability trees + advertised expansion/validation parameters |
| Fragment | `partials/hts-capability-tree.html` |
| Edge cases | Server advertises `$versions` at server-level but **no route** — flag as "advertised but not implemented"; TerminologyCapabilities lists `includeDefinition` in `expansion.parameter` — flag as "advertised but not read"; hide `/metrics` panel unless the deployment permission model allows exposing Prometheus data |

---

## Cross-cutting UI patterns

### Fragment naming

- Page shells: `templates/pages/hts-<screen>.html`
- Swap targets: `templates/partials/hts-<op>-<part>.html`
- Shared HTS-specific alerts: `templates/partials/hts-outcome.html`
- Icons: reuse existing `templates/icons/*.svg`

### Server handler skeleton (pattern only — not copy-paste)

The handler must live inside `crates/ui/src/hts/` — `RequestVersion`,
`RequestTenant`, `RequestLocale`, and `WebState` are `pub(crate)` in the
`helios-ui` crate. They are not reachable from a sibling crate.

```rust
pub(crate) async fn expand_page(
    State(state): State<WebState>,
    RequestLocale(locale): RequestLocale,
    RequestVersion(version): RequestVersion,
    RequestTenant(tenant): RequestTenant,
    Path(id): Path<String>,
) -> impl IntoResponse {
    // Guard: if state.terminology is None, render the "unavailable" partial.
    // Otherwise call helios_fhirpath::TerminologyClient (already wraps auth/timeout)
    // OR a bespoke reqwest for $batch-validate-code / $closure / CRUD.
}
```

Reuse [helios_fhirpath::TerminologyClient](../../crates/fhirpath/src/terminology_client.rs)
for `$lookup`, `$validate-code`, `$subsumes`, `$translate`, and `$expand`;
add a fresh client for `$batch-validate-code`, `$closure`, `/import`, root
batch, and CRUD (none exist in the workspace today).

### Loading, empty, and error states

Every fragment needs three states rendered by the same template:

1. **Loading** — htmx `hx-indicator` targeting an inline spinner
2. **Empty** — semantic empty-state block (e.g. "no matches for filter")
3. **Error** — `partials/hts-outcome.html` for `OperationOutcome`;
   generic banner for pre-handler failures (408, 413, 415, malformed JSON)

`result=false` on validation / translation is **not** an error state — render
it neutral, not red.

### Tenant + language propagation

The current `/ui/editor/expand` handler does not propagate tenant or
`Accept-Language`. Do not repeat the omission. Every new HTS handler:

- Reads `RequestTenant` and forwards `X-Tenant-ID` when the deployment
  terminates tenancy at HTS
- Reads `RequestLocale` and sets `displayLanguage` on the outbound
  operation call (belt-and-braces over `Accept-Language`, which several HTS
  operations ignore)
