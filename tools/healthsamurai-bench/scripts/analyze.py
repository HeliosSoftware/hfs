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

import json, statistics as st
rows = []
def add(server, d, name):
    global D
    for suite in ("prewarm","crud","import","search"):
        for rd in (1,2,3):
            p = os.path.join(d, f"R{rd}-{name}-{suite}.log")
            r = parse(p)
            if suite == "import": r["wall"] = wall(p)
            r.update({"server": server, "suite": suite, "round": rd})
            rows.append(r)
add("HFS Postgres+ES", DES, "hfses")
add("HFS Postgres+ES (ES uncapped)", DES_WIDE, "hfses")
add("WSO2", D, "wso2")
add("HFS Postgres", D, "hfs")
json.dump(rows, open(os.environ.get("BENCH_RUNS_JSON", "bench-runs.json"), "w"), indent=1)
def med(server, suite, k): return st.median([r[k] for r in rows if r["server"]==server and r["suite"]==suite])
def rng(server, suite, k):
    v=[r[k] for r in rows if r["server"]==server and r["suite"]==suite]; return min(v), max(v)
keys = {"prewarm":["rps","p95","med"],"crud":["rps","p95","med"],"import":["wall","p95","med"],"search":["rps","p95","med"]}
for suite, ks in keys.items():
    for k in ks:
        e,w,p = (med(s,suite,k) for s in ("HFS Postgres+ES (ES uncapped)","WSO2","HFS Postgres"))
        er = rng("HFS Postgres+ES (ES uncapped)",suite,k); wr = rng("WSO2",suite,k)
        print(f"{suite:8} {k:5} ES={e:10.2f} {er[0]:.2f}-{er[1]:.2f} | WSO2={w:10.2f} {wr[0]:.2f}-{wr[1]:.2f} | PG={p:10.2f} | ES/WSO2={e/w:.2f}")
