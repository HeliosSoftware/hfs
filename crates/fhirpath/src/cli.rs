//! # FHIRPath CLI Tool
//!
//! This module provides a command-line interface for evaluating FHIRPath expressions
//! against FHIR resources. It supports expression evaluation with context, variables,
//! and various debugging options.
//!
//! ## Overview
//!
//! The CLI tool allows users to:
//! - Evaluate FHIRPath expressions against FHIR resources
//! - Set context expressions for scoped evaluation
//! - Define variables for use in expressions
//! - Generate parse debug trees for expression analysis
//! - Output results in JSON format
//!
//! ## Command Line Options
//!
//! ```text
//! -e, --expression <EXPRESSION>      FHIRPath expression to evaluate
//! -c, --context <CONTEXT>           Context expression to evaluate first
//! -r, --resource <RESOURCE>         Path to FHIR resource JSON file
//! -v, --variables <VARIABLES>       Path to variables JSON file
//!     --var <KEY=VALUE>            Set a variable directly
//! -o, --output <OUTPUT>            Output file path (defaults to stdout)
//!     --parse-debug-tree           Output parse debug tree as JSON
//!     --parse-debug                Output parse debug info
//!     --trace                      Enable trace output
//!     --fhir-version <VERSION>     FHIR version [default: R4]
//!     --validate                   Validate expression before execution
//!     --terminology-server <URL>   Terminology server URL
//! -h, --help                       Print help
//! ```
//!
//! ## Usage Examples
//!
//! ### Basic expression evaluation
//! ```bash
//! fhirpath-cli -e "Patient.name.family" -r patient.json
//! ```
//!
//! ### Using context expression
//! ```bash
//! fhirpath-cli -c "Patient.name" -e "family" -r patient.json
//! ```
//!
//! ### With variables from file
//! ```bash
//! fhirpath-cli -e "value > %threshold" -r observation.json -v variables.json
//! ```
//!
//! ### With inline variables
//! ```bash
//! fhirpath-cli -e "value > %threshold" -r observation.json --var threshold=5.0
//! ```
//!
//! ### Parse debug tree output
//! ```bash
//! fhirpath-cli -e "Patient.name.given.first()" --parse-debug-tree
//! ```
//!
//! ### Output to file
//! ```bash
//! fhirpath-cli -e "Patient.name" -r patient.json -o result.json
//! ```
//!
//! ### Using stdin for resource
//! ```bash
//! cat patient.json | fhirpath-cli -e "Patient.name.family" -r -
//! ```

use std::fs;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::collections::HashMap;

use clap::Parser;
use serde_json::{json, Value};

use crate::error::{FhirPathError, FhirPathResult};
use crate::evaluator::EvaluationContext;
use crate::parse_debug::{expression_to_debug_tree, generate_parse_debug};
use crate::{evaluate_expression, EvaluationResult};
use helios_fhir::{FhirResource, FhirVersion};

#[derive(Parser, Debug)]
#[command(name = "fhirpath-cli")]
#[command(about = "FHIRPath CLI tool for evaluating expressions against FHIR resources")]
#[command(long_about = "Evaluate FHIRPath expressions against FHIR resources with support for context expressions, variables, and debug output")]
pub struct Args {
    /// FHIRPath expression to evaluate
    #[arg(short, long)]
    pub expression: String,

    /// Context expression to evaluate first (optional)
    #[arg(short, long)]
    pub context: Option<String>,

    /// Path to FHIR resource JSON file (use '-' for stdin)
    #[arg(short, long)]
    pub resource: PathBuf,

    /// Path to variables JSON file
    #[arg(short = 'v', long)]
    pub variables: Option<PathBuf>,

    /// Set a variable directly (format: key=value)
    #[arg(long = "var", value_parser = parse_var)]
    pub var: Vec<(String, String)>,

    /// Output file path (defaults to stdout)
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Output parse debug tree as JSON
    #[arg(long)]
    pub parse_debug_tree: bool,

    /// Output parse debug info
    #[arg(long)]
    pub parse_debug: bool,

    /// Enable trace output
    #[arg(long)]
    pub trace: bool,

    /// FHIR version to use for parsing resources
    #[arg(long, value_enum, default_value_t = FhirVersion::R4)]
    pub fhir_version: FhirVersion,

    /// Validate expression before execution
    #[arg(long)]
    pub validate: bool,

    /// Terminology server URL (for terminology operations)
    #[arg(long)]
    pub terminology_server: Option<String>,
}

/// Parse a key=value pair
fn parse_var(s: &str) -> Result<(String, String), String> {
    let pos = s
        .find('=')
        .ok_or_else(|| format!("invalid variable format: {}", s))?;
    Ok((s[..pos].to_string(), s[pos + 1..].to_string()))
}

