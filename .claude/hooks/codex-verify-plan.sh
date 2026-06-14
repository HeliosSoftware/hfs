#!/usr/bin/env bash
#
# codex-verify-plan.sh - HFS-specialized plan gate.
#
# Runs as a PreToolUse hook on the ExitPlanMode tool. When Claude tries to
# submit a plan, Codex reviews it as a senior HFS architect against
# schemas/codex-plan-review.schema.json and either allows the exit (pass) or
# denies it with concrete required changes (fail).
#
# Fail-closed: if neither Codex nor the safe Claude fallback can run, the plan
# is denied. Set CODEX_PLAN_GATE_DISABLE=1 to skip the gate entirely (escape
# hatch). Set CODEX_PLAN_GATE_CLAUDE_FALLBACK=0 to require Codex.
#
set -euo pipefail

project_dir="${CLAUDE_PROJECT_DIR:-$(git rev-parse --show-toplevel 2>/dev/null || pwd)}"
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
codex_bin="${CODEX_BIN:-$(command -v codex || true)}"
if [[ -z "$codex_bin" && -x "$HOME/.local/bin/codex" ]]; then
  codex_bin="$HOME/.local/bin/codex"
fi
claude_bin="${CLAUDE_BIN:-$(command -v claude || true)}"
if [[ -z "$claude_bin" && -x "$HOME/.local/bin/claude" ]]; then
  claude_bin="$HOME/.local/bin/claude"
fi
claude_fallback_enabled="${CODEX_PLAN_GATE_CLAUDE_FALLBACK:-1}"
debug_dir="$project_dir/.claude/debug"
state_root="$project_dir/.claude/state/codex-plan-gate"
schema="${CODEX_PLAN_REVIEW_SCHEMA:-$script_dir/../schemas/codex-plan-review.schema.json}"

# Escape hatch: never block planning when explicitly disabled.
if [[ -n "${CODEX_PLAN_GATE_DISABLE:-}" ]]; then
  exit 0
fi

mkdir -p "$debug_dir" "$state_root"

# Shared observability helpers (timestamped logs, JSONL events, attempt snapshots).
# shellcheck source=/dev/null
source "$script_dir/codex-gate-log.sh"

gate_main_log="$debug_dir/codex-plan-gate.log"
GATE_WF=""
GATE_ATTEMPT=""

payload="$(cat)"
printf '%s\n' "$payload" > "$debug_dir/pretooluse-exit-plan-last.json"

json_get() {
  local expression="$1"
  PAYLOAD="$payload" python3 - "$expression" <<'PY'
import json
import os
import sys

data = json.loads(os.environ["PAYLOAD"])
value = data
for part in sys.argv[1].split("."):
    if not part:
        continue
    if isinstance(value, dict):
        value = value.get(part, "")
    else:
        value = ""
        break
if value is None:
    value = ""
if isinstance(value, (dict, list)):
    print(json.dumps(value, ensure_ascii=False))
else:
    print(value)
PY
}

deny() {
  local reason="$1"
  gate_log_event "plan-gate" "deny" "workflow_type=${GATE_WF:-}" "attempt=${GATE_ATTEMPT:-}" "reason=${reason:0:300}"
  python3 - "$reason" <<'PY'
import json
import sys

print(json.dumps({
    "hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "permissionDecision": "deny",
        "permissionDecisionReason": sys.argv[1],
    }
}))
PY
  exit 0
}

session_id="$(json_get session_id)"
transcript_path="$(json_get transcript_path)"
safe_session_id="$(printf '%s' "${session_id:-unknown}" | tr -c 'A-Za-z0-9_.-' '_')"
session_dir="$state_root/$safe_session_id"
mkdir -p "$session_dir"
accepted_plan="$session_dir/accepted-plan.md"
skip_next_stop_review="$session_dir/skip-next-stop-review"
attempt_counter="$session_dir/plan-attempt-count"

gate_prune_sessions "$state_root"
gate_log_line "$gate_main_log" "session=$session_id transcript=$transcript_path"
printf '%s\n' "$payload" > "$session_dir/plan-hook-input.json"

if [[ ! -f "$schema" ]]; then
  deny "Codex plan verifier schema is missing: $schema"
