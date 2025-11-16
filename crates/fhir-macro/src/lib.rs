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
//!
//! ## FHIR Serialization Challenges
//!
//! FHIR has several unique serialization patterns that require special handling:
//!
//! ### Extension Pattern
//!
//! FHIR primitives can have associated metadata stored in a parallel `_fieldName` object:
//!
//! ```json
//! {
//!   "status": "active",
//!   "_status": {
//!     "id": "status-1",
//!     "extension": [...]
//!   }
//! }
//! ```
//!
//! ### Array Serialization
//!
//! Arrays of primitives are split into separate primitive and extension arrays:
//!
//! ```json
//! {
//!   "given": ["John", "Michael", null],
//!   "_given": [null, {"id": "name-2"}, {}]
//! }
//! ```
//!
//! ### Choice Types
//!
//! FHIR's `[x]` fields are serialized as single key-value pairs with type suffixes:
//!
//! ```json
//! { "valueQuantity": {...} }  // for Quantity type
//! { "valueString": "text" }   // for String type
//! ```
//!
//! ## Usage
//!
//! ```ignore
//! use fhir_macro::{FhirSerde, FhirPath};
//!
//! #[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
//! pub struct Patient {
//!     pub id: Option<String>,
//!     pub extension: Option<Vec<Extension>>,
//!     #[fhir_serde(rename = "implicitRules")]
//!     pub implicit_rules: Option<Uri>,
//!     pub active: Option<Boolean>,  // Element<bool, Extension>
//!     pub name: Option<Vec<HumanName>>,
//! }
//! ```

extern crate proc_macro;

use heck::ToLowerCamelCase;
use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{
    Data, DeriveInput, Fields, GenericArgument, Ident, Lit, Meta, Path, PathArguments, Type,
    TypePath, parse_macro_input, punctuated::Punctuated, token,
};

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
fn get_effective_field_name(field: &syn::Field) -> String {
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
fn is_flattened(field: &syn::Field) -> bool {
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

/// Derives `serde::Serialize` and `serde::Deserialize` implementations for FHIR types.
///
/// This procedural macro automatically generates serialization and deserialization code
/// that handles FHIR's complex JSON representation patterns, including the extension
/// pattern, choice types, and array serialization.
///
/// # Supported Attributes
///
/// - `#[fhir_serde(rename = "name")]` - Renames a field for serialization
/// - `#[fhir_serde(flatten)]` - Flattens a field into the parent object
///
/// # Generated Implementations
///
/// The macro generates both `Serialize` and `Deserialize` implementations that:
///
/// ## For Structs:
/// - Handle FHIR extension pattern (`field` and `_field` pairs)
/// - Support `Element<T, Extension>` and `DecimalElement<Extension>` types
/// - Serialize arrays with split primitive/extension arrays
/// - Apply field renaming and flattening as specified
///
/// ## For Enums:
/// - Serialize as single key-value pairs for choice types
/// - Handle extension patterns for element-containing variants
/// - Support resource type enums with proper discriminators
///
/// # FHIR Extension Pattern
///
/// For fields containing Element types, the macro automatically handles the FHIR
/// extension pattern where primitives and their metadata are stored separately:
///
/// ```json
/// {
///   "status": "active",        // Primitive value
///   "_status": {               // Extension metadata
///     "id": "status-1",
///     "extension": [...]
///   }
/// }
/// ```
///
/// # Examples
///
/// ```rust,ignore
/// use fhir_macro::FhirSerde;
///
/// #[derive(FhirSerde)]
/// pub struct Patient {
///     pub id: Option<String>,
///     #[fhir_serde(rename = "implicitRules")]
///     pub implicit_rules: Option<Uri>,
///     pub active: Option<Boolean>,  // Element<bool, Extension>
/// }
///
/// #[derive(FhirSerde)]
/// pub enum ObservationValue {
///     #[fhir_serde(rename = "valueQuantity")]
///     Quantity(Quantity),
///     #[fhir_serde(rename = "valueString")]
///     String(String),
/// }
/// ```
///
/// # Error Handling
///
/// The generated deserialization code includes comprehensive error handling:
/// - Field-specific error messages for debugging
/// - Graceful handling of missing or malformed extension data
/// - Type validation for choice types and element containers
///
/// # Performance
///
/// The generated code is optimized for:
/// - Minimal allocations during serialization/deserialization
/// - Efficient field access using direct struct field access
/// - Lazy evaluation of extension objects (only when present)
/// - Vector pre-allocation for known array sizes
#[proc_macro_derive(FhirSerde, attributes(fhir_serde))]
pub fn fhir_serde_derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident;
    let generics = input.generics;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let serialize_impl = generate_serialize_impl(&input.data, &name);

    // Pass all generic parts to deserialize generator
    let deserialize_impl = generate_deserialize_impl(&input.data, &name);

    let expanded = quote! {
        // --- Serialize Implementation ---
        impl #impl_generics serde::Serialize for #name #ty_generics #where_clause {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                #serialize_impl
            }
        }

        // --- Deserialize Implementation ---
        impl<'de> #impl_generics serde::Deserialize<'de> for #name #ty_generics #where_clause
        where
            // Add bounds for generic types used in fields if necessary
            // Example: T: serde::Deserialize<'de>,
        {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                #deserialize_impl
            }
        }
    };

    TokenStream::from(expanded)
}

//=============================================================================
// Type Analysis Helper Functions
//=============================================================================

/// Extracts the inner type from an `Option<T>` type.
///
/// This helper function analyzes a type path to determine if it represents an
/// `Option<T>` and extracts the inner type `T` if so.
///
/// # Arguments
///
/// * `ty` - The type to analyze
///
/// # Returns
///
/// - `Some(&Type)` containing the inner type if this is an `Option<T>`
/// - `None` if this is not an `Option` type
///
/// # Examples
///
/// ```rust,ignore
/// // For type: Option<String>
/// // Returns: Some(String)
///
/// // For type: String  
/// // Returns: None
///
/// // For type: Option<Vec<HumanName>>
/// // Returns: Some(Vec<HumanName>)
/// ```
fn get_option_inner_type(ty: &Type) -> Option<&Type> {
    if let Type::Path(TypePath {
        path: Path { segments, .. },
        ..
    }) = ty
    {
        if let Some(segment) = segments.last() {
            if segment.ident == "Option" {
                if let PathArguments::AngleBracketed(args) = &segment.arguments {
                    if let Some(GenericArgument::Type(inner_ty)) = args.args.first() {
                        return Some(inner_ty);
                    }
                }
            }
        }
    }
    None
}

/// Extracts the inner type from a `Vec<T>` type.
///
/// This helper function analyzes a type path to determine if it represents a
/// `Vec<T>` and extracts the inner type `T` if so.
///
/// # Arguments
///
/// * `ty` - The type to analyze
///
/// # Returns
///
/// - `Some(&Type)` containing the inner type if this is a `Vec<T>`
/// - `None` if this is not a `Vec` type
///
/// # Examples
///
/// ```rust,ignore
/// // For type: Vec<String>
/// // Returns: Some(String)
///
/// // For type: String
/// // Returns: None
///
/// // For type: Vec<HumanName>
/// // Returns: Some(HumanName)
/// ```
fn get_vec_inner_type(ty: &Type) -> Option<&Type> {
    if let Type::Path(TypePath {
        path: Path { segments, .. },
        ..
    }) = ty
    {
        if let Some(segment) = segments.last() {
            if segment.ident == "Vec" {
                if let PathArguments::AngleBracketed(args) = &segment.arguments {
                    if let Some(GenericArgument::Type(inner_ty)) = args.args.first() {
                        return Some(inner_ty);
                    }
                }
            }
        }
    }
    None
}

/// Extracts the inner type from a `Box<T>` type.
///
/// This helper function analyzes a type path to determine if it represents a
/// `Box<T>` and extracts the inner type `T` if so. Box types are used in FHIR
/// for cycle breaking in recursive data structures.
///
/// # Arguments
///
/// * `ty` - The type to analyze
///
/// # Returns
///
/// - `Some(&Type)` containing the inner type if this is a `Box<T>`
/// - `None` if this is not a `Box` type
///
/// # Examples
///
/// ```rust,ignore
/// // For type: Box<Reference>
/// // Returns: Some(Reference)
///
/// // For type: Reference
/// // Returns: None
/// ```
fn get_box_inner_type(ty: &Type) -> Option<&Type> {
    if let Type::Path(TypePath {
        path: Path { segments, .. },
        ..
    }) = ty
    {
        if let Some(segment) = segments.last() {
            if segment.ident == "Box" {
                if let PathArguments::AngleBracketed(args) = &segment.arguments {
                    if let Some(GenericArgument::Type(inner_ty)) = args.args.first() {
                        return Some(inner_ty);
                    }
                }
            }
        }
    }
    None
}

/// Analyzes a type to determine FHIR element characteristics and container wrapping.
///
/// This function is central to the FHIR serialization logic, determining how a field
/// should be handled based on its type. It identifies FHIR element types and their
/// container wrappers to generate appropriate serialization code.
///
/// # Type Analysis
///
/// The function recursively unwraps container types in this order:
/// 1. `Option<T>` → T (marks as optional)
/// 2. `Vec<T>` → T (marks as vector, handles `Vec<Option<T>>` case)
/// 3. `Box<T>` → T (unwraps boxed types)
/// 4. Analyzes the final type for FHIR element characteristics
///
/// # FHIR Element Types
///
/// - **Element types**: FHIR primitive type aliases like `String`, `Boolean`, `Code`
/// - **DecimalElement types**: The special `Decimal` type requiring precision preservation
/// - **Direct types**: `Element<V, E>` and `DecimalElement<E>` generic types
///
/// # Arguments
///
/// * `field_ty` - The type to analyze (may be wrapped in Option/Vec/Box)
///
/// # Returns
///
/// A tuple `(is_element, is_decimal_element, is_option, is_vec)` where:
/// - `is_element` - True if this is a FHIR element type (not decimal)
/// - `is_decimal_element` - True if this is a FHIR decimal element type
/// - `is_option` - True if the type was wrapped in `Option<T>`
/// - `is_vec` - True if the type was wrapped in `Vec<T>`
///
/// # Examples
///
/// ```rust,ignore
/// // Option<String> (FHIR element alias)
/// // Returns: (true, false, true, false)
///
/// // Vec<Decimal> (FHIR decimal element alias)  
/// // Returns: (false, true, false, true)
///
/// // Option<Vec<Boolean>> (FHIR element in vector)
/// // Returns: (true, false, true, true)
///
/// // Element<String, Extension> (direct element type)
/// // Returns: (true, false, false, false)
///
/// // i32 (regular Rust type, not FHIR element)
/// // Returns: (false, false, false, false)
/// ```
fn get_element_info(field_ty: &Type) -> (bool, bool, bool, bool) {
    // List of known FHIR primitive type aliases that wrap Element or DecimalElement
    // Note: This list might need adjustment based on the specific FHIR version/implementation details.
    // IMPORTANT: Do not include base Rust types like "String", "bool", "i32" here.
    // This list is for type aliases that *wrap* fhir::Element or fhir::DecimalElement.
    const KNOWN_ELEMENT_ALIASES: &[&str] = &[
        "Base64Binary",
        "Boolean",
        "Canonical",
        "Code",
        "Date",
        "DateTime",
        "Id",
        "Instant",
        "Integer",
        "Markdown",
        "Oid",
        "PositiveInt",
        "String",
        "Time",
        "UnsignedInt",
        "Uri",
        "Url",
        "Uuid",
        "Xhtml",
        // Struct types that might be used directly or within Elements (e.g., Address, HumanName)
        // are NOT typically handled by this _fieldName logic, so they are excluded here.
        // Resource types (Patient, Observation) are also excluded.
    ];
    const KNOWN_DECIMAL_ELEMENT_ALIAS: &str = "Decimal";

    let mut is_option = false;
    let mut is_vec = false;
    let mut current_ty = field_ty;

    // Unwrap Option
    if let Some(inner) = get_option_inner_type(current_ty) {
        is_option = true;
        current_ty = inner;
    }

    // Unwrap Vec
    if let Some(inner) = get_vec_inner_type(current_ty) {
        is_vec = true;
        current_ty = inner;
        // Check if Vec contains Option<Element>
        if let Some(vec_option_inner) = get_option_inner_type(current_ty) {
            current_ty = vec_option_inner; // Now current_ty is the Element type inside Vec<Option<...>>
        } else {
            // If it's Vec<Element> directly (less common for primitives), handle it
            // current_ty is already the Element type inside Vec<...>
        }
    }

    // Unwrap Box if present (e.g., Box<Reference> inside Element)
    if let Some(inner) = get_box_inner_type(current_ty) {
        current_ty = inner;
    }

    // Check if the (potentially unwrapped) type path ends with Element or DecimalElement
    if let Type::Path(TypePath { path, .. }) = current_ty {
        if let Some(segment) = path.segments.last() {
            let type_name_ident = &segment.ident;
            let type_name_str = type_name_ident.to_string();

            // Check if the last segment's identifier is Element, DecimalElement, or a known alias
            let is_direct_element = type_name_str == "Element";
            let is_direct_decimal_element = type_name_str == "DecimalElement";
            let is_known_element_alias = KNOWN_ELEMENT_ALIASES.contains(&type_name_str.as_str());
            let is_known_decimal_alias = type_name_str == KNOWN_DECIMAL_ELEMENT_ALIAS;

            let is_element = is_direct_element || is_known_element_alias;
            let is_decimal_element = is_direct_decimal_element || is_known_decimal_alias;

            if is_element || is_decimal_element {
                // It's considered an element type if it's Element, DecimalElement, or a known alias
                return (
                    is_element && !is_decimal_element, // Ensure is_element is false if it's a decimal type
                    is_decimal_element,
                    is_option,
                    is_vec,
                );
            }
        }
    }

    (false, false, is_option, is_vec) // Not an Element or DecimalElement type we handle specially
}

