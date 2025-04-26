use std::collections::HashMap;

/// Result of evaluating a FHIRPath expression
#[derive(Debug, Clone, PartialEq)]
pub enum EvaluationResult {
    Empty,
    Boolean(bool),
    String(String),
    Number(f64),
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
            EvaluationResult::Number(n) => *n != 0.0,
            EvaluationResult::Integer(i) => *i != 0,
            EvaluationResult::Collection(c) => !c.is_empty(),
            _ => true, // Other types are considered truthy
        }
    }

    /// Converts the result to a string representation
    pub fn to_string_value(&self) -> String {
        match self {
            EvaluationResult::Empty => "".to_string(),
            EvaluationResult::Boolean(b) => b.to_string(),
            EvaluationResult::String(s) => s.clone(),
            EvaluationResult::Number(n) => n.to_string(),
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

// Trait to convert FHIR field values into EvaluationResult
pub trait IntoEvaluationResult {
    fn into_evaluation_result(&self) -> EvaluationResult;
}
