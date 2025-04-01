extern crate proc_macro;

// Removed redundant extern crate declarations

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{
    parse_macro_input, punctuated::Punctuated, token, Attribute, Data, DeriveInput, Fields,
    GenericArgument, Ident, Lit, Meta, Path, PathArguments, Type, TypePath, // Removed unused MetaNameValue
};
use heck::{ToPascalCase, ToLowerCamelCase}; // Added ToLowerCamelCase

// Helper function to get the effective field name for serialization/deserialization
// Respects #[serde(rename = "...")] attribute, otherwise defaults to camelCase.
fn get_effective_field_name(field: &syn::Field) -> String {
    for attr in &field.attrs {
        if attr.path().is_ident("serde") {
            if let Ok(list) = attr.parse_args_with(Punctuated::<Meta, token::Comma>::parse_terminated) {
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
    field.ident.as_ref().unwrap().to_string().to_lower_camel_case()
}


#[proc_macro_derive(FhirSerde)]
pub fn fhir_serde_derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident;
    let generics = input.generics;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let serialize_impl = generate_serialize_impl(&input.data, &name);
    // Pass all generic parts to deserialize generator
    let deserialize_impl = generate_deserialize_impl(&input.data, &name, &impl_generics, &ty_generics, &where_clause);

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
    if let Type::Path(TypePath { path: Path { segments, .. }, .. }) = ty {
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
    if let Type::Path(TypePath { path: Path { segments, .. }, .. }) = ty {
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
    if let Type::Path(TypePath { path: Path { segments, .. }, .. }) = ty {
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

// Helper to check if a Type is Element<V, E> or DecimalElement<E>
// Returns (IsElement, IsDecimalElement, IsOption, IsVec, InnerType)
fn get_element_info(field_ty: &Type) -> (bool, bool, bool, bool, Option<&Type>) {
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

    // Check if the (potentially unwrapped) type is Element or DecimalElement
    if let Type::Path(TypePath { path, .. }) = current_ty {
        if let Some(segment) = path.segments.last() {
            let type_name = &segment.ident;
            let is_element = type_name == "Element";
            let is_decimal_element = type_name == "DecimalElement";
            if is_element || is_decimal_element {
                // Return the type *before* potential Box unwrapping if it was an Element
                // This assumes Element<Box<T>> is not a pattern we need to handle specially for _field logic
                // If the original type was Option<Vec<Option<Element<...>>>>, current_ty is Element<...>
                // If the original type was Option<Element<...>>, current_ty is Element<...>
                // If the original type was Element<...>, current_ty is Element<...>
                return (is_element, is_decimal_element, is_option, is_vec, Some(current_ty));
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
                                    match attr.parse_args_with(Punctuated::<Meta, token::Comma>::parse_terminated) {
                                        Ok(args) => {
                                            args.iter().find_map(|meta| {
                                                if let Meta::NameValue(nv) = meta {
                                                    if nv.path.is_ident("skip_serializing_if") {
                                                        // The value is now an Expr, check if it's a Lit::Str
                                                        if let syn::Expr::Lit(expr_lit) = &nv.value {
                                                            if let Lit::Str(lit_str) = &expr_lit.lit {
                                                                return syn::parse_str::<Path>(&lit_str.value()).ok();
                                                            }
                                                        }
                                                    }
                                                }
                                                None // Not the meta item we are looking for
                                            })
                                        },
                                        Err(_) => None, // Failed to parse args, ignore this attribute
                                    }
                                } else { None } // Not a #[serde(...)] attribute
                            })
                        }

                        let skip_serializing_if_path = get_skip_serializing_if_path(&field.attrs);

                        let (is_element, is_decimal_element, is_option, is_vec, _inner_ty_opt) = get_element_info(field_ty);
                        let is_fhir_element = is_element || is_decimal_element;

                        // Use field_name_ident for accessing the struct field
                        let field_access = quote! { self.#field_name_ident };

                        // --- Field Count Calculation ---
                        let base_count_logic = if let Some(condition_path) = &skip_serializing_if_path {
                             // If skip_serializing_if is present, use its condition
                             quote! { !#condition_path(&#field_access) }
                        } else if is_option {
                             // Default for Option is skip if None
                             quote! { #field_access.is_some() }
                        } else {
                             // Always count non-optional fields (unless skipped by attribute handled above)
                             quote! { true }
                        };


                        // Special handling for FHIR elements that might add an extra `_field`
                        if is_fhir_element && !is_vec { // Single Element or DecimalElement
                            // let underscore_field_name_str = format!("_{}", field_name_str); // Not needed for count
                            field_count_calculator.push(quote! {
                                if let Some(element) = &#field_access {
                                    let has_value = element.value.is_some();
                                    let has_extension = element.id.is_some() || element.extension.is_some();
                                    if has_value {
                                        count += 1; // Count the main field
                                    }
                                    if has_extension {
                                        count += 1; // Count the underscore field
                                    }
                                    // If the element exists but has neither value nor extension,
                                    // the Element's serialize impl might output {}, which counts as 1 field.
                                    // However, our Element serialize impl outputs null or primitive if only value,
                                    // and object if id/ext exist. If both are none, it depends on Option.
                                    // Let's refine: if the Option is Some, but the element is empty (no value, no id, no ext),
                                    // the Element serialize outputs {}, so we should count 1.
                                    // But the current Element impl outputs null in that case if id/ext are none.
                                    // Let's stick to counting based on value/extension presence.
                                }
                            });
                        } else if is_fhir_element && is_vec { // Vec<Option<Element>>
                            // let underscore_field_name_str = format!("_{}", effective_field_name_str); // Use effective name if needed
                            field_count_calculator.push(quote! {
                                if let Some(vec) = &#field_access {
                                     // Check if the vector itself should be serialized based on skip_serializing_if or Option rules
                                     if #base_count_logic {
                                        // Check if the primitive array needs serialization (non-empty or skip doesn't apply)
                                        let serialize_primitive_array = !vec.is_empty(); // Always serialize if not empty? Or respect skip? Let's assume serialize if not empty for now.
                                        if serialize_primitive_array {
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
                                }
                            });
                        } else {
                            // Use the standard count logic for non-FHIR elements or non-optional fields
                            field_count_calculator.push(quote! { if #base_count_logic { count += 1; } });
                        }


                        // --- Field Serialization Logic ---
                        let skip_check = if let Some(condition_path) = &skip_serializing_if_path {
                            quote! { #condition_path(&#field_access) }
                        } else if is_option {
                            quote! { #field_access.is_none() }
                        } else {
                            quote! { false } // Don't skip non-optional fields by default
                        };

                        if is_fhir_element && !is_vec { // Single Element or DecimalElement
                            // Use effective_field_name_str here
                            // let underscore_field_name_str = format!("_{}", effective_field_name_str);
                            field_serializers.push(quote! {
                                // Check the outer skip condition first
                                if !(#skip_check) {
                                    if let Some(element) = &#field_access {
                                        let has_value = element.value.is_some();
                                        let has_extension = element.id.is_some() || element.extension.is_some();

                                        let has_value = element.value.is_some();
                                        let has_extension = element.id.is_some() || element.extension.is_some();

                                        let has_value = element.value.is_some();
                                        let has_extension = element.id.is_some() || element.extension.is_some();

                                        if has_value {
                                            // Serialize primitive value under fieldName
                                            state.serialize_field(&#effective_field_name_str, element.value.as_ref().unwrap())?;
                                        }

                                        // Define helper struct locally for serialization (used in cases 2 & 3)
                                        // Assuming E is crate::r4::Extension for now
                                        #[derive(Serialize)]
                                        struct IdAndExtensionHelper<'a> {
                                            #[serde(skip_serializing_if = "Option::is_none")]
                                            id: &'a Option<String>,
                                            #[serde(skip_serializing_if = "Option::is_none")]
                                            extension: &'a Option<Vec<crate::r4::Extension>>, // Use concrete type
                                        }

                                        // Case 1: Value only -> Serialize primitive
                                        if has_value && !has_extension {
                                            state.serialize_field(&#effective_field_name_str, element.value.as_ref().unwrap())?;
                                        }
                                        // Case 2: Extension only -> Serialize helper object under _fieldName
                                        else if !has_value && has_extension {
                                            let underscore_field_name_str = format!("_{}", effective_field_name_str);
                                            let extension_part = IdAndExtensionHelper {
                                                id: &element.id,
                                                extension: &element.extension,
                                            };
                                            state.serialize_field(&underscore_field_name_str, &extension_part)?;
                                        }
                                        // Case 3: Both value and extension -> Serialize both fieldName and _fieldName
                                        else if has_value && has_extension {
                                            // Serialize primitive value under fieldName
                                            state.serialize_field(&#effective_field_name_str, element.value.as_ref().unwrap())?;
                                            // Serialize extension object under _fieldName using helper struct
                                            let underscore_field_name_str = format!("_{}", effective_field_name_str);
                                            let extension_part = IdAndExtensionHelper {
                                                id: &element.id,
                                                extension: &element.extension,
                                            };
                                            state.serialize_field(&underscore_field_name_str, &extension_part)?;
                                        }
                                        // Case 4: Neither value nor extension (but Option is Some) -> Serialize nothing here
                                    }
                                }
                            });
                        } else if is_fhir_element && is_vec { // Vec<Option<Element>> or Vec<Element>
                            // Use effective_field_name_str here if needed for underscore name generation inside
                            // let underscore_field_name_str = format!("_{}", effective_field_name_str);
                            field_serializers.push(quote!{
                                // Check the outer skip condition first
                                if !(#skip_check) {
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
                                if !(#skip_check) {
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
                    let visitor_name = format_ident!("{}Visitor", name.to_string().to_pascal_case());

                    // Remove unused field_name_strs
                    // let field_name_strs: Vec<_> = field_names.iter().map(|f| f.to_string()).collect();

                    // Create enum variants for field matching
                    let field_enum_name = format_ident!("{}Field", name.to_string().to_pascal_case()); // Keep this for the enum name
                    // Helper to get aliases
                    fn get_field_aliases(attrs: &[Attribute]) -> Vec<String> {
                        attrs.iter().flat_map(|attr| -> Vec<String> { // Outer closure returns Vec<String>
                            if attr.path().is_ident("serde") {
                                match attr.parse_args_with(Punctuated::<Meta, token::Comma>::parse_terminated) {
                                    Ok(args) => {
                                        // Inner closure for filter_map returns Option<String>
                                        args.iter().filter_map(|meta| {
                                            if let Meta::NameValue(nv) = meta {
                                                if nv.path.is_ident("alias") {
                                                    if let syn::Expr::Lit(expr_lit) = &nv.value {
                                                        if let Lit::Str(lit_str) = &expr_lit.lit {
                                                            return Some(lit_str.value()); // Correct: filter_map expects Option
                                                        }
                                                    }
                                                }
                                            }
                                            None // Correct: filter_map expects Option
                                        }).collect::<Vec<String>>() // Collects Option<String> into Vec<String>
                                    },
                                    Err(_) => vec![], // Correct: flat_map expects IntoIterator<Item = String>
                                }
                            } else {
                                vec![] // Correct: flat_map expects IntoIterator<Item = String>
                            }
                        }).collect() // Collects Strings from all attributes
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
                         let (is_element, is_decimal_element, _is_option, _is_vec, _inner_ty_opt) = get_element_info(field_ty);
                         let is_fhir_elem = is_element || is_decimal_element;
                         is_fhir_element_field.push(is_fhir_elem); // Store result

                         // Match effective name
                         field_match_arms.push(quote! { #effective_field_name_str => Ok(#field_enum_name::#variant) });

                         // Match aliases
                         let aliases = get_field_aliases(&field.attrs);
                         for alias in aliases {
                             field_match_arms.push(quote! { #alias => Ok(#field_enum_name::#variant) });
                         }

                         if is_fhir_elem {
                             let underscore_field_name_str = format!("_{}", effective_field_name_str);
                             let underscore_variant = format_ident!("_{}", field_ident.to_string().to_pascal_case()); // Underscore variant also based on Rust ident
                             underscore_field_enum_variants.push(underscore_variant.clone()); // Add to list of variants
                             // Match underscore name
                             field_match_arms.push(quote! { #underscore_field_name_str => Ok(#field_enum_name::#underscore_variant) });
                             // Match underscore aliases? (Less common, skip for now)
                         }
                    }
                    let ignore_variant = format_ident!("Ignore");
                    // Extract just the variant names for the enum definition
                    let field_enum_variants: Vec<_> = field_enum_variants_map.values().cloned().collect();


                    // Generate field storage (using Option for all fields)
                    // let field_storage: Vec<_> = field_types.iter().zip(field_names.iter()).map(|(ty, name)| {
                    //     // Use Option<Type> for storage to track presence
                    //     quote! { #name: Option<#ty> }
                    // }).collect();

                    // Generate visitor map access logic
                    let mut map_access_logic = Vec::new();
                    let mut temp_field_storage = Vec::new(); // For storing Option<serde_json::Value>

                    for (i, field) in fields.named.iter().enumerate() { // Iterate over fields again
                        let field_ident = field.ident.as_ref().unwrap();
                        let effective_field_name_str = get_effective_field_name(field); // Use helper
                        let variant = field_enum_variants_map.get(field_ident).unwrap(); // Get variant from map
                        let temp_field_name = format_ident!("temp_{}", field_ident);

                        // Initialize temporary storage for the main field
                        temp_field_storage.push(quote! { let mut #temp_field_name: Option<serde_json::Value> = None; });

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
                            let underscore_field_name_str = format!("_{}", effective_field_name_str); // Use effective name
                            let underscore_variant = format_ident!("_{}", field_ident.to_string().to_pascal_case()); // Variant based on Rust ident
                            let temp_underscore_field_name = format_ident!("temp_{}", underscore_field_name_str);

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

                    // Generate final struct construction logic
                    let mut final_construction_logic = Vec::new();
                    let mut final_field_assignments = Vec::new(); // To build the final struct { field1, field2, ... }

                    for (i, field) in fields.named.iter().enumerate() {
                        let field_ident = field.ident.as_ref().unwrap(); // Use ident for assignment
                        let field_ty = &field.ty;
                        let temp_field_name = format_ident!("temp_{}", field_ident);
                        let is_fhir_elem = is_fhir_element_field[i];

                        final_field_assignments.push(quote! { #field_ident }); // Add field ident for final struct construction

                        if is_fhir_elem {
                            let effective_field_name_str = get_effective_field_name(field); // Use helper
                            let underscore_field_name_str = format!("_{}", effective_field_name_str);
                            let temp_underscore_field_name = format_ident!("temp_{}", underscore_field_name_str);
                            // Remove underscore from is_option as it's needed later
                            let (_is_element, _is_decimal_element, is_option, is_vec, _inner_ty_opt) = get_element_info(field_ty);

                            if is_vec { // Handle Vec<Option<Element>> or Vec<Element>
                                final_construction_logic.push(quote! {
                                    // Use field_ident for the variable name
                                    let #field_ident: #field_ty = {
                                        // Deserialize primitive array (fieldName)
                                        let primitives: Option<Vec<Option<_>>> = #temp_field_name
                                            .map(|v| serde_json::from_value(v).map_err(serde::de::Error::custom)) // Map error here
                                            .transpose()?; // Propagate V::Error

                                        // Deserialize extension array (_fieldName) - Use concrete Extension type
                                        let extensions: Option<Vec<Option<crate::Element<(), crate::r4::Extension>>>> = #temp_underscore_field_name
                                            .map(|v| serde_json::from_value(v).map_err(serde::de::Error::custom)) // Map error here
                                            .transpose()?; // Propagate V::Error

                                        // Combine logic
                                        match (primitives, extensions) {
                                            (Some(p_vec), Some(e_vec)) => {
                                                if p_vec.len() != e_vec.len() {
                                                    // Use field_ident in stringify!
                                                    return Err(serde::de::Error::custom(format!("Array length mismatch for field '{}' ({} vs {})", stringify!(#field_ident), p_vec.len(), e_vec.len())));
                                                }
                                                let mut combined = Vec::with_capacity(p_vec.len());
                                                for (p_opt, e_opt) in p_vec.into_iter().zip(e_vec.into_iter()) {
                                                    // Reconstruct the Option<Element> based on parts
                                                    let element_opt = match (p_opt, e_opt) {
                                                        // Value and Extension present
                                                        (Some(p), Some(e)) => Some(crate::Element { id: e.id, extension: e.extension, value: Some(p) }),
                                                        // Only Value present
                                                        (Some(p), None) => Some(crate::Element { id: None, extension: None, value: Some(p) }),
                                                        // Only Extension present
                                                        (None, Some(e)) => Some(crate::Element { id: e.id, extension: e.extension, value: None }),
                                                        // Both null or absent
                                                        (None, None) => None,
                                                    };
                                                    combined.push(element_opt);
                                                }
                                                Some(combined) // Result is Option<Vec<Option<Element>>>
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
                                            (None, None) => None, // Neither array present
                                        }
                                    };
                                });

                            } else { // Handle single Option<Element> or Element
                                final_construction_logic.push(quote! {
                                    // Use field_ident for the variable name
                                    let #field_ident: #field_ty = {
                                        // Define helper struct locally for id/extension part
                                        #[derive(Deserialize, Debug, Default)] // Add Default
                                        struct IdAndExtension<E: serde::de::DeserializeOwned + Default> { // Add Default bound
                                            id: Option<String>,
                                            extension: Option<Vec<E>>,
                                        }

                                        // Deserialize the primitive value part (fieldName) into Option<V>
                                        // This requires knowing V. We deserialize into serde_json::Value first.
                                        let primitive_value_json: Option<serde_json::Value> = #temp_field_name;

                                        // Deserialize extension part (_fieldName) into Option<IdAndExtension>
                                        let extension_part: Option<IdAndExtension<crate::r4::Extension>> = #temp_underscore_field_name // Assuming r4::Extension
                                            .map(|v| serde_json::from_value(v).map_err(serde::de::Error::custom)) // Map error here
                                            .transpose()?; // Propagate V::Error

                                        // Combine logic: Construct the final Element<V, E> or Option<Element<V, E>>
                                        let result_element_opt = match (primitive_value_json, extension_part) {
                                            // Both primitive value and extension object are present
                                            (Some(prim_json), Some(ep)) => {
                                                // Deserialize prim_json into V. This is the hard part.
                                                // We rely on Element<V,E>::deserialize handling primitive input.
                                                // Let's deserialize into a temporary Element and extract the value.
                                                let temp_element: crate::Element<_, crate::r4::Extension> = serde_json::from_value(prim_json)
                                                    .map_err(|e| serde::de::Error::custom(format!("Failed to deserialize primitive value for {}: {}", stringify!(#field_ident), e)))?;

                                                Some(crate::Element {
                                                    id: ep.id,
                                                    extension: ep.extension,
                                                    value: temp_element.value, // Use the extracted value
                                                })
                                            },
                                            // Only primitive value is present
                                            (Some(prim_json), None) => {
                                                // Deserialize the primitive JSON directly into the Element type.
                                                let element: crate::Element<_, crate::r4::Extension> = serde_json::from_value(prim_json)
                                                     .map_err(|e| serde::de::Error::custom(format!("Failed to deserialize primitive value for {}: {}", stringify!(#field_ident), e)))?;
                                                Some(element)
                                            },
                                            // Only extension object is present
                                            (None, Some(ep)) => {
                                                 // Construct Element with id/extension and value: None
                                                 Some(crate::Element {
                                                     id: ep.id,
                                                     extension: ep.extension,
                                                     value: None, // Explicitly None
                                                 })
                                            },
                                            // Neither field is present
                                            (None, None) => None,
                                        };

                                        // Assign to the final field, handling Option vs non-Option field type
                                        if #is_option {
                                            result_element_opt // Assign Option<Element<V, E>>
                                        } else {
                                            // If the field is not optional, but we got None (e.g., empty JSON object), return Default
                                            // If we constructed Some(element), unwrap it.
                                            result_element_opt.unwrap_or_default() // Assign Element<V, E>, requires Default
                                        }
                                    };
                                     #field_ident // Assign the constructed Element or Option<Element>
                                });
                            }

                        } else {
                            // Default deserialization for non-FHIR-element fields
                            final_construction_logic.push(quote! {
                                // Deserialize directly from the temporary JSON value
                                let #field_ident : #field_ty = #temp_field_name // Use field_ident here
                                    .map(|v| serde_json::from_value(v).map_err(serde::de::Error::custom)) // Map error here
                                    .transpose()? // Propagate V::Error
                                    .unwrap_or_default(); // Use default if field was missing or null, assuming Default trait
                            });
                        }
                    }

                    // Assemble the final struct instantiation
                    let struct_instantiation = quote! {
                        #name {
                            #(#final_field_assignments),* // Use the generated list of field names
                        }
                    };

                    // --- Visitor and Deserialize Implementation ---
                    quote! {
                        #[derive(serde::Deserialize)]
                        #[serde(field_identifier, rename_all = "camelCase")] // Use camelCase for enum variants from JSON field names
                        enum #field_enum_name {
                            #(#field_enum_variants,)*
                            #(#underscore_field_enum_variants,)* // Include underscore variants
                            #[serde(other)] // Handle unknown fields gracefully
                            #ignore_variant
                        }

                        // Define visitor struct with type generics and where clause
                        struct #visitor_name #ty_generics #where_clause;

                        // Apply impl generics and where clause correctly to the impl block
                        impl<'de> #impl_generics serde::de::Visitor<'de> for #visitor_name #ty_generics #where_clause {
                            type Value = #name #ty_generics;

                            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                                formatter.write_str(concat!("struct ", #struct_name_str))
                            }

                            fn visit_map<V>(self, mut map: V) -> Result<Self::Value, V::Error>
                            where
                                V: serde::de::MapAccess<'de>,
                            {
                                #(#temp_field_storage)* // Declare temp storage

                                while let Some(key) = map.next_key()? {
                                    match key {
                                        #(#map_access_logic)*
                                        #field_enum_name::#ignore_variant => {
                                            // Ignore unknown fields
                                            let _ = map.next_value::<serde::de::IgnoredAny>()?;
                                        }
                                    }
                                }

                                // Process temp storage to build final fields
                                #(#final_construction_logic)*

                                // Construct the final struct
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
