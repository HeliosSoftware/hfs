#!/usr/bin/env python3
"""Markdown report for the MongoDB search-protection ABAB runs (#2293).

Usage: abab_report.py <abab-dir> [--rounds N] [--baseline off]

<abab-dir> holds one folder per round and variant, named r<N>-<variant>
(for example r1-off, r1-timeout+gate), each with:

  crud.json, insert.json, search.json, read-under-search.json
      k6 --summary-export files (any of them may be missing)
  status-counts.txt
      one line per suite and HTTP status code:
      suite=<suite> status=<code> count=<n>

The baseline variant (A, default "off") is compared against every other
variant (B). For each metric the report gives median, min and max over the
rounds, the percentage change of B against A, and the ABAB verdict from
#2180: BETTER only when every B round beats every A round. A run with fewer
rounds than expected, or a round missing one of the two variants, is marked
incomplete.
"""

import argparse
import json
import os
import re
import statistics
import sys

SUITES = ("crud", "insert", "search", "read-under-search")
FAIL_CODES = ("408", "503", "504")
ROUND_RE = re.compile(r"^r(\d+)-(.+)$")
STATUS_RE = re.compile(r"^suite=(\S+)\s+status=(\d+)\s+count=(\d+)\s*$")

# (label, direction): "up" means a higher value is better, "down" lower.
HIGHER, LOWER = "up", "down"


def load_json(path):
    try:
        with open(path) as f:
            return json.load(f)
    except (OSError, ValueError):
        return None


def metric(summary, name, key):
    """Return summary.metrics[name][key] as a float, or None."""
    try:
        v = summary["metrics"][name][key]
    except (KeyError, TypeError):
        return None
    return float(v) if isinstance(v, (int, float)) else None


def suite_metrics(suite, summary):
    """Latency and throughput metrics of one suite summary: {label: (value, direction)}."""
    out = {}
    if summary is None:
        return out

    def put(label, value, direction):
        if value is not None:
            out[label] = (value, direction)

    if suite == "read-under-search":
        for scen in ("read", "broad"):
            dur = "http_req_duration{scenario:%s}" % scen
            put("%s p50 (ms)" % scen, metric(summary, dur, "med"), LOWER)
            put("%s p95 (ms)" % scen, metric(summary, dur, "p(95)"), LOWER)
            put(
                "%s throughput (req/s)" % scen,
                metric(summary, "http_reqs{scenario:%s}" % scen, "rate"),
                HIGHER,
            )
    else:
        put("p50 (ms)", metric(summary, "http_req_duration", "med"), LOWER)
        put("p95 (ms)", metric(summary, "http_req_duration", "p(95)"), LOWER)
        put("throughput (req/s)", metric(summary, "http_reqs", "rate"), HIGHER)
    failed = metric(summary, "http_req_failed", "value")
    put("http_req_failed (%)", None if failed is None else failed * 100.0, LOWER)
    return out


def load_status_counts(path):
    """{suite: {code: count}} from status-counts.txt; missing file -> {}."""
    counts = {}
    try:
        with open(path) as f:
            for line in f:
                m = STATUS_RE.match(line)
                if m:
                    suite, code, n = m.group(1), m.group(2), int(m.group(3))
                    d = counts.setdefault(suite, {})
                    d[code] = d.get(code, 0) + n
    except OSError:
        pass
    return counts


def failure_rates(counts_for_suite):
    """{code: percent} for 408/503/504; None when the suite saw no responses."""
    total = sum(counts_for_suite.values()) if counts_for_suite else 0
    if total <= 0:
        return {c: None for c in FAIL_CODES}
    return {c: 100.0 * counts_for_suite.get(c, 0) / total for c in FAIL_CODES}


def load_runs(root):
    """{variant: {round: {suite: {label: (value, dir)}}, status...}}.

    Returns (runs, rounds) where runs[variant][round] = (metrics, status)
    with metrics = {suite: {label: (value, dir)}} and
    status = {suite: {code: count}}.
    """
    runs = {}
    try:
        names = sorted(os.listdir(root))
    except OSError:
        return runs
    for name in names:
        m = ROUND_RE.match(name)
        d = os.path.join(root, name)
        if not m or not os.path.isdir(d):
            continue
        rnd, variant = int(m.group(1)), m.group(2)
        metrics = {}
        for suite in SUITES:
            sm = suite_metrics(suite, load_json(os.path.join(d, suite + ".json")))
            if sm:
                metrics[suite] = sm
        status = load_status_counts(os.path.join(d, "status-counts.txt"))
        # Failure rates become metrics too, so they get the same ABAB treatment.
        for suite, counts in status.items():
            rates = failure_rates(counts)
            for code, rate in rates.items():
                if rate is not None:
                    metrics.setdefault(suite, {})["%s rate (%%)" % code] = (rate, LOWER)
        runs.setdefault(variant, {})[rnd] = (metrics, status)
    return runs


