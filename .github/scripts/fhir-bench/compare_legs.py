#!/usr/bin/env python3
#
# Cross-leg comparison for the FHIR Benchmark run summary: three metric tables
# (throughput, p95 latency, errors) with suites as rows and legs as columns,
# a legend, and a leg-health table, so a reader does not have to scroll six
# stacked per-leg summaries. The import row is not k6's request figures: its
# throughput is resources/s and its p95 is per Bundle (ROW_LABELS, IMPORT_NOTE,
# _import_row_values). The `insert` row is HFS's own suite (k6/insert.js, not
# upstream): its throughput row is labelled creates/s and it never carries the
# corpus marker `†` (see INSERT_NOTE). Display only: it never decides anything,
# it always exits 0 and every problem goes out as a `::warning` instead.
#
# Called from: the `compare` job's "Write comparison summary" step
# (fhir-benchmark.yml), via:
#   python3 "$GITHUB_WORKSPACE/.github/scripts/fhir-bench/compare_legs.py" \
#       bench-artifacts > comparison.md
# and the step then appends comparison.md to $GITHUB_STEP_SUMMARY.
#
# Required environment: none. Every value below is optional, and with none
# set the script still renders (that is how it runs locally). The workflow
# step passes them through env: and never interpolates them into a script.
#   BENCH_RUN_ID      github.run_id: header, and rejects a leg whose
#                     runner-info.txt says it was written by another run
#                     (the leg checkout uses `clean: false` and only "Run
#                     benchmark suites" wipes bench-results/, so a leg that
#                     dies earlier uploads whatever an older run left behind)
#   BENCH_MATRIX      setup's matrix output, {"include":[{"backend":...}]}:
#                     the legs that were supposed to run, so one that never
#                     uploaded still gets a column
#   BENCH_IN_TESTS    inputs.tests: which suites were requested
#   BENCH_REF_NAME    github.ref_name (header)
#   BENCH_SHA         github.sha (header, first 7 characters)
#   DOWNLOAD_OUTCOME  steps.download.outcome (banner when not "success")
#
# Input: argv[1] (default `bench-artifacts`), the directory the download step
# filled. Four layouts are understood, because download-artifact writes
# straight into `path` when exactly one artifact matches:
#   A. skip-decompress, 2+ artifacts: ROOT/<artifact-name>/<raw zip>
#   B. skip-decompress, one artifact:  ROOT/<raw zip>
#   C. extracted, 2+ artifacts (also `gh run download`):
#                                      ROOT/<artifact-name>/<leg>/<files>
#   D. extracted, one artifact:        ROOT/<leg>/<files>
# Each artifact holds exactly `<backend>/...` (the leg writes into
# bench-results/<backend>), so the leg name comes from that inner directory,
# never from the artifact or directory name. Only the small k6 --summary-export
# files (<suite>.json) and the leg's *.txt files are read, through one reader
# (_read) that caps their size.
#
# It never opens, reads, decompresses or parses a *-points.json file: every
# leg's artifact carries search-points.json, the k6 point stream (60-660 MB
# each in run 36546493701, several GB in a bad run), and _read refuses such a
# name outright. That is also why the workflow downloads with skip-decompress
# and this script reads members straight out of the zips.
#
# Output: stdout, Markdown (UTF-8). stderr, workflow commands only
# (`::notice`, `::warning`). Exit code: always 0.
#
# Python 3.8-compatible, standard library only.
import json
import math
import os
import re
import sys
import zipfile
import zlib

# resolve-matrix.sh order: primary backends, then their ES composites.
KNOWN_LEGS = [
    "sqlite",
    "postgres",
    "mongodb",
    "sqlite-elasticsearch",
    "postgres-elasticsearch",
    "mongodb-elasticsearch",
]
# "Run benchmark suites" CANONICAL: the order the suites run in.
CANONICAL_SUITES = ["prewarm", "import", "crud", "search", "insert"]
MAX_JSON_BYTES = 8 * 1024 * 1024
MAX_TXT_BYTES = 1024 * 1024
IMPORT_TARGET = 1000
ERR_PCT_HIGH = 1.0
PG_CONTENDED_MS = 3.0

LEG_RE = re.compile(r"[a-z0-9][a-z0-9-]*")
HINT_RE = re.compile(r"fhir-benchmark-(.+)-([0-9]+)")
SUITE_RE = re.compile(r"[A-Za-z0-9_-]+")
NOT_A_ZIP = (".json", ".txt", ".log", ".md", ".sql", ".csv")
READ_ERRORS = (zipfile.BadZipFile, zlib.error, OSError, EOFError, ValueError)
TITLE = "Leg comparison"

