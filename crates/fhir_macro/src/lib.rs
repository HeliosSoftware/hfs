use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::{format_ident, quote}; // Added format_ident back
use syn::{
    Data, DeriveInput, Fields, GenericArgument, Ident, LitStr, Meta, NestedMeta, PathArguments,
    Type, parse_macro_input,
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

// Helper function to check if a type is Element<V, E> or DecimalElement<E>
fn is_fhir_element_type(ty: &Type) -> bool {
    if let Type::Path(type_path) = ty {
        if type_path.qself.is_none() && type_path.path.segments.len() == 1 {
            let segment = &type_path.path.segments[0];
            return segment.ident == "Element" || segment.ident == "DecimalElement";
        }
    }
    false
}

// Helper function to get the original field name from serde attributes
fn get_original_field_name(field: &syn::Field) -> String {
    for attr in &field.attrs {
        // Use attr.path directly (it's a field)
        if attr.path.is_ident("serde") {
            // Use parse_meta to get the Meta item inside the attribute
            if let Ok(Meta::List(meta_list)) = attr.parse_meta() {
                // Iterate over the nested meta items
                for nested in meta_list.nested {
                    // Check if the nested item is a Meta::NameValue pair
                    if let NestedMeta::Meta(Meta::NameValue(nv)) = nested {
                        // Check if the path of the name-value pair is "rename"
                        if nv.path.is_ident("rename") {
                            // Check if the literal is a string literal
                            if let syn::Lit::Str(lit_str) = nv.lit {
                                // Return the value of the string literal
                                return lit_str.value();
                            }
                        }
                    }
                }
            }
        }
    }
    // If no rename attribute, use the field identifier
    field.ident.as_ref().unwrap().to_string()
}

#[proc_macro_derive(FhirSerde)]
pub fn fhir_derive_macro(input: TokenStream) -> TokenStream {
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

    for field in fields {
        let field_ident = field.ident.as_ref().unwrap();
        let field_ty = &field.ty;
        let original_name = get_original_field_name(field);
        let original_name_lit = LitStr::new(&original_name, Span::call_site());
        let underscore_name_lit = LitStr::new(&format!("_{}", original_name), Span::call_site());

        // Check if the field is Option<Element<...>> or Option<DecimalElement<...>>
        if let Some(inner_ty) = get_option_inner_type(field_ty) {
            if is_fhir_element_type(inner_ty) {
                // This is a potentially extended primitive field (like Option<Element<String, Extension>>)
                // Need special handling for fieldName and _fieldName

                // Calculate contribution to field count
                field_count_calculation.push(quote! {
                    if let Some(element) = &self.#field_ident {
                        // Add 1 if value is present
                        if element.value.is_some() { count += 1; }
                        // Add 1 if id or extension is present (for the underscore field)
                        if element.id.is_some() || element.extension.is_some() { count += 1; }
                    }
                });

                // Generate serialization logic
                serialize_fields.push(quote! {
                    if let Some(element) = &self.#field_ident {
                        // Serialize the value under the original name, if present
                        if let Some(value) = &element.value {
                            state.serialize_field(#original_name_lit, value)?;
                        }
                        // Serialize id and extension under the underscore name, if present
                        if element.id.is_some() || element.extension.is_some() {
                            // Create a temporary struct holding only id and extension
                            #[derive(::serde::Serialize)] // Use ::serde::
                            struct ExtensionHelper<'a, E: ::serde::Serialize> { // Use ::serde::
                                #[serde(skip_serializing_if = "Option::is_none")] // This is an attribute macro arg, keep as is
                                id: &'a Option<String>,
                                #[serde(skip_serializing_if = "Option::is_none")] // This is an attribute macro arg, keep as is
                                extension: &'a Option<Vec<E>>,
                            }
                            let helper = ExtensionHelper {
                                id: &element.id,
                                extension: &element.extension,
                            };
                            state.serialize_field(#underscore_name_lit, &helper)?;
                        }
                    }
                });
            } else {
                // Regular Option<T> field
                field_count_calculation.push(quote! {
                    if self.#field_ident.is_some() { count += 1; }
                });
                serialize_fields.push(quote! {
                    if let Some(value) = &self.#field_ident {
                        state.serialize_field(#original_name_lit, value)?;
                    }
                });
            }
        } else {
            // Non-optional field (assuming required fields exist based on FHIR spec)
            // Note: FHIR generator seems to make everything Option currently,
            // but handle non-optional just in case.
            field_count_calculation.push(quote! {
                count += 1; // Always count non-optional fields
            });
            serialize_fields.push(quote! {
                state.serialize_field(#original_name_lit, &self.#field_ident)?;
            });
        }
    }

    // --- Generate Deserialize Implementation ---

    // Define Field enum variants and match arms (needed outside the final quote!)
    let mut field_enum_variants = Vec::new();
    let mut field_match_arms = Vec::new();
    let mut field_strings = Vec::new(); // For deserialize_struct

    // Temporary struct to hold field info for deserialization generation
    struct FieldInfo<'a> {
        ident: &'a Ident,
        ty: &'a Type,
        original_name: String,
        underscore_name: String,
        is_element: bool,
        // is_option: bool, // Removed unused field
        inner_ty: &'a Type, // Type inside Option if applicable, otherwise same as ty
    }
    let mut field_infos = Vec::new();

    for (_idx, field) in fields.iter().enumerate() { // Prefix idx with _
        let field_ident = field.ident.as_ref().unwrap();
        let field_ty = &field.ty;
        let original_name = get_original_field_name(field);
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


        let (_is_option, inner_ty) = match get_option_inner_type(field_ty) { // Prefix is_option with _
            Some(inner) => (true, inner),
            None => (false, field_ty), // If not Option, inner_ty is the same as ty
        };
        let is_element = is_fhir_element_type(inner_ty);

        field_infos.push(FieldInfo {
            ident: field_ident,
            ty: field_ty,
            original_name: original_name.clone(),
            underscore_name: underscore_name.clone(),
            is_element,
            // is_option, // Removed unused field
            inner_ty,
        });

        // For Field enum and match arms
        field_enum_variants.push(quote! { #field_ident_enum });
        field_match_arms.push(quote! { #original_name => Ok(Field::#field_ident_enum) });
        field_strings.push(original_name.clone()); // Add original name

        if is_element {
            field_enum_variants.push(quote! { #underscore_ident_enum });
            field_match_arms.push(quote! { #underscore_name => Ok(Field::#underscore_ident_enum) });
            field_strings.push(underscore_name); // Add underscore name only for element types
        }
    }
    // Add Ignore variant for unknown fields
    field_enum_variants.push(quote! { Ignore });
    field_match_arms.push(quote! { _ => Ok(Field::Ignore) });

    // Generate unique names for helper types
    let field_enum_name = format_ident!("{}Field", name);
    let field_visitor_name = format_ident!("{}FieldVisitor", name);
    let visitor_struct_name = format_ident!("{}Visitor", name);
    let extension_helper_name = format_ident!("__{}FhirSerdeExtensionHelper", name);


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
            where E: ::serde::de::Error,
            {
                 // Use the unique enum name
                match value {
                    #(#field_match_arms),*
                }
            }
             // Handle borrowed strings as well
            fn visit_borrowed_str<E>(self, value: &'de str) -> Result<Self::Value, E>
            where E: ::serde::de::Error,
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
            where D: ::serde::Deserializer<'de>,
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

        // Declare the final variable matching the struct field type
        visitor_field_defs.push(quote! { let mut #field_ident: #field_ty = None; });

        if info.is_element {
            // Declare temporary variables for element parts
            let inner_ty = info.inner_ty; // Element<V, E> or DecimalElement<E>
             // Extract V and E types from Element<V, E> or DecimalElement<E>
            let (val_ty, ext_ty) = if let Type::Path(type_path) = inner_ty {
                if type_path.path.segments.len() == 1 {
                    let segment = &type_path.path.segments[0];
                    if let PathArguments::AngleBracketed(args) = &segment.arguments {
                        if segment.ident == "Element" && args.args.len() == 2 {
                            let v_arg = &args.args[0]; let e_arg = &args.args[1];
                            let v_type = match v_arg { GenericArgument::Type(t) => t, _ => panic!("Expected Type for V") };
                            let e_type = match e_arg { GenericArgument::Type(t) => t, _ => panic!("Expected Type for E") };
                            (v_type.clone(), e_type.clone())
                        } else if segment.ident == "DecimalElement" && args.args.len() == 1 {
                            let precise_decimal_type = syn::parse_str::<Type>("crate::PreciseDecimal").unwrap();
                            let e_arg = &args.args[0];
                            let e_type = match e_arg { GenericArgument::Type(t) => t, _ => panic!("Expected Type for E") };
                            (precise_decimal_type, e_type.clone())
                        } else { panic!("Unsupported Element type structure: {}", quote!(#inner_ty).to_string()); }
                    } else { panic!("Element type missing generics: {}", quote!(#inner_ty).to_string()); }
                } else { panic!("Unsupported Element type path: {}", quote!(#inner_ty).to_string()); }
            } else { panic!("Expected Element or DecimalElement type, found: {}", quote!(#inner_ty).to_string()); };

            let val_field = format_ident!("{}_value", field_ident);
            let id_field = format_ident!("{}_id", field_ident);
            let ext_field = format_ident!("{}_extension", field_ident);

            visitor_field_defs.push(quote! { let mut #val_field: Option<#val_ty> = None; });
            visitor_field_defs.push(quote! { let mut #id_field: Option<String> = None; });
            visitor_field_defs.push(quote! { let mut #ext_field: Option<Vec<#ext_ty>> = None; });
        }
    }


    // Generate map assignments (assign to temp vars or final vars)
    for info in &field_infos {
        let field_ident = info.ident;
        let inner_ty = info.inner_ty; // Type inside Option (e.g., Element<String, E>)

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
             if let Some(first) = camel_case.chars().next() { if !first.is_uppercase() { camel_case.insert(0, first.to_ascii_uppercase()); camel_case.remove(1); } } else { camel_case = format!("Field{}", field_ident); }
            camel_case
        };
        let field_ident_enum = format_ident!("{}", field_ident_enum_str); // e.g., BirthDate
        let underscore_ident_enum = format_ident!("{}Underscore", field_ident_enum_str); // e.g., BirthDateUnderscore


        if info.is_element {
            // For Element types, we need Option<Value>, Option<Id>, Option<Extension>
            let id_field = format_ident!("{}_id", field_ident); // Keep one definition
            // let id_field = format_ident!("{}_id", field_ident); // Remove duplicate definition
            let ext_field = format_ident!("{}_extension", field_ident);
            let val_field = format_ident!("{}_value", field_ident);

             // Extract E type for the helper struct
            let ext_ty = if let Type::Path(type_path) = inner_ty {
                if type_path.path.segments.len() == 1 {
                    let segment = &type_path.path.segments[0];
                    if let PathArguments::AngleBracketed(args) = &segment.arguments {
                         if (segment.ident == "Element" && args.args.len() == 2) || (segment.ident == "DecimalElement" && args.args.len() == 1) {
                            let e_arg = if segment.ident == "Element" { &args.args[1] } else { &args.args[0] };
                            match e_arg { GenericArgument::Type(t) => t.clone(), _ => panic!("Expected Type for E") }
                        } else { panic!("Unsupported Element type structure: {}", quote!(#inner_ty).to_string()); }
                    } else { panic!("Element type missing generics: {}", quote!(#inner_ty).to_string()); }
                } else { panic!("Unsupported Element type path: {}", quote!(#inner_ty).to_string()); }
            } else { panic!("Expected Element or DecimalElement type, found: {}", quote!(#inner_ty).to_string()); };


            // Assign to temporary _value field
            let original_name_lit = LitStr::new(&info.original_name, Span::call_site());
            visitor_map_assignments.push(quote! {
               #field_enum_name::#field_ident_enum => {
                   if #val_field.is_some() { return Err(::serde::de::Error::duplicate_field(#original_name_lit)); }
                   #val_field = Some(map.next_value()?);
               }
            });

            // Assign to temporary _id and _extension fields
            let underscore_name_lit = LitStr::new(&info.underscore_name, Span::call_site());
            visitor_map_assignments.push(quote! {
                 #field_enum_name::#underscore_ident_enum => {
                    let helper: #extension_helper_name<#ext_ty> = map.next_value()?;
                    if #id_field.is_some() || #ext_field.is_some() { return Err(::serde::de::Error::duplicate_field(#underscore_name_lit)); }
                    #id_field = helper.id;
                    #ext_field = helper.extension;
                }
            });

        } else {
            // Assign directly to the final field variable for non-elements
            let original_name_lit = LitStr::new(&info.original_name, Span::call_site());
            visitor_map_assignments.push(quote! {
               #field_enum_name::#field_ident_enum => {
                   if #field_ident.is_some() { return Err(::serde::de::Error::duplicate_field(#original_name_lit)); }
                   #field_ident = Some(map.next_value()?); // Assign to final variable #field_ident
               }
            });
        }
    }

    // Generate the final struct construction logic (remains the same)
    let final_struct_fields: Vec<_> = field_infos.iter().map(|info| {
        let field_ident = info.ident;
        quote! { #field_ident: #field_ident }
    }).collect();


    // visitor_struct_name already defined above with unique name

    // Element construction logic generation (moved back outside)
    let element_construction_logic: Vec<_> = field_infos.iter().filter_map(|info| {
        if info.is_element {
            let field_ident = info.ident; // Final field ident (e.g., birth_date)
            let inner_ty = info.inner_ty; // Type: Element<String, E>
            // Idents for temporary variables *inside* visit_map
            let val_field_ident = format_ident!("{}_value", field_ident);
            let id_field_ident = format_ident!("{}_id", field_ident);
            let ext_field_ident = format_ident!("{}_extension", field_ident);
            Some(quote! {
                // This generated code will be placed inside visit_map
                // It references #field_ident (final variable) and the temporary variables
                #field_ident = if #val_field_ident.is_some() || #id_field_ident.is_some() || #ext_field_ident.is_some() {
                    Some(#inner_ty { value: #val_field_ident, id: #id_field_ident, extension: #ext_field_ident })
                } else { None };
            })
        } else {
            None
        }
    }).collect();


    let deserialize_impl = quote! {
        // No helper types defined outside the impl block anymore

        impl<'de> ::serde::Deserialize<'de> for #name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: ::serde::Deserializer<'de>,
            {
                // Define ALL helper types *inside* the deserialize function's scope
                // This ensures they have access to generic parameters if needed and avoids name collisions

                // 1. Field Enum and its Visitor/Deserialize impl
                #field_enum // Contains the enum definition
                #field_visitor_impl // Contains the visitor struct and Deserialize impl for the enum

                // 2. Extension Helper Struct (for _fieldName)
                #[derive(::serde::Deserialize)] // Keep ::serde:: here
                struct #extension_helper_name<E> {
                     #[serde(default)]
                     id: Option<String>,
                     #[serde(default)]
                     extension: Option<Vec<E>>,
                }

                // 3. Main Visitor Struct for the type #name
                struct #visitor_struct_name;

                impl<'de> ::serde::de::Visitor<'de> for #visitor_struct_name {
                    type Value = #name;

                    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                        formatter.write_str(concat!("struct ", #struct_name_str))
                    }

                    fn visit_map<V>(self, mut map: V) -> Result<#name, V::Error>
                    where
                        V: ::serde::de::MapAccess<'de>,
                    {
                        // Initialize Option fields for the visitor
                        #(#visitor_field_defs;)*

                        // Loop through fields in the JSON map
                        while let Some(key) = map.next_key::<#field_enum_name>()? { // Use unique enum name
                            match key {
                                #(#visitor_map_assignments)*
                                #field_enum_name::Ignore => { let _ = map.next_value::<::serde::de::IgnoredAny>()?; } // Use unique enum name
                            }
                        }

                        // Execute the construction logic (generated outside)
                        #(#element_construction_logic)*


                        // Construct the final struct using the final field variables
                        Ok(#name {
                            #(#final_struct_fields),*
                        })
                    }
                }

                // Define the fields Serde should expect
                const FIELDS: &'static [&'static str] = &[#(#field_strings),*];
                // Start deserialization using the main visitor struct defined above
                deserializer.deserialize_struct(#struct_name_str, FIELDS, #visitor_struct_name)
            }
        }
    };

    // --- Combine Serialize and Deserialize ---
    let serialize_impl = quote! {
        impl ::serde::Serialize for #name { // Revert to ::serde::
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: ::serde::Serializer, // Revert to ::serde::
            {
                // Add use statements for serde traits/types needed within the impl
                use serde::ser::{SerializeStruct, Serializer}; // Add use statement

                // Calculate the number of fields to serialize
                let mut count = 0;
                #(#field_count_calculation)*

                // Start serialization
                let mut state = serializer.serialize_struct(stringify!(#name), count)?;

                // Serialize each field
                #(#serialize_fields)*

                state.end()
            }
        }
    };

    // Combine implementations
    let expanded = quote! {
        #serialize_impl
        #deserialize_impl // Add the deserialize implementation
    };

    // For debugging: Print the generated code
    // println!("{}", expanded.to_string());

    expanded.into()
}
