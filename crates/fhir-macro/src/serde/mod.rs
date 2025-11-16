mod common;
mod deserialize;
mod serialize;

use common::get_enum_tag;
pub(crate) use common::is_flattened;
use deserialize::generate_deserialize_impl;
use proc_macro2::TokenStream;
use quote::quote;
use serialize::generate_serialize_impl;
use syn::{Data, DataEnum, DeriveInput, Ident};

pub(crate) fn fhir_serde_derive_impl(input: DeriveInput) -> TokenStream {
    let name = &input.ident;
    let generics = &input.generics;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    if let Data::Enum(ref data) = input.data {
        if let Some(tag) = get_enum_tag(&input.attrs) {
            return generate_internally_tagged_enum_impl(
                name,
                data,
                &impl_generics,
                &ty_generics,
                &where_clause,
                &tag,
            );
        }
    }

    let serialize_impl = generate_serialize_impl(&input.data, name, &ty_generics);

    let deserialize_impl = generate_deserialize_impl(&input.data, name);

    quote! {
        impl #impl_generics ::helios_serde::FhirSerialize<::helios_serde::Json> for #name #ty_generics #where_clause {
            fn fhir_serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                #serialize_impl
            }
        }

        impl #impl_generics ::helios_serde::FhirDeserialize<::helios_serde::Json> for #name #ty_generics #where_clause {
            fn fhir_deserialize<'de, D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                #deserialize_impl
            }
        }
    }
}

