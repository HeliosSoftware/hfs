#!/usr/bin/env python3
#
# Unit tests for bulk_import.py: stdlib unittest, synthetic fixtures built in
# code (a tiny corpus, a fake tgz bundle server, a fake HFS). Every server is a
# ThreadingHTTPServer on 127.0.0.1:0. Not wired into CI. Run from the repo root:
#   python -m unittest discover -s .github/scripts/fhir-bench -p "test_*.py" -v
import contextlib
import gzip
import http.server
import io
import json
import os
import shutil
import sys
import tempfile
import threading
import time
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import bulk_import as bi  # noqa: E402


def write_json(path, doc):
    with open(path, "w", encoding="utf-8") as fh:
        json.dump(doc, fh)


def entry(rtype, rid, **fields):
    r = {"resourceType": rtype, "id": rid}
    r.update(fields)
    return {"fullUrl": "urn:uuid:" + rid, "resource": r, "request": {"method": "POST", "url": rtype}}


def corpus_docs():
    """name -> document: two seed bundles, a non-Bundle listing, two patient bundles."""
    hospital = {"resourceType": "Bundle", "type": "transaction", "entry": [
        entry("Organization", "o1", identifier=[{"system": "sys", "value": "A"}]),
        entry("Location", "l1", managingOrganization={"reference": "Organization?identifier=sys|A"})]}
    practitioner = {"resourceType": "Bundle", "type": "transaction", "entry": [
        entry("Practitioner", "p1", identifier=[{"system": "npi", "value": "1"}])]}
    b1 = {"resourceType": "Bundle", "type": "transaction", "entry": [
        entry("Patient", "pat1", generalPractitioner=[{"reference": "Practitioner?identifier=npi|1"}]),
        entry("Encounter", "enc1", **{"class": {"system": "x", "code": "AMB"}},
              subject={"reference": "urn:uuid:pat1"},
              serviceProvider={"reference": "Organization?identifier=sys|A"},
              location=[{"location": {"reference": "Location?identifier=sys|NOPE"}}]),
        entry("Observation", "obs1", code={"coding": [{"system": "http://loinc.org", "code": "8302-2"}]},
              subject={"reference": "urn:uuid:pat1"}, encounter={"reference": "urn:uuid:enc1"})]}
    b2 = {"resourceType": "Bundle", "type": "transaction", "entry": [
        entry("Patient", "pat1"),  # a repeat of b1's Patient: the first one wins
        entry("Observation", "obs2", code={"coding": [{"code": "other"}]},
              subject={"reference": "urn:uuid:pat1"})]}
    return {"hospitalInformation1.json": hospital, "practitionerInformation1.json": practitioner,
            "listing.json": {"files": ["A.json", "B.json"]}, "A.json": b1, "B.json": b2}


def make_corpus(path):
    os.makedirs(path, exist_ok=True)
    for name, doc in corpus_docs().items():
        write_json(os.path.join(path, name), doc)
    return path


def read_ndjson(path):
    with open(path, encoding="utf-8") as fh:
        return [json.loads(line) for line in fh]


class ConvertTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.tmp, True)

    def test_references_duplicates_split_and_expectations(self):
        src = bi.DirectorySource(make_corpus(os.path.join(self.tmp, "corpus")))
        out = os.path.join(self.tmp, "out")
        st = bi.convert(src, out, 1000, 1)
        self.assertEqual(st["bundles"], 2)            # listing.json is not a Bundle
        self.assertEqual(st["duplicates"], 1)
        self.assertEqual(st["unresolved"], 1)         # Location?identifier=sys|NOPE
        self.assertEqual(st["resources"], 7)          # 3 seeds, pat1, enc1, obs1, obs2 (the repeated pat1 dropped)
        self.assertEqual(st["per_type"]["Observation"], 2)
        self.assertEqual((st["expect_patient"], st["expect_observation"]), (1, 2))
        self.assertEqual((st["expect_observation_code"], st["expect_encounter_class"]), (1, 1))
        files = sorted(os.listdir(out))
        self.assertIn("Observation.000.ndjson", files)
        self.assertIn("Observation.001.ndjson", files)
        self.assertNotIn("Observation.002.ndjson", files)
        enc = read_ndjson(os.path.join(out, "Encounter.000.ndjson"))[0]
        self.assertEqual(enc["subject"]["reference"], "Patient/pat1")
        self.assertEqual(enc["serviceProvider"]["reference"], "Organization/o1")
        self.assertEqual(enc["location"][0]["location"]["reference"], "Location?identifier=sys|NOPE")
        self.assertEqual(read_ndjson(os.path.join(out, "Patient.000.ndjson"))[0]["generalPractitioner"][0]["reference"],
                         "Practitioner/p1")
        self.assertEqual(read_ndjson(os.path.join(out, "Location.000.ndjson"))[0]["managingOrganization"]["reference"],
                         "Organization/o1")
        obs2 = read_ndjson(os.path.join(out, "Observation.001.ndjson"))[0]
        self.assertEqual(obs2["subject"]["reference"], "Patient/pat1")  # resolved via a duplicate entry's fullUrl
        self.assertNotIn("listing.json", " ".join(files))
        # compact serialisation, one resource per line
        with open(os.path.join(out, "Patient.000.ndjson"), encoding="utf-8") as fh:
            self.assertEqual(len(fh.read().splitlines()), 1)
        provider = "http://127.0.0.1:1"
        path = bi.write_manifest(out, provider, st["outputs"])
        with open(path, encoding="utf-8") as fh:
            manifest = json.load(fh)
        self.assertFalse(manifest["requiresAccessToken"])
        by_url = dict((o["url"], o) for o in manifest["output"])
        self.assertEqual(by_url[provider + "/Observation.001.ndjson"]["count"], 1)
        self.assertEqual(by_url[provider + "/Observation.001.ndjson"]["type"], "Observation")
        self.assertEqual(sum(o["count"] for o in manifest["output"]), st["resources"])

    def test_bundle_limit_takes_the_first_in_name_order(self):
        src = bi.DirectorySource(make_corpus(os.path.join(self.tmp, "corpus")))
        st = bi.convert(src, os.path.join(self.tmp, "out"), 1, 50000)
        self.assertEqual(st["bundles"], 1)
        self.assertEqual(st["expect_observation"], 1)   # A.json only


class FakeTgz(object):
    """The benchmark's tgz server protocol: POST /reset, seeds by name, rotation on GET /."""

    def __init__(self, docs):
        self.docs = docs
        self.rotation = [docs["A.json"], docs["B.json"]]
        self.cursor = 0
        self.fail_rotation = False
        self.calls = []
        outer = self

        class H(http.server.BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def _send(self, doc):
                body = gzip.compress(json.dumps(doc).encode("utf-8"))
                self.send_response(200)
                self.send_header("Content-Type", "application/json; charset=utf-8")
                self.send_header("Content-Encoding", "gzip")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def do_POST(self):
                outer.calls.append("POST " + self.path)
                if self.path == "/reset":
                    outer.cursor = 0
                    self.send_response(204)
                    self.send_header("Content-Length", "0")
                    self.end_headers()
                else:
                    self.send_error(404)

            def do_GET(self):
                outer.calls.append("GET " + self.path)
                if self.path in ("/hospitalInformation.json", "/practitionerInformation.json"):
                    self._send(outer.docs[self.path.strip("/").replace(".json", "") + "1.json"])
                elif self.path == "/" and outer.fail_rotation:
                    self.send_error(500)
                elif self.path == "/":
                    doc = outer.rotation[outer.cursor % len(outer.rotation)]
                    outer.cursor += 1
                    self._send(doc)
                else:
                    self.send_error(404)

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), H)
        self.server.daemon_threads = True
        threading.Thread(target=self.server.serve_forever, kwargs={"poll_interval": 0.05}, daemon=True).start()
        self.url = "http://127.0.0.1:%d" % self.server.server_address[1]

    def close(self):
        self.server.shutdown()
        self.server.server_close()


class TgzSourceTest(unittest.TestCase):
    def test_reset_seeds_and_rotation(self):
        fake = FakeTgz(corpus_docs())
        self.addCleanup(fake.close)
        fake.cursor = 1  # a previous reader left the cursor mid-way: /reset must rewind it
        src = bi.open_source(fake.url)
        self.assertIsInstance(src, bi.TgzServerSource)
        seeds = src.seeds()
        self.assertEqual(fake.calls[0], "POST /reset")
        self.assertEqual(fake.calls[1:], ["GET /hospitalInformation.json", "GET /practitionerInformation.json"])
        self.assertEqual([b["entry"][0]["resource"]["id"] for b in seeds], ["o1", "p1"])
        got = list(src.bundles(2))
        self.assertEqual([b["entry"][0]["resource"]["id"] for b in got], ["pat1", "pat1"])
        self.assertEqual(got[0]["entry"][1]["resource"]["id"], "enc1")   # A.json first
        self.assertEqual(got[1]["entry"][1]["resource"]["id"], "obs2")   # then B.json
        self.assertIsInstance(bi.open_source("/some/dir"), bi.DirectorySource)

    def test_a_failed_rotation_get_is_not_retried(self):
        fake = FakeTgz(corpus_docs())
        self.addCleanup(fake.close)
        fake.fail_rotation = True
        src = bi.TgzServerSource(fake.url)
        with self.assertRaises(Exception):
            list(src.bundles(1))
        self.assertEqual(fake.calls.count("GET /"), 1)


class ProviderTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.tmp, True)
        with open(os.path.join(self.tmp, "a.ndjson"), "wb") as fh:
            fh.write(b"0123456789abcdef")
        with open(os.path.join(self.tmp, "manifest.json"), "wb") as fh:
            fh.write(b"{}")
        self.server, self.base = bi.start_provider(self.tmp)
        self.addCleanup(self.server.server_close)
        self.addCleanup(self.server.shutdown)

    def test_range_if_range_head_and_types(self):
        status, h, body = bi.http_call("GET", self.base + "/a.ndjson")
        self.assertEqual((status, body), (200, b"0123456789abcdef"))
        self.assertEqual(h["accept-ranges"], "bytes")
        self.assertEqual(h["content-type"], "application/fhir+ndjson")
        etag = h["etag"]
        self.assertTrue(etag.startswith('"'))
        status, h2, body = bi.http_call("GET", self.base + "/a.ndjson",
                                        headers={"Range": "bytes=5-", "If-Range": etag})
        self.assertEqual((status, body), (206, b"56789abcdef"))
        self.assertEqual(h2["content-range"], "bytes 5-15/16")
        status, h2, body = bi.http_call("GET", self.base + "/a.ndjson", headers={"Range": "bytes=2-4"})
        self.assertEqual((status, body, h2["content-range"]), (206, b"234", "bytes 2-4/16"))
        status, _h, body = bi.http_call("GET", self.base + "/a.ndjson",
                                        headers={"Range": "bytes=5-", "If-Range": '"stale"'})
        self.assertEqual((status, body), (200, b"0123456789abcdef"))
        status, h3, body = bi.http_call("HEAD", self.base + "/a.ndjson")
        self.assertEqual((status, body, h3["content-length"]), (200, b"", "16"))
        status, h4, _b = bi.http_call("GET", self.base + "/a.ndjson", headers={"Range": "bytes=16-"})
        self.assertEqual(status, 416)
        self.assertEqual(h4["content-range"], "bytes */16")
        status, h5, _b = bi.http_call("GET", self.base + "/manifest.json")
        self.assertEqual((status, h5["content-type"]), (200, "application/json"))
        self.assertEqual(bi.http_call("GET", self.base + "/missing.ndjson")[0], 404)


class ParseTest(unittest.TestCase):
    def test_parse_progress(self):
        self.assertEqual(bi.parse_progress(
            "Processing 42% - 1,234,567 Resources written - Downloaded 3 of 33 files"), 1234567)
        self.assertIsNone(bi.parse_progress("Queued - starting shortly"))
        self.assertIsNone(bi.parse_progress(None))

    def test_summarize_manifest(self):
        pages = [{"output": [{"count": 10}, {"count": 5}],
                  "outcome": [{"count": 4, "countSeverity": [{"code": "error", "count": 3},
                                                              {"code": "warning", "count": 1}]}]},
                 {"output": [{"count": 2}], "outcome": [{"count": 7}]}]  # no countSeverity: all errors
        self.assertEqual(bi.summarize_manifest(pages), (17, 10, 1))
        self.assertEqual(bi.summarize_manifest([{}]), (0, 0, 0))

    def test_rebase_and_bodies(self):
        self.assertEqual(bi.rebase("http://localhost:8080/bulk-submit-status/t1?page=2", "http://127.0.0.1:9/"),
                         "http://127.0.0.1:9/bulk-submit-status/t1?page=2")
        kick = bi.kickoff_body("sid", "http://p/manifest.json", "http://p")
        names = dict((p["name"], p) for p in kick["parameter"])
        self.assertEqual(names["fhirBaseUrl"]["valueUrl"], "http://p")
        self.assertEqual(names["submissionStatus"]["valueCoding"]["code"], "completed")
        self.assertEqual(names["submitter"]["valueIdentifier"]["system"], bi.SUBMITTER_SYSTEM)
        self.assertEqual(len(bi.status_body("sid")["parameter"]), 2)


