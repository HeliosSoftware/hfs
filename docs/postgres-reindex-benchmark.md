# PostgreSQL reindex benchmark for issue #1086

This protocol measures the deferred PostgreSQL reindex after `$bulk-submit` without importing the full manual-testing corpus. It is an issue-specific performance check, not a replacement for the release matrix.

## Fixed scope

- Build only the `hfs` R4 PostgreSQL binary from the source revision under test.
- Use `2,000` deterministic Patient resources per trial. The controller rejects more than `10,000` resources when the #1086 evidence profile is selected.
- Run one import and one deferred reindex at a time. The Rust phase counters are process-global and cannot separate concurrent jobs.
- Run three fresh, equivalent trials for the baseline and three for the candidate. Restore the same empty PostgreSQL image or dump before every trial.
- Keep the PostgreSQL image, container limits, HFS settings, host load policy, fixture generation, and sampling intervals identical across both arms.

The 18,955,865-resource corpus in `MANUAL_TESTING_MATRIX.md` is intentionally excluded. Downloading, unpacking, importing, and reindexing it takes hours and measures many unrelated costs. The workspace all-features release build is also excluded because this change affects the R4 PostgreSQL path. The full corpus and all-features build remain release checks.

## Build each arm

From the exact baseline or candidate checkout:

```bash
CARGO_BUILD_JOBS=4 \
RUSTFLAGS='--cfg perf_phases' \
cargo build --locked --release -p helios-hfs --bin hfs \
  --no-default-features --features R4,postgres
```

Record the source SHA and binary SHA256 before moving or copying the binary. Keep both binaries so the trials do not rebuild between arms.

Use the candidate checkout's controller for both arms so the measurement code is identical. Set `ARM_REPO` to the exact baseline or candidate source checkout that produced the binary. After the build, compute that checkout's full source fingerprint. It includes `HEAD`, the binary form of every tracked change, and the path and content of every untracked file:

```bash
CONTROLLER=/absolute/path/to/candidate-checkout/crates/hfs/tests/bulk_submit/measure_memory.py
ARM_REPO=/absolute/path/to/baseline-or-candidate-source-checkout

python3 "$CONTROLLER" \
  --dry-run \
  --binary /absolute/path/to/hfs-baseline-or-candidate \
  --output-dir /tmp/hfs-1086/fingerprint-placeholder \
  --repo-root "$ARM_REPO" \
  --pg-container hfs-1086-postgres-reindex-pg \
  --database-url "$HFS_1086_DATABASE_URL" \
  --postgres-reindex-evidence \
  > /tmp/hfs-1086/source-plan.json

SOURCE_FINGERPRINT=$(python3 -c \
  'import json; print(json.load(open("/tmp/hfs-1086/source-plan.json"))["source"]["fingerprint_sha256"])')
shasum -a 256 /absolute/path/to/hfs-baseline-or-candidate
```

Do not edit the checkout or rebuild after this point. Every timed invocation verifies `SOURCE_FINGERPRINT`. The run records that fingerprint and the binary SHA256 together. This binds a trial to both files, although an external build system could still supply an unrelated binary under that path.

## Run one trial

Use a new output directory and a freshly restored database for every invocation:

```bash
python3 "$CONTROLLER" \
  --binary /absolute/path/to/hfs-baseline-or-candidate \
  --output-dir /tmp/hfs-1086/<arm>-trial-01 \
  --repo-root "$ARM_REPO" \
  --resources 2000 \
  --jobs 1 \
  --postgres-reindex-evidence \
  --expected-source-fingerprint "$SOURCE_FINGERPRINT" \
  --pg-container hfs-1086-postgres-reindex-pg \
  --database-url "$HFS_1086_DATABASE_URL" \
  --idle-seconds 0
```

Set `HFS_1086_DATABASE_URL` in the invoking shell; do not put the real password in notes or committed files. Use `--explain-analyze` only when the database is disposable and no competing measurement is running. It executes three bounded `SELECT ... LIMIT 100` cursor probes. Plain `EXPLAIN` is the default.

