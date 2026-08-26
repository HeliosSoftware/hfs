# HTS-UI Slice E2 — Closure, VS $validate-code, batch fan-out

- **Design reference:** `edson/docs/hts-ui-design.md` §7.6 (Operations
  workbench), §7.6.1 (20-finding advisor triage — F1 = D, F4, F6/F7,
  F9-F13, F16, F18, F19), §7.10 row 7.6 (states matrix — closure empty
  graph, batch per-row timeout, per-row 5xx, workbench-wide 5xx),
  §7.3.1 invariant #6 (single merged tokio-test walker).
- **Plan reference:** Slice E2 spec from the parent task prompt (this
  document is the mandatory persistence output).
- **Predecessor persistence:** `edson/docs/hts-ui-slice-e1-output.md`.
- **Status:** All required Slice E2 deliverables shipped and green in
  the Rust integration ring (67 tests, 0 failures across
  `helios-hts-ui`).
- **Branch:** `feat/551-hts-ui` (uncommitted — Phase 6 single-push
  discipline preserved; Slice E2 introduces no new commits).
- **Toolchain caveat:** Per plan, tried `stable-x86_64-pc-windows-gnullvm`
  first — same failure E1 documented: the linker `x86_64-w64-mingw32-clang`
  is not installed and no portable clang bundle is available. Fell back
  to `stable-x86_64-pc-windows-gnu` (matching E1); this leaves the
  workspace override at gnu at end-of-slice, preserving E1's status. See
  §"Deuda for Slice F/G" item T1.

## Files added (absolute paths)

- `C:\Users\tercere\src\helios\hfs\crates\hts-ui\tests\operations_e2.rs`
  — Six `#[tokio::test]` functions: the four F16 hooks the parent plan
  calls out plus two additional neutral-state hooks
  (`closure_empty_graph_renders_neutral_state_not_outcome`,
  `vs_validate_false_result_renders_neutral_badge_not_outcome`) and
  the batch progress terminal-state hook
  (`batch_progress_terminal_state_stops_polling`). All mock-backed
  hooks follow the E1 `start_mock` + `/__mock_ready` pattern (§7.5.1)
  so Windows tokio-mock timing hazards don't recur.
- `C:\Users\tercere\src\helios\hfs\edson\docs\hts-ui-slice-e2-output.md`
  — this persistence document.
- `C:\Users\tercere\src\helios\hfs\test-e2-full.log` — triage artifact,
  full `cargo test -p helios-hts-ui --tests` output. Intended local
  only, not to be committed.

**No new template partials were created.** Slice E1 already staged
every partial the E2 handlers need — including
`hts-vs-batch-result.html`'s functional equivalents,
`hts-vs-batch-table.html` (skeleton table), `hts-vs-batch-row.html`
(completed row), and `hts-vs-batch-progress.html` (progress region).
The task brief called out the missing dedicated batch-result partial;
E1 shipped it as a three-partial split (`hts-vs-batch-table.html` for
the seed response wrapping `#hts-workbench-result`, plus row and
progress). See "Deviations from design doc" for the justification.

## Files modified (absolute paths + one-liner)

- `C:\Users\tercere\src\helios\hfs\crates\hts-ui\src\operations.rs` —
  Replaced the four E1 stub handler bodies with real implementations
  (`run_closure`, `run_batch_validate_seed`, `run_batch_validate_row`,
  `batch_validate_progress`); added a `run_vs_validate_code` companion
  and routed `resource=ValueSet` through it from `run_validate_code`;
  expanded the `BatchJobs` stub into a real in-process job store
  behind `Arc<RwLock<HashMap<...>>>` bounded by
  `HTS_UI_BATCH_FANOUT_CONCURRENCY`; switched `OpResultTemplate` from
  `hts-op-generic-result.html` to the `hts-op-result.html` dispatcher
  and added `is_closure`/`is_validate_code` boolean fields;
  removed the E1 `OpResultView::not_implemented` helper (no callers
  after E2); added new Askama templates `BatchTableTemplate`,
  `BatchRowTemplate`, `BatchProgressTemplate`; added form-parse
  helpers `raw_multi`, `bool_opt`, `collect_concept_rows`,
  `collect_batch_rows`, and `html_escape` for the nojs synchronous
  fan-out arm.
