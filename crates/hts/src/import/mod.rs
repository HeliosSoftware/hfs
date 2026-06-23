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
//! | [`ndc`] | FDA NDC Directory (`product.txt` or `.zip`) | Free (public domain) |
//!
//! All importers are invoked from the CLI (`hts import`) via `main.rs` using
//! the format auto-detection in [`crate::config::detect_format`], or
//! programmatically via [`BundleImportBackend::import_bundle`] for HTTP-based
//! Bundle imports.

pub mod bundle_builder;
pub mod bundle_parser;
pub mod dicom;
pub mod fhir_bundle;
pub mod hl7_v2_tables;
pub mod icd10_cm;
pub mod icd9_cm;
pub mod loinc_csv;
pub mod mesh;
pub mod nci_thesaurus;
pub mod ndc;
pub mod nucc;
pub mod rxnorm_rrf;
pub mod snomed_rf2;
pub mod tgz;
pub mod ucum;

use async_trait::async_trait;
use helios_persistence::tenant::TenantContext;

use crate::error::HtsError;

/// Restricts which translation languages multilingual importers ingest.
///
/// Built from a comma-separated list of BCP-47 tags (`HTS_IMPORT_LANGUAGES`
/// env var / `--languages` CLI flag). An empty list means *no restriction* —
/// every language present in the source archive is imported, which is the
/// historical behavior.
///
/// Matching is BCP-47-aware via [`crate::language::lang_match_rank`], which
/// ranks a stored tag against a configured tag in the preference order
/// "`es-ES`, then `esES`, then `es`/`es*`". Two selection modes are built on it:
///
/// * [`allows`](Self::allows) — a per-tag predicate (any rank matches). Used by
///   importers whose stored designation languages are bare ISO codes (SNOMED
///   RF2 `languageCode`), where there are no regional siblings to disambiguate.
/// * [`resolve_retained`](Self::resolve_retained) — *best-tier* selection over a
///   known set of candidate tags: for each configured tag it keeps only the
///   candidates sharing the best (lowest) rank. So a configured `es-ES` keeps
///   `es-ES` when present (excluding `es-AR`/`es-MX`), but a configured `es`
///   keeps every `es-*`. Used by the LOINC importer, whose linguistic-variant
///   files are region-qualified and enumerable up front.
///
/// The filter only governs *translations* (SNOMED descriptions per language,
/// LOINC linguistic variants); the English content that drives concept
/// displays is never filtered out.
#[derive(Debug, Clone, Default)]
pub struct LanguageFilter {
    /// Normalized (trimmed, lowercased) BCP-47 tags. Empty = allow all.
    tags: Vec<String>,
}

impl LanguageFilter {
    /// Parse a comma-separated tag list. Whitespace around tags is ignored;
    /// empty segments are dropped. An empty or all-whitespace `spec` yields
    /// the allow-all filter.
    pub fn parse(spec: &str) -> Self {
        let mut tags: Vec<String> = spec
            .split(',')
            .map(|t| t.trim().to_ascii_lowercase())
            .filter(|t| !t.is_empty())
            .collect();
        tags.sort();
        tags.dedup();
        Self { tags }
    }

    /// `true` when no restriction is configured (every language imported).
    pub fn allows_all(&self) -> bool {
        self.tags.is_empty()
    }

    /// `true` when designations in `language` should be imported.
    pub fn allows(&self, language: &str) -> bool {
        self.allows_all()
            || self
                .tags
                .iter()
                .any(|t| crate::language::lang_matches(t, language))
    }