Add `--require-phase-summary` to every candidate trial. A pre-instrumentation baseline may omit it. The controller still completes functional validation when required measurement evidence is absent, but it sets `timings.comparable=false` and records each reason. Do not accept that trial for the six-row comparison.

For the #1086 profile, the controller enables `HFS_PERF_PHASES=1` and uses
`RUST_LOG=info,hfs=warn,hfs_perf=info`. The explicit `hfs_perf` directive is
required because tracing target filters match prefixes: `hfs=warn` alone also
matches `hfs_perf` and would suppress its INFO summary. A caller-provided
`--hfs-env RUST_LOG=...` is applied last and intentionally overrides this
default; if it filters out `hfs_perf`, a candidate run requiring the summary is
non-comparable.

The #1086 profile defaults PostgreSQL container sampling to 1 second and endpoint polling to 0.25 seconds. Keep those defaults in both arms. The controller parses the RFC3339 timestamp embedded in the correlated `deferred-index rebuild started` log line and pairs it with the wall-clock time when polling first observes completed status. It reports `start_log_to_completion_observed_interval_s` plus the endpoint polling resolution. This observational interval has no guaranteed error direction. Task spawning precedes the start log and can shorten the reported interval, while completion polling can lengthen it. It is not an exact database rebuild duration. A missing or invalid start timestamp makes the trial non-comparable.

Preflight blocks an existing `hfs`, `rustc`, or `cargo` process. It cannot detect a competing process that starts after preflight. Review the run's host load and memory evidence before accepting it, and reject the trial if a competing build or server appeared during the measured window.

The controller records:

- source revision, full source fingerprint, binary hash, controller hash, corpus hash, settings, host details, immutable PostgreSQL image ID, image name, and server version;
- kickoff to polling-observed terminal manifest publication, observed terminal to verified search readiness, and kickoff to verified readiness as separate durations;
- the start-log to completion-observation interval, with its polling resolution and stated uncertainty;
- throughput to terminal publication and throughput to verified search readiness as separate values;
- HFS RSS, PostgreSQL container memory, host memory pressure, swap, and load samples;
- `pg_stat_database` transaction, block, tuple, temporary-file, and WAL counter deltas for kickoff to polling-observed terminal, observed terminal to verified search readiness, and the whole kickoff-to-readiness window;
- early, middle, and late cursor plans for the reindex page query;
- the opt-in process-global reindex phase summary extracted from the HFS log;
- exact receipt, current-resource, history, version, indexed-search, and zero-unindexed-resource checks.

`pg_stat_statements` is not required. If an operator collects it separately, record whether the extension was already enabled and its reset point. Do not mix its numbers into trials that lack the same setup.

The terminal database snapshot is taken immediately after the controller has polled and validated the terminal manifest. It is an observed boundary, not the exact instant of publication, so its first interval may contain a small amount of reindex work that had already started. The readiness snapshot follows the status and SQL probes. Those probes contribute a little database work too. Therefore, none of the three deltas is exact DB-only reindex attribution. WAL and database counters can also include other sessions, which is why a dedicated PostgreSQL container and one job at a time are required.

## Candidate correctness and stable-ID reimport check

Run this small candidate-only check once, separately from the six fresh timed trials. Set `CANDIDATE_REPO` to the source checkout used for the candidate binary and set `SOURCE_FINGERPRINT` from that checkout as described above. The command submits the same 12 stable Patient IDs twice and leaves strict validation enabled:

```bash
python3 "$CONTROLLER" \
  --binary /absolute/path/to/hfs-candidate \
  --output-dir /tmp/hfs-1086/candidate-reimport-correctness \
  --repo-root "$CANDIDATE_REPO" \
  --resources 12 \
  --jobs 2 \
  --mode consecutive \
  --strict-validation \
  --postgres-reindex-evidence \
  --require-phase-summary \
  --expected-source-fingerprint "$SOURCE_FINGERPRINT" \
  --pg-container hfs-1086-postgres-reindex-pg \
  --database-url "$HFS_1086_DATABASE_URL" \
  --idle-seconds 0
```