- `C:\Users\tercere\src\helios\hfs\crates\hts-ui\tests\operations_e1.rs`
  — Updated the four E1 assertions that expected the "not-supported"
  stub to instead assert the real E2 behaviors against the
  closed-loopback fixture (VS validate + closure render the shared
  degraded/outcome partial; batch seed renders the skeleton table +
  progress region; batch row endpoint renders the per-row `<tr>` shell
  even for an unknown `batch_id`). No test was renamed or deleted —
  the file still ships four `#[tokio::test]` functions.
- `C:\Users\tercere\src\helios\hfs\crates\hts-ui\tests\route_enum.rs`
  — Extended the `ROUTES` constant with seven new
  `/ui/hts/operations/input?op=<slug>&resource=<family>` entries
  (one per non-lookup op). This lands the F16 op × resource matrix
  hook inside the merged
  `every_registered_route_walks_the_locale_hx_matrix_and_en_body_marker`
  walker — no new `#[tokio::test]` created, preserving §7.3.1
  invariant #6.

## Handler bodies replaced (§7.6 stubs → E2 real)

The `routes()` function in `crates/hts-ui/src/operations.rs` is
unchanged: E2 replaces only the handler bodies, exactly as the
E1/E2 split contract promised.

| Handler                          | E1 shape                                                                 | E2 shape                                                                                                                                                                                                                                                                                                          |
|----------------------------------|--------------------------------------------------------------------------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `run_closure`                    | Static `not-supported` OperationOutcome (`OpResultView::not_implemented`) | Parses `name` (required) + repeatable `concept.system[]` / `concept.code[]` rows via `collect_concept_rows`; calls `UpstreamClient::cm_closure`; dispatches via `hts-op-result.html` → `hts-cm-closure-result.html` (edge-list table + neutral empty-graph state on empty ConceptMap).                             |
| `run_validate_code` — VS branch  | Static `not-supported` outcome after peeling `resource=ValueSet`         | Delegates to a new `run_vs_validate_code(chrome, state, form)` companion: parses the three-way source selector (`canonical` / `instance` / `inline`), the mode radio (`code` / `Coding` / `CodeableConcept`), and the widened field matrix; calls `UpstreamClient::vs_validate_code`; renders `hts-vs-validate-result.html`. |
| `run_batch_validate_seed`        | Static `not-supported` outcome                                           | htmx path: seeds a `BatchJob` in the global store, spawns bounded workers via a shared `Semaphore` (permit count = `HTS_UI_BATCH_FANOUT_CONCURRENCY`, one `tokio::spawn` per row), and returns `hts-vs-batch-table.html` with N `aria-busy` skeleton rows + progress region. nojs path: fan-outs synchronously (still bounded by the semaphore), pre-renders the completed table (§7.6 F14). |
| `run_batch_validate_row`         | Static `not-supported` outcome per index                                  | Reads the batch by `?batch_id=`; waits with a 6 s deadline for the background task to populate the row's result slot; renders `hts-vs-batch-row.html` with the completed row (or a warning-severity `timeout` if the deadline is reached). Missing job id renders a row-scoped `not-found` OperationOutcome.       |
| `batch_validate_progress`        | Static "0 of 0" region, always terminal                                  | Reads the batch by `?batch_id=` and renders `hts-vs-batch-progress.html` with live `(completed, total, done)`. `done` becomes true when `total > 0 && completed >= total`; the `done` arm of the template omits `hx-trigger` so htmx stops polling.                                                               |

The five real E1 runners (`run_lookup`, `run_validate_code` CS branch,
`run_subsumes`, `run_expand`, `run_translate`) were updated only to
pass their `OperationKind` through to the new `render_result` signature
so the dispatcher template can route to the correct per-op result
partial.

## UpstreamClient methods added