fi

prompt_file="$session_dir/plan-review-prompt.md"
review_file="$session_dir/plan-review.json"
codex_stdout="$session_dir/plan-codex.stdout"
codex_stderr="$session_dir/plan-codex.stderr"
claude_stdout="$session_dir/plan-claude.stdout"
claude_stderr="$session_dir/plan-claude.stderr"
reviewer_engine="codex"
codex_available=1

if [[ -z "$codex_bin" || ! -x "$codex_bin" ]]; then
  codex_available=0
  if [[ "$claude_fallback_enabled" != "0" && -n "$claude_bin" && -x "$claude_bin" ]]; then
    reviewer_engine="claude"
    gate_log_event "plan-gate" "fallback" "reason=codex-unavailable" "fallback=claude"
    gate_log_line "$gate_main_log" "codex unavailable; falling back to claude safe-mode verifier"
  else
    deny "Codex plan verifier could not run because no executable Codex binary was found (CODEX_BIN='${CODEX_BIN:-}'), and the safe Claude fallback is unavailable or disabled (CLAUDE_BIN='${CLAUDE_BIN:-}', CODEX_PLAN_GATE_CLAUDE_FALLBACK=$claude_fallback_enabled). Install Codex, set CODEX_BIN, install Claude, or set CODEX_PLAN_GATE_DISABLE=1 to bypass the gate."
  fi
fi

codex_args=()
search_enabled=0

if [[ "$codex_available" == "1" ]]; then
  codex_help="$("$codex_bin" --help 2>&1 || true)"
  codex_exec_help="$("$codex_bin" exec --help 2>&1 || true)"

  if printf '%s\n' "$codex_help" | grep -q -- '--ask-for-approval'; then
    codex_args+=(--ask-for-approval never)
  fi

  if printf '%s\n' "$codex_help" | grep -q -- '--search'; then
    codex_args+=(--search)
    search_enabled=1
  fi

  codex_args+=(exec -C "$project_dir")

  if printf '%s\n' "$codex_exec_help" | grep -q -- '--sandbox'; then
    codex_args+=(--sandbox read-only)
  fi

  if printf '%s\n' "$codex_exec_help" | grep -q -- '--ephemeral'; then
    codex_args+=(--ephemeral)
  fi

  if ! printf '%s\n' "$codex_exec_help" | grep -q -- '--output-schema'; then
    deny "Codex plan verifier requires codex exec --output-schema, but this Codex CLI does not advertise it. Update Codex or set CODEX_BIN to a compatible binary."
  fi

  if ! printf '%s\n' "$codex_exec_help" | grep -q -- '--output-last-message'; then
    deny "Codex plan verifier requires codex exec --output-last-message/-o, but this Codex CLI does not advertise it. Update Codex or set CODEX_BIN to a compatible binary."
  fi

  codex_args+=(--output-schema "$schema" -o "$review_file" -)
fi

transcript_tail="$(
  TRANSCRIPT_PATH="$transcript_path" python3 <<'PY'
import os
from pathlib import Path

path_text = os.environ.get("TRANSCRIPT_PATH", "")
if not path_text:
    raise SystemExit(0)

path = Path(path_text).expanduser()
if not path.exists():
    raise SystemExit(0)

lines = path.read_text(errors="replace").splitlines()
tail = "\n".join(lines[-250:])
print(tail[-200000:])
PY
)"

# Resolve and read the exact plan Claude is submitting, so Codex reviews the
# final plan verbatim. Preference order, most authoritative first:
#   1. tool_input.plan - the plan literally being submitted through this
#      ExitPlanMode call. This is the source of truth and must win.
#   2. The last .claude/plans/*.md path referenced in this session's transcript.
#   3. The most-recently-modified plan file in ~/.claude/plans.
# Fallbacks 2 and 3 read a shared, cross-session directory, so they can resolve
# to an unrelated plan (another project/session). Only consult them when the
# submitted call carries no inline plan.
plan_from_input="$(json_get tool_input.plan)"

plan_text=""
plan_source=""
if [[ "$plan_from_input" =~ [^[:space:]] ]]; then
  plan_text="$plan_from_input"
  plan_source="ExitPlanMode tool_input.plan"
