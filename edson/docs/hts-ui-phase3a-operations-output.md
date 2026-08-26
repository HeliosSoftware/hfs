# HTS-UI Phase 3a — Playwright Operations spec extension

- **Design reference:** `edson/docs/hts-ui-design.md` §7.6 (Operations
  workbench), §7.6.1 (20-finding advisor triage — F1 = D, F3, F6/F7,
  F11), §7.10 row 7.6 (states matrix).
- **Predecessor persistence:** `edson/docs/hts-ui-slice-e1-output.md`
  and `edson/docs/hts-ui-slice-e2-output.md` (Rust ring already 78/0
  green; commits `d99f3d972` + `0aaf22775`).
- **Phase 3a scope:** extend the Playwright ring at
  `crates/hts-ui/e2e/tests/operations.spec.ts` with append-only
  coverage for the Slice E2 features that E1's spec explicitly
  deferred: `$closure` invocation, VS `$validate-code` widened form,
  and `$batch-validate` fan-out. E1 blocks left intact.
- **Branch:** `feat/551-hts-ui` (uncommitted — parent will commit at
  the end of Phase 3).

## 1. File touched (append-only)

- `C:\Users\tercere\src\helios\hfs\crates\hts-ui\e2e\tests\operations.spec.ts`

E1 content spans lines 1–254 and is unmodified. Phase 3a appends
lines 255–651 (≈ 397 new lines, including block header comments) after
the last E1 `test.describe` block. No other files touched.

## 2. Describe blocks added (names + test counts)

Appended in the exact order the parent task called out:

| # | `test.describe` name                                              | Tests | Section anchor                                     |
|---|-------------------------------------------------------------------|-------|----------------------------------------------------|
| A | `HTS Operations $closure invocation (§7.6 E2)`                    | 3     | §7.6 F6/F7 pre-flight + happy path + banner gate   |
| B | `HTS Operations VS $validate-code widened form (§7.6 F3 E2)`      | 4     | §7.6 F3 source selector + §7.6 F4 canonical submit |
| C | `HTS Operations $batch-validate fan-out (§7.6 F1=D E2)`           | 2     | §7.6.1 F1 = D seed + terminal-state contract       |

**Total: 3 describe blocks / 9 new tests.**

Test-by-test:

- **A1** `submitting closure with an empty 'name' renders the OperationOutcome partial inline`
- **A2** `submitting closure with 'name' + one coding row renders result content in the workbench`
- **A3** `closure banner (F7) stays visible after a result render`
- **B1** `free-scope input renders three source radios plus all three input surfaces`
- **B2** `each source radio can be selected and clears the other two`
- **B3** `submitting canonical + code renders a result surface inline`
- **B4** `submitting inline mode with an empty body triggers the pre-flight OperationOutcome`
- **C1** `seed submit returns the skeleton table with per-row + progress htmx polling attributes`
- **C2** `progress endpoint reaches the terminal (no 'hx-trigger') arm after fan-out drains`

## 3. DOM selectors + Fluent strings the spec keys off

Role-based / label selectors (Playwright `getByRole` / `getByLabel`):

- `page.getByLabel(/Closure name/i)` — Fluent key `hts-cm-closure-name`
  (English: *Closure name*).
- `page.getByRole("button", { name: /Run/i, exact: false })` — Fluent
  key `hts-workbench-run` (English: *Run*).
- `page.getByText(/Closure state lives on the server/i)` — Fluent key
  `hts-operations-closure-stateless-warning`.

Structural DOM ids / classes / attributes (mirrored from the E2
template layer):

- `#hts-workbench-result` — shared swap target (F15 rename, E1).
- `.hts-op-banner[role='status']` — closure stateless banner (§7.6 F7);
  emitted by `templates/pages/operations.html` only when `flags.
  shows_closure_banner` is true.
- `.hts-outcome`, `.hts-outcome__code`, `role="alert"` — shared error
  partial `partials/hts-outcome.html`.
- `.hts-degraded`, `.hts-degraded__title` — shared degraded banner
  (used by `hts-cm-closure-result.html` and `hts-vs-validate-result.html`
  when `view.degraded_reason` is set).
- `.hts-op-workbench__badge` (with modifiers `--true`, `--false`,
  `--warning`, `--error`) — VS validate + batch-row result surfaces.
- `input[type='radio'][name='sourceMode'][value='canonical'|'instance'|'inline']`
  — Fluent keys `hts-vs-validate-source-canonical|instance|inline`.
- `input[name='sourceCanonical']`, `input[name='sourceInstance']`,
  `textarea[name='sourceInline']` — the three paired VS source input
  surfaces.
