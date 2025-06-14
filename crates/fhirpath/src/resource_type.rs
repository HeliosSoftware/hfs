use fhirpath_support::{EvaluationError, EvaluationResult};
use crate::parser::TypeSpecifier;
use crate::fhir_type_hierarchy::capitalize_first_letter;
use crate::evaluator::EvaluationContext;
use fhir::{FhirVersion, FhirResourceTypeProvider};

/// Handles type operations for FHIR resources, supporting is/as operators.
/// This module provides enhanced support for handling FHIR resource types
/// in FHIRPath expressions, allowing for proper type checking and filtering
/// based on resource types.

/// Checks if a type name is a resource type for the given FHIR version
/// 
/// # Arguments
/// 
/// * `type_name` - The type name to check (e.g., "Patient", "Observation")
/// * `fhir_version` - The FHIR version to check against
/// 
/// # Returns
/// 
/// * `true` if the type is a resource type in the given FHIR version, `false` otherwise
pub fn is_resource_type_for_version(type_name: &str, fhir_version: &FhirVersion) -> bool {
    match fhir_version {
        #[cfg(feature = "R4")]
        FhirVersion::R4 => fhir::r4::Resource::is_resource_type(type_name),
        #[cfg(feature = "R4B")]
        FhirVersion::R4B => fhir::r4b::Resource::is_resource_type(type_name),
        #[cfg(feature = "R5")]
        FhirVersion::R5 => fhir::r5::Resource::is_resource_type(type_name),
        #[cfg(feature = "R6")]
        FhirVersion::R6 => fhir::r6::Resource::is_resource_type(type_name),
        #[allow(unreachable_patterns)]
        _ => false, // For versions not enabled by feature flags
    }
}

/// Determines if a value is of a specified FHIR or System type (context-aware version)
/// 
/// # Arguments
/// 
/// * `value` - The value to check
/// * `type_spec` - The type specifier to check against
/// * `context` - The evaluation context containing FHIR version information
/// 
/// # Returns
/// 
/// * `true` if the value is of the specified type, `false` otherwise
pub fn is_of_type_with_context(value: &EvaluationResult, type_spec: &TypeSpecifier, context: &EvaluationContext) -> Result<bool, EvaluationError> {
    let (target_namespace, target_type) = extract_namespace_and_type_with_context(type_spec, context)?;
    match value {
        EvaluationResult::Boolean(_, type_info) => {
            if let Some(type_info) = type_info {
                check_type_match(&Some(type_info.namespace.clone()), &type_info.name, &target_namespace, &target_type)
            } else {
                // Default to System.Boolean for boolean values
                check_type_match(&Some("System".to_string()), "Boolean", &target_namespace, &target_type)
            }
        },
        EvaluationResult::Integer(_, type_info) => {
            if let Some(type_info) = type_info {
                check_type_match(&Some(type_info.namespace.clone()), &type_info.name, &target_namespace, &target_type)
            } else {
                // Default to System.Integer for integer values
                check_type_match(&Some("System".to_string()), "Integer", &target_namespace, &target_type)
            }
        },
        EvaluationResult::Decimal(_, type_info) => {
            if let Some(type_info) = type_info {
                check_type_match(&Some(type_info.namespace.clone()), &type_info.name, &target_namespace, &target_type)
            } else {
                // Default to System.Decimal for decimal values
                check_type_match(&Some("System".to_string()), "Decimal", &target_namespace, &target_type)
            }
        },
        EvaluationResult::String(_, type_info) => {
            if let Some(type_info) = type_info {
                check_type_match(&Some(type_info.namespace.clone()), &type_info.name, &target_namespace, &target_type)
            } else {
                // Default to System.String for string values
                check_type_match(&Some("System".to_string()), "String", &target_namespace, &target_type)
            }
        },
        EvaluationResult::Date(_, type_info) => {
            if let Some(type_info) = type_info {
                check_type_match(&Some(type_info.namespace.clone()), &type_info.name, &target_namespace, &target_type)
            } else {
                // Default to System.Date for date values
                check_type_match(&Some("System".to_string()), "Date", &target_namespace, &target_type)
            }
        },
        EvaluationResult::DateTime(_, type_info) => {
            if let Some(type_info) = type_info {
                check_type_match(&Some(type_info.namespace.clone()), &type_info.name, &target_namespace, &target_type)
            } else {
                // Default to System.DateTime for datetime values
                check_type_match(&Some("System".to_string()), "DateTime", &target_namespace, &target_type)
            }
        },
        EvaluationResult::Time(_, type_info) => {
            if let Some(type_info) = type_info {
                check_type_match(&Some(type_info.namespace.clone()), &type_info.name, &target_namespace, &target_type)
            } else {
                // Default to System.Time for time values
                check_type_match(&Some("System".to_string()), "Time", &target_namespace, &target_type)
            }
        },
        EvaluationResult::Quantity(_, _, type_info) => {
            if let Some(type_info) = type_info {
                check_type_match(&Some(type_info.namespace.clone()), &type_info.name, &target_namespace, &target_type)
            } else {
                // Default to System.Quantity for quantity values
                check_type_match(&Some("System".to_string()), "Quantity", &target_namespace, &target_type)
            }
        },
        EvaluationResult::Object { map, type_info, .. } => {
            // First check if there's type_info available
            if let Some(type_info) = type_info {
                check_type_match(&Some(type_info.namespace.clone()), &type_info.name, &target_namespace, &target_type)
            } else if let Some(resource_type_value) = map.get("resourceType") {
                // For FHIR resources, check the resourceType property
                if let EvaluationResult::String(resource_type, _) = resource_type_value {
                    // Check if the resource type matches the target type
                    check_type_match(&Some("FHIR".to_string()), resource_type, &target_namespace, &target_type)
                } else {
                    Ok(false)
                }
            } else {
                // Default to System.Object for object values
                check_type_match(&Some("System".to_string()), "Object", &target_namespace, &target_type)
            }
        },
        EvaluationResult::Collection { type_info, .. } => {
            if let Some(type_info) = type_info {
                check_type_match(&Some(type_info.namespace.clone()), &type_info.name, &target_namespace, &target_type)
            } else {
                // Default to System.Collection for collection values
                check_type_match(&Some("System".to_string()), "Collection", &target_namespace, &target_type)
            }
        },
        EvaluationResult::Empty => {
            // Empty values don't match any specific type
            Ok(false)
        },
        #[cfg(not(any(feature = "R4", feature = "R4B")))]
        EvaluationResult::Integer64(_, type_info) => {
            if let Some(type_info) = type_info {
                check_type_match(&Some(type_info.namespace.clone()), &type_info.name, &target_namespace, &target_type)
            } else {
                // Default to System.Integer64 for integer64 values
                check_type_match(&Some("System".to_string()), "Integer64", &target_namespace, &target_type)
            }
        },
    }
}

