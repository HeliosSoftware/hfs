# pysof

Python wrapper for the Helios SOF (SQL on FHIR) toolkit.

This is an initial scaffolding for the Python package. In a future iteration we will expose the Rust `sof` crate APIs to Python (likely via PyO3 + maturin). For now, this package provides a project layout and tooling configured for Python 3.11 with uv.

## Requirements

- Python 3.11
- uv (package and environment manager)

## Quickstart

```bash
# From repo root
cd crates/pysof

# Ensure Python 3.11 is available and create a venv
uv venv --python 3.11

# Install the project (no dependencies yet)
uv sync

# Sanity check the package import
uv run python -c "import pysof; print(pysof.__version__)"
```

## Project layout

```
crates/pysof/
├─ pyproject.toml          # PEP 621 metadata, Python >=3.11, uv-compatible
├─ README.md
└─ src/
   └─ pysof/
      └─ __init__.py       # package root
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

  - [ ] Introduce PyO3 bindings to call Rust `sof` library
  - [ ] Switch build backend to `maturin` (PEP 621) for extension builds
  - [ ] **Core API Functions:**
    - [ ] Expose `run_view_definition(view: dict, bundle: dict, format: str) -> bytes`
    - [ ] Expose `run_view_definition_with_options(view: dict, bundle: dict, format: str, *, since: str = None, limit: int = None, page: int = None) -> bytes`
  - [ ] **Content Type Support:** Map content types: `csv`, `csv_with_header`, `json`, `ndjson`
  - [ ] **FHIR Version Support:**
    - [ ] Add optional `fhir_version` parameter (default: "R4")
    - [ ] Support R4, R4B, R5, R6 versions based on Rust feature compilation
    - [ ] Add version compatibility validation
  - [ ] **Comprehensive Error Handling:** Map Rust errors to Python exceptions:
    - [ ] `InvalidViewDefinitionError` (from `SofError::InvalidViewDefinition`)
    - [ ] `FhirPathError` (from `SofError::FhirPathError`)
    - [ ] `SerializationError` (from `SofError::SerializationError`)
    - [ ] `UnsupportedContentTypeError` (from `SofError::UnsupportedContentType`)
    - [ ] `CsvError` (from `SofError::CsvError`)
    - [ ] `IoError` (from `SofError::IoError`)
    - [ ] Base `SofError` exception class hierarchy
  - [ ] **Utility Functions:**
    - [ ] `validate_view_definition(view: dict) -> bool` - Pre-validate ViewDefinition structure
    - [ ] `validate_bundle(bundle: dict) -> bool` - Pre-validate Bundle structure
    - [ ] `parse_content_type(mime_type: str) -> str` - Parse MIME types to format strings
    - [ ] `get_supported_fhir_versions() -> List[str]` - List available FHIR versions
  - [ ] Add wheel builds for Windows, Linux (manylinux/musllinux)
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
