# Sub-Detail output — HTS UI design sync (§7.7 Import + §7.9 Diagnostics)

**Scope.** Six edits to `edson/docs/hts-ui-design.md` covering §7.7 (Import)
and §7.9 (Diagnostics), including two new Slice-notes subsections (§7.7.1
Slice F, §7.9.1 Slice G). This file is a **draft output**: it captures the
verbatim `old_string` / `new_string` payloads plus rationale so a follow-up
edit pass can apply them cleanly against `hts-ui-design.md` without
re-reading every source. The design doc itself was **not** modified by this
sub-detail (read-only per task contract).

**Anchor context.**

- `hts-ui-design.md` §7.7 spans L1724–L1758 (Import), §7.8 header is at L1760.
- `hts-ui-design.md` §7.9 spans L1798–L1829 (Diagnostics), §7.10 header is at L1831.
- Existing Slice-notes precedent: `#### 7.3.1` (L856), `#### 7.4.1` (L1025),
  `#### 7.5.1` (L1144), `#### 7.6.1` (L1515). All use `####` heading level and
  slot inside their parent `### 7.X` block before the next `### 7.X+1` header.
- Real commit hashes verified via `git log`:
  - Slice F + G shipped together in `59a9b9fe3` — "feat(hts-ui): Slices F+G —
    Import and Diagnostics (#551)". The task prompt cites `0aaf22775`, which
    does not exist on this branch; the drafts below use the verified hash and
    flag the discrepancy under "Concerns" at the bottom.
  - Grupo B/C/D residuals (which shipped the `uptime_seconds: f64` fix, the
    Metadata swap-slot fix, and seed / spec drift patches) landed in
    `1949014c7` — "fix(hts-ui): Grupo B/C/D residuals — health f64, Metadata
    swap slot, seed + spec drift". The task prompt cites `8d56eac6a`, which
    also does not exist; drafts use the verified hash. See "Concerns" below.

---

## Edit 1: D7 — §7.7 Purpose states `500` should be `413`

**Rationale.** §7.10 row 7.7 (states matrix, L1847) already lists
`✓ 207 / 400 / 413` for the Import page. §7.7's Purpose bullet at L1727 is
the only place in the doc that still says `500` — a stale value from before
Slice F pinned the payload-size gate at 10 MB (`ImportStatus::TooLarge` in
`crates/hts-ui/src/import.rs` L108, cross-checked against
`import_post_413_renders_too_large_guidance` in
`crates/hts-ui/tests/import.rs`). `500` here would be actively misleading
because the `/import` transport 5xx path is folded into the shared
degraded-reason chain (`StatusView::from_error`, `import.rs` L139–L155),
not a distinct visible state.

**Old string (verbatim, includes the two-line bullet as it appears at L1726–L1727):**

```
- **Purpose** — POST a JSON Bundle to `/import`; show counts + non-fatal
  error list (200 / 207 / 400 / 500).
```

**New string:**

```
- **Purpose** — POST a JSON Bundle to `/import`; show counts + non-fatal
  error list (200 / 207 / 400 / 413). See §7.10 row 7.7 for the states
  matrix and §7.7.1 for the Slice F wire-shape decisions.
```

---

## Edit 2: D8 — §7.7 wireframe "paste + file" annotation (paste-only in v1)

**Rationale.** The wireframe at L1739–L1753 still shows `Source: (o) paste
( ) file`, implying two live upload paths. Slice F v1 wires **only** the
paste path — the `<input type="file">` renders for a11y symmetry with the
radio group but its value is ignored by `import_run` (`import.rs`
L207–L255 reads `bundle` from `parse_form`, never a file field). The
empty-bundle pre-flight gate (`import.rs` L227–L232) surfaces a stub-input
error via the same OperationOutcome partial that catches empty pastes, so
a nojs POST with the file radio selected still lands cleanly. The
wireframe is a design surface; the reality of "file input renders but is a
stub" belongs inside the wireframe block as a caveat, not silently in
§7.7.1 only. Full rationale lives in §7.7.1.

**Old string (verbatim, the wireframe fence closer + blank line + a11y bullet head):**

```
+--------------------------------------------------+
```

- **a11y** — status region `aria-live="polite"`; issue list uses `<details>`
```

**New string:**

```
+--------------------------------------------------+
```

> **Slice F v1 note.** The `Source: (o) paste  ( ) file` radio pair and the
> `<input type="file">` render for a11y symmetry with the paste flow, but
> Slice F v1 wires only the paste path end-to-end. A submit with the file
> radio selected falls through the empty-bundle pre-flight gate and renders
> the shared invalid-input OperationOutcome. Multipart wire-up is deferred
> to v1.5; see §7.7.1 for rationale.

- **a11y** — status region `aria-live="polite"`; issue list uses `<details>`
```

---

## Edit 3: M13 — new `#### 7.7.1 Slice F implementation notes`

**Rationale.** Mirrors the §7.3.1 / §7.4.1 / §7.5.1 / §7.6.1 pattern
established by earlier slices. Captures the paste-only v1 decision, the
UI-owned pre-flight validation contract, the four-state matrix
(200 / 207 / 400 / 413), the Playwright skip footprint, and the Rust-ring
coverage that keeps those skips honest. Format-aligned with §7.6.1
(bulleted decisions, backticked module paths, explicit test function names).

**Playwright-skip count clarification.** The task prompt says "3 Playwright
skips (import:214, import:259)" — the file actually has **2** skips (both
line numbers cited). Draft reflects the two-skip reality; see "Concerns".

