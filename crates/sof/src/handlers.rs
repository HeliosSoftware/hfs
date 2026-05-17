//! Request handlers for the SQL-on-FHIR server
//!
//! This module implements the HTTP request handlers for all server endpoints,
//! including the CapabilityStatement and ViewDefinition/$viewdefinition-run operations.

use axum::{
    Json,
    extract::Query,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use helios_sof::{
    ContentType, RunOptions, SofBundle, SofError, SofViewDefinition,
    create_bundle_from_resources_for_version as sof_create_bundle_from_resources_for_version,
    data_source::{DataSource, UniversalDataSource},
    filter_resources_by_patient_and_group as sof_filter_resources_by_patient_and_group,
    filter_resources_by_since as sof_filter_resources_by_since, format_parquet_multi_file,
    get_fhir_version_string, get_newest_enabled_fhir_version,
    parse_view_definition_for_version as sof_parse_view_definition_for_version,
    process_view_definition, run_view_definition_with_options,
};
use tracing::{debug, info};

use super::{
    error::{ServerError, ServerResult},
    models::{
        RunParameters, RunQueryParams, apply_result_filtering, extract_all_parameters,
        parse_content_type, validate_query_params,
    },
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

/// Handler for POST /ViewDefinition/$viewdefinition-run - executes a ViewDefinition
///
/// The `$viewdefinition-run` operation on a ViewDefinition resource applies the view definition to
/// transform FHIR resources into a tabular format and returns the results synchronously.
///
/// # Arguments
/// * `params` - Query parameters for filtering, pagination, and output format
/// * `headers` - HTTP headers including Accept for content negotiation
/// * `body` - FHIR Parameters resource containing ViewDefinition and resources
///
/// # Parameters (in specification order)
///
/// Parameters can be provided as query parameters or in the request body (FHIR Parameters resource).
/// Parameters in request body take precedence over query parameters.
///
/// | Name | Type | Use | Scope | Min | Max | Documentation |
/// |------|------|-----|-------|-----|-----|---------------|
/// | _format | code | in | type, instance | 1 | 1 | Output format - `application/json`, `application/ndjson`, `text/csv`, `application/parquet` |
/// | header | boolean | in | type, instance | 0 | 1 | This parameter only applies to `text/csv` requests. `true` (default) - return headers in the response, `false` - do not return headers. |
/// | viewReference | Reference | in | type, instance | 0 | * | Reference(s) to ViewDefinition(s) to be used for data transformation. (not yet supported) |
/// | viewResource | ViewDefinition | in | type | 0 | * | ViewDefinition(s) to be used for data transformation. |
/// | patient | Reference | in | type, instance | 0 | * | Filter resources by patient. |
/// | group | Reference | in | type, instance | 0 | * | Filter resources by group. (not yet supported) |
/// | source | string | in | type, instance | 0 | 1 | If provided, the source of FHIR data to be transformed into a tabular projection. Supports file://, http(s)://, s3://, gs://, and azure:// URLs. |
/// | _limit | integer | in | type, instance | 0 | 1 | Limits the number of results. (1-10000) |
/// | _since | instant | in | type, instance | 0 | 1 | Return resources that have been modified after the supplied time. (RFC3339 format, validates format only) |
/// | resource | Resource | in | type, instance | 0 | * | Collection of FHIR resources to be transformed into a tabular projection. |
///
/// ## Query Parameters
/// All parameters except `viewReference`, `viewResource`, `patient`, `group`, and `resource` can be provided as POST query parameters
///
/// # Returns
/// * `Ok(Response)` - The output of the operation is in the requested format, defined by the format parameter or accept header
/// * `Err(ServerError)` - Various errors for invalid input or processing failures
pub async fn run_view_definition_handler(
    Query(params): Query<RunQueryParams>,
    headers: HeaderMap,
    body: Option<Json<serde_json::Value>>,
) -> ServerResult<Response> {
    info!("Handling ViewDefinition/$viewdefinition-run request");
    debug!("Query params: {:?}", params);

    // Enforce spec cardinality `_format = 1..1`: at least one of `_format`
    // (query or body) or a usable `Accept` header must be present. Body is
    // inspected permissively here so we can fail fast before parsing the
    // typed Parameters resource.
    let accept_header = headers.get(header::ACCEPT).and_then(|h| h.to_str().ok());
    let body_has_format = body
        .as_ref()
        .is_some_and(|Json(b)| helios_sof::extract_run_params_from_json(b).format.is_some());
    if params.format.is_none() && accept_header.is_none() && !body_has_format {
        return Err(ServerError::BadRequest(
            "_format is required (or provide an Accept header with a supported MIME type); \
             supported formats: json, ndjson, csv, parquet"
                .to_string(),
        ));
    }

    // GET / bodyless requests can't carry viewResource or resource. With no
    // body to extract a ViewDefinition from and no viewReference support
    // (sof-server is stateless), we reject early with a 400.
    let Some(Json(body)) = body else {
        return Err(ServerError::BadRequest(
            "GET /ViewDefinition/$viewdefinition-run requires a 'viewReference' to be supported \
             by the server; this stateless server does not resolve viewReference. Use POST \
             with viewResource in a Parameters body instead."
                .to_string(),
        ));
    };

    // Validate and parse query parameters
    let validated_params =
        validate_query_params(&params, accept_header).map_err(ServerError::BadRequest)?;

    // Parse the Parameters resource using version detection
    let parameters = parse_parameters(body)?;

    // Extract all parameters including filters
    let extracted_params = extract_all_parameters(parameters).map_err(ServerError::BadRequest)?;

    // Check for not-yet-implemented parameters
    if extracted_params.view_reference.is_some() {
        return Err(ServerError::NotImplemented(
            "The viewReference parameter is not yet implemented. Please provide the ViewDefinition directly using the viewResource parameter.".to_string()
        ));
    }

    if extracted_params.group.is_some() {
        return Err(ServerError::NotImplemented(
            "The group parameter is not yet implemented.".to_string(),
        ));
    }

    // For backward compatibility, extract the legacy tuple format
    let view_def_json = extracted_params.view_definition;
    let resources_json = if extracted_params.resources.is_empty() {
        None
    } else {
        Some(extracted_params.resources)
    };
    let format_from_body = extracted_params.format;
    let header_from_body = extracted_params.header;

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
            _ => {
                return Err(ServerError::BadRequest(
                    "Header parameter only applies to CSV format".to_string(),
                ));
            }
        };
        let content_type = parse_content_type(None, Some(format_str), Some(header_bool))?;
        validated_params.format = content_type;
    }

    // Apply patient and group filters from body parameters to resources if provided
    let mut filtered_resources = resources_json.unwrap_or_default();

    // Merge filter parameters from body and query
    let patient_filter = extracted_params
        .patient
        .or(validated_params.patient.clone());
    let group_filter = extracted_params.group.or(validated_params.group.clone());
    let source_param = extracted_params.source.or(validated_params.source.clone());

    // Merge limit parameter - body takes precedence over query
    if let Some(limit) = extracted_params.limit {
        validated_params.limit = Some(limit as usize);
    }

    // Merge _since parameter - body takes precedence over query
    if let Some(since_str) = extracted_params.since {
        // Parse and validate the timestamp
        match DateTime::parse_from_rfc3339(&since_str) {
            Ok(dt) => validated_params.since = Some(dt.with_timezone(&Utc)),
            Err(_) => {
                return Err(ServerError::BadRequest(format!(
                    "_since parameter must be a valid RFC3339 timestamp: {}",
                    since_str
                )));
            }
        }
    }

    // Merge Parquet options - body parameters take precedence over query parameters
    if extracted_params.max_file_size.is_some()
        || extracted_params.row_group_size.is_some()
        || extracted_params.page_size.is_some()
        || extracted_params.compression.is_some()
    {
        // Create or update Parquet options from body parameters
        let mut parquet_opts = validated_params.parquet_options.clone().unwrap_or_else(|| {
            helios_sof::ParquetOptions {
                row_group_size_mb: 256,
                page_size_kb: 1024,
                compression: "snappy".to_string(),
                max_file_size_mb: None,
            }
        });

        if let Some(max_size) = extracted_params.max_file_size {
            parquet_opts.max_file_size_mb = Some(max_size);
        }
        if let Some(row_group) = extracted_params.row_group_size {
            parquet_opts.row_group_size_mb = row_group;
        }
        if let Some(page_size) = extracted_params.page_size {
            parquet_opts.page_size_kb = page_size;
        }
        if let Some(compression) = extracted_params.compression {
            parquet_opts.compression = compression;
        }

        validated_params.parquet_options = Some(parquet_opts);
    }

    // Handle source parameter - load data from external source if provided
    // IMPORTANT: We load the source bundle FIRST so we can determine its FHIR version
    // and parse the ViewDefinition using the same version
    let mut source_bundle = None;
    let mut source_fhir_version = None;
    if let Some(source) = &source_param {
        info!("Loading data from source: {}", source);
        let data_source = UniversalDataSource::new();
        let loaded_bundle = data_source.load(source).await?;

        // Capture the FHIR version from the loaded source bundle
        source_fhir_version = Some(loaded_bundle.version());

        // Apply filters to source bundle if needed
        let loaded_bundle = if patient_filter.is_some()
            || group_filter.is_some()
            || validated_params.since.is_some()
        {
            // Extract resources from source bundle for filtering
            let mut source_resources = extract_resources_from_bundle(&loaded_bundle)?;

            // Apply filters
            if patient_filter.is_some() || group_filter.is_some() {
                source_resources = filter_resources_by_patient_and_group(
                    source_resources,
                    patient_filter.as_deref(),
                    group_filter.as_deref(),
                )?;
            }

            if let Some(since) = validated_params.since {
                source_resources = filter_resources_by_since(source_resources, since)?;
            }

            // Recreate bundle with filtered resources using the same FHIR version
            create_bundle_from_resources_for_version(
                source_resources,
                source_fhir_version.unwrap(),
            )?
        } else {
            loaded_bundle
        };

        source_bundle = Some(loaded_bundle);
    }

    // Create ViewDefinition - use the source bundle's version if available,
    // otherwise use the default (newest enabled) version
    let view_definition = if let Some(version) = source_fhir_version {
        info!(
            "Parsing ViewDefinition as {:?} (matching source bundle)",
            version
        );
        parse_view_definition_for_version(view_def_json, version)?
    } else {
        parse_view_definition(view_def_json)?
    };

    // Apply filters to provided resources
    if patient_filter.is_some() || group_filter.is_some() {
        filtered_resources = filter_resources_by_patient_and_group(
            filtered_resources,
            patient_filter.as_deref(),
            group_filter.as_deref(),
        )?;
    }

    // Apply _since filter if provided
    if let Some(since) = validated_params.since {
        filtered_resources = filter_resources_by_since(filtered_resources, since)?;
    }

    // Create Bundle from resources, merging source bundle if provided
    let bundle = if let Some(source_bundle) = source_bundle {
        // If we have a source bundle, merge it with any resources from the request
        if filtered_resources.is_empty() {
            // Only source data, use it directly
            source_bundle
        } else {
            // Merge source bundle with provided resources
            merge_bundles(source_bundle, filtered_resources)?
        }
    } else {
        // No source, create bundle from provided resources
        create_bundle_from_resources(filtered_resources)?
    };

    // Build RunOptions from validated parameters
    let run_options = RunOptions {
        since: validated_params.since,
        limit: validated_params.limit,
        page: None, // Pagination not supported via query params yet
        parquet_options: validated_params.parquet_options.clone(),
    };

    // Execute the ViewDefinition
    info!(
        "Executing ViewDefinition with output format: {:?}",
        validated_params.format
    );

    // Check if we need to handle multi-file Parquet output
    if validated_params.format == ContentType::Parquet
        && validated_params
            .parquet_options
            .as_ref()
            .and_then(|opts| opts.max_file_size_mb)
            .is_some()
    {
        // Use multi-file Parquet generation
        let processed_result = process_view_definition(view_definition, bundle)?;

        // Get max file size in bytes
        let max_file_size_bytes = validated_params
            .parquet_options
            .as_ref()
            .and_then(|opts| opts.max_file_size_mb)
            .map(|mb| mb as usize * 1024 * 1024)
            .unwrap_or(usize::MAX);

        let file_buffers = format_parquet_multi_file(
            processed_result,
            validated_params.parquet_options.as_ref(),
            max_file_size_bytes,
        )?;

        // If multiple files, stream them as a ZIP archive
        if file_buffers.len() > 1 {
            info!(
                "Generating ZIP archive with {} Parquet files",
                file_buffers.len()
            );
            crate::streaming::stream_parquet_zip_response(file_buffers, "data")
        } else {
            // Single file - check if we should stream it
            let file_size = file_buffers[0].len();
            if crate::streaming::should_use_streaming(file_size) {
                info!("Streaming single Parquet file ({} bytes)", file_size);
                crate::streaming::stream_single_parquet_response(file_buffers[0].clone())
            } else {
                // Small file, return directly
                Ok((
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "application/parquet")],
                    file_buffers[0].clone(),
                )
                    .into_response())
            }
        }
    } else {
        // Standard processing
        let output = run_view_definition_with_options(
            view_definition,
            bundle,
            validated_params.format,
            run_options,
        )?;

        // Apply any additional filtering (already applied in run_view_definition_with_options, but kept for compatibility)
        let filtered_output = apply_result_filtering(output, &validated_params)
            .map_err(|e| ServerError::InternalError(format!("Failed to apply filtering: {}", e)))?;

        // Determine the MIME type for the response
        let mime_type = match validated_params.format {
            ContentType::Csv | ContentType::CsvWithHeader => "text/csv",
            ContentType::Json => "application/json",
            ContentType::NdJson => "application/x-ndjson",
            ContentType::Parquet => "application/parquet",
        };

        Ok((
            StatusCode::OK,
            [(header::CONTENT_TYPE, mime_type)],
            filtered_output,
        )
            .into_response())
    }
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
        "format": ["json"],
        "rest": [{
            "mode": "server",
            "operation": [{
                "name": "viewdefinition-run",
                "definition": "http://sql-on-fhir.org/OperationDefinition/$viewdefinition-run",
                "documentation": "Execute a ViewDefinition to transform FHIR resources into tabular format. Supports CSV, JSON, and NDJSON output formats. This is a type-level operation invoked at /ViewDefinition/$viewdefinition-run"
            }]
        }]
    })
}

