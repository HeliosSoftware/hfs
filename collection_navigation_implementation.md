# Collection Navigation Implementation Fix

This document provides a step-by-step implementation guide for fixing collection navigation and path traversal issues in the FHIRPath implementation.

## 1. Enhanced Collection Handling

### 1.1. Improve Collection Flattening and Navigation

First, we need to enhance the collection flattening and navigation in the evaluator:

```rust
// evaluator.rs

use fhirpath_support::{EvaluationResult, EvaluationError};
use std::collections::{HashMap, HashSet};

/// Flattens a collection and all nested collections recursively according to FHIRPath rules.
/// This function ensures that collections are properly flattened at all levels.
fn flatten_collections_recursive(result: &EvaluationResult) -> Vec<EvaluationResult> {
    let mut flattened = Vec::new();
    
    match result {
        EvaluationResult::Collection(items) => {
            // For each item in the collection, recursively flatten it
            for item in items {
                let nested_flattened = flatten_collections_recursive(item);
                flattened.extend(nested_flattened);
            }
        },
        EvaluationResult::Empty => {
            // Skip empty results as per FHIRPath rules
        },
        other => {
            // Add non-collection, non-empty items directly
            flattened.push(other.clone());
        }
    }
    
    flattened
}

/// Normalize a collection result according to FHIRPath singleton evaluation rules.
/// Returns Empty if collection is empty, the single item if len is 1, or Collection otherwise.
fn normalize_collection_result(items: Vec<EvaluationResult>) -> EvaluationResult {
    if items.is_empty() {
        EvaluationResult::Empty
    } else if items.len() == 1 {
        items[0].clone() // Take the single item
    } else {
        EvaluationResult::Collection(items)
    }
}

/// Normalizes path navigation access for proper context tracking
/// This is a key function for ensuring that collection operations like .skip() and .where()
/// properly maintain the context for subsequent property access
fn evaluate_path_navigation(
    base: &EvaluationResult, 
    property: &str,
    context: &EvaluationContext
) -> Result<EvaluationResult, EvaluationError> {
    match base {
        EvaluationResult::Collection(items) => {
            // For collections, process each item and collect results
            let mut results = Vec::new();
            
            for item in items {
                // Process each item in the collection
                let item_result = access_property(item, property)?;
                
                // Only include non-empty results
                if !matches!(item_result, EvaluationResult::Empty) {
                    // If the result is a collection, add its items to our results
                    if let EvaluationResult::Collection(nested_items) = &item_result {
                        results.extend(nested_items.clone());
                    } else {
                        // Otherwise add the single result
                        results.push(item_result);
                    }
                }
            }
            
            // Properly flatten and normalize the results
            if results.is_empty() {
                Ok(EvaluationResult::Empty)
            } else {
                Ok(EvaluationResult::Collection(results))
            }
        },
        EvaluationResult::Object(obj) => {
            // Direct property access on an object
            if let Some(value) = obj.get(property) {
                Ok(value.clone())
            } else {
                // Check for polymorphic field
                if is_choice_element(obj, property) {
                    Ok(access_polymorphic_element(obj, property))
                } else {
                    Ok(EvaluationResult::Empty)
                }
            }
        },
        EvaluationResult::Empty => {
            // Property access on Empty returns Empty
            Ok(EvaluationResult::Empty)
        },
        // Handle primitive types that might have extensions
        EvaluationResult::String(_) |
        EvaluationResult::Boolean(_) |
        EvaluationResult::Integer(_) |
        EvaluationResult::Decimal(_) |
        EvaluationResult::Date(_) |
        EvaluationResult::DateTime(_) |
        EvaluationResult::Time(_) |
        EvaluationResult::Quantity(_, _) => {
            // For primitive FHIR types, only allow access to id and extension
            if property == "id" || property == "extension" {
                // For now, return Empty for these properties
                // TODO: Implement proper extension support for primitives
                Ok(EvaluationResult::Empty)
            } else {
                // For other properties on primitives, return Empty
                Ok(EvaluationResult::Empty)
            }
        }
    }
}

/// Access a property on a result, handling objects, collections, and primitives
fn access_property(value: &EvaluationResult, property: &str) -> Result<EvaluationResult, EvaluationError> {
    match value {
        EvaluationResult::Object(obj) => {
            // Direct property access on an object
            if let Some(property_value) = obj.get(property) {
                Ok(property_value.clone())
            } else {
                // Check for polymorphic field
                if is_choice_element(obj, property) {
                    Ok(access_polymorphic_element(obj, property))
                } else {
                    Ok(EvaluationResult::Empty)
                }
            }
        },
        EvaluationResult::Collection(items) => {
            // For collections, apply property access to each item and collect results
            let mut results = Vec::new();
            
            for item in items {
                let item_result = access_property(item, property)?;
                
                // Only include non-empty results
                if !matches!(item_result, EvaluationResult::Empty) {
                    // If the result is a collection, add its items to our results
                    if let EvaluationResult::Collection(nested_items) = &item_result {
                        results.extend(nested_items.clone());
                    } else {
                        // Otherwise add the single result
                        results.push(item_result);
                    }
                }
            }
            
            // Properly flatten and normalize the results
            if results.is_empty() {
                Ok(EvaluationResult::Empty)
            } else {
                Ok(EvaluationResult::Collection(results))
            }
        },
        // Handle primitive types that might have extensions
        EvaluationResult::String(_) |
        EvaluationResult::Boolean(_) |
        EvaluationResult::Integer(_) |
        EvaluationResult::Decimal(_) |
        EvaluationResult::Date(_) |
        EvaluationResult::DateTime(_) |
        EvaluationResult::Time(_) |
        EvaluationResult::Quantity(_, _) => {
            // For primitive FHIR types, only allow access to id and extension
            if property == "id" || property == "extension" {
                // For now, return Empty for these properties
                // TODO: Implement proper extension support for primitives
                Ok(EvaluationResult::Empty)
            } else {
                // For other properties on primitives, return Empty
                Ok(EvaluationResult::Empty)
            }
        },
        EvaluationResult::Empty => {
            // Property access on Empty returns Empty
            Ok(EvaluationResult::Empty)
        }
    }
}
```