/// Determines if a value is of a specified FHIR or System type with cross-namespace matching (for ofType operations)
/// 
/// # Arguments
/// 
/// * `value` - The value to check
/// * `type_spec` - The type specifier to check against
/// 
/// # Returns
/// 
/// * `true` if the value is of the specified type, `false` otherwise
pub fn is_of_type_for_of_type(value: &EvaluationResult, type_spec: &TypeSpecifier) -> Result<bool, EvaluationError> {
    let (target_namespace, target_type) = extract_namespace_and_type(type_spec)?;
    
    match value {
        EvaluationResult::Boolean(_, type_info) => {
            if let Some(type_info) = type_info {
                check_type_match_with_cross_namespace(&Some(type_info.namespace.clone()), &type_info.name, &target_namespace, &target_type, true)
            } else {
                // Default to System.Boolean for boolean values
                check_type_match_with_cross_namespace(&Some("System".to_string()), "Boolean", &target_namespace, &target_type, true)
            }
        },
        EvaluationResult::Integer(_, type_info) => {
            if let Some(type_info) = type_info {
                check_type_match_with_cross_namespace(&Some(type_info.namespace.clone()), &type_info.name, &target_namespace, &target_type, true)
            } else {
                // Default to System.Integer for integer values
                check_type_match_with_cross_namespace(&Some("System".to_string()), "Integer", &target_namespace, &target_type, true)
            }
        },
        EvaluationResult::Decimal(_, type_info) => {
            if let Some(type_info) = type_info {
                check_type_match_with_cross_namespace(&Some(type_info.namespace.clone()), &type_info.name, &target_namespace, &target_type, true)
            } else {
                // Default to System.Decimal for decimal values
                check_type_match_with_cross_namespace(&Some("System".to_string()), "Decimal", &target_namespace, &target_type, true)
            }
        },
        EvaluationResult::String(_, type_info) => {
            if let Some(type_info) = type_info {
                check_type_match_with_cross_namespace(&Some(type_info.namespace.clone()), &type_info.name, &target_namespace, &target_type, true)
            } else {
                // Default to System.String for string values
                check_type_match_with_cross_namespace(&Some("System".to_string()), "String", &target_namespace, &target_type, true)
            }
        },
        EvaluationResult::Date(_, type_info) => {
            if let Some(type_info) = type_info {
                check_type_match_with_cross_namespace(&Some(type_info.namespace.clone()), &type_info.name, &target_namespace, &target_type, true)
            } else {
                // Default to System.Date for date values
                check_type_match_with_cross_namespace(&Some("System".to_string()), "Date", &target_namespace, &target_type, true)
            }
        },
        EvaluationResult::DateTime(_, type_info) => {
            if let Some(type_info) = type_info {
                check_type_match_with_cross_namespace(&Some(type_info.namespace.clone()), &type_info.name, &target_namespace, &target_type, true)
            } else {
                // Default to System.DateTime for datetime values
                check_type_match_with_cross_namespace(&Some("System".to_string()), "DateTime", &target_namespace, &target_type, true)
            }
        },
        EvaluationResult::Time(_, type_info) => {
            if let Some(type_info) = type_info {
                check_type_match_with_cross_namespace(&Some(type_info.namespace.clone()), &type_info.name, &target_namespace, &target_type, true)
            } else {
                // Default to System.Time for time values
                check_type_match_with_cross_namespace(&Some("System".to_string()), "Time", &target_namespace, &target_type, true)
            }
        },
        EvaluationResult::Quantity(_, _, type_info) => {
            if let Some(type_info) = type_info {
                check_type_match_with_cross_namespace(&Some(type_info.namespace.clone()), &type_info.name, &target_namespace, &target_type, true)
            } else {
                // Default to System.Quantity for quantity values
                check_type_match_with_cross_namespace(&Some("System".to_string()), "Quantity", &target_namespace, &target_type, true)
            }
        },
        EvaluationResult::Collection { .. } => {
            // Collections are not simple types and don't match single type checks
            Ok(false)
        },
        EvaluationResult::Empty => {
            // Empty values don't match any specific type
            Ok(false)
        },
        #[cfg(not(any(feature = "R4", feature = "R4B")))]
        EvaluationResult::Integer64(_, type_info) => {
            if let Some(type_info) = type_info {
                check_type_match_with_cross_namespace(&Some(type_info.namespace.clone()), &type_info.name, &target_namespace, &target_type, true)
            } else {
                // Default to System.Integer64 for integer64 values
                check_type_match_with_cross_namespace(&Some("System".to_string()), "Integer64", &target_namespace, &target_type, true)
            }
        },
        EvaluationResult::Object { map, type_info, .. } => {
            // First check if there's type_info available
            if let Some(type_info) = type_info {
                check_type_match_with_cross_namespace(&Some(type_info.namespace.clone()), &type_info.name, &target_namespace, &target_type, true)
            } else if let Some(resource_type_value) = map.get("resourceType") {
                // For FHIR resources, check the resourceType property
                if let EvaluationResult::String(resource_type, _) = resource_type_value {
                    // Check if the resource type matches the target type
                    check_type_match_with_cross_namespace(&Some("FHIR".to_string()), resource_type, &target_namespace, &target_type, true)
                } else {
                    Ok(false)
                }
            } else {
                // For non-FHIR objects, we can't determine the type
                Ok(false)
            }
        },
    }
}