// Keep this in sync with generate_primitive_type in fhir_gen/src/lib.rs
// Helper function to get the inner type T from Option<T>, Vec<T>, or Box<T>
fn get_inner_type(ty: &Type) -> Option<&Type> {
    if let Type::Path(TypePath { path, .. }) = ty {
        if let Some(segment) = path.segments.last() {
            if segment.ident == "Option" || segment.ident == "Vec" || segment.ident == "Box" {
                if let PathArguments::AngleBracketed(args) = &segment.arguments {
                    if let Some(GenericArgument::Type(inner_ty)) = args.args.first() {
                        return Some(inner_ty);
                    }
                }
            }
        }
    }
    None
}

// Helper function to recursively unwrap Option, Vec, and Box to get the base type
fn get_base_type(ty: &Type) -> &Type {
    let mut current_ty = ty;
    while let Some(inner) = get_inner_type(current_ty) {
        current_ty = inner;
    }
    current_ty
}

fn extract_inner_element_type(type_name: &str) -> &str {
    match type_name {
        "Boolean" => "bool",
        "Integer" | "PositiveInt" | "UnsignedInt" => "std::primitive::i32",
        "Decimal" => "rust_decimal::Decimal", // Use the actual Decimal type
        "Integer64" => "std::primitive::i64",
        "String" | "Code" | "Base64Binary" | "Canonical" | "Id" | "Oid" | "Uri" | "Url"
        | "Uuid" | "Markdown" | "Xhtml" => "std::string::String",
        "Date" => "crate::PrecisionDate",
        "DateTime" => "crate::PrecisionDateTime",
        "Instant" => "crate::PrecisionInstant",
        "Time" => "crate::PrecisionTime",
        _ => "std::string::String", // Default or consider panic/error
    }
}

