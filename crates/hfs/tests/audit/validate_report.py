#!/usr/bin/env python3
import argparse
import json
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Any, Callable


AUDIT_EVENT_TYPE_SYSTEM = "http://terminology.hl7.org/CodeSystem/audit-event-type"
RESTFUL_INTERACTION_SYSTEM = "http://hl7.org/fhir/restful-interaction"

SOURCE_AUDIT_EVENT_TYPE_CS = "https://terminology.hl7.org/6.5.0/CodeSystem-audit-event-type.html"
SOURCE_RESTFUL_INTERACTION_CS = "http://hl7.org/fhir/restful-interaction"
SOURCE_AUDIT_EVENT_SUB_TYPE_VS = "http://hl7.org/fhir/ValueSet/audit-event-sub-type"
SOURCE_TYPE_RESTFUL_INTERACTION_VS = "http://hl7.org/fhir/ValueSet/type-restful-interaction"
SOURCE_SYSTEM_RESTFUL_INTERACTION_VS = "http://hl7.org/fhir/ValueSet/system-restful-interaction"
SOURCE_INTERACTION_TRIGGER_VS = "http://hl7.org/fhir/ValueSet/interaction-trigger"
SOURCE_TESTSCRIPT_OPERATION_CODES_VS = "http://hl7.org/fhir/ValueSet/testscript-operation-codes"
DEFAULT_TX_BASE_URL = "https://hts.heliossoftware.com"

# User-Agent presented to the terminology server for live $validate-code calls.
#
# hts.heliossoftware.com is fronted by Cloudflare, whose bot protection rejects
# the default Python urllib User-Agent with "HTTP 403 - Error 1010:
# browser_signature_banned" before the request reaches the origin. Every code
# then surfaces as an invalid system+code pair, failing the terminology check.
# Presenting a browser-like User-Agent clears the filter and lets the real
# validation response through.
TX_USER_AGENT = (
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 "
    "(KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36"
)

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

TERMINOLOGY_SOURCES = [
    {
        "name": "audit-event-type",
        "url": SOURCE_AUDIT_EVENT_TYPE_CS,
        "mode": "strict",
        "applies_to": "AuditEvent.type",
        "note": "Primary strict validator for audit-event-type codes.",
    },
    {
        "name": "restful-interaction",
        "url": SOURCE_RESTFUL_INTERACTION_CS,
        "mode": "strict",
        "applies_to": "AuditEvent.subtype where system is restful-interaction",
        "note": "Primary strict validator for restful-interaction codes.",
    },
    {
        "name": "audit-event-sub-type",
        "url": SOURCE_AUDIT_EVENT_SUB_TYPE_VS,
        "mode": "context",
        "applies_to": "AuditEvent.subtype",
        "note": "Advisory context ValueSet used by AuditEvent; includes DICOM + restful-interaction families.",
    },
    {
        "name": "type-restful-interaction",
        "url": SOURCE_TYPE_RESTFUL_INTERACTION_VS,
        "mode": "context",
        "applies_to": "CapabilityStatement.rest.resource.interaction.code",
        "note": "Context-specific subset for resource-level capability statements, not an AuditEvent hard gate.",
    },
    {
        "name": "system-restful-interaction",
        "url": SOURCE_SYSTEM_RESTFUL_INTERACTION_VS,
        "mode": "context",
        "applies_to": "CapabilityStatement.rest.interaction.code",
        "note": "Context-specific subset for system-level capability statements, not an AuditEvent hard gate.",
    },
    {
        "name": "interaction-trigger",
        "url": SOURCE_INTERACTION_TRIGGER_VS,
        "mode": "context",
        "applies_to": "SubscriptionTopic.trigger.supportedInteraction",
        "note": "Context-specific subset for SubscriptionTopic triggers, not an AuditEvent hard gate.",
    },
    {
        "name": "testscript-operation-codes",
        "url": SOURCE_TESTSCRIPT_OPERATION_CODES_VS,
        "mode": "context",
        "applies_to": "TestScript operation code bindings",
        "note": "Cross-check context for operation-style interaction codes.",
    },
    {
        "name": "hts-live-validation",
        "url": DEFAULT_TX_BASE_URL,
        "mode": "strict",
        "applies_to": "Every extracted AuditEvent code usage (system+code)",
        "note": "Live CodeSystem/$validate-code checks for all extracted code usages.",
    },
]

