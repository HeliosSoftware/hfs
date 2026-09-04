---
name: work-with-ui
description: Work on the HFS web UI in crates/ui (helios-ui). Use for Askama templates, htmx fragments, /ui routes, vendored assets and CSS, the schema-driven resource editor, i18n/locales, theme handling, per-user settings, and the Rust + Playwright UI tests.
---

# HFS Web UI (`helios-ui`)

The crate is **`helios-ui` at `crates/ui`** — a thin Axum library crate mounted by
the `hfs` binary as a sub-router under `/ui`. It owns templates, static assets,
and view handlers. `crates/ui/README.md` is the long-form rationale; this skill
is the operational summary.

## Stack: server-rendered htmx, no SPA

There is **no React, Vue, Svelte, Alpine, or jQuery, and no bundler or build
step**, with one narrow exception: CodeMirror 6 is vendored as a prebuilt
bundle via a documented, hand-run ritual under `crates/ui/vendor/codemirror/`
— never executed by `cargo build` or CI. See `crates/ui/README.md` § "The one
exception: a vendored, prebuilt bundle". Do not introduce another one.

| Layer | What we use |
|---|---|
| Templates | **Askama** — Jinja2-like, compiled and type-checked at build time, auto-escaping |
| Interactivity | **htmx 2.0.4**, vendored at `assets/htmx.min.js` |
| Client JS | Hand-written vanilla IIFEs in `assets/*.js` — no framework, no npm deps |
| Asset delivery | `rust-embed` + `axum-embed`, embedded into the binary; **never a runtime CDN** |
| Header handling | `axum-htmx` — `HxRequest` extractor, `AutoVaryLayer` (`Vary: HX-Request`) |
| i18n | `fluent-templates` over `locales/<locale>/main.ftl` at the **workspace root** |

Handlers return a **full page** on a hard navigation and an **HTML fragment** on
`HX-Request`. State lives on the server.

## Running

```bash
# The `ui` feature is on by default in helios-hfs.
cargo run -p helios-hfs            # then open http://127.0.0.1:8080/ui
cargo run -p helios-hfs --features ui

# Headless deployments: `headless` wins over `ui`.
cargo run -p helios-hfs --features headless
```

The mount is `#[cfg(all(feature = "ui", not(feature = "headless")))]` in
`crates/hfs/src/main.rs`. FHIR version features forward through
`helios-ui?/R4|R4B|R5|R6`, so the UI's viewers cover exactly the versions the
server was built with.

## Routes (`crates/ui/src/lib.rs`)

| Route | Method | Page |
|---|---|---|
| `/ui` | GET | Dashboard (stat cards + resources-over-time chart) |
| `/ui/resources` | GET | Resources workspace — type rail, search, edit modal |
| `/ui/editor` | GET | Standalone schema-driven resource editor |
| `/ui/editor/render` | POST | Applies every structural mutation and re-renders; the document rides with the request |
| `/ui/editor/expand` | GET | ValueSet expansion proxy to `HFS_TERMINOLOGY_SERVER` |
| `/ui/queries` | GET | Saved queries + visual search builder |
| `/ui/queries/params` | GET | Per-type search-parameter catalog (datalist) |
| `/ui/search` | GET | Natural-language search — **only registered when NL search is enabled** |
| `/ui/search-parameters` | GET | SearchParameter viewer/CRUD |
| `/ui/compartments` | GET | Compartment viewer + membership tester |
| `/ui/history` | GET | Version rail |
| `/ui/history/diff` | POST | Server-side diff of two versions (see `docs/history-diff-rendering.md`) |
| `/ui/batch` | GET | Batch/Transaction workspace |
| `/ui/tenants` | GET/POST | Tenant maintenance; `/ui/tenants/rows` (GET), `/ui/tenants/{id}` (DELETE) |
| `/ui/status` | GET | Reference implementation of the fragment-vs-full-page pattern |
| `/ui/version` | POST | Persists the sidebar FHIR-version choice, redirects back |
| `/ui/tenant`, `/ui/tenant/options` | POST/GET | Tenant selector |
| `/ui/assets/*` | GET | Embedded htmx, CSS, JS, fonts, logo |

