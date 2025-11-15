use crate::serde::is_flattened;
use crate::util::get_option_inner_type;
use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, Ident, Lit, Meta, Type, punctuated::Punctuated, token};

pub(crate) fn expand(input: DeriveInput) -> TokenStream {
    let name = &input.ident;
    let generics = &input.generics;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let trait_impl = match &input.data {
        Data::Struct(data) => generate_fhirpath_struct_impl(
            name,
            data,
            &input.attrs,
            &impl_generics,
            &ty_generics,
            where_clause,
        ),
        Data::Enum(data) => generate_fhirpath_enum_impl(
            name,
            data,
            &input.attrs,
            &impl_generics,
            &ty_generics,
            where_clause,
        ),
        Data::Union(_) => panic!("FhirPath derive macro does not support unions."),
    };

    trait_impl
}

/// Determines the effective field name for FHIRPath object property access.
///
/// This function extracts the field name that should be used as a property key
/// in the generated `EvaluationResult::Object`, ensuring that FHIRPath expressions
/// can access fields using their FHIR specification names.
///
/// # Attribute Processing
///
/// - If `#[fhir_serde(rename = "customName")]` is present, uses the custom name
/// - Otherwise, uses the raw Rust field identifier without case conversion
///
/// # Difference from Serialization
///
/// Unlike `get_effective_field_name()` which converts to camelCase for JSON
/// serialization, this function preserves exact FHIR names for FHIRPath access.
/// This ensures FHIRPath expressions match the FHIR specification exactly.
///
/// # Arguments
///
/// * `field` - The field definition from the parsed struct
///
/// # Returns
///
/// The field name as it should appear in FHIRPath object property access.
///
/// # Examples
///
/// ```rust,ignore
/// // Field: pub implicit_rules: Option<Uri>
/// // Result: "implicit_rules" (raw identifier)
///
/// // Field: #[fhir_serde(rename = "implicitRules")]
/// //        pub implicit_rules: Option<Uri>
/// // Result: "implicitRules" (explicit rename for FHIR compliance)
/// ```
fn get_fhirpath_field_name(field: &syn::Field) -> String {
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
    // Default to the raw field identifier if no rename attribute found
    field.ident.as_ref().unwrap().to_string()
}

