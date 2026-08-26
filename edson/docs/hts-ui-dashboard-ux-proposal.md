# HTS Dashboard UX proposal

Advisor deliverable for the operator dashboard at `/ui/hts` (design doc §7.1).
Read-only research artifact — no source file was modified as part of authoring
this proposal. Its scope ends at
`crates/hts-ui/templates/partials/hts-dashboard-cards.html`; the surrounding
page shell (`pages/dashboard.html`), sidebar/topbar chrome, degraded banner
partial, htmx polling contract, and Fluent catalogs are treated as fixed.

Composes with:
- `.claude/skills/work-with-ui/SKILL.md` — architecture rules (Askama + htmx,
  no SPA, no CDN, WCAG 2.2 AA, Fluent i18n, `nojs` degradation).
- `.claude/skills/frontend-design/SKILL.md` — HFS design discipline
  (subject-first, restraint, tokens over hex, avoid AI-defaults).
- `edson/docs/hts-ui-design.md` §7.1 — the dashboard's data sources and states.

---

## 1. Diagnosis

The current partial (`crates/hts-ui/templates/partials/hts-dashboard-cards.html`)
reads as a **linear stack of block sections** with no hierarchy between the
concept (label) and the value. Concretely:

- The BEM tree it emits — `hts-dashboard__cards`, `hts-card-row`,
  `hts-card-row--status`, `hts-card`, `hts-card__label`, `hts-card__value`,
  `hts-card__hint`, `hts-card__value--ok`, `hts-card__value--err`,
  `hts-card--wide`, `hts-card--soon`, `hts-quick-link*` — has **zero
  matching rules** anywhere in the workspace (verified against
  `crates/ui/assets/app.css`, the only stylesheet, which HTS reuses via
  `#[folder = "../ui/assets"]` in `crates/hts-ui/src/lib.rs`). Every one
  of those elements therefore inherits browser defaults.
- `<article>` and `<p>` default to `display: block` with roughly the same
  font size and weight. The label and the value end up on separate lines
  with the *same* typographic weight — the value never dominates.
- `<section class="hts-card-row hts-card-row--status">` is a bare block
  element with no grid or flex. All four tiles inside stack vertically at
  100% width. The dashboard reads as one long column.
- The row headings (`<h2 class="visually-hidden">`) do **not** disappear
  either: no `.visually-hidden` rule exists in the workspace, so the
  browser renders each heading at its default `<h2>` size, adding a big
  bold interruption between rows. This is a pre-existing bug adjacent
  to — but not caused by — this design work; the proposal below is
  compatible with either fixing it or leaving the class rule as future
  work.

HFS solved this exact problem for its own dashboard. `crates/ui/assets/app.css`
already defines the primitives:

- `.stat-grid` (L594-599) — 4-column CSS grid, 24 px gap, bottom-margin.
- `.stat-grid--2` (L789-792) — 2-column variant, capped at 620 px wide, used
  by `/ui/tenants`.
- `.card` (L587-592) — surface, border, radius, shadow token bundle.
- `.stat` (L601-604) — padding + min-height so tiles are uniform.
- `.stat__label` (L606-613) — the *eyebrow*: 10 px, uppercase, 0.6 px
  tracking, `--muted`.
- `.stat__value` (L615-622) — the *headline*: 34 px, weight 500, tight
  −0.7 px tracking, `--text`.
- `.stat__unit` (L624-628) — small-cap unit rider (17 px, super-baseline).
- `.stat__sub` (L630-635) — 12 px muted hint under the value.
- `@media (max-width: 900px)` (L1080-1083) — collapses `.stat-grid` to 2
  columns; matches the tenant maintenance page and the reference dashboard.

The reference dashboard uses exactly this vocabulary in
`crates/ui/templates/pages/index.html` (`<article class="card stat">` with
`<span class="stat__label">`, `<span class="stat__value">`, `<span
class="stat__sub">`). Adopting it verbatim removes the CSS gap and gives HTS
the same visual grammar as the FHIR-server dashboard — which is the point:
"same style, same technologies" per §1 of the design doc.

## 2. Design principles applied

Drawn from `.claude/skills/frontend-design/SKILL.md`:

- **Subject-first (§Applying the discipline).** The operator's first
  question on this page is a single verb: *is HTS up?* The second question
  is *what does it hold?* The third is *how fast?* Structure follows that
  question ladder — not a generic "top KPI row + supporting stats" layout.
