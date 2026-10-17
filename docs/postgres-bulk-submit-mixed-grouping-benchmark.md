# PostgreSQL bulk-submit mixed-grouping benchmark

This protocol measures the private PostgreSQL optimization from issue #1456.
It compares the unchanged baseline at
`3c09d6a87a80a16d9cb4198fd9cafef3faea4e2e` with the candidate implementation.
No benchmark result is recorded here. The percentage thresholds below are stop
gates for a future run, not performance claims.

## Scope

The workload sends one NDJSON file containing 10,000 deterministic R4 Patient
resources. HFS processes it with one submit worker, file concurrency 1 and
100 resources per transaction. Search indexing is deferred until the manifest
finishes. Each trial therefore has exactly 100 ordered ingestion batches and a
separate search-ready boundary.

The seven database fixtures are:

| Fixture | Existing IDs in each 100-entry batch | Purpose |
|---|---:|---|
| `existing-000` | 0 | all-fresh control |
| `existing-001-edge` | 1 at offset 0 | primary performance gate |
| `existing-001-center` | 1 at offset 50 | primary performance gate with two fresh runs |
| `existing-010-center-clustered` | offsets 45 through 54 | one cluster with fresh runs on both sides |
| `existing-010-spaced` | offsets 0, 10, ..., 90 | ten separated existing IDs |
| `existing-050-alternating` | every even offset | worst-case alternating existing/fresh order |
| `existing-100` | 100 | all-existing control |

The controller runs three baseline and three candidate trials per fixture, for
42 timed trials. Order is counterbalanced by fixture: one member of each gated
pair uses `B,C / C,B / B,C`, while the other uses `C,B / B,C / C,B`. Thus the
0/100 controls and the edge/center one-existing fixtures give opposite arms
the two second-position runs. The dry-run records every position explicitly.
Fixtures with 0 or 100 existing IDs are not duplicated under another layout
because their clustered and interleaved forms are identical.

Only standalone PostgreSQL is measured. PostgreSQL plus Elasticsearch is
excluded because search offload disables the optimized path. This protocol
does not support claims about SQLite, MongoDB, S3 or composite deployments.

## Build both arms

Build before starting any trial. Use the same command, toolchain and build
environment for both source checkouts:

```bash
CARGO_BUILD_JOBS=4 cargo build --locked --release \
  -p helios-hfs --bin hfs --no-default-features --features R4,postgres
```

Keep both binaries. Do not rebuild while measuring. The controller requires
explicit paths for both binaries and both source checkouts. It records each
source HEAD, branch, full working-tree fingerprint, binary SHA-256, controller
SHA-256, corpus SHA-256 and R4 search-registry SHA-256. The baseline and
candidate registries must match.

The baseline checkout must have the exact base as `HEAD` and no tracked
changes. The candidate may have tracked changes, but the base must be an
ancestor of its `HEAD`. Before every measured arm and once after a successful
campaign, the controller rechecks each binary and registry hash, the controller
hash, and a source identity made from `HEAD` plus the complete tracked diff.
Untracked paths are content-hashed into the full worktree fingerprint and
recorded at every check, but are informational: new local evidence does not
change executable source identity and does not abort the campaign.
The output directory must remain outside both repositories so growing trial
artifacts are not mistaken for source evidence or repeatedly fingerprinted.

## Runtime isolation

The controller creates and owns these services:

- one PostgreSQL Docker container, published only on loopback;
- one in-process loopback HTTP provider for the manifest and NDJSON file;
- one native HFS process at a time.

The PostgreSQL container starts with
`shared_preload_libraries=pg_stat_statements`,
`pg_stat_statements.track=all` and
`pg_stat_statements.track_utility=on`. The last setting is required to observe
`SAVEPOINT`, `RELEASE SAVEPOINT` and `ROLLBACK TO SAVEPOINT` calls.

The controller initializes the schema once, then creates a deterministic dump
for each fixture. Before every arm and trial it restores that fixture into a
new, exclusive database. It snapshots statement statistics after HFS startup
and immediately before kick-off, then again at the first terminal `200`.
Queries are filtered by that database's OID. The controller stops its HFS
process after search readiness and before restoring the next trial. It removes
only the process and container identifiers it created.

A fresh restored database does not guarantee a cold operating-system page
cache. Keep the PostgreSQL container lifetime, host and arm order unchanged
for the whole campaign. Preserve the recorded order and raw evidence when
interpreting spread.

HFS runs from the selected arm's repository root so
`data/search-parameters-r4.json` is loaded. The fixed runtime settings are:

