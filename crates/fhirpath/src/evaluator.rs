use crate::parser::{Expression, Invocation, Literal, Term, TypeSpecifier};
use chrono::{Local, Timelike};
use fhir::FhirResource;
use fhirpath_support::{EvaluationResult, IntoEvaluationResult};
use regex::Regex;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use std::collections::{HashMap, HashSet};

/// Context for evaluating FHIRPath expressions
pub struct EvaluationContext {
    /// The FHIR resources being evaluated
    pub resources: Vec<FhirResource>,
    /// Variables defined in the context as string values
    pub variables: HashMap<String, String>,
}

impl EvaluationContext {
    /// Creates a new evaluation context with the given FHIR resources
    pub fn new(resources: Vec<FhirResource>) -> Self {
        Self {
            resources,
            variables: HashMap::new(),
        }
    }

    /// Creates a new empty evaluation context with no resources
    pub fn new_empty() -> Self {
        Self {
            resources: Vec::new(),
            variables: HashMap::new(),
        }
    }

    /// Adds a resource to the context
    pub fn add_resource(&mut self, resource: FhirResource) {
        self.resources.push(resource);
    }

    /// Sets a variable in the context
    pub fn set_variable(&mut self, name: &str, value: String) {
        self.variables.insert(name.to_string(), value);
    }

    /// Gets a variable from the context
    pub fn get_variable(&self, name: &str) -> Option<&String> {
        self.variables.get(name)
    }

    /// Gets a variable from the context as an EvaluationResult
    pub fn get_variable_as_result(&self, name: &str) -> EvaluationResult {
        match self.variables.get(name) {
            Some(value) => EvaluationResult::String(value.clone()),
            None => EvaluationResult::Empty,
        }
    }
}

/// Applies decimal-only multiplicative operators (div, mod)
fn apply_decimal_multiplicative(left: Decimal, op: &str, right: Decimal) -> EvaluationResult {
    if right.is_zero() {
        return EvaluationResult::Empty; // Division/Modulo by zero
    }
    match op {
        "div" => {
            // Decimal div Decimal -> Integer (truncate)
            (left / right)
                .trunc() // Truncate the result
                .to_i64() // Convert to i64
                .map(EvaluationResult::Integer)
                .unwrap_or(EvaluationResult::Empty) // Handle potential conversion errors
        }
        "mod" => {
            // Decimal mod Decimal -> Decimal
            EvaluationResult::Decimal(left % right)
        }
        _ => EvaluationResult::Empty, // Should not happen
    }
}

/// Evaluates a FHIRPath expression in the given context, potentially with a specific item as context ($this).
pub fn evaluate(
    expr: &Expression,
    context: &EvaluationContext,
    current_item: Option<&EvaluationResult>,
) -> EvaluationResult {
    match expr {
        Expression::Term(term) => evaluate_term(term, context, current_item),
        Expression::Invocation(left, invocation) => {
            // Evaluate the left side first, passing the current item context
            let left_result = evaluate(left, context, current_item);
            // Pass the evaluated left side result and the original context for invocation
            evaluate_invocation(&left_result, invocation, context)
        }
        Expression::Indexer(left, index) => {
            let left_result = evaluate(left, context, current_item);
            // Index expression doesn't depend on $this, evaluate normally
            let index_result = evaluate(index, context, None);
            evaluate_indexer(&left_result, &index_result)
        }
        Expression::Polarity(op, expr) => {
            let result = evaluate(expr, context, current_item);
            apply_polarity(*op, &result)
        }
        Expression::Multiplicative(left, op, right) => {
            let left_result = evaluate(left, context, current_item);
            let right_result = evaluate(right, context, current_item);
            apply_multiplicative(&left_result, op, &right_result)
        }
        Expression::Additive(left, op, right) => {
            let left_result = evaluate(left, context, current_item);
            let right_result = evaluate(right, context, current_item);
            apply_additive(&left_result, op, &right_result)
        }
        Expression::Type(left, op, type_spec) => {
            let result = evaluate(left, context, current_item);
            apply_type_operation(&result, op, type_spec)
        }
        Expression::Union(left, right) => {
            let left_result = evaluate(left, context, current_item);
            let right_result = evaluate(right, context, current_item);
            union_collections(&left_result, &right_result)
        }
        Expression::Inequality(left, op, right) => {
            let left_result = evaluate(left, context, current_item);
            let right_result = evaluate(right, context, current_item);
            compare_inequality(&left_result, op, &right_result)
        }
        Expression::Equality(left, op, right) => {
            let left_result = evaluate(left, context, current_item);
            let right_result = evaluate(right, context, current_item);
            compare_equality(&left_result, op, &right_result)
        }
        Expression::Membership(left, op, right) => {
            let left_result = evaluate(left, context, current_item);
            let right_result = evaluate(right, context, current_item); // Evaluate right side normally for 'in'/'contains'
            check_membership(&left_result, op, &right_result)
        }
        Expression::And(left, right) => {
            let left_result = evaluate(left, context, current_item);
            // Short-circuit evaluation
            if !left_result.to_boolean() {
                return EvaluationResult::Boolean(false);
            }
            // Only evaluate right if left is true
            let right_result = evaluate(right, context, current_item);
            EvaluationResult::Boolean(right_result.to_boolean())
        }
        Expression::Or(left, op, right) => {
            let left_result = evaluate(left, context, current_item);
            // Short-circuit for 'or'
            if op == "or" && left_result.to_boolean() {
                return EvaluationResult::Boolean(true);
            }
            // Evaluate right side
            let right_result = evaluate(right, context, current_item);
            if op == "or" {
                EvaluationResult::Boolean(left_result.to_boolean() || right_result.to_boolean())
            } else {
                // xor: requires both sides to be evaluated unless one is Empty
                if left_result == EvaluationResult::Empty || right_result == EvaluationResult::Empty
                {
                    EvaluationResult::Empty // FHIRPath spec: xor with Empty is Empty
                } else {
                    EvaluationResult::Boolean(left_result.to_boolean() != right_result.to_boolean())
                }
            }
        }
        Expression::Implies(left, right) => {
            let left_result = evaluate(left, context, current_item);
            // Short-circuit: false implies anything is true
            if !left_result.to_boolean() && left_result != EvaluationResult::Empty {
                return EvaluationResult::Boolean(true);
            }
            // Handle Empty implies X -> true
            if left_result == EvaluationResult::Empty {
                return EvaluationResult::Boolean(true);
            }
            // Evaluate right side
            let right_result = evaluate(right, context, current_item);
            // Handle X implies Empty -> Empty
            if right_result == EvaluationResult::Empty {
                return EvaluationResult::Empty;
            }
            // Otherwise, the result is the boolean value of the right side
            EvaluationResult::Boolean(right_result.to_boolean())
        }
        Expression::Lambda(_, _) => {
            // Lambda expressions are not directly evaluated here.
            // They are used in function calls
            EvaluationResult::Empty
        }
    }
}

/// Evaluates a term in the given context, potentially with a specific item as context ($this).
fn evaluate_term(
    term: &Term,
    context: &EvaluationContext,
    current_item: Option<&EvaluationResult>,
) -> EvaluationResult {
    match term {
        Term::Invocation(invocation) => {
            // Explicitly handle $this first and return
            if *invocation == Invocation::This {
                return if let Some(item) = current_item.cloned() {
                    item // Return the item if Some
                } else {
                    // Return the default context if None
                    if context.resources.is_empty() {
                        EvaluationResult::Empty
                    } else if context.resources.len() == 1 {
                        convert_resource_to_result(&context.resources[0])
                    } else {
                        EvaluationResult::Collection(
                            context
                                .resources
                                .iter()
                                .map(convert_resource_to_result)
                                .collect(),
                        )
                    }
                };
            }

            // Handle variables (%var, %context) next and return
            if let Invocation::Member(name) = invocation {
                if name.starts_with('%') {
                    let var_name = &name[1..]; // Remove the % prefix
                    if var_name == "context" {
                        // Return %context value
                        return if context.resources.is_empty() {
                            EvaluationResult::Empty
                        } else if context.resources.len() == 1 {
                            convert_resource_to_result(&context.resources[0])
                        } else {
                            EvaluationResult::Collection(
                                context
                                    .resources
                                    .iter()
                                    .map(convert_resource_to_result)
                                    .collect(),
                            )
                        };
                    } else {
                        // Return other variable value
                        return context.get_variable_as_result(var_name);
                    }
                }
            }

            // If not $this or a variable, it must be a member/function invocation.
            // Determine the base context for this invocation.
            let base_context = current_item.cloned().unwrap_or_else(|| {
                // Default to the main resource context if no specific item
                if context.resources.is_empty() {
                    EvaluationResult::Empty
                } else if context.resources.len() == 1 {
                    convert_resource_to_result(&context.resources[0])
                } else {
                    EvaluationResult::Collection(
                        context
                            .resources
                            .iter()
                            .map(convert_resource_to_result)
                            .collect(),
                    )
                }
            });

            // Evaluate the member/function invocation on the base context.
            // This is the final return value for this match arm in this case.
            evaluate_invocation(&base_context, invocation, context)
        }
        Term::Literal(literal) => evaluate_literal(literal),
        Term::ExternalConstant(name) => {
            // Look up external constant in the context
            // Special handling for %context
            if name == "context" {
                if context.resources.is_empty() {
                    EvaluationResult::Empty
                } else if context.resources.len() == 1 {
                    convert_resource_to_result(&context.resources[0])
                } else {
                    EvaluationResult::Collection(
                        context
                            .resources
                            .iter()
                            .map(convert_resource_to_result)
                            .collect(),
                    )
                }
            } else {
                context.get_variable_as_result(name)
            }
        }
        Term::Parenthesized(expr) => evaluate(expr, context, current_item),
    }
}

/// Converts a FHIR resource to an EvaluationResult by calling the trait method directly.
#[inline] // Suggest inlining this simple function call
fn convert_resource_to_result(resource: &FhirResource) -> EvaluationResult {
    // Now that FhirResource implements IntoEvaluationResult, just call the method.
    resource.into_evaluation_result()
}

