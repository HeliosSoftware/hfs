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
                label = cells[0]
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
             f"?branch={branch}&status=success&per_page={per_page}"])
    if r.returncode != 0:
        return []
    return json.loads(r.stdout).get("workflow_runs", [])


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


def _fill(vals):
    """The series as plotted: only a leg with a value in every run gets a line.
    A gap is never filled with a neighbouring value (a renamed row or a leg that
    did not run would otherwise draw a flat line); the tables show the gap as a dash."""
    if not vals or any(v is None for v in vals):
        return None
    return list(vals)


def _clean(s):
    return re.sub(r'["\[\]\n|`]', "", s).strip()


def render_chart(label, legs, series, xlabels):
    lines = ["```mermaid", "xychart-beta",
             f'    title "{_clean(label)[:70]}"',
             "    x-axis [" + ", ".join(f'"{x}"' for x in xlabels) + "]",
             f'    y-axis "{_clean(label)[:20]}" 0']
    # y-axis with only a label keeps min auto; drop the stray 0 form
    lines[-1] = '    y-axis "value"'
    used = []
    for leg, vals in zip(legs, series):
        f = _fill(vals)
        if f is None:
            continue
        used.append(leg)
        lines.append("    line [" + ", ".join(f"{v:.1f}" for v in f) + "]")
    lines.append("```")
    if not used:
        return ""
    return "\n".join(lines) + "\n\nLines in order: " + ", ".join(f"`{u}`" for u in used) + "\n"


def build(args, fetch_runs_fn, fetch_comparison_fn, current_text=None):
    try:
        runs = fetch_runs_fn(args.repo, args.workflow, args.branch,
                             args.run_id, args.max_runs)
    except Exception as e:  # noqa: BLE001
        print(f"history: listing failed: {e}", file=sys.stderr)
        runs = []
    entries = []  # (run dict, tables)
    for run in sorted(runs, key=lambda r: (r.get("date", ""), r.get("id", 0))):
        try:
            text = fetch_comparison_fn(args.repo, run, args.artifact_name)
            if not text:
                continue
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
        out.append(f"**{lab}**\n")
        out.append("| Leg | " + " | ".join(xlabels) + " |")
        out.append("|---|" + "---:|" * len(xlabels))
        for lg in legs:
            vs = data[lg]
            if all(v is None for v in vs):
                continue
            cells = []
            for i, v in enumerate(vs):
                c = fmt_num(v)
                d = pct(vs[i - 1], v) if i else ""
                cells.append(f"{c} ({d})" if d else c)
            out.append(f"| `{lg}` | " + " | ".join(cells) + " |")
        out.append("")
        if _is_chart_row(lab) and len(charts) < MAX_CHARTS:
            c = render_chart(lab, legs, [data[g] for g in legs], xlabels)
            if c:
                charts.append(c)

    if charts:
        out.append("Charts (one line per leg, runs oldest to newest):\n")
        out.extend(c + "\n" for c in charts)
    out.append("</details>\n")
    text = "\n".join(out)
    if len(text.encode("utf-8")) > MAX_BYTES:
        text = text.encode("utf-8")[:MAX_BYTES - 200].decode("utf-8", "ignore")
        text += "\n\n_(truncated)_\n</details>\n"
    return text


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
                     lambda repo, run, name: fetch_comparison(repo, run, name), cur)
        with open(args.out, "w", encoding="utf-8") as f:
            f.write(text)
    except Exception as e:  # noqa: BLE001
        print(f"history: failed: {e}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
