# Python Bindings (pysof)

`pysof` provides Python bindings for SQL-on-FHIR via [PyO3](https://pyo3.rs/) and [maturin](https://maturin.rs/). It gives Python data pipelines native Rust performance for FHIR data transformation.

---

## Installing via PyPI

```bash
pip install pysof
```

Pre-built wheels are available for:
- Linux (x86_64, aarch64)
- Windows (x86_64)
- macOS (x86_64, Apple Silicon / aarch64)

No Rust toolchain is needed when installing from PyPI.

---

## Running ViewDefinitions from Python

### Basic usage

```python
import pysof

# Load JSON strings (or dicts serialized to strings)
result = pysof.run_view_definition(
    view_definition=view_def_json,   # ViewDefinition as a JSON string
    bundle=fhir_bundle_json,          # FHIR Bundle as a JSON string
    format="csv"                      # "csv", "json", "ndjson", "parquet"
)
print(result)
```

### With individual resources (no Bundle wrapper)

```python
result = pysof.run_view_definition(
    view_definition=view_def_json,
    resources=[patient1_json, patient2_json],
    format="json"
)
```

### With options

```python
result = pysof.run_view_definition_with_options(
    view=view_def_json,
    bundle=fhir_bundle_json,
    format="json",
    limit=100,
    fhir_version="R4"   # "R4", "R4B", "R5", "R6"
)
```

---

## Exporting to Parquet

```python
import pysof

# Returns bytes — write to file or pass to pyarrow/pandas
parquet_bytes = pysof.run_view_definition(
    view_definition=view_def_json,
    bundle=bundle_json,
    format="parquet"
)

with open("output.parquet", "wb") as f:
    f.write(parquet_bytes)
```

Parquet output uses Snappy compression by default and follows Pathling type conventions. All fields are `OPTIONAL`. Arrays map to Arrow `List` types.

---

## Streaming Large Bundles

### ChunkedProcessor

For large NDJSON files that do not fit in memory, use `ChunkedProcessor`:

```python
import pysof

for chunk in pysof.ChunkedProcessor(
    view_definition=view_def_json,
    ndjson_path="large-patients.ndjson",
    chunk_size=500      # resources per chunk (default: 1000)
):
    rows = chunk["rows"]   # list of row dicts
    process(rows)
```

### process_ndjson_to_file

The most memory-efficient path — streams directly from an NDJSON input file to an output file without loading everything into Python:

```python
import pysof

stats = pysof.process_ndjson_to_file(
    view_definition=view_def_json,
    input_path="input.ndjson",
    output_path="output.csv",
    format="csv"
)
print(f"Processed {stats['rows_written']} rows")
```

---

## Performance

`pysof` uses [Rayon](https://github.com/rayon-rs/rayon) for automatic multi-core parallelism, which gives a **5–7× speedup** on typical multi-core machines compared to single-threaded processing.

Control the number of threads:

```bash
RAYON_NUM_THREADS=4 python process.py
```

Or from Python before the first call:

```python
import os
os.environ["RAYON_NUM_THREADS"] = "4"
import pysof
```

---

## Building from Source with maturin

You need Python 3.x and the Rust toolchain.

```bash
cd crates/pysof

# Create a virtual environment (recommended: use uv)
uv venv --python 3.11
uv sync --group dev

# Build and install into the virtual environment
uv run maturin develop --release

# Verify the installation
uv run python -c "import pysof; print(pysof.get_version()); print(pysof.get_supported_fhir_versions())"
```

To build distributable wheels:

```bash
uv run maturin build --release -o dist
```

---

## Running Tests

```bash
cd crates/pysof

# Python tests (58 tests)
uv run pytest python-tests/ -v

# Rust unit tests (17 tests)
cargo test
```

---

## Utility Functions

```python
import pysof

# Get the library version
pysof.get_version()         # e.g. "0.1.47"

# List compiled FHIR versions
pysof.get_supported_fhir_versions()  # e.g. ["R4"]
```
