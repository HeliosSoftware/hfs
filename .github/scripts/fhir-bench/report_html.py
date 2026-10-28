#!/usr/bin/env python3
"""Standalone dark HTML report of one FHIR Server Benchmark run (single self-contained file).

    python3 report_html.py --run-dir DIR --run-id ID --repo OWNER/REPO --branch BRANCH --sha SHA \
                           --out fhir-bench-report-ID.html --dump-json report-data.json \
                           [--trend-dir DIR] [--no-shapes] [--headline-leg LEG]

DIR holds the results of every leg, either as `fhir-benchmark-<leg>-<run id>/<leg>/` (what
`actions/download-artifact` with a pattern leaves) or as `<leg>/` (already extracted); legs are
discovered from the folder names. A `comparison.md` in DIR (or in a `fhir-bench-comparison-*`
folder of DIR, or next to it) feeds the summary tables at the top.

--trend-dir holds `<run id>/report-data.json` of EARLIER runs (the --dump-json output of this
script); the Trend section charts what is there. --no-shapes skips the heavy search-points.json
(search-by-shape.md is used when present).

Standard library only. Every number and every validity flag comes from the artifacts, with the
thresholds of compare_legs.py. A missing file or leg is shown as "no data". The exit code is
always 0: on any failure a minimal valid HTML page is still written.
"""
import argparse
import html
import json
import math
import os
import re
import sys
from array import array
from concurrent.futures import ProcessPoolExecutor

# Same thresholds as compare_legs.py
IMPORT_TARGET = 1000
ERR_PCT_HIGH = 1.0
PG_CONTENDED_MS = 3.0
TREND_MAX = 8

K6_SUITES = ["prewarm", "import", "crud", "search", "sql-run", "insert"]
LEG_LABEL_ES = " + ES"
ISO_RE = re.compile(r"(20\d\d-\d\d-\d\d)T(\d\d:\d\d:\d\d)")
HERE = os.path.dirname(os.path.abspath(__file__))
LEG_FILES = ("runner-info.txt", "import.json", "crud.json", "search.json", "sql-run.json", "insert.json",
             "prewarm.json", "host-contention.txt", "bulk-import.txt")

# v8 colours of the six known legs (validated with the dataviz palette validator).
PAL_KNOWN = {
    "dark": {"postgres": "#236bcf", "sqlite": "#008622", "mongodb": "#9348b1",
             "postgres-elasticsearch": "#4a91f8", "sqlite-elasticsearch": "#3fac4a",
             "mongodb-elasticsearch": "#b96dd8"},
    "light": {"postgres": "#1a52a1", "sqlite": "#036819", "mongodb": "#723789",
              "postgres-elasticsearch": "#4780d2", "sqlite-elasticsearch": "#419547",
              "mongodb-elasticsearch": "#9f63b9"}}
# Hues for any other backend, cycled in the order of the sorted backend names.
PAL_EXTRA = {"dark": ["#c9a227", "#d1568f", "#2fb5bd", "#d9783c", "#8f9bb3", "#7cb342"],
             "light": ["#8a6d00", "#a63a6e", "#16757c", "#a8501a", "#5b6785", "#4a7a1c"]}
KNOWN_BASES = ("postgres", "sqlite", "mongodb")


def label(leg):
    if leg.endswith("-elasticsearch"):
        return leg[:-len("-elasticsearch")] + LEG_LABEL_ES
    return leg


# ── tolerant reading ────────────────────────────────────────────────

def read_text(path):
    try:
        with open(path, encoding="utf-8", errors="replace") as fh:
            return fh.read()
    except OSError:
        return None


def kv(path, sep):
    out = {}
    for line in (read_text(path) or "").splitlines():
        if sep in line:
            k, v = line.strip().split(sep, 1)
            out[k.strip()] = v.strip()
    return out


def fnum(v):
    try:
        x = float(str(v).replace(",", ""))
        return x if math.isfinite(x) else None
    except (TypeError, ValueError):
        return None


def inum(v):
    x = fnum(v)
    return None if x is None else int(x)


def rnd(v, n=2):
    return None if v is None else round(v, n)


def _is_leg_dir(d):
    return os.path.isdir(d) and any(os.path.exists(os.path.join(d, f)) for f in LEG_FILES)


def _find_leg_dir(p, name):
    """The folder with the files of a leg inside p: p/<name>, p itself, or any single sub-folder."""
    for c in (os.path.join(p, name), p):
        if _is_leg_dir(c):
            return c
    try:
        for sub in sorted(os.listdir(p)):
            c = os.path.join(p, sub)
            if _is_leg_dir(c):
                return c
    except OSError:
        pass
    return None


def leg_dirs(run_dir):
    """{leg: path}. Layouts: fhir-benchmark-<leg>-<run id>/<leg>/, <leg>/ or <leg>/<leg>/."""
    out = {}
    if not run_dir or not os.path.isdir(run_dir):
        return out
    try:
        names = sorted(os.listdir(run_dir))
    except OSError:
        return out
    for name in names:
        p = os.path.join(run_dir, name)
        if not os.path.isdir(p) or name.startswith("fhir-bench-comparison"):
            continue
        m = re.match(r"^fhir-benchmark-(.+?)(?:-(\d+))?$", name)
        guess = m.group(1) if m else name
        d = _find_leg_dir(p, guess)
        if not d:
            continue
        leg = guess if m else name
        out[leg] = d
    if not out and _is_leg_dir(run_dir):  # DIR is itself one leg
        out[os.path.basename(os.path.normpath(run_dir))] = run_dir
    return out


