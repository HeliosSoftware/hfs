//! # FHIRPath Polymorphic Element Access
//!
//! Handles accessing polymorphic FHIR elements (e.g., value[x]) in FHIRPath expressions.

use helios_fhir::FhirVersion;
use helios_fhirpath_support::{EvaluationError, EvaluationResult};
use std::collections::HashMap;

/// # Polymorphic Access
///
/// This module implements polymorphic access for FHIR choice elements in FHIRPath.
///
/// In FHIR, choice elements are fields that can contain different types of data,
/// indicated by a suffix in the field name. For example, Observation.value\[x\]
/// might be represented as:
/// - valueQuantity (with type Quantity)
/// - valueString (with type String)
/// - valueCodeableConcept (with type CodeableConcept)
/// - etc.
///
/// FHIRPath allows accessing choice elements using the base name, without the type suffix.
/// For example, `Observation.value` should resolve to the appropriate element (valueQuantity,
/// valueString, etc.) based on which one is present in the resource.
///
/// This module provides the implementation for this polymorphic access pattern, including:
/// - Identifying choice elements in FHIR resources
/// - Accessing choice elements by their base name
/// - Filtering choice elements by type (using is/as operators)
///
/// Handles polymorphic access to FHIR resource choice elements.
///
/// This function resolves a field name in a FHIR resource object, handling choice elements
/// according to FHIRPath rules. For choice elements like value\[x\], it will find the
/// appropriate concrete field (e.g., valueQuantity) based on what's available in the object.
///
/// # Arguments
///
/// * `obj` - A reference to a HashMap representing a FHIR resource or part of a resource
/// * `field_name` - The name of the field to access, which may be a choice element base name
/// * `version` - The FHIR version of the data being evaluated. Choice-element
///   metadata is version-specific (`Observation.valueAttachment` exists in R5/R6
///   but not R4), so this must be the *evaluation context's* version — never
///   `FhirVersion::default_enabled()`. See issue #309.
///
/// # Returns
///
/// * `Some(EvaluationResult)` if the field was found (either directly or via polymorphic access)
/// * `None` if the field wasn't found
///
/// # Examples
///
/// // For a FHIR Observation with valueQuantity:
/// // access_polymorphic_element(observation, "value", version) -> Some(valueQuantity)
/// // access_polymorphic_element(observation, "value.unit", version) -> Some(unit)
pub fn access_polymorphic_element(
    obj: &HashMap<String, EvaluationResult>,
    field_name: &str,
    version: FhirVersion,
) -> Option<EvaluationResult> {
    // First, try direct access - field might already be the right name
    if let Some(value) = obj.get(field_name) {
        return Some(value.clone());
    }

    // Special case for common polymorphic path patterns (like 'value.unit', 'value.code', etc.)
    if field_name.contains('.') {
        let parts: Vec<&str> = field_name.split('.').collect();
        let first_part = parts[0];
        let rest = &parts[1..].join(".");

        // Handle path with potential choice element as the first part
        if is_choice_element(first_part, version) {
            // Try to resolve the choice element
            let matches = get_polymorphic_fields(obj, first_part, version);

            // Process each matching field
            for (_, value) in &matches {
                if let EvaluationResult::Object {
                    map: inner_obj,
                    type_info: _,
                } = value
                {
                    // Recursively resolve the rest of the path
                    if let Some(result) = access_polymorphic_element(inner_obj, rest, version) {
                        return Some(result);
                    }
                }
            }

            // Handle special cases for all potential typed fields
            // This covers patterns like value.unit -> valueQuantity.unit
            for (key, value) in obj.iter() {
                // Check if key starts with the first part and has a type suffix
                if key.starts_with(first_part) && key.len() > first_part.len() {
                    // Extract the type suffix (need uppercase letter after base name)
                    if let Some(c) = key.chars().nth(first_part.len()) {
                        if c.is_uppercase() {
                            // This is a potential choice element with type suffix
                            if let EvaluationResult::Object {
                                map: inner_obj,
                                type_info: _,
                            } = value
                            {
                                // Try to resolve the rest of the path
                                if let Some(result) =
                                    access_polymorphic_element(inner_obj, rest, version)
                                {
                                    return Some(result);
                                }
                            }
                        }
                    }
                }
            }
        } else {
            // Regular path (not a choice element)
            if let Some(value) = obj.get(first_part) {
                if let EvaluationResult::Object {
                    map: inner_obj,
                    type_info: _,
                } = value
                {
                    return access_polymorphic_element(inner_obj, rest, version);
                }
            }
        }

        // No match found for the path
        return None;
    }

    // Check if this could be a choice element
    // Even without metadata, we can try to find polymorphic fields
    // based on the pattern of fields in the object
    let matching_fields = get_polymorphic_fields(obj, field_name, version);

    // If we found any matches, it's likely a choice element
    if !matching_fields.is_empty() {
        // If we found exactly one match, return it
        if matching_fields.len() == 1 {
            return Some(matching_fields[0].1.clone());
        }

        // If we found multiple matches, return the first one
        return Some(matching_fields[0].1.clone());
    }

    // No matching field found
    None
}

