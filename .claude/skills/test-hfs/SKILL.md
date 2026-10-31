---
name: test-hfs
description: Plan, run, or debug tests in the Helios HFS Rust workspace. Use for cargo test selection, FHIRPath tests, SQL-on-FHIR tests, persistence integration tests, testcontainers, tenant isolation, Elasticsearch test tuning, and test data locations.
---

# Testing HFS

Use this for test strategy, test setup, and test commands across this workspace.

## General Commands

```bash
# Run all default workspace tests
cargo test

# Run tests with all FHIR versions
cargo test --features R4,R4B,R5,R6

# Run a specific crate
cargo test -p helios-sof
cargo test -p helios-fhirpath
cargo test -p helios-persistence

# Run tests matching a name pattern
cargo test test_name_pattern

# Run a test target
cargo test --test test_file_name

# Show test output
cargo test -- --nocapture
```

## Patterns

### FHIRPath

- Test cases live in `crates/fhirpath/tests/`.
- Official FHIR test cases come from the `fhir-test-cases` repository.

### SQL-on-FHIR

- Unit tests live in `src/` files.
- Integration tests live in `tests/`.

### Persistence

- Integration tests use testcontainers for PostgreSQL and Elasticsearch, so Docker is required.
- Use `tokio::sync::OnceCell` for shared containers across tests: one container per test binary.
- Shared containers are labelled with `with_cleanup_label` (`crates/persistence/tests/common/container_cleanup.rs`; `crates/rest/tests/common/` and `crates/hts/tests/common/` have copies that differ only in their label keys, and a unit test in each copy fails when they drift). An exit hook removes them when the test process exits normally. On Windows a run with a failing test (libtest exits through `ExitProcess`, which skips exit hooks), or any killed run, leaves them behind. The next test process of the same crate (same label key) on that machine removes every container with that label whose owner PID is no longer running, in the same process namespace. Containers also carry a `test-pidns` label, so a WSL distro or dev container that shares the Docker engine never sweeps the other side's live containers, and a container without that label is never swept. That sweep is skipped when `CI` is set or the docker CLI points at a remote daemon, and CI reaps by `github.run_id` and an age-guarded `org.testcontainers.managed-by` sweep instead (#2404, #2414).
- Isolate test data with unique UUID-based prefixes or tenant IDs instead of separate containers.
- Cap Elasticsearch JVM heap with `ES_JAVA_OPTS=-Xms256m -Xmx256m`.

### MongoDB on a RAM-backed mongod

`mongodb_tests` starts its own `mongo:7.0` single-node replica set unless
`HFS_TEST_MONGODB_URL` is set; then it connects to that URL instead. On a
host with slow disk syncs, putting `/data/db` on tmpfs cuts the run sharply
(one VPS: 2927 s against a disk-backed mongod, about 1740 s against tmpfs).
Start it with the harness container's own flags (the `with_cmd` list in
`crates/persistence/tests/mongodb_tests.rs`):

```bash
MSYS_NO_PATHCONV=1 docker run -d --name mongodb-tests-tmpfs \
    -p 127.0.0.1:27019:27017 --tmpfs /data/db --ulimit nofile=64000:64000 \
    mongo:7.0 mongod --bind_ip_all --wiredTigerCacheSizeGB 0.25 \
    --setParameter enableTestCommands=1 --replSet rs0 --oplogSize 128 \
    --setParameter minSnapshotHistoryWindowInSeconds=5
for _ in $(seq 60); do
    docker exec mongodb-tests-tmpfs mongosh --quiet --eval 'db.hello()' \
        >/dev/null 2>&1 && break
    sleep 1
done
docker exec mongodb-tests-tmpfs mongosh --quiet --eval \
    'rs.initiate({_id:"rs0",members:[{_id:0,host:"localhost:27017"}]});
     for (let i = 0; i < 120 && !db.hello().isWritablePrimary; i++) sleep(500)'

export HFS_TEST_MONGODB_URL='mongodb://127.0.0.1:27019/?directConnection=true'
cargo test -p helios-persistence --features mongodb --test mongodb_tests; \
cargo test -p helios-persistence --features mongodb --test mongodb_tests -- \
    --ignored --exact db_cleanup::end_of_run_report

docker rm -fv mongodb-tests-tmpfs; unset HFS_TEST_MONGODB_URL
```

- The member advertises `localhost:27017`, its address inside the container,
  so the URL needs `?directConnection=true`; the harness adds it only to its
  own container's URL.
