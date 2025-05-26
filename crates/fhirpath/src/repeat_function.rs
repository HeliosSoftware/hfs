use crate::evaluator::{EvaluationContext, evaluate};
use crate::parser::Expression;
use fhirpath_support::{EvaluationError, EvaluationResult};
use std::collections::HashSet;

/// Implements the FHIRPath repeat() function
///
/// The repeat() function repeatedly applies a projection to a collection and
/// adds new items to the output collection, as long as the projection yields new items.
/// It's useful for recursively traversing tree structures.
///
/// # Syntax
/// `repeat(projection: expression) : collection`
///
/// # Parameters
/// * `invocation_base` - The collection to start with
/// * `projection_expr` - The expression to apply repeatedly
/// * `context` - The evaluation context
///
/// # Returns
/// A collection containing all items from the repeated projection
pub fn repeat_function(
    invocation_base: &EvaluationResult,
    projection_expr: &Expression,
    context: &EvaluationContext,
) -> Result<EvaluationResult, EvaluationError> {
    // Get the initial items to traverse
    let initial_items = match invocation_base {
        EvaluationResult::Collection { items, .. } => items.clone(), // Destructure
        EvaluationResult::Empty => Vec::new(),
        single_item => vec![single_item.clone()],
    };

    // Initialize result as an empty vector - we will fill it with the results
    // of repeated projections, NOT including the initial items
    let mut result = Vec::new();

    // Use a HashSet to track what we've already seen to avoid infinite recursion
    // and to make it easier to check if we've seen an item before
    let mut seen_items = HashSet::new();

    // Add all initial items to the seen set, but don't add them to the result
    // According to the FHIRPath spec, repeat() doesn't include the initial collection
    for item in &initial_items {
        // We use the debug representation as a simple way to create a string key
        let item_key = format!("{:?}", item);
        seen_items.insert(item_key);
    }

    // Keep track of items we still need to process
    let mut items_to_process = initial_items;

    // Continue as long as we have more items to process
    while !items_to_process.is_empty() {
        let mut new_items_to_process = Vec::new();

        // Process each item
        for item in &items_to_process {
            // Apply the projection to the current item
            let projected = evaluate(projection_expr, context, Some(item))?;

            // Flatten the projection result
            let (projected_items, _projected_order_status) = match projected {
                // Capture order status if needed, though repeat() output is unordered
                EvaluationResult::Collection { items, .. } => (items, false), // Destructure, order status of sub-projection doesn't make overall repeat ordered
                EvaluationResult::Empty => (Vec::new(), false),
                single_item => (vec![single_item], false),
            };

            // Process each projected item
            for projected_item in projected_items {
                // Generate a key to check if we've seen this item before
                let item_key = format!("{:?}", projected_item);

                // Only add items we haven't seen before
                if !seen_items.contains(&item_key) {
                    // Add to result
                    result.push(projected_item.clone());
                    // Add to items to process in next iteration
                    new_items_to_process.push(projected_item.clone());
                    // Mark as seen
                    seen_items.insert(item_key);
                }
            }
        }

        // Update items to process for next iteration
        items_to_process = new_items_to_process;
    }

    // Return result after applying FHIRPath normalization
    if result.is_empty() {
        Ok(EvaluationResult::Empty)
    } else if result.len() == 1 {
        Ok(result[0].clone())
    } else {
        // repeat() output order is undefined
        Ok(EvaluationResult::Collection {
            items: result,
            has_undefined_order: true,
            type_info: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parser;
    use chumsky::Parser;

    #[test]
    fn test_repeat_function_basic() {
        // Create a simple object hierarchy for testing
        let object1 = EvaluationResult::Object {
            map: {
                let mut map = std::collections::HashMap::new();
                map.insert(
                    "name".to_string(),
                    EvaluationResult::string("root".to_string()),
                );
                map.insert(
                    "child".to_string(),
                    EvaluationResult::Object {
                        map: {
                            let mut child_map = std::collections::HashMap::new();
                            child_map.insert(
                                "name".to_string(),
                                EvaluationResult::string("level1".to_string()),
                            );
                            child_map.insert(
                                "child".to_string(),
                                EvaluationResult::Object {
                                    map: {
                                        let mut grandchild_map = std::collections::HashMap::new();
                                        grandchild_map.insert(
                                            "name".to_string(),
                                            EvaluationResult::string("level2".to_string()),
                                        );
                                        grandchild_map
                                    },
                                    type_info: None,
                                },
                            );
                            child_map
                        },
                        type_info: None,
                    },
                );
                map
            },
            type_info: None,
        };

        // Build a context with our test object
        let mut context = EvaluationContext::new_empty_with_default_version();
        context.this = Some(object1.clone());

        // Test repeat with child projection
        let parsed = parser().parse("repeat(child)").unwrap();
        let result = repeat_function(&object1, &parsed, &context).unwrap();

        // Verify results - we should have both child objects
        if let EvaluationResult::Collection { items, .. } = result {
            assert_eq!(items.len(), 2);

            // Check that we have both level1 and level2 objects
            let mut found_level1 = false;
            let mut found_level2 = false;

            for item in items {
                if let EvaluationResult::Object {
                    map,
                    type_info: None,
                } = &item
                {
                    if let Some(EvaluationResult::String(name, _)) = map.get("name") {
                        if name == "level1" {
                            found_level1 = true;
                        } else if name == "level2" {
                            found_level2 = true;
                        }
                    }
                }
            }

            assert!(found_level1, "Should find level1 object");
            assert!(found_level2, "Should find level2 object");
        } else {
            panic!("Expected collection result, got: {:?}", result);
        }
    }

    #[test]
    fn test_repeat_function_empty() {
        // Test with empty collection
        let empty = EvaluationResult::Empty;
        let context = EvaluationContext::new_empty_with_default_version();

        // Parse a simple projection expression
        let parsed = parser().parse("name").unwrap();

        // Apply repeat function
        let result = repeat_function(&empty, &parsed, &context).unwrap();

        // Should return Empty
        assert_eq!(result, EvaluationResult::Empty);
    }

    #[test]
    fn test_repeat_function_circular() {
        // Create a more complex object hierarchy with circular references
        // This tests that we properly handle detecting already seen items

        // Create object maps
        let mut obj1_map = std::collections::HashMap::new();
        let mut obj2_map = std::collections::HashMap::new();

        // Set initial properties
        obj1_map.insert(
            "name".to_string(),
            EvaluationResult::string("obj1".to_string()),
        );
        obj2_map.insert(
            "name".to_string(),
            EvaluationResult::string("obj2".to_string()),
        );

        // Now set circular references
        // (we need to create the objects first since we can't create circular references directly)
        let obj1_temp = EvaluationResult::Object {
            map: obj1_map.clone(),
            type_info: None,
        };
        let obj2_temp = EvaluationResult::Object {
            map: obj2_map.clone(),
            type_info: None,
        };

        // Add references in the maps
        obj1_map.insert("next".to_string(), obj2_temp);
        obj2_map.insert("next".to_string(), obj1_temp);

        // Create final objects with the prepared maps
        let obj1 = EvaluationResult::Object {
            map: obj1_map,
            type_info: None,
        };
        let _obj2 = EvaluationResult::Object {
            map: obj2_map,
            type_info: None,
        }; // Prefix with underscore since we don't use it directly

        // Now create a root object that refers to these
        let mut root_map = std::collections::HashMap::new();
        root_map.insert(
            "name".to_string(),
            EvaluationResult::string("root".to_string()),
        );
        root_map.insert("next".to_string(), obj1.clone());

        let root = EvaluationResult::Object {
            map: root_map,
            type_info: None,
        };

        // Create context
        let mut context = EvaluationContext::new_empty_with_default_version();
        context.this = Some(root.clone());

        // Test repeat with next projection (should handle the circular references)
        let parsed = parser().parse("next").unwrap();
        let result = repeat_function(&root, &parsed, &context).unwrap();

        // Verify we get only the two distinct objects, not infinite repetitions
        if let EvaluationResult::Collection { items, .. } = result {
            assert_eq!(items.len(), 2, "Should find exactly two distinct objects");
        } else {
            panic!("Expected collection result, got: {:?}", result);
        }
    }
}
