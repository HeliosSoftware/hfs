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
mod walk;

pub use errors::{ErrorKind, Severity, ValidationError};
pub use path::dotted_to_fhirpath;

use crate::effects::Deferred;
use crate::resolver::SchemaResolver;
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
}
