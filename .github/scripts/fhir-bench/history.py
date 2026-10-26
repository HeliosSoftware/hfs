#!/usr/bin/env python3
"""Progress over time of the FHIR benchmark, for the job summary.

Reads the comparison.md artifact of earlier successful runs (via the gh CLI),
parses its Throughput / p95 tables generically and writes a collapsible
Markdown block with per-row tables and Mermaid xychart-beta line charts.
Best effort: every failure is swallowed and the exit code is always 0.
"""
import argparse
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
OLD_REBUILD = "↳ search-index rebuild after Completed (seconds)"
# older wordings of renamed rows -> canonical (new) wording, so they join into one row
LABEL_ALIASES = {
    "Batch/Transaction Import, `POST /` Bundles (resources/s)": NEW_BATCH,
    "Bulk Import, `$bulk-submit` (resources/s)": NEW_BULK,
    "Bulk Import: ingest (resources/s, seconds)": NEW_INGEST,
    "↳ ingest (resources/s, seconds)": NEW_INGEST,
    # a REAL deferred rebuild: a different measurement from NEW_INDEX (whose ES
    # cells are "= ingest"), so it stays its own row and never joins NEW_INDEX
    "Bulk Import: index rebuild (seconds)": OLD_REBUILD,
}


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


def parse_tables(md):
    """Return {'throughput': T, 'p95': T}; T = {'legs': [...], 'rows': [(label, [vals])]}."""
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
            i += 2  # skip separator
            while i < len(lines) and lines[i].strip().startswith("|"):
                cells = _split_row(lines[i])
                label = canon_label(cells[0])
                if any(label == r[0] for r in rows):
                    i += 1
                    continue
                vals = [parse_cell(c) for c in cells[1:1 + len(legs)]]
                vals += [None] * (len(legs) - len(vals))
                rows.append((label, vals))
                i += 1
            tables.append((heading, {"legs": legs, "rows": rows}))
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
            "url": run.get("html_url") or ""}


def fetch_runs(repo, workflow, branch, exclude_id, max_runs):
    runs = []
    try:
        runs = [_norm(x) for x in _list_runs(repo, workflow, branch, max_runs + 5)
                if int(x["id"]) != int(exclude_id)][:max_runs]
    except Exception as e:  # noqa: BLE001
        print(f"history: branch run list failed: {e}", file=sys.stderr)
    if branch != "main":
        try:
            for x in _list_runs(repo, workflow, "main", 5):
                if int(x["id"]) != int(exclude_id):
                    n = _norm(x)
                    if all(n["id"] != r["id"] for r in runs):
                        runs.append(n)
                    break
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


def fetch_data(repo, run, data_dir):
    """Best effort: report-data.json artifact of a run into data_dir/<run id>."""
    try:
        _gh(["run", "download", str(run["id"]), "-R", repo,
             "-n", f"fhir-bench-report-data-{run['id']}",
             "-D", os.path.join(data_dir, str(run["id"]))])
    except Exception as e:  # noqa: BLE001
        print(f"history: data download failed for {run.get('id')}: {e}", file=sys.stderr)


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
    out.append("Throughput per run (change vs the previous run in brackets; higher is better "
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
        for _, tabs in entries:
            t = tabs["throughput"]
            row = dict(t["rows"]).get(lab)
            for lg in legs:
                v = None
                if row is not None and lg in t["legs"]:
                    v = row[t["legs"].index(lg)]
                data[lg].append(v)
        if all(v is None for vs in data.values() for v in vs):
            continue
        sec = [f"**{lab}**\n", "| Leg | " + " | ".join(xlabels) + " |",
               "|---|" + "---:|" * len(xlabels)]
        for lg in legs:
            vs = data[lg]
            if all(v is None for v in vs):
                continue
            cells = []
            for i, v in enumerate(vs):
                c = fmt_num(v)
                d = pct(vs[i - 1], v) if i else ""
                cells.append(f"{c} ({d})" if d else c)
            sec.append(f"| `{lg}` | " + " | ".join(cells) + " |")
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
                     fetch_data if args.data_dir else None)
        with open(args.out, "w", encoding="utf-8") as f:
            f.write(text)
    except Exception as e:  # noqa: BLE001
        print(f"history: failed: {e}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