fn generate_internally_tagged_enum_impl(
    name: &Ident,
    data: &DataEnum,
    impl_generics: &syn::ImplGenerics,
    ty_generics: &syn::TypeGenerics,
    where_clause: &Option<&syn::WhereClause>,
    tag: &str,
) -> TokenStream {
    let mut serialize_arms = Vec::new();
    let mut fast_path_arms = Vec::new();
    let mut slow_path_arms = Vec::new();
    let mut variant_names = Vec::new();

    for variant in &data.variants {
        let variant_name = &variant.ident;
        let variant_str = variant_name.to_string();
        variant_names.push(variant_str.clone());

        match &variant.fields {
            syn::Fields::Unnamed(fields) if fields.unnamed.len() == 1 => {
                let field_ty = &fields.unnamed.first().unwrap().ty;

                serialize_arms.push(quote! {
                    #name #ty_generics::#variant_name(inner) => {
                        let ctx = ::helios_serde::SerializationContext::json(inner);
                        let inner_json = serde_json::to_value(&ctx).map_err(|e| {
                            serde::ser::Error::custom(format!(
                                "Failed to serialize {} variant '{}': {}",
                                stringify!(#name),
                                #variant_str,
                                e
                            ))
                        })?;
                        if let serde_json::Value::Object(obj) = inner_json {
                            let mut map = serializer.serialize_map(Some(obj.len() + 1))?;
                            map.serialize_entry(#tag, #variant_str)?;
                            for (k, v) in obj.into_iter() {
                                map.serialize_entry(&k, &v)?;
                            }
                            map.end()
                        } else {
                            Err(serde::ser::Error::custom(format!(
                                "Flattened {} variant '{}' did not produce an object",
                                stringify!(#name),
                                #variant_str
                            )))
                        }
                    }
                });

                fast_path_arms.push(quote! {
                    #variant_str => {
                        use serde::de::value::MapAccessDeserializer;
                        let map = remaining_map
                            .take()
                            .ok_or_else(|| de::Error::custom("resourceType map already taken"))?;
                        let map_access = FastPathMapAccess {
                            first_key: prefix_key.take(),
                            first_value: prefix_value.take(),
                            map,
                        };
                        let ctx = ::helios_serde::DeserializationContext::<#field_ty, ::helios_serde::Json>::json();
                        let inner = ctx
                            .deserialize(MapAccessDeserializer::new(map_access))
                            .map_err(|e| {
                                de::Error::custom(format!(
                                    "Failed to deserialize variant {}: {}",
                                    #variant_str, e
                                ))
                            })?;
                        Ok(#name::#variant_name(inner))
                    }
                });

                slow_path_arms.push(quote! {
                    #variant_str => {
                        use serde::de::{self, DeserializeSeed, IntoDeserializer};
                        let ctx = ::helios_serde::DeserializationContext::<#field_ty, ::helios_serde::Json>::json();
                        let inner = ctx.deserialize(json_value.into_deserializer())
                            .map_err(|e| de::Error::custom(format!("Failed to deserialize variant {}: {}", #variant_str, e)))?;
                        Ok(#name::#variant_name(inner))
                    }
                });
            }
            _ => {
                panic!("Internally-tagged enums must have newtype variants");
            }
        }
    }

    let expanded = quote! {
        impl #impl_generics ::helios_serde::FhirSerialize<::helios_serde::Json> for #name #ty_generics #where_clause {
            fn fhir_serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                use serde::ser::SerializeMap;
                match self {
                    #(#serialize_arms)*
                }
            }
        }

        impl #impl_generics ::helios_serde::FhirDeserialize<::helios_serde::Json> for #name #ty_generics #where_clause {
            fn fhir_deserialize<'de, D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                use serde::de::{self, DeserializeSeed, IntoDeserializer, MapAccess, Visitor};
                use serde::de::value::{MapAccessDeserializer, StringDeserializer};
                use serde_json;

                struct EnumVisitor;

                struct FastPathMapAccess<A> {
                    first_key: Option<::std::string::String>,
                    first_value: Option<serde_json::Value>,
                    map: A,
                }

                impl<'de, A> MapAccess<'de> for FastPathMapAccess<A>
                where
                    A: MapAccess<'de>,
                {
                    type Error = A::Error;

                    fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>, A::Error>
                    where
                        K: serde::de::DeserializeSeed<'de>,
                    {
                        if let Some(key) = self.first_key.take() {
                            let key_de = StringDeserializer::new(key);
                            seed
                                .deserialize(key_de)
                                .map(Some)
                                .map_err(|e: serde::de::value::Error| {
                                    <A::Error as de::Error>::custom(e.to_string())
                                })
                        } else {
                            self.map.next_key_seed(seed)
                        }
                    }

                    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value, A::Error>
                    where
                        V: serde::de::DeserializeSeed<'de>,
                    {
                        if let Some(value) = self.first_value.take() {
                            seed
                                .deserialize(value.into_deserializer())
                                .map_err(|e: serde_json::Error| {
                                    <A::Error as de::Error>::custom(e.to_string())
                                })
                        } else {
                            self.map.next_value_seed(seed)
                        }
                    }

                    fn size_hint(&self) -> Option<usize> {
                        let extra = if self.first_key.is_some() { 1 } else { 0 };
                        self.map.size_hint().map(|hint| hint + extra)
                    }
                }

                impl<'de> Visitor<'de> for EnumVisitor {
                    type Value = #name;

                    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                        formatter.write_str(concat!("a ", stringify!(#name), " with a ", #tag, " field"))
                    }

                    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
                    where
                        A: MapAccess<'de>,
                    {
                        let first_key = match map.next_key::<::std::string::String>()? {
                            Some(key) => key,
                            None => return Err(de::Error::missing_field(#tag)),
                        };

                        if first_key == #tag {
                            let tag_value_raw = map.next_value::<serde_json::Value>()?;
                            let tag_value = tag_value_raw
                                .as_str()
                                .ok_or_else(|| de::Error::custom(concat!(#tag, " must be a string")))?
                                .to_string();

                            let mut prefix_key = Some(first_key);
                            let mut prefix_value = Some(tag_value_raw);
                            let mut remaining_map = Some(map);

                            return match tag_value.as_str() {
                                #(#fast_path_arms)*
                                _ => Err(de::Error::unknown_variant(&tag_value, &[#(#variant_names),*])),
                            };
                        }

                        let first_value: serde_json::Value = map.next_value()?;
                        let mut buffered_entries: ::std::vec::Vec<(::std::string::String, serde_json::Value)> =
                            ::std::vec::Vec::new();
                        buffered_entries.push((first_key.clone(), first_value));
                        let mut tag_value_json: Option<serde_json::Value> = None;

                        while let Some((key, value)) =
                            map.next_entry::<::std::string::String, serde_json::Value>()?
                        {
                            if key == #tag {
                                tag_value_json = Some(value);
                            } else {
                                buffered_entries.push((key, value));
                            }
                        }

                        let tag_value_json = tag_value_json.ok_or_else(|| de::Error::missing_field(#tag))?;
                        let tag_value = tag_value_json
                            .as_str()
                            .ok_or_else(|| de::Error::custom(concat!(#tag, " must be a string")))?
                            .to_string();

                        let mut obj = serde_json::Map::with_capacity(buffered_entries.len() + 1);
                        for (key, value) in buffered_entries {
                            obj.insert(key, value);
                        }
                        obj.insert(#tag.to_string(), tag_value_json);

                        let json_value = serde_json::Value::Object(obj);

                        match tag_value.as_str() {
                            #(#slow_path_arms)*
                            _ => Err(de::Error::unknown_variant(&tag_value, &[#(#variant_names),*])),
                        }
                    }
                }

                deserializer.deserialize_map(EnumVisitor)
            }
        }
    };

    expanded
}
