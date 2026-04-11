//! SNOMED CT RF2 importer.
//!
//! Reads a SNOMED CT RF2 distribution ZIP and imports active concepts,
//! preferred terms, and `Is-a` hierarchy edges into the existing HTS
//! normalized schema.
//!
//! # ⚠️  LICENSE REQUIRED
//!
//! Real SNOMED CT data requires a license from SNOMED International.
//! Most countries have a National Release Center (NRC) that grants free
//! licenses for healthcare use.  This parser was written and tested using
//! **synthetic fixture data only**.
//!
//! Known limitations to verify once a real license and distribution are
//! available:
//! - Preferred-term selection here uses the first active English synonym;
//!   the official approach requires a language reference set file that is
//!   not parsed in this phase.
//! - `concept_hierarchy` stores **direct** Is-a edges only.  Transitive
//!   closure is handled by the `$subsumes` query layer.  Performance at
//!   SNOMED scale (~1 M+ edges) has not been benchmarked.
//! - Full-release files contain multiple rows per concept (one per
//!   snapshot date); this importer keeps the **last occurrence** per ID,
//!   which works correctly for both Snapshot and Full releases when files
//!   are sorted by effectiveTime ascending (standard RF2 sort order).

#[cfg(feature = "sqlite")]
use r2d2::Pool;
#[cfg(feature = "sqlite")]
use r2d2_sqlite::SqliteConnectionManager;

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::path::Path;

use zip::ZipArchive;

use crate::error::HtsError;
use crate::import::ImportStats;

// ── SNOMED CT constants ───────────────────────────────────────────────────────

const SNOMED_URL: &str = "http://snomed.info/sct";
const SNOMED_NAME: &str = "SNOMED_CT";
const SNOMED_TITLE: &str = "SNOMED CT";

/// SNOMED description typeId for a Fully Specified Name.
const TYPE_FSN: &str = "900000000000003001";
/// SNOMED description typeId for a Synonym (preferred term candidate).
const TYPE_SYNONYM: &str = "900000000000013009";
/// SNOMED relationship typeId for the Is-a relationship.
const IS_A_TYPE: &str = "116680003";

// ── Public entry point ────────────────────────────────────────────────────────