- **Structure encodes information.** Rows 1–3 are peers by function
  (health / inventory / traffic). We use the `.stat-grid` grammar
  consistently and vary only cardinality (4 tiles → 2 tiles → 2 tiles) so
  the geometry teaches the operator to scan left-to-right within a
  category, top-to-bottom across categories.
- **Restraint / spend boldness once.** Only ONE tile carries a colour
  signal; every other value is pure typography. All chrome — borders,
  radii, shadow, spacing — is inherited from `.card` / `.stat` tokens.
- **Tokens over hex.** No `hex` values are introduced. The single new
  colour signal is `var(--accent-text)`, which HFS already tuned per
  theme for 4.5:1 on the light *and* dark surfaces (see the
  light-vs-dark override in `app.css` L64-67 and L86-88).
- **Copy is design material.** Every visible string maps to a Fluent
  key that already exists (§7 below), so `en`/`es`/`de` catalogs are
  untouched. Where a value is unavailable (upstream degrade,
  metrics-not-wired) we render an em-dash — never a fake zero.

### The one aesthetic risk

**Only the Status tile is coloured.** The Status value becomes the page's
single accent moment (`.stat__value--ok`, rendering `var(--accent-text)`);
Backend, Uptime, FHIR version, Loaded systems, Bundled data, Requests, Avg
latency remain in the neutral `--text` colour. On degrade, Status falls to
`—` — again neutral — and the degraded banner (already emitted by the
partial) becomes the loud signal instead.

*Justification.* HTS is defined by "is the terminology backend answering,
and what is it serving?". The bold moment lives on the primary answer to
that thesis. Colouring counts too would flatten the hierarchy and turn the
dashboard into a generic KPI wall — the "AI-default" trap called out in
the skill.

## 3. Structure

Four sections, in reading order. Each section keeps its existing
`aria-labelledby`, so the sr-only heading contract on the current partial
is preserved (the Quick links heading becomes an in-card eyebrow, still
addressable by assistive tech).

| # | Section | Heading | Grid | Tiles | Signature |
|---|---|---|---|---|---|
| 1 | **Status** | `<h2 class="visually-hidden">` `hts-dashboard-row-status` | `.stat-grid` (4 col) | 4 | The one place colour lives |
| 2 | **Inventory** | `<h2 class="visually-hidden">` `hts-dashboard-row-inventory` | `.stat-grid.stat-grid--2` | 2 | Wide neutral counts |
| 3 | **Metrics** | `<h2 class="visually-hidden">` `hts-dashboard-row-metrics` | `.stat-grid.stat-grid--2` | 2 | Em-dash placeholders + hint |
| 4 | **Quick links** | eyebrow inside a `.card` (`.stat__label`) | full-width `.card` | 5 anchors | Real `<a href>`; nojs-safe |

### Data → tile mapping

Preserves every field from `DashboardCards` in
`crates/hts-ui/src/dashboard.rs` (L36-83).

- **Status** — `cards.health.status` (`hts-dashboard-status-{status}`);
  falls back to `—` in the `Err` arm. Colour on the `Ok` arm only.
- **Backend** — `cards.health.backend` in `<code>` (subject-appropriate
  monospace for a backend id).
- **Uptime** — `cards.health.uptime_pretty()`.
- **FHIR version** — `cards.capabilities.fhir_version` when present,
  otherwise the compiled-in `chrome.fhir_version` (matches the current
  fallback chain).
- **Loaded systems** — `cards.loaded_system_count()`; em-dash on `None`
  with hint `hts-dashboard-tile-loaded-systems-hint`.
- **Bundled data** — `cards.bundled_data_mib()` piped through
  `hts-dashboard-tile-bundled-data-value` (Fluent handles the unit) with
  hint `hts-dashboard-tile-bundled-data-hint`.
- **Requests / Avg latency** — em-dash pair, hint
  `hts-dashboard-tile-metrics-hint` (Wave 2 wiring; **no chart** per
  proposal constraint).
- **Quick links** — five anchors keyed by
  `hts-nav-{code-systems, value-sets, concept-maps, operations, import}`;
  Operations is the primary and uses the `.pill` pattern already in
  `app.css` (L654-666) with `aria-current="false"` neutrality.

