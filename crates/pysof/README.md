# pysof - SQL on FHIR for Python

[![PyPI version](https://badge.fury.io/py/pysof.svg)](https://pypi.org/project/pysof/)
[![Python versions](https://img.shields.io/pypi/pyversions/pysof.svg)](https://pypi.org/project/pysof/)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Downloads](https://pepy.tech/badge/pysof)](https://pepy.tech/project/pysof)

**High-performance FHIR data transformation for Python.** Transform FHIR resources into tabular formats (CSV, JSON, Parquet) using declarative ViewDefinitions from the [SQL on FHIR specification](https://build.fhir.org/ig/FHIR/sql-on-fhir-v2/).

Built in Rust for speed, exposed to Python with a simple, Pythonic API. Part of the [Helios FHIR Server](https://github.com/HeliosSoftware/hfs) project.

## ✨ Key Features

- 🚀 **High Performance**: Native Rust implementation with minimal Python overhead
- 📊 **Multiple Output Formats**: CSV, JSON, NDJSON, and Parquet
- 🔄 **Parallel Processing**: Automatic multithreading with 5-7x speedup on multi-core systems
- 📦 **Streaming Support**: Memory-efficient chunked processing for large NDJSON files
- 🌐 **Multi-Version FHIR**: Supports R4, R4B, R5, and R6 (based on build features)
- 🎯 **Type-Safe**: Leverages Rust's type safety with a Pythonic interface
- ⚡ **GIL-Free**: Python GIL released during processing for true parallelism

## 🎯 Why pysof?

Working with FHIR data in Python just got faster. **pysof** lets you:

- **Transform complex FHIR resources** into clean, analyzable tables without writing custom parsers
- **Process large datasets efficiently** with automatic parallel processing and Rust-level performance
- **Use standard SQL on FHIR ViewDefinitions** for portable, maintainable data transformations
- **Export to multiple formats** (CSV, JSON, NDJSON, Parquet) for analytics, ML, or reporting workflows

Perfect for healthcare data engineers, researchers, and developers building FHIR-based analytics pipelines.

## 🔗 Quick Links

- 📦 **[PyPI Package](https://pypi.org/project/pysof/)**
- 📚 **[Documentation](https://github.com/HeliosSoftware/hfs/tree/main/crates/pysof)**
- 🐛 **[Issue Tracker](https://github.com/HeliosSoftware/hfs/issues)**
- 💻 **[Source Code](https://github.com/HeliosSoftware/hfs)**
- 📋 **[GitHub Releases](https://github.com/HeliosSoftware/hfs/releases)**

## 📥 Installation

### From PyPI (Recommended)

```bash
pip install pysof
```

**Supported Platforms:**
- **Linux**: x86_64 (glibc and musl)
- **Windows**: x86_64 (MSVC)
- **macOS**: AArch64 (Apple Silicon)
- **Python**: 3.10, 3.11, 3.12, 3.13, 3.14

### From GitHub Releases

Download pre-built wheels from the [releases page](https://github.com/HeliosSoftware/hfs/releases):

```bash
pip install pysof-*.whl
```

## 🚀 Quick Start

Transform FHIR patient data to CSV in just a few lines:

```python
import pysof

# Define what data to extract
view_definition = {
    "resourceType": "ViewDefinition",
    "id": "patient-demographics",
    "name": "PatientDemographics",
    "status": "active",
    "resource": "Patient",
    "select": [{
        "column": [
            {"name": "id", "path": "id"},
            {"name": "family_name", "path": "name.family"},
            {"name": "given_name", "path": "name.given.first()"},
            {"name": "gender", "path": "gender"},
            {"name": "birth_date", "path": "birthDate"}
        ]
    }]
}

# Sample FHIR Bundle
bundle = {
    "resourceType": "Bundle",
    "type": "collection",
    "entry": [{
        "resource": {
            "resourceType": "Patient",
            "id": "patient-1",
            "name": [{"family": "Doe", "given": ["John"]}],
            "gender": "male",
            "birthDate": "1990-01-01"
        }
    }]
}

# Transform to CSV
csv_output = pysof.run_view_definition(view_definition, bundle, "csv")
print(csv_output.decode('utf-8'))
# Output:
# id,family_name,given_name,gender,birth_date
# patient-1,Doe,John,male,1990-01-01
```

## 📖 Usage

### Multiple Output Formats

```python
import pysof
import json

# Transform to different formats
csv_result = pysof.run_view_definition(view_definition, bundle, "csv")
json_result = pysof.run_view_definition(view_definition, bundle, "json")
ndjson_result = pysof.run_view_definition(view_definition, bundle, "ndjson")
parquet_result = pysof.run_view_definition(view_definition, bundle, "parquet")

print("CSV Output:")
print(csv_result.decode('utf-8'))

print("\nJSON Output:")
data = json.loads(json_result.decode('utf-8'))
print(json.dumps(data, indent=2))
```

### Advanced Options

```python
import pysof

# Transform with pagination and filtering
result = pysof.run_view_definition_with_options(
    view_definition,
    bundle,
    "json",
    limit=10,                          # Limit results
    page=1,                            # Page number
    since="2023-01-01T00:00:00Z",     # Filter by modification date
    fhir_version="R4"                  # Specify FHIR version
)
```

### Remote `resolve()` (Trusted Servers)

`resolve()` dereferences references against the input bundle by default. You can
optionally let it fetch references that point at **explicitly trusted** FHIR
servers and fold them into the resolution pool before rows are generated. This is
**off by default** and gated by a strict allowlist (matching scheme + host + port
+ path-prefix; private/loopback addresses are blocked unless opted in).

```python
import pysof

config = pysof.RemoteResolveConfig(
    ["https://fhir.example.org/r4"],   # trusted base URLs (allowlist)
    max_fetches=256,                    # per-run/-stream fetch cap
    max_depth=1,                        # chained-reference rounds
    allow_private_addresses=False,      # allow internal LBs (e.g. Traefik) by hostname
    bearer_tokens={"fhir.example.org": "TOKEN"},  # optional per-host auth
)

# Or build it from SOF_RESOLVE_* environment variables:
# config = pysof.RemoteResolveConfig.from_env()

result = pysof.run_view_definition_remote(
    view_definition, bundle, "json", config, fhir_version="R4"
)

# Streaming NDJSON with remote resolve() (one shared cache across chunks):
stats = pysof.process_ndjson_to_file_remote(
    view_definition, "input.ndjson", "output.json", "ndjson", config, chunk_size=1000
)
```

When `config.is_active()` is `False` (disabled or empty allowlist) these behave
exactly like their non-remote counterparts. Enabling remote resolution makes
output depend on remote server state at run time. See the
[SQL-on-FHIR guide](../../book/src/ch06-sql-on-fhir.md) for the full security
model and configuration reference.

### Utility Functions

```python
import pysof

# Validate structures
is_valid_view = pysof.validate_view_definition(view_definition)
is_valid_bundle = pysof.validate_bundle(bundle)

# Parse content types
format_str = pysof.parse_content_type("text/csv")  # Returns "csv_with_header"

# Check supported FHIR versions
versions = pysof.get_supported_fhir_versions()  # Returns ["R4"] or more
print(f"Supported FHIR versions: {versions}")

# Package info
print(f"Version: {pysof.get_version()}")
print(pysof.get_status())
```

### Streaming Large NDJSON Files

For memory-efficient processing of large NDJSON files, use the `ChunkedProcessor` iterator or `process_ndjson_to_file` function:

```python
import pysof

view_definition = {
    "resourceType": "ViewDefinition",
    "status": "active",
    "resource": "Patient",
    "select": [{"column": [
        {"name": "id", "path": "id"},
        {"name": "gender", "path": "gender"}
    ]}]
}

# Iterator approach - process chunks incrementally
for chunk in pysof.ChunkedProcessor(view_definition, "patients.ndjson", chunk_size=500):
    print(f"Chunk {chunk['chunk_index']}: {len(chunk['rows'])} rows")
    for row in chunk["rows"]:
        process_row(row)
    if chunk["is_last"]:
        print("Processing complete!")

# Access column names before iterating
processor = pysof.ChunkedProcessor(view_definition, "patients.ndjson")
print(f"Columns: {processor.columns}")
for chunk in processor:
    # Process chunks...
    pass

# File-to-file approach - most memory efficient
stats = pysof.process_ndjson_to_file(
    view_definition,
    "input.ndjson",
    "output.csv",
    "csv",  # or "csv_with_header", "ndjson"
    chunk_size=1000,
    skip_invalid=True,  # Continue past invalid JSON lines
    fhir_version="R4"
)
print(f"Processed {stats['resources_processed']} resources")
print(f"Output {stats['output_rows']} rows in {stats['chunks_processed']} chunks")
print(f"Skipped {stats['skipped_lines']} invalid lines")
```

**When to use streaming:**
- Processing NDJSON files larger than available memory
- Working with datasets of 100K+ resources
- Building ETL pipelines that process data incrementally
- When you need fault-tolerant processing (skip invalid lines)

### Error Handling

```python
import pysof

try:
    result = pysof.run_view_definition(view_definition, bundle, "json")
except pysof.InvalidViewDefinitionError as e:
    print(f"ViewDefinition validation error: {e}")
except pysof.SerializationError as e:
    print(f"JSON parsing error: {e}")
except pysof.UnsupportedContentTypeError as e:
    print(f"Unsupported format: {e}")
except pysof.SofError as e:
    print(f"General SOF error: {e}")
```

## ⚡ Performance

### Automatic Parallel Processing

pysof automatically processes FHIR resources in parallel using rayon:

- **5-7x speedup** on typical batch workloads with multi-core CPUs
- **Streaming benefits**: `ChunkedProcessor` and `process_ndjson_to_file` also use parallel processing
- **Zero configuration** - parallelization is always enabled
- **Python GIL released** during processing for true parallel execution

### Performance Benchmarks

| Mode | Dataset | Time | Memory | Notes |
|------|---------|------|--------|-------|
| **Batch** | 10k Patients | ~2.7s | 1.6 GB | All resources in memory |
| **Streaming** | 10k Patients | ~0.9s | 45 MB | 35x less memory, 2.9x faster |
| **Batch** | 93k Encounters | ~4s | 3.9 GB | All resources in memory |
| **Streaming** | 93k Encounters | ~2.8s | 25 MB | 155x less memory, 1.4x faster |

Streaming mode (`ChunkedProcessor`, `process_ndjson_to_file`) is recommended for large NDJSON files.

### Controlling Thread Count (RAYON_NUM_THREADS)

Set the `RAYON_NUM_THREADS` environment variable to control parallel processing:

```python
import os
os.environ['RAYON_NUM_THREADS'] = '4'  # Must be set before first import

import pysof
result = pysof.run_view_definition(view_definition, bundle, "json")
```

Or from the command line:

```bash
# Linux/Mac
RAYON_NUM_THREADS=4 python my_script.py

# Windows PowerShell
$env:RAYON_NUM_THREADS=4
python my_script.py
```

**When to adjust thread count:**
- **Reduce threads** (`RAYON_NUM_THREADS=2-4`): On shared systems, containers with CPU limits, or when running multiple instances
- **Increase threads**: Rarely needed; rayon auto-detects available cores
- **Single thread** (`RAYON_NUM_THREADS=1`): For debugging or deterministic output ordering

**Performance Tips:**
- Use all available cores for large datasets (default behavior)
- Limit threads on shared systems to avoid resource contention
- Prefer streaming mode (`ChunkedProcessor`) for NDJSON files > 100MB

## 📋 Supported Features

### Output Formats

| Format | Description | Output |
|--------|-------------|--------|
| `csv` | CSV with headers | Comma-separated values with header row |
| `json` | JSON array | Array of objects, one per result row |
| `ndjson` | Newline-delimited JSON | One JSON object per line |
| `parquet` | Parquet format | Columnar binary format for analytics |

### FHIR Versions

- **R4** (default, always available)
- **R4B** (if compiled with R4B feature)
- **R5** (if compiled with R5 feature)
- **R6** (if compiled with R6 feature)

Use `pysof.get_supported_fhir_versions()` to check available versions in your build.

---

## 🔧 Development

### Requirements

- Python 3.10 or later (3.10, 3.11, 3.12, 3.13, 3.14 supported)
- uv (package and environment manager)
- Rust toolchain (for building from source)

> **Note**: This crate is excluded from the default workspace build. When running `cargo build` from the repository root, `pysof` will not be built automatically.

### Building from Source

### Building with Cargo

This crate is excluded from the default workspace build to allow building the core Rust components without Python. To build it explicitly:

```bash
# Your current directory MUST be the pysof crate:
cd crates/pysof

# From the pysof folder
cargo build

# Or build with specific FHIR version features
cargo build -p pysof --features R4,R5
```

### Building with Maturin (Recommended)

For Python development, it's recommended to use `maturin` via `uv`:

```bash
# From repo root
cd crates/pysof

# Create a venv with your preferred Python version (3.10+)
uv venv --python 3.11  # or 3.10, 3.12, 3.13, 3.14

# Install the project dev dependencies
uv sync --group dev

# Build and install the Rust extension into the venv
uv run maturin develop --release

# Build distributable artifacts
uv run maturin build --release -o dist     # wheels
uv run maturin sdist -o dist               # source distribution

# Sanity checks
uv run python -c "import pysof; print(pysof.__version__); print(pysof.get_status()); print(pysof.get_supported_fhir_versions())"
```

### Installing from Source

Requires Rust toolchain:

```bash
# Install directly
pip install -e .

# Or build wheel locally
maturin build --release --out dist
pip install dist/*.whl
```

### Testing

The project has separate test suites for Python and Rust components:

#### Python Tests

Run the comprehensive Python test suite:

```bash
# Run all Python tests
uv run pytest python-tests/

# Run specific test files
uv run pytest python-tests/test_core_functions.py -v
uv run pytest python-tests/test_content_types.py -v
uv run pytest python-tests/test_import.py -v

# Run with coverage
uv run pytest python-tests/ --cov=pysof --cov-report=html

# Run tests with detailed output
uv run pytest python-tests/ -v --tb=short
```

#### Rust Tests

Run the Rust unit and integration tests:

```bash
# Run all Rust tests
cargo test

# Run unit tests only
cargo test --test lib_tests

# Run integration tests only
cargo test --test integration

# Run with verbose output
cargo test -- --nocapture
```

## Configuring FHIR Version Support

By default, pysof is compiled with **R4 support only**. You can configure which FHIR versions are available by modifying the feature compilation settings.

### Change Default FHIR Version

To change from R4 to another version (e.g., R5):

1. **Edit `crates/pysof/Cargo.toml`**:
   ```toml
   [features]
   default = ["R5"]  # Changed from ["R4"]
   R4 = ["helios-sof/R4", "helios-fhir/R4"]
   R4B = ["helios-sof/R4B", "helios-fhir/R4B"]
   R5 = ["helios-sof/R5", "helios-fhir/R5"]
   R6 = ["helios-sof/R6", "helios-fhir/R6"]
   ```

2. **Rebuild the extension**:
   ```bash
   cd crates/pysof
   uv run maturin develop --release
   ```

3. **Verify the change**:
   ```bash
   uv run python -c "
   import pysof
   versions = pysof.get_supported_fhir_versions()
   print('Supported FHIR versions:', versions)
   "
   ```
   This should now show `['R5']` instead of `['R4']`.

### Enable Multiple FHIR Versions

To support multiple FHIR versions simultaneously:

1. **Edit `crates/pysof/Cargo.toml`**:
   ```toml
   [features]
   default = ["R4", "R5"]  # Enable both R4 and R5
   # Or enable all versions:
   # default = ["R4", "R4B", "R5", "R6"]
   ```

2. **Rebuild and verify**:
   ```bash
   uv run maturin develop --release
   uv run python -c "import pysof; print(pysof.get_supported_fhir_versions())"
   ```
   This should show `['R4', 'R5']` (or all enabled versions).

3. **Use specific versions in code**:
   ```python
   import pysof
   
   # Use R4 explicitly
   result_r4 = pysof.run_view_definition(view, bundle, "json", fhir_version="R4")
   
   # Use R5 explicitly  
   result_r5 = pysof.run_view_definition(view, bundle, "json", fhir_version="R5")
   ```

### Build with Specific Features (Without Changing Default)

To temporarily build with different features without modifying `Cargo.toml`:

```bash
# Build with only R5
cargo build --features R5 --no-default-features

# Build with R4 and R6
cargo build --features R4,R6 --no-default-features

# With maturin
uv run --with maturin -- maturin develop --release --cargo-extra-args="--features R5 --no-default-features"
```

### Testing After Version Changes

After changing FHIR version support, run the test suite to ensure compatibility:

```bash
# Run all tests
uv run pytest

# Run FHIR version-specific tests
uv run pytest tests/test_fhir_versions.py -v

# Test with your new default version
uv run python -c "
import pysof

# Test with default version (should be your new default)
view = {'resourceType': 'ViewDefinition', 'id': 'test', 'name': 'Test', 'status': 'active', 'resource': 'Patient', 'select': [{'column': [{'name': 'id', 'path': 'id'}]}]}
bundle = {'resourceType': 'Bundle', 'type': 'collection', 'entry': [{'resource': {'resourceType': 'Patient', 'id': 'test'}}]}

result = pysof.run_view_definition(view, bundle, 'json')
print('Default version test successful:', len(result), 'bytes')
"
```

## Project layout

```
crates/pysof/
├─ pyproject.toml          # PEP 621 metadata, Python >=3.8, uv-compatible
├─ README.md
├─ src/
│  ├─ pysof/
│  │  └─ __init__.py       # Python package root
│  └─ lib.rs               # Rust PyO3 bindings
├─ tests/                  # Rust tests (17 tests)
│  ├─ lib_tests.rs         # Unit tests for core library functions
│  ├─ integration.rs       # Integration tests for component interactions
│  └─ integration/         # Organized integration test modules
│     ├─ mod.rs
│     ├─ content_types.rs
│     ├─ error_handling.rs
│     └─ fhir_versions.rs
├─ python-tests/           # Python test suite (58 tests)
│  ├─ __init__.py
│  ├─ test_core_functions.py
│  ├─ test_content_types.py
│  ├─ test_fhir_versions.py
│  ├─ test_import.py
│  └─ test_package_metadata.py
└─ Cargo.toml              # Rust crate metadata
```

## 📄 License

MIT License - See [LICENSE.md](../../LICENSE.md) for details.

Copyright (c) 2025 Helios Software

## 🤝 Contributing

Contributions are welcome! Please see our [Contributing Guidelines](../../CONTRIBUTING.md) for details.

### Reporting Issues

- **Bug Reports**: [GitHub Issues](https://github.com/HeliosSoftware/hfs/issues)
- **Security Issues**: Email team@heliossoftware.com

### Development Setup

See the [Development](#-development) section above for instructions on setting up your development environment.

## 🙏 Acknowledgments

Built with:
- [PyO3](https://pyo3.rs/) - Rust bindings for Python
- [maturin](https://www.maturin.rs/) - Build system for Rust Python extensions
- [helios-sof](../sof) - Core SQL-on-FHIR implementation in Rust

Part of the [Helios FHIR Server](https://github.com/HeliosSoftware/hfs) project.

---

**Made with ❤️ by [Helios Software](https://heliossoftware.com)**
