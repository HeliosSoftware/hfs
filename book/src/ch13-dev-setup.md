# Development Setup

This chapter covers building from source, running the test suite, and linting — everything you need to contribute to HFS or work on the codebase.

---

## Building from Source

### Default build (R4 only)

```bash
cargo build
```

Builds all default workspace members (excludes `pysof`). Compiles R4 only.

### Build with all FHIR versions

```bash
cargo build --features R4,R4B,R5,R6
```

### Release binaries

```bash
cargo build --release
cargo build --release --features R4,R4B,R5,R6
```

Binaries land in `target/release/`.

### Build a specific crate

```bash
cargo build -p helios-hfs
cargo build -p helios-fhirpath
cargo build -p helios-sof
cargo build -p helios-persistence --features postgres,elasticsearch
```

### Python bindings (pysof)

`pysof` is excluded from the default workspace build because it requires Python and maturin.

```bash
cd crates/pysof

# Set up a virtual environment
uv venv --python 3.11
uv sync --group dev

# Build and install into the venv (recommended for development)
uv run maturin develop --release

# Build distributable wheels
uv run maturin build --release -o dist
```

> **Note:** Build times can exceed 10 minutes on a first build, especially with all FHIR versions, due to the large generated files in `helios-fhir`.

---

## Running the Test Suite

### All tests (R4)

```bash
cargo test
```

### All tests with all FHIR versions

```bash
cargo test --features R4,R4B,R5,R6
```

### Specific crate

```bash
cargo test -p helios-sof
cargo test -p helios-fhirpath
cargo test -p helios-persistence
```

### Single test by name pattern

```bash
cargo test test_name_pattern
```

### Show stdout output

```bash
cargo test -- --nocapture
```

### Docker requirement for integration tests

PostgreSQL and Elasticsearch integration tests in `helios-persistence` use [testcontainers](https://testcontainers.com/) to spin up real database instances in Docker. **Docker must be running** to execute these tests.

To skip integration tests when Docker is unavailable:

```bash
cargo test --all-features -- --skip postgres_integration --skip es_integration
```

Testcontainers details:
- A shared container is created once per test binary via `tokio::sync::OnceCell`
- Data isolation uses unique UUID-based prefixes and tenant IDs — not separate containers per test
- Elasticsearch containers cap JVM heap: `ES_JAVA_OPTS=-Xms256m -Xmx256m`

### Python tests (pysof)

```bash
cd crates/pysof

# Python tests (58 tests)
uv run pytest python-tests/ -v

# Rust unit tests (17 tests)
cargo test
```

### Test data locations

| What | Where |
|------|-------|
| FHIR example resources | `crates/fhir/tests/data/` |
| Search parameter definitions | `data/search-parameters-{r4,r4b,r5,r6}.json` |
| FHIRPath official test cases | `crates/fhirpath/tests/` |
| ViewDefinition examples | Inline in test files |

---

## Linting and Formatting

### Format all code

```bash
cargo fmt --all
```

### Lint with CI-compatible flags

```bash
cargo clippy --all-targets --all-features -- -D warnings \
  -A clippy::items_after_test_module \
  -A clippy::large_enum_variant \
  -A clippy::question_mark \
  -A clippy::collapsible_match \
  -A clippy::collapsible_if \
  -A clippy::field_reassign_with_default \
  -A clippy::doc-overindented-list-items \
  -A clippy::doc-lazy-continuation
```

> **Note:** `--all-features` is safe for clippy — it enables `skip-r6-download`, preventing R6 spec downloads during the check. All lints are enforced as errors (`-D warnings`).

### Type checking without building

```bash
cargo check
```

---

## Pre-Submission Checklist

Before opening a pull request, run these three commands and fix any issues:

```bash
# 1. Format
cargo fmt --all

# 2. Lint (must be zero warnings)
cargo clippy --all-targets --all-features -- -D warnings \
  -A clippy::items_after_test_module \
  -A clippy::large_enum_variant \
  -A clippy::question_mark \
  -A clippy::collapsible_match \
  -A clippy::collapsible_if \
  -A clippy::field_reassign_with_default \
  -A clippy::doc-overindented-list-items \
  -A clippy::doc-lazy-continuation

# 3. Test affected crates
cargo test -p <affected-crate>
```

---

## Useful Debug Commands

```bash
# Enable trace logging for the HFS server
HFS_LOG_LEVEL=trace cargo run --bin hfs

# Enable debug logging for the FHIRPath server
FHIRPATH_LOG_LEVEL=debug cargo run --bin fhirpath-server

# Test a FHIRPath expression interactively
cargo run --bin fhirpath-cli -- -e "Patient.name.family" -r patient.json

# Show println! output in tests
cargo test -p helios-fhirpath -- --nocapture
```