/// Evaluates a literal value
fn evaluate_literal(literal: &Literal) -> EvaluationResult {
    match literal {
        Literal::Null => EvaluationResult::Empty,
        Literal::Boolean(b) => EvaluationResult::Boolean(*b),
        Literal::String(s) => EvaluationResult::String(s.clone()),
        Literal::Number(d) => EvaluationResult::Decimal(*d), // Decimal literal
        Literal::Integer(n) => EvaluationResult::Integer(*n), // Integer literal
        Literal::Date(d) => EvaluationResult::Date(d.clone()),
        Literal::DateTime(d, t) => {
            // Include timezone in the result string if present
            if let Some((time, timezone_opt)) = t {
                let mut dt_string = format!("{}T{}", d, time);
                if let Some(tz) = timezone_opt {
                    dt_string.push_str(tz);
                }
                EvaluationResult::DateTime(dt_string)
            } else {
                // Handle date-only DateTime literals (e.g., @2023T) if necessary,
                // though the parser might prevent this specific case.
                // For now, assume if 't' is None, it's just a date.
                // However, the Literal::Date variant should handle this.
                // If we reach here with t=None, it might indicate a parser issue
                // or an unexpected Literal variant. Let's treat it as just the date part for now.
                eprintln!(
                    "Warning: DateTime literal evaluated without time part: {}",
                    d
                );
                EvaluationResult::DateTime(d.clone()) // Or potentially return Date(d.clone()) or Empty
            }
        }
        Literal::Time(t) => EvaluationResult::Time(t.clone()),
        Literal::Quantity(n, _) => {
            // For now, we ignore the unit.
            // Return Integer if the Decimal represents a whole number and fits in i64,
            // otherwise return Decimal.
            if n.is_integer() {
                // Attempt to convert to i64 if it fits
                n.to_i64()
                    .map(EvaluationResult::Integer)
                    .unwrap_or_else(|| EvaluationResult::Decimal(*n))
            } else {
                EvaluationResult::Decimal(*n)
            }
        }
    }
}

/// Evaluates an invocation on a value
fn evaluate_invocation(
    invocation_base: &EvaluationResult, // The result of the expression the invocation is called on
    invocation: &Invocation,
    context: &EvaluationContext, // The overall evaluation context (for variables etc.)
) -> EvaluationResult {
    match invocation {
        Invocation::Member(name) => {
            // Handle member access on the invocation_base
            // Special handling for boolean literals that might be parsed as identifiers
            if name == "true" && matches!(invocation_base, EvaluationResult::Empty) {
                // Only if base is empty context
                return EvaluationResult::Boolean(true);
            } else if name == "false" && matches!(invocation_base, EvaluationResult::Empty) {
                return EvaluationResult::Boolean(false);
            }

            // Access a member of the invocation_base
            match invocation_base {
                EvaluationResult::Object(obj) => {
                    obj.get(name).cloned().unwrap_or(EvaluationResult::Empty)
                }
                EvaluationResult::Collection(items) => {
                    // For collections, apply member access to each item and collect results
                    let results: Vec<EvaluationResult> = items
                        .iter()
                        .map(|item| {
                            // Recursively call member access on each item
                            evaluate_invocation(item, &Invocation::Member(name.clone()), context)
                        })
                        .filter(|res| *res != EvaluationResult::Empty) // Filter out empty results from individual items
                        .collect();

                    // Flatten nested collections that might result from member access on items
                    let flattened_results: Vec<EvaluationResult> = results
                        .into_iter()
                        .flat_map(|res| match res {
                            EvaluationResult::Collection(inner) => inner,
                            EvaluationResult::Empty => vec![],
                            other => vec![other],
                        })
                        .collect();

                    if flattened_results.is_empty() {
                        EvaluationResult::Empty
                    } else if flattened_results.len() == 1 {
                        flattened_results.into_iter().next().unwrap() // Return single item directly
                    } else {
                        EvaluationResult::Collection(flattened_results)
                    }
                }
                // Accessing member on primitive types or Empty returns Empty
                _ => EvaluationResult::Empty,
            }
        }
        Invocation::Function(name, args_exprs) => {
            // Use args_exprs (AST)
            // Handle functions that take lambdas specially
            match name.as_str() {
                "exists" if !args_exprs.is_empty() => {
                    let criteria_expr = &args_exprs[0];
                    evaluate_exists_with_criteria(invocation_base, criteria_expr, context)
                }
                "where" if !args_exprs.is_empty() => {
                    let criteria_expr = &args_exprs[0];
                    evaluate_where(invocation_base, criteria_expr, context)
                }
                "select" if !args_exprs.is_empty() => {
                    let projection_expr = &args_exprs[0];
                    evaluate_select(invocation_base, projection_expr, context)
                }
                "all" if !args_exprs.is_empty() => {
                    let criteria_expr = &args_exprs[0];
                    evaluate_all_with_criteria(invocation_base, criteria_expr, context)
                }
                "ofType" if args_exprs.len() == 1 => {
                    let type_spec_opt = match &args_exprs[0] {
                        // Handle simple identifier like 'Integer'
                        Expression::Term(Term::Invocation(Invocation::Member(type_name))) => {
                            Some(TypeSpecifier::QualifiedIdentifier(type_name.clone(), None))
                        }
                        // Handle qualified identifier like 'System.Integer'
                        // The parser seems to produce Invocation(base, member) for this
                        Expression::Invocation(base_expr, Invocation::Member(member_name)) => {
                            // Check if the base is a simple member invocation (like 'System')
                            if let Expression::Term(Term::Invocation(Invocation::Member(
                                base_name,
                            ))) = &**base_expr
                            {
                                // Reconstruct the qualified name: "System.Integer"
                                // For now, store the full name as the identifier. evaluate_of_type will handle it.
                                let full_name = format!("{}.{}", base_name, member_name);
                                Some(TypeSpecifier::QualifiedIdentifier(full_name, None))
                            } else {
                                None // Unexpected structure for qualified identifier base
                            }
                        }
                        _ => None, // Argument is not a recognized type identifier structure
                    };

                    if let Some(type_spec) = type_spec_opt {
                        evaluate_of_type(invocation_base, &type_spec)
                    } else {
                        eprintln!(
                            "Warning: ofType argument was not a recognized type identifier structure: {:?}",
                            args_exprs[0]
                        );
                        EvaluationResult::Empty // Invalid argument for ofType
                    }
                }
                "iif" if args_exprs.len() >= 2 => {
                    // iif(condition, trueResult, [otherwiseResult])
                    let condition_expr = &args_exprs[0];
                    let true_result_expr = &args_exprs[1];
                    let otherwise_result_expr = args_exprs.get(2); // Optional third argument

                    // Evaluate the condition expression.
                    // iif condition is evaluated in the context of the invocation_base ($this)
                    let condition_result = evaluate(condition_expr, context, Some(invocation_base));

                    if condition_result.to_boolean() {
                        // Condition is true, evaluate the trueResult expression
                        // trueResult is also evaluated in the context of the invocation_base ($this)
                        evaluate(true_result_expr, context, Some(invocation_base))
                    } else {
                        // Condition is false or empty
                        if let Some(otherwise_expr) = otherwise_result_expr {
                            // Evaluate the otherwiseResult expression if present
                            // otherwiseResult is also evaluated in the context of the invocation_base ($this)
                            evaluate(otherwise_expr, context, Some(invocation_base))
                        } else {
                            // Otherwise result is omitted, return empty collection
                            EvaluationResult::Empty
                        }
                    }
                }
                // Add other functions taking lambdas here (e.g., any, repeat)
                _ => {
                    // Default: Evaluate all standard function arguments first (without $this context), then call function
                    let evaluated_args: Vec<EvaluationResult> = args_exprs
                        .iter()
                        .map(|arg_expr| evaluate(arg_expr, context, None)) // Evaluate args in outer context
                        .collect();
                    // Call with updated signature (name, base, args)
                    call_function(name, invocation_base, &evaluated_args)
                }
            }
        }
        Invocation::This => {
            // This should be handled by evaluate_term, but as a fallback:
            invocation_base.clone() // Return the base it was invoked on
        }
        Invocation::Index => {
            // $index should return the current index in a collection operation
            // This is typically used in filter expressions
            // For now, we return Empty as this requires tracking iteration state
            EvaluationResult::Empty
        }
        Invocation::Total => {
            // $total should return the total number of items in the original collection
            // For now, we return Empty as this requires tracking the original collection
            EvaluationResult::Empty
        }
    }
}

// --- Helper functions for lambda evaluation ---

/// Evaluates the 'exists' function with a criteria expression.
fn evaluate_exists_with_criteria(
    collection: &EvaluationResult,
    criteria_expr: &Expression,
    context: &EvaluationContext,
) -> EvaluationResult {
    let items_to_check = match collection {
        EvaluationResult::Collection(items) => items.clone(),
        EvaluationResult::Empty => vec![],
        // Treat single item as a one-item collection
        single_item => vec![single_item.clone()],
    };

    if items_to_check.is_empty() {
        return EvaluationResult::Boolean(false); // Exists is false for empty collection
    }

    for item in items_to_check {
        // Evaluate the criteria expression with the current item as $this
        let criteria_result = evaluate(criteria_expr, context, Some(&item));
        // exists returns true if the criteria evaluates to true for *any* item
        if criteria_result.to_boolean() {
            return EvaluationResult::Boolean(true);
        }
    }

    // If no item satisfied the criteria
    EvaluationResult::Boolean(false)
}

/// Evaluates the 'where' function.
fn evaluate_where(
    collection: &EvaluationResult,
    criteria_expr: &Expression,
    context: &EvaluationContext,
) -> EvaluationResult {
    let items_to_filter = match collection {
        EvaluationResult::Collection(items) => items.clone(),
        EvaluationResult::Empty => vec![],
        single_item => vec![single_item.clone()],
    };

    let mut filtered_items = Vec::new();
    for item in items_to_filter {
        let criteria_result = evaluate(criteria_expr, context, Some(&item));
        if criteria_result.to_boolean() {
            filtered_items.push(item.clone());
        }
    }

    // Return Empty or Collection
    if filtered_items.is_empty() {
        EvaluationResult::Empty
    } else {
        EvaluationResult::Collection(filtered_items)
    }
}

/// Evaluates the 'select' function.
fn evaluate_select(
    collection: &EvaluationResult,
    projection_expr: &Expression,
    context: &EvaluationContext,
) -> EvaluationResult {
    let items_to_project = match collection {
        EvaluationResult::Collection(items) => items.clone(),
        EvaluationResult::Empty => vec![],
        single_item => vec![single_item.clone()],
    };

    let mut projected_items = Vec::new();
    for item in items_to_project {
        let projection_result = evaluate(projection_expr, context, Some(&item));
        // Flatten results: if projection yields a collection, add its items individually
        match projection_result {
            EvaluationResult::Collection(inner_items) => {
                projected_items.extend(inner_items);
            }
            EvaluationResult::Empty => {} // Skip empty results
            single_result => {
                projected_items.push(single_result);
            }
        }
    }

    // Return Empty or Collection
    if projected_items.is_empty() {
        EvaluationResult::Empty
    } else {
        EvaluationResult::Collection(projected_items)
    }
}

/// Evaluates the 'all' function with a criteria expression.
fn evaluate_all_with_criteria(
    collection: &EvaluationResult,
    criteria_expr: &Expression,
    context: &EvaluationContext,
) -> EvaluationResult {
    let items_to_check = match collection {
        EvaluationResult::Collection(items) => items.clone(),
        EvaluationResult::Empty => vec![],
        // Treat single item as a one-item collection
        single_item => vec![single_item.clone()],
    };

    // 'all' is true for an empty collection
    if items_to_check.is_empty() {
        return EvaluationResult::Boolean(true);
    }

    for item in items_to_check {
        // Evaluate the criteria expression with the current item as $this
        let criteria_result = evaluate(criteria_expr, context, Some(&item));
        // 'all' returns false if the criteria evaluates to false for *any* item
        if !criteria_result.to_boolean() {
            return EvaluationResult::Boolean(false);
        }
    }

    // If all items satisfied the criteria
    EvaluationResult::Boolean(true)
}