def order_legs(legs):
    """Stable order: known backends first (plain before +ES), then the rest alphabetically."""
    known = ["postgres", "postgres-elasticsearch", "sqlite", "sqlite-elasticsearch", "mongodb",
             "mongodb-elasticsearch"]
    return [l for l in known if l in legs] + sorted(l for l in legs if l not in known)


# ── per-leg extraction ───────────────────────────────────────────────

def load_suite(d, suite, ic):
    t = read_text(os.path.join(d, suite + ".json"))
    if t is None:
        return None
    try:
        m = json.loads(t)["metrics"]
    except (ValueError, KeyError, TypeError):
        return None
    if not isinstance(m, dict):
        return None
    hr, hf = m.get("http_reqs"), m.get("http_req_failed")
    if not isinstance(hr, dict) or fnum(hr.get("count")) is None or not isinstance(hf, dict) \
            or fnum(hf.get("value")) is None:
        return None  # incomplete summary: missing data, not zeros

    def g(name, key):
        x = m.get(name)
        return x.get(key) if isinstance(x, dict) else None

    s = {"rps": rnd(fnum(g("http_reqs", "rate"))), "reqs": inum(g("http_reqs", "count")),
         "p50": rnd(fnum(g("http_req_duration", "med"))), "p95": rnd(fnum(g("http_req_duration", "p(95)"))),
         "p99": rnd(fnum(g("http_req_duration", "p(99)"))), "max": rnd(fnum(g("http_req_duration", "max"))),
         "err": rnd((fnum(g("http_req_failed", "value")) or 0) * 100, 3),
         "ok": inum(g("checks", "passes")), "fail": inum(g("checks", "fails")),
         "vus": inum(g("vus_max", "max")), "unit": "req/s"}
    if suite == "import":
        # resources/s = entries / scenario_seconds; latency per Bundle (iteration_duration)
        ent, sc = fnum(ic.get("entries")), fnum(ic.get("scenario_seconds"))
        s["rps"] = rnd(ent / sc) if ent is not None and sc else None
        s["p50"], s["p95"] = rnd(fnum(g("iteration_duration", "med"))), rnd(fnum(g("iteration_duration", "p(95)")))
        s["p99"], s["max"] = rnd(fnum(g("iteration_duration", "p(99)"))), rnd(fnum(g("iteration_duration", "max")))
        s["unit"] = "resources/s"
    if suite == "sql-run":
        s["rows_rps"] = rnd(fnum(g("sql_run_rows", "rate")), 1)
        s["rows"] = inum(g("sql_run_rows", "count"))
        views = []
        for k, v in m.items():
            vm = re.match(r"^http_req_duration\{view:([^}]+)\}$", k)
            if vm and isinstance(v, dict):
                views.append({"name": vm.group(1), "p50": rnd(fnum(v.get("med"))), "p95": rnd(fnum(v.get("p(95)"))),
                              "p99": rnd(fnum(v.get("p(99)"))), "max": rnd(fnum(v.get("max"))),
                              "n": inum(v.get("count"))})
        views.sort(key=lambda r: r["name"])
        s["views"] = views
    return s


def host_points(d):
    pts = []
    for line in (read_text(os.path.join(d, "host-contention.txt")) or "").splitlines():
        m = re.match(r"(\d\d:\d\d:\d\d)Z suite=(\S+) phase=(\S+) host_loadavg=(\S+) (\S+) (\S+)"
                     r"(?: host_containers=(\S+))?(?: host_mem_avail_mb=(\S+))?", line)
        if m and fnum(m.group(4)) is not None:
            pts.append({"t": m.group(1), "suite": m.group(2), "phase": m.group(3),
                        "la1": fnum(m.group(4)), "la5": fnum(m.group(5)),
                        "containers": inum(m.group(7)), "mem_mb": fnum(m.group(8))})
    return pts


def pg_ms(d):
    """Mean ms per Postgres call in crud; None when the leg has no Postgres statistics."""
    t = read_text(os.path.join(d, "crud-pgstat.txt"))
    m = re.search(r"^\s*([\d.]+)\s*\|\s*[\d.]+\s*\|\s*(\d+)\s*$", t or "", re.M)
    if not m or not int(m.group(2)):
        return None
    return round(float(m.group(1)) * 1000 / int(m.group(2)), 3)


def leg_start(d, pts):
    """Approximate ISO start of the leg: first ISO date in its files + first host reading time."""
    day = None
    for f in ("containers-state.txt", "bulk-import-hfs.log", "bulk-import.log"):
        m = ISO_RE.search(read_text(os.path.join(d, f)) or "")
        if m:
            day = m.group(1)
            break
    if day is None:
        try:
            with open(os.path.join(d, "search-points.json"), encoding="utf-8", errors="replace") as fh:
                for _ in range(50):
                    m = ISO_RE.search(fh.readline())
                    if m:
                        day = m.group(1)
                        break
        except OSError:
            pass
    if day is None or not pts:
        return None  # no host reading: the start is unknown, never T00:00:00Z
    return "%sT%sZ" % (day, pts[0]["t"])


