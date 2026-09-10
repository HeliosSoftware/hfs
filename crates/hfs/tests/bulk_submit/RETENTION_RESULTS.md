# #995 retention ownership follow-up — 2026-09-10

Completed: forty validated jobs plus one validated pilot. The process retains one
finished reindex job and one closed cancellation sender per successful deferred
manifest; the restart control stays at one of each. This confirms an ownership
and cleanup issue, but does not explain large transient memory peaks.
Protocol: [RETENTION_MEASUREMENT.md](RETENTION_MEASUREMENT.md).

## Build and provenance

Baseline source `1386c0ef4c2e8969a1d90b6256772bdc6db1a408`. Temporary observation
patch adds manager inventory to get_progress, gated by HFS_995_RETENTION_PROBE=1.
It does not remove entries, close senders or otherwise change ingestion semantics.

Incremental release R4/Postgres build, one Cargo job: Cargo 2m42s; wrapper 166s.
`time -l` maximum RSS: 1,301,331,968 bytes; sampled compiler-tree maximum: 1,400.7 MiB.
These are different accounting methods. Host pressure stayed normal and no new
swapouts occurred. No build overlapped a workload.

Instrumented binary SHA-256:
`7833c66fc4887da2e313a08805e62e935b64af0625c719c228173cd03dbf57a7`.
Patch SHA-256:
`129d969edc17e61d1281bd9173d313c534e90ccdb9d56be00d558a36bdf72b16`.
Original binary and temporary patch are retained in `target/issue-995-retention/`.

Dedicated PostgreSQL 16.14 (same immutable image as the first campaign), one CPU,
768 MiB, no container swap. Its own databases are disposable. Existing unrelated
services remain running. Full identity and resource settings: `postgres.json`.

## Scope

One validated pilot (1,000 Patients), then twenty 1,000-resource jobs per arm:
continuous HFS process versus restart per job, preserving DB/history by ordinal.
Every job waits for verified reindex completion and thirty seconds of idle, then
records manager inventory. Malloc/footprint/vmmap snapshots at startup and after
jobs 1, 5, 10, 15 and 20. All validations remain mandatory. The 30-second idle
interval differs from the preceding campaign's 60 seconds.

## What the byte counters mean

On this build each retained entry contributes 232 bytes of occupied Rust tuple
storage across the two maps and 172 bytes of owned String/Vec capacities in a
successful, error-free job: 404 accounted bytes per job. This excludes actual
HashMap bucket allocation, channel internals, allocator rounding and headers.
It is neither a complete manager size nor an estimate of RSS. Closed senders show
that receivers were dropped even though the manager still owns the sender.


## Continuous process: completed observations

All twenty submissions passed exact data/version/history/receipt validation,
search coverage and completed reindex totals with zero errors. Every post-idle
inventory matched `jobs_len = finished_jobs = channels_len = closed_channels =
job ordinal`. Capacities stepped 3 → 7 → 14 → 28 and did not shrink.

| Post-idle job | Jobs / closed senders | Accounted entry + buffer bytes | Malloc blocks | Malloc bytes | Footprint reported by heap |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 | 1 / 1 | 404 | 56,023 | 5,072,304 | 23.2M |
| 5 | 5 / 5 | 2,020 | 56,433 | 5,078,800 | 17.7M |
| 10 | 10 / 10 | 4,040 | 56,509 | 5,095,488 | 17.7M |
| 15 | 15 / 15 | 6,060 | 56,562 | 5,096,432 | 17.7M |
| 20 | 20 / 20 | 8,080 | 56,607 | 5,109,344 | 17.7M |

Post-idle malloc delta job 1 → 20: +37,040 bytes (+584 blocks). These aggregate
snapshots do not identify the owners of the full delta. The map counts establish
retention of specific objects; their partial byte accounting must not be subtracted
from total malloc growth to claim an exact remainder for other owners.

Raw evidence: `target/issue-995-retention/consecutive-20/`; per-job inventory/heap
summary `consecutive-summary.md`; RSS summary `consecutive-rss.md`.


## Restart control: completed observations

