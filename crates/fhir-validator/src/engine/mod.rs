//! The cooperative validation engine.
//!
//! Validation never flattens schemas: it builds a *set* of schemas (the
//! *schemata*) for every data node — root schema from `resourceType`,
//! profiles, plus every transitively referenced `base` and complex `type` —
//! and every member of the set judges the same node together. See
//! `docs/guide/mental-model.md` in the FHIR Schema repo for the model, and
//! `walk.rs` for the deterministic error-emission order.

mod errors;
mod path;
mod primitives;
mod slicing;
mod walk;

pub use errors::{ErrorKind, Severity, ValidationError};
pub use path::dotted_to_fhirpath;

/// Crate-internal access to the effect-issue message templates (the full
/// template catalog lives in `errors.rs`, the single source of truth).
pub(crate) mod messages {
    pub(crate) use super::errors::{
        msg_constraint_not_evaluable as constraint_not_evaluable,
        msg_fhirpath_constraint as fhirpath_constraint,
        msg_terminology_binding as terminology_binding,
        msg_terminology_unavailable as terminology_unavailable,
    };
}

use crate::effects::{Deferred, EffectHandlers};
use crate::resolver::SchemaResolver;
use helios_fhir::FhirVersion;
use serde_json::Value;
use std::sync::Arc;

/// Policy for profile references (`meta.profile` / caller-supplied) that the
/// resolver cannot resolve.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UnknownProfilePolicy {
    /// Report as a warning-severity issue (default).
    #[default]
    Warn,
    /// Report as an error-severity issue.
    Error,
    /// Skip silently.
    Ignore,
}

/// Options for one validation run.
#[derive(Debug, Clone)]
pub struct ValidationOptions {
    /// Extra profile canonicals/names to layer on, in addition to the root
    /// schema and (optionally) `meta.profile`.
    pub profiles: Vec<String>,
    /// Layer on the profiles the resource claims in `meta.profile`.
    pub use_meta_profiles: bool,
    /// What to do when a profile reference cannot be resolved.
    pub unknown_profile: UnknownProfilePolicy,
}

impl Default for ValidationOptions {
    fn default() -> Self {
        Self {
            profiles: Vec::new(),
            use_meta_profiles: true,
            unknown_profile: UnknownProfilePolicy::default(),
        }
    }
}

/// Result of the pure synchronous walk.
#[derive(Debug, Default)]
pub struct SyncOutcome {
    /// Structural issues, in deterministic emission order.
    pub errors: Vec<ValidationError>,
    /// Constraint/binding obligations for the async effects pass (Phase 4).
    pub deferred: Vec<Deferred>,
}

/// The validator: a resolver plus the walk.
pub struct Validator {
    resolver: Arc<dyn SchemaResolver>,
}

impl Validator {
    pub fn new(resolver: Arc<dyn SchemaResolver>) -> Self {
        Self { resolver }
    }

    /// Pure, synchronous structural validation. No I/O, no FHIRPath —
    /// constraint/binding obligations are returned as [`Deferred`] entries
    /// instead of being executed.
    pub fn validate_sync(&self, resource: &Value, opts: &ValidationOptions) -> SyncOutcome {
        walk::validate(self.resolver.as_ref(), resource, opts)
    }

    /// Full validation: the structural walk plus execution of the deferred
    /// FHIRPath constraints and terminology bindings through `effects`.
    /// Structural issues come first, then constraint issues, then binding
    /// issues (each in walk order).
    pub async fn validate(
        &self,
        resource: &Value,
        version: FhirVersion,
        opts: &ValidationOptions,
        effects: &EffectHandlers<'_>,
    ) -> Vec<ValidationError> {
        let SyncOutcome { mut errors, deferred } = self.validate_sync(resource, opts);
        crate::effects::execute(resource, version, &deferred, effects, &mut errors).await;
        errors
    }
}
