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
pub mod ucum;
// Public for internal testing only - not part of the public API
#[doc(hidden)]
pub mod date_operation;
mod datetime_impl;
pub mod debug_trace;
mod distinct_functions;
mod extension_function;
// Curated catalog of built-in functions; re-exported below.
mod fhir_type_hierarchy;
mod functions;
mod long_conversion;
mod not_function;
mod polymorphic_access;
mod reference_key_functions;
mod repeat_all_function;
mod repeat_function;
mod resolve_function;
mod resource_type;
mod set_operations;
mod subset_functions;
mod terminology_client;
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
pub use evaluator::EvaluationContext;
pub use functions::{FunctionCategory, FunctionInfo, builtin_functions};
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
    let parsed = parse_expression(expression)?;

    // Evaluate the parsed expression
    evaluator::evaluate(&parsed, context, None).map_err(|e| {
        format!(
            "Failed to evaluate FHIRPath expression '{}': {}",
            expression, e
        )
    })
}

/// Parse a FHIRPath expression source string into a typed [`parser::Expression`] AST.
///
/// Provides a chumsky-free entry point for consumers that need the AST
/// (e.g. compiling FHIRPath to SQL) without taking a dependency on the
/// parser-combinator crate.
pub fn parse_expression(expression: &str) -> Result<parser::Expression, String> {
    use chumsky::Parser;

    parser::parser()
        .parse(expression)
        .into_result()
        .map_err(|e| {
            format!(
                "Failed to parse FHIRPath expression '{}': {:?}",
                expression, e
            )
        })
}

/// A single parse error from [`parse_expression_diagnostics`], with its span
/// expressed in **Unicode scalar value (`char`) offsets** into the original
/// `expression` string — never UTF-8 byte offsets.
///
/// chumsky's own `Rich` errors (which the FHIRPath parser produces) report
/// spans as byte offsets, since that is what its `&str` `Input` impl tracks
/// internally. Callers that index into the expression by character (e.g. a
/// browser editor counting Unicode code points, or anything that turns the
/// span into a substring via `.chars()`) would silently miscount on any
/// expression containing a multi-byte character, so the conversion happens
/// once here rather than being every caller's problem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseDiagnostic {
    /// `(start, end)` char offsets of the erroring span within `expression`.
    /// `start == end` for a zero-width error (e.g. unexpected end of input).
    pub span: (usize, usize),
    /// A human-readable description of the problem, taken from chumsky's own
    /// [`Display`](std::fmt::Display) rendering of the error — not a `Debug`
    /// dump of its internal structure, and never the expression text itself.
    pub message: String,
}

/// Parses `expression` and, on failure, returns every diagnostic chumsky's
/// [`Rich`](chumsky::error::Rich) error reporting produced, spans converted
/// to Unicode char offsets (see [`ParseDiagnostic`]).
///
/// Purely additive: [`parse_expression`] keeps its existing signature and
/// behavior unchanged. This is a second, richer entry point for callers that
/// need error *positions* rather than one flattened message — e.g. a lint
/// pass that must underline the offending span inside a larger document.
///
/// Never evaluates the expression and never touches a FHIR resource or a
/// terminology server: like [`parse_expression`], this is parsing alone.
///
/// # Examples
///
/// ```
/// use helios_fhirpath::parse_expression_diagnostics;
///
/// assert!(parse_expression_diagnostics("Patient.name.family").is_ok());
///
/// let errors = parse_expression_diagnostics("Patient.name.").unwrap_err();
/// assert!(!errors.is_empty());
/// assert!(!errors[0].message.is_empty());
/// ```
pub fn parse_expression_diagnostics(
    expression: &str,
) -> Result<parser::Expression, Vec<ParseDiagnostic>> {
    use chumsky::Parser;

    parser::parser()
        .parse(expression)
        .into_result()
        .map_err(|errors| {
            errors
                .iter()
                .map(|error| {
                    let span = error.span();
                    ParseDiagnostic {
                        span: (
                            byte_to_char_offset(expression, span.start),
                            byte_to_char_offset(expression, span.end),
                        ),
                        message: error.to_string(),
                    }
                })
                .collect()
        })
}

