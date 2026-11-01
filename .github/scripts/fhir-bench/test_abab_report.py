import json
import os
import tempfile
import unittest

import abab_report as ab


def summary(p50, p95, rate, failed=0.0):
    return {
        "metrics": {
            "http_req_duration": {"med": p50, "p(95)": p95},
            "http_reqs": {"rate": rate},
            "http_req_failed": {"value": failed},
        }
    }


def write_round(root, name, search=None, status=None, rus=None):
    d = os.path.join(root, name)
    os.makedirs(d)
    if search is not None:
        with open(os.path.join(d, "search.json"), "w") as f:
            json.dump(search, f)
    if rus is not None:
        with open(os.path.join(d, "read-under-search.json"), "w") as f:
            json.dump(rus, f)
    if status is not None:
        with open(os.path.join(d, "status-counts.txt"), "w") as f:
            f.write(status)


class VerdictTests(unittest.TestCase):
    def test_higher_better(self):
        self.assertEqual(ab.verdict([10, 11], [12, 13], ab.HIGHER), "BETTER")
        self.assertEqual(ab.verdict([12, 13], [10, 11], ab.HIGHER), "WORSE")

    def test_lower_better(self):
        self.assertEqual(ab.verdict([10, 11], [5, 6], ab.LOWER), "BETTER")
        self.assertEqual(ab.verdict([5, 6], [10, 11], ab.LOWER), "WORSE")

    def test_overlap_is_mixed(self):
        self.assertEqual(ab.verdict([10, 14], [12, 16], ab.HIGHER), "MIXED")

    def test_equal_is_same(self):
        self.assertEqual(ab.verdict([0, 0], [0, 0], ab.LOWER), "SAME")

    def test_empty(self):
        self.assertEqual(ab.verdict([], [1], ab.LOWER), "n/a")


class FailureRateTests(unittest.TestCase):
    def test_zero_total_is_na(self):
        self.assertEqual(ab.failure_rates({}), {c: None for c in ab.FAIL_CODES})
        self.assertEqual(ab.failure_rates({"200": 0}), {c: None for c in ab.FAIL_CODES})

    def test_rates(self):
        r = ab.failure_rates({"200": 90, "504": 10})
        self.assertEqual(r["504"], 10.0)
        self.assertEqual(r["408"], 0.0)

    def test_pct_change_zero_base(self):
        self.assertIsNone(ab.pct_change(0, 5))
        self.assertEqual(ab.pct_change(10, 15), 50.0)


class RenderTests(unittest.TestCase):
    def make(self, rounds=4, skip_b_round=None):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        for r in range(1, rounds + 1):
            write_round(
                tmp.name,
                "r%d-off" % r,
                search=summary(100 + r, 200 + r, 50 + r),
                status="suite=search status=200 count=100\nsuite=search status=504 count=%d\n" % (r,),
            )
            if r == skip_b_round:
                continue
            write_round(
                tmp.name,
                "r%d-timeout" % r,
                search=summary(50 + r, 100 + r, 80 + r),
                status="suite=search status=200 count=100\n",
            )
        return tmp.name

    def test_complete_better(self):
        out = ab.render(self.make(), expected=4)
        self.assertIn("ABAB complete: 4 paired rounds", out)
        self.assertIn("| search | p50 (ms) |", out)
        row = [l for l in out.splitlines() if l.startswith("| search | p50 (ms)")][0]
        self.assertTrue(row.endswith("| BETTER |"))
        row504 = [l for l in out.splitlines() if l.startswith("| search | 504 rate")][0]
        self.assertTrue(row504.endswith("| BETTER |"))

    def test_incomplete_marked(self):
        out = ab.render(self.make(skip_b_round=3), expected=4)
        self.assertIn("ABAB incomplete", out)
        self.assertIn("[3]", out)
        self.assertIn("3 of 4 expected rounds complete", out)

    def test_read_under_search_submetrics(self):
        with tempfile.TemporaryDirectory() as tmp:
            for r in (1, 2):
                for v, rate in (("off", 10), ("timeout+gate", 20)):
                    rus = {
                        "metrics": {
                            "http_req_duration{scenario:read}": {"med": 5, "p(95)": 9},
                            "http_reqs{scenario:read}": {"rate": rate + r},
                        }
                    }
                    write_round(tmp, "r%d-%s" % (r, v), rus=rus)
            out = ab.render(tmp, expected=2)
            row = [l for l in out.splitlines() if "read throughput" in l][0]
            self.assertTrue(row.endswith("| BETTER |"))

    def test_no_baseline(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.assertIn("nothing to compare", ab.render(tmp))

    def test_status_parse_ignores_garbage(self):
        with tempfile.TemporaryDirectory() as tmp:
            p = os.path.join(tmp, "s.txt")
            with open(p, "w") as f:
                f.write("junk\nsuite=crud status=408 count=3\nsuite=crud status=408 count=2\n")
            self.assertEqual(ab.load_status_counts(p), {"crud": {"408": 5}})


if __name__ == "__main__":
    unittest.main()
