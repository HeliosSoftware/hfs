# FHIR 

The **helios-fhir** crate is the comprehensive FHIR (Fast Healthcare Interoperability Resources) specification model implementation that contains strongly-typed Rust representations of all FHIR data types, and resourcess for each supported version of the FHIR specification.

## Purpose and Scope

**fhir** serves as the central module for FHIR specification model code, providing:

- **Complete FHIR Type System**: Rust implementations of all FHIR resources, data types, and primitive types
- **Multi-Version Support**: Support for FHIR R4, R4B, R5, and R6 specifications
- **Type Safety**: Compile-time guarantees for FHIR data structure correctness
- **Serialization/Deserialization**: Full JSON compatibility with official FHIR examples
- **FHIRPath Integration**: Native support for FHIRPath expressions through generated traits

The crate transforms the official HL7 FHIR specifications into idiomatic, type-safe Rust code that can be used in healthcare applications, research tools, and interoperability systems.

## Architecture Overview

### Version-Based Module Structure

The crate is organized around FHIR specification versions, with each version contained in its own module:

```rust
// Access different FHIR versions
use helios_fhir::r4::Patient;   // FHIR R4 Patient resource
use helios_fhir::r4b::Patient;  // FHIR R4B Patient resource
use helios_fhir::r5::Patient;   // FHIR R5 Patient resource
use helios_fhir::r6::Patient;   // FHIR R6 Patient resource
```

### Generated vs Hand-Coded Content

The crate combines:

1. **Generated Code** (95%): Version-specific modules (`r4.rs`, `r4b.rs`, `r5.rs`, `r6.rs`) containing:
   - All FHIR resource types (Patient, Observation, Encounter, etc.)
   - All FHIR data types (HumanName, Address, CodeableConcept, etc.)
   - All FHIR primitive types (string, integer, decimal, boolean, etc.)
   - Choice type enums for polymorphic elements
   - Resource collection enums

2. **Hand-Coded Infrastructure** (`lib.rs`): Foundational types that support the generated code:
   - `Element<T, Extension>` - Base container for FHIR elements with extensions
   - `DecimalElement<Extension>` - Specialized decimal handling with precision preservation
   - `PreciseDecimal` - High-precision decimal arithmetic
   - `FhirVersion` - Version enumeration and utilities

### Code Generation Process

The FHIR types are generated from official HL7 specification files by the `fhir_gen` crate:

1. **Source**: Official FHIR StructureDefinition JSON files from HL7
2. **Processing**: `fhir_gen` parses specifications and generates Rust code
3. **Output**: Complete type-safe Rust implementations in version-specific modules
4. **Integration**: Generated code uses hand-coded infrastructure types

## Supported FHIR Versions

### Version Coverage

| FHIR Version | Status | Module | Resources | Data Types | Features |
|--------------|--------|--------|-----------|------------|----------|
| **R4** | ✅ Current Default | `fhir::r4` | ~150 resources | ~60 data types | Mature, widely adopted |
| **R4B** | ✅ Supported | `fhir::r4b` | ~150 resources | ~60 data types | Errata and clarifications |
| **R5** | ✅ Current Standard | `fhir::r5` | ~180 resources | ~70 data types | Latest features |
| **R6** | ✅ Latest | `fhir::r6` | ~190 resources | ~75 data types | Cutting-edge development |

### Feature Flag System

The crate uses Cargo feature flags to control which FHIR versions are compiled.  For example:

```toml
[dependencies]
helios-fhir = { version = "0.1.0", features = ["R5"] }           # R5 only
helios-fhir = { version = "0.1.0", features = ["R4", "R5"] }     # R4 and R5
helios-fhir = { version = "0.1.0" }                              # R4 (default)
```


## Generated Code Features

### Resource Types

Every FHIR resource is represented as a strongly-typed Rust struct:

```rust
use helios_fhir::r5::{Patient, HumanName, Identifier};

let patient = Patient {
    id: Some("patient-123".to_string()),
    identifier: Some(vec![
        Identifier {
            system: Some("http://hospital.smarthealthit.org".to_string()),
            value: Some("12345".to_string()),
            ..Default::default()
        }
    ]),
    name: Some(vec![
        HumanName {
            family: Some("Doe".to_string()),
            given: Some(vec!["John".to_string()]),
            ..Default::default()
        }
    ]),
    ..Default::default()
};
```

### Data Types

All FHIR data types are available as Rust structs:

```rust
use helios_fhir::r5::{Address, ContactPoint, CodeableConcept, Coding};

let address = Address {
    line: Some(vec!["123 Main St".to_string()]),
    city: Some("Anytown".to_string()),
    state: Some("NY".to_string()),
    postal_code: Some("12345".to_string()),
    country: Some("US".to_string()),
    ..Default::default()
};
```

### Primitive Types

FHIR primitive types are implemented with proper constraint handling:

```rust
use helios_fhir::r5::{Boolean, String as FhirString, Integer, Decimal};

// FHIR primitives include extension support
let enabled: Boolean = true.into();
let name: FhirString = "Patient Name".to_string().into();
let count: Integer = 42.into();
```

### Choice Types (Polymorphic Elements)

FHIR choice elements (ending in `[x]`) are represented as enums:

```rust
use helios_fhir::r5::{Observation, ObservationValue};

let observation = Observation {
    value: Some(ObservationValue::String("Normal".to_string())),
    // or
    // value: Some(ObservationValue::Quantity(quantity_value)),
    // or  
    // value: Some(ObservationValue::CodeableConcept(coded_value)),
    ..Default::default()
};
```

### Resource Collection

Each version provides a unified Resource enum containing all resource types:

```rust
use helios_fhir::r5::Resource;

let resource = Resource::Patient(patient);
let json = serde_json::to_string(&resource)?;

// Deserialize from JSON
let parsed: Resource = serde_json::from_str(&json)?;
```

## Serialization and Deserialization

### JSON Compatibility

All generated types are fully compatible with official FHIR JSON representations:

```rust
use helios_fhir::r5::Patient;
use serde_json;

// Deserialize from FHIR JSON
let fhir_json = r#"{
    "resourceType": "Patient",
    "id": "example",
    "name": [
        {
            "family": "Doe",
            "given": ["John"]
        }
    ]
}"#;

let patient: Patient = serde_json::from_str(fhir_json)?;

// Serialize back to JSON
let json = serde_json::to_string_pretty(&patient)?;
```

### Precision Handling

The crate includes specialized handling for FHIR's decimal precision requirements:

```rust
use helios_fhir::r5::Decimal;
use rust_decimal::Decimal as RustDecimal;

// Preserves original string precision
let precise_value = Decimal::from_string("12.340".to_string());

// Mathematical operations maintain precision
// RustDecimal::new(1, 1) creates 1 × 10^(-1) = 0.1
let calculated = precise_value + RustDecimal::new(1, 1); // add 0.1 = 12.440
```

## FHIRPath Integration

### Generated Traits

All FHIR types automatically implement FHIRPath-compatible traits:

```rust
use helios_fhir::r5::Patient;
use fhirpath_support::IntoEvaluationResult;

let patient = Patient::default();

// Convert to FHIRPath evaluation context
let result = patient.into_evaluation_result();

// Use with FHIRPath expressions
let name_result = evaluate("name.given", &patient)?;
```

### Property Access

Generated types support FHIRPath property access patterns:

```rust
// FHIRPath: Patient.name.family
let family_names = patient.name
    .unwrap_or_default()
    .into_iter()
    .filter_map(|name| name.family)
    .collect::<Vec<_>>();
```

## Testing and Validation

### Comprehensive Test Suite

The crate includes extensive testing against official FHIR examples:

```bash
# Test all versions with official examples
cargo test --features "R4,R4B,R5,R6"

# Test specific version
cargo test --features R5

# Test serialization/deserialization
cargo test test_serde
```

### Example Data

Each FHIR version includes hundreds of official example resources:

- **R4**: 2,000+ official examples from HL7
- **R4B**: Updated examples with errata fixes  
- **R5**: 2,500+ examples with new resource types
- **R6**: Latest examples from ongoing development

### Validation Coverage

Tests verify:
- **Round-trip serialization**: JSON → Rust → JSON consistency
- **Schema compliance**: Generated types match FHIR specifications
- **Example compatibility**: All official examples deserialize correctly
- **Type safety**: Compile-time guarantees for data structure integrity

## Usage Patterns

### Basic Resource Creation

```rust
use helios_fhir::r5::{Patient, HumanName, ContactPoint};

let patient = Patient {
    name: Some(vec![HumanName {
        family: Some("Smith".to_string()),
        given: Some(vec!["Jane".to_string()]),
        ..Default::default()
    }]),
    telecom: Some(vec![ContactPoint {
        system: Some("email".to_string()),
        value: Some("jane.smith@example.com".to_string()),
        ..Default::default()
    }]),
    ..Default::default()
};
```

### Working with Extensions

```rust
use helios_fhir::r5::{Patient, Extension};

let mut patient = Patient::default();

// Add custom extension
patient.extension = Some(vec![Extension {
    url: "http://example.org/custom-field".to_string(),
    value_string: Some("Custom Value".to_string()),
    ..Default::default()
}]);
```

### Resource Collections and Bundles

```rust
use helios_fhir::r5::{Bundle, BundleEntry, Resource};

let bundle = Bundle {
    entry: Some(vec![
        BundleEntry {
            resource: Some(Resource::Patient(patient1)),
            ..Default::default()
        },
        BundleEntry {
            resource: Some(Resource::Observation(observation1)),
            ..Default::default()
        },
    ]),
    ..Default::default()
};
```