/// Main CLI execution function
pub fn run_cli(args: Args) -> FhirPathResult<()> {
    // If only parse debug is requested, handle that first
    if args.parse_debug_tree || args.parse_debug {
        return handle_parse_debug(&args);
    }

    // Read the resource
    let resource_content = read_input(&args.resource)?;
    let resource_json: Value = serde_json::from_str(&resource_content)?;

    // Parse the resource based on FHIR version
    let fhir_resource = parse_fhir_resource(resource_json, args.fhir_version)?;

    // Create evaluation context
    let mut context = EvaluationContext::new(vec![fhir_resource]);

    // Load variables if provided
    if let Some(vars_path) = &args.variables {
        load_variables_from_file(&mut context, vars_path)?;
    }

    // Set inline variables
    for (key, value) in &args.var {
        set_variable(&mut context, key, value)?;
    }

    // Set terminology server if provided
    if let Some(terminology_server) = &args.terminology_server {
        // Note: This would need to be implemented in the evaluator
        // For now, we'll just note it in the context
        context.set_variable_result(
            "terminologyServer",
            EvaluationResult::string(terminology_server.clone()),
        );
    }

    // Enable trace if requested
    if args.trace {
        // Note: This would need to be implemented in the evaluator
        // For now, we'll set a flag in the context
        context.set_variable_result("_trace", EvaluationResult::boolean(true));
    }

    // Evaluate context expression if provided
    let result = if let Some(context_expr) = &args.context {
        // First evaluate the context expression
        let context_result = evaluate_expression(context_expr, &context)
            .map_err(|e| FhirPathError::EvaluationError(e))?;

        // Create a new context with the context result
        let mut scoped_context = EvaluationContext::new(vec![]);
        // Set the context result as the root
        let context_items = match context_result {
            EvaluationResult::Collection { items, .. } => items,
            single_value => vec![single_value],
        };
        
        for value in context_items {
            // Note: This is a simplified approach. In a full implementation,
            // we'd need to properly handle setting the context
            scoped_context.set_variable_result("this", value);
        }

        // Evaluate the main expression in the scoped context
        evaluate_expression(&args.expression, &scoped_context)
            .map_err(|e| FhirPathError::EvaluationError(e))?
    } else {
        // Evaluate the expression directly
        evaluate_expression(&args.expression, &context)
            .map_err(|e| FhirPathError::EvaluationError(e))?
    };

    // Convert result to JSON
    let output = result_to_json(&result)?;

    // Write output
    write_output(&args.output, &output)?;

    Ok(())
}

/// Handle parse debug output
fn handle_parse_debug(args: &Args) -> FhirPathResult<()> {
    use chumsky::Parser as ChumskyParser;

    // Parse the expression
    let parsed = crate::parser::parser()
        .parse(args.expression.as_str())
        .map_err(|e| FhirPathError::ParseError(format!("{:?}", e)))?;

    let output = if args.parse_debug_tree {
        // Generate JSON debug tree
        let debug_tree = expression_to_debug_tree(&parsed);
        serde_json::to_string_pretty(&debug_tree)?
    } else {
        // Generate text debug output
        generate_parse_debug(&parsed)
    };

    write_output(&args.output, &output)?;
    Ok(())
}

/// Read input from file or stdin
fn read_input(path: &PathBuf) -> FhirPathResult<String> {
    if path.to_str() == Some("-") {
        let mut buffer = String::new();
        io::stdin().read_to_string(&mut buffer)?;
        Ok(buffer)
    } else {
        Ok(fs::read_to_string(path)?)
    }
}

/// Write output to file or stdout
fn write_output(path: &Option<PathBuf>, content: &str) -> FhirPathResult<()> {
    match path {
        Some(p) => {
            fs::write(p, content)?;
        }
        None => {
            let stdout = io::stdout();
            let mut handle = stdout.lock();
            handle.write_all(content.as_bytes())?;
            handle.write_all(b"\n")?;
        }
    }
    Ok(())
}

/// Parse FHIR resource based on version
fn parse_fhir_resource(json: Value, version: FhirVersion) -> FhirPathResult<FhirResource> {
    match version {
        #[cfg(feature = "R4")]
        FhirVersion::R4 => {
            let resource: helios_fhir::r4::Resource = serde_json::from_value(json)?;
            Ok(FhirResource::R4(Box::new(resource)))
        }
        #[cfg(feature = "R4B")]
        FhirVersion::R4B => {
            let resource: helios_fhir::r4b::Resource = serde_json::from_value(json)?;
            Ok(FhirResource::R4B(Box::new(resource)))
        }
        #[cfg(feature = "R5")]
        FhirVersion::R5 => {
            let resource: helios_fhir::r5::Resource = serde_json::from_value(json)?;
            Ok(FhirResource::R5(Box::new(resource)))
        }
        #[cfg(feature = "R6")]
        FhirVersion::R6 => {
            let resource: helios_fhir::r6::Resource = serde_json::from_value(json)?;
            Ok(FhirResource::R6(Box::new(resource)))
        }
        #[cfg(not(any(feature = "R4", feature = "R4B", feature = "R5", feature = "R6")))]
        _ => Err(FhirPathError::InvalidInput(format!(
            "FHIR version {:?} is not enabled. Compile with the appropriate feature flag.",
            version
        ))),
    }
}