Start from an empty database. Job 1 covers fresh import; job 2 reimports the same IDs. The controller's hard checks cover terminal manifests, receipts, current resources, incremented versions, history, zero unindexed resources, indexed family search, and verified reindex readiness after each job. Do not include its timings in the baseline/candidate performance comparison.

## Trial acceptance

A trial is comparable only when all of these are true:

- the database began from the agreed empty state;
- the manifest reached terminal status with no outcome or deleted entries;
- the correlated reindex job completed with `errorCount=0` and `processed=total`;
- SQL reports zero unindexed Patients;
- indexed family search returns all 2,000 resources;
- the run contains the source, binary, corpus, host, PostgreSQL, memory, database delta, and cursor-plan evidence;
- all three cursor plans parse, and both HFS and PostgreSQL have at least one memory sample inside the kickoff-to-readiness window;
- the run records whether a phase summary was available. A saved baseline binary built before the #1086 phase instrumentation may lack it. Record that limitation and do not compare internal phase totals across arms.

Reject and rerun a trial if another HFS, Cargo, or rustc process competed for the host, memory pressure became non-normal, the watchdog fired, or any hard validation failed.

## Results from the bounded local run

The six fresh trials run on 2026-09-12 passed every hard and soft validation and
met the comparability gates. These results apply only to the 2,000-Patient
workload described above; they do not establish behavior for the 10,000-resource
protocol cap or the 18.9-million-resource manual corpus.

| Arm | Trial | Source SHA | Source fingerprint | Binary SHA256 | Corpus SHA256 | Publish s | Search-ready s | Observed terminal-to-ready s | Start-log to completion-observed interval s | Resources/s to publish | Resources/s to ready | HFS peak MiB | PostgreSQL peak MiB | WAL bytes K-to-Tobs / Tobs-to-ready / K-to-ready | Commits K-to-Tobs / Tobs-to-ready / K-to-ready | Blocks read/hit K-to-Tobs / Tobs-to-ready / K-to-ready | Phase summary present | Cursor plans parsed | Comparable | Accepted |
|---|---:|---|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---|---|---|---|---|---|---|
| baseline | 1 | `2fa9dfaee722d1dad76acfb89c7b3d7e83c64bb9` | `a1c0ff03f553cc86ff1de017b9fd1370032162610109f91d2273ebeace0f3e36` | `3cc3dce622e8affb995208f70bb9297528d2feef3f90662c8d72315e90d21d70` | `73c86175d59c02915736be5074447e7789736771bea1e5f9c784f0119d2ce964` | 5.292 | 12.439 | 7.147 | 3.856 | 377.929 | 160.785 | 58.172 | 262.2 | 8,743,856 / 32,001,224 / 40,745,080 | 4,242 / 7,778 / 12,020 | 14/310,289 / 0/3,095,559 / 14/3,405,848 | no | 3/3 | yes | yes |
| baseline | 2 | `2fa9dfaee722d1dad76acfb89c7b3d7e83c64bb9` | `a1c0ff03f553cc86ff1de017b9fd1370032162610109f91d2273ebeace0f3e36` | `3cc3dce622e8affb995208f70bb9297528d2feef3f90662c8d72315e90d21d70` | `73c86175d59c02915736be5074447e7789736771bea1e5f9c784f0119d2ce964` | 5.300 | 12.139 | 6.839 | 3.565 | 377.358 | 164.758 | 57.922 | 265.1 | 8,365,720 / 32,322,024 / 40,687,744 | 4,290 / 7,692 / 11,982 | 14/320,212 / 0/3,104,623 / 14/3,424,835 | no | 3/3 | yes | yes |
| baseline | 3 | `2fa9dfaee722d1dad76acfb89c7b3d7e83c64bb9` | `a1c0ff03f553cc86ff1de017b9fd1370032162610109f91d2273ebeace0f3e36` | `3cc3dce622e8affb995208f70bb9297528d2feef3f90662c8d72315e90d21d70` | `73c86175d59c02915736be5074447e7789736771bea1e5f9c784f0119d2ce964` | 5.274 | 9.265 | 3.991 | 3.884 | 379.219 | 215.866 | 57.797 | 269.6 | 8,755,656 / 31,943,256 / 40,698,912 | 4,175 / 7,643 / 11,818 | 14/294,133 / 1/980,159 / 15/1,274,292 | no | 3/3 | yes | yes |
| candidate | 1 | `2fa9dfaee722d1dad76acfb89c7b3d7e83c64bb9` | `9b20d270862f3cf930fb2ffcbd9a74305c8af8ae7133a5df15ea9aa1c8cbd9ed` | `0a73991cacf645367d546cabbc6188ba6486a5348d4246dec4a74fb428c1ce78` | `73c86175d59c02915736be5074447e7789736771bea1e5f9c784f0119d2ce964` | 5.294 | 6.942 | 1.648 | 1.593 | 377.786 | 288.101 | 65.375 | 272.2 | 12,294,256 / 28,242,440 / 40,536,696 | 3,914 / 66 / 3,980 | 11/368,073 / 0/330,438 / 11/698,511 | yes | 3/3 | yes | yes |
| candidate | 2 | `2fa9dfaee722d1dad76acfb89c7b3d7e83c64bb9` | `9b20d270862f3cf930fb2ffcbd9a74305c8af8ae7133a5df15ea9aa1c8cbd9ed` | `0a73991cacf645367d546cabbc6188ba6486a5348d4246dec4a74fb428c1ce78` | `73c86175d59c02915736be5074447e7789736771bea1e5f9c784f0119d2ce964` | 5.265 | 6.879 | 1.614 | 1.519 | 379.867 | 290.740 | 64.797 | 298.3 | 14,977,872 / 28,349,832 / 43,327,704 | 3,897 / 78 / 3,975 | 11/309,773 / 0/411,906 / 11/721,679 | yes | 3/3 | yes | yes |
| candidate | 3 | `2fa9dfaee722d1dad76acfb89c7b3d7e83c64bb9` | `9b20d270862f3cf930fb2ffcbd9a74305c8af8ae7133a5df15ea9aa1c8cbd9ed` | `0a73991cacf645367d546cabbc6188ba6486a5348d4246dec4a74fb428c1ce78` | `73c86175d59c02915736be5074447e7789736771bea1e5f9c784f0119d2ce964` | 5.031 | 9.958 | 4.927 | 1.610 | 397.535 | 200.844 | 65.266 | 295.3 | 10,351,800 / 32,591,672 / 42,943,472 | 3,799 / 179 / 3,978 | 14/284,482 / 1/1,531,850 / 15/1,816,332 | yes | 3/3 | yes | yes |

