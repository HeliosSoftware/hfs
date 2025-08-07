//! Parse debug tree generation for FHIRPath expressions
//!
//! This module provides functionality to convert FHIRPath AST (Abstract Syntax Tree)
//! into the JSON format expected by fhirpath-lab and other tools. The format includes
//! expression types, names, arguments, and optional return type information.

use crate::parser::{Expression, Term, Invocation, Literal, TypeSpecifier};
use serde_json::{json, Value};

/// Convert a FHIRPath expression AST to a JSON debug tree
///
/// The output format matches the structure expected by fhirpath-lab:
/// ```json
/// {
///   "ExpressionType": "BinaryExpression",
///   "Name": "|",
///   "Arguments": [...],
///   "ReturnType": "string[]"
/// }
/// ```
pub fn expression_to_debug_tree(expr: &Expression) -> Value {
    match expr {
        Expression::Term(term) => term_to_debug_tree(term),
        
        Expression::Invocation(expr, invocation) => {
            json!({
                "ExpressionType": "InvocationExpression",
                "Name": invocation_name(invocation),
                "Arguments": vec![
                    expression_to_debug_tree(expr),
                    invocation_to_debug_tree(invocation)
                ]
            })
        }
        
        Expression::Indexer(expr, index) => {
            json!({
                "ExpressionType": "IndexerExpression",
                "Name": "[]",
                "Arguments": vec![
                    expression_to_debug_tree(expr),
                    expression_to_debug_tree(index)
                ]
            })
        }
        
        Expression::Polarity(op, expr) => {
            json!({
                "ExpressionType": "UnaryExpression",
                "Name": op.to_string(),
                "Arguments": vec![expression_to_debug_tree(expr)]
            })
        }
        
        Expression::Multiplicative(left, op, right) |
        Expression::Additive(left, op, right) |
        Expression::Inequality(left, op, right) |
        Expression::Equality(left, op, right) |
        Expression::Membership(left, op, right) |
        Expression::Or(left, op, right) => {
            json!({
                "ExpressionType": "BinaryExpression",
                "Name": op,
                "Arguments": vec![
                    expression_to_debug_tree(left),
                    expression_to_debug_tree(right)
                ]
            })
        }
        
        Expression::Type(expr, op, type_spec) => {
            json!({
                "ExpressionType": "TypeExpression",
                "Name": op,
                "Arguments": vec![
                    expression_to_debug_tree(expr),
                    type_specifier_to_debug_tree(type_spec)
                ]
            })
        }
        
        Expression::Union(left, right) => {
            json!({
                "ExpressionType": "BinaryExpression",
                "Name": "|",
                "Arguments": vec![
                    expression_to_debug_tree(left),
                    expression_to_debug_tree(right)
                ]
            })
        }
        
        Expression::And(left, right) => {
            json!({
                "ExpressionType": "BinaryExpression",
                "Name": "and",
                "Arguments": vec![
                    expression_to_debug_tree(left),
                    expression_to_debug_tree(right)
                ]
            })
        }
        
        Expression::Implies(left, right) => {
            json!({
                "ExpressionType": "BinaryExpression",
                "Name": "implies",
                "Arguments": vec![
                    expression_to_debug_tree(left),
                    expression_to_debug_tree(right)
                ]
            })
        }
        
        Expression::Lambda(param, expr) => {
            let mut node = json!({
                "ExpressionType": "LambdaExpression",
                "Name": "=>",
                "Arguments": vec![expression_to_debug_tree(expr)]
            });
            if let Some(param_name) = param {
                node["Parameter"] = json!(param_name);
            }
            node
        }
    }
}

fn term_to_debug_tree(term: &Term) -> Value {
    match term {
        Term::Literal(lit) => literal_to_debug_tree(lit),
        
        Term::Invocation(invocation) => invocation_to_debug_tree(invocation),
        
        Term::ExternalConstant(name) => {
            json!({
                "ExpressionType": "VariableRefExpression",
                "Name": name
            })
        }
        
        Term::Parenthesized(expr) => expression_to_debug_tree(expr),
    }
}

fn literal_to_debug_tree(literal: &Literal) -> Value {
    match literal {
        Literal::Null => {
            json!({
                "ExpressionType": "ConstantExpression",
                "Name": "{}",
                "ReturnType": "null"
            })
        }
        
        Literal::Boolean(b) => {
            json!({
                "ExpressionType": "ConstantExpression",
                "Name": b.to_string(),
                "ReturnType": "boolean"
            })
        }
        
        Literal::String(s) => {
            json!({
                "ExpressionType": "ConstantExpression",
                "Name": s,
                "ReturnType": "string"
            })
        }
        
        Literal::Number(n) => {
            json!({
                "ExpressionType": "ConstantExpression",
                "Name": n.to_string(),
                "ReturnType": "decimal"
            })
        }
        
        Literal::Integer(i) => {
            json!({
                "ExpressionType": "ConstantExpression",
                "Name": i.to_string(),
                "ReturnType": "integer"
            })
        }
        
        Literal::Date(d) => {
            json!({
                "ExpressionType": "ConstantExpression",
                "Name": format!("@{}", d),
                "ReturnType": "date"
            })
        }
        
        Literal::DateTime(date, time_opt) => {
            let value = match time_opt {
                Some((time, tz_opt)) => match tz_opt {
                    Some(tz) => format!("@{}T{}{}", date, time, tz),
                    None => format!("@{}T{}", date, time),
                },
                None => format!("@{}", date),
            };
            json!({
                "ExpressionType": "ConstantExpression",
                "Name": value,
                "ReturnType": "dateTime"
            })
        }
        
        Literal::Time(t) => {
            json!({
                "ExpressionType": "ConstantExpression",
                "Name": format!("@T{}", t),
                "ReturnType": "time"
            })
        }
        
        Literal::Quantity(value, unit) => {
            json!({
                "ExpressionType": "ConstantExpression",
                "Name": format!("{} '{}'", value, unit),
                "ReturnType": "quantity"
            })
        }
    }
}

