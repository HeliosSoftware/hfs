//! # FHIR Type Hierarchy
//!
//! Implements FHIR type system navigation and inheritance checking for FHIRPath type operations.

use helios_fhir::{FhirPrimitiveTypeProvider, FhirVersion};

/// FHIR Type Hierarchy module
///
/// This module provides utility functions for FHIR type checking and string manipulation.
/// It includes primitive type checking and string capitalization utilities.
///
/// Checks if a type is a FHIR primitive type in the given FHIR version.
///
/// Delegates to the macro-generated [`FhirPrimitiveTypeProvider`] implementation,
/// which is derived from the FHIR specification (StructureDefinitions of kind
/// `primitive-type`), rather than a hand-maintained list.
pub fn is_fhir_primitive_type_for_version(type_name: &str, fhir_version: &FhirVersion) -> bool {
    match fhir_version {
        #[cfg(feature = "R4")]
        FhirVersion::R4 => helios_fhir::r4::PrimitiveTypes::is_primitive_type(type_name),
        #[cfg(feature = "R4B")]
        FhirVersion::R4B => helios_fhir::r4b::PrimitiveTypes::is_primitive_type(type_name),
        #[cfg(feature = "R5")]
        FhirVersion::R5 => helios_fhir::r5::PrimitiveTypes::is_primitive_type(type_name),
        #[cfg(feature = "R6")]
        FhirVersion::R6 => helios_fhir::r6::PrimitiveTypes::is_primitive_type(type_name),
        #[allow(unreachable_patterns)]
        _ => false, // For versions not enabled by feature flags
    }
}

/// Checks if a type is a FHIR primitive type.
///
/// Version-agnostic convenience wrapper around [`is_fhir_primitive_type_for_version`]:
/// returns `true` if the type is a primitive in any enabled FHIR version.
pub fn is_fhir_primitive_type(type_name: &str) -> bool {
    FhirVersion::enabled_versions()
        .iter()
        .any(|version| is_fhir_primitive_type_for_version(type_name, version))
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
