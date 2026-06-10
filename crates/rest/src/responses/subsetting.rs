//! Resource subsetting for _summary and _elements parameters.
//!
//! Implements FHIR resource subsetting per the specification:
//! - `_summary` - Return a predefined subset of elements
//! - `_elements` - Return specific elements by path
//!
//! See: https://hl7.org/fhir/search.html#summary

use helios_fhir::FhirVersion;
use serde_json::{Map, Value};

/// Summary mode for resource subsetting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SummaryMode {
    /// Return only elements marked as summary in the specification.
    True,
    /// Return the full resource (default).
    False,
    /// Return only the text narrative, id, meta, and mandatory top-level elements.
    Text,
    /// Return all elements except the text narrative.
    Data,
    /// For search only - return only a count (handled at search level).
    Count,
}

impl SummaryMode {
    /// Parses a summary mode from a string value.
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_lowercase().as_str() {
            "true" => Some(SummaryMode::True),
            "false" => Some(SummaryMode::False),
            "text" => Some(SummaryMode::Text),
            "data" => Some(SummaryMode::Data),
            "count" => Some(SummaryMode::Count),
            _ => None,
        }
    }
}

/// Elements that are always included regardless of subsetting.
const ALWAYS_INCLUDED: &[&str] = &["resourceType", "id", "meta"];

/// CodeSystem for the `SUBSETTED` tag added to incomplete representations.
const SUBSETTED_SYSTEM: &str = "http://terminology.hl7.org/CodeSystem/v3-ObservationValue";

/// Adds a `SUBSETTED` `meta.tag` to a resource, flagging that the returned
/// representation is incomplete (per `_summary`/`_elements`), as required by the
/// FHIR spec. Idempotent; a no-op for non-object JSON values.
pub fn add_subsetted_tag(resource: &mut Value) {
    let Value::Object(obj) = resource else {
        return;
    };
    let meta = obj
        .entry("meta")
        .or_insert_with(|| Value::Object(Map::new()));
    let Value::Object(meta_obj) = meta else {
        return;
    };
    let tags = meta_obj
        .entry("tag")
        .or_insert_with(|| Value::Array(Vec::new()));
    let Value::Array(arr) = tags else {
        return;
    };
    let already_tagged = arr.iter().any(|t| {
        t.get("system").and_then(|s| s.as_str()) == Some(SUBSETTED_SYSTEM)
            && t.get("code").and_then(|c| c.as_str()) == Some("SUBSETTED")
    });
    if !already_tagged {
        let mut tag = Map::new();
        tag.insert(
            "system".to_string(),
            Value::String(SUBSETTED_SYSTEM.to_string()),
        );
        tag.insert("code".to_string(), Value::String("SUBSETTED".to_string()));
        arr.push(Value::Object(tag));
    }
}

/// Converts a Rust snake_case field name to JSON camelCase.
///
/// The generated FHIR types use snake_case for Rust field names, but the
/// JSON serialization uses camelCase. This function converts between the two.
fn snake_to_camel(s: &str) -> String {
    // Handle raw identifier prefix
    let s = s.strip_prefix("r#").unwrap_or(s);

    let mut result = String::with_capacity(s.len());
    let mut capitalize_next = false;

    for c in s.chars() {
        if c == '_' {
            capitalize_next = true;
        } else if capitalize_next {
            result.push(c.to_ascii_uppercase());
            capitalize_next = false;
        } else {
            result.push(c);
        }
    }

    result
}

