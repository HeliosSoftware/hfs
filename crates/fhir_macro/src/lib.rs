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
                            #[derive(Serialize)]
                            struct ExtensionHelper<'a, E: Serialize> {
                                #[serde(skip_serializing_if = "Option::is_none")]
                                id: &'a Option<String>,
                                #[serde(skip_serializing_if = "Option::is_none")]
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

    // 1. Define Field enum
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
        is_option: bool,
        inner_ty: &'a Type, // Type inside Option if applicable, otherwise same as ty
    }
    let mut field_infos = Vec::new();

    for (idx, field) in fields.iter().enumerate() {
        let field_ident = field.ident.as_ref().unwrap();
        let field_ty = &field.ty;
        let original_name = get_original_field_name(field);
        let underscore_name = format!("_{}", original_name);

        let field_ident_enum = format_ident!("{}", field_ident.to_string().to_uppercase());
        let underscore_ident_enum = format_ident!("__{}", field_ident.to_string().to_uppercase()); // Prefix with __ to avoid clashes if field starts with _

        let (is_option, inner_ty) = match get_option_inner_type(field_ty) {
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
            is_option,
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
    // Add catch-all arm for unknown fields
    field_match_arms.push(quote! { _ => Ok(Field::Ignore) });

    let field_enum = quote! {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        enum Field { #(#field_enum_variants),* }
    };

    // 2. Implement Visitor trait for Field enum
    let field_visitor_impl = quote! {
        struct FieldVisitor;

        impl<'de> ::serde::de::Visitor<'de> for FieldVisitor {
            type Value = Field;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a field identifier")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where E: ::serde::de::Error,
            {
                match value {
                    #(#field_match_arms),*
                }
            }
             // Handle borrowed strings as well
            fn visit_borrowed_str<E>(self, value: &'de str) -> Result<Self::Value, E>
            where E: ::serde::de::Error,
            {
                 match value {
                    #(#field_match_arms),*
                }
            }
        }

        impl<'de> ::serde::Deserialize<'de> for Field {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where D: ::serde::Deserializer<'de>,
            {
                deserializer.deserialize_identifier(FieldVisitor)
            }
        }
    };

    // 3. Generate Visitor struct and its implementation
    let mut visitor_field_defs = Vec::new();
    let mut visitor_map_assignments = Vec::new();
    let mut visitor_build_steps = Vec::new();
    let struct_name_str = name.to_string();

    for info in &field_infos {
        let field_ident = info.ident;
        let field_ty = info.ty; // Original type (e.g., Option<Element<String, E>>)
        let inner_ty = info.inner_ty; // Type inside Option (e.g., Element<String, E>)
        let field_ident_enum = format_ident!("{}", field_ident.to_string().to_uppercase());

        if info.is_element {
            // For Element types, we need Option<Value>, Option<Id>, Option<Extension>
            let underscore_ident_enum =
                format_ident!("__{}", field_ident.to_string().to_uppercase());
            let id_field = format_ident!("{}_id", field_ident);
            let ext_field = format_ident!("{}_extension", field_ident);
            let val_field = format_ident!("{}_value", field_ident);

            // Extract V and E from Element<V, E> or DecimalElement<E>
            let (val_ty_token, ext_ty_token) = if let Type::Path(type_path) = inner_ty {
                if type_path.path.segments.len() == 1 {
                    let segment = &type_path.path.segments[0];
                    if let PathArguments::AngleBracketed(args) = &segment.arguments {
                        if segment.ident == "Element" && args.args.len() == 2 {
                            (args.args[0].clone(), args.args[1].clone())
                        } else if segment.ident == "DecimalElement" && args.args.len() == 1 {
                            // For DecimalElement, value type is PreciseDecimal
                            let precise_decimal_type =
                                syn::parse_str::<Type>("crate::PreciseDecimal").unwrap();
                            (
                                GenericArgument::Type(precise_decimal_type),
                                args.args[0].clone(),
                            )
                        } else {
                            // Use quote!(...).to_string() for formatting Type
                            panic!("Unsupported Element type structure: {}", quote!(#inner_ty).to_string());
                        }
                    } else {
                         // Use quote!(...).to_string() for formatting Type
                         panic!("Element type missing generics: {}", quote!(#inner_ty).to_string());
                    }
                } else {
                     // Use quote!(...).to_string() for formatting Type
                     panic!("Unsupported Element type path: {}", quote!(#inner_ty).to_string());
                }
            } else {
                 // Use quote!(...).to_string() for formatting Type
                 panic!(
                    "Expected Element or DecimalElement type, found: {}",
                    quote!(#inner_ty).to_string()
                );
            };

            visitor_field_defs.push(quote! { #val_field: Option<#val_ty_token> = None });
            visitor_field_defs.push(quote! { #id_field: Option<String> = None });
            visitor_field_defs.push(quote! { #ext_field: Option<Vec<#ext_ty_token>> = None });

            // Deserialize the value part (fieldName)
            // Create LitStr for field name interpolation in error messages
            let original_name_lit = LitStr::new(&info.original_name, Span::call_site());
            visitor_map_assignments.push(quote! {
               Field::#field_ident_enum => {
                   // Use the LitStr for duplicate_field
                   if #val_field.is_some() { return Err(::serde::de::Error::duplicate_field(#original_name_lit)); }
                   #val_field = Some(map.next_value()?);
               }
            });

            // Deserialize the extension part (_fieldName)
            // Create LitStr for field name interpolation in error messages
            let underscore_name_lit = LitStr::new(&info.underscore_name, Span::call_site());
            let ext_ty_token_clone = ext_ty_token.clone(); // Clone for use in helper struct definition
            visitor_map_assignments.push(quote! {
                 Field::#underscore_ident_enum => {
                    // Define a helper struct to deserialize id and extension
                    // Use #ext_ty_token_clone here
                    #[derive(Deserialize)]
                    struct ExtensionHelper<E> {
                        id: Option<String>,
                        id: Option<String>,
                        extension: Option<Vec<E>>,
                    }
                    // Deserialize the helper struct
                    let helper: ExtensionHelper<#ext_ty_token_clone> = map.next_value()?; // Use #ext_ty_token_clone here too
                    // Check for duplicates before assigning
                    // Use the LitStr for duplicate_field
                    if #id_field.is_some() || #ext_field.is_some() { return Err(::serde::de::Error::duplicate_field(#underscore_name_lit)); }
                    #id_field = helper.id;
                    #ext_field = helper.extension;
                }
            });

            // Build the final Element/DecimalElement
            visitor_build_steps.push(quote! {
                let #field_ident = if #val_field.is_some() || #id_field.is_some() || #ext_field.is_some() {
                    Some(#inner_ty {
                        value: #val_field,
                        id: #id_field,
                        extension: #ext_field,
                    })
                } else {
                    None
                };
            });
        } else {
            // Regular field (might be Option<T> or just T)
            visitor_field_defs.push(quote! { #field_ident: Option<#inner_ty> = None }); // Always use Option<Inner> in visitor for simplicity
            // Create LitStr for field name interpolation in error messages
            let original_name_lit = LitStr::new(&info.original_name, Span::call_site());
            visitor_map_assignments.push(quote! {
               Field::#field_ident_enum => {
                   // Use the LitStr for duplicate_field
                   if #field_ident.is_some() { return Err(::serde::de::Error::duplicate_field(#original_name_lit)); }
                   #field_ident = Some(map.next_value()?);
               }
            });
            // No build step needed, just use the value directly
            // If the original field was not Option, we need to unwrap or handle None later
            if info.is_option {
                visitor_build_steps.push(quote! { let #field_ident = #field_ident; }); // Already Option<T>
            } else {
                // If original field was T, unwrap the Option<T> from visitor or error if None
                // Create LitStr for field name interpolation in error messages
                let original_name_lit = LitStr::new(&info.original_name, Span::call_site());
                visitor_build_steps.push(quote! {
                     // Use the LitStr for missing_field
                     let #field_ident = #field_ident.ok_or_else(|| ::serde::de::Error::missing_field(#original_name_lit))?;
                 });
            }
        }
    }

    let visitor_struct_name = format_ident!("{}Visitor", name);

    let deserialize_impl = quote! {
        #field_enum
        #field_visitor_impl

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
                while let Some(key) = map.next_key::<Field>()? {
                    match key {
                        #(#visitor_map_assignments)*
                        Field::Ignore => { let _ = map.next_value::<::serde::de::IgnoredAny>()?; }
                    }
                }

                // Combine parts for Element fields and check required fields
                #(#visitor_build_steps)*

                // Construct the final struct
                Ok(#name {
                    #(#field_infos.iter().map(|info| info.ident)),*
                })
            }
        }

        impl<'de> ::serde::Deserialize<'de> for #name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: ::serde::Deserializer<'de>,
            {
                // Define the fields Serde should expect
                const FIELDS: &'static [&'static str] = &[#(#field_strings),*];
                deserializer.deserialize_struct(#struct_name_str, FIELDS, #visitor_struct_name)
            }
        }
    };

    // --- Combine Serialize and Deserialize ---
    let serialize_impl = quote! {
        impl ::serde::Serialize for #name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: ::serde::Serializer,
            {
                use ::serde::ser::SerializeStruct; // Ensure SerializeStruct is in scope

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
