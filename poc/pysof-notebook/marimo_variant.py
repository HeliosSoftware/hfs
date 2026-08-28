"""marimo variant of the #650 pysof analytics POC.

The reactive-notebook option: same idea as the Jupyter notebook, as a plain .py.

    uv run --with marimo --with requests --with pyarrow --with pandas --with matplotlib \
        marimo run marimo_variant.py      # app mode
    # or `marimo edit marimo_variant.py`  to edit reactively

Path A hits a live HFS at http://localhost:8080. Start one with `cargo run --bin hfs`
and seed a few Patients first; otherwise the cell reports that and stops.
"""

import marimo

app = marimo.App()


@app.cell
def _():
    import json
    import marimo as mo
    import pandas as pd

    HFS = "http://localhost:8080"
    view = {
        "resourceType": "ViewDefinition",
        "status": "active",
        "resource": "Patient",
        "select": [
            {
                "column": [
                    {"name": "id", "path": "id"},
                    {"name": "gender", "path": "gender"},
                    {"name": "birth_date", "path": "birthDate"},
                    {"name": "active", "path": "active"},
                ]
            }
        ],
    }
    mo.md("# SQL-on-FHIR analytics (marimo) &mdash; #650 do-nothing baseline")
    return HFS, json, mo, pd, view


@app.cell
def _(HFS, json, mo, pd, view):
    # Path A: consume a live HFS $sql-run Arrow stream.
    import pyarrow as pa
    import requests

    try:
        r = requests.post(
            f"{HFS}/$sql-run?_format=arrow",
            headers={
                "Content-Type": "application/json",
                "Accept": "application/vnd.apache.arrow.stream",
            },
            data=json.dumps(view),
            timeout=10,
        )
        r.raise_for_status()
        table = pa.ipc.open_stream(pa.py_buffer(r.content)).read_all()
        df = table.to_pandas()
        note = mo.md(f"**{len(df)} rows** from live `$sql-run` (Arrow, zero-copy to pandas)")
    except Exception as e:  # noqa: BLE001
        df = pd.DataFrame()
        note = mo.md(f"Live HFS not reachable &mdash; start `cargo run --bin hfs` and seed Patients.\n\n`{e}`")
    note
    return (df,)


@app.cell
def _(df, mo):
    import matplotlib.pyplot as plt

    if len(df):
        counts = df["gender"].value_counts()
        fig, ax = plt.subplots(figsize=(5, 3))
        counts.plot.bar(ax=ax, color="#2a78d6")
        ax.set_title("Patients by gender")
        ax.set_ylabel("count")
        out = mo.vstack([mo.md(f"### Total patients: {len(df)}"), fig])
    else:
        out = mo.md("_(no data yet)_")
    out
    return


if __name__ == "__main__":
    app.run()
