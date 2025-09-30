//! Request and response models for the SQL-on-FHIR server
//!
//! This module contains data structures for handling HTTP requests and responses,
//! including parameter parsing and content type negotiation.

use chrono::{DateTime, Utc};
use helios_sof::{ContentType, SofParameters};
use serde::Deserialize;
use tracing::debug;

/// Query parameters for ViewDefinition/$run endpoint
#[derive(Debug, Deserialize)]
pub struct RunQueryParams {
    /// Output format override (alternative to Accept header)
    #[serde(rename = "_format")]
    pub format: Option<String>,

    /// Whether to include headers in CSV output
    #[serde(rename = "header")]
    pub header: Option<String>,

    /// Limit number of results
    #[serde(rename = "_limit")]
    pub limit: Option<usize>,

    /// Include only resources modified after this time
    #[serde(rename = "_since")]
    pub since: Option<String>,

    /// Reference to ViewDefinition(s) for GET requests
    #[serde(rename = "viewReference")]
    pub view_reference: Option<String>,

    /// Filter resources by patient (reference)
    #[serde(rename = "patient")]
    pub patient: Option<String>,

    /// Filter resources by group (reference)
    #[serde(rename = "group")]
    pub group: Option<String>,

    /// Data source for transformation
    #[serde(rename = "source")]
    pub source: Option<String>,

    /// Maximum file size for Parquet output (in MB)
    #[serde(rename = "maxFileSize")]
    pub max_file_size: Option<u32>,

    /// Row group size for Parquet output (in MB)
    #[serde(rename = "rowGroupSize")]
    pub row_group_size: Option<u32>,

    /// Page size for Parquet output (in KB)
    #[serde(rename = "pageSize")]
    pub page_size: Option<u32>,

    /// Compression for Parquet output
    #[serde(rename = "compression")]
    pub compression: Option<String>,
}

/// Validated and parsed query parameters
#[derive(Debug, Clone)]
#[allow(dead_code)] // Some fields are placeholders for future implementation
pub struct ValidatedRunParams {
    /// Output format
    pub format: ContentType,

    /// Limit number of results (None means no limit)
    pub limit: Option<usize>,

    /// Include only resources modified after this time
    pub since: Option<DateTime<Utc>>,

    /// Reference to ViewDefinition(s)
    pub view_reference: Option<String>,

    /// Filter resources by patient
    pub patient: Option<String>,

    /// Filter resources by group
    pub group: Option<String>,

    /// Data source for transformation
    pub source: Option<String>,

    /// Parquet-specific options
    pub parquet_options: Option<helios_sof::ParquetOptions>,
}

/// Parameters for ViewDefinition/$run operation - now using proper FHIR Parameters
pub type RunParameters = SofParameters;

/// Validate and parse query parameters into structured format
///
/// # Arguments
/// * `params` - Raw query parameters from the HTTP request
/// * `accept_header` - Optional Accept header value for content type negotiation
///
/// # Returns
/// * `Ok(ValidatedRunParams)` - Successfully validated parameters
/// * `Err(String)` - Validation error message
///
/// # Validation Rules
/// * `_limit` must be between 1 and 10000
////// * `_since` must be a valid RFC3339 timestamp
/// * `_format` takes precedence over Accept header
pub fn validate_query_params(
    params: &RunQueryParams,
    accept_header: Option<&str>,
) -> Result<ValidatedRunParams, String> {
    // Parse content type
    // Convert header string parameter to boolean
    let header_bool = match params.header.as_deref() {
        Some("true") => Some(true),
        Some("false") => Some(false),
        None => None,
        Some(other) => {
            return Err(format!(
                "Invalid header parameter value: {}. Must be 'true' or 'false'",
                other
            ));
        }
    };
    let format = parse_content_type(accept_header, params.format.as_deref(), header_bool)
        .map_err(|e| e.to_string())?;

    // Validate limit parameter
    let limit = if let Some(c) = params.limit {
        if c == 0 {
            return Err("_limit parameter must be greater than 0".to_string());
        }
        if c > 10000 {
            return Err("_limit parameter cannot exceed 10000".to_string());
        }
        Some(c)
    } else {
        None
    };

    // Validate since parameter
    let since = if let Some(since_str) = &params.since {
        match DateTime::parse_from_rfc3339(since_str) {
            Ok(dt) => Some(dt.with_timezone(&Utc)),
            Err(_) => {
                return Err(format!(
                    "_since parameter must be a valid RFC3339 timestamp: {}",
                    since_str
                ));
            }
        }
    } else {
        None
    };

    // Validate and build Parquet options if any Parquet parameters are provided
    let parquet_options = if params.max_file_size.is_some()
        || params.row_group_size.is_some()
        || params.page_size.is_some()
        || params.compression.is_some()
    {
        // Validate max_file_size
        let max_file_size_mb = if let Some(size) = params.max_file_size {
            if !(10..=10000).contains(&size) {
                return Err("maxFileSize must be between 10 and 10000 MB".to_string());
            }
            Some(size)
        } else {
            None
        };

        // Validate row_group_size
        let row_group_size_mb = if let Some(size) = params.row_group_size {
            if !(64..=1024).contains(&size) {
                return Err("rowGroupSize must be between 64 and 1024 MB".to_string());
            }
            size
        } else {
            256 // Default
        };

        // Validate page_size
        let page_size_kb = if let Some(size) = params.page_size {
            if !(64..=8192).contains(&size) {
                return Err("pageSize must be between 64 and 8192 KB".to_string());
            }
            size
        } else {
            1024 // Default
        };

        // Validate compression
        let compression = if let Some(comp) = &params.compression {
            match comp.to_lowercase().as_str() {
                "none" | "snappy" | "gzip" | "lz4" | "brotli" | "zstd" => comp.clone(),
                _ => {
                    return Err(format!(
                        "Invalid compression type: {}. Must be one of: none, snappy, gzip, lz4, brotli, zstd",
                        comp
                    ));
                }
            }
        } else {
            "snappy".to_string() // Default
        };

        Some(helios_sof::ParquetOptions {
            row_group_size_mb,
            page_size_kb,
            compression,
            max_file_size_mb,
        })
    } else {
        None
    };

    Ok(ValidatedRunParams {
        format,
        limit,
        since,
        view_reference: params.view_reference.clone(),
        patient: params.patient.clone(),
        group: params.group.clone(),
        source: params.source.clone(),
        parquet_options,
    })
}

