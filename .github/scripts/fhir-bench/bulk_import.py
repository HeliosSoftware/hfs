#!/usr/bin/env python3
#
# Bulk-import suite for the FHIR benchmark: loads the same Synthea corpus the
# `import` suite loads, through HFS's `$bulk-submit` (Bulk Data Submit, the
# bulk NDJSON ingestion path; HFS has no `$import` operation), into a FRESH,
# empty database, and records how long it took until search covers the data.
#
# What it does, in order:
#   1. Reads the corpus the way import.js does: POST /reset on the tgz bundle
#      server, the two seed bundles by name, then N rotation GETs (or, for a
#      local run, the same files from a directory).
#   2. Converts the bundles to per-resource-type NDJSON (split into parts of at
#      most --max-lines-per-file lines). `urn:uuid:` references and conditional
#      `Type?identifier=system|value` references (which only a transaction
#      Bundle can resolve) are rewritten to `Type/id`. Duplicate Type/id are
#      dropped (first wins) and counted. Counts that the search checks below
#      are compared against are computed here.
#   3. Serves the files plus a bulk export manifest from 127.0.0.1 with a
#      stdlib HTTP server that honours Range and If-Range, so HFS's resume-on-
#      broken-body path works (`python -m http.server` ignores Range).
#   4. Starts a second HFS (--hfs-bin on --port) with the caller's environment
#      (the caller points it at a fresh database), or attaches to a running one
#      (--base-url).
#   5. POSTs `$bulk-submit`, then `$bulk-submit-status`, and polls to the final
#      status manifest. Ingested = sum of the manifest's output[].count; failed
#      = submitted - ingested.
#   6. Under HFS_BULK_SUBMIT_DEFER_INDEXING (the default) status turns 200
#      before search is complete: the per-type search-index rebuild follows.
#      HFS exposes no API for it, so the clock keeps running until the bulk
#      HFS log shows the rebuild ("deferred reindex generation started" ...
#      "completed"). See the follow-up in the issue: the log lines are the
#      contract this script depends on.
#   7. After the clock stops (not timed): Patient and Observation totals plus
#      two indexed queries, compared with the counts computed in step 2.
#
# Called from: bulk-import.sh (the "Run benchmark suites" step of
# fhir-benchmark.yml runs that, after the search suite).
#
# Local usage:
#   python3 bulk_import.py --source <dir of Synthea bundles | http://tgz-host:port> \
#       --hfs-bin ./hfs --port 18171 --workdir /tmp/bulk --results-dir /tmp/bulk/results \
#       --hfs-log /tmp/bulk/hfs.log --bundles 30
#   (HFS_STORAGE_BACKEND, HFS_DATABASE_URL, HFS_DATA_DIR, HFS_BASE_URL ... come from the
#   environment; bulk-import.sh sets them per backend.)
#   python3 bulk_import.py --source <dir> --workdir /tmp/x --convert-only   # stats only
#
# Inputs (flags): --source --results-dir --workdir --hfs-log (required; the
# last two only when HFS is started or attached), --hfs-bin + --port | --base-url,
# --bundles (1000), --max-lines-per-file (50000), --timeout-s (3600),
# --deadline-epoch (0 = none), --min-free-gb (6), --ready-timeout-s (600),
# --poll-s (5), --index-start-grace-s (120), --settle-s (10), --submission-id,
# --convert-only.
#
# Output: <results-dir>/bulk-import.txt, key=value lines in RESULT_KEYS order,
# then the env_<NAME>=value lines of the HFS_BULK_SUBMIT_* / HFS_REINDEX_* /
# HFS_ELASTICSEARCH_REINDEX_* settings in effect. Always written, whatever
# happens. status is complete | timeout | error | unsupported | skipped, with a
# reason, and the phase reached. stdout: progress lines and one summary line.
# Exit code: 0 once the result file is written (the verdict is in the file).
#
# Python 3.8-compatible, standard library only.
import argparse
import calendar
import functools
import gzip
import http.client
import http.server
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.parse
import urllib.request

