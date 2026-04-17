//! Request handlers for the FHIRPath server
//!
//! This module implements the HTTP request handlers for the FHIRPath server endpoints,
//! following the specification in server-api.md for fhirpath-lab integration.

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::Utc;
use serde_json::{Value, json};
use tracing::{debug, info, warn};

use crate::error::{FhirPathError, FhirPathResult};
use crate::evaluator::EvaluationContext;
use crate::models::{ExtractedParameters, FhirPathParameters, extract_parameters};
use crate::parse_debug::{generate_parse_debug, spanned_expression_to_debug_tree};
use crate::type_inference::{InferredType, TypeContext};
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
        context.set_terminology_server(ts.clone());
    }

    // Generate parse debug information if needed
    let (parse_debug_tree, parse_debug, expected_return_type) = if extracted.validate {
        use chumsky::Parser as ChumskyParser;

        match crate::parser::spanned_parser()
            .parse(expression.as_str())
            .into_result()
        {
            Ok(spanned) => {
                // Create a type context with the resource type
                let mut type_context = TypeContext::new();

                // Try to infer the root resource type from the resource JSON
                if let Some(resource_type) =
                    resource_json.get("resourceType").and_then(|rt| rt.as_str())
                {
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

                let debug_tree = spanned_expression_to_debug_tree(&spanned, &type_context);
                let parsed = spanned.to_expression();
                let debug_text = generate_parse_debug(&parsed);
                let return_type =
                    crate::type_inference::infer_expression_type(&parsed, &type_context)
                        .map(|t| t.to_display_string());
                (Some(debug_tree), Some(debug_text), return_type)
            }
            Err(e) => {
                warn!("Parse error during validation: {:?}", e);
                (None, Some(format!("Parse error: {:?}", e)), None)
            }
        }
    } else {
        (None, None, None)
    };

    // Check if debug-trace is enabled via environment variable
    let debug_trace_enabled = std::env::var("FHIRPATH_DEBUG_TRACE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    // Set up debug tracer if enabled
    if debug_trace_enabled {
        use chumsky::Parser as ChumskyParser;
        if let Ok(spanned) = crate::parser::spanned_parser()
            .parse(expression.as_str())
            .into_result()
        {
            let parsed = spanned.to_expression();
            let span_map = crate::debug_trace::build_span_map(&spanned, &parsed);
            let tracer = crate::debug_trace::DebugTracer::new(span_map);
            context.debug_tracer = Some(std::sync::Arc::new(parking_lot::Mutex::new(tracer)));
        }
    }

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
        let parsed_expr = match crate::parser::parser()
            .parse(expression.as_str())
            .into_result()
        {
            Ok(parsed) => parsed,
            Err(e) => {
                return create_error_response(
                    &expression,
                    &extracted,
                    format!("Parse error: {:?}", e),
                );
            }
        };

        // For each context result, evaluate the main expression
        let context_items = match context_results {
            EvaluationResult::Collection { items, .. } => items,
            single_value => vec![single_value],
        };

        for (context_index, context_value) in context_items.into_iter().enumerate() {
            // Clear trace outputs before each evaluation
            context.clear_trace_outputs();

            // Evaluate expression with context value as current item
            match crate::evaluator::evaluate(&parsed_expr, &context, Some(&context_value)) {
                Ok(result) => {
                    let context_path = format!("{}[{}]", context_expr, context_index);
                    // Get trace outputs collected during this evaluation
                    let trace_outputs = context.get_trace_outputs();
                    results.push(create_result_parameter(
                        Some(context_path),
                        result,
                        trace_outputs,
                    )?);
                }
                Err(e) => {
                    warn!("Evaluation error for context {}: {}", context_index, e);
                }
            }
        }
    } else {
        // Evaluate without context
        match evaluate_expression(&expression, &context) {
            Ok(result) => {
                // Get trace outputs collected during evaluation
                let trace_outputs = context.get_trace_outputs();
                results.push(create_result_parameter(None, result, trace_outputs)?);
            }
            Err(e) => {
                return create_error_response(&expression, &extracted, e);
            }
        }
    }

    // Collect debug trace steps if tracer was active
    let debug_trace_steps = if let Some(tracer) = &context.debug_tracer {
        let tracer = tracer.lock();
        if tracer.steps.is_empty() {
            None
        } else {
            Some(tracer.steps.clone())
        }
    } else {
        None
    };

    // Build response
    let response = build_evaluation_response(
        &expression,
        &extracted,
        results,
        parse_debug_tree,
        parse_debug,
        expected_return_type,
        resource_json,
        fhir_version,
        debug_trace_steps,
    );

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Helper function to evaluate FHIRPath with a specific version
async fn evaluate_fhirpath_with_version(
    params: FhirPathParameters,
    version: FhirVersion,
) -> Result<Response, FhirPathError> {
    info!(
        "Handling FHIRPath evaluation request for version {:?}",
        version
    );

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

    // Parse resource with specific version
    let fhir_resource = parse_fhir_resource(resource_json.clone(), version)?;

    // Create evaluation context
    let mut context = EvaluationContext::new(vec![fhir_resource]);

    // Preserve underscore properties in context
    preserve_underscore_properties(&mut context, &resource_json);

    // Set variables
    for var in &extracted.variables {
        set_variable_from_json(&mut context, &var.name, &var.value)?;
    }

    // Set terminology server if provided
    if let Some(ts) = &extracted.terminology_server {
        context.set_terminology_server(ts.clone());
    }

    // Generate parse debug information if needed
    let (parse_debug_tree, parse_debug, expected_return_type) = if extracted.validate {
        use chumsky::Parser as ChumskyParser;

        match crate::parser::spanned_parser()
            .parse(expression.as_str())
            .into_result()
        {
            Ok(spanned) => {
                // Create a type context with the resource type
                let mut type_context = TypeContext::new();

                // Try to infer the root resource type from the resource JSON
                if let Some(resource_type) =
                    resource_json.get("resourceType").and_then(|rt| rt.as_str())
                {
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

                let debug_tree = spanned_expression_to_debug_tree(&spanned, &type_context);
                let parsed = spanned.to_expression();
                let debug_text = generate_parse_debug(&parsed);
                let return_type =
                    crate::type_inference::infer_expression_type(&parsed, &type_context)
                        .map(|t| t.to_display_string());
                (Some(debug_tree), Some(debug_text), return_type)
            }
            Err(e) => {
                warn!("Parse error during validation: {:?}", e);
                (None, Some(format!("Parse error: {:?}", e)), None)
            }
        }
    } else {
        (None, None, None)
    };

    // Check if debug-trace is enabled via environment variable
    let debug_trace_enabled = std::env::var("FHIRPATH_DEBUG_TRACE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    // Set up debug tracer if enabled
    if debug_trace_enabled {
        use chumsky::Parser as ChumskyParser;
        if let Ok(spanned) = crate::parser::spanned_parser()
            .parse(expression.as_str())
            .into_result()
        {
            let parsed = spanned.to_expression();
            let span_map = crate::debug_trace::build_span_map(&spanned, &parsed);
            let tracer = crate::debug_trace::DebugTracer::new(span_map);
            context.debug_tracer = Some(std::sync::Arc::new(parking_lot::Mutex::new(tracer)));
        }
    }

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
        let parsed_expr = match crate::parser::parser()
            .parse(expression.as_str())
            .into_result()
        {
            Ok(parsed) => parsed,
            Err(e) => {
                return create_error_response(
                    &expression,
                    &extracted,
                    format!("Parse error: {:?}", e),
                );
            }
        };

        // For each context result, evaluate the main expression
        let context_items = match context_results {
            EvaluationResult::Collection { items, .. } => items,
            single_value => vec![single_value],
        };

        for (context_index, context_value) in context_items.into_iter().enumerate() {
            // Clear trace outputs before each evaluation
            context.clear_trace_outputs();

            // Evaluate expression with context value as current item
            match crate::evaluator::evaluate(&parsed_expr, &context, Some(&context_value)) {
                Ok(result) => {
                    let context_path = format!("{}[{}]", context_expr, context_index);
                    // Get trace outputs collected during this evaluation
                    let trace_outputs = context.get_trace_outputs();
                    results.push(create_result_parameter(
                        Some(context_path),
                        result,
                        trace_outputs,
                    )?);
                }
                Err(e) => {
                    warn!("Evaluation error for context {}: {}", context_index, e);
                }
            }
        }
    } else {
        // Evaluate without context
        match evaluate_expression(&expression, &context) {
            Ok(result) => {
                // Get trace outputs collected during evaluation
                let trace_outputs = context.get_trace_outputs();
                results.push(create_result_parameter(None, result, trace_outputs)?);
            }
            Err(e) => {
                return create_error_response(&expression, &extracted, e);
            }
        }
    }

    // Collect debug trace steps if tracer was active
    let debug_trace_steps = if let Some(tracer) = &context.debug_tracer {
        let tracer = tracer.lock();
        if tracer.steps.is_empty() {
            None
        } else {
            Some(tracer.steps.clone())
        }
    } else {
        None
    };

    // Build response
    let response = build_evaluation_response(
        &expression,
        &extracted,
        results,
        parse_debug_tree,
        parse_debug,
        expected_return_type,
        resource_json,
        version,
        debug_trace_steps,
    );

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handler for health check endpoint
pub async fn health_check() -> impl IntoResponse {
    Json(json!({
        "status": "ok",
        "service": "fhirpath-server",
        "version": env!("CARGO_PKG_VERSION"),
        "timestamp": Utc::now().to_rfc3339()
    }))
}

/// Handler for R4-specific evaluation
#[cfg(feature = "R4")]
pub async fn evaluate_fhirpath_r4(
    Json(params): Json<FhirPathParameters>,
) -> Result<Response, FhirPathError> {
    evaluate_fhirpath_with_version(params, FhirVersion::R4).await
}

/// Handler for R4B-specific evaluation
#[cfg(feature = "R4B")]
pub async fn evaluate_fhirpath_r4b(
    Json(params): Json<FhirPathParameters>,
) -> Result<Response, FhirPathError> {
    evaluate_fhirpath_with_version(params, FhirVersion::R4B).await
}

/// Handler for R5-specific evaluation
#[cfg(feature = "R5")]
pub async fn evaluate_fhirpath_r5(
    Json(params): Json<FhirPathParameters>,
) -> Result<Response, FhirPathError> {
    evaluate_fhirpath_with_version(params, FhirVersion::R5).await
}

/// Handler for R6-specific evaluation
#[cfg(feature = "R6")]
pub async fn evaluate_fhirpath_r6(
    Json(params): Json<FhirPathParameters>,
) -> Result<Response, FhirPathError> {
    evaluate_fhirpath_with_version(params, FhirVersion::R6).await
}

/// Detect FHIR version from resource
fn detect_fhir_version(resource: &Value) -> FhirVersion {
    // Try to detect version from meta.profile or other version-specific markers
    if let Some(meta) = resource.get("meta") {
        if let Some(profiles) = meta.get("profile").and_then(|p| p.as_array()) {
            for profile in profiles {
                if let Some(url) = profile.as_str() {
                    // Check for version indicators in profile URLs
                    #[cfg(feature = "R4B")]
                    if url.contains("/R4B/") || url.contains("/4.3.") {
                        return FhirVersion::R4B;
                    }
                    #[cfg(feature = "R5")]
                    if url.contains("/R5/") || url.contains("/5.0.") {
                        return FhirVersion::R5;
                    }
                    #[cfg(feature = "R6")]
                    if url.contains("/R6/") || url.contains("/6.0.") {
                        return FhirVersion::R6;
                    }
                    #[cfg(feature = "R4")]
                    if url.contains("/R4/") || url.contains("/4.0.") {
                        return FhirVersion::R4;
                    }
                }
            }
        }
    }

    // Check for version-specific fields
    // R5+ has meta.versionId as a distinct element
    // R4B has some specific elements in certain resources
    // For now, default to R4 if available, otherwise first available version
    #[cfg(feature = "R4")]
    return FhirVersion::R4;

    #[cfg(all(not(feature = "R4"), feature = "R4B"))]
    return FhirVersion::R4B;

    #[cfg(all(not(feature = "R4"), not(feature = "R4B"), feature = "R5"))]
    return FhirVersion::R5;

    #[cfg(all(
        not(feature = "R4"),
        not(feature = "R4B"),
        not(feature = "R5"),
        feature = "R6"
    ))]
    return FhirVersion::R6;
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

/// Preserve underscore properties in the evaluation context
/// This is needed because FHIR deserialization loses underscore properties
fn preserve_underscore_properties(context: &mut EvaluationContext, resource_json: &Value) {
    // Get the resource type and create a variable for it
    if let Value::Object(obj) = resource_json {
        if let Some(Value::String(resource_type)) = obj.get("resourceType") {
            // Clone the existing resource from context
            if let Some(existing_resource) = context.this.as_ref() {
                if let EvaluationResult::Object {
                    map: existing_map,
                    type_info,
                } = existing_resource
                {
                    let mut enhanced_map = existing_map.clone();

                    // Add underscore properties from JSON
                    for (key, value) in obj {
                        if key.starts_with('_') {
                            // Convert JSON value to EvaluationResult
                            if let Ok(eval_result) = json_value_to_evaluation_result(value) {
                                enhanced_map.insert(key.clone(), eval_result);
                            }
                        }
                    }

                    // Update context with enhanced resource
                    let enhanced_resource = EvaluationResult::Object {
                        map: enhanced_map,
                        type_info: type_info.clone(),
                    };
                    context.set_this(enhanced_resource.clone());

                    // Also set as a variable with the resource type name
                    context.set_variable_result(resource_type, enhanced_resource);
                }
            }
        }
    }
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
        Value::Object(obj) => {
            // Convert JSON object to EvaluationResult object
            let mut map = std::collections::HashMap::new();
            for (key, val) in obj {
                let eval_val = json_value_to_evaluation_result(val)?;
                map.insert(key.clone(), eval_val);
            }
            Ok(EvaluationResult::Object {
                map,
                type_info: None,
            })
        }
    }
}

