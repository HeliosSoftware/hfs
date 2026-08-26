# HTS Search & Forms UX proposal

Read-only advisor deliverable for the three HTS resource browsers (CS / VS / CM)
and their filter forms — the surfaces at `/ui/hts/code-systems`,
`/ui/hts/value-sets`, `/ui/hts/concept-maps`. No source file was modified as
part of authoring this proposal; scope ends at this document.

Composes with:

- [`.claude/skills/work-with-ui/SKILL.md`](../../.claude/skills/work-with-ui/SKILL.md) — architecture rules (Askama + htmx, no SPA, no CDN, WCAG 2.2 AA, Fluent i18n, `nojs` degradation, no browser-facing JSON API).
- [`.claude/skills/frontend-design/SKILL.md`](../../.claude/skills/frontend-design/SKILL.md) — HFS design discipline (subject-first, restraint, tokens over hex, avoid AI-defaults).
- [`.claude/skills/work-with-hts/SKILL.md`](../../.claude/skills/work-with-hts/SKILL.md) — HTS runtime and API surface.
- [`.claude/skills/hts-api-skill/SKILL.md`](../../.claude/skills/hts-api-skill/SKILL.md) — HTS HTTP contract.
- [`edson/docs/hts-ui-design.md`](hts-ui-design.md) §7.2 / §7.4 / §7.5 / §9.2 — page tree, wireframes, and shared-chrome extraction plan.
- [`edson/docs/hts-ui-dashboard-ux-proposal.md`](hts-ui-dashboard-ux-proposal.md) — sibling proposal (`/ui/hts` dashboard); this document adopts the same voice, class-map format, and CSS-budget discipline.

Locked decisions from the ask (not re-opened):

- Case-insensitive is always ON (spec-aligned, no toggle).
- Match-mode toggle offers `exact | starts-with | contains`, **default = contains**.
- FHIR version scope = `compile_multi_ui_switch` (additive Cargo features + topbar switcher sending `Accept: application/fhir+json; fhirVersion=X.X`). Descope-able at Gate 1 — an honest S/M/L estimate appears in §6.

---

## 1. Diagnosis

### 1.1 Current filters, columns, and gaps

| Browser | Filter fields (accepted) | Columns rendered | Gaps |
|---|---|---|---|
| **CodeSystem** | `url`, `version`, `name`, `title`, `status` — `crates/hts-ui/templates/pages/cs-browser.html:26-73`, wired to [`BrowserForm`](../../crates/hts-ui/src/code_systems.rs) at `crates/hts-ui/src/code_systems.rs:94-108` | `URL`, `Version`, `Title`, `Status` — `crates/hts-ui/templates/partials/hts-cs-rows.html:35-40` | `name` is filterable but has **no column**; the title cell hides it behind a `title → name → id` fallback at `crates/hts-ui/templates/partials/hts-cs-rows.html:57-60`, so the operator cannot tell whether they are looking at a title or a synonymized name. `URL` renders raw canonical inside `<code>`, so long URLs push every neighbour column narrower. |
| **ValueSet** | `url`, `version`, `name`, `title`, `status` — `crates/hts-ui/templates/pages/vs-browser.html:26-72`, wired at `crates/hts-ui/src/value_sets.rs:82-95` | `URL`, `Version`, `Title`, `Status` — `crates/hts-ui/templates/partials/hts-vs-rows.html:33-38` | Same as CS: `name` filterable, no column; same `title → name → id` collision at `crates/hts-ui/templates/partials/hts-vs-rows.html:55-58`. |
| **ConceptMap** | `url`, `name`, `title`, `source`, `target`, `status` — `crates/hts-ui/templates/pages/cm-browser.html:26-71`, wired at `crates/hts-ui/src/concept_maps.rs:101-116` | `URL`, `Version`, `Title`, `Status` — `crates/hts-ui/templates/partials/hts-cm-rows.html:37-42` | Same missing `Name` column. **Also silently drops `source` and `target` server-side** — the row projection captures `source_uri` / `target_uri` at `crates/hts-ui/src/upstream.rs:2233-2234` and `crates/hts-ui/src/upstream.rs:2250-2257`, but nothing renders them and the search itself never filters by them (see §1.3). |

### 1.2 The CSS-nothingness of the current BEM tree

Every class the three row partials emit under the `.hts-{cs,vs,cm}-browser__*`
namespace has **zero matching rules anywhere in the workspace**. Verified: `rg
'hts-cs-browser__|hts-vs-browser__|hts-cm-browser__' crates/ui/assets/app.css`
returns nothing. Concretely:

- `.hts-cs-browser__filters`, `.hts-cs-browser__field`, `.hts-cs-browser__actions`
  in `crates/hts-ui/templates/pages/cs-browser.html:18-73` — none styled. The
  form collapses to browser defaults: each `<div class="hts-cs-browser__field">`
  is a block-level element, so five filter groups stack vertically, each label
  above its input, running roughly 300 px of vertical space before the Search /
  Reset buttons appear. On a 1440×900 laptop the first data row is below the
  fold.
- `.hts-cs-browser__rows`, `.hts-cs-browser__table`, `.hts-cs-browser__row-link`,
  `.hts-cs-browser__url`, `.hts-cs-browser__pager`, `.hts-cs-browser__empty`,
  `.hts-cs-browser__count` in `crates/hts-ui/templates/partials/hts-cs-rows.html:11-101`
  — none styled. The table renders through the *unrelated* `.data-table` class
  that the same `<table>` also carries (line 32), which is the sole reason the
  results look presentable at all.
- The `<code class="hts-cs-browser__url">` at
  `crates/hts-ui/templates/partials/hts-cs-rows.html:53` inherits nothing beyond
  the browser's default monospace, so long canonicals overflow horizontally and
  push `Title` and `Status` off-screen before the media query fires.
- The same nothingness holds for the VS mirror
  (`crates/hts-ui/templates/partials/hts-vs-rows.html`) and the CM mirror
  (`crates/hts-ui/templates/partials/hts-cm-rows.html`).

The row partials do reach the shared `.data-table` (`crates/ui/assets/app.css:1928-1985`),
`.pill` for the status badge, and the workspace's shared `.btn` /
`.btn--primary` / `.btn--ghost` on the form actions — but there is no
`.filter-layout` frame, no `.filter-rail`, no sticky detail column, and no
facet chip strip. The three pages therefore look like *scaffold pages*, not
like HFS's [SearchParameters](../../crates/ui/templates/pages/search-parameters.html)
or Compartments viewers.

### 1.3 Backend is spec-drifted: exact `=`, case-sensitive, CM source/target silently dropped

- SQLite CS `search` — `crates/hts/src/backends/sqlite/code_system.rs:1167-1171`
  and `crates/hts/src/backends/sqlite/code_system.rs:1220-1224`:
  `WHERE (?1 IS NULL OR url = ?1) AND (?2 IS NULL OR version = ?2) AND (?3 IS NULL OR name = ?3) AND (?4 IS NULL OR title = ?4) AND (?5 IS NULL OR status = ?5)`.
  Exact `=` comparison with default `BINARY` collation — case-sensitive. FHIR
  R4 §Search specifies string search parameters default to case-insensitive
  match-starts-with, so this is a spec-conformance bug in every CS row search.
