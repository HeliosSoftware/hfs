use syn::{GenericArgument, Path, PathArguments, Type, TypePath};

/// Extracts the inner type from an `Option<T>` type.
///
/// This helper function analyzes a type path to determine if it represents an
/// `Option<T>` and extracts the inner type `T` if so.
///
/// # Arguments
///
/// * `ty` - The type to analyze
///
/// # Returns
///
/// - `Some(&Type)` containing the inner type if this is an `Option<T>`
/// - `None` if this is not an `Option` type
///
/// # Examples
///
/// ```rust,ignore
/// // For type: Option<String>
/// // Returns: Some(String)
///
/// // For type: String  
/// // Returns: None
///
/// // For type: Option<Vec<HumanName>>
/// // Returns: Some(Vec<HumanName>)
/// ```
pub(crate) fn get_option_inner_type(ty: &Type) -> Option<&Type> {
    if let Type::Path(TypePath {
        path: Path { segments, .. },
        ..
    }) = ty
        && let Some(segment) = segments.last()
        && segment.ident == "Option"
        && let PathArguments::AngleBracketed(args) = &segment.arguments
        && let Some(GenericArgument::Type(inner_ty)) = args.args.first()
    {
        return Some(inner_ty);
    }
    None
}

/// Extracts the inner type from a `Vec<T>` type.
///
/// This helper function analyzes a type path to determine if it represents a
/// `Vec<T>` and extracts the inner type `T` if so.
///
/// # Arguments
///
/// * `ty` - The type to analyze
///
/// # Returns
///
/// - `Some(&Type)` containing the inner type if this is a `Vec<T>`
/// - `None` if this is not a `Vec` type
///
/// # Examples
///
/// ```rust,ignore
/// // For type: Vec<String>
/// // Returns: Some(String)
///
/// // For type: String
/// // Returns: None
///
/// // For type: Vec<HumanName>
/// // Returns: Some(HumanName)
/// ```
pub(crate) fn get_vec_inner_type(ty: &Type) -> Option<&Type> {
    if let Type::Path(TypePath {
        path: Path { segments, .. },
        ..
    }) = ty
    {
        if let Some(segment) = segments.last() {
            if segment.ident == "Vec" {
                if let PathArguments::AngleBracketed(args) = &segment.arguments {
                    if let Some(GenericArgument::Type(inner_ty)) = args.args.first() {
                        return Some(inner_ty);
                    }
                }
            }
        }
    }
    None
}

/// Extracts the inner type from a `Box<T>` type.
///
/// This helper function analyzes a type path to determine if it represents a
/// `Box<T>` and extracts the inner type `T` if so. Box types are used in FHIR
/// for cycle breaking in recursive data structures.
///
/// # Arguments
///
/// * `ty` - The type to analyze
///
/// # Returns
///
/// - `Some(&Type)` containing the inner type if this is a `Box<T>`
/// - `None` if this is not a `Box` type
///
/// # Examples
///
/// ```rust,ignore
/// // For type: Box<Reference>
/// // Returns: Some(Reference)
///
/// // For type: Reference
/// // Returns: None
/// ```
pub(crate) fn get_box_inner_type(ty: &Type) -> Option<&Type> {
    if let Type::Path(TypePath {
        path: Path { segments, .. },
        ..
    }) = ty
    {
        if let Some(segment) = segments.last() {
            if segment.ident == "Box" {
                if let PathArguments::AngleBracketed(args) = &segment.arguments {
                    if let Some(GenericArgument::Type(inner_ty)) = args.args.first() {
                        return Some(inner_ty);
                    }
                }
            }
        }
    }
    None
}