/// Determines if a value is of a specified FHIR or System type (legacy version without context)
/// 
/// # Arguments
/// 
/// * `value` - The value to check
/// * `type_spec` - The type specifier to check against
/// 
/// # Returns
/// 
/// * `true` if the value is of the specified type, `false` otherwise
pub fn is_of_type(value: &EvaluationResult, type_spec: &TypeSpecifier) -> Result<bool, EvaluationError> {
    let (target_namespace, target_type) = extract_namespace_and_type(type_spec)?;
    
    match value {
        EvaluationResult::Boolean(_, type_info) => {
            if let Some(type_info) = type_info {
                check_type_match(&Some(type_info.namespace.clone()), &type_info.name, &target_namespace, &target_type)
            } else {
                // Default to System.Boolean for boolean values
                check_type_match(&Some("System".to_string()), "Boolean", &target_namespace, &target_type)
            }
        },
        EvaluationResult::Integer(_, type_info) => {
            if let Some(type_info) = type_info {
                check_type_match(&Some(type_info.namespace.clone()), &type_info.name, &target_namespace, &target_type)
            } else {
                // Default to System.Integer for integer values
                check_type_match(&Some("System".to_string()), "Integer", &target_namespace, &target_type)
            }
        },
        EvaluationResult::Decimal(_, type_info) => {
            if let Some(type_info) = type_info {
                check_type_match(&Some(type_info.namespace.clone()), &type_info.name, &target_namespace, &target_type)
            } else {
                // Default to System.Decimal for decimal values
                check_type_match(&Some("System".to_string()), "Decimal", &target_namespace, &target_type)
            }
        },
        EvaluationResult::String(_, type_info) => {
            if let Some(type_info) = type_info {
                check_type_match(&Some(type_info.namespace.clone()), &type_info.name, &target_namespace, &target_type)
            } else {
                // Default to System.String for string values
                check_type_match(&Some("System".to_string()), "String", &target_namespace, &target_type)
            }
        },
        EvaluationResult::Date(_, type_info) => {
            if let Some(type_info) = type_info {
                check_type_match(&Some(type_info.namespace.clone()), &type_info.name, &target_namespace, &target_type)
            } else {
                // Default to System.Date for date values
                check_type_match(&Some("System".to_string()), "Date", &target_namespace, &target_type)
            }
        },
        EvaluationResult::DateTime(_, type_info) => {
            if let Some(type_info) = type_info {
                check_type_match(&Some(type_info.namespace.clone()), &type_info.name, &target_namespace, &target_type)
            } else {
                // Default to System.DateTime for datetime values
                check_type_match(&Some("System".to_string()), "DateTime", &target_namespace, &target_type)
            }
        },
        EvaluationResult::Time(_, type_info) => {
            if let Some(type_info) = type_info {
                check_type_match(&Some(type_info.namespace.clone()), &type_info.name, &target_namespace, &target_type)
            } else {
                // Default to System.Time for time values
                check_type_match(&Some("System".to_string()), "Time", &target_namespace, &target_type)
            }
        },
        EvaluationResult::Quantity(_, _, type_info) => {
            if let Some(type_info) = type_info {
                check_type_match(&Some(type_info.namespace.clone()), &type_info.name, &target_namespace, &target_type)
            } else {
                // Default to System.Quantity for quantity values
                check_type_match(&Some("System".to_string()), "Quantity", &target_namespace, &target_type)
            }
        },
        EvaluationResult::Collection { .. } => {
            // Collections are not simple types and don't match single type checks
            Ok(false)
        },
        EvaluationResult::Empty => {
            // Empty values don't match any specific type
            Ok(false)
        },
        #[cfg(not(any(feature = "R4", feature = "R4B")))]
        EvaluationResult::Integer64(_, type_info) => {
            if let Some(type_info) = type_info {
                check_type_match(&Some(type_info.namespace.clone()), &type_info.name, &target_namespace, &target_type)
            } else {
                // Default to System.Integer64 for integer64 values
                check_type_match(&Some("System".to_string()), "Integer64", &target_namespace, &target_type)
            }
        },
        EvaluationResult::Object { map, type_info, .. } => {
            // First check if there's type_info available
            if let Some(type_info) = type_info {
                check_type_match(&Some(type_info.namespace.clone()), &type_info.name, &target_namespace, &target_type)
            } else if let Some(resource_type_value) = map.get("resourceType") {
                // For FHIR resources, check the resourceType property
                if let EvaluationResult::String(resource_type, _) = resource_type_value {
                    // Check if the resource type matches the target type
                    check_type_match(&Some("FHIR".to_string()), resource_type, &target_namespace, &target_type)
                } else {
                    Ok(false)
                }
            } else {
                // For non-FHIR objects, we can't determine the type
                Ok(false)
            }
        },
    }
}

/// Helper function to check if two types match, considering namespace and case-insensitive comparison
fn check_type_match(
    value_namespace: &Option<String>, 
    value_type: &str,
    target_namespace: &Option<String>, 
    target_type: &str
) -> Result<bool, EvaluationError> {
    check_type_match_with_cross_namespace(value_namespace, value_type, target_namespace, target_type, false)
}