- `input[name='code']`, `input[name='system']` — VS validate code +
  system fields (Fluent keys `hts-vs-validate-code` / `-system`).
- `input[name='target']` — batch target ValueSet field (Fluent key
  `hts-vs-batch-target-value-set-label`).
- `input[name='row.code']`, `input[name='row.system']`,
  `input[name='row.display']` — repeatable batch row inputs (Fluent
  keys `hts-vs-batch-row-code|system|display`).
- `#hts-batch-row-0`, `#hts-batch-row-1` — skeleton row ids emitted by
  `partials/hts-vs-batch-table.html`.
- `#hts-batch-progress` — progress region emitted by both
  `hts-vs-batch-table.html` (seed) and `hts-vs-batch-progress.html`
  (polled updates).

hx-attribute regex matchers (per the parent's rule
"`toHaveAttribute('hx-get', /pattern/)` rather than exact string
match"):

- `/\/ui\/hts\/operations\/batch-validate\/row\/0\?batch_id=/` (row 0)
- `/\/ui\/hts\/operations\/batch-validate\/row\/1\?batch_id=/` (row 1)
- `/\/ui\/hts\/operations\/batch-validate\/progress\?batch_id=/` (progress)
- `hx-trigger` matches `/every\s+\d+\s*s/i` (interval intentionally
  not hardcoded).

Response-fragment shape matchers (raw HTML asserted via `page.request`
where DOM validation of `required` fields would otherwise block a
pre-flight submit):

- `hts-outcome__code` / `role="alert"` — invalid-input outcome.
- `Closure edges` | `No closure edges yet` | `hts-outcome__code` |
  `hts-degraded__title` — accepted result surfaces for the closure
  happy path.
- `hts-workbench-result` — the wrapping id re-emitted by every result
  partial (used to prove the outerHTML swap took hold).

## 4. Implementation-vs-design discrepancies noticed

Three gaps between the parent brief's phrasing and the actual E2
implementation. The specs adapt to what the code really does and
flag the gap in inline comments so Phase 3b / 3c can pick up cleanly.

1. **VS `$validate-code` source-radio visibility swap.** The parent
   asked for a spec that "selecting each radio swaps the visible
   input surface (canonical → url input; instance → id input; inline
   → textarea/JSON input)". Slice E2's
   `templates/partials/hts-vs-validate-input.html` renders all three
   paired input surfaces at the same time — selecting a radio only
   sets `sourceMode` on submit. There is no JS wiring in Slice E2
   that hides/shows the non-active surface. Spec B1 therefore
   asserts *all three surfaces are visible simultaneously*; spec B2
   asserts *native radio-group single-choice semantics*.

2. **VS `$validate-code` inline "invalid JSON" pre-flight.** The
   parent asked for `Submitting inline with invalid JSON → pre-flight
   error`. Slice E2's `run_vs_validate_code` pre-flight only rejects
   an *empty* `sourceInline` (via `trim().is_empty()`). Non-empty
   invalid JSON is passed through to
   `UpstreamClient::vs_validate_code`, which uses
   `serde_json::from_str(...).unwrap_or(Value::Null)` and simply
   omits the `valueSet` parameter when parsing fails — leaving HTS
   to render its own error. Spec B4 therefore exercises the *empty*
   inline body (which is deterministic) and comments that the
   invalid-JSON case is routed through the upstream error path.

3. **Batch progress route path.** The parent described the polling
   target as `/ui/hts/operations/batch-progress/<id>`. The actual E2
   route registered in `crates/hts-ui/src/operations.rs` is
   `/ui/hts/operations/batch-validate/progress` with the batch id on
   the `?batch_id=` query string. Spec C1's `toHaveAttribute("hx-get",
   /\/ui\/hts\/operations\/batch-validate\/progress\?batch_id=/)`
   asserts on the actual URL shape.

None of these gaps blocks the Playwright ring; they are documented so
Phase 3b (spec runner) reads them as intentional, and Phase 3c can
choose to close either the E2 template (radio show/hide) or the
brief itself.

## 5. `npx tsc --noEmit` outcome

**Skipped.**

Rationale: the `crates/hts-ui/e2e/` project has no `tsconfig.json`,
no local `node_modules/typescript`, and no `package.json` script that
wires up `tsc`. Running `npx tsc --noEmit` from that directory would
require fetching TypeScript from the npm registry through the corporate
proxy dance in `corporate-proxy-bypass.mdc`, which the parent brief
explicitly said to skip if it would cost more than ~60 s or fail
network. The append landed via `StrReplace` and passed the workspace's
linter (`ReadLints` reports zero issues on the file); syntactic
verification will be re-run by Phase 3b when Playwright itself
compiles the spec against `@playwright/test`.

## 6. Red flags for Phase 3b

- **Spec C2 polling timeout (15 s).** The batch progress terminal-state
  poll uses `expect.poll` with `timeout: 15_000` and staged
  intervals `[200, 400, 800, 1_200]`. On a Windows host under CPU
  load (or when the shared `hts` binary hasn't fully warmed up), the
  per-row upstream call plus the writer task's 6 s watchdog inside
  `run_batch_validate_row` could push the observation window close
  to 15 s. If Phase 3b sees flake, raise the outer timeout to
  30–45 s rather than tightening the interval schedule.
- **Spec C1 relies on the skeleton being present after a single
  htmx roundtrip.** `page.waitForResponse` on the POST is fired,
  then the assertions locate `#hts-batch-row-0` via `toBeAttached`
  with an 8 s timeout. If the htmx swap into `#hts-workbench-result`
  fails silently (e.g. the response has an unexpected `HX-*` header
  that htmx rejects), the DOM assertion will time out. Phase 3b
  should capture `page.on("response", …)` logs for the POST if C1
  fails to disambiguate template rendering vs client-side swap.
