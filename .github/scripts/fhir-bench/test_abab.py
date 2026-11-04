import json
import os
import tempfile
import unittest

import abab


def put_round(leg, n, mode, suite, rps, p50, p95, drain_status="drained",
              drain_s=5, depth_p95=0, depth_area=0):
    d = os.path.join(leg, f"r{n}-{mode}")
    os.makedirs(d, exist_ok=True)
    with open(os.path.join(d, f"{suite}.json"), "w") as fh:
        json.dump({"metrics": {"http_req_duration": {"med": p50, "p(95)": p95},
                               "http_reqs": {"rate": rps, "count": 1000}}}, fh)
    with open(os.path.join(d, f"{suite}-queue.txt"), "w") as fh:
        fh.write(f"samples=10\ninterval_s=1\ndepth_p95={depth_p95}\ndepth_area={depth_area}\n")
    with open(os.path.join(d, f"es-drain-{suite}.txt"), "w") as fh:
        fh.write(f"status={drain_status}\ngate_seconds={drain_s}\nround={n}\nsync_mode={mode}\n")


def metric(res, name):
    return next(m for m in res["metrics"] if m["metric"] == name)


class AbabTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.leg = self.tmp.name

    def tearDown(self):
        self.tmp.cleanup()

    def test_b_always_better(self):
        for n, (a, b) in enumerate([(100, 150), (105, 140), (98, 160), (102, 145)], 1):
            put_round(self.leg, n, "sync", "crud", a, 10, 20)
            put_round(self.leg, n, "async", "crud", b, 10, 20)
        res = abab.analyze(self.leg)
        m = metric(res, "crud.rps")
        self.assertEqual(m["verdict"], abab.MEJORA)
        self.assertEqual(m["spread_a"], [98, 105])
        self.assertAlmostEqual(m["delta_pct"], (147.5 - 101) / 101 * 100, places=1)

    def test_b_always_worse_lower_is_better(self):
        for n, (a, b) in enumerate([(10, 30), (11, 28), (9, 31), (10, 29)], 1):
            put_round(self.leg, n, "sync", "search", 100, a, 20)
            put_round(self.leg, n, "async", "search", 100, b, 20)
        self.assertEqual(metric(abab.analyze(self.leg), "search.p50")["verdict"], abab.EMPEORA)

    def test_overlap_is_not_demonstrable(self):
        for n, (a, b) in enumerate([(100, 150), (105, 99), (98, 160), (102, 145)], 1):
            put_round(self.leg, n, "sync", "crud", a, 10, 20)
            put_round(self.leg, n, "async", "crud", b, 10, 20)
        self.assertEqual(metric(abab.analyze(self.leg), "crud.rps")["verdict"], abab.NO_DIFF)

    def test_tie_is_not_demonstrable(self):
        for n in (1, 2):
            put_round(self.leg, n, "sync", "crud", 100, 10, 20)
            put_round(self.leg, n, "async", "crud", 100, 10, 20)
        self.assertEqual(metric(abab.analyze(self.leg), "crud.rps")["verdict"], abab.NO_DIFF)

    def test_invalid_round_excluded(self):
        put_round(self.leg, 1, "sync", "crud", 100, 10, 20)
        put_round(self.leg, 2, "sync", "crud", 100, 10, 20)
        put_round(self.leg, 1, "async", "crud", 150, 10, 20)
        put_round(self.leg, 2, "async", "crud", 1, 10, 20, drain_status="timeout")
        res = abab.analyze(self.leg)
        m = metric(res, "crud.rps")
        self.assertEqual(m["b"], [150])
        self.assertEqual(m["verdict"], abab.NO_DIFF)  # only 1 valid B round
        self.assertEqual(res["invalid"], [{"suite": "crud", "round": 2, "mode": "async"}])

    def test_cli_writes_files(self):
        for n in (1, 2):
            put_round(self.leg, n, "sync", "insert", 100, 10, 20)
            put_round(self.leg, n, "async", "insert", 200 + n, 10, 20)
        self.assertEqual(abab.main([self.leg]), 0)
        with open(os.path.join(self.leg, "abab.json")) as fh:
            data = json.load(fh)
        self.assertEqual(data["rounds"], [1, 2])
        self.assertIn("MEJORA", open(os.path.join(self.leg, "abab.txt")).read())

    def test_no_rounds(self):
        res = abab.analyze(self.leg)
        self.assertEqual(res["metrics"], [])


if __name__ == "__main__":
    unittest.main()
