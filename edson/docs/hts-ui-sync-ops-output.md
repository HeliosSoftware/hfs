# Sub-Ops output — HTS UI design sync

## Edit: REWRITE §7.6.1 stale E1 block (D3 M3 M4 M5 M7 D11)

- **File**: `edson/docs/hts-ui-design.md`
- **Rationale**: The L1619-L1702 block was written mid-Slice-E1 and describes
  closure + batch-validate as `not-supported` stubs, the batch state store as
  "E2 scope", `operations.spec.ts` as "not run", and Playwright as blocked on
  the Slice G seed loader. All four claims are now wrong: E2 landed real
  handlers backed by a process-global `BatchJobs` store bounded by
  `HTS_UI_BATCH_FANOUT_CONCURRENCY = 8`, and Phase 3 shipped three residual
  fixes (M3 batch skeleton swap-race, M4 op-selector ARIA, M5 wrapper-vs-form
  id) that further hardened the ops surface. Rust ring is 80/0 green;
  Playwright `operations.spec.ts` is 75/0/3-skipped green. This rewrite
  converts the stub-era prose to current-state notes without renumbering any
  F-references or restructuring §7.6.1.
- **Line range** (verified by re-reading L1615-1710):
  - **Start**: L1619 — first byte is the bold paragraph opener
    `**Slice E1 shipping notes (implementation).** The initial PR ships the`
  - **End**: L1702 — last byte is the closing paren of
    `(the free-scope inputs accept ad-hoc system + code).`
  - **Exclusions**: L1618 is a blank line separator from the F1-F20 triage
    list above (kept untouched); L1703 is a blank line separator from the
    surviving `**Slice E test hooks (F16 triage)**` bold paragraph at L1704
    (also kept untouched). Old and new strings therefore both terminate at
    the closing paren with no trailing newline in the replacement content.
    The parent applying `StrReplace` should preserve L1618 + L1703 as-is.
- **F-numbers preserved** (all appear in the rewritten block as anchors so
  cross-refs from §2 / §4 / §7.10 / §8 still resolve):
  - F1, F3, F4, F6, F7, F8, F10, F12, F15, F16, F17 — all referenced in the
    stale block AND retained in the rewrite.
  - F16 is the only F-number that is NOT defined in the main §7.6.1 triage
    list (L1545-L1617); it lives only in the block being rewritten (L1698)
    and in the surviving `**Slice E test hooks (F16 triage)**` bold
    paragraph at L1704. The rewrite keeps a `(F16)` reference in the
    Playwright ops parity bullet so no anchor is orphaned.
  - No F-number is introduced for the first time inside the block, so
    nothing is at risk of vanishing from the doc.
- **M / D anchors introduced by the rewrite** (Phase 3 sync labels, safe to
  add — no existing occurrences elsewhere in the design doc):
  - `M3` — batch skeleton swap-race, `operations.spec.ts:531` fix.
  - `M4` — Grupo A op-selector ARIA (`<nav>` not `role="tablist"`).
  - `M5` — Grupo A wrapper-vs-form id (wrapper `<div>` no longer shadows
    `#hts-workbench-input`).
  - `M7` — metadata-workbench slot placeholder cross-ref to §7.3 / §7.4 /
    §7.5.
  - `D11` — F3 scope-wrapper deferral, folded into current-state notes
    instead of the obsolete "Slice E2 will factor" stub context.
  - `D3` — kept as the umbrella tag for the "detail-page embed vs
    standalone workbench" surface; matches the sync-plan label the parent
    uses to group these edits.

### old_string (verbatim, L1619-L1702)

