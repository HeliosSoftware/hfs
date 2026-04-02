# pysof — Python Bindings

`pysof` provides Python bindings for SQL-on-FHIR via [PyO3](https://pyo3.rs/) and [maturin](https://maturin.rs/). It brings native Rust performance to Python data pipelines.

## Installation

```bash
pip install pysof
```

Cross-platform wheels for Linux, Windows, and macOS are published to [PyPI](https://pypi.org/project/pysof/).

## Basic Usage

```python
import pysof

# Transform a FHIR bundle to CSV
result = pysof.run_view_definition(
    view_definition=view_def_json,
    bundle=fhir_bundle_json,
    format="csv"
)

# Process individual resources
result = pysof.run_view_definition(
    view_definition=view_def_json,
    resources=[patient1, patient2],
    format="parquet"
)
```

## With Options

```python
result = pysof.run_view_definition_with_options(
    view=view_def_json,
    bundle=fhir_bundle_json,
    format="json",
    limit=10,
    fhir_version="R4"
)
```

## Streaming Large Files

```python
# Chunk-based streaming (memory-efficient for large NDJSON files)
for chunk in pysof.ChunkedProcessor(view_def_json, "patients.ndjson", chunk_size=500):
    process(chunk["rows"])

# File-to-file (most memory-efficient path)
stats = pysof.process_ndjson_to_file(view_def_json, "input.ndjson", "output.csv", "csv")
```

## Key Features

- ViewDefinition processing to CSV, JSON, NDJSON, and Parquet
- Streaming support for large FHIR bundles
- Auto-detection of R4, R4B, R5, and R6 FHIR versions
- Type-safe Rust core with a Pythonic API

## Development Setup

```bash
cd crates/pysof
uv venv --python 3.11
uv sync --group dev
uv run maturin develop --release

# Verify
uv run python -c "import pysof; print(pysof.get_version())"
```

## Running Tests

```bash
# Python tests
cd crates/pysof && uv run pytest python-tests/ -v

# Rust tests
cd crates/pysof && cargo test
```
