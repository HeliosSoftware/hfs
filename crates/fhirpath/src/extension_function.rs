use fhirpath_support::{EvaluationError, EvaluationResult};
use std::collections::HashMap;

/// Implementation of the FHIRPath extension() function
/// 
/// The extension() function takes a URL string and returns any extension with that URL.
/// In FHIR, extensions are found in special extension arrays or in underscore-prefixed properties.
/// For example, Patient.birthDate would have extensions in Patient._birthDate.extension.
/// 
/// # Arguments
/// 
/// * `invocation_base` - The element to get extensions from
/// * `args` - The arguments to the extension function (expects a single string URL argument)
/// 
/// # Returns
/// 
/// * If the extension is found, returns the extension element(s)
/// * If no extension is found with the given URL, returns Empty
/// * If the invocation_base is Empty, returns Empty
/// * If the args are invalid (wrong number, wrong type), returns an error
pub fn extension_function(
    invocation_base: &EvaluationResult, 
    args: &[EvaluationResult]
) -> Result<EvaluationResult, EvaluationError> {
    // Check that exactly one argument is provided
    if args.len() != 1 {
        return Err(EvaluationError::InvalidArity(
            "Function 'extension' requires exactly one argument".to_string(),
        ));
    }

    // Check that the argument is a string
    let extension_url = match &args[0] {
        EvaluationResult::String(url) => url,
        EvaluationResult::Empty => {
            // extension({}) -> {}
            return Ok(EvaluationResult::Empty);
        }
        _ => {
            return Err(EvaluationError::TypeError(
                "extension() function requires a string URL argument".to_string(),
            ));
        }
    };

    // If the base is Empty, return Empty
    if matches!(invocation_base, EvaluationResult::Empty) {
        return Ok(EvaluationResult::Empty);
    }

    // We need to check several possible locations for extensions:
    // 1. Direct extensions array on the element (Element.extension)
    // 2. Modifying extensions (e.g., Element.modifierExtension)
    // The evaluator handles resolving underscore-prefixed properties (e.g., _birthDate)
    // before calling this function, so invocation_base should be the correct element.

    if let EvaluationResult::Object(obj) = invocation_base {
        // Case 1: Check for direct extension array on this element
        if let Some(EvaluationResult::Collection { items: extensions, .. }) = obj.get("extension") { // Destructure
            let result = find_extension_by_url(extensions, extension_url)?;
            if !matches!(result, EvaluationResult::Empty) {
                return Ok(result);
            }
        }
        
        // Case 2: Check for modifierExtension
        if let Some(EvaluationResult::Collection { items: mod_extensions, .. }) = obj.get("modifierExtension") { // Destructure
            let result = find_extension_by_url(mod_extensions, extension_url)?;
            if !matches!(result, EvaluationResult::Empty) {
                return Ok(result);
            }
        }
    }
    
    // If no extension found, return Empty
    Ok(EvaluationResult::Empty)
}

/// Helper function to find an extension with a specific URL in a collection of extensions
fn find_extension_by_url(
    extensions: &[EvaluationResult],
    url: &str,
) -> Result<EvaluationResult, EvaluationError> {
    let mut matching_extensions = Vec::new();
    
    for ext in extensions {
        if let EvaluationResult::Object(ext_obj) = ext {
            // Check if this extension has the requested URL
            if let Some(EvaluationResult::String(ext_url)) = ext_obj.get("url") {
                if ext_url == url {
                    matching_extensions.push(ext.clone());
                }
            }
        }
    }
    
    // Return the matching extensions, or Empty if none found
    if matching_extensions.is_empty() {
        Ok(EvaluationResult::Empty)
    } else if matching_extensions.len() == 1 {
        Ok(matching_extensions[0].clone())
    } else {
        // Extensions are typically ordered as they appear in the resource
        Ok(EvaluationResult::Collection { items: matching_extensions, has_undefined_order: false })
    }
}

