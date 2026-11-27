#!/usr/bin/env bash
# Caps how many testcontainer suites run on the shared Docker host at once.
#
#   docker-host-slot.sh acquire   wait for a free slot and take it
#   docker-host-slot.sh release   give the slot back (run with `if: always()`)
#
# Every self-hosted agent runs its tests on its own machine, but all of them
# start their Elasticsearch, PostgreSQL and MongoDB containers on the one host
# behind DOCKER_HOST. Nothing limited how many suites shared it. Over
# 2026-11-25/26, 56 of 95 `Test Rust` jobs failed, almost all in
# time-sensitive persistence tests ("writing to the search index timed out
# after 30s", "the deferred rebuild timed out", MaxTimeMSExpired), and the
# failure rate followed the number of suites running together: 23% with two,
# 50% with three, 59% with four, 72% with five. The test step also grew from
# 49 to 66 minutes.
#
# A slot is a container on that host with a fixed name. `docker run --name`
# fails when the name is taken, so taking a slot is atomic on the host and
# needs no state on the agents. GitHub's own `concurrency:` groups do not
# fit: a group holds one running and one pending job, and a newer pending job
# cancels the older one.
#
# This never fails the job. If Docker is unreachable, the image cannot be
# pulled, or no slot frees up in time, the job goes ahead without one.
set -uo pipefail

slots=${DOCKER_HOST_TEST_SLOTS:-}
[ -n "$slots" ] || slots=2
max_wait_min=${DOCKER_HOST_SLOT_MAX_WAIT_MIN:-120}
# How long a slot outlives a job that never releases it (a killed runner).
# Longer than a healthy test step, shorter than docker-host-gc's 180 minutes.
hold_min=${DOCKER_HOST_SLOT_HOLD_MIN:-170}
image=${DOCKER_HOST_SLOT_IMAGE:-alpine:3}
prefix=hfs-ci-test-slot
owner="${GITHUB_RUN_ID:-local}-${GITHUB_RUN_ATTEMPT:-1}-${GITHUB_JOB:-job}-${RUNNER_NAME:-runner}"
state_file="${RUNNER_TEMP:-/tmp}/docker-host-slot-${GITHUB_JOB:-job}"

summary() { echo "$*" | tee -a "${GITHUB_STEP_SUMMARY:-/dev/null}"; }

# Load on the Docker host itself, which the agents cannot see otherwise.
host_load() {
	local out
	out=$(docker run --rm "$image" sh -c \
		'echo "load $(cut -d" " -f1-3 /proc/loadavg) on $(nproc) CPUs, $(awk "/MemAvailable/ {print int(\$2/1024)}" /proc/meminfo) MiB available"' 2>/dev/null) || out="load unknown"
	echo "$out, $(docker ps -q 2>/dev/null | wc -l | tr -d ' ') containers running"
}

holders() {
	docker ps --filter "name=^${prefix}-" \
		--format '  {{.Names}}: {{.Label "hfs-ci-slot-owner"}} (up {{.RunningFor}})' 2>/dev/null
}

acquire() {
	rm -f "$state_file"
	if ! docker version >/dev/null 2>&1; then
		echo "::warning::Docker host not reachable from this step; running the tests without a slot."
		return 0
	fi
	case "$slots" in
	'' | *[!0-9]* | 0)
		echo "DOCKER_HOST_TEST_SLOTS='$slots': the cap is off."
		return 0
		;;
	esac

	local start now waited k i out last_report=-1
	start=$(date +%s)
	while :; do
		# Start at a random slot so waiting jobs don't all retry the same one.
		for ((i = 0; i < slots; i++)); do
			k=$(((RANDOM + i) % slots + 1))
			if out=$(docker run -d --rm --name "${prefix}-${k}" \
				--label hfs-ci=true \
				--label "github.run_id=${GITHUB_RUN_ID:-local}" \
				--label "hfs-ci-slot-owner=${owner}" \
				"$image" sleep "$((hold_min * 60))" 2>&1); then
				echo "$k" >"$state_file"
				now=$(date +%s)
				summary "Docker host slot ${k} of ${slots} taken after $(((now - start) / 60)) min. Host at start: $(host_load)."
				return 0
			fi
			if ! grep -q 'already in use' <<<"$out"; then
				echo "::warning::Could not take a Docker host slot; running the tests without one: ${out}"
				return 0
			fi
		done

		now=$(date +%s)
		waited=$(((now - start) / 60))
		if [ "$waited" -ge "$max_wait_min" ]; then
			echo "::warning::No Docker host slot freed up in ${max_wait_min} minutes; running the tests without one."
			holders
			return 0
		fi
		if [ $((waited / 5)) -ne "$last_report" ]; then
			last_report=$((waited / 5))
			echo "Waited ${waited} min for one of ${slots} Docker host slots. Held by:"
			holders
		fi
		sleep $((20 + RANDOM % 20))
	done
}

release() {
	[ -f "$state_file" ] || return 0
	local k name held_by
	k=$(cat "$state_file")
	rm -f "$state_file"
	name="${prefix}-${k}"
	summary "Docker host at end: $(host_load)."
	# The slot expires on its own after hold_min, and another job may have
	# taken the name since. Only remove it while it is still this job's.
	held_by=$(docker inspect -f '{{index .Config.Labels "hfs-ci-slot-owner"}}' "$name" 2>/dev/null) || return 0
	if [ "$held_by" = "$owner" ]; then
		docker rm -f "$name" >/dev/null 2>&1 || true
		echo "Released Docker host slot ${k}."
	fi
}

case "${1:-}" in
acquire) acquire ;;
release) release ;;
*)
	echo "usage: $0 acquire|release" >&2
	exit 2
	;;
esac
exit 0
