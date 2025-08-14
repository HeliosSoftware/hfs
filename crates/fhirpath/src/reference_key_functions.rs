use helios_fhirpath_support::{EvaluationError, EvaluationResult};

/// Implementation of the getResourceKey() function
///
/// Returns the ID of the current resource (e.g., "123" from a Patient with id "123").
///
/// # Arguments
///
/// * `invocation_base` - The resource to get the key for
///
/// # Returns
///
/// * The ID of the resource
/// * Empty if the resource doesn't have an id
pub fn get_resource_key_function(
    invocation_base: &EvaluationResult,
) -> Result<EvaluationResult, EvaluationError> {
    match invocation_base {
        EvaluationResult::Object { map, .. } => {
            // Extract resourceType and id
            let resource_type = map.get("resourceType").and_then(|rt| match rt {
                EvaluationResult::String(s, _) => Some(s.clone()),
                _ => None,
            });

            let id = map.get("id").and_then(|id_val| match id_val {
                EvaluationResult::String(s, _) => Some(s.clone()),
                _ => None,
            });

            match (resource_type, id) {
                (Some(_rt), Some(id_str)) => Ok(EvaluationResult::String(id_str, None)),
                _ => Ok(EvaluationResult::Empty),
            }
        }
        _ => Ok(EvaluationResult::Empty),
    }
}

/// Implementation of the getReferenceKey([type]) function
///
/// Extracts the ID portion from a Reference object, optionally filtering by resource type.
/// The Reference object should have a "reference" field containing a string like
/// "ResourceType/id". This function returns just the ID part (e.g., "123" from "Patient/123").
///
/// # Arguments
///
/// * `invocation_base` - The Reference object to extract the key from
/// * `args` - Optional type filter argument
///
/// # Returns
///
/// * The ID portion of the reference (without the resource type prefix)
/// * Empty if no reference found or type doesn't match
pub fn get_reference_key_function(
    invocation_base: &EvaluationResult,
    args: &[EvaluationResult],
) -> Result<EvaluationResult, EvaluationError> {
    // Check argument count
    if args.len() > 1 {
        return Err(EvaluationError::InvalidArity(
            "Function 'getReferenceKey' expects 0 or 1 argument (type filter)".to_string(),
        ));
    }

    // Extract optional type filter
    let type_filter = if args.is_empty() {
        None
    } else {
        match &args[0] {
            EvaluationResult::String(s, _) => Some(s.clone()),
            EvaluationResult::Empty => {
                // When a bare type identifier evaluates to Empty, treat as no filter
                None
            }
            // Handle type identifiers - extract the type name from type info
            result if result.type_name() == "Type" => {
                // For type arguments like 'Patient', extract the type name
                if let EvaluationResult::Object { map, .. } = result {
                    if let Some(EvaluationResult::String(type_name, _)) = map.get("name") {
                        Some(type_name.clone())
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            _ => {
                return Err(EvaluationError::TypeError(format!(
                    "getReferenceKey type filter must be a string or type, got: {:?}",
                    &args[0]
                )));
            }
        }
    };

    match invocation_base {
        EvaluationResult::Object { map, .. } => {
            // Look for the reference field
            if let Some(reference_value) = map.get("reference") {
                match reference_value {
                    EvaluationResult::String(ref_str, _) => {
                        // Parse the reference string (e.g., "Patient/123")
                        if let Some((resource_type, id)) = parse_reference(ref_str) {
                            // Check type filter if provided
                            if let Some(filter_type) = &type_filter {
                                if resource_type != *filter_type {
                                    return Ok(EvaluationResult::Empty);
                                }
                            }

                            // Return just the ID part as the key
                            Ok(EvaluationResult::String(id, None))
                        } else {
                            Ok(EvaluationResult::Empty)
                        }
                    }
                    _ => Ok(EvaluationResult::Empty),
                }
            } else {
                Ok(EvaluationResult::Empty)
            }
        }
        _ => Ok(EvaluationResult::Empty),
    }
}

/// Parse a FHIR reference string like "Patient/123" into (resource_type, id)
fn parse_reference(reference: &str) -> Option<(String, String)> {
    if let Some(slash_pos) = reference.find('/') {
        let resource_type = reference[..slash_pos].to_string();
        let id = reference[slash_pos + 1..].to_string();
        if !resource_type.is_empty() && !id.is_empty() {
            Some((resource_type, id))
        } else {
            None
        }
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_get_resource_key_function() {
        // Create a test resource
        let mut resource_map = HashMap::new();
        resource_map.insert(
            "resourceType".to_string(),
            EvaluationResult::String("Patient".to_string(), None),
        );
        resource_map.insert(
            "id".to_string(),
            EvaluationResult::String("123".to_string(), None),
        );

        let resource = EvaluationResult::Object {
            map: resource_map,
            type_info: None,
        };

        let result = get_resource_key_function(&resource).unwrap();

        match result {
            EvaluationResult::String(key, _) => {
                assert_eq!(key, "123");
            }
            _ => panic!("Expected string result"),
        }
    }

    #[test]
    fn test_get_reference_key_function() {
        // Create a test reference
        let mut reference_map = HashMap::new();
        reference_map.insert(
            "reference".to_string(),
            EvaluationResult::String("Patient/456".to_string(), None),
        );

        let reference = EvaluationResult::Object {
            map: reference_map,
            type_info: None,
        };

        // Test without type filter
        let result = get_reference_key_function(&reference, &[]).unwrap();

        match result {
            EvaluationResult::String(key, _) => {
                assert_eq!(key, "456");
            }
            _ => panic!("Expected string result"),
        }

        // Test with matching type filter
        let type_filter = EvaluationResult::String("Patient".to_string(), None);
        let result = get_reference_key_function(&reference, &[type_filter]).unwrap();

        match result {
            EvaluationResult::String(key, _) => {
                assert_eq!(key, "456");
            }
            _ => panic!("Expected string result"),
        }

        // Test with non-matching type filter
        let wrong_type_filter = EvaluationResult::String("Observation".to_string(), None);
        let result = get_reference_key_function(&reference, &[wrong_type_filter]).unwrap();

        assert!(matches!(result, EvaluationResult::Empty));
    }

    #[test]
    fn test_parse_reference() {
        assert_eq!(
            parse_reference("Patient/123"),
            Some(("Patient".to_string(), "123".to_string()))
        );

        assert_eq!(
            parse_reference("Observation/obs-001"),
            Some(("Observation".to_string(), "obs-001".to_string()))
        );

        assert_eq!(parse_reference("InvalidReference"), None);
        assert_eq!(parse_reference("/"), None);
        assert_eq!(parse_reference("Patient/"), None);
        assert_eq!(parse_reference("/123"), None);
    }
}
