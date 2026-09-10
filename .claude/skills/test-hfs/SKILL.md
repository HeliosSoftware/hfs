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
- Isolate test data with unique UUID-based prefixes or tenant IDs instead of separate containers.
- Cap Elasticsearch JVM heap with `ES_JAVA_OPTS=-Xms256m -Xmx256m`.

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
(the `HFS_BULK_SUBMIT_DEFER_INDEXING` path), `--no-phases`, `--keep` (leave the
database for `dbstat`), `--data-dir`.

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
