# fhir_gen

The **fhir_gen** crate is module that serves as the cornerstone for generating Rust code from official FHIR (Fast Healthcare Interoperability Resources) specifications. This tool transforms FHIR StructureDefinitions into Rust types, enabling type-safe interaction with FHIR resources across multiple specification versions.

## Purpose and Scope

**fhir_gen** has a singular, focused responsibility: **generate Rust code for each supported FHIR version**. It bridges the gap between the JSON-based FHIR specification files and the strongly-typed Rust ecosystem, providing:

- **Type-safe FHIR resource representations** with proper serialization/deserialization
- **Multi-version support** for FHIR R4, R4B, R5, and R6
- **Automated code generation** from official FHIR StructureDefinitions
- **Minimal bootstrap dependencies** with hand-coded types only where necessary

The generated code powers the entire FHIR ecosystem in this project, serving as input for the main `fhir` crate and enabling FHIRPath operations in the `fhirpath` crate.

## Architecture

### Minimal Bootstrap Model

The crate includes a carefully curated set of hand-coded Rust structures and enums in `src/initial_fhir_model.rs`. These types represent the minimal subset necessary to:

- **Parse FHIR StructureDefinitions** from the specification JSON files
- **Extract type information** needed for code generation
- **Bootstrap the generation process** without circular dependencies

Key bootstrap types include:
- `StructureDefinition` - Core FHIR structure definition format
- `ElementDefinition` - Individual element specifications within structures
- `Bundle` - Container for collections of FHIR resources
- `Extension` - FHIR extension mechanism with choice types
- Basic primitive types (`Address`, `Coding`, `CodeableConcept`, etc.)

These hand-coded types are the absolute minimum required to read and process the FHIR specification files themselves.

### Code Generation Process

1. **Resource Loading**: Loads official FHIR specification files from `resources/{VERSION}/`
2. **Definition Parsing**: Uses the minimal bootstrap model to parse StructureDefinitions
3. **Type Analysis**: Extracts type hierarchies, properties, and constraints
4. **Code Generation**: Generates idiomatic Rust code with proper derive macros
5. **Output Writing**: Creates version-specific modules (e.g., `r4.rs`, `r5.rs`)

### Supported FHIR Versions

- **R4** - FHIR Release 4 (current default, widely adopted)
- **R4B** - FHIR Release 4B (interim release with errata)
- **R5** - FHIR Release 5 (current standard)
- **R6** - FHIR Release 6 (upcoming release)

For released versions of the FHIR Specification (R4, R4B, and R5), specification files are copied from the FHIR Specification
and maintained in this repo in the `resources` folder. 

For the latest upcoming release (R6), the specification files are automatically downloaded from `https://build.fhir.org/definitions.json.zip` during the build process. To trigger this download, build the crate with the R6 feature enabled:

```bash
cargo build -p fhir_gen --features R6
```

The build script (`build.rs`) will automatically fetch the latest R6 specification files from HL7's build server and extract them to the `resources/R6/` directory, ensuring you always have the most current development version.  


## Command Line Interface

The **fhir_gen** binary provides a simple command-line interface for code generation:

### Basic Usage

```bash
# Generate code for default version (R4)
cargo run -p fhir_gen

# Generate code for a specific FHIR version
cargo run -p fhir_gen R5
cargo run -p fhir_gen R4B
cargo run -p fhir_gen R6

# Generate code for all supported versions
cargo run -p fhir_gen --all
```

### Available Commands

```
FHIR Generator - Process FHIR definitions

Usage: fhir_gen [OPTIONS] [VERSION]

Arguments:
  [VERSION]  FHIR version to process
             [possible values: R4, R4B, R5, R6]

Options:
  -a, --all   Process all versions
  -h, --help  Print help
```

### Examples

```bash
# Generate only R5 code (for latest standard development)
fhir_gen R5

# Generate all versions (for comprehensive compatibility)
fhir_gen --all

# Use from build scripts or CI/CD
./target/debug/fhir_gen R4
```

### Output Location

Generated code is written to `crates/fhir/src/` with version-specific modules:
- `crates/fhir/src/r4.rs` - R4 generated types
- `crates/fhir/src/r4b.rs` - R4B generated types  
- `crates/fhir/src/r5.rs` - R5 generated types
- `crates/fhir/src/r6.rs` - R6 generated types

