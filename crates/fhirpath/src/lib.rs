//! # FHIRPath Expression Engine
//!
//! This crate provides a complete implementation of the [FHIRPath 3.0.0 specification](https://hl7.org/fhirpath/2025Jan/)
//! for evaluating FHIRPath expressions against FHIR resources. FHIRPath is a path-based navigation
//! and extraction language designed specifically for FHIR resources, enabling powerful queries
//! and data manipulation operations.

//!
//! ## Overview
//!
//! FHIRPath is a declarative language that allows you to:
//! - **Navigate FHIR resources** using path expressions (e.g., `Patient.name.family`)
//! - **Filter collections** with boolean predicates (e.g., `telecom.where(system = 'email')`)
//! - **Transform data** using built-in functions (e.g., `name.given.first()`)
//! - **Perform calculations** with mathematical operations (e.g., `birthDate.today() - birthDate`)
//! - **Access extensions** in FHIR resources (e.g., `Patient.extension('http://example.org/birthPlace')`)
//! - **Work with types** using type checking and conversion (e.g., `value.is(Quantity)`)
//!
//! ## Key Features
//!
//! ### Core Functionality
//! - **Parser**: Complete FHIRPath syntax support including literals, operators, and function calls
//! - **Evaluator**: Fast evaluation engine with proper type handling and error reporting
//! - **Type System**: Support for both FHIR and System namespaces with automatic type inference
//! - **Extension Support**: Native handling of FHIR extensions and choice elements
//!
//! ### Language Support
//! - **Collections**: Comprehensive collection operations (where, select, all, exists, etc.)
//! - **Mathematics**: Arithmetic operations with proper decimal precision handling
//! - **String Operations**: Text manipulation and pattern matching functions
//! - **Date/Time**: Temporal operations with timezone and precision support
//! - **Type Operations**: Dynamic type checking with `is`, `as`, and `ofType` operators
//! - **Variables**: Support for external variables and built-in constants
//!
//! ### FHIR Integration
//! - **Multi-version Support**: Works with FHIR R4, R4B, R5, and R6 via feature flags
//! - **Resource Navigation**: Smart navigation of FHIR choice elements (e.g., `value[x]`)
//! - **Extension Access**: Built-in `extension()` function for FHIR extension handling
//! - **Type Hierarchy**: Understanding of FHIR resource and data type relationships
//!
//! ## Architecture
//!
//! The crate is organized into several key components:
//!
//! - **Public API** (`lib.rs`): Simple interface with [`evaluate_expression`] function
//! - **Parser** (`parser.rs`): Converts FHIRPath text into an Abstract Syntax Tree (AST)
//! - **Evaluator** (`evaluator.rs`): Executes the AST against FHIR resources  
//! - **Function Modules**: Specialized implementations for FHIRPath functions
//! - **Type System**: FHIR type hierarchy and namespace management
//! - **Support Types**: Integration with the `fhirpath_support` crate for results
//!
//! ## Usage Examples
//!
//! ### Basic Navigation
//!
//! ```rust,no_run
//! use helios_fhirpath::{evaluate_expression, EvaluationContext};
//! # use helios_fhir::r4::{Patient, HumanName};
//!
//! # // Create a patient resource
//! # let patient = Patient::default();
//! # let context = EvaluationContext::new(vec![
//! #     helios_fhir::FhirResource::R4(Box::new(helios_fhir::r4::Resource::Patient(Box::new(patient))))
//! # ]);
//!
//! // Navigate to family name
//! let result = evaluate_expression("Patient.name.family", &context)?;
//! // Result: Collection containing family names
//!
//! // Get first given name  
//! let result = evaluate_expression("Patient.name.given.first()", &context)?;
//! // Result: First given name as string
//!
//! // Check if patient is active
//! let result = evaluate_expression("Patient.active", &context)?;
//! // Result: Boolean value
//! # Ok::<(), String>(())
//! ```
//!
//! ### Collection Operations
//!
//! ```rust,no_run
//! # use helios_fhirpath::{evaluate_expression, EvaluationContext};
//! # use helios_fhir::r4::Patient;
//! # let patient = Patient::default();
//! # let context = EvaluationContext::new(vec![helios_fhir::FhirResource::R4(Box::new(helios_fhir::r4::Resource::Patient(Box::new(patient))))]);
//!
//! // Filter email addresses
//! let result = evaluate_expression(
//!     "Patient.telecom.where(system = 'email')",
//!     &context
//! )?;
//!
//! // Check if any email exists
//! let result = evaluate_expression(
//!     "Patient.telecom.where(system = 'email').exists()",
//!     &context
//! )?;
//!
//! // Count phone numbers
//! let result = evaluate_expression(
//!     "Patient.telecom.where(system = 'phone').count()",
//!     &context
//! )?;
//! # Ok::<(), String>(())
//! ```
//!
//! ### Type Operations
//!
//! ```rust,no_run
//! # use helios_fhirpath::{evaluate_expression, EvaluationContext};
//! # use helios_fhir::r4::Observation;
//! # let observation = Observation::default();
//! # let context = EvaluationContext::new(vec![helios_fhir::FhirResource::R4(Box::new(helios_fhir::r4::Resource::Observation(Box::new(observation))))]);
//!
//! // Check if observation value is a Quantity
//! let result = evaluate_expression(
//!     "Observation.value.is(Quantity)",
//!     &context
//! )?;
//!
//! // Cast value to Quantity and get unit
//! let result = evaluate_expression(
//!     "Observation.value.as(Quantity).unit",
//!     &context
//! )?;
//!
//! // Get type information
//! let result = evaluate_expression(
//!     "Observation.value.type().name",
//!     &context
//! )?;
//! # Ok::<(), String>(())
//! ```
//!
//! ### Extension Access
//!
//! ```rust,no_run
//! # use helios_fhirpath::{evaluate_expression, EvaluationContext, EvaluationResult};
//! # use helios_fhir::r4::Patient;
//!
//! // Create context with patient data
//! let mut context = EvaluationContext::new(vec![]);
//!
//! // Access FHIR extension by URL
//! let result = evaluate_expression(
//!     "Patient.extension('http://hl7.org/fhir/StructureDefinition/patient-birthPlace')",
//!     &context
//! )?;
//!
//! // Extension with variable
//! context.set_variable_result("birthPlaceUrl", EvaluationResult::string(
//!     "http://hl7.org/fhir/StructureDefinition/patient-birthPlace".to_string()
//! ));
//! let result = evaluate_expression(
//!     "Patient.extension(%birthPlaceUrl).value",
//!     &context
//! )?;
//! # Ok::<(), String>(())
//! ```
//!
//! ### Mathematical Operations
//!
//! ```rust,no_run
//! # use helios_fhirpath::{evaluate_expression, EvaluationContext};
//! # let context = EvaluationContext::new(vec![]);
//!
//! // Basic arithmetic
//! let result = evaluate_expression("1 + 2 * 3", &context)?; // Result: 7
//!
//! // Decimal operations
//! let result = evaluate_expression("10.5 / 2.1", &context)?;
//!
//! // Age calculation (if Patient.birthDate exists)
//! let result = evaluate_expression(
//!     "today() - Patient.birthDate",
//!     &context
//! )?;
//! # Ok::<(), String>(())
//! ```
//!
//! ### Variables and Constants
//!
//! ```rust,no_run
//! # use helios_fhirpath::{evaluate_expression, EvaluationContext, EvaluationResult};
//! let mut context = EvaluationContext::new(vec![]);
//!
//! // Set custom variables
//! context.set_variable_result("threshold", EvaluationResult::decimal(rust_decimal::Decimal::new(5, 0)));
//! context.set_variable_result("unitSystem", EvaluationResult::string("metric".to_string()));
//!
//! // Use variables in expressions
//! let result = evaluate_expression("value > %threshold", &context)?;
//!
//! // Built-in constants are automatically available
//! let result = evaluate_expression("system = %ucum", &context)?; // %ucum = 'http://unitsofmeasure.org'
//! # Ok::<(), String>(())
//! ```
//!
//! ## Error Handling
//!
//! The [`evaluate_expression`] function returns detailed error messages for both parsing and evaluation failures:
//!
//! ```rust,no_run
//! # use helios_fhirpath::{evaluate_expression, EvaluationContext};
//! # let context = EvaluationContext::new(vec![]);
//!
//! // Syntax error
//! match evaluate_expression("Patient.name.", &context) {
//!     Err(err) => println!("Parse error: {}", err),
//!     Ok(_) => {}
//! }
//!
//! // Runtime error
//! match evaluate_expression("Patient.nonExistentField", &context) {
//!     Err(err) => println!("Evaluation error: {}", err),
//!     Ok(_) => {}
//! }
//! ```
//!
//! ## Performance Considerations
//!
//! - **Parsing**: Expression parsing is relatively expensive; consider caching parsed expressions for repeated use
//! - **Evaluation**: Evaluation performance depends on resource size and expression complexity
//! - **Memory**: Large collections in FHIR resources may consume significant memory during evaluation
//!
//! ## Specification Compliance
//!
//! This implementation aims for full compliance with [FHIRPath 3.0.0](https://hl7.org/fhirpath/2025Jan/).
//! Current implementation status includes:
//!
//! - ✅ **Core Language**: Literals, operators, path navigation
//! - ✅ **Collection Functions**: where, select, first, last, tail, etc.
//! - ✅ **Boolean Logic**: and, or, not, implies, xor
//! - ✅ **Type Operations**: is, as, ofType with FHIR type system
//! - ✅ **String Functions**: matches, contains, startsWith, etc.
//! - ✅ **Math Functions**: abs, ceiling, floor, round, sqrt, etc.
//! - ✅ **Date Functions**: today, now, date/time arithmetic
//! - ✅ **Extension Functions**: FHIR extension access
//! - ✅ **Variables**: External variables and built-in constants
//! - 🟡 **Advanced Features**: Some STU (Standard for Trial Use) functions
//!
//! See the [FHIRPath README](https://github.com/HeliosSoftware/hfs/blob/main/crates/fhirpath/README.md)
//! for detailed implementation status.
//!
//! ## FHIR Version Support
//!
//! This crate supports multiple FHIR versions through Cargo feature flags:
//!
//! ```toml
//! [dependencies]
//! fhirpath = { version = "0.1", features = ["R4"] }      # FHIR R4 support
//! fhirpath = { version = "0.1", features = ["R5"] }      # FHIR R5 support  
//! fhirpath = { version = "0.1", features = ["R4", "R5"] } # Multiple versions
//! ```
//!
//! Available features:
//! - `R4`: FHIR 4.0.1 (normative)
//! - `R4B`: FHIR 4.3.0 (ballot)
//! - `R5`: FHIR 5.0.0 (ballot)
//! - `R6`: FHIR 6.0.0 (draft)