/// Converts a UTF-8 byte offset into `s` (assumed to already fall on a
/// `char` boundary, which every span chumsky's `&str` parser produces does)
/// into a count of Unicode scalar values before that offset.
fn byte_to_char_offset(s: &str, byte_offset: usize) -> usize {
    s.get(..byte_offset)
        .map_or_else(|| s.chars().count(), |prefix| prefix.chars().count())
}

/// Parses `expression` into a [`parser::SpannedExpression`] — the same AST
/// as [`parse_expression`], but with every node annotated with its
/// [`parser::ExprSpan`] (a byte `position`/`length` pair into `expression`).
///
/// Uses [`parser::spanned_parser`] under the hood; on failure, returns the
/// same diagnostics [`parse_expression_diagnostics`] would (span converted
/// to Unicode char offsets — see [`ParseDiagnostic`]).
///
/// This is a third, purely additive entry point: [`parse_expression`] and
/// [`parse_expression_diagnostics`] keep their existing signatures and
/// behavior unchanged, and this function does not affect the debug tracer
/// (`FHIRPATH_DEBUG_TRACE=1`), which already calls [`parser::spanned_parser`]
/// directly. It exists for callers that need to locate *where* a specific
/// construct (e.g. an external constant reference) sits in the source text —
/// [`external_constants`] is one such caller.
///
/// Never evaluates the expression and never touches a FHIR resource or a
/// terminology server.
///
/// # Examples
///
/// ```
/// use helios_fhirpath::parse_expression_spanned;
///
/// let spanned = parse_expression_spanned("Patient.name.family").unwrap();
/// assert_eq!(spanned.span.position, 0);
///
/// let errors = parse_expression_spanned("Patient.name.").unwrap_err();
/// assert!(!errors.is_empty());
/// ```
pub fn parse_expression_spanned(
    expression: &str,
) -> Result<parser::SpannedExpression, Vec<ParseDiagnostic>> {
    use chumsky::Parser;

    parser::spanned_parser()
        .parse(expression)
        .into_result()
        .map_err(|errors| {
            errors
                .iter()
                .map(|error| {
                    let span = error.span();
                    ParseDiagnostic {
                        span: (
                            byte_to_char_offset(expression, span.start),
                            byte_to_char_offset(expression, span.end),
                        ),
                        message: error.to_string(),
                    }
                })
                .collect()
        })
}

/// Converts a byte-offset [`parser::ExprSpan`] (as produced by
/// [`parser::spanned_parser`] / [`parse_expression_spanned`]) into a
/// `(start, end)` pair of Unicode char offsets into `expression`, with the
/// same semantics as [`ParseDiagnostic::span`].
///
/// `ExprSpan` stores a byte `position`/`length` because that is what
/// chumsky's `&str` input tracks internally (see [`parser::ExprSpan`] and
/// [`debug_trace`], its only other consumer today). A caller indexing into
/// `expression` by character — a browser editor counting Unicode code
/// points, or a diagnostic span meant to be sliced with `.chars()` — would
/// silently miscount on any expression containing a multi-byte character
/// before the span, so this conversion exists once here rather than being
/// every such caller's problem.
///
/// # Examples
///
/// ```
/// use helios_fhirpath::{expr_span_to_char_offsets, parse_expression_spanned};
///
/// // "café" is 4 chars / 5 bytes (é is a 2-byte UTF-8 sequence).
/// let spanned = parse_expression_spanned("'café' & %foo").unwrap();
/// // The whole expression's span covers the full source in bytes...
/// assert_eq!(spanned.span.position + spanned.span.length, 14);
/// // ...but converted to chars, it covers the 13-char source instead.
/// let (_, end) = expr_span_to_char_offsets("'café' & %foo", &spanned.span);
/// assert_eq!(end, 13);
/// ```
pub fn expr_span_to_char_offsets(expression: &str, span: &parser::ExprSpan) -> (usize, usize) {
    (
        byte_to_char_offset(expression, span.position),
        byte_to_char_offset(expression, span.position + span.length),
    )
}

