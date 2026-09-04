# CodeMirror 6 vendoring ritual

`crates/ui` has no bundler and no browser build step (see the "no bundler, no
CDN" rules in `crates/ui/README.md`). CodeMirror 6 is distributed as ESM
packages that assume one. This directory is the exception introduced for the
richer ViewDefinition editor evaluated in issue #753: a **one-off vendoring
ritual** — run by hand, on demand, never from `cargo build`, `build.rs`, or
CI — that produces a single vendored artifact:

```
crates/ui/assets/vendor/codemirror.bundle.js
```

That file is a minified IIFE with no `import`/`export`, no dynamic `import()`,
and no external URLs. Loaded with a plain `<script>` tag, it defines exactly
one global: `window.HfsCodeMirror`. The assignment is explicit
(`window.HfsCodeMirror = ...`, falling back to `globalThis` when `window` is
undefined) rather than left to Rollup's implicit iife `var` — see the header
comment in `src/entry.js` for why. Nothing else in this crate depends on npm,
Node, or this directory at build or run time — `node_modules/` here is
gitignored, and the checked-in bundle is the only thing the rest of the crate
ever sees.

## Why this exists (and why it's the exception)

`crates/ui/README.md` states that htmx is the only vendored third-party
script and that there is no bundler in this crate. CodeMirror 6 cannot be
vendored as a single hand-copied file the way `htmx.min.js` is — its packages
are ESM modules with a dependency graph, and there is no published UMD/IIFE
build that includes a custom language mix (JSON host + injected FHIRPath).
Bundling is unavoidable to get from "a dozen npm packages" to "one script tag".

The ritual is deliberately narrow: it runs only when someone chooses to
update the bundle, its inputs (`package.json` + `package-lock.json`) are
pinned and committed, and its one output is committed too. Nothing about
`cargo build`, the `Dockerfile`, or `.github/workflows/` changes because of
it — see [Constraints](#constraints-what-this-ritual-must-never-do) below.

## Regenerating the bundle

Requires **Node ≥ 20** (works the same in Git Bash, PowerShell, or a Linux
shell — there is nothing platform-specific in the recipe).

```sh
cd crates/ui/vendor/codemirror
npm install     # installs into a local, gitignored node_modules/
npm run build   # rollup + terser -> ../../assets/vendor/codemirror.bundle.js
```

Then verify and commit the regenerated file:

```sh
git status crates/ui/assets/vendor/codemirror.bundle.js
git add crates/ui/vendor/codemirror/package-lock.json \
        crates/ui/assets/vendor/codemirror.bundle.js
```

`npm install` only ever touches `package-lock.json` when a pinned version
actually changes — commit it alongside the bundle so the two never drift.
Two runs against the same lockfile produce a byte-identical output (rollup
and terser emit no timestamps or non-deterministic ordering here); if a
regenerated bundle differs unexpectedly from what's committed, diff it before
assuming the change is intentional.

### Updating a pinned version

1. Edit the exact version in `package.json` (no `^`/`~` — this project pins
   every dependency here exactly, including transitive packages this bundle
   imports from directly).
2. Run `npm install && npm run build` as above.
3. Re-run the size and content checks below and update the numbers in this
   README.
4. Commit `package.json`, `package-lock.json`, and the regenerated bundle
   together with the version bump noted in the commit message.

## What's bundled

| Package | Version | License |
|---|---|---|
| `codemirror` | 6.0.2 | MIT |
| `@codemirror/state` | 6.7.1 | MIT |
| `@codemirror/view` | 6.43.9 | MIT |
| `@codemirror/language` | 6.12.4 | MIT |
| `@codemirror/commands` | 6.11.0 | MIT |
| `@codemirror/autocomplete` | 6.20.3 | MIT |
| `@codemirror/lint` | 6.9.7 | MIT |
| `@codemirror/search` | 6.7.1 | MIT |
| `@codemirror/lang-json` | 6.0.2 | MIT |
| `@codemirror/lang-sql` | 6.10.0 | MIT |
| `@lezer/common` | 1.5.2 | MIT |
| `@lezer/highlight` | 1.2.3 | MIT |
| `lezer-fhirpath` | 1.2.0 | MIT [^lezer-fhirpath-license] |

[^lezer-fhirpath-license]: Declared in `README.md` § License of the tarball published on npm
    (`lezer-fhirpath@1.2.0`,
    <https://registry.npmjs.org/lezer-fhirpath/-/lezer-fhirpath-1.2.0.tgz>, published
    2025-12-08), verified 2026-09-03. `package.json` in that tarball has no `license` field, and
    the upstream repository (<https://github.com/HealthSamurai/lezer-fhirpath>) returned 404 on
    that date.

The first thirteen are all published by the CodeMirror/Lezer project (Marijn
Haverbeke and others) under the MIT license; the bundle's banner comment
carries the required copyright notice.

`@lezer/lr` — a transitive dependency of `@codemirror/lang-sql` — is
deliberately not added to `dependencies` or to `BUNDLED_PACKAGES`: `src/entry.js`
never imports from it directly, exactly as with `@lezer/lr` under
`@codemirror/lang-json` today.

`lezer-fhirpath` (the FHIRPath grammar for CM6, published by HealthSamurai)
declares no `license` field in its `package.json` and ships no `LICENSE` file
in its published tarball — `npm view lezer-fhirpath` falls back to labeling
it "Proprietary", which is npm's own placeholder for "unspecified", not a
license anyone has actually granted. This was a known, accepted risk for the
evaluation proof-of-concept #753 shipped it in: writing a replacement grammar
then would have meant implementing before the evaluation had concluded. The
risk is now closed — the package's published README does declare MIT (see
the table footnote above) — with the one reservation that the declaration
lives in that README rather than a `package.json` `license` field or a
`LICENSE` file in the tarball.

`rollup`, `@rollup/plugin-node-resolve`, and `@rollup/plugin-terser` are
`devDependencies` (build-time only; nothing about them ships in the output).

Every version and license string above is generated at build time by
`rollup.config.js` reading each package's own `package.json` — both in the
bundle's banner comment and in the runtime `HfsCodeMirror.version` object —
so this table is the only place they are written by hand, purely for a human
skimming this file.

## Bundle contents (`window.HfsCodeMirror`)

`HfsCodeMirror` is a **flat namespace**: every CodeMirror/Lezer export listed
in `src/entry.js` lives directly on it (e.g. `HfsCodeMirror.EditorState`,
`HfsCodeMirror.json`, `HfsCodeMirror.linter`), with two exceptions kept
nested because they are not part of CodeMirror's own export surface:

- `HfsCodeMirror.fhirpath` — `{ parser, props, terminals }` from
  `lezer-fhirpath`.
- `HfsCodeMirror.version` — `{ "<package>": "<version>" }` for every package
  in the table above, generated from each package's `package.json`.

`@codemirror/lang-sql` contributes `HfsCodeMirror.sql`, `.SQLDialect`,
`.StandardSQL`, `.SQLite`, `.keywordCompletionSource`, and
`.schemaCompletionSource` — the language factory, the dialect it takes a
`dialect:` config of, the generic and SQLite-flavored built-in dialects, and
the two completion sources autocomplete wires up. The package's other
built-in dialects (MySQL, PostgreSQL, ...) are not exposed: `$sqlquery-run`
executes on SQLite (rusqlite), so only that dialect (plus `StandardSQL` and
the `SQLDialect` class, kept in case a future dialect swap needs them) is
re-exported.

`@codemirror/autocomplete` and `@codemirror/lint` each gained more of their
own API surface (#821), beyond the editor-wiring names bundled since #753:
`snippetCompletion`, `snippet`, `startCompletion`, `closeCompletion`,
`acceptCompletion`, `completionStatus`, `currentCompletions`, and
`insertCompletionText` build and drive individual completions;
`forEachDiagnostic`, `openLintPanel`, `closeLintPanel`, `nextDiagnostic`,
`previousDiagnostic`, and `diagnosticCount` inspect and navigate the lint
panel's diagnostics. `@codemirror/view` adds `hoverTooltip` and
`showTooltip`, the primitives a hover tooltip is built from. None of these
change how an editor mounts — they are building blocks the ViewDefinition
editor's completion and quick-fix UI consumes directly off the global.

See `src/entry.js` for the full, explicit list of re-exported names — it is
the single source of truth the ViewDefinition editor's completion and
diagnostics code builds against.

## Measured sizes

Measured against the bundle committed with this regeneration (`node -e` using
`zlib.gzipSync`/`brotliCompressSync` at max quality):

| Encoding | Bytes | Delta vs. #838 |
|---|---|---|
| raw (uncompressed) | 446 874 | +5 427 |
| gzip | 145 146 | +1 989 |
| brotli | 123 117 | +1 635 |

"#838" is the bundle committed there (441 447 / 143 157 / 121 482), before
#821 added the `@codemirror/autocomplete`, `@codemirror/lint`, and
`@codemirror/view` exports listed above.

Budget: raw bundle must stay **≤ 500 000 bytes**. Current margin: ≈ 52 KB.

The gzip/brotli figures are informational — what compressing this exact
artifact yields — not what today's server response carries. See
[Serving](#serving) below: nothing in this crate ships a precompressed sibling
file yet, for any asset, so every asset (this bundle included) is currently
served as `identity` regardless of the client's `Accept-Encoding`.

## Constraints (what this ritual must never do)

- **Never runs automatically.** No `build.rs`, no `cargo build`/`cargo test`
  step, no `.github/workflows/` job invokes `npm` or `rollup`. This directory
  is inert to Cargo; only a human runs `npm run build` here.
- **`node_modules/` is never committed** (see `.gitignore`); `package.json`
  and `package-lock.json` are, so the ritual is reproducible.
- **One artifact, one global.** The only output this ritual commits is
  `../../assets/vendor/codemirror.bundle.js`; it defines exactly
  `window.HfsCodeMirror` and touches no other global.
- **No `eval`, `new Function`, or `document.write`** in the output (checked
  by hand on every regeneration; also asserted by the `helios-ui` test that
  loads this bundle).
- This directory is separate from `crates/ui/e2e/` — the Playwright/axe-core
  toolchain for this crate's browser tests. Nothing here participates in that
  tooling, and nothing in `e2e/` depends on this directory existing.

## Serving

No change to how assets are declared: `rust-embed`'s `#[folder = "assets/"]`
already walks subdirectories — `assets/fonts/` is the existing precedent —
so `assets/vendor/codemirror.bundle.js` is picked up automatically and served
at `GET /ui/assets/vendor/codemirror.bundle.js` with the same
`Cache-Control: no-cache` and content-hash `ETag` as every other asset in
this crate.

`axum-embed` *can* negotiate `br`/`gzip`/`deflate`, but it never compresses
on the fly: for a request path `p`, it serves a compressed body only if a
sibling `p.br` / `p.gz` / `p.zz` also exists among the *embedded* files (and
the client's `Accept-Encoding` asks for that scheme) — otherwise it falls
back to `identity`, independent of what the client accepts. Today `crates/ui`
checks in no precompressed sibling for any asset (`htmx.min.js`, `app.css`,
…), so `codemirror.bundle.js` — like every other asset in this crate — is
currently always served uncompressed. Nothing about this bundle disables
compression; the same "no asset compresses yet" is true crate-wide, and
enabling it (checking in `.br`/`.gz` variants, or fronting the server with a
compressing proxy) is a separate, crate-wide change out of scope for this
ticket.
