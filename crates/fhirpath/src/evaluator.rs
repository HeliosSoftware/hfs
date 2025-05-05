use crate::parser::{Expression, Invocation, Literal, Term, TypeSpecifier};
use chrono::{Local, Timelike};
use fhir::FhirResource;
use fhirpath_support::{EvaluationError, EvaluationResult, IntoEvaluationResult}; // Import EvaluationError
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
fn apply_decimal_multiplicative(
    left: Decimal,
    op: &str,
    right: Decimal,
) -> Result<EvaluationResult, EvaluationError> {
    if right.is_zero() {
        return Err(EvaluationError::DivisionByZero); // Return error
    }
    match op {
        "div" => {
            // Decimal div Decimal -> Integer (truncate)
            (left / right)
                .trunc() // Truncate the result
                .to_i64() // Convert to i64
                .map(EvaluationResult::Integer)
                // Return error if conversion fails (e.g., overflow)
                .ok_or_else(|| EvaluationError::ArithmeticOverflow)
        }
        "mod" => {
            // Decimal mod Decimal -> Decimal
            Ok(EvaluationResult::Decimal(left % right))
        }
        _ => Err(EvaluationError::InvalidOperation(format!(
            "Unknown decimal multiplicative operator: {}",
            op
        ))),
    }
}

/// Evaluates a FHIRPath expression in the given context, potentially with a specific item as context ($this).
pub fn evaluate(
    expr: &Expression,
    context: &EvaluationContext,
    current_item: Option<&EvaluationResult>,
) -> Result<EvaluationResult, EvaluationError> {
    match expr {
        Expression::Term(term) => evaluate_term(term, context, current_item),
        Expression::Invocation(left, invocation) => {
            // Evaluate the left side first, passing the current item context
            let left_result = evaluate(left, context, current_item)?; // Use ?
            // Pass the evaluated left side result and the original context for invocation
            evaluate_invocation(&left_result, invocation, context)
        }
        Expression::Indexer(left, index) => {
            let left_result = evaluate(left, context, current_item)?; // Use ?
            // Index expression doesn't depend on $this, evaluate normally
            let index_result = evaluate(index, context, None)?; // Use ?
            evaluate_indexer(&left_result, &index_result)
        }
        Expression::Polarity(op, expr) => {
            let result = evaluate(expr, context, current_item)?; // Use ?
            apply_polarity(*op, &result)
        }
        Expression::Multiplicative(left, op, right) => {
            let left_result = evaluate(left, context, current_item)?; // Use ?
            let right_result = evaluate(right, context, current_item)?; // Use ?
            apply_multiplicative(&left_result, op, &right_result)
        }
        Expression::Additive(left, op, right) => {
            let left_result = evaluate(left, context, current_item)?; // Use ?
            let right_result = evaluate(right, context, current_item)?; // Use ?
            apply_additive(&left_result, op, &right_result)
        }
        Expression::Type(left, op, type_spec) => {
            let result = evaluate(left, context, current_item)?; // Use ?
            apply_type_operation(&result, op, type_spec)
        }
        Expression::Union(left, right) => {
            let left_result = evaluate(left, context, current_item)?; // Use ?
            let right_result = evaluate(right, context, current_item)?; // Use ?
            // Union itself doesn't typically error, just returns combined set
            Ok(union_collections(&left_result, &right_result))
        }
        Expression::Inequality(left, op, right) => {
            let left_result = evaluate(left, context, current_item)?; // Use ?
            let right_result = evaluate(right, context, current_item)?; // Use ?
            // compare_inequality now returns Result, so just call it directly
            compare_inequality(&left_result, op, &right_result)
        }
        Expression::Equality(left, op, right) => {
            let left_result = evaluate(left, context, current_item)?; // Use ?
            let right_result = evaluate(right, context, current_item)?; // Use ?
            // compare_equality now returns Result, so just call it directly
            compare_equality(&left_result, op, &right_result)
        }
        Expression::Membership(left, op, right) => {
            let left_result = evaluate(left, context, current_item)?; // Use ?
            let right_result = evaluate(right, context, current_item)?; // Use ?
            // Membership returns Empty on empty operand or errors on multi-item left
            check_membership(&left_result, op, &right_result)
        }
        Expression::And(left, right) => {
            // Evaluate operands first
            let left_eval = evaluate(left, context, current_item)?;
            let right_eval = evaluate(right, context, current_item)?;

            // Check types *before* logical conversion
            if !matches!(left_eval, EvaluationResult::Boolean(_) | EvaluationResult::Empty) ||
               !matches!(right_eval, EvaluationResult::Boolean(_) | EvaluationResult::Empty) {
                // Allow Empty for 3-valued logic, but reject other types
                if !matches!(left_eval, EvaluationResult::Empty) && !matches!(right_eval, EvaluationResult::Empty) {
                     return Err(EvaluationError::TypeError(format!(
                        "Operator 'and' requires Boolean operands, found {} and {}",
                        left_eval.type_name(), right_eval.type_name()
                    )));
                }
            }

            // Convert to boolean for logic AFTER type check
            let left_bool = left_eval.to_boolean_for_logic()?;
            let right_bool = right_eval.to_boolean_for_logic()?;


            match left_bool {
                EvaluationResult::Boolean(false) => Ok(EvaluationResult::Boolean(false)), // false and X -> false
                EvaluationResult::Empty => {
                    // Evaluate right, handle potential error
                    let right_eval = evaluate(right, context, current_item)?;
                    let right_bool = right_eval.to_boolean_for_logic()?; // Propagate error
                    match right_bool {
                        EvaluationResult::Boolean(false) => Ok(EvaluationResult::Boolean(false)), // {} and false -> false
                        _ => Ok(EvaluationResult::Empty), // {} and (true | {}) -> {}
                    }
                }
                EvaluationResult::Boolean(true) => {
                    // Evaluate right, handle potential error
                    let right_eval = evaluate(right, context, current_item)?;
                    let right_bool = right_eval.to_boolean_for_logic()?; // Propagate error
                    // Check if right_bool is actually Boolean or Empty
                    if matches!(right_bool, EvaluationResult::Boolean(_) | EvaluationResult::Empty) {
                         Ok(right_bool) // true and X -> X (where X is Boolean or Empty)
                    } else {
                         Err(EvaluationError::TypeError(format!(
                            "Invalid type for 'and' right operand: {}", right_bool.type_name()
                        )))
                    }
                }
                 // This case should be unreachable if to_boolean_for_logic works correctly
                _ => Err(EvaluationError::TypeError(format!(
                    "Invalid type for 'and' left operand: {}", left_bool.type_name()
                ))),
            }
        }
        Expression::Or(left, op, right) => {
            // Evaluate left, handle potential error
            let left_eval = evaluate(left, context, current_item)?;
            let left_bool = left_eval.to_boolean_for_logic()?; // Propagate error

            // Evaluate right, handle potential error
            let right_eval = evaluate(right, context, current_item)?;
            let right_bool = right_eval.to_boolean_for_logic()?; // Propagate error

            // Ensure both operands resolved to Boolean or Empty
            if !matches!(left_bool, EvaluationResult::Boolean(_) | EvaluationResult::Empty) {
                 return Err(EvaluationError::TypeError(format!(
                    "Invalid type for '{}' left operand: {}", op, left_bool.type_name()
                )));
            }
             if !matches!(right_bool, EvaluationResult::Boolean(_) | EvaluationResult::Empty) {
                 return Err(EvaluationError::TypeError(format!(
                    "Invalid type for '{}' right operand: {}", op, right_bool.type_name()
                )));
            }


            if op == "or" {
                match (&left_bool, &right_bool) {
                    (EvaluationResult::Boolean(true), _) | (_, EvaluationResult::Boolean(true)) => {
                        Ok(EvaluationResult::Boolean(true))
                    }
                    (EvaluationResult::Empty, EvaluationResult::Empty) => {
                        Ok(EvaluationResult::Empty)
                    }
                    (EvaluationResult::Empty, EvaluationResult::Boolean(false)) => {
                        Ok(EvaluationResult::Empty)
                    }
                    (EvaluationResult::Boolean(false), EvaluationResult::Empty) => {
                        Ok(EvaluationResult::Empty)
                    }
                    (EvaluationResult::Boolean(false), EvaluationResult::Boolean(false)) => {
                        Ok(EvaluationResult::Boolean(false))
                    }
                     // Cases involving Empty handled above, this should not be reached with invalid types
                    _ => unreachable!("Invalid types should have been caught earlier for 'or'"),
                }
            } else { // xor
                match (&left_bool, &right_bool) {
                    (EvaluationResult::Empty, _) | (_, EvaluationResult::Empty) => {
                        Ok(EvaluationResult::Empty)
                    }
                    (EvaluationResult::Boolean(l), EvaluationResult::Boolean(r)) => {
                        Ok(EvaluationResult::Boolean(l != r))
                    }
                     // Cases involving Empty handled above, this should not be reached with invalid types
                    _ => unreachable!("Invalid types should have been caught earlier for 'xor'"),
                }
            }
        }
        Expression::Implies(left, right) => {
            // Evaluate left, handle potential error
            let left_eval = evaluate(left, context, current_item)?;
            let left_bool = left_eval.to_boolean_for_logic()?; // Propagate error

             // Ensure left operand resolved to Boolean or Empty
            if !matches!(left_bool, EvaluationResult::Boolean(_) | EvaluationResult::Empty) {
                 return Err(EvaluationError::TypeError(format!(
                    "Invalid type for 'implies' left operand: {}", left_bool.type_name()
                )));
            }


            match left_bool {
                EvaluationResult::Boolean(false) => Ok(EvaluationResult::Boolean(true)), // false implies X -> true
                EvaluationResult::Empty => {
                    // Evaluate right, handle potential error
                    let right_eval = evaluate(right, context, current_item)?;
                    let right_bool = right_eval.to_boolean_for_logic()?; // Propagate error
                     // Ensure right operand resolved to Boolean or Empty
                    if !matches!(right_bool, EvaluationResult::Boolean(_) | EvaluationResult::Empty) {
                         return Err(EvaluationError::TypeError(format!(
                            "Invalid type for 'implies' right operand: {}", right_bool.type_name()
                        )));
                    }
                    match right_bool {
                        EvaluationResult::Boolean(true) => Ok(EvaluationResult::Boolean(true)), // {} implies true -> true
                        _ => Ok(EvaluationResult::Empty), // {} implies (false | {}) -> {}
                    }
                }
                EvaluationResult::Boolean(true) => {
                    // Evaluate right, handle potential error
                    let right_eval = evaluate(right, context, current_item)?;
                    let right_bool = right_eval.to_boolean_for_logic()?; // Propagate error
                     // Ensure right operand resolved to Boolean or Empty
                    if !matches!(right_bool, EvaluationResult::Boolean(_) | EvaluationResult::Empty) {
                         return Err(EvaluationError::TypeError(format!(
                            "Invalid type for 'implies' right operand: {}", right_bool.type_name()
                        )));
                    }
                    Ok(right_bool) // true implies X -> X (Boolean or Empty)
                }
                 // This case should be unreachable if to_boolean_for_logic works correctly
                 _ => unreachable!("Invalid type for 'implies' left operand should have been caught"),
            }
        }
        Expression::Lambda(_, _) => {
            // Lambda expressions are not directly evaluated here.
            // They are used in function calls
            // Return Ok(Empty) as it's not an error, just not evaluated yet.
            Ok(EvaluationResult::Empty)
        }
    }
}