/// Resolve a ViewDefinition from a reference
///
/// This function implements the reference resolution algorithm described in the
/// SQL-on-FHIR specification for the viewReference parameter:
///
/// 1. If the reference is a relative URL, resolve it on the server side
/// 2. If the reference is an absolute URL with a canonical URL, look up in artifact registry
/// 3. Otherwise, try to load the ViewDefinition from the provided absolute URL
///
/// # Arguments
/// * `reference` - The reference string (e.g., "ViewDefinition/123", canonical URL, or absolute URL)
///
/// # Returns
/// * `Ok(SofViewDefinition)` - Successfully resolved ViewDefinition
/// * `Err(ServerError)` - Resolution failed
#[allow(dead_code)]
fn resolve_view_reference(reference: &str) -> ServerResult<SofViewDefinition> {
    info!("Resolving ViewDefinition reference: {}", reference);

    // Check if it's a relative reference (e.g., "ViewDefinition/123")
    if !reference.starts_with("http://") && !reference.starts_with("https://") {
        // This would be a server-relative reference
        // Since we're stateless, we can't resolve this
        return Err(ServerError::NotImplemented(format!(
            "Relative ViewDefinition references are not supported in this stateless implementation: {}",
            reference
        )));
    }

    // Check if it's a canonical URL (contains |version)
    if reference.contains('|') {
        // This would require an artifact registry lookup
        return Err(ServerError::NotImplemented(format!(
            "Canonical URL references with versions are not yet supported: {}",
            reference
        )));
    }

    // Try to load from absolute URL
    // For now, we don't support loading from external URLs
    Err(ServerError::NotImplemented(format!(
        "Loading ViewDefinitions from external URLs is not yet implemented: {}",
        reference
    )))
}

