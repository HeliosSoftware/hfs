#[cfg(test)]
mod tests {
    use helios_fhirpath::{EvaluationContext, evaluate_expression};
    use helios_fhirpath_support::EvaluationResult;
    use std::collections::HashMap;

    #[test]
    fn test_is_method_form() {
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

        // Test cases for the .is() method
        let test_cases = vec![
            // Basic type tests (primitive types)
            (
                "true.is('Boolean')",
                EvaluationResult::boolean(true),
                EvaluationResult::boolean(true),
            ),
            (
                "123.is('Integer')",
                EvaluationResult::integer(123),
                EvaluationResult::boolean(true),
            ),
            // Use single quotes for strings in FHIRPath
            (
                "'test'.is('String')",
                EvaluationResult::string("test".to_string()),
                EvaluationResult::boolean(true),
            ),
            // FHIR resource type tests
            (
                "$this.is('Patient')",
                EvaluationResult::object(patient.clone()),
                EvaluationResult::boolean(true),
            ),
            (
                "$this.is('Observation')",
                EvaluationResult::object(patient.clone()),
                EvaluationResult::boolean(false),
            ),
            // Test with namespace qualifiers
            (
                "$this.is('FHIR.Patient')",
                EvaluationResult::object(patient.clone()),
                EvaluationResult::boolean(true),
            ),
            (
                "true.is('System.Boolean')",
                EvaluationResult::boolean(true),
                EvaluationResult::boolean(true),
            ),
            // Test with Observation resource
            (
                "$this.is('Observation')",
                EvaluationResult::object(observation.clone()),
                EvaluationResult::boolean(true),
            ),
            (
                "$this.is('Patient')",
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
    fn test_as_method_form() {
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

        // Test cases for the .as() method
        let test_cases = vec![
            // Basic type tests (primitive types)
            (
                "true.as('Boolean')",
                EvaluationResult::boolean(true),
                EvaluationResult::boolean(true),
            ),
            // Use single quotes for strings in FHIRPath
            (
                "'test'.as('String')",
                EvaluationResult::string("test".to_string()),
                EvaluationResult::string("test".to_string()),
            ),
            (
                "123.as('Integer')",
                EvaluationResult::integer(123),
                EvaluationResult::integer(123),
            ),
            // FHIR resource type tests
            (
                "$this.as('Patient')",
                EvaluationResult::object(patient.clone()),
                EvaluationResult::object(patient.clone()),
            ),
            (
                "$this.as('Observation')",
                EvaluationResult::object(patient.clone()),
                EvaluationResult::Empty,
            ),
            // Test with namespace qualifiers
            (
                "$this.as('FHIR.Patient')",
                EvaluationResult::object(patient.clone()),
                EvaluationResult::object(patient.clone()),
            ),
            (
                "true.as('System.Boolean')",
                EvaluationResult::boolean(true),
                EvaluationResult::boolean(true),
            ),
            // Test with Observation resource
            (
                "$this.as('Observation')",
                EvaluationResult::object(observation.clone()),
                EvaluationResult::object(observation.clone()),
            ),
            (
                "$this.as('Patient')",
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
    fn test_oftype_method_form() {
        // Create a collection with mixed types
        let collection = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::boolean(true),
                EvaluationResult::integer(42),
                EvaluationResult::boolean(false),
                EvaluationResult::string("test".to_string()),
            ],
            has_undefined_order: false,
            type_info: None,
        };

        // Create a context with the collection
        let mut context = EvaluationContext::new_empty_with_default_version();
        context.set_this(collection);

        // Test cases for the .ofType() method
        let test_cases = vec![
            // Filter for Boolean values
            (
                "$this.ofType('Boolean')",
                EvaluationResult::Collection {
                    items: vec![
                        EvaluationResult::boolean(true),
                        EvaluationResult::boolean(false),
                    ],
                    has_undefined_order: false,
                    type_info: None,
                },
            ),
            // Filter for String values
            (
                "$this.ofType('String')",
                EvaluationResult::string("test".to_string()),
            ),
            // Filter for Integer values
            ("$this.ofType('Integer')", EvaluationResult::integer(42)),
            // Filter with System namespace
            (
                "$this.ofType('System.Boolean')",
                EvaluationResult::Collection {
                    items: vec![
                        EvaluationResult::boolean(true),
                        EvaluationResult::boolean(false),
                    ],
                    has_undefined_order: false,
                    type_info: None,
                },
            ),
            // Filter with no matches
            ("$this.ofType('Decimal')", EvaluationResult::Empty),
        ];

        for (expression, expected) in test_cases {
            // Evaluate the expression
            let result = evaluate_expression(expression, &context).unwrap();

            // Check if the result matches the expected result
            assert_eq!(result, expected, "Failed test for: {}", expression);
        }
    }
}