LEGEND = (
    "`†` crud/search ran on an incomplete corpus (import below 1000/1000 bundles or not run "
    "before it, or the ES index not drained) · `‡` more than 1% errors or failed checks in "
    "that suite · `⚠` contended host (postgres crud above 3 ms/call) or a backend container "
    "died · **bold** = best in its row, shown only when no leg in that row carries a marker · "
    "n/a = no result (reason in brackets)"
)
FOOTER = (
    "_Per-leg detail (leg configuration, p50/p99 and checks, the result-size cross-check, "
    "search latency by query shape) is in each `Benchmark (<leg>)` job summary; raw files are "
    "in the `fhir-benchmark-<leg>-<run id>` artifacts._"
)
# A row whose value in a metric table is not that table's default measure gets
# its own label: (suite, table kind) -> row label. The Throughput table is
# requests/s and the p95 table per HTTP request unless a row names otherwise.
ROW_LABELS = {
    ("import", "rps"): "import (resources/s)",
    ("import", "p95"): "import (per Bundle)",
    # every insert.js request is one create
    ("insert", "rps"): "insert (creates/s)",
}
IMPORT_NOTE = (
    "Import row: **resources/s** = entries of the transaction Bundles that committed (k6 "
    "`bundle_size`) per second of the import scenario's own clock, which starts after k6 "
    "setup() has loaded the two seed Bundles (that time is in Leg health) — the unit "
    "upstream's report uses. Its p95 is **per Bundle**: one k6 iteration, the corpus fetch "
    "plus the transaction POST. k6's `http_reqs` is not used for import: it counts a corpus "
    "GET beside every Bundle POST. Bundles/s is in Leg health."
)
INSERT_NOTE = (
    "`insert` is HFS's own suite (`.github/scripts/fhir-bench/k6/insert.js`, load shape "
    "in its header): single-resource `POST Patient` and `POST Observation` creates, each "
    "Observation referencing that Patient, so every request is one create and its throughput "
    "row is creates/s. It runs last, after search and after the result-size snapshot, so its "
    "resources are in neither, and it never carries `†` because it does not read the corpus."
)


# ── Sources and the one reader ──────────────────────────────────────

class TooLarge(Exception):
    pass


class DirSource(object):
    """One leg's results as an extracted directory."""

    def __init__(self, path, leg):
        self.path = path
        self.leg = leg
        self.where = path
        self._names = None

    def names(self):
        if self._names is None:
            found = set()
            with os.scandir(self.path) as it:
                for e in it:
                    if e.is_file(follow_symlinks=False):
                        found.add(e.name)
            self._names = found
        return self._names

    def size(self, name):
        return os.path.getsize(os.path.join(self.path, name))

    def read_bytes(self, name):
        with open(os.path.join(self.path, name), "rb") as fh:
            return fh.read()


class ZipSource(object):
    """One leg's results as the `<leg>/` members of an open raw artifact zip."""

    def __init__(self, zf, leg, path=""):
        self.zf = zf
        self.leg = leg
        self.where = "%s!%s" % (path or "zip", leg)
        self._names = None

    def names(self):
        if self._names is None:
            prefix = self.leg + "/"
            found = set()
            for n in self.zf.namelist():
                if n.startswith(prefix):
                    rest = n[len(prefix):]
                    if rest and "/" not in rest:
                        found.add(rest)
            self._names = found
        return self._names

    def size(self, name):
        return self.zf.getinfo(self.leg + "/" + name).file_size

    def read_bytes(self, name):
        return self.zf.read(self.leg + "/" + name)


def _read(src, name, cap):
    """The only reader. Bytes, or None when `name` is not in the source."""
    if name.endswith("-points.json"):
        raise ValueError("refusing to read " + name)
    if name not in src.names():
        return None
    if src.size(name) > cap:
        raise TooLarge(name)
    return src.read_bytes(name)


def _text(src, name):
    """Small text file as str; missing, too large or unreadable is None."""
    try:
        raw = _read(src, name, MAX_TXT_BYTES)
    except Exception:
        return None
    if raw is None:
        return None
    return raw.decode("utf-8", errors="replace")


# ── Small helpers ───────────────────────────────────────────────────

def md_cell(s):
    return str(s).replace("|", "\\|").replace("\r", " ").replace("\n", " ")


def _oneline(s):
    return str(s).replace("`", "'").replace("\r", " ").replace("\n", " ").strip()


def _num(v):
    if isinstance(v, bool) or not isinstance(v, (int, float)):
        return None
    if not math.isfinite(v):
        return None
    return v