```text
HFS_STORAGE_BACKEND=postgres
HFS_DEFAULT_FHIR_VERSION=R4
HFS_BULK_SUBMIT_WORKER_CONCURRENCY=1
HFS_BULK_SUBMIT_FILE_CONCURRENCY=1
HFS_BULK_SUBMIT_MAX_CONCURRENT_PER_TENANT=1
HFS_BULK_SUBMIT_BATCH_SIZE=100
HFS_BULK_SUBMIT_DEFER_INDEXING=true
HFS_BULK_SUBMIT_SKIP_UNCHANGED=false
HFS_PG_MAX_CONNECTIONS=4
HFS_AUTH_ENABLED=false
HFS_AUDIT_BACKEND=none
HFS_UI_ENABLED=false
```

## Observations and correctness gates

The ingestion duration starts immediately before kick-off and ends when status
polling first observes a terminal `200`. Search-ready time uses the same start
and ends only after all of these checks reach 10,000:

- indexed `Patient?family=GroupingAfter` search;
- `_text=HeliosGroupingNarrative` full-text search;
- distinct Patient coverage in `search_index`;
- Patient rows in `resource_fts`.

HFS RSS and PostgreSQL container memory are sampled throughout the kick-off to
search-ready window. Raw start/completion timestamps and the maximum sampling
gap are retained. A comparable trial requires at least two samples, a first
sample near kick-off, an explicit endpoint sample near search readiness, no gap
above the recorded bound, both memory sources, and a terminated sampler
thread. PostgreSQL memory is reported separately and is not called HFS RSS.

`pg_stat_statements` deltas count these exact operation families:

- the tenant and paired `(resource_type, id)` classification query;
- `PostgresTransaction::read` point reads;
- `SAVEPOINT bulk_entry`;
- `RELEASE SAVEPOINT bulk_entry`;
- `ROLLBACK TO SAVEPOINT bulk_entry`;
- the grouped `INSERT_RESOURCES_SQL` statement.

Missing statement statistics, disabled utility tracking, or a count that does
not match the fixture invariant makes a trial non-comparable. It does not get
silently treated as a zero.

Every trial also requires:

- one ordered receipt reference for each input line;
- exact current Patient content and IDs;
- version 1 and exact `After` history content for fresh IDs;
- version 2 plus exact `Before` v1 and `After` v2 history content for
  pre-existing IDs, with R4 and non-deleted flags checked on every row;
- one correctly typed and manifest-scoped submission change per ID, including
  null previous fields for creates and exact rollback content for updates;
- terminal manifest counters of 10,000 processed, zero failed and zero skipped;
- no outcome or deleted artifact;
- successful indexed family and full-text searches after rebuild.

The run stops on the first correctness failure or non-comparable fixture. For
three accepted trials per arm, the controller reports median, minimum, maximum
and absolute spread. It applies these performance gates:

- candidate median ingest time must be at least 10% lower for both one-existing
  fixtures;
- candidate median ingest time and median peak HFS RSS may regress by at most
  5% for the all-fresh and all-existing controls.

The alternating fixture is reported without reordering input and has no
required speedup.

Performance gates are enabled only when every measurement-sensitive CLI field
is canonical: 10,000 resources, three trials, `postgres:16-alpine`, HFS pool 4,
poll interval 0.25 seconds and memory interval 0.5 seconds. `plan.json` records
the actual/canonical maps and every mismatch. A reduced or otherwise modified
configuration still enforces correctness and comparability but cannot produce
an acceptance verdict.

## Run

Inspect the complete default matrix without starting Docker or HFS:

```bash
crates/hfs/tests/bulk_submit/run_bulk_submit_postgres_grouping_benchmark.sh \
  --baseline-binary /absolute/baseline/target/release/hfs \
  --baseline-repo /absolute/baseline \
  --candidate-binary /absolute/candidate/target/release/hfs \
  --candidate-repo /absolute/candidate \
  --dry-run
```

The dry-run must report `matrix_trials: 42`. Run the full campaign with the
same command after removing `--dry-run`. The default evidence directory is
`/tmp/hfs-1456-pgi02-r1/benchmark` and must not already exist.

For a bounded harness check, use disposable binaries and reduce only the
corpus and repetition count:

```bash
crates/hfs/tests/bulk_submit/run_bulk_submit_postgres_grouping_benchmark.sh \
  --baseline-binary /absolute/baseline/target/release/hfs \
  --baseline-repo /absolute/baseline \
  --candidate-binary /absolute/candidate/target/release/hfs \
  --candidate-repo /absolute/candidate \
  --resources 100 --trials 1 \
  --output-dir /tmp/hfs-1456-pgi02-smoke
```

This reduced command validates the harness but cannot replace the 10,000-row,
three-trial acceptance campaign. Performance gates are enforced only for the
canonical 10,000-row, three-trial configuration; correctness and comparability
still stop a reduced run. Preserve `plan.json`, `results.json`,
`summary.json`, `summary.md`, per-trial logs, memory CSV files, statement
deltas, manifests and receipt-validation records from any claimed run.
`postgres.json` records the container/image IDs, server version and effective
statement-tracking settings so a tag change cannot silently alter the runtime.
`provenance-checks.json` records the enforced identity and informational full
worktree fingerprint before each arm and at successful completion.
