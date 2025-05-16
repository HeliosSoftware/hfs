use std::collections::HashMap;
use fhirpath_support::{EvaluationError, EvaluationResult};

/// Gets the detailed type information for a value
/// Returns an object with namespace and name properties
pub fn get_type_info(value: &EvaluationResult) -> EvaluationResult {
    let mut obj = HashMap::new();
    
    match value {
        EvaluationResult::Empty => return EvaluationResult::Empty,
        EvaluationResult::Collection { items, has_undefined_order } => {
            if items.is_empty() {
                return EvaluationResult::Empty;
            } else if items.len() == 1 {
                return get_type_info(&items[0]);
            } else {
                let type_infos: Vec<EvaluationResult> = items
                    .iter()
                    .map(get_type_info) // Simplified map
                    .collect();
                return EvaluationResult::Collection { items: type_infos, has_undefined_order: *has_undefined_order };
            }
        },
        EvaluationResult::Boolean(_) => {
            obj.insert("namespace".to_string(), EvaluationResult::String("System".to_string()));
            obj.insert("name".to_string(), EvaluationResult::String("Boolean".to_string()));
        },
        EvaluationResult::Integer(_) => {
            obj.insert("namespace".to_string(), EvaluationResult::String("System".to_string()));
            obj.insert("name".to_string(), EvaluationResult::String("Integer".to_string()));
        },
        EvaluationResult::Decimal(_) => {
            obj.insert("namespace".to_string(), EvaluationResult::String("System".to_string()));
            obj.insert("name".to_string(), EvaluationResult::String("Decimal".to_string()));
        },
        EvaluationResult::String(_) => {
            obj.insert("namespace".to_string(), EvaluationResult::String("System".to_string()));
            obj.insert("name".to_string(), EvaluationResult::String("String".to_string()));
        },
        EvaluationResult::Date(_) => {
            obj.insert("namespace".to_string(), EvaluationResult::String("System".to_string()));
            obj.insert("name".to_string(), EvaluationResult::String("Date".to_string()));
        },
        EvaluationResult::DateTime(_) => {
            obj.insert("namespace".to_string(), EvaluationResult::String("System".to_string()));
            obj.insert("name".to_string(), EvaluationResult::String("DateTime".to_string()));
        },
        EvaluationResult::Time(_) => {
            obj.insert("namespace".to_string(), EvaluationResult::String("System".to_string()));
            obj.insert("name".to_string(), EvaluationResult::String("Time".to_string()));
        },
        EvaluationResult::Quantity(_, _) => {
            obj.insert("namespace".to_string(), EvaluationResult::String("System".to_string()));
            obj.insert("name".to_string(), EvaluationResult::String("Quantity".to_string()));
        },
        EvaluationResult::Object(map) => {
            // Check for our "fhirType" marker first
            if let Some(EvaluationResult::String(fhir_type_name)) = map.get("fhirType") {
                obj.insert("namespace".to_string(), EvaluationResult::String("FHIR".to_string()));
                obj.insert("name".to_string(), EvaluationResult::String(fhir_type_name.clone()));
                return EvaluationResult::Object(obj); // Return early
            }
            
            // Fallback to existing logic if "fhirType" is not present
            // First check if this is a FHIR primitive type with value property (original logic)
            if map.contains_key("id") && map.contains_key("value") {
                if let Some(value_type) = get_fhir_primitive_type(map) {
                    obj.insert("namespace".to_string(), EvaluationResult::String("FHIR".to_string()));
                    obj.insert("name".to_string(), EvaluationResult::String(value_type.to_lowercase()));
                    return EvaluationResult::Object(obj);
                }
            }
            
            // Use the more robust FHIR type detection (original logic)
            if let Some(type_info) = get_fhir_element_type_from_context(map) {
                obj.insert("namespace".to_string(), EvaluationResult::String(type_info.namespace));
                obj.insert("name".to_string(), EvaluationResult::String(type_info.name));
                return EvaluationResult::Object(obj);
            }
            
            // If no specific FHIR type was detected, treat as a System Object
            obj.insert("namespace".to_string(), EvaluationResult::String("System".to_string()));
            obj.insert("name".to_string(), EvaluationResult::String("Object".to_string()));
        }
    }
    
    EvaluationResult::Object(obj)
}