def _field(metrics, metric, key):
    m = metrics.get(metric)
    return m.get(key) if isinstance(m, dict) else None


def _count(v):
    n = _num(v)
    return int(n) if n is not None else 0


def _int(v):
    try:
        return int(float(v))
    except (TypeError, ValueError, OverflowError):
        return None


def _seconds(v):
    """A positive, finite number of seconds from a text value, or None."""
    try:
        f = float(v)
    except (TypeError, ValueError, OverflowError):
        return None
    return f if math.isfinite(f) and f > 0 else None


def _n(v):
    return "{:,}".format(v) if isinstance(v, int) else "?"


def _kv(text, sep):
    kv = {}
    if text:
        for line in text.splitlines():
            if sep in line:
                k, v = line.split(sep, 1)
                kv[k.strip()] = v.strip()
    return kv


def label(leg):
    suffix = "-elasticsearch"
    if leg.endswith(suffix):
        return md_cell(leg[:-len(suffix)] + "+ES")
    return md_cell(leg)


def _escape(msg):
    return msg.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")


def _annotation(level, msg):
    return "::%s title=%s::%s" % (level, TITLE, _escape(msg))


def _order(legs):
    known = [l for l in KNOWN_LEGS if l in legs]
    return known + sorted(l for l in legs if l not in KNOWN_LEGS)


def _header(env):
    ref = _oneline(env.get("BENCH_REF_NAME") or "")
    if ref:
        return "## FHIR Benchmark — `%s` (leg comparison)" % ref
    return "## FHIR Benchmark — leg comparison"


# ── Discovery ───────────────────────────────────────────────────────

def _scan(path):
    """Direct children as (name, path, is_dir, is_file); never follows links."""
    out = []
    try:
        with os.scandir(path) as it:
            for e in it:
                try:
                    out.append((e.name, e.path, e.is_dir(follow_symlinks=False),
                                e.is_file(follow_symlinks=False)))
                except OSError:
                    pass
    except OSError:
        pass
    out.sort()
    return out


def _zip_magic(path):
    try:
        with open(path, "rb") as fh:
            return fh.read(4) == b"PK\x03\x04"
    except OSError:
        return False


def _open_zip(path, parent, sources, unreadable, zips, warn):
    hint = None
    if parent:
        m = HINT_RE.fullmatch(parent)
        if m and LEG_RE.fullmatch(m.group(1)):
            hint = m.group(1)
    try:
        if not zipfile.is_zipfile(path):
            # A zip cut short by an interrupted download has no end-of-archive
            # record, so is_zipfile() says no; its first bytes still give it away.
            if not _zip_magic(path):
                return
            raise zipfile.BadZipFile("truncated zip")
        zf = zipfile.ZipFile(path)
    except READ_ERRORS as e:
        unreadable.append((path, hint))
        warn("unreadable artifact %s (%s: %s)" % (path, type(e).__name__, e))
        return
    zips.append(zf)
    legs = set()
    for n in zf.namelist():
        if n.startswith("/") or ".." in n.split("/"):
            continue
        if "/" in n:
            legs.add(n.split("/", 1)[0])
    for leg in sorted(legs):
        if LEG_RE.fullmatch(leg):
            sources.append(ZipSource(zf, leg, path))
        else:
            warn("ignoring unexpected leg name %r in %s" % (leg, path))


def _discover(root, warn, zips):
    """Every readable source, plus the artifacts that could not be opened."""
    sources = []
    unreadable = []
    if not os.path.isdir(root):
        return sources, unreadable
    listings = {}
    candidates = []
    for name, path, is_dir, is_file in _scan(root):
        candidates.append((name, path, is_dir, is_file, None))
        if is_dir:
            listings[path] = _scan(path)
            for n2, p2, d2, f2 in listings[path]:
                candidates.append((n2, p2, d2, f2, name))
    for name, path, is_dir, is_file, parent in candidates:
        if is_file:
            if not name.lower().endswith(NOT_A_ZIP):
                _open_zip(path, parent, sources, unreadable, zips, warn)
        elif is_dir:
            children = listings.get(path)
            if children is None:
                children = _scan(path)
            holds_results = False
            for cn, _cp, _cd, cf in children:
                low = cn.lower()
                if cf and (low == "runner-info.txt" or low.endswith(".json")
                           or low.endswith(".log")):
                    holds_results = True
                    break
            if holds_results:
                if LEG_RE.fullmatch(name):
                    sources.append(DirSource(path, name))
                else:
                    warn("ignoring unexpected leg directory %r" % name)
    return sources, unreadable


