#!/usr/bin/env bash
#
# Why a backend container died (host OOM vs. cgroup OOM vs. crash), captured
# before the Stop steps remove the evidence. Based on inferno-us-core.yml's
# "Diagnose MongoDB failure", widened to every container this leg owns.
#
# Called from: the `benchmark` job's "Diagnose backend container failure"
# step (fhir-benchmark.yml), which runs on
# `failure() || cancelled() || env.LEG_CONTAINER_DIED == 'true'` — see that
# step's own `if:` for why (that rationale stays in the YAML, not here).
#
# The original step's `run:` had no explicit `set` line, so it ran under
# GitHub's bare default of `bash -e {0}` (errexit only, no pipefail, no -u).
# `bash <script>` does not inherit that, so it is reproduced explicitly
# below rather than assumed.
#
# Required environment (exported by the workflow step's env:):
#   BACKEND   matrix.backend
#   RUN_ID    github.run_id
#
# Also reads PG_CONTAINER / MONGO_CONTAINER / ES_CONTAINER, already exported
# to $GITHUB_ENV by "Configure backend env" (whichever of the three this leg
# sets — unset ones are fine, see the loop below), and host-mem.sh (same
# directory) for the docker-info/docker-stats-derived memory reading
# captured below.
#
# Outputs: none (stdout/`::group::`/`::endgroup::` only — this never fails
# the step, every command is `|| true`).
set -e

# `timeout 15`, not the usual 60: this step's own timeout-minutes is 3, and
# on cancellation it competes with Upload results/server log and the Stop
# steps for the 5-minute cancellation window. Up to 2 containers (a
# *-elasticsearch leg) x 2 calls each (4), plus the 5 standalone calls below
# (dmesg run + rm, meminfo run + rm, docker stats), keeps that part's worst
# case near 135s (9 calls x 15s) instead of eating the whole step budget on
# ~60s-per-call timeouts. The host-mem.sh call further below is bounded the
# same way, but via ITS OWN internal per-call timeout (HFS_BENCH_MEM_TIMEOUT
# below), not an external `timeout` wrapper — see the comment at that call
# for why an external wrapper is the wrong tool here. host-mem.sh no longer
# starts any helper containers (it derives MemAvailable from `docker info` +
# `docker stats` alone, see its header), so at the HFS_BENCH_MEM_TIMEOUT=10
# override used below its own worst case is ~20s (10s docker info + 10s
# docker stats) — down from the ~50s the three-candidate-read era needed.
# Step total: ~155s against the 180s (3min) ceiling — 25s of margin for
# GitHub's own overhead and cancellation-window contention.
for c in ${PG_CONTAINER:-} ${MONGO_CONTAINER:-} ${ES_CONTAINER:-}; do
  echo "::group::$c state + last 60 log lines"
  timeout 15 docker inspect "$c" --format 'OOMKilled={{.State.OOMKilled}} ExitCode={{.State.ExitCode}} Status={{.State.Status}} Error={{.State.Error}} FinishedAt={{.State.FinishedAt}}' 2>&1 || true
  timeout 15 docker logs "$c" --tail 60 2>&1 || true
  echo "::endgroup::"
done
echo "::group::Host OOM-killer (kernel ring buffer)"
DMESG_NAME="hfs-bench-dmesg-${BACKEND}-${RUN_ID}"
timeout 15 docker run --rm --privileged --name "$DMESG_NAME" \
  --label hfs-bench=1 --label "hfs-bench-run=${RUN_ID}" --label "hfs-bench-leg=${BACKEND}" \
  busybox dmesg 2>/dev/null \
  | grep -iE "out of memory|killed process|oom" | tail -25 || true
# `timeout 15` only kills the local docker CLI, not a container still
# starting on the daemon — force it gone rather than trust `--rm`
# alone, same reasoning as the voldf cleanups in the YAML's Start
# Postgres step, start-mongodb.sh and start-elasticsearch.sh.
timeout 15 docker rm -f "$DMESG_NAME" >/dev/null 2>&1 || true
echo "(end of OOM grep — empty means no OOM lines found)"
echo "::endgroup::"

