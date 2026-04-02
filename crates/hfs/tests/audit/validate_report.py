#!/usr/bin/env python3
import argparse
import json
import sys
from pathlib import Path
from typing import Any, Callable


AUDIT_EVENT_TYPE_SYSTEM = "http://terminology.hl7.org/CodeSystem/audit-event-type"
RESTFUL_INTERACTION_SYSTEM = "http://hl7.org/fhir/restful-interaction"

VALID_AUDIT_EVENT_TYPE_CODES = {"rest", "hl7-v2", "hl7-v3", "document", "object"}
VALID_RESTFUL_INTERACTION_CODES = {
    "read",
    "vread",
    "update",
    "patch",
    "delete",
    "history",
    "history-instance",
    "history-type",
    "history-system",
    "create",
    "search",
    "search-type",
    "search-system",
    "search-compartment",
    "capabilities",
    "transaction",
    "batch",
    "operation",
}


def load_ndjson(path: Path) -> list[dict[str, Any]]:
    events: list[dict[str, Any]] = []
    if not path.exists():
        return events

    for idx, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
        line = raw.strip()
        if not line:
            continue
        try:
            item = json.loads(line)
        except json.JSONDecodeError as exc:
            raise RuntimeError(f"Invalid NDJSON at {path}:{idx}: {exc}") from exc
        item["__line"] = idx
        events.append(item)
    return events


def read_context(path: Path) -> dict[str, Any]:
    if not path.exists():
        return {}
    return json.loads(path.read_text(encoding="utf-8"))


def scalar(value: Any) -> str:
    if value is None:
        return ""
    if isinstance(value, str):
        return value
    if isinstance(value, (int, float, bool)):
        return str(value)
    if isinstance(value, dict):
        if "value" in value:
            return scalar(value.get("value"))
        for sub in value.values():
            text = scalar(sub)
            if text:
                return text
    return ""


def type_code(event: dict[str, Any]) -> str:
    return scalar((event.get("type") or {}).get("code"))


def type_system(event: dict[str, Any]) -> str:
    return scalar((event.get("type") or {}).get("system"))


def action(event: dict[str, Any]) -> str:
    return scalar(event.get("action"))


def outcome(event: dict[str, Any]) -> str:
    return scalar(event.get("outcome"))


def outcome_desc(event: dict[str, Any]) -> str:
    return scalar(event.get("outcomeDesc"))


def subtype_codes(event: dict[str, Any]) -> list[str]:
    codes: list[str] = []
    for sub in event.get("subtype") or []:
        code = scalar(sub.get("code"))
        if code:
            codes.append(code)
    return codes


def extract_reference(value: Any) -> str:
    if isinstance(value, str):
        return value
    if isinstance(value, dict):
        return scalar(value.get("reference"))
    return ""


def entity_refs(event: dict[str, Any]) -> list[str]:
    refs: list[str] = []
    for entity in event.get("entity") or []:
        ref = extract_reference((entity.get("what") or {}).get("reference"))
        if ref:
            refs.append(ref)
    return refs


def has_patient_ref(event: dict[str, Any], patient_ref: str | None = None) -> bool:
    for ref in entity_refs(event):
        if not ref.startswith("Patient/"):
            continue
        if patient_ref is None or ref == patient_ref:
            return True
    return False


def detail_map(event: dict[str, Any]) -> dict[str, list[str]]:
    details: dict[str, list[str]] = {}
    for entity in event.get("entity") or []:
        for detail in entity.get("detail") or []:
            key = scalar(detail.get("type"))
            if not key:
                continue

            value = ""
            for typed_key in (
                "valueString",
                "valueCode",
                "valueUri",
                "valueBoolean",
                "valueInteger",
                "valueDecimal",
                "valueDateTime",
            ):
                if typed_key in detail:
                    value = scalar(detail.get(typed_key))
                    break
            if not value and "value" in detail:
                value = scalar(detail.get("value"))

            details.setdefault(key, []).append(value)
    return details


