# PostgreSQL grouped-create benchmark for issue #1455

This protocol measures the standalone PostgreSQL `$bulk-submit` fresh-create
path. It is intentionally separate from the general memory and deferred-index
evidence profiles: the new controller behavior is enabled only by
`--postgres-grouped-create-evidence`.

## What is compared

The runner normally builds fresh release binaries and executes the same
candidate-branch controller against three source arms:

- issue source: `8b127592b30083946feb54cf530a26aebf1f3940`, in a temporary
  detached worktree
- current main: `3c09d6a87a80a16d9cb4198fd9cafef3faea4e2e`, in a temporary
  detached worktree
- candidate: the issue worktree that contains this runner, including tracked
  and untracked changes

On the normal build path, all arms use their own fresh `CARGO_TARGET_DIR`. The
runner records and removes only the two detached worktrees it creates, on both
successful and abnormal exit. It never removes or resets the candidate issue
worktree or another user worktree.

If a completed build was followed by a preflight stop, set
`PREBUILT_BINARY_ROOT` to the prior run's `binaries` directory to resume without
rebuilding under memory pressure. The directory must contain executable
`hfs-<arm>` and `hfs-<arm>.build.json` files for every selected arm. A normal
build writes these manifests. Each one binds the arm and source commit, the
normalized Cargo command, feature inputs, Cargo and rustc versions, a SHA-256
over the scoped compiled inputs, and the binary's size and SHA-256. The
compiled-input scope covers workspace Cargo and toolchain files, `.cargo`
configuration, crate build scripts and source, plus non-test data directories
such as migrations, assets, templates, generated packs, grammar, resources,
terminology data, and vendored inputs. It also covers the root `data` tree. It
excludes tests, benchmark runners, and documentation because those files do not
enter this HFS release binary.

Before a prebuilt run copies a binary, the runner recreates the pinned source
worktree and recomputes the compiled-input fingerprint. It rejects a changed
source commit, build command, feature set, binary hash, or compiled input. A
`DRY_RUN=1` prebuilt check validates the manifest envelope and binary hash
without creating source worktrees. The full fingerprint check occurs in a real
run before the copy.

Legacy binary directories without manifests cannot be attested after the
fact. In particular,
`/tmp/hfs-1455-r1-evidence/benchmark-full3/binaries` has no build manifests and
the runner rejects it. The detached build source snapshots no longer exist, so
there is no honest way to bind those binaries to unchanged compiled inputs.
Rebuild the three arms to create a reusable manifest-backed set.

Each trial gets a fresh PostgreSQL 16 container and database. The container
preloads `pg_stat_statements` and enables `pg_stat_statements.track_utility`.
The controller starts HFS on an automatically selected port with one submit
worker, one file fetch, deferred indexing, `HFS_REQUEST_TIMEOUT=300`, and
`HFS_PG_STATEMENT_TIMEOUT_MS=300000` for every arm. The fixed five-minute HTTP
and PostgreSQL statement timeouts are paired with the controller's
`--request-timeout 300`. Together they let the exact post-ingest search
validation finish even on the historically slower issue-source arm; they do
not change the measured kickoff-to-terminal ingest interval. Caller
`--hfs-env` overrides remain last. The controller captures the source,
controller, binary, PostgreSQL image, and fixture hashes so that unlike inputs
cannot silently enter an aggregate.

Family coverage is exact SQL evidence: the controller counts distinct Patient
`resource_id` values for tenant `default`, `param_name='family'`, and
`value_string='Pilot995'`, and requires that count to equal the complete
fixture size. This uses the PostgreSQL string-value index instead of the
historical HTTP prefix/`COALESCE` scan. HTTP search-API validation remains
covered by a bounded `_id=<middle-id>` query. The `active=true` oracle likewise
counts distinct Patient IDs in SQL where `param_name='active'`,
`value_token_system IS NULL`, and `value_token_code='true'`; the normal fixture
requires exactly 5,000. Full-text coverage is also exact SQL evidence: the
controller applies the production `_content` predicate,
`content_tsvector @@ plainto_tsquery('english', 'Pilot995')`, and requires all
10,000 non-deleted Patients. These SQL counts, together with the bounded HTTP
`_id` oracle, preserve representative structured, FTS, and end-to-end API
coverage without an expensive indexed HTTP total query. The SQL values and
expected active count are recorded in `run.json` under each attempt's
validation evidence.

