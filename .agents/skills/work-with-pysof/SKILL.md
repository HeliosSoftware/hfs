---
name: work-with-pysof
description: Work on pysof Python bindings for SQL-on-FHIR. Use for PyO3, maturin, Python package setup, pysof tests, Python APIs, PyPI publishing concerns, and any files under crates/pysof.
---

# pysof

Use this when working in `crates/pysof` or changing Python bindings for SQL-on-FHIR.

`pysof` is excluded from default workspace builds. Root `cargo build` does not build it.

## Installation

```bash
pip install pysof
```

## Development Setup

```bash
cd crates/pysof
uv venv --python 3.11
uv sync --group dev
uv run maturin develop --release

# Verify
uv run python -c "import pysof; print(pysof.get_version()); print(pysof.get_supported_fhir_versions())"
```

## Testing

```bash
cd crates/pysof
uv run pytest python-tests/ -v
cargo test
```

The Python suite has historically had 58 tests, and the Rust pysof suite has historically had 17 tests. Recheck counts from the current tree instead of relying on those numbers when reporting results.

## Key API

```python
import pysof

# Basic transformation
result = pysof.run_view_definition(view_definition, bundle, "csv")

# With options
result = pysof.run_view_definition_with_options(view, bundle, "json", limit=10, fhir_version="R4")

# Streaming large NDJSON files
for chunk in pysof.ChunkedProcessor(view, "patients.ndjson", chunk_size=500):
    process(chunk["rows"])

# File-to-file, most memory efficient
stats = pysof.process_ndjson_to_file(view, "input.ndjson", "output.csv", "csv")
```
