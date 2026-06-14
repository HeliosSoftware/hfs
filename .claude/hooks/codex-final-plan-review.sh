#!/usr/bin/env bash
#
# codex-final-plan-review.sh - HFS implementation gate.
#
# Runs as a Stop hook. After Claude finishes working on a plan that was accepted
# by codex-verify-plan.sh, Codex compares the accepted plan against the actual
# implementation (git diff + test evidence) and blocks the stop until the
# implementation is complete, correct, and accurately summarized.
#
# Fail-closed: if neither Codex nor the safe Claude fallback can run, the stop
# is blocked (up to the retry cap). Set CODEX_PLAN_GATE_DISABLE=1 to skip the
# gate entirely (escape hatch). CODEX_PLAN_GATE_MAX_BLOCKS caps consecutive
# blocks to avoid infinite loops. Set CODEX_PLAN_GATE_CLAUDE_FALLBACK=0 to
# require Codex.
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
schema="${CODEX_FINAL_REVIEW_SCHEMA:-$script_dir/../schemas/codex-final-review.schema.json}"
policy_state="$project_dir/.claude/state/policy.env"
max_blocks="${CODEX_PLAN_GATE_MAX_BLOCKS:-10}"

# Escape hatch: never block stopping when explicitly disabled.
if [[ -n "${CODEX_PLAN_GATE_DISABLE:-}" ]]; then
  exit 0
fi

mkdir -p "$debug_dir" "$state_root"

# Shared observability helpers (timestamped logs, JSONL events, attempt snapshots).
# shellcheck source=/dev/null
source "$script_dir/codex-gate-log.sh"

gate_main_log="$debug_dir/codex-final-plan-review.log"
GATE_WF=""
GATE_ATTEMPT=""

payload="$(cat)"
printf '%s\n' "$payload" > "$debug_dir/stop-codex-final-review-last.json"

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

block_stop() {
  local reason="$1"
  local blocks=""
  [[ -f "${block_count_file:-}" ]] && blocks="$(tr -dc '0-9' < "$block_count_file" 2>/dev/null || true)/${max_blocks:-}"
  gate_log_event "final-review" "block" "workflow_type=${GATE_WF:-}" "attempt=${GATE_ATTEMPT:-}" "blocks=$blocks" "reason=${reason:0:300}"
  python3 - "$reason" <<'PY'
import json
import sys

print(json.dumps({
    "decision": "block",
    "reason": sys.argv[1],
}))
PY
  exit 0
}

post_plan_work_detected() {
  [[ -f "$accepted_plan" && -f "$transcript_path" ]] || return 1
  python3 - "$accepted_plan" "$transcript_path" <<'PY'
import json
import sys
from datetime import datetime
from pathlib import Path

try:
    accepted_at = Path(sys.argv[1]).stat().st_mtime
except OSError:
    sys.exit(1)

work_tools = {
    "Edit",
    "MultiEdit",
    "NotebookEdit",
    "TaskCreate",
    "Write",
}

def timestamp_to_epoch(value):
    if not value:
        return 0
    try:
        return datetime.fromisoformat(str(value).replace("Z", "+00:00")).timestamp()
    except ValueError:
        return 0

try:
    lines = Path(sys.argv[2]).read_text(errors="replace").splitlines()
except OSError:
    sys.exit(1)

for line in lines:
    try:
        entry = json.loads(line)
    except json.JSONDecodeError:
        continue

    if timestamp_to_epoch(entry.get("timestamp")) <= accepted_at:
        continue

    message = entry.get("message")
    if not isinstance(message, dict):
        continue

    content = message.get("content")
    if not isinstance(content, list):
        continue

    for item in content:
        if not isinstance(item, dict) or item.get("type") != "tool_use":
            continue
        name = str(item.get("name", ""))
        if name in work_tools:
            sys.exit(0)

sys.exit(1)
PY
}

session_id="$(json_get session_id)"
transcript_path="$(json_get transcript_path)"
last_assistant_message="$(json_get last_assistant_message)"
stop_hook_active="$(json_get stop_hook_active)"
safe_session_id="$(printf '%s' "${session_id:-unknown}" | tr -c 'A-Za-z0-9_.-' '_')"
session_dir="$state_root/$safe_session_id"
accepted_plan="$session_dir/accepted-plan.md"
skip_next_stop_review="$session_dir/skip-next-stop-review"
block_count_file="$session_dir/stop-block-count"
attempt_counter="$session_dir/final-attempt-count"

