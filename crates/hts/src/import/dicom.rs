//! DICOM (Digital Imaging and Communications in Medicine) code importer.
//!
//! Parses the NEMA DICOM Part 16 code table (exported as CSV or tab-delimited)
//! and imports all code meanings into the HTS normalized schema.
//!
//! # No license required
//!
//! NEMA makes the DICOM standard freely available. The code meanings used in FHIR
//! imaging resources (`ImagingStudy`, `ImagingSelection`) are published in
//! DICOM PS3.16 (Content Mapping Resource). Download from:
//! <https://www.dicomstandard.org/current/>
//!
//! # File format
//!
//! The importer accepts a CSV or tab-delimited file with the DICOM code table.
//! The expected columns are:
//!
//! ```text
//! CodeValue,CodingSchemeDesignator,CodeMeaning
//! 001,DCM,Quantitative Immunofluorescence
//! 002,DCM,Qualitative Immunofluorescence
//! ```
//!
//! - A header row is optional; it is skipped when the first column does not look
//!   like a DICOM code (i.e. contains letters other than `DCM` or similar).
//! - Both comma (`,`) and tab (`\t`) delimiters are supported; the importer
//!   auto-detects from the first line.
//! - Lines with fewer than two non-empty columns are skipped.
//!
//! # Hierarchy
//!
//! DICOM codes form a flat list. All codes are placed as children of a virtual
//! root `DCM` concept in the `concept_hierarchy` table.

#[cfg(feature = "sqlite")]
use r2d2::Pool;
#[cfg(feature = "sqlite")]
use r2d2_sqlite::SqliteConnectionManager;

use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use crate::error::HtsError;
use crate::import::ImportStats;

// ── Constants ─────────────────────────────────────────────────────────────────

const DICOM_URL: &str = "http://dicom.nema.org/resources/ontology/DCM";
const DICOM_NAME: &str = "DCM";
const DICOM_TITLE: &str = "DICOM Controlled Terminology";
const ROOT_CODE: &str = "DCM";

// ── Data types ────────────────────────────────────────────────────────────────

#[derive(Debug)]
struct DicomConcept {
    /// DICOM code value, e.g. `121049`.
    code: String,
    /// Human-readable code meaning.
    display: String,
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Import a DICOM Part 16 code table (CSV or TSV) into the HTS database.
///
/// `path` may point to:
/// - A raw `.csv` or `.txt` / `.tsv` file, or
/// - A `.zip` archive containing such a file with "dicom" or "dcm" in the name.
#[cfg(feature = "sqlite")]
pub fn import_dicom(
    pool: &Pool<SqliteConnectionManager>,
    path: &Path,
    batch_size: usize,
    dry_run: bool,
) -> Result<ImportStats, HtsError> {
    let batch_size = batch_size.max(1);
    let text = read_text(path)?;
    let (concepts, errors) = parse_dicom_table(&text);

    let mut stats = ImportStats {
        errors,
        ..Default::default()
    };

    if dry_run {
        stats.code_systems = 1;
        stats.concepts = concepts.len() as u32;
        eprintln!(
            "[dicom] dry-run — {} codes parsed, no DB writes",
            concepts.len()
        );
        return Ok(stats);
    }

    let conn = pool
        .get()
        .map_err(|e| HtsError::Internal(format!("DB pool error: {e}")))?;

    let system_id = upsert_code_system(&conn)?;
    stats.code_systems = 1;

    // Virtual root so hierarchy edges have a valid parent.
    conn.execute(
        "INSERT OR IGNORE INTO concepts (system_id, code, display, definition) \
         VALUES (?1, ?2, ?3, 'header')",
        rusqlite::params![system_id, ROOT_CODE, DICOM_TITLE],
    )
    .map_err(|e| HtsError::Internal(format!("Insert virtual root: {e}")))?;

    let total = concepts.len();
    let total_batches = total.div_ceil(batch_size);

    for (batch_idx, batch) in concepts.chunks(batch_size).enumerate() {
        insert_concept_batch(&conn, &system_id, batch)?;

        let inserted = ((batch_idx + 1) * batch_size).min(total);
        eprintln!(
            "[dicom] batch {}/{total_batches} — +{} codes (total: {inserted})",
            batch_idx + 1,
            batch.len()
        );
    }

    // All codes hang off the virtual root.
    insert_flat_hierarchy(&conn, &system_id, &concepts)?;

    stats.concepts = total as u32;
    Ok(stats)
}

// ── File reader ───────────────────────────────────────────────────────────────

fn read_text(path: &Path) -> Result<String, HtsError> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    if ext == "zip" {
        read_text_from_zip(path)
    } else {
        std::fs::read_to_string(path)
            .map_err(|e| HtsError::InvalidRequest(format!("Cannot read '{}': {e}", path.display())))
    }
}

