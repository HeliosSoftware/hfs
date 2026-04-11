//! ICD-9-CM flat-text importer.
//!
//! Parses the CMS ICD-9-CM pipe-delimited text distribution and imports all
//! diagnosis codes with an inferred parent–child hierarchy into the HTS
//! normalized schema.
//!
//! # No license required
//!
//! ICD-9-CM is a US government work in the public domain.  The final release
//! was FY2015 (the last year before the ICD-10-CM transition on Oct 1, 2015).
//! Download from the CMS archive:
//! <https://www.cms.gov/medicare/coding-billing/icd-10-codes/icd-9-cm-diagnosis-procedure-codes-abbreviated-and-full-code-titles>
//!
//! # File format
//!
//! The CMS distribution is a ZIP containing pipe-delimited text files:
//!
//! ```text
//! 0010|Cholera due to vibrio cholerae
//! 00100|Cholera due to vibrio cholerae
//! 00101|Cholera due to vibrio cholerae el tor
//! ```
//!
//! Codes are stored **without** the decimal point.  This importer inserts it
//! for display:
//!
//! | Raw code | Display code |
//! |----------|-------------|
//! | `001`    | `001`        |
//! | `0010`   | `001.0`      |
//! | `00100`  | `001.00`     |
//! | `E800`   | `E800`       |
//! | `E8000`  | `E800.0`     |
//! | `V01`    | `V01`        |
//! | `V010`   | `V01.0`      |
//!
//! # Hierarchy
//!
//! # Hierarchy
//!
//! Hierarchy is inferred from the display code:
//! - No dot → top-level category; parent is the virtual root `ICD-9-CM`.
//! - One char after dot (e.g., `001.0`) → parent is the part before the dot (`001`).
//! - Two chars after dot (e.g., `001.00`) → parent is the code without the last char (`001.0`).
//!
//! # Known limitations
//!
//! - **Diagnosis codes only.**  CMS also distributes procedure/surgery codes in
//!   `*_DESC_LONG_SG*.txt`.  This importer accepts any pipe-delimited `.txt`
//!   file, so procedure codes will import but share the same CodeSystem URL.
//!   If separate CodeSystems are needed, import the files separately into
//!   different databases.
//! - **No chapter/section groupers.**  ICD-9-CM has named chapters (e.g.,
//!   "Infectious And Parasitic Diseases: 001–139") but CMS flat files do not
//!   include them.  Top-level 3-digit codes are placed directly under the
//!   virtual root `ICD-9-CM` rather than under a chapter concept.
//! - **V-codes with 4-char bases are not handled specially.**  `V700` is treated
//!   as a sub-code of `V70` (3-char base), which matches ICD-9-CM conventions.

#[cfg(feature = "sqlite")]
use r2d2::Pool;
#[cfg(feature = "sqlite")]
use r2d2_sqlite::SqliteConnectionManager;

use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use crate::error::HtsError;
use crate::import::ImportStats;

// ── Constants ─────────────────────────────────────────────────────────────────

const ICD9CM_URL: &str = "http://hl7.org/fhir/sid/icd-9-cm";
const ICD9CM_NAME: &str = "ICD-9-CM";
const ICD9CM_TITLE: &str =
    "ICD-9-CM (International Classification of Diseases, 9th Revision, Clinical Modification)";
const ICD9CM_VERSION: &str = "2015";
/// Virtual root code — all top-level categories hang off this.
const ROOT_CODE: &str = "ICD-9-CM";

// ── Data types ────────────────────────────────────────────────────────────────