SEED_PREFIXES = ("hospitalInformation", "practitionerInformation")
RESULT_FILE = "bulk-import.txt"
SUBMITTER_SYSTEM = "https://helios.software/fhir-benchmark"
# A reference only a transaction Bundle can resolve: urn:uuid:<id> (to another
# entry) or Type?identifier=system|value (conditional, to a seed resource).
REF_RE = re.compile(r'"reference":"((?:urn:uuid:[^"]+)|(?:[A-Za-z]+\?identifier=[^"]+))"')
# The same predicates as suite-lib.sh write_search_counts, so the checks below
# compare like with like: Observation?code=8302-2,29463-7 and
# Encounter?class=AMB,EMER.
EXPECT_OBS_CODES = {"8302-2", "29463-7"}
EXPECT_ENC_CLASSES = {"AMB", "EMER"}
VERIFY_QUERIES = [
    ("patient", "Patient?_summary=count"),
    ("observation", "Observation?_summary=count"),
    ("observation_code", "Observation?code=8302-2,29463-7&_summary=count"),
    ("encounter_class", "Encounter?class=AMB,EMER&_summary=count"),
]
RESULT_KEYS = [
    "status", "reason", "phase", "backend", "source", "bundles", "resources_submitted",
    "duplicate_resources", "unresolved_references", "ndjson_files", "ndjson_bytes",
    "convert_seconds", "hfs_ready_seconds", "defer_indexing", "timeout_seconds",
    "resources_ok", "resources_failed", "outcome_errors", "outcome_warnings",
    "ingest_seconds", "index_status", "index_jobs", "index_errors", "index_seconds",
    "total_seconds", "resources_per_s", "ingest_resources_per_s",
    "expect_patient", "check_patient", "expect_observation", "check_observation",
    "expect_observation_code", "check_observation_code",
    "expect_encounter_class", "check_encounter_class", "searchable",
]
ENV_RECORD_PREFIXES = ("HFS_BULK_SUBMIT_", "HFS_REINDEX_", "HFS_ELASTICSEARCH_REINDEX_")
ENV_RECORD_NAMES = ("HFS_COMPOSITE_SYNC_MODE", "HFS_ELASTICSEARCH_WRITE_REFRESH",
                    "HFS_REQUEST_TIMEOUT", "HFS_MAX_BODY_SIZE")
# Never write these into an artifact (HFS_BULK_SUBMIT_PRIVATE_KEY / _DECRYPTION_KEY).
ENV_SECRET_RE = re.compile(r"KEY|SECRET|TOKEN|PASSWORD|PRIVATE", re.I)

DEFER_LINE = "Bulk submit fast-load: search indexing deferred to post-manifest reindex"
ANSI_RE = re.compile(r"\x1b\[[0-9;]*m")
TS_RE = re.compile(r"^(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d)(?:\.(\d+))?(?:Z|[+-]00:?00)?\s")

# Opener without proxy handling: every URL here is on 127.0.0.1 or the tgz host.
_OPENER = urllib.request.build_opener(urllib.request.ProxyHandler({}))


class Terminated(Exception):
    """SIGTERM from bulk-import.sh's `timeout` backstop."""


# ── HTTP helpers ────────────────────────────────────────────────────

def http_call(method, url, body=None, timeout=60, headers=None):
    """(status, lower-cased headers, bytes). HTTPError is returned, not raised."""
    data = None
    hdrs = {"Accept": "application/fhir+json, application/json"}
    if headers:
        hdrs.update(headers)
    if body is not None:
        if isinstance(body, (bytes, bytearray)):
            data = bytes(body)
        else:
            data = json.dumps(body).encode("utf-8")
            hdrs.setdefault("Content-Type", "application/fhir+json")
    req = urllib.request.Request(url, data=data, method=method, headers=hdrs)
    try:
        with _OPENER.open(req, timeout=timeout) as resp:
            return resp.status, dict((k.lower(), v) for k, v in resp.headers.items()), resp.read()
    except urllib.error.HTTPError as e:
        try:
            raw = e.read()
        except Exception:
            raw = b""
        return e.code, dict((k.lower(), v) for k, v in e.headers.items()), raw


def kickoff_body(submission_id, manifest_url, provider_base):
    return {"resourceType": "Parameters", "parameter": [
        {"name": "submitter", "valueIdentifier": {"system": SUBMITTER_SYSTEM, "value": "bulk-import"}},
        {"name": "submissionId", "valueString": submission_id},
        {"name": "manifestUrl", "valueUrl": manifest_url},
        {"name": "fhirBaseUrl", "valueUrl": provider_base},
        {"name": "submissionStatus", "valueCoding": {
            "system": "http://hl7.org/fhir/event-status", "code": "completed"}},
    ]}


def status_body(submission_id):
    return {"resourceType": "Parameters", "parameter": [
        {"name": "submitter", "valueIdentifier": {"system": SUBMITTER_SYSTEM, "value": "bulk-import"}},
        {"name": "submissionId", "valueString": submission_id},
    ]}


def rebase(url, base):
    """Path and query of `url` on `base`: HFS advertises its own HFS_BASE_URL."""
    parts = urllib.parse.urlsplit(url)
    out = base.rstrip("/") + (parts.path or "/")
    if parts.query:
        out += "?" + parts.query
    return out


def operation_outcome_text(raw, limit=100):
    try:
        doc = json.loads(raw.decode("utf-8", "replace"))
        for issue in doc.get("issue") or []:
            text = issue.get("diagnostics") or (issue.get("details") or {}).get("text")
            if text:
                return str(text)[:limit]
    except Exception:
        pass
    return raw.decode("utf-8", "replace").strip()[:limit]


# ── Corpus sources ──────────────────────────────────────────────────

def _decode_body(headers, body):
    if headers.get("content-encoding", "").lower() == "gzip":
        body = gzip.decompress(body)
    return json.loads(body.decode("utf-8"))


