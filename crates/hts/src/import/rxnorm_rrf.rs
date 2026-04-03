//! RxNorm RRF importer.
//!
//! Parses the NLM RxNorm full release distribution and imports drug concepts,
//! preferred display names, and `isa` hierarchy edges into the HTS normalized
//! schema.
//!
//! # ⚠️  LICENSE REQUIRED
//!
//! Real RxNorm data requires acceptance of the NLM Terms of Service.
//! The agreement is free and takes ~5 minutes at
//! <https://www.nlm.nih.gov/databases/umls.html>.
//! This parser was written and tested using **synthetic fixture data only**.
//!
//! # Input formats
//!
//! `path` may be:
//! - A **folder** containing `RXNCONSO.RRF` and `RXNREL.RRF` directly.
//! - A **ZIP file** containing those files anywhere inside the archive.
//!
//! # RRF format
//!
//! RRF files are pipe-delimited with **no header row** and an optional
//! trailing pipe on each line.
//!
//! ## `RXNCONSO.RRF` columns (indices used)
//!
//! | Index | Field | Notes |
//! |-------|-------|-------|
//! | 0 | RXCUI | Concept identifier |
//! | 1 | LAT | Language — filter to `ENG` |
//! | 6 | ISPREF | `Y` = preferred term |
//! | 11 | SAB | Source — filter to `RXNORM` |
//! | 12 | TTY | Term type (IN, BN, SCD, …) |
//! | 14 | STR | Display string |
//! | 16 | SUPPRESS | `O` = obsolete — skip |
//!
//! ## `RXNREL.RRF` columns (indices used)
//!
//! | Index | Field | Notes |
//! |-------|-------|-------|
//! | 0 | RXCUI1 | Child concept |
//! | 4 | RXCUI2 | Parent concept |
//! | 7 | RELA | Relationship type — import `isa` |
//! | 10 | SAB | Source — filter to `RXNORM` |
//! | 14 | SUPPRESS | `O` = obsolete — skip |

#[cfg(feature = "sqlite")]
use r2d2::Pool;
#[cfg(feature = "sqlite")]
use r2d2_sqlite::SqliteConnectionManager;

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::Path;

use crate::error::HtsError;
use crate::import::ImportStats;

// ── RxNorm constants ──────────────────────────────────────────────────────────

const RXNORM_URL: &str = "http://www.nlm.nih.gov/research/umls/rxnorm";
const RXNORM_NAME: &str = "RxNorm";
const RXNORM_TITLE: &str = "RxNorm — NLM Drug Terminology";

// ── Public entry point ────────────────────────────────────────────────────────

/// Import an RxNorm RRF distribution into the HTS database.
///
/// `path` may point to a **folder** containing `RXNCONSO.RRF` + `RXNREL.RRF`,
/// or a **ZIP file** containing those files inside the archive.
///
/// Returns [`ImportStats`] with concept counts and any non-fatal errors.
///
/// # ⚠️  LICENSE REQUIRED
/// Real RxNorm data requires acceptance of the NLM Terms of Service
/// (<https://www.nlm.nih.gov/databases/umls.html>).
#[cfg(feature = "sqlite")]
pub fn import_rxnorm_rrf(
    pool: &Pool<SqliteConnectionManager>,
    path: &Path,
    batch_size: usize,
    dry_run: bool,
) -> Result<ImportStats, HtsError> {
    let batch_size = batch_size.max(1);
    let (conso_bytes, rel_bytes) = read_rrf_files(path)?;

    let mut parse_errors: Vec<String> = Vec::new();
    let concepts = parse_concepts(BufReader::new(conso_bytes.as_slice()), &mut parse_errors)?;
    let edges = parse_relationships(
        BufReader::new(rel_bytes.as_slice()),
        &concepts,
        &mut parse_errors,
    )?;

    let mut stats = ImportStats {
        errors: parse_errors,
        ..Default::default()
    };

    if dry_run {
        stats.code_systems = 1;
        stats.concepts = concepts.len() as u32;
        eprintln!(
            "[rxnorm] dry-run — {} concepts, {} isa edges parsed, no DB writes",
            concepts.len(),
            edges.len()
        );
        return Ok(stats);
    }

    // ── Write to DB ────────────────────────────────────────────────────────
    let conn = pool
        .get()
        .map_err(|e| HtsError::Internal(format!("DB pool error: {e}")))?;

    let system_id = upsert_code_system(&conn)?;
    stats.code_systems = 1;

    let concept_list: Vec<(String, String)> = concepts.into_iter().collect();
    let total = concept_list.len();
    let total_batches = total.div_ceil(batch_size);

    for (batch_idx, batch) in concept_list.chunks(batch_size).enumerate() {
        insert_concept_batch(&conn, &system_id, batch)?;

        let inserted = ((batch_idx + 1) * batch_size).min(total);
        eprintln!(
            "[rxnorm] concept batch {}/{total_batches} — +{} concepts (total: {inserted})",
            batch_idx + 1,
            batch.len()
        );
    }
    stats.concepts = total as u32;

    let edge_total = edges.len();
    let edge_batches = edge_total.div_ceil(batch_size);
    for (batch_idx, batch) in edges.chunks(batch_size).enumerate() {
        insert_hierarchy_batch(&conn, &system_id, batch)?;
        eprintln!(
            "[rxnorm] hierarchy batch {}/{edge_batches} — +{} edges",
            batch_idx + 1,
            batch.len()
        );
    }

    Ok(stats)
}

