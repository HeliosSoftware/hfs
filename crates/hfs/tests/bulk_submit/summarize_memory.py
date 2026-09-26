#!/usr/bin/env python3
"""Render retained #995 series as Markdown, including unsuccessful attempts."""

import argparse
import csv
import json
from pathlib import Path
from statistics import mean


def number(value):
    return "—" if value is None else f"{value:.1f}"


def summarize(directory):
    run = json.loads((directory / "run.json").read_text())
    with (directory / "rss.csv").open() as source:
        samples = list(csv.DictReader(source))
    with (directory / "host.csv").open() as source:
        host = list(csv.DictReader(source))
    config = run["config"]
    print(f"### {directory.name}\n")
    print(f"Result: **{run['status']}**. Mode: `{config['mode']}`; "
          f"deferred indexing: `{config['defer_indexing']}`; "
          f"resources per submission: {config['resources']:,}.\n")
    print("| Job | Status | Pre-kickoff MiB | Sampled peak MiB | Post-idle MiB | "
          "Last 10 s mean MiB | Validation peak MiB | Submit s | Reindex verified s |")
    print("| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |")
    for attempt in run.get("attempts", []):
        job = attempt["ordinal"]
        phases = {p["phase"]: p for p in run.get("phases", []) if p["job"] == job}
        idle = phases.get("idle_end", {})
        end = idle.get("mono_s")
        last = [float(row["rss_mib"]) for row in samples
                if int(row["job"]) == job and end is not None
                and end - 10 <= float(row["mono_s"]) <= end]
        rss = attempt.get("rss", {})
        measured = rss.get("measured_window", {})
        times = attempt.get("timings", {})
        values = [job, attempt["status"],
                  number(phases.get("prekickoff_baseline", {}).get("rss_mib")),
                  number(measured.get("peak_mib")), number(idle.get("rss_mib")),
                  number(mean(last) if last else None),
                  number(rss.get("validation_window", {}).get("peak_mib")),
                  number(times.get("kickoff_to_terminal_s")),
                  number(times.get("kickoff_to_reindex_s"))]
        print("| " + " | ".join(map(str, values)) + " |")
    intervals = [float(b["mono_s"]) - float(a["mono_s"])
                 for a, b in zip(samples, samples[1:])
                 if a["pid"] == b["pid"] and a["job"] == b["job"]]
    swap = [float(row["swapout_bytes_since_start"]) / 1024**2
            for row in host if row.get("swapout_bytes_since_start")]
    pressure = sorted({row["pressure_level"] for row in host if row.get("pressure_level")})
    failed = [c["name"] for c in run.get("checks", []) if not c["ok"]]
    print(f"\nRSS samples: {len(samples)}; maximum within-job sampling gap: "
          f"{number(max(intervals) if intervals else None)} s. "
          f"Maximum new host swapouts: {number(max(swap) if swap else None)} MiB. "
          f"Host pressure levels: {', '.join(pressure) or 'unavailable'}.\n")
    print(f"Failed checks: {', '.join(failed) or 'none'}. "
          f"Stop records: {len(run.get('stops', []))}.\n")
    if run.get("reason"):
        print(f"Reason: {run['reason']}\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("series", nargs="+", type=Path)
    args = parser.parse_args()
    print("# Current-code bulk-submit memory observations\n")
    print("RSS is resident memory, not a live-allocation measurement. "
          "Peaks below are sampled lower bounds. Validation follows the idle "
          "window and can affect the next job's baseline. Job 1 creates the "
          "corpus; subsequent jobs reimport it and grow history. "
          "Times are external observations, not internal stage durations.\n")
    for directory in args.series:
        summarize(directory)


if __name__ == "__main__":
    main()
