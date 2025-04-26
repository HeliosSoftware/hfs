use crate::parser::{Expression, Invocation, Literal, Term, TypeSpecifier};
// Removed prelude imports as they caused E0432
use fhir::FhirResource;
use fhirpath_support::{EvaluationResult, IntoEvaluationResult};
use std::collections::HashMap;
// Import specific version modules needed for type annotations
#[cfg(feature = "R4")]
use fhir::r4;
#[cfg(feature = "R4B")]
use fhir::r4b;
#[cfg(feature = "R5")]
use fhir::r5;
#[cfg(feature = "R6")]
use fhir::r6;

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

/// Evaluates a FHIRPath expression in the given context
pub fn evaluate(expr: &Expression, context: &EvaluationContext) -> EvaluationResult {
    match expr {
        Expression::Term(term) => evaluate_term(term, context),
        Expression::Invocation(left, invocation) => {
            let left_result = evaluate(left, context);
            evaluate_invocation(&left_result, invocation, context)
        }
        Expression::Indexer(left, index) => {
            let left_result = evaluate(left, context);
            let index_result = evaluate(index, context);
            evaluate_indexer(&left_result, &index_result)
        }
        Expression::Polarity(op, expr) => {
            let result = evaluate(expr, context);
            apply_polarity(*op, &result)
        }
        Expression::Multiplicative(left, op, right) => {
            let left_result = evaluate(left, context);
            let right_result = evaluate(right, context);
            apply_multiplicative(&left_result, op, &right_result)
        }
        Expression::Additive(left, op, right) => {
            let left_result = evaluate(left, context);
            let right_result = evaluate(right, context);
            apply_additive(&left_result, op, &right_result)
        }
        Expression::Type(left, op, type_spec) => {
            let result = evaluate(left, context);
            apply_type_operation(&result, op, type_spec)
        }
        Expression::Union(left, right) => {
            let left_result = evaluate(left, context);
            let right_result = evaluate(right, context);
            union_collections(&left_result, &right_result)
        }
        Expression::Inequality(left, op, right) => {
            let left_result = evaluate(left, context);
            let right_result = evaluate(right, context);
            compare_inequality(&left_result, op, &right_result)
        }
        Expression::Equality(left, op, right) => {
            let left_result = evaluate(left, context);
            let right_result = evaluate(right, context);
            compare_equality(&left_result, op, &right_result)
        }
        Expression::Membership(left, op, right) => {
            let left_result = evaluate(left, context);
            let right_result = evaluate(right, context);
            check_membership(&left_result, op, &right_result)
        }
        Expression::And(left, right) => {
            let left_result = evaluate(left, context);
            // Short-circuit evaluation
            if !left_result.to_boolean() {
                return EvaluationResult::Boolean(false);
            }
            let right_result = evaluate(right, context);
            EvaluationResult::Boolean(right_result.to_boolean())
        }
        Expression::Or(left, op, right) => {
            let left_result = evaluate(left, context);
            // Short-circuit for 'or'
            if op == "or" && left_result.to_boolean() {
                return EvaluationResult::Boolean(true);
            }
            let right_result = evaluate(right, context);
            if op == "or" {
                EvaluationResult::Boolean(left_result.to_boolean() || right_result.to_boolean())
            } else {
                // xor
                EvaluationResult::Boolean(left_result.to_boolean() != right_result.to_boolean())
            }
        }
        Expression::Implies(left, right) => {
            let left_result = evaluate(left, context);
            // If the left side is false, the implication is true
            if !left_result.to_boolean() {
                return EvaluationResult::Boolean(true);
            }
            // Otherwise, the result is the same as the right side
            let right_result = evaluate(right, context);
            EvaluationResult::Boolean(right_result.to_boolean())
        }
        Expression::Lambda(_, _) => {
            // Lambda expressions are not directly evaluated
            // They are used in function calls
            EvaluationResult::Empty
        }
    }
}