/// Parse a ViewDefinition from JSON using the newest enabled FHIR version
fn parse_view_definition(json: serde_json::Value) -> ServerResult<SofViewDefinition> {
    parse_view_definition_for_version(json, get_newest_enabled_fhir_version())
}

/// Parse a ViewDefinition from JSON using a specific FHIR version
fn parse_view_definition_for_version(
    json: serde_json::Value,
    version: helios_fhir::FhirVersion,
) -> ServerResult<SofViewDefinition> {
    sof_parse_view_definition_for_version(json, version).map_err(|e| match e {
        SofError::InvalidViewDefinition(msg) => ServerError::BadRequest(msg),
        other => ServerError::from(other),
    })
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
        helios_fhir::FhirVersion::R4 => {
            let params: helios_fhir::r4::Parameters = serde_json::from_value(json)
                .map_err(|e| ServerError::BadRequest(format!("Invalid R4 Parameters: {}", e)))?;
            Ok(RunParameters::R4(params))
        }
        #[cfg(feature = "R4B")]
        helios_fhir::FhirVersion::R4B => {
            let params: helios_fhir::r4b::Parameters = serde_json::from_value(json)
                .map_err(|e| ServerError::BadRequest(format!("Invalid R4B Parameters: {}", e)))?;
            Ok(RunParameters::R4B(params))
        }
        #[cfg(feature = "R5")]
        helios_fhir::FhirVersion::R5 => {
            let params: helios_fhir::r5::Parameters = serde_json::from_value(json)
                .map_err(|e| ServerError::BadRequest(format!("Invalid R5 Parameters: {}", e)))?;
            Ok(RunParameters::R5(params))
        }
        #[cfg(feature = "R6")]
        helios_fhir::FhirVersion::R6 => {
            let params: helios_fhir::r6::Parameters = serde_json::from_value(json)
                .map_err(|e| ServerError::BadRequest(format!("Invalid R6 Parameters: {}", e)))?;
            Ok(RunParameters::R6(params))
        }
    }
}