// ── File reader ───────────────────────────────────────────────────────────────

/// Return raw bytes for `(RXNCONSO.RRF, RXNREL.RRF)` from a folder or ZIP.
fn read_rrf_files(path: &Path) -> Result<(Vec<u8>, Vec<u8>), HtsError> {
    if path.is_dir() {
        let conso = std::fs::read(path.join("RXNCONSO.RRF")).map_err(|e| {
            HtsError::InvalidRequest(format!(
                "Cannot read RXNCONSO.RRF in '{}': {e}",
                path.display()
            ))
        })?;
        let rel = std::fs::read(path.join("RXNREL.RRF")).map_err(|e| {
            HtsError::InvalidRequest(format!(
                "Cannot read RXNREL.RRF in '{}': {e}",
                path.display()
            ))
        })?;
        return Ok((conso, rel));
    }

    // Try as ZIP
    let file = std::fs::File::open(path)
        .map_err(|e| HtsError::InvalidRequest(format!("Cannot open '{}': {e}", path.display())))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| HtsError::InvalidRequest(format!("Invalid ZIP '{}': {e}", path.display())))?;

    let conso_idx = find_zip_entry(&mut archive, "rxnconso.rrf").ok_or_else(|| {
        HtsError::InvalidRequest(format!("RXNCONSO.RRF not found in '{}'", path.display()))
    })?;
    let rel_idx = find_zip_entry(&mut archive, "rxnrel.rrf").ok_or_else(|| {
        HtsError::InvalidRequest(format!("RXNREL.RRF not found in '{}'", path.display()))
    })?;

    let conso = read_zip_entry(&mut archive, conso_idx)?;
    let rel = read_zip_entry(&mut archive, rel_idx)?;
    Ok((conso, rel))
}

fn find_zip_entry(archive: &mut zip::ZipArchive<std::fs::File>, suffix: &str) -> Option<usize> {
    (0..archive.len()).find(|&i| {
        archive
            .by_index(i)
            .ok()
            .map(|e| e.name().to_ascii_lowercase().ends_with(suffix))
            .unwrap_or(false)
    })
}

fn read_zip_entry(
    archive: &mut zip::ZipArchive<std::fs::File>,
    index: usize,
) -> Result<Vec<u8>, HtsError> {
    use std::io::Read;
    let mut entry = archive
        .by_index(index)
        .map_err(|e| HtsError::InvalidRequest(format!("Cannot open ZIP entry: {e}")))?;
    let mut buf = Vec::new();
    entry
        .read_to_end(&mut buf)
        .map_err(|e| HtsError::InvalidRequest(format!("Cannot read ZIP entry: {e}")))?;
    Ok(buf)
}

// ── RRF parsers ───────────────────────────────────────────────────────────────

