//! Request handlers for the FHIRPath server
//!
//! This module implements the HTTP request handlers for the FHIRPath server endpoints,
//! following the specification in server-api.md for fhirpath-lab integration.

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};
use tracing::{debug, info, warn};

use crate::error::{FhirPathError, FhirPathResult};
use crate::evaluator::EvaluationContext;
use crate::models::{ExtractedParameters, FhirPathParameters, extract_parameters};
use crate::parse_debug::{expression_to_debug_tree, generate_parse_debug};
use crate::type_inference::{TypeContext, InferredType};
use crate::{EvaluationResult, evaluate_expression};
use helios_fhir::{FhirResource, FhirVersion};

/// Handler for the main evaluation endpoint
///
/// This endpoint accepts a FHIR Parameters resource and evaluates the FHIRPath
/// expression against the provided resource, returning results in the format
/// specified by the fhirpath-lab API.
pub async fn evaluate_fhirpath(
    Json(params): Json<FhirPathParameters>,
) -> Result<Response, FhirPathError> {
    info!("Handling FHIRPath evaluation request");

    // Extract parameters
    let extracted = extract_parameters(params)?;
    debug!("Extracted parameters: {:?}", extracted);

    // Get expression
    let expression = extracted.expression.clone().ok_or_else(|| {
        FhirPathError::InvalidInput("Missing required parameter: expression".to_string())
    })?;

    // Get resource
    let resource_json = extracted.resource.clone().ok_or_else(|| {
        FhirPathError::InvalidInput("Missing required parameter: resource".to_string())
    })?;

    // Detect FHIR version from resource
    let fhir_version = detect_fhir_version(&resource_json);
    let fhir_resource = parse_fhir_resource(resource_json.clone(), fhir_version)?;

    // Create evaluation context
    let mut context = EvaluationContext::new(vec![fhir_resource]);

    // Set variables
    for var in &extracted.variables {
        set_variable_from_json(&mut context, &var.name, &var.value)?;
    }

    // Set terminology server if provided
    if let Some(ts) = &extracted.terminology_server {
        context.set_variable_result("terminologyServer", EvaluationResult::string(ts.clone()));
    }

    // Generate parse debug information if needed
    let (parse_debug_tree, parse_debug) = if extracted.validate {
        use chumsky::Parser as ChumskyParser;

        match crate::parser::parser().parse(expression.as_str()) {
            Ok(parsed) => {
                // Create a type context with the resource type
                let mut type_context = TypeContext::new();
                
                // Try to infer the root resource type from the resource JSON
                if let Some(resource_type) = resource_json.get("resourceType").and_then(|rt| rt.as_str()) {
                    type_context = type_context.with_root_type(InferredType::fhir(resource_type));
                }
                
                // Add any variables from the context
                for var in &extracted.variables {
                    // Simple type inference for variables - could be improved
                    let var_type = match &var.value {
                        Value::Bool(_) => InferredType::system("Boolean"),
                        Value::Number(n) => {
                            if n.is_i64() {
                                InferredType::system("Integer")
                            } else {
                                InferredType::system("Decimal")
                            }
                        }
                        Value::String(_) => InferredType::system("String"),
                        _ => InferredType::system("Any"),
                    };
                    type_context.variables.insert(var.name.clone(), var_type);
                }
                
                let debug_tree = expression_to_debug_tree(&parsed, &type_context);
                let debug_text = generate_parse_debug(&parsed);
                (Some(debug_tree), Some(debug_text))
            }
            Err(e) => {
                warn!("Parse error during validation: {:?}", e);
                (None, Some(format!("Parse error: {:?}", e)))
            }
        }
    } else {
        (None, None)
    };

    // Prepare results collection
    let mut results = Vec::new();
    
    // Clear any previous trace outputs
    context.clear_trace_outputs();

    // Evaluate with context if provided
    if let Some(context_expr) = &extracted.context {
        // Evaluate context expression
        let context_results = match evaluate_expression(context_expr, &context) {
            Ok(r) => r,
            Err(e) => {
                return create_error_response(&expression, &extracted, e);
            }
        };

        // Parse the main expression once
        use chumsky::Parser as ChumskyParser;
        let parsed_expr = match crate::parser::parser().parse(expression.as_str()) {
            Ok(parsed) => parsed,
            Err(e) => {
                return create_error_response(&expression, &extracted, 
                    format!("Parse error: {:?}", e));
            }
        };

        // For each context result, evaluate the main expression
        let mut context_index = 0;
        let context_items = match context_results {
            EvaluationResult::Collection { items, .. } => items,
            single_value => vec![single_value],
        };

        for context_value in context_items {
            // Clear trace outputs before each evaluation
            context.clear_trace_outputs();
            
            // Evaluate expression with context value as current item
            match crate::evaluator::evaluate(&parsed_expr, &context, Some(&context_value)) {
                Ok(result) => {
                    let context_path = format!("{}[{}]", context_expr, context_index);
                    // Get trace outputs collected during this evaluation
                    let trace_outputs = context.get_trace_outputs();
                    results.push(create_result_parameter(context_path, result, trace_outputs)?);
                }
                Err(e) => {
                    warn!("Evaluation error for context {}: {}", context_index, e);
                }
            }
            context_index += 1;
        }
    } else {
        // Evaluate without context
        match evaluate_expression(&expression, &context) {
            Ok(result) => {
                // Get trace outputs collected during evaluation
                let trace_outputs = context.get_trace_outputs();
                results.push(create_result_parameter("Resource".to_string(), result, trace_outputs)?);
            }
            Err(e) => {
                return create_error_response(&expression, &extracted, e);
            }
        }
    }

    // Build response
    let response = build_evaluation_response(
        &expression,
        &extracted,
        results,
        parse_debug_tree,
        parse_debug,
        resource_json,
    );

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handler for health check endpoint
pub async fn health_check() -> impl IntoResponse {
    Json(json!({
        "status": "ok",
        "service": "fhirpath-server"
    }))
}

/// Detect FHIR version from resource
fn detect_fhir_version(_resource: &Value) -> FhirVersion {
    // Simple heuristic - could be improved
    // Check for version-specific fields or meta information

    // Default to R4 for now
    FhirVersion::R4
}

/// Parse FHIR resource based on version
fn parse_fhir_resource(json: Value, version: FhirVersion) -> FhirPathResult<FhirResource> {
    match version {
        #[cfg(feature = "R4")]
        FhirVersion::R4 => {
            let resource: helios_fhir::r4::Resource = serde_json::from_value(json)
                .map_err(|e| FhirPathError::InvalidInput(format!("Invalid R4 resource: {}", e)))?;
            Ok(FhirResource::R4(Box::new(resource)))
        }
        #[cfg(feature = "R4B")]
        FhirVersion::R4B => {
            let resource: helios_fhir::r4b::Resource = serde_json::from_value(json)
                .map_err(|e| FhirPathError::InvalidInput(format!("Invalid R4B resource: {}", e)))?;
            Ok(FhirResource::R4B(Box::new(resource)))
        }
        #[cfg(feature = "R5")]
        FhirVersion::R5 => {
            let resource: helios_fhir::r5::Resource = serde_json::from_value(json)
                .map_err(|e| FhirPathError::InvalidInput(format!("Invalid R5 resource: {}", e)))?;
            Ok(FhirResource::R5(Box::new(resource)))
        }
        #[cfg(feature = "R6")]
        FhirVersion::R6 => {
            let resource: helios_fhir::r6::Resource = serde_json::from_value(json)
                .map_err(|e| FhirPathError::InvalidInput(format!("Invalid R6 resource: {}", e)))?;
            Ok(FhirResource::R6(Box::new(resource)))
        }
        #[cfg(not(any(feature = "R4", feature = "R4B", feature = "R5", feature = "R6")))]
        _ => Err(FhirPathError::InvalidInput(format!(
            "FHIR version {:?} is not enabled",
            version
        ))),
    }
}

/// Set variable from JSON value
fn set_variable_from_json(
    context: &mut EvaluationContext,
    name: &str,
    value: &Value,
) -> FhirPathResult<()> {
    let result = json_value_to_evaluation_result(value)?;
    context.set_variable_result(name, result);
    Ok(())
}

/// Convert JSON value to EvaluationResult
fn json_value_to_evaluation_result(value: &Value) -> FhirPathResult<EvaluationResult> {
    match value {
        Value::Null => Ok(EvaluationResult::Empty),
        Value::Bool(b) => Ok(EvaluationResult::boolean(*b)),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(EvaluationResult::integer(i))
            } else if let Some(f) = n.as_f64() {
                Ok(EvaluationResult::decimal(
                    rust_decimal::Decimal::try_from(f).map_err(|e| {
                        FhirPathError::InvalidInput(format!("Invalid decimal: {}", e))
                    })?,
                ))
            } else {
                Err(FhirPathError::InvalidInput("Invalid number".to_string()))
            }
        }
        Value::String(s) => Ok(EvaluationResult::string(s.clone())),
        Value::Array(arr) => {
            let results: Result<Vec<_>, _> =
                arr.iter().map(json_value_to_evaluation_result).collect();
            Ok(EvaluationResult::collection(results?))
        }
        Value::Object(_) => {
            // For complex objects, store as JSON string
            Ok(EvaluationResult::string(value.to_string()))
        }
    }
}

