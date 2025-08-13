//! # FHIRPath Support Types
//!
//! This crate provides the foundational types and traits that serve as a bridge between
//! the FHIRPath evaluator and the broader FHIR ecosystem. It defines the common data
//! structures and conversion interfaces that enable seamless integration across all
//! components of the FHIRPath implementation.
//!
//! ## Overview
//!
//! The fhirpath_support crate acts as the universal communication layer that allows:
//! - FHIRPath evaluator to work with unified result types
//! - FHIR data structures to convert into FHIRPath-compatible formats
//! - Code generation macros to produce FHIRPath-aware implementations
//! - Type conversion system to handle data transformation
//!
//! ## Core Types
//!
//! - [`EvaluationResult`] - Universal result type for FHIRPath expression evaluation
//! - [`EvaluationError`] - Comprehensive error handling for evaluation failures
//! - [`IntoEvaluationResult`] - Trait for converting types to evaluation results
//!
//! ## Usage Example
//!
//! ```rust
//! use helios_fhirpath_support::{EvaluationResult, IntoEvaluationResult};
//!
//! // Convert a string to an evaluation result
//! let text = "Hello, FHIR!".to_string();
//! let result = text.to_evaluation_result();
//! assert_eq!(result, EvaluationResult::String("Hello, FHIR!".to_string(), None));
//!
//! // Work with collections
//! let numbers = vec![1, 2, 3];
//! let collection = numbers.to_evaluation_result();
//! assert_eq!(collection.count(), 3);
//! ```

use rust_decimal::Decimal;
use rust_decimal::prelude::*;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

mod type_info;
pub use type_info::TypeInfoResult;

/// Universal conversion trait for transforming values into FHIRPath evaluation results.
///
/// This trait provides the bridge between FHIR data types and the FHIRPath evaluation
/// system. It allows any type to be converted into an `EvaluationResult` that can be
/// processed by FHIRPath expressions.
///
/// # Implementation Guidelines
///
/// When implementing this trait:
/// - Return `EvaluationResult::Empty` for `None` or missing values
/// - Use appropriate variant types (Boolean, String, Integer, etc.)
/// - For complex types, use `EvaluationResult::Object` with field mappings
/// - For arrays/collections, use `EvaluationResult::Collection`
///
/// # Examples
///
/// ```rust
/// use helios_fhirpath_support::{EvaluationResult, IntoEvaluationResult};
///
/// struct CustomType {
///     value: String,
///     active: bool,
/// }
///
/// impl IntoEvaluationResult for CustomType {
///     fn to_evaluation_result(&self) -> EvaluationResult {
///         let mut map = std::collections::HashMap::new();
///         map.insert("value".to_string(), self.value.to_evaluation_result());
///         map.insert("active".to_string(), self.active.to_evaluation_result());
///         EvaluationResult::Object { map, type_info: None }
///     }
/// }
/// ```
pub trait IntoEvaluationResult {
    /// Converts this value into a FHIRPath evaluation result.
    ///
    /// This method should transform the implementing type into the most appropriate
    /// `EvaluationResult` variant that represents the value's semantics in FHIRPath.
    fn to_evaluation_result(&self) -> EvaluationResult;
}

/// Universal result type for FHIRPath expression evaluation.
///
/// This enum represents any value that can result from evaluating a FHIRPath expression
/// against FHIR data. It provides a unified type system that bridges FHIR's data model
/// with FHIRPath's evaluation semantics.
///
/// # Variants
///
/// - **`Empty`**: Represents no value or null (equivalent to FHIRPath's empty collection)
/// - **`Boolean`**: True/false values from boolean expressions
/// - **`String`**: Text values from FHIR strings, codes, URIs, etc.
/// - **`Decimal`**: High-precision decimal numbers for accurate numeric computation
/// - **`Integer`**: Whole numbers for counting and indexing operations
/// - **`Integer64`**: Explicit 64-bit integers for special cases
/// - **`Date`**: Date values in ISO format (YYYY-MM-DD)
/// - **`DateTime`**: DateTime values in ISO format with optional timezone
/// - **`Time`**: Time values in ISO format (HH:MM:SS)
/// - **`Quantity`**: Value with unit (e.g., "5.4 mg", "10 years")
/// - **`Collection`**: Ordered collections of evaluation results
/// - **`Object`**: Key-value structures representing complex FHIR types
///
/// # Type Safety
///
/// The enum is designed to prevent type errors at runtime by encoding FHIRPath's
/// type system at the Rust type level. Operations that require specific types
/// can pattern match on the appropriate variants.
///
/// # Examples
///
/// ```rust
/// use helios_fhirpath_support::EvaluationResult;
/// use rust_decimal::Decimal;
///
/// // Creating different result types
/// let empty = EvaluationResult::Empty;
/// let text = EvaluationResult::String("Patient".to_string(), None);
/// let number = EvaluationResult::Integer(42, None);
/// let number64 = EvaluationResult::Integer64(9223372036854775807, None); // max i64
/// let decimal = EvaluationResult::Decimal(Decimal::new(1234, 2), None); // 12.34
///
/// // Working with collections
/// let items = vec![text, number];
/// let collection = EvaluationResult::Collection {
///     items,
///     has_undefined_order: false,
///     type_info: None
/// };
///
/// assert_eq!(collection.count(), 2);
/// assert!(collection.is_collection());
/// ```
#[derive(Debug, Clone)]
pub enum EvaluationResult {
    /// No value or empty collection.
    ///
    /// Represents the absence of a value, equivalent to FHIRPath's empty collection `{}`.
    /// This is the result when accessing non-existent properties or when filters
    /// match no elements.
    Empty,
    /// Boolean true/false value.
    ///
    /// Results from boolean expressions, existence checks, and logical operations.
    /// Also used for FHIR boolean fields.
    Boolean(bool, Option<TypeInfoResult>),
    /// Text string value.
    ///
    /// Used for FHIR string, code, uri, canonical, id, and other text-based types.
    /// Also results from string manipulation functions and conversions.
    String(String, Option<TypeInfoResult>),
    /// High-precision decimal number.
    ///
    /// Uses `rust_decimal::Decimal` for precise arithmetic without floating-point
    /// errors. Required for FHIR's decimal type and mathematical operations.
    Decimal(Decimal, Option<TypeInfoResult>),
    /// Whole number value.
    ///
    /// Used for FHIR integer, positiveInt, unsignedInt types and counting operations.
    /// Also results from indexing and length functions.
    Integer(i64, Option<TypeInfoResult>),
    /// 64-bit integer value.
    ///
    /// Explicit 64-bit integer type for cases where the distinction from regular
    /// integers is important.
    Integer64(i64, Option<TypeInfoResult>),
    /// Date value in ISO format.
    ///
    /// Stores date as string in YYYY-MM-DD format. Handles FHIR date fields
    /// and results from date extraction functions.
    Date(String, Option<TypeInfoResult>),
    /// DateTime value in ISO format.
    ///
    /// Stores datetime as string in ISO 8601 format with optional timezone.
    /// Handles FHIR dateTime and instant fields.
    DateTime(String, Option<TypeInfoResult>),
    /// Time value in ISO format.
    ///
    /// Stores time as string in HH:MM:SS format. Handles FHIR time fields
    /// and results from time extraction functions.
    Time(String, Option<TypeInfoResult>),
    /// Quantity with value and unit.
    ///
    /// Represents measurements with units (e.g., "5.4 mg", "10 years").
    /// First element is the numeric value, second is the unit string.
    /// Used for FHIR Quantity, Age, Duration, Distance, Count, and Money types.
    Quantity(Decimal, String, Option<TypeInfoResult>),
    /// Ordered collection of evaluation results.
    ///
    /// Represents arrays, lists, and multi-valued FHIR elements. Collections
    /// maintain order for FHIRPath operations like indexing and iteration.
    ///
    /// # Fields
    ///
    /// - `items`: The ordered list of contained evaluation results
    /// - `has_undefined_order`: Flag indicating if the original source order
    ///   was undefined (affects certain FHIRPath operations)
    Collection {
        /// The ordered items in this collection
        items: Vec<EvaluationResult>,
        /// Whether the original source order was undefined
        has_undefined_order: bool,
        /// Optional type information
        type_info: Option<TypeInfoResult>,
    },
    /// Key-value object representing complex FHIR types.
    ///
    /// Used for FHIR resources, data types, and backbone elements. Keys are
    /// field names and values are the corresponding evaluation results.
    /// Enables property access via FHIRPath dot notation.
    ///
    /// The optional type_namespace and type_name fields preserve type information
    /// for the FHIRPath type() function.
    Object {
        /// The object's properties
        map: HashMap<String, EvaluationResult>,
        /// Optional type information
        type_info: Option<TypeInfoResult>,
    },
}