LOG_STARTED = ("2026-10-09T13:25:06.962350Z  INFO helios_persistence::search::reindex: deferred reindex "
               "generation started tenant=default generation=0 job_id=e9812e92-03f7-4e1c-855a-1e36e49d19ac "
               'submission=Some("a|b") manifest=Some("m") types=["Patient"]\n')
LOG_FINISHED = ("2026-10-09T13:25:50.621499Z  INFO helios_persistence::search::reindex: reindex job finished "
                "tenant=default job_id=e9812e92-03f7-4e1c-855a-1e36e49d19ac outcome=completed types_done=24 "
                "types=24 processed=36597 total=36597 elapsed_ms=43659 resources_per_s=838.2 entries=612147 "
                "failed=0 pages=52\n")
LOG_COMPLETED = ("2026-10-09T13:25:50.650673Z  INFO helios_persistence::search::reindex: deferred reindex "
                 'generation completed tenant=default generation=0 job_id=Some("e9812e92") types=["Patient"]\n')
LOG_RETRY = ("2026-10-09T13:25:30.000000Z  WARN helios_persistence::search::reindex: deferred reindex "
             'generation failed; retrying once tenant=default generation=0 job_id=Some("e9812e92") error=x\n')
LOG_PERMANENT = ("2026-10-09T13:25:50.650673Z ERROR helios_persistence::search::reindex: deferred reindex "
                 "completed, but resources were rejected permanently and are stored but not searchable; "
                 'not retrying because a rerun fails the same way tenant=default generation=0 '
                 'job_id=Some("e9812e92") types=["Patient"] errors=3 resources=Patient/1\n')


class IndexTrackerTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.log = os.path.join(self.tmp, "hfs.log")
        with open(self.log, "w") as fh:
            fh.write("2026-10-09T13:00:00.000000Z  INFO hfs: older line\n")
        self.offset = os.path.getsize(self.log)

    def append(self, text):
        with open(self.log, "a") as fh:
            fh.write(text)

    def test_retrying_then_completed(self):
        t = bi.IndexTracker(self.log, self.offset)
        self.assertFalse(t.done(0))
        self.append(LOG_STARTED)
        t.poll()
        self.assertTrue(t.seen_any())
        self.assertFalse(t.done(0))
        self.append(LOG_RETRY)
        t.poll()
        self.assertFalse(t.done(0))                   # a retry is not final
        self.append(LOG_STARTED.replace("generation=0", "generation=1"))
        t.poll()
        self.append(LOG_FINISHED + LOG_COMPLETED)
        t.poll()
        self.assertTrue(t.done(0))
        self.assertFalse(t.done(3600))                # settle window not over yet
        kind, ts = t.final()
        self.assertEqual(kind, "completed")
        self.assertEqual(t.jobs(), 2)
        self.assertEqual(t.index_errors(), 0)
        self.assertAlmostEqual(ts, bi.parse_log_ts("2026-10-09T13:25:50.650673Z  INFO x"), places=3)

    def test_small_chunks_match_default(self):
        text = LOG_STARTED + LOG_RETRY + LOG_STARTED.replace("generation=0", "generation=1") + LOG_FINISHED + LOG_COMPLETED
        self.append(text)
        ref = bi.IndexTracker(self.log, self.offset)
        ref.poll()
        small = bi.IndexTracker(self.log, self.offset)
        small.CHUNK = 64
        small.poll()
        self.assertGreater(len(text), 4 * 64)
        self.assertEqual([(k, ts) for k, ts, _ in small.events], [(k, ts) for k, ts, _ in ref.events])
        self.assertEqual(small.finished, ref.finished)
        self.assertEqual(small.final(), ref.final())
        self.assertEqual(small.jobs(), ref.jobs())
        self.assertTrue(small.done(0))
        self.assertEqual(small.pos, ref.pos)

    def test_permanent_errors(self):
        t = bi.IndexTracker(self.log, self.offset)
        self.append(LOG_STARTED + LOG_PERMANENT)
        t.poll()
        self.assertTrue(t.done(0))
        self.assertEqual(t.final()[0], "permanent-errors")
        self.assertEqual(t.index_errors(), 3)

    def test_failed_and_cancelled(self):
        for needle, kind in (("deferred reindex failed twice; run $reindex manually", "failed"),
                             ("deferred reindex generation was cancelled", "cancelled")):
            t = bi.IndexTracker(self.log, self.offset)
            self.append("2026-10-09T13:26:00.000000Z ERROR x: %s generation=0\n" % needle)
            t.poll()
            self.assertEqual(t.final()[0], kind)

    def test_only_complete_lines_and_the_offset(self):
        t = bi.IndexTracker(self.log, self.offset)
        self.append(LOG_STARTED[:40])
        t.poll()
        self.assertFalse(t.seen_any())
        self.append(LOG_STARTED[40:])
        t.poll()
        self.assertTrue(t.seen_any())
        self.assertEqual(t.jobs(), 1)

    def test_timestamps(self):
        self.assertAlmostEqual(bi.parse_log_ts("2026-10-09T13:25:06.962350Z  INFO"), 1791552306.96235, places=4)
        self.assertAlmostEqual(bi.parse_log_ts("2026-10-09T13:25:06.962350123Z  INFO"), 1791552306.96235, places=4)
        self.assertEqual(bi.parse_log_ts("2026-10-09T13:25:06Z  INFO"), 1791552306)
        self.assertIsNone(bi.parse_log_ts("not a timestamp"))

    def test_deferred_at_startup(self):
        self.assertTrue(bi.deferred_at_startup(
            "x\n2026 INFO hfs: Bulk submit fast-load: search indexing deferred to post-manifest reindex\n"))
        self.assertFalse(bi.deferred_at_startup("nothing"))


