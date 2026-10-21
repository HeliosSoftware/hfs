#!/usr/bin/env bash
#
# Cap the workspace target/ dir with cargo-sweep. Shared verbatim by the Unix
# and Windows steps in action.yml — see the header there for why this exists.
#
# Inputs arrive as environment variables (MODE, MAXSIZE, WANT, FREE_FLOOR_GB,
# MIN_FREE_GB, INCREMENTAL_MAX_AGE_DAYS) rather than `${{ inputs.* }}` interpolation, so the two call
# sites run byte-identical code and cannot drift.
#
# MODE=post (default) is the end-of-job sweep to MAXSIZE. MODE=preflight runs
# before a build and only acts when the disk has less than MIN_FREE_GB free —
# see "Pre-build headroom" in action.yml for why the post sweep alone is not
# enough.

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

MODE="${MODE:-post}"
MAXSIZE="${MAXSIZE:-}"
WANT="${WANT:-}"
FREE_FLOOR_GB="${FREE_FLOOR_GB:-0}"
MIN_FREE_GB="${MIN_FREE_GB:-0}"
INCREMENTAL_MAX_AGE_DAYS="${INCREMENTAL_MAX_AGE_DAYS:-0}"

# `du` over a multi-million-file target dir is slow everywhere and *very* slow
# on NTFS through Git Bash, where every stat crosses the Win32 layer. It is only
# reporting, so bound it and accept "?" when it does not finish.
#
# On Windows, skip it outright. A capped (~100GB) target dir never finished
# inside the 180s bound there, so the two calls burned ~6 minutes of every
# Windows job to print "? before, ? after" — out of test-fhirpath's 45-minute
# budget. cargo-sweep's own "Cleaned N GiB" line already reports what the sweep
# did, and the `df` below reports what is left.
#
# `timeout` is GNU coreutils: present on Linux and in Git Bash, absent from a
# stock macOS (which has neither `timeout` nor `gtimeout` unless coreutils is
# brewed). Degrade to a plain `du` there rather than silently reporting "?" on a
# runner where this used to print real numbers.
if [ "${RUNNER_OS:-}" = "Windows" ]; then
  dir_size() { :; }
elif command -v timeout >/dev/null 2>&1; then
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