/// Comprehensive error type for FHIRPath evaluation failures.
///
/// This enum covers all categories of errors that can occur during FHIRPath expression
/// evaluation, from type mismatches to semantic violations. Each variant provides
/// specific context about the failure to aid in debugging and error reporting.
///
/// # Error Categories
///
/// - **Type Errors**: Mismatched types in operations or function calls
/// - **Argument Errors**: Invalid arguments passed to functions
/// - **Runtime Errors**: Errors during expression evaluation (division by zero, etc.)
/// - **Semantic Errors**: Violations of FHIRPath semantic rules
/// - **System Errors**: Internal errors and edge cases
///
/// # Error Handling
///
/// All variants implement `std::error::Error` and `Display` for standard Rust
/// error handling patterns. The error messages are designed to be user-friendly
/// and provide actionable information for debugging.
///
/// # Examples
///
/// ```rust
/// use helios_fhirpath_support::EvaluationError;
///
/// // Type error example
/// let error = EvaluationError::TypeError(
///     "Cannot add String and Integer".to_string()
/// );
///
/// // Display the error
/// println!("{}", error); // "Type Error: Cannot add String and Integer"
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvaluationError {
    /// Type mismatch or incompatible type operation.
    ///
    /// Occurs when operations are attempted on incompatible types or when
    /// functions receive arguments of unexpected types.
    ///
    /// Example: "Expected Boolean, found Integer"
    TypeError(String),
    /// Invalid argument provided to a function.
    ///
    /// Occurs when function arguments don't meet the required constraints
    /// or format expectations.
    ///
    /// Example: "Invalid argument for function 'where'"
    InvalidArgument(String),
    /// Reference to an undefined variable.
    ///
    /// Occurs when expressions reference variables that haven't been defined
    /// in the current evaluation context.
    ///
    /// Example: "Variable '%undefinedVar' not found"
    UndefinedVariable(String),
    /// Invalid operation for the given operand types.
    ///
    /// Occurs when operators are used with incompatible operand types or
    /// when operations are not supported for the given types.
    ///
    /// Example: "Cannot add String and Integer"
    InvalidOperation(String),
    /// Incorrect number of arguments provided to a function.
    ///
    /// Occurs when functions are called with too many or too few arguments
    /// compared to their specification.
    ///
    /// Example: "Function 'substring' expects 1 or 2 arguments, got 3"
    InvalidArity(String),
    /// Invalid array or collection index.
    ///
    /// Occurs when collection indexing operations use invalid indices
    /// (negative numbers, non-integers, out of bounds).
    ///
    /// Example: "Index must be a non-negative integer"
    InvalidIndex(String),
    /// Attempted division by zero.
    ///
    /// Occurs during mathematical operations when the divisor is zero.
    /// This is a specific case of arithmetic error with clear semantics.
    DivisionByZero,
    /// Arithmetic operation resulted in overflow.
    ///
    /// Occurs when mathematical operations produce results that exceed
    /// the representable range of the target numeric type.
    ArithmeticOverflow,
    /// Invalid regular expression pattern.
    ///
    /// Occurs when regex-based functions receive malformed regex patterns
    /// that cannot be compiled.
    ///
    /// Example: "Invalid regex pattern: unclosed parenthesis"
    InvalidRegex(String),
    /// Invalid type specifier in type operations.
    ///
    /// Occurs when type checking operations (is, as, ofType) receive
    /// invalid or unrecognized type specifiers.
    ///
    /// Example: "Unknown type 'InvalidType'"
    InvalidTypeSpecifier(String),
    /// Collection cardinality error for singleton operations.
    ///
    /// Occurs when operations expecting a single value receive collections
    /// with zero or multiple items.
    ///
    /// Example: "Expected singleton, found collection with 3 items"
    SingletonEvaluationError(String),
    /// Semantic rule violation.
    ///
    /// Occurs when expressions violate FHIRPath semantic rules, such as
    /// accessing non-existent properties in strict mode or violating
    /// contextual constraints.
    ///
    /// Example: "Property 'invalidField' does not exist on type 'Patient'"
    SemanticError(String),
    /// Generic error for cases not covered by specific variants.
    ///
    /// Used for internal errors, edge cases, or temporary error conditions
    /// that don't fit into the specific error categories.
    ///
    /// Example: "Internal evaluation error"
    Other(String),
}