fn generate_fhirpath_struct_impl(
    name: &Ident,
    data: &syn::DataStruct,
    attrs: &[syn::Attribute],
    impl_generics: &syn::ImplGenerics,
    ty_generics: &syn::TypeGenerics,
    where_clause: Option<&syn::WhereClause>,
) -> proc_macro2::TokenStream {
    let fields = match &data.fields {
        Fields::Named(fields) => &fields.named,
        _ => panic!("FhirPath derive macro only supports structs with named fields."),
    };

    let field_conversions = fields.iter().map(|field| {
        let field_name_ident = field.ident.as_ref().unwrap();
        let field_key_str = get_fhirpath_field_name(field); // Use the specific FHIRPath naming helper
        let field_ty = &field.ty; // Get the field type

        // Check if this field is flattened
        let is_field_flattened = is_flattened(field);

        // Check if this field is a FHIR primitive type that needs special handling
        let fhir_type_name = extract_fhir_primitive_type_name(field_ty);
        // Generate code to handle the field based on whether it's Option
        let is_option = get_option_inner_type(field_ty).is_some();

        // Special handling for flattened fields
        if is_field_flattened {
            // For flattened fields, we need to expand the inner object's fields into the parent map
            if is_option {
                quote! {
                    if let Some(inner_value) = &self.#field_name_ident {
                        let inner_result = inner_value.to_evaluation_result();
                        // If the inner result is an object, merge its fields into our map
                        if let helios_fhirpath_support::EvaluationResult::Object { map: inner_map, .. } = inner_result {
                            for (key, value) in inner_map {
                                map.insert(key, value);
                            }
                        }
                    }
                }
            } else {
                quote! {
                    let inner_result = self.#field_name_ident.to_evaluation_result();
                    // If the inner result is an object, merge its fields into our map
                    if let helios_fhirpath_support::EvaluationResult::Object { map: inner_map, .. } = inner_result {
                        for (key, value) in inner_map {
                            map.insert(key, value);
                        }
                    }
                }
            }
        } else if is_option {
            // For Option<T>, evaluate the inner value only if Some
            if let Some(type_name) = fhir_type_name {
                // Special handling for FHIR primitive types to preserve type information
                quote! {
                    if let Some(inner_value) = &self.#field_name_ident {
                        // Handle FHIR primitive types with proper type preservation
                        let mut field_result = inner_value.to_evaluation_result();
                        // Override type information for string-based FHIR primitive types
                        field_result = match field_result {
                            helios_fhirpath_support::EvaluationResult::String(s, _) => {
                                helios_fhirpath_support::EvaluationResult::fhir_string(s, #type_name)
                            },
                            helios_fhirpath_support::EvaluationResult::Boolean(b, _) => {
                                helios_fhirpath_support::EvaluationResult::fhir_boolean(b)
                            },
                            helios_fhirpath_support::EvaluationResult::Integer(i, _) => {
                                helios_fhirpath_support::EvaluationResult::fhir_integer(i)
                            },
                            helios_fhirpath_support::EvaluationResult::Decimal(d, _) => {
                                helios_fhirpath_support::EvaluationResult::fhir_decimal(d)
                            },
                            _ => field_result,
                        };
                        // Only insert if the inner evaluation is not Empty
                        if field_result != helios_fhirpath_support::EvaluationResult::Empty {
                            map.insert(#field_key_str.to_string(), field_result);
                        }
                    }
                    // If self.#field_name_ident is None, do nothing (don't insert Empty)
                }
            } else {
                quote! {
                    if let Some(inner_value) = &self.#field_name_ident {
                        let field_result = inner_value.to_evaluation_result();
                        // Only insert if the inner evaluation is not Empty
                        if field_result != helios_fhirpath_support::EvaluationResult::Empty {
                            map.insert(#field_key_str.to_string(), field_result);
                        }
                    }
                    // If self.#field_name_ident is None, do nothing (don't insert Empty)
                }
            }
        } else {
            // For non-Option<T>, evaluate directly
            if let Some(type_name) = fhir_type_name {
                // Special handling for FHIR primitive types to preserve type information
                quote! {
                    // Handle FHIR primitive types with proper type preservation
                    let mut field_result = self.#field_name_ident.to_evaluation_result();
                    // Override type information for FHIR primitive types
                    field_result = match field_result {
                        helios_fhirpath_support::EvaluationResult::String(s, _) => {
                            helios_fhirpath_support::EvaluationResult::fhir_string(s, #type_name)
                        },
                        helios_fhirpath_support::EvaluationResult::Boolean(b, _) => {
                            helios_fhirpath_support::EvaluationResult::fhir_boolean(b)
                        },
                        helios_fhirpath_support::EvaluationResult::Integer(i, _) => {
                            helios_fhirpath_support::EvaluationResult::fhir_integer(i)
                        },
                        helios_fhirpath_support::EvaluationResult::Decimal(d, _) => {
                            helios_fhirpath_support::EvaluationResult::fhir_decimal(d)
                        },
                        _ => field_result,
                    };
                    // Only insert if the evaluation is not Empty
                    if field_result != helios_fhirpath_support::EvaluationResult::Empty {
                        map.insert(#field_key_str.to_string(), field_result);
                    }
                }
            } else {
                quote! {
                    let field_result = self.#field_name_ident.to_evaluation_result();
                    // Only insert if the evaluation is not Empty
                    if field_result != helios_fhirpath_support::EvaluationResult::Empty {
                        map.insert(#field_key_str.to_string(), field_result);
                    }
                }
            }
        } // Return the generated code for this field
    });

    // Determine the type name to use for type info
    // For now, we'll use the struct name as the type name
    let type_name_str = name.to_string();

    let into_evaluation_result_impl = quote! {
        impl #impl_generics helios_fhirpath_support::IntoEvaluationResult for #name #ty_generics #where_clause {
            fn to_evaluation_result(&self) -> helios_fhirpath_support::EvaluationResult {
                // Use fully qualified path for HashMap
                let mut map = std::collections::HashMap::new();

                #(#field_conversions)* // Expand the field conversion logic

                // Return a typed object with FHIR type information
                helios_fhirpath_support::EvaluationResult::typed_object(
                    map,
                    "FHIR",
                    &#type_name_str
                )
            }
        }
    };

    // Check if this struct has the fhir_resource attribute with choice_elements
    if let Some(choice_elements) = extract_resource_choice_elements(attrs) {
        let choice_element_literals: Vec<_> = choice_elements
            .iter()
            .map(|elem| quote! { #elem })
            .collect();

        quote! {
            #into_evaluation_result_impl

            impl #impl_generics helios_fhirpath_support::FhirResourceMetadata for #name #ty_generics #where_clause {
                fn choice_elements() -> &'static [&'static str] {
                    &[#(#choice_element_literals),*]
                }
            }
        }
    } else {
        into_evaluation_result_impl
    }
}

fn extract_type_suffix_from_field_name(field_name: &str) -> Option<(&str, &str)> {
    let chars: Vec<char> = field_name.chars().collect();

    let mut transition_index = None;
    for i in 1..chars.len() {
        if chars[i - 1].is_lowercase() && chars[i].is_uppercase() {
            transition_index = Some(i);
            break;
        }
    }

    if let Some(idx) = transition_index {
        let base_name = &field_name[..idx];
        let type_suffix = &field_name[idx..];

        if type_suffix.len() >= 2
            && type_suffix.chars().next().is_some_and(|c| c.is_uppercase())
            && type_suffix.chars().all(|c| c.is_alphanumeric())
            && !base_name.is_empty()
        {
            return Some((base_name, type_suffix));
        }
    }

    None
}

fn extract_choice_element_base_name(attrs: &[syn::Attribute]) -> Option<String> {
    for attr in attrs {
        if attr.path().is_ident("fhir_choice_element") {
            if let Ok(list) =
                attr.parse_args_with(Punctuated::<Meta, token::Comma>::parse_terminated)
            {
                for meta in list {
                    if let Meta::NameValue(nv) = meta {
                        if nv.path.is_ident("base_name") {
                            if let syn::Expr::Lit(expr_lit) = nv.value {
                                if let Lit::Str(lit_str) = expr_lit.lit {
                                    return Some(lit_str.value());
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

fn extract_resource_choice_elements(attrs: &[syn::Attribute]) -> Option<Vec<String>> {
    for attr in attrs {
        if attr.path().is_ident("fhir_resource") {
            if let Ok(list) =
                attr.parse_args_with(Punctuated::<Meta, token::Comma>::parse_terminated)
            {
                for meta in list {
                    if let Meta::NameValue(nv) = meta {
                        if nv.path.is_ident("choice_elements") {
                            if let syn::Expr::Lit(expr_lit) = nv.value {
                                if let Lit::Str(lit_str) = expr_lit.lit {
                                    let elements: Vec<String> = lit_str
                                        .value()
                                        .split(',')
                                        .map(|s| s.trim().to_string())
                                        .filter(|s| !s.is_empty())
                                        .collect();
                                    return Some(elements);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

fn extract_fhir_primitive_type_name(ty: &Type) -> Option<&'static str> {
    let inner_type = if let Some(inner) = get_option_inner_type(ty) {
        inner
    } else {
        ty
    };

    if let Type::Path(type_path) = inner_type {
        if let Some(segment) = type_path.path.segments.last() {
            let type_name = segment.ident.to_string();
            match type_name.as_str() {
                "Uri" => Some("uri"),
                "Code" => Some("code"),
                "Id" => Some("id"),
                "Oid" => Some("oid"),
                "Uuid" => Some("uuid"),
                "Canonical" => Some("canonical"),
                "Url" => Some("url"),
                "Markdown" => Some("markdown"),
                "Base64Binary" => Some("base64Binary"),
                "Instant" => Some("instant"),
                "Date" => Some("date"),
                "DateTime" => Some("dateTime"),
                "Time" => Some("time"),
                "String" => Some("string"),
                "Boolean" => Some("boolean"),
                "Integer" => Some("integer"),
                "Integer64" => Some("integer64"),
                "PositiveInt" => Some("positiveInt"),
                "UnsignedInt" => Some("unsignedInt"),
                "Decimal" => Some("decimal"),
                _ => None,
            }
        } else {
            None
        }
    } else {
        None
    }
}

fn generate_fhirpath_enum_impl(
    name: &Ident,
    data: &syn::DataEnum,
    attrs: &[syn::Attribute],
    impl_generics: &syn::ImplGenerics,
    ty_generics: &syn::TypeGenerics,
    where_clause: Option<&syn::WhereClause>,
) -> proc_macro2::TokenStream {
    // Handle empty enums (like initial R6 Resource enum)
    if data.variants.is_empty() {
        let is_resource_enum = name == "Resource";

        let additional_impl = if is_resource_enum {
            quote! {
                impl #impl_generics crate::FhirResourceTypeProvider for #name #ty_generics #where_clause {
                    fn get_resource_type_names() -> Vec<&'static str> {
                        vec![] // Empty enum has no resource types
                    }
                }
            }
        } else {
            quote! {}
        };

        return quote! {
            impl #impl_generics helios_fhirpath_support::IntoEvaluationResult for #name #ty_generics #where_clause {
                fn to_evaluation_result(&self) -> helios_fhirpath_support::EvaluationResult {
                    // This should never be called for an empty enum
                    unreachable!("Empty enum should not be instantiated")
                }
            }

            #additional_impl
        };
    }

    // Check if the enum being derived is the top-level Resource enum
    let is_resource_enum = name == "Resource";

    // If this is a Resource enum, collect all variant names for the FhirResourceTypeProvider trait
    let resource_type_names: Vec<String> = if is_resource_enum {
        data.variants
            .iter()
            .map(|variant| variant.ident.to_string())
            .collect()
    } else {
        Vec::new()
    };

    let match_arms = data.variants.iter().map(|variant| {
        let variant_name = &variant.ident;
        let variant_name_str = variant_name.to_string();

        match &variant.fields {
            Fields::Unit => {
                // For unit variants, return the variant name as a string (like a code)
                // This is likely for status codes etc., not the Resource enum
                quote! {
                    Self::#variant_name => helios_fhirpath_support::EvaluationResult::string(#variant_name_str.to_string()),
                }
            }
            Fields::Unnamed(fields) if fields.unnamed.len() == 1 => {
                // Newtype variant
                if is_resource_enum {
                    // Special handling for the Resource enum: add resourceType
                    quote! {
                        Self::#variant_name(value) => {
                            let mut result = value.to_evaluation_result(); // Call on inner Box<ResourceStruct>
                            if let helios_fhirpath_support::EvaluationResult::Object { ref mut map, .. } = result {
                                // Insert the resourceType field using the variant name
                                map.insert(
                                    "resourceType".to_string(),
                                    helios_fhirpath_support::EvaluationResult::string(#variant_name_str.to_string())
                                );
                            }
                            // Return the (potentially modified) result
                            result
                        }
                    }
                } else {
                    // For other enums (like choice types), preserve type information from the variant
                    // Extract type information from the variant name or rename attribute
                    let variant_name_str = variant_name.to_string();
                    // Check for fhir_serde rename attribute to get the FHIR field name
                    let mut fhir_field_name = variant_name_str.clone();
                    for attr in &variant.attrs {
                        if attr.path().is_ident("fhir_serde") {
                            if let Ok(list) = attr.parse_args_with(syn::punctuated::Punctuated::<syn::Meta, syn::token::Comma>::parse_terminated) {
                                for meta in list {
                                    if let syn::Meta::NameValue(nv) = meta {
                                        if nv.path.is_ident("rename") {
                                            if let syn::Expr::Lit(expr_lit) = nv.value {
                                                if let syn::Lit::Str(lit_str) = expr_lit.lit {
                                                    fhir_field_name = lit_str.value();
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Extract FHIR type from choice element field name (e.g., "valueCode" -> "code")
                    let fhir_type = if fhir_field_name.starts_with("value") && fhir_field_name.len() > 5 {
                        // Convert first character to lowercase for FHIR primitive types
                        let type_part = &fhir_field_name[5..]; // Remove "value" prefix
                        let mut chars = type_part.chars();
                        match chars.next() {
                            None => variant_name_str.clone(),
                            Some(first) => first.to_lowercase().collect::<String>() + chars.as_str(),
                        }
                    } else if fhir_field_name.ends_with("Boolean") {
                        // Special case for FHIR boolean primitives - use lowercase
                        "boolean".to_string()
                    } else if fhir_field_name.ends_with("Integer") {
                        // Special case for FHIR integer primitives - use lowercase  
                        "integer".to_string()
                    } else if fhir_field_name.ends_with("Decimal") {
                        // Special case for FHIR decimal primitives - use lowercase
                        "decimal".to_string()
                    } else if fhir_field_name.ends_with("String") {
                        // Special case for FHIR string primitives - use lowercase
                        "string".to_string()
                    } else if fhir_field_name.ends_with("Instant") {
                        // Special case for FHIR instant primitives - use lowercase
                        "instant".to_string()
                    } else if fhir_field_name.ends_with("DateTime") {
                        // Special case for FHIR dateTime primitives - use lowercase
                        "dateTime".to_string()
                    } else if fhir_field_name.ends_with("Date") {
                        // Special case for FHIR date primitives - use lowercase
                        "date".to_string()
                    } else if fhir_field_name.ends_with("Time") {
                        // Special case for FHIR time primitives - use lowercase
                        "time".to_string()
                    } else {
                        // Fallback to variant name if it doesn't match known patterns
                        // Convert first character to lowercase for consistency with FHIR primitive naming
                        let mut chars = variant_name_str.chars();
                        match chars.next() {
                            None => variant_name_str.clone(),
                            Some(first) => first.to_lowercase().collect::<String>() + chars.as_str(),
                        }
                    };
                    // For choice type enums that will be flattened, we need to return an object
                    // with the polymorphic field name as the key
                    // A choice type enum is one where variants have rename attributes with type suffixes
                    // e.g., "deceasedBoolean", "valueString", etc.
                    let is_choice_type_enum = fhir_field_name != variant_name_str &&
                        extract_type_suffix_from_field_name(&fhir_field_name).is_some();

                    if is_choice_type_enum {
                        quote! {
                            Self::#variant_name(value) => {
                                // Get the base evaluation result from the inner value
                                let mut result = value.to_evaluation_result();
                                // Add FHIR type information to preserve type for .ofType() operations
                                // For choice type enums, always use the type determined from the field name
                                result = match result {
                                    helios_fhirpath_support::EvaluationResult::String(s, _existing_type_info) => {
                                        // Always use the determined type from the field name for choice types
                                        let type_info = helios_fhirpath_support::TypeInfoResult::new("FHIR", &#fhir_type);
                                        helios_fhirpath_support::EvaluationResult::String(s, Some(type_info))
                                    },
                                    helios_fhirpath_support::EvaluationResult::Integer(i, existing_type_info) => {
                                        let type_info = existing_type_info.unwrap_or_else(|| helios_fhirpath_support::TypeInfoResult::new("FHIR", &#fhir_type));
                                        helios_fhirpath_support::EvaluationResult::Integer(i, Some(type_info))
                                    },
                                    helios_fhirpath_support::EvaluationResult::Decimal(d, existing_type_info) => {
                                        let type_info = existing_type_info.unwrap_or_else(|| helios_fhirpath_support::TypeInfoResult::new("FHIR", &#fhir_type));
                                        helios_fhirpath_support::EvaluationResult::Decimal(d, Some(type_info))
                                    },
                                    helios_fhirpath_support::EvaluationResult::Boolean(b, existing_type_info) => {
                                        let type_info = existing_type_info.unwrap_or_else(|| helios_fhirpath_support::TypeInfoResult::new("FHIR", &#fhir_type));
                                        helios_fhirpath_support::EvaluationResult::Boolean(b, Some(type_info))
                                    },
                                    helios_fhirpath_support::EvaluationResult::Object { map, type_info: existing_type_info } => {
                                        let type_info = existing_type_info.unwrap_or_else(|| helios_fhirpath_support::TypeInfoResult::new("FHIR", &#fhir_type));
                                        helios_fhirpath_support::EvaluationResult::Object {
                                            map,
                                            type_info: Some(type_info),
                                        }
                                    },
                                    _ => result, // For other types, return as-is
                                };

                                // Wrap the result in an object with the field name as the key
                                let mut map = std::collections::HashMap::new();
                                map.insert(#fhir_field_name.to_string(), result);
                                helios_fhirpath_support::EvaluationResult::Object {
                                    map,
                                    type_info: None, // No type info for the wrapper object
                                }
                            }
                        }
                    } else {
                        quote! {
                            Self::#variant_name(value) => {
                                // Get the base evaluation result from the inner value
                                let mut result = value.to_evaluation_result();
                                // Add FHIR type information to preserve type for .ofType() operations
                                // For choice type enums, always use the type determined from the field name
                                result = match result {
                                    helios_fhirpath_support::EvaluationResult::String(s, _existing_type_info) => {
                                        // Always use the determined type from the field name for choice types
                                        let type_info = helios_fhirpath_support::TypeInfoResult::new("FHIR", &#fhir_type);
                                        helios_fhirpath_support::EvaluationResult::String(s, Some(type_info))
                                    },
                                    helios_fhirpath_support::EvaluationResult::Integer(i, existing_type_info) => {
                                        let type_info = existing_type_info.unwrap_or_else(|| helios_fhirpath_support::TypeInfoResult::new("FHIR", &#fhir_type));
                                        helios_fhirpath_support::EvaluationResult::Integer(i, Some(type_info))
                                    },
                                    helios_fhirpath_support::EvaluationResult::Decimal(d, existing_type_info) => {
                                        let type_info = existing_type_info.unwrap_or_else(|| helios_fhirpath_support::TypeInfoResult::new("FHIR", &#fhir_type));
                                        helios_fhirpath_support::EvaluationResult::Decimal(d, Some(type_info))
                                    },
                                    helios_fhirpath_support::EvaluationResult::Boolean(b, existing_type_info) => {
                                        let type_info = existing_type_info.unwrap_or_else(|| helios_fhirpath_support::TypeInfoResult::new("FHIR", &#fhir_type));
                                        helios_fhirpath_support::EvaluationResult::Boolean(b, Some(type_info))
                                    },
                                    helios_fhirpath_support::EvaluationResult::Object { map, type_info: existing_type_info } => {
                                        let type_info = existing_type_info.unwrap_or_else(|| helios_fhirpath_support::TypeInfoResult::new("FHIR", &#fhir_type));
                                        helios_fhirpath_support::EvaluationResult::Object {
                                            map,
                                            type_info: Some(type_info),
                                        }
                                    },
                                    _ => result, // For other types, return as-is
                                };
                                result
                            }
                        }
                    }
                }
           }
            // For tuple or struct variants (uncommon in FHIR choice types or Resource enum),
            // the direct FHIRPath evaluation is less clear.
            // Returning Empty seems like a reasonable default.
            Fields::Unnamed(_) | Fields::Named(_) => {
                 quote! {
                     // Match all fields but ignore them for now
                     Self::#variant_name { .. } => helios_fhirpath_support::EvaluationResult::Empty,
                 }
            }
        }
    });

    // Handle the case where the enum has no variants
    let body = if data.variants.is_empty() {
        // An empty enum cannot be instantiated, so this method is technically unreachable.
        // Return Empty as a safe default.
        quote! { helios_fhirpath_support::EvaluationResult::Empty }
    } else {
        // Generate the match statement for enums with variants
        quote! {
            match self {
                #(#match_arms)*
            }
        }
    };

    let into_evaluation_result_impl = quote! {
        impl #impl_generics helios_fhirpath_support::IntoEvaluationResult for #name #ty_generics #where_clause {
            fn to_evaluation_result(&self) -> helios_fhirpath_support::EvaluationResult {
                 #body // Use the generated body (either Empty or the match statement)
            }
        }
    };

    // Generate additional FhirResourceTypeProvider implementation for Resource enums
    if is_resource_enum {
        let resource_type_literals: Vec<_> = resource_type_names
            .iter()
            .map(|name| {
                quote! { #name }
            })
            .collect();

        // Generate resource_name method for Resource enum
        let resource_name_arms = data.variants.iter().map(|variant| {
            let variant_name = &variant.ident;
            let variant_name_str = variant_name.to_string();

            match &variant.fields {
                Fields::Unnamed(fields) if fields.unnamed.len() == 1 => {
                    // Newtype variant (expected for Resource enum)
                    quote! {
                        Self::#variant_name(_) => #variant_name_str,
                    }
                }
                _ => {
                    // For other field types, still return the variant name
                    quote! {
                        Self::#variant_name { .. } => #variant_name_str,
                    }
                }
            }
        });

        // Generate get_last_updated method for Resource enum
        let get_last_updated_arms = data.variants.iter().map(|variant| {
            let variant_name = &variant.ident;

            match &variant.fields {
                Fields::Unnamed(fields) if fields.unnamed.len() == 1 => {
                    quote! {
                        Self::#variant_name(resource) => {
                            resource.meta.as_ref()
                                .and_then(|m| m.last_updated.as_ref())
                                .and_then(|lu| {
                                    // Handle Element<PrecisionDateTime> - get the value and convert to chrono
                                    lu.value.as_ref().map(|precision_dt| {
                                        // PrecisionDateTime has a to_chrono_datetime() method
                                        precision_dt.to_chrono_datetime()
                                    })
                                })
                        }
                    }
                }
                _ => {
                    quote! {
                        Self::#variant_name { .. } => None,
                    }
                }
            }
        });

        quote! {
            #into_evaluation_result_impl

            impl #impl_generics #name #ty_generics #where_clause {
                /// Returns the resource type name as a string.
                /// This is equivalent to the resourceType field in FHIR JSON.
                pub fn resource_name(&self) -> &'static str {
                    match self {
                        #(#resource_name_arms)*
                    }
                }

                /// Returns the lastUpdated timestamp from the resource's metadata if available.
                pub fn get_last_updated(&self) -> Option<::chrono::DateTime<::chrono::Utc>> {
                    match self {
                        #(#get_last_updated_arms)*
                    }
                }
            }

            impl #impl_generics crate::FhirResourceTypeProvider for #name #ty_generics #where_clause {
                fn get_resource_type_names() -> Vec<&'static str> {
                    vec![#(#resource_type_literals),*]
                }
            }
        }
    } else {
        // Check if this is a choice element enum
        if let Some(base_name) = extract_choice_element_base_name(attrs) {
            // Extract possible field names from the enum variants
            let field_names: Vec<String> = data.variants.iter().filter_map(|variant| {
                // Look for the fhir_serde(rename = "...") attribute
                for attr in &variant.attrs {
                    if attr.path().is_ident("fhir_serde") {
                        if let Ok(list) = attr.parse_args_with(syn::punctuated::Punctuated::<syn::Meta, syn::token::Comma>::parse_terminated) {
                            for meta in list {
                                if let syn::Meta::NameValue(nv) = meta {
                                    if nv.path.is_ident("rename") {
                                        if let syn::Expr::Lit(expr_lit) = nv.value {
                                            if let syn::Lit::Str(lit_str) = expr_lit.lit {
                                                return Some(lit_str.value());
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                None
            }).collect();

            let field_name_literals: Vec<_> =
                field_names.iter().map(|name| quote! { #name }).collect();

            quote! {
                #into_evaluation_result_impl

                impl #impl_generics helios_fhirpath_support::ChoiceElement for #name #ty_generics #where_clause {
                    fn base_name() -> &'static str {
                        #base_name
                    }

                    fn possible_field_names() -> Vec<&'static str> {
                        vec![#(#field_name_literals),*]
                    }
                }
            }
        } else {
            into_evaluation_result_impl
        }
    }
}
