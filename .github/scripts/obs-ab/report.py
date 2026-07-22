#!/usr/bin/env python3
"""Same-session observability A/B reporter (issue #297).

Consumes the per-(arm, round) k6 summaries written by run-arms.sh under
``results/${RUN_ID}__${arm-slug}__r${round}/hts/${TEST}_vu${VU}.json`` and
produces:

  * a medians-only Markdown table on stdout (destined for
    ``$GITHUB_STEP_SUMMARY``; kept small to stay under the 1 MiB limit), and
  * a detailed CSV artifact (``--csv-out``) carrying spread, sign-consistency
    and the noise flag for every cell.

Method (why it is shaped this way):

  * Deltas are PAIRED WITHIN A ROUND — ``median_r( B[r] - A[r] )`` — not a
    difference of independent per-arm medians. Because the arms are interleaved
    round-robin, pairing within a round cancels the slow within-session drift
    that the two-dispatch #292 A/B could not, to first order.
  * Every cell carries spread (min/median/max) and a flag, so a bare point
    estimate is never presented as fact. The flag is deliberately conservative:
      - INCONCLUSIVE  <2 paired rounds, or too few requests to trust (the
                      low-rps EX family at 30 s lands here).
      - VOID          the two arms' error rates differ materially — a partly
                      failing arm's throughput is not comparable.
      - SIGNAL        sign-consistent across (nearly) all rounds AND the median
                      delta clears twice the within-arm noise.
      - NOISE         sign flips across rounds, or the delta is within noise.
      - WEAK          directionally consistent but under the SIGNAL bar.
  * Only ``off→default`` is a single-component isolate (the cost of metrics; in
    the benchmark span and reqlog are inert under Default). Later adjacencies
    bundle components and are labelled as such — the "per-component" reading
    stops after the first hop, by construction of the current arm switch.

The numbers are from a DEBUG build: use them to rank components and confirm
direction, never as release magnitudes.
"""

from __future__ import annotations

import argparse
import csv
import glob
import json
import os
import statistics as stats
from typing import Optional

# A cell with fewer than this many observed requests (min across the paired
# rounds/arms) is treated as too small to difference — the low-rps EX tests at
# 30 s produce ~tens–hundreds and land here rather than being quoted.
LOW_SAMPLE = 300
# Error-rate gap (fraction) beyond which a paired cell is voided as not
# comparable.
ERR_VOID = 0.01

# What each adjacency attributes, given the arm switch semantics. Keyed by
# (from_arm, to_arm). Anything not listed is labelled "bundled".
ADJACENCY_MEANING = {
    ("off", "default"): "metrics (clean isolate)",
    ("default", "no-span"): "reqlog (span still off in bench)",
    ("no-span", "full"): "span",
    ("default", "full"): "span + reqlog (bundled)",
    ("full", "full+probe"): "probe stdout logs (RUST_LOG)",
    ("default", "full+probe"): "span + reqlog + probe logs (bundled)",
}


def slug(arm: str) -> str:
    return arm.replace("+", "-")


def load_cell(path: str) -> Optional[dict]:
    try:
        with open(path) as fh:
            d = json.load(fh)
    except (OSError, ValueError):
        return None
    m = d.get("metrics", {})
    dur = m.get("http_req_duration", {})
    reqs = m.get("http_reqs", {})
    failed = m.get("http_req_failed", {})
    return {
        "p50": dur.get("med"),
        "rps": reqs.get("rate"),
        "count": reqs.get("count"),
        "err": failed.get("rate", 0.0) or 0.0,
    }


def collect(results_dir: str, run_id: str, arms: list[str], rounds: int):
    """-> data[(test, vu)][arm][round] = cell, plus sorted (test, vu) keys."""
    data: dict = {}
    for arm in arms:
        for rnd in range(1, rounds + 1):
            base = os.path.join(results_dir, f"{run_id}__{slug(arm)}__r{rnd}", "hts")
            for path in glob.glob(os.path.join(base, "*_vu*.json")):
                name = os.path.basename(path)[: -len(".json")]
                if "_vu" not in name:
                    continue
                test, vu = name.rsplit("_vu", 1)
                try:
                    vu = int(vu)
                except ValueError:
                    continue
                cell = load_cell(path)
                if cell is None or cell["p50"] is None:
                    continue
                data.setdefault((test, vu), {}).setdefault(arm, {})[rnd] = cell
    return data