/// Helper function to check if two types match with configurable cross-namespace matching
fn check_type_match_with_cross_namespace(
    value_namespace: &Option<String>, 
    value_type: &str,
    target_namespace: &Option<String>, 
    target_type: &str,
    allow_cross_namespace: bool
) -> Result<bool, EvaluationError> {
    // Case-insensitive type name comparison
    let type_matches = value_type.eq_ignore_ascii_case(target_type);
    
    // If no target namespace is specified, match any namespace
    if target_namespace.is_none() {
        return Ok(type_matches);
    }
    
    // Special case: FHIR complex types should match their System equivalents
    // For example, FHIR.Quantity should match System.Quantity
    if type_matches {
        match (value_namespace, target_namespace) {
            (Some(value_ns), Some(target_ns)) => {
                let namespace_matches = value_ns.eq_ignore_ascii_case(target_ns);
                
                // Allow FHIR/System cross-matching for specific types:
                // 1. Complex types like Quantity, Date, DateTime, Time (always allowed)
                // 2. Primitive types like boolean, string, integer, decimal (only for ofType operations when allow_cross_namespace is true)
                // 3. Resource types (checked dynamically)  
                let is_complex_type = matches!(
                    value_type.to_lowercase().as_str(), 
                    "quantity" | "date" | "datetime" | "time"
                );
                
                // Only allow cross-namespace matching for primitive types when explicitly enabled (for ofType operations)
                // FHIR.boolean should match the type specifier "boolean" only in ofType, not in is operations
                let is_primitive_type = allow_cross_namespace && matches!(
                    value_type.to_lowercase().as_str(), 
                    "boolean" | "string" | "integer" | "decimal" | "date" | "datetime" | "time" | 
                    "instant" | "id" | "oid" | "url" | "uuid" | "uri" | "canonical" | "code" | 
                    "markdown" | "base64binary" | "positiveint" | "unsignedint"
                );
                
                let is_cross_matchable_type = is_complex_type || is_primitive_type;
                
                let cross_namespace_match = is_cross_matchable_type && (
                    (value_ns.eq_ignore_ascii_case("FHIR") && target_ns.eq_ignore_ascii_case("System")) ||
                    (value_ns.eq_ignore_ascii_case("System") && target_ns.eq_ignore_ascii_case("FHIR"))
                );
                
                Ok(namespace_matches || cross_namespace_match)
            },
            (None, Some(_)) => {
                // Value has no namespace but target does - no match
                Ok(false)
            },
            _ => Ok(type_matches), // This case is already handled above
        }
    } else {
        Ok(false)
    }
}


/// Extract namespace and type name from a TypeSpecifier with context-aware resource checking
/// Handles qualified names like "System.Boolean" or "FHIR.Patient"
/// including backtick-quoted variants
pub fn extract_namespace_and_type_with_context(type_spec: &TypeSpecifier, context: &EvaluationContext) -> Result<(Option<String>, String), EvaluationError> {
    match type_spec {
        // Handle explicitly qualified types like FHIR.Patient in the AST
        // The parser should have already cleaned up backticks in both parts
        TypeSpecifier::QualifiedIdentifier(ns, Some(name)) => {
            // Clean the namespace and name
            let clean_name = clean_identifier(name);
            let clean_ns = clean_identifier(ns);
            
            
            // Special handling for multi-part namespaces (e.g. System.Collections.List)
            if clean_ns.contains('.') {
                // For our current qualified type checks, we only need the main namespace
                // (System, FHIR, etc.) and don't need to further split multi-part namespaces
                let primary_ns = clean_ns.split('.').next().unwrap_or(&clean_ns).to_string();
                
                // For now, we'll treat complex namespaces as just their first part
                // This simplifies handling System.Collections.List.Item vs System.Boolean
                return Ok((Some(primary_ns), clean_name));
            }
            
            // Normalize namespace casing for standard namespaces
            let normalized_ns = match clean_ns.to_lowercase().as_str() {
                "system" => "System".to_string(),
                "fhir" => "FHIR".to_string(),
                _ => clean_ns,
            };
            
            Ok((Some(normalized_ns), clean_name))
        },
        
        // Handle plain identifiers - these might be:
        // 1. Simple types like "Boolean"
        // 2. Already-qualified types like "System.Boolean" that need parsing
        TypeSpecifier::QualifiedIdentifier(name, _) => {
            // Clean the identifier
            let clean_name = clean_identifier(name);
            
            // Check if this is a dot-separated qualified name like "System.Boolean"
            if clean_name.contains('.') {
                let parts: Vec<&str> = clean_name.split('.').collect();
                
                if parts.len() >= 2 {
                    // Get namespace (first part) and type name (last part)
                    // For multi-part qualifiers like System.Collections.List,
                    // we'll consider only the first part as the primary namespace
                    let raw_namespace = parts[0].to_string();
                    let type_name = parts[parts.len() - 1].to_string();
                    
                    // Normalize namespace casing for standard namespaces
                    let normalized_ns = match raw_namespace.to_lowercase().as_str() {
                        "system" => "System".to_string(),
                        "fhir" => "FHIR".to_string(),
                        _ => raw_namespace,
                    };
                    
                    // Return namespace and clean type name
                    return Ok((Some(normalized_ns), clean_identifier(&type_name)));
                }
            }
            
            // Improved detection of type names with context-aware resource checking
            
            // First check for System types (with capitalized first letter)
            let first_char = clean_name.chars().next().unwrap_or('_');
            let is_likely_system_type = first_char.is_uppercase();
            
            // Known System primitive types
            let system_primitives = [
                "Boolean", "String", "Integer", "Decimal", "Date", 
                "DateTime", "Time", "Quantity"
            ];
            
            // Known FHIR primitive types (lowercase)
            let fhir_primitives = [
                "boolean", "string", "integer", "decimal", "date", 
                "dateTime", "time", "code", "id", "uri", "url", 
                "canonical", "markdown", "base64Binary", "instant", 
                "positiveInt", "unsignedInt", "uuid"
            ];
            
            // Check type based on capitalization to determine intended namespace
            if is_likely_system_type {
                // Capitalized names likely intended for System namespace
                if system_primitives.iter().any(|&t| t.eq_ignore_ascii_case(&clean_name)) {
                    return Ok((Some("System".to_string()), clean_name.clone()));
                }
            } else {
                // Lowercase names likely intended for FHIR namespace  
                if fhir_primitives.iter().any(|&t| t.eq_ignore_ascii_case(&clean_name)) {
                    return Ok((Some("FHIR".to_string()), clean_name.to_lowercase()));
                }
            }
            
            // Fallback: check if it matches any primitive type regardless of case preference
            if system_primitives.iter().any(|&t| t.eq_ignore_ascii_case(&clean_name)) {
                // When using "is(Boolean)" or "is(Integer)", normalize to System.X
                let normalized_type = if is_likely_system_type {
                    // Keep original capitalization for System types
                    clean_name.clone()
                } else {
                    // Capitalize first letter for System types
                    capitalize_first_letter(&clean_name)
                };
                
                return Ok((Some("System".to_string()), normalized_type));
            }
            
            else if fhir_primitives.iter().any(|&t| t.eq_ignore_ascii_case(&clean_name)) {
                // FHIR primitive types are conventionally lowercase
                let normalized_type = if is_likely_system_type {
                    // If capitalized, it might be a System type
                    clean_name.clone()
                } else {
                    // Otherwise keep lowercase for FHIR primitive
                    clean_name.to_lowercase()
                };
                
                return Ok((Some("FHIR".to_string()), normalized_type));
            }
            
            // Use context-aware resource checking instead of hard-coded lists
            else if is_resource_type_for_version(&clean_name, &context.fhir_version) {
                // Resource types default to FHIR namespace when unqualified since FHIR resources have FHIR type info
                return Ok((Some("FHIR".to_string()), capitalize_first_letter(&clean_name)));
            }
            
            // For complex types and unknown types, make an educated guess based on capitalization
            else {
                if is_likely_system_type {
                    // Capitalized types are likely System types
                    return Ok((Some("System".to_string()), clean_name));
                } else {
                    // Lowercase types are likely FHIR types
                    return Ok((Some("FHIR".to_string()), clean_name));
                }
            }
        }
    }
}