### Responsive

At `≤ 900 px` the existing `@media` rule (L1080-1083) collapses `.stat-grid`
to 2 columns; the Status row wraps to 2 × 2 without a new breakpoint.
`.stat-grid--2` at that width naturally becomes 2 × 1 within its 620 px cap
(Inventory / Metrics rows unchanged). The Quick links card wraps its `<nav>`
via `flex-wrap` — a property the `.pill` class already accepts.

## 4. Class map (current → proposed)

| Current | Proposed | Notes |
|---|---|---|
| `<div class="hts-dashboard__cards" id="hts-dashboard-cards" aria-live="polite" aria-busy="false">` | `<div id="hts-dashboard-cards" class="hts-dashboard" aria-live="polite" aria-busy="false">` | Keep `id` (htmx anchor) + `aria-*` intact. `.hts-dashboard` acts as a namespace hook only, no CSS attached; drops the BEM `__cards` suffix. |
| `<section class="hts-card-row hts-card-row--status">` | `<section class="stat-grid" aria-labelledby="hts-dashboard-row-status">` | Reuse HFS 4-col grid. |
| `<section class="hts-card-row hts-card-row--inventory">` | `<section class="stat-grid stat-grid--2" aria-labelledby="hts-dashboard-row-inventory">` | Reuse HFS 2-col variant. |
| `<section class="hts-card-row hts-card-row--metrics">` | `<section class="stat-grid stat-grid--2" aria-labelledby="hts-dashboard-row-metrics">` | Same 2-col variant; visually parallel to Inventory. |
| `<section class="hts-card-row hts-card-row--links">` | `<section class="card hts-quick-strip" aria-labelledby="hts-dashboard-row-links">` | Full-width card (uses `.card` chrome). `.hts-quick-strip` is a namespace hook only, no new CSS. |
| `<article class="hts-card">` | `<article class="card stat">` | HFS pattern — surface + tile padding come from tokens. |
| `<article class="hts-card hts-card--wide">` | `<article class="card stat">` | The width modifier is unnecessary; the `.stat-grid--2` parent already handles it. |
| `<article class="hts-card hts-card--soon">` | `<article class="card stat">` | Placeholder-ness is conveyed by the em-dash + `stat__sub` hint copy, not a class. |
| `<p class="hts-card__label">` | `<span class="stat__label">` | HFS uses `<span>` with `display: block`. Same visual: uppercase eyebrow. |
| `<p class="hts-card__value">` | `<span class="stat__value">` | Gets the 34 px display type. Wrap `<code>` for the Backend id. |
| `<p class="hts-card__value hts-card__value--ok">` | `<span class="stat__value stat__value--ok">` | Only tile with a colour modifier — the one aesthetic risk. |
| `<p class="hts-card__value hts-card__value--err">` | `<span class="stat__value">` (em-dash content) | Negative state carried by content (`—`) + the degraded banner, not a red colour. |
| `<p class="hts-card__hint">` | `<span class="stat__sub">` | 12 px muted, matches HFS reference. |
| `<h2 class="hts-card-row__heading">` (only on Quick links row) | Eyebrow `<span class="stat__label">` inside the Quick links card, plus `<h2 class="visually-hidden">` for the sr-only heading | Aligns Quick links with the other three sections' sr-only h2 pattern; the visible label reuses `.stat__label` typography. |
| `<nav class="hts-quick-links">` | `<nav class="hts-quick-links">` | Kept as a namespace hook. The children use `.pill`. |
| `<a class="hts-quick-link">` | `<a class="pill">` | Reuse the existing `.pill` chip pattern (`app.css` L654-666). |
| `<a class="hts-quick-link hts-quick-link--emphasis">` | `<a class="pill" aria-current="page">` (operator can still find the primary), or keep as first-in-order | Do NOT introduce a new emphasized pill variant — restraint. |

Every `<article>` and `<span>` swap preserves the outer element for tests
that count `card`/`stat` occurrences; the wrapper div keeps `id`,
`aria-live`, and `aria-busy` so the htmx contract in
`crates/hts-ui/templates/pages/dashboard.html` (L21-26) is untouched.

## 5. Full proposed markup for `partials/hts-dashboard-cards.html`

Preserves every `{% match %}` and `{% if %}` block from the current file;
only element choices and classes change. Askama comments retained.

