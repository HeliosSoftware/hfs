#!/usr/bin/env bash
#
# codex-gate-log.sh - shared observability helpers for the codex plan/final gates.
#
# Sourced (not executed) by codex-verify-plan.sh and codex-final-plan-review.sh.
# Provides:
#   - gate_log_line     : timestamped append to a named .log (with rotation)
#   - gate_log_event    : one structured JSONL event to codex-gate-events.jsonl
#   - gate_snapshot_attempt : preserve this run's prompt/verdict/codex output per attempt
#   - gate_prune_sessions   : keep only the newest N per-session state dirs
#
# Expects the caller to have defined: debug_dir, session_id (may be empty).
# All functions are defensive and safe under `set -euo pipefail`.

CODEX_GATE_EVENTS_LOG="${CODEX_GATE_EVENTS_LOG:-$debug_dir/codex-gate-events.jsonl}"
CODEX_GATE_LOG_MAX_BYTES="${CODEX_GATE_LOG_MAX_BYTES:-5242880}"   # rotate a log past 5 MB
CODEX_GATE_SESSION_KEEP="${CODEX_GATE_SESSION_KEEP:-25}"          # keep newest N session dirs

gate_now() { date -u '+%Y-%m-%dT%H:%M:%SZ'; }

# Rotate a file to .1 once it exceeds the byte budget (single-generation rotation).
gate_rotate_if_big() {  # $1=file
  local f="$1" size
  [[ -f "$f" ]] || return 0
  size="$(wc -c < "$f" 2>/dev/null | tr -d ' ')"
  size="${size:-0}"
  if [[ "$size" -gt "$CODEX_GATE_LOG_MAX_BYTES" ]]; then
    mv -f "$f" "$f.1" 2>/dev/null || true
  fi
}

# Timestamped human-readable log line.
gate_log_line() {  # $1=logfile  $2=message
  local logfile="$1" message="$2"
  gate_rotate_if_big "$logfile"
  printf '[%s] %s\n' "$(gate_now)" "$message" >> "$logfile" 2>/dev/null || true
}

# Structured JSONL event. Extra args are key=value pairs; empty values are dropped.
gate_log_event() {  # $1=hook  $2=event  [key=value ...]
  local hook="$1" event="$2"
  shift 2
  gate_rotate_if_big "$CODEX_GATE_EVENTS_LOG"
  CODEX_GATE_HOOK="$hook" \
  CODEX_GATE_EVENT="$event" \
  CODEX_GATE_TS="$(gate_now)" \
  CODEX_GATE_SESSION="${session_id:-unknown}" \
  python3 - "$CODEX_GATE_EVENTS_LOG" "$@" <<'PY' 2>/dev/null || true
import json
import os
import sys

path = sys.argv[1]
rec = {
    "ts": os.environ.get("CODEX_GATE_TS", ""),
    "hook": os.environ.get("CODEX_GATE_HOOK", ""),
    "session": os.environ.get("CODEX_GATE_SESSION", ""),
    "event": os.environ.get("CODEX_GATE_EVENT", ""),
}
for pair in sys.argv[2:]:
    if "=" in pair:
        key, value = pair.split("=", 1)
        if value != "":
            rec[key] = value
try:
    with open(path, "a") as fh:
        fh.write(json.dumps(rec, ensure_ascii=False) + "\n")
except OSError:
    pass
PY
}

# Preserve this attempt's forensic artifacts so retries do not overwrite history.
# Increments the counter file and copies prompt/verdict/codex output to
# attempt-suffixed names. Sets GATE_ATTEMPT for the caller.
gate_snapshot_attempt() {  # $1=counter_file ; uses prompt_file/review_file/codex_stdout/codex_stderr if set
  local counter_file="$1" attempt f base dir
  attempt=0
  [[ -f "$counter_file" ]] && attempt="$(tr -dc '0-9' < "$counter_file" 2>/dev/null || true)"
  attempt="${attempt:-0}"
  attempt=$((attempt + 1))
  printf '%s\n' "$attempt" > "$counter_file" 2>/dev/null || true
  GATE_ATTEMPT="$attempt"
  for f in "${prompt_file:-}" "${review_file:-}" "${codex_stdout:-}" "${codex_stderr:-}"; do
    [[ -n "$f" && -f "$f" ]] || continue
    base="${f##*/}"
    dir="$(dirname "$f")"
    cp -f "$f" "$dir/${base%.*}.attempt-${attempt}.${base##*.}" 2>/dev/null || true
  done
}

# Keep only the newest N per-session state dirs under the gate state root.
gate_prune_sessions() {  # $1=state_root
  local root="$1" d
  [[ -d "$root" ]] || return 0
  while IFS= read -r d; do
    [[ -n "$d" ]] || continue
    rm -rf "$d" 2>/dev/null || true
  done < <(ls -1dt "$root"/*/ 2>/dev/null | tail -n +"$((CODEX_GATE_SESSION_KEEP + 1))" || true)
}