/// Extract namespace and type name from a TypeSpecifier (legacy version without context)
/// Handles qualified names like "System.Boolean" or "FHIR.Patient"
/// including backtick-quoted variants
pub fn extract_namespace_and_type(type_spec: &TypeSpecifier) -> Result<(Option<String>, String), EvaluationError> {
    match type_spec {
        // Handle explicitly qualified types like FHIR.Patient in the AST
        // The parser should have already cleaned up backticks in both parts
        TypeSpecifier::QualifiedIdentifier(ns, Some(name)) => {
            // Clean the namespace and name
            let clean_name = clean_identifier(name);
            let clean_ns = clean_identifier(ns);
            
            // Special handling for multi-part namespaces (e.g. System.Collections.List)
            if clean_ns.contains('.') {
                // For our current qualified type checks, we only need the main namespace
                // (System, FHIR, etc.) and don't need to further split multi-part namespaces
                let primary_ns = clean_ns.split('.').next().unwrap_or(&clean_ns).to_string();
                
                // For now, we'll treat complex namespaces as just their first part
                // This simplifies handling System.Collections.List.Item vs System.Boolean
                return Ok((Some(primary_ns), clean_name));
            }
            
            // Normalize namespace casing for standard namespaces
            let normalized_ns = match clean_ns.to_lowercase().as_str() {
                "system" => "System".to_string(),
                "fhir" => "FHIR".to_string(),
                _ => clean_ns,
            };
            
            Ok((Some(normalized_ns), clean_name))
        },
        
        // Handle plain identifiers - these might be:
        // 1. Simple types like "Boolean"
        // 2. Already-qualified types like "System.Boolean" that need parsing
        TypeSpecifier::QualifiedIdentifier(name, _) => {
            // Clean the identifier
            let clean_name = clean_identifier(name);
            
            // Check if this is a dot-separated qualified name like "System.Boolean"
            if clean_name.contains('.') {
                let parts: Vec<&str> = clean_name.split('.').collect();
                
                if parts.len() >= 2 {
                    // Get namespace (first part) and type name (last part)
                    // For multi-part qualifiers like System.Collections.List,
                    // we'll consider only the first part as the primary namespace
                    let raw_namespace = parts[0].to_string();
                    let type_name = parts[parts.len() - 1].to_string();
                    
                    // Normalize namespace casing for standard namespaces
                    let normalized_ns = match raw_namespace.to_lowercase().as_str() {
                        "system" => "System".to_string(),
                        "fhir" => "FHIR".to_string(),
                        _ => raw_namespace,
                    };
                    
                    // Return namespace and clean type name
                    return Ok((Some(normalized_ns), clean_identifier(&type_name)));
                }
            }
            
            // Improved detection of type names with more accurate namespace detection
            
            // First check for System types (with capitalized first letter)
            let first_char = clean_name.chars().next().unwrap_or('_');
            let is_likely_system_type = first_char.is_uppercase();
            
            // Known System primitive types
            let system_primitives = [
                "Boolean", "String", "Integer", "Decimal", "Date", 
                "DateTime", "Time", "Quantity"
            ];
            
            // Known FHIR primitive types (lowercase)
            let fhir_primitives = [
                "boolean", "string", "integer", "decimal", "date", 
                "dateTime", "time", "code", "id", "uri", "url", 
                "canonical", "markdown", "base64Binary", "instant", 
                "positiveInt", "unsignedInt", "uuid"
            ];
            
            // Known FHIR resource types (always start with uppercase)
            let fhir_resource_types = [
                "Patient", "Observation", "MedicationRequest", "Condition", 
                "Encounter", "DomainResource", "Resource", "Questionnaire",
                "ValueSet", "Bundle", "Practitioner", "Organization",
                "CarePlan", "Procedure", "Immunization", "DiagnosticReport"
            ];
            
            // Known FHIR complex types (always start with uppercase)
            let fhir_complex_types = [
                "Quantity", "Money", "HumanName", "Address", "Reference", 
                "Identifier", "CodeableConcept", "Period", "Timing", 
                "ContactPoint", "Coding", "Attachment", "Range", "Ratio"
            ];
            
            // Check if the clean_name is a known System primitive type
            if system_primitives.iter().any(|&t| t.eq_ignore_ascii_case(&clean_name)) {
                // When using "is(Boolean)" or "is(Integer)", normalize to System.X
                let normalized_type = if is_likely_system_type {
                    // Keep original capitalization for System types
                    clean_name.clone()
                } else {
                    // Capitalize first letter for System types
                    capitalize_first_letter(&clean_name)
                };
                
                return Ok((Some("System".to_string()), normalized_type));
            }
            
            // Check if the clean_name is a known FHIR primitive type
            else if fhir_primitives.iter().any(|&t| t.eq_ignore_ascii_case(&clean_name)) {
                // FHIR primitive types are conventionally lowercase
                let normalized_type = if is_likely_system_type {
                    // If capitalized, it might be a System type
                    clean_name.clone()
                } else {
                    // Otherwise keep lowercase for FHIR primitive
                    clean_name.to_lowercase()
                };
                
                return Ok((Some("FHIR".to_string()), normalized_type));
            }
            
            // Check if the clean_name is a known FHIR resource type
            else if fhir_resource_types.iter().any(|&t| t.eq_ignore_ascii_case(&clean_name)) {
                // Unqualified resource types default to FHIR namespace
                return Ok((Some("FHIR".to_string()), capitalize_first_letter(&clean_name)));
            }
            
            // Check if the clean_name is a known FHIR complex type
            else if fhir_complex_types.iter().any(|&t| t.eq_ignore_ascii_case(&clean_name)) {
                // Unqualified complex types default to FHIR namespace
                return Ok((Some("FHIR".to_string()), capitalize_first_letter(&clean_name)));
            }
            
            // For types we're not confident about, make an educated guess based on capitalization
            else {
                if is_likely_system_type {
                    // Capitalized types are likely System types
                    return Ok((Some("System".to_string()), clean_name));
                } else {
                    // Lowercase types are likely FHIR types
                    return Ok((Some("FHIR".to_string()), clean_name));
                }
            }
        }
    }
}