/// Parse content type from Accept header and query parameters
pub fn parse_content_type(
    accept_header: Option<&str>,
    format_param: Option<&str>,
    header_param: Option<bool>,
) -> Result<ContentType, helios_sof::SofError> {
    // Query parameter takes precedence over Accept header
    let content_type_str = format_param.or(accept_header).unwrap_or("application/json");

    // Handle CSV header parameter
    let content_type_str = if content_type_str == "text/csv" {
        match header_param {
            Some(false) => "text/csv;header=false",
            Some(true) | None => "text/csv;header=true", // Default to true if not specified
        }
    } else {
        content_type_str
    };

    ContentType::from_string(content_type_str)
}

/// Result type for parameter extraction
#[derive(Debug, Default)]
pub struct ExtractedParameters {
    pub view_definition: Option<serde_json::Value>,
    pub resources: Vec<serde_json::Value>,
    pub format: Option<String>,
    pub header: Option<bool>,
    pub view_reference: Option<String>,
    pub patient: Option<String>,
    pub group: Option<String>,
    pub source: Option<String>,
    pub limit: Option<u32>,
    pub since: Option<String>,
    pub max_file_size: Option<u32>,
    pub row_group_size: Option<u32>,
    pub page_size: Option<u32>,
    pub compression: Option<String>,
}

