use rust_decimal::prelude::*;
use rust_decimal::Decimal;
use std::collections::HashMap; // Removed HashSet import
use std::cmp::Ordering;
use std::hash::{Hash, Hasher}; // Import Hash and Hasher

/// Trait to convert FHIR field values into EvaluationResult
pub trait IntoEvaluationResult {
    fn into_evaluation_result(&self) -> EvaluationResult;
}

/// Result of evaluating a FHIRPath expression
#[derive(Debug, Clone)] // Removed PartialEq, Eq here, will implement manually
pub enum EvaluationResult {
    Empty,
    Boolean(bool),
    String(String),
    Decimal(Decimal),
    Integer(i64),
    Date(String),
    DateTime(String),
    Time(String),
    Quantity(Decimal, String), // Added Quantity variant (value, unit)
    Collection {
        items: Vec<EvaluationResult>,
        has_undefined_order: bool,
    },
    Object(HashMap<String, EvaluationResult>),
}

/// Data for a collection, including items and order status
// Remove derived traits as EvaluationResult's implementations are manual
#[derive(Debug, Clone)] 
pub struct CollectionData {
    pub items: Vec<EvaluationResult>,
    pub has_undefined_order: bool,
}

/// Represents errors that can occur during FHIRPath evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvaluationError {
    TypeError(String), // e.g., "Expected Boolean, found Integer"
    InvalidArgument(String), // e.g., "Invalid argument for function 'where'"
    UndefinedVariable(String), // e.g., "Variable '%undefinedVar' not found"
    InvalidOperation(String), // e.g., "Cannot add String and Integer"
    InvalidArity(String), // e.g., "Function 'substring' expects 1 or 2 arguments, got 3"
    InvalidIndex(String), // e.g., "Index must be a non-negative integer"
    DivisionByZero,
    ArithmeticOverflow,
    InvalidRegex(String),
    InvalidTypeSpecifier(String),
    // Error for singleton evaluation failures
    SingletonEvaluationError(String), // e.g., "Expected singleton, found collection with N items"
    SemanticError(String), // Added for semantic errors like accessing non-existent members in strict mode
    // Add more specific errors as needed
    Other(String), // Generic error
}

// Implement standard Error trait for EvaluationError
impl std::error::Error for EvaluationError {}

// Implement Display trait for EvaluationError
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
            EvaluationError::InvalidTypeSpecifier(msg) => write!(f, "Invalid Type Specifier: {}", msg),
            EvaluationError::SingletonEvaluationError(msg) => write!(f, "Singleton Evaluation Error: {}", msg),
            EvaluationError::SemanticError(msg) => write!(f, "Semantic Error: {}", msg),
            EvaluationError::Other(msg) => write!(f, "Evaluation Error: {}", msg),
        }
    }
}

// --- Ord Implementation ---
// Define an arbitrary but consistent order for variants for sorting purposes.
// Note: This order does not necessarily reflect FHIRPath comparison rules,
// which are handled separately by compare_equality/compare_inequality.
impl PartialOrd for EvaluationResult {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other)) // Defer to Ord implementation
    }
}

impl Ord for EvaluationResult {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            // Compare based on variant order first
            (EvaluationResult::Empty, EvaluationResult::Empty) => Ordering::Equal,
            (EvaluationResult::Empty, _) => Ordering::Less,
            (_, EvaluationResult::Empty) => Ordering::Greater,

            (EvaluationResult::Boolean(a), EvaluationResult::Boolean(b)) => a.cmp(b),
            (EvaluationResult::Boolean(_), _) => Ordering::Less,
            (_, EvaluationResult::Boolean(_)) => Ordering::Greater,

            (EvaluationResult::Integer(a), EvaluationResult::Integer(b)) => a.cmp(b),
            (EvaluationResult::Integer(_), _) => Ordering::Less,
            (_, EvaluationResult::Integer(_)) => Ordering::Greater,

            (EvaluationResult::Decimal(a), EvaluationResult::Decimal(b)) => a.cmp(b),
            (EvaluationResult::Decimal(_), _) => Ordering::Less,
            (_, EvaluationResult::Decimal(_)) => Ordering::Greater,