```html
{#-
  Dashboard cards fragment (design doc §7.1).
  Rendered once on hard navigation as part of `pages/dashboard.html`, and
  again on every 15 s htmx refresh via `GET /ui/hts/dashboard/cards`. Same
  markup either way — the outer `#hts-dashboard-cards` region is swapped
  wholesale, so a partial failure never blanks the healthy tiles.

  Style: reuse the HFS `.stat-grid` / `.card.stat` primitives from
  `crates/ui/assets/app.css` so the terminology dashboard reads with the
  same visual grammar as the FHIR-server dashboard. HTS-namespaced hooks
  (`.hts-dashboard`, `.hts-quick-strip`, `.hts-quick-links`) carry no
  visual weight — they exist so this fragment can be selected in tests
  and future style deltas without renaming.
-#}
<div id="hts-dashboard-cards"
     class="hts-dashboard"
     aria-live="polite"
     aria-busy="false">

  {% if let Some(reason) = cards.degraded_reason() %}
  <aside class="hts-degraded" role="alert" aria-live="assertive">
    <p class="hts-degraded__title">{{ chrome.i18n.t("hts-degraded-title") }}</p>
    <p class="hts-degraded__reason">
      {{ chrome.i18n.t(format!("hts-degraded-reason-{}", reason).as_str()) }}
    </p>
    <p class="hts-degraded__body">{{ chrome.i18n.t("hts-degraded-body") }}</p>
  </aside>
  {% endif %}

  {#-- Row 1: Status / Backend / Uptime / FHIR version --#}
  <section class="stat-grid" aria-labelledby="hts-dashboard-row-status">
    <h2 id="hts-dashboard-row-status" class="visually-hidden">{{ chrome.i18n.t("hts-dashboard-row-status") }}</h2>

    <article class="card stat">
      <span class="stat__label">{{ chrome.i18n.t("hts-dashboard-tile-status") }}</span>
      {% match cards.health %}
        {% when Ok(h) %}
          <span class="stat__value stat__value--ok">
            {{ chrome.i18n.t(format!("hts-dashboard-status-{}", h.status).as_str()) }}
          </span>
        {% when Err(_) %}
          <span class="stat__value">&mdash;</span>
      {% endmatch %}
    </article>

    <article class="card stat">
      <span class="stat__label">{{ chrome.i18n.t("hts-dashboard-tile-backend") }}</span>
      {% match cards.health %}
        {% when Ok(h) %}<span class="stat__value"><code>{{ h.backend }}</code></span>
        {% when Err(_) %}<span class="stat__value">&mdash;</span>
      {% endmatch %}
    </article>

    <article class="card stat">
      <span class="stat__label">{{ chrome.i18n.t("hts-dashboard-tile-uptime") }}</span>
      {% match cards.health %}
        {% when Ok(h) %}<span class="stat__value">{{ h.uptime_pretty() }}</span>
        {% when Err(_) %}<span class="stat__value">&mdash;</span>
      {% endmatch %}
    </article>

    <article class="card stat">
      <span class="stat__label">{{ chrome.i18n.t("hts-dashboard-tile-fhir-version") }}</span>
      <span class="stat__value">
        {%- match cards.capabilities -%}
          {%- when Ok(c) -%}
            {%- if c.fhir_version.is_empty() -%}{{ chrome.fhir_version }}{%- else -%}{{ c.fhir_version }}{%- endif -%}
          {%- when Err(_) -%}
            {{ chrome.fhir_version }}
        {%- endmatch -%}
      </span>
    </article>
  </section>

  {#-- Row 2: Loaded systems / Bundled data --#}
  <section class="stat-grid stat-grid--2" aria-labelledby="hts-dashboard-row-inventory">
    <h2 id="hts-dashboard-row-inventory" class="visually-hidden">{{ chrome.i18n.t("hts-dashboard-row-inventory") }}</h2>

    <article class="card stat">
      <span class="stat__label">{{ chrome.i18n.t("hts-dashboard-tile-loaded-systems") }}</span>
      <span class="stat__value">
        {%- match cards.loaded_system_count() -%}
          {%- when Some(n) -%}{{ n }}
          {%- when None -%}&mdash;
        {%- endmatch -%}
      </span>
      <span class="stat__sub">{{ chrome.i18n.t("hts-dashboard-tile-loaded-systems-hint") }}</span>
    </article>

    <article class="card stat">
      <span class="stat__label">{{ chrome.i18n.t("hts-dashboard-tile-bundled-data") }}</span>
      <span class="stat__value">
        {%- match cards.bundled_data_mib() -%}
          {%- when Some(mib) -%}{{ chrome.i18n.t_arg("hts-dashboard-tile-bundled-data-value", "mib", mib.to_string()) }}
          {%- when None -%}&mdash;
        {%- endmatch -%}
      </span>
      <span class="stat__sub">{{ chrome.i18n.t("hts-dashboard-tile-bundled-data-hint") }}</span>
    </article>
  </section>

  {#-- Row 3: Metrics placeholder (Wave 2 — no chart per §7.1 states matrix). --#}
  <section class="stat-grid stat-grid--2" aria-labelledby="hts-dashboard-row-metrics">
    <h2 id="hts-dashboard-row-metrics" class="visually-hidden">{{ chrome.i18n.t("hts-dashboard-row-metrics") }}</h2>

    <article class="card stat">
      <span class="stat__label">{{ chrome.i18n.t("hts-dashboard-tile-requests") }}</span>
      <span class="stat__value">&mdash;</span>
      <span class="stat__sub">{{ chrome.i18n.t("hts-dashboard-tile-metrics-hint") }}</span>
    </article>

    <article class="card stat">
      <span class="stat__label">{{ chrome.i18n.t("hts-dashboard-tile-avg-latency") }}</span>
      <span class="stat__value">&mdash;</span>
      <span class="stat__sub">{{ chrome.i18n.t("hts-dashboard-tile-metrics-hint") }}</span>
    </article>
  </section>

  {#-- Row 4: Quick links — real `<a>` first (nojs-safe), styled as pills. --#}
  <section class="card hts-quick-strip" aria-labelledby="hts-dashboard-row-links">
    <h2 id="hts-dashboard-row-links" class="visually-hidden">{{ chrome.i18n.t("hts-dashboard-quick-links") }}</h2>
    <span class="stat__label">{{ chrome.i18n.t("hts-dashboard-quick-links") }}</span>
    <nav class="hts-quick-links" aria-label="{{ chrome.i18n.t("hts-dashboard-quick-links") }}">
      <a class="pill" href="/ui/hts/code-systems">{{ chrome.i18n.t("hts-nav-code-systems") }}</a>
      <a class="pill" href="/ui/hts/value-sets">{{ chrome.i18n.t("hts-nav-value-sets") }}</a>
      <a class="pill" href="/ui/hts/concept-maps">{{ chrome.i18n.t("hts-nav-concept-maps") }}</a>
      <a class="pill" href="/ui/hts/operations">{{ chrome.i18n.t("hts-nav-operations") }}</a>
      <a class="pill" href="/ui/hts/import">{{ chrome.i18n.t("hts-nav-import") }}</a>
    </nav>
  </section>
</div>
```

## 6. CSS additions (if any)

**One rule. Two lines.** Added to `crates/ui/assets/app.css`, adjacent to the
existing `.stat__value` block (~L622). This location is HFS-safe: the class
name `.stat__value--ok` is not used anywhere in HFS's own dashboard today
(verified against `crates/ui/templates/pages/index.html` and the workspace),
so introducing the rule cannot regress the FHIR-server dashboard — but the
primitive becomes available to any future HFS card that wants an "answered"
signal (uptime health, submit worker liveness, etc.).

```css
.stat__value--ok { color: var(--accent-text); }
```

Optional two lines for the Quick links card layout (only if the operator
review flags the pill row as too tight against the eyebrow). Also placed
next to `.stat-grid--2` (~L792):

```css
.hts-quick-strip { padding: 16px 20px; display: grid; gap: 12px; }
.hts-quick-strip .hts-quick-links { display: flex; flex-wrap: wrap; gap: 8px; }
```

**Total budget:** 3 lines mandatory, up to 5 lines if the optional block is
adopted — well under the 10-line ceiling. Neither block touches the HFS
dashboard or the tenants page.

No HTS-only stylesheet is needed. HTS's `RustEmbed` at
`crates/hts-ui/src/lib.rs::Assets` already points at `../ui/assets`, so
edits to `crates/ui/assets/app.css` ship to both binaries without any
build wiring changes.

## 7. Fluent keys required

Every visible string in the proposed markup already exists in the
workspace catalogs (`locales/en/main.ftl`, `locales/es/main.ftl`,
`locales/de/main.ftl`). Grepped against `hts-dashboard-` and `hts-nav-`
and `hts-degraded-` prefixes at the workspace root.

| Key | Status | Used for |
|---|---|---|
| `hts-dashboard-title` | `[EXISTS]` (en/es/de L621/617/617) | Page `<h1>` (already emitted by `pages/dashboard.html`; unchanged) |
| `hts-dashboard-subtitle` | `[EXISTS]` | Page subtitle (unchanged) |
| `hts-dashboard-row-status` | `[EXISTS]` | Row 1 sr-only `<h2>` |
| `hts-dashboard-row-inventory` | `[EXISTS]` | Row 2 sr-only `<h2>` |
| `hts-dashboard-row-metrics` | `[EXISTS]` | Row 3 sr-only `<h2>` |
| `hts-dashboard-quick-links` | `[EXISTS]` | Row 4 sr-only `<h2>` **and** visible in-card eyebrow (double use of the same key is intended — the sr-only heading and the visual eyebrow name the same section) |
| `hts-dashboard-tile-status` | `[EXISTS]` | Status tile label |
| `hts-dashboard-tile-backend` | `[EXISTS]` | Backend tile label |
| `hts-dashboard-tile-uptime` | `[EXISTS]` | Uptime tile label |
| `hts-dashboard-tile-fhir-version` | `[EXISTS]` | FHIR version tile label |
| `hts-dashboard-tile-loaded-systems` | `[EXISTS]` | Inventory tile label |
| `hts-dashboard-tile-loaded-systems-hint` | `[EXISTS]` | Inventory hint sub |
| `hts-dashboard-tile-bundled-data` | `[EXISTS]` | Inventory tile label |
| `hts-dashboard-tile-bundled-data-value` | `[EXISTS]` | Bundled-data formatted value (uses `$mib`) |
| `hts-dashboard-tile-bundled-data-hint` | `[EXISTS]` | Inventory hint sub |
| `hts-dashboard-tile-requests` | `[EXISTS]` | Metrics tile label |
| `hts-dashboard-tile-avg-latency` | `[EXISTS]` | Metrics tile label |
| `hts-dashboard-tile-metrics-hint` | `[EXISTS]` | Metrics hint sub |
| `hts-dashboard-status-ok` | `[EXISTS]` | Status value when upstream returns `ok` |
| `hts-degraded-title` / `-body` / `-reason-*` | `[EXISTS]` | Degraded banner (unchanged, still consumed via the existing partial) |
| `hts-nav-code-systems` / `-value-sets` / `-concept-maps` / `-operations` / `-import` | `[EXISTS]` | Quick-link anchor labels |

**No new Fluent keys required.** The proposal ships as a template + CSS
change only; catalogs remain byte-identical.

## 8. a11y checklist

- **Focus order after 15 s poll swap.** The htmx swap replaces the entire
  `#hts-dashboard-cards` region (`hx-swap="outerHTML"` per
  `pages/dashboard.html`). Focus lives on the sidebar or the topbar
  during a swap, so the region churn never yanks it. The `<a
  class="pill">` links inside the Quick links card do participate in tab
  order — they must survive the swap, which they do because the region
  is replaced *between* poll cycles when JS is idle. If a future
  operator-triggered manual refresh is added, prefer `hx-preserve` or
  swap only the interior sections rather than the whole region so the
  focused anchor is not blown away mid-tab.
- **Colour tokens used for ok / err.** The `Ok` arm uses
  `var(--accent-text)` — HFS's theme-adapted accent colour (`#0a5f96`
  light, `#7cc9ff` dark). Both hit ≥ 4.5:1 on their respective card
  surfaces per the `app.css` comments at L64-67 and L86-88. The `Err`
  arm intentionally uses **no colour** — negative state is carried by
  the em-dash and the `hts-degraded` banner (`role="alert"`, ANSI red
  visual notwithstanding). Consequence: `--danger` / `--ok` tokens are
  *not* introduced; if the operator later asks for a red degrade tile,
  we add both tokens together, in a follow-up.
- **Screen-reader path.** Row 1–3 keep the sr-only `<h2>` inside each
  `<section aria-labelledby="…">`; assistive tech announces "Server
  status, section" before the four tiles, then "Loaded inventory,
  section" before the two, then "Traffic metrics, section". Row 4's
  `<h2>` is also sr-only for parity, but the visible `.stat__label`
  eyebrow (same key) doubles the affordance for sighted users. `<code>`
  inside the Backend tile renders a monospace token — SR announces its
  content verbatim.
- **Keyboard.** All Quick links are real `<a href>`; the `.pill` class
  inherits focus outlines from the shared CSS (`.pill` L654-666 is
  purely visual chrome, focus styling comes from HFS's global focus
  rules).
- **Reduced motion.** The only motion on this page is the htmx 15 s
  poll, which is a *content swap* rather than an animation — nothing to
  gate on `prefers-reduced-motion`. No CSS transitions are introduced by
  this proposal.
- **`nojs` degradation.** The `pages/dashboard.html` shell already fetches
  once on hard navigation, so the tiles paint with real data on first
  paint. Quick links are `<a href>` — traversal works with JS off.
  Degraded banner is server-rendered; JS is never required to see it.
- **Contrast in both themes.** All values sit on `.card` (linear-gradient
  of `#fafafa`/`#f8f8f8` light, `#222`/`#242` dark). The `--text`
  (`#101010` / `#ffffff`) values clear ≥ 15:1; the `--muted`
  (`#6a6a6a` / `#a0a0a0`) labels clear ≥ 4.5:1 (already tuned by HFS,
  see `app.css` L49 comment). The one accent (`--accent-text`) is
  independently verified per the same tokens' theme block.

## 9. Playwright / test impact

Grepped `crates/hts-ui/e2e/tests/dashboard.spec.ts` and
`crates/hts-ui/tests/router_http.rs` for class-based selectors:

- `dashboard.spec.ts` — uses only `getByRole` and `getByText` for
  "Dashboard", "Status", "Backend", "Uptime", "FHIR version", "Loaded
  systems", "Bundled data", plus `.dialect-chip__value` (topbar chrome,
  untouched) and `.pane` (base layout, untouched). No `.hts-card*`
  selectors. **All specs pass unmodified.**
- `tests/router_http.rs` — asserts Fluent-key *translations* land in the
  HTML (`hts-dashboard-title`, `hts-degraded-title`, `hts-dialect-prefix`,
  etc.) and negotiates locales; no class assertions. **Safe.**
- Ripgrep of `hts-card`, `hts-card-row`, `hts-dashboard__cards`,
  `hts-quick-link` across `crates/hts-ui/` returns only the current
  partial as a hit — no other template, no Rust test, no e2e spec
  references these classes. The class map in §4 can therefore be
  applied without a companion test update.

**Recommended follow-up spec** (optional, not a blocker): add one
assertion in `dashboard.spec.ts` that `.card.stat` renders four tiles in
Row 1 to lock the new structural grammar. Belongs in the PR that lands
the template change, not this design pass.

## 10. Open questions for user gate

Two decisions where two options are equally defensible — surfacing so the
implementation PR can land with a clear intent.

1. **Row 1 heading — sr-only or visible eyebrow?**
   - *Option A (proposed above).* Keep the current sr-only `<h2>`
     pattern. Sighted users learn the sections from the geometry
     (4-tile row vs 2-tile row) and the label eyebrows on the tiles
     themselves.
   - *Option B.* Promote Row 1's heading to a visible eyebrow inside a
     wrapping `.card` container (turning the four tiles into a "vital
     signs" pod). Increases explicit hierarchy at the cost of one
     extra visual layer.

2. **Backend token typography — `<code>` or plain text?**
   - *Option A (proposed).* `<code>` — the backend id is a system name
     (`sqlite`, `postgres`, `elasticsearch`); monospace signals
     "identifier".
   - *Option B.* Plain text in `.stat__value`. Reads more like a
     product name; the operator may prefer this if the backend field
     ever renders a human string like "SQLite (embedded)".

3. **The optional `.hts-quick-strip` CSS block** (§6). Adopt on
   day one for visual polish, or defer until an operator review flags
   the pill row as too tight? Neither is wrong; day-one adoption keeps
   the aesthetic risk contained to a single review cycle.

---

*End of proposal. No source files modified. Implementation is a separate PR.*