- SQLite VS mirror — same shape at
  `crates/hts/src/backends/sqlite/value_set.rs:2113-2117` and `crates/hts/src/backends/sqlite/value_set.rs:2166-2170`.
- SQLite CM — `crates/hts/src/backends/sqlite/concept_map.rs:85-89`:
  `WHERE (?1 IS NULL OR url = ?1) AND (?2 IS NULL OR version = ?2) AND (?3 IS NULL OR name = ?3) AND (?4 IS NULL OR title = ?4) AND (?5 IS NULL OR status = ?5)`.
  **No `source_uri` or `target_uri` clause at all.**
- Postgres CM mirror — `crates/hts/src/backends/postgres/concept_map.rs:232-238`:
  same five-clause `WHERE` with the same missing source/target predicates. Uses
  `= $1`, which is also case-sensitive on Postgres `text`.
- The typed query shape that reaches the SQL is
  [`ResourceSearchQuery`](../../crates/hts/src/types.rs) at
  `crates/hts/src/types.rs:498-518`: only `url`, `version`, `name`, `title`,
  `status`, `_count`, `_offset`, `_summary`. **No `source_uri`, no
  `target_uri`, no modifier vocabulary.**
- The UI already tries to forward CM source/target at
  `crates/hts-ui/src/upstream.rs:2549-2557` (as the FHIR-canonical
  `source-uri` / `target-uri` search params), so the wire is correct — axum's
  `Query<ResourceSearchQuery>` extractor silently discards the two keys before
  the SQL sees them.

Net effect: the ConceptMap filter form advertises Source and Target inputs
that do nothing, and every filter across the three browsers behaves as strict
case-sensitive `=` rather than the case-insensitive starts-with the FHIR spec
prescribes.

---

## 2. Design principles applied

Drawn from [`.claude/skills/frontend-design/SKILL.md`](../../.claude/skills/frontend-design/SKILL.md):

- **Subject-first.** The operator's first question on these pages is *"is the
  canonical URL I'm looking for in the catalog?"*, not *"give me the whole
  registry sorted alphabetically."* The Name column moves to first position;
  the URL column narrows to a `<code>`-monospace slug and truncates gracefully.
  Status is a chip strip above the table so the operator scans values before
  typing a URL.
- **Structure encodes information.** Filters that share a spatial group are
  filters that share a semantic group. Field-level match-mode toggles are
  co-located with each input rather than gathered into a single form-level
  control, because FHIR modifiers *belong* to a parameter (§4 defends this).
- **Restraint / spend boldness once.** Only one place carries a colour cue —
  the active row's `.tag--stored` / `.tag--member` status pill, inherited
  verbatim from `crates/ui/assets/app.css:2030-2034`. Everything else is
  neutral surface + `--muted` labels + `--text` values.
- **Tokens over hex.** No new colour or radius primitives are introduced.
  Every reused primitive is already tuned for both themes in
  `crates/ui/assets/app.css:37-94` and re-verified by the axe gate in
  `crates/ui/e2e`.
- **Copy is design material.** Because case-insensitive is locked ON, no visible
  string ever mentions it — the mode select shows *what* is being matched
  ("contains" / "starts with" / "exact"), never *how* casing is handled.

### The one aesthetic risk

**Move the filter form into a sticky left rail (`.filter-rail`) rather than a
top strip.** The four HFS pages that ship the same "browse + filter"
grammar — search-parameters, compartments, queries, tenants — all put filters
in the top of the content area or in a rail-with-picker, never in a rail-as-form.
This proposal is the first to use `.filter-rail` as a *stacked
field form*.

*Justification.* CS/VS/CM catalogs are dense long-tail collections — hundreds
of code systems for a boot-strapped HL7 catalog is normal, and the operator
scrolls through results while iterating on the filter. A top strip forces
scrolling past the form to see the results, then back up to refine. The rail
keeps the filter in constant peripheral vision and matches the design doc §7.2
wireframe's implicit "filter card next to results table" layout without
inventing a new primitive. The rail's chrome (`.filter-rail__search`,
`.filter-rail__heading`) already accepts inputs and headings, so the reuse is
literal, not analogical.

---

## 3. Form layout (all 3 browsers, unified)

### 3.1 Class map (current → HFS shared primitive)

Applies identically to CS, VS, and CM; the only difference is the field list
(§3.3).

| Current (all three pages) | Proposed | Notes |
|---|---|---|
| `<section class="hts-cs-browser">` | `<section class="content--wide">` on the page body, then `<div class="filter-layout filter-layout--two">` wrapping rail + center | `content--wide` at `crates/ui/assets/app.css:1715-1717` un-caps the max width, and `.filter-layout--two` at `crates/ui/assets/app.css:1739-1741` is the same 280 px + 1fr grid the Compartments page uses. A third right column is not proposed — HTS detail lives on its own route (design doc §7.3 / §7.4 / §7.5). |
| `<header class="page-header">` + `<h1>` + `<p class="page-header__subtitle">` | `<section class="page-head">` + `<h1 class="page-head__title">` + `<p class="page-head__lede">` | The shared `.page-head` primitive is what search-parameters.html and compartments.html use; class rules already exist for it. |
| `<form class="hts-cs-browser__filters">` | `<aside class="card filter-rail" aria-label="{{ i18n.t("hts-search-rail-label") }}">` containing a single `<form method="get" ... hx-get="...">` | `.filter-rail` (`crates/ui/assets/app.css:1767-1775`) is sticky at `top: 20px` and capped at `calc(100vh - 110px)`. The htmx contract from `crates/hts-ui/templates/pages/cs-browser.html:20-24` is preserved verbatim inside the rail. |
| `<div class="hts-cs-browser__field">` × N | `<div class="filter-rail__field">` per field, each containing `<label class="filter-rail__field-label">`, then `<div class="filter-rail__field-row">` with a mode `<select>` + text `<input>` | `.filter-rail__field*` are the three (≤ 6-line) additions to `app.css` justified in §8. Field row reuses `.builder-row__value` (`crates/ui/assets/app.css:2486-2491`) for the input skin so no duplicate input CSS is added. |
| `<select id="filter-status">` | `.facets` chip strip in the center column: `<section class="card facets">` with `.chip` links per status | Reuses the exact primitives from `crates/ui/templates/pages/search-parameters.html:75-82`; `.chip` at `crates/ui/assets/app.css:1892-1920`. `status` is a small closed vocabulary (`draft`, `active`, `retired`, `unknown`, plus "All"), so chips beat a `<select>` on scannability and touch-target size. |
| `<div class="hts-cs-browser__actions">` | `<div class="filter-rail__actions">` inside the rail form, `.btn.btn--primary` for Search, `.btn` for Reset | Sticky at the bottom of the rail so long field lists (CM) don't push the actions off-screen. |
| `<div class="hts-cs-browser__rows">` and `<table class="data-table hts-cs-browser__table">` | `<section class="card">` around `<div class="table-wrap"><table class="data-table" ...>` | `.table-wrap` (`crates/ui/assets/app.css:1924-1926`) is the `overflow-x: auto` primitive that makes horizontal scroll work on narrow viewports for the CM 6-column table (§5). |
| `<div class="hts-cs-browser__pager">` | `<div class="table-foot">` with a `<button class="btn btn--secondary">` for Load-more | `.table-foot` (`crates/ui/assets/app.css:1977-1986`) already renders the Showing-N caption + button pair with the correct baseline. |
| `<span class="pill pill--{status}">` in rows | Keep verbatim | `.pill--*` already themed. |
| `<code class="hts-cs-browser__url">` | `<span class="url">` in a dedicated URL cell | `.url` (`crates/ui/assets/app.css:1970-1975`) is the shared monospace-small style, with `overflow-wrap: anywhere` so long canonicals wrap inside the cell rather than blowing out the row width. |

