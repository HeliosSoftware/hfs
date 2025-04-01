use proc_macro::TokenStream;

#[proc_macro_derive(FhirSerde)]
pub fn fhir_derive_macro(input: TokenStream) -> TokenStream {
    input
}
