//! SNOMED CT RF2 importer.
//!
//! Reads a SNOMED CT RF2 distribution ZIP and imports active concepts,
//! preferred terms, and `Is-a` hierarchy edges into the HTS normalized schema.
//!
//! # ⚠️  LICENSE REQUIRED
//!
//! Real SNOMED CT data requires a license from SNOMED International.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::path::Path;

use helios_persistence::tenant::TenantContext;
use zip::ZipArchive;

use crate::error::HtsError;
use crate::import::BundleImportBackend;
use crate::import::ImportStats;
use crate::import::bundle_builder::{
    BuilderConcept, BuilderProperty, CodeSystemMeta, build_code_system_bundle,
};

// ── SNOMED CT constants ───────────────────────────────────────────────────────

const SNOMED_URL: &str = "http://snomed.info/sct";
const SNOMED_ID: &str = "snomed-ct";
const SNOMED_NAME: &str = "SNOMED_CT";
const SNOMED_TITLE: &str = "SNOMED CT";

const TYPE_FSN: &str = "900000000000003001";
const TYPE_SYNONYM: &str = "900000000000013009";
const IS_A_TYPE: &str = "116680003";

// ── Public entry point ────────────────────────────────────────────────────────

#[derive(Debug)]
struct SnomedParseResult {
    /// concept id → display term.
    preferred_terms: HashMap<String, String>,
    /// (child, parent) is-a edges.
    is_a_edges: Vec<(String, String)>,
    release_version: Option<String>,
    parse_errors: Vec<String>,
}