/// Create a Bundle from a list of resources using the newest enabled FHIR version
fn create_bundle_from_resources(resources: Vec<serde_json::Value>) -> ServerResult<SofBundle> {
    create_bundle_from_resources_for_version(resources, get_newest_enabled_fhir_version())
}

/// Create a Bundle from a list of resources using a specific FHIR version
fn create_bundle_from_resources_for_version(
    resources: Vec<serde_json::Value>,
    version: helios_fhir::FhirVersion,
) -> ServerResult<SofBundle> {
    sof_create_bundle_from_resources_for_version(resources, version).map_err(|e| match e {
        SofError::InvalidViewDefinition(msg) => ServerError::InternalError(msg),
        other => ServerError::from(other),
    })
}

/// Extract resources from a bundle as JSON values
fn extract_resources_from_bundle(bundle: &SofBundle) -> ServerResult<Vec<serde_json::Value>> {
    let mut resources = Vec::new();

    match bundle {
        #[cfg(feature = "R4")]
        SofBundle::R4(bundle) => {
            if let Some(entries) = &bundle.entry {
                for entry in entries {
                    if let Some(resource) = &entry.resource {
                        resources.push(serde_json::to_value(resource)?);
                    }
                }
            }
        }
        #[cfg(feature = "R4B")]
        SofBundle::R4B(bundle) => {
            if let Some(entries) = &bundle.entry {
                for entry in entries {
                    if let Some(resource) = &entry.resource {
                        resources.push(serde_json::to_value(resource)?);
                    }
                }
            }
        }
        #[cfg(feature = "R5")]
        SofBundle::R5(bundle) => {
            if let Some(entries) = &bundle.entry {
                for entry in entries {
                    if let Some(resource) = &entry.resource {
                        resources.push(serde_json::to_value(resource)?);
                    }
                }
            }
        }
        #[cfg(feature = "R6")]
        SofBundle::R6(bundle) => {
            if let Some(entries) = &bundle.entry {
                for entry in entries {
                    if let Some(resource) = &entry.resource {
                        resources.push(serde_json::to_value(resource)?);
                    }
                }
            }
        }
    }

    Ok(resources)
}