/// Create a result parameter
fn create_result_parameter(
    context_path: String,
    result: EvaluationResult,
    trace_outputs: Vec<(String, EvaluationResult)>,
) -> FhirPathResult<Value> {
    let mut parts = Vec::new();

    // Convert each result value
    let result_items = match result {
        EvaluationResult::Collection { items, .. } => items,
        single_value => vec![single_value],
    };

    for value in result_items {
        parts.push(evaluation_result_to_result_value(value)?);
    }
    
    // Add trace outputs as parts
    for (trace_name, trace_value) in trace_outputs {
        let mut trace_parts = Vec::new();
        
        // Convert trace values to result parts
        let trace_items = match trace_value {
            EvaluationResult::Collection { items, .. } => items,
            single_value => vec![single_value],
        };
        
        for value in trace_items {
            trace_parts.push(evaluation_result_to_result_value(value)?);
        }
        
        // Create trace part with name and valueString
        parts.push(json!({
            "name": "trace",
            "valueString": trace_name,
            "part": trace_parts
        }));
    }

    Ok(json!({
        "name": "result",
        "valueString": context_path,
        "part": parts
    }))
}

/// Convert object map to JSON
fn convert_object_to_json(map: &std::collections::HashMap<String, EvaluationResult>) -> Value {
    let mut json_map = serde_json::Map::new();
    
    for (key, value) in map {
        json_map.insert(key.clone(), convert_evaluation_result_to_json(value));
    }
    
    json!(json_map)
}