def _group(sources, unreadable, run_id, warn):
    """leg -> {fresh: [sources], stale_run: id or None, unreadable: [paths]}."""
    found = {}

    def slot(leg):
        return found.setdefault(leg, {"fresh": [], "stale_run": None, "unreadable": []})

    for src in sorted(sources, key=lambda s: s.where):
        s = slot(src.leg)
        gr = _kv(_text(src, "runner-info.txt"), ":").get("github_run")
        if run_id and gr and gr != run_id:
            if s["stale_run"] is None:
                s["stale_run"] = gr
        else:
            s["fresh"].append(src)
    for leg, s in found.items():
        if len(s["fresh"]) > 1:
            warn("duplicate results for leg %s; using %s" % (leg, s["fresh"][0].where))
    for path, hint in unreadable:
        if hint:
            slot(hint)["unreadable"].append(path)
    return found


def _matrix_legs(raw, warn):
    if not raw or not raw.strip():
        return None
    try:
        doc = json.loads(raw)
    except Exception:
        return None
    include = doc.get("include") if isinstance(doc, dict) else None
    if not isinstance(include, list):
        return None
    legs = []
    for item in include:
        b = item.get("backend") if isinstance(item, dict) else None
        if not isinstance(b, str):
            continue
        if LEG_RE.fullmatch(b):
            if b not in legs:
                legs.append(b)
        else:
            warn("ignoring unexpected leg name %r in the matrix" % b)
    return legs or None


def _requested(raw, warn):
    text = (raw or "").strip()
    if not text or text == "all":
        return list(CANONICAL_SUITES)
    out = []
    for part in text.split(","):
        part = part.replace(" ", "")
        if not part:
            continue
        if not SUITE_RE.fullmatch(part):
            warn("ignoring unexpected suite name %r in tests" % part)
        elif part not in out:
            out.append(part)
    return out


# ── Loading one leg ─────────────────────────────────────────────────

def _load_suite(src, suite, names):
    jn = suite + ".json"
    if suite.endswith("-points") or jn not in names:
        has_log = (suite + ".log") in names and not suite.endswith("-points")
        return {"reason": "no k6 summary" if has_log else "not run"}
    try:
        raw = _read(src, jn, MAX_JSON_BYTES)
    except TooLarge:
        return {"reason": "too large"}
    except Exception:
        return {"reason": "unparsable"}
    if raw is None:
        return {"reason": "no k6 summary"}
    try:
        doc = json.loads(raw.decode("utf-8"))
    except Exception:
        return {"reason": "unparsable"}
    if not isinstance(doc, dict) or not isinstance(doc.get("metrics"), dict):
        return {"reason": "unparsable"}
    m = doc["metrics"]
    checks = m.get("checks") if isinstance(m.get("checks"), dict) else {}
    # http_req_failed is a k6 Rate metric: the fraction is `value`, not `rate`.
    err = _num(_field(m, "http_req_failed", "value"))
    return {
        "reason": None,
        "rps": _num(_field(m, "http_reqs", "rate")),
        "p95": _num(_field(m, "http_req_duration", "p(95)")),
        "iter_p95": _num(_field(m, "iteration_duration", "p(95)")),
        "err": None if err is None else err * 100.0,
        "passes": _count(checks.get("passes")),
        "fails": _count(checks.get("fails")),
    }


def _load_counts(text):
    rows = []
    for line in text.splitlines()[1:]:
        parts = line.split("|")
        if len(parts) == 4:
            rows.append(tuple(p.strip() for p in parts))

    def total(t):
        return int(t) if re.fullmatch(r"[0-9]+", t) else t

    def first(query):
        for q, t, _h, _s in rows:
            if q == query:
                return total(t)
        return None

    obs = first("Observation?_summary=count")
    prefixes = ("Observation?code-value-quantity=", "Observation?combo-code-value-quantity=")
    fallback = isinstance(obs, int) and any(
        q.startswith(prefixes) and isinstance(total(t), int) and total(t) == obs
        for q, t, _h, _s in rows)
    return {
        "patient": first("Patient?_summary=count"),
        "observation": obs,
        "non200": sum(1 for _q, _t, h, _s in rows if h != "200"),
        "fallback": fallback,
    }


