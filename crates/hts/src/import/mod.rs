pub mod fhir_bundle;

use async_trait::async_trait;
use helios_persistence::tenant::TenantContext;

use crate::error::HtsError;

/// Statistics returned from a single import operation.
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct ImportStats {
    /// Number of CodeSystem resources successfully imported.
    pub code_systems: u32,
    /// Number of ValueSet resources successfully imported.
    pub value_sets: u32,
    /// Number of ConceptMap resources successfully imported.
    pub concept_maps: u32,
    /// Total number of concept rows inserted.
    pub concepts: u32,
    /// Non-fatal errors (malformed resources, missing fields).
    /// The import continues past these; fatal errors are returned as `Err`.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub errors: Vec<String>,
}

impl ImportStats {
    /// Returns `true` if any non-fatal errors were recorded during import.
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }
}

/// Backend capability for FHIR Bundle import.
///
/// Separate from [`crate::traits::TerminologyBackend`] so that backends can opt
/// into import support independently.  The `POST /import` HTTP handler requires
/// `B: TerminologyBackend + BundleImportBackend`.
#[async_trait]
pub trait BundleImportBackend: Send + Sync {
    /// Parse a FHIR Bundle (raw JSON bytes) and insert all contained
    /// CodeSystem, ValueSet, and ConceptMap resources into the store.
    ///
    /// Resources are processed in dependency order:
    /// `CodeSystem`s first → `ValueSet`s → `ConceptMap`s.
    async fn import_bundle(
        &self,
        ctx: &TenantContext,
        data: &[u8],
    ) -> Result<ImportStats, HtsError>;
}