fn read_text_from_zip(path: &Path) -> Result<String, HtsError> {
    let file = std::fs::File::open(path).map_err(|e| {
        HtsError::InvalidRequest(format!("Cannot open ZIP '{}': {e}", path.display()))
    })?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| HtsError::InvalidRequest(format!("Invalid ZIP '{}': {e}", path.display())))?;

    let best_index = (0..archive.len())
        .find_map(|i| {
            let entry = archive.by_index(i).ok()?;
            let name = entry.name().to_ascii_lowercase();
            let is_data_file =
                name.ends_with(".csv") || name.ends_with(".tsv") || name.ends_with(".txt");
            if is_data_file && (name.contains("dicom") || name.contains("dcm")) {
                Some(i)
            } else {
                None
            }
        })
        .ok_or_else(|| {
            HtsError::InvalidRequest(format!(
                "No DICOM code table found inside ZIP '{}'. \
                 Expected a CSV/TSV file with 'dicom' or 'dcm' in the name.",
                path.display()
            ))
        })?;

    let mut entry = archive
        .by_index(best_index)
        .map_err(|e| HtsError::InvalidRequest(format!("Cannot read ZIP entry: {e}")))?;
    let mut buf = String::new();
    entry
        .read_to_string(&mut buf)
        .map_err(|e| HtsError::InvalidRequest(format!("Cannot read text from ZIP: {e}")))?;
    Ok(buf)
}

// ── Parser ────────────────────────────────────────────────────────────────────

/// Auto-detect delimiter: tab if the first non-empty line contains a tab, else comma.
fn detect_delimiter(text: &str) -> char {
    for line in text.lines() {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            return if trimmed.contains('\t') { '\t' } else { ',' };
        }
    }
    ','
}

/// Parse the DICOM code table into a flat list of concepts.
///
/// Returns `(concepts, non_fatal_errors)`.
fn parse_dicom_table(text: &str) -> (Vec<DicomConcept>, Vec<String>) {
    let delimiter = detect_delimiter(text);
    let mut concepts = Vec::new();
    let mut errors = Vec::new();
    let mut lines = BufReader::new(text.as_bytes()).lines().enumerate();

    // Skip the header row if the first column is not numeric-looking.
    if let Some((_, Ok(first))) = lines.next() {
        let trimmed = first.trim();
        let first_col = trimmed.split(delimiter).next().unwrap_or("").trim();
        if looks_like_code(first_col) {
            process_line(0, trimmed, delimiter, &mut concepts, &mut errors);
        }
        // else: header — skip
    }

    for (line_num, line) in lines {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        process_line(line_num + 1, trimmed, delimiter, &mut concepts, &mut errors);
    }

    (concepts, errors)
}

/// Returns `true` if the string looks like a DICOM code value.
///
/// DICOM code values (Part 16) are 1–16 characters containing digits, uppercase
/// letters, and punctuation — never lowercase letters. Header field names like
/// "CodeValue" or "CodeMeaning" always contain lowercase, so this check reliably
/// distinguishes data rows from header rows.
fn looks_like_code(s: &str) -> bool {
    !s.is_empty() && s.len() <= 16 && !s.chars().any(|c| c.is_ascii_lowercase())
}

