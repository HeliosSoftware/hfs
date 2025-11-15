# helios-serde

Format-aware serialization and deserialization support for FHIR resources.

## Overview

This crate provides a context-driven serialization architecture that enables FHIR types to be serialized to multiple formats (JSON, XML, etc.) with format-specific behavior. The implementation uses compile-time type parameters to achieve zero-overhead format specialization.

## Architecture

### The Problem: Rust's Orphan Rules

When implementing serialization for FHIR types across multiple formats, we encounter Rust's orphan rule: you cannot implement a foreign trait (like `serde::Serialize`) for a type when both the trait and the type are defined in external crates.

For example, if we tried to implement `Serialize` for `SerializationContext<&Patient, Json>`:
- `Serialize` is foreign (defined in serde)
- `SerializationContext` is foreign from helios-fhir's perspective (defined in helios-serde)
- Even though `Patient` is local, it's "covered" by the wrapper types

This violates Rust's orphan rules and won't compile.

### The Solution: Bridging Traits

We solve this using a **bridging trait pattern** that breaks the problem into two legal steps:

```
┌───────────────────┐
│  helios-serde     │
│                   │
│  - Json marker    │
│  - Contexts       │
│  - FhirSerialize  │ ◄─── Bridging trait (defined here)
│  - FhirDeserialize│
└───────────────────┘
        ▲
        │ depends on
        │
┌───────────────────┐
│  helios-fhir      │
│                   │
│  - Patient        │───► implements FhirSerialize<Json>
│  - Observation    │───► implements FhirDeserialize<Json>
│  - etc.           │
└───────────────────┘
```

#### Step 1: Bridging Traits in helios-serde

```rust
pub trait FhirSerialize<F> {
    fn fhir_serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer;
}

pub trait FhirDeserialize<F>: Sized {
    fn fhir_deserialize<'de, D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>;
}
```

#### Step 2: Implement Bridging Traits in helios-fhir

The helios-fhir crate implements these bridging traits for all FHIR types:

```rust
impl FhirSerialize<Json> for Patient {
    fn fhir_serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // ... actual serialization logic
    }
}
```

This is **legal** because:
- The trait (`FhirSerialize`) is a dependency, not foreign
- The type (`Patient`) is local to helios-fhir

#### Step 3: Blanket Implementation Connects Everything

Back in helios-serde, we provide a blanket implementation:

```rust
impl<T, F> Serialize for SerializationContext<&T, F>
where
    T: FhirSerialize<F>,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.data.fhir_serialize(serializer)
    }
}
```

This is **legal** because:
- `SerializationContext` is local to helios-serde
- This satisfies Rust's orphan rules

## Usage

### Basic Serialization

```rust
use helios_fhir::r4::Patient;
use helios_serde::json;

let patient = Patient::default();

// Serialize to JSON string
let json_string = json::to_string(&patient)?;

// Deserialize from JSON string
let patient: Patient = json::from_str(&json_string)?;
```

### Direct Context Usage

```rust
use helios_serde::{SerializationContext, Json};

let patient = Patient::default();
let ctx = SerializationContext::<_, Json>::json(&patient);

// ctx now implements Serialize via the blanket impl
let json_string = serde_json::to_string(&ctx)?;
```

## Format Support

Currently supported formats:
- **JSON**: Full support with FHIR primitive extension handling

Planned formats:
- **XML**: Coming soon
- **CSV**: Planned for tabular resources

## Implementation Details

### Format Markers

Format marker types are zero-sized types used for compile-time dispatch:

```rust
#[derive(Debug, Clone, Copy)]
pub struct Json;
```

These have no runtime overhead - they exist purely to enable compile-time format selection.

### Context Types

#### SerializationContext<V, F>

Wraps a value with format information for serialization:

```rust
pub struct SerializationContext<V, F> {
    pub data: V,
    _format: PhantomData<F>,
}
```

#### DeserializationContext<V, F>

Provides format-aware deserialization via `DeserializeSeed`:

```rust
pub struct DeserializationContext<V, F> {
    _value: PhantomData<V>,
    _format: PhantomData<F>,
}
```

### Generic Type Support

The crate provides generic implementations for common Rust types:

- `Option<T>` - Serializes as null when None
- `Vec<T>` - Serializes as JSON array with context wrapping
- `Box<T>` - Transparent serialization/deserialization
- Primitive types (`String`, `bool`, numerics, etc.)

All generic implementations recursively wrap nested values in contexts, ensuring format awareness propagates through the entire data structure.

## Why Not Use Serde's Data Formats Directly?

FHIR has format-specific requirements that don't map cleanly to serde's standard approach:

1. **JSON Primitive Extensions**: FHIR JSON uses `_fieldName` for primitive type extensions
2. **XML Attributes vs Elements**: FHIR XML has specific rules for when to use attributes vs elements
3. **Format-Specific Validation**: Different formats have different validation requirements

The context-driven approach allows us to handle these format-specific details cleanly while maintaining type safety.

## Adding New Formats

To add support for a new format (e.g., XML):

1. Define a format marker type:
   ```rust
   #[derive(Debug, Clone, Copy)]
   pub struct Xml;
   ```

2. Implement the bridging traits for the format in helios-fhir:
   ```rust
   impl FhirSerialize<Xml> for Patient {
       fn fhir_serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error> {
           // XML-specific serialization logic
       }
   }
   ```

3. Provide convenience functions in helios-serde:
   ```rust
   pub mod xml {
       pub fn to_string<T>(value: &T) -> Result<String, XmlError> {
           // ...
       }
   }
   ```

The blanket implementations automatically work for any format marker type, so no changes are needed in helios-serde!

## Performance

The context-driven approach has **zero runtime overhead**:

- Format markers are zero-sized types (ZSTs)
- `PhantomData` fields are optimized away by the compiler
- Monomorphization happens at compile time
- No virtual dispatch or dynamic trait objects

## License

MIT OR Apache-2.0