def _import_row_values(d):
    """The import row in ROW_LABELS' units, replacing k6's request figures:
    resources/s (committed entries over the import scenario's own seconds, from
    import-completeness.txt) and per-Bundle p95 (k6 iteration_duration)."""
    s = d["suites"].get("import")
    if not s or s["reason"] is not None:
        return
    ic = d["ic"] or {}
    entries, scenario = ic.get("entries"), ic.get("scenario")
    if isinstance(entries, int) and scenario is not None:
        s["rps"] = entries / scenario
    else:
        s["rps"] = None
        s["na"] = {"rps": "no import timing"}
    s["p95"] = s.get("iter_p95")


def _load_leg(leg, src, rows):
    names = src.names()
    d = {"leg": leg, "status": "ok", "suites": {}}
    for suite in rows:
        d["suites"][suite] = _load_suite(src, suite, names)
    if not any(s["reason"] is None for s in d["suites"].values()):
        d["status"] = "no-summaries"

    ri = _kv(_text(src, "runner-info.txt"), ":")
    d["ri"] = ri

    ict = _text(src, "import-completeness.txt")
    d["ic"] = None
    if ict is not None:
        ic = _kv(ict, "=")
        d["ic"] = {"bundles_ok": _int(ic.get("bundles_ok")), "entries": _int(ic.get("entries")),
                   "wall": _int(ic.get("wall_seconds")), "note": ic.get("note", ""),
                   "scenario": _seconds(ic.get("scenario_seconds")),
                   "setup": _seconds(ic.get("setup_seconds"))}
    _import_row_values(d)

    dr = _text(src, "es-drain.txt") if leg.endswith("-elasticsearch") else None
    d["drain"] = _kv(dr, "=") if dr is not None else None

    ht = _text(src, "host-contention.txt")
    m = re.search(r"suite=crud phase=start host_loadavg=(\S+).*?host_containers=(\S+)", ht) if ht else None
    d["host"] = (m.group(1), m.group(2)) if m else None

    d["pg_ms"] = None
    if leg == "postgres":
        pt = _text(src, "crud-pgstat.txt")
        pm = re.search(r"^\s*([\d.]+)\s*\|\s*[\d.]+\s*\|\s*(\d+)\s*$", pt, re.M) if pt else None
        if pm:
            try:
                calls = int(pm.group(2))
                d["pg_ms"] = float(pm.group(1)) * 1000.0 / calls if calls else None
            except ValueError:
                d["pg_ms"] = None
    d["contended"] = d["pg_ms"] is not None and d["pg_ms"] > PG_CONTENDED_MS

    sc = _text(src, "search-counts.txt")
    d["counts"] = _load_counts(sc) if sc is not None else None

    rt = _text(src, "crud-residue.txt")
    d["residue"] = 0
    if rt is not None:
        try:
            d["residue"] = sum(int(v) for v in _kv(rt, "=").values())
        except ValueError:
            d["residue"] = 0

    cs = _text(src, "containers-state.txt")
    d["died"] = False
    if cs is not None:
        d["died"] = any(
            re.search(r"OOMKilled=true|Status=(exited|dead)|inspect-failed gone", line)
            for line in cs.splitlines() if not line.startswith("hfs-bench-tgz-"))
    return d


# ── Rendering ───────────────────────────────────────────────────────

def _markers(d, suite, rows):
    s = d["suites"].get(suite)
    if not s or s["reason"] is not None:
        return ""
    out = ""
    # insert writes new resources and reads none of the corpus: no `†`.
    if suite not in ("prewarm", "import", "insert"):
        incomplete = "import" not in rows or rows.index(suite) < rows.index("import")
        if not incomplete:
            ok = d["ic"]["bundles_ok"] if d["ic"] else None
            incomplete = ok is None or ok < IMPORT_TARGET
        if not incomplete and suite == "search" and d["leg"].endswith("-elasticsearch"):
            incomplete = d["drain"] is None or d["drain"].get("status") != "drained"
        if incomplete:
            out += "†"
    if (s["err"] is not None and s["err"] > ERR_PCT_HIGH) or s["fails"] > 0:
        out += "‡"
    if d["died"] or (d["leg"] == "postgres" and suite == "crud" and d["contended"]):
        out += "⚠"
    return out


def _fmt_rps(v):
    return "{:,.0f}".format(v)


def _fmt_p95(v):
    return "{:.1f}".format(v)


def _fmt_err(e):
    s = "{:.1f}%".format(e)
    return "**%s**" % s if e > 1 else s


_LEG_NA = {"no-artifact": "no artifact", "stale": "stale", "unreadable": "unreadable artifact"}