/// Gets all possible polymorphic fields for a choice element.
///
/// This function searches an object for fields that match the polymorphic pattern
/// for a given base name. For example, with base_name "value", it will look for
/// fields like "valueQuantity", "valueString", etc.
///
/// # Arguments
///
/// * `obj` - A reference to a HashMap representing a FHIR resource or part of a resource
/// * `base_name` - The base name of the choice element to search for
/// * `version` - The FHIR version of the data being evaluated (see issue #309)
///
/// # Returns
///
/// A vector of tuples containing the field name and value for all matching fields
fn get_polymorphic_fields(
    obj: &HashMap<String, EvaluationResult>,
    base_name: &str,
    version: FhirVersion,
) -> Vec<(String, EvaluationResult)> {
    let mut matches = Vec::new();

    if let Some(value) = obj.get(base_name) {
        matches.push((base_name.to_string(), value.clone()));
    }

    // Preferred path: when `obj` identifies a FHIR resource, consult the
    // generated `FIELD_TYPES` table for that parent and pull out only the
    // typed variants that are both declared in the spec and present in the
    // data. More accurate than the JSON-key prefix scan below, which can
    // match unrelated fields whose names happen to start with `base_name`.
    //
    // The table MUST be the one for `version` — the evaluation context's FHIR
    // version — not `default_enabled()`. Choice-element type sets differ
    // between versions (`Observation.valueAttachment` is R5/R6 only,
    // `Person.deceased[x]` is R5+ only), and consulting the wrong table makes
    // the variant present in the data invisible here. See issue #309.
    let mut consulted_field_types = false;
    if let Some(EvaluationResult::String(resource_type, _, _)) = obj.get("resourceType")
        && let Some(table) = helios_fhir::field_types(version)
    {
        consulted_field_types = true;
        for (parent, field, _ty, _is_collection) in table {
            if parent != resource_type {
                continue;
            }
            let Some(suffix) = field.strip_prefix(base_name) else {
                continue;
            };
            if !suffix
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_uppercase())
            {
                continue;
            }
            if matches.iter().any(|(n, _)| n == field) {
                continue;
            }
            if let Some(value) = obj.get(*field) {
                let converted = convert_fhir_field_to_fhirpath_type(value, suffix);
                matches.push(((*field).to_string(), converted));
            }
        }
    }

    // Fallback for nested objects (no `resourceType`) and for the
    // version-feature-disabled case — preserves prior behavior so callers
    // working below the resource root still resolve typed variants.
    if !consulted_field_types {
        for (field_name, value) in obj {
            if matches.iter().any(|(name, _)| name == field_name) {
                continue;
            }
            if field_name.starts_with(base_name) && field_name.len() > base_name.len() {
                if let Some(c) = field_name.chars().nth(base_name.len()) {
                    if c.is_uppercase() {
                        let type_suffix = &field_name[base_name.len()..];
                        let converted_value =
                            convert_fhir_field_to_fhirpath_type(value, type_suffix);
                        matches.push((field_name.clone(), converted_value));
                    }
                }
            }
        }
    }

    // Observation/`value` policy: prefer `valueQuantity` when present. This
    // is a FHIRPath-evaluator product choice (the FHIR spec doesn't declare
    // it canonical), kept stable through the structural refactor above.
    if base_name == "value"
        && matches.len() > 1
        && obj.get("resourceType") == Some(&EvaluationResult::string("Observation".to_string()))
        && let Some(idx) = matches.iter().position(|(name, _)| name == "valueQuantity")
    {
        let item = matches.remove(idx);
        matches.insert(0, item);
    }

    matches
}

/// Converts a FHIR field value to the appropriate FHIRPath type based on the field suffix.
///
/// This function handles the conversion of FHIR string values to their appropriate
/// FHIRPath types when accessed through polymorphic paths. For example, a `valueDateTime`
/// field that contains a string like "2010-10-10" should be treated as a `DateTime`
/// type in FHIRPath expressions.
///
/// # Arguments
///
/// * `value` - The original FHIR field value
/// * `suffix` - The FHIR type suffix (e.g., "DateTime", "Date", "Time")
///
/// # Returns
///
/// An `EvaluationResult` with the appropriate FHIRPath type
fn convert_fhir_field_to_fhirpath_type(value: &EvaluationResult, suffix: &str) -> EvaluationResult {
    match value {
        EvaluationResult::String(s, _, _) => {
            match suffix {
                "DateTime" => {
                    // Convert string to DateTime if it's a valid date/datetime format
                    EvaluationResult::datetime(s.clone())
                }
                "Date" => {
                    // Convert string to Date if it's a valid date format
                    EvaluationResult::date(s.clone())
                }
                "Time" => {
                    // Convert string to Time if it's a valid time format
                    EvaluationResult::time(s.clone())
                }
                "Instant" => {
                    // Convert string to Instant type (which is a datetime with required timezone)
                    // Use DateTime with instant type info
                    EvaluationResult::DateTime(
                        s.clone(),
                        Some(helios_fhirpath_support::TypeInfoResult::new(
                            "FHIR", "instant",
                        )),
                        None,
                    )
                }
                "Code" => {
                    // Convert string to code type
                    EvaluationResult::fhir_string(s.clone(), "code")
                }
                "Id" => {
                    // Convert string to id type
                    EvaluationResult::fhir_string(s.clone(), "id")
                }
                "Uri" => {
                    // Convert string to uri type
                    EvaluationResult::fhir_string(s.clone(), "uri")
                }
                "Url" => {
                    // Convert string to url type
                    EvaluationResult::fhir_string(s.clone(), "url")
                }
                "Uuid" => {
                    // Convert string to uuid type
                    EvaluationResult::fhir_string(s.clone(), "uuid")
                }
                "Canonical" => {
                    // Convert string to canonical type
                    EvaluationResult::fhir_string(s.clone(), "canonical")
                }
                "Oid" => {
                    // Convert string to oid type
                    EvaluationResult::fhir_string(s.clone(), "oid")
                }
                "Markdown" => {
                    // Convert string to markdown type
                    EvaluationResult::fhir_string(s.clone(), "markdown")
                }
                "Base64Binary" => {
                    // Convert string to base64Binary type
                    EvaluationResult::fhir_string(s.clone(), "base64Binary")
                }
                _ => {
                    // For other types or when the conversion doesn't apply, return as-is
                    value.clone()
                }
            }
        }
        _ => {
            // For non-string values, return as-is
            value.clone()
        }
    }
}