Everything above the class-map dividing line is inline in `pages/cs-browser.html`
today; nothing needs a new partial. VS and CM apply the same swaps
mechanically.

### 3.2 ASCII wireframe (representative — CS at ≥ 1250 px)

```
+------------- .page-head ----------------------------------------------------+
| CodeSystems                                        [ + New CodeSystem ]      |
| Browse the terminology server's catalog...                                    |
+-----------------------------------------------------------------------------+

+-- .filter-rail (sticky, 280px) --+  +-- .filter-center --------------------+
|                                  |  |                                     |
| SEARCH FILTERS                   |  | STATUS                              |
|                                  |  | [All 152] [draft 3] [active 141]    |
| Name                             |  | [retired 6] [unknown 2]             |
| [contains v] [                 ] |  |                                     |
|                                  |  +-- .card --------------------------- +
| Title                            |  | .data-table (5 columns)             |
| [contains v] [                 ] |  | Name       | Title      | URL       |
|                                  |  |            |            | Version   |
| Canonical URL                    |  |            |            | Status    |
| [starts-w v] [                 ] |  |------------+------------+-----------|
|                                  |  | loinc      | LOINC ...  | http://l… |
| Version                          |  |            |            | 2.77 · [Ac]
| [exact    v] [                 ] |  | snomed-ct  | SNOMED ... | http://s… |
|                                  |  |            |            | 20240401 · |
| ─────────────────────────────    |  |            |            |         [Ac]|
| [ Search ]        [ Reset ]      |  | ...                                 |
+----------------------------------+  | Showing 25 · [ Load more ]          |
                                      +-------------------------------------+
```

Notes on the sketch:

- The rail's `hx-boost="true"` from the compartments page (`crates/ui/templates/pages/compartments.html:23`) is inherited so field changes still fire the debounced htmx swap on the results table. The `hx-trigger="input changed delay:300ms, submit"` on the form node is preserved from `crates/hts-ui/templates/pages/cs-browser.html:24`.
- Status chips are anchors that append `?status=active` to the current URL (with the other filters preserved via hidden fields), same pattern as `crates/ui/templates/pages/search-parameters.html:69-73` — no JavaScript needed.
- The results card is not sticky (unlike `.detail` from the SP page); the rail is sticky, the table scrolls.

### 3.3 Field list per browser

| Browser | Rail fields (top to bottom) | Center facet chips |
|---|---|---|
| CS | Name, Title, Canonical URL, Version | Status × 4 + All |
| VS | Name, Title, Canonical URL, Version | Status × 4 + All |
| CM | Name, Title, Canonical URL, Source system (URI), Target system (URI) | Status × 4 + All |

CM deliberately drops `Version` (rail); the design doc §7.5 already notes HTS
ignores CM version on `$translate` and does not surface it in the browser
facet either (`edson/docs/hts-ui-design.md:1253-1258`).

### 3.4 Responsive

Only the breakpoints HFS already ships:

- `@media (max-width: 1250px)` — `crates/ui/assets/app.css:2404-2413` collapses
  `.filter-layout` to a single column and drops rail + detail stickiness. The
  rail becomes a card *above* the results table on tablets and 13-inch
  laptops. No new media query.
- `@media (max-width: 1100px)` — `crates/ui/assets/app.css:2652-2656` is
  scoped to `.queries-layout` today; not touched.
- `@media (max-width: 900px)` — `crates/ui/assets/app.css:2619-2627` is
  scoped to `.builder-grid`; not touched. At this width the CM 6-column
  table falls into `overflow-x: auto` via `.table-wrap`, which is the
  intentional degradation.

### 3.5 Rail vs top-strip — decision

**Rail wins.** Reasons:

1. Filter density: CS/VS carry 4 fields with per-field match mode (8 controls);
   CM carries 5 fields (10 controls). A top strip would either wrap onto three
   rows or push the results below the fold on the 1440-px design target.
2. Interaction cadence: browsers are refined iteratively — type, scan
   results, refine. A sticky rail keeps every input in the periphery during
   the scan.
3. Grammar reuse: `.filter-rail` is the sole HFS sticky-sidebar primitive.
   Introducing a fifth "top-strip form" layout for HTS would fragment the
   shared vocabulary #543 is trying to consolidate (`edson/docs/hts-ui-design.md:2322-2358`).
4. Cost: rail is a `.filter-layout--two` grid — the compartments page proves
   the geometry works with a form-shaped rail (`crates/ui/templates/pages/compartments.html:23-35`).

---

## 4. Match-mode toggle

### 4.1 Placement decision: per-field toggle

**Per-field** — a `<select>` sits next to each `<input>`.

*Recommendation and defence.* Different fields legitimately need different
match modes and the operator forms this intuition per field:

- Canonical `url` typically wants `starts-with` — canonicals are hierarchical
  (`http://loinc.org/vs/…`), so starts-with is the natural narrowing verb.
- `version` typically wants `exact` — versions are compared as opaque tokens.
- `name` / `title` typically want `contains` (the default) — the operator has
  a partial substring in mind, not a prefix.
- CM `source` / `target` want `starts-with` — same canonical-URL logic as
  main `url`.

A single form-level toggle forces "contains" on `version` (surfacing
`2.77-beta-3` when the operator typed `2.77`) or forces "exact" on `title`
(making the operator paste the entire "LOINC 2.77 Laboratory Codes" string).
Either state is worse than typing one selector once per field.

### 4.2 Concrete HTML shape (Askama-friendly)

Per field, inside the rail form. Reuses `.builder-row__modifier` (`crates/ui/assets/app.css:2481-2484`)
for the mode select and `.builder-row__value` (`crates/ui/assets/app.css:2486-2491`)
for the text input — both already have the correct height, radius, and focus
ring:

```html
<div class="filter-rail__field">
  <label class="filter-rail__field-label" for="filter-name">
    {{ chrome.i18n.t("hts-cs-browser-filter-name") }}
  </label>
  <div class="filter-rail__field-row">
    <select class="builder-row__modifier"
            name="name:mode"
            aria-label="{{ chrome.i18n.t_arg("hts-search-match-label-of",
                                             "field",
                                             chrome.i18n.t("hts-cs-browser-filter-name").to_string()) }}">
      <option value="contains" selected>{{ chrome.i18n.t("hts-search-match-contains") }}</option>
      <option value="starts-with">{{ chrome.i18n.t("hts-search-match-startswith") }}</option>
      <option value="exact">{{ chrome.i18n.t("hts-search-match-exact") }}</option>
    </select>
    <input class="builder-row__value" id="filter-name" type="text"
           name="name"
           value="{% if let Some(n) = view.filters.name %}{{ n }}{% endif %}"
           autocomplete="off" spellcheck="false">
  </div>
</div>
```

Askama constraint: the `{% match %}` gymnastics from
`crates/hts-ui/templates/pages/cs-browser.html:44-67` (status `<select>`) do
not apply — no option needs to be compared against `view.filters.<field>`
because the `selected` attribute follows the *mode* not the *value*, which
lives on `view.filters.<field>_mode`.

### 4.3 URL contract: proper FHIR modifier

**Send `?name:contains=foo`, not `?name=foo&_matchmode=contains`.**

*Defence.* HTS is a FHIR terminology server. The `:contains` modifier is
already the HL7-defined vocabulary for string search parameters (FHIR R4
§Search.Modifiers). Using it here:

- Aligns the browser's URL with what any conformant FHIR client sends. An
  operator who bookmarks a filtered browser URL, or copies it into a shell
  `curl`, gets a spec-legible request rather than a UI-specific hybrid.
- Keeps `crates/hts/src/types.rs:498-518` on a spec-parallel path — the parser
  learns modifiers, not a new sibling `_matchmode` field that has no meaning
  outside the browser.
- Composes with future modifiers HTS may add (`:not`, `:missing`) without
  another schema knob.
- Consequence: `contains` (the default) is sent *bare* (`?name=foo`), so URLs
  stay clean in the common case. Only non-default modes stamp the modifier.

The rail's hidden `<input>` used to preserve state across facet-chip clicks
therefore carries `name` and `name:mode` as two separate keys — the
mode-to-URL mapper collapses `name:mode=contains` back to no modifier before
building the outbound href.

### 4.4 Fluent keys

Defined once, reused across CS/VS/CM (no per-browser copy).

| Key | en | es | de |
|---|---|---|---|
| `hts-search-match-label` | Match mode | Modo de coincidencia | Übereinstimmungsmodus |
| `hts-search-match-label-of` | Match mode for { $field } | Modo de coincidencia para { $field } | Übereinstimmungsmodus für { $field } |
| `hts-search-match-contains` | contains | contiene | enthält |
| `hts-search-match-startswith` | starts with | comienza con | beginnt mit |
| `hts-search-match-exact` | exact | exacta | genau |

Default position: `contains` (locked). See §9 for the full key delta.

---

## 5. Column parity strategy per browser

### 5.1 Table

| Browser | Current cols | Filters | Gaps | Proposed cols | Strategy |
|---|---|---|---|---|---|
| **CS** | URL · Version · Title · Status | url · version · name · title · status | `name` invisible; title cell has `title→name→id` fallback so column identity is ambiguous | **Name · Title · URL · Version · Status** | Add `Name` first; URL demoted to third; row anchor moves from Title cell to Name cell |
| **VS** | URL · Version · Title · Status | url · version · name · title · status | same as CS | **Name · Title · URL · Version · Status** | Same as CS |
| **CM** | URL · Version · Title · Status | url · name · title · source · target · status | `name`, `source`, `target` all invisible; source/target additionally dropped server-side (§1.3) | **Name · Title · URL · Source · Target · Status** | Version dropped (see §3.3); Source and Target render as `.tag--param` monospace chips |

### 5.2 Table-width budget (px)

Target: comfortable at ≥ 1250 px, degrades to `.table-wrap` horizontal scroll
below. Padding assumption: `.data-table td` 14 px inline (`crates/ui/assets/app.css:1945-1949`).

| Column | CS/VS width | CM width | Rationale |
|---|---:|---:|---|
| Name | 240 | 240 | Truncated with `text-overflow: ellipsis`; tooltip carries full value |
| Title | 1fr (min 260) | 1fr (min 220) | Absorbs slack |
| URL | 240 (mono) | 180 (mono) | `.url` primitive; wraps with `overflow-wrap: anywhere` |
| Version | 100 | — | — |
| Source | — | 160 | `.tag--param` chip; wraps to two lines |
| Target | — | 160 | Same |
| Status | 110 | 110 | Existing pill |
| **Sum** | ~ 950 + fluid | ~ 1070 + fluid | ≥ 1250 px viewport fits with slack; below that, horizontal scroll |

At `≤ 1250 px` the shared media query (`crates/ui/assets/app.css:2404-2413`)
collapses the rail to a stacked card above; the results card then has the
full pane width. The CM 6-column table still exceeds the pane on very narrow
viewports; `.table-wrap` handles it. This is the most conservative
degradation — no columns hidden, no click-to-reveal, no re-flow surprise.

### 5.3 Strategy choice for CM (the hardest case)

Choices considered:

1. **Horizontal scroll** via existing `.table-wrap` — **recommended**.
2. Expandable rows / `<details>` per row — rejected. Hides source/target
   behind a click; discovery UI should not require a click to answer *"does
   this map SNOMED to LOINC?"*. Also expands the a11y contract non-trivially
   (row-level `aria-expanded`, keyboard toggling, focus retention).
3. Columns conditional on active filter — rejected. Table shape changes based
   on filter state → operator has to re-learn the grid every time they
   refine.
4. Drop the extra columns — rejected. That is the diagnosed bug; the whole
   point of §1.3 is to *stop* silently dropping data.
5. Stack Source above Target in a single cell — rejected. Doubles row height,
   uneven when either is empty, and breaks the visual rhythm with CS/VS.

Recommendation: **horizontal scroll below 1250 px.** The primary desktop
target for HTS operators is 1440 px+; the sub-1250 case is the exception, not
the rule, and the primitive is already in the stylesheet.

---

## 6. FHIR version switcher

### 6.1 Topbar placement

Insert the switcher **into the HTS topbar between the dialect chip and the
language switcher** at `crates/hts-ui/templates/layouts/base.html:62-96`:

```
[dialect chip]  →  [FHIR version chip *]  →  [lang: en/es/de]  →  [theme]
```

*Defence.* On HTS specifically, FHIR version is a *data-shape dimension*, not
a user-personal preference like language or theme. It belongs adjacent to the
dialect chip (which is the other data-shape dimension, BCP-47 designation
language), not adjacent to the theme or avatar. Placing it after the dialect
chip also reads left-to-right as "language of concepts → version of concepts
→ display language of chrome" which matches the operator's mental grouping.

The current sidebar-footer `fhir-badge` at
`crates/hts-ui/templates/layouts/base.html:54-58` becomes redundant and is
removed; the topbar chip is the single source of truth.