// Internal modules - not part of the public API
mod aggregate_function;
mod aggregate_math_functions;
mod boolean_functions;
mod boundary_functions;
mod collection_functions;
mod collection_navigation;
mod contains_function;
mod conversion_functions;
mod format_functions;
mod interval_functions;
mod json_utils;
mod ucum;
// Public for internal testing only - not part of the public API
#[doc(hidden)]
pub mod date_operation;
mod datetime_impl;
pub mod debug_trace;
mod distinct_functions;
mod extension_function;
mod fhir_type_hierarchy;
mod long_conversion;
mod not_function;
mod polymorphic_access;
mod reference_key_functions;
mod repeat_all_function;
mod repeat_function;
mod resource_type;
mod set_operations;
mod subset_functions;
pub mod terminology_client;
mod terminology_functions;
mod trace_function;
mod type_function;
pub mod type_inference;

// Modules for CLI and server functionality
pub mod cli;
pub mod error;
pub mod handlers;
pub mod models;
pub mod parse_debug;
pub mod server;

// Public modules needed for the public API
pub mod evaluator;
pub mod parser;

// Public API exports - this is what users of the fhirpath crate should use
use crate::parser::TypeSpecifier;
pub use evaluator::EvaluationContext;
pub use helios_fhirpath_support::EvaluationResult;

