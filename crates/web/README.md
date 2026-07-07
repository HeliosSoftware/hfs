# helios-web — HTMX-first web UI (foundation)

> Status: **proof of concept / foundation.** This crate establishes *where UI
> code goes and why*. It is not a finished application.
>
> Tracking issue: [#186](https://github.com/HeliosSoftware/hfs/issues/186)

This document is the "rules of the road" for building browser UI in HFS. Read it
before adding a screen.

---

## 1. Approach — and why

HFS is a Rust/Axum server that already owns all the FHIR logic (persistence,
terminology, SQL-on-FHIR, auth). We render UI **on the server** and use
[HTMX](https://htmx.org/) for partial page updates, instead of building a
separate single-page application (SPA).

**Why server-rendered hypermedia over an SPA:**

- **One source of truth.** An SPA duplicates view logic and state on the client
  and forces us to expose a second, browser-shaped JSON API alongside the FHIR
  API. Hypermedia keeps rendering and state in Rust, next to the data.
- **Less JavaScript to own.** HTMX adds `hx-*` attributes to plain HTML; there is
  no build step, bundler, or client framework to maintain.
- **Fits a healthcare deployment.** Assets are vendored and served locally — no
  runtime CDN dependency, which matters for air-gapped / regulated installs.

The canonical references for this style:

- **HTMX docs** — <https://htmx.org/docs/>
- **"Hypermedia Systems"** (Gross, Stepinski, Akşimşek), the book-length rationale
  — <https://hypermedia.systems/>
- **HATEOAS** — <https://htmx.org/essays/hateoas/>
- **Locality of Behaviour (LoB)** — <https://htmx.org/essays/locality-of-behaviour/>
- **HTMX examples** (active search, click-to-edit, inline validation, infinite
  scroll — the fragment patterns we standardize on) — <https://htmx.org/examples/>

## 2. Templating engine

The POC uses **[Askama](https://docs.rs/askama)**: Jinja2-like templates checked
**at compile time**, so a bad field reference or missing template is a build
error, not a production 500. Askama **auto-escapes HTML by default**, which is
our first line of defense against XSS (see
[OWASP XSS Prevention](https://cheatsheetseries.owasp.org/cheatsheets/Cross_Site_Scripting_Prevention_Cheat_Sheet.html)).

Alternatives considered (revisit if the POC assumptions break down):

| Engine | Model | Trade-off |
|--------|-------|-----------|
| **Askama** (chosen) | Compile-time, typed | Safest; templates are part of the build; least runtime flexibility |
| [Maud](https://maud.lambda.xyz/) | Macro, Rust-in-HTML | Type-safe, no template files; markup lives in `.rs`, which some find harder to scan |
| [Minijinja](https://docs.rs/minijinja) | Runtime | Hot-reload, dynamic templates; errors surface at runtime |

**This is the one decision most worth revisiting with the team** before the UI
grows large. It is called out in the issue for that reason.

## 3. File placement — rules of the road

### Where things go

```
crates/web/
├── src/            Axum handlers + router. THIN: parse → call HFS crates → render.
├── templates/
│   ├── layouts/    Shared page shells (base.html).
│   ├── pages/      Full HTML documents (one per route/navigation target).
│   └── partials/   HTMX-swappable fragments (NOT full documents).
├── assets/         Vendored, version-pinned static files (htmx.min.js, app.css).
└── examples/       Standalone runner(s) for local development.
```

- Handlers branch on the **`HX-Request`** header: return a **fragment** to HTMX,
  the **full page** on a hard navigation / bookmarked URL. This keeps the UI
  working (degraded) without JavaScript — progressive enhancement.
- Data comes from the existing crates (`helios-rest`, `helios-persistence`,
  `helios-hts`, …). `helios-web` orchestrates and renders; it does not own data
  logic.

### Where things must NOT go

- ❌ **No HTML in Rust source** (`format!`, string literals). Markup lives in
  `templates/`.
- ❌ **No business / FHIR logic in templates.** Templates render data; they don't
  compute it. Don't re-implement persistence or terminology logic here.
- ❌ **No browser-facing JSON API for the UI.** HTMX consumes HTML fragments. The
  FHIR JSON API stays for API clients, not for the browser.
- ❌ **No inline `<script>` blobs or scattered JS.** Prefer `hx-*` attributes
  (Locality of Behaviour). If you truly need scripting, add a small **pinned**
  asset — never a CDN `<script>` at runtime.
- ❌ **Don't make `helios-rest` depend on `helios-web`.** The FHIR REST surface
  stays UI-agnostic; the dependency arrow points UI → API, never the reverse.
- ❌ **Don't hard-code a FHIR version** in templates/handlers; go through the
  workspace's version-agnostic abstractions.

## 4. Assets & security policy

- HTMX and CSS are **vendored** into `assets/` and **version-pinned** (currently
  `htmx.org@2.0.4`). Upgrades are deliberate commits, reviewed like any dep.
- **No runtime CDN.** Offline/air-gapped installs and supply-chain hygiene.
- Rely on Askama's **auto-escaping**; never build HTML by string concatenation.
  A future hardening pass should add a Content-Security-Policy header.
- For single-binary deploys, assets can later be embedded with
  [`rust-embed`](https://docs.rs/rust-embed) instead of `ServeDir`.

## 5. Running the POC

```bash
cargo run -p helios-web --example serve
# open http://127.0.0.1:8088/  and type in the search box
cargo test -p helios-web
```

The search box is backed by a **static in-memory dataset** (FHIR resource-type
names) to keep the skeleton self-contained. Replacing it with a **real read
path** (e.g. a CodeSystem/ValueSet lookup through `helios-hts`) is the first
follow-up task.

## 6. Known follow-ups (for the mount step)

- **Mount into `hfs`.** The router is exposed as `helios_web::router()`; the host
  binary nests it, e.g. `app.nest("/ui", helios_web::router())`. When mounting
  under a prefix, asset URLs need a base path — the POC uses root-relative
  `/assets/...` (correct for the standalone example). Decide between: serving
  assets at the site root, threading a `base_path` into the layout template, or
  embedding assets. **Do not** ship the prefix mismatch.
- **Real data source + auth.** Wire handlers to actual read paths and respect the
  existing auth/tenant context.
- **Feature-gate the UI** so headless deployments can compile it out.
