# POC: `pysof` + example notebook — the #650 "do-nothing" baseline

The client-library option from the [#650 evaluation](../../docs/sql-on-fhir-analytics-evaluation.md):
instead of embedding a notebook in HFS, **invest in `pysof` and publish great
example notebooks**. HFS stays a pure, governed data source; analysis happens in
the user's own Jupyter or marimo.

## What's here

| File | Purpose |
|------|---------|
| `sql-on-fhir-analytics.ipynb` | Jupyter notebook — two consumption paths (below) |
| `marimo_variant.py` | the same idea as a reactive **marimo** notebook / app |
| `view.json` | the Patient ViewDefinition both paths run |
| `sample_bundle.json` | a 5-patient Bundle, so Path B runs with no server |
| `requirements.txt` | pyarrow, pandas, matplotlib, requests, jupyterlab, marimo, pysof |

Two paths in the notebook:

- **Path A — live HFS, Arrow over HTTP.** `POST /$sql-run?_format=arrow` to a
  running server, read the Arrow IPC stream with `pyarrow`, `to_pandas()`, chart.
  This is the fast wire format (Arrow parses ~114× quicker to a DataFrame than CSV).
- **Path B — `pysof`, fully offline.** `pysof.run_view_definition(view, bundle, "json")`
  runs the same ViewDefinition in-process over `sample_bundle.json` with **no server**.

## Verified

**Path B was run for real** (2026-08-28) against the actual `pysof` build:

```
pysof version: 0.2.1
rows: 5
gender counts: {'female': 2, 'male': 2, 'other': 1}
```

Path A is exercised whenever an HFS server is running at `HFS` (default
`http://localhost:8080`); without one, the notebook says so and falls through to
Path B.

## Run it

```sh
uv venv .venv --python 3.12
uv pip install --python .venv/Scripts/python.exe -r requirements.txt
# pysof: `uv pip install pysof`, or from the repo `cd crates/pysof && maturin develop --release`

# Jupyter:
.venv/Scripts/jupyter.exe lab sql-on-fhir-analytics.ipynb
# or marimo:
.venv/Scripts/marimo.exe run marimo_variant.py
```

For **Path A** (live), first start a server and seed a few Patients:

```sh
cargo run --bin hfs        # http://localhost:8080
# then POST some Patient resources (or use the /ui/batch page)
```

Set `TENANT` in the first cell if auth / multi-tenancy is enabled.
