//! # FHIRPath Type Information
//!
//! Provides type metadata support for FHIRPath's `type()` function and type system.
//! This module defines structures for representing type information including
//! namespace and type name.

/// Type information result for FHIRPath type() function
#[derive(Debug, Clone, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct TypeInfoResult {
    pub namespace: String,
    pub name: String,
}

impl TypeInfoResult {
    pub fn new(namespace: &str, name: &str) -> Self {
        Self {
            namespace: namespace.to_string(),
            name: name.to_string(),
        }
    }
}

/// Trait for types that can provide their type information for FHIRPath.
///
/// This trait is implemented by FHIR types to provide namespace and name
/// information used by the FHIRPath type() function and type operations.
pub trait TypeInfo {
    /// Returns the namespace for this type (e.g., "FHIR", "System").
    fn type_namespace() -> &'static str;

    /// Returns the name of this type within its namespace.
    fn type_name() -> &'static str;
}
