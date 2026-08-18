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

### Cluster testing (T1/T2/T3)

Testing HFS running as N instances behind a load balancer uses three tiers:

- **T1 — unit**: pure logic, no shared infrastructure (config fail-fast validation, fencing-token arithmetic, job-state transitions against an in-memory fake store). Plain `#[test]`, runs in every `cargo test`.
- **T2 — shared-backend, multi-handle, one process** (the workhorse, ~80% of cluster coverage): **two independently constructed backend handles — never a cloned `Arc`** — sharing one Postgres testcontainer, asserting the cross-instance protocol. Every suite covers whichever of these apply to the subsystem: **visibility** (write via handle A → observable via handle B), **isolation** (wrong-tenant write via A is NOT observable via B — 404/empty), **exclusivity** (A and B both claim/redeem the same item → exactly one succeeds), **fencing** (a stale-token write after the lease moved → `LeaseLost`), **durability** (drop and re-create a handle, simulating a redeploy → state survives), **invalidation** (mutate via A → a stale cached read via B is refreshed). Not every row applies to every subsystem (a cache has no lease; a token store has no durability guarantee).
- **T3 — true multi-process E2E**: two real `hfs &` (or `hts &`) processes on separate ports sharing one backend, real HTTP/WS traffic through a round-robin front — for the few things T2 structurally cannot reach (an event/socket that lives in one OS process and must cross the network to another). Runs via the `cluster-smoke` GitHub Actions workflow, not `cargo test`.

```bash
# T2: two independently constructed backend handles racing over one shared
# Postgres testcontainer — no special env var, just the postgres feature + Docker
cargo test -p helios-persistence --features postgres   # cluster_job_store, cluster_refresh_cache,
                                                         # subscription_*, composite_sync_outbox (postgres_tests.rs)
cargo test -p helios-hts --features postgres            # D1 (postgres_bootstrap_lock.rs), C3 (postgres_epoch_cluster.rs)
cargo test -p helios-rest --features postgres           # jwks_cluster_pg.rs, sof_export_cluster_pg.rs
cargo test -p helios-subscriptions --features postgres  # cluster_engine.rs (T1, in-memory), subscriptions_cluster_pg.rs (T2)
RUN_REDIS_CLUSTER_TESTS=1 cargo test -p helios-auth --features redis  # jwks_cluster_redis.rs (Redis twin)
```

T3 (real two-process smoke, self-hosted CI runners): `.github/workflows/cluster-smoke.yml` — dispatch against a branch with `gh workflow run cluster-smoke.yml --ref <branch>`; add `-f nightly=true` for the kill-9 recovery cases (A1 SoF export, E1 composite sync outbox), which are otherwise schedule-only and execute `main`'s copy of the workflow until a branch merges.

## Test Data

- FHIR examples live in `crates/fhir/tests/data/`.
- Search parameter definitions live in `data/search-parameters-{r4,r4b,r5,r6}.json`.
- ViewDefinition examples are embedded in test files.