            (EvaluationResult::String(a), EvaluationResult::String(b)) => a.cmp(b),
            (EvaluationResult::String(_), _) => Ordering::Less,
            (_, EvaluationResult::String(_)) => Ordering::Greater,

            (EvaluationResult::Date(a), EvaluationResult::Date(b)) => a.cmp(b),
            (EvaluationResult::Date(_), _) => Ordering::Less,
            (_, EvaluationResult::Date(_)) => Ordering::Greater,

            (EvaluationResult::DateTime(a), EvaluationResult::DateTime(b)) => a.cmp(b),
            (EvaluationResult::DateTime(_), _) => Ordering::Less,
            (_, EvaluationResult::DateTime(_)) => Ordering::Greater,

            (EvaluationResult::Time(a), EvaluationResult::Time(b)) => a.cmp(b),
            (EvaluationResult::Time(_), _) => Ordering::Less,
            (_, EvaluationResult::Time(_)) => Ordering::Greater,

            (EvaluationResult::Quantity(val_a, unit_a), EvaluationResult::Quantity(val_b, unit_b)) => {
                // Order by value first, then by unit string
                match val_a.cmp(val_b) {
                    Ordering::Equal => unit_a.cmp(unit_b),
                    other => other,
                }
            }
            (EvaluationResult::Quantity(_, _), _) => Ordering::Less,
            (_, EvaluationResult::Quantity(_, _)) => Ordering::Greater,

            (EvaluationResult::Collection { items: a_items, has_undefined_order: a_undef }, EvaluationResult::Collection { items: b_items, has_undefined_order: b_undef }) => {
                // Order by has_undefined_order first (false < true), then by items.
                match a_undef.cmp(b_undef) {
                    Ordering::Equal => {
                        // If order flags are the same, compare items as ordered lists.
                        // FHIRPath collections are ordered, even if the source order was "undefined".
                        a_items.cmp(b_items)
                    }
                    other => other,
                }
            }
            (EvaluationResult::Collection { .. }, _) => Ordering::Less,
            (_, EvaluationResult::Collection { .. }) => Ordering::Greater,

            (EvaluationResult::Object(a), EvaluationResult::Object(b)) => {
                // Compare objects based on sorted keys and then values
                let mut a_keys: Vec<_> = a.keys().collect();
                let mut b_keys: Vec<_> = b.keys().collect();
                a_keys.sort();
                b_keys.sort();

                match a_keys.cmp(&b_keys) {
                    Ordering::Equal => {
                        // Keys are the same, compare values in key order
                        for key in a_keys {
                            match a[key].cmp(&b[key]) {
                                Ordering::Equal => continue,
                                non_equal => return non_equal,
                            }
                        }
                        Ordering::Equal // All keys and values matched
                    }
                    non_equal => non_equal, // Keys differ
                }
            }
            // Object is the last variant in our defined order
            // (EvaluationResult::Object(_), _) => Ordering::Less, // This arm is unreachable due to previous matches
            // (_, EvaluationResult::Object(_)) => Ordering::Greater, // This arm is unreachable
        }
    }
}
// --- End Ord Implementation ---


// --- Hash Implementation ---

// Implement Hash for EvaluationResult to use it in HashSet for distinct/intersect
// Note: Hashing floating-point numbers (Decimal) directly can be problematic due to precision.
// We'll hash their string representation for stability. Hashing collections/objects might be complex.
impl Hash for EvaluationResult {
    fn hash<H: Hasher>(&self, state: &mut H) {
        core::mem::discriminant(self).hash(state); // Hash the enum variant first
        match self {
            EvaluationResult::Empty => {} // No extra data to hash
            EvaluationResult::Boolean(b) => b.hash(state),
            EvaluationResult::String(s) => s.hash(state),
            // Hash the normalized decimal value for consistency with PartialEq
            EvaluationResult::Decimal(d) => d.normalize().hash(state),
            EvaluationResult::Integer(i) => i.hash(state),
            EvaluationResult::Date(d) => d.hash(state),
            EvaluationResult::DateTime(dt) => dt.hash(state),
            EvaluationResult::Time(t) => t.hash(state),
            EvaluationResult::Quantity(val, unit) => {
                val.normalize().hash(state); // Hash normalized decimal value
                unit.hash(state); // Hash the unit string
            }
            EvaluationResult::Collection { items, has_undefined_order } => {
                has_undefined_order.hash(state); // Hash the flag
                items.len().hash(state);         // Hash length
                for item in items {              // Hash items in their given order
                    item.hash(state);
                }
            }
            EvaluationResult::Object(map) => {
                // Hashing HashMaps requires sorting keys for consistency
                let mut keys: Vec<_> = map.keys().collect();
                keys.sort(); // Sort keys alphabetically
                keys.len().hash(state); // Hash the number of keys
                for key in keys {
                    key.hash(state); // Hash the key
                    map[key].hash(state); // Hash the value
                }
            }
        }
    }
}


