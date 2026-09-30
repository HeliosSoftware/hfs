#!/usr/bin/env python3
#
# CRUD residue (search-counts comparability) — sums, per FHIR resource type,
# how many rows a benchmark leg's prewarm + crud suites left behind live.
#
# Called from: the `benchmark` job's "Run benchmark suites" step
# (fhir-benchmark.yml), via the step's own `$RESIDUE_PY` (python3, falling
# back to python — that resolution stays inline in the workflow), invoked as:
#   "$RESIDUE_PY" crud_residue.py <prewarm.json> <crud.json> > crud-residue.txt
#
# crud.js (upstream: HealthSamurai/fhir-server-performance-benchmark
# k6/crud.js) creates 9 resources per iteration then deletes them in reverse
# order; the checks it runs are named literally `${rt} created` / `${rt}
# delete` (verified against crud.js's `check(x, { [...]: ... })` calls).
# prewarm.js re-exports crud.js's SAME default/setup at 10 VUs for 30s, so a
# check failure on delete during EITHER suite (e.g. one 409/500) leaves a
# live row that the search-counts table would otherwise attribute to import
# instead of to crud/prewarm leftovers. created - delete, summed over both
# files' k6 --summary-export JSON, is that leftover per resource type. Walks
# root_group's checks/groups recursively rather than assuming a fixed
# nesting depth.
#
# Input: sys.argv[1:] — one or more k6 --summary-export JSON paths (a
# missing or unparsable file is skipped, not fatal).
# Output: stdout — one `<resourceType>=<created - deleted>` line per type
# that had a "created" or "delete" check, sorted by resource type name.
import json, re, sys


def collect_checks(node, out):
    if not isinstance(node, dict):
        return
    for name, c in (node.get("checks") or {}).items():
        if isinstance(c, dict):
            slot = out.setdefault(name, {"passes": 0, "fails": 0})
            slot["passes"] += c.get("passes", 0) or 0
            slot["fails"] += c.get("fails", 0) or 0
    for g in (node.get("groups") or {}).values():
        collect_checks(g, out)


checks = {}
for path in sys.argv[1:]:
    try:
        with open(path) as fh:
            root = json.load(fh).get("root_group", {})
    except (OSError, ValueError):
        continue
    collect_checks(root, checks)

totals = {}
for name, c in checks.items():
    m = re.match(r"^(.+) created$", name)
    if m:
        totals.setdefault(m.group(1), {"created": 0, "deleted": 0})["created"] += c["passes"]
        continue
    m = re.match(r"^(.+) delete$", name)
    if m:
        totals.setdefault(m.group(1), {"created": 0, "deleted": 0})["deleted"] += c["passes"]

for rt in sorted(totals):
    print(f"{rt}={totals[rt]['created'] - totals[rt]['deleted']}")
