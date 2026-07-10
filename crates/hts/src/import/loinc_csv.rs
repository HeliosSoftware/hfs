//! LOINC CSV importer.
//!
//! Reads a LOINC distribution ZIP and imports LOINC codes, preferred display
//! names, status, and the multi-axial hierarchy into the HTS normalized schema.
//!
//! ## Linguistic variants (language translations)
//!
//! Official LOINC releases ship language translations under
//! `AccessoryFiles/LinguisticVariants/`: an index file `LinguisticVariants.csv`
//! (mapping a numeric variant id → ISO language / country / display name) plus
//! one CSV per language named like `frFR28LinguisticVariant.csv`. Each such file
//! carries translated `LONG_COMMON_NAME` / `SHORTNAME` columns keyed by
//! `LOINC_NUM`. When present, those translations are imported as FHIR
//! `concept.designation` entries tagged with the BCP-47 language (e.g. `fr-FR`,
//! `de-DE`). The English `display` is left untouched. Because both the `hts
//! import` CLI and the server bootstrap funnel LOINC ZIPs through
//! [`import_loinc_csv`], language variants are picked up automatically by both
//! paths — no extra flag or file is required. The imported language set can
//! optionally be restricted via [`LanguageFilter`] (`HTS_IMPORT_LANGUAGES` /
//! `--languages`); excluded variant files are skipped without being parsed.
//!
//! # ⚠️  LICENSE REQUIRED
//!
//! Real LOINC data requires a free license from the Regenstrief Institute.
//! Register at <https://loinc.org/license/> (takes ~5 minutes, no approval wait).
//! This parser was written and tested using **synthetic fixture data only**.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use helios_persistence::tenant::TenantContext;
use zip::ZipArchive;

use crate::error::HtsError;
use crate::import::BundleImportBackend;
use crate::import::ImportStats;
use crate::import::LanguageFilter;
use crate::import::bundle_builder::{
    BuilderConcept, BuilderDesignation, CodeSystemMeta, build_parsed_code_system,
};

// ── LOINC constants ───────────────────────────────────────────────────────────

const LOINC_URL: &str = "http://loinc.org";
const LOINC_ID: &str = "loinc";
const LOINC_NAME: &str = "LOINC";
const LOINC_TITLE: &str = "Logical Observation Identifiers Names and Codes (LOINC)";

// ── Data types ────────────────────────────────────────────────────────────────

#[derive(Debug)]
struct LoincConcept {
    display: String,
    /// Raw STATUS value from LoincTable: ACTIVE, DEPRECATED, DISCOURAGED, TRIAL.
    status: String,
}

/// Merged concept map: code → (display, optional definition, optional parent).
type ConceptEntries = HashMap<String, MergedConcept>;

#[derive(Debug, Default)]
struct MergedConcept {
    display: String,
    definition: Option<String>,
    parent: Option<String>,
    /// Translated display names from LOINC linguistic-variant files. Owned so
    /// the borrowed [`BuilderDesignation`] slice can be assembled per batch.
    designations: Vec<OwnedDesignation>,
}

/// One translated display name, owned by its [`MergedConcept`].
#[derive(Debug, Clone)]
struct OwnedDesignation {
    /// BCP-47 language tag, e.g. `fr-FR`, `de-DE`, `zh-CN`.
    language: String,
    value: String,
}

