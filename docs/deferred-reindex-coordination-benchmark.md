# Deferred reindex coordination benchmark

This protocol measures issue #1087 on PostgreSQL. The coordination code is
shared by every backend that wires `ReindexOperation`, including standalone
SQLite, PostgreSQL, and MongoDB plus the Elasticsearch composites. Only
PostgreSQL performance claims belong in results produced by this protocol.

## Behavior under test

When deferred indexing is enabled, completed bulk-submit manifests request an
automatic full-type reindex. One `ReindexOperation` coordinates its own target
set in one process:

- One generation runs per tenant. Requests received during that generation add
  their resource types to one pending set.
- The process runs at most `W` automatic generations and retains state for at
  most `2W` tenants. `W` is the existing
  `HFS_BULK_SUBMIT_WORKER_CONCURRENCY` value. HFS adds no new environment
  variable for reindex coordination.
- A generation releases its execution permit before a pending generation
  acquires one. This lets another admitted tenant progress.
- Two `ReindexOperation` instances remain independent because they may have
  different writers or registries. An explicit `$reindex` job is also
  independent and may overlap automatic work.
- `Completed` with zero resource errors is a clean completion. A failed or
  errorful generation retries once with its active and pending types. A second
  failure abandons that generation and logs the manual `$reindex` repair.
  Independent work queued during the retry still runs as a new generation with
  its own retry budget.
- Cancellation does not retry the cancelled active types. Requests that were
  added independently while it ran remain pending. The next generation starts
  only after the cancelled task has stopped writing.

The state and job records are process-local. A restart loses pending automatic
work. Separate HFS processes do not coordinate with each other. Operators must
run `$reindex` after a restart or suspected cross-process race.

The implementation retains full-type scans. A finite burst can cause one
active scan and one accumulated follow-up scan. The project evaluated using
only successful manifest resource IDs, but deferred that option. A generic
implementation still needs bounded receipt paging and deduplication, current
resource reads, deleted or missing resource semantics, and consistent writes
to every composite target.

## Controller

[`measure_reindex_coordination.py`](../crates/hfs/tests/bulk_submit/measure_reindex_coordination.py)
drives four cases:

| Case | Requests | Purpose |
|---|---:|---|
| `combined` | one manifest with two files | Reference cost for one scan |
| `consecutive` | two stable-ID manifests after the first is searchable and idle | Reference cost for two intentional scans |
| `overlapping` | two manifests kicked off together | Pairwise coalescing behavior |
| `burst` | four manifests by default | Pending-set accumulation under load |

Within a case, every manifest uses the same tenant and resource type. Each case
gets a separate tenant so prior full-type scans do not inflate later scan
totals. Resource IDs and indexed family values are unique to the run and case.
The controller first creates a bounded number of the resources through the REST
API with an obsolete family name, then imports their replacement values through
`$bulk-submit`.

The controller reads physical job IDs from HFS INFO logs and polls
`$reindex-status` on the same node. It records:

- physical jobs and scans;
- each job's total resources, processed resources, index entries created, and
  error count;
- the maximum number of jobs observed active during 100 ms sampling;
- kickoff to verified indexed-search readiness;
- terminal manifest polling and the correlated reindex log lines.

`reported_resource_totals` is the sum of each physical job's `total` value.
`entries_created` is the reported writer output. These are application-level
counts, not PostgreSQL buffer or WAL counters. The overlap sample is useful for
measurement, while the deterministic PostgreSQL integration test provides the
race-proof acceptance check.

The controller does not use `HFS_PERF_PHASES`. Those counters are global to the
process and cannot separate overlapping jobs. It also does not manage HFS,
PostgreSQL, or any other process. It stops only its own in-process fixture HTTP
server and never removes resources.

## Prepare each arm

Build baseline and candidate binaries from their exact source revisions. Use
an immutable detached checkout for the baseline and a clean issue checkout for
the candidate. Use the same Rust toolchain and flags for both:

```bash
CARGO_BUILD_JOBS=4 cargo build --locked --release -p helios-hfs --bin hfs \
  --no-default-features --features R4,postgres
sha256sum target/release/hfs
git rev-parse HEAD
```

Use a fresh PostgreSQL 16 database and a new HFS process for every trial. Run
HFS natively so it can reach the controller's loopback fixture server. Save the
server log outside the repository. The controller requires authentication to
be disabled and INFO events from `helios_persistence` to be present.

Example environment for the main `W=2` trial:

```bash
mkdir -p /tmp/hfs-1087/candidate-trial-01
RUST_LOG=info \
HFS_STORAGE_BACKEND=postgres \
HFS_DATABASE_URL="$HFS_1087_DATABASE_URL" \
HFS_DEFAULT_FHIR_VERSION=R4 \
HFS_AUTH_ENABLED=false \
HFS_BULK_SUBMIT_ENABLED=true \
HFS_BULK_SUBMIT_DEFER_INDEXING=true \
HFS_BULK_SUBMIT_POLL_RATE_LIMIT=100000 \
HFS_BULK_SUBMIT_WORKER_CONCURRENCY=2 \
HFS_BULK_SUBMIT_MAX_CONCURRENT_PER_TENANT=4 \
HFS_PG_MAX_CONNECTIONS=8 \
HFS_SERVER_HOST=127.0.0.1 \
HFS_SERVER_PORT=18087 \
./hfs > /tmp/hfs-1087/candidate-trial-01/hfs.log 2>&1
```

Keep the password in the invoking shell. Do not copy it into evidence. Record
the PostgreSQL image digest, server version, HFS binary hash, source commit,
worker concurrency, connection pool size, host load, and trial start time next
to the controller output.

## Run a trial

Choose a new output directory. The controller refuses an existing path so it
cannot overwrite prior evidence:

```bash
python3 crates/hfs/tests/bulk_submit/measure_reindex_coordination.py \
  --base-url http://127.0.0.1:18087 \
  --hfs-log /tmp/hfs-1087/candidate-trial-01/hfs.log \
  --output-dir /tmp/hfs-1087/candidate-trial-01/evidence \
  --resources-per-manifest 200 \
  --preexisting 20 \
  --burst-manifests 4
```

Run at least three fresh trials per arm. Alternate baseline and candidate
trials. Run a second candidate pass with
`HFS_BULK_SUBMIT_WORKER_CONCURRENCY=1`. The controller starts every manifest in
an overlapping or burst case concurrently, so the PostgreSQL pool must also
have enough capacity for those HTTP kickoff requests. A `503 connection pool
exhausted` response means the trial did not reach the coordinator and is not a
valid low-pool measurement.

The pre-change baseline has no coordinator. Its overlap check may fail and the
controller may exit with status 1 after writing complete evidence. Preserve
that output. Status 2 means the procedure itself failed, such as a missing log,
HTTP error, or timeout.

## Accept or reject a trial

A candidate trial passes when every scenario reports:

- all submission manifests are clean;
- every observed physical job completed with no errors;
- indexed family search returns the exact number of unique current resources;
- no two automatic jobs for the tenant were observed active together;
- at least one physical job was correlated from the log.

Also run the deterministic PostgreSQL integration test. It locks the real
`search_index` table after generation one has fetched an old Patient value,
updates that Patient, and enqueues generation two. It proves the requests do
not overlap and that the follow-up leaves one fresh family row and one FTS row:

```bash
newgrp docker <<'EOF'
CARGO_BUILD_JOBS=4 cargo test --locked -p helios-persistence \
  --no-default-features --features R4,postgres \
  --test postgres_tests postgres_integration_deferred_reindex_coordination \
  -- --nocapture
EOF
```

Reject a performance trial if another HFS instance or database workload used
the measured PostgreSQL server, the server restarted, a status response came
from another node, any request timed out, or search readiness was not verified.
Do not generalize the resulting latency or work-count changes to SQLite,
MongoDB, S3 sources, or Elasticsearch targets without separate measurements.
