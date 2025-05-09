# Type System Implementation Fix

This document provides a step-by-step implementation guide for fixing the FHIRPath type system and is/as operations. It focuses on the highest priority issues identified in failing tests.

## 1. Unified Type System Implementation

### 1.1. Enhanced Type Hierarchy Module

First, we need to enhance the `fhir_type_hierarchy.rs` module with a more complete type hierarchy and better namespace handling.

```rust
// fhir_type_hierarchy.rs

use once_cell::sync::Lazy;
use std::collections::{HashMap, HashSet};

// Define namespace constants
pub const SYSTEM_NAMESPACE: &str = "System";
pub const FHIR_NAMESPACE: &str = "FHIR";

// Represents a fully qualified type in the FHIRPath type system
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeInfo {
    pub namespace: String,
    pub name: String,
}

impl TypeInfo {
    pub fn new(namespace: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            namespace: namespace.into(),
            name: name.into(),
        }
    }
    
    pub fn system(name: impl Into<String>) -> Self {
        Self::new(SYSTEM_NAMESPACE, name)
    }
    
    pub fn fhir(name: impl Into<String>) -> Self {
        Self::new(FHIR_NAMESPACE, name)
    }
    
    // Return a canonical representation for consistent comparison
    pub fn canonical(&self) -> String {
        format!("{}.{}", self.namespace, self.name)
    }
}

// Expand the resource type hierarchy to include ALL resource types
// This is a more comprehensive map than the current implementation
static RESOURCE_TYPE_HIERARCHY: Lazy<HashMap<&'static str, &'static str>> = Lazy::new(|| {
    let mut h = HashMap::new();
    // Base types
    h.insert("DomainResource", "Resource");
    
    // Add administrative resources
    h.insert("Patient", "DomainResource");
    h.insert("Practitioner", "DomainResource");
    h.insert("Organization", "DomainResource");
    h.insert("Group", "DomainResource");
    h.insert("Person", "DomainResource");
    h.insert("RelatedPerson", "DomainResource");
    h.insert("HealthcareService", "DomainResource");
    h.insert("Location", "DomainResource");
    h.insert("Endpoint", "DomainResource");
    
    // Add clinical resources
    h.insert("Observation", "DomainResource");
    h.insert("Condition", "DomainResource");
    h.insert("Procedure", "DomainResource");
    h.insert("MedicationRequest", "DomainResource");
    h.insert("DiagnosticReport", "DomainResource");
    h.insert("CarePlan", "DomainResource");
    
    // ... add all other FHIR R4 resources ...
    
    h
});

// Improve FHIR primitive type handling with type information
static FHIR_PRIMITIVE_TYPES: Lazy<HashMap<&'static str, TypeInfo>> = Lazy::new(|| {
    let mut m = HashMap::new();
    
    // Map FHIR primitive types to their System equivalents
    m.insert("boolean", TypeInfo::system("Boolean"));
    m.insert("string", TypeInfo::system("String"));
    m.insert("decimal", TypeInfo::system("Decimal"));
    m.insert("integer", TypeInfo::system("Integer"));
    m.insert("positiveInt", TypeInfo::system("Integer"));
    m.insert("unsignedInt", TypeInfo::system("Integer"));
    m.insert("date", TypeInfo::system("Date"));
    m.insert("dateTime", TypeInfo::system("DateTime"));
    m.insert("time", TypeInfo::system("Time"));
    m.insert("instant", TypeInfo::system("DateTime"));
    m.insert("code", TypeInfo::system("String"));
    m.insert("oid", TypeInfo::system("String"));
    m.insert("id", TypeInfo::system("String"));
    m.insert("markdown", TypeInfo::system("String"));
    m.insert("uri", TypeInfo::system("String"));
    m.insert("url", TypeInfo::system("String"));
    m.insert("canonical", TypeInfo::system("String"));
    
    m
});

// Handle FHIR complex types with inheritance information
static FHIR_COMPLEX_TYPES: Lazy<HashMap<&'static str, &'static str>> = Lazy::new(|| {
    let mut h = HashMap::new();
    
    // Element is the base type for all complex types
    h.insert("Element", "");
    h.insert("BackboneElement", "Element");
    
    // Define complex type hierarchy
    h.insert("Identifier", "Element");
    h.insert("HumanName", "Element");
    h.insert("Address", "Element");
    h.insert("ContactPoint", "Element");
    h.insert("Timing", "Element");
    h.insert("Quantity", "Element");
    h.insert("Period", "Element");
    h.insert("Range", "Element");
    h.insert("Ratio", "Element");
    h.insert("SampledData", "Element");
    h.insert("Attachment", "Element");
    h.insert("CodeableConcept", "Element");
    h.insert("Coding", "Element");
    h.insert("Money", "Quantity");
    h.insert("Count", "Quantity");
    h.insert("Distance", "Quantity");
    h.insert("Duration", "Quantity");
    h.insert("Age", "Quantity");
    
    // ... add other complex types ...
    
    h
});

/// Improved function to check if a type is a FHIR primitive type
pub fn is_fhir_primitive_type(type_name: &str) -> bool {
    FHIR_PRIMITIVE_TYPES.contains_key(&type_name.to_lowercase())
}

/// Get the System type equivalent for a FHIR primitive type
pub fn get_system_type_for_fhir_primitive(type_name: &str) -> Option<TypeInfo> {
    FHIR_PRIMITIVE_TYPES.get(&type_name.to_lowercase()).cloned()
}

/// Check if a type is a FHIR complex type
pub fn is_fhir_complex_type(type_name: &str) -> bool {
    FHIR_COMPLEX_TYPES.contains_key(&capitalize_first_letter(type_name))
}

/// Check if a type is a FHIR resource type
pub fn is_fhir_resource_type(type_name: &str) -> bool {
    RESOURCE_TYPE_HIERARCHY.contains_key(&capitalize_first_letter(type_name)) ||
    capitalize_first_letter(type_name) == "Resource"
}

/// Determine if a type is derived from a parent type
/// This is used for type hierarchy checks (e.g., Patient is Resource)
pub fn is_derived_from(type_name: &str, parent_type: &str) -> bool {
    // Handle case-insensitive direct equality
    if type_name.eq_ignore_ascii_case(parent_type) {
        return true;
    }
    
    // Normalize types to canonical form (capitalized)
    let normalized_type = capitalize_first_letter(type_name);
    let normalized_parent = capitalize_first_letter(parent_type);
    
    // Check resources
    if is_fhir_resource_type(&normalized_type) && is_fhir_resource_type(&normalized_parent) {
        // Special case for "Resource" as the root type
        if normalized_parent == "Resource" {
            return true;
        }
        
        // Follow the resource type hierarchy
        let mut current = normalized_type.as_str();
        while let Some(&parent) = RESOURCE_TYPE_HIERARCHY.get(current) {
            if parent.eq_ignore_ascii_case(&normalized_parent) {
                return true;
            }
            current = parent;
        }
    }
    
    // Check complex types
    if is_fhir_complex_type(&normalized_type) && is_fhir_complex_type(&normalized_parent) {
        // Follow the complex type hierarchy
        let mut current = normalized_type.as_str();
        while let Some(&parent) = FHIR_COMPLEX_TYPES.get(current) {
            if parent.is_empty() {
                // Reached the root with no match
                break;
            }
            if parent.eq_ignore_ascii_case(&normalized_parent) {
                return true;
            }
            current = parent;
        }
    }
    
    false
}

/// Determine the namespace for a type name based on conventions
pub fn determine_type_namespace(type_name: &str) -> &'static str {
    // If first letter is lowercase, it's likely a FHIR primitive
    let first_char = type_name.chars().next();
    if let Some(c) = first_char {
        if c.is_lowercase() && is_fhir_primitive_type(type_name) {
            return FHIR_NAMESPACE;
        }
    }
    
    // If it's a FHIR resource type, it's in the FHIR namespace
    if is_fhir_resource_type(type_name) {
        return FHIR_NAMESPACE;
    }
    
    // If it's a FHIR complex type, it's in the FHIR namespace
    if is_fhir_complex_type(type_name) {
        return FHIR_NAMESPACE;
    }
    
    // Default to System namespace
    SYSTEM_NAMESPACE
}

/// Utility function to capitalize the first letter of a string
pub fn capitalize_first_letter(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

/// Parse a type specifier into a TypeInfo
pub fn parse_type_specifier(namespace: Option<&str>, type_name: &str) -> TypeInfo {
    // Handle backtick-quoted names (remove backticks)
    let cleaned_type = type_name.replace('`', "");
    
    // If namespace is provided, use it
    if let Some(ns) = namespace {
        return TypeInfo::new(ns, cleaned_type);
    }
    
    // Otherwise, determine namespace based on the type name
    let ns = determine_type_namespace(&cleaned_type);
    TypeInfo::new(ns, cleaned_type)
}

