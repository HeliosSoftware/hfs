import types
import unittest

import history

FIXTURE = """## FHIR Benchmark — leg comparison

run 39657850695 · suites: all · 6 of 6 legs have results

### Throughput (requests/s unless the row names its unit)

| Suite | sqlite | postgres | mongodb | sqlite+ES | postgres+ES | mongodb+ES |
|---|---:|---:|---:|---:|---:|---:|
| prewarm | 1,475 | **1,867** | 1,232 | 90 | 102 | 97 |
| Batch/Transaction Import, `POST /` Bundles (resources/s) | 1,122 ‡ | 4,369 | 1,201 ‡ | 6,325 | 6,141 | 4,972 ‡ |
| Bulk Import, `$bulk-submit` (resources/s) | 1,122 | 1,042 | 620 | **3,057** | 2,565 | 1,638 |
| Bulk Import: ingest (resources/s, seconds) | 4,591 (355 s) | 3,196 (510 s) | 1,752 (930 s) | **7,409** (220 s) | 5,175 (315 s) | 2,527 (645 s) |
| Bulk Import: index rebuild (seconds) | 1,099 s | 1,054 s | 1,698 s | **313 s** | 321 s | 350 s |
| crud | 1,132 † | 2,175 | 1,539 † | 995 | 879 | 875 † |
| search | 4 † | 2,186 | 22 † | 920 | 869 | 1,081 † |
| insert (creates/s) | 827 | **1,425** | 1,167 | 267 | 263 | 261 |

### p95 latency (ms)

| Suite | sqlite | postgres | mongodb | sqlite+ES | postgres+ES | mongodb+ES |
|---|---:|---:|---:|---:|---:|---:|
| prewarm | 20.6 | **11.6** | 19.9 | 292.3 | 287.7 | 304.3 |
| Bulk Import, `$bulk-submit` | n/a (one async job) | n/a (one async job) | n/a (one async job) | n/a (one async job) | n/a (one async job) | n/a (one async job) |
| crud | 902.2 † | 290.0 | 380.0 † | 749.2 | 798.6 | 825.6 † |

### Errors (Err% · failed checks)

| Suite | sqlite |
|---|---:|
| crud | 0.0% |
"""


def run(n, date="2026-10-0%d", branch="feat"):
    return {"id": 1000 + n, "number": n, "date": date % n, "branch": branch,
            "url": f"https://example/runs/{1000 + n}"}


def args(**kw):
    d = dict(repo="o/r", workflow="w.yml", branch="feat", run_id="999",
             artifact_name="a", max_runs=5)
    d.update(kw)
    d["artifact_name"] = d.get("artifact_name")
    return types.SimpleNamespace(**d)


def scaled(k):
    # scale every number in the throughput table by k via crude replace
    return FIXTURE.replace("| crud | 1,132 † |", f"| crud | {int(1132 * k):,} † |")