```
**Slice E1 shipping notes (implementation).** The initial PR ships the
seven-op workbench shell, the widened input surfaces for the five
"real" operations (`$lookup`, `$validate-code`, `$subsumes`, `$expand`,
`$translate`), and stub handlers for the two new operations
(`$closure`, `batch-validate`) that keep the URL contract but return
a `not-supported` OperationOutcome until Slice E2 lands the full run
logic. Divergences worth flagging for reviewers:

- **Scope wrapper (F3).** Slice E1 keeps the resource-family detail
  pages (`crates/hts-ui/templates/pages/cs-detail.html`,
  `vs-detail.html`, `cm-detail.html`) rendering the existing
  Slice B/C/D input partials directly with the resource pinned inline;
  the outer `<fieldset name="scope">` wrapper is realized only inside
  the standalone workbench templates (`hts-op-lookup-input.html`,
  `hts-op-validate-cs-input.html`, `hts-op-subsumes-input.html`,
  `hts-op-expand-input.html`, `hts-op-translate-input.html`,
  `hts-vs-validate-input.html`, `hts-cm-closure-input.html`,
  `hts-vs-batch-input.html`). Detail-page embeds retain their existing
  behavior; Slice E2 will factor the two renderings behind a shared
  `WorkbenchScope` partial once the run handlers land and the scope
  enum has a real second caller. Reviewers cross-checking against F3
  should read this as "F3 realized in the standalone workbench,
  detail-page embed refactor deferred to E2".
- **Batch state store (F1 = D, F10).** The
  `HTS_UI_BATCH_FANOUT_CONCURRENCY = 8` constant lives in
  `crates/hts-ui/src/operations.rs` alongside the batch stubs. E1
  does not yet attach a
  `RwLock<HashMap<BatchId, BatchJobState>>` field to
  `HtsUiState` — the seed and per-row handlers return a
  `not-supported` outcome, and the progress endpoint returns a static
  `<div id="hts-batch-progress" role="status">` with the polling
  trigger already omitted (so nojs and JS rings both see the "done"
  arm). Wiring the real state store and the tokio `Semaphore`-bounded
  fan-out is the E2 scope. `crates/hts/src/server.rs` was NOT
  modified — the batch state will land as a module-static
  `LazyLock<RwLock<..>>` inside `operations.rs` in E2 to keep the
  `HtsUiState` constructor stable.
- **Threshold panel (F12).** The op-conditional rendering is realized
  via `OpsFlags::show_advanced_panel` (a precomputed boolean on the
  input template struct rather than a `crate::` path lookup inside
  the Askama template) so the panel appears in the DOM only for
  `?op=expand`. Verified by `threshold_panel_hidden_for_non_expand_ops`
  in `tests/operations.rs`.
- **VS Validate three-way source (F3 + F4).** The VS Validate input
  partial renders three source-selector radios (canonical URL /
  instance id / inline JSON) inside a single `<fieldset>`; the E1
  run handler dispatches through
  `UpstreamClient::vs_validate_code(source, params)` and routes to
  `POST /ValueSet/{id}/$validate-code` for `Instance`, and
  `POST /ValueSet/$validate-code` with `url=` / `valueSet=` for
  `Canonical` and `Inline`.
- **Closure stub (F6 + F7).** The E1 `run_closure` handler returns a
  `not-supported` outcome; the `hts-cm-closure-result.html` edge-list
  partial exists on disk with the neutral empty-graph state
  (`hts-operations-closure-empty-graph`) so E2 can wire the
  `UpstreamClient::cm_closure` call site without re-authoring the
  result template. The stateless-warning banner is fully wired in E1
  (renders only for `?op=closure`) and asserted in
  `closure_banner_renders_only_on_closure_op`.
- **Fluent-key parity (F8 + F17).** Slice E1 ends the workspace at
  426 `hts-` keys per locale (`en` / `es` / `de`), all with parity.
  During the E1 append, 26 duplicate keys were briefly introduced
  (existing `hts-cs-validate-*`, `hts-vs-expand-*`,
  `hts-cs-subsumes-*`, `hts-workbench-*`, and `hts-nav-operations`
  keys) and then removed; the reviewer-facing invariant is that
  Slice E adds new keys under the `hts-operations-*`,
  `hts-cm-closure-*`, `hts-vs-validate-*`, and `hts-vs-batch-*`
  namespaces plus small extensions to existing CS Lookup, CS
  Validate, and VS Expand namespaces.
- **Windows `reqwest` proxy interference (test-ring diagnostic).**
  The corporate `HTTP_PROXY` / `HTTPS_PROXY` env vars route
  loopback traffic through an off-VPN corporate proxy, which caused
  the `start_mock`-based tests (concept_maps / value_sets) to fail
  with "Could not reach the terminology server" panics that looked
  like `axum::serve` flakiness. Fix: set
  `NO_PROXY=127.0.0.1,localhost` (or clear the proxy vars) before
  running `cargo test -p helios-hts-ui`. This is an environment
  invariant, not a code change, and belongs alongside the
  `corporate-proxy-bypass.mdc` rule in `~/.cursor/rules`.
- **Playwright spec (F16 optional).** `e2e/tests/operations.spec.ts`
  is authored but not run — the seed loader (Slice G) has not landed.
  The spec targets the E1 shell + widened input surfaces; the closure
  and batch run assertions land alongside E2. No new seed identifiers
  are required (the free-scope inputs accept ad-hoc system + code).
```

