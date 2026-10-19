//! Introspection trait for terminology backends.
//!
//! [`TerminologyMetadata`] exposes capability metadata and indexed identity
//! reads for stored terminology resources.
#![allow(dead_code)]

/// Introspection trait for terminology backends.
///
/// Supplies capability metadata and stored resource identities. Implementations
/// may query their terminology store; these methods are not limited to in-memory
/// metadata.
pub trait TerminologyMetadata: Send + Sync {
    /// A short name identifying this backend (e.g. `"sqlite"`, `"postgres"`).
    fn backend_name(&self) -> &'static str;

    /// The canonical URLs of all code systems currently stored in this backend.
    ///
    /// Used to populate `TerminologyCapabilities.codeSystem[].uri`.
    fn supported_systems(&self) -> Vec<String>;

    /// Whether this backend supports the `$subsumes` operation.
    ///
    /// Returns `false` for backends that have not imported hierarchy data.
    fn supports_subsumption(&self) -> bool;

    /// Return the canonical URL for a resource identified by its FHIR `id`.
    ///
    /// `resource_type` must be `"CodeSystem"`, `"ValueSet"`, or `"ConceptMap"`.
    /// Returns [`None`] when no resource with that ID exists.
    ///
    /// Used where a canonical URL uniquely identifies the stored resource, such
    /// as ConceptMap translation. Versioned instance operations must retain the
    /// exact public identity instead.
    fn resource_url_by_id(&self, resource_type: &str, id: &str) -> Option<String>;

    /// Read an imported terminology resource by its unique public identity.
    /// Unlike canonical resolution, this selects exactly one business version.
    fn resource_by_id(
        &self,
        resource_type: &str,
        id: &str,
    ) -> Result<Option<serde_json::Value>, crate::error::HtsError>;

    /// Read only identity and language metadata for an exact public ID.
    /// Avoids hydrating a full CodeSystem merely to route an instance operation.
    fn resource_identity_by_id(
        &self,
        resource_type: &str,
        id: &str,
    ) -> Result<Option<crate::types::ResourceIdentity>, crate::error::HtsError>;
}