class T(unittest.TestCase):
    def test_parse_cells(self):
        p = history.parse_cell
        self.assertEqual(p("1,122 ‡"), 1122.0)
        self.assertEqual(p("**3,057**"), 3057.0)
        self.assertEqual(p("4 †"), 4.0)
        self.assertEqual(p("4,591 (355 s)"), 4591.0)
        self.assertEqual(p("1,099 s"), 1099.0)
        self.assertIsNone(p("n/a (one async job)"))
        self.assertIsNone(p(""))

    def test_parse_tables(self):
        t = history.parse_tables(FIXTURE)
        self.assertEqual(len(t["throughput"]["legs"]), 6)
        self.assertEqual(len(t["throughput"]["rows"]), 8)
        self.assertEqual(t["throughput"]["rows"][1][1][0], 1122.0)
        self.assertIn("p95", t)
        self.assertIsNone(t["p95"]["rows"][1][1][0])

    def _fetch(self, missing=()):
        def fr(*a):
            return [run(i) for i in (1, 2, 3)]

        def fc(repo, r, name):
            if r["number"] in missing:
                return None
            return scaled(r["number"])
        return fr, fc

    def test_missing_artifact_skipped(self):
        fr, fc = self._fetch(missing=(1, 2))
        out = history.build(args(), fr, fc, FIXTURE)
        self.assertIn("Progress over the last 2 runs", out)
        self.assertNotIn("#1,", out)

    def test_fewer_than_two(self):
        fr, fc = self._fetch(missing=(1, 2, 3))
        out = history.build(args(), fr, fc, None)
        self.assertIn("fewer than 2", out)
        self.assertNotIn("<details>", out)
        out = history.build(args(), lambda *a: [], fc, FIXTURE)
        self.assertIn("fewer than 2", out)

    def test_fetch_errors_swallowed(self):
        def boom(*a):
            raise RuntimeError("net")
        out = history.build(args(), boom, boom, FIXTURE)
        self.assertIn("fewer than 2", out)
        fr, _ = self._fetch()
        out = history.build(args(), fr, boom, FIXTURE)
        self.assertIn("fewer than 2", out)

    def test_charts_capped_and_shape(self):
        fr, fc = self._fetch()
        out = history.build(args(), fr, fc, FIXTURE)
        n = out.count("```mermaid")
        self.assertTrue(1 <= n <= history.MAX_CHARTS)
        self.assertEqual(n, 5)  # prewarm, sub-row and seconds row excluded
        self.assertIn("xychart-beta", out)
        self.assertNotIn('title "prewarm', out)
        self.assertIn("(+", out)  # change vs previous run
        self.assertLess(len(out.encode()), history.MAX_BYTES)

    def test_chart_cap_many_rows(self):
        rows = "\n".join(f"| row{i} | {i + 1} | {i + 2} |" for i in range(12))
        md = "### Throughput\n\n| Suite | a | b |\n|---|---:|---:|\n" + rows + "\n"
        out = history.build(args(), lambda *a: [run(1), run(2)],
                            lambda *a: md, md)
        self.assertEqual(out.count("```mermaid"), history.MAX_CHARTS)

    def test_deterministic(self):
        fr, fc = self._fetch()
        a = history.build(args(), fr, fc, FIXTURE)
        b = history.build(args(), fr, fc, FIXTURE)
        self.assertEqual(a, b)

    def test_artifact_name_gets_run_id_suffix(self):
        import unittest.mock as m
        calls = []

        def fake(a):
            calls.append(a)
            return types.SimpleNamespace(returncode=1, stdout="", stderr="")
        with m.patch.object(history, "_gh", fake):
            self.assertIsNone(history.fetch_comparison("o/r", run(7), "pre"))
        self.assertIn("pre-1007", calls[0])
        self.assertIn("1007", calls[0])

    def test_alias_join_and_cell(self):
        old = FIXTURE
        new = FIXTURE.replace(
            "Batch/Transaction Import, `POST /` Bundles (resources/s)", history.NEW_BATCH
        ).replace("| Bulk Import: index rebuild (seconds) | 1,099 s",
                  "| ↳ search indexing (seconds) | 195 s (= ingest)")
        t = history.parse_tables(old)["throughput"]["rows"]
        self.assertIn(history.NEW_BATCH, [r[0] for r in t])
        self.assertIn(history.NEW_INDEX, [r[0] for r in t])
        self.assertEqual(history.parse_cell("195 s (= ingest)"), 195.0)
        runs = [run(1), run(2)]
        out = history.build(args(), lambda *a: runs,
                            lambda repo, r, n: old if r["number"] == 1 else new, new)
        self.assertEqual(out.count("**" + history.NEW_BATCH + "**"), 1)
        self.assertNotIn("Bundles (resources/s)**\n\n| Leg" + "x", out)
        self.assertNotIn("**Batch/Transaction Import, `POST", out)
        self.assertIn("1,122 (", out)
        # old rebuild joins the new indexing row on non-ES legs only
        self.assertEqual(out.count("**" + history.NEW_INDEX + "**"), 1)
        sec = out.split("**" + history.NEW_INDEX + "**")[1].split("**")[0]
        self.assertIn("| `sqlite` | 1,099 | 195 (-82%) | 195 (+0%) |", sec)
        self.assertIn("| `sqlite+ES` | – | 313 |", sec)  # deferred-era ES cell is blanked

    def test_pct_sign_and_missing(self):
        self.assertEqual(history.pct(100, 80), "-20%")  # seconds: lower is better
        self.assertEqual(history.pct(None, 5), "")
        self.assertEqual(history.pct(0, 5), "")

    def test_truncation_keeps_fences_balanced(self):
        secs = ["head"] + ["```mermaid\nx\n```\n" + "y" * 300 for _ in range(50)]
        text = history._finish(secs, max_bytes=3000)
        self.assertEqual(text.count("```") % 2, 0)
        self.assertIn("output cut", text)
        self.assertTrue(text.rstrip().endswith("</details>"))
        self.assertLess(len(text.encode()), 3000)
        self.assertTrue(history._finish(["```a", "b"]).count("```") % 2 == 0)

    def test_failed_run_with_artifact_included(self):
        import unittest.mock as m
        payload = {"workflow_runs": [
            {"id": 5, "conclusion": "failure", "head_branch": "feat"},
            {"id": 6, "conclusion": "cancelled", "head_branch": "feat"},
            {"id": 7, "conclusion": "success", "head_branch": "feat"}]}
        fake = lambda a: types.SimpleNamespace(returncode=0, stdout=__import__("json").dumps(payload))
        with m.patch.object(history, "_gh", fake):
            got = history._list_runs("o/r", "w.yml", "feat", 5)
        self.assertEqual([x["id"] for x in got], [5, 7])

    def test_suites_mismatch_skipped_unparseable_kept(self):
        other = FIXTURE.replace("suites: all", "suites: crud")
        fr = lambda *a: [run(1), run(2, branch="main"), run(3)]
        fc = lambda repo, r, n: other if r["number"] != 3 else FIXTURE
        out = history.build(args(), fr, fc, FIXTURE)
        self.assertIn("last 3 runs", out)  # run 1 skipped, main kept, 3 + current
        nohdr = FIXTURE.replace("suites: all", "")
        out = history.build(args(), lambda *a: [run(1)], lambda *a: other, nohdr)
        self.assertIn("last 2 runs", out)

    def test_data_dir_downloads_each_run(self):
        calls = []
        fr, fc = self._fetch()
        history.build(args(data_dir="D"), fr, fc, FIXTURE,
                      lambda repo, r, d: calls.append((r["id"], d)))
        self.assertEqual(calls, [(1001, "D"), (1002, "D"), (1003, "D")])

    def test_single_series_chart_shape(self):
        fr, fc = self._fetch()
        out = history.build(args(), fr, fc, FIXTURE)
        self.assertNotIn("Lines in order", out)
        self.assertEqual(out.count("    line ["), out.count("```mermaid"))
        self.assertIn('title "crud - sqlite"', out)
        c = history.render_chart("r", "leg", [1.0, None], ["a", "b"])
        self.assertEqual(c, "")

    def test_main_exit_zero(self):
        self.assertEqual(history.main(["--bogus"]), 0)