- If mongod never comes up, `docker logs mongodb-tests-tmpfs` shows why; both
  waits above give up instead of hanging.
- Against an external URL the suite does not check these flags. Most tests that
  need one print `Skipping` (visible with `-- --nocapture`) and pass, so a
  missing flag is easy to miss: without the replica set most transaction tests
  skip, but the `missing_index::` transaction tests panic at their first
  `process_transaction` call, and
  `mongodb_repeated_type_composite_legacy_rows_require_reindex` fails on the
  topology error it gets in place of a `$reindex` one. Without
  `enableTestCommands=1` the `failCommand` failpoint tests skip, except
  `mongodb_broad_search_admission_protects_reads`, which fails. Without the
  `nofile` ulimit, index builds fail with `TooManyFilesOpen` late in the run.
- `minSnapshotHistoryWindowInSeconds=5` is a speed-up, not a requirement. It
  frees a dropped test database's WiredTiger tables at the next checkpoint
  instead of after the default 300 s (#2326). One laptop, full suite: 650 s
  against 583 s, with peak data handles 63k against 29k.
- `MSYS_NO_PATHCONV=1` in front of `docker run` is for Git Bash on Windows,
  which would otherwise rewrite `/data/db` into a Windows path; elsewhere it
  does nothing.
- **Never point `HFS_TEST_MONGODB_URL` at a server that holds real data**: the
  suite creates and drops databases on it. Remove the container afterwards
  (last line above; `-v` also removes its anonymous `/data/configdb` volume);
  its data stays in RAM until you do.

Cleanup knobs (#2311). Each test's database is dropped once its test ends, and
at exit the suite prints one
`[hfs-test-db-cleanup] registered=... dropped=... kept=...` summary line.

- `HFS_TEST_MONGODB_KEEP_FAILED=1` (any value but empty or `0`) keeps the
  databases of tests that panicked instead, up to 8 per test process, and logs
  each kept name on a `[hfs-test-db-cleanup]` line. Any panic on a test thread
  counts, even one the test catches. Useful only with `HFS_TEST_MONGODB_URL`
  (the harness's own container is removed at exit); inspect the kept databases
  before the `docker rm` line.
- `db_cleanup::end_of_run_report` is `#[ignore]`d and skips unless
  `HFS_TEST_MONGODB_URL` is set. Run it right after the suite in the same
  command, as above; the `;` runs it when the suite fails too. It prints a
  `[hfs-2311-report]` line to stderr (no `--nocapture` needed): leftover
  `hfs_phase2_mongo_*` databases (and up to 20 of their names), WiredTiger
  data handles, drop-pending idents, checkpoint figures and average latencies.
  The server figures count from mongod start, so use a fresh container for
  each measured run. `HFS_TEST_MONGODB_MAX_LEFTOVER_DATABASES=N` makes it fail
  when more than N test databases are left.
- A test that leaves a transaction open (the failpoint tests do: a commit that
  mongod refused leaves its transaction open) no longer holds its database, or a
  write ticket, for the 60-90 s mongod takes to expire it (#2436). Before it
  drops a finished test's database the sweeper ends the sessions that still hold
  locks on it or run operations in it (`lockInfo` and `$currentOp`, matched on
  the database's exact name, then `killSessions`), and drops it with a 5 s
  server-side time bound, again until 120 s have passed. It logs
  `[hfs-test-db-cleanup] ended N session(s) still using <db> after test <name> ended`
  and the summary line carries an `ended=` count. So a short filtered run such as
  `transaction_retry::` should end with `outstanding=0 flush=done`. A server
  that refuses `lockInfo`, `$currentOp` or `killSessions` (an authenticated
  server without the `serverStatus`, `inprog` or `killAnySession` privileges)
  skips ending the sessions, says so once, and falls back to the bound drop
  alone; the two db_cleanup tests that need `lockInfo` then print `Skipping`.

### pysof

```bash
cd crates/pysof
uv run pytest python-tests/ -v
cargo test
```

## Profiling the ingest write path

`helios_persistence::perf` carries per-phase timers across the SQLite ingest
path — NDJSON parse, read-before-write, resource and history INSERT, search
extraction, index INSERT, FTS, bulk bookkeeping, commit. They answer "where did
the time go", which a wall-clock rate cannot.

**They are compiled out unless you ask for them, with `--cfg perf_phases`.**
That is a cfg and not a cargo feature on purpose: `ci.yml`'s `build` job runs
`cargo build --workspace --all-features --release` and the `release` job
publishes exactly those artifacts (the Docker images copy them in), so a
feature would have shipped the instrumentation in every binary. `--all-features`
cannot turn a cfg on. Same reasoning as `tokio_unstable`.

```bash
# Rate only — no phase table, nothing compiled in:
cargo run --release -p helios-persistence --example bulk_submit_bench -- \
    --limit 25000 /path/to/CarePlan.ndjson

# With the phase table:
RUSTFLAGS='--cfg perf_phases' \
  cargo run --release -p helios-persistence --example bulk_submit_bench -- \
    --limit 25000 /path/to/CarePlan.ndjson
```

The example prints a reminder if you ask for phases from a binary built without
the flag, rather than showing a table of zeros. Within a `--cfg perf_phases`
build, collection is still off until `HFS_PERF_PHASES=1` or
`perf::set_enabled(true)` — the benchmark does the latter.

`bulk_submit_bench` drives the real `process_ndjson_stream`, the same call the
bulk-submit worker makes per manifest file, against local NDJSON on a fresh
database. Useful flags: `--limit` (resources per file), `--batch`, `--defer-index`
(the `HFS_BULK_SUBMIT_DEFER_INDEXING` path), `--reindex` (then run the
post-manifest rebuild the worker's hook fires, timed and phased on its own —
`reindex_fetch` / `reindex_page (total)` plus the shared `extract`,
`search_index_insert`, `fts`, `commit`), `--reindex-batch`,
`--bulk-index-rebuild` (the `HFS_BULK_SUBMIT_BULK_INDEX_REBUILD` mode),
`--no-phases`, `--keep` (leave the database for `dbstat`), `--data-dir`.
`HFS_EXPERIMENT_SQL="<statements>"` runs arbitrary SQL against the fresh
schema before the ingest — drop an index, recreate a trigger, tune FTS5 — so a
schema idea can be priced without a build.

The server's default path is `--batch 1000 --defer-index --reindex`, and on it
the rebuild is ~90% of the wall clock — profile that stage, not the ingest, when
the question is "why is import slow". `extract` there sums CPU across the
extraction pool's threads and can exceed the wall clock; `prepare_batch (wall)`
is what the writer actually waited for. The mix that fits a coffee break is
`--limit 20000` over `Observation`, `Condition`, `Patient`, `Encounter` from the
corpus in `MANUAL_TESTING_MATRIX.md` (72k resources, ~20s per arm).

`bulk_submit_mongo_bench` is its MongoDB twin (#1000), and needs a live server —
that ingest path is round-trip-bound, which no in-process fake reproduces. Same
flags minus the SQLite-only ones, plus `--uri` (or `HFS_MONGO_URI`) and `--db`.
It drops and recreates its database on every run, so the rate is a cold-index
rate and two runs compare. It also prints documents-per-resource per collection,
which is the write-volume half of a MongoDB ingest number:

```bash
docker run -d --name bench-mongo -p 27018:27018 mongo:7.0 \
    --port 27018 --replSet rs --bind_ip_all
docker exec bench-mongo mongosh --port 27018 --quiet \
    --eval 'rs.initiate({_id:"rs",members:[{_id:0,host:"localhost:27018"}]})'

cargo run --release -p helios-persistence --features mongodb \
    --example bulk_submit_mongo_bench -- \
    --uri "mongodb://localhost:27018/?replicaSet=rs" --limit 5000 \
    /path/to/Condition.ndjson /path/to/Patient.ndjson
```

Give the container a `--wiredTigerCacheSizeGB` you can actually exceed if you are
measuring index-pressure decay rather than round trips: the two regimes have
different fixes, and a run that fits in cache never shows the second one.

### Benchmarking discipline

Two traps, both of which have produced wrong numbers here:

- **Run it from the repo root, or pass `--data-dir`.** The default `data`
  directory holds `search-parameters-r4.json`; without it the registry falls
  back to five embedded parameters, index volume drops from ~14 rows per
  resource to ~2, and the run is meaninglessly fast. The same applies to a real
  `hfs` benchmark: set `HFS_DATA_DIR`.
- **Interleave arms; never compare across sessions.** This workload drifts
  several percent between runs of a machine on different days. Build each arm
  in its own worktree and alternate them in one loop, three rounds minimum, and
  compare medians. A single run against another arm's median is not a
  measurement — differences under ~1% are below this harness's noise floor.

## Test Data

- FHIR examples live in `crates/fhir/tests/data/`.
- Search parameter definitions live in `data/search-parameters-{r4,r4b,r5,r6}.json`.
- ViewDefinition examples are embedded in test files.
