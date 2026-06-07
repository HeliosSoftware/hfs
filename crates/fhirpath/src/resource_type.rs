//! # FHIR Resource Type Handling
//!
//! Provides utilities for working with FHIR resource types in FHIRPath evaluation.

use crate::evaluator::EvaluationContext;
use crate::fhir_type_hierarchy::{capitalize_first_letter, is_fhir_primitive_type};
use crate::parser::TypeSpecifier;
use helios_fhir::{FhirComplexTypeProvider, FhirResourceTypeProvider, FhirVersion};
use helios_fhirpath_support::{EvaluationError, EvaluationResult};

/// Handles type operations for FHIR resources, supporting is/as operators.
/// This module provides enhanced support for handling FHIR resource types
/// in FHIRPath expressions, allowing for proper type checking and filtering
/// based on resource types.
///
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
        FhirVersion::R4 => helios_fhir::r4::Resource::is_resource_type(type_name),
        #[cfg(feature = "R4B")]
        FhirVersion::R4B => helios_fhir::r4b::Resource::is_resource_type(type_name),
        #[cfg(feature = "R5")]
        FhirVersion::R5 => helios_fhir::r5::Resource::is_resource_type(type_name),
        #[cfg(feature = "R6")]
        FhirVersion::R6 => helios_fhir::r6::Resource::is_resource_type(type_name),
        #[allow(unreachable_patterns)]
        _ => false, // For versions not enabled by feature flags
    }
}