import json
import contextlib
import io
import os
import tempfile
import unittest.mock as mock

TESTDATA = os.path.join(os.path.dirname(os.path.abspath(__file__)), "legacy_testdata")


def fixture(run_id):
    with open(os.path.join(TESTDATA, f"{run_id}.md"), encoding="utf-8") as f:
        return f.read()


def have_fixtures():
    return os.path.isdir(TESTDATA)


class LegacyAliases(unittest.TestCase):
    def rows(self, run_id):
        t = history.parse_tables(fixture(run_id))["throughput"]
        return t["legs"], {l: v for l, v in t["rows"]}

    @unittest.skipUnless(have_fixtures(), "legacy_testdata missing")
    def test_38501925223_import_bulk_and_no_reindex(self):
        legs, rows = self.rows(38501925223)
        self.assertEqual(rows[history.NEW_BATCH][0], 1190.0)
        self.assertEqual(rows[history.NEW_BULK][0], 1239.0)
        self.assertIsNone(rows[history.NEW_BATCH][legs.index("postgres")])
        self.assertIn("reindex, `$reindex` Encounter (resources/s)", rows)  # kept apart
        self.assertNotIn(history.NEW_INDEX, rows)

    @unittest.skipUnless(have_fixtures(), "legacy_testdata missing")
    def test_38751206249_deferred_index_non_es_only(self):
        legs, rows = self.rows(38751206249)
        i = legs.index("sqlite")
        self.assertEqual(rows[history.NEW_INDEX][i], 1541.0)
        self.assertEqual(rows[history.NEW_INGEST][i], 2547.0)
        for k, lg in enumerate(legs):
            if "ES" in lg:
                self.assertIsNone(rows[history.NEW_INDEX][k])
                self.assertIsNone(rows[history.NEW_INGEST][k])
        # bulk end-to-end stays valid on ES legs
        self.assertEqual(rows[history.NEW_BULK][legs.index("sqlite+ES")], 1564.0)

    @unittest.skipUnless(have_fixtures(), "legacy_testdata missing")
    def test_39657850695_index_joins_new_label(self):
        legs, rows = self.rows(39657850695)
        self.assertEqual(rows[history.NEW_INDEX][legs.index("sqlite")], 1099.0)
        self.assertIsNone(rows[history.NEW_INDEX][legs.index("mongodb+ES")])
        self.assertEqual(rows[history.NEW_INGEST][legs.index("sqlite")], 4591.0)

    @unittest.skipUnless(have_fixtures(), "legacy_testdata missing")
    def test_37986575337_9_oct_bulk_row_aliased(self):
        legs, rows = self.rows(37986575337)
        self.assertEqual(rows[history.NEW_BULK][0], 1455.0)
        self.assertEqual(rows[history.NEW_BATCH][0], 1330.0)
        self.assertNotIn("bulk-import (resources/s)", rows)

    def test_search_index_rebuild_after_completed_label(self):
        lab = "↳ search-index rebuild after Completed (seconds)"
        md = ("### Throughput\n\n| Suite | sqlite | sqlite+ES |\n|---|---:|---:|\n"
              "| ↳ ingest (resources/s, seconds) | 4,125 (395 s) | 3,327 (490 s) |\n"
              f"| {lab} | 1,655 s | 490 s |\n")
        self.assertEqual(history.canon_label(lab), history.NEW_INDEX)
        rows = dict(history.parse_tables(md)["throughput"]["rows"])
        self.assertEqual(rows[history.NEW_INDEX], [1655.0, None])
        self.assertEqual(rows[history.NEW_INGEST], [4125.0, None])

    def test_new_format_not_blanked(self):
        md = ("### Throughput\n\n| Suite | sqlite | sqlite+ES |\n|---|---:|---:|\n"
              f"| {history.NEW_INGEST} | 4,125 (395 s) | 3,327 (490 s) |\n"
              f"| {history.NEW_INDEX} | 1,655 s | 490 s (= ingest) |\n")
        rows = dict(history.parse_tables(md)["throughput"]["rows"])
        self.assertEqual(rows[history.NEW_INDEX], [1655.0, 490.0])
        self.assertEqual(rows[history.NEW_INGEST], [4125.0, 3327.0])

    def test_reindex_label_never_aliased(self):
        for lab in ("reindex, `$reindex` Encounter (resources/s)",
                    "Indexing ($reindex of Encounter)"):
            self.assertEqual(history.canon_label(lab), lab)


