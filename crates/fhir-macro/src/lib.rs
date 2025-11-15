//! # FHIR Macro - Procedural Macros for FHIR Implementation
//!
//! This crate provides procedural macros that enable automatic code generation for FHIR
//! (Fast Healthcare Interoperability Resources) implementations in Rust. It contains the
//! core macro functionality that powers serialization, deserialization, and FHIRPath
//! evaluation across the entire FHIR ecosystem.
//!
//! ## Overview
//!
//! The `fhir_macro` crate implements two essential derive macros:
//!
//! - **`#[derive(FhirSerde)]`** - Custom serialization/deserialization handling FHIR's
//!   JSON representation including its extension pattern
//! - **`#[derive(FhirPath)]`** - Automatic conversion to FHIRPath evaluation results for
//!   resource traversal
//!
//! These macros are automatically applied to thousands of generated FHIR types, eliminating
//! the need for hand-written serialization code while ensuring compliance with FHIR's
//! complex serialization requirements.

extern crate proc_macro;

use proc_macro::TokenStream;
use quote::quote;
use syn::{DeriveInput, Ident, Lit, Meta, parse_macro_input, punctuated::Punctuated, token};

mod fhirpath;
mod serde;
mod util;

#[proc_macro_derive(FhirSerde, attributes(fhir_serde))]
pub fn fhir_serde_derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    serde::fhir_serde_derive_impl(input).into()
}

#[proc_macro_derive(FhirPath, attributes(fhir_serde, fhir_choice_element, fhir_resource))]
pub fn fhir_path_derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    fhirpath::expand(input).into()
}

#[proc_macro_derive(TypeInfo, attributes(type_info))]
pub fn type_info_derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let generics = &input.generics;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let (namespace, type_name) = extract_type_info_attributes(&input.attrs, name);

    let expanded = quote! {
        impl #impl_generics helios_fhirpath_support::TypeInfo for #name #ty_generics #where_clause {
            fn type_namespace() -> &'static str {
                #namespace
            }

            fn type_name() -> &'static str {
                #type_name
            }
        }
    };

    expanded.into()
}

fn extract_type_info_attributes(attrs: &[syn::Attribute], type_name: &Ident) -> (String, String) {
    for attr in attrs {
        if attr.path().is_ident("type_info") {
            if let Ok(list) =
                attr.parse_args_with(Punctuated::<Meta, token::Comma>::parse_terminated)
            {
                let mut namespace = None;
                let mut name = None;

                for meta in list {
                    if let Meta::NameValue(nv) = meta {
                        if nv.path.is_ident("namespace") {
                            if let syn::Expr::Lit(expr_lit) = nv.value {
                                if let Lit::Str(lit_str) = expr_lit.lit {
                                    namespace = Some(lit_str.value());
                                }
                            }
                        } else if nv.path.is_ident("name") {
                            if let syn::Expr::Lit(expr_lit) = nv.value {
                                if let Lit::Str(lit_str) = expr_lit.lit {
                                    name = Some(lit_str.value());
                                }
                            }
                        }
                    }
                }

                if let (Some(ns), Some(n)) = (namespace, name) {
                    return (format!("\"{}\"", ns), format!("\"{}\"", n));
                }
            }
        }
    }

    let inferred_name = type_name.to_string();
    ("\"FHIR\"".to_string(), format!("\"{}\"", inferred_name))
}