def cv_pct(values: list[float]) -> Optional[float]:
    vals = [v for v in values if v is not None]
    if len(vals) < 2:
        return None
    mean = stats.mean(vals)
    if mean <= 0:
        return None
    return stats.pstdev(vals) / mean * 100.0


def summarize_arm(rounds_map: dict, key: str):
    vals = [c[key] for c in rounds_map.values() if c.get(key) is not None]
    if not vals:
        return None
    return {
        "median": stats.median(vals),
        "min": min(vals),
        "max": max(vals),
        "n": len(vals),
    }


def paired_delta(a: dict, b: dict, key: str):
    """Per-round paired deltas of `key` for rounds present in BOTH arms."""
    common = sorted(set(a) & set(b))
    abs_d, pct_d = [], []
    for r in common:
        av, bv = a[r].get(key), b[r].get(key)
        if av is None or bv is None:
            continue
        abs_d.append(bv - av)
        if av != 0:
            pct_d.append((bv - av) / av * 100.0)
    return common, abs_d, pct_d


def sign_string(deltas: list[float], eps: float = 1e-9):
    signs = [1 if d > eps else (-1 if d < -eps else 0) for d in deltas]
    n = len(signs)
    pos = signs.count(1)
    neg = signs.count(-1)
    k = max(pos, neg)
    dom = "+" if pos >= neg else "-"
    consistent = n >= 2 and k >= n - 1 and pos != neg
    return f"{k}/{n} {dom}", consistent


def flag_for(pct_deltas, abs_deltas, cvp, min_count, err_gap, n_pair):
    if n_pair < 2 or min_count is None or min_count < LOW_SAMPLE:
        return "INCONCLUSIVE"
    if err_gap is not None and err_gap > ERR_VOID:
        return "VOID"
    if cvp is None:
        return "INCONCLUSIVE"
    median_pct = abs(stats.median(pct_deltas)) if pct_deltas else 0.0
    _, consistent = sign_string(abs_deltas)
    if consistent and median_pct > 2 * cvp:
        return "SIGNAL"
    if (not consistent) or median_pct <= cvp:
        return "NOISE"
    return "WEAK"


