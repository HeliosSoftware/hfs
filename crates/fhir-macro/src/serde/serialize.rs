use super::common::{get_effective_field_name, is_flattened};
use crate::util::get_element_info;
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
                            // For Element types, we need special handling for the _fieldName pattern
                            let underscore_variant_key = format!("_{}", variant_key);

                            match_arms.push(quote! {
                                // Removed 'ref' from pattern
                                #name #ty_generics::#variant_name(value) => {
                                    let has_extension = value.id.is_some() || value.extension.is_some();
                                    let primitive_json = if let Some(inner_value) = value.value.as_ref() {
                                        let ctx = ::helios_serde::SerializationContext::json(inner_value);
                                        Some(serde_json::to_value(&ctx).map_err(|e| serde::ser::Error::custom(format!("Failed to serialize primitive value: {}", e)))?)
                                    } else {
                                        None
                                    };
                                    if let Some(json_val) = primitive_json {
                                        state.serialize_entry(#variant_key, &json_val)?;
                                    }
                                    if has_extension {
                                        // Create JSON object for id/extension
                                        let mut ext_obj = serde_json::Map::new();
                                        if let Some(ref id) = value.id {
                                            ext_obj.insert("id".to_string(), serde_json::Value::String(id.clone()));
                                        }
                                        if let Some(ref extension) = value.extension {
                                            let ctx = ::helios_serde::SerializationContext::json(extension);
                                            match serde_json::to_value(&ctx) {
                                                Ok(json_val) => {
                                                    ext_obj.insert("extension".to_string(), json_val);
                                                },
                                                Err(e) => return Err(serde::ser::Error::custom(format!("Failed to serialize extension: {}", e))),
                                            }
                                        }
                                        let ext_json = serde_json::Value::Object(ext_obj);
                                        state.serialize_entry(#underscore_variant_key, &ext_json)?;
                                    }
                                    // Don't return Result here, just continue
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

            // Generate the enum serialization implementation
            quote! {
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

                    // Import SerializeMap trait if we have flattened fields
                    let import_serialize_map = if has_flattened_fields {
                        quote! { use serde::ser::SerializeMap; }
                    } else {
                        quote! { use serde::ser::SerializeStruct; }
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

                        let extension_field_ident =
                            format_ident!("is_{}_extension", field_name_ident);

                        // Check if field has flatten attribute
                        let field_is_flattened = is_flattened(field);

                        let field_counting_code = if field_is_flattened {
                            // For flattened fields, we don't increment the count
                            // as they will be flattened into the parent object
                            quote! {
                                // No count increment for flattened fields
                                #[allow(unused_variables)]
                                let mut #extension_field_ident = false;
                            }
                        } else if is_option && !is_vec && is_fhir_element {
                            quote! {
                                let mut #extension_field_ident = false;
                                if let Some(field) = &#field_access {
                                    if field.value.is_some() {
                                        count += 1;
                                    }
                                    if field.id.is_some() || field.extension.is_some() {
                                        count += 1;
                                        #extension_field_ident = true;
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
                                        // Count primitive array
                                        count += 1;
                                        // Count extension array if any elements have extensions
                                        if vec_value.iter().any(|element| element.id.is_some() || element.extension.is_some()) {
                                            count += 1;
                                        }
                                    }
                                }
                            }
                        } else if !is_vec && is_fhir_element {
                            quote! {
                                let mut #extension_field_ident = false;
                                if #field_access.value.is_some() {
                                    count += 1;
                                }
                                if #field_access.id.is_some() || #field_access.extension.is_some() {
                                    count += 1;
                                    #extension_field_ident = true;
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

                            quote! {
                                // Handle Vec<Element> by splitting into primitive and extension arrays
                                if let Some(vec_value) = #vec_access { // Use the adjusted access logic
                                    if !vec_value.is_empty() {
                                        // Create primitive array
                                        let mut primitive_array = Vec::with_capacity(vec_value.len());
                                        // Create extension array
                                        let mut extension_array = Vec::with_capacity(vec_value.len());
                                        // Track if we need to include _fieldName
                                        let mut has_extensions = false;

                                        // Process each element
                                        for element in vec_value.iter() {
                                            // Add primitive value or null
                                            match &element.value {
                                                Some(value) => {
                                                    // Wrap value in serialization context
                                                    let ctx = ::helios_serde::SerializationContext::json(value);
                                                    match serde_json::to_value(&ctx) {
                                                        Ok(json_val) => primitive_array.push(json_val),
                                                        Err(e) => return Err(serde::ser::Error::custom(format!("Failed to serialize primitive value: {}", e))),
                                                    }
                                                },
                                                None => primitive_array.push(serde_json::Value::Null),
                                            }

                                            // Check if this element has id or extension (pure context system)
                                            if element.id.is_some() || element.extension.is_some() {
                                                has_extensions = true;

                                                // Create JSON object for id/extension
                                                let mut ext_obj = serde_json::Map::new();
                                                if let Some(ref id) = element.id {
                                                    ext_obj.insert("id".to_string(), serde_json::Value::String(id.clone()));
                                                }
                                                if let Some(ref extension) = element.extension {
                                                    let ctx = ::helios_serde::SerializationContext::json(extension);
                                                    match serde_json::to_value(&ctx) {
                                                        Ok(json_val) => {
                                                            ext_obj.insert("extension".to_string(), json_val);
                                                        },
                                                        Err(e) => return Err(serde::ser::Error::custom(format!("Failed to serialize extension: {}", e))),
                                                    }
                                                }
                                                extension_array.push(serde_json::Value::Object(ext_obj));
                                            } else {
                                                // No id or extension
                                                extension_array.push(serde_json::Value::Null);
                                            }
                                        }

                                        // Check if the primitive array contains any non-null values
                                        let should_serialize_primitive_array = primitive_array.iter().any(|v| !v.is_null());

                                        // Serialize primitive array only if it has non-null values
                                        if should_serialize_primitive_array {
                                            #serialize_call(&#effective_field_name_str, &primitive_array)?;
                                        }

                                        // Serialize extension array if needed, using the correct method
                                        if has_extensions {
                                            // Use the existing underscore_field_name_str variable which lives longer
                                            #serialize_call(&#underscore_field_name_str, &extension_array)?;
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
                                            // Wrap value in serialization context
                                            let ctx = ::helios_serde::SerializationContext::json(value);
                                            // Use serialize_entry for SerializeMap
                                            state.serialize_entry(&#effective_field_name_str, &ctx)?;
                                        }
                                        if #extension_field_ident {
                                            // Create JSON object for id/extension
                                            let mut ext_obj = serde_json::Map::new();
                                            if let Some(ref id) = field.id {
                                                ext_obj.insert("id".to_string(), serde_json::Value::String(id.clone()));
                                            }
                                            if let Some(ref extension) = field.extension {
                                                let ctx = ::helios_serde::SerializationContext::json(extension);
                                                match serde_json::to_value(&ctx) {
                                                    Ok(json_val) => {
                                                        ext_obj.insert("extension".to_string(), json_val);
                                                    },
                                                    Err(e) => return Err(serde::ser::Error::custom(format!("Failed to serialize extension: {}", e))),
                                                }
                                            }
                                            state.serialize_entry(&#underscore_field_name_str, &serde_json::Value::Object(ext_obj))?;
                                        }
                                    }
                                }
                            } else {
                                // For SerializeStruct
                                quote! {
                                    if let Some(field) = &#field_access {
                                        if let Some(value) = field.value.as_ref() {
                                            // Wrap value in serialization context
                                            let ctx = ::helios_serde::SerializationContext::json(value);
                                            // Use serialize_field for SerializeStruct
                                            state.serialize_field(&#effective_field_name_str, &ctx)?;
                                        }
                                        if #extension_field_ident {
                                            // Create JSON object for id/extension
                                            let mut ext_obj = serde_json::Map::new();
                                            if let Some(ref id) = field.id {
                                                ext_obj.insert("id".to_string(), serde_json::Value::String(id.clone()));
                                            }
                                            if let Some(ref extension) = field.extension {
                                                let ctx = ::helios_serde::SerializationContext::json(extension);
                                                match serde_json::to_value(&ctx) {
                                                    Ok(json_val) => {
                                                        ext_obj.insert("extension".to_string(), json_val);
                                                    },
                                                    Err(e) => return Err(serde::ser::Error::custom(format!("Failed to serialize extension: {}", e))),
                                                }
                                            }
                                            state.serialize_field(&#underscore_field_name_str, &serde_json::Value::Object(ext_obj))?;
                                        }
                                    }
                                }
                            }
                        } else if !is_vec && is_fhir_element {
                            if has_flattened_fields {
                                // For SerializeMap
                                quote! {
                                    if let Some(value) = #field_access.value.as_ref() {
                                        // Wrap value in serialization context
                                        let ctx = ::helios_serde::SerializationContext::json(value);
                                        // Use serialize_entry for SerializeMap
                                        state.serialize_entry(&#effective_field_name_str, &ctx)?;
                                    }
                                    if #extension_field_ident {
                                        // Create JSON object for id/extension
                                        let mut ext_obj = serde_json::Map::new();
                                        if let Some(ref id) = #field_access.id {
                                            ext_obj.insert("id".to_string(), serde_json::Value::String(id.clone()));
                                        }
                                        if let Some(ref extension) = #field_access.extension {
                                            let ctx = ::helios_serde::SerializationContext::json(extension);
                                            match serde_json::to_value(&ctx) {
                                                Ok(json_val) => {
                                                    ext_obj.insert("extension".to_string(), json_val);
                                                },
                                                Err(e) => return Err(serde::ser::Error::custom(format!("Failed to serialize extension: {}", e))),
                                            }
                                        }
                                        state.serialize_entry(#underscore_field_name_str, &serde_json::Value::Object(ext_obj))?;
                                    }
                                }
                            } else {
                                // For SerializeStruct
                                quote! {
                                    if let Some(value) = #field_access.value.as_ref() {
                                        // Wrap value in serialization context
                                        let ctx = ::helios_serde::SerializationContext::json(value);
                                        // Use serialize_field for SerializeStruct
                                        state.serialize_field(&#effective_field_name_str, &ctx)?;
                                    }
                                    if #extension_field_ident {
                                        // Create JSON object for id/extension
                                        let mut ext_obj = serde_json::Map::new();
                                        if let Some(ref id) = #field_access.id {
                                            ext_obj.insert("id".to_string(), serde_json::Value::String(id.clone()));
                                        }
                                        if let Some(ref extension) = #field_access.extension {
                                            let ctx = ::helios_serde::SerializationContext::json(extension);
                                            match serde_json::to_value(&ctx) {
                                                Ok(json_val) => {
                                                    ext_obj.insert("extension".to_string(), json_val);
                                                },
                                                Err(e) => return Err(serde::ser::Error::custom(format!("Failed to serialize extension: {}", e))),
                                            }
                                        }
                                        state.serialize_field(&#underscore_field_name_str, &serde_json::Value::Object(ext_obj))?;
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
                                quote! {
                                    // Wrap value in serialization context for checking
                                    let ctx = ::helios_serde::SerializationContext::json(&#field_access);
                                    // Use serde_json to check if the field serializes to null or empty object
                                    let json_value = serde_json::to_value(&ctx).map_err(|_| serde::ser::Error::custom("serialization failed"))?;
                                    if !json_value.is_null() && !(json_value.is_object() && json_value.as_object().unwrap().is_empty()) {
                                        // Use serialize_entry for SerializeMap
                                        state.serialize_entry(&#effective_field_name_str, &ctx)?;
                                    }
                                }
                            } else {
                                // For SerializeStruct
                                quote! {
                                    // Wrap value in serialization context for checking
                                    let ctx = ::helios_serde::SerializationContext::json(&#field_access);
                                    // Use serde_json to check if the field serializes to null or empty object
                                    let json_value = serde_json::to_value(&ctx).map_err(|_| serde::ser::Error::custom("serialization failed"))?;
                                    if !json_value.is_null() && !(json_value.is_object() && json_value.as_object().unwrap().is_empty()) {
                                        // Use serialize_field for SerializeStruct
                                        state.serialize_field(&#effective_field_name_str, &ctx)?;
                                    }
                                }
                            }
                        } else {
                            // For non-Option types, check if it's a struct with all None/null fields
                            if has_flattened_fields {
                                // For SerializeMap
                                quote! {
                                    // Wrap value in serialization context for checking
                                    let ctx = ::helios_serde::SerializationContext::json(&#field_access);
                                    // Use serde_json to check if the field serializes to null or empty object
                                    let json_value = serde_json::to_value(&ctx).map_err(|_| serde::ser::Error::custom("serialization failed"))?;
                                    if !json_value.is_null() && !(json_value.is_object() && json_value.as_object().unwrap().is_empty()) {
                                        // Use serialize_entry for SerializeMap
                                        state.serialize_entry(&#effective_field_name_str, &ctx)?;
                                    }
                                }
                            } else {
                                // For SerializeStruct
                                quote! {
                                    // Wrap value in serialization context for checking
                                    let ctx = ::helios_serde::SerializationContext::json(&#field_access);
                                    // Use serde_json to check if the field serializes to null or empty object
                                    let json_value = serde_json::to_value(&ctx).map_err(|_| serde::ser::Error::custom("serialization failed"))?;
                                    if !json_value.is_null() && !(json_value.is_object() && json_value.as_object().unwrap().is_empty()) {
                                        // Use serialize_field for SerializeStruct
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