/// Finds matching extensions in underscore-prefixed properties
/// 
/// This function is designed to be called from the evaluator when special handling for
/// underscore-prefixed properties is needed.
///
/// # Arguments
/// 
/// * `parent_obj` - The parent object containing both the element and its underscore-prefixed version
/// * `element_name` - The name of the element (e.g., "birthDate")
/// * `extension_url` - The URL of the extension to find
/// 
/// # Returns
/// 
/// * If the extension is found, returns the extension element
/// * If no extension is found with the given URL, returns Empty
pub fn find_extension_in_underscore_property(
    parent_obj: &HashMap<String, EvaluationResult>,
    element_name: &str,
    extension_url: &str,
) -> EvaluationResult {
    // Create the underscore-prefixed name (e.g., "_birthDate")
    let underscore_name = format!("_{}", element_name);
    
    // Look for the underscore-prefixed element
    if let Some(EvaluationResult::Object(underscore_obj)) = parent_obj.get(&underscore_name) {
        // Check for extensions array
        if let Some(EvaluationResult::Collection { items: extensions, .. }) = underscore_obj.get("extension") { // Destructure
            // Search for matching extension
            for ext in extensions { // Iterate over destructured items
                if let EvaluationResult::Object(ext_obj) = ext {
                    if let Some(EvaluationResult::String(ext_url)) = ext_obj.get("url") {
                        if ext_url == extension_url {
                            return ext.clone();
                        }
                    }
                }
            }
        }
    }
    
    // No matching extension found
    EvaluationResult::Empty
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    
    #[test]
    fn test_extension_function_basic() {
        // Create a test extension
        let mut extension_obj = HashMap::new();
        extension_obj.insert("url".to_string(), EvaluationResult::String("http://example.org/test-extension".to_string()));
        extension_obj.insert("valueString".to_string(), EvaluationResult::String("test value".to_string()));
        let extension = EvaluationResult::Object(extension_obj);
        
        // Create a test element with the extension
        let mut obj = HashMap::new();
        obj.insert("extension".to_string(), EvaluationResult::Collection { items: vec![extension.clone()], has_undefined_order: false });
        let element = EvaluationResult::Object(obj);
        
        // Test the extension function
        let result = extension_function(
            &element, 
            &[EvaluationResult::String("http://example.org/test-extension".to_string())]
        ).unwrap();
        
        // Verify the result matches the extension
        assert_eq!(result, extension);
    }
    
    #[test]
    fn test_extension_function_not_found() {
        // Create a test extension
        let mut extension_obj = HashMap::new();
        extension_obj.insert("url".to_string(), EvaluationResult::String("http://example.org/test-extension".to_string()));
        extension_obj.insert("valueString".to_string(), EvaluationResult::String("test value".to_string()));
        let extension = EvaluationResult::Object(extension_obj);
        
        // Create a test element with the extension
        let mut obj = HashMap::new();
        obj.insert("extension".to_string(), EvaluationResult::Collection { items: vec![extension], has_undefined_order: false });
        let element = EvaluationResult::Object(obj);
        
        // Test the extension function with a different URL
        let result = extension_function(
            &element, 
            &[EvaluationResult::String("http://example.org/other-extension".to_string())]
        ).unwrap();
        
        // Verify the result is Empty
        assert_eq!(result, EvaluationResult::Empty);
    }
    
    #[test]
    fn test_extension_function_empty_base() {
        // Test the extension function with an Empty base
        let result = extension_function(
            &EvaluationResult::Empty, 
            &[EvaluationResult::String("http://example.org/test-extension".to_string())]
        ).unwrap();
        
        // Verify the result is Empty
        assert_eq!(result, EvaluationResult::Empty);
    }
    
    #[test]
    fn test_extension_function_empty_url() {
        // Create a test element
        let element = EvaluationResult::Object(HashMap::new());
        
        // Test the extension function with an Empty URL
        let result = extension_function(
            &element, 
            &[EvaluationResult::Empty]
        ).unwrap();
        
        // Verify the result is Empty
        assert_eq!(result, EvaluationResult::Empty);
    }
    
    #[test]
    fn test_extension_function_multiple_matches() {
        // Create test extensions with the same URL
        let mut extension_obj1 = HashMap::new();
        extension_obj1.insert("url".to_string(), EvaluationResult::String("http://example.org/test-extension".to_string()));
        extension_obj1.insert("valueString".to_string(), EvaluationResult::String("value 1".to_string()));
        let extension1 = EvaluationResult::Object(extension_obj1);
        
        let mut extension_obj2 = HashMap::new();
        extension_obj2.insert("url".to_string(), EvaluationResult::String("http://example.org/test-extension".to_string()));
        extension_obj2.insert("valueString".to_string(), EvaluationResult::String("value 2".to_string()));
        let extension2 = EvaluationResult::Object(extension_obj2);
        
        // Create a test element with multiple extensions
        let mut obj = HashMap::new();
        obj.insert("extension".to_string(), EvaluationResult::Collection { items: vec![extension1.clone(), extension2.clone()], has_undefined_order: false });
        let element = EvaluationResult::Object(obj);
        
        // Test the extension function
        let result = extension_function(
            &element, 
            &[EvaluationResult::String("http://example.org/test-extension".to_string())]
        ).unwrap();
        
        // Verify the result is a collection containing both extensions
        assert!(matches!(result, EvaluationResult::Collection(_)));
        if let EvaluationResult::Collection(extensions) = result {
            assert_eq!(extensions.len(), 2);
            assert_eq!(extensions[0], extension1);
            assert_eq!(extensions[1], extension2);
        }
    }
    
    #[test]
    fn test_find_extension_in_underscore_property() {
        // Create a test extension
        let mut extension_obj = HashMap::new();
        extension_obj.insert("url".to_string(), EvaluationResult::String("http://example.org/test-extension".to_string()));
        extension_obj.insert("valueString".to_string(), EvaluationResult::String("test value".to_string()));
        let extension = EvaluationResult::Object(extension_obj);
        
        // Create a test underscore element
        let mut underscore_obj = HashMap::new();
        underscore_obj.insert("extension".to_string(), EvaluationResult::Collection { items: vec![extension.clone()], has_undefined_order: false });
        
        // Create a test parent object with the underscore element
        let mut parent_obj = HashMap::new();
        parent_obj.insert("_element".to_string(), EvaluationResult::Object(underscore_obj));
        
        // Test finding the extension
        let result = find_extension_in_underscore_property(
            &parent_obj,
            "element",
            "http://example.org/test-extension"
        );
        
        // Verify the result matches the extension
        assert_eq!(result, extension);
    }
}