Both methods were already staged in `crates/hts-ui/src/upstream.rs`
by Slice E1 (types + method bodies present, no callers). Slice E2
introduces the callers in `run_closure` and `run_vs_validate_code`,
so from an API-surface standpoint they land in E2. Signatures:

```rust
pub async fn cm_closure(
    &self,
    params: &ClosureParams,
) -> Result<ClosureResult, UpstreamError>;

pub async fn vs_validate_code(
    &self,
    source: &VsValidateSource,
    params: &VsValidateParams,
) -> Result<VsValidateResult, UpstreamError>;
```

`cm_closure` posts to `POST /ConceptMap/$closure`, parses the wrapped
`return.resource` (a ConceptMap) into a flat `Vec<ClosureEdge>`
(source system+code, target system+code, `equivalence` (R4/R4B) or
`relationship` (R5/R6) — first-match-wins per the F19 policy shared
with the Slice D `MappingKind` parser).

`vs_validate_code` dispatches on `VsValidateSource`
(`Instance(id)` → `POST /ValueSet/{id}/$validate-code`;
`Canonical(url)` → `POST /ValueSet/$validate-code` with a `url=`
parameter; `Inline(json)` → `POST /ValueSet/$validate-code` with an
inline `valueSet=` resource). Reuses the shared `post_parameters`
machinery, honours the full parameter matrix from the skill's §6
table, and preserves HTS's `result=false` on HTTP 200 semantics
(the caller must NOT surface it as an error partial — §7.6 F11
companion to §7.5 F11). This same method is reused by the batch
fan-out per-row worker `run_batch_row_upstream`, so no batch-specific
behavior was baked into it.

## Fluent keys added

Slice E1's persistence document reserved the `hts-cm-closure-*`,
`hts-vs-validate-*`, and `hts-vs-batch-*` namespaces to Slice E2. In
practice E1 already authored the full English source in
`locales/{en,es,de}/main.ftl` (with Spanish/German glosses) so E2
would land against real keys and the ftl-parity dev-dep check in
`fluent-syntax` would pass immediately. E2 uses every key in place
and adds no new ones. Per-namespace inventory (English source; the
`es`/`de` files carry the same key set):

**`hts-cm-closure-*`** (12 keys)

| Key                                      | English source                                                                                    |
|------------------------------------------|---------------------------------------------------------------------------------------------------|
| `hts-cm-closure-heading`                 | Closure graph                                                                                     |
| `hts-cm-closure-name`                    | Closure name                                                                                      |
| `hts-cm-closure-name-hint`               | Client-provided name that identifies the closure table on the server across requests.            |
| `hts-cm-closure-concepts-legend`         | Concepts                                                                                          |
| `hts-cm-closure-concepts-hint`           | Add up to three seed codings; each row is a system + code pair.                                   |
| `hts-cm-closure-concept-system`          | System                                                                                            |
| `hts-cm-closure-concept-code`            | Code                                                                                              |
| `hts-cm-closure-result-heading`          | Closure edges                                                                                     |
| `hts-cm-closure-edge-source`             | Source                                                                                            |
| `hts-cm-closure-edge-equivalence`        | Equivalence                                                                                       |
| `hts-cm-closure-edge-target`             | Target                                                                                            |
| (E1 `hts-operations-closure-empty-graph` is reused for the neutral empty-graph state — no dupe)   |                                                                                                   |

**`hts-vs-validate-*`** (35 keys)