HFS's sidebar-based selector lives at
`crates/ui/templates/layouts/base.html:124-155` — reused as a *template
pattern*, not literally, because HTS has no sidebar footer worth extending.

### 6.2 Cookie + query param

| Aspect | Value |
|---|---|
| Cookie | `hts_fhir_version` (parallel naming to `hts_lang` — `crates/hts-ui/src/i18n.rs:50`) |
| Query param | `?fhir=R4B` (short, distinct from HFS's `?version=` to avoid cross-app cookie / query collision when both binaries share a domain) |
| Persistence route | `POST /ui/hts/version` with body `version=R4B`, redirect back to the referring URL — mirrors HFS `POST /ui/version` at `crates/ui/templates/layouts/base.html:144-151` |
| Extractor | `RequestFhirVersion` on the HTS side, paralleling `RequestVersion` at `crates/ui/src/lib.rs:159-182` |
| Middleware | `resolve_hts_prefs`, paralleling `resolve_prefs` at `crates/ui/src/lib.rs:218-260` |

### 6.3 Selector element — Askama shape (nojs-safe)

Reuses HFS's `<details>`-based disclosure (`crates/ui/templates/layouts/base.html:132-154`)
verbatim in shape. Every version choice is a submit button inside its own
`<form>` — no JS required for the write:

```html
<details class="menu menu--fhir-version">
  <summary class="selector selector--outline"
           aria-label="{{ chrome.i18n.t("hts-fhir-version-label") }}">
    <span class="selector__prefix">{{ chrome.i18n.t("hts-fhir-version-prefix") }}</span>
    <code class="selector__value">{{ chrome.fhir_version }}</code>
    <span class="selector__chevrons">
      <span class="icon">{% include "icons/chevron-down.svg" %}</span>
    </span>
  </summary>
  <div class="menu__panel">
    <div class="menu__heading">{{ chrome.i18n.t("hts-fhir-version-heading") }}</div>
    {% for v in chrome.enabled_fhir_versions %}
    <form method="post" action="/ui/hts/version">
      <input type="hidden" name="version" value="{{ v }}">
      <input type="hidden" name="return_to" value="{{ chrome.request_path }}">
      <button type="submit" class="menu__option"
              {% if v == chrome.fhir_version %} aria-current="true"{% endif %}>
        {{ v }}
        {% if v == chrome.fhir_version %}
          <span class="check">{% include "icons/check.svg" %}</span>
        {% endif %}
      </button>
    </form>
    {% endfor %}
  </div>
</details>
```

Optional htmx enhancement: add `hx-boost="true"` on the whole topbar so a
submit doesn't cause a full-page reload — but the fallback (a real
`<form method="post">`) must remain the load-bearing path, per the
`nojs` Playwright project's contract in [`work-with-ui`](../../.claude/skills/work-with-ui/SKILL.md).

### 6.4 Multi-version compile refactor — honest S/M/L: **L**

Impacted crates/files, top-down:

| Layer | File(s) | Kind of edit | Cost signal |
|---|---|---|---|
| Cargo features | [`crates/hts/Cargo.toml`](../../crates/hts/Cargo.toml) L27-34 | Features are already declared as pure `helios-fhir/RX` forwards (they can technically coexist), but the *code* asserts exactly one. Docs the `default = ["sqlite", "R4"]` triple and adds `full = ["R4", "R4B", "R5", "R6"]` for the multi-version image. | S at the manifest — the L is downstream. |
| Version label | [`crates/hts/src/server.rs`](../../crates/hts/src/server.rs) L297-316 | `FHIR_VERSION_LABEL` cfg-ladder becomes a runtime enumeration returning `&'static [&'static str]`. Doc comment at L299-301 explicitly claims "features are mutually exclusive" — that assertion is retracted. | M — one file, but read-through by every handler. |
| Operation handlers | `crates/hts/src/operations/*` | Every operation currently deserializes / constructs typed FHIR resources against exactly one `helios_fhir::rX::*` module. Multi-version means either (a) dispatch on the incoming `fhirVersion=` media-type parameter and pick the right decoder, or (b) keep decoding version-agnostically (raw `serde_json::Value` for the whole path) and only pick the version when a typed emitter absolutely needs it. Option (b) is closer to the current architecture but still requires a per-handler audit. | **L** — 42 routes per [`hts-api-skill`](../../.claude/skills/hts-api-skill/SKILL.md); ~10 unique handlers. |
| Persistence adapter | `crates/hts/src/backends/{sqlite,postgres}/*` | Table schemas are version-agnostic (they store raw JSON) so the *schema* survives untouched. Any code path that decodes `resource_json` into a typed FHIR resource must fan out over the compiled-in versions. The HTS backends currently do this in a handful of places (`code_system.rs`, `value_set.rs`, `concept_map.rs`) — each needs a per-version arm. | M. |
| Persistence crate | [`helios-persistence`](../../crates/persistence) forwarding at `crates/hts/Cargo.toml:41` (`features = ["sqlite"]`) and `crates/hts/Cargo.toml:30-34` (feature forwards) | Already multi-version-capable via `SofViewDefinition::R4/R4B/R5/R6` idiom (per [`CLAUDE.md`](../../CLAUDE.md) §Version-Agnostic Abstraction). Confirm the HTS-specific storage adapter picks the tenant + version tuple, not just the tenant. Likely S. | S. |
| HTS UI crate | [`crates/hts-ui/Cargo.toml`](../../crates/hts-ui/Cargo.toml) L12-19 | Same feature-forward story; declare `full = ["R4", "R4B", "R5", "R6"]`. | S. |
| HTS UI router / state | [`crates/hts-ui/src/lib.rs`](../../crates/hts-ui/src/lib.rs) — `HtsUiState.fhir_version: &'static str` (single value today) → `enabled_fhir_versions: &'static [&'static str]` plus a `default_fhir_version: &'static str`. Add `RequestFhirVersion` extractor + `resolve_hts_prefs` middleware. | M. |
| Upstream client | [`crates/hts-ui/src/upstream.rs`](../../crates/hts-ui/src/upstream.rs) — every `.header("Accept", "application/fhir+json")` at `crates/hts-ui/src/upstream.rs:788` and the parallel operation POST paths must become `.header("Accept", format!("application/fhir+json; fhirVersion={}", version))`. Threads `RequestFhirVersion` through `search_code_systems`, `search_value_sets`, `search_concept_maps`, `read_*`, `cs_lookup`, `cs_validate_code`, `cs_subsumes`, `vs_expand`, `vs_validate_code`, `cm_translate_instance`, `cm_closure`, and `vs_batch_validate_code`. | **L** — 12+ methods, each with its own signature. |
| Templates | Every `chrome.fhir_version` in `crates/hts-ui/templates/**/*.html` becomes potentially a *list plus a selected* rather than a scalar; the sidebar footer badge (`crates/hts-ui/templates/layouts/base.html:54-58`) is removed. | S. |
| Playwright | New `crates/hts-ui/e2e/tests/fhir-version.spec.ts` (§6.6). | S. |

