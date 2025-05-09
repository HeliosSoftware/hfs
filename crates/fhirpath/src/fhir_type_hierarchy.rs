use std::collections::{HashMap, HashSet};
use once_cell::sync::Lazy;

/// FHIR Type Hierarchy module
/// 
/// This module provides a comprehensive model of the FHIR type hierarchy
/// and utility functions for type checking, type conversion, and type reflection.
/// It models both resource types (Patient, Observation, etc.) and data types
/// (boolean, string, Quantity, etc.) with their inheritance relationships.

/// Constants for namespaces
pub const SYSTEM_NAMESPACE: &str = "System";
pub const FHIR_NAMESPACE: &str = "FHIR";

/// Static type hierarchy for FHIR resource types
static RESOURCE_TYPE_HIERARCHY: Lazy<HashMap<&'static str, &'static str>> = Lazy::new(|| {
    let mut h = HashMap::new();
    
    // Base Resource types
    h.insert("DomainResource", "Resource");
    
    // Resource descendants - all FHIR resources derive from Resource
    // DomainResource descendants
    h.insert("Patient", "DomainResource");
    h.insert("Practitioner", "DomainResource");
    h.insert("PractitionerRole", "DomainResource");
    h.insert("Organization", "DomainResource");
    h.insert("Location", "DomainResource");
    h.insert("Observation", "DomainResource");
    h.insert("DiagnosticReport", "DomainResource");
    h.insert("MedicationRequest", "DomainResource");
    h.insert("MedicationAdministration", "DomainResource");
    h.insert("Encounter", "DomainResource");
    h.insert("Condition", "DomainResource");
    h.insert("Procedure", "DomainResource");
    h.insert("Immunization", "DomainResource");
    h.insert("AllergyIntolerance", "DomainResource");
    h.insert("CarePlan", "DomainResource");
    h.insert("Goal", "DomainResource");
    h.insert("Claim", "DomainResource");
    h.insert("ExplanationOfBenefit", "DomainResource");
    h.insert("Coverage", "DomainResource");
    h.insert("Questionnaire", "DomainResource");
    h.insert("QuestionnaireResponse", "DomainResource");
    h.insert("ValueSet", "DomainResource");
    h.insert("CodeSystem", "DomainResource");
    h.insert("StructureDefinition", "DomainResource");
    h.insert("OperationDefinition", "DomainResource");
    h.insert("SearchParameter", "DomainResource");
    
    // Non-DomainResource descendants
    h.insert("Bundle", "Resource");
    h.insert("Parameters", "Resource");
    h.insert("Binary", "Resource");
    
    h
});

/// Set of FHIR primitive types
static FHIR_PRIMITIVE_TYPES: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    let mut s = HashSet::new();
    s.insert("boolean");
    s.insert("string");
    s.insert("integer");
    s.insert("decimal");
    s.insert("date");
    s.insert("dateTime");
    s.insert("time");
    s.insert("code");
    s.insert("id");
    s.insert("uri");
    s.insert("url");
    s.insert("canonical");
    s.insert("markdown");
    s.insert("base64Binary");
    s.insert("instant");
    s.insert("oid");
    s.insert("positiveInt");
    s.insert("unsignedInt");
    s.insert("uuid");
    s
});

/// Set of FHIR complex data types
static FHIR_COMPLEX_TYPES: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    let mut s = HashSet::new();
    s.insert("Quantity");
    s.insert("Money");
    s.insert("Period");
    s.insert("Range");
    s.insert("Ratio");
    s.insert("Reference");
    s.insert("HumanName");
    s.insert("Address");
    s.insert("ContactPoint");
    s.insert("Identifier");
    s.insert("CodeableConcept");
    s.insert("Coding");
    s.insert("Attachment");
    s.insert("Timing");
    s.insert("Signature");
    s.insert("Annotation");
    s.insert("SampledData");
    s.insert("Age");
    s.insert("Distance");
    s.insert("Duration");
    s.insert("Count");
    s
});

/// Set of System primitive types
static SYSTEM_PRIMITIVE_TYPES: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    let mut s = HashSet::new();
    s.insert("Boolean");
    s.insert("String");
    s.insert("Integer");
    s.insert("Decimal");
    s.insert("Date");
    s.insert("DateTime");
    s.insert("Time");
    s.insert("Quantity");
    s
});

/// Mapping from FHIR primitive types to their equivalent System types
static FHIR_TO_SYSTEM_TYPE_MAP: Lazy<HashMap<&'static str, &'static str>> = Lazy::new(|| {
    let mut m = HashMap::new();
    m.insert("boolean", "Boolean");
    m.insert("string", "String");
    m.insert("integer", "Integer");
    m.insert("decimal", "Decimal");
    m.insert("date", "Date");
    m.insert("dateTime", "DateTime");
    m.insert("time", "Time");
    m.insert("Quantity", "Quantity"); // Complex type that exists in both namespaces
    m
});

