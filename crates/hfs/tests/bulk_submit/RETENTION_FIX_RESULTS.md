# #995 reindex lifecycle correction

The fix passed thirteen regression tests and a fresh pilot plus twenty consecutive
bulk-submit jobs. Every post-idle inventory has zero cancellation channels.
Baseline findings: RETENTION_RESULTS.md.

## Intended behavior

- Release the cancellation sender when the background task exits, including
  normal completion, backend failure, panic/unwind and dropped kickoff futures.
- Protect a cancellation that is already reported terminal while the worker is
  still executing. Do not overwrite an existing terminal status in a racing
  completion/failure/cancellation path.
- Keep at most the most recent 1024 terminal states whose tasks have exited,
  for up to 24 hours. Sweep expired states every minute, including during idle.
  Tasks still executing are exempt. Evicted status endpoints return 404.
- Start one cleanup task lazily at the first job; construction remains valid
  without a Tokio runtime. The cleanup task holds weak map references while
  sleeping and does not retain the manager after it is dropped.
- Reclaim oversized map capacity after a burst subsides.

The policy bounds terminal record count and age, not total process memory. Active
jobs and each job's error details are not given new bounds in this change.

## Regression cases

Tests exercise the real ReindexOperation::start driver with a controlled source,
including completion, failure, cancellation blocked in the source, a panicked
task, an aborted kickoff blocked in audit, idle expiry using paused Tokio time,
1025 successive completions and dropping the manager. Polling of recent terminal
states and terminal cancellation idempotence remain checked.

Evidence root: `target/issue-995-fix/`. Regression tests are run first against
unchanged lifecycle code, then against the correction. Builds and workloads are
serial, with one Cargo job and host-memory monitoring. No unrelated containers
are stopped or reconfigured.


## Regression validation

The first test build with only R4/Postgres failed before running tests because
existing bulk-submit lib tests import SQLite without feature gating. No unrelated
source was changed; the test feature set was adjusted to R4,postgres,sqlite.
This caused additional dependency compilation. All builds used one Cargo job.

Against unchanged lifecycle code, the eight new tests executed: seven failed for
the intended retention/lifecycle reasons and one (manager release) passed.
After applying the fix, all thirteen tests under search::reindex passed,
including the original progress/status/request/audit cases. The green incremental
build plus tests took about 160 seconds; test execution itself took 0.10 seconds.

```sh
CARGO_BUILD_JOBS=1 cargo test --locked --release -p helios-persistence --lib \
  --no-default-features --features R4,postgres,sqlite \
  search::reindex:: -- --test-threads=1
```

Logs: `tests-red.log` (configuration failure), `tests-red-sqlite.log` (seven
expected regressions), `tests-green.log` (13 passed). The tests use controlled
sources and SQLite is enabled for compatibility with the existing lib test
modules; no external database container is needed for these selected tests.

## Repeated-job validation setup

The fixed production binary and the temporary inventory binary were built
serially with `CARGO_BUILD_JOBS=1`, release profile, R4/Postgres features.
Their builds succeeded in about 161 and 156 seconds respectively. The temporary
probe is the same probe used for the previous retention campaign. After the
instrumented build, the clean fixed source and default release binary were
restored before starting the workloads. SHA-256 hashes are in
`target/issue-995-fix/provenance.json`; the reviewable production diff is in
`target/issue-995-fix/fix.patch`.

A dedicated PostgreSQL 16.14 container uses the same pinned image as the prior
campaign, with one CPU, 768 MiB memory, no additional container swap, 128 MiB
shared buffers and 20 connections. Image, version and limits are recorded in
`target/issue-995-fix/postgres.json`. Each arm uses a fresh database.

The pilot and continuous arm submit 1,000 Patient resources per job in four
files, repeating the same IDs/content, with deferred indexing, one submit
worker, file concurrency one, a four-connection pool and 30 seconds idle.
The pilot runs one job; the main arm runs twenty jobs in the same HFS process.
Inventories are captured after verified reindex and idle. Heap, vmmap and
footprint are captured at startup and after jobs 1, 5, 10, 15 and 20. No build
runs alongside these workloads. The earlier restart arm remains the control;
it is not rerun here.

## Measured result

All 21 jobs passed all 336 hard checks, including stored content, IDs, version
distribution, search and receipts. All 24 diagnostic tool calls succeeded
(six in the pilot, eighteen in the continuous arm). Host pressure stayed at
level 1 and additional swapouts were zero in both arms.

| Post-idle observation | Previous continuous run | Fixed continuous run |
| --- | ---: | ---: |
| Finished states after job 20 | 20 | 20 |
| Cancellation channels after job 20 | 20 (all closed) | 0 |
| Jobs / channels map capacity after job 20 | 28 / 28 | 28 / 3 |
| Accounted occupied entries + owned buffers after job 20 | 8,080 bytes | 6,720 bytes |
| Malloc bytes after job 1 | 5,072,304 | 5,122,512 |
| Malloc bytes after job 20 | 5,109,344 | 5,106,384 |
| Malloc change, job 1 → 20 | +37,040 bytes | −16,128 bytes |
| Malloc blocks change, job 1 → 20 | +584 | +530 |

The fixed inventories show N finished states and zero channels at every ordinal
N from 1 through 20. Status retrieval succeeds after idle. The pilot likewise
retains one finished state and no channels. The 1024-state quota and idle expiry
are exercised by regression tests, not by this twenty-job workload.

The direct byte accounting excludes map buckets, channel internals and allocator
overhead. Its reduction is 1,360 bytes at twenty jobs. Whole-process malloc
measurements fluctuate: the fixed run falls to 5,079,648 bytes at job 5, then
reaches 5,106,384 bytes at job 20. The final value is close to the earlier run;
these observations do not establish a substantial reduction in total heap or
solve peak memory during ingestion. The demonstrated result is removal of the
per-job cancellation-channel retention and a tested bound on terminal metadata
count/age. Active jobs and per-job error payloads remain outside that bound.

Full inventories and heap checkpoints: `target/issue-995-fix/inventory-summary.md`.
Raw jobs, checks, receipts, resource fixtures, process/host samples and tool output
remain under `pilot/` and `consecutive-20/` in the same evidence directory.

## Final workspace and cleanup

The production fix remains in the workspace. Temporary inventory instrumentation
is absent from production source, and `target/release/hfs` matches the clean
fixed binary. The instrumented binary is preserved under its explicit diagnostic
name. Both measurement HFS processes exited; the dedicated PostgreSQL container
was stopped and removed. The 21 unrelated running containers were left untouched.
Verification is recorded in `target/issue-995-fix/final-verification.json`.

Python syntax checks passed for all four measurement/summary scripts, and
`git diff --check` passed. No commit, push or issue comment was made. This closes
the confirmed reindex metadata retention finding; #995's ingestion-peak
investigation remains separate.

## PR branch validation after synchronization

The PR branch was rebased onto origin/main at
`5d738d3fb84d629870d35d5ac01dabe9fd9682c1`. All thirteen selected reindex
tests passed again (0 failed; 0.09 seconds execution) with the same command
and one Cargo build job. Build plus tests took about 176 seconds, with normal
host pressure and no new swapouts. Evidence: `target/issue-995-fix/tests-rebased.*`.
The workload measurements above predate this rebase and are not measurements
of the updated base.