/// Evaluates a FHIRPath expression against a given context.
///
/// This is the primary interface for FHIRPath evaluation. It combines parsing and evaluation
/// into a single convenient function call.
///
/// # Arguments
///
/// * `expression` - The FHIRPath expression string to evaluate
/// * `context` - The evaluation context containing the FHIR resource(s) to evaluate against
///
/// # Returns
///
/// Returns a `Result` containing either:
/// - `Ok(EvaluationResult)` - The result of evaluating the expression
/// - `Err(String)` - An error message if parsing or evaluation fails
///
/// # Examples
///
/// ```rust,no_run
/// use helios_fhirpath::{evaluate_expression, EvaluationContext};
/// use helios_fhir::r4::Observation;
///
/// // Create a context with a FHIR resource
/// # let observation = Observation::default();
/// let context = EvaluationContext::new(vec![helios_fhir::FhirResource::R4(Box::new(helios_fhir::r4::Resource::Observation(Box::new(observation))))]);
///
/// // Evaluate a simple expression
/// let result = evaluate_expression("value.unit", &context)?;
/// # Ok::<(), String>(())
/// ```
///
/// # Notes
///
/// - The expression is parsed using the FHIRPath parser, which follows the FHIRPath 3.0.0 specification
/// - Evaluation is performed against the resources in the provided context
/// - Variables should be set on the context before calling this function
/// - The function handles all parsing errors and evaluation errors uniformly
pub fn evaluate_expression(
    expression: &str,
    context: &EvaluationContext,
) -> Result<EvaluationResult, String> {
    use chumsky::Parser;

    // Parse the expression
    let parsed = parser::parser()
        .parse(expression)
        .into_result()
        .map_err(|e| {
            format!(
                "Failed to parse FHIRPath expression '{}': {:?}",
                expression, e
            )
        })?;

    // Evaluate the parsed expression
    evaluator::evaluate(&parsed, context, None).map_err(|e| {
        format!(
            "Failed to evaluate FHIRPath expression '{}': {}",
            expression, e
        )
    })
}