//=============================================================================
// FhirSerde Implementation Generator Functions
//=============================================================================

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
fn generate_serialize_impl(data: &Data, name: &Ident) -> proc_macro2::TokenStream {
    match *data {
        Data::Enum(ref data) => {
            // Handle enum serialization
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
                                Self::#variant_name(value) => {
                                    // Check if the element has id or extension that needs to be serialized
                                    let has_extension = value.id.is_some() || value.extension.is_some();
                                    // Serialize the primitive value
                                    if value.value.is_some() {
                                        // Use serialize_entry for SerializeMap
                                        state.serialize_entry(#variant_key, &value.value)?;
                                    }
                                    // Serialize the extension part if present
                                    if has_extension {
                                        #[derive(serde::Serialize)]
                                        struct IdAndExtensionHelper<'a> {
                                            #[serde(skip_serializing_if = "Option::is_none")]
                                            id: &'a Option<std::string::String>,
                                            #[serde(skip_serializing_if = "Option::is_none")]
                                            extension: &'a Option<Vec<Extension>>,
                                        }
                                        let extension_part = IdAndExtensionHelper {
                                            id: &value.id,
                                            extension: &value.extension,
                                        };
                                        // Use serialize_entry for SerializeMap
                                        state.serialize_entry(#underscore_variant_key, &extension_part)?;
                                    }
                                    // Don't return Result here, just continue
                                }
                            });
                        } else {
                            // Regular newtype variant
                            match_arms.push(quote! {
                                // Removed 'ref' from pattern
                                Self::#variant_name(value) => {
                                    state.serialize_entry(#variant_key, value)?;
                                }
                            });
                        }
                    }
                    Fields::Unnamed(_) => {
                        // Tuple variant with multiple fields
                        match_arms.push(quote! {
                            Self::#variant_name(ref value) => {
                                state.serialize_entry(#variant_key, value)?;
                            }
                        });
                    }
                    Fields::Named(_fields) => {
                        // Struct variant
                        match_arms.push(quote! {
                            Self::#variant_name { .. } => {
                                state.serialize_entry(#variant_key, self)?;
                            }
                        });
                    }
                    Fields::Unit => {
                        // Unit variant
                        match_arms.push(quote! {
                            Self::#variant_name => {
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
                        quote! {
                            use serde::ser::SerializeMap;
                            use crate::serde_helpers;
                        }
                    } else {
                        quote! {
                            use serde::ser::SerializeStruct;
                            use crate::serde_helpers;
                        }
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
                            // For flattened fields, use FlatMapSerializer
                            quote! {
                                // Use serde::Serialize::serialize with FlatMapSerializer
                                serde::Serialize::serialize(
                                    &#field_access,
                                    serde::__private::ser::FlatMapSerializer(&mut state)
                                )?;
                            }
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
                                                    match serde_json::to_value(value) {
                                                        Ok(json_val) => primitive_array.push(json_val),
                                                        Err(e) => return Err(serde::ser::Error::custom(format!("Failed to serialize primitive value: {}", e))),
                                                    }
                                                },
                                                None => primitive_array.push(serde_json::Value::Null),
                                            }

                                            // Check if this element has id or extension
                                            if element.id.is_some() || element.extension.is_some() {
                                                has_extensions = true;
                                                // Use helper struct for consistent serialization of id/extension
                                                #[derive(serde::Serialize)]
                                                struct IdAndExtensionHelper<'a> {
                                                    #[serde(skip_serializing_if = "Option::is_none")]
                                                    id: &'a Option<std::string::String>,
                                                    #[serde(skip_serializing_if = "Option::is_none")]
                                                    extension: &'a Option<Vec<Extension>>,
                                                }
                                                let extension_part = IdAndExtensionHelper {
                                                    id: &element.id,
                                                    extension: &element.extension,
                                                };
                                                // Serialize the helper and push null if it serializes to null (e.g., both fields are None)
                                                match serde_json::to_value(&extension_part) {
                                                    Ok(json_val) if !json_val.is_null() => extension_array.push(json_val),
                                                    Ok(_) => extension_array.push(serde_json::Value::Null), // Push null if helper serialized to null
                                                    Err(e) => return Err(serde::ser::Error::custom(format!("Failed to serialize extension part: {}", e))),
                                                }
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
                                            // Use serialize_entry for SerializeMap
                                            state.serialize_entry(&#effective_field_name_str, value)?;
                                        }
                                        if #extension_field_ident {
                                            #[derive(serde::Serialize)]
                                            struct IdAndExtensionHelper<'a> {
                                                #[serde(skip_serializing_if = "Option::is_none")]
                                                id: &'a Option<std::string::String>,
                                                #[serde(skip_serializing_if = "Option::is_none")]
                                                extension: &'a Option<Vec<Extension>>,
                                            }
                                            let extension_part = IdAndExtensionHelper {
                                                id: &field.id,
                                                extension: &field.extension,
                                            };
                                            // Use serialize_entry for SerializeMap
                                            // No format! here, #underscore_field_name_str is already a string literal
                                            state.serialize_entry(&#underscore_field_name_str, &extension_part)?;
                                        }
                                    }
                                }
                            } else {
                                // For SerializeStruct
                                quote! {
                                    if let Some(field) = &#field_access {
                                        if let Some(value) = field.value.as_ref() {
                                            // Use serialize_field for SerializeStruct
                                            state.serialize_field(&#effective_field_name_str, value)?;
                                        }
                                        if #extension_field_ident {
                                            #[derive(serde::Serialize)]
                                            struct IdAndExtensionHelper<'a> {
                                                #[serde(skip_serializing_if = "Option::is_none")]
                                                id: &'a Option<std::string::String>,
                                                #[serde(skip_serializing_if = "Option::is_none")]
                                                extension: &'a Option<Vec<Extension>>,
                                            }
                                            let extension_part = IdAndExtensionHelper {
                                                id: &field.id,
                                                extension: &field.extension,
                                            };
                                            // Use serialize_field for SerializeStruct
                                            // No format! here, #underscore_field_name_str is already a string literal
                                            state.serialize_field(&#underscore_field_name_str, &extension_part)?;
                                        }
                                    }
                                }
                            }
                        } else if !is_vec && is_fhir_element {
                            if has_flattened_fields {
                                // For SerializeMap
                                quote! {
                                    if let Some(value) = #field_access.value.as_ref() {
                                        // Use serialize_entry for SerializeMap
                                        state.serialize_entry(&#effective_field_name_str, value)?;
                                    }
                                    if #extension_field_ident {
                                        #[derive(serde::Serialize)]
                                        struct IdAndExtensionHelper<'a> {
                                            #[serde(skip_serializing_if = "Option::is_none")]
                                            id: &'a Option<std::string::String>,
                                            #[serde(skip_serializing_if = "Option::is_none")]
                                            extension: &'a Option<Vec<Extension>>,
                                        }
                                        let extension_part = IdAndExtensionHelper {
                                            id: &#field_access.id,
                                            extension: &#field_access.extension,
                                        };
                                        // Use serialize_entry for SerializeMap
                                        state.serialize_entry(#underscore_field_name_str, &extension_part)?;
                                    }
                                }
                            } else {
                                // For SerializeStruct
                                quote! {
                                    if let Some(value) = #field_access.value.as_ref() {
                                        // Use serialize_field for SerializeStruct
                                        state.serialize_field(&#effective_field_name_str, value)?;
                                    }
                                    if #extension_field_ident {
                                        #[derive(serde::Serialize)]
                                        struct IdAndExtensionHelper<'a> {
                                            #[serde(skip_serializing_if = "Option::is_none")]
                                            id: &'a Option<std::string::String>,
                                            #[serde(skip_serializing_if = "Option::is_none")]
                                            extension: &'a Option<Vec<Extension>>,
                                        }
                                        let extension_part = IdAndExtensionHelper {
                                            id: &#field_access.id,
                                            extension: &#field_access.extension,
                                        };
                                        // Use serialize_field for SerializeStruct
                                        // No format! here, #underscore_field_name_str is already a string literal
                                        state.serialize_field(&#underscore_field_name_str, &extension_part)?;
                                    }
                                }
                            }
                        } else if is_option {
                            // Skip serializing if the Option is None
                            if has_flattened_fields {
                                // For SerializeMap
                                quote! {
                                    if let Some(value) = &#field_access {
                                        // Use serialize_entry for SerializeMap
                                        state.serialize_entry(&#effective_field_name_str, value)?;
                                    }
                                }
                            } else {
                                // For SerializeStruct
                                quote! {
                                    if let Some(value) = &#field_access {
                                        // Use serialize_field for SerializeStruct
                                        state.serialize_field(&#effective_field_name_str, value)?;
                                    }
                                }
                            }
                        } else if is_vec {
                            // Regular Vec handling (not Element)
                            if has_flattened_fields {
                                // For SerializeMap
                                quote! {
                                    if !#field_access.is_empty() {
                                        // Use serialize_entry for SerializeMap
                                        state.serialize_entry(&#effective_field_name_str, &#field_access)?;
                                    }
                                }
                            } else {
                                // For SerializeStruct
                                quote! {
                                    if !#field_access.is_empty() {
                                        // Use serialize_field for SerializeStruct
                                        state.serialize_field(&#effective_field_name_str, &#field_access)?;
                                    }
                                }
                            }
                        } else {
                            // For non-Option types, check if it's a struct with all None/null fields
                            if has_flattened_fields {
                                // For SerializeMap
                                quote! {
                                    if crate::serde_helpers::has_non_empty_content(&#field_access) {
                                        // Use serialize_entry for SerializeMap
                                        state.serialize_entry(&#effective_field_name_str, &#field_access)?;
                                    }
                                }
                            } else {
                                // For SerializeStruct
                                quote! {
                                    if crate::serde_helpers::has_non_empty_content(&#field_access) {
                                        // Use serialize_field for SerializeStruct
                                        state.serialize_field(&#effective_field_name_str, &#field_access)?;
                                    }
                                }
                            }
                        };

                        field_counts.push(field_counting_code);
                        field_serializers.push(field_serializing_code);
                    }
                    if has_flattened_fields {
                        // If we have flattened fields, use serialize_map instead of serialize_struct
                        quote! {
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

#[cfg(test)]
mod tests {
    use super::*;
    use syn::{Type, parse_str};

    #[test]
    fn test_get_element_info_option_element() {
        let ty: Type = parse_str("Option<Element<Markdown, Extension>>").unwrap();
        let (is_element, is_decimal, is_option, is_vec) = get_element_info(&ty);
        assert!(is_element);
        assert!(!is_decimal);
        assert!(is_option);
        assert!(!is_vec);
    }

    #[test]
    fn test_get_element_info_option_decimal_element() {
        let ty: Type = parse_str("Option<DecimalElement<Extension>>").unwrap();
        let (is_element, is_decimal, is_option, is_vec) = get_element_info(&ty);
        assert!(!is_element);
        assert!(is_decimal);
        assert!(is_option);
        assert!(!is_vec);
    }

    #[test]
    fn test_get_element_info_option_markdown() {
        let ty: Type = parse_str("Option<Markdown>").unwrap();
        let (is_element, is_decimal, is_option, is_vec) = get_element_info(&ty);
        assert!(is_element); // Markdown should be identified as Element
        assert!(!is_decimal);
        assert!(is_option); // It is an Option
        assert!(!is_vec);
    }

    #[test]
    fn test_get_element_info_option_vec_option_element() {
        let ty: Type = parse_str("Option<Vec<Option<Element<bool, Extension>>>>").unwrap();
        let (is_element, is_decimal, is_option, is_vec) = get_element_info(&ty);
        assert!(is_element);
        assert!(!is_decimal);
        assert!(is_option); // Outer Option
        assert!(is_vec); // Vec is present
    }

    #[test]
    fn test_get_element_info_option_vec_option_decimal_element() {
        let ty: Type = parse_str("Option<Vec<Option<DecimalElement<Extension>>>>").unwrap();
        let (is_element, is_decimal, is_option, is_vec) = get_element_info(&ty);
        assert!(!is_element);
        assert!(is_decimal);
        assert!(is_option); // Outer Option
        assert!(is_vec); // Vec is present
    }

    #[test]
    fn test_get_element_info_option_vec_markdown() {
        let ty: Type = parse_str("Option<Vec<Markdown>>").unwrap();
        let (is_element, is_decimal, is_option, is_vec) = get_element_info(&ty);
        assert!(is_element); // Markdown should be identified as Element
        assert!(!is_decimal);
        assert!(is_option); // Outer Option
        assert!(is_vec); // Vec is present
    }

    #[test]
    fn test_get_element_info_element() {
        let ty: Type = parse_str("Element<String, Extension>").unwrap();
        let (is_element, is_decimal, is_option, is_vec) = get_element_info(&ty);
        assert!(is_element);
        assert!(!is_decimal);
        assert!(!is_option);
        assert!(!is_vec);
    }

    #[test]
    fn test_get_element_info_decimal_element() {
        let ty: Type = parse_str("DecimalElement<Extension>").unwrap();
        let (is_element, is_decimal, is_option, is_vec) = get_element_info(&ty);
        assert!(!is_element);
        assert!(is_decimal);
        assert!(!is_option);
        assert!(!is_vec);
    }

    #[test]
    fn test_get_element_info_markdown() {
        let ty: Type = parse_str("Markdown").unwrap();
        let (is_element, is_decimal, is_option, is_vec) = get_element_info(&ty);
        assert!(is_element); // Markdown should be identified as Element
        assert!(!is_decimal);
        assert!(!is_option);
        assert!(!is_vec);
    }

    #[test]
    fn test_get_element_info_vec_option_element() {
        // Less common, but test Vec<Option<Element>> without outer Option
        let ty: Type = parse_str("Vec<Option<Element<bool, Extension>>>").unwrap();
        let (is_element, is_decimal, is_option, is_vec) = get_element_info(&ty);
        assert!(is_element);
        assert!(!is_decimal);
        assert!(!is_option); // No outer Option
        assert!(is_vec); // Vec is present
    }

    #[test]
    fn test_get_element_info_vec_option_decimal_element() {
        let ty: Type = parse_str("Vec<Option<DecimalElement<Extension>>>").unwrap();
        let (is_element, is_decimal, is_option, is_vec) = get_element_info(&ty);
        assert!(!is_element);
        assert!(is_decimal);
        assert!(!is_option); // No outer Option
        assert!(is_vec); // Vec is present
    }

    #[test]
    fn test_get_element_info_vec_string() {
        let ty: Type = parse_str("Vec<String>").unwrap();
        let (is_element, is_decimal, is_option, is_vec) = get_element_info(&ty);
        // String IS identified as Element because it's in KNOWN_ELEMENT_ALIASES
        assert!(is_element);
        assert!(!is_decimal);
        assert!(!is_option);
        assert!(is_vec);
    }

    #[test]
    fn test_get_element_info_option_box_element() {
        // Test with Box wrapping
        let ty: Type = parse_str("Option<Box<Element<String, Extension>>>").unwrap();
        let (is_element, is_decimal, is_option, is_vec) = get_element_info(&ty);
        assert!(is_element);
        assert!(!is_decimal);
        assert!(is_option);
        assert!(!is_vec);
    }

    #[test]
    fn test_get_element_info_option_vec_option_box_element() {
        // Test with Box inside Vec<Option<...>>
        let ty: Type = parse_str("Option<Vec<Option<Box<Element<bool, Extension>>>>>").unwrap();
        let (is_element, is_decimal, is_option, is_vec) = get_element_info(&ty);
        assert!(is_element);
        assert!(!is_decimal);
        assert!(is_option); // Outer Option
        assert!(is_vec); // Vec is present
    }

    #[test]
    fn test_get_element_info_type_alias() {
        // Simulate a type alias like `type Date = Element<String, Extension>;`
        // We parse the underlying type directly here. The function should resolve it.
        let _ty: Type = parse_str("fhir::r4::Date").unwrap(); // Prefix unused variable
        // We can't directly test the alias resolution here without more context,
        // but we can test if it correctly identifies an Element path.
        // This test assumes `fhir::r4::Date` *looks like* an Element path segment.
        // A more robust test would involve actual type resolution which is complex in macros.

        // Let's test a path that *ends* with Element, simulating an alias.
        let _ty_path: Type = parse_str("some::module::MyElementAlias").unwrap(); // Prefix unused variable
        // Manually construct a scenario where the last segment is "Element"
        // This is a simplification as we don't have real type info.
        let ty_simulated_alias: Type = parse_str("Element<String, Extension>").unwrap();

        // Test with a path that *doesn't* end in Element/DecimalElement
        let ty_non_element_path: Type = parse_str("some::module::RegularStruct").unwrap();
        let (is_element, is_decimal, is_option, is_vec) = get_element_info(&ty_non_element_path);
        assert!(!is_element);
        assert!(!is_decimal);
        assert!(!is_option);
        assert!(!is_vec);

        // Test with a path that *does* end in Element (simulating alias)
        // We use the actual Element type parsed earlier for this simulation
        let (is_element, is_decimal, is_option, is_vec) = get_element_info(&ty_simulated_alias);
        assert!(is_element);
        assert!(!is_decimal);
        assert!(!is_option);
        assert!(!is_vec);
    }

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
        let serialize_impl = generate_serialize_impl(&input.data, name);

        // Convert to string to check if FlatMapSerializer is used
        let serialize_impl_str = serialize_impl.to_string();

        // Check that FlatMapSerializer is used for the flattened field
        assert!(serialize_impl_str.contains("FlatMapSerializer"));

        // Check that regular serialization uses serialize_entry when flattening is active (due to serialize_map)
        assert!(serialize_impl_str.contains("serialize_entry"));
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
/// TokenStream containing the complete `deserialize` method implementation.
fn generate_deserialize_impl(data: &Data, name: &Ident) -> proc_macro2::TokenStream {
    let struct_name = format_ident!("Temp{}", name);

    let mut temp_struct_attributes = Vec::new();
    let mut constructor_attributes = Vec::new();

    match *data {
        Data::Enum(ref data) => {
            // For enums, we need to deserialize from a map with a single key-value pair
            // where the key is the variant name and the value is the variant data

            // Generate a visitor for the enum
            let enum_name = name.to_string();
            let variants = &data.variants;

            let mut variant_matches = Vec::new(); // Stores the generated match arms
            let mut variant_names = Vec::new(); // Stores the string names for error messages/expecting

            for variant in variants {
                let variant_name = &variant.ident; // The Ident (e.g., String)
                let variant_name_str = variant_name.to_string();

                // Get the rename attribute if present
                let mut rename = None;
                for attr in &variant.attrs {
                    if attr.path().is_ident("fhir_serde") {
                        if let Ok(list) =
                            attr.parse_args_with(Punctuated::<Meta, token::Comma>::parse_terminated)
                        {
                            for meta in list {
                                if let Meta::NameValue(nv) = meta {
                                    if nv.path.is_ident("rename") {
                                        if let syn::Expr::Lit(expr_lit) = nv.value {
                                            if let Lit::Str(lit_str) = expr_lit.lit {
                                                rename = Some(lit_str.value());
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if rename.is_some() {
                        break;
                    }
                }

                // Use the rename value or the variant name as a string for the JSON key
                let variant_key = rename.unwrap_or_else(|| variant_name_str.clone());
                variant_names.push(variant_key.clone()); // Keep track of expected keys

                // Generate the specific deserialization logic for this variant
                let deserialization_logic = match &variant.fields {
                    Fields::Unnamed(fields) if fields.unnamed.len() == 1 => {
                        // Newtype variant (e.g., String(String))
                        let field = fields.unnamed.first().unwrap();
                        let field_ty = &field.ty;
                        let (is_element, is_decimal_element, _, _) = get_element_info(field_ty);

                        if is_element || is_decimal_element {
                            // --- Element/DecimalElement Variant Construction ---
                            let underscore_variant_key_str = format!("_{}", variant_key); // For error messages

                            // Determine the primitive type V or PreciseDecimal for the value field
                            let primitive_type_for_element = if is_decimal_element {
                                quote! { crate::PreciseDecimal }
                            } else {
                                // Extract V from Element<V, E> or the alias's underlying primitive
                                // Need to re-determine the base type here
                                let base_type = get_base_type(field_ty);
                                if let Type::Path(type_path) = base_type {
                                    if let Some(last_segment) = type_path.path.segments.last() {
                                        if last_segment.ident == "Element" {
                                            // Direct Element<V, E>
                                            if let PathArguments::AngleBracketed(generics) =
                                                &last_segment.arguments
                                            {
                                                if let Some(GenericArgument::Type(inner_v_type)) =
                                                    generics.args.first()
                                                {
                                                    quote! { #inner_v_type }
                                                } else {
                                                    panic!("Element missing generic argument V");
                                                }
                                            } else {
                                                panic!("Element missing angle bracketed arguments");
                                            }
                                        } else {
                                            // Alias
                                            let alias_name = last_segment.ident.to_string();
                                            let primitive_type_str =
                                                extract_inner_element_type(&alias_name);
                                            let primitive_type_parsed: Type = syn::parse_str(
                                                primitive_type_str,
                                            )
                                            .expect("Failed to parse primitive type string");
                                            quote! { #primitive_type_parsed }
                                        }
                                    } else {
                                        panic!("Could not get last segment of Element type path");
                                    }
                                } else {
                                    panic!("Element type is not a Type::Path");
                                }
                            };

                            quote! {
                                // Check if parts exist *before* potentially moving them
                                let has_value_part = value_part.is_some();
                                let has_extension_part = extension_part.is_some();

                                // Deserialize the extension part if present
                                let mut ext_helper_opt: Option<IdAndExtensionHelper> = None;
                                if let Some(ext_value) = extension_part { // Move happens here
                                    ext_helper_opt = Some(serde::Deserialize::deserialize(ext_value)
                                        .map_err(|e| serde::de::Error::custom(format!("Error deserializing extension {}: {}", #underscore_variant_key_str, e)))?);
                                }

                                // Deserialize the value part if present, consuming value_part
                                let deserialized_value_opt = if let Some(prim_value) = value_part { // Move of value_part happens here
                                    // Use #primitive_type_for_element determined outside
                                    Some(<#primitive_type_for_element>::deserialize(prim_value)
                                         .map_err(|e| serde::de::Error::custom(format!("Error deserializing primitive {}: {}", #variant_key, e)))?)
                                } else {
                                    None::<#primitive_type_for_element> // Explicit type needed for None
                                };

                                // Construct the element using deserialized parts
                                let mut element: #field_ty = Default::default(); // Start with default

                                // Assign deserialized value
                                element.value = deserialized_value_opt; // Assign the Option<V> or Option<PreciseDecimal>

                                // Merge the extension data if it exists
                                if let Some(ext_helper) = ext_helper_opt {
                                    if ext_helper.id.is_some() {
                                        element.id = ext_helper.id;
                                    }
                                    if ext_helper.extension.is_some() {
                                        element.extension = ext_helper.extension;
                                    }
                                }
                                // Note: The check `if !has_value_part && has_extension_part { element.value = None; }`
                                // is now redundant because element.value is already None if !has_value_part.

                                Ok(#name::#variant_name(element))
                            }
                            // --- End Element/DecimalElement Variant Construction ---
                        } else {
                            // --- Regular Newtype Variant Construction ---
                            quote! {
                                let value = value_part.ok_or_else(|| serde::de::Error::missing_field(#variant_key))?;
                                let inner_value = serde::Deserialize::deserialize(value)
                                    .map_err(|e| serde::de::Error::custom(format!("Error deserializing non-element variant {}: {}", #variant_key, e)))?;
                                Ok(#name::#variant_name(inner_value)) // Removed .into()
                            }
                            // --- End Regular Newtype Variant Construction ---
                        }
                    }
                    Fields::Unnamed(_) => {
                        // Tuple variant
                        quote! {
                            let value = value_part.ok_or_else(|| serde::de::Error::missing_field(#variant_key))?;
                            let inner_value = serde::Deserialize::deserialize(value)
                                .map_err(|e| serde::de::Error::custom(format!("Error deserializing tuple variant {}: {}", #variant_key, e)))?;
                            Ok(#name::#variant_name(inner_value)) // Use variant_name directly
                        }
                    }
                    Fields::Named(_) => {
                        // Struct variant
                        quote! {
                            let value = value_part.ok_or_else(|| serde::de::Error::missing_field(#variant_key))?;
                            let inner_value = serde::Deserialize::deserialize(value)
                                .map_err(|e| serde::de::Error::custom(format!("Error deserializing struct variant {}: {}", #variant_key, e)))?;
                            Ok(#name::#variant_name(inner_value)) // Use variant_name directly
                        }
                    }
                    Fields::Unit => {
                        // Unit variant
                        quote! {
                            Ok(#name::#variant_name) // Use variant_name directly
                        }
                    }
                }; // End match variant.fields

                // Push the complete match arm
                variant_matches.push(quote! {
                    #variant_key => { // Use the string key as the match pattern
                        #deserialization_logic // Embed the generated logic block
                    }
                });
            } // End loop over variants

            // Define the helper struct needed for enum deserialization
            let id_extension_helper_def = quote! {
                // Helper struct for deserializing the id/extension part from _fieldName
                #[derive(Clone, Deserialize, Default)] // Add Default derive
                struct IdAndExtensionHelper {
                    #[serde(skip_serializing_if = "Option::is_none")] // Change from default
                    id: Option<std::string::String>,
                    #[serde(skip_serializing_if = "Option::is_none")] // Change from default
                    extension: Option<Vec<Extension>>,
                }
            };

            // Generate the enum deserialization implementation
            return quote! {
                // Import necessary crates/modules at the top level of the impl block
                use serde::{Deserialize, de::{self, Visitor, MapAccess}};
                use serde_json; // Needed for Value
                use std::collections::HashSet; // Needed for processed_keys
                // NOTE: Removed `use syn;` as it's not needed at runtime

                // Define the helper struct at the top level of the impl block
                #id_extension_helper_def

                // Define a visitor for the enum (no longer needs variants reference)
                struct EnumVisitor; // Removed lifetime and variants field

                impl<'de> serde::de::Visitor<'de> for EnumVisitor { // Removed lifetime 'a
                    type Value = #name;

                    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                        formatter.write_str(concat!("a ", #enum_name, " enum"))
                    }

                    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
                    where
                        A: serde::de::MapAccess<'de>,
                    {
                        let mut found_variant_key: Option<std::string::String> = None;
                        let mut value_part: Option<serde_json::Value> = None;
                        let mut extension_part: Option<serde_json::Value> = None;
                        let mut processed_keys = std::collections::HashSet::new(); // Track processed keys

                        // Iterate through map entries directly, deserializing key as Value
                        while let Some((key_value, current_value)) = map.next_entry::<serde_json::Value, serde_json::Value>()? {
                            // Ensure the key is a string
                            let key_str = match key_value {
                                serde_json::Value::String(s) => s,
                                _ => return Err(serde::de::Error::invalid_type(serde::de::Unexpected::Other("non-string key"), &"a string key")),
                            };

                            let mut key_matched = false;
                            #( // Loop over variant_names (&'static str)
                                let base_name = #variant_names; // e.g., "authorString"
                                let underscore_name = format!("_{}", base_name); // e.g., "_authorString"

                                if key_str.as_str() == base_name { // Compare &str == &'static str
                                    if value_part.is_some() {
                                        return Err(serde::de::Error::duplicate_field(base_name));
                                    }
                                    value_part = Some(current_value.clone()); // Store the value
                                    // If we already found a key based on the underscore version, ensure it matches
                                    if let Some(ref existing_key) = found_variant_key {
                                        if existing_key != base_name {
                                             // Use key_str.as_str() for formatting
                                             return Err(serde::de::Error::custom(format!("Mismatched keys found: {} and {}", existing_key, key_str.as_str())));
                                        }
                                    } else {
                                        found_variant_key = Some(base_name.to_string());
                                    }
                                    processed_keys.insert(key_str.clone()); // Clone the String key
                                    key_matched = true;
                                } else if key_str.as_str() == underscore_name.as_str() { // Compare &str == &str
                                    if extension_part.is_some() {
                                        // Use custom error message as duplicate_field requires 'static str
                                        return Err(serde::de::Error::custom(format!("duplicate field '{}'", key_str)));
                                    }
                                    extension_part = Some(current_value.clone()); // Store the extension value
                                    // If we already found a key based on the base version, ensure it matches
                                     if let Some(ref existing_key) = found_variant_key {
                                        if existing_key != base_name {
                                             // Use key_str.as_str() for formatting
                                             return Err(serde::de::Error::custom(format!("Mismatched keys found: {} and {}", existing_key, key_str.as_str())));
                                        }
                                    } else {
                                        found_variant_key = Some(base_name.to_string()); // Store the BASE name
                                    }
                                    processed_keys.insert(key_str.clone());
                                    key_matched = true;
                                }
                            )*
                            // If the key didn't match any expected variant key (base or underscore), ignore it?
                            // Or error? Let's ignore for now, assuming other fields might be present.
                            // if !key_matched {
                            //     // Handle unexpected fields if necessary
                            // }
                        }

                        // Ensure a variant key was found
                        let variant_key = match found_variant_key {
                            Some(key) => key, // key is the base name (String)
                            None => {
                                // No matching key found at all
                                return Err(serde::de::Error::custom(format!(
                                    "Expected one of the variant keys {:?} (or their underscore-prefixed versions) but found none",
                                    [#(#variant_names),*]
                                )));
                            }
                        };

                        // --- Construct the variant based on found_variant_key, value_part, extension_part ---
                        match variant_key.as_str() {
                            // Use the pre-generated match arms
                            #(#variant_matches)*

                            // Fallback for unknown variant key (should not be reached if logic above is correct)
                            _ => Err(serde::de::Error::unknown_variant(&variant_key, &[#(#variant_names),*])),
                        }
                    }
                }

                // Use the visitor to deserialize the enum (no longer needs variants)
                deserializer.deserialize_map(EnumVisitor) // Removed variants passing
            };
        }
        Data::Struct(ref data) => {
            match data.fields {
                Fields::Named(ref fields) => {
                    for field in fields.named.iter() {
                        let field_name_ident = field.ident.as_ref().unwrap(); // Keep original ident for access
                        let field_name_ident_ext = format_ident!("{}_ext", field_name_ident);
                        let field_ty = &field.ty;
                        let effective_field_name_str = get_effective_field_name(field);
                        let _underscore_field_name_str =
                            format_ident!("_{}", effective_field_name_str);

                        // Destructure the 4 return values
                        let (is_element, is_decimal_element, is_option, is_vec) =
                            get_element_info(field_ty);

                        let is_fhir_element = is_element || is_decimal_element;

                        // Determine the type for the primitive value field in the temp struct
                        let temp_primitive_type_quote = if is_fhir_element {
                            // Need to re-determine the base type here
                            let base_type = get_base_type(field_ty);

                            // Determine the base primitive type (e.g., bool, String, rust_decimal::Decimal)
                            let primitive_type_ident = if is_decimal_element {
                                // For DecimalElement, use serde_json::Value in temp struct to preserve original string
                                quote! { serde_json::Value }
                            } else {
                                // is_element is true here
                                if let Type::Path(type_path) = base_type {
                                    if let Some(last_segment) = type_path.path.segments.last() {
                                        if last_segment.ident == "Element" {
                                            // Direct Element<V, E>
                                            if let PathArguments::AngleBracketed(generics) =
                                                &last_segment.arguments
                                            {
                                                if let Some(GenericArgument::Type(inner_v_type)) =
                                                    generics.args.first()
                                                {
                                                    quote! { #inner_v_type } // Use the inner type V directly
                                                } else {
                                                    panic!("Element missing generic argument V");
                                                }
                                            } else {
                                                panic!("Element missing angle bracketed arguments");
                                            }
                                        } else {
                                            // It's an alias like 'Code'. Get the primitive type it wraps.
                                            let alias_name = last_segment.ident.to_string();
                                            let primitive_type_str =
                                                extract_inner_element_type(&alias_name);
                                            // Parse the primitive type string back into a Type for quoting
                                            let primitive_type_parsed: Type = syn::parse_str(
                                                primitive_type_str,
                                            )
                                            .unwrap_or_else(|_| {
                                                panic!(
                                                    "Failed to parse primitive type string: {}",
                                                    primitive_type_str
                                                )
                                            });
                                            quote! { #primitive_type_parsed } // Use the parsed primitive type
                                        }
                                    } else {
                                        panic!("Could not get last segment of Element type path");
                                    }
                                } else {
                                    panic!("Element type is not a Type::Path");
                                }
                            }; // End of let primitive_type_ident assignment

                            // Adjust the quote based on whether it's a vector
                            if is_vec {
                                // Temp type should be Option<Vec<Option<Primitive>>> to handle nulls inside the array
                                quote! { Option<Vec<Option<#primitive_type_ident>>> } // Add Option inside Vec
                            } else {
                                // If original field was Option<T>, temp type is Option<Primitive>
                                // If original field was T, temp type is Primitive
                                if is_option {
                                    quote! { Option<#primitive_type_ident> }
                                } else {
                                    // Always use Option<Primitive> in temp struct for single elements
                                    quote! { Option<#primitive_type_ident> }
                                }
                            }
                        } else {
                            // Not an element, use the original type
                            quote! { #field_ty }
                        };

                        // Determine the type for the extension helper field in the temp struct
                        let temp_extension_type = if is_fhir_element {
                            if is_vec {
                                // For Vec<Element> or Option<Vec<Element>>, temp type is Option<Vec<Option<IdAndExtensionHelper>>>
                                quote! { Option<Vec<Option<IdAndExtensionHelper>>> }
                            } else {
                                // For Element or Option<Element>, temp type is Option<IdAndExtensionHelper>
                                quote! { Option<IdAndExtensionHelper> }
                            }
                        } else {
                            // Not an element, no extension helper needed
                            quote! { () } // Use unit type as placeholder, won't be generated anyway
                        };

                        // Create the string literal for the underscore field name
                        let underscore_field_name_literal =
                            format!("_{}", effective_field_name_str);

                        // Base attribute for the regular field (primitive value)
                        let base_attribute = quote! {
                            // Use default for Option types in the temp struct
                            #[serde(default, rename = #effective_field_name_str)]
                            #field_name_ident: #temp_primitive_type_quote, // Use the determined Option<V> or original type
                        };

                        // Conditionally add the underscore field attribute if it's an element type
                        let underscore_attribute = if is_fhir_element {
                            quote! {
                                // Use default for Option types in the temp struct
                                #[serde(default, rename = #underscore_field_name_literal)]
                                #field_name_ident_ext: #temp_extension_type,
                            }
                        } else {
                            quote! {} // Empty if not an element
                        };

                        // Combine the attributes for the temp struct
                        let flatten_attr = if is_flattened(field) {
                            quote! { #[serde(flatten)] }
                        } else {
                            quote! {}
                        };
                        let temp_struct_attribute = quote! {
                            #flatten_attr // Add flatten attribute if needed
                            #base_attribute
                            #underscore_attribute
                        };

                        let constructor_attribute = if is_fhir_element {
                            if is_vec {
                                // Handle Vec<Element> or Option<Vec<Element>> first
                                // Generate different construction logic based on whether it's decimal
                                let construction_logic = if is_decimal_element {
                                    // Logic specifically for Vec<DecimalElement> or Option<Vec<DecimalElement>>
                                    let element_type = {
                                        // Determine DecimalElement<E> type
                                        let vec_inner_type = if is_option {
                                            get_option_inner_type(field_ty)
                                        } else {
                                            Some(field_ty)
                                        }
                                        .and_then(get_vec_inner_type)
                                        .expect("Vec inner type not found for DecimalElement");
                                        quote! { #vec_inner_type }
                                    };
                                    quote! { { // Block expression starts
                                        // Handle Option for primitives and extensions
                                        let primitives = temp_struct.#field_name_ident.unwrap_or_default(); // Vec<Option<Primitive>>
                                        let extensions = temp_struct.#field_name_ident_ext.unwrap_or_default(); // Vec<Option<IdAndExtensionHelper>>
                                        let len = primitives.len().max(extensions.len());
                                        let mut result_vec = Vec::with_capacity(len);
                                        for i in 0..len {
                                            // Get Option<Primitive> by flattening the Option<Option<Primitive>> from the vec
                                            let prim_val_opt = primitives.get(i).cloned().flatten();
                                            let ext_helper_opt = extensions.get(i).cloned().flatten(); // Keep flatten here
                                            if prim_val_opt.is_some() || ext_helper_opt.is_some() {
                                                // Deserialize the Option<serde_json::Value> into Option<PreciseDecimal>
                                                let precise_decimal_value = match prim_val_opt {
                                                    Some(json_val) if !json_val.is_null() => {
                                                        // Map error explicitly
                                                        crate::PreciseDecimal::deserialize(json_val)
                                                            .map(Some)
                                                            .map_err(serde::de::Error::custom)? // Map error here
                                                    },
                                                    _ => None, // Treat None or JSON null as None
                                                };
                                                result_vec.push(#element_type {
                                                    value: precise_decimal_value,
                                                    id: ext_helper_opt.as_ref().and_then(|h| h.id.clone()),
                                                    extension: ext_helper_opt.as_ref().and_then(|h| h.extension.clone()),
                                                });
                                            }
                                            // Note: Skipping adding element if both parts are null/None
                                        }
                                        result_vec // Return the vec directly
                                    } } // Block expression ends
                                } else {
                                    // Logic specifically for Vec<Element<V, E>> or Option<Vec<Element<V, E>>> (non-decimal)
                                    let element_type = {
                                        // Determine Element<V, E> type
                                        let vec_inner_type = if is_option {
                                            get_option_inner_type(field_ty)
                                        } else {
                                            Some(field_ty)
                                        }
                                        .and_then(get_vec_inner_type)
                                        .expect("Vec inner type not found for Element");
                                        quote! { #vec_inner_type }
                                    };
                                    quote! { { // Block expression starts
                                        // Handle Option for primitives and extensions
                                        let primitives = temp_struct.#field_name_ident.unwrap_or_default(); // Vec<Option<Primitive>>
                                        let extensions = temp_struct.#field_name_ident_ext.unwrap_or_default(); // Vec<Option<IdAndExtensionHelper>>
                                        let len = primitives.len().max(extensions.len());
                                        let mut result_vec = Vec::with_capacity(len);
                                        for i in 0..len {
                                            // Get Option<Primitive> by flattening the Option<Option<Primitive>> from the vec
                                            let prim_val_opt = primitives.get(i).cloned().flatten();
                                            let ext_helper_opt = extensions.get(i).cloned().flatten(); // Keep flatten here
                                            if prim_val_opt.is_some() || ext_helper_opt.is_some() {
                                                result_vec.push(#element_type {
                                                    value: prim_val_opt, // Assign Option<V> directly
                                                    id: ext_helper_opt.as_ref().and_then(|h| h.id.clone()),
                                                    extension: ext_helper_opt.as_ref().and_then(|h| h.extension.clone()),
                                                });
                                            }
                                            // Note: Skipping adding element if both parts are null/None
                                        }
                                        result_vec
                                    } } // Block expression ends
                                }; // End of outer if/else determining construction_logic

                                // Assign the correct construction_logic based on is_option
                                if is_option {
                                    // For Option<Vec<Element>>, construct Some if either primitive or extension array was present
                                    quote! {
                                        #field_name_ident: if temp_struct.#field_name_ident.is_some() || temp_struct.#field_name_ident_ext.is_some() {
                                            // No '?' needed here as the block returns Vec<Element> directly
                                            Some(#construction_logic)
                                        } else {
                                            None
                                        },
                                    }
                                } else {
                                    // Direct Vec<Element> field assignment (always construct the Vec)
                                    quote! {
                                        // No '?' needed here as the block returns Vec<Element> directly
                                        #field_name_ident: #construction_logic,
                                    }
                                }
                            } else if is_decimal_element {
                                // Handle single DecimalElement or Option<DecimalElement>
                                if is_option {
                                    // Logic for Option<DecimalElement>
                                    let construction_logic = quote! { { // Block expression starts
                                        // Deserialize PreciseDecimal from Option<serde_json::Value>
                                        let precise_decimal_value = match temp_struct.#field_name_ident {
                                            Some(json_val) if !json_val.is_null() => {
                                                // Attempt deserialization, map error explicitly
                                                crate::PreciseDecimal::deserialize(json_val)
                                                    .map(Some)
                                                    .map_err(serde::de::Error::custom)? // Map error here
                                            },
                                            _ => None, // Treat None or JSON null as None
                                        };
                                        // Construct the DecimalElement (no Ok() needed)
                                        crate::DecimalElement {
                                            value: precise_decimal_value,
                                            id: temp_struct.#field_name_ident_ext.as_ref().and_then(|h| h.id.clone()),
                                            extension: temp_struct.#field_name_ident_ext.as_ref().and_then(|h| h.extension.clone()),
                                        }
                                    } }; // Block expression ends
                                    // Wrap in Some() only if value or extension exists
                                    quote! {
                                         #field_name_ident: if temp_struct.#field_name_ident.is_some() || temp_struct.#field_name_ident_ext.is_some() {
                                             // No '?' needed here as the block returns DecimalElement directly
                                             Some(#construction_logic)
                                         } else {
                                             None // If neither field present, result is None
                                         },
                                    }
                                } else {
                                    // Logic for non-optional DecimalElement
                                    quote! {
                                        #field_name_ident: { // Block expression starts
                                            // Deserialize PreciseDecimal from Option<serde_json::Value>
                                            let precise_decimal_value = match temp_struct.#field_name_ident {
                                                Some(json_val) if !json_val.is_null() => {
                                                    // Attempt deserialization, map error explicitly
                                                    crate::PreciseDecimal::deserialize(json_val)
                                                        .map(Some)
                                                        .map_err(serde::de::Error::custom)? // Map error here
                                                },
                                                _ => None, // Treat None or JSON null as None
                                            };
                                            // Construct the DecimalElement (no Ok() needed)
                                            crate::DecimalElement {
                                                value: precise_decimal_value,
                                                id: temp_struct.#field_name_ident_ext.as_ref().and_then(|h| h.id.clone()),
                                                extension: temp_struct.#field_name_ident_ext.as_ref().and_then(|h| h.extension.clone()),
                                            }
                                        }, // No '?' needed after block
                                    }
                                }
                            } else if is_option {
                                // Handle single Option<Element> (already checked !is_vec)
                                // Revert to simpler logic without explicit type annotation for value
                                // Get the inner type T from Option<T> to construct Element<V, E>
                                let inner_element_type = get_option_inner_type(field_ty)
                                    .expect("Option inner type not found");
                                quote! {
                                    #field_name_ident: if temp_struct.#field_name_ident.is_some() || temp_struct.#field_name_ident_ext.is_some() {
                                        Some(#inner_element_type { // Use the unwrapped Element type
                                            value: temp_struct.#field_name_ident, // Assign directly
                                            id: temp_struct.#field_name_ident_ext.as_ref().and_then(|h| h.id.clone()),
                                            extension: temp_struct.#field_name_ident_ext.as_ref().and_then(|h| h.extension.clone()),
                                        })
                                    } else {
                                        None // Assign None if neither value nor extension part exists
                                    },
                                }
                            } else {
                                // Handles Element<V, E> (non-option, non-vec)
                                // Construct element explicitly
                                quote! {
                                    #field_name_ident: {
                                        let mut element = #field_ty::default(); // Create default element (e.g., Code::default())
                                        element.value = temp_struct.#field_name_ident; // Assign Option<Primitive>
                                        // Assign id/extension from helper if present
                                        if let Some(helper) = temp_struct.#field_name_ident_ext {
                                            element.id = helper.id;
                                            element.extension = helper.extension;
                                        }
                                        element // Return the constructed element
                                    },
                                }
                            }
                        } else {
                            // Not an FHIR element type
                            quote! {
                                #field_name_ident: temp_struct.#field_name_ident,
                            }
                        }; // Semicolon ends the let constructor_attribute binding

                        temp_struct_attributes.push(temp_struct_attribute);
                        constructor_attributes.push(constructor_attribute); // Push the result
                    }
                }
                Fields::Unnamed(_) => panic!("Tuple structs not supported by FhirSerde"),
                Fields::Unit => panic!("Unit structs not supported by FhirSerde"),
            }
        }
        Data::Union(_) => panic!("Enums and Unions not supported by FhirSerde"),
    }

    let id_extension_helper_def = quote! {
        // Helper struct for deserializing the id/extension part from _fieldName
        #[derive(Clone, Deserialize, Default)] // Add Default derive
        struct IdAndExtensionHelper {
            #[serde(skip_serializing_if = "Option::is_none")] // Change from default
            id: Option<std::string::String>,
            #[serde(skip_serializing_if = "Option::is_none")] // Change from default
            extension: Option<Vec<Extension>>,
        }
    };

    let temp_struct = quote! {
        #[derive(Deserialize)]
        struct #struct_name {
            #(#temp_struct_attributes)*
        }
    };

    quote! {
        // Define the helper struct at the top level of the deserialize function
        #id_extension_helper_def

        // Define the temporary struct for deserialization
        #temp_struct

         // Perform the actual deserialization into the temporary struct
        let temp_struct = #struct_name::deserialize(deserializer)?;


        Ok(#name{#(#constructor_attributes)*})

    }
}

//=============================================================================
// FHIRPath Derive Macro and Implementation Functions
//=============================================================================

/// Derives the `helios_fhirpath_support::IntoEvaluationResult` trait for FHIRPath evaluation.
///
/// This procedural macro automatically generates implementations that convert FHIR
/// types into `EvaluationResult` objects that can be used in FHIRPath expressions.
/// This enables seamless integration between FHIR resources and the FHIRPath evaluator.
///
/// # Generated Implementations
///
/// ## For Structs:
/// - Converts struct fields to an `EvaluationResult::Object` with a HashMap
/// - Uses FHIR field names (respecting `#[fhir_serde(rename)]` attributes)
/// - Filters out empty/None fields to produce clean object representations
/// - Handles nested objects recursively through the trait
///
/// ## For Enums:
/// - **Choice types**: Delegates to the contained value's implementation
/// - **Resource enum**: Adds `resourceType` field automatically for resource variants
/// - **Unit variants**: Returns the variant name as a string (for status codes, etc.)
///
/// # FHIRPath Integration
///
/// The generated implementations enable FHIR resources to be used directly in
/// FHIRPath expressions such as:
/// - `Patient.name.family` - Access nested object properties
/// - `Observation.value.unit` - Access choice type properties  
/// - `Bundle.entry.resource.resourceType` - Access resource type discriminators
///
/// # Field Name Handling
///
/// Field names in the resulting object follow FHIR naming conventions:
/// - Uses `#[fhir_serde(rename = "name")]` if present
/// - Otherwise uses the raw Rust field identifier (not converted to camelCase)
/// - This ensures FHIRPath expressions match FHIR specification naming
///
/// # Examples
///
/// ```rust,ignore
/// use fhir_macro::FhirPath;
/// use helios_fhirpath_support::{IntoEvaluationResult, EvaluationResult};
///
/// #[derive(FhirPath)]
/// pub struct Patient {
///     pub id: Option<String>,
///     #[fhir_serde(rename = "implicitRules")]
///     pub implicit_rules: Option<Uri>,
///     pub active: Option<Boolean>,
/// }
///
/// // Usage in FHIRPath evaluation
/// let patient = Patient {
///     id: Some("123".to_string()),
///     active: Some(Boolean::from(true)),
///     implicit_rules: None,  // Filtered out
/// };
///
/// let result = patient.into_evaluation_result();
/// // Results in EvaluationResult::Object with:
/// // - "id" → "123"
/// // - "active" → true  
/// // - "implicitRules" field omitted (was None)
/// ```
///
/// # Resource Enum Special Handling
///
/// For the top-level `Resource` enum, the macro automatically adds the `resourceType`
/// field to enable proper FHIRPath resource type discrimination:
///
/// ```rust,ignore
/// #[derive(FhirPath)]
/// pub enum Resource {
///     Patient(Patient),
///     Observation(Observation),
/// }
///
/// // Resource::Patient(patient_data) becomes:
/// // {
/// //   "resourceType": "Patient",
/// //   ...patient_data fields...
/// // }
/// ```
///
/// # Empty Field Filtering
///
/// The generated implementation automatically filters out fields that evaluate to
/// `EvaluationResult::Empty`, ensuring clean object representations for FHIRPath
/// traversal. This includes:
/// - `None` values in `Option<T>` fields
/// - Empty collections
/// - Objects with no meaningful content
#[proc_macro_derive(FhirPath, attributes(fhir_serde, fhir_choice_element, fhir_resource))]
pub fn fhir_path_derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let generics = &input.generics;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let trait_impl = match &input.data {
        Data::Struct(data) => generate_fhirpath_struct_impl(
            name,
            data,
            &input.attrs,
            &impl_generics,
            &ty_generics,
            where_clause,
        ),
        Data::Enum(data) => generate_fhirpath_enum_impl(
            name,
            data,
            &input.attrs,
            &impl_generics,
            &ty_generics,
            where_clause,
        ),
        Data::Union(_) => panic!("FhirPath derive macro does not support unions."),
    };

    TokenStream::from(trait_impl)
}

/// Determines the effective field name for FHIRPath object property access.
///
/// This function extracts the field name that should be used as a property key
/// in the generated `EvaluationResult::Object`, ensuring that FHIRPath expressions
/// can access fields using their FHIR specification names.
///
/// # Attribute Processing
///
/// - If `#[fhir_serde(rename = "customName")]` is present, uses the custom name
/// - Otherwise, uses the raw Rust field identifier without case conversion
///
/// # Difference from Serialization
///
/// Unlike `get_effective_field_name()` which converts to camelCase for JSON
/// serialization, this function preserves exact FHIR names for FHIRPath access.
/// This ensures FHIRPath expressions match the FHIR specification exactly.
///
/// # Arguments
///
/// * `field` - The field definition from the parsed struct
///
/// # Returns
///
/// The field name as it should appear in FHIRPath object property access.
///
/// # Examples
///
/// ```rust,ignore
/// // Field: pub implicit_rules: Option<Uri>
/// // Result: "implicit_rules" (raw identifier)
///
/// // Field: #[fhir_serde(rename = "implicitRules")]
/// //        pub implicit_rules: Option<Uri>
/// // Result: "implicitRules" (explicit rename for FHIR compliance)
/// ```
fn get_fhirpath_field_name(field: &syn::Field) -> String {
    for attr in &field.attrs {
        if attr.path().is_ident("fhir_serde") {
            if let Ok(list) =
                attr.parse_args_with(Punctuated::<Meta, token::Comma>::parse_terminated)
            {
                for meta in list {
                    if let Meta::NameValue(nv) = meta {
                        if nv.path.is_ident("rename") {
                            if let syn::Expr::Lit(expr_lit) = nv.value {
                                if let Lit::Str(lit_str) = expr_lit.lit {
                                    return lit_str.value();
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    // Default to the raw field identifier if no rename attribute found
    field.ident.as_ref().unwrap().to_string()
}

fn generate_fhirpath_struct_impl(
    name: &Ident,
    data: &syn::DataStruct,
    attrs: &[syn::Attribute],
    impl_generics: &syn::ImplGenerics,
    ty_generics: &syn::TypeGenerics,
    where_clause: Option<&syn::WhereClause>,
) -> proc_macro2::TokenStream {
    let fields = match &data.fields {
        Fields::Named(fields) => &fields.named,
        _ => panic!("FhirPath derive macro only supports structs with named fields."),
    };

    let field_conversions = fields.iter().map(|field| {
        let field_name_ident = field.ident.as_ref().unwrap();
        let field_key_str = get_fhirpath_field_name(field); // Use the specific FHIRPath naming helper
        let field_ty = &field.ty; // Get the field type

        // Check if this field is flattened
        let is_field_flattened = is_flattened(field);

        // Check if this field is a FHIR primitive type that needs special handling
        let fhir_type_name = extract_fhir_primitive_type_name(field_ty);
        // Generate code to handle the field based on whether it's Option
        let is_option = get_option_inner_type(field_ty).is_some();

        // Special handling for flattened fields
        if is_field_flattened {
            // For flattened fields, we need to expand the inner object's fields into the parent map
            if is_option {
                quote! {
                    if let Some(inner_value) = &self.#field_name_ident {
                        let inner_result = inner_value.to_evaluation_result();
                        // If the inner result is an object, merge its fields into our map
                        if let helios_fhirpath_support::EvaluationResult::Object { map: inner_map, .. } = inner_result {
                            for (key, value) in inner_map {
                                map.insert(key, value);
                            }
                        }
                    }
                }
            } else {
                quote! {
                    let inner_result = self.#field_name_ident.to_evaluation_result();
                    // If the inner result is an object, merge its fields into our map
                    if let helios_fhirpath_support::EvaluationResult::Object { map: inner_map, .. } = inner_result {
                        for (key, value) in inner_map {
                            map.insert(key, value);
                        }
                    }
                }
            }
        } else if is_option {
            // For Option<T>, evaluate the inner value only if Some
            if let Some(type_name) = fhir_type_name {
                // Special handling for FHIR primitive types to preserve type information
                quote! {
                    if let Some(inner_value) = &self.#field_name_ident {
                        // Handle FHIR primitive types with proper type preservation
                        let mut field_result = inner_value.to_evaluation_result();
                        // Override type information for string-based FHIR primitive types
                        field_result = match field_result {
                            helios_fhirpath_support::EvaluationResult::String(s, _) => {
                                helios_fhirpath_support::EvaluationResult::fhir_string(s, #type_name)
                            },
                            helios_fhirpath_support::EvaluationResult::Boolean(b, _) => {
                                helios_fhirpath_support::EvaluationResult::fhir_boolean(b)
                            },
                            helios_fhirpath_support::EvaluationResult::Integer(i, _) => {
                                helios_fhirpath_support::EvaluationResult::fhir_integer(i)
                            },
                            helios_fhirpath_support::EvaluationResult::Decimal(d, _) => {
                                helios_fhirpath_support::EvaluationResult::fhir_decimal(d)
                            },
                            _ => field_result,
                        };
                        // Only insert if the inner evaluation is not Empty
                        if field_result != helios_fhirpath_support::EvaluationResult::Empty {
                            map.insert(#field_key_str.to_string(), field_result);
                        }
                    }
                    // If self.#field_name_ident is None, do nothing (don't insert Empty)
                }
            } else {
                quote! {
                    if let Some(inner_value) = &self.#field_name_ident {
                        let field_result = inner_value.to_evaluation_result();
                        // Only insert if the inner evaluation is not Empty
                        if field_result != helios_fhirpath_support::EvaluationResult::Empty {
                            map.insert(#field_key_str.to_string(), field_result);
                        }
                    }
                    // If self.#field_name_ident is None, do nothing (don't insert Empty)
                }
            }
        } else {
            // For non-Option<T>, evaluate directly
            if let Some(type_name) = fhir_type_name {
                // Special handling for FHIR primitive types to preserve type information
                quote! {
                    // Handle FHIR primitive types with proper type preservation
                    let mut field_result = self.#field_name_ident.to_evaluation_result();
                    // Override type information for FHIR primitive types
                    field_result = match field_result {
                        helios_fhirpath_support::EvaluationResult::String(s, _) => {
                            helios_fhirpath_support::EvaluationResult::fhir_string(s, #type_name)
                        },
                        helios_fhirpath_support::EvaluationResult::Boolean(b, _) => {
                            helios_fhirpath_support::EvaluationResult::fhir_boolean(b)
                        },
                        helios_fhirpath_support::EvaluationResult::Integer(i, _) => {
                            helios_fhirpath_support::EvaluationResult::fhir_integer(i)
                        },
                        helios_fhirpath_support::EvaluationResult::Decimal(d, _) => {
                            helios_fhirpath_support::EvaluationResult::fhir_decimal(d)
                        },
                        _ => field_result,
                    };
                    // Only insert if the evaluation is not Empty
                    if field_result != helios_fhirpath_support::EvaluationResult::Empty {
                        map.insert(#field_key_str.to_string(), field_result);
                    }
                }
            } else {
                quote! {
                    let field_result = self.#field_name_ident.to_evaluation_result();
                    // Only insert if the evaluation is not Empty
                    if field_result != helios_fhirpath_support::EvaluationResult::Empty {
                        map.insert(#field_key_str.to_string(), field_result);
                    }
                }
            }
        } // Return the generated code for this field
    });

    // Determine the type name to use for type info
    // For now, we'll use the struct name as the type name
    let type_name_str = name.to_string();

    let into_evaluation_result_impl = quote! {
        impl #impl_generics helios_fhirpath_support::IntoEvaluationResult for #name #ty_generics #where_clause {
            fn to_evaluation_result(&self) -> helios_fhirpath_support::EvaluationResult {
                // Use fully qualified path for HashMap
                let mut map = std::collections::HashMap::new();

                #(#field_conversions)* // Expand the field conversion logic

                // Return a typed object with FHIR type information
                helios_fhirpath_support::EvaluationResult::typed_object(
                    map,
                    "FHIR",
                    &#type_name_str
                )
            }
        }
    };

    // Check if this struct has the fhir_resource attribute with choice_elements
    if let Some(choice_elements) = extract_resource_choice_elements(attrs) {
        let choice_element_literals: Vec<_> = choice_elements
            .iter()
            .map(|elem| quote! { #elem })
            .collect();

        quote! {
            #into_evaluation_result_impl

            impl #impl_generics helios_fhirpath_support::FhirResourceMetadata for #name #ty_generics #where_clause {
                fn choice_elements() -> &'static [&'static str] {
                    &[#(#choice_element_literals),*]
                }
            }
        }
    } else {
        into_evaluation_result_impl
    }
}

fn generate_fhirpath_enum_impl(
    name: &Ident,
    data: &syn::DataEnum,
    attrs: &[syn::Attribute],
    impl_generics: &syn::ImplGenerics,
    ty_generics: &syn::TypeGenerics,
    where_clause: Option<&syn::WhereClause>,
) -> proc_macro2::TokenStream {
    // Handle empty enums (like initial R6 Resource enum)
    if data.variants.is_empty() {
        let is_resource_enum = name == "Resource";

        let additional_impl = if is_resource_enum {
            quote! {
                impl #impl_generics crate::FhirResourceTypeProvider for #name #ty_generics #where_clause {
                    fn get_resource_type_names() -> Vec<&'static str> {
                        vec![] // Empty enum has no resource types
                    }
                }
            }
        } else {
            quote! {}
        };

        return quote! {
            impl #impl_generics helios_fhirpath_support::IntoEvaluationResult for #name #ty_generics #where_clause {
                fn to_evaluation_result(&self) -> helios_fhirpath_support::EvaluationResult {
                    // This should never be called for an empty enum
                    unreachable!("Empty enum should not be instantiated")
                }
            }

            #additional_impl
        };
    }

    // Check if the enum being derived is the top-level Resource enum
    let is_resource_enum = name == "Resource";

    // If this is a Resource enum, collect all variant names for the FhirResourceTypeProvider trait
    let resource_type_names: Vec<String> = if is_resource_enum {
        data.variants
            .iter()
            .map(|variant| variant.ident.to_string())
            .collect()
    } else {
        Vec::new()
    };

    let match_arms = data.variants.iter().map(|variant| {
        let variant_name = &variant.ident;
        let variant_name_str = variant_name.to_string();

        match &variant.fields {
            Fields::Unit => {
                // For unit variants, return the variant name as a string (like a code)
                // This is likely for status codes etc., not the Resource enum
                quote! {
                    Self::#variant_name => helios_fhirpath_support::EvaluationResult::string(#variant_name_str.to_string()),
                }
            }
            Fields::Unnamed(fields) if fields.unnamed.len() == 1 => {
                // Newtype variant
                if is_resource_enum {
                    // Special handling for the Resource enum: add resourceType
                    quote! {
                        Self::#variant_name(value) => {
                            let mut result = value.to_evaluation_result(); // Call on inner Box<ResourceStruct>
                            if let helios_fhirpath_support::EvaluationResult::Object { ref mut map, .. } = result {
                                // Insert the resourceType field using the variant name
                                map.insert(
                                    "resourceType".to_string(),
                                    helios_fhirpath_support::EvaluationResult::string(#variant_name_str.to_string())
                                );
                            }
                            // Return the (potentially modified) result
                            result
                        }
                    }
                } else {
                    // For other enums (like choice types), preserve type information from the variant
                    // Extract type information from the variant name or rename attribute
                    let variant_name_str = variant_name.to_string();
                    // Check for fhir_serde rename attribute to get the FHIR field name
                    let mut fhir_field_name = variant_name_str.clone();
                    for attr in &variant.attrs {
                        if attr.path().is_ident("fhir_serde") {
                            if let Ok(list) = attr.parse_args_with(syn::punctuated::Punctuated::<syn::Meta, syn::token::Comma>::parse_terminated) {
                                for meta in list {
                                    if let syn::Meta::NameValue(nv) = meta {
                                        if nv.path.is_ident("rename") {
                                            if let syn::Expr::Lit(expr_lit) = nv.value {
                                                if let syn::Lit::Str(lit_str) = expr_lit.lit {
                                                    fhir_field_name = lit_str.value();
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Extract FHIR type from choice element field name (e.g., "valueCode" -> "code")
                    let fhir_type = if fhir_field_name.starts_with("value") && fhir_field_name.len() > 5 {
                        // Convert first character to lowercase for FHIR primitive types
                        let type_part = &fhir_field_name[5..]; // Remove "value" prefix
                        let mut chars = type_part.chars();
                        match chars.next() {
                            None => variant_name_str.clone(),
                            Some(first) => first.to_lowercase().collect::<String>() + chars.as_str(),
                        }
                    } else if fhir_field_name.ends_with("Boolean") {
                        // Special case for FHIR boolean primitives - use lowercase
                        "boolean".to_string()
                    } else if fhir_field_name.ends_with("Integer") {
                        // Special case for FHIR integer primitives - use lowercase  
                        "integer".to_string()
                    } else if fhir_field_name.ends_with("Decimal") {
                        // Special case for FHIR decimal primitives - use lowercase
                        "decimal".to_string()
                    } else if fhir_field_name.ends_with("String") {
                        // Special case for FHIR string primitives - use lowercase
                        "string".to_string()
                    } else if fhir_field_name.ends_with("Instant") {
                        // Special case for FHIR instant primitives - use lowercase
                        "instant".to_string()
                    } else if fhir_field_name.ends_with("DateTime") {
                        // Special case for FHIR dateTime primitives - use lowercase
                        "dateTime".to_string()
                    } else if fhir_field_name.ends_with("Date") {
                        // Special case for FHIR date primitives - use lowercase
                        "date".to_string()
                    } else if fhir_field_name.ends_with("Time") {
                        // Special case for FHIR time primitives - use lowercase
                        "time".to_string()
                    } else {
                        // Fallback to variant name if it doesn't match known patterns
                        // Convert first character to lowercase for consistency with FHIR primitive naming
                        let mut chars = variant_name_str.chars();
                        match chars.next() {
                            None => variant_name_str.clone(),
                            Some(first) => first.to_lowercase().collect::<String>() + chars.as_str(),
                        }
                    };
                    // For choice type enums that will be flattened, we need to return an object
                    // with the polymorphic field name as the key
                    // A choice type enum is one where variants have rename attributes with type suffixes
                    // e.g., "deceasedBoolean", "valueString", etc.
                    let is_choice_type_enum = fhir_field_name != variant_name_str &&
                        extract_type_suffix_from_field_name(&fhir_field_name).is_some();

                    if is_choice_type_enum {
                        quote! {
                            Self::#variant_name(value) => {
                                // Get the base evaluation result from the inner value
                                let mut result = value.to_evaluation_result();
                                // Add FHIR type information to preserve type for .ofType() operations
                                // For choice type enums, always use the type determined from the field name
                                result = match result {
                                    helios_fhirpath_support::EvaluationResult::String(s, _existing_type_info) => {
                                        // Always use the determined type from the field name for choice types
                                        let type_info = helios_fhirpath_support::TypeInfoResult::new("FHIR", &#fhir_type);
                                        helios_fhirpath_support::EvaluationResult::String(s, Some(type_info))
                                    },
                                    helios_fhirpath_support::EvaluationResult::Integer(i, existing_type_info) => {
                                        let type_info = existing_type_info.unwrap_or_else(|| helios_fhirpath_support::TypeInfoResult::new("FHIR", &#fhir_type));
                                        helios_fhirpath_support::EvaluationResult::Integer(i, Some(type_info))
                                    },
                                    helios_fhirpath_support::EvaluationResult::Decimal(d, existing_type_info) => {
                                        let type_info = existing_type_info.unwrap_or_else(|| helios_fhirpath_support::TypeInfoResult::new("FHIR", &#fhir_type));
                                        helios_fhirpath_support::EvaluationResult::Decimal(d, Some(type_info))
                                    },
                                    helios_fhirpath_support::EvaluationResult::Boolean(b, existing_type_info) => {
                                        let type_info = existing_type_info.unwrap_or_else(|| helios_fhirpath_support::TypeInfoResult::new("FHIR", &#fhir_type));
                                        helios_fhirpath_support::EvaluationResult::Boolean(b, Some(type_info))
                                    },
                                    helios_fhirpath_support::EvaluationResult::Object { map, type_info: existing_type_info } => {
                                        let type_info = existing_type_info.unwrap_or_else(|| helios_fhirpath_support::TypeInfoResult::new("FHIR", &#fhir_type));
                                        helios_fhirpath_support::EvaluationResult::Object {
                                            map,
                                            type_info: Some(type_info),
                                        }
                                    },
                                    _ => result, // For other types, return as-is
                                };

                                // Wrap the result in an object with the field name as the key
                                let mut map = std::collections::HashMap::new();
                                map.insert(#fhir_field_name.to_string(), result);
                                helios_fhirpath_support::EvaluationResult::Object {
                                    map,
                                    type_info: None, // No type info for the wrapper object
                                }
                            }
                        }
                    } else {
                        quote! {
                            Self::#variant_name(value) => {
                                // Get the base evaluation result from the inner value
                                let mut result = value.to_evaluation_result();
                                // Add FHIR type information to preserve type for .ofType() operations
                                // For choice type enums, always use the type determined from the field name
                                result = match result {
                                    helios_fhirpath_support::EvaluationResult::String(s, _existing_type_info) => {
                                        // Always use the determined type from the field name for choice types
                                        let type_info = helios_fhirpath_support::TypeInfoResult::new("FHIR", &#fhir_type);
                                        helios_fhirpath_support::EvaluationResult::String(s, Some(type_info))
                                    },
                                    helios_fhirpath_support::EvaluationResult::Integer(i, existing_type_info) => {
                                        let type_info = existing_type_info.unwrap_or_else(|| helios_fhirpath_support::TypeInfoResult::new("FHIR", &#fhir_type));
                                        helios_fhirpath_support::EvaluationResult::Integer(i, Some(type_info))
                                    },
                                    helios_fhirpath_support::EvaluationResult::Decimal(d, existing_type_info) => {
                                        let type_info = existing_type_info.unwrap_or_else(|| helios_fhirpath_support::TypeInfoResult::new("FHIR", &#fhir_type));
                                        helios_fhirpath_support::EvaluationResult::Decimal(d, Some(type_info))
                                    },
                                    helios_fhirpath_support::EvaluationResult::Boolean(b, existing_type_info) => {
                                        let type_info = existing_type_info.unwrap_or_else(|| helios_fhirpath_support::TypeInfoResult::new("FHIR", &#fhir_type));
                                        helios_fhirpath_support::EvaluationResult::Boolean(b, Some(type_info))
                                    },
                                    helios_fhirpath_support::EvaluationResult::Object { map, type_info: existing_type_info } => {
                                        let type_info = existing_type_info.unwrap_or_else(|| helios_fhirpath_support::TypeInfoResult::new("FHIR", &#fhir_type));
                                        helios_fhirpath_support::EvaluationResult::Object {
                                            map,
                                            type_info: Some(type_info),
                                        }
                                    },
                                    _ => result, // For other types, return as-is
                                };
                                result
                            }
                        }
                    }
                }
           }
            // For tuple or struct variants (uncommon in FHIR choice types or Resource enum),
            // the direct FHIRPath evaluation is less clear.
            // Returning Empty seems like a reasonable default.
            Fields::Unnamed(_) | Fields::Named(_) => {
                 quote! {
                     // Match all fields but ignore them for now
                     Self::#variant_name { .. } => helios_fhirpath_support::EvaluationResult::Empty,
                 }
            }
        }
    });

    // Handle the case where the enum has no variants
    let body = if data.variants.is_empty() {
        // An empty enum cannot be instantiated, so this method is technically unreachable.
        // Return Empty as a safe default.
        quote! { helios_fhirpath_support::EvaluationResult::Empty }
    } else {
        // Generate the match statement for enums with variants
        quote! {
            match self {
                #(#match_arms)*
            }
        }
    };

    let into_evaluation_result_impl = quote! {
        impl #impl_generics helios_fhirpath_support::IntoEvaluationResult for #name #ty_generics #where_clause {
            fn to_evaluation_result(&self) -> helios_fhirpath_support::EvaluationResult {
                 #body // Use the generated body (either Empty or the match statement)
            }
        }
    };

    // Generate additional FhirResourceTypeProvider implementation for Resource enums
    if is_resource_enum {
        let resource_type_literals: Vec<_> = resource_type_names
            .iter()
            .map(|name| {
                quote! { #name }
            })
            .collect();

        // Generate resource_name method for Resource enum
        let resource_name_arms = data.variants.iter().map(|variant| {
            let variant_name = &variant.ident;
            let variant_name_str = variant_name.to_string();

            match &variant.fields {
                Fields::Unnamed(fields) if fields.unnamed.len() == 1 => {
                    // Newtype variant (expected for Resource enum)
                    quote! {
                        Self::#variant_name(_) => #variant_name_str,
                    }
                }
                _ => {
                    // For other field types, still return the variant name
                    quote! {
                        Self::#variant_name { .. } => #variant_name_str,
                    }
                }
            }
        });

        // Generate get_last_updated method for Resource enum
        let get_last_updated_arms = data.variants.iter().map(|variant| {
            let variant_name = &variant.ident;

            match &variant.fields {
                Fields::Unnamed(fields) if fields.unnamed.len() == 1 => {
                    quote! {
                        Self::#variant_name(resource) => {
                            resource.meta.as_ref()
                                .and_then(|m| m.last_updated.as_ref())
                                .and_then(|lu| {
                                    // Handle Element<PrecisionDateTime> - get the value and convert to chrono
                                    lu.value.as_ref().map(|precision_dt| {
                                        // PrecisionDateTime has a to_chrono_datetime() method
                                        precision_dt.to_chrono_datetime()
                                    })
                                })
                        }
                    }
                }
                _ => {
                    quote! {
                        Self::#variant_name { .. } => None,
                    }
                }
            }
        });

        quote! {
            #into_evaluation_result_impl

            impl #impl_generics #name #ty_generics #where_clause {
                /// Returns the resource type name as a string.
                /// This is equivalent to the resourceType field in FHIR JSON.
                pub fn resource_name(&self) -> &'static str {
                    match self {
                        #(#resource_name_arms)*
                    }
                }

                /// Returns the lastUpdated timestamp from the resource's metadata if available.
                pub fn get_last_updated(&self) -> Option<::chrono::DateTime<::chrono::Utc>> {
                    match self {
                        #(#get_last_updated_arms)*
                    }
                }
            }

            impl #impl_generics crate::FhirResourceTypeProvider for #name #ty_generics #where_clause {
                fn get_resource_type_names() -> Vec<&'static str> {
                    vec![#(#resource_type_literals),*]
                }
            }
        }
    } else {
        // Check if this is a choice element enum
        if let Some(base_name) = extract_choice_element_base_name(attrs) {
            // Extract possible field names from the enum variants
            let field_names: Vec<String> = data.variants.iter().filter_map(|variant| {
                // Look for the fhir_serde(rename = "...") attribute
                for attr in &variant.attrs {
                    if attr.path().is_ident("fhir_serde") {
                        if let Ok(list) = attr.parse_args_with(syn::punctuated::Punctuated::<syn::Meta, syn::token::Comma>::parse_terminated) {
                            for meta in list {
                                if let syn::Meta::NameValue(nv) = meta {
                                    if nv.path.is_ident("rename") {
                                        if let syn::Expr::Lit(expr_lit) = nv.value {
                                            if let syn::Lit::Str(lit_str) = expr_lit.lit {
                                                return Some(lit_str.value());
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                None
            }).collect();

            let field_name_literals: Vec<_> =
                field_names.iter().map(|name| quote! { #name }).collect();

            quote! {
                #into_evaluation_result_impl

                impl #impl_generics helios_fhirpath_support::ChoiceElement for #name #ty_generics #where_clause {
                    fn base_name() -> &'static str {
                        #base_name
                    }

                    fn possible_field_names() -> Vec<&'static str> {
                        vec![#(#field_name_literals),*]
                    }
                }
            }
        } else {
            into_evaluation_result_impl
        }
    }
}

/// Derive macro for TypeInfo trait.
///
/// This macro generates implementations of the TypeInfo trait for FHIR types,
/// providing type namespace and name information needed by the FHIRPath type() function.
///
/// # Attributes
///
/// - `#[type_info(namespace = "FHIR", name = "boolean")]` - Specifies custom namespace and name
/// - If not specified, defaults are inferred from the type name
///
/// # Examples
///
/// ```rust,ignore
/// #[derive(TypeInfo)]
/// #[type_info(namespace = "FHIR", name = "boolean")]
/// pub struct Boolean(pub Element<bool, Extension>);
///
/// #[derive(TypeInfo)]
/// pub struct Patient {
///     // fields...
/// }
/// ```
#[proc_macro_derive(TypeInfo, attributes(type_info))]
pub fn type_info_derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let generics = &input.generics;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    // Extract type_info attributes if present
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

    TokenStream::from(expanded)
}

/// Extracts namespace and name from type_info attributes.
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

    // Default: Assume FHIR namespace and use the type name
    ("\"FHIR\"".to_string(), format!("\"{}\"", type_name))
}

/// Extracts the FHIR type suffix from a choice element field name using pattern matching.
/// For example, "valueQuantity" -> Some(("value", "Quantity")), "valueString" -> Some(("value", "String"))
fn extract_type_suffix_from_field_name(field_name: &str) -> Option<(&str, &str)> {
    let chars: Vec<char> = field_name.chars().collect();

    // Look for the pattern: lowercase...Uppercase...
    // This indicates the transition from base name to type name
    let mut transition_index = None;

    for i in 1..chars.len() {
        if chars[i - 1].is_lowercase() && chars[i].is_uppercase() {
            transition_index = Some(i);
            break;
        }
    }

    if let Some(idx) = transition_index {
        let base_name = &field_name[..idx];
        let type_suffix = &field_name[idx..];

        // Validate that this looks like a valid FHIR type suffix:
        // - Starts with uppercase letter
        // - Has at least 2 characters (to avoid false positives like "valueA")
        // - Contains only alphanumeric characters (and potentially numbers at the end like Integer64)
        if type_suffix.len() >= 2
            && type_suffix.chars().next().is_some_and(|c| c.is_uppercase())
            && type_suffix.chars().all(|c| c.is_alphanumeric())
            && !base_name.is_empty()
        {
            return Some((base_name, type_suffix));
        }
    }

    None
}

/// Extracts the base name from fhir_choice_element attribute if present.
fn extract_choice_element_base_name(attrs: &[syn::Attribute]) -> Option<String> {
    for attr in attrs {
        if attr.path().is_ident("fhir_choice_element") {
            if let Ok(list) =
                attr.parse_args_with(Punctuated::<Meta, token::Comma>::parse_terminated)
            {
                for meta in list {
                    if let Meta::NameValue(nv) = meta {
                        if nv.path.is_ident("base_name") {
                            if let syn::Expr::Lit(expr_lit) = nv.value {
                                if let Lit::Str(lit_str) = expr_lit.lit {
                                    return Some(lit_str.value());
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

/// Extracts choice elements from fhir_resource attribute if present.
fn extract_resource_choice_elements(attrs: &[syn::Attribute]) -> Option<Vec<String>> {
    for attr in attrs {
        if attr.path().is_ident("fhir_resource") {
            if let Ok(list) =
                attr.parse_args_with(Punctuated::<Meta, token::Comma>::parse_terminated)
            {
                for meta in list {
                    if let Meta::NameValue(nv) = meta {
                        if nv.path.is_ident("choice_elements") {
                            if let syn::Expr::Lit(expr_lit) = nv.value {
                                if let Lit::Str(lit_str) = expr_lit.lit {
                                    // Split the comma-separated list of choice elements
                                    let elements: Vec<String> = lit_str
                                        .value()
                                        .split(',')
                                        .map(|s| s.trim().to_string())
                                        .filter(|s| !s.is_empty())
                                        .collect();
                                    return Some(elements);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

/// Extracts the FHIR type name from a type path for primitive FHIR types.
/// Returns None if the type is not a recognized FHIR primitive type.
fn extract_fhir_primitive_type_name(ty: &syn::Type) -> Option<&'static str> {
    // Get the inner type if this is an Option<T>
    let inner_type = if let Some(inner) = get_option_inner_type(ty) {
        inner
    } else {
        ty
    };

    // Check if this is a path type
    if let syn::Type::Path(type_path) = inner_type {
        if let Some(segment) = type_path.path.segments.last() {
            let type_name = segment.ident.to_string();

            // Map FHIR type aliases to their lowercase primitive names
            match type_name.as_str() {
                "Uri" => Some("uri"),
                "Code" => Some("code"),
                "Id" => Some("id"),
                "Oid" => Some("oid"),
                "Uuid" => Some("uuid"),
                "Canonical" => Some("canonical"),
                "Url" => Some("url"),
                "Markdown" => Some("markdown"),
                "Base64Binary" => Some("base64Binary"),
                "Instant" => Some("instant"),
                "Date" => Some("date"),
                "DateTime" => Some("dateTime"),
                "Time" => Some("time"),
                "String" => Some("string"),
                "Boolean" => Some("boolean"),
                "Integer" => Some("integer"),
                "Integer64" => Some("integer64"),
                "PositiveInt" => Some("positiveInt"),
                "UnsignedInt" => Some("unsignedInt"),
                "Decimal" => Some("decimal"),
                _ => None,
            }
        } else {
            None
        }
    } else {
        None
    }
}