/// A reference to an external constant (`%name`) found by [`external_constants`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalConstantRef {
    /// The constant's name, without the leading `%` and with any
    /// `` `backtick` `` or `'single-quote'` delimiters stripped (escape
    /// sequences inside a delimited name are already decoded, matching what
    /// [`Term::ExternalConstant`](parser::Term::ExternalConstant) stores).
    pub name: String,
    /// The **byte** span of the full token, from the `%` through the end of
    /// the name — including its delimiters in the two quoted lexical forms.
    /// Convert to char offsets with [`expr_span_to_char_offsets`].
    pub span: parser::ExprSpan,
}

/// Walks every node of `expr` and returns a reference for each external
/// constant (`%name`, `` %`quoted name` ``, or `%'quoted name'``) found
/// anywhere in the tree — as an operand, a function argument, inside a
/// lambda, an indexer, a union, or the operand of `is`/`as`.
///
/// `source` must be the exact expression string `expr` was parsed from
/// ([`parse_expression_spanned`] or [`parser::spanned_parser`] directly);
/// it is needed to correct a quirk of [`parser::spanned_parser`]'s spans.
/// Every lexical token in that grammar is built from combinators ending in
/// `.padded()`, so the recorded `ExprSpan` for an external constant also
/// swallows any whitespace/comments immediately following the token —
/// harmless for [`debug_trace`], the only existing consumer of those spans,
/// but wrong for a diagnostic span meant to underline just the `%name`
/// token. This walker recovers the exact end offset from `source` itself
/// rather than trusting the parser's (over-wide) span, so every
/// [`ExternalConstantRef::span`] this returns covers precisely the token.
///
/// # Examples
///
/// ```
/// use helios_fhirpath::{external_constants, parse_expression_spanned};
///
/// let source = "name.where(system = %ucum)";
/// let spanned = parse_expression_spanned(source).unwrap();
/// let refs = external_constants(&spanned, source);
/// assert_eq!(refs.len(), 1);
/// assert_eq!(refs[0].name, "ucum");
/// assert_eq!(&source[refs[0].span.position..refs[0].span.position + refs[0].span.length], "%ucum");
/// ```
pub fn external_constants(
    expr: &parser::SpannedExpression,
    source: &str,
) -> Vec<ExternalConstantRef> {
    let mut refs = Vec::new();
    collect_external_constants(expr, source, &mut refs);
    refs
}

fn collect_external_constants(
    expr: &parser::SpannedExpression,
    source: &str,
    out: &mut Vec<ExternalConstantRef>,
) {
    use parser::{SpannedExprKind, SpannedTerm};

    match &expr.kind {
        SpannedExprKind::Term(term) => match term {
            SpannedTerm::ExternalConstant(name) => out.push(ExternalConstantRef {
                name: name.clone(),
                span: exact_external_constant_span(source, &expr.span),
            }),
            SpannedTerm::Invocation(invocation) => {
                collect_external_constants_in_invocation(invocation, source, out)
            }
            SpannedTerm::Parenthesized(inner) => collect_external_constants(inner, source, out),
            SpannedTerm::Literal(_) => {}
        },
        SpannedExprKind::Invocation(base, invocation) => {
            collect_external_constants(base, source, out);
            collect_external_constants_in_invocation(invocation, source, out);
        }
        SpannedExprKind::Indexer(base, index) => {
            collect_external_constants(base, source, out);
            collect_external_constants(index, source, out);
        }
        SpannedExprKind::Polarity(_, inner) => collect_external_constants(inner, source, out),
        SpannedExprKind::Multiplicative(left, _, right)
        | SpannedExprKind::Additive(left, _, right)
        | SpannedExprKind::Inequality(left, _, right)
        | SpannedExprKind::Equality(left, _, right)
        | SpannedExprKind::Membership(left, _, right)
        | SpannedExprKind::Or(left, _, right)
        | SpannedExprKind::Union(left, right)
        | SpannedExprKind::And(left, right)
        | SpannedExprKind::Implies(left, right) => {
            collect_external_constants(left, source, out);
            collect_external_constants(right, source, out);
        }
        SpannedExprKind::Type(inner, _, _) => collect_external_constants(inner, source, out),
        SpannedExprKind::Lambda(_, inner) => collect_external_constants(inner, source, out),
        SpannedExprKind::InstanceSelector(_, fields) => {
            for (_, field_expr) in fields {
                collect_external_constants(field_expr, source, out);
            }
        }
    }
}