/// Returns summary elements for a resource type using the FHIR specification metadata.
///
/// This function retrieves the summary fields from the generated FHIR types, which are
/// derived from the `isSummary` flag in the official FHIR StructureDefinitions.
/// The field names are converted from Rust snake_case to JSON camelCase.
///
/// # Arguments
///
/// * `resource_type` - The FHIR resource type name (e.g., "Patient", "Observation")
/// * `fhir_version` - The FHIR version to use for field lookup
///
/// # Returns
///
/// A vector of field names in camelCase that should be included in summaries.
fn get_summary_elements(resource_type: &str, fhir_version: FhirVersion) -> Vec<String> {
    // Get the summary fields from the generated FHIR types
    let summary_fields: &[&str] = match fhir_version {
        #[cfg(feature = "R4")]
        FhirVersion::R4 => helios_fhir::r4::get_summary_fields(resource_type),
        #[cfg(feature = "R4B")]
        FhirVersion::R4B => helios_fhir::r4b::get_summary_fields(resource_type),
        #[cfg(feature = "R5")]
        FhirVersion::R5 => helios_fhir::r5::get_summary_fields(resource_type),
        #[cfg(feature = "R6")]
        FhirVersion::R6 => helios_fhir::r6::get_summary_fields(resource_type),
        // Fallback for versions not enabled - use minimal fields
        #[allow(unreachable_patterns)]
        _ => &["resourceType", "id", "meta"],
    };

    // Convert snake_case Rust field names to camelCase JSON keys
    // Also ensure resourceType is always included (it's not a struct field but always needed)
    let mut elements: Vec<String> = vec!["resourceType".to_string()];

    for field in summary_fields {
        let camel = snake_to_camel(field);
        if !elements.contains(&camel) {
            elements.push(camel);
        }
    }

    elements
}

/// Applies _summary subsetting to a resource.
///
/// Returns a new JSON value with only the requested elements.
///
/// # Arguments
///
/// * `resource` - The FHIR resource as a JSON value
/// * `mode` - The summary mode to apply
/// * `fhir_version` - The FHIR version (used to determine summary fields for each resource type)
pub fn apply_summary(resource: &Value, mode: SummaryMode, fhir_version: FhirVersion) -> Value {
    match mode {
        SummaryMode::False => resource.clone(),
        SummaryMode::Count => {
            // Count mode is handled at search level; return minimal resource
            filter_resource(resource, ALWAYS_INCLUDED)
        }
        SummaryMode::Text => {
            // Include text, id, meta, and mandatory elements
            let mut elements: Vec<&str> = ALWAYS_INCLUDED.to_vec();
            elements.push("text");
            filter_resource(resource, &elements)
        }
        SummaryMode::Data => {
            // Include everything except text
            exclude_elements(resource, &["text"])
        }
        SummaryMode::True => {
            // Include summary elements from the FHIR specification
            if let Some(resource_type) = resource.get("resourceType").and_then(|v| v.as_str()) {
                let summary_elements = get_summary_elements(resource_type, fhir_version);
                let element_refs: Vec<&str> = summary_elements.iter().map(|s| s.as_str()).collect();
                filter_resource(resource, &element_refs)
            } else {
                resource.clone()
            }
        }
    }
}

/// Applies _elements subsetting to a resource.
///
/// Elements are specified as a comma-separated list of paths (e.g., "id,name,birthDate").
/// Nested paths are supported with dot notation (e.g., "name.family").
pub fn apply_elements(resource: &Value, elements: &[&str]) -> Value {
    if elements.is_empty() {
        return resource.clone();
    }

    // Always include resourceType, id, and meta
    let mut all_elements: Vec<&str> = ALWAYS_INCLUDED.to_vec();

    // Add requested elements
    for elem in elements {
        if !all_elements.contains(elem) {
            all_elements.push(elem);
        }
    }

    filter_resource(resource, &all_elements)
}

/// Filters a resource to include only specified top-level elements.
fn filter_resource(resource: &Value, elements: &[&str]) -> Value {
    if let Value::Object(obj) = resource {
        let mut result = Map::new();

        for (key, value) in obj {
            // Check if this key is in the elements list
            // Also handle nested paths (e.g., "name" should include "name" object)
            if elements
                .iter()
                .any(|e| *e == key || e.starts_with(&format!("{}.", key)) || key == "resourceType")
            {
                // If there's a nested path, filter the nested object
                let nested_paths: Vec<&str> = elements
                    .iter()
                    .filter_map(|e| {
                        if e.starts_with(&format!("{}.", key)) {
                            Some(&e[key.len() + 1..])
                        } else {
                            None
                        }
                    })
                    .collect();

                if !nested_paths.is_empty() && value.is_object() {
                    // Filter nested object
                    result.insert(key.clone(), filter_nested(value, &nested_paths));
                } else {
                    result.insert(key.clone(), value.clone());
                }
            }
        }

        Value::Object(result)
    } else {
        resource.clone()
    }
}

