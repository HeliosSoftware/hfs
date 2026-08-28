# POC: embedded JupyterLite notebook (the #650 escape hatch)

The **opt-in escape hatch** from the [#650 evaluation](../../docs/sql-on-fhir-analytics-evaluation.md):
a full Python notebook, compiled to WebAssembly, running **entirely in the
browser** — no server-side interpreter. It's the runner-up, not the primary
recommendation, because of its size and shape (a Python IDE, not governed BI).

The built site is 70–463 MB, so it is **not committed**. This POC is the wiring —
a build script, a `.gitignore`, and a pre-seeded sample notebook — that
reproduces a runnable site.

## What's here

| File | Purpose |
|------|---------|
| `build.sh` / `build.ps1` | build a JupyterLite site into `dist/` |
| `content/hfs-sql-on-fhir.ipynb` | sample notebook: calls `$sql-run` from the browser (Pyodide), charts with pandas + matplotlib |
| `.gitignore` | excludes the built `dist/` and the build `.venv/` |

## Run it

```sh
sh build.sh                                   # or:  ./build.ps1  on PowerShell
python -m http.server 8080 --directory dist   # then open http://localhost:8080/lab
```

`build.sh` runs `jupyter lite build --contents content`, so the sample notebook
appears in the Lab file browser. Open it and run the cells.

## The no-CDN reality (measured, #650 §10)

| Build | Command | Off-origin at runtime | Size |
|-------|---------|----------------------|------|
| **Default** | `build.sh` as-is | **Yes** — Pyodide from `cdn.jsdelivr.net` | 70 MB |
| **Vendored** | uncomment the `--pyodide <tarball>` line | **No** — 0 off-origin (browser-verified in the spike) | **463 MB** |

Only the vendored build passes the HFS no-CDN Playwright guard, and it costs
~463 MB (≈3× the largest binary in the workspace) — which is why JupyterLite is
the escape hatch behind an opt-in feature, never in the default image.

## To actually embed in HFS

Serve `dist/` as vendored static assets under a route on the **HFS origin**
(e.g. `/ui/notebook/`), so the notebook's `fetch("/$sql-run")` is same-origin and
rides the UI's session. Two things to resolve first (see the evaluation's Open
Questions):

- **CSP / isolation** — Pyodide needs `script-src 'wasm-unsafe-eval'`; give the
  notebook subtree its own CSP and consider a sandboxed/opaque-origin iframe.
- **IndexedDB persistence** — JupyterLite stores notebooks per browser profile,
  shared across tenants on a shared machine; scope or disable it near PHI.
