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
//!
//! Known limitations to verify once a real distribution is available:
//! - LOINC ZIP structure changed across versions (v2.73 vs v2.74+). This
//!   importer scans by filename pattern, which handles both layouts.
//! - LOINC `MultiAxialHierarchy.csv` uses `LP` (LOINC Part) codes as
//!   category nodes.  These are imported as concepts alongside LOINC codes,
//!   giving a navigable hierarchy tree.
//! - Deprecated/Discouraged codes are imported with their status stored in
//!   the concept `definition` field so they remain look-up-able for legacy data.
//! - Very large panels (LOINC has ~100,000 codes) have not been benchmarked.

#[cfg(feature = "sqlite")]
use r2d2::Pool;
#[cfg(feature = "sqlite")]
use r2d2_sqlite::SqliteConnectionManager;

use std::collections::{HashMap, HashSet};
use std::path::Path;

use zip::ZipArchive;

use crate::error::HtsError;
use crate::import::ImportStats;

// ── LOINC constants ───────────────────────────────────────────────────────────

const LOINC_URL: &str = "http://loinc.org";
const LOINC_NAME: &str = "LOINC";
const LOINC_TITLE: &str = "Logical Observation Identifiers Names and Codes (LOINC)";

// ── Data types ────────────────────────────────────────────────────────────────

