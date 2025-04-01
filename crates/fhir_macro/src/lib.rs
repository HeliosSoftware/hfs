use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::{format_ident, quote}; // Added format_ident back
use syn::{
    Data,
    DeriveInput,
    Fields,
    GenericArgument,
    Ident,
    LitStr,
    PathArguments, // Removed Meta, NestedMeta
    Type,
    parse_macro_input,
}; // Added Ident back
// Removed unused HashMap import

// Helper function to check if a type is Option<T> and return T
fn get_option_inner_type(ty: &Type) -> Option<&Type> {
    if let Type::Path(type_path) = ty {
        if type_path.qself.is_none() && type_path.path.segments.len() == 1 {
            let segment = &type_path.path.segments[0];
            if segment.ident == "Option" {
                if let PathArguments::AngleBracketed(args) = &segment.arguments {
                    if args.args.len() == 1 {
                        if let GenericArgument::Type(inner_ty) = &args.args[0] {
                            return Some(inner_ty);
                        }
                    }
                }
            }
        }
    }
    None
}

// Helper function to get the V and E types from Element<V, E>, DecimalElement<E>, or known aliases.
// Returns Ok((V_Type, E_Type)) or Err(()) if not an element-like type.
// Replaces both is_element_or_decimal_element and get_element_generics
fn try_get_element_generics(ty: &Type) -> Result<(Type, Type), ()> {
    if let Type::Path(type_path) = ty {
        if type_path.qself.is_none() {
            // Allow multi-segment paths
            if let Some(segment) = type_path.path.segments.last() {
                // Check the last segment
                if let PathArguments::AngleBracketed(args) = &segment.arguments {
                    // Handle Element<V, E> (potentially multi-segment path)
                    if segment.ident == "Element" && args.args.len() == 2 {
                        let v_arg = &args.args[0];
                        let e_arg = &args.args[1];
                        let v_type = match v_arg {
                            GenericArgument::Type(t) => t.clone(),
                            _ => panic!("Expected Type for V"),
                        };
                        let e_type = match e_arg {
                            GenericArgument::Type(t) => t.clone(),
                            _ => panic!("Expected Type for E"),
                        };
                        return Ok((v_type, e_type));
                    // Handle DecimalElement<E> (potentially multi-segment path)
                    } else if segment.ident == "DecimalElement" && args.args.len() == 1 {
                        // V is crate::PreciseDecimal for DecimalElement
                        let precise_decimal_type =
                            syn::parse_str::<Type>("crate::PreciseDecimal").unwrap();
                        let e_arg = &args.args[0];
                        let e_type = match e_arg {
                            GenericArgument::Type(t) => t.clone(),
                            _ => panic!("Expected Type for E"),
                        };
                        return Ok((precise_decimal_type, e_type));
                    }
                    // If it has generics but isn't Element/DecimalElement, fall through to panic below.
                }
                // else: No angle-bracketed generics found. Proceed to alias check.

                // Attempt 2: If it wasn't Element/DecimalElement with generics, try matching known aliases.
                let ident_str = segment.ident.to_string();
                // Assume R4 context for aliases like Date, Boolean -> use crate::r4::Extension
                let extension_type = syn::parse_str::<Type>("crate::r4::Extension")
                    .expect("Failed to parse crate::r4::Extension type");
                // --- Refined Alias Check ---
                // Only match FHIR-specific aliases, not base Rust types.
                let value_type_str_opt: Option<&str> = match ident_str.as_str() {
                    // Keep FHIR aliases (assuming they alias Element<...>)
                    "Boolean" | "Integer" | "PositiveInt" | "UnsignedInt" | "Integer64" | "Decimal" |
                    "Base64Binary" | "Canonical" | "Code" | "Date" | "DateTime" | "Id" |
                    "Instant" | "Markdown" | "Oid" | "Time" | "Uri" | "Url" | "Uuid" | "Xhtml"
                        => {
                            // Map the alias name back to the expected primitive *value* type inside the Element
                            match ident_str.as_str() {
                                "Boolean" => Some("bool"),
                                "Integer" | "PositiveInt" | "UnsignedInt" => Some("std::primitive::i32"),
                                "Integer64" => Some("std::primitive::i64"),
                                "Decimal" => Some("crate::PreciseDecimal"),
                                // All other FHIR aliases map to String
                                _ => Some("std::string::String"),
                            }
                        }
                    // Do NOT match base types like "String", "bool", "i32" here.
                    _ => None, // Not a known FHIR alias
                };


                if let Some(value_type_str) = value_type_str_opt {
                    let value_type =
                        syn::parse_str::<Type>(value_type_str).expect("Failed to parse value type");
                    return Ok((value_type, extension_type));
                }
                // else: Fall through to panic if not Element/DecimalElement and not a known alias.
            }
        }
    }

    // If initial Type::Path check failed or if it wasn't Element/DecimalElement or a known alias.
    Err(())
}