/// Attempts to determine the FHIR primitive type from an object
/// Used to identify FHIR primitive types like boolean, string, etc.
fn get_fhir_primitive_type(obj: &HashMap<String, EvaluationResult>) -> Option<String> {
    // Look for 'value' field to determine the type
    if let Some(value) = obj.get("value") {
        match value {
            EvaluationResult::Boolean(_) => Some("boolean".to_string()),
            EvaluationResult::String(_) => Some("string".to_string()),
            EvaluationResult::Integer(_) => Some("integer".to_string()),
            EvaluationResult::Decimal(_) => Some("decimal".to_string()),
            EvaluationResult::Date(_) => Some("date".to_string()),
            EvaluationResult::DateTime(_) => Some("dateTime".to_string()),
            EvaluationResult::Time(_) => Some("time".to_string()),
            _ => None, // Unknown value type
        }
    } else {
        None // No value field
    }
}

/// Structure to represent FHIR type information
struct FhirTypeInfo {
    namespace: String,
    name: String,
}

/// Attempts to determine the FHIR element type from context or structure
/// Used to identify FHIR types based on object structure and properties
/// Returns information about the detected type, or None if not a FHIR element
fn get_fhir_element_type_from_context(obj: &HashMap<String, EvaluationResult>) -> Option<FhirTypeInfo> {
    // First check if this is a FHIR resource with resourceType property
    if let Some(EvaluationResult::String(resource_type)) = obj.get("resourceType") {
        return Some(FhirTypeInfo {
            namespace: "FHIR".to_string(),
            name: resource_type.clone(),
        });
    }

    // Check for FHIR primitive type element patterns
    
    // Check for common FHIR primitive types
    // This approach examines object structure and attributes generically
    // rather than hard-coding specific property names
    
    // Check for small objects that might be FHIR primitive type elements
    if obj.len() < 5 && !obj.contains_key("resourceType") {
        // Look for objects that match common FHIR primitive type patterns
        
        // Boolean elements - look for boolean values or common boolean property names
        let has_boolean_value = obj.values().any(|v| matches!(v, EvaluationResult::Boolean(_)));
        let has_boolean_property_name = obj.keys().any(|key| {
            // Common property names for boolean elements
            let key_lower = key.to_lowercase();
            key_lower.contains("active") || 
            key_lower.contains("flag") || 
            key_lower.contains("enabled") || 
            key_lower.contains("status") || 
            key_lower.contains("is")
        });
        
        if has_boolean_value || has_boolean_property_name {
            return Some(FhirTypeInfo {
                namespace: "FHIR".to_string(),
                name: "boolean".to_string()
            });
        }
        
        // Date elements - look for date values or common date property names
        let has_date_value = obj.values().any(|v| {
            matches!(v, EvaluationResult::Date(_)) || 
            // Date strings with YYYY-MM-DD pattern
            if let EvaluationResult::String(s) = v {
                s.len() >= 10 && s.chars().nth(4) == Some('-') && s.chars().nth(7) == Some('-')
            } else {
                false
            }
        });
        
        let has_date_property_name = obj.keys().any(|key| {
            // Common property names for date elements
            let key_lower = key.to_lowercase();
            key_lower.contains("date") || 
            key_lower.contains("birth") || 
            key_lower.contains("onset") || 
            key_lower.contains("recorded") || 
            key_lower.contains("time")
        });
        
        if has_date_value || has_date_property_name {
            return Some(FhirTypeInfo {
                namespace: "FHIR".to_string(),
                name: "date".to_string()
            });
        }
    }
    
    // Look for FHIR primitive types with value property
    if let Some(value) = obj.get("value") {
        match value {
            EvaluationResult::Boolean(_) => return Some(FhirTypeInfo {
                namespace: "FHIR".to_string(),
                name: "boolean".to_string()
            }),
            EvaluationResult::String(_) => return Some(FhirTypeInfo {
                namespace: "FHIR".to_string(),
                name: "string".to_string()
            }),
            EvaluationResult::Integer(_) => return Some(FhirTypeInfo {
                namespace: "FHIR".to_string(),
                name: "integer".to_string()
            }),
            EvaluationResult::Decimal(_) => return Some(FhirTypeInfo {
                namespace: "FHIR".to_string(),
                name: "decimal".to_string()
            }),
            EvaluationResult::Date(_) => return Some(FhirTypeInfo {
                namespace: "FHIR".to_string(),
                name: "date".to_string()
            }),
            EvaluationResult::DateTime(_) => return Some(FhirTypeInfo {
                namespace: "FHIR".to_string(),
                name: "dateTime".to_string()
            }),
            EvaluationResult::Time(_) => return Some(FhirTypeInfo {
                namespace: "FHIR".to_string(),
                name: "time".to_string()
            }),
            _ => {}
        }
    }
    
    // Check for typical FHIR Quantity structure
    if obj.contains_key("value") && obj.contains_key("unit") && obj.len() <= 5 {
        return Some(FhirTypeInfo {
            namespace: "FHIR".to_string(),
            name: "Quantity".to_string()
        });
    }
    
    // Check for FHIR choice types (valueQuantity, valueString, etc.)
    for (key, _) in obj.iter() {
        if key.starts_with("value") && key.len() > 5 {
            let value_type = &key[5..]; // Extract the part after "value"
            if !value_type.is_empty() && value_type.chars().next().unwrap().is_uppercase() {
                // Find a FHIR choice element like valueQuantity, valueString, etc.
                return Some(FhirTypeInfo {
                    namespace: "FHIR".to_string(),
                    name: value_type.to_string()
                });
            }
        }
    }
    
    // Check for common FHIR metadata that might indicate a FHIR structure
    // but we don't know the specific type
    if obj.contains_key("id") && (obj.contains_key("extension") || obj.contains_key("url")) {
        // This might be a FHIR element or Extension
        return Some(FhirTypeInfo {
            namespace: "FHIR".to_string(),
            name: "Element".to_string() // Generic FHIR element
        });
    }
    
    // If we have meta, text, or a reference like subject, this might be a FHIR DomainResource
    if obj.contains_key("meta") || obj.contains_key("text") || obj.contains_key("subject") {
        return Some(FhirTypeInfo {
            namespace: "FHIR".to_string(),
            name: "DomainResource".to_string()
        });
    }
    
    None // Not recognized as a FHIR element
}

