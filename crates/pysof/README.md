# pysof

Python wrapper for the Helios SOF (SQL on FHIR) toolkit.

This package provides Python bindings for the Rust `helios-sof` library via PyO3 and maturin. Use `uv` to manage the environment and run builds.

> **Note**: This crate is excluded from the default workspace build via workspace default-members. When running `cargo build` from the repository root, `pysof` will not be built automatically. Build it explicitly when needed.

## Requirements

- Python 3.11
- uv (package and environment manager)

## Building the Crate

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

# Ensure Python 3.11 is available and create a venv
uv venv --python 3.11

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

## Installation

### From PyPI (Recommended)

```bash
pip install pysof
```

### From Source

```bash
# Install from source (requires Rust toolchain)
pip install -e .

# Or build wheel locally
maturin build --release --out dist
pip install dist/*.whl
```

### From GitHub Releases

Download the appropriate wheel for your platform from the [releases page](https://github.com/HeliosSoftware/hfs/releases) and install:

```bash
pip install pysof-*.whl
```

### Supported Platforms

- **Linux**: x86_64 (glibc and musl)
- **Windows**: x86_64 (MSVC)
- **macOS**: AArch64 (x86_64 is not supported) 
- **Python**: 3.11 only

For detailed information about wheel building and distribution, see [WHEEL_BUILDING.md](WHEEL_BUILDING.md).

## Testing

The project has separate test suites for Python and Rust components:

### Python Tests

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

Current Python test coverage:
- **58 total tests** across 5 test files
- Core API functions (19 tests)
- Content type support (14 tests) 
- FHIR version support (16 tests)
- Package structure and imports (6 tests)
- Package metadata (3 tests)

### Rust Tests

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

Current Rust test coverage:
- **17 total tests** across 2 test files
- Unit tests: 14 tests covering core library functions
- Integration tests: 3 tests covering component interactions

## Usage

### Basic Example

```python
import pysof
import json

# Sample ViewDefinition for extracting patient data
view_definition = {
    "resourceType": "ViewDefinition",
    "id": "patient-demographics",
    "name": "PatientDemographics", 
    "status": "active",
    "resource": "Patient",
    "select": [
        {
            "column": [
                {"name": "id", "path": "id"},
                {"name": "family_name", "path": "name.family"},
                {"name": "given_name", "path": "name.given.first()"},
                {"name": "gender", "path": "gender"},
                {"name": "birth_date", "path": "birthDate"}
            ]
        }
    ]
}

# Sample FHIR Bundle with patient data
bundle = {
    "resourceType": "Bundle",
    "type": "collection", 
    "entry": [
        {
            "resource": {
                "resourceType": "Patient",
                "id": "patient-1",
                "name": [{"family": "Doe", "given": ["John"]}],
                "gender": "male",
                "birthDate": "1990-01-01"
            }
        },
        {
            "resource": {
                "resourceType": "Patient", 
                "id": "patient-2",
                "name": [{"family": "Smith", "given": ["Jane"]}],
                "gender": "female",
                "birthDate": "1985-05-15"
            }
        }
    ]
}

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

print("\nParquet Output (binary):")
print(f"Parquet data: {len(parquet_result)} bytes")
```

### Advanced Usage with Options

```python
import pysof
from datetime import datetime

# Transform with pagination and filtering
result = pysof.run_view_definition_with_options(
    view_definition,
    bundle, 
    "json",
    limit=10,           # Limit to 10 results
    page=1,             # First page
    since="2023-01-01T00:00:00Z",  # Filter by modification date
    fhir_version="R4"   # Specify FHIR version
)
```

### Utility Functions

```python
import pysof

# Validate structures before processing
is_valid_view = pysof.validate_view_definition(view_definition)
is_valid_bundle = pysof.validate_bundle(bundle)

# Parse content types
format_str = pysof.parse_content_type("text/csv")  # Returns "csv_with_header"
format_str = pysof.parse_content_type("application/json")  # Returns "json"

# Check supported FHIR versions
versions = pysof.get_supported_fhir_versions()  # Returns ["R4"] (or more based on build)
print(f"Supported FHIR versions: {versions}")

# Check package status
print(pysof.get_status())  # Shows current implementation status
print(f"Version: {pysof.get_version()}")
```

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

### Key Features

- **Performance**: Efficient processing of large FHIR Bundles using Rust
- **Parallel Processing**: Automatic multithreading of FHIR resources using rayon (5-7x speedup on multi-core systems)
- **GIL Release**: Python GIL is released during Rust execution for better performance
- **Multiple Formats**: Support for CSV, JSON, NDJSON, and Parquet outputs
- **FHIR Versions**: Support for R4, R4B, R5, and R6 (depending on build features)

### Available Options

The `run_view_definition_with_options()` function accepts the following parameters:

- **limit**: Limit the number of results returned
- **page**: Page number for pagination (1-based)
- **since**: Filter resources modified after this ISO8601 datetime
- **fhir_version**: FHIR version to use ("R4", "R4B", "R5", "R6")

### Supported Content Types

| Format | Description | Output |
|--------|-------------|---------|
| `csv` | CSV with headers | Comma-separated values with header row |
| `json` | JSON array | Array of objects, one per result row |
| `ndjson` | Newline-delimited JSON | One JSON object per line |
| `parquet` | Parquet format | Columnar binary format optimized for analytics |

### Supported FHIR Versions

- **R4** (default, always available)
- **R4B** (if compiled with R4B feature)
- **R5** (if compiled with R5 feature) 
- **R6** (if compiled with R6 feature)

Use `pysof.get_supported_fhir_versions()` to check what's available in your build.

### Usage Notes

- The basic `run_view_definition()` function is suitable for simple use cases
- Use `run_view_definition_with_options()` for pagination, filtering, and other advanced features
- All heavy processing happens in Rust code; Python GIL is properly released for optimal performance

## Multithreading and Performance

### Automatic Parallel Processing

pysof automatically processes FHIR resources in parallel using rayon, providing significant performance improvements on multi-core systems:

- **5-7x speedup** for typical workloads on modern CPUs
- **Zero configuration required** - parallelization is always enabled
- **Python GIL released** during processing for true parallel execution

### Controlling Thread Count

By default, pysof uses all available CPU cores. You can control the thread count using the `RAYON_NUM_THREADS` environment variable:

```python
import os
import pysof

# Set thread count BEFORE first use (must be set before rayon initializes)
os.environ['RAYON_NUM_THREADS'] = '4'

# Now all operations will use 4 threads
result = pysof.run_view_definition(view_definition, bundle, "json")
```

Or from the command line:

```bash
# Linux/Mac
RAYON_NUM_THREADS=4 python my_script.py

# Windows (cmd)
set RAYON_NUM_THREADS=4
python my_script.py

# Windows (PowerShell)
$env:RAYON_NUM_THREADS=4
python my_script.py
```

**Important**: The environment variable must be set before rayon initializes its global thread pool (typically on first use of pysof). Once initialized, the thread count cannot be changed for that process.

### Performance Tips

- **Large datasets**: Use all available cores (default behavior)
- **Shared systems**: Limit threads to avoid resource contention (`RAYON_NUM_THREADS=4`)
- **Memory constrained**: Reduce thread count to lower memory usage
- **Benchmarking**: Test different thread counts to find optimal performance for your workload

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

### Important Considerations

1. **Dependency Requirements**: Ensure the underlying `helios-sof` and `helios-fhir` crates support the features you want to enable
2. **Binary Size**: Enabling multiple FHIR versions increases the compiled binary size significantly
3. **Memory Usage**: Multiple versions require more memory at runtime
4. **Testing**: Always run your test suite after changing version support
5. **Documentation**: Update any project documentation that mentions specific FHIR versions
6. **CI/CD**: Update build scripts and CI workflows if they depend on specific versions

### Version Compatibility Matrix

| FHIR Version | Feature Flag | Status | Notes |
|--------------|--------------|---------|-------|
| R4 | `R4` | ✅ Stable | Default, always recommended |
| R4B | `R4B` | ✅ Stable | Minor updates to R4 |
| R5 | `R5` | ✅ Stable | Current latest stable version |
| R6 | `R6` | ⚠️ Preview | May have limited support |

### Common Configuration Examples

```toml
# Production: Single version for minimal size
default = ["R4"]

# Development: Multiple versions for testing
default = ["R4", "R5"]

# Bleeding edge: Latest versions only
default = ["R5", "R6"]

# Maximum compatibility: All versions (not recommended for production)
default = ["R4", "R4B", "R5", "R6"]
```

## Project layout

```
crates/pysof/
├─ pyproject.toml          # PEP 621 metadata, Python >=3.11, uv-compatible
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

## License

This package inherits the license from the repository root.