struct LoincConcept {
    display: String,
    /// Raw STATUS value from LoincTable: ACTIVE, DEPRECATED, DISCOURAGED, TRIAL.
    status: String,
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Import a LOINC distribution ZIP into the HTS normalized schema.
///
/// Reads two CSV files from `path`:
/// - `*loinc*.csv` (case-insensitive) — LOINC codes and display names
/// - `*multiaxial*.csv` — LP category codes and parent-child hierarchy
///
/// All status values (ACTIVE, DEPRECATED, DISCOURAGED, TRIAL) are imported.
/// Non-ACTIVE codes have their status stored in the `definition` field.
///
/// Progress is written to **stderr** in the uniform `[loinc] …` format.
/// When `dry_run` is `true` the data is parsed and counted but nothing is
/// written to the database.
#[cfg(feature = "sqlite")]
pub fn import_loinc_csv(
    pool: &Pool<SqliteConnectionManager>,
    path: &Path,
    batch_size: usize,
    dry_run: bool,
) -> Result<ImportStats, HtsError> {
    const FORMAT: &str = "loinc";
    let batch_size = batch_size.max(1);

    // ── Step 1: locate CSV files ──────────────────────────────────────────────
    let (loinc_table_path, hierarchy_path) = find_loinc_paths(path)?;

    tracing::info!(
        loinc_table = %loinc_table_path,
        hierarchy = %hierarchy_path,
        "Located LOINC CSV files in archive"
    );

    // ── Step 2: parse LoincTable ──────────────────────────────────────────────
    let mut parse_errors: Vec<String> = Vec::new();

    let loinc_concepts = {
        let mut zip = open_zip(path)?;
        let entry = zip
            .by_name(&loinc_table_path)
            .map_err(|e| HtsError::InvalidRequest(format!("Cannot open LoincTable: {e}")))?;
        parse_loinc_table(entry, &mut parse_errors)?
    };

    tracing::info!(codes = loinc_concepts.len(), "Parsed LOINC codes");

    // ── Step 3: parse multi-axial hierarchy ───────────────────────────────────
    let (hierarchy_concepts, edges) = {
        let mut zip = open_zip(path)?;
        let entry = zip.by_name(&hierarchy_path).map_err(|e| {
            HtsError::InvalidRequest(format!("Cannot open MultiAxialHierarchy: {e}"))
        })?;
        parse_hierarchy(entry, &mut parse_errors)?
    };

    tracing::info!(
        lp_codes = hierarchy_concepts.len(),
        edges = edges.len(),
        "Parsed LOINC hierarchy"
    );

    // ── Step 4: merge all concepts ────────────────────────────────────────────
    // LOINC codes take precedence; LP category codes come from hierarchy.
    let mut all_concepts: HashMap<String, (String, Option<String>)> = HashMap::new();

    // LP codes from hierarchy (display from CODE_TEXT, no status)
    for (code, display) in &hierarchy_concepts {
        all_concepts.insert(code.clone(), (display.clone(), None));
    }
    // LOINC codes (override LP entries for the same code; add definition for non-ACTIVE)
    for (code, concept) in &loinc_concepts {
        let definition = if concept.status != "ACTIVE" {
            Some(format!("STATUS:{}", concept.status))
        } else {
            None
        };
        all_concepts.insert(code.clone(), (concept.display.clone(), definition));
    }

    let total_concepts = all_concepts.len() as u32;

    // ── Step 5: dry-run short-circuit ─────────────────────────────────────────
    if dry_run {
        eprintln!(
            "[{FORMAT}] dry-run — would import {total_concepts} concepts \
             ({} LOINC codes, {} LP category nodes), {} hierarchy edges",
            loinc_concepts.len(),
            hierarchy_concepts.len(),
            edges.len(),
        );
        return Ok(ImportStats {
            code_systems: 1,
            concepts: total_concepts,
            errors: parse_errors,
            ..Default::default()
        });
    }

    // ── Step 6: write to database ─────────────────────────────────────────────
    let conn = pool
        .get()
        .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;

    let system_id = upsert_code_system(&conn)?;

    // Batch-insert concepts.
    let concept_list: Vec<(&str, &str, Option<&str>)> = all_concepts
        .iter()
        .map(|(code, (display, def))| (code.as_str(), display.as_str(), def.as_deref()))
        .collect();

    let num_concept_batches = concept_list.len().div_ceil(batch_size);
    let mut total_inserted: u32 = 0;

    for (i, chunk) in concept_list.chunks(batch_size).enumerate() {
        insert_concept_batch(&conn, &system_id, chunk)?;
        total_inserted += chunk.len() as u32;
        eprintln!(
            "[{FORMAT}] concept batch {}/{num_concept_batches} — +{} concepts (total: {total_inserted})",
            i + 1,
            chunk.len(),
        );
    }

    // Batch-insert hierarchy edges.
    let num_edge_batches = edges.len().div_ceil(batch_size);
    for (i, chunk) in edges.chunks(batch_size).enumerate() {
        insert_hierarchy_batch(&conn, &system_id, chunk)?;
        eprintln!(
            "[{FORMAT}] hierarchy batch {}/{num_edge_batches} — +{} edges",
            i + 1,
            chunk.len(),
        );
    }

    Ok(ImportStats {
        code_systems: 1,
        concepts: total_inserted,
        errors: parse_errors,
        ..Default::default()
    })
}

// ── ZIP helpers ───────────────────────────────────────────────────────────────

fn open_zip(path: &Path) -> Result<ZipArchive<std::fs::File>, HtsError> {
    let file = std::fs::File::open(path)
        .map_err(|e| HtsError::InvalidRequest(format!("Cannot open {}: {e}", path.display())))?;
    ZipArchive::new(file)
        .map_err(|e| HtsError::InvalidRequest(format!("Not a valid ZIP archive: {e}")))
}

/// Scan the ZIP and return paths to LoincTable CSV and MultiAxialHierarchy CSV.
///
/// Handles both old LOINC ZIP layout (files at root) and new layout
/// (files inside subdirectories).
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
                // LOINC ≤ 2.73: MultiAxialHierarchy.csv
                // LOINC ≥ 2.74: ComponentHierarchyBySystem.csv
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

/// Find the index of a required column by name (case-sensitive).
fn find_col(headers: &csv::StringRecord, name: &str) -> Result<usize, HtsError> {
    headers
        .iter()
        .position(|h| h.trim() == name)
        .ok_or_else(|| {
            HtsError::InvalidRequest(format!("Required column '{name}' not found in CSV headers"))
        })
}

/// Parse `LoincTable.csv` (or `Loinc.csv`), returning a map of
/// LOINC_NUM → [`LoincConcept`].
///
/// Display priority: `LONG_COMMON_NAME` → `ShortName` → empty string.
/// Records with an empty `LOINC_NUM` are skipped and recorded in `errors`.
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

/// `(lp_code → display, [(child_code, parent_code)])` — result of parsing the hierarchy CSV.
type HierarchyResult = (HashMap<String, String>, Vec<(String, String)>);

/// Parse `MultiAxialHierarchy.csv`, returning:
/// - A map of LP/LOINC code → display text (for category nodes)
/// - A vec of `(child_code, parent_code)` edges
///
/// Records with an empty `CODE` are skipped and recorded in `errors`.
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

        // Register every node as a candidate concept.
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

// ── Schema writers ────────────────────────────────────────────────────────────

/// Insert or find the LOINC CodeSystem row.
///
/// Uses `INSERT OR IGNORE` so re-importing does not cascade-delete existing concepts.
#[cfg(feature = "sqlite")]
fn upsert_code_system(conn: &rusqlite::Connection) -> Result<String, HtsError> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    conn.execute(
        "INSERT OR IGNORE INTO code_systems \
         (id, url, name, title, status, content, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, 'active', 'complete', ?5, ?5)",
        rusqlite::params![id, LOINC_URL, LOINC_NAME, LOINC_TITLE, now],
    )
    .map_err(|e| HtsError::StorageError(format!("Failed to insert CodeSystem: {e}")))?;