impl EvaluationResult {
    /// Checks if the result is a collection variant.
    pub fn is_collection(&self) -> bool {
        matches!(self, EvaluationResult::Collection { .. })
    }

    /// Returns the count of items in the result according to FHIRPath rules.
    pub fn count(&self) -> usize {
        match self {
            EvaluationResult::Empty => 0,
            EvaluationResult::Collection { items, .. } => items.len(),
            _ => 1, // All single items count as 1
        }
    }
    /// Converts the result to a boolean value according to FHIRPath rules
    pub fn to_boolean(&self) -> bool {
        match self {
            EvaluationResult::Empty => false,
            EvaluationResult::Boolean(b) => *b,
            EvaluationResult::String(s) => !s.is_empty(),
            EvaluationResult::Decimal(d) => !d.is_zero(),
            EvaluationResult::Integer(i) => *i != 0,
            EvaluationResult::Quantity(q, _) => !q.is_zero(), // Quantity is truthy if value is non-zero
            EvaluationResult::Collection { items, .. } => !items.is_empty(),
            _ => true, // Other types (Date, DateTime, Time, Object) are considered truthy
        }
    }

    /// Converts the result to a string representation
    pub fn to_string_value(&self) -> String {
        match self {
            EvaluationResult::Empty => "".to_string(),
            EvaluationResult::Boolean(b) => b.to_string(),
            EvaluationResult::String(s) => s.clone(),
            EvaluationResult::Decimal(d) => d.to_string(),
            EvaluationResult::Integer(i) => i.to_string(),
            EvaluationResult::Date(d) => d.clone(), // Return stored string
            EvaluationResult::DateTime(dt) => dt.clone(), // Return stored string
            EvaluationResult::Time(t) => t.clone(), // Return stored string
            EvaluationResult::Quantity(val, unit) => format!("{} '{}'", val, unit), // Format as "value 'unit'"
            EvaluationResult::Collection { items, .. } => {
                // toString on collection: Empty if 0 or >1 items, string of item if 1 item
                if items.len() == 1 {
                    items[0].to_string_value()
                } else {
                    format!(
                        "[{}]",
                        items.iter()
                            .map(|r| r.to_string_value())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                }
            }
            EvaluationResult::Object(_) => "[object]".to_string(),
        }
    }

    /// Converts the result to Boolean specifically for logical operators (and, or, xor, implies).
    /// Handles String conversion ('true'/'false' variants) -> Boolean.
    /// Other types (including other strings) result in Empty.
    /// Collections with > 1 item result in an error.
    /// Single-item collections are evaluated based on the item.
    pub fn to_boolean_for_logic(&self) -> Result<EvaluationResult, EvaluationError> {
        match self {
            EvaluationResult::Boolean(b) => Ok(EvaluationResult::Boolean(*b)),
            EvaluationResult::String(s) => Ok(match s.to_lowercase().as_str() {
                "true" | "t" | "yes" | "1" | "1.0" => EvaluationResult::Boolean(true),
                "false" | "f" | "no" | "0" | "0.0" => EvaluationResult::Boolean(false),
                _ => EvaluationResult::Empty, // Other strings evaluate to empty in boolean logic
            }),
            EvaluationResult::Collection { items, .. } => {
                if items.len() == 1 {
                    // Recursively call on the single item
                    items[0].to_boolean_for_logic()
                } else if items.is_empty() {
                    Ok(EvaluationResult::Empty) // Empty collection -> Empty
                } else {
                    // Multi-item collection -> Error
                    Err(EvaluationError::SingletonEvaluationError(format!(
                        "Boolean logic requires singleton collection, found {} items", items.len()
                    )))
                }
            }
            // Other types evaluate to Empty for logical operators per spec section 5.2
            EvaluationResult::Integer(_)
            | EvaluationResult::Decimal(_)
            | EvaluationResult::Date(_)
            | EvaluationResult::DateTime(_)
            | EvaluationResult::Time(_)
            | EvaluationResult::Quantity(_, _) // Quantity evaluates to Empty in boolean logic
            | EvaluationResult::Object(_) => Ok(EvaluationResult::Empty),
            EvaluationResult::Empty => Ok(EvaluationResult::Empty),
        }
    }

    /// Helper to check if the result is a String or Empty
    pub fn is_string_or_empty(&self) -> bool {
        matches!(self, EvaluationResult::String(_) | EvaluationResult::Empty)
    }

    /// Returns a string representation of the variant name for debugging/errors.
    pub fn type_name(&self) -> &'static str {
        match self {
            EvaluationResult::Empty => "Empty",
            EvaluationResult::Boolean(_) => "Boolean",
            EvaluationResult::String(_) => "String",
            EvaluationResult::Decimal(_) => "Decimal",
            EvaluationResult::Integer(_) => "Integer",
            EvaluationResult::Date(_) => "Date",
            EvaluationResult::DateTime(_) => "DateTime",
            EvaluationResult::Time(_) => "Time",
            EvaluationResult::Quantity(_, _) => "Quantity",
            EvaluationResult::Collection { .. } => "Collection",
            EvaluationResult::Object(_) => "Object",
        }
    }
}

