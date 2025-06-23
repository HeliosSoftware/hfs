//! Request handlers for the SQL-on-FHIR server
//!
//! This module implements the HTTP request handlers for all server endpoints,
//! including the CapabilityStatement and ViewDefinition/$run operations.

use axum::{
    Json,
    extract::Query,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use sof::{ContentType, SofBundle, SofViewDefinition, run_view_definition};
use tracing::{debug, info};

use super::{
    error::{ServerError, ServerResult},
    models::{RunParameters, RunQueryParams, extract_run_parameters, parse_content_type},
};

/// Handler for GET /metadata - returns the server's CapabilityStatement
pub async fn capability_statement() -> ServerResult<impl IntoResponse> {
    info!("Handling CapabilityStatement request");

    let capability_statement = create_capability_statement();

    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/fhir+json")],
        Json(capability_statement),
    ))
}

/// Handler for POST /ViewDefinition/$run - executes a ViewDefinition
pub async fn run_view_definition_handler(
    Query(params): Query<RunQueryParams>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> ServerResult<Response> {
    info!("Handling ViewDefinition/$run request");
    debug!("Query params: {:?}", params);

    // Parse the Parameters resource
    let parameters: RunParameters = serde_json::from_value(body)
        .map_err(|e| ServerError::BadRequest(format!("Invalid Parameters resource: {}", e)))?;

    if parameters.resource_type != "Parameters" {
        return Err(ServerError::BadRequest(
            "Request body must be a Parameters resource".to_string(),
        ));
    }

    // Extract ViewDefinition and resources
    let (view_def_json, resources_json) =
        extract_run_parameters(parameters).map_err(|e| ServerError::BadRequest(e))?;

    let view_def_json = view_def_json
        .ok_or_else(|| ServerError::BadRequest("No ViewDefinition provided".to_string()))?;

    // Parse content type from Accept header and query parameters
    let accept_header = headers.get(header::ACCEPT).and_then(|h| h.to_str().ok());

    let content_type = parse_content_type(
        accept_header,
        params.format.as_deref(),
        params.header.as_deref(),
    )
    .map_err(|e| ServerError::UnsupportedMediaType(e))?;

    // Create ViewDefinition
    let view_definition = parse_view_definition(view_def_json)?;

    // Create Bundle from resources
    let bundle = create_bundle_from_resources(resources_json.unwrap_or_default())?;

    // Execute the ViewDefinition
    info!(
        "Executing ViewDefinition with output format: {:?}",
        content_type
    );
    let output = run_view_definition(view_definition, bundle, content_type)?;

    // Determine the MIME type for the response
    let mime_type = match content_type {
        ContentType::Csv | ContentType::CsvWithHeader => "text/csv",
        ContentType::Json => "application/json",
        ContentType::NdJson => "application/ndjson",
        ContentType::Parquet => "application/parquet",
    };

    Ok((StatusCode::OK, [(header::CONTENT_TYPE, mime_type)], output).into_response())
}

/// Create the server's CapabilityStatement
fn create_capability_statement() -> serde_json::Value {
    serde_json::json!({
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
    })
}

/// Parse a ViewDefinition from JSON
fn parse_view_definition(json: serde_json::Value) -> ServerResult<SofViewDefinition> {
    // For now, we only support R4
    #[cfg(feature = "R4")]
    {
        let view_def: fhir::r4::ViewDefinition = serde_json::from_value(json)
            .map_err(|e| ServerError::BadRequest(format!("Invalid ViewDefinition: {}", e)))?;
        Ok(SofViewDefinition::R4(view_def))
    }

    #[cfg(not(feature = "R4"))]
    {
        Err(ServerError::InternalError(
            "R4 feature not enabled".to_string(),
        ))
    }
}

/// Create a Bundle from a list of resources
fn create_bundle_from_resources(resources: Vec<serde_json::Value>) -> ServerResult<SofBundle> {
    #[cfg(feature = "R4")]
    {
        let bundle_json = serde_json::json!({
            "resourceType": "Bundle",
            "type": "collection",
            "entry": resources.into_iter().map(|resource| {
                serde_json::json!({
                    "resource": resource
                })
            }).collect::<Vec<_>>()
        });

        let bundle: fhir::r4::Bundle = serde_json::from_value(bundle_json)
            .map_err(|e| ServerError::InternalError(format!("Failed to create Bundle: {}", e)))?;

        Ok(SofBundle::R4(bundle))
    }

    #[cfg(not(feature = "R4"))]
    {
        Err(ServerError::InternalError(
            "R4 feature not enabled".to_string(),
        ))
    }
}

/// Simple health check endpoint
pub async fn health_check() -> impl IntoResponse {
    info!("Handling Health Check request");
    Json(serde_json::json!({
        "status": "ok",
        "service": "sof-server",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capability_statement_structure() {
        let cap_stmt = create_capability_statement();

        assert_eq!(cap_stmt["resourceType"], "CapabilityStatement");
        assert_eq!(cap_stmt["kind"], "instance");
        assert_eq!(cap_stmt["fhirVersion"], "4.0.1");

        // Check that ViewDefinition resource is listed
        let resources = &cap_stmt["rest"][0]["resource"];
        assert!(
            resources
                .as_array()
                .unwrap()
                .iter()
                .any(|r| { r["type"] == "ViewDefinition" })
        );
    }
}

