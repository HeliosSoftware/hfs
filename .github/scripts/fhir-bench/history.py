#!/usr/bin/env python3
"""Progress over time of the FHIR benchmark, for the job summary.

Reads the comparison.md artifact of earlier successful runs (via the gh CLI),
parses its Throughput / p95 tables generically and writes a collapsible
Markdown block with per-row tables and Mermaid xychart-beta line charts.
Best effort: every failure is swallowed and the exit code is always 0.
"""
import argparse
import datetime
import glob
import json
import os
import re
import subprocess
import sys
import tempfile

MAX_CHARTS = 6
MAX_BYTES = 200 * 1024
NUM_RE = re.compile(r"^-?\d[\d,]*(?:\.\d+)?")


NEW_BATCH = "Batch/Transaction Import (`/ui/batch`: `POST /` Bundles) (resources/s)"
NEW_BULK = "Bulk Import (`/ui/bulk-import`: `$bulk-submit`, until searchable) (resources/s)"
NEW_INGEST = "↳ ingest, until Completed (resources/s, seconds)"
NEW_INDEX = "↳ search indexing (seconds)"
NEW_BATCH_P95 = "Batch/Transaction Import (`/ui/batch`: `POST /` Bundles) (per Bundle)"
NEW_BULK_P95 = "Bulk Import (`/ui/bulk-import`: `$bulk-submit`)"
NEW_BATCH_ERR = "Batch/Transaction Import (`/ui/batch`: `POST /` Bundles)"
# older wordings of renamed rows -> canonical (new) wording, so they join into one row.
# The `reindex, `$reindex` Encounter` rows and the "Indexing ($reindex of Encounter)"
# table are a different measurement and are deliberately NOT aliased.
LABEL_ALIASES = {
    "import (resources/s)": NEW_BATCH,
    "import, `POST /` transaction Bundles (resources/s)": NEW_BATCH,
    "Batch/Transaction Import, `POST /` Bundles (resources/s)": NEW_BATCH,
    "bulk-import (resources/s)": NEW_BULK,
    "bulk-import, `$bulk-submit` (resources/s)": NEW_BULK,
    "Bulk Import, `$bulk-submit` (resources/s)": NEW_BULK,
    "bulk-import ingest (resources/s, seconds)": NEW_INGEST,
    "Bulk Import: ingest (resources/s, seconds)": NEW_INGEST,
    "↳ ingest (resources/s, seconds)": NEW_INGEST,
    "bulk-import initial index (seconds)": NEW_INDEX,
    "Bulk Import: index rebuild (seconds)": NEW_INDEX,
    "↳ search-index rebuild after Completed (seconds)": NEW_INDEX,
    # p95 and error tables
    "import (per Bundle)": NEW_BATCH_P95,
    "import, `POST /` transaction Bundles (per Bundle)": NEW_BATCH_P95,
    "Batch/Transaction Import, `POST /` Bundles (per Bundle)": NEW_BATCH_P95,
    "bulk-import, `$bulk-submit`": NEW_BULK_P95,
    "Bulk Import, `$bulk-submit`": NEW_BULK_P95,
    "import": NEW_BATCH_ERR,
    "import, `POST /` transaction Bundles": NEW_BATCH_ERR,
    "Batch/Transaction Import, `POST /` Bundles": NEW_BATCH_ERR,
}
# Raw labels of the DEFERRED-indexing era (before #2248). On the Elasticsearch legs
# their ingest/index numbers are not the quantity today's rows carry (today
# index = ingest): they stay in the series, marked DEFERRED_MARK, and the change of
# a cell is only computed against runs with the same indexing mode (see index_mode).
DEFERRED_INDEX_LABELS = {
    "bulk-import initial index (seconds)",
    "Bulk Import: index rebuild (seconds)",
    "↳ search-index rebuild after Completed (seconds)",
}
MODE_BLOCKS = {NEW_INGEST, NEW_INDEX}  # the blocks that carry the indexing mode
DEFERRED_MARK = "ᵈ"


