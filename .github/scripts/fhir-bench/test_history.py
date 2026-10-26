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

    def test_main_exit_zero(self):
        self.assertEqual(history.main(["--bogus"]), 0)


if __name__ == "__main__":
    unittest.main()
