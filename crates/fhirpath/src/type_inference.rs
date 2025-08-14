//! Type inference for FHIRPath expressions
//!
//! This module provides static type inference for FHIRPath expressions,
//! determining the return type of expressions without evaluating them.

use crate::parser::{Expression, Term, Invocation, Literal, TypeSpecifier};
use std::collections::HashMap;

/// Represents a type in the FHIRPath type system
#[derive(Debug, Clone, PartialEq)]
pub struct InferredType {
    pub namespace: String,
    pub name: String,
    pub is_collection: bool,
}

impl InferredType {
    pub fn new(namespace: &str, name: &str) -> Self {
        Self {
            namespace: namespace.to_string(),
            name: name.to_string(),
            is_collection: false,
        }
    }

    pub fn collection(mut self) -> Self {
        self.is_collection = true;
        self
    }

    pub fn system(name: &str) -> Self {
        Self::new("system", name)
    }

    pub fn fhir(name: &str) -> Self {
        Self::new("FHIR", name)
    }

    /// Convert to display string for parseDebugTree
    pub fn to_display_string(&self) -> String {
        let base = if self.namespace == "system" {
            format!("system.{}", capitalize_first(&self.name))
        } else if self.namespace == "FHIR" {
            self.name.clone()
        } else {
            format!("{}.{}", self.namespace, self.name)
        };

        if self.is_collection {
            format!("{}[]", base)
        } else {
            base
        }
    }
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

/// Context for type inference
#[derive(Default)]
pub struct TypeContext {
    /// The type of the root resource
    pub root_type: Option<InferredType>,
    /// The current context type (changes during traversal)
    pub current_type: Option<InferredType>,
    /// Variables and their types
    pub variables: HashMap<String, InferredType>,
}

impl TypeContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_root_type(mut self, root_type: InferredType) -> Self {
        self.current_type = Some(root_type.clone());
        self.root_type = Some(root_type);
        self
    }
}

/// Infer the return type of a FHIRPath expression
pub fn infer_expression_type(expr: &Expression, context: &TypeContext) -> Option<InferredType> {
    match expr {
        Expression::Term(term) => infer_term_type(term, context),
        
        Expression::Invocation(base_expr, invocation) => {
            let base_type = infer_expression_type(base_expr, context)?;
            infer_invocation_type(invocation, &base_type, context)
        }
        
        Expression::Indexer(expr, _index) => {
            let base_type = infer_expression_type(expr, context)?;
            // Indexing a collection returns the element type
            if base_type.is_collection {
                Some(InferredType {
                    namespace: base_type.namespace,
                    name: base_type.name,
                    is_collection: false,
                })
            } else {
                None // Can't index non-collections
            }
        }
        
        Expression::Polarity(op, expr) => {
            let _inner_type = infer_expression_type(expr, context)?;
            match op {
                '+' | '-' => Some(InferredType::system("Integer")),
                _ => None,
            }
        }
        
        Expression::Multiplicative(left, op, right) |
        Expression::Additive(left, op, right) => {
            let left_type = infer_expression_type(left, context)?;
            let _right_type = infer_expression_type(right, context)?;
            
            match op.as_str() {
                "*" | "/" | "mod" | "div" => Some(left_type), // Preserve numeric type
                "+" | "-" => Some(left_type), // Preserve numeric type
                _ => None,
            }
        }
        
        Expression::Inequality(_left, _op, _right) |
        Expression::Equality(_left, _op, _right) => {
            Some(InferredType::system("Boolean"))
        }
        
        Expression::Membership(expr, op, _type_or_expr) => {
            match op.as_str() {
                "in" | "contains" => Some(InferredType::system("Boolean")),
                _ => infer_expression_type(expr, context),
            }
        }
        
        Expression::Type(expr, op, type_spec) => {
            match op.as_str() {
                "is" => Some(InferredType::system("Boolean")),
                "as" => {
                    // 'as' returns the cast type
                    let type_name = match type_spec {
                        TypeSpecifier::QualifiedIdentifier(namespace_or_type, type_opt) => {
                            match type_opt {
                                Some(t) => t.clone(),
                                None => namespace_or_type.clone(),
                            }
                        }
                    };
                    Some(InferredType::fhir(&type_name))
                }
                _ => infer_expression_type(expr, context),
            }
        }
        
        Expression::Union(left, right) => {
            let left_type = infer_expression_type(left, context);
            let right_type = infer_expression_type(right, context);
            
            // Union combines collections
            match (left_type, right_type) {
                (Some(lt), Some(rt)) if lt == rt => Some(lt.collection()),
                (Some(lt), None) => Some(lt.collection()),
                (None, Some(rt)) => Some(rt.collection()),
                _ => None,
            }
        }
        
        Expression::And(_left, _right) |
        Expression::Implies(_left, _right) => {
            Some(InferredType::system("Boolean"))
        }
        
        Expression::Or(_left, _op, _right) => {
            Some(InferredType::system("Boolean"))
        }
        
        Expression::Lambda(_param, expr) => {
            // Lambda returns the type of its body expression
            infer_expression_type(expr, context)
        }
    }
}

fn infer_term_type(term: &Term, context: &TypeContext) -> Option<InferredType> {
    match term {
        Term::Literal(lit) => infer_literal_type(lit),
        Term::Invocation(inv) => infer_invocation_type(inv, &context.current_type.clone()?, context),
        Term::ExternalConstant(name) => {
            // Look up variable type
            context.variables.get(name).cloned()
        }
        Term::Parenthesized(expr) => infer_expression_type(expr, context),
    }
}