// === Standard Error Trait Implementations ===

/// Implements the standard `Error` trait for `EvaluationError`.
///
/// This allows `EvaluationError` to be used with Rust's standard error handling
/// mechanisms, including `?` operator, `Result` combinators, and error chains.
impl std::error::Error for EvaluationError {}

/// Implements the `Display` trait for user-friendly error messages.
///
/// Provides formatted, human-readable error messages that include error category
/// prefixes for easy identification of error types.
impl std::fmt::Display for EvaluationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvaluationError::TypeError(msg) => write!(f, "Type Error: {}", msg),
            EvaluationError::InvalidArgument(msg) => write!(f, "Invalid Argument: {}", msg),
            EvaluationError::UndefinedVariable(name) => write!(f, "Undefined Variable: {}", name),
            EvaluationError::InvalidOperation(msg) => write!(f, "Invalid Operation: {}", msg),
            EvaluationError::InvalidArity(msg) => write!(f, "Invalid Arity: {}", msg),
            EvaluationError::InvalidIndex(msg) => write!(f, "Invalid Index: {}", msg),
            EvaluationError::DivisionByZero => write!(f, "Division by zero"),
            EvaluationError::ArithmeticOverflow => write!(f, "Arithmetic overflow"),
            EvaluationError::InvalidRegex(msg) => write!(f, "Invalid Regex: {}", msg),
            EvaluationError::InvalidTypeSpecifier(msg) => {
                write!(f, "Invalid Type Specifier: {}", msg)
            }
            EvaluationError::SingletonEvaluationError(msg) => {
                write!(f, "Singleton Evaluation Error: {}", msg)
            }
            EvaluationError::SemanticError(msg) => write!(f, "Semantic Error: {}", msg),
            EvaluationError::Other(msg) => write!(f, "Evaluation Error: {}", msg),
        }
    }
}

// === EvaluationResult Trait Implementations ===

/// Implements equality comparison for `EvaluationResult`.
///
/// This implementation follows FHIRPath equality semantics:
/// - Decimal values are normalized before comparison for precision consistency
/// - Collections compare both items and order flags
/// - Objects use HashMap equality (order-independent)
/// - Cross-variant comparisons always return `false`
///
/// # Examples
///
/// ```rust
/// use helios_fhirpath_support::EvaluationResult;
/// use rust_decimal::Decimal;
///
/// let a = EvaluationResult::String("test".to_string(), None);
/// let b = EvaluationResult::String("test".to_string(), None);
/// assert_eq!(a, b);
///
/// let c = EvaluationResult::Decimal(Decimal::new(100, 2), None); // 1.00
/// let d = EvaluationResult::Decimal(Decimal::new(1, 0), None);   // 1
/// assert_eq!(c, d); // Normalized decimals are equal
/// ```
impl PartialEq for EvaluationResult {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (EvaluationResult::Empty, EvaluationResult::Empty) => true,
            (EvaluationResult::Boolean(a, _), EvaluationResult::Boolean(b, _)) => a == b,
            (EvaluationResult::String(a, _), EvaluationResult::String(b, _)) => a == b,
            (EvaluationResult::Decimal(a, _), EvaluationResult::Decimal(b, _)) => {
                // Normalize decimals to handle precision differences (e.g., 1.0 == 1.00)
                a.normalize() == b.normalize()
            }
            (EvaluationResult::Integer(a, _), EvaluationResult::Integer(b, _)) => a == b,
            (EvaluationResult::Integer64(a, _), EvaluationResult::Integer64(b, _)) => a == b,
            (EvaluationResult::Date(a, _), EvaluationResult::Date(b, _)) => a == b,
            (EvaluationResult::DateTime(a, _), EvaluationResult::DateTime(b, _)) => a == b,
            (EvaluationResult::Time(a, _), EvaluationResult::Time(b, _)) => a == b,
            (
                EvaluationResult::Quantity(val_a, unit_a, _),
                EvaluationResult::Quantity(val_b, unit_b, _),
            ) => {
                // Quantities are equal if both value and unit match (normalized values)
                val_a.normalize() == val_b.normalize() && unit_a == unit_b
            }
            (
                EvaluationResult::Collection {
                    items: a_items,
                    has_undefined_order: a_undef,
                    ..
                },
                EvaluationResult::Collection {
                    items: b_items,
                    has_undefined_order: b_undef,
                    ..
                },
            ) => {
                // Collections are equal if both order flags and items match
                a_undef == b_undef && a_items == b_items
            }
            (EvaluationResult::Object { map: a, .. }, EvaluationResult::Object { map: b, .. }) => {
                a == b
            }
            _ => false,
        }
    }
}
/// Marker trait implementation indicating that `EvaluationResult` has total equality.
///
/// Since we implement `PartialEq` with total equality semantics (no NaN-like values),
/// we can safely implement `Eq`.
impl Eq for EvaluationResult {}

