use super::common::{
    generate_id_and_extension_helper_owned, get_effective_field_name, is_flattened,
};
use crate::util::{
    extract_inner_element_type, get_base_type, get_element_info, get_option_inner_type,
    get_vec_inner_type,
};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{
    Data, Fields, GenericArgument, Ident, Lit, Meta, PathArguments, Type, punctuated::Punctuated,
    token,
};

/// TokenStream containing the complete `deserialize` method implementation.
pub(crate) fn generate_deserialize_impl(data: &Data, name: &Ident) -> TokenStream {
    let struct_name = format_ident!("Temp{}", name);

    let mut constructor_attributes = Vec::new();
    let deserialize_impl: TokenStream;

    match *data {
        Data::Enum(ref data) => {
            // For enums, we need to deserialize from a map with a single key-value pair
            // where the key is the variant name and the value is the variant data

            // Generate a visitor for the enum
            let enum_name = name.to_string();
            let variants = &data.variants;

            let mut variant_matches = Vec::new(); // Stores the generated match arms
            let mut variant_names = Vec::new(); // Stores the string names for error messages/expecting

            for variant in variants {
                let variant_name = &variant.ident; // The Ident (e.g., String)
                let variant_name_str = variant_name.to_string();

                // Get the rename attribute if present
                let mut rename = None;
                for attr in &variant.attrs {
                    if attr.path().is_ident("fhir_serde") {
                        if let Ok(list) =
                            attr.parse_args_with(Punctuated::<Meta, token::Comma>::parse_terminated)
                        {
                            for meta in list {
                                if let Meta::NameValue(nv) = meta {
                                    if nv.path.is_ident("rename") {
                                        if let syn::Expr::Lit(expr_lit) = nv.value {
                                            if let Lit::Str(lit_str) = expr_lit.lit {
                                                rename = Some(lit_str.value());
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if rename.is_some() {
                        break;
                    }
                }

                // Use the rename value or the variant name as a string for the JSON key
                let variant_key = rename.unwrap_or_else(|| variant_name_str.clone());
                variant_names.push(variant_key.clone()); // Keep track of expected keys

                // Generate the specific deserialization logic for this variant
                let deserialization_logic = match &variant.fields {
                    Fields::Unnamed(fields) if fields.unnamed.len() == 1 => {
                        // Newtype variant (e.g., String(String))
                        let field = fields.unnamed.first().unwrap();
                        let field_ty = &field.ty;
                        let (is_element, is_decimal_element, _, _) = get_element_info(field_ty);

                        if is_element || is_decimal_element {
                            // --- Element/DecimalElement Variant Construction ---
                            let underscore_variant_key_str = format!("_{}", variant_key); // For error messages

                            // Determine the primitive type V or PreciseDecimal for the value field
                            let primitive_type_for_element = if is_decimal_element {
                                quote! { crate::PreciseDecimal }
                            } else {
                                // Extract V from Element<V, E> or the alias's underlying primitive
                                // Need to re-determine the base type here
                                let base_type = get_base_type(field_ty);
                                if let Type::Path(type_path) = base_type {
                                    if let Some(last_segment) = type_path.path.segments.last() {
                                        if last_segment.ident == "Element" {
                                            // Direct Element<V, E>
                                            if let PathArguments::AngleBracketed(generics) =
                                                &last_segment.arguments
                                            {
                                                if let Some(GenericArgument::Type(inner_v_type)) =
                                                    generics.args.first()
                                                {
                                                    quote! { #inner_v_type }
                                                } else {
                                                    panic!("Element missing generic argument V");
                                                }
                                            } else {
                                                panic!("Element missing angle bracketed arguments");
                                            }
                                        } else {
                                            // Alias
                                            let alias_name = last_segment.ident.to_string();
                                            let primitive_type_str =
                                                extract_inner_element_type(&alias_name);
                                            let primitive_type_parsed: Type = syn::parse_str(
                                                primitive_type_str,
                                            )
                                            .expect("Failed to parse primitive type string");
                                            quote! { #primitive_type_parsed }
                                        }
                                    } else {
                                        panic!("Could not get last segment of Element type path");
                                    }
                                } else {
                                    panic!("Element type is not a Type::Path");
                                }
                            };

                            quote! {
                                // Check if parts exist *before* potentially moving them
                                let has_value_part = value_part.is_some();
                                let has_extension_part = extension_part.is_some();

                                // Deserialize the extension part if present
                                let mut ext_helper_opt: Option<IdAndExtensionHelper> = None;
                                if let Some(ext_value) = extension_part { // Move happens here
                                    use serde::de::DeserializeSeed;
                                    let ctx = ::helios_serde::DeserializationContext::<IdAndExtensionHelper, ::helios_serde::Json>::json();
                                    ext_helper_opt = Some(ctx.deserialize(ext_value)
                                        .map_err(|e| serde::de::Error::custom(format!("Error deserializing extension {}: {}", #underscore_variant_key_str, e)))?);
                                }

                                // Deserialize the value part if present, consuming value_part
                                let deserialized_value_opt = if let Some(prim_value) = value_part { // Move of value_part happens here
                                    // Use #primitive_type_for_element determined outside
                                    use serde::de::{DeserializeSeed, IntoDeserializer};
                                    let ctx = ::helios_serde::DeserializationContext::<#primitive_type_for_element, ::helios_serde::Json>::json();
                                    Some(ctx.deserialize(prim_value.into_deserializer())
                                         .map_err(|e| serde::de::Error::custom(format!("Error deserializing primitive {}: {}", #variant_key, e)))?)
                                } else {
                                    None::<#primitive_type_for_element> // Explicit type needed for None
                                };

                                // Construct the element using deserialized parts
                                let mut element: #field_ty = Default::default(); // Start with default

                                // Assign deserialized value
                                element.value = deserialized_value_opt; // Assign the Option<V> or Option<PreciseDecimal>

                                // Merge the extension data if it exists
                                if let Some(ext_helper) = ext_helper_opt {
                                    if ext_helper.id.is_some() {
                                        element.id = ext_helper.id;
                                    }
                                    if ext_helper.extension.is_some() {
                                        element.extension = ext_helper.extension;
                                    }
                                }
                                // Note: The check `if !has_value_part && has_extension_part { element.value = None; }`
                                // is now redundant because element.value is already None if !has_value_part.

                                Ok(#name::#variant_name(element))
                            }
                            // --- End Element/DecimalElement Variant Construction ---
                        } else {
                            // --- Regular Newtype Variant Construction ---
                            quote! {
                                use serde::de::DeserializeSeed;
                                let value = value_part.ok_or_else(|| serde::de::Error::missing_field(#variant_key))?;
                                let ctx = ::helios_serde::DeserializationContext::<#field_ty, ::helios_serde::Json>::json();
                                let inner_value = ctx.deserialize(value)
                                    .map_err(|e| serde::de::Error::custom(format!("Error deserializing non-element variant {}: {}", #variant_key, e)))?;
                                Ok(#name::#variant_name(inner_value)) // Removed .into()
                            }
                            // --- End Regular Newtype Variant Construction ---
                        }
                    }
                    Fields::Unnamed(_) => {
                        // Tuple variant
                        quote! {
                            let value = value_part.ok_or_else(|| serde::de::Error::missing_field(#variant_key))?;
                            let inner_value = serde::Deserialize::deserialize(value)
                                .map_err(|e| serde::de::Error::custom(format!("Error deserializing tuple variant {}: {}", #variant_key, e)))?;
                            Ok(#name::#variant_name(inner_value)) // Use variant_name directly
                        }
                    }
                    Fields::Named(_) => {
                        // Struct variant
                        quote! {
                            let value = value_part.ok_or_else(|| serde::de::Error::missing_field(#variant_key))?;
                            let inner_value = serde::Deserialize::deserialize(value)
                                .map_err(|e| serde::de::Error::custom(format!("Error deserializing struct variant {}: {}", #variant_key, e)))?;
                            Ok(#name::#variant_name(inner_value)) // Use variant_name directly
                        }
                    }
                    Fields::Unit => {
                        // Unit variant
                        quote! {
                            Ok(#name::#variant_name) // Use variant_name directly
                        }
                    }
                }; // End match variant.fields

                // Push the complete match arm
                variant_matches.push(quote! {
                    #variant_key => { // Use the string key as the match pattern
                        #deserialization_logic // Embed the generated logic block
                    }
                });
            } // End loop over variants

            // Define the helper struct needed for enum deserialization
            let id_extension_helper_def = generate_id_and_extension_helper_owned();

            // Generate the enum deserialization implementation
            return quote! {
                // Import necessary crates/modules at the top level of the impl block
                use serde::{Deserialize, de::{self, Visitor, MapAccess}};
                use serde_json; // Needed for Value
                use std::collections::HashSet; // Needed for processed_keys
                // NOTE: Removed `use syn;` as it's not needed at runtime

                // Define the helper struct at the top level of the impl block
                #id_extension_helper_def

                // Define a visitor for the enum (no longer needs variants reference)
                struct EnumVisitor; // Removed lifetime and variants field

                impl<'de> serde::de::Visitor<'de> for EnumVisitor { // Removed lifetime 'a
                    type Value = #name;

                    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                        formatter.write_str(concat!("a ", #enum_name, " enum"))
                    }

                    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
                    where
                        A: serde::de::MapAccess<'de>,
                    {
                        let mut found_variant_key: Option<std::string::String> = None;
                        let mut value_part: Option<serde_json::Value> = None;
                        let mut extension_part: Option<serde_json::Value> = None;
                        let mut processed_keys = std::collections::HashSet::new(); // Track processed keys

                        // Iterate through map entries directly, deserializing key as Value
                        while let Some((key_value, current_value)) = map.next_entry::<serde_json::Value, serde_json::Value>()? {
                            // Ensure the key is a string
                            let key_str = match key_value {
                                serde_json::Value::String(s) => s,
                                _ => return Err(serde::de::Error::invalid_type(serde::de::Unexpected::Other("non-string key"), &"a string key")),
                            };

                            let mut key_matched = false;
                            #( // Loop over variant_names (&'static str)
                                let base_name = #variant_names; // e.g., "authorString"
                                let underscore_name = format!("_{}", base_name); // e.g., "_authorString"

                                if key_str.as_str() == base_name { // Compare &str == &'static str
                                    if value_part.is_some() {
                                        return Err(serde::de::Error::duplicate_field(base_name));
                                    }
                                    value_part = Some(current_value.clone()); // Store the value
                                    // If we already found a key based on the underscore version, ensure it matches
                                    if let Some(ref existing_key) = found_variant_key {
                                        if existing_key != base_name {
                                             // Use key_str.as_str() for formatting
                                             return Err(serde::de::Error::custom(format!("Mismatched keys found: {} and {}", existing_key, key_str.as_str())));
                                        }
                                    } else {
                                        found_variant_key = Some(base_name.to_string());
                                    }
                                    processed_keys.insert(key_str.clone()); // Clone the String key
                                    key_matched = true;
                                } else if key_str.as_str() == underscore_name.as_str() { // Compare &str == &str
                                    if extension_part.is_some() {
                                        // Use custom error message as duplicate_field requires 'static str
                                        return Err(serde::de::Error::custom(format!("duplicate field '{}'", key_str)));
                                    }
                                    extension_part = Some(current_value.clone()); // Store the extension value
                                    // If we already found a key based on the base version, ensure it matches
                                     if let Some(ref existing_key) = found_variant_key {
                                        if existing_key != base_name {
                                             // Use key_str.as_str() for formatting
                                             return Err(serde::de::Error::custom(format!("Mismatched keys found: {} and {}", existing_key, key_str.as_str())));
                                        }
                                    } else {
                                        found_variant_key = Some(base_name.to_string()); // Store the BASE name
                                    }
                                    processed_keys.insert(key_str.clone());
                                    key_matched = true;
                                }
                            )*
                            // If the key didn't match any expected variant key (base or underscore), ignore it?
                            // Or error? Let's ignore for now, assuming other fields might be present.
                            // if !key_matched {
                            //     // Handle unexpected fields if necessary
                            // }
                        }

                        // Ensure a variant key was found
                        let variant_key = match found_variant_key {
                            Some(key) => key, // key is the base name (String)
                            None => {
                                // No matching key found at all
                                return Err(serde::de::Error::custom(format!(
                                    "Expected one of the variant keys {:?} (or their underscore-prefixed versions) but found none",
                                    [#(#variant_names),*]
                                )));
                            }
                        };

                        // --- Construct the variant based on found_variant_key, value_part, extension_part ---
                        match variant_key.as_str() {
                            // Use the pre-generated match arms
                            #(#variant_matches)*

                            // Fallback for unknown variant key (should not be reached if logic above is correct)
                            _ => Err(serde::de::Error::unknown_variant(&variant_key, &[#(#variant_names),*])),
                        }
                    }
                }

                // Use the visitor to deserialize the enum (no longer needs variants)
                deserializer.deserialize_map(EnumVisitor) // Removed variants passing
            };
        }
        Data::Struct(ref data) => {
            match data.fields {
                Fields::Named(ref fields) => {
                    // Struct to hold field metadata for generating FhirDeserialize impl
                    struct FieldInfo {
                        field_ident: syn::Ident,
                        field_ident_ext: Option<syn::Ident>,
                        json_name: String,
                        json_name_ext: Option<String>,
                        field_type: TokenStream,
                        field_type_ext: Option<TokenStream>,
                        is_flattened: bool,
                        tmp_ident: syn::Ident,
                        tmp_ident_ext: Option<syn::Ident>,
                    }
                    let mut field_infos = Vec::new();

                    for field in fields.named.iter() {
                        let field_name_ident = field.ident.as_ref().unwrap(); // Keep original ident for access
                        let field_name_ident_ext = format_ident!("{}_ext", field_name_ident);
                        let tmp_ident = format_ident!("tmp_{}", field_name_ident);
                        let field_ty = &field.ty;
                        let effective_field_name_str = get_effective_field_name(field);
                        let _underscore_field_name_str =
                            format_ident!("_{}", effective_field_name_str);

                        // Destructure the 4 return values
                        let (is_element, is_decimal_element, is_option, is_vec) =
                            get_element_info(field_ty);

                        let is_fhir_element = is_element || is_decimal_element;

                        // Determine the type for the primitive value field in the temp struct
                        let temp_primitive_type_quote = if is_fhir_element {
                            // Need to re-determine the base type here
                            let base_type = get_base_type(field_ty);

                            // Determine the base primitive type (e.g., bool, String, rust_decimal::Decimal)
                            let primitive_type_ident = if is_decimal_element {
                                // For DecimalElement, use serde_json::Value in temp struct to preserve original string
                                quote! { serde_json::Value }
                            } else {
                                // is_element is true here
                                if let Type::Path(type_path) = base_type {
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
                                            .unwrap_or_else(|_| {
                                                panic!(
                                                    "Failed to parse primitive type string: {}",
                                                    primitive_type_str
                                                )
                                            });
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
                                // Temp type should be Option<Vec<Option<Primitive>>> to handle nulls inside the array
                                quote! { Option<Vec<Option<#primitive_type_ident>>> } // Add Option inside Vec
                            } else {
                                // If original field was Option<T>, temp type is Option<Primitive>
                                // If original field was T, temp type is Primitive
                                if is_option {
                                    quote! { Option<#primitive_type_ident> }
                                } else {
                                    // Always use Option<Primitive> in temp struct for single elements
                                    quote! { Option<#primitive_type_ident> }
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
                                        // Handle Option for primitives and extensions
                                        let primitives = temp_struct.#field_name_ident.unwrap_or_default(); // Vec<Option<Primitive>>
                                        let extensions = temp_struct.#field_name_ident_ext.unwrap_or_default(); // Vec<Option<IdAndExtensionHelper>>
                                        let len = primitives.len().max(extensions.len());
                                        let mut result_vec = Vec::with_capacity(len);
                                        for i in 0..len {
                                            // Get Option<Primitive> by flattening the Option<Option<Primitive>> from the vec
                                            let prim_val_opt = primitives.get(i).cloned().flatten();
                                            let ext_helper_opt = extensions.get(i).cloned().flatten(); // Keep flatten here
                                            if prim_val_opt.is_some() || ext_helper_opt.is_some() {
                                                // Deserialize the Option<serde_json::Value> into Option<PreciseDecimal>
                                                let precise_decimal_value = match prim_val_opt {
                                                    Some(json_val) if !json_val.is_null() => {
                                                        use serde::de::{DeserializeSeed, IntoDeserializer};
                                                        let ctx = ::helios_serde::DeserializationContext::<crate::PreciseDecimal, ::helios_serde::Json>::json();
                                                        ctx.deserialize(json_val.into_deserializer())
                                                            .map(Some)
                                                            .map_err(serde::de::Error::custom)? // Map error here
                                                    },
                                                    _ => None, // Treat None or JSON null as None
                                                };
                                                result_vec.push(#element_type {
                                                    value: precise_decimal_value,
                                                    id: ext_helper_opt.as_ref().and_then(|h| h.id.clone()),
                                                    extension: ext_helper_opt.as_ref().and_then(|h| h.extension.clone()),
                                                });
                                            }
                                            // Note: Skipping adding element if both parts are null/None
                                        }
                                        result_vec // Return the vec directly
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
                                        // Handle Option for primitives and extensions
                                        let primitives = temp_struct.#field_name_ident.unwrap_or_default(); // Vec<Option<Primitive>>
                                        let extensions = temp_struct.#field_name_ident_ext.unwrap_or_default(); // Vec<Option<IdAndExtensionHelper>>
                                        let len = primitives.len().max(extensions.len());
                                        let mut result_vec = Vec::with_capacity(len);
                                        for i in 0..len {
                                            // Get Option<Primitive> by flattening the Option<Option<Primitive>> from the vec
                                            let prim_val_opt = primitives.get(i).cloned().flatten();
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
                                    // For Option<Vec<Element>>, construct Some if either primitive or extension array was present
                                    quote! {
                                        #field_name_ident: if temp_struct.#field_name_ident.is_some() || temp_struct.#field_name_ident_ext.is_some() {
                                            // No '?' needed here as the block returns Vec<Element> directly
                                            Some(#construction_logic)
                                        } else {
                                            None
                                        },
                                    }
                                } else {
                                    // Direct Vec<Element> field assignment (always construct the Vec)
                                    quote! {
                                        // No '?' needed here as the block returns Vec<Element> directly
                                        #field_name_ident: #construction_logic,
                                    }
                                }
                            } else if is_decimal_element {
                                // Handle single DecimalElement or Option<DecimalElement>
                                if is_option {
                                    // Logic for Option<DecimalElement>
                                    let construction_logic = quote! { { // Block expression starts
                                        // Deserialize PreciseDecimal from Option<serde_json::Value>
                                        let precise_decimal_value = match temp_struct.#field_name_ident {
                                            Some(json_val) if !json_val.is_null() => {
                                                use serde::de::{DeserializeSeed, IntoDeserializer};
                                                let ctx = ::helios_serde::DeserializationContext::<crate::PreciseDecimal, ::helios_serde::Json>::json();
                                                ctx.deserialize(json_val.into_deserializer())
                                                    .map(Some)
                                                    .map_err(serde::de::Error::custom)? // Map error here
                                            },
                                        _ => None, // Treat None or JSON null as None
                                        };
                                        // Construct the DecimalElement (no Ok() needed)
                                        crate::DecimalElement {
                                            value: precise_decimal_value,
                                            id: temp_struct.#field_name_ident_ext.as_ref().and_then(|h| h.id.clone()),
                                            extension: temp_struct.#field_name_ident_ext.as_ref().and_then(|h| h.extension.clone()),
                                        }
                                    } }; // Block expression ends
                                    // Wrap in Some() only if value or extension exists
                                    quote! {
                                         #field_name_ident: if temp_struct.#field_name_ident.is_some() || temp_struct.#field_name_ident_ext.is_some() {
                                             // No '?' needed here as the block returns DecimalElement directly
                                             Some(#construction_logic)
                                         } else {
                                             None // If neither field present, result is None
                                         },
                                    }
                                } else {
                                    // Logic for non-optional DecimalElement
                                    quote! {
                                        #field_name_ident: { // Block expression starts
                                            // Deserialize PreciseDecimal from Option<serde_json::Value>
                                            let precise_decimal_value = match temp_struct.#field_name_ident {
                                                Some(json_val) if !json_val.is_null() => {
                                                    use serde::de::{DeserializeSeed, IntoDeserializer};
                                                    let ctx = ::helios_serde::DeserializationContext::<crate::PreciseDecimal, ::helios_serde::Json>::json();
                                                    ctx.deserialize(json_val.into_deserializer())
                                                        .map(Some)
                                                        .map_err(serde::de::Error::custom)? // Map error here
                                                },
                                                _ => None, // Treat None or JSON null as None
                                            };
                                            // Construct the DecimalElement (no Ok() needed)
                                            crate::DecimalElement {
                                                value: precise_decimal_value,
                                                id: temp_struct.#field_name_ident_ext.as_ref().and_then(|h| h.id.clone()),
                                                extension: temp_struct.#field_name_ident_ext.as_ref().and_then(|h| h.extension.clone()),
                                            }
                                        }, // No '?' needed after block
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
                                // Construct element explicitly
                                quote! {
                                    #field_name_ident: {
                                        let mut element = #field_ty::default(); // Create default element (e.g., Code::default())
                                        element.value = temp_struct.#field_name_ident; // Assign Option<Primitive>
                                        // Assign id/extension from helper if present
                                        if let Some(helper) = temp_struct.#field_name_ident_ext {
                                            element.id = helper.id;
                                            element.extension = helper.extension;
                                        }
                                        element // Return the constructed element
                                    },
                                }
                            }
                        } else {
                            // Not an FHIR element type
                            quote! {
                                #field_name_ident: temp_struct.#field_name_ident,
                            }
                        }; // Semicolon ends the let constructor_attribute binding

                        constructor_attributes.push(constructor_attribute); // Push the result

                        // Collect field info for generating FhirDeserialize impl
                        let is_field_flattened = is_flattened(field);
                        let tmp_ident_ext = if is_fhir_element {
                            Some(format_ident!("tmp_{}", field_name_ident_ext))
                        } else {
                            None
                        };

                        field_infos.push(FieldInfo {
                            field_ident: field_name_ident.clone(),
                            field_ident_ext: if is_fhir_element {
                                Some(field_name_ident_ext.clone())
                            } else {
                                None
                            },
                            json_name: effective_field_name_str.clone(),
                            json_name_ext: if is_fhir_element {
                                Some(underscore_field_name_literal.clone())
                            } else {
                                None
                            },
                            field_type: temp_primitive_type_quote.clone(),
                            field_type_ext: if is_fhir_element {
                                Some(temp_extension_type.clone())
                            } else {
                                None
                            },
                            is_flattened: is_field_flattened,
                            tmp_ident,
                            tmp_ident_ext,
                        });
                    }

                    // We mirror serde_derive's generated layout: materialise a temporary struct,
                    // stream JSON fields into locals via a Visitor, then build the TmpStruct and
                    // finally map it into the actual FHIR type. This keeps behaviour consistent
                    // with serde's derive while still routing every field through our
                    // DeserializationContext bridge.
                    let mut temp_struct_field_names = Vec::new();
                    let mut temp_struct_field_decls = Vec::new();
                    let mut temp_struct_field_values = Vec::new();
                    let mut field_var_decls = Vec::new();
                    let mut match_arms = Vec::new();
                    let mut flatten_storage_decls = Vec::new();
                    let mut flatten_key_checks = Vec::new();
                    let mut flatten_postprocess = Vec::new();

                    for (index, field_info) in field_infos.iter().enumerate() {
                        let field_ident = &field_info.field_ident;
                        let field_type = &field_info.field_type;
                        let tmp_ident = &field_info.tmp_ident;

                        temp_struct_field_names.push(field_ident.clone());
                        temp_struct_field_decls.push(quote! {
                            #field_ident: #field_type,
                        });
                        temp_struct_field_values.push(quote! {
                            #tmp_ident.unwrap_or_default()
                        });

                        field_var_decls.push(quote! {
                            let mut #tmp_ident: Option<#field_type> = None;
                        });

                        if !field_info.is_flattened {
                            let json_name = &field_info.json_name;
                            match_arms.push(quote! {
                                #json_name => {
                                    if #tmp_ident.is_some() {
                                        return Err(serde::de::Error::duplicate_field(#json_name));
                                    }
                                    let ctx = ::helios_serde::DeserializationContext::<#field_type, ::helios_serde::Json>::json();
                                    #tmp_ident = Some(map.next_value_seed(ctx)?);
                                    continue;
                                }
                            });
                        }

                        if let Some(field_ident_ext) = &field_info.field_ident_ext {
                            if let (
                                Some(json_name_ext),
                                Some(field_type_ext),
                                Some(tmp_ident_ext),
                            ) = (
                                &field_info.json_name_ext,
                                &field_info.field_type_ext,
                                &field_info.tmp_ident_ext,
                            ) {
                                temp_struct_field_names.push(field_ident_ext.clone());
                                temp_struct_field_decls.push(quote! {
                                    #field_ident_ext: #field_type_ext,
                                });
                                temp_struct_field_values.push(quote! {
                                    #tmp_ident_ext.unwrap_or_default()
                                });
                                field_var_decls.push(quote! {
                                    let mut #tmp_ident_ext: Option<#field_type_ext> = None;
                                });
                                match_arms.push(quote! {
                                    #json_name_ext => {
                                        if #tmp_ident_ext.is_some() {
                                            return Err(serde::de::Error::duplicate_field(#json_name_ext));
                                        }
                                        let ctx = ::helios_serde::DeserializationContext::<#field_type_ext, ::helios_serde::Json>::json();
                                        #tmp_ident_ext = Some(map.next_value_seed(ctx)?);
                                        continue;
                                    }
                                });
                            }
                        }

                        if field_info.is_flattened {
                            let storage_ident = format_ident!("flatten_storage_{}", index);
                            let base_name = field_info.json_name.clone();
                            flatten_storage_decls.push(quote! {
                                let mut #storage_ident: Option<
                                    serde_json::Map<std::string::String, serde_json::Value>
                                > = None;
                            });
                            flatten_key_checks.push(quote! {
                                if !handled && matches_flatten_key(&key, #base_name) {
                                    let entry = #storage_ident.get_or_insert_with(serde_json::Map::new);
                                    if entry.insert(key.clone(), value.clone()).is_some() {
                                        return Err(serde::de::Error::custom(format!(
                                            "duplicate field '{}'",
                                            key
                                        )));
                                    }
                                    handled = true;
                                }
                            });
                            flatten_postprocess.push(quote! {
                                if let Some(map) = #storage_ident {
                                    if #tmp_ident.is_some() {
                                        return Err(serde::de::Error::duplicate_field(#base_name));
                                    }
                                    let ctx = ::helios_serde::DeserializationContext::<#field_type, ::helios_serde::Json>::json();
                                    let owned = serde_json::Value::Object(map);
                                    let parsed = ctx
                                        .deserialize(serde::de::IntoDeserializer::into_deserializer(owned))
                                        .map_err(|e| serde::de::Error::custom(format!(
                                            "Failed to deserialize flattened field '{}': {}",
                                            #base_name, e
                                        )))?;
                                    #tmp_ident = Some(parsed);
                                }
                            });
                        }
                    }

                    let id_extension_helper_def = generate_id_and_extension_helper_owned();
                    let temp_struct = quote! {
                        struct #struct_name {
                            #(#temp_struct_field_decls)*
                        }
                    };

                    let has_flatten = !flatten_storage_decls.is_empty();
                    let flatten_helpers = if has_flatten {
                        quote! {
                            fn matches_flatten_key(name: &str, base: &str) -> bool {
                                fn tail_is_upper(tail: &str) -> bool {
                                    tail.chars().next().map(|c| c.is_ascii_uppercase()).unwrap_or(false)
                                }
                                if let Some(rest) = name.strip_prefix(base) {
                                    if tail_is_upper(rest) {
                                        return true;
                                    }
                                }
                                if let Some(rest) = name.strip_prefix('_') {
                                    if let Some(rest) = rest.strip_prefix(base) {
                                        if tail_is_upper(rest) {
                                            return true;
                                        }
                                    }
                                }
                                false
                            }
                        }
                    } else {
                        TokenStream::new()
                    };

                    let fallback_arm = if has_flatten {
                        quote! {
                            {
                                let value: serde_json::Value = map.next_value()?;
                                let mut handled = false;
                                #(#flatten_key_checks)*
                                if !handled {
                                    let _ = value;
                                }
                            }
                        }
                    } else {
                        quote! {
                            {
                                let _: serde::de::IgnoredAny = map.next_value()?;
                            }
                        }
                    };

                    deserialize_impl = quote! {
                        #id_extension_helper_def

                        #temp_struct

                        #flatten_helpers

                        struct StructVisitor;

                        impl<'de> serde::de::Visitor<'de> for StructVisitor {
                            type Value = #struct_name;

                            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                                formatter.write_str(concat!("struct ", stringify!(#name)))
                            }

                            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
                            where
                                A: serde::de::MapAccess<'de>,
                            {
                                use serde::de::DeserializeSeed;
                                #(#field_var_decls)*
                                #(#flatten_storage_decls)*

                                while let Some(key) = map.next_key::<::std::string::String>()? {
                                    match key.as_str() {
                                        #(#match_arms)*
                                        _ => #fallback_arm,
                                    }
                                }

                                #(#flatten_postprocess)*

                                Ok(#struct_name {
                                    #(#temp_struct_field_names: #temp_struct_field_values),*
                                })
                            }
                        }

                        let temp_struct = deserializer.deserialize_map(StructVisitor)?;

                        Ok(#name{#(#constructor_attributes)*})
                    };
                }
                Fields::Unnamed(_) => panic!("Tuple structs not supported by FhirSerde"),
                Fields::Unit => panic!("Unit structs not supported by FhirSerde"),
            }
        }
        Data::Union(_) => panic!("Enums and Unions not supported by FhirSerde"),
    }

    deserialize_impl
}
