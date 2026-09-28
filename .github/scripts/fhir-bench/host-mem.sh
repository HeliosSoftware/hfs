#!/usr/bin/env bash
#
# Single-source Docker host memory reading — never reads /proc/meminfo.
#
# Called from: capacity-gate.sh (MemTotal preflight and every MemAvailable
# poll), probe_host() (inline in the `benchmark` job's "Run benchmark
# suites" step, fhir-benchmark.yml), and diagnose.sh (post-mortem capture).
#
# Why /proc/meminfo is not used, from anywhere, any more (run 36410157709):
# "Docker host capacity gate" logged `docker info --format '{{.MemTotal}}'`
# as 12000MB, then polled MemAvailable via
# `docker run alpine:3 awk '/MemAvailable/' /proc/meminfo` and got 61324MB
# back — over 5x the host's own reported total, on a host that has
# OOM-killed a mongod before (#453) at the 12GB figure. The likely
# mechanism, not proven against this specific host: the Docker daemon here
# runs inside an LXC-like environment whose /proc/meminfo is virtualised by
# lxcfs, and lxcfs resolves MemAvailable against the READING PROCESS's own
# cgroup rather than the host's. If that is right, no container we spin up
# to read /proc/meminfo can ever be trusted for this — even a container
# that bind-mounts the daemon side's /proc/meminfo would still be read by a
# process sitting in ITS OWN (near-empty, just-started) cgroup, so it would
# report its own usage rather than the host's regardless of which
# /proc/meminfo path it opened. `docker info` itself is unaffected because
# it asks the daemon directly, not a file read from inside a container.
# So this script never reads /proc/meminfo, from any container, for any
# purpose — it derives MemAvailable from `docker info` (MemTotal) and
# `docker stats` (per-container usage) instead, neither of which involves a
# nested container reporting its own view of memory.
#
# Method:
#   total_mb = `docker info --format '{{.MemTotal}}'` (bytes -> MB), bounded
#     by `timeout ${HFS_BENCH_MEM_TIMEOUT:-60}`. Empty, non-numeric, or 0 is
#     "unknown" (source=none — nothing left to derive an estimate from).
#   avail_mb = total_mb
#              - (sum of every running container's current memory usage,
#                from `docker stats --no-stream --format '{{.MemUsage}}'`,
#                same timeout)
#              - 1024 (flat allowance for dockerd itself and any
#                non-container process on the host)
#              clamped at 0 if that goes negative.
#   `docker stats` exiting non-zero, or timing out, means "couldn't ask" —
#   NOT "nothing is running" — so avail_mb is reported "unknown" in that
#   case; it is never guessed as total_mb minus just the flat allowance.
#   Empty stdout with exit 0 legitimately means no containers are running,
#   i.e. 0 used.
#
# What the estimate includes/excludes: container memory exactly as `docker
# stats` accounts it for each container — this excludes page cache
# attributed to a container's cgroup that the kernel could reclaim under
# pressure, so a real host is often somewhat MORE available than this
# number says. The flat 1024MB is not measured; it is just assumed to
# cover dockerd plus whatever else runs directly on the host outside a
# container. Both choices make this estimate deliberately conservative: it
# is meant to never tell a caller more memory is free than actually is, at
# the cost of occasionally under-reporting.
#
# This never fails its caller: set -uo pipefail (deliberately no -e), every
# docker call bounded by `timeout`, always exits 0.
#
# Optional environment:
#   HFS_BENCH_MEM_TIMEOUT   per-docker-call timeout in seconds (default
#                           60). Up to 2 such calls run per invocation
#                           (docker info, docker stats), so this script's
#                           own worst case is about 2x this value.
#                           diagnose.sh, with its own tight step budget,
#                           passes a smaller value; capacity-gate.sh, which
#                           mainly wants correctness, leaves it at the
#                           default.
#
# Output: stdout, exactly one line, always:
#   source=<docker-stats|none> total_mb=<n|unknown> avail_mb=<n|unknown>
# source=none only when total_mb itself could not be determined. Once
# total_mb is known, source is always docker-stats, even when avail_mb
# ends up "unknown" because the docker-stats call itself failed.
#
# Exit status: always 0.
set -uo pipefail

DOCKER_TIMEOUT_S="${HFS_BENCH_MEM_TIMEOUT:-60}"

emit() {
  echo "source=$1 total_mb=$2 avail_mb=$3"
  exit 0
}

# ---- total_mb: docker info MemTotal, in MB -------------------------------
T_BYTES=$(timeout "$DOCKER_TIMEOUT_S" docker info --format '{{.MemTotal}}' 2>/dev/null) || T_BYTES=""
case "$T_BYTES" in ''|*[!0-9]*) T_BYTES="" ;; esac
T_MB=""
if [ -n "$T_BYTES" ] && [ "$T_BYTES" -gt 0 ]; then
  T_MB=$(( T_BYTES / 1024 / 1024 ))
  [ "$T_MB" -gt 0 ] || T_MB=""
fi

if [ -z "$T_MB" ]; then
  emit "none" "unknown" "unknown"
fi

# ---- avail_mb: total_mb minus every running container's current usage
# (docker stats), minus a flat 1024MB allowance -----------------------------
STATS_OUT=""
if ! STATS_OUT=$(timeout "$DOCKER_TIMEOUT_S" docker stats --no-stream --format '{{.MemUsage}}' 2>/dev/null); then
  # "Couldn't ask" (timeout, daemon hiccup) — NOT "nothing running". T is
  # still known; there is nothing left to derive an availability figure
  # from.
  emit "docker-stats" "$T_MB" "unknown"
fi

USED_MB="0"
while IFS= read -r line; do
  [ -z "$line" ] && continue
  used_part="${line%% / *}"
  num=$(printf '%s' "$used_part" | sed -E 's/^([0-9.]+).*/\1/')
  unit=$(printf '%s' "$used_part" | sed -E 's/^[0-9.]+//')
  case "$num" in ''|*[!0-9.]*) continue ;; esac
  line_mb=$(awk -v n="$num" -v u="$unit" 'BEGIN {
    u = tolower(u)
    if (u == "b")          m = 1 / 1048576
    else if (u == "kb")    m = 1000 / 1048576
    else if (u == "kib")   m = 1024 / 1048576
    else if (u == "mb")    m = 1000000 / 1048576
    else if (u == "mib")   m = 1
    else if (u == "gb")    m = 1000000000 / 1048576
    else if (u == "gib")   m = 1024
    else { print ""; exit }
    printf "%.6f", n * m
  }')
  [ -z "$line_mb" ] && continue
  USED_MB=$(awk -v a="$USED_MB" -v b="$line_mb" 'BEGIN{printf "%.6f", a+b}')
done <<< "$STATS_OUT"

USED_INT_MB=$(awk -v v="$USED_MB" 'BEGIN{printf "%d", v + 0.5}')
AVAIL_MB=$(( T_MB - USED_INT_MB - 1024 ))
[ "$AVAIL_MB" -lt 0 ] && AVAIL_MB=0

emit "docker-stats" "$T_MB" "$AVAIL_MB"