/// Check if two TypeInfo objects represent compatible types
pub fn are_types_compatible(type1: &TypeInfo, type2: &TypeInfo) -> bool {
    // For primitives, check System <-> FHIR mappings
    if type1.namespace == FHIR_NAMESPACE && is_fhir_primitive_type(&type1.name) {
        if let Some(system_type) = get_system_type_for_fhir_primitive(&type1.name) {
            if system_type.name.eq_ignore_ascii_case(&type2.name) && 
               type2.namespace == SYSTEM_NAMESPACE {
                return true;
            }
        }
    } else if type2.namespace == FHIR_NAMESPACE && is_fhir_primitive_type(&type2.name) {
        if let Some(system_type) = get_system_type_for_fhir_primitive(&type2.name) {
            if system_type.name.eq_ignore_ascii_case(&type1.name) && 
               type1.namespace == SYSTEM_NAMESPACE {
                return true;
            }
        }
    }
    
    // For non-primitives, check if either type derives from the other
    is_derived_from(&type1.name, &type2.name) || is_derived_from(&type2.name, &type1.name)
}
```

### 1.2. Update Resource Type Module

Next, update the `resource_type.rs` module to use the enhanced type hierarchy:

```rust
// resource_type.rs

use crate::fhir_type_hierarchy::{
    TypeInfo, SYSTEM_NAMESPACE, FHIR_NAMESPACE, 
    is_derived_from, parse_type_specifier, are_types_compatible,
    is_fhir_primitive_type, is_fhir_complex_type, is_fhir_resource_type
};
use fhirpath_support::{EvaluationResult, EvaluationError};
use crate::parser::TypeSpecifier;
use std::collections::HashMap;

