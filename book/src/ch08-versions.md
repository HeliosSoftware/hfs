# Multi-Version FHIR Support

HFS is built around the assumption that real-world systems need to handle multiple FHIR versions simultaneously. The workspace supports R4, R4B, R5, and R6 through Cargo feature flags and version-agnostic runtime abstractions.

---

## Feature Flags

Each FHIR version is gated behind a Cargo feature flag. The default build compiles **R4 only**.

### Enabling versions at build time

```bash
# Default: R4 only
cargo build

# All versions
cargo build --features R4,R4B,R5,R6

# Specific combination
cargo build --features R4,R5

# Single crate with specific versions
cargo build -p helios-fhirpath --features R4B,R5
```

### Running tests with all versions

```bash
cargo test --features R4,R4B,R5,R6
```

### The `skip-r6-download` feature

When running `cargo clippy --all-features`, add the `skip-r6-download` feature to prevent `helios-fhir-gen` from downloading the R6 StructureDefinition files from `https://build.fhir.org/` during the build. This makes CI linting safe and fast:

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

(`--all-features` automatically activates `skip-r6-download`.)

---

## Auto-Detection of FHIR Version

Several tools can automatically detect the FHIR version from their input data, so you don't need to always specify it explicitly.

### fhirpath-server

The root endpoint (`POST /`) inspects the resource's `meta.profile`, `fhirVersion`, or structural markers to select the appropriate version model automatically. Use version-specific endpoints when you need explicit control:

```
POST /r4    → always use R4
POST /r4b   → always use R4B
POST /r5    → always use R5
POST /r6    → always use R6
```

### fhirpath-cli

Pass `--fhir-version` to override auto-detection:

```bash
fhirpath-cli --fhir-version R5 -e "Patient.name.family" -r patient.json
```

### sof-cli

```bash
sof-cli --fhir-version R4B -v view.json -b data.json
```

### EvaluationContext (library API)

```rust
use helios_fhirpath::evaluator::EvaluationContext;
use helios_fhir::FhirVersion;

// Version auto-detected from resources
let ctx = EvaluationContext::new(fhir_resources);

// Explicit version
let ctx = EvaluationContext::new_with_version(fhir_resources, FhirVersion::R5);

// Empty context with explicit version (no resource)
let ctx = EvaluationContext::new_empty(FhirVersion::R4);
```

---

## Version-Aware Type Checking

### The `FhirVersion` enum

`helios-fhir` exports a `FhirVersion` enum used throughout the workspace to carry version information at runtime:

```rust
pub enum FhirVersion {
    R4,
    R4B,
    R5,
    R6,
}
```

### Version-agnostic enum wrappers

Rather than generic type parameters, the workspace uses enum wrappers to handle multiple versions in a single code path. This avoids monomorphization explosion and keeps the API ergonomic:

```rust
pub enum SofViewDefinition {
    R4(fhir::r4::ViewDefinition),
    R4B(fhir::r4b::ViewDefinition),
    R5(fhir::r5::ViewDefinition),
    R6(fhir::r6::ViewDefinition),
}

pub enum SofBundle {
    R4(fhir::r4::Bundle),
    R4B(fhir::r4b::Bundle),
    R5(fhir::r5::Bundle),
    R6(fhir::r6::Bundle),
}
```

Traits like `ViewDefinitionTrait` and `BundleTrait` abstract over these enums so application code does not need to pattern-match on the version.

### `ofType()` with namespace qualification

FHIRPath type checks are version-aware. Use the `FHIR.` namespace prefix for FHIR resource and data types:

```fhirpath
# Check if a value is a FHIR Quantity (any version)
value.ofType(FHIR.Quantity)

# Check a choice type
Observation.value.ofType(FHIR.boolean)

# Type-based navigation (is / as)
value is FHIR.Quantity
value as FHIR.Quantity
```

The `FhirResourceTypeProvider` trait is implemented for each version's `Resource` enum and is used by the FHIRPath evaluator to validate resource type names at evaluation time. Resource type lists are generated at compile time from the actual FHIR specification — they are never hardcoded.

### Version in HFS server

Set the default FHIR version the server uses when a client does not specify:

```bash
HFS_DEFAULT_FHIR_VERSION=R5 hfs
```

Valid values: `R4`, `R4B`, `R5`, `R6`. Default is `R4`.