All twenty jobs passed the same 320 hard checks as the continuous arm. Each
post-idle inventory has one finished job, one closed sender, map capacities 3/3
and 404 accounted bytes. Databases preserve versions/history between restarts.

| Post-idle job | Malloc blocks | Malloc bytes | Footprint reported by heap |
| --- | ---: | ---: | ---: |
| 1 | 56,022 | 5,031,504 | 22.2M |
| 5 | 54,968 | 4,765,696 | 20.7M |
| 10 | 54,963 | 4,759,328 | 20.1M |
| 15 | 54,963 | 4,763,488 | 16.3M |
| 20 | 54,962 | 4,761,984 | 20.3M |

Later restarted processes are lower than the first, which initialized the fresh
database. Restart resets all process state, so neither the approximately 0.3 MB
between-arm heap difference nor the first-to-last restart decrease can be assigned
solely to reindex metadata. Across restart checkpoints 5–20, malloc bytes vary by
6,368 bytes. Continuous checkpoints 5–20 increase by 30,544 bytes. These are
aggregate observations with no allocation backtraces, not a measured leak rate.

RSS after idle: continuous job 1 → 20 was 56.4 → 56.7 MiB; restart was
55.4 → 54.6 MiB. A sharp RSS fall also occurred during restarted job 6, which
had only one job's process state. This further demonstrates why RSS changes
cannot by themselves identify retained manager objects. It does not identify
the OS mechanism of that fall. Footprint values above retain the tool's units.

## Conclusions and limits

1. The retained owners are confirmed: ReindexOperation.jobs keeps terminal
   ReindexProgress values, and cancel_channels keeps Senders after their receivers
   close. In the current source, start inserts entries; terminal paths update
   status but do not remove entries. cleanup_old_jobs is the sole removal path
   and has no callers in the repository. There is no effective automatic bound
   on this metadata accumulation in the examined path.
2. The measured successful-job cost is small in this experiment. Twenty retained
   records account for 8,080 bytes in the specifically measured fields; total
   malloc-visible bytes grew by 37,040 bytes from the first post-idle snapshot.
   These numbers are distinct observations. Do not extrapolate the aggregate
   malloc delta as a per-job rate or claim it fully measures the manager.
3. Error-free 1,000-resource imports do not exercise large retained error vectors,
   overlapping reindexes, other resource shapes, or a long process lifetime.
   No cleanup intervention or allocation-stack experiment was run, so attribution
   of the entire heap delta remains unresolved. The original bulk-submit peaks
   remain a separate question; #995 should remain open.

A scoped lifecycle fix should consider releasing cancellation senders once the
background task has actually returned, and bounding terminal status retention
with an explicit policy. Removing status immediately would break polling; active
jobs must survive any cleanup. That fix needs completion/failure/cancellation and
status-retention tests plus a repeat of this inventory experiment. No such fix
was applied in this campaign.

The next peak-focused experiment should instrument receipt accumulation,
serialization, output and release boundaries, coordinated with #982. Increasing
volume without those boundaries would again leave phase ownership unresolved.

## Evidence, verification and cleanup

Raw campaign root: `target/issue-995-retention/` (local ignored artifacts).
`comparison.md` contains all forty inventory rows and twelve allocation snapshots;
`rss-comparison.md` contains phase RSS, sample gaps and environment observations.
`audit.json` checks run status, all 656 hard checks across both arms and pilot,
all 41 post-idle inventories, and all 42 successful inspection tool calls. No
attempt failed in this campaign. Host pressure remained normal and no new
swapouts occurred. `build.json`, `build-memory.jsonl`, `instrumentation.patch`,
`reindex-instrumented.rs`, original source/binary and both driver hashes retain
provenance; `postgres.json` records the dedicated database image and limits.

The controller stopped its HFS processes and fixture providers. Only the dedicated
PostgreSQL container was stopped and auto-removed; its disposable databases were
removed with it. Fixtures, receipts, SQL validation results and snapshots remain.
The exact uninstrumented reindex.rs and original target/release/hfs were restored
and verified; `restoration.json` records the baseline binary hash. No production
source change remains, and no commit, push or issue comment was made. The diagnostic
scripts and reports remain in the workspace for review/reuse.
