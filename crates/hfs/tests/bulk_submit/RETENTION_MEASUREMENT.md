# #995: repeated-job retention follow-up

Diagnostic experiment, not a production fix. Baseline source is
`1386c0ef4c2e8969a1d90b6256772bdc6db1a408`. The earlier RSS campaign is documented
in MEMORY_RESULTS.md. Its binary is retained separately.

## Predictions

1. Without cleanup, each successful deferred manifest adds one finished reindex
   job and one closed cancellation sender to the process-owned manager maps.
   Twenty jobs should leave twenty entries; restarting HFS should leave one.
2. If additional owners grow, malloc-visible retained bytes may increase beyond
   the directly accounted manager fields. Map inventory does not measure channel
   internals, allocator overhead or HashMap bucket allocations.
3. OS residence can change while malloc-visible bytes stay stable. Compare RSS,
   physical footprint and heap separately, without claiming RSS is live memory.

## Instrumentation

A temporary environment-gated probe in ReindexOperation::get_progress emits
`[DEBUG-995-retention]` inventory lines. It reports map lengths/capacities, finished
jobs, closed senders, occupied inline entry bytes and owned String/Vec buffer
capacities. It does not run cleanup. The last two byte counts are accounting of
specific fields, NOT allocator measurements or a complete manager size.

The patch and source are retained in `target/issue-995-retention/`. Build once,
release R4/Postgres, CARGO_BUILD_JOBS=1, with host monitoring. Preserve the binary
hash and restore the uninstrumented source after collecting evidence. Each arm
uses the same instrumented binary and settings. Polling also emits inventory;
the authoritative sample is a separate status request AFTER idle, correlated by
job ID and log offset, before heap inspection and validation.

## Arms and budget

One pilot followed by two serial arms: twenty submissions of the same 1,000
synthetic Patient IDs, four input files, deferred indexing, one worker, one file
task, PG pool four. First job creates, later jobs replace and grow exact
versions/history. Fresh database per arm; restart only HFS between jobs in the
restart arm, preserving database state and job ordinal.

After verified reindex completion, idle 30 seconds each job. This differs from
the previous campaign's 60 seconds and is not pooled with it. Snapshot startup
once, then post-idle at jobs 1, 5, 10, 15 and 20. Each snapshot uses footprint,
vmmap summary and heap summary sequentially, with a 15-second tool timeout.
Any inspection failure aborts; artifacts remain. No allocation stack logging.

Use the same 1 CPU / 768 MiB / no-swap PostgreSQL container budget, dedicated
loopback port and existing cached image as before. Never touch unrelated
containers. No builds overlap a measurement. RSS and host watchdogs remain the
same as measure_memory.py. Abort on failed data/index/receipt validation. The
experiment does not include immediate-next-job/reindex overlap or injected errors.

## Command

Provision the dedicated fresh database first. The driver requires the temporary
probe binary; an ordinary binary cannot produce the inventory and will fail.

```sh
python3 crates/hfs/tests/bulk_submit/measure_retention.py \
  --binary target/issue-995-retention/hfs-instrumented \
  --output-dir target/issue-995-retention/NEW_ARM \
  --resources 1000 --jobs 20 --mode consecutive --defer-indexing true \
  --idle-seconds 30 --file-concurrency 1 \
  --hfs-env HFS_995_RETENTION_PROBE=1 \
  --pg-container BENCHMARK_CONTAINER \
  --database-url postgres://postgres@127.0.0.1:56810/FRESH_DATABASE
```

`retention-inventory.jsonl` holds post-idle manager samples. `retention/` holds
raw snapshots, tool exit codes/timings and the diagnostic driver hash. Standard
controller artifacts retain full validation and environmental evidence. Snapshot
inspection can page memory in, and validation affects the next job; both arms
apply the same ordinal schedule. Post-idle RSS is sampled before these probes.
