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
REAL = os.environ.get("FHIR_BENCH_REAL_RUN_DIR", "")


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

    def test_no_literal_run_id_from_a_past_run(self):
        self.assertNotIn("39657850695", self.html)

    def test_docker_cpus_omitted_when_unknown(self):
        self.assertIsNone(self.data["docker_cpus"])
        self.assertNotIn("host CPUs\"", self.html.split("<script>")[-1].split("const D =")[0])

    def test_dump_is_slim_and_trend_needs_no_history(self):
        pt = self.data["legs"]["sqlite"]["point"]
        self.assertIn("sql_rows", pt["suites"])
        self.assertNotIn("shapes", self.data["legs"]["sqlite"])
        self.assertEqual(self.data["history_runs"], 0)


class MissingInputs(unittest.TestCase):
    def build(self, mutate, vus=None):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        d = make_leg(tmp.name, "sqlite", False)
        mutate(d)
        out, dump = os.path.join(tmp.name, "r.html"), os.path.join(tmp.name, "d.json")
        p = run(["--run-dir", tmp.name, "--run-id", "777", "--out", out, "--dump-json", dump, "--no-shapes"])
        self.assertEqual(p.returncode, 0, p.stderr)
        with open(out, encoding="utf-8") as fh:
            html = fh.read()
        with open(dump, encoding="utf-8") as fh:
            return html, json.load(fh)

    def leg(self, data):
        return data["legs"]["sqlite"]

    def test_missing_checks_is_na_not_ok(self):
        def m(d):
            put(d, "crud.json", json.dumps({"metrics": {k: v for k, v in json.loads(k6())["metrics"].items() if k != "checks"}}))
        _, data = self.build(m)
        self.assertIsNone(self.leg(data)["suites"]["crud"]["fail"])
        st, txt = self.leg(data)["checks"]["err"]
        self.assertEqual(st, "na")
        self.assertIn("n/a", txt)
        self.assertNotEqual(txt, "0 %")

    def test_missing_resources_failed_is_na(self):
        def m(d):
            p = os.path.join(d, "bulk-import.txt")
            with open(p) as fh:
                t = fh.read().replace("resources_failed=0\n", "")
            put(d, "bulk-import.txt", t)
        _, data = self.build(m)
        self.assertIsNone(self.leg(data)["bulk"]["failed"])
        self.assertEqual(self.leg(data)["checks"]["err"][0], "na")

    def test_measured_zero_checks_stay_ok(self):
        _, data = self.build(lambda d: None)
        self.assertEqual(self.leg(data)["suites"]["crud"]["fail"], 0)
        self.assertEqual(self.leg(data)["checks"]["err"], ["ok", "0 %"])

    def test_no_host_contention_no_fake_midnight(self):
        _, data = self.build(lambda d: os.remove(os.path.join(d, "host-contention.txt")))
        self.assertIsNone(self.leg(data)["start"])
        self.assertIsNone(data["date"])
        html, _ = self.build(lambda d: os.remove(os.path.join(d, "host-contention.txt")))
        self.assertNotIn("T00:00:00Z", html)
        self.assertIn('D.date?fdate(D.date,true):"unknown"', html)

    def test_prose_numbers_not_literals(self):
        html, data = self.build(lambda d: put(d, "import.json", k6(extra={"vus_max": {"max": 37}})))
        self.assertEqual(self.leg(data)["suites"]["import"]["vus"], 37)
        for lit in ("out of 1,000", "20 concurrent", "300 VUs"):
            self.assertNotIn(lit, html)
        self.assertIn('id="proseImpVus"', html)
        self.assertIn("sv(\"import\")", html)


