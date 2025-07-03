//! Common test utilities for server tests

use axum::response::IntoResponse;
use axum::{Json, Router};
use axum_test::TestServer;

/// Create a test server instance
pub async fn test_server() -> TestServer {
    let app = create_test_app();
    TestServer::new(app).expect("Failed to create test server")
}

/// Create the test application (copied from server.rs to avoid binary/lib conflicts)
fn create_test_app() -> Router {
    use axum::routing::{get, post};
    use tower_http::cors::CorsLayer;

    // Import handlers from the sof crate
    // Note: We need to ensure these are properly exported from lib.rs

    Router::new()
        .route("/metadata", get(capability_statement_handler))
        .route(
            "/ViewDefinition/$run",
            post(run_view_definition_handler).get(run_view_definition_get_handler),
        )
        .route(
            "/ViewDefinition/:id/$run",
            get(run_view_definition_by_id_handler),
        )
        .route("/health", get(health_check))
        .layer(CorsLayer::permissive())
}

/// Placeholder handlers - these will be replaced with actual imports
async fn capability_statement_handler() -> axum::response::Response {
    // This is a simplified version for testing
    // In production, this would use the actual handler from sof::server::handlers

    let capability_statement = serde_json::json!({
        "resourceType": "CapabilityStatement",
        "id": "sof-server",
        "name": "SQL-on-FHIR Server",
        "title": "SQL-on-FHIR Server CapabilityStatement",
        "status": "active",
        "date": chrono::Utc::now().to_rfc3339(),
        "publisher": "SQL-on-FHIR Implementation",
        "kind": "instance",
        "software": {
            "name": "sof-server",
            "version": env!("CARGO_PKG_VERSION")
        },
        "implementation": {
            "description": "SQL-on-FHIR ViewDefinition Runner",
            "url": "http://localhost:8080"
        },
        "fhirVersion": "4.0.1",
        "format": ["json", "xml"],
        "rest": [{
            "mode": "server",
            "resource": [{
                "type": "ViewDefinition",
                "operation": [{
                    "name": "run",
                    "definition": "http://sql-on-fhir.org/OperationDefinition/ViewDefinition-run",
                    "documentation": "Execute a ViewDefinition to transform FHIR resources into tabular format"
                }]
            }],
            "operation": [{
                "name": "run",
                "definition": "http://sql-on-fhir.org/OperationDefinition/ViewDefinition-run",
                "documentation": "Execute a ViewDefinition to transform FHIR resources into tabular format. Supports CSV, JSON, and NDJSON output formats."
            }]
        }]
    });

    (
        axum::http::StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/fhir+json")],
        Json(capability_statement),
    )
        .into_response()
}

