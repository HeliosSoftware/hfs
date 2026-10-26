#!/usr/bin/env python3
#
# Unit tests for compare_legs.py: stdlib unittest, synthetic fixtures built in
# code (no real artifacts). Not wired into CI. Run from the repo root:
#   python -m unittest discover -s .github/scripts/fhir-bench -p "test_*.py" -v
import contextlib
import io
import json
import os
import re
import shutil
import sys
import tempfile
import unittest
import zipfile
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import compare_legs as cl  # noqa: E402

RUN = "100"
SUITES = ("prewarm", "import", "crud", "search", "sql-run")
INSERT_ROW = "insert (creates/s)"
IMPORT = "Batch/Transaction Import (`/ui/batch`: `POST /` Bundles)"  # Errors label; Throughput and p95 add a unit
IMPORT_ROW = IMPORT + " (resources/s)"
BULK = "Bulk Import (`/ui/bulk-import`: `$bulk-submit`)"
BULK_ROW = "Bulk Import (`/ui/bulk-import`: `$bulk-submit`, until searchable) (resources/s)"
INGEST_ROW = "↳ ingest, until Completed (resources/s, seconds)"
INDEX_ROW = "↳ search indexing (seconds)"
SQL_ROW = "SQL-on-FHIR `$sql-run` (requests/s)"
SQL_ROWS_ROW = "SQL-on-FHIR `$sql-run` (rows/s)"
SQL = "SQL-on-FHIR `$sql-run`"  # p95 and Errors label
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
    metrics["sql_run_rows"] = {"count": 100, "rate": 2500.0}
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


def bulk_txt(status="complete", **kv):
    """A bulk-import.txt body (key=value), complete by default."""
    fields = {"status": status, "resources_submitted": 1630685, "resources_ok": 1630685,
              "resources_failed": 0, "ingest_seconds": 600, "index_seconds": 1800,
              "total_seconds": 2400, "resources_per_s": 679.5, "ingest_resources_per_s": 2717.8,
              "index_status": "completed", "searchable": "yes",
              "expect_patient": 1000, "check_patient": 1000,
              "expect_observation": 689080, "check_observation": 689080,
              "expect_observation_code": 33473, "check_observation_code": 33473,
              "expect_encounter_class": 65900, "check_encounter_class": 65900}
    fields.update(kv)
    return "".join("%s=%s\n" % (k, v) for k, v in fields.items())


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