class TgzServerSource(object):
    """The benchmark's tgz bundle server, read with its own protocol (import.js)."""

    def __init__(self, url):
        self.url = url.rstrip("/")
        self.label = self.url

    def seeds(self):
        status, _h, _b = http_call("POST", self.url + "/reset", body=b"", timeout=120)
        if not 200 <= status < 300:
            raise RuntimeError("tgz /reset returned HTTP %d" % status)
        out = []
        for name in ("hospitalInformation.json", "practitionerInformation.json"):
            status, h, b = http_call("GET", "%s/%s" % (self.url, name), timeout=120)
            if status != 200:
                raise RuntimeError("tgz %s returned HTTP %d" % (name, status))
            out.append(_decode_body(h, b))
        return out

    def bundles(self, n):
        for _ in range(n):
            # No retry: the server advances its cursor per GET, so a retry would skip a bundle.
            status, h, b = http_call("GET", self.url + "/", timeout=120)
            if status != 200:
                raise RuntimeError("tgz rotation GET returned HTTP %d" % status)
            yield _decode_body(h, b)


class DirectorySource(object):
    """A directory of the extracted corpus: seeds by name prefix, bundles in Go
    os.ReadDir order (byte order of the names)."""

    def __init__(self, path):
        self.path = path
        self.label = path

    def _names(self):
        return sorted(f for f in os.listdir(self.path) if f.endswith(".json"))

    def _load(self, name):
        try:
            with open(os.path.join(self.path, name), encoding="utf-8") as fh:
                doc = json.load(fh)
        except (OSError, ValueError):
            return None
        if isinstance(doc, dict) and doc.get("resourceType") == "Bundle":
            return doc
        return None

    def seeds(self):
        out = []
        for name in self._names():
            if name.startswith(SEED_PREFIXES):
                doc = self._load(name)
                if doc is not None:
                    out.append(doc)
        return out

    def bundles(self, n):
        taken = 0
        for name in self._names():
            if taken >= n:
                return
            if name.startswith(SEED_PREFIXES):
                continue
            doc = self._load(name)  # listing.json and friends are not Bundles
            if doc is None:
                continue
            taken += 1
            yield doc


def open_source(spec):
    if spec.startswith(("http://", "https://")):
        return TgzServerSource(spec)
    return DirectorySource(spec)


# ── Conversion ──────────────────────────────────────────────────────

def identifier_index(seed_bundles):
    """{"Type?identifier=system|value": "Type/id"} for every seed identifier."""
    idx = {}
    for bundle in seed_bundles:
        for entry in bundle.get("entry") or []:
            r = entry.get("resource")
            if not isinstance(r, dict) or not r.get("resourceType") or r.get("id") is None:
                continue
            for ident in r.get("identifier") or []:
                if isinstance(ident, dict):
                    key = "%s?identifier=%s|%s" % (r["resourceType"], ident.get("system"), ident.get("value"))
                    idx.setdefault(key, "%s/%s" % (r["resourceType"], r["id"]))
    return idx


class NdjsonWriter(object):
    def __init__(self, out_dir, max_lines):
        self.out_dir = out_dir
        self.max_lines = max(1, int(max_lines))
        self._cur = {}      # type -> current file record
        self._parts = {}    # type -> parts started
        self._files = []

    def write(self, rtype, line):
        rec = self._cur.get(rtype)
        if rec is None or rec["count"] >= self.max_lines:
            if rec is not None:
                rec["fh"].close()
            n = self._parts.get(rtype, 0)
            self._parts[rtype] = n + 1
            name = "%s.%03d.ndjson" % (rtype, n)
            rec = {"type": rtype, "name": name, "count": 0, "bytes": 0,
                   "fh": open(os.path.join(self.out_dir, name), "wb")}
            self._cur[rtype] = rec
            self._files.append(rec)
        data = line.encode("utf-8") + b"\n"
        rec["fh"].write(data)
        rec["count"] += 1
        rec["bytes"] += len(data)

    def close(self):
        for rec in self._cur.values():
            if not rec["fh"].closed:
                rec["fh"].close()

    def outputs(self):
        return sorted(((r["type"], r["name"], r["count"], r["bytes"]) for r in self._files),
                      key=lambda t: t[1])


