"""Tests for the remote resolve() Python bindings.

These exercise the RemoteResolveConfig class and the *_remote entry points
without requiring network access: an *inactive* config (no allowlist) drives the
same async pipeline as the active one but performs no fetches, so it still does
bundle-level (in-scope) resolution. End-to-end remote fetching against a server is
covered by the Rust integration tests (crates/sof/tests/resolve_remote_tests.rs).
"""

import json
from typing import Any, Dict

import pysof


def encounter_with_subject_view() -> Dict[str, Any]:
    return {
        "resourceType": "ViewDefinition",
        "status": "active",
        "resource": "Encounter",
        "select": [
            {"column": [{"name": "encounter_id", "path": "id"}]},
            {
                "forEach": "subject.resolve()",
                "column": [
                    {"name": "patient_id", "path": "id"},
                    {"name": "patient_family", "path": "name.family"},
                ],
            },
        ],
    }


def bundle_with_sibling_patient() -> Dict[str, Any]:
    return {
        "resourceType": "Bundle",
        "type": "collection",
        "entry": [
            {
                "resource": {
                    "resourceType": "Patient",
                    "id": "pat-1",
                    "name": [{"family": "Sibling"}],
                }
            },
            {
                "resource": {
                    "resourceType": "Encounter",
                    "id": "enc-1",
                    "status": "finished",
                    "subject": {"reference": "Patient/pat-1"},
                }
            },
        ],
    }


def test_remote_symbols_exported() -> None:
    for name in [
        "RemoteResolveConfig",
        "run_view_definition_remote",
        "process_ndjson_to_file_remote",
    ]:
        assert name in pysof.__all__
        assert hasattr(pysof, name)


def test_remote_resolve_config_construction() -> None:
    cfg = pysof.RemoteResolveConfig(
        ["https://fhir.example.org/r4"],
        max_fetches=10,
        max_depth=2,
        allow_private_addresses=True,
        bearer_tokens={"fhir.example.org": "tok"},
    )
    assert cfg.is_active() is True  # enabled (default) + non-empty allowlist

    # Disabled or empty allowlist => inactive.
    assert pysof.RemoteResolveConfig([]).is_active() is False
    assert (
        pysof.RemoteResolveConfig(
            ["https://fhir.example.org/r4"], enabled=False
        ).is_active()
        is False
    )

    # from_env() is constructible and inactive by default (no env set).
    assert isinstance(pysof.RemoteResolveConfig.from_env(), pysof.RemoteResolveConfig)


def test_run_view_definition_remote_inactive_does_in_bundle_resolution() -> None:
    # With an inactive remote config (no allowlist), no fetch happens, but the
    # bundle-level resolution pool still resolves Encounter.subject to the sibling
    # Patient in the same bundle.
    cfg = pysof.RemoteResolveConfig([])  # inactive
    result = pysof.run_view_definition_remote(
        encounter_with_subject_view(),
        bundle_with_sibling_patient(),
        "json",
        cfg,
    )
    rows = json.loads(result)
    assert len(rows) == 1
    assert rows[0]["encounter_id"] == "enc-1"
    assert rows[0]["patient_id"] == "pat-1"
    assert rows[0]["patient_family"] == "Sibling"


def test_process_ndjson_to_file_remote_inactive(tmp_path: Any) -> None:
    # Two Encounters referencing patients that are not in the stream; with an
    # inactive config nothing is fetched, so the resolved columns are null, but
    # the call succeeds and returns stats.
    input_path = tmp_path / "in.ndjson"
    output_path = tmp_path / "out.json"
    lines = [
        json.dumps(
            {
                "resourceType": "Encounter",
                "id": f"enc-{i}",
                "status": "finished",
                "subject": {"reference": f"Patient/p{i}"},
            }
        )
        for i in range(2)
    ]
    input_path.write_text("\n".join(lines))

    cfg = pysof.RemoteResolveConfig([])  # inactive
    stats = pysof.process_ndjson_to_file_remote(
        encounter_with_subject_view(),
        str(input_path),
        str(output_path),
        "json",
        cfg,
        chunk_size=1,
    )
    assert stats["resources_processed"] == 2
    rows = json.loads(output_path.read_text())
    assert len(rows) == 2
    for row in rows:
        assert row["patient_family"] is None