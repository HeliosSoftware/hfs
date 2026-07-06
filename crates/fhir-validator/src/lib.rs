//! # helios-fhir-validator
//!
//! FHIR resource validation for the Helios FHIR server, built on the
//! [FHIR Schema](https://fhir-schema.github.io/fhir-schema/) approach: a
//! JSON-Schema-like, differential-by-design compiled form of FHIR
//! StructureDefinitions, validated via **cooperative schema sets** rather
//! than snapshot flattening.
//!
//! ## Quick start
//!
//! ```
//! use helios_fhir_validator::{FhirSchema, SchemaRegistry, ValidationOptions, Validator};
//! use std::sync::Arc;
//!
//! let mut registry = SchemaRegistry::new();
//! registry.insert_named(
//!     "string",
//!     serde_json::from_str::<FhirSchema>(r#"{ "kind": "primitive-type" }"#).unwrap(),
//! );
//! registry.insert_named(
//!     "Patient",
//!     serde_json::from_str::<FhirSchema>(
//!         r#"{ "elements": { "resourceType": {"type": "string"}, "status": {"type": "string"} } }"#,
//!     )
//!     .unwrap(),
//! );
//!
//! let validator = Validator::new(Arc::new(registry));
//! let resource = serde_json::json!({ "resourceType": "Patient", "status": "active" });
//! let outcome = validator.validate_sync(&resource, &ValidationOptions::default());
//! assert!(outcome.errors.is_empty());
//! ```
//!
//! The engine walks raw `serde_json::Value` — deliberately, since the typed
//! `helios-fhir` models deserialize leniently and cannot surface unknown
//! elements. Structural validation is pure and synchronous; FHIRPath
//! constraints and terminology bindings are collected as [`Deferred`]
//! obligations for an async effects pass.
//!
//! The behavioral contract is the vendored FHIR Schema conformance suite in
//! `tests/fixtures/upstream/` (exact ordered error matching), plus Helios
//! extended fixtures in `tests/fixtures/extended/`.

pub mod converter;
pub mod effects;
pub mod engine;
pub mod packs;
pub mod resolver;
pub mod schema;

pub use effects::Deferred;
pub use engine::{
    dotted_to_fhirpath, ErrorKind, Severity, SyncOutcome, UnknownProfilePolicy, ValidationError,
    ValidationOptions, Validator,
};
pub use resolver::{CompositeResolver, SchemaRegistry, SchemaResolver};
pub use schema::{Binding, Constraint, FhirSchema, Match, Slice, Slicing};