### new_string (complete replacement block)

```
**Slice E1 + E2 shipping notes (post-Phase-3).** The Slice E1 PR shipped
the seven-op workbench shell and the widened input surfaces for the five
"real" operations (`$lookup`, `$validate-code`, `$subsumes`, `$expand`,
`$translate`); Slice E2 replaced the two `not-supported` stubs (`$closure`
and `batch-validate`) with real handlers backed by a process-global
`BatchJobs` store and a shared `UpstreamClient::cm_closure` path. Three
Phase 3 residual fixes then stabilized the ops surface without changing
the on-wire contract (M3 batch skeleton swap-race, M4 op-selector ARIA
semantics, M5 wrapper-vs-form id contract). Test parity holds:
`cargo test -p helios-hts-ui` is 80/0 green under the `NO_PROXY` proxy
bypass; Playwright `e2e/tests/operations.spec.ts` is
75 passed / 0 failed / 3 skipped. Current-state notes worth pinning:

- **Batch fan-out in production (F1 = D, F10).** The seed handler
  `run_batch_seed_htmx` in `crates/hts-ui/src/operations.rs` inserts a
  job into a process-global `OnceLock<BatchJobs>` store, then spawns
  one `tokio::spawn` per row bounded by a shared `Semaphore` sized to
  `HTS_UI_BATCH_FANOUT_CONCURRENCY = 8` (compile-time const exported
  from `upstream.rs`). Per-row target
  `/ui/hts/operations/batch-validate/row/{i}?batch_id=…` waits on the
  job with a ~6 s deadline (`run_batch_validate_row`). The progress
  region at `/ui/hts/operations/batch-validate/progress` omits the
  polling trigger on the terminal `done` arm so htmx polling halts
  naturally. A hard cap `HTS_UI_BATCH_MAX_ROWS = 50` collapses over-cap
  submissions to an invalid-input `OperationOutcome` without seeding a
  job. `crates/hts/src/server.rs` remains untouched — the batch store
  is module-static (`static BATCH_JOBS: OnceLock<BatchJobs>`), not a
  new `HtsUiState` field, honoring the E1 constructor-stability note.
- **Closure real handler (F6 + F7).** `run_closure` posts to
  `POST /ConceptMap/$closure` via `UpstreamClient::cm_closure`, reading
  `name` plus repeatable `concept.system` / `concept.code` rows through
  the shared `collect_concept_rows` helper (also reused by VS Validate
  `CodeableConcept` mode). Results render through `hts-op-result.html`
  into `hts-cm-closure-result.html` (F6 edge-list partial, per-op
  family preserved per F11 = A). The stateless-warning banner remains
  gated on `flags.shows_closure_banner` (only `?op=closure`) with
  `aria-live="off"` per F7 — still asserted by
  `closure_banner_renders_only_on_closure_op` in `tests/operations.rs`.
- **Op-selector ARIA (M4 — Grupo A authoritative).**
  `partials/hts-op-selector.html` is a `<nav aria-label>` with plain
  `<ul>/<li>/<a>` links; the active entry carries
  `aria-current="page"`. It is **not** a `role="tablist"` /
  `role="tab"` surface. The `[role="tablist"]` contract is reserved
  for the resource-family strip (§7.6 F5) — the only nested
  tabpanel-bearing structure on the page. Test consequence: Playwright
  must query with `getByRole('link')` / `getByRole('navigation')` and
  never `getByRole('tab')`; any E1-era assertion relying on tab roles
  for this strip is superseded.
- **Wrapper-vs-form id contract (F15 + M5 — Grupo A).**
  `partials/hts-op-input.html` is a plain
  `<div class="hts-op-workbench__input-wrap">` that **does not**
  duplicate `id="hts-workbench-input"`. The inner per-op `<form>`
  carries that id and is the outerHTML swap target for op-selector
  clicks. Rationale (in the dispatcher's header comment): a duplicate
  id trips Playwright strict-mode locators and gives htmx an ambiguous
  target on the initial swap. This is the current realization of F15;
  any partial that re-introduces the wrapper id is a regression.
- **Batch skeleton swap-race (M3, `operations.spec.ts:531`).** The
  skeleton `<tr>` in `hts-vs-batch-table.html` emits
  `hx-trigger="load"`, which htmx fires immediately on insertion. The
  per-row endpoint returns in ~10–60 ms because the fan-out tasks
  complete while the seed HTML is still being rendered, and
  `hx-swap="outerHTML"` replaces the skeleton with the completed-row
  partial (same `id`, no `hx-*`) inside a window Playwright's ~100 ms
  locator poll cannot outrun. Contract: **the seed response body IS
  the skeleton contract, not the live DOM.** Playwright asserts on the
  response body captured via `page.waitForResponse(...)` and regex
  (`operations.spec.ts:561-602`); the Rust ring
  `batch_seed_returns_n_skeleton_rows` still asserts on the rendered
  DOM (no htmx layer, deterministic). Deep rationale in
  `edson/docs/hts-ui-ops531-diagnosis.md`.
- **F3 scope-wrapper deferral (D11 — folded from the E1 stub).**
  Detail pages (`cs-detail.html`, `vs-detail.html`, `cm-detail.html`)
  still embed the Slice B/C/D input partials directly with the
  resource pinned inline as prose; the outer
  `<fieldset name="scope">` wrapper is realized only inside the
  standalone workbench partials (`hts-op-lookup-input.html`,
  `hts-op-validate-cs-input.html`, `hts-op-subsumes-input.html`,
  `hts-op-expand-input.html`, `hts-op-translate-input.html`,
  `hts-vs-validate-input.html`, `hts-cm-closure-input.html`,
  `hts-vs-batch-input.html`). The deferral is accurate but belongs
  here as a current-state note rather than as Slice E1 backlog — no
  successor slice is scheduled; a shared `WorkbenchScope` partial
  will land only when a second detail-page caller demands it.
- **Metadata workbench slot (M7 cross-ref).** The operations shell
  does not carry a metadata slot of its own; the CS / VS / CM
  metadata-workbench treatment lives in §7.3 (CS), §7.4 (VS), and
  §7.5 (CM). This bullet exists so the Phase 3 sync M7 label resolves
  from a single place — see those sections for the authoritative
  surface.
- **Threshold panel + VS Validate three-way source (unchanged since
  E1).** `OpsFlags::show_advanced_panel` still gates the Advanced
  `<details>` for `?op=expand` only (asserted by
  `threshold_panel_hidden_for_non_expand_ops`). The VS Validate
  three-way source selector (canonical / instance / inline) is now
  driven by the real `run_vs_validate_code` branch inside
  `run_validate_code`: `POST /ValueSet/{id}/$validate-code` for
  `Instance`, `POST /ValueSet/$validate-code` with `url=` /
  `valueSet=` for `Canonical` / `Inline`.
- **Environment invariant (Windows `reqwest` proxy).** Unchanged from
  E1 and still required. Corporate `HTTP_PROXY` / `HTTPS_PROXY` env
  vars route loopback traffic through an off-VPN proxy, causing
  `start_mock`-based tests to fail with "Could not reach the
  terminology server" panics that look like `axum::serve` flakiness.
  Fix: set `NO_PROXY=127.0.0.1,localhost` (or clear the proxy vars)
  before running `cargo test -p helios-hts-ui`. Aligned with the
  `corporate-proxy-bypass.mdc` rule in `~/.cursor/rules`.
- **Fluent-key parity (F8 + F17).** Post-Phase-3, per-locale key counts
  stay in parity across `en` / `es` / `de` under the same namespace
  discipline: new keys land under `hts-operations-*`,
  `hts-cm-closure-*`, `hts-vs-validate-*`, `hts-vs-batch-*`, plus small
  extensions to existing CS Lookup, CS Validate, and VS Expand
  namespaces. The E1-era 26-duplicate-key transient is history; the
  standing invariant is that no `hts-operations-{op}-*` key shadows a
  resource-scoped key already shipped by Slices B/C/D (F17).
- **Playwright ops parity (F16).** `e2e/tests/operations.spec.ts` runs
  green at 75 passed / 0 failed / 3 skipped. The three skips document
  design-vs-implementation gaps (e.g. non-empty-invalid-JSON inline
  pass-through in VS `$validate-code`) flagged in
  `edson/docs/hts-ui-phase3a-operations-output.md`, not regressions.
  No new seed identifiers were required — the free-scope inputs accept
  ad-hoc `system` + `code` values, matching the E1 note.
```