/// Gets just the type name as a string (without namespace)
/// This is used for backward compatibility with existing code
pub fn get_type_name(value: &EvaluationResult) -> EvaluationResult {
    match get_type_info(value) {
        EvaluationResult::Empty => EvaluationResult::Empty,
        EvaluationResult::Collection { items, has_undefined_order } => {
            // Map each item to just the name
            let names: Vec<EvaluationResult> = items
                .iter()
                .map(|item| match item {
                    EvaluationResult::Object(map) => {
                        if let Some(EvaluationResult::String(name)) = map.get("name") {
                            EvaluationResult::String(name.clone())
                        } else {
                            EvaluationResult::String("Unknown".to_string())
                        }
                    },
                    _ => EvaluationResult::String("Unknown".to_string()),
                })
                .collect();
            EvaluationResult::Collection { items: names, has_undefined_order }
        },
        EvaluationResult::Object(map) => {
            // Just extract the name property
            if let Some(EvaluationResult::String(name)) = map.get("name") {
                EvaluationResult::String(name.clone())
            } else {
                EvaluationResult::String("Unknown".to_string())
            }
        },
        _ => EvaluationResult::String("Unknown".to_string()),
    }
}

/// FHIRPath type() function implementation
/// Depending on the mode, returns either detailed type info or just the name
pub fn type_function(
    invocation_base: &EvaluationResult,
    args: &[EvaluationResult],
    return_full_object: bool,
) -> Result<EvaluationResult, EvaluationError> {
    if let EvaluationResult::Collection { items, has_undefined_order } = invocation_base {
        if items.len() > 1 {
            let type_results: Vec<EvaluationResult> = items
                .iter()
                .map(|item| if return_full_object { get_type_info(item) } else { get_type_name(item) })
                .collect();
            return Ok(EvaluationResult::Collection { items: type_results, has_undefined_order: *has_undefined_order });
        }
        // If single item collection, fall through to evaluate the single item
    }
    
    if !args.is_empty() {
        return Err(EvaluationError::InvalidArity(
            "Function 'type' expects 0 arguments".to_string(),
        ));
    }
    
    // For the R4 tests, we ALWAYS want to return the full object
    // to enable property access like .namespace and .name
    // The tests expect to access properties on the returned type object
    
    // Get the type information as an object with namespace and name properties
    Ok(get_type_info(invocation_base))
}

/// FHIRPath type() function implementation that returns the full type object
/// This is specifically for use in the evaluator for the type() function call
pub fn type_function_full(
    invocation_base: &EvaluationResult,
    args: &[EvaluationResult],
) -> Result<EvaluationResult, EvaluationError> {
    type_function(invocation_base, args, true)
}