/// Extract namespace and type name from a type specifier
pub fn extract_namespace_and_type(type_spec: &TypeSpecifier) -> Result<(Option<String>, String), EvaluationError> {
    match type_spec {
        TypeSpecifier::Identifier(name) => {
            // Remove any backticks from the identifier
            let clean_name = name.replace('`', "");
            Ok((None, clean_name))
        },
        TypeSpecifier::QualifiedIdentifier(name, ns_opt) => {
            let clean_name = name.replace('`', "");
            Ok((ns_opt.clone(), clean_name))
        }
    }
}

/// Check if a value is of the specified type
pub fn is_of_type(value: &EvaluationResult, type_spec: &TypeSpecifier) -> Result<bool, EvaluationError> {
    // Extract namespace and type name
    let (namespace, type_name) = extract_namespace_and_type(type_spec)?;
    
    // Parse into a TypeInfo object
    let target_type = parse_type_specifier(namespace.as_deref(), &type_name);
    
    // Get the actual type of the value
    let actual_type = get_value_type_info(value)?;
    
    match actual_type {
        Some(actual) => {
            // Check direct equality first (case-insensitive)
            if actual.name.eq_ignore_ascii_case(&target_type.name) && 
               (actual.namespace.eq_ignore_ascii_case(&target_type.namespace) || 
                namespace.is_none()) {
                return Ok(true);
            }
            
            // Check type compatibility (e.g., System.Boolean <-> FHIR.boolean)
            if are_types_compatible(&actual, &target_type) {
                return Ok(true);
            }
            
            // Check for inheritance relationships
            if actual.namespace == FHIR_NAMESPACE && target_type.namespace == FHIR_NAMESPACE {
                // FHIR resource/type inheritance
                return Ok(is_derived_from(&actual.name, &target_type.name));
            }
            
            // No match
            Ok(false)
        },
        None => Ok(false) // Empty or unrecognized type
    }
}