/// Parse `RXNCONSO.RRF` and return a map of `RXCUI → preferred display name`.
///
/// Filters to `SAB=RXNORM`, `LAT=ENG`, non-suppressed rows.
/// Preferred terms (`ISPREF=Y`) take priority over non-preferred ones.
/// Malformed lines (fewer than 15 fields) are skipped and recorded in `errors`.
fn parse_concepts(
    reader: impl BufRead,
    errors: &mut Vec<String>,
) -> Result<HashMap<String, String>, HtsError> {
    let mut concepts: HashMap<String, String> = HashMap::new();
    let mut preferred: HashMap<String, bool> = HashMap::new();

    for (line_no, line) in reader.lines().enumerate() {
        let line = line.map_err(|e| {
            HtsError::InvalidRequest(format!("RXNCONSO read error line {}: {e}", line_no + 1))
        })?;
        let line = line.trim_end_matches('|');

        let fields: Vec<&str> = line.split('|').collect();
        if fields.len() < 15 {
            errors.push(format!(
                "RXNCONSO.RRF line {}: expected ≥15 fields, got {} — skipped",
                line_no + 1,
                fields.len()
            ));
            continue;
        }

        let rxcui = fields[0];
        let lat = fields[1];
        let ispref = fields[6];
        let sab = fields[11];
        let str_val = fields[14];

        // Filter: English, RXNORM source only
        if lat != "ENG" || sab != "RXNORM" {
            continue;
        }
        // Skip suppressed (SUPPRESS column at index 16 when present)
        if fields.len() > 16 && fields[16] == "O" {
            continue;
        }
        // Skip empty display
        if str_val.is_empty() {
            continue;
        }

        let is_pref = ispref == "Y";
        let already_preferred = *preferred.get(rxcui).unwrap_or(&false);

        // Accept if: preferred term, OR no entry yet
        if is_pref || !concepts.contains_key(rxcui) {
            if !already_preferred || is_pref {
                concepts.insert(rxcui.to_string(), str_val.to_string());
                preferred.insert(rxcui.to_string(), is_pref);
            }
        }
    }

    Ok(concepts)
}

/// Parse `RXNREL.RRF` and return `(child_rxcui, parent_rxcui)` `isa` edges.
///
/// Filters to `SAB=RXNORM`, `RELA=isa`, non-suppressed rows.
/// Only edges where both concepts are in `active_concepts` are kept.
/// Malformed lines (fewer than 11 fields) are skipped and recorded in `errors`.
fn parse_relationships(
    reader: impl BufRead,
    active_concepts: &HashMap<String, String>,
    errors: &mut Vec<String>,
) -> Result<Vec<(String, String)>, HtsError> {
    let mut edges: Vec<(String, String)> = Vec::new();

    for (line_no, line) in reader.lines().enumerate() {
        let line = line.map_err(|e| {
            HtsError::InvalidRequest(format!("RXNREL read error line {}: {e}", line_no + 1))
        })?;
        let line = line.trim_end_matches('|');

        let fields: Vec<&str> = line.split('|').collect();
        if fields.len() < 11 {
            errors.push(format!(
                "RXNREL.RRF line {}: expected ≥11 fields, got {} — skipped",
                line_no + 1,
                fields.len()
            ));
            continue;
        }

        let rxcui1 = fields[0]; // child
        let rxcui2 = fields[4]; // parent
        let rela = fields[7];
        let sab = fields[10];

        if sab != "RXNORM" || rela != "isa" {
            continue;
        }
        // Skip suppressed
        if fields.len() > 14 && fields[14] == "O" {
            continue;
        }
        // Both ends must be in active concepts
        if !active_concepts.contains_key(rxcui1) || !active_concepts.contains_key(rxcui2) {
            continue;
        }

        edges.push((rxcui1.to_string(), rxcui2.to_string()));
    }

    // Deduplicate
    edges.sort_unstable();
    edges.dedup();
    Ok(edges)
}

// ── DB helpers ────────────────────────────────────────────────────────────────

#[cfg(feature = "sqlite")]
fn upsert_code_system(conn: &rusqlite::Connection) -> Result<String, HtsError> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    conn.execute(
        "INSERT OR IGNORE INTO code_systems \
         (id, url, version, name, title, status, content, created_at, updated_at) \
         VALUES (?1, ?2, 'current', ?3, ?4, 'active', 'complete', ?5, ?5)",
        rusqlite::params![id, RXNORM_URL, RXNORM_NAME, RXNORM_TITLE, now],
    )
    .map_err(|e| HtsError::Internal(format!("Upsert CodeSystem: {e}")))?;

    let system_id: String = conn
        .query_row(
            "SELECT id FROM code_systems WHERE url = ?1",
            rusqlite::params![RXNORM_URL],
            |row| row.get(0),
        )
        .map_err(|e| HtsError::Internal(format!("Fetch CodeSystem id: {e}")))?;

    Ok(system_id)
}