def find_example(events: list[dict[str, Any]], predicate: Callable[[dict[str, Any]], bool]) -> dict[str, Any] | None:
    for event in events:
        if predicate(event):
            return event
    return None


def is_lifecycle_phase(event: dict[str, Any], phase: str) -> bool:
    if type_system(event) != AUDIT_EVENT_TYPE_SYSTEM:
        return False
    if type_code(event) != "object":
        return False
    details = detail_map(event)
    if phase not in details.get("phase", []):
        return False
    return f"lifecycle-{phase}" in details.get("audit-operation", [])


def is_auth_missing_token(event: dict[str, Any]) -> bool:
    return outcome(event) == "8" and "Missing Authorization header" in outcome_desc(event)


def is_auth_invalid_token(event: dict[str, Any]) -> bool:
    return outcome(event) == "8" and "Invalid token format" in outcome_desc(event)


def is_auth_success(event: dict[str, Any]) -> bool:
    if type_code(event) != "rest":
        return False
    if outcome(event) != "0":
        return False
    if action(event) != "E":
        return False
    if "operation" not in subtype_codes(event):
        return False
    desc = outcome_desc(event)
    if desc:
        return False
    return True


def is_authz_grant(event: dict[str, Any]) -> bool:
    return outcome(event) == "0" and outcome_desc(event).startswith("Granted:")


def is_authz_denial(event: dict[str, Any]) -> bool:
    return outcome(event) == "8" and outcome_desc(event).startswith("Forbidden:")


def has_subtype(event: dict[str, Any], code: str) -> bool:
    return code in subtype_codes(event)


def has_ref(event: dict[str, Any], ref: str) -> bool:
    return ref in entity_refs(event)


def validate_terminology(events: list[dict[str, Any]]) -> tuple[str, dict[str, Any] | None]:
    for event in events:
        t_system = type_system(event)
        t_code = type_code(event)
        if t_system == AUDIT_EVENT_TYPE_SYSTEM and t_code not in VALID_AUDIT_EVENT_TYPE_CODES:
            return (
                f"Invalid audit-event-type code '{t_code}' for system '{AUDIT_EVENT_TYPE_SYSTEM}'",
                event,
            )

        for subtype in event.get("subtype") or []:
            s_system = scalar(subtype.get("system"))
            s_code = scalar(subtype.get("code"))
            if s_system == RESTFUL_INTERACTION_SYSTEM and s_code not in VALID_RESTFUL_INTERACTION_CODES:
                return (
                    f"Invalid restful-interaction code '{s_code}' for system '{RESTFUL_INTERACTION_SYSTEM}'",
                    event,
                )

    return (
        "All audited events use valid HL7 audit-event-type and restful-interaction codes",
        None,
    )


def read_ranges(path: Path) -> list[dict[str, Any]]:
    records: list[dict[str, Any]] = []
    lines = path.read_text(encoding="utf-8").splitlines()
    if not lines:
        return records
    for row in lines[1:]:
        if not row.strip():
            continue
        name, method, req_path, expected, actual, start, end = row.split("\t")
        records.append(
            {
                "name": name,
                "method": method,
                "path": req_path,
                "expected": expected,
                "actual": actual,
                "start_line": int(start),
                "end_line": int(end),
            }
        )
    return records


def call_events(output_dir: Path, call_name: str) -> list[dict[str, Any]]:
    return load_ndjson(output_dir / "events" / f"{call_name}.ndjson")


def write_example(output_dir: Path, key: str, event: dict[str, Any] | None) -> str | None:
    if event is None:
        return None
    examples_dir = output_dir / "examples"
    examples_dir.mkdir(parents=True, exist_ok=True)
    path = examples_dir / f"{key}.json"
    clean = {k: v for k, v in event.items() if k != "__line"}
    path.write_text(json.dumps(clean, indent=2, sort_keys=True), encoding="utf-8")
    return str(path.relative_to(output_dir))


