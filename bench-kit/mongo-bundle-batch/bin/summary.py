#!/usr/bin/env python3
"""Paired comparison of every arm against the baseline arm. Stdlib only.

  python3 -I bin/summary.py <session-dir> [--baseline LABEL] [--boot 10000] [--seed 1] [--md out.md]
  python3 -I bin/summary.py --csv runs.csv      # columns: label,round,total_s[,rate_res_s,p50_ms,p95_ms,hfs_rss_peak_mb,mongo_mem_peak_mb]

For each non-baseline arm and metric: per-round paired percent difference (arm - baseline) / baseline * 100 (rounds in which
both runs are valid), the MEDIAN of those differences, the mean, and a bootstrap 95 % percentile CI of the median
(pairs resampled with replacement, fixed seed). With n = 3 pairs the CI is necessarily wide (it can only span the observed
differences); report it honestly and add rounds if it crosses 0. Time metrics: negative = faster. Rate: positive = better.
"""
import argparse, csv, glob, json, os, random, statistics as st, sys

METRICS = [("total_s", "total time (s)", "lower"), ("rate_res_s", "import rate (resources/s)", "higher"),
           ("p50_ms", "per-Bundle p50 (ms)", "lower"), ("p95_ms", "per-Bundle p95 (ms)", "lower"),
           ("hfs_rss_peak_mb", "hfs peak RSS (MB)", "info"), ("mongo_mem_peak_mb", "mongod container peak mem (MB)", "info")]


def load(a):
    runs = []
    if a.csv:
        for r in csv.DictReader(open(a.csv)):
            d = {"label": r["label"], "round": int(r["round"]), "valid": True}
            for k, v in r.items():
                if k not in ("label", "round") and v not in ("", None):
                    d[k] = float(v)
            runs.append(d)
        return runs, None
    for f in sorted(glob.glob(os.path.join(a.dir, "*", "run.json"))):
        runs.append(json.load(open(f)))
    sj = os.path.join(a.dir, "session.json")
    return runs, (json.load(open(sj)) if os.path.exists(sj) else None)


def boot_ci(d, n, rng):
    if len(d) < 2:
        return None
    m = sorted(st.median([rng.choice(d) for _ in d]) for _ in range(n))
    return m[int(0.025 * n)], m[min(n - 1, int(0.975 * n))]


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("dir", nargs="?")
    ap.add_argument("--csv")
    ap.add_argument("--baseline")
    ap.add_argument("--boot", type=int, default=10000)
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--md")
    a = ap.parse_args()
    if not a.dir and not a.csv:
        ap.error("give a session directory or --csv")
    runs, sess = load(a)
    labels = []
    for r in runs:
        if r["label"] not in labels:
            labels.append(r["label"])
    if sess:
        labels = [x["label"] for x in sess["arms"] if x["label"] in labels] or labels
    base = a.baseline or (sess or {}).get("baseline") or labels[0]
    if base not in labels:
        sys.exit("baseline %r has no runs" % base)
    by = {(r["label"], r["round"]): r for r in runs}
    rounds = sorted({r["round"] for r in runs})
    out = []
    P = out.append
    P("# Paired summary (baseline = %s)\n" % base)
    inval = [r for r in runs if not r.get("valid", True)]
    if inval:
        P("Invalid runs (excluded together with their round partner): " + "; ".join("%s (%s)" % (r["name"], r.get("invalid_reason", "?")) for r in inval) + "\n")
    P("| run | valid | total_s | resources/s | p50 ms | p95 ms | errors | hfs RSS peak MB | mongod mem peak MB | load1 before |")
    P("|---|---|---|---|---|---|---|---|---|---|")
    for r in sorted(runs, key=lambda r: (r["round"], labels.index(r["label"]) if r["label"] in labels else 99)):
        g = lambda k: ("%.1f" % r[k]) if isinstance(r.get(k), (int, float)) else ""
        P("| %s-r%d | %s | %s | %s | %s | %s | %s | %s | %s | %s |" % (r["label"], r["round"], "yes" if r.get("valid", True) else "NO", g("total_s"), g("rate_res_s"),
          g("p50_ms"), g("p95_ms"), r.get("errors", ""), g("hfs_rss_peak_mb"), g("mongo_mem_peak_mb"), (r.get("loadavg_before") or "").split(" ")[0]))
    P("")
    rng = random.Random(a.seed)
    for lab in labels:
        if lab == base:
            continue
        ok = [k for k in rounds if (base, k) in by and (lab, k) in by and by[(base, k)].get("valid", True) and by[(lab, k)].get("valid", True)]
        P("## %s vs %s (%d valid pairs: rounds %s)\n" % (lab, base, len(ok), ok))
        if not ok:
            P("No complete pair.\n")
            continue
        P("| metric | %s per round | %s per round | paired diffs %% | median %% | mean %% | bootstrap 95 %% CI of median | verdict |" % (base, lab))
        P("|---|---|---|---|---|---|---|---|")
        for key, title, better in METRICS:
            pairs = [(by[(base, k)].get(key), by[(lab, k)].get(key)) for k in ok]
            if any(x is None or y is None or x == 0 for x, y in pairs):
                continue
            d = [(y - x) / x * 100 for x, y in pairs]
            med = st.median(d)
            ci = boot_ci(d, a.boot, rng)
            A = [x for x, _ in pairs]
            B = [y for _, y in pairs]
            spread = (max(A) - min(A)) / (sum(A) / len(A)) * 100
            if better == "info":
                v = "memory (informational)"
            else:
                good = (lambda z: z < 0) if better == "lower" else (lambda z: z > 0)
                every = all(good(z) for z in d)
                beat = (max(B) < min(A)) if better == "lower" else (min(B) > max(A))
                ci_ok = ci is not None and len(d) >= 3 and (ci[1] < 0 if better == "lower" else ci[0] > 0)
                v = ("better" if good(med) else "worse") + ("; every pair favours the arm" if every else "; NOT every pair agrees") + \
                    ("; no overlap with baseline runs" if beat else "") + ("; CI excludes 0" if ci_ok else "; CI includes 0 (or n too small)") + \
                    "; baseline spread %.1f%%" % spread
            P("| %s | %s | %s | %s | %+.1f | %+.1f | %s | %s |" % (title, ", ".join("%.1f" % x for x in A), ", ".join("%.1f" % y for y in B),
              ", ".join("%+.1f" % z for z in d), med, sum(d) / len(d), ("[%+.1f, %+.1f]" % ci) if ci else "n/a", v))
        P("")
    P("Method: paired percent difference per round; median; bootstrap = %d resamples of the pairs, percentile 2.5/97.5, seed %d. With few pairs (3 in the standard plan) the bootstrap CI can only span the observed differences, so it is a sanity check, not strong inference." % (a.boot, a.seed))
    txt = "\n".join(out)
    print(txt)
    if a.md:
        open(a.md, "w").write(txt + "\n")


if __name__ == "__main__":
    main()