fi

if ! [[ "$plan_text" =~ [^[:space:]] ]]; then
  plan_file="$(
    TRANSCRIPT_PATH="$transcript_path" python3 <<'PY'
import os
import re
from pathlib import Path

tp = os.environ.get("TRANSCRIPT_PATH", "")
candidate = ""
if tp:
    p = Path(tp).expanduser()
    if p.exists():
        matches = re.findall(r'/[^"\\]*?/\.claude/plans/[A-Za-z0-9._-]+\.md', p.read_text(errors="replace"))
        if matches:
            candidate = matches[-1]
print(candidate)
PY
  )"

  if [[ -z "$plan_file" || ! -f "$plan_file" ]]; then
    plans_dir="$HOME/.claude/plans"
    if [[ -d "$plans_dir" ]]; then
      plan_file="$(ls -1t "$plans_dir"/*.md 2>/dev/null | head -1 || true)"
    fi
  fi

  if [[ -n "$plan_file" && -f "$plan_file" ]]; then
    plan_text="$(cat "$plan_file")"
    plan_source="plan file (fallback): $plan_file"
  fi
fi
gate_log_line "$gate_main_log" "plan_source=${plan_source:-none}"

url_scan_blob="$(printf '%s\n%s\n%s\n' "$payload" "$transcript_tail" "$plan_text")"
if [[ "$search_enabled" != "1" ]] && grep -Eqi 'https?://' <<< "$url_scan_blob"; then
  deny "Codex plan verifier cannot inspect referenced HTTP/HTTPS URLs because this Codex CLI does not advertise --search. Update Codex or remove/inline the required external context before exiting plan mode."
fi

cat > "$prompt_file" <<EOF
You are Codex acting as an independent plan gate for Claude Code working on the Helios HFS repository. HFS is a Rust workspace implementing a multi-version FHIR server (R4/R4B/R5/R6 via feature flags), a FHIRPath engine, SQL-on-FHIR, a terminology server, and a tenant-first polyglot persistence layer. There is no frontend. Act as a senior HFS backend architect with technical product review discipline: validate architecture, implementation readiness, user constraints, acceptance criteria, and verification strategy.

Return only JSON matching the provided output schema. Do not edit files. Do not run mutating commands.

If the proposed plan or transcript references HTTP/HTTPS URLs, documentation pages, issues, PRs, specs, or other external sources that materially affect the implementation, inspect them before deciding. Fail the plan if required external context is inaccessible, ignored, or not reflected accurately in the plan.

Review the proposed plan that Claude is trying to submit through ExitPlanMode. The exact plan is provided verbatim in the PROPOSED PLAN section below; treat it as authoritative. Use the hook payload and transcript tail only as supplementary context (e.g. user constraints stated earlier in the conversation). Decide:
- "pass" only if every universal checklist field is true, every applicable domain checklist field is true, every issue array is empty, and the plan is decision-complete and ready for implementation.
- "fail" if the plan has missing steps, unresolved decisions, missing tests/verification, violated user constraints, or unclear acceptance criteria.

First classify workflow_type exactly:
- server_api: REST/Axum work in helios-rest - handlers, routes, extractors, middleware, multi-tenancy routing, request/response and compression behavior, FHIR OperationOutcome error handling, endpoint tests. Authentication/authorization (helios-auth: SMART/OAuth2 JWT, JWKS, scopes, JTI cache) and audit logging (helios-audit: AuditEvent/BALP middleware and sinks) are request-path concerns and classify here; surface storage-sink aspects of audit in persistence_issues when relevant.
- persistence: storage/persistence work in helios-persistence - backends (SQLite/PostgreSQL/Elasticsearch/MongoDB), composite storage, search registry, tenant isolation, trait hierarchy, capability advertisement, integration tests.
- fhir_core: FHIR data models (helios-fhir), serialization (helios-serde), FHIRPath (helios-fhirpath), SQL-on-FHIR (helios-sof) - version-gated types, enum wrappers, parser/evaluator/transformation logic, conformance tests, code generation. CDS Hooks protocol types and the CdsHooksService trait (helios-cds-hooks, a standalone library) classify here unless a server endpoint is being wired (then server_api).
- subscriptions: FHIR Subscriptions engine work in helios-subscriptions - SubscriptionTopic/event matching, delivery channels (rest-hook, websocket, email, messaging), notification building, retry/backoff, endpoint-safety policy.
- tooling_config: hooks, skills, CLAUDE.md, CI/config, scripts, settings.json, Docker/release, or developer workflow configuration.
Pick the single dominant domain. For coupled work spanning domains, pick the one carrying the most risk and surface the other domain's gaps in its issue array.

