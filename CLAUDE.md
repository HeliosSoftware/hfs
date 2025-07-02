# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build and Development Commands

### Building
```bash
# Build default (R4 only)
cargo build

# Build with all FHIR versions
cargo build --features R4,R4B,R5,R6

# Build specific crate
cargo build -p sof
cargo build -p fhirpath
```

### Testing
```bash
# Run all tests (default R4)
cargo test

# Test with all FHIR versions
cargo test --features R4,R4B,R5,R6

# Test specific crate
cargo test -p sof
cargo test -p fhirpath

# Run single test
cargo test test_name_pattern

# Run tests in specific file
cargo test --test test_file_name

# Show test output
cargo test -- --nocapture
```

### Linting and Formatting
```bash
# Lint code
cargo clippy

# Format code
cargo fmt

# Check types without building
cargo check
```

### Documentation
```bash
# Generate and view docs
cargo doc --no-deps --open
```

### FHIR Code Generation
```bash
# Generate FHIR models for all versions
cargo build -p fhir_gen --features R6
./target/debug/fhir_gen --all

# Note: R6 specification files are auto-downloaded from HL7 build server
```

### SQL-on-FHIR Executables
```bash
# Run CLI tool
cargo run --bin sof-cli -- --view view.json --bundle data.json --format csv

# Run HTTP server (default port 3000)
cargo run --bin sof-server

# With environment variables
SOF_SERVER_PORT=8080 SOF_LOG_LEVEL=debug cargo run --bin sof-server
```

## Architecture Overview

### Workspace Structure
The project is a Rust workspace with 7 main crates:

1. **`fhir`** - Core FHIR data models (auto-generated)
   - Supports R4, R4B, R5, R6 via feature flags
   - Version-specific modules: `r4.rs`, `r4b.rs`, etc.
   - Code generated from official FHIR JSON schemas

2. **`fhir_gen`** - Code generator for FHIR models
   - Generates Rust structs from FHIR JSON schemas
   - Run with `./target/debug/fhir_gen --all` after building
   - R6 specs auto-downloaded from HL7 build server

3. **`fhirpath`** - FHIRPath expression language implementation
   - Parser based on ANTLR grammar
   - Comprehensive function support (see README for feature matrix)
   - Version-aware type checking with auto-detection
   - Namespace resolution for FHIR and System types

4. **`sof`** - SQL-on-FHIR implementation (actively developed)
   - Two binaries: `sof-cli` and `sof-server`
   - ViewDefinition processing for tabular data transformation
   - HTTP API with Axum framework
   - `$run` operation with extensive parameters

5. **`fhir_macro`** - Procedural macros for FHIR functionality

6. **`fhirpath_support`** - Support utilities for FHIRPath

7. **`hfs`** - Main server application (placeholder)

### Key Design Patterns

#### Version-Agnostic Abstraction
The codebase uses enum wrappers and traits to handle multiple FHIR versions:

```rust
// Example from sof crate
pub enum SofViewDefinition {
    R4(fhir::r4::ViewDefinition),
    R4B(fhir::r4b::ViewDefinition),
    R5(fhir::r5::ViewDefinition),
    R6(fhir::r6::ViewDefinition),
}
```

#### Trait-Based Processing
Core functionality is defined through traits, allowing version-independent logic:
- `ViewDefinitionTrait`
- `BundleTrait`
- `ResourceTrait`

### Active Development Areas
Currently focused on the `sof` crate:
- Implementation of `$run` operation
- Server API enhancements
- Parameter validation
- Test coverage improvements

## Environment Setup

### Required Environment Variable
```bash
export RUST_MIN_STACK=8388608
```

### LLD Linker Configuration
Add to `~/.cargo/config.toml`:
```toml
[target.x86_64-unknown-linux-gnu]
linker = "clang"
rustflags = ["-C", "link-arg=-fuse-ld=lld"]
```

### Memory-Constrained Builds
```bash
export CARGO_BUILD_JOBS=4
```

## SOF Server Configuration

### Environment Variables
- `SOF_SERVER_PORT` - Server port (default: 3000)
- `SOF_SERVER_HOST` - Host to bind (default: 0.0.0.0)
- `SOF_LOG_LEVEL` - Log level: error, warn, info, debug, trace (default: info)
- `SOF_MAX_BODY_SIZE` - Max request body size in bytes (default: 10485760)
- `SOF_REQUEST_TIMEOUT` - Request timeout in seconds (default: 300)
- `SOF_CORS_ALLOWED_ORIGINS` - Comma-separated CORS origins
- `SOF_CORS_ALLOWED_METHODS` - Allowed HTTP methods
- `SOF_CORS_ALLOWED_HEADERS` - Allowed headers
- `SOF_CORS_MAX_AGE` - CORS max age in seconds

### API Endpoints
- `GET /metadata` - Returns CapabilityStatement
- `POST /ViewDefinition/$run` - Execute ViewDefinition transformation
  - Query parameters:
    - `_format` - Output format (csv, ndjson, json)
    - `_header` - CSV header control (true/false)
    - `_count` - Limit results (1-10000)
    - `_page` - Page number for pagination
    - `_since` - Filter by modification time
  - Parameter precedence: Request body > Query params > Accept header

## Testing Patterns

### FHIRPath Tests
- Test cases in `crates/fhirpath/tests/`
- Official FHIR test cases from `fhir-test-cases` repository

### SQL-on-FHIR Tests
- Unit tests in `src/` files
- Integration tests in `tests/` directory
- Parameter validation tests being added

### Test Data
- FHIR examples in `crates/fhir/tests/data/`
- ViewDefinition examples in test files

## Common Development Tasks

### Adding a New FHIRPath Function
1. Add function implementation in appropriate module under `crates/fhirpath/src/`
2. Update parser if needed in `parser.rs`
3. Add test cases covering the function
4. Update feature matrix in README.md

### Working with ViewDefinitions
1. ViewDefinition JSON goes through version-specific parsing
2. Wrapped in `SofViewDefinition` enum for version-agnostic processing
3. Use `run_view_definition()` for transformation

### Debugging Tips
- Use `cargo test -- --nocapture` to see println! output
- Enable trace logging: `RUST_LOG=trace cargo run`
- FHIRPath expressions can be tested independently

## Important Notes

- Default FHIR version is R4 when no features specified
- The project follows standard Rust conventions
- Git status shows active work on `sof` crate files
- Server returns appropriate HTTP status codes and FHIR OperationOutcomes for errors