### 1.2. Enhanced Invocation Evaluation

Next, update the `evaluate_invocation` function to properly handle collections and path navigation:

```rust
/// Evaluates an invocation on a value
fn evaluate_invocation(
    invocation_base: &EvaluationResult,
    invocation: &Invocation,
    context: &EvaluationContext,
) -> Result<EvaluationResult, EvaluationError> {
    match invocation {
        Invocation::Member(name) => {
            // Handle special cases like true/false literals
            if name == "true" && matches!(invocation_base, EvaluationResult::Empty) {
                return Ok(EvaluationResult::Boolean(true));
            } else if name == "false" && matches!(invocation_base, EvaluationResult::Empty) {
                return Ok(EvaluationResult::Boolean(false));
            }
            
            // Use our enhanced path navigation function for property access
            evaluate_path_navigation(invocation_base, name, context)
        },
        Invocation::Function(name, args_exprs) => {
            // Evaluate function arguments
            let mut args = Vec::new();
            for arg_expr in args_exprs {
                if is_lambda_expression(arg_expr) {
                    // Don't evaluate lambda expressions - they are passed as-is
                    continue;
                }
                
                // Evaluate non-lambda arguments
                args.push(evaluate(arg_expr, context, None)?);
            }
            
            // Handle various function types
            match name.as_str() {
                // Collection functions with special lambda handling
                "where" if !args_exprs.is_empty() => {
                    let criteria_expr = &args_exprs[0];
                    evaluate_where(invocation_base, criteria_expr, context)
                },
                "select" if !args_exprs.is_empty() => {
                    let projection_expr = &args_exprs[0];
                    evaluate_select(invocation_base, projection_expr, context)
                },
                "all" if !args_exprs.is_empty() => {
                    let criteria_expr = &args_exprs[0];
                    evaluate_all_with_criteria(invocation_base, criteria_expr, context)
                },
                "exists" if !args_exprs.is_empty() => {
                    let criteria_expr = &args_exprs[0];
                    evaluate_exists_with_criteria(invocation_base, criteria_expr, context)
                },
                
                // Simple collection functions
                "empty" => evaluate_empty(invocation_base),
                "exists" => evaluate_exists(invocation_base),
                "count" => evaluate_count(invocation_base),
                "first" => evaluate_first(invocation_base),
                "last" => evaluate_last(invocation_base),
                "tail" => evaluate_tail(invocation_base),
                "skip" if args.len() == 1 => evaluate_skip(invocation_base, &args[0]),
                "take" if args.len() == 1 => evaluate_take(invocation_base, &args[0]),
                "single" => evaluate_single(invocation_base),
                
                // Advanced functions delegated to specialized modules
                "repeat" if !args_exprs.is_empty() => {
                    let projection_expr = &args_exprs[0];
                    crate::repeat_function::repeat_function(invocation_base, projection_expr, context)
                },
                "aggregate" if !args_exprs.is_empty() => {
                    let aggregator_expr = &args_exprs[0];
                    let init_value = args_exprs.get(1).map(|expr| evaluate(expr, context, None)).transpose()?;
                    crate::aggregate_function::aggregate_function(invocation_base, aggregator_expr, init_value.as_ref(), context)
                },
                "trace" => {
                    if args_exprs.len() == 1 {
                        // Simple trace with just a name
                        crate::trace_function::trace_function(invocation_base, &args[0], None, context)
                    } else if args_exprs.len() == 2 {
                        // Trace with projection
                        let projection_expr = &args_exprs[1];
                        crate::trace_function::trace_function(invocation_base, &args[0], Some(projection_expr), context)
                    } else {
                        Err(EvaluationError::InvalidArgument(
                            format!("trace() requires 1 or 2 arguments, got {}", args_exprs.len())
                        ))
                    }
                },
                
                // Type functions
                "ofType" if args_exprs.len() == 1 => {
                    // Handle ofType with type name
                    let type_spec_opt = extract_type_specifier_from_expression(&args_exprs[0])?;
                    if let Some(type_spec) = type_spec_opt {
                        crate::resource_type::of_type(invocation_base, &type_spec)
                    } else {
                        Err(EvaluationError::InvalidArgument(
                            format!("Invalid type specifier for ofType: {:?}", args_exprs[0])
                        ))
                    }
                },
                "type" => {
                    // Call the type function
                    crate::type_function::type_function(invocation_base)
                },
                
                // Other functions
                _ => {
                    // Handle other common functions
                    evaluate_common_function(name, invocation_base, &args)
                }
            }
        },
        Invocation::This => {
            // Return the current context
            if let Some(item) = context.this.as_ref() {
                Ok(item.clone())
            } else {
                Ok(invocation_base.clone())
            }
        }
    }
}
```