Universal checklist semantics (the "checklist" object - all must be true to pass):
- clear_goal: desired outcome and user-visible result are explicit.
- user_constraints_identified: relevant user constraints and preferences from the transcript are reflected in the plan.
- concrete_implementation_steps: another engineer could execute the plan without inventing major steps.
- implementation_targets_identified: affected workspace crates, modules, files, or commands are named where needed.
- decisions_resolved: no "maybe", "decide later", or unresolved implementation choices.
- test_or_verification_strategy: concrete checks/tests/manual verification are named.
- acceptance_criteria_clear: success can be objectively evaluated.
- risk_and_edge_cases_considered: likely failure modes, compatibility concerns, or edge cases are covered where relevant.
- required_checks_named: the plan names the developer pre-commit gate where code changes - cargo fmt --all, CI clippy (cargo clippy --all-targets --all-features -- -D warnings), and an affected cargo test; plus, when crates/pysof is touched, cd crates/pysof && uv run pytest python-tests/ -v and cd crates/pysof && cargo test. True only if such checks are genuinely unnecessary (e.g. pure docs) and that is justified.
- ci_coverage_considered: HFS does much of its testing on GitHub Actions runners, not on the developer machine. The plan must account for this. On every PR to main (and push to main/develop), .github/workflows/ci.yml runs lint (cargo fmt --all -- --check; cargo clippy --all-targets --all-features -- -D warnings with repo-specific -A allows), test-rust (cargo test --workspace --all-features on self-hosted Linux runners using testcontainers), plus coverage, security, test-python, test-fhirpath, hts-conformance, and build. ci-extended.yml runs the full --workspace --all-features suite nightly on beta/nightly and as a release gate. Heavier conformance/smoke suites run as their own workflows: inferno-us-core, inferno-bulk-data, inferno-bulk-submit-data, inferno-subscription, audit-events, bulk-export-smoke, bulk-submit-smoke, subscriptions-channels, subscriptions-smoke, hts-ig-conformance, hts-benchmark. ci_coverage_considered is true when the plan identifies which workflow(s) will validate the change, flags that a workflow must be added or updated to cover new behavior, or justifies why CI coverage is unaffected. Deferring heavy integration/conformance/full-all-features testing to these runners is the EXPECTED workflow - do not fail a plan merely because such tests are run in CI rather than locally; only fail if the change would land in a CI gap with no runner validating it and the plan ignores that gap.
- fhir_version_impact_considered: the plan states which FHIR versions / feature flags (R4/R4B/R5/R6) are affected and how multi-version gating is handled, or justifies why FHIR versioning does not apply.
- crate_boundaries_correct: changes land in the correct workspace crate(s) for the work.
- reuse_over_duplication: the plan reuses existing workspace traits, enums, structs, functions, and utilities where suitable instead of proposing parallel or duplicate implementations of things that already exist (e.g. version-agnostic enum wrappers like SofViewDefinition, the ResourceStorage/VersionedStorage trait hierarchy, shared helios-serde-support / helios-fhirpath-support helpers, helios-auth JWKS/JTI caches and scope/policy types, helios-audit AuditSink/AuditEventBuilder/middleware, the helios-subscriptions SubscriptionEngine/channels/evaluator, and helios-cds-hooks protocol types). The plan should name the existing abstractions it builds on; any genuinely new type/abstraction must be justified. Use the read-only repo access to check whether a suitable abstraction already exists. True when the work is small/isolated enough that no meaningful reuse opportunity applies. Record reuse/duplication problems in missing_steps and required_changes.
- build_cost_acknowledged: if a full multi-version build (--features R4,R4B,R5,R6) or FHIR code generation (fhir-gen, which can download R6 specs) is required locally, it is flagged as expensive (>10 min) and ideally deferred to CI runners. True if no such expensive operation is needed. Do not demand the developer run full --workspace --all-features builds/tests locally when the CI runners already cover them.

