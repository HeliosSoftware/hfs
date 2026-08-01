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
//!
//! ## Current limitations (hardening backlog)
//!
//! - Slice matchers: `pattern`, `type`, `profile`, and `binding` are
//!   evaluated. `resolve-ref` remains inert. Binding discriminators that
//!   name a ValueSet canonical (rather than an inline code) do not expand
//!   the ValueSet at mark time. The converter emits a warning when it
//!   cannot translate a discriminator into a match.
//! - `refers` (reference target types) is carried but not enforced.
//! - `extensible`-strength bindings are never checked (only `required`,
//!   per the FHIR Schema spec); a warning mode may come later.
//! - Constraint evaluation resolves `%resource`/`%rootResource` to the root
//!   resource and evaluates via `path.all(expr)`, so invariants relying on
//!   nested-resource `%resource` semantics can misfire (helios-fhirpath
//!   limitation; see `fhirpath_effects`).
//! - Schema sets are assembled per node without cross-resource memoization;
//!   structural validation of a typical Patient measures ~300µs in debug
//!   builds (see `tests/pack_smoke.rs`), so this has not been worth it yet.
//! - Core extension definitions (`extension-definitions.json`) are not in
//!   the vendored spec bundles, so pack profiles whose `extensions` sugar
//!   references core extension URLs report `unknown-schema` when exercised.

pub mod converter;
pub mod editor;
pub mod effects;
pub mod engine;
pub mod packages;
pub mod packs;
pub mod resolver;
pub mod schema;
pub mod terminology;

#[cfg(feature = "fhirpath")]
pub mod fhirpath_effects;

pub use editor::{
    Addable, AddableKind, Path, Present, Step, add_element, add_extension, addable, choose_type,
    node_at, path_from_string, path_to_string, present_children, remove_at, schema_at, set_value,
};
pub use effects::{
    CodedValue, ConstraintEvaluator, ConstraintOutcome, Deferred, DeferredConstraint,
    EffectHandlers, TerminologyError, TerminologyProvider,
};
pub use engine::{
    ErrorKind, Severity, SyncOutcome, UnknownProfilePolicy, ValidationError, ValidationOptions,
    Validator, dotted_to_fhirpath,
};
pub use packages::{
    MaterializeReport, PackageCache, PackageError, PackageId, PackageManifest, PackageRef,
    ResolvedPackage, ScannedPackage, materialize_package, materialize_package_layers,
    materialize_tgz, resolve_packages, scan_package_dir,
};
pub use resolver::{CompositeResolver, SchemaRegistry, SchemaResolver};
pub use schema::{Binding, Constraint, FhirSchema, Match, Slice, Slicing};
pub use terminology::{CoreTerminology, core_terminology};
