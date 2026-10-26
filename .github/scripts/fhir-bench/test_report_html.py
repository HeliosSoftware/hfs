"""Tests of report_html.py (python3 -m unittest -q test_report_html, from this folder)."""
import json
import os
import re
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPT = os.path.join(HERE, "report_html.py")
REAL = os.path.expanduser("~/tmp/ci-bench/40335047310")


def k6(rps=100.0, extra=None):
    m = {"http_reqs": {"count": 1000, "rate": rps},
         "http_req_failed": {"value": 0.0},
         "http_req_duration": {"med": 10.0, "p(95)": 40.0, "p(99)": 80.0, "max": 120.0},
         "iteration_duration": {"med": 500.0, "p(95)": 900.0, "p(99)": 1200.0, "max": 1500.0},
         "checks": {"passes": 1000, "fails": 0}, "vus_max": {"max": 10}}
    m.update(extra or {})
    return json.dumps({"metrics": m})


def put(d, name, text):
    os.makedirs(d, exist_ok=True)
    with open(os.path.join(d, name), "w", encoding="utf-8") as fh:
        fh.write(text)


def make_leg(root, leg, es):
    d = os.path.join(root, "fhir-benchmark-%s-777" % leg, leg)
    put(d, "runner-info.txt", "runner_name:  r1\nrunner_cpus:  8\nrunner_ram:   16G\ngithub_run:   777\n"
        "git_sha:      abcdef1234567\ngit_ref:      feature/x\n")
    for s in ("prewarm", "import", "crud", "search", "insert"):
        put(d, s + ".json", k6())
    views = {}
    for v in ("claim", "eob"):
        views["http_req_duration{view:%s}" % v] = {"med": 20.0, "p(95)": 50.0, "p(99)": 70.0, "max": 90.0, "count": 5}
    views["sql_run_rows"] = {"count": 5000, "rate": 1234.5}
    put(d, "sql-run.json", k6(160.0, views))
    put(d, "import-completeness.txt", "bundles_ok=1000\nentries=100000\nscenario_seconds=100\nsetup_seconds=50\nwall_seconds=160\n")
    put(d, "bulk-import.txt", "status=complete\nphase=done\nconvert_seconds=40\ningest_seconds=200\n"
        "index_status=%s\nindex_seconds=%s\ntotal_seconds=200\nresources_per_s=5000\n"
        "ingest_resources_per_s=5000\nresources_ok=1000\nresources_submitted=1000\nresources_failed=0\nsearchable=yes\n"
        % (("inline", "0.0") if es else ("deferred", "35.5")))
    put(d, "containers-state.txt", "hfs-bench-%s-1 Status=running OOMKilled=false\n" % leg)
    put(d, "host-contention.txt",
        "10:00:00Z suite=import phase=start host_loadavg=1.0 0.5 0.2 host_containers=3 host_mem_avail_mb=9000\n"
        "10:05:00Z suite=import phase=end host_loadavg=4.0 2.0 1.0 host_containers=3 host_mem_avail_mb=8000\n")
    if es:
        put(d, "es-drain.txt", "status=drained\nmissing=0\nsync_mode=synchronous\n")
    return d


def run(args):
    return subprocess.run([sys.executable, "-I", SCRIPT] + args, capture_output=True, text=True, timeout=300)