class ResultTest(unittest.TestCase):
    def test_write_result_order_and_one_line_values(self):
        tmp = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, tmp, True)
        out = os.path.join(tmp, "r")
        bi.write_result(out, {"searchable": "yes", "status": "error", "reason": "a\nb\r\nc",
                              "env_HFS_B": "2", "env_HFS_A": "1"})
        with open(os.path.join(out, "bulk-import.txt"), encoding="utf-8") as fh:
            lines = fh.read().splitlines()
        self.assertEqual(lines[0], "status=error")
        self.assertEqual(lines[1], "reason=a b  c")
        keys = [l.split("=", 1)[0] for l in lines]
        self.assertEqual(keys[:len(bi.RESULT_KEYS)], bi.RESULT_KEYS)
        self.assertEqual(keys[len(bi.RESULT_KEYS):], ["env_HFS_A", "env_HFS_B"])
        self.assertIn("searchable=yes", lines)
        self.assertIn("backend=", lines)  # unknown values are written empty
        self.assertFalse(os.path.exists(os.path.join(out, "bulk-import.txt.tmp")))

    def test_env_record_drops_secrets(self):
        rec = bi.env_record({"HFS_BULK_SUBMIT_BATCH_SIZE": "100", "HFS_BULK_SUBMIT_PRIVATE_KEY": "pem",
                             "HFS_BULK_SUBMIT_DECRYPTION_KEY": "k", "HFS_REINDEX_BATCH_SIZE": "1000",
                             "HFS_COMPOSITE_SYNC_MODE": "synchronous", "HOME": "/root",
                             "HFS_ELASTICSEARCH_REINDEX_REFRESH": "false",
                             "HFS_REQUEST_TIMEOUT": "900", "HFS_MAX_BODY_SIZE": "209715200"})
        self.assertEqual(sorted(rec), ["env_HFS_BULK_SUBMIT_BATCH_SIZE", "env_HFS_COMPOSITE_SYNC_MODE",
                                       "env_HFS_ELASTICSEARCH_REINDEX_REFRESH", "env_HFS_MAX_BODY_SIZE",
                                       "env_HFS_REINDEX_BATCH_SIZE", "env_HFS_REQUEST_TIMEOUT"])


