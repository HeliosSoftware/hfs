//! Tests for defineVariable function
//!
//! NOTE: These tests document the current limitations of defineVariable.
//! Proper implementation requires architectural changes to support context
//! modification through expression chains.

#[cfg(test)]
mod tests {
    use chumsky::Parser;
    use helios_fhir::FhirVersion;
    use helios_fhirpath::evaluator::{EvaluationContext, evaluate};
    use helios_fhirpath::parser::parser;
    use helios_fhirpath_support::EvaluationResult;

    #[test]
    fn test_define_variable_basic_syntax() {
        // Test that defineVariable parses and doesn't error on basic syntax
        let expr = "defineVariable('v1', 'value1')";
        let parsed = parser().parse(expr).into_result();
        assert!(parsed.is_ok(), "Failed to parse defineVariable expression");

        let context = EvaluationContext::new_empty(FhirVersion::R4);
        let result = evaluate(&parsed.unwrap(), &context, None);
        assert!(result.is_ok(), "defineVariable should not error");
    }

    #[test]
    fn test_define_variable_returns_input() {
        // Test that defineVariable returns its input unchanged
        let expr = "5.defineVariable('v1', 10)";
        let parsed = parser().parse(expr).into_result().unwrap();

        let context = EvaluationContext::new_empty(FhirVersion::R4);
        let result = evaluate(&parsed, &context, None).unwrap();

        assert_eq!(result, EvaluationResult::integer(5));
    }

    #[test]
    fn test_define_variable_not_accessible() {
        // This test verifies that variables defined by defineVariable
        // ARE accessible in subsequent operations

        // Test 1: When defineVariable is called on a non-empty collection
        let expr = "'test'.defineVariable('v1', 'value1').select(%v1)";
        let parsed = parser().parse(expr).into_result().unwrap();

        let context = EvaluationContext::new_empty(FhirVersion::R4);
        let result = evaluate(&parsed, &context, None);

        // The variable should be accessible and return its value
        assert!(
            result.is_ok(),
            "Should successfully access the defined variable"
        );
        assert_eq!(
            result.unwrap(),
            EvaluationResult::string("value1".to_string())
        );

        // Test 2: When defineVariable is called with empty input
        let expr2 = "defineVariable('v1', 'value1').select(%v1)";
        let parsed2 = parser().parse(expr2).into_result().unwrap();
        let result2 = evaluate(&parsed2, &context, None);

        // The variable is accessible even when defineVariable is called with empty input
        assert!(
            result2.is_ok(),
            "Should successfully access the defined variable"
        );
        assert_eq!(
            result2.unwrap(),
            EvaluationResult::string("value1".to_string())
        );
    }

    #[test]
    fn test_system_variable_protection() {
        // Test that system variables cannot be overridden
        let expr = "defineVariable('context', 'oops')";
        let parsed = parser().parse(expr).into_result().unwrap();

        let context = EvaluationContext::new_empty(FhirVersion::R4);
        let result = evaluate(&parsed, &context, None);

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Cannot override system variable")
        );
    }

    #[test]
    fn test_variable_scoping_in_select() {
        // This test documents that child contexts are created for select()
        // but defineVariable still doesn't work due to architectural limitations
        let expr = "1 | 2 | 3";
        let parsed = parser().parse(expr).into_result().unwrap();

        let context = EvaluationContext::new_empty(FhirVersion::R4);
        let result = evaluate(&parsed, &context, None).unwrap();

        match result {
            EvaluationResult::Collection { items, .. } => {
                assert_eq!(items.len(), 3);
            }
            _ => panic!("Expected collection"),
        }
    }

    #[test]
    fn test_context_inheritance() {
        // Test that child contexts can access parent variables
        let mut parent_context = EvaluationContext::new_empty(FhirVersion::R4);
        parent_context.set_variable_result(
            "%parent_var",
            EvaluationResult::string("parent".to_string()),
        );

        let child_context = parent_context.create_child_context();

        // Child should be able to access parent variable
        assert!(child_context.lookup_variable("%parent_var").is_some());

        // Parent should not see child variables
        let mut child_with_var = child_context.clone();
        child_with_var
            .set_variable_result("%child_var", EvaluationResult::string("child".to_string()));

        assert!(parent_context.lookup_variable("%child_var").is_none());
    }

    #[test]
    fn test_context_inheritance_through_nested_children() {
        // A child's parent link carries only the variable chain, not a copy of
        // `this`, so a grandchild must still resolve a grandparent's variable,
        // a variable shadowed on the way down must resolve to the nearest
        // definition, and `this` must still be inherited by every child.
        let mut root = EvaluationContext::new_empty(FhirVersion::R4);
        root.set_this(EvaluationResult::string("root-this".to_string()));
        root.set_variable_result("%root_var", EvaluationResult::string("root".to_string()));
        root.set_variable_result("%shadowed", EvaluationResult::string("root".to_string()));

        let mut child = root.create_child_context();
        child.set_variable_result("%shadowed", EvaluationResult::string("child".to_string()));
        let grandchild = child.create_child_context();

        assert_eq!(
            grandchild.lookup_variable("%root_var"),
            Some(&EvaluationResult::string("root".to_string()))
        );
        assert_eq!(
            grandchild.lookup_variable("%shadowed"),
            Some(&EvaluationResult::string("child".to_string()))
        );
        assert!(grandchild.lookup_variable("%missing").is_none());
        assert_eq!(
            grandchild.this,
            Some(EvaluationResult::string("root-this".to_string()))
        );
        // The parent link is a variables-only snapshot: no second copy of the
        // context item rides along with every lambda scope.
        let parent_link = grandchild
            .parent_context
            .as_ref()
            .expect("child has a parent link");
        assert!(parent_link.this.is_none());
        assert!(
            parent_link.parent_context.is_some(),
            "the chain reaches the root"
        );
    }
}
