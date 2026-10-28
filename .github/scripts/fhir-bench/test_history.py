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
                  "| ↳ search indexing (seconds) | 195 s (= ingest)"
        ).replace("| **313 s** | 321 s | 350 s |",
                  "| 313 s (= ingest) | 321 s (= ingest) | 350 s (= ingest) |")
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
        # old rebuild joins the new indexing row; the deferred-era ES cell is kept, marked
        self.assertEqual(out.count("**" + history.NEW_INDEX + "**"), 1)
        sec = out.split("**" + history.NEW_INDEX + "**")[1].split("**")[0]
        self.assertIn("| `sqlite` | 1,099 | 195 (-82%) | 195 (+0%) |", sec)
        # deferred (run 1) is never a reference for inline (runs 2, this run)
        self.assertIn("| `sqlite+ES` | 313ᵈ | 313 | 313 (+0%) |", sec)

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
    def test_38751206249_deferred_index_values_kept_on_es(self):
        legs, rows = self.rows(38751206249)
        i = legs.index("sqlite")
        self.assertEqual(rows[history.NEW_INDEX][i], 1541.0)
        self.assertEqual(rows[history.NEW_INGEST][i], 2547.0)
        j = legs.index("sqlite+ES")  # no longer blanked
        self.assertEqual(rows[history.NEW_INDEX][j], 352.0)
        self.assertIsNotNone(rows[history.NEW_INGEST][j])
        # bulk end-to-end stays valid on ES legs
        self.assertEqual(rows[history.NEW_BULK][legs.index("sqlite+ES")], 1564.0)

    @unittest.skipUnless(have_fixtures(), "legacy_testdata missing")
    def test_39657850695_index_joins_new_label(self):
        legs, rows = self.rows(39657850695)
        self.assertEqual(rows[history.NEW_INDEX][legs.index("sqlite")], 1099.0)
        self.assertEqual(rows[history.NEW_INDEX][legs.index("mongodb+ES")], 350.0)
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
        self.assertEqual(rows[history.NEW_INDEX], [1655.0, 490.0])
        self.assertEqual(rows[history.NEW_INGEST], [4125.0, 3327.0])
        t = history.parse_tables(md)["throughput"]
        self.assertEqual(t["index_mode"], "deferred")

    def test_new_format_values_and_mode(self):
        md = ("### Throughput\n\n| Suite | sqlite | sqlite+ES |\n|---|---:|---:|\n"
              f"| {history.NEW_INGEST} | 4,125 (395 s) | 3,327 (490 s) |\n"
              f"| {history.NEW_INDEX} | 1,655 s | 490 s (= ingest) |\n")
        rows = dict(history.parse_tables(md)["throughput"]["rows"])
        self.assertEqual(rows[history.NEW_INDEX], [1655.0, 490.0])
        self.assertEqual(rows[history.NEW_INGEST], [4125.0, 3327.0])
        t = history.parse_tables(md)["throughput"]
        self.assertEqual(t["index_mode"], "inline")
        self.assertTrue(t["cells"][history.NEW_INDEX][1]["inline"])

    def test_reindex_label_never_aliased(self):
        for lab in ("reindex, `$reindex` Encounter (resources/s)",
                    "Indexing ($reindex of Encounter)"):
            self.assertEqual(history.canon_label(lab), lab)


def md_table(rows, legs=("sqlite", "sqlite+ES")):
    head = "### Throughput\n\n| Suite | " + " | ".join(legs) + " |\n|---|" + "---:|" * len(legs) + "\n"
    return head + "\n".join("| " + " | ".join(r) + " |" for r in rows) + "\n"


def hist(texts, current=None):
    """Markdown history of runs 1..n (texts) plus an optional current text."""
    runs = [run(i + 1) for i in range(len(texts))]
    by = {r["number"]: t for r, t in zip(runs, texts)}
    return history.build(args(), lambda *a: runs, lambda repo, r, n: by[r["number"]], current)


def block(out, label):
    return out.split("**" + label + "**")[1].split("</details>")[0].split("\n**")[0]


def line(sec, leg):
    return next(l for l in sec.splitlines() if l.startswith(f"| `{leg}` |"))


class NearestReference(unittest.TestCase):
    def tbl(self, v):
        return md_table([[ "crud", v, "1"]])

    def test_skips_empty_column_and_labels_it(self):
        out = hist([self.tbl("100"), self.tbl("n/a (x)"), self.tbl("150")], self.tbl("n/a (x)"))
        sec = block(out, "crud")
        self.assertIn("| `sqlite` | 100 | – | 150 (+50% vs #1) | – |", sec)

    def test_previous_column_has_no_label(self):
        out = hist([self.tbl("100")], self.tbl("120"))
        self.assertIn("| `sqlite` | 100 | 120 (+20%) |", block(out, "crud"))

    def test_no_reference_no_pct(self):
        out = hist([self.tbl("n/a (x)"), self.tbl("100")], self.tbl("n/a (x)"))
        self.assertIn("| `sqlite` | – | 100 | – |", block(out, "crud"))

    def test_legend_on_every_block(self):
        out = hist([self.tbl("100")], self.tbl("120"))
        self.assertIn(history.LEGEND_CHANGE, out)
        self.assertNotIn(history.LEGEND_MODE, out)  # no index block
        self.assertIn("\n\n" + history.LEGEND_CHANGE, out)


