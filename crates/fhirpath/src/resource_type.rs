use fhirpath_support::{EvaluationError, EvaluationResult};
use crate::parser::TypeSpecifier;
use crate::fhir_type_hierarchy::capitalize_first_letter;

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
pub fn is_of_type(_value: &EvaluationResult, _type_spec: &TypeSpecifier) -> Result<bool, EvaluationError> {
    // Temporarily return false until we have an implementation 
    Ok(false)
}


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
pub fn as_type(_value: &EvaluationResult, _type_spec: &TypeSpecifier) -> Result<EvaluationResult, EvaluationError> {
    // Temporarily return empty until we have an implementation
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
            Ok(EvaluationResult::Collection { items: result, has_undefined_order: input_was_unordered, type_info: None })
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