/// Import a SNOMED CT RF2 distribution ZIP through the given backend.
pub async fn import_snomed_rf2(
    backend: &dyn BundleImportBackend,
    ctx: &TenantContext,
    path: &Path,
    batch_size: usize,
    dry_run: bool,
) -> Result<ImportStats, HtsError> {
    const FORMAT: &str = "snomed-rf2";
    let batch_size = batch_size.max(1);

    let path_owned = path.to_path_buf();
    let parsed = tokio::task::spawn_blocking(move || -> Result<SnomedParseResult, HtsError> {
        let (concept_path, desc_path, rel_path) = find_rf2_paths(&path_owned)?;

        tracing::info!(
            concept_file = %concept_path,
            description_file = %desc_path,
            relationship_file = %rel_path,
            "Located RF2 files in archive"
        );

        let mut parse_errors: Vec<String> = Vec::new();

        let active_concepts = {
            let mut zip = open_zip(&path_owned)?;
            let entry = zip
                .by_name(&concept_path)
                .map_err(|e| HtsError::InvalidRequest(format!("Cannot open concept file: {e}")))?;
            parse_active_concepts(BufReader::new(entry), &mut parse_errors)
        };

        let preferred_terms = {
            let mut zip = open_zip(&path_owned)?;
            let entry = zip.by_name(&desc_path).map_err(|e| {
                HtsError::InvalidRequest(format!("Cannot open description file: {e}"))
            })?;
            parse_preferred_terms(BufReader::new(entry), &active_concepts, &mut parse_errors)
        };

        let is_a_edges = {
            let mut zip = open_zip(&path_owned)?;
            let entry = zip.by_name(&rel_path).map_err(|e| {
                HtsError::InvalidRequest(format!("Cannot open relationship file: {e}"))
            })?;
            parse_is_a_edges(BufReader::new(entry), &active_concepts, &mut parse_errors)
        };

        let release_version = extract_release_date(&concept_path);

        Ok(SnomedParseResult {
            preferred_terms,
            is_a_edges,
            release_version,
            parse_errors,
        })
    })
    .await
    .map_err(|e| HtsError::Internal(format!("SNOMED parser panicked: {e}")))??;

    let SnomedParseResult {
        preferred_terms,
        is_a_edges,
        release_version,
        parse_errors,
    } = parsed;

    let concept_count = preferred_terms.len() as u32;
    let edge_count = is_a_edges.len();

    let mut stats = ImportStats {
        code_systems: 1,
        errors: parse_errors,
        ..Default::default()
    };

    if dry_run {
        stats.concepts = concept_count;
        eprintln!(
            "[{FORMAT}] dry-run — would import {concept_count} concepts, {edge_count} Is-a edges"
        );
        return Ok(stats);
    }

    // Build child → parents map.
    let mut parents_of: HashMap<String, Vec<String>> = HashMap::new();
    for (child, parent) in &is_a_edges {
        parents_of
            .entry(child.clone())
            .or_default()
            .push(parent.clone());
    }

    let meta_version = release_version.clone().unwrap_or_else(|| "current".into());
    let meta = CodeSystemMeta {
        id: SNOMED_ID,
        url: SNOMED_URL,
        version: Some(&meta_version),
        name: Some(SNOMED_NAME),
        title: Some(SNOMED_TITLE),
        status: "active",
        content: "complete",
    };

    // Seed empty CodeSystem.
    let seed = build_code_system_bundle(&meta, &[]);
    let seed_stats = backend.import_bundle(ctx, &seed).await?;
    stats.code_systems = seed_stats.code_systems;
    stats.errors.extend(seed_stats.errors);

    let concept_list: Vec<(String, String)> = preferred_terms.into_iter().collect();
    let total = concept_list.len();
    let total_batches = total.div_ceil(batch_size).max(1);

    for (i, chunk) in concept_list.chunks(batch_size).enumerate() {
        let extras_per: Vec<Vec<BuilderProperty<'_>>> = chunk
            .iter()
            .map(|(code, _)| {
                parents_of
                    .get(code)
                    .map(|parents| {
                        parents
                            .iter()
                            .skip(1)
                            .map(|p| BuilderProperty {
                                code: "parent",
                                value_key: "valueCode",
                                value: p.as_str(),
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            })
            .collect();

        let builder: Vec<BuilderConcept<'_>> = chunk
            .iter()
            .enumerate()
            .map(|(idx, (code, display))| BuilderConcept {
                code: code.as_str(),
                display: Some(display.as_str()).filter(|s| !s.is_empty()),
                parent_code: parents_of
                    .get(code)
                    .and_then(|p| p.first().map(|s| s.as_str())),
                extra_properties: extras_per[idx].as_slice(),
                ..Default::default()
            })
            .collect();

        let bytes = build_code_system_bundle(&meta, &builder);
        let chunk_stats = backend.import_bundle(ctx, &bytes).await?;
        stats.errors.extend(chunk_stats.errors);
        stats.concepts += chunk.len() as u32;

        eprintln!(
            "[{FORMAT}] concept batch {}/{total_batches} — +{} concepts (total: {})",
            i + 1,
            chunk.len(),
            stats.concepts
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

fn find_rf2_paths(path: &Path) -> Result<(String, String, String), HtsError> {
    let mut zip = open_zip(path)?;

    let mut concept_path: Option<String> = None;
    let mut desc_path: Option<String> = None;
    let mut rel_path: Option<String> = None;

    for i in 0..zip.len() {
        let entry = zip
            .by_index(i)
            .map_err(|e| HtsError::InvalidRequest(format!("ZIP entry error: {e}")))?;
        let name = entry.name().to_string();

        if !name.ends_with(".txt") {
            continue;
        }
        let lower = name.to_lowercase();
        if lower.contains("refset") {
            continue;
        }

        if lower.contains("concept_") {
            concept_path = Some(name);
        } else if lower.contains("description_") {
            desc_path = Some(name);
        } else if lower.contains("relationship_") && !lower.contains("statedrelationship") {
            rel_path = Some(name);
        }
    }

    Ok((
        concept_path.ok_or_else(|| {
            HtsError::InvalidRequest(
                "No Concept RF2 file found. Expected a file containing 'Concept_' in its path.".into(),
            )
        })?,
        desc_path.ok_or_else(|| {
            HtsError::InvalidRequest(
                "No Description RF2 file found. Expected a file containing 'Description_' in its path.".into(),
            )
        })?,
        rel_path.ok_or_else(|| {
            HtsError::InvalidRequest(
                "No Relationship RF2 file found. Expected a file containing 'Relationship_' in its path.".into(),
            )
        })?,
    ))
}

// ── RF2 parsers ───────────────────────────────────────────────────────────────

fn parse_active_concepts(reader: impl BufRead, errors: &mut Vec<String>) -> HashSet<String> {
    let mut active = HashSet::new();

    for (line_num, line_result) in reader.lines().enumerate() {
        let line = match line_result {
            Ok(l) => l,
            Err(_) => continue,
        };
        if line_num == 0 || line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.splitn(6, '\t').collect();
        if parts.len() < 3 {
            errors.push(format!(
                "Concept RF2 line {}: expected ≥3 fields, got {} — skipped",
                line_num + 1,
                parts.len()
            ));
            continue;
        }

        let id = parts[0].trim().to_string();
        let is_active = parts[2].trim() == "1";

        if is_active {
            active.insert(id);
        } else {
            active.remove(&id);
        }
    }
    active
}

fn parse_preferred_terms(
    reader: impl BufRead,
    active_concepts: &HashSet<String>,
    errors: &mut Vec<String>,
) -> HashMap<String, String> {
    let mut synonyms: HashMap<String, String> = HashMap::new();
    let mut fsns: HashMap<String, String> = HashMap::new();

    for (line_num, line_result) in reader.lines().enumerate() {
        let line = match line_result {
            Ok(l) => l,
            Err(_) => continue,
        };
        if line_num == 0 || line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.splitn(10, '\t').collect();
        if parts.len() < 9 {
            errors.push(format!(
                "Description RF2 line {}: expected ≥9 fields, got {} — skipped",
                line_num + 1,
                parts.len()
            ));
            continue;
        }

        let active = parts[2].trim() == "1";
        let concept_id = parts[4].trim();
        let language = parts[5].trim();
        let type_id = parts[6].trim();
        let term = parts[7].trim();

        if !active || language != "en" || !active_concepts.contains(concept_id) {
            continue;
        }

        match type_id {
            TYPE_SYNONYM => {
                synonyms
                    .entry(concept_id.to_string())
                    .or_insert_with(|| term.to_string());
            }
            TYPE_FSN => {
                fsns.entry(concept_id.to_string())
                    .or_insert_with(|| term.to_string());
            }
            _ => {}
        }
    }

    let mut terms: HashMap<String, String> = synonyms;
    for concept_id in active_concepts {
        if !terms.contains_key(concept_id) {
            if let Some(fsn) = fsns.get(concept_id) {
                terms.insert(concept_id.clone(), fsn.clone());
            } else {
                terms.entry(concept_id.clone()).or_default();
            }
        }
    }
    terms
}

fn parse_is_a_edges(
    reader: impl BufRead,
    active_concepts: &HashSet<String>,
    errors: &mut Vec<String>,
) -> Vec<(String, String)> {
    let mut edges: Vec<(String, String)> = Vec::new();
    let mut seen: HashSet<(String, String)> = HashSet::new();

    for (line_num, line_result) in reader.lines().enumerate() {
        let line = match line_result {
            Ok(l) => l,
            Err(_) => continue,
        };
        if line_num == 0 || line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.splitn(11, '\t').collect();
        if parts.len() < 9 {
            errors.push(format!(
                "Relationship RF2 line {}: expected ≥9 fields, got {} — skipped",
                line_num + 1,
                parts.len()
            ));
            continue;
        }

        let active = parts[2].trim() == "1";
        let child = parts[4].trim();
        let parent = parts[5].trim();
        let type_id = parts[7].trim();

        if !active || type_id != IS_A_TYPE {
            continue;
        }
        if !active_concepts.contains(child) || !active_concepts.contains(parent) {
            continue;
        }

        let edge = (child.to_string(), parent.to_string());
        if seen.insert(edge.clone()) {
            edges.push(edge);
        }
    }
    edges
}

fn extract_release_date(path: &str) -> Option<String> {
    let stem = path.rsplit('/').next().unwrap_or(path);
    let without_ext = stem.strip_suffix(".txt")?;
    let date_part = without_ext.rsplit('_').next()?;
    if date_part.len() == 8 && date_part.chars().all(|c| c.is_ascii_digit()) {
        Some(date_part.to_string())
    } else {
        None
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    use super::*;
    use crate::backends::SqliteTerminologyBackend;
    use std::io::Write;
    use tempfile::NamedTempFile;

    const CONCEPT_TSV: &str = "\
id\teffectiveTime\tactive\tmoduleId\tdefinitionStatusId\r\n\
123456001\t20240101\t1\t900000000000207008\t900000000000074008\r\n\
789012001\t20240101\t1\t900000000000207008\t900000000000074008\r\n\
999999001\t20240101\t0\t900000000000207008\t900000000000074008\r\n";

    const DESCRIPTION_TSV: &str = "\
id\teffectiveTime\tactive\tmoduleId\tconceptId\tlanguageCode\ttypeId\tterm\tcaseSignificanceId\r\n\
111001\t20240101\t1\t900000000000207008\t123456001\ten\t900000000000013009\tFoo disorder\t900000000000448009\r\n\
111002\t20240101\t1\t900000000000207008\t123456001\ten\t900000000000003001\tFoo disorder (disorder)\t900000000000448009\r\n\
111003\t20240101\t1\t900000000000207008\t789012001\ten\t900000000000003001\tBar finding (finding)\t900000000000448009\r\n\
111004\t20240101\t1\t900000000000207008\t999999001\ten\t900000000000013009\tInactive concept\t900000000000448009\r\n";

    const RELATIONSHIP_TSV: &str = "\
id\teffectiveTime\tactive\tmoduleId\tsourceId\tdestinationId\trelationshipGroup\ttypeId\tcharacteristicTypeId\tmodifierId\r\n\
444001\t20240101\t1\t900000000000207008\t789012001\t123456001\t0\t116680003\t900000000000011006\t900000000000451002\r\n";

    fn make_test_rf2_zip() -> NamedTempFile {
        let tmp = NamedTempFile::with_suffix(".zip").unwrap();
        {
            let mut zip = zip::ZipWriter::new(tmp.reopen().unwrap());
            let opts = zip::write::FileOptions::default();

            zip.start_file(
                "Snapshot/Terminology/sct2_Concept_Snapshot_INT_20240101.txt",
                opts,
            )
            .unwrap();
            zip.write_all(CONCEPT_TSV.as_bytes()).unwrap();

            zip.start_file(
                "Snapshot/Terminology/sct2_Description_Snapshot_INT_20240101.txt",
                opts,
            )
            .unwrap();
            zip.write_all(DESCRIPTION_TSV.as_bytes()).unwrap();

            zip.start_file(
                "Snapshot/Terminology/sct2_Relationship_Snapshot_INT_20240101.txt",
                opts,
            )
            .unwrap();
            zip.write_all(RELATIONSHIP_TSV.as_bytes()).unwrap();

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
    fn parse_active_concepts_returns_only_active_ids() {
        let mut errors = Vec::new();
        let active = parse_active_concepts(CONCEPT_TSV.as_bytes(), &mut errors);
        assert!(active.contains("123456001"));
        assert!(active.contains("789012001"));
        assert!(!active.contains("999999001"));
        assert!(errors.is_empty());
    }

    #[test]
    fn parse_preferred_terms_synonym_takes_priority_over_fsn() {
        let mut errors = Vec::new();
        let active = parse_active_concepts(CONCEPT_TSV.as_bytes(), &mut errors);
        let terms = parse_preferred_terms(DESCRIPTION_TSV.as_bytes(), &active, &mut errors);

        assert_eq!(
            terms.get("123456001").map(String::as_str),
            Some("Foo disorder")
        );
        assert_eq!(
            terms.get("789012001").map(String::as_str),
            Some("Bar finding (finding)")
        );
        assert!(!terms.contains_key("999999001"));
    }

    #[test]
    fn parse_is_a_edges_returns_correct_pairs() {
        let mut errors = Vec::new();
        let active = parse_active_concepts(CONCEPT_TSV.as_bytes(), &mut errors);
        let edges = parse_is_a_edges(RELATIONSHIP_TSV.as_bytes(), &active, &mut errors);

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0], ("789012001".to_string(), "123456001".to_string()));
    }

    #[test]
    fn parse_concept_malformed_line_recorded_in_errors() {
        let concept_data = "\
id\teffectiveTime\tactive\tmoduleId\tdefinitionStatusId\r\n\
123456001\t20240101\t1\t900000000000207008\t900000000000074008\r\n\
BADLINE\r\n";
        let mut errors = Vec::new();
        let active = parse_active_concepts(concept_data.as_bytes(), &mut errors);
        assert_eq!(active.len(), 1);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("line 3"));
    }

    #[test]
    fn extract_release_date_parses_standard_rf2_filename() {
        assert_eq!(
            extract_release_date("Snapshot/Terminology/sct2_Concept_Snapshot_INT_20240101.txt"),
            Some("20240101".to_string())
        );
    }

    #[test]
    fn extract_release_date_returns_none_for_unknown_format() {
        assert_eq!(extract_release_date("random_file.txt"), None);
    }

    // ── Importer integration tests ────────────────────────────────────────────

    #[tokio::test]
    async fn import_snomed_rf2_dry_run_does_not_write_to_db() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();
        let zip_file = make_test_rf2_zip();

        let stats = import_snomed_rf2(&backend, &ctx, zip_file.path(), 500, true)
            .await
            .expect("dry-run should succeed");

        assert_eq!(stats.code_systems, 1);
        assert_eq!(stats.concepts, 2);

        assert_eq!(count_rows(&backend, "code_systems"), 0);
        assert_eq!(count_rows(&backend, "concepts"), 0);
        assert_eq!(count_rows(&backend, "concept_hierarchy"), 0);
    }

    #[tokio::test]
    async fn import_snomed_rf2_live_writes_concepts_and_hierarchy() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();
        let zip_file = make_test_rf2_zip();

        let stats = import_snomed_rf2(&backend, &ctx, zip_file.path(), 500, false)
            .await
            .expect("live import should succeed");

        assert_eq!(stats.code_systems, 1);
        assert_eq!(stats.concepts, 2);

        assert_eq!(count_rows(&backend, "code_systems"), 1);
        assert_eq!(count_rows(&backend, "concepts"), 2);
        assert_eq!(count_rows(&backend, "concept_hierarchy"), 1);
    }

    #[tokio::test]
    async fn import_snomed_rf2_idempotent_reimport() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();
        let zip_file = make_test_rf2_zip();

        import_snomed_rf2(&backend, &ctx, zip_file.path(), 500, false)
            .await
            .unwrap();
        import_snomed_rf2(&backend, &ctx, zip_file.path(), 500, false)
            .await
            .unwrap();

        assert_eq!(count_rows(&backend, "code_systems"), 1);
        assert_eq!(count_rows(&backend, "concepts"), 2);
        assert_eq!(count_rows(&backend, "concept_hierarchy"), 1);
    }

    #[tokio::test]
    async fn import_snomed_rf2_batching_preserves_all_concepts() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();
        let zip_file = make_test_rf2_zip();

        let stats = import_snomed_rf2(&backend, &ctx, zip_file.path(), 1, false)
            .await
            .unwrap();

        assert_eq!(stats.concepts, 2);
        assert_eq!(count_rows(&backend, "concepts"), 2);
    }

    #[tokio::test]
    async fn import_snomed_rf2_missing_file_returns_error() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();

        let result = import_snomed_rf2(
            &backend,
            &ctx,
            Path::new("/nonexistent/snomed.zip"),
            500,
            false,
        )
        .await;
        assert!(result.is_err());
    }
}
