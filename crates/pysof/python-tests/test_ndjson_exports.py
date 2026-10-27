"""The local NDJSON streaming APIs documented in the README must be exported.

Regression tests for a packaging gap: ``process_ndjson_to_file`` and
``ChunkedProcessor`` existed in the Rust extension but were never re-exported
from the ``pysof`` package (only the ``_remote`` variant was).
"""

import json
from pathlib import Path
from typing import Any

import pysof


def get_view_definition() -> dict[str, Any]:
    return {
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


def write_patients_ndjson(path: Path, count: int) -> None:
    with path.open("w") as f:
        for i in range(count):
            patient = {
                "resourceType": "Patient",
                "id": f"p{i}",
                "name": [{"family": f"Family{i}"}],
            }
            f.write(json.dumps(patient) + "\n")


def test_process_ndjson_to_file_is_exported_and_works(tmp_path: Path) -> None:
    """pysof.process_ndjson_to_file processes an NDJSON file end to end."""
    input_path = tmp_path / "patients.ndjson"
    output_path = tmp_path / "out.csv"
    write_patients_ndjson(input_path, 3)

    stats = pysof.process_ndjson_to_file(
        get_view_definition(),
        str(input_path),
        str(output_path),
        "csv",
        chunk_size=2,
    )

    assert stats["total_lines_read"] == 3
    assert stats["resources_processed"] == 3
    assert output_path.exists()
    content = output_path.read_text()
    assert "Family0" in content
    assert "Family2" in content


def test_chunked_processor_is_exported_and_iterates(tmp_path: Path) -> None:
    """pysof.ChunkedProcessor streams chunks of transformed rows."""
    input_path = tmp_path / "patients.ndjson"
    write_patients_ndjson(input_path, 5)

    processor = pysof.ChunkedProcessor(
        get_view_definition(), str(input_path), chunk_size=2
    )
    assert processor.columns == ["id", "family"]

    chunks = list(processor)
    assert len(chunks) == 3  # 2 + 2 + 1
    all_rows = [row for chunk in chunks for row in chunk["rows"]]
    assert len(all_rows) == 5
    assert all_rows[0] == ["p0", "Family0"]
    assert chunks[-1]["is_last"] is True


def test_ndjson_apis_are_in_all() -> None:
    """The public surface advertises the local NDJSON APIs."""
    assert "process_ndjson_to_file" in pysof.__all__
    assert "ChunkedProcessor" in pysof.__all__