def is_es_leg(name):
    n = name.lower()
    return "elasticsearch" in n or "+es" in n.replace(" ", "")


def canon_label(label):
    return LABEL_ALIASES.get(label.strip(), label)


def parse_cell(cell):
    """Numeric value of a table cell, or None. Strips bold/markers/commas."""
    c = cell.strip().replace("**", "").replace("`", "").strip()
    m = NUM_RE.match(c)
    if not m:
        return None
    try:
        return float(m.group(0).replace(",", ""))
    except ValueError:
        return None


def _split_row(line):
    line = line.strip()
    if line.startswith("|"):
        line = line[1:]
    if line.endswith("|"):
        line = line[:-1]
    return [c.strip() for c in line.split("|")]


def cell_flags(cell):
    """Flags of a table cell: failed rebuild, '(= ingest)' inline marker."""
    c = cell.lower()
    return {"failed": "failed" in c, "inline": "(= ingest)" in c}


def index_mode(raw_labels):
    """Indexing mode of the Elasticsearch legs of one table, from its raw row labels:
    'inline' (current `↳ search indexing` row), 'deferred' (old index rows), else 'unknown'."""
    if NEW_INDEX in raw_labels:
        return "inline"
    if raw_labels & DEFERRED_INDEX_LABELS:
        return "deferred"
    return "unknown"


def leg_mode(table, leg):
    """Indexing mode of a leg in a table: non-ES legs are always 'deferred' (before and
    after #2248, so they stay comparable); ES legs follow the table."""
    return table.get("index_mode", "unknown") if is_es_leg(leg) else "deferred"


def parse_tables(md):
    """Return {'throughput': T, 'p95': T}; T = {'legs': [...], 'rows': [(label, [vals])],
    'cells': {label: [flags per leg]}, 'index_mode': 'inline'|'deferred'|'unknown'}."""
    lines = md.splitlines()
    tables = []
    i = 0
    heading = ""
    while i < len(lines):
        ln = lines[i]
        if ln.startswith("#"):
            heading = ln.lower()
        if re.match(r"^\|\s*Suite\s*\|", ln):
            legs = _split_row(ln)[1:]
            rows = []
            cells_of = {}
            raw_labels = set()
            i += 2  # skip separator
            while i < len(lines) and lines[i].strip().startswith("|"):
                cells = _split_row(lines[i])
                raw_labels.add(cells[0].strip())
                label = canon_label(cells[0])
                if any(label == r[0] for r in rows):
                    i += 1
                    continue
                vals = [parse_cell(c) for c in cells[1:1 + len(legs)]]
                vals += [None] * (len(legs) - len(vals))
                rows.append((label, vals))
                fl = [cell_flags(c) for c in cells[1:1 + len(legs)]]
                cells_of[label] = fl + [cell_flags("")] * (len(legs) - len(fl))
                i += 1
            tables.append((heading, {"legs": legs, "rows": rows, "cells": cells_of,
                                     "index_mode": index_mode(raw_labels)}))
            continue
        i += 1
    out = {}
    for h, t in tables:
        if "throughput" in h and "throughput" not in out:
            out["throughput"] = t
        elif "p95" in h and "p95" not in out:
            out["p95"] = t
    if "throughput" not in out and tables:
        out["throughput"] = tables[0][1]
    return out


# ---- fetching (gh CLI) ----

def _gh(args):
    return subprocess.run(["gh"] + args, capture_output=True, text=True, timeout=180)


def _list_runs(repo, workflow, branch, per_page):
    r = _gh(["api", f"repos/{repo}/actions/workflows/{workflow}/runs"
             f"?branch={branch}&status=completed&per_page={per_page}"])
    if r.returncode != 0:
        return []
    # a run with one failed matrix leg is "failure" yet may still have uploaded
    # the comparison artifact; fetch_comparison skips it silently when it did not
    return [x for x in json.loads(r.stdout).get("workflow_runs", [])
            if x.get("conclusion") in ("success", "failure")]