/// Evaluates a term in the given context
fn evaluate_term(term: &Term, context: &EvaluationContext) -> EvaluationResult {
    match term {
        Term::Invocation(invocation) => {
            // Check if this is a variable reference (starting with %)
            if let Invocation::Member(name) = invocation {
                if name.starts_with('%') {
                    let var_name = &name[1..]; // Remove the % prefix
                    return context.get_variable_as_result(var_name);
                }
            }

            if context.resources.is_empty() {
                // If there are no resources, return Empty for resource-based invocations
                evaluate_invocation(&EvaluationResult::Empty, invocation, context)
            } else {
                // Convert the first FHIR resource to an EvaluationResult for processing
                // In a more complete implementation, we might need to handle multiple resources
                let resource_result = convert_resource_to_result(&context.resources[0]);
                evaluate_invocation(&resource_result, invocation, context)
            }
        }
        Term::Literal(literal) => evaluate_literal(literal),
        Term::ExternalConstant(name) => {
            // Look up external constant in the context
            context.get_variable_as_result(name)
        }
        Term::Parenthesized(expr) => evaluate(expr, context),
    }
}

/// Converts a FHIR resource to an EvaluationResult
fn convert_resource_to_result(resource: &FhirResource) -> EvaluationResult {
    // This is a simplified conversion - in a real implementation,
    // you would convert the resource to a proper object representation
    // that can be navigated with FHIRPath.
    // This needs to be significantly expanded for full FHIRPath support.
    match resource {
        #[cfg(feature = "R4")]
        FhirResource::R4(r) => {
            // Convert R4 resource to an object representation
            let mut obj = HashMap::new();
            // Match on the specific resource type inside the Box to access fields
            match &**r {
                // Dereference Box -> Resource enum -> reference inner struct
                r4::Resource::Account(inner) => {
                    if let Some(id_element) = &inner.id {
                        // Access 'id' field directly
                        obj.insert("id".to_string(), id_element.into_evaluation_result());
                    }
                    // Add other fields from Account if needed
                }
                r4::Resource::Patient(inner) => {
                    if let Some(id_element) = &inner.id {
                        obj.insert("id".to_string(), id_element.into_evaluation_result());
                    }
                    // Add other fields from Patient if needed
                }
                // Add cases for other R4 resource types as needed...
                _ => { /* Resource type not handled or doesn't have 'id' */ }
            }
            EvaluationResult::Object(obj)
        }
        #[cfg(feature = "R4B")]
        FhirResource::R4B(r) => {
            let mut obj = HashMap::new();
            match &**r {
                r4b::Resource::Account(inner) => {
                    if let Some(id_element) = &inner.id {
                        obj.insert("id".to_string(), id_element.into_evaluation_result());
                    }
                }
                r4b::Resource::Patient(inner) => {
                    if let Some(id_element) = &inner.id {
                        obj.insert("id".to_string(), id_element.into_evaluation_result());
                    }
                }
                // Add cases for other R4B resource types as needed...
                _ => { /* Resource type not handled or doesn't have 'id' */ }
            }
            EvaluationResult::Object(obj)
        }
        #[cfg(feature = "R5")]
        FhirResource::R5(r) => {
            let mut obj = HashMap::new();
            match &**r {
                r5::Resource::Account(inner) => {
                    if let Some(id_element) = &inner.id {
                        obj.insert("id".to_string(), id_element.into_evaluation_result());
                    }
                }
                r5::Resource::Patient(inner) => {
                    if let Some(id_element) = &inner.id {
                        obj.insert("id".to_string(), id_element.into_evaluation_result());
                    }
                }
                // Add cases for other R5 resource types as needed...
                _ => { /* Resource type not handled or doesn't have 'id' */ }
            }
            EvaluationResult::Object(obj)
        }
        #[cfg(feature = "R6")]
        FhirResource::R6(r) => {
            let mut obj = HashMap::new();
            match &**r {
                r6::Resource::Account(inner) => {
                    if let Some(id_element) = &inner.id {
                        obj.insert("id".to_string(), id_element.into_evaluation_result());
                    }
                }
                r6::Resource::Patient(inner) => {
                    if let Some(id_element) = &inner.id {
                        obj.insert("id".to_string(), id_element.into_evaluation_result());
                    }
                }
                // Add cases for other R6 resource types as needed...
                _ => { /* Resource type not handled or doesn't have 'id' */ }
            }
            EvaluationResult::Object(obj)
        }
    }
}