/// Analyzes a type to determine FHIR element characteristics and container wrapping.
///
/// This function is central to the FHIR serialization logic, determining how a field
/// should be handled based on its type. It identifies FHIR element types and their
/// container wrappers to generate appropriate serialization code.
///
/// # Type Analysis
///
/// The function recursively unwraps container types in this order:
/// 1. `Option<T>` → T (marks as optional)
/// 2. `Vec<T>` → T (marks as vector, handles `Vec<Option<T>>` case)
/// 3. `Box<T>` → T (unwraps boxed types)
/// 4. Analyzes the final type for FHIR element characteristics
///
/// # FHIR Element Types
///
/// - **Element types**: FHIR primitive type aliases like `String`, `Boolean`, `Code`
/// - **DecimalElement types**: The special `Decimal` type requiring precision preservation
/// - **Direct types**: `Element<V, E>` and `DecimalElement<E>` generic types
///
/// # Arguments
///
/// * `field_ty` - The type to analyze (may be wrapped in Option/Vec/Box)
///
/// # Returns
///
/// A tuple `(is_element, is_decimal_element, is_option, is_vec)` where:
/// - `is_element` - True if this is a FHIR element type (not decimal)
/// - `is_decimal_element` - True if this is a FHIR decimal element type
/// - `is_option` - True if the type was wrapped in `Option<T>`
/// - `is_vec` - True if the type was wrapped in `Vec<T>`
///
/// # Examples
///
/// ```rust,ignore
/// // Option<String> (FHIR element alias)
/// // Returns: (true, false, true, false)
///
/// // Vec<Decimal> (FHIR decimal element alias)  
/// // Returns: (false, true, false, true)
///
/// // Option<Vec<Boolean>> (FHIR element in vector)
/// // Returns: (true, false, true, true)
///
/// // Element<String, Extension> (direct element type)
/// // Returns: (true, false, false, false)
///
/// // i32 (regular Rust type, not FHIR element)
/// // Returns: (false, false, false, false)
/// ```
pub(crate) fn get_element_info(field_ty: &Type) -> (bool, bool, bool, bool) {
    // List of known FHIR primitive type aliases that wrap Element or DecimalElement
    // Note: This list might need adjustment based on the specific FHIR version/implementation details.
    // IMPORTANT: Do not include base Rust types like "String", "bool", "i32" here.
    // This list is for type aliases that *wrap* fhir::Element or fhir::DecimalElement.
    const KNOWN_ELEMENT_ALIASES: &[&str] = &[
        "Base64Binary",
        "Boolean",
        "Canonical",
        "Code",
        "Date",
        "DateTime",
        "Id",
        "Instant",
        "Integer",
        "Markdown",
        "Oid",
        "PositiveInt",
        "String",
        "Time",
        "UnsignedInt",
        "Uri",
        "Url",
        "Uuid",
        "Xhtml",
        // Struct types that might be used directly or within Elements (e.g., Address, HumanName)
        // are NOT typically handled by this _fieldName logic, so they are excluded here.
        // Resource types (Patient, Observation) are also excluded.
    ];
    const KNOWN_DECIMAL_ELEMENT_ALIAS: &str = "Decimal";

    let mut is_option = false;
    let mut is_vec = false;
    let mut current_ty = field_ty;

    // Unwrap Option
    if let Some(inner) = get_option_inner_type(current_ty) {
        is_option = true;
        current_ty = inner;
    }

    // Unwrap Vec
    if let Some(inner) = get_vec_inner_type(current_ty) {
        is_vec = true;
        current_ty = inner;
        // Check if Vec contains Option<Element>
        if let Some(vec_option_inner) = get_option_inner_type(current_ty) {
            current_ty = vec_option_inner; // Now current_ty is the Element type inside Vec<Option<...>>
        } else {
            // If it's Vec<Element> directly (less common for primitives), handle it
            // current_ty is already the Element type inside Vec<...>
        }
    }

    // Unwrap Box if present (e.g., Box<Reference> inside Element)
    if let Some(inner) = get_box_inner_type(current_ty) {
        current_ty = inner;
    }

    // Check if the (potentially unwrapped) type path ends with Element or DecimalElement
    if let Type::Path(TypePath { path, .. }) = current_ty {
        if let Some(segment) = path.segments.last() {
            let type_name_ident = &segment.ident;
            let type_name_str = type_name_ident.to_string();

            // Check if the last segment's identifier is Element, DecimalElement, or a known alias
            let is_direct_element = type_name_str == "Element";
            let is_direct_decimal_element = type_name_str == "DecimalElement";
            let is_known_element_alias = KNOWN_ELEMENT_ALIASES.contains(&type_name_str.as_str());
            let is_known_decimal_alias = type_name_str == KNOWN_DECIMAL_ELEMENT_ALIAS;

            let is_element = is_direct_element || is_known_element_alias;
            let is_decimal_element = is_direct_decimal_element || is_known_decimal_alias;

            if is_element || is_decimal_element {
                // It's considered an element type if it's Element, DecimalElement, or a known alias
                return (
                    is_element && !is_decimal_element, // Ensure is_element is false if it's a decimal type
                    is_decimal_element,
                    is_option,
                    is_vec,
                );
            }
        }
    }

    (false, false, is_option, is_vec) // Not an Element or DecimalElement type we handle specially
}

