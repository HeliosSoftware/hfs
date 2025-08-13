# fhirpath_support

The **helios-fhirpath-support** crate serves as a bridge module that provides essential types and traits for integration between the FHIRPath evaluator, it's associated functions, and also the FHIR model code in the `fhir` module, and it's generated code that is created by `fhir_macro`.

## Purpose and Scope

**helios-fhirpath-support** acts as a communication layer that allows:

- **FHIRPath evaluator** to work with unified result types
- **FHIR data structures** to convert into FHIRPath-compatible formats  
- **Code generation macros** to produce FHIRPath-aware implementations
- **Type conversion system** to handle data transformation of primitive types

The crate provides the shared vocabulary that all other components use when working with FHIRPath expressions and FHIR data.

## Architecture and Design

### Core Responsibility

As a bridge module, fhirpath_support:

1. **Defines Common Types**: Provides `EvaluationResult` and `EvaluationError` that serve as the universal data exchange format
2. **Enables Conversions**: Offers the `IntoEvaluationResult` trait for converting FHIR types to FHIRPath results
3. **Ensures Consistency**: Guarantees that all components use the same result representation
4. **Minimizes Dependencies**: Keeps a lean dependency footprint to avoid circular dependencies

### Key Types

#### `EvaluationResult`
The central data type representing any value that can result from FHIRPath expression evaluation:

```rust
pub enum EvaluationResult {
    Empty,                              // FHIRPath empty result
    Boolean(bool),                      // true/false values
    String(String),                     // Text values
    Decimal(Decimal),                   // High-precision numbers
    Integer(i64),                       // Whole numbers
    Date(String),                       // Date values (ISO format)
    DateTime(String),                   // DateTime values (ISO format)
    Time(String),                       // Time values (ISO format)
    Quantity(Decimal, String),          // Value with unit (e.g., "5.4 mg")
    Collection {                        // Arrays/lists of values
        items: Vec<EvaluationResult>,
        has_undefined_order: bool,
    },
    Object(HashMap<String, EvaluationResult>), // Key-value structures
}
```

#### `EvaluationError`
Comprehensive error handling for FHIRPath evaluation failures:

```rust
pub enum EvaluationError {
    TypeError(String),                  // Type mismatch errors
    InvalidArgument(String),            // Function argument errors
    UndefinedVariable(String),          // Variable resolution errors
    InvalidOperation(String),           // Operation errors
    InvalidArity(String),               // Function arity errors
    InvalidIndex(String),               // Array indexing errors
    DivisionByZero,                     // Math errors
    ArithmeticOverflow,                 // Overflow errors
    InvalidRegex(String),               // Regex compilation errors
    InvalidTypeSpecifier(String),       // Type specifier errors
    SingletonEvaluationError(String),   // Collection cardinality errors
    SemanticError(String),              // Semantic validation errors
    Other(String),                      // Generic errors
}
```

#### `IntoEvaluationResult` Trait
The universal conversion interface that enables any FHIR type to become a FHIRPath result:

```rust
pub trait IntoEvaluationResult {
    fn to_evaluation_result(&self) -> EvaluationResult;
}
```

## Usage Across the Codebase

### 1. **FHIRPath Evaluator Integration** (`crates/fhirpath`)

**Primary Consumer**: The fhirpath crate is the main consumer of fhirpath_support, importing it in virtually every module:

```rust
// Used in all FHIRPath function modules
use helios_fhirpath_support::{EvaluationError, EvaluationResult};
```

**Modules using fhirpath_support**:
- `evaluator.rs` - Core expression evaluation engine
- `aggregate_function.rs` - `aggregate()` function implementation
- `boolean_functions.rs` - Boolean logic operations
- `collection_functions.rs` - Collection manipulation functions
- `conversion_functions.rs` - Type conversion functions
- `date_arithmetic.rs` - Date/time arithmetic operations
- `extension_function.rs` - FHIR extension access
- `polymorphic_access.rs` - Choice element handling
- `trace_function.rs` - Debug tracing functionality
- `truncate_function.rs` - Decimal truncation
- `type_function.rs` - Type reflection operations
- And many more...

**Public Re-export**: The fhirpath crate re-exports EvaluationResult for external consumers:

```rust
pub use helios_fhirpath_support::EvaluationResult;
```

### 2. **FHIR Data Structure Integration** (`crates/fhir`)

**FHIR Type Conversion**: The main fhir crate imports fhirpath_support to enable FHIR data structures to work with FHIRPath:

```rust
use helios_fhirpath_support::{EvaluationResult, IntoEvaluationResult};
```

**Purpose**: Enables FHIR resources, data types, and elements to be seamlessly converted into FHIRPath-compatible formats for expression evaluation.

### 3. **Code Generation Integration** (`crates/fhir_macro`)

**Macro-Generated Implementations**: The fhir_macro crate uses fhirpath_support extensively to generate FHIRPath-aware code:

```rust
/// Derives the `fhirpath_support::IntoEvaluationResult` trait.
impl fhirpath_support::IntoEvaluationResult for GeneratedType {
    fn to_evaluation_result(&self) -> fhirpath_support::EvaluationResult {
        // Generated conversion logic
    }
}
```

**Generated Code Features**:
- Automatic `IntoEvaluationResult` implementations for all FHIR types
- Object serialization to `EvaluationResult::Object`
- Enum handling for choice types
- Field access and property resolution

## Implementation Features

### Type System Integration

**FHIRPath Type Mapping**: Maps FHIR primitive types to appropriate FHIRPath representations:
- FHIR `boolean` → `EvaluationResult::Boolean`
- FHIR `string`/`code`/`uri` → `EvaluationResult::String`
- FHIR `integer` → `EvaluationResult::Integer`
- FHIR `decimal` → `EvaluationResult::Decimal` (high-precision)
- FHIR `date`/`dateTime`/`time` → `EvaluationResult::Date/DateTime/Time`
- FHIR arrays → `EvaluationResult::Collection`
- FHIR complex types → `EvaluationResult::Object`

### Conversion Implementations

**Built-in Conversions**: Provides implementations for Rust standard types:

```rust
impl IntoEvaluationResult for String { /* ... */ }
impl IntoEvaluationResult for bool { /* ... */ }
impl IntoEvaluationResult for i32 { /* ... */ }
impl IntoEvaluationResult for i64 { /* ... */ }
impl IntoEvaluationResult for f64 { /* ... */ }
impl IntoEvaluationResult for Decimal { /* ... */ }
impl<T> IntoEvaluationResult for Option<T> { /* ... */ }
impl<T> IntoEvaluationResult for Vec<T> { /* ... */ }
impl<T> IntoEvaluationResult for Box<T> { /* ... */ }
```

### Advanced Features

**Equality and Ordering**: Implements proper comparison semantics for FHIRPath:
- Decimal normalization for precise comparison
- Collection order handling
- Object comparison with sorted keys
- Hash implementation for set operations

**Boolean Conversion**: Sophisticated boolean logic handling:
- Standard boolean values
- String-to-boolean conversion ("true"/"false", "t"/"f", "yes"/"no", "1"/"0")
- Empty result handling
- Collection singleton evaluation

**Utility Methods**: Rich API for working with evaluation results:
- `count()` - FHIRPath-compliant item counting
- `to_boolean()` - FHIRPath boolean conversion rules  
- `to_string_value()` - String representation
- `type_name()` - Runtime type identification
- `is_collection()` - Type checking utilities

## Dependencies

**Minimal and Focused**: The crate maintains a lean dependency profile:

- **`rust_decimal`** - High-precision decimal arithmetic required for FHIR's precise numeric requirements
- **Standard Library** - HashMap, collections, and basic types

**No FHIR Dependencies**: Intentionally avoids depending on FHIR types to prevent circular dependencies and maintain clean separation of concerns.

## Development Patterns

### Bridge Pattern Implementation

The crate exemplifies the bridge pattern by:

1. **Abstracting Representations**: Provides a unified result type that abstracts over different FHIR data formats
2. **Enabling Conversion**: Offers a standard interface for converting between representations
3. **Facilitating Communication**: Ensures all components speak the same "language" when exchanging data
4. **Maintaining Independence**: Allows different parts of the system to evolve independently

### Extension Points

**New Type Support**: Adding support for new FHIRPath types requires:
1. Adding variant to `EvaluationResult`
2. Implementing conversion logic
3. Updating comparison and utility methods
4. Adding appropriate error handling

**New Conversion Sources**: Supporting new FHIR types requires:
1. Implementing `IntoEvaluationResult` for the type
2. Handling the conversion logic appropriately
3. Testing the conversion behavior

## Testing and Quality

The crate's types implement comprehensive trait support:
- **Debug** - For debugging and error reporting
- **Clone** - For value copying and manipulation
- **PartialEq/Eq** - For value comparison and testing
- **PartialOrd/Ord** - For sorting and ordering operations
- **Hash** - For set operations and deduplication

All implementations follow FHIRPath specification requirements for type behavior and conversion rules.

## Status and Future

**fhirpath_support** is a mature, stable component that successfully bridges the FHIRPath evaluator with the FHIR type system. As the foundation for data exchange in the FHIRPath ecosystem, it maintains:

- **Backward Compatibility** - API stability for dependent crates
- **Performance** - Efficient conversion and comparison operations
- **Correctness** - FHIRPath specification compliance
- **Extensibility** - Clean extension points for new features

Future enhancements focus on:
- Additional FHIRPath type support as the specification evolves
- Performance optimizations for large data sets
- Enhanced error reporting and diagnostics
- Improved conversion utilities for complex scenarios

The crate serves as the essential foundation that enables the entire FHIRPath implementation to function as a cohesive, interoperable system.