/// Parse a single delimited data line and push the result into `concepts`,
/// or push a message into `errors` if the line is malformed or incomplete.
fn process_line(
    line_num: usize,
    line: &str,
    delimiter: char,
    concepts: &mut Vec<DicomConcept>,
    errors: &mut Vec<String>,
) {
    let cols: Vec<&str> = line.splitn(3, delimiter).collect();

    // Support both 2-column (code, meaning) and 3-column (code, scheme, meaning) formats.
    let (code_col, meaning_col) = if cols.len() >= 3 {
        // 3-column: CodeValue, CodingSchemeDesignator, CodeMeaning
        (cols[0].trim(), cols[2].trim())
    } else if cols.len() == 2 {
        // 2-column: CodeValue, CodeMeaning
        (cols[0].trim(), cols[1].trim())
    } else {
        errors.push(format!("line {line_num}: too few columns — skipped"));
        return;
    };

    if code_col.is_empty() {
        errors.push(format!("line {line_num}: empty code — skipped"));
        return;
    }
    if meaning_col.is_empty() {
        errors.push(format!(
            "line {line_num}: empty meaning for code '{code_col}' — skipped"
        ));
        return;
    }

    concepts.push(DicomConcept {
        code: code_col.to_string(),
        display: meaning_col.to_string(),
    });
}

// ── DB helpers ────────────────────────────────────────────────────────────────

/// Insert the DICOM CodeSystem row if absent, then return its `id`.
#[cfg(feature = "sqlite")]
fn upsert_code_system(conn: &rusqlite::Connection) -> Result<String, HtsError> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    conn.execute(
        "INSERT OR IGNORE INTO code_systems \
         (id, url, version, name, title, status, content, created_at, updated_at) \
         VALUES (?1, ?2, 'current', ?3, ?4, 'active', 'complete', ?5, ?5)",
        rusqlite::params![id, DICOM_URL, DICOM_NAME, DICOM_TITLE, now],
    )
    .map_err(|e| HtsError::Internal(format!("Upsert CodeSystem: {e}")))?;

    let system_id: String = conn
        .query_row(
            "SELECT id FROM code_systems WHERE url = ?1",
            rusqlite::params![DICOM_URL],
            |row| row.get(0),
        )
        .map_err(|e| HtsError::Internal(format!("Fetch CodeSystem id: {e}")))?;

    Ok(system_id)
}

/// Upsert one batch of DICOM concepts inside a single transaction.
#[cfg(feature = "sqlite")]
fn insert_concept_batch(
    conn: &rusqlite::Connection,
    system_id: &str,
    batch: &[DicomConcept],
) -> Result<(), HtsError> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| HtsError::Internal(format!("Begin transaction: {e}")))?;

    for c in batch {
        tx.execute(
            "INSERT OR REPLACE INTO concepts (system_id, code, display) VALUES (?1, ?2, ?3)",
            rusqlite::params![system_id, c.code, c.display],
        )
        .map_err(|e| HtsError::Internal(format!("Insert concept '{}': {e}", c.code)))?;
    }

    tx.commit()
        .map_err(|e| HtsError::Internal(format!("Commit: {e}")))?;
    Ok(())
}

