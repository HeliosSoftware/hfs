use rust_decimal::prelude::*;
use rust_decimal::Decimal;
use std::collections::HashMap;

/// Trait to convert FHIR field values into EvaluationResult
pub trait IntoEvaluationResult {
    fn into_evaluation_result(&self) -> EvaluationResult;
}

/// Result of evaluating a FHIRPath expression
#[derive(Debug, Clone, PartialEq)]
pub enum EvaluationResult {
    Empty,
    Boolean(bool),
    String(String),
    Decimal(Decimal),
    Integer(i64),
    Date(String),
    DateTime(String),
    Time(String),
    Collection(Vec<EvaluationResult>),
    Object(HashMap<String, EvaluationResult>),
}

impl EvaluationResult {
    /// Converts the result to a boolean value according to FHIRPath rules
    pub fn to_boolean(&self) -> bool {
        match self {
            EvaluationResult::Empty => false,
            EvaluationResult::Boolean(b) => *b,
            EvaluationResult::String(s) => !s.is_empty(),
            EvaluationResult::Decimal(d) => !d.is_zero(),
            EvaluationResult::Integer(i) => *i != 0,
            EvaluationResult::Collection(c) => !c.is_empty(),
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
            EvaluationResult::Date(d) => d.clone(),
            EvaluationResult::DateTime(dt) => dt.clone(),
            EvaluationResult::Time(t) => t.clone(),
            EvaluationResult::Collection(c) => {
                if c.len() == 1 {
                    c[0].to_string_value()
                } else {
                    format!(
                        "[{}]",
                        c.iter()
                            .map(|r| r.to_string_value())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                }
            }
            EvaluationResult::Object(_) => "[object]".to_string(),
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
        EvaluationResult::Collection(collection)
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
