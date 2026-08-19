#!/usr/bin/env bash
#
# Cap the workspace target/ dir with cargo-sweep. Shared verbatim by the Unix
# and Windows steps in action.yml — see the header there for why this exists.
#
# Inputs arrive as environment variables (MAXSIZE, WANT, FREE_FLOOR_GB) rather
# than `${{ inputs.* }}` interpolation, so the two call sites run byte-identical
# code and cannot drift.

# `set +e` is load-bearing, and NOT redundant with omitting `set -e`. Both call
# sites invoke bash with `-e -o pipefail` at the process level, so errexit is
# already on before the first line runs; simply not turning it on does not turn
# it off. Without this the very first probe -- `cargo sweep --version` on a
# runner that has not got cargo-sweep yet -- exits with cargo's 101 and fails
# the job, which is exactly the opposite of what this script is for. That is not
# hypothetical: it happened on the first CI run of this action.
#
# This is janitorial work running under `if: always()`. Failing to reclaim space
# must never turn a green build red, so every command below is non-fatal and
# reports loudly instead, and the script ends in an explicit `exit 0`.
set +e
set -uo pipefail

MAXSIZE="${MAXSIZE:-}"
WANT="${WANT:-}"
FREE_FLOOR_GB="${FREE_FLOOR_GB:-0}"

# `du` over a multi-million-file target dir is slow everywhere and *very* slow
# on NTFS through Git Bash, where every stat crosses the Win32 layer. It is only
# reporting, so bound it and accept "?" when it does not finish.
#
# `timeout` is GNU coreutils: present on Linux and in Git Bash, absent from a
# stock macOS (which has neither `timeout` nor `gtimeout` unless coreutils is
# brewed). Degrade to a plain `du` there rather than silently reporting "?" on a
# runner where this used to print real numbers.
if command -v timeout >/dev/null 2>&1; then
  dir_size() { timeout 180 du -sh "$1" 2>/dev/null | cut -f1; }
elif command -v gtimeout >/dev/null 2>&1; then
  dir_size() { gtimeout 180 du -sh "$1" 2>/dev/null | cut -f1; }
else
  dir_size() { du -sh "$1" 2>/dev/null | cut -f1; }
fi

# Free space on the filesystem holding the checkout, in whole GB. `df -k` is the
# portable spelling: GNU coreutils (Linux, Git Bash) and BSD df (macOS) all
# agree on 1K blocks in field 4, whereas `-h` output is not machine-readable and
# `--output=` is GNU-only.
free_gb() {
  df -k . 2>/dev/null | awk 'NR==2 {printf "%d", $4/1024/1024}'
}

if [ ! -d target ]; then
  echo "No target/ directory in $(pwd) — nothing to sweep."
  exit 0
fi

# Guard against the footgun the flag invites. cargo-sweep's own help says "Unit
# defaults to MB", so `--maxsize 100` means 100 megabytes, not 100 gigabytes.
# Refuse a unitless value rather than silently nuking the cache the
# `clean: false` checkouts exist to preserve.
if ! printf '%s' "$MAXSIZE" | grep -qiE '^[0-9]+(\.[0-9]+)?[[:space:]]*(k|m|g|t)i?b$'; then
  echo "::error::maxsize='${MAXSIZE}' has no unit suffix. cargo-sweep reads a bare number as MEGABYTES, which would wipe the warm build cache on every run. Use e.g. 100GB."
  exit 0
fi

HAVE="$(cargo sweep --version 2>/dev/null | awk '{print $NF}')"
if [ "$HAVE" != "$WANT" ]; then
  echo "cargo-sweep: have '${HAVE:-none}', want '${WANT}' — installing."
  cargo install cargo-sweep --locked --version "$WANT" || true
fi
if ! cargo sweep --version >/dev/null 2>&1; then
  echo "::warning::cargo-sweep unavailable (install failed?); skipping target dir GC. This runner's target/ is unbounded until a later run succeeds."
  exit 0
fi

before="$(dir_size target)"
cargo sweep --maxsize "$MAXSIZE" || true
after="$(dir_size target)"
echo "target/: ${before:-?} before, ${after:-?} after (ceiling ${MAXSIZE})."
df -h . 2>/dev/null | tail -2 || true

# The failure this guard exists for was invisible until it was terminal: the
# runner filled up and the NEXT job died in "Set up job", before any step could
# run and therefore before any workflow change could help it. A ceiling on
# target/ does not bound anything else on the box (~/.cargo/registry, the
# runner's _diag logs, stale _work/_actions, Docker/WSL images), so once we have
# swept, say so out loud while a human can still act on it.
free="$(free_gb)"
if [ -n "$free" ] && [ "$FREE_FLOOR_GB" -gt 0 ] && [ "$free" -lt "$FREE_FLOOR_GB" ]; then
  echo "::warning title=Runner low on disk::${RUNNER_NAME:-this runner} has ${free}GB free after sweeping target/ to ${MAXSIZE}, below the ${FREE_FLOOR_GB}GB floor. target/ is not the only thing growing here — check ~/.cargo/registry, the runner's _diag/ logs and _work/_actions/. A runner that reaches 0 fails the NEXT job in 'Set up job', where no workflow step can rescue it."
fi

# Belt and braces: the script's exit status is the last command's, and no future
# edit to the reporting above should be able to fail the job.
exit 0
