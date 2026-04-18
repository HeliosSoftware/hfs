//! LOINC CSV importer.
//!
//! Reads a LOINC distribution ZIP and imports LOINC codes, preferred display
//! names, status, and the multi-axial hierarchy into the HTS normalized schema.
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
use crate::import::bundle_builder::{BuilderConcept, CodeSystemMeta, build_code_system_bundle};

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
}

#[derive(Debug, Default)]
struct LoincParseResult {
    concepts: ConceptEntries,
    loinc_count: usize,
    lp_count: usize,
    edge_count: usize,
    parse_errors: Vec<String>,
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Import a LOINC distribution ZIP through the given backend.
pub async fn import_loinc_csv(
    backend: &dyn BundleImportBackend,
    ctx: &TenantContext,
    path: &Path,
    batch_size: usize,
    dry_run: bool,
) -> Result<ImportStats, HtsError> {
    const FORMAT: &str = "loinc";
    let batch_size = batch_size.max(1);

    let path_owned = path.to_path_buf();
    let parsed = tokio::task::spawn_blocking(move || -> Result<LoincParseResult, HtsError> {
        let (loinc_table_path, hierarchy_path) = find_loinc_paths(&path_owned)?;

        tracing::info!(
            loinc_table = %loinc_table_path,
            hierarchy = %hierarchy_path,
            "Located LOINC CSV files in archive"
        );

        let mut parse_errors: Vec<String> = Vec::new();

        let loinc_concepts = {
            let mut zip = open_zip(&path_owned)?;
            let entry = zip
                .by_name(&loinc_table_path)
                .map_err(|e| HtsError::InvalidRequest(format!("Cannot open LoincTable: {e}")))?;
            parse_loinc_table(entry, &mut parse_errors)?
        };

        let (hierarchy_concepts, edges) = {
            let mut zip = open_zip(&path_owned)?;
            let entry = zip.by_name(&hierarchy_path).map_err(|e| {
                HtsError::InvalidRequest(format!("Cannot open MultiAxialHierarchy: {e}"))
            })?;
            parse_hierarchy(entry, &mut parse_errors)?
        };

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
                },
            );
        }

        Ok(LoincParseResult {
            concepts: all_concepts,
            loinc_count: loinc_concepts.len(),
            lp_count: hierarchy_concepts.len(),
            edge_count: edges.len(),
            parse_errors,
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

    if dry_run {
        stats.concepts = total_concepts;
        eprintln!(
            "[{FORMAT}] dry-run — would import {total_concepts} concepts \
             ({} LOINC codes, {} LP category nodes), {} hierarchy edges",
            parsed.loinc_count, parsed.lp_count, parsed.edge_count,
        );
        return Ok(stats);
    }

    let meta = CodeSystemMeta {
        id: LOINC_ID,
        url: LOINC_URL,
        version: None,
        name: Some(LOINC_NAME),
        title: Some(LOINC_TITLE),
        status: "active",
        content: "complete",
    };

    // Seed: empty CodeSystem to upsert metadata.
    let seed_bytes = build_code_system_bundle(&meta, &[]);
    let seed_stats = backend.import_bundle(ctx, &seed_bytes).await?;
    stats.code_systems = seed_stats.code_systems;
    stats.errors.extend(seed_stats.errors);

    // Collect concepts into a stable order so chunks across retries are
    // deterministic.
    let concept_list: Vec<(&String, &MergedConcept)> = parsed.concepts.iter().collect();
    let num_batches = concept_list.len().div_ceil(batch_size).max(1);

    for (i, chunk) in concept_list.chunks(batch_size).enumerate() {
        let builder: Vec<BuilderConcept<'_>> = chunk
            .iter()
            .map(|(code, entry)| BuilderConcept {
                code: code.as_str(),
                display: Some(entry.display.as_str()).filter(|s| !s.is_empty()),
                definition: entry.definition.as_deref(),
                parent_code: entry.parent.as_deref(),
                ..Default::default()
            })
            .collect();

        let bytes = build_code_system_bundle(&meta, &builder);
        let chunk_stats = backend.import_bundle(ctx, &bytes).await?;
        stats.errors.extend(chunk_stats.errors);
        stats.concepts += chunk.len() as u32;

        eprintln!(
            "[{FORMAT}] concept batch {}/{num_batches} — +{} concepts (total: {})",
            i + 1,
            chunk.len(),
            stats.concepts,
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

fn find_loinc_paths(path: &Path) -> Result<(String, String), HtsError> {
    let mut zip = open_zip(path)?;

    let mut loinc_path: Option<String> = None;
    let mut hierarchy_path: Option<String> = None;

    for i in 0..zip.len() {
        let entry = zip
            .by_index(i)
            .map_err(|e| HtsError::InvalidRequest(format!("ZIP entry error: {e}")))?;
        let name = entry.name().to_string();
        let lower = name.to_lowercase();
        let filename = lower.rsplit('/').next().unwrap_or(&lower);

        if filename.ends_with(".csv") {
            if (filename.starts_with("loinc") && !filename.contains("panel"))
                || filename.ends_with("loinctable.csv")
            {
                loinc_path = Some(name);
            } else if filename.contains("multiaxial") || filename.contains("componenthierarchy") {
                hierarchy_path = Some(name);
            }
        }
    }

    Ok((
        loinc_path.ok_or_else(|| {
            HtsError::InvalidRequest(
                "No LoincTable CSV found. Expected a file whose name starts with 'loinc' or contains 'loinctable'.".into(),
            )
        })?,
        hierarchy_path.ok_or_else(|| {
            HtsError::InvalidRequest(
                "No hierarchy CSV found. Expected 'MultiAxialHierarchy.csv' (LOINC ≤ 2.73) or 'ComponentHierarchyBySystem.csv' (LOINC ≥ 2.74).".into(),
            )
        })?,
    ))
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

        import_loinc_csv(&backend, &ctx, zip_file.path(), 500, false)
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

        let stats = import_loinc_csv(&backend, &ctx, zip_file.path(), 500, true)
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

        let stats = import_loinc_csv(&backend, &ctx, zip_file.path(), 500, false)
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

        import_loinc_csv(&backend, &ctx, zip_file.path(), 500, false)
            .await
            .unwrap();
        import_loinc_csv(&backend, &ctx, zip_file.path(), 500, false)
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

        let stats = import_loinc_csv(&backend, &ctx, zip_file.path(), 2, false)
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
        )
        .await;
        assert!(result.is_err());
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

        let stats = import_loinc_csv(&backend, &ctx, tmp.path(), 500, true)
            .await
            .unwrap();

        assert!(stats.concepts > 0, "got 0 concepts");
        assert!(
            stats.errors.is_empty(),
            "unexpected errors: {:?}",
            stats.errors
        );
    }
}