/// Checks if a type name is a complex type for the given FHIR version
///
/// # Arguments
///
/// * `type_name` - The type name to check (e.g., "Quantity", "HumanName")
/// * `fhir_version` - The FHIR version to check against
///
/// # Returns
///
/// * `true` if the type is a complex type in the given FHIR version, `false` otherwise
pub fn is_complex_type_for_version(type_name: &str, fhir_version: &FhirVersion) -> bool {
    match fhir_version {
        #[cfg(feature = "R4")]
        FhirVersion::R4 => helios_fhir::r4::ComplexTypes::is_complex_type(type_name),
        #[cfg(feature = "R4B")]
        FhirVersion::R4B => helios_fhir::r4b::ComplexTypes::is_complex_type(type_name),
        #[cfg(feature = "R5")]
        FhirVersion::R5 => helios_fhir::r5::ComplexTypes::is_complex_type(type_name),
        #[cfg(feature = "R6")]
        FhirVersion::R6 => helios_fhir::r6::ComplexTypes::is_complex_type(type_name),
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
pub fn is_of_type_with_context(
    value: &EvaluationResult,
    type_spec: &TypeSpecifier,
    context: &EvaluationContext,
) -> Result<bool, EvaluationError> {
    let (target_namespace, target_type) =
        extract_namespace_and_type_with_context(type_spec, context)?;
    match value {
        EvaluationResult::Boolean(_, type_info, _) => {
            if let Some(type_info) = type_info {
                check_type_match(
                    &Some(type_info.namespace.clone()),
                    &type_info.name,
                    &target_namespace,
                    &target_type,
                )
            } else {
                // Default to System.Boolean for boolean values
                check_type_match(
                    &Some("System".to_string()),
                    "Boolean",
                    &target_namespace,
                    &target_type,
                )
            }
        }
        EvaluationResult::Integer(_, type_info, _) => {
            if let Some(type_info) = type_info {
                check_type_match(
                    &Some(type_info.namespace.clone()),
                    &type_info.name,
                    &target_namespace,
                    &target_type,
                )
            } else {
                // Default to System.Integer for integer values
                check_type_match(
                    &Some("System".to_string()),
                    "Integer",
                    &target_namespace,
                    &target_type,
                )
            }
        }
        EvaluationResult::Decimal(_, type_info, _) => {
            if let Some(type_info) = type_info {
                check_type_match(
                    &Some(type_info.namespace.clone()),
                    &type_info.name,
                    &target_namespace,
                    &target_type,
                )
            } else {
                // Default to System.Decimal for decimal values
                check_type_match(
                    &Some("System".to_string()),
                    "Decimal",
                    &target_namespace,
                    &target_type,
                )
            }
        }
        EvaluationResult::String(_, type_info, _) => {
            if let Some(type_info) = type_info {
                check_type_match(
                    &Some(type_info.namespace.clone()),
                    &type_info.name,
                    &target_namespace,
                    &target_type,
                )
            } else {
                // Default to System.String for string values
                check_type_match(
                    &Some("System".to_string()),
                    "String",
                    &target_namespace,
                    &target_type,
                )
            }
        }
        EvaluationResult::Date(_, type_info, _) => {
            if let Some(type_info) = type_info {
                check_type_match(
                    &Some(type_info.namespace.clone()),
                    &type_info.name,
                    &target_namespace,
                    &target_type,
                )
            } else {
                // Default to System.Date for date values
                check_type_match(
                    &Some("System".to_string()),
                    "Date",
                    &target_namespace,
                    &target_type,
                )
            }
        }
        EvaluationResult::DateTime(_, type_info, _) => {
            if let Some(type_info) = type_info {
                check_type_match(
                    &Some(type_info.namespace.clone()),
                    &type_info.name,
                    &target_namespace,
                    &target_type,
                )
            } else {
                // Default to System.DateTime for datetime values
                check_type_match(
                    &Some("System".to_string()),
                    "DateTime",
                    &target_namespace,
                    &target_type,
                )
            }
        }
        EvaluationResult::Time(_, type_info, _) => {
            if let Some(type_info) = type_info {
                check_type_match(
                    &Some(type_info.namespace.clone()),
                    &type_info.name,
                    &target_namespace,
                    &target_type,
                )
            } else {
                // Default to System.Time for time values
                check_type_match(
                    &Some("System".to_string()),
                    "Time",
                    &target_namespace,
                    &target_type,
                )
            }
        }
        EvaluationResult::Quantity(_, _, type_info, _) => {
            if let Some(type_info) = type_info {
                check_type_match(
                    &Some(type_info.namespace.clone()),
                    &type_info.name,
                    &target_namespace,
                    &target_type,
                )
            } else {
                // Default to System.Quantity for quantity values
                check_type_match(
                    &Some("System".to_string()),
                    "Quantity",
                    &target_namespace,
                    &target_type,
                )
            }
        }
        EvaluationResult::Object { map, type_info, .. } => {
            // First check if there's type_info available
            if let Some(type_info) = type_info {
                check_type_match(
                    &Some(type_info.namespace.clone()),
                    &type_info.name,
                    &target_namespace,
                    &target_type,
                )
            } else if let Some(resource_type_value) = map.get("resourceType") {
                // For FHIR resources, check the resourceType property
                if let EvaluationResult::String(resource_type, _, _) = resource_type_value {
                    // Check if the resource type matches the target type
                    check_type_match(
                        &Some("FHIR".to_string()),
                        resource_type,
                        &target_namespace,
                        &target_type,
                    )
                } else {
                    Ok(false)
                }
            } else {
                // Default to System.Object for object values
                check_type_match(
                    &Some("System".to_string()),
                    "Object",
                    &target_namespace,
                    &target_type,
                )
            }
        }
        EvaluationResult::Collection { type_info, .. } => {
            if let Some(type_info) = type_info {
                check_type_match(
                    &Some(type_info.namespace.clone()),
                    &type_info.name,
                    &target_namespace,
                    &target_type,
                )
            } else {
                // Default to System.Collection for collection values
                check_type_match(
                    &Some("System".to_string()),
                    "Collection",
                    &target_namespace,
                    &target_type,
                )
            }
        }
        EvaluationResult::Empty => {
            // Empty values don't match any specific type
            Ok(false)
        }
        #[cfg(not(any(feature = "R4", feature = "R4B")))]
        EvaluationResult::Integer64(_, type_info, _) => {
            if let Some(type_info) = type_info {
                check_type_match(
                    &Some(type_info.namespace.clone()),
                    &type_info.name,
                    &target_namespace,
                    &target_type,
                )
            } else {
                // Default to System.Integer64 for integer64 values
                check_type_match(
                    &Some("System".to_string()),
                    "Integer64",
                    &target_namespace,
                    &target_type,
                )
            }
        }
        #[cfg(any(feature = "R4", feature = "R4B"))]
        EvaluationResult::Integer64(_, _, _) => {
            // In R4 and R4B, Integer64 should be treated as Integer
            check_type_match(
                &Some("System".to_string()),
                "Integer",
                &target_namespace,
                &target_type,
            )
        }
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
#[allow(dead_code)]
pub fn is_of_type_for_of_type(
    value: &EvaluationResult,
    type_spec: &TypeSpecifier,
) -> Result<bool, EvaluationError> {
    let (target_namespace, target_type) = extract_namespace_and_type_without_context(type_spec)?;

    match value {
        EvaluationResult::Boolean(_, type_info, _) => {
            if let Some(type_info) = type_info {
                check_type_match_with_cross_namespace(
                    &Some(type_info.namespace.clone()),
                    &type_info.name,
                    &target_namespace,
                    &target_type,
                    true,
                )
            } else {
                // Default to System.Boolean for boolean values
                check_type_match_with_cross_namespace(
                    &Some("System".to_string()),
                    "Boolean",
                    &target_namespace,
                    &target_type,
                    true,
                )
            }
        }
        EvaluationResult::Integer(_, type_info, _) => {
            if let Some(type_info) = type_info {
                check_type_match_with_cross_namespace(
                    &Some(type_info.namespace.clone()),
                    &type_info.name,
                    &target_namespace,
                    &target_type,
                    true,
                )
            } else {
                // Default to System.Integer for integer values
                check_type_match_with_cross_namespace(
                    &Some("System".to_string()),
                    "Integer",
                    &target_namespace,
                    &target_type,
                    true,
                )
            }
        }
        EvaluationResult::Decimal(_, type_info, _) => {
            if let Some(type_info) = type_info {
                check_type_match_with_cross_namespace(
                    &Some(type_info.namespace.clone()),
                    &type_info.name,
                    &target_namespace,
                    &target_type,
                    true,
                )
            } else {
                // Default to System.Decimal for decimal values
                check_type_match_with_cross_namespace(
                    &Some("System".to_string()),
                    "Decimal",
                    &target_namespace,
                    &target_type,
                    true,
                )
            }
        }
        EvaluationResult::String(_, type_info, _) => {
            if let Some(type_info) = type_info {
                check_type_match_with_cross_namespace(
                    &Some(type_info.namespace.clone()),
                    &type_info.name,
                    &target_namespace,
                    &target_type,
                    true,
                )
            } else {
                // Default to System.String for string values
                check_type_match_with_cross_namespace(
                    &Some("System".to_string()),
                    "String",
                    &target_namespace,
                    &target_type,
                    true,
                )
            }
        }
        EvaluationResult::Date(_, type_info, _) => {
            if let Some(type_info) = type_info {
                check_type_match_with_cross_namespace(
                    &Some(type_info.namespace.clone()),
                    &type_info.name,
                    &target_namespace,
                    &target_type,
                    true,
                )
            } else {
                // Default to System.Date for date values
                check_type_match_with_cross_namespace(
                    &Some("System".to_string()),
                    "Date",
                    &target_namespace,
                    &target_type,
                    true,
                )
            }
        }
        EvaluationResult::DateTime(_, type_info, _) => {
            if let Some(type_info) = type_info {
                check_type_match_with_cross_namespace(
                    &Some(type_info.namespace.clone()),
                    &type_info.name,
                    &target_namespace,
                    &target_type,
                    true,
                )
            } else {
                // Default to System.DateTime for datetime values
                check_type_match_with_cross_namespace(
                    &Some("System".to_string()),
                    "DateTime",
                    &target_namespace,
                    &target_type,
                    true,
                )
            }
        }
        EvaluationResult::Time(_, type_info, _) => {
            if let Some(type_info) = type_info {
                check_type_match_with_cross_namespace(
                    &Some(type_info.namespace.clone()),
                    &type_info.name,
                    &target_namespace,
                    &target_type,
                    true,
                )
            } else {
                // Default to System.Time for time values
                check_type_match_with_cross_namespace(
                    &Some("System".to_string()),
                    "Time",
                    &target_namespace,
                    &target_type,
                    true,
                )
            }
        }
        EvaluationResult::Quantity(_, _, type_info, _) => {
            if let Some(type_info) = type_info {
                check_type_match_with_cross_namespace(
                    &Some(type_info.namespace.clone()),
                    &type_info.name,
                    &target_namespace,
                    &target_type,
                    true,
                )
            } else {
                // Default to System.Quantity for quantity values
                check_type_match_with_cross_namespace(
                    &Some("System".to_string()),
                    "Quantity",
                    &target_namespace,
                    &target_type,
                    true,
                )
            }
        }
        EvaluationResult::Collection { .. } => {
            // Collections are not simple types and don't match single type checks
            Ok(false)
        }
        EvaluationResult::Empty => {
            // Empty values don't match any specific type
            Ok(false)
        }
        #[cfg(not(any(feature = "R4", feature = "R4B")))]
        EvaluationResult::Integer64(_, type_info, _) => {
            if let Some(type_info) = type_info {
                check_type_match_with_cross_namespace(
                    &Some(type_info.namespace.clone()),
                    &type_info.name,
                    &target_namespace,
                    &target_type,
                    true,
                )
            } else {
                // Default to System.Integer64 for integer64 values
                check_type_match_with_cross_namespace(
                    &Some("System".to_string()),
                    "Integer64",
                    &target_namespace,
                    &target_type,
                    true,
                )
            }
        }
        #[cfg(any(feature = "R4", feature = "R4B"))]
        EvaluationResult::Integer64(_, _, _) => {
            // In R4 and R4B, Integer64 should be treated as Integer
            check_type_match_with_cross_namespace(
                &Some("System".to_string()),
                "Integer",
                &target_namespace,
                &target_type,
                true,
            )
        }
        EvaluationResult::Object { map, type_info, .. } => {
            // First check if there's type_info available
            if let Some(type_info) = type_info {
                check_type_match_with_cross_namespace(
                    &Some(type_info.namespace.clone()),
                    &type_info.name,
                    &target_namespace,
                    &target_type,
                    true,
                )
            } else if let Some(resource_type_value) = map.get("resourceType") {
                // For FHIR resources, check the resourceType property
                if let EvaluationResult::String(resource_type, _, _) = resource_type_value {
                    // Check if the resource type matches the target type
                    check_type_match_with_cross_namespace(
                        &Some("FHIR".to_string()),
                        resource_type,
                        &target_namespace,
                        &target_type,
                        true,
                    )
                } else {
                    Ok(false)
                }
            } else {
                // For non-FHIR objects, we can't determine the type
                Ok(false)
            }
        }
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
#[allow(dead_code)]
pub fn is_of_type(
    value: &EvaluationResult,
    type_spec: &TypeSpecifier,
) -> Result<bool, EvaluationError> {
    let (target_namespace, target_type) = extract_namespace_and_type_without_context(type_spec)?;

    match value {
        EvaluationResult::Boolean(_, type_info, _) => {
            if let Some(type_info) = type_info {
                check_type_match(
                    &Some(type_info.namespace.clone()),
                    &type_info.name,
                    &target_namespace,
                    &target_type,
                )
            } else {
                // Default to System.Boolean for boolean values
                check_type_match(
                    &Some("System".to_string()),
                    "Boolean",
                    &target_namespace,
                    &target_type,
                )
            }
        }
        EvaluationResult::Integer(_, type_info, _) => {
            if let Some(type_info) = type_info {
                check_type_match(
                    &Some(type_info.namespace.clone()),
                    &type_info.name,
                    &target_namespace,
                    &target_type,
                )
            } else {
                // Default to System.Integer for integer values
                check_type_match(
                    &Some("System".to_string()),
                    "Integer",
                    &target_namespace,
                    &target_type,
                )
            }
        }
        EvaluationResult::Decimal(_, type_info, _) => {
            if let Some(type_info) = type_info {
                check_type_match(
                    &Some(type_info.namespace.clone()),
                    &type_info.name,
                    &target_namespace,
                    &target_type,
                )
            } else {
                // Default to System.Decimal for decimal values
                check_type_match(
                    &Some("System".to_string()),
                    "Decimal",
                    &target_namespace,
                    &target_type,
                )
            }
        }
        EvaluationResult::String(_, type_info, _) => {
            if let Some(type_info) = type_info {
                check_type_match(
                    &Some(type_info.namespace.clone()),
                    &type_info.name,
                    &target_namespace,
                    &target_type,
                )
            } else {
                // Default to System.String for string values
                check_type_match(
                    &Some("System".to_string()),
                    "String",
                    &target_namespace,
                    &target_type,
                )
            }
        }
        EvaluationResult::Date(_, type_info, _) => {
            if let Some(type_info) = type_info {
                check_type_match(
                    &Some(type_info.namespace.clone()),
                    &type_info.name,
                    &target_namespace,
                    &target_type,
                )
            } else {
                // Default to System.Date for date values
                check_type_match(
                    &Some("System".to_string()),
                    "Date",
                    &target_namespace,
                    &target_type,
                )
            }
        }
        EvaluationResult::DateTime(_, type_info, _) => {
            if let Some(type_info) = type_info {
                check_type_match(
                    &Some(type_info.namespace.clone()),
                    &type_info.name,
                    &target_namespace,
                    &target_type,
                )
            } else {
                // Default to System.DateTime for datetime values
                check_type_match(
                    &Some("System".to_string()),
                    "DateTime",
                    &target_namespace,
                    &target_type,
                )
            }
        }
        EvaluationResult::Time(_, type_info, _) => {
            if let Some(type_info) = type_info {
                check_type_match(
                    &Some(type_info.namespace.clone()),
                    &type_info.name,
                    &target_namespace,
                    &target_type,
                )
            } else {
                // Default to System.Time for time values
                check_type_match(
                    &Some("System".to_string()),
                    "Time",
                    &target_namespace,
                    &target_type,
                )
            }
        }
        EvaluationResult::Quantity(_, _, type_info, _) => {
            if let Some(type_info) = type_info {
                check_type_match(
                    &Some(type_info.namespace.clone()),
                    &type_info.name,
                    &target_namespace,
                    &target_type,
                )
            } else {
                // Default to System.Quantity for quantity values
                check_type_match(
                    &Some("System".to_string()),
                    "Quantity",
                    &target_namespace,
                    &target_type,
                )
            }
        }
        EvaluationResult::Collection { .. } => {
            // Collections are not simple types and don't match single type checks
            Ok(false)
        }
        EvaluationResult::Empty => {
            // Empty values don't match any specific type
            Ok(false)
        }
        #[cfg(not(any(feature = "R4", feature = "R4B")))]
        EvaluationResult::Integer64(_, type_info, _) => {
            if let Some(type_info) = type_info {
                check_type_match(
                    &Some(type_info.namespace.clone()),
                    &type_info.name,
                    &target_namespace,
                    &target_type,
                )
            } else {
                // Default to System.Integer64 for integer64 values
                check_type_match(
                    &Some("System".to_string()),
                    "Integer64",
                    &target_namespace,
                    &target_type,
                )
            }
        }
        #[cfg(any(feature = "R4", feature = "R4B"))]
        EvaluationResult::Integer64(_, _, _) => {
            // In R4 and R4B, Integer64 should be treated as Integer
            check_type_match(
                &Some("System".to_string()),
                "Integer",
                &target_namespace,
                &target_type,
            )
        }
        EvaluationResult::Object { map, type_info, .. } => {
            // First check if there's type_info available
            if let Some(type_info) = type_info {
                check_type_match(
                    &Some(type_info.namespace.clone()),
                    &type_info.name,
                    &target_namespace,
                    &target_type,
                )
            } else if let Some(resource_type_value) = map.get("resourceType") {
                // For FHIR resources, check the resourceType property
                if let EvaluationResult::String(resource_type, _, _) = resource_type_value {
                    // Check if the resource type matches the target type
                    check_type_match(
                        &Some("FHIR".to_string()),
                        resource_type,
                        &target_namespace,
                        &target_type,
                    )
                } else {
                    Ok(false)
                }
            } else {
                // For non-FHIR objects, we can't determine the type
                Ok(false)
            }
        }
    }
}

/// Helper function to check if two types match, considering namespace and case-insensitive comparison
fn check_type_match(
    value_namespace: &Option<String>,
    value_type: &str,
    target_namespace: &Option<String>,
    target_type: &str,
) -> Result<bool, EvaluationError> {
    // Special handling for FHIR types in is() operations
    // When target is FHIR.string, FHIR string subtypes should match
    if let (Some(value_ns), Some(target_ns)) = (value_namespace, target_namespace) {
        if value_ns.eq_ignore_ascii_case("FHIR") && target_ns.eq_ignore_ascii_case("FHIR") {
            // Use generated type hierarchy to check if value_type is a subtype of target_type
            #[cfg(feature = "R4")]
            if helios_fhir::r4::type_hierarchy::is_subtype_of(value_type, target_type) {
                return Ok(true);
            }
            #[cfg(feature = "R4B")]
            if helios_fhir::r4b::type_hierarchy::is_subtype_of(value_type, target_type) {
                return Ok(true);
            }
            #[cfg(feature = "R5")]
            if helios_fhir::r5::type_hierarchy::is_subtype_of(value_type, target_type) {
                return Ok(true);
            }
            #[cfg(feature = "R6")]
            if helios_fhir::r6::type_hierarchy::is_subtype_of(value_type, target_type) {
                return Ok(true);
            }
        }
    }

    // Special handling for FHIR primitive types matching unqualified System types in is() operations
    if target_namespace.is_none() {
        if let Some(ns) = value_namespace {
            if ns.eq_ignore_ascii_case("FHIR") {
                // Use generated type hierarchy for FHIR subtypes
                #[cfg(feature = "R4")]
                if helios_fhir::r4::type_hierarchy::is_subtype_of(value_type, target_type) {
                    if target_type.eq_ignore_ascii_case("string") {
                        eprintln!("Matched FHIR string subtype: {} -> string", value_type);
                    }
                    return Ok(true);
                }
                #[cfg(feature = "R4B")]
                if helios_fhir::r4b::type_hierarchy::is_subtype_of(value_type, target_type) {
                    if target_type.eq_ignore_ascii_case("string") {
                        eprintln!("Matched FHIR string subtype: {} -> string", value_type);
                    }
                    return Ok(true);
                }
                #[cfg(feature = "R5")]
                if helios_fhir::r5::type_hierarchy::is_subtype_of(value_type, target_type) {
                    if target_type.eq_ignore_ascii_case("string") {
                        eprintln!("Matched FHIR string subtype: {} -> string", value_type);
                    }
                    return Ok(true);
                }
                #[cfg(feature = "R6")]
                if helios_fhir::r6::type_hierarchy::is_subtype_of(value_type, target_type) {
                    if target_type.eq_ignore_ascii_case("string") {
                        eprintln!("Matched FHIR string subtype: {} -> string", value_type);
                    }
                    return Ok(true);
                }

                // Other FHIR primitive types should match their unqualified equivalents
                let direct_matches = [
                    ("boolean", "boolean"),
                    ("decimal", "decimal"),
                    ("date", "date"),
                    ("datetime", "datetime"),
                    ("time", "time"),
                    ("instant", "datetime"), // instant is a special datetime
                ];

                for (fhir_type, system_type) in &direct_matches {
                    if value_type.eq_ignore_ascii_case(fhir_type)
                        && target_type.eq_ignore_ascii_case(system_type)
                    {
                        return Ok(true);
                    }
                }
            }
        }
    }

    // For Quantity types and their subtypes, we need to allow cross-namespace matching
    // because FHIR.Age should match System.Quantity
    let allow_cross_namespace = if target_type.eq_ignore_ascii_case("quantity") {
        #[cfg(feature = "R4")]
        if helios_fhir::r4::type_hierarchy::is_subtype_of(value_type, "Quantity") {
            return check_type_match_with_cross_namespace(
                value_namespace,
                value_type,
                target_namespace,
                target_type,
                true,
            );
        }
        #[cfg(feature = "R4B")]
        if helios_fhir::r4b::type_hierarchy::is_subtype_of(value_type, "Quantity") {
            return check_type_match_with_cross_namespace(
                value_namespace,
                value_type,
                target_namespace,
                target_type,
                true,
            );
        }
        #[cfg(feature = "R5")]
        if helios_fhir::r5::type_hierarchy::is_subtype_of(value_type, "Quantity") {
            return check_type_match_with_cross_namespace(
                value_namespace,
                value_type,
                target_namespace,
                target_type,
                true,
            );
        }
        #[cfg(feature = "R6")]
        if helios_fhir::r6::type_hierarchy::is_subtype_of(value_type, "Quantity") {
            return check_type_match_with_cross_namespace(
                value_namespace,
                value_type,
                target_namespace,
                target_type,
                true,
            );
        }
        false
    } else {
        false
    };

    check_type_match_with_cross_namespace(
        value_namespace,
        value_type,
        target_namespace,
        target_type,
        allow_cross_namespace,
    )
}

/// Helper function to check if two types match with configurable cross-namespace matching
fn check_type_match_with_cross_namespace(
    value_namespace: &Option<String>,
    value_type: &str,
    target_namespace: &Option<String>,
    target_type: &str,
    allow_cross_namespace: bool,
) -> Result<bool, EvaluationError> {
    // Case-insensitive type name comparison
    let type_matches = value_type.eq_ignore_ascii_case(target_type);

    // Check FHIR type hierarchy using generated hierarchy
    let type_hierarchy_matches = if allow_cross_namespace
        || (value_namespace.as_deref() == target_namespace.as_deref()
            && value_namespace.as_deref() == Some("FHIR"))
    {
        #[cfg(feature = "R4")]
        if helios_fhir::r4::type_hierarchy::is_subtype_of(value_type, target_type) {
            return Ok(true);
        }
        #[cfg(feature = "R4B")]
        if helios_fhir::r4b::type_hierarchy::is_subtype_of(value_type, target_type) {
            return Ok(true);
        }
        #[cfg(feature = "R5")]
        if helios_fhir::r5::type_hierarchy::is_subtype_of(value_type, target_type) {
            return Ok(true);
        }
        #[cfg(feature = "R6")]
        if helios_fhir::r6::type_hierarchy::is_subtype_of(value_type, target_type) {
            return Ok(true);
        }
        false
    } else {
        false
    };

    let type_or_hierarchy_matches = type_matches || type_hierarchy_matches;

    // If no target namespace is specified, match any namespace
    if target_namespace.is_none() {
        return Ok(type_or_hierarchy_matches);
    }

    // Special case: FHIR complex types should match their System equivalents
    // For example, FHIR.Quantity should match System.Quantity
    if type_or_hierarchy_matches {
        match (value_namespace, target_namespace) {
            (Some(value_ns), Some(target_ns)) => {
                let namespace_matches = value_ns.eq_ignore_ascii_case(target_ns);

                // Allow FHIR/System cross-matching for specific types:
                // 1. Complex types like Quantity, Date, DateTime, Time (always allowed)
                // 2. Primitive types like boolean, string, integer, decimal (only for ofType operations when allow_cross_namespace is true)
                // 3. Resource types (checked dynamically)
                // Check if it's a complex type or a Quantity subtype
                let value_type_lower = value_type.to_lowercase();
                let is_complex_type = matches!(
                    value_type_lower.as_str(),
                    "quantity" | "date" | "datetime" | "time"
                );

                // Also check if it's a Quantity subtype (Age, Distance, Duration, Count)
                let is_quantity_subtype = matches!(
                    value_type_lower.as_str(),
                    "age" | "distance" | "duration" | "count"
                ) && target_type.eq_ignore_ascii_case("quantity");

                // Only allow cross-namespace matching for primitive types when explicitly enabled (for ofType operations)
                // FHIR.boolean should match the type specifier "boolean" only in ofType, not in is operations
                let is_primitive_type = allow_cross_namespace
                    && matches!(
                        value_type.to_lowercase().as_str(),
                        "boolean"
                            | "string"
                            | "integer"
                            | "decimal"
                            | "date"
                            | "datetime"
                            | "time"
                            | "instant"
                            | "id"
                            | "oid"
                            | "url"
                            | "uuid"
                            | "uri"
                            | "canonical"
                            | "code"
                            | "markdown"
                            | "base64binary"
                            | "positiveint"
                            | "unsignedint"
                    );

                let is_cross_matchable_type =
                    is_complex_type || is_primitive_type || is_quantity_subtype;

                let cross_namespace_match = is_cross_matchable_type
                    && ((value_ns.eq_ignore_ascii_case("FHIR")
                        && target_ns.eq_ignore_ascii_case("System"))
                        || (value_ns.eq_ignore_ascii_case("System")
                            && target_ns.eq_ignore_ascii_case("FHIR")));

                Ok(namespace_matches || cross_namespace_match)
            }
            (None, Some(_)) => {
                // Value has no namespace but target does - no match
                Ok(false)
            }
            _ => Ok(type_matches), // This case is already handled above
        }
    } else {
        Ok(false)
    }
}

/// Extract namespace and type name from a TypeSpecifier with context-aware resource checking
/// Handles qualified names like "System.Boolean" or "FHIR.Patient"
/// including backtick-quoted variants
pub fn extract_namespace_and_type_with_context(
    type_spec: &TypeSpecifier,
    context: &EvaluationContext,
) -> Result<(Option<String>, String), EvaluationError> {
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
        }

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
                "Boolean", "String", "Integer", "Decimal", "Date", "DateTime", "Time", "Quantity",
            ];

            // Known FHIR primitive types (lowercase)
            let fhir_primitives = [
                "boolean",
                "string",
                "integer",
                "decimal",
                "date",
                "dateTime",
                "time",
                "code",
                "id",
                "uri",
                "url",
                "canonical",
                "markdown",
                "base64Binary",
                "instant",
                "positiveInt",
                "unsignedInt",
                "uuid",
            ];

            // Check type based on capitalization to determine intended namespace
            if is_likely_system_type {
                // Capitalized names likely intended for System namespace
                if system_primitives
                    .iter()
                    .any(|&t| t.eq_ignore_ascii_case(&clean_name))
                {
                    return Ok((Some("System".to_string()), clean_name.clone()));
                }
            } else {
                // Lowercase names likely intended for FHIR namespace
                if fhir_primitives
                    .iter()
                    .any(|&t| t.eq_ignore_ascii_case(&clean_name))
                {
                    return Ok((Some("FHIR".to_string()), clean_name.to_lowercase()));
                }
            }

            // Fallback: check if it matches any primitive type regardless of case preference
            if system_primitives
                .iter()
                .any(|&t| t.eq_ignore_ascii_case(&clean_name))
            {
                // When using "is(Boolean)" or "is(Integer)", normalize to System.X
                let normalized_type = if is_likely_system_type {
                    // Keep original capitalization for System types
                    clean_name.clone()
                } else {
                    // Capitalize first letter for System types
                    capitalize_first_letter(&clean_name)
                };

                Ok((Some("System".to_string()), normalized_type))
            } else if fhir_primitives
                .iter()
                .any(|&t| t.eq_ignore_ascii_case(&clean_name))
            {
                // FHIR primitive types are conventionally lowercase
                let normalized_type = if is_likely_system_type {
                    // If capitalized, it might be a System type
                    clean_name.clone()
                } else {
                    // Otherwise keep lowercase for FHIR primitive
                    clean_name.to_lowercase()
                };

                Ok((Some("FHIR".to_string()), normalized_type))
            }
            // Use context-aware resource checking
            else if is_resource_type_for_version(&clean_name, &context.fhir_version) {
                // Resource types default to FHIR namespace
                Ok((
                    Some("FHIR".to_string()),
                    capitalize_first_letter(&clean_name),
                ))
            }
            // Check for complex types
            else if is_complex_type_for_version(&clean_name, &context.fhir_version) {
                // Complex types default to FHIR namespace
                Ok((
                    Some("FHIR".to_string()),
                    capitalize_first_letter(&clean_name),
                ))
            }
            // For unknown types, validate them
            else if !is_valid_type_name(&clean_name, &context.fhir_version) {
                // Unknown type - this should throw an error
                Err(EvaluationError::InvalidTypeSpecifier(format!(
                    "The type '{}' is unknown",
                    clean_name
                )))
            } else if is_likely_system_type {
                // Capitalized types are likely System types
                Ok((Some("System".to_string()), clean_name))
            } else {
                // Lowercase types are likely FHIR types
                Ok((Some("FHIR".to_string()), clean_name))
            }
        }
    }
}

/// Validates if a type name is a known primitive type (without version context)
fn is_primitive_type(type_name: &str) -> bool {
    // Known System primitive types
    let system_primitives = [
        "Boolean", "String", "Integer", "Decimal", "Date", "DateTime", "Time", "Quantity",
        "boolean", "string", "integer", "decimal", "date", "datetime", "time", "quantity",
    ];

    // Check against System primitives
    if system_primitives
        .iter()
        .any(|&t| t.eq_ignore_ascii_case(type_name))
    {
        return true;
    }

    // Check against FHIR primitives using the existing function
    is_fhir_primitive_type(type_name)
}

/// Validates if a type name is valid in the FHIR/System type system
fn is_valid_type_name(type_name: &str, fhir_version: &FhirVersion) -> bool {
    // Check the common types first (primitives)
    if is_primitive_type(type_name) {
        return true;
    }

    // Check if it's a valid resource type
    if is_resource_type_for_version(type_name, fhir_version) {
        return true;
    }

    // Check if it's a valid complex type
    is_complex_type_for_version(type_name, fhir_version)
}

/// Checks if a given suffix is a valid FHIR type suffix for polymorphic fields
///
/// This function checks if a suffix (like "Quantity", "CodeableConcept", etc.) is a valid
/// FHIR type that could be used as a suffix in polymorphic fields. It leverages the existing
/// type checking infrastructure rather than hard-coding type names.
///
/// # Arguments
///
/// * `suffix` - The type suffix to check (e.g., "Quantity", "CodeableConcept")
/// * `fhir_version` - The FHIR version to check against
///
/// # Returns
///
/// * `true` if the suffix is a valid FHIR type that could be used in polymorphic fields
pub fn is_valid_fhir_type_suffix(suffix: &str, fhir_version: &FhirVersion) -> bool {
    // First check if it's a primitive type (these can be suffixes)
    if is_fhir_primitive_type(suffix) {
        return true;
    }

    // Check if it's a complex type (these are commonly used as suffixes)
    if is_complex_type_for_version(suffix, fhir_version) {
        return true;
    }

    // Special case: SimpleQuantity is not a separate type but a profiled Quantity
    // It's commonly used as a suffix in polymorphic fields (e.g., doseSimpleQuantity)
    if suffix.eq_ignore_ascii_case("SimpleQuantity") {
        return true;
    }

    false
}

/// Extract namespace and type name from a TypeSpecifier (version without context)
/// Handles qualified names like "System.Boolean" or "FHIR.Patient"
/// including backtick-quoted variants
/// Note: This version cannot validate resource/complex types without FHIR version context
#[allow(dead_code)]
pub fn extract_namespace_and_type_without_context(
    type_spec: &TypeSpecifier,
) -> Result<(Option<String>, String), EvaluationError> {
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
        }

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
                "Boolean", "String", "Integer", "Decimal", "Date", "DateTime", "Time", "Quantity",
            ];

            // Known FHIR primitive types (lowercase)
            let fhir_primitives = [
                "boolean",
                "string",
                "integer",
                "decimal",
                "date",
                "dateTime",
                "time",
                "code",
                "id",
                "uri",
                "url",
                "canonical",
                "markdown",
                "base64Binary",
                "instant",
                "positiveInt",
                "unsignedInt",
                "uuid",
            ];

            // Note: Without context, we cannot check against version-specific resource/complex types
            // This legacy function relies on primitive type checking and basic heuristics

            // Check if the clean_name is a known System primitive type
            if system_primitives
                .iter()
                .any(|&t| t.eq_ignore_ascii_case(&clean_name))
            {
                // When using "is(Boolean)" or "is(Integer)", normalize to System.X
                let normalized_type = if is_likely_system_type {
                    // Keep original capitalization for System types
                    clean_name.clone()
                } else {
                    // Capitalize first letter for System types
                    capitalize_first_letter(&clean_name)
                };

                Ok((Some("System".to_string()), normalized_type))
            }
            // Check if the clean_name is a known FHIR primitive type
            else if fhir_primitives
                .iter()
                .any(|&t| t.eq_ignore_ascii_case(&clean_name))
            {
                // FHIR primitive types are conventionally lowercase
                let normalized_type = if is_likely_system_type {
                    // If capitalized, it might be a System type
                    clean_name.clone()
                } else {
                    // Otherwise keep lowercase for FHIR primitive
                    clean_name.to_lowercase()
                };

                Ok((Some("FHIR".to_string()), normalized_type))
            }
            // For non-primitive types, make an educated guess
            // But reject obviously invalid types (lowercase non-primitives that look like typos)
            else if !is_likely_system_type && clean_name.chars().any(|c| c.is_ascii_digit()) {
                // Types with digits that aren't primitives are likely invalid (e.g., "string1")
                Err(EvaluationError::InvalidTypeSpecifier(format!(
                    "The type '{}' is unknown",
                    clean_name
                )))
            } else if is_likely_system_type {
                // Capitalized types are likely System types
                Ok((Some("System".to_string()), clean_name))
            } else {
                // Lowercase types are likely FHIR types
                Ok((Some("FHIR".to_string()), clean_name))
            }
        }
    }
}