fn invocation_to_debug_tree(invocation: &Invocation) -> Value {
    match invocation {
        Invocation::Function(name, args) => {
            let mut node = json!({
                "ExpressionType": "FunctionCallExpression",
                "Name": name
            });
            
            if !args.is_empty() {
                node["Arguments"] = json!(args.iter().map(expression_to_debug_tree).collect::<Vec<_>>());
            }
            
            node
        }
        
        Invocation::Member(name) => {
            json!({
                "ExpressionType": "ChildExpression",
                "Name": name
            })
        }
        
        Invocation::This => {
            json!({
                "ExpressionType": "AxisExpression",
                "Name": "builtin.this"
            })
        }
        
        Invocation::Index => {
            json!({
                "ExpressionType": "AxisExpression",
                "Name": "builtin.index"
            })
        }
        
        Invocation::Total => {
            json!({
                "ExpressionType": "AxisExpression",
                "Name": "builtin.total"
            })
        }
    }
}

fn invocation_name(invocation: &Invocation) -> &str {
    match invocation {
        Invocation::Function(name, _) => name,
        Invocation::Member(name) => name,
        Invocation::This => "$this",
        Invocation::Index => "$index",
        Invocation::Total => "$total",
    }
}

fn type_specifier_to_debug_tree(type_spec: &TypeSpecifier) -> Value {
    match type_spec {
        TypeSpecifier::QualifiedIdentifier(namespace_or_type, type_opt) => {
            let type_name = match type_opt {
                Some(t) => format!("{}.{}", namespace_or_type, t),
                None => namespace_or_type.clone(),
            };
            json!({
                "ExpressionType": "TypeSpecifier",
                "Name": type_name
            })
        }
    }
}

/// Generate parse debug output (textual format) for a FHIRPath expression
///
/// This generates a simple text representation of the parse tree with type annotations
pub fn generate_parse_debug(expr: &Expression) -> String {
    let mut output = String::new();
    generate_parse_debug_inner(expr, &mut output, 0);
    output
}

fn generate_parse_debug_inner(expr: &Expression, output: &mut String, indent: usize) {
    let indent_str = "  ".repeat(indent);
    
    match expr {
        Expression::Term(term) => {
            match term {
                Term::Literal(lit) => output.push_str(&format!("{}{:?}\n", indent_str, lit)),
                Term::Invocation(inv) => output.push_str(&format!("{}{:?}\n", indent_str, inv)),
                Term::ExternalConstant(name) => output.push_str(&format!("{}%{}\n", indent_str, name)),
                Term::Parenthesized(expr) => {
                    output.push_str(&format!("{}(\n", indent_str));
                    generate_parse_debug_inner(expr, output, indent + 1);
                    output.push_str(&format!("{})\n", indent_str));
                }
            }
        }
        
        Expression::Invocation(expr, inv) => {
            generate_parse_debug_inner(expr, output, indent);
            output.push_str(&format!("{}.{:?}\n", indent_str, inv));
        }
        
        Expression::Indexer(expr, index) => {
            generate_parse_debug_inner(expr, output, indent);
            output.push_str(&format!("{}[\n", indent_str));
            generate_parse_debug_inner(index, output, indent + 1);
            output.push_str(&format!("{}]\n", indent_str));
        }
        
        Expression::Polarity(op, expr) => {
            output.push_str(&format!("{}{}\n", indent_str, op));
            generate_parse_debug_inner(expr, output, indent + 1);
        }
        
        Expression::Multiplicative(left, op, right) |
        Expression::Additive(left, op, right) |
        Expression::Inequality(left, op, right) |
        Expression::Equality(left, op, right) |
        Expression::Membership(left, op, right) |
        Expression::Or(left, op, right) => {
            generate_parse_debug_inner(left, output, indent);
            output.push_str(&format!("{}{}\n", indent_str, op));
            generate_parse_debug_inner(right, output, indent + 1);
        }
        
        Expression::Type(expr, op, type_spec) => {
            generate_parse_debug_inner(expr, output, indent);
            output.push_str(&format!("{}{} {:?}\n", indent_str, op, type_spec));
        }
        
        Expression::Union(left, right) => {
            generate_parse_debug_inner(left, output, indent);
            output.push_str(&format!("{}|\n", indent_str));
            generate_parse_debug_inner(right, output, indent + 1);
        }
        
        Expression::And(left, right) => {
            generate_parse_debug_inner(left, output, indent);
            output.push_str(&format!("{}and\n", indent_str));
            generate_parse_debug_inner(right, output, indent + 1);
        }
        
        Expression::Implies(left, right) => {
            generate_parse_debug_inner(left, output, indent);
            output.push_str(&format!("{}implies\n", indent_str));
            generate_parse_debug_inner(right, output, indent + 1);
        }
        
        Expression::Lambda(param, expr) => {
            if let Some(p) = param {
                output.push_str(&format!("{}{} =>\n", indent_str, p));
            } else {
                output.push_str(&format!("{}=>\n", indent_str));
            }
            generate_parse_debug_inner(expr, output, indent + 1);
        }
    }
}