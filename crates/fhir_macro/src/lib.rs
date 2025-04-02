extern crate proc_macro;

use heck::{ToLowerCamelCase, ToPascalCase};
use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{
    parse_macro_input,
    punctuated::Punctuated,
    token,
    Attribute,
    Data,
    DeriveInput,
    Fields,
    GenericArgument,
    Ident,
    Lit,
    Meta,
    Path,
    PathArguments,
    Type,
    TypePath, 
};

// Helper function to get the effective field name for serialization/deserialization
// Respects #[serde(rename = "...")] attribute, otherwise defaults to camelCase.
fn get_effective_field_name(field: &syn::Field) -> String {
    for attr in &field.attrs {
        if attr.path().is_ident("serde") {
            if let Ok(list) =
                attr.parse_args_with(Punctuated::<Meta, token::Comma>::parse_terminated)
            {
                for meta in list {
                    if let Meta::NameValue(nv) = meta {
                        if nv.path.is_ident("rename") {
                            if let syn::Expr::Lit(expr_lit) = nv.value {
                                if let Lit::Str(lit_str) = expr_lit.lit {
                                    return lit_str.value();
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    // Default to camelCase if no rename attribute found
    field
        .ident
        .as_ref()
        .unwrap()
        .to_string()
        .to_lower_camel_case()
}

#[proc_macro_derive(FhirSerde)]
pub fn fhir_serde_derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident;
    let generics = input.generics;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let serialize_impl = generate_serialize_impl(&input.data, &name);

    // Pass all generic parts to deserialize generator
    let deserialize_impl = generate_deserialize_impl(
        &input.data,
        &name,
        &impl_generics,
        &ty_generics,
        &where_clause,
    );

    let expanded = quote! {
        // --- Serialize Implementation ---
        impl #impl_generics serde::Serialize for #name #ty_generics #where_clause {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                #serialize_impl
            }
        }

        // --- Deserialize Implementation ---
        impl<'de> #impl_generics serde::Deserialize<'de> for #name #ty_generics #where_clause
        where
            // Add bounds for generic types used in fields if necessary
            // Example: T: serde::Deserialize<'de>,
        {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                #deserialize_impl
            }
        }
    };

    TokenStream::from(expanded)
}

// Helper to check if a Type is Option<T> and return T
fn get_option_inner_type(ty: &Type) -> Option<&Type> {
    if let Type::Path(TypePath {
        path: Path { segments, .. },
        ..
    }) = ty
    {
        if let Some(segment) = segments.last() {
            if segment.ident == "Option" {
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

// Helper to check if a Type is Vec<T> and return T
fn get_vec_inner_type(ty: &Type) -> Option<&Type> {
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

// Helper to check if a Type is Box<T> and return T
fn get_box_inner_type(ty: &Type) -> Option<&Type> {
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

// Helper to check if a Type is Element<V, E> or DecimalElement<E>, potentially via a known alias.
// Returns (IsElement, IsDecimalElement, IsOption, IsVec, InnerType)
fn get_element_info(field_ty: &Type) -> (bool, bool, bool, bool, Option<&Type>) {
    // List of known FHIR primitive type aliases that wrap Element or DecimalElement
    // Note: This list might need adjustment based on the specific FHIR version/implementation details.
    const KNOWN_ELEMENT_ALIASES: &[&str] = &[
        "Base64Binary", "Boolean", "Canonical", "Code", "Date", "DateTime",
        "Id", "Instant", "Integer", "Markdown", "Oid", "PositiveInt",
        "String", "Time", "UnsignedInt", "Uri", "Url", "Uuid", "Xhtml",
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
                    Some(current_ty), // Return the type path itself
                );
            }
        }
    }

    (false, false, is_option, is_vec, None) // Not an Element or DecimalElement type we handle specially
}


fn generate_serialize_impl(data: &Data, name: &Ident) -> proc_macro2::TokenStream {
    match *data {
        Data::Struct(ref data) => {
            match data.fields {
                Fields::Named(ref fields) => {
                    // let field_count = fields.named.len(); // Field count is dynamic now
                    let mut field_serializers = Vec::new();
                    let mut field_count_calculator = Vec::new();

                    for field in fields.named.iter() {
                        let field_name_ident = field.ident.as_ref().unwrap(); // Keep original ident for access
                        let field_ty = &field.ty;
                        // Get effective name for JSON serialization
                        let effective_field_name_str = get_effective_field_name(field);

                        // Function to find and extract the path from skip_serializing_if
                        fn get_skip_serializing_if_path(attrs: &[Attribute]) -> Option<Path> {
                            attrs.iter().find_map(|attr| {
                                if attr.path().is_ident("serde") {
                                    // Use parse_args_with for syn 2.0
                                    match attr.parse_args_with(
                                        Punctuated::<Meta, token::Comma>::parse_terminated,
                                    ) {
                                        Ok(args) => {
                                            args.iter().find_map(|meta| {
                                                if let Meta::NameValue(nv) = meta {
                                                    if nv.path.is_ident("skip_serializing_if") {
                                                        // The value is now an Expr, check if it's a Lit::Str
                                                        if let syn::Expr::Lit(expr_lit) = &nv.value
                                                        {
                                                            if let Lit::Str(lit_str) = &expr_lit.lit
                                                            {
                                                                return syn::parse_str::<Path>(
                                                                    &lit_str.value(),
                                                                )
                                                                .ok();
                                                            }
                                                        }
                                                    }
                                                }
                                                None // Not the meta item we are looking for
                                            })
                                        }
                                        Err(_) => None, // Failed to parse args, ignore this attribute
                                    }
                                } else {
                                    None
                                } // Not a #[serde(...)] attribute
                            })
                        }

                        let skip_serializing_if_path = get_skip_serializing_if_path(&field.attrs);

                        let (is_element, is_decimal_element, is_option, is_vec, _inner_ty_opt) =
                            get_element_info(field_ty);

                        let is_fhir_element = is_element || is_decimal_element;

                        // Use field_name_ident for accessing the struct field
                        let field_access = quote! { self.#field_name_ident };

                        // --- Define skip_check logic FIRST ---
                        let skip_check = if let Some(condition_path) = &skip_serializing_if_path {
                            quote! { #condition_path(&#field_access) }
                        } else if is_option {
                            quote! { #field_access.is_some() }
                        } else {
                            quote! { false } // Don't skip non-optional fields by default
                        };

                        if is_fhir_element {
                            field_count_calculator.push(quote! { let mut #field_name_ident = false; });

                        // --- Field Count Calculation ---
                        // Special handling for FHIR elements that might add an extra `_field`
                        if !is_vec {
                            // Single Element or DecimalElement
                            field_count_calculator.push(quote! {
                                // Check outer skip condition first
                                if #skip_check {
                                     if let Some(element) = &#field_access {
                                         let has_value = element.value.is_some();
                                         let has_extension = element.id.is_some() || element.extension.is_some();
                                         if has_value && has_extension {
                                             count += 2; // fieldName + _fieldName
                                         } else if has_value || has_extension {
                                             count += 1; // fieldName OR _fieldName
                                         }
                                         // If neither, count remains 0 for this field
                                     }
                                }
                            });
                        } else if is_vec {
                            // Vec<Option<Element>>
                            field_count_calculator.push(quote! {
                                // Check outer skip condition first (for the Option<Vec> itself)
                                if #skip_check { // Now skip_check is defined
                                    if let Some(vec) = &#field_access {
                                        // Check if the primitive array needs serialization (any non-null value)
                                        let has_any_value = vec.iter().any(|opt_elem| opt_elem.as_ref().map_or(false, |elem| elem.value.is_some()));
                                        if has_any_value || !vec.is_empty() { // Serialize even if empty but not skipped
                                            count += 1; // Count the main field array
                                        }

                                        // Check if any element in the vec has extensions or id
                                        let has_any_extension = vec.iter().any(|opt_elem| {
                                            opt_elem.as_ref().map_or(false, |elem| elem.id.is_some() || elem.extension.is_some())
                                        });
                                        if has_any_extension {
                                            count += 1; // Count the underscore field array
                                        }
                                    }
                                     // If Option<Vec> is None, the outer skip_check handles it.
                                }
                            });
                            } else { panic!("Shouldn't get here") }

                        } else {
                            // Standard count logic for non-FHIR elements
                            field_count_calculator.push(quote! {
                                if #skip_check { // Now skip_check is defined
                                    count += 222221;
                                }
                            });
                        }

                        // --- Field Serialization Logic ---
                        // skip_check is already defined above

                        if is_fhir_element && !is_vec {
                            // Single Element or DecimalElement
                            field_serializers.push(quote! {
                                // Check the outer skip condition first
                                if #skip_check {
                                    if let Some(element) = &#field_access {
                                        let has_value = element.value.is_some();
                                        let has_extension = element.id.is_some() || element.extension.is_some();

                                        // Define helper struct locally for serializing only id/extension
                                        // Use ::fhir::r4::Extension as a concrete type for now, assuming R4 context or similar structure.
                                        // This might need adjustment if the macro needs to be more generic across FHIR versions.
                                        #[derive(serde::Serialize)]
                                        struct IdAndExtensionHelper<'a> {
                                            #[serde(skip_serializing_if = "Option::is_none")]
                                            id: &'a Option<String>,
                                            #[serde(skip_serializing_if = "Option::is_none")]
                                            extension: &'a Option<Vec<::fhir::r4::Extension>>, // Use concrete type
                                        }

                                        // Case 3: Both value and extension -> Serialize both fieldName and _fieldName
                                        if has_value && has_extension {
                                            // Serialize primitive value under fieldName explicitly using its Serialize impl
                                            // Assuming the inner value type V implements Serialize
                                            state.serialize_field(&#effective_field_name_str, element.value.as_ref().unwrap())?; // Pass the inner value directly
                                            
                                            // Serialize extension object under _fieldName using helper struct
                                            let underscore_field_name_str = format!("_{}", #effective_field_name_str);
                                            let extension_part = IdAndExtensionHelper {
                                                id: &element.id,
                                                extension: &element.extension,
                                            };
                                            state.serialize_field(&underscore_field_name_str, &extension_part)?;
                                        }
                                        // Case 1: Value only -> Serialize primitive under fieldName explicitly using its Serialize impl
                                        else if has_value { // && !has_extension is implied
                                            // Assuming the inner value type V implements Serialize
                                            state.serialize_field(&#effective_field_name_str, element.value.as_ref().unwrap())?; // Pass the inner value directly
                                        }
                                        // Case 2: Extension only -> Serialize helper object under fieldName (not _fieldName)
                                        else if has_extension { // && !has_value is implied
                                            #[derive(serde::Serialize)]
                                            struct IdAndExtensionHelper<'a> {
                                                #[serde(skip_serializing_if = "Option::is_none")]
                                                id: &'a Option<String>,
                                                #[serde(skip_serializing_if = "Option::is_none")]
                                                extension: &'a Option<Vec<::fhir::r4::Extension>>,
                                            }
                                            
                                            let extension_part = IdAndExtensionHelper {
                                                id: &element.id,
                                                extension: &element.extension,
                                            };
                                            state.serialize_field(&#effective_field_name_str, &extension_part)?;
                                        }
                                        // Case 4: Neither value nor extension -> Serialize nothing (field is skipped by count logic)
                                    }
                                    // If the outer Option was None, the skip_check handles it.
                                }
                            });
                        } else if is_fhir_element && is_vec {
                            // Vec<Option<Element>> or Vec<Element>
                            // Use effective_field_name_str here if needed for underscore name generation inside
                            // let underscore_field_name_str = format!("_{}", effective_field_name_str);
                            field_serializers.push(quote!{
                                // Check the outer skip condition first
                                if #skip_check {
                                    if let Some(vec) = &#field_access {
                                        // Serialize primitive array (fieldName) if not empty
                                        if !vec.is_empty() {
                                            // Prepare primitive values array: Vec<Option<ValueType>>
                                            let primitive_values: Vec<_> = vec.iter().map(|opt_elem| {
                                                opt_elem.as_ref().and_then(|elem| elem.value.as_ref()) // Clones the inner value if present
                                            }).collect();
                                            // Use effective name for serialization
                                            state.serialize_field(&#effective_field_name_str, &primitive_values)?;
                                        } else {
                                             // Serialize empty array if input vec is empty but Some and not skipped
                                             // Use effective name for serialization
                                             state.serialize_field(&#effective_field_name_str, &Vec::<Option<()>>::new())?; // Use appropriate dummy type if needed
                                        }

                                        // Prepare extension values array (_fieldName): Vec<Option<Element<(), Extension>>>
                                        let extension_values: Vec<Option<crate::Element<(), crate::r4::Extension>>> = vec.iter().map(|opt_elem| { // Assuming r4::Extension
                                            opt_elem.as_ref().and_then(|elem| {
                                                if elem.id.is_some() || elem.extension.is_some() {
                                                    Some(crate::Element::<(), crate::r4::Extension> { // Use dummy type for V
                                                        id: elem.id.clone(),
                                                        extension: elem.extension.clone(),
                                                        value: None,
                                                    })
                                                } else {
                                                    None // Represents null in the _fieldName array
                                                }
                                            })
                                        }).collect();

                                        // Only serialize _fieldName array if there's at least one non-null extension part
                                        if extension_values.iter().any(|opt_ext| opt_ext.is_some()) {
                                            // Create the underscore field name based on the effective name
                                            let underscore_field_name_str = format!("_{}", effective_field_name_str);
                                            state.serialize_field(&underscore_field_name_str, &extension_values)?;
                                        }
                                    }
                                     // If Option<Vec> is None, the outer skip_check handles it.
                                }
                            });
                        } else {
                            // Default serialization for non-FHIR-element fields
                            field_serializers.push(quote! {
                                if #skip_check {
                                    // Use effective name for serialization
                                    state.serialize_field(&#effective_field_name_str, &#field_access)?;
                                }
                            });
                        }
                    }

                    // Combine field count calculation and serialization
                    quote! {
                        let mut count = 0;
                        #(#field_count_calculator)* // Calculate the actual number of fields to serialize

                        let mut state = serializer.serialize_struct(stringify!(#name), count)?;
                        #(#field_serializers)*
                        state.end()
                    }
                }
                Fields::Unnamed(_) => panic!("Tuple structs not supported by FhirSerde"),
                Fields::Unit => panic!("Unit structs not supported by FhirSerde"),
            }
        }
        Data::Enum(_) | Data::Union(_) => panic!("Enums and Unions not supported by FhirSerde"),
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use syn::{parse_str, Type};
    use quote::ToTokens; // Import ToTokens trait

    // Helper to compare Option<&Type> by converting to string
    fn type_option_to_string(ty_opt: Option<&Type>) -> Option<String> {
        ty_opt.map(|ty| ty.to_token_stream().to_string())
    }

    #[test]
    fn test_get_element_info_option_element() {
        let ty: Type = parse_str("Option<Element<String, Extension>>").unwrap();
        let (is_element, is_decimal, is_option, is_vec, inner_ty) = get_element_info(&ty);
        assert!(is_element);
        assert!(!is_decimal);
        assert!(is_option);
        assert!(!is_vec);
        assert_eq!(type_option_to_string(inner_ty), Some("Element < String , Extension >".to_string()));
    }

    #[test]
    fn test_get_element_info_option_decimal_element() {
        let ty: Type = parse_str("Option<DecimalElement<Extension>>").unwrap();
        let (is_element, is_decimal, is_option, is_vec, inner_ty) = get_element_info(&ty);
        assert!(!is_element);
        assert!(is_decimal);
        assert!(is_option);
        assert!(!is_vec);
        assert_eq!(type_option_to_string(inner_ty), Some("DecimalElement < Extension >".to_string()));
    }

    #[test]
    fn test_get_element_info_option_string() {
        let ty: Type = parse_str("Option<String>").unwrap();
        let (is_element, is_decimal, is_option, is_vec, inner_ty) = get_element_info(&ty);
        assert!(!is_element);
        assert!(!is_decimal);
        assert!(is_option); // It is an Option
        assert!(!is_vec);
        assert!(inner_ty.is_none()); // Inner type is not Element or DecimalElement
    }

    #[test]
    fn test_get_element_info_option_vec_option_element() {
        let ty: Type = parse_str("Option<Vec<Option<Element<bool, Extension>>>>").unwrap();
        let (is_element, is_decimal, is_option, is_vec, inner_ty) = get_element_info(&ty);
        assert!(is_element);
        assert!(!is_decimal);
        assert!(is_option); // Outer Option
        assert!(is_vec);    // Vec is present
        assert_eq!(type_option_to_string(inner_ty), Some("Element < bool , Extension >".to_string()));
    }
    
    #[test]
    fn test_get_element_info_option_vec_option_decimal_element() {
        let ty: Type = parse_str("Option<Vec<Option<DecimalElement<Extension>>>>").unwrap();
        let (is_element, is_decimal, is_option, is_vec, inner_ty) = get_element_info(&ty);
        assert!(!is_element);
        assert!(is_decimal);
        assert!(is_option); // Outer Option
        assert!(is_vec);    // Vec is present
        assert_eq!(type_option_to_string(inner_ty), Some("DecimalElement < Extension >".to_string()));
    }

    #[test]
    fn test_get_element_info_option_vec_string() {
        let ty: Type = parse_str("Option<Vec<String>>").unwrap();
        let (is_element, is_decimal, is_option, is_vec, inner_ty) = get_element_info(&ty);
        assert!(!is_element);
        assert!(!is_decimal);
        assert!(is_option); // Outer Option
        assert!(is_vec);    // Vec is present
        assert!(inner_ty.is_none()); // Inner type is not Element or DecimalElement
    }

    #[test]
    fn test_get_element_info_element() {
        let ty: Type = parse_str("Element<String, Extension>").unwrap();
        let (is_element, is_decimal, is_option, is_vec, inner_ty) = get_element_info(&ty);
        assert!(is_element);
        assert!(!is_decimal);
        assert!(!is_option);
        assert!(!is_vec);
        assert_eq!(type_option_to_string(inner_ty), Some("Element < String , Extension >".to_string()));
    }

    #[test]
    fn test_get_element_info_decimal_element() {
        let ty: Type = parse_str("DecimalElement<Extension>").unwrap();
        let (is_element, is_decimal, is_option, is_vec, inner_ty) = get_element_info(&ty);
        assert!(!is_element);
        assert!(is_decimal);
        assert!(!is_option);
        assert!(!is_vec);
        assert_eq!(type_option_to_string(inner_ty), Some("DecimalElement < Extension >".to_string()));
    }

    #[test]
    fn test_get_element_info_string() {
        let ty: Type = parse_str("String").unwrap();
        let (is_element, is_decimal, is_option, is_vec, inner_ty) = get_element_info(&ty);
        assert!(!is_element);
        assert!(!is_decimal);
        assert!(!is_option);
        assert!(!is_vec);
        assert!(inner_ty.is_none());
    }

    #[test]
    fn test_get_element_info_vec_option_element() {
        // Less common, but test Vec<Option<Element>> without outer Option
        let ty: Type = parse_str("Vec<Option<Element<bool, Extension>>>").unwrap();
        let (is_element, is_decimal, is_option, is_vec, inner_ty) = get_element_info(&ty);
        assert!(is_element);
        assert!(!is_decimal);
        assert!(!is_option); // No outer Option
        assert!(is_vec);     // Vec is present
        assert_eq!(type_option_to_string(inner_ty), Some("Element < bool , Extension >".to_string()));
    }

    #[test]
    fn test_get_element_info_vec_option_decimal_element() {
        let ty: Type = parse_str("Vec<Option<DecimalElement<Extension>>>").unwrap();
        let (is_element, is_decimal, is_option, is_vec, inner_ty) = get_element_info(&ty);
        assert!(!is_element);
        assert!(is_decimal);
        assert!(!is_option); // No outer Option
        assert!(is_vec);     // Vec is present
        assert_eq!(type_option_to_string(inner_ty), Some("DecimalElement < Extension >".to_string()));
    }

    #[test]
    fn test_get_element_info_vec_string() {
        let ty: Type = parse_str("Vec<String>").unwrap();
        let (is_element, is_decimal, is_option, is_vec, inner_ty) = get_element_info(&ty);
        assert!(!is_element);
        assert!(!is_decimal);
        assert!(!is_option);
        assert!(is_vec);
        assert!(inner_ty.is_none());
    }

    #[test]
    fn test_get_element_info_option_box_element() {
        // Test with Box wrapping
        let ty: Type = parse_str("Option<Box<Element<String, Extension>>>").unwrap();
        let (is_element, is_decimal, is_option, is_vec, inner_ty) = get_element_info(&ty);
        assert!(is_element);
        assert!(!is_decimal);
        assert!(is_option);
        assert!(!is_vec);
        // The inner type returned should be the Element itself after unwrapping Box
        assert_eq!(type_option_to_string(inner_ty), Some("Element < String , Extension >".to_string()));
    }

    #[test]
    fn test_get_element_info_option_vec_option_box_element() {
        // Test with Box inside Vec<Option<...>>
        let ty: Type = parse_str("Option<Vec<Option<Box<Element<bool, Extension>>>>>").unwrap();
        let (is_element, is_decimal, is_option, is_vec, inner_ty) = get_element_info(&ty);
        assert!(is_element);
        assert!(!is_decimal);
        assert!(is_option); // Outer Option
        assert!(is_vec);    // Vec is present
        // The inner type returned should be the Element itself after unwrapping Box
        assert_eq!(type_option_to_string(inner_ty), Some("Element < bool , Extension >".to_string()));
    }

    #[test]
    fn test_get_element_info_type_alias() {
        // Simulate a type alias like `type Date = Element<String, Extension>;`
        // We parse the underlying type directly here. The function should resolve it.
        let ty: Type = parse_str("fhir::r4::Date").unwrap(); // Assuming Date is Element<...>
        // We can't directly test the alias resolution here without more context,
        // but we can test if it correctly identifies an Element path.
        // This test assumes `fhir::r4::Date` *looks like* an Element path segment.
        // A more robust test would involve actual type resolution which is complex in macros.

        // Let's test a path that *ends* with Element, simulating an alias.
        let ty_path: Type = parse_str("some::module::MyElementAlias").unwrap();
        // Manually construct a scenario where the last segment is "Element"
        // This is a simplification as we don't have real type info.
        let ty_simulated_alias: Type = parse_str("Element<String, Extension>").unwrap();


        // Test with a path that *doesn't* end in Element/DecimalElement
        let ty_non_element_path: Type = parse_str("some::module::RegularStruct").unwrap();
        let (is_element, is_decimal, is_option, is_vec, inner_ty) = get_element_info(&ty_non_element_path);
        assert!(!is_element);
        assert!(!is_decimal);
        assert!(!is_option);
        assert!(!is_vec);
        assert!(inner_ty.is_none());

        // Test with a path that *does* end in Element (simulating alias)
        // We use the actual Element type parsed earlier for this simulation
        let (is_element, is_decimal, is_option, is_vec, inner_ty) = get_element_info(&ty_simulated_alias);
        assert!(is_element);
        assert!(!is_decimal);
        assert!(!is_option);
        assert!(!is_vec);
        assert_eq!(type_option_to_string(inner_ty), Some("Element < String , Extension >".to_string()));

    }
}

// Add impl_generics and where_clause as parameters
fn generate_deserialize_impl(
    data: &Data,
    name: &Ident,
    impl_generics: &syn::ImplGenerics,
    ty_generics: &syn::TypeGenerics,
    // Accept the type returned by split_for_impl
    where_clause: &Option<&syn::WhereClause>,
) -> proc_macro2::TokenStream {
    match *data {
        Data::Struct(ref data) => {
            match data.fields {
                Fields::Named(ref fields) => {
                    let struct_name_str = name.to_string();
                    let visitor_name =
                        format_ident!("{}Visitor", name.to_string().to_pascal_case());

                    // Remove unused field_name_strs
                    // let field_name_strs: Vec<_> = field_names.iter().map(|f| f.to_string()).collect();

                    // Create enum variants for field matching
                    let field_enum_name =
                        format_ident!("{}Field", name.to_string().to_pascal_case()); // Keep this for the enum name
                                                                                     // Helper to get aliases
                    fn get_field_aliases(attrs: &[Attribute]) -> Vec<String> {
                        attrs
                            .iter()
                            .flat_map(|attr| -> Vec<String> {
                                // Outer closure returns Vec<String>
                                if attr.path().is_ident("serde") {
                                    match attr.parse_args_with(
                                        Punctuated::<Meta, token::Comma>::parse_terminated,
                                    ) {
                                        Ok(args) => {
                                            // Inner closure for filter_map returns Option<String>
                                            args.iter()
                                                .filter_map(|meta| {
                                                    if let Meta::NameValue(nv) = meta {
                                                        if nv.path.is_ident("alias") {
                                                            if let syn::Expr::Lit(expr_lit) =
                                                                &nv.value
                                                            {
                                                                if let Lit::Str(lit_str) =
                                                                    &expr_lit.lit
                                                                {
                                                                    return Some(lit_str.value());
                                                                    // Correct: filter_map expects Option
                                                                }
                                                            }
                                                        }
                                                    }
                                                    None // Correct: filter_map expects Option
                                                })
                                                .collect::<Vec<String>>() // Collects Option<String> into Vec<String>
                                        }
                                        Err(_) => vec![], // Correct: flat_map expects IntoIterator<Item = String>
                                    }
                                } else {
                                    vec![] // Correct: flat_map expects IntoIterator<Item = String>
                                }
                            })
                            .collect() // Collects Strings from all attributes
                    }

                    // Remove unused field_enum_name variable definition here
                    // let field_enum_name = format_ident!("{}Field", name.to_string().to_pascal_case());
                    let mut field_enum_variants_map = std::collections::HashMap::new(); // Store PascalCase variant name per field ident
                    let mut underscore_field_enum_variants = Vec::new();
                    let mut field_match_arms = Vec::new();
                    let mut is_fhir_element_field = Vec::new(); // Track which fields are FHIR elements

                    for field in fields.named.iter() {
                        let field_ident = field.ident.as_ref().unwrap();
                        let effective_field_name_str = get_effective_field_name(field); // Use helper
                        let variant = format_ident!("{}", field_ident.to_string().to_pascal_case()); // Variant based on Rust ident
                        field_enum_variants_map.insert(field_ident.clone(), variant.clone());

                        let field_ty = &field.ty;
                        let (is_element, is_decimal_element, _is_option, _is_vec, _inner_ty_opt) =
                            get_element_info(field_ty);
                        let is_fhir_elem = is_element || is_decimal_element;
                        is_fhir_element_field.push(is_fhir_elem); // Store result

                        // Match effective name
                        field_match_arms.push(
                            quote! { #effective_field_name_str => Ok(#field_enum_name::#variant) },
                        );

                        // Match aliases
                        let aliases = get_field_aliases(&field.attrs);
                        for alias in aliases {
                            field_match_arms
                                .push(quote! { #alias => Ok(#field_enum_name::#variant) });
                        }

                        if is_fhir_elem {
                            let underscore_field_name_str =
                                format!("_{}", effective_field_name_str);
                            let underscore_variant =
                                format_ident!("_{}", field_ident.to_string().to_pascal_case()); // Underscore variant also based on Rust ident
                            underscore_field_enum_variants.push(underscore_variant.clone()); // Add to list of variants
                                                                                             // Match underscore name
                            field_match_arms.push(quote! { #underscore_field_name_str => Ok(#field_enum_name::#underscore_variant) });
                            // Match underscore aliases? (Less common, skip for now)
                        }
                    }
                    let ignore_variant = format_ident!("Ignore");
                    // Extract just the variant names for the enum definition
                    let field_enum_variants: Vec<_> =
                        field_enum_variants_map.values().cloned().collect();

                    // Generate field storage (using Option for all fields)
                    // let field_storage: Vec<_> = field_types.iter().zip(field_names.iter()).map(|(ty, name)| {
                    //     // Use Option<Type> for storage to track presence
                    //     quote! { #name: Option<#ty> }
                    // }).collect();

                    // Generate visitor map access logic
                    let mut map_access_logic = Vec::new();
                    let mut temp_field_storage = Vec::new(); // For storing Option<serde_json::Value>

                    for (i, field) in fields.named.iter().enumerate() {
                        // Iterate over fields again
                        let field_ident = field.ident.as_ref().unwrap();
                        let effective_field_name_str = get_effective_field_name(field); // Use helper
                        let variant = field_enum_variants_map.get(field_ident).unwrap(); // Get variant from map
                        let temp_field_name = format_ident!("temp_{}", field_ident);

                        // Initialize temporary storage for the main field
                        temp_field_storage.push(
                            quote! { let mut #temp_field_name: Option<serde_json::Value> = None; },
                        );

                        // Logic to populate temporary storage for the main field
                        map_access_logic.push(quote! {
                            #field_enum_name::#variant => {
                                if #temp_field_name.is_some() {
                                    // Use effective name in error
                                    return Err(serde::de::Error::duplicate_field(#effective_field_name_str));
                                }
                                #temp_field_name = Some(map.next_value()?);
                            }
                        });

                        // If it's a FHIR element, also handle the underscore field
                        if is_fhir_element_field[i] {
                            let underscore_field_name_str =
                                format!("_{}", effective_field_name_str); // Use effective name
                            let underscore_variant =
                                format_ident!("_{}", field_ident.to_string().to_pascal_case()); // Variant based on Rust ident
                            let temp_underscore_field_name =
                                format_ident!("temp_{}", underscore_field_name_str);

                            // Initialize temporary storage for the underscore field
                            temp_field_storage.push(quote! { let mut #temp_underscore_field_name: Option<serde_json::Value> = None; });

                            // Logic to populate temporary storage for the underscore field
                            map_access_logic.push(quote! {
                                #field_enum_name::#underscore_variant => {
                                    if #temp_underscore_field_name.is_some() {
                                        return Err(serde::de::Error::duplicate_field(#underscore_field_name_str));
                                    }
                                    #temp_underscore_field_name = Some(map.next_value()?);
                                }
                            });
                        }
                    }

                    // Generate the logic to construct the final field values *after* the loop
                    let final_construction_logic: Vec<_> = fields.named.iter().enumerate().map(|(i, field)| {
                        let field_ident = field.ident.as_ref().unwrap();
                        let field_ty = &field.ty;
                        let temp_field_name = format_ident!("temp_{}", field_ident);
                        let is_fhir_elem = is_fhir_element_field[i]; // Use the stored boolean

                        if is_fhir_elem {
                            let effective_field_name_str = get_effective_field_name(field);
                            let underscore_field_name_str = format!("_{}", effective_field_name_str);
                            let temp_underscore_field_name = format_ident!("temp_{}", underscore_field_name_str);
                            let (_is_element, _is_decimal_element, is_option, is_vec, _inner_ty_opt) = get_element_info(field_ty);

                            if is_vec {
                                // Handle Vec<Option<Element>> or Vec<Element>
                                // Remove outer braces, return only the let statement
                                quote! {
                                    let #field_ident: #field_ty = {
                                        // Deserialize primitive array (fieldName)
                                        let primitives: Option<Vec<Option<_>>> = #temp_field_name
                                            .map(|v| serde_json::from_value(v).map_err(serde::de::Error::custom))
                                            .transpose()?;

                                        // Deserialize extension array (_fieldName) - Use concrete Extension type
                                        let extensions: Option<Vec<Option<crate::Element<(), crate::r4::Extension>>>> = #temp_underscore_field_name
                                            .map(|v| serde_json::from_value(v).map_err(serde::de::Error::custom))
                                            .transpose()?;

                                        // Combine logic
                                        match (primitives, extensions) {
                                            (Some(p_vec), Some(e_vec)) => {
                                                if p_vec.len() != e_vec.len() {
                                                    return Err(serde::de::Error::custom(format!("Array length mismatch for field '{}' ({} vs {})", stringify!(#field_ident), p_vec.len(), e_vec.len())));
                                                }
                                                let mut combined = Vec::with_capacity(p_vec.len());
                                                for (p_opt, e_opt) in p_vec.into_iter().zip(e_vec.into_iter()) {
                                                    let element_opt = match (p_opt, e_opt) {
                                                        (Some(p), Some(e)) => Some(crate::Element { id: e.id, extension: e.extension, value: Some(p) }),
                                                        (Some(p), None) => Some(crate::Element { id: None, extension: None, value: Some(p) }),
                                                        (None, Some(e)) => Some(crate::Element { id: e.id, extension: e.extension, value: None }),
                                                        (None, None) => None,
                                                    };
                                                    combined.push(element_opt);
                                                }
                                                Some(combined)
                                            },
                                            (Some(p_vec), None) => { // Only primitives
                                                Some(p_vec.into_iter().map(|p_opt| {
                                                    p_opt.map(|p| crate::Element { id: None, extension: None, value: Some(p) })
                                                }).collect())
                                            },
                                            (None, Some(e_vec)) => { // Only extensions
                                                Some(e_vec.into_iter().map(|e_opt| {
                                                    e_opt.map(|e| crate::Element { id: e.id, extension: e.extension, value: None })
                                                }).collect())
                                            },
                                            (None, None) => None,
                                        }
                                    }; // End of let binding block
                                }
                            } else {
                                // Handle single Option<Element> or Element
                                // Remove outer braces, return only the let statement
                                quote! {
                                    let #field_ident: #field_ty = {
                                        let primitive_value_json: Option<serde_json::Value> = #temp_field_name;
                                        let extension_value_json: Option<serde_json::Value> = #temp_underscore_field_name;

                                        let combined_json_to_deserialize = match (primitive_value_json, extension_value_json) {
                                            (Some(prim_val), Some(mut ext_obj @ serde_json::Value::Object(_))) => {
                                                if let serde_json::Value::Object(map) = &mut ext_obj {
                                                    map.insert("value".to_string(), prim_val);
                                                }
                                                Some(ext_obj)
                                            },
                                            (Some(prim_val), None) => Some(prim_val),
                                            (None, Some(ext_obj)) => Some(ext_obj),
                                            (None, None) => None,
                                        };

                                        match combined_json_to_deserialize {
                                            Some(json) => serde_json::from_value(json).map_err(serde::de::Error::custom)?,
                                            None => {
                                                if #is_option {
                                                    None
                                                } else {
                                                    Default::default()
                                                }
                                            }
                                        }
                                    }; // End of let binding block
                                }
                            }
                        } else {
                            // Default deserialization for non-FHIR-element fields
                            // Default deserialization for non-FHIR-element fields
                            // Ensure this also returns just the let statement TokenStream
                            quote! {
                                let #field_ident: #field_ty = match #temp_field_name {
                                    Some(v) => serde_json::from_value(v).map_err(serde::de::Error::custom)?,
                                    None => Default::default(), // Assumes #field_ty implements Default
                                };
                            }
                        }
                    }).collect();

                    // Get just the field idents for the final struct construction
                    let final_field_idents: Vec<_> = fields.named.iter().map(|field| {
                        field.ident.as_ref().unwrap()
                    }).collect();

                    // Assemble the final struct instantiation using the field idents
                    let struct_instantiation = quote! {
                        #name {
                            #(#final_field_idents),* // Use just the idents
                        }
                    };

                    // --- Visitor and Deserialize Implementation ---
                    quote! {
                        #[derive(serde::Deserialize)]
                        #[serde(field_identifier, rename_all = "camelCase")]
                        enum #field_enum_name {
                            #(#field_enum_variants,)*
                            #(#underscore_field_enum_variants,)*
                            #[serde(other)]
                            #ignore_variant
                        }

                        struct #visitor_name #ty_generics #where_clause;

                        impl<'de> #impl_generics serde::de::Visitor<'de> for #visitor_name #ty_generics #where_clause {
                            type Value = #name #ty_generics;

                            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                                formatter.write_str(concat!("struct ", #struct_name_str))
                            }

                            fn visit_map<V>(self, mut map: V) -> Result<Self::Value, V::Error>
                            where
                                V: serde::de::MapAccess<'de>,
                            {
                                #(#temp_field_storage)*

                                while let Some(key) = map.next_key()? {
                                    match key {
                                        #(#map_access_logic)*
                                        #field_enum_name::#ignore_variant => {
                                            let _ = map.next_value::<serde::de::IgnoredAny>()?;
                                        }
                                    }
                                }

                                // Process temp storage to build final fields using let bindings
                                #(#final_construction_logic)*

                                // Construct the final struct using the field idents
                                Ok(#struct_instantiation)
                            }
                        } // end impl Visitor

                        // Start deserialization using the visitor
                        deserializer.deserialize_map(#visitor_name) // Pass visitor with generics if needed, but deserialize_map doesn't take it directly
                    } // end quote!
                }
                Fields::Unnamed(_) => panic!("Tuple structs not supported by FhirSerde"),
                Fields::Unit => panic!("Unit structs not supported by FhirSerde"),
            }
        }
        Data::Enum(_) | Data::Union(_) => panic!("Enums and Unions not supported by FhirSerde"),
    }
}