Issue arrays are authoritative:
- Put every missing implementation action in missing_steps.
- Put every unresolved choice in unresolved_decisions.
- Put every missing or weak check in missing_tests_or_verification.
- Put every user constraint conflict in violated_constraints.
- Put every unclear done/success condition in unclear_acceptance_criteria.

Domain-specific checklist enforcement:
- server_api: server_api_checklist must be fully true and server_api_issues must be empty.
- persistence: persistence_checklist must be fully true and persistence_issues must be empty.
- fhir_core: fhir_core_checklist must be fully true and fhir_core_issues must be empty.
- subscriptions: subscriptions_checklist must be fully true and subscriptions_issues must be empty.
- tooling_config: tooling_config_checklist must be fully true and tooling_config_issues must be empty.
- Fill non-applicable domain checklists with true values and leave their issue arrays empty unless they reveal a real cross-cutting concern.

Domain checklist semantics:
- server_api_checklist: handler_and_route_registration_planned (handler in crates/rest/src/handlers/, route in crates/rest/src/routes.rs); request_response_contract_defined (extractors, content negotiation, compression, payload shapes); operation_outcome_error_paths_considered (FHIR OperationOutcome + correct HTTP status codes); endpoint_tests_named.
- persistence_checklist: tenant_context_first_and_isolation_preserved (TenantContext is the first argument; tenant boundaries enforced at the query level); trait_hierarchy_and_capabilities_addressed (ResourceStorage -> VersionedStorage / SearchProvider / TransactionProvider hierarchy; new backends advertise via CapabilityProvider); backend_compatibility_and_migration_considered (SQLite/PostgreSQL/Elasticsearch/MongoDB/composite); persistence_integration_tests_named (testcontainers where relevant).
- fhir_core_checklist: fhir_version_coverage_planned (R4/R4B/R5/R6 feature flags, or justified single-version scope); version_agnostic_abstraction_preserved (enum wrappers like SofViewDefinition and traits; only R4 default may be assumed); serde_or_codegen_impact_considered (helios-serde, fhir-gen regeneration if schema-driven); fhir_conformance_tests_named (FHIRPath/SOF suites or spec test data).
- subscriptions_checklist: topic_and_event_matching_planned (SubscriptionTopic criteria and resource-event evaluation/filter matching); channel_delivery_planned (rest-hook/websocket/email/messaging channels and notification bundle shape, reusing SubscriptionEngine/channels rather than ad-hoc delivery); endpoint_safety_and_retry_planned (private/loopback endpoint policy via HFS_SUBSCRIPTION_ALLOW_PRIVATE_ENDPOINTS, plus delivery retry/backoff); subscription_tests_named (local tests or the inferno-subscription / subscriptions-channels / subscriptions-smoke workflows).
- tooling_config_checklist: affected_config_or_docs_identified (settings.json, hooks, skills, CLAUDE.md, CI); hook_or_workflow_compatibility_considered (does not break the existing hfs-policy.sh stop-gate; notes jq/python3 availability); apply_or_reload_steps_named (e.g. restart session to reload settings); verification_command_named.
For the domain "tests named" fields above (endpoint_tests_named, persistence_integration_tests_named, fhir_conformance_tests_named, subscription_tests_named) and for verification_command_named, naming the GitHub Actions workflow that validates the change (e.g. inferno-us-core, inferno-bulk-data, inferno-bulk-submit-data, inferno-subscription, subscriptions-channels, subscriptions-smoke, the ci.yml test-rust / test-fhirpath / hts-conformance jobs, or ci-extended) counts as satisfying the requirement; the tests do not have to run on the developer machine. New behavior that no existing workflow exercises should either add/extend a workflow or name a local test, and the plan must say which.