**Anchor.** Insert the new `#### 7.7.1` block **immediately after the last
bullet of §7.7** (`- **i18n** — hts-import-*.` at L1758) and **immediately
before the `### 7.8 Bootstrap ledger` header** at L1760.

**Old string (verbatim, tail of §7.7 + blank line + §7.8 header):**

```
- **Exemplar** — `crates/ui` bulk-import polling patterns.
- **i18n** — `hts-import-*`.

### 7.8 Bootstrap ledger — `/ui/hts/bootstrap` (v1.5)
```

**New string:**

```
- **Exemplar** — `crates/ui` bulk-import polling patterns.
- **i18n** — `hts-import-*`.

#### 7.7.1 Slice F implementation notes

Slice F inherits the invariants pinned in §7.3.1 (Slice B) through §7.6.1
(Slice E) — the `_count` clamp shape, the `form_urlencoded` multi-map, the
100 ms / 250 ms test-only timeout pair, canonical URL resolution at page
render, 404 → `OperationOutcome`-in-shell, the merged route-enum matrix
walker, the `HTS_UI_MAX_EXPANSION_SIZE_HINT` ceiling, the tree/flat
parameter mapping, the mock ready-probe pattern, and the first-match-wins
mapping-kind rule. Slice-F-specific decisions:

- **Paste-only v1 (deferred file upload).** The design lists two Source
  radios and `<input type="file">` alongside the textarea, and the
  wireframe still renders both — but Slice F v1 wires only the paste path
  through `POST /hts/import` (`crates/hts-ui/src/import.rs` L207–L255).
  File / multipart plumbing (drag-and-drop UX, browser-side size
  pre-check, and the size negotiation that pairs with the 413 arm) inflates
  the diff without adding coverage the paste path does not already
  exercise. The stub input still renders for a11y symmetry with the
  radio group; a POST with the file radio selected falls through the
  empty-bundle pre-flight gate and lands on the shared invalid-input
  OperationOutcome — see the pre-flight bullet below. File upload is
  scheduled for **v1.5**; see the `# TODO(F): file input follow-up`
  marker at `crates/hts-ui/src/import.rs` L15–L19.