/// Evaluates the 'ofType' function.
fn evaluate_of_type(collection: &EvaluationResult, type_spec: &TypeSpecifier) -> EvaluationResult {
    let items_to_filter = match collection {
        EvaluationResult::Collection(items) => items.clone(),
        EvaluationResult::Empty => vec![],
        single_item => vec![single_item.clone()],
    };

    let target_type_name = match type_spec {
        // The name might now be "System.Integer" or just "Integer"
        TypeSpecifier::QualifiedIdentifier(name, _namespace) => {
            // If the name contains '.', take the part after the last dot.
            // This handles both "System.Integer" -> "Integer" and "Integer" -> "Integer".
            name.split('.').last().unwrap_or(name.as_str())
        }
    };

    let mut filtered_items = Vec::new();
    for item in items_to_filter {
        let matches = match (target_type_name, &item) {
            ("Boolean", EvaluationResult::Boolean(_)) => true,
            ("String", EvaluationResult::String(_)) => true,
            ("Integer", EvaluationResult::Integer(_)) => true,
            ("Decimal", EvaluationResult::Decimal(_)) => true,
            ("Date", EvaluationResult::Date(_)) => true,
            ("DateTime", EvaluationResult::DateTime(_)) => true,
            ("Time", EvaluationResult::Time(_)) => true,
            // Handle complex FHIR types by checking resourceType
            (fhir_type, EvaluationResult::Object(fields)) => {
                if let Some(EvaluationResult::String(rt)) = fields.get("resourceType") {
                    rt == fhir_type
                } else {
                    false // Object has no resourceType field
                }
            }
            // Add checks for other FHIR primitive types if needed (e.g., Quantity, Code)
            // These might require inspecting the structure more deeply if not directly mapped
            // to EvaluationResult variants. For now, basic types and resourceType check.
            _ => false, // Type mismatch
        };

        if matches {
            filtered_items.push(item.clone());
        }
    }

    // Return Empty or Collection
    if filtered_items.is_empty() {
        EvaluationResult::Empty
    } else {
        EvaluationResult::Collection(filtered_items)
    }
}

