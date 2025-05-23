# FHIR Macro - Procedural Macros for FHIR Implementation

This crate provides procedural macros that enable automatic code generation for FHIR (Fast Healthcare Interoperability Resources) implementations in Rust. It contains the core macro functionality that powers serialization, deserialization, and FHIRPath evaluation.

## Overview

The `fhir_macro` crate implements two essential derive macros that handle the complex serialization patterns required by the FHIR specification:

- **`#[derive(FhirSerde)]`** - Custom serialization/deserialization handling FHIR's JSON representation including it's extension pattern
- **`#[derive(FhirPath)]`** - Automatic conversion to FHIRPath evaluation results for resource traversal

These macros are automatically applied to thousands of generated FHIR types, eliminating the need for hand-written serialization code while ensuring compliance with FHIR's serialization requirements.

## Derive Macros

### `#[derive(FhirSerde)]`

The `FhirSerde` derive macro automatically implements `serde::Serialize` and `serde::Deserialize` for FHIR types, handling the serialization patterns required by the FHIR specification.

#### Key Features

- **Extension Pattern Handling**: Automatically manages FHIR's `_fieldName` extension pattern
- **Element Container Support**: Handles `Element<T, Extension>` and `DecimalElement<Extension>` types
- **Primitive Array Serialization**: Correctly serializes arrays with mixed primitive and extension data
- **Choice Type Support**: Handles FHIR choice types (e.g., `value[x]` fields) with proper renaming
- **Flattening Support**: Supports field flattening for inheritance patterns

#### Supported Attributes

```rust
// Field renaming for FHIR naming conventions
#[fhir_serde(rename = "implicitRules")]
pub implicit_rules: Option<Uri>,

// Field flattening for choice types and inheritance
#[fhir_serde(flatten)]
pub subject: Option<ActivityDefinitionSubject>,
```

#### Usage Examples

```rust
use fhir_macro::FhirSerde;

// Basic struct with FHIR serialization
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Patient {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub active: Option<Boolean>,  // Element<bool, Extension>
    pub name: Option<Vec<HumanName>>,
}

// Choice type enum with proper serialization
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde)]
pub enum ObservationValue {
    #[fhir_serde(rename = "valueQuantity")]
    Quantity(Quantity),
    #[fhir_serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    #[fhir_serde(rename = "valueString")]
    String(String),
}
```

#### FHIR Extension Pattern

The macro automatically handles FHIR's extension pattern where primitive elements can have extensions:

```json
// Input JSON
{
  "status": "active",
  "_status": {
    "id": "status-1",
    "extension": [...]
  }
}
```

```rust
// Generated serialization code handles both parts automatically
pub struct SomeResource {
    pub status: Option<Code>,  // Element<String, Extension>
}
```

### `#[derive(FhirPath)]`

The `FhirPath` derive macro automatically implements the `fhirpath_support::IntoEvaluationResult` trait, enabling FHIR resources to be used in FHIRPath expressions.

#### Key Features

- **Object Conversion**: Converts structs to `EvaluationResult::Object` with field mappings
- **Enum Handling**: Supports both choice types and resource enums
- **Resource Type Injection**: Automatically adds `resourceType` field for Resource enum variants
- **Empty Field Filtering**: Excludes empty/None fields from the result object
- **Proper Field Naming**: Uses FHIR field names (respecting `#[fhir_serde(rename)]`)

#### Usage Examples

```rust
use fhir_macro::FhirPath;
use fhirpath_support::{IntoEvaluationResult, EvaluationResult};

// Struct conversion to FHIRPath object
#[derive(FhirPath)]
pub struct Patient {
    pub id: Option<String>,
    pub active: Option<Boolean>,
    pub name: Option<Vec<HumanName>>,
}

// Usage in FHIRPath evaluation
let patient = Patient { 
    id: Some("123".to_string()),
    active: Some(Boolean::from(true)),
    name: Some(vec![...]),
};

let result = patient.into_evaluation_result();
// Results in EvaluationResult::Object with fields: id, active, name
```

## Macro Implementation Details