/// Merge a source bundle with additional resources
fn merge_bundles(
    source_bundle: SofBundle,
    additional_resources: Vec<serde_json::Value>,
) -> ServerResult<SofBundle> {
    // First, extract all resources from the source bundle
    let mut all_resources = Vec::new();

    match source_bundle {
        #[cfg(feature = "R4")]
        SofBundle::R4(bundle) => {
            if let Some(entries) = bundle.entry {
                for entry in entries {
                    if let Some(resource) = entry.resource {
                        all_resources.push(serde_json::to_value(&resource)?);
                    }
                }
            }
        }
        #[cfg(feature = "R4B")]
        SofBundle::R4B(bundle) => {
            if let Some(entries) = bundle.entry {
                for entry in entries {
                    if let Some(resource) = entry.resource {
                        all_resources.push(serde_json::to_value(&resource)?);
                    }
                }
            }
        }
        #[cfg(feature = "R5")]
        SofBundle::R5(bundle) => {
            if let Some(entries) = bundle.entry {
                for entry in entries {
                    if let Some(resource) = entry.resource {
                        all_resources.push(serde_json::to_value(&resource)?);
                    }
                }
            }
        }
        #[cfg(feature = "R6")]
        SofBundle::R6(bundle) => {
            if let Some(entries) = bundle.entry {
                for entry in entries {
                    if let Some(resource) = entry.resource {
                        all_resources.push(serde_json::to_value(&resource)?);
                    }
                }
            }
        }
    }

    // Add the additional resources
    all_resources.extend(additional_resources);

    // Create a new bundle with all resources
    create_bundle_from_resources(all_resources)
}

