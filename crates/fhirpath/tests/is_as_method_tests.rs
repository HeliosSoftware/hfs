#[cfg(test)]
mod tests {
    use chumsky::Parser;
    use fhirpath::evaluator::{EvaluationContext, evaluate};
    use fhirpath::parser::parser;
    use fhirpath_support::EvaluationResult;
    use std::collections::HashMap;

    #[test]
    fn test_is_method_form() {
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
        
        // Test cases for the .is() method
        let test_cases = vec![
            // Basic type tests (primitive types)
            ("true.is('Boolean')", EvaluationResult::Boolean(true), EvaluationResult::Boolean(true)),
            ("123.is('Integer')", EvaluationResult::Integer(123), EvaluationResult::Boolean(true)),
            // Use single quotes for strings in FHIRPath 
            ("'test'.is('String')", EvaluationResult::String("test".to_string()), EvaluationResult::Boolean(true)),
            
            // FHIR resource type tests
            ("$this.is('Patient')", EvaluationResult::Object(patient.clone()), EvaluationResult::Boolean(true)),
            ("$this.is('Observation')", EvaluationResult::Object(patient.clone()), EvaluationResult::Boolean(false)),
            
            // Test with namespace qualifiers
            ("$this.is('FHIR.Patient')", EvaluationResult::Object(patient.clone()), EvaluationResult::Boolean(true)),
            ("true.is('System.Boolean')", EvaluationResult::Boolean(true), EvaluationResult::Boolean(true)),
            
            // Test with Observation resource
            ("$this.is('Observation')", EvaluationResult::Object(observation.clone()), EvaluationResult::Boolean(true)),
            ("$this.is('Patient')", EvaluationResult::Object(observation.clone()), EvaluationResult::Boolean(false)),
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
    fn test_as_method_form() {
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
        
        // Test cases for the .as() method
        let test_cases = vec![
            // Basic type tests (primitive types)
            ("true.as('Boolean')", EvaluationResult::Boolean(true), EvaluationResult::Boolean(true)),
            // Use single quotes for strings in FHIRPath
            ("'test'.as('String')", EvaluationResult::String("test".to_string()), EvaluationResult::String("test".to_string())),
            ("123.as('Integer')", EvaluationResult::Integer(123), EvaluationResult::Integer(123)),
            
            // FHIR resource type tests
            ("$this.as('Patient')", EvaluationResult::Object(patient.clone()), EvaluationResult::Object(patient.clone())),
            ("$this.as('Observation')", EvaluationResult::Object(patient.clone()), EvaluationResult::Empty),
            
            // Test with namespace qualifiers
            ("$this.as('FHIR.Patient')", EvaluationResult::Object(patient.clone()), EvaluationResult::Object(patient.clone())),
            ("true.as('System.Boolean')", EvaluationResult::Boolean(true), EvaluationResult::Boolean(true)),
            
            // Test with Observation resource
            ("$this.as('Observation')", EvaluationResult::Object(observation.clone()), EvaluationResult::Object(observation.clone())),
            ("$this.as('Patient')", EvaluationResult::Object(observation.clone()), EvaluationResult::Empty),
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
    fn test_oftype_method_form() {
        // Create a collection with mixed types
        let collection = EvaluationResult::Collection(vec![
            EvaluationResult::Boolean(true),
            EvaluationResult::Integer(42),
            EvaluationResult::Boolean(false),
            EvaluationResult::String("test".to_string()),
        ]);
        
        // Create a context with the collection
        let mut context = EvaluationContext::new_empty();
        context.set_this(collection);
        
        // Test cases for the .ofType() method
        let test_cases = vec![
            // Filter for Boolean values
            ("$this.ofType('Boolean')", EvaluationResult::Collection(vec![
                EvaluationResult::Boolean(true),
                EvaluationResult::Boolean(false),
            ])),
            
            // Filter for String values
            ("$this.ofType('String')", EvaluationResult::String("test".to_string())),
            
            // Filter for Integer values
            ("$this.ofType('Integer')", EvaluationResult::Integer(42)),
            
            // Filter with System namespace
            ("$this.ofType('System.Boolean')", EvaluationResult::Collection(vec![
                EvaluationResult::Boolean(true),
                EvaluationResult::Boolean(false),
            ])),
            
            // Filter with no matches
            ("$this.ofType('Decimal')", EvaluationResult::Empty),
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