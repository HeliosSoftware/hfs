# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build Commands

### Building
```bash
# Build all crates
cargo build

# Build with release optimizations
cargo build --release

# Build specific crate
cargo build -p fhir
cargo build -p fhirpath
```

### Testing
```bash
# Run all tests
cargo test

# Run tests for a specific crate
cargo test -p fhirpath

# Run specific test
cargo test -p fhirpath truncate_test
cargo test -p fhirpath r4_tests

# Run tests with filter
cargo test -p fhirpath -- truncate
```

### FHIR Version Selection
The codebase supports multiple FHIR versions through Cargo features:

```bash
# R4 is the default version
cargo build

# For other versions:
cargo build --features R4B
cargo build --features R5
cargo build --features R6
```

## Code Architecture

### Project Structure
- **crates/fhir**: Core FHIR model types and operations
  - Supports multiple FHIR versions (R4, R4B, R5, R6) via feature flags
  - Includes JSON serialization/deserialization
  
- **crates/fhir_gen**: Code generation for FHIR resources
  - Contains FHIR definition files for each version
  - Used to generate Rust models

- **crates/fhir_macro**: Procedural macros supporting FHIR operations

- **crates/fhirpath**: Implementation of the FHIRPath specification
  - Evaluates FHIRPath expressions against FHIR resources
  - Parser using chumsky
  - Evaluator for FHIRPath operations

- **crates/fhirpath_support**: Common support types for FHIRPath

- **crates/hfs**: Main Helios FHIR Server application (WIP)

### FHIRPath Implementation

FHIRPath implementation is divided into:

1. **Parser** (`crates/fhirpath/src/parser.rs`): Parses FHIRPath expressions into AST
2. **Evaluator** (`crates/fhirpath/src/evaluator.rs`): Evaluates parsed expressions against FHIR resources
3. **Result Types** (`crates/fhirpath_support`): Types for evaluation results

The FHIRPath README in `crates/fhirpath/README.md` provides a comprehensive list of implemented features.

## Current Development

The git status shows active development on the FHIRPath implementation, particularly:

1. Implementing the `truncate()` function and its tests
2. Fixing issues in the evaluator
3. Improving quantity unit validation and conversion
4. Adding XML-based tests for the R4 version

When working on FHIRPath functions:
1. Implement the function in the evaluator
2. Add tests in the corresponding test files
3. Update the README.md to reflect implementation status