class RunSelection(unittest.TestCase):
    def test_runs_without_artifact_do_not_use_a_slot(self):
        payload = {"workflow_runs": [
            {"id": i, "conclusion": "success", "head_branch": "main"} for i in range(10, 0, -1)]}

        def fake(a):
            if a[1].endswith("/artifacts?per_page=100"):
                rid = int(a[1].split("/runs/")[1].split("/")[0])
                arts = [] if rid % 2 else [{"name": f"fhir-bench-comparison-{rid}"}]
                return types.SimpleNamespace(returncode=0, stdout=json.dumps({"artifacts": arts}))
            return types.SimpleNamespace(returncode=0, stdout=json.dumps(payload))
        with mock.patch.object(history, "_gh", fake):
            got = history.fetch_runs("o/r", "w.yml", "main", 99, 3)
        self.assertEqual([r["id"] for r in got], [10, 8, 6])

    def test_only_benchmark_data_artifacts_count(self):
        def has(names):
            arts = {"artifacts": [{"name": n} for n in names]}
            fake = lambda a: types.SimpleNamespace(returncode=0, stdout=json.dumps(arts))
            with mock.patch.object(history, "_gh", fake):
                return history.has_artifact("o/r", 1)
        self.assertFalse(has(["fhir-bench-server-logs-1", "fhir-bench-sqlite-1"]))
        self.assertTrue(has(["fhir-bench-server-logs-1", "fhir-bench-comparison-1"]))
        self.assertTrue(has(["fhir-bench-report-data-1"]))

    def test_expired_artifact_counts_as_none(self):
        arts = {"artifacts": [{"name": "fhir-bench-comparison-1", "expired": True}]}
        fake = lambda a: types.SimpleNamespace(returncode=0, stdout=json.dumps(arts))
        with mock.patch.object(history, "_gh", fake):
            self.assertFalse(history.has_artifact("o/r", 1))


