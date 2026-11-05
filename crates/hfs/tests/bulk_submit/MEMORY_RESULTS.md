# Issue #995: current-code measurements, 2026-09-10

The four reference series completed 20/20 jobs in successful attempts. Production
processing code is unchanged. A separate three-job allocation snapshot pass also completed all validations.

## Baseline and build

Source `1386c0ef4c2e8969a1d90b6256772bdc6db1a408`; native macOS ARM64,
16 GiB RAM, eight logical CPUs. Release R4 + PostgreSQL, no default features,
one Cargo build job. Rust 1.98.0. Binary SHA-256
`ed8247ed85faf069c51aadb16502d49eeb92bba1cf4071566394aa90243153d8`.
Build took 34m18s; maximum RSS reported by `/usr/bin/time -l` was
3,936,436,224 bytes (3.67 GiB). No new host swapouts during the build.
No builds overlapped ingestion measurements.

Dedicated PostgreSQL 16.14 container: one CPU, 768 MiB RAM, no container swap,
128 MiB shared buffers. Existing unrelated containers were left running.
Exact image identity is retained in `target/issue-995/postgres-identity.json`.
All corpora are synthetic. Each series starts with a fresh database; later jobs
reimport identical IDs/content and grow versions/history. Restart arms preserve
that database between HFS processes. One worker, one file task, four input files,
PG pool four, effective ingestion batch 100. See [protocol](MEMORY_MEASUREMENT.md).

## Validated initial observations

Both indexing modes completed five-job continuous/restart pairs (20 jobs). Each submission contains
12,000 Patients; receipt references, full current content, exact versions/history,
index coverage and parameterized search passed; deferred runs also required a
completed background reindex with exact totals and zero errors.
Post-job observations wait for indexing and 60 seconds of idle; full downloads
and validation follow that window.

| Configuration | Post-idle RSS, jobs 1–5 (MiB) | Maximum sampled RSS (MiB) |
| --- | --- | ---: |
| Deferred, same HFS process | 60.1, 28.9, 31.3, 31.4, 31.4 | 60.2 |
| Deferred, restart HFS per job | 58.2, 58.1, 58.0, 57.9, 57.9 | 59.8 |
| Inline, same HFS process | 59.3, 59.6, 59.6, 58.1, 59.7 | 59.7 |
| Inline, restart HFS per job | 58.3, 57.6, 57.6, 57.9, 57.8 | 58.3 |

These runs do not show monotonically increasing resident memory. They do not
prove the absence of retained allocations or establish a live-heap bound.
In the continuous run, RSS fell by approximately 34 MiB during job 2 ingestion,
with unchanged virtual size and increased host compression. The event occurred
before submit completion, so it cannot be attributed to job-terminal cleanup.
Host compression is system-wide evidence, not a measurement of compressed HFS
pages. Clean-page reclamation, anonymous-page compression and allocation release
remain possible explanations. Earlier Linux raw/buffered results are not direct
comparison arms for this native macOS baseline.

All four successful reference series kept host pressure at level 1 and had zero
new host swapouts; maximum within-job sampling gap rounded to 0.6 seconds.

## Attempts and evidence

Raw evidence is local under `target/issue-995/` (ignored build directory):

- `build.log`, `build-memory.jsonl`, `build.json`: successful constrained build.
- `pilot-1000-a`: controller initialization failure before HFS startup.
- `pilot-1000-b`: ingestion completed, reindex UUID parser failed; interrupted,
  not a verified measurement.
- `pilot-1000-c`: successful 1,000-resource pilot, all validation passed.
- `12k-consecutive-deferred`, `12k-restart-deferred`: completed reference pair.
- `reference-comparison.md`: all four series, per-job measurements and sampling quality.
- `12k-consecutive-inline-v2`, `12k-restart-inline-v3`: completed inline pair.
- `12k-restart-inline-v2`: aborted after the first submit. SQL showed 12,000
  Patients and zero Patients without index rows, but the HTTP search check returned
  no valid total after about 30 seconds. The original controller discarded HTTP
  error details, so timeout source/cause is unconfirmed. A fresh repeat with the
  same HFS settings and added HTTP evidence logging passed all five jobs. This
  failed attempt remains excluded; it is not evidence of a wrong resource count.