### 1.3. Enhanced Implementation of Collection Functions

Implement improved versions of key collection functions that properly preserve context:

```rust
/// Evaluate the empty() function
fn evaluate_empty(value: &EvaluationResult) -> Result<EvaluationResult, EvaluationError> {
    let is_empty = match value {
        EvaluationResult::Empty => true,
        EvaluationResult::Collection(items) => items.is_empty(),
        _ => false, // Single non-Empty value is not empty
    };
    
    Ok(EvaluationResult::Boolean(is_empty))
}

/// Evaluate the exists() function
fn evaluate_exists(value: &EvaluationResult) -> Result<EvaluationResult, EvaluationError> {
    let exists = match value {
        EvaluationResult::Empty => false,
        EvaluationResult::Collection(items) => !items.is_empty(),
        _ => true, // Single non-Empty value exists
    };
    
    Ok(EvaluationResult::Boolean(exists))
}

/// Evaluate exists() with criteria
fn evaluate_exists_with_criteria(
    collection: &EvaluationResult, 
    criteria_expr: &Expression,
    context: &EvaluationContext
) -> Result<EvaluationResult, EvaluationError> {
    match collection {
        EvaluationResult::Collection(items) => {
            // Check if any item satisfies the criteria
            for item in items {
                // Create a context with this item
                let mut item_context = context.clone();
                item_context.set_this(item.clone());
                
                // Evaluate the criteria
                let criteria_result = evaluate(criteria_expr, &item_context, Some(item))?;
                
                // Check if the criteria is true
                if criteria_result.to_boolean_for_logic()?.as_boolean().unwrap_or(false) {
                    return Ok(EvaluationResult::Boolean(true));
                }
            }
            
            // No item satisfied the criteria
            Ok(EvaluationResult::Boolean(false))
        },
        EvaluationResult::Empty => {
            // Empty collection never has any items that satisfy criteria
            Ok(EvaluationResult::Boolean(false))
        },
        _ => {
            // For a single item, evaluate the criteria on it
            let mut item_context = context.clone();
            item_context.set_this(collection.clone());
            
            // Evaluate the criteria
            let criteria_result = evaluate(criteria_expr, &item_context, Some(collection))?;
            
            // Check if the criteria is true
            let result = criteria_result.to_boolean_for_logic()?.as_boolean().unwrap_or(false);
            Ok(EvaluationResult::Boolean(result))
        }
    }
}

/// Evaluate the count() function
fn evaluate_count(value: &EvaluationResult) -> Result<EvaluationResult, EvaluationError> {
    let count = match value {
        EvaluationResult::Empty => 0,
        EvaluationResult::Collection(items) => items.len() as i64,
        _ => 1, // Single non-Empty value has count 1
    };
    
    Ok(EvaluationResult::Integer(count))
}

/// Evaluate the first() function
fn evaluate_first(value: &EvaluationResult) -> Result<EvaluationResult, EvaluationError> {
    match value {
        EvaluationResult::Collection(items) if !items.is_empty() => {
            Ok(items[0].clone())
        },
        EvaluationResult::Empty => Ok(EvaluationResult::Empty),
        EvaluationResult::Collection(_) => Ok(EvaluationResult::Empty), // Empty collection
        _ => Ok(value.clone()), // Single item is its own first element
    }
}

/// Evaluate the last() function
fn evaluate_last(value: &EvaluationResult) -> Result<EvaluationResult, EvaluationError> {
    match value {
        EvaluationResult::Collection(items) if !items.is_empty() => {
            Ok(items.last().unwrap().clone())
        },
        EvaluationResult::Empty => Ok(EvaluationResult::Empty),
        EvaluationResult::Collection(_) => Ok(EvaluationResult::Empty), // Empty collection
        _ => Ok(value.clone()), // Single item is its own last element
    }
}

/// Evaluate the tail() function
fn evaluate_tail(value: &EvaluationResult) -> Result<EvaluationResult, EvaluationError> {
    match value {
        EvaluationResult::Collection(items) if items.len() > 1 => {
            let tail_items = items[1..].to_vec();
            Ok(EvaluationResult::Collection(tail_items))
        },
        EvaluationResult::Collection(_) => Ok(EvaluationResult::Empty), // Collection with 0-1 items
        EvaluationResult::Empty => Ok(EvaluationResult::Empty),
        _ => Ok(EvaluationResult::Empty), // Single item has empty tail
    }
}

/// Evaluate the skip() function
fn evaluate_skip(value: &EvaluationResult, count: &EvaluationResult) -> Result<EvaluationResult, EvaluationError> {
    // Convert count to integer
    let num = match count {
        EvaluationResult::Integer(n) => *n as usize,
        EvaluationResult::Decimal(d) => d.to_usize().unwrap_or(0),
        _ => return Err(EvaluationError::InvalidArgument(
            format!("skip() requires an integer argument, got {}", count.type_name())
        )),
    };
    
    match value {
        EvaluationResult::Collection(items) => {
            if num >= items.len() {
                // Skip all items
                Ok(EvaluationResult::Empty)
            } else {
                // Skip the first 'num' items
                let skipped_items = items[num..].to_vec();
                Ok(EvaluationResult::Collection(skipped_items))
            }
        },
        EvaluationResult::Empty => Ok(EvaluationResult::Empty),
        _ => {
            // Single item
            if num == 0 {
                // Skip 0 items, keep the single item
                Ok(value.clone())
            } else {
                // Skip the single item
                Ok(EvaluationResult::Empty)
            }
        }
    }
}

/// Evaluate the take() function
fn evaluate_take(value: &EvaluationResult, count: &EvaluationResult) -> Result<EvaluationResult, EvaluationError> {
    // Convert count to integer
    let num = match count {
        EvaluationResult::Integer(n) => *n as usize,
        EvaluationResult::Decimal(d) => d.to_usize().unwrap_or(0),
        _ => return Err(EvaluationError::InvalidArgument(
            format!("take() requires an integer argument, got {}", count.type_name())
        )),
    };
    
    match value {
        EvaluationResult::Collection(items) => {
            if num == 0 {
                // Take 0 items
                Ok(EvaluationResult::Empty)
            } else if num >= items.len() {
                // Take all items
                Ok(value.clone())
            } else {
                // Take the first 'num' items
                let taken_items = items[..num].to_vec();
                Ok(EvaluationResult::Collection(taken_items))
            }
        },
        EvaluationResult::Empty => Ok(EvaluationResult::Empty),
        _ => {
            // Single item
            if num == 0 {
                // Take 0 items
                Ok(EvaluationResult::Empty)
            } else {
                // Take the single item
                Ok(value.clone())
            }
        }
    }
}

/// Evaluate the single() function
fn evaluate_single(value: &EvaluationResult) -> Result<EvaluationResult, EvaluationError> {
    match value {
        EvaluationResult::Collection(items) => {
            if items.len() == 1 {
                // Return the single item
                Ok(items[0].clone())
            } else if items.is_empty() {
                // Empty collection
                Ok(EvaluationResult::Empty)
            } else {
                // Multiple items
                Err(EvaluationError::MultipleItems(
                    format!("single() called on collection with {} items", items.len())
                ))
            }
        },
        EvaluationResult::Empty => Ok(EvaluationResult::Empty),
        _ => Ok(value.clone()), // Single item is already a singleton
    }
}
```