class LegacyFetch(unittest.TestCase):
    RUN = {"id": 77, "date": "2026-10-14", "branch": "main", "conclusion": "failure"}

    def _gh_fail(self, a):
        return types.SimpleNamespace(returncode=1, stdout="", stderr="")

    def _fake_module(self, expires="2999-01-01"):
        calls = []

        def point_doc(text, run_id, date, branch, partial_run=False):
            calls.append((run_id, date, branch, partial_run))
            return {"source": "comparison.md", "run": run_id, "legs": {}}
        return types.SimpleNamespace(point_doc=point_doc, EXPIRES=expires), calls

    def test_writes_legacy_point_when_no_report_data(self):
        mod, calls = self._fake_module()
        with tempfile.TemporaryDirectory() as d, \
                mock.patch.dict("sys.modules", {"legacy_points": mod}), \
                mock.patch.object(history, "_gh", self._gh_fail), \
                mock.patch.object(history, "fetch_comparison", lambda *a: "md"):
            history.fetch_data("o/r", self.RUN, d)
            with open(os.path.join(d, "77", "legacy-point.json")) as f:
                self.assertEqual(json.load(f)["run"], "77")
        self.assertEqual(calls, [("77", "2026-10-14", "main", True)])

    def test_not_called_when_report_data_downloaded(self):
        mod, calls = self._fake_module()
        ok = lambda a: types.SimpleNamespace(returncode=0, stdout="", stderr="")
        with tempfile.TemporaryDirectory() as d, \
                mock.patch.dict("sys.modules", {"legacy_points": mod}), \
                mock.patch.object(history, "_gh", ok):
            history.fetch_data("o/r", self.RUN, d)
            self.assertFalse(os.path.exists(os.path.join(d, "77", "legacy-point.json")))
        self.assertEqual(calls, [])

    def test_flag_off_and_expired_skip(self):
        mod, calls = self._fake_module()
        gone = self._fake_module(expires="2000-01-01")[0]
        with tempfile.TemporaryDirectory() as d, \
                mock.patch.object(history, "_gh", self._gh_fail), \
                mock.patch.object(history, "fetch_comparison", lambda *a: "md"):
            with mock.patch.dict("sys.modules", {"legacy_points": mod}):
                history.fetch_data("o/r", self.RUN, d, legacy=False)
            history._EXPIRY_WARNED = False
            out = io.StringIO()
            with mock.patch.dict("sys.modules", {"legacy_points": gone}), \
                    contextlib.redirect_stdout(out):
                history.fetch_data("o/r", self.RUN, d)
                history.fetch_data("o/r", dict(self.RUN, id=78), d)  # second run: no 2nd warning
            self.assertEqual(out.getvalue().count("::warning::history: --legacy-trend expired on 2000-01-01"), 1)
            self.assertFalse(os.path.exists(os.path.join(d, "77", "legacy-point.json")))
        self.assertEqual(calls, [])

    def test_missing_module_swallowed(self):
        with tempfile.TemporaryDirectory() as d, \
                mock.patch.dict("sys.modules", {"legacy_points": None}), \
                mock.patch.object(history, "_gh", self._gh_fail):
            history.fetch_data("o/r", self.RUN, d)  # must not raise


class YAxisUnit(unittest.TestCase):
    def test_unit_from_row_label(self):
        for lab, u in (("Batch/Transaction Import (`/ui/batch`: `POST /` Bundles) (resources/s)", "resources/s"),
                       ("insert (creates/s)", "creates/s"), ("crud", "requests/s"),
                       ("x (seconds)", "seconds")):
            c = history.render_chart(lab, "sqlite", [1.0, 2.0], ["#1", "#2"])
            self.assertIn('y-axis "%s"' % u, c)
            self.assertNotIn('y-axis "value"', c)
            self.assertNotIn("`", c.split("title")[1].split("\n")[0])

if __name__ == "__main__":
    unittest.main()
