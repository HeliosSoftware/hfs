# Slice D output — ConceptMap browser + detail with `$translate` tab

**Design ref:** `edson/docs/hts-ui-design.md` §7.5 / §7.5.1.
**Plan ref:** `c:\Users\tercere\.cursor\plans\hts_ui_delivery_strategy_8b4bcd79.plan.md` (Phase 2 slice D).
**Status:** completed 2026-08-18. Reconstructed from disk state + design doc §7.5.1 implementation notes + plan snapshot.
**Branch:** `feat/551-hts-ui` (uncommitted).

## Deliverable

- Full page + fragment dual-mode ConceptMap browser at `/ui/hts/concept-maps`.
- Detail page `/ui/hts/concept-maps/{id}` with tabs `Metadata | Translate` (only two).
- Embedded `$translate` workbench input + result partials; forward/reverse direction toggle; match-grid column heading pinned to first-match-wins `MappingKind` (`equivalence` on R4/R4B, `relationship` on R5/R6 — read from the response, never from the compiled FHIR version).
- Inline pre-flight validation: missing `code`+`system` (forward) or missing `targetCode` (reverse) renders a synthetic `OperationOutcome` in the result region without touching HTS. Tests assert the mock upstream sees zero incoming requests when the gate fires.

## Files added

### Rust source
- `crates\hts-ui\src\concept_maps.rs` (~22983 bytes) — handlers + `CmTab` enum (Metadata / Translate).
- `crates\hts-ui\src\upstream.rs` — additions:
  - `search_concept_maps`, `read_concept_map(&self, id) -> Result<ConceptMapSummary, UpstreamError>`
  - `cm_translate_instance(&self, id, params: &TranslateParams) -> Result<TranslateResult, UpstreamError>` — POST to `/ConceptMap/{id}/$translate`.
  - Types: `TranslateParams`, `TranslateResult`, `TranslateMatch`, `TranslateDirection`, `MappingKind`, `CmBrowserFilters`, `CmBrowserPage`, `CmBrowserRow`, `ConceptMapSummary`.
  - `MappingKind` first-match-wins parser: locks in the response-level kind from the first `match` group and reuses it for the whole grid heading. HTS emits either `equivalence` (R4/R4B) or `relationship` (R5/R6) uniformly across every `match` in a single response — the two field names never coexist. Fluent selector `hts-cm-translate-column-mapping` reads the lowercase kind string (`equivalence` / `relationship` / `unknown`) so no cfg-ladder is needed. This is what makes an R4-compiled UI legible when pointed at an R5 HTS via `HTS_UI_UPSTREAM_URL`, and vice versa.

### Templates
- `crates\hts-ui\templates\pages\cm-browser.html`
- `crates\hts-ui\templates\pages\cm-detail.html`
- `crates\hts-ui\templates\partials\hts-cm-rows.html`
- `crates\hts-ui\templates\partials\hts-cm-translate-input.html` (~7060 bytes)
- `crates\hts-ui\templates\partials\hts-cm-translate-result.html` (~6102 bytes)

### Rust tests
- `crates\hts-ui\tests\concept_maps.rs` — **15** `#[tokio::test]` functions (35636 bytes):
  - `browser_renders_full_page_with_translated_heading`
  - `browser_rows_fragment_targets_and_varies_on_htmx_request`
  - `browser_over_max_count_renders_invalid_input_outcome`
  - `detail_renders_shell_and_degraded_on_upstream_failure`
  - `detail_unknown_id_renders_outcome_inside_shell`
  - `translate_tab_htmx_returns_input_partial_only`
  - `translate_forward_posts_code_and_system_parameters`
  - `translate_reverse_posts_target_code_parameter`
  - `translate_reverse_without_target_code_renders_inline_validation_outcome_without_posting_to_hts`
  - `translate_forward_without_code_renders_inline_validation_outcome_without_posting_to_hts`
  - `translate_no_matches_renders_neutral_state_not_error`
  - `translate_r4_response_labels_column_as_equivalence`
  - `translate_r5_response_labels_column_as_relationship`
  - `translate_hts_error_renders_outcome_partial`
  - `translate_does_not_expose_unsupported_params`
- Reuses `start_mock` ready-probe pattern from Slice C.
- `tests/route_enum.rs` extended: `/ui/hts/concept-maps`, `/ui/hts/concept-maps/rows`, `/ui/hts/concept-maps/does-not-exist`.

