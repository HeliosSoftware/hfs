import os
import unittest

import legacy_points

HERE = os.path.dirname(os.path.abspath(__file__))


def doc(run, partial=False):
    with open(os.path.join(HERE, "legacy_testdata", run + ".md"), encoding="utf-8") as fh:
        return legacy_points.point_doc(fh.read(), run, "2026-10-14", "main", partial_run=partial)


class FailedRun(unittest.TestCase):
    def setUp(self):
        self.d = doc("38501925223", partial=True)

    def test_header(self):
        self.assertEqual(self.d["source"], "comparison.md")
        self.assertEqual(self.d["run"], "38501925223")
        p = self.d["legs"]["sqlite"]["point"]
        self.assertEqual((p["sha"], p["date"], p["branch"]), ("4eff688", "2026-10-14", "main"))
        self.assertTrue(p["rounded"] and p["partial_run"])
        self.assertEqual(p["source"], "comparison.md")

    def test_postgres_has_no_point(self):
        self.assertNotIn("postgres", self.d["legs"])
        self.assertEqual(len(self.d["legs"]), 5)

    def test_values_and_markers(self):
        p = self.d["legs"]["sqlite"]["point"]
        self.assertEqual(p["suites"]["import"], {"rps": 1190, "p95": 57764.3})
        self.assertEqual(p["markers"]["import"], "‡")
        self.assertEqual(p["markers"]["crud"], "†")
        self.assertEqual(p["bundles_ok"], 932)
        self.assertEqual(p["runner"], "github-agent6")
        self.assertEqual(p["crud_load"], 3.52)
        m = self.d["legs"]["mongodb"]["point"]
        self.assertEqual(m["suites"]["insert"]["rps"], 1144)
        self.assertEqual(m["suites"]["insert"]["p95"], 87.6)

    def test_reindex_never_mapped_but_noted(self):
        p = self.d["legs"]["sqlite"]["point"]
        self.assertNotIn("indexing", p["suites"])
        self.assertNotIn("reindex", p["suites"])
        self.assertEqual(set(p["suites"]), {"import", "crud", "search", "insert"})
        self.assertEqual(p["notes"], ["insert and bulk may be skewed by a preceding $reindex"])

    def test_bulk_non_es(self):
        b = self.d["legs"]["sqlite"]["point"]["bulk"]
        self.assertEqual(b, {"rps": 1239, "total": 1317, "ingest_rps": 5259, "ingest": 310, "index_eff": 1006})
        self.assertFalse(self.d["legs"]["sqlite"]["point"]["index_inline"] is not False)

    def test_bulk_es_keeps_deferred_quantities(self):
        for leg, want in (("sqlite-elasticsearch", (3062, 532, 7581, 215, 317)),
                          ("postgres-elasticsearch", (2359, 691, 4465, 365, 326)),
                          ("mongodb-elasticsearch", (1495, 1091, 2328, 700, 390))):
            p = self.d["legs"][leg]["point"]
            self.assertEqual(p["bulk"], dict(zip(("rps", "total", "ingest_rps", "ingest", "index_eff"), want)))
            self.assertIs(p["index_inline"], False)
            self.assertIn(legacy_points.ES_DEFERRED_NOTE, p["notes"])


class PostgresOnly(unittest.TestCase):
    def test_values(self):
        d = doc("38564965562")
        self.assertEqual(list(d["legs"]), ["postgres"])
        p = d["legs"]["postgres"]["point"]
        self.assertEqual(p["suites"]["import"]["rps"], 4705)
        self.assertEqual(p["bulk"]["rps"], 964)
        self.assertEqual(p["suites"]["insert"]["rps"], 493)
        self.assertEqual(p["pg_ms"], 1.54)
        self.assertEqual(p["bundles_ok"], 1000)
        self.assertFalse(p["partial_run"])
        self.assertEqual(p["bulk"]["index_eff"], 1232)
        self.assertIn("notes", p)


class BulkOnly(unittest.TestCase):
    def setUp(self):
        self.d = doc("38751206249")

    def test_missing_leg(self):
        self.assertNotIn("mongodb-elasticsearch", self.d["legs"])
        self.assertEqual(len(self.d["legs"]), 5)

    def test_only_bulk(self):
        for leg in self.d["legs"].values():
            self.assertEqual(leg["point"]["suites"], {})
            self.assertIsNone(leg["point"]["bundles_ok"])
        for name, leg in self.d["legs"].items():
            self.assertEqual("notes" in leg["point"], name.endswith("-elasticsearch"))

    def test_values(self):
        s = self.d["legs"]["sqlite"]["point"]["bulk"]
        self.assertEqual(s, {"rps": 748, "total": 2181, "ingest_rps": 2547, "ingest": 640, "index_eff": 1541})
        e = self.d["legs"]["sqlite-elasticsearch"]["point"]["bulk"]
        self.assertEqual(e["rps"], 1564)
        self.assertEqual((e["ingest_rps"], e["ingest"], e["index_eff"]), (2363, 690, 352))
        self.assertEqual(self.d["legs"]["postgres-elasticsearch"]["point"]["bulk"]["total"], 971)


