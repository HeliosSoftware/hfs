#[cfg(test)]
mod tests {
    use helios_fhirpath::{EvaluationContext, evaluate_expression};
    use helios_fhirpath_support::EvaluationResult;
    use std::collections::HashMap;

    #[test]
    fn test_is_operator_with_fhir_resources() {
        // Create a Patient resource as a HashMap
        let mut patient = HashMap::new();
        patient.insert(
            "resourceType".to_string(),
            EvaluationResult::string("Patient".to_string()),
        );
        patient.insert(
            "id".to_string(),
            EvaluationResult::string("123".to_string()),
        );

        // Create an Observation resource as a HashMap
        let mut observation = HashMap::new();
        observation.insert(
            "resourceType".to_string(),
            EvaluationResult::string("Observation".to_string()),
        );
        observation.insert(
            "id".to_string(),
            EvaluationResult::string("456".to_string()),
        );

        // Create a context with the resources
        let mut context = EvaluationContext::new_empty_with_default_version();

        // Test cases for the 'is' operator
        let test_cases = vec![
            // Basic type tests (primitive types)
            (
                "true is Boolean",
                EvaluationResult::boolean(true),
                EvaluationResult::boolean(true),
            ),
            (
                "123 is Integer",
                EvaluationResult::integer(123),
                EvaluationResult::boolean(true),
            ),
            // Use single quotes for strings in FHIRPath
            (
                "'test' is String",
                EvaluationResult::string("test".to_string()),
                EvaluationResult::boolean(true),
            ),
            // FHIR resource type tests
            (
                "$this is Patient",
                EvaluationResult::object(patient.clone()),
                EvaluationResult::boolean(true),
            ),
            (
                "$this is Observation",
                EvaluationResult::object(patient.clone()),
                EvaluationResult::boolean(false),
            ),
            // FHIRPath uses dot notation for namespaces
            (
                "$this is FHIR.Patient",
                EvaluationResult::object(patient.clone()),
                EvaluationResult::boolean(true),
            ),
            // Test with Observation resource
            (
                "$this is Observation",
                EvaluationResult::object(observation.clone()),
                EvaluationResult::boolean(true),
            ),
            (
                "$this is Patient",
                EvaluationResult::object(observation.clone()),
                EvaluationResult::boolean(false),
            ),
        ];

        for (expression, input, expected) in test_cases {
            // Set the $this variable to the input
            context.set_this(input);

            // Evaluate the expression
            let result = evaluate_expression(expression, &context).unwrap();

            // Check if the result matches the expected result
            assert_eq!(result, expected, "Failed test for: {}", expression);
        }
    }

    #[test]
    fn test_as_operator_with_fhir_resources() {
        // Create a Patient resource as a HashMap
        let mut patient = HashMap::new();
        patient.insert(
            "resourceType".to_string(),
            EvaluationResult::string("Patient".to_string()),
        );
        patient.insert(
            "id".to_string(),
            EvaluationResult::string("123".to_string()),
        );

        // Create an Observation resource as a HashMap
        let mut observation = HashMap::new();
        observation.insert(
            "resourceType".to_string(),
            EvaluationResult::string("Observation".to_string()),
        );
        observation.insert(
            "id".to_string(),
            EvaluationResult::string("456".to_string()),
        );

        // Create a context with the resources
        let mut context = EvaluationContext::new_empty_with_default_version();

        // Test cases for the 'as' operator
        let test_cases = vec![
            // Basic type tests (primitive types)
            (
                "true as Boolean",
                EvaluationResult::boolean(true),
                EvaluationResult::boolean(true),
            ),
            // Use single quotes for strings in FHIRPath
            (
                "'test' as String",
                EvaluationResult::string("test".to_string()),
                EvaluationResult::string("test".to_string()),
            ),
            (
                "123 as Integer",
                EvaluationResult::integer(123),
                EvaluationResult::integer(123),
            ),
            // FHIR resource type tests
            (
                "$this as Patient",
                EvaluationResult::object(patient.clone()),
                EvaluationResult::object(patient.clone()),
            ),
            (
                "$this as Observation",
                EvaluationResult::object(patient.clone()),
                EvaluationResult::Empty,
            ),
            // FHIRPath uses dot notation for namespaces
            (
                "$this as FHIR.Patient",
                EvaluationResult::object(patient.clone()),
                EvaluationResult::object(patient.clone()),
            ),
            // Test with Observation resource
            (
                "$this as Observation",
                EvaluationResult::object(observation.clone()),
                EvaluationResult::object(observation.clone()),
            ),
            (
                "$this as Patient",
                EvaluationResult::object(observation.clone()),
                EvaluationResult::Empty,
            ),
        ];

        for (expression, input, expected) in test_cases {
            // Set the $this variable to the input
            context.set_this(input.clone());

            // Evaluate the expression
            let result = evaluate_expression(expression, &context).unwrap();

            // Check if the result matches the expected result
            assert_eq!(result, expected, "Failed test for: {}", expression);
        }
    }

    #[test]
    fn test_fhir_resource_without_resourcetype() {
        // Create an object without a resourceType field
        let mut invalid_resource = HashMap::new();
        invalid_resource.insert(
            "id".to_string(),
            EvaluationResult::string("123".to_string()),
        );

        // Create a context with the resource
        let mut context = EvaluationContext::new_empty_with_default_version();
        context.set_this(EvaluationResult::object(invalid_resource.clone()));

        // Test cases
        let test_cases = vec![
            ("$this is Patient", EvaluationResult::boolean(false)),
            ("$this as Patient", EvaluationResult::Empty),
        ];

        for (expression, expected) in test_cases {
            // Evaluate the expression
            let result = evaluate_expression(expression, &context).unwrap();

            // Check if the result matches the expected result
            assert_eq!(result, expected, "Failed test for: {}", expression);
        }
    }

    #[test]
    fn test_invalid_resourcetype() {
        // Create an object with a non-string resourceType field
        let mut invalid_resource = HashMap::new();
        invalid_resource.insert("resourceType".to_string(), EvaluationResult::integer(123));
        invalid_resource.insert(
            "id".to_string(),
            EvaluationResult::string("123".to_string()),
        );

        // Create a context with the resource
        let mut context = EvaluationContext::new_empty_with_default_version();
        context.set_this(EvaluationResult::object(invalid_resource.clone()));

        // Test cases
        let test_cases = vec![
            ("$this is Patient", EvaluationResult::boolean(false)),
            ("$this as Patient", EvaluationResult::Empty),
        ];

        for (expression, expected) in test_cases {
            // Evaluate the expression
            let result = evaluate_expression(expression, &context).unwrap();

            // Check if the result matches the expected result
            assert_eq!(result, expected, "Failed test for: {}", expression);
        }
    }
}
