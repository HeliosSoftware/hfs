use helios_fhirpath::{EvaluationContext, evaluate_expression};
use helios_fhirpath_support::EvaluationResult;
use rust_decimal::Decimal;

pub fn run_fhir_test(
    expression: &str,
    context: &EvaluationContext,
    expected: &[EvaluationResult],
    is_predicate_test: bool,
) -> Result<(), String> {
    // Evaluate the expression
    let eval_result = evaluate_expression(expression, context)
        .map_err(|e| format!("Evaluation error: {:?}", e))?;

    // If this is a predicate test, coerce the result according to FHIRPath spec 5.1.1
    let final_eval_result_for_comparison = if is_predicate_test {
        match eval_result.count() {
            0 => EvaluationResult::Empty, // Empty collection or Empty item
            1 => {
                // Single item. If it's a Boolean, use its value. Otherwise, it becomes true.
                let single_item_value = if let EvaluationResult::Collection {
                    items: ref c_items,
                    ..
                } = eval_result
                {
                    // This case handles a collection with one item.
                    // We need to get the item itself to check if it's a boolean.
                    c_items[0].clone()
                } else {
                    // This case handles a single, non-collection item (e.g. String, Integer).
                    eval_result.clone()
                };

                if let EvaluationResult::Boolean(b_val, None, None) = single_item_value {
                    EvaluationResult::Boolean(b_val, None, None) // Preserve original boolean value
                } else {
                    EvaluationResult::Boolean(true, None, None) // Non-boolean single item becomes true in boolean context
                }
            }
            _ => {
                // count > 1
                return Err(format!(
                    "Predicate test expression resulted in a collection with {} items, evaluation cannot proceed according to FHIRPath spec 5.1.1: {:?}",
                    eval_result.count(),
                    eval_result
                ));
            }
        }
    } else {
        eval_result
    };

    // Convert the (potentially coerced) result to a vec for comparison
    let result_vec = match &final_eval_result_for_comparison {
        EvaluationResult::Collection { items, .. } => items.clone(), // Destructure
        EvaluationResult::Empty => Vec::new(), // Empty result means an empty list for comparison
        single_item => vec![single_item.clone()], // Single item becomes a list with one item
    };

    // Special case: If there are no expected results, we just verify execution completed
    if expected.is_empty() {
        return Ok(());
    }

    // Check if result matches expected
    if result_vec.len() != expected.len() {
        return Err(format!(
            "Expected {} results, got {}: {:?} vs {:?}",
            expected.len(),
            result_vec.len(),
            expected,
            result_vec
        ));
    }

    // Check each result value to see if it matches expected
    compare_results(&result_vec, expected)
}

fn compare_results(
    actual: &[EvaluationResult],
    expected: &[EvaluationResult],
) -> Result<(), String> {
    for (i, (actual, expected)) in actual.iter().zip(expected.iter()).enumerate() {
        match (actual, expected) {
            (EvaluationResult::Boolean(a, _, _), EvaluationResult::Boolean(b, _, _)) => {
                if a != b {
                    return Err(format!(
                        "Boolean result {} doesn't match: expected {:?}, got {:?}",
                        i, b, a
                    ));
                }
            }
            (EvaluationResult::Integer(a, _, _), EvaluationResult::Integer(b, _, _)) => {
                if a != b {
                    return Err(format!(
                        "Integer result {} doesn't match: expected {:?}, got {:?}",
                        i, b, a
                    ));
                }
            }
            (EvaluationResult::String(a, _, _), EvaluationResult::String(b, _, _)) => {
                if a != b {
                    return Err(format!(
                        "String result {} doesn't match: expected {:?}, got {:?}",
                        i, b, a
                    ));
                }
            }
            (EvaluationResult::Decimal(a, _, _), EvaluationResult::Decimal(b, _, _)) => {
                if a != b {
                    return Err(format!(
                        "Decimal result {} doesn't match: expected {} ({}), got {} ({})",
                        i, b, b, a, a
                    ));
                }
            }
            (
                EvaluationResult::Quantity(a_val, a_unit, _, _),
                EvaluationResult::Quantity(b_val, b_unit, _, _),
            ) => {
                if a_val != b_val || a_unit != b_unit {
                    return Err(format!(
                        "Quantity result {} doesn't match: expected value {:?} unit {:?}, got value {:?} unit {:?}",
                        i, b_val, b_unit, a_val, a_unit
                    ));
                }
            }
            // Date types which are currently stored as strings
            (EvaluationResult::Date(a, _, _), EvaluationResult::Date(b, _, _)) => {
                if a != b {
                    return Err(format!(
                        "Date result {} doesn't match: expected {:?}, got {:?}",
                        i, b, a
                    ));
                }
            }
            (EvaluationResult::DateTime(a, _, _), EvaluationResult::DateTime(b, _, _)) => {
                if a != b {
                    return Err(format!(
                        "DateTime result {} doesn't match: expected {:?}, got {:?}",
                        i, b, a
                    ));
                }
            }
            (EvaluationResult::Time(a, _, _), EvaluationResult::Time(b, _, _)) => {
                if a != b {
                    return Err(format!(
                        "Time result {} doesn't match: expected {:?}, got {:?}",
                        i, b, a
                    ));
                }
            }
            // Special case for FHIR types that are stored differently but might be equivalent
            // String vs. Code compatibility (since code is stored as String in our implementation)
            (EvaluationResult::String(a, _, _), EvaluationResult::Date(b, _, _)) => {
                // A String can be equal to a Date in certain contexts
                if a != b {
                    return Err(format!(
                        "String/Date mismatch {} doesn't match: expected Date {:?}, got String {:?}",
                        i, b, a
                    ));
                }
            }
            (EvaluationResult::Date(a, _, _), EvaluationResult::String(b, _, _)) => {
                // A Date can be equal to a String in certain contexts
                if a != b {
                    return Err(format!(
                        "Date/String mismatch {} doesn't match: expected String {:?}, got Date {:?}",
                        i, b, a
                    ));
                }
            }
            // Add more cross-type compatibility cases here
            // Add more cases as needed for other types
            _ => {
                // Different types or unhandled types
                if actual.type_name() != expected.type_name() {
                    return Err(format!(
                        "Result type {} doesn't match: expected {:?} ({}), got {:?} ({})",
                        i,
                        expected,
                        expected.type_name(),
                        actual,
                        actual.type_name()
                    ));
                } else {
                    return Err(format!(
                        "Unsupported result comparison for type {}: expected {:?}, got {:?}",
                        actual.type_name(),
                        expected,
                        actual
                    ));
                }
            }
        }
    }

    Ok(())
}

