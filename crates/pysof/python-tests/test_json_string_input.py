"""ViewDefinitions and Bundles may be passed as JSON strings or bytes.

Pre-serialized JSON skips the Python-dict conversion entirely: the input is
handed to Rust as text and parsed off the GIL, which is both faster per call
and keeps other Python threads responsive during large-bundle parsing.
"""

import json
from pathlib import Path
from typing import Any

import pytest

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
            ]
        }
    ],
}

BUNDLE: dict[str, Any] = {
    "resourceType": "Bundle",
    "type": "collection",
    "entry": [
        {
            "resource": {
                "resourceType": "Patient",
                "id": f"p{i}",
                "name": [{"family": f"Family{i}"}],
            }
        }
        for i in range(3)
    ],
}


def test_run_view_definition_accepts_json_strings() -> None:
    """String inputs produce byte-identical output to dict inputs."""
    from_dicts = pysof.run_view_definition(VIEW, BUNDLE, "csv")
    from_strs = pysof.run_view_definition(json.dumps(VIEW), json.dumps(BUNDLE), "csv")
    assert from_strs == from_dicts


def test_run_view_definition_accepts_json_bytes() -> None:
    """Bytes inputs produce byte-identical output to dict inputs."""
    from_dicts = pysof.run_view_definition(VIEW, BUNDLE, "csv")
    from_bytes = pysof.run_view_definition(
        json.dumps(VIEW).encode(), json.dumps(BUNDLE).encode(), "csv"
    )
    assert from_bytes == from_dicts


def test_run_view_definition_accepts_mixed_inputs() -> None:
    """A dict view may be combined with a string bundle, and vice versa."""
    from_dicts = pysof.run_view_definition(VIEW, BUNDLE, "csv")
    assert pysof.run_view_definition(VIEW, json.dumps(BUNDLE), "csv") == from_dicts
    assert pysof.run_view_definition(json.dumps(VIEW), BUNDLE, "csv") == from_dicts


def test_run_view_definition_rejects_invalid_json_string() -> None:
    with pytest.raises(pysof.SerializationError):
        pysof.run_view_definition("{not json", json.dumps(BUNDLE), "csv")


def test_run_view_definition_rejects_wrong_shaped_json_string() -> None:
    """Valid JSON that is not a ViewDefinition still fails as before."""
    with pytest.raises(pysof.SerializationError):
        pysof.run_view_definition('"just a string"', json.dumps(BUNDLE), "csv")


def test_run_view_definition_with_options_accepts_json_strings() -> None:
    from_dicts = pysof.run_view_definition_with_options(VIEW, BUNDLE, "csv", limit=2)
    from_strs = pysof.run_view_definition_with_options(
        json.dumps(VIEW), json.dumps(BUNDLE), "csv", limit=2
    )
    assert from_strs == from_dicts


def test_run_view_definition_remote_accepts_json_strings() -> None:
    cfg = pysof.RemoteResolveConfig([])  # inactive: in-bundle resolution only
    from_dicts = pysof.run_view_definition_remote(VIEW, BUNDLE, "csv", cfg)
    from_strs = pysof.run_view_definition_remote(
        json.dumps(VIEW), json.dumps(BUNDLE), "csv", cfg
    )
    assert from_strs == from_dicts


def test_validate_functions_accept_json_strings() -> None:
    assert pysof.validate_view_definition(json.dumps(VIEW)) is True
    assert pysof.validate_bundle(json.dumps(BUNDLE)) is True


def test_ndjson_paths_accept_json_string_view(tmp_path: Path) -> None:
    input_path = tmp_path / "patients.ndjson"
    with input_path.open("w") as f:
        for entry in BUNDLE["entry"]:
            f.write(json.dumps(entry["resource"]) + "\n")

    output_path = tmp_path / "out.csv"
    stats = pysof.process_ndjson_to_file(
        json.dumps(VIEW), str(input_path), str(output_path), "csv"
    )
    assert stats["resources_processed"] == 3

    processor = pysof.ChunkedProcessor(json.dumps(VIEW), str(input_path))
    rows = [row for chunk in processor for row in chunk["rows"]]
    assert len(rows) == 3
