import json
import os
import tempfile
import unittest

import run_timing as rt

D = "2026-10-26T"


def job(name, c, s, e):
    return {"name": name, "created_at": D + c + "Z", "started_at": D + s + "Z",
            "completed_at": (D + e + "Z") if e else None}


def make_fetcher(run, jobs):
    def f(kind, repo, rid):
        return run if kind == "run" else jobs
    return f


def write_info(root, leg, cap, host=None):
    d = os.path.join(root, f"fhir-benchmark-{leg}-1", leg)
    os.makedirs(d)
    with open(os.path.join(d, "runner-info.txt"), "w") as f:
        f.write(f"runner_name: r1\ncapacity_wait_s: {cap}\n")
        if host is not None:
            f.write(f"host_wait_s: {host}\n")


REAL_RUN = {"id": 40353889556, "created_at": D + "15:54:17Z"}
REAL_JOBS = [
    job("Resolve backend matrix", "15:54:19", "15:54:20", "15:54:25"),
    job("Build HFS binary", "15:54:25", "15:54:27", "16:07:50"),
    job("Benchmark (postgres-elasticsearch)", "16:07:50", "16:07:51", "16:47:25"),
    job("Compare legs", "16:47:25", "16:47:28", "16:47:43"),
]


class T(unittest.TestCase):
    def test_humanize(self):
        self.assertEqual(rt.humanize(11), "11 s")
        self.assertEqual(rt.humanize(3206), "53 min 26 s")
        self.assertEqual(rt.humanize(4 * 3600 + 27 * 60), "4 h 27 min")

    def test_real_fixture(self):
        with tempfile.TemporaryDirectory() as d:
            write_info(d, "postgres-elasticsearch", 2)
            rep = rt.build("o/r", "1", d, make_fetcher(REAL_RUN, REAL_JOBS))
            self.assertEqual(rep["total_s"], 3206)
            self.assertEqual(rep["waiting_s"], 11)
            self.assertEqual(rep["working_s"], 3195)
            self.assertEqual(rep["total_s"], rep["waiting_s"] + rep["working_s"])
            md = rt.to_markdown(rep)
            self.assertTrue(md.startswith("\n### Run time\n"))
            self.assertIn("**Total 53 min 26 s** · waiting for resources **11 s** "
                          "· working **53 min 15 s**", md)
            self.assertIn("| Resolve backend matrix | 3 s | – | 5 s |", md)
            self.assertIn("| Build HFS binary | 2 s | – | 13 min 23 s |", md)
            self.assertIn("| `postgres-elasticsearch` | 3 s (runner 1 s + capacity "
                          "gate 2 s) | – | 39 min 32 s |", md)
            self.assertIn("| Compare legs | 3 s | – | 15 s |", md)
            self.assertTrue(md.endswith("\n"))
            js = rt.to_json(rep)
            self.assertEqual([j["kind"] for j in js["jobs"]],
                             ["setup", "build", "leg", "compare"])
            self.assertEqual(js["jobs"][2]["leg"], "postgres-elasticsearch")
            self.assertEqual(js["jobs"][2]["capacity_wait_s"], 2)
            self.assertFalse(js["jobs"][0]["running"])

    def test_three_sequential_legs(self):
        run = {"id": 1, "created_at": D + "10:00:00Z"}
        jobs = [
            job("Build HFS binary", "10:00:00", "10:00:00", "10:10:00"),
            job("Benchmark (a)", "10:10:00", "10:10:00", "10:30:00"),
            job("Benchmark (b)", "10:10:00", "10:30:00", "10:50:00"),
            job("Benchmark (c)", "10:10:00", "10:50:00", "11:10:00"),
        ]
        with tempfile.TemporaryDirectory() as d:
            rep = rt.build("o/r", "1", d, make_fetcher(run, jobs))
        by = {r["leg"]: r for r in rep["rows"] if r["leg"]}
        self.assertEqual(by["a"]["behind"], 0)
        self.assertEqual(by["b"]["behind"], 1200)
        self.assertEqual(by["c"]["behind"], 2400)
        self.assertEqual(by["b"]["waiting"], 0)
        self.assertEqual(by["c"]["waiting"], 0)
        self.assertEqual(rep["total_s"], 4200)
        self.assertEqual(rep["waiting_s"], 0)
        self.assertEqual(rep["working_s"], 4200)
        md = rt.to_markdown(rep)
        self.assertIn("| `c` | 0 s | 40 min | 20 min |", md)

    def test_running_job(self):
        now = rt.parse_ts(D + "16:48:00Z")
        jobs = REAL_JOBS[:3] + [job("Compare legs", "16:47:25", "16:47:28", None)]
        rep = rt.build("o/r", "1", "", make_fetcher(REAL_RUN, jobs), now=now)
        last = rep["rows"][-1]
        self.assertTrue(last["running"])
        self.assertEqual(last["working"], 32)
        self.assertEqual(rep["total_s"], 3223)
        self.assertEqual(rep["total_s"], rep["waiting_s"] + rep["working_s"])
        self.assertIn("(running)", rt.to_markdown(rep))
        self.assertIsNone(rt.to_json(rep)["jobs"][-1]["completed"])

    def test_missing_runner_info_and_host_wait(self):
        with tempfile.TemporaryDirectory() as d:
            rep = rt.build("o/r", "1", d, make_fetcher(REAL_RUN, REAL_JOBS))
            self.assertEqual(rep["waiting_s"], 9)
        with tempfile.TemporaryDirectory() as d:
            write_info(d, "postgres-elasticsearch", 2, host=10)
            rep = rt.build("o/r", "1", d, make_fetcher(REAL_RUN, REAL_JOBS))
            self.assertEqual(rep["waiting_s"], 21)
            self.assertIn("quiet host 10 s", rt.to_markdown(rep))

    def test_api_failure_exits_zero_no_output(self):
        def boom(*a):
            raise RuntimeError("api down")
        with tempfile.TemporaryDirectory() as d:
            out, js = os.path.join(d, "o.md"), os.path.join(d, "o.json")
            rc = rt.main(["--repo", "o/r", "--run-id", "1", "--run-dir", d,
                          "--out", out, "--dump-json", js], fetcher=boom)
            self.assertEqual(rc, 0)
            self.assertFalse(os.path.exists(out))
            self.assertFalse(os.path.exists(js))

    def test_main_writes_files(self):
        with tempfile.TemporaryDirectory() as d:
            out, js = os.path.join(d, "o.md"), os.path.join(d, "o.json")
            rc = rt.main(["--repo", "o/r", "--run-id", "1", "--run-dir", d,
                          "--out", out, "--dump-json", js],
                         fetcher=make_fetcher(REAL_RUN, REAL_JOBS))
            self.assertEqual(rc, 0)
            self.assertEqual(json.load(open(js))["total_s"], 3206)
            self.assertIn("### Run time", open(out).read())


if __name__ == "__main__":
    unittest.main()
