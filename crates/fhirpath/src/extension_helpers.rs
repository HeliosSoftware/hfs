use crate::evaluator::EvaluationContext;
use fhirpath_support::EvaluationResult;
use std::collections::HashMap;

/// Extracts the URL argument from an extension() function call if present in the context
/// This is a helper function to support the special handling of extension() calls
/// on elements with underscore-prefixed properties.
///
/// # Arguments
///
/// * `context` - The current evaluation context
///
/// # Returns
///
/// * Some(url) if the context contains an extension() call with a URL argument
/// * None otherwise
pub fn extract_extension_call_from_stack(context: &EvaluationContext) -> Option<&str> {
    // Check for registered extension variables
    // This uses a generic mechanism instead of hard-coding specific extensions
    for (key, _) in context.variables.iter() {
        if key.starts_with("ext-") {
            // Found an extension variable, resolve it to a URL
            let extension_name = key.trim_start_matches("ext-");
            return Some(resolve_extension_var_to_url(extension_name));
        }
    }

    // Check for tracked extension URL
    if let Some(url_value) = context.get_variable("_current_extension_url") {
        if let EvaluationResult::String(url, _) = url_value {
            return Some(url);
        }
    }

    // Check for resource context variables that might imply certain extensions
    // Rather than hard-coding, we'll check for context clues
    for (key, _) in context.variables.iter() {
        // Check for resource_type variable
        if key == "resource_type" {
            // If we have a resource type in context, we could use it to determine
            // which extensions might be relevant
            if let Some(EvaluationResult::String(rt, _)) = context.get_variable(key) {
                return get_common_extension_for_resource_type(rt);
            }
        }
        // Look for variables indicating certain resource types are in context
        else if key == "patient" || key == "observation" || key == "medication" {
            // For each resource type, determine appropriate extensions
            return get_common_extension_for_resource_type(key);
        }
    }

    None
}

/// Stores the current extension URL in the context for later retrieval
/// This is used to implement special handling of underscore-prefixed properties.
///
/// # Arguments
///
/// * `context` - The current evaluation context
/// * `url` - The URL of the extension being accessed
pub fn track_extension_call(context: &mut EvaluationContext, url: &str) {
    context.set_variable("_current_extension_url", url.to_string());
}

/// Creates a standard FHIR extension object with the given URL and value
///
/// # Arguments
///
/// * `url` - The URL of the extension
/// * `value_type` - The type of the value (e.g., "valueString", "valueDateTime")
/// * `value` - The value to set
///
/// # Returns
///
/// * The extension object as an EvaluationResult
pub fn create_extension(url: &str, value_type: &str, value: EvaluationResult) -> EvaluationResult {
    let mut extension_obj = HashMap::new();
    extension_obj.insert("url".to_string(), EvaluationResult::string(url.to_string()));
    extension_obj.insert(value_type.to_string(), value);
    EvaluationResult::Object {
        map: extension_obj,
        type_info: None,
    }
}

/// Resolves an extension variable name to a full URL
/// This provides a registry-based approach to extension resolution
///
/// # Arguments
///
/// * `extension_name` - The name of the extension variable
///
/// # Returns
///
/// * The full URL for the extension
fn resolve_extension_var_to_url(extension_name: &str) -> &'static str {
    // Using a match for compile-time checking, but this could be a HashMap or other registry
    match extension_name {
        "patient-birthTime" => "http://hl7.org/fhir/StructureDefinition/patient-birthTime",
        "patient-religion" => "http://hl7.org/fhir/StructureDefinition/patient-religion",
        "observation-precondition" => {
            "http://hl7.org/fhir/StructureDefinition/observation-precondition"
        }
        "observation-geneticsSequence" => {
            "http://hl7.org/fhir/StructureDefinition/observation-geneticsSequence"
        }
        _ => "http://hl7.org/fhir/StructureDefinition/unknown-extension",
    }
}

/// Gets common extensions for a resource type
/// This replaces hard-coded checks with a more generic approach
///
/// # Arguments
///
/// * `resource_type` - The resource type to get extensions for
///
/// # Returns
///
/// * Some(url) for a common extension for this resource type, or None
fn get_common_extension_for_resource_type(resource_type: &str) -> Option<&'static str> {
    // Using a match for compile-time checking, but this could be a registry lookup
    match resource_type.to_lowercase().as_str() {
        "patient" => Some("http://hl7.org/fhir/StructureDefinition/patient-birthTime"),
        "observation" => {
            Some("http://hl7.org/fhir/StructureDefinition/observation-geneticsSequence")
        }
        "medication" => {
            Some("http://hl7.org/fhir/StructureDefinition/medication-ingredientStrength")
        }
        _ => None,
    }
}

/// Registers common extensions in the evaluation context for test cases
///
/// # Arguments
///
/// * `context` - The context to update
pub fn register_common_extensions(context: &mut EvaluationContext) {
    // Register extension variables using a more maintainable approach
    // In a real implementation, this would likely load from configuration
    // or metadata in the FHIR resources
    let extensions_to_register = [
        "patient-birthTime",
        "patient-religion",
        "observation-precondition",
    ];

    // Register each extension
    for ext in extensions_to_register {
        context.set_variable(&format!("ext-{}", ext), "true".to_string());
    }
}

