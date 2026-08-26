# Slice B output — CodeSystem browser + detail with embedded workbench

**Design ref:** `edson/docs/hts-ui-design.md` §7.2 / §7.3 / §7.3.1.
**Plan ref:** `c:\Users\tercere\.cursor\plans\hts_ui_delivery_strategy_8b4bcd79.plan.md` (Phase 2 slice B).
**Status:** completed 2026-08-18. Reconstructed from disk state + design doc §7.3.1 implementation notes + plan snapshot (previous chat transcript saturated).
**Branch:** `feat/551-hts-ui` (uncommitted; single-push discipline — see plan Phase 6).

## Deliverable

- Full page + fragment dual-mode CodeSystem browser at `/ui/hts/code-systems`.
- Detail page `/ui/hts/code-systems/{id}` with tab strip `Metadata | Lookup | Validate | Subsumes` (four tabs).
- Embedded workbench input + result partials for the three CS ops (`$lookup`, `$validate-code`, `$subsumes`).
- Every workbench POST proxies to HTS as POST regardless of source form verb (design doc §7.6 proxy verb rule).

## Files added (all absolute paths under `c:\Users\tercere\src\helios\hfs\`)

### Rust source
- `crates\hts-ui\src\code_systems.rs` (~28060 bytes) — handlers + `CsTab` enum (Metadata / Lookup / Validate / Subsumes).
- `crates\hts-ui\src\upstream.rs` — additions:
  - `search_code_systems`, `read_code_system`
  - `cs_lookup(&self, params: &LookupParams) -> Result<LookupResult, UpstreamError>`
  - `cs_validate_code(&self, source, params: &ValidateCodeParams) -> Result<ValidateCodeResult, UpstreamError>`
  - `cs_subsumes(&self, params: &SubsumesParams) -> Result<SubsumesResult, UpstreamError>`
  - `UpstreamClient::new_with_timeouts(base_url, request_timeout, connect_timeout)` — test-only fast path (100 ms / 250 ms) so the Windows `reqwest`-on-closed-loopback stack does not blow the wallclock budget of the route-enum matrix. Production still uses `new` (2 s / 5 s).
  - Types: `LookupParams`, `LookupResult`, `LookupProperty`, `LookupDesignation`, `SubsumesParams`, `SubsumesResult`, `ValidateCodeParams`, `ValidateCodeResult`, `ValidateInputMode`, `CsBrowserFilters`, `CsBrowserPage`, `CsBrowserRow`, `CodeSystemSummary`, `OutcomeView`.

### Templates
- `crates\hts-ui\templates\pages\cs-browser.html`
- `crates\hts-ui\templates\pages\cs-detail.html`
- `crates\hts-ui\templates\partials\hts-cs-rows.html`
- `crates\hts-ui\templates\partials\hts-cs-lookup-input.html`
- `crates\hts-ui\templates\partials\hts-cs-validate-input.html`
- `crates\hts-ui\templates\partials\hts-cs-subsumes-input.html`
- `crates\hts-ui\templates\partials\hts-cs-workbench-result.html` (shared across all three CS ops — per-op result partial family Slice C/D also follow)
- `crates\hts-ui\templates\partials\hts-outcome.html` (shared `OperationOutcome` renderer — added in Slice A; Slice B is first heavy user)

### Rust tests
- `crates\hts-ui\tests\code_systems.rs` — 8 `#[tokio::test]` functions:
  - `browser_renders_full_page_with_translated_heading`
  - `browser_rows_fragment_vary_on_htmx_request`
  - `browser_over_max_count_renders_invalid_input_outcome`
  - `browser_rejects_over_max_count_partial_shape_too`
  - `detail_renders_shell_and_outcome_on_upstream_failure`
  - `detail_soft_deleted_would_render_outcome_not_page_404`
  - `lookup_input_hx_renders_input_partial_only`
  - `lookup_run_without_code_renders_invalid_input_outcome`
- `crates\hts-ui\tests\route_enum.rs` — merged matrix + shell-marker walker in a single `#[tokio::test]` (design doc §7.3.1 — splitting the walks re-triggers the Windows `reqwest::Client` handle-drop abort `STATUS_INVALID_HANDLE = 0xFFFFFFFF`). CS routes added: `/ui/hts/code-systems`, `/ui/hts/code-systems/rows`, `/ui/hts/code-systems/does-not-exist`.

### Locales
- `locales\en\main.ftl`, `locales\es\main.ftl`, `locales\de\main.ftl` — added `hts-cs-browser-*`, `hts-cs-detail-*`, `hts-cs-lookup-*`, `hts-cs-validate-*`, `hts-cs-subsumes-*`, and shared `hts-workbench-*`, `hts-outcome-*` prefixes. Fluent key parity across en/es/de is a build-time assertion via `fluent-syntax` in dev-deps.

### Cargo
- `crates\hts-ui\Cargo.toml` — dependency additions:
  - `form_urlencoded = "1"` — needed for multi-map POST body parsing (repeatable `property` checkboxes on the Lookup form; `axum::Form` collapses those).
  - `helios-fhir` (default-features = false) + FHIR version features (`R4`/`R4B`/`R5`/`R6`).
  - `serde`, `serde_json`, `reqwest`, `async-trait`, `thiserror`, `tokio`.