/// Implements partial ordering for `EvaluationResult`.
///
/// This provides a consistent ordering for sorting operations, but note that this
/// ordering is primarily for internal use (e.g., in collections) and may not
/// reflect FHIRPath's comparison semantics, which are handled separately.
impl PartialOrd for EvaluationResult {
    /// Compares two evaluation results for partial ordering.
    ///
    /// Since we implement total ordering, this always returns `Some`.
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Implements total ordering for `EvaluationResult`.
///
/// This provides a deterministic ordering for all evaluation results, enabling
/// their use in sorted collections. The ordering is defined by:
/// 1. Variant precedence (Empty < Boolean < Integer < ... < Object)
/// 2. Value comparison within the same variant
///
/// Note: This is an arbitrary but consistent ordering for internal use.
/// FHIRPath comparison operators use different semantics.
impl Ord for EvaluationResult {
    /// Compares two evaluation results for total ordering.
    ///
    /// Returns the ordering relationship between `self` and `other`.
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            // Order variants by type precedence
            (EvaluationResult::Empty, EvaluationResult::Empty) => Ordering::Equal,
            (EvaluationResult::Empty, _) => Ordering::Less,
            (_, EvaluationResult::Empty) => Ordering::Greater,

            (EvaluationResult::Boolean(a, _), EvaluationResult::Boolean(b, _)) => a.cmp(b),
            (EvaluationResult::Boolean(_, _), _) => Ordering::Less,
            (_, EvaluationResult::Boolean(_, _)) => Ordering::Greater,

            (EvaluationResult::Integer(a, _), EvaluationResult::Integer(b, _)) => a.cmp(b),
            (EvaluationResult::Integer(_, _), _) => Ordering::Less,
            (_, EvaluationResult::Integer(_, _)) => Ordering::Greater,

            (EvaluationResult::Integer64(a, _), EvaluationResult::Integer64(b, _)) => a.cmp(b),
            (EvaluationResult::Integer64(_, _), _) => Ordering::Less,
            (_, EvaluationResult::Integer64(_, _)) => Ordering::Greater,

            (EvaluationResult::Decimal(a, _), EvaluationResult::Decimal(b, _)) => a.cmp(b),
            (EvaluationResult::Decimal(_, _), _) => Ordering::Less,
            (_, EvaluationResult::Decimal(_, _)) => Ordering::Greater,

            (EvaluationResult::String(a, _), EvaluationResult::String(b, _)) => a.cmp(b),
            (EvaluationResult::String(_, _), _) => Ordering::Less,
            (_, EvaluationResult::String(_, _)) => Ordering::Greater,

            (EvaluationResult::Date(a, _), EvaluationResult::Date(b, _)) => a.cmp(b),
            (EvaluationResult::Date(_, _), _) => Ordering::Less,
            (_, EvaluationResult::Date(_, _)) => Ordering::Greater,

            (EvaluationResult::DateTime(a, _), EvaluationResult::DateTime(b, _)) => a.cmp(b),
            (EvaluationResult::DateTime(_, _), _) => Ordering::Less,
            (_, EvaluationResult::DateTime(_, _)) => Ordering::Greater,

            (EvaluationResult::Time(a, _), EvaluationResult::Time(b, _)) => a.cmp(b),
            (EvaluationResult::Time(_, _), _) => Ordering::Less,
            (_, EvaluationResult::Time(_, _)) => Ordering::Greater,

            (
                EvaluationResult::Quantity(val_a, unit_a, _),
                EvaluationResult::Quantity(val_b, unit_b, _),
            ) => {
                // Order by value first, then by unit string
                match val_a.cmp(val_b) {
                    Ordering::Equal => unit_a.cmp(unit_b),
                    other => other,
                }
            }
            (EvaluationResult::Quantity(_, _, _), _) => Ordering::Less,
            (_, EvaluationResult::Quantity(_, _, _)) => Ordering::Greater,

            (
                EvaluationResult::Collection {
                    items: a_items,
                    has_undefined_order: a_undef,
                    ..
                },
                EvaluationResult::Collection {
                    items: b_items,
                    has_undefined_order: b_undef,
                    ..
                },
            ) => {
                // Order by undefined_order flag first (false < true), then by items
                match a_undef.cmp(b_undef) {
                    Ordering::Equal => {
                        // Compare items as ordered lists (FHIRPath collections maintain order)
                        a_items.cmp(b_items)
                    }
                    other => other,
                }
            }
            (EvaluationResult::Collection { .. }, _) => Ordering::Less,
            (_, EvaluationResult::Collection { .. }) => Ordering::Greater,

            (EvaluationResult::Object { map: a, .. }, EvaluationResult::Object { map: b, .. }) => {
                // Compare objects by sorted keys, then by values
                let mut a_keys: Vec<_> = a.keys().collect();
                let mut b_keys: Vec<_> = b.keys().collect();
                a_keys.sort();
                b_keys.sort();

                match a_keys.cmp(&b_keys) {
                    Ordering::Equal => {
                        // Same keys: compare values in sorted key order
                        for key in a_keys {
                            match a[key].cmp(&b[key]) {
                                Ordering::Equal => continue,
                                non_equal => return non_equal,
                            }
                        }
                        Ordering::Equal
                    }
                    non_equal => non_equal,
                }
            } // Note: Object is the last variant, so no additional arms needed
        }
    }
}
/// Implements hashing for `EvaluationResult`.
///
/// This implementation enables use of `EvaluationResult` in hash-based collections
/// like `HashSet` and `HashMap`. The hash implementation is consistent with equality:
/// values that are equal will have the same hash.
///
/// # Hash Stability
///
/// - Decimal values are normalized before hashing for consistency
/// - Collections hash both the items and the order flag
/// - Objects hash keys in sorted order for deterministic results
/// - All variants include a discriminant hash to avoid collisions
///
/// # Use Cases
///
/// This implementation enables FHIRPath operations like:
/// - `distinct()` function using `HashSet` for deduplication
/// - `intersect()` and `union()` set operations
/// - Efficient lookups in evaluation contexts
impl Hash for EvaluationResult {
    /// Computes the hash of this evaluation result.
    ///
    /// The hash implementation ensures that equal values produce equal hashes
    /// and provides good distribution for hash-based collections.
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Hash the enum variant first to avoid cross-variant collisions
        core::mem::discriminant(self).hash(state);
        match self {
            // Empty has no additional data to hash
            EvaluationResult::Empty => {}
            EvaluationResult::Boolean(b, _) => b.hash(state),
            EvaluationResult::String(s, _) => s.hash(state),
            // Hash normalized decimal for consistency with equality
            EvaluationResult::Decimal(d, _) => d.normalize().hash(state),
            EvaluationResult::Integer(i, _) => i.hash(state),
            EvaluationResult::Integer64(i, _) => i.hash(state),
            EvaluationResult::Date(d, _) => d.hash(state),
            EvaluationResult::DateTime(dt, _) => dt.hash(state),
            EvaluationResult::Time(t, _) => t.hash(state),
            EvaluationResult::Quantity(val, unit, _) => {
                // Hash both normalized value and unit
                val.normalize().hash(state);
                unit.hash(state);
            }
            EvaluationResult::Collection {
                items,
                has_undefined_order,
                ..
            } => {
                // Hash order flag and items
                has_undefined_order.hash(state);
                items.len().hash(state);
                for item in items {
                    item.hash(state);
                }
            }
            EvaluationResult::Object { map, .. } => {
                // Hash objects with sorted keys for deterministic results
                // Note: We don't hash type_namespace/type_name to maintain compatibility
                let mut keys: Vec<_> = map.keys().collect();
                keys.sort();
                keys.len().hash(state);
                for key in keys {
                    key.hash(state);
                    map[key].hash(state);
                }
            }
        }
    }
}