def _crowned(loaded, suite, key, pick, fmt, rows, nocrown):
    cand = [d for d in loaded
            if d["suites"].get(suite, {"reason": "x"})["reason"] is None
            and d["suites"][suite][key] is not None]
    if nocrown or len(cand) < 2:
        return set()
    if any(_markers(d, suite, rows) for d in cand):
        return set()
    best = fmt(pick(d["suites"][suite][key] for d in cand))
    return set(d["leg"] for d in cand if fmt(d["suites"][suite][key]) == best)


def _metric_cell(d, suite, kind, crown, rows):
    if d["status"] in _LEG_NA:
        return "n/a (%s)" % _LEG_NA[d["status"]]
    s = d["suites"][suite]
    if s["reason"] is not None:
        return "n/a (%s)" % md_cell(s["reason"])
    if kind == "err":
        if s["err"] is None:
            text = "n/a (no data)"
        else:
            text = _fmt_err(s["err"])
        if s["fails"] > 0:
            text += " · **{:,} ✗**".format(s["fails"])
        return text
    value = s[kind]
    if value is None:
        return "n/a (%s)" % md_cell(s.get("na", {}).get(kind, "no data"))
    text = _fmt_rps(value) if kind == "rps" else _fmt_p95(value)
    if d["leg"] in crown:
        text = "**%s**" % text
    mk = _markers(d, suite, rows)
    return text + " " + mk if mk else text


def _metric_table(title, kind, datas, loaded, rows, nocrown):
    lines = ["### " + title, ""]
    lines.append("| Suite | " + " | ".join(label(d["leg"]) for d in datas) + " |")
    lines.append("|---|" + "---:|" * len(datas))
    for suite in rows:
        crown = set()
        if kind == "rps":
            crown = _crowned(loaded, suite, "rps", max, _fmt_rps, rows, nocrown)
        elif kind == "p95":
            crown = _crowned(loaded, suite, "p95", min, _fmt_p95, rows, nocrown)
        cells = [_metric_cell(d, suite, kind, crown, rows) for d in datas]
        lines.append("| " + md_cell(ROW_LABELS.get((suite, kind), suite)) + " | "
                     + " | ".join(cells) + " |")
    lines.append("")
    return lines


def _runner_cell(d):
    ri = d["ri"]
    name = ri.get("runner_name")
    cpus, ram = ri.get("runner_cpus"), ri.get("runner_ram")
    if not name and not (cpus and ram):
        return "n/a"
    text = md_cell(name or "?")
    if cpus and ram:
        text += " · %s CPU / %s" % (md_cell(cpus), md_cell(ram))
    return text


def _import_cell(d, rows):
    ic = d["ic"]
    if ic is None:
        return "not run †" if "import" not in rows else "n/a †"
    if ic["note"] == "no-k6-summary":
        return "0/%d · k6 wrote no summary †" % IMPORT_TARGET
    ok = ic["bundles_ok"]
    text = "%s/%d · %s" % (_n(ok), IMPORT_TARGET, _import_timing(ic))
    if ok is None or ok < IMPORT_TARGET:
        text += " †"
    return text


def _import_timing(ic):
    """'<entries> entries in <scenario> s + <setup> s setup · <n> Bundles/s', or, without
    the scenario clock (an older artifact), '<entries> entries in <wall> s'."""
    scenario = ic.get("scenario")
    if scenario is None:
        return "%s entries in %s s" % (_n(ic["entries"]), _n(ic["wall"]))
    text = "%s entries in %s s" % (_n(ic["entries"]), "{:,.0f}".format(scenario))
    if ic.get("setup") is not None:
        text += " + {:,.0f} s setup".format(ic["setup"])
    if isinstance(ic["bundles_ok"], int):
        text += " · {:.2f} Bundles/s".format(ic["bundles_ok"] / scenario)
    return text


def _total(v):
    if v is None:
        return "?"
    return "{:,}".format(v) if isinstance(v, int) else md_cell(v)


def _sizes_cell(d):
    c = d["counts"]
    if c is None:
        return "n/a"
    text = "Patient %s · Observation %s" % (_total(c["patient"]), _total(c["observation"]))
    if c["non200"] > 0:
        text += " · **%d non-200**" % c["non200"]
    if c["fallback"]:
        text += " · **composite = Observation total**"
    if d["residue"] > 0:
        text += " · {:,} crud leftovers".format(d["residue"])
    return text


def _drain_cell(d):
    if not d["leg"].endswith("-elasticsearch"):
        return "—"
    dr = d["drain"]
    if dr is None:
        return "n/a †"
    status = dr.get("status") or "?"
    reason = dr.get("reason") or ""
    text = "drained" if status == "drained" else (
        "%s (%s)" % (md_cell(status), md_cell(reason)) if reason else md_cell(status))
    if "missing" in dr:
        missing = _int(dr["missing"])
        text += " · {:,} missing".format(missing) if missing is not None \
            else " · missing %s" % md_cell(dr["missing"])
    reindex = _int(dr.get("needs_reindex_after"))
    if reindex is not None and reindex > 0:
        text += " · {:,} to reindex".format(reindex)
    if status != "drained":
        text += " †"
    return text