def convert(source, out_dir, bundles, max_lines):
    t0 = time.time()
    os.makedirs(out_dir, exist_ok=True)
    seeds = source.seeds()
    index = identifier_index(seeds)
    writer = NdjsonWriter(out_dir, max_lines)
    seen = set()
    st = {"per_type": {}, "resources": 0, "duplicates": 0, "unresolved": 0,
          "expect_patient": 0, "expect_observation": 0,
          "expect_observation_code": 0, "expect_encounter_class": 0}

    def handle(bundle):
        entries = bundle.get("entry") or []
        by_url = {}
        for entry in entries:
            r = entry.get("resource")
            if isinstance(r, dict) and r.get("resourceType") and r.get("id") is not None:
                by_url[entry.get("fullUrl") or ""] = "%s/%s" % (r["resourceType"], r["id"])

        def sub(m):
            ref = m.group(1)
            target = by_url.get(ref) if ref.startswith("urn:uuid:") else index.get(ref)
            if target is None:
                st["unresolved"] += 1
                return m.group(0)
            return '"reference":"%s"' % target

        for entry in entries:
            r = entry.get("resource")
            if not isinstance(r, dict) or not r.get("resourceType") or r.get("id") is None:
                continue
            rtype = r["resourceType"]
            key = "%s/%s" % (rtype, r["id"])
            if key in seen:
                st["duplicates"] += 1
                continue
            seen.add(key)
            line = REF_RE.sub(sub, json.dumps(r, separators=(",", ":"), ensure_ascii=False))
            writer.write(rtype, line)
            st["resources"] += 1
            st["per_type"][rtype] = st["per_type"].get(rtype, 0) + 1
            if rtype == "Patient":
                st["expect_patient"] += 1
            elif rtype == "Observation":
                st["expect_observation"] += 1
                codes = ((r.get("code") or {}).get("coding") or [])
                if any(isinstance(c, dict) and c.get("code") in EXPECT_OBS_CODES for c in codes):
                    st["expect_observation_code"] += 1
            elif rtype == "Encounter":
                cls = r.get("class")
                if isinstance(cls, dict) and cls.get("code") in EXPECT_ENC_CLASSES:
                    st["expect_encounter_class"] += 1

    n = 0
    try:
        for b in seeds:
            handle(b)
        for b in source.bundles(bundles):
            handle(b)
            n += 1
    finally:
        writer.close()
    outputs = writer.outputs()
    st["files"] = len(outputs)
    st["bytes"] = sum(o[3] for o in outputs)
    st["bundles"] = n
    st["seconds"] = round(time.time() - t0, 1)
    st["outputs"] = outputs
    return st


def write_manifest(out_dir, provider_base, outputs):
    """A bulk export manifest over the NDJSON parts; returns its path."""
    manifest = {
        "transactionTime": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "request": provider_base + "/$export",
        "requiresAccessToken": False,
        "output": [{"type": t, "url": "%s/%s" % (provider_base, name), "count": count}
                   for (t, name, count, _bytes) in outputs],
        "error": [],
    }
    path = os.path.join(out_dir, "manifest.json")
    with open(path, "w", encoding="utf-8") as fh:
        json.dump(manifest, fh)
    return path


# ── File provider (Range-capable) ───────────────────────────────────

class RangeHandler(http.server.SimpleHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    _range_len = None

    def log_message(self, *args):
        pass

    def send_head(self):
        path = self.translate_path(self.path)
        if not os.path.isfile(path):
            self.send_error(404, "File not found")
            return None
        try:
            fh = open(path, "rb")
        except OSError:
            self.send_error(404, "File not found")
            return None
        st = os.fstat(fh.fileno())
        size = st.st_size
        etag = '"%d-%d"' % (size, st.st_mtime_ns)
        last_modified = self.date_time_string(int(st.st_mtime))
        start, end, partial = 0, size - 1, False
        rng = self.headers.get("Range")
        if rng:
            if_range = (self.headers.get("If-Range") or "").strip()
            fresh = (not if_range) or if_range == etag or if_range == last_modified
            m = re.fullmatch(r"bytes=(\d*)-(\d*)", rng.strip()) if fresh else None
            if m and (m.group(1) or m.group(2)):
                if m.group(1):
                    start = int(m.group(1))
                    end = min(int(m.group(2)), size - 1) if m.group(2) else size - 1
                else:  # suffix range: the last N bytes
                    start = max(0, size - int(m.group(2)))
                    end = size - 1
                if start >= size or start > end:
                    fh.close()
                    self.send_response(416)
                    self.send_header("Content-Range", "bytes */%d" % size)
                    self.send_header("Content-Length", "0")
                    self.end_headers()
                    return None
                partial = True
        if self.path.split("?", 1)[0].endswith(".ndjson"):
            ctype = "application/fhir+ndjson"
        else:
            ctype = "application/json"
        length = end - start + 1 if size else 0
        self.send_response(206 if partial else 200)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(length))
        self.send_header("Accept-Ranges", "bytes")
        self.send_header("ETag", etag)
        self.send_header("Last-Modified", last_modified)
        if partial:
            self.send_header("Content-Range", "bytes %d-%d/%d" % (start, end, size))
        self.end_headers()
        fh.seek(start)
        self._range_len = length
        return fh

    def copyfile(self, source, outputfile):
        remaining = self._range_len
        while remaining is None or remaining > 0:
            chunk = source.read(65536 if remaining is None else min(65536, remaining))
            if not chunk:
                break
            outputfile.write(chunk)
            if remaining is not None:
                remaining -= len(chunk)


def start_provider(directory):
    """(server, base_url): `directory` served on 127.0.0.1 from a daemon thread."""
    handler = functools.partial(RangeHandler, directory=directory)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    server.daemon_threads = True
    threading.Thread(target=server.serve_forever, kwargs={"poll_interval": 0.05}, daemon=True).start()
    return server, "http://127.0.0.1:%d" % server.server_address[1]


# ── The bulk HFS process ────────────────────────────────────────────