// === EvaluationResult Methods ===

impl EvaluationResult {
    // === Constructor Methods ===

    /// Creates a Boolean result with System type.
    pub fn boolean(value: bool) -> Self {
        EvaluationResult::Boolean(value, Some(TypeInfoResult::new("System", "Boolean")))
    }

    /// Creates a Boolean result with FHIR type.
    pub fn fhir_boolean(value: bool) -> Self {
        EvaluationResult::Boolean(value, Some(TypeInfoResult::new("FHIR", "boolean")))
    }

    /// Creates a String result with System type.
    pub fn string(value: String) -> Self {
        EvaluationResult::String(value, Some(TypeInfoResult::new("System", "String")))
    }

    /// Creates a String result with FHIR type.
    pub fn fhir_string(value: String, fhir_type: &str) -> Self {
        EvaluationResult::String(value, Some(TypeInfoResult::new("FHIR", fhir_type)))
    }

    /// Creates an Integer result with System type.
    pub fn integer(value: i64) -> Self {
        EvaluationResult::Integer(value, Some(TypeInfoResult::new("System", "Integer")))
    }

    /// Creates an Integer result with FHIR type.
    pub fn fhir_integer(value: i64) -> Self {
        EvaluationResult::Integer(value, Some(TypeInfoResult::new("FHIR", "integer")))
    }

    /// Creates an Integer64 result with System type.
    pub fn integer64(value: i64) -> Self {
        EvaluationResult::Integer64(value, Some(TypeInfoResult::new("System", "Integer64")))
    }

    /// Creates an Integer64 result with FHIR type.
    pub fn fhir_integer64(value: i64) -> Self {
        EvaluationResult::Integer64(value, Some(TypeInfoResult::new("FHIR", "integer64")))
    }

    /// Creates a Decimal result with System type.
    pub fn decimal(value: Decimal) -> Self {
        EvaluationResult::Decimal(value, Some(TypeInfoResult::new("System", "Decimal")))
    }

    /// Creates a Decimal result with FHIR type.
    pub fn fhir_decimal(value: Decimal) -> Self {
        EvaluationResult::Decimal(value, Some(TypeInfoResult::new("FHIR", "decimal")))
    }

    /// Creates a Date result with System type.
    pub fn date(value: String) -> Self {
        EvaluationResult::Date(value, Some(TypeInfoResult::new("System", "Date")))
    }

    /// Creates a DateTime result with System type.
    pub fn datetime(value: String) -> Self {
        EvaluationResult::DateTime(value, Some(TypeInfoResult::new("System", "DateTime")))
    }

    /// Creates a Time result with System type.
    pub fn time(value: String) -> Self {
        EvaluationResult::Time(value, Some(TypeInfoResult::new("System", "Time")))
    }

    /// Creates a Quantity result with System type.
    pub fn quantity(value: Decimal, unit: String) -> Self {
        EvaluationResult::Quantity(value, unit, Some(TypeInfoResult::new("System", "Quantity")))
    }

    /// Creates a Collection result.
    pub fn collection(items: Vec<EvaluationResult>) -> Self {
        EvaluationResult::Collection {
            items,
            has_undefined_order: false,
            type_info: None,
        }
    }

    /// Creates an Object variant with just the map, no type information.
    pub fn object(map: HashMap<String, EvaluationResult>) -> Self {
        EvaluationResult::Object {
            map,
            type_info: None,
        }
    }

    /// Creates an Object variant with type information.
    pub fn typed_object(
        map: HashMap<String, EvaluationResult>,
        type_namespace: &str,
        type_name: &str,
    ) -> Self {
        EvaluationResult::Object {
            map,
            type_info: Some(TypeInfoResult::new(type_namespace, type_name)),
        }
    }

    // === Value Extraction Methods ===

    /// Extracts the boolean value if this is a Boolean variant.
    pub fn as_boolean(&self) -> Option<bool> {
        match self {
            EvaluationResult::Boolean(val, _) => Some(*val),
            _ => None,
        }
    }

    /// Extracts the string value if this is a String variant.
    pub fn as_string(&self) -> Option<&String> {
        match self {
            EvaluationResult::String(val, _) => Some(val),
            _ => None,
        }
    }

    /// Extracts the integer value if this is an Integer variant.
    pub fn as_integer(&self) -> Option<i64> {
        match self {
            EvaluationResult::Integer(val, _) => Some(*val),
            _ => None,
        }
    }

    /// Extracts the integer value if this is an Integer64 variant.
    pub fn as_integer64(&self) -> Option<i64> {
        match self {
            EvaluationResult::Integer64(val, _) => Some(*val),
            _ => None,
        }
    }

    /// Extracts the decimal value if this is a Decimal variant.
    pub fn as_decimal(&self) -> Option<Decimal> {
        match self {
            EvaluationResult::Decimal(val, _) => Some(*val),
            _ => None,
        }
    }

    /// Extracts the date value if this is a Date variant.
    pub fn as_date(&self) -> Option<&String> {
        match self {
            EvaluationResult::Date(val, _) => Some(val),
            _ => None,
        }
    }