// Using capitalize_first_letter from fhir_type_hierarchy instead

/// Helper function to clean up backtick-quoted identifiers
fn clean_identifier(ident: &str) -> String {
    if ident.starts_with('`') && ident.ends_with('`') && ident.len() > 2 {
        ident[1..ident.len()-1].to_string()
    } else {
        ident.to_string()
    }
}

/// Determines if a FHIR resource type is a DomainResource
/// 
/// In FHIR, many resource types inherit from DomainResource.
/// This function checks if a given resource type is a DomainResource.
pub fn is_fhir_domain_resource(resource_type: &str) -> bool {
    // In a real implementation, this should ideally use the FHIR type system
    // or a proper knowledge base of FHIR resource type hierarchy.
    // Instead of a static list, we should query the actual type system.
    
    // These are *some* of the resources that inherit from DomainResource
    // This is not a comprehensive list - in a production implementation,
    // this should be derived from the FHIR specification metadata
    match resource_type {
        // Clinical Resources
        "Patient" | "Practitioner" | "PractitionerRole" | "RelatedPerson" |
        "Person" | "Group" | "Organization" | "CareTeam" | "EpisodeOfCare" |
        
        // Clinical Summary
        "Condition" | "Procedure" | "Observation" | "DiagnosticReport" |
        "CarePlan" | "ClinicalImpression" | "FamilyMemberHistory" |
        
        // Medications
        "Medication" | "MedicationRequest" | "MedicationAdministration" |
        "MedicationDispense" | "MedicationStatement" | "Immunization" |
        
        // Workflow
        "Encounter" | "Appointment" | "Schedule" | "ServiceRequest" | "Task" |
        
        // Financial
        "Coverage" | "Claim" | "ClaimResponse" | "Invoice" |
        
        // Administrative
        "Location" | "Device" | "Questionnaire" | "QuestionnaireResponse" |
        
        // Foundation Resources
        "ValueSet" | "CodeSystem" | "StructureDefinition" | "CapabilityStatement" |
        "ImplementationGuide" | "OperationDefinition" | "SearchParameter" => true,
        
        // Not a DomainResource (or not recognized)
        _ => false,
    }
}

// is_fhir_type and is_system_type were removed as they were never used and
// their functionality has been incorporated into is_of_type and other functions

/// Attempts to cast a value to a specific type
///
/// # Arguments
///
/// * `value` - The value to cast
/// * `type_spec` - The type to cast to
///
/// # Returns
///
/// * The value as the specified type if possible, or Empty if not
pub fn as_type(value: &EvaluationResult, type_spec: &TypeSpecifier) -> Result<EvaluationResult, EvaluationError> {
    // Check if the value is of the specified type
    if is_of_type(value, type_spec)? {
        // If it matches, return the value as-is
        Ok(value.clone())
    } else {
        // If it doesn't match, return Empty
        Ok(EvaluationResult::Empty)
    }
}

/// Helper function to try converting a value to a target type for ofType operations
fn try_convert_for_of_type(value: &EvaluationResult, type_spec: &TypeSpecifier) -> Result<Option<EvaluationResult>, EvaluationError> {
    let (_target_namespace, target_type) = extract_namespace_and_type(type_spec)?;
    
    match value {
        EvaluationResult::String(s, type_info) => {
            // Check if this is a FHIR string that might represent a different type
            if let Some(type_info) = type_info {
                if type_info.namespace == "FHIR" && type_info.name == "string" {
                    // Try to convert FHIR strings to target types
                    match target_type.to_lowercase().as_str() {
                        "datetime" => {
                            // Only convert to datetime if it looks like a date or datetime, not a time
                            if s.len() == 10 && s.chars().nth(4) == Some('-') && s.chars().nth(7) == Some('-') {
                                // This is a date string like "2010-10-10", convert to DateTime
                                Ok(Some(EvaluationResult::datetime(s.clone())))
                            } else if s.contains('T') || s.len() > 10 {
                                // This looks like a full datetime string
                                Ok(Some(EvaluationResult::datetime(s.clone())))
                            } else {
                                // Don't convert time-only strings to datetime
                                Ok(None)
                            }
                        }
                        "time" => {
                            // Try to parse as time
                            if s.contains(':') {
                                Ok(Some(EvaluationResult::time(s.clone())))
                            } else {
                                Ok(None)
                            }
                        }
                        "date" => {
                            // Try to parse as date
                            if s.len() >= 7 && s.chars().nth(4) == Some('-') {
                                Ok(Some(EvaluationResult::date(s.clone())))
                            } else {
                                Ok(None)
                            }
                        }
                        "instant" => {
                            // Try to parse as instant (must have timezone)
                            if s.contains('T') && (s.contains('+') || s.contains('-') || s.ends_with('Z')) {
                                Ok(Some(EvaluationResult::fhir_string(s.clone(), "instant")))
                            } else {
                                Ok(None)
                            }
                        }
                        "id" => {
                            // Only convert simple strings to ID, not URLs or other special formats
                            if s.starts_with("http://") || s.starts_with("https://") || s.starts_with("urn:") {
                                // These look like URLs, URIs, OIDs, UUIDs - not simple IDs
                                Ok(None)
                            } else {
                                // Simple string values can be IDs
                                Ok(Some(EvaluationResult::fhir_string(s.clone(), "id")))
                            }
                        }
                        "oid" => {
                            // OID strings start with "urn:oid:"
                            if s.starts_with("urn:oid:") {
                                Ok(Some(EvaluationResult::fhir_string(s.clone(), "oid")))
                            } else {
                                Ok(None)
                            }
                        }
                        "url" => {
                            // URL strings typically start with "http://" or "https://"
                            if s.starts_with("http://") || s.starts_with("https://") {
                                Ok(Some(EvaluationResult::fhir_string(s.clone(), "url")))
                            } else {
                                Ok(None)
                            }
                        }
                        "uuid" => {
                            // UUID strings start with "urn:uuid:"
                            if s.starts_with("urn:uuid:") {
                                Ok(Some(EvaluationResult::fhir_string(s.clone(), "uuid")))
                            } else {
                                Ok(None)
                            }
                        }
                        "uri" => {
                            // URI strings can be various formats
                            if s.starts_with("urn:") || s.starts_with("http://") || s.starts_with("https://") {
                                Ok(Some(EvaluationResult::fhir_string(s.clone(), "uri")))
                            } else {
                                Ok(None)
                            }
                        }
                        "canonical" => {
                            // Canonical URIs are similar to URIs
                            if s.starts_with("http://") || s.starts_with("https://") {
                                Ok(Some(EvaluationResult::fhir_string(s.clone(), "canonical")))
                            } else {
                                Ok(None)
                            }
                        }
                        "code" => {
                            // Simple string values can be codes
                            Ok(Some(EvaluationResult::fhir_string(s.clone(), "code")))
                        }
                        "markdown" => {
                            // Any string can be markdown
                            Ok(Some(EvaluationResult::fhir_string(s.clone(), "markdown")))
                        }
                        "base64binary" => {
                            // Base64 encoded strings 
                            Ok(Some(EvaluationResult::fhir_string(s.clone(), "base64Binary")))
                        }
                        _ => Ok(None)
                    }
                } else {
                    Ok(None)
                }
            } else {
                Ok(None)
            }
        }
        _ => Ok(None)
    }
}

