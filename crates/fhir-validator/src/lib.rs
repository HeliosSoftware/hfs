//! # helios-fhir-validator
//!
//! FHIR resource validation for the Helios FHIR server, built on the
//! [FHIR Schema](https://fhir-schema.github.io/fhir-schema/) approach: a
//! JSON-Schema-like, differential-by-design compiled form of FHIR
//! StructureDefinitions, validated via **cooperative schema sets** rather
//! than snapshot flattening.
//!
//! ## Overview
//!
//! | Concern | Module |
//! |---------|--------|
//! | StructureDefinition → schema | [`converter`] |
//! | Structural walk (cardinality, slices, fixed/pattern, …) | [`engine`] |
//! | FHIRPath constraints + terminology bindings | [`effects`], [`fhirpath_effects`] |
//! | Embedded core packs (R4–R6) | [`packs`], [`terminology`] |
//! | FHIR NPM / IG package overlays | [`packages`] |
//! | QuestionnaireResponse vs Questionnaire | [`questionnaire`] |
//! | Authoring projection (“what can I add?”) | [`editor`] |
//!
//! The engine walks raw `serde_json::Value` — deliberately, since the typed
//! `helios-fhir` models deserialize leniently and cannot surface unknown
//! elements. Structural validation is pure and synchronous; FHIRPath
//! constraints and terminology bindings are collected as [`Deferred`]
//! obligations for an async effects pass.
//!
//! Resolver layering (earlier wins): tenant stored StructureDefinitions →
//! package layers (filtered by `fhirVersions`) → embedded core pack.
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
//! The behavioral contract is the vendored FHIR Schema conformance suite in
//! `tests/fixtures/upstream/` (exact ordered error matching), plus Helios
//! extended fixtures in `tests/fixtures/extended/`. Package materialization
//! is documented in [`docs/packages.md`](../docs/packages.md).
//!
//! ## Current limitations
//!
//! - Slice matchers: `pattern`, `type`, `profile`, `binding`, `exists`, and
//!   `extension` (paths traversing `extension('url')`) are evaluated;
//!   reslices (`parent/child`) are scoped to the parent match. Discriminator
//!   paths using `resolve()` remain unsupported (need an instance graph), as
//!   does `resolve-ref`. Binding discriminators that name a ValueSet
//!   canonical do not expand it at mark time.
//! - `refers` (reference target types) is enforced only when
//!   [`ValidationOptions::enforce_refers`] is set (off by default for
//!   conformance-suite parity). Profile-target resolution is not performed.
//! - `extensible`-strength bindings emit warnings only when
//!   [`EffectHandlers::check_extensible_bindings`] is set; `preferred` /
//!   `example` are never checked.
//! - Constraint evaluation resolves `%resource`/`%rootResource` to the root
//!   resource and evaluates via `path.all(expr)`, so invariants relying on
//!   nested-resource `%resource` semantics can misfire (helios-fhirpath
//!   limitation; see `fhirpath_effects`).
//! - Non-goals: XHTML well-formedness, Bundle `fullUrl` uniqueness rules.
//! - Core extension definitions (`extension-definitions.json`) are not in
//!   the vendored spec bundles, so pack profiles whose `extensions` sugar
//!   references core extension URLs report `unknown-schema` when exercised
//!   without an IG package that provides them.

pub mod converter;
pub mod editor;
pub mod effects;
pub mod engine;
pub mod packages;
pub mod packs;
pub mod questionnaire;
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
    ResolvedPackage, ScannedPackage, ensure_package_path, manifest_supports_fhir_version,
    materialize_package, materialize_package_layers, materialize_package_layers_by_version,
    materialize_tgz, resolve_packages, scan_package_dir,
};
pub use questionnaire::validate_questionnaire_response;
pub use resolver::{CompositeResolver, SchemaRegistry, SchemaResolver};
pub use schema::{Binding, Constraint, FhirSchema, Match, Slice, Slicing};
pub use terminology::{CoreTerminology, core_terminology};