/// Cast a value to the specified type if possible
pub fn as_type(value: &EvaluationResult, type_spec: &TypeSpecifier) -> Result<EvaluationResult, EvaluationError> {
    // Check if the value is of the specified type
    if is_of_type(value, type_spec)? {
        // If yes, return the value as is
        Ok(value.clone())
    } else {
        // If not, return Empty
        Ok(EvaluationResult::Empty)
    }
}

/// Filter a collection to only include items of the specified type
pub fn of_type(collection: &EvaluationResult, type_spec: &TypeSpecifier) -> Result<EvaluationResult, EvaluationError> {
    match collection {
        EvaluationResult::Collection(items) => {
            let mut filtered = Vec::new();
            for item in items {
                if is_of_type(item, type_spec)? {
                    filtered.push(item.clone());
                }
            }
            if filtered.is_empty() {
                Ok(EvaluationResult::Empty)
            } else {
                Ok(EvaluationResult::Collection(filtered))
            }
        },
        // For singleton values, apply is_of_type directly
        _ => {
            if is_of_type(collection, type_spec)? {
                Ok(collection.clone())
            } else {
                Ok(EvaluationResult::Empty)
            }
        }
    }
}

/// Get the type information for a value
fn get_value_type_info(value: &EvaluationResult) -> Result<Option<TypeInfo>, EvaluationError> {
    match value {
        EvaluationResult::Boolean(_) => Ok(Some(TypeInfo::system("Boolean"))),
        EvaluationResult::String(_) => Ok(Some(TypeInfo::system("String"))),
        EvaluationResult::Integer(_) => Ok(Some(TypeInfo::system("Integer"))),
        EvaluationResult::Decimal(_) => Ok(Some(TypeInfo::system("Decimal"))),
        EvaluationResult::Date(_) => Ok(Some(TypeInfo::system("Date"))),
        EvaluationResult::DateTime(_) => Ok(Some(TypeInfo::system("DateTime"))),
        EvaluationResult::Time(_) => Ok(Some(TypeInfo::system("Time"))),
        EvaluationResult::Quantity(_, _) => Ok(Some(TypeInfo::system("Quantity"))),
        EvaluationResult::Object(obj) => {
            // Check for FHIR resource type
            if let Some(EvaluationResult::String(resource_type)) = obj.get("resourceType") {
                return Ok(Some(TypeInfo::fhir(resource_type)));
            }
            
            // Try to determine complex type by properties
            if is_complex_type_by_properties(obj) {
                return Ok(Some(TypeInfo::fhir("Element")));
            }
            
            // Generic object
            Ok(Some(TypeInfo::system("Object")))
        },
        EvaluationResult::Collection(_) => Ok(Some(TypeInfo::system("Collection"))),
        EvaluationResult::Empty => Ok(None),
    }
}

