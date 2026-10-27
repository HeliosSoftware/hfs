#!/usr/bin/env python3
import argparse
import json
from datetime import datetime
from pathlib import Path
from typing import Any


def parse_recorded_ms(event: dict[str, Any]) -> int | None:
    raw = event.get("recorded")
    if not isinstance(raw, str) or not raw:
        return None
    try:
        if raw.endswith("Z"):
            raw = raw[:-1] + "+00:00"
        dt = datetime.fromisoformat(raw)
        return int(dt.timestamp() * 1000)
    except ValueError:
        return None


def load_ndjson(path: Path) -> list[dict[str, Any]]:
    events: list[dict[str, Any]] = []
    if not path.exists():
        return events
    for idx, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
        line = raw.strip()
        if not line:
            continue
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        event["__line"] = idx
        event["__recorded_ms"] = parse_recorded_ms(event)
        events.append(event)
    return events


def read_ranges(path: Path) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    if not path.exists():
        return rows
    lines = path.read_text(encoding="utf-8").splitlines()
    for line in lines[1:]:
        if not line.strip():
            continue
        parts = line.split("\t")
        if len(parts) < 7:
            continue
        row = {
            "name": parts[0],
            "start_line": int(parts[5]),
            "end_line": int(parts[6]),
            "start_ts_ms": None,
            "end_ts_ms": None,
        }
        if len(parts) >= 9:
            try:
                row["start_ts_ms"] = int(parts[7])
                row["end_ts_ms"] = int(parts[8])
            except ValueError:
                row["start_ts_ms"] = None
                row["end_ts_ms"] = None
        rows.append(row)
    return rows


def write_ndjson(path: Path, events: list[dict[str, Any]]) -> None:
    lines: list[str] = []
    for event in events:
        clean = {k: v for k, v in event.items() if not k.startswith("__")}
        lines.append(json.dumps(clean, separators=(",", ":")))
    content = "\n".join(lines)
    if content:
        content += "\n"
    path.write_text(content, encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Partition exported audit NDJSON into per-call windows using interaction timestamps."
    )
    parser.add_argument("--audit-file", required=True)
    parser.add_argument("--ranges-file", required=True)
    parser.add_argument("--output-dir", required=True)
    parser.add_argument("--start-margin-ms", type=int, default=1500)
    parser.add_argument("--end-margin-ms", type=int, default=3000)
    args = parser.parse_args()

    audit_file = Path(args.audit_file)
    ranges_file = Path(args.ranges_file)
    output_dir = Path(args.output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)

    events = load_ndjson(audit_file)
    ranges = read_ranges(ranges_file)

    if not ranges:
        print("No interaction ranges found; no partition files created.")
        return 0

    total_written = 0
    for idx, row in enumerate(ranges):
        name = row["name"]
        out_path = output_dir / f"{name}.ndjson"

        start_ts = row.get("start_ts_ms")
        end_ts = row.get("end_ts_ms")

        if start_ts is None or end_ts is None:
            # Fallback to line-based slicing when timestamps are unavailable.
            selected = [
                e
                for e in events
                if row["start_line"] < int(e.get("__line", 0)) <= row["end_line"]
            ]
            write_ndjson(out_path, selected)
            total_written += len(selected)
            continue

        next_start = None
        for later in ranges[idx + 1 :]:
            if later.get("start_ts_ms") is not None:
                next_start = int(later["start_ts_ms"])
                break

        window_start = int(start_ts) - int(args.start_margin_ms)
        if next_start is not None:
            window_end = next_start - 1
        else:
            window_end = int(end_ts) + int(args.end_margin_ms)

        selected = []
        for event in events:
            recorded_ms = event.get("__recorded_ms")
            if recorded_ms is None:
                continue
            if window_start <= int(recorded_ms) <= window_end:
                selected.append(event)

        write_ndjson(out_path, selected)
        total_written += len(selected)

    print(
        f"Partitioned {len(events)} events across {len(ranges)} interaction windows; wrote {total_written} records."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