### Aggregate comparison

Each aggregate cell is `median / minimum / maximum / absolute spread` across
the three fresh trials. The change is candidate median relative to baseline
median.

| Metric | Baseline | Candidate | Median change |
|---|---:|---:|---:|
| Publication (s) | 5.292 / 5.274 / 5.300 / 0.026 | 5.265 / 5.031 / 5.294 / 0.263 | -0.51% |
| Verified readiness (s) | 12.139 / 9.265 / 12.439 / 3.174 | 6.942 / 6.879 / 9.958 / 3.079 | -42.81% |
| Observed terminal to ready (s) | 6.839 / 3.991 / 7.147 / 3.156 | 1.648 / 1.614 / 4.927 / 3.313 | -75.90% |
| Start-log to completion-observed interval (s) | 3.856 / 3.565 / 3.884 / 0.319 | 1.593 / 1.519 / 1.610 / 0.091 | -58.69% |
| Throughput to publication (resources/s) | 377.929 / 377.358 / 379.219 / 1.861 | 379.867 / 377.786 / 397.535 / 19.749 | +0.51% |
| Throughput to readiness (resources/s) | 164.758 / 160.785 / 215.866 / 55.081 | 288.101 / 200.844 / 290.740 / 89.896 | +74.86% |
| HFS peak (MiB) | 57.922 / 57.797 / 58.172 / 0.375 | 65.266 / 64.797 / 65.375 / 0.578 | +12.68% |
| PostgreSQL peak (MiB) | 265.100 / 262.200 / 269.600 / 7.400 | 295.300 / 272.200 / 298.300 / 26.100 | +11.39% |
| Post-terminal WAL (bytes) | 32,001,224 / 31,943,256 / 32,322,024 / 378,768 | 28,349,832 / 28,242,440 / 32,591,672 / 4,349,232 | -11.41% |
| Post-terminal commits | 7,692 / 7,643 / 7,778 / 135 | 78 / 66 / 179 / 113 | -98.99% |
| Post-terminal block hits | 3,095,559 / 980,159 / 3,104,623 / 2,124,464 | 411,906 / 330,438 / 1,531,850 / 1,201,412 | -86.69% |

