# Building

## Default Build

Builds all workspace crates with R4 support (excludes `pysof`):

```bash
cargo build
```

## With All FHIR Versions

```bash
cargo build --features R4,R4B,R5,R6
```

## Specific Crates

```bash
cargo build -p helios-hfs
cargo build -p helios-fhirpath
cargo build -p helios-sof
cargo build -p helios-persistence --features postgres,elasticsearch
```

## Release Binaries

```bash
cargo build --release
cargo build --release --features R4,R4B,R5,R6
```

Binaries land in `target/release/`.

## Python Bindings (`pysof`)

`pysof` is excluded from the default workspace build because it requires Python.

```bash
# Rust-only crate build
cargo build -p pysof

# Recommended: build into a virtual env via maturin
cd crates/pysof
uv venv --python 3.11
uv sync
uv run maturin develop --release

# Build distributable wheels
uv run maturin build --release -o dist
uv run maturin sdist -o dist
```

## Linting and Formatting

```bash
# Format all code
cargo fmt --all

# Lint (CI-compatible flags)
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

> **Note:** `--all-features` is safe for clippy — it enables the `skip-r6-download` feature in FHIR crates, preventing spec downloads during the check.

## Checklist Before Submitting Changes

1. `cargo fmt --all`
2. `cargo clippy` with CI flags (fix all warnings)
3. `cargo test -p <affected-crate>`
