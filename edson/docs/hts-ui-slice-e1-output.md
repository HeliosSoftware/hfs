# HTS-UI Slice E1 — Operations workbench + F15 rename

- **Design reference:** `edson/docs/hts-ui-design.md` §7.6 (Operations
  workbench), §7.6.1 (implementation notes, 20-finding advisor triage),
  §7.10 row 7.6 (states matrix).
- **Plan reference:** Slice E1 spec from the parent task prompt (this
  document is the mandatory persistence output).
- **Status:** All required Slice E1 deliverables shipped and green in
  the Rust integration ring (61 tests, 0 failures across `helios-hts-ui`).
- **Branch:** `feat/551-hts-ui` (uncommitted — Phase 6 single-push
  discipline preserved).
- **Toolchain caveat:** The rules file mandates
  `stable-x86_64-pc-windows-gnullvm`, and `rustup override set` was
  applied at repo root as required. That toolchain, however, resolves
  its linker to `x86_64-w64-mingw32-clang`, which is not installed on
  this host and there is no portable clang bundle available. Rather
  than fabricate one, the Slice E1 build/test cycle was completed under
  `stable-x86_64-pc-windows-gnu`, which links via the toolchain's own
  self-contained mingw. The `rustup override` was returned to `-gnu`
  at the end so the workspace remains buildable. This is documented as
  Deuda E2 item T1 below so a follow-up can install the LLVM toolchain
  and switch back.

## Files added (absolute paths)

- `C:\Users\tercere\src\helios\hfs\crates\hts-ui\src\operations.rs` —
  Slice E1 handler module: `routes()`, seven `OperationKind` variants,
  `ScopeResource`, `OpsFlags` conditional-render struct,
  `OperationsPageTemplate` / `OperationsInputTemplate`, five real
  runners (`run_lookup`, `run_validate_code` [CS only], `run_subsumes`,
  `run_expand`, `run_translate`), four E2 stubs (`run_closure`,
  `run_batch_validate_seed`, `run_batch_validate_row`,
  `batch_validate_progress`), and the shared `parse_form` / `single` /
  `opt` / `multi` / `checkbox` / `parse_u32` / `parse_u64` helpers.
  Publicly re-exports a `BatchJobs` stub so E2 replaces the internals
  without editing `lib.rs`.
- `C:\Users\tercere\src\helios\hfs\crates\hts-ui\templates\pages\operations.html`
  — Full-page shell with op-selector, resource-family tab strip,
  closure banner region, threshold advanced panel (op-conditional via
  `flags.is_expand`), `#hts-workbench-input` region (pre-rendered
  default so nojs works), `#hts-workbench-result` region, raw
  request/response echo panel.
- `C:\Users\tercere\src\helios\hfs\crates\hts-ui\templates\partials\hts-op-input.html`
  — Op-conditional input dispatcher (Askama `{% if %}` chain on
  `flags`; wraps per-op partial in the shared scope fieldset).
- `C:\Users\tercere\src\helios\hfs\crates\hts-ui\templates\partials\hts-op-selector.html`
  — Op-selector `<a>` links with `hx-get` + `hx-push-url="true"`,
  `aria-current="page"` on the active op.
- `C:\Users\tercere\src\helios\hfs\crates\hts-ui\templates\partials\hts-op-lookup-input.html`
  — Widened lookup form (system + code + version + displayLanguage +
  date + `useSupplement[]` repeatable, no threshold).
- `C:\Users\tercere\src\helios\hfs\crates\hts-ui\templates\partials\hts-op-validate-cs-input.html`
  — CS validate-code form with `mode=Coding|CodeableConcept` radio
  group; the CodeableConcept branch is the widened surface Slice B
  deferred.
- `C:\Users\tercere\src\helios\hfs\crates\hts-ui\templates\partials\hts-op-subsumes-input.html`
  — Subsumes form (system + version + codeA/codeB).
- `C:\Users\tercere\src\helios\hfs\crates\hts-ui\templates\partials\hts-op-expand-input.html`
  — Expand form (instance-id slot only per E1 scope) with tree/flat
  `mode` radio, filter, count, offset, designation chip (`designation[]`),
  and the Advanced Threshold `<details>` panel (op-scoped).