# Where the space outside target/ is going. A ceiling on target/ bounds nothing
# else on the box, and on the Windows agents the non-target share has been
# climbing ~20GB per `test-rust` run (agent9: ~170GB -> ~235GB in one day of
# 2026-10-14) while cargo-sweep's own accounting stayed flat at the cap. Each
# probe is bounded; "?" means it did not finish in time, not that it is empty.
report_disk_consumers() {
  echo "Disk consumers outside the swept part of target/ (bounded probes, '?' = timed out):"
  # Siblings of this checkout under the runner's _work/ (_actions, _tool, _temp,
  # other repositories), but not the checkout itself: that is target/.
  local ws="${RUNNER_WORKSPACE:-}" work=""
  ws="${ws//\\//}"
  [ -n "$ws" ] && work="$(dirname "$ws")"
  local d
  for d in "${TEMP:-}" "${TMPDIR:-}" \
           "$HOME/.cargo/registry" "$HOME/.cargo/git" "$HOME/.rustup" \
           target/tmp ${work:+"$work"/*}; do
    [ -n "$d" ] && [ -e "$d" ] || continue
    [ "$d" = "$ws" ] && continue
    local sz=""
    if command -v timeout >/dev/null 2>&1; then
      sz="$(timeout 60 du -sh "$d" 2>/dev/null | cut -f1)"
    else
      sz="$(du -sh "$d" 2>/dev/null | cut -f1)"
    fi
    printf '  %8s  %s\n' "${sz:-?}" "$d"
  done
  # cargo-sweep picks what to evict by file *access* time. If NTFS last-access
  # updates are disabled (DisableLastAccess = 1 or 3), atime never moves on a
  # dependency that is reused rather than rebuilt, so the warm deps look oldest
  # and are evicted first, and every run rebuilds them.
  if [ "${RUNNER_OS:-}" = "Windows" ] && command -v fsutil >/dev/null 2>&1; then
    MSYS_NO_PATHCONV=1 fsutil behavior query disablelastaccess 2>/dev/null | sed 's/^/  /' || true
  fi
}

ensure_cargo_sweep() {
  local have
  have="$(cargo sweep --version 2>/dev/null | awk '{print $NF}')"
  if [ "$have" != "$WANT" ]; then
    echo "cargo-sweep: have '${have:-none}', want '${WANT}' — installing."
    cargo install cargo-sweep --locked --version "$WANT" || true
  fi
  cargo sweep --version >/dev/null 2>&1
}

# cargo-sweep skips incremental/ entirely ("not tracked by fingerprint"): it
# never removes those dirs, yet counts them toward --maxsize, so stale ones
# grow without bound and push warm dependencies out of the cap instead. Prune
# per-crate dirs (`incremental/<crate>-<hash>`) not written in
# INCREMENTAL_MAX_AGE_DAYS. Directory mtime is the right clock: rustc creates a
# new session dir inside on every compile of that crate, and it does not depend
# on NTFS last-access updates the way cargo-sweep's atime LRU does.
prune_incremental() {
  [ "$INCREMENTAL_MAX_AGE_DAYS" -gt 0 ] 2>/dev/null || return 0
  local inc n
  for inc in target/*/incremental target/*/*/incremental; do
    [ -d "$inc" ] || continue
    n="$(find "$inc" -mindepth 1 -maxdepth 1 -type d -mtime +"$INCREMENTAL_MAX_AGE_DAYS" 2>/dev/null | wc -l | tr -d ' ')"
    [ "${n:-0}" -gt 0 ] || continue
    echo "Pruning ${n} incremental cache dir(s) under ${inc} not built in ${INCREMENTAL_MAX_AGE_DAYS}+ days (cargo-sweep never evicts these)."
    find "$inc" -mindepth 1 -maxdepth 1 -type d -mtime +"$INCREMENTAL_MAX_AGE_DAYS" \
      -exec rm -rf {} + 2>/dev/null || true
  done
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

if [ "$MODE" = "preflight" ]; then
  prune_incremental
  free="$(free_gb)"
  if [ -z "$free" ] || [ "$MIN_FREE_GB" -le 0 ]; then
    echo "Pre-build headroom check disabled or free space unknown; nothing to do."
    exit 0
  fi
  if [ "$free" -ge "$MIN_FREE_GB" ]; then
    echo "${free}GB free, this job needs ${MIN_FREE_GB}GB — no pre-build eviction needed."
    exit 0
  fi
  echo "::warning title=Pre-build eviction::${RUNNER_NAME:-this runner} has ${free}GB free but this job needs ${MIN_FREE_GB}GB to build. Evicting from target/ below its ${MAXSIZE} cap so the build cannot run out of disk mid-link; expect a colder build than usual."
  report_disk_consumers
  # Shrink the ceiling by the shortfall and re-measure, a few times: cargo-sweep
  # counts hardlinked outputs twice, so sweeping N "GiB" can free less than N on
  # disk. Starting from the post-job cap is right because the previous job's
  # post sweep left target/ there.
  ceiling="$(printf '%s' "$MAXSIZE" | awk '{ n = $0 + 0; u = tolower($0);
    if (u ~ /t/) n *= 1024; else if (u ~ /m/) n /= 1024; else if (u ~ /k/) n /= 1048576;
    printf "%d", n }')"
  if ensure_cargo_sweep; then
    for _ in 1 2 3 4; do
      [ "$free" -ge "$MIN_FREE_GB" ] && break
      ceiling=$(( ceiling - (MIN_FREE_GB - free) ))
      [ "$ceiling" -lt 10 ] && break
      echo "Sweeping target/ to ${ceiling}GB..."
      cargo sweep --maxsize "${ceiling}GB" || true
      free="$(free_gb)"
      echo "  ${free}GB free."
    done
  fi
  # A cold build is slow; a build that dies of ENOSPC an hour in is slower.
  if [ "$free" -lt "$MIN_FREE_GB" ]; then
    echo "Still ${free}GB free after sweeping; removing target/ entirely."
    cargo clean || rm -rf target || true
    free="$(free_gb)"
    echo "  ${free}GB free."
  fi
  if [ "$free" -lt "$MIN_FREE_GB" ]; then
    echo "::warning title=Runner low on disk::${RUNNER_NAME:-this runner} has only ${free}GB free with target/ removed; this job needs ~${MIN_FREE_GB}GB and will likely fail with 'no space on device'. The space is outside target/ — see the consumer report above."
  fi
  exit 0
fi

if ! ensure_cargo_sweep; then
  echo "::warning::cargo-sweep unavailable (install failed?); skipping target dir GC. This runner's target/ is unbounded until a later run succeeds."
  exit 0
fi

before="$(dir_size target)"
prune_incremental
cargo sweep --maxsize "$MAXSIZE" || true
after="$(dir_size target)"
if [ "${RUNNER_OS:-}" = "Windows" ]; then
  echo "target/: swept to ceiling ${MAXSIZE} (size not measured on Windows; see cargo-sweep's 'Cleaned' line above)."
else
  echo "target/: ${before:-?} before, ${after:-?} after (ceiling ${MAXSIZE})."
fi
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
  report_disk_consumers
fi

# Belt and braces: the script's exit status is the last command's, and no future
# edit to the reporting above should be able to fail the job.
exit 0
