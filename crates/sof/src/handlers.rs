//! Request handlers for the SQL-on-FHIR server
//!
//! This module implements the HTTP request handlers for all server endpoints,
//! including the CapabilityStatement and ViewDefinition/$run operations.

use axum::{
    Json,
    extract::{Query, Path},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use sof::{ContentType, SofBundle, SofViewDefinition, run_view_definition, get_newest_enabled_fhir_version, get_fhir_version_string};
use tracing::{debug, info};

use super::{
    error::{ServerError, ServerResult},
    models::{RunParameters, RunQueryParams, extract_run_parameters, validate_query_params, apply_result_filtering, parse_content_type},
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
///
/// This is the main endpoint for executing ViewDefinition transformations.
///
/// # Arguments
/// * `params` - Query parameters for filtering, pagination, and output format
/// * `headers` - HTTP headers including Accept for content negotiation
/// * `body` - FHIR Parameters resource containing ViewDefinition and resources
///
/// # Parameters Resource
/// The body should contain a FHIR Parameters resource with:
/// * `viewResource` - The ViewDefinition to execute (inline)
/// * `viewReference` - Reference to a ViewDefinition (not yet supported)
/// * `resource` - FHIR resources to transform (can be repeated)
/// * `_format` or `format` - Output format as valueCode or valueString (overrides query params and Accept header)
/// * `_header` or `header` - CSV header control as valueBoolean (true/false, overrides query params)
///
/// # Query Parameters
/// * `_format` - Output format (json/csv/ndjson), overrides Accept header
/// * `_header` - CSV header control (true/false, defaults to true for CSV)
/// * `_count` - Limit results (1-10000)
/// * `_page` - Page number for pagination (1-based)
/// * `_since` - Filter by modification time (RFC3339 format)
///
/// # Returns
/// * `Ok(Response)` - Transformed and filtered data in requested format
/// * `Err(ServerError)` - Various errors for invalid input or processing failures
pub async fn run_view_definition_handler(
    Query(params): Query<RunQueryParams>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> ServerResult<Response> {
    info!("Handling ViewDefinition/$run request");
    debug!("Query params: {:?}", params);

    // Validate and parse query parameters
    let accept_header = headers.get(header::ACCEPT).and_then(|h| h.to_str().ok());
    let validated_params = validate_query_params(&params, accept_header)
        .map_err(|e| ServerError::BadRequest(e))?;

    // Parse the Parameters resource using version detection
    let parameters = parse_parameters(body)?;

    // Extract ViewDefinition and resources
    let (view_def_json, resources_json, format_from_body, header_from_body) =
        extract_run_parameters(parameters).map_err(|e| ServerError::BadRequest(e))?;

    let view_def_json = view_def_json
        .ok_or_else(|| ServerError::BadRequest("No ViewDefinition provided".to_string()))?;

    // If format is provided in body, update the validated params
    let mut validated_params = validated_params;
    if let Some(format_str) = format_from_body {
        // The _format parameter in body overrides query parameter and Accept header
        // Also check if header was provided in body
        let header_param = if let Some(h) = header_from_body {
            Some(h)
        } else {
            // Convert query parameter header to boolean
            match params.header.as_deref() {
                Some("true") => Some(true),
                Some("false") => Some(false),
                _ => None,
            }
        };
        let content_type = parse_content_type(
            None, // Ignore Accept header when body param is present
            Some(&format_str),
            header_param,
        )?;
        validated_params.format = content_type;
    } else if let Some(header_bool) = header_from_body {
        // If only header is provided in body, update the format accordingly
        let format_str = match validated_params.format {
            ContentType::Csv | ContentType::CsvWithHeader => "text/csv",
            _ => return Err(ServerError::BadRequest("Header parameter only applies to CSV format".to_string())),
        };
        let content_type = parse_content_type(
            None,
            Some(format_str),
            Some(header_bool),
        )?;
        validated_params.format = content_type;
    }

    // Create ViewDefinition
    let view_definition = parse_view_definition(view_def_json)?;

    // Create Bundle from resources
    let bundle = create_bundle_from_resources(resources_json.unwrap_or_default())?;

    // Execute the ViewDefinition
    info!(
        "Executing ViewDefinition with output format: {:?}",
        validated_params.format
    );
    let output = run_view_definition(view_definition, bundle, validated_params.format)?;

    // Apply query parameter filtering
    let filtered_output = apply_result_filtering(output, &validated_params)
        .map_err(|e| ServerError::InternalError(format!("Failed to apply filtering: {}", e)))?;

    // Determine the MIME type for the response
    let mime_type = match validated_params.format {
        ContentType::Csv | ContentType::CsvWithHeader => "text/csv",
        ContentType::Json => "application/json",
        ContentType::NdJson => "application/ndjson",
        ContentType::Parquet => "application/parquet",
    };

    Ok((StatusCode::OK, [(header::CONTENT_TYPE, mime_type)], filtered_output).into_response())
}

/// Handler for GET /ViewDefinition/$run - executes a ViewDefinition via query parameters
///
/// This endpoint allows executing a ViewDefinition by providing a viewReference query parameter.
/// 
/// # Arguments
/// * `params` - Query parameters including viewReference and other options
/// * `headers` - HTTP headers including Accept for content negotiation
///
/// # Query Parameters
/// * `viewReference` - Reference to the ViewDefinition to execute (required)
/// * `_format` - Output format (json/csv/ndjson), overrides Accept header
/// * `_header` - CSV header control (true/false, defaults to true for CSV)
/// * `_count` - Limit results (1-10000)
/// * `_page` - Page number for pagination (1-based)
/// * `_since` - Filter by modification time (RFC3339 format)
/// * `patient` - Filter resources by patient reference
/// * `group` - Filter resources by group reference
/// * `source` - Data source for transformation
///
/// # Returns
/// * `Ok(Response)` - Transformed data in requested format
/// * `Err(ServerError)` - Various errors for invalid input or processing failures
pub async fn run_view_definition_get_handler(
    Query(params): Query<RunQueryParams>,
    headers: HeaderMap,
) -> ServerResult<Response> {
    info!("Handling GET ViewDefinition/$run request");
    debug!("Query params: {:?}", params);

    // Validate and parse query parameters
    let accept_header = headers.get(header::ACCEPT).and_then(|h| h.to_str().ok());
    let validated_params = validate_query_params(&params, accept_header)
        .map_err(|e| ServerError::BadRequest(e))?;

    // Check for required viewReference parameter
    if validated_params.view_reference.is_none() {
        return Err(ServerError::BadRequest(
            "viewReference parameter is required for GET requests".to_string(),
        ));
    }

    // For now, return an error as viewReference is not yet implemented
    // In a real implementation, this would load the ViewDefinition using the reference
    Err(ServerError::NotImplemented(
        "viewReference parameter is not yet supported. Use POST /ViewDefinition/$run with the ViewDefinition in the request body.".to_string(),
    ))
}

/// Handler for GET /ViewDefinition/{id}/$run - executes a ViewDefinition by ID
///
/// This endpoint allows executing a stored ViewDefinition by its identifier.
/// 
/// # Arguments
/// * `view_definition_id` - The ID of the ViewDefinition to execute
/// * `params` - Query parameters for filtering and formatting
/// * `headers` - HTTP headers including Accept for content negotiation
///
/// # Returns
/// * `Ok(Response)` - Transformed data in requested format (when implemented)
/// * `Err(ServerError::NotImplemented)` - Currently not implemented
///
/// # Future Implementation
/// When ViewDefinition storage is implemented, this handler will:
/// 1. Retrieve the ViewDefinition from storage by ID
/// 2. Validate query parameters
/// 3. Execute the transformation with provided resources
/// 4. Apply filtering and pagination
/// 5. Return results in the requested format
pub async fn run_view_definition_by_id_handler(
    Path(view_definition_id): Path<String>,
    Query(params): Query<RunQueryParams>,
    headers: HeaderMap,
) -> ServerResult<Response> {
    info!("Handling ViewDefinition/{}/$run request", view_definition_id);
    debug!("Query params: {:?}", params);

    // Validate and parse query parameters
    let accept_header = headers.get(header::ACCEPT).and_then(|h| h.to_str().ok());
    let _validated_params = validate_query_params(&params, accept_header)
        .map_err(|e| ServerError::BadRequest(e))?;

    // For now, return an error as we don't have a ViewDefinition repository
    // In a real implementation, this would load the ViewDefinition from storage
    Err(ServerError::NotImplemented(
        format!("ViewDefinition lookup by ID '{}' is not implemented. Use POST /ViewDefinition/$run with the ViewDefinition in the request body.", view_definition_id)
    ))
}

/// Create the server's CapabilityStatement
fn create_capability_statement() -> serde_json::Value {
    // Get the FHIR version string dynamically based on enabled features
    let fhir_version = get_fhir_version_string();
    
    // Create a CapabilityStatement JSON that uses the correct FHIR version
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
        "fhirVersion": fhir_version,
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
    let newest_version = get_newest_enabled_fhir_version();
    
    match newest_version {
        #[cfg(feature = "R4")]
        fhir::FhirVersion::R4 => {
            let view_def: fhir::r4::ViewDefinition = serde_json::from_value(json)
                .map_err(|e| ServerError::BadRequest(format!("Invalid R4 ViewDefinition: {}", e)))?;
            Ok(SofViewDefinition::R4(view_def))
        }
        #[cfg(feature = "R4B")]
        fhir::FhirVersion::R4B => {
            let view_def: fhir::r4b::ViewDefinition = serde_json::from_value(json)
                .map_err(|e| ServerError::BadRequest(format!("Invalid R4B ViewDefinition: {}", e)))?;
            Ok(SofViewDefinition::R4B(view_def))
        }
        #[cfg(feature = "R5")]
        fhir::FhirVersion::R5 => {
            let view_def: fhir::r5::ViewDefinition = serde_json::from_value(json)
                .map_err(|e| ServerError::BadRequest(format!("Invalid R5 ViewDefinition: {}", e)))?;
            Ok(SofViewDefinition::R5(view_def))
        }
        #[cfg(feature = "R6")]
        fhir::FhirVersion::R6 => {
            let view_def: fhir::r6::ViewDefinition = serde_json::from_value(json)
                .map_err(|e| ServerError::BadRequest(format!("Invalid R6 ViewDefinition: {}", e)))?;
            Ok(SofViewDefinition::R6(view_def))
        }
    }
}

/// Parse a Parameters resource from JSON
fn parse_parameters(json: serde_json::Value) -> ServerResult<RunParameters> {
    // Validate that it's a Parameters resource
    if let Some(resource_type) = json.get("resourceType") {
        if resource_type != "Parameters" {
            return Err(ServerError::BadRequest(
                "Request body must be a Parameters resource".to_string(),
            ));
        }
    } else {
        return Err(ServerError::BadRequest(
            "Missing resourceType field".to_string(),
        ));
    }
    
    let newest_version = get_newest_enabled_fhir_version();
    
    match newest_version {
        #[cfg(feature = "R4")]
        fhir::FhirVersion::R4 => {
            let params: fhir::r4::Parameters = serde_json::from_value(json)
                .map_err(|e| ServerError::BadRequest(format!("Invalid R4 Parameters: {}", e)))?;
            Ok(RunParameters::R4(params))
        }
        #[cfg(feature = "R4B")]
        fhir::FhirVersion::R4B => {
            let params: fhir::r4b::Parameters = serde_json::from_value(json)
                .map_err(|e| ServerError::BadRequest(format!("Invalid R4B Parameters: {}", e)))?;
            Ok(RunParameters::R4B(params))
        }
        #[cfg(feature = "R5")]
        fhir::FhirVersion::R5 => {
            let params: fhir::r5::Parameters = serde_json::from_value(json)
                .map_err(|e| ServerError::BadRequest(format!("Invalid R5 Parameters: {}", e)))?;
            Ok(RunParameters::R5(params))
        }
        #[cfg(feature = "R6")]
        fhir::FhirVersion::R6 => {
            let params: fhir::r6::Parameters = serde_json::from_value(json)
                .map_err(|e| ServerError::BadRequest(format!("Invalid R6 Parameters: {}", e)))?;
            Ok(RunParameters::R6(params))
        }
    }
}

/// Create a Bundle from a list of resources
fn create_bundle_from_resources(resources: Vec<serde_json::Value>) -> ServerResult<SofBundle> {
    let newest_version = get_newest_enabled_fhir_version();
    
    let bundle_json = serde_json::json!({
        "resourceType": "Bundle",
        "type": "collection",
        "entry": resources.into_iter().map(|resource| {
            serde_json::json!({
                "resource": resource
            })
        }).collect::<Vec<_>>()
    });
    
    match newest_version {
        #[cfg(feature = "R4")]
        fhir::FhirVersion::R4 => {
            let bundle: fhir::r4::Bundle = serde_json::from_value(bundle_json)
                .map_err(|e| ServerError::InternalError(format!("Failed to create R4 Bundle: {}", e)))?;
            Ok(SofBundle::R4(bundle))
        }
        #[cfg(feature = "R4B")]
        fhir::FhirVersion::R4B => {
            let bundle: fhir::r4b::Bundle = serde_json::from_value(bundle_json)
                .map_err(|e| ServerError::InternalError(format!("Failed to create R4B Bundle: {}", e)))?;
            Ok(SofBundle::R4B(bundle))
        }
        #[cfg(feature = "R5")]
        fhir::FhirVersion::R5 => {
            let bundle: fhir::r5::Bundle = serde_json::from_value(bundle_json)
                .map_err(|e| ServerError::InternalError(format!("Failed to create R5 Bundle: {}", e)))?;
            Ok(SofBundle::R5(bundle))
        }
        #[cfg(feature = "R6")]
        fhir::FhirVersion::R6 => {
            let bundle: fhir::r6::Bundle = serde_json::from_value(bundle_json)
                .map_err(|e| ServerError::InternalError(format!("Failed to create R6 Bundle: {}", e)))?;
            Ok(SofBundle::R6(bundle))
        }
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
        assert_eq!(cap_stmt["fhirVersion"], get_fhir_version_string());

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

