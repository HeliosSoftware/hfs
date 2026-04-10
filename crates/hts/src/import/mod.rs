//! Terminology package importers and the [`BundleImportBackend`] trait.
//!
//! Each sub-module handles a specific terminology distribution format.
//! The shared [`bundle_parser`] module handles all JSON-walking logic so that
//! both the SQLite and PostgreSQL backends consume the same intermediate types.
//!
//! ## Supported formats
//!
//! | Module | Format | Source |
//! |--------|--------|--------|
//! | [`fhir_bundle`] | FHIR Bundle JSON | Any FHIR server or file |
//! | [`tgz`] | HL7 FHIR NPM package (`.tgz`) | <https://terminology.hl7.org> |
//! | [`snomed_rf2`] | SNOMED CT RF2 (`.zip`) | NRC license required |
//! | [`loinc_csv`] | LOINC CSV (`.zip`) | Free Regenstrief registration |
//! | [`icd10_cm`] | ICD-10-CM tabular XML | Free (CMS / CDC) |
//! | [`icd9_cm`]  | ICD-9-CM pipe-delimited text  | Free (public domain, retired 2015) |
//! | [`rxnorm_rrf`] | RxNorm RRF (folder or `.zip`) | Free NLM Terms of Service |
//!
//! All importers are invoked from the CLI (`hts import`) via `main.rs` using
//! the format auto-detection in [`crate::config::detect_format`], or
//! programmatically via [`BundleImportBackend::import_bundle`] for HTTP-based
//! Bundle imports.

pub mod bundle_parser;
pub mod dicom;
pub mod fhir_bundle;
pub mod hl7_v2_tables;
pub mod icd10_cm;
pub mod icd9_cm;
pub mod loinc_csv;
pub mod mesh;
pub mod nci_thesaurus;
pub mod nucc;
pub mod rxnorm_rrf;
pub mod snomed_rf2;
pub mod tgz;
pub mod ucum;

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

/// Outcome of a completed CLI import run.
///
/// Wraps [`ImportStats`] with the format label and wall-clock duration so that
/// `main.rs` can print a consistent summary line regardless of which importer
/// produced the result.
#[derive(Debug)]
pub struct ImportResult {
    /// Counts of imported resources and any non-fatal errors.
    pub stats: ImportStats,
    /// Human-readable format label (e.g. `"hl7-npm"`, `"loinc"`).
    pub format: String,
    /// Total wall-clock time for the import.
    pub duration: std::time::Duration,
}

impl ImportResult {
    /// Build an `ImportResult` from its components.
    pub fn new(
        stats: ImportStats,
        format: impl Into<String>,
        duration: std::time::Duration,
    ) -> Self {
        Self {
            stats,
            format: format.into(),
            duration,
        }
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

    /// Remove all HTS normalized rows for the resource identified by `resource_url`.
    ///
    /// Called by the CRUD DELETE handler after the persistence soft-delete so
    /// that `$lookup`, `$expand`, and `$search` no longer return stale data.
    ///
    /// The default implementation is a no-op.  Backends that manage their own
    /// normalized tables (e.g. `PostgresTerminologyBackend`) override this.
    /// The SQLite backend uses a separate `hts_pool`-based path in `crud.rs`
    /// and relies on the default here.
    async fn delete_normalized(
        &self,
        _resource_type: &str,
        _resource_url: &str,
    ) -> Result<(), HtsError> {
        Ok(())
    }
}