def _norm(run):
    return {"id": int(run["id"]), "number": run.get("run_number"),
            "date": (run.get("created_at") or "")[:10],
            "branch": run.get("head_branch") or "",
            "conclusion": run.get("conclusion") or "",
            "url": run.get("html_url") or ""}


LIST_CAP = 100  # most runs examined per branch while looking for ones with artifacts


def has_artifact(repo, run_id):
    """True when the run still has a benchmark artifact. Unknown counts as True."""
    try:
        r = _gh(["api", f"repos/{repo}/actions/runs/{run_id}/artifacts?per_page=100"])
        if r.returncode != 0:
            return True
        arts = json.loads(r.stdout).get("artifacts", [])
        return any(not a.get("expired") and str(a.get("name", "")).startswith(
                       ("fhir-bench-comparison-", "fhir-bench-report-data-")) for a in arts)
    except Exception:  # noqa: BLE001
        return True


def _with_artifacts(repo, runs, limit, exclude_id):
    """First `limit` runs (newest first) that still have an artifact: a run whose
    artifacts expired or were never uploaded must not use up a slot."""
    out = []
    for x in runs:
        if len(out) >= limit:
            break
        if int(x["id"]) == int(exclude_id):
            continue
        if has_artifact(repo, x["id"]):
            out.append(_norm(x))
    return out


def fetch_runs(repo, workflow, branch, exclude_id, max_runs):
    runs = []
    try:
        runs = _with_artifacts(repo, _list_runs(repo, workflow, branch, LIST_CAP),
                               max_runs, exclude_id)
    except Exception as e:  # noqa: BLE001
        print(f"history: branch run list failed: {e}", file=sys.stderr)
    if branch != "main":
        try:
            for n in _with_artifacts(repo, _list_runs(repo, workflow, "main", 20), 1, exclude_id):
                if all(n["id"] != r["id"] for r in runs):
                    runs.append(n)
        except Exception as e:  # noqa: BLE001
            print(f"history: main run list failed: {e}", file=sys.stderr)
    return runs


def fetch_comparison(repo, run, artifact_name):
    """comparison.md text of a run's artifact (<prefix>-<run_id>), or None."""
    try:
        with tempfile.TemporaryDirectory() as d:
            r = _gh(["run", "download", str(run["id"]), "-R", repo,
                     "-n", f"{artifact_name}-{run['id']}", "-D", d])
            if r.returncode != 0:
                return None
            found = glob.glob(os.path.join(d, "**", "comparison.md"), recursive=True)
            if not found:
                found = glob.glob(os.path.join(d, "**", "*.md"), recursive=True)
            if not found:
                return None
            with open(found[0], encoding="utf-8") as f:
                return f.read()
    except Exception as e:  # noqa: BLE001
        print(f"history: download failed for {run.get('id')}: {e}", file=sys.stderr)
        return None


_EXPIRY_WARNED = False