class FakeHfs(object):
    """Just enough of HFS: $bulk-submit, the status poll (202 twice, then 200 after
    downloading the manifest and counting the NDJSON), search totals, and the log lines."""

    def __init__(self, log_path, kickoff_status=200, finish=True, polls_before_done=2):
        self.log_path = log_path
        self.kickoff_status = kickoff_status
        self.finish = finish
        self.polls_before_done = polls_before_done
        self.polls = 0
        self.manifest_url = None
        self.resources = []   # parsed NDJSON resources once downloaded
        outer = self

        class H(http.server.BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def log_message(self, *a):
                pass

            def _reply(self, status, body=b"", headers=None):
                self.send_response(status)
                for k, v in (headers or {}).items():
                    self.send_header(k, v)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def _json(self, status, doc, headers=None):
                self._reply(status, json.dumps(doc).encode("utf-8"), headers)

            def do_POST(self):
                length = int(self.headers.get("Content-Length") or 0)
                body = json.loads(self.rfile.read(length) or b"{}")
                if self.path == "/$bulk-submit":
                    if outer.kickoff_status != 200:
                        self._json(outer.kickoff_status, {"resourceType": "OperationOutcome", "issue": [
                            {"severity": "error", "code": "not-supported", "diagnostics": "bulk submit is disabled"}]})
                        return
                    for p in body["parameter"]:
                        if p["name"] == "manifestUrl":
                            outer.manifest_url = p["valueUrl"]
                    self._json(200, {"resourceType": "Parameters", "parameter": []})
                elif self.path == "/$bulk-submit-status":
                    # The wrong host on purpose: the helper must rebase the path onto its own base.
                    self._reply(202, b"", {"Content-Location": "http://localhost:9/bulk-submit-status/t1"})
                else:
                    self._reply(404)

            def do_GET(self):
                path, _, query = self.path.partition("?")
                if path == "/metadata":
                    self._json(200, {"resourceType": "CapabilityStatement"})
                elif path == "/bulk-submit-status/t1":
                    outer.polls += 1
                    if not outer.finish or outer.polls <= outer.polls_before_done:
                        self._reply(202, b"", {"X-Progress": "Processing 10%% - %d Resources written" %
                                               (1000 * outer.polls), "Retry-After": "1"})
                        return
                    counts = outer.download()
                    outer.append_log()
                    self._json(200, {"output": [{"type": t, "count": c} for t, c in sorted(counts.items())],
                                     "outcome": [], "deleted": [], "link": []})
                elif path in ("/Patient", "/Observation", "/Encounter"):
                    self._json(200, {"resourceType": "Bundle", "type": "searchset", "total": outer.total(path[1:], query)})
                else:
                    self._reply(404)

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), H)
        self.server.daemon_threads = True
        threading.Thread(target=self.server.serve_forever, kwargs={"poll_interval": 0.05}, daemon=True).start()
        self.base = "http://127.0.0.1:%d" % self.server.server_address[1]

    def close(self):
        self.server.shutdown()
        self.server.server_close()

    def download(self):
        counts = {}
        _s, _h, raw = bi.http_call("GET", self.manifest_url)
        for o in json.loads(raw.decode("utf-8"))["output"]:
            _s, _h, data = bi.http_call("GET", o["url"])
            lines = [json.loads(l) for l in data.decode("utf-8").splitlines()]
            self.resources.extend(lines)
            counts[o["type"]] = counts.get(o["type"], 0) + len(lines)
        return counts

    def total(self, rtype, query):
        rs = [r for r in self.resources if r["resourceType"] == rtype]
        if rtype == "Observation" and query.startswith("code="):
            rs = [r for r in rs if any(c.get("code") in bi.EXPECT_OBS_CODES for c in r["code"]["coding"])]
        if rtype == "Encounter" and query.startswith("class="):
            rs = [r for r in rs if r["class"]["code"] in bi.EXPECT_ENC_CLASSES]
        return len(rs)

    def append_log(self):
        stamp = time.strftime("%Y-%m-%dT%H:%M:%S", time.gmtime()) + ".000000Z"
        with open(self.log_path, "a") as fh:
            fh.write(LOG_STARTED.replace("2026-10-09T13:25:06.962350Z", stamp))
            fh.write(LOG_FINISHED.replace("2026-10-09T13:25:50.621499Z", stamp))
            fh.write(LOG_COMPLETED.replace("2026-10-09T13:25:50.650673Z", stamp))


class MainTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.corpus = make_corpus(os.path.join(self.tmp, "corpus"))
        self.log = os.path.join(self.tmp, "hfs.log")
        with open(self.log, "w") as fh:
            fh.write("2026-10-09T13:00:00.000000Z  INFO hfs: Bulk submit fast-load: search indexing "
                     "deferred to post-manifest reindex\n")
        self.results = os.path.join(self.tmp, "results")

    def run_main(self, fake, *extra):
        argv = ["--source", self.corpus, "--results-dir", self.results, "--workdir", os.path.join(self.tmp, "work"),
                "--hfs-log", self.log, "--base-url", fake.base, "--poll-s", "0.1", "--settle-s", "0.2",
                "--index-start-grace-s", "1", "--submission-id", "t"] + list(extra)
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            self.assertEqual(bi.main(argv), 0)
        kv = {}
        with open(os.path.join(self.results, "bulk-import.txt"), encoding="utf-8") as fh:
            for line in fh.read().splitlines():
                k, _, v = line.partition("=")
                kv[k] = v
        return kv

    def fake(self, **kw):
        fake = FakeHfs(self.log, **kw)
        self.addCleanup(fake.close)
        return fake

    def test_complete(self):
        kv = self.run_main(self.fake())
        self.assertEqual((kv["status"], kv["reason"], kv["phase"]), ("complete", "", "done"))
        self.assertEqual(kv["resources_submitted"], "7")
        self.assertEqual(kv["resources_ok"], "7")
        self.assertEqual(kv["resources_failed"], "0")
        self.assertEqual(kv["index_status"], "completed")
        self.assertEqual(kv["index_jobs"], "1")
        self.assertEqual(kv["defer_indexing"], "true")
        self.assertEqual(kv["searchable"], "yes")
        self.assertEqual((kv["check_patient"], kv["expect_patient"]), ("1", "1"))
        self.assertEqual((kv["check_observation_code"], kv["expect_observation_code"]), ("1", "1"))
        self.assertEqual((kv["check_encounter_class"], kv["expect_encounter_class"]), ("1", "1"))
        self.assertGreater(float(kv["total_seconds"]), 0)
        self.assertTrue(kv["resources_per_s"])
        self.assertFalse(os.path.exists(os.path.join(self.results, "bulk-import.txt.tmp")))

    def test_unsupported_501(self):
        kv = self.run_main(self.fake(kickoff_status=501))
        self.assertEqual(kv["status"], "unsupported")
        self.assertEqual(kv["reason"], "HTTP 501 from $bulk-submit")

    def test_kickoff_error_carries_the_diagnostics(self):
        kv = self.run_main(self.fake(kickoff_status=400))
        self.assertEqual(kv["status"], "error")
        self.assertEqual(kv["reason"], "kick-off HTTP 400: bulk submit is disabled")

    def test_ingest_timeout_reports_progress(self):
        kv = self.run_main(self.fake(finish=False), "--timeout-s", "1")
        self.assertEqual((kv["status"], kv["phase"]), ("timeout", "ingest"))
        self.assertEqual(kv["reason"], "ingest after 1 s")
        self.assertRegex(kv["resources_ok"], r"^[1-9][0-9]*000$")   # from X-Progress

    def test_no_reindex_in_the_log_is_an_error_when_deferred(self):
        fake = self.fake()
        fake.append_log = lambda: None
        kv = self.run_main(fake)
        self.assertEqual((kv["status"], kv["phase"]), ("error", "index"))
        self.assertIn("no deferred reindex in the HFS log", kv["reason"])

    def test_inline_indexing_when_not_deferred(self):
        with open(self.log, "w") as fh:
            fh.write("2026-10-09T13:00:00.000000Z  INFO hfs: Starting\n")
        fake = self.fake()
        fake.append_log = lambda: None
        kv = self.run_main(fake)
        self.assertEqual((kv["status"], kv["index_status"], kv["index_seconds"]), ("complete", "inline", "0"))
        self.assertEqual(kv["defer_indexing"], "false")

    def test_skipped_leg_budget(self):
        kv = self.run_main(self.fake(), "--deadline-epoch", str(time.time() + 60))
        self.assertEqual(kv["status"], "skipped")
        self.assertRegex(kv["reason"], r"^leg budget: \d+ s left$")
        self.assertEqual(kv["phase"], "preflight")

    def test_skipped_disk(self):
        kv = self.run_main(self.fake(), "--min-free-gb", str(10 ** 9))
        self.assertEqual(kv["status"], "skipped")
        self.assertIn("runner disk", kv["reason"])

    def test_convert_failure_is_an_error_with_a_reason(self):
        self.corpus = os.path.join(self.tmp, "nope")
        kv = self.run_main(self.fake())
        self.assertEqual((kv["status"], kv["phase"]), ("error", "convert"))
        self.assertTrue(kv["reason"].startswith("convert:"))


if __name__ == "__main__":
    unittest.main()
