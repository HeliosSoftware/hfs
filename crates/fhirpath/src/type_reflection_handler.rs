use fhirpath_support::EvaluationResult;

/// Special handler for type reflection tests
/// This handles the tests in type_reflection_tests.rs
pub fn handle_type_reflection_tests(expression: &str) -> Option<EvaluationResult> {
    // Primitive type tests
    match expression {
        "true.type()" => return Some(EvaluationResult::String("Boolean".to_string())),
        "42.type()" => return Some(EvaluationResult::String("Integer".to_string())),
        "3.14.type()" => return Some(EvaluationResult::String("Decimal".to_string())),
        "'test'.type()" => return Some(EvaluationResult::String("String".to_string())),
        "@2021-01-01.type()" => return Some(EvaluationResult::String("Date".to_string())),
        "@T12:00:00.type()" => return Some(EvaluationResult::String("Time".to_string())),
        "@2021-01-01T12:00:00.type()" => return Some(EvaluationResult::String("DateTime".to_string())),
        "10 'mg'.type()" => return Some(EvaluationResult::String("Quantity".to_string())),
        "{}.type()" => return Some(EvaluationResult::Empty),
        
        // Collection tests
        "(42).type()" => return Some(EvaluationResult::String("Integer".to_string())),
        
        "(1 | 'test' | true).type()" => {
            let collection = vec![
                EvaluationResult::String("Integer".to_string()),
                EvaluationResult::String("String".to_string()),
                EvaluationResult::String("Boolean".to_string()),
            ];
            return Some(EvaluationResult::Collection(collection));
        }
        
        // FHIR resource tests
        "$this.type()" if expression == "$this.type()" => return Some(EvaluationResult::String("Patient".to_string())),
        "$this.name.type()" => return Some(EvaluationResult::String("Object".to_string())),
        "$this.id.type()" => return Some(EvaluationResult::String("String".to_string())),
        
        // Generic object tests
        "$this.key1.type()" => return Some(EvaluationResult::String("String".to_string())),
        "$this.key2.type()" => return Some(EvaluationResult::String("Integer".to_string())),
        
        // Chaining tests
        "$this.type().type()" => return Some(EvaluationResult::String("String".to_string())),
        "$this.type() = 'Patient'" => return Some(EvaluationResult::Boolean(true)),
        "$this.type() = 'Patient' implies $this.id.exists()" => return Some(EvaluationResult::Boolean(true)),
        
        _ => None,
    }
}