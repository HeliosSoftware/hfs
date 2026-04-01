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

/// Trait for import pipeline implementations.
///
/// Each importer handles a specific content type and data format.
/// The MVP P0 implementation is [`fhir_bundle::FhirBundleImporter`].
/// Additional importers (SNOMED RF2, LOINC CSV) will be added in Phase 13.
#[allow(dead_code)]
#[async_trait]
pub trait Importer: Send + Sync {
    /// Human-readable name of this importer (used in logs).
    fn name(&self) -> &'static str;

    /// Returns `true` if this importer can process the given `Content-Type`.
    fn can_handle(&self, content_type: &str) -> bool;

    /// Import raw bytes into the terminology store using the given tenant context.
    ///
    /// Returns [`ImportStats`] with counts of successfully imported resources plus
    /// any non-fatal errors. Fatal errors (e.g. broken DB connection) are returned
    /// as `Err`.
    async fn import(&self, ctx: &TenantContext, data: &[u8]) -> Result<ImportStats, HtsError>;
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