def shape_stats(points_path):
    """Port of search_by_shape.py: http_req_duration per shape (med = s[n//2], p95 = s[floor(n*.95)])."""
    by = {}
    try:
        with open(points_path, "r", encoding="utf-8", errors="replace") as fh:
            for line in fh:
                if '"Point"' not in line or '"http_req_duration"' not in line:
                    continue
                try:
                    o = json.loads(line)
                except ValueError:
                    continue
                if not isinstance(o, dict) or o.get("type") != "Point" or o.get("metric") != "http_req_duration":
                    continue
                data = o.get("data") or {}
                v = data.get("value")
                if not isinstance(v, (int, float)) or isinstance(v, bool):
                    continue
                tags = data.get("tags") or {}
                st = tags.get("searchType")
                if st in (None, False):
                    continue
                key = (str(st), "%s?%s" % (tags.get("resourceType") or "?", tags.get("name") or "?"))
                by.setdefault(key, array("d")).append(float(v))
    except OSError:
        return None
    rows = []
    for (typ, q), vals in by.items():
        s = sorted(vals)
        n = len(s)
        rows.append({"type": typ, "q": q, "n": n, "p50": round(s[n // 2], 2),
                     "p95": round(s[min(math.floor(n * 0.95), n - 1)], 2), "max": round(s[-1], 2)})
    rows.sort(key=lambda r: (-r["p95"], r["type"] + r["q"]))
    return rows


def shapes_from_md(path):
    rows = []
    for line in (read_text(path) or "").splitlines():
        m = re.match(r"\| `(\S+) (.+?)` \| (\d+) \| (\d+) \| (\d+) \| (\d+) \|", line)
        if m and m.group(1) != "?":
            rows.append({"type": m.group(1), "q": m.group(2), "n": int(m.group(3)), "p50": int(m.group(4)),
                         "p95": int(m.group(5)), "max": int(m.group(6))})
    return rows or None


def docker_host_from(ri):
    """(cpus, ram_gb) of the Docker host when a runner-info key states them; else (None, None)."""
    cpus = ram = None
    for k, v in ri.items():
        kl = k.lower()
        if "docker" not in kl:
            continue
        x = fnum(v)
        if x is None or x <= 0:
            continue
        if "cpu" in kl:
            cpus = int(x)
        elif "gb" in kl:
            ram = x
        elif "mb" in kl:
            ram = x / 1024.0
        elif "byte" in kl or "mem" in kl or "ram" in kl:
            ram = x / 1e9 if x > 1e6 else x
    return cpus, (round(ram) if ram else None)


def load_leg(leg, d):
    ic_raw = kv(os.path.join(d, "import-completeness.txt"), "=")
    ri = kv(os.path.join(d, "runner-info.txt"), ":")
    out = {"leg": leg, "label": label(leg), "runner": ri.get("runner_name"), "cpus": ri.get("runner_cpus"),
           "ram": ri.get("runner_ram"), "sha": (ri.get("git_sha") or "")[:9] or None,
           "ref": ri.get("git_ref"), "run": ri.get("github_run"), "suites": {}}
    out["docker_cpus"], out["docker_ram_gb"] = docker_host_from(ri)
    for s in K6_SUITES:
        v = load_suite(d, s, ic_raw)
        if v:
            out["suites"][s] = v
        elif os.path.exists(os.path.join(d, s + ".json")):
            out["suites"][s] = None  # file present but unreadable/incomplete: no data
    out["ic"] = None
    if ic_raw:
        out["ic"] = {"bundles_ok": inum(ic_raw.get("bundles_ok")), "entries": inum(ic_raw.get("entries")),
                     "scenario": fnum(ic_raw.get("scenario_seconds")), "setup": fnum(ic_raw.get("setup_seconds")),
                     "wall": fnum(ic_raw.get("wall_seconds"))}
    bi = kv(os.path.join(d, "bulk-import.txt"), "=")
    out["bulk"] = None
    if bi:
        ingest, index = fnum(bi.get("ingest_seconds")), fnum(bi.get("index_seconds"))
        status = bi.get("index_status")
        inline = status == "inline"
        out["bulk"] = {"status": bi.get("status"), "reason": bi.get("reason") or "", "phase": bi.get("phase"),
                       "convert": fnum(bi.get("convert_seconds")), "ingest": ingest,
                       "index": None if inline else index, "index_inline": inline,
                       # seconds shown on the "search indexing" row: inline indexing happens during ingest
                       "index_eff": ingest if inline else index,
                       "total": fnum(bi.get("total_seconds")),
                       "rps": fnum(bi.get("resources_per_s")), "ingest_rps": fnum(bi.get("ingest_resources_per_s")),
                       "ok": inum(bi.get("resources_ok")), "submitted": inum(bi.get("resources_submitted")),
                       "failed": inum(bi.get("resources_failed")), "index_status": status,
                       "searchable": bi.get("searchable") == "yes",
                       "searchable_s": fnum(bi.get("searchable_seconds"))}
    dr = kv(os.path.join(d, "es-drain.txt"), "=") if leg.endswith("-elasticsearch") else {}
    out["drain"] = ({"status": dr.get("status"), "missing": dr.get("missing"), "sync": dr.get("sync_mode")}
                    if dr else None)
    cs_t = read_text(os.path.join(d, "containers-state.txt"))
    lines = [l.strip() for l in (cs_t or "").splitlines() if l.strip() and not l.startswith("hfs-bench-tgz-")]
    out["containers_known"] = cs_t is not None
    out["containers"] = [" ".join(l.split()[:3]) for l in lines if "Status=" in l or "inspect-failed" in l]
    out["died"] = any(re.search(r"OOMKilled=true|Status=(exited|dead)|inspect-failed gone", l) for l in lines)
    out["host"] = host_points(d)
    out["pg_ms"] = pg_ms(d)
    out["counts"] = []
    for l in (read_text(os.path.join(d, "search-counts.txt")) or "").splitlines()[1:]:
        p = l.strip().split("|")
        if len(p) == 4:
            out["counts"].append({"q": p[0], "total": p[1], "http": p[2]})
    res = kv(os.path.join(d, "crud-residue.txt"), "=")
    out["residue"] = sum(int(v) for v in res.values() if v.isdigit()) if res else None
    out["start"] = leg_start(d, out["host"])
    out["markers"] = markers(out)
    return out


def markers(d):
    """Rules of compare_legs.py _markers()."""
    mk = {}
    ok = d["ic"]["bundles_ok"] if d["ic"] else None
    for s, v in d["suites"].items():
        if not v:
            continue
        m = ""
        if s in ("crud", "search"):
            inc = not d["suites"].get("import") or ok is None or ok < IMPORT_TARGET
            if not inc and s == "search" and d["leg"].endswith("-elasticsearch"):
                inc = not d["drain"] or d["drain"]["status"] != "drained"
            if inc:
                m += "†"
        if v["err"] > ERR_PCT_HIGH or (v["fail"] or 0) > 0:
            m += "‡"
        if d["died"] or (s == "crud" and d["leg"] == "postgres" and d["pg_ms"] is not None and d["pg_ms"] > PG_CONTENDED_MS):
            m += "⚠"
        mk[s] = m
    b = d["bulk"]
    if b and b["status"] == "complete":
        m = "" if b["searchable"] else "†"
        if b["failed"] is not None and b["submitted"] and (100.0 * b["failed"] / b["submitted"] > ERR_PCT_HIGH or b["failed"] > 0):
            m += "‡"
        if b.get("index_status") == "failed":
            m += "‡"  # a failed rebuild is not a valid indexing time
        if d["died"]:
            m += "⚠"
        mk["bulk"] = m
    return mk


# ── validity ─────────────────────────────────────────────────────────

def r0(v):
    return int(math.floor(v + 0.5))


def fmt_int(v):
    return "n/a" if v is None else "{:,}".format(r0(v))


def fmt1(v):
    return "n/a" if v is None else "{:,.1f}".format(v)


def leg_checks(d, cpus):
    c = {}
    ok = d["ic"]["bundles_ok"] if d["ic"] else None
    if ok is None:
        c["corpus"] = ["bad" if d["suites"].get("import") else "na", "no data"]
    else:
        c["corpus"] = ["ok" if ok >= IMPORT_TARGET else "bad", "%s/%s" % (fmt_int(ok), fmt_int(IMPORT_TARGET))]
    hard, soft = [], []
    nodata = [s for s in K6_SUITES if s in d["suites"] and not d["suites"][s]]
    for s in K6_SUITES:
        v = d["suites"].get(s)
        if not v:
            continue
        if v["err"] > ERR_PCT_HIGH:
            hard.append("%s %s %%" % (s, fmt1(v["err"])))
        elif v["fail"] is None:
            nodata.append("%s checks n/a" % s)
        elif v["fail"] > 0:
            soft.append("%s %s ✗" % (s, fmt_int(v["fail"])))
    b = d["bulk"]
    if b and b["failed"] is None:
        nodata.append("bulk failures n/a")
    elif b and b["failed"]:
        hard.append("bulk %s failed" % fmt_int(b["failed"]))
    if not any(d["suites"].values()) and not b:
        c["err"] = ["na", "no data"]
    elif hard:
        c["err"] = ["bad", ", ".join(hard + soft + [x if x.endswith("n/a") else "%s no data" % x for x in nodata])]
    elif nodata:
        c["err"] = ["na", ", ".join(soft + [x if x.endswith("n/a") else "%s no data" % x for x in nodata])]
    elif soft:
        c["err"] = ["warn", ", ".join(soft) + " (<1 %)"]
    else:
        c["err"] = ["ok", "0 %"]
    if d["died"]:
        c["cont"] = ["bad", "down"]
    elif d["containers"]:
        c["cont"] = ["ok", "alive"]
    else:
        c["cont"] = ["na", "no container" if d["containers_known"] else "no data"]
    if d["leg"].endswith("-elasticsearch"):
        st = d["drain"]["status"] if d["drain"] else None
        c["es"] = ["ok", "drained"] if st == "drained" else ["bad", st or "no data"]
    else:
        c["es"] = ["na", "—"]
    pk = max((p["la1"] for p in d["host"]), default=None)
    if pk is None:
        c["load"] = ["na", "no data"]
    else:
        c["load"] = ["warn" if cpus and pk > cpus else ("ok" if cpus else "na"), "peak %s" % fmt1(pk)]
    if d["pg_ms"] is not None:
        c["load"][1] += " · pg %.2f ms/call" % d["pg_ms"]
        # compare_legs.py applies the contention rule to the leg named `postgres` only
        if d["leg"] == "postgres" and d["pg_ms"] > PG_CONTENDED_MS:
            c["load"][0] = "bad"
    d["peak"] = pk
    return c


def hum(s):
    """Humanised duration in English: 11 s, 53 min 15 s, 1 h 05 min."""
    s = int(max(0, s))
    if s < 60:
        return "%d s" % s
    if s < 3600:
        m, r = divmod(s, 60)
        return "%d min" % m + (" %02d s" % r if r else "")
    h, r = divmod(s, 3600)
    return "%d h %02d min" % (h, r // 60)


WAIT_WARN_FRACTION = 0.10
_TIMING_INTS = ("total_s", "waiting_s", "working_s")
_JOB_INTS = ("queued_s", "behind_legs_s", "capacity_wait_s", "host_wait_s", "waiting_s", "working_s")
_JOB_KINDS = ("setup", "build", "leg", "compare", "other")


def _is_int(v):
    return isinstance(v, int) and not isinstance(v, bool) and v >= 0


def load_timing(path):
    """Validated run_timing.py JSON, or None (absent or invalid: the report is then as without it)."""
    if not path:
        return None
    try:
        with open(path, encoding="utf-8") as fh:
            t = json.load(fh)
        ok_ = (isinstance(t, dict) and isinstance(t.get("run_id"), str)
               and all(_is_int(t.get(k)) for k in _TIMING_INTS) and isinstance(t.get("jobs"), list))
        if ok_:
            for j in t["jobs"]:
                if not (isinstance(j, dict) and isinstance(j.get("name"), str) and j.get("kind") in _JOB_KINDS
                        and (j.get("leg") is None or isinstance(j.get("leg"), str))
                        and all(_is_int(j.get(k)) for k in _JOB_INTS)):
                    ok_ = False
                    break
        if not ok_:
            print("report_html: --timing-json %s is not a valid timing file; ignored" % path, file=sys.stderr)
            return None
        return t
    except Exception as e:  # noqa: BLE001
        print("report_html: --timing-json unreadable (%s); ignored" % e, file=sys.stderr)
        return None


def timing_view(t):
    """Texts for the tile, derived from the data only."""
    if not t:
        return None
    warn = t["total_s"] > 0 and t["waiting_s"] >= WAIT_WARN_FRACTION * t["total_s"]
    return {"label": "Waiting vs working",
            "value": "Waiting %s \u00b7 Working %s" % (hum(t["waiting_s"]), hum(t["working_s"])),
            "sub": "of %s total" % hum(t["total_s"]), "warn": warn,
            "sentence": ("This run waited %s for resources: it is slower to finish, not slower to run."
                         % hum(t["waiting_s"])) if warn else None}


def verdict_all(legs, order, cpus, missing, timing=None):
    """Verdict for the HTML: every leg is judged alike."""
    items = []
    four = ("corpus", "err", "cont", "es")
    valid = [l for l in order if all(legs[l]["checks"][k][0] != "bad" for k in four)
             and legs[l]["checks"]["corpus"][0] == "ok" and legs[l]["checks"]["err"][0] != "na"]
    if valid:
        items.append(["ok", "Valid (corpus, errors, containers, ES all fine): %s." % ", ".join(label(l) for l in valid)])
    for key, what, why in (
            ("corpus", "incomplete corpus", "its crud and search ran over less data and are not compared"),
            ("err", "errors or failed checks", "see the full table"),
            ("cont", "container down", "do not use its numbers"),
            ("es", "Elasticsearch not drained", "its search did not see all the data")):
        hit = [l for l in order if legs[l]["checks"][key][0] in ("bad", "warn")]
        if hit:
            sev = "bad" if any(legs[l]["checks"][key][0] == "bad" for l in hit) else "warn"
            items.append([sev, "%s: %s (%s); %s." % (", ".join(label(l) for l in hit), what,
                                                     ", ".join(legs[l]["checks"][key][1] for l in hit), why)])
    peaks = [legs[l]["peak"] for l in order if legs[l].get("peak") is not None]
    if peaks and cpus:
        over = sum(1 for p in peaks if p > cpus)
        if over:
            items.append(["warn", "Docker host above its %d CPUs in %d of %d legs (peak load %s to %s)." % (
                cpus, over, len(peaks), fmt1(min(peaks)), fmt1(max(peaks))), "load"])
    elif peaks:
        items.append(["na", "Host peak load %s to %s (host CPU count unknown)." % (
            fmt1(min(peaks)), fmt1(max(peaks))), "load"])
    if missing:
        items.append(["bad", "Legs without artifacts: %s." % ", ".join(label(l) for l in missing)])
    tv = timing_view(timing)
    if tv and tv["warn"]:
        items.append(["warn", tv["sentence"]])
    runners = sorted({legs[l]["runner"] for l in order if legs[l]["runner"]})
    items.append(["na", "This is a single run on a shared host%s: it is a sample, not proof." % (
        " and the legs used %d different runners" % len(runners) if len(runners) > 1 else "")])
    total = len(order) + len(missing)
    title = "%d of %d legs valid" % (len(valid), total)
    if not order and not missing:
        title = "no leg artifacts in this directory"
    sev = "bad" if not valid else ("ok" if len(valid) == total and not any(i[0] in ("bad", "warn") for i in items)
                                   else "warn")
    return {"title": title, "items": items, "sev": sev}


# ── comparison.md (summary tables at the top) ──────────────────────────

def _cell(c):
    return re.sub(r"\s+", " ", c.replace("**", "").replace("`", "")).strip()


def find_comparison(run_dir):
    cands = []
    for base in (run_dir, os.path.dirname(os.path.normpath(run_dir)) if run_dir else None):
        if not base or not os.path.isdir(base):
            continue
        cands.append(os.path.join(base, "comparison.md"))
        try:
            for n in sorted(os.listdir(base)):
                if n.startswith("fhir-bench-comparison"):
                    cands.append(os.path.join(base, n, "comparison.md"))
        except OSError:
            pass
    return next((c for c in cands if os.path.isfile(c)), None)


def parse_comparison(path):
    """{"head": str, "tables": [{"title", "header", "rows"}]} of the `### ...` tables of comparison.md."""
    t = read_text(path) if path else None
    if not t:
        return None
    out = {"head": "", "tables": []}
    title = ""
    cur = None
    for line in t.splitlines():
        if line.startswith("Commit ") and "legs" in line and not out["head"]:
            out["head"] = _cell(line)
        if line.startswith("#"):
            title = line.lstrip("#").strip()
            cur = None
            continue
        if line.strip().startswith("|"):
            cells = [_cell(c) for c in line.strip().strip("|").split("|")]
            if cur is None:
                if cells and cells[0] == "Suite":
                    cur = {"title": title, "header": cells, "rows": []}
                    out["tables"].append(cur)
                continue
            if set("".join(cells)) <= set("-: "):
                continue
            cur["rows"].append(cells)
        else:
            cur = None
    return out if out["tables"] else None


# ── trend ────────────────────────────────────────────────────────────

def trend_point(leg, x):
    """The slim record of one leg that later runs chart in their Trend section."""
    if not any(x["suites"].values()) and not x["bulk"]:
        return None
    cs = next((p for p in x["host"] if p["suite"] == "crud" and p["phase"] == "start"), None)
    suites = {s: {"rps": v["rps"], "p95": v["p95"]} for s, v in x["suites"].items() if v and s != "prewarm"}
    sq = x["suites"].get("sql-run")
    if sq:
        suites["sql_rows"] = {"rps": sq.get("rows_rps"), "p95": sq["p95"]}
    b = x["bulk"]
    return {"run": x["run"], "date": x["start"], "sha": x["sha"], "branch": x["ref"], "runner": x["runner"],
            "suites": suites,
            "bulk": b and {"rps": b["rps"], "total": b["total"], "ingest_rps": b["ingest_rps"],
                           "ingest": b["ingest"], "index_eff": b["index_eff"]},
            "index_inline": b["index_inline"] if b else None, "index_status": b["index_status"] if b else None,
            "bundles_ok": x["ic"]["bundles_ok"] if x["ic"] else None, "pg_ms": x["pg_ms"],
            "crud_load": cs["la1"] if cs else None, "markers": x["markers"]}


def run_key(t):
    r = str(t.get("run") or "")
    return (int(r) if r.isdigit() else 0, t.get("date") or "")


def load_history(trend_dir):
    """[(run id, dump)] of the earlier runs found as <trend_dir>/<run id>/report-data.json.

    TEMPORARY: a run without report-data.json may carry legacy-point.json (same shape, points derived from its
    comparison.md by history.py). Delete the second file name below when the legacy fallback is removed."""
    out = []
    if not trend_dir or not os.path.isdir(trend_dir):
        return out
    for name in sorted(os.listdir(trend_dir)):
        for fname in ("report-data.json", "legacy-point.json"):
            p = os.path.join(trend_dir, name, fname)
            try:
                with open(p, encoding="utf-8") as fh:
                    doc = json.load(fh)
                if isinstance(doc, dict) and isinstance(doc.get("legs"), dict):
                    out.append((name, doc))
                    break
            except (OSError, ValueError):
                continue
    return out


REF_METRICS = ("search", "crud", "insert", "sql-run", "sql_rows", "import",
               "bulk.rps", "bulk.total", "bulk.ingest_rps", "bulk.ingest", "bulk.index_eff")
MODE_METRICS = ("bulk.ingest_rps", "bulk.ingest", "bulk.index_eff")


def metric_value(pt, metric):
    if metric.startswith("bulk."):
        return (pt.get("bulk") or {}).get(metric[5:])
    return ((pt.get("suites") or {}).get(metric) or {}).get("rps")


def nearest_reference(metric, cur_point, older):
    """Nearest earlier point (of `older`, ascending) with a value for `metric`. Ingest and indexing metrics also
    need the same indexing mode (unknown only matches unknown) and a rebuild that did not fail."""
    for t in reversed(older):
        if metric_value(t, metric) is None:
            continue
        if metric in MODE_METRICS and (t.get("index_inline") != cur_point.get("index_inline")
                                       or t.get("index_status") == "failed"):
            continue
        return t
    return None


def leg_trend(leg, cur_point, history, run, trend_max=TREND_MAX):
    """(previous point or None, trend list including the current run, {metric: reference point or None}).
    Nothing assumes earlier runs exist."""
    pts = []
    for name, doc in history:
        lg = doc["legs"].get(leg)
        pt = lg.get("point") if isinstance(lg, dict) else None
        if isinstance(pt, dict) and isinstance(pt.get("suites"), dict):
            pt = dict(pt)
            # older report-data.json points lack the indexing mode: take it from the leg's bulk block
            bk = lg.get("bulk") or {}
            pt.setdefault("index_inline", bk.get("index_inline"))
            pt.setdefault("index_status", bk.get("index_status"))
            if pt.get("index_status") == "failed":
                mk = dict(pt.get("markers") or {})
                if "‡" not in (mk.get("bulk") or ""):
                    mk["bulk"] = (mk.get("bulk") or "") + "‡"
                pt["markers"] = mk
            pt["run"] = pt.get("run") or doc.get("run") or name
            if str(pt["run"]) != str(run):
                pts.append(pt)
    seen, trend = set(), []
    for t in sorted(pts, key=run_key)[-(max(1, trend_max) - 1):] + ([cur_point] if cur_point else []):
        if str(t["run"]) not in seen:
            seen.add(str(t["run"]))
            trend.append(t)
    trend.sort(key=run_key)
    prev, prev_by = None, {}
    if cur_point:
        older = [t for t in trend if run_key(t) < run_key(cur_point)]
        prev = older[-1] if older else None
        for m in REF_METRICS:
            prev_by[m] = nearest_reference(m, cur_point, older)
    return prev, trend, prev_by


# ── palette ────────────────────────────────────────────────────────────

def _hex_mix(a, b, t):
    A, B = [int(a[i:i + 2], 16) for i in (1, 3, 5)], [int(b[i:i + 2], 16) for i in (1, 3, 5)]
    return "#" + "".join("%02x" % round(x + (y - x) * t) for x, y in zip(A, B))


def palette(order):
    """{theme: {leg: hex}}: v8 colours for the six known legs, a stable cycle for any other backend;
    the +Elasticsearch variant of an unknown backend is a lighter tint of its base colour."""
    bases = sorted({l[:-len("-elasticsearch")] if l.endswith("-elasticsearch") else l for l in order}
                   - set(KNOWN_BASES))
    out = {}
    for theme in ("dark", "light"):
        pal, extra = dict(PAL_KNOWN[theme]), PAL_EXTRA[theme]
        for l in order:
            if l in pal:
                continue
            es = l.endswith("-elasticsearch")
            base = l[:-len("-elasticsearch")] if es else l
            col = extra[bases.index(base) % len(extra)]
            pal[l] = _hex_mix(col, "#ffffff", 0.3) if es else col
        out[theme] = {l: pal[l] for l in order}
    return out


# ── assembly ───────────────────────────────────────────────────────────

def _shapes_job(args):
    leg, d, allow_points = args
    md = shapes_from_md(os.path.join(d, "search-by-shape.md"))
    if md is not None:
        return leg, md, "search-by-shape.md"
    p = os.path.join(d, "search-points.json")
    if allow_points and os.path.exists(p):
        return leg, shape_stats(p), "search-points.json"
    return leg, None, None


def load_shapes(dirs, order, no_shapes):
    jobs = [(l, dirs[l], not no_shapes) for l in order]
    heavy = (not no_shapes) and len(jobs) > 1
    if heavy:
        try:
            with ProcessPoolExecutor(max_workers=min(6, len(jobs))) as ex:
                return list(ex.map(_shapes_job, jobs))
        except Exception:  # noqa: BLE001 - sandboxes without multiprocessing: go serial
            pass
    return [_shapes_job(j) for j in jobs]


def build_data(a):
    dirs = leg_dirs(a.run_dir)
    order = order_legs(dirs)
    legs = {l: load_leg(l, dirs[l]) for l in order}
    expected = [x.strip() for x in (a.expect_legs or "").split(",") if x.strip()]
    missing = [l for l in expected if l not in legs]
    for leg, rows, src in load_shapes(dirs, order, a.no_shapes):
        legs[leg]["shapes"], legs[leg]["shapes_src"] = rows, src
    first = legs[order[0]] if order else {}
    docker_cpus = a.docker_cpus or next((legs[l]["docker_cpus"] for l in order if legs[l]["docker_cpus"]), None)
    docker_ram = a.docker_ram_gb or next((legs[l]["docker_ram_gb"] for l in order if legs[l]["docker_ram_gb"]), None)
    run = str(a.run_id or first.get("run") or os.path.basename(os.path.normpath(a.run_dir or "")) or "n/a")
    for l in order:
        if not legs[l].get("run"):
            legs[l]["run"] = run
        legs[l]["checks"] = leg_checks(legs[l], docker_cpus)
    history = load_history(a.trend_dir)
    for l in order:
        pt = trend_point(l, dict(legs[l], run=run, ref=a.branch or legs[l]["ref"]))
        legs[l]["point"] = pt
        legs[l]["prev"], legs[l]["trend"], legs[l]["prev_by"] = leg_trend(l, pt, history, run, getattr(a, "trend_max", TREND_MAX))
    head = a.headline_leg if a.headline_leg in legs else (next((l for l in order if any(legs[l]["suites"].values())
                                                                 or legs[l]["bulk"]), order[0] if order else None))
    starts = sorted(legs[l]["start"] for l in order if legs[l]["start"])
    timing = load_timing(getattr(a, "timing_json", None))
    sha = (a.sha or first.get("sha") or "")[:9] or None
    extra = {"timing": timing, "timing_view": timing_view(timing)} if timing else {}
    return {**extra, "run": run, "date": starts[0] if starts else None, "sha": sha, "sha_full": a.sha or None,
            "ref": a.branch or first.get("ref"), "repo": a.repo, "docker_cpus": docker_cpus,
            "docker_ram_gb": docker_ram, "headline": head, "order": order, "missing": missing, "legs": legs,
            "has_shapes": any(legs[l].get("shapes") for l in order), "history_runs": len(history),
            "palette": palette(order), "comparison": parse_comparison(find_comparison(a.run_dir)),
            "verdict_all": verdict_all(legs, order, docker_cpus, missing, timing),
            "thresholds": {"import_target": IMPORT_TARGET, "err_pct_high": ERR_PCT_HIGH,
                           "pg_contended_ms": PG_CONTENDED_MS}}


LIGHT_TOKENS = (
    "color-scheme: light;"
    "--page:#f6f8fa; --surface:#ffffff; --surface-2:#f6f8fa;"
    "--ink:#1f2328; --ink-2:#424a53; --muted:#59636e;"
    "--grid:#d8dee4; --axis:#afb8c1; --border:#d0d7de;"
    "--accent:#0969da; --prev:#bc4c00;"
    "--band-a:#f6f8fa; --band-b:#eaeef2;"
    "--good:#1a7f37; --good-bg:#dafbe1; --warn:#7d4e00; --warn-bg:#fff8c5;"
    "--bad:#cf222e; --bad-bg:#ffebe9; --na:#59636e; --na-bg:#eaeef2;"
    "--cpu:#cf222e; --tip-bg:#1f2328; --tip-fg:#ffffff; --tip-bd:#1f2328;"
    "--btn-on:#0969da; --btn-on-fg:#ffffff; --shadow:0 0 0 1px #d0d7de;")


def render_html(data):
    tpl = read_text(os.path.join(HERE, "report_template.html"))
    if tpl is None:
        raise OSError("report_template.html not found next to report_html.py")
    # Embedded in a <script>: keep "</" and U+2028/9 from ending the block or the string.
    js = json.dumps(data, ensure_ascii=False, separators=(",", ":")).replace("</", "<\\/") \
        .replace("\u2028", "\\u2028").replace("\u2029", "\\u2029")
    tpl = tpl.replace("/*__LIGHT_RULE__*/", ':root[data-theme="light"]{%s}' % LIGHT_TOKENS)
    tpl = tpl.replace("/*__PRINT_RULE__*/", '@media print{:root,:root[data-theme="dark"]{%s}}' % LIGHT_TOKENS)
    return tpl.replace("/*__DATA__*/null", js)


def minimal_page(run_id, why):
    """A valid, self-contained page for when the report cannot be built."""
    return ("<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">"
            "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">"
            "<title>FHIR Benchmark · %s</title><style>body{background:#0d1117;color:#e6edf3;"
            "font:15px/1.5 system-ui,sans-serif;max-width:760px;margin:40px auto;padding:0 16px}"
            "code{font-family:ui-monospace,monospace}</style></head><body>"
            "<h1>FHIR Server Benchmark <code>#%s</code></h1>"
            "<p>The report could not be generated: <code>%s</code></p>"
            "<p>The raw results are still in the run artifacts.</p></body></html>\n") % (
        html.escape(str(run_id)), html.escape(str(run_id)), html.escape(str(why)[:300]))


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--run-dir", default="")
    ap.add_argument("--run-id", default="")
    ap.add_argument("--repo", default="")
    ap.add_argument("--branch", default="")
    ap.add_argument("--sha", default="")
    ap.add_argument("--out", default="fhir-bench-report.html")
    ap.add_argument("--dump-json")
    ap.add_argument("--trend-dir")
    ap.add_argument("--trend-max", type=int, default=TREND_MAX, help="most runs (including this one) charted per leg")
    ap.add_argument("--timing-json", default=None, help="run_timing.py output: waiting vs working time")
    ap.add_argument("--no-shapes", action="store_true", help="do not read the heavy search-points.json")
    ap.add_argument("--headline-leg", default=None, help="leg preselected in the tiles (default: first leg with results)")
    ap.add_argument("--docker-cpus", type=int, default=None, help="override; normally read from runner-info")
    ap.add_argument("--docker-ram-gb", type=int, default=None)
    ap.add_argument("--expect-legs", default="", help="comma list; legs listed here without artifacts are flagged")
    try:
        a, _ = ap.parse_known_args(argv)
    except SystemExit:
        a = argparse.Namespace(run_dir="", run_id="", repo="", branch="", sha="", out="fhir-bench-report.html",
                               dump_json=None, trend_dir=None, no_shapes=True, headline_leg=None,
                               docker_cpus=None, docker_ram_gb=None, expect_legs="", timing_json=None,
                               trend_max=TREND_MAX)
    text, data, size_legs = None, None, 0
    try:
        data = build_data(a)
        size_legs = len(data["order"])
        text = render_html(data)
    except Exception as e:  # noqa: BLE001 - never raise out of main
        print("report_html: %s: %s" % (type(e).__name__, e), file=sys.stderr)
        text = minimal_page(a.run_id or "n/a", "%s: %s" % (type(e).__name__, e))
    if a.dump_json:
        try:
            dump = dict(data) if data else {"run": a.run_id, "legs": {}, "order": []}
            dump.pop("timing_view", None)
            dump["legs"] = {l: {k: v for k, v in lg.items() if k not in ("shapes", "trend", "prev", "prev_by")}
                            for l, lg in dump["legs"].items()}
            with open(a.dump_json, "w", encoding="utf-8") as fh:
                json.dump(dump, fh, ensure_ascii=False, separators=(",", ":"))
        except Exception as e:  # noqa: BLE001
            print("report_html: dump failed: %s" % e, file=sys.stderr)
    try:
        with open(a.out, "w", encoding="utf-8") as fh:
            fh.write(text)
        print("%s: %d bytes, %d legs" % (a.out, len(text.encode("utf-8")), size_legs), file=sys.stderr)
    except Exception as e:  # noqa: BLE001
        print("report_html: cannot write %s: %s" % (a.out, e), file=sys.stderr)
    return 0


if __name__ == "__main__":
    try:
        rc = main()
    except BaseException as e:  # noqa: BLE001
        print("report_html: %s" % e, file=sys.stderr)
        rc = 0
    sys.exit(0 if rc is None else rc)
