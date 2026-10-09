#!/usr/bin/env python3
#
# Unit tests for summary_backends.py's indexing-suite lines: stdlib unittest,
# synthetic bench-results/<leg>/ files built in code, the script run as the
# workflow runs it (its own python process, GITHUB_WORKSPACE + BACKEND set).
# Not wired into CI. Run from the repo root:
#   python -m unittest discover -s .github/scripts/fhir-bench -p "test_*.py" -v
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

SCRIPT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "summary_backends.py")
FINISHED = ("2026-10-09T13:19:08.201143Z  INFO helios_persistence::search::reindex: reindex job "
            "finished tenant=default job_id=j outcome=completed types_done=1 types=1 "
            "processed=5000 total=5000 elapsed_ms=8018 resources_per_s=623.6 entries=45000 "
            "failed=0 pages=6 fetch_ms=512 write_ms=7401 extract_ms=0 delete_ms=0 insert_ms=0 "
            "writer_other_ms=7401 yield_ms=31 other_ms=74 db_wait_ms=0\n")


class SummaryIndexingTest(unittest.TestCase):
    def summary(self, files, backend="sqlite"):
        ws = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, ws, True)
        d = os.path.join(ws, "bench-results", backend)
        os.makedirs(d)
        for name, text in files.items():
            with open(os.path.join(d, name), "w", encoding="utf-8") as fh:
                fh.write(text)
        env = dict(os.environ, GITHUB_WORKSPACE=ws, BACKEND=backend)
        return subprocess.run([sys.executable, SCRIPT], env=env, stdout=subprocess.PIPE,
                              stderr=subprocess.PIPE, universal_newlines=True,
                              check=True).stdout

    def test_completed_with_phase_split(self):
        out = self.summary({
            "indexing.txt": "status=completed\nreason=\nresource_type=Encounter\ntotal=5000\n"
                            "processed=5000\nentries=45000\nerrors=0\nseconds=8.028\n"
                            "resources_per_s=622.8\nentries_per_resource=9.00\nbatch_size=1000\n"
                            "budget_s=180\nes_refresh=n/a\n",
            "indexing-hfs-log.txt": FINISHED})
        self.assertIn("**Indexing (`$reindex` of Encounter):** completed — 5,000/5,000 resources "
                      "in 8.0 s = **623 resources/s**", out)
        self.assertIn("fetch 0.5 s · write 7.4 s", out)
        self.assertIn("budget 180 s", out)
        self.assertNotIn("⚠", out)

    def test_partial_es_and_failed_kickoff(self):
        out = self.summary({
            "indexing.txt": "status=partial\nreason=time-cap\nresource_type=Encounter\n"
                            "total=67692\nprocessed=30000\nentries=450000\nerrors=0\n"
                            "seconds=180.4\nresources_per_s=166.3\nentries_per_resource=15.00\n"
                            "batch_size=1000\nbudget_s=180\nes_refresh=wait_for\n"},
            backend="sqlite-elasticsearch")
        self.assertIn("30,000/67,692", out)
        self.assertIn("Elasticsearch refresh `wait_for`", out)
        self.assertIn("⚠ partial (time-cap)", out)
        self.assertIn("budget 180 s", out)
        out = self.summary({"indexing.txt": "status=kickoff-failed\nreason=http-501\n"
                                            "resource_type=Encounter\nkickoff_http=501\n"})
        self.assertIn("answered HTTP 501", out)

    def test_no_indexing_file_prints_nothing(self):
        out = self.summary({"runner-info.txt": "runner_name:  agent-sqlite\nrunner_cpus:  8\n"})
        self.assertNotIn("Indexing", out)

    def test_budget_other_value(self):
        out = self.summary({"indexing.txt": "status=completed\nresource_type=Encounter\n"
                                            "total=10\nprocessed=10\nentries=90\nerrors=0\n"
                                            "seconds=1.0\nresources_per_s=10.0\n"
                                            "entries_per_resource=9.00\nbatch_size=1000\n"
                                            "budget_s=240\nes_refresh=n/a\n"})
        self.assertIn("budget 240 s", out)


if __name__ == "__main__":
    unittest.main()