fn infer_literal_type(literal: &Literal) -> Option<InferredType> {
    match literal {
        Literal::Null => None, // null has no type
        Literal::Boolean(_) => Some(InferredType::system("Boolean")),
        Literal::String(_) => Some(InferredType::system("String")),
        Literal::Integer(_) => Some(InferredType::system("Integer")),
        Literal::Number(_) => Some(InferredType::system("Decimal")),
        Literal::Date(_) => Some(InferredType::system("Date")),
        Literal::DateTime(_, _) => Some(InferredType::system("DateTime")),
        Literal::Time(_) => Some(InferredType::system("Time")),
        Literal::Quantity(_, _) => Some(InferredType::system("Quantity")),
    }
}

fn infer_invocation_type(
    invocation: &Invocation,
    input_type: &InferredType,
    _context: &TypeContext,
) -> Option<InferredType> {
    match invocation {
        Invocation::Function(name, args) => {
            infer_function_return_type(name, input_type, args.len())
        }
        Invocation::Member(name) => {
            // Member access depends on the input type
            infer_member_type(name, input_type)
        }
        Invocation::This => Some(input_type.clone()),
        Invocation::Index => Some(InferredType::system("Integer")),
        Invocation::Total => Some(InferredType::system("Integer")),
    }
}

fn infer_member_type(member_name: &str, input_type: &InferredType) -> Option<InferredType> {
    // This is a simplified version - in reality, we'd need the full FHIR schema
    match input_type.name.as_str() {
        "Patient" => match member_name {
            "name" => Some(InferredType::fhir("HumanName").collection()),
            "birthDate" => Some(InferredType::system("Date")),
            "gender" => Some(InferredType::system("code")),
            "identifier" => Some(InferredType::fhir("Identifier").collection()),
            _ => Some(InferredType::system("String")), // Default fallback
        },
        "HumanName" => match member_name {
            "family" => Some(InferredType::system("String")),
            "given" => Some(InferredType::system("String").collection()),
            "text" => Some(InferredType::system("String")),
            "use" => Some(InferredType::system("code")),
            _ => Some(InferredType::system("String")),
        },
        _ => {
            // Generic fallback - could be improved with full schema
            Some(InferredType::system("String"))
        }
    }
}

fn infer_function_return_type(
    function_name: &str,
    input_type: &InferredType,
    _arg_count: usize,
) -> Option<InferredType> {
    // Map of FHIRPath functions to their return types
    match function_name {
        // String functions
        "toString" => Some(InferredType::system("String")),
        "toChars" => Some(InferredType::system("String").collection()),
        "substring" => Some(InferredType::system("String")),
        "startsWith" | "endsWith" | "contains" | "matches" => Some(InferredType::system("Boolean")),
        "replace" | "replaceMatches" | "trim" | "upper" | "lower" => Some(InferredType::system("String")),
        "split" => Some(InferredType::system("String").collection()),
        "join" => Some(InferredType::system("String")),
        "encode" | "decode" => Some(InferredType::system("String")),
        
        // Numeric functions
        "toInteger" => Some(InferredType::system("Integer")),
        "toDecimal" => Some(InferredType::system("Decimal")),
        "toQuantity" => Some(InferredType::system("Quantity")),
        "abs" | "ceiling" | "floor" | "round" | "truncate" => Some(input_type.clone()),
        "sqrt" | "exp" | "ln" | "log" | "power" => Some(InferredType::system("Decimal")),
        
        // Date/Time functions
        "toDate" => Some(InferredType::system("Date")),
        "toDateTime" => Some(InferredType::system("DateTime")),
        "toTime" => Some(InferredType::system("Time")),
        "today" => Some(InferredType::system("Date")),
        "now" => Some(InferredType::system("DateTime")),
        "timeOfDay" => Some(InferredType::system("Time")),
        
        // Boolean functions
        "toBoolean" => Some(InferredType::system("Boolean")),
        "not" => Some(InferredType::system("Boolean")),
        "allTrue" | "anyTrue" | "allFalse" | "anyFalse" => Some(InferredType::system("Boolean")),
        
        // Collection functions
        "count" => Some(InferredType::system("Integer")),
        "empty" | "exists" | "all" | "subsetOf" | "supersetOf" => Some(InferredType::system("Boolean")),
        "first" | "last" | "single" => {
            if input_type.is_collection {
                Some(InferredType {
                    namespace: input_type.namespace.clone(),
                    name: input_type.name.clone(),
                    is_collection: false,
                })
            } else {
                Some(input_type.clone())
            }
        }
        "tail" | "skip" | "take" | "distinct" => Some(input_type.clone()),
        "where" | "select" => Some(input_type.clone()),
        "repeat" => Some(input_type.clone()),
        "aggregate" => Some(InferredType::system("Any")), // Type depends on aggregator
        
        // Type functions
        "ofType" => Some(input_type.clone()),
        "is" => Some(InferredType::system("Boolean")),
        "as" => Some(input_type.clone()),
        
        // Utility functions
        "trace" => Some(input_type.clone()),
        "combine" => {
            // combine returns a collection of the same type
            if input_type.is_collection {
                Some(input_type.clone())
            } else {
                Some(input_type.clone().collection())
            }
        }
        
        // Math aggregates
        "sum" => Some(input_type.clone()),
        "min" | "max" | "avg" | "mean" => Some(input_type.clone()),
        
        _ => None, // Unknown function
    }
}