def check(label: str, ok: bool, message: str, example: dict[str, Any] | None) -> dict[str, Any]:
    return {
        "key": label,
        "status": "pass" if ok else "fail",
        "message": message,
        "example": example,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description="Validate HFS audit NDJSON coverage and generate a summary report")
    parser.add_argument("--audit-file", required=True)
    parser.add_argument("--ranges-file", required=True)
    parser.add_argument("--context-file", required=True)
    parser.add_argument("--output-dir", required=True)
    parser.add_argument("--strict-correlation", action="store_true")
    args = parser.parse_args()

    audit_file = Path(args.audit_file)
    ranges_file = Path(args.ranges_file)
    context_file = Path(args.context_file)
    output_dir = Path(args.output_dir)

    events = load_ndjson(audit_file)
    context = read_context(context_file)
    ranges = read_ranges(ranges_file)

    patient_id = context.get("patient_id", "")
    observation_id = context.get("observation_id", "")
    patient_ref = f"Patient/{patient_id}" if patient_id else None
    observation_ref = f"Observation/{observation_id}" if observation_id else None

    checks: list[dict[str, Any]] = []

    # Lifecycle
    startup_example = find_example(events, lambda e: is_lifecycle_phase(e, "startup"))
    shutdown_example = find_example(events, lambda e: is_lifecycle_phase(e, "shutdown"))
    checks.append(check("lifecycle_startup", startup_example is not None, "Startup lifecycle audit event", startup_example))
    checks.append(check("lifecycle_shutdown", shutdown_example is not None, "Shutdown lifecycle audit event", shutdown_example))

    # Auth global categories
    auth_missing_example = find_example(events, is_auth_missing_token)
    auth_invalid_example = find_example(events, is_auth_invalid_token)
    auth_success_example = find_example(events, is_auth_success)
    authz_grant_example = find_example(events, is_authz_grant)
    authz_denial_example = find_example(events, is_authz_denial)

    checks.append(check("auth_missing_token", auth_missing_example is not None, "Authentication failure: missing token", auth_missing_example))
    checks.append(check("auth_invalid_token", auth_invalid_example is not None, "Authentication failure: invalid token", auth_invalid_example))
    checks.append(check("auth_success", auth_success_example is not None, "Authentication success", auth_success_example))
    checks.append(check("authz_grant", authz_grant_example is not None, "Authorization grant", authz_grant_example))
    checks.append(check("authz_denial", authz_denial_example is not None, "Authorization denial", authz_denial_example))
    terminology_message, terminology_example = validate_terminology(events)
    checks.append(
        check(
            "terminology_valid",
            terminology_example is None,
            terminology_message,
            terminology_example,
        )
    )

    # Per-interaction checks from captured windows
    interaction_specs: list[tuple[str, str, Callable[[list[dict[str, Any]]], dict[str, Any] | None]]] = [
        (
            "create_patient",
            "POST /Patient -> Create",
            lambda evs: find_example(evs, lambda e: has_subtype(e, "create") and action(e) == "C" and (patient_ref is None or has_ref(e, patient_ref))),
        ),
        (
            "read_patient",
            "GET /Patient/{id} -> Read",
            lambda evs: find_example(evs, lambda e: has_subtype(e, "read") and action(e) == "R" and (patient_ref is None or has_ref(e, patient_ref))),
        ),
        (
            "head_patient",
            "HEAD /Patient/{id} -> Read",
            lambda evs: find_example(evs, lambda e: has_subtype(e, "read") and action(e) == "R" and (patient_ref is None or has_ref(e, patient_ref))),
        ),
        (
            "search_patient_get",
            "GET /Patient -> Query",
            lambda evs: find_example(evs, lambda e: has_subtype(e, "search") and action(e) == "E"),
        ),
        (
            "search_patient_post",
            "POST /Patient/_search -> Query",
            lambda evs: find_example(evs, lambda e: has_subtype(e, "search") and action(e) == "E"),
        ),
        (
            "history_type",
            "GET /Patient/_history -> Query",
            lambda evs: find_example(evs, lambda e: has_subtype(e, "search") and action(e) == "E"),
        ),
        (
            "history_system",
            "GET /_history -> Query",
            lambda evs: find_example(evs, lambda e: has_subtype(e, "search") and action(e) == "E"),
        ),
        (
            "history_instance",
            "GET /Patient/{id}/_history -> Query",
            lambda evs: find_example(evs, lambda e: has_subtype(e, "search") and action(e) == "E" and (patient_ref is None or has_ref(e, patient_ref))),
        ),
        (
            "create_observation",
            "POST /Observation -> Create",
            lambda evs: find_example(
                evs,
                lambda e: has_subtype(e, "create")
                and action(e) == "C"
                and (observation_ref is None or has_ref(e, observation_ref))
                and (patient_ref is None or has_patient_ref(e, patient_ref)),
            ),
        ),
        (
            "search_subject_query",
            "GET /Observation?subject=... -> Query with patient search param",
            lambda evs: find_example(evs, lambda e: has_subtype(e, "search") and has_patient_ref(e, patient_ref)),
        ),
        (
            "search_patient_query",
            "GET /Observation?patient=... -> Query with patient search param",
            lambda evs: find_example(evs, lambda e: has_subtype(e, "search") and has_patient_ref(e, patient_ref)),
        ),
        (
            "search_unresolved_query",
            "GET /Observation?code=... -> Query without patient resolution",
            lambda evs: find_example(evs, lambda e: has_subtype(e, "search") and not has_patient_ref(e, None)),
        ),
        (
            "update_patient_put",
            "PUT /Patient/{id} -> Update",
            lambda evs: find_example(evs, lambda e: has_subtype(e, "update") and action(e) == "U" and (patient_ref is None or has_ref(e, patient_ref))),
        ),
        (
            "patch_patient",
            "PATCH /Patient/{id} -> Update",
            lambda evs: find_example(evs, lambda e: has_subtype(e, "update") and action(e) == "U" and (patient_ref is None or has_ref(e, patient_ref))),
        ),
        (
            "readonly_denied_create",
            "Authorization denial on write attempt",
            lambda evs: find_example(evs, is_authz_denial),
        ),
        (
            "options_execute",
            "Other method (OPTIONS) -> operation",
            lambda evs: find_example(evs, lambda e: has_subtype(e, "operation") and type_code(e) == "rest" and outcome(e) == "8"),
        ),
        (
            "delete_observation",
            "DELETE /Observation/{id} -> Delete",
            lambda evs: find_example(evs, lambda e: has_subtype(e, "delete") and action(e) == "D" and (observation_ref is None or has_ref(e, observation_ref))),
        ),
        (
            "delete_patient",
            "DELETE /Patient/{id} -> Delete",
            lambda evs: find_example(evs, lambda e: has_subtype(e, "delete") and action(e) == "D" and (patient_ref is None or has_ref(e, patient_ref))),
        ),
    ]

    for call_name, label, matcher in interaction_specs:
        evs = call_events(output_dir, call_name)
        example = matcher(evs)
        checks.append(check(f"interaction_{call_name}", example is not None, label, example))

    # Batch/transaction per-entry and correlation checks
    for call_name, bundle_type in (("batch_bundle", "batch"), ("transaction_bundle", "transaction")):
        evs = call_events(output_dir, call_name)
        entry_events = [
            e
            for e in evs
            if type_code(e) == "rest" and any(code in {"read", "create", "update", "delete"} for code in subtype_codes(e))
        ]
        has_per_entry = len(entry_events) >= 2
        checks.append(
            check(
                f"interaction_{call_name}_per_entry",
                has_per_entry,
                f"{bundle_type} bundle emits per-entry audit events",
                entry_events[0] if entry_events else None,
            )
        )

        if args.strict_correlation:
            correlation_ok = True
            failing_example: dict[str, Any] | None = None
            for event in entry_events:
                dmap = detail_map(event)
                missing_keys = [k for k in ("bundle-id", "bundle-type", "entry-index") if k not in dmap]
                if missing_keys:
                    correlation_ok = False
                    failing_example = event
                    break
                bundle_types = [v for v in dmap.get("bundle-type", []) if v]
                if bundle_types and bundle_type not in bundle_types:
                    correlation_ok = False
                    failing_example = event
                    break

            checks.append(
                check(
                    f"interaction_{call_name}_correlation",
                    correlation_ok and has_per_entry,
                    f"{bundle_type} per-entry events include bundle-id/bundle-type/entry-index",
                    (entry_events[0] if correlation_ok and entry_events else failing_example),
                )
            )

    # Every recorded call should produce at least one audit event window
    for rec in ranges:
        produced = rec["end_line"] > rec["start_line"]
        checks.append(
            check(
                f"window_{rec['name']}",
                produced,
                f"Captured audit window for {rec['method']} {rec['path']}",
                call_events(output_dir, rec["name"])[0] if produced and call_events(output_dir, rec["name"]) else None,
            )
        )

    # Emit examples and coverage artifact
    coverage: dict[str, Any] = {
        "audit_file": str(audit_file),
        "total_events": len(events),
        "strict_correlation": args.strict_correlation,
        "checks": [],
        "excluded": [
            {
                "operation": "bulk export",
                "event_type": "object",
                "audit_operation": "bulk-export",
                "reason": "Not externally routable from the current hfs binary; persistence-layer helper exists but is not wired to public REST operations",
            },
            {
                "operation": "bulk submit/import",
                "event_type": "object",
                "audit_operation": "bulk-import",
                "reason": "Not externally routable from the current hfs binary; persistence-layer helper exists but is not wired to public REST operations",
            },
            {
                "operation": "purge",
                "event_type": "object",
                "audit_operation": "purge",
                "reason": "Not externally routable from the current hfs binary; persistence-layer helper exists but is not wired to public REST operations",
            },
            {
                "operation": "reindex",
                "event_type": "object",
                "audit_operation": "reindex",
                "reason": "Not externally routable from the current hfs binary; persistence-layer helper exists but is not wired to public REST operations",
            },
        ],
    }

    for item in checks:
        example_rel = write_example(output_dir, item["key"], item["example"])
        coverage["checks"].append(
            {
                "key": item["key"],
                "status": item["status"],
                "message": item["message"],
                "example_file": example_rel,
            }
        )

    (output_dir / "coverage.json").write_text(json.dumps(coverage, indent=2, sort_keys=True), encoding="utf-8")

    failed = [c for c in coverage["checks"] if c["status"] == "fail"]
    passed = [c for c in coverage["checks"] if c["status"] == "pass"]

    lines: list[str] = []
    lines.append("# HFS Audit Coverage Report")
    lines.append("")
    lines.append(f"- Total audit events parsed: **{len(events)}**")
    lines.append(f"- Checks passed: **{len(passed)}**")
    lines.append(f"- Checks failed: **{len(failed)}**")
    lines.append(f"- Strict correlation mode: **{'on' if args.strict_correlation else 'off'}**")
    lines.append("")

    lines.append("## Coverage Checks")
    lines.append("")
    lines.append("| Check | Status | Detail | Example |")
    lines.append("|---|---|---|---|")
    for c in coverage["checks"]:
        status_icon = "PASS" if c["status"] == "pass" else "FAIL"
        example = c["example_file"] or "-"
        lines.append(f"| `{c['key']}` | {status_icon} | {c['message']} | `{example}` |")

    lines.append("")
    lines.append("## Excluded Interactions")
    lines.append("")
    for excluded in coverage["excluded"]:
        lines.append(
            f"- `{excluded['operation']}` (`type={excluded['event_type']}`, `audit-operation={excluded['audit_operation']}`): {excluded['reason']}"
        )

    if failed:
        lines.append("")
        lines.append("## Failures")
        lines.append("")
        for c in failed:
            lines.append(f"- `{c['key']}`: {c['message']}")

    (output_dir / "report.md").write_text("\n".join(lines) + "\n", encoding="utf-8")

    print(f"Wrote coverage report: {output_dir / 'report.md'}")
    print(f"Wrote machine-readable coverage: {output_dir / 'coverage.json'}")

    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
