"""Client-side parse benchmark: CSV vs Arrow IPC consumption of $sql-run output.

Generates a large patient bundle, has sof-cli render it as CSV and as an Arrow
IPC stream (the two wire formats a live-query client could receive), then
times what a Python analysis client must do with each to obtain a usable
table. Build sof-cli first (cargo build --release --bin sof-cli), then:
    uv run --with pyarrow --with pandas python crates/sof/scripts/bench_arrow_client.py
"""

import json
import tempfile
import statistics
import subprocess
import sys
import time
from pathlib import Path

N_PATIENTS = 50_000
REPEATS = 5
REPO_ROOT = Path(__file__).resolve().parents[3]
WORKDIR = Path(tempfile.gettempdir()) / "sof_arrow_bench"

VIEW = {
    "resourceType": "ViewDefinition",
    "status": "active",
    "resource": "Patient",
    "select": [
        {
            "column": [
                {"name": "id", "path": "id"},
                {"name": "gender", "path": "gender"},
                {"name": "birth_date", "path": "birthDate"},
                {"name": "family", "path": "name.first().family"},
                {"name": "given", "path": "name.first().given.first()"},
                {"name": "city", "path": "address.first().city"},
                {"name": "active", "path": "active"},
            ]
        }
    ],
}


def generate_inputs() -> tuple[Path, Path]:
    WORKDIR.mkdir(exist_ok=True)
    view_path = WORKDIR / "view.json"
    bundle_path = WORKDIR / "bundle.json"
    if bundle_path.exists():
        return view_path, bundle_path

    view_path.write_text(json.dumps(VIEW))
    entries = [
        {
            "resource": {
                "resourceType": "Patient",
                "id": f"p{i}",
                "gender": "male" if i % 2 == 0 else "female",
                "birthDate": "1980-01-01",
                "active": i % 3 != 0,
                "name": [{"family": f"Family{i}", "given": [f"Given{i}"]}],
                "address": [{"city": "Springfield"}],
            }
        }
        for i in range(N_PATIENTS)
    ]
    bundle_path.write_text(
        json.dumps({"resourceType": "Bundle", "type": "collection", "entry": entries})
    )
    return view_path, bundle_path


def render(view: Path, bundle: Path, fmt: str, out: Path) -> None:
    if out.exists():
        return
    with out.open("wb") as f:
        subprocess.run(
            [
                r"C:\Users\Doug\Code\target\release\sof-cli.exe",
                "--view", str(view), "--bundle", str(bundle), "--format", fmt,
            ],
            stdout=f,
            check=True,
        )


def timed(fn, repeats=REPEATS):
    times = []
    for _ in range(repeats):
        t0 = time.perf_counter()
        result = fn()
        times.append(time.perf_counter() - t0)
    return result, min(times), statistics.median(times)


def main() -> None:
    import pandas as pd
    import pyarrow as pa

    view, bundle = generate_inputs()
    csv_path = WORKDIR / "out.csv"
    arrow_path = WORKDIR / "out.arrow"
    render(view, bundle, "csv", csv_path)
    render(view, bundle, "arrow", arrow_path)

    csv_bytes = csv_path.read_bytes()
    arrow_bytes = arrow_path.read_bytes()
    print(f"rows={N_PATIENTS}  csv={len(csv_bytes)/1e6:.1f}MB  arrow={len(arrow_bytes)/1e6:.1f}MB")

    import io

    df_csv, csv_best, csv_med = timed(lambda: pd.read_csv(io.BytesIO(csv_bytes)))
    tbl, arrow_best, arrow_med = timed(
        lambda: pa.ipc.open_stream(io.BytesIO(arrow_bytes)).read_all()
    )
    df_arrow, topd_best, topd_med = timed(lambda: tbl.to_pandas())

    assert len(df_csv) == N_PATIENTS and tbl.num_rows == N_PATIENTS
    print(f"pandas.read_csv:            best {csv_best*1000:7.1f} ms   median {csv_med*1000:7.1f} ms")
    print(f"pa.ipc.open_stream+read_all best {arrow_best*1000:7.1f} ms   median {arrow_med*1000:7.1f} ms")
    print(f"  ...to_pandas() on top:    best {topd_best*1000:7.1f} ms   median {topd_med*1000:7.1f} ms")
    speed = csv_best / arrow_best
    speed_pd = csv_best / (arrow_best + topd_best)
    print(f"arrow vs csv to a table:    {speed:.1f}x faster ({speed_pd:.1f}x if ending in pandas)")


if __name__ == "__main__":
    main()