class HfsProcess(object):
    def __init__(self, hfs_bin, log_path, cwd):
        self.hfs_bin = hfs_bin
        self.log_path = log_path
        self.cwd = cwd
        self.proc = None
        self._log = None

    def start(self):
        self._log = open(self.log_path, "ab")
        self.proc = subprocess.Popen([self.hfs_bin], env=dict(os.environ), cwd=self.cwd,
                                     stdout=self._log, stderr=subprocess.STDOUT,
                                     start_new_session=True)

    def wait_ready(self, base, timeout):
        deadline = time.time() + timeout
        while time.time() < deadline:
            if self.proc.poll() is not None:
                return False
            try:
                status, _h, _b = http_call("GET", base + "/metadata", timeout=10)
                if status == 200:
                    return True
            except (OSError, http.client.HTTPException):
                pass
            time.sleep(2)
        return False

    def stop(self):
        if self.proc is not None and self.proc.poll() is None:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=30)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                self.proc.wait()
        if self._log is not None:
            self._log.close()


# ── Parsing ─────────────────────────────────────────────────────────

def parse_progress(x_progress):
    """Resources written so far from an X-Progress header, else None."""
    m = re.search(r"([\d,]+) Resources written", x_progress or "")
    return int(m.group(1).replace(",", "")) if m else None


def summarize_manifest(pages):
    """(ok, errors, warnings) over the status-manifest pages."""
    ok = errors = warnings = 0
    for page in pages:
        for o in page.get("output") or []:
            c = o.get("count")
            ok += c if isinstance(c, int) else 0
        for o in page.get("outcome") or []:
            sev = o.get("countSeverity")
            if not isinstance(sev, list):
                errors += o.get("count") if isinstance(o.get("count"), int) else 0
                continue
            for s in sev:
                c = s.get("count") if isinstance(s.get("count"), int) else 0
                if s.get("code") in ("error", "fatal"):
                    errors += c
                elif s.get("code") == "warning":
                    warnings += c
    return ok, errors, warnings


def parse_log_ts(line):
    """Epoch seconds from a log line's leading UTC timestamp, else None."""
    m = TS_RE.match(line)
    if not m:
        return None
    try:
        secs = calendar.timegm(time.strptime(m.group(1), "%Y-%m-%dT%H:%M:%S"))
    except ValueError:
        return None
    frac = m.group(2)
    return secs + (float("0." + frac) if frac else 0.0)


class IndexTracker(object):
    """Follows the deferred search-index rebuild through the bulk HFS log.

    The strings come from helios-persistence reindex.rs (the generation
    started / completed / failed lines, ~2015 and ~2140-2189) and the job
    summary line; the tests pin today's wording."""

    STARTED_RE = re.compile(r"deferred reindex generation started\b.*?\bgeneration=(\d+).*?\bjob_id=([0-9A-Za-z-]+)")
    ERRORS_RE = re.compile(r"\berrors=(\d+)")
    FINISHED = "reindex job finished"
    FINAL = {
        "deferred reindex generation completed": "completed",
        "deferred reindex completed, but resources were rejected permanently": "permanent-errors",
        "deferred reindex failed twice": "failed",
        "deferred reindex coordinator closed": "failed",
        "deferred reindex generation was cancelled": "cancelled",
    }
    RETRYING = "deferred reindex generation failed; retrying once"
    CHUNK = 8 * 1024 * 1024

    def __init__(self, log_path, offset=0):
        self.log_path = log_path
        self.pos = offset
        self.events = []        # (kind, log epoch, extra) of started / retrying / final events
        self.finished = []      # (job_id, outcome, processed, total, failed)
        self.last_seen = None   # wall time the latest started/retrying/final event was read
        self._partial = b""

    def poll(self):
        # Bounded reads: a *-elasticsearch leg's log can gain hundreds of MB
        # between kick-off and the first poll.
        now = time.time()
        try:
            with open(self.log_path, "rb") as fh:
                fh.seek(self.pos)
                while True:
                    data = fh.read(self.CHUNK)
                    if not data:
                        break
                    self.pos += len(data)
                    data = self._partial + data
                    cut = data.rfind(b"\n")
                    if cut < 0:
                        self._partial = data
                    else:
                        self._partial = data[cut + 1:]
                        for raw in data[:cut].decode("utf-8", "replace").splitlines():
                            self._line(ANSI_RE.sub("", raw), now)
                    if len(self._partial) > 4 * self.CHUNK:
                        self._partial = b""   # no tracked event line is that long
        except OSError:
            return

    def _line(self, line, now):
        ts = parse_log_ts(line)
        if ts is None:
            ts = now
        if self.FINISHED in line:
            def grab(name):
                m = re.search(r"\b%s=(\S+)" % name, line)
                return m.group(1) if m else ""
            self.finished.append((grab("job_id"), grab("outcome"), grab("processed"),
                                  grab("total"), grab("failed")))
            return
        m = self.STARTED_RE.search(line)
        if m:
            self.events.append(("started", ts, m.group(2)))
            self.last_seen = now
            return
        if self.RETRYING in line:
            self.events.append(("retrying", ts, None))
            self.last_seen = now
            return
        for needle, kind in self.FINAL.items():
            if needle in line:
                extra = None
                if kind == "permanent-errors":
                    em = self.ERRORS_RE.search(line)
                    extra = int(em.group(1)) if em else None
                self.events.append((kind, ts, extra))
                self.last_seen = now
                return

    def seen_any(self):
        return bool(self.events)

    def done(self, settle_s):
        """The latest event is a final one and nothing new arrived for settle_s."""
        if not self.events or self.last_seen is None:
            return False
        if self.events[-1][0] in ("started", "retrying"):
            return False
        return time.time() - self.last_seen >= settle_s

    def jobs(self):
        return sum(1 for e in self.events if e[0] == "started")

    def final(self):
        """(status, log epoch) of the last final event, else (None, None)."""
        for kind, ts, _extra in reversed(self.events):
            if kind not in ("started", "retrying"):
                return kind, ts
        return None, None

    def index_errors(self):
        for kind, _ts, extra in reversed(self.events):
            if kind == "permanent-errors" and extra is not None:
                return extra
        if self.finished:
            try:
                return int(self.finished[-1][4])
            except ValueError:
                return 0
        return 0


