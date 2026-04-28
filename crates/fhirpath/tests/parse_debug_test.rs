#[cfg(test)]
mod tests {
    use chumsky::Parser;
    use helios_fhirpath::parse_debug::expression_to_debug_tree;
    use helios_fhirpath::type_inference::{InferredType, TypeContext};

    #[test]
    fn test_parse_debug_tree_with_types() {
        // The expression from the example: trace('trc').given.join(' ').combine(family).join(', ')
        let expression = "trace('trc').given.join(' ').combine(family).join(', ')";

        // Parse the expression
        let parsed = helios_fhirpath::parser::parser()
            .parse(expression)
            .into_result()
            .expect("Failed to parse expression");

        // Create a type context with Patient.name as the root type
        let type_context = TypeContext::new().with_root_type(InferredType::fhir("HumanName"));

        // Generate the debug tree
        let debug_tree = expression_to_debug_tree(&parsed, &type_context);

        // Pretty print the JSON
        let json_string =
            serde_json::to_string_pretty(&debug_tree).expect("Failed to serialize JSON");

        println!("parseDebugTree for expression: {}", expression);
        println!("{}", json_string);

        // Check that the root node has ReturnType
        assert!(debug_tree.get("ReturnType").is_some());

        // Check the structure matches expected format
        assert_eq!(
            debug_tree.get("ExpressionType").and_then(|v| v.as_str()),
            Some("FunctionCallExpression")
        );
        assert_eq!(
            debug_tree.get("Name").and_then(|v| v.as_str()),
            Some("join")
        );
    }

    #[test]
    fn test_simple_member_access() {
        let expression = "given";

        let parsed = helios_fhirpath::parser::parser()
            .parse(expression)
            .into_result()
            .expect("Failed to parse expression");

        let type_context = TypeContext::new().with_root_type(InferredType::fhir("HumanName"));

        let debug_tree = expression_to_debug_tree(&parsed, &type_context);

        println!("Simple member access debug tree:");
        println!("{}", serde_json::to_string_pretty(&debug_tree).unwrap());

        // Check for builtin.that
        let args = debug_tree.get("Arguments").and_then(|a| a.as_array());
        assert!(args.is_some());
        let args = args.unwrap();
        assert!(!args.is_empty());
        assert_eq!(
            args[0].get("Name").and_then(|n| n.as_str()),
            Some("builtin.that")
        );
    }

    fn infer_display(expression: &str, root: &str) -> Option<String> {
        let parsed = helios_fhirpath::parser::parser()
            .parse(expression)
            .into_result()
            .expect("Failed to parse expression");
        let context = TypeContext::new().with_root_type(InferredType::fhir(root));
        helios_fhirpath::type_inference::infer_expression_type(&parsed, &context)
            .map(|t| t.to_display_string())
    }

    #[test]
    fn test_observation_code_is_codeable_concept() {
        assert_eq!(
            infer_display("code", "Observation").as_deref(),
            Some("CodeableConcept")
        );
    }

    #[test]
    fn test_patient_name_given_is_string_collection() {
        // Using `name` since `name` collection of HumanName, then `.given` returns a string collection.
        assert_eq!(
            infer_display("name.given", "Patient").as_deref(),
            Some("system.String[]")
        );
    }

    #[test]
    fn test_patient_contact_backbone_lookup() {
        // Type inference walks the schema: Patient.contact (PatientContact backbone) → name (HumanName).
        // Collection-ness propagation is not currently modelled; this test documents the
        // schema-driven lookup, not FHIRPath collection semantics.
        assert_eq!(
            infer_display("contact.name", "Patient").as_deref(),
            Some("HumanName")
        );
        assert_eq!(
            infer_display("contact.name.family", "Patient").as_deref(),
            Some("system.String")
        );
    }

    #[test]
    fn test_choice_typed_variant_resolves() {
        // Observation.valueQuantity → Quantity (concrete choice variant)
        assert_eq!(
            infer_display("valueQuantity", "Observation").as_deref(),
            Some("Quantity")
        );
    }

    #[test]
    fn test_unknown_field_returns_none() {
        // Unknown root type with unknown field → None
        let parsed = helios_fhirpath::parser::parser()
            .parse("notARealField")
            .into_result()
            .expect("Failed to parse expression");
        let context = TypeContext::new().with_root_type(InferredType::fhir("NotARealResourceType"));
        assert!(
            helios_fhirpath::type_inference::infer_expression_type(&parsed, &context).is_none()
        );
    }
}