#[cfg(feature = "sqlite")]
fn insert_concept_batch(
    conn: &rusqlite::Connection,
    system_id: &str,
    batch: &[(String, String)],
) -> Result<(), HtsError> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| HtsError::Internal(format!("Begin transaction: {e}")))?;

    for (code, display) in batch {
        tx.execute(
            "INSERT OR REPLACE INTO concepts (system_id, code, display)
             VALUES (?1, ?2, ?3)",
            rusqlite::params![system_id, code, display],
        )
        .map_err(|e| HtsError::Internal(format!("Insert concept '{code}': {e}")))?;
    }

    tx.commit()
        .map_err(|e| HtsError::Internal(format!("Commit transaction: {e}")))?;
    Ok(())
}

#[cfg(feature = "sqlite")]
fn insert_hierarchy_batch(
    conn: &rusqlite::Connection,
    system_id: &str,
    batch: &[(String, String)],
) -> Result<(), HtsError> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| HtsError::Internal(format!("Begin hierarchy transaction: {e}")))?;

    for (child_code, parent_code) in batch {
        tx.execute(
            "INSERT OR IGNORE INTO concept_hierarchy (system_id, parent_code, child_code)
             VALUES (?1, ?2, ?3)",
            rusqlite::params![system_id, parent_code, child_code],
        )
        .map_err(|e| HtsError::Internal(format!("Insert edge {parent_code}->{child_code}: {e}")))?;
    }

    tx.commit()
        .map_err(|e| HtsError::Internal(format!("Commit hierarchy transaction: {e}")))?;
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    use super::*;
    use crate::backend::SqliteTerminologyBackend;

    // ── Synthetic RRF fixtures ────────────────────────────────────────────
    //
    // RXNCONSO.RRF — no header, pipe-delimited, 18 fields
    // Cols: RXCUI|LAT|TS|LUI|STT|SUI|ISPREF|RXAUI|SAUI|SCUI|SDUI|SAB|TTY|CODE|STR|SRL|SUPPRESS|CVF
    //
    // Concepts:
    //   1049502 = acetaminophen (ingredient, preferred)
    //   1049520 = ibuprofen     (ingredient, preferred)
    //   198444  = Tylenol       (brand name, preferred)
    //   1049527 = acetaminophen 325 MG Oral Tablet (SCD, preferred)
    //   9999999 = suppressed concept (should be skipped)
    // RXNCONSO.RRF columns (0-indexed):
    // 0=RXCUI 1=LAT 2=TS 3=LUI 4=STT 5=SUI 6=ISPREF 7=RXAUI 8=SAUI 9=SCUI
    // 10=SDUI 11=SAB 12=TTY 13=CODE 14=STR 15=SRL 16=SUPPRESS 17=CVF
    const CONSO_RRF: &str = "\
1049502|ENG|P|L0000001|PF|S0000001|Y|1049502|||1049502|RXNORM|IN|1049502|acetaminophen|0|N|\n\
1049520|ENG|P|L0000002|PF|S0000002|Y|1049520|||1049520|RXNORM|IN|1049520|ibuprofen|0|N|\n\
198444|ENG|P|L0000003|PF|S0000003|Y|198444|||198444|RXNORM|BN|198444|Tylenol|0|N|\n\
1049527|ENG|P|L0000004|PF|S0000004|Y|1049527|||1049527|RXNORM|SCD|1049527|acetaminophen 325 MG Oral Tablet|0|N|\n\
9999999|ENG|P|L0000005|PF|S0000005|Y|9999999|||9999999|RXNORM|IN|9999999|suppressed_drug|0|O|\n";

    // RXNREL.RRF — no header, pipe-delimited, 16 fields
    // Cols: RXCUI1|RXAUI1|STYPE1|REL|RXCUI2|RXAUI2|STYPE2|RELA|RUI|SRUI|SAB|SL|DIR|RG|SUPPRESS|CVF
    //
    // Edges (isa):
    //   198444  isa 1049502  (Tylenol is-a acetaminophen)
    //   1049527 isa 1049502  (acet 325mg tablet is-a acetaminophen)
    //   9999999 isa 1049502  (suppressed edge — skipped)
    const REL_RRF: &str = "\
198444||RXCUI|RN|1049502||RXCUI|isa|RUI001||RXNORM|||N|N|N|\n\
1049527||RXCUI|RN|1049502||RXCUI|isa|RUI002||RXNORM|||N|N|N|\n\
9999999||RXCUI|RN|1049502||RXCUI|isa|RUI003||RXNORM|||N|N|O|\n";

    fn count_rows(pool: &Pool<SqliteConnectionManager>, table: &str) -> i64 {
        let conn = pool.get().unwrap();
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
    }

    fn make_folder() -> tempfile::TempDir {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        std::fs::File::create(dir.path().join("RXNCONSO.RRF"))
            .unwrap()
            .write_all(CONSO_RRF.as_bytes())
            .unwrap();
        std::fs::File::create(dir.path().join("RXNREL.RRF"))
            .unwrap()
            .write_all(REL_RRF.as_bytes())
            .unwrap();
        dir
    }

    // ── Parser unit tests ─────────────────────────────────────────────────

    #[test]
    fn parse_concepts_returns_four_active_concepts() {
        let mut errors = Vec::new();
        let concepts = parse_concepts(BufReader::new(CONSO_RRF.as_bytes()), &mut errors).unwrap();
        // suppressed concept (9999999) is excluded
        assert_eq!(concepts.len(), 4);
        assert_eq!(concepts["1049502"], "acetaminophen");
        assert_eq!(concepts["198444"], "Tylenol");
        assert!(errors.is_empty());
    }

    #[test]
    fn parse_concepts_filters_non_rxnorm_source() {
        let data =
            "1111111|ENG|P|L1|PF|S1|Y|1111111|||1111111|SNOMEDCT_US|IN|1111111|SomeCode|0|N|\n";
        let mut errors = Vec::new();
        let concepts = parse_concepts(BufReader::new(data.as_bytes()), &mut errors).unwrap();
        assert!(concepts.is_empty());
    }

    #[test]
    fn parse_concepts_filters_non_english() {
        let data = "1111111|SPA|P|L1|PF|S1|Y|1111111|||1111111|RXNORM|IN|1111111|aspirina|0|N|\n";
        let mut errors = Vec::new();
        let concepts = parse_concepts(BufReader::new(data.as_bytes()), &mut errors).unwrap();
        assert!(concepts.is_empty());
    }

    #[test]
    fn parse_concepts_prefers_ispref_y() {
        // Two rows for same RXCUI; ISPREF=Y should win
        let data = "\
1049502|ENG|P|L1|PF|S1|N|1049502|||1049502|RXNORM|IN|1049502|acetaminophen alt|0|N|\n\
1049502|ENG|P|L1|PF|S2|Y|1049502|||1049502|RXNORM|IN|1049502|acetaminophen|0|N|\n";
        let mut errors = Vec::new();
        let concepts = parse_concepts(BufReader::new(data.as_bytes()), &mut errors).unwrap();
        assert_eq!(concepts["1049502"], "acetaminophen");
    }

    #[test]
    fn parse_relationships_returns_two_isa_edges() {
        let mut errors = Vec::new();
        let concepts = parse_concepts(BufReader::new(CONSO_RRF.as_bytes()), &mut errors).unwrap();
        let edges = parse_relationships(BufReader::new(REL_RRF.as_bytes()), &concepts, &mut errors)
            .unwrap();
        // 9999999 edge skipped (concept not in active set and suppress=O)
        assert_eq!(edges.len(), 2);
        assert!(edges.contains(&("198444".to_string(), "1049502".to_string())));
        assert!(edges.contains(&("1049527".to_string(), "1049502".to_string())));
    }

    #[test]
    fn parse_relationships_skips_non_isa_rela() {
        let concepts = {
            let mut m = HashMap::new();
            m.insert("A".to_string(), "Drug A".to_string());
            m.insert("B".to_string(), "Drug B".to_string());
            m
        };
        let data = "A||RXCUI|RO|B||RXCUI|ingredient_of|RUI001||RXNORM||||N|\n";
        let mut errors = Vec::new();
        let edges =
            parse_relationships(BufReader::new(data.as_bytes()), &concepts, &mut errors).unwrap();
        assert!(edges.is_empty());
    }

    #[test]
    fn import_rxnorm_malformed_conso_line_recorded_in_errors() {
        // CONSO with one valid line (18 fields) and one short line (3 fields)
        let data = "\
1049502|ENG|P|L0000001|PF|S0000001|Y|1049502|||1049502|RXNORM|IN|1049502|acetaminophen|0|N|\n\
BAD|LINE|ONLY_THREE_FIELDS\n";
        let mut errors = Vec::new();
        let concepts = parse_concepts(BufReader::new(data.as_bytes()), &mut errors).unwrap();
        assert_eq!(concepts.len(), 1, "only the valid line should be parsed");
        assert_eq!(errors.len(), 1, "one error for the malformed line");
        assert!(
            errors[0].contains("line 2"),
            "error should mention line number: {}",
            errors[0]
        );
    }

    // ── Integration tests ─────────────────────────────────────────────────

    #[test]
    fn import_rxnorm_dry_run_returns_correct_counts() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();
        let dir = make_folder();

        let stats = import_rxnorm_rrf(&pool, dir.path(), 500, true).unwrap();

        assert_eq!(stats.code_systems, 1);
        assert_eq!(stats.concepts, 4);
        assert!(stats.errors.is_empty());
    }

    #[test]
    fn import_rxnorm_dry_run_does_not_write_to_db() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();
        let dir = make_folder();

        import_rxnorm_rrf(&pool, dir.path(), 500, true).unwrap();

        assert_eq!(count_rows(&pool, "code_systems"), 0);
        assert_eq!(count_rows(&pool, "concepts"), 0);
    }

    #[test]
    fn import_rxnorm_live_writes_to_db() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();
        let dir = make_folder();

        let stats = import_rxnorm_rrf(&pool, dir.path(), 500, false).unwrap();

        assert_eq!(stats.code_systems, 1);
        assert_eq!(stats.concepts, 4);
        assert_eq!(count_rows(&pool, "code_systems"), 1);
        assert_eq!(count_rows(&pool, "concepts"), 4);
        assert_eq!(count_rows(&pool, "concept_hierarchy"), 2);
    }

    #[test]
    fn import_rxnorm_idempotent_reimport() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();
        let dir = make_folder();

        import_rxnorm_rrf(&pool, dir.path(), 500, false).unwrap();
        import_rxnorm_rrf(&pool, dir.path(), 500, false).unwrap();

        assert_eq!(count_rows(&pool, "code_systems"), 1);
        assert_eq!(count_rows(&pool, "concepts"), 4);
        assert_eq!(count_rows(&pool, "concept_hierarchy"), 2);
    }

    #[test]
    fn import_rxnorm_batching_preserves_all_concepts() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();
        let dir = make_folder();

        // batch_size=2 forces multiple batches across 4 concepts
        let stats = import_rxnorm_rrf(&pool, dir.path(), 2, false).unwrap();

        assert_eq!(stats.concepts, 4);
        assert_eq!(count_rows(&pool, "concepts"), 4);
        assert_eq!(count_rows(&pool, "concept_hierarchy"), 2);
    }

    #[test]
    fn import_rxnorm_missing_folder_returns_error() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();
        let result = import_rxnorm_rrf(&pool, Path::new("/nonexistent/rxnorm"), 500, false);
        assert!(result.is_err());
    }

    #[test]
    fn import_rxnorm_lookup_drug_code() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();
        let dir = make_folder();

        import_rxnorm_rrf(&pool, dir.path(), 500, false).unwrap();

        let conn = pool.get().unwrap();
        let display: String = conn
            .query_row(
                "SELECT display FROM concepts WHERE code = ?1",
                rusqlite::params!["1049527"],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(display, "acetaminophen 325 MG Oral Tablet");
    }

    #[test]
    fn import_rxnorm_from_zip() {
        use std::io::Write;

        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("rxnorm_full_current.zip");

        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let opts = zip::write::FileOptions::default();
            zip.start_file("rrf/RXNCONSO.RRF", opts).unwrap();
            zip.write_all(CONSO_RRF.as_bytes()).unwrap();
            zip.start_file("rrf/RXNREL.RRF", opts).unwrap();
            zip.write_all(REL_RRF.as_bytes()).unwrap();
            zip.finish().unwrap();
        }

        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool().clone();

        let stats = import_rxnorm_rrf(&pool, &zip_path, 500, false).unwrap();
        assert_eq!(stats.concepts, 4);
        assert_eq!(count_rows(&pool, "concepts"), 4);
        assert_eq!(count_rows(&pool, "concept_hierarchy"), 2);
    }
}
