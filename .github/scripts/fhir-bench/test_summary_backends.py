#!/usr/bin/env python3
#
# Unit tests for summary_backends.py's bulk-import lines: stdlib unittest,
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

class SummaryRunMixin(object):
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


def bulk_txt(**kv):
    """A complete bulk-import.txt body (key=value), deferred indexing by default."""
    fields = {"status": "complete", "resources_submitted": 1630685, "resources_ok": 1630685,
              "resources_failed": 0, "defer_indexing": "true", "ingest_seconds": 600,
              "index_status": "completed", "index_seconds": 1800, "total_seconds": 2400,
              "resources_per_s": 679.5, "ingest_resources_per_s": 2717.8, "searchable": "yes"}
    fields.update(kv)
    return "".join("%s=%s\n" % (k, v) for k, v in fields.items())


class SummaryBulkImportTest(SummaryRunMixin, unittest.TestCase):
    def test_bulk_line_deferred_unchanged(self):
        out = self.summary({"bulk-import.txt": bulk_txt()})
        self.assertIn("resources in 2400 s (ingest until Completed 600 s + search-index rebuild 1800 s, "
                      "`completed`) = **679.5 resources/s** end to end, 2717.8 resources/s "
                      "ingest only; fresh database, `defer_indexing=true`.", out)

    def test_bulk_line_inline(self):
        out = self.summary({"bulk-import.txt": bulk_txt(
            defer_indexing="false", index_status="inline", index_seconds=0, total_seconds=600)})
        self.assertIn("(ingest until Completed 600 s, search index built inline during ingest)", out)
        self.assertNotIn("rebuild 0 s", out)
        self.assertNotIn("search-index rebuild", out)

    def test_bulk_line_inline_with_rebuild_of_rejected_types(self):
        out = self.summary({"bulk-import.txt": bulk_txt(
            defer_indexing="false", index_status="completed", index_seconds=12,
            total_seconds=612)})
        self.assertIn("(ingest until Completed 600 s, search index built inline during ingest + 12 s rebuild "
                      "of rejected types, `completed`)", out)
        self.assertNotIn("search-index rebuild", out)


def sql_run_json(views=("observation_page", "observation_large", "observation_filter", "claim"),
                 fails=0):
    """A small sql-run.json (k6 --summary-export); `eob` has no samples by default."""
    import json
    m = {"sql_run_rows": {"count": 120000, "rate": 2000.0},
         "http_req_failed": {"value": 0.0125, "passes": 1, "fails": 80}}
    for v in views:
        m["http_req_duration{view:%s}" % v] = {"avg": 10.0, "min": 1.0, "med": 8.5,
                                                "max": 99.0, "p(90)": 20.0, "p(95)": 30.25,
                                                "count": 500}
    checks = {"sql-run 200": {"passes": 500, "fails": fails},
              "sql-run rows > 0": {"passes": 500, "fails": 0}}
    return json.dumps({"metrics": m, "root_group": {"checks": checks}})


class SummarySqlRunTest(SummaryRunMixin, unittest.TestCase):
    def test_section_absent_without_file(self):
        self.assertNotIn("$sql-run", self.summary({}))

    def test_table_rows_and_totals(self):
        out = self.summary({"sql-run.json": sql_run_json()})
        self.assertIn("**SQL-on-FHIR `$sql-run` by view**", out)
        self.assertIn("| View | Requests | avg ms | p50 ms | p90 ms | p95 ms | max ms |", out)
        self.assertIn("| observation_filter | 500 | 10.0 | 8.5 | 20.0 | 30.2 | 99.0 |", out)
        self.assertNotIn("p99", out)
        self.assertIn("Rows returned: 120,000 total, 2,000 rows/s · errors 1.25%", out)
        self.assertIn("sql-run 200 ✓; sql-run rows > 0 ✓", out)
        self.assertIn("the filtered view is the scan", out)

    def test_view_without_samples(self):
        out = self.summary({"sql-run.json": sql_run_json()})
        self.assertIn("| eob | ? | ? | ? | ? | ? | ? |", out)

    def test_failed_check_flagged(self):
        out = self.summary({"sql-run.json": sql_run_json(fails=7)})
        self.assertIn("sql-run 200 ✗ 7", out)

    def test_invalid_json_omits_section(self):
        self.assertNotIn("$sql-run", self.summary({"sql-run.json": "{not json"}))


if __name__ == "__main__":
    unittest.main()