// Helper function to convert Rust field name (snake_case) to FHIR JSON name (camelCase)
fn rust_to_fhir_name(ident: &Ident) -> String {
    let name = ident.to_string();
    let name = name.trim_start_matches("r#"); // Handle raw identifiers like r#type

    let mut camel_case = String::new();
    let mut capitalize_next = false;

    for c in name.chars() {
        if c == '_' {
            capitalize_next = true;
        } else if capitalize_next {
            camel_case.push(c.to_ascii_uppercase());
            capitalize_next = false;
        } else {
            camel_case.push(c);
        }
    }
    camel_case
}

#[proc_macro_derive(FhirSerde)]
pub fn fhir_derive_macro(input: TokenStream) -> TokenStream {
    // Bring quote into scope for the entire macro function
    use quote::quote;
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;

    let fields = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(fields) => &fields.named,
            _ => panic!("FhirSerde only supports structs with named fields"),
        },
        _ => panic!("FhirSerde can only be derived for structs"),
    };

    // --- Generate Serialize Implementation ---
    let mut serialize_fields = Vec::new();
    let mut field_count_calculation = Vec::new();

    // Define the serialization helper name *before* the loop that uses it
    // Removed leading __
    let serialize_extension_helper_name = format_ident!("{}SerializeExtensionHelper", name);

    for field in fields {
        let field_ident = field.ident.as_ref().unwrap();
        let field_ty = &field.ty;
        // Convert Rust field name to FHIR JSON name (camelCase)
        let original_name = rust_to_fhir_name(field_ident);
        let original_name_lit = LitStr::new(&original_name, Span::call_site());
        let underscore_name_lit = LitStr::new(&format!("_{}", original_name), Span::call_site());

        // Determine if the field is Option<T> and get the inner type
        let (is_option, inner_ty) = match get_option_inner_type(field_ty) {
            Some(inner) => (true, inner),
            None => (false, field_ty), // Treat non-optional as inner_ty = field_ty
        };

        // Try to determine if the inner type represents an Element-like structure
        match try_get_element_generics(inner_ty) {
            Ok((_v_ty, ext_ty)) => {
                // Type IS Element-like (Element, DecimalElement, or alias like r4::Date)

                if is_option {
                    // Case: Option<Element-like>
                    // Apply the complex FHIR serialization logic (_fieldName vs fieldName)

                    // Calculate contribution to field count (based on element fields)
                    field_count_calculation.push(quote! {


                       if let Some(element) = &self.#field_ident {
                            // Add 1 if value is present
                            if element.value.is_some() { count += 1; }
                            // Add 1 if id or extension is present (for the underscore field)
                            if element.id.is_some() || element.extension.is_some() { count += 1; }
                        }
                    });

                    // Generate serialization logic according to FHIR JSON rules
                    serialize_fields.push(quote! {
                    if let Some(element) = &self.#field_ident {
                        let has_value = element.value.is_some();
                        let has_extension_data = element.id.is_some() || element.extension.is_some();

                        if has_value && !has_extension_data {
                            // Case 1: Only value -> "fieldName": value
                            state.serialize_field(#original_name_lit, element.value.as_ref().unwrap())?;
                        } else if !has_value && has_extension_data {
                            // Case 2: Only id/extension -> "_fieldName": { ... }
                            // Use ::serde prefix for Serialize trait bound on E
                            let helper = #serialize_extension_helper_name::<'_, #ext_ty> {
                                id: &element.id,
                                extension: &element.extension,
                            };
                            state.serialize_field(#underscore_name_lit, &helper)?;
                        } else if has_value && has_extension_data {
                            // Case 3: Both value and id/extension -> "fieldName": value, "_fieldName": { ... }
                            state.serialize_field(#original_name_lit, element.value.as_ref().unwrap())?;
                            // Use ::serde prefix for Serialize trait bound on E
                            let helper = #serialize_extension_helper_name::<'_, #ext_ty> {
                                id: &element.id,
                                extension: &element.extension,
                            };
                            state.serialize_field(#underscore_name_lit, &helper)?;
                        }
                    }
                });
                    // Case 4: Neither value nor id/extension -> Field is omitted entirely (handled by field_count_calculation)

                    // Case: Non-optional Element-like (e.g., Element<String, Extension>)
                    // This is less common with the FHIR generator but should be handled.
                    // Serialize as a full object { id?, extension?, value? }
                    // Note: This assumes the Element/DecimalElement Serialize impl handles this correctly.
                    // We just need to count it and add it to the state.
                    field_count_calculation.push(quote! { count += 1; });
                    serialize_fields.push(quote! {
                        state.serialize_field(#original_name_lit, &self.#field_ident)?;
                    });
                }
            }
            Err(_) => {
                // Type is NOT Element-like (e.g., String, i32, bool, other struct)

                if is_option {
                    // Case: Option<Regular T>
                    field_count_calculation.push(quote! {
                        if self.#field_ident.is_some() { count += 1; }
                    });
                    serialize_fields.push(quote! {
                        // Serialize the inner value directly using the original field name
                        if let Some(value) = &self.#field_ident {
                             state.serialize_field(#original_name_lit, value)?;
                        }
                    });
                } else {
                    // Case: Regular T (non-optional)
                    field_count_calculation.push(quote! {
                        count += 1; // Always count non-optional fields
                    });
                    serialize_fields.push(quote! {
                        state.serialize_field(#original_name_lit, &self.#field_ident)?;
                    });
                }
            }
        }
    } // End of loop through fields

    // --- Generate Deserialize Implementation --- (Rest of the macro remains largely the same)
    // ... existing code ...

    // Define Field enum variants and match arms (needed outside the final quote!)
    let mut field_enum_variants = Vec::new();
    let mut field_match_arms = Vec::new();
    let mut field_strings = Vec::new(); // For deserialize_struct

    // Generate unique names for helper types *before* the loop
    let field_enum_name = format_ident!("{}Field", name);
    let field_visitor_name = format_ident!("{}FieldVisitor", name);
    let visitor_struct_name = format_ident!("{}Visitor", name);
    // Removed leading __
    let extension_helper_name = format_ident!("{}ExtensionHelper", name);
    // Serialization helper name is now defined before the serialization loop

    // Temporary struct to hold field info for deserialization generation
    struct FieldInfo<'a> {
        ident: &'a Ident,
        ty: &'a Type,
        original_name: String,
        underscore_name: String,
        is_element: bool,

        inner_ty: &'a Type, // Type inside Option if applicable, otherwise same as ty
        // Store V and E types if is_element is true
        v_ty: Option<Type>,
        ext_ty: Option<Type>,
    }
    let mut field_infos = Vec::new();

    for field in fields {
        // Removed _idx as it's unused
        // Prefix idx with _
        let field_ident = field.ident.as_ref().unwrap();
        let field_ty = &field.ty;
        // Convert Rust field name to FHIR JSON name (camelCase)
        let original_name = rust_to_fhir_name(field_ident);
        let underscore_name = format!("_{}", original_name);
        // Get the field name as a string, removing raw identifier prefix if present
        let clean_field_ident_str = field_ident.to_string().trim_start_matches("r#").to_string();
        // Convert snake_case or camelCase to UpperCamelCase for the base enum variant name
        let field_ident_enum_str = {
            let mut camel_case = String::new();
            let mut capitalize = true;
            for c in clean_field_ident_str.chars() {
                if c == '_' {
                    capitalize = true;
                } else if capitalize {
                    camel_case.push(c.to_ascii_uppercase());
                    capitalize = false;
                } else {
                    camel_case.push(c);
                }
            }
            // Ensure first char is uppercase even if original starts with non-alphabetic
            if let Some(first) = camel_case.chars().next() {
                if !first.is_uppercase() {
                    // This case is less likely with typical field names but handles edge cases
                    camel_case.insert(0, first.to_ascii_uppercase());
                    camel_case.remove(1);
                }
            } else {
                // Handle empty or unusual field names if necessary, though unlikely
                camel_case = format!("Field{}", field_ident); // Fallback name
            }

            camel_case
        };
        let field_ident_enum = format_ident!("{}", field_ident_enum_str);
        // Append "Underscore" for the underscore variant name
        let underscore_ident_enum = format_ident!("{}Underscore", field_ident_enum_str);

        // Determine if the field is Option<T> and get the inner type T
        let (_is_option, inner_ty) = match get_option_inner_type(field_ty) {
            // Prefix is_option with _
            Some(inner) => (true, inner),
            None => (false, field_ty),
        };

        // Try to get generics; determine is_element based on success
        let (is_element, v_ty_opt, ext_ty_opt) = match try_get_element_generics(inner_ty) {
            // <-- Use try_get_element_generics
            Ok((v, e)) => (true, Some(v), Some(e)),
            Err(_) => (false, None, None),
        };

        // Populate field_infos
        field_infos.push(FieldInfo {
            // <-- Populated correctly using try_get_element_generics result
            ident: field_ident,
            ty: field_ty, // Original field type (Option<T> or T)
            original_name: original_name.clone(),
            underscore_name: underscore_name.clone(),
            is_element, // Use the determined flag
            inner_ty,   // Store the inner type T (or original type if not Option)
            // Store V and E types if is_element is true
            v_ty: v_ty_opt,
            ext_ty: ext_ty_opt,
        });

        // For Field enum and match arms
        field_enum_variants.push(quote! { #field_ident_enum });
        // Use #field_enum_name instead of Field
        field_match_arms.push(quote! { #original_name => Ok(#field_enum_name::#field_ident_enum) });
        field_strings.push(original_name.clone()); // Add original name

        // Make sure to add the underscore variant *only* if is_element is true
        if is_element {
            field_enum_variants.push(quote! { #underscore_ident_enum });
            // Use #field_enum_name instead of Field
            field_match_arms
                .push(quote! { #underscore_name => Ok(#field_enum_name::#underscore_ident_enum) });
            field_strings.push(underscore_name);
        } else {
            // If not an element type, ensure we don't add the underscore variant/match arm
            // (This prevents errors if a field coincidentally starts with _)
        }
    } // <-- This closing brace marks the end of the first loop

    // --- The code below belongs *after* the first loop ---

    // Add Ignore variant for unknown fields
    field_enum_variants.push(quote! { Ignore });
    // Use #field_enum_name instead of Field
    field_match_arms.push(quote! { _ => Ok(#field_enum_name::Ignore) });

    // Unique names are now generated before the loop

    let field_enum = quote! {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        // Use the unique name
        enum #field_enum_name { #(#field_enum_variants),* }
    };

    // 2. Implement Visitor trait for Field enum
    let field_visitor_impl = quote! {
        // Use the unique name
        struct #field_visitor_name;

        // Use the unique names
        impl<'de> ::serde::de::Visitor<'de> for #field_visitor_name {
            type Value = #field_enum_name;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a field identifier")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where E: ::serde::de::Error, // Use ::serde path
            {
                 // Use the unique enum name
                match value {
                    #(#field_match_arms),*
                }
            }
             // Handle borrowed strings as well
            fn visit_borrowed_str<E>(self, value: &'de str) -> Result<Self::Value, E>
            where E: ::serde::de::Error, // Use ::serde path
            {
                 match value {
                    // Use the unique enum name here
                    #(#field_match_arms),*
                }
            }
        }

        // Use the unique enum name
        impl<'de> ::serde::Deserialize<'de> for #field_enum_name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where D: ::serde::Deserializer<'de>, // Use ::serde path
            {
                 // Use the unique visitor name
                deserializer.deserialize_identifier(#field_visitor_name)
            }
        }
    };

    // 3. Generate parts needed for the Visitor struct implementation
    let mut visitor_field_defs = Vec::new();
    let mut visitor_map_assignments = Vec::new();
    // Remove visitor_build_steps, construction happens after the loop
    let struct_name_str = name.to_string();

    // Declare ALL final field variables and temporary element parts at the top
    for info in &field_infos {
        let field_ident = info.ident;
        let field_ty = info.ty; // Use the original field type (Option<...> or T)

        // Declare the final variable matching the struct field type, explicitly using None path
        visitor_field_defs
            .push(quote! { let mut #field_ident: #field_ty = ::std::option::Option::None; });

        if info.is_element {
            // Get V type from stored info (populated using try_get_element_generics)
            let val_ty = info.v_ty.as_ref().expect("v_ty missing for element");
            // E type (_ext_ty) is not needed for these definitions

            let val_field = format_ident!("{}_value", field_ident);
            let id_field = format_ident!("{}_id", field_ident);
            let ext_field = format_ident!("{}_extension", field_ident);

            // Use fully qualified paths for Option and Vec
            visitor_field_defs
                .push(quote! { let mut #val_field: ::std::option::Option<#val_ty> = None; });
            visitor_field_defs
                .push(quote! { let mut #id_field: ::std::option::Option<String> = None; });
            // Temporary extension field holds Vec<Value>
            visitor_field_defs.push(quote! { let mut #ext_field: ::std::option::Option<::std::vec::Vec<::serde_json::Value>> = None; });
        }
    }

    // Generate map assignments (assign to temp vars or final vars)
    for info in &field_infos {
        let field_ident = info.ident;
        let _inner_ty = info.inner_ty; // Use inner_ty (prefixed as unused)

        // Get the correct UpperCamelCase enum variant names generated earlier
        let clean_field_ident_str = field_ident.to_string().trim_start_matches("r#").to_string();
        let field_ident_enum_str = {
            let mut camel_case = String::new();
            let mut capitalize = true;
            for c in clean_field_ident_str.chars() {
                if c == '_' {
                    capitalize = true;
                } else if capitalize {
                    camel_case.push(c.to_ascii_uppercase());
                    capitalize = false;
                } else {
                    camel_case.push(c);
                }
            }
            if let Some(first) = camel_case.chars().next() {
                if !first.is_uppercase() {
                    camel_case.insert(0, first.to_ascii_uppercase());
                    camel_case.remove(1);
                }
            } else {
                camel_case = format!("Field{}", field_ident);
            }
            camel_case
        };
        let field_ident_enum = format_ident!("{}", field_ident_enum_str); // e.g., BirthDate
        let underscore_ident_enum = format_ident!("{}Underscore", field_ident_enum_str); // e.g., BirthDateUnderscore

        if info.is_element {
            // Get V type from stored info (populated using try_get_element_generics)
            let val_ty = info.v_ty.as_ref().expect("v_ty missing for element");
            // E type (_ext_ty) is not needed for these assignments

            let id_field = format_ident!("{}_id", field_ident);
            let ext_field = format_ident!("{}_extension", field_ident);
            let val_field = format_ident!("{}_value", field_ident);
            let original_name_lit = LitStr::new(&info.original_name, Span::call_site());
            let underscore_name_lit = LitStr::new(&info.underscore_name, Span::call_site());

            // Assignment for the primitive value (fieldName)
            visitor_map_assignments.push(quote! {
               #field_enum_name::#field_ident_enum => {
                   if #val_field.is_some() { return Err(::serde::de::Error::duplicate_field(#original_name_lit)); }
                   // Deserialize into Option<V> using the extracted val_ty
                   let primitive_value: ::std::option::Option<#val_ty> = map.next_value()?;
                   #val_field = primitive_value;
               }
            });

            // Assignment for the extension object (_fieldName)
            visitor_map_assignments.push(quote! {
                 // Use #field_enum_name directly (defined outside impl block)
                 #field_enum_name::#underscore_ident_enum => {
                    // Deserialize into Value first to check for null
                    let value = map.next_value::<::serde_json::Value>()?;
                    if !value.is_null() {
                        // If not null, deserialize the Value into the helper
                        // Use updated #extension_helper_name (no __) directly (defined outside impl block)
                        // Helper is now non-generic
                        let helper: #extension_helper_name = ::serde_json::from_value(value)
                            .map_err(|e| ::serde::de::Error::custom(format!("Failed to deserialize _field helper: {}", e)))?; // Provide context on error
                        if #id_field.is_some() || #ext_field.is_some() { return Err(::serde::de::Error::duplicate_field(#underscore_name_lit)); }
                        #id_field = helper.id;
                        // Assign the Vec<Value> to the temporary field
                        #ext_field = helper.extension;
                    }
                    // If value was null, do nothing, fields remain None
                }
            });
        } else {
            // Assign directly to the final field variable for non-elements
            let original_name_lit = LitStr::new(&info.original_name, Span::call_site());
            visitor_map_assignments.push(quote! {
               // Use #field_enum_name directly (defined inside impl block)
               #field_enum_name::#field_ident_enum => {
                   if #field_ident.is_some() { return Err(::serde::de::Error::duplicate_field(#original_name_lit)); }
                   // Deserialize directly into the final Option<T> field
                   #field_ident = map.next_value()?; // Use map.next_value() which returns Result<T, Error>
               }
            });
        }
    }

    // Generate the final struct construction logic (remains the same)
    let final_struct_fields: Vec<_> = field_infos
        .iter()
        .map(|info| {
            let field_ident = info.ident;
            quote! { #field_ident: #field_ident }
        })
        .collect();

    // visitor_struct_name already defined above with unique name

    // Element construction logic generation (moved back outside)
    let element_construction_logic: Vec<_> = field_infos.iter().filter_map(|info| {
        if info.is_element {
            let field_ident = info.ident; // Final field ident (e.g., birth_date)

            // Idents for temporary variables *inside* visit_map
            let val_field_ident = format_ident!("{}_value", field_ident);
            let id_field_ident = format_ident!("{}_id", field_ident);
            let ext_field_ident = format_ident!("{}_extension", field_ident); // This holds Option<Vec<Value>>
            let final_ext_field_ident = format_ident!("{}_final_extension", field_ident); // New var for Option<Vec<E>>

            // Get V and E types from the stored info (populated using try_get_element_generics)
            let v_ty_construct = info.v_ty.as_ref().expect("v_ty missing for element");
            let ext_ty = info.ext_ty.as_ref().expect("ext_ty missing for element");

            // --- New Robust Check ---
            // Parse the target type `crate::PreciseDecimal` once for comparison
            // --- Check Original Type Name for "Decimal" ---
            // Determine if we should construct DecimalElement based *only* on whether the
            // original inner type identifier was exactly "Decimal".
            // Get the string representation of the inner_ty (the type inside Option<...>)
            let inner_ty_var = info.inner_ty; // Assign to a local variable first
            let _inner_ty_path_str = quote!(#inner_ty_var).to_string(); // Interpolate the variable // PREFIXED
            // --- Logic moved inside quote block below ---


            Some(quote! {
                // This generated code will be placed inside visit_map
                // It references #field_ident (final variable) and the temporary variables
                 // Deserialize Vec<Value> into Vec<E> *after* the main loop using the stored #ext_ty
                 let #final_ext_field_ident: ::std::option::Option<::std::vec::Vec<#ext_ty>> =
                     match #ext_field_ident {
                         Some(values) => {
                             let mut deserialized_extensions = ::std::vec::Vec::with_capacity(values.len());
                             for value in values {
                                 // Use fully qualified path for Error::custom
                                 // Use the stored #ext_ty
                                 let deserialized_ext: #ext_ty = ::serde_json::from_value(value)
                                     .map_err(|e| ::serde::de::Error::custom(format!("Failed to deserialize extension element: {}", e)))?;
                                 deserialized_extensions.push(deserialized_ext);
                             }
                             Some(deserialized_extensions)
                         }
                         None => None,
                     };

                 // Assign the constructed Option<Element/DecimalElement> directly to the final field variable
                 // Use the final_ext_field_ident which holds Option<Vec<E>>
                 // Use stored #v_ty_construct and #ext_ty
                #field_ident = if #val_field_ident.is_some() || #id_field_ident.is_some() || #final_ext_field_ident.is_some() {
                    // Construct the Element/DecimalElement directly inside Some()
                    // --- Check V type inside generated code ---
                    // Convert the interpolated type #v_ty_construct to a string and normalize it
                    let v_ty_string_check = stringify!(#v_ty_construct).replace(" ", ""); // Revert to stringify!
                    // Compare against the normalized target string
                    let is_decimal_type_check = v_ty_string_check == "crate::PreciseDecimal";
                    // --- End check ---
                    ::std::option::Option::Some(
                        if is_decimal_type_check { // Use the check performed inside the generated code
                            // Use stored #ext_ty
                            crate::DecimalElement::<#ext_ty> { value: #val_field_ident, id: #id_field_ident, extension: #final_ext_field_ident }
                        } else {
                            // Use stored #v_ty_construct and #ext_ty
                            crate::Element::<#v_ty_construct, #ext_ty> { value: #val_field_ident, id: #id_field_ident, extension: #final_ext_field_ident }
                        }
                    ) // End of Some(...)
                } else {
                    // If no value, id, or extension were found, the final field is None
                    ::std::option::Option::None
                };
            })
        } else {
            None
        }
    }).collect();

    // Define the extension helper struct for Deserialize here as well
    // Use updated helper name (no __)
    // Make it non-generic, deserialize extension as Vec<Value>
    let deserialize_extension_helper_def = quote! {
        #[derive(::serde::Deserialize)] // Use Deserialize from use statement
        struct #extension_helper_name { // No generic E, no lifetime 'de needed here
             #[serde(default)]
             id: ::std::option::Option<String>,
             #[serde(default)]
             // Deserialize into Vec<Value> initially
             extension: ::std::option::Option<::std::vec::Vec<::serde_json::Value>>,
        }
    };

    // --- Generate Deserialize Implementation ---

    // Define the main visitor struct and its implementation
    let visitor_impl = quote! {
        // Define the main visitor struct
        struct #visitor_struct_name;

        impl<'de> ::serde::de::Visitor<'de> for #visitor_struct_name {
            type Value = #name; // Use #name directly as it's in the outer scope

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str(concat!("struct ", #struct_name_str))
            }

            fn visit_map<V>(self, mut map: V) -> Result<#name, V::Error>
            where
                V: ::serde::de::MapAccess<'de>, // Use ::serde path
            {
                // Bring quote into scope for generated code inside visit_map
                use quote::quote;
                // Use fully qualified paths instead of use statements inside function
                // use ::serde::de; // Needed for de::Error
                // Need Deserialize in scope for helper derives
                // use ::serde::Deserialize;

                // Define helper types (Field enum, FieldVisitor, ExtensionHelper) *inside* the visit_map scope
                // This ensures they are unique per struct and avoids polluting the outer scope,
                // but requires them to be accessible when map.next_key/value is called.
                // Let's keep them defined *outside* the impl block for simplicity and clarity.

                // Initialize Option fields for the visitor
                #(#visitor_field_defs;)*

                // Loop through fields in the JSON map
                // Use #field_enum_name directly (defined outside impl block)
                while let Some(key) = map.next_key::<#field_enum_name>()? {
                    match key {
                        #(#visitor_map_assignments)*
                        // Use #field_enum_name directly (defined outside impl block)
                        #field_enum_name::Ignore => { let _ = map.next_value::<::serde::de::IgnoredAny>()?; } // Use ::serde path
                    }
                }

                // Construct Element fields *after* the loop using temp variables
                // This block now includes the logic to deserialize extensions from Vec<Value>
                #(#element_construction_logic)*

                // Construct the final struct using the final field variables
                Ok(#name {
                    #(#final_struct_fields),*
                })
            }
        }
    };

    // Ensure the main Deserialize impl correctly declares and uses the 'de lifetime
    let deserialize_impl = quote! {
        impl<'de> ::serde::Deserialize<'de> for #name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: ::serde::Deserializer<'de>, // Use ::serde path
            {
                // Define the fields Serde should expect
                const FIELDS: &'static [&'static str] = &[#(#field_strings),*];
                // Start deserialization using the main visitor struct defined outside
                deserializer.deserialize_struct(#struct_name_str, FIELDS, #visitor_struct_name)
            }
        }
    };

    // --- Combine Serialize and Deserialize ---
    // Define the serialization helper struct definition
    // Use updated helper name (no __)
    let serialize_helper_struct_def = quote! {
        // Use ::serde::Serialize path for derive and trait bound
        #[derive(::serde::Serialize)]
        struct #serialize_extension_helper_name<'a, E: ::serde::Serialize> {
            #[serde(skip_serializing_if = "Option::is_none")]
            id: &'a ::std::option::Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            extension: &'a ::std::option::Option<::std::vec::Vec<E>>,
        }
    };

    let serialize_impl = quote! {
        impl ::serde::Serialize for #name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: ::serde::Serializer, // Use ::serde path
            {
                // Use fully qualified paths instead of use statements inside function
                // use serde::ser::{SerializeStruct, Serializer};
                // Need Serialize in scope for the helper derive
                // use ::serde::Serialize;

                // Serialization helper struct is defined outside the impl block now

                // Calculate the number of fields to serialize
                let mut count = 0;
                #(#field_count_calculation)*

                // Start serialization
                // Use ::serde path for SerializeStruct
                let mut state = serializer.serialize_struct(stringify!(#name), count)?;

                // Serialize each field
                #(#serialize_fields)*

                state.end()
            }
        }
    };

    // Combine implementations. Helper types are defined *before* the impl blocks.
    let expanded = quote! {
        // Define ALL helper types first
        #field_enum
        #field_visitor_impl
        #deserialize_extension_helper_def
        #serialize_helper_struct_def

        // Define the main visitor struct and its implementation
        #visitor_impl

        // Define the main impls that use these helpers
        #serialize_impl
        #deserialize_impl
    };

    // For debugging: Print the generated code
    // println!("{}", expanded.to_string());

    expanded.into()
}
