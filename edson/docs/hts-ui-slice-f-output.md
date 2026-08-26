# Slice F output — Import (`/ui/hts/import`)

- **Design ref:** `edson/docs/hts-ui-design.md` §7.7 Import (+ §7.10 row 7.7 states matrix and §7 preamble inherited guards).
- **Plan ref:** issue [#551](https://github.com/HeliosSoftware/hfs/issues/551) — HTS-UI Phase 2, Slice F.
- **Status:** Green. `cargo test -p helios-hts-ui --tests` = **73 passed / 0 failed** (baseline 67 + 6 new import tests). Working copy uncommitted per task rule #2.
- **Branch:** `feat/551-hts-ui`.
- **HEAD before F:** `d99f3d972 feat(hts-ui): Phase 2 v1 — dashboard, browsers, ops workbench (#551)`.
- **Toolchain used:** `stable-x86_64-pc-windows-gnu`. The repo already carried this override (E1/E2 fall-back), so `rustup override set stable-x86_64-pc-windows-gnullvm` was not re-issued — the gnu toolchain compiles the workspace cleanly on this box and `rustup show` confirms it is the active override for `c:\Users\tercere\src\helios\hfs`.

## Files added

| Path | Purpose |
|---|---|
| `c:\Users\tercere\src\helios\hfs\crates\hts-ui\src\import.rs` | Slice F module: `import_page` + `import_run` handlers, `StatusView` for the four visual variants, pre-flight gates, `probe_degraded`, `parse_form`. |
| `c:\Users\tercere\src\helios\hfs\crates\hts-ui\templates\pages\import.html` | Full-page shell extending `layouts/base.html`; renders degraded banner (§7 preamble) + form partial + status region. |
| `c:\Users\tercere\src\helios\hfs\crates\hts-ui\templates\partials\hts-import-form.html` | Upload form (real `<form>` with `hx-post`); paste-only in v1 (file input rendered disabled — see deuda). Submit disabled when degraded. |
| `c:\Users\tercere\src\helios\hfs\crates\hts-ui\templates\partials\hts-import-status.html` | Status region discriminated on `is_success` / `is_partial` / `is_rejected` / `is_too_large`; reuses `hts-outcome.html` for the 400 arm and `hts-degraded.html` for the transport-failure arm. |
| `c:\Users\tercere\src\helios\hfs\crates\hts-ui\tests\import.rs` | Six `#[tokio::test]` functions covering the 200/207/400/413 matrix + pre-flight empty-bundle gate + shell shape. Uses the same `start_mock` + `/__mock_ready` pattern as `tests/value_sets.rs` / `tests/concept_maps.rs`. |

## Files modified

| Path | Change (one-line) |
|---|---|
| `c:\Users\tercere\src\helios\hfs\crates\hts-ui\src\lib.rs` | Registered `mod import;`, appended `.merge(import::routes())` after the last existing merge, re-exported `ImportCounts`, `ImportResult`, `ImportStatus`. |
| `c:\Users\tercere\src\helios\hfs\crates\hts-ui\src\upstream.rs` | Added `ImportStatus` enum, `ImportCounts`, `ImportResult`, `UpstreamClient::import_bundle`, plus `parse_import_counts` and `collect_outcome_diagnostics` helpers. |
| `c:\Users\tercere\src\helios\hfs\crates\hts-ui\tests\route_enum.rs` | Appended one `ROUTES` entry for `/ui/hts/import` (shell marker `>Import terminology<`). No new `#[tokio::test]` was added — the merged matrix walker picks it up automatically. |
| `c:\Users\tercere\src\helios\hfs\locales\en\main.ftl` | Appended `hts-import-*` block (24 keys incl. Fluent plural selector for issue count). `hts-nav-import` was already present from Phase 1 stubs. |
| `c:\Users\tercere\src\helios\hfs\locales\es\main.ftl` | Appended the same 24 keys, with initial Spanish translations plus `# TODO(F): review es`. |
| `c:\Users\tercere\src\helios\hfs\locales\de\main.ftl` | Appended the same 24 keys, with initial German translations plus `# TODO(F): review de`. |

No files under `crates/hts/*`, `crates/ui/*`, or any Slice A/B/C/D/E1/E2 handler/partial/test were touched (task rule #4). The sidebar nav slot in `templates/layouts/base.html` was already wired to `/ui/hts/import` with `hts-nav-import` during the Phase 1 chrome pass — no edit needed for F.

## Routes registered

| Verb | Path | Handler |
|---|---|---|
| GET | `/hts/import` | `import_page` — full page on hard nav; upload-form partial on `HX-Request`. Runs `probe_degraded` (upstream `/health`) so the shared degraded banner + disabled submit appear when HTS is unreachable. |
| POST | `/hts/import` | `import_run` — parses the paste-mode form, runs the empty-bundle + invalid-JSON pre-flight gates, calls `UpstreamClient::import_bundle` on success, then renders the status partial (htmx) or full page with the status embedded (hard nav). |

Both routes are appended after the last existing `.merge(...)` in `router()` so Slice G's own `.merge(diagnostics::routes())` slots in cleanly after Slice F (task rule #5, append-friendly).

## `UpstreamClient::import_bundle`

```rust
pub async fn import_bundle(
    &self,
    bundle_json: &str,
) -> Result<ImportResult, UpstreamError>;
```

- **Wire:** `POST {base_url}/import` with headers `Accept: application/fhir+json` + `Content-Type: application/fhir+json` and the raw JSON string as body. No streaming, no multipart — matches the paste-only v1 form.
- **Timeouts:** whatever the caller's `UpstreamClient` was built with — production uses `new` (5 s / 2 s), tests use `new_with_timeouts` (2 s / 5 s for mock, 100 ms / 250 ms for closed-loopback).
- **Result variants:** absorbs 200/207/400/413 into `Ok(ImportResult)` so the status partial owns rendering, and forwards 5xx + transport failures through `Err(UpstreamError::*)` so the shared degraded banner picks them up.

### `ImportResult` variant taxonomy

| Variant (`ImportResult::status`) | Trigger | `counts` | `issues` | `outcome` |
|---|---|---|---|---|
| `Success` | HTTP 200, body is HTS's `ImportResponse` JSON with `errors[]` empty or absent | `Some(ImportCounts)` from `code_systems` / `value_sets` / `concept_maps` / `concepts` | `[]` | `None` |
| `PartialSuccess` | HTTP 207 or HTTP 200 with non-empty `errors[]` (HTS's actual behaviour is 207 whenever `has_errors`, but the code also handles a stale 200 arm defensively) | `Some(ImportCounts)` | The `errors[]` strings from HTS's response body | `None` |
| `Rejected` | HTTP 400 (body is `OperationOutcome` per HTS's error mapping), or HTTP 200/207 body that fails `serde_json::from_str` | `None` | Diagnostics collected from `OperationOutcome.issue[].diagnostics` (falls back to `.code` when diagnostics is empty) | `Some(OutcomeView)` when the body was an `OperationOutcome`; `None` for the JSON-decode fallback |
| `TooLarge` | HTTP 413 (body is typically empty) | `None` | `[]` | `None` — status partial renders the split-Bundle hint from `hts-import-too-large-hint` |

Transport-layer failures (`UpstreamError::Connect` / `UpstreamError::Timeout` / `UpstreamError::ClientBuild`) reach the status partial through `StatusView::from_error`, which flips `degraded_reason` on and the partial renders `hts-degraded.html` in place of the variant banners.

### `ImportCounts` shape

```rust
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportCounts {
    pub code_systems: u32,
    pub value_sets: u32,
    pub concept_maps: u32,
    pub concepts: u32,
}
```

Field names mirror the JSON keys HTS emits (`crates/hts/src/operations/import_bundle.rs::ImportResponse`). Absent counts on 400/413 render as `&mdash;` in the status partial — no fabricated zeros.

## Fluent keys added

24 keys under the `hts-import-*` namespace, appended verbatim at the end of each locale so Slice G can append its `hts-diagnostics-*` block after them without a merge conflict.

- **en (source):** all 24 keys authored as user-facing English strings. `hts-import-issues-heading` uses a Fluent plural selector (`[one] { $n } issue *[other] { $n } issues`); `hts-import-duration` carries a `{ $seconds }` placeable.
- **es:** 24 keys, initial Spanish translations, header comment `# TODO(F): review es`. All keys are present — the `# TODO` is a copy-quality flag, not a coverage gap.
- **de:** 24 keys, initial German translations, header comment `# TODO(F): review de`. Same coverage story as `es`.

**Parity note:** the three locales share the same key set (verified by `diff`-friendly reading of the appended blocks). The workspace does not currently ship an automated parity test — the design comment in `crates/hts-ui/Cargo.toml` mentions `fluent-syntax` for that purpose, but no `tests/` file consumes it as of `d99f3d972`. Slice F did not add such a test either; it stays a Phase 3 mini-slice per the E1/E2 discipline.

The `hts-nav-import` key that Slice A / B added during the Phase 1 chrome pass is untouched; the sidebar nav item in `templates/layouts/base.html` was already wired to `/ui/hts/import` and simply lights up now that a real page answers there.

## Test results — full `cargo test -p helios-hts-ui --tests` output

```
running 12 tests
test i18n::tests::accept_language_is_matched_by_rfc4647_lookup ... ok
test i18n::tests::query_override_beats_everything_and_is_explicit ... ok
test upstream::tests::base_url_strips_trailing_slashes ... ok
test upstream::tests::code_system_summary_heading_falls_back_through_title_name_id ... ok
test i18n::tests::hts_lang_cookie_beats_accept_language ... ok
test upstream::tests::browser_page_next_offset_stops_when_rows_are_short ... ok
test upstream::tests::outcome_view_parses_hts_operation_outcome_shape ... ok
test upstream::tests::uptime_pretty_shapes_units_from_seconds ... ok
test upstream::tests::degraded_reason_is_stable ... ok
test upstream::tests::browser_filters_clamp_count_to_the_hard_cap ... ok
test i18n::tests::default_locale_is_english ... ok
test dashboard::tests::cards_render_the_degraded_banner_when_upstream_is_unreachable ... ok

test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.02s

     Running tests\code_systems.rs
running 8 tests
test lookup_run_without_code_renders_invalid_input_outcome ... ok
test browser_rejects_over_max_count_partial_shape_too ... ok
test browser_over_max_count_renders_invalid_input_outcome ... ok
test browser_rows_fragment_vary_on_htmx_request ... ok
test detail_soft_deleted_would_render_outcome_not_page_404 ... ok
test lookup_input_hx_renders_input_partial_only ... ok
test browser_renders_full_page_with_translated_heading ... ok
test detail_renders_shell_and_outcome_on_upstream_failure ... ok

test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.13s

     Running tests\concept_maps.rs
running 15 tests
test browser_over_max_count_renders_invalid_input_outcome ... ok
test browser_rows_fragment_targets_and_varies_on_htmx_request ... ok
test translate_tab_htmx_returns_input_partial_only ... ok
test browser_renders_full_page_with_translated_heading ... ok
test translate_does_not_expose_unsupported_params ... ok
test detail_renders_shell_and_degraded_on_upstream_failure ... ok
test translate_forward_without_code_renders_inline_validation_outcome_without_posting_to_hts ... ok
test translate_reverse_without_target_code_renders_inline_validation_outcome_without_posting_to_hts ... ok
test translate_no_matches_renders_neutral_state_not_error ... ok
test detail_unknown_id_renders_outcome_inside_shell ... ok
test translate_hts_error_renders_outcome_partial ... ok
test translate_r5_response_labels_column_as_relationship ... ok
test translate_r4_response_labels_column_as_equivalence ... ok
test translate_forward_posts_code_and_system_parameters ... ok
test translate_reverse_posts_target_code_parameter ... ok

test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.05s

     Running tests\import.rs
running 6 tests
test import_pre_flight_empty_bundle_returns_outcome_without_calling_hts ... ok
test import_post_413_renders_too_large_guidance ... ok
test import_post_200_renders_success_summary ... ok
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
test vs_validate_false_result_renders_neutral_badge_not_outcome ... ok
test closure_empty_graph_renders_neutral_state_not_outcome ... ok
test verb_rule_all_ops_post_to_hts ... ok
test closure_banner_renders_only_on_closure_op ... ok
test batch_progress_terminal_state_stops_polling ... ok

test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.08s

     Running tests\route_enum.rs
running 2 tests
test unknown_route_under_ui_hts_returns_404 ... ok
test every_registered_route_walks_the_locale_hx_matrix_and_en_body_marker ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 7.87s

     Running tests\router_http.rs
running 4 tests
test assets_serve_the_embedded_bundle_under_ui_hts_assets ... ok
test dashboard_localizes_via_accept_language_when_no_query_or_cookie ... ok
test dashboard_advertises_vary_hx_request_for_htmx_caching ... ok
test dashboard_serves_full_page_at_ui_hts ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.13s

     Running tests\value_sets.rs
running 16 tests
test browser_over_max_count_renders_invalid_input_outcome ... ok
test browser_rows_fragment_targets_and_varies_on_htmx_request ... ok
test expand_tab_htmx_returns_input_partial_only ... ok
test detail_renders_shell_and_degraded_on_upstream_failure ... ok
test expand_input_shows_advanced_details_and_threshold_field ... ok
test browser_renders_full_page_with_translated_heading ... ok
test expand_flat_mode_sends_exclude_nested_true_and_no_hierarchical ... ok
test expand_tree_mode_sends_hierarchical_true_and_no_exclude_nested ... ok
test detail_unknown_id_renders_outcome_inside_shell ... ok
test expand_tree_hides_pager_and_labels_total_leaves ... ok
test expand_threshold_above_ceiling_drops_header_and_warns ... ok
test expand_422_renders_too_costly_banner_with_raise_form ... ok
test expand_flat_renders_load_more_when_total_exceeds_page ... ok
test expand_threshold_below_ceiling_attaches_x_too_costly_header ... ok
test expand_no_members_renders_neutral_state ... ok
test expand_filter_no_match_renders_neutral_state_with_filter ... ok

test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.02s
```

**Aggregate:** 12 + 8 + 15 + 6 + 4 + 6 + 2 + 4 + 16 = **73 tests, 0 failed, 0 ignored**. Baseline before F was 67 tests; Slice F contributes 6 new tests in `tests/import.rs` and one new row to `ROUTES` in `tests/route_enum.rs` (picked up transparently by the existing matrix walker — no additional `#[tokio::test]` added there, per the task's route-enum discipline).

No warnings from `cargo build --tests -p helios-hts-ui` after the final trim.

## Deuda pendiente (Slice F+1 / Phase 3)

1. **File upload (`bundle_file`).** The design lists paste and file sources for §7.7. Slice F ships the paste path only; the `<input type="file">` renders (a11y symmetry with the radio group) but is `disabled`, and the server-side `parse_form` only inspects `bundle`. Adding the file path requires wiring `axum::extract::Multipart` (or a `Bytes` reader with a content-type sniff) plus a new pre-flight gate. See `# TODO(F): file input in follow-up` in `src/import.rs`. This is called out in the design doc §7.7 wireframe but not in the states matrix (§7.10 row 7.7), which only mandates the 4 status arms — which F covers.
2. **Async status polling.** Design doc §7.7 flags an `hx-trigger="load, every 2s"` fragment for whenever HTS grows an async `/import` status route. That route does not exist today — HTS's `POST /import` is synchronous — so the current inline status render matches the spec's "until then" arm. When HTS grows the async route, this slice's status partial can be extended with a job-id `hx-get`; the shape does not require re-shaping `ImportResult`.
3. **Locale review for `es` / `de`.** Both locale files carry an explicit `# TODO(F): review es` / `review de` header on the appended block. Coverage is complete (same key set as `en`); only the phrasing needs a native-speaker pass. The Fluent plural selector for `hts-import-issues-heading` uses the correct `[one] / *[other]` categories per CLDR for both languages, but a translator may want to review the exact wording.
4. **CSS.** The status partial uses the class hooks `hts-import-status--ok` / `--warn` / `--error` in keeping with the Slice A/B/C pattern (`hts-outcome--error` and friends). No CSS was added because the vendored / shared stylesheet in `crates/ui/assets` is off-limits for F (task rule #4); the classes render without style overrides today, matching how E2's `hts-op-banner` shipped ahead of a dedicated stylesheet.
5. **Playwright spec.** Deferred to Phase 3 integration, matching E1/E2 discipline. No spec was added under `crates/hts-ui/e2e/tests/`.

## Cross-check vs git

- **Branch:** `feat/551-hts-ui` (unchanged, confirmed via `git branch --show-current`).
- **HEAD:** `d99f3d972` (unchanged, confirmed via `git rev-parse HEAD`).
- **Working copy:** uncommitted, matches the "additive on top of `d99f3d972`" handoff contract (task rule #2). `git status --short` shows:

    ```
     M crates/hts-ui/src/lib.rs
     M crates/hts-ui/src/upstream.rs
     M crates/hts-ui/tests/route_enum.rs
     M locales/de/main.ftl
     M locales/en/main.ftl
     M locales/es/main.ftl
    ?? crates/hts-ui/src/import.rs
    ?? crates/hts-ui/templates/pages/import.html
    ?? crates/hts-ui/templates/partials/hts-import-form.html
    ?? crates/hts-ui/templates/partials/hts-import-status.html
    ?? crates/hts-ui/tests/import.rs
    ```

    (Plus the unchanged pre-F Slice-B/C/D/E1/E2 files that were already untracked or modified in the parent handoff — none touched by F.)

- **`git diff --stat HEAD`** (F contributions only):

    ```
     crates/hts-ui/src/lib.rs          |  16 ++-
     crates/hts-ui/src/upstream.rs     | 274 ++++++++++++++++++++++++++++++++++++++
     crates/hts-ui/tests/route_enum.rs |  11 ++
     locales/de/main.ftl               |  34 +++++
     locales/en/main.ftl               |  37 +++++
     locales/es/main.ftl               |  34 +++++
     6 files changed, 399 insertions(+), 7 deletions(-)
    ```

    Plus the five untracked files (`import.rs`, three templates, one test file) that account for the rest of the slice.

- **Slice G touchpoints (append-friendly):**
  - `crates/hts-ui/src/lib.rs::router()` — F appended `.merge(import::routes())` after `operations::routes()`. G's `.merge(diagnostics::routes())` can go on the next line without touching F.
  - `crates/hts-ui/tests/route_enum.rs::ROUTES` — F appended the `/ui/hts/import` row after the last existing operations input row. G appends its own row(s) after F's, no reorder required.
  - `locales/{en,es,de}/main.ftl` — F appended `hts-import-*` blocks at EOF. G's `hts-diagnostics-*` block can append immediately after F's, and stays parity-safe as long as G authors the same key set in all three files.