/// Normalizes a vector of results according to FHIRPath singleton evaluation rules.
/// Returns Empty if vec is empty, the single item if len is 1, or Collection(vec) otherwise.
fn normalize_collection_result(mut items: Vec<EvaluationResult>) -> EvaluationResult {
    if items.is_empty() {
        EvaluationResult::Empty
    } else if items.len() == 1 {
        items.pop().unwrap() // Take the single item
    } else {
        EvaluationResult::Collection(items)
    }
}

/// Evaluates a term in the given context, potentially with a specific item as context ($this).
fn evaluate_term(
    term: &Term,
    context: &EvaluationContext,
    current_item: Option<&EvaluationResult>,
) -> Result<EvaluationResult, EvaluationError> {
    match term {
        Term::Invocation(invocation) => {
            // Explicitly handle $this first and return
            if *invocation == Invocation::This {
                return Ok(if let Some(item) = current_item.cloned() {
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
                }); // Close Ok() here
            }

            // Handle variables (%var, %context) next and return
            if let Invocation::Member(name) = invocation {
                if name.starts_with('%') {
                    let var_name = &name[1..]; // Remove the % prefix
                    if var_name == "context" {
                        // Return %context value
                        // Correctly wrap the entire conditional result in Ok()
                        return Ok(if context.resources.is_empty() {
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
                        }); // Removed the misplaced }); here
                    } else {
                        // Return other variable value or error if undefined
                        return match context.get_variable(var_name) {
                            Some(value) => Ok(EvaluationResult::String(value.clone())),
                            None => {
                                Err(EvaluationError::UndefinedVariable(format!("%{}", var_name)))
                            }
                        };
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
        Term::Literal(literal) => Ok(evaluate_literal(literal)), // Wrap in Ok
        Term::ExternalConstant(name) => {
            // Look up external constant in the context
            // Special handling for %context
            if name == "context" {
                Ok(if context.resources.is_empty() {
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
                }) // Correctly placed Ok() wrapping
            } else {
                // Return variable value or error if undefined
                match context.get_variable(name) {
                    Some(value) => Ok(EvaluationResult::String(value.clone())),
                    None => Err(EvaluationError::UndefinedVariable(format!("%{}", name))),
                }
            }
        }
        Term::Parenthesized(expr) => evaluate(expr, context, current_item), // Propagate Result
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
) -> Result<EvaluationResult, EvaluationError> {
    match invocation {
        Invocation::Member(name) => {
            // Handle member access on the invocation_base
            // Special handling for boolean literals that might be parsed as identifiers
            if name == "true" && matches!(invocation_base, EvaluationResult::Empty) {
                // Only if base is empty context
                return Ok(EvaluationResult::Boolean(true));
            } else if name == "false" && matches!(invocation_base, EvaluationResult::Empty) {
                return Ok(EvaluationResult::Boolean(false));
            }

            // Access a member of the invocation_base
            match invocation_base {
                EvaluationResult::Object(obj) => {
                    // Add debug print for member access
                    eprintln!(
                        "Debug [evaluate_invocation]: Accessing member '{}' on object: {:?}",
                        name, obj
                    );
                    let result = obj
                        .get(name.as_str())
                        .cloned()
                        .unwrap_or(EvaluationResult::Empty);
                    eprintln!(
                        "Debug [evaluate_invocation]: Member access result: {:?}",
                        result
                    );
                    Ok(result) // Wrap in Ok
                }
                EvaluationResult::Collection(items) => {
                    // For collections, apply member access to each item and collect results
                    let mut results = Vec::new();
                    for item in items {
                        // Recursively call member access on each item, propagating errors
                        match evaluate_invocation(item, &Invocation::Member(name.clone()), context)
                        {
                            Ok(EvaluationResult::Empty) => {} // Skip empty results
                            Ok(res) => results.push(res),
                            Err(e) => return Err(e), // Propagate error immediately
                        }
                    }

                    // Flatten nested collections that might result from member access on items
                    let flattened_results: Vec<EvaluationResult> = results
                        .into_iter()
                        .flat_map(|res| match res {
                            EvaluationResult::Collection(inner) => inner,
                            EvaluationResult::Empty => vec![], // Should not happen due to filter above, but safe
                            other => vec![other],
                        })
                        .collect();

                    Ok(normalize_collection_result(flattened_results)) // Wrap normalized result in Ok
                }
                // Accessing member on primitive types or Empty returns Empty
                _ => Ok(EvaluationResult::Empty), // Wrap in Ok
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
                        Ok(evaluate_of_type(invocation_base, &type_spec)) // Wrap in Ok
                    } else {
                        Err(EvaluationError::InvalidArgument(format!(
                            "Invalid type specifier argument for ofType: {:?}",
                            args_exprs[0]
                        )))
                    }
                }
                "iif" if args_exprs.len() >= 2 => {
                    // iif(condition, trueResult, [otherwiseResult])
                    let condition_expr = &args_exprs[0];
                    let true_result_expr = &args_exprs[1];
                    let otherwise_result_expr = args_exprs.get(2); // Optional third argument

                    // Evaluate the condition expression, handle potential error
                    let condition_result =
                        evaluate(condition_expr, context, Some(invocation_base))?;
                    let condition_bool = condition_result.to_boolean_for_logic()?; // Use logic conversion

                    if matches!(condition_bool, EvaluationResult::Boolean(true)) {
                        // Condition is true, evaluate the trueResult expression, propagate error
                        evaluate(true_result_expr, context, Some(invocation_base))
                    } else {
                        // Condition is false or empty
                        if let Some(otherwise_expr) = otherwise_result_expr {
                            // Evaluate the otherwiseResult expression if present, propagate error
                            evaluate(otherwise_expr, context, Some(invocation_base))
                        } else {
                            // Otherwise result is omitted, return empty collection
                            Ok(EvaluationResult::Empty)
                        }
                    }
                }
                // Add other functions taking lambdas here (e.g., any, repeat)
                _ => {
                    // Default: Evaluate all standard function arguments first (without $this context), then call function
                    let mut evaluated_args = Vec::with_capacity(args_exprs.len());
                    for arg_expr in args_exprs {
                        evaluated_args.push(evaluate(arg_expr, context, None)?); // Evaluate args, propagate error
                    }
                    // Call with updated signature (name, base, args)
                    call_function(name, invocation_base, &evaluated_args)
                }
            }
        }
        Invocation::This => {
            // This should be handled by evaluate_term, but as a fallback:
            Ok(invocation_base.clone()) // Return the base it was invoked on
        }
        Invocation::Index => {
            // $index should return the current index in a collection operation
            // This is typically used in filter expressions
            // For now, we return Empty as this requires tracking iteration state
            Ok(EvaluationResult::Empty)
        }
        Invocation::Total => {
            // $total should return the total number of items in the original collection
            // For now, we return Empty as this requires tracking the original collection
            Ok(EvaluationResult::Empty)
        }
    }
}

// --- Helper functions for lambda evaluation ---

/// Evaluates the 'exists' function with a criteria expression.
fn evaluate_exists_with_criteria(
    collection: &EvaluationResult,
    criteria_expr: &Expression,
    context: &EvaluationContext,
) -> Result<EvaluationResult, EvaluationError> {
    let items_to_check = match collection {
        EvaluationResult::Collection(items) => items.clone(),
        EvaluationResult::Empty => vec![],
        // Treat single item as a one-item collection
        single_item => vec![single_item.clone()],
    };

    if items_to_check.is_empty() {
        return Ok(EvaluationResult::Boolean(false)); // Exists is false for empty collection
    }

    for item in items_to_check {
        // Evaluate the criteria expression with the current item as $this, propagate error
        let criteria_result = evaluate(criteria_expr, context, Some(&item))?;
        // exists returns true if the criteria evaluates to true for *any* item
        if criteria_result.to_boolean() {
            return Ok(EvaluationResult::Boolean(true)); // Ensure this return is Ok()
        }
    }

    // If no item satisfied the criteria
    Ok(EvaluationResult::Boolean(false)) // This was likely the source of E0308 at 422
}

/// Evaluates the 'where' function.
fn evaluate_where(
    collection: &EvaluationResult,
    criteria_expr: &Expression,
    context: &EvaluationContext,
) -> Result<EvaluationResult, EvaluationError> {
    let items_to_filter = match collection {
        EvaluationResult::Collection(items) => items.clone(),
        EvaluationResult::Empty => vec![],
        single_item => vec![single_item.clone()],
    };

    let mut filtered_items = Vec::new();
    for item in items_to_filter {
        // Evaluate criteria, propagate error
        let criteria_result = evaluate(criteria_expr, context, Some(&item))?;
        // Check if criteria is boolean, otherwise error per spec
        match criteria_result {
            EvaluationResult::Boolean(true) => filtered_items.push(item.clone()),
            EvaluationResult::Boolean(false) | EvaluationResult::Empty => {} // Ignore false/empty
            other => {
                return Err(EvaluationError::TypeError(format!(
                    "where criteria evaluated to non-boolean: {:?}",
                    other
                )));
            }
        }
    }

    // Return Empty or Collection, apply normalization
    Ok(normalize_collection_result(filtered_items))
}

/// Evaluates the 'select' function.
fn evaluate_select(
    collection: &EvaluationResult,
    projection_expr: &Expression,
    context: &EvaluationContext,
) -> Result<EvaluationResult, EvaluationError> {
    let items_to_project = match collection {
        EvaluationResult::Collection(items) => items.clone(),
        EvaluationResult::Empty => vec![],
        single_item => vec![single_item.clone()],
    };

    let mut projected_items = Vec::new();
    for item in items_to_project {
        // Evaluate projection, propagate error
        let projection_result = evaluate(projection_expr, context, Some(&item))?;
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

    // Return Empty or Collection, apply normalization
    Ok(normalize_collection_result(projected_items))
}

/// Evaluates the 'all' function with a criteria expression.
fn evaluate_all_with_criteria(
    collection: &EvaluationResult,
    criteria_expr: &Expression,
    context: &EvaluationContext,
) -> Result<EvaluationResult, EvaluationError> {
    let items_to_check = match collection {
        EvaluationResult::Collection(items) => items.clone(),
        EvaluationResult::Empty => vec![],
        // Treat single item as a one-item collection
        single_item => vec![single_item.clone()],
    };

    // 'all' is true for an empty collection
    if items_to_check.is_empty() {
        return Ok(EvaluationResult::Boolean(true));
    }

    for item in items_to_check {
        // Evaluate the criteria expression with the current item as $this, propagate error
        let criteria_result = evaluate(criteria_expr, context, Some(&item))?;
        // Check if criteria is boolean, otherwise error
        match criteria_result {
            EvaluationResult::Boolean(false) | EvaluationResult::Empty => {
                return Ok(EvaluationResult::Boolean(false));
            } // False or empty means not all are true
            EvaluationResult::Boolean(true) => {} // Continue checking
            other => {
                return Err(EvaluationError::TypeError(format!(
                    "all criteria evaluated to non-boolean: {:?}",
                    other
                )));
            }
        }
    }

    // If all items satisfied the criteria (were true)
    Ok(EvaluationResult::Boolean(true))
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

    // Return Empty or Collection, apply normalization
    normalize_collection_result(filtered_items)
}

/// Calls a standard FHIRPath function (that doesn't take a lambda).
fn call_function(
    name: &str,
    invocation_base: &EvaluationResult, // Renamed from context to avoid confusion
    args: &[EvaluationResult],
) -> Result<EvaluationResult, EvaluationError> {
    match name {
        "count" => {
            // Returns the number of items in the collection, including duplicates
            Ok(match invocation_base {
                // Wrap result in Ok
                EvaluationResult::Collection(items) => {
                    EvaluationResult::Integer(items.len() as i64)
                }
                EvaluationResult::Empty => EvaluationResult::Integer(0),
                _ => EvaluationResult::Integer(1), // Single item counts as 1
            })
        }
        "empty" => {
            // Returns true if the collection is empty (0 items)
            Ok(match invocation_base {
                // Wrap result in Ok
                // Use invocation_base, not context
                EvaluationResult::Empty => EvaluationResult::Boolean(true),
                EvaluationResult::Collection(items) => EvaluationResult::Boolean(items.is_empty()),
                _ => EvaluationResult::Boolean(false), // Single non-empty item is not empty
            })
        }
        "exists" => {
            // This handles exists() without criteria.
            // exists(criteria) is handled in evaluate_invocation.
            Ok(match invocation_base {
                // Wrap result in Ok
                EvaluationResult::Empty => EvaluationResult::Boolean(false),
                EvaluationResult::Collection(items) => EvaluationResult::Boolean(!items.is_empty()),
                _ => EvaluationResult::Boolean(true), // Single non-empty item exists
            })
        }
        "all" => {
            // This handles all() without criteria.
            // all(criteria) is handled in evaluate_invocation.
            Ok(match invocation_base {
                // Wrap result in Ok
                EvaluationResult::Empty => EvaluationResult::Boolean(true), // all() is true for empty
                EvaluationResult::Collection(items) => {
                    // Check if all items evaluate to true
                    // Wrap the boolean result
                    EvaluationResult::Boolean(items.iter().all(|item| item.to_boolean()))
                }
                single_item => EvaluationResult::Boolean(single_item.to_boolean()), // Check single item
            })
        }
        "allTrue" => {
            let items = match invocation_base {
                EvaluationResult::Collection(items) => items.clone(),
                EvaluationResult::Empty => vec![],
                single_item => vec![single_item.clone()],
            };
            // allTrue is true for an empty collection
            if items.is_empty() {
                return Ok(EvaluationResult::Boolean(true));
            }
            for item in items {
                match item {
                    EvaluationResult::Boolean(true) => continue,
                    EvaluationResult::Boolean(false) | EvaluationResult::Empty => {
                        return Ok(EvaluationResult::Boolean(false));
                    }
                    // If any item is not boolean, it's an error according to spec (implicitly)
                    _ => {
                        return Err(EvaluationError::TypeError(
                            "allTrue expects a collection of Booleans".to_string(),
                        ));
                    }
                }
            }
            Ok(EvaluationResult::Boolean(true))
        }
        "anyTrue" => {
            let items = match invocation_base {
                EvaluationResult::Collection(items) => items.clone(),
                EvaluationResult::Empty => vec![],
                single_item => vec![single_item.clone()],
            };
            // anyTrue is false for an empty collection
            if items.is_empty() {
                return Ok(EvaluationResult::Boolean(false));
            }
            for item in items {
                match item {
                    EvaluationResult::Boolean(true) => return Ok(EvaluationResult::Boolean(true)),
                    EvaluationResult::Boolean(false) | EvaluationResult::Empty => continue,
                    // If any item is not boolean, it's an error according to spec (implicitly)
                    _ => {
                        return Err(EvaluationError::TypeError(
                            "anyTrue expects a collection of Booleans".to_string(),
                        ));
                    }
                }
            }
            Ok(EvaluationResult::Boolean(false)) // No true item found
        }
        "allFalse" => {
            let items = match invocation_base {
                EvaluationResult::Collection(items) => items.clone(),
                EvaluationResult::Empty => vec![],
                single_item => vec![single_item.clone()],
            };
            // allFalse is true for an empty collection
            if items.is_empty() {
                return Ok(EvaluationResult::Boolean(true));
            }
            for item in items {
                match item {
                    EvaluationResult::Boolean(false) => continue,
                    EvaluationResult::Boolean(true) | EvaluationResult::Empty => {
                        return Ok(EvaluationResult::Boolean(false));
                    }
                    // If any item is not boolean, it's an error according to spec (implicitly)
                    _ => {
                        return Err(EvaluationError::TypeError(
                            "allFalse expects a collection of Booleans".to_string(),
                        ));
                    }
                }
            }
            Ok(EvaluationResult::Boolean(true))
        }
        "anyFalse" => {
            let items = match invocation_base {
                EvaluationResult::Collection(items) => items.clone(),
                EvaluationResult::Empty => vec![],
                single_item => vec![single_item.clone()],
            };
            // anyFalse is false for an empty collection
            if items.is_empty() {
                return Ok(EvaluationResult::Boolean(false));
            }
            for item in items {
                match item {
                    EvaluationResult::Boolean(false) => return Ok(EvaluationResult::Boolean(true)),
                    EvaluationResult::Boolean(true) | EvaluationResult::Empty => continue,
                    // If any item is not boolean, it's an error according to spec (implicitly)
                    _ => {
                        return Err(EvaluationError::TypeError(
                            "anyFalse expects a collection of Booleans".to_string(),
                        ));
                    }
                }
            }
            Ok(EvaluationResult::Boolean(false)) // No false item found
        }
        "first" => {
            // Returns the first item in the collection
            Ok(
                if let EvaluationResult::Collection(items) = invocation_base {
                    // Wrap in Ok
                    items.first().cloned().unwrap_or(EvaluationResult::Empty)
                } else {
                    // A single item is returned as is (unless it's Empty)
                    invocation_base.clone()
                },
            )
        }
        "last" => {
            // Returns the last item in the collection
            Ok(
                if let EvaluationResult::Collection(items) = invocation_base {
                    // Wrap in Ok
                    items.last().cloned().unwrap_or(EvaluationResult::Empty)
                } else {
                    // A single item is returned as is (unless it's Empty)
                    invocation_base.clone()
                },
            )
        }
        "not" => {
            // Logical negation
            Ok(match invocation_base {
                // Wrap in Ok
                EvaluationResult::Boolean(b) => EvaluationResult::Boolean(!b),
                EvaluationResult::Empty => EvaluationResult::Empty, // not({}) is {}
                // Other types are implicitly converted to boolean first
                _ => EvaluationResult::Boolean(!invocation_base.to_boolean()),
            })
        }
        "contains" => {
            // Function call version
            // Check if the invocation_base contains the argument
            if args.len() != 1 {
                return Err(EvaluationError::InvalidArity(
                    "Function 'contains' expects 1 argument".to_string(),
                ));
            }
            let arg = &args[0];

            // Check for singleton base *if it's not a string*
            if !matches!(invocation_base, EvaluationResult::String(_)) && invocation_base.count() > 1 {
                 return Err(EvaluationError::SingletonEvaluationError(
                    "contains function requires singleton input collection (or string)".to_string(),
                ));
            }

            // Spec: X contains {} -> {}
            if arg == &EvaluationResult::Empty {
                return Ok(EvaluationResult::Empty);
            }
            // Spec: {} contains X -> false (where X is not empty)
            if invocation_base == &EvaluationResult::Empty {
                return Ok(EvaluationResult::Boolean(false));
            }
            // Check for multi-item argument (error)
            if arg.count() > 1 {
                return Err(EvaluationError::SingletonEvaluationError(
                    "contains argument must be a singleton".to_string(),
                ));
            }

            Ok(match invocation_base {
                // Wrap result in Ok
                EvaluationResult::String(s) => {
                    // String contains substring
                    if let EvaluationResult::String(substr) = arg {
                        EvaluationResult::Boolean(s.contains(substr))
                    } else {
                        // Argument is not String (and not Empty) -> false
                        EvaluationResult::Boolean(false)
                    }
                }
                EvaluationResult::Collection(items) => {
                    // Collection contains item (using equality)
                    // Use map_or to handle potential error from compare_equality
                    let contains = items
                        .iter()
                        .any(|item| compare_equality(item, "=", arg).map_or(false, |r| r.to_boolean()));
                    EvaluationResult::Boolean(contains)
                }
                // contains on single non-collection/non-string item
                single_item => {
                    // Treat as single-item collection: check if the item equals the argument
                    // Use map_or to handle potential error from compare_equality
                    EvaluationResult::Boolean(compare_equality(single_item, "=", arg).map_or(false, |r| r.to_boolean()))
                }
            })
        }
        "isDistinct" => {
            // Returns true if all items in the collection are distinct (based on equality)
            let items = match invocation_base {
                EvaluationResult::Collection(items) => items.clone(),
                EvaluationResult::Empty => vec![],
                single_item => vec![single_item.clone()], // Treat single item as collection
            };

            if items.len() <= 1 {
                return Ok(EvaluationResult::Boolean(true)); // Empty or single-item collections are distinct
            }

            for i in 0..items.len() {
                for j in (i + 1)..items.len() {
                    // Use compare_equality, handle potential error, default to false if error
                    if compare_equality(&items[i], "=", &items[j]).map_or(false, |r| r.to_boolean()) {
                        return Ok(EvaluationResult::Boolean(false)); // Found a duplicate
                    }
                }
            }

            Ok(EvaluationResult::Boolean(true)) // No duplicates found using strict equality
        }
        "subsetOf" => {
            // Checks if the invocation collection is a subset of the argument collection
            if args.len() != 1 {
                return Err(EvaluationError::InvalidArity(
                    "Function 'subsetOf' expects 1 argument".to_string(),
                ));
            }
            let other_collection = &args[0];

            let self_items = match invocation_base {
                EvaluationResult::Collection(items) => items,
                EvaluationResult::Empty => return Ok(EvaluationResult::Boolean(true)), // Empty set is subset of anything
                single => &[single.clone()][..], // Treat single item as slice
            };
            let other_items = match other_collection {
                EvaluationResult::Collection(items) => items,
                EvaluationResult::Empty => &[][..], // Empty slice
                single => &[single.clone()][..],    // Treat single item as slice
            };

            // Use HashSet for efficient lookup in the 'other' collection
            let other_set: HashSet<_> = other_items.iter().collect();

            // Check if every item in self_items is present in other_set
            let is_subset = self_items.iter().all(|item| other_set.contains(item));
            Ok(EvaluationResult::Boolean(is_subset))
        }
        "supersetOf" => {
            // Checks if the invocation collection is a superset of the argument collection
            if args.len() != 1 {
                return Err(EvaluationError::InvalidArity(
                    "Function 'supersetOf' expects 1 argument".to_string(),
                ));
            }
            let other_collection = &args[0];

            let self_items = match invocation_base {
                EvaluationResult::Collection(items) => items,
                EvaluationResult::Empty => &[][..],
                single => &[single.clone()][..],
            };
            let other_items = match other_collection {
                EvaluationResult::Collection(items) => items,
                EvaluationResult::Empty => return Ok(EvaluationResult::Boolean(true)), // Anything is superset of empty set
                single => &[single.clone()][..],
            };

            // Use HashSet for efficient lookup in the 'self' collection
            let self_set: HashSet<_> = self_items.iter().collect();

            // Check if every item in other_items is present in self_set
            let is_superset = other_items.iter().all(|item| self_set.contains(item));
            Ok(EvaluationResult::Boolean(is_superset))
        }
        "toDecimal" => {
            // Converts the input to Decimal according to FHIRPath rules
            // Check for singleton
            if invocation_base.count() > 1 {
                return Err(EvaluationError::SingletonEvaluationError(
                    "toDecimal requires a singleton input".to_string(),
                ));
            }
            Ok(match invocation_base {
                // Wrap in Ok
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
                // Collections handled by initial check
                EvaluationResult::Collection(_) => unreachable!(), // This arm is unreachable due to the count check above
                // Other types are not convertible
                _ => EvaluationResult::Empty,
            })
        }
        "toInteger" => {
            // Converts the input to Integer according to FHIRPath rules
            // Check for singleton
            if invocation_base.count() > 1 {
                return Err(EvaluationError::SingletonEvaluationError(
                    "toInteger requires a singleton input".to_string(),
                ));
            }
            Ok(match invocation_base {
                // Wrap in Ok
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
                // Collections handled by initial check
                EvaluationResult::Collection(_) => unreachable!(),
                // Other types are not convertible
                _ => EvaluationResult::Empty,
            })
        }
        "distinct" => {
            // Returns the collection with duplicates removed (based on equality)
            let items = match invocation_base {
                EvaluationResult::Collection(items) => items.clone(),
                EvaluationResult::Empty => return Ok(EvaluationResult::Empty),
                single_item => return Ok(single_item.clone()), // Wrap single_item return in Ok
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

            // Return Empty or Collection, apply normalization
            Ok(normalize_collection_result(distinct_items))
        }
        "skip" => {
            // Returns the collection with the first 'num' items removed
            if args.len() != 1 {
                return Err(EvaluationError::InvalidArity(
                    "Function 'skip' expects 1 argument".to_string(),
                ));
            }
            let num_to_skip = match &args[0] {
                EvaluationResult::Integer(i) => {
                    if *i < 0 { 0 } else { *i as usize } // Treat negative skip as 0
                }
                // Add conversion from Decimal if it's an integer value
                EvaluationResult::Decimal(d) if d.is_integer() && d.is_sign_positive() => {
                    d.to_usize().unwrap_or(0) // Convert non-negative integer Decimal
                }
                _ => {
                    return Err(EvaluationError::InvalidArgument(
                        "skip argument must be a non-negative integer".to_string(),
                    ));
                }
            };

            let items = match invocation_base {
                EvaluationResult::Collection(items) => items.clone(),
                EvaluationResult::Empty => vec![],
                single_item => vec![single_item.clone()], // Treat single item as collection
            };

            Ok(if num_to_skip >= items.len() {
                // Wrap in Ok
                EvaluationResult::Empty
            } else {
                let skipped_items = items[num_to_skip..].to_vec();
                // Return Empty or Collection, apply normalization
                normalize_collection_result(skipped_items)
            })
        }
        "tail" => {
            // Returns the collection with all items except the first
            Ok(
                if let EvaluationResult::Collection(items) = invocation_base {
                    // Wrap in Ok
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
                },
            )
        }
        "take" => {
            // Returns the first 'num' items from the collection
            if args.len() != 1 {
                return Err(EvaluationError::InvalidArity(
                    "Function 'take' expects 1 argument".to_string(),
                ));
            }
            let num_to_take = match &args[0] {
                EvaluationResult::Integer(i) => {
                    if *i <= 0 { 0 } else { *i as usize } // Treat non-positive take as 0
                }
                // Add conversion from Decimal if it's an integer value
                EvaluationResult::Decimal(d) if d.is_integer() && d.is_sign_positive() => {
                    d.to_usize().unwrap_or(0) // Convert non-negative integer Decimal
                }
                _ => {
                    return Err(EvaluationError::InvalidArgument(
                        "take argument must be a non-negative integer".to_string(),
                    ));
                }
            };

            if num_to_take == 0 {
                return Ok(EvaluationResult::Empty);
            }

            let items = match invocation_base {
                EvaluationResult::Collection(items) => items.clone(),
                EvaluationResult::Empty => vec![],
                single_item => vec![single_item.clone()], // Treat single item as collection
            };

            let taken_items: Vec<EvaluationResult> = items.into_iter().take(num_to_take).collect();

            // Return Empty or Collection, apply normalization
            Ok(normalize_collection_result(taken_items))
        }
        "intersect" => {
            // Returns the intersection of two collections (items present in both, order not guaranteed)
            if args.len() != 1 {
                return Err(EvaluationError::InvalidArity(
                    "Function 'intersect' expects 1 argument".to_string(),
                ));
            }
            let other_collection = &args[0];

            // If either input is empty, the intersection is empty
            if invocation_base == &EvaluationResult::Empty
                || other_collection == &EvaluationResult::Empty
            {
                return Ok(EvaluationResult::Empty);
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
                // Use map_or to handle potential error from compare_equality
                let exists_in_right = right_items
                    .iter()
                    .any(|right_item| compare_equality(left_item, "=", right_item).map_or(false, |r| r.to_boolean()));

                if exists_in_right {
                    // Attempt to insert the item into the HashSet.
                    // If insert returns true, it means the item was not already present.
                    if added_items_set.insert(left_item.clone()) {
                        intersection_items.push(left_item.clone());
                    }
                }
            }

            // Return Empty or Collection, apply normalization
            Ok(normalize_collection_result(intersection_items))
        }
        "exclude" => {
            // Returns items in invocation_base that are NOT in the argument collection (preserves order and duplicates)
            if args.len() != 1 {
                return Err(EvaluationError::InvalidArity(
                    "Function 'exclude' expects 1 argument".to_string(),
                ));
            }
            let other_collection = &args[0];

            // If invocation_base is empty, result is empty
            if invocation_base == &EvaluationResult::Empty {
                return Ok(EvaluationResult::Empty);
            }
            // If other_collection is empty, result is invocation_base
            if other_collection == &EvaluationResult::Empty {
                return Ok(invocation_base.clone());
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
                // Use map_or to handle potential error from compare_equality
                let exists_in_right = right_items
                    .iter()
                    .any(|right_item| compare_equality(left_item, "=", right_item).map_or(false, |r| r.to_boolean()));

                // Keep the item if it does NOT exist in the right collection
                if !exists_in_right {
                    result_items.push(left_item.clone());
                }
            }

            // Return Empty or Collection, preserving duplicates and order, apply normalization
            Ok(normalize_collection_result(result_items))
        }
        "union" => {
            // Returns the union of two collections (distinct items from both, order not guaranteed)
            if args.len() != 1 {
                return Err(EvaluationError::InvalidArity(
                    "Function 'union' expects 1 argument".to_string(),
                ));
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

            // Return Empty or Collection, apply normalization
            Ok(normalize_collection_result(union_items))
        }
        "combine" => {
            // Returns a collection containing all items from both collections, including duplicates, preserving order
            if args.len() != 1 {
                return Err(EvaluationError::InvalidArity(
                    "Function 'combine' expects 1 argument".to_string(),
                ));
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

            // Return Empty or Collection, apply normalization
            Ok(normalize_collection_result(combined_items))
        }
        "single" => {
            // Returns the single item in a collection, or empty if 0 or >1 items
            match invocation_base {
                EvaluationResult::Collection(items) => {
                    if items.len() == 1 {
                        Ok(items[0].clone())
                    } else if items.is_empty() {
                        Ok(EvaluationResult::Empty) // Empty input -> Empty output
                    } else {
                        // Error if multiple items
                        Err(EvaluationError::SingletonEvaluationError(format!(
                            "single() requires a singleton collection, found {} items",
                            items.len()
                        )))
                    }
                }
                EvaluationResult::Empty => Ok(EvaluationResult::Empty),
                single_item => Ok(single_item.clone()), // Single non-collection item is returned as is
            }
        }
        "convertsToDecimal" => {
            // Checks if the input can be converted to Decimal
            // Check for singleton first
            if invocation_base.count() > 1 {
                return Err(EvaluationError::SingletonEvaluationError(
                    "convertsToDecimal requires a singleton input".to_string(),
                ));
            }
            Ok(match invocation_base {
                // Wrap in Ok
                EvaluationResult::Empty => EvaluationResult::Empty, // Empty input -> Empty result
                // Collections handled by initial check
                EvaluationResult::Collection(_) => unreachable!(),
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
            })
        }
        "convertsToInteger" => {
            // Checks if the input can be converted to Integer
            // Check for singleton first
            if invocation_base.count() > 1 {
                return Err(EvaluationError::SingletonEvaluationError(
                    "convertsToInteger requires a singleton input".to_string(),
                ));
            }
            Ok(match invocation_base {
                // Wrap in Ok
                EvaluationResult::Empty => EvaluationResult::Empty, // Empty input -> Empty result
                // Collections handled by initial check
                EvaluationResult::Collection(_) => unreachable!(),
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
            })
        }
        "convertsToBoolean" => {
            // Checks if the input can be converted to Boolean
            // Check for singleton first
            if invocation_base.count() > 1 {
                return Err(EvaluationError::SingletonEvaluationError(
                    "convertsToBoolean requires a singleton input".to_string(),
                ));
            }
            Ok(match invocation_base {
                // Wrap in Ok
                EvaluationResult::Empty => EvaluationResult::Empty, // Empty input -> Empty result
                // Collections handled by initial check
                EvaluationResult::Collection(_) => unreachable!(),
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
            })
        }
        "toBoolean" => {
            // Converts the input to Boolean according to FHIRPath rules
            // Check for singleton first
            if invocation_base.count() > 1 {
                return Err(EvaluationError::SingletonEvaluationError(
                    "toBoolean requires a singleton input".to_string(),
                ));
            }
            Ok(match invocation_base {
                // Wrap in Ok
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
                // Collections handled by initial check
                EvaluationResult::Collection(_) => unreachable!(),
                // Other types are not convertible
                _ => EvaluationResult::Empty,
            })
        }
        "convertsToString" => {
            // Checks if the input can be converted to String
            // Check for singleton first
            if invocation_base.count() > 1 {
                return Err(EvaluationError::SingletonEvaluationError(
                    "convertsToString requires a singleton input".to_string(),
                ));
            }
            // Handle Empty case explicitly after singleton check
            if invocation_base == &EvaluationResult::Empty {
                 return Ok(EvaluationResult::Empty);
            }
            // Now we know it's a non-empty singleton
            Ok(match invocation_base {
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
                // Add cases for Empty and Collection to satisfy exhaustiveness check
                EvaluationResult::Empty => EvaluationResult::Empty, // Already handled above, but needed for match
                EvaluationResult::Collection(_) => unreachable!(), // Already handled by singleton check
            })
        }
        "toString" => {
            // Converts the input to its string representation using the helper
            // Check for singleton first
            if invocation_base.count() > 1 {
                return Err(EvaluationError::SingletonEvaluationError(
                    "toString requires a singleton input".to_string(),
                ));
            }
            Ok(match invocation_base {
                // Wrap in Ok
                EvaluationResult::Empty => EvaluationResult::Empty, // toString on empty is empty
                // Collections handled by initial check
                EvaluationResult::Collection(_) => unreachable!(),
                // Convert single item to string
                single_item => EvaluationResult::String(single_item.to_string_value()),
            })
        }
        "toDate" => {
            // Converts the input to Date according to FHIRPath rules
            // Check for singleton first
            if invocation_base.count() > 1 {
                return Err(EvaluationError::SingletonEvaluationError(
                    "toDate requires a singleton input".to_string(),
                ));
            }
            Ok(match invocation_base {
                // Wrap in Ok
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
                // Collections handled by initial check
                EvaluationResult::Collection(_) => unreachable!(),
                _ => EvaluationResult::Empty, // Other types cannot convert
            })
        }
        "convertsToDate" => {
            // Checks if the input can be converted to Date
            // Check for singleton first
            if invocation_base.count() > 1 {
                return Err(EvaluationError::SingletonEvaluationError(
                    "convertsToDate requires a singleton input".to_string(),
                ));
            }
            Ok(match invocation_base {
                // Wrap in Ok
                EvaluationResult::Empty => EvaluationResult::Empty,
                // Collections handled by initial check
                EvaluationResult::Collection(_) => unreachable!(),
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
            })
        }
        "toDateTime" => {
            // Converts the input to DateTime according to FHIRPath rules
            // Check for singleton first
            if invocation_base.count() > 1 {
                return Err(EvaluationError::SingletonEvaluationError(
                    "toDateTime requires a singleton input".to_string(),
                ));
            }
            Ok(match invocation_base {
                // Wrap in Ok
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
                // Collections handled by initial check
                EvaluationResult::Collection(_) => unreachable!(),
                _ => EvaluationResult::Empty, // Other types cannot convert
            })
        }
        "convertsToDateTime" => {
            // Checks if the input can be converted to DateTime
            // Check for singleton first
            if invocation_base.count() > 1 {
                return Err(EvaluationError::SingletonEvaluationError(
                    "convertsToDateTime requires a singleton input".to_string(),
                ));
            }
            Ok(match invocation_base {
                // Wrap in Ok
                EvaluationResult::Empty => EvaluationResult::Empty,
                // Collections handled by initial check
                EvaluationResult::Collection(_) => unreachable!(),
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
            })
        }
        "toTime" => {
            // Converts the input to Time according to FHIRPath rules
            // Check for singleton first
            if invocation_base.count() > 1 {
                return Err(EvaluationError::SingletonEvaluationError(
                    "toTime requires a singleton input".to_string(),
                ));
            }
            Ok(match invocation_base {
                // Wrap in Ok
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
                // Collections handled by initial check
                EvaluationResult::Collection(_) => unreachable!(),
                _ => EvaluationResult::Empty, // Other types cannot convert
            })
        }
        "convertsToTime" => {
            // Checks if the input can be converted to Time
            // Check for singleton first
            if invocation_base.count() > 1 {
                return Err(EvaluationError::SingletonEvaluationError(
                    "convertsToTime requires a singleton input".to_string(),
                ));
            }
            Ok(match invocation_base {
                // Wrap in Ok
                EvaluationResult::Empty => EvaluationResult::Empty,
                // Collections handled by initial check
                EvaluationResult::Collection(_) => unreachable!(),
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
            })
        }
        "toQuantity" => {
            // Converts the input to Quantity according to FHIRPath rules
            // The result is just the numeric value (Decimal or Integer) as unit handling is complex
            // Check for singleton first
            if invocation_base.count() > 1 {
                return Err(EvaluationError::SingletonEvaluationError(
                    "toQuantity requires a singleton input".to_string(),
                ));
            }
            Ok(match invocation_base {
                // Wrap in Ok
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
                // Collections handled by initial check
                EvaluationResult::Collection(_) => unreachable!(),
                _ => EvaluationResult::Empty, // Other types cannot convert
            }) // Close the Ok() wrapper here
        }
        "convertsToQuantity" => {
            // Checks if the input can be converted to Quantity
            // Check for singleton first
            if invocation_base.count() > 1 {
                return Err(EvaluationError::SingletonEvaluationError(
                    "convertsToQuantity requires a singleton input".to_string(),
                ));
            }
             // Handle Empty case explicitly after singleton check
            if invocation_base == &EvaluationResult::Empty {
                 return Ok(EvaluationResult::Empty);
            }
            // Now we know it's a non-empty singleton
            Ok(match invocation_base {
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
            })
        }
        "length" => {
            // Returns the length of a string
            // Check for singleton first
            if invocation_base.count() > 1 {
                return Err(EvaluationError::SingletonEvaluationError(
                    "length requires a singleton input".to_string(),
                ));
            }
            Ok(match invocation_base {
                // Wrap in Ok
                EvaluationResult::String(s) => EvaluationResult::Integer(s.chars().count() as i64), // Use chars().count() for correct length
                EvaluationResult::Empty => EvaluationResult::Empty, // Length on empty is empty
                // Collections handled by initial check
                EvaluationResult::Collection(_) => unreachable!(),
                _ => {
                    return Err(EvaluationError::TypeError(
                        "length() requires a String input".to_string(),
                    ));
                }
            })
        }
        "indexOf" => {
            // Returns the 0-based index of the first occurrence of the substring
            if args.len() != 1 {
                return Err(EvaluationError::InvalidArity(
                    "Function 'indexOf' expects 1 argument".to_string(),
                ));
            }
            // Check for singleton base
            if invocation_base.count() > 1 {
                return Err(EvaluationError::SingletonEvaluationError(
                    "indexOf requires a singleton input".to_string(),
                ));
            }
            // Check for singleton argument
            if args[0].count() > 1 {
                return Err(EvaluationError::SingletonEvaluationError(
                    "indexOf requires a singleton argument".to_string(),
                ));
            }
            Ok(match (invocation_base, &args[0]) {
                // Wrap in Ok
                (EvaluationResult::String(s), EvaluationResult::String(substring)) => {
                    match s.find(substring) {
                        Some(index) => EvaluationResult::Integer(index as i64),
                        None => EvaluationResult::Integer(-1),
                    }
                }
                // Handle empty cases according to spec
                (EvaluationResult::String(_), EvaluationResult::Empty) => EvaluationResult::Empty,
                (EvaluationResult::Empty, _) => EvaluationResult::Empty,
                // Invalid types
                _ => {
                    return Err(EvaluationError::TypeError(
                        "indexOf requires String input and argument".to_string(),
                    ));
                }
            })
        }
        "substring" => {
            // Returns a part of the string
            if args.is_empty() || args.len() > 2 {
                return Err(EvaluationError::InvalidArity(
                    "Function 'substring' expects 1 or 2 arguments".to_string(),
                ));
            }
            // Check for singleton base
            if invocation_base.count() > 1 {
                return Err(EvaluationError::SingletonEvaluationError(
                    "substring requires a singleton input".to_string(),
                ));
            }
            // Check for singleton arguments
            if args[0].count() > 1 || args.get(1).map_or(false, |a| a.count() > 1) {
                return Err(EvaluationError::SingletonEvaluationError(
                    "substring requires singleton arguments".to_string(),
                ));
            }

            let start_index_res = &args[0];
            let length_res_opt = args.get(1);

            Ok(match invocation_base {
                // Wrap in Ok
                EvaluationResult::String(s) => {
                    let start_index = match start_index_res {
                        EvaluationResult::Integer(i) if *i >= 0 => *i as usize,
                        // Handle empty start index
                        EvaluationResult::Empty => return Ok(EvaluationResult::Empty),
                        _ => {
                            return Err(EvaluationError::InvalidArgument(
                                "substring start index must be a non-negative integer".to_string(),
                            ));
                        }
                    };

                    // If start index is out of bounds (>= length), return empty string
                    if start_index >= s.chars().count() {
                        // Spec says empty {}, but returning "" is safer for string context
                        return Ok(EvaluationResult::String("".to_string()));
                    }

                    if let Some(length_res) = length_res_opt {
                        // Two arguments: start and length
                        let length = match length_res {
                            // If length is negative, return empty string per spec
                            EvaluationResult::Integer(l) if *l < 0 => {
                                return Ok(EvaluationResult::String("".to_string()));
                            }
                            // If length is non-negative integer, use it
                            EvaluationResult::Integer(l) => *l as usize,
                            // Handle empty length argument (treat as if not provided)
                            EvaluationResult::Empty => s.chars().count() - start_index, // Length to end of string
                            // Any other type for length is invalid
                            _ => {
                                return Err(EvaluationError::InvalidArgument(
                                    "substring length must be an integer".to_string(),
                                ));
                            }
                        };

                        let result: String = s.chars().skip(start_index).take(length).collect();
                        EvaluationResult::String(result)
                    } else {
                        // One argument: start index only (substring to end)
                        let result: String = s.chars().skip(start_index).collect();
                        EvaluationResult::String(result)
                    }
                }
                EvaluationResult::Empty => EvaluationResult::Empty, // Substring on empty is empty
                // Collections handled by initial check
                EvaluationResult::Collection(_) => unreachable!(),
                _ => {
                    return Err(EvaluationError::TypeError(
                        "substring requires a String input".to_string(),
                    ));
                }
            })
        }
        "startsWith" => {
            if args.len() != 1 {
                return Err(EvaluationError::InvalidArity(
                    "Function 'startsWith' expects 1 argument".to_string(),
                ));
            }
            // Check for singleton base and arg
            if invocation_base.count() > 1 || args[0].count() > 1 {
                return Err(EvaluationError::SingletonEvaluationError(
                    "startsWith requires singleton input and argument".to_string(),
                ));
            }
            Ok(match (invocation_base, &args[0]) {
                // Wrap in Ok
                (EvaluationResult::String(s), EvaluationResult::String(prefix)) => {
                    EvaluationResult::Boolean(s.starts_with(prefix))
                }
                // Handle empty cases
                (EvaluationResult::String(_), EvaluationResult::Empty) => EvaluationResult::Empty,
                (EvaluationResult::Empty, _) => EvaluationResult::Empty,
                _ => {
                    return Err(EvaluationError::TypeError(
                        "startsWith requires String input and argument".to_string(),
                    ));
                }
            })
        }
        "endsWith" => {
            if args.len() != 1 {
                return Err(EvaluationError::InvalidArity(
                    "Function 'endsWith' expects 1 argument".to_string(),
                ));
            }
            // Check for singleton base and arg
            if invocation_base.count() > 1 || args[0].count() > 1 {
                return Err(EvaluationError::SingletonEvaluationError(
                    "endsWith requires singleton input and argument".to_string(),
                ));
            }
            Ok(match (invocation_base, &args[0]) {
                // Wrap in Ok
                (EvaluationResult::String(s), EvaluationResult::String(suffix)) => {
                    EvaluationResult::Boolean(s.ends_with(suffix))
                }
                // Handle empty cases
                (EvaluationResult::String(_), EvaluationResult::Empty) => EvaluationResult::Empty,
                (EvaluationResult::Empty, _) => EvaluationResult::Empty,
                _ => {
                    return Err(EvaluationError::TypeError(
                        "endsWith requires String input and argument".to_string(),
                    ));
                }
            })
        }
        "upper" => {
            // Check for singleton base
            if invocation_base.count() > 1 {
                return Err(EvaluationError::SingletonEvaluationError(
                    "upper requires a singleton input".to_string(),
                ));
            }
            Ok(match invocation_base {
                // Wrap in Ok
                EvaluationResult::String(s) => EvaluationResult::String(s.to_uppercase()),
                EvaluationResult::Empty => EvaluationResult::Empty,
                _ => {
                    return Err(EvaluationError::TypeError(
                        "upper requires a String input".to_string(),
                    ));
                }
            })
        }
        "lower" => {
            // Check for singleton base
            if invocation_base.count() > 1 {
                return Err(EvaluationError::SingletonEvaluationError(
                    "lower requires a singleton input".to_string(),
                ));
            }
            Ok(match invocation_base {
                // Wrap in Ok
                EvaluationResult::String(s) => EvaluationResult::String(s.to_lowercase()),
                EvaluationResult::Empty => EvaluationResult::Empty,
                _ => {
                    return Err(EvaluationError::TypeError(
                        "lower requires a String input".to_string(),
                    ));
                }
            })
        }
        "replace" => {
            if args.len() != 2 {
                return Err(EvaluationError::InvalidArity(
                    "Function 'replace' expects 2 arguments".to_string(),
                ));
            }
            // Check for singleton base and args
            if invocation_base.count() > 1 || args[0].count() > 1 || args[1].count() > 1 {
                return Err(EvaluationError::SingletonEvaluationError(
                    "replace requires singleton input and arguments".to_string(),
                ));
            }
            Ok(match (invocation_base, &args[0], &args[1]) {
                // Wrap in Ok
                (
                    EvaluationResult::String(s),
                    EvaluationResult::String(pattern),
                    EvaluationResult::String(substitution),
                ) => EvaluationResult::String(s.replace(pattern, substitution)),
                // Handle empty cases
                (EvaluationResult::Empty, _, _)
                | (_, EvaluationResult::Empty, _)
                | (_, _, EvaluationResult::Empty) => EvaluationResult::Empty,
                _ => {
                    return Err(EvaluationError::TypeError(
                        "replace requires String input and arguments".to_string(),
                    ));
                }
            })
        }
        "matches" => {
            if args.len() != 1 {
                return Err(EvaluationError::InvalidArity(
                    "Function 'matches' expects 1 argument".to_string(),
                ));
            }
            // Check for singleton base and arg
            if invocation_base.count() > 1 || args[0].count() > 1 {
                return Err(EvaluationError::SingletonEvaluationError(
                    "matches requires singleton input and argument".to_string(),
                ));
            }
            Ok(match (invocation_base, &args[0]) {
                // Wrap in Ok
                (EvaluationResult::String(s), EvaluationResult::String(regex_pattern)) => {
                    match Regex::new(regex_pattern) {
                        Ok(re) => EvaluationResult::Boolean(re.is_match(s)),
                        Err(e) => return Err(EvaluationError::InvalidRegex(e.to_string())), // Return Err
                    }
                }
                // Handle empty cases
                (EvaluationResult::String(_), EvaluationResult::Empty) => EvaluationResult::Empty,
                (EvaluationResult::Empty, _) => EvaluationResult::Empty,
                _ => {
                    return Err(EvaluationError::TypeError(
                        "matches requires String input and argument".to_string(),
                    ));
                }
            })
        }
        "replaceMatches" => {
            if args.len() != 2 {
                return Err(EvaluationError::InvalidArity(
                    "Function 'replaceMatches' expects 2 arguments".to_string(),
                ));
            }
            // Check for singleton base and args
            if invocation_base.count() > 1 || args[0].count() > 1 || args[1].count() > 1 {
                return Err(EvaluationError::SingletonEvaluationError(
                    "replaceMatches requires singleton input and arguments".to_string(),
                ));
            }
            Ok(match (invocation_base, &args[0], &args[1]) {
                // Wrap in Ok
                (
                    EvaluationResult::String(s),
                    EvaluationResult::String(regex_pattern),
                    EvaluationResult::String(substitution),
                ) => {
                    match Regex::new(regex_pattern) {
                        Ok(re) => {
                            EvaluationResult::String(re.replace_all(s, substitution).to_string())
                        }
                        Err(e) => return Err(EvaluationError::InvalidRegex(e.to_string())), // Return Err
                    }
                }
                // Handle empty cases
                (EvaluationResult::Empty, _, _)
                | (_, EvaluationResult::Empty, _)
                | (_, _, EvaluationResult::Empty) => EvaluationResult::Empty,
                _ => {
                    return Err(EvaluationError::TypeError(
                        "replaceMatches requires String input and arguments".to_string(),
                    ));
                }
            })
        }
        "toChars" => {
            // Check for singleton base
            if invocation_base.count() > 1 {
                return Err(EvaluationError::SingletonEvaluationError(
                    "toChars requires a singleton input".to_string(),
                ));
            }
            Ok(match invocation_base {
                // Wrap in Ok
                EvaluationResult::String(s) => {
                    if s.is_empty() {
                        EvaluationResult::Empty
                    } else {
                        let chars: Vec<EvaluationResult> = s
                            .chars()
                            .map(|c| EvaluationResult::String(c.to_string()))
                            .collect();
                        normalize_collection_result(chars) // Apply normalization
                    }
                }
                EvaluationResult::Empty => EvaluationResult::Empty,
                // Collections handled by initial check
                EvaluationResult::Collection(_) => unreachable!(),
                _ => {
                    return Err(EvaluationError::TypeError(
                        "toChars requires a String input".to_string(),
                    ));
                }
            })
        }
        "now" => {
            // Returns the current DateTime
            let now = Local::now();
            // Format according to FHIRPath spec (ISO 8601 with timezone offset)
            Ok(EvaluationResult::DateTime(
                now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            ))
        }
        "today" => {
            // Returns the current Date
            let today = Local::now().date_naive();
            // Format as YYYY-MM-DD
            Ok(EvaluationResult::Date(today.format("%Y-%m-%d").to_string()))
        }
        "timeOfDay" => {
            // Returns the current Time
            let now = Local::now();
            // Format as HH:mm:ss.sss (using Millis for consistency with now())
            Ok(EvaluationResult::Time(format!(
                "{:02}:{:02}:{:02}.{:03}",
                now.hour(),
                now.minute(),
                now.second(),
                now.nanosecond() / 1_000_000 // Convert nanoseconds to milliseconds
            )))
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
            Ok(EvaluationResult::Empty) // Return Ok(Empty) for unhandled but potentially valid functions
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
fn evaluate_indexer(
    collection: &EvaluationResult,
    index: &EvaluationResult,
) -> Result<EvaluationResult, EvaluationError> {
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
        None => {
            return Err(EvaluationError::InvalidIndex(format!(
                "Invalid index value: {:?}",
                index
            )));
        }
    };

    // Access the item at the given index
    Ok(match collection {
        // Wrap result in Ok
        EvaluationResult::Collection(items) => {
            items.get(idx).cloned().unwrap_or(EvaluationResult::Empty)
        }
        // Indexer on single item or empty returns empty
        _ => EvaluationResult::Empty,
    })
}

/// Applies a polarity operator to a value
fn apply_polarity(op: char, value: &EvaluationResult) -> Result<EvaluationResult, EvaluationError> {
    match op {
        '+' => Ok(value.clone()), // Unary plus doesn't change the value
        '-' => {
            // Negate numeric values
            Ok(match value {
                // Wrap result in Ok
                EvaluationResult::Decimal(d) => EvaluationResult::Decimal(-*d),
                EvaluationResult::Integer(i) => EvaluationResult::Integer(-*i),
                // Polarity on non-numeric or empty returns empty
                _ => EvaluationResult::Empty,
            })
        }
        _ => Err(EvaluationError::InvalidOperation(format!(
            "Unknown polarity operator: {}",
            op
        ))),
    }
}

/// Applies a multiplicative operator to two values
fn apply_multiplicative(
    left: &EvaluationResult,
    op: &str,
    right: &EvaluationResult,
) -> Result<EvaluationResult, EvaluationError> {
    match op {
        "*" => {
            // Handle multiplication: Int * Int = Int, otherwise Decimal
            Ok(match (left, right) {
                // Wrap result in Ok
                (EvaluationResult::Integer(l), EvaluationResult::Integer(r)) => {
                    // Check for potential overflow before multiplying
                    l.checked_mul(*r)
                        .map(EvaluationResult::Integer)
                        .ok_or(EvaluationError::ArithmeticOverflow)? // Return Err on overflow
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
                // Handle empty operands
                (EvaluationResult::Empty, _) | (_, EvaluationResult::Empty) => {
                    EvaluationResult::Empty
                }
                _ => {
                    return Err(EvaluationError::TypeError(format!(
                        "Cannot multiply {} and {}",
                        left.type_name(),
                        right.type_name()
                    )));
                }
            })
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
                    Err(EvaluationError::DivisionByZero) // Return error
                } else {
                    // Decimal division preserves precision
                    l.checked_div(r)
                        .map(EvaluationResult::Decimal)
                        .ok_or(EvaluationError::ArithmeticOverflow) // Return error on overflow
                }
            } else {
                // Handle empty operands
                if left == &EvaluationResult::Empty || right == &EvaluationResult::Empty {
                    Ok(EvaluationResult::Empty)
                } else {
                    Err(EvaluationError::TypeError(format!(
                        "Cannot divide {} by {}",
                        left.type_name(),
                        right.type_name()
                    )))
                }
            }
        }
        "div" | "mod" => {
            // Handle div/mod: Int/Int -> Int, Dec/Dec -> Int/Dec, mixed -> Error
            match (left, right) {
                (EvaluationResult::Integer(l), EvaluationResult::Integer(r)) => {
                    apply_integer_multiplicative(*l, op, *r) // Returns Result
                }
                (EvaluationResult::Decimal(l), EvaluationResult::Decimal(r)) => {
                    apply_decimal_multiplicative(*l, op, *r) // Returns Result
                }
                // Handle empty operands
                (EvaluationResult::Empty, _) | (_, EvaluationResult::Empty) => {
                    Ok(EvaluationResult::Empty)
                }
                _ => Err(EvaluationError::TypeError(format!(
                    // Mixed types are invalid
                    "Operator '{}' requires operands of the same numeric type (Integer or Decimal), found {} and {}",
                    op,
                    left.type_name(),
                    right.type_name()
                ))),
            }
        }
        _ => Err(EvaluationError::InvalidOperation(format!(
            "Unknown multiplicative operator: {}",
            op
        ))),
    }
}

/// Applies integer-only multiplicative operators (div, mod)
fn apply_integer_multiplicative(
    left: i64,
    op: &str,
    right: i64,
) -> Result<EvaluationResult, EvaluationError> {
    if right == 0 {
        return Err(EvaluationError::DivisionByZero); // Return error
    }
    match op {
        "div" => Ok(EvaluationResult::Integer(left / right)), // Integer division
        "mod" => Ok(EvaluationResult::Integer(left % right)), // Integer modulo
        _ => Err(EvaluationError::InvalidOperation(format!(
            "Unknown integer multiplicative operator: {}",
            op
        ))),
    }
}

/// Applies an additive operator to two values
fn apply_additive(
    left: &EvaluationResult,
    op: &str,
    right: &EvaluationResult,
) -> Result<EvaluationResult, EvaluationError> {
    // The variables left_dec and right_dec were removed as they were unused.
    // The logic below handles type checking and promotion directly.

    match op {
        "+" => {
            // Handle numeric addition: Int + Int = Int, otherwise Decimal
            Ok(match (left, right) {
                // Wrap result in Ok
                (EvaluationResult::Integer(l), EvaluationResult::Integer(r)) => {
                    // Check for potential overflow before adding
                    l.checked_add(*r)
                        .map(EvaluationResult::Integer)
                        .ok_or(EvaluationError::ArithmeticOverflow)? // Return Err on overflow
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
                // Handle String + Number (attempt conversion, prioritize Integer result if possible)
                (EvaluationResult::String(s), EvaluationResult::Integer(i)) => {
                    // Try parsing string as Integer first
                    if let Ok(s_int) = s.parse::<i64>() {
                        s_int
                            .checked_add(*i)
                            .map(EvaluationResult::Integer)
                            .ok_or(EvaluationError::ArithmeticOverflow)? // Handle potential overflow
                    } else {
                        // If not integer, try parsing as Decimal
                        s.parse::<Decimal>()
                            .ok()
                            .map(|d| EvaluationResult::Decimal(d + Decimal::from(*i)))
                            // If string cannot be parsed as number, it's a type error for '+'
                            .ok_or_else(|| {
                                EvaluationError::TypeError(format!(
                                    "Cannot add String '{}' and Integer {}",
                                    s, i
                                ))
                            })?
                    }
                }
                (EvaluationResult::Integer(i), EvaluationResult::String(s)) => {
                    // Try parsing string as Integer first
                    if let Ok(s_int) = s.parse::<i64>() {
                        i.checked_add(s_int)
                            .map(EvaluationResult::Integer)
                            .ok_or(EvaluationError::ArithmeticOverflow)? // Handle potential overflow
                    } else {
                        // If not integer, try parsing as Decimal
                        s.parse::<Decimal>()
                            .ok()
                            .map(|d| EvaluationResult::Decimal(Decimal::from(*i) + d))
                            // If string cannot be parsed as number, it's a type error for '+'
                            .ok_or_else(|| {
                                EvaluationError::TypeError(format!(
                                    "Cannot add Integer {} and String '{}'",
                                    i, s
                                ))
                            })?
                    }
                }
                (EvaluationResult::String(s), EvaluationResult::Decimal(d)) => {
                    // String + Decimal -> Decimal
                    s.parse::<Decimal>()
                        .ok()
                        .map(|sd| EvaluationResult::Decimal(sd + *d))
                        // If string cannot be parsed as number, it's a type error for '+'
                        .ok_or_else(|| {
                            EvaluationError::TypeError(format!(
                                "Cannot add String '{}' and Decimal {}",
                                s, d
                            ))
                        })?
                }
                (EvaluationResult::Decimal(d), EvaluationResult::String(s)) => {
                    s.parse::<Decimal>()
                        .ok()
                        .map(|sd| EvaluationResult::Decimal(*d + sd))
                        // If string cannot be parsed as number, it's a type error for '+'
                        .ok_or_else(|| {
                            EvaluationError::TypeError(format!(
                                "Cannot add Decimal {} and String '{}'",
                                d, s
                            ))
                        })?
                }
                // Handle empty operands
                (EvaluationResult::Empty, _) | (_, EvaluationResult::Empty) => {
                    EvaluationResult::Empty
                }
                // Other combinations are invalid for '+'
                _ => {
                    return Err(EvaluationError::TypeError(format!(
                        "Cannot add {} and {}",
                        left.type_name(),
                        right.type_name()
                    )));
                }
            })
        }
        "-" => {
            // Handle numeric subtraction: Int - Int = Int, otherwise Decimal
            Ok(match (left, right) {
                // Wrap result in Ok
                (EvaluationResult::Integer(l), EvaluationResult::Integer(r)) => {
                    // Check for potential overflow before subtracting
                    l.checked_sub(*r)
                        .map(EvaluationResult::Integer)
                        .ok_or(EvaluationError::ArithmeticOverflow)? // Return Err on overflow
                }
                // If either operand is Decimal, promote and result is Decimal
                (EvaluationResult::Decimal(l), EvaluationResult::Decimal(r)) => {
                    EvaluationResult::Decimal(*l - *r)
                }
                (EvaluationResult::Decimal(l), EvaluationResult::Integer(r)) => {
                    EvaluationResult::Decimal(*l - Decimal::from(*r))
                }
                (EvaluationResult::Integer(l), EvaluationResult::Decimal(r)) => {
                    EvaluationResult::Decimal(Decimal::from(*l) - *r)
                }
                // Handle String - Number (attempt conversion, prioritize Integer result if possible)
                (EvaluationResult::String(s), EvaluationResult::Integer(i)) => {
                    // Try parsing string as Integer first
                    if let Ok(s_int) = s.parse::<i64>() {
                        s_int
                            .checked_sub(*i)
                            .map(EvaluationResult::Integer)
                            .ok_or(EvaluationError::ArithmeticOverflow)? // Handle potential overflow
                    } else {
                        // If not integer, try parsing as Decimal
                        s.parse::<Decimal>()
                            .ok()
                            .map(|d| EvaluationResult::Decimal(d - Decimal::from(*i)))
                            // If string cannot be parsed as number, it's a type error for '-'
                            .ok_or_else(|| {
                                EvaluationError::TypeError(format!(
                                    "Cannot subtract Integer {} from String '{}'",
                                    i, s
                                ))
                            })?
                    }
                }
                (EvaluationResult::Integer(i), EvaluationResult::String(s)) => {
                    // Try parsing string as Integer first
                    if let Ok(s_int) = s.parse::<i64>() {
                        i.checked_sub(s_int)
                            .map(EvaluationResult::Integer)
                            .ok_or(EvaluationError::ArithmeticOverflow)? // Handle potential overflow
                    } else {
                        // If not integer, try parsing as Decimal
                        s.parse::<Decimal>()
                            .ok()
                            .map(|d| EvaluationResult::Decimal(Decimal::from(*i) - d))
                            // If string cannot be parsed as number, it's a type error for '-'
                            .ok_or_else(|| {
                                EvaluationError::TypeError(format!(
                                    "Cannot subtract String '{}' from Integer {}",
                                    s, i
                                ))
                            })?
                    }
                }
                (EvaluationResult::String(s), EvaluationResult::Decimal(d)) => {
                    // String - Decimal -> Decimal
                    s.parse::<Decimal>()
                        .ok()
                        .map(|sd| EvaluationResult::Decimal(sd - *d))
                        // If string cannot be parsed as number, it's a type error for '-'
                        .ok_or_else(|| {
                            EvaluationError::TypeError(format!(
                                "Cannot subtract Decimal {} from String '{}'",
                                d, s
                            ))
                        })?
                }
                (EvaluationResult::Decimal(d), EvaluationResult::String(s)) => {
                    s.parse::<Decimal>()
                        .ok()
                        .map(|sd| EvaluationResult::Decimal(*d - sd))
                        // If string cannot be parsed as number, it's a type error for '-'
                        .ok_or_else(|| {
                            EvaluationError::TypeError(format!(
                                "Cannot subtract String '{}' from Decimal {}",
                                s, d
                            ))
                        })?
                }
                // Handle empty operands
                (EvaluationResult::Empty, _) | (_, EvaluationResult::Empty) => {
                    EvaluationResult::Empty
                }
                // Other combinations are invalid for '-'
                _ => {
                    return Err(EvaluationError::TypeError(format!(
                        "Cannot subtract {} from {}",
                        right.type_name(),
                        left.type_name()
                    )));
                }
            })
        }
        "&" => {
            // Handle string concatenation using '&'
            let left_str = match left {
                EvaluationResult::Empty => "".to_string(),
                _ => left.to_string_value(), // Convert left to string
            };
            let right_str = match right {
                EvaluationResult::Empty => "".to_string(),
                _ => right.to_string_value(), // Convert right to string
            };
            Ok(EvaluationResult::String(format!(
                "{}{}",
                left_str, right_str
            )))
        }
        _ => Err(EvaluationError::InvalidOperation(format!(
            "Unknown additive operator: {}",
            op
        ))),
    }
}

/// Applies a type operation (is/as) to a value
fn apply_type_operation(
    value: &EvaluationResult,
    op: &str,
    type_spec: &TypeSpecifier,
) -> Result<EvaluationResult, EvaluationError> {
    match op {
        "is" => {
            // Check if the value is of the specified type
            // Correctly extract the base type name, ignoring the namespace if present
            let base_type_name = match type_spec {
                TypeSpecifier::QualifiedIdentifier(_ns_or_name, Some(name)) => name, // e.g., System.Integer -> Integer
                TypeSpecifier::QualifiedIdentifier(name, None) => name, // e.g., Integer -> Integer
            };

            // Check if the identifier resolves to a valid type
            // TODO: Need access to model info here to validate type_spec.
            // For now, assume it's valid if parsed.

            // Handle singleton evaluation: 'is' errors on multi-item collections
            if value.count() > 1 {
                return Err(EvaluationError::SingletonEvaluationError(
                    "'is' operator requires a singleton input".to_string(),
                ));
            }

            let is_type = match (base_type_name.as_str(), value) {
                // Compare using base_type_name
                (_, EvaluationResult::Empty) => false, // Empty is not of any type
                // Collections handled by initial check
                (_, EvaluationResult::Collection(_)) => unreachable!(),
                ("Boolean", EvaluationResult::Boolean(_)) => true,
                ("String", EvaluationResult::String(_)) => true,
                ("Integer", EvaluationResult::Integer(_)) => true,
                ("Decimal", EvaluationResult::Decimal(_)) => true, // Check for Decimal
                ("Date", EvaluationResult::Date(_)) => true,
                ("DateTime", EvaluationResult::DateTime(_)) => true,
                ("Time", EvaluationResult::Time(_)) => true,
                // Add more type checks as needed (e.g., Quantity, complex types)
                _ => false,
            };

            Ok(EvaluationResult::Boolean(is_type))
        }
        "as" => {
            // Cast the value to the specified type if possible
            // Correctly extract the base type name as &str
            let base_type_name: &str = match type_spec {
                TypeSpecifier::QualifiedIdentifier(_ns_or_name, Some(name)) => name.as_str(), // Prefix unused var with _
                TypeSpecifier::QualifiedIdentifier(name, None) => name.as_str(), // Use .as_str()
            };

            // Check if the identifier resolves to a valid type
            // TODO: Need access to model info here to validate type_spec.
            // For now, assume it's valid if parsed.

            // Handle singleton evaluation: 'as' errors on multi-item collections
            if value.count() > 1 {
                return Err(EvaluationError::SingletonEvaluationError(
                    "'as' operator requires a singleton input".to_string(),
                ));
            }

            Ok(match (base_type_name, value) {
                // Match on &str
                (_, EvaluationResult::Empty) => EvaluationResult::Empty, // 'as' on empty is empty
                // Collections handled by initial check
                (_, EvaluationResult::Collection(_)) => unreachable!(),
                // Return the value if it's already of the right type.
                ("Boolean", EvaluationResult::Boolean(_)) => value.clone(),
                ("String", EvaluationResult::String(_)) => value.clone(),
                ("Integer", EvaluationResult::Integer(_)) => value.clone(),
                ("Decimal", EvaluationResult::Decimal(_)) => value.clone(), // Return if already Decimal
                ("Date", EvaluationResult::Date(_)) => value.clone(),
                ("DateTime", EvaluationResult::DateTime(_)) => value.clone(),
                ("Time", EvaluationResult::Time(_)) => value.clone(),
                // Add more type checks as needed
                // If type doesn't match, return Empty
                _ => EvaluationResult::Empty,
            })
        }
        _ => Err(EvaluationError::InvalidOperation(format!(
            "Unknown type operator: {}",
            op
        ))),
    }
}

/// Combines two collections into a union
fn union_collections(left: &EvaluationResult, right: &EvaluationResult) -> EvaluationResult {
    // Returns EvaluationResult, not Result
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

    // Removed unused `result` variable assignment
    let mut union_items = Vec::new();
    // Use HashSet to track items already added to ensure uniqueness based on FHIRPath equality
    let mut added_items_set = HashSet::new();

    // Add items from the left collection if they haven't been added
    // Now iterates over `left_items` directly, which hasn't been moved
    for item in left_items {
        if added_items_set.insert(item.clone()) {
            union_items.push(item); // Push the original item, not a clone from `result`
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


/// Compares two values for inequality - Returns Result now
fn compare_inequality(
    left: &EvaluationResult,
    op: &str,
    right: &EvaluationResult,
) -> Result<EvaluationResult, EvaluationError> { // Changed return type
    // Handle empty operands: comparison with empty returns empty
    if left == &EvaluationResult::Empty || right == &EvaluationResult::Empty {
        return Ok(EvaluationResult::Empty); // Return Ok(Empty)
    }

    // Check for collection vs singleton comparison (error)
    if left.is_collection() != right.is_collection() {
        return Err(EvaluationError::TypeError(format!(
            "Cannot compare {} and {}",
            left.type_name(),
            right.type_name()
        )));
    }
    // If both are collections, comparison is not defined (error)
    if left.is_collection() { // && right.is_collection() implicitly
         return Err(EvaluationError::TypeError(format!(
            "Cannot compare collections using '{}'", op
        )));
    }

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
        // Incomparable types - Return error instead of None/Empty
        _ => {
            return Err(EvaluationError::TypeError(format!(
                "Cannot compare {} and {}",
                left.type_name(),
                right.type_name()
            )));
        }
    };

    // compare_result is now guaranteed to be Some(Ordering) if we reach here
    let ordering = compare_result.unwrap(); // Safe to unwrap

    let result = match op {
        "<" => ordering.is_lt(),
        "<=" => ordering.is_le(),
        ">" => ordering.is_gt(),
        ">=" => ordering.is_ge(),
        _ => false, // Should not happen
    };
    Ok(EvaluationResult::Boolean(result)) // Return Ok result
}

/// Compares two values for equality - Returns Result now
fn compare_equality(
    left: &EvaluationResult,
    op: &str,
    right: &EvaluationResult,
) -> Result<EvaluationResult, EvaluationError> { // Changed return type
    // Helper function for string equivalence normalization
    fn normalize_string(s: &str) -> String {
        let trimmed = s.trim();
        let words: Vec<&str> = trimmed.split_whitespace().collect();
        words.join(" ").to_lowercase()
    }

    match op {
        "=" => {
            // FHIRPath Spec 5.1 Equality (=, !=): If either operand is empty, the result is empty.
            if left == &EvaluationResult::Empty || right == &EvaluationResult::Empty {
                return Ok(EvaluationResult::Empty); // Return Ok(Empty)
            }
            // Strict equality: Order and duplicates matter for collections
            Ok(match (left, right) { // Wrap result in Ok
                (EvaluationResult::Collection(l_items), EvaluationResult::Collection(r_items)) => {
                    if l_items.len() != r_items.len() {
                        EvaluationResult::Boolean(false)
                    } else {
                        // Compare element by element using '=' recursively
                        let all_equal = l_items
                            .iter()
                            .zip(r_items.iter())
                            .all(|(li, ri)| compare_equality(li, "=", ri).map_or(false, |r| r.to_boolean())); // Handle potential error from recursive call
                        EvaluationResult::Boolean(all_equal)
                    }
                }
                // If only one is a collection, they are not equal
                (EvaluationResult::Collection(_), _) | (_, EvaluationResult::Collection(_)) => {
                    EvaluationResult::Boolean(false)
                }
                // Primitive comparison (Empty case handled above)
                 (EvaluationResult::Boolean(l), EvaluationResult::Boolean(r)) => {
                    EvaluationResult::Boolean(l == r)
                }
                (EvaluationResult::String(l), EvaluationResult::String(r)) => {
                    EvaluationResult::Boolean(l == r)
                }
                (EvaluationResult::Decimal(l), EvaluationResult::Decimal(r)) => {
                    EvaluationResult::Boolean(l == r)
                }
                (EvaluationResult::Integer(l), EvaluationResult::Integer(r)) => {
                    EvaluationResult::Boolean(l == r)
                }
                (EvaluationResult::Decimal(l), EvaluationResult::Integer(r)) => {
                    EvaluationResult::Boolean(*l == Decimal::from(*r))
                }
                (EvaluationResult::Integer(l), EvaluationResult::Decimal(r)) => {
                    EvaluationResult::Boolean(Decimal::from(*l) == *r)
                }
                (EvaluationResult::Date(l), EvaluationResult::Date(r)) => {
                    EvaluationResult::Boolean(l == r)
                }
                (EvaluationResult::DateTime(l), EvaluationResult::DateTime(r)) => {
                    EvaluationResult::Boolean(l == r)
                }
                (EvaluationResult::Time(l), EvaluationResult::Time(r)) => {
                    EvaluationResult::Boolean(l == r)
                }
                // Any other combination is an error for '='
                _ => return Err(EvaluationError::TypeError(format!(
                        "Cannot compare {} and {} using '='",
                        left.type_name(),
                        right.type_name()
                    ))),
            }) // This parenthesis now correctly closes the Ok() started above
        }
        "!=" => {
            // FHIRPath Spec 5.1 Equality (=, !=): If either operand is empty, the result is empty.
            if left == &EvaluationResult::Empty || right == &EvaluationResult::Empty {
                return Ok(EvaluationResult::Empty); // Return Ok(Empty)
            }
            // Strict inequality: Negation of '='
            let eq_result = compare_equality(left, "=", right)?; // Propagate error
            Ok(match eq_result { // Wrap result in Ok
                EvaluationResult::Boolean(b) => EvaluationResult::Boolean(!b),
                // If '=' returned Empty (due to empty operand), '!=' also returns Empty
                EvaluationResult::Empty => EvaluationResult::Empty,
                _ => EvaluationResult::Empty, // Should not happen otherwise
            })
        }
        "~" => {
            // Equivalence: Order doesn't matter, duplicates DO matter.
            Ok(match (left, right) { // Wrap result in Ok
                // Handle Empty cases specifically for '~'
                (EvaluationResult::Empty, EvaluationResult::Empty) => {
                    EvaluationResult::Boolean(true)
                }
                (EvaluationResult::Empty, _) | (_, EvaluationResult::Empty) => {
                    EvaluationResult::Boolean(false)
                } // Empty is only equivalent to Empty
                // String equivalence (normalized)
                (EvaluationResult::String(l), EvaluationResult::String(r)) => {
                    EvaluationResult::Boolean(normalize_string(l) == normalize_string(r))
                }
                // Collection equivalence: Order doesn't matter, duplicates DO matter.
                (EvaluationResult::Collection(l_items), EvaluationResult::Collection(r_items)) => {
                    if l_items.len() != r_items.len() {
                        EvaluationResult::Boolean(false) // Different counts cannot be equivalent
                    } else {
                        // Sort copies of the collections
                        let mut l_sorted = l_items.clone();
                        let mut r_sorted = r_items.clone();
                        // Note: Sorting requires Ord. EvaluationResult implements Ord.
                        l_sorted.sort();
                        r_sorted.sort();

                        // Compare sorted collections element-wise using '~' recursively
                        let all_equivalent = l_sorted
                            .iter()
                            .zip(r_sorted.iter())
                            .all(|(li, ri)| compare_equality(li, "~", ri).map_or(false, |r| r.to_boolean())); // Handle potential error
                        EvaluationResult::Boolean(all_equivalent)
                    }
                }
                // If only one is a collection, they are not equivalent (Empty case handled earlier)
                (EvaluationResult::Collection(_), _) | (_, EvaluationResult::Collection(_)) => {
                    EvaluationResult::Boolean(false)
                }
                // Primitive equivalence falls back to strict equality ('=')
                _ => compare_equality(left, "=", right)?, // Propagate error
            })
        }
        "!~" => {
            // Non-equivalence: Negation of '~'
            // Handle empty cases specifically for '!~'
            Ok(match (left, right) { // Wrap result in Ok
                (EvaluationResult::Empty, EvaluationResult::Empty) => {
                    EvaluationResult::Boolean(false)
                } // Empty is equivalent to Empty
                (EvaluationResult::Empty, _) | (_, EvaluationResult::Empty) => {
                    EvaluationResult::Boolean(true)
                } // Empty is not equivalent to non-empty
                // For non-empty operands, negate the result of '~'
                _ => {
                    let equiv_result = compare_equality(left, "~", right)?; // Propagate error
                    match equiv_result {
                        EvaluationResult::Boolean(b) => EvaluationResult::Boolean(!b),
                        // If '~' somehow returned Empty for non-empty operands, propagate Empty
                        EvaluationResult::Empty => EvaluationResult::Empty,
                        _ => EvaluationResult::Empty, // Should not happen
                    }
                }
            })
        }
        _ => Err(EvaluationError::InvalidOperation(format!("Unknown equality operator: {}", op))), // Return error
    }
}

/// Checks membership of a value in a collection
fn check_membership(
    left: &EvaluationResult,
    op: &str,
    right: &EvaluationResult,
) -> Result<EvaluationResult, EvaluationError> {
    // Specific handling for 'in' and 'contains' based on FHIRPath spec regarding empty collections
    match op {
        "in" => {
            // Spec: {} in X -> {}
            if left == &EvaluationResult::Empty {
                return Ok(EvaluationResult::Empty);
            }
            // Spec: X in {} -> false
            if right == &EvaluationResult::Empty {
                return Ok(EvaluationResult::Boolean(false));
            }
            // Check for multi-item left operand (error)
            if left.count() > 1 {
                return Err(EvaluationError::SingletonEvaluationError(
                    "'in' operator requires singleton left operand".to_string(),
                ));
            }
            // Proceed with check if both are non-empty (operands are not Empty)
            let is_in = match right {
                EvaluationResult::Collection(items) => items
                    .iter()
                    // Use map_or to handle potential error from compare_equality
                    .any(|item| compare_equality(left, "=", item).map_or(false, |r| r.to_boolean())),
                // If right is a single non-empty item, compare directly
                // Use map_or to handle potential error from compare_equality
                single_item => compare_equality(left, "=", single_item).map_or(false, |r| r.to_boolean()),
            };

            Ok(EvaluationResult::Boolean(is_in))
        }
        "contains" => {
            // Spec: X contains {} -> {}
            if right == &EvaluationResult::Empty {
                return Ok(EvaluationResult::Empty);
            }
            // Spec: {} contains X -> false (where X is not empty)
            if left == &EvaluationResult::Empty {
                return Ok(EvaluationResult::Boolean(false));
            }
            // Check for multi-item right operand (error)
            if right.count() > 1 {
                return Err(EvaluationError::SingletonEvaluationError(
                    "'contains' operator requires singleton right operand".to_string(),
                ));
            }
            // Proceed with check if both operands are non-empty
            Ok(match left {
                // Wrap result in Ok
                // For collections, check if any item equals the right value
                EvaluationResult::Collection(items) => {
                    // Use map_or to handle potential error from compare_equality
                    let contains = items
                        .iter()
                        .any(|item| compare_equality(item, "=", right).map_or(false, |r| r.to_boolean()));
                    EvaluationResult::Boolean(contains)
                }
                // For strings, check if the string contains the substring
                EvaluationResult::String(s) => match right {
                    EvaluationResult::String(substr) => {
                        EvaluationResult::Boolean(s.contains(substr))
                    }
                    // Contains on string requires string argument, otherwise error
                    _ => {
                        return Err(EvaluationError::TypeError(format!(
                            "'contains' on String requires String argument, found {}",
                            right.type_name()
                        )));
                    }
                },
                // Treat single non-empty item as collection of one
                // Use map_or to handle potential error from compare_equality
                single_item => EvaluationResult::Boolean(
                    compare_equality(single_item, "=", right).map_or(false, |r| r.to_boolean()),
                ),
            })
        }
        _ => Err(EvaluationError::InvalidOperation(format!(
            "Unknown membership operator: {}",
            op
        ))),
    }
}