/// Evaluates a literal value
fn evaluate_literal(literal: &Literal) -> EvaluationResult {
    match literal {
        Literal::Null => EvaluationResult::Empty,
        Literal::Boolean(b) => EvaluationResult::Boolean(*b),
        Literal::String(s) => EvaluationResult::String(s.clone()),
        Literal::Number(n) => EvaluationResult::Number(*n),
        Literal::LongNumber(n) => EvaluationResult::Integer(*n),
        Literal::Date(d) => EvaluationResult::Date(d.clone()),
        Literal::DateTime(d, t) => {
            if let Some((time, _)) = t {
                EvaluationResult::DateTime(format!("{}T{}", d, time))
            } else {
                EvaluationResult::DateTime(d.clone())
            }
        }
        Literal::Time(t) => EvaluationResult::Time(t.clone()),
        Literal::Quantity(n, _) => {
            // For now, we ignore the unit and just return the number
            // In a full implementation, we would handle units properly
            EvaluationResult::Number(*n)
        }
    }
}

/// Evaluates an invocation on a value
fn evaluate_invocation(
    value: &EvaluationResult,
    invocation: &Invocation,
    context: &EvaluationContext,
) -> EvaluationResult {
    match invocation {
        Invocation::Member(name) => {
            // Special handling for boolean literals that might be parsed as identifiers
            if name == "true" {
                return EvaluationResult::Boolean(true);
            } else if name == "false" {
                return EvaluationResult::Boolean(false);
            }

            // Check if this is a function call without parentheses (e.g., "string.contains")
            if name == "contains" {
                // For string contains without arguments, we need to handle it specially
                // This is a workaround for the parser not handling method calls without parentheses
                return call_function(name, value, &[]);
            }

            // Access a member of the value
            match value {
                EvaluationResult::Object(obj) => {
                    obj.get(name).cloned().unwrap_or(EvaluationResult::Empty)
                }
                EvaluationResult::Collection(items) => {
                    // For collections, we apply the member access to each item
                    let results: Vec<EvaluationResult> = items
                        .iter()
                        .filter_map(|item| {
                            if let EvaluationResult::Object(obj) = item {
                                obj.get(name).cloned()
                            } else {
                                None
                            }
                        })
                        .collect();

                    if results.is_empty() {
                        EvaluationResult::Empty
                    } else {
                        EvaluationResult::Collection(results)
                    }
                }
                // Special handling for empty values
                EvaluationResult::Empty => {
                    // Empty values return empty for any member access
                    EvaluationResult::Empty
                }
                _ => EvaluationResult::Empty,
            }
        }
        Invocation::Function(name, args) => {
            // Evaluate function arguments
            let evaluated_args: Vec<EvaluationResult> =
                args.iter().map(|arg| evaluate(arg, context)).collect();

            // Call the appropriate function
            call_function(name, value, &evaluated_args)
        }
        Invocation::This => {
            if context.resources.is_empty() {
                EvaluationResult::Empty
            } else {
                // Return the first resource as the context
                convert_resource_to_result(&context.resources[0])
            }
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

/// Calls a FHIRPath function
fn call_function(
    name: &str,
    context: &EvaluationResult,
    args: &[EvaluationResult],
) -> EvaluationResult {
    match name {
        "count" => {
            // Returns the number of items in the collection
            if let EvaluationResult::Collection(items) = context {
                EvaluationResult::Integer(items.len() as i64)
            } else {
                // Single items count as 1, empty counts as 0
                match context {
                    EvaluationResult::Empty => EvaluationResult::Integer(0),
                    _ => EvaluationResult::Integer(1),
                }
            }
        }
        "empty" => {
            // Returns true if the collection is empty
            match context {
                EvaluationResult::Empty => EvaluationResult::Boolean(true),
                EvaluationResult::Collection(items) => EvaluationResult::Boolean(items.is_empty()),
                _ => EvaluationResult::Boolean(false),
            }
        }
        "exists" => {
            // Returns true if the collection has any items
            match context {
                EvaluationResult::Empty => EvaluationResult::Boolean(false),
                EvaluationResult::Collection(items) => EvaluationResult::Boolean(!items.is_empty()),
                _ => EvaluationResult::Boolean(true),
            }
        }
        "first" => {
            // Returns the first item in the collection
            if let EvaluationResult::Collection(items) = context {
                items.first().cloned().unwrap_or(EvaluationResult::Empty)
            } else {
                // A single item is returned as is
                context.clone()
            }
        }
        "last" => {
            // Returns the last item in the collection
            if let EvaluationResult::Collection(items) = context {
                items.last().cloned().unwrap_or(EvaluationResult::Empty)
            } else {
                // A single item is returned as is
                context.clone()
            }
        }
        "not" => {
            // Logical negation
            EvaluationResult::Boolean(!context.to_boolean())
        }
        "contains" => {
            // Check if the context contains the argument
            if args.is_empty() {
                return EvaluationResult::Empty;
            }

            match context {
                EvaluationResult::String(s) => {
                    if let EvaluationResult::String(arg) = &args[0] {
                        EvaluationResult::Boolean(s.contains(arg))
                    } else {
                        EvaluationResult::Boolean(false)
                    }
                }
                EvaluationResult::Collection(items) => {
                    let contains = items
                        .iter()
                        .any(|item| compare_equality(item, "=", &args[0]).to_boolean());
                    EvaluationResult::Boolean(contains)
                }
                _ => EvaluationResult::Boolean(false),
            }
        }
        "length" => {
            // Returns the length of a string
            match context {
                EvaluationResult::String(s) => EvaluationResult::Integer(s.len() as i64),
                _ => EvaluationResult::Empty,
            }
        }
        "where" => {
            // Filter the collection based on a predicate
            // The predicate is the first argument, which should be a lambda
            if args.is_empty() {
                return EvaluationResult::Empty;
            }

            // For now, we'll just return the original collection
            // In a full implementation, we would evaluate the predicate for each item
            context.clone()
        }
        "select" => {
            // Project each item in the collection to a new value
            // The projection is the first argument, which should be a lambda
            if args.is_empty() {
                return EvaluationResult::Empty;
            }

            // For now, we'll just return the original collection
            // In a full implementation, we would apply the projection to each item
            context.clone()
        }
        // Add more functions as needed
        _ => EvaluationResult::Empty,
    }
}

/// Evaluates an indexer expression
fn evaluate_indexer(collection: &EvaluationResult, index: &EvaluationResult) -> EvaluationResult {
    // Get the index as an integer
    let idx = match index {
        EvaluationResult::Number(n) => *n as usize,
        EvaluationResult::Integer(i) => *i as usize,
        _ => return EvaluationResult::Empty,
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
                EvaluationResult::Number(n) => EvaluationResult::Number(-n),
                EvaluationResult::Integer(i) => EvaluationResult::Integer(-i),
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
    match (left, right) {
        (EvaluationResult::Number(l), EvaluationResult::Number(r)) => match op {
            "*" => EvaluationResult::Number(l * r),
            "/" => {
                if *r == 0.0 {
                    EvaluationResult::Empty
                } else {
                    EvaluationResult::Number(l / r)
                }
            }
            "div" => {
                if *r == 0.0 {
                    EvaluationResult::Empty
                } else {
                    EvaluationResult::Integer((l / r).floor() as i64)
                }
            }
            "mod" => {
                if *r == 0.0 {
                    EvaluationResult::Empty
                } else {
                    EvaluationResult::Number(l % r)
                }
            }
            _ => EvaluationResult::Empty,
        },
        (EvaluationResult::Integer(l), EvaluationResult::Integer(r)) => match op {
            "*" => EvaluationResult::Integer(l * r),
            "/" => {
                if *r == 0 {
                    EvaluationResult::Empty
                } else {
                    EvaluationResult::Number(*l as f64 / *r as f64)
                }
            }
            "div" => {
                if *r == 0 {
                    EvaluationResult::Empty
                } else {
                    EvaluationResult::Integer(l / r)
                }
            }
            "mod" => {
                if *r == 0 {
                    EvaluationResult::Empty
                } else {
                    EvaluationResult::Integer(l % r)
                }
            }
            _ => EvaluationResult::Empty,
        },
        _ => EvaluationResult::Empty,
    }
}

/// Applies an additive operator to two values
fn apply_additive(left: &EvaluationResult, op: &str, right: &EvaluationResult) -> EvaluationResult {
    match op {
        "+" => match (left, right) {
            (EvaluationResult::Number(l), EvaluationResult::Number(r)) => {
                EvaluationResult::Number(l + r)
            }
            (EvaluationResult::Integer(l), EvaluationResult::Integer(r)) => {
                EvaluationResult::Integer(l + r)
            }
            (EvaluationResult::String(l), EvaluationResult::String(r)) => {
                EvaluationResult::String(format!("{}{}", l, r))
            }
            _ => EvaluationResult::Empty,
        },
        "-" => match (left, right) {
            (EvaluationResult::Number(l), EvaluationResult::Number(r)) => {
                EvaluationResult::Number(l - r)
            }
            (EvaluationResult::Integer(l), EvaluationResult::Integer(r)) => {
                EvaluationResult::Integer(l - r)
            }
            _ => EvaluationResult::Empty,
        },
        "&" => {
            // String concatenation
            let left_str = left.to_string_value();
            let right_str = right.to_string_value();
            EvaluationResult::String(format!("{}{}", left_str, right_str))
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
                ("Decimal", EvaluationResult::Number(_)) => true,
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
                ("Boolean", EvaluationResult::Boolean(_)) => value.clone(),
                ("String", EvaluationResult::String(_)) => value.clone(),
                ("Integer", EvaluationResult::Integer(_)) => value.clone(),
                ("Decimal", EvaluationResult::Number(_)) => value.clone(),
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
    match (left, right) {
        (EvaluationResult::Number(l), EvaluationResult::Number(r)) => {
            let result = match op {
                "<" => l < r,
                "<=" => l <= r,
                ">" => l > r,
                ">=" => l >= r,
                _ => false,
            };
            EvaluationResult::Boolean(result)
        }
        (EvaluationResult::Integer(l), EvaluationResult::Integer(r)) => {
            let result = match op {
                "<" => l < r,
                "<=" => l <= r,
                ">" => l > r,
                ">=" => l >= r,
                _ => false,
            };
            EvaluationResult::Boolean(result)
        }
        (EvaluationResult::String(l), EvaluationResult::String(r)) => {
            let result = match op {
                "<" => l < r,
                "<=" => l <= r,
                ">" => l > r,
                ">=" => l >= r,
                _ => false,
            };
            EvaluationResult::Boolean(result)
        }
        (EvaluationResult::Date(l), EvaluationResult::Date(r)) => {
            let result = match op {
                "<" => l < r,
                "<=" => l <= r,
                ">" => l > r,
                ">=" => l >= r,
                _ => false,
            };
            EvaluationResult::Boolean(result)
        }
        (EvaluationResult::DateTime(l), EvaluationResult::DateTime(r)) => {
            let result = match op {
                "<" => l < r,
                "<=" => l <= r,
                ">" => l > r,
                ">=" => l >= r,
                _ => false,
            };
            EvaluationResult::Boolean(result)
        }
        (EvaluationResult::Time(l), EvaluationResult::Time(r)) => {
            let result = match op {
                "<" => l < r,
                "<=" => l <= r,
                ">" => l > r,
                ">=" => l >= r,
                _ => false,
            };
            EvaluationResult::Boolean(result)
        }
        _ => EvaluationResult::Empty,
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
                (EvaluationResult::Empty, EvaluationResult::Empty) => true,
                (EvaluationResult::Boolean(l), EvaluationResult::Boolean(r)) => l == r,
                (EvaluationResult::String(l), EvaluationResult::String(r)) => l == r,
                (EvaluationResult::Number(l), EvaluationResult::Number(r)) => l == r,
                (EvaluationResult::Integer(l), EvaluationResult::Integer(r)) => l == r,
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
                (EvaluationResult::Empty, EvaluationResult::Empty) => false,
                (EvaluationResult::Boolean(l), EvaluationResult::Boolean(r)) => l != r,
                (EvaluationResult::String(l), EvaluationResult::String(r)) => l != r,
                (EvaluationResult::Number(l), EvaluationResult::Number(r)) => l != r,
                (EvaluationResult::Integer(l), EvaluationResult::Integer(r)) => l != r,
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