/// Insert one `ROOT_CODE → code` edge per concept (flat enumeration).
#[cfg(feature = "sqlite")]
fn insert_flat_hierarchy(
    conn: &rusqlite::Connection,
    system_id: &str,
    concepts: &[DicomConcept],
) -> Result<(), HtsError> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| HtsError::Internal(format!("Begin hierarchy transaction: {e}")))?;

    for c in concepts {
        tx.execute(
            "INSERT OR IGNORE INTO concept_hierarchy (system_id, parent_code, child_code) \
             VALUES (?1, ?2, ?3)",
            rusqlite::params![system_id, ROOT_CODE, c.code],
        )
        .map_err(|e| HtsError::Internal(format!("Insert hierarchy DCM->{}: {e}", c.code)))?;
    }

    tx.commit()
        .map_err(|e| HtsError::Internal(format!("Commit hierarchy: {e}")))?;
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_CSV: &str = "\
CodeValue,CodingSchemeDesignator,CodeMeaning\n\
001,DCM,Quantitative Immunofluorescence\n\
002,DCM,Qualitative Immunofluorescence\n\
003,DCM,Threshold\n\
";

    const SAMPLE_TSV: &str = "\
CodeValue\tCodingSchemeDesignator\tCodeMeaning\n\
001\tDCM\tQuantitative Immunofluorescence\n\
002\tDCM\tQualitative Immunofluorescence\n\
";

    const SAMPLE_2COL: &str = "\
001,Quantitative Immunofluorescence\n\
002,Qualitative Immunofluorescence\n\
";

    #[test]
    fn parse_csv_3col_skips_header() {
        let (concepts, errors) = parse_dicom_table(SAMPLE_CSV);
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
        assert_eq!(concepts.len(), 3);
    }

    #[test]
    fn parse_tsv_3col() {
        let (concepts, errors) = parse_dicom_table(SAMPLE_TSV);
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
        assert_eq!(concepts.len(), 2);
    }

    #[test]
    fn parse_2col_no_header() {
        let (concepts, errors) = parse_dicom_table(SAMPLE_2COL);
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
        assert_eq!(concepts.len(), 2);
        assert_eq!(concepts[0].code, "001");
        assert_eq!(concepts[0].display, "Quantitative Immunofluorescence");
    }

    #[test]
    fn parse_extracts_code_meaning() {
        let (concepts, _) = parse_dicom_table(SAMPLE_CSV);
        assert_eq!(concepts[0].code, "001");
        assert_eq!(concepts[0].display, "Quantitative Immunofluorescence");
    }

    #[test]
    fn detect_delimiter_tab() {
        assert_eq!(detect_delimiter("a\tb\tc\n"), '\t');
    }

    #[test]
    fn detect_delimiter_comma() {
        assert_eq!(detect_delimiter("a,b,c\n"), ',');
    }

    #[cfg(feature = "sqlite")]
    mod integration {
        use super::*;
        use crate::backend::SqliteTerminologyBackend;
        use std::io::Write;

        fn count(pool: &Pool<SqliteConnectionManager>, table: &str) -> i64 {
            pool.get()
                .unwrap()
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap()
        }

        fn make_csv_file(content: &str) -> tempfile::NamedTempFile {
            let mut f = tempfile::NamedTempFile::with_suffix(".csv").unwrap();
            f.write_all(content.as_bytes()).unwrap();
            f
        }

        #[test]
        fn dry_run_does_not_write() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let f = make_csv_file(SAMPLE_CSV);
            let stats = import_dicom(backend.pool(), f.path(), 500, true).unwrap();
            assert_eq!(stats.code_systems, 1);
            assert_eq!(stats.concepts, 3);
            assert_eq!(count(backend.pool(), "code_systems"), 0);
        }

        #[test]
        fn live_import_writes_concepts_and_hierarchy() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let f = make_csv_file(SAMPLE_CSV);
            let stats = import_dicom(backend.pool(), f.path(), 500, false).unwrap();
            assert_eq!(stats.code_systems, 1);
            assert_eq!(stats.concepts, 3);
            // virtual root + 3 codes
            assert_eq!(count(backend.pool(), "concepts"), 4);
            // 3 flat hierarchy edges
            assert_eq!(count(backend.pool(), "concept_hierarchy"), 3);
        }

        #[test]
        fn idempotent_reimport() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let f = make_csv_file(SAMPLE_CSV);
            import_dicom(backend.pool(), f.path(), 500, false).unwrap();
            import_dicom(backend.pool(), f.path(), 500, false).unwrap();
            assert_eq!(count(backend.pool(), "code_systems"), 1);
            assert_eq!(count(backend.pool(), "concepts"), 4);
            assert_eq!(count(backend.pool(), "concept_hierarchy"), 3);
        }
    }
}
