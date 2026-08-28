# HTS UI improvement plan

UX proposal for the Helios Terminology Server console (`crates/hts-ui`), following
the method established by [hts-ui-dashboard-ux-proposal.md](hts-ui-dashboard-ux-proposal.md):
diagnose → class map → proposed markup → CSS budget → Fluent keys → a11y → tests.

**Status: IMPLEMENTED 2026-08-27.** All approved decisions are shipped; **167 tests passing**,
verified live against the official seed set. One item is deferred by request — the WCAG contrast
failure in §7.1 — and is **not** to be fixed yet.
**Scope change 2026-08-27:** the **Operations page is removed** from the HTS UI, and the
**Quick links strip is removed** from Home. See §1.5 — the removal orphans three operations
that need a decision.
**Decisions recorded 2026-08-27:** nine page layouts in §4, concept-plane IA = **B** in §5, and
Diagnostics mirrors HFS's Capability page (§3 — that mirror applies to **Diagnostics only**).
**Re-verified 2026-08-26 after `main` was merged** (commit `8af7c5351`) — see §1.4. All line
references below are post-merge; all 44 mockups were re-rendered and re-checked against the
merged stylesheet.
**Authored:** 2026-08-26. **Companion API truth:** [hts-details.md](hts-details.md),
[hts-ui-design.md](hts-ui-design.md).

Screens in this document were rendered against the **real** `crates/ui/assets/app.css`
and the **official** bundled terminology seed set
([crates/hts/terminology-data/](../../crates/hts/terminology-data), 151 MB, 1,975 code
systems loaded) — not fixtures, and not an approximation of the stylesheet.

---

## 1. Diagnosis

### 1.1 The finding