/// Checks if a type is a FHIR primitive type
pub fn is_fhir_primitive_type(type_name: &str) -> bool {
    FHIR_PRIMITIVE_TYPES.contains(type_name.to_lowercase().as_str())
}

/// Checks if a type is a FHIR complex data type
pub fn is_fhir_complex_type(type_name: &str) -> bool {
    FHIR_COMPLEX_TYPES.contains(capitalize_first_letter(type_name).as_str())
}

/// Checks if a type is a System primitive type
pub fn is_system_primitive_type(type_name: &str) -> bool {
    SYSTEM_PRIMITIVE_TYPES.contains(capitalize_first_letter(type_name).as_str())
}

/// Checks if a type name is a known FHIR resource type
pub fn is_fhir_resource_type(type_name: &str) -> bool {
    // Handle "Resource" directly (special case)
    if type_name.eq_ignore_ascii_case("Resource") {
        return true;
    }
    
    // Check if the type is in the hierarchy (case-insensitive)
    // First normalize to proper capitalization
    let normalized = capitalize_first_letter(type_name);
    
    // Then check if it's directly in the map
    if RESOURCE_TYPE_HIERARCHY.contains_key(normalized.as_str()) {
        return true;
    }
    
    // If not, try a case-insensitive search through the keys
    for key in RESOURCE_TYPE_HIERARCHY.keys() {
        if key.eq_ignore_ascii_case(type_name) {
            return true;
        }
    }
    
    false
}

/// Checks if a type is derived from a given parent type
/// This function handles the FHIR resource type hierarchy
/// 
/// # Arguments
/// 
/// * `type_name` - The type to check
/// * `parent_type` - The potential parent type
/// 
/// # Returns
/// 
/// * `true` if type_name is equal to or derived from parent_type, `false` otherwise
pub fn is_derived_from(type_name: &str, parent_type: &str) -> bool {
    // Handle case-insensitive direct equality
    if type_name.eq_ignore_ascii_case(parent_type) {
        return true;
    }
    
    // Normalize types to canonical form (capitalized)
    let normalized_type = capitalize_first_letter(type_name);
    let normalized_parent = capitalize_first_letter(parent_type);
    
    // Special case for Resource (or resource) as the root type
    if normalized_parent == "Resource" && is_fhir_resource_type(&normalized_type) {
        return true;
    }
    
    // Also handle lower-case 'resource' the same way
    if parent_type.eq_ignore_ascii_case("resource") && is_fhir_resource_type(&normalized_type) {
        return true;
    }
    
    // Follow the type hierarchy
    let mut current = normalized_type.as_str();
    while let Some(&parent) = RESOURCE_TYPE_HIERARCHY.get(current) {
        if parent.eq_ignore_ascii_case(&normalized_parent) {
            return true;
        }
        current = parent;
    }
    
    false
}

/// Gets the appropriate namespace for a primitive type name
/// 
/// # Arguments
/// 
/// * `type_name` - The type name to check
/// 
/// # Returns
/// 
/// * The namespace ("System" or "FHIR") for the given type
pub fn get_primitive_type_namespace(type_name: &str) -> &'static str {
    // Check if the name starts with uppercase (System namespace)
    // or lowercase (FHIR namespace)
    let first_char = type_name.chars().next().unwrap_or('_');
    if first_char.is_uppercase() {
        SYSTEM_NAMESPACE
    } else {
        FHIR_NAMESPACE
    }
}

/// Converts a type name from one namespace to the other
/// 
/// # Arguments
/// 
/// * `type_name` - The type name to convert
/// * `target_namespace` - The target namespace ("System" or "FHIR")
/// 
/// # Returns
/// 
/// * The equivalent type name in the target namespace, or None if not convertible
pub fn convert_type_namespace(type_name: &str, target_namespace: &str) -> Option<String> {
    let normalized = type_name.to_lowercase();
    
    match target_namespace {
        SYSTEM_NAMESPACE => {
            // Convert FHIR type to System type
            FHIR_TO_SYSTEM_TYPE_MAP.get(normalized.as_str())
                .map(|&system_type| system_type.to_string())
        },
        FHIR_NAMESPACE => {
            // Convert System type to FHIR type
            for (fhir_type, system_type) in FHIR_TO_SYSTEM_TYPE_MAP.iter() {
                if system_type.eq_ignore_ascii_case(type_name) {
                    return Some((*fhir_type).to_string());
                }
            }
            None
        },
        _ => None
    }
}