/// Try to determine if an object is a FHIR complex type by examining its properties
fn is_complex_type_by_properties(obj: &HashMap<String, EvaluationResult>) -> bool {
    // Check for common FHIR complex type properties
    
    // Identifier has system, value properties
    if obj.contains_key("system") && obj.contains_key("value") {
        return true;
    }
    
    // HumanName has family, given properties
    if obj.contains_key("family") || obj.contains_key("given") {
        return true;
    }
    
    // Quantity has value, unit properties
    if obj.contains_key("value") && obj.contains_key("unit") {
        return true;
    }
    
    // Period has start, end properties
    if obj.contains_key("start") || obj.contains_key("end") {
        return true;
    }
    
    // Default to false if no specific type detected
    false
}
```

### 1.3. Update Type Function Module

Update the `type_function.rs` module to use our improved type system:

```rust
// type_function.rs

use crate::fhir_type_hierarchy::{TypeInfo, SYSTEM_NAMESPACE, FHIR_NAMESPACE};
use fhirpath_support::{EvaluationResult, EvaluationError};
use std::collections::HashMap;

/// Get type information for a value
pub fn get_type_info(result: &EvaluationResult) -> Result<TypeInfo, EvaluationError> {
    match result {
        EvaluationResult::Boolean(_) => Ok(TypeInfo::system("Boolean")),
        EvaluationResult::String(_) => Ok(TypeInfo::system("String")),
        EvaluationResult::Integer(_) => Ok(TypeInfo::system("Integer")),
        EvaluationResult::Decimal(_) => Ok(TypeInfo::system("Decimal")),
        EvaluationResult::Date(_) => Ok(TypeInfo::system("Date")),
        EvaluationResult::DateTime(_) => Ok(TypeInfo::system("DateTime")),
        EvaluationResult::Time(_) => Ok(TypeInfo::system("Time")),
        EvaluationResult::Quantity(_, _) => Ok(TypeInfo::system("Quantity")),
        EvaluationResult::Collection(_) => Ok(TypeInfo::system("Collection")),
        EvaluationResult::Object(obj) => {
            // Check for FHIR resource type
            if let Some(EvaluationResult::String(resource_type)) = obj.get("resourceType") {
                Ok(TypeInfo::fhir(resource_type))
            } else {
                // Try to determine complex type
                determine_complex_type(obj)
            }
        },
        EvaluationResult::Empty => Err(EvaluationError::TypeError(
            "Cannot determine type of Empty value".to_string()
        )),
    }
}

/// Get just the name of a type (for backward compatibility)
pub fn get_type_name(result: &EvaluationResult) -> Result<String, EvaluationError> {
    match get_type_info(result) {
        Ok(type_info) => Ok(type_info.name),
        Err(e) => Err(e),
    }
}

/// Try to determine the specific complex type
fn determine_complex_type(obj: &HashMap<String, EvaluationResult>) -> Result<TypeInfo, EvaluationError> {
    // Check for common FHIR complex types
    
    // Identifier
    if obj.contains_key("system") && obj.contains_key("value") {
        return Ok(TypeInfo::fhir("Identifier"));
    }
    
    // HumanName
    if obj.contains_key("family") || obj.contains_key("given") {
        return Ok(TypeInfo::fhir("HumanName"));
    }
    
    // Quantity
    if obj.contains_key("value") && obj.contains_key("unit") {
        return Ok(TypeInfo::fhir("Quantity"));
    }
    
    // Period
    if obj.contains_key("start") || obj.contains_key("end") {
        return Ok(TypeInfo::fhir("Period"));
    }
    
    // CodeableConcept
    if obj.contains_key("coding") && obj.contains_key("text") {
        return Ok(TypeInfo::fhir("CodeableConcept"));
    }
    
    // Default to generic FHIR Element
    Ok(TypeInfo::fhir("Element"))
}