class IndexingMode(unittest.TestCase):
    OLD = [history.NEW_INGEST.replace("↳ ingest, until Completed", "Bulk Import: ingest")]

    def old(self, sq, es):
        return md_table([["Bulk Import: ingest (resources/s, seconds)", "10 (1 s)", "10 (1 s)"],
                         ["Bulk Import: index rebuild (seconds)", sq, es]])

    def new(self, sq, es):
        return md_table([[history.NEW_INGEST, "10 (1 s)", "10 (1 s)"],
                         [history.NEW_INDEX, sq, es]])

    def test_es_deferred_never_reference_for_inline_and_marker(self):
        out = hist([self.old("1,000 s", "300 s"), self.new("900 s", "200 s (= ingest)")],
                   self.new("450 s", "100 s (= ingest)"))
        sec = block(out, history.NEW_INDEX)
        # standalone leg compares across #2248; ES inline skips the deferred run
        self.assertIn("| `sqlite` | 1,000 | 900 (-10%) | 450 (-50%) |", sec)
        self.assertIn("| `sqlite+ES` | 300ᵈ | 200 | 100 (-50%) |", sec)
        self.assertIn(history.LEGEND_MODE, sec)

    def test_inline_never_reference_for_deferred(self):
        out = hist([self.new("900 s", "200 s (= ingest)"), self.old("1,000 s", "300 s")],
                   self.old("1,100 s", "330 s"))
        self.assertIn("| `sqlite+ES` | 200 | 300ᵈ | 330ᵈ (+10%) |", block(out, history.NEW_INDEX))

    def test_marker_only_on_es_legs_of_mode_blocks(self):
        out = hist([self.old("1,000 s", "300 s")], self.old("1,000 s", "300 s"))
        self.assertIn("| `sqlite` | 1,000 | 1,000 (+0%) |", block(out, history.NEW_INDEX))
        self.assertIn("ᵈ", block(out, history.NEW_INGEST))
        self.assertNotIn("ᵈ", out.split("</summary>")[0])

    def test_unknown_only_with_unknown(self):
        unk = md_table([["crud", "1", "1"], [history.NEW_INGEST.replace("↳", "x"), "1", "1"]])
        self.assertEqual(history.parse_tables(unk)["throughput"]["index_mode"], "unknown")
        mode = history.leg_mode(history.parse_tables(unk)["throughput"], "sqlite+ES")
        self.assertEqual(mode, "unknown")
        self.assertEqual(history.leg_mode({"index_mode": "unknown"}, "sqlite"), "deferred")

    def test_failed_rebuild_never_reference(self):
        out = hist([self.new("1,000 s", "5 s (= ingest)"),
                    self.new("1,923 s (failed) ‡", "5 s (= ingest)")], self.new("900 s", "5 s"))
        sec = block(out, history.NEW_INDEX)
        # the failed cell is flagged and has no change; it is not the reference of the next run
        self.assertIn("| `sqlite` | 1,000 | 1,923 (failed) | 900 (-10% vs #1) |", sec)

    def test_no_mode_legend_without_es_data(self):
        md = md_table([[history.NEW_INDEX, "9 s"]], legs=("sqlite",))
        out = hist([md], md)
        self.assertNotIn(history.LEGEND_MODE, out)
        self.assertIn(history.LEGEND_CHANGE, out)

    def test_other_blocks_ignore_mode(self):
        a = md_table([["Bulk Import, `$bulk-submit` (resources/s)", "10", "20"],
                      ["Bulk Import: index rebuild (seconds)", "1", "1"]])
        b = md_table([[history.NEW_BULK, "11", "22"], [history.NEW_INDEX, "1", "1"]])
        out = hist([a], b)
        self.assertIn("| `sqlite+ES` | 20.0 | 22.0 (+10%) |", block(out, history.NEW_BULK))

    def test_cancelled_rebuild_flagged_like_failed(self):
        self.assertEqual(history.cell_flags("900 s (cancelled) ‡")["status"], "cancelled")
        self.assertFalse(history.cell_flags("900 s")["failed"])
        self.assertFalse(history.cell_flags("490 s (= ingest)")["failed"])
        out = hist([self.new("1,000 s", "5 s (= ingest)"),
                    self.new("800 s (cancelled) ‡", "5 s (= ingest)")], self.new("900 s", "5 s"))
        self.assertIn("| `sqlite` | 1,000 | 800 (cancelled) | 900 (-10% vs #1) |",
                      block(out, history.NEW_INDEX))

    def test_mode_from_own_index_cell(self):
        # table-level mode says deferred (old row label somewhere), the ES leg's own cell
        # says inline: the cell wins; a numeric cell is deferred
        mixed = md_table([[history.NEW_INGEST, "1,000 (1 s)", "1,000 (1 s)"],
                          [history.NEW_INDEX, "100 s", "5 s (= ingest)"]])
        deferred = md_table([[history.NEW_INGEST, "1,000 (1 s)", "1,000 (1 s)"],
                             [history.NEW_INDEX, "100 s", "300 s"]])
        out = hist([deferred], mixed)
        ing = block(out, history.NEW_INGEST)
        self.assertIn("| `sqlite+ES` | 1,000ᵈ | 1,000 |", ing)  # deferred run, then inline: no change
        self.assertIn("| `sqlite` | 1,000 | 1,000 (+0%) |", ing)

    def test_bulk_until_searchable_flagged_when_index_failed(self):
        def t(bulk, idx):
            return md_table([[history.NEW_BULK, bulk, "500"], [history.NEW_INDEX, idx, "5 s (= ingest)"]])
        out = hist([t("1,000", "9 s"), t("700", "9 s (failed) ‡")], t("900", "9 s"))
        self.assertIn("| `sqlite` | 1,000 | 700 (index failed) | 900 (-10% vs #1) |",
                      block(out, history.NEW_BULK))

    def test_legend_mode_says_es_only(self):
        self.assertIn("(marked on Elasticsearch legs only; the other legs always defer)",
                      history.LEGEND_MODE)

    @unittest.skipUnless(have_fixtures(), "legacy_testdata missing")
    def test_real_fixtures(self):
        ids = (38501925223, 38751206249, 39657850695, 40547405940)
        runs = [run(i + 1, branch="main") for i in range(4)]  # main: suites not compared
        by = {r["number"]: fixture(i) for r, i in zip(runs[:3], ids[:3])}
        out = history.build(args(), lambda *a: runs[:3], lambda repo, r, n: by[r["number"]],
                            fixture(40547405940))
        idx = block(out, history.NEW_INDEX)
        self.assertIn("352ᵈ", idx)  # sqlite+ES of the oldest run, kept and marked
        # the inline value of the current run has no earlier inline run: no change at all
        self.assertTrue(line(idx, "sqlite+ES").endswith("| 490 |"))
        self.assertNotIn("(= ingest)", idx)
        ing = block(out, history.NEW_INGEST)
        self.assertTrue(line(ing, "sqlite+ES").endswith("| 3,327 |"))
        self.assertIn("ᵈ", line(ing, "sqlite+ES"))
        self.assertIn("ᵈ", line(idx, "postgres+ES"))
        # postgres rebuild failed in the current run: flagged, no change; its until-searchable
        # cell is flagged too and was not a reference
        self.assertIn("1,923 (failed)", line(idx, "postgres"))
        self.assertIn("707 (index failed)", line(block(out, history.NEW_BULK), "postgres"))
        self.assertIn("**reindex, `$reindex` Encounter (resources/s)**", out)