### Locales
- `hts-cm-browser-*`, `hts-cm-detail-*`, `hts-cm-translate-*` added to `locales\{en,es,de}\main.ftl`. Includes column-heading Fluent selector on `hts-cm-translate-column-mapping` keyed by lowercase kind string.
- Shared `hts-workbench-*` reused from Slice B/C.

## Routes registered

| Verb | Path | Handler |
|---|---|---|
| GET  | `/hts/concept-maps` | `browser_page` |
| GET  | `/hts/concept-maps/rows` | `browser_rows` |
| GET  | `/hts/concept-maps/{id}` | `detail_page` |
| GET  | `/hts/concept-maps/{id}/translate` | `translate_input` |
| POST | `/hts/concept-maps/{id}/translate` | `translate_run` |

## Slice-D-specific decisions

- **Reverse-direction wire shape.** `POST /ConceptMap/{id}/$translate` emits `reverse=true` (`valueBoolean`) plus `targetCode` (`valueCode`) in reverse mode; source-side `code` / `system` are dropped from the payload entirely rather than swapped. The FHIR R4 spec allows both shapes and HTS accepts either, but the reverse-mode form's source group intentionally does not surface `code`/`system` inputs (§7.5 wireframe), so the emitter mirrors the visible controls. Tests assert both directions bit-exactly.
- **First-match-wins for mapping kind.** Response-level `MappingKind` locked in from the first `match` and reused across all rows + column heading (visible text + `aria-label`).
- **`origin` column collapses forward and reverse URIs.** Grid keeps 5 columns regardless of direction. Forward mode's `originMap` and reverse mode's source part both flow into a single `origin: Option<String>` field on `TranslateMatch`; template renders whichever is present.
- **Direction-toggle re-render.** Radios carry `hx-get="/ui/hts/concept-maps/{id}/translate?direction=…"` + `hx-target="#hts-cm-workbench-input"` so flipping the toggle fetches the appropriate source-group partial (forward: system/code/display; reverse: targetCode). Keeps the field set a11y-clean (no `display: none` on inputs that would still submit) and the same URL + query params work as nojs fallback.
- **Pre-flight validation gate.** Missing `code`+`system` (forward) or `targetCode` (reverse) renders synthetic `OperationOutcome` in the result region without a `$translate` round-trip — mirrors Slice B's `_count > MAX` pattern. Tests inspect the mock upstream's captured-request log: zero incoming requests when the gate fires.
- **No matches = neutral state, not error.** §7.5 F11 realized for CM (Slice C explicitly deferred the analog for VS `$validate-code` to Slice E). HTTP 200 with `result=false` renders the `hts-cm-workbench__no-matches` label; the shared error partial does NOT fire.
- **Result partial family = per-op.** `hts-cm-translate-result.html` ships alongside Slice B's CS partial + Slice C's VS partial. Cross-slice refactor deferred to Phase 3 mini-slice (§7.6.1 F11 = A resolution — Slice E's advisor pass locked this in).
- **Unsupported parameters excluded from the form.** `version` (of the ConceptMap), `dependency`, lowercase `targetsystem` alias — none surface in the form and the emitter mirrors that in the wire body. `translate_does_not_expose_unsupported_params` pins it.

## Slice B/C invariants inherited (design doc §7.5.1 explicit)

All six from §7.3.1 (Slice B) + Slice C additions apply verbatim:
1. `_count` clamp — HTTP 200 + `OperationOutcome`.
2. `form_urlencoded` multi-map.
3. `UpstreamClient::new_with_timeouts` in tests.
4. Canonical URL resolution at page render.
5. 404 → `OperationOutcome` in shell.
6. Merged route-enum matrix walker (Windows split-test hazard).

## e2e (Playwright) — added under `crates\hts-ui\e2e\tests\`

- `concept-maps.spec.ts` — 8438 bytes. Guards: browser, detail, direction toggle round-trip, no-matches neutral state, R4/R5 column heading, axe-core.

## Debt carried to Slice E

- `$translate` supersets (already complete in Slice D per §7.6 F4 table — nothing to widen).
- Shared workbench partial id rename (`#hts-cm-workbench-*` → `#hts-workbench-*`, §7.6 F15).
- `$closure` — Slice E ships from scratch, new `cm_closure` upstream method (§7.6.1 F19).

## Cross-check vs git

Files under `crates\hts-ui\` and `locales\` are untracked/modified; nothing pushed. Consistent with Slice B and C output docs.
