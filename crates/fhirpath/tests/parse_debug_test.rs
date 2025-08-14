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
}