gate_prune_sessions "$state_root"
gate_log_line "$gate_main_log" "session=$session_id stop_hook_active=$stop_hook_active"

# Nothing to review unless the plan gate accepted a plan this session.
if [[ ! -f "$accepted_plan" ]]; then
  exit 0
fi

# Wait until the accepted plan has actually been worked on before reviewing.
if [[ -f "$skip_next_stop_review" ]]; then
  if post_plan_work_detected; then
    rm -f "$skip_next_stop_review"
    gate_log_line "$gate_main_log" "post-plan work detected; running final review session=$session_id"
  else
    gate_log_line "$gate_main_log" "skip stop review until post-plan work is detected session=$session_id"
    gate_log_event "final-review" "skip" "reason=awaiting post-plan work"
    exit 0
  fi
fi

current_blocks=0
if [[ -f "$block_count_file" ]]; then
  current_blocks="$(tr -dc '0-9' < "$block_count_file" || true)"
  current_blocks="${current_blocks:-0}"
fi

# Retry cap: stop blocking after too many consecutive failures to avoid loops.
if [[ "$current_blocks" -ge "$max_blocks" ]]; then
  gate_log_event "final-review" "cap" "blocks=$current_blocks/$max_blocks"
  gate_log_line "$gate_main_log" "allow after retry cap session=$session_id blocks=$current_blocks"
  exit 0
fi

if [[ ! -f "$schema" ]]; then
  next_blocks=$((current_blocks + 1))
  printf '%s\n' "$next_blocks" > "$block_count_file"
  block_stop "Codex final verifier schema is missing: $schema. Restore the verifier schema, then provide a final summary."
fi

prompt_file="$session_dir/final-review-prompt.md"
review_file="$session_dir/final-review.json"
codex_stdout="$session_dir/final-codex.stdout"
codex_stderr="$session_dir/final-codex.stderr"
claude_stdout="$session_dir/final-claude.stdout"
claude_stderr="$session_dir/final-claude.stderr"
reviewer_engine="codex"
codex_available=1

if [[ -z "$codex_bin" || ! -x "$codex_bin" ]]; then
  codex_available=0
  if [[ "$claude_fallback_enabled" != "0" && -n "$claude_bin" && -x "$claude_bin" ]]; then
    reviewer_engine="claude"
    gate_log_event "final-review" "fallback" "reason=codex-unavailable" "fallback=claude"
    gate_log_line "$gate_main_log" "codex unavailable; falling back to claude safe-mode verifier"
  else
    next_blocks=$((current_blocks + 1))
    printf '%s\n' "$next_blocks" > "$block_count_file"
    block_stop "Codex final verifier could not run because no executable Codex binary was found (CODEX_BIN='${CODEX_BIN:-}'), and the safe Claude fallback is unavailable or disabled (CLAUDE_BIN='${CLAUDE_BIN:-}', CODEX_PLAN_GATE_CLAUDE_FALLBACK=$claude_fallback_enabled). Fix verifier availability or set CODEX_PLAN_GATE_DISABLE=1, then provide a final summary."
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
    next_blocks=$((current_blocks + 1))
    printf '%s\n' "$next_blocks" > "$block_count_file"
    block_stop "Codex final verifier requires codex exec --output-schema, but this Codex CLI does not advertise it. Update Codex or set CODEX_BIN to a compatible binary, then provide a final summary."
  fi

  if ! printf '%s\n' "$codex_exec_help" | grep -q -- '--output-last-message'; then
    next_blocks=$((current_blocks + 1))
    printf '%s\n' "$next_blocks" > "$block_count_file"
    block_stop "Codex final verifier requires codex exec --output-last-message/-o, but this Codex CLI does not advertise it. Update Codex or set CODEX_BIN to a compatible binary, then provide a final summary."
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
tail = "\n".join(lines[-350:])
print(tail[-250000:])
PY
)"

# Fall back to the last assistant text from the transcript if the payload did
# not carry last_assistant_message.
if ! [[ "$last_assistant_message" =~ [^[:space:]] ]]; then
  last_assistant_message="$(
    TRANSCRIPT_PATH="$transcript_path" python3 <<'PY'
