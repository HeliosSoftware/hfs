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

Cluster-capable-state work (discussion #223) uses three tiers:

- **T1 unit** — pure logic with no store: the `HFS_CLUSTER` refusal table in
  `crates/hfs/src/cluster.rs`, state machines against in-memory reference
  models, serde round-trips.
- **T2 two-handle** — two *independently constructed* backends over one
  testcontainer database, raced from a barrier; never a cloned `Arc`, which
  shares the in-process heap and proves nothing. Helpers live in
  `crates/persistence/tests/common/cluster_harness.rs` (`two_handles`,
  `race2`, `assert_exactly_one`, `assert_visible`,
  `assert_wrong_tenant_hidden`) and are `#[path]`-included by each test binary
  that needs them (nothing declares `tests/common` as a module). Every T2 suite
  asserts the applicable definition-of-done rows — visibility, wrong-tenant
  isolation (mandatory), claim exclusivity, fencing, durability across a
  handle drop, cache invalidation — and, where a `memory` variant exists, that
  two memory handles do *not* see each other. Suites that claim from a
  cross-tenant queue take a per-suite `tokio::Mutex` so parallel tests cannot
  steal each other's rows.
- **T3 two-process** — `.github/workflows/cluster-smoke.yml`: two `hfs`
  binaries on one PostgreSQL behind nginx. Reserved for what T2 cannot reach:
  a live WebSocket on the far instance and kill-9 recovery of durable job
  paths (nightly tier). Dispatch on a branch with
  `gh workflow run cluster-smoke.yml --ref <branch>` (add `-f nightly=true`
  for the nightly tier); scheduled runs execute `main`'s copy.

Run the T2 suites that exist today:

```bash
cargo test -p helios-persistence --features postgres --test postgres_tests -- cluster_
cargo test -p helios-persistence --features mongodb --test mongodb_tests -- cluster_
```

Every cluster PR writes its test first and quotes the failing run against
`main` in the PR body; a test that cannot go red is not evidence.

## Test Data

- FHIR examples live in `crates/fhir/tests/data/`.
- Search parameter definitions live in `data/search-parameters-{r4,r4b,r5,r6}.json`.
- ViewDefinition examples are embedded in test files.