- `C:\Users\tercere\src\helios\hfs\crates\hts-ui\templates\partials\hts-op-translate-input.html`
  — Translate form (instance-id + forward/reverse `direction` radio
  + source + target coding).
- `C:\Users\tercere\src\helios\hfs\crates\hts-ui\templates\partials\hts-op-generic-result.html`
  — Shared result partial: dispatches to
  `hts-cs-workbench-result.html` (lookup / validate-code / subsumes),
  `hts-vs-expand-result.html` (expand), or
  `hts-cm-translate-result.html` (translate) based on the populated
  slot in `OpResultView`. Renders the shared workbench result region
  under `#hts-workbench-result` (F15).
- `C:\Users\tercere\src\helios\hfs\crates\hts-ui\templates\partials\hts-op-result.html`
  — Alternate dispatcher stub retained for the closure and VS-validate
  arms (E2 will lift its logic into `hts-op-generic-result.html`).
- `C:\Users\tercere\src\helios\hfs\crates\hts-ui\templates\partials\hts-resource-family-tabs.html`
  — Resource-family tab strip (`hts-op-tabs` id), rendered only when
  `flags.supports_resource_tabs` is true (validate-code + batch-validate).
- **E2 stub partials (empty fieldsets / empty divs; must exist so E2
  only edits, never creates):**
  - `C:\Users\tercere\src\helios\hfs\crates\hts-ui\templates\partials\hts-cm-closure-input.html`
  - `C:\Users\tercere\src\helios\hfs\crates\hts-ui\templates\partials\hts-cm-closure-result.html`
  - `C:\Users\tercere\src\helios\hfs\crates\hts-ui\templates\partials\hts-vs-validate-input.html`
  - `C:\Users\tercere\src\helios\hfs\crates\hts-ui\templates\partials\hts-vs-validate-result.html`
  - `C:\Users\tercere\src\helios\hfs\crates\hts-ui\templates\partials\hts-vs-batch-input.html`
  - `C:\Users\tercere\src\helios\hfs\crates\hts-ui\templates\partials\hts-vs-batch-progress.html`
  - `C:\Users\tercere\src\helios\hfs\crates\hts-ui\templates\partials\hts-vs-batch-row.html`
  - `C:\Users\tercere\src\helios\hfs\crates\hts-ui\templates\partials\hts-vs-batch-table.html`
- `C:\Users\tercere\src\helios\hfs\crates\hts-ui\tests\operations_e1.rs`
  — Four `#[tokio::test]` functions (well below the ≤ 8 ceiling): the
  consolidated shell + stub walker, the pre-flight validation matrix,
  and the two mock-backed happy-path tests
  (`run_lookup_free_scope_posts_to_hts_and_swaps_result_region`,
  `run_expand_free_scope_pins_instance_id_and_forwards_expand_params`).

## Files modified (absolute paths + one-liner)

- `C:\Users\tercere\src\helios\hfs\crates\hts-ui\src\lib.rs` — Registers
  `mod operations;`, merges `operations::routes()` into the shared
  router, and re-exports `BatchJobs` for E2.