/// Create a result parameter
fn create_result_parameter(
    context_path: Option<String>,
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

    let mut result_param = json!({
        "name": "result",
        "part": parts
    });

    // Only include valueString when a context expression is provided
    if let Some(path) = context_path {
        result_param["valueString"] = json!(path);
    }

    Ok(result_param)
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
        EvaluationResult::Quantity(v, u, _) => crate::json_utils::quantity_to_json(v, u),
        #[cfg(not(any(feature = "R4", feature = "R4B")))]
        EvaluationResult::Integer64(i, _) => json!(i),
        #[cfg(any(feature = "R4", feature = "R4B"))]
        EvaluationResult::Integer64(i, _) => json!(i),
        EvaluationResult::Object { map, .. } => convert_object_to_json(map),
        EvaluationResult::Collection { items, .. } => {
            json!(
                items
                    .iter()
                    .map(convert_evaluation_result_to_json)
                    .collect::<Vec<_>>()
            )
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
        }
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

            if s.is_empty() {
                Ok(json!({
                    "name": type_name,
                    value_property: s,
                    "part": [{ "name": "empty-string" }]
                }))
            } else {
                Ok(json!({
                    "name": type_name,
                    value_property: s
                }))
            }
        }
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
        }
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
        }
        EvaluationResult::Date(d, type_info) => {
            let type_name = if let Some(info) = type_info {
                info.name.clone()
            } else {
                "date".to_string()
            };

            // Strip @ prefix if present
            let date_value = if let Some(stripped) = d.strip_prefix('@') {
                stripped
            } else {
                d.as_str()
            };

            Ok(json!({
                "name": type_name,
                "valueDate": date_value
            }))
        }
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

            // Strip @ prefix if present
            let datetime_value = if let Some(stripped) = dt.strip_prefix('@') {
                stripped
            } else {
                dt.as_str()
            };

            Ok(json!({
                "name": type_name,
                value_property: datetime_value
            }))
        }
        EvaluationResult::Time(t, type_info) => {
            let type_name = if let Some(info) = type_info {
                info.name.clone()
            } else {
                "time".to_string()
            };

            // Strip @T prefix if present
            let time_value = if let Some(stripped) = t.strip_prefix("@T") {
                stripped
            } else if let Some(stripped) = t.strip_prefix('@') {
                // Also try stripping just @ if @T is not found
                stripped
            } else {
                t.as_str()
            };

            Ok(json!({
                "name": type_name,
                "valueTime": time_value
            }))
        }
        EvaluationResult::Quantity(value, unit, _) => {
            let value_quantity = crate::json_utils::quantity_to_json(&value, &unit);

            Ok(json!({
                "name": "quantity",
                "valueQuantity": value_quantity
            }))
        }
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
        }
        #[cfg(any(feature = "R4", feature = "R4B"))]
        EvaluationResult::Integer64(i, _) => {
            // In R4/R4B, treat as regular integer
            Ok(json!({
                "name": "integer",
                "valueInteger": i
            }))
        }
        EvaluationResult::Object { map, type_info } => {
            // For FHIR complex types, convert to JSON and use the extension mechanism
            // as specified in the server-api.md for values that can't be represented
            // as FHIR primitive types in Parameters
            let json_value = convert_object_to_json(&map);
            let string_value =
                serde_json::to_string(&json_value).unwrap_or_else(|_| json_value.to_string());

            // Use the type name from type_info if available
            let type_name = type_info
                .as_ref()
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
    // Check if this is an unsupported function error
    if error.contains("Unsupported Function:") || error.contains("is not implemented") {
        return Err(FhirPathError::NotImplemented(error));
    }

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
#[allow(clippy::too_many_arguments)]
fn build_evaluation_response(
    expression: &str,
    params: &ExtractedParameters,
    results: Vec<Value>,
    parse_debug_tree: Option<Value>,
    parse_debug: Option<String>,
    expected_return_type: Option<String>,
    resource: Value,
    fhir_version: FhirVersion,
    debug_trace_steps: Option<Vec<crate::debug_trace::DebugTraceStep>>,
) -> Value {
    let mut parameters = Vec::new();

    // Build parameters part
    let mut param_parts = vec![
        json!({
            "name": "evaluator",
            "valueString": format!("Helios Software-{} ({:?})", env!("CARGO_PKG_VERSION"), fhir_version)
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

    if let Some(return_type) = expected_return_type {
        param_parts.push(json!({
            "name": "expectedReturnType",
            "valueString": return_type
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

    // Add debug-trace if steps were recorded
    if let Some(steps) = debug_trace_steps {
        let trace_parts: Vec<Value> = steps
            .into_iter()
            .map(|step| {
                let step_name = format!("{},{},{}", step.position, step.length, step.function_name);
                let result_parts = evaluation_result_to_trace_parts(step.result);
                json!({
                    "name": step_name,
                    "part": result_parts
                })
            })
            .collect();

        parameters.push(json!({
            "name": "debug-trace",
            "valueString": "Context[0]",
            "part": trace_parts
        }));
    }

    json!({
        "resourceType": "Parameters",
        "id": "fhirpath",
        "parameter": parameters
    })
}

/// Convert an EvaluationResult into debug-trace part entries.
///
/// Each item in the result collection becomes a part entry using
/// `evaluation_result_to_result_value`. If the result is a collection,
/// each element gets its own part entry.
fn evaluation_result_to_trace_parts(result: EvaluationResult) -> Vec<Value> {
    match result {
        EvaluationResult::Collection { items, .. } => items
            .into_iter()
            .filter_map(|item| evaluation_result_to_result_value(item).ok())
            .collect(),
        other => evaluation_result_to_result_value(other)
            .map(|v| vec![v])
            .unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ExtractedParameters;
    use helios_fhirpath_support::{EvaluationResult, TypeInfoResult};
    use serde_json::json;

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
        let result =
            EvaluationResult::Integer(42, Some(TypeInfoResult::new("FHIR", "positiveInt")));
        let json_result = evaluation_result_to_result_value(result).unwrap();

        assert_eq!(json_result["name"], "positiveInt");
        assert_eq!(json_result["valuePositiveInt"], 42);
        assert!(json_result.get("valueInteger").is_none());
    }

    #[test]
    fn test_instant_uses_value_instant() {
        let result = EvaluationResult::DateTime(
            "2023-01-01T12:00:00Z".to_string(),
            Some(TypeInfoResult::new("FHIR", "instant")),
        );
        let json_result = evaluation_result_to_result_value(result).unwrap();

        assert_eq!(json_result["name"], "instant");
        assert_eq!(json_result["valueInstant"], "2023-01-01T12:00:00Z");
        assert!(json_result.get("valueDateTime").is_none());
    }

    #[test]
    fn test_quantity_json_format() {
        use rust_decimal::Decimal;
        use std::str::FromStr;

        // Test that Quantity results have numeric value and UCUM fields for UCUM units
        let quantity =
            EvaluationResult::quantity(Decimal::from_str("1.5865").unwrap(), "cm".to_string());

        let json_value = convert_evaluation_result_to_json(&quantity);

        // Check the structure for UCUM unit
        assert_eq!(json_value["value"], 1.5865);
        assert_eq!(json_value["unit"], "cm");
        assert_eq!(json_value["system"], "http://unitsofmeasure.org");
        assert_eq!(json_value["code"], "cm");

        // Verify value is numeric
        assert!(json_value["value"].is_f64());
        assert!(!json_value["value"].is_string());

        // Test non-UCUM unit
        let non_ucum_quantity = EvaluationResult::quantity(
            Decimal::from_str("100").unwrap(),
            "custom_unit".to_string(),
        );

        let non_ucum_json = convert_evaluation_result_to_json(&non_ucum_quantity);

        // Check the structure for non-UCUM unit
        assert_eq!(non_ucum_json["value"], 100.0);
        assert_eq!(non_ucum_json["unit"], "custom_unit");
        // Should NOT have system/code for non-UCUM units
        assert!(non_ucum_json.get("system").is_none());
        assert!(non_ucum_json.get("code").is_none());
    }

    #[test]
    fn test_quantity_value_quantity_format() {
        use rust_decimal::Decimal;
        use std::str::FromStr;

        // Test that valueQuantity in server responses has correct format for UCUM units
        let quantity =
            EvaluationResult::quantity(Decimal::from_str("1.5865").unwrap(), "cm".to_string());

        let result_value = evaluation_result_to_result_value(quantity).unwrap();

        assert_eq!(result_value["name"], "quantity");
        assert_eq!(result_value["valueQuantity"]["value"], 1.5865);
        assert_eq!(result_value["valueQuantity"]["unit"], "cm");
        assert_eq!(
            result_value["valueQuantity"]["system"],
            "http://unitsofmeasure.org"
        );
        assert_eq!(result_value["valueQuantity"]["code"], "cm");

        // Verify value is numeric
        assert!(result_value["valueQuantity"]["value"].is_f64());
        assert!(!result_value["valueQuantity"]["value"].is_string());

        // Test non-UCUM unit
        let non_ucum_quantity =
            EvaluationResult::quantity(Decimal::from_str("42.0").unwrap(), "widgets".to_string());

        let non_ucum_result = evaluation_result_to_result_value(non_ucum_quantity).unwrap();

        assert_eq!(non_ucum_result["name"], "quantity");
        assert_eq!(non_ucum_result["valueQuantity"]["value"], 42.0);
        assert_eq!(non_ucum_result["valueQuantity"]["unit"], "widgets");
        // Should NOT have system/code for non-UCUM units
        assert!(non_ucum_result["valueQuantity"].get("system").is_none());
        assert!(non_ucum_result["valueQuantity"].get("code").is_none());
    }

    #[test]
    fn test_evaluator_format_r4() {
        let params = ExtractedParameters {
            expression: Some("Patient.name".to_string()),
            context: None,
            resource: Some(json!({"resourceType": "Patient"})),
            variables: vec![],
            validate: false,
            terminology_server: None,
        };

        let response = build_evaluation_response(
            "Patient.name",
            &params,
            vec![],
            None,
            None,
            None,
            json!({"resourceType": "Patient"}),
            FhirVersion::R4,
            None,
        );

        // Extract the evaluator value
        let evaluator_value = response["parameter"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "parameters")
            .unwrap()["part"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "evaluator")
            .unwrap()["valueString"]
            .as_str()
            .unwrap();

        assert!(evaluator_value.starts_with("Helios Software-"));
        assert!(evaluator_value.ends_with(" (R4)"));
    }

    #[cfg(feature = "R5")]
    #[test]
    fn test_evaluator_format_r5() {
        let params = ExtractedParameters {
            expression: Some("Patient.name".to_string()),
            context: None,
            resource: Some(json!({"resourceType": "Patient"})),
            variables: vec![],
            validate: false,
            terminology_server: None,
        };

        let response = build_evaluation_response(
            "Patient.name",
            &params,
            vec![],
            None,
            None,
            None,
            json!({"resourceType": "Patient"}),
            FhirVersion::R5,
            None,
        );

        // Extract the evaluator value
        let evaluator_value = response["parameter"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "parameters")
            .unwrap()["part"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "evaluator")
            .unwrap()["valueString"]
            .as_str()
            .unwrap();

        assert!(evaluator_value.starts_with("Helios Software-"));
        assert!(evaluator_value.ends_with(" (R5)"));
    }
}