def _host_cell(d):
    h = d["host"]
    if h is None:
        return "n/a"
    text = "load %s · %s containers" % (md_cell(h[0]), md_cell(h[1]))
    if d["leg"] == "postgres" and d["pg_ms"] is not None:
        text += " · pg {:.2f} ms/call".format(d["pg_ms"])
        if d["contended"]:
            text += " ⚠ contended"
    return text


def _health_row(d, rows):
    leg = "`%s`" % md_cell(d["leg"])
    st = d["status"]
    if st in _LEG_NA:
        if st == "stale":
            results = "n/a: stale (run %s)" % md_cell(d["stale_run"])
        else:
            results = "n/a: " + _LEG_NA[st]
        return "| " + " | ".join([leg, results] + ["n/a"] * 5) + " |"
    if st == "ok":
        results = "⚠ container died" if d["died"] else "✓"
    else:
        results = "no k6 summaries"
    cells = [leg, results, _runner_cell(d), _import_cell(d, rows), _sizes_cell(d),
             _drain_cell(d), _host_cell(d)]
    return "| " + " | ".join(cells) + " |"


def _notes(loaded):
    notes = []
    names = sorted(set(d["ri"]["runner_name"] for d in loaded if d["ri"].get("runner_name")))
    pairs = []
    groups = {}
    for d in loaded:
        cpus, ram = d["ri"].get("runner_cpus"), d["ri"].get("runner_ram")
        if cpus and ram:
            if (cpus, ram) not in groups:
                groups[(cpus, ram)] = []
                pairs.append((cpus, ram))
            groups[(cpus, ram)].append(d["leg"])
    if names:
        joined = ", ".join(md_cell(n) for n in names)
        if len(pairs) == 1:
            notes.append("Runners: %s (%s CPU / %s each)." % (joined, md_cell(pairs[0][0]),
                                                             md_cell(pairs[0][1])))
        elif not pairs:
            notes.append("Runners: %s." % joined)
        else:
            parts = ["%s CPU / %s: %s" % (md_cell(c), md_cell(r), ", ".join(groups[(c, r)]))
                     for c, r in pairs]
            notes.append("⚠ Runner hardware differs across legs (%s), so no best is marked in "
                         "any table." % "; ".join(parts))
    sized = [d["counts"] for d in loaded
             if d["counts"] is not None and isinstance(d["counts"]["observation"], int)]
    if len(sized) >= 2:
        if len(set((c["patient"], c["observation"]) for c in sized)) == 1:
            notes.append("All legs with results report the same totals (Patient %s · "
                         "Observation %s)." % (_total(sized[0]["patient"]),
                                               _total(sized[0]["observation"])))
        else:
            obs = [c["observation"] for c in sized]
            notes.append("Result sizes differ across legs (Observation {:,} – {:,}), so crud "
                         "and search did not run on the same corpus.".format(min(obs), max(obs)))
    return notes