### 1.4. Enhanced Implementation of Where and Select Functions

Implement improved versions of the `where()` and `select()` functions:

```rust
/// Evaluate the where() function, which filters a collection based on a criteria
fn evaluate_where(
    collection: &EvaluationResult, 
    criteria_expr: &Expression,
    context: &EvaluationContext
) -> Result<EvaluationResult, EvaluationError> {
    match collection {
        EvaluationResult::Collection(items) => {
            // Filter items based on criteria
            let mut filtered_items = Vec::new();
            
            for item in items {
                // Create a context with this item
                let mut item_context = context.clone();
                item_context.set_this(item.clone());
                
                // Evaluate the criteria
                let criteria_result = evaluate(criteria_expr, &item_context, Some(item))?;
                
                // Check if the criteria is true
                if criteria_result.to_boolean_for_logic()?.as_boolean().unwrap_or(false) {
                    filtered_items.push(item.clone());
                }
            }
            
            // Return the filtered collection
            if filtered_items.is_empty() {
                Ok(EvaluationResult::Empty)
            } else {
                Ok(EvaluationResult::Collection(filtered_items))
            }
        },
        EvaluationResult::Empty => {
            // Empty collection results in empty result
            Ok(EvaluationResult::Empty)
        },
        _ => {
            // For a single item, evaluate the criteria on it
            let mut item_context = context.clone();
            item_context.set_this(collection.clone());
            
            // Evaluate the criteria
            let criteria_result = evaluate(criteria_expr, &item_context, Some(collection))?;
            
            // Check if the criteria is true
            if criteria_result.to_boolean_for_logic()?.as_boolean().unwrap_or(false) {
                Ok(collection.clone())
            } else {
                Ok(EvaluationResult::Empty)
            }
        }
    }
}

/// Evaluate the select() function, which projects a collection using an expression
fn evaluate_select(
    collection: &EvaluationResult, 
    projection_expr: &Expression,
    context: &EvaluationContext
) -> Result<EvaluationResult, EvaluationError> {
    match collection {
        EvaluationResult::Collection(items) => {
            // Project items using the projection expression
            let mut projected_items = Vec::new();
            
            for item in items {
                // Create a context with this item
                let mut item_context = context.clone();
                item_context.set_this(item.clone());
                
                // Evaluate the projection expression
                let projection_result = evaluate(projection_expr, &item_context, Some(item))?;
                
                // Add the projection result to the output
                match projection_result {
                    EvaluationResult::Empty => {
                        // Skip empty results
                    },
                    EvaluationResult::Collection(nested_items) => {
                        // Add all items from the nested collection
                        projected_items.extend(nested_items);
                    },
                    _ => {
                        // Add the single result
                        projected_items.push(projection_result);
                    }
                }
            }
            
            // Return the projected collection
            if projected_items.is_empty() {
                Ok(EvaluationResult::Empty)
            } else {
                Ok(EvaluationResult::Collection(projected_items))
            }
        },
        EvaluationResult::Empty => {
            // Empty collection results in empty result
            Ok(EvaluationResult::Empty)
        },
        _ => {
            // For a single item, evaluate the projection expression on it
            let mut item_context = context.clone();
            item_context.set_this(collection.clone());
            
            // Evaluate the projection expression
            let projection_result = evaluate(projection_expr, &item_context, Some(collection))?;
            
            // Return the projection result
            Ok(projection_result)
        }
    }
}
```

