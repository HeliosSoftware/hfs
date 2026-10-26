"""Tests for md_charts.py (stdlib unittest): python3 -m unittest -q test_md_charts"""
import contextlib
import io
import os
import re
import tempfile
import unittest

import md_charts as mc

# Trimmed from the real comparison.md of run 40335047310 (one leg, Elasticsearch: indexing
# runs during ingest).
ONE_LEG = """## FHIR Benchmark — `tmp/bench-steve-ci-test` (leg comparison)

Commit `a4f41e7` · run 40335047310 · suites: all · 1 of 1 legs have results

### Throughput (requests/s unless the row names its unit)

| Suite | postgres+ES |
|---|---:|
| prewarm | 100 |
| Batch/Transaction Import (`/ui/batch`: `POST /` Bundles) (resources/s) | 6,178 |
| Bulk Import (`/ui/bulk-import`: `$bulk-submit`, until searchable) (resources/s) | 7,758 |
| ↳ ingest, until Completed (resources/s, seconds) | 7,758 (210 s) |
| ↳ search indexing (seconds) | 210 s (= ingest) |
| crud | 961 |
| search | 948 |
| SQL-on-FHIR `$sql-run` (requests/s) | 160 |
| SQL-on-FHIR `$sql-run` (rows/s) | 419,202 |
| insert (creates/s) | 259 |

### p95 latency (ms)

| Suite | postgres+ES |
|---|---:|
| prewarm | 286.8 |
| Batch/Transaction Import (`/ui/batch`: `POST /` Bundles) (per Bundle) | 11064.5 |
| Bulk Import (`/ui/bulk-import`: `$bulk-submit`) | n/a (one async job) |
| crud | 697.4 |
| search | 68.0 |
| SQL-on-FHIR `$sql-run` | 88.2 |
| insert | 248.7 |

### Errors (Err% · failed checks)

| Suite | postgres+ES |
|---|---:|
| prewarm | 0.0% |

`†` crud/search ran on an incomplete corpus · `‡` more than 1% errors

### Bulk import (`$bulk-submit`)

| Leg | Result | Resources ingested | Ingest | Index | Total | Search check |
|---|---|---:|---:|---:|---:|---|
| `postgres-elasticsearch` | ✓ | 1,630,685/1,630,685 | 210 s · 7,758/s | inline (during ingest) | 210 s | ✓ Patient 1,000 |

_Per-leg detail is in each `Benchmark (<leg>)` job summary._
"""

SIX = ["sqlite", "postgres", "mongodb", "sqlite+ES", "postgres+ES", "mongodb+ES"]


def many(legs, rebuild_for=("sqlite", "postgres", "mongodb"), marks=True):
    """A comparison with one column per leg; non-ES legs have a real deferred rebuild."""
    n = len(legs)

    def row(label, f):
        return "| %s | %s |" % (label, " | ".join(f(i, l) for i, l in enumerate(legs)))

    head = "| Suite | %s |\n|---|%s\n" % (" | ".join(legs), "---:|" * n)
    mark = lambda i: " †" if marks and i == 1 else ""
    thr = [
        row("prewarm", lambda i, l: "%d" % (100 + i)),
        row("Batch/Transaction Import (`/ui/batch`: `POST /` Bundles) (resources/s)",
            lambda i, l: "**%s**" % format(6000 + 100 * i, ",") if i == 0 else format(6000 + 100 * i, ",") + mark(i)),
        row("Bulk Import (`/ui/bulk-import`: `$bulk-submit`, until searchable) (resources/s)",
            lambda i, l: format(1000 * (i + 1), ",")),
        row("↳ ingest, until Completed (resources/s, seconds)",
            lambda i, l: "%s (%d s)" % (format(1000 * (i + 1), ","), 200 + i)),
        row("↳ search indexing (seconds)",
            lambda i, l: ("%d s" % (60 + i)) if l in rebuild_for else "%d s (= ingest)" % (200 + i)),
        row("crud", lambda i, l: "n/a (no results)" if i == 2 else format(900 + i, ",")),
        row("SQL-on-FHIR `$sql-run` (requests/s)", lambda i, l: "%d" % (160 + i)),
        row("SQL-on-FHIR `$sql-run` (rows/s)", lambda i, l: format(419202 + i, ",")),
    ]
    p95 = [
        row("crud", lambda i, l: "%.1f" % (600 + i)),
        row("Bulk Import (`/ui/bulk-import`: `$bulk-submit`)", lambda i, l: "n/a (one async job)"),
    ]
    return ("## FHIR Benchmark — `main` (leg comparison)\n\n### Throughput (requests/s unless the row names its unit)\n\n"
            + head + "\n".join(thr) + "\n\n### p95 latency (ms)\n\n" + head + "\n".join(p95)
            + "\n\n### Bulk import (`$bulk-submit`)\n\n| Leg | Result | Resources ingested | Ingest | Index | Total | Search check |\n"
            "|---|---|---:|---:|---:|---:|---|\n"
            + "\n".join("| `%s` | ✓ | 1/1 | 210 s · 7,758/s | inline (during ingest) | 210 s | ✓ |" % l for l in legs)
            + "\n\nfooter text\n")


