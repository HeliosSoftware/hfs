#!/usr/bin/env python3
"""Run time section for the FHIR Server Benchmark report.

Splits the wall-clock of a workflow run into time WAITING for resources
(runner queue, capacity gate, quiet-host gate, dispatch) and time WORKING, so a
queued run is not mistaken for a slow one. Always exits 0.

  run_timing.py --repo OWNER/REPO --run-id ID --run-dir DIR \
                --out run-time.md --dump-json run-timing.json
"""
import argparse
import glob
import json
import os
import subprocess
import sys
from datetime import datetime, timezone


def parse_ts(s):
    return int(datetime.strptime(s, "%Y-%m-%dT%H:%M:%SZ")
               .replace(tzinfo=timezone.utc).timestamp())


def iso(t):
    return datetime.fromtimestamp(t, timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def humanize(s):
    s = int(round(s))
    if s < 60:
        return f"{s} s"
    m, sec = divmod(s, 60)
    if m < 60:
        return f"{m} min {sec} s" if sec else f"{m} min"
    h, m = divmod(m, 60)
    return f"{h} h {m} min" if m else f"{h} h"


def default_fetcher(kind, repo, run_id):
    """kind 'run' -> dict, 'jobs' -> list of job dicts."""
    base = f"repos/{repo}/actions/runs/{run_id}"
    if kind == "run":
        out = subprocess.run(["gh", "api", base], capture_output=True,
                             text=True, check=True, timeout=60).stdout
        return json.loads(out)
    out = subprocess.run(
        ["gh", "api", "--paginate", base + "/jobs?per_page=100", "--jq", ".jobs[]"],
        capture_output=True, text=True, check=True, timeout=120).stdout
    return [json.loads(line) for line in out.splitlines() if line.strip()]


def read_runner_info(run_dir, leg):
    pats = [os.path.join(run_dir, f"fhir-benchmark-{leg}-*", leg, "runner-info.txt"),
            os.path.join(run_dir, leg, "runner-info.txt")]
    for p in pats:
        for f in sorted(glob.glob(p)):
            info = {}
            try:
                with open(f, encoding="utf-8", errors="replace") as fh:
                    for line in fh:
                        if ":" in line:
                            k, v = line.split(":", 1)
                            info[k.strip()] = v.strip()
            except OSError:
                continue
            return info
    return {}


def _int(v):
    try:
        return max(0, int(float(v)))
    except (TypeError, ValueError):
        return 0


def union(intervals):
    out = []
    for a, b in sorted(i for i in intervals if i[1] > i[0]):
        if out and a <= out[-1][1]:
            out[-1][1] = max(out[-1][1], b)
        else:
            out.append([a, b])
    return out


def covered(a, b, merged):
    return sum(max(0, min(b, y) - max(a, x)) for x, y in merged)


def kind_of(name):
    n = name.lower()
    if name.startswith("Benchmark ("):
        return "leg"
    if n.startswith("resolve"):
        return "setup"
    if n.startswith("build"):
        return "build"
    if n.startswith("compare"):
        return "compare"
    return "other"


def compute(run, jobs, run_dir, now):
    run_created = parse_ts(run["created_at"])
    rows = []
    for j in jobs:
        created = parse_ts(j["created_at"])
        if j.get("started_at"):
            started = parse_ts(j["started_at"])
        else:
            started = now
        running = not j.get("completed_at")
        completed = None if running else parse_ts(j["completed_at"])
        end = now if running else completed
        end = max(end, started)
        name = j["name"]
        kind = kind_of(name)
        leg = name[len("Benchmark ("):].rstrip(")") if kind == "leg" else None
        rows.append(dict(name=name, kind=kind, leg=leg, created=created,
                         started=started, completed=completed, end=end,
                         running=running))
    rows.sort(key=lambda r: (r["created"], r["started"]))

    for r in rows:
        info = read_runner_info(run_dir, r["leg"]) if r["leg"] else {}
        r["cap"] = _int(info.get("capacity_wait_s"))
        r["host"] = _int(info.get("host_wait_s"))
        r["queued"] = max(0, r["started"] - r["created"])
        run_s = r["end"] - r["started"]
        # the gates happen inside the job; never take more than it ran
        r["cap"] = min(r["cap"], run_s)
        r["host"] = min(r["host"], run_s - r["cap"])
        others = union([(o["started"], o["end"]) for o in rows
                        if o["kind"] == "leg" and o is not r])
        r["behind"] = (min(r["queued"], covered(r["created"], r["started"], others))
                       if r["kind"] == "leg" else 0)
        r["waiting"] = r["queued"] - r["behind"] + r["cap"] + r["host"]
        r["working"] = run_s - r["cap"] - r["host"]

    dispatch = 0
    if rows:
        dispatch = max(0, rows[0]["created"] - run_created)
        rows[0]["waiting"] += dispatch
        end_all = max(r["end"] for r in rows)
    else:
        end_all = now
    total = max(0, end_all - run_created)
    busy = sum(b - a for a, b in union([(r["started"], r["end"]) for r in rows]))
    gates = sum(r["cap"] + r["host"] for r in rows)
    working = max(0, min(total, busy - gates))
    waiting = total - working
    return dict(run_id=str(run.get("id", "")), total_s=total, waiting_s=waiting,
                working_s=working, rows=rows, dispatch_s=dispatch)


def to_json(rep):
    jobs = []
    for r in rep["rows"]:
        jobs.append({
            "name": r["name"], "kind": r["kind"], "leg": r["leg"],
            "queued_s": r["queued"], "behind_legs_s": r["behind"],
            "capacity_wait_s": r["cap"], "host_wait_s": r["host"],
            "waiting_s": r["waiting"], "working_s": r["working"],
            "created": iso(r["created"]), "started": iso(r["started"]),
            "completed": iso(r["completed"]) if r["completed"] is not None else None,
            "running": r["running"],
        })
    return {"run_id": rep["run_id"], "total_s": rep["total_s"],
            "waiting_s": rep["waiting_s"], "working_s": rep["working_s"],
            "jobs": jobs}


def to_markdown(rep):
    lines = ["", "### Run time", "",
             f"**Total {humanize(rep['total_s'])}** · waiting for resources "
             f"**{humanize(rep['waiting_s'])}** · working **{humanize(rep['working_s'])}**",
             "",
             "| Stage | Waiting for resources | Behind earlier legs (by design) | Working |",
             "|---|---:|---:|---:|"]
    for i, r in enumerate(rep["rows"]):
        stage = f"`{r['leg']}`" if r["leg"] else r["name"]
        if r["running"]:
            stage += " (running)"
        wait = humanize(r["waiting"])
        if r["kind"] == "leg" and (r["cap"] or r["host"]):
            runner = r["waiting"] - r["cap"] - r["host"]
            parts = [f"runner {humanize(runner)}"]
            if r["cap"]:
                parts.append(f"capacity gate {humanize(r['cap'])}")
            if r["host"]:
                parts.append(f"quiet host {humanize(r['host'])}")
            wait += " (" + " + ".join(parts) + ")"
        behind = humanize(r["behind"]) if r["behind"] else "–"
        lines.append(f"| {stage} | {wait} | {behind} | {humanize(r['working'])} |")
    lines += ["",
              "_Waiting for resources = the wait for a runner plus the capacity gate "
              "(plus the wait for a quiet host when that exists). \"Behind earlier "
              "legs\" is sequencing (max_parallel), not idle time._",
              ""]
    return "\n".join(lines)


def build(repo, run_id, run_dir, fetcher=default_fetcher, now=None):
    now = int(now if now is not None else datetime.now(timezone.utc).timestamp())
    run = fetcher("run", repo, run_id)
    jobs = fetcher("jobs", repo, run_id)
    if isinstance(jobs, dict):
        jobs = jobs.get("jobs", [])
    run = dict(run)
    run.setdefault("id", run_id)
    rep = compute(run, jobs, run_dir or "", now)
    return rep


def main(argv=None, fetcher=default_fetcher, now=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("--repo", required=True)
    ap.add_argument("--run-id", required=True)
    ap.add_argument("--run-dir", default="")
    ap.add_argument("--out")
    ap.add_argument("--dump-json")
    try:
        a = ap.parse_args(argv)
    except SystemExit:
        return 0
    try:
        rep = build(a.repo, a.run_id, a.run_dir, fetcher, now)
        md = to_markdown(rep)
        js = json.dumps(to_json(rep), indent=2)
        if a.out:
            with open(a.out, "w", encoding="utf-8") as f:
                f.write(md)
        if a.dump_json:
            with open(a.dump_json, "w", encoding="utf-8") as f:
                f.write(js + "\n")
    except Exception as e:  # never fail the report
        print(f"run_timing: skipped ({e})", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