- `12k-consecutive-inline`: submit completed but controller coverage query hit
  its 120-second timeout; series failed before full validation, excluded from
  valid comparisons. A query was still active after the client timeout.
- `measure_memory-reference-v1.py`: controller used for the deferred pair and
  failed first inline attempt. Subsequent controller replaces correlated
  `NOT EXISTS` coverage with equivalent filtered ID-set `EXCEPT`, and sets a
  PostgreSQL statement timeout below the client timeout. This changes observation
  overhead, not the HFS binary. Treat cross-indexing timing comparisons cautiously.
- `task-inspection-preflight`: footprint/vmmap/heap succeeded on an owned temporary
  sleep process; this is tool-access evidence, not HFS allocation evidence.

## Supplemental allocation snapshots

Same binary and deferred configuration, fresh database, three 12,000-resource
jobs. Sequential footprint/vmmap/heap snapshots were taken before kickoff and
after the idle marker, before validation. All 18 tool calls succeeded, each under
1.25 seconds. Tools can fault pages into residency; these timings and RSS values
are not pooled with the reference runs. Driver identity and source are retained
in `target/issue-995/diagnostic-driver-identity.json` and `diagnostic_driver.py`.

| Post-idle snapshot | malloc blocks | malloc bytes | Physical footprint reported by heap |
| --- | ---: | ---: | ---: |
| Job 1 | 56,034 | 5,091,328 | 19.2M |
| Job 2 | 56,276 | 5,082,576 | 19.4M |
| Job 3 | 56,360 | 5,106,752 | 19.4M |

The initial pre-kickoff snapshot had 55,366 blocks / 5,153,680 malloc bytes.
Post-idle bytes vary by approximately 24 KiB across the three jobs; block count
increases modestly. This does not demonstrate linear retained-byte growth, and
cannot assign those blocks to reindex metadata, caches or other owners. `heap`
reports malloc-visible allocations, not every form of process memory. No allocation
backtraces were enabled. The original ~34 MiB RSS fall did not recur, so these
snapshots cannot resolve its cause retrospectively.

Raw snapshots, timestamps, exit codes and parsed totals live under
`target/issue-995/12k-diagnostic-deferred/diagnostic/`; the complete run passed
with normal pressure and no new swapouts. The benchmark HFS processes and provider
were shut down by the controller. The dedicated PostgreSQL container was stopped
and auto-removed after evidence collection; its disposable databases are gone.
Logs, generated fixtures, receipts and validation evidence remain on disk.

Controller revisions are preserved as `measure_memory-reference-v1/v2/v3.py`.
After the campaign, the reusable controller also gained server-side timeout
protection for its streamed SQL validation and linear-time duplicate detection
for receipts. Those final maintenance edits were syntax/help/dry-run checked;
they were not used for the recorded campaign and did not change production code.

## Scope still unresolved

Whole-manifest receipt accumulation can create transient memory proportional to
receipt count; streaming remains coordinated with #982. Static inspection also
found retained reindex job/cancellation metadata with no cleanup caller. Neither
owner is quantified by these RSS observations. Small repeated Patient imports,
quiescent scheduling and five jobs cannot establish behavior for large/mixed
resources, overlapping reindexes or long-running production workloads.


## Decision and next experiment

Keep #995 open. This is a current-code baseline under constrained, quiescent
settings, not a finding that production retention is solved. Do not compare its
absolute RSS with historical Linux measurements. No larger run was launched:
48k/192k escalation was conditional, and more RSS-only volume would not settle
ownership of retained allocations or the OS residency event.

Next, use a separate instrumented experiment to measure receipt accumulation,
serialization/output and release boundaries, coordinated with #982. For true
cross-job retention, attribute allocations to owners and measure reindex job/map
cardinality over more small consecutive jobs, with a restart control. Keep job
counts and pending reindexes explicit. Investigate the intermittent post-submit
search-check failure separately with retained HTTP error details and query plans;
the first failure lacks those details and cannot support a root-cause claim.
