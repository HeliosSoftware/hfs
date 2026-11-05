"""TEMPORARY: trend points derived from the comparison.md of runs that have no report-data.json.

Why: the HTML report charts its Trend section from the report-data.json artifact
(fhir-bench-report-data-<run id>) of earlier runs, and no run before 2026-10-27 has
one. history.py already reads those runs' comparison.md for the Markdown history; this
module turns the same file into a point of the shape report_html.trend_point() emits,
so the first HTML reports are not blank. history.py writes the result to
<data-dir>/<run id>/legacy-point.json, never to a report-data.json.

Nothing is invented. Every value comes from a table cell of the file: a cell that
reads "n/a (...)" is None, never 0; values are whole numbers in the file, so every
point says "rounded": true. The date is not in the file: the caller passes the run's
created_at date (the start of the run).

Runs in the current format (they carry the row "↳ search indexing (seconds)" or the
/ui/batch and /ui/bulk-import labels) are not read: point_doc returns None for them
rather than a partial document that would lose the import suite and show a failed
rebuild as valid. Those runs have report-data.json or will get one.

Indexing: runs written before PR #2248 deferred indexing on every leg, so every point
says "index_inline": False. On the Elasticsearch legs the file's ingest/s, ingest
seconds and index seconds (the post-load rebuild) are kept exactly as written and SHOWN,
with the note "indexing was deferred (before #2248): ingest excludes indexing, index_eff
is the post-load rebuild". They are not the quantity measured today (index = ingest,
inline), so later code must never compare them with inline-mode values; index_inline
False is the flag for that.

Remove this module by about EXPIRES: see the "Removal" section of the fhir-bench
README (this file, test_legacy_points.py, legacy_testdata/, the fallback in
history.py::fetch_data, the legacy-point.json read in report_html.load_history and
the --legacy-trend flag).
"""
import re

EXPIRES = "2026-12-15"

LEG_ES = "-elasticsearch"  # header "sqlite+ES" -> leg id "sqlite-elasticsearch"
SUITES = ("import", "crud", "search", "insert")

# old wording of a row -> the measurement it names (rule: same measurement, other words)
IMPORT_ROWS = {"import (resources/s)", "import, `POST /` transaction Bundles (resources/s)",
               "Batch/Transaction Import, `POST /` Bundles (resources/s)"}
IMPORT_P95_ROWS = {"import (per Bundle)", "import, `POST /` transaction Bundles (per Bundle)",
                   "Batch/Transaction Import, `POST /` Bundles (per Bundle)"}
BULK_ROWS = {"bulk-import (resources/s)", "bulk-import, `$bulk-submit` (resources/s)",
             "Bulk Import, `$bulk-submit` (resources/s)"}
INGEST_ROWS = {"bulk-import ingest (resources/s, seconds)", "Bulk Import: ingest (resources/s, seconds)",
               "↳ ingest (resources/s, seconds)"}
INDEX_ROWS = {"bulk-import initial index (seconds)", "Bulk Import: index rebuild (seconds)",
              "↳ search-index rebuild after Completed (seconds)"}
# labels of the CURRENT format (post-#2248): this module cannot read them
CURRENT_LABELS = ("↳ search indexing (seconds)", "Batch/Transaction Import (`/ui/batch`",
                  "Bulk Import (`/ui/bulk-import`")
PLAIN_ROWS = {"crud": "crud", "search": "search", "insert (creates/s)": "insert"}
PLAIN_P95_ROWS = {"crud": "crud", "search": "search", "insert": "insert"}

ES_DEFERRED_NOTE = "indexing was deferred (before #2248): ingest excludes indexing, index_eff is the post-load rebuild"

NUM_RE = re.compile(r"\d[\d,]*(?:\.\d+)?")


def _split(line):
    line = line.strip()
    if line.startswith("|"):
        line = line[1:]
    if line.endswith("|"):
        line = line[:-1]
    return [c.strip() for c in line.split("|")]


def _tables(text):
    """[(heading, header cells, [row cells])] of every pipe table of the file."""
    lines = text.splitlines()
    out, heading, i = [], "", 0
    while i < len(lines):
        ln = lines[i]
        if ln.startswith("#"):
            heading = ln.lower()
        if ln.strip().startswith("|") and i + 1 < len(lines) and re.match(r"^\|[\s:|-]+\|?\s*$", lines[i + 1]):
            head, rows = _split(ln), []
            i += 2
            while i < len(lines) and lines[i].strip().startswith("|"):
                rows.append(_split(lines[i]))
                i += 1
            out.append((heading, head, rows))
            continue
        i += 1
    return out


def _clean(cell):
    return cell.replace("**", "").replace("`", "").strip()


def _num(cell):
    """First number of a cell, or None (also for every "n/a ..." cell)."""
    c = _clean(cell)
    m = NUM_RE.match(c)
    return float(m.group(0).replace(",", "")) if m else None


def _whole(cell):
    v = _num(cell)
    return None if v is None else (int(v) if v == int(v) else v)


def _marks(cell):
    return "".join(ch for ch in "†‡⚠" if ch in cell)


def _leg_id(header):
    h = header.strip().strip("`")
    return h[:-len("+ES")] + LEG_ES if h.endswith("+ES") else h


def _seconds(cell):
    """Seconds of "355 s" / "1,099 s"; None for anything else (n/a, "inline (during ingest)")."""
    m = re.match(r"^([\d,]+)\s*s\b", _clean(cell))
    return float(m.group(1).replace(",", "")) if m else None


