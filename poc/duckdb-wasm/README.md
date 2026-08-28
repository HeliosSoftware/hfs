# POC: client-side analytics over a `$sql-run` Arrow IPC stream (HFS #650, Stage 2)

This proof-of-concept closes the one gap the #650 evaluation never actually
exercised: **DuckDB-WASM ingesting a live Arrow IPC stream in the browser and
aggregating it** — no server-side GROUP BY, no round-trip.

## What it demonstrates

A static page (`index.html`) that, entirely in the browser:

1. `fetch()`es `sample.arrow` — a real Arrow IPC **stream** produced by
   `sof-cli --format arrow` (the exact wire format an HFS `$sql-run` endpoint
   streams) — as an `ArrayBuffer`.
2. Parses it with the vendored **apache-arrow** UMD build to report the source
   row count (proves the stream round-trips through the Arrow JS lib too).
3. Boots **DuckDB-WASM** from **explicit local bundle URLs** — never jsdelivr —
   via `selectBundle({ mvp/eh -> vendor/duckdb-eh.wasm + vendor/duckdb-browser-eh.worker.js })`.
4. Ingests the raw IPC bytes into an in-memory table `t` with
   `conn.insertArrowFromIPCStream(bytes, { name: "t", create: true })`.
5. Runs `SELECT gender, count(*) n FROM t GROUP BY gender ORDER BY n DESC`.
6. Renders the result as a **table** and an **inline SVG bar chart**, and shows
   the running SQL, the source row count, and the DuckDB build in use.

`sample.arrow` was generated with the real `sof-cli` binary (not synthesized
with pyarrow) from a 500-patient Patient `ViewDefinition` with columns
`id, gender, birth_date, active, city`. Gender distribution:
female = 275, male = 200, other = 25.

## Run it

The page loads ES modules, a Web Worker, and a `.wasm` file — and the Arrow ESM
is served as ~158 `.mjs` modules — so it **must be served over HTTP** (browsers
block modules/workers/WASM from `file://`) with **correct MIME types**. A tiny
`serve.py` sets those; plain `python -m http.server` mis-serves `.mjs` as
`text/plain` on some platforms (notably Windows) and the modules fail to load.

From this directory (`poc/duckdb-wasm/`):

```sh
python serve.py            # defaults to :8095; `python serve.py 9000` to change
```

Then open <http://localhost:8095/> in a modern browser (Chrome/Edge/Firefox).
The pipeline log fills in top-to-bottom and the chart appears in a second or two.

**Browser-verified** (Playwright, 2026-08-28): loads with **0 console errors**,
makes **0 off-origin requests**, ingests the 500-row `sample.arrow`, runs the
GROUP BY, and renders female = 275 / male = 200 / other = 25. See
`browser-verified.png`.

> The first load reads the ~36 MB `vendor/duckdb-eh.wasm` from disk; it is
> instant on localhost. Nothing is downloaded.

## What it took to run offline (the honest bits)

Vendoring DuckDB-WASM + Arrow for a **no-bundler, no-CDN** page surfaced three
real integration problems, all fixed in `index.html` / `serve.py`:

1. **Bare import specifiers.** The DuckDB ESM does `import … from "apache-arrow"`,
   and Arrow's ESM in turn imports `tslib` and `flatbuffers` — bare specifiers a
   browser cannot resolve without a bundler. Fixed with an **import map** pointing
   each at the locally-vendored ESM build (`vendor/arrow/Arrow.dom.mjs`,
   `vendor/tslib.es6.mjs`, `vendor/flatbuffers/flatbuffers.js`).
2. **`.mjs` MIME.** `serve.py` serves `.mjs`/`.js` as `text/javascript` and
   `.wasm` as `application/wasm`. (`duckdb-browser.js` keeps a `.js` name so even
   a naive server gets it right.)
3. **Worker-relative URL doubling.** The DuckDB worker resolves `mainModule`
   relative to *itself* (it lives in `vendor/`), so a bare `vendor/duckdb-eh.wasm`
   became `vendor/vendor/…` (404). Fixed by handing `selectBundle()` **absolute**
   URLs anchored to the document base.

These are exactly the vendoring costs the #650 evaluation flagged for putting a
WASM engine behind the no-CDN guard — now measured, not assumed.

## No CDN / self-hosted

Every asset is local. There are **no runtime fetches** to jsdelivr, unpkg,
cdnjs, or PyPI. `index.html` references only `./vendor/*` and `./sample.arrow`.

`vendor/` contains, copied from the `@duckdb/duckdb-wasm` (1.33.1) and
`apache-arrow` (21.2.0) npm packages:

| File | Purpose |
|------|---------|
| `duckdb-browser.js` | DuckDB-WASM async API (ESM; named `.js`, not `.mjs`, so `http.server` serves it as `text/javascript` — see note below) |
| `duckdb-eh.wasm` | DuckDB engine, exception-handling build |
| `duckdb-browser-eh.worker.js` | DuckDB Web Worker (eh build) |
| `Arrow.es2015.min.js` | apache-arrow UMD (`window.Arrow`) — used by the page to parse the IPC and report the source row count |
| `arrow/` (ESM tree, ~158 `.mjs`) | apache-arrow ESM — what DuckDB's `import "apache-arrow"` resolves to via the import map |
| `tslib.es6.mjs` | Arrow ESM runtime dep (import-mapped `tslib`) |
| `flatbuffers/` (ESM tree) | Arrow ESM runtime dep (import-mapped `flatbuffers`) |

DuckDB-WASM's default loader would pull its `.wasm` + worker from jsdelivr; this
POC overrides that by handing `selectBundle()` explicit local URLs.

## Note on threads / COOP / COEP

DuckDB-WASM's multi-threaded **coi** build requires the page to be
*cross-origin isolated* — the server must send `Cross-Origin-Opener-Policy:
same-origin` and `Cross-Origin-Embedder-Policy: require-corp`. A plain
`python -m http.server` sends neither.

This POC therefore vendors and selects the single-threaded **eh**
(exception-handling, non-coi) build, which runs **without** COOP/COEP headers.
That is why it works from a bare static file server. (A production HFS
deployment that wanted the threaded build would add those two response headers.)

## Regenerating `sample.arrow`

Requires the release `sof-cli` binary. The generator (`gen_arrow_inputs.py`)
lives beside this README. From this directory (`poc/duckdb-wasm/`):

```sh
# build once, from repo root: cargo build --release --bin sof-cli
python gen_arrow_inputs.py                        # writes view.json + bundle.json here
../../target/release/sof-cli --view view.json --bundle bundle.json \
    --format arrow --output sample.arrow
```

`gen_arrow_inputs.py` builds the 500-patient bundle and the
`patient_demographics` ViewDefinition used above (`view.json` / `bundle.json`
are throwaway intermediates and can be deleted afterward).