## 2. Tree Navigation Functions

### 2.1. Implement Children and Descendants Functions

Add improved implementations of the `children()` and `descendants()` functions:

```rust
/// Evaluate the children() function, which returns all immediate child elements
fn evaluate_children(value: &EvaluationResult) -> Result<EvaluationResult, EvaluationError> {
    match value {
        EvaluationResult::Object(obj) => {
            // Collect all values from the object
            let mut children = Vec::new();
            
            for (_, child) in obj {
                // Only include non-empty values
                if !matches!(child, EvaluationResult::Empty) {
                    children.push(child.clone());
                }
            }
            
            // Return the collection of children
            if children.is_empty() {
                Ok(EvaluationResult::Empty)
            } else {
                Ok(EvaluationResult::Collection(children))
            }
        },
        EvaluationResult::Collection(items) => {
            // For collections, apply children() to each item and collect results
            let mut all_children = Vec::new();
            
            for item in items {
                let item_children = evaluate_children(item)?;
                
                match item_children {
                    EvaluationResult::Empty => {
                        // Skip empty results
                    },
                    EvaluationResult::Collection(children) => {
                        // Add all children from the nested collection
                        all_children.extend(children);
                    },
                    _ => {
                        // Add the single child
                        all_children.push(item_children);
                    }
                }
            }
            
            // Return the combined collection of children
            if all_children.is_empty() {
                Ok(EvaluationResult::Empty)
            } else {
                Ok(EvaluationResult::Collection(all_children))
            }
        },
        _ => {
            // Primitive values have no children
            Ok(EvaluationResult::Empty)
        }
    }
}

/// Evaluate the descendants() function, which returns all nested child elements recursively
fn evaluate_descendants(value: &EvaluationResult) -> Result<EvaluationResult, EvaluationError> {
    match value {
        EvaluationResult::Object(obj) => {
            // Collect all values from the object and their descendants
            let mut descendants = Vec::new();
            
            for (_, child) in obj {
                // Only include non-empty values
                if !matches!(child, EvaluationResult::Empty) {
                    // Add the direct child
                    descendants.push(child.clone());
                    
                    // Add descendants of the child
                    let child_descendants = evaluate_descendants(child)?;
                    match child_descendants {
                        EvaluationResult::Empty => {
                            // Skip empty results
                        },
                        EvaluationResult::Collection(nested_descendants) => {
                            // Add all descendants from the nested collection
                            descendants.extend(nested_descendants);
                        },
                        _ => {
                            // Add the single descendant
                            descendants.push(child_descendants);
                        }
                    }
                }
            }
            
            // Return the collection of descendants
            if descendants.is_empty() {
                Ok(EvaluationResult::Empty)
            } else {
                Ok(EvaluationResult::Collection(descendants))
            }
        },
        EvaluationResult::Collection(items) => {
            // For collections, apply descendants() to each item and collect results
            let mut all_descendants = Vec::new();
            
            for item in items {
                let item_descendants = evaluate_descendants(item)?;
                
                match item_descendants {
                    EvaluationResult::Empty => {
                        // Skip empty results
                    },
                    EvaluationResult::Collection(descendants) => {
                        // Add all descendants from the nested collection
                        all_descendants.extend(descendants);
                    },
                    _ => {
                        // Add the single descendant
                        all_descendants.push(item_descendants);
                    }
                }
            }
            
            // Return the combined collection of descendants
            if all_descendants.is_empty() {
                Ok(EvaluationResult::Empty)
            } else {
                Ok(EvaluationResult::Collection(all_descendants))
            }
        },
        _ => {
            // Primitive values have no descendants
            Ok(EvaluationResult::Empty)
        }
    }
}
```