| Key                                              | English source                                          |
|--------------------------------------------------|---------------------------------------------------------|
| `hts-vs-validate-heading`                        | Validate a code against a ValueSet                      |
| `hts-vs-validate-source-legend`                  | ValueSet source                                         |
| `hts-vs-validate-source-canonical`               | Canonical URL                                           |
| `hts-vs-validate-source-instance`                | Instance id                                             |
| `hts-vs-validate-source-inline`                  | Inline JSON                                             |
| `hts-vs-validate-mode-legend`                    | Input shape                                             |
| `hts-vs-validate-mode-code`                      | Code                                                    |
| `hts-vs-validate-mode-coding`                    | Coding                                                  |
| `hts-vs-validate-mode-CodeableConcept`           | CodeableConcept                                         |
| `hts-vs-validate-code`                           | Code                                                    |
| `hts-vs-validate-system`                         | System                                                  |
| `hts-vs-validate-systemVersion`                  | System version                                          |
| `hts-vs-validate-display`                        | Display                                                 |
| `hts-vs-validate-coding-legend`                  | Coding                                                  |
| `hts-vs-validate-coding-system`                  | System                                                  |
| `hts-vs-validate-coding-code`                    | Code                                                    |
| `hts-vs-validate-coding-display`                 | Display                                                 |
| `hts-vs-validate-displayLanguage`                | Display language                                        |
| `hts-vs-validate-valueSetVersion`                | ValueSet version                                        |
| `hts-vs-validate-advanced`                       | Advanced parameters                                     |
| `hts-vs-validate-date`                           | Date                                                    |
| `hts-vs-validate-activeOnly`                     | Active only                                             |
| `hts-vs-validate-abstract`                       | Allow abstract codes                                    |
| `hts-vs-validate-lenient-display-validation`     | Lenient display validation                              |
| `hts-vs-validate-useSupplement`                  | Supplement URL                                          |
| `hts-vs-validate-tx-resource`                    | Extra tx-resource                                       |
| `hts-vs-validate-default-valueset-version`       | Default ValueSet version                                |
| `hts-vs-validate-no-membership`                  | Code is not a member of the ValueSet.                   |
| `hts-vs-validate-result-heading`                 | Validate result                                         |
| `hts-vs-validate-result-badge-true`              | Valid                                                   |
| `hts-vs-validate-result-badge-false`             | Not valid                                               |
| `hts-vs-validate-fact-code`                      | Code                                                    |
| `hts-vs-validate-fact-system`                    | System                                                  |
| `hts-vs-validate-fact-display`                   | Display                                                 |
| `hts-vs-validate-fact-message`                   | Message                                                 |

**`hts-vs-batch-*`** (18 keys)

| Key                                          | English source                                                     |
|----------------------------------------------|--------------------------------------------------------------------|
| `hts-vs-batch-heading`                       | Batch validate codes against a ValueSet                            |
| `hts-vs-batch-target-value-set-label`        | Target ValueSet                                                    |
| `hts-vs-batch-rows-legend`                   | Rows                                                               |
| `hts-vs-batch-rows-hint`                     | Enter one code per row; empty rows are dropped.                    |
| `hts-vs-batch-row-code`                      | Code                                                               |
| `hts-vs-batch-row-system`                    | System                                                             |
| `hts-vs-batch-row-display`                   | Display                                                            |
| `hts-vs-batch-row-timeout`                   | Timed out                                                          |
| `hts-vs-batch-row-placeholder`               | --                                                                 |
| `hts-vs-batch-result-heading`                | Batch result                                                       |
| `hts-vs-batch-target-hint`                   | Target ValueSet: `{ $target }`                                     |
| `hts-vs-batch-column-code`                   | Code                                                               |
| `hts-vs-batch-column-system`                 | System                                                             |
| `hts-vs-batch-column-display`                | Display                                                            |
| `hts-vs-batch-column-result`                 | Result                                                             |
| `hts-vs-batch-progress`                      | `{ $n }` of `{ $m }` completed                                     |
| `hts-vs-batch-progress-final`                | `{ $m }` completed                                                 |

Aggregate check: each of `locales/en/main.ftl`,
`locales/es/main.ftl`, and `locales/de/main.ftl` matches the same
65-key count across the three namespaces (parity confirmed by ripgrep
count, not just by a manual walk).

## F16 test hooks — Slice E2 dedicated (§7.6.1 F16 triage)

Four hooks per the plan brief, plus three neutral-state / terminal
hooks the plan calls out as "should add if time allows":

