//! Parse debug tree generation for FHIRPath expressions
//!
//! This module provides functionality to convert FHIRPath AST (Abstract Syntax Tree)
//! into the JSON format expected by fhirpath-lab and other tools. The format includes
//! expression types, names, arguments, and optional return type information.

use crate::parser::{Expression, Invocation, Literal, Term, TypeSpecifier};
use crate::type_inference::{TypeContext, infer_expression_type};
use serde_json::{Value, json};

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
pub fn expression_to_debug_tree(expr: &Expression, context: &TypeContext) -> Value {
    expression_to_debug_tree_inner(expr, context)
}

fn expression_to_debug_tree_inner(expr: &Expression, context: &TypeContext) -> Value {
    // Get the inferred type for this expression
    let return_type = infer_expression_type(expr, context).map(|t| t.to_display_string());

    let mut node = match expr {
        Expression::Term(term) => term_to_debug_tree(term, context),

        Expression::Invocation(base_expr, invocation) => {
            // For invocations, we need to handle the structure differently
            // The invocation is the main node, and the base expression is its first argument
            let mut inv_node = invocation_to_debug_tree(invocation, context);

            // Get existing arguments or create empty array
            let mut args = inv_node
                .get("Arguments")
                .and_then(|a| a.as_array())
                .cloned()
                .unwrap_or_default();

            // Insert the base expression as the first argument (implicit "that")
            let base_node = expression_to_debug_tree_inner(base_expr, context);
            args.insert(0, base_node);

            inv_node["Arguments"] = json!(args);
            inv_node
        }

        Expression::Indexer(expr, index) => {
            json!({
                "ExpressionType": "IndexerExpression",
                "Name": "[]",
                "Arguments": vec![
                    expression_to_debug_tree_inner(expr, context),
                    expression_to_debug_tree_inner(index, context)
                ]
            })
        }

        Expression::Polarity(op, expr) => {
            json!({
                "ExpressionType": "UnaryExpression",
                "Name": op.to_string(),
                "Arguments": vec![expression_to_debug_tree_inner(expr, context)]
            })
        }

        Expression::Multiplicative(left, op, right)
        | Expression::Additive(left, op, right)
        | Expression::Inequality(left, op, right)
        | Expression::Equality(left, op, right)
        | Expression::Membership(left, op, right) => {
            json!({
                "ExpressionType": "BinaryExpression",
                "Name": op,
                "Arguments": vec![
                    expression_to_debug_tree_inner(left, context),
                    expression_to_debug_tree_inner(right, context)
                ]
            })
        }

        Expression::Type(expr, op, type_spec) => {
            json!({
                "ExpressionType": "TypeExpression",
                "Name": op,
                "Arguments": vec![
                    expression_to_debug_tree_inner(expr, context),
                    type_specifier_to_debug_tree(type_spec)
                ]
            })
        }

        Expression::Union(left, right) => {
            json!({
                "ExpressionType": "BinaryExpression",
                "Name": "|",
                "Arguments": vec![
                    expression_to_debug_tree_inner(left, context),
                    expression_to_debug_tree_inner(right, context)
                ]
            })
        }

        Expression::And(left, right) => {
            json!({
                "ExpressionType": "BinaryExpression",
                "Name": "and",
                "Arguments": vec![
                    expression_to_debug_tree_inner(left, context),
                    expression_to_debug_tree_inner(right, context)
                ]
            })
        }

        Expression::Or(left, op, right) => {
            json!({
                "ExpressionType": "BinaryExpression",
                "Name": op,
                "Arguments": vec![
                    expression_to_debug_tree_inner(left, context),
                    expression_to_debug_tree_inner(right, context)
                ]
            })
        }

        Expression::Implies(left, right) => {
            json!({
                "ExpressionType": "BinaryExpression",
                "Name": "implies",
                "Arguments": vec![
                    expression_to_debug_tree_inner(left, context),
                    expression_to_debug_tree_inner(right, context)
                ]
            })
        }

        Expression::Lambda(param, expr) => {
            let mut node = json!({
                "ExpressionType": "LambdaExpression",
                "Name": "=>",
                "Arguments": vec![expression_to_debug_tree_inner(expr, context)]
            });
            if let Some(param_name) = param {
                node["Parameter"] = json!(param_name);
            }
            node
        }
    };

    // Add return type if available
    if let Some(rt) = return_type {
        node["ReturnType"] = json!(rt);
    }

    node
}

