use heck::ToLowerCamelCase;
use quote::quote;
use syn::{Lit, Meta, punctuated::Punctuated, token};

/// Generates an owned IdAndExtensionHelper with full deserialization support.
///
/// This helper is used for deserialization to handle FHIR's `_fieldName` pattern.
/// It has owned fields and includes both serialization and deserialization.
pub(crate) fn generate_id_and_extension_helper_owned() -> proc_macro2::TokenStream {
    quote! {
        // Helper struct for deserializing the id/extension part from _fieldName
        #[derive(Clone, Default)]
        struct IdAndExtensionHelper {
            id: Option<std::string::String>,
            extension: Option<Vec<Extension>>,
        }

        impl ::helios_serde::FhirSerialize<::helios_serde::Json> for IdAndExtensionHelper {
            fn fhir_serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                use serde::ser::SerializeStruct;
                let mut state = serializer.serialize_struct("IdAndExtensionHelper", 2)?;
                if self.id.is_some() {
                    state.serialize_field("id", &self.id)?;
                }
                if let Some(ref ext) = self.extension {
                    let ctx = ::helios_serde::SerializationContext::json(ext);
                    state.serialize_field("extension", &ctx)?;
                }
                state.end()
            }
        }

        impl ::helios_serde::FhirDeserialize<::helios_serde::Json> for IdAndExtensionHelper {
            fn fhir_deserialize<'de, D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                use serde::de::{self, Visitor, MapAccess, DeserializeSeed};

                struct HelperVisitor;

                impl<'de> Visitor<'de> for HelperVisitor {
                    type Value = IdAndExtensionHelper;

                    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                        formatter.write_str("a map with optional id and extension fields")
                    }

                    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
                    where
                        A: MapAccess<'de>,
                    {
                        let mut id = None;
                        let mut extension = None;

                        while let Some(key) = map.next_key::<std::string::String>()? {
                            match key.as_str() {
                                "id" => {
                                    id = Some(map.next_value()?);
                                }
                                "extension" => {
                                    let ctx = ::helios_serde::DeserializationContext::<Vec<Extension>, ::helios_serde::Json>::json();
                                    extension = Some(map.next_value_seed(ctx)?);
                                }
                                _ => {
                                    let _ = map.next_value::<serde::de::IgnoredAny>()?;
                                }
                            }
                        }

                        Ok(IdAndExtensionHelper { id, extension })
                    }
                }

                deserializer.deserialize_map(HelperVisitor)
            }
        }
    }
}

/// Generates a borrowed IdAndExtensionHelperRef for serialization.
///
/// This helper avoids allocating new `String`/`Vec` instances by holding references
/// to the existing `id` and `extension` data when producing the `_fieldName` objects.
pub(crate) fn generate_id_and_extension_helper_ref() -> proc_macro2::TokenStream {
    quote! {
        #[allow(non_camel_case_types)]
        struct IdAndExtensionHelperRef<'a, Id, Ext> {
            id: Option<&'a Id>,
            extension: Option<&'a Ext>,
        }

        impl<'a, Id, Ext> IdAndExtensionHelperRef<'a, Id, Ext> {
            fn new(id: Option<&'a Id>, extension: Option<&'a Ext>) -> Option<Self> {
                if id.is_none() && extension.is_none() {
                    None
                } else {
                    Some(Self { id, extension })
                }
            }
        }

        impl<'a, Id, Ext> serde::Serialize for IdAndExtensionHelperRef<'a, Id, Ext>
        where
            Id: serde::Serialize,
            for<'b> ::helios_serde::SerializationContext<&'b Ext, ::helios_serde::Json>: serde::Serialize,
        {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                use serde::ser::SerializeMap;
                let mut len = 0;
                if self.id.is_some() {
                    len += 1;
                }
                if self.extension.is_some() {
                    len += 1;
                }

                let mut map = serializer.serialize_map(Some(len))?;
                if let Some(id) = self.id {
                    map.serialize_entry("id", id)?;
                }
                if let Some(extension) = self.extension {
                    let ctx = ::helios_serde::SerializationContext::json(extension);
                    map.serialize_entry("extension", &ctx)?;
                }
                map.end()
            }
        }
    }
}