#[derive(Debug, Default)]
struct LoincParseResult {
    concepts: ConceptEntries,
    loinc_count: usize,
    lp_count: usize,
    edge_count: usize,
    designation_count: usize,
    /// Per-language designation counts, sorted for deterministic reporting.
    designations_by_language: Vec<(String, usize)>,
    parse_errors: Vec<String>,
    /// Release version parsed from a `Loinc_<ver>/` ZIP path segment, if present.
    release_version: Option<String>,
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Import a LOINC distribution ZIP through the given backend.
///
/// `languages` restricts which linguistic-variant translations are ingested
/// as designations (see [`LanguageFilter`]); excluded variant files are never
/// parsed. The English main-table content is unaffected.
pub async fn import_loinc_csv(
    backend: &dyn BundleImportBackend,
    ctx: &TenantContext,
    path: &Path,
    batch_size: usize,
    dry_run: bool,
    languages: &LanguageFilter,
) -> Result<ImportStats, HtsError> {
    const FORMAT: &str = "loinc";
    let batch_size = batch_size.max(1);

    let path_owned = path.to_path_buf();
    let languages = languages.clone();
    let parsed = tokio::task::spawn_blocking(move || -> Result<LoincParseResult, HtsError> {
        let paths = find_loinc_paths(&path_owned)?;
        let loinc_table_path = &paths.loinc_table;
        let hierarchy_path = &paths.hierarchy;

        tracing::info!(
            loinc_table = %loinc_table_path,
            hierarchy = %hierarchy_path,
            linguistic_variants = paths.lv_variants.len(),
            "Located LOINC CSV files in archive"
        );

        let mut parse_errors: Vec<String> = Vec::new();

        let loinc_concepts = {
            let mut zip = open_zip(&path_owned)?;
            let entry = zip
                .by_name(loinc_table_path)
                .map_err(|e| HtsError::InvalidRequest(format!("Cannot open LoincTable: {e}")))?;
            parse_loinc_table(entry, &mut parse_errors)?
        };

        let (hierarchy_concepts, edges) = {
            let mut zip = open_zip(&path_owned)?;
            let entry = zip.by_name(hierarchy_path).map_err(|e| {
                HtsError::InvalidRequest(format!("Cannot open MultiAxialHierarchy: {e}"))
            })?;
            parse_hierarchy(entry, &mut parse_errors)?
        };

        // Linguistic variants (language translations). The index file maps a
        // numeric variant id → ISO language/country; per-language files carry
        // the translated LONG_COMMON_NAME keyed by LOINC_NUM. Each parsed file
        // yields (loinc_code, language_tag, translated_display) tuples.
        let lv_index = match &paths.lv_index {
            Some(idx_path) => {
                let mut zip = open_zip(&path_owned)?;
                match zip.by_name(idx_path) {
                    Ok(entry) => parse_linguistic_index(entry, &mut parse_errors),
                    Err(e) => {
                        parse_errors.push(format!(
                            "Cannot open {idx_path}: {e} — \
                             language tags will be derived from filenames"
                        ));
                        HashMap::new()
                    }
                }
            }
            None => HashMap::new(),
        };

        // First pass: resolve every linguistic-variant file's language tag so
        // the language filter can make a *best-tier* decision over the whole
        // candidate set (e.g. keep `es-ES` and drop `es-AR`/`es-MX`), rather
        // than admitting every tag that loosely matches.
        let mut variant_langs: Vec<(&String, String, String)> = Vec::new();
        for variant_path in &paths.lv_variants {
            let filename = variant_path
                .rsplit('/')
                .next()
                .unwrap_or(variant_path)
                .to_string();
            match derive_language_tag(&filename, &lv_index) {
                Some(language) => variant_langs.push((variant_path, filename, language)),
                None => parse_errors.push(format!(
                    "Cannot determine language for linguistic-variant file '{filename}' — skipped"
                )),
            }
        }
        // Best-tier retained set across all candidate languages. English
        // designations are always kept (consistent with SNOMED), independent of
        // the filter.
        let retained = languages.resolve_retained(variant_langs.iter().map(|(_, _, l)| l.as_str()));

        // code → (language → translated display). Last writer wins per language.
        let mut translations: HashMap<String, HashMap<String, String>> = HashMap::new();
        for (variant_path, filename, language) in &variant_langs {
            if !crate::language::lang_matches("en", language) && !retained.contains(language) {
                tracing::info!(
                    file = %filename,
                    language = %language,
                    "skipping LOINC linguistic variant excluded by language filter"
                );
                continue;
            }
            let mut zip = open_zip(&path_owned)?;
            let entry = match zip.by_name(variant_path.as_str()) {
                Ok(e) => e,
                Err(e) => {
                    parse_errors.push(format!("Cannot open {variant_path}: {e} — skipped"));
                    continue;
                }
            };
            let rows = parse_linguistic_variant(entry, filename, &mut parse_errors)?;
            for (code, value) in rows {
                translations
                    .entry(code)
                    .or_default()
                    .insert(language.clone(), value);
            }
        }

        // Build parent map from the edges (child → parent).
        let mut parent_of: HashMap<String, String> = HashMap::new();
        for (child, parent) in &edges {
            // A code may appear under multiple parents; keep the first edge
            // seen to match the old behaviour where the `concept_hierarchy`
            // table stored only one incoming edge per child/parent pair.
            parent_of
                .entry(child.clone())
                .or_insert_with(|| parent.clone());
        }

        let mut all_concepts: ConceptEntries = HashMap::new();
        for (code, display) in &hierarchy_concepts {
            all_concepts.insert(
                code.clone(),
                MergedConcept {
                    display: display.clone(),
                    definition: None,
                    parent: parent_of.get(code).cloned(),
                    ..Default::default()
                },
            );
        }
        for (code, concept) in &loinc_concepts {
            let definition = if concept.status != "ACTIVE" {
                Some(format!("STATUS:{}", concept.status))
            } else {
                None
            };
            all_concepts.insert(
                code.clone(),
                MergedConcept {
                    display: concept.display.clone(),
                    definition,
                    parent: parent_of.get(code).cloned(),
                    ..Default::default()
                },
            );
        }

        // Attach translated display names as designations on the concepts that
        // exist in this CodeSystem. Translations for unknown codes are recorded
        // as parse errors rather than creating orphan concepts.
        let mut designation_count: usize = 0;
        let mut per_language: HashMap<String, usize> = HashMap::new();
        for (code, langs) in translations {
            let Some(concept) = all_concepts.get_mut(&code) else {
                parse_errors.push(format!(
                    "Linguistic variant for unknown LOINC code '{code}' — skipped"
                ));
                continue;
            };
            // Sort by language for deterministic designation ordering.
            let mut langs: Vec<(String, String)> = langs.into_iter().collect();
            langs.sort_by(|a, b| a.0.cmp(&b.0));
            for (language, value) in langs {
                *per_language.entry(language.clone()).or_default() += 1;
                designation_count += 1;
                concept
                    .designations
                    .push(OwnedDesignation { language, value });
            }
        }

        let mut designations_by_language: Vec<(String, usize)> = per_language.into_iter().collect();
        designations_by_language.sort_by(|a, b| a.0.cmp(&b.0));

        Ok(LoincParseResult {
            concepts: all_concepts,
            loinc_count: loinc_concepts.len(),
            lp_count: hierarchy_concepts.len(),
            edge_count: edges.len(),
            designation_count,
            designations_by_language,
            parse_errors,
            release_version: paths.release_version,
        })
    })
    .await
    .map_err(|e| HtsError::Internal(format!("LOINC parser panicked: {e}")))??;

    let total_concepts = parsed.concepts.len() as u32;

    let mut stats = ImportStats {
        code_systems: 1,
        errors: parsed.parse_errors,
        ..Default::default()
    };

    let lang_summary = format_language_summary(&parsed.designations_by_language);

    if dry_run {
        stats.concepts = total_concepts;
        eprintln!(
            "[{FORMAT}] dry-run — would import {total_concepts} concepts \
             ({} LOINC codes, {} LP category nodes), {} hierarchy edges, \
             {} translated designations{lang_summary}",
            parsed.loinc_count, parsed.lp_count, parsed.edge_count, parsed.designation_count,
        );
        return Ok(stats);
    }

    let release_version = parsed.release_version.clone();
    let meta = CodeSystemMeta {
        id: LOINC_ID,
        url: LOINC_URL,
        version: release_version.as_deref(),
        name: Some(LOINC_NAME),
        title: Some(LOINC_TITLE),
        status: "active",
        content: "complete",
    };

    // Probe once before loading: if the system has no concepts yet, every
    // concept in every batch is brand-new, so the per-concept
    // delete-before-reinsert can be skipped across the whole load. On a
    // re-import (system already populated) we leave it off so replacement
    // semantics still apply. Probe failure falls back to the safe path.
    let fresh_load = !backend
        .code_system_has_concepts(ctx, LOINC_URL)
        .await
        .unwrap_or(true);

    // Seed: empty CodeSystem to upsert metadata.
    let seed = build_parsed_code_system(&meta, &[]);
    let seed_stats = backend.import_parsed(ctx, seed).await?;
    stats.code_systems = seed_stats.code_systems;
    stats.errors.extend(seed_stats.errors);

    // Collect concepts into a stable order so chunks across retries are
    // deterministic.
    let concept_list: Vec<(&String, &MergedConcept)> = parsed.concepts.iter().collect();
    let num_batches = concept_list.len().div_ceil(batch_size).max(1);

    for (i, chunk) in concept_list.chunks(batch_size).enumerate() {
        // Build the borrowed designation slices first so they outlive the
        // `BuilderConcept`s that reference them within this iteration.
        let desig_sets: Vec<Vec<BuilderDesignation<'_>>> = chunk
            .iter()
            .map(|(_, entry)| {
                entry
                    .designations
                    .iter()
                    .map(|d| BuilderDesignation {
                        language: Some(d.language.as_str()),
                        use_system: None,
                        use_code: None,
                        value: d.value.as_str(),
                    })
                    .collect()
            })
            .collect();

        let builder: Vec<BuilderConcept<'_>> = chunk
            .iter()
            .zip(&desig_sets)
            .map(|((code, entry), desigs)| BuilderConcept {
                code: code.as_str(),
                display: Some(entry.display.as_str()).filter(|s| !s.is_empty()),
                definition: entry.definition.as_deref(),
                parent_code: entry.parent.as_deref(),
                designations: desigs.as_slice(),
                ..Default::default()
            })
            .collect();

        let mut parsed = build_parsed_code_system(&meta, &builder);
        parsed.fresh_load = fresh_load;
        let chunk_stats = backend.import_parsed(ctx, parsed).await?;
        stats.errors.extend(chunk_stats.errors);
        stats.concepts += chunk.len() as u32;

        eprintln!(
            "[{FORMAT}] concept batch {}/{num_batches} — +{} concepts (total: {})",
            i + 1,
            chunk.len(),
            stats.concepts,
        );
    }

    if parsed.designation_count > 0 {
        eprintln!(
            "[{FORMAT}] imported {} translated designations{lang_summary}",
            parsed.designation_count,
        );
    }

    Ok(stats)
}

// ── ZIP helpers ───────────────────────────────────────────────────────────────

fn open_zip(path: &Path) -> Result<ZipArchive<std::fs::File>, HtsError> {
    let file = std::fs::File::open(path)
        .map_err(|e| HtsError::InvalidRequest(format!("Cannot open {}: {e}", path.display())))?;
    ZipArchive::new(file)
        .map_err(|e| HtsError::InvalidRequest(format!("Not a valid ZIP archive: {e}")))
}

/// Locations of the CSV files HTS reads from a LOINC distribution ZIP.
#[derive(Debug, Default)]
struct LoincArchivePaths {
    loinc_table: String,
    hierarchy: String,
    /// `AccessoryFiles/LinguisticVariants/LinguisticVariants.csv`, if present.
    lv_index: Option<String>,
    /// Per-language `*LinguisticVariant.csv` files (excludes the index), sorted
    /// for deterministic processing order.
    lv_variants: Vec<String>,
    /// Parsed from a top-level `Loinc_<ver>/` directory in the ZIP, e.g. `2.82`.
    release_version: Option<String>,
}

/// Extract LOINC release version from a ZIP entry path such as
/// `Loinc_2.82/LoincTable/Loinc.csv`.
///
/// Stored on `CodeSystem.version` at import time so unpinned `$validate-code`
/// and `version=current` resolution prefer the full release over empty HL7 or
/// consult-minimal stub rows. See `docs/ig-publisher-compatibility.md`.
fn loinc_release_version_from_entry(entry_path: &str) -> Option<String> {
    for segment in entry_path.split('/') {
        let lower = segment.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("loinc_") {
            let ver = rest.trim();
            if !ver.is_empty()
                && ver.chars()
                    .all(|c| c.is_ascii_digit() || c == '.')
            {
                return Some(ver.to_string());
            }
        }
    }
    None
}

fn find_loinc_paths(path: &Path) -> Result<LoincArchivePaths, HtsError> {
    let mut zip = open_zip(path)?;

    let mut loinc_path: Option<String> = None;
    let mut hierarchy_path: Option<String> = None;
    let mut lv_index: Option<String> = None;
    let mut lv_variants: Vec<String> = Vec::new();
    let mut release_version: Option<String> = None;

    for i in 0..zip.len() {
        let entry = zip
            .by_index(i)
            .map_err(|e| HtsError::InvalidRequest(format!("ZIP entry error: {e}")))?;
        let name = entry.name().to_string();
        let lower = name.to_lowercase();
        let filename = lower.rsplit('/').next().unwrap_or(&lower);

        if filename.ends_with(".csv") {
            // The main table is always exactly `Loinc.csv` or `LoincTable.csv`
            // (flat or under `Loinc_<ver>/` or `LoincTable/`). Accessory files
            // like `LoincPartLink_Primary.csv`, `LoincParts.csv`, or
            // `LoincUniversalLabOrdersValueSet.csv` would be false positives
            // for a looser prefix match.
            if filename == "loinc.csv" || filename == "loinctable.csv" {
                if loinc_path.is_none() {
                    release_version = loinc_release_version_from_entry(&name);
                    loinc_path = Some(name);
                }
            } else if filename.contains("multiaxial") || filename.contains("componenthierarchy") {
                hierarchy_path = Some(name);
            } else if filename == "linguisticvariants.csv" {
                // The translation index (note trailing 's').
                lv_index = Some(name);
            } else if filename.ends_with("linguisticvariant.csv") {
                // A per-language translation file, e.g. `frFR28LinguisticVariant.csv`.
                lv_variants.push(name);
            }
        }
    }

    lv_variants.sort();

    Ok(LoincArchivePaths {
        loinc_table: loinc_path.ok_or_else(|| {
            HtsError::InvalidRequest(
                "No LoincTable CSV found. Expected a file whose name starts with 'loinc' or contains 'loinctable'.".into(),
            )
        })?,
        hierarchy: hierarchy_path.ok_or_else(|| {
            HtsError::InvalidRequest(
                "No hierarchy CSV found. Expected 'MultiAxialHierarchy.csv' (LOINC ≤ 2.73) or 'ComponentHierarchyBySystem.csv' (LOINC ≥ 2.74).".into(),
            )
        })?,
        lv_index,
        lv_variants,
        release_version,
    })
}

// ── Linguistic-variant helpers ─────────────────────────────────────────────────

/// Parse `LinguisticVariants.csv` into `variant_id → (iso_language, iso_country)`.
///
/// Columns: `ID, ISO_LANGUAGE, ISO_COUNTRY, LANGUAGE_NAME` (the documented
/// `LinguisticVariantId`/`IsoLanguage`/`IsoCountry` spellings are also
/// accepted). A missing or malformed index is non-fatal — callers fall back to
/// deriving the language tag from the per-variant filename.
fn parse_linguistic_index(
    reader: impl std::io::Read,
    errors: &mut Vec<String>,
) -> HashMap<String, (String, String)> {
    let mut rdr = csv::ReaderBuilder::new().flexible(true).from_reader(reader);

    let headers = match rdr.headers() {
        Ok(h) => h.clone(),
        Err(e) => {
            errors.push(format!("Cannot read LinguisticVariants.csv headers: {e}"));
            return HashMap::new();
        }
    };

    // Real LOINC releases name these columns `ID`, `ISO_LANGUAGE`,
    // `ISO_COUNTRY`; accept the documented `LinguisticVariantId`/`IsoLanguage`/
    // `IsoCountry` spellings too so either form parses.
    let find_any = |names: &[&str]| names.iter().find_map(|n| find_col(&headers, n).ok());
    let id_idx = find_any(&["ID", "LinguisticVariantId"]);
    let lang_idx = find_any(&["ISO_LANGUAGE", "IsoLanguage"]);
    let country_idx = find_any(&["ISO_COUNTRY", "IsoCountry"]);

    let (Some(id_idx), Some(lang_idx)) = (id_idx, lang_idx) else {
        errors.push(
            "LinguisticVariants.csv missing ID/ISO_LANGUAGE columns — \
             language tags will be derived from filenames"
                .into(),
        );
        return HashMap::new();
    };

    let mut map: HashMap<String, (String, String)> = HashMap::new();
    for result in rdr.records() {
        let Ok(record) = result else { continue };
        let id = record.get(id_idx).unwrap_or("").trim().to_string();
        if id.is_empty() {
            continue;
        }
        let lang = record.get(lang_idx).unwrap_or("").trim().to_lowercase();
        let country = country_idx
            .and_then(|i| record.get(i))
            .unwrap_or("")
            .trim()
            .to_uppercase();
        if !lang.is_empty() {
            map.insert(id, (lang, country));
        }
    }
    map
}

/// Derive the BCP-47 language tag for a per-variant file.
///
/// Prefers the index (matched by the numeric id embedded in the filename),
/// falling back to the `<lang><COUNTRY>` letters that prefix the filename.
/// `filename` is the lowercased basename, e.g. `frfr28linguisticvariant.csv`.
fn derive_language_tag(
    filename: &str,
    index: &HashMap<String, (String, String)>,
) -> Option<String> {
    let lower = filename.to_lowercase();
    let stem = lower.strip_suffix("linguisticvariant.csv")?;

    // The numeric id (if any) sits between the language/country letters and the
    // `LinguisticVariant` suffix, e.g. `frfr28` → "28".
    let id: String = stem.chars().filter(|c| c.is_ascii_digit()).collect();
    if !id.is_empty() {
        if let Some((lang, country)) = index.get(&id) {
            return Some(format_lang_tag(lang, country));
        }
    }

    // Fallback: leading letters encode `<lang><country>` (e.g. `frfr`).
    let letters: String = stem
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect();
    if letters.len() >= 2 {
        let lang = &letters[0..2];
        let country = if letters.len() >= 4 {
            &letters[2..4]
        } else {
            ""
        };
        return Some(format_lang_tag(lang, country));
    }
    None
}

fn format_lang_tag(lang: &str, country: &str) -> String {
    let lang = lang.to_lowercase();
    if country.is_empty() {
        lang
    } else {
        format!("{lang}-{}", country.to_uppercase())
    }
}

/// LOINC axis columns, in fully-specified-name order. Used to synthesize a
/// translated display when a linguistic-variant file leaves the combined-name
/// columns empty (e.g. the es-ES and it-IT translations only populate the
/// individual axes).
const LOINC_AXIS_COLUMNS: [&str; 6] = [
    "COMPONENT",
    "PROPERTY",
    "TIME_ASPCT",
    "SYSTEM",
    "SCALE_TYP",
    "METHOD_TYP",
];

/// Parse one `*LinguisticVariant.csv` into `(loinc_code, translated_display)`
/// rows. The translated display prefers `LONG_COMMON_NAME`, then `SHORTNAME`,
/// then `LinguisticVariantDisplayName`. When none of those combined-name
/// columns carry text (as in the es-ES and it-IT variants, which translate only
/// the individual axes), the display is synthesized as a LOINC fully-specified
/// name — the populated `COMPONENT:PROPERTY:TIME_ASPCT:SYSTEM:SCALE_TYP:METHOD_TYP`
/// axes joined with `:`. Rows without a code or any translated text are skipped.
fn parse_linguistic_variant(
    reader: impl std::io::Read,
    filename: &str,
    errors: &mut Vec<String>,
) -> Result<Vec<(String, String)>, HtsError> {
    let mut rdr = csv::ReaderBuilder::new().flexible(true).from_reader(reader);

    let headers = rdr
        .headers()
        .map_err(|e| HtsError::InvalidRequest(format!("Cannot read {filename} headers: {e}")))?;
    let headers = headers.clone();

    let code_idx = find_col(&headers, "LOINC_NUM")?;
    let long_idx = find_col(&headers, "LONG_COMMON_NAME").ok();
    let short_idx = find_col(&headers, "SHORTNAME").ok();
    let display_idx = find_col(&headers, "LinguisticVariantDisplayName").ok();
    let axis_idxs: Vec<usize> = LOINC_AXIS_COLUMNS
        .iter()
        .filter_map(|c| find_col(&headers, c).ok())
        .collect();

    if long_idx.is_none() && short_idx.is_none() && display_idx.is_none() && axis_idxs.is_empty() {
        errors.push(format!(
            "{filename}: no LONG_COMMON_NAME/SHORTNAME/LinguisticVariantDisplayName or axis \
             columns — no translations imported"
        ));
        return Ok(Vec::new());
    }

    let mut rows: Vec<(String, String)> = Vec::new();
    for result in rdr.records() {
        let record = result
            .map_err(|e| HtsError::InvalidRequest(format!("{filename} record error: {e}")))?;

        let code = record.get(code_idx).unwrap_or("").trim().to_string();
        if code.is_empty() {
            continue;
        }
        let pick = |idx: Option<usize>| {
            idx.and_then(|i| record.get(i))
                .map(str::trim)
                .filter(|s| !s.is_empty())
        };
        let value: String = match pick(long_idx)
            .or_else(|| pick(short_idx))
            .or_else(|| pick(display_idx))
        {
            Some(v) => v.to_string(),
            // No combined name — synthesize the LOINC fully-specified name from
            // the translated axis columns (e.g. es-ES, it-IT). Trailing empty
            // axes are dropped so a partly-populated row doesn't render dangling
            // `:` separators; interior gaps are kept to preserve axis position.
            None => {
                let mut axes: Vec<&str> = axis_idxs
                    .iter()
                    .map(|&i| record.get(i).unwrap_or("").trim())
                    .collect();
                while axes.last().is_some_and(|s| s.is_empty()) {
                    axes.pop();
                }
                axes.join(":")
            }
        };
        // A synthesized FSN of only empty axes collapses to ":::::" — treat
        // anything without an alphanumeric character as no translation.
        if !value.chars().any(|c| c.is_alphanumeric()) {
            continue;
        }
        rows.push((code, value));
    }
    Ok(rows)
}

/// Render a `" (fr-FR: 3, de-DE: 2)"`-style suffix for import logging, or an
/// empty string when there are no translations.
fn format_language_summary(by_language: &[(String, usize)]) -> String {
    if by_language.is_empty() {
        return String::new();
    }
    let parts: Vec<String> = by_language
        .iter()
        .map(|(lang, n)| format!("{lang}: {n}"))
        .collect();
    format!(" ({})", parts.join(", "))
}

// ── CSV parsers ───────────────────────────────────────────────────────────────

fn find_col(headers: &csv::StringRecord, name: &str) -> Result<usize, HtsError> {
    headers
        .iter()
        .position(|h| h.trim() == name)
        .ok_or_else(|| {
            HtsError::InvalidRequest(format!("Required column '{name}' not found in CSV headers"))
        })
}

fn parse_loinc_table(
    reader: impl std::io::Read,
    errors: &mut Vec<String>,
) -> Result<HashMap<String, LoincConcept>, HtsError> {
    let mut rdr = csv::ReaderBuilder::new().flexible(true).from_reader(reader);

    let headers = rdr
        .headers()
        .map_err(|e| HtsError::InvalidRequest(format!("Cannot read CSV headers: {e}")))?
        .clone();

    let code_idx = find_col(&headers, "LOINC_NUM")?;
    let long_idx = find_col(&headers, "LONG_COMMON_NAME").ok();
    let short_idx = find_col(&headers, "ShortName").ok();
    let status_idx = find_col(&headers, "STATUS").ok();

    let mut concepts: HashMap<String, LoincConcept> = HashMap::new();
    let mut record_no: usize = 0;

    for result in rdr.records() {
        record_no += 1;
        let record =
            result.map_err(|e| HtsError::InvalidRequest(format!("CSV record error: {e}")))?;

        let code = record.get(code_idx).unwrap_or("").trim().to_string();
        if code.is_empty() {
            errors.push(format!(
                "LoincTable record {record_no}: LOINC_NUM is empty — skipped"
            ));
            continue;
        }

        let display = long_idx
            .and_then(|i| record.get(i))
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .or_else(|| {
                short_idx
                    .and_then(|i| record.get(i))
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or("")
            .to_string();

        let status = status_idx
            .and_then(|i| record.get(i))
            .map(str::trim)
            .unwrap_or("ACTIVE")
            .to_string();

        concepts.insert(code, LoincConcept { display, status });
    }

    Ok(concepts)
}

type HierarchyResult = (HashMap<String, String>, Vec<(String, String)>);

fn parse_hierarchy(
    reader: impl std::io::Read,
    errors: &mut Vec<String>,
) -> Result<HierarchyResult, HtsError> {
    let mut rdr = csv::ReaderBuilder::new().flexible(true).from_reader(reader);

    let headers = rdr
        .headers()
        .map_err(|e| HtsError::InvalidRequest(format!("Cannot read hierarchy CSV headers: {e}")))?
        .clone();

    let code_idx = find_col(&headers, "CODE")?;
    let parent_idx = find_col(&headers, "IMMEDIATE_PARENT")?;
    let text_idx = find_col(&headers, "CODE_TEXT").ok();

    let mut concepts: HashMap<String, String> = HashMap::new();
    let mut edges: Vec<(String, String)> = Vec::new();
    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut record_no: usize = 0;

    for result in rdr.records() {
        record_no += 1;
        let record = result
            .map_err(|e| HtsError::InvalidRequest(format!("Hierarchy CSV record error: {e}")))?;

        let code = record.get(code_idx).unwrap_or("").trim().to_string();
        if code.is_empty() {
            errors.push(format!(
                "MultiAxialHierarchy record {record_no}: CODE is empty — skipped"
            ));
            continue;
        }

        let text = text_idx
            .and_then(|i| record.get(i))
            .unwrap_or("")
            .trim()
            .to_string();

        concepts.entry(code.clone()).or_insert(text);

        let parent = record.get(parent_idx).unwrap_or("").trim().to_string();
        if !parent.is_empty() {
            concepts.entry(parent.clone()).or_default();
            let edge = (code, parent);
            if seen.insert(edge.clone()) {
                edges.push(edge);
            }
        }
    }

    Ok((concepts, edges))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    use super::*;
    use crate::backends::SqliteTerminologyBackend;
    use std::io::Write;
    use tempfile::NamedTempFile;

    const LOINC_TABLE_CSV: &str = "\
LOINC_NUM,LONG_COMMON_NAME,ShortName,STATUS\r\n\
2160-0,Creatinine [Mass/volume] in Serum or Plasma,Creat SerPl-mCnc,ACTIVE\r\n\
718-7,Hemoglobin [Mass/volume] in Blood,Hgb Bld-mCnc,ACTIVE\r\n\
99999-9,Old deprecated test,Old test,DEPRECATED\r\n";

    const HIERARCHY_CSV: &str = "\
PATH_TO_ROOT,SEQUENCE,IMMEDIATE_PARENT,CODE,CODE_TEXT\r\n\
LP7786-3,1,,LP7786-3,Laboratory\r\n\
LP7786-3.LP29693-6,2,LP7786-3,LP29693-6,Chemistry\r\n\
LP7786-3.LP29693-6.2160-0,3,LP29693-6,2160-0,Creatinine\r\n\
LP7786-3.LP10156-0,2,LP7786-3,LP10156-0,Hematology\r\n\
LP7786-3.LP10156-0.718-7,3,LP10156-0,718-7,Hemoglobin\r\n";

    fn make_test_loinc_zip() -> NamedTempFile {
        let tmp = NamedTempFile::with_suffix(".zip").unwrap();
        {
            let mut zip = zip::ZipWriter::new(tmp.reopen().unwrap());
            let opts = zip::write::FileOptions::default();

            zip.start_file("LoincTable.csv", opts).unwrap();
            zip.write_all(LOINC_TABLE_CSV.as_bytes()).unwrap();

            zip.start_file("MultiAxialHierarchy.csv", opts).unwrap();
            zip.write_all(HIERARCHY_CSV.as_bytes()).unwrap();

            zip.finish().unwrap();
        }
        tmp
    }

    // Linguistic-variant fixtures. The index maps numeric ids → language/country;
    // the per-variant files carry translated LONG_COMMON_NAME values.
    // Uses the real LOINC release column names (`ID`, `ISO_LANGUAGE`, …); the
    // documented `LinguisticVariantId`/`IsoLanguage` spellings are accepted too.
    const LV_INDEX_CSV: &str = "\
ID,ISO_LANGUAGE,ISO_COUNTRY,LANGUAGE_NAME,PRODUCER\r\n\
28,fr,FR,French (France),Some Producer\r\n\
8,de,DE,German (Germany),Some Producer\r\n";

    // es-ES / it-IT style: only the individual axes are translated; the
    // combined-name columns (LONG_COMMON_NAME/SHORTNAME/LinguisticVariantDisplayName)
    // are empty. The display must be synthesized from the axes.
    const LV_ES_AXES_CSV: &str = "\
LOINC_NUM,COMPONENT,PROPERTY,TIME_ASPCT,SYSTEM,SCALE_TYP,METHOD_TYP,CLASS,SHORTNAME,LONG_COMMON_NAME,RELATEDNAMES2,LinguisticVariantDisplayName\r\n\
2160-0,Creatinina,MCnc,Punto temporal,Suero o Plasma,Cuant,,,,,,\r\n\
718-7,Hemoglobina,MCnc,Punto temporal,Sangre,Cuant,,,,,,\r\n";

    const LV_FR_CSV: &str = "\
LOINC_NUM,COMPONENT,LONG_COMMON_NAME,SHORTNAME\r\n\
2160-0,Créatinine,Créatinine [Masse/volume] dans le sérum ou le plasma,Créat SerPl\r\n\
718-7,Hémoglobine,Hémoglobine [Masse/volume] dans le sang,Hb Sang\r\n";

    const LV_DE_CSV: &str = "\
LOINC_NUM,COMPONENT,LONG_COMMON_NAME,SHORTNAME\r\n\
2160-0,Kreatinin,Kreatinin [Masse/Volumen] in Serum oder Plasma,Krea SerPl\r\n";

    fn make_test_loinc_zip_with_variants() -> NamedTempFile {
        let tmp = NamedTempFile::with_suffix(".zip").unwrap();
        {
            let mut zip = zip::ZipWriter::new(tmp.reopen().unwrap());
            let opts = zip::write::FileOptions::default();

            zip.start_file("LoincTable/Loinc.csv", opts).unwrap();
            zip.write_all(LOINC_TABLE_CSV.as_bytes()).unwrap();

            zip.start_file(
                "AccessoryFiles/MultiAxialHierarchy/MultiAxialHierarchy.csv",
                opts,
            )
            .unwrap();
            zip.write_all(HIERARCHY_CSV.as_bytes()).unwrap();

            zip.start_file(
                "AccessoryFiles/LinguisticVariants/LinguisticVariants.csv",
                opts,
            )
            .unwrap();
            zip.write_all(LV_INDEX_CSV.as_bytes()).unwrap();

            zip.start_file(
                "AccessoryFiles/LinguisticVariants/frFR28LinguisticVariant.csv",
                opts,
            )
            .unwrap();
            zip.write_all(LV_FR_CSV.as_bytes()).unwrap();

            zip.start_file(
                "AccessoryFiles/LinguisticVariants/deDE8LinguisticVariant.csv",
                opts,
            )
            .unwrap();
            zip.write_all(LV_DE_CSV.as_bytes()).unwrap();

            zip.finish().unwrap();
        }
        tmp
    }

    /// Build a ZIP carrying the regional variant set from the user-reported
    /// case: de-AT, de-DE, es-AR, es-ES, es-MX. The numeric ids (15/91/92/93)
    /// are absent from `LV_INDEX_CSV` so `derive_language_tag` resolves those
    /// tags from the filename letters (id 8 maps to de-DE via the index).
    fn make_test_loinc_zip_with_regional_variants() -> NamedTempFile {
        let tmp = NamedTempFile::with_suffix(".zip").unwrap();
        {
            let mut zip = zip::ZipWriter::new(tmp.reopen().unwrap());
            let opts = zip::write::FileOptions::default();

            zip.start_file("LoincTable/Loinc.csv", opts).unwrap();
            zip.write_all(LOINC_TABLE_CSV.as_bytes()).unwrap();
            zip.start_file(
                "AccessoryFiles/MultiAxialHierarchy/MultiAxialHierarchy.csv",
                opts,
            )
            .unwrap();
            zip.write_all(HIERARCHY_CSV.as_bytes()).unwrap();
            zip.start_file(
                "AccessoryFiles/LinguisticVariants/LinguisticVariants.csv",
                opts,
            )
            .unwrap();
            zip.write_all(LV_INDEX_CSV.as_bytes()).unwrap();

            // German variants (LONG_COMMON_NAME-style, 1 row each): de-AT, de-DE.
            for fname in ["deAT15LinguisticVariant.csv", "deDE8LinguisticVariant.csv"] {
                zip.start_file(format!("AccessoryFiles/LinguisticVariants/{fname}"), opts)
                    .unwrap();
                zip.write_all(LV_DE_CSV.as_bytes()).unwrap();
            }
            // Spanish variants (axes-synthesized, 2 rows each): es-AR, es-ES, es-MX.
            for fname in [
                "esAR92LinguisticVariant.csv",
                "esES91LinguisticVariant.csv",
                "esMX93LinguisticVariant.csv",
            ] {
                zip.start_file(format!("AccessoryFiles/LinguisticVariants/{fname}"), opts)
                    .unwrap();
                zip.write_all(LV_ES_AXES_CSV.as_bytes()).unwrap();
            }

            zip.finish().unwrap();
        }
        tmp
    }

    fn count_rows(backend: &SqliteTerminologyBackend, table: &str) -> i64 {
        let conn = backend.pool().get().unwrap();
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap_or(0)
    }

    // ── Parser unit tests ─────────────────────────────────────────────────────

    #[test]
    fn parse_loinc_table_returns_all_statuses() {
        let mut errors = Vec::new();
        let concepts = parse_loinc_table(LOINC_TABLE_CSV.as_bytes(), &mut errors).unwrap();
        assert_eq!(concepts.len(), 3, "all three rows should be parsed");
        assert_eq!(
            concepts["2160-0"].display,
            "Creatinine [Mass/volume] in Serum or Plasma"
        );
        assert_eq!(concepts["2160-0"].status, "ACTIVE");
        assert_eq!(concepts["99999-9"].status, "DEPRECATED");
        assert!(errors.is_empty());
    }

    #[test]
    fn parse_loinc_table_falls_back_to_short_name() {
        let csv = "LOINC_NUM,LONG_COMMON_NAME,ShortName,STATUS\r\n\
                   1234-5,,Short only,ACTIVE\r\n";
        let mut errors = Vec::new();
        let concepts = parse_loinc_table(csv.as_bytes(), &mut errors).unwrap();
        assert_eq!(concepts["1234-5"].display, "Short only");
    }

    #[test]
    fn parse_hierarchy_returns_lp_codes_and_edges() {
        let mut errors = Vec::new();
        let (concepts, edges) = parse_hierarchy(HIERARCHY_CSV.as_bytes(), &mut errors).unwrap();
        assert_eq!(concepts.len(), 5);
        assert_eq!(concepts["LP7786-3"], "Laboratory");
        assert_eq!(edges.len(), 4);
    }

    #[test]
    fn parse_loinc_empty_code_recorded_in_errors() {
        let csv = "LOINC_NUM,LONG_COMMON_NAME,ShortName,STATUS\r\n\
                   2160-0,Creatinine,Creat,ACTIVE\r\n\
                   ,Missing code,Bad,ACTIVE\r\n";
        let mut errors = Vec::new();
        let concepts = parse_loinc_table(csv.as_bytes(), &mut errors).unwrap();
        assert_eq!(concepts.len(), 1);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("LOINC_NUM is empty"));
    }

    #[tokio::test]
    async fn deprecated_status_stored_in_definition() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();
        let zip_file = make_test_loinc_zip();

        import_loinc_csv(
            &backend,
            &ctx,
            zip_file.path(),
            500,
            false,
            &LanguageFilter::default(),
        )
        .await
        .unwrap();

        let conn = backend.pool().get().unwrap();
        let definition: Option<String> = conn
            .query_row(
                "SELECT definition FROM concepts WHERE code = '99999-9'",
                [],
                |row| row.get(0),
            )
            .ok();
        assert_eq!(definition.as_deref(), Some("STATUS:DEPRECATED"));
    }

    #[tokio::test]
    async fn import_loinc_csv_dry_run_does_not_write_to_db() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();
        let zip_file = make_test_loinc_zip();

        let stats = import_loinc_csv(
            &backend,
            &ctx,
            zip_file.path(),
            500,
            true,
            &LanguageFilter::default(),
        )
        .await
        .expect("dry-run should succeed");

        assert_eq!(stats.code_systems, 1);
        assert!(stats.concepts > 0);

        assert_eq!(count_rows(&backend, "code_systems"), 0);
        assert_eq!(count_rows(&backend, "concepts"), 0);
    }