fn term_to_debug_tree(term: &Term, context: &TypeContext) -> Value {
    match term {
        Term::Literal(lit) => literal_to_debug_tree(lit),

        Term::Invocation(invocation) => {
            // For a standalone invocation (e.g., at the start of an expression),
            // we need to add an implicit "builtin.that" as the context
            let mut inv_node = invocation_to_debug_tree(invocation, context);

            // Add implicit "that" context as first argument for member access
            if matches!(invocation, Invocation::Member(_)) {
                let that_node = json!({
                    "ExpressionType": "AxisExpression",
                    "Name": "builtin.that",
                    "ReturnType": context.current_type.as_ref()
                        .map(|t| t.to_display_string())
                        .unwrap_or_else(|| "Any".to_string())
                });

                let mut args = vec![that_node];
                if let Some(existing_args) = inv_node.get("Arguments").and_then(|a| a.as_array()) {
                    args.extend(existing_args.clone());
                }
                inv_node["Arguments"] = json!(args);
            }

            inv_node
        }

        Term::ExternalConstant(name) => {
            let mut node = json!({
                "ExpressionType": "VariableRefExpression",
                "Name": name
            });

            // Add type if variable is known
            if let Some(var_type) = context.variables.get(name) {
                node["ReturnType"] = json!(var_type.to_display_string());
            }

            node
        }

        Term::Parenthesized(expr) => expression_to_debug_tree_inner(expr, context),
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
                "ReturnType": "system.Boolean"
            })
        }

        Literal::String(s) => {
            json!({
                "ExpressionType": "ConstantExpression",
                "Name": s,
                "ReturnType": "system.String"
            })
        }

        Literal::Number(n) => {
            json!({
                "ExpressionType": "ConstantExpression",
                "Name": n.to_string(),
                "ReturnType": "system.Decimal"
            })
        }

        Literal::Integer(i) => {
            json!({
                "ExpressionType": "ConstantExpression",
                "Name": i.to_string(),
                "ReturnType": "system.Integer"
            })
        }

        Literal::Date(d) => {
            json!({
                "ExpressionType": "ConstantExpression",
                "Name": format!("@{}", d),
                "ReturnType": "system.Date"
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
                "ReturnType": "system.DateTime"
            })
        }

        Literal::Time(t) => {
            json!({
                "ExpressionType": "ConstantExpression",
                "Name": format!("@T{}", t),
                "ReturnType": "system.Time"
            })
        }

        Literal::Quantity(value, unit) => {
            json!({
                "ExpressionType": "ConstantExpression",
                "Name": format!("{} '{}'", value, unit),
                "ReturnType": "system.Quantity"
            })
        }
    }
}

fn invocation_to_debug_tree(invocation: &Invocation, context: &TypeContext) -> Value {
    match invocation {
        Invocation::Function(name, args) => {
            let mut node = json!({
                "ExpressionType": "FunctionCallExpression",
                "Name": name
            });

            if !args.is_empty() {
                node["Arguments"] = json!(
                    args.iter()
                        .map(|arg| expression_to_debug_tree_inner(arg, context))
                        .collect::<Vec<_>>()
                );
            } else {
                node["Arguments"] = json!([]);
            }

            node
        }

        Invocation::Member(name) => {
            json!({
                "ExpressionType": "ChildExpression",
                "Name": name,
                "Arguments": []
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
        Expression::Term(term) => match term {
            Term::Literal(lit) => output.push_str(&format!("{}{:?}\n", indent_str, lit)),
            Term::Invocation(inv) => output.push_str(&format!("{}{:?}\n", indent_str, inv)),
            Term::ExternalConstant(name) => output.push_str(&format!("{}%{}\n", indent_str, name)),
            Term::Parenthesized(expr) => {
                output.push_str(&format!("{}(\n", indent_str));
                generate_parse_debug_inner(expr, output, indent + 1);
                output.push_str(&format!("{})\n", indent_str));
            }
        },

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

        Expression::Multiplicative(left, op, right)
        | Expression::Additive(left, op, right)
        | Expression::Inequality(left, op, right)
        | Expression::Equality(left, op, right)
        | Expression::Membership(left, op, right) => {
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

        Expression::Or(left, op, right) => {
            generate_parse_debug_inner(left, output, indent);
            output.push_str(&format!("{}{}\n", indent_str, op));
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
