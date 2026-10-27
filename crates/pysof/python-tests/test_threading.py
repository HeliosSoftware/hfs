"""Concurrent use of pysof from multiple Python threads.

Regression guards for thread safety: every entry point releases the GIL during
parsing and processing, so concurrent calls must neither corrupt results nor
interfere with each other. (Note: concurrency is not a throughput lever — each
call is already internally parallel via rayon's shared pool.)
"""

import json
import threading
from typing import Any

import pysof

VIEW: dict[str, Any] = {
    "resourceType": "ViewDefinition",
    "status": "active",
    "resource": "Patient",
    "select": [
        {
            "column": [
                {"name": "id", "path": "id"},
                {"name": "family", "path": "name.first().family"},
                {"name": "gender", "path": "gender"},
            ]
        }
    ],
}


def make_bundle(n: int) -> dict[str, Any]:
    return {
        "resourceType": "Bundle",
        "type": "collection",
        "entry": [
            {
                "resource": {
                    "resourceType": "Patient",
                    "id": f"p{i}",
                    "name": [{"family": f"Family{i}"}],
                    "gender": "male" if i % 2 == 0 else "female",
                }
            }
            for i in range(n)
        ],
    }


def run_concurrently(worker, n_threads: int) -> list[Any]:
    """Run `worker(index)` on n_threads barrier-synchronized threads."""
    barrier = threading.Barrier(n_threads)
    results: list[Any] = [None] * n_threads
    errors: list[Exception] = []

    def wrapped(i: int) -> None:
        try:
            barrier.wait()
            results[i] = worker(i)
        except Exception as e:  # noqa: BLE001 - re-raised below
            errors.append(e)

    threads = [threading.Thread(target=wrapped, args=(i,)) for i in range(n_threads)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    if errors:
        raise errors[0]
    return results


def test_concurrent_transforms_produce_identical_results() -> None:
    """Eight threads transforming the same bundle all get the same bytes."""
    bundle = make_bundle(200)
    expected = pysof.run_view_definition(VIEW, bundle, "csv")

    results = run_concurrently(
        lambda i: pysof.run_view_definition(VIEW, bundle, "csv"), 8
    )
    assert all(r == expected for r in results)


def test_concurrent_transforms_with_distinct_inputs() -> None:
    """Threads with different bundles each get their own correct result."""
    bundles = [make_bundle(10 + i) for i in range(6)]

    results = run_concurrently(
        lambda i: pysof.run_view_definition(VIEW, bundles[i], "json"), 6
    )
    for i, raw in enumerate(results):
        rows = json.loads(raw)
        assert len(rows) == 10 + i
        assert rows[0]["id"] == "p0"


def test_concurrent_string_and_dict_inputs_agree() -> None:
    """Mixed dict and JSON-string inputs run concurrently without interference."""
    bundle = make_bundle(100)
    bundle_str = json.dumps(bundle)
    expected = pysof.run_view_definition(VIEW, bundle, "csv")

    results = run_concurrently(
        lambda i: pysof.run_view_definition(
            VIEW, bundle if i % 2 == 0 else bundle_str, "csv"
        ),
        8,
    )
    assert all(r == expected for r in results)
