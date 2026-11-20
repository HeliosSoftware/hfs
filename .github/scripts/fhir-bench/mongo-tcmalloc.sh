#!/usr/bin/env bash
#
# Are mongod's tcmalloc per-CPU caches starved? A cheap, read-only check
# for the mongodb / mongodb-elasticsearch legs.
#
# Why: MongoDB 8.0+ ships tcmalloc-google with per-CPU caches. mongod
# sizes those caches from the container's memory limit; with NO limit (cgroup v2 memory.max =
# "max") it computes a per-CPU size of 0 (SERVER-109273,
# https://jira.mongodb.org/browse/SERVER-109273), and every malloc/free
# then goes through tcmalloc's central lock. Reads under concurrency get
# several times slower. start-mongodb.sh passes
# MONGO_TCMALLOC_PER_CPU_CACHE_SIZE_BYTES for 8.x+ images to avoid it;
# this script proves whether that took.
#
# Verdict:
#   AFFECTED  usingPerCPUCaches is true AND (maxPerCPUCacheSizeBytes is 0
#             OR the cpuCache capacities sum to 0)
#   OK        per-CPU caches with capacity, or per-thread caches
#             (usingPerCPUCaches false, e.g. GLIBC_TUNABLES= set empty)
#   n/a       no per-CPU cache stats at all: mongo:7.0 still ships the
#             older tcmalloc; expected there, not an error
#
# Every host fact is read INSIDE the mongo container, not on the runner:
# mongod runs on the remote Docker daemon (DOCKER_HOST), a different
# machine with its own kernel, CPU count and cgroups.
#
# Called from: start-mongodb.sh (startup snapshot: full report in the step
# log, verdict and sizes in the "MongoDB tuning" table of
# $GITHUB_STEP_SUMMARY) and capture_backend_stats in suite-lib.sh (after
# every suite, <suite>-tcmalloc.txt in the leg artifact; overflows and
# used bytes only mean something after load).
#
# Required environment:
#   MONGO_CONTAINER   already exported to $GITHUB_ENV by "Configure backend env"
#
# Optional environment:
#   HFS_BENCH_TCM_TIMEOUT   per-call timeout in seconds (default 30)
#
# Outputs: plain `key: value` lines on stdout. Never fails its caller:
# set -uo pipefail (deliberately no -e), every docker call bounded by
# `timeout`, always exits 0.
set -uo pipefail

T="${HFS_BENCH_TCM_TIMEOUT:-30}"
if [ -z "${MONGO_CONTAINER:-}" ]; then
  echo "tcmalloc_check: skipped (MONGO_CONTAINER unset)"
  exit 0
fi

# Single quotes on purpose: these expand inside the container, not here.
# shellcheck disable=SC2016
timeout "$T" docker exec "$MONGO_CONTAINER" sh -c '
  echo "host_kernel: $(uname -r)"
  echo "container_nproc: $(nproc 2>/dev/null || echo ?)"
  echo "cgroup_memory_max: $(cat /sys/fs/cgroup/memory.max 2>/dev/null || echo "? (not cgroup v2)")"
  echo "cgroup_cpu_max: $(cat /sys/fs/cgroup/cpu.max 2>/dev/null || echo ?)"
' 2>&1 || echo "host_facts: unreadable (docker exec failed or timed out)"

# The container's configured env (image ENV plus any `-e`), from the
# daemon rather than /proc/1/environ: mongod runs as the mongodb user and
# a root `docker exec` is refused that file on some hosts. The leading
# "=" keeps set-but-empty apart from unset.
ENV_LINES=$(timeout "$T" docker inspect -f '{{range .Config.Env}}{{println .}}{{end}}' "$MONGO_CONTAINER" 2>/dev/null) \
  || ENV_LINES="?"
for n in GLIBC_TUNABLES MONGO_TCMALLOC_PER_CPU_CACHE_SIZE_BYTES; do
  if [ "$ENV_LINES" = "?" ]; then
    echo "container_$n: ? (docker inspect failed)"
    continue
  fi
  v=$(printf '%s\n' "$ENV_LINES" | sed -n "s/^$n=/=/p" | tail -1)
  case "$v" in
    "") echo "container_$n: <unset>" ;;
    *)  echo "container_$n: \"${v#=}\"" ;;
  esac
done

# serverStatus({ tcmalloc: 2 }) is the verbose form. Paths: tcmalloc.
# usingPerCPUCaches / maxPerCPUCacheSizeBytes at the top, the allocator's
# own stats one level down under tcmalloc.tcmalloc, where cpuCache is an
# OBJECT keyed by CPU id. Any field may be absent (7.0 has none of them);
# counters can be Long, hence Number(). No single quotes inside: this is
# one shell-quoted string.
timeout "$T" docker exec "$MONGO_CONTAINER" mongosh --quiet --eval '
  const out = { mongod_version: db.version() };
  let s = {};
  try { s = db.serverStatus({ tcmalloc: 2 }); } catch (e) { out.serverStatus_error = String(e); }
  const t = s.tcmalloc || {};
  const inner = t.tcmalloc || {};
  const num = function (v) { return (v === undefined || v === null) ? null : Number(v); };
  out.usingPerCPUCaches = (t.usingPerCPUCaches === undefined) ? "absent" : t.usingPerCPUCaches;
  out.maxPerCPUCacheSizeBytes = (t.maxPerCPUCacheSizeBytes === undefined) ? "absent" : num(t.maxPerCPUCacheSizeBytes);
  out.cpu_free = (inner.cpu_free === undefined) ? "absent" : num(inner.cpu_free);
  out.thread_cache_count = (inner.thread_cache_count === undefined) ? "absent" : num(inner.thread_cache_count);
  const c = (inner.cpuCache && typeof inner.cpuCache === "object") ? inner.cpuCache : null;
  let capSum = null;
  if (c !== null) {
    let cpus = 0, nonzero = 0, over = 0, under = 0, used = 0;
    capSum = 0;
    Object.keys(c).forEach(function (k) {
      const e = c[k] || {};
      const cap = Number(e.capacity || 0);
      cpus++; capSum += cap; if (cap > 0) { nonzero++; }
      over += Number(e.overflows || 0);
      under += Number(e.underflows || 0);
      used += Number(e.used_bytes || 0);
    });
    out.cpuCache = { cpus: cpus, nonzero_capacity: nonzero, capacity_sum: capSum,
                     used_bytes: used, overflows: over, underflows: under };
  } else {
    out.cpuCache = "absent";
  }
  if (t.usingPerCPUCaches === undefined) {
    out.verdict = "n/a (no per-CPU cache stats: older tcmalloc, e.g. mongo:7.0)";
  } else if (t.usingPerCPUCaches === true
             && (num(t.maxPerCPUCacheSizeBytes) === 0 || capSum === 0)) {
    out.verdict = "AFFECTED (per-CPU caches with no capacity: every malloc takes the central lock, SERVER-109273)";
  } else if (t.usingPerCPUCaches === true) {
    out.verdict = "OK (per-CPU caches with capacity)";
  } else {
    out.verdict = "OK (per-thread caches)";
  }
  Object.keys(out).forEach(function (k) {
    const v = out[k];
    print(k + ": " + (typeof v === "object" && v !== null ? JSON.stringify(v) : v));
  });
' 2>&1 || echo "tcmalloc: unreadable (mongosh failed or timed out)"

exit 0
