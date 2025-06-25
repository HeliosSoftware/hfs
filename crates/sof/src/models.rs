//! Request and response models for the SQL-on-FHIR server
//!
//! This module contains data structures for handling HTTP requests and responses,
//! including parameter parsing and content type negotiation.

use serde::{Deserialize, Serialize};
use sof::{ContentType, SofParameters};
use chrono::{DateTime, Utc};

/// Query parameters for ViewDefinition/$run endpoint
#[derive(Debug, Deserialize)]
pub struct RunQueryParams {
    /// Output format override (alternative to Accept header)
    #[serde(rename = "_format")]
    pub format: Option<String>,
    
    /// Whether to include headers in CSV output
    #[serde(rename = "_header")]
    pub header: Option<String>,
    
    /// Limit number of results
    #[serde(rename = "_count")]
    pub count: Option<usize>,
    
    /// Page number for pagination
    #[serde(rename = "_page")]
    pub page: Option<usize>,
    
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
}

/// Validated and parsed query parameters
#[derive(Debug, Clone)]
#[allow(dead_code)] // Some fields are placeholders for future implementation
pub struct ValidatedRunParams {
    /// Output format
    pub format: ContentType,
    
    /// Limit number of results (None means no limit)
    pub count: Option<usize>,
    
    /// Page number for pagination (1-based, None means first page)
    pub page: Option<usize>,
    
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
}

/// Parameters for ViewDefinition/$run operation - now using proper FHIR Parameters
pub type RunParameters = SofParameters;

/// FHIR Reference type
#[derive(Debug, Deserialize, Serialize)]
pub struct Reference {
    /// Reference string (e.g., "ViewDefinition/123")
    pub reference: Option<String>,
    
    /// Display text
    pub display: Option<String>,
}

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
/// * `_count` must be between 1 and 10000
/// * `_page` must be >= 1 (1-based pagination)
/// * `_since` must be a valid RFC3339 timestamp
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
        Some(other) => return Err(format!("Invalid _header parameter value: {}. Must be 'true' or 'false'", other)),
    };
    let format = parse_content_type(
        accept_header,
        params.format.as_deref(),
        header_bool,
    ).map_err(|e| e.to_string())?;
    
    // Validate count parameter
    let count = if let Some(c) = params.count {
        if c == 0 {
            return Err("_count parameter must be greater than 0".to_string());
        }
        if c > 10000 {
            return Err("_count parameter cannot exceed 10000".to_string());
        }
        Some(c)
    } else {
        None
    };
    
    // Validate page parameter
    let page = if let Some(p) = params.page {
        if p == 0 {
            return Err("_page parameter must be greater than 0 (1-based)".to_string());
        }
        Some(p)
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
    
    Ok(ValidatedRunParams {
        format,
        count,
        page,
        since,
        view_reference: params.view_reference.clone(),
        patient: params.patient.clone(),
        group: params.group.clone(),
        source: params.source.clone(),
    })
}