Publication was effectively unchanged, as expected for work performed after
deferred ingestion. Median verified readiness fell by 42.81%, median observed
post-terminal time fell by 75.90%, and median throughput to verified readiness
rose by 74.86%. The clearest database effect was transaction reduction: median
post-terminal commits fell from 7,692 to 78. Post-terminal block hits fell
86.69% and WAL fell 11.41%.

The tradeoff in this bounded run was higher measured peak memory: HFS increased
by 7.344 MiB at the median (+12.68%) and PostgreSQL by 30.2 MiB (+11.39%). These
are sampled process/container peaks, not allocation attribution. The candidate
also had wider PostgreSQL memory and post-terminal WAL variation.

Trial 3 was the readiness outlier in both directions: baseline trial 3 was
faster than its first two trials, while candidate trial 3 took 9.958 seconds to
verified readiness rather than about 6.9 seconds. Candidate internal phase WALL
remained stable at 1.48 seconds and its start-log-to-completion-observed interval
remained 1.610 seconds. The extra candidate trial-3 time coincided with the
readiness probes and 1,531,850 post-terminal block hits, versus 330,438 and
411,906 in trials 1 and 2. Baseline block-hit variation was also large. This
supports treating the readiness spread as observed end-to-end variation rather
than a change in the candidate's page algorithm.

The start-log interval is observational, not an exact rebuild duration. Task
spawn precedes the start log, completion is polled at 0.25-second resolution,
and the subsequent search/SQL readiness probes are outside that interval.

### Candidate phase attribution and cursor plans

Only the candidate can be compared internally. The saved baseline binary
predates the phase instrumentation, so setting `HFS_PERF_PHASES=1` could not
produce a baseline phase summary and no cross-arm internal-phase comparison is
made.

Across candidate trials, the page WALL was 1.43-1.45 seconds for 2,000 resources
over 20 pages. FTS accounted for 41.7-43.1% of WALL (42.0% median), grouped
search insertion for 29.5-30.2% (29.6% median), extraction for 12.8-13.9%
(13.6% median), search deletion for 7.1-7.2%, fetch for 2.6-2.7% (2.7%
median), commit for 2.4-2.5% (2.5% median), and grouped FTS deletion for
1.2-1.3%. Connection acquisition rounded to 0.0%. Each summary reported 46,000
search rows, or 23 per resource. These process-global counters are valid here
because only one job ran at a time.

All 18 representative cursor plans parsed. In both arms, early, middle, and
late probes used `idx_resources_search` with a backward index scan followed by
incremental sort. Total cost was 8.35 for early and 8.36 for middle/late. With
fetch contributing only about 2.7% of candidate WALL, this workload provides no
measured reason to change the cursor index order.

### Correctness reimport

The separate candidate run at
`/tmp/hfs-1086-postgres-reindex/final-candidate-reimport-12x2` submitted the same
12 stable IDs twice. Both jobs were verified: each reindex processed 12/12
resources with `errorCount=0` and created 276 entries. Validation observed
version distribution `{1: 12}` after the fresh import and `{2: 12}` after the
reimport, with 12 history IDs in both checks. All 32 checks passed with zero
hard and zero soft failures; receipt, content, indexed search, and zero-unindexed
checks also passed. This correctness run is not included in the timing
comparison.

### Recorded environment and provenance