## 3. Variable Context Handling

### 3.1. Enhanced Variable Resolution

Improve the handling of variables, especially `$this`:

```rust
/// Enhanced evaluation context with improved variable handling
pub struct EvaluationContext {
    /// The FHIR resources being evaluated
    pub resources: Vec<FhirResource>,
    /// Variables defined in the context
    pub variables: HashMap<String, EvaluationResult>,
    /// The 'this' context for direct evaluation
    pub this: Option<EvaluationResult>,
    /// Root context for resolving path expressions
    pub root: Option<EvaluationResult>,
}

impl EvaluationContext {
    /// Creates a new evaluation context with the given FHIR resources
    pub fn new(resources: Vec<FhirResource>) -> Self {
        let root = if resources.len() == 1 {
            Some(convert_resource_to_result(&resources[0]))
        } else if !resources.is_empty() {
            Some(EvaluationResult::Collection(
                resources.iter().map(convert_resource_to_result).collect()
            ))
        } else {
            None
        };
        
        Self {
            resources,
            variables: HashMap::new(),
            this: None,
            root,
        }
    }
    
    /// Sets the 'this' context
    pub fn set_this(&mut self, value: EvaluationResult) {
        self.this = Some(value);
    }
    
    /// Gets the root context
    pub fn get_root(&self) -> EvaluationResult {
        if let Some(root) = &self.root {
            root.clone()
        } else if !self.resources.is_empty() {
            if self.resources.len() == 1 {
                convert_resource_to_result(&self.resources[0])
            } else {
                EvaluationResult::Collection(
                    self.resources.iter().map(convert_resource_to_result).collect()
                )
            }
        } else {
            EvaluationResult::Empty
        }
    }
    
    /// Gets the 'this' context or the root context if 'this' is not set
    pub fn get_this_or_root(&self) -> EvaluationResult {
        if let Some(this) = &self.this {
            this.clone()
        } else {
            self.get_root()
        }
    }
    
    /// Sets a variable in the context
    pub fn set_variable(&mut self, name: &str, value: EvaluationResult) {
        self.variables.insert(name.to_string(), value);
    }
    
    /// Gets a variable from the context
    pub fn get_variable(&self, name: &str) -> Option<&EvaluationResult> {
        self.variables.get(name)
    }
    
    /// Creates a new context with additional variables
    pub fn with_variables(&self, variables: HashMap<String, EvaluationResult>) -> Self {
        let mut new_context = self.clone();
        new_context.variables.extend(variables);
        new_context
    }
    
    /// Creates a new context with a new 'this' value
    pub fn with_this(&self, this: EvaluationResult) -> Self {
        let mut new_context = self.clone();
        new_context.this = Some(this);
        new_context
    }
}

/// Handle variable references in expressions
fn evaluate_variable_reference(name: &str, context: &EvaluationContext) -> Result<EvaluationResult, EvaluationError> {
    match name {
        "this" => {
            // Return the current context
            Ok(context.get_this_or_root())
        },
        "index" => {
            // Try to get the index variable
            if let Some(value) = context.get_variable("index") {
                Ok(value.clone())
            } else {
                Err(EvaluationError::UndefinedVariable("$index".to_string()))
            }
        },
        "total" => {
            // Try to get the total variable (for aggregate function)
            if let Some(value) = context.get_variable("total") {
                Ok(value.clone())
            } else {
                Err(EvaluationError::UndefinedVariable("$total".to_string()))
            }
        },
        _ => {
            // Try to get a custom variable
            if let Some(value) = context.get_variable(name) {
                Ok(value.clone())
            } else {
                Err(EvaluationError::UndefinedVariable(format!("${}", name)))
            }
        }
    }
}
```

