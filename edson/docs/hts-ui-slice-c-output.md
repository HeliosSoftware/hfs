# Slice C output — ValueSet browser + detail with `$expand` tab

**Design ref:** `edson/docs/hts-ui-design.md` §7.4 / §7.4.1.
**Plan ref:** `c:\Users\tercere\.cursor\plans\hts_ui_delivery_strategy_8b4bcd79.plan.md` (Phase 2 slice C).
**Status:** completed 2026-08-18. Reconstructed from disk state + design doc §7.4.1 implementation notes + plan snapshot (previous chat transcript saturated).
**Branch:** `feat/551-hts-ui` (uncommitted).
**Opus 4.7 advisor pre-integration:** COMPLETE (agent 35d76f6f, 14 findings triaged; design doc §7.4 rewritten + §7.4.1 implementation-notes block added; §7.6 threshold clause softened to per-request; §7.10 matrix updated with tree-mode row; ui-design-map §5 rewritten; §6 VS `$validate-code` marked deferred to Slice E).

## Deliverable

- Full page + fragment dual-mode ValueSet browser at `/ui/hts/value-sets` (mirrors Slice B's CS browser).
- Detail page `/ui/hts/value-sets/{id}` with tabs `Metadata | Expand` (only two — VS `$validate-code` defers to Slice E per F9).
- Embedded `$expand` workbench input + result partial, exposing 14 of 15 HTS `$expand` params inline.
- Tree/flat toggle, `X-TOO-COSTLY-THRESHOLD` header handling with Advanced `<details>` panel, filter-no-match neutral state.

## Files added

### Rust source
- `crates\hts-ui\src\value_sets.rs` (~23596 bytes) — handlers + `VsTab` enum (Metadata / Expand).
- `crates\hts-ui\src\upstream.rs` — additions:
  - `search_value_sets`, `read_value_set(&self, id) -> Result<ValueSetSummary, UpstreamError>`
  - `vs_expand_instance(&self, id, params: &ExpandParams) -> Result<ExpansionResult, UpstreamError>` — POST to `/ValueSet/{id}/$expand`. Reads/writes the `X-TOO-COSTLY-THRESHOLD` header when the per-request threshold ≤ `HTS_UI_MAX_EXPANSION_SIZE_HINT`, drops it above.
  - Types: `ExpandParams`, `ExpansionResult`, `ExpansionConcept`, `ExpansionDesignation`, `VsBrowserFilters`, `VsBrowserPage`, `VsBrowserRow`, `ValueSetSummary`. Constant `HTS_UI_MAX_EXPANSION_SIZE_HINT: u64 = 100_000` (exported from `lib.rs`).
  - Helper `ExpansionResult::flat_tree_rows()` — walks `contains[]` once in Rust, emits `(depth, code, system, display, has_children)` tuples. See "Tree rendering" decision below.

### Templates
- `crates\hts-ui\templates\pages\vs-browser.html`
- `crates\hts-ui\templates\pages\vs-detail.html`
- `crates\hts-ui\templates\partials\hts-vs-rows.html`
- `crates\hts-ui\templates\partials\hts-vs-expand-input.html` (~9036 bytes)
- `crates\hts-ui\templates\partials\hts-vs-expand-result.html` (~11133 bytes)

### Rust tests
- `crates\hts-ui\tests\value_sets.rs` — **16** `#[tokio::test]` functions:
  - `browser_renders_full_page_with_translated_heading`
  - `browser_rows_fragment_targets_and_varies_on_htmx_request`
  - `browser_over_max_count_renders_invalid_input_outcome`
  - `detail_renders_shell_and_degraded_on_upstream_failure`
  - `detail_unknown_id_renders_outcome_inside_shell`
  - `expand_tab_htmx_returns_input_partial_only`
  - `expand_input_shows_advanced_details_and_threshold_field`
  - `expand_tree_mode_sends_hierarchical_true_and_no_exclude_nested`
  - `expand_flat_mode_sends_exclude_nested_true_and_no_hierarchical`
  - `expand_flat_renders_load_more_when_total_exceeds_page`
  - `expand_tree_hides_pager_and_labels_total_leaves`
  - `expand_422_renders_too_costly_banner_with_raise_form`
  - `expand_threshold_below_ceiling_attaches_x_too_costly_header`
  - `expand_threshold_above_ceiling_drops_header_and_warns`
  - `expand_no_members_renders_neutral_state`
  - `expand_filter_no_match_renders_neutral_state_with_filter`
- In-process axum mock upstream (`start_mock`) with a `/__mock_ready` probe polled at 10 ms until 2 s deadline before returning the base URL — see "Mock ready-probe" decision below.
- `tests/route_enum.rs` extended: three new rows (`/ui/hts/value-sets`, `/ui/hts/value-sets/rows`, `/ui/hts/value-sets/does-not-exist`).

### Locales
- `hts-vs-browser-*`, `hts-vs-detail-*`, `hts-vs-expand-*` added to `locales\{en,es,de}\main.ftl`. Enumerated per design doc §7.4 i18n bullet: 14 param labels (`filter`, `count`, `offset`, `displayLanguage`, `activeOnly`, `includeDesignations`, `useSupplement`, `date`, `property`, `tx-resource`, `system-version`, `check-system-version`, `force-system-version`, `default-valueset-version`), `tree` / `flat` toggle, `showing full tree {N}`, `expansion.total` / `expansion.offset`, `no-members`, `filter-no-match`, `raise-threshold`, `why?`, `threshold-numeric`, `ceiling-warning`, `tree-node-expand`/`collapse`, `advanced-summary`.
- Shared `hts-workbench-*` (raw response, copy url, format json/xml, run) reused from Slice B.
- VS `$validate-code` keys (`hts-vs-validate-*`) intentionally NOT added — defer to Slice E.

## Routes registered

| Verb | Path | Handler |
|---|---|---|
| GET  | `/hts/value-sets` | `browser_page` |
| GET  | `/hts/value-sets/rows` | `browser_rows` |
| GET  | `/hts/value-sets/{id}` | `detail_page` |
| GET  | `/hts/value-sets/{id}/expand` | `expand_input` |
| POST | `/hts/value-sets/{id}/expand` | `expand_run` |

## Slice-C-specific decisions (from Opus 4.7 advisor triage + implementation)

- **Result partial family = per-op.** `hts-vs-expand-result.html` mirrors Slice B's `hts-cs-workbench-result.html`. Abstract `hts-concept` renderer (design doc §6.3) stays aspirational — cross-slice refactor deferred to Phase 3 mini-slice (§7.6.1 F11 = A).
- **Threshold storage = per-request hidden form field named `threshold`.** No cookies, no session store. Advanced `<details>` numeric input and banner "Raise" action both bind to the same input; value echoes on the next Expand submit. §7.6 original "session-scoped" wording is explicitly superseded.
- **Inline field set = 14 of 15 `$expand` params.** `designation[]` (repeatable filter) defers to Slice E (§7.4.1 F2). `includeDefinition` is advertised by HTS's CapabilityStatement but ignored server-side — NO UI toggle emitted.
- **VS `$validate-code` deferred (F9 triage).** Validate tab removed from §7.4; reachable via Slice E's standalone workbench at `/ui/hts/operations?op=validate-code&resource=ValueSet`.
- **Tree/flat mapping is authoritative (F7 triage).** `tree` ⇒ `hierarchical=true`; `flat` ⇒ `excludeNested=true`. No dual-flag emission, no `auto` state — pinned by two dedicated tests (`_sends_hierarchical_true_and_no_exclude_nested`, `_sends_exclude_nested_true_and_no_hierarchical`).
- **Pager rule (F6 + F10 triage).** Flat mode: `remaining = expansion.total - expansion.offset - contains.len()`; hide `[Load more]` when `remaining ≤ 0` or `expansion.total` absent (falls back to §7.3.1 terminal-page heuristic). Tree mode: pager hidden entirely; metadata line renders `showing full tree {N}` — HTS ignores `count`/`offset` in tree mode.
- **`too-costly` control.** Both banner action and Advanced numeric input write to hidden `threshold` field. Values ≤ `HTS_UI_MAX_EXPANSION_SIZE_HINT` (100_000) → `X-TOO-COSTLY-THRESHOLD` header attached. Above ceiling → header dropped, warning rendered inline. See tests `_below_ceiling_attaches_x_too_costly_header` and `_above_ceiling_drops_header_and_warns`.
- **Membership-`result=false` neutral state (F11 triage).** Reserved for VS `$validate-code` when it ships in Slice E. Shared error partial MUST NOT fire on HTTP 200 with `result=false` (analog to Slice D's CM Translate pattern).
- **nojs = flat-only (F14 triage).** In a nojs browser the tree/flat toggle is a plain form GET-submit that re-renders the page with `hierarchical` / `excludeNested` baked into the URL; tree ARIA affordances do not activate without JS.

## Implementation notes discovered while landing Slice C (design doc §7.4.1 tail)

- **Tree rendering = flat-in-Rust, not recursive-in-Askama.** Askama's derive expands templates at compile time and hits its stack limit on self-including partials, so the recursive `hts-vs-expand-node.html` pattern the wireframe implied is not viable. `ExpansionResult::flat_tree_rows()` walks `contains[]` once in Rust; the tree-mode loop in `hts-vs-expand-result.html` indents with a `padding-inline-start: {depth}rem` inline style; `role="tree"` still wraps the whole list. Wire contract (`hierarchical=true`) unchanged.
- **Mock-upstream ready-probe.** `tests/value_sets.rs::start_mock` binds `axum::serve` on `127.0.0.1:0` and polls a `/__mock_ready` route (10 ms interval / 2 s deadline) before returning the base URL. Under the Windows current-thread `#[tokio::test]` runtime the spawned mock task can trail the first client request by several milliseconds — the probe keeps client-side timeouts tight without producing phantom `Connect` failures. Closed-loopback tests (127.0.0.1:1) keep the 100 ms / 250 ms envelope from §7.3.1.

## Slice B invariants inherited (design doc §7.4.1 explicit)

1. `_count` clamp — over-max is HTTP 200 + `OperationOutcome`, not HTTP 400.
2. `form_urlencoded` multi-map — repeatable POST fields (`useSupplement[]`, `property[]`, `tx-resource[]`, all three version pins) survive round-trip.
3. `UpstreamClient::new_with_timeouts` in tests (100 ms / 250 ms).
4. Canonical URL resolution at page render — detail-embedded Expand resolves the instance's `ValueSet.url` (and version) once and pins subsequent op calls.
5. 404 → `OperationOutcome` in shell — HTTP 200, never a page 404.
6. Merged route-enum matrix walker — VS routes appended to the single `#[tokio::test]` walker; splitting re-triggers the Windows `STATUS_INVALID_HANDLE` abort.

## e2e (Playwright) — added under `crates\hts-ui\e2e\tests\`

- `value-sets.spec.ts` — 7056 bytes. Guards: browser + filter + rows fragment, detail page + Expand tab, tree/flat toggle round-trip, too-costly banner render, axe-core.

## Debt carried to Slice E

- `designation[]` chip multi-select (§7.6 F4 widening).
- Three-way ValueSet source selector (canonical URL / instance id / inline JSON textarea) for the standalone workbench (§7.6 F3 scope-Free mode).
- VS `$validate-code` full input matrix (all three modes: `code` / `Coding` / `CodeableConcept`).
- Shared workbench partial id rename (§7.6 F15).

## Cross-check vs git

Files under `crates\hts-ui\` and `locales\` are untracked/modified as summarized in Slice B output; nothing pushed remotely (Phase 6 discipline).
