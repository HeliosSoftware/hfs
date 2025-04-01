extern crate proc_macro;

use proc_macro::TokenStream;
// Removed unused Span
use quote::{format_ident, quote}; // Removed unused ToTokens
use syn::{
    parse_macro_input, Data, DeriveInput, Fields, GenericArgument, Ident, Path, PathArguments, Type, TypePath, Attribute, Meta, PathSegment // Added Attribute, Meta, PathSegment
    // Removed unused Field, TypePtr, TypeReference, parse_quote, spanned::Spanned
};
use heck::ToPascalCase; // For generating visitor names

#[proc_macro_derive(FhirSerde)]
pub fn fhir_serde_derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident;
    let generics = input.generics;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let serialize_impl = generate_serialize_impl(&input.data, &name);
    // Pass ty_generics to deserialize generator
    let deserialize_impl = generate_deserialize_impl(&input.data, &name, &ty_generics);

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
                        let field_name = field.ident.as_ref().unwrap();
                        let field_name_str = field_name.to_string();
                        let field_ty = &field.ty;

                        // Function to find and extract the path from skip_serializing_if
                        fn get_skip_serializing_if_path(attrs: &[Attribute]) -> Option<Path> {
                            attrs.iter().find_map(|attr| {
                                if attr.path().is_ident("serde") {
                                    if let Ok(Meta::List(list)) = attr.parse_meta() {
                                        list.nested.iter().find_map(|nested| {
                                            if let syn::NestedMeta::Meta(Meta::NameValue(nv)) = nested {
                                                if nv.path.is_ident("skip_serializing_if") {
                                                    if let syn::Lit::Str(lit_str) = &nv.lit {
                                                        return syn::parse_str::<Path>(&lit_str.value()).ok();
                                                    }
                                                }
                                            }
                                            None
                                        })
                                    } else { None }
                                } else { None }
                            })
                        }


                        let skip_serializing_if_path = get_skip_serializing_if_path(&field.attrs);

                        let (is_element, is_decimal_element, is_option, is_vec, _inner_ty_opt) = get_element_info(field_ty);
                        let is_fhir_element = is_element || is_decimal_element;

                        let field_access = quote! { self.#field_name };

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
                            let underscore_field_name_str = format!("_{}", field_name_str);
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
                            let underscore_field_name_str = format!("_{}", field_name_str);
                            field_serializers.push(quote! {
                                // Check the outer skip condition first
                                if !(#skip_check) {
                                    if let Some(element) = &#field_access {
                                        // Serialize primitive value under fieldName if present
                                        if let Some(value) = &element.value {
                                            // Use the Element's own Serialize impl for the value part? No, serialize the inner value directly.
                                            // state.serialize_field(#field_name_str, value)?;
                                            // Let Element's custom serialize handle the primitive case correctly
                                            let primitive_element = crate::Element { id: None, extension: None, value: element.value.clone() };
                                            state.serialize_field(#field_name_str, &primitive_element)?;

                                        }

                                        // Serialize id/extension under _fieldName if present
                                        if element.id.is_some() || element.extension.is_some() {
                                            // Create a temporary Element containing only id and extension
                                            // Use a concrete type for E if possible, or keep generic if needed. Assuming Extension for now.
                                            let extension_part = crate::Element::<(), crate::r4::Extension> { // Use dummy type for V, concrete Extension
                                                id: element.id.clone(),
                                                extension: element.extension.clone(),
                                                value: None, // Ensure value is None here
                                            };
                                            // Serialize this temporary object, which should output {"id":..., "extension":...}
                                            state.serialize_field(#underscore_field_name_str, &extension_part)?;
                                        }
                                    }
                                     // If Option is Some but element has neither value nor id/extension, nothing is serialized here.
                                     // The count logic should ensure the field count is correct (0 in this case).
                                }
                            });
                        } else if is_fhir_element && is_vec { // Vec<Option<Element>> or Vec<Element>
                            let underscore_field_name_str = format!("_{}", field_name_str);
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
                                            state.serialize_field(#field_name_str, &primitive_values)?;
                                        } else {
                                             // Serialize empty array if input vec is empty but Some and not skipped
                                             state.serialize_field(#field_name_str, &Vec::<Option<()>>::new())?; // Use appropriate dummy type if needed
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
                                            state.serialize_field(#underscore_field_name_str, &extension_values)?;
                                        }
                                    }
                                     // If Option<Vec> is None, the outer skip_check handles it.
                                }
                            });
                        } else {
                            // Default serialization for non-FHIR-element fields
                            field_serializers.push(quote! {
                                if !(#skip_check) {
                                    state.serialize_field(#field_name_str, &#field_access)?;
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


fn generate_deserialize_impl(data: &Data, name: &Ident, ty_generics: &syn::TypeGenerics) -> proc_macro2::TokenStream { // Added ty_generics
    match *data {
        Data::Struct(ref data) => {
            match data.fields {
                Fields::Named(ref fields) => {
                    let struct_name_str = name.to_string();
                    let visitor_name = format_ident!("{}Visitor", name.to_string().to_pascal_case());

                    let field_names: Vec<_> = fields.named.iter().map(|f| f.ident.as_ref().unwrap()).collect();
                    let field_name_strs: Vec<_> = field_names.iter().map(|f| f.to_string()).collect();
                    let field_types: Vec<_> = fields.named.iter().map(|f| &f.ty).collect();

                    // Create enum variants for field matching
                    let field_enum_name = format_ident!("{}Field", name.to_string().to_pascal_case());
                    let field_enum_variants: Vec<_> = field_name_strs.iter().map(|s| format_ident!("{}", s.to_pascal_case())).collect();
                    // Generate underscore variants only for fields that *could* have extensions
                    let mut underscore_field_enum_variants = Vec::new();
                    let mut field_match_arms = Vec::new();
                    let mut is_fhir_element_field = Vec::new(); // Track which fields are FHIR elements

                    for (i, field) in fields.named.iter().enumerate() {
                         let field_name_str = &field_name_strs[i];
                         let variant = &field_enum_variants[i];
                         let field_ty = &field.ty;
                         let (is_element, is_decimal_element, _is_option, _is_vec, _inner_ty_opt) = get_element_info(field_ty);
                         let is_fhir_elem = is_element || is_decimal_element;
                         is_fhir_element_field.push(is_fhir_elem); // Store result

                         field_match_arms.push(quote! { #field_name_str => Ok(#field_enum_name::#variant) });

                         if is_fhir_elem {
                             let underscore_field_name_str = format!("_{}", field_name_str);
                             let underscore_variant = format_ident!("_{}", field_name_str.to_pascal_case());
                             underscore_field_enum_variants.push(underscore_variant.clone()); // Add to list of variants
                             field_match_arms.push(quote! { #underscore_field_name_str => Ok(#field_enum_name::#underscore_variant) });
                         }
                    }
                    let ignore_variant = format_ident!("Ignore");


                    // Generate field storage (using Option for all fields)
                    // let field_storage: Vec<_> = field_types.iter().zip(field_names.iter()).map(|(ty, name)| {
                    //     // Use Option<Type> for storage to track presence
                    //     quote! { #name: Option<#ty> }
                    // }).collect();

                    // Generate visitor map access logic
                    let mut map_access_logic = Vec::new();
                    let mut temp_field_storage = Vec::new(); // For storing Option<serde_json::Value>

                    for (i, field_name) in field_names.iter().enumerate() {
                        let field_name_str = &field_name_strs[i];
                        let variant = &field_enum_variants[i];
                        let temp_field_name = format_ident!("temp_{}", field_name);

                        // Initialize temporary storage for the main field
                        temp_field_storage.push(quote! { let mut #temp_field_name: Option<serde_json::Value> = None; });

                        // Logic to populate temporary storage for the main field
                        map_access_logic.push(quote! {
                            #field_enum_name::#variant => {
                                if #temp_field_name.is_some() {
                                    return Err(serde::de::Error::duplicate_field(#field_name_str));
                                }
                                #temp_field_name = Some(map.next_value()?);
                            }
                        });

                        // If it's a FHIR element, also handle the underscore field
                        if is_fhir_element_field[i] {
                            let underscore_field_name_str = format!("_{}", field_name_str);
                            let underscore_variant = format_ident!("_{}", field_name_str.to_pascal_case());
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
                        let field_name = field.ident.as_ref().unwrap();
                        let field_ty = &field.ty;
                        let temp_field_name = format_ident!("temp_{}", field_name);
                        let is_fhir_elem = is_fhir_element_field[i];

                        final_field_assignments.push(quote! { #field_name }); // Add field name for final struct construction

                        if is_fhir_elem {
                            let underscore_field_name_str = format!("_{}", field_name.to_string());
                            let temp_underscore_field_name = format_ident!("temp_{}", underscore_field_name_str);
                            let (_is_element, _is_decimal_element, is_option, is_vec, _inner_ty_opt) = get_element_info(field_ty);

                            if is_vec { // Handle Vec<Option<Element>> or Vec<Element>
                                final_construction_logic.push(quote! {
                                    let #field_name: #field_ty = {
                                        // Deserialize primitive array (fieldName)
                                        let primitives: Option<Vec<Option<_>>> = #temp_field_name
                                            .map(|v| serde_json::from_value(v))
                                            .transpose()
                                            .map_err(|e| serde::de::Error::custom(format!("Failed to deserialize primitive array for {}: {}", stringify!(#field_name), e)))?;

                                        // Deserialize extension array (_fieldName) - Use concrete Extension type
                                        let extensions: Option<Vec<Option<crate::Element<(), crate::r4::Extension>>>> = #temp_underscore_field_name
                                            .map(|v| serde_json::from_value(v))
                                            .transpose()
                                            .map_err(|e| serde::de::Error::custom(format!("Failed to deserialize extension array for {}: {}", stringify!(#field_name), e)))?;

                                        // Combine logic
                                        match (primitives, extensions) {
                                            (Some(p_vec), Some(e_vec)) => {
                                                if p_vec.len() != e_vec.len() {
                                                    return Err(serde::de::Error::custom(format!("Array length mismatch for field '{}' ({} vs {})", stringify!(#field_name), p_vec.len(), e_vec.len())));
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
                                    let #field_name: #field_ty = {
                                        // Deserialize value part (fieldName)
                                        // The target type for value_part depends on the Element's V type.
                                        // We need to deserialize into Option<V> where Element<V, E>
                                        // This requires knowing V. Let's assume we can deserialize into the Element directly
                                        // and extract the value later if needed, or handle primitives specially.
                                        // Let's try deserializing the whole Element from each part if present.

                                        let value_element: Option<#field_ty> = #temp_field_name
                                            .map(|v| serde_json::from_value(v))
                                            .transpose()
                                            .map_err(|e| serde::de::Error::custom(format!("Failed to deserialize value for {}: {}", stringify!(#field_name), e)))?;

                                        // Deserialize extension part (_fieldName) - Use concrete Extension type
                                        let extension_element: Option<crate::Element<(), crate::r4::Extension>> = #temp_underscore_field_name
                                            .map(|v| serde_json::from_value(v))
                                            .transpose()
                                            .map_err(|e| serde::de::Error::custom(format!("Failed to deserialize extension for {}: {}", stringify!(#field_name), e)))?;

                                        // Combine logic
                                        match (value_element, extension_element) {
                                            // Both present: merge id/extension from _field into element from main field
                                            (Some(mut ve), Some(ee)) => {
                                                ve.id = ee.id;
                                                ve.extension = ee.extension;
                                                Some(ve)
                                            },
                                            // Only value element present
                                            (Some(ve), None) => Some(ve),
                                            // Only extension element present: create a new element with id/ext but no value
                                            (None, Some(ee)) => {
                                                 // We need to construct the correct Element<V, E> type here.
                                                 // This requires knowing V and E. Let's assume #field_ty is Option<Element<V, E>>
                                                 // and construct Element<V, E> { id, extension, value: None }
                                                 // This might require extracting V and E from field_ty.
                                                 // For now, let's assume the target type #field_ty handles default construction.
                                                 // Or, more simply, deserialize the extension part *into* the target type directly,
                                                 // relying on the Element's Deserialize impl to handle the object format.
                                                 let combined_element: #field_ty = #temp_underscore_field_name
                                                      .map(|v| serde_json::from_value(v))
                                                      .transpose()? // Propagate error
                                                      .flatten(); // Flatten Option<Option<T>> -> Option<T>
                                                 combined_element

                                            },
                                            // Neither present
                                            (None, None) => None,
                                        }
                                    };
                                    // If the field itself is not Option<...>, handle unwrap or default
                                    if !#is_option {
                                         #field_name.unwrap_or_default() // Assuming Default trait
                                    } else {
                                         #field_name
                                    }
                                });
                            }

                        } else {
                            // Default deserialization for non-FHIR-element fields
                            final_construction_logic.push(quote! {
                                // Deserialize directly from the temporary JSON value
                                let #field_name : #field_ty = #temp_field_name
                                    .map(|v| serde_json::from_value(v))
                                    .transpose()? // Propagate potential serde_json::Error
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

                        struct #visitor_name #ty_generics; // Add generics here

                        impl<'de> #impl_generics serde::de::Visitor<'de> for #visitor_name #ty_generics #where_clause { // Add generics and where clause
                            type Value = #name #ty_generics; // Use ty_generics here

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