/// Implementation of the type() function
pub fn type_function(value: &EvaluationResult) -> Result<EvaluationResult, EvaluationError> {
    match value {
        EvaluationResult::Collection(items) => {
            // For collections, return a collection of type information
            let mut type_results = Vec::new();
            for item in items {
                match type_function(item) {
                    Ok(result) => type_results.push(result),
                    Err(_) => () // Skip items that don't have a valid type
                }
            }
            
            if type_results.is_empty() {
                Ok(EvaluationResult::Empty)
            } else {
                Ok(EvaluationResult::Collection(type_results))
            }
        },
        EvaluationResult::Empty => Ok(EvaluationResult::Empty),
        _ => {
            // Get type info and convert to an object
            let type_info = get_type_info(value)?;
            
            // Create object with namespace and name properties
            let mut obj = HashMap::new();
            obj.insert("namespace".to_string(), EvaluationResult::String(type_info.namespace));
            obj.insert("name".to_string(), EvaluationResult::String(type_info.name));
            
            Ok(EvaluationResult::Object(obj))
        }
    }
}

/// Full type function implementation that takes a context for testing
pub fn type_function_full(value: &EvaluationResult) -> Result<EvaluationResult, EvaluationError> {
    type_function(value)
}
```

### 1.4. Fix Type Operation Application

Update the way type operations are applied in `apply_type_operation.rs`:

```rust
// apply_type_operation.rs

use crate::parser::TypeSpecifier;
use fhirpath_support::{EvaluationResult, EvaluationError};
use crate::resource_type::{is_of_type, as_type};

/// Applies a type operator (is, as) to a value
pub fn apply_type_operation(
    value: &EvaluationResult,
    operator: &str,
    type_spec: &TypeSpecifier,
) -> Result<EvaluationResult, EvaluationError> {
    // Ensure value is a singleton (not a collection) - type operations can't be applied to collections
    // Extract singleton value if it's a single-item collection
    let singleton_value = match value {
        EvaluationResult::Collection(items) if items.len() == 1 => &items[0],
        EvaluationResult::Collection(_) => {
            return Err(EvaluationError::TypeError(
                format!("Cannot apply type operator '{}' to a collection with multiple items", operator)
            ));
        },
        _ => value,
    };
    
    // Now apply the appropriate type operation
    match operator {
        "is" => {
            // Check if the value is of the specified type
            let is_result = is_of_type(singleton_value, type_spec)?;
            Ok(EvaluationResult::Boolean(is_result))
        },
        "as" => {
            // Cast the value to the specified type if possible
            as_type(singleton_value, type_spec)
        },
        _ => Err(EvaluationError::InvalidOperation(
            format!("Unsupported type operator: {}", operator)
        )),
    }
}
```

## 2. Polymorphic Access Improvements

### 2.1. Enhanced Polymorphic Access Module

Update the `polymorphic_access.rs` module for better choice element handling:

```rust
// polymorphic_access.rs

use std::collections::HashMap;
use fhirpath_support::{EvaluationResult, EvaluationError};
use crate::parser::TypeSpecifier;
use crate::resource_type::{is_of_type, as_type, extract_namespace_and_type};
use crate::fhir_type_hierarchy::{parse_type_specifier};

/// Check if a field is a choice element by looking for typed variants
pub fn is_choice_element(obj: &HashMap<String, EvaluationResult>, field_name: &str) -> bool {
    // Don't rely just on a static list - actually check for the pattern
    
    // Special case for common choice elements we know
    if field_name == "value" || field_name == "effective" || field_name == "onset" {
        // These are frequently used choice elements in FHIR
        return true;
    }
    
    // Check for [x] suffix in the original field name
    if field_name.ends_with("[x]") {
        return true;
    }
    
    // Look for the pattern fieldNameType in the object keys
    for key in obj.keys() {
        if key.len() > field_name.len() && 
           key.starts_with(field_name) &&
           key.chars().nth(field_name.len())
               .map_or(false, |c| c.is_uppercase()) {
            return true;
        }
    }
    
    false
}

