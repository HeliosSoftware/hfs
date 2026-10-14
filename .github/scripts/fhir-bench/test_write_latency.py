#!/usr/bin/env python3
#
# Unit tests for write_latency.py: stdlib unittest, one fake server on 127.0.0.1:0 that
# plays both HFS (POST) and Elasticsearch (GET _settings). Not wired into CI. Run from
# the repo root:
#   python -m unittest discover -s .github/scripts/fhir-bench -p "test_*.py" -v
import http.server
import json
import os
import sys
import threading
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import write_latency as wl  # noqa: E402


class Handler(http.server.BaseHTTPRequestHandler):
    posts = []

    def log_message(self, *_a):
        pass

    def _send(self, status, doc):
        raw = json.dumps(doc).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

    def do_POST(self):
        n = int(self.headers.get("Content-Length", 0))
        Handler.posts.append((self.path, json.loads(self.rfile.read(n))))
        self._send(201, {"resourceType": "Basic", "id": "x"})

    def do_GET(self):
        if self.path.startswith("/hfs_default_riskassessment/_settings"):
            self._send(200, {"hfs_default_riskassessment": {
                "settings": {"index.refresh_interval": "30s"}}})
        elif self.path.startswith("/hfs_default_group/_settings"):
            self._send(200, {"hfs_default_group": {
                "settings": {}, "defaults": {"index.refresh_interval": "1s"}}})
        else:
            self._send(404, {})


class WriteLatencyTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        cls.url = "http://127.0.0.1:%d" % cls.srv.server_address[1]
        threading.Thread(target=cls.srv.serve_forever, daemon=True).start()

    @classmethod
    def tearDownClass(cls):
        cls.srv.shutdown()
        cls.srv.server_close()

    def setUp(self):
        Handler.posts = []

    def test_percentiles(self):
        s = wl.summarize([0.1, 0.2, 0.3, 0.4, 1.0], failures=1)
        self.assertEqual((s["n"], s["failures"], s["p50"], s["max"]), (5, 1, 0.3, 1.0))
        self.assertAlmostEqual(s["p95"], 0.88, places=3)
        self.assertIsNone(wl.summarize([])["p50"])

    def test_synthetic_bundles_have_the_issue_sizes(self):
        self.assertEqual(len(wl.synthetic_bundle("batch", wl.BATCH_ENTRIES)["entry"]), 8)
        t = wl.synthetic_bundle("transaction", wl.TRANSACTION_ENTRIES)
        self.assertEqual((t["type"], len(t["entry"])), ("transaction", 662))

    def test_full_run_posts_and_reports(self):
        args = wl.parse_args(["--base-url", self.url, "--samples", "3", "--timeout-s", "5",
                              "--es-url", self.url, "--new-type", "RiskAssessment"])
        out = wl.run(args)
        self.assertEqual((out["single"]["n"], out["single"]["failures"]), (3, 0))
        self.assertEqual((out["batch"]["entries"], out["batch"]["status"]), (8, 201))
        self.assertTrue(out["transaction"]["synthetic"])
        self.assertEqual(out["new_type"]["index_refresh_interval"], "30s")
        self.assertEqual(out["new_type"]["index"], "hfs_default_riskassessment")
        self.assertEqual(len(Handler.posts), 3 + 2 + 1)
        self.assertEqual(Handler.posts[-1][0], "/RiskAssessment")

    def test_default_refresh_interval_is_read_from_defaults(self):
        self.assertEqual(wl.refresh_interval_of(self.url, "hfs_default_group"), "1s")
        self.assertIsNone(wl.refresh_interval_of(self.url, "hfs_default_missing"))

    def test_skip_and_unreachable_server(self):
        args = wl.parse_args(["--base-url", "http://127.0.0.1:1", "--samples", "1",
                              "--timeout-s", "1", "--skip", "batch,transaction"])
        out = wl.run(args)
        self.assertEqual(out["single"]["failures"], 1)
        self.assertNotIn("batch", out)


if __name__ == "__main__":
    unittest.main()
