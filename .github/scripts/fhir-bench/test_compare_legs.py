#!/usr/bin/env python3
#
# Unit tests for compare_legs.py: stdlib unittest, synthetic fixtures built in
# code (no real artifacts). Not wired into CI. Run from the repo root:
#   python -m unittest discover -s .github/scripts/fhir-bench -p "test_*.py" -v
import contextlib
import io
import json
import os
import shutil
import sys
import tempfile
import unittest
import zipfile
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import compare_legs as cl  # noqa: E402

RUN = "100"
SUITES = ("prewarm", "import", "crud", "search")
_guard = {"on": False}


class PointsOpened(BaseException):  # not an Exception, so no `except Exception` hides it
    pass


def _audit(event, args):
    if _guard["on"] and event == "open" and str(args[0]).endswith("search-points.json"):
        raise PointsOpened(str(args[0]))


sys.addaudithook(_audit)  # cannot be removed, so it only acts while the flag is set


def summary(rps, p95, err=0.0, fails=0):
    return json.dumps({"metrics": {
        "http_reqs": {"count": 1, "rate": rps},
        "http_req_duration": {"p(95)": p95},
        "http_req_failed": {"value": err},
        "checks": {"passes": 10, "fails": fails}}})


def leg_files(leg, bundles=1000, run=RUN, **override):
    files = {
        "runner-info.txt": "runner_name:  agent-%s\nrunner_cpus:  8\nrunner_ram:   23G\n"
                           "github_run:   %s\n" % (leg, run),
        "import-completeness.txt": "bundles_ok=%d\niterations=1000\nentries=5000\nwall_seconds=60\n"
                                   % bundles,
        "host-contention.txt": "09:00:00Z suite=crud phase=start host_loadavg=1.50 2.00 3.00 "
                               "host_containers=7 host_mem_avail_mb=1 host_mem_source=none\n",
        "search-counts.txt": "query|total|http|seconds\nPatient?_summary=count|%d|200|0\n"
                             "Observation?_summary=count|5000|200|0\n" % bundles,
        "search-points.json": "not json {",
    }
    for s in SUITES:
        files[s + ".json"] = summary(100.0, 10.0)
        files[s + ".log"] = "k6 output\n"
    files.update(override)
    return dict((k, v) for k, v in files.items() if v is not None)


def put_dir(root, leg, files):  # layout C: ROOT/fhir-benchmark-<leg>-<run>/<leg>/<files>
    d = os.path.join(root, "fhir-benchmark-%s-%s" % (leg, RUN), leg)
    os.makedirs(d)
    for name, text in files.items():
        with open(os.path.join(d, name), "w", encoding="utf-8") as fh:
            fh.write(text)
    return d


def put_zip(path, leg, files):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as zf:
        for name, text in files.items():
            zf.writestr("%s/%s" % (leg, name), text)


def render(root, legs, tests="all", run=RUN):
    env = {"BENCH_RUN_ID": run, "BENCH_IN_TESTS": tests,
           "BENCH_MATRIX": json.dumps({"include": [{"backend": l} for l in legs]}),
           "BENCH_REF_NAME": "ref", "BENCH_SHA": "abcdef1234", "DOWNLOAD_OUTCOME": "success"}
    _guard["on"] = True  # the fixtures write search-points.json themselves, so only guard render
    try:
        return cl.render(root, env)[0]
    finally:
        _guard["on"] = False


def row(md, heading, suite):
    block = md.split("### " + heading, 1)[1].split("###", 1)[0]
    return next(l for l in block.splitlines() if l.startswith("| %s |" % suite))


