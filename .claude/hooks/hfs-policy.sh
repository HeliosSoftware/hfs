#!/usr/bin/env bash
set -u

ACTION="${1:-}"
INPUT="$(cat)"

if ! command -v jq >/dev/null 2>&1; then
  exit 0
fi

PROJECT_DIR="${CLAUDE_PROJECT_DIR:-}"
if [ -z "$PROJECT_DIR" ]; then
  PROJECT_DIR="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
fi

STATE_DIR="$PROJECT_DIR/.claude/state"
STATE_FILE="$STATE_DIR/policy.env"

CODE_DIRTY=0
PYSOF_DIRTY=0
FMT_OK=0
CLIPPY_OK=0
TEST_OK=0
PYSOF_PYTEST_OK=0
PYSOF_RUST_TEST_OK=0
LAST_EDITED_FILE=""
LAST_UPDATED_AT=""

load_state() {
  if [ -f "$STATE_FILE" ]; then
    # The state file is written only by this script as shell-escaped assignments.
    # shellcheck disable=SC1090
    . "$STATE_FILE"
  fi
}

save_state() {
  mkdir -p "$STATE_DIR"
  local tmp="$STATE_FILE.$$"
  {
    printf 'CODE_DIRTY=%q\n' "$CODE_DIRTY"
    printf 'PYSOF_DIRTY=%q\n' "$PYSOF_DIRTY"
    printf 'FMT_OK=%q\n' "$FMT_OK"
    printf 'CLIPPY_OK=%q\n' "$CLIPPY_OK"
    printf 'TEST_OK=%q\n' "$TEST_OK"
    printf 'PYSOF_PYTEST_OK=%q\n' "$PYSOF_PYTEST_OK"
    printf 'PYSOF_RUST_TEST_OK=%q\n' "$PYSOF_RUST_TEST_OK"
    printf 'LAST_EDITED_FILE=%q\n' "$LAST_EDITED_FILE"
    printf 'LAST_UPDATED_AT=%q\n' "$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
  } > "$tmp"
  mv "$tmp" "$STATE_FILE"
}

json_context() {
  local event="$1"
  local msg="$2"
  jq -nc --arg event "$event" --arg msg "$msg" \
    '{hookSpecificOutput: {hookEventName: $event, additionalContext: $msg}}'
}

json_ask() {
  local reason="$1"
  jq -nc --arg reason "$reason" \
    '{hookSpecificOutput: {hookEventName: "PreToolUse", permissionDecision: "ask", permissionDecisionReason: $reason}}'
}

json_deny() {
  local reason="$1"
  jq -nc --arg reason "$reason" \
    '{hookSpecificOutput: {hookEventName: "PreToolUse", permissionDecision: "deny", permissionDecisionReason: $reason}}'
}

json_stop_block() {
  local reason="$1"
  jq -nc --arg reason "$reason" \
    '{decision: "block", reason: $reason}'
}

is_code_path() {
  local path="$1"
  case "$path" in
    *.rs|Cargo.toml|Cargo.lock|build.rs|*/build.rs|rust-toolchain|rust-toolchain.toml)
      return 0
      ;;
    crates/pysof/*.py|crates/pysof/**/*.py|crates/pysof/pyproject.toml|crates/pysof/uv.lock)
      return 0
      ;;
    Dockerfile|docker/*|docker/**/*)
      return 0
      ;;
  esac
  return 1
}

is_pysof_path() {
  local path="$1"
  case "$path" in
    crates/pysof/*|crates/pysof/**/*)
      return 0
      ;;
  esac
  return 1
}

changed_code_paths() {
  git -C "$PROJECT_DIR" status --porcelain --untracked-files=all 2>/dev/null |
    sed -e 's/^...//' |
    while IFS= read -r path; do
      if is_code_path "$path"; then
        printf '%s\n' "$path"
      fi
    done
}

staged_files() {
  git -C "$PROJECT_DIR" diff --cached --name-only 2>/dev/null
}

staged_has_compiled_code() {
  local path
  while IFS= read -r path; do
    [ -z "$path" ] && continue
    if is_code_path "$path"; then
      return 0
    fi
  done < <(staged_files)
  return 1
}

staged_has_files() {
  [ -n "$(staged_files)" ]
}

command_text() {
  jq -r '.tool_input.command // ""' <<<"$INPUT"
}

edited_path() {
  jq -r '.tool_input.file_path // .tool_response.filePath // ""' <<<"$INPUT"
}