The router `fallback_service` is the FHIR app, so anything not under `/ui` falls
through to the normal REST surface.

## Layout

- `src/` — Axum handlers returning `impl IntoResponse`. **Thin:** parse request →
  call into `helios-rest` / `helios-persistence` / `helios-fhir-validator` →
  render a template. Modules: `lib.rs` (router, dashboard, search, prefs),
  `editor.rs`, `search_params.rs`, `compartments.rs`, `conformance.rs`,
  `history.rs`, `tenants.rs`, `json_view.rs`, `i18n.rs`.
- `templates/layouts/` — `base.html`, the document shell.
- `templates/pages/` — full documents, extend a layout.
- `templates/partials/` — htmx-swappable fragments, no `<html>` wrapper;
  `{% include %}`d into pages so the first render and the swap emit identical markup.
- `templates/icons/*.svg` — Figma exports, fills normalized to `currentColor`, inlined.
- `assets/` — `htmx.min.js` (pinned), `app.css`, `fonts/`, `logo.png`, the
  shared `busy.js` (#679, `window.hfsBusy`), the vendored CodeMirror 6 bundle
  (`vendor/codemirror.bundle.js`, `window.HfsCodeMirror`) with its shared
  mount helper `code-editor.js` (`window.HfsCodeEditor`, #838, also the
  shared JSON token-color preset `jsonHighlight()`, #840), the shared
  guided-form loop `editor-form.js` (`window.HfsEditorForm.attach(root,
  host)`, #843; `host.fields` for constant extra request fields, #840), the
  shared editor/guided-form pairing `editor-pair.js`
  (`window.HfsEditorPair.mount({ textarea, view?, grid, fields? })`, #840,
  extracted out of `vd-editor.js`'s original #843 implementation: two-way
  JSON↔form sync, the validity chip, and the row↔editor cross-highlight),
  and the per-page scripts: `theme.js`, `editor.js`, `resources.js`,
  `saved-queries.js`, `batch.js`, `history.js`, `nl-search.js`,
  `resource-filter.js`, `conformance-crud.js`, `vd-editor.js` (ViewDefinition
  editor, `/ui/sql/view-definitions` — JSON + injected FHIRPath language,
  highlighting, and lint; hands its mounted `EditorView` to
  `editor-pair.js`), `sql-editor.js` (SQL pane editor, `/ui/sql/queries` and
  `/ui/sql/views`), and `sql-library-details.js` (Details JSON editor on
  those same two pages — plain JSON language and highlighting, no lint;
  hands its mounted view, `hidden: "content"` and `legend: "sql-library"`,
  to `editor-pair.js`, #840).

`/ui/sql/queries` and `/ui/sql/views` (`pages/sql-library.html`, one
template keyed by the route's own `LibraryKind`) each edit `Library`
resources of their own SQL on FHIR type code (`sql-query`/`sql-view`): a
title row, a Details section — the Details JSON editor (`sql-library-
details.js`) beside the same guided-form card View Definitions uses,
`content` hidden from it since the SQL card below owns that attachment —,
the SQL card itself (`sql-editor.js`), and a `$sql-run` preview that
follows whichever text is current. Save fuses the two cards' documents
server-side (`sql_libraries::embed_sql`); a document whose type code names
the other kind is rejected with a warning.

  `vd-editor.js` also drives completion and quick fixes (#821), server-
  backed like the async lint above it — the browser only locates the cursor
  in its own syntax tree, `POST /ui/sql/view-definitions/complete` (`kind:
  "key"` for a structural JSON node, `kind: "fhirpath"` for a partial
  expression) answers what fits there. Ctrl-Space opens the popup manually
  (typing opens it too); Enter accepts. Each `/lint` diagnostic's `fixes`
  becomes a button in the hover tooltip and the bottom lint panel — **Ctrl+.**
  applies the one fix under the cursor, or opens the panel when several
  apply (F8/Ctrl-Shift-M reach the panel too, `lintKeymap`); every fix is one
  undoable transaction. Saving with at least one uncorrected error (Save,
  not Duplicate) pops a plural-correct `window.confirm`. To regenerate the
  vendored bundle after touching `entry.js`, see `crates/ui/vendor/
  codemirror/README.md`'s own ritual — it is never run by `cargo build` or CI.

`theme.js` loads **without `defer`**, before first paint, to avoid a FOUC —
and, since #843, is what marks `<html class="js">` for `app.css`'s `.needs-js`
utility (a card that renders inline, server-side, on a page's own first paint
and needs a client-side loop wired to it before it shows — View Definitions'
guided-form card, so far). Every other script is `defer`. Busy/working states
go through `hfsBusy` for fetch-driven code and `hx-disabled-elt` for htmx
controls — see `crates/ui/README.md` § Busy states.

## Rules of the road

Enforced by review, and three of them by `e2e/tests/no-cdn.spec.ts`:

- **No HTML in Rust** string literals or `format!`. All markup lives in templates.
- **No business/FHIR logic in templates.** Templates render data; they don't compute it.
- **No new browser-facing JSON API** to feed the UI — htmx consumes HTML fragments.
- **No inline `<script>` blobs.** Prefer `hx-*` attributes (Locality of Behaviour);
  where JS is genuinely needed, add a small pinned asset. Inert
  `type="application/json"` data carriers are the one allowed exception.
- **No off-origin requests.** No CDN, no remote font, no remote image. HFS may run
  air-gapped, and a runtime CDN is a supply-chain risk.
- **`helios-rest` stays UI-agnostic** — the UI depends on the workspace, never the reverse.
- **Don't couple templates to a single FHIR version** — go through the
  version-agnostic abstractions.
- Every htmx-backed control needs a real `<a href>` / `<form>` underneath so it
  works with JavaScript disabled.

To update htmx, replace `assets/htmx.min.js` with the new pinned release and note
the version bump in the commit message.

### CSS: one vocabulary, four layers

`assets/app.css` is layered — `@layer tokens, base, components, pages`. Shared
primitives live in `components`; put in `pages` only what no other screen wants.
**Never invent a second class for an existing primitive** (`.button` next to
`.btn` is how the Import page drifted, #543) and never restyle a shared control
page-locally. The canonical spellings: `.btn`/`.btn--primary`, `.card` +
`.card-head`, `.page-head__title` (the only `<h1>` class), `.table-wrap` >
`.data-table`, `.field__*`, `.addbox`, `.menu`, `.notice`, `.tag`, `.chip`,
`.toolbar`, `.tabs`/`.tab`, `.filter-rail`, `.icon-button`. Full table:
`crates/ui/README.md` § Component vocabulary. `e2e/tests/design-system.spec.ts`
fails undefined classes, off-canon `<h1>`s, diverging primary buttons, and
duplicate selectors; every full page must be in `e2e/pages/routes.ts`, the one
route list all cross-page guards share. New pages start from
`templates/pages/_scaffold.html`.

## Configuration

The UI reads no configuration of its own; `hfs` passes it in at `mount()`.

| Variable | Effect on the UI |
|---|---|
| `HFS_DATA_DIR` | Spec bundle behind the SearchParameter / CompartmentDefinition viewers |
| `HFS_TERMINOLOGY_SERVER` | Backs `/ui/editor/expand` (binding lookups in the editor) |
| `HFS_NL_SEARCH_ENABLED` | Registers `/ui/search` and the NL toggle |
| `HFS_NL_SEARCH_API_KEY` | Whether NL search is configured vs. showing its setup state |
| `HFS_NL_SEARCH_MODEL` | Shown in the setup state |
| `HFS_OUTBOUND_BEARER_TOKEN` | Credentials for the UI's self-call (below) |
| `HFS_DEFAULT_TENANT`, `HFS_DEFAULT_FHIR_VERSION` | Defaults for the sidebar selectors |

### Conformance data comes over HTTP, from the server itself

SearchParameter and CompartmentDefinition are **not vendored into the UI**. The
crate fetches them from the server's own FHIR API on the loopback address —
storage is the source of truth.

When auth is enabled this self-call needs a valid bearer via
`HFS_OUTBOUND_BEARER_TOKEN`; without one it is rejected and the conformance pages
degrade to a warning (`pages/compartments-degraded.html`, covered by
`e2e/tests/auth/degraded.spec.ts`). A short-lived auto-minted token is the
planned follow-up (`crates/auth/src/outbound.rs`).

### Per-user preferences

Theme, nav state, FHIR version, tenant, saved/recent queries, and — since
#754/#755 — every sidebar rail's `rails.<page>` record of `last`/`recent`
(`rail_state`) roam in the `/_user/settings` document (weak `ETag`, JSON
merge patch, `If-Match` on write). Tenant-derived keys live under a reserved
`byTenant` map so a tenant purge can reach them — see `/run-hfs-server` for
the full semantics. The server renders each rail's "Recently used" group
from `recent`; Compartments is the one exception — it remembers only `last`,
no group, since its 4-5 definitions would make one noise. With no settings
store configured there is nothing to render or restore, and every rail opens
on its page default. `resource-filter.js` now only owns a rail's tooltip and
scroll-to-selection on arrival — recents are server-rendered, not client-side.

### i18n

Negotiation order: `?lang=` → `hfs_lang` cookie → `Accept-Language` (RFC 4647
Lookup) → `en`. Locales are `en`, `es`, `de` at `locales/<locale>/main.ftl`
(repo root, embedded at compile time). A key missing from a translation falls back
to English. Tests enforce key-set parity across locales. See `docs/multi-language.md`.

## Testing

Two rings. Both must pass.

**Inner ring — Rust, fast** (`tower::oneshot` against the mounted router):

```bash
cargo test -p helios-ui
```

`tests/router_http.rs`, `tests/i18n_http.rs`, `tests/tenants_http.rs`, plus
`mod tests` in most `src/` modules. `tenants_http.rs` uses a real SQLite store
(test-only `sqlite` feature on `helios-persistence`).

**Outer ring — Playwright + axe-core** (`crates/ui/e2e/`, self-contained Node;
the cargo workspace is untouched):

```bash
cargo build -p helios-hfs --features ui   # boot.mjs runs the newest target/{release,debug}/hfs
cd crates/ui/e2e && npm ci && npx playwright install chromium
npx playwright test                        # all projects
npx playwright test theme                  # one spec
HFS_E2E_BASE_URL=http://127.0.0.1:8080 npx playwright test   # drive a server you started
```

Specs live in `tests/`; the Page Object Model lives in `pages/`, wired onto
`test` via `pages/fixtures.ts` — specs import `{ test, expect }` from
`../pages/fixtures`. `pages/api.ts` seeds resources over the REST API.

The axe gate is strict: **every WCAG 2.2 AA rule, including `color-contrast`, in
both light and dark, is a hard failure.** The `nojs` project asserts the UI still
works with JavaScript disabled.

CI: `ui-tests.yml` per PR (SQLite, fast); `ui-tests-matrix.yml` manual + nightly
across every storage backend.

## Gotchas

- **Askama fails the build**, not the request — a template referencing a missing
  field is a compile error. That is the point; don't route around it.
- `rust-embed` has `debug-embed` on, so debug builds embed assets too. Editing a
  file under `assets/` or `templates/` needs a **rebuild** to take effect.
- Assets are served with `Cache-Control: no-cache` plus a content ETag: unchanged
  assets 304, rebuilt ones always re-fetch. Don't "fix" a stale asset with a
  cache-busting query string.
- `AutoVaryLayer` emits `Vary: HX-Request` so a cache never serves a fragment for
  a hard navigation. A new handler that reads `HX-Request` gets this for free by
  being on this router — don't hand-roll the header.
- `/ui/search` does not exist when NL search is disabled; a test that navigates
  there unconditionally will 404.
- The `headless` feature is checked as `not(feature = "headless")`, so enabling
  both `ui` and `headless` yields **no UI**.