    /// Extracts the datetime value if this is a DateTime variant.
    pub fn as_datetime(&self) -> Option<&String> {
        match self {
            EvaluationResult::DateTime(val, _) => Some(val),
            _ => None,
        }
    }

    /// Extracts the time value if this is a Time variant.
    pub fn as_time(&self) -> Option<&String> {
        match self {
            EvaluationResult::Time(val, _) => Some(val),
            _ => None,
        }
    }

    /// Extracts the quantity value if this is a Quantity variant.
    pub fn as_quantity(&self) -> Option<(Decimal, &String)> {
        match self {
            EvaluationResult::Quantity(val, unit, _) => Some((*val, unit)),
            _ => None,
        }
    }
    /// Checks if this result represents a collection.
    ///
    /// Returns `true` only for the `Collection` variant, not for other
    /// multi-valued representations like `Object`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use helios_fhirpath_support::EvaluationResult;
    ///
    /// let collection = EvaluationResult::Collection {
    ///     items: vec![],
    ///     has_undefined_order: false,
    ///     type_info: None,
    /// };
    /// assert!(collection.is_collection());
    ///
    /// let string = EvaluationResult::String("test".to_string(), None);
    /// assert!(!string.is_collection());
    /// ```
    pub fn is_collection(&self) -> bool {
        matches!(self, EvaluationResult::Collection { .. })
    }

    /// Returns the count of items according to FHIRPath counting rules.
    ///
    /// FHIRPath counting semantics:
    /// - `Empty`: 0 items
    /// - `Collection`: number of items in the collection
    /// - All other variants: 1 item (single values)
    ///
    /// This matches the behavior of FHIRPath's `count()` function.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use helios_fhirpath_support::EvaluationResult;
    ///
    /// assert_eq!(EvaluationResult::Empty.count(), 0);
    /// assert_eq!(EvaluationResult::String("test".to_string(), None).count(), 1);
    ///
    /// let collection = EvaluationResult::Collection {
    ///     items: vec![
    ///         EvaluationResult::Integer(1, None),
    ///         EvaluationResult::Integer(2, None),
    ///     ],
    ///     has_undefined_order: false,
    ///     type_info: None,
    /// };
    /// assert_eq!(collection.count(), 2);
    /// ```
    pub fn count(&self) -> usize {
        match self {
            EvaluationResult::Empty => 0,
            EvaluationResult::Collection { items, .. } => items.len(),
            _ => 1, // All non-collection variants count as 1
        }
    }
    /// Converts the result to a boolean value according to FHIRPath truthiness rules.
    ///
    /// FHIRPath truthiness semantics:
    /// - `Empty`: `false`
    /// - `Boolean`: the boolean value itself
    /// - `String`: `false` if empty, `true` otherwise
    /// - `Decimal`/`Integer`: `false` if zero, `true` otherwise
    /// - `Quantity`: `false` if value is zero, `true` otherwise
    /// - `Collection`: `false` if empty, `true` otherwise
    /// - Other types: `true` (Date, DateTime, Time, Object)
    ///
    /// Note: This is different from boolean conversion for logical operators.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use helios_fhirpath_support::EvaluationResult;
    /// use rust_decimal::Decimal;
    ///
    /// assert_eq!(EvaluationResult::Empty.to_boolean(), false);
    /// assert_eq!(EvaluationResult::Boolean(true, None).to_boolean(), true);
    /// assert_eq!(EvaluationResult::String("".to_string(), None).to_boolean(), false);
    /// assert_eq!(EvaluationResult::String("text".to_string(), None).to_boolean(), true);
    /// assert_eq!(EvaluationResult::Integer(0, None).to_boolean(), false);
    /// assert_eq!(EvaluationResult::Integer(42, None).to_boolean(), true);
    /// ```
    pub fn to_boolean(&self) -> bool {
        match self {
            EvaluationResult::Empty => false,
            EvaluationResult::Boolean(b, _) => *b,
            EvaluationResult::String(s, _) => !s.is_empty(),
            EvaluationResult::Decimal(d, _) => !d.is_zero(),
            EvaluationResult::Integer(i, _) => *i != 0,
            EvaluationResult::Integer64(i, _) => *i != 0,
            EvaluationResult::Quantity(q, _, _) => !q.is_zero(), // Truthy if value is non-zero
            EvaluationResult::Collection { items, .. } => !items.is_empty(),
            _ => true, // Date, DateTime, Time, Object are always truthy
        }
    }