/// Calls a standard FHIRPath function (that doesn't take a lambda).
fn call_function(
    name: &str,
    invocation_base: &EvaluationResult, // Renamed from context to avoid confusion
    args: &[EvaluationResult],
) -> EvaluationResult {
    match name {
        "count" => {
            // Returns the number of items in the collection
            if let EvaluationResult::Collection(items) = invocation_base {
                EvaluationResult::Integer(items.len() as i64)
            } else {
                // Single items count as 1, empty counts as 0
                match invocation_base {
                    EvaluationResult::Empty => EvaluationResult::Integer(0),
                    _ => EvaluationResult::Integer(1),
                }
            }
        }
        "empty" => {
            // Returns true if the collection is empty
            match invocation_base {
                // Use invocation_base, not context
                EvaluationResult::Empty => EvaluationResult::Boolean(true),
                EvaluationResult::Collection(items) => EvaluationResult::Boolean(items.is_empty()),
                _ => EvaluationResult::Boolean(false), // Single non-empty item is not empty
            }
        }
        "exists" => {
            // This handles exists() without criteria.
            // exists(criteria) is handled in evaluate_invocation.
            match invocation_base {
                EvaluationResult::Empty => EvaluationResult::Boolean(false),
                EvaluationResult::Collection(items) => EvaluationResult::Boolean(!items.is_empty()),
                _ => EvaluationResult::Boolean(true), // Single non-empty item exists
            }
        }
        "all" => {
            // This handles all() without criteria, which is equivalent to exists(criteria)
            // where the criteria is implicitly '$this = true'.
            // However, the spec implies all() without criteria might not be standard.
            // For now, let's align with exists() logic for non-empty check.
            // all(criteria) is handled in evaluate_invocation.
            match invocation_base {
                EvaluationResult::Empty => EvaluationResult::Boolean(true), // all() is true for empty
                EvaluationResult::Collection(items) => EvaluationResult::Boolean(!items.is_empty()), // Placeholder: Needs proper criteria check if used
                _ => EvaluationResult::Boolean(true), // Single non-empty item exists
            }
        }
        "allTrue" => {
            let items = match invocation_base {
                EvaluationResult::Collection(items) => items.clone(),
                EvaluationResult::Empty => vec![],
                single_item => vec![single_item.clone()],
            };
            // allTrue is true for an empty collection
            if items.is_empty() {
                return EvaluationResult::Boolean(true);
            }
            for item in items {
                if !matches!(item, EvaluationResult::Boolean(true)) {
                    return EvaluationResult::Boolean(false);
                }
            }
            EvaluationResult::Boolean(true)
        }
        "anyTrue" => {
            let items = match invocation_base {
                EvaluationResult::Collection(items) => items.clone(),
                EvaluationResult::Empty => vec![],
                single_item => vec![single_item.clone()],
            };
            // anyTrue is false for an empty collection
            if items.is_empty() {
                return EvaluationResult::Boolean(false);
            }
            for item in items {
                if matches!(item, EvaluationResult::Boolean(true)) {
                    return EvaluationResult::Boolean(true);
                }
            }
            EvaluationResult::Boolean(false) // No true item found
        }
        "allFalse" => {
            let items = match invocation_base {
                EvaluationResult::Collection(items) => items.clone(),
                EvaluationResult::Empty => vec![],
                single_item => vec![single_item.clone()],
            };
            // allFalse is true for an empty collection
            if items.is_empty() {
                return EvaluationResult::Boolean(true);
            }
            for item in items {
                if !matches!(item, EvaluationResult::Boolean(false)) {
                    return EvaluationResult::Boolean(false);
                }
            }
            EvaluationResult::Boolean(true)
        }
        "anyFalse" => {
            let items = match invocation_base {
                EvaluationResult::Collection(items) => items.clone(),
                EvaluationResult::Empty => vec![],
                single_item => vec![single_item.clone()],
            };
            // anyFalse is false for an empty collection
            if items.is_empty() {
                return EvaluationResult::Boolean(false);
            }
            for item in items {
                if matches!(item, EvaluationResult::Boolean(false)) {
                    return EvaluationResult::Boolean(true);
                }
            }
            EvaluationResult::Boolean(false) // No false item found
        }
        "first" => {
            // Returns the first item in the collection
            if let EvaluationResult::Collection(items) = invocation_base {
                items.first().cloned().unwrap_or(EvaluationResult::Empty)
            } else {
                // A single item is returned as is (unless it's Empty)
                invocation_base.clone()
            }
        }
        "last" => {
            // Returns the last item in the collection
            if let EvaluationResult::Collection(items) = invocation_base {
                items.last().cloned().unwrap_or(EvaluationResult::Empty)
            } else {
                // A single item is returned as is (unless it's Empty)
                invocation_base.clone()
            }
        }
        "not" => {
            // Logical negation
            match invocation_base {
                EvaluationResult::Boolean(b) => EvaluationResult::Boolean(!b),
                EvaluationResult::Empty => EvaluationResult::Empty, // not({}) is {}
                // Other types are implicitly converted to boolean first
                _ => EvaluationResult::Boolean(!invocation_base.to_boolean()),
            }
        }
        "contains" => {
            // Function call version
            // Check if the invocation_base contains the argument
            if args.is_empty() {
                return EvaluationResult::Empty; // Requires one argument
            }
            let arg = &args[0];

            match invocation_base {
                EvaluationResult::String(s) => {
                    // String contains substring
                    // Explicitly check if arg is Empty *within this arm*
                    if arg == &EvaluationResult::Empty {
                        EvaluationResult::Empty
                    } else if let EvaluationResult::String(substr) = arg {
                        EvaluationResult::Boolean(s.contains(substr))
                    } else {
                        // Argument is not Empty and not String
                        EvaluationResult::Empty
                    }
                }
                EvaluationResult::Collection(items) => {
                    // Collection contains item (using equality)
                    // Check if arg is empty first (already done above, but double-check doesn't hurt)
                    if arg == &EvaluationResult::Empty {
                        EvaluationResult::Empty
                    } else {
                        let contains = items
                            .iter()
                            .any(|item| compare_equality(item, "=", arg).to_boolean());
                        EvaluationResult::Boolean(contains)
                    }
                }
                // contains on single non-collection/non-string item
                EvaluationResult::Empty => EvaluationResult::Empty, // Spec: If input collection is empty, result is empty.
                single_item => {
                    // Treat as single-item collection: check if the item equals the argument
                    // Check if arg is empty first
                    if arg == &EvaluationResult::Empty {
                        EvaluationResult::Empty
                    } else {
                        EvaluationResult::Boolean(
                            compare_equality(single_item, "=", arg).to_boolean(),
                        )
                    }
                }
            }
        }
        "isDistinct" => {
            // Returns true if all items in the collection are distinct (based on equality)
            let items = match invocation_base {
                EvaluationResult::Collection(items) => items.clone(),
                EvaluationResult::Empty => vec![],
                single_item => vec![single_item.clone()], // Treat single item as collection
            };

            if items.len() <= 1 {
                return EvaluationResult::Boolean(true); // Empty or single-item collections are distinct
            }

            for i in 0..items.len() {
                for j in (i + 1)..items.len() {
                    // Use compare_equality to check for duplicates
                    if compare_equality(&items[i], "=", &items[j]).to_boolean() {
                        return EvaluationResult::Boolean(false); // Found a duplicate
                    }
                }
            }

            EvaluationResult::Boolean(true) // No duplicates found
        }
        "toDecimal" => {
            // Converts the input to Decimal according to FHIRPath rules
            match invocation_base {
                EvaluationResult::Empty => EvaluationResult::Empty,
                EvaluationResult::Boolean(b) => {
                    EvaluationResult::Decimal(if *b { Decimal::ONE } else { Decimal::ZERO })
                }
                EvaluationResult::Integer(i) => EvaluationResult::Decimal(Decimal::from(*i)),
                EvaluationResult::Decimal(d) => EvaluationResult::Decimal(*d),
                EvaluationResult::String(s) => {
                    // Try parsing as Decimal
                    s.parse::<Decimal>()
                        .map(EvaluationResult::Decimal)
                        .unwrap_or(EvaluationResult::Empty) // Return Empty if parsing fails
                }
                // Collections: Convert single item, multiple items -> Empty
                EvaluationResult::Collection(items) => {
                    if items.len() == 1 {
                        // Recursively call toDecimal on the single item
                        call_function("toDecimal", &items[0], &[])
                    } else {
                        EvaluationResult::Empty // Multi-item or empty collection -> Empty
                    }
                }
                // Other types are not convertible
                _ => EvaluationResult::Empty,
            }
        }
        "toInteger" => {
            // Converts the input to Integer according to FHIRPath rules
            match invocation_base {
                EvaluationResult::Empty => EvaluationResult::Empty,
                EvaluationResult::Boolean(b) => EvaluationResult::Integer(if *b { 1 } else { 0 }),
                EvaluationResult::Integer(i) => EvaluationResult::Integer(*i),
                EvaluationResult::String(s) => {
                    // Try parsing as i64
                    s.parse::<i64>()
                        .map(EvaluationResult::Integer)
                        .unwrap_or(EvaluationResult::Empty) // Return Empty if parsing fails
                }
                // Per FHIRPath spec, Decimal cannot be converted to Integer via toInteger()
                EvaluationResult::Decimal(_) => EvaluationResult::Empty,
                // Collections: Convert single item, multiple items -> Empty
                EvaluationResult::Collection(items) => {
                    if items.len() == 1 {
                        // Recursively call toInteger on the single item
                        call_function("toInteger", &items[0], &[])
                    } else {
                        EvaluationResult::Empty // Multi-item or empty collection -> Empty
                    }
                }
                // Other types are not convertible
                _ => EvaluationResult::Empty,
            }
        }
        "distinct" => {
            // Returns the collection with duplicates removed (based on equality)
            let items = match invocation_base {
                EvaluationResult::Collection(items) => items.clone(),
                EvaluationResult::Empty => return EvaluationResult::Empty, // Distinct on empty is empty
                single_item => return single_item.clone(), // Distinct on single item is the item itself
            };

            // If we reach here, items has at least 2 elements

            // Use HashSet to efficiently find distinct items
            let mut distinct_set = HashSet::new();
            let mut distinct_items = Vec::new(); // Maintain original order of first appearance

            for item in items {
                // Attempt to insert the item into the HashSet.
                // If insert returns true, it's the first time we've seen this item (or an equivalent one).
                if distinct_set.insert(item.clone()) {
                    distinct_items.push(item); // Add to the result list to preserve order
                }
            }

            // Return Empty or Collection
            if distinct_items.is_empty() {
                EvaluationResult::Empty
            } else {
                EvaluationResult::Collection(distinct_items)
            }
        }
        "skip" => {
            // Returns the collection with the first 'num' items removed
            if args.len() != 1 {
                return EvaluationResult::Empty; // Skip requires exactly one argument
            }
            let num_to_skip = match &args[0] {
                EvaluationResult::Integer(i) => {
                    if *i < 0 { 0 } else { *i as usize } // Treat negative skip as 0
                }
                // Add conversion from Decimal if it's an integer value
                EvaluationResult::Decimal(d) if d.is_integer() && d.is_sign_positive() => {
                    d.to_usize().unwrap_or(0) // Convert non-negative integer Decimal
                }
                _ => return EvaluationResult::Empty, // Invalid argument type
            };

            let items = match invocation_base {
                EvaluationResult::Collection(items) => items.clone(),
                EvaluationResult::Empty => vec![],
                single_item => vec![single_item.clone()], // Treat single item as collection
            };

            if num_to_skip >= items.len() {
                EvaluationResult::Empty
            } else {
                let skipped_items = items[num_to_skip..].to_vec();
                // Return Empty or Collection
                if skipped_items.is_empty() {
                    EvaluationResult::Empty
                } else {
                    EvaluationResult::Collection(skipped_items)
                }
            }
        }
        "tail" => {
            // Returns the collection with all items except the first
            if let EvaluationResult::Collection(items) = invocation_base {
                if items.len() > 1 {
                    // Create a new Vec containing elements from the second one onwards
                    EvaluationResult::Collection(items[1..].to_vec())
                } else {
                    // Empty or single-item collection results in empty
                    EvaluationResult::Empty
                }
            } else if invocation_base == &EvaluationResult::Empty {
                // Tail on Empty results in Empty
                EvaluationResult::Empty
            } else {
                // Tail on a single non-collection item results in empty
                EvaluationResult::Empty
            }
        }
        "take" => {
            // Returns the first 'num' items from the collection
            if args.len() != 1 {
                return EvaluationResult::Empty; // Take requires exactly one argument
            }
            let num_to_take = match &args[0] {
                EvaluationResult::Integer(i) => {
                    if *i <= 0 { 0 } else { *i as usize } // Treat non-positive take as 0
                }
                // Add conversion from Decimal if it's an integer value
                EvaluationResult::Decimal(d) if d.is_integer() && d.is_sign_positive() => {
                    d.to_usize().unwrap_or(0) // Convert non-negative integer Decimal
                }
                _ => return EvaluationResult::Empty, // Invalid argument type
            };

            if num_to_take == 0 {
                return EvaluationResult::Empty;
            }

            let items = match invocation_base {
                EvaluationResult::Collection(items) => items.clone(),
                EvaluationResult::Empty => vec![],
                single_item => vec![single_item.clone()], // Treat single item as collection
            };

            let taken_items: Vec<EvaluationResult> = items.into_iter().take(num_to_take).collect();

            // Return Empty or Collection
            if taken_items.is_empty() {
                EvaluationResult::Empty
            } else {
                EvaluationResult::Collection(taken_items)
            }
        }
        "intersect" => {
            // Returns the intersection of two collections (items present in both)
            if args.len() != 1 {
                return EvaluationResult::Empty; // Intersect requires exactly one argument
            }
            let other_collection = &args[0];

            // If either input is empty, the intersection is empty
            if invocation_base == &EvaluationResult::Empty
                || other_collection == &EvaluationResult::Empty
            {
                return EvaluationResult::Empty;
            }

            // Convert inputs to Vec for processing
            let left_items = match invocation_base {
                EvaluationResult::Collection(items) => items.clone(),
                single_item => vec![single_item.clone()],
            };

            let right_items = match other_collection {
                EvaluationResult::Collection(items) => items.clone(),
                single_item => vec![single_item.clone()],
            };

            let mut intersection_items = Vec::new();
            // Use HashSet for efficient duplicate checking in the result
            let mut added_items_set = HashSet::new();

            for left_item in &left_items {
                // Check if the left_item exists in the right_items (using equality '=')
                let exists_in_right = right_items
                    .iter()
                    .any(|right_item| compare_equality(left_item, "=", right_item).to_boolean());

                if exists_in_right {
                    // Attempt to insert the item into the HashSet.
                    // If insert returns true, it means the item was not already present.
                    if added_items_set.insert(left_item.clone()) {
                        intersection_items.push(left_item.clone());
                    }
                }
            }

            // Return Empty or Collection, do not apply singleton rule here
            if intersection_items.is_empty() {
                EvaluationResult::Empty
            } else {
                EvaluationResult::Collection(intersection_items)
            }
        }
        "exclude" => {
            // Returns items in invocation_base that are NOT in the argument collection
            if args.len() != 1 {
                return EvaluationResult::Empty; // Exclude requires exactly one argument
            }
            let other_collection = &args[0];

            // If invocation_base is empty, result is empty
            if invocation_base == &EvaluationResult::Empty {
                return EvaluationResult::Empty;
            }
            // If other_collection is empty, result is invocation_base
            if other_collection == &EvaluationResult::Empty {
                return invocation_base.clone();
            }

            // Convert inputs to Vec for processing
            let left_items = match invocation_base {
                EvaluationResult::Collection(items) => items.clone(),
                single_item => vec![single_item.clone()],
            };

            let right_items = match other_collection {
                EvaluationResult::Collection(items) => items.clone(),
                single_item => vec![single_item.clone()],
            };

            let mut result_items = Vec::new();
            for left_item in &left_items {
                // Check if the left_item exists in the right_items (using equality '=')
                let exists_in_right = right_items
                    .iter()
                    .any(|right_item| compare_equality(left_item, "=", right_item).to_boolean());

                // Keep the item if it does NOT exist in the right collection
                if !exists_in_right {
                    result_items.push(left_item.clone());
                }
            }

            // Return Empty or Collection, preserving duplicates and order
            if result_items.is_empty() {
                EvaluationResult::Empty
            } else {
                EvaluationResult::Collection(result_items)
            }
        }
        "union" => {
            // Returns the union of two collections (distinct items from both)
            if args.len() != 1 {
                return EvaluationResult::Empty; // Union requires exactly one argument
            }
            let other_collection = &args[0];

            // Convert inputs to Vec for processing
            let left_items = match invocation_base {
                EvaluationResult::Collection(items) => items.clone(),
                EvaluationResult::Empty => vec![],
                single_item => vec![single_item.clone()],
            };

            let right_items = match other_collection {
                EvaluationResult::Collection(items) => items.clone(),
                EvaluationResult::Empty => vec![],
                single_item => vec![single_item.clone()],
            };

            let mut union_items = Vec::new();
            // Use HashSet to track items already added to ensure uniqueness based on FHIRPath equality
            let mut added_items_set = HashSet::new();

            // Add items from the left collection if they haven't been added
            for item in left_items {
                if added_items_set.insert(item.clone()) {
                    union_items.push(item);
                }
            }

            // Add items from the right collection if they haven't been added
            for item in right_items {
                if added_items_set.insert(item.clone()) {
                    union_items.push(item);
                }
            }

            // Return Empty or Collection
            if union_items.is_empty() {
                EvaluationResult::Empty
            } else {
                EvaluationResult::Collection(union_items)
            }
        }
        "combine" => {
            // Returns a collection containing all items from both collections, including duplicates
            if args.len() != 1 {
                return EvaluationResult::Empty; // Combine requires exactly one argument
            }
            let other_collection = &args[0];

            // Convert inputs to Vec for processing
            let left_items = match invocation_base {
                EvaluationResult::Collection(items) => items.clone(),
                EvaluationResult::Empty => vec![],
                single_item => vec![single_item.clone()],
            };

            let right_items = match other_collection {
                EvaluationResult::Collection(items) => items.clone(),
                EvaluationResult::Empty => vec![],
                single_item => vec![single_item.clone()],
            };

            // Concatenate the two vectors
            let mut combined_items = left_items;
            combined_items.extend(right_items);

            // Return Empty or Collection
            if combined_items.is_empty() {
                EvaluationResult::Empty
            } else {
                EvaluationResult::Collection(combined_items)
            }
        }
        "convertsToDecimal" => {
            // Checks if the input can be converted to Decimal
            match invocation_base {
                EvaluationResult::Empty => EvaluationResult::Empty, // Empty input -> Empty result
                EvaluationResult::Collection(items) => {
                    // Only single-item collections can be converted
                    if items.len() == 1 {
                        // Recursively call convertsToDecimal on the single item
                        call_function("convertsToDecimal", &items[0], &[])
                    } else {
                        EvaluationResult::Boolean(false) // Multi-item collection cannot be converted
                    }
                }
                // Check convertibility for single items
                EvaluationResult::Boolean(_) => EvaluationResult::Boolean(true), // Booleans can convert (1.0 or 0.0)
                EvaluationResult::Integer(_) => EvaluationResult::Boolean(true), // Integers can convert
                EvaluationResult::Decimal(_) => EvaluationResult::Boolean(true), // Decimals can convert
                EvaluationResult::String(s) => {
                    // Check if the string parses to a Decimal
                    EvaluationResult::Boolean(s.parse::<Decimal>().is_ok())
                }
                // Other types are not convertible to Decimal
                _ => EvaluationResult::Boolean(false),
            }
        }
        "convertsToInteger" => {
            // Checks if the input can be converted to Integer
            match invocation_base {
                EvaluationResult::Empty => EvaluationResult::Empty, // Empty input -> Empty result
                EvaluationResult::Collection(items) => {
                    // Only single-item collections can be converted
                    if items.len() == 1 {
                        // Recursively call convertsToInteger on the single item
                        call_function("convertsToInteger", &items[0], &[])
                    } else {
                        EvaluationResult::Boolean(false) // Multi-item collection cannot be converted
                    }
                }
                // Check convertibility for single items
                EvaluationResult::Integer(_) => EvaluationResult::Boolean(true),
                EvaluationResult::String(s) => {
                    // Check if the string parses to an i64
                    EvaluationResult::Boolean(s.parse::<i64>().is_ok())
                }
                EvaluationResult::Boolean(_) => EvaluationResult::Boolean(true), // Booleans can convert (0 or 1)
                // Per FHIRPath spec, Decimal cannot be converted unless it has no fractional part
                EvaluationResult::Decimal(d) => {
                    EvaluationResult::Boolean(d.fract() == Decimal::ZERO)
                }
                // Other types are not convertible to Integer
                _ => EvaluationResult::Boolean(false),
            }
        }
        "convertsToBoolean" => {
            // Checks if the input can be converted to Boolean
            match invocation_base {
                EvaluationResult::Empty => EvaluationResult::Empty, // Empty input -> Empty result
                EvaluationResult::Collection(items) => {
                    // Only single-item collections can be converted
                    if items.len() == 1 {
                        // Recursively call convertsToBoolean on the single item
                        call_function("convertsToBoolean", &items[0], &[])
                    } else {
                        EvaluationResult::Boolean(false) // Multi-item collection cannot be converted
                    }
                }
                // Check convertibility for single items
                EvaluationResult::Boolean(_) => EvaluationResult::Boolean(true),
                EvaluationResult::Integer(i) => EvaluationResult::Boolean(*i == 0 || *i == 1),
                EvaluationResult::Decimal(d) => {
                    EvaluationResult::Boolean(d.is_zero() || *d == Decimal::ONE)
                }
                EvaluationResult::String(s) => {
                    let lower = s.to_lowercase();
                    EvaluationResult::Boolean(matches!(
                        lower.as_str(),
                        "true" | "t" | "yes" | "1" | "1.0" | "false" | "f" | "no" | "0" | "0.0"
                    ))
                }
                // Other types are not convertible to Boolean
                _ => EvaluationResult::Boolean(false),
            }
        }
        "toBoolean" => {
            // Converts the input to Boolean according to FHIRPath rules
            match invocation_base {
                EvaluationResult::Empty => EvaluationResult::Empty,
                EvaluationResult::Boolean(b) => EvaluationResult::Boolean(*b),
                EvaluationResult::Integer(i) => match i {
                    1 => EvaluationResult::Boolean(true),
                    0 => EvaluationResult::Boolean(false),
                    _ => EvaluationResult::Empty, // Other integers are not convertible
                },
                EvaluationResult::Decimal(d) => {
                    if *d == Decimal::ONE {
                        EvaluationResult::Boolean(true)
                    } else if d.is_zero() {
                        // Check for 0.0, -0.0 etc.
                        EvaluationResult::Boolean(false)
                    } else {
                        EvaluationResult::Empty // Other decimals are not convertible
                    }
                }
                EvaluationResult::String(s) => {
                    match s.to_lowercase().as_str() {
                        "true" | "t" | "yes" | "1" | "1.0" => EvaluationResult::Boolean(true),
                        "false" | "f" | "no" | "0" | "0.0" => EvaluationResult::Boolean(false),
                        _ => EvaluationResult::Empty, // Other strings are not convertible
                    }
                }
                // Collections: Convert single item, multiple items -> Empty
                EvaluationResult::Collection(items) => {
                    if items.len() == 1 {
                        // Recursively call toBoolean on the single item
                        call_function("toBoolean", &items[0], &[])
                    } else {
                        EvaluationResult::Empty // Multi-item or empty collection -> Empty
                    }
                }
                // Other types are not convertible
                _ => EvaluationResult::Empty,
            }
        }
        "convertsToString" => {
            // Checks if the input can be converted to String
            match invocation_base {
                EvaluationResult::Empty => EvaluationResult::Empty, // Empty input -> Empty result
                EvaluationResult::Collection(items) => {
                    // Only single-item collections can be converted
                    if items.len() == 1 {
                        // Recursively call convertsToString on the single item
                        call_function("convertsToString", &items[0], &[])
                    } else {
                        EvaluationResult::Boolean(false) // Multi-item collection cannot be converted
                    }
                }
                // Check convertibility for single items (most primitives can be)
                EvaluationResult::Boolean(_)
                | EvaluationResult::String(_)
                | EvaluationResult::Integer(_)
                | EvaluationResult::Decimal(_)
                | EvaluationResult::Date(_)
                | EvaluationResult::DateTime(_)
                | EvaluationResult::Time(_) => EvaluationResult::Boolean(true),
                // Objects are not convertible to String via this function
                EvaluationResult::Object(_) => EvaluationResult::Boolean(false),
            }
        }
        "toString" => {
            // Converts the input to its string representation using the helper
            // Handles single items, collections (returning Empty for multi-item), and Empty correctly.
            let string_val = invocation_base.to_string_value();
            // Check if the helper returned the collection representation "[...]" which means Empty for toString
            if string_val.starts_with('[')
                && string_val.ends_with(']')
                && invocation_base.is_collection()
                && invocation_base.count() != 1
            {
                EvaluationResult::Empty
            } else if string_val.is_empty()
                && invocation_base != &EvaluationResult::String("".to_string())
            {
                // If the string is empty, but the original wasn't an empty string, return Empty
                // This handles the case where to_string_value returns "" for Empty input.
                EvaluationResult::Empty
            } else {
                EvaluationResult::String(string_val)
            }
        }
        "toDate" => {
            // Converts the input to Date according to FHIRPath rules
            match invocation_base {
                EvaluationResult::Empty => EvaluationResult::Empty,
                EvaluationResult::Date(d) => EvaluationResult::Date(d.clone()),
                EvaluationResult::DateTime(dt) => {
                    // Extract the date part
                    if let Some(date_part) = dt.split('T').next() {
                        EvaluationResult::Date(date_part.to_string())
                    } else {
                        EvaluationResult::Empty // Should not happen if DateTime format is valid
                    }
                }
                EvaluationResult::String(s) => {
                    // Attempt to parse as Date or DateTime and extract date part
                    // This requires a robust date/datetime parsing logic
                    // For now, assume valid FHIR date/datetime strings
                    if s.contains('T') {
                        // Looks like DateTime
                        if let Some(date_part) = s.split('T').next() {
                            // Basic validation: check if date_part looks like YYYY, YYYY-MM, or YYYY-MM-DD
                            if date_part.len() == 4 || date_part.len() == 7 || date_part.len() == 10
                            {
                                EvaluationResult::Date(date_part.to_string())
                            } else {
                                EvaluationResult::Empty
                            }
                        } else {
                            EvaluationResult::Empty
                        }
                    } else {
                        // Looks like Date
                        // Basic validation
                        if s.len() == 4 || s.len() == 7 || s.len() == 10 {
                            EvaluationResult::Date(s.clone())
                        } else {
                            EvaluationResult::Empty
                        }
                    }
                }
                // Collections: Convert single item, multiple items -> Empty
                EvaluationResult::Collection(items) => {
                    if items.len() == 1 {
                        call_function("toDate", &items[0], &[])
                    } else {
                        EvaluationResult::Empty
                    }
                }
                _ => EvaluationResult::Empty, // Other types cannot convert
            }
        }
        "convertsToDate" => {
            // Checks if the input can be converted to Date
            match invocation_base {
                EvaluationResult::Empty => EvaluationResult::Empty,
                EvaluationResult::Collection(items) => {
                    if items.len() == 1 {
                        call_function("convertsToDate", &items[0], &[])
                    } else {
                        EvaluationResult::Boolean(false)
                    }
                }
                EvaluationResult::Date(_) => EvaluationResult::Boolean(true),
                EvaluationResult::DateTime(_) => EvaluationResult::Boolean(true), // Can extract date part
                EvaluationResult::String(s) => {
                    // Basic check: Does it look like YYYY, YYYY-MM, YYYY-MM-DD, or start like a DateTime?
                    let is_date_like = s.len() == 4 || s.len() == 7 || s.len() == 10;
                    let is_datetime_like = s.contains('T')
                        && (s.starts_with(|c: char| c.is_ascii_digit()) && s.len() >= 5); // Basic check
                    EvaluationResult::Boolean(is_date_like || is_datetime_like)
                }
                _ => EvaluationResult::Boolean(false),
            }
        }
        "toDateTime" => {
            // Converts the input to DateTime according to FHIRPath rules
            match invocation_base {
                EvaluationResult::Empty => EvaluationResult::Empty,
                EvaluationResult::DateTime(dt) => EvaluationResult::DateTime(dt.clone()),
                EvaluationResult::Date(d) => EvaluationResult::DateTime(d.clone()), // Date becomes DateTime (no time part)
                EvaluationResult::String(s) => {
                    // Basic check: Does it look like YYYY, YYYY-MM, YYYY-MM-DD, or YYYY-MM-DDTHH...?
                    let is_date_like = s.len() == 4 || s.len() == 7 || s.len() == 10;
                    let is_datetime_like =
                        s.contains('T') && s.starts_with(|c: char| c.is_ascii_digit());
                    if is_date_like || is_datetime_like {
                        EvaluationResult::DateTime(s.clone())
                    } else {
                        EvaluationResult::Empty
                    }
                }
                // Collections: Convert single item, multiple items -> Empty
                EvaluationResult::Collection(items) => {
                    if items.len() == 1 {
                        call_function("toDateTime", &items[0], &[])
                    } else {
                        EvaluationResult::Empty
                    }
                }
                _ => EvaluationResult::Empty, // Other types cannot convert
            }
        }
        "convertsToDateTime" => {
            // Checks if the input can be converted to DateTime
            match invocation_base {
                EvaluationResult::Empty => EvaluationResult::Empty,
                EvaluationResult::Collection(items) => {
                    if items.len() == 1 {
                        call_function("convertsToDateTime", &items[0], &[])
                    } else {
                        EvaluationResult::Boolean(false)
                    }
                }
                EvaluationResult::DateTime(_) => EvaluationResult::Boolean(true),
                EvaluationResult::Date(_) => EvaluationResult::Boolean(true), // Can represent as DateTime
                EvaluationResult::String(s) => {
                    // Basic check: Does it look like YYYY, YYYY-MM, YYYY-MM-DD, or YYYY-MM-DDTHH...?
                    let is_date_like = s.len() == 4 || s.len() == 7 || s.len() == 10;
                    let is_datetime_like =
                        s.contains('T') && s.starts_with(|c: char| c.is_ascii_digit());
                    EvaluationResult::Boolean(is_date_like || is_datetime_like)
                }
                _ => EvaluationResult::Boolean(false),
            }
        }
        "toTime" => {
            // Converts the input to Time according to FHIRPath rules
            match invocation_base {
                EvaluationResult::Empty => EvaluationResult::Empty,
                EvaluationResult::Time(t) => EvaluationResult::Time(t.clone()),
                EvaluationResult::String(s) => {
                    // Basic check: Does it look like HH, HH:mm, HH:mm:ss, HH:mm:ss.sss?
                    let parts: Vec<&str> = s.split(':').collect();
                    let is_time_like = match parts.len() {
                        1 => parts[0].len() == 2 && parts[0].chars().all(|c| c.is_ascii_digit()),
                        2 => {
                            parts[0].len() == 2
                                && parts[1].len() == 2
                                && parts.iter().all(|p| p.chars().all(|c| c.is_ascii_digit()))
                        }
                        3 => {
                            parts[0].len() == 2
                                && parts[1].len() == 2
                                && parts[2].len() >= 2
                                && parts[2]
                                    .split('.')
                                    .next()
                                    .map_or(false, |sec| sec.len() == 2)
                                && parts
                                    .iter()
                                    .all(|p| p.chars().all(|c| c.is_ascii_digit() || c == '.'))
                        }
                        _ => false,
                    };
                    if is_time_like {
                        EvaluationResult::Time(s.clone())
                    } else {
                        EvaluationResult::Empty
                    }
                }
                // Collections: Convert single item, multiple items -> Empty
                EvaluationResult::Collection(items) => {
                    if items.len() == 1 {
                        call_function("toTime", &items[0], &[])
                    } else {
                        EvaluationResult::Empty
                    }
                }
                _ => EvaluationResult::Empty, // Other types cannot convert
            }
        }
        "convertsToTime" => {
            // Checks if the input can be converted to Time
            match invocation_base {
                EvaluationResult::Empty => EvaluationResult::Empty,
                EvaluationResult::Collection(items) => {
                    if items.len() == 1 {
                        call_function("convertsToTime", &items[0], &[])
                    } else {
                        EvaluationResult::Boolean(false)
                    }
                }
                EvaluationResult::Time(_) => EvaluationResult::Boolean(true),
                EvaluationResult::String(s) => {
                    // Basic check (same as toTime)
                    let parts: Vec<&str> = s.split(':').collect();
                    let is_time_like = match parts.len() {
                        1 => parts[0].len() == 2 && parts[0].chars().all(|c| c.is_ascii_digit()),
                        2 => {
                            parts[0].len() == 2
                                && parts[1].len() == 2
                                && parts.iter().all(|p| p.chars().all(|c| c.is_ascii_digit()))
                        }
                        3 => {
                            parts[0].len() == 2
                                && parts[1].len() == 2
                                && parts[2].len() >= 2
                                && parts[2]
                                    .split('.')
                                    .next()
                                    .map_or(false, |sec| sec.len() == 2)
                                && parts
                                    .iter()
                                    .all(|p| p.chars().all(|c| c.is_ascii_digit() || c == '.'))
                        }
                        _ => false,
                    };
                    EvaluationResult::Boolean(is_time_like)
                }
                _ => EvaluationResult::Boolean(false),
            }
        }
        "toQuantity" => {
            // Converts the input to Quantity according to FHIRPath rules
            // The result is just the numeric value (Decimal or Integer) as unit handling is complex
            match invocation_base {
                EvaluationResult::Empty => EvaluationResult::Empty,
                EvaluationResult::Boolean(b) => {
                    EvaluationResult::Decimal(if *b { Decimal::ONE } else { Decimal::ZERO })
                } // Convert to 1.0 or 0.0
                EvaluationResult::Integer(i) => EvaluationResult::Decimal(Decimal::from(*i)), // Convert to Decimal with '1' unit implicitly
                EvaluationResult::Decimal(d) => EvaluationResult::Decimal(*d), // Convert to Decimal with '1' unit implicitly
                EvaluationResult::String(s) => {
                    // Attempt to parse as "value unit" or just "value"
                    let parts: Vec<&str> = s.split_whitespace().collect();
                    if parts.is_empty() {
                        EvaluationResult::Empty // Empty string cannot convert
                    } else if parts.len() == 1 {
                        // Only a value part, try parsing it
                        parts[0]
                            .parse::<Decimal>()
                            .map(EvaluationResult::Decimal)
                            .unwrap_or(EvaluationResult::Empty)
                    } else if parts.len() == 2 {
                        // Value and unit parts
                        let value_part = parts[0];
                        let unit_part = parts[1];
                        // Try parsing the value part
                        if let Ok(decimal_value) = value_part.parse::<Decimal>() {
                            // Check if the unit part is valid
                            if is_valid_fhirpath_quantity_unit(unit_part) {
                                EvaluationResult::Decimal(decimal_value)
                            } else {
                                EvaluationResult::Empty // Invalid unit
                            }
                        } else {
                            EvaluationResult::Empty // Invalid value part
                        }
                    } else {
                        // More than two parts is invalid format
                        EvaluationResult::Empty
                    }
                }
                // Collections: Convert single item, multiple items -> Empty
                EvaluationResult::Collection(items) => {
                    if items.len() == 1 {
                        call_function("toQuantity", &items[0], &[])
                    } else {
                        EvaluationResult::Empty
                    }
                }
                _ => EvaluationResult::Empty, // Other types cannot convert
            }
        }
        "convertsToQuantity" => {
            // Checks if the input can be converted to Quantity
            match invocation_base {
                EvaluationResult::Empty => EvaluationResult::Empty,
                EvaluationResult::Collection(items) => {
                    if items.len() == 1 {
                        call_function("convertsToQuantity", &items[0], &[])
                    } else {
                        EvaluationResult::Boolean(false)
                    }
                }
                EvaluationResult::Boolean(_) => EvaluationResult::Boolean(true),
                EvaluationResult::Integer(_) => EvaluationResult::Boolean(true),
                EvaluationResult::Decimal(_) => EvaluationResult::Boolean(true),
                EvaluationResult::String(s) => EvaluationResult::Boolean({
                    // Wrap the entire block
                    // Check if the string represents a valid quantity format
                    let parts: Vec<&str> = s.split_whitespace().collect();
                    if parts.is_empty() {
                        false // Empty string
                    } else if parts.len() == 1 {
                        // Only a value part, check if it parses as Decimal
                        parts[0].parse::<Decimal>().is_ok()
                    } else if parts.len() == 2 {
                        // Value and unit parts, check both
                        let value_parses = parts[0].parse::<Decimal>().is_ok();
                        let unit_is_valid = is_valid_fhirpath_quantity_unit(parts[1]);
                        value_parses && unit_is_valid
                    } else {
                        // More than two parts is invalid
                        false
                    }
                }), // Close the EvaluationResult::Boolean wrapper
                _ => EvaluationResult::Boolean(false),
            }
        }
        "length" => {
            // Returns the length of a string
            match invocation_base {
                EvaluationResult::String(s) => EvaluationResult::Integer(s.chars().count() as i64), // Use chars().count() for correct length
                _ => EvaluationResult::Empty, // Length only defined for strings
            }
        }
        "indexOf" => {
            // Returns the 0-based index of the first occurrence of the substring
            if args.len() != 1 {
                return EvaluationResult::Empty;
            }
            match (invocation_base, &args[0]) {
                (EvaluationResult::String(s), EvaluationResult::String(substring)) => {
                    match s.find(substring) {
                        Some(index) => EvaluationResult::Integer(index as i64),
                        None => EvaluationResult::Integer(-1),
                    }
                }
                _ => EvaluationResult::Empty, // Invalid types or empty base/arg
            }
        }
        "substring" => {
            // Returns a part of the string
            if args.is_empty() || args.len() > 2 {
                return EvaluationResult::Empty;
            } // Needs 1 or 2 args
            let start_index_res = &args[0];
            let length_res_opt = args.get(1);

            match invocation_base {
                EvaluationResult::String(s) => {
                    let start_index = match start_index_res {
                        EvaluationResult::Integer(i) if *i >= 0 => *i as usize,
                        _ => return EvaluationResult::Empty, // Invalid start index type or negative
                    };

                    // If start index is out of bounds (>= length), return empty string
                    if start_index >= s.chars().count() {
                        return EvaluationResult::String("".to_string());
                    }

                    if let Some(length_res) = length_res_opt {
                        // Two arguments: start and length
                        let length = match length_res {
                            // If length is negative, return empty string per spec
                            EvaluationResult::Integer(l) if *l < 0 => {
                                return EvaluationResult::String("".to_string());
                            }
                            // If length is non-negative integer, use it
                            EvaluationResult::Integer(l) => *l as usize,
                            // Any other type for length is invalid
                            _ => return EvaluationResult::Empty,
                        };

                        let result: String = s.chars().skip(start_index).take(length).collect();
                        EvaluationResult::String(result)
                    } else {
                        // One argument: start index only (substring to end)
                        let result: String = s.chars().skip(start_index).collect();
                        EvaluationResult::String(result)
                    }
                }
                _ => EvaluationResult::Empty, // Substring only defined for strings
            }
        }
        "startsWith" => {
            if args.len() != 1 {
                return EvaluationResult::Empty;
            }
            match (invocation_base, &args[0]) {
                (EvaluationResult::String(s), EvaluationResult::String(prefix)) => {
                    EvaluationResult::Boolean(s.starts_with(prefix))
                }
                _ => EvaluationResult::Empty,
            }
        }
        "endsWith" => {
            if args.len() != 1 {
                return EvaluationResult::Empty;
            }
            match (invocation_base, &args[0]) {
                (EvaluationResult::String(s), EvaluationResult::String(suffix)) => {
                    EvaluationResult::Boolean(s.ends_with(suffix))
                }
                _ => EvaluationResult::Empty,
            }
        }
        "upper" => match invocation_base {
            EvaluationResult::String(s) => EvaluationResult::String(s.to_uppercase()),
            _ => EvaluationResult::Empty,
        },
        "lower" => match invocation_base {
            EvaluationResult::String(s) => EvaluationResult::String(s.to_lowercase()),
            _ => EvaluationResult::Empty,
        },
        "replace" => {
            if args.len() != 2 {
                return EvaluationResult::Empty;
            }
            match (invocation_base, &args[0], &args[1]) {
                (
                    EvaluationResult::String(s),
                    EvaluationResult::String(pattern),
                    EvaluationResult::String(substitution),
                ) => EvaluationResult::String(s.replace(pattern, substitution)),
                _ => EvaluationResult::Empty,
            }
        }
        "matches" => {
            if args.len() != 1 {
                return EvaluationResult::Empty;
            }
            match (invocation_base, &args[0]) {
                (EvaluationResult::String(s), EvaluationResult::String(regex_pattern)) => {
                    match Regex::new(regex_pattern) {
                        Ok(re) => EvaluationResult::Boolean(re.is_match(s)),
                        Err(_) => EvaluationResult::Empty, // Invalid regex pattern
                    }
                }
                _ => EvaluationResult::Empty,
            }
        }
        "replaceMatches" => {
            if args.len() != 2 {
                return EvaluationResult::Empty;
            }
            match (invocation_base, &args[0], &args[1]) {
                (
                    EvaluationResult::String(s),
                    EvaluationResult::String(regex_pattern),
                    EvaluationResult::String(substitution),
                ) => {
                    match Regex::new(regex_pattern) {
                        Ok(re) => {
                            EvaluationResult::String(re.replace_all(s, substitution).to_string())
                        }
                        Err(_) => EvaluationResult::Empty, // Invalid regex pattern
                    }
                }
                _ => EvaluationResult::Empty,
            }
        }
        "toChars" => match invocation_base {
            EvaluationResult::String(s) => {
                if s.is_empty() {
                    EvaluationResult::Empty
                } else {
                    let chars: Vec<EvaluationResult> = s
                        .chars()
                        .map(|c| EvaluationResult::String(c.to_string()))
                        .collect();
                    EvaluationResult::Collection(chars)
                }
            }
            _ => EvaluationResult::Empty,
        },
        "now" => {
            // Returns the current DateTime
            let now = Local::now();
            // Format according to FHIRPath spec (ISO 8601 with timezone offset)
            EvaluationResult::DateTime(now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        }
        "today" => {
            // Returns the current Date
            let today = Local::now().date_naive();
            // Format as YYYY-MM-DD
            EvaluationResult::Date(today.format("%Y-%m-%d").to_string())
        }
        "timeOfDay" => {
            // Returns the current Time
            let now = Local::now();
            // Format as HH:mm:ss.sss (using Millis for consistency with now())
            EvaluationResult::Time(format!(
                "{:02}:{:02}:{:02}.{:03}",
                now.hour(),
                now.minute(),
                now.second(),
                now.nanosecond() / 1_000_000 // Convert nanoseconds to milliseconds
            ))
        }
        // where, select, ofType are handled in evaluate_invocation
        // Add other standard functions here
        _ => {
            // Only print warning for functions not handled elsewhere
            // Added conversion functions and now/today/timeOfDay to the list
            let handled_functions = [
                "where",
                "select",
                "exists",
                "all",
                "iif",
                "ofType",
                "toBoolean",
                "convertsToBoolean",
                "toInteger",
                "convertsToInteger",
                "toDecimal",
                "convertsToDecimal",
                "toString",
                "convertsToString",
                "toDate",
                "convertsToDate",
                "toDateTime",
                "convertsToDateTime",
                "toTime",
                "convertsToTime",
                "toQuantity",
                "convertsToQuantity",
                // Add other handled functions here
                "count",
                "empty",
                "first",
                "last",
                "not",
                "contains",
                "isDistinct",
                "distinct",
                "skip",
                "tail",
                "take",
                "intersect",
                "exclude",
                "union",
                "combine",
                "length",
                "indexOf",
                "substring",
                "startsWith",
                "endsWith",
                "upper",
                "lower",
                "replace",
                "matches",
                "replaceMatches",
                "toChars",
                "now",
                "today",
                "timeOfDay",
            ];
            if !handled_functions.contains(&name) {
                eprintln!("Warning: Unsupported function called: {}", name); // Keep this warning for truly unhandled functions
            }
            EvaluationResult::Empty
        }
    }
}

/// Checks if a string is a valid FHIRPath quantity unit (UCUM or time-based).
/// Note: This is a simplified check. A full UCUM validator is complex.
fn is_valid_fhirpath_quantity_unit(unit: &str) -> bool {
    // Allow known time-based units
    const TIME_UNITS: &[&str] = &[
        "year",
        "month",
        "week",
        "day",
        "hour",
        "minute",
        "second",
        "millisecond",
        "years",
        "months",
        "weeks",
        "days",
        "hours",
        "minutes",
        "seconds",
        "milliseconds",
    ];
    if TIME_UNITS.contains(&unit) {
        return true;
    }

    // Basic check for UCUM units (starts with non-digit, doesn't contain invalid chars like spaces after first char)
    // This is NOT a full UCUM validation.
    if unit.is_empty() {
        return false;
    }
    let first_char = unit.chars().next().unwrap();
    // UCUM units often start with letters or symbols like '{', '[', '(', etc.
    // They generally don't start with digits.
    if first_char.is_ascii_digit() {
        return false;
    }
    // Check for invalid characters (e.g., whitespace within the unit)
    if unit.chars().skip(1).any(|c| c.is_whitespace()) {
        return false;
    }

    // Stricter check: Allow only alphanumeric, '.', '/', '{', '}', '[', ']', '(', ')', '%'
    // This is still a simplification of full UCUM validation.
    let is_potentially_ucum = unit.chars().all(|c| {
        c.is_ascii_alphanumeric()
            || matches!(c, '.' | '/' | '{' | '}' | '[' | ']' | '(' | ')' | '%')
    });

    is_potentially_ucum // Return true only if it's a time unit or passes the stricter UCUM check
}

/// Evaluates an indexer expression
fn evaluate_indexer(collection: &EvaluationResult, index: &EvaluationResult) -> EvaluationResult {
    // Get the index as an integer, ensuring it's non-negative
    let idx_opt: Option<usize> = match index {
        EvaluationResult::Integer(i) => {
            if *i >= 0 {
                (*i).try_into().ok() // Convert non-negative i64 to usize
            } else {
                None // Negative index is invalid
            }
        }
        EvaluationResult::Decimal(d) => {
            // Check if decimal is a non-negative integer before converting
            if d.is_integer() && d.is_sign_positive() {
                d.to_usize() // Convert non-negative integer Decimal to usize
            } else {
                None // Non-integer or negative decimal is invalid
            }
        }
        _ => None, // Non-numeric index is invalid
    };

    let idx = match idx_opt {
        Some(i) => i,
        None => return EvaluationResult::Empty, // Invalid index results in Empty
    };

    // Access the item at the given index
    match collection {
        EvaluationResult::Collection(items) => {
            items.get(idx).cloned().unwrap_or(EvaluationResult::Empty)
        }
        _ => EvaluationResult::Empty,
    }
}

/// Applies a polarity operator to a value
fn apply_polarity(op: char, value: &EvaluationResult) -> EvaluationResult {
    match op {
        '+' => value.clone(), // Unary plus doesn't change the value
        '-' => {
            // Negate numeric values
            match value {
                EvaluationResult::Decimal(d) => EvaluationResult::Decimal(-*d),
                EvaluationResult::Integer(i) => EvaluationResult::Integer(-*i),
                _ => EvaluationResult::Empty,
            }
        }
        _ => EvaluationResult::Empty,
    }
}

/// Applies a multiplicative operator to two values
fn apply_multiplicative(
    left: &EvaluationResult,
    op: &str,
    right: &EvaluationResult,
) -> EvaluationResult {
    match op {
        "*" => {
            // Handle multiplication: Int * Int = Int, otherwise Decimal
            match (left, right) {
                (EvaluationResult::Integer(l), EvaluationResult::Integer(r)) => {
                    // Check for potential overflow before multiplying
                    l.checked_mul(*r)
                        .map(EvaluationResult::Integer)
                        .unwrap_or(EvaluationResult::Empty) // Return Empty on overflow
                }
                (EvaluationResult::Decimal(l), EvaluationResult::Decimal(r)) => {
                    EvaluationResult::Decimal(*l * *r)
                }
                (EvaluationResult::Decimal(l), EvaluationResult::Integer(r)) => {
                    EvaluationResult::Decimal(*l * Decimal::from(*r))
                }
                (EvaluationResult::Integer(l), EvaluationResult::Decimal(r)) => {
                    EvaluationResult::Decimal(Decimal::from(*l) * *r)
                }
                _ => EvaluationResult::Empty, // Invalid types for multiplication
            }
        }
        "/" => {
            // Handle division: Always results in Decimal
            let left_dec = match left {
                EvaluationResult::Decimal(d) => Some(*d),
                EvaluationResult::Integer(i) => Some(Decimal::from(*i)),
                _ => None,
            };
            let right_dec = match right {
                EvaluationResult::Decimal(d) => Some(*d),
                EvaluationResult::Integer(i) => Some(Decimal::from(*i)),
                _ => None,
            };

            if let (Some(l), Some(r)) = (left_dec, right_dec) {
                if r.is_zero() {
                    EvaluationResult::Empty // Division by zero
                } else {
                    // Decimal division preserves precision
                    l.checked_div(r)
                        .map(EvaluationResult::Decimal) // Keep only one map
                        .unwrap_or(EvaluationResult::Empty) // Handle potential overflow/errors
                }
                // Removed the incorrect inner else block
            } else {
                EvaluationResult::Empty // Invalid types for division (this else is correct)
            }
        }
        "div" | "mod" => {
            // Handle div/mod: Int/Int -> Int, Dec/Dec -> Int/Dec, mixed -> Empty
            match (left, right) {
                (EvaluationResult::Integer(l), EvaluationResult::Integer(r)) => {
                    apply_integer_multiplicative(*l, op, *r)
                }
                (EvaluationResult::Decimal(l), EvaluationResult::Decimal(r)) => {
                    apply_decimal_multiplicative(*l, op, *r) // Need helper for Decimal div/mod
                }
                _ => EvaluationResult::Empty, // Mixed types are invalid for div/mod
            }
        }
        _ => EvaluationResult::Empty, // Unknown operator
    }
}

/// Applies integer-only multiplicative operators (div, mod)
fn apply_integer_multiplicative(left: i64, op: &str, right: i64) -> EvaluationResult {
    if right == 0 {
        return EvaluationResult::Empty; // Division/Modulo by zero
    }
    match op {
        "div" => EvaluationResult::Integer(left / right), // Integer division
        "mod" => EvaluationResult::Integer(left % right), // Integer modulo
        _ => EvaluationResult::Empty,                     // Should not happen if called correctly
    }
}

/// Applies an additive operator to two values
fn apply_additive(left: &EvaluationResult, op: &str, right: &EvaluationResult) -> EvaluationResult {
    // Promote Integer to Decimal for numeric operations
    let left_dec = match left {
        EvaluationResult::Decimal(d) => Some(*d),
        EvaluationResult::Integer(i) => Some(Decimal::from(*i)),
        _ => None,
    };
    let right_dec = match right {
        EvaluationResult::Decimal(d) => Some(*d),
        EvaluationResult::Integer(i) => Some(Decimal::from(*i)),
        _ => None,
    };

    match op {
        "+" => {
            // Handle numeric addition: Int + Int = Int, otherwise Decimal
            match (left, right) {
                (EvaluationResult::Integer(l), EvaluationResult::Integer(r)) => {
                    // Check for potential overflow before adding
                    l.checked_add(*r)
                        .map(EvaluationResult::Integer)
                        .unwrap_or(EvaluationResult::Empty) // Return Empty on overflow
                }
                // If either operand is Decimal, promote and result is Decimal
                (EvaluationResult::Decimal(l), EvaluationResult::Decimal(r)) => {
                    EvaluationResult::Decimal(*l + *r)
                }
                (EvaluationResult::Decimal(l), EvaluationResult::Integer(r)) => {
                    EvaluationResult::Decimal(*l + Decimal::from(*r))
                }
                (EvaluationResult::Integer(l), EvaluationResult::Decimal(r)) => {
                    EvaluationResult::Decimal(Decimal::from(*l) + *r)
                }
                // Handle string concatenation with '+'
                (EvaluationResult::String(l), EvaluationResult::String(r)) => {
                    EvaluationResult::String(format!("{}{}", l, r))
                }
                // Other combinations are invalid for '+'
                _ => EvaluationResult::Empty,
            }
        }
        // Removed duplicate "-" arm
        "-" => {
            // Handle numeric subtraction (always results in Decimal)
            if let (Some(l), Some(r)) = (left_dec, right_dec) {
                EvaluationResult::Decimal(l - r)
            } else {
                // Subtraction is only defined for numeric types or if promotion failed
                EvaluationResult::Empty
            }
        }
        "&" => {
            // Handle string concatenation using '&'
            let left_str = left.to_string_value(); // Convert left to string
            let right_str = right.to_string_value(); // Convert right to string
            EvaluationResult::String(format!("{}{}", left_str, right_str))
        }
        _ => EvaluationResult::Empty, // Unknown operator
    }
}

/// Applies a type operation (is/as) to a value
fn apply_type_operation(
    value: &EvaluationResult,
    op: &str,
    type_spec: &TypeSpecifier,
) -> EvaluationResult {
    match op {
        "is" => {
            // Check if the value is of the specified type
            let type_name = match type_spec {
                TypeSpecifier::QualifiedIdentifier(name, _) => name,
            };

            let is_type = match (type_name.as_str(), value) {
                ("Boolean", EvaluationResult::Boolean(_)) => true,
                ("String", EvaluationResult::String(_)) => true,
                ("Integer", EvaluationResult::Integer(_)) => true,
                ("Decimal", EvaluationResult::Decimal(_)) => true, // Check for Decimal
                ("Date", EvaluationResult::Date(_)) => true,
                ("DateTime", EvaluationResult::DateTime(_)) => true,
                ("Time", EvaluationResult::Time(_)) => true,
                // Add more type checks as needed
                _ => false,
            };

            EvaluationResult::Boolean(is_type)
        }
        "as" => {
            // Cast the value to the specified type if possible
            let type_name = match type_spec {
                TypeSpecifier::QualifiedIdentifier(name, _) => name,
            };

            match (type_name.as_str(), value) {
                // For now, we just return the value if it's already of the right type
                // In a full implementation, we would attempt to convert between types
                // In a full implementation, we would attempt to convert between types
                // e.g., Integer to Decimal, String to Decimal, etc.
                ("Boolean", EvaluationResult::Boolean(_)) => value.clone(),
                ("String", EvaluationResult::String(_)) => value.clone(),
                ("Integer", EvaluationResult::Integer(_)) => value.clone(),
                ("Decimal", EvaluationResult::Decimal(_)) => value.clone(), // Return if already Decimal
                ("Date", EvaluationResult::Date(_)) => value.clone(),
                ("DateTime", EvaluationResult::DateTime(_)) => value.clone(),
                ("Time", EvaluationResult::Time(_)) => value.clone(),
                // Add more type conversions as needed
                _ => EvaluationResult::Empty,
            }
        }
        _ => EvaluationResult::Empty,
    }
}

/// Combines two collections into a union
fn union_collections(left: &EvaluationResult, right: &EvaluationResult) -> EvaluationResult {
    let left_items = match left {
        EvaluationResult::Collection(items) => items.clone(),
        EvaluationResult::Empty => vec![],
        _ => vec![left.clone()],
    };

    let right_items = match right {
        EvaluationResult::Collection(items) => items.clone(),
        EvaluationResult::Empty => vec![],
        _ => vec![right.clone()],
    };

    let mut result = left_items;
    result.extend(right_items);

    // Return Empty or Collection, do not apply singleton rule here
    if result.is_empty() {
        EvaluationResult::Empty
    } else {
        EvaluationResult::Collection(result)
    }
}

/// Compares two values for inequality
fn compare_inequality(
    left: &EvaluationResult,
    op: &str,
    right: &EvaluationResult,
) -> EvaluationResult {
    // Promote Integer to Decimal for mixed comparisons
    let compare_result = match (left, right) {
        // Both Decimal
        (EvaluationResult::Decimal(l), EvaluationResult::Decimal(r)) => Some(l.cmp(r)),
        // Both Integer
        (EvaluationResult::Integer(l), EvaluationResult::Integer(r)) => Some(l.cmp(r)),
        // Mixed Decimal/Integer
        (EvaluationResult::Decimal(l), EvaluationResult::Integer(r)) => {
            Some(l.cmp(&Decimal::from(*r)))
        }
        (EvaluationResult::Integer(l), EvaluationResult::Decimal(r)) => {
            Some(Decimal::from(*l).cmp(r))
        }
        // String comparison
        (EvaluationResult::String(l), EvaluationResult::String(r)) => Some(l.cmp(r)),
        // Date comparison
        (EvaluationResult::Date(l), EvaluationResult::Date(r)) => Some(l.cmp(r)),
        // DateTime comparison
        (EvaluationResult::DateTime(l), EvaluationResult::DateTime(r)) => Some(l.cmp(r)),
        // Time comparison
        (EvaluationResult::Time(l), EvaluationResult::Time(r)) => Some(l.cmp(r)),
        // Incomparable types
        _ => None,
    };

    if let Some(ordering) = compare_result {
        let result = match op {
            "<" => ordering.is_lt(),
            "<=" => ordering.is_le(),
            ">" => ordering.is_gt(),
            ">=" => ordering.is_ge(),
            _ => false, // Should not happen
        };
        EvaluationResult::Boolean(result)
    } else {
        EvaluationResult::Empty // Incomparable types result in Empty
    }
}

/// Compares two values for equality
fn compare_equality(
    left: &EvaluationResult,
    op: &str,
    right: &EvaluationResult,
) -> EvaluationResult {
    match op {
        "=" => {
            // Strict equality
            let result = match (left, right) {
                (EvaluationResult::Empty, EvaluationResult::Empty) => true, // Empty equals Empty
                (EvaluationResult::Boolean(l), EvaluationResult::Boolean(r)) => l == r,
                (EvaluationResult::String(l), EvaluationResult::String(r)) => l == r,
                (EvaluationResult::Decimal(l), EvaluationResult::Decimal(r)) => l == r,
                (EvaluationResult::Integer(l), EvaluationResult::Integer(r)) => l == r,
                // Mixed number/integer comparison
                (EvaluationResult::Decimal(l), EvaluationResult::Integer(r)) => {
                    *l == Decimal::from(*r)
                }
                (EvaluationResult::Integer(l), EvaluationResult::Decimal(r)) => {
                    Decimal::from(*l) == *r
                }
                (EvaluationResult::Date(l), EvaluationResult::Date(r)) => l == r,
                (EvaluationResult::DateTime(l), EvaluationResult::DateTime(r)) => l == r,
                (EvaluationResult::Time(l), EvaluationResult::Time(r)) => l == r,
                _ => false,
            };
            EvaluationResult::Boolean(result)
        }
        "!=" => {
            // Strict inequality
            let result = match (left, right) {
                (EvaluationResult::Empty, EvaluationResult::Empty) => false, // Empty equals Empty
                (EvaluationResult::Boolean(l), EvaluationResult::Boolean(r)) => l != r,
                (EvaluationResult::String(l), EvaluationResult::String(r)) => l != r,
                (EvaluationResult::Decimal(l), EvaluationResult::Decimal(r)) => l != r,
                (EvaluationResult::Integer(l), EvaluationResult::Integer(r)) => l != r,
                // Mixed number/integer comparison
                (EvaluationResult::Decimal(l), EvaluationResult::Integer(r)) => {
                    *l != Decimal::from(*r)
                }
                (EvaluationResult::Integer(l), EvaluationResult::Decimal(r)) => {
                    Decimal::from(*l) != *r
                }
                (EvaluationResult::Date(l), EvaluationResult::Date(r)) => l != r,
                (EvaluationResult::DateTime(l), EvaluationResult::DateTime(r)) => l != r,
                (EvaluationResult::Time(l), EvaluationResult::Time(r)) => l != r,
                _ => true,
            };
            EvaluationResult::Boolean(result)
        }
        "~" => {
            // Equivalence (case-insensitive for strings)
            let result = match (left, right) {
                (EvaluationResult::String(l), EvaluationResult::String(r)) => {
                    l.to_lowercase() == r.to_lowercase()
                }
                _ => compare_equality(left, "=", right).to_boolean(),
            };
            EvaluationResult::Boolean(result)
        }
        "!~" => {
            // Non-equivalence
            let result = !compare_equality(left, "~", right).to_boolean();
            EvaluationResult::Boolean(result)
        }
        _ => EvaluationResult::Empty,
    }
}

/// Checks membership of a value in a collection
fn check_membership(
    left: &EvaluationResult,
    op: &str,
    right: &EvaluationResult,
) -> EvaluationResult {
    // Per FHIRPath spec for operators: If either operand is empty, the result is empty.
    if left == &EvaluationResult::Empty || right == &EvaluationResult::Empty {
        return EvaluationResult::Empty;
    }

    match op {
        "in" => {
            // Check if left is in right (Empty check already handled above)
            let right_items = match right {
                EvaluationResult::Collection(items) => items,
                _ => return EvaluationResult::Boolean(false),
            };

            let is_in = right_items
                .iter()
                .any(|item| compare_equality(left, "=", item).to_boolean());

            EvaluationResult::Boolean(is_in)
        }
        "contains" => {
            match left {
                // For collections, check if any item equals the right value
                EvaluationResult::Collection(items) => {
                    let contains = items
                        .iter()
                        .any(|item| compare_equality(item, "=", right).to_boolean());

                    EvaluationResult::Boolean(contains)
                }
                // For strings, check if the string contains the substring
                EvaluationResult::String(s) => match right {
                    EvaluationResult::String(substr) => {
                        EvaluationResult::Boolean(s.contains(substr))
                    }
                    _ => EvaluationResult::Boolean(false),
                },
                // For other types, return false
                _ => EvaluationResult::Boolean(false),
            }
        }
        _ => EvaluationResult::Empty,
    }
}