### Concerns / open questions for parent adjudication

1. **Heading level choice** — I kept the block as a bold-paragraph opener
   (`**Slice E1 + E2 shipping notes (post-Phase-3).**`) instead of promoting
   it to `#### 7.6.1a` as the brief's structure suggestion proposed. Reason:
   the surviving `**Slice E test hooks (F16 triage)**` block at L1704 is
   also a bold paragraph, and promoting the E1+E2 notes to an H4 would
   silently make the test-hooks block a subsection of it. Preserving the
   two-peer bold-paragraph structure of the current §7.6.1 keeps the doc
   TOC unchanged. If the parent prefers the H4 heading, just swap the
   opener line — no other change needed.
2. **Referenced doc not on disk** —
   `edson/docs/hts-ui-phase3a-operations-output.md` is referenced from
   `operations.spec.ts` (L508 and L544) and I preserved that reference in
   the "Playwright ops parity" bullet. It does not currently exist on
   disk (globbed `edson/docs/hts-ui-phase3*.md` → 0 matches). If the
   parent knows this file lives at a different path (or is still to be
   created), swap the reference; otherwise the anchor still matches what
   the test comments already point to.
3. **Grupo A diagnosis doc not on disk** — the brief mentioned
   `edson/docs/hts-ui-grupo-a-diagnosis.md` as an optional consult; it
   was not on disk (glob → 0 matches). The M4 and M5 bullets therefore
   cite the source-of-truth files (`hts-op-selector.html` and
   `hts-op-input.html`) instead of the diagnosis doc — the two partials'
   own header comments are the authoritative rationale.