class CompareLegsTest(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.root, True)

    def two_legs(self, **postgres):
        put_dir(self.root, "sqlite", leg_files("sqlite", **{
            "crud.json": summary(1210.9, 505.157)}))
        put_dir(self.root, "postgres", leg_files("postgres", **dict(
            {"crud.json": summary(500, 600)}, **postgres)))

    def test_formatting_and_crown(self):
        self.two_legs(**{"import.json": summary(100.0, 10.0, err=0.0628, fails=126)})
        md = render(self.root, ["sqlite", "postgres"])
        self.assertEqual(row(md, "Throughput", "crud"), "| crud | **1,211** | 500 |")
        self.assertEqual(row(md, "p95", "crud"), "| crud | **505.2** | 600.0 |")
        self.assertIn("**6.3%** · **126 ✗**", row(md, "Errors", "import"))
        self.assertNotIn("**", row(md, "Throughput", "import"))  # a marked leg blocks the crown
        self.assertTrue(row(md, "Throughput", "import").endswith(" ‡ |"))

    def test_incomplete_import_blocks_crown(self):
        put_dir(self.root, "sqlite", leg_files("sqlite"))
        put_dir(self.root, "postgres", leg_files("postgres", bundles=874))
        md = render(self.root, ["sqlite", "postgres"])
        for suite in ("crud", "search"):
            self.assertTrue(row(md, "Throughput", suite).endswith("100 † |"))
            self.assertNotIn("**", row(md, "Throughput", suite))
        self.assertIn("**", row(md, "Throughput", "prewarm"))  # prewarm runs on an empty DB by design
        self.assertRegex(md, r"874/1000 · 5,000 entries in 60 s †")

    def test_missing_leg_bad_json_missing_suite(self):
        put_dir(self.root, "sqlite", leg_files("sqlite"))
        put_dir(self.root, "postgres", leg_files("postgres", **{
            "crud.json": '{"metrics": {', "search.json": None,
            "prewarm.json": None, "prewarm.log": None}))
        md = render(self.root, ["sqlite", "postgres", "mongodb"])
        self.assertIn("n/a (unparsable)", row(md, "Throughput", "crud"))
        self.assertIn("n/a (no k6 summary)", row(md, "Throughput", "search"))
        self.assertIn("n/a (not run)", row(md, "Throughput", "prewarm"))
        self.assertIn("n/a (no artifact)", row(md, "Throughput", "crud"))
        self.assertIn("| `mongodb` | n/a: no artifact | n/a |", md)

    def test_zip_layouts_and_points_guard(self):
        flat = os.path.join(self.root, "b")  # layout B: one raw zip straight in ROOT
        put_zip(os.path.join(flat, "artifact"), "sqlite", leg_files("sqlite"))
        self.assertIn("| crud | 100 |", render(flat, ["sqlite"]))
        c, a = os.path.join(self.root, "c"), os.path.join(self.root, "a")
        for leg in ("sqlite", "postgres"):  # C: extracted dirs, A: ROOT/<artifact>/<raw zip>
            files = leg_files(leg, bundles=874 if leg == "postgres" else 1000)
            leg_dir = put_dir(c, leg, files)
            put_zip(os.path.join(a, "fhir-benchmark-%s-%s" % (leg, RUN), "artifact.zip"), leg, files)
        md_c, md_a = render(c, ["sqlite", "postgres"]), render(a, ["sqlite", "postgres"])
        self.assertEqual(md_a, md_c)
        self.assertIn("100 † |", row(md_a, "Throughput", "crud"))
        self.assertNotIn("search-points", md_a)
        with self.assertRaises(ValueError):
            cl._read(cl.DirSource(leg_dir, "postgres"), "search-points.json", cl.MAX_JSON_BYTES)

    def test_stale_and_hardware_mismatch(self):
        put_dir(self.root, "sqlite", leg_files("sqlite"))
        put_dir(self.root, "postgres", leg_files("postgres", run="1"))
        md = render(self.root, ["sqlite", "postgres"])
        self.assertIn("n/a (stale)", row(md, "Throughput", "crud"))
        self.assertIn("| `postgres` | n/a: stale (run 1) |", md)
        other = os.path.join(self.root, "hw")
        put_dir(other, "sqlite", leg_files("sqlite"))
        put_dir(other, "postgres", leg_files("postgres", **{
            "runner-info.txt": "runner_name: x\nrunner_cpus: 4\nrunner_ram: 23G\ngithub_run: 100\n"}))
        md = render(other, ["sqlite", "postgres"])
        self.assertNotIn("**", md.split("`†`")[0])  # everything above the legend: no crown anywhere
        self.assertIn("Runner hardware differs", md)

    def test_es_drain_and_dead_container(self):
        put_dir(self.root, "sqlite-elasticsearch", leg_files("sqlite-elasticsearch"))
        put_dir(self.root, "sqlite", leg_files("sqlite", **{
            "containers-state.txt": "hfs-bench-pg-x OOMKilled=true Status=exited\n"}))
        md = render(self.root, ["sqlite", "sqlite-elasticsearch"])
        self.assertTrue(row(md, "Throughput", "search").endswith("100 † |"))  # ES leg, no es-drain.txt
        self.assertTrue(row(md, "Throughput", "prewarm").startswith("| prewarm | 100 ⚠ |"))
        self.assertIn("⚠ container died", md)

    def test_never_raises(self):
        gone = os.path.join(self.root, "missing")
        self.assertIn("n/a (no artifact)", render(gone, ["sqlite"]))
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            self.assertEqual(cl.main(["x", gone]), 0)
            with mock.patch.object(cl, "render", side_effect=RuntimeError("boom")):
                out.seek(0)
                out.truncate(0)
                self.assertEqual(cl.main(["x", gone]), 0)
        self.assertIn("Leg comparison failed", out.getvalue())
        self.assertIn("::warning", err.getvalue())


if __name__ == "__main__":
    unittest.main()