/// Access a polymorphic element by field name
pub fn access_polymorphic_element(obj: &HashMap<String, EvaluationResult>, field_name: &str) -> EvaluationResult {
    // Try direct access first
    if let Some(value) = obj.get(field_name) {
        return value.clone();
    }
    
    // Remove [x] suffix if present
    let base_name = if field_name.ends_with("[x]") {
        &field_name[..field_name.len() - 3]
    } else {
        field_name
    };
    
    // Look for matching choice element (fieldNameType pattern)
    for (key, value) in obj.iter() {
        if key.starts_with(base_name) && 
           key.len() > base_name.len() && 
           key.chars().nth(base_name.len())
               .map_or(false, |c| c.is_uppercase()) {
            return value.clone();
        }
    }
    
    // Not found
    EvaluationResult::Empty
}

/// Get the type name of a choice element
pub fn get_choice_element_type(obj: &HashMap<String, EvaluationResult>, field_name: &str) -> Option<String> {
    // Remove [x] suffix if present
    let base_name = if field_name.ends_with("[x]") {
        &field_name[..field_name.len() - 3]
    } else {
        field_name
    };
    
    // Look for matching choice element (fieldNameType pattern)
    for key in obj.keys() {
        if key.starts_with(base_name) && 
           key.len() > base_name.len() && 
           key.chars().nth(base_name.len())
               .map_or(false, |c| c.is_uppercase()) {
            // Extract the type name from the suffix (e.g., "valueQuantity" -> "Quantity")
            return Some(key[base_name.len()..].to_string());
        }
    }
    
    None
}

/// Apply a type operation to a polymorphic element
pub fn apply_polymorphic_type_operation(
    obj: &HashMap<String, EvaluationResult>,
    field_name: &str,
    operator: &str,
    type_spec: &TypeSpecifier,
) -> Result<EvaluationResult, EvaluationError> {
    // Extract the target type information
    let (namespace, type_name) = extract_namespace_and_type(type_spec)?;
    let target_type = parse_type_specifier(namespace.as_deref(), &type_name);
    
    // Get the actual type of the choice element
    if let Some(actual_type) = get_choice_element_type(obj, field_name) {
        match operator {
            "is" => {
                // Check if types match (case-insensitive)
                let is_match = actual_type.eq_ignore_ascii_case(&target_type.name);
                Ok(EvaluationResult::Boolean(is_match))
            },
            "as" => {
                if actual_type.eq_ignore_ascii_case(&target_type.name) {
                    // Types match, return the value
                    access_polymorphic_element(obj, field_name)
                } else {
                    // Types don't match, return Empty
                    Ok(EvaluationResult::Empty)
                }
            },
            _ => Err(EvaluationError::InvalidOperation(
                format!("Unsupported type operator for polymorphic element: {}", operator)
            )),
        }
    } else {
        // No choice element found
        match operator {
            "is" => Ok(EvaluationResult::Boolean(false)),
            "as" => Ok(EvaluationResult::Empty),
            _ => Err(EvaluationError::InvalidOperation(
                format!("Unsupported type operator for polymorphic element: {}", operator)
            )),
        }
    }
}
```

## 3. Integration with Evaluator

Finally, we need to update the evaluator to properly handle type expressions:

```rust
// In evaluator.rs

// Add import for the updated type operations
use crate::apply_type_operation::apply_type_operation;
use crate::resource_type::{is_of_type, as_type, of_type};
use crate::polymorphic_access::{
    is_choice_element, access_polymorphic_element, 
    apply_polymorphic_type_operation
};

