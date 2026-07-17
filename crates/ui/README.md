# helios-ui — HTMX-first web UI for HFS

This crate is the foundation for a server-rendered, **HTMX-first** web UI for the
Helios FHIR Server. It is a thin Axum library crate that owns templates, static
assets, and view handlers, and is mounted by the `hfs` binary as a sub-router
under `/ui`.

This document is both the discussion doc and the **rules of the road**: when we
build UI in this codebase, this is where things go and why.

---

## Approach: server-rendered HTMX, not a SPA

We render HTML on the server (Rust) and use [htmx](https://htmx.org/docs/) for
partial page updates. Handlers return **full pages** on hard navigations and
**HTML fragments** on `HX-Request`s. State lives on the server; the client stays
thin.

Why, over a SPA + JSON API:

- **Stays close to the FHIR logic we already have.** The UI calls into the
  existing workspace crates (`helios-rest`, `helios-persistence`, `helios-hts`,
  …) and renders the result. There is no second copy of the domain model in a
  browser client, and no client-side view state to keep in sync.
- **No duplicated API surface.** We do not add a browser-facing JSON API.
  htmx consumes HTML fragments, so the FHIR REST surface stays clean and
  UI-agnostic.
- **Progressive enhancement.** Because the server renders real HTML at real
  URLs, the UI degrades to working full-page loads when JavaScript is absent
  (see the `/ui/status` handler). See
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

htmx and CSS are **vendored** under `assets/` (pinned: `htmx.org@2.0.4`) and
**embedded into the binary** at compile time with
[rust-embed](https://docs.rs/rust-embed), served from `/ui/assets/*`.

**Never hotlink a CDN in production.** A healthcare server may run offline or
air-gapped, and a runtime CDN dependency is a supply-chain risk. Embedding also
keeps `hfs` a single self-contained binary — no asset directory to ship
alongside it.

To update htmx, replace `assets/htmx.min.js` with the new pinned release and
note the version bump in the commit.

---

## Rules of the road — where things go

- `crates/ui/src/` — Axum handlers/routers returning `impl IntoResponse`
  (HTML). **Thin:** parse request → call into `helios-rest` /
  `helios-persistence` / `helios-hts` → render a template.
- `crates/ui/templates/` — `.html` templates:
  - `layouts/` — shared document shells (`base.html`).
  - `pages/` — full documents (extend a layout).
  - `partials/` — HTMX-swappable fragments (no `<html>` wrapper).
- `crates/ui/assets/` — vendored, pinned `htmx.min.js`, CSS, images. Embedded;
  never fetched at runtime.
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
- **No inline `<script>` blobs or scattered JS.** Prefer `hx-*` attributes
  (Locality of Behaviour); where JS is truly needed, use small pinned assets.
- **`helios-rest`'s FHIR REST handlers stay UI-agnostic** — the UI depends on
  them, not the reverse.
- **Don't couple templates to a single FHIR version** — go through the
  version-agnostic abstractions already in the workspace.

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

Light/dark theming is CSS custom properties on `:root` /
`[data-theme="dark"]`; `assets/theme.js` (loaded without `defer`, before
first paint) applies the stored or OS preference and handles the top-bar
toggle. The metric cards and chart render sample values from
`DashboardMetrics::sample()` until real read paths land (below).

## Fragment / partial conventions & progressive enhancement

- A **page** (`pages/`) extends `layouts/base.html` and returns a full document.
- A **partial** (`partials/`) returns just the fragment to be swapped — no
  `<html>`/`<head>` wrapper — and is `{% include %}`d into pages so the initial
  full-page render and the htmx swap render identical markup.
- Handlers that back an htmx swap should also work as a **hard navigation**:
  when the `HX-Request` header is absent, return the full page. The POC's
  `/ui/status` handler does exactly this and stays as the working reference
  for the pattern; the dashboard no longer surfaces a control for it, so the
  first real read paths (metric cards, chart) will wire their own swap
  targets the same way — always with a real `<a href>`/`<form>` underneath
  so the control works with JavaScript disabled.

Relevant htmx request/response headers we rely on: `HX-Request` (present on
htmx-issued requests). See the
[htmx patterns](https://htmx.org/examples/) for active-search, click-to-edit,
inline-validation, and infinite-scroll fragment recipes we'll standardize on.

---

## POC in this crate

Mounted under `/ui` when running `hfs` (the `ui` feature is on by default; the
`headless` feature disables it):

- `GET /ui` — full landing page (`pages/index.html` → `layouts/base.html`).
- `GET /ui/status` — a system-status read path. Returns the
  `partials/status.html` **fragment** on `HX-Request`, and the **full page** on
  a hard navigation — demonstrating the same URL working with and without JS.
- `GET /ui/assets/*` — embedded, pinned htmx and CSS.

```bash
cargo run -p helios-hfs --features ui   # then open http://127.0.0.1:8080/ui
```

`cargo build`, `cargo clippy`, and `cargo test -p helios-ui` are clean.

### Left for follow-up work

- Swap the self-contained status snapshot for richer real read paths
  (CodeSystem/ValueSet lookup, resource counts) by calling into
  `helios-persistence` / `helios-hts` from the handlers.
- Resolve the asset base-path generically if the mount point ever moves off the
  hardcoded `/ui` prefix.

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
- rust-embed — <https://docs.rs/rust-embed>
- OWASP XSS Prevention Cheat Sheet —
  <https://cheatsheetseries.owasp.org/cheatsheets/Cross_Site_Scripting_Prevention_Cheat_Sheet.html>
