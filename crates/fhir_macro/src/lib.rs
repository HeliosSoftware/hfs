use proc_macro::TokenStream;
use quote::quote; // Removed format_ident
use syn::{parse_macro_input, DeriveInput, Data, Fields, Type, PathArguments, GenericArgument, LitStr, Meta, NestedMeta}; // Removed Ident, Added Meta, NestedMeta
use proc_macro2::Span;

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

    // --- Generate Deserialize Implementation (Placeholder for now) ---
    // TODO: Implement the complex deserialization logic
    // Mark as unused for now
    let _deserialize_impl = quote! {
         // Placeholder - Use standard derive for now until custom logic is implemented
         // This will NOT handle the _fieldName correctly yet.
         #[derive(::serde::Deserialize)]
         #[serde(rename = "PLACEHOLDER_STRUCT_NAME")] // Avoid conflict
         struct #name; // This needs to be replaced with the actual struct definition + custom impl

         // We need a custom Deserialize impl here that uses a visitor
         // to handle fieldName and _fieldName merging.
         // This is complex and will be added in the next step.
    };


    // Combine implementations
    let expanded = quote! {
        #serialize_impl
        // #_deserialize_impl // Add this back once implemented
    };

    expanded.into()
}