import json
import os
from pathlib import Path

path_text = os.environ.get("TRANSCRIPT_PATH", "")
if not path_text:
    raise SystemExit(0)
path = Path(path_text).expanduser()
if not path.exists():
    raise SystemExit(0)

last_text = ""
for line in path.read_text(errors="replace").splitlines():
    try:
        entry = json.loads(line)
    except json.JSONDecodeError:
        continue
    message = entry.get("message")
    if not isinstance(message, dict) or message.get("role") != "assistant":
        continue
    content = message.get("content")
    if isinstance(content, str):
        last_text = content
    elif isinstance(content, list):
        parts = [b.get("text", "") for b in content if isinstance(b, dict) and b.get("type") == "text"]
        if any(parts):
            last_text = "\n".join(p for p in parts if p)
print(last_text)
PY
  )"
fi

git_status="$(git -C "$project_dir" status --short 2>&1 || true)"
git_diff_stat="$(git -C "$project_dir" diff --stat HEAD -- 2>&1 || true)"
git_diff="$(git -C "$project_dir" diff --no-ext-diff --unified=80 HEAD -- 2>&1 || true)"
untracked_files="$(git -C "$project_dir" ls-files --others --exclude-standard 2>&1 || true)"
policy_state_text="$(cat "$policy_state" 2>/dev/null || printf '(no hfs-policy state recorded)')"
hook_log_tail="$(
  for log in "$debug_dir"/*.log; do
    [[ -f "$log" ]] || continue
    printf '\n== %s ==\n' "$(basename "$log")"
    tail -n 80 "$log"
  done
)"

# Build the scan blob first, then grep a here-string: a `printf ... | grep -q`
# pipe can return non-zero under `set -o pipefail` when grep short-circuits and
# printf gets SIGPIPE, which would silently skip this guard on large diffs.
url_scan_blob="$(printf '%s\n%s\n%s\n%s\n%s\n%s\n' "$(cat "$accepted_plan")" "$payload" "$last_assistant_message" "$git_diff" "$hook_log_tail" "$transcript_tail")"
if [[ "$search_enabled" != "1" ]] && grep -Eqi 'https?://' <<< "$url_scan_blob"; then
  next_blocks=$((current_blocks + 1))
  printf '%s\n' "$next_blocks" > "$block_count_file"
  block_stop "Codex final verifier cannot inspect referenced HTTP/HTTPS URLs because this Codex CLI does not advertise --search. Update Codex or inline the required external context, then provide an updated final summary."
fi

cat > "$prompt_file" <<EOF
You are Codex acting as an independent final implementation gate for Claude Code working on the Helios HFS repository. HFS is a Rust workspace implementing a multi-version FHIR server (R4/R4B/R5/R6 via feature flags), a FHIRPath engine, SQL-on-FHIR, a terminology server, and a tenant-first polyglot persistence layer. There is no frontend. Act as a senior HFS backend architect with technical product review discipline: verify implementation completeness, plan fidelity, test evidence, user constraints, and final reporting accuracy.

Return only JSON matching the provided output schema. Do not edit files. Do not run mutating commands.

If the accepted plan, transcript, or implementation evidence references HTTP/HTTPS URLs, documentation pages, issues, PRs, specs, or other external sources that materially affect correctness, inspect them before deciding. Fail the final review if required external context is inaccessible, ignored, or not reflected accurately in the implementation or final summary.

Compare the accepted plan against the final implementation evidence (git diff against HEAD, untracked files, transcript, recorded check state). Decide:
- "pass" only if every universal checklist field is true, every applicable domain checklist field is true, every issue array is empty, and the implementation fully satisfies the accepted plan or explicitly justifies any no-longer-needed item.
- "fail" if plan items are missing, constraints were violated, required checks/tests are missing, the final summary is inaccurate, or unrelated risky changes were introduced.

CRITICAL - remote CI execution is OUT OF SCOPE: This is a local, read-only review with no access to GitHub Actions results. NEVER fail the implementation, and never populate any issue array, because a GitHub CI job has not run, has not passed, cannot run locally, or still needs to be added or enabled remotely before it can run. Verification that legitimately runs only on GitHub runners (Inferno suites, conformance/smoke suites, full --workspace --all-features, testcontainers-backed integration) is SATISFIED when the change is wired to or covered by the appropriate workflow, when the workflow file is added or updated in the diff, or when the plan explicitly defers it to a named workflow - regardless of whether that workflow has executed yet. A plan item whose completion is inherently remote (e.g. "enable/trigger workflow X on GitHub") must be treated as addressed once its local artifacts (workflow file, wiring, config) are present in the diff; do not mark it missing for lack of a remote run. Only checks that CAN run locally are required: cargo fmt --all, the CI-style cargo clippy, and an affected cargo test (plus pysof's local tests when applicable). Hold the implementation to those local checks, and to correct CI wiring/coverage - never to the existence or result of a remote CI run.

First classify workflow_type exactly, using the accepted plan and final diff:
- server_api: REST/Axum work in helios-rest - handlers, routes, extractors, middleware, multi-tenancy routing, request/response and compression behavior, FHIR OperationOutcome error handling, endpoint tests. Authentication/authorization (helios-auth: SMART/OAuth2 JWT, JWKS, scopes, JTI cache) and audit logging (helios-audit: AuditEvent/BALP middleware and sinks) are request-path concerns and classify here; surface storage-sink aspects of audit in persistence_issues when relevant.
- persistence: storage/persistence work in helios-persistence - backends (SQLite/PostgreSQL/Elasticsearch/MongoDB), composite storage, search registry, tenant isolation, trait hierarchy, capability advertisement, integration tests.
- fhir_core: FHIR data models (helios-fhir), serialization (helios-serde), FHIRPath (helios-fhirpath), SQL-on-FHIR (helios-sof) - version-gated types, enum wrappers, parser/evaluator/transformation logic, conformance tests, code generation. CDS Hooks protocol types and the CdsHooksService trait (helios-cds-hooks, a standalone library) classify here unless a server endpoint is being wired (then server_api).
- subscriptions: FHIR Subscriptions engine work in helios-subscriptions - SubscriptionTopic/event matching, delivery channels (rest-hook, websocket, email, messaging), notification building, retry/backoff, endpoint-safety policy.
- tooling_config: hooks, skills, CLAUDE.md, CI/config, scripts, settings.json, Docker/release, or developer workflow configuration.
Pick the single dominant domain. For coupled work, pick the one carrying the most risk and surface the other domain's gaps in its issue array.

Universal checklist semantics (the "checklist" object - all must be true to pass):
- all_plan_items_addressed: every accepted plan item is implemented or explicitly justified as unnecessary. A plan item that can only be completed remotely (e.g. enabling or triggering a GitHub Actions workflow) counts as addressed once its local artifacts are present in the diff; do not list it in missing_items for lack of a remote CI run.
- implementation_matches_plan: the diff and transcript evidence match the accepted plan's intended approach.
- constraints_respected: user constraints and accepted-plan constraints are followed.
- no_unrelated_risky_changes: the diff contains no unrelated or risky changes outside the plan.
- tests_or_verification_performed: concrete test/check/manual verification evidence is present - either local runs (fmt/clippy/affected cargo test) OR the change is wired to/covered by a named CI workflow. CI-deferred verification counts as performed even if the workflow has not run yet or must be enabled remotely first; the absence of a remote CI run is never a deficiency here.
- final_summary_accurate: Claude's final message accurately reports changes, checks, and residual failures.
- required_checks_performed: the mandatory developer gate ran for the code changes - cargo fmt --all, CI clippy (cargo clippy --all-targets --all-features -- -D warnings), and an affected cargo test; plus, when crates/pysof changed, cd crates/pysof && uv run pytest python-tests/ -v and cd crates/pysof && cargo test. Judge primarily from the recorded hfs-policy state below (FMT_OK / CLIPPY_OK / TEST_OK, and PYSOF_* when pysof changed) and corroborating transcript evidence. True only if such checks are genuinely unnecessary (e.g. pure docs/config) and that is justified.
- ci_coverage_addressed: HFS does much of its testing on GitHub Actions runners. ci.yml (every PR to main / push to main/develop) runs lint, test-rust (cargo test --workspace --all-features on self-hosted runners with testcontainers), coverage, security, test-python, test-fhirpath, hts-conformance, build. ci-extended.yml runs the full suite nightly and as a release gate. Heavier suites run as their own workflows: inferno-us-core, inferno-bulk-data, inferno-bulk-submit-data, inferno-subscription, audit-events, bulk-export-smoke, bulk-submit-smoke, subscriptions-channels, subscriptions-smoke, hts-ig-conformance, hts-benchmark. True when the change is covered by an existing workflow, a workflow was added/updated in the diff, the plan defers it to a named workflow, or CI coverage is genuinely unaffected - in ALL of these cases it is true even if the workflow has not executed yet or must first be added/enabled remotely. Do NOT fail because heavy integration/conformance/full-all-features tests run in CI rather than locally, because a CI job has not run, or because a new workflow cannot be exercised locally. Only fail if the change lands in a real CI gap with no workflow validating it AND the plan neither adds coverage nor justifies the gap.
- fhir_version_impact_handled: FHIR version gating is implemented across the affected R4/R4B/R5/R6 features as planned (version-agnostic enum wrappers and feature flags consistent), or FHIR versioning genuinely does not apply.
- crate_boundaries_correct: changes landed in the correct workspace crate(s).
- reuse_over_duplication: the diff reuses existing workspace traits, enums, structs, functions, and helpers instead of duplicating or copy-pasting logic that already exists. Verify against the ACTUAL codebase using your read-only repo access: before accepting a newly added struct/enum/trait/function, search the workspace for an existing equivalent it should have reused or extended (e.g. version-agnostic enum wrappers like SofViewDefinition, the ResourceStorage/VersionedStorage trait hierarchy and CapabilityProvider, shared helios-serde-support / helios-fhirpath-support utilities, helios-auth JWKS/JTI caches and scope/policy types, helios-audit AuditSink/AuditEventBuilder/middleware, the helios-subscriptions SubscriptionEngine/channels/evaluator, and helios-cds-hooks protocol types). Flag parallel re-implementations and copy-pasted blocks. Any genuinely new abstraction must be justified. True when the change is small/isolated enough that no meaningful reuse opportunity applies. Record duplication/reuse findings in violations and required_next_steps.

Issue arrays are authoritative:
- Put confirmed-implemented accepted-plan items in completed_items.
- Put every unfinished accepted-plan item in missing_items.
- Put every constraint or plan violation in violations.
- Put every missing or weak check in missing_tests_or_verification.
- Put every inaccurate or omitted final-summary point in inaccurate_summary_points.
- Put concrete evidence of checks/tests performed (commands run, covering CI workflows, observed results) in test_evidence.
- Put concrete corrective actions in required_next_steps.

Domain-specific checklist enforcement:
- server_api: server_api_checklist must be fully true and server_api_issues must be empty.
- persistence: persistence_checklist must be fully true and persistence_issues must be empty.
- fhir_core: fhir_core_checklist must be fully true and fhir_core_issues must be empty.
- subscriptions: subscriptions_checklist must be fully true and subscriptions_issues must be empty.
- tooling_config: tooling_config_checklist must be fully true and tooling_config_issues must be empty.
- Fill non-applicable domain checklists with true values and leave their issue arrays empty unless they reveal a real cross-cutting concern.

Domain checklist semantics (all framed as "implemented/evidenced in the diff"):
- server_api_checklist: handler_and_route_registration_implemented (handler in crates/rest/src/handlers/, route in crates/rest/src/routes.rs); request_response_contract_correct (extractors, content negotiation, compression, payload shapes); operation_outcome_error_paths_handled (FHIR OperationOutcome + correct HTTP status codes); endpoint_tests_evidenced (local run or named covering CI workflow).
- persistence_checklist: tenant_context_first_and_isolation_preserved (TenantContext first argument; tenant boundaries enforced at the query level); trait_hierarchy_and_capabilities_correct (ResourceStorage -> VersionedStorage / SearchProvider / TransactionProvider; new backends advertise via CapabilityProvider); backend_compatibility_and_migration_handled (SQLite/PostgreSQL/Elasticsearch/MongoDB/composite); persistence_integration_tests_evidenced (testcontainers locally or named covering CI workflow).
- fhir_core_checklist: fhir_version_coverage_implemented (R4/R4B/R5/R6 feature flags, or justified single-version scope); version_agnostic_abstraction_preserved (enum wrappers like SofViewDefinition and traits); serde_or_codegen_consistent (helios-serde, fhir-gen regeneration if schema-driven); fhir_conformance_tests_evidenced (FHIRPath/SOF suites or spec test data, local or covering CI workflow).
- subscriptions_checklist: topic_and_event_matching_implemented (SubscriptionTopic criteria and resource-event evaluation/filter matching); channel_delivery_implemented (rest-hook/websocket/email/messaging channels and notification building reuse SubscriptionEngine/channels rather than ad-hoc delivery); endpoint_safety_and_retry_handled (private/loopback policy via HFS_SUBSCRIPTION_ALLOW_PRIVATE_ENDPOINTS, plus delivery retry/backoff); subscription_tests_evidenced (local tests or a named/added inferno-subscription / subscriptions-channels / subscriptions-smoke workflow, which need not have executed remotely yet).
- tooling_config_checklist: config_or_docs_changes_complete (settings.json, hooks, skills, CLAUDE.md, CI); hook_or_workflow_compatibility_preserved (does not break the existing hfs-policy.sh stop-gate; jq/python3 availability); apply_or_reload_evidenced (e.g. restart session to reload settings); verification_command_evidenced.
For the domain "evidenced" fields and tests_or_verification_performed, naming (or adding/updating in the diff) the GitHub Actions workflow that validates the change counts as evidence; tests do not have to run on the developer machine, and the workflow does not have to have executed remotely yet. Never put a CI-only or not-yet-run workflow into a domain issue array.

Domain issue arrays:
- server_api_issues: missing or broken API/interface contract, handler/route registration, error/permission path, or endpoint test evidence.
- persistence_issues: missing tenant isolation, trait/capability correctness, backend compatibility/migration, or integration test evidence.
- fhir_core_issues: missing FHIR version coverage, version-agnostic abstraction, serde/codegen consistency, or conformance test evidence.
- subscriptions_issues: missing or broken topic/event matching, channel delivery, endpoint-safety/retry handling, or subscription test evidence.
- tooling_config_issues: missing config/docs change, hook/workflow compatibility, apply/reload behavior, or verification evidence.

Accepted plan:
\`\`\`markdown
$(cat "$accepted_plan")
\`\`\`

Stop hook payload:
\`\`\`json
$payload
\`\`\`

Last assistant message:
\`\`\`markdown
$last_assistant_message
\`\`\`

Recorded hfs-policy check state (.claude/state/policy.env):
\`\`\`
$policy_state_text
\`\`\`

Git status:
\`\`\`
$git_status
\`\`\`

Untracked files:
\`\`\`
$untracked_files
\`\`\`

Git diff stat:
\`\`\`
$git_diff_stat
\`\`\`

Git diff against HEAD:
\`\`\`diff
$git_diff
\`\`\`

Hook log tail:
\`\`\`
$hook_log_tail
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
  next_blocks=$((current_blocks + 1))
  printf '%s\n' "$next_blocks" > "$block_count_file"
  verifier_stderr="$codex_stderr"
  [[ "$reviewer_engine" == "claude" ]] && verifier_stderr="$claude_stderr"
  reason="${reviewer_engine} final verifier failed closed. Fix verifier execution or address any implementation gaps, then provide a final summary. stderr: $(tail -n 20 "$verifier_stderr" | tr '\n' ' ')"
  gate_log_line "$gate_main_log" "$reason"
  block_stop "$reason"
fi

if ! python3 - "$review_file" <<'PY'
import json
import sys
from pathlib import Path

UNIVERSAL_CHECKS = [
    "all_plan_items_addressed",
    "implementation_matches_plan",
    "constraints_respected",
    "no_unrelated_risky_changes",
    "tests_or_verification_performed",
    "final_summary_accurate",
    "required_checks_performed",
    "ci_coverage_addressed",
    "fhir_version_impact_handled",
    "crate_boundaries_correct",
    "reuse_over_duplication",
]
UNIVERSAL_LISTS = [
    "completed_items",
    "missing_items",
    "violations",
    "missing_tests_or_verification",
    "inaccurate_summary_points",
    "test_evidence",
    "required_next_steps",
]
DOMAIN_CHECKLISTS = {
    "server_api_checklist": [
        "handler_and_route_registration_implemented",
        "request_response_contract_correct",
        "operation_outcome_error_paths_handled",
        "endpoint_tests_evidenced",
    ],
    "persistence_checklist": [
        "tenant_context_first_and_isolation_preserved",
        "trait_hierarchy_and_capabilities_correct",
        "backend_compatibility_and_migration_handled",
        "persistence_integration_tests_evidenced",
    ],
    "fhir_core_checklist": [
        "fhir_version_coverage_implemented",
        "version_agnostic_abstraction_preserved",
        "serde_or_codegen_consistent",
        "fhir_conformance_tests_evidenced",
    ],
    "subscriptions_checklist": [
        "topic_and_event_matching_implemented",
        "channel_delivery_implemented",
        "endpoint_safety_and_retry_handled",
        "subscription_tests_evidenced",
    ],
    "tooling_config_checklist": [
        "config_or_docs_changes_complete",
        "hook_or_workflow_compatibility_preserved",
        "apply_or_reload_evidenced",
        "verification_command_evidenced",
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
    if review.get("decision") not in {"pass", "fail"}:
        raise ValueError("invalid decision")
    if review.get("workflow_type") not in {"server_api", "persistence", "fhir_core", "subscriptions", "tooling_config"}:
        raise ValueError("invalid workflow_type")
    checklist = review.get("checklist")
    if not isinstance(checklist, dict):
        raise ValueError("missing checklist")
    for key in UNIVERSAL_CHECKS:
        if not isinstance(checklist.get(key), bool):
            raise ValueError(f"invalid checklist.{key}")
    for key in UNIVERSAL_LISTS:
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
except Exception:
    raise SystemExit(1)
PY
then
  next_blocks=$((current_blocks + 1))
  printf '%s\n' "$next_blocks" > "$block_count_file"
  block_stop "${reviewer_engine} final verifier returned invalid JSON. Continue by fixing verifier output or implementation evidence, then provide a final summary."
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
    "all_plan_items_addressed",
    "implementation_matches_plan",
    "constraints_respected",
    "no_unrelated_risky_changes",
    "tests_or_verification_performed",
    "final_summary_accurate",
    "required_checks_performed",
    "ci_coverage_addressed",
    "fhir_version_impact_handled",
    "crate_boundaries_correct",
    "reuse_over_duplication",
]
issue_keys = [
    "missing_items",
    "violations",
    "missing_tests_or_verification",
    "inaccurate_summary_points",
    "required_next_steps",
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
print("pass" if review.get("decision") == "pass" and required_true and no_issues and workflow_pass else "fail")
PY
)"

if [[ "$decision" != "pass" ]]; then
  next_blocks=$((current_blocks + 1))
  printf '%s\n' "$next_blocks" > "$block_count_file"
  reason="$(
    python3 - "$review_file" "$next_blocks" "$max_blocks" <<'PY'
import json
import sys
from pathlib import Path

review = json.loads(Path(sys.argv[1]).read_text())
next_blocks = sys.argv[2]
max_blocks = sys.argv[3]
parts = [f"Codex final review failed ({next_blocks}/{max_blocks} stop blocks used)."]
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
    ("Missing items", "missing_items"),
    ("Violations", "violations"),
    ("Missing tests/verification", "missing_tests_or_verification"),
    ("Inaccurate summary points", "inaccurate_summary_points"),
    ("Required next steps", "required_next_steps"),
]:
    items = [str(item).strip() for item in review.get(key, []) if str(item).strip()]
    if items:
        parts.append(f"{label}: " + "; ".join(items))
parts.append("Continue implementation, fix these issues, rerun needed checks, and provide an updated final summary.")
print(" ".join(parts))
PY
  )"
  gate_log_line "$gate_main_log" "$reason"
  block_stop "$reason"
fi

# Pass: clear the gate state so the next plan starts fresh.
rm -f "$accepted_plan" "$skip_next_stop_review" "$block_count_file" "$attempt_counter"
gate_log_event "final-review" "pass" "workflow_type=${GATE_WF:-}" "attempt=${GATE_ATTEMPT:-}"
gate_log_line "$gate_main_log" "pass session=$session_id workflow_type=${GATE_WF:-} attempt=${GATE_ATTEMPT:-}"
exit 0
