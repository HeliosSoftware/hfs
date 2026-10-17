#!/usr/bin/env python3
#
# Interactive write-latency probe for the Elasticsearch refresh_interval study (#1749).
#
# Measures how long an interactive write waits on a running HFS server, for one server
# configuration (the caller sets HFS_ELASTICSEARCH_REFRESH_INTERVAL / _WRITE_REFRESH and
# restarts HFS; this script never touches the server's configuration):
#   single       N x POST /Basic with a minimal resource (default N=10): p50 / p95 / max
#   batch        one batch Bundle of 8 entries
#   transaction  one transaction Bundle of 662 entries
#   new_type     (--new-type, "C" in the issue) the first write of a resource type never
#                written before, then GET {es}/{prefix}_{tenant}_{type}/_settings to show
#                which refresh_interval the freshly created index carries
#
# Run it once without an import running and once while one runs, same label scheme:
#   write_latency.py --base-url http://127.0.0.1:18080 --label wait_for-30s \
#       --es-url http://127.0.0.1:19200 --new-type RiskAssessment --out lat.json
#
# Payloads: --batch-file / --transaction-file take real Bundles (the 8-entry practitioner
# batch and the 662-entry transaction of MANUAL_TESTING_MATRIX.md T2); without them a
# synthetic Bundle of Basic resources with the same entry count is sent and the output
# says so ("synthetic": true). Entry content differs, so compare runs of one payload only.
#
# Output: one JSON document on stdout (and --out). Exit 0 unless the arguments are bad.
# Python 3.8-compatible, standard library only.
import argparse
import json
import os
import sys
import time
import urllib.parse

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from bulk_import import http_call  # noqa: E402

BATCH_ENTRIES = 8
TRANSACTION_ENTRIES = 662

# Smallest valid body per type, for the "first write of a new type" probe.
NEW_TYPE_BODIES = {
    "Basic": {"code": {"text": "refresh probe"}},
    "Group": {"type": "person", "actual": True},
    "Library": {"status": "draft", "type": {"text": "logic-library"}},
    "RiskAssessment": {"status": "final", "subject": {"display": "refresh probe"}},
}


def percentile(sorted_values, pct):
    """Linear-interpolated percentile of an ascending list; None when empty."""
    if not sorted_values:
        return None
    if len(sorted_values) == 1:
        return sorted_values[0]
    rank = (len(sorted_values) - 1) * pct / 100.0
    lo = int(rank)
    hi = min(lo + 1, len(sorted_values) - 1)
    return sorted_values[lo] + (sorted_values[hi] - sorted_values[lo]) * (rank - lo)


def summarize(seconds, failures=0):
    vals = sorted(seconds)
    return {
        "n": len(vals), "failures": failures,
        "p50": _r(percentile(vals, 50)), "p95": _r(percentile(vals, 95)),
        "max": _r(vals[-1] if vals else None),
    }


def _r(v):
    return None if v is None else round(v, 3)


def basic_resource(tag):
    return {"resourceType": "Basic", "code": {"text": "refresh probe %s" % tag}}


def synthetic_bundle(kind, entries):
    return {"resourceType": "Bundle", "type": kind, "entry": [
        {"resource": basic_resource("%s-%d" % (kind, i)),
         "request": {"method": "POST", "url": "Basic"}} for i in range(entries)]}


def timed_post(url, body, timeout):
    """(seconds, HTTP status); a connection failure counts as status 0."""
    t0 = time.time()
    try:
        status, _h, _b = http_call("POST", url, body, timeout=timeout)
    except OSError:
        status = 0
    return time.time() - t0, status


def probe_single(base, samples, timeout):
    secs, bad = [], 0
    for i in range(samples):
        took, status = timed_post("%s/Basic" % base, basic_resource("single-%d" % i), timeout)
        secs.append(took)
        bad += 0 if 200 <= status < 300 else 1
    return summarize(secs, bad)