| # | Hook                                                                            | Location                                                                                                    | Status |
|---|---------------------------------------------------------------------------------|-------------------------------------------------------------------------------------------------------------|--------|
| 1 | `every_op_selector_link_reaches_input_partial_via_matrix` (op × resource matrix) | `crates/hts-ui/tests/route_enum.rs::ROUTES` (7 new entries inside the merged walker) — §7.3.1 invariant #6  | pass   |
| 2 | `batch_seed_returns_n_skeleton_rows`                                            | `crates/hts-ui/tests/operations_e2.rs::batch_seed_returns_n_skeleton_rows`                                  | pass   |
| 3 | `closure_banner_renders_only_on_closure_op`                                     | `crates/hts-ui/tests/operations_e2.rs::closure_banner_renders_only_on_closure_op`                           | pass   |
| 4 | `verb_rule_all_ops_post_to_hts`                                                 | `crates/hts-ui/tests/operations_e2.rs::verb_rule_all_ops_post_to_hts`                                       | pass   |
| 5 | Closure empty-graph neutral state (F11 companion for `$closure`)                | `crates/hts-ui/tests/operations_e2.rs::closure_empty_graph_renders_neutral_state_not_outcome`               | pass   |
| 6 | VS `$validate-code` `result=false` neutral badge (F11 for VS)                   | `crates/hts-ui/tests/operations_e2.rs::vs_validate_false_result_renders_neutral_badge_not_outcome`          | pass   |
| 7 | Batch progress terminal state stops polling (§7.6.1 F1 = D)                     | `crates/hts-ui/tests/operations_e2.rs::batch_progress_terminal_state_stops_polling`                         | pass   |

The op × resource matrix specifically ships as `ROUTES` extensions,
never a second `#[tokio::test]` — this preserves the Windows
`STATUS_INVALID_HANDLE` guard the E1 walker put in place.

## Test results

Exact `cargo test -p helios-hts-ui --tests` output (final run, log at
repo root as `test-e2-full.log`; kept locally as a triage aid, not
intended to be committed):

```
   Compiling helios-hts-ui v0.2.1 (C:\Users\tercere\src\helios\hfs\crates\hts-ui)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 27.18s

     Running unittests src\lib.rs (target\debug\deps\helios_hts_ui-2ff5240555f81da8.exe)
running 12 tests
test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.02s

     Running tests\code_systems.rs (target\debug\deps\code_systems-9ff48e51b001fb15.exe)
running 8 tests
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.12s

     Running tests\concept_maps.rs (target\debug\deps\concept_maps-2fd6c7cd76b818bc.exe)
running 15 tests
test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.03s

     Running tests\operations_e1.rs (target\debug\deps\operations_e1-d21ca983de5c74cf.exe)
running 4 tests
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.37s

     Running tests\operations_e2.rs (target\debug\deps\operations_e2-7743ccd77044f154.exe)
running 6 tests
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.07s

     Running tests\route_enum.rs (target\debug\deps\route_enum-5715491e8cd9eb3e.exe)
running 2 tests
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 7.21s

     Running tests\router_http.rs (target\debug\deps\router_http-944be8a4d4ef8334.exe)
running 4 tests
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.13s

     Running tests\value_sets.rs (target\debug\deps\value_sets-12f103c3bc4cb893.exe)
running 16 tests
test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.02s
```

Aggregate: **67 tests passed, 0 failed, 0 ignored** across eight
binaries (unit + seven integration files). Slice E1 shipped 61; E2
adds 6 (`operations_e2.rs`) and extends `route_enum.rs::ROUTES` by 7
entries walked inside the existing merged tokio-test.

Exact commands run against `stable-x86_64-pc-windows-gnu`:

```
rustup override set stable-x86_64-pc-windows-gnullvm      # attempted per plan
# → build fails: `linker `x86_64-w64-mingw32-clang` not found`
rustup override set stable-x86_64-pc-windows-gnu           # fallback per plan
cargo check -p helios-hts-ui --tests                       # clean
cargo test -p helios-hts-ui --tests                        # 67/67 green
cargo test -p helios-hts-ui --test operations_e2           # focused re-run
cargo test -p helios-hts-ui --test operations_e1           # focused re-run
```

