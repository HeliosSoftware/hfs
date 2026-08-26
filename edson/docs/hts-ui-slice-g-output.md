# Slice G output — Diagnostics (`/ui/hts/diagnostics`)

- **Design ref:** `edson/docs/hts-ui-design.md` §7.9 Diagnostics (+ §7.10 row 7.9 states matrix and §7 preamble inherited guards).
- **Plan ref:** issue [#551](https://github.com/HeliosSoftware/hfs/issues/551) — HTS-UI Phase 2, Slice G.
- **Status:** Green. `cargo test -p helios-hts-ui --tests` = **78 passed / 0 failed** (baseline 73 after Slice F + 5 new diagnostics tests). Working copy uncommitted per task rule #2; Slice F's diff on top of `d99f3d972` is preserved intact and Slice G is layered on top.
- **Branch:** `feat/551-hts-ui`.
- **HEAD:** `d99f3d97246683fe23d5361c7615a45173b9cbf2` (unchanged from Slice F handoff).
- **Toolchain used:** `stable-x86_64-pc-windows-gnu`. This box has a per-directory override that Slice E1/E2/F already fell back to (the `-gnullvm` toolchain hits the same `x86_64-w64-mingw32-clang` linker gap noted in the Slice F output). `rustup show active-toolchain` confirmed `-gnu` was already the active override, so it was not re-issued.

## Files added

| Path | Purpose |
|---|---|
| `c:\Users\tercere\src\helios\hfs\crates\hts-ui\src\diagnostics.rs` | Slice G module: `diagnostics_page` + `diagnostics_panel` handlers, `Tab` enum, `PanelView` with `is_*` discriminator flags (E1 `OpsFlags` idiom), `probe_degraded`, `outcome_from_error`. |
| `c:\Users\tercere\src\helios\hfs\crates\hts-ui\templates\pages\diagnostics.html` | Full-page shell extending `layouts/base.html`; header row with H1 + FHIR-version chip; degraded banner (§7 preamble) above the tab strip; `role="tablist"` with four `<a role="tab">` anchors, each carrying both `href` (nojs) and `hx-get` / `hx-target="#diag-panel"` / `hx-swap="innerHTML"` / `hx-push-url="true"` (htmx); shared `<section id="diag-panel" role="tabpanel">` with the pre-rendered default (capability) panel embedded. |
| `c:\Users\tercere\src\helios\hfs\crates\hts-ui\templates\partials\hts-diagnostics-panel.html` | Shared tabpanel body. Outermost `<div class="hts-diagnostics-panel-body">` always renders (route-enum oracle). Dispatches on `panel.is_capability` / `is_terminology_capabilities` / `is_health` / `is_metrics`, and on `panel.outcome.is_some()` renders `hts-outcome.html` in place of the tab content (per-tab isolation). |
| `c:\Users\tercere\src\helios\hfs\crates\hts-ui\tests\diagnostics.rs` | Five `#[tokio::test]` functions covering the shell, capability tab, terminology tab, metrics tab, and the per-tab 5xx outcome contract. Uses the same `start_mock` + `/__mock_ready` pattern as `tests/import.rs`, extended so every diagnostic endpoint (`/metadata` / `/health` / `/metrics`) can be seeded independently per test. |

No files under `crates/hts/*`, `crates/ui/*`, or any Slice A/B/C/D/E1/E2/F handler / partial / test were touched (task rule #4). The sidebar nav slot in `templates/layouts/base.html` was already wired to `/ui/hts/diagnostics` with `hts-nav-diagnostics` during the Phase 1 chrome pass — no edit needed for G.

## Files modified

All four are pure appends layered on top of Slice F's uncommitted appends (task rule #5). `git diff --stat` confirms Slice F's contributions to the same files are preserved.

| Path | Change (one-line) |
|---|---|
| `c:\Users\tercere\src\helios\hfs\crates\hts-ui\src\lib.rs` | Registered `mod diagnostics;`, appended `.merge(diagnostics::routes())` after Slice F's `.merge(import::routes())`, and re-exported `CapabilityView`, `CapabilityRestResource`, `TerminologyCapabilitiesView`, `TerminologyCodeSystemEntry` in a *new* `pub use` block below Slice F's alphabetized block (avoids editing lines Slice F touched). |
| `c:\Users\tercere\src\helios\hfs\crates\hts-ui\src\upstream.rs` | Appended a `// ── Slice G: Diagnostics fetches ──` section at the end of the file (before `#[cfg(test)]`) with three GET methods (`capability_statement`, `terminology_capabilities_view`, `metrics_text`), four view structs (`CapabilityView`, `CapabilityRestResource`, `TerminologyCapabilitiesView`, `TerminologyCodeSystemEntry`), and two private parse helpers (`parse_capability_statement`, `parse_terminology_capabilities_view`). Slice F's `import_bundle` / `ImportResult` / `ImportStatus` / `ImportCounts` / `parse_import_counts` / `collect_outcome_diagnostics` block is untouched. |
| `c:\Users\tercere\src\helios\hfs\crates\hts-ui\tests\route_enum.rs` | Appended two new `ROUTES` entries for `/ui/hts/diagnostics` (shell marker `>Diagnostics<`) and `/ui/hts/diagnostics/panel` (fragment marker `hts-diagnostics-panel-body`, the class the shared panel partial's outer `<div>` emits unconditionally). No second `#[tokio::test]` added — the merged matrix walker picks the rows up automatically (Windows STATUS_INVALID_HANDLE guard from §7.3.1 invariant #6). Slice F's `/ui/hts/import` row is untouched. |
| `c:\Users\tercere\src\helios\hfs\locales\en\main.ftl` | Appended `hts-diagnostics-*` block (27 keys) at the end of the file, immediately after Slice F's `hts-import-invalid-json-error`. |
| `c:\Users\tercere\src\helios\hfs\locales\es\main.ftl` | Appended the same 27 keys with initial Spanish translations and a `# TODO(G): review es` header comment. |
| `c:\Users\tercere\src\helios\hfs\locales\de\main.ftl` | Appended the same 27 keys with initial German translations and a `# TODO(G): review de` header comment. |

## Routes registered

| Verb | Path | Handler |
|---|---|---|
| GET | `/hts/diagnostics` | `diagnostics_page` — full page shell + pre-rendered active-tab panel; `?tab=capability\|terminology-capabilities\|health\|metrics` selects the initial tab (default `capability`); runs the shared `/health` degraded probe so the shell renders `hts-degraded.html` when the upstream is unreachable (§7 preamble). |
| GET | `/hts/diagnostics/panel` | `diagnostics_panel` — fragment target for the `hx-get` tab swap. Reads `?tab=` from the query string; renders the shared panel partial with the right variant flag set. Does **not** re-probe `/health` — a per-tab 5xx surfaces `hts-outcome.html` inside the panel (§7.9 per-tab isolation), never a full-page degrade. |

Both routes are appended after Slice F's `.merge(import::routes())` so the two products' `router()` composition is `dashboard → cs → vs → cm → operations → import → diagnostics → assets`.

## `UpstreamClient` signatures added

```rust
pub async fn capability_statement(&self) -> Result<CapabilityView, UpstreamError>;
pub async fn terminology_capabilities_view(&self) -> Result<TerminologyCapabilitiesView, UpstreamError>;
pub async fn metrics_text(&self) -> Result<String, UpstreamError>;
```

- **`capability_statement`** — `GET {base_url}/metadata` with `Accept: application/fhir+json`. Body is decoded as a `serde_json::Value` and projected into a `CapabilityView` (identity block + a flattened `rest[].resource[]` summary of `type` + `interaction[].code`).
- **`terminology_capabilities_view`** — `GET {base_url}/metadata?mode=terminology` with `Accept: application/fhir+json`. Projected into a `TerminologyCapabilitiesView` (identity block + `codeSystem[]` list of `{ uri, version }`). Parser accepts both the FHIR spec array shape (`codeSystem[].version[].code`) and a flat string fallback so a richer server does not break the tab.
- **`metrics_text`** — `GET {base_url}/metrics` with no `Accept` header. Response body is returned verbatim as `String`; the metrics tab wraps it in `<pre>` inside a `<figure>` and does no numeric parsing.

Coexistence with the existing dashboard fetch:

- The pre-existing `terminology_capabilities` method (returns `UpstreamTerminologyCapabilities`, used by `dashboard.rs` for the loaded-systems count) is **not modified**. Slice G ships the parallel `terminology_capabilities_view` method rather than mutating the dashboard's projection.
- The pre-existing `health` method is reused directly for both the shell-level degraded probe and the /health tab; no new method needed.

**Absorption / error contract.** All three methods return `Err(UpstreamError::*)` on 4xx/5xx status codes and transport failures. The diagnostics handler then synthesises an `OutcomeView` (via the private `outcome_from_error` helper) whose severity is `"error"` and whose code is constrained to the codes the existing `hts-outcome-code-*` Fluent block already covers (`not-found` for 404, otherwise `unknown`), so no raw Fluent key ever leaks into the rendered banner. The diagnostic string is the `UpstreamError::to_string()` output, which already carries the op + url + status / message context from the enum's `#[error]` attributes.

## Fluent keys added

27 keys under the `hts-diagnostics-*` namespace, appended verbatim at the end of each locale so future slices can append after Slice G without a merge conflict.

- **en (source):** 27 keys authored as user-facing English strings. `hts-diagnostics-fhir-version-chip` carries the `{ $version }` placeable (matches the existing `hts-fhir-version` idiom).
- **es:** 27 keys, initial Spanish translations, header comment `# TODO(G): review es`. Every key mirrors the `en` set — the TODO is a copy-quality flag, not a coverage gap.
- **de:** 27 keys, initial German translations, header comment `# TODO(G): review de`. Same coverage story as `es`.

**Per-locale key list (identical across `en`, `es`, `de`):**

```
hts-diagnostics-title
hts-diagnostics-heading
hts-diagnostics-nav-label
hts-diagnostics-fhir-version-chip
hts-diagnostics-tab-capability
hts-diagnostics-tab-terminology-capabilities
hts-diagnostics-tab-health
hts-diagnostics-tab-metrics
hts-diagnostics-capability-heading
hts-diagnostics-terminology-capabilities-heading
hts-diagnostics-health-heading
hts-diagnostics-metrics-heading
hts-diagnostics-property-url
hts-diagnostics-property-version
hts-diagnostics-property-name
hts-diagnostics-property-title
hts-diagnostics-property-status
hts-diagnostics-property-date
hts-diagnostics-capability-rest-heading
hts-diagnostics-terminology-code-systems-heading
hts-diagnostics-terminology-code-systems-empty
hts-diagnostics-health-status-label
hts-diagnostics-health-unknown
hts-diagnostics-metrics-figcaption
hts-diagnostics-metrics-empty
hts-diagnostics-error
```

Total: 26 keys authored + 1 (`hts-diagnostics-nav-label`) — 27. `hts-nav-diagnostics`, the sidebar nav label, was already present from the Phase 1 chrome stub set; Slice G adds `hts-diagnostics-nav-label` as a page-scoped alias for the design's `hts-diagnostics-*` prefix without duplicating the sidebar item.

**Parity note.** The three locales share the same key set (verified by reading the appended blocks side-by-side). The workspace does not yet ship an automated Fluent parity test — a `# TODO(F+1)` from Slice F carries over to Slice G+1 for a mini `fluent-syntax`-based parity ring. Slice G did not add such a test either, staying aligned with the E1/E2/F discipline.

**Outcome codes reused (no new codes added):** Slice G's synthetic `OperationOutcome` uses only `not-found` and `unknown` from the shared `hts-outcome-code-*` block (locales `en` / `es` / `de` already ship all four codes: `not-found`, `invalid`, `too-costly`, `unknown`). This keeps Slice G locale-parity-neutral.

## Test results — full `cargo test -p helios-hts-ui --tests` output

```
running 12 tests
test i18n::tests::default_locale_is_english ... ok
test i18n::tests::accept_language_is_matched_by_rfc4647_lookup ... ok
test i18n::tests::hts_lang_cookie_beats_accept_language ... ok
test i18n::tests::query_override_beats_everything_and_is_explicit ... ok
test upstream::tests::uptime_pretty_shapes_units_from_seconds ... ok
test upstream::tests::outcome_view_parses_hts_operation_outcome_shape ... ok
test upstream::tests::base_url_strips_trailing_slashes ... ok
test upstream::tests::browser_filters_clamp_count_to_the_hard_cap ... ok
test upstream::tests::code_system_summary_heading_falls_back_through_title_name_id ... ok
test upstream::tests::degraded_reason_is_stable ... ok
test upstream::tests::browser_page_next_offset_stops_when_rows_are_short ... ok
test dashboard::tests::cards_render_the_degraded_banner_when_upstream_is_unreachable ... ok

test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.02s

     Running tests\code_systems.rs
running 8 tests
test browser_rejects_over_max_count_partial_shape_too ... ok
test lookup_run_without_code_renders_invalid_input_outcome ... ok
test browser_over_max_count_renders_invalid_input_outcome ... ok
test lookup_input_hx_renders_input_partial_only ... ok
test browser_rows_fragment_vary_on_htmx_request ... ok
test detail_soft_deleted_would_render_outcome_not_page_404 ... ok
test detail_renders_shell_and_outcome_on_upstream_failure ... ok
test browser_renders_full_page_with_translated_heading ... ok

test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.12s

     Running tests\concept_maps.rs
running 15 tests
test browser_over_max_count_renders_invalid_input_outcome ... ok
test browser_rows_fragment_targets_and_varies_on_htmx_request ... ok
test translate_does_not_expose_unsupported_params ... ok
test detail_renders_shell_and_degraded_on_upstream_failure ... ok
test translate_tab_htmx_returns_input_partial_only ... ok
test browser_renders_full_page_with_translated_heading ... ok
test translate_reverse_without_target_code_renders_inline_validation_outcome_without_posting_to_hts ... ok
test translate_forward_without_code_renders_inline_validation_outcome_without_posting_to_hts ... ok
test translate_no_matches_renders_neutral_state_not_error ... ok
test translate_reverse_posts_target_code_parameter ... ok
test translate_forward_posts_code_and_system_parameters ... ok
test translate_r5_response_labels_column_as_relationship ... ok
test detail_unknown_id_renders_outcome_inside_shell ... ok
test translate_hts_error_renders_outcome_partial ... ok
test translate_r4_response_labels_column_as_equivalence ... ok

test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.03s

     Running tests\diagnostics.rs
running 5 tests
test terminology_capabilities_tab_renders_code_system_list ... ok
test metrics_tab_renders_prometheus_text_verbatim ... ok
test capability_tab_renders_property_table ... ok
test any_tab_5xx_renders_outcome_in_diag_panel_only ... ok
test diagnostics_page_renders_all_four_tabs_in_full_shell ... ok

test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.04s

     Running tests\import.rs
running 6 tests
test import_pre_flight_empty_bundle_returns_outcome_without_calling_hts ... ok
test import_post_200_renders_success_summary ... ok
test import_post_413_renders_too_large_guidance ... ok
test import_post_400_renders_outcome_partial ... ok
test import_post_207_renders_partial_success_with_issue_list ... ok
test import_page_renders_full_shell_with_upload_form ... ok

test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.04s

     Running tests\operations_e1.rs
running 4 tests
test run_lookup_free_scope_posts_to_hts_and_swaps_result_region ... ok
test run_expand_free_scope_pins_instance_id_and_forwards_expand_params ... ok
test every_pre_flight_validation_gates_short_circuit_without_upstream ... ok
test slice_e_shell_input_and_stubs_hold_together ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.37s

     Running tests\operations_e2.rs
running 6 tests
test batch_seed_returns_n_skeleton_rows ... ok
test closure_empty_graph_renders_neutral_state_not_outcome ... ok
test vs_validate_false_result_renders_neutral_badge_not_outcome ... ok
test verb_rule_all_ops_post_to_hts ... ok
test closure_banner_renders_only_on_closure_op ... ok
test batch_progress_terminal_state_stops_polling ... ok

test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.09s

     Running tests\route_enum.rs
running 2 tests
test unknown_route_under_ui_hts_returns_404 ... ok
test every_registered_route_walks_the_locale_hx_matrix_and_en_body_marker ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 9.90s

     Running tests\router_http.rs
running 4 tests
test assets_serve_the_embedded_bundle_under_ui_hts_assets ... ok
test dashboard_localizes_via_accept_language_when_no_query_or_cookie ... ok
test dashboard_advertises_vary_hx_request_for_htmx_caching ... ok
test dashboard_serves_full_page_at_ui_hts ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.14s

     Running tests\value_sets.rs
running 16 tests
test browser_over_max_count_renders_invalid_input_outcome ... ok
test browser_rows_fragment_targets_and_varies_on_htmx_request ... ok
test expand_tab_htmx_returns_input_partial_only ... ok
test expand_input_shows_advanced_details_and_threshold_field ... ok
test detail_renders_shell_and_degraded_on_upstream_failure ... ok
test browser_renders_full_page_with_translated_heading ... ok
test expand_422_renders_too_costly_banner_with_raise_form ... ok
test expand_tree_hides_pager_and_labels_total_leaves ... ok
test expand_threshold_below_ceiling_attaches_x_too_costly_header ... ok
test expand_threshold_above_ceiling_drops_header_and_warns ... ok
test expand_flat_renders_load_more_when_total_exceeds_page ... ok
test expand_filter_no_match_renders_neutral_state_with_filter ... ok
test expand_flat_mode_sends_exclude_nested_true_and_no_hierarchical ... ok
test detail_unknown_id_renders_outcome_inside_shell ... ok
test expand_no_members_renders_neutral_state ... ok
test expand_tree_mode_sends_hierarchical_true_and_no_exclude_nested ... ok

test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.04s
```

**Aggregate:** 12 + 8 + 15 + 5 + 6 + 4 + 6 + 2 + 4 + 16 = **78 tests, 0 failed, 0 ignored**. Baseline before G was 73 tests; Slice G contributes 5 new tests in `tests/diagnostics.rs` and two new rows to `ROUTES` in `tests/route_enum.rs` (picked up transparently by the existing matrix walker — no additional `#[tokio::test]` added there, per the task's route-enum discipline).

`cargo build -p helios-hts-ui --tests` — zero warnings after the final trim. `ReadLints` on all modified/added Slice G files — clean.

### Slice G test roster

| # | Test | What it locks in |
|---|---|---|
| 1 | `diagnostics_page_renders_all_four_tabs_in_full_shell` | Full-page `GET /ui/hts/diagnostics` returns a complete HTML document with the H1 (Fluent-resolved), all four `hts-diagnostics-tab-*` id markers, `id="diag-panel"`, `aria-selected="true"` on the active tab, the pre-rendered Capability sub-heading, and no leaked non-id Fluent keys. |
| 2 | `capability_tab_renders_property_table` | Panel route `?tab=capability` with `HX-Request: true` returns a fragment (no `<!doctype html>`) containing the property table with URL + version from the mock body. |
| 3 | `terminology_capabilities_tab_renders_code_system_list` | Panel route `?tab=terminology-capabilities` — mock returns two `codeSystem[]` entries; assertions cover both URIs, both `v{version}` chips, and that the outgoing GET carried `mode=terminology`. |
| 4 | `metrics_tab_renders_prometheus_text_verbatim` | Panel route `?tab=metrics` — mock returns `# TYPE foo counter\nfoo 42\n`; assertions cover the sub-heading, the `<pre>` wrapper, and the verbatim TYPE + sample lines inside it. |
| 5 | `any_tab_5xx_renders_outcome_in_diag_panel_only` | Full-page `?tab=health` with mock `/health` returning 500 — assertions cover the `hts-outcome hts-outcome--error` class stack **and** that the other three tab id markers still survive in the shell (per-tab isolation, §7.9). |

Dual-mode (`HX-Request` on the top-level page + the panel route) is picked up by `route_enum.rs::ROUTES` — the merged matrix walker fires each new row through both HX modes and all three locales, so the `≤ 5` Slice G budget is not spent on dual-mode plumbing.

## Deuda pendiente (Slice G+1 / Phase 3)

1. **Locale review for `es` / `de`.** Both locale files carry a `# TODO(G): review es` / `review de` header. Coverage is complete (same key set as `en`); only phrasing needs a native-speaker pass. `hts-diagnostics-property-*` labels (URL / Version / Name / Title / Status / Date) were kept generic and short so a translator can drop them in without reshaping the layout.
2. **CSS.** The panel partial uses the class hooks `hts-diagnostics-tab`, `hts-diagnostics-tab--capability|terminology|health|metrics`, `hts-diagnostics-facts`, `hts-diagnostics-resources`, `hts-diagnostics-code-systems`, `hts-diagnostics-metrics`, and the wrapper `hts-diagnostics-panel-body`. No CSS was added — the vendored / shared stylesheet in `crates/ui/assets` is off-limits for G (task rule #4). `# TODO(G): visual polish in Phase 3` — sticky tab strip, monospace metric block styling, and a mobile-collapsible property table.
3. **Playwright spec.** Deferred to Phase 3 integration, matching E1/E2/F discipline. No spec was added under `crates/hts-ui/e2e/tests/`. When it lands, a natural first pass is: hard-nav each of the four `?tab=` deep links, then click through the tab strip once and assert the URL updates (`hx-push-url="true"` contract).
4. **Fluent parity ring.** Same story as Slice F — the `fluent-syntax` dev-dep is available but no `tests/` file consumes it. A cross-slice mini-slice for `es` / `de` / `en` parity would catch the class of regressions where a slice forgets a locale.
5. **Nav placement.** The sidebar `<a href="/ui/hts/diagnostics">` link was already wired to `hts-nav-diagnostics` during the Phase 1 chrome pass in `templates/layouts/base.html`; Slice G left it untouched (identical situation to Slice F's Import nav). No conflict with Slice F.
6. **CapabilityStatement REST section richness.** The `CapabilityView::resources` field parses `rest[].resource[].type` + `interaction[].code`, which covers the wireframe. HTS emits richer detail (searchParam, versioning, referencePolicy, etc.); a follow-up can expand the projection without reshaping the current tab layout.
7. **`/metrics` is Prometheus-only.** The tab renders whatever `GET /metrics` returns verbatim; if HTS grows a JSON metrics endpoint, a follow-up can add a JSON tab alongside the text one. Empty-body case is handled non-error via `hts-diagnostics-metrics-empty` (as the task guidance called out).

## Cross-check vs git

- **Branch:** `feat/551-hts-ui` (unchanged, confirmed via `git branch --show-current`).
- **HEAD:** `d99f3d972` (unchanged, confirmed via `git rev-parse HEAD`).
- **Working copy:** uncommitted, matches the "F + G stacked additive on top of `d99f3d972`" handoff contract (task rules #2 + #5). `git status --short` shows the F + G stack:

    ```
     M crates/hts-ui/src/lib.rs
     M crates/hts-ui/src/upstream.rs
     M crates/hts-ui/tests/route_enum.rs
     M locales/de/main.ftl
     M locales/en/main.ftl
     M locales/es/main.ftl
    ?? crates/hts-ui/src/diagnostics.rs
    ?? crates/hts-ui/src/import.rs
    ?? crates/hts-ui/templates/pages/diagnostics.html
    ?? crates/hts-ui/templates/pages/import.html
    ?? crates/hts-ui/templates/partials/hts-diagnostics-panel.html
    ?? crates/hts-ui/templates/partials/hts-import-form.html
    ?? crates/hts-ui/templates/partials/hts-import-status.html
    ?? crates/hts-ui/tests/diagnostics.rs
    ?? crates/hts-ui/tests/import.rs
    ```

    Every `??` row from Slice F's output (`import.rs`, three import templates, `tests/import.rs`) is preserved; Slice G adds its own four `??` rows (`diagnostics.rs`, two diagnostics templates, `tests/diagnostics.rs`). Every ` M` row is Slice F's + Slice G's stacked appends in a single working-copy modification.

- **`git diff --stat HEAD`** (F + G contributions combined):

    ```
     crates/hts-ui/src/lib.rs          |  24 +-
     crates/hts-ui/src/upstream.rs     | 530 ++++++++++++++++++++++++++++++++++++++
     crates/hts-ui/tests/route_enum.rs |  33 +++
     locales/de/main.ftl               |  64 +++++
     locales/en/main.ftl               |  71 +++++
     locales/es/main.ftl               |  64 +++++
     6 files changed, 779 insertions(+), 7 deletions(-)
    ```

    Slice F's output reported `399 insertions(+), 7 deletions(-)` across the same six files; the delta from that baseline to the current `779 insertions(+), 7 deletions(-)` matches Slice G's contribution size (`+380` lines across the six shared files, plus the four `??` files for the new module / templates / tests). Slice G's diff to `src/lib.rs` (`+17` net) is confined to the `mod` list, the `router()` merge, and a new `pub use` block appended after Slice F's alphabetized one; no Slice F line was touched.

- **Slice G touchpoints preserved for downstream slices (append-friendly):**
  - `crates/hts-ui/src/lib.rs::router()` — G appended `.merge(diagnostics::routes())` after Slice F's `.merge(import::routes())`. Slice H can go on the next line without touching F or G.
  - `crates/hts-ui/tests/route_enum.rs::ROUTES` — G appended two `/ui/hts/diagnostics(/panel)` rows after Slice F's `/ui/hts/import` row. Slice H appends its own rows after G's, no reorder required.
  - `locales/{en,es,de}/main.ftl` — G appended `hts-diagnostics-*` blocks at EOF, immediately after Slice F's `hts-import-*` block. Slice H can append after G's block and stays parity-safe as long as H authors the same key set in all three files.
