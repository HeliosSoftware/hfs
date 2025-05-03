use crate::parser::{Expression, Invocation, Literal, Term, TypeSpecifier}; // Re-added TypeSpecifier
use fhir::FhirResource;
use fhirpath_support::{EvaluationResult, IntoEvaluationResult};
use rust_decimal::prelude::*;
use rust_decimal::Decimal;
// Removed unused import: use rust_decimal_macros::dec;
use std::collections::HashMap;

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
                if left_result == EvaluationResult::Empty || right_result == EvaluationResult::Empty {
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
            // Handle $this invocation
            if let Invocation::This = invocation {
                // Let the if/else block evaluate to the result, which becomes the
                // implicit return value for this match arm if invocation is $this.
                if let Some(item) = current_item.cloned() {
                    item // Evaluate to the item if Some
                } else {
                    // Evaluate to the default context if None
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
                }
            }

            // Check if this is a variable reference (starting with %)
            if let Invocation::Member(name) = invocation {
                if name.starts_with('%') {
                    let var_name = &name[1..]; // Remove the % prefix
                    // Handle %context explicitly if needed, otherwise lookup variable
                    if var_name == "context" {
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
                        return context.get_variable_as_result(var_name);
                    }
                }
            }

            // Determine the base context for the invocation
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

            evaluate_invocation(&base_context, invocation, context)
        }
        Term::Literal(literal) => evaluate_literal(literal),
        Term::ExternalConstant(name) => {
            // Look up external constant in the context
            context.get_variable_as_result(name)
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
                 eprintln!("Warning: DateTime literal evaluated without time part: {}", d);
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
                 n.to_i64().map(EvaluationResult::Integer).unwrap_or_else(|| EvaluationResult::Decimal(*n))
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
            if name == "true" && matches!(invocation_base, EvaluationResult::Empty) { // Only if base is empty context
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
                    }
                     else {
                        EvaluationResult::Collection(flattened_results)
                    }
                }
                // Accessing member on primitive types or Empty returns Empty
                _ => EvaluationResult::Empty,
            }
        }
        Invocation::Function(name, args_exprs) => { // Use args_exprs (AST)
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
                // Add other functions taking lambdas here (e.g., all, any, repeat)
                _ => {
                    // Default: Evaluate all arguments first (without $this context), then call function
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

    if filtered_items.is_empty() {
        EvaluationResult::Empty
    } else if filtered_items.len() == 1 {
        filtered_items.into_iter().next().unwrap() // Return single item directly
    }
     else {
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

     if projected_items.is_empty() {
        EvaluationResult::Empty
    } else if projected_items.len() == 1 {
        projected_items.into_iter().next().unwrap() // Return single item directly
    }
     else {
        EvaluationResult::Collection(projected_items)
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
            match invocation_base { // Use invocation_base, not context
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
        "contains" => { // Function call version
            // Check if the invocation_base contains the argument
            if args.is_empty() {
                return EvaluationResult::Empty; // Requires one argument
            }
            let arg = &args[0];

            match invocation_base {
                EvaluationResult::String(s) => {
                    // String contains substring
                    if let EvaluationResult::String(substr) = arg {
                        EvaluationResult::Boolean(s.contains(substr))
                    } else {
                        EvaluationResult::Empty // Invalid argument type for string contains
                    }
                }
                EvaluationResult::Collection(items) => {
                    // Collection contains item (using equality)
                    let contains = items
                        .iter()
                        .any(|item| compare_equality(item, "=", arg).to_boolean());
                    EvaluationResult::Boolean(contains)
                }
                 // contains on single non-collection/non-string item
                 EvaluationResult::Empty => EvaluationResult::Boolean(false), // Empty cannot contain anything
                 single_item => {
                     // Treat as single-item collection: check if the item equals the argument
                     EvaluationResult::Boolean(compare_equality(single_item, "=", arg).to_boolean())
                 }
            }
        }
        "length" => {
            // Returns the length of a string
            match invocation_base {
                EvaluationResult::String(s) => EvaluationResult::Integer(s.chars().count() as i64), // Use chars().count() for correct length
                _ => EvaluationResult::Empty, // Length only defined for strings
            }
        }
        // where and select are handled in evaluate_invocation
        // Add other standard functions here
        _ => {
             eprintln!("Warning: Unsupported function called: {}", name);
             EvaluationResult::Empty
        }
    }
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
    // Promote Integer to Decimal for mixed operations
    let (left_num, right_num) = match (left, right) {
        (EvaluationResult::Decimal(l), EvaluationResult::Decimal(r)) => (Some(*l), Some(*r)),
        (EvaluationResult::Integer(l), EvaluationResult::Integer(r)) => {
            // Keep as Integer if both are Integer for div/mod
            if op == "div" || op == "mod" {
                return apply_integer_multiplicative(*l, op, *r);
            }
            (Some(Decimal::from(*l)), Some(Decimal::from(*r)))
        }
        (EvaluationResult::Decimal(l), EvaluationResult::Integer(r)) => {
            (Some(*l), Some(Decimal::from(*r)))
        }
        (EvaluationResult::Integer(l), EvaluationResult::Decimal(r)) => {
            (Some(Decimal::from(*l)), Some(*r))
        }
        _ => (None, None),
    };

    if let (Some(l), Some(r)) = (left_num, right_num) {
        match op {
            "*" => EvaluationResult::Decimal(l * r),
            "/" => {
                if r.is_zero() {
                    EvaluationResult::Empty
                } else {
                    // Decimal division preserves precision
                    l.checked_div(r)
                        .map(EvaluationResult::Decimal)
                        .unwrap_or(EvaluationResult::Empty) // Handle potential overflow/errors
                }
            }
            // div and mod handled by integer path or return Empty if mixed with non-integer
            _ => EvaluationResult::Empty, // Unknown operator or invalid type combination for div/mod
        }
    } else {
        // Operands are not compatible numeric types for *, /
        EvaluationResult::Empty
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
        _ => EvaluationResult::Empty, // Should not happen if called correctly
    }
}


/// Applies an additive operator to two values
fn apply_additive(left: &EvaluationResult, op: &str, right: &EvaluationResult) -> EvaluationResult {
    match op {
        "+" => {
            // Promote Integer to Decimal for mixed operations
            let (left_num, right_num) = match (left, right) {
                (EvaluationResult::Decimal(l), EvaluationResult::Decimal(r)) => (Some(*l), Some(*r)),
                (EvaluationResult::Integer(l), EvaluationResult::Integer(r)) => {
                    (Some(Decimal::from(*l)), Some(Decimal::from(*r)))
                }
                (EvaluationResult::Decimal(l), EvaluationResult::Integer(r)) => {
                    (Some(*l), Some(Decimal::from(*r)))
                }
                (EvaluationResult::Integer(l), EvaluationResult::Decimal(r)) => {
                    (Some(Decimal::from(*l)), Some(*r))
                }
                _ => (None, None),
            };
            // Handle numeric addition
            if let (Some(l), Some(r)) = (left_num, right_num) {
                EvaluationResult::Decimal(l + r) // Correct: Use addition for '+'
            // Handle string concatenation
            } else if let (EvaluationResult::String(l), EvaluationResult::String(r)) = (left, right) {
                EvaluationResult::String(format!("{}{}", l, r)) // Correct: Concatenate for '+'
            } else {
                // Addition is not defined for other type combinations
                EvaluationResult::Empty
            }
        }
        "-" => {
            // Promote Integer to Decimal for mixed operations
            let (left_num, right_num) = match (left, right) {
                (EvaluationResult::Decimal(l), EvaluationResult::Decimal(r)) => (Some(*l), Some(*r)),
                (EvaluationResult::Integer(l), EvaluationResult::Integer(r)) => {
                    (Some(Decimal::from(*l)), Some(Decimal::from(*r)))
                }
                (EvaluationResult::Decimal(l), EvaluationResult::Integer(r)) => {
                    (Some(*l), Some(Decimal::from(*r)))
                }
                (EvaluationResult::Integer(l), EvaluationResult::Decimal(r)) => {
                    (Some(Decimal::from(*l)), Some(*r))
                }
                _ => (None, None),
            };
            // Handle numeric subtraction
            if let (Some(l), Some(r)) = (left_num, right_num) {
                EvaluationResult::Decimal(l - r) // Correct: Use subtraction for '-'
            } else {
                // Subtraction is only defined for numeric types
                EvaluationResult::Empty
            }
        }
        _ => EvaluationResult::Empty,
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
                (EvaluationResult::Decimal(l), EvaluationResult::Integer(r)) => *l == Decimal::from(*r),
                (EvaluationResult::Integer(l), EvaluationResult::Decimal(r)) => Decimal::from(*l) == *r,
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
                (EvaluationResult::Decimal(l), EvaluationResult::Integer(r)) => *l != Decimal::from(*r),
                (EvaluationResult::Integer(l), EvaluationResult::Decimal(r)) => Decimal::from(*l) != *r,
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
    match op {
        "in" => {
            // Check if left is in right
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