### Extension Pattern Implementation

The `FhirSerde` macro implements FHIR's complex extension pattern:

```rust
// For fields like: pub status: Option<Code>
// Where Code = Element<String, Extension>

// Serializes as:
{
  "status": "active",           // The primitive value
  "_status": {                  // Extension object if present
    "id": "status-1",
    "extension": [...]
  }
}

// Deserializes by:
// 1. Collecting both "status" and "_status" fields
// 2. Constructing Element<String, Extension> with both parts
// 3. Handling cases where only primitive or only extension exists
```

### Choice Type Handling

Choice types (FHIR's `[x]` fields) are handled through enum serialization:

```rust
// Generated enum
pub enum ObservationValue {
    #[fhir_serde(rename = "valueQuantity")]
    Quantity(Quantity),
    #[fhir_serde(rename = "valueString")]  
    String(String),
}

// Serializes as single key-value pair:
{ "valueQuantity": {...} }  // for Quantity variant
{ "valueString": "text" }   // for String variant
```

### Array Serialization

For arrays of Element types, the macro implements FHIR's split array pattern:

```rust
// Field: pub given: Option<Vec<String>>  // Vec<Element<String, Extension>>

// Serializes as:
{
  "given": ["John", "Michael", null],     // Primitive array
  "_given": [null, {"id": "name-2"}, {}] // Extension array  
}
```

## Macro Attributes

### `#[fhir_serde(rename = "...")]`

Renames fields during serialization to match FHIR naming conventions:

```rust
#[fhir_serde(rename = "modifierExtension")]
pub modifier_extension: Option<Vec<Extension>>,

// Serializes as "modifierExtension" not "modifier_extension"
```

### `#[fhir_serde(flatten)]`

Flattens fields into the parent object for choice types and inheritance:

```rust
pub struct ActivityDefinition {
    pub name: Option<String>,
    #[fhir_serde(flatten)]
    pub subject: Option<ActivityDefinitionSubject>, // Choice type enum
}

// Flattens the enum variant directly into the parent:
{
  "name": "My Activity",
  "subjectCodeableConcept": {...}  // From flattened choice
}
```

## Usage Throughout the Codebase

### Generated FHIR Types

All generated FHIR types in the `fhir` crate automatically use these macros:

```rust
// In crates/fhir/src/r4.rs, r4b.rs, r5.rs, r6.rs:
use fhir_macro::{FhirSerde, FhirPath};

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Patient {
    // ... fields with proper attributes
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]  
pub enum ObservationValue {
    #[fhir_serde(rename = "valueQuantity")]
    Quantity(Quantity),
    // ... other variants
}
```

### Integration with FHIRPath

The `FhirPath` derive enables seamless integration with the FHIRPath evaluator:

```rust
// In crates/fhirpath/src/evaluator.rs:
use fhirpath_support::IntoEvaluationResult;

// All FHIR types can be used directly in FHIRPath expressions
fn evaluate_expression(resource: &Patient, path: &str) -> EvaluationResult {
    // Patient automatically implements IntoEvaluationResult via #[derive(FhirPath)]
    resource.into_evaluation_result()
}
```

### Code Generation Integration

The `fhir_gen` crate automatically applies these derives:

```rust
// In crates/fhir_gen/src/lib.rs:
let struct_derives = vec!["Debug", "Clone", "PartialEq", "Eq", "FhirSerde", "FhirPath", "Default"];
output.push_str(&format!("#[derive({})]\n", struct_derives.join(", ")));

// For enums:
let enum_derives = vec!["Debug", "Clone", "PartialEq", "Eq", "FhirSerde", "FhirPath"];
```

## Testing and Validation

The macro implementations include comprehensive tests covering:

- **Element detection**: Correctly identifying FHIR element types
- **Extension pattern**: Proper handling of `_fieldName` patterns  
- **Array serialization**: Split arrays for primitives and extensions
- **Choice types**: Enum serialization with proper renaming
- **Flattening**: Field flattening for inheritance patterns
- **Edge cases**: Null handling, empty arrays, mixed data