    /// Best-tier selection over a known set of candidate language tags.
    ///
    /// For each configured tag, finds the best (lowest) match rank achievable
    /// among `available` and keeps every candidate sharing that rank; the
    /// retained set is the union across all configured tags. The allow-all
    /// filter keeps every candidate.
    ///
    /// This is what makes a configured `es-ES` keep only `es-ES` when it is
    /// present (its tier-0 exact match outranks the tier-2 siblings `es-AR` /
    /// `es-MX`), while a configured `es` — which can only ever reach tier 2 —
    /// keeps every `es-*` variant. Returned tags preserve the casing of the
    /// `available` inputs so callers can test membership against the same
    /// strings they passed in.
    pub fn resolve_retained<I, S>(&self, available: I) -> std::collections::HashSet<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let available: Vec<String> = available
            .into_iter()
            .map(|s| s.as_ref().to_string())
            .collect();
        if self.allows_all() {
            return available.into_iter().collect();
        }
        let mut keep = std::collections::HashSet::new();
        for tag in &self.tags {
            let Some(best) = available
                .iter()
                .filter_map(|a| crate::language::lang_match_rank(tag, a))
                .min()
            else {
                continue;
            };
            for a in &available {
                if crate::language::lang_match_rank(tag, a) == Some(best) {
                    keep.insert(a.clone());
                }
            }
        }
        keep
    }

    /// Canonical form of the configured tag list (sorted, lowercased,
    /// comma-joined). Folded into the bootstrap ledger signature so changing
    /// the language set re-triggers imports on the next startup.
    pub fn canonical_spec(&self) -> String {
        self.tags.join(",")
    }
}

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

    /// Accumulate counts and errors from another import into this one.
    ///
    /// Used when `hts import <dir>` imports multiple files in sequence and
    /// needs to report an aggregate summary.
    pub fn merge(&mut self, other: ImportStats) {
        self.code_systems = self.code_systems.saturating_add(other.code_systems);
        self.value_sets = self.value_sets.saturating_add(other.value_sets);
        self.concept_maps = self.concept_maps.saturating_add(other.concept_maps);
        self.concepts = self.concepts.saturating_add(other.concepts);
        self.errors.extend(other.errors);
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

    /// Import an already-parsed bundle, skipping the JSON parse step.
    ///
    /// Filesystem importers that batch large code systems (SNOMED RF2, LOINC)
    /// construct [`bundle_parser::ParsedBundle`] values directly via
    /// `bundle_builder::build_parsed_code_system` and feed them here, avoiding
    /// a serialise→reparse round-trip per batch. Semantics are identical to
    /// [`Self::import_bundle`] after parsing.
    async fn import_parsed(
        &self,
        ctx: &TenantContext,
        parsed: bundle_parser::ParsedBundle,
    ) -> Result<ImportStats, HtsError>;

    /// `true` when the code system at `url` already has at least one concept
    /// stored. Chunked importers call this **once** before a bulk load to decide
    /// whether it is a fresh first-time import (no prior concepts) — in which
    /// case they set [`ParsedBundle::fresh_load`] to skip the pointless
    /// delete-before-reinsert on every concept of every batch.
    ///
    /// The default returns `true` (conservative: assume data may exist, so
    /// deletes are not skipped), keeping any backend that does not override it
    /// correct, if slightly slower.
    async fn code_system_has_concepts(
        &self,
        _ctx: &TenantContext,
        _url: &str,
    ) -> Result<bool, HtsError> {
        Ok(true)
    }

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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::LanguageFilter;

    #[test]
    fn empty_spec_allows_everything() {
        for spec in ["", "  ", ",", " , "] {
            let f = LanguageFilter::parse(spec);
            assert!(f.allows_all(), "spec {spec:?} must be allow-all");
            assert!(f.allows("de"));
            assert!(f.allows("zh-Hans-CN"));
            assert_eq!(f.canonical_spec(), "");
        }
    }

    #[test]
    fn filter_matches_bcp47_in_both_directions() {
        let f = LanguageFilter::parse("de, fr-FR");
        assert!(!f.allows_all());
        // Configured bare tag admits regional variants.
        assert!(f.allows("de"));
        assert!(f.allows("de-DE"));
        // Configured regional tag admits the bare RF2 language code.
        assert!(f.allows("fr"));
        assert!(f.allows("fr-FR"));
        // Unrelated languages are rejected; prefix without a subtag
        // boundary must not match.
        assert!(!f.allows("da"));
        assert!(!f.allows("den"));
    }

    #[test]
    fn canonical_spec_is_sorted_lowercased_deduped() {
        let f = LanguageFilter::parse("FR-fr, de , fr-FR,de");
        assert_eq!(f.canonical_spec(), "de,fr-fr");
    }

    fn sorted(set: std::collections::HashSet<String>) -> Vec<String> {
        let mut v: Vec<String> = set.into_iter().collect();
        v.sort();
        v
    }

    #[test]
    fn resolve_retained_picks_best_tier_per_tag() {
        // The motivating LOINC case: filter `de,es-ES` against the regional
        // linguistic-variant files. `es-ES` is an exact (tier-0) match, so its
        // siblings es-AR/es-MX are excluded; `de` can only reach tier 2, so it
        // keeps every German regional variant.
        let f = LanguageFilter::parse("de,es-ES");
        let available = ["de-AT", "de-DE", "es-AR", "es-ES", "es-MX"];
        assert_eq!(
            sorted(f.resolve_retained(available)),
            vec!["de-AT", "de-DE", "es-ES"]
        );
    }

    #[test]
    fn resolve_retained_bare_tag_keeps_all_regionals() {
        // A bare `es` only ever reaches tier 2, so every es-* sibling ties and
        // is kept — "es or es*".
        let f = LanguageFilter::parse("es");
        let available = ["es-AR", "es-ES", "es-MX"];
        assert_eq!(
            sorted(f.resolve_retained(available)),
            vec!["es-AR", "es-ES", "es-MX"]
        );
    }

    #[test]
    fn resolve_retained_falls_back_to_bare_then_siblings() {
        // No exact es-ES present: the best tier reachable is 2, shared by the
        // bare `es` and the regional siblings, so all of them are kept.
        let f = LanguageFilter::parse("es-ES");
        let available = ["es", "es-AR", "es-MX"];
        assert_eq!(
            sorted(f.resolve_retained(available)),
            vec!["es", "es-AR", "es-MX"]
        );
    }

    #[test]
    fn resolve_retained_allow_all_keeps_everything() {
        let f = LanguageFilter::parse("");
        let available = ["de-DE", "es-MX"];
        assert_eq!(
            sorted(f.resolve_retained(available)),
            vec!["de-DE", "es-MX"]
        );
    }
}
