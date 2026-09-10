# Bulk-submit memory investigation (#995)

This experiment establishes a new baseline for the current source. The earlier
raw/buffered measurements are historical context, not comparison arms. Production
processing code is unchanged. Initial measurements are external RSS observations,
not live-heap measurements or evidence of a leak.

## Build and resource budget

The initial host is macOS ARM64, 16 GiB RAM, eight logical CPUs. Docker shares
the host with existing services. Build HFS natively and run only the dedicated
benchmark PostgreSQL in Docker. Do not stop or reconfigure existing services.

```sh
CARGO_BUILD_JOBS=1 cargo build --locked --release \
  -p helios-hfs --bin hfs --no-default-features --features R4,postgres
```

Retain the target directory and record the source revision, binary SHA-256,
compiler, features, database image identity and effective runtime configuration.
Never build while measuring. A single build job reduces parallel compiler memory;
it does not cap one compiler process. Monitor host memory and swap during the build.

Use one dedicated PostgreSQL container, initially limited to one CPU and 768 MiB,
with swap disabled for that container. Use a disposable database per series and
retain the evidence before removing benchmark-owned infrastructure. HFS starts
with one submit worker, one file task and a small explicitly configured database
pool. These are resource-constrained settings, not an assertion of default-server
throughput. The current worker uses an effective batch of 100 regardless of the
configured bulk-submit batch-size setting.

## Stages

1. Pilot: 1,000 deterministic Patient resources in four plaintext NDJSON files.
2. First series: 12,000 resources, five consecutive submissions in one HFS process,
   compared with five submissions with HFS restarted each time.
3. Increase to 48,000 and 192,000 only if the smaller series leaves resource
   headroom and further volume addresses an unresolved question.

Reimports use identical resource IDs. The first job creates resources; subsequent
jobs update them and grow history and submission bookkeeping. Match job ordinal
and database state between process-lifetime arms. Do not call later reimports
fresh jobs. A fresh-ID series is a separate experiment with growing current data.

The first measurements use deferred indexing, the current runtime default. Record
submit-terminal and actual reindex-terminal separately. Existing searchable data
means a matching search count alone cannot prove a reimport's reindex finished.
Then repeat with deferred indexing disabled if resources permit.

These are quiescent consecutive jobs: each next submission waits for background
indexing, idle observation and validation to finish. They do not measure the
overlap possible when a client submits the next job immediately after a terminal
submit response while the preceding reindex is still active.

## Observation and validation

- Sample HFS RSS separately from PostgreSQL/container memory. Docker VM residency
  and host memory compression are not PostgreSQL process RSS.
- The PostgreSQL stats collector runs separately from the RSS/watchdog thread.
  Retain actual sample timestamps and report gaps instead of assuming the target
  0.5-second cadence was achieved perfectly.
- Retain wall-clock and monotonic timestamps, PIDs, external phase markers, host
  swap/VM counters and all failed or interrupted attempts.
- Observe startup, pre-kickoff, submit-terminal, reindex-terminal, and a fixed
  60-second idle interval. External markers do not distinguish internal receipt
  readback, serialization and writes; a later instrumentation pass is needed
  for that attribution.
- Validate resource content, versions/history, counts and receipt references.
  Perform downloads and full validation after the idle window, labelling that
  phase so it cannot be mistaken for ingestion or post-job retention. Such
  validation still affects the next job's process/cache state; apply it equally
  to both arms and record the next pre-kickoff baseline.
- Keep the standard R4 search-parameter data available. Record the startup registry
  count so a fallback registry cannot silently turn this into a smaller workload.
- Treat sustained new swap activity or rising host memory pressure as a reason
  to stop escalation. A pre-existing swap allocation alone is not a stop signal.
  Container OOM, timeout and host-pressure interruptions remain reported attempts.

## Interpretation

RSS growth identifies a question, not its owner. Follow persistent growth with
live-allocation evidence on the smallest reproducible series. Distinguish
temporary receipts, application caches, reindex job metadata, driver buffers and
allocator/OS residency. Do not infer allocator implementation from the absence of
a Rust global-allocator override. No historical RSS median is a pre-job baseline.

The initial campaign does not change receipt streaming, output buffering,
indexing, batch-size propagation or job cleanup. Each confirmed defect needs its
own scoped follow-up; receipt streaming remains coordinated with #982.

## Running the controller

Provision a dedicated PostgreSQL container/database before invoking the controller;
it never creates, deletes or reconfigures the supplied database/container. The
database must be fresh at series start. Reuse its state within the series even
when `--mode restart` restarts HFS between jobs.

```sh
python3 crates/hfs/tests/bulk_submit/measure_memory.py \
  --binary target/release/hfs --output-dir target/issue-995/new-series \
  --resources 12000 --jobs 5 --mode consecutive --defer-indexing true \
  --idle-seconds 60 --file-concurrency 1 \
  --pg-container BENCHMARK_CONTAINER \
  --database-url postgres://postgres@127.0.0.1:BENCHMARK_PORT/BENCHMARK_DATABASE

python3 crates/hfs/tests/bulk_submit/summarize_memory.py \
  target/issue-995/new-series
```

`--dry-run` prints configuration without launching anything. Each output directory
must be new. `run.json`, phase/host/RSS/container CSV files, logs and validation
results remain on failure. A root-managed local trust-auth database was used for
the initial run; it contains only generated fixtures. HFS's URL-bearing startup
info log is filtered with `RUST_LOG=info,hfs=warn`; persistence registry/reindex
markers remain enabled. Other inherited `HFS_*` settings are removed.