/// Determines if a field name represents a FHIR choice element.
///
/// In FHIR, choice elements are indicated by a \[x\] suffix in the field definition,
/// such as value\[x\]. In actual JSON data, these appear with a type suffix (valueQuantity).
/// This function checks if a given field name (without the type suffix) is likely to be
/// a choice element.
///
/// # Arguments
///
/// * `field_name` - The field name to check
///
/// # Returns
///
/// `true` if the field is likely to be a choice element, `false` otherwise
///
/// # Examples
///
/// ```ignore
/// // This function is used internally by the FHIRPath evaluator.
/// // The answer is version-specific: `class` is a choice base in R4 only,
/// // `address` in R5 only (see issue #309).
/// assert!(is_choice_element("value", FhirVersion::R4));
/// assert!(is_choice_element("effective", FhirVersion::R4));
/// ```
/// Checks if a field name represents a FHIR choice element.
///
/// This function uses context-aware detection to determine if a field
/// is a choice element. When metadata is available (through FhirResourceMetadata),
/// it uses that for accurate detection. Otherwise, it falls back to
/// conservative heuristics.
///
/// # Arguments
/// * `field_name` - The field name to check
/// * `context_metadata` - Optional slice of known choice element names for the context
/// * `version` - The FHIR version of the data being evaluated, used only when
///   `context_metadata` is `None` (see issue #309)
///
/// # Returns
/// `true` if the field is a choice element, `false` otherwise
pub fn is_choice_element_with_context(
    field_name: &str,
    context_metadata: Option<&[&str]>,
    version: FhirVersion,
) -> bool {
    // Pattern 1: Field name contains [x] - definitely a choice element
    if field_name.contains("[x]") {
        return true;
    }

    // If we have metadata, use it for accurate detection
    if let Some(choice_elements) = context_metadata {
        // Check if this field name is in the known choice elements
        if choice_elements.contains(&field_name) {
            return true;
        }

        // Also check if this looks like a typed variant of a known choice element
        // e.g., if "value" is a choice element, then "valueQuantity" is too
        for base_name in choice_elements {
            if field_name.starts_with(base_name) && field_name.len() > base_name.len() {
                // Check if the character after the base name is uppercase
                if let Some(c) = field_name.chars().nth(base_name.len()) {
                    if c.is_uppercase() {
                        return true;
                    }
                }
            }
        }

        return false;
    }

    // Without metadata, consult the generated `FIELD_TYPES` table for the
    // *evaluated data's* FHIR version: `field_name` is a choice base if at
    // least one field in any parent type has the form
    // `<field_name><UppercaseLetter>...`.
    is_polymorphic_base_in_version(field_name, version)
}

/// Convenience wrapper over [`is_choice_element_with_context`] for callers with
/// no choice metadata to offer.
fn is_choice_element(field_name: &str, version: FhirVersion) -> bool {
    is_choice_element_with_context(field_name, None, version)
}

/// Returns true when `name` is the base of a polymorphic FHIR field in
/// `version`'s generated `FIELD_TYPES` table — i.e. some declared field is
/// `<name><UppercaseLetter>...`. Lets the no-context choice-element check
/// return a useful answer for common polymorphic bases (`value`, `effective`,
/// `onset`, …) instead of the always-false fallback that preceded this.
///
/// The answer is version-specific and genuinely differs: `class` is a
/// polymorphic base in R4 (`Encounter.classHistory`) but not R5, and `address`
/// is one in R5 (`VirtualServiceDetail.addressString`) but not R4 — hence
/// issue #309.
///
/// Returns `false` when `version`'s feature isn't compiled in: the honest
/// answer is "unknown", and `false` preserves the prior behaviour for that case.
///
/// NOTE: this scan is *parent-unscoped* — it asks whether any parent type
/// anywhere has such a field — which makes it loose even within a single
/// version. Tightening that is tracked separately; this function only fixes
/// which version's table is consulted.
fn is_polymorphic_base_in_version(name: &str, version: FhirVersion) -> bool {
    let Some(table) = helios_fhir::field_types(version) else {
        return false;
    };
    table.iter().any(|(_, f, _, _)| {
        f.strip_prefix(name)
            .and_then(|rest| rest.chars().next())
            .is_some_and(|c| c.is_ascii_uppercase())
    })
}