def deferred_at_startup(text):
    return DEFER_LINE in text


# ── Verification ────────────────────────────────────────────────────

def _total(base, query):
    status, _h, raw = http_call("GET", "%s/%s" % (base, query), timeout=120)
    if status != 200:
        return None
    try:
        total = json.loads(raw.decode("utf-8")).get("total")
    except ValueError:
        return None
    return total if isinstance(total, int) else None


def verify(base, expect, retries=3, wait_s=5):
    """({check_<name>: total or None}, "yes" | "no" | "unknown")."""
    checks = {}
    for name, query in VERIFY_QUERIES:
        want = expect.get("expect_" + name)
        got = None
        for attempt in range(retries + 1):
            try:
                got = _total(base, query)
            except (OSError, http.client.HTTPException):
                got = None
            if got is not None and got == want:
                break
            if attempt < retries:
                time.sleep(wait_s)
        checks["check_" + name] = got
    if any(v is None for v in checks.values()):
        return checks, "unknown"
    ok = all(checks["check_" + n] == expect.get("expect_" + n) for n, _q in VERIFY_QUERIES)
    return checks, "yes" if ok else "no"


# ── Result file ─────────────────────────────────────────────────────

def _oneline(v):
    return str(v).replace("\r", " ").replace("\n", " ")


def env_record(environ):
    out = {}
    for k, v in environ.items():
        if ENV_SECRET_RE.search(k):
            continue
        if k.startswith(ENV_RECORD_PREFIXES) or k in ENV_RECORD_NAMES:
            out["env_" + k] = v
    return out


def write_result(results_dir, fields):
    os.makedirs(results_dir, exist_ok=True)
    lines = []
    for key in RESULT_KEYS:
        v = fields.get(key)
        lines.append("%s=%s" % (key, "" if v is None else _oneline(v)))
    for key in sorted(k for k in fields if k.startswith("env_")):
        lines.append("%s=%s" % (key, _oneline(fields[key])))
    path = os.path.join(results_dir, RESULT_FILE)
    with open(path + ".tmp", "w", encoding="utf-8") as fh:
        fh.write("\n".join(lines) + "\n")
    os.replace(path + ".tmp", path)


def _n(v):
    return "{:,}".format(v) if isinstance(v, int) else str(v)


# ── Main ────────────────────────────────────────────────────────────

def parse_args(argv):
    p = argparse.ArgumentParser(description="Bulk-import ($bulk-submit) suite for the FHIR benchmark")
    p.add_argument("--source", required=True)
    p.add_argument("--results-dir")
    p.add_argument("--workdir", required=True)
    p.add_argument("--hfs-log")
    p.add_argument("--hfs-bin")
    p.add_argument("--port", type=int)
    p.add_argument("--base-url")
    p.add_argument("--bundles", type=int, default=1000)
    p.add_argument("--max-lines-per-file", type=int, default=50000)
    p.add_argument("--timeout-s", type=float, default=3600)
    p.add_argument("--deadline-epoch", type=float, default=0)
    p.add_argument("--min-free-gb", type=float, default=6)
    p.add_argument("--ready-timeout-s", type=float, default=600)
    p.add_argument("--poll-s", type=float, default=5)
    p.add_argument("--index-start-grace-s", type=float, default=120)
    p.add_argument("--settle-s", type=float, default=10)
    p.add_argument("--submission-id", default="hfs-bench-bulk-import")
    p.add_argument("--convert-only", action="store_true")
    args = p.parse_args(argv)
    if not args.convert_only:
        if not args.results_dir or not args.hfs_log:
            p.error("--results-dir and --hfs-log are required")
        if not args.base_url and not (args.hfs_bin and args.port):
            p.error("give --hfs-bin and --port, or --base-url")
    return args


class _Stop(Exception):
    """Ends the run early with a status and reason (the result is still written)."""

    def __init__(self, status, reason):
        Exception.__init__(self, reason)
        self.status = status
        self.reason = reason


def _on_sigterm(_signum, _frame):
    raise Terminated()


