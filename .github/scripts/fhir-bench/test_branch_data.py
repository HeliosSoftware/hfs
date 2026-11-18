"""Tests of branch_data.py (python3 -m unittest -q test_branch_data, from this folder). gh is mocked."""
import json
import os
import sys
import tempfile
import types
import unittest
import unittest.mock as mock

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import branch_data  # noqa: E402
import history  # noqa: E402


def run(i, branch, day, conclusion="success"):
    return {"id": i, "run_number": i, "head_branch": branch, "conclusion": conclusion,
            "created_at": "2026-11-%02dT10:00:00Z" % day, "html_url": ""}


class FakeGh:
    """runs: workflow runs listed; no_data: run ids without a report-data artifact."""
    def __init__(self, runs, no_data=(), fail_list=False):
        self.runs, self.no_data, self.fail_list, self.calls = runs, set(no_data), fail_list, []

    def __call__(self, args):
        self.calls.append(args)
        path = args[1] if args[0] == "api" else ""
        if "/workflows/" in path:
            if self.fail_list:
                return types.SimpleNamespace(returncode=1, stdout="", stderr="boom")
            return types.SimpleNamespace(returncode=0, stdout=json.dumps({"workflow_runs": self.runs}))
        if path.endswith("artifacts?per_page=100"):
            rid = int(path.split("/runs/")[1].split("/")[0])
            arts = [] if rid in self.no_data else [{"name": "fhir-bench-report-data-%d" % rid, "expired": False}]
            return types.SimpleNamespace(returncode=0, stdout=json.dumps({"artifacts": arts}))
        return types.SimpleNamespace(returncode=1, stdout="", stderr="")


def writer(run, d):
    p = os.path.join(d, str(run["id"]))
    os.makedirs(p, exist_ok=True)
    with open(os.path.join(p, "report-data.json"), "w", encoding="utf-8") as fh:
        json.dump({"run": str(run["id"]), "legs": {}}, fh)


class BranchData(unittest.TestCase):
    def go(self, gh, **kw):
        with tempfile.TemporaryDirectory() as t, mock.patch.object(history, "_gh", gh):
            s = branch_data.fetch("o/r", "fhir-benchmark.yml", kw.pop("exclude", "99"), kw.pop("current", "feat/x"),
                                  t, data_fn=kw.pop("data_fn", writer), **kw)
            with open(os.path.join(t, "branches.json"), encoding="utf-8") as fh:
                man = json.load(fh)
            dirs = sorted(x for x in os.listdir(t) if x != "branches.json")
        return s, man, dirs

    def test_groups_branches_drops_tags_current_and_excluded(self):
        runs = [run(1, "main", 1), run(2, "main", 2), run(3, "feat/x", 3), run(4, "v0.2.9", 4),
                run(5, "0.3.0", 5), run(6, "fix/y", 6), run(99, "fix/y", 7), run(7, "fix/y", 8, "cancelled")]
        s, man, dirs = self.go(FakeGh(runs))
        self.assertEqual(s["branches"], {"main": 2, "fix/y": 1})
        self.assertEqual(dirs, ["1", "2", "6"])
        self.assertEqual(man["6"], {"branch": "fix/y", "date": "2026-11-06T10:00:00Z", "number": 6})

    def test_per_branch_cap_and_runs_without_artifact_do_not_use_a_slot(self):
        runs = [run(i, "main", i) for i in range(1, 9)]
        s, man, dirs = self.go(FakeGh(runs, no_data={8, 7}), per_branch=3)
        self.assertEqual(dirs, ["4", "5", "6"])  # newest three that have the artifact
        self.assertEqual(s["branches"], {"main": 3})

    def test_max_branches_keeps_most_recent_activity(self):
        runs = [run(1, "old", 1), run(2, "mid", 2), run(3, "new", 3)]
        s, _, _ = self.go(FakeGh(runs), max_branches=2)
        self.assertEqual(sorted(s["branches"]), ["mid", "new"])

    def test_time_budget_stops_downloads(self):
        ticks = iter([0, 0, 1000, 1000, 1000, 1000, 1000])
        runs = [run(1, "a", 1), run(2, "b", 2)]
        s, man, _ = self.go(FakeGh(runs), clock=lambda: next(ticks), time_budget=10)
        self.assertTrue(s["over_budget"])
        self.assertEqual(len(man), 1)
        self.assertEqual(s["skipped"], 1)

    def test_never_fails(self):
        s, man, dirs = self.go(FakeGh([], fail_list=True))
        self.assertEqual((s["branches"], man, dirs), ({}, {}, []))
        boom = lambda run, d: (_ for _ in ()).throw(RuntimeError("x"))
        s, man, _ = self.go(FakeGh([run(1, "main", 1)]), data_fn=boom)
        self.assertEqual(man, {})
        # a download that leaves no report-data.json is not recorded
        s, man, _ = self.go(FakeGh([run(1, "main", 1)]), data_fn=lambda run, d: None)
        self.assertEqual(man, {})
        with tempfile.TemporaryDirectory() as t, mock.patch.object(history, "_gh", FakeGh([], fail_list=True)):
            self.assertEqual(branch_data.main(["--repo", "o/r", "--out-dir", t]), 0)
        self.assertEqual(branch_data.main(["--bogus"]), 0)

    def test_tag_pattern(self):
        for ref in ("v0.2.9", "0.2.9", "v1.2.3-rc.1"):
            self.assertTrue(branch_data.is_tag(ref), ref)
        for ref in ("main", "feat/v1", "release-1.2"):
            self.assertFalse(branch_data.is_tag(ref), ref)


if __name__ == "__main__":
    unittest.main()
