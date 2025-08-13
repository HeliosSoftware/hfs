[![Helios FHIR Server](https://github.com/HeliosSoftware/hfs/blob/main/github-banner.png)](https://heliossoftware.com)

# Helios FHIR Server

The Helios FHIR Server is an implementation of the [HL7® FHIR®](https://hl7.org/fhir) standard, built in Rust for high performance and optimized for clinical analytics workloads. It provides modular components that can be run as standalone command-line tools, integrated as microservices, or embedded directly into your data analytics pipeline.

## Why Helios FHIR Server?

- **🚀 Blazing Fast**: Built in Rust for maximum performance and minimal resource usage
- **📊 Analytics-First**: Optimized for clinical data analytics and research workloads
- **🔧 Modular Design**: Use only what you need - from FHIRPath expressions to full server capabilities
- **🌐 Multi-Version Support**: Work with R4, R4B, R5, and R6 data in the same application
- **🛠️ Developer Friendly**: Excellent error messages, comprehensive documentation, and CLI tools

## What People Build with the Helios FHIR Server

- **Clinical Research Platforms**: Transform FHIR data into research-ready datasets using SQL-on-FHIR
- **Real-time Analytics Dashboards**: Process streaming FHIR data for operational insights
- **Data Quality Tools**: Validate and profile FHIR data using FHIRPath expressions
- **ETL Pipelines**: Extract and transform FHIR data for data warehouses and lakes
- **Healthcare APIs**: Build high-performance FHIR-compliant REST APIs
- **Healthcare Analytics**: Analyze patient cohorts at scale


# Quick Start

## Prerequisites

1. **Install [Rust](https://www.rust-lang.org/tools/install)**
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

2. **Install [LLD](https://lld.llvm.org/)**

Linux (Ubuntu/Debian):
```bash
sudo apt install clang lld
```

macOS:
```bash
brew install llvm
```

3. **Configure Rust to use LLD**

Create or modify `~/.cargo/config.toml`:
```toml
[target.x86_64-unknown-linux-gnu]
linker = "clang"
rustflags = ["-C", "link-arg=-fuse-ld=lld"]

[target.x86_64-apple-darwin]
linker = "clang"
rustflags = ["-C", "link-arg=-fuse-ld=lld"]
```

4. **Set stack size and parallel build count** (add to `~/.bashrc` or `~/.zshrc`):
```bash
export RUST_MIN_STACK=8388608
```
💡 **Tip**: If you run out of memory during compilation on Linux, especially on high CPU core count machines, limit parallel jobs to 4 (or less):
```bash
export CARGO_BUILD_JOBS=4
```

## Installation

```bash
# Clone the repository
git clone https://github.com/HeliosSoftware/hfs.git
cd hfs

# Build (R4 only by default)
cargo build --release

# Or build with all FHIR versions
cargo build --release --features R4,R4B,R5,R6
```

## Try It Out

```bash
# Run FHIRPath expressions
echo '{"resourceType": "Patient", "id": "123"}' | cargo run --bin fhirpath-cli -- 'Patient.id'

# Transform FHIR to CSV using SQL-on-FHIR
cargo run --bin sof-cli -- --view examples/patient-view.json --bundle examples/patients.json

# Start the SQL-on-FHIR server
cargo run --bin sof-server
# Then POST to http://localhost:8080/ViewDefinition/$run

# Start the FHIRPath server
cargo run --bin fhirpath-server
# Then POST expressions to http://localhost:3000/fhirpath
```

# Architecture Overview

The Helios FHIR Server is organized as a Rust workspace with modular components that can be used independently or together. Each component is designed for high performance and can be embedded directly into your data analytics pipeline.

## Core Components

### 1. [`helios-fhir`](crates/fhir) - FHIR Data Models
Generated from FHIR StructureDefinitions, type-safe Rust representations of all FHIR resources and data types.
- Supports FHIR R4, R4B, R5, and R6 via feature flags
- JSON serialization/deserialization with full FHIR compliance
- Precision decimal handling for clinical accuracy
- Default: R4 (use `--features R4,R4B,R5,R6` for all versions)

### 2. [`helios-fhir-gen`](crates/fhir-gen) - Code Generator
Generates the FHIR data models from official HL7 specifications.
- Transforms FHIR StructureDefinitions into Rust types
- Automatically downloads the latest R6 specs from the HL7 build server
- Run with: `cargo run -p helios-fhir-gen -- --all`

### 3. [`helios-fhirpath`](crates/fhirpath) - FHIRPath Expression Engine
Complete implementation of the [FHIRPath 3.0.0-ballot specification](https://hl7.org/fhirpath/2025Jan/).
- **Executables:**
  - `fhirpath-cli` - Evaluate FHIRPath expressions from the command line
  - `fhirpath-server` - HTTP server for FHIRPath evaluation
- Parser built with chumsky for excellent error messages
- Comprehensive function library with version-aware type checking
- Auto-detects FHIR version from input data

### 4. [`helios-sof`](crates/sof) - SQL-on-FHIR Implementation
Transform FHIR resources into tabular data using [ViewDefinitions](https://sql-on-fhir.org/ig/latest/index.html).
- **Executables:**
  - `sof-cli` - Command-line tool for batch transformations
  - `sof-server` - HTTP server with `ViewDefinition/$run` operation
- Supports multiple output formats: CSV, JSON, NDJSON, Parquet (planned)
- Version-agnostic processing with automatic FHIR version detection

### 5. [`helios-fhir-macro`](crates/fhir-macro) - Procedural Macros
Helper macros for code generation used by other components.

### 6. [`helios-fhirpath-support`](crates/fhirpath-support) - Shared Utilities
Common types and traits for FHIRPath evaluation.

### 7. [`helios-hfs`](crates/hfs) - Main Server Application
The main Helios FHIR Server application (coming soon).

## Design Principles

- **Version Agnostic**: All components use enum wrappers to seamlessly handle multiple FHIR versions
- **Modular Architecture**: Each component can be used standalone or integrated
- **Type Safety**: Leverages Rust's type system for compile-time correctness
- **Performance**: Built for high-throughput clinical analytics workloads

# Features

## FHIR Version Support
- ✅ **FHIR R4** (4.0.1) - Default
- ✅ **FHIR R4B** (4.3.0)
- ✅ **FHIR R5** (5.0.0)
- ✅ **FHIR R6** (6.0.0-ballot2)

## FHIRPath Expression Language
- Complete implementation of [FHIRPath 3.0.0-ballot specification](https://hl7.org/fhirpath/2025Jan/)
- 100+ built-in functions across all categories
- HTTP server for integration with FHIRPath Lab
- Command-line tool for testing and development

## SQL-on-FHIR
- ViewDefinition-based transformation to tabular formats
- Multiple output formats: CSV, JSON, NDJSON, Parquet (planned)
- Streaming support for large datasets
- HTTP API with `$run` operation

## FHIR REST API (Coming Soon)
- Full CRUD operations
- Search with chained parameters
- History and versioning
- Batch/transaction support
# Documentation

```bash
# Generate and view API documentation
cargo doc --no-deps --open
```

# Development

## Building from Source
```bash
# Build with default features (R4 only)
cargo build

# Build with all FHIR versions
cargo build --features R4,R4B,R5,R6

# Build specific component
cargo build -p helios-fhirpath
```

## Running Tests
```bash
# Run all tests
cargo test

# Run tests for all FHIR versions
cargo test --features R4,R4B,R5,R6

# Run specific test
cargo test test_name_pattern

# Run with output
cargo test -- --nocapture
```

## Code Generation
To regenerate FHIR models from specifications:
```bash
cargo build -p helios-fhir-gen --features R6
./target/debug/fhir_gen --all
```

# Contributing

Please see our [Contributing Guidelines](CONTRIBUTING.md) for details.

# License

The Helios FHIR Server is licensed under the [MIT License](LICENSE).

# Support

- **Issues**: [GitHub Issues](https://github.com/HeliosSoftware/hfs/issues)
- **Website**: [heliossoftware.com](https://heliossoftware.com)

---

HL7® and FHIR® are registered trademarks of Health Level Seven International.
