//! Common test utilities for server tests

use axum::{Router, Json};
use axum::response::IntoResponse;
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
        .route("/ViewDefinition/$run", post(run_view_definition_handler))
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
    ).into_response()
}

async fn run_view_definition_handler(
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
    headers: axum::http::HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> axum::response::Response {
    // This would use the actual handler in production
    // For now, we'll implement a basic version for testing
    
    use sof::{run_view_definition, ContentType, SofBundle, SofViewDefinition};
    
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
    
    if let Some(parameters) = body["parameter"].as_array() {
        for param in parameters {
            match param["name"].as_str() {
                Some("viewResource") => {
                    view_def_json = param["resource"].as_object().cloned();
                }
                Some("patient") => {
                    if let Some(resource) = param["resource"].as_object() {
                        resources.push(serde_json::Value::Object(resource.clone()));
                    }
                }
                _ => {}
            }
        }
    }
    
    let view_def_json = match view_def_json {
        Some(v) => serde_json::Value::Object(v),
        None => return error_response(
            axum::http::StatusCode::BAD_REQUEST,
            "No ViewDefinition provided",
        ),
    };
    
    // Parse content type
    let format = params.get("_format");
    let accept = headers.get(axum::http::header::ACCEPT).and_then(|h| h.to_str().ok());
    let header_param = params.get("_header");
    
    let content_type = match parse_content_type(accept, format.map(|s| s.as_str()), header_param.map(|s| s.as_str())) {
        Ok(ct) => ct,
        Err(e) => return error_response(
            axum::http::StatusCode::UNSUPPORTED_MEDIA_TYPE,
            &e,
        ),
    };
    
    // Create ViewDefinition and Bundle
    let view_definition = match serde_json::from_value::<fhir::r4::ViewDefinition>(view_def_json) {
        Ok(vd) => SofViewDefinition::R4(vd),
        Err(e) => return error_response(
            axum::http::StatusCode::BAD_REQUEST,
            &format!("Invalid ViewDefinition: {}", e),
        ),
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
        Err(e) => return error_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            &format!("Failed to create Bundle: {}", e),
        ),
    };
    
    // Execute ViewDefinition
    match run_view_definition(view_definition, bundle, content_type) {
        Ok(output) => {
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
            ).into_response()
        }
        Err(e) => error_response(
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            &e.to_string(),
        ),
    }
}

fn parse_content_type(
    accept: Option<&str>,
    format: Option<&str>,
    header: Option<&str>,
) -> Result<sof::ContentType, String> {
    use sof::ContentType;
    
    let content_type_str = format.or(accept).unwrap_or("application/json");
    
    let content_type_str = if content_type_str == "text/csv" {
        match header {
            Some("absent") => "text/csv",
            Some("present") | None => "text/csv;header=present",
            _ => return Err(format!("Invalid _header parameter: {}", header.unwrap())),
        }
    } else {
        content_type_str
    };
    
    ContentType::from_string(content_type_str).map_err(|e| e.to_string())
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