def fmt(x, nd=1):
    return "—" if x is None else f"{x:,.{nd}f}"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--results-dir", required=True)
    ap.add_argument("--run-id", required=True)
    ap.add_argument("--arms", required=True, help="space/comma list, ladder order")
    ap.add_argument("--rounds", type=int, required=True)
    ap.add_argument("--backend", default="")
    ap.add_argument("--commit", default="")
    ap.add_argument("--build", default="debug (opt-level 0)")
    ap.add_argument("--csv-out", default="")
    args = ap.parse_args()

    arms = [a for a in args.arms.replace(",", " ").split() if a]
    data = collect(args.results_dir, args.run_id, arms, args.rounds)

    out = []
    out.append(f"## HTS observability A/B — same session ({args.backend})")
    out.append("")
    out.append("| | |")
    out.append("|---|---|")
    out.append(f"| **Commit** | `{args.commit[:7]}` |")
    out.append(f"| **Backend** | `{args.backend}` |")
    out.append(f"| **Arms** | `{' → '.join(arms)}` |")
    out.append(f"| **Rounds** | {args.rounds} (interleaved; deltas paired within a round) |")
    out.append(f"| **Build** | **{args.build} — ratios/direction only, NOT release magnitude** |")
    out.append("")
    out.append(
        "> In the benchmark `default` = metrics-only (no OTLP exporter → span off; "
        "no reqlog consumer → reqlog off). Only `off→default` isolates a single "
        "component (metrics); later hops bundle span/reqlog/probe-logs. Primary "
        "metric below is **VU=1 p50** (purest per-request cost); RPS is the headline."
    )
    out.append("")

    if not data:
        out.append("_No results collected — every arm may have failed to start._")
        print("\n".join(out))
        return 0

    # Adjacency legend actually present in this ladder.
    out.append("**Adjacencies:** " + " · ".join(
        f"`{a}→{b}` = {ADJACENCY_MEANING.get((a, b), 'bundled')}"
        for a, b in zip(arms, arms[1:])
    ))
    out.append("")

    csv_rows = []
    vus = sorted({vu for (_t, vu) in data})
    for vu in vus:
        out.append(f"### VU = {vu}")
        out.append("")
        header = ["Test"] + [f"{a} p50" for a in arms]
        for a, b in zip(arms, arms[1:]):
            header.append(f"Δ {a}→{b}")
        out.append("| " + " | ".join(header) + " |")
        out.append("|" + "---|" * len(header))

        for (test, cvu) in sorted(k for k in data if k[1] == vu):
            per_arm = data[(test, cvu)]
            row = [test]
            arm_stats = {}
            for a in arms:
                s = summarize_arm(per_arm.get(a, {}), "p50")
                arm_stats[a] = s
                row.append(fmt(s["median"] if s else None))

            for a, b in zip(arms, arms[1:]):
                am = per_arm.get(a, {})
                bm = per_arm.get(b, {})
                common, abs_d, pct_d = paired_delta(am, bm, "p50")
                if not abs_d:
                    row.append("—")
                    continue
                med_abs = stats.median(abs_d)
                med_pct = stats.median(pct_d) if pct_d else 0.0
                cvp = max(
                    [c for c in (cv_pct([am[r]["p50"] for r in common if r in am]),
                                 cv_pct([bm[r]["p50"] for r in common if r in bm])) if c is not None]
                    or [None]
                ) if common else None
                counts = [am[r]["count"] for r in common if am[r].get("count") is not None] + \
                         [bm[r]["count"] for r in common if bm[r].get("count") is not None]
                min_count = min(counts) if counts else None
                erra = stats.median([am[r]["err"] for r in common if r in am])
                errb = stats.median([bm[r]["err"] for r in common if r in bm])
                err_gap = abs(errb - erra)
                sig, _ = sign_string(abs_d)
                flag = flag_for(pct_d, abs_d, cvp, min_count, err_gap, len(common))
                row.append(f"{med_pct:+.1f}% [{flag}]")

                csv_rows.append({
                    "vu": vu, "test": test, "from": a, "to": b,
                    "meaning": ADJACENCY_MEANING.get((a, b), "bundled"),
                    "median_dp50_ms": round(med_abs, 3),
                    "median_dp50_pct": round(med_pct, 2),
                    "min_dp50_ms": round(min(abs_d), 3),
                    "max_dp50_ms": round(max(abs_d), 3),
                    "pooled_cv_pct": None if cvp is None else round(cvp, 2),
                    "sign": sig, "min_count": min_count,
                    "err_from": round(erra, 4), "err_to": round(errb, 4),
                    "rounds_paired": len(common), "flag": flag,
                    "from_p50_median": None if not arm_stats[a] else round(arm_stats[a]["median"], 3),
                    "to_p50_median": None if not arm_stats[b] else round(arm_stats[b]["median"], 3),
                })
            out.append("| " + " | ".join(row) + " |")
        out.append("")

    out.append(
        "_Flags: SIGNAL = sign-consistent across rounds and > 2× within-arm noise; "
        "WEAK = consistent but under that bar; NOISE = sign flips or within noise; "
        "INCONCLUSIVE = < 2 paired rounds or < "
        f"{LOW_SAMPLE} requests (low-rps tests); VOID = arms' error rates differ. "
        "Full per-round spread is in the CSV artifact._"
    )
    print("\n".join(out))

    if args.csv_out and csv_rows:
        os.makedirs(os.path.dirname(args.csv_out) or ".", exist_ok=True)
        with open(args.csv_out, "w", newline="") as fh:
            w = csv.DictWriter(fh, fieldnames=list(csv_rows[0].keys()))
            w.writeheader()
            w.writerows(csv_rows)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