// /// If `v` represents a FHIR primitive (either as a typed primitive result or as a typed Element object),
// /// return a view of:
// /// - its underlying primitive `value` (as EvaluationResult) if present
// /// - whether it is a FHIR primitive and its FHIR primitive name if known
// #[derive(Debug)]
// struct FhirPrimitiveView<'a> {
//     /// Underlying primitive value (System.* or already-evaluated primitive EvaluationResult)
//     value: Option<&'a EvaluationResult>,
//     /// True if this is *definitely* a FHIR primitive representation
//     is_fhir_primitive: bool,
//     /// Best-effort FHIR primitive name: "boolean", "string", "date", etc.
//     fhir_name: Option<&'a str>,
// }
//
// /// Try to interpret an EvaluationResult as a FHIR primitive (Option B compatible).
// pub fn fhir_primitive_view<'a>(v: &'a EvaluationResult) -> FhirPrimitiveView<'a> {
//     // Case A: value is already a typed FHIR primitive result (FHIR.boolean, FHIR.string, etc.)
//     match v {
//         EvaluationResult::Boolean(_, Some(ti)) if ti.namespace.eq_ignore_ascii_case("FHIR") => {
//             return FhirPrimitiveView {
//                 value: Some(v),
//                 is_fhir_primitive: true,
//                 fhir_name: Some(ti.name.as_str()), // e.g. "boolean"
//             };
//         }
//         EvaluationResult::Integer(_, Some(ti)) if ti.namespace.eq_ignore_ascii_case("FHIR") => {
//             return FhirPrimitiveView {
//                 value: Some(v),
//                 is_fhir_primitive: true,
//                 fhir_name: Some(ti.name.as_str()),
//             };
//         }
//         EvaluationResult::Decimal(_, Some(ti)) if ti.namespace.eq_ignore_ascii_case("FHIR") => {
//             return FhirPrimitiveView {
//                 value: Some(v),
//                 is_fhir_primitive: true,
//                 fhir_name: Some(ti.name.as_str()),
//             };
//         }
//         EvaluationResult::String(_, Some(ti)) if ti.namespace.eq_ignore_ascii_case("FHIR") => {
//             return FhirPrimitiveView {
//                 value: Some(v),
//                 is_fhir_primitive: true,
//                 fhir_name: Some(ti.name.as_str()), // could be "string", "uri", etc
//             };
//         }
//         EvaluationResult::DateTime(_, Some(ti)) if ti.namespace.eq_ignore_ascii_case("FHIR") => {
//             return FhirPrimitiveView {
//                 value: Some(v),
//                 is_fhir_primitive: true,
//                 fhir_name: Some(ti.name.as_str()), // "dateTime" / "instant"
//             };
//         }
//         EvaluationResult::Date(_, Some(ti)) if ti.namespace.eq_ignore_ascii_case("FHIR") => {
//             return FhirPrimitiveView {
//                 value: Some(v),
//                 is_fhir_primitive: true,
//                 fhir_name: Some(ti.name.as_str()), // "date"
//             };
//         }
//         EvaluationResult::Time(_, Some(ti)) if ti.namespace.eq_ignore_ascii_case("FHIR") => {
//             return FhirPrimitiveView {
//                 value: Some(v),
//                 is_fhir_primitive: true,
//                 fhir_name: Some(ti.name.as_str()), // "time"
//             };
//         }
//         _ => {}
//     }
//
//     // Case B: typed Element object that contains "value" (Option B wrapper)
//     if let EvaluationResult::Object { map, type_info } = v {
//         if let Some(ti) = type_info {
//             if ti.namespace.eq_ignore_ascii_case("FHIR") && ti.name.eq_ignore_ascii_case("Element")
//             {
//                 // "value" can be missing if it's id/extension-only; still a FHIR primitive wrapper
//                 let val = map.get("value");
//                 // optional: support map.get("fhirType") if you store it
//                 let fhir_name = map
//                     .get("fhirType")
//                     .and_then(|x| match x {
//                         EvaluationResult::String(s, _) => Some(s.as_str()),
//                         _ => None,
//                     });
//
//                 return FhirPrimitiveView {
//                     value: val,
//                     is_fhir_primitive: true,
//                     fhir_name,
//                 };
//             }
//         }
//     }
//
//     // Default: not recognized as a FHIR primitive wrapper
//     FhirPrimitiveView {
//         value: None,
//         is_fhir_primitive: false,
//         fhir_name: None,
//     }
// }
//
// pub fn primitive_system_value<'a>(v: &'a EvaluationResult) -> &'a EvaluationResult {
//     let view = fhir_primitive_view(v);
//     // If it's the object-wrapper (FHIR.Element), return the inner value when present.
//     if view.is_fhir_primitive {
//         if let Some(inner) = view.value {
//             return inner;
//         }
//     }
//     v
// }

pub fn type_spec_is_system(type_spec: &TypeSpecifier) -> bool {
    matches!(type_spec,
        TypeSpecifier::QualifiedIdentifier(ns, Some(_)) if ns.eq_ignore_ascii_case("System")
    ) || matches!(type_spec,
        TypeSpecifier::QualifiedIdentifier(t, None)
            if matches!(t.as_str(), "Boolean"|"String"|"Integer"|"Decimal"|"Date"|"DateTime"|"Time"|"Quantity")
    )
}
