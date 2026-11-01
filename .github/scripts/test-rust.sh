#!/usr/bin/env bash
# The "Test Rust" job in ci.yml, as two subcommands so the build and the test
# run show up as separate steps with their own timings.
#
#   test-rust.sh build   compile every test target (`--no-run`), with
#                        `--timings` and a memory sampler
#   test-rust.sh run     run the tests in two lanes
#
# Both keep `--workspace --all-features`. Target selection doesn't change
# feature resolution, but package selection does (see the MinIO step in
# ci.yml), so `run` selects targets, never packages, and `cargo test` only
# checks freshness against the `build` artifacts.
set -euo pipefail

features=(--workspace --all-features)

# --- memory sampler ----------------------------------------------------------
#
# Samples this container's cgroup v2 counters once a second while the build
# runs, to answer "how much RAM does CARGO_BUILD_JOBS=N need".
#
# `memory.current` is the wrong number: it counts page cache, and a build that
# writes tens of GB of artifacts fills the cache up to the limit whatever the
# real need is. Cache is reclaimable, so the sampler tracks `anon` (process
# memory) from memory.stat and swap use separately. `anon + swap` is the
# working set the build actually needed. Each time anon reaches a new high by
# 512 MiB, it records the largest processes, so the report names the crates
# (rustc `--crate-name`) or links that were running at the peak.
#
# On Proxmox LXC this is the container's cgroup. It is a no-op where there is
# no cgroup v2 memory controller (Windows, macOS).
cg=/sys/fs/cgroup
mem_log=memory-sample.log
mem_top=memory-peak-procs.log

sampler() {
	local max_anon=0 max_swap=0 max_total=0 next_snap=0 anon swap total
	while :; do
		anon=$(awk '$1 == "anon" { print $2 }' "$cg/memory.stat")
		swap=$(cat "$cg/memory.swap.current" 2>/dev/null || echo 0)
		total=$((anon + swap))
		((anon > max_anon)) && max_anon=$anon
		((swap > max_swap)) && max_swap=$swap
		((total > max_total)) && max_total=$total
		if ((anon >= next_snap)); then
			next_snap=$((anon + 512 * 1024 * 1024))
			{
				echo "=== $(date -u +%H:%M:%S) anon=$((anon >> 20)) MiB swap=$((swap >> 20)) MiB"
				ps -eo rss=,args= --sort=-rss | head -8 |
					awk '{ rss = int($1 / 1024); name = $2
					       for (i = 2; i < NF; i++) if ($i == "--crate-name") name = "rustc " $(i + 1)
					       sub(/^.*\//, "", name); printf "  %6d MiB  %s\n", rss, name }'
			} >"$mem_top.new"
			mv "$mem_top.new" "$mem_top"
		fi
		echo "$max_anon $max_swap $max_total" >"$mem_log"
		sleep 1
	done
}

start_sampler() {
	if [ ! -r "$cg/memory.stat" ]; then
		echo "No cgroup v2 memory.stat here; skipping the memory sampler."
		return
	fi
	oom_before=$(awk '$1 == "oom_kill" { print $2 }' "$cg/memory.events" 2>/dev/null || echo "?")
	rm -f "$mem_log" "$mem_top"
	sampler &
	sampler_pid=$!
}

stop_sampler() {
	[ -n "${sampler_pid:-}" ] || return 0
	kill "$sampler_pid" 2>/dev/null || true
	wait "$sampler_pid" 2>/dev/null || true
	local max_anon max_swap max_total limit swap_limit oom_after
	read -r max_anon max_swap max_total <"$mem_log" || return 0
	limit=$(cat "$cg/memory.max" 2>/dev/null || echo "?")
	swap_limit=$(cat "$cg/memory.swap.max" 2>/dev/null || echo "?")
	oom_after=$(awk '$1 == "oom_kill" { print $2 }' "$cg/memory.events" 2>/dev/null || echo "?")
	mib() { case $1 in '' | *[!0-9]*) echo "$1" ;; *) echo "$(($1 >> 20)) MiB" ;; esac; }
	{
		echo "### Build memory (CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-default}, $(nproc) CPUs)"
		echo
		echo "| | |"
		echo "|---|---|"
		echo "| Peak anon (RAM used by processes) | $(mib "$max_anon") |"
		echo "| Peak swap used | $(mib "$max_swap") |"
		echo "| Peak anon + swap | $(mib "$max_total") |"
		echo "| memory.max | $(mib "$limit") |"
		echo "| memory.swap.max | $(mib "$swap_limit") |"
		echo "| OOM kills during build | $oom_before -> $oom_after |"
		echo
		echo "Largest processes when anon last crossed a 512 MiB step:"
		echo '```'
		cat "$mem_top" 2>/dev/null || true
		echo '```'
	} | tee -a "${GITHUB_STEP_SUMMARY:-/dev/null}"
}