def fetch_data(repo, run, data_dir, artifact_name="fhir-bench-comparison", legacy=True):
    """Best effort: report-data.json artifact of a run into data_dir/<run id>.

    TEMPORARY fallback (see legacy_points.py): a run from before that artifact
    existed gets <run id>/legacy-point.json derived from its comparison.md."""
    dest = os.path.join(data_dir, str(run["id"]))
    ok = False
    try:
        r = _gh(["run", "download", str(run["id"]), "-R", repo,
                 "-n", f"fhir-bench-report-data-{run['id']}", "-D", dest])
        ok = r.returncode == 0
    except Exception as e:  # noqa: BLE001
        print(f"history: data download failed for {run.get('id')}: {e}", file=sys.stderr)
    if ok or not legacy:
        return
    try:
        import legacy_points  # TEMPORARY: delete with the module
        if datetime.date.today().isoformat() > legacy_points.EXPIRES:
            global _EXPIRY_WARNED
            if not _EXPIRY_WARNED:  # once per history.py run, not once per legacy run
                _EXPIRY_WARNED = True
                print(f"::warning::history: --legacy-trend expired on {legacy_points.EXPIRES}; "
                      "remove legacy_points.py (see README, Removal)")
            return
        text = fetch_comparison(repo, run, artifact_name)
        if not text:
            return
        doc = legacy_points.point_doc(text, str(run["id"]), run.get("date") or "",
                                      run.get("branch") or "",
                                      partial_run=run.get("conclusion") == "failure")
        if doc:
            os.makedirs(dest, exist_ok=True)
            with open(os.path.join(dest, "legacy-point.json"), "w", encoding="utf-8") as f:
                json.dump(doc, f)
    except Exception as e:  # noqa: BLE001
        print(f"history: legacy point failed for {run.get('id')}: {e}", file=sys.stderr)


def suites_of(text):
    """The `suites:` text of a comparison.md header, or None when not parseable."""
    m = re.search(r"^.*\bsuites:\s*([^·\n]+?)\s*(?:·|$)", text or "", re.M)
    return m.group(1).strip() if m else None


# ---- rendering ----

def fmt_num(v):
    if v is None:
        return "–"
    return f"{v:,.0f}" if abs(v) >= 100 else f"{v:,.1f}"


def pct(prev, cur):
    if prev is None or cur is None or prev == 0:
        return ""
    p = (cur - prev) / abs(prev) * 100
    return f"{p:+.0f}%"


def reference_index(series, i, mode_block):
    """Index of the nearest earlier entry of series[i] with a value, or None. series is
    [(value, mode, failed)]; in the ingest/index blocks the reference must also have the
    same indexing mode and not be a failed rebuild."""
    _v, mode, _f = series[i]
    for j in range(i - 1, -1, -1):
        rv, rmode, rfailed = series[j]
        if rv is None:
            continue
        if mode_block and (rmode != mode or rfailed):
            continue
        return j
    return None


def _is_chart_row(label):
    l = label.lower()
    return not (l.startswith("prewarm") or l.startswith("↳") or "seconds)" in l)


def _clean(s):
    return re.sub(r'["\[\]\n|`]', "", s).strip()


def headline_leg(legs, data):
    """First leg with a value in every plotted run (never invent a point)."""
    for lg in legs:
        vs = data.get(lg) or []
        if len(vs) >= 2 and all(v is not None for v in vs):
            return lg
    return None


def y_unit(label):
    """Axis unit from the row label, e.g. '(resources/s)'; plain rows are requests/s."""
    l = label.lower()
    for u in ("resources/s", "creates/s", "rows/s", "req/s", "seconds", "ms"):
        if u in l:
            return "requests/s" if u == "req/s" else u
    return "requests/s"


def render_chart(label, leg, vals, xlabels):
    """Single-series trend chart; the title names the row and the leg."""
    if len(vals) < 2 or any(v is None for v in vals):
        return ""
    return "\n".join([
        "```mermaid", "xychart-beta",
        f'    title "{_clean(label)[:80]} - {_clean(leg)}"',
        "    x-axis [" + ", ".join(f'"{_clean(x)}"' for x in xlabels) + "]",
        f'    y-axis "{_clean(y_unit(label))}"',
        "    line [" + ", ".join(f"{v:.1f}" for v in vals) + "]",
        "```"]) + "\n"


LEGEND_CHANGE = ('_Change is against the nearest earlier run of the same leg with a value '
                 '("vs #N" when it is not the previous column); no change shown = no earlier '
                 'run to compare with; – = no data._')