// Update the Expression::Type case in the evaluate function
Expression::Type(left, op, type_spec) => {
    let result = evaluate(left, context, current_item)?;
    
    // Handle the 'as' operator operator syntax (Value as Type)
    if op == "as" {
        // Apply the type operation directly
        apply_type_operation(&result, op, type_spec)
    } else {
        // For 'is' operator, handle the result
        match apply_type_operation(&result, op, type_spec) {
            Ok(is_result) => Ok(is_result),
            Err(e) => {
                // Better error handling for is() operation
                if op == "is" {
                    Err(EvaluationError::TypeError(
                        format!("Error applying 'is' operator: {}", e)
                    ))
                } else {
                    Err(e)
                }
            }
        }
    }
}

// Function for special type function handling
fn evaluate_of_type_function(value: &EvaluationResult, type_spec: &TypeSpecifier) -> Result<EvaluationResult, EvaluationError> {
    of_type(value, type_spec)
}
```

## 4. Testing the Implementation

Create unit tests to verify the fixes:

```rust
// In appropriate test file

#[test]
fn test_is_operator_with_primitive_types() {
    // Test basic primitives
    assert_eq!(
        evaluate("true.is(Boolean)", &context, None).unwrap(),
        EvaluationResult::Boolean(true)
    );
    
    assert_eq!(
        evaluate("1.is(Integer)", &context, None).unwrap(),
        EvaluationResult::Boolean(true)
    );
    
    assert_eq!(
        evaluate("1.0.is(Decimal)", &context, None).unwrap(),
        EvaluationResult::Boolean(true)
    );
    
    // Test FHIR primitives
    assert_eq!(
        evaluate("true.is(boolean)", &context, None).unwrap(),
        EvaluationResult::Boolean(true)
    );
    
    // Test namespace qualification
    assert_eq!(
        evaluate("true.is(System.Boolean)", &context, None).unwrap(),
        EvaluationResult::Boolean(true)
    );
    
    assert_eq!(
        evaluate("true.is(FHIR.boolean)", &context, None).unwrap(),
        EvaluationResult::Boolean(true)
    );
}

#[test]
fn test_as_operator() {
    // Test basic as operator
    assert_eq!(
        evaluate("(5 as Integer) + 3", &context, None).unwrap(),
        EvaluationResult::Integer(8)
    );
    
    // Test as with wrong type
    assert_eq!(
        evaluate("(5 as String).empty()", &context, None).unwrap(),
        EvaluationResult::Boolean(true)
    );
}

#[test]
fn test_choice_element_access() {
    // Setup context with Observation
    let context = create_test_context_with_observation();
    
    // Test choice element access
    assert_eq!(
        evaluate("Observation.value.unit", &context, None).unwrap(),
        EvaluationResult::String("lbs")
    );
    
    // Test type checking
    assert_eq!(
        evaluate("Observation.value is Quantity", &context, None).unwrap(),
        EvaluationResult::Boolean(true)
    );
    
    // Test as operator
    assert_eq!(
        evaluate("(Observation.value as Quantity).unit", &context, None).unwrap(),
        EvaluationResult::String("lbs")
    );
    
    // Test as with wrong type
    assert_eq!(
        evaluate("(Observation.value as Period).empty()", &context, None).unwrap(),
        EvaluationResult::Boolean(true)
    );
}
```

## 5. Main Changes Summary

This implementation fixes the following key issues:

1. **Type Checking with `is` Operator**:
   - Properly handles both qualified and unqualified type names
   - Correctly checks resource inheritance relationships
   - Improves error messages for type mismatches

2. **Type Casting with `as` Operator**:
   - Uses consistent logic with the `is` operator
   - Properly handles complex and resource types
   - Fixed syntax with parentheses: `(value as Type).property`

3. **Polymorphic Access**:
   - Better detection of choice elements
   - Improved property access on choice elements
   - Integration with type system for proper type checking

4. **Type Reflection**:
   - Enhanced type() function to return proper namespace and name
   - Better handling of resource types and complex types
   - Support for collections of types

These changes address the majority of the failing tests related to type operations and polymorphic access, which were identified as the highest priority issues in the FHIRPath implementation.