class HostChange(unittest.TestCase):
    TS = "2026-10-28T14:45:00Z"

    def tbl(self, v):
        return md_table([["crud", v, "1"]])

    def go(self, stamps, cur_stamp=None):
        runs = []
        for i, st in enumerate(stamps):
            r = run(i + 1)
            r["created_at"] = st
            r["date"] = st[:10]
            runs.append(r)
        by = {r["number"]: self.tbl(str(100 + r["number"])) for r in runs}
        return history.build(args(created_at=cur_stamp), lambda *a: runs,
                             lambda repo, r, n: by[r["number"]], self.tbl("120"))

    def test_legend_when_span_crosses_change(self):
        import host_changes
        out = self.go(["2026-10-27T10:00:00Z", "2026-10-28T16:00:00Z"])
        text = "_" + host_changes.legend(host_changes.HOST_CHANGES[0]) + "_"
        self.assertIn(text, out.split("</summary>")[1].split("**crud**")[0])  # intro
        self.assertIn(text, block(out, "crud"))  # and under the block

    def test_no_legend_when_all_runs_after_change(self):
        out = self.go(["2026-10-28T15:00:00Z", "2026-10-29T10:00:00Z"], "2026-10-29T12:00:00Z")
        self.assertNotIn("older Docker host", out)

    def test_no_legend_when_all_runs_before_change(self):
        out = self.go(["2026-10-20T15:00:00Z", "2026-10-21T10:00:00Z"], "2026-10-22T12:00:00Z")
        self.assertNotIn("older Docker host", out)

    def test_current_run_timestamp_closes_the_span(self):
        out = self.go(["2026-10-20T15:00:00Z", "2026-10-21T10:00:00Z"], "2026-10-29T12:00:00Z")
        self.assertIn("older Docker host", out)

    def test_legacy_runs_fall_back_to_date(self):
        runs = [run(1, date="2026-10-2%d"), run(2, date="2026-10-2%d")]  # 21, 22: no created_at
        runs[1]["date"] = "2026-10-29"
        out = history.build(args(), lambda *a: runs, lambda *a: self.tbl("100"), self.tbl("120"))
        self.assertIn("older Docker host", out)

    def test_norm_keeps_created_at(self):
        n = history._norm({"id": 1, "created_at": "2026-10-28T10:00:00Z"})
        self.assertEqual(n["created_at"], "2026-10-28T10:00:00Z")
        self.assertEqual(n["date"], "2026-10-28")


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