// Function to parse expected output value with version-specific date handling
pub fn parse_output_value(
    output_type: &str,
    output_value: &str,
    fhir_version: &str,
) -> Result<EvaluationResult, String> {
    match output_type {
        "boolean" => match output_value {
            "true" => Ok(EvaluationResult::Boolean(true, None, None)),
            "false" => Ok(EvaluationResult::Boolean(false, None, None)),
            _ => Err(format!("Invalid boolean value: {}", output_value)),
        },
        "integer" => output_value
            .parse::<i64>()
            .map(EvaluationResult::integer)
            .map_err(|_| format!("Invalid integer value: {}", output_value)),
        "string" => Ok(EvaluationResult::String(
            output_value.to_string(),
            None,
            None,
        )),
        "date" => {
            // Handle R5's @ prefix for dates
            let date_str = if fhir_version == "R5" && output_value.starts_with('@') {
                &output_value[1..]
            } else {
                output_value
            };
            Ok(EvaluationResult::Date(date_str.to_string(), None, None))
        }
        "dateTime" => Ok(EvaluationResult::DateTime(
            output_value.to_string(),
            None,
            None,
        )),
        "time" => Ok(EvaluationResult::Time(output_value.to_string(), None, None)),
        "code" => Ok(EvaluationResult::String(
            output_value.to_string(),
            None,
            None,
        )),
        "decimal" => output_value
            .parse::<Decimal>()
            .map(EvaluationResult::decimal)
            .map_err(|_| format!("Invalid decimal value: {}", output_value)),
        "Quantity" => parse_quantity(output_value),
        _ => Err(format!("Unsupported output type: {}", output_type)),
    }
}

fn parse_quantity(output_value: &str) -> Result<EvaluationResult, String> {
    // Parse "value 'unit'" format, e.g., "1 '1'" or "10.5 'mg'"
    let parts: Vec<&str> = output_value.splitn(2, ' ').collect();
    if parts.len() == 2 {
        let value_str = parts[0];
        let unit_str_quoted = parts[1];
        if unit_str_quoted.starts_with('\'')
            && unit_str_quoted.ends_with('\'')
            && unit_str_quoted.len() >= 2
        {
            let unit_str = &unit_str_quoted[1..unit_str_quoted.len() - 1];
            match value_str.parse::<Decimal>() {
                Ok(decimal_val) => Ok(EvaluationResult::Quantity(
                    decimal_val,
                    unit_str.to_string(),
                    None,
                    None,
                )),
                Err(_) => Err(format!(
                    "Invalid decimal value for Quantity: {}",
                    output_value
                )),
            }
        } else {
            Err(format!(
                "Invalid unit format for Quantity (expected 'unit'): {}",
                output_value
            ))
        }
    } else {
        Err(format!(
            "Invalid Quantity format (expected \"value 'unit'\"): {}",
            output_value
        ))
    }
}
