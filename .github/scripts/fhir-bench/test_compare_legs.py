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
INSERT_ROW = "insert (creates/s)"
_guard = {"on": False}


class PointsOpened(BaseException):  # not an Exception, so no `except Exception` hides it
    pass


def _audit(event, args):
    if _guard["on"] and event == "open" and str(args[0]).endswith("search-points.json"):
        raise PointsOpened(str(args[0]))


sys.addaudithook(_audit)  # cannot be removed, so it only acts while the flag is set


def summary(rps, p95, err=0.0, fails=0, it_p95=None):
    metrics = {
        "http_reqs": {"count": 1, "rate": rps},
        "http_req_duration": {"p(95)": p95},
        "http_req_failed": {"value": err},
        "checks": {"passes": 10, "fails": fails}}
    if it_p95 is not None:
        metrics["iteration_duration"] = {"p(95)": it_p95}
    return json.dumps({"metrics": metrics})


def leg_files(leg, bundles=1000, run=RUN, **override):
    files = {
        "runner-info.txt": "runner_name:  agent-%s\nrunner_cpus:  8\nrunner_ram:   23G\n"
                           "github_run:   %s\n" % (leg, run),
        "import-completeness.txt": "bundles_ok=%d\niterations=1000\nentries=5000\nwall_seconds=60\n"
                                   "scenario_seconds=50.0\nsetup_seconds=10.0\n" % bundles,
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


def indexing(status="completed", rate="1200.4", processed=5000, total=5000, errors=0,
             reason="", seconds="4.2"):
    return ("status=%s\nreason=%s\nresource_type=Encounter\njob_id=j\nkickoff_http=202\n"
            "total=%d\nprocessed=%d\nentries=%d\nerrors=%d\nseconds=%s\nwall_seconds=5\n"
            "resources_per_s=%s\nentries_per_resource=9.00\nbatch_size=1000\nbudget_s=240\n"
            "es_refresh=n/a\n" % (status, reason, total, processed, processed * 9, errors,
                                   seconds, rate))


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
        self.assertNotIn("**", row(md, "Throughput", "import (resources/s)"))  # a marked leg blocks the crown
        self.assertTrue(row(md, "Throughput", "import (resources/s)").endswith(" ‡ |"))

    def test_incomplete_import_blocks_crown(self):
        put_dir(self.root, "sqlite", leg_files("sqlite"))
        put_dir(self.root, "postgres", leg_files("postgres", bundles=874))
        md = render(self.root, ["sqlite", "postgres"])
        for suite in ("crud", "search"):
            self.assertTrue(row(md, "Throughput", suite).endswith("100 † |"))
            self.assertNotIn("**", row(md, "Throughput", suite))
        self.assertIn("**", row(md, "Throughput", "prewarm"))  # prewarm runs on an empty DB by design
        self.assertIn("874/1000 · 5,000 entries in 50 s + 10 s setup · 17.48 Bundles/s †", md)

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

    def test_import_row_units(self):
        put_dir(self.root, "sqlite", leg_files("sqlite", **{
            "import.json": summary(4.0, 900.0, it_p95=30000.0)}))
        put_dir(self.root, "postgres", leg_files("postgres", **{
            "import.json": summary(4.0, 900.0, it_p95=20000.0),
            "import-completeness.txt": "bundles_ok=1000\niterations=1000\nentries=5000\n"
                                       "wall_seconds=60\nscenario_seconds=25.0\nsetup_seconds=35.0\n"}))
        md = render(self.root, ["sqlite", "postgres"])
        self.assertIn("### Throughput (requests/s unless the row names its unit)", md)
        # entries / scenario_seconds, not k6's http_reqs.rate (4.0) nor entries / wall_seconds
        self.assertEqual(row(md, "Throughput", "import (resources/s)"),
                         "| import (resources/s) | 100 | **200** |")
        self.assertEqual(row(md, "p95", "import (per Bundle)"), "| import (per Bundle) | 30000.0 | **20000.0** |")
        self.assertEqual(row(md, "Errors", "import"), "| import | 0.0% | 0.0% |")
        self.assertEqual(row(md, "Throughput", "crud"), "| crud | **100** | **100** |")  # other rows keep their label
        self.assertIn("1,000/1000 · 5,000 entries in 25 s + 35 s setup · 40.00 Bundles/s |", md)
        self.assertIn(cl.IMPORT_NOTE, md)

    def test_import_without_scenario_time(self):  # an artifact from before scenario_seconds existed
        put_dir(self.root, "sqlite", leg_files("sqlite"))
        put_dir(self.root, "postgres", leg_files("postgres", **{
            "import-completeness.txt": "bundles_ok=1000\niterations=1000\nentries=5000\nwall_seconds=60\n"}))
        md = render(self.root, ["sqlite", "postgres"])
        self.assertEqual(row(md, "Throughput", "import (resources/s)"),
                         "| import (resources/s) | 100 | n/a (no import timing) |")
        self.assertIn("| `postgres` | ✓ | agent-postgres · 8 CPU / 23G | 1,000/1000 · 5,000 entries in 60 s |", md)

    def test_insert_row_label_crown_and_no_corpus_marker(self):
        put_dir(self.root, "sqlite", leg_files("sqlite", **{
            "insert.json": summary(812.4, 95.3), "insert.log": "k6\n"}))
        put_dir(self.root, "postgres", leg_files("postgres", bundles=874, **{
            "insert.json": summary(640.0, 120.0), "insert.log": "k6\n"}))
        md = render(self.root, ["sqlite", "postgres"])
        self.assertEqual(row(md, "Throughput", INSERT_ROW), "| insert (creates/s) | **812** | 640 |")
        self.assertEqual(row(md, "p95", "insert"), "| insert | **95.3** | 120.0 |")
        self.assertEqual(row(md, "Errors", "insert"), "| insert | 0.0% | 0.0% |")
        # same fixture: crud keeps its corpus marker, insert does not
        self.assertTrue(row(md, "Throughput", "crud").endswith("100 † |"))
        self.assertIn("its throughput row is creates/s", md)
        # tests=insert on a leg that ran only insert: the same row, nothing else
        # (a suite with a .log in the artifact would still show as an extra row)
        only = os.path.join(self.root, "only")
        put_dir(only, "sqlite", {"insert.json": summary(812.4, 95.3), "insert.log": "k6\n"})
        put_dir(only, "postgres", {"insert.json": summary(640.0, 120.0), "insert.log": "k6\n"})
        md = render(only, ["sqlite", "postgres"], tests="insert")
        self.assertEqual(row(md, "Throughput", INSERT_ROW), "| insert (creates/s) | **812** | 640 |")
        self.assertNotIn("| crud", md)

    def test_insert_not_run_and_not_requested(self):
        put_dir(self.root, "sqlite", leg_files("sqlite"))
        put_dir(self.root, "postgres", leg_files("postgres"))
        md = render(self.root, ["sqlite", "postgres"])
        self.assertEqual(row(md, "Throughput", INSERT_ROW),
                         "| insert (creates/s) | n/a (not run) | n/a (not run) |")
        md = render(self.root, ["sqlite", "postgres"], tests="prewarm,import,crud,search")
        self.assertNotIn("| insert", md)
        self.assertNotIn("creates/s", md)

    def test_insert_errors_block_crown(self):
        put_dir(self.root, "sqlite", leg_files("sqlite", **{
            "insert.json": summary(900.0, 50.0, err=0.02, fails=30), "insert.log": "k6\n"}))
        put_dir(self.root, "postgres", leg_files("postgres", **{
            "insert.json": summary(500.0, 80.0), "insert.log": "k6\n"}))
        md = render(self.root, ["sqlite", "postgres"])
        self.assertEqual(row(md, "Throughput", INSERT_ROW), "| insert (creates/s) | 900 ‡ | 500 |")
        self.assertIn("**2.0%** · **30 ✗**", row(md, "Errors", "insert"))

    def test_indexing_table_and_crown(self):
        put_dir(self.root, "sqlite", leg_files("sqlite", **{"indexing.txt": indexing()}))
        put_dir(self.root, "postgres", leg_files("postgres", **{
            "indexing.txt": indexing(rate="800")}))
        md = render(self.root, ["sqlite", "postgres"])
        self.assertIn("### Indexing (`$reindex` of Encounter)", md)
        self.assertEqual(row(md, "Indexing", "resources/s"), "| resources/s | **1,200** | 800 |")
        self.assertEqual(row(md, "Indexing", "seconds"), "| seconds | 4.2 | 4.2 |")
        self.assertEqual(row(md, "Indexing", "resources reindexed"),
                         "| resources reindexed | 5,000/5,000 | 5,000/5,000 |")
        self.assertNotIn("| indexing |", md)  # never a k6 row

    def test_indexing_partial_errors_and_short_import(self):
        put_dir(self.root, "sqlite", leg_files("sqlite", **{"indexing.txt": indexing(
            status="partial", reason="time-cap", processed=3000, rate="500.0", seconds="180.2")}))
        put_dir(self.root, "postgres", leg_files("postgres", bundles=874, **{
            "indexing.txt": indexing(errors=2)}))
        md = render(self.root, ["sqlite", "postgres"])
        self.assertEqual(row(md, "Indexing", "resources/s"),
                         "| resources/s | 500 (partial) | 1,200 †‡ |")
        self.assertEqual(row(md, "Indexing", "seconds"), "| seconds | 180.2 (time-cap) | 4.2 |")
        self.assertIn("3,000/5,000", row(md, "Indexing", "resources reindexed"))

    def test_indexing_shown_when_requested_hidden_otherwise(self):
        put_dir(self.root, "sqlite", leg_files("sqlite"))
        put_dir(self.root, "postgres", leg_files("postgres"))
        md = render(self.root, ["sqlite", "postgres"], tests="prewarm,import,crud,search")
        self.assertNotIn("### Indexing", md)  # not requested, nothing written (older runs)
        md = render(self.root, ["sqlite", "postgres", "mongodb"])  # tests=all includes indexing
        self.assertEqual(row(md, "Indexing", "resources/s"),
                         "| resources/s | n/a (not run) | n/a (not run) | n/a (no artifact) |")
        for heading in ("Throughput", "p95", "Errors"):
            self.assertNotIn("| indexing |", md.split("### " + heading, 1)[1].split("###", 1)[0])
        other = os.path.join(self.root, "x")
        put_dir(other, "sqlite", leg_files("sqlite", **{
            "indexing.txt": "status=kickoff-failed\nreason=http-501\nkickoff_http=501\n"}))
        put_dir(other, "postgres", leg_files("postgres", **{"indexing.txt": "status=weird\n"}))
        md = render(other, ["sqlite", "postgres"])
        self.assertEqual(row(md, "Indexing", "resources/s"),
                         "| resources/s | n/a (kick-off HTTP 501) | n/a (weird) |")
        self.assertEqual(row(md, "Indexing", "seconds"), "| seconds | n/a | n/a |")

    def test_indexing_no_crown_on_hardware_mismatch(self):
        put_dir(self.root, "sqlite", leg_files("sqlite", **{"indexing.txt": indexing()}))
        put_dir(self.root, "postgres", leg_files("postgres", **{
            "indexing.txt": indexing(rate="800"),
            "runner-info.txt": "runner_name: x\nrunner_cpus: 4\nrunner_ram: 23G\ngithub_run: 100\n"}))
        md = render(self.root, ["sqlite", "postgres"])
        self.assertEqual(row(md, "Indexing", "resources/s"), "| resources/s | 1,200 | 800 |")

    def test_indexing_marks_undrained_es_leg(self):
        put_dir(self.root, "sqlite-elasticsearch", leg_files("sqlite-elasticsearch", **{
            "indexing.txt": indexing()}))  # no es-drain.txt: never drained
        put_dir(self.root, "postgres-elasticsearch", leg_files("postgres-elasticsearch", **{
            "indexing.txt": indexing(rate="800"), "es-drain.txt": "status=drained\nmissing=0\n"}))
        md = render(self.root, ["sqlite-elasticsearch", "postgres-elasticsearch"])
        self.assertEqual(row(md, "Indexing", "resources/s"), "| resources/s | 1,200 † | 800 |")

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