#[derive(Debug)]
struct Icd9Concept {
    /// FHIR-facing code with decimal point inserted (e.g. `"001.0"`).
    code: String,
    /// Human-readable description from the source file.
    display: String,
    /// Parent code (also with decimal) or `None` when the parent is the
    /// virtual root `ICD-9-CM`.
    parent: Option<String>,
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Import a CMS ICD-9-CM distribution into the HTS database.
///
/// `path` may point to:
/// - The raw pipe-delimited `.txt` file, or
/// - A `.zip` archive containing a `*_DESC_LONG_DX*.txt` (or any single `.txt`) file.
#[cfg(feature = "sqlite")]
pub fn import_icd9_cm(
    pool: &Pool<SqliteConnectionManager>,
    path: &Path,
    batch_size: usize,
    dry_run: bool,
) -> Result<ImportStats, HtsError> {
    let batch_size = batch_size.max(1);
    let text = read_text(path)?;

    let (concepts, errors) = parse_pipe_delimited(&text);

    let mut stats = ImportStats {
        errors,
        ..Default::default()
    };

    if dry_run {
        stats.code_systems = 1;
        stats.concepts = concepts.len() as u32;
        eprintln!(
            "[icd9-cm] dry-run — {} concepts parsed, no DB writes",
            concepts.len()
        );
        return Ok(stats);
    }

    let conn = pool
        .get()
        .map_err(|e| HtsError::Internal(format!("DB pool error: {e}")))?;

    let system_id = upsert_code_system(&conn)?;
    stats.code_systems = 1;

    // Insert the virtual root concept first so hierarchy edges can reference it.
    conn.execute(
        "INSERT OR IGNORE INTO concepts (system_id, code, display, definition) \
         VALUES (?1, ?2, ?3, 'header')",
        rusqlite::params![system_id, ROOT_CODE, ICD9CM_TITLE],
    )
    .map_err(|e| HtsError::Internal(format!("Insert virtual root: {e}")))?;

    let total = concepts.len();
    let total_batches = total.div_ceil(batch_size);

    for (batch_idx, batch) in concepts.chunks(batch_size).enumerate() {
        insert_concept_batch(&conn, &system_id, batch)?;
        insert_hierarchy_batch(&conn, &system_id, batch)?;

        let inserted = ((batch_idx + 1) * batch_size).min(total);
        eprintln!(
            "[icd9-cm] batch {}/{total_batches} — +{} concepts (total: {inserted})",
            batch_idx + 1,
            batch.len()
        );
    }

    stats.concepts = total as u32;
    Ok(stats)
}

// ── File reader ───────────────────────────────────────────────────────────────

/// Read the pipe-delimited text from either a raw `.txt` file or a `.zip`.
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

/// Extract the first suitable text file from a ZIP archive.
///
/// Preference order:
/// 1. A file whose name contains `_desc_long_dx` (CMS long-description file).
/// 2. A file whose name contains `_desc_short_dx` (CMS short-description file).
/// 3. The first `.txt` file that is not a readme or license.
fn read_text_from_zip(path: &Path) -> Result<String, HtsError> {
    let file = std::fs::File::open(path).map_err(|e| {
        HtsError::InvalidRequest(format!("Cannot open ZIP '{}': {e}", path.display()))
    })?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| HtsError::InvalidRequest(format!("Invalid ZIP '{}': {e}", path.display())))?;