fn collect_external_constants_in_invocation(
    invocation: &parser::SpannedInvocation,
    source: &str,
    out: &mut Vec<ExternalConstantRef>,
) {
    if let parser::SpannedInvocation::Function(_, args) = invocation {
        for arg in args {
            collect_external_constants(arg, source, out);
        }
    }
}

/// Recomputes the exact `(position, length)` of an external-constant token
/// from `source`, discarding any trailing whitespace [`parser::spanned_parser`]
/// folded into `padded_span` (see [`external_constants`]'s doc comment).
///
/// Falls back to `padded_span` unchanged if `source` doesn't start a valid
/// external constant at that position — which should never happen for a
/// span the parser itself produced, but a silent fallback is safer for a
/// diagnostics helper than panicking on an unexpected drift between this
/// and the grammar.
fn exact_external_constant_span(source: &str, padded_span: &parser::ExprSpan) -> parser::ExprSpan {
    let tail = source.get(padded_span.position..).unwrap_or("");
    match external_constant_token_len(tail) {
        Some(length) => parser::ExprSpan {
            position: padded_span.position,
            length,
        },
        None => padded_span.clone(),
    }
}

/// Given `source` starting exactly at the `%` of an external constant,
/// returns the byte length of the token itself — `%` plus the bare
/// identifier, or plus a `` `delimited` `` / `'quoted'` name including its
/// closing delimiter. Returns `None` if `source` doesn't start with `%`
/// followed by a syntactically valid external-constant name.
///
/// Delimited/quoted forms may contain `\`-escaped characters (matching the
/// `esc` rule [`parser::parser`] and [`parser::spanned_parser`] both use for
/// these tokens); this only needs to skip over them without decoding them,
/// since the decoded name is already available from the parsed AST.
fn external_constant_token_len(source: &str) -> Option<usize> {
    if !source.starts_with('%') {
        return None;
    }
    let after_percent = '%'.len_utf8();
    let rest = &source[after_percent..];
    let mut chars = rest.char_indices();
    match chars.next() {
        Some((_, delimiter @ ('`' | '\''))) => {
            let mut escaped = false;
            for (i, c) in chars {
                if escaped {
                    escaped = false;
                    continue;
                }
                match c {
                    '\\' => escaped = true,
                    c if c == delimiter => return Some(after_percent + i + c.len_utf8()),
                    _ => {}
                }
            }
            None // Unterminated — shouldn't happen for a node the parser accepted.
        }
        Some((_, first)) if first.is_ascii_alphabetic() || first == '_' => {
            let mut end = after_percent + first.len_utf8();
            for (i, c) in chars {
                if c.is_ascii_alphanumeric() || c == '_' {
                    end = after_percent + i + c.len_utf8();
                } else {
                    break;
                }
            }
            Some(end)
        }
        _ => None,
    }
}

/// The environment variables [`evaluator`] resolves as special cases when
/// evaluating `%name` — the fixed subset of the [FHIRPath environment
/// variables](https://hl7.org/fhirpath/2025Jan/#environment-variables) that
/// have a single literal name. Excludes the `%vs-[name]`/`%ext-[name]`
/// families, which are patterns rather than enumerable names, and
/// `%terminologies`, which is a namespace object rather than a value.
///
/// Kept in sync with [`evaluator`]'s handling of these names by a test
/// (`each_environment_variable_evaluates_without_an_undefined_variable_error`)
/// that evaluates `%<name>` for each entry against a minimal context and
/// asserts it does not produce the "undefined variable" error an unknown
/// name like `%definitelyUnknown` does.
pub fn environment_variables() -> &'static [&'static str] {
    &[
        "context",
        "resource",
        "rootResource",
        "ucum",
        "sct",
        "loinc",
    ]
}

/// Returns `true` if `name` (without the leading `%`) is one of
/// [`environment_variables`].
pub fn is_environment_variable(name: &str) -> bool {
    environment_variables().contains(&name)
}