    let system_id: String = conn
        .query_row(
            "SELECT id FROM code_systems WHERE url = ?1",
            rusqlite::params![LOINC_URL],
            |row| row.get(0),
        )
        .map_err(|e| HtsError::StorageError(format!("Failed to query CodeSystem id: {e}")))?;

    Ok(system_id)
}

/// Batch-insert `(code, display, definition)` triples in a single transaction.
#[cfg(feature = "sqlite")]
fn insert_concept_batch(
    conn: &rusqlite::Connection,
    system_id: &str,
    batch: &[(&str, &str, Option<&str>)],
) -> Result<(), HtsError> {
    conn.execute_batch("BEGIN")
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    for (code, display, definition) in batch {
        conn.execute(
            "INSERT OR REPLACE INTO concepts (system_id, code, display, definition) \
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![system_id, code, display, definition],
        )
        .map_err(|e| HtsError::StorageError(format!("Concept insert error: {e}")))?;
    }

    conn.execute_batch("COMMIT")
        .map_err(|e| HtsError::StorageError(e.to_string()))
}

/// Batch-insert `(child_code, parent_code)` hierarchy edges in a single transaction.
#[cfg(feature = "sqlite")]
fn insert_hierarchy_batch(
    conn: &rusqlite::Connection,
    system_id: &str,
    batch: &[(String, String)],
) -> Result<(), HtsError> {
    conn.execute_batch("BEGIN")
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    for (child, parent) in batch {
        conn.execute(
            "INSERT OR IGNORE INTO concept_hierarchy (system_id, parent_code, child_code) \
             VALUES (?1, ?2, ?3)",
            rusqlite::params![system_id, parent, child],
        )
        .map_err(|e| HtsError::StorageError(format!("Hierarchy insert error: {e}")))?;
    }

    conn.execute_batch("COMMIT")
        .map_err(|e| HtsError::StorageError(e.to_string()))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    use super::*;
    use crate::backend::SqliteTerminologyBackend;
    use std::io::Write;
    use tempfile::NamedTempFile;

    // ── Synthetic CSV fixtures ────────────────────────────────────────────────

    /// Minimal LoincTable with 2 active codes and 1 deprecated.
    const LOINC_TABLE_CSV: &str = "\
LOINC_NUM,LONG_COMMON_NAME,ShortName,STATUS\r\n\
2160-0,Creatinine [Mass/volume] in Serum or Plasma,Creat SerPl-mCnc,ACTIVE\r\n\
718-7,Hemoglobin [Mass/volume] in Blood,Hgb Bld-mCnc,ACTIVE\r\n\
99999-9,Old deprecated test,Old test,DEPRECATED\r\n";

    /// Hierarchy: root LP → chemistry LP → 2160-0; root LP → hematology LP → 718-7.
    const HIERARCHY_CSV: &str = "\
PATH_TO_ROOT,SEQUENCE,IMMEDIATE_PARENT,CODE,CODE_TEXT\r\n\
LP7786-3,1,,LP7786-3,Laboratory\r\n\
LP7786-3.LP29693-6,2,LP7786-3,LP29693-6,Chemistry\r\n\
LP7786-3.LP29693-6.2160-0,3,LP29693-6,2160-0,Creatinine\r\n\
LP7786-3.LP10156-0,2,LP7786-3,LP10156-0,Hematology\r\n\
LP7786-3.LP10156-0.718-7,3,LP10156-0,718-7,Hemoglobin\r\n";

    /// Build a ZIP with the two synthetic CSV files.
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

    fn count_rows(pool: &Pool<SqliteConnectionManager>, table: &str) -> i64 {
        let conn = pool.get().unwrap();
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
        // 5 unique nodes: LP7786-3, LP29693-6, 2160-0, LP10156-0, 718-7
        assert_eq!(concepts.len(), 5);
        assert_eq!(concepts["LP7786-3"], "Laboratory");
        // 4 edges (LP7786-3 has no parent, so only 4 rows have IMMEDIATE_PARENT)
        assert_eq!(edges.len(), 4);
    }

    #[test]
    fn import_loinc_malformed_table_row_recorded_in_errors() {
        // LoincTable with one valid row and one row with an empty LOINC_NUM
        let csv = "LOINC_NUM,LONG_COMMON_NAME,ShortName,STATUS\r\n\
                   2160-0,Creatinine,Creat,ACTIVE\r\n\
                   ,Missing code,Bad,ACTIVE\r\n";
        let mut errors = Vec::new();
        let concepts = parse_loinc_table(csv.as_bytes(), &mut errors).unwrap();
        assert_eq!(concepts.len(), 1, "only the valid row should be parsed");
        assert_eq!(errors.len(), 1, "one error for the empty-code row");
        assert!(
            errors[0].contains("LOINC_NUM is empty"),
            "error message should describe the problem: {}",
            errors[0]
        );
    }

    #[test]
    fn deprecated_status_stored_in_definition() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();
        let zip_file = make_test_loinc_zip();

        import_loinc_csv(&pool, zip_file.path(), 500, false).unwrap();

        let conn = pool.get().unwrap();
        let definition: Option<String> = conn
            .query_row(
                "SELECT definition FROM concepts WHERE code = '99999-9'",
                [],
                |row| row.get(0),
            )
            .ok();
        assert_eq!(
            definition.as_deref(),
            Some("STATUS:DEPRECATED"),
            "deprecated codes should carry their status in definition"
        );
    }

    // ── Importer integration tests ────────────────────────────────────────────

    #[test]
    fn import_loinc_csv_dry_run_does_not_write_to_db() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();
        let zip_file = make_test_loinc_zip();

        let stats =
            import_loinc_csv(&pool, zip_file.path(), 500, true).expect("dry-run should succeed");

        // 3 LOINC codes + 2 LP codes (LP7786-3 and LP29693-6 and LP10156-0 = 3 LP codes)
        assert_eq!(stats.code_systems, 1);
        assert!(stats.concepts > 0);

        assert_eq!(
            count_rows(&pool, "code_systems"),
            0,
            "dry-run must not write"
        );
        assert_eq!(count_rows(&pool, "concepts"), 0, "dry-run must not write");
    }

    #[test]
    fn import_loinc_csv_live_writes_concepts_and_hierarchy() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();
        let zip_file = make_test_loinc_zip();

        let stats = import_loinc_csv(&pool, zip_file.path(), 500, false)
            .expect("live import should succeed");

        assert_eq!(stats.code_systems, 1);
        // 3 LOINC codes + 3 LP codes = 6 total concepts
        assert_eq!(stats.concepts, 6);

        assert_eq!(count_rows(&pool, "code_systems"), 1);
        assert_eq!(count_rows(&pool, "concepts"), 6);
        assert_eq!(count_rows(&pool, "concept_hierarchy"), 4);
    }

    #[test]
    fn import_loinc_csv_idempotent_reimport() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();
        let zip_file = make_test_loinc_zip();

        import_loinc_csv(&pool, zip_file.path(), 500, false).unwrap();
        import_loinc_csv(&pool, zip_file.path(), 500, false).unwrap();

        assert_eq!(
            count_rows(&pool, "code_systems"),
            1,
            "no duplicate CodeSystems"
        );
        assert_eq!(count_rows(&pool, "concepts"), 6, "no duplicate concepts");
        assert_eq!(
            count_rows(&pool, "concept_hierarchy"),
            4,
            "no duplicate edges"
        );
    }

    #[test]
    fn import_loinc_csv_batching_preserves_all_concepts() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();
        let zip_file = make_test_loinc_zip();

        // batch_size=2 forces multiple batches
        let stats = import_loinc_csv(&pool, zip_file.path(), 2, false).unwrap();
        assert_eq!(stats.concepts, 6);
        assert_eq!(count_rows(&pool, "concepts"), 6);
    }

    #[test]
    fn import_loinc_csv_missing_file_returns_error() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();

        let result = import_loinc_csv(&pool, Path::new("/nonexistent/loinc.zip"), 500, false);
        assert!(result.is_err());
    }

    /// Gap 7 — verify that `find_loinc_paths` resolves files inside ZIP subdirectories.
    ///
    /// Some LOINC distributions nest the CSVs inside a version-named folder
    /// (e.g. `Loinc_2.77_August_release/LoincTable.csv`).  The importer must
    /// handle both root-level and nested layouts without any code changes.
    #[test]
    fn import_loinc_nested_zip_layout() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();

        let tmp = NamedTempFile::with_suffix(".zip").unwrap();
        {
            let mut zip = zip::ZipWriter::new(tmp.reopen().unwrap());
            let opts = zip::write::FileOptions::default();

            // CSVs are nested two levels deep — simulates real LOINC ZIP structure
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

        let stats = import_loinc_csv(&pool, tmp.path(), 500, true).unwrap();

        // dry_run=true so no DB writes, but concept count must be non-zero
        assert!(
            stats.concepts > 0,
            "nested ZIP layout should resolve both CSV files; got 0 concepts"
        );
        assert!(
            stats.errors.is_empty(),
            "unexpected errors: {:?}",
            stats.errors
        );
    }
}
