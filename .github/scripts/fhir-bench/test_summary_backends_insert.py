#!/usr/bin/env python3
#
# Unit tests for the insert-suite line of summary_backends.py: stdlib unittest,
# synthetic insert.json fixtures written to a temp workspace, the script run as
# a subprocess (it is a top-level script driven by env vars). Not wired into
# CI. Run from the repo root:
#   python -m unittest discover -s .github/scripts/fhir-bench -p "test_*.py" -v
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

SCRIPT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "summary_backends.py")


def insert_summary(p_pass=500, o_pass=2000, fails=0, submetrics=True):
    metrics = {"http_reqs": {"count": p_pass + o_pass, "rate": 208.4},
               "vus_max": {"max": 50}}
    if submetrics:
        metrics["http_req_duration{resource:Patient}"] = {"p(95)": 412.34}
        metrics["http_req_duration{resource:Observation}"] = {"p(95)": 398.06}
    return json.dumps({"metrics": metrics, "root_group": {"checks": {
        "insert Patient 201": {"passes": p_pass, "fails": fails},
        "insert Observation 201": {"passes": o_pass, "fails": 0}}}})


class InsertLineTest(unittest.TestCase):
    def setUp(self):
        self.ws = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.ws, True)
        os.makedirs(os.path.join(self.ws, "bench-results", "sqlite"))

    def run_summary(self, files):
        for name, text in files.items():
            with open(os.path.join(self.ws, "bench-results", "sqlite", name), "w") as fh:
                fh.write(text)
        env = dict(os.environ, GITHUB_WORKSPACE=self.ws, BACKEND="sqlite")
        env.pop("CAPACITY_NEED_MB", None)
        p = subprocess.run([sys.executable, SCRIPT], env=env, stdout=subprocess.PIPE,
                           stderr=subprocess.PIPE, universal_newlines=True)
        self.assertEqual(p.returncode, 0, p.stderr)
        return p.stdout

    def test_insert_line(self):
        out = self.run_summary({"insert.json": insert_summary()})
        self.assertIn("**Insert:** 2,500 create requests by 50 VUs, 208/s — "
                      "Patient 500 created (p95 412.3 ms) · "
                      "Observation 2,000 created (p95 398.1 ms).", out)
        self.assertNotIn("failed**", out)

    def test_insert_failures_flagged(self):
        out = self.run_summary({"insert.json": insert_summary(fails=7)})
        self.assertIn("⚠ **7 failed**", out)

    def test_no_insert_json(self):
        self.assertNotIn("**Insert:**", self.run_summary({}))

    def test_unparsable_insert_json(self):
        self.assertNotIn("**Insert:**", self.run_summary({"insert.json": "{"}))

    def test_missing_submetrics(self):
        out = self.run_summary({"insert.json": insert_summary(submetrics=False)})
        self.assertIn("Patient 500 created (p95 ? ms)", out)


if __name__ == "__main__":
    unittest.main()