## Integration with Build Process

### Development Workflow

1. **Initial Setup**: Run `fhir_gen --all` to generate all version code
2. **Version-Specific Development**: Use `fhir_gen R5` for focused development
3. **Testing**: Generated code integrates with the broader test suite
4. **CI/CD**: Include generation step in automated builds

### Feature Flags

The generated code works with Cargo feature flags in the main `fhir` crate:

```bash
# Build with specific FHIR version
cargo build --features R5

# Build with multiple versions
cargo build --features "R4,R5"

# Default to R4 if no features specified
cargo build
```

### Build Integration

Add to your build script or Makefile:

```bash
# Complete build process
export RUST_MIN_STACK=8388608
cargo run -p fhir_gen -- --all
cargo build --features R4,R4B,R5,R6
cargo test --features R4,R4B,R5,R6
```

## Resource Management

### FHIR Specification Files

Resources are organized by version in `resources/{VERSION}/`:

```
resources/
├── R4/
│   ├── profiles-resources.json    # Core resources
│   ├── profiles-types.json        # Data types
│   ├── profiles-others.json       # Extensions
│   ├── valuesets.json            # Terminology
│   ├── conceptmaps.json          # Mappings
│   ├── search-parameters.json    # Search defs
│   └── version.info              # Version metadata
├── R4B/ [same structure]
├── R5/  [same structure]
└── R6/  [same structure]
```

## Generated Code Characteristics

The code generator produces:

- **Strongly-typed structs** for all FHIR resources and data types
- **Serde integration** for JSON serialization/deserialization
- **Derive macros** for common operations (Debug, Clone, etc.)
- **Choice type handling** for FHIR's polymorphic elements (e.g., `value[x]`)
- **Option wrapping** for optional FHIR elements
- **Vec collections** for FHIR arrays and lists
- **Documentation** extracted from FHIR definitions

### Example Generated Code

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Patient {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub active: Option<bool>,
    pub name: Option<Vec<HumanName>>,
    pub gender: Option<String>,
    pub birth_date: Option<String>,
    // ... additional fields
}
```

## Error Handling

The generator includes comprehensive error handling:

- **Parse errors** for malformed FHIR specification files
- **Type resolution errors** for missing or circular dependencies  
- **File I/O errors** with clear diagnostic messages
- **Version compatibility warnings** for specification changes

Failed parsing of individual files produces warnings but doesn't halt the generation process, allowing partial code generation when possible.

## Development and Maintenance

### Adding New FHIR Versions

1. Create new directory under `resources/NEW_VERSION/`
2. Add FHIR specification files for the new version
3. Update `FhirVersion` enum in the main `fhir` crate
4. Test generation and integration with existing code
5. Update documentation and examples

### Extending the Bootstrap Model

When FHIR introduces new structural elements:

1. Add minimal types to `initial_fhir_model.rs`
2. Focus only on what's needed for parsing StructureDefinitions
3. Keep bootstrap types as simple as possible
4. Let the generator create the full, rich types

### Code Generation Improvements

Areas for potential enhancement:

- **Performance optimization** for large specification files
- **Incremental generation** to avoid regenerating unchanged types
- **Custom derive macro integration** for FHIR-specific operations
- **Documentation generation** from FHIR specification text
- **Validation rule generation** from FHIR constraints

## Dependencies

**fhir_gen** maintains minimal dependencies:

- `serde` and `serde_json` - JSON parsing and serialization
- `clap` - Command-line argument parsing  
- `fhir` - Access to FhirVersion enum (circular dependency managed carefully)

This lean dependency set ensures fast compilation and reduces the risk of dependency conflicts in the broader ecosystem.

## Status and Future

**fhir_gen** is a mature, stable component that successfully generates working Rust code for all supported FHIR versions. Future development focuses on:

- **Specification updates** as new FHIR versions are released
- **Generation performance** improvements for faster development cycles
- **Code quality enhancements** in generated output
- **Integration improvements** with IDE tooling and development workflows

The crate serves as the foundation for the entire FHIR ecosystem in this project and maintains backward compatibility while supporting the latest FHIR specifications.