Domain issue arrays:
- server_api_issues: missing API/interface contract, handler/route registration, error/permission path, or endpoint test planning.
- persistence_issues: missing tenant isolation, trait/capability handling, backend compatibility/migration, or integration test planning.
- fhir_core_issues: missing FHIR version coverage, version-agnostic abstraction, serde/codegen impact, or conformance test planning.
- subscriptions_issues: missing topic/event matching, channel delivery, endpoint-safety/retry handling, or subscription test planning.
- tooling_config_issues: missing config/docs target, hook/workflow compatibility, apply/reload behavior, or verification command.

If decision is "pass", accepted_plan_markdown must contain the clean accepted plan markdown.
If decision is "fail", required_changes must list concrete, actionable changes Claude must make before implementation.

PROPOSED PLAN (verbatim, source: ${plan_source:-unavailable - fall back to tool_input/transcript}):
<<<HFS_PLAN_BEGIN>>>
$plan_text
<<<HFS_PLAN_END>>>

Hook payload JSON:
\`\`\`json
$payload
\`\`\`

Transcript tail:
\`\`\`jsonl
$transcript_tail
\`\`\`
EOF

gate_codex_t0="$(date +%s)"
codex_rc=0
if [[ "$reviewer_engine" == "claude" ]]; then
  # claude --print does not write the bare schema object to stdout the way
  # codex -o does. With --output-format json it emits a result envelope whose
  # schema-conforming verdict lives under .structured_output; extract that into
  # review_file so the downstream validator sees the same shape codex produces.
  CLAUDE_CODE_SAFE_MODE=1 "$claude_bin" \
    --safe-mode \
    --print \
    --output-format json \
    --permission-mode dontAsk \
    --allowedTools Read,Grep,Glob \
    --add-dir "$project_dir" \
    --json-schema "$(cat "$schema")" \
    < "$prompt_file" > "$claude_stdout" 2> "$claude_stderr" || codex_rc=$?
  if [[ "$codex_rc" -eq 0 ]]; then
    python3 - "$claude_stdout" "$review_file" <<'PY' || codex_rc=$?
import json
import sys
from pathlib import Path

env = json.loads(Path(sys.argv[1]).read_text())
verdict = env.get("structured_output")
if verdict is None:
    raise SystemExit(1)
Path(sys.argv[2]).write_text(json.dumps(verdict))
PY
  fi
else
  "$codex_bin" "${codex_args[@]}" < "$prompt_file" > "$codex_stdout" 2> "$codex_stderr" || codex_rc=$?
fi
gate_codex_s=$(( $(date +%s) - gate_codex_t0 ))
gate_snapshot_attempt "$attempt_counter"
gate_log_line "$gate_main_log" "verifier_invoked engine=$reviewer_engine attempt=$GATE_ATTEMPT rc=$codex_rc duration_s=$gate_codex_s"

if [[ "$codex_rc" -ne 0 ]]; then
  verifier_stderr="$codex_stderr"
  [[ "$reviewer_engine" == "claude" ]] && verifier_stderr="$claude_stderr"
  reason="${reviewer_engine} rejected or failed during plan verification. Revise the plan only after addressing verifier execution errors. stderr: $(tail -n 20 "$verifier_stderr" | tr '\n' ' ')"
  gate_log_line "$gate_main_log" "$reason"
  deny "$reason"
fi

if ! python3 - "$review_file" <<'PY'
import json
import sys
from pathlib import Path

UNIVERSAL_CHECKS = [
    "clear_goal",
    "user_constraints_identified",
    "concrete_implementation_steps",
    "implementation_targets_identified",
    "decisions_resolved",
    "test_or_verification_strategy",
    "acceptance_criteria_clear",
    "risk_and_edge_cases_considered",
    "required_checks_named",
    "ci_coverage_considered",
    "fhir_version_impact_considered",
    "crate_boundaries_correct",
    "reuse_over_duplication",
    "build_cost_acknowledged",
]
UNIVERSAL_ISSUES = [
    "missing_steps",
    "unresolved_decisions",
    "missing_tests_or_verification",
    "violated_constraints",
    "unclear_acceptance_criteria",
    "required_changes",
]
DOMAIN_CHECKLISTS = {
    "server_api_checklist": [
        "handler_and_route_registration_planned",
        "request_response_contract_defined",
        "operation_outcome_error_paths_considered",
        "endpoint_tests_named",
    ],
    "persistence_checklist": [
        "tenant_context_first_and_isolation_preserved",
        "trait_hierarchy_and_capabilities_addressed",
        "backend_compatibility_and_migration_considered",
        "persistence_integration_tests_named",
    ],
    "fhir_core_checklist": [
        "fhir_version_coverage_planned",
        "version_agnostic_abstraction_preserved",
        "serde_or_codegen_impact_considered",
        "fhir_conformance_tests_named",
    ],
    "subscriptions_checklist": [
        "topic_and_event_matching_planned",
        "channel_delivery_planned",
        "endpoint_safety_and_retry_planned",
        "subscription_tests_named",
    ],
    "tooling_config_checklist": [
        "affected_config_or_docs_identified",
        "hook_or_workflow_compatibility_considered",
        "apply_or_reload_steps_named",
        "verification_command_named",
    ],
}
DOMAIN_ISSUES = [
    "server_api_issues",
    "persistence_issues",
    "fhir_core_issues",
    "subscriptions_issues",
    "tooling_config_issues",
]

try:
    review = json.loads(Path(sys.argv[1]).read_text())
    decision = review.get("decision")
    workflow_type = review.get("workflow_type")
    accepted_plan = review.get("accepted_plan_markdown", "")
    checklist = review.get("checklist")
    if decision not in {"pass", "fail"}:
        raise ValueError("invalid decision")
    if workflow_type not in {"server_api", "persistence", "fhir_core", "subscriptions", "tooling_config"}:
        raise ValueError("invalid workflow_type")
    if not isinstance(checklist, dict):
        raise ValueError("missing checklist")
    for key in UNIVERSAL_CHECKS:
        if not isinstance(checklist.get(key), bool):
            raise ValueError(f"invalid checklist.{key}")
    for key in UNIVERSAL_ISSUES:
        if not isinstance(review.get(key), list):
            raise ValueError(f"invalid {key}")
    for checklist_name, keys in DOMAIN_CHECKLISTS.items():
        domain_checklist = review.get(checklist_name)
        if not isinstance(domain_checklist, dict):
            raise ValueError(f"missing {checklist_name}")
        for key in keys:
            if not isinstance(domain_checklist.get(key), bool):
                raise ValueError(f"invalid {checklist_name}.{key}")
    for key in DOMAIN_ISSUES:
        if not isinstance(review.get(key), list):
            raise ValueError(f"invalid {key}")
    if decision == "pass" and not accepted_plan.strip():
        raise ValueError("missing accepted plan")
except Exception:
    raise SystemExit(1)
PY
then
  deny "${reviewer_engine} plan verifier returned invalid JSON or an invalid pass without accepted_plan_markdown. Revise the plan and try again."
fi

# Verdict is structurally valid from here; capture workflow_type for event logging.
GATE_WF="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1])).get("workflow_type",""))' "$review_file" 2>/dev/null || true)"

decision="$(python3 - "$review_file" <<'PY'
import json
import sys
from pathlib import Path

review = json.loads(Path(sys.argv[1]).read_text())
workflow_type = review.get("workflow_type")
checklist = review.get("checklist", {})
universal_checks = [
    "clear_goal",
    "user_constraints_identified",
    "concrete_implementation_steps",
    "implementation_targets_identified",
    "decisions_resolved",
    "test_or_verification_strategy",
    "acceptance_criteria_clear",
    "risk_and_edge_cases_considered",
    "required_checks_named",
    "ci_coverage_considered",
    "fhir_version_impact_considered",
    "crate_boundaries_correct",
    "reuse_over_duplication",
    "build_cost_acknowledged",
]
issue_keys = [
    "missing_steps",
    "unresolved_decisions",
    "missing_tests_or_verification",
    "violated_constraints",
    "unclear_acceptance_criteria",
]
workflow_requirements = {
    "server_api": ("server_api_checklist", "server_api_issues"),
    "persistence": ("persistence_checklist", "persistence_issues"),
    "fhir_core": ("fhir_core_checklist", "fhir_core_issues"),
    "subscriptions": ("subscriptions_checklist", "subscriptions_issues"),
    "tooling_config": ("tooling_config_checklist", "tooling_config_issues"),
}
required_true = all(checklist.get(key) is True for key in universal_checks)
no_issues = all(not review.get(key) for key in issue_keys)
workflow_pass = False
if workflow_type in workflow_requirements:
    checklist_name, issue_key = workflow_requirements[workflow_type]
    workflow_pass = (
        all(value is True for value in review.get(checklist_name, {}).values())
        and not review.get(issue_key)
    )
has_plan = bool(str(review.get("accepted_plan_markdown", "")).strip())
print("pass" if review.get("decision") == "pass" and required_true and no_issues and workflow_pass and has_plan else "fail")
PY
)"

if [[ "$decision" != "pass" ]]; then
  reason="$(
    python3 - "$review_file" <<'PY'
import json
import sys
from pathlib import Path

review = json.loads(Path(sys.argv[1]).read_text())
parts = ["Codex rejected the plan."]
workflow_type = review.get("workflow_type", "unknown")
parts.append(f"Workflow: {workflow_type}.")
summary = review.get("summary", "").strip()
if summary:
    parts.append(summary)
checklist = review.get("checklist", {})
false_checks = [key for key, value in checklist.items() if value is not True]
if false_checks:
    parts.append("Failed checklist: " + "; ".join(false_checks))
workflow_requirements = {
    "server_api": ("server_api_checklist", "server_api_issues", "Server/API"),
    "persistence": ("persistence_checklist", "persistence_issues", "Persistence"),
    "fhir_core": ("fhir_core_checklist", "fhir_core_issues", "FHIR core"),
    "subscriptions": ("subscriptions_checklist", "subscriptions_issues", "Subscriptions"),
    "tooling_config": ("tooling_config_checklist", "tooling_config_issues", "Tooling/config"),
}
if workflow_type in workflow_requirements:
    checklist_name, issue_key, label = workflow_requirements[workflow_type]
    workflow_checklist = review.get(checklist_name, {})
    failed = [key for key, value in workflow_checklist.items() if value is not True]
    if failed:
        parts.append(f"Failed {label} checklist: " + "; ".join(failed))
    workflow_issues = [str(item).strip() for item in review.get(issue_key, []) if str(item).strip()]
    if workflow_issues:
        parts.append(f"{label} issues: " + "; ".join(workflow_issues))
for label, key in [
    ("Missing steps", "missing_steps"),
    ("Unresolved decisions", "unresolved_decisions"),
    ("Missing tests/verification", "missing_tests_or_verification"),
    ("Violated constraints", "violated_constraints"),
    ("Unclear acceptance criteria", "unclear_acceptance_criteria"),
]:
    items = [str(item).strip() for item in review.get(key, []) if str(item).strip()]
    if items:
        parts.append(f"{label}: " + "; ".join(items))
changes = [str(item).strip() for item in review.get("required_changes", []) if str(item).strip()]
if changes:
    parts.append("Required changes: " + "; ".join(changes))
print(" ".join(parts))
PY
  )"
  gate_log_line "$gate_main_log" "$reason"
  deny "$reason"
fi

python3 - "$review_file" "$accepted_plan" <<'PY'
import json
import sys
from pathlib import Path

review = json.loads(Path(sys.argv[1]).read_text())
Path(sys.argv[2]).write_text(review["accepted_plan_markdown"].strip() + "\n")
PY

# Hand off to codex-final-plan-review.sh (Stop hook): skip final reviews until
# post-plan implementation work is detected, and reset the stop-block counter.
printf 'Plan accepted by ExitPlanMode; skip Stop reviews until post-plan work is detected.\n' > "$skip_next_stop_review"
rm -f "$session_dir/stop-block-count" "$attempt_counter"
gate_log_event "plan-gate" "pass" "workflow_type=${GATE_WF:-}" "attempt=${GATE_ATTEMPT:-}"
gate_log_line "$gate_main_log" "pass session=$session_id workflow_type=${GATE_WF:-} attempt=${GATE_ATTEMPT:-}"
exit 0