def _poll_status(args, status_url, t0, cap, f):
    """Poll to the final status manifest. Returns (pages, ingest_end_epoch)."""
    last_progress = None
    last_text = None
    bad = 0
    while True:
        if time.time() - t0 >= cap:
            f["resources_ok"] = last_progress
            raise _Stop("timeout", "ingest after %s s" % _n(int(cap)))
        try:
            status, h, body = http_call("GET", status_url, timeout=60)
        except (OSError, http.client.HTTPException) as e:
            status, h, body = None, {}, b""
            err = type(e).__name__
        else:
            err = None
        if status == 200:
            ingest_end = time.time()
            pages = []
            page_url = status_url
            for _ in range(1000):
                try:
                    pages.append(json.loads(body.decode("utf-8")))
                except ValueError:
                    raise _Stop("error", "status manifest is not JSON")
                nxt = None
                for link in pages[-1].get("link") or []:
                    if isinstance(link, dict) and link.get("relation") == "next" and link.get("url"):
                        nxt = rebase(link["url"], args._base)
                if not nxt or nxt == page_url:
                    break
                page_url = nxt
                st2, _h2, body = http_call("GET", page_url, timeout=60)
                if st2 != 200:
                    raise _Stop("error", "status manifest page HTTP %s" % st2)
            return pages, ingest_end
        if status == 202:
            bad = 0
            text = h.get("x-progress")
            if text and text != last_text:
                print("  progress: %s (%d s)" % (text, int(time.time() - t0)), flush=True)
                last_text = text
            prog = parse_progress(text)
            if prog is not None:
                last_progress = prog
        elif status == 429:
            try:
                wait = min(int(h.get("retry-after", "5")), 60)
            except ValueError:
                wait = 5
            time.sleep(max(wait, 0))
            continue
        else:
            bad += 1
            if bad >= 5:
                raise _Stop("error", "status poll %s" % (("HTTP %d" % status) if status else err))
        time.sleep(args.poll_s)


def run(args, f):
    """The whole flow; fills `f` and raises _Stop for a clean early end."""
    os.makedirs(args.workdir, exist_ok=True)
    now = time.time()
    if args.deadline_epoch and args.deadline_epoch - now < 900:
        raise _Stop("skipped", "leg budget: %s s left" % _n(max(0, int(args.deadline_epoch - now))))
    free_gb = shutil.disk_usage(args.workdir).free / 1e9
    if free_gb < args.min_free_gb:
        raise _Stop("skipped", "runner disk: %.1f GB free, %g GB needed" % (free_gb, args.min_free_gb))

    f["phase"] = "convert"
    ndjson = os.path.join(args.workdir, "ndjson")
    shutil.rmtree(ndjson, ignore_errors=True)
    print("convert: %s -> %s" % (args.source, ndjson), flush=True)
    try:
        stats = convert(open_source(args.source), ndjson, args.bundles, args.max_lines_per_file)
    except Exception as e:
        raise _Stop("error", ("convert: %s" % e)[:80])
    f.update({"bundles": stats["bundles"], "resources_submitted": stats["resources"],
              "duplicate_resources": stats["duplicates"], "unresolved_references": stats["unresolved"],
              "ndjson_files": stats["files"], "ndjson_bytes": stats["bytes"],
              "convert_seconds": stats["seconds"]})
    for k in ("expect_patient", "expect_observation", "expect_observation_code", "expect_encounter_class"):
        f[k] = stats[k]
    print("convert: %s resources, %s files, %.1f GB in %s s (%s duplicates, %s unresolved)" % (
        _n(stats["resources"]), stats["files"], stats["bytes"] / 1e9, stats["seconds"],
        stats["duplicates"], stats["unresolved"]), flush=True)

    server, provider = start_provider(ndjson)
    try:
        manifest = write_manifest(ndjson, provider, stats["outputs"])
        manifest_url = "%s/%s" % (provider, os.path.basename(manifest))
        _ingest(args, f, stats, provider, manifest_url)
    finally:
        server.shutdown()
        server.server_close()


def _ingest(args, f, stats, provider, manifest_url):
    hfs = None
    if args.base_url:
        base = args.base_url.rstrip("/")
    else:
        base = "http://127.0.0.1:%d" % args.port
    args._base = base
    try:
        if not args.base_url:
            f["phase"] = "start"
            hfs = HfsProcess(args.hfs_bin, args.hfs_log, args.workdir)
            hfs.start()
            t_start = time.time()
            if not hfs.wait_ready(base, args.ready_timeout_s):
                raise _Stop("error", "bulk HFS not ready after %s s" % _n(int(time.time() - t_start)))
            f["hfs_ready_seconds"] = int(time.time() - t_start)
        try:
            with open(args.hfs_log, "rb") as fh:
                f["defer_indexing"] = "true" if deferred_at_startup(fh.read().decode("utf-8", "replace")) else "false"
        except OSError:
            f["defer_indexing"] = ""
        _drive(args, f, stats, base, manifest_url, provider)
    finally:
        if hfs is not None:
            hfs.stop()


