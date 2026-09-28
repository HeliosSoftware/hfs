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
# sets — unset ones are fine, see the loop below).
#
# Outputs: none (stdout/`::group::`/`::endgroup::` only — this never fails
# the step, every command is `|| true`).
set -e

# `timeout 15`, not the usual 60: this step's own timeout-minutes is 3, and
# on cancellation it competes with Upload results/server log and the Stop
# steps for the 5-minute cancellation window. Up to 2 containers (a
# *-elasticsearch leg) x 2 calls each (4), plus the 5 standalone calls below
# (dmesg run + rm, meminfo run + rm, docker stats), keeps the worst case
# near 135s (9 calls x 15s) instead of eating the full 3-minute budget on
# ~60s-per-call timeouts.
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
# neighbour's memory pressure. These two numbers are what tell the
# two apart after the fact.
echo "::group::Docker host memory + containers"
MEMINFO_NAME="hfs-bench-meminfo-${BACKEND}-${RUN_ID}"
timeout 15 docker run --rm --name "$MEMINFO_NAME" \
  --label hfs-bench=1 --label "hfs-bench-run=${RUN_ID}" --label "hfs-bench-leg=${BACKEND}" \
  busybox sh -c 'grep -E "MemTotal|MemFree|MemAvailable|SwapTotal|SwapFree" /proc/meminfo' 2>&1 || true
# `timeout 15` only kills the local docker CLI, not a container still
# starting on the daemon — force it gone rather than trust `--rm`
# alone, same reasoning as the voldf cleanups in the YAML's Start
# Postgres step, start-mongodb.sh and start-elasticsearch.sh.
timeout 15 docker rm -f "$MEMINFO_NAME" >/dev/null 2>&1 || true
timeout 15 docker stats --no-stream --format 'table {{.Name}}\t{{.MemUsage}}\t{{.MemPerc}}\t{{.CPUPerc}}' 2>&1 || true
echo "::endgroup::"
