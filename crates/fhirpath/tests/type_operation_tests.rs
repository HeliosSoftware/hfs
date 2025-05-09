#[cfg(test)]
mod tests {
    use chumsky::Parser;
    use fhirpath::evaluator::{EvaluationContext, evaluate};
    use fhirpath::parser::parser;
    use fhirpath_support::EvaluationResult;
    use std::collections::HashMap;

    #[test]
    fn test_is_operator_with_fhir_resources() {
        // Create a Patient resource as a HashMap
        let mut patient = HashMap::new();
        patient.insert("resourceType".to_string(), EvaluationResult::String("Patient".to_string()));
        patient.insert("id".to_string(), EvaluationResult::String("123".to_string()));
        
        // Create an Observation resource as a HashMap
        let mut observation = HashMap::new();
        observation.insert("resourceType".to_string(), EvaluationResult::String("Observation".to_string()));
        observation.insert("id".to_string(), EvaluationResult::String("456".to_string()));
        
        // Create a context with the resources
        let mut context = EvaluationContext::new_empty();
        
        // Test cases for the 'is' operator
        let test_cases = vec![
            // Basic type tests (primitive types)
            ("true is Boolean", EvaluationResult::Boolean(true), EvaluationResult::Boolean(true)),
            ("123 is Integer", EvaluationResult::Integer(123), EvaluationResult::Boolean(true)),
            // Use single quotes for strings in FHIRPath 
            ("'test' is String", EvaluationResult::String("test".to_string()), EvaluationResult::Boolean(true)),
            
            // FHIR resource type tests
            ("$this is Patient", EvaluationResult::Object(patient.clone()), EvaluationResult::Boolean(true)),
            ("$this is Observation", EvaluationResult::Object(patient.clone()), EvaluationResult::Boolean(false)),
            // FHIRPath uses dot notation for namespaces
            ("$this is FHIR.Patient", EvaluationResult::Object(patient.clone()), EvaluationResult::Boolean(true)),
            
            // Test with Observation resource
            ("$this is Observation", EvaluationResult::Object(observation.clone()), EvaluationResult::Boolean(true)),
            ("$this is Patient", EvaluationResult::Object(observation.clone()), EvaluationResult::Boolean(false)),
        ];
        
        for (expression, input, expected) in test_cases {
            // Parse the expression
            let parsed = parser().parse(expression).unwrap();
            
            // Set the $this variable to the input
            context.set_this(input);
            
            // Evaluate the expression
            let result = evaluate(&parsed, &context, None).unwrap();
            
            // Check if the result matches the expected result
            assert_eq!(result, expected, "Failed test for: {}", expression);
        }
    }
    
    #[test]
    fn test_as_operator_with_fhir_resources() {
        // Create a Patient resource as a HashMap
        let mut patient = HashMap::new();
        patient.insert("resourceType".to_string(), EvaluationResult::String("Patient".to_string()));
        patient.insert("id".to_string(), EvaluationResult::String("123".to_string()));
        
        // Create an Observation resource as a HashMap
        let mut observation = HashMap::new();
        observation.insert("resourceType".to_string(), EvaluationResult::String("Observation".to_string()));
        observation.insert("id".to_string(), EvaluationResult::String("456".to_string()));
        
        // Create a context with the resources
        let mut context = EvaluationContext::new_empty();
        
        // Test cases for the 'as' operator
        let test_cases = vec![
            // Basic type tests (primitive types)
            ("true as Boolean", EvaluationResult::Boolean(true), EvaluationResult::Boolean(true)),
            // Use single quotes for strings in FHIRPath
            ("'test' as String", EvaluationResult::String("test".to_string()), EvaluationResult::String("test".to_string())),
            ("123 as Integer", EvaluationResult::Integer(123), EvaluationResult::Integer(123)),
            
            // FHIR resource type tests
            ("$this as Patient", EvaluationResult::Object(patient.clone()), EvaluationResult::Object(patient.clone())),
            ("$this as Observation", EvaluationResult::Object(patient.clone()), EvaluationResult::Empty),
            // FHIRPath uses dot notation for namespaces
            ("$this as FHIR.Patient", EvaluationResult::Object(patient.clone()), EvaluationResult::Object(patient.clone())),
            
            // Test with Observation resource
            ("$this as Observation", EvaluationResult::Object(observation.clone()), EvaluationResult::Object(observation.clone())),
            ("$this as Patient", EvaluationResult::Object(observation.clone()), EvaluationResult::Empty),
        ];
        
        for (expression, input, expected) in test_cases {
            // Parse the expression
            let parsed = parser().parse(expression).unwrap();
            
            // Set the $this variable to the input
            context.set_this(input.clone());
            
            // Evaluate the expression
            let result = evaluate(&parsed, &context, None).unwrap();
            
            // Check if the result matches the expected result
            assert_eq!(result, expected, "Failed test for: {}", expression);
        }
    }
    
    #[test]
    fn test_fhir_resource_without_resourcetype() {
        // Create an object without a resourceType field
        let mut invalid_resource = HashMap::new();
        invalid_resource.insert("id".to_string(), EvaluationResult::String("123".to_string()));
        
        // Create a context with the resource
        let mut context = EvaluationContext::new_empty();
        context.set_this(EvaluationResult::Object(invalid_resource.clone()));
        
        // Test cases
        let test_cases = vec![
            ("$this is Patient", EvaluationResult::Boolean(false)),
            ("$this as Patient", EvaluationResult::Empty),
        ];
        
        for (expression, expected) in test_cases {
            // Parse the expression
            let parsed = parser().parse(expression).unwrap();
            
            // Evaluate the expression
            let result = evaluate(&parsed, &context, None).unwrap();
            
            // Check if the result matches the expected result
            assert_eq!(result, expected, "Failed test for: {}", expression);
        }
    }
    
    #[test]
    fn test_invalid_resourcetype() {
        // Create an object with a non-string resourceType field
        let mut invalid_resource = HashMap::new();
        invalid_resource.insert("resourceType".to_string(), EvaluationResult::Integer(123));
        invalid_resource.insert("id".to_string(), EvaluationResult::String("123".to_string()));
        
        // Create a context with the resource
        let mut context = EvaluationContext::new_empty();
        context.set_this(EvaluationResult::Object(invalid_resource.clone()));
        
        // Test cases
        let test_cases = vec![
            ("$this is Patient", EvaluationResult::Boolean(false)),
            ("$this as Patient", EvaluationResult::Empty),
        ];
        
        for (expression, expected) in test_cases {
            // Parse the expression
            let parsed = parser().parse(expression).unwrap();
            
            // Evaluate the expression
            let result = evaluate(&parsed, &context, None).unwrap();
            
            // Check if the result matches the expected result
            assert_eq!(result, expected, "Failed test for: {}", expression);
        }
    }
}