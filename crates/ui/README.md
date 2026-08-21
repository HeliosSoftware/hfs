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
npm dependency, and no build step for the browser code.** The only vendored
third-party script is htmx itself; everything else under `assets/` is
hand-written vanilla JS in an IIFE. Do not introduce a framework.

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

### Client-side scripts

Each is a small, self-contained IIFE loaded with `defer` by the one page that
needs it — except `theme.js`, which loads **without `defer`**, before first
paint, to avoid a flash of the wrong theme.

| Asset | Owns |
|---|---|
| `theme.js` | Light/dark preference: stored choice → OS preference, plus the top-bar toggle |
| `saved-queries.js` | Saved queries, the visual search builder, and the `/_user/settings` read/modify/write cycle |
| `editor.js` | The schema-driven editor loop — posts the document to `/ui/editor/render` and swaps in the server's HTML |
| `resources.js` | The Resources workspace edit modal and "Create new" |
| `batch.js` | Bundle pick → execution plan → per-entry outcomes |
| `history.js` | Version selection and diff requests |
| `nl-search.js` | Natural-language search mode (only loaded when configured) |
| `resource-filter.js`, `conformance-crud.js` | The conformance viewers' rail filter and write half |

`editor.js` is deliberately thin, and that is the architectural point: it does
not model the resource, know what a choice type is, or understand cardinality.
All of that lives in Rust behind `/ui/editor/render`, where it is tested.

---

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
- **No SPA framework, no bundler, no npm dependency** for browser code. (The
  `e2e/` directory has a `package.json`, but that is test-only and never ships.)
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
| `.btn`, `.btn--primary`, `.btn--danger`, `.btn--current` | The button. Secondary by default; primary is the one blue action on a page. |
| `.card`, `.card-head`, `.table-card` | Raised surface; its header row; the padding variant that hosts a table. |
| `.page-head`, `.page-head__title`, `.page-head__lede`, `.page-head--row` | Page heading block; the only `<h1>` treatment; `--row` puts an action on the right. |
| `.table-wrap` > `.data-table`, `.data-table__empty`, `.table-foot` | The table, always in its scroll wrapper; empty-state row; footer with pagination. |
| `.field`, `.field__label`, `.field__input`, `.field__hint`, `.field__hint--error` | A labelled form field. |
| `.addbox`, `.addbox--modal`, `.addbox__panel`, `.addbox__head`, `.addbox__x`, `.addbox__actions` | The `<details>` disclosure for create/add flows; `--modal` centers it as a dialog. |
| `.menu`, `.menu__panel`, `.menu__heading`, `.menu__option` | The `<details>` dropdown (tenant/version selectors, Recent). |
| `.notice`, `.notice--warn` | Inline banner. |
| `.pill` | Large control chip (chart tools). |
| `.tag`, `.tag--*` | Small status pill in tables and lists. |
| `.chip` | Facet/filter chip row member. |
| `.toolbar`, `.toolbar__title`, `.toolbar__search`, `.toolbar__count` | In-card section header with optional search. |
| `.tabs`, `.tab`, `.tab--on` | Tab strip. |
| `.filter-rail`, `.nav-panel` | Left rails: the filter list inside a page; the type panel flush against the sidebar. |
| `.icon-button`, `.icon-button--danger` | Bare icon action (table rows). |

Starting a new page: copy `templates/pages/_scaffold.html` (or crib
`tenants.html`, the smallest real page). Both compose only this vocabulary.

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
  recent queries) roam in the `/_user/settings` document: weak `ETag`, JSON merge
  patch, `If-Match` on write. Tenant-derived keys live under a reserved
  `byTenant` map so a tenant purge can reach them.

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
