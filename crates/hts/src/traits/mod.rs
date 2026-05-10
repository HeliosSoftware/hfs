//! Trait definitions for the Helios Terminology Server.
//!
//! Each sub-module defines one slice of the backend contract:
//!
//! - [`CodeSystemOperations`] — `$lookup`, `$validate-code`, `$subsumes`
//! - [`ValueSetOperations`] — `$expand`, `$validate-code` (value-set form)
//! - [`ConceptMapOperations`] — `$translate`, `$closure`
//! - [`TerminologyMetadata`] — introspection helpers used by `/metadata`
//!
//! The [`TerminologyBackend`] supertrait aggregates all four with the
//! `Send + Sync + 'static` bounds required for sharing in [`AppState`].
//!
//! [`AppState`]: crate::state::AppState

mod code_system;
mod concept_map;
mod metadata;
mod value_set;

pub use code_system::{
    CodeSystemOperations, ConceptDesignation, ConceptExpansionFlags, SupplementInfo,
};
pub use concept_map::ConceptMapOperations;
pub use metadata::TerminologyMetadata;
pub use value_set::ValueSetOperations;

/// Combined supertrait that all HTS storage backends must implement.
///
/// An implementation of this trait is the single object placed in [`AppState`]
/// and shared across every Axum handler.  The trait bounds ensure that a
/// backend is:
///
/// - Fully operational (`CodeSystemOperations`, `ValueSetOperations`,
///   `ConceptMapOperations`)
/// - Introspectable (`TerminologyMetadata`)
/// - Safe to share across async tasks (`Send + Sync + 'static`)
///
/// [`AppState`]: crate::state::AppState
pub trait TerminologyBackend:
    CodeSystemOperations
    + ValueSetOperations
    + ConceptMapOperations
    + TerminologyMetadata
    + Send
    + Sync
    + 'static
{
}

/// Blanket impl: any type that satisfies all four sub-trait bounds automatically
/// implements `TerminologyBackend`.
impl<T> TerminologyBackend for T where
    T: CodeSystemOperations
        + ValueSetOperations
        + ConceptMapOperations
        + TerminologyMetadata
        + Send
        + Sync
        + 'static
{
}