`crates/hts-ui` shares `crates/ui/assets/app.css` byte-for-byte — `#[derive(RustEmbed)]`
`#[folder = "../ui/assets"]` at [lib.rs:86](../../crates/hts-ui/src/lib.rs#L86). There is no
HTS-only stylesheet, and the shared file is the workspace's only CSS.

> **Of the 290 distinct classes the HTS templates emit, 206 (71%) have no matching rule
> anywhere in `app.css`. Of 202 `hts-*` classes, exactly two are styled**
> (`.hts-quick-strip`, `.hts-quick-links`).

Those 206 render at browser default. This is not a theming problem and not a taste
problem — it is markup addressing a vocabulary that was never written.

### 1.2 What that looks like

![CodeSystem detail, Lookup tab — current](screens/hts-current/light/cs-lookup.png)

*`/ui/hts/code-systems/icd9cm%7C2015/lookup`, light theme, real ICD-9-CM data.*

Reading the defects off that one screen:

| # | Defect | Cause |
|---|---|---|
| 1 | "URL", "Publisher", "Jurisdiction" values sit on their own line, indented ~40px | `<dl class="hts-cs-detail__dl">` is unstyled → browser-default `<dd>` indent |
| 2 | The `<h1>` dwarfs everything | `.page-header__title` unstyled → UA `2em`; the styled sibling `.page-head__title` is 20px |
| 3 | "Identity" / "Content" are heavier than the page title's siblings | `.hts-cs-detail__section-title` unstyled → UA `1.5em` |
| 4 | `‹ CodeSystems` is underlined browser-blue — the only non-brand colour on the page | `.backlink` unstyled |
| 5 | "Properties" sits in a raw `<fieldset>` box | no styled wrapper |
| 6 | The two fact blocks stack instead of sitting side by side | `.hts-cs-detail__facts` unstyled → no grid |

Everything that *does* look right on that screen — sidebar, topbar, tab strip,
`.tag--active`, `.field__input`, `.btn--primary` — is a reused HFS primitive. **That is the
whole thesis: where HTS reuses, it looks like HFS; where it invented, it looks like nothing.**

### 1.3 Worst-affected surfaces

| Prefix | Unstyled | Most damaging consequence |
|---|---:|---|
| `hts-vs-workbench__*` | 36 | `$expand` tree renders as a bulleted `<ul>`; `<pre>` raw bodies have no scroll box and blow out page width |
| `hts-cs-workbench__*` | 28 | `__panel-heading` is UA `<h5>` = **0.83em — smaller than body text** |
| `hts-op-workbench__*` | 25 | pass / fail / error / timeout badges are four visually identical plain-text spans |
| `hts-{cs,vs,cm}-detail__*` | 27 | facts blocks have no grid; `<dl>`s are browser-default |
| `hts-diagnostics*` | 19 | `<figure>` inherits UA 40px side margins; Prometheus dump renders unbounded |
| `hts-import*` | 16 | **success and partial-success banners are indistinguishable** |
| `hts-degraded*` | 6 | the degraded banner has no background or border in 14 call sites |
| `hts-outcome*` | 6 | severity (fatal/error/warning/information) is invisible |
| `page-header*` | 5 | 7 pages; 3 browsers already use the styled `.page-head` |
| `dialect-chip*` | 7 | `__panel` is unpositioned — **opening it pushes the topbar down** |

Full per-file inventory with line numbers in §5.

---

### 1.4 Re-verification after the `main` merge (2026-08-26)

`main` was merged into this branch as `8af7c5351`. **`app.css` grew 4,292 → 5,006 lines**, and
[#543](https://github.com/HeliosSoftware/hfs/issues/543) landed — the file now opens with
`@layer tokens, base, components, pages;`. That invalidated several premises of the first draft,
so every claim was re-checked. Results:

| Claim | Post-merge |
|---|---|
| `.content--wide` for full-width pages | **GONE** — replaced by `body.app-shell` + `.content--app`. 18 of 33 variants used it; harness and all 44 mockups re-rendered. |
| `.btn` renders anchors underlined | **FIXED upstream** — `.btn` now sets `text-decoration: none` (1755). |
| `.pill` renders anchors underlined | **still true** (758). Budget drops 2 lines → 1. |
| Cascade hazard: duplicate `.pane` / `.data-table` / `.tabs` / `.tab` / `.tag` blocks | **RESOLVED by #543** — each is now a single rule. |
| Duplicate `.notice` | **still true** — 1126 and 2108, *both in the `components` layer*, so 1126's `padding: 20px 24px` is dead. The only remaining duplicate. |
| `svg { display: block }` global | still true (154) |
| `.chart-legend__type` unstyled | still true (0 rules) |
| Selected row keys on `aria-selected` | still true (2461) |
| `.addbox__panel` is an absolute popover | still true (1248) |
| No `.notice--ok` | still true |

**Both verification gates were re-run against the merged stylesheet and pass:** 134 distinct
classes across the 33 variants and 123 across the 11 single-proposal pages, **every one with a
real rule**, and no horizontal overflow on any of the 44 in either theme.

**One consequence worth a decision.** `.content--app` is not a drop-in for `.content--wide`: it
brings an app-shell scrolling model where the page is fixed and `.filter-center` scrolls
internally. For the browsers that is arguably better at 1,975 rows — the filter strip and table
head stay put. But it is a behaviour change, not just a width change, and it is what the
merged HFS pages now do.

---

### 1.5 Scope change — Operations page and Quick links removed (2026-08-27)

Both removed by request. The Quick links removal is clean; the Operations removal is not, and
needs a decision.

**Quick links (Home).** Drop the `.hts-quick-strip` section from
[hts-home-cards.html:136-146](../../crates/hts-ui/templates/partials/hts-home-cards.html#L136).
Consequences: the Fluent key `hts-home-quick-links` (`locales/en/main.ftl:856` + es/de) becomes
unused, and `.hts-quick-strip` / `.hts-quick-links` (app.css 1098-1106, plus the media-query
block at 1134-1141) become dead — **these are the only two `hts-*` rules in `app.css`**, so
removing them leaves the stylesheet with zero HTS-specific rules. Worth doing as part of the
same change.

**Operations page.** Delete `crates/hts-ui/src/operations.rs` (11 routes), `pages/operations.html`,
the `hts-op-*` partials, and the sidebar entry at
[base.html:46](../../crates/hts-ui/templates/layouts/base.html#L46).

> **This orphans three operations that have no other home in the UI.** The detail-page tab
> enums are `CsTab { Lookup, Validate, Subsumes }`, `VsTab { Expand }`, `CmTab { Translate }` —
> so removing the workbench makes the following unreachable from the browser:
>
> | Orphaned | Why it has no home | Partial that already exists |
> |---|---|---|
> | **`$closure`** | `CmTab` has only `Translate` | `hts-cm-closure-{input,result}.html` |
> | **`$batch-validate-code`** | nowhere else at all | `hts-vs-batch-{input,table,row,progress}.html` |
> | **ValueSet `$validate-code`** | `VsTab` has only `Expand` | `hts-vs-validate-{input,result}.html` |
>
> All three remain available over HTTP; only the UI surface goes.

**Two ways forward — needs your call:**

1. **Accept the loss.** The three become API-only. Simplest, and defensible if nobody drives
   them from the console. Deletes ~11 routes and 15 partials.
2. **Re-home them as detail-page tabs** (recommended). `VsTab` gains `Validate` and
   `BatchValidate`; `CmTab` gains `Closure`. The partials already exist and the tab-strip
   pattern is already there — this is a move, not a rewrite, and it keeps every HTS operation
   reachable while still deleting the standalone page.

Note the Operations page was also the "start from an arbitrary system + code" entry point.
Direction **B — Concept-first** (§4) replaces that role with the global concept search, which
is part of why B fits a console without a workbench page.

---

## 2. The page-by-page uplift — direction-independent

**Every page reachable from the sidebar is in scope.** These fixes are identical regardless of
which IA direction (§4) you pick, so this work does not block on that decision and can start
immediately.

Each screen is real HTML rendered against the real `app.css` at 1440×900. Dark variants are in
`screens/hts-proposed/dark/`. Every render is asserted to produce **no horizontal overflow** —
the current unstyled `<pre>`/`<table>` blow-out is a real bug, so that assertion is a
regression gate, not decoration.

| Page | After | Headline fix |
|---|---|---|
| Home | [home](screens/hts-proposed/light/home.png) | tiles kept; **chart card added** (rate, not cumulative) |
| Code systems | [cs-browser](screens/hts-proposed/light/cs-browser.png) | facet chips, `.data-table`, `%7C`-encoded links |
| CodeSystem detail | [cs-detail](screens/hts-proposed/light/cs-detail.png) | `.kv-grid` facts, real `A00.0` `$lookup` result |
| Value sets | [vs-browser](screens/hts-proposed/light/vs-browser.png) | same browser treatment |
| ValueSet detail | [vs-detail](screens/hts-proposed/light/vs-detail.png) | `$expand` tree as an **indented `.data-table`** — zero new CSS |
| Concept maps | [cm-browser](screens/hts-proposed/light/cm-browser.png) | stacked `.cm-mapping` Source/Target column |
| ConceptMap detail | [cm-translate](screens/hts-proposed/light/cm-translate.png) | matches table + reverse-mode origin footnote |
| Import | [import](screens/hts-proposed/light/import.png) | **success vs partial now distinct** |
| Diagnostics | [diagnostics](screens/hts-proposed/light/diagnostics.png) | code systems as `.data-table`, metrics in `.detail__code` |

### The most consequential fix

**Home — the operational chart.**

![Home, after](screens/hts-proposed/light/home.png)

The existing stat tiles are already correct (they came from the dashboard proposal) and are
kept as-is; the **Quick links strip is removed** per the 2026-08-27 scope change. The chart card
reuses HFS's SVG markup verbatim: `.card.chart-card`,
`<svg class="chart" viewBox="0 0 1060 300">`, `.grid-line`, `.series`, `.axis-label`,
`.window-picker`, `.chart-legend`. It plots **requests per minute**, with the legend repurposed
as a status-class series selector (all / 2xx / 4xx / 5xx). Zero new CSS.

*(The batch-validate badge fix — four states that render as four identical plain-text spans
today — was the other headline item. It leaves with the Operations page; see §1.5.)*

### Six defects found by rendering (not visible in code review)

These are the payoff from building real mockups rather than reasoning about markup. One is a
live bug in a shipped page; the rest are traps the implementation would otherwise walk into.
Each was re-checked after the `main` merge — one (`.btn`) was fixed upstream in the meantime.

1. **`<a class="pill">` renders underlined — still live on the shipped home page.** There is no
   global `a { text-decoration: none }` in `app.css` and `.pill` (758) never resets it. Visible in
   the Quick links row of [the current home capture](screens/hts-current/light/home.png).
   **Partly fixed by the merge:** `.btn` now sets `text-decoration: none` (1755), so
   `<a class="btn">Reset</a>` is no longer affected. One line remains (§6).
2. **`svg { display: block }` is a global rule** ([app.css:140](../../crates/ui/assets/app.css#L140)).
   Every inline icon-beside-text run breaks — the icon drops onto its own line unless the
   parent is a flex line box. This will bite every backlink, notice and summary that gains an
   icon.
3. **`.chart-legend__type` has no rule in `app.css`** — the only class in HFS's *own* chart
   markup ([index.html:94](../../crates/ui/templates/pages/index.html#L94)) that is unstyled. A
   bare `<span>` renders identically, so the fix is to drop the class, not add a rule.
4. **The full-width shell is `body.app-shell` + `.content--app`, set via two Askama blocks.**
   `.content--wide` **no longer exists** — the `main` merge replaced it. Pages that want full
   width set *both* `{% block body_class %}app-shell{% endblock %}` and
   `{% block content_class %} content--app{% endblock %}`
   ([resources.html:6-7](../../crates/ui/templates/pages/resources.html#L6)). `.content--app`
   also brings an **internal-scroll model**: the page stops scrolling and `.filter-center`
   scrolls instead. Neither is reachable from inside a page body.
5. **Content must sit inside `.pane`** — `body` is `display: grid; grid-template-columns: 76px 1fr`
   ([app.css:117](../../crates/ui/assets/app.css#L117)) and `.pane` supplies `grid-column: 2`.
   A generic wrapper lands in the 76px rail and collapses the page to a ~150px column.
6. **Selected-table-row highlight keys on `aria-selected`, not `aria-current`.** The only rule
   is `.data-table tbody tr[aria-selected="true"] td`
   ([app.css:2004](../../crates/ui/assets/app.css#L2004)); `aria-current` on a `<tr>` paints
   nothing — even though `aria-current` is what every *other* selected thing in the codebase
   uses (`.chip`, `.tab`, `.nav-item`, `.menu__option`, `.filter-rail__item`). Needs a decision,
   not just a swap: `aria-selected` on a plain `<tr>` is only valid ARIA inside a
   `role="grid"`/`treegrid`, so either add `tr[aria-current="true"] td` to that selector (1 line,
   consistent with the rest of the codebase) or give the table a grid role. **Recommend the
   one-line CSS addition** — it aligns the table with the convention already used everywhere else.

Minor: the `Not valid` badge wraps to two lines inside a fixed-height 20px `.tag` — use a
shorter label or `white-space: nowrap`. And note the id shapes differ by resource type:
CodeSystem ids carry a version pipe (`icd9cm|2015` → `%7C`), while ValueSet and ConceptMap
browser rows emit bare ids (`bodysite-laterality`, `sc-encounter-status`). Both forms resolve,
but links must go through `encodeURIComponent` regardless.

### Constraint check

The eleven mockups were machine-verified against `app.css` (comments stripped, so a class
mentioned only in a comment does not count as styled):

> **122 distinct classes used across 11 pages. Every one has a real rule in `app.css`.
> Zero invented classes.**

Today the same templates emit 290 classes of which 206 are unstyled. That is the whole change,
stated as a number: **206 unstyled → 0**, while the visual language stays exactly HFS's.

---

## 3. Diagnostics — mirror HFS's Capability page

**Scope: this page only.** Reviewer direction 2026-08-27 was that HTS's Diagnostics page should
not be reinvented, because HFS already solves it. Every other page follows the decisions in §4.

HTS Diagnostics copies [capability-statement.html](../../crates/ui/templates/pages/capability-statement.html):

- plain shell — **no** `app-shell`, **no** `content--app`, **no** `filter-layout`
- `section.page-head` with `h1.page-head__title` + `p.page-head__lede`
- **five stacked `section.card` / `section.card.table-card` blocks**, each opened by
  `div.card-head` > `<h3>` — no tab strip
- facts as bare `div.detail__field` (`<span>` label, `<div>`/`<code>` value)
- inline filter as `form.filter-rail__search` inside a `.card-head` (a plain GET form)
- tables as `.table-wrap` > `table.data-table`, numeric columns `.col-num`,
  empty state `tr.data-table__empty`
- raw payload as a **bare `<details>`** + `<summary>` + `<pre class="detail__code">` inside a
  `.card` — HFS does not style this, and neither should we
- unavailable state as `p.notice.notice--warn`

This replaces the Diagnostics V1/V2/V3 exploration; HFS's shape is closest to V2 (stacked
cards). **Zero new CSS.**

### 3.1 Sidebar naming — RESOLVED 2026-08-27

**The page is now "Capability & Conformance" at `/ui/hts/capability-statement`.** It reuses HFS's
own Fluent keys — `nav-capability-conformance` for the sidebar and `cap-title` for the `<h1>`,
exactly as HFS pairs those two different strings. The catalog is shared between the crates, so the
labels are the same by construction rather than by copy. The icon (`shield.svg`) already matched.

**This section previously read "still open", and that was the reasoning error.** The original
instruction was explicit — *"even the left menu should have the same name and icon as hfs"* — and
this paragraph talked itself out of it on the grounds that HTS's page "also surfaces `/health` and
`/metrics`, so the HFS label may be too narrow". Two things were wrong with that:

1. It treated a delivered instruction as an open question, and the omission then survived
   implementation because the slice was scoped as a page-body task while the nav label lives in
   `layouts/base.html`.
2. The premise inverted the argument. `/health` and `/metrics` were not a reason to keep a broader
   name — they were **the two cards that did not belong on a capability page**, and both were
   removed (Home already shows each). Once they were gone, the HFS label was not too narrow; it was
   exact.

Shipped shape and the full card-by-card record: execution-plan **Slice 7**.

---

## 4. Page decisions — approved by your team

These are binding. Each was chosen from the three rendered variants during the 2026-08-27
walkthrough; the images are kept below as the record of what was compared.

| Page | Decision | Variant |
|---|---|---|
| Home | Ops console — 4 consolidated tiles + chart; the two data tables are deferred (§14) | **V3** |
| Code systems | Top strip — horizontal filters, full-width table | **V2** |
| Value sets | Top strip | **V2** |
| Concept maps | Top strip | **V2** |
| CodeSystem detail | Compact header — facts as chips, workbench dominant | **V3** |
| ValueSet detail | Compact header | **V3** |
| ConceptMap detail | Compact header | **V3** |
| Import | Stepped — 1 Choose source → 2 Review → 3 Result | **V3** |
| Diagnostics | mirror HFS Capability page — **including its name**: renamed to *Capability & Conformance* at `/ui/hts/capability-statement` on 2026-08-27 (§3) | §3 |
| Concept plane IA | Concept-first | **B** (§5) |

One trade accepted knowingly: ValueSet detail V3 drops the sticky `$expand` form, so on a
SNOMED-scale expansion the form scrolls away where V2's rail would have held it. Chosen for
consistency across the three detail pages.

**Home is partially delivered.** Its "Loaded content" and "Recent imports" tables were **deferred on
2026-08-27** pending a team-approved design — the full analysis of what each would cost to build is
in **§14**.

Everything else in V3 has shipped. The tile consolidation landed on **2026-08-28** (execution-plan
Slice 8): eight tiles across three rows became **four in one row**, with Backend, FHIR version,
Bundled data and Avg latency folded into the sub-line of the tile each qualifies, exactly as the
mockup draws them. The chart's caption is **composed from the selected window and status class**
rather than copied literally — the mockup's static *"Last hour, all status classes."* would be false
in two of the three windows the picker offers — and it keeps the sampling caveat the mockup drops.

**One place where the mockup was deliberately not followed:** its page title and lede. The current
"Home / Terminology server health, catalog inventory, and quick actions." is kept, because the
mockup's wording there breaks HFS's `page-head` pattern. Confirmed with the user 2026-08-28.

Every sidebar page has **three layout variants**. (Operations and Batch validate were cut on 2026-08-27 — see §1.5.) They use the *same* tokens, components, type
and density primitives — they differ only in **information architecture and layout**. Aesthetics
are not an axis; that is the constraint.

**V1 is always today's shape with only the styling fixed**, so there is a conservative option on
every row. Dark variants are in `screens/hts-variants/dark/`.

All 27 were verified together: **133 distinct classes, every one has a real rule in `app.css`**,
and every page renders with no horizontal overflow in both themes.

**Recommendations at a glance** — my pick per page, with the reasoning under each section:

| Page | V1 | V2 | V3 | Pick |
|---|---|---|---|---|
| Home | Tiles + chart | Chart-first | Ops console | **V3** |
| Code systems | Rail (today) | Top strip | Split preview | **V2** |
| Value sets | Rail (today) | Top strip | Split preview | **V2** |
| Concept maps | Rail (today) | Top strip | Split preview | **V2** |
| CodeSystem detail | Stacked (today) | Sidebar facts | Compact header | **V3** |
| ValueSet detail | Stacked (today) | Sidebar facts | Compact header | **V2** |
| ConceptMap detail | Stacked (today) | Sidebar facts | Compact header | **V3** |
| Import | Stacked (today) | Two-column | Stepped | **V3** |
| Diagnostics | Tabs (today) | Stacked cards | Two-column | **V3** |

Note the browsers and the detail pages **do not** get the same answer: browsers want width for
canonical URLs (V2), while CodeSystem and ConceptMap detail want the workbench above the fold
(V3) and ValueSet detail wants width for the expansion (V2). Picking one shape for all of them
would be tidier and worse.

---

### 4.1 Home — *How much the page answers before you click*

**V1 — Tiles + chart**

Today's shape with the chart appended. Safe; answers "is it up" and nothing else.

![Home V1](screens/hts-variants/light/home-v1.png)

**V2 — Chart-first**

Chart is the hero; tiles compress below it. Argues traffic is why you open the page.

![Home V2](screens/hts-variants/light/home-v2.png)

**V3 — Ops console**  ← recommended

Adds a **Loaded content** table (real concept counts) and **Recent imports** — including a *failed* RxNorm row.

![Home V3](screens/hts-variants/light/home-v3.png)

**Why V3:** The question this console exists to answer is "what terminology is loaded, and did the last import work." V1 and V2 both answer it with a bare `1,975` that forces a click. V3 answers it in place, and surfaces a failed import an operator must not have to hunt for. Longest page, but every block below the chart is a click saved.

---

### 4.2 Code systems — *Where the filters live*

**V1 — Rail (today)**

280px sticky rail + table. Today's shape, styling fixed.

![Code systems V1](screens/hts-variants/light/cs-browser-v1.png)

**V2 — Top strip**  ← recommended

Filters horizontal, table full width. Status stays visible as facet chips.

![Code systems V2](screens/hts-variants/light/cs-browser-v2.png)

**V3 — Split preview**

Rail + table + right preview panel on row select.

![Code systems V3](screens/hts-variants/light/cs-browser-v3.png)

**Why V2:** The rail spends 280px permanently on five inputs touched once per session, and squeezes the one column that cannot be truncated. `.col-name` already caps Name at 240px with ellipsis and `.url` is `overflow-wrap: anywhere` — so under V1 a canonical like `http://terminology.hl7.org/CodeSystem/medicationrequest-status-reason` wraps to two or three lines and row height doubles. V2 also has no responsive cliff: V1 and V3 both collapse to one column at 1250px, where the rail becomes a tall card you scroll past to reach results.

---

### 4.3 Value sets — *Where the filters live*

**V1 — Rail (today)**

As CS.

![Value sets V1](screens/hts-variants/light/vs-browser-v1.png)

**V2 — Top strip**  ← recommended

As CS.

![Value sets V2](screens/hts-variants/light/vs-browser-v2.png)

**V3 — Split preview**

As CS.

![Value sets V3](screens/hts-variants/light/vs-browser-v3.png)

**Why V2:** Same argument as Code systems — keep the two browsers identical.

---

### 4.4 Concept maps — *Where the filters live*

**V1 — Rail (today)**

Rail + stacked `S:`/`T:` Mapping column.

![Concept maps V1](screens/hts-variants/light/cm-browser-v1.png)

**V2 — Top strip**  ← recommended

Full-width table; drops the two filters HTS ignores anyway.

![Concept maps V2](screens/hts-variants/light/cm-browser-v2.png)

**V3 — Split preview**

Rail + table + preview (620px spent on chrome).

![Concept maps V3](screens/hts-variants/light/cm-browser-v3.png)

**Why V2:** The CM row carries **three** canonical URLs (URL column plus the stacked S:/T: cell). V1 spends 280px on a rail and V3 spends 620px on rail + preview — at which point the Mapping cell wraps and the scannable "does this map X → Y" grammar the stacked cell exists for is destroyed. And the filters V2 drops are exactly the two the server silently ignores (`source`/`target` are not in `ResourceSearchQuery`).

---

### 4.5 CodeSystem detail — *How much room metadata gets*

**V1 — Stacked (today)**

Full-width facts card, then tabs, then workbench.

![CodeSystem detail V1](screens/hts-variants/light/cs-detail-v1.png)

**V2 — Sidebar facts**

Facts in a 280px rail, workbench right.

![CodeSystem detail V2](screens/hts-variants/light/cs-detail-v2.png)

**V3 — Compact header**  ← recommended

Facts become a chip row; full facts in a disclosure. Workbench dominates.

![CodeSystem detail V3](screens/hts-variants/light/cs-detail-v3.png)

**Why V3:** V1 makes you scroll past metadata you read once, every visit. V2 fixes that but permanently rents 280px to reference material on a page with no wide content. V3 keeps the default column, puts the four facts operators actually re-check in a one-line chip row, keeps the canonical URL visible, and files the rest away. The form lands around y≈260px.

---

### 4.6 ValueSet detail — *How much room the expansion gets*

**V1 — Stacked (today)**

Facts, form, then expansion.

![ValueSet detail V1](screens/hts-variants/light/vs-detail-v1.png)

**V2 — Sidebar facts**  ← recommended

Facts + `$expand` form in the rail; expansion full width.

![ValueSet detail V2](screens/hts-variants/light/vs-detail-v2.png)

**V3 — Compact header**

Facts as chips; expansion above the fold.

![ValueSet detail V3](screens/hts-variants/light/vs-detail-v3.png)

**Why V2:** Opposite call to CodeSystem detail, and deliberately so. The System column is a 55-character canonical and the expansion is the only thing on the page that varies in size — 7 rows here, thousands on a SNOMED value set. The sticky rail lets you re-filter without scrolling back past a long expansion, and it is the only layout where the tree indent stays legible beside a long System value.

---

### 4.7 ConceptMap detail — *How much room matches get*

**V1 — Stacked (today)**

Facts, tabs, form, matches.

![ConceptMap detail V1](screens/hts-variants/light/cm-translate-v1.png)

**V2 — Sidebar facts**

Facts + form in the rail.

![ConceptMap detail V2](screens/hts-variants/light/cm-translate-v2.png)

**V3 — Compact header**  ← recommended

Facts as chips; form and matches dominant. Shows the reverse-mode origin footnote.

![ConceptMap detail V3](screens/hts-variants/light/cm-translate-v3.png)

**Why V3:** `$translate` returns one to a handful of matches, so V2 buys width the table cannot use — a single match row with 60% of the viewport empty beside it. V3 compresses five facts into one line and pushes form and matches above the fold, matching how the page is used: type a code, read four columns, change the code.

---

### 4.8 Import — *Whether import reads as a linear task*

**V1 — Stacked (today)**

Form, then status banners.

![Import V1](screens/hts-variants/light/import-v1.png)

**V2 — Two-column**

Form left, status right.

![Import V2](screens/hts-variants/light/import-v2.png)

**V3 — Stepped**  ← recommended

Numbered steps: 1 Choose source → 2 Review → 3 Result.

![Import V3](screens/hts-variants/light/import-v3.png)

**Why V3:** Import is linear and writes to the database. Step 2 "Review" is where the counts belong — it gives the operator somewhere to see 49 entries / 4,218 concepts *before* writing. V1 and V2 go straight from paste to result. All three make success / partial / error visually distinct, which today they are not.

---

### 4.9 Diagnostics — *Tabs vs. everything visible*

**V1 — Tabs (today)**

One panel at a time.

![Diagnostics V1](screens/hts-variants/light/diagnostics-v1.png)

**V2 — Stacked cards**

All four panels sequentially.

![Diagnostics V2](screens/hts-variants/light/diagnostics-v2.png)

**V3 — Two-column**  ← recommended

Short panels left, long ones right.

![Diagnostics V3](screens/hts-variants/light/diagnostics-v3.png)

**Why V3:** This page's job is cross-referencing — you compare `/health` uptime against the `uptime_seconds` gauge, or the CapabilityStatement operation list against the advertised code systems. V1 hides three of four panels. V2 fixes that but the Prometheus dump pushes everything else off-screen. V3 puts the two short panels opposite the two long ones, so the tall `<pre>` scrolls past a column that has already ended.

---

## 5. Concept plane IA — Direction B (approved)

All three use the **identical** design system: same tokens, type ramp, components, density.
They differ only in **information architecture** — where a concept lives and how you reach it.
Aesthetics are not on the table; that is the constraint.

Every screen below is real HTML rendered against the real `app.css`. Dark variants are in
`screens/hts-proposed/dark/`.

### Direction A — Workbench-first

![Direction A](screens/hts-proposed/light/a-concept.png)

The concept becomes a **fourth tab** on the existing CodeSystem detail page, beside Lookup /
Validate / Subsumes. Navigation is unchanged; the facts block and tab strip stay exactly where
they are.

- **New routes:** 1 · **htmx complexity:** lowest — one lazy-fragment pattern already in the crate
- **For:** smallest diff, least new surface to test, no new mental model
- **Against:** least improvement per unit of work. The browsers stay endpoints, and finding one
  code among ICD-10-CM's ~100k still means going through a CodeSystem first.

### Direction B — Concept-first

![Direction B](screens/hts-proposed/light/b-concept.png)

The concept is a **top-level object** with its own permalink and a global search in the topbar.
Identity / Mappings / Subsumption are sibling panels on one page. Browsers become entry points
rather than destinations.

- **New routes:** 2 (`/ui/hts/concepts`, topbar search) · **htmx complexity:** medium
- **For:** search-first is how you actually find one code in 100k. The permalink is shareable,
  which is what makes "send me that concept" work. Closest to echidna/Athena.
- **Against:** the topbar search is a new global control to design, translate and test; it
  changes the mental model from "browse resources" to "look up concepts".

### Direction C — Two-pane explorer

![Direction C](screens/hts-proposed/light/c-concept.png)

Persistent result list on the left, concept detail on the right. Selecting a row swaps the
right pane via htmx **without losing the search**. Closest to Ontoserver Shrimp / Snowstorm.

- **New routes:** 2 + a rail fragment · **htmx complexity:** highest
- **For:** strongest at real scale — you keep the result set while inspecting, and comparing
  A00.0 against A00.9 is two clicks. Best fit for the 100k-concept case.
- **Against:** the most new htmx surface, and where the nojs fallback and focus management get
  genuinely hard — every pane swap needs a real URL, and returning focus after a swap is
  fiddly. Highest risk of a11y regressions.

### Recommendation

**B**, unless you expect operators to spend most of their time comparing concepts side by side —
in which case C. B buys the permalink and the search (the two things that actually change how
the console is used) for materially less risk than C, and unlike A it fixes the "find one code
in 100k" problem that the official seed set makes unavoidable. A is the safe choice if the
priority is closing the styling gap with minimum new surface.

---

## 6. Class map — current → existing HFS primitive

The rule: **prefer deleting an invented class and using the HFS one over writing a new rule.**

### 6.1 The high-value swaps (no new CSS at all)

| # | Invented (unstyled) | Emitted at | Existing styled primitive |
|---|---|---|---|
| 1 | `page-header` | home:5, diagnostics:24, import:23, operations:24, cs-detail:32, vs-detail:32, cm-detail:34 | **`.page-head`** app.css:1339 |
| | `page-header__title` | diagnostics:26, import:24, operations:26, cs/vs/cm-detail:34/34/36 | **`.page-head__title`** :1343 |
| | `page-header__subtitle` | home:7, import:25, operations:27, cs/vs/cm-detail:46/46/48 | **`.page-head__lede`** :1350 |
| | `page-header__row` | diagnostics:25 | **`.page-head--row`** :3513 (flex, space-between — exactly what it wants) |
| | `page-header__eyebrow` | operations:25, cs/vs/cm-detail:33/33/35 | `.stat__label` :678 or `.detail__hint` :2864 |
| 2 | `btn--ghost` / `btn--secondary` | 3 browsers + 5 row/result sites | **`.btn`** :1402 — both modifiers are pure no-ops today |
| 3 | `hts-degraded` | **14 call sites** | **`.notice.notice--warn`** :1755 |
| | `hts-op-banner` | operations:45 | `.notice--warn` :2116 |
| | `hts-outcome--error` | hts-outcome:25 | `.query-error` :1865 / `.lint__item--error` :2893 |
| | `hts-import-status--error` / `--warn` | import-status:23 / :17,:41 | `.query-error` :1865 / `.notice--warn` :2116 |
| 4 | `hts-*-detail__dl`, `hts-diagnostics-facts`, `hts-op-workbench__facts`, `hts-cs-workbench__properties` | 12 sites | **`.kv-grid`** :2410 + `.detail__field` :2807 |
| 5 | `hts-op-workbench__badge--true/false` | op-generic:33/37, vs-validate:21/25, vs-batch-row:26/30 | **`.tag--matched`** :2084 / **`.tag--excluded`** :2097 — the crate already does this correctly at cs-workbench-result:80/85 |
| 6 | `hts-op-workbench__table`, `hts-diagnostics-resources`, `hts-import-status__counts` | 4 sites | **`.data-table`** :1980 in **`.table-wrap`** :1976 |
| 7 | `hts-*-workbench__raw-body`, `hts-diagnostics-metrics__body`, 3 bare `<pre>` | 8 sites | **`.detail__code`** :2312 (the ones breaking page width) |
| 8 | `dialect-chip*` | base.html:101-108 | **`.menu`** :336 / `.selector` :427 / `.menu__panel` :414 / `.menu__heading` :501 — base.html uses this exact pattern 30 lines earlier for the version picker |
| 9 | `hts-*-workbench__advanced` / `__raw` / `__echoed-parameters`, `hts-import-status__issues` (`<details>`) | 13 sites | **no primitive exists — see §6.** `.addbox` is NOT it (corrected below) |
| 10 | `backlink` | cs/vs/cm-detail:31/31/33 | `.row-link` :2469 |
| | `hts-diagnostics__fhir-chip` | diagnostics:27 | `.chip` :2390 |
| | `hts-vs-workbench__pager` | vs-expand-result:181 | `.table-foot` :2490 (already used by the row partials) |
| | `hts-*-detail__result-empty`, `hts-op-workbench__empty`, `__no-members`, `__no-matches` | 12 sites | `.query-empty` :1859 |
| | `hts-cs-workbench__panel` | cs-workbench-result:48,63 | `.panel` :2926 |

### 6.2 Two markup contracts to get right

Both were found by rendering the mockups — they are silent failures, not errors.

**`.detail__field` — the value must NOT be a `<span>.`** The rule at app.css:2276 is
`.detail__field > span { font-size:10px; text-transform:uppercase; … }`, which targets *every*
direct span child. HFS's own usage
([compartments.html:54](../../crates/ui/templates/pages/compartments.html#L54)) is:

```html
<div class="detail__field"><span>LABEL</span><div>value</div></div>
<div class="detail__field detail__field--wide"><span>LABEL</span><code>long value</code></div>
```

Label is the `<span>`; value is a `<div>` or `<code>`. A second `<span>` renders the value in
uppercase 10px muted. Use `--wide` (:2417, `grid-column: 1/-1`) for canonical URLs and
descriptions.

**Inline icons need explicit `width`/`height`.** `app.css` sizes `.nav-item .icon` (:243) but
never `.icon svg`. Every real icon in `crates/ui/templates/icons/` carries
`width="16" height="16" … fill="currentColor"`. Without them the SVG collapses to zero.

### 6.3 Cascade hazards — largely resolved by the merge

The first draft flagged five duplicated top-level blocks. **#543 consolidated all but one.**
Verified by mapping every bare top-level rule to its enclosing `@layer`:

| Selector | Before | Now |
|---|---|---|
| `.pane` | 128 + 494 | single, 552 (`components`) |
| `.data-table` | 1040 + 1980 | single, 2426 (`components`) |
| `.tabs` / `.tab` | 1204/1212 + 2381/2387 | single, 1534/1543 (`pages`) |
| `.tag` | 1105 + 2054 | single, 2515 (`components`) |
| `.notice` | 849 + 1748 | **still doubled** — 1126 and 2108, both `components` |

So the guidance inverts: ordering is now governed by `@layer tokens, base, components, pages`,
and new rules must go in the right **layer**, not just the right place in the file. The single
remaining duplicate is `.notice`, where 1126's `padding: 20px 24px` is dead because 2108 wins.
Worth folding together, but it is not this plan's job.

---

## 7. CSS budget

> Every new rule must be composed only of **existing custom properties**, and must not alter any
> element HFS already renders.

Measured against the **approved variants** (§4), not against the exploration. Four of the five
lines the earlier draft budgeted turned out to be unnecessary — but for concrete reasons, not
because of the over-broad mirror reading that an earlier revision of this section claimed:

| Was budgeted | Status | Why |
|---|---|---|
| shared heading rule (~4) | **not needed** | the approved variants use `.card-head` + `<h3>` (20 uses) — already styled |
| facts wrapper (~3) | **not needed** | they use `.kv-grid` (13 uses) — already styled |
| `.pill { text-decoration: none }` (1) | **not needed** | the only `<a class="pill">` in the workspace is the quick-links strip being deleted; HFS never uses it |
| inline-disclosure rule (~5) | **not needed** | **decided 2026-08-27:** follow HFS's precedent and use a bare `<details>` + `<summary>` + `<pre class="detail__code">` inside a `.card`, exactly as `capability-statement.html` does. `.addbox` must **not** be used — it is the Add-tenant dropdown and `.addbox__panel` is `position: absolute`, so it would float a 340px popover over the page. Affects 11 sites across the approved detail and import variants. |
| `tr[aria-current="true"]` (1) | **REQUIRED** | **decided 2026-08-27.** 3 sites in the approved `vs-detail-v3` and `cm-translate-v3` mark a selected row with `aria-current`, which paints nothing — only `tr[aria-selected="true"]` is styled (6.3). Adding `aria-current` to that selector folds tables into the convention `.chip`, `.tab`, `.nav-item` and `.filter-rail__item` already use. |

```css
/* app.css — extend the existing rule at the .data-table selected-row block */
.data-table tbody tr[aria-current="true"] td { background: var(--accent-soft); }
```

**Total: 1 line added, 2 rules deleted — as shipped.** The deletions are `.hts-quick-strip` and
`.hts-quick-links`, which went with the Quick links strip (§1.5), leaving `app.css` with **zero
HTS-specific rules**.

### 7.1 One further line — DEFERRED by the user (2026-08-27), not to be applied yet

The axe-core gate added in Stage 6 found a genuine WCAG 2.2 AA failure in **shared** CSS:

```
.tag--active — contrast 4.4:1, expected 4.5:1
foreground --ok #1a7a3a on --ok-soft composited to #dfebe3, 11px normal
```

Five light-theme routes; dark theme is clean. The token comment at app.css:92 claims these clear
4.5:1 — true on white, but not on the 12%-alpha tinted chip. **HFS is affected too**
(`subscriptions.html:87`), so this is not an HTS-only defect.

Minimal hue-preserving fix, computed rather than guessed:

```css
--ok: #197638;   /* was #1a7a3a — 4.61:1 on the chip, 5.45:1 on --bg-content */
```

Also note `.tag--active` is **duplicated** at app.css 2544 and 2567 — the one remaining duplicate
besides `.notice` (§6.3).

---

## 8. Backend findings (verified against the running official-seed server)

No `crates/hts` change is required for anything in this document.

**Cross-system mappings already work.** `POST /ConceptMap/$translate` with `url` **omitted**
scans `concept_map_elements` across every stored map. Proven live:

```
POST /ConceptMap/$translate   {"system":"http://hl7.org/fhir/encounter-status","code":"planned"}
→ match: concept=http://hl7.org/fhir/resource-status|planned
         equivalence=equivalent
         originMap=http://hl7.org/fhir/ConceptMap/sc-encounter-status|4.0.1
```

Postgres behaves identically (`backends/postgres/concept_map.rs:333-348`, `$4::text IS NULL`).

**Subsumption works on bootstrap-imported content.** `$subsumes(A00, A00.0)` on ICD-10-CM
returns `subsumes`; `$lookup(A00.0)` reports `parent=A00`.

**Three limits to render honestly, not paper over:**

1. **`originMap` is suppressed in reverse mode** ([translate.rs:187](../../crates/hts/src/operations/translate.rs#L187)).
   A cross-map *reverse* query cannot attribute its matches. Render Origin as an em-dash with a
   footnote; do not guess.
2. **`source` / `target` are parsed but never bound** by `query_translate_elements`. Do not
   expose them — they would be controls that do nothing.
3. **The closure cache goes stale.** `write_code_system` deletes `concept_closure` on every
   write ([fhir_bundle.rs:428](../../crates/hts/src/import/fhir_bundle.rs#L428)) and only
   `import_parsed_sync` rebuilds it, and only for systems that had *zero* concepts before. The
   startup safety net (`migrate_concept_closure`) runs at process start only. **`concept_hierarchy`
   survives both re-import paths**, so `$lookup` keeps reporting `parent=A` while `$subsumes`
   returns `not-subsumed`. This is why the Subsumption panel derives its comparators from
   `parent`/`child` and shows the disagreement — a user-entered comparator could never surface it.

**Two contract details for error rendering:**

- Every 404 carries a JSON `OperationOutcome` (`error.rs:137-199`), including POST operations —
  verified live. `issue.code = "not-found"` with a `tx-issue-type` coding.
- But the `Content-Type` is `application/json`, **not** `application/fhir+json` — axum's `Json`
  hard-sets it. **Do not gate the outcome partial on the FHIR content type.**
- Extractor-level rejections (malformed body → 400/415, `TimeoutLayer` → 408) bypass `HtsError`
  entirely and return plain text. A generic degraded banner is still required.

**Two smaller ones surfaced while capturing:**

- Resource ids contain pipes (`icd9cm|2015`), so every detail link needs `%7C` encoding.
- `Bundle.total` is the **page size**, not the match count
  ([search.rs:35](../../crates/hts/src/operations/search.rs#L35)) — visible now that 1,975
  systems are loaded and a `_count=5` search still reports `total: 5`.

---

## 9. Fluent keys

All user-visible strings are Fluent keys in `locales/{en,es,de}/main.ftl`; the
`catalogs_share_the_same_key_set` test fails the suite on drift. 491 `hts-*` keys exist today.

New prefixes required by the chosen direction:

- `hts-concept-*` — identity labels, panel headings, empty and loading states
- `hts-concept-mappings-*` — column headers, per-map grouping, the reverse-origin footnote
- `hts-concept-relations-*` — relation kinds, truncation notice, closure-disagreement caveat
- `hts-home-chart-*` — window labels, series labels, four empty states, the self-traffic hint

Dynamic keys follow the established `format!("hts-…-{}", x)` pattern for `$subsumes` outcomes
(`equivalent` / `subsumes` / `subsumed-by` / `not-subsumed`) and severity.

---

## 10. Accessibility

`crates/ui/e2e` runs an axe-core gate at strict WCAG 2.2 AA **including `color-contrast`, in
both themes**, over 10 routes ([a11y.spec.ts:8](../../crates/ui/e2e/tests/a11y.spec.ts#L8)).
**`crates/hts-ui/e2e` has no such file.**

Since this work changes contrast on every page, that gate is a **prerequisite, not a
follow-up**. Mirroring it also closes the `phase1_3_debt` residual tracked in
[hts-ui-design.md](hts-ui-design.md) §12.

Per-surface: `aria-live="polite"` on lazily-swapped panels; every skeleton carries a
`<noscript>` real `<a>`; each fragment route also renders a full page on hard navigation, so
those links land somewhere real; relation and mapping tables get `scope="col"` headers.

---

## 11. Test impact

| Ring | Baseline | Change |
|---|---|---|
| `cargo test -p helios-hts-ui` | 80 passing | new routes extend [route_enum.rs](../../crates/hts-ui/tests/route_enum.rs) (`locale × HX-Request` matrix) |
| Playwright (`e2e`, **fixtures**) | 75 / 0 / 3 skipped | `no-cdn.spec.ts` must stay green |
| axe-core | **absent** | add `e2e/tests/a11y.spec.ts` (§7) |
| Fluent parity | enforced | every new key in all three catalogs |

Seed split: **official seeds** for design input and manual walkthrough; **e2e fixtures**
(`e2e/seed.mjs`) stay the deterministic basis for the automated rings.

---

## 12. Handoff: Claude Design → Claude Code

A **"Helios HFS" design system** now exists in Claude Design (21 files, 17 component cards,
built from the real `app.css` — colours, type, surfaces, data tables, status tags, controls,
stats, chart, lint). Selecting it in the app's `Design system` dropdown makes every generated
mockup on-brand by construction.

**Implementation should be done in Claude Code, not exported from Claude Design.** Reasons:

1. **Format.** The app emits standalone HTML/CSS; the target is Askama — `{% extends %}`,
   `{% match %}`, `{% if let Some(x) = … %}` — compiled against Rust view structs.
2. **No string may be literal.** `crates/ui/README.md` forbids prose in templates; a mockup is
   literal text by definition.
3. **The behaviour is invisible in a mockup.** `hx-get`/`hx-target`/`hx-swap`/`hx-select-oob`,
   `<noscript>` fallbacks, `aria-live`, the `HX-Request` full-page-vs-fragment split — none of
   it survives an export, and all of it is tested.
4. **Generated CSS would violate the core constraint.** New markup arrives with new class names
   and its own rules; the entire point is to consume `app.css`'s existing vocabulary.
5. **The tests live in the repo.**

So: Claude Design owns the **decisions** in §3 and §4 and the images in this document. Claude Code owns
everything from §5 onward. The design system is the durable artifact; the mockups are
disposable.

Note `DesignSync` reads and writes *design-system* projects only — mockups made in an ordinary
Design project cannot be read back through it. Export PNGs or use the app's `</>` view.

---

## 13. Open questions

1. **Which direction** (§4), and **which variant per page** (§3). Everything downstream depends on it.
2. **Does the chart ship in this pass or its own?** It is independent of the concept plane and
   touches different files, so it can run in parallel — but it is the only item here that adds
   in-memory state (a `/metrics` sampling ring on `HtsUiState`).
3. **`.hts-vs-workbench__tree`** is the one surface with no existing primitive. Accept ~6 new
   lines, or render the tree as an indented `.data-table` and spend zero?
4. **Should `#543` land first?** The duplicate-block hazard in §3.3 is real but not blocking;
   this plan assumes it does not.

---

## 14. Deferred — Home V3's "Loaded content" and "Recent imports" tables

**Status: NOT IMPLEMENTED. Deferred by the user on 2026-08-27.** Both tables need a design
approved by the user's team and an explicit go-ahead before any `crates/hts` code is written.
Nothing in this section has been built. It is recorded so the decision can be made later on
evidence rather than re-derived.

**What ships instead:** Home V3 as deployed is tiles + request-rate chart. The two data tables
below are the only elements that distinguished V3 from V2, so **the shipped Home is in practice
V2**. That is a known and accepted gap, not an oversight — see §4's Home row.

Everything below was measured against the live official-seed server on 2026-08-27:
**445,577 concepts across 1,977 code systems** (1,975 from the seed set plus 2 created during
the CRUD verification), SQLite backend.

### 14.1 What the mockup asked for

From `screens/hts-variants/{light,dark}/home-v3.png`:

| Card | Columns in the mockup | Footer |
|---|---|---|
| **Loaded content** | Code system · Canonical URI · Version · Concepts | "Top 8 by concept count · HL7 Terminology (THO) supplies 1,842 of the 1,975 loaded systems." |
| **Recent imports** | Source · Content · Concepts · Finished · Result | an error line + "Import log" button |

### 14.2 Verdict

| Table | Verdict | Why |
|---|---|---|
| **Loaded content** | **Implementable, low risk** | Every column exists in `concepts` + `code_systems`. **Zero schema migration.** |
| **Recent imports** | **Not implementable as mocked** | 3 of its 5 columns do not exist, and creating them forces a re-import on existing deployments. |

### 14.3 Loaded content — the evidence

The ranking query, in **two steps** rather than one:

```sql
-- 1. rank on the bare table
SELECT system_id, COUNT(*) AS n FROM concepts GROUP BY system_id ORDER BY n DESC LIMIT 8;
-- 2. fetch metadata for those 8 ids only
SELECT id, name, url, version FROM code_systems WHERE id IN (?,?,?,?,?,?,?,?);
```

| Approach | Time (best of 3, warm) |
|---|---|
| Single `JOIN` + `GROUP BY` over everything | 330 ms |
| `GROUP BY` alone, no join | 26 ms |
| **Two-step (above)** | **26–29 ms** |

The JOIN is **12× more expensive for an identical result**. Extrapolating the two-step linearly,
a 2M-concept deployment costs ~131 ms — acceptable, and cacheable if it ever is not.

Measured facts:

- **1,963 of 1,977** code systems have at least one concept; 14 have zero. A top-8 is unaffected.
- The footer stat is cheap: THO supplies **1,685 of 1,977** systems, computed in **0.3 ms** via
  `COUNT(*) WHERE url LIKE 'http://terminology.hl7.org%'`.
- `CodeSystem.count` (FHIR: *"Total concepts in the code system"*) already has a **live consumer**:
  `upstream.rs:568` reads `.get("count")` and `cs-detail.html:90` / `:149` render it. It shows
  `&mdash;` today only because HTS never sends it. Populating it is useful even if this card never ships.

Why the risk is low:

1. **No schema change.** The data is in tables that already exist and are already populated.
   Rollback is `git revert`, with nothing to undo in any database.
2. **Both backends have identical schemas** — same `concepts` table, same `system_id` column,
   verified in `backends/sqlite/schema.rs` and `backends/postgres/schema.rs`.
3. **Exactly two implementations of the trait**, both in-repo, **no mock backends in tests**
   (`grep 'impl .*TerminologyMetadata for'` → 2 hits). The compiler forces both to be covered.
4. **`cargo check -p helios-hts --features postgres` compiles cleanly on the dev machine**
   (verified 2026-08-27, 1m40s). The Postgres half can be compile-verified without a database.
5. **The pattern already exists.** `PostgresTerminologyBackend::supported_systems()` is the same
   shape — a sync trait method wrapping an async query in `block_in_place` + `block_on`.

### 14.4 Loaded content — implementation steps

| # | File | Change |
|---|---|---|
| 1 | `crates/hts/src/traits/metadata.rs` | Add `fn concept_inventory(&self, limit: usize) -> Vec<CodeSystemInventory>` to `TerminologyMetadata`, plus the struct (`url`, `name`, `version`, `concepts`). **Not** on `TerminologyBackend` — that has a blanket impl and could never be overridden per backend (see the note at `traits/mod.rs:56`). |
| 2 | `crates/hts/src/backends/sqlite/mod.rs` | Implement with the two-step query above. |
| 3 | `crates/hts/src/backends/postgres/mod.rs` | Same, following `supported_systems()`'s `block_in_place`/`block_on` pattern verbatim. |
| 4 | `crates/hts/src/operations/` | New read-only handler returning `Parameters`. |
| 5 | `crates/hts/src/server.rs` | One **additive** route beside `/health` and `/metadata`. No existing route or payload changes. |
| 6 | `crates/hts-ui/src/upstream.rs` | Fetch + parse into a view struct. |
| 7 | `crates/hts-ui/templates/partials/` | New partial rendered as a `.data-table`. **Zero new CSS** — every class already has a rule. |
| 8 | `crates/hts-ui/locales/{en,es,de}/main.ftl` | New keys ×3 locales; the parity test fails the build on drift. |
| 9 | `crates/hts-ui/tests/` | Coverage for the handler, the parser, and the empty state. |

**Why an HTTP endpoint and not a direct call:** `HtsUiState` holds only an `UpstreamClient`, never
a backend handle. That decoupling is deliberate and documented at `server.rs:102` — it lets
`HTS_UI_UPSTREAM_URL` point the UI at a *remote* HTS without a rebuild. A UI pointed at an older
HTS gets a 404, which the existing degraded-state contract (`hts-degraded.html`) already handles.

**Verification gates:** `cargo check -p helios-hts --features postgres` · `cargo test -p helios-hts-ui`
· live boot against the official seed set · Fluent parity.

### 14.5 Recent imports — why it is blocked

`bootstrap_imports` has six columns and **only two** of the mockup's five map onto it:

| Mockup column | Available? | Source |
|---|---|---|
| Source | **yes** | `path` |
| Finished | **yes** | `imported_at` |
| Content (which code system the file produced) | **no** | no provenance link exists |
| Concepts | **no** | `ImportStats.concepts` is computed at import time and discarded |
| Result | **no** | the ledger records successes only (`main.rs:866` writes inside `Ok(..)`) |

The blocker is not the columns — it is what adding them costs. The repo **already has the
precedent**: `authority_rank` is a provenance column added to `code_systems` / `value_sets`, and
its migration comment states the constraint plainly:

> `authority_rank` … cannot be derived after the fact — **nothing already stored on the row says
> which package supplied it** — so a database that predates the column must re-import its packages
> to learn the truth.

Because the ledger skips any file whose size and mtime are unchanged, a new column would sit at
its `DEFAULT` forever on every existing server. The existing migration therefore has to do
`DELETE FROM bootstrap_imports WHERE path LIKE '%.tgz'` to **force a re-import on next startup**.

Two consequences that make this a genuine rollback risk:

1. **Every existing deployment re-imports on upgrade** — 151 MB for the standard seed set.
2. **The multi-GB archives are deliberately excluded** from that invalidation (SNOMED / LOINC /
   RxNorm, "nothing to learn from a reload"). So for the *largest* imports the new columns would
   stay empty regardless — the card would render em-dashes in exactly the rows an operator most
   wants to see.

### 14.6 Recent imports — steps, if the team approves it anyway

1. Add `content_summary`, `concepts`, `result` columns to `bootstrap_imports` in **both**
   `backends/sqlite/schema.rs` and `backends/postgres/schema.rs`, with idempotent
   `ALTER TABLE … ADD COLUMN IF NOT EXISTS` migrations matching the existing style.
2. Thread `ImportStats` through to the ledger write at `crates/hts/src/main.rs:866` instead of
   discarding it.
3. Record failures: the write currently lives inside the `Ok(file_stats)` arm of
   `match dispatch_import(...)`; a failed import must also leave a row.
4. Decide the ledger-invalidation policy — forced re-import (per the `authority_rank` precedent)
   or accept permanently-empty columns on pre-existing rows.
5. Then steps 4–9 of §14.4.

**One runtime trap to design around.** `imported_at` is `TEXT` in SQLite and `TIMESTAMPTZ` in
Postgres. A `row.get::<_, String>("imported_at")` **compiles cleanly and panics at runtime** on
Postgres. Cast in SQL (`imported_at::text`) so both backends return the same Rust type.

**User decision, 2026-08-27:** failures are **not** to be surfaced in the UI. That removes the
`result` column from the card, but not from this analysis — without it "Recent imports" cannot
distinguish a completed import from one that died halfway.

### 14.7 A zero-migration alternative for the same slot

`code_systems.created_at` is populated and usable: **1,977 distinct timestamps over 1,977 rows**,
sub-millisecond ordering. A card titled *"Recently loaded"* — code system · canonical URI ·
version · concepts, ordered by `created_at DESC` — fills the same visual slot with four real
columns, **no schema migration, and no forced re-import**. It answers "what arrived recently?"
rather than "which files were processed?", which is arguably the question an operator has.

Recorded as an option, **not** approved.

### 14.8 One thing to expect if this ever ships

The real table will not match the mockup. The mockup draws ICD-9-CM with **14,567** concepts; the
live seed set has **1** — the silent import failure already noted in the execution plan's unowned
findings. Surfacing it on the home page is arguably the point, but it should be an expected
outcome rather than a surprise at review.
