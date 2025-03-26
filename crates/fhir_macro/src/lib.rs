use proc_macro::TokenStream;
use quote::quote;
use syn::{DeriveInput, parse_macro_input};

// Define our derive macro
#[proc_macro_derive(FhirSerde)]
pub fn fhir_derive_macro(input: TokenStream) -> TokenStream {
    // Parse the input tokens into a syntax tree
    let input = parse_macro_input!(input as DeriveInput);

    // Get the name of the struct or enum
    let name = input.ident;

    // Build the implementation
    let expanded = quote! {
        impl FhirSerde for #name {
            fn fhir_derive_macro(&self) {
                println!("Hello, World! My name is {}", stringify!(#name));
            }
        }
    };

    // Convert back to TokenStream
    expanded.into()
}