    // Score each entry: 2 = long-desc match, 1 = short-desc match, 0 = other .txt
    let best_index = (0..archive.len())
        .filter_map(|i| {
            let entry = archive.by_index(i).ok()?;
            let name = entry.name().to_ascii_lowercase();
            if !name.ends_with(".txt") {
                return None;
            }
            // Skip obvious non-data files
            if name.contains("readme") || name.contains("license") || name.contains("read_me") {
                return None;
            }
            let score = if name.contains("_desc_long_dx") {
                2u8
            } else if name.contains("_desc_short_dx") {
                1
            } else {
                0
            };
            Some((i, score))
        })
        .max_by_key(|&(_, score)| score)
        .map(|(i, _)| i)
        .ok_or_else(|| {
            HtsError::InvalidRequest(format!(
                "No suitable text file found inside ZIP '{}'. \
                 Expected a pipe-delimited '*_DESC_LONG_DX*.txt' file.",
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

/// Parse a pipe-delimited ICD-9-CM text file into [`Icd9Concept`]s.
///
/// Expected line format: `code|description`
///
/// Lines that do not contain `|` or have an empty code are skipped and
/// recorded as non-fatal errors.
fn parse_pipe_delimited(text: &str) -> (Vec<Icd9Concept>, Vec<String>) {
    let mut concepts = Vec::new();
    let mut errors = Vec::new();

    for (line_num, line) in BufReader::new(text.as_bytes()).lines().enumerate() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let Some(pipe_pos) = line.find('|') else {
            errors.push(format!(
                "line {}: no pipe separator — skipped: {line}",
                line_num + 1
            ));
            continue;
        };

        let raw_code = line[..pipe_pos].trim();
        let description = line[pipe_pos + 1..].trim();

        if raw_code.is_empty() {
            errors.push(format!("line {}: empty code — skipped", line_num + 1));
            continue;
        }

        let code = insert_dot(raw_code);
        let parent = parent_of(&code);

        concepts.push(Icd9Concept {
            code,
            display: description.to_string(),
            parent,
        });
    }

    (concepts, errors)
}

// ── Code helpers ──────────────────────────────────────────────────────────────

/// Insert the decimal point into a raw ICD-9-CM code.
///
/// The base length (before the decimal) is:
/// - 4 for E-codes (`E` prefix) — e.g., `E800` is a base code, `E8000` → `E800.0`
/// - 3 for all other codes (numeric and V-codes)
fn insert_dot(raw: &str) -> String {
    let base = if raw.starts_with('E') || raw.starts_with('e') {
        4
    } else {
        3
    };
    if raw.len() <= base {
        raw.to_string()
    } else {
        format!("{}.{}", &raw[..base], &raw[base..])
    }
}

/// Infer the parent code from a display code (with decimal).
///
/// Returns `None` for top-level categories (no dot), which means the parent
/// is the virtual root and the hierarchy batch will use `ROOT_CODE`.
fn parent_of(code: &str) -> Option<String> {
    match code.find('.') {
        None => None, // top-level → parent is virtual root
        Some(dot) => {
            let after_dot = &code[dot + 1..];
            if after_dot.len() <= 1 {
                // e.g. "001.0" → parent is "001"
                Some(code[..dot].to_string())
            } else {
                // e.g. "001.00" → parent is "001.0"
                Some(code[..code.len() - 1].to_string())
            }
        }
    }
}

// ── DB helpers ────────────────────────────────────────────────────────────────

#[cfg(feature = "sqlite")]
fn upsert_code_system(conn: &rusqlite::Connection) -> Result<String, HtsError> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    conn.execute(
        "INSERT OR IGNORE INTO code_systems \
         (id, url, version, name, title, status, content, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, 'active', 'complete', ?6, ?6)",
        rusqlite::params![
            id,
            ICD9CM_URL,
            ICD9CM_VERSION,
            ICD9CM_NAME,
            ICD9CM_TITLE,
            now
        ],
    )
    .map_err(|e| HtsError::Internal(format!("Upsert CodeSystem: {e}")))?;

    let system_id: String = conn
        .query_row(
            "SELECT id FROM code_systems WHERE url = ?1",
            rusqlite::params![ICD9CM_URL],
            |row| row.get(0),
        )
        .map_err(|e| HtsError::Internal(format!("Fetch CodeSystem id: {e}")))?;

    Ok(system_id)
}

#[cfg(feature = "sqlite")]
fn insert_concept_batch(
    conn: &rusqlite::Connection,
    system_id: &str,
    batch: &[Icd9Concept],
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

#[cfg(feature = "sqlite")]
fn insert_hierarchy_batch(
    conn: &rusqlite::Connection,
    system_id: &str,
    batch: &[Icd9Concept],
) -> Result<(), HtsError> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| HtsError::Internal(format!("Begin hierarchy transaction: {e}")))?;

    for c in batch {
        let parent = c.parent.as_deref().unwrap_or(ROOT_CODE);
        tx.execute(
            "INSERT OR IGNORE INTO concept_hierarchy (system_id, parent_code, child_code) \
             VALUES (?1, ?2, ?3)",
            rusqlite::params![system_id, parent, c.code],
        )
        .map_err(|e| HtsError::Internal(format!("Insert hierarchy {parent}->{}: {e}", c.code)))?;
    }

    tx.commit()
        .map_err(|e| HtsError::Internal(format!("Commit hierarchy: {e}")))?;
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── insert_dot ────────────────────────────────────────────────────────────

    #[test]
    fn dot_not_inserted_for_3_char_numeric() {
        assert_eq!(insert_dot("001"), "001");
    }

    #[test]
    fn dot_inserted_for_4_char_numeric() {
        assert_eq!(insert_dot("0010"), "001.0");
    }

    #[test]
    fn dot_inserted_for_5_char_numeric() {
        assert_eq!(insert_dot("00100"), "001.00");
    }

    #[test]
    fn dot_not_inserted_for_e_code_base() {
        assert_eq!(insert_dot("E800"), "E800");
    }

    #[test]
    fn dot_inserted_for_e_code_sub() {
        assert_eq!(insert_dot("E8000"), "E800.0");
    }

    #[test]
    fn dot_not_inserted_for_v_code_base() {
        assert_eq!(insert_dot("V01"), "V01");
    }

    #[test]
    fn dot_inserted_for_v_code_sub() {
        assert_eq!(insert_dot("V010"), "V01.0");
    }

    // ── parent_of ─────────────────────────────────────────────────────────────

    #[test]
    fn parent_of_top_level_is_none() {
        assert_eq!(parent_of("001"), None);
        assert_eq!(parent_of("E800"), None);
        assert_eq!(parent_of("V01"), None);
    }

    #[test]
    fn parent_of_one_decimal_digit() {
        assert_eq!(parent_of("001.0"), Some("001".to_string()));
        assert_eq!(parent_of("E800.0"), Some("E800".to_string()));
        assert_eq!(parent_of("V01.0"), Some("V01".to_string()));
    }

    #[test]
    fn parent_of_two_decimal_digits() {
        assert_eq!(parent_of("001.00"), Some("001.0".to_string()));
        assert_eq!(parent_of("E800.01"), Some("E800.0".to_string()));
    }

    // ── parse_pipe_delimited ──────────────────────────────────────────────────

    const SAMPLE: &str = "\
001|Cholera\n\
0010|Cholera due to vibrio cholerae\n\
00100|Cholera due to vibrio cholerae\n\
00101|Cholera due to vibrio cholerae el tor\n\
E800|Railway accidents\n\
E8000|Railway accident injuring occupant of railway vehicle\n\
V01|Contact with or exposure to communicable diseases\n\
";

    #[test]
    fn parse_returns_correct_count() {
        let (concepts, errors) = parse_pipe_delimited(SAMPLE);
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
        assert_eq!(concepts.len(), 7);
    }

    #[test]
    fn parse_inserts_dot_in_codes() {
        let (concepts, _) = parse_pipe_delimited(SAMPLE);
        let codes: Vec<&str> = concepts.iter().map(|c| c.code.as_str()).collect();
        assert!(codes.contains(&"001.0"), "expected 001.0 in {codes:?}");
        assert!(codes.contains(&"001.00"), "expected 001.00 in {codes:?}");
        assert!(codes.contains(&"E800.0"), "expected E800.0 in {codes:?}");
    }

    #[test]
    fn parse_sets_correct_parents() {
        let (concepts, _) = parse_pipe_delimited(SAMPLE);
        let find = |code: &str| concepts.iter().find(|c| c.code == code).unwrap();

        assert_eq!(find("001").parent, None); // virtual root
        assert_eq!(find("001.0").parent, Some("001".to_string()));
        assert_eq!(find("001.00").parent, Some("001.0".to_string()));
        assert_eq!(find("E800").parent, None); // virtual root
        assert_eq!(find("E800.0").parent, Some("E800".to_string()));
    }

    #[test]
    fn parse_skips_lines_without_pipe() {
        let text = "001|Cholera\nBADLINE\n0010|Sub-cholera\n";
        let (concepts, errors) = parse_pipe_delimited(text);
        assert_eq!(concepts.len(), 2);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("no pipe separator"));
    }

    #[test]
    fn parse_skips_empty_code() {
        let text = "|No code here\n001|Cholera\n";
        let (concepts, errors) = parse_pipe_delimited(text);
        assert_eq!(concepts.len(), 1);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("empty code"));
    }