class PgContention(unittest.TestCase):
    PGSTAT = " total_s | mean_ms | calls\n 3500.0 | 1.0 | 1000000\n"

    def test_rule_applies_only_to_leg_named_postgres(self):
        with tempfile.TemporaryDirectory() as t:
            for leg, es in (("postgres", False), ("postgres-elasticsearch", True)):
                d = make_leg(t, leg, es)
                put(d, "crud-pgstat.txt", self.PGSTAT)  # 3.5 ms/call
            out, dump = os.path.join(t, "r.html"), os.path.join(t, "d.json")
            p = run(["--run-dir", t, "--run-id", "777", "--out", out, "--dump-json", dump, "--no-shapes"])
            self.assertEqual(p.returncode, 0, p.stderr)
            with open(dump, encoding="utf-8") as fh:
                legs = json.load(fh)["legs"]
            with open(out, encoding="utf-8") as fh:
                html = fh.read()
            self.assertNotIn("39657850695", html)
            pg, pe = legs["postgres"], legs["postgres-elasticsearch"]
            self.assertEqual(pg["checks"]["load"][0], "bad")
            self.assertIn("⚠", pg["markers"].get("crud", ""))
            self.assertIn("3.50 ms/call", pe["checks"]["load"][1])  # value still shown
            self.assertNotEqual(pe["checks"]["load"][0], "bad")
            self.assertNotIn("⚠", pe["markers"].get("crud", ""))


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


@unittest.skipUnless(bool(REAL) and os.path.isdir(REAL),
                     "set FHIR_BENCH_REAL_RUN_DIR to a downloaded CI run directory")
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