- **States matrix (four arms).** `StatusView` (`import.rs` L72–L160)
  discriminates via four booleans that Askama branches on directly, per
  the Slice E1 `OpsFlags` idiom:
  - `is_success` → HTTP 200. Green summary strip with per-resource counts
    (`counts_code_systems` / `counts_value_sets` / `counts_concept_maps`
    / `counts_concepts`), echoing the returned Bundle shape.
  - `is_partial` → HTTP 207 `PartialSuccess`. Amber banner with a
    `<details>` issue expander; the plural-selected heading reads
    "N issues" and each entry renders through the shared
    `hts-outcome.html` partial. Counts remain populated where HTS
    reports them; missing counts render as `—`.
  - `is_rejected` → HTTP 400 (or a pre-flight gate). Renders the shared
    `hts-outcome.html` inside `hts-import-status--error`. The Rust ring
    pins the class stack (`hts-import-status hts-import-status--error` +
    `hts-outcome hts-outcome--error`) so template refactors that drop
    either marker must land alongside a matched Playwright edit.
  - `is_too_large` → HTTP 413. Amber `hts-import-status--warn` plus the
    Fluent `hts-import-too-large-hint` copy pointing at the "split the
    Bundle" guidance. The 10 MB ceiling is enforced upstream by HTS and
    surfaced by `ImportStatus::TooLarge` — the UI does not pre-check
    size on the paste path (browser POST would already be through the
    wire by the time the ceiling is knowable in v1).
  - **Transport 5xx / connect / timeout.** Not a fifth `is_*` arm.
    `StatusView::from_error` (`import.rs` L139–L155) sets
    `degraded_reason` and reuses the shared degraded partial inside the
    status region; the four discriminator booleans stay `false`. This
    is the reason `500` was **removed** from the §7.7 Purpose bullet in
    D7 — 500 is not a distinct visible state on this page.
- **Pre-flight validation is UI-owned.** Two gates fire before the HTS
  round-trip (`import.rs` L223–L242):
  1. **Empty bundle** (`bundle.trim().is_empty()`) → synthesize
     `OutcomeView::invalid_input(hts-import-empty-bundle-error)`. This is
     also the arm the (currently-stub) file radio falls through when the
     `<input type="file">` value is ignored by v1.
  2. **Invalid JSON** (`serde_json::from_str::<serde_json::Value>` fails)
     → synthesize `OutcomeView::invalid_input(hts-import-invalid-json-error)`
     with a different diagnostic so the operator can tell empty from
     malformed without opening the network tab.
  Both gates render the same rejected-status shape (`is_rejected = true`)
  and the submit button re-enables after the error banner renders — no
  page reload is required, and no HTTP request reaches HTS.
- **Degraded probe (shell only, POST does not re-probe).** `import_page`
  (`import.rs` L164–L196) runs `probe_degraded` on the initial GET;
  a failed `/health` renders the shared `hts-degraded.html` above the
  form and disables the submit button (§7 preamble). The POST handler
  intentionally does **not** re-probe — if the round-trip fails,
  `from_error.degraded_reason` renders inside the status region instead
  (so a mid-submit degradation is legible without blanking the shell).
- **Playwright skips (two, both intentional).** The e2e ring at
  `crates/hts-ui/e2e/tests/import.spec.ts` skips two arms explicitly:
  - `import.spec.ts:214` — the 207 `PartialSuccess` amber arm. The
    Playwright suite boots a real `hts` binary via `e2e/boot.mjs` and
    that binary will not emit 207 on demand without a seeded ValueSet /
    ConceptMap topology that Slice F does not ship. Covered end-to-end
    by the Rust ring's canned mock in `crates/hts-ui/tests/import.rs`
    (`import_post_207_renders_partial_success_with_issue_list`), which
    asserts the class marker, the Fluent title, the `<details>` issue
    expander, and the plural-selected heading.
  - `import.spec.ts:259` — the 413 `TooLarge` arm. A 13 MB paste is
    impractical over Playwright's default Chromium input path (browser
    process memory + WS frame pressure + the fact that the Playwright
    `webServer` runs on the same box as the browser). Covered end-to-end
    by the Rust ring's canned response in
    `import_post_413_renders_too_large_guidance`, which asserts both the
    Fluent title ("Bundle too large") and the split-the-Bundle hint.
- **Rust ring covers all four visible arms.** `crates/hts-ui/tests/import.rs`
  ships `import_post_200_renders_success_summary`,
  `import_post_207_renders_partial_success_with_issue_list`,
  `import_post_400_renders_outcome_partial`, and
  `import_post_413_renders_too_large_guidance`. Any template refactor
  that alters the discriminator class stack must land alongside matched
  edits in these four tests.