    #[test]
    fn parse_ignores_blank_lines() {
        let text = "\n001|Cholera\n\n0010|Sub\n";
        let (concepts, errors) = parse_pipe_delimited(text);
        assert!(errors.is_empty());
        assert_eq!(concepts.len(), 2);
    }

    // ── Integration (SQLite) ──────────────────────────────────────────────────

    #[cfg(feature = "sqlite")]
    mod integration {
        use super::*;
        use crate::backends::SqliteTerminologyBackend;
        use std::io::Write;

        fn count(pool: &Pool<SqliteConnectionManager>, table: &str) -> i64 {
            pool.get()
                .unwrap()
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap()
        }

        fn make_txt_file(content: &str) -> tempfile::NamedTempFile {
            let mut f = tempfile::NamedTempFile::with_suffix(".txt").unwrap();
            f.write_all(content.as_bytes()).unwrap();
            f
        }

        fn make_zip_file(content: &str) -> tempfile::NamedTempFile {
            let tmp = tempfile::NamedTempFile::with_suffix(".zip").unwrap();
            {
                let mut zip = zip::ZipWriter::new(tmp.reopen().unwrap());
                zip.start_file("CMS32_DESC_LONG_DX.txt", zip::write::FileOptions::default())
                    .unwrap();
                zip.write_all(content.as_bytes()).unwrap();
                zip.finish().unwrap();
            }
            tmp
        }

