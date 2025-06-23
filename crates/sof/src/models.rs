//! Request and response models for the SQL-on-FHIR server
//!
//! This module contains data structures for handling HTTP requests and responses,
//! including parameter parsing and content type negotiation.

use serde::{Deserialize, Serialize};
use sof::ContentType;

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
}

/// Parameters for ViewDefinition/$run operation
#[derive(Debug, Deserialize)]
pub struct RunParameters {
    /// The FHIR Parameters resource
    #[serde(rename = "resourceType")]
    pub resource_type: String,
    
    /// The parameter entries
    pub parameter: Vec<Parameter>,
}

/// Individual parameter in a Parameters resource
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Parameter {
    /// Parameter name
    pub name: String,
    
    /// Reference value (for viewReference, patient, group)
    pub value_reference: Option<Reference>,
    
    /// String value (for format)
    pub value_string: Option<String>,
    
    /// Resource value (for viewResource, patient resources)
    pub resource: Option<serde_json::Value>,
    
    /// Nested parameters
    pub part: Option<Vec<Parameter>>,
}

/// FHIR Reference type
#[derive(Debug, Deserialize, Serialize)]
pub struct Reference {
    /// Reference string (e.g., "ViewDefinition/123")
    pub reference: Option<String>,
    
    /// Display text
    pub display: Option<String>,
}

/// Parse content type from Accept header and query parameters
pub fn parse_content_type(
    accept_header: Option<&str>,
    format_param: Option<&str>,
    header_param: Option<&str>,
) -> Result<ContentType, String> {
    // Query parameter takes precedence over Accept header
    let content_type_str = format_param
        .or(accept_header)
        .unwrap_or("application/json");
    
    // Handle CSV header parameter
    let content_type_str = if content_type_str == "text/csv" {
        match header_param {
            Some("absent") => "text/csv",
            Some("present") | None => "text/csv;header=present",
            _ => return Err(format!("Invalid _header parameter: {}", header_param.unwrap())),
        }
    } else {
        content_type_str
    };
    
    ContentType::from_string(content_type_str)
        .map_err(|e| e.to_string())
}

/// Extract ViewDefinition and Bundle from Parameters
pub fn extract_run_parameters(
    params: RunParameters,
) -> Result<(Option<serde_json::Value>, Option<Vec<serde_json::Value>>), String> {
    let mut view_definition = None;
    let mut resources = Vec::new();
    
    for param in params.parameter {
        match param.name.as_str() {
            "viewResource" => {
                if let Some(resource) = param.resource {
                    view_definition = Some(resource);
                }
            }
            "viewReference" => {
                // In a real implementation, this would fetch the ViewDefinition
                // For now, we don't support references
                return Err("viewReference parameter is not yet supported".to_string());
            }
            "patient" => {
                if let Some(resource) = param.resource {
                    resources.push(resource);
                }
            }
            _ => {
                // Ignore other parameters for now
            }
        }
    }
    
    Ok((view_definition, if resources.is_empty() { None } else { Some(resources) }))
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
        
        // Test CSV with header parameter
        assert_eq!(
            parse_content_type(None, Some("text/csv"), Some("absent")).unwrap(),
            ContentType::Csv
        );
    }
}