LEG_BOX = """### Postgres tuning (postgres)

| setting | value |
|---|---|
| shared_buffers | `2GB` |

## FHIR Benchmark — `main` (postgres)

| | |
|---|---|
| **Commit**  | `e0bd5cb` |

### Results

| Suite | Throughput | p50 ms | p95 ms | p99 ms | Err% | Checks ✓/✗ |
|-------|----:|-------:|-------:|-------:|-----:|-----------:|
| crud | 2,175 | 122.7 | 290.0 | 415.7 | 0.0% | 526,680/0 |
| Batch/Transaction Import (`/ui/batch`: `POST /` Bundles) (resources/s; latency per Bundle) | 4,369 | 2673.3 | 26629.8 | 66378.2 | 0.0% | 1,000/0 |
| Bulk Import (`/ui/bulk-import`: `$bulk-submit`, until searchable) (resources/s) | 1,042 | n/a | n/a | n/a | 0.0% | 1,630,685/0 |
| insert | 1,425 | 31.9 | 57.7 | 102.9 | 0.0% | 171,220/0 |
| search | 2,186 | 7.7 | 40.4 | 114.4 | 0.0% | 262,872/0 |

_Throughput is requests/s._
"""


def run_cli(*args):
    out = io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(io.StringIO()):
        rc = mc.main(list(args))
    return rc, out.getvalue()


def write(d, name, text):
    p = os.path.join(d, name)
    with open(p, "w", encoding="utf-8") as fh:
        fh.write(text)
    return p


def mermaid_blocks(text):
    return re.findall(r"```mermaid\n(.*?)```", text, re.S)


