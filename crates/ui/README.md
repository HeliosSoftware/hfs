# helios-ui — HTMX-first web UI for HFS

A server-rendered, **HTMX-first** web UI for the Helios FHIR Server. It is a thin
Axum library crate that owns templates, static assets, and view handlers, and is
mounted by the `hfs` binary as a sub-router under `/ui`.

This document is both the discussion doc and the **rules of the road**: when we
build UI in this codebase, this is where things go and why. For the operational
summary — routes, config, test commands — see the `/work-with-ui` skill
(`.claude/skills/work-with-ui/SKILL.md`).

---

## Approach: server-rendered HTMX, not a SPA

We render HTML on the server (Rust) and use [htmx](https://htmx.org/docs/) for
partial page updates. Handlers return **full pages** on hard navigations and
**HTML fragments** on `HX-Request`s. State lives on the server; the client stays
thin.

**There is no React, Vue, Svelte, Alpine, or jQuery here — and no bundler, no
npm dependency, and no build step for the browser code, with one narrow,
documented exception** (see "Assets: vendored & embedded" below). Two
third-party scripts are vendored: htmx, and — as a prebuilt, never-built-here
bundle — CodeMirror 6, mounted over the ViewDefinition JSON editor (#753) and
the SQL Queries/SQL Views editor panes (#838). Everything else under
`assets/` is hand-written vanilla JS in an IIFE. Do not introduce a
framework.

Why, over a SPA + JSON API:

- **Stays close to the FHIR logic we already have.** The UI calls into the
  existing workspace crates (`helios-fhir`, `helios-fhir-validator`,
  `helios-persistence`, `helios-observability`) and renders the result. There is
  no second copy of the domain model in a browser client, and no client-side view
  state to keep in sync.
- **No duplicated API surface.** We do not add a browser-facing JSON API.
  htmx consumes HTML fragments, so the FHIR REST surface stays clean and
  UI-agnostic.
- **Progressive enhancement.** Because the server renders real HTML at real
  URLs, the UI degrades to working full-page loads when JavaScript is absent —
  asserted by the `nojs` Playwright project, not just promised. See
  [HATEOAS](https://htmx.org/essays/hateoas/) and
  [Hypermedia Systems](https://hypermedia.systems/).
- **Locality of Behaviour.** Behavior is co-located with markup via `hx-*`
  attributes rather than scattered across JS files. See
  [Locality of Behaviour](https://htmx.org/essays/locality-of-behaviour/).

Keeping the UI in a **separate crate** from `helios-rest` preserves the clean
FHIR REST surface and lets the UI be feature-gated off (`--no-default-features`
or the `headless` feature on `hfs`) for headless deployments.

---

## Templating decision: Askama (finalized)

We use **[Askama](https://docs.rs/askama)** — Jinja2-like templates that are
**compiled and type-checked at build time**, with **auto-escaping** on `.html`
templates.

Trade-offs weighed:

| Engine | Checked at | HTML lives in | Verdict |
|--------|-----------|---------------|---------|
| **Askama** (chosen) | **compile time** | template files | Type-checked, auto-escaping, keeps markup out of Rust. Templates that reference missing fields fail the build. |
| [Maud](https://maud.lambda.xyz/) | compile time | **Rust macros** | Fast and type-safe, but markup lives *in Rust source* — in tension with our "no HTML in Rust" rule. |
| [Minijinja](https://docs.rs/minijinja) | runtime | template files | Flexible / hot-reloadable, but template errors surface at request time, not build time. |

Askama wins because it keeps markup in template files (satisfying the rule
below) **and** fails the build on template errors, matching the rest of this
codebase's compile-time-correctness bias. Auto-escaping gives us
[XSS](https://cheatsheetseries.owasp.org/cheatsheets/Cross_Site_Scripting_Prevention_Cheat_Sheet.html)
protection by default — `{{ value }}` is HTML-escaped unless explicitly marked
safe.

---

## Assets: vendored & embedded, never a runtime CDN

htmx, CSS, fonts, and the per-page scripts are **vendored** under `assets/`
(htmx pinned at `htmx.org@2.0.4`) and **embedded into the binary** at compile
time with [rust-embed](https://docs.rs/rust-embed), served from `/ui/assets/*`
by [axum-embed](https://docs.rs/axum-embed) with br/gzip/deflate negotiation.

**Never hotlink a CDN in production.** A healthcare server may run offline or
air-gapped, and a runtime CDN dependency is a supply-chain risk. Embedding also
keeps `hfs` a single self-contained binary — no asset directory to ship
alongside it. `e2e/tests/no-cdn.spec.ts` makes this executable: it fails if any
page issues an off-origin request.

`debug-embed` is on, so **debug builds embed too** — editing a file under
`assets/` or `templates/` needs a rebuild to take effect. Assets are served with
`Cache-Control: no-cache` plus a content-based `ETag`, so unchanged assets come
back as a cheap `304` while a rebuilt one is always re-fetched rather than served
stale.

To update htmx, replace `assets/htmx.min.js` with the new pinned release and
note the version bump in the commit.

### The one exception: a vendored, prebuilt bundle

"No bundler" (above) is the default, not an absolute: a third-party script that
is a real parser or grammar — not a widget — cannot reasonably be hand-written
in the style every other asset in this crate uses. For that narrow case:

> Third-party browser code may be vendored as a prebuilt single-file bundle produced by a
> documented, checked-in, one-off script under `crates/ui/vendor/`; pinned versions and a
> lockfile are committed alongside it; the script is never executed at build time or in CI;
> the resulting bundle is never loaded from a CDN; and the bundle ships with its license
> banner intact.

CodeMirror 6 is the first, and so far only, case this applies to (#753,
extended by #838): [`crates/ui/vendor/codemirror/`](vendor/codemirror/README.md)
is the vendoring ritual (pinned npm dependencies, a committed lockfile, a
rollup + terser recipe run by hand, never by `cargo build` or CI); its one
output, [`assets/vendor/codemirror.bundle.js`](assets/vendor/codemirror.bundle.js),
is the vendored bundle itself — embedded and served exactly like every other
asset in this crate (above), nothing bundler-specific about how it ships. It
backs the ViewDefinition JSON editor on `/ui/sql/view-definitions` and the SQL
pane editors on `/ui/sql/queries` and `/ui/sql/views`; see
[`docs/viewdefinition-editor-evaluation.md`](../../docs/viewdefinition-editor-evaluation.md)
for the full evaluation this amendment is drawn from.

### Client-side scripts

Each is a small, self-contained IIFE. Page-specific scripts load with `defer`;
`json-view.js` and `busy.js` load from the shared layout because their behavior
is used across workspaces — `busy.js` ahead of every page script (defer runs in
document order) because page scripts call into it. `theme.js` loads **without
`defer`**, before first paint, to avoid a flash of the wrong theme. `busy.js`
exports `window.hfsBusy`; the CodeMirror stack (below) exports two more
globals for the same reason — code that exists to be called by other scripts,
not just a closed IIFE.

| Asset | Owns |
|---|---|
| `theme.js` | Light/dark preference: stored choice → OS preference, plus the top-bar toggle |
| `busy.js` | The shared busy states (#679): `during(buttons, work)` and `region(el, label)` |
| `saved-queries.js` | Saved queries, the visual search builder, the `/_user/settings` read/modify/write cycle, and — on Resources/Search/Saved Queries — writing `rails.<page>` back on an in-page rail click (#754/#755) |
| `editor.js` | The schema-driven editor loop — posts the document to `/ui/editor/render` and swaps in the server's HTML |
| `json-view.js` | Delegated folding and accessibility state for every server-rendered JSON view |
| `combobox.js` | Shared multi-select state, chips, keyboard/ARIA behavior, and progressive fallback upgrade; htmx owns transport and callers own result semantics |
| `resources.js` | The Resources workspace edit modal and "Create new" |
| `batch.js` | Bundle pick → lazy highlighted previews → execution plan → per-entry outcomes |
| `bulk-export.js` | All Resources, individual resource types, and Since/Custom instant state on the Bulk Export builder |
| `history.js` | Version selection and diff requests |
| `nl-search.js` | Natural-language search mode (only loaded when configured) |
| `resource-filter.js` | Shared truncated-name tooltips (type rails and the resource grid) and each rail's scroll-to-selection on arrival — the "Recently used" group itself is server-rendered (#754/#755) |
| `conformance-crud.js` | The conformance viewers' write half (create/edit/delete against the FHIR API) |
| `code-editor.js` | Shared CodeMirror 6 mount helper (#838): textarea-as-source-of-truth sync, aria-label, Tab-not-captured, silent degradation — `window.HfsCodeEditor.mount(textarea, options)`, exported for `vd-editor.js` and `sql-editor.js` to build their own language/highlight/lint on top of |
| `vd-editor.js` | The ViewDefinition editor on `/ui/sql/view-definitions`: JSON + injected FHIRPath language and highlighting, fold, and the async server lint (#753, generalized onto `code-editor.js` in #838) |
| `sql-editor.js` | The SQL pane editor on `/ui/sql/queries` and `/ui/sql/views`: SQLite-dialect SQL language and highlighting, no fold or lint yet (#838) |

`editor.js` is deliberately thin, and that is the architectural point: it does
not model the resource, know what a choice type is, or understand cardinality.
All of that lives in Rust behind `/ui/editor/render`, where it is tested.

---

## Copy capitalization (#652)

The English catalog (`locales/en/main.ftl`) follows one convention:

- **Title Case** for page titles, section headings, card titles, nav entries,
  tab labels, and action buttons/controls (`Server Dashboard`, `Add Tenant`,
  `Save Changes`). Small words stay lowercase mid-title (`a, an, and, as, at,
  by, for, from, in, of, on, or, the, to, with`); hyphenated compounds
  capitalize both halves (`Per-Action Outcomes`).
- **Sentence case** for anything that reads as a sentence or fragment:
  ledes, help text, hints, subtitles, placeholders, confirmation prompts,
  status values, and error messages — even when the key suffix says `-title`
  or `-heading` (a full-sentence heading stays a sentence).
- FHIR and spec spellings are verbatim, always: `FHIR`, `SQL on FHIR`,
  `ViewDefinition`, `NDJSON`, env-var names.

Spanish and German keep their own capitalization norms (both languages use
sentence case where English uses Title Case); the convention above is for
`en` only.

## Required-field marker (#680)

Required fields get an accent-colored asterisk, matching the marker already
used by the resource editor's add-picker (`.editor-add__name em`). This is
implemented as a single CSS `:has()` rule next to `.field__label` in
`assets/app.css`, not per-field markup: the `required` attribute on the input
is the source of truth, and the rule appends `*` after the field's visible
label whenever it wraps a required input. The one field without a visible
label — the search builder's query URL — is marked on its `query-builder__tag`
chip instead. No template renders `*` for this; the only literal `<em>*</em>`
in the codebase is the editor's own (unrelated) marker. Changing the
convention later (e.g. an `(optional)` suffix instead) means editing only
that one CSS rule.

## Error wording (#677)

Errors follow one convention across the Fluent catalogs
(`locales/{en,es,de}/main.ftl`) and the OperationOutcome diagnostics that
`crates/rest/src/error.rs` renders into them:

- Full-sentence errors end with a terminal period; fragments, labels, and
  status values (`Failed`, `failed`, `unavailable`) take none.
- Operation failures use one shape: a full sentence naming the object, with
  the cause after an em dash — `Could not add the tenant — that ID is
  already in use.`
- Interpolated values are single-quoted: `Content type 'text/csv' is not
  supported.`
- Duality: this convention governs what the user sees — the message catalogs
  and the `diagnostics` field of returned `OperationOutcome`s, which reach
  the browser verbatim. It does not apply to `RestError`'s `Display` impl in
  `crates/rest/src/error.rs`, which stays in its own `Label: value` shape —
  that form is for logs and traces, not the UI, and is intentionally
  distinct.

## Rules of the road — where things go

- `crates/ui/src/` — Axum handlers/routers returning `impl IntoResponse`
  (HTML). **Thin:** parse request → call into the workspace → render a template.
  Modules: `lib.rs` (router, dashboard, prefs, search), `editor.rs`,
  `search_params.rs`, `compartments.rs`, `conformance.rs`, `history.rs`,
  `tenants.rs`, `json_view.rs`, `i18n.rs`.
- `crates/ui/templates/` — `.html` templates:
  - `layouts/` — shared document shells (`base.html`).
  - `pages/` — full documents (extend a layout).
  - `partials/` — HTMX-swappable fragments (no `<html>` wrapper).
  - `icons/` — Figma-exported SVGs, inlined with `{% include %}`.
- `crates/ui/assets/` — vendored, pinned `htmx.min.js`, CSS, JS, fonts, images.
  Embedded; never fetched at runtime.
- `locales/<locale>/main.ftl` — **at the workspace root**, not in this crate.
- Handlers branch on the **`HX-Request`** header to return a fragment vs. a full
  page (progressive enhancement).

## Rules of the road — where things must NOT go

- **No HTML in Rust string literals or `format!`.** All markup lives in
  templates.
- **No business/FHIR logic in templates.** Templates render data; they don't
  compute it. Reuse existing crates for data access — don't re-implement
  persistence or terminology logic here.
- **No new browser-facing JSON API** to feed the UI. htmx consumes HTML
  fragments, not JSON.
- **No SPA framework, no bundler, no npm dependency** for browser code, with
  one documented exception — see "Assets: vendored & embedded" above. (The
  `e2e/` directory also has a `package.json`, but that is test-only and never
  ships.)
- **No inline `<script>` blobs or scattered JS.** Prefer `hx-*` attributes
  (Locality of Behaviour); where JS is truly needed, use small pinned assets.
  Inert `type="application/json"` data carriers are the one allowed exception,
  and `no-cdn.spec.ts` enforces the rest.
- **No off-origin requests** — no CDN, no remote font, no remote image.
- **No user-visible prose in templates** — templates hold Fluent catalog keys.
- **`helios-rest`'s FHIR REST handlers stay UI-agnostic** — the UI depends on
  them, not the reverse.
- **Don't couple templates to a single FHIR version** — go through the
  version-agnostic abstractions already in the workspace.
- **No new component class for a primitive that already exists, and no
  page-local CSS for a shared control.** The vocabulary below is the whole
  list; `.button` next to `.btn` is how the Import page drifted off the design
  (#543). `design-system.spec.ts` fails a page that uses an undefined class,
  restyles a shared primitive, or defines a selector twice.

---

## Component vocabulary

`assets/app.css` is one stylesheet in four cascade layers —
`@layer tokens, base, components, pages` — so a page rule outranks a component
rule by layer, never by accident of specificity or source order. `tokens` holds
the custom properties, `base` element defaults, `components` the shared
vocabulary, `pages` what is genuinely unique to one screen.

These are the shared primitives. Before styling anything, reach for one; add to
`pages` only what no other screen will ever want.

| Class | What it is |
|---|---|
| `.btn`, `.btn--primary`, `.btn--danger`, `.btn--current`, `.btn--icon` | The action button: 30px high, 12px horizontal padding, 12px type, and a 9px radius. Primary, danger, and current change emphasis only; `--icon` makes the control a 30px square with no horizontal padding. |
| `.card`, `.card-head`, `.table-card` | Raised surface; its header row; the padding variant that hosts a table. |
| `.panel` | Padding for a full-width card that hosts detail fields without rail behavior. |
| `.detail__field`, `.detail__field--wide` | One labelled value. The field owns the 5px label/value gap; `--wide` spans all columns when the field is inside a key/value grid. |
| `.detail-stack` | Padding-free vertical composition for detail fields and form actions, with a 12px gap. Direct `.form-actions` children rely on that gap instead of adding their usual top margin. |
| `.kv-grid`, `.kv-grid--flush` | Responsive two-column key/value layout with 14px row and 18px column gaps; it collapses to one column at 1250px. `--flush` removes the grid's trailing margin when its container already provides the bottom inset. |
| `.page-head`, `.page-head__title`, `.page-head__lede`, `.page-head--row` | Page heading block; the only `<h1>` treatment; `--row` puts an action on the right. |
| `.back-link` | In-page return link with theme-safe normal, visited, hover, and focus states. |
| `.table-wrap` > `.data-table`, `.col-num`, `.col-actions`, `.data-table__empty`, `.table-foot` | The table, always in its scroll wrapper: ordinary headers and data align left; `.col-num` uses tabular figures without changing alignment; `.col-actions` aligns right; empty-state rows stay centered; the footer hosts pagination. |
| `.empty-state` | The same centered, muted empty treatment for non-table content. |
| `.field`, `.field__label`, `.field__input`, `.field__hint`, `.field__hint--error` | A labelled form field. |
| `.combobox`, `.combobox__*` | Shared progressively enhanced multi-select. Render it through `partials/combobox.html`; callers provide localized domain copy and an HTML-fragment endpoint, while `combobox.js` owns selection/keyboard state and repeated hidden inputs. Keep a named textarea fallback usable without JavaScript. |
| `.addbox`, `.addbox--modal`, `.addbox__panel`, `.addbox__head`, `.addbox__x`, `.addbox__actions` | The `<details>` disclosure for create/add flows; `--modal` centers it as a dialog. |
| `.choice-grid`, `.choice-card`, `.choice-card__title`, `.choice-card__hint` | The radio-group treatment: one selectable card per choice, `:has(:checked)` accent (#735). |
| `.progress`, `.progress__bar`, `.progress--complete`, `.progress--failed`, `.progress--cancelled` | Full-width job progress track; terminal states recolor the fill. |
| `.job-card`, `.job-card__head`, `.job-card__name`, `.job-card__actions`, `.job-card__meta`, `.job-card__files` | One async job: name + action row, progress track, one meta line, download pills. |
| `.form-legend`, `.field-row` | Standalone section heading between cards; uppercase-labelled fields side by side. |
| `.menu`, `.menu__panel`, `.menu__heading`, `.menu__option` | The `<details>` dropdown (tenant/version selectors, Recent). |
| `.notice`, `.notice--warn` | Inline banner. |
| `.pill` | Large control chip (chart tools). |
| `.tag`, `.tag--*` | Small status pill in tables and lists. |
| `.chip` | Facet/filter chip row member. |
| `.toolbar`, `.toolbar__title`, `.toolbar__search`, `.toolbar__count` | In-card section header with optional search. |
| `.tabs`, `.tab`, `.tab--on` | Tab strip. |
| `.filter-rail`, `.nav-panel` | Left rails: the filter list inside a page; the type panel flush against the sidebar. |
| `.icon-button`, `.icon-button--danger` | Bare 30px-square icon action (table rows), with the action-button 9px radius. |
| `.busy-status` > `.spinner` | Inline working state: the ring plus a short label, `role="status"` in the markup so it announces. |

Labeled values follow one spacing contract: `.detail__field` owns only its 5px
internal label/value gap, while its direct parent owns the larger external
rhythm. Use `.detail-stack`, `.kv-grid`, `.detail`, or `.tester` as that parent;
do not add outer margins to individual fields or place them directly in a
generic `.card__body`.

```text
Parent owns external rhythm (> 5px)

  Wide metadata (> 1250px)                 Compact metadata (<= 1250px)
  ┌──────────────────────────────────┐      ┌─────────────────────────┐
  │ DESCRIPTION — full width         │      │ DESCRIPTION — full width│
  │   ↕ 5px  value                   │      │   ↕ 5px  value          │
  │              ↕ 14px              │      │          ↕ 14px         │
  │ BASE URL — full width            │      │ BASE URL — full width   │
  │   ↕ 5px  value                   │      │   ↕ 5px  value          │
  │              ↕ 14px              │      │          ↕ 14px         │
  │ FHIR VERSION       ← 18px → STATUS│      │ FHIR VERSION            │
  │   ↕ 5px value          ↕ 5px value│      │   ↕ 5px  value          │
  │              ↕ 14px              │      │          ↕ 14px         │
  │ KIND               ← 18px → DATE │      │ STATUS                  │
  │   ↕ 5px value          ↕ 5px value│      │   ↕ 5px  value          │
  │              ↕ 14px              │      │          ↕ 14px         │
  │ FORMATS             │              │      │ KIND                    │
  │   ↕ 5px value       │              │      │   ↕ 5px  value          │
  └──────────────────────────────────┘      │          ↕ 14px         │
                                            │ DATE                    │
                                            │   ↕ 5px  value          │
                                            │          ↕ 14px         │
                                            │ FORMATS                 │
                                            │   ↕ 5px  value          │
                                            └─────────────────────────┘

  `.detail-stack` uses the same contract vertically: 12px between fields,
  while each field keeps exactly 5px between its label and value.
```

Starting a new page: copy `templates/pages/_scaffold.html` (or crib
`tenants.html`, the smallest real page). Both compose only this vocabulary.

Button emphasis never changes geometry: pair `.btn` with `--primary`,
`--danger`, or `--current` for color and state, and add `--icon` only for a
square icon-only action. Inputs keep their own field scale and do not dictate
button height. The sole fixed-height exception is the open
`.addbox--modal > summary.btn`: while its native `<details>` is open, that
summary becomes the full-viewport backdrop (`height: auto`, zero padding and
radius); its closed state and the actions inside the dialog use the canonical
button scale.

### Busy states

One convention for "this control is doing something" (#679), in two lanes:

- **Fetch-driven scripts** call `window.hfsBusy` (`assets/busy.js`).
  `during(buttons, work)` disables the controls, stamps `aria-busy="true"`,
  and clears when the promise `work()` returns settles — pass a *function*,
  never a promise, so the re-entrancy guard runs before the request exists;
  a `work()` that navigates away returns a promise that never settles.
  `region(el, label)` reveals a pre-rendered `.busy-status` element and
  labels its `[data-busy-label]`; `opts.region`/`opts.label` on `during` tie
  one to the same lifetime. The CSS ring keys off `aria-busy`, so the
  visuals cannot ship without the semantics; reduced motion gets the same
  ring as a static glyph. The `::after` ring must keep `content: ""` — CSS
  generated *text* would join the accessible name.
- **htmx controls** use `hx-disabled-elt` (#581); `hx-indicator` is
  deliberately absent (the tenants tests pin this). A pending state that
  outlives the request belongs in the swapped fragment, like the tenants
  provisioning row.

---

## Design source

The visual design is Brett's Figma file
[`CcLtq79cH2aHv4Ii9aNQTP`](https://www.figma.com/design/CcLtq79cH2aHv4Ii9aNQTP/Untitled?node-id=34-2)
— frames "Dashboard V1.1" (34:2, light), "… - Dark" (34:484), and
"… - Tenant Selector" (34:222). Colors, type scale, radii, and shadows in
`assets/app.css` are the inspected values from that file; change them there
first, then here.

Exported from Figma via the REST API (needs a `FIGMA_TOKEN` with file-content
read access):

- `templates/icons/*.svg` — icon nodes exported as SVG, fills normalized to
  `currentColor` so CSS theming applies; inlined with `{% include %}`.
- `assets/logo.png` — the brand mark, exported at 3×.
- `assets/fonts/figtree-*.woff2` — vendored [Figtree](https://fonts.google.com/specimen/Figtree)
  variable font (OFL), embedded like every other asset.

Light/dark theming is CSS custom properties on `:root` / `[data-theme="dark"]`.
Both themes are held to WCAG 2.2 AA, `color-contrast` included, by the axe gate
in `e2e/tests/a11y.spec.ts`.

## Fragment / partial conventions & progressive enhancement

- A **page** (`pages/`) extends `layouts/base.html` and returns a full document.
- A **partial** (`partials/`) returns just the fragment to be swapped — no
  `<html>`/`<head>` wrapper — and is `{% include %}`d into pages so the initial
  full-page render and the htmx swap render identical markup.
- Handlers that back an htmx swap must also work as a **hard navigation**: when
  the `HX-Request` header is absent, return the full page. `/ui/status` is the
  minimal reference implementation of the pattern. Every control needs a real
  `<a href>` / `<form>` underneath so it works with JavaScript disabled — the
  dashboard's window and series selectors are plain links for exactly this
  reason.
- `AutoVaryLayer` emits `Vary: HX-Request` on handlers that read the header, so
  a cache never serves a fragment for a hard navigation. Being on this router
  gets you this for free; don't hand-roll the header.

Relevant htmx request/response headers we rely on: `HX-Request` (present on
htmx-issued requests). See the
[htmx patterns](https://htmx.org/examples/) for active-search, click-to-edit,
inline-validation, and infinite-scroll fragment recipes we'll standardize on.

---

## Pages & routes

Mounted under `/ui` when running `hfs` (the `ui` feature is on by default; the
`headless` feature disables it — the mount is
`cfg(all(feature = "ui", not(feature = "headless")))`, so enabling both yields
no UI).

```bash
cargo run -p helios-hfs   # then open http://127.0.0.1:8080/ui
```

| Route | Method | Page |
|---|---|---|
| `/ui` | GET | Dashboard — stat cards and the "FHIR resources over time" chart |
| `/ui/resources` | GET | Resources workspace: type rail with live counts, search, edit modal |
| `/ui/editor` | GET | Standalone schema-driven resource editor |
| `/ui/editor/render` | POST | Applies every structural mutation and re-renders; the document rides with the request |
| `/ui/json-view/render` | POST | Renders raw `application/json` as a highlighted, foldable HTML fragment; applies no FHIR semantics and retains no payload |
| `/ui/editor/expand` | GET | ValueSet expansion, proxied to `HFS_TERMINOLOGY_SERVER` |
| `/ui/queries` | GET | Saved FHIR queries per resource type (#234) and the visual search builder |
| `/ui/queries/params` | GET | Per-type search-parameter catalog backing the builder's datalist |
| `/ui/search` | GET | Natural-language search — **registered only when NL search is enabled** |
| `/ui/search-parameters` | GET | SearchParameter viewer (#238): rail, facets, paginated table, detail panel, plus the write half |
| `/ui/compartments` | GET | Compartment viewer & route tester (#237): "is this type in this compartment, via which parameters, and what search does the server run?" |
| `/ui/history` | GET | Version rail with from/to selection |
| `/ui/history/diff` | POST | Server-side field-level and word-level diff (`docs/history-diff-rendering.md`) |
| `/ui/batch` | GET | Batch/Transaction workspace (#476) |
| `/ui/tenants` | GET/POST | Tenant maintenance; `/ui/tenants/rows` (GET), `/ui/tenants/{id}` (DELETE) |
| `/ui/status` | GET | System-status read path; the fragment-vs-full-page reference |
| `/ui/version` | POST | Persists the sidebar FHIR-version choice (#343) and redirects back |
| `/ui/tenant`, `/ui/tenant/options` | POST/GET | Tenant selector (#344), options loaded lazily |
| `/ui/assets/*` | GET | Embedded htmx, CSS, JS, fonts, logo |

The router's `fallback_service` is the FHIR app, so anything not under `/ui`
falls through to the normal REST surface.

### Where the data comes from

- **Conformance resources are not vendored into this crate.** SearchParameter and
  CompartmentDefinition are fetched from the server's *own* FHIR API over HTTP on
  the loopback address — storage is the source of truth, seeded per provisioned
  tenant. `ConformanceSource` is the seam; `StaticConformanceSource` stands in
  for it in tests.
- **Dashboard counts and the chart** come from the storage backend through
  `helios_observability::dashboard`, a provider the server registers at startup.
  That keeps this crate free of any persistence dependency for the read path;
  with no provider registered, the dashboard renders placeholder figures through
  the same rendering path. Counts reflect the **default tenant** only — an
  operator view, never exported to the public Prometheus `/metrics` endpoint.
- **Per-user preferences** (theme, nav state, FHIR version, tenant, saved and
  recent queries, and — since #754/#755 — every sidebar rail's `rails.<page>`
  record of `last`/`recent`, tenant-scoped, see `rail_state`) roam in the
  `/_user/settings` document: weak `ETag`, JSON merge patch, `If-Match` on
  write. Tenant-derived keys live under a reserved `byTenant` map so a tenant
  purge can reach them. The server renders the "Recently used" group from
  `recent` on every rail but Compartments — its 4-5 definitions make a group
  noise, so it remembers only `last`; with no settings store configured, there
  is nothing to render and every rail opens on its page default instead.

### Internationalization

All user-visible text resolves from Fluent catalogs at `locales/<locale>/main.ftl`
(workspace root, embedded at compile time). Negotiation order: `?lang=` → the
`hfs_lang` cookie → `Accept-Language` (RFC 4647 Lookup) → `en`. Supported today:
`en`, `es`, `de`. A key missing from a translation falls back to its English
string, and `catalogs_share_the_same_key_set` fails the build-time test suite if
the catalogs drift apart. See `docs/multi-language.md`.

---

## Tests

Two rings, both required.

**Inner ring — Rust, fast.** `tower::oneshot` against the mounted router,
locale middleware included:

```bash
cargo test -p helios-ui
```

`tests/router_http.rs`, `tests/i18n_http.rs`, `tests/tenants_http.rs` (a real
SQLite store, via the test-only `sqlite` feature), plus `mod tests` in most
`src/` modules.

**Outer ring — Playwright + axe-core.** Behavior only a real browser can
observe. Everything Node lives in `e2e/`; the cargo workspace is untouched. See
[`e2e/README.md`](e2e/README.md) for the spec inventory.

```bash
cargo build -p helios-hfs --features ui
cd e2e && npm ci && npx playwright install chromium
npx playwright test
```

The axe gate is strict: every WCAG 2.2 AA rule, `color-contrast` included, in
both themes, is a hard failure. CI runs `ui-tests.yml` per PR on SQLite, and
`ui-tests-matrix.yml` across every storage backend nightly and on demand.

---

## Known follow-ups

- **Outbound service token.** When auth is enabled, the conformance self-call
  relies on an operator provisioning a non-expiring bearer via
  `HFS_OUTBOUND_BEARER_TOKEN`; without one the fetch is rejected and the pages
  degrade to a warning (`pages/compartments-degraded.html`). The fix is a
  short-lived, auto-refreshed token minted through the planned
  `JwtAssertionOutboundAuthProvider` (SMART Backend Services
  `client_credentials` + `private_key_jwt`, `crates/auth/src/outbound.rs`).
- **Asset base path.** `/ui/assets/*` is hardcoded in `layouts/base.html`; it
  would need resolving generically if the mount point ever moves off `/ui`.
- **Richer status read paths** — CodeSystem/ValueSet lookup alongside the
  existing resource counts.

---

## References

- htmx documentation — <https://htmx.org/docs/>
- *Hypermedia Systems* — <https://hypermedia.systems/>
- Locality of Behaviour — <https://htmx.org/essays/locality-of-behaviour/>
- HATEOAS — <https://htmx.org/essays/hateoas/>
- htmx patterns/examples — <https://htmx.org/examples/>
- Axum — <https://docs.rs/axum> · tower-http — <https://docs.rs/tower-http>
- Askama — <https://docs.rs/askama> · Maud — <https://maud.lambda.xyz/> ·
  Minijinja — <https://docs.rs/minijinja>
- rust-embed — <https://docs.rs/rust-embed> · axum-htmx — <https://docs.rs/axum-htmx>
- Fluent — <https://projectfluent.org/> · axe-core — <https://github.com/dequelabs/axe-core>
- OWASP XSS Prevention Cheat Sheet —
  <https://cheatsheetseries.owasp.org/cheatsheets/Cross_Site_Scripting_Prevention_Cheat_Sheet.html>