// Using capitalize_first_letter from fhir_type_hierarchy instead

/// Helper function to clean up backtick-quoted identifiers
fn clean_identifier(ident: &str) -> String {
    if ident.starts_with('`') && ident.ends_with('`') && ident.len() > 2 {
        ident[1..ident.len() - 1].to_string()
    } else {
        ident.to_string()
    }
}

/// Determines if a FHIR resource type is a DomainResource for the given FHIR version
///
/// This delegates to the macro-generated `type_hierarchy` module emitted into each
/// version's model file, which records every type's `baseDefinition` parent. A type
/// is a `DomainResource` if walking its parent chain reaches `DomainResource`
/// (true for almost all resources; `Binary`, `Bundle`, and `Parameters` derive
/// directly from `Resource` and are therefore not DomainResources).
pub fn is_fhir_domain_resource_for_version(
    resource_type: &str,
    fhir_version: &FhirVersion,
) -> bool {
    match fhir_version {
        #[cfg(feature = "R4")]
        FhirVersion::R4 => {
            helios_fhir::r4::type_hierarchy::is_subtype_of(resource_type, "DomainResource")
        }
        #[cfg(feature = "R4B")]
        FhirVersion::R4B => {
            helios_fhir::r4b::type_hierarchy::is_subtype_of(resource_type, "DomainResource")
        }
        #[cfg(feature = "R5")]
        FhirVersion::R5 => {
            helios_fhir::r5::type_hierarchy::is_subtype_of(resource_type, "DomainResource")
        }
        #[cfg(feature = "R6")]
        FhirVersion::R6 => {
            helios_fhir::r6::type_hierarchy::is_subtype_of(resource_type, "DomainResource")
        }
        #[allow(unreachable_patterns)]
        _ => false, // For versions not enabled by feature flags
    }
}