# --- subcommands -------------------------------------------------------------

build() {
	start_sampler
	trap stop_sampler EXIT
	# `--timings` writes target/cargo-timings/cargo-timing.html, uploaded as
	# an artifact by ci.yml: the per-crate and per-link timeline, which shows
	# whether more jobs would help or the build is waiting on the
	# helios-fhir -> persistence -> rest -> hfs chain.
	cargo test "${features[@]}" --no-run --timings
}

run() {
	# `cargo test` runs test binaries one at a time, and `mongodb_tests` alone
	# took 13-17 of the ~28-35 test minutes, holding up the other ~390
	# binaries. Lane A runs it on its own while lane B runs everything else,
	# the same split as the coverage job.
	#
	# Lane B names every other test target rather than excluding one, since
	# cargo has no `--exclude-test`. `--doc` can't be mixed with other target
	# flags, so the doc-tests run after the rest in lane B. A renamed
	# `mongodb_tests` fails lane A ("no test target named") instead of passing
	# vacuously, and a new test file is picked up by lane B automatically.
	#
	# `cargo test` releases the build-dir lock before it runs tests, so the two
	# lanes don't block each other.
	local others
	# Plain grep/sed rather than python or jq, which the Windows agents may
	# lack. Cargo prints each target as `"kind":[...],"crate_types":[...],
	# "name":"..."`. If that shape ever changes, the pattern stops matching some
	# test targets and the count check fails rather than silently skipping them.
	local meta pattern kinds names
	meta=$(cargo metadata --no-deps --format-version 1)
	pattern='"kind":\["test"\],"crate_types":\["bin"\],"name":"[^"]*"'
	kinds=$(grep -o '"kind":\["test"\]' <<<"$meta" | wc -l)
	names=$(grep -o "$pattern" <<<"$meta" | sed 's/.*"name":"\([^"]*\)"/\1/')
	if [ "$kinds" -eq 0 ] || [ "$(wc -l <<<"$names")" -ne "$kinds" ]; then
		echo "::error::Matched $(wc -l <<<"$names") of $kinds test targets in cargo metadata; has its format changed?"
		exit 1
	fi
	others=$(sort -u <<<"$names" | grep -vx mongodb_tests | sed 's/^/--test /' | tr '\n' ' ')

	cargo test "${features[@]}" --test mongodb_tests >mongodb-tests.log 2>&1 &
	local lane_a=$!

	local lane_b_status=0
	# shellcheck disable=SC2086 # $others is a list of `--test NAME` args
	cargo test "${features[@]}" --lib --bins $others || lane_b_status=$?
	cargo test "${features[@]}" --doc || lane_b_status=$?

	local lane_a_status=0
	wait "$lane_a" || lane_a_status=$?
	echo "::group::mongodb_tests (lane A)"
	cat mongodb-tests.log
	echo "::endgroup::"

	if [ "$lane_a_status" -ne 0 ] || [ "$lane_b_status" -ne 0 ]; then
		echo "::error::Tests failed (mongodb_tests lane: $lane_a_status, other tests lane: $lane_b_status)"
		exit 1
	fi
}

case "${1:-}" in
build) build ;;
run) run ;;
*)
	echo "usage: $0 build|run" >&2
	exit 2
	;;
esac