LEGEND_MODE = ('_Change in these rows is only computed against runs with the same indexing '
               'mode; ᵈ = indexing deferred (runs before #2248: ingest excludes indexing and '
               'the index row is the post-load rebuild); today\'s Elasticsearch legs index '
               'inline during ingest. For Elasticsearch compare "Bulk Import … until '
               'searchable (resources/s)", which covers load plus indexing in both modes._')


def build(args, fetch_runs_fn, fetch_comparison_fn, current_text=None, fetch_data_fn=None):
    try:
        runs = fetch_runs_fn(args.repo, args.workflow, args.branch,
                             args.run_id, args.max_runs)
    except Exception as e:  # noqa: BLE001
        print(f"history: listing failed: {e}", file=sys.stderr)
        runs = []
    data_dir = getattr(args, "data_dir", None)
    if data_dir and fetch_data_fn:
        for run in runs:
            try:
                fetch_data_fn(args.repo, run, data_dir)
            except Exception:  # noqa: BLE001
                pass
    cur_suites = suites_of(current_text)
    entries = []  # (run dict, tables)
    for run in sorted(runs, key=lambda r: (r.get("date", ""), r.get("id", 0))):
        try:
            text = fetch_comparison_fn(args.repo, run, args.artifact_name)
            if not text:
                continue
            run_suites = suites_of(text)
            if (cur_suites and run_suites and run_suites != cur_suites
                    and run.get("branch") != "main"):
                continue  # a different suite selection is not comparable
            tabs = parse_tables(text)
            if "throughput" in tabs and tabs["throughput"]["rows"]:
                entries.append((run, tabs))
        except Exception as e:  # noqa: BLE001
            print(f"history: run {run.get('id')} skipped: {e}", file=sys.stderr)
    if current_text:
        try:
            tabs = parse_tables(current_text)
            if "throughput" in tabs and tabs["throughput"]["rows"]:
                entries.append(({"id": int(args.run_id), "number": None, "date": "",
                                 "branch": args.branch, "current": True,
                                 "url": f"https://github.com/{args.repo}/actions/runs/{args.run_id}"},
                                tabs))
        except Exception as e:  # noqa: BLE001
            print(f"history: current parse failed: {e}", file=sys.stderr)
    if len(entries) < 2:
        return "_Progress over time: fewer than 2 runs with a comparison artifact available yet._\n"

    def xl(run):
        return "this run" if run.get("current") else f"#{run.get('number') or run['id']}"

    xlabels = [xl(r) for r, _ in entries]
    out = [f"<details><summary>Progress over the last {len(entries)} runs</summary>\n"]
    out.append("Runs, oldest to newest: " + " · ".join(
        f"[{xl(r)}]({r['url']}) ({r['branch']}{', ' + r['date'] if r.get('date') else ''})"
        for r, _ in entries) + "\n")
    out.append("Throughput per run (change vs the nearest earlier run of the same leg with a value, in brackets; higher is better "
               "for resources/s and creates/s, lower for seconds rows).\n")

    # row labels in order of first appearance, from the newest table backwards
    labels, seen = [], set()
    for _, tabs in reversed(entries):
        for lab, _v in tabs["throughput"]["rows"]:
            if lab not in seen:
                seen.add(lab)
                labels.append(lab)
    order = {}
    for _, tabs in entries:
        for i, (lab, _v) in enumerate(tabs["throughput"]["rows"]):
            order.setdefault(lab, i)
    labels.sort(key=lambda l: order[l])

    charts = []
    for lab in labels:
        legs = []
        for _, tabs in entries:
            for lg in tabs["throughput"]["legs"]:
                if lg not in legs:
                    legs.append(lg)
        data = {lg: [] for lg in legs}
        metas = {lg: [] for lg in legs}  # (indexing mode, failed rebuild) per run
        for _, tabs in entries:
            t = tabs["throughput"]
            row = dict(t["rows"]).get(lab)
            for lg in legs:
                v, failed = None, False
                if row is not None and lg in t["legs"]:
                    k = t["legs"].index(lg)
                    v = row[k]
                    failed = bool(t.get("cells", {}).get(lab, [{}] * (k + 1))[k].get("failed"))
                data[lg].append(v)
                metas[lg].append((leg_mode(t, lg), failed))
        if all(v is None for vs in data.values() for v in vs):
            continue
        sec = [f"**{lab}**\n", "| Leg | " + " | ".join(xlabels) + " |",
               "|---|" + "---:|" * len(xlabels)]
        mode_block = lab in MODE_BLOCKS
        es_data = False
        for lg in legs:
            vs = data[lg]
            if all(v is None for v in vs):
                continue
            series = [(v,) + meta for v, meta in zip(vs, metas[lg])]
            cells = []
            for i, v in enumerate(vs):
                c = fmt_num(v)
                if v is not None and mode_block and is_es_leg(lg):
                    es_data = True
                    if series[i][1] == "deferred":
                        c += DEFERRED_MARK
                # a failed rebuild is the time until it failed, not a result: flag it, no change
                if v is not None and mode_block and series[i][2]:
                    cells.append(c + " (failed)")
                    continue
                ref = reference_index(series, i, mode_block) if v is not None else None
                d = pct(vs[ref], v) if ref is not None else ""
                if d:
                    c += f" ({d})" if ref == i - 1 else f" ({d} vs {xlabels[ref]})"
                cells.append(c)
            sec.append(f"| `{lg}` | " + " | ".join(cells) + " |")
        sec.append("\n" + LEGEND_CHANGE)
        if es_data:
            sec.append("\n" + LEGEND_MODE)
        out.append("\n".join(sec) + "\n")
        if _is_chart_row(lab) and len(charts) < MAX_CHARTS:
            lg = headline_leg(legs, data)
            c = render_chart(lab, lg, data[lg], xlabels) if lg else ""
            if c:
                charts.append(c)

    if charts:
        out.append("Trend of the headline leg per row (runs oldest to newest; "
                   "the other legs are in the tables above):\n")
        out.extend(charts)
    return _finish(out)


