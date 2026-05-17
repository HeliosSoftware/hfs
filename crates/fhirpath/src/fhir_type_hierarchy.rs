//! # FHIR Type Hierarchy
//!
//! Implements FHIR type system navigation and inheritance checking for FHIRPath type operations.

/// Checks if a type code is a FHIR primitive datatype. Forgiving on case so
/// callers can pass `"Boolean"` or `"boolean"`; delegates to the canonical
/// list in [`helios_fhir::is_primitive_type`].
pub fn is_fhir_primitive_type(type_name: &str) -> bool {
    helios_fhir::is_primitive_type(&lowercase_first_char(type_name))
}

/// FHIR primitive type codes are lowercase in the spec, but FHIRPath
/// expressions often use the capitalized System form (`Boolean`,
/// `Integer`). Lowering just the first character normalizes both shapes
/// to the FHIR primitive code (`boolean`, `integer`, `dateTime`).
fn lowercase_first_char(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_ascii_lowercase().to_string() + chars.as_str(),
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