4. **Adjacent stale block at L1704-L1722** — the "Slice E test hooks
   (F16 triage)" bold paragraph plus its 4 numbered test hooks describe
   the E2 test scope in the future tense ("Four dedicated `#[tokio::test]`
   additions live in a new `tests/operations.rs`"). Those tests DID land
   (see `crates/hts-ui/tests/operations.rs` and `operations_e2.rs`), so
   this block is also mildly stale. Out of scope for this edit (the brief
   caps at L1702), but worth flagging as a natural follow-up sync target
   for the same pass. Fixing that block alongside would let §7.6.1 close
   fully in past-tense reality.
5. **F1 = D notation** — kept the "F1 = D" convention (option-D of the
   F1 triage decision matrix) exactly as used at L1545 and elsewhere in
   §7.6.1. No changes to the notation semantics.
6. **Line-range verification** — verified twice by re-reading
   L1615-L1710 in `hts-ui-design.md`. The block is 84 lines (L1619-L1702
   inclusive). L1618 (blank) is not part of the replacement; L1703
   (blank) is not part of the replacement. The `StrReplace` `old_string`
   above starts at the `**Slice E1 shipping notes` line and ends at the
   closing `system + code).` with no trailing newline sensitivity beyond
   the last line's own line terminator.
7. **New anchors are safe additions** — `M3`, `M4`, `M5`, `M7`, and `D11`
   do not appear anywhere else in `hts-ui-design.md` (verified via grep
   for `^(M[0-9]+|D[0-9]+|Grupo A)\b` → no matches). Introducing them
   here as bullet-tag anchors does not collide with any existing symbol.

### Sanity check checklist for the parent before locking the edit

- [ ] Re-grep `F1|F3|F4|F6|F7|F8|F10|F12|F15|F16|F17` in `hts-ui-design.md`
      after applying — count MUST be strictly greater than or equal to the
      pre-edit count for each anchor.
- [ ] Confirm the L1704 `**Slice E test hooks (F16 triage)**` block still
      renders as a peer of the rewritten block (no accidental heading
      demotion).
- [ ] Optional: pair this edit with a follow-up rewrite of L1704-L1722 to
      close §7.6.1 fully in past-tense.