        #[test]
        fn dry_run_does_not_write() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let f = make_txt_file(SAMPLE);
            let stats = import_icd9_cm(backend.pool(), f.path(), 500, true).unwrap();
            assert_eq!(stats.code_systems, 1);
            assert_eq!(stats.concepts, 7);
            assert_eq!(count(backend.pool(), "code_systems"), 0);
            assert_eq!(count(backend.pool(), "concepts"), 0);
        }

        #[test]
        fn live_import_writes_concepts_and_hierarchy() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let f = make_txt_file(SAMPLE);
            let stats = import_icd9_cm(backend.pool(), f.path(), 500, false).unwrap();
            assert_eq!(stats.code_systems, 1);
            assert_eq!(stats.concepts, 7);
            // virtual root + 7 concepts
            assert_eq!(count(backend.pool(), "concepts"), 8);
            // 7 hierarchy edges (all concepts have a parent — virtual root or real)
            assert_eq!(count(backend.pool(), "concept_hierarchy"), 7);
        }

        #[test]
        fn idempotent_reimport() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let f = make_txt_file(SAMPLE);
            import_icd9_cm(backend.pool(), f.path(), 500, false).unwrap();
            import_icd9_cm(backend.pool(), f.path(), 500, false).unwrap();
            assert_eq!(count(backend.pool(), "code_systems"), 1);
            assert_eq!(count(backend.pool(), "concepts"), 8);
            assert_eq!(count(backend.pool(), "concept_hierarchy"), 7);
        }

        #[test]
        fn import_from_zip() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let f = make_zip_file(SAMPLE);
            let stats = import_icd9_cm(backend.pool(), f.path(), 500, false).unwrap();
            assert_eq!(stats.concepts, 7);
            assert_eq!(count(backend.pool(), "concepts"), 8);
        }

        #[test]
        fn batching_preserves_all_concepts() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let f = make_txt_file(SAMPLE);
            let stats = import_icd9_cm(backend.pool(), f.path(), 2, false).unwrap();
            assert_eq!(stats.concepts, 7);
            assert_eq!(count(backend.pool(), "concepts"), 8);
        }

        #[test]
        fn missing_file_returns_error() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let result = import_icd9_cm(
                backend.pool(),
                Path::new("/nonexistent/icd9.txt"),
                500,
                false,
            );
            assert!(result.is_err());
        }
    }
}