class SyntheticRun(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        t = cls.tmp.name
        make_leg(t, "sqlite", False)
        make_leg(t, "sqlite-elasticsearch", True)
        make_leg(t, "weird-backend", False)  # not a known leg: must still be discovered
        cls.out, cls.dump = os.path.join(t, "r.html"), os.path.join(t, "d.json")
        cls.p = run(["--run-dir", t, "--run-id", "777", "--repo", "o/r", "--branch", "feature/x",
                     "--sha", "abcdef1234567", "--out", cls.out, "--dump-json", cls.dump, "--no-shapes"])
        with open(cls.out, encoding="utf-8") as fh:
            cls.html = fh.read()
        with open(cls.dump, encoding="utf-8") as fh:
            cls.data = json.load(fh)

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def test_exit_and_valid_page(self):
        self.assertEqual(self.p.returncode, 0, self.p.stderr)
        self.assertTrue(self.html.startswith("<!doctype html>"))
        self.assertTrue(self.html.rstrip().endswith("</html>"))
        self.assertNotIn("/*__DATA__*/", self.html)
        self.assertNotRegex(self.html, r"(src|href)=\"https?://(?!github\.com)")

    def test_embedded_json_parses(self):
        m = re.search(r"const D = (\{.*?\});\nconst LEGS", self.html, re.S)
        self.assertIsNotNone(m)
        d = json.loads(m.group(1).replace("<\\/", "</"))
        self.assertEqual(d["order"], ["sqlite", "sqlite-elasticsearch", "weird-backend"])

    def test_legs_discovered_without_fixed_list(self):
        self.assertEqual(self.data["order"], ["sqlite", "sqlite-elasticsearch", "weird-backend"])
        self.assertEqual(self.data["missing"], [])
        for leg in self.data["order"]:
            self.assertIn(leg, self.html)
        pal = self.data["palette"]["dark"]
        self.assertEqual(pal["sqlite"], "#008622")  # v8 colour kept
        self.assertEqual(len(set(pal.values())), 3)

    def test_sql_run_section(self):
        s = self.data["legs"]["sqlite"]["suites"]["sql-run"]
        self.assertEqual(s["rows_rps"], 1234.5)
        self.assertEqual([v["name"] for v in s["views"]], ["claim", "eob"])
        self.assertEqual(s["views"][0]["p95"], 50.0)
        self.assertIn("sqlBlock", self.html)
        self.assertIn("$sql-run", self.html)
        self.assertIn('"sql-run"', self.html)

    def test_inline_wording_never_zero(self):
        b = self.data["legs"]["sqlite-elasticsearch"]["bulk"]
        self.assertTrue(b["index_inline"])
        self.assertIsNone(b["index"])
        self.assertEqual(b["index_eff"], 200.0)
        self.assertIn("inline (during ingest)", self.html)
        self.assertIn("(= ingest)", self.html)
        d = self.data["legs"]["sqlite"]["bulk"]
        self.assertFalse(d["index_inline"])
        self.assertEqual(d["index"], 35.5)

    def test_labels_and_dropped_text(self):
        for txt in ("Batch/Transaction Import (/ui/batch)", "Bulk Import (/ui/bulk-import)",
                    "↳ ingest", "↳ search indexing (seconds)"):
            self.assertIn(txt, self.html)
        self.assertNotRegex(self.html, r"(?i)indexing suite")
        self.assertNotIn("Bolivia", self.html)

    def test_docker_cpus_omitted_when_unknown(self):
        self.assertIsNone(self.data["docker_cpus"])
        self.assertNotIn("host CPUs\"", self.html.split("<script>")[-1].split("const D =")[0])

    def test_dump_is_slim_and_trend_needs_no_history(self):
        pt = self.data["legs"]["sqlite"]["point"]
        self.assertIn("sql_rows", pt["suites"])
        self.assertNotIn("shapes", self.data["legs"]["sqlite"])
        self.assertEqual(self.data["history_runs"], 0)


class TrendAndFailures(unittest.TestCase):
    def test_trend_dir_is_used(self):
        with tempfile.TemporaryDirectory() as t:
            cur, hist = os.path.join(t, "cur"), os.path.join(t, "hist")
            make_leg(cur, "sqlite", False)
            first = os.path.join(t, "first.json")
            r = run(["--run-dir", cur, "--run-id", "700", "--branch", "feature/x", "--out", os.path.join(t, "a.html"),
                     "--dump-json", first, "--no-shapes"])
            self.assertEqual(r.returncode, 0)
            os.makedirs(os.path.join(hist, "700"))
            os.replace(first, os.path.join(hist, "700", "report-data.json"))
            out, dump = os.path.join(t, "b.html"), os.path.join(t, "b.json")
            run(["--run-dir", cur, "--run-id", "777", "--branch", "feature/x", "--out", out, "--dump-json", dump,
                 "--trend-dir", hist, "--no-shapes"])
            d = json.load(open(dump))
            self.assertEqual(d["history_runs"], 1)
            with open(out, encoding="utf-8") as fh:
                self.assertIn('"trend":[', fh.read())

    def test_garbage_input_still_yields_page(self):
        with tempfile.TemporaryDirectory() as t:
            junk = os.path.join(t, "fhir-benchmark-x-1", "x")
            put(junk, "crud.json", "{not json")
            put(junk, "bulk-import.txt", "\x00\xff garbage\n===")
            put(junk, "host-contention.txt", "nonsense")
            out = os.path.join(t, "o.html")
            r = run(["--run-dir", t, "--run-id", "1", "--out", out, "--dump-json", os.path.join(t, "o.json")])
            self.assertEqual(r.returncode, 0, r.stderr)
            html = open(out, encoding="utf-8").read()
            self.assertTrue(html.startswith("<!doctype html>"))
            self.assertTrue(html.rstrip().endswith("</html>"))

    def test_missing_run_dir_and_bad_args(self):
        with tempfile.TemporaryDirectory() as t:
            out = os.path.join(t, "o.html")
            r = run(["--run-dir", os.path.join(t, "nope"), "--out", out])
            self.assertEqual(r.returncode, 0)
            self.assertIn("no leg artifacts", open(out, encoding="utf-8").read())
            r = run(["--bogus-flag", "--out", out])
            self.assertEqual(r.returncode, 0)

    def test_render_failure_writes_minimal_page(self):
        import importlib.util
        spec = importlib.util.spec_from_file_location("report_html", SCRIPT)
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        page = mod.minimal_page("9", "boom <b>")
        self.assertIn("&lt;b&gt;", page)
        self.assertTrue(page.rstrip().endswith("</html>"))


@unittest.skipUnless(os.path.isdir(REAL), "real CI run not downloaded")
class RealRun(unittest.TestCase):
    def test_smoke(self):
        with tempfile.TemporaryDirectory() as t:
            out, dump = os.path.join(t, "r.html"), os.path.join(t, "d.json")
            r = run(["--run-dir", REAL, "--run-id", "40335047310", "--repo", "HeliosSoftware/hfs",
                     "--branch", "tmp/bench-steve-ci-test", "--sha", "a4f41e7", "--out", out,
                     "--dump-json", dump, "--no-shapes"])
            self.assertEqual(r.returncode, 0, r.stderr)
            html = open(out, encoding="utf-8").read()
            d = json.load(open(dump))
            self.assertIn("postgres-elasticsearch", d["order"])
            b = d["legs"]["postgres-elasticsearch"]["bulk"]
            self.assertTrue(b["index_inline"])
            self.assertEqual(len(d["legs"]["postgres-elasticsearch"]["suites"]["sql-run"]["views"]), 5)
            self.assertIn("inline (during ingest)", html)
            self.assertLess(len(html), 400_000)


if __name__ == "__main__":
    unittest.main()
