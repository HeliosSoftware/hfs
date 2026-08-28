# HTS UI improvement — plan of record

**Deliverable:** `edson/docs/hts-ui-improvement-plan.md` (+ `edson/docs/screens/`)
**Status:** Stages 1-6 SHIPPED. **167 tests passing** on the plain `cargo` default, verified live
against the official seed set. Two items deferred by the user (the WCAG contrast fix, and Home
V3's two data tables — see Open decisions 1 and 3); the toolchain item is resolved. Stage 7
Playwright verification remains blocked by a pre-existing harness bug.
**Decisions recorded 2026-08-27.** Nine page layouts approved from the rendered variants
(improvement-plan §4); concept-plane IA = **B — Concept-first**. **Diagnostics — and only
Diagnostics — mirrors HFS's Capability page** (§3). **CSS: 1 line added, 2 deleted.**
**Scope change 2026-08-27:** Operations page and Home Quick links **removed**. Nine pages remain.
**Scope change 2026-08-27:** Home V3's "Loaded content" and "Recent imports" tables **deferred** —
they need a team-approved design and explicit sign-off, and neither is built. Analysis and
implementation steps for both are in improvement-plan **§14**.
**Last updated:** 2026-08-26 · re-verified after `main` merge `8af7c5351`

---

## The problem, in one number

`crates/hts-ui` shares `crates/ui/assets/app.css` byte-for-byte
([lib.rs:86](../../crates/hts-ui/src/lib.rs#L86)) — but **206 of the 290 classes its templates emit
(71%) have no rule in that file.** Of 202 `hts-*` classes, exactly two are styled. Those 206
render at browser default.

So "improve the UI without changing the look and feel" is a **mapping exercise**, not a
restyle: point the markup at primitives that already exist.

## Constraints (hard)

- No CSS token, font, colour, radius or shadow changes. Askama + htmx only — no SPA, no
  bundler, no npm browser dependency, no off-origin request.
- Mappings and subsumption are shown as **information** (tables, labelled rows), never a graph.
- No new HTS backend routes, and no change to HTS terminology, storage or operation logic.
  **Held.** The one candidate exception — an inventory endpoint for Home V3's two tables — was
  analysed and then **deferred** rather than taken (improvement-plan §14), so this constraint
  survives the plan intact. The only edit to `crates/hts` in this branch is **3 lines** in
  `server.rs`, passing the new `metrics_ring` field when constructing `HtsUiState`; it adds no
  route and touches no backend behaviour.
- Every string is a Fluent key in `locales/{en,es,de}` — a test fails the build on drift.
- **Claude Design app work uses the Fable model** (the app's `Model` dropdown; client-side).

---

## Stages

| # | Stage | State |
|---|---|---|
| 1 | Diagnosis — class inventory, backend verification | **done** |
| 2 | Capture current state against official seeds | **done** — 24 shots |
| 3 | "Helios HFS" design system → Claude Design | **done** — 21 files, 17 cards |
| 4 | **3 variants per page + 3 IA directions** | **done** — 27 variants + 3 directions (Operations cut) |
| 5 | Page decisions + IA direction | **done** — 9 layouts approved, IA = B, Diagnostics mirrors HFS |
| 6 | Implementation — all slices | **done** — 167 tests passing |
| 7 | Verification | Rust ring **green**; a11y gate **added and run** (1 real violation found); Playwright suite blocked by a pre-existing harness bug |

### Stage 2 — capture (done)

Official seed set, not fixtures: `HTS_BOOTSTRAP_DIR=./crates/hts/terminology-data` (151 MB,
17 files) → **1,975 code systems loaded**. Real seeds change real design decisions that
fixtures hide: long canonical URLs blow out columns, ICD-10-CM's ~100k concepts genuinely hit
the `too-costly` ceiling, VSAC ships thousands of ValueSets.
e2e fixtures stay the deterministic basis for the automated test rings.

### Stage 3 — design system (done)

`DesignSync` → project **"Helios HFS"**, 21 files / 17 component cards authored from the real
`app.css`. Selecting it in the app's `Design system` dropdown makes generated mockups on-brand
by construction. The `/design-sync` skill was unusable (it bundles React; HFS has no
`package.json` and no Storybook) — hand-authored `@dsCard` previews are the supported path.

### Stage 4 — variants (done)

**3 variants for every page**, all using only existing `app.css` classes. Variants differ in
**layout and information density only** — never aesthetics.

| Page group | V1 | V2 | V3 |
|---|---|---|---|
| Browsers (CS/VS/CM) | Rail (today's shape) | **Top strip ← chosen** | Split preview pane |
| Details (CS/VS/CM) | Stacked | Sidebar facts | **Compact header ← chosen** |
| Home | Tiles + chart | Chart-first | **Ops console ← chosen**, tables deferred → ships as V1/V2 |
| Import | Stacked | Two-column | **Stepped ← chosen** |
| Diagnostics | Tabs | Stacked cards | Two-column | → **mirrors HFS Capability page** |
| **Concept plane** | A Workbench-first | **B Concept-first ← chosen** | C Two-pane |

27 page variants + 3 IA directions, ×2 themes — **delivered**. Five agents in parallel; assembled by
`gen-variants.mjs` over a shared `shell.mjs`; rendered with a **horizontal-overflow assertion**
per page and an **independent class check** against `app.css`.

### Stage 6 — implementation (two independent chains)

**Chain A — concept plane.** `GET /ui/hts/concepts?system=&code=&version=` (query, not path:
`system` is a canonical URI and a path segment needs double-encoded `%2F`). Identity renders
server-side; Mappings and Relations lazy-load via the existing `hx-trigger="load"` +
`hx-swap="outerHTML"` pattern from
[hts-vs-batch-table.html:28](../../crates/hts-ui/templates/partials/hts-vs-batch-table.html#L28).
Subsumption comparators are **derived from `parent`/`child`**, because that is the only way to
surface the stale-closure disagreement.

**Chain B — home chart.** Copy HFS's `build_chart` geometry (do NOT extract a shared crate —
`hts-ui-design.md` §9.0 defers that), guard the copy with a `chart_geometry_matches_hfs` test.
Plot **rate**, not cumulative. Exclude the page's own poll from the series. Ring buffer on
`HtsUiState`, fed by the `/metrics` fetch `HomeCards::fetch` already performs.

Then the class-map sweep across the remaining nine pages.

### Slice 1 — removals and cleanup (SHIPPED 2026-08-27)

| Removed | Detail |
|---|---|
| `src/operations.rs` | 11 routes; `mod`, `pub use BatchJobs`, and the `.merge()` dropped from `lib.rs` |
| `templates/pages/operations.html` | — |
| **18 partials** | every `hts-op-*`, `hts-vs-batch-*`, `hts-vs-validate-*`, `hts-cm-closure-*`, `hts-resource-family-tabs` — verified unreachable by a full reference graph, not by name |
| Sidebar entry | Operations nav-item; the Tools section survives on Import |
| Quick links | `.hts-quick-strip` section in `hts-home-cards.html` |
| `tests/operations_e1.rs`, `operations_e2.rs`, `e2e/tests/operations.spec.ts` | — |
| 9 `route_enum.rs` rows | the operations landing + 8 `operations/input` matrix rows |
| **153 dead Fluent keys × 3 locales** | unreferenced leftovers, many predating this slice (e.g. the catalog had `hts-cs-subsumes-codeA` while templates use `hts-cs-subsumes-code-a`). 491 → 338 `hts-*` keys, parity preserved |
| `.hts-quick-strip` / `.hts-quick-links` in `app.css` | **`app.css` now has zero HTS-specific rules** |

**Added:** one line — `.data-table tbody tr[aria-current="true"] td { background: var(--accent-soft); }`

**Accepted capability loss.** `$closure`, `$batch-validate-code` and ValueSet `$validate-code`
had no UI home outside the Operations page and are now **API-only**. Confirmed by the user on
2026-08-27 after being flagged twice. All three remain reachable over HTTP.

**Verified:** `cargo test -p helios-hts-ui` → **99 passing, 0 failing** (109 baseline − 10 in the
deleted operations test files). Live boot against the official seeds: the six remaining
`/ui/hts/*` routes return 200, `/ui/hts/operations` returns 404, and **no page leaks a raw
Fluent key** — checked across all nine surfaces.

### Stage 7 — verification

`cargo test -p helios-hts-ui` (baseline 80) · Playwright against fixtures (baseline 75/0/3) ·
**new `e2e/tests/a11y.spec.ts`** — `crates/ui/e2e` has an axe-core WCAG 2.2 AA gate in both
themes, `crates/hts-ui/e2e` has none, and this work touches contrast on every page · Fluent
parity · re-capture and diff · manual walk with JS disabled.

---

## Verified facts (no backend change needed)

- **Cross-map `$translate` works today.** `url` omitted → scans every stored ConceptMap. Proven
  live: `originMap=http://hl7.org/fhir/ConceptMap/sc-encounter-status|4.0.1`. Postgres matches.
- **`$subsumes(A00, A00.0)` → `subsumes`** on bootstrap-imported ICD-10-CM.
- **`originMap` is suppressed in reverse mode** — render em-dash + footnote, never a guess.
- **`source`/`target` translate params are parsed but never bound** — do not expose them.
- **`concept_hierarchy` survives re-import while `concept_closure` is wiped** — so the
  disagreement is renderable, not a degenerate empty state.
- **Every 404 carries an OperationOutcome**, but `Content-Type` is `application/json`, not
  `application/fhir+json` — do not gate the outcome partial on the FHIR type.
- **`Bundle.total` is the page size, not the match count.**

## Defects found by rendering (not by code review)

1. **`.pill` never resets `text-decoration`** — `<a class="pill">` renders underlined, live on
   the shipped home page. (`.btn` had the same bug; **the merge fixed it upstream**.)
2. **`svg { display: block }` is global** (app.css:140) — icon-beside-text breaks without a flex
   parent.
3. **`.chart-legend__type` has no rule** — the one unstyled class in HFS's own chart markup.
4. **`.content--wide` comes from `{% block content_class %}`** — unreachable from a page body.
5. **Content must sit inside `.pane`** — `body` is a `76px 1fr` grid.
6. **Selected table rows key on `aria-selected`, not `aria-current`** (app.css:2004) — the
   opposite of every other selected element in the codebase.
7. **`.addbox` is NOT a collapsible section** — it is the Add-tenant *dropdown*
   (`.addbox__panel` is `position: absolute`). Two agents caught this in my class map. There is
   **no inline-disclosure primitive** in app.css, and 13 `<details>` sites need one.
8. **`.notice` with no modifier has no background or border** — there is no `notice--ok`, so a
   success banner cannot look affirmative without one.

## CSS budget

**1 line added, 2 rules deleted.** The one addition is
`.data-table tbody tr[aria-current="true"] td { background: var(--accent-soft); }` — 3 sites in
the approved variants mark a selected row with `aria-current`, which paints nothing today.
Everything else budgeted earlier proved unnecessary: `.card-head` and `.kv-grid` are already
styled; disclosures use a bare `<details>` per HFS's own precedent (never `.addbox`, which is an
absolutely-positioned dropdown); and `.pill` goes away with the quick links. Deleting
`.hts-quick-strip` / `.hts-quick-links` leaves **zero HTS-specific rules in app.css**. · an inline-disclosure rule
(~5, genuinely earned — nothing exists) · `tr[aria-current]` row highlight (1) · shared heading
rule (~4) · facts wrapper (~3). The `$expand` tree needs **zero** — rendered as an indented
`.data-table`, which closed the one open question that wanted ~6 lines.

## Handoff

Claude Design owns the **decision** and the images. **Claude Code owns implementation** —
the app emits standalone HTML/CSS, the target is Askama with Fluent keys, htmx contracts,
nojs fallbacks and `HX-Request` fragment splits, none of which survive an export. `DesignSync`
reads design-system projects only; ordinary Design projects cannot be read back.

## Risks

- **#543 has landed.** `app.css` now opens `@layer tokens, base, components, pages;` and the
  duplicate blocks are consolidated — only `.notice` (1126 + 2108, both `components`) remains
  doubled. New rules must target the correct **layer**, not just the right file position.
- **`.content--wide` was removed.** Full-width pages now set `body.app-shell` +
  `.content--app`, which also switches the page to an internal-scroll model. Harness and all
  44 mockups were re-rendered against this.
- Editing `app.css` ships to both binaries — check each rule against the HFS pages that use the
  surrounding selectors.


---

## Stage 6 — what shipped (2026-08-27)

### Slice 3 — Diagnostics mirrors HFS's Capability page
Tab strip removed; **six stacked cards** in the old tabs' order (CapabilityStatement facts, REST
resources table, TerminologyCapabilities facts, Code Systems table, Health facts, Prometheus raw).
`Tab`/`TabQuery`/`TabEntry`/`PanelView` deleted, and `/ui/hts/diagnostics/panel` removed — with no
tabs there is no swap target, and HFS's capability page has no fragment endpoint either. Raw
payload is a **bare `<details>` + `<pre class="detail__code">`**, never `.addbox`. A guard test
scans every `class="…"` in both templates and asserts each has a rule in `app.css`.

**Incomplete — completed by Slice 7 (below).** The instruction was *"the same ui elements,
distributions, namings — even the left menu should have the same name and icon as hfs"*. This slice
delivered the elements and the distribution; the **naming was omitted**, because the slice was
scoped as a page-body task and the nav label lives in `layouts/base.html`.

### Slice 7 — Capability & Conformance: the naming, the route, and content parity (SHIPPED 2026-08-27)

Closes the gap Slice 3 left, plus two defects found while closing it.

| | Before | After |
|---|---|---|
| Sidebar label | `hts-nav-diagnostics` → "Diagnostics" | **HFS's own `nav-capability-conformance`** → "Capability & Conformance" |
| Page `<h1>` | `hts-diagnostics-heading` | **HFS's own `cap-title`** → "Capability Statement" |
| Route | `/ui/hts/diagnostics` | **`/ui/hts/capability-statement`**, old path 308s |
| Module / template | `diagnostics.rs`, `pages/diagnostics.html` + a partial | `capability.rs`, one `pages/capability-statement.html` (HFS has no partial either) |

**Keys are shared, not copied.** Both crates load `"../../locales"`, so the sidebar reuses HFS's
key rather than cloning the string — the two labels cannot drift. Non-`hts-` key count unchanged at
**693**; 28 dead `hts-diagnostics-*` / `hts-nav-diagnostics` keys retired, 16 `hts-capability-*`
added, ×3 locales.

**Cards: six, HFS's five plus one.** Server Summary (HFS's seven-field set), System Interactions
(*conditional* — HTS serves `POST /` but declares no `rest[].interaction[]`, so the card is omitted
rather than blank), Operations (7, from `rest[].operation[]`), Per-Resource Capabilities
(+ search-param count), **Terminology Capabilities**, and the raw statement.

**Removed, each because something already served it better:** Health (Home's status tile),
Prometheus raw (Home's chart; HFS folds the raw *CapabilityStatement* here instead, so that is what
this page now folds), and Code Systems — `supported_systems()` is `SELECT url FROM code_systems`,
the same table `/ui/hts/code-systems` browses with five columns instead of two, real paging instead
of a 50-row cap, and links into each detail page. Only the count survives, as a capability.

**Two defects found while doing it:**

1. **The TerminologyCapabilities card showed nothing new.** Raised by the user. It rendered `url`
   (always `—`, HTS emits none) plus `version`/`name`/`title`/`status` — **byte-identical** to the
   CapabilityStatement card above it. Rebuilt to show what the resource is actually for: expansion
   flags, the **12 `$expand` parameters** as `.tag` chips, validate-code / translation, and closure.
   `closure` is detected by **presence** (`{}` means supported); reading it as a boolean would have
   reported every server as unsupported. All flags are `Option` — a server that omits a block
   renders `—`, never a fabricated `false`.
2. **`nav-item--soon` on four shipped pages.** The "coming soon" modifier (`cursor: default`) was
   still on Value Sets, Concept Maps, Import *and* this page. All four removed.

**Judgment call, stated:** HFS pairs its resource table with a `filter-rail__search` form because it
lists ~150 types. HTS lists **three**. The filter is deliberately not ported — a search box over
three rows is noise, not parity. `searchInclude`/`searchRevInclude` are absent from HTS's metadata,
so those two columns are not ported either; a column of zeroes would read as a measurement.

**No CSS.** No backend change either — `crates/hts` is untouched by this slice; the 308 lives in
the UI router beside `home.rs`'s trailing-slash redirect.

### Slice 4 — Home request-rate chart
`metrics_ring.rs` (1440-sample ring on `HtsUiState`, `std::sync::RwLock`, guard never held across
an await) + `chart.rs` (HFS geometry copied, **not** extracted — §9.0 defers that; guarded by
`chart_geometry_matches_hfs`, which reads `crates/ui/src/lib.rs` and `pages/index.html` off disk).
Plots **requests per minute**, not cumulative. Clears the ring on `uptime_seconds` regression so a
restart renders as a **gap**, not a cliff to zero. Excludes `/ui/hts/home/cards` and `/metrics`, or
an idle server would plot a flat ~4 req/min — the chart measuring itself. Fed from the `/metrics`
fetch `HomeCards::fetch` already performs: **zero extra upstream requests**. The never-swapped
outer htmx wrapper is gone; `#hts-home-cards` now carries its own `hx-get` with no `hx-target`.

This is **all** of Home V3 that shipped. The "Loaded content" and "Recent imports" tables below the
chart are deferred (Open decisions 3); no backend work was done for them. The ring lives entirely
in `crates/hts-ui`, and the only `crates/hts` edit is the 3-line `HtsUiState` field wiring in
`server.rs`.

### Slice 5 — Concept plane (Direction B)
`GET /ui/hts/concepts?system=&code=&version=` plus `/identity`, `/mappings`, `/relations`. Query
not path, because `system` is a canonical URI. Identity renders server-side; Mappings and Relations
lazy-load. Mappings uses `$translate` with **`url` omitted** to scan every stored ConceptMap,
grouped by `originMap`. Subsumption derives comparators from `parent`/`child` — the only way to
surface the closure-vs-hierarchy disagreement — always calling with the ancestor as `codeA`, capped
at 20 with the dropped count stated. 24 new tests, 64 new Fluent keys × 3 locales.

### Slice 6 — axe-core a11y gate
`crates/hts-ui/e2e/tests/a11y.spec.ts`, 11 routes × 2 themes, mirroring the crates/ui gate. **It
found a real violation** (below). The gate was left at full strength — no rules disabled, no routes
dropped.

### Bug found and fixed — ConceptMap Origin column was always empty
`parse_translate_match` read `originMap` as `valueUri`; `crates/hts/src/operations/translate.rs`
emits **`valueCanonical`**. Reverse-mode `source` is a `valueCoding`, not a URI. The defect survived
because the Slice D test fixture *fabricated* `valueUri` — it validated against a payload HTS never
sends. Both spellings are now accepted and the fixture corrected. Proven before/after on a live
server: `Origin: —` → `Origin: http://hl7.org/fhir/ConceptMap/sc-encounter-status`.

---

### Slice 8 — chrome parity + Home V3 tiles (SHIPPED 2026-08-28)

Found by putting the deployed HTS and HFS side by side.

**The topbar had three divergences, all predating this branch.**

| | HFS | HTS before | HTS now |
|---|---|---|---|
| Language switcher | `?lang=` → cookie → `Accept-Language` → `en` | **already identical** | untouched |
| Theme toggle | `icons/sun.svg` / `moon.svg` | Unicode glyphs `&#9728;` / `&#9789;` | HFS's SVGs (2 icon files copied) |
| Avatar | `<span class="topbar__avatar">K</span>` | absent | present, same letter |
| "Dialect" chip | no such concept | a `<details>` HFS never had | **removed** |

The language selector needed **no change at all**: `crates/ui/src/i18n.rs` and
`crates/hts-ui/src/i18n.rs` are line-for-line equivalent apart from the cookie name, and this was
proven at runtime on both servers — `?lang=es` sets the cookie with identical attributes, a
cookie-only request persists the choice, and `Accept-Language: es-ES` negotiates to `es`.

**The dialect chip shipped non-functional and stayed that way.** Introduced in `03c8e9d4d` already
as a `<details>` holding a heading and a hint — no options, no form, no POST. The `hts_dialect`
cookie its comment promises **has never appeared in a Rust file in any commit**; the only `-S
dialect` hit under `src/` is "OpenMetrics *dialect*" in the Prometheus parser's docs. It looked
alive because it echoed the negotiated locale (`dialect: en` / `dialecto: es` / `Dialekt: de`) beside
the switcher that actually sets it. Its own hint text pointed at the Operations page, which was
deleted in Slice 1. Removing it loses nothing: HFS has no dialect concept, **neither server
propagates the UI locale to outgoing requests**, and HTS's real `displayLanguage` support is
per-operation form fields in the workbenches, untouched.

What was actually this branch's doing: restyling it. It carried `.dialect-chip*` classes with no
rule in `app.css` and the class-map sweep mapped them onto `.menu` / `.selector--outline`, turning a
dead element into a first-class-looking control. The divergence was old; the prominence was new.

**Home V3's tile consolidation had been skipped.** Eight tiles across three rows became **four in
one row**, per the approved mockup: Backend and FHIR version fold into Status's sub-line, Bundled
data into Loaded code systems', Avg latency into Requests'. No datum was dropped — each moved one
line down. `.stat-grid` is already `repeat(4, 1fr)`, so **zero CSS**; HFS's own home uses the same
`.stat__label` / `.stat__value` / `.stat__sub` primitives, so tile *count* was never an HFS pattern.
`UpstreamHealth::started_at_utc_hhmm()` (`now − uptime_seconds`, UTC) feeds the "no restarts since"
sub-line; it is honest by construction, because a restart resets uptime and moves the timestamp.

**The chart caption is composed, not fixed.** The mockup's *"Last hour, all status classes."* is
true only for the 1h chip with the All legend item, and the picker offers three windows and four
classes — a literal copy would have been false in two of three windows. `ChartWindow::hint_key()` and
`SeriesFilter::hint_key()` supply sentence forms ("Last 15 minutes", "4xx responses only") that
`I18n::t_arg2_msg` interpolates, so each locale owns its own word order. The mockup's dropped caveat
— *"Sampled while this page is open"* — was **kept**: it is the only thing that explains a sparse
chart. Verified live across all three windows, all four classes and all three locales.

**Also fixed:** `nav-item--soon` (a "coming soon" marker setting `cursor: default`) was still on
Value Sets, Concept Maps and Import — all shipped pages. Removed from all of them.

**Verified:** 184 passing / 0 failing · non-`hts-` key count unchanged at **693** · no raw Fluent
key leaks across six pages × three locales · both topbars now render the same three controls in the
same order. **No CSS change**; `.topbar__avatar` and `.stat-grid` already existed.

### Slice 9 — two defects the official seed set exposed (SHIPPED 2026-08-28)

Both surfaced while rewriting `hts-demo.md` against the real 151 MB seed set. Neither is visible
against the e2e fixtures, which hold ~34 code systems.

**1. Half the catalog had no reachable detail page.** `resolve_canonical_url`
([upstream.rs:928](../../crates/hts-ui/src/upstream.rs#L928)) resolved an id by fetching
`GET /{type}?_count=1000` and searching **that single page**. Everything past it was unreachable:

| | resources | had no detail page |
|---|---|---|
| CodeSystem | 1,977 | **977 (49%)** |
| ValueSet | 20,689 | **19,689 (95%)** |
| ConceptMap | 80 | 0 |

ICD-10-CM sits at position 1,968: the browser row linked to its detail page, the route answered
**HTTP 200**, and the page rendered "The requested resource was not found." inside the shell — so
nothing looked broken from the outside.

Fixed by paging with `_offset` until a match, capped at 40 pages. A short page ends the walk, so a
store smaller than one page still costs exactly one request. Measured against the seeded server:

| position | before | after |
|---|---|---|
| 1st ValueSet | 141 ms | 141 ms |
| 1,001st | not found | 219 ms |
| 10,001st | not found | 734 ms |
| 20,001st (last) | not found | **1,354 ms** |

**Not** fixed by raising `_count`: `GET /ValueSet?_count=100000` resets the connection outright on
this store, so the one-shot alternative does not work at seed scale.

No targeted lookup was available. HTS's `GET /{type}/{id}` 404s on bootstrap-imported resources and
`?_id=` is **silently ignored** (it returns an unfiltered page). Only `?url=` narrows, and the url
is exactly what this function exists to discover.

**2. The OperationOutcome fallback was documented but never implemented.**
`hts-outcome.html` composes `hts-outcome-code-<code>` from the server's FHIR issue code, and its own
comment promises it "falls back to the raw code so unknown codes still surface". `I18n::t` returns
the **key** on a miss, so an ordinary `business-rule` issue printed the literal string
`hts-outcome-code-business-rule` at the reader. The catalog carries sentences for only four codes
(`not-found`, `invalid`, `too-costly`, `unknown`).

Added `I18n::t_or(key, fallback)` and pointed the partial at it, making the documented behaviour
real: an unknown code now surfaces as itself. Guarded by a test that asserts both directions — a
known code still gets its sentence, an unknown one degrades to the code and never to the key.

**Verified:** 185 passing / 0 failing · ICD-10-CM, NDC and NUCC detail pages resolve live · four
ValueSets sampled across the catalog (positions 1 / 1,001 / 10,001 / 20,001) all resolve.

## Open decisions

1. **WCAG contrast failure in shared CSS — DEFERRED by the user 2026-08-27** (not to be fixed yet). `.tag--active` renders `--ok #1a7a3a` on `--ok-soft`
   composited to `#dfebe3` — **4.4:1, needs 4.5:1** at 11px normal. Five light-theme routes; dark is
   clean. **HFS is affected too** (`subscriptions.html:87`). Minimal hue-preserving fix computed:
   **`--ok: #197638` → 4.61:1** (5.45:1 on the content pane). One token. Shared CSS, so it needs
   sign-off. Note `.tag--active` is also **duplicated** at app.css 2544 and 2567.
2. ~~**Toolchain.**~~ **RESOLVED 2026-08-27.** The user installed Visual Studio Build Tools and the
   rustup default was switched to `stable-x86_64-pc-windows-msvc`. Plain `cargo build` /
   `cargo test` now work with no `+toolchain` flag — whole workspace builds, 167 tests pass.
   Installing the Build Tools does **not** fix the GNU toolchain (it still lacks `dlltool.exe`,
   which is a mingw component); MSVC is simply the supported target now.
   **No `rust-toolchain.toml` was added** — this is a machine-level rustup default, not a repo
   change. Anyone else on a GNU default will still hit it, so pinning remains a possible future
   repo-level decision.
3. **Home V3's two data tables — DEFERRED by the user 2026-08-27.** "Loaded content" (top 8 by
   concept count) and "Recent imports" are **not implemented** and are not to be built without a
   team-approved design and explicit sign-off. They were the only elements distinguishing Home V3
   from V2, so **the shipped Home is in practice V2** — tiles + request-rate chart.
   Full analysis, measurements and per-table implementation steps: **improvement-plan §14.**
   Summary of what that analysis established:
   - **Loaded content is low-risk and needs no schema migration.** Two-step ranking query
     (`GROUP BY` on `concepts`, then metadata for the top 8 only) measured at **26–29 ms** over
     445,577 concepts — 12× faster than the single-`JOIN` form at 330 ms. Both backends share the
     schema; there are exactly **two** trait impls and **no mock backends**, so the compiler
     enforces coverage; and `cargo check -p helios-hts --features postgres` compiles cleanly here,
     so the Postgres half is verifiable without a database.
   - **Recent imports is blocked.** Three of its five mockup columns (Content, Concepts, Result)
     do not exist, and creating them means a schema migration that — per the `authority_rank`
     precedent already in the schema — must **invalidate the ledger and force a re-import** on
     every existing deployment, while *still* leaving the multi-GB SNOMED/LOINC/RxNorm rows empty.
   - A **zero-migration alternative** exists for the same slot: "Recently loaded", ordered by
     `code_systems.created_at` (verified populated, 1,977 distinct timestamps). Recorded, not approved.
   - Independent of either card, **`CodeSystem.count` is still worth populating**: `upstream.rs:568`
     and `cs-detail.html:90`/`:149` already consume it and render `&mdash;` because HTS never sends it.

## Known issues (not blocking)

- **Playwright harness hangs on this machine.** Any `globalSetup` hangs indefinitely —
  reproduced with a two-line no-op, and `home.spec.ts` hangs identically, so it predates this work.
  `@playwright/test` 1.49.1 + Node v24.19.0 on Windows. Blocks the whole hts-ui e2e suite until
  Playwright is bumped or Node pinned.
- **Rust suite flakes ~1 run in 5** under full-suite parallelism (11 binaries each spinning loopback
  mocks on Windows). Individual files pass consistently; failures are timeouts, not assertions.

---

## Stage 7 — deployed verification (2026-08-27)

Both servers run together per [run-hfs-and-hts](../../.claude/skills/run-hfs-and-hts/SKILL.md):
HTS on 8090 with the official 151 MB seed set (**1,975 code systems**), HFS on 8080 wired via
`HFS_TERMINOLOGY_SERVER` / `FHIRPATH_TERMINOLOGY_SERVER`. All four sanity checks pass, and the
wiring is proven live — `GET /ui/editor/expand?url=…/administrative-gender` on HFS proxies a real
`$expand` through HTS and returns the four codes.

Screens: `edson/docs/screens/hts-deployed/{light,dark}/`.

### HFS would not start on MSVC — 1 MB main-thread stack

`thread 'main' has overflowed its stack`, deterministically, right after the SearchParameter
registry loads. Not caused by this work (`crates/hfs`, `crates/rest` and `crates/ui/src` are
untouched) — it is the **MSVC default 1 MB main-thread stack**, where the previous GNU default
gave more. `RUST_MIN_STACK` does not help; it only affects spawned threads. Confirmed by
rebuilding with an 8 MB stack, after which HFS starts cleanly:

```
RUSTFLAGS="-C link-arg=/STACK:8388608" cargo build --bin hfs
```

**This is a workaround applied at the command line, not committed.** A durable fix is either a
`.cargo/config.toml` rustflag or running the server body on a thread with an explicit stack size —
both repo-level changes, so neither was made. **Anyone building `hfs` on MSVC will hit this.**

### Regression found only by deploying — and fixed

The stacked-card Diagnostics page rendered **all 1,975 loaded code systems**: a 288 KB table and a
**71,628 px** page. This was a regression from the tabs→stacked-cards mirror — the list used to sit
behind an inactive tab and only drew when selected; stacked cards draw every panel. It is invisible
with the e2e fixtures (34 code systems) and only appears against the real seed set. HFS's own
capability page never hits it because it lists ~150 resource types.

Fixed with HFS's own idiom: a `form.filter-rail__search` GET form in the `.card-head` (exactly what
`capability-statement.html` does for its resource table), plus a **stated** 50-row cap.

| | before | after |
|---|---|---|
| page height | 71,628 px | **3,211 px** |
| page bytes | 357,416 | **29,162** |
| table rows | 1,976 | 52 |

The cap is never silent — the table foot reads *"Showing 50 of 1975 matching code systems — narrow
the filter to see more."* Filtering works without JS: `?filter=icd` → 3 rows and
*"3 of 1975 code systems match this filter."* Three new Fluent keys × 3 locales. **No new CSS.**
Suite still **167 passing / 0 failing**.

*(Superseded 2026-08-27 by Slice 7: this table was removed from the page entirely. The real fix was
that it never belonged on a capability page — `/ui/hts/code-systems` already lists the same rows
from the same table, with more columns and real paging.)*

### The same trap, one layer down — porting HFS's raw block (2026-08-27)

Slice 7 ported HFS's foldable raw `CapabilityStatement`. Against fixtures it is a few KB. Against
the **official seed set it was 422 KB — 95% of a 442 KB page**, shipped on every load whether or not
the `<details>` is ever opened.

Cause: HFS's CapabilityStatement is a fixed-size document. **HTS's grows with its data** — one
`capabilitystatement-supported-system` extension per loaded code system, so 1,977 of them. Porting
an HFS element one-for-one is safe only when the payload behind it scales the same way, and here it
does not.

Fixed with the same idiom as the row cap: a **stated** 16 KiB budget
(`RAW_STATEMENT_BYTE_CAP`), a note giving both sizes, and a link to `/metadata` for the complete
document.

| | before | after |
|---|---|---|
| page bytes | 441,684 | **39,624** |
| raw block | 421,920 | 19,636 *(16 KiB, expanded by HTML escaping)* |
| rest of page | 19,764 | 19,988 |

**Only the seeded deployment shows this.** A regression test now carries its own bulky fixture so
the cap cannot silently come undone.