def probe_bundle(base, bundle, timeout):
    took, status = timed_post(base, bundle, timeout)
    return {"entries": len(bundle.get("entry", [])), "seconds": _r(took), "status": status}


def index_name(prefix, tenant, resource_type):
    """{prefix}_{tenant}_{type}; the tenant segment is the plain id for [a-z0-9._-]."""
    return "%s_%s_%s" % (prefix, tenant, resource_type.lower())


def refresh_interval_of(es_url, index, timeout=30):
    """The index's effective refresh_interval, or None when it cannot be read."""
    url = "%s/%s/_settings/index.refresh_interval?flat_settings=true&include_defaults=true" % (
        es_url.rstrip("/"), urllib.parse.quote(index, safe=""))
    try:
        status, _h, raw = http_call("GET", url, timeout=timeout)
        doc = json.loads(raw.decode("utf-8"))
    except (OSError, ValueError):
        return None
    if status != 200 or not isinstance(doc, dict):
        return None
    for entry in doc.values():
        for section in ("settings", "defaults"):
            v = (entry.get(section) or {}).get("index.refresh_interval")
            if v is not None:
                return v
    return None


def probe_new_type(base, es_url, prefix, tenant, resource_type, timeout):
    body = {"resourceType": resource_type}
    body.update(NEW_TYPE_BODIES.get(resource_type, {}))
    took, status = timed_post("%s/%s" % (base, resource_type), body, timeout)
    index = index_name(prefix, tenant, resource_type)
    out = {"type": resource_type, "seconds": _r(took), "status": status, "index": index}
    if es_url:
        out["index_refresh_interval"] = refresh_interval_of(es_url, index)
    return out


def load_bundle(path, kind, entries):
    if path:
        with open(path, encoding="utf-8") as fh:
            return json.load(fh), False
    return synthetic_bundle(kind, entries), True


def parse_args(argv):
    p = argparse.ArgumentParser(description="Interactive write-latency probe (#1749)")
    p.add_argument("--base-url", required=True, help="HFS base URL, e.g. http://127.0.0.1:18080")
    p.add_argument("--label", default="", help="free text copied into the output")
    p.add_argument("--samples", type=int, default=10)
    p.add_argument("--batch-file")
    p.add_argument("--transaction-file")
    p.add_argument("--timeout-s", type=float, default=300,
                   help="per request; keep it above the refresh interval under test")
    p.add_argument("--es-url", help="Elasticsearch URL, to read the new index's settings")
    p.add_argument("--index-prefix", default="hfs")
    p.add_argument("--tenant", default="default")
    p.add_argument("--new-type", help="resource type never written before (mode C)")
    p.add_argument("--skip", default="", help="comma list of single,batch,transaction")
    p.add_argument("--out")
    return p.parse_args(argv)


def run(args):
    base = args.base_url.rstrip("/")
    skip = set(x.strip() for x in args.skip.split(",") if x.strip())
    result = {"label": args.label, "base_url": base}
    if "single" not in skip:
        result["single"] = probe_single(base, args.samples, args.timeout_s)
    if "batch" not in skip:
        bundle, synth = load_bundle(args.batch_file, "batch", BATCH_ENTRIES)
        result["batch"] = probe_bundle(base, bundle, args.timeout_s)
        result["batch"]["synthetic"] = synth
    if "transaction" not in skip:
        bundle, synth = load_bundle(args.transaction_file, "transaction", TRANSACTION_ENTRIES)
        result["transaction"] = probe_bundle(base, bundle, args.timeout_s)
        result["transaction"]["synthetic"] = synth
    if args.new_type:
        result["new_type"] = probe_new_type(base, args.es_url, args.index_prefix,
                                            args.tenant, args.new_type, args.timeout_s)
    return result


def main(argv=None):
    args = parse_args(sys.argv[1:] if argv is None else argv)
    text = json.dumps(run(args), indent=2, sort_keys=True)
    print(text, flush=True)
    if args.out:
        with open(args.out, "w", encoding="utf-8") as fh:
            fh.write(text + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