    /// Converts the result to its string representation.
    ///
    /// This method provides the string representation used by FHIRPath's
    /// `toString()` function and string conversion operations.
    ///
    /// # Conversion Rules
    ///
    /// - `Empty`: empty string
    /// - `Boolean`: "true" or "false"
    /// - `String`: the string value itself
    /// - Numeric types: string representation of the number
    /// - Date/Time types: the ISO format string
    /// - `Quantity`: formatted as "value 'unit'"
    /// - `Collection`: if single item, its string value; otherwise bracketed list
    /// - `Object`: "\[object\]" placeholder
    ///
    /// # Examples
    ///
    /// ```rust
    /// use helios_fhirpath_support::EvaluationResult;
    /// use rust_decimal::Decimal;
    ///
    /// assert_eq!(EvaluationResult::Empty.to_string_value(), "");
    /// assert_eq!(EvaluationResult::Boolean(true, None).to_string_value(), "true");
    /// assert_eq!(EvaluationResult::Integer(42, None).to_string_value(), "42");
    ///
    /// let quantity = EvaluationResult::Quantity(Decimal::new(54, 1), "mg".to_string(), None);
    /// assert_eq!(quantity.to_string_value(), "5.4 'mg'");
    /// ```
    pub fn to_string_value(&self) -> String {
        match self {
            EvaluationResult::Empty => "".to_string(),
            EvaluationResult::Boolean(b, _) => b.to_string(),
            EvaluationResult::String(s, _) => s.clone(),
            EvaluationResult::Decimal(d, _) => d.to_string(),
            EvaluationResult::Integer(i, _) => i.to_string(),
            EvaluationResult::Integer64(i, _) => i.to_string(),
            EvaluationResult::Date(d, _) => d.clone(), // Return stored string
            EvaluationResult::DateTime(dt, _) => dt.clone(), // Return stored string
            EvaluationResult::Time(t, _) => t.clone(), // Return stored string
            EvaluationResult::Quantity(val, unit, _) => {
                // Format as "value 'unit'" per FHIRPath specification
                // Use UCUM formatting for calendar units
                let formatted_unit = format_unit_for_display(unit);
                format!("{} '{}'", val, formatted_unit)
            }
            EvaluationResult::Collection { items, .. } => {
                // FHIRPath toString rules for collections
                if items.len() == 1 {
                    // Single item: return its string value
                    items[0].to_string_value()
                } else {
                    // Multiple items: return bracketed comma-separated list
                    format!(
                        "[{}]",
                        items
                            .iter()
                            .map(|r| r.to_string_value())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                }
            }
            EvaluationResult::Object { .. } => "[object]".to_string(),
        }
    }

    /// Converts the result to Boolean for logical operators (and, or, xor, implies).
    ///
    /// This method implements the specific boolean conversion rules used by FHIRPath
    /// logical operators, which are different from general truthiness rules.
    ///
    /// # Conversion Rules
    ///
    /// - `Boolean`: returns the boolean value unchanged
    /// - `String`: converts "true"/"t"/"yes"/"1"/"1.0" to `true`,
    ///   "false"/"f"/"no"/"0"/"0.0" to `false`, others to `Empty`
    /// - `Collection`: single items are recursively converted, empty becomes `Empty`,
    ///   multiple items cause an error
    /// - Other types: result in `Empty`
    ///
    /// # Errors
    ///
    /// Returns `SingletonEvaluationError` if called on a collection with multiple items.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use helios_fhirpath_support::{EvaluationResult, EvaluationError};
    ///
    /// let true_str = EvaluationResult::String("true".to_string(), None);
    /// assert_eq!(true_str.to_boolean_for_logic().unwrap(), EvaluationResult::Boolean(true, None));
    ///
    /// let false_str = EvaluationResult::String("false".to_string(), None);
    /// assert_eq!(false_str.to_boolean_for_logic().unwrap(), EvaluationResult::Boolean(false, None));
    ///
    /// let other_str = EvaluationResult::String("maybe".to_string(), None);
    /// assert_eq!(other_str.to_boolean_for_logic().unwrap(), EvaluationResult::Empty);
    ///
    /// let integer = EvaluationResult::Integer(42, None);
    /// assert_eq!(integer.to_boolean_for_logic().unwrap(), EvaluationResult::Boolean(true, None));
    /// ```
    pub fn to_boolean_for_logic(&self) -> Result<EvaluationResult, EvaluationError> {
        match self {
            EvaluationResult::Boolean(b, type_info) => {
                Ok(EvaluationResult::Boolean(*b, type_info.clone()))
            }
            EvaluationResult::String(s, _) => {
                // Convert string to boolean based on recognized values
                Ok(match s.to_lowercase().as_str() {
                    "true" | "t" | "yes" | "1" | "1.0" => EvaluationResult::boolean(true),
                    "false" | "f" | "no" | "0" | "0.0" => EvaluationResult::boolean(false),
                    _ => EvaluationResult::Empty, // Unrecognized strings become Empty
                })
            }
            EvaluationResult::Collection { items, .. } => {
                match items.len() {
                    0 => Ok(EvaluationResult::Empty),
                    1 => items[0].to_boolean_for_logic(), // Recursive conversion
                    n => Err(EvaluationError::SingletonEvaluationError(format!(
                        "Boolean logic requires singleton collection, found {} items",
                        n
                    ))),
                }
            }
            EvaluationResult::Integer(i, _) => {
                // Integers have boolean semantics: 0 is false, non-zero is true
                Ok(EvaluationResult::boolean(*i != 0))
            }
            EvaluationResult::Integer64(i, _) => {
                // Integer64s have boolean semantics: 0 is false, non-zero is true
                Ok(EvaluationResult::boolean(*i != 0))
            }
            // Per FHIRPath spec section 5.2: other types evaluate to Empty for logical operators
            EvaluationResult::Decimal(_, _)
            | EvaluationResult::Date(_, _)
            | EvaluationResult::DateTime(_, _)
            | EvaluationResult::Time(_, _)
            | EvaluationResult::Quantity(_, _, _)
            | EvaluationResult::Object { .. } => Ok(EvaluationResult::Empty),
            EvaluationResult::Empty => Ok(EvaluationResult::Empty),
        }
    }

    /// Checks if the result is a String or Empty variant.
    ///
    /// This is a utility method used in various FHIRPath operations that
    /// need to distinguish string-like values from other types.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use helios_fhirpath_support::EvaluationResult;
    ///
    /// assert!(EvaluationResult::Empty.is_string_or_empty());
    /// assert!(EvaluationResult::String("test".to_string(), None).is_string_or_empty());
    /// assert!(!EvaluationResult::Integer(42, None).is_string_or_empty());
    /// ```
    pub fn is_string_or_empty(&self) -> bool {
        matches!(
            self,
            EvaluationResult::String(_, _) | EvaluationResult::Empty
        )
    }

    /// Returns the type name of this evaluation result.
    ///
    /// This method returns a string representation of the variant type,
    /// useful for error messages, debugging, and type checking operations.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use helios_fhirpath_support::EvaluationResult;
    ///
    /// assert_eq!(EvaluationResult::Empty.type_name(), "Empty");
    /// assert_eq!(EvaluationResult::String("test".to_string(), None).type_name(), "String");
    /// assert_eq!(EvaluationResult::Integer(42, None).type_name(), "Integer");
    ///
    /// let collection = EvaluationResult::Collection {
    ///     items: vec![],
    ///     has_undefined_order: false,
    ///     type_info: None,
    /// };
    /// assert_eq!(collection.type_name(), "Collection");
    /// ```
    pub fn type_name(&self) -> &'static str {
        match self {
            EvaluationResult::Empty => "Empty",
            EvaluationResult::Boolean(_, _) => "Boolean",
            EvaluationResult::String(_, _) => "String",
            EvaluationResult::Decimal(_, _) => "Decimal",
            EvaluationResult::Integer(_, _) => "Integer",
            EvaluationResult::Integer64(_, _) => "Integer64",
            EvaluationResult::Date(_, _) => "Date",
            EvaluationResult::DateTime(_, _) => "DateTime",
            EvaluationResult::Time(_, _) => "Time",
            EvaluationResult::Quantity(_, _, _) => "Quantity",
            EvaluationResult::Collection { .. } => "Collection",
            EvaluationResult::Object { .. } => "Object",
        }
    }
}

