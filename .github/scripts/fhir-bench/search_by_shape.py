#!/usr/bin/env python3
#
# Per-query-shape search latency attribution — replaces a `jq -rs` program
# that slurped the whole k6 `--out json` point stream into memory in one go.
#
# Called from: the `benchmark` job's "Attribute search latency per query
# shape" step (fhir-benchmark.yml), via:
#   python3 "$GITHUB_WORKSPACE/.github/scripts/fhir-bench/search_by_shape.py" \
#     "$POINTS" > "$OUT"
#
# Why this streams instead of slurping (`jq -rs` reads the entire NDJSON
# stream into one in-memory array before doing anything else): the
# self-hosted runner has been lost during this exact step three times —
# runs 30550776427, 33515369645 and 36423973848. The last of those was
# backend=sqlite, tests=prewarm,search: searching a near-empty database is
# very fast, so k6 emits an enormous number of points in a short wall-clock
# window and the points file balloons — that run's artifact was 317 MB
# compressed, and run 36412022609's 854 MB points file only just made it
# through before `jq -rs` started failing this way on bigger ones. This
# script reads the file one line at a time and keeps only the (few) floats
# it needs per query shape, so memory stays proportional to the number of
# distinct shapes (~21), not to the number of requests in the run.
#
# Input: sys.argv[1] — a k6 `--out json` NDJSON file, one JSON object per
# line, every metric of every request (Point/http_req_duration is one of
# many). A line that isn't valid JSON (a runner killed mid-write, e.g.) is
# skipped rather than aborting the whole file, unlike `jq -s`, which fails
# the entire slurp on the first parse error.
# Output: stdout — a Markdown table, one row per query shape
# (`<searchType> <resourceType>?<name>`), sorted by p95 descending, with
# n/median/p95/max in milliseconds. With no matching points, only the two
# header lines are printed (matching what the old jq program produced for
# an empty group list).
#
# Reproduces the jq program's semantics exactly, including two quirks that
# would silently diverge under naive Python equivalents:
#   - jq's `//` treats null AND false as "missing" (not just null) when
#     falling back to "?" for an absent tag; an empty string is a value and
#     is NOT replaced. tag_or() below implements that, not `dict.get(k, "?")`.
#   - jq's `round` rounds half away from zero (round(2.5) == 3,
#     round(-2.5) == -3); Python's built-in round() rounds half to even
#     (round(2.5) == 2), which would silently misreport rounded milliseconds
#     ending in .5. jq_round() below reimplements the C `round()` behaviour.
# median/p95/max use the same indices as the jq program: med = sorted[n//2],
# p95 = sorted[min(floor(n*0.95), n-1)], max = sorted[n-1]. Ties on p95 sort
# by ascending shape name, matching jq's `group_by(.shape)` (ascending)
# followed by a stable `sort_by(-.p95)`.
import json
import math
import sys
from array import array


def tag_or(tags, key):
    """jq's `(.data.tags.KEY // "?")`: null and false both count as
    missing; an empty string does not."""
    val = tags.get(key)
    if val is None or val is False:
        return "?"
    return val if isinstance(val, str) else str(val)


def jq_round(x):
    """jq's `round`: round half away from zero, unlike Python's
    round-half-to-even."""
    if x >= 0:
        return math.floor(x + 0.5)
    return math.ceil(x - 0.5)


def main():
    if len(sys.argv) != 2:
        print("usage: search_by_shape.py <points.json>", file=sys.stderr)
        return 2

    path = sys.argv[1]
    by_shape = {}

    try:
        with open(path, "r", encoding="utf-8", errors="replace") as fh:
            for line in fh:
                # Cheap pre-filter before paying for json.loads: every
                # http_req_duration Point line contains both substrings.
                if '"Point"' not in line or '"http_req_duration"' not in line:
                    continue
                try:
                    obj = json.loads(line)
                except ValueError:
                    continue
                if not isinstance(obj, dict):
                    continue
                if obj.get("type") != "Point" or obj.get("metric") != "http_req_duration":
                    continue
                data = obj.get("data") or {}
                value = data.get("value")
                if not isinstance(value, (int, float)) or isinstance(value, bool):
                    continue
                tags = data.get("tags") or {}
                shape = (
                    tag_or(tags, "searchType") + " " +
                    tag_or(tags, "resourceType") + "?" +
                    tag_or(tags, "name")
                )
                values = by_shape.get(shape)
                if values is None:
                    values = array("d")
                    by_shape[shape] = values
                values.append(float(value))
    except OSError as exc:
        print(f"search_by_shape.py: {exc}", file=sys.stderr)
        return 1

    print("| Query shape | n | median (ms) | p95 (ms) | max (ms) |")
    print("|---|---:|---:|---:|---:|")

    # Ascending shape order first (mirrors jq's group_by(.shape)); the
    # later sort by -p95 is stable, so ties keep this ascending order,
    # matching jq's group_by + stable sort_by(-.p95).
    rows = []
    for shape in sorted(by_shape.keys()):
        s = sorted(by_shape[shape])
        n = len(s)
        med = s[n // 2]
        p95 = s[min(math.floor(n * 0.95), n - 1)]
        mx = s[n - 1]
        rows.append((shape, n, med, p95, mx))

    rows.sort(key=lambda r: -r[3])

    for shape, n, med, p95, mx in rows:
        print(f"| `{shape}` | {n} | {jq_round(med)} | {jq_round(p95)} | {jq_round(mx)} |")

    return 0


if __name__ == "__main__":
    sys.exit(main())