/// Helper function to process a single parameter in a version-independent way
fn process_parameter(
    name: &str,
    param_json: serde_json::Value,
    result: &mut ExtractedParameters,
) -> Result<(), String> {
    // Helper function to check if any value[X] field exists
    let has_any_value_field = |param: &serde_json::Value| -> bool {
        param
            .as_object()
            .is_some_and(|obj| obj.keys().any(|k| k.starts_with("value")))
    };

    match name {
        "viewResource" => {
            if let Some(resource) = param_json.get("resource") {
                // Check if a ViewDefinition has already been provided
                if result.view_definition.is_some() {
                    return Err(
                        "Only one viewResource parameter is allowed. Multiple ViewDefinitions are not supported"
                            .to_string(),
                    );
                }
                result.view_definition = Some(resource.clone());
            } else if has_any_value_field(&param_json) {
                return Err(
                    "viewResource parameter must contain a 'resource' field, not a value[X] field"
                        .to_string(),
                );
            }
        }
        "viewReference" => {
            // Check for valueReference first
            if let Some(value_ref) = param_json.get("valueReference") {
                if let Some(reference) = value_ref.get("reference") {
                    if let Some(ref_str) = reference.as_str() {
                        result.view_reference = Some(ref_str.to_string());
                    }
                }
            } else if let Some(value_str) = param_json.get("valueString") {
                if let Some(ref_str) = value_str.as_str() {
                    result.view_reference = Some(ref_str.to_string());
                }
            } else if has_any_value_field(&param_json) {
                return Err(
                    "viewReference parameter must use valueReference or valueString".to_string(),
                );
            }
        }
        "resource" => {
            if let Some(resource) = param_json.get("resource") {
                // Check if the resource is a Bundle
                if resource.get("resourceType") == Some(&serde_json::json!("Bundle")) {
                    // Extract resources from Bundle entries
                    if let Some(entries) = resource.get("entry").and_then(|e| e.as_array()) {
                        for entry in entries {
                            if let Some(entry_resource) = entry.get("resource") {
                                result.resources.push(entry_resource.clone());
                            }
                        }
                    }
                } else {
                    // Not a Bundle, add the resource directly
                    result.resources.push(resource.clone());
                }
            } else if has_any_value_field(&param_json) {
                return Err(
                    "resource parameter must contain a 'resource' field, not a value[X] field"
                        .to_string(),
                );
            }
        }
        "patient" => {
            // Check for valueReference first
            if let Some(value_ref) = param_json.get("valueReference") {
                if let Some(reference) = value_ref.get("reference") {
                    if let Some(ref_str) = reference.as_str() {
                        result.patient = Some(ref_str.to_string());
                    }
                }
            } else if let Some(value_str) = param_json.get("valueString") {
                if let Some(ref_str) = value_str.as_str() {
                    result.patient = Some(ref_str.to_string());
                }
            } else if has_any_value_field(&param_json) {
                return Err("patient parameter must use valueReference or valueString".to_string());
            }
        }
        "group" => {
            // Check for valueReference first
            if let Some(value_ref) = param_json.get("valueReference") {
                if let Some(reference) = value_ref.get("reference") {
                    if let Some(ref_str) = reference.as_str() {
                        result.group = Some(ref_str.to_string());
                    }
                }
            } else if let Some(value_str) = param_json.get("valueString") {
                if let Some(ref_str) = value_str.as_str() {
                    result.group = Some(ref_str.to_string());
                }
            } else if has_any_value_field(&param_json) {
                return Err("group parameter must use valueReference or valueString".to_string());
            }
        }
        "source" => {
            if let Some(value_str) = param_json.get("valueString") {
                if let Some(source_str) = value_str.as_str() {
                    result.source = Some(source_str.to_string());
                }
            } else if let Some(value_uri) = param_json.get("valueUri") {
                if let Some(source_str) = value_uri.as_str() {
                    result.source = Some(source_str.to_string());
                }
            } else if has_any_value_field(&param_json) {
                return Err("source parameter must use valueString or valueUri".to_string());
            }
        }
        "_format" | "format" => {
            if let Some(value_code) = param_json.get("valueCode") {
                if let Some(format_str) = value_code.as_str() {
                    result.format = Some(format_str.to_string());
                }
            } else if let Some(value_str) = param_json.get("valueString") {
                if let Some(format_str) = value_str.as_str() {
                    result.format = Some(format_str.to_string());
                }
            } else if has_any_value_field(&param_json) {
                return Err("_format parameter must use valueCode or valueString".to_string());
            }
        }
        "header" => {
            if let Some(value_bool) = param_json.get("valueBoolean") {
                if let Some(bool_val) = value_bool.as_bool() {
                    result.header = Some(bool_val);
                } else {
                    return Err("Header parameter must be a boolean value".to_string());
                }
            } else if param_json.get("valueString").is_some()
                || param_json.get("valueCode").is_some()
                || param_json.get("valueInteger").is_some()
            {
                return Err(
                    "Header parameter must be a boolean value (use valueBoolean)".to_string(),
                );
            }
        }
        "_limit" => {
            // Handle both valueInteger and valuePositiveInt
            if let Some(value_int) = param_json.get("valueInteger") {
                if let Some(int_val) = value_int.as_i64() {
                    if int_val <= 0 {
                        return Err("_limit parameter must be greater than 0".to_string());
                    }
                    if int_val > 10000 {
                        return Err("_limit parameter cannot exceed 10000".to_string());
                    }
                    result.limit = Some(int_val as u32);
                }
            } else if let Some(value_pos_int) = param_json.get("valuePositiveInt") {
                if let Some(int_val) = value_pos_int.as_u64() {
                    if int_val > 10000 {
                        return Err("_limit parameter cannot exceed 10000".to_string());
                    }
                    result.limit = Some(int_val as u32);
                }
            } else if has_any_value_field(&param_json) {
                return Err(
                    "_limit parameter must use valueInteger or valuePositiveInt".to_string()
                );
            }
        }
        "maxFileSize" => {
            if let Some(value_int) = param_json.get("valueInteger") {
                if let Some(int_val) = value_int.as_i64() {
                    if !(10..=10000).contains(&int_val) {
                        return Err("maxFileSize must be between 10 and 10000 MB".to_string());
                    }
                    result.max_file_size = Some(int_val as u32);
                }
            } else if let Some(value_pos_int) = param_json.get("valuePositiveInt") {
                if let Some(int_val) = value_pos_int.as_u64() {
                    if !(10..=10000).contains(&int_val) {
                        return Err("maxFileSize must be between 10 and 10000 MB".to_string());
                    }
                    result.max_file_size = Some(int_val as u32);
                }
            } else if has_any_value_field(&param_json) {
                return Err(
                    "maxFileSize parameter must use valueInteger or valuePositiveInt".to_string(),
                );
            }
        }
        "rowGroupSize" => {
            if let Some(value_int) = param_json.get("valueInteger") {
                if let Some(int_val) = value_int.as_i64() {
                    if !(64..=1024).contains(&int_val) {
                        return Err("rowGroupSize must be between 64 and 1024 MB".to_string());
                    }
                    result.row_group_size = Some(int_val as u32);
                }
            } else if let Some(value_pos_int) = param_json.get("valuePositiveInt") {
                if let Some(int_val) = value_pos_int.as_u64() {
                    if !(64..=1024).contains(&int_val) {
                        return Err("rowGroupSize must be between 64 and 1024 MB".to_string());
                    }
                    result.row_group_size = Some(int_val as u32);
                }
            } else if has_any_value_field(&param_json) {
                return Err(
                    "rowGroupSize parameter must use valueInteger or valuePositiveInt".to_string(),
                );
            }
        }
        "pageSize" => {
            if let Some(value_int) = param_json.get("valueInteger") {
                if let Some(int_val) = value_int.as_i64() {
                    if !(64..=8192).contains(&int_val) {
                        return Err("pageSize must be between 64 and 8192 KB".to_string());
                    }
                    result.page_size = Some(int_val as u32);
                }
            } else if let Some(value_pos_int) = param_json.get("valuePositiveInt") {
                if let Some(int_val) = value_pos_int.as_u64() {
                    if !(64..=8192).contains(&int_val) {
                        return Err("pageSize must be between 64 and 8192 KB".to_string());
                    }
                    result.page_size = Some(int_val as u32);
                }
            } else if has_any_value_field(&param_json) {
                return Err(
                    "pageSize parameter must use valueInteger or valuePositiveInt".to_string(),
                );
            }
        }
        "compression" => {
            if let Some(value_code) = param_json.get("valueCode") {
                if let Some(comp_str) = value_code.as_str() {
                    match comp_str.to_lowercase().as_str() {
                        "none" | "snappy" | "gzip" | "lz4" | "brotli" | "zstd" => {
                            result.compression = Some(comp_str.to_string());
                        }
                        _ => {
                            return Err(format!(
                                "Invalid compression type: {}. Must be one of: none, snappy, gzip, lz4, brotli, zstd",
                                comp_str
                            ));
                        }
                    }
                }
            } else if let Some(value_str) = param_json.get("valueString") {
                if let Some(comp_str) = value_str.as_str() {
                    match comp_str.to_lowercase().as_str() {
                        "none" | "snappy" | "gzip" | "lz4" | "brotli" | "zstd" => {
                            result.compression = Some(comp_str.to_string());
                        }
                        _ => {
                            return Err(format!(
                                "Invalid compression type: {}. Must be one of: none, snappy, gzip, lz4, brotli, zstd",
                                comp_str
                            ));
                        }
                    }
                }
            } else if has_any_value_field(&param_json) {
                return Err("compression parameter must use valueCode or valueString".to_string());
            }
        }
        "_since" => {
            // Handle valueInstant (primary) or valueDateTime (alternate)
            if let Some(value_instant) = param_json.get("valueInstant") {
                if let Some(instant_str) = value_instant.as_str() {
                    // Validate RFC3339 format
                    match DateTime::parse_from_rfc3339(instant_str) {
                        Ok(_) => result.since = Some(instant_str.to_string()),
                        Err(_) => {
                            return Err(format!(
                                "_since parameter must be a valid RFC3339 timestamp: {}",
                                instant_str
                            ));
                        }
                    }
                }
            } else if let Some(value_datetime) = param_json.get("valueDateTime") {
                if let Some(datetime_str) = value_datetime.as_str() {
                    // Validate RFC3339 format
                    match DateTime::parse_from_rfc3339(datetime_str) {
                        Ok(_) => result.since = Some(datetime_str.to_string()),
                        Err(_) => {
                            return Err(format!(
                                "_since parameter must be a valid RFC3339 timestamp: {}",
                                datetime_str
                            ));
                        }
                    }
                }
            } else if has_any_value_field(&param_json) {
                return Err("_since parameter must use valueInstant or valueDateTime".to_string());
            }
        }
        _ => {
            // Ignore unknown parameters
            debug!("Ignoring unknown parameter: {}", name);
        }
    }
    Ok(())
}