/// Import a SNOMED CT RF2 distribution ZIP into the HTS normalized schema.
///
/// Reads three RF2 files from `path`:
/// - `*Concept_*.txt`     — active concept IDs
/// - `*Description_*.txt` — preferred terms (English synonyms / FSN fallback)
/// - `*Relationship_*.txt`— direct Is-a hierarchy edges
///
/// Concepts are inserted in batches of `batch_size`.  Progress is written
/// to **stderr** in the uniform `[snomed-rf2] …` format.
///
/// When `dry_run` is `true` the data is parsed and counted but nothing is
/// written to the database.
///
/// # Direct schema writes
///
/// Unlike the HL7 NPM importer, this function writes **directly** to the
/// pool rather than going through `import_bundle_sync`.  This avoids the
/// `INSERT OR REPLACE INTO code_systems` cascade that would delete concepts
/// inserted by earlier batches.
#[cfg(feature = "sqlite")]
pub fn import_snomed_rf2(
    pool: &Pool<SqliteConnectionManager>,
    path: &Path,
    batch_size: usize,
    dry_run: bool,
) -> Result<ImportStats, HtsError> {
    const FORMAT: &str = "snomed-rf2";
    let batch_size = batch_size.max(1);

    // ── Step 1: locate the three RF2 files inside the ZIP ────────────────────
    let (concept_path, desc_path, rel_path) = find_rf2_paths(path)?;

    tracing::info!(
        concept_file = %concept_path,
        description_file = %desc_path,
        relationship_file = %rel_path,
        "Located RF2 files in archive"
    );

    // ── Step 2: parse active concept IDs ─────────────────────────────────────
    let mut parse_errors: Vec<String> = Vec::new();

    let active_concepts = {
        let mut zip = open_zip(path)?;
        let entry = zip
            .by_name(&concept_path)
            .map_err(|e| HtsError::InvalidRequest(format!("Cannot open concept file: {e}")))?;
        parse_active_concepts(BufReader::new(entry), &mut parse_errors)
    };

    tracing::info!(active = active_concepts.len(), "Parsed active concepts");

    // ── Step 3: parse preferred terms ────────────────────────────────────────
    let preferred_terms = {
        let mut zip = open_zip(path)?;
        let entry = zip
            .by_name(&desc_path)
            .map_err(|e| HtsError::InvalidRequest(format!("Cannot open description file: {e}")))?;
        parse_preferred_terms(BufReader::new(entry), &active_concepts, &mut parse_errors)
    };

    tracing::info!(terms = preferred_terms.len(), "Parsed preferred terms");

    // ── Step 4: parse Is-a edges ──────────────────────────────────────────────
    let is_a_edges = {
        let mut zip = open_zip(path)?;
        let entry = zip
            .by_name(&rel_path)
            .map_err(|e| HtsError::InvalidRequest(format!("Cannot open relationship file: {e}")))?;
        parse_is_a_edges(BufReader::new(entry), &active_concepts, &mut parse_errors)
    };

    tracing::info!(edges = is_a_edges.len(), "Parsed Is-a hierarchy edges");

    let concept_count = preferred_terms.len() as u32;
    let edge_count = is_a_edges.len();

    // ── Step 5: dry-run short-circuit ─────────────────────────────────────────
    if dry_run {
        eprintln!(
            "[{FORMAT}] dry-run — would import {concept_count} concepts, {edge_count} Is-a edges"
        );
        return Ok(ImportStats {
            code_systems: 1,
            concepts: concept_count,
            errors: parse_errors,
            ..Default::default()
        });
    }

    // ── Step 6: write to database ─────────────────────────────────────────────
    let conn = pool
        .get()
        .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;

    // Extract the release version from any concept file entry name (YYYYMMDD suffix).
    let release_version = extract_release_date(&concept_path);

    let system_id = upsert_code_system(&conn, release_version.as_deref())?;

    // Batch-insert concepts.
    let concepts: Vec<(&str, Option<&str>)> = preferred_terms
        .iter()
        .map(|(id, display)| (id.as_str(), Some(display.as_str())))
        .collect();

    let num_concept_batches = concepts.len().div_ceil(batch_size);
    let mut total_concepts: u32 = 0;

    for (i, chunk) in concepts.chunks(batch_size).enumerate() {
        insert_concept_batch(&conn, &system_id, chunk)?;
        total_concepts += chunk.len() as u32;
        eprintln!(
            "[{FORMAT}] concept batch {}/{num_concept_batches} — +{} concepts (total: {total_concepts})",
            i + 1,
            chunk.len(),
        );
    }

    // Batch-insert hierarchy edges.
    let num_edge_batches = is_a_edges.len().div_ceil(batch_size);
    for (i, chunk) in is_a_edges.chunks(batch_size).enumerate() {
        insert_hierarchy_batch(&conn, &system_id, chunk)?;
        eprintln!(
            "[{FORMAT}] hierarchy batch {}/{num_edge_batches} — +{} edges",
            i + 1,
            chunk.len(),
        );
    }

    Ok(ImportStats {
        code_systems: 1,
        concepts: total_concepts,
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

/// Scan the archive and return the paths of the three required RF2 files.
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
        // Avoid language reference set files and other Refset files.
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
                "No Concept RF2 file found. Expected a file containing 'Concept_' in its path."
                    .into(),
            )
        })?,
        desc_path.ok_or_else(|| {
            HtsError::InvalidRequest(
                "No Description RF2 file found. Expected a file containing 'Description_' in its path."
                    .into(),
            )
        })?,
        rel_path.ok_or_else(|| {
            HtsError::InvalidRequest(
                "No Relationship RF2 file found. Expected a file containing 'Relationship_' in its path."
                    .into(),
            )
        })?,
    ))
}

// ── RF2 parsers ───────────────────────────────────────────────────────────────