- **Slice F shipped in `59a9b9fe3`** (feat(hts-ui): Slices F+G — Import
  and Diagnostics, PR #551). The `uptime_seconds: f64` fix that
  restored the shell's degraded-probe accuracy — and therefore kept the
  Import submit button correctly enabled — shipped separately in
  `1949014c7` (Grupo B/C/D residuals). See §7.9.1 for the health
  typing rationale.

### 7.8 Bootstrap ledger — `/ui/hts/bootstrap` (v1.5)
```

---

## Edit 4: M6 — `UpstreamHealth.uptime_seconds` is `f64`, not `u64`

**Rationale.** §7.9 does not currently carry any note about the shape of
`UpstreamHealth`; the Grupo B fix silently changed the field to `f64` and
the design doc still implies (via the pre-fix status of the codebase and
via §7.1 dashboard prose) that uptime is a whole-second count. Real HTS
emits fractional seconds through `helios_observability::uptime`
(evidenced by the freeze-in-time regression test
`health_deserializes_fractional_uptime_seconds` at
`crates/hts-ui/src/upstream.rs` L3881–L3892, which sends
`"uptime_seconds":0.218212` and asserts `f64` decode). The comment block
around the field itself (`upstream.rs` L173–L179) explains that a `u64`
typing would fail decode on any freshly-started upstream, cascade
`degraded_reason=upstream-shape`, and disable the Import submit button —
which is exactly the Grupo B regression. `uptime_pretty()` (`upstream.rs`
L292–L311) floors to whole seconds for display, so no localised uptime
copy needs to change.

Add the note as a new bullet between §7.9's `- **States**` bullet and
`- **Wireframe**` header. Anchor is unique (the `- **States**` bullet
copy is verbatim only inside §7.9).

**Old string (verbatim, L1806–L1809 of `hts-ui-design.md`):**

```
- **States** — Any of the four sources may 5xx independently; a tab that
  fails renders an OperationOutcome partial inside the same `#diag-panel`
  without disturbing the other tabs.
- **Wireframe**
```

**New string:**

```
- **States** — Any of the four sources may 5xx independently; a tab that
  fails renders an OperationOutcome partial inside the same `#diag-panel`
  without disturbing the other tabs.
- **`/health` typing (Grupo B fix).** `UpstreamHealth.uptime_seconds` is
  deserialized as `f64` — HTS emits a fractional second count from
  `helios_observability::uptime` (see `crates/hts/src/operations/health.rs`
  and the regression test `health_deserializes_fractional_uptime_seconds`
  at `crates/hts-ui/src/upstream.rs` L3881–L3892). A `u64` typing (as
  earlier revisions of this doc implied) fails JSON decode on any
  couple-seconds-old server (`uptime_seconds: 0.2`), sets
  `degraded_reason = "upstream-shape"` on `UpstreamError::Decode`, and
  cascades into the Import shell rendering as degraded and the submit
  button rendering as disabled. `UpstreamHealth::uptime_pretty()` floors
  to whole seconds for display, so no locale copy changes are required.
  See `crates/hts-ui/src/upstream.rs` L164–L180 (struct + field comment)
  and L292–L311 (`uptime_pretty`). Fix shipped in `1949014c7`.
- **Wireframe**
```

---

## Edit 5: M14 — new `#### 7.9.1 Slice G implementation notes` (includes M15 skip footnote)

**Rationale.** Mirrors §7.3.1–§7.6.1 and the new §7.7.1 pattern. Locks
in the four tab slugs, the `hx-push-url="true"` deep-link contract, the
per-tab error-isolation contract, the `/metrics` Prometheus passthrough
decision, and the single intentional Playwright skip at
`diagnostics.spec.ts:268` (M15 — a Playwright browser cannot force
`/health` or `/metrics` to 5xx against a real HTS upstream). The M15
skip footnote is folded into this subsection rather than living as a
separate §11.2 cross-ref, both to keep Slice G's rationale in one place
and because the shared structural invariant it depends on (every tab
targets only `#diag-panel`) is exercised by the immediately-adjacent
in-file Playwright test at `diagnostics.spec.ts:231`.

**Anchor.** Insert the new `#### 7.9.1` block **immediately after the
last bullet of §7.9** (`- **i18n** — hts-diagnostics-*.` at L1829) and
**immediately before the `### 7.10 States matrix` header** at L1831.

**Old string (verbatim, tail of §7.9 + blank line + §7.10 header):**

```
- **a11y** — tabs implemented as `<a role="tab">` with `aria-selected` and
  a single `role="tabpanel"` container.
- **i18n** — `hts-diagnostics-*`.

### 7.10 States matrix (per page × per state)
```

**New string:**

```
- **a11y** — tabs implemented as `<a role="tab">` with `aria-selected` and
  a single `role="tabpanel"` container.
- **i18n** — `hts-diagnostics-*`.

#### 7.9.1 Slice G implementation notes

Slice G inherits the invariants pinned in §7.3.1 (Slice B) through §7.6.1
(Slice E) plus the Slice F pre-flight-gate + degraded-probe pattern from
§7.7.1. Slice-G-specific decisions:

- **Four tab slugs (URL contract).** The `?tab=` query parameter is the
  authoritative selector, both for hard-nav and for the htmx-driven
  panel swap. Legal values, exactly as emitted by `Tab::slug` in
  `crates/hts-ui/src/diagnostics.rs` L69–L76:
  - `capability` — CapabilityStatement view (default when `?tab=` is
    missing or unrecognized; `Tab::from_slug` collapses everything else
    to `Capability`).
  - `terminology-capabilities` — `TerminologyCapabilities` view. The
    hyphenated slug is deliberate: it matches the `tab_label_key`
    Fluent key (`hts-diagnostics-tab-terminology-capabilities`) and
    keeps the URL parseable without a query-string escape.
  - `health` — `/health` JSON panel. Renders `UpstreamHealth` via
    `uptime_pretty()` (see Edit 4 for the `f64` typing rationale).
  - `metrics` — `/metrics` Prometheus text panel; see the passthrough
    bullet below.
- **`hx-push-url="true"` deep-link contract.** Every tab anchor carries
  `hx-get="/ui/hts/diagnostics/panel?tab={slug}"` +
  `hx-target="#diag-panel"` + `hx-swap="innerHTML"` +
  `hx-push-url="true"` (see `templates/pages/diagnostics.html`
  L47–L50). Clicking a tab swaps only the panel body but *also* pushes
  `/ui/hts/diagnostics?tab={slug}` into the browser history — so tabs
  are shareable, back / forward navigation works, and the nojs
  fallback URL (via the tab's real `href`) resolves to the same view.
  Structural invariant asserted by `diagnostics.spec.ts:231` (each
  tab's `hx-target` = `#diag-panel`, `hx-swap` = `innerHTML`, and
  `hx-get` / `href` both include the matching slug).