/// Extract all parameters from a Parameters resource in a version-independent way
pub fn extract_all_parameters(params: RunParameters) -> Result<ExtractedParameters, String> {
    let mut result = ExtractedParameters::default();

    // Convert to JSON for version-independent processing
    let params_json = match serde_json::to_value(&params) {
        Ok(json) => json,
        Err(e) => return Err(format!("Failed to serialize parameters: {}", e)),
    };

    // The JSON structure after serialization wraps the actual Parameters in an enum variant
    // We need to extract the actual Parameters object from the enum variant
    let actual_params = match &params {
        #[cfg(feature = "R4")]
        RunParameters::R4(_) => params_json.get("R4"),
        #[cfg(feature = "R4B")]
        RunParameters::R4B(_) => params_json.get("R4B"),
        #[cfg(feature = "R5")]
        RunParameters::R5(_) => params_json.get("R5"),
        #[cfg(feature = "R6")]
        RunParameters::R6(_) => params_json.get("R6"),
    }
    .unwrap_or(&params_json);

    // Extract parameter array
    if let Some(param_array) = actual_params.get("parameter").and_then(|p| p.as_array()) {
        for param in param_array {
            // In the serialized FHIR structure, name is a complex type with a "value" field
            let name = if let Some(name_obj) = param.get("name") {
                if let Some(name_value) = name_obj.get("value") {
                    name_value.as_str()
                } else {
                    // Try direct string access (for simpler test cases)
                    name_obj.as_str()
                }
            } else {
                None
            };

            if let Some(name_str) = name {
                process_parameter(name_str, param.clone(), &mut result)?;
            }
        }
    }

    Ok(result)
}

/// Apply filtering to output data based on validated parameters
///
/// This function applies post-processing filters like pagination to the
/// transformed output data. It handles different output formats appropriately.
///
/// # Arguments
/// * `output_data` - Raw output bytes from ViewDefinition execution
/// * `params` - Validated query parameters containing filtering options
///
/// # Returns
/// * `Ok(Vec<u8>)` - Filtered output data
/// * `Err(String)` - Error message if filtering fails
///
/// # Supported Filters
/// * Count limiting - Applied using `_limit` parameter
/// * Format-aware - Handles CSV headers correctly during pagination
///
/// # Note
/// The `_since` parameter is validated but not applied here as it requires
/// filtering at the resource level before transformation.
pub fn apply_result_filtering(
    output_data: Vec<u8>,
    params: &ValidatedRunParams,
) -> Result<Vec<u8>, String> {
    // Apply pagination and count limiting
    // Note: _since filtering is applied at the resource level before ViewDefinition transformation

    match params.format {
        ContentType::Json | ContentType::NdJson => apply_json_filtering(output_data, params),
        ContentType::Csv | ContentType::CsvWithHeader => apply_csv_filtering(output_data, params),
        ContentType::Parquet => {
            // Parquet filtering is not implemented in this scope
            Ok(output_data)
        }
    }
}

