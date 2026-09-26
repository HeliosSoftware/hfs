#!/usr/bin/env python3
"""Summarize #995 inventories and successful malloc snapshots, preserving units."""
import argparse
import json
import re
from pathlib import Path


def report(path, released_channels=False):
    run = json.loads((path / "run.json").read_text())
    config = run["config"]
    inventory_path = path / "retention-inventory.jsonl"
    inventories = [json.loads(line) for line in inventory_path.read_text().splitlines()] if inventory_path.exists() else []
    samples_path = path / "retention/snapshots.jsonl"
    samples = [json.loads(line) for line in samples_path.read_text().splitlines()] if samples_path.exists() else []
    verified = sum(attempt["status"] == "verified" for attempt in run["attempts"])
    print(f"\n## {path.name}\n")
    print(f"Run: **{run['status']}**; mode `{config['mode']}`; {config['resources']:,} resources/job; "
          f"{verified}/{config['jobs']} verified; idle {config['idle_seconds']:g} seconds.\n")
    print("| Job | PID | Jobs / finished | Channels / closed | Map capacities jobs / channels | Occupied entry bytes | Owned buffer capacity bytes |")
    print("| --- | ---: | ---: | ---: | ---: | ---: | ---: |")
    for row in inventories:
        f = row["fields"]
        print(f"| {row['job']} | {row['pid']} | {f['jobs_len']} / {f['finished_jobs']} | "
              f"{f['channels_len']} / {f['closed_channels']} | {f['jobs_capacity']} / {f['channels_capacity']} | "
              f"{f['occupied_entry_bytes']:,} | {f['owned_buffer_capacity_bytes']:,} |")
    complete = run["status"] == "ok" and verified == config["jobs"] and len(inventories) == config["jobs"]
    expected = lambda row: row["job"] if config["mode"] == "consecutive" else 1
    signature = bool(inventories) and all(
        row["fields"]["jobs_len"] == min(expected(row), 1024) if released_channels else row["fields"]["jobs_len"] == expected(row)
        for row in inventories
    ) and all(
        row["fields"]["finished_jobs"] == row["fields"]["jobs_len"]
        and row["fields"]["channels_len"] == (0 if released_channels else expected(row))
        and row["fields"]["closed_channels"] == (0 if released_channels else expected(row))
        for row in inventories
    )
    expectation = "retained polling status with no cancellation channels" if released_channels else (
        "one retained entry per job" if config["mode"] == "consecutive" else "one entry after each restart")
    print(f"\nComplete validated inventory: **{complete}**. "
          f"Observed counts match {expectation}: **{signature}**.\n")
    print("| Snapshot | Malloc blocks | Malloc bytes | Physical footprint reported by heap |")
    print("| --- | ---: | ---: | ---: |")
    failures = []
    heap_rows = []
    for sample in samples:
        if sample.get("returncode") != 0 or sample.get("timeout"):
            failures.append(sample)
            continue
        if Path(sample["command"][0]).name != "heap":
            continue
        raw = (path / "retention" / Path(sample["raw"]).name).read_text()
        sizes = re.search(r"All zones: (\d+) nodes \((\d+) bytes\)", raw)
        footprint = re.search(r"Physical footprint:\s*(\S+)", raw)
        if not sizes or not footprint:
            failures.append({"parse_failure": sample["raw"]})
            continue
        nodes, allocated = map(int, sizes.groups())
        heap_rows.append((sample["job"], sample["label"], allocated, nodes))
        print(f"| Job {sample['job']} {sample['label']} | {nodes:,} | {allocated:,} | {footprint[1]} |")
    postidle = sorted(row for row in heap_rows if row[1] == "postidle")
    if len(postidle) >= 2:
        print(f"\nPost-idle malloc delta job {postidle[0][0]} → {postidle[-1][0]}: "
              f"**{postidle[-1][2] - postidle[0][2]:+,} bytes**, "
              f"**{postidle[-1][3] - postidle[0][3]:+,} blocks**.")
    print(f"\nTool calls: {len(samples)}; failed calls/parses: {len(failures)}.")
    if failures:
        print("Failures: " + json.dumps(failures))


if __name__ == "__main__":
    print("# #995 repeated-job retention observations\n")
    print("Inventories are sampled after verified reindex and idle, before heap inspection and validation. "
          "Byte fields account for occupied inline entries and owned buffer capacities only; they exclude "
          "HashMap bucket allocations, channel internals and allocator overhead. Malloc snapshots do not "
          "cover every process allocation. RSS/footprint are separate metrics. This summarizer does not modify captured state.")
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--expect-released-channels", action="store_true",
                        help="expect the fixed lifecycle: status retained, channels removed")
    parser.add_argument("directories", nargs="+", type=Path)
    args = parser.parse_args()
    for directory in args.directories:
        report(directory, args.expect_released_channels)