class Compare(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)

    def compare(self, text, layout="sidebyside"):
        p = write(self.tmp.name, "comparison.md", text)
        rc, out = run_cli("compare", "--in", p, "--layout", layout)
        self.assertEqual(rc, 0)
        return out

    def test_one_leg_sections_and_charts(self):
        out = self.compare(ONE_LEG)
        self.assertTrue(out.startswith("\n### Charts\n"))
        self.assertIn("<details><summary>Chart: throughput by leg", out)
        self.assertIn("<table>", out)
        titles = re.findall(r"^\*\*(.+)\*\*$", out, re.M)
        for want in ("prewarm (requests/s)", "Batch/Transaction Import (resources/s)",
                     "Bulk Import (resources/s)", "Bulk Import ingest (resources/s, seconds)",
                     "Bulk Import search indexing (seconds)", "SQL-on-FHIR sql-run (requests/s)",
                     "SQL-on-FHIR sql-run (rows/s)", "insert (creates/s)",
                     "Batch/Transaction Import p95 (ms)", "crud p95 (ms)"):
            self.assertIn(want, titles)
        self.assertNotIn("Errors", out)
        self.assertNotIn("Bulk Import p95 (ms)", titles)  # n/a: one async job
        for t in titles:
            self.assertNotRegex(t, r"[`$]")

    def test_numbers_parse_through_markers_and_commas(self):
        out = self.compare(ONE_LEG)
        self.assertIn('"postgres+ES · 419,202"', out)
        self.assertIn("bar [419202]", out)
        self.assertIn("bar [7758]", out)  # "7,758 (210 s)" on the ingest row
        self.assertIn("bar [11064.5]", out)

    def test_inline_indexing_has_no_rebuild_phase(self):
        out = self.compare(ONE_LEG)
        gantts = [b for b in mermaid_blocks(out) if b.split("\n", 2)[1].startswith("gantt")]
        self.assertEqual(len(gantts), 1)
        self.assertIn("ingest 210 s", gantts[0])
        self.assertNotIn("rebuild", gantts[0])
        self.assertNotIn("rebuild", out.lower().replace("search indexing", ""))
        self.assertNotIn("deferred", out)

    def test_old_wording_bulk_table_fallback(self):
        old = ONE_LEG.replace("| ↳ ingest, until Completed (resources/s, seconds) | 7,758 (210 s) |\n", "") \
                     .replace("| ↳ search indexing (seconds) | 210 s (= ingest) |\n", "")
        out = self.compare(old)
        self.assertIn("Bulk Import phases by leg", out)
        self.assertIn("values in the Bulk import table above", out)

    def test_six_legs_colours_and_rebuild_only_where_real(self):
        out = self.compare(many(SIX))
        pal = re.findall(r'"plotColorPalette": "([^"]+)"', out)
        self.assertTrue(any(len(p.split(", ")) == 6 for p in pal))
        known = mc.leg_colours(SIX)
        for p in pal:
            for c in p.split(", "):
                self.assertIn(c, known.values())
        g = [b for b in mermaid_blocks(out) if "gantt" in b.split("\n")[1:3][0]][0]
        for leg in ("sqlite", "postgres", "mongodb"):
            self.assertRegex(g, r"section %s\n    ingest \d+ s :i\d+, [^\n]+\n    rebuild" % re.escape(leg))
        for leg in ("sqlite+ES", "postgres+ES", "mongodb+ES"):
            self.assertRegex(g, r"section %s\n    ingest \d+ s :i\d+, [^\n]+\n(    section|\Z)" % re.escape(leg))
        self.assertEqual(g.count("rebuild"), 3)
        self.assertIn("Bulk Import: ingest until Completed, then search-index rebuild", out)

    def test_n_a_cell_gives_zero_bar_but_keeps_label(self):
        out = self.compare(many(SIX))
        self.assertIn("mongodb · n/a (no results)", out)

    def test_each_bar_has_its_own_series_and_colour(self):
        out = self.compare(many(SIX))
        blk = [b for b in mermaid_blocks(out) if '"crud' not in b and "x-axis" in b][0]
        self.assertEqual(blk.count("    bar ["), 6)

    def test_more_than_six_legs_get_colours(self):
        legs = ["leg%d" % i for i in range(1, 13)]
        cols = mc.leg_colours(legs)
        self.assertEqual(len(cols), 12)
        self.assertTrue(all(re.match(r"^#[0-9a-f]{6}$", c) for c in cols.values()))
        self.assertEqual(len(set(list(cols.values())[:len(mc.EXTRA)])), len(mc.EXTRA))
        self.assertEqual(mc.leg_colours(["leg3"])["leg3"], mc.leg_colours(["leg3", "leg9"])["leg3"])
        out = self.compare(many(legs, rebuild_for=()))
        self.assertTrue(any(len(p.split(", ")) == 12 for p in re.findall(r'"plotColorPalette": "([^"]+)"', out)))

    def test_known_legs_keep_prototype_colours_for_short_labels(self):
        c = mc.leg_colours(["postgres+ES", "sqlite", "mongodb-elasticsearch"])
        self.assertEqual(c["postgres+ES"], "#4a91f8")
        self.assertEqual(c["sqlite"], "#008622")
        self.assertEqual(c["mongodb-elasticsearch"], "#b96dd8")

    def test_prefix_invariant(self):
        for text in (ONE_LEG, many(SIX), many(SIX[:2]), ONE_LEG.rstrip("\n")):
            out = self.compare(text)
            self.assertTrue(out)
            self.assertTrue((text + out).startswith(text))
            self.assertEqual((text + out)[:len(text)].encode(), text.encode())
            self.assertTrue(out.startswith("\n"))  # a blank line separates it from the report text

    def test_stacked_layout_has_no_table(self):
        out = self.compare(ONE_LEG, "stacked")
        self.assertNotIn("<table>", out)
        self.assertIn("```mermaid", out)

    def test_no_backticks_or_dollars_in_mermaid(self):
        for b in mermaid_blocks(self.compare(many(SIX))):
            self.assertNotIn("`", b)
            self.assertNotIn("$", b)