/// Applies a type-based operation to a value, handling polymorphic choice elements.
///
/// This function implements the 'is' and 'as' operators for FHIRPath, with special
/// handling for FHIR choice elements. It allows expressions like:
/// - Observation.value.is(Quantity) - Returns true if value is a Quantity
/// - Observation.value.as(Quantity) - Returns the value as a Quantity if possible
///
/// # Arguments
///
/// * `value` - The value to apply the type operation to
/// * `op` - The operation to perform: "is" or "as"
/// * `type_name` - The name of the type to check/convert to
/// * `namespace` - Optional namespace for the type (e.g., "System", "FHIR")
/// * `context` - The caller's evaluation context. Used for its `fhir_version`
///   when delegating to [`crate::resource_type`] and to the choice-element
///   check; previously a throwaway default-version context was fabricated here,
///   which resolved types against the wrong FHIR version (issue #309).
///
/// # Returns
///
/// * For "is" operations, returns a Boolean result indicating if the value matches the type
/// * For "as" operations, returns the value converted to the requested type, or Empty if not possible
///
/// # Examples
///
/// ```ignore
/// // This function is used internally by the FHIRPath evaluator
/// // to handle polymorphic type operations on FHIR choice elements
/// let result1 = apply_polymorphic_type_operation(&value, "is", "Quantity", None, context);
/// let result2 = apply_polymorphic_type_operation(&value, "as", "Quantity", None, context);
/// ```
pub fn apply_polymorphic_type_operation(
    value: &EvaluationResult,
    op: &str,
    type_name: &str,
    namespace: Option<&str>,
    context: &crate::EvaluationContext,
) -> Result<EvaluationResult, EvaluationError> {
    // Handle empty values first
    if let EvaluationResult::Empty = value {
        // For Empty values, we can't perform type operations but we can do some operation-specific handling
        if op == "is" && type_name == "Empty" {
            // Empty.is(Empty) is true
            return Ok(EvaluationResult::boolean(true));
        } else if op == "is" {
            // Empty is not any other type
            return Ok(EvaluationResult::boolean(false));
        } else if op == "as" {
            // Casting Empty to any type remains Empty
            return Ok(EvaluationResult::Empty);
        }
        return Ok(EvaluationResult::Empty);
    }

    if let EvaluationResult::Collection { items, .. } = value {
        if items.len() != 1 {
            return Ok(EvaluationResult::Empty);
        }
        return apply_polymorphic_type_operation(&items[0], op, type_name, namespace, context);
    }

    // Since we need to determine if the original path is a choice element
    if op == "is" || op == "as" {
        // The value being checked could be:
        // 1. Direct access already succeeded (like Observation.valueQuantity)
        // 2. Polymorphic access that needs to be checked (like Observation.value which should match valueQuantity)

        // First handle direct FHIR resource type checks
        if let EvaluationResult::Object {
            map: obj,
            type_info: _,
        } = value
        {
            // For polymorphic value checks (like value.is(Quantity))
            // We need to handle both:
            // - Direct check on a quantity-like object
            // - Check on a polymorphic property that could be a choice element

            // Special case for Quantity type when called on a value object
            if type_name == "Quantity" || type_name == "quantity" {
                // Check if this is already a Quantity by structure
                if obj.contains_key("value")
                    && (obj.contains_key("unit") || obj.contains_key("code"))
                {
                    return if op == "is" {
                        // This looks like a Quantity, so return true
                        Ok(EvaluationResult::boolean(true))
                    } else {
                        // op == "as"
                        // Return the object itself since it already has the expected Quantity structure
                        Ok(value.clone())
                    };
                }

                // Check if this object has a valueQuantity field (for parent objects)
                if obj.contains_key("valueQuantity") {
                    return if op == "is" {
                        Ok(EvaluationResult::boolean(true))
                    } else {
                        // op == "as"
                        // Return the valueQuantity field
                        if let Some(quantity) = obj.get("valueQuantity") {
                            Ok(quantity.clone())
                        } else {
                            Ok(EvaluationResult::Empty)
                        }
                    };
                }

                // Check if this resource is an Observation with a valueQuantity field
                if let Some(EvaluationResult::String(resource_type, _, _)) = obj.get("resourceType")
                {
                    if resource_type == "Observation" && obj.contains_key("valueQuantity") {
                        return if op == "is" {
                            Ok(EvaluationResult::boolean(true))
                        } else {
                            // op == "as"
                            // Return the valueQuantity field
                            if let Some(quantity) = obj.get("valueQuantity") {
                                Ok(quantity.clone())
                            } else {
                                Ok(EvaluationResult::Empty)
                            }
                        };
                    }
                }
            }

            // Check resource type - handle FHIR resource type checking generically
            if let Some(EvaluationResult::String(resource_type, _, _)) = obj.get("resourceType") {
                // For direct resource type checks (like Patient.is(Patient)), use case-insensitive comparison
                if resource_type.to_lowercase() == type_name.to_lowercase() {
                    return if op == "is" {
                        Ok(EvaluationResult::boolean(true))
                    } else {
                        // op == "as"
                        Ok(value.clone())
                    };
                }

                // Handle parent types like DomainResource and Resource
                if type_name.to_lowercase() == "domainresource"
                    && crate::resource_type::is_fhir_domain_resource(resource_type)
                {
                    return if op == "is" {
                        Ok(EvaluationResult::boolean(true))
                    } else {
                        // op == "as"
                        Ok(value.clone())
                    };
                }

                // All FHIR resources are Resource types
                if type_name.to_lowercase() == "resource" {
                    return if op == "is" {
                        Ok(EvaluationResult::boolean(true))
                    } else {
                        // op == "as"
                        Ok(value.clone())
                    };
                }
            }
        }

        // For proper type checking, delegate to resource_type module which has type hierarchy support
        match op {
            "is" => {
                // First try using the resource_type module for proper type checking with hierarchy
                // support. The caller's context is passed through so the type hierarchy is read
                // for the evaluated data's FHIR version rather than a fabricated default one
                // (issue #309); it also avoids allocating a throwaway context per `is`/`as`.
                let type_spec = match namespace {
                    Some(ns) => crate::parser::TypeSpecifier::QualifiedIdentifier(
                        ns.to_string(),
                        Some(type_name.to_string()),
                    ),
                    None => crate::parser::TypeSpecifier::QualifiedIdentifier(
                        type_name.to_string(),
                        None,
                    ),
                };
                // A resolution failure deliberately falls through to the heuristics below
                // rather than propagating — `test_as_type_operation` depends on it.
                if let Ok(result) =
                    crate::resource_type::is_of_type_with_context(value, &type_spec, context)
                {
                    return Ok(EvaluationResult::boolean(result));
                }

                // Fall back to original implementation if resource_type didn't handle it
                match value {
                    EvaluationResult::Object {
                        map: obj,
                        type_info: _,
                    } => {
                        // First check for FHIR resource type matching (for objects with type_info)
                        if let Some(EvaluationResult::String(resource_type, _, _)) =
                            obj.get("resourceType")
                        {
                            // For direct resource type checks (like Patient.is(Patient) or Patient.is(FHIR.Patient))
                            if resource_type.to_lowercase() == type_name.to_lowercase() {
                                return Ok(EvaluationResult::boolean(true));
                            }

                            // Handle parent types like DomainResource and Resource
                            if type_name.to_lowercase() == "domainresource"
                                && crate::resource_type::is_fhir_domain_resource(resource_type)
                            {
                                return Ok(EvaluationResult::boolean(true));
                            }

                            // All FHIR resources are Resource types
                            if type_name.to_lowercase() == "resource" {
                                return Ok(EvaluationResult::boolean(true));
                            }
                        }

                        // Continue with other type checking logic...
                        // Check for boolean-like properties in FHIR resources without hardcoding specific fields
                        if type_name.to_lowercase() == "boolean" {
                            // Check for properties with names often used for boolean flags in FHIR
                            for key in obj.keys() {
                                // Skip resourceType
                                if key == "resourceType" {
                                    continue;
                                }

                                // Properties that typically contain booleans have names relating to state/flags
                                if key.to_lowercase().contains("active")
                                    || key.to_lowercase().contains("flag")
                                    || key.to_lowercase().contains("enabled")
                                    || key.to_lowercase().contains("status")
                                    || key.to_lowercase().contains("is")
                                {
                                    return Ok(EvaluationResult::boolean(true));
                                }
                            }

                            // If this object contains a boolean field (other than resourceType), it's likely a boolean property
                            for (key, value) in obj.iter() {
                                if key != "resourceType"
                                    && matches!(value, EvaluationResult::Boolean(_, _, _))
                                {
                                    return Ok(EvaluationResult::boolean(true));
                                }
                            }

                            // If this is a small object that represents a single property
                            // (like a FHIR boolean property), check if it has the right structure
                            if obj.len() < 5 && !obj.contains_key("resourceType") {
                                // Look for clues that this is a boolean property
                                // Often FHIR properties are wrapped in objects with few fields
                                if obj.contains_key("id") || obj.contains_key("extension") {
                                    return Ok(EvaluationResult::boolean(true));
                                }

                                // Special case for the 'active' property itself
                                if obj.keys().len() <= 2 {
                                    // If it's a very small object, it's likely a primitive boolean property
                                    return Ok(EvaluationResult::boolean(true));
                                }
                            }
                        }

                        // Check for date-like properties in any FHIR resource without hardcoding specific fields
                        if type_name.to_lowercase() == "date" || type_name == "Date" {
                            // Look for any property that could be a date
                            for (key, val) in obj.iter() {
                                // Skip resourceType
                                if key == "resourceType" {
                                    continue;
                                }

                                // Check value type - date values could be stored as strings or as Date type
                                match val {
                                    EvaluationResult::Date(_, None, None) => {
                                        return Ok(EvaluationResult::boolean(true));
                                    }
                                    EvaluationResult::String(s, _, _) => {
                                        // Check if string looks like a date (YYYY-MM-DD)
                                        if s.len() >= 10
                                            && s.chars().nth(4) == Some('-')
                                            && s.chars().nth(7) == Some('-')
                                        {
                                            return Ok(EvaluationResult::boolean(true));
                                        }
                                    }
                                    _ => {}
                                }

                                // Date-related property names often contain "date" or "time"
                                if key.to_lowercase().contains("date")
                                    || key.to_lowercase().contains("time")
                                    || key.to_lowercase().contains("birth")
                                {
                                    return Ok(EvaluationResult::boolean(true));
                                }
                            }
                        }

                        // First try direct polymorphic field matching
                        for key in obj.keys() {
                            if key.ends_with(type_name) && key.len() > type_name.len() {
                                let base_name = &key[0..(key.len() - type_name.len())];
                                if is_choice_element(base_name, context.fhir_version) {
                                    return Ok(EvaluationResult::boolean(true));
                                }
                            }
                        }

                        // Check for specific cases like "value" -> valueQuantity for Observation.value.is(Quantity)
                        if obj.contains_key("value") && type_name == "Quantity" {
                            // Check if the value field looks like a Quantity
                            if let Some(EvaluationResult::Object { map: value_obj, .. }) =
                                obj.get("value")
                            {
                                if value_obj.contains_key("value") && value_obj.contains_key("unit")
                                {
                                    return Ok(EvaluationResult::boolean(true));
                                }
                            }

                            // Also check for valueQuantity
                            if obj.contains_key("valueQuantity") {
                                return Ok(EvaluationResult::boolean(true));
                            }
                        }

                        // Try matching the value's type directly
                        // For native types mapped to FHIR primitive types
                        if let Some(EvaluationResult::String(value_type, _, _)) = obj.get("type") {
                            if value_type == type_name {
                                return Ok(EvaluationResult::boolean(true));
                            }
                        }

                        // No match found
                        Ok(EvaluationResult::boolean(false))
                    }
                    // Match native types to FHIRPath types
                    EvaluationResult::Boolean(_, _, _) => {
                        // Check for qualifiers like "System.Boolean" and "FHIR.boolean"
                        let is_boolean_type = type_name == "Boolean"
                            || type_name == "boolean"
                            || type_name.ends_with(".Boolean")
                            || type_name.ends_with(".boolean");
                        Ok(EvaluationResult::boolean(is_boolean_type))
                    }
                    EvaluationResult::Integer(_, _, _) => {
                        // Check for qualifiers like "System.Integer" and "FHIR.integer"
                        let is_integer_type = type_name == "Integer"
                            || type_name == "integer"
                            || type_name.ends_with(".Integer")
                            || type_name.ends_with(".integer");
                        Ok(EvaluationResult::boolean(is_integer_type))
                    }
                    EvaluationResult::Decimal(_, _, _) => {
                        // Check for qualifiers like "System.Decimal" and "FHIR.decimal"
                        let is_decimal_type = type_name == "Decimal"
                            || type_name == "decimal"
                            || type_name.ends_with(".Decimal")
                            || type_name.ends_with(".decimal");
                        Ok(EvaluationResult::boolean(is_decimal_type))
                    }
                    EvaluationResult::String(_, _, _) => {
                        // Check for qualifiers like "System.String" and "FHIR.string"
                        let is_string_type = type_name == "String"
                            || type_name == "string"
                            || type_name.ends_with(".String")
                            || type_name.ends_with(".string");
                        Ok(EvaluationResult::boolean(is_string_type))
                    }
                    EvaluationResult::Date(_, _, _) => {
                        // Check for qualifiers like "System.Date" and "FHIR.date"
                        let is_date_type = type_name == "Date"
                            || type_name == "date"
                            || type_name.ends_with(".Date")
                            || type_name.ends_with(".date");
                        Ok(EvaluationResult::boolean(is_date_type))
                    }
                    EvaluationResult::DateTime(_, _, _) => {
                        // Check for qualifiers like "System.DateTime" and "FHIR.dateTime"
                        let is_datetime_type = type_name == "DateTime"
                            || type_name == "dateTime"
                            || type_name.ends_with(".DateTime")
                            || type_name.ends_with(".dateTime");
                        Ok(EvaluationResult::boolean(is_datetime_type))
                    }
                    EvaluationResult::Time(_, _, _) => {
                        // Check for qualifiers like "System.Time" and "FHIR.time"
                        let is_time_type = type_name == "Time"
                            || type_name == "time"
                            || type_name.ends_with(".Time")
                            || type_name.ends_with(".time");
                        Ok(EvaluationResult::boolean(is_time_type))
                    }
                    EvaluationResult::Quantity(_, _, _, _) => {
                        // Check for qualifiers like "System.Quantity" and "FHIR.Quantity"
                        let is_quantity_type =
                            type_name == "Quantity" || type_name.ends_with(".Quantity");
                        Ok(EvaluationResult::boolean(is_quantity_type))
                    }
                    // These cases should never happen due to earlier checks
                    EvaluationResult::Empty => Ok(EvaluationResult::boolean(false)),
                    EvaluationResult::Collection { .. } => Ok(EvaluationResult::boolean(false)),
                    #[cfg(not(any(feature = "R4", feature = "R4B")))]
                    EvaluationResult::Integer64(_, _, _) => {
                        // Check for qualifiers like "System.Integer64" and "FHIR.integer64"
                        let is_integer64_type = type_name == "Integer64"
                            || type_name == "integer64"
                            || type_name.ends_with(".Integer64")
                            || type_name.ends_with(".integer64");
                        Ok(EvaluationResult::boolean(is_integer64_type))
                    }
                    #[cfg(any(feature = "R4", feature = "R4B"))]
                    EvaluationResult::Integer64(_, _, _) => {
                        // In R4 and R4B, Integer64 should be treated as Integer
                        let is_integer_type = type_name == "Integer"
                            || type_name == "integer"
                            || type_name.ends_with(".Integer")
                            || type_name.ends_with(".integer");
                        Ok(EvaluationResult::boolean(is_integer_type))
                    }
                }
            }
            "as" => {
                // The 'as' operator returns the input value if it 'is' of the specified type,
                // otherwise it returns Empty.
                let is_type_result =
                    apply_polymorphic_type_operation(value, "is", type_name, namespace, context)?;
                match is_type_result {
                    EvaluationResult::Boolean(true, _, _) => Ok(value.clone()),
                    EvaluationResult::Boolean(false, _, _) => Ok(EvaluationResult::Empty),
                    EvaluationResult::Empty => Ok(EvaluationResult::Empty), // 'is' on Empty can be Empty
                    _ => Err(EvaluationError::TypeError(format!(
                        "'is' operation returned non-Boolean: {:?}",
                        is_type_result
                    ))),
                }
            }
            _ => Err(EvaluationError::TypeError(format!(
                "Unsupported polymorphic type operation: {}",
                op
            ))),
        }
    } else {
        // Unsupported operation
        Err(EvaluationError::TypeError(format!(
            "Unsupported polymorphic type operation: {}",
            op
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The version these version-agnostic tests run under. They use
    /// `valueQuantity`, which is declared in every FHIR version, so the choice
    /// of table does not affect them.
    fn test_version() -> FhirVersion {
        FhirVersion::default_enabled()
    }

    /// A minimal context for the `is`/`as` tests, at the default version.
    fn ctx() -> crate::EvaluationContext {
        crate::EvaluationContext::new_empty(test_version())
    }

    // Helper function to create a FHIR Observation with a valueQuantity
    fn create_observation_with_quantity() -> HashMap<String, EvaluationResult> {
        let mut obs = HashMap::new();

        // Add resourceType
        obs.insert(
            "resourceType".to_string(),
            EvaluationResult::string("Observation".to_string()),
        );

        // Add id
        obs.insert(
            "id".to_string(),
            EvaluationResult::string("123".to_string()),
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

        obs
    }

    #[test]
    fn test_access_polymorphic_element() {
        let obs = create_observation_with_quantity();

        // Test accessing a polymorphic element
        let value = access_polymorphic_element(&obs, "value", test_version()).unwrap();

        // Verify that it correctly finds valueQuantity
        if let EvaluationResult::Object {
            map: quantity,
            type_info: _,
        } = &value
        {
            assert_eq!(
                quantity.get("unit").unwrap(),
                &EvaluationResult::string("lbs".to_string())
            );
        } else {
            panic!("Expected Object result, got {:?}", value);
        }
    }

    #[test]
    fn test_is_type_operation() {
        let obs = create_observation_with_quantity();
        let value_quantity = obs.get("valueQuantity").unwrap().clone();

        // Test is(Quantity) on valueQuantity object directly
        // Since we enhanced our polymorphic_access.rs for choice elements,
        // we'll now recognize a valueQuantity object as a Quantity type
        let result =
            apply_polymorphic_type_operation(&value_quantity, "is", "Quantity", None, &ctx())
                .unwrap();
        assert_eq!(result, EvaluationResult::boolean(true)); // Now tests for true

        // Test is(String) on valueQuantity object directly
        let result =
            apply_polymorphic_type_operation(&value_quantity, "is", "String", None, &ctx())
                .unwrap();
        assert_eq!(result, EvaluationResult::boolean(false));

        // Test is() on the Observation object itself
        let obj = EvaluationResult::Object {
            map: obs,
            type_info: None,
        };
        let result =
            apply_polymorphic_type_operation(&obj, "is", "Observation", None, &ctx()).unwrap();
        assert_eq!(result, EvaluationResult::boolean(true));
    }

    #[test]
    fn test_as_type_operation() {
        let obs = create_observation_with_quantity();

        // First, let's test as(Quantity) on the valueQuantity object directly
        let value_quantity = obs.get("valueQuantity").unwrap().clone();
        let result =
            apply_polymorphic_type_operation(&value_quantity, "is", "Quantity", None, &ctx())
                .unwrap();
        // The valueQuantity looks like a Quantity type now, so is(Quantity) should be true
        assert_eq!(result, EvaluationResult::boolean(true)); // Updated to true

        // Now since is(Quantity) is true, as(Quantity) should return the original value
        let result =
            apply_polymorphic_type_operation(&value_quantity, "as", "Quantity", None, &ctx())
                .unwrap();
        assert_eq!(result, value_quantity);

        // Test with an Observation object
        let obj = EvaluationResult::Object {
            map: obs.clone(),
            type_info: None,
        };

        // Testing valueQuantity field indirectly via Quantity
        // In our updated implementation, Observation.is(Quantity) should return true if it contains a valueQuantity
        let result =
            apply_polymorphic_type_operation(&obj, "is", "Quantity", None, &ctx()).unwrap();
        assert_eq!(result, EvaluationResult::boolean(true)); // Should return true because it contains valueQuantity

        // Test for a wrong type
        let result =
            apply_polymorphic_type_operation(&obj, "is", "NonExistentType", None, &ctx()).unwrap();
        assert_eq!(result, EvaluationResult::boolean(false));
    }

    // ------------------------------------------------------------------
    // Issue #309 — version-specific choice-element resolution.
    //
    // These need two version features compiled in to say anything: the bug is
    // "consulted the default version's table instead of the data's", which is
    // only observable when those differ. Each assertion is two-sided (R5 says
    // yes, R4 says no) so it cannot be satisfied by hardcoding a different
    // default.
    // ------------------------------------------------------------------

    /// An R5-only choice variant must be found when asking for R5, and must not
    /// be invented when asking for R4.
    ///
    /// `Observation.valueAttachment` is declared in R5/R6 but not R4.
    #[cfg(all(feature = "R4", feature = "R5"))]
    #[test]
    fn get_polymorphic_fields_honours_the_requested_version() {
        let mut attachment = HashMap::new();
        attachment.insert(
            "title".to_string(),
            EvaluationResult::string("scan.pdf".to_string()),
        );

        let mut obs = HashMap::new();
        obs.insert(
            "resourceType".to_string(),
            EvaluationResult::string("Observation".to_string()),
        );
        obs.insert(
            "valueAttachment".to_string(),
            EvaluationResult::Object {
                map: attachment,
                type_info: None,
            },
        );

        let r5 = get_polymorphic_fields(&obs, "value", FhirVersion::R5);
        assert_eq!(
            r5.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
            vec!["valueAttachment"],
            "R5 declares Observation.valueAttachment, so it must resolve"
        );

        let r4 = get_polymorphic_fields(&obs, "value", FhirVersion::R4);
        assert!(
            r4.is_empty(),
            "R4 does not declare Observation.valueAttachment; the R4 table must \
             not report it. Getting a hit here would mean the version argument \
             is ignored."
        );
    }

    /// The same, through the entry point the evaluator actually calls.
    #[cfg(all(feature = "R4", feature = "R5"))]
    #[test]
    fn access_polymorphic_element_honours_the_requested_version() {
        let mut attachment = HashMap::new();
        attachment.insert(
            "title".to_string(),
            EvaluationResult::string("scan.pdf".to_string()),
        );

        let mut obs = HashMap::new();
        obs.insert(
            "resourceType".to_string(),
            EvaluationResult::string("Observation".to_string()),
        );
        obs.insert(
            "valueAttachment".to_string(),
            EvaluationResult::Object {
                map: attachment,
                type_info: None,
            },
        );

        assert!(
            access_polymorphic_element(&obs, "value", FhirVersion::R5).is_some(),
            "R5 Observation.value must resolve to the Attachment (issue #309)"
        );
        assert!(
            access_polymorphic_element(&obs, "value", FhirVersion::R4).is_none(),
            "R4 has no Observation.valueAttachment to resolve to"
        );
    }

    /// Isolates "this choice base is new in R5" from "this parent type is new
    /// in R5": `Person` is declared in both tables (18 field rows in R4), but
    /// only R5 gives it `deceased[x]`.
    #[cfg(all(feature = "R4", feature = "R5"))]
    #[test]
    fn choice_base_added_in_a_later_version_on_a_shared_parent() {
        let mut person = HashMap::new();
        person.insert(
            "resourceType".to_string(),
            EvaluationResult::string("Person".to_string()),
        );
        person.insert(
            "deceasedBoolean".to_string(),
            EvaluationResult::boolean(true),
        );

        assert_eq!(
            access_polymorphic_element(&person, "deceased", FhirVersion::R5),
            Some(EvaluationResult::boolean(true)),
            "Person.deceased[x] is R5+; it must resolve under R5 (issue #309)"
        );
        assert!(
            access_polymorphic_element(&person, "deceased", FhirVersion::R4).is_none(),
            "R4 Person has no deceased[x]"
        );
    }

    /// The choice-base *classification* is version-specific too. Verified
    /// against the generated tables: `class` is a base in R4 only
    /// (`Encounter.classHistory`), `address` in R5 only
    /// (`VirtualServiceDetail.addressString`), and `value` in both.
    #[cfg(all(feature = "R4", feature = "R5"))]
    #[test]
    fn is_polymorphic_base_is_version_specific() {
        assert!(is_polymorphic_base_in_version("class", FhirVersion::R4));
        assert!(!is_polymorphic_base_in_version("class", FhirVersion::R5));

        assert!(!is_polymorphic_base_in_version("address", FhirVersion::R4));
        assert!(is_polymorphic_base_in_version("address", FhirVersion::R5));

        // Control: a base present in every version must stay stable, so the
        // check does not become spuriously version-sensitive.
        assert!(is_polymorphic_base_in_version("value", FhirVersion::R4));
        assert!(is_polymorphic_base_in_version("value", FhirVersion::R5));
    }
}