case "$ACTION" in
  session-context)
    json_context "SessionStart" \
      "HFS project hooks are active. After code edits, completion is gated until cargo fmt --all, CI-style cargo clippy --all-targets --all-features -- -D warnings ..., and an affected cargo test have been recorded. Full FHIR-version builds and FHIR generation commands ask before running because they can exceed 10 minutes."
    ;;

  prompt-context)
    prompt="$(jq -r '.prompt // ""' <<<"$INPUT" | tr '[:upper:]' '[:lower:]')"
    notes=()

    if [[ "$prompt" == *"fhirpath"* && "$prompt" == *"function"* ]]; then
      notes+=("FHIRPath function changes normally require implementation under crates/fhirpath/src/, parser updates when syntax changes, tests, and the README feature matrix update.")
    fi

    if [[ "$prompt" == *"rest endpoint"* || "$prompt" == *"api endpoint"* ]]; then
      notes+=("New REST endpoints should add the handler in crates/rest/src/handlers/, register the route in crates/rest/src/routes.rs, and include endpoint tests.")
    fi

    if [[ "$prompt" == *"storage backend"* || "$prompt" == *"persistence"* ]]; then
      notes+=("Persistence changes must preserve tenant-first behavior: storage operations take TenantContext first and enforce tenant boundaries at the query level. New backends should advertise capabilities through CapabilityProvider.")
    fi

    if [[ "$prompt" == *"viewdefinition"* || "$prompt" == *"sql-on-fhir"* || "$prompt" == *" sof "* ]]; then
      notes+=("ViewDefinition work should keep version-specific parsing wrapped behind SofViewDefinition and use run_view_definition() for transformations.")
    fi

    if [[ "$prompt" == *"pysof"* || "$prompt" == *"python binding"* || "$prompt" == *"python bindings"* ]]; then
      notes+=("pysof is excluded from default workspace builds; relevant changes should run cd crates/pysof && uv run pytest python-tests/ -v and cd crates/pysof && cargo test.")
    fi

    if [ "${#notes[@]}" -gt 0 ]; then
      msg="$(printf '%s\n' "${notes[@]}")"
      json_context "UserPromptSubmit" "$msg"
    fi
    ;;

  guard-bash)
    cmd="$(command_text)"

    if [[ "$cmd" == git\ commit* ]] && staged_has_files && ! staged_has_compiled_code && [[ "$cmd" != *"[skip ci]"* ]]; then
      json_deny "Docs/non-compiled-only commits in this repository must include [skip ci] in the commit message."
      exit 0
    fi

    if [[ "$cmd" == *"fhir_gen --all"* || "$cmd" == *"cargo build -p helios-fhir-gen"* ]]; then
      json_ask "FHIR generation can take 5-10+ minutes and may download R6 specs. Continue?"
      exit 0
    fi

    if [[ "$cmd" == *"cargo build --features R4,R4B,R5,R6"* || "$cmd" == *"cargo test --features R4,R4B,R5,R6"* ]]; then
      json_ask "Full multi-version workspace builds/tests can exceed 10 minutes. Continue?"
      exit 0
    fi

    if [[ "$cmd" == cargo\ release* && "$cmd" == *"--execute"* ]]; then
      json_ask "This runs the release process, including version bump, commit, tag, publish, and push. Continue?"
      exit 0
    fi
    ;;

  mark-edit)
    path="$(edited_path)"
    [ -z "$path" ] && exit 0
    rel="${path#$PROJECT_DIR/}"

    if is_code_path "$rel"; then
      load_state
      CODE_DIRTY=1
      FMT_OK=0
      CLIPPY_OK=0
      TEST_OK=0
      LAST_EDITED_FILE="$rel"
      if is_pysof_path "$rel"; then
        PYSOF_DIRTY=1
        PYSOF_PYTEST_OK=0
        PYSOF_RUST_TEST_OK=0
      fi
      save_state
      json_context "PostToolUse" "Recorded code edit in $rel; before completion, run formatting, CI-style clippy, and affected tests."
    fi
    ;;

  record-check)
    cmd="$(command_text)"
    load_state
    changed=0

    if [[ "$cmd" == *"cargo fmt --all"* ]]; then
      FMT_OK=1
      changed=1
    fi

    if [[ "$cmd" == *"cargo clippy"* && "$cmd" == *"--all-targets"* && "$cmd" == *"--all-features"* && "$cmd" == *"-D warnings"* ]]; then
      CLIPPY_OK=1
      changed=1
    fi

    if [[ "$cmd" == *"cargo test"* ]]; then
      TEST_OK=1
      changed=1
      if [[ "$cmd" == *"crates/pysof"* ]]; then
        PYSOF_RUST_TEST_OK=1
      fi
    fi

    if [[ "$cmd" == *"uv run pytest python-tests/"* || "$cmd" == *"uv run pytest"* && "$cmd" == *"python-tests"* ]]; then
      PYSOF_PYTEST_OK=1
      changed=1
    fi

    if [ "$changed" -eq 1 ]; then
      save_state
    fi
    ;;

  stop-gate)
    load_state
    code_paths="$(changed_code_paths)"

    if [ -z "$code_paths" ]; then
      CODE_DIRTY=0
      PYSOF_DIRTY=0
      FMT_OK=0
      CLIPPY_OK=0
      TEST_OK=0
      PYSOF_PYTEST_OK=0
      PYSOF_RUST_TEST_OK=0
      save_state
      exit 0
    fi

    pysof_changed=0
    while IFS= read -r path; do
      if is_pysof_path "$path"; then
        pysof_changed=1
      fi
    done <<< "$code_paths"

    missing=()
    [ "$FMT_OK" = "1" ] || missing+=("cargo fmt --all")
    [ "$CLIPPY_OK" = "1" ] || missing+=("CI-style cargo clippy --all-targets --all-features -- -D warnings ...")
    [ "$TEST_OK" = "1" ] || missing+=("affected cargo test")

    if [ "$pysof_changed" -eq 1 ]; then
      [ "$PYSOF_PYTEST_OK" = "1" ] || missing+=("cd crates/pysof && uv run pytest python-tests/ -v")
      [ "$PYSOF_RUST_TEST_OK" = "1" ] || missing+=("cd crates/pysof && cargo test")
    fi

    if [ "${#missing[@]}" -gt 0 ]; then
      missing_text=""
      for item in "${missing[@]}"; do
        if [ -z "$missing_text" ]; then
          missing_text="$item"
        else
          missing_text="$missing_text, $item"
        fi
      done
      reason="Code files changed but required repository checks have not been recorded: $missing_text. Run the missing checks or explain why they cannot be run."
      json_stop_block "$reason"
    fi
    ;;

  *)
    exit 0
    ;;
esac
