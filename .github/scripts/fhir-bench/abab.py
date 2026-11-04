#!/usr/bin/env python3
"""Strict ABAB verdict for the sync/async rounds of one Elasticsearch leg (#2292).

Reads the round directories a leg writes in ``es_sync_mode=abab``::

    <leg>/r<N>-sync/<suite>.json            k6 summary
    <leg>/r<N>-sync/<suite>-queue.txt       key=value queue-lag stats
    <leg>/r<N>-sync/es-drain-<suite>.txt    key=value drain-gate result
    <leg>/r<N>-async/...                    same files

Arm A is the synchronous mode, arm B the asynchronous one. A metric gets a
verdict only under the strict rule of #2180: EVERY B round beats EVERY A round
(worst B better than best A) is MEJORA, the mirror is EMPEORA, anything else
-- overlap, a tie, or fewer than ``--min-pairs`` valid rounds per arm -- is
"sin diferencia demostrable". Unlike obs-ab/report.py, no dissenting round is
tolerated.

A suite round whose drain gate did not end ``status=drained`` is invalid and
is excluded (never averaged); its number is listed under ``invalid``.

Writes ``abab.json`` and ``abab.txt`` into the leg directory (or --out-dir).
Always exits 0 so it can never fail a benchmark leg.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import statistics as stats
import sys

SUITES = ("crud", "insert", "search")
MIN_PAIRS = 2

MEJORA = "MEJORA"
EMPEORA = "EMPEORA"
NO_DIFF = "sin diferencia demostrable"

# (metric key, source, field, better)
#   source "k6": k6 summary; "queue": <suite>-queue.txt; "drain": es-drain-<suite>.txt
METRICS = (
    ("rps", "k6", "rps", "higher"),
    ("p50", "k6", "p50", "lower"),
    ("p95", "k6", "p95", "lower"),
    ("drain_s", "drain", "gate_seconds", "lower"),
    ("depth_p95", "queue", "depth_p95", "lower"),
    ("depth_area", "queue", "depth_area", "lower"),
)

_ROUND_RE = re.compile(r"^r(\d+)-(sync|async)$")


def _num(v):
    try:
        f = float(v)
    except (TypeError, ValueError):
        return None
    return f if f == f else None


def read_kv(path):
    out = {}
    try:
        with open(path) as fh:
            for ln in fh:
                if "=" in ln:
                    k, v = ln.split("=", 1)
                    out[k.strip()] = v.strip()
    except OSError:
        return None
    return out


def read_k6(path):
    try:
        with open(path) as fh:
            m = json.load(fh).get("metrics", {})
    except (OSError, ValueError):
        return None
    dur = m.get("http_req_duration", {})
    return {
        "p50": _num(dur.get("med")),
        "p95": _num(dur.get("p(95)")),
        "rps": _num(m.get("http_reqs", {}).get("rate")),
    }


def find_rounds(leg_dir):
    """{round_number: {'sync': dir, 'async': dir}}"""
    rounds = {}
    for name in sorted(os.listdir(leg_dir)):
        m = _ROUND_RE.match(name)
        p = os.path.join(leg_dir, name)
        if m and os.path.isdir(p):
            rounds.setdefault(int(m.group(1)), {})[m.group(2)] = p
    return rounds


def round_values(rdir, suite):
    """(values, valid) for one suite in one round dir; values keyed by metric."""
    drain = read_kv(os.path.join(rdir, f"es-drain-{suite}.txt"))
    valid = drain is None or drain.get("status") == "drained"
    k6 = read_k6(os.path.join(rdir, f"{suite}.json")) or {}
    queue = read_kv(os.path.join(rdir, f"{suite}-queue.txt")) or {}
    srcs = {"k6": k6, "queue": queue, "drain": drain or {}}
    vals = {key: _num(srcs[src].get(field)) for key, src, field, _ in METRICS}
    return vals, valid


def verdict(a, b, better, min_pairs=MIN_PAIRS):
    if len(a) < min_pairs or len(b) < min_pairs:
        return NO_DIFF
    if better == "higher":
        if min(b) > max(a):
            return MEJORA
        if max(b) < min(a):
            return EMPEORA
    else:
        if max(b) < min(a):
            return MEJORA
        if min(b) > max(a):
            return EMPEORA
    return NO_DIFF


def _spread(v):
    return [min(v), max(v)] if v else None


def analyze(leg_dir, min_pairs=MIN_PAIRS):
    rounds = find_rounds(leg_dir)
    metrics = []
    invalid = []
    for suite in SUITES:
        vals = {"sync": {k: [] for k, *_ in METRICS}, "async": {k: [] for k, *_ in METRICS}}
        seen = False
        for n in sorted(rounds):
            for mode, rdir in rounds[n].items():
                if not any(os.path.exists(os.path.join(rdir, f)) for f in
                           (f"{suite}.json", f"es-drain-{suite}.txt", f"{suite}-queue.txt")):
                    continue
                seen = True
                v, ok = round_values(rdir, suite)
                if not ok:
                    invalid.append({"suite": suite, "round": n, "mode": mode})
                    continue
                for k, x in v.items():
                    if x is not None:
                        vals[mode][k].append(x)
        if not seen:
            continue
        for key, _, _, better in METRICS:
            a, b = vals["sync"][key], vals["async"][key]
            if not a and not b:
                continue
            ma = stats.median(a) if a else None
            mb = stats.median(b) if b else None
            delta = (mb - ma) / ma * 100 if ma and mb is not None else None
            metrics.append({
                "metric": f"{suite}.{key}", "suite": suite, "key": key,
                "better": better, "a": a, "b": b,
                "median_a": ma, "median_b": mb,
                "spread_a": _spread(a), "spread_b": _spread(b),
                "delta_pct": delta,
                "verdict": verdict(a, b, better, min_pairs),
            })
    return {"arms": {"A": "synchronous", "B": "asynchronous"},
            "rounds": sorted(rounds), "min_pairs": min_pairs,
            "invalid": invalid, "metrics": metrics}


def _f(x):
    return "-" if x is None else f"{x:.4g}"


def render_txt(res):
    lines = ["ABAB sync (A) vs async (B); rounds: "
             + (",".join(map(str, res["rounds"])) or "none"), ""]
    for m in res["metrics"]:
        sa, sb = m["spread_a"] or [None, None], m["spread_b"] or [None, None]
        d = "-" if m["delta_pct"] is None else f"{m['delta_pct']:+.1f}%"
        lines.append(f"{m['metric']:<18} {m['verdict']:<28} A {_f(m['median_a'])} "
                     f"[{_f(sa[0])}..{_f(sa[1])}]  B {_f(m['median_b'])} "
                     f"[{_f(sb[0])}..{_f(sb[1])}]  delta {d}  ({m['better']} is better)")
    for i in res["invalid"]:
        lines.append(f"INVALID {i['suite']} r{i['round']}-{i['mode']} (drain not drained)")
    return "\n".join(lines) + "\n"


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("leg_dir")
    ap.add_argument("--out-dir", default=None)
    ap.add_argument("--min-pairs", type=int, default=MIN_PAIRS)
    args = ap.parse_args(argv)
    if not os.path.isdir(args.leg_dir):
        print(f"abab: not a directory: {args.leg_dir}", file=sys.stderr)
        return 0
    res = analyze(args.leg_dir, args.min_pairs)
    out = args.out_dir or args.leg_dir
    with open(os.path.join(out, "abab.json"), "w") as fh:
        json.dump(res, fh, indent=2)
    txt = render_txt(res)
    with open(os.path.join(out, "abab.txt"), "w") as fh:
        fh.write(txt)
    sys.stdout.write(txt)
    return 0


if __name__ == "__main__":
    sys.exit(main())