/// Convert EvaluationResult to JSON Value
fn convert_evaluation_result_to_json(result: &EvaluationResult) -> Value {
    match result {
        EvaluationResult::Empty => Value::Null,
        EvaluationResult::Boolean(b, _) => json!(b),
        EvaluationResult::String(s, _) => json!(s),
        EvaluationResult::Integer(i, _) => json!(i),
        EvaluationResult::Decimal(d, _) => json!(d.to_string()),
        EvaluationResult::Date(d, _) => json!(d),
        EvaluationResult::DateTime(dt, _) => json!(dt),
        EvaluationResult::Time(t, _) => json!(t),
        EvaluationResult::Quantity(v, u, _) => json!({"value": v, "unit": u}),
        #[cfg(not(any(feature = "R4", feature = "R4B")))]
        EvaluationResult::Integer64(i, _) => json!(i),
        #[cfg(any(feature = "R4", feature = "R4B"))]
        EvaluationResult::Integer64(i, _) => json!(i),
        EvaluationResult::Object { map, .. } => convert_object_to_json(map),
        EvaluationResult::Collection { items, .. } => {
            json!(items.iter().map(convert_evaluation_result_to_json).collect::<Vec<_>>())
        }
    }
}

/// Convert EvaluationResult to ResultValue
fn evaluation_result_to_result_value(result: EvaluationResult) -> FhirPathResult<Value> {
    // This is a simplified conversion
    // In a full implementation, we'd need proper type detection

    match result {
        EvaluationResult::Empty => Ok(json!({
            "name": "null"
        })),
        EvaluationResult::Boolean(b, type_info) => {
            let type_name = if let Some(info) = type_info {
                info.name.clone()
            } else {
                "boolean".to_string()
            };
            
            Ok(json!({
                "name": type_name,
                "valueBoolean": b
            }))
        },
        EvaluationResult::String(s, type_info) => {
            // Use the type information if available, otherwise default to "string"
            let type_name = if let Some(info) = type_info {
                // Use the FHIR type name from the type info
                info.name.clone()
            } else {
                "string".to_string()
            };
            
            // Use the correct FHIR value property name based on the type
            let value_property = match type_name.as_str() {
                "uri" => "valueUri",
                "url" => "valueUrl",
                "canonical" => "valueCanonical",
                "code" => "valueCode",
                "oid" => "valueOid",
                "id" => "valueId",
                "uuid" => "valueUuid",
                "markdown" => "valueMarkdown",
                "base64Binary" => "valueBase64Binary",
                _ => "valueString", // Default for string and other string-based types
            };
            
            Ok(json!({
                "name": type_name,
                value_property: s
            }))
        },
        EvaluationResult::Integer(i, type_info) => {
            let type_name = if let Some(info) = type_info {
                info.name.clone()
            } else {
                "integer".to_string()
            };
            
            // Use the correct FHIR value property name based on the type
            let value_property = match type_name.as_str() {
                "positiveInt" => "valuePositiveInt",
                "unsignedInt" => "valueUnsignedInt",
                _ => "valueInteger", // Default for integer type
            };
            
            Ok(json!({
                "name": type_name,
                value_property: i
            }))
        },
        EvaluationResult::Decimal(d, type_info) => {
            let type_name = if let Some(info) = type_info {
                info.name.clone()
            } else {
                "decimal".to_string()
            };
            
            Ok(json!({
                "name": type_name,
                "valueDecimal": d
            }))
        },
        EvaluationResult::Date(d, type_info) => {
            let type_name = if let Some(info) = type_info {
                info.name.clone()
            } else {
                "date".to_string()
            };
            
            Ok(json!({
                "name": type_name,
                "valueDate": d
            }))
        },
        EvaluationResult::DateTime(dt, type_info) => {
            let type_name = if let Some(info) = type_info {
                info.name.clone()
            } else {
                "dateTime".to_string()
            };
            
            // Use the correct FHIR value property name based on the type
            let value_property = match type_name.as_str() {
                "instant" => "valueInstant",
                _ => "valueDateTime", // Default for dateTime type
            };
            
            Ok(json!({
                "name": type_name,
                value_property: dt
            }))
        },
        EvaluationResult::Time(t, type_info) => {
            let type_name = if let Some(info) = type_info {
                info.name.clone()
            } else {
                "time".to_string()
            };
            
            Ok(json!({
                "name": type_name,
                "valueTime": t
            }))
        },
        EvaluationResult::Quantity(value, unit, _) => Ok(json!({
            "name": "quantity",
            "valueQuantity": {
                "value": value,
                "unit": unit
            }
        })),
        #[cfg(not(any(feature = "R4", feature = "R4B")))]
        EvaluationResult::Integer64(i, type_info) => {
            let type_name = if let Some(info) = type_info {
                info.name.clone()
            } else {
                "integer64".to_string()
            };
            
            Ok(json!({
                "name": type_name,
                "valueInteger": i
            }))
        },
        #[cfg(any(feature = "R4", feature = "R4B"))]
        EvaluationResult::Integer64(i, _) => {
            // In R4/R4B, treat as regular integer
            Ok(json!({
                "name": "integer",
                "valueInteger": i
            }))
        },
        EvaluationResult::Object { map, type_info } => {
            // For FHIR complex types, convert to JSON and use the extension mechanism
            // as specified in the server-api.md for values that can't be represented
            // as FHIR primitive types in Parameters
            let json_value = convert_object_to_json(&map);
            let string_value = serde_json::to_string(&json_value)
                .unwrap_or_else(|_| json_value.to_string());

            // Use the type name from type_info if available
            let type_name = type_info.as_ref()
                .map(|t| {
                    if t.namespace == "FHIR" {
                        t.name.clone()
                    } else {
                        format!("{}#{}", t.namespace, t.name)
                    }
                })
                .unwrap_or_else(|| "complex".to_string());

            Ok(json!({
                "name": type_name,
                "extension": [{
                    "url": "http://fhir.forms-lab.com/StructureDefinition/json-value",
                    "valueString": string_value
                }]
            }))
        }
        EvaluationResult::Collection { .. } => {
            // For collections at this level, convert to string representation
            let string_value = format!("{:?}", result);

            Ok(json!({
                "name": "complex",
                "extension": [{
                    "url": "http://fhir.forms-lab.com/StructureDefinition/json-value",
                    "valueString": string_value
                }]
            }))
        }
    }
}