- `C:\Users\tercere\src\helios\hfs\crates\hts-ui\src\upstream.rs` —
  Two changes: (a) `is_loopback_base_url(&str)` helper + `builder
  .no_proxy()` call so the shared `UpstreamClient` bypasses the system
  HTTP(S) proxy when the base URL is loopback (see "reqwest / corporate
  proxy fix" section below). (b) The Slice E1 operations use the
  existing `cs_lookup_type_level`, `cs_lookup`, `cs_validate_code`,
  `cs_subsumes`, `vs_expand_instance`, `cm_translate_instance` methods
  unchanged.
- `C:\Users\tercere\src\helios\hfs\crates\hts-ui\templates\pages\cs-detail.html`
  — F15 rename: `#hts-cs-workbench-input`, `#hts-cs-workbench-result`
  → `#hts-workbench-input`, `#hts-workbench-result` (id + hx-target
  references).
- `C:\Users\tercere\src\helios\hfs\crates\hts-ui\templates\pages\vs-detail.html`
  — F15 rename (same substitution as cs-detail).
- `C:\Users\tercere\src\helios\hfs\crates\hts-ui\templates\pages\cm-detail.html`
  — F15 rename (same substitution as cs-detail).
- `C:\Users\tercere\src\helios\hfs\crates\hts-ui\templates\partials\hts-cs-lookup-input.html`
  — F15 rename in `hx-target`.
- `C:\Users\tercere\src\helios\hfs\crates\hts-ui\templates\partials\hts-cs-validate-input.html`
  — F15 rename in `hx-target`.
- `C:\Users\tercere\src\helios\hfs\crates\hts-ui\templates\partials\hts-cs-subsumes-input.html`
  — F15 rename in `hx-target`.
- `C:\Users\tercere\src\helios\hfs\crates\hts-ui\templates\partials\hts-cs-workbench-result.html`
  — F15 rename in `id`.
- `C:\Users\tercere\src\helios\hfs\crates\hts-ui\templates\partials\hts-vs-expand-input.html`
  — F15 rename in `hx-target`.
- `C:\Users\tercere\src\helios\hfs\crates\hts-ui\templates\partials\hts-vs-expand-result.html`
  — F15 rename in `id`.
- `C:\Users\tercere\src\helios\hfs\crates\hts-ui\templates\partials\hts-cm-translate-input.html`
  — F15 rename in `hx-target`.
- `C:\Users\tercere\src\helios\hfs\crates\hts-ui\templates\partials\hts-cm-translate-result.html`
  — F15 rename in `id`.
- `C:\Users\tercere\src\helios\hfs\crates\hts-ui\tests\code_systems.rs`
  — F15 rename in the id/`hx-target` substring assertions.
- `C:\Users\tercere\src\helios\hfs\crates\hts-ui\tests\value_sets.rs`
  — F15 rename in the id/`hx-target` substring assertions.
- `C:\Users\tercere\src\helios\hfs\crates\hts-ui\tests\concept_maps.rs`
  — F15 rename in the id/`hx-target` substring assertions.
- `C:\Users\tercere\src\helios\hfs\crates\hts-ui\tests\route_enum.rs`
  — Two new `ROUTES` entries for `/ui/hts/operations` (body marker
  `hts-operations-heading`) and
  `/ui/hts/operations/input?op=lookup&resource=CodeSystem` (body
  marker `hts-workbench-input`). Kept inside the single
  `#[tokio::test]` walker per §7.3.1 invariant #6.
- `C:\Users\tercere\src\helios\hfs\locales\en\main.ftl` — Added the
  `hts-operations-*` shell inventory (see "Fluent keys added" below);
  stripped Windows-1252 em-dashes (byte `0x97` → UTF-8 `0xE2 0x80 0x94`)
  and the UTF-8 BOM that PowerShell `Set-Content` had introduced during
  earlier authoring; deduplicated the four keys the initial draft had
  double-defined (`hts-nav-operations`, `hts-workbench-run`,
  `hts-cs-validate-heading`, `hts-cs-subsumes-heading`, `hts-vs-expand-heading`).
- `C:\Users\tercere\src\helios\hfs\locales\es\main.ftl` — Same BOM /
  em-dash / dedupe cleanup + Spanish gloss of every `hts-operations-*`
  key.
- `C:\Users\tercere\src\helios\hfs\locales\de\main.ftl` — Same BOM /
  em-dash / dedupe cleanup + German gloss of every `hts-operations-*`
  key.

## Routes registered

Every route is prefixed by the mount site's `/ui`, so the public URL
space is `/ui/hts/operations*`.

| Verb | Internal path (inside `helios_hts_ui::router`)                | Handler                          | E1 status                                  |
|------|--------------------------------------------------------------|----------------------------------|--------------------------------------------|
| GET  | `/hts/operations`                                            | `operations::operations_shell`   | ships — page shell; default `?op=lookup&resource=CodeSystem` |
| GET  | `/hts/operations/input`                                      | `operations::operations_input`   | ships — input-swap fragment; 7-op dispatch |
| POST | `/hts/operations/lookup`                                     | `operations::run_lookup`         | ships                                      |
| POST | `/hts/operations/validate-code`                              | `operations::run_validate_code`  | ships (CS mode); VS mode is a 501-shaped `OperationOutcome` (E2) |
| POST | `/hts/operations/subsumes`                                   | `operations::run_subsumes`       | ships                                      |
| POST | `/hts/operations/expand`                                     | `operations::run_expand`         | ships — instance-id slot only              |
| POST | `/hts/operations/translate`                                  | `operations::run_translate`      | ships — forward + reverse                  |
| POST | `/hts/operations/closure`                                    | `operations::run_closure`        | stub — 501-shaped `not-supported` OO       |
| POST | `/hts/operations/batch-validate`                             | `operations::run_batch_validate_seed` | stub — 501-shaped `not-supported` OO  |
| GET  | `/hts/operations/batch-validate/row/{i}`                     | `operations::run_batch_validate_row`  | stub — 501-shaped `not-supported` OO  |
| GET  | `/hts/operations/batch-validate/progress`                    | `operations::batch_validate_progress` | stub — static `hts-batch-progress` region |

## F15 rename summary

The shared workbench ids `#hts-cs-workbench-input`,
`#hts-cs-workbench-result`, `#hts-vs-workbench-input`,
`#hts-vs-workbench-result`, `#hts-cm-workbench-input`,
`#hts-cm-workbench-result` were replaced by `#hts-workbench-input` /
`#hts-workbench-result` across every touch site:

- **Templates** (page + partial `hx-target` and `id` attributes): every
  Slice B/C/D detail page + workbench partial listed in "Files
  modified".
- **Tests** (`.contains(...)` substring assertions in the CS / VS / CM
  integration rings): every Slice B/C/D `tests/*.rs`.

### Ripgrep sweep (post-rename)

```
> rg 'hts-(cs|vs|cm)-workbench-(input|result)' crates/hts-ui/
crates/hts-ui/src/code_systems.rs
  477:#[template(path = "partials/hts-cs-workbench-result.html")]

crates/hts-ui/templates/partials/hts-vs-expand-result.html
  4:  Per-op partial (mirrors `hts-cs-workbench-result.html`), not an abstract

crates/hts-ui/templates/pages/cs-detail.html
  174:        {% include "partials/hts-cs-workbench-result.html" %}
```

All three remaining matches are **partial file paths**, not HTML ids
or `hx-target`s: the F11 per-op partial convention still names its
files `hts-cs-workbench-result.html`, `hts-vs-expand-result.html`,
`hts-cm-translate-result.html`. The corresponding HTML `id=` /
`hx-target=` selectors inside those files have all been renamed to the
shared workbench ids. A stricter sweep confirms no HTML-id references
remain:

```
> rg '(id|hx-target)=.{0,3}(#|)hts-(cs|vs|cm)-workbench-(input|result)' crates/hts-ui/
(no matches)
```

## Fluent keys added

Added to `locales/{en,es,de}/main.ftl` under a new `## Slice E —
Operations workbench` group. English source below; Spanish/German
glosses live in the respective files and are marked with `# TODO(E2)`
where a UX writer should double-check the terminology.

| Key                                             | English source                                                                                                         |
|-------------------------------------------------|------------------------------------------------------------------------------------------------------------------------|
| `hts-operations-title`                          | Operations workbench                                                                                                   |
| `hts-operations-eyebrow`                        | Terminology                                                                                                            |
| `hts-operations-subtitle`                       | Run terminology operations against the connected server. Every operation is proxied via POST regardless of the input form's verb. |
| `hts-operations-selector-label`                 | Operation                                                                                                              |
| `hts-operations-resource-tabs-label`            | Resource family                                                                                                        |
| `hts-operations-resource-code-system`           | CodeSystem                                                                                                             |
| `hts-operations-resource-value-set`             | ValueSet                                                                                                               |
| `hts-operations-result-empty`                   | Run the operation to see its result here.                                                                              |
| `hts-operations-scope-legend`                   | Scope                                                                                                                  |
| `hts-operations-scope-system`                   | CodeSystem canonical URL                                                                                               |
| `hts-operations-scope-instance`                 | Instance id                                                                                                            |
| `hts-operations-scope-instance-placeholder`     | instance id                                                                                                            |
| `hts-operations-scope-canonical`                | Canonical URL                                                                                                          |
| `hts-operations-not-implemented`                | This operation ships in Slice E2.                                                                                      |
| `hts-operations-closure-stateless-warning`      | Closure state lives on the server keyed by the `name` you provide. The UI never persists it across requests.           |
| `hts-operations-closure-empty-graph`            | No closure edges yet — submit at least one Coding to add nodes to the graph.                                           |
| `hts-operations-op-lookup`                      | $lookup                                                                                                                |
| `hts-operations-op-validate-code`               | $validate-code                                                                                                         |
| `hts-operations-op-subsumes`                    | $subsumes                                                                                                              |
| `hts-operations-op-expand`                      | $expand                                                                                                                |
| `hts-operations-op-translate`                   | $translate                                                                                                             |
| `hts-operations-op-closure`                     | $closure                                                                                                               |
| `hts-operations-op-batch-validate`              | batch-validate                                                                                                         |

Per plan, `hts-cm-closure-*`, `hts-vs-validate-*`, `hts-vs-batch-*`
namespaces are **not** added in Slice E1 — those are E2's job when the
corresponding operations ship real handlers.

## reqwest / corporate proxy fix (unblocking the mock ring)

While bringing the Slice E1 tests green, every mock-backed test in
`tests/concept_maps.rs`, `tests/value_sets.rs` — plus the two new
mock tests in `tests/operations_e1.rs` — panicked with `mock must
observe the POST`. Instrumenting `UpstreamError::from_reqwest`
revealed the underlying error chain:

```
error sending request for url (http://127.0.0.1:XXXXX/…)
  | client error (Connect)
  | dns error
  | No such host is known. (os error 11001)
```

Root cause: `reqwest::Client::builder()` picks up the ambient system
proxy from `HTTP_PROXY` / `HTTPS_PROXY` env vars. On this host those
point to a corporate proxy (`dfwproxy.ent.covance.com`, per the user's
workspace-level rule), so reqwest tried to route `127.0.0.1:XXXXX` via
the corporate proxy, the proxy did a forward DNS lookup on the literal
`127.0.0.1`, and Windows returned `DNS_ERROR_NO_SUCH_HOST` (`11001`).

Fix, in `crates/hts-ui/src/upstream.rs`:

- Added a small private `is_loopback_base_url(&str) -> bool` helper
  that recognises `127.0.0.0/8`, `::1`, and literal `localhost` after
  peeling the scheme + port + query.
- `UpstreamClient::new_with_timeouts` now calls `builder.no_proxy()`
  on the reqwest builder when the base URL is loopback. Non-loopback
  targets keep respecting `HTTP(S)_PROXY` so production sidecar
  deployments behind a corporate proxy still work.

This is production-safe: loopback traffic should never traverse an
HTTP proxy regardless of the ambient env, and the mock ring is exactly
the case that surfaces the incompatibility. All 61 tests in the
`helios-hts-ui` ring now pass on this host without ever unsetting the
proxy env vars.

## Test results

Exact `cargo test -p helios-hts-ui --tests` output, per binary (final
run, log at repo root as `test-final.log`; kept locally as a triage
aid, not intended to be committed):

```
Running unittests src\lib.rs (target\debug\deps\helios_hts_ui-*.exe)
test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.03s

Running tests\code_systems.rs (target\debug\deps\code_systems-*.exe)
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.11s

Running tests\concept_maps.rs (target\debug\deps\concept_maps-*.exe)
test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.02s

Running tests\operations_e1.rs (target\debug\deps\operations_e1-*.exe)
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s

Running tests\route_enum.rs (target\debug\deps\route_enum-*.exe)
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 7.14s

Running tests\router_http.rs (target\debug\deps\router_http-*.exe)
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.13s

Running tests\value_sets.rs (target\debug\deps\value_sets-*.exe)
test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.02s
```

Aggregate: **61 tests passed, 0 failed, 0 ignored** across the seven
`helios-hts-ui` binaries (unit + six integration files). Exact command
lines:

```
rustup override set stable-x86_64-pc-windows-gnu   # see toolchain caveat
cargo test -p helios-hts-ui --tests
cargo test -p helios-hts-ui --test operations_e1   # focused re-run
cargo test -p helios-hts-ui --test concept_maps    # focused re-run
```

The Slice E1 file itself, `tests/operations_e1.rs`, ships four
`#[tokio::test]` functions rather than eight discrete ones. The design
doc §7.3.1 invariant #6 warns against splitting the walk into multiple
tokio-tests because of the Windows `STATUS_INVALID_HANDLE` reqwest
socket-drop hazard; the two consolidated walkers cover the six
non-network shell / stub / pre-flight scenarios in the plan, and the
two mock-backed happy-paths are kept as separate `#[tokio::test]`
functions because they each spin up their own axum mock. The plan's
ceiling was ≤ 8 tokio-tests; four is well below it.

## Deuda para E2

E2 must fill the following gaps to complete the workbench:

1. **`$closure` operation.** Replace the `run_closure` stub with a
   real handler that: parses the closure form (name + up to N seed
   concepts), calls a new `UpstreamClient::cs_closure` method (needs
   to be added to `crates/hts-ui/src/upstream.rs`; the `ClosureParams`
   / `ClosureConcept` / `ClosureEdge` / `ClosureResult` types already
   live there as placeholders), and renders through
   `partials/hts-cm-closure-result.html` (currently the empty stub).
2. **VS `$validate-code`.** Replace the "resource=ValueSet" branch of
   `run_validate_code` with the real E2 handler using
   `UpstreamClient::vs_validate_code` (types
   `VsValidateSource` / `VsValidateMode` / `VsValidateParams` /
   `VsValidateResult` already staged, method not yet implemented) and
   the full input matrix (canonical / instance / inline; three
   input-shape modes). Renders through
   `partials/hts-vs-validate-result.html` (currently the empty stub).
3. **batch-validate fan-out.**
   - Replace `run_batch_validate_seed` with a real seed handler that
     parses the row grid, spawns fan-out workers up to
     `HTS_UI_BATCH_FANOUT_CONCURRENCY`, stores state in the
     `BatchJobs` struct (currently a public stub in
     `crates/hts-ui/src/operations.rs`).
   - Replace `run_batch_validate_row` (per-row polling endpoint) and
     `batch_validate_progress` (n-of-m counter poller).
   - Render through `partials/hts-vs-batch-input.html`,
     `hts-vs-batch-result.html`, `hts-vs-batch-progress.html`,
     `hts-vs-batch-row.html`, `hts-vs-batch-table.html` (all empty
     stubs today).
4. **Fluent inventory.** Add `hts-cm-closure-*`, `hts-vs-validate-*`,
   `hts-vs-batch-*` keys to `locales/{en,es,de}/main.ftl`.
5. **The 4 F16-triage test hooks** (per §7.6.1 F16): E2-only
   dedicated hooks — the closure banner presence-under-navigation
   hook, the batch progress convergence hook, the batch row 4xx
   escalation hook, and the VS-validate canonical/instance/inline
   round-trip hook.
6. **Playwright** `e2e/tests/operations.spec.ts` (Slice E2 per plan;
   the file already exists as a placeholder stub).
7. **Detail-page embed.** Optional E1 work explicitly deferred:
   embedding the operations workbench under a "Free vs Pinned scope"
   toggle on `cs-detail` / `vs-detail` / `cm-detail`. E1 ships
   `scope=Free` from `/hts/operations` only.
8. **Toolchain (T1).** Restore the mandated
   `stable-x86_64-pc-windows-gnullvm` override once an LLVM /
   `x86_64-w64-mingw32-clang` binary is available on the host (see
   toolchain caveat above).

## Cross-check vs git

- **Untracked** (introduced or preserved during Slice E1):
  `crates/hts-ui/**` (every file under this directory, including
  `src/`, `templates/`, `tests/`, `e2e/`, `Cargo.toml`, `build.rs`),
  `edson/docs/hts-ui-slice-e1-output.md` (this file), and a
  `test-final.log` triage artifact at the repo root that should not
  be committed.
- **Modified** (pre-existing tracked files touched by Slice E1):
  `Cargo.toml` (unchanged since Slice A registered `hts-ui` as a
  default member), `Cargo.lock` (updated by `cargo test` runs),
  `.gitignore` (unchanged since Slice A), and
  `locales/{en,es,de}/main.ftl` (F17 additions + BOM/em-dash / dedupe
  cleanup).
- **Untouched, per plan invariants:** `crates/hts/src/server.rs`,
  `crates/hts/src/config.rs`, `crates/hts/Cargo.toml` (the pre-Slice
  E1 modifications by Slice A remain; Slice E1 added nothing to any
  of them), and `crates/ui/*` (HFS UI). `edson/docs/hts-ui-design.md`
  was read but not modified. `git status --short` confirms
  `crates/hts/*` shows as `M` from Slice A's mount-point wiring only;
  Slice E1 introduced no additional changes there.