- Baseline source fingerprint:
  `a1c0ff03f553cc86ff1de017b9fd1370032162610109f91d2273ebeace0f3e36`;
  candidate source fingerprint:
  `9b20d270862f3cf930fb2ffcbd9a74305c8af8ae7133a5df15ea9aa1c8cbd9ed`.
  Both report HEAD `2fa9dfaee722d1dad76acfb89c7b3d7e83c64bb9`. The
  candidate fingerprint was captured before this results writeback; the final
  source differs only by this evidence documentation, not executable code.
- Baseline binary SHA256:
  `3cc3dce622e8affb995208f70bb9297528d2feef3f90662c8d72315e90d21d70`;
  candidate binary SHA256:
  `0a73991cacf645367d546cabbc6188ba6486a5348d4246dec4a74fb428c1ce78`.
  The shared controller SHA256 was
  `369282c1a32271f1e724f3489549f6a8756f24268e79bdfa6ebcf6d23e64629f`.
- The six timed trials used 2,000 resources with corpus SHA256
  `73c86175d59c02915736be5074447e7789736771bea1e5f9c784f0119d2ce964`.
  The 12-resource reimport corpus was
  `a7143714f95ffe141e77fefd866eec539b161dbf42dbeee97d9a7e26a225b8c5`.
- Host: macOS 26.6.2 build 25G83, Apple M3 Max, 14 logical CPUs, 36 GiB
  (`38,654,705,664` bytes). PostgreSQL: 16.15 in `postgres:16-alpine`, image
  ID `sha256:cf78e76683b9ca8c5733cbbdce6c9262b45b6767934dd0a95e671f9a0fc20685`,
  container ID
  `fa8c6b5701bf1753a905db735d04f7c1bbe69d159ffcb33080f9eb1e6c6485d0`.
- Effective HFS settings were `HFS_STORAGE_BACKEND=postgres`,
  `HFS_DEFAULT_TENANT=default`, `HFS_DEFAULT_FHIR_VERSION=R4`,
  `HFS_BULK_SUBMIT_ENABLED=true`, worker concurrency `1`, maximum concurrent
  jobs per tenant `1`, file concurrency `1`, deferred indexing `true`, poll
  rate limit `1000000`, local-filesystem output, `HFS_PG_MAX_CONNECTIONS=4`,
  `HFS_MAX_PAGE_SIZE=1000`, auth disabled, audit backend `none`, server host
  `127.0.0.1` on port `18810`, log level `info`, `HFS_PERF_PHASES=1`, and
  `RUST_LOG=info,hfs=warn,hfs_perf=info`. Trials used fresh databases, one job,
  strict validation, plain `EXPLAIN`, no idle period, and no container CPU or
  memory limit.
- Timers were 180-second startup, 3,600-second manifest terminal, 900-second
  reindex, and 10,800-second whole-run deadline. Sampling was 0.5 seconds for
  HFS RSS, 1 second for Docker stats, 5 seconds for host vitals, and 0.25
  seconds for endpoint polling. The watchdog limit was 1,536 MiB HFS RSS, six
  pressure samples, and 256 MiB swap growth over three samples.

Raw evidence for this local run remains under these exact paths:

- `/tmp/hfs-1086-postgres-reindex/final-baseline-trial-01` through `03`;
- `/tmp/hfs-1086-postgres-reindex/final-candidate-trial-01` through `03`;
- `/tmp/hfs-1086-postgres-reindex/final-candidate-reimport-12x2`.

Those temporary directories are local and may be removed; this section is the
compact committed record of the accepted evidence.

### PostgreSQL and Elasticsearch composite smoke check

A separate bounded smoke check used an R4 development build with both the
PostgreSQL and Elasticsearch features, a dedicated PostgreSQL database, and
Elasticsearch 8.15.0. After loading eight Patient resources, a
`POST /Patient/$reindex` request with `batchSize=4` reached `Completed` and
reported `processed=8` of `total=8`, `entriesCreated=48`, and `errorCount=0`.
HFS family search through the Elasticsearch secondary returned a count of 8 and the Elasticsearch
document count was 8. This validates preservation of composite pg-es behavior;
it is not a performance measurement. Both services were stopped after the
check.