/// Determines if a FHIR resource type is a DomainResource.
///
/// Version-agnostic convenience wrapper around [`is_fhir_domain_resource_for_version`].
/// Returns `true` if the type is a DomainResource in any enabled FHIR version.
/// DomainResource membership is consistent across versions, so checking the enabled
/// versions in turn is sufficient and avoids requiring callers to thread a version
/// through type-checking code paths that don't have one.
pub fn is_fhir_domain_resource(resource_type: &str) -> bool {
    FhirVersion::enabled_versions()
        .iter()
        .any(|version| is_fhir_domain_resource_for_version(resource_type, version))
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
#[allow(dead_code)]
pub fn as_type(
    value: &EvaluationResult,
    type_spec: &TypeSpecifier,
) -> Result<EvaluationResult, EvaluationError> {
    // Check if the value is of the specified type
    if is_of_type(value, type_spec)? {
        // If it matches, return the value as-is
        Ok(value.clone())
    } else {
        // If it doesn't match, return Empty
        Ok(EvaluationResult::Empty)
    }
}

/// Casts a value to a specified type if it matches (context-aware version)
///
/// # Arguments
///
/// * `value` - The value to cast
/// * `type_spec` - The type specifier to cast to
/// * `context` - The evaluation context containing FHIR version information
///
/// # Returns
///
/// * The original value if it matches the type, Empty otherwise
pub fn as_type_with_context(
    value: &EvaluationResult,
    type_spec: &TypeSpecifier,
    context: &EvaluationContext,
) -> Result<EvaluationResult, EvaluationError> {
    // Check if the value is of the specified type using context-aware version
    if is_of_type_with_context(value, type_spec, context)? {
        // If it matches, return the value as-is
        Ok(value.clone())
    } else {
        // If it doesn't match, return Empty
        Ok(EvaluationResult::Empty)
    }
}

/// Helper function to try converting a value to a target type for ofType operations
#[allow(dead_code)]
fn try_convert_for_of_type(
    value: &EvaluationResult,
    type_spec: &TypeSpecifier,
) -> Result<Option<EvaluationResult>, EvaluationError> {
    let (_target_namespace, target_type) = extract_namespace_and_type_without_context(type_spec)?;

    match value {
        EvaluationResult::String(s, type_info, _) => {
            // Check if this is a FHIR string that might represent a different type
            if let Some(type_info) = type_info {
                if type_info.namespace == "FHIR" && type_info.name == "string" {
                    // Try to convert FHIR strings to target types
                    match target_type.to_lowercase().as_str() {
                        "datetime" => {
                            // Only convert to datetime if it looks like a date or datetime, not a time
                            if s.len() == 10
                                && s.chars().nth(4) == Some('-')
                                && s.chars().nth(7) == Some('-')
                            {
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
                            if s.contains('T')
                                && (s.contains('+') || s.contains('-') || s.ends_with('Z'))
                            {
                                Ok(Some(EvaluationResult::fhir_string(s.clone(), "instant")))
                            } else {
                                Ok(None)
                            }
                        }
                        "id" => {
                            // Only convert simple strings to ID, not URLs or other special formats
                            if s.starts_with("http://")
                                || s.starts_with("https://")
                                || s.starts_with("urn:")
                            {
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
                            if s.starts_with("urn:")
                                || s.starts_with("http://")
                                || s.starts_with("https://")
                            {
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
                            Ok(Some(EvaluationResult::fhir_string(
                                s.clone(),
                                "base64Binary",
                            )))
                        }
                        _ => Ok(None),
                    }
                } else {
                    Ok(None)
                }
            } else {
                Ok(None)
            }
        }
        _ => Ok(None),
    }
}

/// Filters a collection based on a type specifier (context-aware version)
///
/// # Arguments
///
/// * `collection` - The collection to filter
/// * `type_spec` - The type to filter by
/// * `context` - The evaluation context containing FHIR version information
///
/// # Returns
///
/// * A new collection containing only items of the specified type
/// * If there's only one item in the collection, returns that item directly (unwrapped)
/// * If the collection is empty, returns Empty
pub fn of_type_with_context(
    collection: &EvaluationResult,
    type_spec: &TypeSpecifier,
    context: &EvaluationContext,
) -> Result<EvaluationResult, EvaluationError> {
    // Use a consistent helper function for applying the type filter
    let apply_type_filter =
        |items: &[EvaluationResult]| -> Result<EvaluationResult, EvaluationError> {
            let mut result = Vec::new();

            for item in items {
                // Use context-aware type checking
                if is_of_type_with_context(item, type_spec, context)? {
                    result.push(item.clone());
                }
                // Note: try_convert_for_of_type uses the without-context version
                // which can't validate complex types, so we skip it for now
            }

            if result.is_empty() {
                Ok(EvaluationResult::Empty)
            } else if result.len() == 1 {
                Ok(result[0].clone())
            } else {
                // ofType preserves the order of the input collection
                let input_was_unordered = matches!(
                    collection,
                    EvaluationResult::Collection {
                        has_undefined_order: true,
                        ..
                    }
                );
                Ok(EvaluationResult::Collection {
                    items: result,
                    has_undefined_order: input_was_unordered,
                    type_info: None,
                })
            }
        };

    match collection {
        EvaluationResult::Collection { items, .. } => apply_type_filter(items), // Destructure
        EvaluationResult::Empty => Ok(EvaluationResult::Empty),

        // For a singleton value, treat it like a collection of one
        _ => {
            if is_of_type_with_context(collection, type_spec, context)? {
                // Return the value directly for a singleton that matches
                Ok(collection.clone())
            } else {
                Ok(EvaluationResult::Empty)
            }
        }
    }
}

/// Filters a collection based on a type specifier (version without context)
/// This version is deprecated - use of_type_with_context instead
#[allow(dead_code)]
pub fn of_type(
    collection: &EvaluationResult,
    type_spec: &TypeSpecifier,
) -> Result<EvaluationResult, EvaluationError> {
    // Use a consistent helper function for applying the type filter
    let apply_type_filter =
        |items: &[EvaluationResult]| -> Result<EvaluationResult, EvaluationError> {
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
                let input_was_unordered = matches!(
                    collection,
                    EvaluationResult::Collection {
                        has_undefined_order: true,
                        ..
                    }
                );
                Ok(EvaluationResult::Collection {
                    items: result,
                    has_undefined_order: input_was_unordered,
                    type_info: None,
                })
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
        patient.insert(
            "resourceType".to_string(),
            EvaluationResult::string("Patient".to_string()),
        );
        patient.insert(
            "id".to_string(),
            EvaluationResult::string("123".to_string()),
        );
        patient.insert("active".to_string(), EvaluationResult::boolean(true));

        // Add a name
        let mut name = HashMap::new();
        name.insert(
            "use".to_string(),
            EvaluationResult::string("official".to_string()),
        );
        name.insert(
            "family".to_string(),
            EvaluationResult::string("Smith".to_string()),
        );

        let given = vec![
            EvaluationResult::string("John".to_string()),
            EvaluationResult::string("Q".to_string()),
        ];
        name.insert(
            "given".to_string(),
            EvaluationResult::Collection {
                items: given,
                has_undefined_order: false,
                type_info: None,
            },
        );

        let names = vec![EvaluationResult::Object {
            map: name,
            type_info: None,
        }];
        patient.insert(
            "name".to_string(),
            EvaluationResult::Collection {
                items: names,
                has_undefined_order: false,
                type_info: None,
            },
        );

        // Add birthDate
        patient.insert(
            "birthDate".to_string(),
            EvaluationResult::string("1974-12-25".to_string()),
        );

        EvaluationResult::Object {
            map: patient,
            type_info: None,
        }
    }

    // Helper function to create a FHIR Observation resource with a valueQuantity
    fn create_observation() -> EvaluationResult {
        let mut obs = HashMap::new();
        obs.insert(
            "resourceType".to_string(),
            EvaluationResult::string("Observation".to_string()),
        );
        obs.insert(
            "id".to_string(),
            EvaluationResult::string("456".to_string()),
        );
        obs.insert(
            "status".to_string(),
            EvaluationResult::string("final".to_string()),
        );

        // Add valueQuantity
        let mut quantity = HashMap::new();
        quantity.insert(
            "value".to_string(),
            EvaluationResult::decimal(rust_decimal::Decimal::from(185)),
        );
        quantity.insert(
            "unit".to_string(),
            EvaluationResult::string("lbs".to_string()),
        );
        quantity.insert(
            "system".to_string(),
            EvaluationResult::string("http://unitsofmeasure.org".to_string()),
        );
        quantity.insert(
            "code".to_string(),
            EvaluationResult::string("lb_av".to_string()),
        );

        obs.insert(
            "valueQuantity".to_string(),
            EvaluationResult::Object {
                map: quantity,
                type_info: None,
            },
        );

        EvaluationResult::Object {
            map: obs,
            type_info: None,
        }
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
        if let EvaluationResult::Object {
            map: obj,
            type_info: None,
        } = &patient
        {
            eprintln!("Patient object:");
            for (key, value) in obj {
                eprintln!("  {}: {:?}", key, value);
            }
        }

        // Create type specifiers with exact case matching the resourceType property
        // Since Patient is a FHIR resource, we need to specify the FHIR namespace
        let patient_type =
            TypeSpecifier::QualifiedIdentifier("FHIR".to_string(), Some("Patient".to_string()));

        // Print the type specifier for debugging
        eprintln!("Patient type: {:?}", patient_type);

        let is_result = is_of_type(&patient, &patient_type);
        eprintln!("is_of_type result: {:?}", is_result);

        // Test correct matches
        assert!(is_result.unwrap());

        // Create the rest of the type specifiers
        let obs_type =
            TypeSpecifier::QualifiedIdentifier("FHIR".to_string(), Some("Observation".to_string()));
        let fhir_patient_type =
            TypeSpecifier::QualifiedIdentifier("FHIR".to_string(), Some("Patient".to_string()));

        assert!(is_of_type(&observation, &obs_type).unwrap());
        assert!(is_of_type(&patient, &fhir_patient_type).unwrap());

        // Test with different casing (should still work with case-insensitive comparison)
        let patient_type_lowercase =
            TypeSpecifier::QualifiedIdentifier("fhir".to_string(), Some("patient".to_string()));
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
        let patient_type =
            TypeSpecifier::QualifiedIdentifier("FHIR".to_string(), Some("Patient".to_string()));
        let obs_type =
            TypeSpecifier::QualifiedIdentifier("FHIR".to_string(), Some("Observation".to_string()));

        // Test correct casts
        assert_eq!(as_type(&bool_val, &bool_type).unwrap(), bool_val);
        assert_eq!(as_type(&patient, &patient_type).unwrap(), patient);
        assert_eq!(as_type(&observation, &obs_type).unwrap(), observation);

        // Test with different casing (should still work)
        let patient_type_lowercase =
            TypeSpecifier::QualifiedIdentifier("fhir".to_string(), Some("patient".to_string()));
        assert_eq!(as_type(&patient, &patient_type_lowercase).unwrap(), patient);

        // Test incorrect casts
        assert_eq!(
            as_type(&bool_val, &patient_type).unwrap(),
            EvaluationResult::Empty
        );
        assert_eq!(
            as_type(&patient, &bool_type).unwrap(),
            EvaluationResult::Empty
        );
        assert_eq!(
            as_type(&observation, &patient_type).unwrap(),
            EvaluationResult::Empty
        );
    }

    #[test]
    fn test_of_type() {
        // Create a collection with mixed types
        let collection = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::boolean(true),
                EvaluationResult::integer(42),
                EvaluationResult::boolean(false),
                EvaluationResult::string("test".to_string()),
            ],
            has_undefined_order: false,
            type_info: None,
        };

        // Create type specifiers
        let bool_type = TypeSpecifier::QualifiedIdentifier("Boolean".to_string(), None);
        let int_type = TypeSpecifier::QualifiedIdentifier("Integer".to_string(), None);
        let str_type = TypeSpecifier::QualifiedIdentifier("String".to_string(), None);

        // Test filtering with multiple matches - should return a collection
        let collection_with_only_booleans = EvaluationResult::Collection {
            items: vec![
                EvaluationResult::boolean(true),
                EvaluationResult::boolean(false),
            ],
            has_undefined_order: false,
            type_info: None,
        };
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
