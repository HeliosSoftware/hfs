use fhirpath_support::{EvaluationError, EvaluationResult};
use crate::parser::TypeSpecifier;
use crate::fhir_type_hierarchy::{
    SYSTEM_NAMESPACE, FHIR_NAMESPACE,
    is_derived_from, is_fhir_resource_type, is_fhir_primitive_type, 
    is_fhir_complex_type, determine_type_namespace, capitalize_first_letter
};

/// Handles type operations for FHIR resources, supporting is/as operators.
/// This module provides enhanced support for handling FHIR resource types
/// in FHIRPath expressions, allowing for proper type checking and filtering
/// based on resource types.

/// Determines if a value is of a specified FHIR or System type
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
    // Extract namespace and type name
    let (namespace, type_name) = extract_namespace_and_type(type_spec)?;

    // Handle Empty values first
    if matches!(value, EvaluationResult::Empty) {
        return Ok(false);
    }
    
    // Handle collections for expressions like "1 | 1 is Integer"
    if let EvaluationResult::Collection { items, .. } = value { // Destructure
        // If checking if a collection is a specific type, check if all items are of that type
        if !items.is_empty() {
            // Check each item in the collection
            for item in items { // Iterate over destructured items
                if !is_of_type(item, type_spec)? {
                    return Ok(false);
                }
            }
            // If all items passed the type check, return true
            return Ok(true);
        }
        return Ok(false);
    }
    
    // System namespace type checks
    if namespace.as_deref() == Some(SYSTEM_NAMESPACE) || namespace.as_deref().map(|s| s.eq_ignore_ascii_case(SYSTEM_NAMESPACE)).unwrap_or(false) {
        // Check if value is of the specified System type
        match value {
            EvaluationResult::Boolean(_) => return Ok(type_name.eq_ignore_ascii_case("Boolean")),
            EvaluationResult::String(_) => return Ok(type_name.eq_ignore_ascii_case("String")),
            EvaluationResult::Integer(_) => return Ok(type_name.eq_ignore_ascii_case("Integer")),
            EvaluationResult::Decimal(_) => return Ok(type_name.eq_ignore_ascii_case("Decimal")),
            EvaluationResult::Date(_) => return Ok(type_name.eq_ignore_ascii_case("Date")),
            EvaluationResult::DateTime(_) => {
                return Ok(type_name.eq_ignore_ascii_case("DateTime") || 
                          type_name.eq_ignore_ascii_case("dateTime"))
            },
            EvaluationResult::Time(_) => return Ok(type_name.eq_ignore_ascii_case("Time")),
            EvaluationResult::Quantity(_, _) => return Ok(type_name.eq_ignore_ascii_case("Quantity")),
            // For collections and objects, they're never basic System types
            _ => return Ok(false),
        }
    }
    
    // FHIR namespace type checks
    if namespace.as_deref() == Some(FHIR_NAMESPACE) || namespace.as_deref().map(|s| s.eq_ignore_ascii_case(FHIR_NAMESPACE)).unwrap_or(false) {
        // First, check primitive types with FHIR namespace
        if is_fhir_primitive_type(&type_name) {
            match value {
                EvaluationResult::Boolean(_) => return Ok(type_name.eq_ignore_ascii_case("boolean")),
                EvaluationResult::String(_) => return Ok(type_name.eq_ignore_ascii_case("string")),
                EvaluationResult::Integer(_) => return Ok(type_name.eq_ignore_ascii_case("integer")),
                EvaluationResult::Decimal(_) => return Ok(type_name.eq_ignore_ascii_case("decimal")),
                EvaluationResult::Date(_) => return Ok(type_name.eq_ignore_ascii_case("date")),
                EvaluationResult::DateTime(_) => {
                    return Ok(type_name.eq_ignore_ascii_case("dateTime") || 
                              type_name.eq_ignore_ascii_case("datetime"))
                },
                EvaluationResult::Time(_) => return Ok(type_name.eq_ignore_ascii_case("time")),
                // FHIR primitive types don't match non-primitive values
                _ => return Ok(false),
            }
        }
        
        // Check for FHIR resource and complex type matches
        if let EvaluationResult::Object(obj) = value {
            // Check if this is a FHIR resource with matching resourceType
            if let Some(EvaluationResult::String(resource_type)) = obj.get("resourceType") {
                // First, check for direct match
                if resource_type.eq_ignore_ascii_case(&type_name) {
                    return Ok(true);
                }

                // Check for inheritance relationships using the type hierarchy
                let normalized_type = capitalize_first_letter(resource_type);
                let normalized_check = capitalize_first_letter(&type_name);

                if is_derived_from(&normalized_type, &normalized_check) {
                    return Ok(true);
                }

                // Special case for Observation.value polymorphic access
                if resource_type == "Observation" && type_name.eq_ignore_ascii_case("Quantity") {
                    if obj.contains_key("valueQuantity") {
                        return Ok(true);
                    } else if obj.contains_key("value") {
                        // If there's a direct "value" property, check if it looks like a Quantity
                        if let Some(EvaluationResult::Object(value_obj)) = obj.get("value") {
                            if value_obj.contains_key("value") &&
                              (value_obj.contains_key("unit") || value_obj.contains_key("code")) {
                                return Ok(true);
                            }
                        }
                    }
                }
            }
            
            // Check for choice element fields that match the requested type
            // Example: Checking if Observation has value of type Quantity should check valueQuantity
            for key in obj.keys() {
                if key.starts_with("value") && key.len() > 5 {
                    let suffix = &key[5..];
                    if suffix.eq_ignore_ascii_case(&type_name) {
                        return Ok(true);
                    }
                }
            }

            // Check if this matches a complex data type
            if is_fhir_complex_type(&type_name) {
                // Check for Quantity and other complex types
                if type_name.eq_ignore_ascii_case("Quantity") {
                    // A Quantity object should have value and unit properties
                    if obj.contains_key("value") && (obj.contains_key("unit") || obj.contains_key("code")) {
                        return Ok(true);
                    }
                }
                
                // Check for other complex types based on their distinctive properties
                if type_name.eq_ignore_ascii_case("Period") {
                    if obj.contains_key("start") || obj.contains_key("end") {
                        return Ok(true);
                    }
                }
                
                if type_name.eq_ignore_ascii_case("Reference") {
                    if obj.contains_key("reference") || obj.contains_key("identifier") {
                        return Ok(true);
                    }
                }
                
                // For valueX fields that match the type
                for (key, _) in obj.iter() {
                    if key.starts_with("value") && key.len() > 5 {
                        let value_type = &key[5..];
                        if value_type.eq_ignore_ascii_case(&type_name) {
                            return Ok(true);
                        }
                    }
                }
            }
        }
        
        // Check for Quantity type
        if type_name.eq_ignore_ascii_case("Quantity") && matches!(value, EvaluationResult::Quantity(_, _)) {
            return Ok(true);
        }
        
        return Ok(false);
    }
    
    // If no namespace is specified, infer the appropriate namespace based on the type
    if namespace.is_none() {
        // First, determine the most likely namespace based on the type name
        let inferred_namespace = determine_type_namespace(&type_name);
        
        // Recursively check with the inferred namespace
        let qualified_type_spec = match inferred_namespace {
            SYSTEM_NAMESPACE => TypeSpecifier::QualifiedIdentifier(SYSTEM_NAMESPACE.to_string(), Some(capitalize_first_letter(&type_name))),
            FHIR_NAMESPACE => {
                if is_fhir_resource_type(&type_name) || is_fhir_complex_type(&type_name) {
                    TypeSpecifier::QualifiedIdentifier(FHIR_NAMESPACE.to_string(), Some(capitalize_first_letter(&type_name)))
                } else {
                    // For FHIR primitive types, keep lowercase
                    TypeSpecifier::QualifiedIdentifier(FHIR_NAMESPACE.to_string(), Some(type_name.to_lowercase()))
                }
            },
            _ => return Err(EvaluationError::TypeError(format!("Unknown namespace: {}", inferred_namespace))),
        };
        
        return is_of_type(value, &qualified_type_spec);
    }
    
    // If we've reached this point with no match, return false
    Ok(false)
}

