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

use std::collections::HashMap;
use std::fs;
use std::io::{self, Read, Write};
use std::path::PathBuf;

use clap::Parser;
use serde_json::{Value, json};

use crate::error::{FhirPathError, FhirPathResult};
use crate::evaluator::EvaluationContext;
use crate::parse_debug::{expression_to_debug_tree, generate_parse_debug};
use crate::{EvaluationResult, evaluate_expression};
use helios_fhir::{FhirResource, FhirVersion};

#[derive(Parser, Debug)]
#[command(name = "fhirpath-cli")]
#[command(about = "FHIRPath CLI tool for evaluating expressions against FHIR resources")]
#[command(
    long_about = "Evaluate FHIRPath expressions against FHIR resources with support for context expressions, variables, and debug output"
)]
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
        context.set_terminology_server(terminology_server.clone());
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
        let context_result =
            evaluate_expression(context_expr, &context).map_err(FhirPathError::EvaluationError)?;

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
            .map_err(FhirPathError::EvaluationError)?
    } else {
        // Evaluate the expression directly
        evaluate_expression(&args.expression, &context).map_err(FhirPathError::EvaluationError)?
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
        .into_result()
        .map_err(|e| FhirPathError::ParseError(format!("{:?}", e)))?;

    let output = if args.parse_debug_tree {
        // Generate JSON debug tree
        // Create a default type context for CLI usage
        let type_context = crate::type_inference::TypeContext::new();
        let debug_tree = expression_to_debug_tree(&parsed, &type_context);
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
fn load_variables_from_file(context: &mut EvaluationContext, path: &PathBuf) -> FhirPathResult<()> {
    let content = fs::read_to_string(path)?;
    let variables: HashMap<String, Value> = serde_json::from_str(&content)?;

    for (key, value) in variables {
        // Add % prefix if not already present
        let var_name = if key.starts_with('%') {
            key
        } else {
            format!("%{}", key)
        };
        set_variable_from_json(context, &var_name, &value)?;
    }

    Ok(())
}

/// Set a variable from string value
fn set_variable(context: &mut EvaluationContext, key: &str, value: &str) -> FhirPathResult<()> {
    // Add % prefix if not already present
    let var_name = if key.starts_with('%') {
        key.to_string()
    } else {
        format!("%{}", key)
    };

    // Try to parse as JSON first
    if let Ok(json_value) = serde_json::from_str::<Value>(value) {
        set_variable_from_json(context, &var_name, &json_value)?;
    } else {
        // Treat as string
        context.set_variable_result(&var_name, EvaluationResult::string(value.to_string()));
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
            let values: Vec<Value> = items.iter().map(evaluation_result_to_json_value).collect();

            if values.len() == 1 {
                values[0].clone()
            } else {
                json!(values)
            }
        }
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
        EvaluationResult::Quantity(value, unit, _) => {
            crate::json_utils::quantity_to_json(value, unit)
        }
        EvaluationResult::Collection { items, .. } => {
            let values: Vec<Value> = items.iter().map(evaluation_result_to_json_value).collect();
            json!(values)
        }
        _ => {
            // For other complex types, use debug representation
            json!(format!("{:?}", result))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_resource() -> Value {
        json!({
            "resourceType": "Patient",
            "id": "example",
            "name": [{
                "family": "Doe",
                "given": ["John", "James"]
            }],
            "birthDate": "1990-01-01",
            "active": true
        })
    }

    fn create_test_args(expression: &str, resource_path: PathBuf) -> Args {
        Args {
            expression: expression.to_string(),
            context: None,
            resource: resource_path,
            variables: None,
            var: vec![],
            output: None,
            parse_debug_tree: false,
            parse_debug: false,
            trace: false,
            fhir_version: FhirVersion::R4,
            validate: false,
            terminology_server: None,
        }
    }

    #[test]
    fn test_parse_var() {
        assert_eq!(
            parse_var("key=value").unwrap(),
            ("key".to_string(), "value".to_string())
        );
        assert_eq!(
            parse_var("complex=value=with=equals").unwrap(),
            ("complex".to_string(), "value=with=equals".to_string())
        );
        assert!(parse_var("invalid").is_err());
    }

    #[test]
    fn test_basic_expression_evaluation() {
        let temp_dir = TempDir::new().unwrap();
        let resource_path = temp_dir.path().join("patient.json");
        fs::write(&resource_path, create_test_resource().to_string()).unwrap();

        let args = create_test_args("Patient.name.family", resource_path);
        let result = run_cli(args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_context_expression() {
        let temp_dir = TempDir::new().unwrap();
        let resource_path = temp_dir.path().join("patient.json");
        fs::write(&resource_path, create_test_resource().to_string()).unwrap();

        let mut args = create_test_args("family", resource_path);
        args.context = Some("Patient.name".to_string());

        let result = run_cli(args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_variables_from_file() {
        let temp_dir = TempDir::new().unwrap();
        let resource_path = temp_dir.path().join("patient.json");
        let vars_path = temp_dir.path().join("vars.json");

        fs::write(&resource_path, create_test_resource().to_string()).unwrap();
        fs::write(
            &vars_path,
            json!({
                "threshold": 5,
                "testString": "hello"
            })
            .to_string(),
        )
        .unwrap();

        let mut args = create_test_args("%testString", resource_path);
        args.variables = Some(vars_path);

        let result = run_cli(args);
        if let Err(e) = &result {
            eprintln!("test_variables_from_file error: {:?}", e);
        }
        assert!(result.is_ok());
    }

    #[test]
    fn test_inline_variables() {
        let temp_dir = TempDir::new().unwrap();
        let resource_path = temp_dir.path().join("patient.json");
        fs::write(&resource_path, create_test_resource().to_string()).unwrap();

        let mut args = create_test_args("%myVar", resource_path);
        args.var = vec![("myVar".to_string(), "test-value".to_string())];

        let result = run_cli(args);
        if let Err(e) = &result {
            eprintln!("test_inline_variables error: {:?}", e);
        }
        assert!(result.is_ok());
    }

    #[test]
    fn test_output_to_file() {
        let temp_dir = TempDir::new().unwrap();
        let resource_path = temp_dir.path().join("patient.json");
        let output_path = temp_dir.path().join("output.json");

        fs::write(&resource_path, create_test_resource().to_string()).unwrap();

        let mut args = create_test_args("Patient.name.family", resource_path);
        args.output = Some(output_path.clone());

        let result = run_cli(args);
        assert!(result.is_ok());
        assert!(output_path.exists());

        let output_content = fs::read_to_string(output_path).unwrap();
        assert!(output_content.contains("\"Doe\""));
    }

    #[test]
    fn test_parse_debug_tree() {
        let temp_dir = TempDir::new().unwrap();
        let resource_path = temp_dir.path().join("dummy.json");
        fs::write(&resource_path, "{}").unwrap();

        let mut args = create_test_args("Patient.name.family", resource_path);
        args.parse_debug_tree = true;

        let result = run_cli(args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_debug() {
        let temp_dir = TempDir::new().unwrap();
        let resource_path = temp_dir.path().join("dummy.json");
        fs::write(&resource_path, "{}").unwrap();

        let mut args = create_test_args("Patient.name.family", resource_path);
        args.parse_debug = true;

        let result = run_cli(args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_invalid_resource_file() {
        let args = create_test_args("Patient.name", PathBuf::from("/nonexistent/file.json"));
        let result = run_cli(args);
        assert!(result.is_err());
    }

    #[test]
    fn test_stdin_support() {
        // This test would require mocking stdin, which is complex
        // For now, we'll just test the read_input function with a regular file
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.json");
        fs::write(&file_path, "test content").unwrap();

        let result = read_input(&file_path);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "test content");
    }

    #[test]
    fn test_json_value_to_result_conversions() {
        // Test null
        let result = json_value_to_result(&Value::Null).unwrap();
        assert!(matches!(result, EvaluationResult::Empty));

        // Test boolean
        let result = json_value_to_result(&json!(true)).unwrap();
        assert!(matches!(result, EvaluationResult::Boolean(true, _)));

        // Test integer
        let result = json_value_to_result(&json!(42)).unwrap();
        assert!(matches!(result, EvaluationResult::Integer(42, _)));

        // Test string
        let result = json_value_to_result(&json!("hello")).unwrap();
        match result {
            EvaluationResult::String(s, _) => assert_eq!(s, "hello"),
            _ => panic!("Expected string result"),
        }

        // Test array/object (converted to string)
        let result = json_value_to_result(&json!([1, 2, 3])).unwrap();
        assert!(matches!(result, EvaluationResult::String(_, _)));
    }

    #[test]
    fn test_result_to_json_single_value() {
        let result = EvaluationResult::string("test".to_string());
        let json_str = result_to_json(&result).unwrap();
        let json: Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(json, json!("test"));
    }

    #[test]
    fn test_result_to_json_collection() {
        let result = EvaluationResult::collection(vec![
            EvaluationResult::string("a".to_string()),
            EvaluationResult::string("b".to_string()),
        ]);
        let json_str = result_to_json(&result).unwrap();
        let json: Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(json, json!(["a", "b"]));
    }

    #[test]
    fn test_result_to_json_quantity() {
        use rust_decimal::Decimal;
        use std::str::FromStr;

        // Test Quantity with UCUM units
        let result =
            EvaluationResult::quantity(Decimal::from_str("1.5865").unwrap(), "cm".to_string());
        let json_str = result_to_json(&result).unwrap();
        let json: Value = serde_json::from_str(&json_str).unwrap();

        assert_eq!(
            json,
            json!({
                "value": 1.5865,
                "unit": "cm",
                "system": "http://unitsofmeasure.org",
                "code": "cm"
            })
        );

        // Verify that value is numeric, not a string
        assert!(json["value"].is_f64() || json["value"].is_i64());
        assert!(!json["value"].is_string());

        // Test Quantity with non-UCUM unit (arbitrary unit)
        let result_non_ucum =
            EvaluationResult::quantity(Decimal::from_str("42.0").unwrap(), "widgets".to_string());
        let json_str_non_ucum = result_to_json(&result_non_ucum).unwrap();
        let json_non_ucum: Value = serde_json::from_str(&json_str_non_ucum).unwrap();

        assert_eq!(
            json_non_ucum,
            json!({
                "value": 42.0,
                "unit": "widgets"
            })
        );

        // Should NOT have system/code for non-UCUM units
        assert!(json_non_ucum.get("system").is_none());
        assert!(json_non_ucum.get("code").is_none());
    }

    #[test]
    fn test_set_variable_json_types() {
        let mut context = EvaluationContext::new(vec![]);

        // Test setting different JSON types
        set_variable(&mut context, "str", "\"hello\"").unwrap();
        set_variable(&mut context, "num", "42").unwrap();
        set_variable(&mut context, "bool", "true").unwrap();
        set_variable(&mut context, "plain", "plain text").unwrap();

        // Verify variables were set (would need getter methods to fully test)
        assert!(set_variable(&mut context, "test", "value").is_ok());
    }

    #[test]
    fn test_parse_fhir_resource_r4() {
        #[cfg(feature = "R4")]
        {
            let json = create_test_resource();
            let result = parse_fhir_resource(json, FhirVersion::R4);
            assert!(result.is_ok());
            assert!(matches!(result.unwrap(), FhirResource::R4(_)));
        }
    }

    #[test]
    fn test_terminology_server_option() {
        let temp_dir = TempDir::new().unwrap();
        let resource_path = temp_dir.path().join("patient.json");
        fs::write(&resource_path, create_test_resource().to_string()).unwrap();

        let mut args = create_test_args("Patient.name", resource_path);
        args.terminology_server = Some("http://terminology.example.com".to_string());

        let result = run_cli(args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_trace_option() {
        let temp_dir = TempDir::new().unwrap();
        let resource_path = temp_dir.path().join("patient.json");
        fs::write(&resource_path, create_test_resource().to_string()).unwrap();

        let mut args = create_test_args("Patient.name", resource_path);
        args.trace = true;

        let result = run_cli(args);
        assert!(result.is_ok());
    }
}