/// Filters a collection based on a type specifier
/// 
/// # Arguments
/// 
/// * `collection` - The collection to filter
/// * `type_spec` - The type to filter by
/// 
/// # Returns
/// 
/// * A new collection containing only items of the specified type
/// * If there's only one item in the collection, returns that item directly (unwrapped)
/// * If the collection is empty, returns Empty
pub fn of_type(collection: &EvaluationResult, type_spec: &TypeSpecifier) -> Result<EvaluationResult, EvaluationError> {
    
    // Use a consistent helper function for applying the type filter
    let apply_type_filter = |items: &[EvaluationResult]| -> Result<EvaluationResult, EvaluationError> {
        let mut result = Vec::new();
        
        for item in items {
            if is_of_type_for_of_type(item, type_spec)? {
                result.push(item.clone());
            } else if let Some(converted) = try_convert_for_of_type(item, type_spec)? {
                result.push(converted);
            }
        }
        
        if result.is_empty() {
            Ok(EvaluationResult::Empty)
        } else if result.len() == 1 {
            Ok(result[0].clone())
        } else {
            // ofType preserves the order of the input collection
            let input_was_unordered = if let EvaluationResult::Collection { has_undefined_order: true, .. } = collection { true } else { false };
            Ok(EvaluationResult::Collection { items: result, has_undefined_order: input_was_unordered, type_info: None })
        }
    };
    
    match collection {
        EvaluationResult::Collection { items, .. } => apply_type_filter(items), // Destructure
        EvaluationResult::Empty => Ok(EvaluationResult::Empty),
        
        // For a singleton value, treat it like a collection of one
        _ => {
            if is_of_type_for_of_type(collection, type_spec)? {
                // Return the value directly for a singleton that matches
                Ok(collection.clone())
            } else if let Some(converted) = try_convert_for_of_type(collection, type_spec)? {
                Ok(converted)
            } else {
                Ok(EvaluationResult::Empty)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::str::FromStr;
    
    // Helper function to create a FHIR Patient resource
    fn create_patient() -> EvaluationResult {
        let mut patient = HashMap::new();
        patient.insert("resourceType".to_string(), EvaluationResult::string("Patient".to_string()));
        patient.insert("id".to_string(), EvaluationResult::string("123".to_string()));
        patient.insert("active".to_string(), EvaluationResult::boolean(true));
        
        // Add a name
        let mut name = HashMap::new();
        name.insert("use".to_string(), EvaluationResult::string("official".to_string()));
        name.insert("family".to_string(), EvaluationResult::string("Smith".to_string()));
        
        let given = vec![
            EvaluationResult::string("John".to_string()),
            EvaluationResult::string("Q".to_string()),
        ];
        name.insert("given".to_string(), EvaluationResult::Collection { items: given, has_undefined_order: false, type_info: None });
        
        let names = vec![EvaluationResult::Object { map: name, type_info: None }];
        patient.insert("name".to_string(), EvaluationResult::Collection { items: names, has_undefined_order: false, type_info: None });
        
        // Add birthDate
        patient.insert("birthDate".to_string(), EvaluationResult::string("1974-12-25".to_string()));
        
        EvaluationResult::Object { map: patient, type_info: None }
    }
    
    // Helper function to create a FHIR Observation resource with a valueQuantity
    fn create_observation() -> EvaluationResult {
        let mut obs = HashMap::new();
        obs.insert("resourceType".to_string(), EvaluationResult::string("Observation".to_string()));
        obs.insert("id".to_string(), EvaluationResult::string("456".to_string()));
        obs.insert("status".to_string(), EvaluationResult::string("final".to_string()));
        
        // Add valueQuantity
        let mut quantity = HashMap::new();
        quantity.insert("value".to_string(), EvaluationResult::decimal(rust_decimal::Decimal::from(185)));
        quantity.insert("unit".to_string(), EvaluationResult::string("lbs".to_string()));
        quantity.insert("system".to_string(), EvaluationResult::string("http://unitsofmeasure.org".to_string()));
        quantity.insert("code".to_string(), EvaluationResult::string("lb_av".to_string()));
        
        obs.insert("valueQuantity".to_string(), EvaluationResult::Object { map: quantity, type_info: None });
        
        EvaluationResult::Object { map: obs, type_info: None }
    }
    
    #[test]
    fn test_is_of_type_system_types() {
        // Test System types
        let bool_val = EvaluationResult::boolean(true);
        let int_val = EvaluationResult::integer(42);
        let dec_val = EvaluationResult::decimal(rust_decimal::Decimal::from_str("3.14").unwrap());
        let str_val = EvaluationResult::string("test".to_string());
        
        // Create type specifiers
        let bool_type = TypeSpecifier::QualifiedIdentifier("Boolean".to_string(), None);
        let int_type = TypeSpecifier::QualifiedIdentifier("Integer".to_string(), None);
        let dec_type = TypeSpecifier::QualifiedIdentifier("Decimal".to_string(), None);
        let str_type = TypeSpecifier::QualifiedIdentifier("String".to_string(), None);
        
        // Test correct matches
        assert!(is_of_type(&bool_val, &bool_type).unwrap());
        assert!(is_of_type(&int_val, &int_type).unwrap());
        assert!(is_of_type(&dec_val, &dec_type).unwrap());
        assert!(is_of_type(&str_val, &str_type).unwrap());
        
        // Test incorrect matches
        assert!(!is_of_type(&bool_val, &int_type).unwrap());
        assert!(!is_of_type(&int_val, &str_type).unwrap());
        assert!(!is_of_type(&dec_val, &bool_type).unwrap());
        assert!(!is_of_type(&str_val, &dec_type).unwrap());
    }
    
    #[test]
    fn test_is_of_type_fhir_resources() {
        // Create FHIR resources
        let patient = create_patient();
        let observation = create_observation();
        
        // Print the patient object for debugging
        if let EvaluationResult::Object { map: obj, type_info: None } = &patient {
            eprintln!("Patient object:");
            for (key, value) in obj {
                eprintln!("  {}: {:?}", key, value);
            }
        }
        
        // Create type specifiers with exact case matching the resourceType property
        let patient_type = TypeSpecifier::QualifiedIdentifier("Patient".to_string(), None);
        
        // Print the type specifier for debugging
        eprintln!("Patient type: {:?}", patient_type);
        
        let is_result = is_of_type(&patient, &patient_type);
        eprintln!("is_of_type result: {:?}", is_result);
        
        // Test correct matches
        assert!(is_result.unwrap());
        
        // Create the rest of the type specifiers
        let obs_type = TypeSpecifier::QualifiedIdentifier("Observation".to_string(), None);
        let fhir_patient_type = TypeSpecifier::QualifiedIdentifier("FHIR".to_string(), Some("Patient".to_string()));
        
        assert!(is_of_type(&observation, &obs_type).unwrap());
        assert!(is_of_type(&patient, &fhir_patient_type).unwrap());
        
        // Test with different casing (should still work with case-insensitive comparison)
        let patient_type_lowercase = TypeSpecifier::QualifiedIdentifier("patient".to_string(), None);
        assert!(is_of_type(&patient, &patient_type_lowercase).unwrap());
        
        // Test incorrect matches
        assert!(!is_of_type(&patient, &obs_type).unwrap());
        assert!(!is_of_type(&observation, &patient_type).unwrap());
    }
    
    #[test]
    fn test_as_type() {
        // Create test values
        let bool_val = EvaluationResult::boolean(true);
        let patient = create_patient();
        let observation = create_observation();
        
        // Create type specifiers with exact case matching the resourceType property
        let bool_type = TypeSpecifier::QualifiedIdentifier("Boolean".to_string(), None);
        let patient_type = TypeSpecifier::QualifiedIdentifier("Patient".to_string(), None);
        let obs_type = TypeSpecifier::QualifiedIdentifier("Observation".to_string(), None);
        
        // Test correct casts
        assert_eq!(as_type(&bool_val, &bool_type).unwrap(), bool_val);
        assert_eq!(as_type(&patient, &patient_type).unwrap(), patient);
        assert_eq!(as_type(&observation, &obs_type).unwrap(), observation);
        
        // Test with different casing (should still work)
        let patient_type_lowercase = TypeSpecifier::QualifiedIdentifier("patient".to_string(), None);
        assert_eq!(as_type(&patient, &patient_type_lowercase).unwrap(), patient);
        
        // Test incorrect casts
        assert_eq!(as_type(&bool_val, &patient_type).unwrap(), EvaluationResult::Empty);
        assert_eq!(as_type(&patient, &bool_type).unwrap(), EvaluationResult::Empty);
        assert_eq!(as_type(&observation, &patient_type).unwrap(), EvaluationResult::Empty);
    }
    
    #[test]
    fn test_of_type() {
        // Create a collection with mixed types
        let collection = EvaluationResult::Collection { items: vec![
            EvaluationResult::boolean(true),
            EvaluationResult::integer(42),
            EvaluationResult::boolean(false),
            EvaluationResult::string("test".to_string()),
        ], has_undefined_order: false, type_info: None };
        
        // Create type specifiers
        let bool_type = TypeSpecifier::QualifiedIdentifier("Boolean".to_string(), None);
        let int_type = TypeSpecifier::QualifiedIdentifier("Integer".to_string(), None);
        let str_type = TypeSpecifier::QualifiedIdentifier("String".to_string(), None);
        
        // Test filtering with multiple matches - should return a collection
        let collection_with_only_booleans = EvaluationResult::Collection { items: vec![
            EvaluationResult::boolean(true),
            EvaluationResult::boolean(false),
        ], has_undefined_order: false, type_info: None };
        let bool_result = of_type(&collection, &bool_type).unwrap();
        assert_eq!(bool_result, collection_with_only_booleans);
        
        // Test filtering with single match - should return the single item directly
        let int_result = of_type(&collection, &int_type).unwrap();
        assert_eq!(int_result, EvaluationResult::integer(42));
        
        // Test another single match
        let str_result = of_type(&collection, &str_type).unwrap();
        assert_eq!(str_result, EvaluationResult::string("test".to_string()));
        
        // Test with no matches
        let decimal_type = TypeSpecifier::QualifiedIdentifier("Decimal".to_string(), None);
        let decimal_result = of_type(&collection, &decimal_type).unwrap();
        assert_eq!(decimal_result, EvaluationResult::Empty);
    }
}