/// Determines the effective field name for FHIR serialization.
///
/// This function extracts the field name that should be used during JSON serialization,
/// respecting FHIR naming conventions and custom rename attributes.
///
/// # Attribute Processing
///
/// - If `#[fhir_serde(rename = "customName")]` is present, uses the custom name
/// - Otherwise, converts the Rust field name from `snake_case` to `camelCase`
///
/// # Arguments
///
/// * `field` - The field definition from the parsed struct
///
/// # Returns
///
/// The field name as it should appear in the serialized JSON.
///
/// # Examples
///
/// ```rust,ignore
/// // Field: pub implicit_rules: Option<Uri>
/// // Result: "implicitRules" (camelCase conversion)
///
/// // Field: #[fhir_serde(rename = "modifierExtension")]
/// //        pub modifier_extension: Option<Vec<Extension>>
/// // Result: "modifierExtension" (explicit rename)
/// ```
pub(crate) fn get_effective_field_name(field: &syn::Field) -> String {
    for attr in &field.attrs {
        if attr.path().is_ident("fhir_serde")
            && let Ok(list) =
                attr.parse_args_with(Punctuated::<Meta, token::Comma>::parse_terminated)
        {
            for meta in list {
                if let Meta::NameValue(nv) = meta
                    && nv.path.is_ident("rename")
                    && let syn::Expr::Lit(expr_lit) = nv.value
                    && let Lit::Str(lit_str) = expr_lit.lit
                {
                    return lit_str.value();
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

/// Checks if a field should be flattened during serialization.
///
/// This function determines whether a field has the `#[fhir_serde(flatten)]` attribute,
/// which indicates that the field's contents should be serialized directly into the
/// parent object rather than as a nested object.
///
/// # FHIR Usage
///
/// Flattening is commonly used for:
/// - **Choice types**: FHIR `[x]` fields that can be one of several types
/// - **Inheritance**: Base class fields that should appear at the same level
/// - **Resource polymorphism**: Fields that contain different resource types
///
/// # Arguments
///
/// * `field` - The field definition to check for the flatten attribute
///
/// # Returns
///
/// `true` if the field has `#[fhir_serde(flatten)]`, `false` otherwise.
///
/// # Examples
///
/// ```rust,ignore
/// // Regular field (not flattened)
/// pub name: Option<String>,  // false
///
/// // Flattened choice type field
/// #[fhir_serde(flatten)]
/// pub subject: Option<ActivityDefinitionSubject>,  // true
/// ```
pub(crate) fn is_flattened(field: &syn::Field) -> bool {
    for attr in &field.attrs {
        if attr.path().is_ident("fhir_serde")
            && let Ok(list) =
                attr.parse_args_with(Punctuated::<Meta, token::Comma>::parse_terminated)
        {
            for meta in list {
                if let Meta::Path(path) = meta
                    && path.is_ident("flatten")
                {
                    return true;
                }
            }
        }
    }
    false
}

/// Extracts the tag field name from enum-level #[fhir_serde(tag = "field")] attribute.
///
/// This function checks if an enum has a tag attribute for internally-tagged serialization,
/// which is used for enums like Resource where the variant is determined by a field value
/// (e.g., "resourceType": "Patient").
///
/// # Arguments
///
/// * `attrs` - The enum's attributes to search
///
/// # Returns
///
/// `Some(String)` with the tag field name if present, `None` otherwise.
///
/// # Examples
///
/// ```rust,ignore
/// // Enum with internally-tagged serialization
/// #[derive(FhirSerde)]
/// #[fhir_serde(tag = "resourceType")]
/// pub enum Resource {
///     Patient(Patient),
///     Observation(Observation),
/// }
/// ```
pub(crate) fn get_enum_tag(attrs: &[syn::Attribute]) -> Option<String> {
    for attr in attrs {
        if attr.path().is_ident("fhir_serde")
            && let Ok(list) =
                attr.parse_args_with(Punctuated::<Meta, token::Comma>::parse_terminated)
        {
            for meta in list {
                if let Meta::NameValue(nv) = meta
                    && nv.path.is_ident("tag")
                    && let syn::Expr::Lit(expr_lit) = nv.value
                    && let Lit::Str(lit_str) = expr_lit.lit
                {
                    return Some(lit_str.value());
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;
    use syn::{Data, DeriveInput, Fields};
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
}