**S/M/L estimate: L.** Roughly 8–14 PRs across the three crates, coordinated
so the middle PR (upstream Accept header) doesn't ship without the extractor
already merged. Descope-honest: if the R4B/R5/R6 audience is one operator or
one integration test, the value doesn't justify the L. The topbar chip can be
static (compile-time version, no selector) in an interim ship and the
selector added once the persistence + operation matrix is verified against
real multi-version data.

### 6.5 Compat when the selected version has no data

**Recommend: informative empty state**, not a banner and not 404.

*Defence.* The three alternatives fail as follows:

- 404 says *this page does not exist*; but the browser route exists and is
  perfectly capable of rendering, it just returns zero rows.
- Degraded banner (`crates/hts-ui/templates/partials/hts-degraded.html`) is
  reserved for HTS unreachable / timeout / connection failure — surfacing it
  when HTS is *fine* and just empty overloads its meaning and would fire
  for any operator who has only R4 data.
- Empty state degrades gracefully. It already exists (`hts-cs-browser-empty` /
  `hts-vs-browser-empty` / `hts-cm-browser-empty`) and reads *"No CodeSystems
  match these filters."* — extend with an inline hint keyed off the version:
  `hts-fhir-version-empty-hint = This catalog is empty for the selected FHIR
  version.`, rendered only when no other filter is active *and* the effective
  version differs from the compiled-in default.

The empty state also composes with any bootstrap ledger view that might
later expose "you loaded these packages for these versions" — the operator
can pivot from empty → import without a page reload.

### 6.6 Playwright — validate the plan's 5 blocks and propose additions

The proposal's plan calls for `crates/hts-ui/e2e/tests/fhir-version.spec.ts`
with 5 blocks. Validating the shape assumed by that plan and confirming what
must be present:

| # | Block | Assertion |
|---|---|---|
| 1 | Renders the version selector with the compiled versions only | `enabled_fhir_versions` iterated; disabled versions never appear as an option; the default binary (R4 only) hides the selector entirely |
| 2 | Selecting a version persists via cookie and re-renders the current page | POST `/ui/hts/version` sets `hts_fhir_version=R4B`; the following GET carries the cookie; the topbar chip shows R4B |
| 3 | `?fhir=R4B` explicit override wins over cookie | Cookie set to R5, `?fhir=R4B` → R4B is the effective version for this render |
| 4 | Selected version is echoed in the topbar chip and in `document.documentElement` for CSS hooks | `[data-hts-fhir-version="R4B"]` present on `<html>` (optional but useful for future theming) |
| 5 | Nojs project: submitting via the real `<form method="post">` works with JavaScript disabled | axe-clean, and the redirect to `return_to` lands on the source page |

**Missing dimensions to add — three additional blocks:**

| # | New block | Reason |
|---|---|---|
| 6 | `Accept: application/fhir+json; fhirVersion={effective}` header propagation to HTS | Ensures the upstream request actually asks for the selected wire version; a UI test that only checks the *chip* would let a silently-broken accept header ship. Assert via a mock upstream in the fixture (mirror `crates/hts-ui/tests/concept_maps.rs` ready-probe pattern from `edson/docs/hts-ui-design.md:1379-1385`). |
| 7 | Empty-catalog fallback state | Switching to a version with no data renders the `hts-fhir-version-empty-hint` copy and does NOT fire the degraded banner. Guards against the ambiguity in §6.5 by *asserting* which banner does not appear. |
| 8 | `hts_fhir_version` and `hts_lang` cookies do not clobber each other | Setting one leaves the other unchanged across GET/POST; guards a class of copy-paste bugs when the two `Set-Cookie` handlers get refactored together. |

Total: 5 planned + 3 additions = 8 blocks. Keep the plan's 5 verbatim; add 6/7/8.

---

## 7. Backend surface impact (bird's-eye)

| Change | File(s) | Kind of edit | Risk |
|---|---|---|---|
| **contains + starts-with + exact modifiers** | `crates/hts/src/backends/sqlite/{code_system,value_set,concept_map}.rs` search fns; postgres mirrors at `crates/hts/src/backends/postgres/{code_system,value_set,concept_map}.rs` | Replace `column = ?N` in six WHERE clauses (CS L1167-1171 + L1220-1224; VS L2113-2117 + L2166-2170; CM L85-89) with a dispatched fragment: SQLite → `column LIKE ? ESCAPE '\' COLLATE NOCASE` with `%foo%` / `foo%` / `foo`; Postgres → `column ILIKE $N` with the same triad. Prepare-cached lookups keyed by (column, mode) so the plan cache stays small. | Low — six statements, mechanical replacement, existing tests pin exact-match semantics that mostly survive because `contains("foo")` still matches `foo` |
| **Case-insensitive by default** | Same six statements as above | SQLite `COLLATE NOCASE` on every string column comparison; Postgres `ILIKE`. Any existing index on `url` / `name` etc. that was declared `COLLATE BINARY` needs a mirror index with `COLLATE NOCASE`, or a functional index on `LOWER(col)`, otherwise the case-insensitive path table-scans. Audit `crates/hts/src/backends/sqlite/schema.rs` for `CREATE INDEX` declarations. | Medium — potential performance regression on large catalogs if the index isn't mirrored |
| **CM `source-uri` / `target-uri` filters** | [`crates/hts/src/types.rs:498-518`](../../crates/hts/src/types.rs): add `#[serde(rename = "source-uri")] pub source_uri: Option<String>` and `#[serde(rename = "target-uri")] pub target_uri: Option<String>`. SQLite `crates/hts/src/backends/sqlite/concept_map.rs:85-89`: extend WHERE with `AND (?N IS NULL OR source_uri = ?N) AND (?N+1 IS NULL OR target_uri = ?N+1)`. Postgres mirror at `crates/hts/src/backends/postgres/concept_map.rs:232-240`. | The columns already exist — Postgres schema at `crates/hts/src/backends/postgres/schema.rs:195-196` declares `source_uri TEXT, target_uri TEXT`; SQLite `CREATE TABLE concept_maps` at `crates/hts/src/backends/sqlite/concept_map.rs:545` inserts into `(id, url, version, source_uri, target_uri, status, created_at)` so the columns are already there. | Low |
| **Multi-version compile + runtime dispatch** | See §6.4 table — Cargo, `server.rs` L297-316, every operation handler, the upstream client's `Accept` header at `crates/hts-ui/src/upstream.rs:788`. | See §6.4 estimate. | **High** — the L estimate encodes the risk |

---

## 8. CSS additions

**Strict budget: ≤ 20 lines total, all in shared `crates/ui/assets/app.css`.**

The proposal reuses so many existing primitives (`.filter-layout--two`,
`.filter-rail`, `.filter-rail__search`, `.filter-rail__heading`, `.facets`,
`.chip`, `.data-table`, `.table-wrap`, `.table-foot`, `.pill`,
`.builder-row__value`, `.builder-row__modifier`, `.menu`, `.selector--outline`)
that the only genuinely new declarations are the stacked-field rail group and
the Name column truncation:

```css
/* HTS filter-rail: per-field group; input skin comes from .builder-row__value. */
.filter-rail__field { display: flex; flex-direction: column; gap: 4px; padding: 0 4px; }
.filter-rail__field-label { margin: 0 2px; font-size: 11px; font-weight: 600; color: var(--muted); }
.filter-rail__field-row { display: flex; gap: 6px; align-items: center; }
.filter-rail__field-row .builder-row__value { flex: 1; min-width: 0; }
.filter-rail__actions { display: flex; gap: 8px; padding-top: 10px; margin-top: 4px;
  border-top: 1px solid var(--surface-border); }
/* Name column: bound width so long computer-friendly names don't crowd Title. */
.data-table .col-name { max-width: 240px; overflow: hidden;
  text-overflow: ellipsis; white-space: nowrap; }
```

Count (excluding comments and blank lines): 8 rules, ~13 declaration lines.
Under budget with margin left for one small future addition without a
follow-up review.

Rule-by-rule defence:

- `.filter-rail__field` — vertical stack for label + input row; a `.filter-rail`
  child that isn't `__search` or `__list` needs a container of its own.
- `.filter-rail__field-label` — matches the visual weight of `.filter-rail__heading`
  (`crates/ui/assets/app.css:1799-1806`) but at input-label size, not
  section-header size.
- `.filter-rail__field-row` — inline flex to lay out the mode `<select>` and
  the text `<input>` side-by-side.
- `.filter-rail__field-row .builder-row__value` — override `.builder-row__value`'s
  default `flex: 1` behaviour (which is already flex-1 but assumes a wider
  parent); explicit `min-width: 0` prevents the input from establishing an
  intrinsic minimum that busts the rail width.
- `.filter-rail__actions` — separator above the Search / Reset row so the
  action pair reads as a footer to the field list.
- `.data-table .col-name` — the one column-scoped rule; keeps long computer
  names (`hl7.terminology.r4.CodeSystem-observation-category`) from consuming
  the whole width.

The FHIR-version topbar switcher does *not* need new CSS: it uses
`.menu`, `.selector--outline`, `.menu__panel`, `.menu__option`, `.menu__heading`,
and `.check` already defined for HFS's sidebar selector.

The facet chip strip does not need new CSS: `.facets` at
`crates/ui/assets/app.css:1871-1877` and `.chip` at
`crates/ui/assets/app.css:1892-1920` cover the whole surface.

---

## 9. Fluent keys required

Tabled with EXISTS / NEW markers. `[EXISTS]` keys are unchanged. `[NEW]` keys
propose en/es/de values; the operator (fluent speaker) will polish.

| Key | Status | en / es / de |
|---|---|---|
| `hts-cs-browser-title` | [EXISTS] | `crates/hts-ui/templates/pages/cs-browser.html:3` — kept |
| `hts-cs-browser-subtitle` | [EXISTS] | kept |
| `hts-cs-browser-filter-{url,version,name,title,status}` | [EXISTS] | kept as `<label>` copy |
| `hts-cs-browser-filter-{search,reset,legend}` | [EXISTS] | kept |
| `hts-cs-browser-column-{url,version,title,status}` | [EXISTS] | kept |
| `hts-cs-browser-column-name` | [NEW] | `Name` / `Nombre` / `Name` |
| `hts-vs-browser-column-name` | [NEW] | `Name` / `Nombre` / `Name` |
| `hts-cm-browser-column-name` | [NEW] | `Name` / `Nombre` / `Name` |
| `hts-cm-browser-column-source` | [NEW] | `Source system` / `Sistema de origen` / `Quellsystem` |
| `hts-cm-browser-column-target` | [NEW] | `Target system` / `Sistema de destino` / `Zielsystem` |
| `hts-search-rail-label` | [NEW] | `Search filters` / `Filtros de búsqueda` / `Suchfilter` |
| `hts-search-match-label` | [NEW] | `Match mode` / `Modo de coincidencia` / `Übereinstimmungsmodus` |
| `hts-search-match-label-of` | [NEW] | `Match mode for { $field }` / `Modo de coincidencia para { $field }` / `Übereinstimmungsmodus für { $field }` |
| `hts-search-match-contains` | [NEW] | `contains` / `contiene` / `enthält` |
| `hts-search-match-startswith` | [NEW] | `starts with` / `comienza con` / `beginnt mit` |
| `hts-search-match-exact` | [NEW] | `exact` / `exacta` / `genau` |
| `hts-facet-status-label` | [NEW] | `Status` / `Estado` / `Status` |
| `hts-facet-status-all` | [NEW] | `All` / `Todos` / `Alle` |
| `hts-facet-status-{draft,active,retired,unknown}` | Reuse existing `hts-cs-status-{value}` at `crates/hts-ui/templates/pages/cs-browser.html:54-57` | — |
| `hts-fhir-version-label` | [NEW] | `FHIR version` / `Versión FHIR` / `FHIR-Version` |
| `hts-fhir-version-prefix` | [NEW] | `FHIR:` / `FHIR:` / `FHIR:` |
| `hts-fhir-version-heading` | [NEW] | `Choose FHIR version` / `Elegir versión FHIR` / `FHIR-Version wählen` |
| `hts-fhir-version-empty-hint` | [NEW] | `This catalog is empty for the selected FHIR version.` / `Este catálogo está vacío para la versión FHIR seleccionada.` / `Dieser Katalog ist für die gewählte FHIR-Version leer.` |

Parity constraint per [`work-with-ui`](../../.claude/skills/work-with-ui/SKILL.md):
every `[NEW]` key must ship in `locales/en/main.ftl`, `locales/es/main.ftl`,
and `locales/de/main.ftl` — enforced by the key-set parity test in
`crates/hts-ui/src/i18n.rs`.

---

## 10. A11y checklist

- **Focus order** (tab sequence): topbar (dialect → FHIR version selector →
  lang → theme) → sidebar nav → rail search (Name mode, Name input, Title
  mode, …, Search, Reset) → status facet chips → results table (row links)
  → Load more. The rail's action row is deliberately last in the rail so a
  keyboard operator can tab through every field before submitting.
- **Match-mode select keyboard control**: native `<select>` — arrow keys open,
  Enter selects, Space toggles. `aria-label="Match mode for { field }"` per
  instance so a screen reader hears which field the mode belongs to.
- **Version switcher keyboard control**: `<details>` — Enter or Space on the
  summary opens the panel; arrow keys navigate options; Enter submits the
  in-panel `<form>`. Matches HFS `crates/ui/templates/layouts/base.html:132-154`.
- **Screen reader announcements**: `<tbody aria-live="polite">` is preserved
  from `crates/hts-ui/templates/partials/hts-cs-rows.html:42` so row swaps and
  Load-more appends announce as they land. The Load-more button retains focus
  after the swap per the design doc §7.2 a11y clause; that constraint is
  already met by `hx-swap="beforeend"` targeting the tbody rather than the
  button.