    #[tokio::test]
    async fn import_loinc_csv_live_writes_concepts_and_hierarchy() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();
        let zip_file = make_test_loinc_zip();

        let stats = import_loinc_csv(
            &backend,
            &ctx,
            zip_file.path(),
            500,
            false,
            &LanguageFilter::default(),
        )
        .await
        .expect("live import should succeed");

        assert_eq!(stats.code_systems, 1);
        // 3 LOINC codes + 3 LP codes = 6 total concepts
        assert_eq!(stats.concepts, 6);

        assert_eq!(count_rows(&backend, "code_systems"), 1);
        assert_eq!(count_rows(&backend, "concepts"), 6);
        assert_eq!(count_rows(&backend, "concept_hierarchy"), 4);
    }

    #[tokio::test]
    async fn import_loinc_csv_idempotent_reimport() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();
        let zip_file = make_test_loinc_zip();

        import_loinc_csv(
            &backend,
            &ctx,
            zip_file.path(),
            500,
            false,
            &LanguageFilter::default(),
        )
        .await
        .unwrap();
        import_loinc_csv(
            &backend,
            &ctx,
            zip_file.path(),
            500,
            false,
            &LanguageFilter::default(),
        )
        .await
        .unwrap();

        assert_eq!(count_rows(&backend, "code_systems"), 1);
        assert_eq!(count_rows(&backend, "concepts"), 6);
        assert_eq!(count_rows(&backend, "concept_hierarchy"), 4);
    }

    #[tokio::test]
    async fn import_loinc_csv_batching_preserves_all_concepts() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();
        let zip_file = make_test_loinc_zip();

        let stats = import_loinc_csv(
            &backend,
            &ctx,
            zip_file.path(),
            2,
            false,
            &LanguageFilter::default(),
        )
        .await
        .unwrap();
        assert_eq!(stats.concepts, 6);
        assert_eq!(count_rows(&backend, "concepts"), 6);
    }

    #[tokio::test]
    async fn import_loinc_csv_missing_file_returns_error() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();

        let result = import_loinc_csv(
            &backend,
            &ctx,
            Path::new("/nonexistent/loinc.zip"),
            500,
            false,
            &LanguageFilter::default(),
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn import_loinc_ignores_accessory_loinc_files() {
        // Reproduces tx-ecosystem subset layout: real table at
        // `LoincTable/Loinc.csv`, with `AccessoryFiles/PartFile/LoincPartLink_Primary.csv`
        // alongside. The accessory file's name starts with "loinc" but isn't
        // the main table — picking it would fail with "LOINC_NUM not found".
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();

        let tmp = NamedTempFile::with_suffix(".zip").unwrap();
        {
            let mut zip = zip::ZipWriter::new(tmp.reopen().unwrap());
            let opts = zip::write::FileOptions::default();

            zip.start_file("AccessoryFiles/PartFile/LoincPartLink_Primary.csv", opts)
                .unwrap();
            zip.write_all(b"LinkTypeName,LoincNumber,PartNumber\r\n")
                .unwrap();

            zip.start_file("LoincTable/Loinc.csv", opts).unwrap();
            zip.write_all(LOINC_TABLE_CSV.as_bytes()).unwrap();

            zip.start_file(
                "AccessoryFiles/ComponentHierarchyBySystem/ComponentHierarchyBySystem.csv",
                opts,
            )
            .unwrap();
            zip.write_all(HIERARCHY_CSV.as_bytes()).unwrap();

            zip.finish().unwrap();
        }

        let stats = import_loinc_csv(
            &backend,
            &ctx,
            tmp.path(),
            500,
            false,
            &LanguageFilter::default(),
        )
        .await
        .expect("should pick LoincTable/Loinc.csv, not the accessory file");
        assert_eq!(stats.concepts, 6);
        assert_eq!(count_rows(&backend, "concepts"), 6);
    }

    // ── Linguistic-variant tests ──────────────────────────────────────────────

    #[test]
    fn parse_linguistic_index_maps_ids_to_lang_country() {
        let mut errors = Vec::new();
        let index = parse_linguistic_index(LV_INDEX_CSV.as_bytes(), &mut errors);
        assert_eq!(index["28"], ("fr".to_string(), "FR".to_string()));
        assert_eq!(index["8"], ("de".to_string(), "DE".to_string()));
        assert!(errors.is_empty());
    }

    #[test]
    fn derive_language_tag_prefers_index() {
        let mut errors = Vec::new();
        let index = parse_linguistic_index(LV_INDEX_CSV.as_bytes(), &mut errors);
        assert_eq!(
            derive_language_tag("frFR28LinguisticVariant.csv", &index).as_deref(),
            Some("fr-FR")
        );
        assert_eq!(
            derive_language_tag("deDE8LinguisticVariant.csv", &index).as_deref(),
            Some("de-DE")
        );
    }

    #[test]
    fn derive_language_tag_falls_back_to_filename() {
        let empty = HashMap::new();
        // No index → derive `zh-CN` from the leading letters.
        assert_eq!(
            derive_language_tag("zhCN5LinguisticVariant.csv", &empty).as_deref(),
            Some("zh-CN")
        );
        // Language only, no country letters.
        assert_eq!(
            derive_language_tag("fr99LinguisticVariant.csv", &empty).as_deref(),
            Some("fr")
        );
        // Not a variant file.
        assert_eq!(derive_language_tag("LoincTable.csv", &empty), None);
    }

    #[test]
    fn parse_linguistic_variant_extracts_translations() {
        let mut errors = Vec::new();
        let rows = parse_linguistic_variant(
            LV_FR_CSV.as_bytes(),
            "frFR28LinguisticVariant.csv",
            &mut errors,
        )
        .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, "2160-0");
        assert_eq!(
            rows[0].1,
            "Créatinine [Masse/volume] dans le sérum ou le plasma"
        );
        assert!(errors.is_empty());
    }

    #[test]
    fn parse_linguistic_variant_synthesizes_display_from_axes() {
        // es-ES / it-IT only populate the axis columns; the display must be the
        // fully-specified name built from the populated axes (regression: these
        // variants used to import zero designations).
        let mut errors = Vec::new();
        let rows = parse_linguistic_variant(
            LV_ES_AXES_CSV.as_bytes(),
            "esES12LinguisticVariant.csv",
            &mut errors,
        )
        .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, "2160-0");
        // Trailing empty axis (METHOD_TYP) is dropped — no dangling separator.
        assert_eq!(
            rows[0].1,
            "Creatinina:MCnc:Punto temporal:Suero o Plasma:Cuant"
        );
        assert!(errors.is_empty());
    }

    #[test]
    fn parse_linguistic_index_accepts_real_loinc_columns() {
        // Real LOINC releases use ID/ISO_LANGUAGE/ISO_COUNTRY rather than the
        // documented camelCase spellings — both must parse without error.
        let mut errors = Vec::new();
        let index = parse_linguistic_index(LV_INDEX_CSV.as_bytes(), &mut errors);
        assert_eq!(index["28"], ("fr".to_string(), "FR".to_string()));
        assert!(errors.is_empty());
    }

    #[test]
    fn find_loinc_paths_locates_linguistic_variants() {
        let zip_file = make_test_loinc_zip_with_variants();
        let paths = find_loinc_paths(zip_file.path()).unwrap();
        assert!(paths.lv_index.is_some());
        assert_eq!(paths.lv_variants.len(), 2);
        // The index (`LinguisticVariants.csv`, plural) must not be collected as
        // a per-variant file.
        assert!(
            paths
                .lv_variants
                .iter()
                .all(|p| !p.to_lowercase().ends_with("linguisticvariants.csv"))
        );
    }

    #[tokio::test]
    async fn import_loinc_csv_writes_designations() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();
        let zip_file = make_test_loinc_zip_with_variants();

        import_loinc_csv(
            &backend,
            &ctx,
            zip_file.path(),
            500,
            false,
            &LanguageFilter::default(),
        )
        .await
        .expect("import with linguistic variants should succeed");

        // 2 French + 1 German translation = 3 designation rows.
        assert_eq!(count_rows(&backend, "concept_designations"), 3);

        let conn = backend.pool().get().unwrap();
        let (lang, value): (String, String) = conn
            .query_row(
                "SELECT d.language, d.value FROM concept_designations d \
                 JOIN concepts c ON c.id = d.concept_id \
                 WHERE c.code = '2160-0' AND d.language = 'de-DE'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("German designation for 2160-0 should exist");
        assert_eq!(lang, "de-DE");
        assert_eq!(value, "Kreatinin [Masse/Volumen] in Serum oder Plasma");

        // The English display is left untouched.
        let display: String = conn
            .query_row(
                "SELECT display FROM concepts WHERE code = '2160-0'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(display, "Creatinine [Mass/volume] in Serum or Plasma");
    }

    #[tokio::test]
    async fn import_loinc_csv_language_filter_skips_excluded_variants() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();
        let zip_file = make_test_loinc_zip_with_variants();

        import_loinc_csv(
            &backend,
            &ctx,
            zip_file.path(),
            500,
            false,
            &LanguageFilter::parse("fr"),
        )
        .await
        .expect("import with language filter should succeed");

        // Only the 2 French translations survive; the German variant file is
        // skipped without being parsed.
        assert_eq!(count_rows(&backend, "concept_designations"), 2);

        let conn = backend.pool().get().unwrap();
        let de_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM concept_designations WHERE language = 'de-DE'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(de_count, 0);

        // The English display is unaffected by the filter.
        let display: String = conn
            .query_row(
                "SELECT display FROM concepts WHERE code = '2160-0'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(display, "Creatinine [Mass/volume] in Serum or Plasma");
    }

    #[tokio::test]
    async fn import_loinc_best_tier_excludes_regional_siblings() {
        // The user-reported case: filter `de,es-ES` against de-AT, de-DE,
        // es-AR, es-ES, es-MX. `es-ES` is an exact match, so es-AR/es-MX are
        // dropped; `de` only reaches the regional-sibling tier, so both German
        // variants are kept.
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();
        let zip_file = make_test_loinc_zip_with_regional_variants();

        import_loinc_csv(
            &backend,
            &ctx,
            zip_file.path(),
            500,
            false,
            &LanguageFilter::parse("de,es-ES"),
        )
        .await
        .expect("import with language filter should succeed");

        let conn = backend.pool().get().unwrap();
        let count = |lang: &str| -> i64 {
            conn.query_row(
                "SELECT COUNT(*) FROM concept_designations WHERE language = ?1",
                [lang],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(count("de-AT"), 1, "de-AT kept (de* sibling tier)");
        assert_eq!(count("de-DE"), 1, "de-DE kept (de* sibling tier)");
        assert_eq!(count("es-ES"), 2, "es-ES kept (exact match)");
        assert_eq!(count("es-AR"), 0, "es-AR dropped (es-ES exact outranks)");
        assert_eq!(count("es-MX"), 0, "es-MX dropped (es-ES exact outranks)");
        // 2 German + 2 Spanish designations, nothing else.
        assert_eq!(count_rows(&backend, "concept_designations"), 4);
    }

    #[tokio::test]
    async fn loinc_fresh_load_skip_then_narrowed_reimport_replaces() {
        // First import is a fresh load (system empty → fresh_load path skips the
        // delete-before-reinsert). Both French and German designations land.
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();
        let zip_file = make_test_loinc_zip_with_variants();
        import_loinc_csv(
            &backend,
            &ctx,
            zip_file.path(),
            500,
            false,
            &LanguageFilter::default(),
        )
        .await
        .unwrap();
        assert_eq!(count_rows(&backend, "concept_designations"), 3); // fr×2 + de×1

        // Re-import with a narrower filter. The system now has concepts, so the
        // probe must flip fresh_load OFF and the per-concept replacement must
        // run — dropping the German designation. This is the interaction that
        // would silently leave stale rows if the skip path were taken on a
        // re-import.
        import_loinc_csv(
            &backend,
            &ctx,
            zip_file.path(),
            500,
            false,
            &LanguageFilter::parse("fr"),
        )
        .await
        .unwrap();
        let conn = backend.pool().get().unwrap();
        let de: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM concept_designations WHERE language = 'de-DE'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            de, 0,
            "stale German designations must be removed on re-import"
        );
        assert_eq!(count_rows(&backend, "concept_designations"), 2); // fr×2 only
    }

    #[tokio::test]
    async fn import_loinc_csv_dry_run_counts_designations() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();
        let zip_file = make_test_loinc_zip_with_variants();

        import_loinc_csv(
            &backend,
            &ctx,
            zip_file.path(),
            500,
            true,
            &LanguageFilter::default(),
        )
        .await
        .expect("dry-run should succeed");

        // Dry-run writes nothing.
        assert_eq!(count_rows(&backend, "concept_designations"), 0);
    }

    #[tokio::test]
    async fn import_loinc_csv_without_variants_still_works() {
        // The original (no-variant) ZIP must import cleanly with zero designations.
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();
        let zip_file = make_test_loinc_zip();

        import_loinc_csv(
            &backend,
            &ctx,
            zip_file.path(),
            500,
            false,
            &LanguageFilter::default(),
        )
        .await
        .unwrap();
        assert_eq!(count_rows(&backend, "concept_designations"), 0);
    }

    #[test]
    fn loinc_release_version_from_entry_parses_top_level_folder() {
        assert_eq!(
            super::loinc_release_version_from_entry("Loinc_2.82/LoincTable/Loinc.csv").as_deref(),
            Some("2.82")
        );
        assert_eq!(
            super::loinc_release_version_from_entry("LoincTable/Loinc.csv"),
            None
        );
    }

    #[tokio::test]
    async fn import_loinc_nested_zip_layout() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();

        let tmp = NamedTempFile::with_suffix(".zip").unwrap();
        {
            let mut zip = zip::ZipWriter::new(tmp.reopen().unwrap());
            let opts = zip::write::FileOptions::default();

            zip.start_file("Loinc_2.77/LoincTable.csv", opts).unwrap();
            zip.write_all(LOINC_TABLE_CSV.as_bytes()).unwrap();

            zip.start_file(
                "Loinc_2.77/AccessoryFiles/MultiAxialHierarchy/MultiAxialHierarchy.csv",
                opts,
            )
            .unwrap();
            zip.write_all(HIERARCHY_CSV.as_bytes()).unwrap();

            zip.finish().unwrap();
        }

        let stats = import_loinc_csv(
            &backend,
            &ctx,
            tmp.path(),
            500,
            false,
            &LanguageFilter::default(),
        )
        .await
        .unwrap();

        assert!(stats.concepts > 0, "got 0 concepts");
        assert!(
            stats.errors.is_empty(),
            "unexpected errors: {:?}",
            stats.errors
        );

        let conn = backend.pool().get().unwrap();
        let version: Option<String> = conn
            .query_row(
                "SELECT version FROM code_systems WHERE url = 'http://loinc.org'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version.as_deref(), Some("2.77"));
    }
}