/// Utility function to capitalize the first letter of a string
/// 
/// # Arguments
/// 
/// * `s` - The string to capitalize
/// 
/// # Returns
/// 
/// * A new string with the first letter capitalized
pub fn capitalize_first_letter(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => {
            let cap = c.to_uppercase().collect::<String>();
            cap + chars.as_str()
        }
    }
}

/// Determines the most specific namespace for a type
/// 
/// For unqualified type names, this determines whether the type
/// should be in the System or FHIR namespace based on conventions.
/// 
/// # Arguments
/// 
/// * `type_name` - The type name to analyze
/// 
/// # Returns
/// 
/// * The most appropriate namespace for the type
pub fn determine_type_namespace(type_name: &str) -> &'static str {
    let normalized = capitalize_first_letter(type_name);
    
    // Check if it's a known FHIR resource
    if is_fhir_resource_type(&normalized) {
        return FHIR_NAMESPACE;
    }
    
    // Check if it's a known FHIR complex type
    if is_fhir_complex_type(&normalized) {
        return FHIR_NAMESPACE;
    }
    
    // For primitive types, use the case convention
    // lowercase = FHIR, uppercase first letter = System
    let first_char = type_name.chars().next().unwrap_or('_');
    if first_char.is_lowercase() && is_fhir_primitive_type(type_name) {
        FHIR_NAMESPACE
    } else if first_char.is_uppercase() && is_system_primitive_type(type_name) {
        SYSTEM_NAMESPACE
    } else if is_fhir_primitive_type(&type_name.to_lowercase()) {
        // Default to FHIR namespace for primitive types if no clear convention
        FHIR_NAMESPACE
    } else {
        // Default to System namespace for unknown types
        SYSTEM_NAMESPACE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_resource_hierarchy() {
        // Test basic inheritance
        assert!(is_derived_from("Patient", "DomainResource"));
        assert!(is_derived_from("Patient", "Resource"));
        assert!(is_derived_from("DomainResource", "Resource"));
        
        // Test with mixed case
        assert!(is_derived_from("patient", "DomainResource"));
        
        // Is Patient a resource?
        println!("Is Patient a resource type: {}", is_fhir_resource_type("Patient"));
        println!("Is PATIENT a resource type: {}", is_fhir_resource_type("PATIENT"));
        println!("resource check: {}", "resource".eq_ignore_ascii_case("resource"));
        
        // Test explicit checking: PATIENT should be derived from resource
        let is_patient_derived_from_resource = is_derived_from("PATIENT", "resource");
        println!("Is PATIENT derived from resource: {}", is_patient_derived_from_resource);
        assert!(is_patient_derived_from_resource, "PATIENT should be derived from resource");
        
        // Test negative cases
        assert!(!is_derived_from("Patient", "Bundle"));
        assert!(!is_derived_from("Bundle", "DomainResource"));
        
        // Test self-derivation
        assert!(is_derived_from("Patient", "Patient"));
        assert!(is_derived_from("Resource", "Resource"));
    }
    
    #[test]
    fn test_type_namespace_determination() {
        // Test FHIR resource types
        assert_eq!(determine_type_namespace("Patient"), FHIR_NAMESPACE);
        assert_eq!(determine_type_namespace("Resource"), FHIR_NAMESPACE);
        
        // Test FHIR complex types
        assert_eq!(determine_type_namespace("HumanName"), FHIR_NAMESPACE);
        assert_eq!(determine_type_namespace("CodeableConcept"), FHIR_NAMESPACE);
        
        // Test primitive types by case
        assert_eq!(determine_type_namespace("boolean"), FHIR_NAMESPACE);
        assert_eq!(determine_type_namespace("Boolean"), SYSTEM_NAMESPACE);
        assert_eq!(determine_type_namespace("string"), FHIR_NAMESPACE);
        assert_eq!(determine_type_namespace("String"), SYSTEM_NAMESPACE);
    }
    
    #[test]
    fn test_namespace_conversion() {
        // Test FHIR to System conversion
        assert_eq!(convert_type_namespace("boolean", SYSTEM_NAMESPACE), Some("Boolean".to_string()));
        assert_eq!(convert_type_namespace("decimal", SYSTEM_NAMESPACE), Some("Decimal".to_string()));
        
        // Test System to FHIR conversion
        assert_eq!(convert_type_namespace("Boolean", FHIR_NAMESPACE), Some("boolean".to_string()));
        assert_eq!(convert_type_namespace("Decimal", FHIR_NAMESPACE), Some("decimal".to_string()));
        
        // Test case insensitivity
        assert_eq!(convert_type_namespace("BOOLEAN", SYSTEM_NAMESPACE), Some("Boolean".to_string()));
        assert_eq!(convert_type_namespace("Boolean", FHIR_NAMESPACE), Some("boolean".to_string()));
    }
}