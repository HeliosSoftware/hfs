import re, statistics as st, sys
import os
HERE = os.path.dirname(os.path.abspath(__file__))
RES = os.environ.get("BENCH_RESULTS", os.path.join(HERE, "..", "results"))
D = os.environ.get("RESULTS_PG", os.path.join(RES, "postgres-vs-wso2"))
DES = os.environ.get("RESULTS_ES_SHARED", os.path.join(RES, "es-shared-budget"))
DES_WIDE = os.environ.get("RESULTS_ES_OWN", os.path.join(RES, "es-own-budget"))

def ms(v):
    m = re.match(r"([\d.]+)(µs|ms|s|m|h)?", v)
    return None

def dur(s):
    # k6 durations like 5m13s, 683.35ms, 4.94ms, 380.93µs, 1.79s, 13.7s
    s = s.strip()
    tot = 0.0
    for num, unit in re.findall(r"([\d.]+)(µs|ms|h|m|s)", s):
        n = float(num)
        tot += n * {"µs": 0.001, "ms": 1, "s": 1000, "m": 60000, "h": 3600000}[unit]
    return tot

def parse(path):
    t = open(path, encoding="utf-8", errors="replace").read()
    t = t[t.rfind("TOTAL RESULTS"):]
    r = {}
    m = re.search(r"http_req_duration\.+: avg=(\S+)\s+min=(\S+)\s+med=(\S+)\s+max=(\S+)\s+p\(90\)=(\S+)\s+p\(95\)=(\S+)", t)
    r["avg"], r["min"], r["med"], r["max"], r["p90"], r["p95"] = [dur(x) for x in m.groups()]
    m = re.search(r"http_reqs\.+: (\d+)\s+([\d.]+)/s", t); r["reqs"] = int(m.group(1)); r["rps"] = float(m.group(2))
    m = re.search(r"iterations\.+: (\d+)\s+([\d.]+)/s", t); r["iters"] = int(m.group(1)); r["ips"] = float(m.group(2))
    m = re.search(r"http_req_failed\.+: ([\d.]+)%\s+(\d+) out of (\d+)", t); r["failpct"] = float(m.group(1)); r["failn"] = int(m.group(2))
    m = re.search(r"checks_failed\.+: ([\d.]+)%", t); r["chkfail"] = float(m.group(1))
    m = re.search(r"iteration_duration\.+: avg=(\S+)\s+min=(\S+)\s+med=(\S+)\s+max=(\S+)\s+p\(90\)=(\S+)\s+p\(95\)=(\S+)", t)
    r["it_avg"], _, r["it_med"], _, r["it_p90"], r["it_p95"] = [dur(x) for x in m.groups()]
    m = re.search(r"running \((?:(\d+)h)?(\d+)m([\d.]+)s\)", t)
    return r

def wall(path):
    t = open(path, encoding="utf-8", errors="replace").read()
    ms_ = re.findall(r"running \((?:(\d+)h)?(\d+)m([\d.]+)s\), 00/", t)
    h, m, s = ms_[-1]
    return int(h or 0) * 3600 + int(m) * 60 + float(s)

data = {}
for s in ("hfs", "wso2"):
    for suite in ("prewarm", "crud", "import", "search"):
        for rd in (1, 2, 3):
            r = parse(os.path.join(D, f"R{rd}-{s}-{suite}.log"))
            if suite == "import":
                r["wall"] = wall(os.path.join(D, f"R{rd}-{s}-{suite}.log"))
            data[(s, suite, rd)] = r

# metric: (key, label, higher_is_better, fmt)
M = {
 "prewarm": [("rps","HTTP req/s",1,"{:.0f}"),("avg","latency avg ms",0,"{:.2f}"),("med","latency p50 ms",0,"{:.2f}"),("p90","latency p90 ms",0,"{:.2f}"),("p95","latency p95 ms",0,"{:.2f}"),("max","latency max ms",0,"{:.0f}"),("ips","iterations/s",1,"{:.1f}")],
 "crud":    [("rps","HTTP req/s",1,"{:.0f}"),("ips","iterations/s",1,"{:.1f}"),("avg","latency avg ms",0,"{:.1f}"),("med","latency p50 ms",0,"{:.1f}"),("p90","latency p90 ms",0,"{:.1f}"),("p95","latency p95 ms",0,"{:.1f}"),("max","latency max ms",0,"{:.0f}"),("failpct","HTTP failed %",0,"{:.2f}")],
 "import":  [("wall","wall time s (1000 bundles)",0,"{:.0f}"),("ips","iterations/s",1,"{:.3f}"),("avg","latency avg ms",0,"{:.0f}"),("med","latency p50 ms",0,"{:.0f}"),("p90","latency p90 ms",0,"{:.0f}"),("p95","latency p95 ms",0,"{:.0f}"),("max","latency max ms",0,"{:.0f}"),("failpct","HTTP failed %",0,"{:.2f}")],
 "search":  [("rps","HTTP req/s",1,"{:.0f}"),("ips","iterations/s",1,"{:.1f}"),("avg","latency avg ms",0,"{:.2f}"),("med","latency p50 ms",0,"{:.2f}"),("p90","latency p90 ms",0,"{:.2f}"),("p95","latency p95 ms",0,"{:.2f}"),("max","latency max ms",0,"{:.0f}"),("failpct","HTTP failed %",0,"{:.2f}")],
}

def cv(xs):
    m = st.mean(xs)
    return 0.0 if m == 0 else (max(xs) - min(xs)) / st.median(xs) * 100

out = []
for suite in ("prewarm", "crud", "import", "search"):
    out.append(f"\n### {suite.upper()}\n")
    out.append("| metric | better | HFS R1 | R2 | R3 | HFS median | spread% | WSO2 R1 | R2 | R3 | WSO2 median | spread% | ratio HFS/WSO2 | verdict |")
    out.append("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|")
    for key, label, hib, f in M[suite]:
        h = [data[("hfs", suite, i)][key] for i in (1, 2, 3)]
        w = [data[("wso2", suite, i)][key] for i in (1, 2, 3)]
        hm, wm = st.median(h), st.median(w)
        ratio = (hm / wm) if wm else float("nan")
        # separated ranges = gap beyond round noise
        sep = (min(h) > max(w)) or (max(h) < min(w))
        if key == "failpct" and hm == 0 and wm == 0:
            verdict = "tie (0 failures)"
        else:
            good = (hm > wm) if hib else (hm < wm)
            if not sep: verdict = "within noise (ranges overlap)" + (" ~" if abs(ratio-1) < 0.05 else "")
            else: verdict = ("HFS better" if good else "HFS worse") + " (ranges do not overlap)"
        out.append(f"| {label} | {'higher' if hib else 'lower'} | " + " | ".join(f.format(x) for x in h) + f" | {f.format(hm)} | {cv(h):.1f} | " + " | ".join(f.format(x) for x in w) + f" | {f.format(wm)} | {cv(w):.1f} | {ratio:.2f} | {verdict} |")
    # validity
    chk = [data[(s, suite, i)]["chkfail"] for s in ("hfs", "wso2") for i in (1, 2, 3)]
    out.append(f"\nchecks_failed across all 6 runs: {chk}")
print("\n".join(out))