- **Contrast tokens used**: `var(--text)` for values, `var(--muted)` for
  labels + counts, `var(--surface-border)` for dividers, `var(--accent)` for
  focus rings, `var(--accent-soft)` for hover / selected states, `var(--ok)`
  / `var(--warn)` / `var(--danger)` inside the pre-existing `.tag--*`
  variants. All primitives already pass the axe gate in both themes per
  [`work-with-ui`](../../.claude/skills/work-with-ui/SKILL.md).
- **Empty / degraded / outcome partials**: unchanged — `hts-degraded`
  (`crates/hts-ui/templates/partials/hts-cs-rows.html:19-26`), `hts-outcome`,
  and `hts-cs-browser__empty` all keep their existing roles and copy.
- **`nojs`**: the rail form's real `<form action=".../rows" method="get">`
  survives htmx-off (kept from `crates/hts-ui/templates/pages/cs-browser.html:19-25`);
  status facet chips are anchors; version switcher submits a real form.

---

## 11. Test impact

### 11.1 Rust ring — likely to break

- **`crates/hts-ui/tests/router_http.rs`** — any assertion that greps for the
  `.hts-cs-browser__filters` / `.hts-cs-browser__field` classes needs
  updating. Assertions that read a `<label>` and count `<input>` elements
  survive because labels are preserved; assertions that count DOM elements
  by class break.
- **`crates/hts-ui/tests/code_systems.rs`** — the browser + rows fragment
  tests check form contents and column headings. Column count changes
  (`4 → 5`) and column-header i18n changes require the assertions to
  pick up the new `hts-cs-browser-column-name` key.
- **`crates/hts-ui/tests/value_sets.rs`** — same shape.
- **`crates/hts-ui/tests/concept_maps.rs`** — column count `4 → 6` plus
  new `source` / `target` column assertions. Also, the mock upstream fixture
  needs to accept `source-uri` / `target-uri` query params and return only
  matching rows to prove the wire is right.
- **`crates/hts-ui/tests/route_enum.rs`** — the matrix walk survives; the
  shell-marker walk (per `edson/docs/hts-ui-design.md:994-1004`) needs the
  new match-mode strings added to the expected translated set.
- **`crates/hts/tests/**` and per-backend tests** — any test that asserts an
  exact-match result count for a case-differing input will change. Search
  for `search_code_systems`, `search_value_sets`, `search_concept_maps` in
  the persistence backends' test modules and adjust expected sets to the
  case-insensitive-contains contract.

### 11.2 Playwright ring — likely to break

- **`crates/hts-ui/e2e/tests/code-systems.spec.ts`** — L31 `getByLabel("Canonical URL")`
  still works (the `<label>` survives). L36 `getByRole("link", { name: "Reset" })`
  still works. **L42 `getByRole("cell", { name: "http://example.org/cs", exact: true })`
  breaks** — the URL cell moves to the third position and may be truncated;
  update to target Name cell instead (`getByRole("cell", { name: "ex-cs-1" })`
  or the new Name column value). L54 `getByRole("button", { name: "Load more" })`
  survives.
- **`crates/hts-ui/e2e/tests/value-sets.spec.ts`** — same pattern.
- **`crates/hts-ui/e2e/tests/concept-maps.spec.ts`** — same pattern **plus**
  the CM tests must now assert the Source / Target columns render (currently
  they can't because the columns don't exist).
- **`crates/hts-ui/e2e/tests/dashboard.spec.ts`** — Quick-links to
  `/ui/hts/code-systems` etc. remain valid.
- **`crates/hts-ui/e2e/tests/no-cdn.spec.ts`** (implied by the guard) — no
  new asset added, so this spec is unaffected.

### 11.3 New tests to add

Beyond the version-switcher spec (§6.6):

- `crates/hts-ui/e2e/tests/hts-search.spec.ts` (new file):
  1. **contains is default** — typing `loinc` in Name matches a seed CS named
     `loincOrgUnit`.
  2. **starts-with narrows** — switching to starts-with, typing `loinc`,
     only rows whose name starts with `loinc` remain.
  3. **exact matches only** — typing the full canonical URL returns exactly
     one row; typing a prefix returns zero.
  4. **case insensitivity is always on** — typing `LOINC` returns the same
     rows as `loinc` for every mode.
  5. **URL modifier is emitted** — after choosing starts-with, the browser's
     URL contains `?name:starts-with=loinc`; after choosing contains, the URL
     contains `?name=loinc` (bare, no modifier).
  6. **CM source/target end-to-end** — seed CM has `source=http://loinc.org`;
     typing `loinc.org` in Source filter (starts-with) returns the row; the
     Source column shows the URI as a chip.
- Extend `crates/hts-ui/tests/concept_maps.rs` with a mock-upstream test that
  asserts `source-uri=…` and `target-uri=…` appear on the outgoing GET.
- Backend tests: extend `crates/hts/src/backends/sqlite/concept_map.rs` test
  module with a case that filters by `source_uri` and asserts the WHERE clause
  actually narrows the result set.

---

## 12. Open questions for user gate

Maximum three, each phrased as a binary/trinary choice with a recommendation.

**Q1 — FHIR version switcher: ship in this branch, defer to a Phase 3 mini-slice, or skip entirely?**
- **Recommend: defer.** §6.4 estimate is L (~8–14 coordinated PRs across
  `helios-hts`, `helios-persistence`, `helios-hts-ui`), and the UX benefit is
  meaningful only for operators who genuinely run mixed R4/R4B/R5/R6 data —
  a minority per the design doc §9.2 note that HTS is compile-time
  single-version today. Ship §1–§5 + §7 first (contains + name column +
  CM source/target); land the switcher on its own gated branch with the
  three-block Playwright extension proposed in §6.6.

**Q2 — Rail-as-filter-form vs top-strip: reuse `.filter-rail` sticky sidebar (recommended) or introduce a full-width top strip?**
- **Recommend: rail.** §3.5 defence — filter density, sticky visibility during
  scrolling, and reuse of the sole HFS sticky-sidebar primitive.
  Counter-consideration: on ≤ 1250 px viewports the rail collapses to a card
  above the results; if the operator target profile is dominated by narrow
  laptop screens, a top strip with `.builder-grid` (`crates/ui/assets/app.css:2442-2446`)
  might be a better fit. **Defer to operator profile — say `rail` if the
  primary target is 1440 px+ desktops; say `top-strip` if 1280 px laptops
  dominate.**

**Q3 — CM 7th-column strategy: horizontal scroll below 1250 px (recommended), stack Source/Target in one cell, or drop the columns and put source/target in a per-row `<details>`?**
- **Recommend: horizontal scroll.** §5.3 defence — preserves the "scan the
  grid, see the mapping direction at a glance" grammar that is the whole
  reason for adding the columns; reuses `.table-wrap` (`crates/ui/assets/app.css:1924-1926`)
  with no new CSS. Alternatives hide the very information §1.3 identifies
  as silently dropped today.