/// Create error response
fn create_error_response(
    expression: &str,
    _params: &ExtractedParameters,
    error: String,
) -> Result<Response, FhirPathError> {
    let response = json!({
        "resourceType": "OperationOutcome",
        "issue": [{
            "severity": "error",
            "code": "processing",
            "diagnostics": error,
            "expression": [expression]
        }]
    });

    Ok((StatusCode::UNPROCESSABLE_ENTITY, Json(response)).into_response())
}

/// Build the evaluation response
fn build_evaluation_response(
    expression: &str,
    params: &ExtractedParameters,
    results: Vec<Value>,
    parse_debug_tree: Option<Value>,
    parse_debug: Option<String>,
    resource: Value,
) -> Value {
    let mut parameters = Vec::new();

    // Build parameters part
    let mut param_parts = vec![
        json!({
            "name": "evaluator",
            "valueString": format!("Helios Software FHIRPath-{}", env!("CARGO_PKG_VERSION"))
        }),
        json!({
            "name": "expression",
            "valueString": expression
        }),
        json!({
            "name": "resource",
            "resource": resource
        }),
    ];

    if let Some(context) = &params.context {
        param_parts.push(json!({
            "name": "context",
            "valueString": context
        }));
    }

    if let Some(tree) = parse_debug_tree {
        param_parts.push(json!({
            "name": "parseDebugTree",
            "valueString": serde_json::to_string(&tree).unwrap_or_default()
        }));
    }

    if let Some(debug) = parse_debug {
        param_parts.push(json!({
            "name": "parseDebug",
            "valueString": debug
        }));
    }

    if !params.variables.is_empty() {
        let var_parts: Vec<_> = params
            .variables
            .iter()
            .map(|v| {
                json!({
                    "name": v.name,
                    "valueString": v.value.to_string()
                })
            })
            .collect();

        param_parts.push(json!({
            "name": "variables",
            "part": var_parts
        }));
    }

    parameters.push(json!({
        "name": "parameters",
        "part": param_parts
    }));

    // Add results
    parameters.extend(results);

    json!({
        "resourceType": "Parameters",
        "id": "fhirpath",
        "parameter": parameters
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use helios_fhirpath_support::{EvaluationResult, TypeInfoResult};

    #[test]
    fn test_uri_uses_value_uri() {
        let result = EvaluationResult::fhir_string("http://example.com".to_string(), "uri");
        let json_result = evaluation_result_to_result_value(result).unwrap();
        
        assert_eq!(json_result["name"], "uri");
        assert_eq!(json_result["valueUri"], "http://example.com");
        assert!(json_result.get("valueString").is_none());
    }

    #[test]
    fn test_code_uses_value_code() {
        let result = EvaluationResult::fhir_string("MR".to_string(), "code");
        let json_result = evaluation_result_to_result_value(result).unwrap();
        
        assert_eq!(json_result["name"], "code");
        assert_eq!(json_result["valueCode"], "MR");
        assert!(json_result.get("valueString").is_none());
    }

    #[test]
    fn test_id_uses_value_id() {
        let result = EvaluationResult::fhir_string("12345".to_string(), "id");
        let json_result = evaluation_result_to_result_value(result).unwrap();
        
        assert_eq!(json_result["name"], "id");
        assert_eq!(json_result["valueId"], "12345");
        assert!(json_result.get("valueString").is_none());
    }

    #[test]
    fn test_string_uses_value_string() {
        let result = EvaluationResult::fhir_string("Hello World".to_string(), "string");
        let json_result = evaluation_result_to_result_value(result).unwrap();
        
        assert_eq!(json_result["name"], "string");
        assert_eq!(json_result["valueString"], "Hello World");
    }

    #[test]
    fn test_positive_int_uses_value_positive_int() {
        let result = EvaluationResult::Integer(
            42, 
            Some(TypeInfoResult::new("FHIR", "positiveInt"))
        );
        let json_result = evaluation_result_to_result_value(result).unwrap();
        
        assert_eq!(json_result["name"], "positiveInt");
        assert_eq!(json_result["valuePositiveInt"], 42);
        assert!(json_result.get("valueInteger").is_none());
    }

    #[test]
    fn test_instant_uses_value_instant() {
        let result = EvaluationResult::DateTime(
            "2023-01-01T12:00:00Z".to_string(),
            Some(TypeInfoResult::new("FHIR", "instant"))
        );
        let json_result = evaluation_result_to_result_value(result).unwrap();
        
        assert_eq!(json_result["name"], "instant");
        assert_eq!(json_result["valueInstant"], "2023-01-01T12:00:00Z");
        assert!(json_result.get("valueDateTime").is_none());
    }
}