- **Per-tab error isolation.** A 5xx / connect / decode / not-found on
  one tab's upstream call renders `partials/hts-outcome.html` **inside**
  `#diag-panel` and nowhere else. The tab strip itself is untouched:
  the three other tabs remain clickable and keep their `aria-selected`
  state, so operators can navigate off the failing surface without a
  page reload. The implementation lives in `build_panel`
  (`diagnostics.rs` L178–L201) — each `Tab::*` branch converts a
  transport `UpstreamError` into an `OutcomeView` via
  `outcome_from_error` (L212–L224) and stashes it on
  `PanelView.outcome`; the tab strip is rendered from a separate
  `tab_entries` call (`diagnostics.rs` L123–L137) that never sees the
  outcome. Contract explicitly asserted by the Rust integration test
  `any_tab_5xx_renders_outcome_in_diag_panel_only` in
  `crates/hts-ui/tests/diagnostics.rs`, which seeds a 500 on `/health`
  through an in-process axum mock and asserts that `hts-outcome.html`
  renders inside `#diag-panel` while the three other tab id markers
  survive in the shell.
- **Shell-level degraded probe runs once on GET, panel route
  deliberately does not.** `diagnostics_page` (`diagnostics.rs`
  L228–L257) runs `probe_degraded` before building the panel; the
  panel route `diagnostics_panel` (L261–L276) does not. This is
  intentional so an htmx-driven tab swap cannot blank the shell
  (a shell-level degraded banner would replace the tab strip that the
  operator just clicked). Panel-level failure surfaces via the
  per-tab outcome bullet above instead.
- **`/metrics` Prometheus passthrough (raw text, no parse).** The
  `metrics` tab renders the upstream `/metrics` response as
  `<pre>`-wrapped raw Prometheus text with no re-parse and no chart
  (`PanelView.metrics: Option<String>` at `diagnostics.rs` L157–L160;
  `Some("")` renders the neutral `hts-diagnostics-metrics-empty`
  copy). Rationale: Prometheus text format is already the operator-
  facing wire format for every metrics tool downstream, HTS may add
  new metric names between releases without a UI change, and a chart
  layer would need a client-side JS dependency that violates the
  vendored-only rule from §1 (no runtime CDN, no new browser JSON
  API). Operators who need charts point Grafana / Prometheus at
  `HTS_UI_UPSTREAM_URL/metrics` directly.