async fn run_view_definition_handler(
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
    headers: axum::http::HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> axum::response::Response {
    // This would use the actual handler in production
    // For now, we'll implement a basic version for testing

    use sof::{ContentType, SofBundle, SofViewDefinition, run_view_definition};

    // Validate query parameters first
    if let Err(e) = validate_query_params(&params) {
        return error_response(axum::http::StatusCode::BAD_REQUEST, &e);
    }

    // Basic parameter parsing
    if body["resourceType"] != "Parameters" {
        return error_response(
            axum::http::StatusCode::BAD_REQUEST,
            "Request body must be a Parameters resource",
        );
    }

    // Extract ViewDefinition and resources
    let mut view_def_json = None;
    let mut resources = Vec::new();
    let mut format_from_body = None;
    let mut header_from_body = None;
    let mut patient_filter = None;
    let mut count_from_body = None;
    let mut page_from_body = None;

    if let Some(parameters) = body["parameter"].as_array() {
        for param in parameters {
            match param["name"].as_str() {
                Some("viewResource") => {
                    view_def_json = param["resource"].as_object().cloned();
                }
                Some("viewReference") => {
                    // viewReference is not implemented
                    return error_response(
                        axum::http::StatusCode::NOT_IMPLEMENTED,
                        "The viewReference parameter is not yet implemented. Please provide the ViewDefinition directly using the viewResource parameter.",
                    );
                }
                Some("group") => {
                    // group is not implemented
                    return error_response(
                        axum::http::StatusCode::NOT_IMPLEMENTED,
                        "The group parameter is not yet implemented.",
                    );
                }
                Some("source") => {
                    // source is not implemented
                    return error_response(
                        axum::http::StatusCode::NOT_IMPLEMENTED,
                        "The source parameter is not yet implemented.",
                    );
                }
                Some("resource") => {
                    if let Some(resource) = param["resource"].as_object() {
                        // Check if it's a Bundle
                        if resource.get("resourceType") == Some(&serde_json::json!("Bundle")) {
                            // Extract resources from bundle
                            if let Some(entries) = resource.get("entry").and_then(|e| e.as_array())
                            {
                                for entry in entries {
                                    if let Some(res) = entry.get("resource") {
                                        resources.push(res.clone());
                                    }
                                }
                            }
                        } else {
                            resources.push(serde_json::Value::Object(resource.clone()));
                        }
                    }
                }
                Some("patient") => {
                    // Handle patient parameter
                    if let Some(value_ref) = param.get("valueReference") {
                        if let Some(reference) = value_ref.get("reference").and_then(|r| r.as_str())
                        {
                            patient_filter = Some(reference.to_string());
                        }
                    } else if let Some(value_str) =
                        param.get("valueString").and_then(|v| v.as_str())
                    {
                        patient_filter = Some(value_str.to_string());
                    }
                }
                Some("_format") | Some("format") => {
                    // Extract format from valueCode or valueString
                    if let Some(value_code) = param["valueCode"].as_str() {
                        format_from_body = Some(value_code.to_string());
                    } else if let Some(value_string) = param["valueString"].as_str() {
                        format_from_body = Some(value_string.to_string());
                    }
                }
                Some("header") => {
                    // Extract header from valueBoolean
                    if let Some(value_bool) = param["valueBoolean"].as_bool() {
                        header_from_body = Some(value_bool);
                    } else if param.get("valueString").is_some()
                        || param.get("valueCode").is_some()
                        || param.get("valueInteger").is_some()
                    {
                        return error_response(
                            axum::http::StatusCode::BAD_REQUEST,
                            "Header parameter must be a boolean value (use valueBoolean)",
                        );
                    }
                }
                Some("_count") => {
                    // Extract count from valueInteger or valuePositiveInt
                    if let Some(value_int) = param.get("valueInteger").and_then(|v| v.as_i64()) {
                        if value_int <= 0 {
                            return error_response(
                                axum::http::StatusCode::BAD_REQUEST,
                                "_count parameter must be greater than 0",
                            );
                        }
                        if value_int > 10000 {
                            return error_response(
                                axum::http::StatusCode::BAD_REQUEST,
                                "_count parameter cannot exceed 10000",
                            );
                        }
                        count_from_body = Some(value_int as u32);
                    } else if let Some(value_pos) =
                        param.get("valuePositiveInt").and_then(|v| v.as_u64())
                    {
                        if value_pos > 10000 {
                            return error_response(
                                axum::http::StatusCode::BAD_REQUEST,
                                "_count parameter cannot exceed 10000",
                            );
                        }
                        count_from_body = Some(value_pos as u32);
                    }
                }
                Some("_page") => {
                    // Extract page from valueInteger or valuePositiveInt
                    if let Some(value_int) = param.get("valueInteger").and_then(|v| v.as_i64()) {
                        if value_int <= 0 {
                            return error_response(
                                axum::http::StatusCode::BAD_REQUEST,
                                "_page parameter must be greater than 0",
                            );
                        }
                        page_from_body = Some(value_int as u32);
                    } else if let Some(value_pos) =
                        param.get("valuePositiveInt").and_then(|v| v.as_u64())
                    {
                        page_from_body = Some(value_pos as u32);
                    }
                }
                Some("_since") => {
                    // Check if any value[X] field exists
                    let has_value_field = param
                        .as_object()
                        .map_or(false, |obj| obj.keys().any(|k| k.starts_with("value")));

                    // Extract and validate _since from valueInstant or valueDateTime
                    if let Some(value_instant) = param.get("valueInstant").and_then(|v| v.as_str())
                    {
                        if chrono::DateTime::parse_from_rfc3339(value_instant).is_err() {
                            return error_response(
                                axum::http::StatusCode::BAD_REQUEST,
                                &format!(
                                    "_since parameter must be a valid RFC3339 timestamp: {}",
                                    value_instant
                                ),
                            );
                        }
                    } else if let Some(value_datetime) =
                        param.get("valueDateTime").and_then(|v| v.as_str())
                    {
                        if chrono::DateTime::parse_from_rfc3339(value_datetime).is_err() {
                            return error_response(
                                axum::http::StatusCode::BAD_REQUEST,
                                &format!(
                                    "_since parameter must be a valid RFC3339 timestamp: {}",
                                    value_datetime
                                ),
                            );
                        }
                    } else if has_value_field {
                        return error_response(
                            axum::http::StatusCode::BAD_REQUEST,
                            "_since parameter must use valueInstant or valueDateTime",
                        );
                    }
                }
                _ => {}
            }
        }
    }

    let view_def_json = match view_def_json {
        Some(v) => serde_json::Value::Object(v),
        None => {
            return error_response(
                axum::http::StatusCode::BAD_REQUEST,
                "No ViewDefinition provided",
            );
        }
    };

    // Apply patient filter if provided
    if let Some(patient_ref) = patient_filter {
        // Normalize the patient reference to always include "Patient/" prefix
        let normalized_patient_ref = if patient_ref.starts_with("Patient/") {
            patient_ref
        } else {
            format!("Patient/{}", patient_ref)
        };

        resources = resources
            .into_iter()
            .filter(|resource| {
                if let Some(resource_type) = resource.get("resourceType").and_then(|r| r.as_str()) {
                    match resource_type {
                        "Patient" => {
                            if let Some(id) = resource.get("id").and_then(|i| i.as_str()) {
                                return format!("Patient/{}", id) == normalized_patient_ref;
                            }
                        }
                        "Observation" | "Condition" | "MedicationRequest" | "Procedure" => {
                            if let Some(subject) = resource.get("subject") {
                                if let Some(reference) =
                                    subject.get("reference").and_then(|r| r.as_str())
                                {
                                    return reference == normalized_patient_ref;
                                }
                            }
                        }
                        _ => {}
                    }
                }
                false
            })
            .collect();
    }

    // Apply _since filter if provided
    let since_from_query = params.get("_since").map(|s| s.as_str());
    let since_filter = if let Some(parameters) = body["parameter"].as_array() {
        if let Some(param) = parameters
            .iter()
            .find(|p| p.get("name").and_then(|n| n.as_str()) == Some("_since"))
        {
            // Get from body parameter (takes precedence)
            param
                .get("valueInstant")
                .or_else(|| param.get("valueDateTime"))
                .and_then(|v| v.as_str())
        } else {
            since_from_query
        }
    } else {
        since_from_query
    };

    if let Some(since_str) = since_filter {
        // Parse the _since timestamp
        if let Ok(since_dt) = chrono::DateTime::parse_from_rfc3339(since_str) {
            let since_utc = since_dt.with_timezone(&chrono::Utc);

            resources = resources
                .into_iter()
                .filter(|resource| {
                    // Check if resource has meta.lastUpdated field
                    if let Some(meta) = resource.get("meta") {
                        if let Some(last_updated) =
                            meta.get("lastUpdated").and_then(|lu| lu.as_str())
                        {
                            // Parse the lastUpdated timestamp
                            if let Ok(resource_updated) =
                                chrono::DateTime::parse_from_rfc3339(last_updated)
                            {
                                // Compare timestamps - keep if resource was updated after _since
                                return resource_updated.with_timezone(&chrono::Utc) > since_utc;
                            }
                        }
                    }
                    // If no meta.lastUpdated field, exclude the resource
                    false
                })
                .collect();
        }
    }

    // Parse content type - body format takes precedence
    let format = format_from_body
        .as_ref()
        .map(|s| s.as_str())
        .or_else(|| params.get("_format").map(|s| s.as_str()));
    let accept = if format_from_body.is_some() {
        None // Ignore Accept header when body param is present
    } else {
        headers
            .get(axum::http::header::ACCEPT)
            .and_then(|h| h.to_str().ok())
    };

    // Convert header parameter - body takes precedence over query
    let header_param = if let Some(header_bool) = header_from_body {
        Some(header_bool)
    } else {
        match params.get("header").map(|s| s.as_str()) {
            Some("true") => Some(true),
            Some("false") => Some(false),
            _ => None,
        }
    };

    // Check if header parameter is being used with non-CSV format
    if header_param.is_some() && format_from_body.is_none() {
        // We have a header parameter but need to check if format is CSV
        let test_format = format.or(accept).unwrap_or("application/json");
        if test_format != "text/csv" {
            return error_response(
                axum::http::StatusCode::BAD_REQUEST,
                "Header parameter only applies to CSV format",
            );
        }
    }

    let content_type = match parse_content_type(accept, format, header_param) {
        Ok(ct) => ct,
        Err(e) => match e {
            sof::SofError::UnsupportedContentType(_) => {
                return error_response(
                    axum::http::StatusCode::UNSUPPORTED_MEDIA_TYPE,
                    &e.to_string(),
                );
            }
            _ => return error_response(axum::http::StatusCode::BAD_REQUEST, &e.to_string()),
        },
    };

    // Create ViewDefinition and Bundle
    let view_definition = match serde_json::from_value::<fhir::r4::ViewDefinition>(view_def_json) {
        Ok(vd) => SofViewDefinition::R4(vd),
        Err(e) => {
            return error_response(
                axum::http::StatusCode::BAD_REQUEST,
                &format!("Invalid ViewDefinition: {}", e),
            );
        }
    };

    let bundle_json = serde_json::json!({
        "resourceType": "Bundle",
        "type": "collection",
        "entry": resources.into_iter().map(|r| {
            serde_json::json!({"resource": r})
        }).collect::<Vec<_>>()
    });

    let bundle = match serde_json::from_value::<fhir::r4::Bundle>(bundle_json) {
        Ok(b) => SofBundle::R4(b),
        Err(e) => {
            return error_response(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                &format!("Failed to create Bundle: {}", e),
            );
        }
    };

    // Execute ViewDefinition
    match run_view_definition(view_definition, bundle, content_type) {
        Ok(output) => {
            // Apply pagination if requested
            let output = if let Some(paginated) = apply_pagination(
                output,
                &params,
                &content_type,
                count_from_body,
                page_from_body,
            ) {
                paginated
            } else {
                return error_response(
                    axum::http::StatusCode::BAD_REQUEST,
                    "Invalid pagination parameters",
                );
            };

            let mime_type = match content_type {
                ContentType::Csv | ContentType::CsvWithHeader => "text/csv",
                ContentType::Json => "application/json",
                ContentType::NdJson => "application/ndjson",
                ContentType::Parquet => "application/parquet",
            };

            (
                axum::http::StatusCode::OK,
                [(axum::http::header::CONTENT_TYPE, mime_type)],
                output,
            )
                .into_response()
        }
        Err(e) => error_response(axum::http::StatusCode::UNPROCESSABLE_ENTITY, &e.to_string()),
    }
}

fn parse_content_type(
    accept: Option<&str>,
    format: Option<&str>,
    header: Option<bool>,
) -> Result<sof::ContentType, sof::SofError> {
    use sof::ContentType;

    let content_type_str = format.or(accept).unwrap_or("application/json");

    let content_type_str = if content_type_str == "text/csv" {
        match header {
            Some(false) => "text/csv;header=false",
            Some(true) | None => "text/csv;header=true", // Default to true if not specified
        }
    } else {
        content_type_str
    };

    ContentType::from_string(content_type_str)
}

fn error_response(status: axum::http::StatusCode, message: &str) -> axum::response::Response {
    let operation_outcome = serde_json::json!({
        "resourceType": "OperationOutcome",
        "issue": [{
            "severity": "error",
            "code": "invalid",
            "details": {
                "text": message
            }
        }]
    });

    (status, Json(operation_outcome)).into_response()
}

async fn health_check() -> impl axum::response::IntoResponse {
    Json(serde_json::json!({
        "status": "ok",
        "service": "sof-server",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

async fn run_view_definition_get_handler(
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
    _headers: axum::http::HeaderMap,
) -> axum::response::Response {
    // Per FHIR spec, GET operations cannot use complex parameters
    // Validate that no complex parameters are provided
    if params.contains_key("viewReference") {
        return error_response(
            axum::http::StatusCode::BAD_REQUEST,
            "GET operations cannot use complex parameters like viewReference. Use POST instead.",
        );
    }
    if params.contains_key("patient") {
        return error_response(
            axum::http::StatusCode::BAD_REQUEST,
            "GET operations cannot use complex parameters like patient. Use POST instead.",
        );
    }
    if params.contains_key("group") {
        return error_response(
            axum::http::StatusCode::BAD_REQUEST,
            "GET operations cannot use complex parameters like group. Use POST instead.",
        );
    }

    // Check for source parameter - return NotImplemented
    if params.contains_key("source") {
        return error_response(
            axum::http::StatusCode::NOT_IMPLEMENTED,
            "The source parameter is not supported in this stateless implementation. Please provide resources in the request body.",
        );
    }

    // For GET requests without a ViewDefinition, we cannot proceed
    error_response(
        axum::http::StatusCode::BAD_REQUEST,
        "GET /ViewDefinition/$run requires a ViewDefinition to be provided. Since complex parameters cannot be used in GET requests, please use POST with viewResource or viewReference parameter.",
    )
}

async fn run_view_definition_by_id_handler(
    axum::extract::Path(id): axum::extract::Path<String>,
    _query: axum::extract::Query<std::collections::HashMap<String, String>>,
    _headers: axum::http::HeaderMap,
) -> axum::response::Response {
    error_response(
        axum::http::StatusCode::NOT_IMPLEMENTED,
        &format!(
            "ViewDefinition lookup by ID '{}' is not implemented. Use POST /ViewDefinition/$run with the ViewDefinition in the request body.",
            id
        ),
    )
}

fn validate_query_params(params: &std::collections::HashMap<String, String>) -> Result<(), String> {
    // Validate _count parameter
    if let Some(count_str) = params.get("_count") {
        match count_str.parse::<usize>() {
            Ok(count) => {
                if count == 0 {
                    return Err("_count parameter must be greater than 0".to_string());
                }
                if count > 10000 {
                    return Err("_count parameter cannot exceed 10000".to_string());
                }
            }
            Err(_) => return Err("_count parameter must be a valid number".to_string()),
        }
    }

    // Validate _page parameter
    if let Some(page_str) = params.get("_page") {
        match page_str.parse::<usize>() {
            Ok(page) => {
                if page == 0 {
                    return Err("_page parameter must be greater than 0 (1-based)".to_string());
                }
            }
            Err(_) => return Err("_page parameter must be a valid number".to_string()),
        }
    }

    // Validate _since parameter
    if let Some(since_str) = params.get("_since") {
        if chrono::DateTime::parse_from_rfc3339(since_str).is_err() {
            return Err(format!(
                "_since parameter must be a valid RFC3339 timestamp: {}",
                since_str
            ));
        }
    }

    Ok(())
}

fn apply_pagination(
    output: Vec<u8>,
    params: &std::collections::HashMap<String, String>,
    content_type: &sof::ContentType,
    count_from_body: Option<u32>,
    page_from_body: Option<u32>,
) -> Option<Vec<u8>> {
    // Body parameters take precedence over query parameters
    let count = count_from_body
        .map(|c| c as usize)
        .or_else(|| params.get("_count").and_then(|s| s.parse::<usize>().ok()));
    let page = page_from_body
        .map(|p| p as usize)
        .or_else(|| params.get("_page").and_then(|s| s.parse::<usize>().ok()));

    if count.is_none() && page.is_none() {
        return Some(output);
    }

    match content_type {
        sof::ContentType::Json => {
            let output_str = String::from_utf8(output).ok()?;
            let mut records: Vec<serde_json::Value> = serde_json::from_str(&output_str).ok()?;

            // Apply pagination
            if let Some(count) = count {
                let page = page.unwrap_or(1);
                let offset = (page - 1) * count;
                if offset < records.len() {
                    let end = std::cmp::min(offset + count, records.len());
                    records = records[offset..end].to_vec();
                } else {
                    records.clear();
                }
            }

            serde_json::to_string(&records).ok().map(|s| s.into_bytes())
        }
        sof::ContentType::NdJson => {
            let output_str = String::from_utf8(output).ok()?;
            let mut lines: Vec<&str> = output_str.lines().collect();

            // Apply pagination
            if let Some(count) = count {
                let page = page.unwrap_or(1);
                let offset = (page - 1) * count;
                if offset < lines.len() {
                    let end = std::cmp::min(offset + count, lines.len());
                    lines = lines[offset..end].to_vec();
                } else {
                    lines.clear();
                }
            }

            Some(lines.join("\n").into_bytes())
        }
        sof::ContentType::Csv | sof::ContentType::CsvWithHeader => {
            let output_str = match String::from_utf8(output) {
                Ok(s) => s,
                Err(e) => return Some(e.into_bytes()),
            };
            let lines: Vec<&str> = output_str.lines().collect();

            if lines.is_empty() {
                return Some(output_str.into_bytes());
            }

            let has_header = matches!(content_type, sof::ContentType::CsvWithHeader);
            let header_offset = if has_header { 1 } else { 0 };

            if lines.len() <= header_offset {
                return Some(output_str.into_bytes());
            }

            let (header_lines, mut data_lines) = if has_header {
                (vec![lines[0]], lines[1..].to_vec())
            } else {
                (vec![], lines.to_vec())
            };

            // Apply pagination to data lines
            if let Some(count) = count {
                let page = page.unwrap_or(1);
                let offset = (page - 1) * count;
                if offset < data_lines.len() {
                    let end = std::cmp::min(offset + count, data_lines.len());
                    data_lines = data_lines[offset..end].to_vec();
                } else {
                    data_lines.clear();
                }
            }

            let mut result_lines = header_lines;
            result_lines.extend(data_lines);
            let result = result_lines.join("\n");

            // Add final newline if original had one
            if output_str.ends_with('\n') && !result.ends_with('\n') {
                Some(format!("{}\n", result).into_bytes())
            } else {
                Some(result.into_bytes())
            }
        }
        _ => Some(output),
    }
}