/// Parse the Concept file, returning a set of active concept IDs.
///
/// RF2 Concept columns: id, effectiveTime, active, moduleId, definitionStatusId
/// Malformed lines (fewer than 3 tab-delimited fields) are skipped and recorded in `errors`.
fn parse_active_concepts(reader: impl BufRead, errors: &mut Vec<String>) -> HashSet<String> {
    let mut active = HashSet::new();

    for (line_num, line_result) in reader.lines().enumerate() {
        let line = match line_result {
            Ok(l) => l,
            Err(_) => continue,
        };
        if line_num == 0 || line.is_empty() {
            continue; // skip header
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

        // For Full releases (multiple rows per concept), the last row wins.
        // Since RF2 files are sorted by effectiveTime ascending, the last row
        // for each ID is the most recent.
        if is_active {
            active.insert(id);
        } else {
            active.remove(&id);
        }
    }
    active
}

/// Parse the Description file, returning a map of conceptId → preferred display term.
///
/// Selection priority (per concept, English only):
/// 1. Active Synonym (`typeId = 900000000000013009`)
/// 2. Active FSN (`typeId = 900000000000003001`) as fallback
///
/// ⚠️  A production implementation should use the language reference set to
/// determine the official preferred term for the target locale.  This
/// simplified approach picks the first active English synonym found.
///
/// RF2 Description columns:
///   id, effectiveTime, active, moduleId, conceptId, languageCode, typeId, term, caseSignificanceId
///
/// Malformed lines (fewer than 9 fields) are skipped and recorded in `errors`.
fn parse_preferred_terms(
    reader: impl BufRead,
    active_concepts: &HashSet<String>,
    errors: &mut Vec<String>,
) -> HashMap<String, String> {
    // (conceptId → (synonym_term, fsn_term))
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

    // Merge: synonym takes priority; fall back to FSN.
    let mut terms: HashMap<String, String> = synonyms;
    for concept_id in active_concepts {
        if !terms.contains_key(concept_id) {
            if let Some(fsn) = fsns.get(concept_id) {
                terms.insert(concept_id.clone(), fsn.clone());
            } else {
                // Concept with no description at all — insert with empty display.
                terms.entry(concept_id.clone()).or_default();
            }
        }
    }
    terms
}

/// Parse the Relationship file, returning active Is-a edges as (child, parent) pairs.
///
/// RF2 Relationship columns:
///   id, effectiveTime, active, moduleId, sourceId, destinationId,
///   relationshipGroup, typeId, characteristicTypeId, modifierId
///
/// Malformed lines (fewer than 9 fields) are skipped and recorded in `errors`.
fn parse_is_a_edges(
    reader: impl BufRead,
    active_concepts: &HashSet<String>,
    errors: &mut Vec<String>,
) -> Vec<(String, String)> {
    let mut edges: Vec<(String, String)> = Vec::new();
    // Deduplicate: Full releases may repeat active edges across snapshot dates.
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
        let child = parts[4].trim(); // sourceId
        let parent = parts[5].trim(); // destinationId
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

// ── Schema writers ────────────────────────────────────────────────────────────

/// Insert or find the SNOMED CT CodeSystem row.
///
/// Uses `INSERT OR IGNORE` so that re-importing does not delete existing
/// concepts via the `ON DELETE CASCADE` constraint on `concepts.system_id`.
#[cfg(feature = "sqlite")]
fn upsert_code_system(
    conn: &rusqlite::Connection,
    version: Option<&str>,
) -> Result<String, HtsError> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    conn.execute(
        "INSERT OR IGNORE INTO code_systems \
         (id, url, version, name, title, status, content, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, 'active', 'complete', ?6, ?6)",
        rusqlite::params![id, SNOMED_URL, version, SNOMED_NAME, SNOMED_TITLE, now],
    )
    .map_err(|e| HtsError::StorageError(format!("Failed to insert CodeSystem: {e}")))?;

    // Retrieve the actual ID (may differ from `id` if row already existed).
    let system_id: String = conn
        .query_row(
            "SELECT id FROM code_systems WHERE url = ?1",
            rusqlite::params![SNOMED_URL],
            |row| row.get(0),
        )
        .map_err(|e| HtsError::StorageError(format!("Failed to query CodeSystem id: {e}")))?;

    Ok(system_id)
}

/// Batch-insert a slice of (code, display) pairs inside a single transaction.
#[cfg(feature = "sqlite")]
fn insert_concept_batch(
    conn: &rusqlite::Connection,
    system_id: &str,
    batch: &[(&str, Option<&str>)],
) -> Result<(), HtsError> {
    conn.execute_batch("BEGIN")
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    for (code, display) in batch {
        conn.execute(
            "INSERT OR REPLACE INTO concepts (system_id, code, display) VALUES (?1, ?2, ?3)",
            rusqlite::params![system_id, code, display],
        )
        .map_err(|e| HtsError::StorageError(format!("Concept insert error: {e}")))?;
    }

    conn.execute_batch("COMMIT")
        .map_err(|e| HtsError::StorageError(e.to_string()))
}

/// Batch-insert a slice of (child_code, parent_code) Is-a edges inside a single transaction.
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

/// Extract the release date (YYYYMMDD) from an RF2 file name.
///
/// e.g. `SnomedCT/Snapshot/Terminology/sct2_Concept_Snapshot_INT_20240101.txt`
/// returns `Some("20240101")`.
fn extract_release_date(path: &str) -> Option<String> {
    let stem = path.rsplit('/').next().unwrap_or(path);
    // RF2 names end with _YYYYMMDD.txt
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

    // ── Synthetic RF2 fixtures ────────────────────────────────────────────────

    const CONCEPT_TSV: &str = "\
id\teffectiveTime\tactive\tmoduleId\tdefinitionStatusId\r\n\
123456001\t20240101\t1\t900000000000207008\t900000000000074008\r\n\
789012001\t20240101\t1\t900000000000207008\t900000000000074008\r\n\
999999001\t20240101\t0\t900000000000207008\t900000000000074008\r\n";

    /// 123456001 has a synonym and an FSN.
    /// 789012001 has only an FSN (no synonym).
    /// 999999001 is inactive — its description should not appear.
    const DESCRIPTION_TSV: &str = "\
id\teffectiveTime\tactive\tmoduleId\tconceptId\tlanguageCode\ttypeId\tterm\tcaseSignificanceId\r\n\
111001\t20240101\t1\t900000000000207008\t123456001\ten\t900000000000013009\tFoo disorder\t900000000000448009\r\n\
111002\t20240101\t1\t900000000000207008\t123456001\ten\t900000000000003001\tFoo disorder (disorder)\t900000000000448009\r\n\
111003\t20240101\t1\t900000000000207008\t789012001\ten\t900000000000003001\tBar finding (finding)\t900000000000448009\r\n\
111004\t20240101\t1\t900000000000207008\t999999001\ten\t900000000000013009\tInactive concept\t900000000000448009\r\n";

    /// 789012001 is-a 123456001
    const RELATIONSHIP_TSV: &str = "\
id\teffectiveTime\tactive\tmoduleId\tsourceId\tdestinationId\trelationshipGroup\ttypeId\tcharacteristicTypeId\tmodifierId\r\n\
444001\t20240101\t1\t900000000000207008\t789012001\t123456001\t0\t116680003\t900000000000011006\t900000000000451002\r\n";

    /// Build a ZIP containing three synthetic RF2 files.
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

    fn count_rows(pool: &Pool<SqliteConnectionManager>, table: &str) -> i64 {
        let conn = pool.get().unwrap();
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
        assert!(
            !active.contains("999999001"),
            "inactive concept must be excluded"
        );
        assert!(errors.is_empty());
    }

    #[test]
    fn parse_preferred_terms_synonym_takes_priority_over_fsn() {
        let mut errors = Vec::new();
        let active = parse_active_concepts(CONCEPT_TSV.as_bytes(), &mut errors);
        let terms = parse_preferred_terms(DESCRIPTION_TSV.as_bytes(), &active, &mut errors);

        assert_eq!(
            terms.get("123456001").map(String::as_str),
            Some("Foo disorder"),
            "synonym should be preferred over FSN"
        );
        assert_eq!(
            terms.get("789012001").map(String::as_str),
            Some("Bar finding (finding)"),
            "FSN used as fallback when no synonym exists"
        );
        assert!(
            !terms.contains_key("999999001"),
            "inactive concept must not appear in terms"
        );
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
    fn import_snomed_malformed_concept_line_recorded_in_errors() {
        // Concept file with one valid line and one line with only 2 fields (no active flag)
        let concept_data = "\
id\teffectiveTime\tactive\tmoduleId\tdefinitionStatusId\r\n\
123456001\t20240101\t1\t900000000000207008\t900000000000074008\r\n\
BADLINE\r\n";
        let mut errors = Vec::new();
        let active = parse_active_concepts(concept_data.as_bytes(), &mut errors);
        assert_eq!(
            active.len(),
            1,
            "only the valid concept line should be parsed"
        );
        assert_eq!(errors.len(), 1, "one error for the malformed line");
        assert!(
            errors[0].contains("line 3"),
            "error should mention line number: {}",
            errors[0]
        );
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

    #[test]
    fn import_snomed_rf2_dry_run_does_not_write_to_db() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();
        let zip_file = make_test_rf2_zip();

        let stats =
            import_snomed_rf2(&pool, zip_file.path(), 500, true).expect("dry-run should succeed");

        assert_eq!(stats.code_systems, 1);
        assert_eq!(stats.concepts, 2, "only 2 active concepts");

        assert_eq!(
            count_rows(&pool, "code_systems"),
            0,
            "dry-run must not write CodeSystem"
        );
        assert_eq!(
            count_rows(&pool, "concepts"),
            0,
            "dry-run must not write concepts"
        );
        assert_eq!(
            count_rows(&pool, "concept_hierarchy"),
            0,
            "dry-run must not write hierarchy"
        );
    }

    #[test]
    fn import_snomed_rf2_live_writes_concepts_and_hierarchy() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();
        let zip_file = make_test_rf2_zip();

        let stats = import_snomed_rf2(&pool, zip_file.path(), 500, false)
            .expect("live import should succeed");

        assert_eq!(stats.code_systems, 1);
        assert_eq!(stats.concepts, 2);

        assert_eq!(
            count_rows(&pool, "code_systems"),
            1,
            "one SNOMED CodeSystem"
        );
        assert_eq!(count_rows(&pool, "concepts"), 2, "two active concepts");
        assert_eq!(count_rows(&pool, "concept_hierarchy"), 1, "one Is-a edge");
    }

    #[test]
    fn import_snomed_rf2_idempotent_reimport() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();
        let zip_file = make_test_rf2_zip();

        import_snomed_rf2(&pool, zip_file.path(), 500, false).unwrap();
        // Second import must not fail or duplicate data
        import_snomed_rf2(&pool, zip_file.path(), 500, false).unwrap();

        assert_eq!(
            count_rows(&pool, "code_systems"),
            1,
            "no duplicate CodeSystems"
        );
        assert_eq!(count_rows(&pool, "concepts"), 2, "no duplicate concepts");
        assert_eq!(
            count_rows(&pool, "concept_hierarchy"),
            1,
            "no duplicate hierarchy edges"
        );
    }

    #[test]
    fn import_snomed_rf2_batching_preserves_all_concepts() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();
        let zip_file = make_test_rf2_zip();

        // batch_size=1 forces one concept per batch
        let stats = import_snomed_rf2(&pool, zip_file.path(), 1, false).unwrap();

        assert_eq!(stats.concepts, 2);
        assert_eq!(count_rows(&pool, "concepts"), 2);
    }

    #[test]
    fn import_snomed_rf2_missing_file_returns_error() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();

        let result = import_snomed_rf2(&pool, Path::new("/nonexistent/snomed.zip"), 500, false);
        assert!(result.is_err());
    }
}