/// Parse content type from Accept header and query parameters
pub fn parse_content_type(
    accept_header: Option<&str>,
    format_param: Option<&str>,
    header_param: Option<bool>,
) -> Result<ContentType, sof::SofError> {
    // Query parameter takes precedence over Accept header
    let content_type_str = format_param
        .or(accept_header)
        .unwrap_or("application/json");
    
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

/// Extract ViewDefinition and Bundle from Parameters
pub fn extract_run_parameters(
    params: RunParameters,
) -> Result<(Option<serde_json::Value>, Option<Vec<serde_json::Value>>, Option<String>, Option<bool>), String> {
    let mut view_definition = None;
    let mut resources = Vec::new();
    let mut format_param = None;
    let mut header_param = None;
    
    // Extract parameters based on FHIR version - handle each version separately
    match &params {
        #[cfg(feature = "R4")]
        SofParameters::R4(p) => {
            if let Some(param_list) = &p.parameter {
                for param in param_list {
                    if let Some(name_value) = &param.name.value {
                        match name_value.as_str() {
                            "viewResource" => {
                                if let Some(resource) = &param.resource {
                                    match serde_json::to_value(resource) {
                                        Ok(json_val) => view_definition = Some(json_val),
                                        Err(e) => return Err(format!("Failed to serialize viewResource: {}", e)),
                                    }
                                }
                            }
                            "viewReference" => {
                                return Err("viewReference parameter is not yet supported".to_string());
                            }
                            "resource" => {
                                if let Some(resource) = &param.resource {
                                    match serde_json::to_value(resource) {
                                        Ok(json_val) => resources.push(json_val),
                                        Err(e) => return Err(format!("Failed to serialize resource: {}", e)),
                                    }
                                }
                            }
                            // TODO: Add support for other parameters
                            "patient" => {
                                // This is a filter parameter, not a resource parameter
                                // Will be implemented when we add filtering support
                            }
                            "group" => {
                                // This is a filter parameter, not a resource parameter
                                // Will be implemented when we add filtering support
                            }
                            "source" => {
                                // This specifies the data source
                                // Will be implemented when we add source selection support
                            }
                            "_format" | "format" => {
                                // Extract format parameter from the value choice type
                                if let Some(value) = &param.value {
                                    match value {
                                        fhir::r4::ParametersParameterValue::Code(code) => {
                                            if let Some(code_value) = &code.value {
                                                format_param = Some(code_value.clone());
                                            }
                                        }
                                        fhir::r4::ParametersParameterValue::String(string) => {
                                            if let Some(string_value) = &string.value {
                                                format_param = Some(string_value.clone());
                                            }
                                        }
                                        _ => {} // Other value types not applicable for _format
                                    }
                                }
                            }
                            "_header" | "header" => {
                                // Extract header parameter from the value choice type (boolean only)
                                if let Some(value) = &param.value {
                                    match value {
                                        fhir::r4::ParametersParameterValue::Boolean(boolean) => {
                                            if let Some(bool_value) = &boolean.value {
                                                header_param = Some(*bool_value);
                                            }
                                        }
                                        _ => return Err("Header parameter must be a boolean value".to_string()),
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        #[cfg(feature = "R4B")]
        SofParameters::R4B(p) => {
            if let Some(param_list) = &p.parameter {
                for param in param_list {
                    if let Some(name_value) = &param.name.value {
                        match name_value.as_str() {
                            "viewResource" => {
                                if let Some(resource) = &param.resource {
                                    match serde_json::to_value(resource) {
                                        Ok(json_val) => view_definition = Some(json_val),
                                        Err(e) => return Err(format!("Failed to serialize viewResource: {}", e)),
                                    }
                                }
                            }
                            "viewReference" => {
                                return Err("viewReference parameter is not yet supported".to_string());
                            }
                            "resource" => {
                                if let Some(resource) = &param.resource {
                                    match serde_json::to_value(resource) {
                                        Ok(json_val) => resources.push(json_val),
                                        Err(e) => return Err(format!("Failed to serialize resource: {}", e)),
                                    }
                                }
                            }
                            // TODO: Add support for other parameters
                            "patient" => {
                                // This is a filter parameter, not a resource parameter
                                // Will be implemented when we add filtering support
                            }
                            "group" => {
                                // This is a filter parameter, not a resource parameter
                                // Will be implemented when we add filtering support
                            }
                            "source" => {
                                // This specifies the data source
                                // Will be implemented when we add source selection support
                            }
                            "_format" | "format" => {
                                // Extract format parameter from the value choice type
                                if let Some(value) = &param.value {
                                    match value {
                                        fhir::r4b::ParametersParameterValue::Code(code) => {
                                            if let Some(code_value) = &code.value {
                                                format_param = Some(code_value.clone());
                                            }
                                        }
                                        fhir::r4b::ParametersParameterValue::String(string) => {
                                            if let Some(string_value) = &string.value {
                                                format_param = Some(string_value.clone());
                                            }
                                        }
                                        _ => {} // Other value types not applicable for _format
                                    }
                                }
                            }
                            "_header" | "header" => {
                                // Extract header parameter from the value choice type (boolean only)
                                if let Some(value) = &param.value {
                                    match value {
                                        fhir::r4b::ParametersParameterValue::Boolean(boolean) => {
                                            if let Some(bool_value) = &boolean.value {
                                                header_param = Some(*bool_value);
                                            }
                                        }
                                        _ => return Err("Header parameter must be a boolean value".to_string()),
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        #[cfg(feature = "R5")]
        SofParameters::R5(p) => {
            if let Some(param_list) = &p.parameter {
                for param in param_list {
                    if let Some(name_value) = &param.name.value {
                        match name_value.as_str() {
                            "viewResource" => {
                                if let Some(resource) = &param.resource {
                                    match serde_json::to_value(resource) {
                                        Ok(json_val) => view_definition = Some(json_val),
                                        Err(e) => return Err(format!("Failed to serialize viewResource: {}", e)),
                                    }
                                }
                            }
                            "viewReference" => {
                                return Err("viewReference parameter is not yet supported".to_string());
                            }
                            "resource" => {
                                if let Some(resource) = &param.resource {
                                    match serde_json::to_value(resource) {
                                        Ok(json_val) => resources.push(json_val),
                                        Err(e) => return Err(format!("Failed to serialize resource: {}", e)),
                                    }
                                }
                            }
                            // TODO: Add support for other parameters
                            "patient" => {
                                // This is a filter parameter, not a resource parameter
                                // Will be implemented when we add filtering support
                            }
                            "group" => {
                                // This is a filter parameter, not a resource parameter
                                // Will be implemented when we add filtering support
                            }
                            "source" => {
                                // This specifies the data source
                                // Will be implemented when we add source selection support
                            }
                            "_format" | "format" => {
                                // Extract format parameter from the value choice type
                                if let Some(value) = &param.value {
                                    match value {
                                        fhir::r5::ParametersParameterValue::Code(code) => {
                                            if let Some(code_value) = &code.value {
                                                format_param = Some(code_value.clone());
                                            }
                                        }
                                        fhir::r5::ParametersParameterValue::String(string) => {
                                            if let Some(string_value) = &string.value {
                                                format_param = Some(string_value.clone());
                                            }
                                        }
                                        _ => {} // Other value types not applicable for _format
                                    }
                                }
                            }
                            "_header" | "header" => {
                                // Extract header parameter from the value choice type (boolean only)
                                if let Some(value) = &param.value {
                                    match value {
                                        fhir::r5::ParametersParameterValue::Boolean(boolean) => {
                                            if let Some(bool_value) = &boolean.value {
                                                header_param = Some(*bool_value);
                                            }
                                        }
                                        _ => return Err("Header parameter must be a boolean value".to_string()),
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        #[cfg(feature = "R6")]
        SofParameters::R6(p) => {
            if let Some(param_list) = &p.parameter {
                for param in param_list {
                    if let Some(name_value) = &param.name.value {
                        match name_value.as_str() {
                            "viewResource" => {
                                if let Some(resource) = &param.resource {
                                    match serde_json::to_value(resource) {
                                        Ok(json_val) => view_definition = Some(json_val),
                                        Err(e) => return Err(format!("Failed to serialize viewResource: {}", e)),
                                    }
                                }
                            }
                            "viewReference" => {
                                return Err("viewReference parameter is not yet supported".to_string());
                            }
                            "resource" => {
                                if let Some(resource) = &param.resource {
                                    match serde_json::to_value(resource) {
                                        Ok(json_val) => resources.push(json_val),
                                        Err(e) => return Err(format!("Failed to serialize resource: {}", e)),
                                    }
                                }
                            }
                            // TODO: Add support for other parameters
                            "patient" => {
                                // This is a filter parameter, not a resource parameter
                                // Will be implemented when we add filtering support
                            }
                            "group" => {
                                // This is a filter parameter, not a resource parameter
                                // Will be implemented when we add filtering support
                            }
                            "source" => {
                                // This specifies the data source
                                // Will be implemented when we add source selection support
                            }
                            "_format" | "format" => {
                                // Extract format parameter from the value choice type
                                if let Some(value) = &param.value {
                                    match value {
                                        fhir::r6::ParametersParameterValue::Code(code) => {
                                            if let Some(code_value) = &code.value {
                                                format_param = Some(code_value.clone());
                                            }
                                        }
                                        fhir::r6::ParametersParameterValue::String(string) => {
                                            if let Some(string_value) = &string.value {
                                                format_param = Some(string_value.clone());
                                            }
                                        }
                                        _ => {} // Other value types not applicable for _format
                                    }
                                }
                            }
                            "_header" | "header" => {
                                // Extract header parameter from the value choice type (boolean only)
                                if let Some(value) = &param.value {
                                    match value {
                                        fhir::r6::ParametersParameterValue::Boolean(boolean) => {
                                            if let Some(bool_value) = &boolean.value {
                                                header_param = Some(*bool_value);
                                            }
                                        }
                                        _ => return Err("Header parameter must be a boolean value".to_string()),
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }
    
    Ok((view_definition, if resources.is_empty() { None } else { Some(resources) }, format_param, header_param))
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
/// * Pagination - Applied using `_count` and `_page` parameters
/// * Format-aware - Handles CSV headers correctly during pagination
///
/// # Note
/// The `_since` parameter is validated but not applied here as it requires
/// filtering at the resource level before transformation.
pub fn apply_result_filtering(
    output_data: Vec<u8>,
    params: &ValidatedRunParams,
) -> Result<Vec<u8>, String> {
    // For now, we'll apply simple pagination and count limiting
    // The _since filtering would need to be implemented at the data source level
    // which is beyond the scope of the current run_view_definition function
    
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
    let output_str = String::from_utf8(output_data)
        .map_err(|e| format!("Invalid UTF-8 in output: {}", e))?;
    
    if params.count.is_none() && params.page.is_none() {
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
                .map(|r| serde_json::to_string(r))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("Failed to serialize filtered NDJSON: {}", e))?
                .join("\n");
            Ok(filtered_ndjson.into_bytes())
        }
        _ => Ok(output_str.into_bytes())
    }
}

/// Apply filtering to CSV output
fn apply_csv_filtering(
    output_data: Vec<u8>,
    params: &ValidatedRunParams,
) -> Result<Vec<u8>, String> {
    let output_str = String::from_utf8(output_data)
        .map_err(|e| format!("Invalid UTF-8 in CSV output: {}", e))?;
    
    if params.count.is_none() && params.page.is_none() {
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

/// Apply pagination to a vector of JSON records
fn apply_pagination_to_records(records: &mut Vec<serde_json::Value>, params: &ValidatedRunParams) {
    let count = params.count.unwrap_or(records.len());
    let page = params.page.unwrap_or(1);
    
    // Calculate offset based on page (1-based)
    let offset = (page - 1) * count;
    
    if offset >= records.len() {
        records.clear();
        return;
    }
    
    let end = std::cmp::min(offset + count, records.len());
    *records = records[offset..end].to_vec();
}

/// Apply pagination to a vector of string lines
fn apply_pagination_to_lines(lines: &mut Vec<&str>, params: &ValidatedRunParams) {
    let count = params.count.unwrap_or(lines.len());
    let page = params.page.unwrap_or(1);
    
    // Calculate offset based on page (1-based)
    let offset = (page - 1) * count;
    
    if offset >= lines.len() {
        lines.clear();
        return;
    }
    
    let end = std::cmp::min(offset + count, lines.len());
    *lines = lines[offset..end].to_vec();
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
            count: Some(10),
            page: Some(2),
            since: Some("2023-01-01T00:00:00Z".to_string()),
            view_reference: None,
            patient: None,
            group: None,
            source: None,
        };
        
        let result = validate_query_params(&params, None).unwrap();
        assert_eq!(result.format, ContentType::Json);
        assert_eq!(result.count, Some(10));
        assert_eq!(result.page, Some(2));
        assert!(result.since.is_some());
    }
    
    #[test]
    fn test_validate_query_params_invalid_count() {
        let params = RunQueryParams {
            format: None,
            header: None,
            count: Some(0),
            page: None,
            since: None,
            view_reference: None,
            patient: None,
            group: None,
            source: None,
        };
        
        let result = validate_query_params(&params, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("_count parameter must be greater than 0"));
    }
    
    #[test]
    fn test_validate_query_params_invalid_page() {
        let params = RunQueryParams {
            format: None,
            header: None,
            count: None,
            page: Some(0),
            since: None,
            view_reference: None,
            patient: None,
            group: None,
            source: None,
        };
        
        let result = validate_query_params(&params, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("_page parameter must be greater than 0"));
    }
    
    #[test]
    fn test_validate_query_params_invalid_since() {
        let params = RunQueryParams {
            format: None,
            header: None,
            count: None,
            page: None,
            since: Some("invalid-date".to_string()),
            view_reference: None,
            patient: None,
            group: None,
            source: None,
        };
        
        let result = validate_query_params(&params, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("_since parameter must be a valid RFC3339 timestamp"));
    }
    
    #[test]
    fn test_apply_csv_filtering() {
        let csv_data = "id,name\n1,John\n2,Jane\n3,Bob\n4,Alice\n".as_bytes().to_vec();
        let params = ValidatedRunParams {
            format: ContentType::CsvWithHeader,
            count: Some(2),
            page: Some(1),
            since: None,
            view_reference: None,
            patient: None,
            group: None,
            source: None,
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
    fn test_apply_csv_filtering_page_2() {
        let csv_data = "id,name\n1,John\n2,Jane\n3,Bob\n4,Alice\n".as_bytes().to_vec();
        let params = ValidatedRunParams {
            format: ContentType::CsvWithHeader,
            count: Some(2),
            page: Some(2),
            since: None,
            view_reference: None,
            patient: None,
            group: None,
            source: None,
        };
        
        let result = apply_csv_filtering(csv_data, &params).unwrap();
        let result_str = String::from_utf8(result).unwrap();
        
        assert!(result_str.contains("id,name"));
        assert!(!result_str.contains("1,John"));
        assert!(!result_str.contains("2,Jane"));
        assert!(result_str.contains("3,Bob"));
        assert!(result_str.contains("4,Alice"));
    }
    
    #[test]
    fn test_apply_json_filtering() {
        let json_data = r#"[{"id":"1","name":"John"},{"id":"2","name":"Jane"},{"id":"3","name":"Bob"}]"#.as_bytes().to_vec();
        let params = ValidatedRunParams {
            format: ContentType::Json,
            count: Some(2),
            page: Some(1),
            since: None,
            view_reference: None,
            patient: None,
            group: None,
            source: None,
        };
        
        let result = apply_json_filtering(json_data, &params).unwrap();
        let result_str = String::from_utf8(result).unwrap();
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&result_str).unwrap();
        
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0]["id"], "1");
        assert_eq!(parsed[1]["id"], "2");
    }
}