- **`page.request` sessions don't share cookies with `page`.** All
  raw HTTP calls use fresh Playwright request contexts. HTS UI has
  no auth surface today, so this is safe now — but if Phase 3b or
  later slices land session-scoped features (Slice G locale
  cookie?), the `page.request.*` specs would need to switch to
  `page.context().request` to inherit cookies.
- **Empty-SQLite fixture is silent for canonical submits.** Specs
  A2 / B3 assert on a *set* of allowed result surfaces (badge,
  outcome, degraded) rather than a specific one, because the boot
  fixture seeds no ValueSets. If Phase 3b introduces a seed step,
  those specs should be tightened to expect the more specific
  neutral-badge outcome — track under Phase 3c.
- **HTML5 `required` sidestep.** Specs A1 and B4 intentionally use
  `page.request.post` to submit empty required fields; the closure
  banner spec (A3) fills the required `name` field before clicking
  submit so no `noValidate` toggle is needed. This keeps the DOM
  interactive tests honest.

## 7. Not covered — Phase 3c / later

- **Batch cancel.** No cancel affordance exists in Slice E2 (the E2
  persistence doc §"Deuda for Slice F/G" states this explicitly).
  When it lands, add a spec that clicks cancel and asserts the
  progress region jumps to the terminal arm without waiting for all
  rows.
- **nojs batch fallback.** Would require a dedicated
  `e2e/tests/nojs/operations-batch.spec.ts` under the `nojs` project
  in `playwright.config.ts`. The parent brief explicitly deferred
  this to Phase 3c/later.
- **VS `$validate-code` instance-mode + CodeableConcept + advanced
  parameters happy paths.** Spec B3 covers only
  `sourceMode=canonical` + `mode=code`. Additional variants
  (`sourceMode=instance` against a seeded VS, `mode=CodeableConcept`
  with `coding[]` rows, the `Advanced parameters` `<details>` panel)
  are all wired at the handler + template level but not yet
  Playwright-covered.
- **Batch aggregated per-row result assertion.** Spec C1 asserts the
  skeleton shape and the polling attributes; the browser-side htmx
  loop's replacement of each `aria-busy` row with a completed
  `hts-vs-batch-row.html` fragment is documented as a
  `TODO(phase-3b)` inside C1. The lighter version (raw progress
  poll) is covered by C2.
- **Closure result-badge visual assertions.** Spec A2 accepts any
  of the shipped shapes (edge list, empty-graph, outcome, degraded).
  A dedicated spec that seeds a closure table then submits a second
  time (to see the edge list arm) is deferred pending a persistent-
  closure fixture.
- **Locale coverage for E2 copy.** All specs run against the
  default English catalog. Fluent-parity for `hts-cm-closure-*`,
  `hts-vs-validate-*`, and `hts-vs-batch-*` is enforced by the
  Rust ring; adding an `?lang=es` / `?lang=de` Playwright pass is a
  Phase 3c candidate.
- **Route-enum walker.** `route_enum.rs` already extends the merged
  Rust walker with the seven `?op=&resource=` entries per E2
  (§7.6.1 invariant #6); Playwright does not duplicate that matrix.

## 8. Cross-check vs git

- The single-file change lands under `crates/hts-ui/e2e/tests/`
  (already `??` untracked from E1) — no existing tracked file was
  modified, so `git status --porcelain` still shows only the E2
  untracked set plus this new doc under `edson/docs/`.
- No commits, no `git add`, no push. Parent Phase 3 owns commit
  discipline.