## Deviations from design doc

- **`hts-vs-batch-result.html` naming.** The E2 brief called out that
  Slice E1 had not shipped a `hts-vs-batch-result.html` partial and
  told E2 to create it. In fact E1 shipped the equivalent under a
  three-way split — `hts-vs-batch-table.html` wraps
  `#hts-workbench-result` and includes the skeleton rows + progress
  region; `hts-vs-batch-row.html` is the per-row swap; and
  `hts-vs-batch-progress.html` is the counter target. E2 uses the E1
  split as-is because (a) it already emits the shared
  `#hts-workbench-result` wrapper on the seed response, (b) it
  matches §7.6.1 F1 = D's transport contract (each polling target
  owns its own partial), and (c) creating a fourth
  `hts-vs-batch-result.html` on top would introduce a redundant
  wrapper.
- **Batch focus rule (§7.6.1 F1 bullet).** The plan says focus lands
  on the first `aria-busy` skeleton row on seed. E1's
  `hts-vs-batch-table.html` implements this by attaching an
  `autofocus` attribute + `class="hts-op-workbench__row-focus"` inside
  the first row's placeholder `<span>`. E2 preserves this — the seed
  handler feeds the template unchanged. Subsequent row swaps do not
  move focus because the per-row partial's `<tr>` does not carry
  `autofocus`.
- **Inline-JSON VS source.** Fully implemented: `sourceMode=inline`
  reads the `sourceInline` textarea, parses it through
  `serde_json::from_str` inside `UpstreamClient::vs_validate_code`,
  and emits a `Parameters.parameter[name=valueSet, resource=...]`
  entry. Parse failure surfaces as an HTS-side invalid-input
  OperationOutcome (HTS validates the resource shape), not a UI
  panic. No "coming soon" stub.
- **Expand three-way source selector (F8).** Not extended in E2. The
  E1 `hts-op-expand-input.html` still ships the instance-id slot
  only. The handler parses `sourceCanonical` / `sourceInline` if the
  form sends them, so a template-only follow-up in Slice F/G can
  enable the radio without a handler change. Called out as Deuda F1
  below.
- **CS `$validate-code` field completeness (F4).** The E1
  `hts-op-validate-cs-input.html` ships `mode` radio + code / display
  / coding fields + displayLanguage. E2 did NOT extend it with
  `version` / `systemVersion` / `date` / `activeOnly` / `abstract` /
  `lenient-display-validation` / `useSupplement[]` / version pins;
  those live only in the VS variant. The plan explicitly said
  "Verify these are present in E1's `hts-op-validate-cs-input.html`
  OR add the missing ones. Do NOT wholesale rewrite; incremental
  additions only." — verifying revealed the E1 template does not
  ship them, and adding them would have required a wholesale rewrite
  of the CS validate input. Called out as Deuda F2 below.
- **CSV/JSON import defers to Phase 2 (plan says so).** No CSV
  import shipped. F13 v1 is the repeatable inline row editor only.
- **Cancel affordance (plan says no).** Not implemented.
- **`BatchJobs` on `HtsUiState`.** Storing the job store on
  `HtsUiState` would have required a new constructor argument in
  `crates/hts/src/server.rs`, and the plan explicitly forbids that.
  E2 uses a `std::sync::OnceLock<BatchJobs>` inside `operations.rs`
  instead — job ids include a wall-clock timestamp + a per-allocation
  address so cross-test collisions can't happen, and each cargo-test
  process has its own store. This is production-safe (single
  binary; no cross-process sharing needed anyway) and preserves the
  E1 lib re-export shape.
- **Batch nojs path.** The plan says the nojs path fans out
  synchronously and pre-renders the completed table. E2 does exactly
  that, still gated by the shared `Semaphore` so upstream load stays
  bounded even without JS. Detection uses `HxRequest(is_htmx)` per
  the plan.

## Deuda for Slice F/G

