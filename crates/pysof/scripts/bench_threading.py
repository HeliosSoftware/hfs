"""Multi-threaded throughput benchmark for pysof.

Measures aggregate throughput (transforms/sec) at 1/2/4/8 Python threads for:
  - dict input (converted from Python objects under the GIL)
  - JSON-string input (parsed entirely off the GIL, where supported)
  - process_ndjson_to_file (file-to-file path)

Context: each pysof call is already internally parallel — helios-sof fans the
transform out over rayon's global thread pool, so a single call can saturate
the machine and Python-thread scaling plateaus well below linear. Pin the pool
with RAYON_NUM_THREADS to isolate its contribution. This script guards the
allocator choice (mimalloc, ~2x over the system allocator) and the GIL-hold
behavior; run it on an otherwise idle machine — concurrent builds or other
load skew the numbers badly.

Usage:
    uv run python scripts/bench_threading.py [label]
"""

import json
import os
import sys
import tempfile
import threading
import time

import pysof

# Until the local NDJSON API is exported from the package (see pysof issue
# backlog), fall back to the raw extension module.
try:
    from pysof import _pysof

    process_ndjson_to_file = getattr(pysof, "process_ndjson_to_file", None) or (
        lambda view, inp, out, fmt, **kw: _pysof.py_process_ndjson_to_file(
            view, inp, out, fmt, **kw
        )
    )
except ImportError:  # pragma: no cover
    process_ndjson_to_file = pysof.process_ndjson_to_file

LABEL = sys.argv[1] if len(sys.argv) > 1 else "current"

VIEW = {
    "resourceType": "ViewDefinition",
    "resource": "Patient",
    "status": "active",
    "select": [
        {
            "column": [
                {"name": "id", "path": "id", "type": "id"},
                {"name": "gender", "path": "gender", "type": "code"},
                {"name": "birth_date", "path": "birthDate", "type": "date"},
                {"name": "family", "path": "name.first().family", "type": "string"},
                {"name": "given", "path": "name.first().given.first()", "type": "string"},
                {"name": "city", "path": "address.first().city", "type": "string"},
                {"name": "phone", "path": "telecom.first().value", "type": "string"},
            ]
        }
    ],
}


def make_patient(i):
    return {
        "resourceType": "Patient",
        "id": f"p{i}",
        "active": True,
        "gender": "male" if i % 2 == 0 else "female",
        "birthDate": "1980-01-01",
        "name": [
            {"family": f"Family{i}", "given": [f"Given{i}", "Middle"], "use": "official"}
        ],
        "identifier": [
            {"system": "urn:example:mrn", "value": f"mrn-{i:08d}"},
            {"system": "urn:example:ssn", "value": f"ssn-{i:08d}"},
        ],
        "telecom": [
            {"system": "phone", "value": f"555-{i:07d}", "use": "home"},
            {"system": "email", "value": f"p{i}@example.com"},
        ],
        "address": [
            {
                "use": "home",
                "line": [f"{i} Main Street", "Apt 1"],
                "city": "Springfield",
                "state": "IL",
                "postalCode": "62701",
            }
        ],
    }


def make_bundle(n):
    return {
        "resourceType": "Bundle",
        "type": "collection",
        "entry": [{"resource": make_patient(i)} for i in range(n)],
    }


THREAD_COUNTS = [1, 2, 4, 8]
TARGET_SECONDS = 3.0


def run_threads(n_threads, iterations, fn):
    barrier = threading.Barrier(n_threads + 1)
    errors = []

    def worker():
        try:
            barrier.wait()
            for _ in range(iterations):
                fn()
        except Exception as e:
            errors.append(e)

    threads = [threading.Thread(target=worker) for _ in range(n_threads)]
    for t in threads:
        t.start()
    barrier.wait()
    t0 = time.perf_counter()
    for t in threads:
        t.join()
    elapsed = time.perf_counter() - t0
    if errors:
        raise errors[0]
    return (n_threads * iterations) / elapsed


def bench_scenario(name, fn):
    # Warmup, then calibrate iterations for ~TARGET_SECONDS per configuration
    fn()
    t0 = time.perf_counter()
    fn()
    per_call = time.perf_counter() - t0
    iterations = max(3, int(TARGET_SECONDS / max(per_call, 1e-6)))

    results = {}
    base = None
    for n in THREAD_COUNTS:
        ops = run_threads(n, iterations, fn)
        if base is None:
            base = ops
        results[n] = (ops, ops / base)
    row = " | ".join(
        f"{results[n][0]:8.1f} ops/s x{results[n][1]:4.2f}" for n in THREAD_COUNTS
    )
    print(f"{name:34s} | {row}")
    return results


def main():
    print(
        f"pysof {pysof.get_version()} | python {sys.version.split()[0]} | "
        f"cpus={os.cpu_count()} | label={LABEL} | "
        f"rayon={os.environ.get('RAYON_NUM_THREADS', 'default')}"
    )
    header = " | ".join(f"{n} thread{'s' if n > 1 else '':7s}" for n in THREAD_COUNTS)
    print(f"{'scenario':34s} | {header}")

    small = make_bundle(50)
    large = make_bundle(2000)
    large_str = json.dumps(large)
    view_str = json.dumps(VIEW)

    bench_scenario(
        "dict bundle, 50 patients",
        lambda: pysof.run_view_definition(VIEW, small, "csv"),
    )
    bench_scenario(
        "dict bundle, 2000 patients",
        lambda: pysof.run_view_definition(VIEW, large, "csv"),
    )

    try:
        pysof.run_view_definition(view_str, large_str, "csv")
        bench_scenario(
            "str bundle, 2000 patients",
            lambda: pysof.run_view_definition(view_str, large_str, "csv"),
        )
    except Exception as e:
        print(f"{'str bundle, 2000 patients':34s} | unsupported ({type(e).__name__})")

    tmpdir = tempfile.mkdtemp(prefix="pysof_bench_")
    ndjson_path = os.path.join(tmpdir, "patients.ndjson")
    with open(ndjson_path, "w") as f:
        for i in range(5000):
            f.write(json.dumps(make_patient(i)) + "\n")

    counter = threading.local()

    def file_job():
        # unique output per thread+iteration to avoid write collisions
        ident = threading.get_ident()
        n = getattr(counter, "n", 0)
        counter.n = n + 1
        out = os.path.join(tmpdir, f"out_{ident}_{n}.csv")
        process_ndjson_to_file(VIEW, ndjson_path, out, "csv", chunk_size=1000)

    bench_scenario("ndjson file->file, 5000 patients", file_job)


if __name__ == "__main__":
    main()