class Timing(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        cls.t = cls.tmp.name
        make_leg(cls.t, "sqlite", False)

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def timing(self, waiting, working):
        job = {"name": "leg sqlite", "kind": "leg", "leg": "sqlite", "queued_s": 5, "behind_legs_s": 77,
               "capacity_wait_s": 3, "host_wait_s": 0, "waiting_s": waiting, "working_s": working,
               "created": "2026-10-26T10:00:00Z", "started": "2026-10-26T10:01:00Z",
               "completed": "2026-10-26T11:00:00Z", "running": False}
        return {"run_id": "777", "total_s": waiting + working, "waiting_s": waiting, "working_s": working,
                "jobs": [job]}

    def build(self, tag, timing=None, raw=None):
        out, dump = os.path.join(self.t, tag + ".html"), os.path.join(self.t, tag + ".json")
        args = ["--run-dir", self.t, "--run-id", "777", "--out", out, "--dump-json", dump, "--no-shapes"]
        if timing is not None or raw is not None:
            tj = os.path.join(self.t, tag + "-t.json")
            with open(tj, "w", encoding="utf-8") as fh:
                fh.write(raw if raw is not None else json.dumps(timing))
            args += ["--timing-json", tj]
        r = run(args)
        self.assertEqual(r.returncode, 0, r.stderr)
        return open(out, encoding="utf-8").read(), json.load(open(dump))

    def test_tile_from_data(self):
        html, d = self.build("ok", self.timing(11, 3195))
        self.assertIn("Waiting 11 s", html)
        self.assertIn("Working 53 min 15 s", html)
        self.assertIn("of 53 min 26 s total", html)
        self.assertEqual(d["timing"]["waiting_s"], 11)
        self.assertNotIn("waited", json.dumps(d["verdict_all"]))
        self.assertNotIn("timing_view", d)

    def test_warning_sentence(self):
        html, d = self.build("warn", self.timing(3900, 3195))
        self.assertIn("This run waited 1 h 05 min for resources: it is slower to finish, not slower to run.", html)
        self.assertTrue(any("waited 1 h 05 min" in i[1] for i in d["verdict_all"]["items"]))

    def test_invalid_or_absent_is_identical(self):
        base, bd = self.build("base")
        for tag, raw in (("bad1", "{not json"), ("bad2", json.dumps({"run_id": "1"}))):
            html, d = self.build(tag, raw=raw)
            self.assertEqual(html, base)
            self.assertNotIn("timing", d)
        self.assertNotIn("timing", bd)

    def test_template_has_no_fixture_numbers(self):
        with open(os.path.join(HERE, "report_template.html"), encoding="utf-8") as fh:
            tpl = fh.read()
        for lit in ("3195", "3206", "53 min", "1 h 05", "Waiting 11"):
            self.assertNotIn(lit, tpl)


class LegacyTrend(unittest.TestCase):
    """Pure-Python side of the trend: index mode, failed rebuild, --trend-max, legacy-point.json."""
    @classmethod
    def setUpClass(cls):
        sys.path.insert(0, HERE)
        import report_html
        cls.R = report_html

    def bulk(self, **kw):
        b = {"status": "complete", "searchable": True, "failed": 0, "submitted": 10, "index_inline": False,
             "index_status": "deferred", "ingest": 1.0, "index": 1.0, "index_eff": 1.0, "total": 2.0,
             "rps": 5.0, "ingest_rps": 5.0}
        b.update(kw)
        return b

    def leg(self, bulk):
        return {"leg": "sqlite", "ic": None, "suites": {}, "died": False, "drain": None, "pg_ms": None,
                "bulk": bulk, "host": [], "start": "2026-10-27T10:00:00Z", "sha": "abc", "ref": "main",
                "runner": "r", "markers": {}, "run": "9"}

    def test_failed_rebuild_is_marked(self):
        self.assertIn("‡", self.R.markers(self.leg(self.bulk(index_status="failed")))["bulk"])
        self.assertNotIn("‡", self.R.markers(self.leg(self.bulk()))["bulk"])

    def test_trend_panel_hollows_failed_rebuild_and_cuts_line(self):
        t = open(os.path.join(HERE, "report_template.html"), encoding="utf-8").read()
        fn = t[t.index("function trendPanel"):t.index("const trendOf")]
        self.assertIn('idxSeries = key==="bulk" || key==="bulk_index"', fn)
        self.assertIn('!bad(T[i-1])&&!bad(T[i])', fn)
        self.assertIn('fill="var(--surface)" stroke="${col}"', fn)
        self.assertIn('m += "‡"', fn)

    def test_trend_point_carries_index_mode(self):
        pt = self.R.trend_point("sqlite", self.leg(self.bulk(index_inline=True, index_status="inline")))
        self.assertIs(pt["index_inline"], True)
        self.assertEqual(pt["index_status"], "inline")

    def hist(self, root, rid, fname, point):
        os.makedirs(os.path.join(root, rid))
        with open(os.path.join(root, rid, fname), "w", encoding="utf-8") as fh:
            json.dump({"run": rid, "legs": {"sqlite": {"point": point}}}, fh)

    def pt(self, rid, source=None, inline=None):
        p = {"run": rid, "date": "2026-10-%sT10:00:00Z" % rid[-2:], "branch": "main", "suites": {"import": {"rps": 1, "p95": 2}},
             "bulk": {"rps": 5, "total": 2, "ingest_rps": 3, "ingest": 4, "index_eff": 6}, "index_inline": inline}
        if source:
            p["source"] = source
        return p

    def test_load_history_reads_legacy_only_when_no_report_data(self):
        with tempfile.TemporaryDirectory() as t:
            self.hist(t, "10", "legacy-point.json", self.pt("10", "comparison.md", False))
            self.hist(t, "11", "report-data.json", self.pt("11", None, True))
            self.hist(t, "12", "legacy-point.json", self.pt("12", "comparison.md", False))
            with open(os.path.join(t, "12", "report-data.json"), "w", encoding="utf-8") as fh:
                json.dump({"run": "12", "legs": {"sqlite": {"point": self.pt("12", None, True)}}}, fh)
            h = dict(self.R.load_history(t))
            self.assertEqual(sorted(h), ["10", "11", "12"])
            self.assertEqual(h["10"]["legs"]["sqlite"]["point"]["source"], "comparison.md")
            self.assertNotIn("source", h["12"]["legs"]["sqlite"]["point"])

    def test_trend_max_limits_points(self):
        hist = [(str(i), {"legs": {"sqlite": {"point": self.pt(str(i))}}}) for i in range(20, 30)]
        cur = self.pt("40")
        _, tr, _ = self.R.leg_trend("sqlite", cur, hist, "40", trend_max=4)
        self.assertEqual([t["run"] for t in tr], ["27", "28", "29", "40"])
        _, tr, _ = self.R.leg_trend("sqlite", cur, hist, "40")
        self.assertEqual(len(tr), self.R.TREND_MAX)

    def test_trend_max_cli_arg(self):
        with tempfile.TemporaryDirectory() as t:
            cur, hist = os.path.join(t, "cur"), os.path.join(t, "hist")
            make_leg(cur, "sqlite", False)
            for i in range(20, 26):
                self.hist(hist, str(i), "report-data.json", self.pt(str(i)))
            out = os.path.join(t, "o.html")
            dump = os.path.join(t, "d.json")
            r = run(["--run-dir", cur, "--run-id", "777", "--out", out, "--trend-dir", hist, "--trend-max", "3",
                     "--no-shapes", "--dump-json", dump])
            self.assertEqual(r.returncode, 0, r.stderr)
            self.assertEqual(open(out, encoding="utf-8").read().count('"run":"24"'), 1)
            self.assertEqual(json.load(open(dump))["history_runs"], 6)

    def refs(self, cur, pts):
        hist = [(p["run"], {"legs": {"sqlite": {"point": p}}}) for p in pts]
        return self.R.leg_trend("sqlite", cur, hist, cur["run"])

    def test_reference_is_nearest_earlier_point_with_value(self):
        a = self.pt("21", None, False)
        b = self.pt("22", "comparison.md", False)
        b["suites"] = {"crud": {"rps": 9, "p95": 1}}  # no import value
        prev, _, by = self.refs(self.pt("30", None, False), [a, b])
        self.assertEqual(prev["run"], "22")          # immediately previous run
        self.assertEqual(by["crud"]["run"], "22")
        self.assertEqual(by["import"]["run"], "21")  # skipped 22: no value
        self.assertEqual(by["bulk.rps"]["run"], "22")
        self.assertIsNone(by["search"])

    def test_ingest_and_index_need_same_mode_but_others_do_not(self):
        old = self.pt("21", None, False)
        newer = self.pt("22", None, True)
        cur = self.pt("30", None, False)
        _, _, by = self.refs(cur, [old, newer])
        for m in ("bulk.ingest_rps", "bulk.ingest", "bulk.index_eff"):
            self.assertEqual(by[m]["run"], "21", m)   # older, same mode
        for m in ("import", "bulk.rps", "bulk.total"):
            self.assertEqual(by[m]["run"], "22", m)   # mode does not matter
        _, _, by = self.refs(self.pt("30", None, True), [old, newer])
        self.assertEqual(by["bulk.index_eff"]["run"], "22")

    def test_no_same_mode_reference_gives_none(self):
        _, _, by = self.refs(self.pt("30", None, True), [self.pt("21", None, False)])
        self.assertIsNone(by["bulk.index_eff"])
        self.assertIsNone(by["bulk.ingest_rps"])
        self.assertEqual(by["import"]["run"], "21")
        self.assertEqual(by["bulk.rps"]["run"], "21")
        # unknown mode only matches unknown
        _, _, by = self.refs(self.pt("30", None, None), [self.pt("21", None, False), self.pt("22", None, None)])
        self.assertEqual(by["bulk.ingest"]["run"], "22")
        _, _, by = self.refs(self.pt("30", None, None), [self.pt("21", None, False)])
        self.assertIsNone(by["bulk.ingest"])
        # no history at all
        prev, _, by = self.refs(self.pt("30", None, False), [])
        self.assertIsNone(prev)
        self.assertTrue(all(v is None for v in by.values()))

    def test_failed_rebuild_is_never_a_reference_for_index_metrics(self):
        bad = self.pt("22", None, False)
        bad["index_status"] = "failed"
        good = self.pt("21", None, False)
        _, _, by = self.refs(self.pt("30", None, False), [good, bad])
        self.assertEqual(by["bulk.index_eff"]["run"], "21")
        # the ingest of a run whose rebuild failed is still valid; the until-searchable numbers are not
        self.assertEqual(by["bulk.ingest_rps"]["run"], "22")
        self.assertEqual(by["bulk.ingest"]["run"], "22")
        self.assertEqual(by["bulk.rps"]["run"], "21")
        self.assertEqual(by["bulk.total"]["run"], "21")
        self.assertEqual(by["import"]["run"], "22")
        _, _, by = self.refs(self.pt("30", None, False), [bad])
        self.assertIsNone(by["bulk.index_eff"])
        self.assertIsNone(by["bulk.rps"])
        self.assertEqual(by["bulk.ingest_rps"]["run"], "22")

    def test_cancelled_rebuild_is_treated_like_failed(self):
        self.assertIn("‡", self.R.markers(self.leg(self.bulk(index_status="cancelled")))["bulk"])
        bad = self.pt("22", None, False)
        bad["index_status"] = "cancelled"
        good = self.pt("21", None, False)
        _, _, by = self.refs(self.pt("30", None, False), [good, bad])
        self.assertEqual(by["bulk.index_eff"]["run"], "21")
        self.assertEqual(by["bulk.rps"]["run"], "21")
        self.assertEqual(by["bulk.ingest_rps"]["run"], "22")
        p = self.pt("23", None, False)
        p.pop("index_inline")
        hist = [("23", {"legs": {"sqlite": {"point": p, "bulk": self.bulk(index_status="cancelled")}}})]
        _, tr, by = self.R.leg_trend("sqlite", self.pt("30", None, False), hist, "30")
        self.assertIsNone(by["bulk.index_eff"])
        self.assertIn("‡", [t for t in tr if t["run"] == "23"][0]["markers"]["bulk"])

    def test_host_changes_listed_only_when_span_crosses_the_change(self):
        def build(dates):
            with tempfile.TemporaryDirectory() as t:
                cur, hist = os.path.join(t, "cur"), os.path.join(t, "hist")
                make_leg(cur, "sqlite", False)
                for i, d in enumerate(dates):
                    p = self.pt(str(20 + i))
                    p["date"] = d
                    self.hist(hist, str(20 + i), "report-data.json", p)
                dump = os.path.join(t, "d.json")
                r = run(["--run-dir", cur, "--run-id", "777", "--out", os.path.join(t, "o.html"),
                         "--trend-dir", hist, "--no-shapes", "--dump-json", dump])
                self.assertEqual(r.returncode, 0, r.stderr)
                return json.load(open(dump))["host_changes"]
        crossing = build(["2026-10-20T10:00:00Z", "2026-10-29T09:00:00Z"])
        self.assertEqual(len(crossing), 1)
        import host_changes
        self.assertEqual(crossing[0]["at"], host_changes.HOST_CHANGES[0]["at"])
        self.assertEqual(crossing[0]["legend"], host_changes.legend(host_changes.HOST_CHANGES[0]))
        self.assertEqual(crossing[0]["approximate"], bool(host_changes.HOST_CHANGES[0].get("approximate")))
        self.assertIn("older Docker host", crossing[0]["legend"])
        self.assertEqual(build(["2026-10-29T09:00:00Z", "2026-10-30T09:00:00Z"]), [])
        self.assertEqual(build(["2026-10-20", "2026-10-21"]), [])

    def test_template_host_change_and_es_only_era_label(self):
        tpl = open(os.path.join(HERE, "report_template.html"), encoding="utf-8").read()
        self.assertIn("older Docker host", tpl)
        self.assertIn("hostNote", tpl)
        self.assertIn("trendHost", tpl)
        # the "before #2248" era is only for Elasticsearch legs
        for line in tpl.splitlines():
            if "before #2248" in line and "index_inline === false" in line:
                self.assertIn("isEs(", line)
        self.assertIn("Elasticsearch legs: deferred before #2248, inline since; other legs always deferred", tpl)
        self.assertIn("idxBad", tpl)

    def test_old_report_data_failed_status_is_backfilled_before_reference(self):
        p = self.pt("21", None, False)
        p.pop("index_inline")
        hist = [("21", {"legs": {"sqlite": {"point": p, "bulk": self.bulk(index_status="failed")}}})]
        _, _, by = self.R.leg_trend("sqlite", self.pt("30", None, False), hist, "30")
        self.assertIsNone(by["bulk.index_eff"])

    def test_real_27_oct_shape_backfills_mode_and_failed_marker(self):
        # report-data.json of run 40547405940: point has no index_inline / index_status, the leg's bulk block does
        def hp(rid, inline, status, mk):
            p = self.pt(rid)
            p.pop("index_inline")
            p["markers"] = mk
            bulk = self.bulk(index_inline=inline, index_status=status)
            return (rid, {"legs": {"sqlite": {"point": p, "bulk": bulk}}}), p
        (n1, d1), p1 = hp("27", False, "failed", {"bulk": ""})
        orig = dict(p1["markers"])
        cur = self.pt("30", None, False)
        prev, tr, by = self.R.leg_trend("sqlite", cur, [(n1, d1)], "30")
        t27 = [t for t in tr if t["run"] == "27"][0]
        self.assertIs(t27["index_inline"], False)
        self.assertEqual(t27["index_status"], "failed")
        self.assertIn("‡", t27["markers"]["bulk"])
        self.assertEqual(p1["markers"], orig)  # input not mutated
        self.assertNotIn("index_inline", p1)
        # failed deferred previous is no reference for index metrics, but is for import
        self.assertIsNone(by["bulk.index_eff"])
        self.assertEqual(by["import"]["run"], "27")
        # inline current vs deferred previous: no ingest / index reference
        _, _, by = self.R.leg_trend("sqlite", self.pt("30", None, True), [(n1, d1)], "30")
        self.assertIsNone(by["bulk.ingest_rps"])
        # inline previous, no markers key at all
        (n2, d2), _ = hp("28", True, "inline", {})
        d2["legs"]["sqlite"]["point"].pop("markers")
        _, tr, _ = self.R.leg_trend("sqlite", cur, [(n2, d2)], "30")
        self.assertIs([t for t in tr if t["run"] == "28"][0]["index_inline"], True)

    def test_template_uses_per_metric_reference_and_legend(self):
        tpl = open(os.path.join(HERE, "report_template.html"), encoding="utf-8").read()
        self.assertIn("prev_by", tpl)
        self.assertIn("no earlier run with the same indexing mode", tpl)
        self.assertIn("Change is against the nearest earlier run with a value", tpl)
        self.assertIn("deferred indexing (before #2248)", tpl)

    def test_template_script_is_valid_js(self):
        import shutil
        node = shutil.which("node")
        if not node:
            self.skipTest("node not available")
        tpl = open(os.path.join(HERE, "report_template.html"), encoding="utf-8").read()
        scripts = re.findall(r"<script[^>]*>(.*?)</script>", tpl, re.S)
        with tempfile.TemporaryDirectory() as t:
            f = os.path.join(t, "s.js")
            open(f, "w", encoding="utf-8").write(max(scripts, key=len))
            r = subprocess.run([node, "--check", f], capture_output=True, text=True)
            self.assertEqual(r.returncode, 0, r.stderr)


if __name__ == "__main__":
    unittest.main()