/// Apply filtering to JSON/NDJSON output
fn apply_json_filtering(
    output_data: Vec<u8>,
    params: &ValidatedRunParams,
) -> Result<Vec<u8>, String> {
    let output_str =
        String::from_utf8(output_data).map_err(|e| format!("Invalid UTF-8 in output: {}", e))?;

    if params.limit.is_none() {
        return Ok(output_str.into_bytes());
    }

    match params.format {
        ContentType::Json => {
            // Parse as JSON array and apply pagination
            let mut records: Vec<serde_json::Value> = serde_json::from_str(&output_str)
                .map_err(|e| format!("Invalid JSON output: {}", e))?;

            apply_pagination_to_records(&mut records, params);

            let filtered_json = serde_json::to_string(&records)
                .map_err(|e| format!("Failed to serialize filtered JSON: {}", e))?;
            Ok(filtered_json.into_bytes())
        }
        ContentType::NdJson => {
            // Parse as NDJSON and apply pagination
            let mut records = Vec::new();
            for line in output_str.lines() {
                if !line.trim().is_empty() {
                    let record: serde_json::Value = serde_json::from_str(line)
                        .map_err(|e| format!("Invalid NDJSON line: {}", e))?;
                    records.push(record);
                }
            }

            apply_pagination_to_records(&mut records, params);

            let filtered_ndjson = records
                .iter()
                .map(serde_json::to_string)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("Failed to serialize filtered NDJSON: {}", e))?
                .join("\n");
            Ok(filtered_ndjson.into_bytes())
        }
        _ => Ok(output_str.into_bytes()),
    }
}

/// Apply filtering to CSV output
fn apply_csv_filtering(
    output_data: Vec<u8>,
    params: &ValidatedRunParams,
) -> Result<Vec<u8>, String> {
    let output_str = String::from_utf8(output_data)
        .map_err(|e| format!("Invalid UTF-8 in CSV output: {}", e))?;

    if params.limit.is_none() {
        return Ok(output_str.into_bytes());
    }

    let lines: Vec<&str> = output_str.lines().collect();
    if lines.is_empty() {
        return Ok(output_str.into_bytes());
    }

    // Check if we have headers based on the format
    let has_header = matches!(params.format, ContentType::CsvWithHeader);
    let header_offset = if has_header { 1 } else { 0 };

    if lines.len() <= header_offset {
        return Ok(output_str.into_bytes());
    }

    // Split into header and data lines
    let (header_lines, data_lines) = if has_header {
        (lines[0..1].to_vec(), lines[1..].to_vec())
    } else {
        (Vec::new(), lines)
    };

    // Apply pagination to data lines
    let mut data_lines = data_lines;
    apply_pagination_to_lines(&mut data_lines, params);

    // Reconstruct CSV
    let mut result_lines = header_lines;
    result_lines.extend(data_lines);
    let result = result_lines.join("\n");

    // Add final newline if original had one
    if output_str.ends_with('\n') && !result.ends_with('\n') {
        Ok(format!("{}\n", result).into_bytes())
    } else {
        Ok(result.into_bytes())
    }
}

/// Apply limit limiting to a vector of JSON records
fn apply_pagination_to_records(records: &mut Vec<serde_json::Value>, params: &ValidatedRunParams) {
    if let Some(limit) = params.limit {
        records.truncate(limit);
    }
}