// is_fhir_property was removed as it was never used and 
// its functionality was incorporated into other functions

/// Extract namespace and type name from a TypeSpecifier
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
        TypeSpecifier::QualifiedIdentifier(name, None) => {
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
                // FHIR resource types should be capitalized
                return Ok((Some("FHIR".to_string()), capitalize_first_letter(&clean_name)));
            }
            
            // Check if the clean_name is a known FHIR complex type
            else if fhir_complex_types.iter().any(|&t| t.eq_ignore_ascii_case(&clean_name)) {
                // FHIR complex types should be capitalized
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
    // First check if the value is of the specified type
    let is_result = is_of_type(value, type_spec)?;

    if is_result {
        // If it's of the right type, return it
        return Ok(value.clone());
    }

    // Extract namespace and type name
    let (namespace, type_name) = extract_namespace_and_type(type_spec)?;

    // Check for FHIR polymorphic choice elements
    if let EvaluationResult::Object(obj) = value {
        // For FHIR resource types, check if we have resourceType field
        if obj.contains_key("resourceType") {
            if let Some(EvaluationResult::String(resource_type)) = obj.get("resourceType") {
                // Do a case-insensitive comparison
                if resource_type.to_lowercase() == type_name.to_lowercase() {
                    return Ok(value.clone());
                }

                // Special case for Observation.as(Quantity)
                if resource_type == "Observation" && type_name.eq_ignore_ascii_case("Quantity") {
                    // Return valueQuantity if it exists
                    if let Some(value_quantity) = obj.get("valueQuantity") {
                        return Ok(value_quantity.clone());
                    }
                }
            }
        }

        // Direct access to choice element fields with type suffix
        // Example: Observation has valueQuantity field for Observation.value.as(Quantity)
        if type_name.eq_ignore_ascii_case("Quantity") {
            // Look for valueQuantity, effectiveQuantity, etc.
            for (key, val) in obj.iter() {
                if key.len() > 5 && key.starts_with("value") {
                    let suffix = &key[5..];
                    if suffix.eq_ignore_ascii_case("Quantity") {
                        return Ok(val.clone());
                    }
                }
            }
        }

        // If this is a polymorphic value property, handle directly
        // (e.g., object is the "value" property itself)
        if obj.contains_key("value") && obj.contains_key("unit") && type_name.eq_ignore_ascii_case("Quantity") {
            return Ok(value.clone());
        }

        // Check for polymorphic choice elements
        if let Some(ns) = &namespace {
            if ns == "FHIR" || ns == "http://hl7.org/fhir" {
                // This is a FHIR type specifier, check for choice elements
                for (key, val) in obj.iter() {
                    // If the key ends with the type name, it might be a choice element
                    if key.ends_with(&type_name) && key.len() > type_name.len() {
                        let prefix = &key[..key.len() - type_name.len()];
                        if crate::polymorphic_access::is_choice_element(prefix) {
                            return Ok(val.clone());
                        }
                    }
                }
            }
        }

        // If no explicit namespace and the type matches a resource type,
        // check through all field properties
        if namespace.is_none() {
            for (key, val) in obj.iter() {
                if key.to_lowercase() == type_name.to_lowercase() {
                    return Ok(val.clone());
                }
            }
        }
    }
    
    // Special handling for type conversion between dates/times
    if let Some(ns) = &namespace {
        if ns == "System" {
            match type_name.to_lowercase().as_str() {
                "date" => {
                    // Try to convert from DateTime or String to Date
                    if let Some(date_str) = crate::datetime_impl::to_date(value) {
                        return Ok(EvaluationResult::Date(date_str));
                    }
                },
                "datetime" | "dateTime" => {
                    // Try to convert from Date or String to DateTime
                    if let Some(dt_str) = crate::datetime_impl::to_datetime(value) {
                        return Ok(EvaluationResult::DateTime(dt_str));
                    }
                },
                "time" => {
                    // Try to convert from DateTime or String to Time
                    if let Some(time_str) = crate::datetime_impl::to_time(value) {
                        return Ok(EvaluationResult::Time(time_str));
                    }
                },
                _ => {}
            }
        }
    }
    
    // If no match found, return Empty
    Ok(EvaluationResult::Empty)
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
            if is_of_type(item, type_spec)? {
                result.push(item.clone());
            }
        }
        
        if result.is_empty() {
            Ok(EvaluationResult::Empty)
        } else if result.len() == 1 {
            Ok(result[0].clone())
        } else {
            // ofType preserves the order of the input collection
            let input_was_unordered = if let EvaluationResult::Collection { has_undefined_order: true, .. } = collection { true } else { false };
            Ok(EvaluationResult::Collection { items: result, has_undefined_order: input_was_unordered })
        }
    };
    
    match collection {
        EvaluationResult::Collection { items, .. } => apply_type_filter(items), // Destructure
        EvaluationResult::Empty => Ok(EvaluationResult::Empty),
        
        // For a singleton value, treat it like a collection of one
        _ => {
            if is_of_type(collection, type_spec)? {
                // Return the value directly for a singleton that matches
                Ok(collection.clone())
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
        patient.insert("resourceType".to_string(), EvaluationResult::String("Patient".to_string()));
        patient.insert("id".to_string(), EvaluationResult::String("123".to_string()));
        patient.insert("active".to_string(), EvaluationResult::Boolean(true));
        
        // Add a name
        let mut name = HashMap::new();
        name.insert("use".to_string(), EvaluationResult::String("official".to_string()));
        name.insert("family".to_string(), EvaluationResult::String("Smith".to_string()));
        
        let given = vec![
            EvaluationResult::String("John".to_string()),
            EvaluationResult::String("Q".to_string()),
        ];
        name.insert("given".to_string(), EvaluationResult::Collection { items: given, has_undefined_order: false });
        
        let names = vec![EvaluationResult::Object(name)];
        patient.insert("name".to_string(), EvaluationResult::Collection { items: names, has_undefined_order: false });
        
        // Add birthDate
        patient.insert("birthDate".to_string(), EvaluationResult::String("1974-12-25".to_string()));
        
        EvaluationResult::Object(patient)
    }
    
    // Helper function to create a FHIR Observation resource with a valueQuantity
    fn create_observation() -> EvaluationResult {
        let mut obs = HashMap::new();
        obs.insert("resourceType".to_string(), EvaluationResult::String("Observation".to_string()));
        obs.insert("id".to_string(), EvaluationResult::String("456".to_string()));
        obs.insert("status".to_string(), EvaluationResult::String("final".to_string()));
        
        // Add valueQuantity
        let mut quantity = HashMap::new();
        quantity.insert("value".to_string(), EvaluationResult::Decimal(rust_decimal::Decimal::from(185)));
        quantity.insert("unit".to_string(), EvaluationResult::String("lbs".to_string()));
        quantity.insert("system".to_string(), EvaluationResult::String("http://unitsofmeasure.org".to_string()));
        quantity.insert("code".to_string(), EvaluationResult::String("lb_av".to_string()));
        
        obs.insert("valueQuantity".to_string(), EvaluationResult::Object(quantity));
        
        EvaluationResult::Object(obs)
    }
    
    #[test]
    fn test_is_of_type_system_types() {
        // Test System types
        let bool_val = EvaluationResult::Boolean(true);
        let int_val = EvaluationResult::Integer(42);
        let dec_val = EvaluationResult::Decimal(rust_decimal::Decimal::from_str("3.14").unwrap());
        let str_val = EvaluationResult::String("test".to_string());
        
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
        if let EvaluationResult::Object(obj) = &patient {
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
        let bool_val = EvaluationResult::Boolean(true);
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
        let collection = EvaluationResult::Collection(vec![
            EvaluationResult::Boolean(true),
            EvaluationResult::Integer(42),
            EvaluationResult::Boolean(false),
            EvaluationResult::String("test".to_string()),
        ]);
        
        // Create type specifiers
        let bool_type = TypeSpecifier::QualifiedIdentifier("Boolean".to_string(), None);
        let int_type = TypeSpecifier::QualifiedIdentifier("Integer".to_string(), None);
        let str_type = TypeSpecifier::QualifiedIdentifier("String".to_string(), None);
        
        // Test filtering with multiple matches - should return a collection
        let collection_with_only_booleans = EvaluationResult::Collection(vec![
            EvaluationResult::Boolean(true),
            EvaluationResult::Boolean(false),
        ]);
        let bool_result = of_type(&collection, &bool_type).unwrap();
        assert_eq!(bool_result, collection_with_only_booleans);
        
        // Test filtering with single match - should return the single item directly
        let int_result = of_type(&collection, &int_type).unwrap();
        assert_eq!(int_result, EvaluationResult::Integer(42));
        
        // Test another single match
        let str_result = of_type(&collection, &str_type).unwrap();
        assert_eq!(str_result, EvaluationResult::String("test".to_string()));
        
        // Test with no matches
        let decimal_type = TypeSpecifier::QualifiedIdentifier("Decimal".to_string(), None);
        let decimal_result = of_type(&collection, &decimal_type).unwrap();
        assert_eq!(decimal_result, EvaluationResult::Empty);
    }
}