// === IntoEvaluationResult Implementations ===
//
// The following implementations provide conversions from standard Rust types
// and common patterns into EvaluationResult variants. These enable seamless
// integration between Rust code and the FHIRPath evaluation system.

/// Converts a `String` to `EvaluationResult::String`.
///
/// This is the most direct conversion for text values in the FHIRPath system.
impl IntoEvaluationResult for String {
    fn to_evaluation_result(&self) -> EvaluationResult {
        EvaluationResult::string(self.clone())
    }
}

/// Converts a `bool` to `EvaluationResult::Boolean`.
///
/// Enables direct use of Rust boolean values in FHIRPath expressions.
impl IntoEvaluationResult for bool {
    fn to_evaluation_result(&self) -> EvaluationResult {
        EvaluationResult::boolean(*self)
    }
}

/// Converts an `i32` to `EvaluationResult::Integer`.
///
/// Automatically promotes to `i64` for consistent integer handling.
impl IntoEvaluationResult for i32 {
    fn to_evaluation_result(&self) -> EvaluationResult {
        EvaluationResult::integer(*self as i64)
    }
}

/// Converts an `i64` to `EvaluationResult::Integer`.
///
/// This is the primary integer type used in FHIRPath evaluation.
impl IntoEvaluationResult for i64 {
    fn to_evaluation_result(&self) -> EvaluationResult {
        EvaluationResult::integer64(*self)
    }
}

/// Converts an `f64` to `EvaluationResult::Decimal` with error handling.
///
/// Uses high-precision `Decimal` type to avoid floating-point errors.
/// Returns `Empty` for invalid values like NaN or Infinity.
impl IntoEvaluationResult for f64 {
    fn to_evaluation_result(&self) -> EvaluationResult {
        Decimal::from_f64(*self)
            .map(EvaluationResult::decimal)
            .unwrap_or(EvaluationResult::Empty)
    }
}

/// Converts a `rust_decimal::Decimal` to `EvaluationResult::Decimal`.
///
/// This is the preferred conversion for precise decimal values in FHIR.
impl IntoEvaluationResult for Decimal {
    fn to_evaluation_result(&self) -> EvaluationResult {
        EvaluationResult::decimal(*self)
    }
}

// === Generic Container Implementations ===
//
// These implementations handle common Rust container types, enabling
// seamless conversion of complex data structures to FHIRPath results.

/// Converts `Option<T>` to either the inner value's result or `Empty`.
///
/// This is fundamental for handling FHIR's optional fields and nullable values.
/// `Some(value)` converts the inner value, `None` becomes `Empty`.
impl<T> IntoEvaluationResult for Option<T>
where
    T: IntoEvaluationResult,
{
    fn to_evaluation_result(&self) -> EvaluationResult {
        match self {
            Some(value) => value.to_evaluation_result(),
            None => EvaluationResult::Empty,
        }
    }
}

/// Converts `Vec<T>` to `EvaluationResult::Collection`.
///
/// Each item in the vector is converted to an `EvaluationResult`. The resulting
/// collection is marked as having defined order (FHIRPath collections maintain order).
impl<T> IntoEvaluationResult for Vec<T>
where
    T: IntoEvaluationResult,
{
    fn to_evaluation_result(&self) -> EvaluationResult {
        let collection: Vec<EvaluationResult> = self
            .iter()
            .map(|item| item.to_evaluation_result())
            .collect();
        EvaluationResult::Collection {
            items: collection,
            has_undefined_order: false,
            type_info: None,
        }
    }
}

/// Converts `Box<T>` to the result of the boxed value.
///
/// This enables use of boxed values (often used to break circular references
/// in FHIR data structures) directly in FHIRPath evaluation.
impl<T> IntoEvaluationResult for Box<T>
where
    T: IntoEvaluationResult + ?Sized,
{
    fn to_evaluation_result(&self) -> EvaluationResult {
        (**self).to_evaluation_result()
    }
}

/// Convenience function for converting values to evaluation results.
///
/// This function provides a unified interface for conversion that can be used
/// by the evaluator and macro systems. It's particularly useful when working
/// with trait objects or in generic contexts.
///
/// # Arguments
///
/// * `value` - Any value implementing `IntoEvaluationResult`
///
/// # Returns
///
/// The `EvaluationResult` representation of the input value.
///
/// # Examples
///
/// ```rust
/// use helios_fhirpath_support::{convert_value_to_evaluation_result, EvaluationResult};
///
/// let result = convert_value_to_evaluation_result(&"hello".to_string());
/// assert_eq!(result, EvaluationResult::String("hello".to_string(), None));
///
/// let numbers = vec![1, 2, 3];
/// let collection_result = convert_value_to_evaluation_result(&numbers);
/// assert_eq!(collection_result.count(), 3);
/// ```
pub fn convert_value_to_evaluation_result<T>(value: &T) -> EvaluationResult
where
    T: IntoEvaluationResult + ?Sized,
{
    value.to_evaluation_result()
}

/// Formats a unit for display, converting calendar units to UCUM format with curly braces
fn format_unit_for_display(unit: &str) -> String {
    // Calendar units that should be displayed with UCUM curly brace format for toString()
    match unit {
        // Only word-based time units get converted to UCUM braces for display
        // This matches the pattern where "week" becomes "{week}" but "wk" stays "wk"
        "week" => "{week}".to_string(),
        "year" => "{year}".to_string(),
        "month" => "{month}".to_string(),
        "day" => "{day}".to_string(),
        
        // Short UCUM codes and other units - display as-is
        // Note: "wk", "a", "mo", "d" etc. should NOT get braces when displaying
        other => other.to_string(),
    }
}