def _finish(sections, max_bytes=None):
    """Join atomic sections, cutting at a section boundary when over the limit,
    then close any open fence and the <details> block."""
    limit = (max_bytes or MAX_BYTES) - 400
    kept, size, cut = [], 0, False
    for sec in sections:
        n = len(sec.encode("utf-8")) + 1
        if size + n > limit and kept:
            cut = True
            break
        kept.append(sec)
        size += n
    text = "\n".join(kept)
    if text.count("```") % 2:
        text += "\n```"
    if cut:
        text += "\n\n_(output cut here: the rest of the history was left out to stay within the size limit)_"
    return text + "\n</details>\n"


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("--repo", required=True)
    ap.add_argument("--branch", required=True)
    ap.add_argument("--run-id", required=True)
    ap.add_argument("--current", required=True)
    ap.add_argument("--artifact-name", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--max-runs", type=int, default=5)
    ap.add_argument("--workflow", default="fhir-benchmark.yml")
    ap.add_argument("--data-dir", default=None)
    # TEMPORARY: derive trend points from comparison.md of runs without report-data
    ap.add_argument("--legacy-trend", action=argparse.BooleanOptionalAction, default=True)
    try:
        args = ap.parse_args(argv)
    except SystemExit:
        return 0
    try:
        try:
            with open(args.current, encoding="utf-8") as f:
                cur = f.read()
        except OSError:
            cur = None
        text = build(args, lambda *a: fetch_runs(*a),
                     lambda repo, run, name: fetch_comparison(repo, run, name), cur,
                     (lambda repo, run, d: fetch_data(repo, run, d, args.artifact_name,
                                                      args.legacy_trend))
                     if args.data_dir else None)
        with open(args.out, "w", encoding="utf-8") as f:
            f.write(text)
    except Exception as e:  # noqa: BLE001
        print(f"history: failed: {e}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