def _render(root, env, warn, zips):
    run_id = (env.get("BENCH_RUN_ID") or "").strip()
    sources, unreadable = _discover(root, warn, zips)
    found = _group(sources, unreadable, run_id, warn)

    matrix = _matrix_legs(env.get("BENCH_MATRIX"), warn)
    if matrix is not None:
        fresh = [leg for leg, s in found.items() if s["fresh"]]
        legs = _order(list(matrix) + [l for l in fresh if l not in matrix])
    else:
        legs = _order(list(found.keys()))

    requested = _requested(env.get("BENCH_IN_TESTS"), warn)
    chosen = {}
    for leg in legs:
        if leg in found and found[leg]["fresh"]:
            chosen[leg] = found[leg]["fresh"][0]
    extras = set()
    for src in chosen.values():
        try:
            names = src.names()
        except Exception:
            continue
        for n in names:
            if n.endswith(".json"):
                s = n[:-len(".json")]
                if (s + ".log") in names and s not in requested and not s.endswith("-points"):
                    extras.add(s)
    rows = requested + sorted(extras)

    datas = []
    for leg in legs:
        s = found.get(leg)
        if leg in chosen:
            try:
                d = _load_leg(leg, chosen[leg], rows)
            except Exception as e:
                warn("could not read results for leg %s (%s: %s)" % (leg, type(e).__name__, e))
                d = {"leg": leg, "status": "unreadable"}
        elif s and s["stale_run"] is not None:
            d = {"leg": leg, "status": "stale", "stale_run": s["stale_run"]}
        elif s and s["unreadable"]:
            d = {"leg": leg, "status": "unreadable"}
        else:
            d = {"leg": leg, "status": "no-artifact"}
        datas.append(d)
    loaded = [d for d in datas if d["status"] in ("ok", "no-summaries")]
    k = sum(1 for d in datas if d["status"] == "ok")

    pairs = set((d["ri"].get("runner_cpus"), d["ri"].get("runner_ram")) for d in loaded
                if d["ri"].get("runner_cpus") and d["ri"].get("runner_ram"))
    nocrown = len(pairs) > 1

    suites_text = "all" if (env.get("BENCH_IN_TESTS") or "").strip() in ("", "all") \
        else ",".join(requested)
    parts = []
    sha = _oneline(env.get("BENCH_SHA") or "")
    if sha:
        parts.append("Commit `%s`" % sha[:7])
    if run_id:
        parts.append("run %s" % _oneline(run_id))
    parts.append("suites: %s" % md_cell(suites_text))
    parts.append("%d of %d legs have results" % (k, len(datas)))

    lines = [_header(env), "", " · ".join(parts), ""]
    outcome = (env.get("DOWNLOAD_OUTCOME") or "").strip()
    if outcome and outcome != "success":
        lines += ["> ⚠ The artifact download step ended with `%s`; legs shown as n/a may have "
                  "results that could not be fetched." % _oneline(outcome), ""]
        warn("the artifact download step ended with %s" % outcome)
    if not datas:
        lines.append("_No leg results were found._")
        return lines, k, len(datas)

    lines += _metric_table("Throughput (requests/s unless the row names its unit)", "rps",
                           datas, loaded, rows, nocrown)
    lines += _metric_table("p95 latency (ms)", "p95", datas, loaded, rows, nocrown)
    lines += _metric_table("Errors (Err% · failed checks)", "err", datas, loaded, rows, nocrown)
    lines += [LEGEND, "", IMPORT_NOTE, "", "### Leg health", "",
              "| Leg | Results | Runner | Import | Result sizes | ES drain | Host at crud start |",
              "|---|---|---|---|---|---|---|"]
    lines += [_health_row(d, rows) for d in datas]
    for note in _notes(loaded) + ([INSERT_NOTE] if "insert" in rows else []) + [FOOTER]:
        lines += ["", note]
    return lines, k, len(datas)


def render(root, env):
    """Pure: (markdown, workflow-command lines) for the downloaded artifacts."""
    annotations = []
    seen = set()

    def warn(msg):
        if msg not in seen:
            seen.add(msg)
            annotations.append(_annotation("warning", msg))

    zips = []
    try:
        lines, k, n = _render(root, env, warn, zips)
    finally:
        for zf in zips:
            try:
                zf.close()
            except Exception:
                pass
    annotations.append(_annotation(
        "notice", '%d of %d legs have results; the cross-leg tables are in the "Compare legs" '
                  'job summary.' % (k, n)))
    return "\n".join(lines) + "\n", annotations


def main(argv=None):
    for stream in (sys.stdout, sys.stderr):
        if hasattr(stream, "reconfigure"):
            try:
                # newline="\n": no CRLF translation on a Windows console or pipe.
                stream.reconfigure(encoding="utf-8", newline="\n")
            except Exception:
                pass
    argv = sys.argv if argv is None else argv
    root = argv[1] if len(argv) > 1 else "bench-artifacts"
    env = {}
    for key in ("BENCH_RUN_ID", "BENCH_MATRIX", "BENCH_IN_TESTS", "BENCH_REF_NAME", "BENCH_SHA",
              "DOWNLOAD_OUTCOME"):
        env[key] = os.environ.get(key, "")
    try:
        markdown, annotations = render(root, env)
    except Exception as e:
        msg = ("%s: %s" % (type(e).__name__, e)).replace("\r", " ").replace("\n", " ")[:200]
        markdown = "%s\n\n_Leg comparison failed: %s. The per-leg summaries are unaffected._\n" % (
            _header(env), msg)
        annotations = [_annotation("warning", "leg comparison failed: " + msg)]
    try:
        sys.stdout.write(markdown)
        sys.stdout.flush()
        for line in annotations:
            sys.stderr.write(line + "\n")
        sys.stderr.flush()
    except Exception:
        pass
    return 0


if __name__ == "__main__":
    sys.exit(main())