/// Apply limit limiting to a vector of string lines
fn apply_pagination_to_lines(lines: &mut Vec<&str>, params: &ValidatedRunParams) {
    if let Some(limit) = params.limit {
        lines.truncate(limit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_content_type() {
        // Test Accept header
        assert_eq!(
            parse_content_type(Some("text/csv"), None, None).unwrap(),
            ContentType::CsvWithHeader
        );

        // Test format parameter override
        assert_eq!(
            parse_content_type(Some("text/csv"), Some("application/json"), None).unwrap(),
            ContentType::Json
        );

        // Test CSV with header parameter false
        assert_eq!(
            parse_content_type(None, Some("text/csv"), Some(false)).unwrap(),
            ContentType::Csv
        );

        // Test CSV with header parameter true
        assert_eq!(
            parse_content_type(None, Some("text/csv"), Some(true)).unwrap(),
            ContentType::CsvWithHeader
        );

        // Test CSV without header parameter (defaults to true)
        assert_eq!(
            parse_content_type(None, Some("text/csv"), None).unwrap(),
            ContentType::CsvWithHeader
        );
    }

    #[test]
    fn test_validate_query_params() {
        // Test valid parameters
        let params = RunQueryParams {
            format: Some("application/json".to_string()),
            header: None,
            limit: Some(10),
            since: Some("2023-01-01T00:00:00Z".to_string()),
            view_reference: None,
            patient: None,
            group: None,
            source: None,
            max_file_size: None,
            row_group_size: None,
            page_size: None,
            compression: None,
        };

        let result = validate_query_params(&params, None).unwrap();
        assert_eq!(result.format, ContentType::Json);
        assert_eq!(result.limit, Some(10));
        assert!(result.since.is_some());
    }

    #[test]
    fn test_validate_query_params_invalid_limit() {
        let params = RunQueryParams {
            format: None,
            header: None,
            limit: Some(0),
            since: None,
            view_reference: None,
            patient: None,
            group: None,
            source: None,
            max_file_size: None,
            row_group_size: None,
            page_size: None,
            compression: None,
        };

        let result = validate_query_params(&params, None);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("_limit parameter must be greater than 0")
        );
    }

    #[test]
    fn test_validate_query_params_invalid_since() {
        let params = RunQueryParams {
            format: None,
            header: None,
            limit: None,
            since: Some("invalid-date".to_string()),
            view_reference: None,
            patient: None,
            group: None,
            source: None,
            max_file_size: None,
            row_group_size: None,
            page_size: None,
            compression: None,
        };

        let result = validate_query_params(&params, None);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("_since parameter must be a valid RFC3339 timestamp")
        );
    }

    #[test]
    fn test_apply_csv_filtering() {
        let csv_data = "id,name\n1,John\n2,Jane\n3,Bob\n4,Alice\n"
            .as_bytes()
            .to_vec();
        let params = ValidatedRunParams {
            format: ContentType::CsvWithHeader,
            limit: Some(2),
            since: None,
            view_reference: None,
            patient: None,
            group: None,
            source: None,
            parquet_options: None,
        };

        let result = apply_csv_filtering(csv_data, &params).unwrap();
        let result_str = String::from_utf8(result).unwrap();

        assert!(result_str.contains("id,name"));
        assert!(result_str.contains("1,John"));
        assert!(result_str.contains("2,Jane"));
        assert!(!result_str.contains("3,Bob"));
        assert!(!result_str.contains("4,Alice"));
    }

    #[test]
    fn test_apply_json_filtering() {
        let json_data =
            r#"[{"id":"1","name":"John"},{"id":"2","name":"Jane"},{"id":"3","name":"Bob"}]"#
                .as_bytes()
                .to_vec();
        let params = ValidatedRunParams {
            format: ContentType::Json,
            limit: Some(2),
            since: None,
            view_reference: None,
            patient: None,
            group: None,
            source: None,
            parquet_options: None,
        };

        let result = apply_json_filtering(json_data, &params).unwrap();
        let result_str = String::from_utf8(result).unwrap();
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&result_str).unwrap();

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0]["id"], "1");
        assert_eq!(parsed[1]["id"], "2");
    }

    #[test]
    fn test_extract_viewreference_parameter() {
        let params_json = serde_json::json!({
            "resourceType": "Parameters",
            "parameter": [{
                "name": "viewReference",
                "valueReference": {
                    "reference": "ViewDefinition/123"
                }
            }]
        });

        #[cfg(feature = "R4")]
        {
            let params: helios_fhir::r4::Parameters = serde_json::from_value(params_json).unwrap();
            let run_params = RunParameters::R4(params);
            let extracted = extract_all_parameters(run_params).unwrap();

            assert_eq!(
                extracted.view_reference,
                Some("ViewDefinition/123".to_string())
            );
        }
    }

    #[test]
    fn test_extract_patient_parameter() {
        let params_json = serde_json::json!({
            "resourceType": "Parameters",
            "parameter": [{
                "name": "patient",
                "valueReference": {
                    "reference": "Patient/456"
                }
            }]
        });

        #[cfg(feature = "R4")]
        {
            let params: helios_fhir::r4::Parameters = serde_json::from_value(params_json).unwrap();
            let run_params = RunParameters::R4(params);
            let extracted = extract_all_parameters(run_params).unwrap();

            assert_eq!(extracted.patient, Some("Patient/456".to_string()));
        }
    }

    #[test]
    fn test_extract_group_parameter() {
        let params_json = serde_json::json!({
            "resourceType": "Parameters",
            "parameter": [{
                "name": "group",
                "valueString": "Group/my-group"
            }]
        });

        #[cfg(feature = "R4")]
        {
            let params: helios_fhir::r4::Parameters = serde_json::from_value(params_json).unwrap();
            let run_params = RunParameters::R4(params);
            let extracted = extract_all_parameters(run_params).unwrap();

            assert_eq!(extracted.group, Some("Group/my-group".to_string()));
        }
    }

    #[test]
    fn test_extract_source_parameter() {
        let params_json = serde_json::json!({
            "resourceType": "Parameters",
            "parameter": [{
                "name": "source",
                "valueString": "s3://bucket/path"
            }]
        });

        #[cfg(feature = "R4")]
        {
            let params: helios_fhir::r4::Parameters = serde_json::from_value(params_json).unwrap();
            let run_params = RunParameters::R4(params);
            let extracted = extract_all_parameters(run_params).unwrap();

            assert_eq!(extracted.source, Some("s3://bucket/path".to_string()));
        }
    }

    #[test]
    fn test_extract_multiple_parameters() {
        let params_json = serde_json::json!({
            "resourceType": "Parameters",
            "parameter": [
                {
                    "name": "viewResource",
                    "resource": {
                        "resourceType": "ViewDefinition",
                        "status": "active",
                        "resource": "Patient"
                    }
                },
                {
                    "name": "patient",
                    "valueReference": {
                        "reference": "Patient/123"
                    }
                },
                {
                    "name": "_format",
                    "valueCode": "csv"
                },
                {
                    "name": "header",
                    "valueBoolean": false
                }
            ]
        });

        #[cfg(feature = "R4")]
        {
            let params: helios_fhir::r4::Parameters = serde_json::from_value(params_json).unwrap();
            let run_params = RunParameters::R4(params);
            let extracted = extract_all_parameters(run_params).unwrap();

            assert!(extracted.view_definition.is_some());
            assert_eq!(extracted.patient, Some("Patient/123".to_string()));
            assert_eq!(extracted.format, Some("csv".to_string()));
            assert_eq!(extracted.header, Some(false));
        }
    }

    #[test]
    fn test_extract_since_parameter_with_valueinstant() {
        let params_json = serde_json::json!({
            "resourceType": "Parameters",
            "parameter": [{
                "name": "_since",
                "valueInstant": "2023-01-01T00:00:00Z"
            }]
        });

        #[cfg(feature = "R4")]
        {
            let params: helios_fhir::r4::Parameters = serde_json::from_value(params_json).unwrap();
            let run_params = RunParameters::R4(params);
            let extracted = extract_all_parameters(run_params).unwrap();

            assert_eq!(extracted.since, Some("2023-01-01T00:00:00Z".to_string()));
        }
    }

    #[test]
    fn test_extract_since_parameter_with_valuedatetime() {
        let params_json = serde_json::json!({
            "resourceType": "Parameters",
            "parameter": [{
                "name": "_since",
                "valueDateTime": "2023-01-01T00:00:00Z"
            }]
        });

        #[cfg(feature = "R4")]
        {
            let params: helios_fhir::r4::Parameters = serde_json::from_value(params_json).unwrap();
            let run_params = RunParameters::R4(params);
            let extracted = extract_all_parameters(run_params).unwrap();

            assert_eq!(extracted.since, Some("2023-01-01T00:00:00Z".to_string()));
        }
    }

    #[test]
    fn test_extract_since_parameter_invalid() {
        // Note: Due to how FHIR choice types work, an invalid valueInstant might be
        // silently skipped during deserialization rather than causing an error.
        // This test verifies that validation still happens at the extraction level.

        // Test 1: Using valueString instead of valueInstant/valueDateTime
        // This should succeed but _since should be None since wrong type was used
        let params_json = serde_json::json!({
            "resourceType": "Parameters",
            "parameter": [{
                "name": "_since",
                "valueString": "not-a-valid-timestamp"  // Wrong type for _since
            }]
        });

        #[cfg(feature = "R4")]
        {
            let params: helios_fhir::r4::Parameters =
                serde_json::from_value(params_json.clone()).unwrap();
            let run_params = RunParameters::R4(params);
            let result = extract_all_parameters(run_params);

            // The extraction should fail because valueString was used instead of valueInstant/valueDateTime
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert!(err.contains("_since parameter must use valueInstant or valueDateTime"));
        }

        // Test 2: Use from_str with an actual invalid instant to ensure it fails
        let invalid_json_str = r#"{
            "resourceType": "Parameters",
            "parameter": [{
                "name": "_since",
                "valueInstant": "not-a-valid-timestamp"
            }]
        }"#;

        #[cfg(feature = "R4")]
        {
            // When using from_str with an invalid instant, check what actually happens
            let result: Result<helios_fhir::r4::Parameters, _> =
                serde_json::from_str(invalid_json_str);
            match result {
                Ok(params) => {
                    // Check if the parameter value is None (skipped due to invalid instant)
                    if let Some(param_list) = &params.parameter {
                        if let Some(first_param) = param_list.first() {
                            assert!(
                                first_param.value.is_none(),
                                "Expected value to be None due to invalid instant, but got: {:?}",
                                first_param.value
                            );
                        }
                    }

                    // Now test extraction
                    let run_params = RunParameters::R4(params);
                    let extract_result = extract_all_parameters(run_params);

                    // When value is None, extraction should succeed but _since should be None
                    assert!(extract_result.is_ok());
                    let extracted = extract_result.unwrap();
                    assert!(
                        extracted.since.is_none(),
                        "Expected since to be None when valueInstant is invalid"
                    );
                }
                Err(e) => {
                    // If deserialization actually fails, that's also acceptable
                    println!("Deserialization failed as expected: {}", e);
                }
            }
        }

        #[cfg(feature = "R4B")]
        {
            let params: helios_fhir::r4b::Parameters =
                serde_json::from_value(params_json.clone()).unwrap();
            let run_params = RunParameters::R4B(params);
            let result = extract_all_parameters(run_params);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert!(err.contains("_since parameter must use valueInstant or valueDateTime"));

            let result: Result<helios_fhir::r4b::Parameters, _> =
                serde_json::from_str(invalid_json_str);
            if let Ok(params) = result {
                if let Some(param_list) = &params.parameter {
                    if let Some(first_param) = param_list.first() {
                        assert!(first_param.value.is_none());
                    }
                }
            }
        }

        #[cfg(feature = "R5")]
        {
            let params: helios_fhir::r5::Parameters =
                serde_json::from_value(params_json.clone()).unwrap();
            let run_params = RunParameters::R5(params);
            let result = extract_all_parameters(run_params);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert!(err.contains("_since parameter must use valueInstant or valueDateTime"));

            let result: Result<helios_fhir::r5::Parameters, _> =
                serde_json::from_str(invalid_json_str);
            if let Ok(params) = result {
                if let Some(param_list) = &params.parameter {
                    if let Some(first_param) = param_list.first() {
                        assert!(first_param.value.is_none());
                    }
                }
            }
        }
    }

    #[test]
    fn test_invalid_value_types_for_parameters() {
        // Test _since with wrong value type
        let params_json = serde_json::json!({
            "resourceType": "Parameters",
            "parameter": [{
                "name": "_since",
                "valueString": "2023-01-01T00:00:00Z"
            }]
        });

        #[cfg(feature = "R4")]
        {
            let params: helios_fhir::r4::Parameters = serde_json::from_value(params_json).unwrap();
            let run_params = RunParameters::R4(params);
            let result = extract_all_parameters(run_params);
            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err(),
                "_since parameter must use valueInstant or valueDateTime"
            );
        }

        // Test _limit with wrong value type
        let params_json = serde_json::json!({
            "resourceType": "Parameters",
            "parameter": [{
                "name": "_limit",
                "valueString": "10"
            }]
        });

        #[cfg(feature = "R4")]
        {
            let params: helios_fhir::r4::Parameters = serde_json::from_value(params_json).unwrap();
            let run_params = RunParameters::R4(params);
            let result = extract_all_parameters(run_params);
            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err(),
                "_limit parameter must use valueInteger or valuePositiveInt"
            );
        }

        // Test header with wrong value type
        let params_json = serde_json::json!({
            "resourceType": "Parameters",
            "parameter": [{
                "name": "header",
                "valueString": "true"
            }]
        });

        #[cfg(feature = "R4")]
        {
            let params: helios_fhir::r4::Parameters = serde_json::from_value(params_json).unwrap();
            let run_params = RunParameters::R4(params);
            let result = extract_all_parameters(run_params);
            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err(),
                "Header parameter must be a boolean value (use valueBoolean)"
            );
        }

        // Test _format with wrong value type
        let params_json = serde_json::json!({
            "resourceType": "Parameters",
            "parameter": [{
                "name": "_format",
                "valueBoolean": true
            }]
        });

        #[cfg(feature = "R4")]
        {
            let params: helios_fhir::r4::Parameters = serde_json::from_value(params_json).unwrap();
            let run_params = RunParameters::R4(params);
            let result = extract_all_parameters(run_params);
            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err(),
                "_format parameter must use valueCode or valueString"
            );
        }

        // Test patient with wrong value type
        let params_json = serde_json::json!({
            "resourceType": "Parameters",
            "parameter": [{
                "name": "patient",
                "valueInteger": 123
            }]
        });

        #[cfg(feature = "R4")]
        {
            let params: helios_fhir::r4::Parameters = serde_json::from_value(params_json).unwrap();
            let run_params = RunParameters::R4(params);
            let result = extract_all_parameters(run_params);
            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err(),
                "patient parameter must use valueReference or valueString"
            );
        }

        // Test group with wrong value type
        let params_json = serde_json::json!({
            "resourceType": "Parameters",
            "parameter": [{
                "name": "group",
                "valueBoolean": false
            }]
        });

        #[cfg(feature = "R4")]
        {
            let params: helios_fhir::r4::Parameters = serde_json::from_value(params_json).unwrap();
            let run_params = RunParameters::R4(params);
            let result = extract_all_parameters(run_params);
            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err(),
                "group parameter must use valueReference or valueString"
            );
        }

        // Test source with wrong value type
        let params_json = serde_json::json!({
            "resourceType": "Parameters",
            "parameter": [{
                "name": "source",
                "valueInteger": 42
            }]
        });

        #[cfg(feature = "R4")]
        {
            let params: helios_fhir::r4::Parameters = serde_json::from_value(params_json).unwrap();
            let run_params = RunParameters::R4(params);
            let result = extract_all_parameters(run_params);
            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err(),
                "source parameter must use valueString or valueUri"
            );
        }

        // Test viewReference with wrong value type
        let params_json = serde_json::json!({
            "resourceType": "Parameters",
            "parameter": [{
                "name": "viewReference",
                "valueBoolean": true
            }]
        });

        #[cfg(feature = "R4")]
        {
            let params: helios_fhir::r4::Parameters = serde_json::from_value(params_json).unwrap();
            let run_params = RunParameters::R4(params);
            let result = extract_all_parameters(run_params);
            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err(),
                "viewReference parameter must use valueReference or valueString"
            );
        }

        // Test viewResource with value field instead of resource
        let params_json = serde_json::json!({
            "resourceType": "Parameters",
            "parameter": [{
                "name": "viewResource",
                "valueString": "ViewDefinition/123"
            }]
        });

        #[cfg(feature = "R4")]
        {
            let params: helios_fhir::r4::Parameters = serde_json::from_value(params_json).unwrap();
            let run_params = RunParameters::R4(params);
            let result = extract_all_parameters(run_params);
            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err(),
                "viewResource parameter must contain a 'resource' field, not a value[X] field"
            );
        }

        // Test resource parameter with value field
        let params_json = serde_json::json!({
            "resourceType": "Parameters",
            "parameter": [{
                "name": "resource",
                "valueString": "Patient/123"
            }]
        });

        #[cfg(feature = "R4")]
        {
            let params: helios_fhir::r4::Parameters = serde_json::from_value(params_json).unwrap();
            let run_params = RunParameters::R4(params);
            let result = extract_all_parameters(run_params);
            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err(),
                "resource parameter must contain a 'resource' field, not a value[X] field"
            );
        }

        // Test multiple viewResource parameters - should fail
        let params_json = serde_json::json!({
            "resourceType": "Parameters",
            "parameter": [{
                "name": "viewResource",
                "resource": {
                    "resourceType": "ViewDefinition",
                    "resource": "Patient",
                    "select": [{
                        "column": [{
                            "name": "id",
                            "path": "id"
                        }]
                    }]
                }
            }, {
                "name": "viewResource",
                "resource": {
                    "resourceType": "ViewDefinition",
                    "resource": "Observation",
                    "select": [{
                        "column": [{
                            "name": "id",
                            "path": "id"
                        }]
                    }]
                }
            }]
        });

        #[cfg(feature = "R4")]
        {
            let params: helios_fhir::r4::Parameters = serde_json::from_value(params_json).unwrap();
            let run_params = RunParameters::R4(params);
            let result = extract_all_parameters(run_params);
            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err(),
                "Only one viewResource parameter is allowed. Multiple ViewDefinitions are not supported"
            );
        }
    }
}