class Leg(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.d = os.path.join(self.tmp.name, "bench-results")
        os.makedirs(self.d)
        write(self.d, "host-contention.txt",
              "14:11:46Z suite=prewarm phase=start host_loadavg=0.9\n14:12:21Z suite=prewarm phase=end x\n"
              "23:59:00Z suite=crud phase=start x\n00:03:00Z suite=crud phase=end x\n")

    def leg(self, box_text, leg="postgres", bulk=None):
        if bulk is not None:
            write(self.d, "bulk-import.txt", bulk)
        box = write(self.tmp.name, "box.md", box_text)
        rc, out = run_cli("leg", "--box", box, "--leg", leg, "--run-dir", self.d, "--layout", "sidebyside")
        self.assertEqual(rc, 0)
        return out

    def test_leg_charts_and_header_not_first(self):
        out = self.leg(LEG_BOX)
        self.assertTrue(out.startswith("\n### Charts\n"))
        self.assertIn("postgres: k6 suites throughput (requests/s)", out)
        self.assertIn("postgres: Import throughput (resources/s)", out)
        self.assertIn("postgres: latency per suite (ms)", out)
        self.assertIn("bar [2175, 1425, 2186]", out)
        self.assertIn('"plotColorPalette": "#236bcf"', out)
        self.assertIn("section Suites", out)
        self.assertIn("2000-01-02 00:03:00", out)  # crossed midnight UTC

    def test_leg_prefix_invariant(self):
        out = self.leg(LEG_BOX)
        self.assertTrue((LEG_BOX + out).startswith(LEG_BOX))

    def test_leg_inline_vs_rebuild(self):
        inline = self.leg(LEG_BOX, bulk="ingest_seconds=210.2\nindex_status=inline\nindex_seconds=0\n")
        g = [b for b in mermaid_blocks(inline) if "ingest 210 s" in b][0]
        self.assertNotIn("rebuild", g)
        real = self.leg(LEG_BOX, bulk="ingest_seconds=210.2\nindex_status=completed\nindex_seconds=120.4\n")
        g = [b for b in mermaid_blocks(real) if "ingest 210 s" in b][0]
        self.assertIn("rebuild 120 s", g)

    def test_missing_files_still_charts_from_table(self):
        out = self.leg(LEG_BOX)
        self.assertIn("throughput and latency", out)
        empty = os.path.join(self.tmp.name, "nothing")
        box = write(self.tmp.name, "b2.md", LEG_BOX)
        rc, out = run_cli("leg", "--box", box, "--leg", "postgres", "--run-dir", empty)
        self.assertEqual(rc, 0)
        self.assertIn("throughput and latency", out)


class Robust(unittest.TestCase):
    GARBAGE = ["", "\n", "garbage", "### Throughput\n| a |\n", "### Throughput\n\n| Suite | x |\n|---|\n| a |\n",
               "### Throughput\n\n| Suite | a | b |\n|---|---|---|\n| row | ** | †⚠ |\n| | | |\n",
               "### p95 latency (ms)\n\n| Suite |\n|---|\n| crud |\n", "\x00\xff||||\n|--|\n",
               "### Bulk import\n\n| Leg |\n|---|\n| `x` |\n", "### Results\n\n| Suite |\n|---|\n| crud |\n"]

    def test_never_raises_on_garbage(self):
        with tempfile.TemporaryDirectory() as t:
            for g in self.GARBAGE:
                p = write(t, "g.md", g)
                for args in (("compare", "--in", p), ("leg", "--box", p, "--leg", "x", "--run-dir", t)):
                    rc, out = run_cli(*args)
                    self.assertEqual(rc, 0)
                    self.assertLess(len(out), 2000, out[:200])

    def test_missing_files_and_bad_args_exit_zero(self):
        for args in (("compare", "--in", "/nonexistent/x.md"),
                     ("leg", "--box", "/nonexistent", "--leg", "a", "--run-dir", "/nonexistent"),
                     ("bogus",), ("compare",), ()):
            rc, out = run_cli(*args)
            self.assertEqual(rc, 0)
            self.assertLess(len(out), 200)

    def test_binary_file(self):
        with tempfile.TemporaryDirectory() as t:
            p = os.path.join(t, "b.md")
            with open(p, "wb") as fh:
                fh.write(bytes(range(256)) * 20)
            self.assertEqual(run_cli("compare", "--in", p)[0], 0)

    def test_num_parsing(self):
        self.assertEqual(mc.num("**1,042** †"), 1042)
        self.assertEqual(mc.num("⚠ 7,758 (210 s)"), 7758)
        self.assertEqual(mc.num("0.0%"), 0.0)
        self.assertIsNone(mc.num("n/a (one async job)"))
        self.assertIsNone(mc.num("inline (during ingest)"))
        self.assertEqual(mc.num("210 s (= ingest)"), 210)


if __name__ == "__main__":
    unittest.main()
