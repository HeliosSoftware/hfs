use super::common::{generate_id_and_extension_helper_ref, get_effective_field_name, is_flattened};
use crate::util::{get_element_info, get_option_inner_type, get_vec_inner_type};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Data, Fields, Ident, Lit, Meta, punctuated::Punctuated, token};

/// Generates the `serde::Serialize` implementation for FHIR types.
///
/// This function is the core of FHIR serialization code generation, producing
/// implementations that handle all the complex FHIR serialization patterns including
/// the extension pattern, choice types, and array serialization.
///
/// # Generated Code Patterns
///
/// ## For Structs:
/// - **Extension Pattern**: Separates primitive values and extension metadata
/// - **Array Handling**: Splits arrays into primitive and extension arrays
/// - **Field Counting**: Dynamically calculates field count for serializer
/// - **Conditional Serialization**: Only serializes non-empty fields
/// - **Flattening Support**: Handles `#[fhir_serde(flatten)]` attributes
///
/// ## For Enums:
/// - **Choice Type Serialization**: Single key-value pair output
/// - **Extension Support**: Handles element-containing enum variants
/// - **Variant Renaming**: Applies `#[fhir_serde(rename)]` attributes
///
/// # FHIR-Specific Serialization
///
/// The generated code handles several FHIR-specific patterns:
///
/// 1. **Element Extension Pattern**:
///    ```json
///    { "field": "value", "_field": {"id": "...", "extension": []} }
///    ```
///
/// 2. **Array Split Pattern**:
///    ```json
///    { "items": ["a", null, "c"], "_items": [null, {"id": "b"}, null] }
///    ```
///
/// 3. **Choice Type Pattern**:
///    ```json
///    { "valueString": "text" }  // not { "value": {"String": "text"} }
///    ```
///
/// # Arguments
///
/// * `data` - The parsed data structure (struct or enum)
/// * `name` - The type name being generated for
///
/// # Returns
///
/// TokenStream containing the complete `serialize` method implementation.
pub(crate) fn generate_serialize_impl(
    data: &Data,
    name: &Ident,
    ty_generics: &syn::TypeGenerics,
) -> TokenStream {
    match *data {
        Data::Enum(ref data) => {
            // Regular enum serialization (not internally-tagged)
            let mut match_arms = Vec::new();
            let mut needs_primitive_helper = false;

            for variant in &data.variants {
                let variant_name = &variant.ident;

                // Get the rename attribute if present
                let mut rename = None;
                for attr in &variant.attrs {
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
                                rename = Some(lit_str.value());
                            }
                        }
                    }
                }

                // Use the rename value or the variant name as a string
                let variant_key = rename.unwrap_or_else(|| variant_name.to_string());

                // Handle different variant field types
                match &variant.fields {
                    Fields::Unnamed(fields) if fields.unnamed.len() == 1 => {
                        // Newtype variant (e.g., String(String))
                        let field = fields.unnamed.first().unwrap();
                        let field_ty = &field.ty;

                        // Check if this is a primitive type that might have extensions
                        let (is_element, is_decimal_element, _, _) = get_element_info(field_ty);

                        if is_element || is_decimal_element {
                            needs_primitive_helper = true;
                            // For Element types, we need special handling for the _fieldName pattern
                            let underscore_variant_key = format!("_{}", variant_key);

                            match_arms.push(quote! {
                                #name #ty_generics::#variant_name(value) => {
                                    if let Some(inner_value) = value.value.as_ref() {
                                        let ctx = ::helios_serde::SerializationContext::json(inner_value);
                                        state.serialize_entry(#variant_key, &ctx)?;
                                    }
                                    if let Some(ext_value) = IdAndExtensionHelperRef::new(
                                        value.id.as_ref(),
                                        value.extension.as_ref(),
                                    ) {
                                        state.serialize_entry(#underscore_variant_key, &ext_value)?;
                                    }
                                }
                            });
                        } else {
                            // Regular newtype variant - wrap in SerializationContext
                            match_arms.push(quote! {
                                #name #ty_generics::#variant_name(value) => {
                                    let ctx = ::helios_serde::SerializationContext::json(value);
                                    state.serialize_entry(#variant_key, &ctx)?;
                                }
                            });
                        }
                    }
                    Fields::Unnamed(_) => {
                        // Tuple variant with multiple fields
                        match_arms.push(quote! {
                            #name #ty_generics::#variant_name(value) => {
                                let ctx = ::helios_serde::SerializationContext::json(value);
                                state.serialize_entry(#variant_key, &ctx)?;
                            }
                        });
                    }
                    Fields::Named(_fields) => {
                        // Struct variant
                        match_arms.push(quote! {
                            variant @ #name #ty_generics::#variant_name { .. } => {
                                let ctx = ::helios_serde::SerializationContext::json(variant);
                                state.serialize_entry(#variant_key, &ctx)?;
                            }
                        });
                    }
                    Fields::Unit => {
                        // Unit variant
                        match_arms.push(quote! {
                            #name #ty_generics::#variant_name => {
                                state.serialize_entry(#variant_key, &())?;
                            }
                        });
                    }
                }
            }

            let primitive_helper_definition = if needs_primitive_helper {
                generate_id_and_extension_helper_ref()
            } else {
                TokenStream::new()
            };

            // Generate the enum serialization implementation
            quote! {
                #primitive_helper_definition
                // Count the number of fields to serialize (always 1 for an enum variant)
                let count = 1;

                // Import SerializeMap trait to access serialize_entry method
                use serde::ser::SerializeMap;

                // Create a serialization state
                let mut state = serializer.serialize_map(Some(count))?;

                // Match on self to determine which variant to serialize
                match self {
                    #(#match_arms)*
                }

                // End the map serialization
                state.end()
            }
        }
        Data::Struct(ref data) => {
            match data.fields {
                Fields::Named(ref fields) => {
                    // Check if any fields have the flatten attribute - define this at the top level
                    let has_flattened_fields = fields.named.iter().any(is_flattened);
                    let needs_empty_helper = fields.named.iter().any(|field| {
                        if is_flattened(field) {
                            return false;
                        }
                        let (is_element, is_decimal_element, is_option, is_vec) =
                            get_element_info(&field.ty);
                        let is_fhir_element = is_element || is_decimal_element;
                        !is_option && !is_vec && !is_fhir_element
                    });
                    let needs_primitive_helper = fields.named.iter().any(|field| {
                        let field_ty = &field.ty;
                        let (is_element, is_decimal_element, _, _) = get_element_info(field_ty);
                        is_element || is_decimal_element
                    });

                    // Import SerializeMap trait if we have flattened fields
                    let import_serialize_map = if has_flattened_fields {
                        quote! { use serde::ser::SerializeMap; }
                    } else {
                        quote! { use serde::ser::SerializeStruct; }
                    };

                    let primitive_helper_definition = if needs_primitive_helper {
                        generate_id_and_extension_helper_ref()
                    } else {
                        TokenStream::new()
                    };
                    let empty_helper_definition = if needs_empty_helper {
                        empty_check_helper_definition_tokens()
                    } else {
                        TokenStream::new()
                    };

                    let flatten_helper_definition = if has_flattened_fields {
                        quote! {
                            #[inline(always)]
                            fn __helios_serde_flatten_into_map<S, T>(
                                state: &mut S,
                                value: &T,
                                field_name: &'static str,
                            ) -> Result<(), S::Error>
                            where
                                S: serde::ser::SerializeMap,
                                T: serde::Serialize,
                            {
                                struct __HeliosFlattenSerializer<'a, S> {
                                    state: &'a mut S,
                                    field_name: &'static str,
                                }

                                impl<'a, S> __HeliosFlattenSerializer<'a, S>
                                where
                                    S: serde::ser::SerializeMap,
                                {
                                    fn unsupported(&self, kind: &str) -> S::Error {
                                        serde::ser::Error::custom(format!(
                                            "Flattened field '{}' serialized as {}, but flatten requires an object",
                                            self.field_name,
                                            kind
                                        ))
                                    }
                                }

                                impl<'a, S> serde::ser::Serializer for __HeliosFlattenSerializer<'a, S>
                                where
                                    S: serde::ser::SerializeMap,
                                {
                                    type Ok = ();
                                    type Error = S::Error;
                                    type SerializeSeq = serde::ser::Impossible<Self::Ok, Self::Error>;
                                    type SerializeTuple = serde::ser::Impossible<Self::Ok, Self::Error>;
                                    type SerializeTupleStruct = serde::ser::Impossible<Self::Ok, Self::Error>;
                                    type SerializeTupleVariant = serde::ser::Impossible<Self::Ok, Self::Error>;
                                    type SerializeMap = __HeliosFlattenMap<'a, S>;
                                    type SerializeStruct = __HeliosFlattenStruct<'a, S>;
                                    type SerializeStructVariant = __HeliosFlattenStructVariant<'a, S>;

                                    fn serialize_bool(self, _: bool) -> Result<Self::Ok, Self::Error> {
                                        Err(self.unsupported("a boolean"))
                                    }

                                    fn serialize_i8(self, _: i8) -> Result<Self::Ok, Self::Error> {
                                        Err(self.unsupported("an integer"))
                                    }

                                    fn serialize_i16(self, _: i16) -> Result<Self::Ok, Self::Error> {
                                        Err(self.unsupported("an integer"))
                                    }

                                    fn serialize_i32(self, _: i32) -> Result<Self::Ok, Self::Error> {
                                        Err(self.unsupported("an integer"))
                                    }

                                    fn serialize_i64(self, _: i64) -> Result<Self::Ok, Self::Error> {
                                        Err(self.unsupported("an integer"))
                                    }

                                    fn serialize_u8(self, _: u8) -> Result<Self::Ok, Self::Error> {
                                        Err(self.unsupported("an integer"))
                                    }

                                    fn serialize_u16(self, _: u16) -> Result<Self::Ok, Self::Error> {
                                        Err(self.unsupported("an integer"))
                                    }

                                    fn serialize_u32(self, _: u32) -> Result<Self::Ok, Self::Error> {
                                        Err(self.unsupported("an integer"))
                                    }

                                    fn serialize_u64(self, _: u64) -> Result<Self::Ok, Self::Error> {
                                        Err(self.unsupported("an integer"))
                                    }

                                    fn serialize_f32(self, _: f32) -> Result<Self::Ok, Self::Error> {
                                        Err(self.unsupported("a float"))
                                    }

                                    fn serialize_f64(self, _: f64) -> Result<Self::Ok, Self::Error> {
                                        Err(self.unsupported("a float"))
                                    }

                                    fn serialize_char(self, _: char) -> Result<Self::Ok, Self::Error> {
                                        Err(self.unsupported("a char"))
                                    }

                                    fn serialize_str(self, _: &str) -> Result<Self::Ok, Self::Error> {
                                        Err(self.unsupported("a string"))
                                    }

                                    fn serialize_bytes(self, _: &[u8]) -> Result<Self::Ok, Self::Error> {
                                        Err(self.unsupported("bytes"))
                                    }

                                    fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
                                        Err(self.unsupported("None"))
                                    }

                                    fn serialize_some<U>(self, value: &U) -> Result<Self::Ok, Self::Error>
                                    where
                                        U: ?Sized + serde::Serialize,
                                    {
                                        value.serialize(self)
                                    }

                                    fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
                                        Err(self.unsupported("unit"))
                                    }

                                    fn serialize_unit_struct(
                                        self,
                                        _: &'static str,
                                    ) -> Result<Self::Ok, Self::Error> {
                                        Err(self.unsupported("unit struct"))
                                    }

                                    fn serialize_unit_variant(
                                        self,
                                        _: &'static str,
                                        _: u32,
                                        _: &'static str,
                                    ) -> Result<Self::Ok, Self::Error> {
                                        Err(self.unsupported("unit variant"))
                                    }

                                    fn serialize_newtype_struct<U>(
                                        self,
                                        _: &'static str,
                                        value: &U,
                                    ) -> Result<Self::Ok, Self::Error>
                                    where
                                        U: ?Sized + serde::Serialize,
                                    {
                                        value.serialize(self)
                                    }

                                    fn serialize_newtype_variant<U>(
                                        self,
                                        _: &'static str,
                                        _: u32,
                                        _: &'static str,
                                        value: &U,
                                    ) -> Result<Self::Ok, Self::Error>
                                    where
                                        U: ?Sized + serde::Serialize,
                                    {
                                        value.serialize(self)
                                    }

                                    fn serialize_seq(
                                        self,
                                        _: Option<usize>,
                                    ) -> Result<Self::SerializeSeq, Self::Error> {
                                        Err(self.unsupported("a sequence"))
                                    }

                                    fn serialize_tuple(
                                        self,
                                        _: usize,
                                    ) -> Result<Self::SerializeTuple, Self::Error> {
                                        Err(self.unsupported("a tuple"))
                                    }

                                    fn serialize_tuple_struct(
                                        self,
                                        _: &'static str,
                                        _: usize,
                                    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
                                        Err(self.unsupported("a tuple struct"))
                                    }

                                    fn serialize_tuple_variant(
                                        self,
                                        _: &'static str,
                                        _: u32,
                                        _: &'static str,
                                        _: usize,
                                    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
                                        Err(self.unsupported("a tuple variant"))
                                    }

                                    fn serialize_map(
                                        self,
                                        _: Option<usize>,
                                    ) -> Result<Self::SerializeMap, Self::Error> {
                                        Ok(__HeliosFlattenMap {
                                            state: self.state,
                                            field_name: self.field_name,
                                            pending_key: None,
                                        })
                                    }

                                    fn serialize_struct(
                                        self,
                                        _: &'static str,
                                        _: usize,
                                    ) -> Result<Self::SerializeStruct, Self::Error> {
                                        Ok(__HeliosFlattenStruct {
                                            state: self.state,
                                        })
                                    }

                                    fn serialize_struct_variant(
                                        self,
                                        _: &'static str,
                                        _: u32,
                                        _: &'static str,
                                        _: usize,
                                    ) -> Result<Self::SerializeStructVariant, Self::Error> {
                                        Ok(__HeliosFlattenStructVariant {
                                            state: self.state,
                                        })
                                    }
                                }

                                struct __HeliosFlattenMap<'a, S> {
                                    state: &'a mut S,
                                    field_name: &'static str,
                                    pending_key: Option<::std::string::String>,
                                }

                                impl<'a, S> __HeliosFlattenMap<'a, S>
                                where
                                    S: serde::ser::SerializeMap,
                                {
                                    fn take_key(&mut self) -> Result<::std::string::String, S::Error> {
                                        self.pending_key.take().ok_or_else(|| {
                                            serde::ser::Error::custom(format!(
                                                "Flattened field '{}' attempted to serialize a value before a key",
                                                self.field_name
                                            ))
                                        })
                                    }
                                }

                                impl<'a, S> serde::ser::SerializeMap for __HeliosFlattenMap<'a, S>
                                where
                                    S: serde::ser::SerializeMap,
                                {
                                    type Ok = ();
                                    type Error = S::Error;

                                    fn serialize_key<K>(&mut self, key: &K) -> Result<(), Self::Error>
                                    where
                                        K: ?Sized + serde::Serialize,
                                    {
                                        let mut slot = None;
                                        let serializer = __HeliosFlattenKeySerializer::<S::Error> {
                                            slot: &mut slot,
                                            field_name: self.field_name,
                                            _marker: ::core::marker::PhantomData,
                                        };
                                        key.serialize(serializer)?;
                                        if self.pending_key.is_some() {
                                            return Err(serde::ser::Error::custom(format!(
                                                "Flattened field '{}' serialized a new key before providing a value",
                                                self.field_name
                                            )));
                                        }
                                        self.pending_key = slot;
                                        Ok(())
                                    }

                                    fn serialize_value<V>(&mut self, value: &V) -> Result<(), Self::Error>
                                    where
                                        V: ?Sized + serde::Serialize,
                                    {
                                        let key = self.take_key()?;
                                        self.state.serialize_entry(&key, value)
                                    }

                                    fn serialize_entry<K, V>(
                                        &mut self,
                                        key: &K,
                                        value: &V,
                                    ) -> Result<(), Self::Error>
                                    where
                                        K: ?Sized + serde::Serialize,
                                        V: ?Sized + serde::Serialize,
                                    {
                                        self.state.serialize_entry(key, value)
                                    }

                                    fn end(self) -> Result<Self::Ok, Self::Error> {
                                        if self.pending_key.is_some() {
                                            Err(serde::ser::Error::custom(format!(
                                                "Flattened field '{}' serialized a key without a value",
                                                self.field_name
                                            )))
                                        } else {
                                            Ok(())
                                        }
                                    }
                                }

                                struct __HeliosFlattenStruct<'a, S> {
                                    state: &'a mut S,
                                }

                                impl<'a, S> serde::ser::SerializeStruct for __HeliosFlattenStruct<'a, S>
                                where
                                    S: serde::ser::SerializeMap,
                                {
                                    type Ok = ();
                                    type Error = S::Error;

                                    fn serialize_field<T>(
                                        &mut self,
                                        key: &'static str,
                                        value: &T,
                                    ) -> Result<(), Self::Error>
                                    where
                                        T: ?Sized + serde::Serialize,
                                    {
                                        self.state.serialize_entry(key, value)
                                    }

                                    fn end(self) -> Result<Self::Ok, Self::Error> {
                                        Ok(())
                                    }
                                }

                                struct __HeliosFlattenStructVariant<'a, S> {
                                    state: &'a mut S,
                                }

                                impl<'a, S> serde::ser::SerializeStructVariant
                                    for __HeliosFlattenStructVariant<'a, S>
                                where
                                    S: serde::ser::SerializeMap,
                                {
                                    type Ok = ();
                                    type Error = S::Error;

                                    fn serialize_field<T>(
                                        &mut self,
                                        key: &'static str,
                                        value: &T,
                                    ) -> Result<(), Self::Error>
                                    where
                                        T: ?Sized + serde::Serialize,
                                    {
                                        self.state.serialize_entry(key, value)
                                    }

                                    fn end(self) -> Result<Self::Ok, Self::Error> {
                                        Ok(())
                                    }
                                }

                                struct __HeliosFlattenKeySerializer<'a, E> {
                                    slot: &'a mut Option<::std::string::String>,
                                    field_name: &'static str,
                                    _marker: ::core::marker::PhantomData<E>,
                                }

                                impl<'a, E> __HeliosFlattenKeySerializer<'a, E>
                                where
                                    E: serde::ser::Error,
                                {
                                    fn store(&mut self, value: ::std::string::String) -> Result<(), E> {
                                        if self.slot.is_some() {
                                            return Err(serde::ser::Error::custom(format!(
                                                "Flattened field '{}' attempted to serialize multiple key fragments",
                                                self.field_name
                                            )));
                                        }
                                        *self.slot = Some(value);
                                        Ok(())
                                    }

                                    fn unsupported(&self, kind: &str) -> E {
                                        serde::ser::Error::custom(format!(
                                            "Flattened field '{}' used unsupported key type {}",
                                            self.field_name,
                                            kind
                                        ))
                                    }
                                }

                                impl<'a, E> serde::ser::Serializer for __HeliosFlattenKeySerializer<'a, E>
                                where
                                    E: serde::ser::Error,
                                {
                                    type Ok = ();
                                    type Error = E;
                                    type SerializeSeq = serde::ser::Impossible<Self::Ok, Self::Error>;
                                    type SerializeTuple = serde::ser::Impossible<Self::Ok, Self::Error>;
                                    type SerializeTupleStruct = serde::ser::Impossible<Self::Ok, Self::Error>;
                                    type SerializeTupleVariant = serde::ser::Impossible<Self::Ok, Self::Error>;
                                    type SerializeMap = serde::ser::Impossible<Self::Ok, Self::Error>;
                                    type SerializeStruct = serde::ser::Impossible<Self::Ok, Self::Error>;
                                    type SerializeStructVariant =
                                        serde::ser::Impossible<Self::Ok, Self::Error>;

                                    fn serialize_bool(self, _: bool) -> Result<Self::Ok, Self::Error> {
                                        Err(self.unsupported("bool"))
                                    }

                                    fn serialize_i8(self, _: i8) -> Result<Self::Ok, Self::Error> {
                                        Err(self.unsupported("integer"))
                                    }

                                    fn serialize_i16(self, _: i16) -> Result<Self::Ok, Self::Error> {
                                        Err(self.unsupported("integer"))
                                    }

                                    fn serialize_i32(self, _: i32) -> Result<Self::Ok, Self::Error> {
                                        Err(self.unsupported("integer"))
                                    }

                                    fn serialize_i64(self, _: i64) -> Result<Self::Ok, Self::Error> {
                                        Err(self.unsupported("integer"))
                                    }

                                    fn serialize_u8(self, _: u8) -> Result<Self::Ok, Self::Error> {
                                        Err(self.unsupported("integer"))
                                    }

                                    fn serialize_u16(self, _: u16) -> Result<Self::Ok, Self::Error> {
                                        Err(self.unsupported("integer"))
                                    }

                                    fn serialize_u32(self, _: u32) -> Result<Self::Ok, Self::Error> {
                                        Err(self.unsupported("integer"))
                                    }

                                    fn serialize_u64(self, _: u64) -> Result<Self::Ok, Self::Error> {
                                        Err(self.unsupported("integer"))
                                    }

                                    fn serialize_f32(self, _: f32) -> Result<Self::Ok, Self::Error> {
                                        Err(self.unsupported("float"))
                                    }

                                    fn serialize_f64(self, _: f64) -> Result<Self::Ok, Self::Error> {
                                        Err(self.unsupported("float"))
                                    }

                                    fn serialize_char(
                                        mut self,
                                        value: char,
                                    ) -> Result<Self::Ok, Self::Error> {
                                        let mut buf = [0u8; 4];
                                        self.store(value.encode_utf8(&mut buf).to_string())
                                    }

                                    fn serialize_str(
                                        mut self,
                                        value: &str,
                                    ) -> Result<Self::Ok, Self::Error> {
                                        self.store(value.to_owned())
                                    }

                                    fn serialize_bytes(self, _: &[u8]) -> Result<Self::Ok, Self::Error> {
                                        Err(self.unsupported("bytes"))
                                    }

                                    fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
                                        Err(self.unsupported("None"))
                                    }

                                    fn serialize_some<U>(
                                        self,
                                        value: &U,
                                    ) -> Result<Self::Ok, Self::Error>
                                    where
                                        U: ?Sized + serde::Serialize,
                                    {
                                        value.serialize(self)
                                    }

                                    fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
                                        Err(self.unsupported("unit"))
                                    }

                                    fn serialize_unit_struct(
                                        self,
                                        _: &'static str,
                                    ) -> Result<Self::Ok, Self::Error> {
                                        Err(self.unsupported("unit struct"))
                                    }

                                    fn serialize_unit_variant(
                                        self,
                                        _: &'static str,
                                        _: u32,
                                        _: &'static str,
                                    ) -> Result<Self::Ok, Self::Error> {
                                        Err(self.unsupported("unit variant"))
                                    }

                                    fn serialize_newtype_struct<U>(
                                        self,
                                        _: &'static str,
                                        value: &U,
                                    ) -> Result<Self::Ok, Self::Error>
                                    where
                                        U: ?Sized + serde::Serialize,
                                    {
                                        value.serialize(self)
                                    }

                                    fn serialize_newtype_variant<U>(
                                        self,
                                        _: &'static str,
                                        _: u32,
                                        _: &'static str,
                                        value: &U,
                                    ) -> Result<Self::Ok, Self::Error>
                                    where
                                        U: ?Sized + serde::Serialize,
                                    {
                                        value.serialize(self)
                                    }

                                    fn serialize_seq(
                                        self,
                                        _: Option<usize>,
                                    ) -> Result<Self::SerializeSeq, Self::Error> {
                                        Err(self.unsupported("sequence"))
                                    }

                                    fn serialize_tuple(
                                        self,
                                        _: usize,
                                    ) -> Result<Self::SerializeTuple, Self::Error> {
                                        Err(self.unsupported("tuple"))
                                    }

                                    fn serialize_tuple_struct(
                                        self,
                                        _: &'static str,
                                        _: usize,
                                    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
                                        Err(self.unsupported("tuple struct"))
                                    }

                                    fn serialize_tuple_variant(
                                        self,
                                        _: &'static str,
                                        _: u32,
                                        _: &'static str,
                                        _: usize,
                                    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
                                        Err(self.unsupported("tuple variant"))
                                    }

                                    fn serialize_map(
                                        self,
                                        _: Option<usize>,
                                    ) -> Result<Self::SerializeMap, Self::Error> {
                                        Err(self.unsupported("map"))
                                    }

                                    fn serialize_struct(
                                        self,
                                        _: &'static str,
                                        _: usize,
                                    ) -> Result<Self::SerializeStruct, Self::Error> {
                                        Err(self.unsupported("struct"))
                                    }

                                    fn serialize_struct_variant(
                                        self,
                                        _: &'static str,
                                        _: u32,
                                        _: &'static str,
                                        _: usize,
                                    ) -> Result<Self::SerializeStructVariant, Self::Error> {
                                        Err(self.unsupported("struct variant"))
                                    }
                                }

                                let serializer = __HeliosFlattenSerializer { state, field_name };
                                value.serialize(serializer)
                            }
                        }
                    } else {
                        TokenStream::new()
                    };

                    let mut field_serializers = Vec::new();
                    let mut field_counts = Vec::new();
                    for field in fields.named.iter() {
                        let field_name_ident = field.ident.as_ref().unwrap(); // Keep original ident for access
                        let field_ty = &field.ty;
                        let effective_field_name_str = get_effective_field_name(field);
                        let underscore_field_name_str = format!("_{}", effective_field_name_str);

                        // Destructure the 4 return values from get_element_info
                        // We need is_element, is_decimal_element, is_option, is_vec here
                        let (is_element, is_decimal_element, is_option, is_vec) =
                            get_element_info(field_ty);

                        // Determine if it's an FHIR element type we need to handle specially
                        let is_fhir_element = is_element || is_decimal_element;

                        // Use field_name_ident for accessing the struct field
                        // Access through self since FhirSerialize operates on the raw type
                        let field_access = quote! { self.#field_name_ident };

                        // Check if field has flatten attribute
                        let field_is_flattened = is_flattened(field);

                        let field_counting_code = if field_is_flattened {
                            // Flattened fields merge into the parent, so they do not affect count directly.
                            quote! {}
                        } else if is_option && !is_vec && is_fhir_element {
                            quote! {
                                if let Some(field) = &#field_access {
                                    if field.value.is_some() {
                                        count += 1;
                                    }
                                    if field.id.is_some() || field.extension.is_some() {
                                        count += 1;
                                    }
                                }
                            }
                        } else if is_vec && is_fhir_element {
                            // Handle Vec<Element> counting - count both primitive and extension arrays if present
                            let vec_access = if is_option {
                                quote! { #field_access.as_ref() }
                            } else {
                                quote! { Some(&#field_access) }
                            };
                            quote! {
                                if let Some(vec_value) = #vec_access {
                                    if !vec_value.is_empty() {
                                        let mut has_primitive_values = false;
                                        let mut has_extension_values = false;
                                        for element in vec_value.iter() {
                                            if element.value.is_some() {
                                                has_primitive_values = true;
                                            }
                                            if element.id.is_some() || element.extension.is_some() {
                                                has_extension_values = true;
                                            }
                                            if has_primitive_values && has_extension_values {
                                                break;
                                            }
                                        }
                                        if has_primitive_values {
                                            count += 1;
                                        }
                                        if has_extension_values {
                                            count += 1;
                                        }
                                    }
                                }
                            }
                        } else if !is_vec && is_fhir_element {
                            quote! {
                                if #field_access.value.is_some() {
                                    count += 1;
                                }
                                if #field_access.id.is_some() || #field_access.extension.is_some() {
                                    count += 1;
                                }
                            }
                        } else if is_vec {
                            if is_option {
                                quote! {
                                    if #field_access.as_ref().map_or(false, |inner| !inner.is_empty()) {
                                        count += 1;
                                    }
                                }
                            } else {
                                quote! {
                                    if !#field_access.is_empty() {
                                        count += 1;
                                    }
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
                        };

                        // Check if field has flatten attribute
                        let field_is_flattened = is_flattened(field);

                        let field_serializing_code = if field_is_flattened {
                            let flatten_serialization = if is_option {
                                quote! {
                                    if let Some(flattened_field) = &#field_access {
                                        let ctx = ::helios_serde::SerializationContext::json(flattened_field);
                                        __helios_serde_flatten_into_map(
                                            &mut state,
                                            &ctx,
                                            #effective_field_name_str,
                                        )?;
                                    }
                                }
                            } else {
                                quote! {
                                    let ctx = ::helios_serde::SerializationContext::json(&#field_access);
                                    __helios_serde_flatten_into_map(
                                        &mut state,
                                        &ctx,
                                        #effective_field_name_str,
                                    )?;
                                }
                            };
                            // For flattened fields, manually merge serialized entries into the parent SerializeMap
                            flatten_serialization
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

                            let vec_inner_ty = if is_option {
                                get_option_inner_type(field_ty)
                                    .and_then(get_vec_inner_type)
                                    .expect("Option<Vec<T>> type expected for FHIR element vector")
                            } else {
                                get_vec_inner_type(field_ty)
                                    .expect("Vec<T> type expected for FHIR element vector")
                            };
                            let primitive_seq_ident =
                                format_ident!("__helios_{}_primitive_values", field_name_ident);
                            let extension_seq_ident =
                                format_ident!("__helios_{}_primitive_extensions", field_name_ident);

                            let helper_defs = quote! {
                                #[allow(non_camel_case_types)]
                                struct #primitive_seq_ident<'a>(&'a [#vec_inner_ty]);

                                impl<'a> serde::Serialize for #primitive_seq_ident<'a> {
                                    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
                                    where
                                        S: serde::Serializer,
                                    {
                                        use serde::ser::SerializeSeq;
                                        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
                                        for element in self.0 {
                                            if let Some(value) = element.value.as_ref() {
                                                let ctx = ::helios_serde::SerializationContext::json(value);
                                                seq.serialize_element(&ctx)?;
                                            } else {
                                                seq.serialize_element(&())?;
                                            }
                                        }
                                        seq.end()
                                    }
                                }

                                #[allow(non_camel_case_types)]
                                struct #extension_seq_ident<'a>(&'a [#vec_inner_ty]);

                                impl<'a> serde::Serialize for #extension_seq_ident<'a> {
                                    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
                                    where
                                        S: serde::Serializer,
                                    {
                                        use serde::ser::SerializeSeq;
                                        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
                                        for element in self.0 {
                                            if let Some(ext_value) =
                                                IdAndExtensionHelperRef::new(
                                                    element.id.as_ref(),
                                                    element.extension.as_ref(),
                                                )
                                            {
                                                seq.serialize_element(&ext_value)?;
                                            } else {
                                                seq.serialize_element(&())?;
                                            }
                                        }
                                        seq.end()
                                    }
                                }
                            };

                            quote! {
                                #helper_defs
                                if let Some(vec_value) = #vec_access {
                                    if !vec_value.is_empty() {
                                        let mut has_primitive_values = false;
                                        let mut has_extension_values = false;
                                        for element in vec_value.iter() {
                                            if element.value.is_some() {
                                                has_primitive_values = true;
                                            }
                                            if element.id.is_some() || element.extension.is_some() {
                                                has_extension_values = true;
                                            }
                                            if has_primitive_values && has_extension_values {
                                                break;
                                            }
                                        }

                                        if has_primitive_values {
                                            #serialize_call(
                                                &#effective_field_name_str,
                                                &#primitive_seq_ident(vec_value.as_slice()),
                                            )?;
                                        }

                                        if has_extension_values {
                                            #serialize_call(
                                                &#underscore_field_name_str,
                                                &#extension_seq_ident(vec_value.as_slice()),
                                            )?;
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
                                            let ctx = ::helios_serde::SerializationContext::json(value);
                                            state.serialize_entry(&#effective_field_name_str, &ctx)?;
                                        }
                                        if let Some(ext_value) = IdAndExtensionHelperRef::new(
                                            field.id.as_ref(),
                                            field.extension.as_ref(),
                                        ) {
                                            state.serialize_entry(&#underscore_field_name_str, &ext_value)?;
                                        }
                                    }
                                }
                            } else {
                                // For SerializeStruct
                                quote! {
                                    if let Some(field) = &#field_access {
                                        if let Some(value) = field.value.as_ref() {
                                            let ctx = ::helios_serde::SerializationContext::json(value);
                                            state.serialize_field(&#effective_field_name_str, &ctx)?;
                                        }
                                        if let Some(ext_value) = IdAndExtensionHelperRef::new(
                                            field.id.as_ref(),
                                            field.extension.as_ref(),
                                        ) {
                                            state.serialize_field(&#underscore_field_name_str, &ext_value)?;
                                        }
                                    }
                                }
                            }
                        } else if !is_vec && is_fhir_element {
                            if has_flattened_fields {
                                // For SerializeMap
                                quote! {
                                    if let Some(value) = #field_access.value.as_ref() {
                                        let ctx = ::helios_serde::SerializationContext::json(value);
                                        state.serialize_entry(&#effective_field_name_str, &ctx)?;
                                    }
                                    if let Some(ext_value) = IdAndExtensionHelperRef::new(
                                        #field_access.id.as_ref(),
                                        #field_access.extension.as_ref(),
                                    ) {
                                        state.serialize_entry(#underscore_field_name_str, &ext_value)?;
                                    }
                                }
                            } else {
                                // For SerializeStruct
                                quote! {
                                    if let Some(value) = #field_access.value.as_ref() {
                                        let ctx = ::helios_serde::SerializationContext::json(value);
                                        state.serialize_field(&#effective_field_name_str, &ctx)?;
                                    }
                                    if let Some(ext_value) = IdAndExtensionHelperRef::new(
                                        #field_access.id.as_ref(),
                                        #field_access.extension.as_ref(),
                                    ) {
                                        state.serialize_field(#underscore_field_name_str, &ext_value)?;
                                    }
                                }
                            }
                        } else if is_option {
                            // Skip serializing if the Option is None
                            if has_flattened_fields {
                                // For SerializeMap
                                quote! {
                                    if let Some(value) = &#field_access {
                                        // Wrap value in serialization context
                                        let ctx = ::helios_serde::SerializationContext::json(value);
                                        // Use serialize_entry for SerializeMap
                                        state.serialize_entry(&#effective_field_name_str, &ctx)?;
                                    }
                                }
                            } else {
                                // For SerializeStruct
                                quote! {
                                    if let Some(value) = &#field_access {
                                        // Wrap value in serialization context
                                        let ctx = ::helios_serde::SerializationContext::json(value);
                                        // Use serialize_field for SerializeStruct
                                        state.serialize_field(&#effective_field_name_str, &ctx)?;
                                    }
                                }
                            }
                        } else if is_vec {
                            // Regular Vec handling (not Element)
                            if has_flattened_fields {
                                // For SerializeMap
                                if is_option {
                                    quote! {
                                        if let Some(inner_vec) = #field_access.as_ref() {
                                            if !inner_vec.is_empty() {
                                                let ctx = ::helios_serde::SerializationContext::json(inner_vec);
                                                state.serialize_entry(&#effective_field_name_str, &ctx)?;
                                            }
                                        }
                                    }
                                } else {
                                    quote! {
                                        if !#field_access.is_empty() {
                                            let ctx = ::helios_serde::SerializationContext::json(&#field_access);
                                            state.serialize_entry(&#effective_field_name_str, &ctx)?;
                                        }
                                    }
                                }
                            } else {
                                // For SerializeStruct
                                if is_option {
                                    quote! {
                                        if let Some(inner_vec) = #field_access.as_ref() {
                                            if !inner_vec.is_empty() {
                                                let ctx = ::helios_serde::SerializationContext::json(inner_vec);
                                                state.serialize_field(&#effective_field_name_str, &ctx)?;
                                            }
                                        }
                                    }
                                } else {
                                    quote! {
                                        if !#field_access.is_empty() {
                                            let ctx = ::helios_serde::SerializationContext::json(&#field_access);
                                            state.serialize_field(&#effective_field_name_str, &ctx)?;
                                        }
                                    }
                                }
                            }
                        } else {
                            // For non-Option types, check if serialization would produce content
                            if has_flattened_fields {
                                // For SerializeMap
                                quote! {
                                    let ctx = ::helios_serde::SerializationContext::json(&#field_access);
                                    if !__helios_serde_is_empty::<_, S::Error>(&ctx)? {
                                        state.serialize_entry(&#effective_field_name_str, &ctx)?;
                                    }
                                }
                            } else {
                                // For SerializeStruct
                                quote! {
                                    let ctx = ::helios_serde::SerializationContext::json(&#field_access);
                                    if !__helios_serde_is_empty::<_, S::Error>(&ctx)? {
                                        state.serialize_field(&#effective_field_name_str, &ctx)?;
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
                            #primitive_helper_definition
                            #empty_helper_definition
                            #flatten_helper_definition
                            let mut count = 0;
                            #(#field_counts)*
                            #import_serialize_map
                            let mut state = serializer.serialize_map(Some(count))?;
                            #(#field_serializers)*
                            state.end()
                        }
                    } else {
                        // If no flattened fields, use serialize_struct as before
                        quote! {
                            #primitive_helper_definition
                            #empty_helper_definition
                            let mut count = 0;
                            #(#field_counts)*
                            #import_serialize_map
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
        Data::Union(_) => panic!("Enums and Unions not supported by FhirSerde"),
    }
}

fn empty_check_helper_definition_tokens() -> TokenStream {
    quote! {
        #[allow(non_camel_case_types)]
        fn __helios_serde_is_empty<T, E>(value: &T) -> Result<bool, E>
        where
            T: serde::Serialize,
            E: serde::ser::Error,
        {
            use ::core::marker::PhantomData;

            #[allow(non_camel_case_types)]
            struct __HeliosEmptySerializer<'a, E> {
                is_empty: &'a mut bool,
                _marker: PhantomData<E>,
            }

            impl<'a, E> __HeliosEmptySerializer<'a, E>
            where
                E: serde::ser::Error,
            {
                fn mark_non_empty(&mut self) {
                    *self.is_empty = false;
                }

                fn child(&mut self) -> __HeliosEmptySerializer<'_, E> {
                    __HeliosEmptySerializer {
                        is_empty: self.is_empty,
                        _marker: PhantomData,
                    }
                }
            }

            impl<'a, E> serde::ser::Serializer for __HeliosEmptySerializer<'a, E>
            where
                E: serde::ser::Error,
            {
                type Ok = ();
                type Error = E;
                type SerializeSeq = __HeliosEmptySeqSerializer<'a, E>;
                type SerializeTuple = __HeliosEmptySeqSerializer<'a, E>;
                type SerializeTupleStruct = __HeliosEmptySeqSerializer<'a, E>;
                type SerializeTupleVariant = __HeliosEmptySeqSerializer<'a, E>;
                type SerializeMap = __HeliosEmptyMapSerializer<'a, E>;
                type SerializeStruct = __HeliosEmptyStructSerializer<'a, E>;
                type SerializeStructVariant = __HeliosEmptyStructSerializer<'a, E>;

                fn serialize_bool(mut self, _v: bool) -> Result<(), E> {
                    self.mark_non_empty();
                    Ok(())
                }

                fn serialize_i8(self, _v: i8) -> Result<(), E> {
                    self.serialize_bool(true)
                }

                fn serialize_i16(self, _v: i16) -> Result<(), E> {
                    self.serialize_bool(true)
                }

                fn serialize_i32(self, _v: i32) -> Result<(), E> {
                    self.serialize_bool(true)
                }

                fn serialize_i64(self, _v: i64) -> Result<(), E> {
                    self.serialize_bool(true)
                }

                fn serialize_i128(self, _v: i128) -> Result<(), E> {
                    self.serialize_bool(true)
                }

                fn serialize_u8(self, _v: u8) -> Result<(), E> {
                    self.serialize_bool(true)
                }

                fn serialize_u16(self, _v: u16) -> Result<(), E> {
                    self.serialize_bool(true)
                }

                fn serialize_u32(self, _v: u32) -> Result<(), E> {
                    self.serialize_bool(true)
                }

                fn serialize_u64(self, _v: u64) -> Result<(), E> {
                    self.serialize_bool(true)
                }

                fn serialize_u128(self, _v: u128) -> Result<(), E> {
                    self.serialize_bool(true)
                }

                fn serialize_f32(self, _v: f32) -> Result<(), E> {
                    self.serialize_bool(true)
                }

                fn serialize_f64(self, _v: f64) -> Result<(), E> {
                    self.serialize_bool(true)
                }

                fn serialize_char(self, _v: char) -> Result<(), E> {
                    self.serialize_bool(true)
                }

                fn serialize_str(self, _v: &str) -> Result<(), E> {
                    self.serialize_bool(true)
                }

                fn serialize_bytes(self, _v: &[u8]) -> Result<(), E> {
                    self.serialize_bool(true)
                }

                fn serialize_none(self) -> Result<(), E> {
                    Ok(())
                }

                fn serialize_some<T>(mut self, value: &T) -> Result<(), E>
                where
                    T: ?Sized + serde::Serialize,
                {
                    value.serialize(self.child())
                }

                fn serialize_unit(self) -> Result<(), E> {
                    Ok(())
                }

                fn serialize_unit_struct(self, _name: &'static str) -> Result<(), E> {
                    Ok(())
                }

                fn serialize_unit_variant(
                    mut self,
                    _name: &'static str,
                    _variant_index: u32,
                    _variant: &'static str,
                ) -> Result<(), E> {
                    self.mark_non_empty();
                    Ok(())
                }

                fn serialize_newtype_struct<T>(mut self, _name: &'static str, value: &T) -> Result<(), E>
                where
                    T: ?Sized + serde::Serialize,
                {
                    value.serialize(self.child())
                }

                fn serialize_newtype_variant<T>(
                    mut self,
                    _name: &'static str,
                    _variant_index: u32,
                    _variant: &'static str,
                    value: &T,
                ) -> Result<(), E>
                where
                    T: ?Sized + serde::Serialize,
                {
                    self.mark_non_empty();
                    value.serialize(self.child())
                }

                fn serialize_seq(mut self, _len: Option<usize>) -> Result<Self::SerializeSeq, E> {
                    self.mark_non_empty();
                    Ok(__HeliosEmptySeqSerializer {
                        is_empty: self.is_empty,
                        _marker: PhantomData,
                    })
                }

                fn serialize_tuple(self, len: usize) -> Result<Self::SerializeTuple, E> {
                    self.serialize_seq(Some(len))
                }

                fn serialize_tuple_struct(
                    self,
                    _name: &'static str,
                    len: usize,
                ) -> Result<Self::SerializeTupleStruct, E> {
                    self.serialize_seq(Some(len))
                }

                fn serialize_tuple_variant(
                    mut self,
                    _name: &'static str,
                    _variant_index: u32,
                    _variant: &'static str,
                    _len: usize,
                ) -> Result<Self::SerializeTupleVariant, E> {
                    self.mark_non_empty();
                    Ok(__HeliosEmptySeqSerializer {
                        is_empty: self.is_empty,
                        _marker: PhantomData,
                    })
                }

                fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, E> {
                    Ok(__HeliosEmptyMapSerializer {
                        is_empty: self.is_empty,
                        wrote_entry: false,
                        _marker: PhantomData,
                    })
                }

                fn serialize_struct(
                    self,
                    _name: &'static str,
                    _len: usize,
                ) -> Result<Self::SerializeStruct, E> {
                    Ok(__HeliosEmptyStructSerializer {
                        is_empty: self.is_empty,
                        wrote_field: false,
                        _marker: PhantomData,
                    })
                }

                fn serialize_struct_variant(
                    mut self,
                    _name: &'static str,
                    _variant_index: u32,
                    _variant: &'static str,
                    _len: usize,
                ) -> Result<Self::SerializeStructVariant, E> {
                    self.mark_non_empty();
                    Ok(__HeliosEmptyStructSerializer {
                        is_empty: self.is_empty,
                        wrote_field: true,
                        _marker: PhantomData,
                    })
                }
            }

            #[allow(non_camel_case_types)]
            struct __HeliosEmptySeqSerializer<'a, E> {
                is_empty: &'a mut bool,
                _marker: PhantomData<E>,
            }

            impl<'a, E> serde::ser::SerializeSeq for __HeliosEmptySeqSerializer<'a, E>
            where
                E: serde::ser::Error,
            {
                type Ok = ();
                type Error = E;

                fn serialize_element<T>(&mut self, value: &T) -> Result<(), E>
                where
                    T: ?Sized + serde::Serialize,
                {
                    value.serialize(__HeliosEmptySerializer {
                        is_empty: self.is_empty,
                        _marker: PhantomData,
                    })
                }

                fn end(self) -> Result<(), E> {
                    Ok(())
                }
            }

            impl<'a, E> serde::ser::SerializeTuple for __HeliosEmptySeqSerializer<'a, E>
            where
                E: serde::ser::Error,
            {
                type Ok = ();
                type Error = E;

                fn serialize_element<T>(&mut self, value: &T) -> Result<(), E>
                where
                    T: ?Sized + serde::Serialize,
                {
                    serde::ser::SerializeSeq::serialize_element(self, value)
                }

                fn end(self) -> Result<(), E> {
                    Ok(())
                }
            }

            impl<'a, E> serde::ser::SerializeTupleStruct for __HeliosEmptySeqSerializer<'a, E>
            where
                E: serde::ser::Error,
            {
                type Ok = ();
                type Error = E;

                fn serialize_field<T>(&mut self, value: &T) -> Result<(), E>
                where
                    T: ?Sized + serde::Serialize,
                {
                    serde::ser::SerializeSeq::serialize_element(self, value)
                }

                fn end(self) -> Result<(), E> {
                    Ok(())
                }
            }

            impl<'a, E> serde::ser::SerializeTupleVariant for __HeliosEmptySeqSerializer<'a, E>
            where
                E: serde::ser::Error,
            {
                type Ok = ();
                type Error = E;

                fn serialize_field<T>(&mut self, value: &T) -> Result<(), E>
                where
                    T: ?Sized + serde::Serialize,
                {
                    serde::ser::SerializeSeq::serialize_element(self, value)
                }

                fn end(self) -> Result<(), E> {
                    Ok(())
                }
            }

            #[allow(non_camel_case_types)]
            struct __HeliosEmptyMapSerializer<'a, E> {
                is_empty: &'a mut bool,
                wrote_entry: bool,
                _marker: PhantomData<E>,
            }

            impl<'a, E> serde::ser::SerializeMap for __HeliosEmptyMapSerializer<'a, E>
            where
                E: serde::ser::Error,
            {
                type Ok = ();
                type Error = E;

                fn serialize_key<T>(&mut self, key: &T) -> Result<(), E>
                where
                    T: ?Sized + serde::Serialize,
                {
                    if !self.wrote_entry {
                        self.wrote_entry = true;
                        *self.is_empty = false;
                    }
                    key.serialize(__HeliosEmptySerializer {
                        is_empty: self.is_empty,
                        _marker: PhantomData,
                    })
                }

                fn serialize_value<T>(&mut self, value: &T) -> Result<(), E>
                where
                    T: ?Sized + serde::Serialize,
                {
                    value.serialize(__HeliosEmptySerializer {
                        is_empty: self.is_empty,
                        _marker: PhantomData,
                    })
                }

                fn serialize_entry<K, V>(&mut self, key: &K, value: &V) -> Result<(), E>
                where
                    K: ?Sized + serde::Serialize,
                    V: ?Sized + serde::Serialize,
                {
                    self.serialize_key(key)?;
                    self.serialize_value(value)
                }

                fn end(self) -> Result<(), E> {
                    Ok(())
                }
            }

            #[allow(non_camel_case_types)]
            struct __HeliosEmptyStructSerializer<'a, E> {
                is_empty: &'a mut bool,
                wrote_field: bool,
                _marker: PhantomData<E>,
            }

            impl<'a, E> __HeliosEmptyStructSerializer<'a, E> {
                fn touch(&mut self) {
                    if !self.wrote_field {
                        self.wrote_field = true;
                        *self.is_empty = false;
                    }
                }
            }

            impl<'a, E> serde::ser::SerializeStruct for __HeliosEmptyStructSerializer<'a, E>
            where
                E: serde::ser::Error,
            {
                type Ok = ();
                type Error = E;

                fn serialize_field<T>(&mut self, _key: &'static str, value: &T) -> Result<(), E>
                where
                    T: ?Sized + serde::Serialize,
                {
                    self.touch();
                    value.serialize(__HeliosEmptySerializer {
                        is_empty: self.is_empty,
                        _marker: PhantomData,
                    })
                }

                fn end(self) -> Result<(), E> {
                    Ok(())
                }
            }

            impl<'a, E> serde::ser::SerializeStructVariant
                for __HeliosEmptyStructSerializer<'a, E>
            where
                E: serde::ser::Error,
            {
                type Ok = ();
                type Error = E;

                fn serialize_field<T>(&mut self, key: &'static str, value: &T) -> Result<(), E>
                where
                    T: ?Sized + serde::Serialize,
                {
                    serde::ser::SerializeStruct::serialize_field(self, key, value)
                }

                fn end(self) -> Result<(), E> {
                    Ok(())
                }
            }

            let mut is_empty = true;
            let serializer = __HeliosEmptySerializer {
                is_empty: &mut is_empty,
                _marker: PhantomData,
            };
            value.serialize(serializer)?;
            Ok(is_empty)
        }
    }
}

/// Generates the `serde::Deserialize` implementation for FHIR types.
///
/// This function produces deserialization code that can reconstruct FHIR types from
/// their JSON representation, handling the complex patterns required by the FHIR
/// specification including extension reunification and choice type discrimination.
///
/// # Generated Code Patterns
///
/// ## For Structs:
/// - **Temporary Struct**: Creates an intermediate deserialization target
/// - **Extension Reunification**: Combines `field` and `_field` data back into Element types
/// - **Array Reconstruction**: Merges split primitive/extension arrays
/// - **Field Mapping**: Maps JSON field names to Rust struct fields
/// - **Type Construction**: Builds final struct from temporary components
///
/// ## For Enums:
/// - **Visitor Pattern**: Uses custom visitor for flexible JSON parsing
/// - **Key-Based Dispatch**: Routes to variants based on JSON object keys
/// - **Extension Handling**: Reconstructs Element types in enum variants
/// - **Error Handling**: Provides detailed error messages for invalid input
///
/// # FHIR-Specific Deserialization
///
/// The generated code handles several FHIR-specific patterns:
///
/// 1. **Extension Reunification**:
///    ```json
///    // Input: { "status": "active", "_status": {"id": "1"} }
///    // Creates: Element { value: Some("active"), id: Some("1"), extension: None }
///    ```
///
/// 2. **Array Reconstruction**:
///    ```json
///    // Input: { "given": ["John", null], "_given": [null, {"id": "middle"}] }
///    // Creates: Vec<Element> with proper value/extension pairing
///    ```
///
/// 3. **Choice Type Discrimination**:
///    ```json
///    // Input: { "valueString": "text" }
///    // Creates: SomeEnum::String("text")
///    ```
///
/// # Temporary Struct Pattern
///
/// For structs, the generated code uses a temporary deserialization target that:
/// - Has separate fields for primitives and extensions (e.g., `field` and `field_ext`)
/// - Uses appropriate intermediate types (e.g., `serde_json::Value` for decimals)
/// - Applies field renaming and default attributes
/// - Is then converted to the final struct type
///
/// # Error Handling
///
/// The generated deserialization code provides:
/// - Field-specific error messages indicating which field failed
/// - Context about whether primitive or extension data caused the failure
/// - Graceful handling of missing fields (using defaults where appropriate)
/// - Type validation for choice types and element containers
///
/// # Arguments
///
/// * `data` - The parsed data structure (struct or enum)
/// * `name` - The type name being generated for
///
/// # Returns
///

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;
    use syn::DeriveInput;
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
        let generics = &input.generics;
        let (_, ty_generics, _) = generics.split_for_impl();
        let serialize_impl = generate_serialize_impl(&input.data, name, &ty_generics);

        let serialize_impl_str = serialize_impl.to_string();

        // Ensure flattened field serialization uses the inline streaming helper
        assert!(serialize_impl_str.contains("__helios_serde_flatten_into_map"));

        // Check that regular serialization uses serialize_entry when flattening is active (due to serialize_map)
        assert!(serialize_impl_str.contains("serialize_entry"));
    }
}
