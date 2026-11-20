#!/usr/bin/env python3
"""Fetch the report-data.json of other branches' benchmark runs, for the HTML report's 'Compare branches'.

Reads only results that already exist (the fhir-bench-report-data-<run id> artifact of earlier runs of the
workflow); it never starts or re-runs anything. Every branch that ran the workflow is a candidate except the
current branch (its runs are the normal trend) and release tags (they have the version trend). For each kept
branch, the newest runs that still have that artifact, at most --per-branch, go to <out-dir>/<run id>/, and
<out-dir>/branches.json maps each downloaded run id to {"branch", "date", "number"}.

Best effort: any failure is reported on stderr and the script still exits 0; --time-budget stops the
downloads when exceeded. It is separate from history.py so history.md and the branch trend never change.
"""
import argparse
import json
import os
import re
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import history  # noqa: E402  (reuses _gh, _norm and fetch_data; their behaviour is unchanged)

TAG_RE = re.compile(r"^v?\d+\.\d+\.\d+")  # release tags: excluded, they have their own version trend
LIST_PAGES = 3                            # the 300 most recent completed runs of the workflow, every branch


def is_tag(ref):
    return bool(TAG_RE.match(ref or ""))


def list_runs(repo, workflow, pages=LIST_PAGES):
    """Completed runs (success or failure) of the workflow, every branch, newest first."""
    out = []
    for page in range(1, pages + 1):
        r = history._gh(["api", f"repos/{repo}/actions/workflows/{workflow}/runs"
                         f"?status=completed&per_page=100&page={page}"])
        if r.returncode != 0:
            break
        runs = json.loads(r.stdout).get("workflow_runs", [])
        out += [x for x in runs if x.get("conclusion") in ("success", "failure")]
        if len(runs) < 100:
            break
    return sorted(out, key=lambda x: x.get("created_at") or "", reverse=True)


def has_report_data(repo, run_id):
    """True when the run still has a non-expired fhir-bench-report-data-<run id> artifact."""
    r = history._gh(["api", f"repos/{repo}/actions/runs/{run_id}/artifacts?per_page=100"])
    if r.returncode != 0:
        return False
    return any(not a.get("expired") and a.get("name") == f"fhir-bench-report-data-{run_id}"
               for a in json.loads(r.stdout).get("artifacts", []))


def group_by_branch(runs, current_branch, exclude_id, max_branches):
    """{branch: [runs newest first]} without tags, the current branch and the excluded run; the
    max_branches branches with the most recent activity."""
    by = {}
    for x in runs:
        b = x.get("head_branch") or ""
        if not b or b == current_branch or is_tag(b) or str(x.get("id")) == str(exclude_id):
            continue
        by.setdefault(b, []).append(x)
    newest = sorted(by, key=lambda b: by[b][0].get("created_at") or "", reverse=True)
    return {b: by[b] for b in newest[:max_branches]}


def fetch(repo, workflow, exclude_id, current_branch, out_dir, per_branch=10, max_branches=8,
          time_budget=240, clock=time.monotonic, data_fn=None):
    """Download what fits in the time budget; returns a summary dict. Never raises."""
    data_fn = data_fn or (lambda run, d: history.fetch_data(repo, run, d, legacy=False))
    t0, manifest = clock(), {}
    summary = {"branches": {}, "skipped": 0, "listed": 0, "over_budget": False}
    try:
        os.makedirs(out_dir, exist_ok=True)
        runs = list_runs(repo, workflow)
        summary["listed"] = len(runs)
        groups = group_by_branch(runs, current_branch, exclude_id, max_branches)
        # newest run of every branch first, then the second of every branch, ...: a tight budget still
        # gives each branch a point; runs without the artifact are passed over (up to 3x per_branch looked at)
        depth = min(max([len(xs) for xs in groups.values()] or [0]), per_branch * 3)
        queue = [(b, xs[k]) for k in range(depth) for b, xs in groups.items() if k < len(xs)]
        kept = {b: 0 for b in groups}
        for b, x in queue:
            if kept[b] >= per_branch:
                continue
            if clock() - t0 > time_budget:
                summary["over_budget"] = True
                summary["skipped"] += 1
                continue
            try:
                if not has_report_data(repo, x["id"]):
                    continue
                run = history._norm(x)
                data_fn(run, out_dir)
                if os.path.isfile(os.path.join(out_dir, str(run["id"]), "report-data.json")):
                    manifest[str(run["id"])] = {"branch": b, "date": run["created_at"], "number": run["number"]}
                    kept[b] += 1
            except Exception as e:  # noqa: BLE001
                print(f"branch_data: run {x.get('id')} failed: {e}", file=sys.stderr)
        summary["branches"] = {b: n for b, n in kept.items() if n}
    except Exception as e:  # noqa: BLE001
        print(f"branch_data: {type(e).__name__}: {e}", file=sys.stderr)
    try:
        with open(os.path.join(out_dir, "branches.json"), "w", encoding="utf-8") as fh:
            json.dump(manifest, fh, indent=1, sort_keys=True)
    except Exception as e:  # noqa: BLE001
        print(f"branch_data: cannot write branches.json: {e}", file=sys.stderr)
    return summary


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--repo", required=True)
    ap.add_argument("--workflow", default="fhir-benchmark.yml")
    ap.add_argument("--exclude-run-id", default="")
    ap.add_argument("--current-branch", default="")
    ap.add_argument("--out-dir", required=True)
    ap.add_argument("--per-branch", type=int, default=10)
    ap.add_argument("--max-branches", type=int, default=8)
    ap.add_argument("--time-budget", type=float, default=240, help="seconds; no new download starts after it")
    try:
        a = ap.parse_args(argv)
    except SystemExit:
        return 0
    s = fetch(a.repo, a.workflow, a.exclude_run_id, a.current_branch, a.out_dir,
              max(1, a.per_branch), max(1, a.max_branches), a.time_budget)
    br = s["branches"]
    print("branch_data: %d runs listed; %d branch(es), %d run(s): %s%s" % (
        s["listed"], len(br), sum(br.values()), ", ".join("%s=%d" % kv for kv in br.items()) or "none",
        "; time budget exceeded, %d run(s) skipped" % s["skipped"] if s["over_budget"] else ""), file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
