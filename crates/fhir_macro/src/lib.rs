extern crate proc_macro;

use heck::ToLowerCamelCase;
use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{
    Data, DeriveInput, Fields, GenericArgument, Ident, Lit, Meta, Path, PathArguments, Type,
    TypePath, parse_macro_input, punctuated::Punctuated, token,
};

// Helper function to get the effective field name for serialization/deserialization
// Respects #[fhir_serde(rename = "...")] attribute, otherwise defaults to camelCase.
fn get_effective_field_name(field: &syn::Field) -> String {
    for attr in &field.attrs {
        if attr.path().is_ident("fhir_serde") {
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

// Helper function to check if a field has #[fhir_serde(flatten)]
fn is_flattened(field: &syn::Field) -> bool {
    for attr in &field.attrs {
        if attr.path().is_ident("fhir_serde") {
            if let Ok(list) =
                attr.parse_args_with(Punctuated::<Meta, token::Comma>::parse_terminated)
            {
                for meta in list {
                    if let Meta::Path(path) = meta {
                        if path.is_ident("flatten") {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

#[proc_macro_derive(FhirSerde, attributes(fhir_serde))] // Add attributes(fhirserde) here
pub fn fhir_serde_derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident;
    let generics = input.generics;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let serialize_impl = generate_serialize_impl(&input.data, &name);

    // Pass all generic parts to deserialize generator
    let deserialize_impl = generate_deserialize_impl(&input.data, &name);

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
// Returns (IsElement, IsDecimalElement, IsOption, IsVec, Option<InnerType>)
fn get_element_info(field_ty: &Type) -> (bool, bool, bool, bool, Option<&Type>) {
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
                    Some(current_ty), // Return the identified inner type
                );
            }
        }
    }

    (false, false, is_option, is_vec, None) // Not an Element or DecimalElement type we handle specially
}

// Keep this in sync with generate_primitive_type in fhir_gen/src/lib.rs
fn extract_inner_element_type(type_name: &str) -> &str {
    match type_name {
        "Boolean" => "bool",
        "Integer" | "PositiveInt" | "UnsignedInt" => "std::primitive::i32",
        "Decimal" => "rust_decimal::Decimal", // Use the actual Decimal type
        "Integer64" => "std::primitive::i64",
        "String" | "Code" | "Base64Binary" | "Canonical" | "Id" | "Oid" | "Uri" | "Url"
        | "Uuid" | "Markdown" | "Xhtml" | "Date" | "DateTime" | "Instant" | "Time" => {
            "std::string::String"
        }
        _ => "std::string::String", // Default or consider panic/error
    }
}

fn generate_serialize_impl(data: &Data, name: &Ident) -> proc_macro2::TokenStream {
    match *data {
        Data::Struct(ref data) => {
            match data.fields {
                Fields::Named(ref fields) => {
                    // Check if any fields have the flatten attribute - define this at the top level
                    let has_flattened_fields = fields.named.iter().any(is_flattened);

                    let mut field_serializers = Vec::new();
                    let mut field_counts = Vec::new();
                    for field in fields.named.iter() {
                        let field_name_ident = field.ident.as_ref().unwrap(); // Keep original ident for access
                        let field_ty = &field.ty;
                        let effective_field_name_str = get_effective_field_name(field);
                        let underscore_field_name_str = format!("_{}", effective_field_name_str);

                        // Destructure all 5 return values from get_element_info
                        // We need is_element, is_decimal_element, is_option, is_vec here
                        let (is_element, is_decimal_element, is_option, is_vec, _inner_ty) =
                            get_element_info(field_ty);

                        // Determine if it's an FHIR element type we need to handle specially
                        let is_fhir_element = is_element || is_decimal_element;

                        // Use field_name_ident for accessing the struct field
                        let field_access = quote! { self.#field_name_ident };

                        let extension_field_ident =
                            format_ident!("is_{}_extension", field_name_ident);

                        // Check if field has flatten attribute
                        let field_is_flattened = is_flattened(field);

                        let field_counting_code = if field_is_flattened {
                            // For flattened fields, we don't increment the count
                            // as they will be flattened into the parent object
                            quote! {
                                // No count increment for flattened fields
                                #[allow(unused_variables)]
                                let mut #extension_field_ident = false;
                            }
                        } else if is_option && !is_vec && is_fhir_element {
                            quote! {
                                let mut #extension_field_ident = false;
                                if let Some(field) = &#field_access {
                                    if field.value.is_some() {
                                        count += 1;
                                    }
                                    if field.id.is_some() || field.extension.is_some() {
                                        count += 1;
                                        #extension_field_ident = true;
                                    }
                                }
                            }
                        } else {
                            if field_is_flattened {
                                // For flattened fields, we don't increment the count
                                quote! {
                                    // No count increment for flattened fields
                                    #[allow(unused_variables)]
                                    let mut #extension_field_ident = false;
                                }
                            } else if !is_vec && is_fhir_element {
                                quote! {
                                    let mut #extension_field_ident = false;
                                    if #field_access.value.is_some() {
                                        count += 1;
                                    }
                                    if #field_access.id.is_some() || #field_access.extension.is_some() {
                                        count += 1;
                                        #extension_field_ident = true;
                                    }
                                }
                            } else {
                                // Only count non-Option fields or Some Option fields
                                if is_option {
                                    quote! {
                                        if #field_access.is_some() {
                                            count += 1;
                                        }
                                    }
                                } else {
                                    quote! {
                                        count += 1;
                                    }
                                }
                            }
                        };

                        // Check if field has flatten attribute
                        let field_is_flattened = is_flattened(field);

                        let field_serializing_code = if field_is_flattened {
                            // For flattened fields, use FlatMapSerializer
                            quote! {
                                // Use serde::Serialize::serialize with FlatMapSerializer
                                serde::Serialize::serialize(
                                    &#field_access,
                                    serde::__private::ser::FlatMapSerializer(&mut state)
                                )?;
                            }
                        } else if is_vec && is_fhir_element {
                            // Handles Vec<Element> or Option<Vec<Element>>
                            // Determine how to access the vector based on whether it's wrapped in Option
                            let vec_access = if is_option {
                                quote! { #field_access.as_ref() } // Access Option<Vec<T>> as Option<&Vec<T>>
                            } else {
                                quote! { Some(&#field_access) } // Treat Vec<T> as Some(&Vec<T>) for consistent handling
                            };

                            // Determine which serialization method to call (map vs struct)
                            let serialize_call = if has_flattened_fields {
                                quote! { state.serialize_entry }
                            } else {
                                quote! { state.serialize_field }
                            };

                            quote! {
                                // Handle Vec<Element> by splitting into primitive and extension arrays
                                if let Some(vec_value) = #vec_access { // Use the adjusted access logic
                                    if !vec_value.is_empty() {
                                        // Create primitive array
                                        let mut primitive_array = Vec::with_capacity(vec_value.len());
                                        // Create extension array
                                        let mut extension_array = Vec::with_capacity(vec_value.len());
                                        // Track if we need to include _fieldName
                                        let mut has_extensions = false;

                                        // Process each element
                                        for element in vec_value.iter() {
                                            // Add primitive value or null
                                            match &element.value {
                                                Some(value) => {
                                                    match serde_json::to_value(value) {
                                                        Ok(json_val) => primitive_array.push(json_val),
                                                        Err(e) => return Err(serde::ser::Error::custom(format!("Failed to serialize primitive value: {}", e))),
                                                    }
                                                },
                                                None => primitive_array.push(serde_json::Value::Null),
                                            }

                                            // Check if this element has id or extension
                                            if element.id.is_some() || element.extension.is_some() {
                                                has_extensions = true;
                                                // Use helper struct for consistent serialization of id/extension
                                                #[derive(serde::Serialize)]
                                                struct IdAndExtensionHelper<'a> {
                                                    #[serde(skip_serializing_if = "Option::is_none")]
                                                    id: &'a Option<std::string::String>,
                                                    #[serde(skip_serializing_if = "Option::is_none")]
                                                    extension: &'a Option<Vec<Extension>>,
                                                }
                                                let extension_part = IdAndExtensionHelper {
                                                    id: &element.id,
                                                    extension: &element.extension,
                                                };
                                                // Serialize the helper and push null if it serializes to null (e.g., both fields are None)
                                                match serde_json::to_value(&extension_part) {
                                                    Ok(json_val) if !json_val.is_null() => extension_array.push(json_val),
                                                    Ok(_) => extension_array.push(serde_json::Value::Null), // Push null if helper serialized to null
                                                    Err(e) => return Err(serde::ser::Error::custom(format!("Failed to serialize extension part: {}", e))),
                                                }
                                            } else {
                                                // No id or extension
                                                extension_array.push(serde_json::Value::Null);
                                            }
                                        }

                                        // Serialize primitive array using the correct method (entry or field)
                                        #serialize_call(&#effective_field_name_str, &primitive_array)?;

                                        // Serialize extension array if needed, using the correct method
                                        if has_extensions {
                                            // Use the existing underscore_field_name_str variable which lives longer
                                            #serialize_call(&#underscore_field_name_str, &extension_array)?;
                                        }
                                    }
                                }
                            }
                        } else if is_option && !is_vec && is_fhir_element {
                            // Handles Option<Element> (but not Vec)
                            if has_flattened_fields {
                                // For SerializeMap
                                quote! {
                                    if let Some(field) = &#field_access {
                                        if let Some(value) = field.value.as_ref() {
                                            // Use serialize_entry for SerializeMap
                                            state.serialize_entry(&#effective_field_name_str, value)?;
                                        }
                                        if #extension_field_ident {
                                            #[derive(serde::Serialize)]
                                            struct IdAndExtensionHelper<'a> {
                                                #[serde(skip_serializing_if = "Option::is_none")]
                                                id: &'a Option<std::string::String>,
                                                #[serde(skip_serializing_if = "Option::is_none")]
                                                extension: &'a Option<Vec<Extension>>,
                                            }
                                            let extension_part = IdAndExtensionHelper {
                                                id: &field.id,
                                                extension: &field.extension,
                                            };
                                            // Use serialize_entry for SerializeMap
                                            // No format! here, #underscore_field_name_str is already a string literal
                                            state.serialize_entry(&#underscore_field_name_str, &extension_part)?;
                                        }
                                    }
                                }
                            } else {
                                // For SerializeStruct
                                quote! {
                                    if let Some(field) = &#field_access {
                                        if let Some(value) = field.value.as_ref() {
                                            // Use serialize_field for SerializeStruct
                                            state.serialize_field(&#effective_field_name_str, value)?;
                                        }
                                        if #extension_field_ident {
                                            #[derive(serde::Serialize)]
                                            struct IdAndExtensionHelper<'a> {
                                                #[serde(skip_serializing_if = "Option::is_none")]
                                                id: &'a Option<std::string::String>,
                                                #[serde(skip_serializing_if = "Option::is_none")]
                                                extension: &'a Option<Vec<Extension>>,
                                            }
                                            let extension_part = IdAndExtensionHelper {
                                                id: &field.id,
                                                extension: &field.extension,
                                            };
                                            // Use serialize_field for SerializeStruct
                                            // No format! here, #underscore_field_name_str is already a string literal
                                            state.serialize_field(&#underscore_field_name_str, &extension_part)?;
                                        }
                                    }
                                }
                            }
                        } else {
                            if field_is_flattened {
                                // For flattened fields, use FlatMapSerializer
                                quote! {
                                    // Use serde::Serialize::serialize with FlatMapSerializer
                                    serde::Serialize::serialize(
                                        &#field_access,
                                        serde::__private::ser::FlatMapSerializer(&mut state)
                                    )?;
                                }
                            } else if !is_vec && is_fhir_element {
                                if has_flattened_fields {
                                    // For SerializeMap
                                    quote! {
                                        if let Some(value) = #field_access.value.as_ref() {
                                            // Use serialize_entry for SerializeMap
                                            state.serialize_entry(&#effective_field_name_str, value)?;
                                        }
                                        if #extension_field_ident {
                                            #[derive(serde::Serialize)]
                                            struct IdAndExtensionHelper<'a> {
                                                #[serde(skip_serializing_if = "Option::is_none")]
                                                id: &'a Option<std::string::String>,
                                                #[serde(skip_serializing_if = "Option::is_none")]
                                                extension: &'a Option<Vec<Extension>>,
                                            }
                                            let extension_part = IdAndExtensionHelper {
                                                id: &#field_access.id,
                                                extension: &#field_access.extension,
                                            };
                                            // Use serialize_entry for SerializeMap
                                            state.serialize_entry(#underscore_field_name_str, &extension_part)?;
                                        }
                                    }
                                } else {
                                    // For SerializeStruct
                                    quote! {
                                        if let Some(value) = #field_access.value.as_ref() {
                                            // Use serialize_field for SerializeStruct
                                            state.serialize_field(&#effective_field_name_str, value)?;
                                        }
                                        if #extension_field_ident {
                                            #[derive(serde::Serialize)]
                                            struct IdAndExtensionHelper<'a> {
                                                #[serde(skip_serializing_if = "Option::is_none")]
                                                id: &'a Option<std::string::String>,
                                                #[serde(skip_serializing_if = "Option::is_none")]
                                                extension: &'a Option<Vec<Extension>>,
                                            }
                                            let extension_part = IdAndExtensionHelper {
                                                id: &#field_access.id,
                                                extension: &#field_access.extension,
                                            };
                                            // Use serialize_field for SerializeStruct
                                            // No format! here, #underscore_field_name_str is already a string literal
                                            state.serialize_field(&#underscore_field_name_str, &extension_part)?;
                                        }
                                    }
                                }
                            } else if is_option {
                                // Skip serializing if the Option is None
                                if has_flattened_fields {
                                    // For SerializeMap
                                    quote! {
                                        if let Some(value) = &#field_access {
                                            // Use serialize_entry for SerializeMap
                                            state.serialize_entry(&#effective_field_name_str, value)?;
                                        }
                                    }
                                } else {
                                    // For SerializeStruct
                                    quote! {
                                        if let Some(value) = &#field_access {
                                            // Use serialize_field for SerializeStruct
                                            state.serialize_field(&#effective_field_name_str, value)?;
                                        }
                                    }
                                }
                            } else if is_vec {
                                // Regular Vec handling (not Element)
                                if has_flattened_fields {
                                    // For SerializeMap
                                    quote! {
                                        // Use serde_json to check if the field serializes to null or empty object
                                        let json_value = serde_json::to_value(&#field_access).map_err(|_| serde::ser::Error::custom("serialization failed"))?;
                                        if !json_value.is_null() && !(json_value.is_object() && json_value.as_object().unwrap().is_empty()) {
                                            // Use serialize_entry for SerializeMap
                                            state.serialize_entry(&#effective_field_name_str, &#field_access)?;
                                        }
                                    }
                                } else {
                                    // For SerializeStruct
                                    quote! {
                                        // Use serde_json to check if the field serializes to null or empty object
                                        let json_value = serde_json::to_value(&#field_access).map_err(|_| serde::ser::Error::custom("serialization failed"))?;
                                        if !json_value.is_null() && !(json_value.is_object() && json_value.as_object().unwrap().is_empty()) {
                                            // Use serialize_field for SerializeStruct
                                            state.serialize_field(&#effective_field_name_str, &#field_access)?;
                                        }
                                    }
                                }
                            } else {
                                // For non-Option types, check if it's a struct with all None/null fields
                                if has_flattened_fields {
                                    // For SerializeMap
                                    quote! {
                                        // Use serde_json to check if the field serializes to null or empty object
                                        let json_value = serde_json::to_value(&#field_access).map_err(|_| serde::ser::Error::custom("serialization failed"))?;
                                        if !json_value.is_null() && !(json_value.is_object() && json_value.as_object().unwrap().is_empty()) {
                                            // Use serialize_entry for SerializeMap
                                            state.serialize_entry(&#effective_field_name_str, &#field_access)?;
                                        }
                                    }
                                } else {
                                    // For SerializeStruct
                                    quote! {
                                        // Use serde_json to check if the field serializes to null or empty object
                                        let json_value = serde_json::to_value(&#field_access).map_err(|_| serde::ser::Error::custom("serialization failed"))?;
                                        if !json_value.is_null() && !(json_value.is_object() && json_value.as_object().unwrap().is_empty()) {
                                            // Use serialize_field for SerializeStruct
                                            state.serialize_field(&#effective_field_name_str, &#field_access)?;
                                        }
                                    }
                                }
                            }
                        };

                        field_counts.push(field_counting_code);
                        field_serializers.push(field_serializing_code);
                    }
                    // Use the has_flattened_fields variable defined at the top of the function
                    if has_flattened_fields {
                        // If we have flattened fields, use serialize_map instead of serialize_struct
                        quote! {
                            let mut count = 0;
                            #(#field_counts)*
                            use serde::ser::SerializeMap; // Import trait for map methods
                            let mut state = serializer.serialize_map(Some(count))?;
                            #(#field_serializers)*
                            state.end()
                        }
                    } else {
                        // If no flattened fields, use serialize_struct as before
                        quote! {
                            let mut count = 0;
                            #(#field_counts)*
                            use serde::ser::SerializeStruct; // Import trait for struct methods
                            let mut state = serializer.serialize_struct(stringify!(#name), count)?;
                            #(#field_serializers)*
                            state.end()
                        }
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
    use quote::ToTokens;
    use syn::{Type, parse_str}; // Import ToTokens trait

    // Helper to compare Option<&Type> by converting to string
    fn type_option_to_string(ty_opt: Option<&Type>) -> Option<String> {
        ty_opt.map(|ty| ty.to_token_stream().to_string())
    }

    #[test]
    fn test_get_element_info_option_element() {
        let ty: Type = parse_str("Option<Element<Markdown, Extension>>").unwrap();
        let (is_element, is_decimal, is_option, is_vec, inner_ty) = get_element_info(&ty);
        assert!(is_element);
        assert!(!is_decimal);
        assert!(is_option);
        assert!(!is_vec);
        assert_eq!(
            type_option_to_string(inner_ty),
            Some("Element < Markdown , Extension >".to_string())
        );
    }

    #[test]
    fn test_get_element_info_option_decimal_element() {
        let ty: Type = parse_str("Option<DecimalElement<Extension>>").unwrap();
        let (is_element, is_decimal, is_option, is_vec, inner_ty) = get_element_info(&ty);
        assert!(!is_element);
        assert!(is_decimal);
        assert!(is_option);
        assert!(!is_vec);
        assert_eq!(
            type_option_to_string(inner_ty),
            Some("DecimalElement < Extension >".to_string())
        );
    }

    #[test]
    fn test_get_element_info_option_markdown() {
        let ty: Type = parse_str("Option<Markdown>").unwrap();
        let (is_element, is_decimal, is_option, is_vec, inner_ty) = get_element_info(&ty);
        assert!(is_element); // Markdown should be identified as Element
        assert!(!is_decimal);
        assert!(is_option); // It is an Option
        assert!(!is_vec);
        // For aliases, inner_ty should be Some(alias_type)
        assert_eq!(
            type_option_to_string(inner_ty),
            Some("Markdown".to_string())
        );
    }

    #[test]
    fn test_get_element_info_option_vec_option_element() {
        let ty: Type = parse_str("Option<Vec<Option<Element<bool, Extension>>>>").unwrap();
        let (is_element, is_decimal, is_option, is_vec, inner_ty) = get_element_info(&ty);
        assert!(is_element);
        assert!(!is_decimal);
        assert!(is_option); // Outer Option
        assert!(is_vec); // Vec is present
        assert_eq!(
            type_option_to_string(inner_ty),
            Some("Element < bool , Extension >".to_string())
        );
    }

    #[test]
    fn test_get_element_info_option_vec_option_decimal_element() {
        let ty: Type = parse_str("Option<Vec<Option<DecimalElement<Extension>>>>").unwrap();
        let (is_element, is_decimal, is_option, is_vec, inner_ty) = get_element_info(&ty);
        assert!(!is_element);
        assert!(is_decimal);
        assert!(is_option); // Outer Option
        assert!(is_vec); // Vec is present
        assert_eq!(
            type_option_to_string(inner_ty),
            Some("DecimalElement < Extension >".to_string())
        );
    }

    #[test]
    fn test_get_element_info_option_vec_markdown() {
        let ty: Type = parse_str("Option<Vec<Markdown>>").unwrap();
        let (is_element, is_decimal, is_option, is_vec, inner_ty) = get_element_info(&ty);
        assert!(is_element); // Markdown should be identified as Element
        assert!(!is_decimal);
        assert!(is_option); // Outer Option
        assert!(is_vec); // Vec is present
        // For aliases, inner_ty should be Some(alias_type)
        assert_eq!(
            type_option_to_string(inner_ty),
            Some("Markdown".to_string())
        );
    }

    #[test]
    fn test_get_element_info_element() {
        let ty: Type = parse_str("Element<String, Extension>").unwrap();
        let (is_element, is_decimal, is_option, is_vec, inner_ty) = get_element_info(&ty);
        assert!(is_element);
        assert!(!is_decimal);
        assert!(!is_option);
        assert!(!is_vec);
        assert_eq!(
            type_option_to_string(inner_ty),
            Some("Element < String , Extension >".to_string())
        );
    }

    #[test]
    fn test_get_element_info_decimal_element() {
        let ty: Type = parse_str("DecimalElement<Extension>").unwrap();
        let (is_element, is_decimal, is_option, is_vec, inner_ty) = get_element_info(&ty);
        assert!(!is_element);
        assert!(is_decimal);
        assert!(!is_option);
        assert!(!is_vec);
        assert_eq!(
            type_option_to_string(inner_ty),
            Some("DecimalElement < Extension >".to_string())
        );
    }

    #[test]
    fn test_get_element_info_markdown() {
        let ty: Type = parse_str("Markdown").unwrap();
        let (is_element, is_decimal, is_option, is_vec, inner_ty) = get_element_info(&ty);
        assert!(is_element); // Markdown should be identified as Element
        assert!(!is_decimal);
        assert!(!is_option);
        assert!(!is_vec);
        // For aliases, inner_ty should be Some(alias_type)
        assert_eq!(
            type_option_to_string(inner_ty),
            Some("Markdown".to_string())
        );
    }

    #[test]
    fn test_get_element_info_vec_option_element() {
        // Less common, but test Vec<Option<Element>> without outer Option
        let ty: Type = parse_str("Vec<Option<Element<bool, Extension>>>").unwrap();
        let (is_element, is_decimal, is_option, is_vec, inner_ty) = get_element_info(&ty);
        assert!(is_element);
        assert!(!is_decimal);
        assert!(!is_option); // No outer Option
        assert!(is_vec); // Vec is present
        assert_eq!(
            type_option_to_string(inner_ty),
            Some("Element < bool , Extension >".to_string())
        );
    }

    #[test]
    fn test_get_element_info_vec_option_decimal_element() {
        let ty: Type = parse_str("Vec<Option<DecimalElement<Extension>>>").unwrap();
        let (is_element, is_decimal, is_option, is_vec, inner_ty) = get_element_info(&ty);
        assert!(!is_element);
        assert!(is_decimal);
        assert!(!is_option); // No outer Option
        assert!(is_vec); // Vec is present
        assert_eq!(
            type_option_to_string(inner_ty),
            Some("DecimalElement < Extension >".to_string())
        );
    }

    #[test]
    fn test_get_element_info_vec_string() {
        let ty: Type = parse_str("Vec<String>").unwrap();
        let (is_element, is_decimal, is_option, is_vec, inner_ty) = get_element_info(&ty);
        // String IS identified as Element because it's in KNOWN_ELEMENT_ALIASES
        assert!(is_element);
        assert!(!is_decimal);
        assert!(!is_option);
        assert!(is_vec);
        // For aliases, inner_ty should be Some(alias_type)
        assert_eq!(type_option_to_string(inner_ty), Some("String".to_string()));
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
        assert_eq!(
            type_option_to_string(inner_ty),
            Some("Element < String , Extension >".to_string())
        );
    }

    #[test]
    fn test_get_element_info_option_vec_option_box_element() {
        // Test with Box inside Vec<Option<...>>
        let ty: Type = parse_str("Option<Vec<Option<Box<Element<bool, Extension>>>>>").unwrap();
        let (is_element, is_decimal, is_option, is_vec, inner_ty) = get_element_info(&ty);
        assert!(is_element);
        assert!(!is_decimal);
        assert!(is_option); // Outer Option
        assert!(is_vec); // Vec is present
        // The inner type returned should be the Element itself after unwrapping Box
        assert_eq!(
            type_option_to_string(inner_ty),
            Some("Element < bool , Extension >".to_string())
        );
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
        let (is_element, is_decimal, is_option, is_vec, inner_ty) =
            get_element_info(&ty_non_element_path);
        assert!(!is_element);
        assert!(!is_decimal);
        assert!(!is_option);
        assert!(!is_vec);
        assert!(inner_ty.is_none());

        // Test with a path that *does* end in Element (simulating alias)
        // We use the actual Element type parsed earlier for this simulation
        let (is_element, is_decimal, is_option, is_vec, inner_ty) =
            get_element_info(&ty_simulated_alias);
        assert!(is_element);
        assert!(!is_decimal);
        assert!(!is_option);
        assert!(!is_vec);
        assert_eq!(
            type_option_to_string(inner_ty),
            Some("Element < String , Extension >".to_string())
        );
    }

    #[test]
    fn test_is_flattened() {
        let stream = quote! {
            struct TestStruct {
                #[fhir_serde(flatten)]
                field_a: String,
                field_b: i32,
            }
        };
        let input: DeriveInput = syn::parse2(stream).unwrap();
        if let Data::Struct(data) = input.data {
            if let Fields::Named(fields) = data.fields {
                let field_a = fields
                    .named
                    .iter()
                    .find(|f| f.ident.as_ref().unwrap() == "field_a")
                    .unwrap();
                let field_b = fields
                    .named
                    .iter()
                    .find(|f| f.ident.as_ref().unwrap() == "field_b")
                    .unwrap();
                assert!(is_flattened(field_a));
                assert!(!is_flattened(field_b));
            } else {
                panic!("Expected named fields");
            }
        } else {
            panic!("Expected struct");
        }
    }

    #[test]
    fn test_flatten_serialization() {
        // This test verifies that the flatten attribute is correctly processed
        // by checking the generated code for a struct with a flattened field

        let stream = quote! {
            #[derive(FhirSerde)]
            struct TestWithFlatten {
                regular_field: String,
                #[fhir_serde(flatten)]
                flattened_field: NestedStruct,
            }
        };

        let input: DeriveInput = syn::parse2(stream).unwrap();
        let name = &input.ident;
        let serialize_impl = generate_serialize_impl(&input.data, name);

        // Convert to string to check if FlatMapSerializer is used
        let serialize_impl_str = serialize_impl.to_string();

        // Check that FlatMapSerializer is used for the flattened field
        assert!(serialize_impl_str.contains("FlatMapSerializer"));

        // Check that regular serialization uses serialize_entry when flattening is active (due to serialize_map)
        assert!(serialize_impl_str.contains("serialize_entry"));
    }
}

// Add impl_generics and where_clause as parameters
fn generate_deserialize_impl(data: &Data, name: &Ident) -> proc_macro2::TokenStream {
    let struct_name = format_ident!("Temp{}", name);

    let mut temp_struct_attributes = Vec::new();
    let mut constructor_attributes = Vec::new();

    match *data {
        Data::Struct(ref data) => {
            match data.fields {
                Fields::Named(ref fields) => {
                    for field in fields.named.iter() {
                        let field_name_ident = field.ident.as_ref().unwrap(); // Keep original ident for access
                        let field_name_ident_ext = format_ident!("{}_ext", field_name_ident);
                        let field_ty = &field.ty;
                        let effective_field_name_str = get_effective_field_name(field);
                        let _underscore_field_name_str =
                            format_ident!("_{}", effective_field_name_str);

                        // Destructure all 5 return values, ignoring the inner_ty for now if not needed
                        let (is_element, is_decimal_element, is_option, is_vec, inner_ty) =
                            get_element_info(field_ty);

                        let is_fhir_element = is_element || is_decimal_element;

                        // Determine the type for the primitive value field in the temp struct
                        let temp_primitive_type_quote = if is_fhir_element {
                            let inner_ty_opt = inner_ty; // Use the Option<&Type> from get_element_info
                            let inner_ty =
                                inner_ty_opt.expect("Element type expected but not found");

                            // Determine the base primitive type (e.g., bool, String, rust_decimal::Decimal)
                            let primitive_type_ident = if is_decimal_element {
                                // DecimalElement wraps PreciseDecimal, but the temp struct uses rust_decimal::Decimal
                                quote! { rust_decimal::Decimal }
                            } else {
                                // is_element is true here
                                if let Type::Path(type_path) = inner_ty {
                                    if let Some(last_segment) = type_path.path.segments.last() {
                                        if last_segment.ident == "Element" {
                                            // Direct Element<V, E>
                                            if let PathArguments::AngleBracketed(generics) =
                                                &last_segment.arguments
                                            {
                                                if let Some(GenericArgument::Type(inner_v_type)) =
                                                    generics.args.first()
                                                {
                                                    quote! { #inner_v_type } // Use the inner type V directly
                                                } else {
                                                    panic!("Element missing generic argument V");
                                                }
                                            } else {
                                                panic!("Element missing angle bracketed arguments");
                                            }
                                        } else {
                                            // It's an alias like 'Code'. Get the primitive type it wraps.
                                            let alias_name = last_segment.ident.to_string();
                                            let primitive_type_str =
                                                extract_inner_element_type(&alias_name);
                                            // Parse the primitive type string back into a Type for quoting
                                            let primitive_type_parsed: Type = syn::parse_str(
                                                primitive_type_str,
                                            )
                                            .expect(&format!(
                                                "Failed to parse primitive type string: {}",
                                                primitive_type_str
                                            ));
                                            quote! { #primitive_type_parsed } // Use the parsed primitive type
                                        }
                                    } else {
                                        panic!("Could not get last segment of Element type path");
                                    }
                                } else {
                                    panic!("Element type is not a Type::Path");
                                }
                            }; // End of let primitive_type_ident assignment

                            // Adjust the quote based on whether it's a vector
                            if is_vec {
                                // Temp type should be Option<Vec<Primitive>> regardless of original Option<Vec<T>> or Vec<T>
                                // Serde handles missing array as None with #[serde(default)]
                                quote! { Option<Vec<#primitive_type_ident>> } // No Option inside Vec
                            } else {
                                // If original field was Option<T>, temp type is Option<Primitive>
                                // If original field was T, temp type is Primitive
                                if is_option {
                                    quote! { Option<#primitive_type_ident> }
                                } else {
                                    quote! { #primitive_type_ident } // No Option for non-optional fields
                                }
                            }
                        } else {
                            // Not an element, use the original type
                            quote! { #field_ty }
                        };

                        // Determine the type for the extension helper field in the temp struct
                        let temp_extension_type = if is_fhir_element {
                            if is_vec {
                                // For Vec<Element> or Option<Vec<Element>>, temp type is Option<Vec<Option<IdAndExtensionHelper>>>
                                quote! { Option<Vec<Option<IdAndExtensionHelper>>> }
                            } else {
                                // For Element or Option<Element>, temp type is Option<IdAndExtensionHelper>
                                quote! { Option<IdAndExtensionHelper> }
                            }
                        } else {
                            // Not an element, no extension helper needed
                            quote! { () } // Use unit type as placeholder, won't be generated anyway
                        };

                        // Create the string literal for the underscore field name
                        let underscore_field_name_literal =
                            format!("_{}", effective_field_name_str);

                        // Base attribute for the regular field (primitive value)
                        let base_attribute = quote! {
                            // Use default for Option types in the temp struct
                            #[serde(default, rename = #effective_field_name_str)]
                            #field_name_ident: #temp_primitive_type_quote, // Use the determined Option<V> or original type
                        };

                        // Conditionally add the underscore field attribute if it's an element type
                        let underscore_attribute = if is_fhir_element {
                            quote! {
                                // Use default for Option types in the temp struct
                                #[serde(default, rename = #underscore_field_name_literal)]
                                #field_name_ident_ext: #temp_extension_type,
                            }
                        } else {
                            quote! {} // Empty if not an element
                        };

                        // Combine the attributes for the temp struct
                        let flatten_attr = if is_flattened(field) {
                            quote! { #[serde(flatten)] }
                        } else {
                            quote! {}
                        };
                        let temp_struct_attribute = quote! {
                            #flatten_attr // Add flatten attribute if needed
                            #base_attribute
                            #underscore_attribute
                        };

                        let constructor_attribute = if is_fhir_element {
                            if is_vec {
                                // Handle Vec<Element> or Option<Vec<Element>> first
                                // Generate different construction logic based on whether it's decimal
                                let construction_logic = if is_decimal_element {
                                    // Logic specifically for Vec<DecimalElement> or Option<Vec<DecimalElement>>
                                    let element_type = {
                                        // Determine DecimalElement<E> type
                                        let vec_inner_type = if is_option {
                                            get_option_inner_type(field_ty)
                                        } else {
                                            Some(field_ty)
                                        }
                                        .and_then(get_vec_inner_type)
                                        .expect("Vec inner type not found for DecimalElement");
                                        quote! { #vec_inner_type }
                                    };
                                    quote! { { // Block expression starts
                                        let primitives = temp_struct.#field_name_ident.unwrap_or_default();
                                        let extensions = temp_struct.#field_name_ident_ext.unwrap_or_default();
                                        let len = primitives.len().max(extensions.len());
                                        let mut result_vec = Vec::with_capacity(len);
                                        for i in 0..len {
                                            let prim_val_opt = primitives.get(i).cloned(); // Remove .flatten()
                                            let ext_helper_opt = extensions.get(i).cloned().flatten(); // Keep flatten here
                                            if prim_val_opt.is_some() || ext_helper_opt.is_some() {
                                                let precise_decimal_value = match prim_val_opt {
                                                    Some(dec_val) => {
                                                        // This explicit typing is needed here
                                                        let dec: rust_decimal::Decimal = dec_val;
                                                        Some(crate::PreciseDecimal::from(dec))
                                                    },
                                                    None => None,
                                                };
                                                result_vec.push(#element_type {
                                                    value: precise_decimal_value,
                                                    id: ext_helper_opt.as_ref().and_then(|h| h.id.clone()),
                                                    extension: ext_helper_opt.as_ref().and_then(|h| h.extension.clone()),
                                                });
                                            }
                                            // Note: Skipping adding element if both parts are null/None
                                        }
                                        result_vec
                                    } } // Block expression ends
                                } else {
                                    // Logic specifically for Vec<Element<V, E>> or Option<Vec<Element<V, E>>> (non-decimal)
                                    let element_type = {
                                        // Determine Element<V, E> type
                                        let vec_inner_type = if is_option {
                                            get_option_inner_type(field_ty)
                                        } else {
                                            Some(field_ty)
                                        }
                                        .and_then(get_vec_inner_type)
                                        .expect("Vec inner type not found for Element");
                                        quote! { #vec_inner_type }
                                    };
                                    quote! { { // Block expression starts
                                        let primitives = temp_struct.#field_name_ident.unwrap_or_default();
                                        let extensions = temp_struct.#field_name_ident_ext.unwrap_or_default();
                                        let len = primitives.len().max(extensions.len());
                                        let mut result_vec = Vec::with_capacity(len);
                                        for i in 0..len {
                                            let prim_val_opt = primitives.get(i).cloned(); // Remove .flatten()
                                            let ext_helper_opt = extensions.get(i).cloned().flatten(); // Keep flatten here
                                            if prim_val_opt.is_some() || ext_helper_opt.is_some() {
                                                result_vec.push(#element_type {
                                                    value: prim_val_opt, // Assign Option<V> directly
                                                    id: ext_helper_opt.as_ref().and_then(|h| h.id.clone()),
                                                    extension: ext_helper_opt.as_ref().and_then(|h| h.extension.clone()),
                                                });
                                            }
                                            // Note: Skipping adding element if both parts are null/None
                                        }
                                        result_vec
                                    } } // Block expression ends
                                }; // End of outer if/else determining construction_logic

                                // Assign the correct construction_logic based on is_option
                                if is_option {
                                    // For Option<Vec<Element>>, only construct Some if input fields were present
                                    quote! {
                                        #field_name_ident: if temp_struct.#field_name_ident.is_some() || temp_struct.#field_name_ident_ext.is_some() {
                                            Some(#construction_logic)
                                        } else {
                                            None
                                        },
                                    }
                                } else {
                                    // Direct Vec<Element> field assignment (always construct the Vec)
                                    quote! {
                                        #field_name_ident: #construction_logic,
                                    }
                                }
                            } else if is_decimal_element {
                                // Handle single DecimalElement or Option<DecimalElement>
                                if is_option {
                                    // Logic for Option<DecimalElement>
                                    let construction_logic = quote! { { // Block expression
                                        crate::DecimalElement {
                                            // temp_struct field is Option<Decimal>, map it
                                            value: temp_struct.#field_name_ident.map(|dec| <crate::PreciseDecimal>::from(dec)),
                                            id: temp_struct.#field_name_ident_ext.as_ref().and_then(|h| h.id.clone()),
                                            extension: temp_struct.#field_name_ident_ext.as_ref().and_then(|h| h.extension.clone()),
                                        }
                                    } }; // End block expression
                                    // Wrap in Some() only if value or extension exists
                                    quote! {
                                         #field_name_ident: if temp_struct.#field_name_ident.is_some() || temp_struct.#field_name_ident_ext.is_some() {
                                             Some(#construction_logic)
                                         } else {
                                             None
                                         },
                                    }
                                } else {
                                    // Logic for non-optional DecimalElement
                                    quote! {
                                        #field_name_ident: { // Block expression
                                            crate::DecimalElement {
                                                // temp_struct field is Decimal, convert and wrap in Some()
                                                value: Some(<crate::PreciseDecimal>::from(temp_struct.#field_name_ident)),
                                                id: temp_struct.#field_name_ident_ext.as_ref().and_then(|h| h.id.clone()),
                                                extension: temp_struct.#field_name_ident_ext.as_ref().and_then(|h| h.extension.clone()),
                                            }
                                        }, // End block expression
                                    }
                                }
                            } else if is_option {
                                // Handle single Option<Element> (already checked !is_vec)
                                // Revert to simpler logic without explicit type annotation for value
                                // Get the inner type T from Option<T> to construct Element<V, E>
                                let inner_element_type = get_option_inner_type(field_ty)
                                    .expect("Option inner type not found");
                                quote! {
                                    #field_name_ident: if temp_struct.#field_name_ident.is_some() || temp_struct.#field_name_ident_ext.is_some() {
                                        Some(#inner_element_type { // Use the unwrapped Element type
                                            value: temp_struct.#field_name_ident, // Assign directly
                                            id: temp_struct.#field_name_ident_ext.as_ref().and_then(|h| h.id.clone()),
                                            extension: temp_struct.#field_name_ident_ext.as_ref().and_then(|h| h.extension.clone()),
                                        })
                                    } else {
                                        None // Assign None if neither value nor extension part exists
                                    },
                                }
                            } else {
                                // Handles Element<V, E> (non-option, non-vec)
                                // Assign the Option<Primitive> from temp_struct directly
                                quote! {
                                    #field_name_ident: #field_ty { // Use the original Element type here (e.g., Code)
                                        value: temp_struct.#field_name_ident, // Assign Option<Primitive> directly
                                        id: temp_struct.#field_name_ident_ext.as_ref().and_then(|h| h.id.clone()),
                                        extension: temp_struct.#field_name_ident_ext.as_ref().and_then(|h| h.extension.clone()),
                                    },
                                }
                            }
                        } else {
                            // Not an FHIR element type
                            quote! {
                                #field_name_ident: temp_struct.#field_name_ident,
                            }
                        }; // Semicolon ends the let constructor_attribute binding

                        temp_struct_attributes.push(temp_struct_attribute);
                        constructor_attributes.push(constructor_attribute); // Push the result
                    }
                }
                Fields::Unnamed(_) => panic!("Tuple structs not supported by FhirSerde"),
                Fields::Unit => panic!("Unit structs not supported by FhirSerde"),
            }
        }
        Data::Enum(_) | Data::Union(_) => panic!("Enums and Unions not supported by FhirSerde"),
    }

    let id_extension_helper_def = quote! {
        // Helper struct for deserializing the id/extension part from _fieldName
        #[derive(Clone, Deserialize, Default)] // Add Default derive
        struct IdAndExtensionHelper {
            #[serde(skip_serializing_if = "Option::is_none")] // Change from default
            id: Option<std::string::String>,
            #[serde(skip_serializing_if = "Option::is_none")] // Change from default
            extension: Option<Vec<Extension>>,
        }
    };

    let temp_struct = quote! {
        #[derive(Deserialize)]
        struct #struct_name {
            #(#temp_struct_attributes)*
        }
    };

    quote! {

        #id_extension_helper_def

        #temp_struct

         // Perform the actual deserialization into the temporary struct
        let temp_struct = #struct_name::deserialize(deserializer)?;


        Ok(#name{#(#constructor_attributes)*})

    }
}