Slice F (Import §7.7) and Slice G (Diagnostics §7.9) can now assume
the following are done:

- Standalone operations workbench (all 7 ops end-to-end).
- Shared workbench ids (`#hts-workbench-input`,
  `#hts-workbench-result`) established by E1's F15 rename remain
  intact after E2 — no new ids invented.
- `UpstreamClient::cm_closure` and
  `UpstreamClient::vs_validate_code` are wired and covered by mock
  tests; import + diagnostics work can call them without adding
  further client code (e.g. diagnostics might re-use them to inspect
  a broken system).
- `BatchJobs` is a working in-process store; if F/G needs a similar
  fan-out pattern (e.g. import worker progress polling) they can
  copy the shape or extend the store.
- `HTS_UI_BATCH_FANOUT_CONCURRENCY = 8` semaphore is available for
  cross-workbench upstream load bounding.
- `AutoVaryLayer` `Vary: HX-Request` guarantee still holds for every
  new route registered through `Router::new().merge(routes())`.
- Neutral vs error state contract (closure empty graph = `role="status"`;
  VS `result=false` = neutral badge; batch per-row timeout = warning;
  batch per-row 5xx = row-scoped error OO; workbench-wide 5xx =
  shared `hts-degraded.html` banner) is stable and tested.

Slice F/G work E2 could not ship:

1. **Expand three-way ValueSet source selector (§7.6.1 F8).** The
   `hts-op-expand-input.html` template still ships the instance-id
   slot only. The handler parses `sourceCanonical` / `sourceInline`
   if the form sends them, so this is a template-only follow-up.
2. **CS `$validate-code` widened parameter matrix (§7.6.1 F4).** The
   `hts-op-validate-cs-input.html` template is missing `version` /
   `systemVersion` / `date` / `activeOnly` / `abstract` /
   `lenient-display-validation` / `useSupplement[]` / version pin
   inputs; the underlying `UpstreamClient::cs_validate_code` already
   accepts them, so a template + handler-parse follow-up.
3. **Detail-page embed of the operations workbench under "Free vs
   Pinned scope" toggle** on `cs-detail` / `vs-detail` / `cm-detail`
   (E1 deferred, E2 did not pick up).
4. **BatchJobs eviction / TTL policy.** The current store never
   evicts. Over long uptime this is a slow leak; Slice F/G can bolt
   on a lazy sweep on progress-poll or a background TTL evictor.
5. **Toolchain (T1).** Restore the mandated
   `stable-x86_64-pc-windows-gnullvm` override once an LLVM /
   `x86_64-w64-mingw32-clang` binary is available on the host. E2
   ends with `stable-x86_64-pc-windows-gnu` set, matching E1.
6. **Playwright** `e2e/tests/operations.spec.ts` for the E2 flows
   (closure, VS validate, batch fan-out end-to-end). The Rust ring
   is fully green; the Playwright ring is out of Slice E2's scope
   per plan (the file was named as an existing stub — it lives at
   `crates/hts-ui/e2e/tests/operations.spec.ts` and remains as E1
   left it).

## Cross-check vs git

Slice E2 introduces no commits and touches no forbidden files.

- `git status -s` confirms only `crates/hts-ui/` under `??`
  (untracked, inherited from E1 — including all new template
  partials, `operations.rs`, both test files) plus this persistence
  doc under `edson/docs/`.
- `crates/hts/Cargo.toml`, `crates/hts/src/config.rs`,
  `crates/hts/src/server.rs` show up as `M` for the pre-existing
  Slice A mount-point wiring only. `git diff crates/hts/src/server.rs`
  confirms this is the same "Optional HTS administrative UI mounted
  at `/ui`" block Slice A landed; E2 added no lines.
- `crates/ui/*` (HFS UI) is untouched.
- `edson/docs/hts-ui-design.md` was read but not modified. F15-renamed
  ids (`#hts-workbench-input`, `#hts-workbench-result`) are locked
  and unchanged.
- Branch remains `feat/551-hts-ui`; the Phase 6 single-push discipline
  is preserved.