// --- Implementations for Rust Primitives ---

impl IntoEvaluationResult for String {
    fn into_evaluation_result(&self) -> EvaluationResult {
        EvaluationResult::String(self.clone())
    }
}

impl IntoEvaluationResult for bool {
    fn into_evaluation_result(&self) -> EvaluationResult {
        EvaluationResult::Boolean(*self)
    }
}

impl IntoEvaluationResult for i32 {
    fn into_evaluation_result(&self) -> EvaluationResult {
        EvaluationResult::Integer(*self as i64)
    }
}

impl IntoEvaluationResult for i64 {
    fn into_evaluation_result(&self) -> EvaluationResult {
        EvaluationResult::Integer(*self)
    }
}

impl IntoEvaluationResult for f64 { // Convert f64 to Decimal
    fn into_evaluation_result(&self) -> EvaluationResult {
        Decimal::from_f64(*self)
            .map(EvaluationResult::Decimal)
            .unwrap_or(EvaluationResult::Empty) // Handle potential conversion errors (e.g., NaN, Infinity)
    }
}

// --- Implementation for rust_decimal ---
impl IntoEvaluationResult for Decimal {
    fn into_evaluation_result(&self) -> EvaluationResult {
        EvaluationResult::Decimal(*self)
    }
}

// --- Implementations for Option<T>, Vec<T>, Box<T> ---

impl<T> IntoEvaluationResult for Option<T>
where
    T: IntoEvaluationResult,
{
    fn into_evaluation_result(&self) -> EvaluationResult {
        match self {
            Some(value) => value.into_evaluation_result(),
            None => EvaluationResult::Empty,
        }
    }
}

impl<T> IntoEvaluationResult for Vec<T>
where
    T: IntoEvaluationResult,
{
    fn into_evaluation_result(&self) -> EvaluationResult {
        let collection: Vec<EvaluationResult> = self
            .iter()
            .map(|item| item.into_evaluation_result())
            .collect();
        EvaluationResult::Collection { items: collection, has_undefined_order: false } // Default to ordered for new collections from Vec<T>
    }
}

impl<T> IntoEvaluationResult for Box<T>
where
    T: IntoEvaluationResult + ?Sized, // Add ?Sized here
{
    fn into_evaluation_result(&self) -> EvaluationResult {
        (**self).into_evaluation_result()
    }
}

// The actual function used by the evaluator/macro
pub fn convert_value_to_evaluation_result<T>(value: &T) -> EvaluationResult
where
    T: IntoEvaluationResult + ?Sized, // Add ?Sized for potential dyn Trait use later
{
    value.into_evaluation_result()
}