# This host is shared with the rest of CI, so a container that
# died here is not necessarily this leg's own doing — a per-leg
# OOMKilled=false plus a healthy-looking log can still be a
# neighbour's memory pressure. These numbers are what tell the two
# apart after the fact.
#
# Two readings, deliberately: a plain container's own /proc/meminfo
# (below, labelled "container view" — NOT the daemon's real limit) AND
# host-mem.sh's derived estimate (further below, from `docker info` +
# `docker stats`; no /proc/meminfo read at all). Run 36410157709 is why
# both are worth keeping in a post-mortem — that run's plain /proc/meminfo
# read (MemAvailable 61324MB) disagreed with `docker info`'s MemTotal
# (12000MB) by over 5x, most likely (see host-mem.sh's header for the
# fuller reasoning) an lxcfs-virtualised Docker host resolving the reading
# container's own near-empty cgroup rather than the host's. Keeping the
# raw, unvalidated figure alongside host-mem.sh's derived number lets a
# future post-mortem see both the symptom and the corrected number,
# instead of only ever seeing the corrected one.
echo "::group::Docker host memory + containers"
MEMINFO_NAME="hfs-bench-meminfo-${BACKEND}-${RUN_ID}"
echo "container view (not the daemon's 12 GB limit — see host-mem.sh):"
timeout 15 docker run --rm --name "$MEMINFO_NAME" \
  --label hfs-bench=1 --label "hfs-bench-run=${RUN_ID}" --label "hfs-bench-leg=${BACKEND}" \
  busybox sh -c 'grep -E "MemTotal|MemFree|MemAvailable|SwapTotal|SwapFree" /proc/meminfo' 2>&1 || true
# `timeout 15` only kills the local docker CLI, not a container still
# starting on the daemon — force it gone rather than trust `--rm`
# alone, same reasoning as the voldf cleanups in the YAML's Start
# Postgres step, start-mongodb.sh and start-elasticsearch.sh.
timeout 15 docker rm -f "$MEMINFO_NAME" >/dev/null 2>&1 || true
# `docker stats` BEFORE host-mem.sh, deliberately: host-mem.sh's own worst
# case (see below) is the biggest single chunk of this step's remaining
# budget, so capture the cheap, bounded reading first — a step killed by
# its own timeout-minutes mid-host-mem.sh still leaves this one behind,
# instead of losing it too (pre-#1475-followup-round-2 ordering lost this
# capture whenever host-mem.sh ran long).
timeout 15 docker stats --no-stream --format 'table {{.Name}}\t{{.MemUsage}}\t{{.MemPerc}}\t{{.CPUPerc}}' 2>&1 || true
echo "-- host-mem.sh (docker info + docker stats derived estimate; no /proc/meminfo read) --"
# No OUTER `timeout` wrapper here: host-mem.sh no longer starts any helper
# containers (it derives MemAvailable from `docker info` + `docker stats`
# alone — see its header), so the old risk of an external wrapper killing
# it mid-candidate and leaking an orphaned container is gone. This still
# passes host-mem.sh's OWN internal per-call knob (HFS_BENCH_MEM_TIMEOUT) a
# smaller value instead of wrapping the call, so each of its two docker
# calls is individually bounded rather than the whole script being cut off
# mid-call by an outer `timeout` (which would just report whatever
# partial/no output host-mem.sh had produced, instead of host-mem.sh's own
# clean "source=none"/"avail_mb=unknown" fallback). A diagnostic capture
# gains little from waiting out the full default 60s-per-call budget; a
# reading it can get quickly is just as useful as one it spends longer
# confirming, and "avail_mb=unknown" here (this step's other captures above
# already have the raw numbers) is a fine outcome for a post-mortem.
HOST_MEM_SH="$(dirname "${BASH_SOURCE[0]}")/host-mem.sh"
if [ -f "$HOST_MEM_SH" ]; then
  HFS_BENCH_MEM_TIMEOUT=10 bash "$HOST_MEM_SH" 2>&1 || true
else
  echo "(host-mem.sh missing)"
fi
echo "::endgroup::"