def _whole_f(v):
    return None if v is None else int(v)


def _suite_values(tables, wanted_rows, heading_word):
    """{row key: {leg id: cell text}} of the first table whose heading names heading_word."""
    for heading, head, rows in tables:
        if heading_word in heading and head and head[0] == "Suite":
            legs = [_leg_id(h) for h in head[1:]]
            out = {}
            for cells in rows:
                key = wanted_rows.get(cells[0].strip())
                if key:
                    out[key] = dict(zip(legs, cells[1:]))
            return out, legs, rows
    return {}, [], []


def point_doc(text, run_id, date, branch, partial_run=False):
    """{"source", "run", "legs": {leg: {"point": ...}}} for one old comparison.md, or None."""
    tables = _tables(text)
    for _h, head, rows in tables:
        if head and head[0] == "Suite" and any(r[0].strip().startswith(CURRENT_LABELS) for r in rows):
            return None
    sha_m = re.search(r"Commit `([0-9a-f]+)`", text)
    sha = sha_m.group(1) if sha_m else None

    thr_rows = {}
    for lbl in IMPORT_ROWS:
        thr_rows[lbl] = "import"
    for lbl in BULK_ROWS:
        thr_rows[lbl] = "bulk"
    for lbl in INGEST_ROWS:
        thr_rows[lbl] = "ingest"
    for lbl in INDEX_ROWS:
        thr_rows[lbl] = "index"
    thr_rows.update(PLAIN_ROWS)
    thr, legs, thr_all = _suite_values(tables, thr_rows, "throughput")
    p95_rows = {lbl: "import" for lbl in IMPORT_P95_ROWS}
    p95_rows.update(PLAIN_P95_ROWS)
    p95, p95_legs, _ = _suite_values(tables, p95_rows, "p95")
    if not legs:
        return None
    labels = [r[0].strip() for r in thr_all]
    reindexed = any(l.startswith("reindex, ") for l in labels)

    health = {}
    for heading, head, rows in tables:
        if head and head[0] == "Leg" and "Runner" in head:
            for cells in rows:
                health[cells[0].strip("` ")] = dict(zip(head, cells))
    bulk_tbl = {}
    for heading, head, rows in tables:
        if head and head[0] == "Leg" and "Ingest" in head and "Total" in head:
            for cells in rows:
                bulk_tbl[cells[0].strip("` ")] = dict(zip(head, cells))

    out = {}
    for leg in legs:
        suites, markers = {}, {}
        for s in SUITES:
            rc, pc = thr.get(s, {}).get(leg), p95.get(s, {}).get(leg)
            rps, p = (None if rc is None else _num(rc)), (None if pc is None else _num(pc))
            if rps is None and p is None:
                continue
            suites[s] = {"rps": rps, "p95": p}
            markers[s] = _marks(rc or "") or _marks(pc or "")

        bulk = None
        brow = bulk_tbl.get(leg, {})
        rc = thr.get("bulk", {}).get(leg)
        rps = None if rc is None else _num(rc)
        ingest, ingest_rps, index, total = None, None, None, _seconds(brow.get("Total", "n/a"))
        m = re.match(r"^([\d,]+)\s*s\s*·\s*([\d,]+)/s", _clean(brow.get("Ingest", "n/a")))
        if m:
            ingest, ingest_rps = (float(g.replace(",", "")) for g in m.groups())
        else:
            m = re.match(r"^([\d,]+)\s*\(([\d,]+)\s*s\)", _clean(thr.get("ingest", {}).get(leg, "n/a")))
            if m:
                ingest_rps, ingest = (float(g.replace(",", "")) for g in m.groups())
        index = _seconds(brow["Index"]) if "Index" in brow else _seconds(thr.get("index", {}).get(leg, "n/a"))
        if any(v is not None for v in (rps, total, ingest, ingest_rps, index)):
            bulk = {"rps": _whole_f(rps), "total": _whole_f(total), "ingest_rps": _whole_f(ingest_rps),
                    "ingest": _whole_f(ingest), "index_eff": _whole_f(index)}
            markers["bulk"] = _marks(rc or "")
        if not suites and not bulk:
            continue

        h = health.get(leg, {})
        ok = re.match(r"^([\d,]+)/[\d,]+", _clean(h.get("Import", "")))
        pg = re.search(r"pg ([\d.]+) ms/call", h.get("Host at crud start", ""))
        load = re.search(r"load ([\d.]+)", h.get("Host at crud start", ""))
        runner = _clean(h.get("Runner", "")).split(" · ")[0]
        point = {"run": str(run_id), "date": date, "sha": sha, "branch": branch,
                 "runner": runner if runner and not runner.startswith("n/a") else None,
                 "suites": suites, "bulk": bulk,
                 "bundles_ok": int(ok.group(1).replace(",", "")) if ok else None,
                 "pg_ms": float(pg.group(1)) if pg else None,
                 "crud_load": float(load.group(1)) if load else None,
                 "markers": markers, "source": "comparison.md", "rounded": True,
                 "index_inline": False, "partial_run": bool(partial_run)}
        notes = []
        if reindexed:
            notes.append("insert and bulk may be skewed by a preceding $reindex")
        if leg.endswith(LEG_ES) and bulk:
            notes.append(ES_DEFERRED_NOTE)
        if notes:
            point["notes"] = notes
        out[leg] = {"point": point}
    if not out:
        return None
    return {"source": "comparison.md", "run": str(run_id), "legs": out}