def labels(md, heading):
    """The first-column labels of one table, header excluded."""
    block = md.split("### " + heading, 1)[1].split("###", 1)[0]
    return [l[2:].split(" | ", 1)[0] for l in block.splitlines() if l.startswith("| ")][1:]


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
        self.assertIn("**6.3%** · **126 ✗**", row(md, "Errors", IMPORT))
        self.assertNotIn("**", row(md, "Throughput", IMPORT_ROW))  # a marked leg blocks the crown
        self.assertTrue(row(md, "Throughput", IMPORT_ROW).endswith(" ‡ |"))

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
        self.assertEqual(row(md, "Throughput", IMPORT_ROW), "| %s | 100 | **200** |" % IMPORT_ROW)
        self.assertEqual(row(md, "p95", IMPORT + " (per Bundle)"),
                         "| " + IMPORT + " (per Bundle) | 30000.0 | **20000.0** |")
        self.assertEqual(row(md, "Errors", IMPORT), "| " + IMPORT + " | 0.0% | 0.0% |")
        self.assertEqual(row(md, "Throughput", "crud"), "| crud | **100** | **100** |")  # other rows keep their label
        self.assertIn("1,000/1000 · 5,000 entries in 25 s + 35 s setup · 40.00 Bundles/s |", md)
        self.assertIn(cl.IMPORT_NOTE, md)

    def test_import_without_scenario_time(self):  # an artifact from before scenario_seconds existed
        put_dir(self.root, "sqlite", leg_files("sqlite"))
        put_dir(self.root, "postgres", leg_files("postgres", **{
            "import-completeness.txt": "bundles_ok=1000\niterations=1000\nentries=5000\nwall_seconds=60\n"}))
        md = render(self.root, ["sqlite", "postgres"])
        self.assertEqual(row(md, "Throughput", IMPORT_ROW),
                         "| %s | 100 | n/a (no import timing) |" % IMPORT_ROW)
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
        # crud and search are always listed, so a trimmed run does not look like a report
        # that lost them
        self.assertEqual(row(md, "Throughput", "crud"),
                         "| crud | n/a (not requested) | n/a (not requested) |")
        self.assertEqual(row(md, "Throughput", "search"),
                         "| search | n/a (not requested) | n/a (not requested) |")

    def test_bulk_import_only_run_still_lists_crud_and_search(self):
        for leg in ("sqlite", "postgres"):
            put_dir(self.root, leg, {"bulk-import.txt": bulk_txt(), "bulk-import.log": "x\n"})
        md = render(self.root, ["sqlite", "postgres"], tests="bulk-import")
        for table in ("Throughput", "p95", "Errors"):
            for suite in ("crud", "search"):
                self.assertTrue(row(md, table, suite).endswith(
                    "| n/a (not requested) | n/a (not requested) |"), (table, suite))
        self.assertIn(BULK_ROW, md)

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

    def test_canonical_matches_workflow(self):
        # the suites tests=all runs are listed in two places; keep them in step
        here = os.path.dirname(os.path.abspath(__file__))
        path = os.path.join(here, "..", "..", "workflows", "fhir-benchmark.yml")
        with open(path, encoding="utf-8") as fh:
            m = re.search(r'CANONICAL="([^"]+)"', fh.read())
        self.assertIsNotNone(m)
        self.assertEqual(m.group(1), " ".join(cl.CANONICAL_SUITES))

    def test_loading_rows_together_and_named(self):
        for leg in ("sqlite", "postgres"):
            put_dir(self.root, leg, leg_files(leg, **{
                "bulk-import.txt": bulk_txt(),
                "insert.json": summary(800.0, 90.0), "insert.log": "k6\n"}))
        md = render(self.root, ["sqlite", "postgres"])
        self.assertEqual(labels(md, "Throughput"),
                         ["prewarm", IMPORT_ROW, BULK_ROW, INGEST_ROW, INDEX_ROW,
                          "crud", "search", SQL_ROW, SQL_ROWS_ROW, INSERT_ROW])
        self.assertEqual(labels(md, "p95"), ["prewarm", IMPORT + " (per Bundle)", BULK,
                                             "crud", "search", SQL, "insert"])
        self.assertEqual(labels(md, "Errors"), ["prewarm", IMPORT, BULK,
                                                "crud", "search", SQL, "insert"])
        self.assertNotIn("$import", md)  # HFS has none; no label may suggest it
        # the four import rows sit right after prewarm, in this order, under the UI names
        t = labels(md, "Throughput")
        self.assertEqual(t[:5], ["prewarm", IMPORT_ROW, BULK_ROW, INGEST_ROW, INDEX_ROW])
        self.assertNotIn("20 concurrent clients", IMPORT_ROW)
        self.assertIn("k6 drives it with 20 concurrent clients.", cl.IMPORT_NOTE)
        self.assertTrue(INGEST_ROW.startswith("↳ ") and INDEX_ROW.startswith("↳ "))
        # one vocabulary: "search indexing", never "initial index"
        self.assertNotIn("initial index", md)
        self.assertNotIn("initial-index", md)
        self.assertIn("search indexing", md)
        self.assertNotIn("search-index rebuild after Completed", md)
        self.assertIn("`†` crud/search ran on an incomplete corpus", md)
        self.assertIn("shown only when no leg in that row carries a marker or `(partial)`", md)

    def test_display_order_keeps_run_order_markers(self):
        # tests in run order crud, search, import: crud and search ran before import
        for leg in ("sqlite", "postgres"):
            put_dir(self.root, leg, leg_files(leg, **{"bulk-import.txt": bulk_txt()}))
        md = render(self.root, ["sqlite", "postgres"],
                    tests="prewarm,crud,bulk-import,search,import")
        self.assertEqual(labels(md, "Throughput"),
                         ["prewarm", "crud", IMPORT_ROW, BULK_ROW, INGEST_ROW, INDEX_ROW,
                          "search", SQL_ROW, SQL_ROWS_ROW])
        for suite in ("crud", "search"):  # shown after import, still marked as run before it
            self.assertTrue(row(md, "Throughput", suite).endswith("100 † |"), suite)

    def test_bulk_import_row_and_table(self):
        put_dir(self.root, "sqlite", leg_files("sqlite", **{"bulk-import.txt": bulk_txt()}))
        put_dir(self.root, "postgres", leg_files("postgres", **{
            "bulk-import.txt": bulk_txt(resources_per_s=1500.0, total_seconds=1087)}))
        md = render(self.root, ["sqlite", "postgres"])
        self.assertEqual(row(md, "Throughput", BULK_ROW), "| " + BULK_ROW + " | 680 | **1,500** |")
        self.assertEqual(row(md, "p95", BULK),
                         "| " + BULK + " | n/a (one async job) | n/a (one async job) |")
        self.assertEqual(row(md, "Errors", BULK), "| " + BULK + " | 0.0% | 0.0% |")
        # the two phases sit right under the combined row, in the Throughput table only
        self.assertEqual(row(md, "Throughput", INGEST_ROW),
                         "| " + INGEST_ROW + " | **2,718** (600 s) | **2,718** (600 s) |")
        self.assertEqual(row(md, "Throughput", INDEX_ROW),
                         "| " + INDEX_ROW + " | **1,800 s** | **1,800 s** |")
        for heading in ("p95", "Errors"):
            self.assertNotIn(INGEST_ROW, labels(md, heading))
            self.assertNotIn(INDEX_ROW, labels(md, heading))
        self.assertIn("### Bulk import (`$bulk-submit`)", md)
        table = md.split("### Bulk import", 1)[1]
        self.assertIn("| `sqlite` | ✓ | 1,630,685/1,630,685 | 600 s · 2,718/s | 1,800 s | 2,400 s | "
                      "✓ Patient 1,000 · Observation 689,080 |", table)
        self.assertIn("| `postgres` | ✓ | 1,630,685/1,630,685 |", table)
        self.assertLess(md.index("### Leg health"), md.index("### Bulk import"))

    def test_bulk_import_markers_and_reasons(self):
        put_dir(self.root, "sqlite", leg_files("sqlite", **{
            "bulk-import.txt": bulk_txt(resources_ok=1610685, resources_failed=20000)}))
        put_dir(self.root, "postgres", leg_files("postgres", **{
            "bulk-import.txt": bulk_txt(searchable="no", check_observation_code=30000)}))
        put_dir(self.root, "mongodb", leg_files("mongodb", **{
            "bulk-import.txt": bulk_txt("timeout", reason="index phase after 3,600 s",
                                        resources_ok=1630685, phase="index")}))
        put_dir(self.root, "sqlite-elasticsearch", leg_files("sqlite-elasticsearch", **{
            "es-drain.txt": "status=drained\nmissing=0\n"}))
        put_dir(self.root, "postgres-elasticsearch", leg_files(
            "postgres-elasticsearch", bundles=874, **{"bulk-import.txt": bulk_txt()}))
        legs = ["sqlite", "postgres", "mongodb", "sqlite-elasticsearch", "postgres-elasticsearch"]
        md = render(self.root, legs)
        cells = [c.strip() for c in row(md, "Throughput", BULK_ROW).split("|")[2:-1]]
        self.assertTrue(cells[0].endswith("‡"), cells[0])
        self.assertEqual(cells[1], "680 †")
        self.assertEqual(cells[2], "n/a (timeout: index phase after 3,600 s)")
        self.assertEqual(cells[3], "n/a (not run)")
        # The transactional import's completeness (874/1000) does not mark the bulk row.
        self.assertEqual(cells[4], "680")
        self.assertIn("· **20,000 ✗**", row(md, "Errors", BULK))
        table = md.split("### Bulk import", 1)[1]
        self.assertIn("| `sqlite` | ✓ ‡ | 1,610,685/1,630,685 · **20,000 failed** |", table)
        self.assertIn("**short**: `Observation?code` 30,000/33,473", table)
        self.assertIn("| `mongodb` | timeout: index phase after 3,600 s |", table)
        self.assertIn("| `sqlite-elasticsearch` | n/a (not run) |", table)

    def test_bulk_import_not_requested(self):
        put_dir(self.root, "sqlite", leg_files("sqlite", **{"bulk-import.txt": bulk_txt()}))
        md = render(self.root, ["sqlite"], tests="prewarm,import,crud,search")
        self.assertNotIn("| bulk-import", md)
        self.assertNotIn("### Bulk import", md)
        self.assertNotIn(cl.BULK_PHASES_NOTE, md)

    def test_bulk_phase_rows_deferred(self):
        put_dir(self.root, "sqlite", leg_files("sqlite", **{"bulk-import.txt": bulk_txt()}))
        put_dir(self.root, "postgres", leg_files("postgres", **{"bulk-import.txt": bulk_txt(
            ingest_seconds=500, ingest_resources_per_s=3261.4, index_seconds=587.4,
            total_seconds=1087.4, resources_per_s=1499.6)}))
        md = render(self.root, ["sqlite", "postgres"])
        self.assertEqual(row(md, "Throughput", INGEST_ROW),
                         "| " + INGEST_ROW + " | 2,718 (600 s) | **3,261** (500 s) |")
        self.assertEqual(row(md, "Throughput", INDEX_ROW),
                         "| " + INDEX_ROW + " | 1,800 s | **587 s** |")
        self.assertEqual(labels(md, "Throughput"),
                         ["prewarm", IMPORT_ROW, BULK_ROW, INGEST_ROW, INDEX_ROW,
                          "crud", "search", SQL_ROW, SQL_ROWS_ROW, INSERT_ROW])
        for heading in ("p95", "Errors"):
            self.assertNotIn(INGEST_ROW, labels(md, heading))
            self.assertNotIn(INDEX_ROW, labels(md, heading))
        self.assertIn(cl.BULK_PHASES_NOTE, md)
        self.assertLess(md.index(cl.IMPORT_NOTE), md.index(cl.BULK_PHASES_NOTE))
        self.assertLess(md.index(cl.BULK_PHASES_NOTE), md.index("### Leg health"))

    def test_bulk_phase_rows_inline(self):
        inline = dict(defer_indexing="false", index_status="inline", index_seconds=0,
                      ingest_seconds=600, total_seconds=600, resources_per_s=2717.8)
        # one leg deferred, one inline: their index times are different things
        put_dir(self.root, "sqlite", leg_files("sqlite", **{"bulk-import.txt": bulk_txt()}))
        put_dir(self.root, "postgres", leg_files("postgres", **{
            "bulk-import.txt": bulk_txt(**inline)}))
        md = render(self.root, ["sqlite", "postgres"])
        index = row(md, "Throughput", INDEX_ROW)
        self.assertEqual(index, "| " + INDEX_ROW + " | 1,800 s | 600 s (= ingest) |")
        self.assertNotIn("**", index)
        self.assertNotIn("**", row(md, "Throughput", INGEST_ROW))  # mixed: no best either
        self.assertEqual(row(md, "Throughput", BULK_ROW), "| " + BULK_ROW + " | 680 | **2,718** |")
        table = md.split("### Bulk import", 1)[1]
        self.assertIn("| `postgres` | ✓ | 1,630,685/1,630,685 | 600 s · 2,718/s | "
                      "inline (during ingest) | 600 s |", table)
        # every leg inline: the ingest rows still compare, the index row has nothing to rank
        other = os.path.join(self.root, "all-inline")
        put_dir(other, "sqlite", leg_files("sqlite", **{"bulk-import.txt": bulk_txt(
            **dict(inline, ingest_resources_per_s=2000.0))}))
        put_dir(other, "postgres", leg_files("postgres", **{
            "bulk-import.txt": bulk_txt(**inline)}))
        md = render(other, ["sqlite", "postgres"])
        self.assertEqual(row(md, "Throughput", INGEST_ROW),
                         "| " + INGEST_ROW + " | 2,000 (600 s) | **2,718** (600 s) |")
        self.assertEqual(row(md, "Throughput", INDEX_ROW),
                         "| " + INDEX_ROW + " | 600 s (= ingest) | 600 s (= ingest) |")
        # indexing off in the ingest, but a follow-up rebuild of the types the index rejected
        rebuild = os.path.join(self.root, "rebuild")
        put_dir(rebuild, "sqlite", leg_files("sqlite", **{"bulk-import.txt": bulk_txt(
            defer_indexing="false", index_status="completed", index_seconds=12,
            total_seconds=612)}))
        md = render(rebuild, ["sqlite"])
        self.assertEqual(row(md, "Throughput", INDEX_ROW),
                         "| " + INDEX_ROW + " | inline (during ingest) + 12 s rebuild |")
        self.assertIn("| inline (during ingest) + 12 s rebuild |",
                      md.split("### Bulk import", 1)[1])

    def test_sql_run_rows(self):
        for leg in ("sqlite", "postgres"):
            put_dir(self.root, leg, leg_files(leg))
        md = render(self.root, ["sqlite", "postgres"])
        t = labels(md, "Throughput")
        self.assertEqual(t[t.index("search"):t.index("search") + 3],
                         ["search", SQL_ROW, SQL_ROWS_ROW])
        self.assertLess(t.index(SQL_ROWS_ROW), t.index(INSERT_ROW))
        self.assertEqual(row(md, "Throughput", SQL_ROWS_ROW), "| " + SQL_ROWS_ROW + " | **2,500** | **2,500** |")
        for heading in ("p95", "Errors"):
            l = labels(md, heading)
            self.assertLess(l.index("search"), l.index(SQL))
            self.assertLess(l.index(SQL), l.index("insert"))
        self.assertNotIn(SQL_ROWS_ROW, labels(md, "p95"))
        self.assertIn(cl.SQL_RUN_NOTE, md)

    def test_bulk_phase_rows_na_and_markers(self):
        legs = ["sqlite", "postgres", "mongodb", "sqlite-elasticsearch", "postgres-elasticsearch",
                "mongodb-elasticsearch"]
        put_dir(self.root, "sqlite", leg_files("sqlite", **{"bulk-import.txt": bulk_txt(
            resources_ok=1610685, resources_failed=20000)}))
        put_dir(self.root, "postgres", leg_files("postgres", **{"bulk-import.txt": bulk_txt(
            searchable="no", check_observation_code=30000)}))
        put_dir(self.root, "mongodb", leg_files("mongodb", **{"bulk-import.txt": bulk_txt(
            "timeout", reason="index phase after 3,600 s", phase="index", index_status="",
            index_seconds="", total_seconds="", resources_per_s="")}))
        put_dir(self.root, "sqlite-elasticsearch", leg_files("sqlite-elasticsearch", **{
            "es-drain.txt": "status=drained\nmissing=0\n",
            "bulk-import.txt": bulk_txt(
                "timeout", reason="ingest after 3,600 s", phase="ingest", ingest_seconds="",
                ingest_resources_per_s="", index_status="", index_seconds="",
                total_seconds="", resources_per_s="")}))
        put_dir(self.root, "postgres-elasticsearch", leg_files("postgres-elasticsearch"))
        md = render(self.root, legs)  # mongodb-elasticsearch is in the matrix with no artifact

        def cells(label):
            return [c.strip() for c in row(md, "Throughput", label).split("|")[2:-1]]

        ingest, index = cells(INGEST_ROW), cells(INDEX_ROW)
        self.assertEqual(ingest[0], "2,718 (600 s) ‡")
        self.assertEqual(index[0], "1,800 s")  # the failures were in the ingest
        self.assertEqual(ingest[1], "2,718 (600 s)")
        self.assertEqual(index[1], "1,800 s †")  # search came back short
        # the ingest finished and was recorded; only the index phase did not
        self.assertEqual(ingest[2], "2,718 (600 s)")
        self.assertEqual(index[2], "n/a (timeout: index phase after 3,600 s)")
        self.assertEqual(ingest[3], "n/a (timeout: ingest after 3,600 s)")
        self.assertEqual(index[3], "n/a (timeout: ingest after 3,600 s)")
        self.assertEqual(ingest[4], "n/a (not run)")
        self.assertEqual(index[4], "n/a (not run)")
        self.assertEqual(ingest[5], "n/a (no artifact)")
        self.assertEqual(index[5], "n/a (no artifact)")
        # a marker anywhere in a row means no best in it
        self.assertNotIn("**", row(md, "Throughput", INGEST_ROW))
        self.assertNotIn("**", row(md, "Throughput", INDEX_ROW))
        # a rebuild that failed, and a container that died
        other = os.path.join(self.root, "x")
        put_dir(other, "sqlite", leg_files("sqlite", **{"bulk-import.txt": bulk_txt(
            index_status="failed", searchable="no", check_observation_code=30000)}))
        put_dir(other, "postgres", leg_files("postgres", **{
            "containers-state.txt": "hfs-bench-pg-x OOMKilled=true Status=running\n",
            "bulk-import.txt": bulk_txt()}))
        put_dir(other, "mongodb", leg_files("mongodb", **{"bulk-import.txt": bulk_txt(
            index_errors=3)}))
        md = render(other, ["sqlite", "postgres", "mongodb"])
        self.assertEqual(row(md, "Throughput", INDEX_ROW),
                         "| " + INDEX_ROW + " | 1,800 s (failed) †‡ | 1,800 s ⚠ | 1,800 s ‡ |")
        self.assertEqual(row(md, "Throughput", INGEST_ROW),
                         "| " + INGEST_ROW + " | 2,718 (600 s) | 2,718 (600 s) ⚠ | 2,718 (600 s) |")

    def test_bulk_phase_rows_old_artifacts(self):
        # before ingest_* / index_* existed: only the combined figures were written
        old = ("status=complete\nresources_submitted=1630685\nresources_ok=1630685\n"
               "resources_failed=0\ntotal_seconds=2400\nresources_per_s=679.5\nsearchable=yes\n")
        put_dir(self.root, "sqlite", leg_files("sqlite", **{"bulk-import.txt": old}))
        put_dir(self.root, "postgres", leg_files("postgres", **{"bulk-import.txt": bulk_txt(
            ingest_seconds="", ingest_resources_per_s="", index_status="", index_seconds="",
            resources_per_s=700.0)}))
        md = render(self.root, ["sqlite", "postgres"])
        self.assertEqual(row(md, "Throughput", INGEST_ROW),
                         "| " + INGEST_ROW + " | n/a (no ingest timing) | n/a (no ingest timing) |")
        self.assertEqual(row(md, "Throughput", INDEX_ROW),
                         "| " + INDEX_ROW + " | n/a (no index timing) | n/a (no index timing) |")
        self.assertEqual(row(md, "Throughput", BULK_ROW), "| " + BULK_ROW + " | 680 | **700** |")
        table = md.split("### Bulk import", 1)[1]
        self.assertIn("| `sqlite` | ✓ | 1,630,685/1,630,685 | n/a | n/a | 2,400 s |", table)

    def test_bulk_phase_rows_no_crown_on_hardware_mismatch(self):
        put_dir(self.root, "sqlite", leg_files("sqlite", **{"bulk-import.txt": bulk_txt()}))
        put_dir(self.root, "postgres", leg_files("postgres", **{
            "runner-info.txt": "runner_name: x\nrunner_cpus: 4\nrunner_ram: 23G\ngithub_run: 100\n",
            "bulk-import.txt": bulk_txt(ingest_seconds=500, ingest_resources_per_s=3261.4,
                                        index_seconds=587.4)}))
        md = render(self.root, ["sqlite", "postgres"])
        self.assertEqual(row(md, "Throughput", INGEST_ROW),
                         "| " + INGEST_ROW + " | 2,718 (600 s) | 3,261 (500 s) |")
        self.assertEqual(row(md, "Throughput", INDEX_ROW),
                         "| " + INDEX_ROW + " | 1,800 s | 587 s |")

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

    def _main_out(self, *args):
        out = io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(cl.main(["x", self.root] + list(args)), 0)
        return out.getvalue()

    def test_timing_block(self):
        self.two_legs()
        base = self._main_out()
        gone = os.path.join(self.root, "no-such-timing.md")
        empty = os.path.join(self.root, "empty-timing.md")
        open(empty, "w").close()
        self.assertEqual(self._main_out("--timing", gone), base)
        self.assertEqual(self._main_out("--timing", empty), base)
        block = "\n### Run time\n\n| Phase | Seconds |\n|---|---|\n| total | 12 |\n"
        path = os.path.join(self.root, "timing.md")
        with open(path, "w", encoding="utf-8") as f:
            f.write(block)
        md = self._main_out("--timing", path)
        self.assertEqual(md.count("### Run time"), 1)
        head, rest = md.split("### Run time", 1)
        self.assertIn("legs have results", head)
        self.assertNotIn("###", head)
        self.assertIn("| total | 12 |", rest.split("### Throughput", 1)[0])
        self.assertEqual(md.replace(block.strip("\n") + "\n\n", "", 1), base)


if __name__ == "__main__":
    unittest.main()