def _drive(args, f, stats, base, manifest_url, provider):
    cap = args.timeout_s
    if args.deadline_epoch:
        left = args.deadline_epoch - time.time()
        if left < 300:  # conversion and start-up may have used the budget
            raise _Stop("skipped", "leg budget exhausted before kick-off (%s s left)" % _n(max(0, int(left))))
        cap = min(cap, left)
    f["timeout_seconds"] = int(cap)

    f["phase"] = "ingest"
    try:
        offset = os.path.getsize(args.hfs_log)
    except OSError:
        offset = 0
    tracker = IndexTracker(args.hfs_log, offset)
    t0 = time.time()
    status, _h, body = http_call("POST", base + "/$bulk-submit",
                                 kickoff_body(args.submission_id, manifest_url, provider), timeout=120)
    if status == 501:
        raise _Stop("unsupported", "HTTP 501 from $bulk-submit")
    if not 200 <= status < 300:
        raise _Stop("error", ("kick-off HTTP %d: %s" % (status, operation_outcome_text(body)))[:80])
    status, h, body = http_call("POST", base + "/$bulk-submit-status", status_body(args.submission_id), timeout=120)
    loc = h.get("content-location")
    if not loc:
        raise _Stop("error", ("status kick-off HTTP %d without Content-Location" % status)[:80])
    status_url = rebase(loc, base)
    print("kick-off accepted; polling %s" % status_url, flush=True)

    pages, ingest_end = _poll_status(args, status_url, t0, cap, f)
    ok, errors, warnings = summarize_manifest(pages)
    f["resources_ok"] = ok
    f["resources_failed"] = f["resources_submitted"] - ok
    f["outcome_errors"] = errors
    f["outcome_warnings"] = warnings
    f["ingest_seconds"] = round(ingest_end - t0, 1)
    if f["ingest_seconds"] > 0:
        f["ingest_resources_per_s"] = "%.1f" % (ok / f["ingest_seconds"])
    print("ingest done: %s of %s resources in %s s" % (
        _n(ok), _n(f["resources_submitted"]), f["ingest_seconds"]), flush=True)

    f["phase"] = "index"
    grace_end = ingest_end + args.index_start_grace_s
    while True:
        tracker.poll()
        if tracker.done(args.settle_s):
            break
        if time.time() - t0 >= cap:
            f["index_jobs"] = tracker.jobs()
            raise _Stop("timeout", "index phase after %s s" % _n(int(cap)))
        if not tracker.seen_any() and time.time() >= grace_end:
            if f.get("defer_indexing") == "false":
                break
            raise _Stop("error", "no deferred reindex in the HFS log within %s s" % _n(int(args.index_start_grace_s)))
        time.sleep(min(2.0, args.poll_s))
    kind, final_ts = tracker.final()
    if kind is None:  # indexing ran inline with the ingest
        f["index_status"] = "inline"
        f["index_seconds"] = 0
        index_end = ingest_end
    else:
        f["index_status"] = kind
        index_end = max(final_ts, ingest_end)
        f["index_seconds"] = round(index_end - ingest_end, 1)
    f["index_jobs"] = tracker.jobs()
    f["index_errors"] = tracker.index_errors()
    f["total_seconds"] = round(index_end - t0, 1)
    if f["total_seconds"] > 0:
        f["resources_per_s"] = "%.1f" % (ok / f["total_seconds"])
    print("index %s: %s s (jobs=%s)" % (f["index_status"], f["index_seconds"], f["index_jobs"]), flush=True)

    f["phase"] = "verify"
    checks, searchable = verify(base, f)
    f.update(checks)
    f["searchable"] = searchable
    f["phase"] = "done"
    f["status"] = "complete"


def main(argv=None):
    args = parse_args(sys.argv[1:] if argv is None else argv)
    if args.convert_only:
        shutil.rmtree(os.path.join(args.workdir, "ndjson"), ignore_errors=True)
        stats = convert(open_source(args.source), os.path.join(args.workdir, "ndjson"),
                        args.bundles, args.max_lines_per_file)
        stats.pop("outputs", None)
        print(json.dumps(stats, sort_keys=True))
        return 0
    f = {"status": "error", "reason": "", "phase": "preflight",
         "backend": os.environ.get("HFS_STORAGE_BACKEND", ""), "source": args.source}
    f.update(env_record(os.environ))
    previous = signal.signal(signal.SIGTERM, _on_sigterm)
    try:
        run(args, f)
    except _Stop as s:
        f["status"], f["reason"] = s.status, s.reason
    except Terminated:
        f["status"], f["reason"] = "error", "terminated during %s" % f.get("phase")
    except Exception as e:
        f["status"], f["reason"] = "error", ("%s: %s" % (f.get("phase"), e))[:80]
    finally:
        signal.signal(signal.SIGTERM, previous)
        try:
            write_result(args.results_dir, f)
        except OSError as e:
            print("::warning::could not write %s: %s" % (RESULT_FILE, e), flush=True)
    print("bulk-import: status=%s%s phase=%s ok=%s/%s total=%ss rate=%s resources/s searchable=%s" % (
        f["status"], (" (%s)" % f["reason"]) if f["reason"] else "", f.get("phase"),
        f.get("resources_ok"), f.get("resources_submitted"), f.get("total_seconds"),
        f.get("resources_per_s"), f.get("searchable")), flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