/// Load variables from JSON file
fn load_variables_from_file(
    context: &mut EvaluationContext,
    path: &PathBuf,
) -> FhirPathResult<()> {
    let content = fs::read_to_string(path)?;
    let variables: HashMap<String, Value> = serde_json::from_str(&content)?;

    for (key, value) in variables {
        set_variable_from_json(context, &key, &value)?;
    }

    Ok(())
}

/// Set a variable from string value
fn set_variable(context: &mut EvaluationContext, key: &str, value: &str) -> FhirPathResult<()> {
    // Try to parse as JSON first
    if let Ok(json_value) = serde_json::from_str::<Value>(value) {
        set_variable_from_json(context, key, &json_value)?;
    } else {
        // Treat as string
        context.set_variable_result(key, EvaluationResult::string(value.to_string()));
    }
    Ok(())
}

/// Set a variable from JSON value
fn set_variable_from_json(
    context: &mut EvaluationContext,
    key: &str,
    value: &Value,
) -> FhirPathResult<()> {
    let result = match value {
        Value::Null => EvaluationResult::Empty,
        Value::Bool(b) => EvaluationResult::boolean(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                EvaluationResult::integer(i)
            } else if let Some(f) = n.as_f64() {
                EvaluationResult::decimal(rust_decimal::Decimal::try_from(f).map_err(|e| {
                    FhirPathError::InvalidInput(format!("Invalid decimal value: {}", e))
                })?)
            } else {
                return Err(FhirPathError::InvalidInput(format!(
                    "Unsupported number type: {}",
                    n
                )));
            }
        }
        Value::String(s) => EvaluationResult::string(s.clone()),
        Value::Array(arr) => {
            let mut results = Vec::new();
            for item in arr {
                results.push(json_value_to_result(item)?);
            }
            EvaluationResult::collection(results)
        }
        Value::Object(_) => {
            // For complex objects, store as JSON string for now
            EvaluationResult::string(value.to_string())
        }
    };

    context.set_variable_result(key, result);
    Ok(())
}

/// Convert JSON value to EvaluationResult
fn json_value_to_result(value: &Value) -> FhirPathResult<EvaluationResult> {
    match value {
        Value::Null => Ok(EvaluationResult::Empty),
        Value::Bool(b) => Ok(EvaluationResult::boolean(*b)),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(EvaluationResult::integer(i))
            } else if let Some(f) = n.as_f64() {
                Ok(EvaluationResult::decimal(
                    rust_decimal::Decimal::try_from(f).map_err(|e| {
                        FhirPathError::InvalidInput(format!("Invalid decimal value: {}", e))
                    })?,
                ))
            } else {
                Err(FhirPathError::InvalidInput(format!(
                    "Unsupported number type: {}",
                    n
                )))
            }
        }
        Value::String(s) => Ok(EvaluationResult::string(s.clone())),
        Value::Array(_) | Value::Object(_) => {
            // For complex types, convert to JSON string
            Ok(EvaluationResult::string(value.to_string()))
        }
    }
}

/// Convert EvaluationResult to JSON
fn result_to_json(result: &EvaluationResult) -> FhirPathResult<String> {
    let output = match result {
        EvaluationResult::Collection { items, .. } => {
            let values: Vec<Value> = items.iter()
                .map(evaluation_result_to_json_value)
                .collect();
            
            if values.len() == 1 {
                values[0].clone()
            } else {
                json!(values)
            }
        },
        single_value => evaluation_result_to_json_value(single_value),
    };

    Ok(serde_json::to_string_pretty(&output)?)
}

/// Convert a single EvaluationResult to JSON Value
fn evaluation_result_to_json_value(result: &EvaluationResult) -> Value {
    match result {
        EvaluationResult::Empty => Value::Null,
        EvaluationResult::Boolean(b, _) => json!(b),
        EvaluationResult::String(s, _) => json!(s),
        EvaluationResult::Integer(i, _) => json!(i),
        EvaluationResult::Integer64(i, _) => json!(i),
        EvaluationResult::Decimal(d, _) => json!(d),
        EvaluationResult::Date(s, _) => json!(s),
        EvaluationResult::DateTime(s, _) => json!(s),
        EvaluationResult::Time(s, _) => json!(s),
        EvaluationResult::Quantity(value, unit, _) => json!({
            "value": value,
            "unit": unit
        }),
        EvaluationResult::Collection { items, .. } => {
            let values: Vec<Value> = items.iter()
                .map(evaluation_result_to_json_value)
                .collect();
            json!(values)
        },
        _ => {
            // For other complex types, use debug representation
            json!(format!("{:?}", result))
        }
    }
}