## Routes registered (`crates\hts-ui\src\code_systems.rs::routes()`)

| Verb | Path | Handler |
|---|---|---|
| GET  | `/hts/code-systems` | `browser_page` |
| GET  | `/hts/code-systems/rows` | `browser_rows` |
| GET  | `/hts/code-systems/{id}` | `detail_page` (Metadata tab default) |
| GET  | `/hts/code-systems/{id}/lookup` | `lookup_input` (full page on hard nav, partial on HX) |
| POST | `/hts/code-systems/{id}/lookup` | `lookup_run` |
| GET  | `/hts/code-systems/{id}/validate` | `validate_input` |
| POST | `/hts/code-systems/{id}/validate` | `validate_run` |
| GET  | `/hts/code-systems/{id}/subsumes` | `subsumes_input` |
| POST | `/hts/code-systems/{id}/subsumes` | `subsumes_run` |

Merged in `crates\hts-ui\src\lib.rs::router()` alongside dashboard / value-sets / concept-maps.

## Key decisions (all pinned by tests; source = design doc §7.3.1)

1. **`_count > MAX_COUNT` clamp = HTTP 200 + OperationOutcome, not 400.** `MAX_COUNT = 100`. The browser is a discovery surface; a broken pager is worse than a silently-defaulted one. Outcome partial keeps the divergence operator-visible.
2. **Terminal-page pager.** HTS's `Bundle.total` is a page count, not an authoritative match count. `[Load more]` uses `rows.len() >= requested` heuristic (design doc §7.3.1 + hts-details `§Search`).
3. **CS `$validate-code` = type-level only.** HTS has no CS instance route; the Validate tab resolves canonical URL from the detail-page read and POSTs to `/CodeSystem/$validate-code`. `CodeableConcept` mode + version pins + `useSupplement` all defer to Slice E (widened superset, §7.6 F4).
4. **CS `$subsumes` = both codes pinned to canonical URL server-side.** Form asks only `codeA`, `codeB`, optional `version` (hts-details `§$subsumes` requires codeA/codeB to share a system).
5. **404 → OperationOutcome inside shell, HTTP 200.** HTS returns 404 for both truly-missing and soft-deleted resources; the UI cannot tell them apart at the HTTP layer. Never a hard page 404.
6. **`form_urlencoded` multi-map.** The Lookup form uses repeatable `property` checkboxes; `axum::Form` (`serde_urlencoded`) collapses duplicates. Slice B added the direct `form_urlencoded = "1"` dep and hand-parses the POST body into a `Vec<(String, String)>`.
7. **Merged route-enum matrix + shell-marker walk in a single `#[tokio::test]`.** Splitting them into sibling tokio-tests reintroduces the Windows `reqwest::Client` drop-then-reinit socket leak that aborts with `STATUS_INVALID_HANDLE`. The matrix runs the shell-marker assertion inline on its `en, no-hx` cell.
8. **`UpstreamClient::new_with_timeouts` test-only path.** 100 ms connect / 250 ms request keeps the 30-request matrix under a minute against `127.0.0.1:1` on Windows.
9. **Slice B route-enum matrix scope = browser + rows fragment + one detail path only.** Every workbench tab + POST is exercised by `tests/code_systems.rs` directly; walking each through locale × HX-Request combinations multiplies request counts without new coverage.

## Test count reconstruction (from plan's todo note "26/26 tests green on 2026-08-18")

- 4 dashboard/router_http (Slice A carry-over: `dashboard_serves_full_page_at_ui_hts`, `dashboard_advertises_vary_hx_request_for_htmx_caching`, `assets_serve_the_embedded_bundle_under_ui_hts_assets`, `dashboard_localizes_via_accept_language_when_no_query_or_cookie`).
- 8 CS integration in `tests/code_systems.rs` (list above).
- 2 route-enum (merged matrix walk + `unknown_route_under_ui_hts_returns_404`).
- 12 unit tests inside `src/` (dashboard, i18n, upstream helpers).

## e2e (Playwright) — added under `crates\hts-ui\e2e\tests\`

- `code-systems.spec.ts` — 5898 bytes. Guards: browser page loads, filter POST hits rows fragment, detail page renders shell + tabs, workbench Lookup submit round-trip, axe-core baseline.

## Debt carried forward for Slice E

- CS `$validate-code` `CodeableConcept` mode + version pins + `useSupplement[]` (§7.6 F4 widening table).
- Shared workbench partial id rename (`#hts-cs-workbench-input/result` → `#hts-workbench-input/result`, §7.6 F15). Slice E's PR touches the CS templates + integration tests to match.
- The Slice B/C/D per-op result partial family stays; abstract `hts-concept` / `hts-match` renderer refactor defers to a **Phase 3 mini-slice** (§7.6.1 F11 = A resolution).

## Cross-check vs git (2026-08-18)

- `crates\hts-ui\` is entirely **untracked** (matches plan Phase 6 discipline — single push at the end).
- Tracked but modified: `Cargo.lock`, `Cargo.toml`, `crates\hts\Cargo.toml`, `crates\hts\src\config.rs`, `crates\hts\src\server.rs` (mount `/ui` under `HTS_UI_ENABLED`), `locales\{en,es,de}\main.ftl` (~400 lines each).
- No merge conflicts anticipated with `origin/main` — Slice B is additive under a new crate.