class FullRun(unittest.TestCase):
    def setUp(self):
        self.d = doc("39657850695")

    def test_all_legs(self):
        self.assertEqual(set(self.d["legs"]), {"sqlite", "postgres", "mongodb", "sqlite-elasticsearch",
                                               "postgres-elasticsearch", "mongodb-elasticsearch"})

    def test_values(self):
        p = self.d["legs"]["sqlite"]["point"]
        self.assertEqual(p["suites"]["import"], {"rps": 1122, "p95": 60259.9})
        self.assertEqual(p["bulk"], {"rps": 1122, "total": 1454, "ingest_rps": 4591, "ingest": 355,
                                     "index_eff": 1099})
        self.assertEqual(p["bundles_ok"], 922)
        self.assertEqual(p["markers"]["crud"], "†")
        self.assertNotIn("notes", p)
        self.assertEqual(self.d["legs"]["mongodb"]["point"]["bulk"]["index_eff"], 1698)
        pg = self.d["legs"]["postgres"]["point"]
        self.assertEqual(pg["pg_ms"], 0.19)
        self.assertEqual(pg["suites"]["insert"], {"rps": 1425, "p95": 57.7})
        self.assertEqual(pg["markers"]["insert"], "")

    def test_es_bulk(self):
        for leg in ("sqlite-elasticsearch", "postgres-elasticsearch", "mongodb-elasticsearch"):
            b = self.d["legs"][leg]["point"]["bulk"]
            self.assertIs(self.d["legs"][leg]["point"]["index_inline"], False)
            self.assertEqual(self.d["legs"][leg]["point"]["notes"], [legacy_points.ES_DEFERRED_NOTE])
        b = self.d["legs"]["sqlite-elasticsearch"]["point"]["bulk"]
        self.assertEqual(b, {"rps": 3057, "total": 533, "ingest_rps": 7409, "ingest": 220, "index_eff": 313})
        b = self.d["legs"]["postgres-elasticsearch"]["point"]["bulk"]
        self.assertEqual(b, {"rps": 2565, "total": 636, "ingest_rps": 5175, "ingest": 315, "index_eff": 321})
        b = self.d["legs"]["mongodb-elasticsearch"]["point"]["bulk"]
        self.assertEqual(b, {"rps": 1638, "total": 996, "ingest_rps": 2527, "ingest": 645, "index_eff": 350})


class CurrentFormat(unittest.TestCase):
    def test_current_format_is_not_read(self):
        self.assertIsNone(doc("40547405940"))

    def test_minimal_current_table(self):
        text = ("Commit `abc1234` · run 1\n\n### Throughput\n\n| Suite | sqlite | postgres |\n|---|---:|---:|\n"
                "| Bulk Import (`/ui/bulk-import`: `$bulk-submit`, until searchable) (resources/s) | 795 | 707 |\n"
                "| ↳ search indexing (seconds) | 1,655 s | 1,923 s (failed) ‡ |\n"
                "| crud | 973 | 1,798 |\n")
        self.assertIsNone(legacy_points.point_doc(text, 1, "2026-10-27", "main"))

    def test_9_oct_bulk_row_read(self):
        d = doc("37986575337")
        self.assertEqual(d["legs"]["sqlite"]["point"]["bulk"]["rps"], 1455)
        self.assertEqual(d["legs"]["sqlite"]["point"]["suites"]["import"]["rps"], 1330)
        self.assertEqual(d["legs"]["sqlite"]["point"]["bulk"]["total"], 1121)

    def test_search_index_rebuild_after_completed_row(self):
        text = ("Commit `abc1234` · run 1\n\n### Throughput\n\n| Suite | sqlite | sqlite+ES |\n|---|---:|---:|\n"
                "| bulk-import (resources/s) | 700 | 900 |\n"
                "| ↳ ingest (resources/s, seconds) | 4,125 (395 s) | 3,327 (490 s) |\n"
                "| ↳ search-index rebuild after Completed (seconds) | 1,655 s | 490 s |\n")
        d = legacy_points.point_doc(text, 1, "2026-10-24", "main")
        self.assertEqual(d["legs"]["sqlite"]["point"]["bulk"]["index_eff"], 1655)
        self.assertEqual(d["legs"]["sqlite-elasticsearch"]["point"]["bulk"]["index_eff"], 490)


class Edge(unittest.TestCase):
    def test_nothing_parseable(self):
        self.assertIsNone(legacy_points.point_doc("", "1", "2026-10-01", "main"))
        self.assertIsNone(legacy_points.point_doc("## x\n\nno tables\n", "1", "2026-10-01", "main"))

    def test_na_is_none_never_zero(self):
        text = ("Commit `abc1234` · run 1\n\n### Throughput\n\n| Suite | sqlite | postgres |\n|---|---:|---:|\n"
                "| crud | 10 | n/a (no artifact) |\n| insert (creates/s) | n/a (x) | n/a (no artifact) |\n")
        d = legacy_points.point_doc(text, 1, "2026-10-01", "main")
        self.assertEqual(list(d["legs"]), ["sqlite"])
        s = d["legs"]["sqlite"]["point"]["suites"]
        self.assertEqual(s, {"crud": {"rps": 10, "p95": None}})
        self.assertIsNone(d["legs"]["sqlite"]["point"]["bulk"])

    def test_expires(self):
        self.assertEqual(legacy_points.EXPIRES, "2026-12-15")


if __name__ == "__main__":
    unittest.main()