/// Filter resources by patient and/or group reference
///
/// This function implements the patient and group filtering as specified in the
/// SQL-on-FHIR $viewdefinition-run operation:
///
/// - **Patient filter**: Returns only resources in the patient compartment of specified patients
/// - **Group filter**: Returns only resources that are members of the specified group
///
/// # Arguments
/// * `resources` - List of FHIR resources to filter
/// * `patient_ref` - Optional patient reference (e.g., "Patient/123")
/// * `group_ref` - Optional group reference (e.g., "Group/456")
///
/// # Returns
/// * `Ok(Vec<serde_json::Value>)` - Filtered list of resources
/// * `Err(ServerError)` - If filtering fails
fn filter_resources_by_patient_and_group(
    resources: Vec<serde_json::Value>,
    patient_ref: Option<&str>,
    group_ref: Option<&str>,
) -> ServerResult<Vec<serde_json::Value>> {
    sof_filter_resources_by_patient_and_group(resources, patient_ref, group_ref).map_err(
        |e| match e {
            SofError::InvalidViewDefinition(msg) => ServerError::NotImplemented(msg),
            other => ServerError::from(other),
        },
    )
}

/// Filter resources by their last updated time using the _since parameter
///
/// This function filters FHIR resources based on their meta.lastUpdated field,
/// returning only resources that have been modified after the specified timestamp.
///
/// # Arguments
/// * `resources` - Vector of FHIR resources as JSON values
/// * `since` - DateTime filter - only include resources modified after this time
///
/// # Returns
/// * `Ok(Vec<serde_json::Value>)` - Filtered resources
/// * `Err(ServerError)` - If filtering fails
fn filter_resources_by_since(
    resources: Vec<serde_json::Value>,
    since: DateTime<Utc>,
) -> ServerResult<Vec<serde_json::Value>> {
    sof_filter_resources_by_since(resources, since).map_err(ServerError::from)
}