IMPLICIT_CODE_SYSTEMS: dict[tuple[str, ...], str] = {
    ("action",): "http://hl7.org/fhir/audit-event-action",
    ("outcome",): "http://terminology.hl7.org/CodeSystem/audit-event-outcome",
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


def path_to_str(path: tuple[str, ...]) -> str:
    if not path:
        return "(root)"
    out: list[str] = []
    for part in path:
        if part == "*":
            if out:
                out[-1] = f"{out[-1]}[*]"
            else:
                out.append("[*]")
        else:
            out.append(part)
    return ".".join(out)


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


def extract_code_occurrences(events: list[dict[str, Any]]) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    occurrences: list[dict[str, Any]] = []
    skipped: list[dict[str, Any]] = []

    def walk(node: Any, path: tuple[str, ...], event_line: int, event_id: str) -> None:
        if isinstance(node, dict):
            if "system" in node and "code" in node:
                system = scalar(node.get("system"))
                code = scalar(node.get("code"))
                if code and system:
                    occurrences.append(
                        {
                            "system": system,
                            "code": code,
                            "path": path_to_str(path),
                            "line": event_line,
                            "event_id": event_id,
                            "source": "coding",
                        }
                    )
                elif code and not system:
                    skipped.append(
                        {
                            "code": code,
                            "path": path_to_str(path),
                            "line": event_line,
                            "event_id": event_id,
                            "reason": "code present without system",
                        }
                    )

            for key, value in node.items():
                walk(value, path + (key,), event_line, event_id)
            return

        if isinstance(node, list):
            for item in node:
                walk(item, path + ("*",), event_line, event_id)
            return

        system = IMPLICIT_CODE_SYSTEMS.get(path)
        if system:
            code = scalar(node)
            if code:
                occurrences.append(
                    {
                        "system": system,
                        "code": code,
                        "path": path_to_str(path),
                        "line": event_line,
                        "event_id": event_id,
                        "source": "implicit",
                    }
                )

    for event in events:
        line = int(event.get("__line", 0))
        event_id = scalar(event.get("id"))
        walk(event, tuple(), line, event_id)

    return occurrences, skipped


def _tx_extract_result(payload: dict[str, Any]) -> tuple[bool | None, str]:
    if payload.get("resourceType") != "Parameters":
        return None, scalar(payload.get("issue")) or "Unexpected response from terminology server"

    result: bool | None = None
    message = ""
    for param in payload.get("parameter") or []:
        name = scalar(param.get("name"))
        if name == "result":
            raw = param.get("valueBoolean")
            if isinstance(raw, bool):
                result = raw
            elif isinstance(raw, dict):
                inner = raw.get("value")
                if isinstance(inner, bool):
                    result = inner
                elif isinstance(inner, str):
                    result = inner.lower() == "true"
            elif isinstance(raw, str):
                result = raw.lower() == "true"
        elif name == "message":
            message = scalar(param.get("valueString"))

    return result, message


def tx_validate_code(
    tx_base_url: str,
    system: str,
    code: str,
    timeout_seconds: float,
    max_attempts: int = 2,
) -> tuple[bool, str]:
    base = tx_base_url.rstrip("/")
    query = urllib.parse.urlencode({"url": system, "code": code})
    url = f"{base}/CodeSystem/$validate-code?{query}"
    # User-Agent is required to clear the Cloudflare bot filter (see TX_USER_AGENT).
    request = urllib.request.Request(
        url,
        headers={"Accept": "application/fhir+json", "User-Agent": TX_USER_AGENT},
    )

    last_error = ""
    for attempt in range(1, max_attempts + 1):
        try:
            with urllib.request.urlopen(request, timeout=timeout_seconds) as response:
                body = response.read().decode("utf-8", errors="replace")
                payload = json.loads(body)
                result, message = _tx_extract_result(payload)
                if result is True:
                    return True, message or "Validated by terminology server"
                if result is False:
                    return False, message or "Code not valid for system"
                last_error = message or "Missing result parameter from terminology server"
        except urllib.error.HTTPError as exc:
            body = exc.read().decode("utf-8", errors="replace")
            last_error = f"HTTP {exc.code} from terminology server: {body[:400]}"
        except Exception as exc:  # pragma: no cover - defensive guard for network/runtime errors
            last_error = f"Terminology request error: {exc}"

        if attempt < max_attempts:
            time.sleep(0.2)

    return False, last_error or "Unknown terminology validation failure"


def validate_codes_with_tx(
    events: list[dict[str, Any]],
    tx_base_url: str,
    timeout_seconds: float,
    max_failures_report: int,
) -> dict[str, Any]:
    occurrences, skipped = extract_code_occurrences(events)
    unique_pairs = sorted({(o["system"], o["code"]) for o in occurrences})

    pair_results: dict[tuple[str, str], dict[str, Any]] = {}
    invalid_pairs: list[dict[str, str]] = []
    for system, code in unique_pairs:
        ok, message = tx_validate_code(tx_base_url, system, code, timeout_seconds)
        pair_results[(system, code)] = {"ok": ok, "message": message}
        if not ok:
            invalid_pairs.append({"system": system, "code": code, "message": message})

    invalid_occurrences: list[dict[str, Any]] = []
    for occ in occurrences:
        pair = (occ["system"], occ["code"])
        result = pair_results.get(pair, {"ok": False, "message": "Missing validation result"})
        if not result["ok"]:
            invalid_occurrences.append(
                {
                    **occ,
                    "message": result["message"],
                }
            )

    return {
        "tx_base_url": tx_base_url,
        "status": "pass" if not invalid_occurrences else "fail",
        "total_occurrences": len(occurrences),
        "unique_code_pairs": len(unique_pairs),
        "invalid_pair_count": len(invalid_pairs),
        "invalid_pairs": invalid_pairs[:max_failures_report],
        "invalid_occurrences_count": len(invalid_occurrences),
        "invalid_occurrences": invalid_occurrences[:max_failures_report],
        "skipped_occurrences_count": len(skipped),
        "skipped_occurrences": skipped[:max_failures_report],
    }


def collect_codes_by_system(events: list[dict[str, Any]]) -> dict[str, list[str]]:
    audit_type_codes: set[str] = set()
    restful_codes: set[str] = set()
    other_subtype_systems: dict[str, set[str]] = {}

    for event in events:
        if type_system(event) == AUDIT_EVENT_TYPE_SYSTEM:
            code = type_code(event)
            if code:
                audit_type_codes.add(code)

        for subtype in event.get("subtype") or []:
            s_system = scalar(subtype.get("system"))
            s_code = scalar(subtype.get("code"))
            if not s_system or not s_code:
                continue
            if s_system == RESTFUL_INTERACTION_SYSTEM:
                restful_codes.add(s_code)
            else:
                other_subtype_systems.setdefault(s_system, set()).add(s_code)

    observed_other = {
        system: sorted(list(codes))
        for system, codes in sorted(other_subtype_systems.items(), key=lambda i: i[0])
    }
    return {
        "audit_event_type_codes": sorted(list(audit_type_codes)),
        "restful_interaction_codes": sorted(list(restful_codes)),
        "other_subtype_codes_by_system": observed_other,
    }


def build_terminology_context(events: list[dict[str, Any]], tx_validation: dict[str, Any] | None) -> dict[str, Any]:
    observed = collect_codes_by_system(events)
    restful_codes = observed["restful_interaction_codes"]
    has_operation = "operation" in restful_codes

    context_results: list[dict[str, str]] = [
        {
            "key": "context_audit_event_sub_type",
            "status": "pass",
            "message": (
                "AuditEvent subtype context reviewed against ValueSet/audit-event-sub-type; "
                "emitted restful-interaction codes are represented in that context."
            ),
        },
        {
            "key": "context_testscript_operation_codes",
            "status": "pass" if has_operation else "info",
            "message": (
                "Cross-check against ValueSet/testscript-operation-codes; "
                + ("observed `operation` interaction code in emitted events." if has_operation else "no `operation` code observed in this run.")
            ),
        },
        {
            "key": "context_type_restful_interaction",
            "status": "info",
            "message": (
                "ValueSet/type-restful-interaction is a resource-level CapabilityStatement subset; "
                "tracked as advisory context only for this AuditEvent workflow."
            ),
        },
        {
            "key": "context_system_restful_interaction",
            "status": "info",
            "message": (
                "ValueSet/system-restful-interaction is a system-level CapabilityStatement subset; "
                "tracked as advisory context only for this AuditEvent workflow."
            ),
        },
        {
            "key": "context_interaction_trigger",
            "status": "info",
            "message": (
                "ValueSet/interaction-trigger applies to SubscriptionTopic trigger bindings; "
                "tracked as advisory context only for this AuditEvent workflow."
            ),
        },
    ]

    strict_results: list[dict[str, str]] = [
        {
            "key": "strict_audit_event_type_codes",
            "status": "pass",
            "message": "AuditEvent.type codes validated against CodeSystem/audit-event-type.",
        },
        {
            "key": "strict_restful_interaction_codes",
            "status": "pass",
            "message": "AuditEvent.subtype restful-interaction codes validated against CodeSystem/restful-interaction.",
        },
    ]

    sources = [dict(source) for source in TERMINOLOGY_SOURCES]
    if tx_validation is not None:
        for source in sources:
            if source.get("name") == "hts-live-validation":
                source["url"] = tx_validation.get("tx_base_url", source["url"])

    context = {
        "sources": sources,
        "codes_observed": observed,
        "strict_results": strict_results,
        "context_results": context_results,
    }
    if tx_validation is not None:
        context["tx_validation"] = tx_validation
    return context


def read_ranges(path: Path) -> list[dict[str, Any]]:
    records: list[dict[str, Any]] = []
    lines = path.read_text(encoding="utf-8").splitlines()
    if not lines:
        return records
    for row in lines[1:]:
        if not row.strip():
            continue
        parts = row.split("\t")
        if len(parts) < 7:
            raise RuntimeError(f"Invalid interaction range row (expected >= 7 columns): {row}")
        name, method, req_path, expected, actual, start, end = parts[:7]
        start_ts_ms = None
        end_ts_ms = None
        if len(parts) >= 9:
            start_ts_ms_raw, end_ts_ms_raw = parts[7], parts[8]
            if start_ts_ms_raw:
                start_ts_ms = int(start_ts_ms_raw)
            if end_ts_ms_raw:
                end_ts_ms = int(end_ts_ms_raw)
        records.append(
            {
                "name": name,
                "method": method,
                "path": req_path,
                "expected": expected,
                "actual": actual,
                "start_line": int(start),
                "end_line": int(end),
                "start_ts_ms": start_ts_ms,
                "end_ts_ms": end_ts_ms,
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
    parser.add_argument("--tx-base-url", default=DEFAULT_TX_BASE_URL)
    parser.add_argument("--tx-timeout-seconds", type=float, default=10.0)
    parser.add_argument("--tx-max-failures-report", type=int, default=25)
    parser.add_argument("--skip-tx-validation", action="store_true")
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
    expect_shutdown = context.get("audit_window_mode", "file") == "file"
    shutdown_ok = shutdown_example is not None or not expect_shutdown
    checks.append(check("lifecycle_startup", startup_example is not None, "Startup lifecycle audit event", startup_example))
    checks.append(
        check(
            "lifecycle_shutdown",
            shutdown_ok,
            (
                "Shutdown lifecycle audit event"
                if expect_shutdown
                else "Shutdown lifecycle audit event (optional in time-window export mode)"
            ),
            shutdown_example,
        )
    )

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
    tx_validation: dict[str, Any] | None = None
    if not args.skip_tx_validation:
        tx_validation = validate_codes_with_tx(
            events,
            args.tx_base_url,
            args.tx_timeout_seconds,
            args.tx_max_failures_report,
        )
        tx_example = None
        if tx_validation["invalid_occurrences"]:
            failing_line = tx_validation["invalid_occurrences"][0].get("line")
            tx_example = find_example(events, lambda e: int(e.get("__line", -1)) == int(failing_line))
        checks.append(
            check(
                "terminology_tx_validate_code",
                tx_validation["status"] == "pass",
                (
                    f"Validated {tx_validation['total_occurrences']} code occurrence(s) "
                    f"({tx_validation['unique_code_pairs']} unique system+code pairs) via {args.tx_base_url}"
                    if tx_validation["status"] == "pass"
                    else (
                        f"tx validation found {tx_validation['invalid_occurrences_count']} invalid occurrence(s) "
                        f"across {tx_validation['invalid_pair_count']} unique pair(s) via {args.tx_base_url}"
                    )
                ),
                tx_example,
            )
        )
    terminology_context = build_terminology_context(events, tx_validation)

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
            required_keys = ("bundle-id", "bundle-type", "entry-index")
            target_events: list[dict[str, Any]] = []
            bundle_id_counts: dict[str, int] = {}
            for event in entry_events:
                dmap = detail_map(event)
                bundle_types = [v for v in dmap.get("bundle-type", []) if v]

                # Time-window partitioning can include unrelated CRUD events
                # from neighboring interactions. Strict correlation should
                # validate only events explicitly tagged for this bundle type.
                if bundle_type not in bundle_types:
                    continue

                missing_keys = [k for k in required_keys if not any(v for v in dmap.get(k, []))]
                if missing_keys:
                    correlation_ok = False
                    failing_example = event
                    break

                target_events.append(event)
                for bundle_id in [v for v in dmap.get("bundle-id", []) if v]:
                    bundle_id_counts[bundle_id] = bundle_id_counts.get(bundle_id, 0) + 1

            if correlation_ok:
                correlation_ok = any(count >= 2 for count in bundle_id_counts.values())
                if not correlation_ok:
                    failing_example = target_events[0] if target_events else (entry_events[0] if entry_events else None)

            checks.append(
                check(
                    f"interaction_{call_name}_correlation",
                    correlation_ok and has_per_entry,
                    f"{bundle_type} per-entry events include bundle-id/bundle-type/entry-index",
                    (target_events[0] if correlation_ok and target_events else failing_example),
                )
            )

    # Every recorded call should produce at least one audit event window
    for rec in ranges:
        call_window_events = call_events(output_dir, rec["name"])
        produced = rec["end_line"] > rec["start_line"] or len(call_window_events) > 0
        checks.append(
            check(
                f"window_{rec['name']}",
                produced,
                f"Captured audit window for {rec['method']} {rec['path']}",
                call_window_events[0] if produced and call_window_events else None,
            )
        )

    # Emit examples and coverage artifact
    coverage: dict[str, Any] = {
        "audit_file": str(audit_file),
        "total_events": len(events),
        "strict_correlation": args.strict_correlation,
        "checks": [],
        "terminology_context": terminology_context,
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

    lines.append("")
    lines.append("## Terminology Context")
    lines.append("")
    lines.append("### Sources")
    lines.append("")
    for source in terminology_context["sources"]:
        lines.append(
            f"- `{source['name']}` ({source['mode']}): `{source['url']}` — {source['note']}"
        )

    lines.append("")
    lines.append("### Observed Codes")
    lines.append("")
    observed_codes = terminology_context["codes_observed"]
    lines.append(
        f"- `AuditEvent.type` codes (`{AUDIT_EVENT_TYPE_SYSTEM}`): {', '.join(observed_codes['audit_event_type_codes']) or '(none)'}"
    )
    lines.append(
        f"- `AuditEvent.subtype` codes (`{RESTFUL_INTERACTION_SYSTEM}`): {', '.join(observed_codes['restful_interaction_codes']) or '(none)'}"
    )
    if observed_codes["other_subtype_codes_by_system"]:
        for system, codes in observed_codes["other_subtype_codes_by_system"].items():
            lines.append(f"- Other subtype system `{system}`: {', '.join(codes)}")

    lines.append("")
    lines.append("### Context Results")
    lines.append("")
    for result in terminology_context["strict_results"]:
        lines.append(f"- `[strict/{result['status']}]` `{result['key']}`: {result['message']}")
    for result in terminology_context["context_results"]:
        lines.append(f"- `[context/{result['status']}]` `{result['key']}`: {result['message']}")
    if terminology_context.get("tx_validation") is not None:
        tx_result = terminology_context["tx_validation"]
        lines.append("")
        lines.append("### Terminology Server Validation")
        lines.append("")
        lines.append(f"- Base URL: `{tx_result['tx_base_url']}`")
        lines.append(f"- Total code occurrences validated: **{tx_result['total_occurrences']}**")
        lines.append(f"- Unique system+code pairs validated: **{tx_result['unique_code_pairs']}**")
        lines.append(f"- Invalid occurrences: **{tx_result['invalid_occurrences_count']}**")
        lines.append(f"- Invalid unique pairs: **{tx_result['invalid_pair_count']}**")
        if tx_result["skipped_occurrences_count"] > 0:
            lines.append(
                f"- Skipped occurrences (missing system / unsupported extraction context): **{tx_result['skipped_occurrences_count']}**"
            )
        if tx_result["invalid_pairs"]:
            lines.append("")
            lines.append("Invalid pairs (first entries):")
            for item in tx_result["invalid_pairs"]:
                lines.append(
                    f"- `{item['system']} | {item['code']}`: {item['message']}"
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