// Keep this in sync with generate_primitive_type in fhir_gen/src/lib.rs
// Helper function to get the inner type T from Option<T>, Vec<T>, or Box<T>
pub(crate) fn get_inner_type(ty: &Type) -> Option<&Type> {
    if let Type::Path(TypePath { path, .. }) = ty {
        if let Some(segment) = path.segments.last() {
            if segment.ident == "Option" || segment.ident == "Vec" || segment.ident == "Box" {
                if let PathArguments::AngleBracketed(args) = &segment.arguments {
                    if let Some(GenericArgument::Type(inner_ty)) = args.args.first() {
                        return Some(inner_ty);
                    }
                }
            }
        }
    }
    None
}

// Helper function to recursively unwrap Option, Vec, and Box to get the base type
pub(crate) fn get_base_type(ty: &Type) -> &Type {
    let mut current_ty = ty;
    while let Some(inner) = get_inner_type(current_ty) {
        current_ty = inner;
    }
    current_ty
}

pub(crate) fn extract_inner_element_type(type_name: &str) -> &str {
    match type_name {
        "Boolean" => "bool",
        "Integer" | "PositiveInt" | "UnsignedInt" => "std::primitive::i32",
        "Decimal" => "rust_decimal::Decimal", // Use the actual Decimal type
        "Integer64" => "std::primitive::i64",
        "String" | "Code" | "Base64Binary" | "Canonical" | "Id" | "Oid" | "Uri" | "Url"
        | "Uuid" | "Markdown" | "Xhtml" => "std::string::String",
        "Date" => "crate::PrecisionDate",
        "DateTime" => "crate::PrecisionDateTime",
        "Instant" => "crate::PrecisionInstant",
        "Time" => "crate::PrecisionTime",
        _ => "std::string::String", // Default or consider panic/error
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::{Type, parse_str};
    #[test]
    fn test_get_element_info_option_element() {
        let ty: Type = parse_str("Option<Element<Markdown, Extension>>").unwrap();
        let (is_element, is_decimal, is_option, is_vec) = get_element_info(&ty);
        assert!(is_element);
        assert!(!is_decimal);
        assert!(is_option);
        assert!(!is_vec);
    }

    #[test]
    fn test_get_element_info_option_decimal_element() {
        let ty: Type = parse_str("Option<DecimalElement<Extension>>").unwrap();
        let (is_element, is_decimal, is_option, is_vec) = get_element_info(&ty);
        assert!(!is_element);
        assert!(is_decimal);
        assert!(is_option);
        assert!(!is_vec);
    }

    #[test]
    fn test_get_element_info_option_markdown() {
        let ty: Type = parse_str("Option<Markdown>").unwrap();
        let (is_element, is_decimal, is_option, is_vec) = get_element_info(&ty);
        assert!(is_element); // Markdown should be identified as Element
        assert!(!is_decimal);
        assert!(is_option); // It is an Option
        assert!(!is_vec);
    }

    #[test]
    fn test_get_element_info_option_vec_option_element() {
        let ty: Type = parse_str("Option<Vec<Option<Element<bool, Extension>>>>").unwrap();
        let (is_element, is_decimal, is_option, is_vec) = get_element_info(&ty);
        assert!(is_element);
        assert!(!is_decimal);
        assert!(is_option); // Outer Option
        assert!(is_vec); // Vec is present
    }

    #[test]
    fn test_get_element_info_option_vec_option_decimal_element() {
        let ty: Type = parse_str("Option<Vec<Option<DecimalElement<Extension>>>>").unwrap();
        let (is_element, is_decimal, is_option, is_vec) = get_element_info(&ty);
        assert!(!is_element);
        assert!(is_decimal);
        assert!(is_option); // Outer Option
        assert!(is_vec); // Vec is present
    }

    #[test]
    fn test_get_element_info_option_vec_markdown() {
        let ty: Type = parse_str("Option<Vec<Markdown>>").unwrap();
        let (is_element, is_decimal, is_option, is_vec) = get_element_info(&ty);
        assert!(is_element); // Markdown should be identified as Element
        assert!(!is_decimal);
        assert!(is_option); // Outer Option
        assert!(is_vec); // Vec is present
    }

    #[test]
    fn test_get_element_info_element() {
        let ty: Type = parse_str("Element<String, Extension>").unwrap();
        let (is_element, is_decimal, is_option, is_vec) = get_element_info(&ty);
        assert!(is_element);
        assert!(!is_decimal);
        assert!(!is_option);
        assert!(!is_vec);
    }

    #[test]
    fn test_get_element_info_decimal_element() {
        let ty: Type = parse_str("DecimalElement<Extension>").unwrap();
        let (is_element, is_decimal, is_option, is_vec) = get_element_info(&ty);
        assert!(!is_element);
        assert!(is_decimal);
        assert!(!is_option);
        assert!(!is_vec);
    }

    #[test]
    fn test_get_element_info_markdown() {
        let ty: Type = parse_str("Markdown").unwrap();
        let (is_element, is_decimal, is_option, is_vec) = get_element_info(&ty);
        assert!(is_element); // Markdown should be identified as Element
        assert!(!is_decimal);
        assert!(!is_option);
        assert!(!is_vec);
    }

    #[test]
    fn test_get_element_info_vec_option_element() {
        // Less common, but test Vec<Option<Element>> without outer Option
        let ty: Type = parse_str("Vec<Option<Element<bool, Extension>>>").unwrap();
        let (is_element, is_decimal, is_option, is_vec) = get_element_info(&ty);
        assert!(is_element);
        assert!(!is_decimal);
        assert!(!is_option); // No outer Option
        assert!(is_vec); // Vec is present
    }

    #[test]
    fn test_get_element_info_vec_option_decimal_element() {
        let ty: Type = parse_str("Vec<Option<DecimalElement<Extension>>>").unwrap();
        let (is_element, is_decimal, is_option, is_vec) = get_element_info(&ty);
        assert!(!is_element);
        assert!(is_decimal);
        assert!(!is_option); // No outer Option
        assert!(is_vec); // Vec is present
    }

    #[test]
    fn test_get_element_info_vec_string() {
        let ty: Type = parse_str("Vec<String>").unwrap();
        let (is_element, is_decimal, is_option, is_vec) = get_element_info(&ty);
        // String IS identified as Element because it's in KNOWN_ELEMENT_ALIASES
        assert!(is_element);
        assert!(!is_decimal);
        assert!(!is_option);
        assert!(is_vec);
    }

    #[test]
    fn test_get_element_info_option_box_element() {
        // Test with Box wrapping
        let ty: Type = parse_str("Option<Box<Element<String, Extension>>>").unwrap();
        let (is_element, is_decimal, is_option, is_vec) = get_element_info(&ty);
        assert!(is_element);
        assert!(!is_decimal);
        assert!(is_option);
        assert!(!is_vec);
    }

    #[test]
    fn test_get_element_info_option_vec_option_box_element() {
        // Test with Box inside Vec<Option<...>>
        let ty: Type = parse_str("Option<Vec<Option<Box<Element<bool, Extension>>>>>").unwrap();
        let (is_element, is_decimal, is_option, is_vec) = get_element_info(&ty);
        assert!(is_element);
        assert!(!is_decimal);
        assert!(is_option); // Outer Option
        assert!(is_vec); // Vec is present
    }

    #[test]
    fn test_get_element_info_type_alias() {
        // Simulate a type alias like `type Date = Element<String, Extension>;`
        // We parse the underlying type directly here. The function should resolve it.
        let _ty: Type = parse_str("fhir::r4::Date").unwrap(); // Prefix unused variable
        // We can't directly test the alias resolution here without more context,
        // but we can test if it correctly identifies an Element path.
        // This test assumes `fhir::r4::Date` *looks like* an Element path segment.
        // A more robust test would involve actual type resolution which is complex in macros.

        // Let's test a path that *ends* with Element, simulating an alias.
        let _ty_path: Type = parse_str("some::module::MyElementAlias").unwrap(); // Prefix unused variable
        // Manually construct a scenario where the last segment is "Element"
        // This is a simplification as we don't have real type info.
        let ty_simulated_alias: Type = parse_str("Element<String, Extension>").unwrap();

        // Test with a path that *doesn't* end in Element/DecimalElement
        let ty_non_element_path: Type = parse_str("some::module::RegularStruct").unwrap();
        let (is_element, is_decimal, is_option, is_vec) = get_element_info(&ty_non_element_path);
        assert!(!is_element);
        assert!(!is_decimal);
        assert!(!is_option);
        assert!(!is_vec);

        // Test with a path that *does* end in Element (simulating alias)
        // We use the actual Element type parsed earlier for this simulation
        let (is_element, is_decimal, is_option, is_vec) = get_element_info(&ty_simulated_alias);
        assert!(is_element);
        assert!(!is_decimal);
        assert!(!is_option);
        assert!(!is_vec);
    }
}
