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
    fhir_format::{self, accept_requires_unsupported_fhir_xml},
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
        ExtractedParameters, RunParameters, RunQueryParams, extract_all_parameters,
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
/// # Body shapes
///
/// The request body may be either:
/// - A FHIR `Parameters` resource (full form, recommended), or
/// - A bare `ViewDefinition` resource (shortcut — equivalent to a Parameters
///   body with a single `viewResource` entry). Other operation parameters
///   (`patient`, `group`, `_format`, `_limit`, `_since`, `header`, `source`)
///   must come from the query string when this shape is used.
///
/// # Parameters (in specification order)
///
/// Parameters can be provided as query parameters or in the request body (FHIR Parameters resource).
/// Parameters in request body take precedence over query parameters.
///
/// | Name | Type | Use | Scope | Min | Max | Documentation |
/// |------|------|-----|-------|-----|-----|---------------|
/// | _format | code | in | type, instance | 0 | 1 | Output format - `application/json`, `application/x-ndjson`, `text/csv`, `application/octet-stream` (parquet). Defaults to `application/x-ndjson` when neither `_format` nor a usable `Accept` header is supplied. |
/// | header | boolean | in | type, instance | 0 | 1 | This parameter only applies to `text/csv` requests. `true` (default) - return headers in the response, `false` - do not return headers. |
/// | viewReference | Reference | in | type, instance | 0 | * | Reference(s) to ViewDefinition(s) to be used for data transformation. (not yet supported) |
/// | viewResource | ViewDefinition | in | type | 0 | * | ViewDefinition(s) to be used for data transformation. |
/// | patient | Reference | in | type, instance | 0 | * | Filter resources by patient. |
/// | group | Reference | in | type, instance | 0 | * | Filter resources by group (resolved via `Group.member.entity` against inline resources). |
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

    // SoF v2 PR #353: `_format` is `0..1` and defaults to `ndjson` when neither
    // `_format` (query or body) nor a usable `Accept` header is supplied. The
    // default is applied downstream in `parse_content_type` / `validate_query_params`.
    let accept_header = headers.get(header::ACCEPT).and_then(|h| h.to_str().ok());

    // Spec Common Operation Behavior axis 2 (representation): the FHIR XML
    // envelope form is not supported → 406, never raw bytes under a FHIR
    // media type.
    if accept_requires_unsupported_fhir_xml(accept_header) {
        return Err(ServerError::NotAcceptable(
            "the application/fhir+xml representation is not supported; \
             use application/fhir+json"
                .to_string(),
        ));
    }

    // The `fhir` output format lives outside the flat-format `ContentType`
    // machinery: detect it up front (query `_format` here; the body `_format`
    // is folded in after extraction) and strip it so the legacy validation
    // below doesn't reject it.
    let is_fhir_format = |f: &str| {
        f.eq_ignore_ascii_case("fhir") || f.eq_ignore_ascii_case(fhir_format::FHIR_JSON_MIME)
    };
    let query_format = params.format.clone();
    let query_format_fhir = query_format.as_deref().map(is_fhir_format).unwrap_or(false);
    let mut params = params;
    if query_format_fhir {
        params.format = None;
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

    // sof-server accepts two body shapes — match the HFS REST handler:
    //   - A FHIR `Parameters` resource carrying `viewResource`, optional
    //     `resource` entries, and operation parameters (`_format`, `_limit`,
    //     `_since`, `patient`, `group`, `header`, `source`).
    //   - A bare `ViewDefinition` resource — equivalent to a `Parameters`
    //     body with a single `viewResource` entry and no others.
    // The bare-ViewDefinition shortcut keeps the CLI/server ergonomic for
    // callers that just want to pipe a ViewDefinition without building a
    // Parameters wrapper. Other parameters (filters, limits, format) must
    // come from the query string when this shape is used.
    let is_bare_view_definition =
        body.get("resourceType").and_then(|v| v.as_str()) == Some("ViewDefinition");
    let extracted_params = if is_bare_view_definition {
        ExtractedParameters {
            view_definition: Some(body),
            ..Default::default()
        }
    } else {
        let parameters = parse_parameters(body)?;
        extract_all_parameters(parameters).map_err(ServerError::BadRequest)?
    };

    // Check for not-yet-implemented parameters
    if extracted_params.view_reference.is_some() {
        return Err(ServerError::NotImplemented(
            "The viewReference parameter is not yet implemented. Please provide the ViewDefinition directly using the viewResource parameter.".to_string()
        ));
    }

    // Group filtering is wired through the compartment-aware filter (see
    // helios_sof::compartment::resolve_group_members_to_patient_refs): each
    // supplied `Group/{id}` is resolved against Group resources in the
    // inline bundle, and its `member.entity` Patient references join the
    // effective patient-compartment set. Absent `patient` / `group`
    // references are rejected by `filter_resources_by_patient_and_group`
    // with `SofError::ReferencedResourceNotFound`, which the error mapper
    // surfaces as `400 Bad Request` + `OperationOutcome.issue.code =
    // not-found` per the SoF v2 spec error table.

    // For backward compatibility, extract the legacy tuple format
    let view_def_json = extracted_params.view_definition;
    let resources_json = if extracted_params.resources.is_empty() {
        None
    } else {
        Some(extracted_params.resources)
    };
    let format_from_body = extracted_params.format;
    let header_from_body = extracted_params.header;

    // Resolve the two spec negotiation axes now that the effective `_format`
    // request is known (body > query > Accept):
    // - axis 1: `_format=fhir` (or `Accept: application/fhir+json` with no
    //   `_format` anywhere) selects the `fhir` output format;
    // - axis 2: `Accept: application/fhir+json` with an explicit flat
    //   `_format` selects the serialized `Binary` envelope representation.
    let body_format_fhir = format_from_body
        .as_deref()
        .map(is_fhir_format)
        .unwrap_or(false);
    let fhir_output = match format_from_body.as_deref().or(query_format.as_deref()) {
        Some(f) => is_fhir_format(f),
        None => fhir_format::accept_has_mime(accept_header, fhir_format::FHIR_JSON_MIME),
    };
    let wants_envelope =
        !fhir_output && fhir_format::accept_has_mime(accept_header, fhir_format::FHIR_JSON_MIME);
    // Strip `fhir` before the legacy ContentType override below.
    let format_from_body = if body_format_fhir {
        None
    } else {
        format_from_body
    };

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
        // If only header is provided in body, update the CSV header flag.
        // Per spec: "Applies only when csv output is requested" — so when
        // the format isn't CSV we silently ignore the parameter rather
        // than rejecting (audit item #14: the spec gives no requirement
        // to error on extraneous use).
        if matches!(
            validated_params.format,
            ContentType::Csv | ContentType::CsvWithHeader
        ) {
            let content_type = parse_content_type(None, Some("text/csv"), Some(header_bool))?;
            validated_params.format = content_type;
        }
        // else: non-CSV format → header is advisory only, ignore it.
    }

    // Apply patient and group filters from body parameters to resources if provided
    let mut filtered_resources = resources_json.unwrap_or_default();

    // Merge filter parameters from body and query. Body takes precedence
    // when non-empty; otherwise comma-split the query value into the spec's
    // 0..* shape so a `?group=Group/a,Group/b` GET works the same way as
    // repeated body entries.
    let patient_filter: Vec<String> = if !extracted_params.patient.is_empty() {
        extracted_params.patient
    } else {
        helios_sof::split_csv_refs(validated_params.patient.as_deref())
    };
    let group_filter: Vec<String> = if !extracted_params.group.is_empty() {
        extracted_params.group
    } else {
        helios_sof::split_csv_refs(validated_params.group.as_deref())
    };
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
        let loaded_bundle = if !patient_filter.is_empty()
            || !group_filter.is_empty()
            || validated_params.since.is_some()
        {
            // Extract resources from source bundle for filtering
            let mut source_resources = extract_resources_from_bundle(&loaded_bundle)?;

            // Apply filters
            if !patient_filter.is_empty() || !group_filter.is_empty() {
                source_resources = filter_resources_by_patient_and_group(
                    source_resources,
                    &patient_filter,
                    &group_filter,
                    source_fhir_version.unwrap(),
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

    // Keep the raw ViewDefinition JSON around when the `fhir` output format
    // is selected — its declared `column.type`s drive the `value[x]` mapping.
    let view_json_for_fhir = fhir_output.then(|| view_def_json.clone());

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
    if !patient_filter.is_empty() || !group_filter.is_empty() {
        let effective_version = source_fhir_version.unwrap_or_else(get_newest_enabled_fhir_version);
        filtered_resources = filter_resources_by_patient_and_group(
            filtered_resources,
            &patient_filter,
            &group_filter,
            effective_version,
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

    // `_format=fhir`: render the typed `Parameters` resource from the
    // structured rows. `_limit` is applied at the row level to match the
    // flat-format pipeline's `apply_pagination_to_result`.
    if let Some(view_json) = view_json_for_fhir {
        let mut processed = process_view_definition(view_definition, bundle)?;
        if let Some(limit) = validated_params.limit {
            processed.rows.truncate(limit);
        }
        let body = fhir_format::format_view_fhir_parameters(&processed, &view_json)?;
        return Ok((
            StatusCode::OK,
            [(header::CONTENT_TYPE, fhir_format::FHIR_JSON_MIME)],
            body,
        )
            .into_response());
    }

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

        // Multi-file output is bundled into a ZIP archive; a single file is
        // returned as-is. Both are fully materialised in memory before the
        // response starts, so both carry a `Content-Length` — no chunked
        // transfer encoding, the size is already known (see
        // `docs/spec-inconsistencies.md`, entry F).
        if file_buffers.len() > 1 {
            info!(
                "Generating ZIP archive with {} Parquet files",
                file_buffers.len()
            );
            let zip = crate::parquet_zip::create_zip_from_buffers(file_buffers, "data")?;
            Ok((
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, "application/zip"),
                    (
                        header::CONTENT_DISPOSITION,
                        "attachment; filename=\"data.zip\"",
                    ),
                ],
                zip,
            )
                .into_response())
        } else {
            // Per the SoF v2 Common Operation Behavior table, parquet's
            // native media type is `application/vnd.apache.parquet`;
            // Content-Disposition makes browsers download as `.parquet`
            // (audit item #8).
            let payload = file_buffers.into_iter().next().unwrap_or_default();
            if wants_envelope {
                let body = fhir_format::wrap_in_binary_envelope(
                    ContentType::Parquet.mime_type(),
                    &payload,
                )?;
                return Ok((
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, fhir_format::FHIR_JSON_MIME)],
                    body,
                )
                    .into_response());
            }
            Ok((
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, ContentType::Parquet.mime_type()),
                    (
                        header::CONTENT_DISPOSITION,
                        "attachment; filename=\"output.parquet\"",
                    ),
                ],
                payload,
            )
                .into_response())
        }
    } else {
        // Standard processing
        // `run_view_definition_with_options` applies `_limit` at the
        // structured-row level before serialization (via
        // `apply_pagination_to_result`), so we don't need to re-truncate
        // the serialized bytes here. Audit item #16 removed the
        // duplicate `apply_result_filtering` pass that used to re-parse
        // and re-serialize the output — it was inefficient and
        // CSV-fragile (line-splits assumed no embedded newlines).
        let filtered_output = run_view_definition_with_options(
            view_definition,
            bundle,
            validated_params.format,
            run_options,
        )?;

        // Determine the MIME type for the response: each format's native
        // media type per the SoF v2 Common Operation Behavior table
        // (parquet is `application/vnd.apache.parquet`).
        let mime_type = validated_params.format.mime_type();

        // Spec axis 2: `Accept: application/fhir+json` with an explicit flat
        // `_format` returns the payload as a serialized `Binary` resource.
        if wants_envelope {
            let body = fhir_format::wrap_in_binary_envelope(mime_type, &filtered_output)?;
            return Ok((
                StatusCode::OK,
                [(header::CONTENT_TYPE, fhir_format::FHIR_JSON_MIME)],
                body,
            )
                .into_response());
        }

        let response = if matches!(validated_params.format, ContentType::Parquet) {
            // Add Content-Disposition for parquet so browsers download as
            // `.parquet` rather than rendering octet-stream as binary noise.
            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, mime_type),
                    (
                        header::CONTENT_DISPOSITION,
                        "attachment; filename=\"output.parquet\"",
                    ),
                ],
                filtered_output,
            )
                .into_response()
        } else {
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, mime_type)],
                filtered_output,
            )
                .into_response()
        };
        Ok(response)
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
        // Output formats the operation produces (audit item #11 partial
        // closeout): sof-server emits CSV, JSON, NDJSON, Parquet, and FHIR
        // Parameters depending on the `_format` parameter. Parquet's native
        // media type is `application/vnd.apache.parquet`;
        // `application/octet-stream` stays listed as the spec Accept-table
        // alias.
        "format": [
            "application/json",
            "application/x-ndjson",
            "text/csv",
            "application/vnd.apache.parquet",
            "application/octet-stream",
            "application/fhir+json"
        ],
        "rest": [{
            "mode": "server",
            // System-level operation (audit item #6 + #7). sof-server is
            // stateless, so:
            // - System-level (`[base]/$viewdefinition-run`) and type-level
            //   (`[base]/ViewDefinition/$viewdefinition-run`) are both
            //   honored — they're aliases for the same handler.
            // - Instance-level (`[base]/ViewDefinition/{id}/$viewdefinition-run`)
            //   is rejected with a 400 because there's no resource store
            //   to look up a stored ViewDefinition by id.
            "operation": [{
                "name": "viewdefinition-run",
                "definition": "http://sql-on-fhir.org/OperationDefinition/$viewdefinition-run",
                "documentation": "Execute a ViewDefinition to transform FHIR resources into tabular format. Supports CSV, JSON, NDJSON, Parquet, and FHIR Parameters (_format=fhir) output; flat formats may also be returned as a Binary resource envelope via 'Accept: application/fhir+json'. Invoked at the system level (POST /$viewdefinition-run) or type level (POST /ViewDefinition/$viewdefinition-run); the ViewDefinition must be supplied inline in the request body via 'viewResource' (no resource store, so 'viewReference' and instance-level URLs are not supported)."
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

/// Parse a ViewDefinition from JSON using a specific FHIR version.
///
/// Per the SoF v2 spec, "invalid ViewDefinition or processing failure"
/// maps to `422 Unprocessable Entity` (audit item #9). We let the
/// default `From<SofError>` impl route `InvalidViewDefinition` through
/// `ServerError::ProcessingError` so it surfaces as 422; the prior
/// special-case to `BadRequest` (400) was the spec gap.
fn parse_view_definition_for_version(
    json: serde_json::Value,
    version: helios_fhir::FhirVersion,
) -> ServerResult<SofViewDefinition> {
    sof_parse_view_definition_for_version(json, version).map_err(ServerError::from)
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
    patient_refs: &[String],
    group_refs: &[String],
    fhir_version: helios_fhir::FhirVersion,
) -> ServerResult<Vec<serde_json::Value>> {
    sof_filter_resources_by_patient_and_group(resources, patient_refs, group_refs, fhir_version)
        .map_err(ServerError::from)
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

/// `GET /$sql-on-fhir-capabilities`
///
/// Returns a FHIR `Parameters` resource describing which SQL-on-FHIR
/// features this server supports. Shape matches HFS REST's
/// implementation so clients can use the same response decoder against
/// either binary. Audit item #11.
///
/// sof-server is stateless, so:
/// - `supportsViewDefinitionRun` = `true`
/// - `supportsViewDefinitionExport` / `supportsSqlQueryRun` = `false`
///   (no async export controller, no `$sqlquery-run` endpoint)
/// - `supportsInDbRunner` = `false` (in-process FHIRPath evaluator only)
/// - `supportsRelativeReference` / `supportsCanonicalReference` /
///   `supportsAbsoluteReference` = `false` (no resource store, so
///   `viewReference` in any shape is rejected with 501 — the
///   capability block must reflect that truthfully).
/// - `supportedFormat` = ndjson, json, csv, parquet, fhir (the formats the
///   `$viewdefinition-run` handler actually emits).
pub async fn sof_capabilities() -> ServerResult<impl IntoResponse> {
    info!("Handling SQL-on-FHIR capabilities request");
    let caps = serde_json::json!({
        "resourceType": "Parameters",
        "parameter": [
            {"name": "supportsViewDefinitionRun", "valueBoolean": true},
            {"name": "supportsViewDefinitionExport", "valueBoolean": false},
            {"name": "supportsSqlQueryRun", "valueBoolean": false},
            {"name": "supportsInDbRunner", "valueBoolean": false},
            {"name": "supportsRelativeReference", "valueBoolean": false},
            {"name": "supportsCanonicalReference", "valueBoolean": false},
            {"name": "supportsAbsoluteReference", "valueBoolean": false},
            {"name": "supportedFormat", "valueCode": "ndjson"},
            {"name": "supportedFormat", "valueCode": "json"},
            {"name": "supportedFormat", "valueCode": "csv"},
            {"name": "supportedFormat", "valueCode": "parquet"},
            {"name": "supportedFormat", "valueCode": "fhir"},
            // Audit item #13: explicit declaration of the spec's
            // OutputFormatCodes value-set binding (extensible).
            // The bound codes (csv/ndjson/parquet/json/fhir) are listed
            // at the canonical CodeSystem URL. The binding is
            // `extensible`, so a client may use additional codes — but
            // sof-server only accepts the five advertised above.
            {
                "name": "formatBinding",
                "part": [
                    {
                        "name": "valueSet",
                        "valueUri": "https://sql-on-fhir.org/ig/ValueSet/OutputFormatCodes"
                    },
                    {"name": "strength", "valueCode": "extensible"}
                ]
            }
        ]
    });
    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/fhir+json")],
        Json(caps),
    ))
}

/// Handler for instance-level `$viewdefinition-run` URLs
/// (`/ViewDefinition/{id}/$viewdefinition-run`).
///
/// sof-server is stateless: it has no resource store, so there is no
/// stored `ViewDefinition/{id}` to invoke. Per the SoF v2 spec the
/// instance-level form infers the ViewDefinition from the URL path; since
/// sof-server can't resolve that, we return `400 Bad Request` with a
/// `not-supported` OperationOutcome rather than `404 Not Found` (which
/// would imply the id is wrong rather than the form being unsupported).
///
/// Audit item #7: makes the instance-level limitation explicit instead
/// of leaving clients to discover it via a routing 404.
pub async fn instance_level_not_supported() -> ServerResult<Response> {
    Err(ServerError::BadRequest(
        "Instance-level $viewdefinition-run (/ViewDefinition/{id}/$viewdefinition-run) is not \
         supported by this stateless server — there is no resource store to look up a stored \
         ViewDefinition by id. Use POST /ViewDefinition/$viewdefinition-run with a 'viewResource' \
         parameter (or a bare ViewDefinition body) instead."
            .to_string(),
    ))
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

        // Audit item #6: the operation is published at the REST-system
        // level (so it's reachable at both [base]/$viewdefinition-run and
        // [base]/ViewDefinition/$viewdefinition-run).
        let operations = &cap_stmt["rest"][0]["operation"];
        assert!(operations.as_array().is_some());
        assert_eq!(operations[0]["name"], "viewdefinition-run");

        // Audit item #7: the documentation makes the stateless scope
        // explicit — no viewReference, no instance-level invocation.
        let doc = operations[0]["documentation"]
            .as_str()
            .expect("documentation must be a string");
        assert!(
            doc.contains("system level") && doc.contains("type level"),
            "doc must mention which scopes are supported: {doc}"
        );
        assert!(
            doc.contains("viewResource"),
            "doc must mention viewResource as the supply mechanism: {doc}"
        );

        // Audit item #11 partial: output formats are listed.
        let formats: Vec<String> = cap_stmt["format"]
            .as_array()
            .expect("format must be an array")
            .iter()
            .filter_map(|f| f.as_str().map(String::from))
            .collect();
        for required in [
            "application/json",
            "application/x-ndjson",
            "text/csv",
            "application/vnd.apache.parquet",
            "application/octet-stream",
            "application/fhir+json",
        ] {
            assert!(
                formats.iter().any(|f| f == required),
                "format must include {required}: {formats:?}"
            );
        }
    }

    /// Audit item #9: an invalid ViewDefinition (e.g. missing the
    /// required `resource` field) must surface as `422 Unprocessable
    /// Entity` per the SoF v2 spec — not `400 Bad Request`. We assert
    /// both the `ServerError` variant and the rendered HTTP status.
    #[cfg(feature = "R4")]
    #[test]
    fn test_invalid_view_definition_maps_to_422() {
        use axum::response::IntoResponse;

        // Type mismatch in the `select` array — serde rejects this
        // because `select` must be an array of Select objects, not a
        // string.
        let bad_view = serde_json::json!({
            "resourceType": "ViewDefinition",
            "status": "active",
            "resource": "Patient",
            "select": "not-an-array"
        });

        let err = parse_view_definition_for_version(bad_view, helios_fhir::FhirVersion::R4)
            .expect_err("malformed ViewDefinition must error");
        assert!(
            matches!(err, ServerError::ProcessingError(_)),
            "invalid ViewDefinition must map to ProcessingError (→ 422), got {err:?}"
        );

        // And render verifies the HTTP status — locks in the spec
        // requirement at the response boundary, not just internally.
        let response = err.into_response();
        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid ViewDefinition response must be 422"
        );
    }

    /// Audit item #7: instance-level URLs return a clear 400 explaining
    /// the stateless limitation, not a 404 or 501. The handler is
    /// route-agnostic (no path extractor) — axum routes ALL instance
    /// URLs to it, and we just return the canned response.
    #[tokio::test]
    async fn test_instance_level_returns_bad_request() {
        let result = instance_level_not_supported().await;
        match result {
            Err(ServerError::BadRequest(msg)) => {
                assert!(
                    msg.contains("Instance-level") && msg.contains("stateless"),
                    "error message must explain the stateless limitation: {msg}"
                );
                assert!(
                    msg.contains("viewResource"),
                    "error message must point at the supported alternative: {msg}"
                );
            }
            other => {
                panic!("expected ServerError::BadRequest for instance-level URL, got {other:?}")
            }
        }
    }

    #[cfg(feature = "R4")]
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

        let filtered = filter_resources_by_patient_and_group(
            resources,
            &["Patient/123".to_string()],
            &[],
            helios_fhir::FhirVersion::R4,
        )
        .unwrap();

        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0]["id"], "123");
        assert_eq!(filtered[1]["id"], "obs1");
    }

    /// Absent `patient` / `group` references are a hard 400 per the SoF v2
    /// spec error table — not the previous "200 + Warning: 199" path. We
    /// assert both the `ServerError` variant and the rendered HTTP status.
    #[cfg(feature = "R4")]
    #[test]
    fn test_filter_with_unresolvable_group_returns_bad_request() {
        let resources = vec![serde_json::json!({
            "resourceType": "Patient",
            "id": "123"
        })];

        let err = filter_resources_by_patient_and_group(
            resources,
            &[],
            &["Group/test".to_string()],
            helios_fhir::FhirVersion::R4,
        )
        .expect_err("absent Group target must error");
        match &err {
            ServerError::ReferencedResourceNotFound(msg) => {
                assert!(
                    msg.contains("Group/test"),
                    "error must name the absent reference: {msg}"
                );
            }
            other => panic!("expected ReferencedResourceNotFound, got {other:?}"),
        }
        let response = err.into_response();
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "absent reference must surface as 400"
        );
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
