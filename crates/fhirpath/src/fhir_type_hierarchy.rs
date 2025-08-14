use once_cell::sync::Lazy;
use std::collections::HashSet;

/// FHIR Type Hierarchy module
///
/// This module provides utility functions for FHIR type checking and string manipulation.
/// It includes primitive type checking and string capitalization utilities.
///
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

/// Checks if a type is a FHIR primitive type
pub fn is_fhir_primitive_type(type_name: &str) -> bool {
    FHIR_PRIMITIVE_TYPES.contains(type_name.to_lowercase().as_str())
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