def series(runs_of_variant, suite, label, rounds):
    """Values of one metric over the given rounds (rounds without it are skipped)."""
    vals = []
    for r in rounds:
        entry = runs_of_variant.get(r)
        if entry is None:
            continue
        got = entry[0].get(suite, {}).get(label)
        if got is not None:
            vals.append(got[0])
    return vals


def direction_of(runs_of_variant, suite, label):
    for metrics, _ in runs_of_variant.values():
        got = metrics.get(suite, {}).get(label)
        if got is not None:
            return got[1]
    return LOWER


def pct_change(a, b):
    if a == 0:
        return None
    return 100.0 * (b - a) / abs(a)


def verdict(a_vals, b_vals, direction):
    """ABAB verdict: BETTER if every B beats every A, WORSE if every B loses
    to every A, SAME if all values are equal, else MIXED."""
    if not a_vals or not b_vals:
        return "n/a"
    if set(a_vals) == set(b_vals) and len(set(a_vals)) == 1:
        return "SAME"
    if direction == HIGHER:
        if min(b_vals) > max(a_vals):
            return "BETTER"
        if max(b_vals) < min(a_vals):
            return "WORSE"
    else:
        if max(b_vals) < min(a_vals):
            return "BETTER"
        if min(b_vals) > max(a_vals):
            return "WORSE"
    return "MIXED"


def fmt(v):
    return "n/a" if v is None else "%.2f" % v


def fmt_stats(vals):
    if not vals:
        return "n/a"
    return "%s (%s–%s)" % (
        fmt(statistics.median(vals)),
        fmt(min(vals)),
        fmt(max(vals)),
    )


def fmt_pct(p):
    return "n/a" if p is None else "%+.1f%%" % p


def completeness(runs, baseline, other, expected):
    """Return (rounds_used, problems). Rounds used have both A and B."""
    a_rounds = set(runs.get(baseline, {}))
    b_rounds = set(runs.get(other, {}))
    both = sorted(a_rounds & b_rounds)
    problems = []
    missing = sorted((a_rounds | b_rounds) - set(both))
    if missing:
        problems.append(
            "rounds %s have only one of %s / %s" % (missing, baseline, other)
        )
    if expected and len(both) < expected:
        problems.append("%d of %d expected rounds complete" % (len(both), expected))
    return both, problems


def render(root, expected=None, baseline="off"):
    runs = load_runs(root)
    lines = ["## MongoDB search protections: ABAB (#2293)", ""]
    if baseline not in runs:
        lines.append("No `%s` rounds found in `%s`; nothing to compare." % (baseline, root))
        return "\n".join(lines) + "\n"
    others = sorted(v for v in runs if v != baseline)
    if not others:
        lines.append("Only the `%s` variant was found; nothing to compare." % baseline)
        return "\n".join(lines) + "\n"

    for other in others:
        rounds, problems = completeness(runs, baseline, other, expected)
        lines.append("### A=`%s` vs B=`%s`" % (baseline, other))
        lines.append("")
        if problems:
            lines.append("**ABAB incomplete:** " + "; ".join(problems) + ".")
        else:
            lines.append("ABAB complete: %d paired rounds." % len(rounds))
        lines.append("")
        lines.append(
            "Cells are median (min–max) over the rounds. Change is B against A; "
            "the verdict is BETTER only when every B round beats every A round."
        )
        lines.append("")
        lines.append("| suite | metric | A median (min–max) | B median (min–max) | change | verdict |")
        lines.append("|---|---|---|---|---|---|")
        a_runs, b_runs = runs[baseline], runs[other]
        for suite in SUITES:
            labels = []
            for rr in (a_runs, b_runs):
                for metrics, _ in rr.values():
                    for lab in metrics.get(suite, {}):
                        if lab not in labels:
                            labels.append(lab)
            for label in labels:
                a_vals = series(a_runs, suite, label, rounds)
                b_vals = series(b_runs, suite, label, rounds)
                direction = direction_of(a_runs, suite, label) if a_vals else direction_of(b_runs, suite, label)
                change = (
                    pct_change(statistics.median(a_vals), statistics.median(b_vals))
                    if a_vals and b_vals
                    else None
                )
                lines.append(
                    "| %s | %s | %s | %s | %s | %s |"
                    % (
                        suite,
                        label,
                        fmt_stats(a_vals),
                        fmt_stats(b_vals),
                        fmt_pct(change),
                        verdict(a_vals, b_vals, direction),
                    )
                )
        lines.append("")
    lines.append(
        "Rates are the share of all responses in the suite, from `http_requests_total` "
        "deltas; `n/a` means the suite saw no responses. A 504 comes from the search "
        "deadline or mongod's `maxTimeMS`; /metrics does not tell them apart."
    )
    return "\n".join(lines) + "\n"


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("abab_dir")
    ap.add_argument("--rounds", type=int, default=None, help="expected rounds per variant")
    ap.add_argument("--baseline", default="off")
    args = ap.parse_args(argv)
    sys.stdout.write(render(args.abab_dir, args.rounds, args.baseline))
    return 0


if __name__ == "__main__":
    sys.exit(main())
