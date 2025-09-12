# pysof

Python wrapper for the Helios SOF (SQL on FHIR) toolkit.

This package provides Python bindings for the Rust `helios-sof` library via PyO3 and maturin. Use `uv` to manage the environment and run builds.

## Requirements

- Python 3.11
- uv (package and environment manager)

## Quickstart

```bash
# From repo root
cd crates/pysof

# Ensure Python 3.11 is available and create a venv
uv venv --python 3.11

# Install the project
uv sync

# Build and install the Rust extension into the venv
uv run --with maturin maturin develop --release

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

Download the appropriate wheel for your platform from the [releases page](https://github.com/helios/hfs/releases) and install:

```bash
pip install pysof-*.whl
```

### Supported Platforms

- **Linux**: x86_64 (glibc and musl)
- **Windows**: x86_64 (MSVC)
- **macOS**: x86_64 and ARM64 (when available)
- **Python**: 3.11 only

For detailed information about wheel building and distribution, see [WHEEL_BUILDING.md](WHEEL_BUILDING.md).

## Testing

Run the comprehensive test suite:

```bash
# Run all tests
uv run pytest

# Run specific test files
uv run pytest tests/test_core_functions.py -v
uv run pytest tests/test_content_types.py -v
uv run pytest tests/test_import.py -v

# Run with coverage
uv run pytest --cov=pysof --cov-report=html

# Run tests with detailed output
uv run pytest -v --tb=short
```

Current test coverage:
- **58 total tests** across 5 test files
- Core API functions (19 tests)
- Content type support (14 tests) 
- FHIR version support (16 tests)
- Package structure and imports (6 tests)
- Package metadata (3 tests)

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

print("CSV Output:")
print(csv_result.decode('utf-8'))

print("\nJSON Output:")
data = json.loads(json_result.decode('utf-8'))
print(json.dumps(data, indent=2))
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

### Supported Content Types

| Format | Description | Output |
|--------|-------------|---------|
| `csv` | CSV with headers | Comma-separated values with header row |
| `json` | JSON array | Array of objects, one per result row |
| `ndjson` | Newline-delimited JSON | One JSON object per line |
| ~~`parquet`~~ | ~~Parquet format~~ | *(Planned but not yet implemented)* |

### Supported FHIR Versions

- **R4** (default, always available)
- **R4B** (if compiled with R4B feature)
- **R5** (if compiled with R5 feature) 
- **R6** (if compiled with R6 feature)

Use `pysof.get_supported_fhir_versions()` to check what's available in your build.

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
├─ tests/                  # Test suite (58 tests)
│  ├─ test_core_functions.py
│  ├─ test_content_types.py
│  ├─ test_fhir_versions.py
│  ├─ test_import.py
│  └─ test_package_metadata.py
└─ Cargo.toml              # Rust crate metadata
```

## Roadmap / TODO

- v0 (MVP: packaging and docs)

  - [x] Package skeleton with src layout
  - [x] `pyproject.toml` targeting Python 3.11
  - [x] README with quickstart, layout, roadmap
  - [x] Define Python public API surface in `pysof/__init__.py`
  - [x] Add `ruff` (lint) and `mypy` (types) configs
  - [x] Add `uv` scripts for fmt, lint, test
  - [x] Add minimal tests: import/package metadata/version
  - [x] Add CI workflow for Python 3.11: lint + tests (Linux/Windows self-hosted)
  - [ ] Install uv on self-hosted runners to enable CI execution
  - [ ] Add macOS self-hosted runner support to CI

- v1 (Rust bindings and wheels)

  - [x] Introduce PyO3 bindings to call Rust `helios-sof` library
  - [x] Switch build backend to `maturin` (PEP 621) for extension builds
  - [x] **Core API Functions:**
    - [x] Expose `run_view_definition(view: dict, bundle: dict, format: str) -> bytes`
    - [x] Expose `run_view_definition_with_options(view: dict, bundle: dict, format: str, *, since: str = None, limit: int = None, page: int = None) -> bytes`
  - [x] **Content Type Support:** Map content types: `csv` (with headers), `json`, `ndjson` *(parquet planned but not yet implemented)*
  - [x] **FHIR Version Support:**
    - [x] Add optional `fhir_version` parameter (default: "R4")
    - [x] Support R4, R4B, R5, R6 versions based on Rust feature compilation
    - [x] Add version compatibility validation
  - [x] **Comprehensive Error Handling:** Map Rust errors to Python exceptions:
    - [x] `InvalidViewDefinitionError` (from `SofError::InvalidViewDefinition`)
    - [x] `FhirPathError` (from `SofError::FhirPathError`)
    - [x] `SerializationError` (from `SofError::SerializationError`)
    - [x] `UnsupportedContentTypeError` (from `SofError::UnsupportedContentType`)
    - [x] `CsvError` (from `SofError::CsvError`)
    - [x] `IoError` (from `SofError::IoError`)
    - [x] Base `SofError` exception class hierarchy
  - [x] **Utility Functions:**
    - [x] `validate_view_definition(view: dict) -> bool` - Pre-validate ViewDefinition structure
    - [x] `validate_bundle(bundle: dict) -> bool` - Pre-validate Bundle structure
    - [x] `parse_content_type(mime_type: str) -> str` - Parse MIME types to format strings
    - [x] `get_supported_fhir_versions() -> List[str]` - List available FHIR versions
  - [x] Add wheel builds for Windows, Linux (manylinux/musllinux)
  - [ ] Add macOS wheel builds (x86_64, arm64) when self-hosted macOS runner available
  - [ ] Add integration tests mirroring Rust crate examples
  - [ ] Provide examples: in-memory, file-based, stdin/stdout-like workflows

- v2 (Advanced features and optimization)

  - [ ] Enable Parquet support when available in `sof` crate
  - [ ] **Streaming Support:**
    - [ ] Add streaming API for large datasets with NDJSON output
    - [ ] Memory-efficient processing for large bundles
  - [ ] **Async Support:**
    - [ ] Async variants of core functions when Rust crate adds async support
    - [ ] Integration with asyncio for concurrent processing
  - [ ] **Performance Optimization:**
    - [ ] Batch processing utilities for multiple ViewDefinitions
    - [ ] Caching mechanisms for repeated transformations
  - [ ] **Enhanced Validation:**
    - [ ] Schema validation against FHIR specification
    - [ ] FHIRPath expression syntax validation
    - [ ] ViewDefinition linting and optimization suggestions

- Infrastructure/DevOps
  - [ ] Install uv package manager on self-hosted CI runners
  - [ ] Set up macOS self-hosted runner (when needed for wheel distribution)
  - [ ] Configure Python 3.11+ on all CI environments

- Future (Server integration and ecosystem)
  - [ ] **HTTP Client Integration:**
    - [ ] Optional async client wrappers for `sof-server` HTTP endpoints
    - [ ] Connection pooling and retry logic
    - [ ] Authentication and authorization support
  - [ ] **Ecosystem Integration:**
    - [ ] Pandas DataFrame output format support
    - [ ] Apache Arrow integration for columnar data
    - [ ] Integration with popular FHIR Python libraries
  - [ ] **Developer Experience:**
    - [ ] Interactive Jupyter notebook examples
    - [ ] VS Code extension for ViewDefinition editing
    - [ ] CLI tool (`pysof-cli`) mirroring Rust `sof-cli` functionality
  - [ ] Prebuilt wheels for additional Python versions (3.12+) as demand dictates

## License

This package inherits the license from the repository root.