- **Playwright skip (one, intentional).**
  `crates/hts-ui/e2e/tests/diagnostics.spec.ts:268` is a
  `test.skip` that documents the reason the 5xx-isolation contract
  cannot be exercised from a browser: the Playwright suite boots a
  real `hts` binary via `e2e/boot.mjs`, and there is no way from the
  browser to force `/metadata`, `/health`, or `/metrics` to fail —
  HTS is its own upstream for those endpoints and stays up while the
  suite is running. The contract is covered end-to-end by the Rust
  integration test cited above
  (`any_tab_5xx_renders_outcome_in_diag_panel_only`); the adjacent
  `diagnostics.spec.ts:231` test locks down the structural invariant
  (`#diag-panel`-only tab targets) that makes the outcome-render
  path safe under all four tabs.
- **Slice G shipped in `59a9b9fe3`** (feat(hts-ui): Slices F+G —
  Import and Diagnostics, PR #551), alongside Slice F. The Grupo B
  `uptime_seconds: f64` follow-up that stabilised the `health` tab
  and its cascade into the Import shell shipped in `1949014c7`; see
  Edit 4 above.

### 7.10 States matrix (per page × per state)
```

---

## Concerns

1. **Commit-hash mismatch with task prompt.** The prompt cites
   `0aaf22775` for Slice F+G and `8d56eac6a` for the Grupo B `uptime_seconds`
   fix. Neither hash exists on this branch. The real hashes verified via
   `git log --oneline --all`:
   - `59a9b9fe3` — "feat(hts-ui): Slices F+G — Import and Diagnostics (#551)"
   - `1949014c7` — "fix(hts-ui): Grupo B/C/D residuals — health f64, Metadata
     swap slot, seed + spec drift (+13)"
   Drafts use the verified hashes. If the prompt hashes are what the design
   doc must cite (e.g. because they refer to a different history rewrite),
   the four `59a9b9fe3` and two `1949014c7` occurrences in Edits 3–5 need a
   substitution pass before landing.
2. **Playwright-skip count in the prompt.** The prompt says "3 Playwright
   skips (import:214, import:259)" but the file actually contains **2**
   `test.skip` blocks (both cited line numbers, verified via ripgrep). The
   §7.7.1 draft documents two skips. If the prompt intended a third pending
   skip that is not yet in `import.spec.ts`, it needs to be identified and
   added to the draft.
3. **Wireframe-annotation style choice (Edit 2).** The draft inserts a
   `> Slice F v1 note` blockquote **inside** the `### 7.7` prose (between
   the closing wireframe fence and the a11y bullet) rather than editing the
   ASCII wireframe itself. Editing the wireframe (`(o) paste  ( ) file` →
   `(o) paste  ( ) file  [v1: paste only]`) would break the fixed-width
   ASCII alignment; the blockquote keeps the wireframe pristine and lets
   §7.7.1 hold the full rationale. If reviewers prefer the note to live
   only in §7.7.1 (no wireframe-adjacent hint), Edit 2 can be dropped —
   Edit 3 already carries the paste-only-v1 information.
4. **Edit 4 anchor placement.** The Grupo B typing note is inserted as a
   new bullet **between `- **States**` and `- **Wireframe**`** in §7.9.
   Alternative: fold the note into §7.9.1 (Edit 5) only. Split into a §7.9
   bullet **and** the §7.9.1 rationale intentionally, mirroring §7.7's
   short Purpose bullet + long §7.7.1 pattern — the top-level bullet
   surfaces the fact for anyone scanning §7.9 without reading the Slice
   notes, and §7.9.1 supplies the "why this cascaded into Import" context.
5. **Two live edits touch §7.7 (Edits 1 + 2), two live edits touch §7.9
   (Edits 4 + 5), and two new subsections are appended (Edits 3 + 5).**
   Total: **six edits, three files' worth of `hts-ui-design.md` text
   touched, zero writes to code**. No source files under
   `crates/hts-ui/**` were modified by this sub-detail — the drafts only
   cite them.