/// Simple health check endpoint
pub async fn health_check() -> impl IntoResponse {
    info!("Handling Health Check request");
    Json(serde_json::json!({
        "status": "ok",
        "service": "sof-server",
        "version": env!("CARGO_PKG_VERSION"),
        "timestamp": chrono::Utc::now().to_rfc3339()
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

        // Check that operation is listed at rest level (type-level operation)
        let operations = &cap_stmt["rest"][0]["operation"];
        assert!(operations.as_array().is_some());
        assert_eq!(operations[0]["name"], "viewdefinition-run");
    }

    #[test]
    fn test_filter_resources_by_patient() {
        let resources = vec![
            serde_json::json!({
                "resourceType": "Patient",
                "id": "123"
            }),
            serde_json::json!({
                "resourceType": "Patient",
                "id": "456"
            }),
            serde_json::json!({
                "resourceType": "Observation",
                "id": "obs1",
                "subject": {
                    "reference": "Patient/123"
                }
            }),
            serde_json::json!({
                "resourceType": "Observation",
                "id": "obs2",
                "subject": {
                    "reference": "Patient/456"
                }
            }),
        ];

        let filtered =
            filter_resources_by_patient_and_group(resources, Some("Patient/123"), None).unwrap();

        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0]["id"], "123");
        assert_eq!(filtered[1]["id"], "obs1");
    }

    #[test]
    fn test_filter_resources_with_group_returns_error() {
        let resources = vec![serde_json::json!({
            "resourceType": "Patient",
            "id": "123"
        })];

        let result = filter_resources_by_patient_and_group(resources, None, Some("Group/test"));

        assert!(result.is_err());
        if let Err(ServerError::NotImplemented(msg)) = result {
            assert!(msg.contains("Group filtering is not yet implemented"));
        } else {
            panic!("Expected NotImplemented error");
        }
    }

    #[test]
    fn test_resolve_view_reference_relative() {
        let result = resolve_view_reference("ViewDefinition/123");

        assert!(result.is_err());
        if let Err(ServerError::NotImplemented(msg)) = result {
            assert!(msg.contains("Relative ViewDefinition references are not supported"));
        } else {
            panic!("Expected NotImplemented error");
        }
    }

    #[test]
    fn test_resolve_view_reference_canonical() {
        let result = resolve_view_reference("http://example.org/ViewDefinition/test|1.0.0");

        assert!(result.is_err());
        if let Err(ServerError::NotImplemented(msg)) = result {
            assert!(msg.contains("Canonical URL references with versions are not yet supported"));
        } else {
            panic!("Expected NotImplemented error");
        }
    }

    #[test]
    fn test_resolve_view_reference_absolute() {
        let result = resolve_view_reference("http://example.org/ViewDefinition/123");

        assert!(result.is_err());
        if let Err(ServerError::NotImplemented(msg)) = result {
            assert!(
                msg.contains("Loading ViewDefinitions from external URLs is not yet implemented")
            );
        } else {
            panic!("Expected NotImplemented error");
        }
    }
}