The normal fixture is one deterministic NDJSON file containing 10,000 all-new
Patients. The oversized fixture is a separate 203-Patient file with a compact
serialized Patient of 8 MiB plus one byte at offset 100. The latter is a
correctness and RSS check only; it is never included in performance gates.

## SQL-count oracles

Counts are deltas from after HFS schema startup and immediately before kickoff
through the observed terminal and verified search-ready boundaries. The query
is scoped to the current database and user. `INSERT` means the grouped
`INSERT INTO resources` statement; `SAVEPOINT` means the exact
`SAVEPOINT bulk_entry` utility statement.

| Normal batch size | Candidate INSERT / SAVEPOINT | Baseline INSERT / SAVEPOINT |
|---:|---:|---:|
| 100 | 100 / 0 | 100 / 0 |
| 101 | 199 / 0 | 10,000 / 9,999 |
| 128 | 157 / 0 | 9,985 / 9,984 |
| 500 | 100 / 0 | 10,000 / 10,000 |
| 1,000 | 100 / 0 | 10,000 / 10,000 |

For the 203-resource oversized fixture at batch size 500, the candidate oracle
is `4 / 0`; both baseline arms use `203 / 203`.

A trial is non-comparable if the extension or utility tracking is unavailable,
the startup pilot cannot observe its savepoint, a counter matches more than one
query ID, a boundary count differs from its oracle, or required fingerprints,
PostgreSQL activity snapshots, validation, or memory samples are missing.
Timed trials do not enable `log_statement=all`.

## Commands

Inspect the complete default campaign without changing worktrees or starting
containers:

```bash
DRY_RUN=1 crates/hfs/tests/bulk_submit/run_postgres_grouped_create_benchmark.sh
```

Run a quick end-to-end pilot before the full campaign:

```bash
PILOT=1 crates/hfs/tests/bulk_submit/run_postgres_grouped_create_benchmark.sh
```

Run the full normal and oversized matrices:

```bash
crates/hfs/tests/bulk_submit/run_postgres_grouped_create_benchmark.sh
```

Resume a stopped campaign with already-built release binaries, while writing a
new evidence tree and recreating the pinned source worktrees:

```bash
PREBUILT_BINARY_ROOT=/path/to/manifest-backed-run/binaries \
  crates/hfs/tests/bulk_submit/run_postgres_grouped_create_benchmark.sh
```

The default normal matrix is five batch sizes, three fresh trials, and all
three arms. The oversized matrix is batch size 500, three fresh trials, and all
three arms. Normal trials use exactly 10,000 resources and oversized trials use
exactly 203. The runner rotates the first arm by trial to counter fixed-order
cache and host-load effects. Useful subset controls are comma-separated `ARMS`, `BATCH_SIZES`,
and `FIXTURE_MODES`, plus `TRIALS`, `NORMAL_RESOURCES`,
`OVERSIZED_RESOURCES`, and `EVIDENCE_ROOT`. `PILOT_*` variants override the
pilot defaults. `PG_PORT` and `PROVIDER_PORT` start at 19455 and 19457; the
runner selects the next free port without stopping unrelated processes or
containers. Containers are named `hfs-pgi01-r1-*`, and HFS uses port `0` so the
controller allocates a free port.

An equivalent focused controller dry-run is:

```bash
python3 crates/hfs/tests/bulk_submit/measure_memory.py \
  --dry-run \
  --binary /path/to/hfs \
  --output-dir /tmp/hfs-1455-example \
  --repo-root . \
  --database-url postgres://helios:helios@127.0.0.1:19455/helios \
  --pg-container hfs-pgi01-r1-example \
  --postgres-grouped-create-evidence \
  --fixture-mode normal \
  --resources 10000 \
  --batch-size 128 \
  --expected-resource-inserts 157 \
  --expected-bulk-entry-savepoints 0 \
  --request-timeout 300 \
  --jobs 1 \
  --file-concurrency 1 \
  --defer-indexing true
```

## Artifacts and aggregation

By default, raw evidence is stored under
`target/issue-1455/<UTC-run-id>/raw/<fixture>/<arm>/batch-<n>/trial-<n>`.
Every trial contains `run.json`, the phase and RSS samples, validation output,
and `jobs/job01/postgres-grouped-create-evidence.json`. The runner writes
`aggregate.json` beside `raw`; it includes hashes, per-trial validity, ingest
and search-ready duration, HFS peak RSS, WAL bytes, SQL counts, and per-cell
median/min/max/spread. Every row also records the configured resource count.
The full-matrix classification requires 10,000 normal resources and 203
oversized resources in addition to the arms, batch sizes, fixtures, and trial
count.

The full gate requires:

1. At least one batch size above 100 improves candidate median ingest time by
   at least 10% versus current main.
2. At batch size 100, candidate median ingest time and median HFS peak RSS each
   regress by no more than 5% versus current main.
3. The same performance gate passes against the issue-source arm.
4. All required trials are comparable and pass exact receipt, ID, body,
   history, search, and reindex validation.

Current-main versus candidate is the causal comparison. Issue-source versus
candidate is retained as the historical comparison. If their pass/fail results
disagree, the aggregate reports the disagreement and cannot succeed.

After writing `aggregate.json`, a requested default full matrix exits nonzero
unless `gate.status` is `pass`. A pilot or explicit subset remains diagnostic:
its gate is `incomplete`, but the runner exits zero when all commands and
checks used by that subset completed.

## Results

The [issue acceptance comment](https://github.com/HeliosSoftware/hfs/issues/1455#issuecomment-5784876394)
accepts the focused performance evidence from 2026-09-22. This was not a full
matrix pass.

The causal 10,000-resource batch-101 comparison in `benchmark-full2` used fresh
PostgreSQL databases. Current main took 14.027 seconds and issued 10,000
resource INSERTs and 9,999 per-entry savepoints. The candidate took 3.469
seconds and issued 199 grouped INSERTs and no savepoints. Ingest time improved
by 75.3%.

An independent 1,000-resource batch-101 run in `benchmark-pilot-rerun2`
confirmed the result. Current main took 2.673 seconds with 910 INSERTs and 909
savepoints. The candidate took 2.167 seconds with 19 INSERTs and no savepoints,
an 18.9% improvement.

The batch-100 regression check used three comparable trials per arm from
`benchmark-full2`. Median ingest time changed from 3.182 to 3.217 seconds, a
1.1% regression. Median peak HFS RSS changed from 104.266 to 107.203 MiB, a
2.8% regression. Both remained below the 5% acceptance limits.

Every cited run passed the exact resource, history, receipt, structured-index,
FTS, and deferred-reindex checks. The later `benchmark-full5` campaign completed
eight valid runs. The ninth stopped when the swap watchdog observed 391 MiB of
growth, above its 256 MiB limit. The stop was environmental, not a correctness
or SQL-oracle failure.

The issue acceptance comment waived the remaining exhaustive matrix and the
oversized-fixture benchmark. Unit and PostgreSQL integration tests cover the
oversized grouping boundary. Durability, replay, cancellation, exclusion, and
COMMIT behavior remain required and are covered by the test suite.

The accepted evidence roots are:

- `/tmp/hfs-1455-r1-evidence/benchmark-full2`
- `/tmp/hfs-1455-r1-evidence/benchmark-pilot-rerun2`
- `/tmp/hfs-1455-r1-evidence/benchmark-full5`