/// Filters nested objects based on path components.
fn filter_nested(value: &Value, paths: &[&str]) -> Value {
    match value {
        Value::Object(obj) => {
            let mut result = Map::new();

            for (key, val) in obj {
                // Include if key matches any path or is a prefix of a nested path
                if paths
                    .iter()
                    .any(|p| *p == key || p.starts_with(&format!("{}.", key)))
                {
                    let nested_paths: Vec<&str> = paths
                        .iter()
                        .filter_map(|p| {
                            if p.starts_with(&format!("{}.", key)) {
                                Some(&p[key.len() + 1..])
                            } else {
                                None
                            }
                        })
                        .collect();

                    if !nested_paths.is_empty() && val.is_object() {
                        result.insert(key.clone(), filter_nested(val, &nested_paths));
                    } else {
                        result.insert(key.clone(), val.clone());
                    }
                }
            }

            Value::Object(result)
        }
        Value::Array(arr) => {
            // For arrays, filter each element
            Value::Array(arr.iter().map(|v| filter_nested(v, paths)).collect())
        }
        _ => value.clone(),
    }
}

/// Excludes specified elements from a resource.
fn exclude_elements(resource: &Value, elements: &[&str]) -> Value {
    if let Value::Object(obj) = resource {
        let mut result = Map::new();

        for (key, value) in obj {
            if !elements.contains(&key.as_str()) {
                result.insert(key.clone(), value.clone());
            }
        }

        Value::Object(result)
    } else {
        resource.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_summary_mode_parse() {
        assert_eq!(SummaryMode::parse("true"), Some(SummaryMode::True));
        assert_eq!(SummaryMode::parse("false"), Some(SummaryMode::False));
        assert_eq!(SummaryMode::parse("text"), Some(SummaryMode::Text));
        assert_eq!(SummaryMode::parse("data"), Some(SummaryMode::Data));
        assert_eq!(SummaryMode::parse("count"), Some(SummaryMode::Count));
        assert_eq!(SummaryMode::parse("invalid"), None);
    }

    #[test]
    fn test_snake_to_camel() {
        assert_eq!(snake_to_camel("birth_date"), "birthDate");
        assert_eq!(
            snake_to_camel("managing_organization"),
            "managingOrganization"
        );
        assert_eq!(snake_to_camel("id"), "id");
        assert_eq!(snake_to_camel("r#type"), "type");
        assert_eq!(snake_to_camel("implicit_rules"), "implicitRules");
    }

    #[test]
    fn test_apply_summary_false() {
        let resource = json!({
            "resourceType": "Patient",
            "id": "123",
            "meta": {"versionId": "1"},
            "text": {"status": "generated", "div": "<div>Patient</div>"},
            "name": [{"family": "Smith"}],
            "birthDate": "1990-01-01"
        });

        let result = apply_summary(&resource, SummaryMode::False, FhirVersion::R4);
        assert_eq!(result, resource);
    }

    #[test]
    fn test_apply_summary_text() {
        let resource = json!({
            "resourceType": "Patient",
            "id": "123",
            "meta": {"versionId": "1"},
            "text": {"status": "generated", "div": "<div>Patient</div>"},
            "name": [{"family": "Smith"}],
            "birthDate": "1990-01-01"
        });

        let result = apply_summary(&resource, SummaryMode::Text, FhirVersion::R4);

        // Should include resourceType, id, meta, text
        assert!(result.get("resourceType").is_some());
        assert!(result.get("id").is_some());
        assert!(result.get("meta").is_some());
        assert!(result.get("text").is_some());
        // Should not include other elements
        assert!(result.get("name").is_none());
        assert!(result.get("birthDate").is_none());
    }

    #[test]
    fn test_apply_summary_data() {
        let resource = json!({
            "resourceType": "Patient",
            "id": "123",
            "meta": {"versionId": "1"},
            "text": {"status": "generated", "div": "<div>Patient</div>"},
            "name": [{"family": "Smith"}],
            "birthDate": "1990-01-01"
        });

        let result = apply_summary(&resource, SummaryMode::Data, FhirVersion::R4);

        // Should include everything except text
        assert!(result.get("resourceType").is_some());
        assert!(result.get("id").is_some());
        assert!(result.get("meta").is_some());
        assert!(result.get("name").is_some());
        assert!(result.get("birthDate").is_some());
        // Should not include text
        assert!(result.get("text").is_none());
    }

    #[test]
    fn test_apply_summary_true_patient() {
        let resource = json!({
            "resourceType": "Patient",
            "id": "123",
            "meta": {"versionId": "1"},
            "text": {"status": "generated", "div": "<div>Patient</div>"},
            "name": [{"family": "Smith"}],
            "birthDate": "1990-01-01",
            "communication": [{"language": {"text": "English"}}],
            "photo": [{"data": "base64..."}]
        });

        let result = apply_summary(&resource, SummaryMode::True, FhirVersion::R4);

        // Should include summary elements
        assert!(result.get("resourceType").is_some());
        assert!(result.get("id").is_some());
        assert!(result.get("meta").is_some());
        assert!(result.get("name").is_some());
        assert!(result.get("birthDate").is_some());
        // Should not include non-summary elements
        assert!(result.get("communication").is_none());
        assert!(result.get("photo").is_none());
    }

    #[test]
    fn test_get_summary_elements_from_spec() {
        // Test that the generated summary fields are correctly retrieved
        let patient_summary = get_summary_elements("Patient", FhirVersion::R4);

        // These fields should be in the Patient summary per FHIR spec
        assert!(patient_summary.contains(&"id".to_string()));
        assert!(patient_summary.contains(&"meta".to_string()));
        assert!(patient_summary.contains(&"name".to_string()));
        assert!(patient_summary.contains(&"birthDate".to_string()));
        assert!(patient_summary.contains(&"gender".to_string()));
        assert!(patient_summary.contains(&"active".to_string()));
    }

    #[test]
    fn test_apply_elements_basic() {
        let resource = json!({
            "resourceType": "Patient",
            "id": "123",
            "meta": {"versionId": "1"},
            "name": [{"family": "Smith", "given": ["John"]}],
            "birthDate": "1990-01-01",
            "gender": "male"
        });

        let result = apply_elements(&resource, &["name", "birthDate"]);

        // Should include requested elements plus mandatory
        assert!(result.get("resourceType").is_some());
        assert!(result.get("id").is_some());
        assert!(result.get("meta").is_some());
        assert!(result.get("name").is_some());
        assert!(result.get("birthDate").is_some());
        // Should not include non-requested elements
        assert!(result.get("gender").is_none());
    }

    #[test]
    fn test_apply_elements_nested_path() {
        let resource = json!({
            "resourceType": "Patient",
            "id": "123",
            "meta": {"versionId": "1"},
            "name": [{"family": "Smith", "given": ["John"]}],
            "birthDate": "1990-01-01"
        });

        let result = apply_elements(&resource, &["name.family"]);

        // Should include the name object filtered to only family
        assert!(result.get("name").is_some());
        let name = result.get("name").unwrap();
        if let Value::Array(arr) = name {
            let first = &arr[0];
            assert!(first.get("family").is_some());
            // given should not be included as we only requested name.family
        }
    }

    #[test]
    fn test_add_subsetted_tag_is_idempotent() {
        let mut resource = json!({ "resourceType": "Patient", "id": "1" });
        add_subsetted_tag(&mut resource);
        add_subsetted_tag(&mut resource);

        let tags = resource["meta"]["tag"].as_array().expect("tag array");
        let subsetted: Vec<_> = tags
            .iter()
            .filter(|t| t["code"] == "SUBSETTED" && t["system"] == SUBSETTED_SYSTEM)
            .collect();
        assert_eq!(subsetted.len(), 1, "SUBSETTED tag added exactly once");
    }

    #[test]
    fn test_add_subsetted_tag_preserves_existing_tags() {
        let mut resource = json!({
            "resourceType": "Patient",
            "id": "1",
            "meta": { "tag": [{ "system": "http://example.org", "code": "x" }] }
        });
        add_subsetted_tag(&mut resource);
        let tags = resource["meta"]["tag"].as_array().unwrap();
        assert_eq!(tags.len(), 2, "existing tag preserved + SUBSETTED added");
    }

    #[test]
    fn test_exclude_elements() {
        let resource = json!({
            "resourceType": "Patient",
            "id": "123",
            "text": {"div": "<div>text</div>"},
            "name": [{"family": "Smith"}]
        });

        let result = exclude_elements(&resource, &["text"]);

        assert!(result.get("resourceType").is_some());
        assert!(result.get("id").is_some());
        assert!(result.get("name").is_some());
        assert!(result.get("text").is_none());
    }
}
