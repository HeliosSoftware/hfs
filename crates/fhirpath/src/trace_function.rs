use crate::evaluator::{EvaluationContext, evaluate};
use crate::parser::Expression;
use helios_fhirpath_support::{EvaluationError, EvaluationResult};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parser;
    use chumsky::Parser;

    #[test]
    fn test_trace_function() {
        let context = EvaluationContext::new_empty_with_default_version();

        // Basic test cases
        let trace_cases = vec![
            // Basic trace with literal
            ("1.trace('test')", EvaluationResult::integer(1)),
            // Trace with a chain
            (
                "1.trace('first').trace('second')",
                EvaluationResult::integer(1),
            ),
            // Trace with a collection
            (
                "(1 | 2 | 3).trace('collection')",
                EvaluationResult::Collection {
                    items: vec![
                        EvaluationResult::integer(1),
                        EvaluationResult::integer(2),
                        EvaluationResult::integer(3),
                    ],
                    has_undefined_order: true, // Union operator implies undefined order
                    type_info: None,
                },
            ),
            // Trace with a projection (second parameter)
            (
                "(1 | 2 | 3).trace('projection', $this + 1)",
                EvaluationResult::Collection {
                    items: vec![
                        EvaluationResult::integer(1),
                        EvaluationResult::integer(2),
                        EvaluationResult::integer(3),
                    ],
                    has_undefined_order: true, // Input collection from union has undefined order
                    type_info: None,
                },
            ),
        ];

        // Run test cases
        for (expr, expected) in trace_cases {
            println!("Testing: {}", expr);

            let parsed = parser().parse(expr).into_result().unwrap();
            let result = evaluate(&parsed, &context, None).unwrap();

            assert_eq!(result, expected, "Expression: {}", expr);
        }

        // Test error cases
        let error_cases = vec![
            // Missing the required name parameter
            "1.trace()",
            // Name parameter is not a string
            "1.trace(123)",
        ];

        for expr in error_cases {
            println!("Testing error case: {}", expr);

            let parsed = parser().parse(expr).into_result().unwrap();
            let result = evaluate(&parsed, &context, None);

            assert!(result.is_err(), "Expected error for expression: {}", expr);
        }
    }
}

/// Implements the trace() function for FHIRPath expressions
///
/// The trace() function allows for debugging FHIRPath expressions by logging
/// the current collection (or a projection of it) and returning the input unchanged.
///
/// # Syntax
/// `trace(name [, projection])`
///
/// # Parameters
/// * `name` - A string label used to identify the traced output
/// * `projection` - (Optional) An expression to evaluate against each item in the collection
///
/// # Returns
/// The original input collection, unmodified (side effect is collecting trace output)
pub fn trace_function(
    invocation_base: &EvaluationResult,
    name: &str,
    projection_expr: Option<&Expression>,
    context: &EvaluationContext,
) -> Result<EvaluationResult, EvaluationError> {
    // Determine what to trace: either the input collection or a projection of it
    let trace_value = if let Some(projection) = projection_expr {
        // When projection is provided, evaluate it on each item and collect results
        let (items, base_was_unordered) = match invocation_base {
            EvaluationResult::Collection {
                items,
                has_undefined_order,
                ..
            } => (items.clone(), *has_undefined_order), // Destructure
            EvaluationResult::Empty => (Vec::new(), false),
            single_item => (vec![single_item.clone()], false),
        };

        let mut projected_items = Vec::new();
        let mut projected_is_unordered = base_was_unordered; // Start with base order status
        for item in items {
            // Iterate over destructured items
            // Evaluate the projection expression with the current item as context
            let result = evaluate(projection, context, Some(&item))?;
            match result {
                EvaluationResult::Collection {
                    items: inner,
                    has_undefined_order,
                    ..
                } => {
                    // Destructure
                    projected_items.extend(inner);
                    if has_undefined_order {
                        projected_is_unordered = true;
                    }
                }
                EvaluationResult::Empty => {} // Skip empty results
                single_result => projected_items.push(single_result),
            }
        }

        if projected_items.is_empty() {
            EvaluationResult::Empty
        } else if projected_items.len() == 1 {
            projected_items[0].clone()
        } else {
            // The order of projected items depends on the input and the projection itself.
            // If the base was unordered, or if any projection resulted in an unordered collection,
            // the result is unordered.
            EvaluationResult::Collection {
                items: projected_items,
                has_undefined_order: projected_is_unordered,
                type_info: None,
            }
        }
    } else {
        // When no projection is provided, trace the input directly
        invocation_base.clone()
    };

    // Store the trace output in the context using Mutex
    context
        .trace_outputs
        .lock()
        .push((name.to_string(), trace_value));

    // Return the original input collection unchanged
    Ok(invocation_base.clone())
}