## 4. Integration with Evaluator

Update the main `evaluate` function to support the enhanced collection handling:

```rust
/// Evaluates a FHIRPath expression in the given context
pub fn evaluate(
    expr: &Expression,
    context: &EvaluationContext,
    current_item: Option<&EvaluationResult>,
) -> Result<EvaluationResult, EvaluationError> {
    match expr {
        Expression::Term(term) => {
            evaluate_term(term, context, current_item)
        },
        Expression::Invocation(left, invocation) => {
            // Evaluate the left side first, passing the current item as context
            let left_result = evaluate(left, context, current_item)?;
            
            // Pass the evaluated left side and the context for invocation
            evaluate_invocation(&left_result, invocation, context)
        },
        Expression::Path(left, right) => {
            // Evaluate the left side first
            let left_result = evaluate(left, context, current_item)?;
            
            // Then evaluate the right side with the left result as the current item
            evaluate(right, context, Some(&left_result))
        },
        // ... other expression types ...
    }
}
```

## 5. Testing the Implementation

Create unit tests to verify the collection navigation fixes:

```rust
// In appropriate test file

#[test]
fn test_path_navigation() {
    // Create test context with Patient resource
    let context = create_test_context_with_patient();
    
    // Test basic navigation
    assert_eq!(
        evaluate("Patient.name.given", &context, None).unwrap(),
        EvaluationResult::Collection(vec![
            EvaluationResult::String("Peter".to_string()),
            EvaluationResult::String("James".to_string()),
            EvaluationResult::String("Jim".to_string()),
            EvaluationResult::String("Peter".to_string()),
            EvaluationResult::String("James".to_string()),
        ])
    );
}

#[test]
fn test_collection_functions() {
    // Create test context
    let context = create_test_context_with_patient();
    
    // Test count
    assert_eq!(
        evaluate("Patient.name.count()", &context, None).unwrap(),
        EvaluationResult::Integer(3)
    );
    
    // Test first and last
    assert_eq!(
        evaluate("Patient.name.first().given", &context, None).unwrap(),
        EvaluationResult::Collection(vec![
            EvaluationResult::String("Peter".to_string()),
            EvaluationResult::String("James".to_string()),
        ])
    );
    
    assert_eq!(
        evaluate("Patient.name.last().given", &context, None).unwrap(),
        EvaluationResult::Collection(vec![
            EvaluationResult::String("Peter".to_string()),
            EvaluationResult::String("James".to_string()),
        ])
    );
    
    // Test where
    assert_eq!(
        evaluate("Patient.name.where(given = 'Jim').count()", &context, None).unwrap(),
        EvaluationResult::Integer(1)
    );
    
    // Test select
    assert_eq!(
        evaluate("Patient.name.select(given).count()", &context, None).unwrap(),
        EvaluationResult::Integer(5)
    );
    
    // Test skip and take
    assert_eq!(
        evaluate("Patient.name.skip(1).given", &context, None).unwrap(),
        EvaluationResult::Collection(vec![
            EvaluationResult::String("Jim".to_string()),
            EvaluationResult::String("Peter".to_string()),
            EvaluationResult::String("James".to_string()),
        ])
    );
    
    assert_eq!(
        evaluate("Patient.name.take(1).given", &context, None).unwrap(),
        EvaluationResult::Collection(vec![
            EvaluationResult::String("Peter".to_string()),
            EvaluationResult::String("James".to_string()),
        ])
    );
}

#[test]
fn test_tree_navigation() {
    // Create test context with Questionnaire resource
    let context = create_test_context_with_questionnaire();
    
    // Test children
    assert_eq!(
        evaluate("Questionnaire.children().code.count()", &context, None).unwrap(),
        EvaluationResult::Integer(2)
    );
    
    // Test descendants
    assert_eq!(
        evaluate("Questionnaire.descendants().code.count()", &context, None).unwrap(),
        EvaluationResult::Integer(23)
    );
    
    // Test repeat
    assert_eq!(
        evaluate("Questionnaire.repeat(item).code.count()", &context, None).unwrap(),
        EvaluationResult::Integer(11)
    );
}

#[test]
fn test_variable_context() {
    // Create test context
    let context = create_test_context_with_patient();
    
    // Test $this in where
    assert_eq!(
        evaluate("Patient.name.given.where(substring($this.length()-3) = 'ter')", &context, None).unwrap(),
        EvaluationResult::Collection(vec![
            EvaluationResult::String("Peter".to_string()),
            EvaluationResult::String("Peter".to_string()),
        ])
    );
    
    // Test $this in select
    assert_eq!(
        evaluate("Patient.name.select($this.given)", &context, None).unwrap(),
        EvaluationResult::Collection(vec![
            EvaluationResult::Collection(vec![
                EvaluationResult::String("Peter".to_string()),
                EvaluationResult::String("James".to_string()),
            ]),
            EvaluationResult::Collection(vec![
                EvaluationResult::String("Jim".to_string()),
            ]),
            EvaluationResult::Collection(vec![
                EvaluationResult::String("Peter".to_string()),
                EvaluationResult::String("James".to_string()),
            ]),
        ])
    );
}
```

## 6. Main Changes Summary

This implementation fixes the following key issues:

1. **Collection Navigation**:
   - Properly handles nested collections in path expressions
   - Ensures correct flattening of collection results
   - Fixes chained navigation like Patient.name.given

2. **Collection Functions**:
   - Improved `where()` and `select()` functions with proper context handling
   - Fixed `skip()`, `take()`, `first()`, `last()` to maintain context for subsequent navigation
   - Fixed collection count and existence checks

3. **Tree Navigation**:
   - Enhanced `children()` and `descendants()` functions for proper recursion
   - Fixed counting and collection handling in tree navigation

4. **Variable Context Handling**:
   - Improved handling of $this variable in expressions
   - Fixed context propagation through collection operations
   - Enhanced variable resolution in lambda expressions

These changes collectively address the collection navigation and path traversal issues in the FHIRPath implementation, ensuring correct behavior in complex nested expressions.