# Testing

## Running Tests

```bash
# All tests (R4 only by default)
cargo test

# All FHIR versions
cargo test --features R4,R4B,R5,R6

# Specific crate
cargo test -p helios-sof
cargo test -p helios-fhirpath
cargo test -p helios-persistence

# Single test by name pattern
cargo test test_name_pattern

# Show println! output
cargo test -- --nocapture
```

## Docker Requirement

PostgreSQL and Elasticsearch integration tests use [testcontainers](https://testcontainers.com/) to spin up real database instances in Docker. **Docker must be running** to execute the full test suite.

To skip those tests when Docker is unavailable:

```bash
cargo test --all-features -- --skip postgres_integration --skip es_integration
```

## Python Tests (`pysof`)

```bash
# Python tests (58 tests)
cd crates/pysof && uv run pytest python-tests/ -v

# Rust tests (17 tests)
cd crates/pysof && cargo test
```

## Test Patterns

### FHIRPath Tests

Test cases live in `crates/fhirpath/tests/`. The official FHIR test cases from the `fhir-test-cases` repository are included.

### SQL-on-FHIR Tests

- Unit tests are colocated with source in `src/` files
- Integration tests are in the `tests/` directory within the crate

### Persistence Tests

- Integration tests use **testcontainers** for PostgreSQL and Elasticsearch (Docker required)
- A shared container is created once per test binary using `tokio::sync::OnceCell`
- Data isolation is achieved via unique UUID-based prefixes and tenant IDs — not separate containers
- Elasticsearch containers cap JVM heap: `ES_JAVA_OPTS=-Xms256m -Xmx256m`

### Test Data

- FHIR examples: `crates/fhir/tests/data/`
- Search parameter definitions: `data/search-parameters-{r4,r4b,r5,r6}.json`
- ViewDefinition examples: inline in test files
