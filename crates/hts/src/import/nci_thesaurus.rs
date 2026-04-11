//! NCI Thesaurus (NCIt) importer.
//!
//! Parses the NCI Thesaurus flat-text distribution (`Thesaurus.txt`) and imports
//! ~170 k biomedical concepts with parent–child hierarchy into the HTS normalized
//! schema.
//!
//! # No license required
//!
//! The NCI Thesaurus is a product of the National Cancer Institute (NCI), a US
//! federal agency, and is **public domain**. Download the latest release from:
//! <https://evs.nci.nih.gov/ftp1/NCI_Thesaurus/>
//!
//! # File format
//!
//! The distribution is a tab-delimited `.txt` file (optionally wrapped in a
//! `.zip`). Each line after the header represents one concept:
//!
//! ```text
//! code\tconcept_name\tparent_codes\tsynonyms\tdefinition\tdisplay_name\tstatus\tsemantic_type
//! C000001\tSome Concept\tC000002|C000003\t...\tA definition.\tSome Concept\tConcept\tDisease
//! ```
//!
//! | Column | Field | Notes |
//! |--------|-------|-------|
//! | 0 | Code | NCI code, e.g. `C12345` |
//! | 1 | Concept name | Internal name |
//! | 2 | Parents | Pipe-separated parent codes; empty = root concept |
//! | 4 | Definition | Used as `definition` in HTS |
//! | 5 | Display name | Preferred label; may differ from column 1 |
//! | 6 | Concept status | `Retired_Concept`, `Header_Concept`, … |

#[cfg(feature = "sqlite")]
use r2d2::Pool;
#[cfg(feature = "sqlite")]
use r2d2_sqlite::SqliteConnectionManager;

use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use crate::error::HtsError;
use crate::import::ImportStats;

// ── Constants ─────────────────────────────────────────────────────────────────

const NCI_URL: &str = "http://ncicb.nci.nih.gov/xml/owl/EVS/Thesaurus.owl";
const NCI_NAME: &str = "NCIt";
const NCI_TITLE: &str = "NCI Thesaurus (NCIt)";

// ── Data types ────────────────────────────────────────────────────────────────

#[derive(Debug)]
struct NciConcept {
    /// NCI concept code, e.g. `C12345`.
    code: String,
    /// Preferred display name (column 5); falls back to the internal concept name (column 1).
    display: String,
    /// Prose definition from column 4, if present.
    definition: Option<String>,
    /// Zero or more parent concept codes parsed from the pipe-separated column 2.
    /// Empty for root concepts.
    parents: Vec<String>,
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Import an NCI Thesaurus flat-text distribution into the HTS database.
///
/// `path` may point to:
/// - The raw `Thesaurus.txt` file, or
/// - A `.zip` archive containing a file with "thesaurus" in the name.
#[cfg(feature = "sqlite")]
pub fn import_nci_thesaurus(
    pool: &Pool<SqliteConnectionManager>,
    path: &Path,
    batch_size: usize,
    dry_run: bool,
) -> Result<ImportStats, HtsError> {
    let batch_size = batch_size.max(1);
    let text = read_text(path)?;
    let (concepts, errors) = parse_thesaurus_txt(&text);

    let mut stats = ImportStats {
        errors,
        ..Default::default()
    };

    if dry_run {
        stats.code_systems = 1;
        stats.concepts = concepts.len() as u32;
        eprintln!(
            "[nci-thesaurus] dry-run — {} concepts parsed, no DB writes",
            concepts.len()
        );
        return Ok(stats);
    }

    let conn = pool
        .get()
        .map_err(|e| HtsError::Internal(format!("DB pool error: {e}")))?;

    let system_id = upsert_code_system(&conn)?;
    stats.code_systems = 1;

    let total = concepts.len();
    let total_batches = total.div_ceil(batch_size);

    for (batch_idx, batch) in concepts.chunks(batch_size).enumerate() {
        insert_concept_batch(&conn, &system_id, batch)?;
        insert_hierarchy_batch(&conn, &system_id, batch)?;

        let inserted = ((batch_idx + 1) * batch_size).min(total);
        eprintln!(
            "[nci-thesaurus] batch {}/{total_batches} — +{} concepts (total: {inserted})",
            batch_idx + 1,
            batch.len()
        );
    }

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
            if name.ends_with(".txt") && name.contains("thesaurus") {
                Some(i)
            } else {
                None
            }
        })
        .ok_or_else(|| {
            HtsError::InvalidRequest(format!(
                "No Thesaurus.txt found inside ZIP '{}'. \
                 Expected a file with 'thesaurus' in the name.",
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

/// Parse the NCI Thesaurus flat-text file into a list of concepts.
///
/// Returns `(concepts, non_fatal_errors)`.
fn parse_thesaurus_txt(text: &str) -> (Vec<NciConcept>, Vec<String>) {
    let mut concepts = Vec::new();
    let mut errors = Vec::new();
    let mut reader = BufReader::new(text.as_bytes()).lines().enumerate();

    // Skip the header row if present (starts with "code" or "Code", not "C" followed by digits).
    let first = reader.next();
    if let Some((_, Ok(line))) = first {
        let trimmed = line.trim();
        // If the first column doesn't look like an NCI code, treat it as a header.
        let first_col = trimmed.split('\t').next().unwrap_or("");
        if !looks_like_nci_code(first_col) {
            // Header row — skip, process remaining lines below.
        } else {
            process_line(0, trimmed, &mut concepts, &mut errors);
        }
    }

    for (line_num, line) in reader {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        process_line(line_num + 1, trimmed, &mut concepts, &mut errors);
    }

    (concepts, errors)
}

/// Returns `true` if the string looks like an NCI concept code (`C` followed by digits).
fn looks_like_nci_code(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some('C')) && chars.next().map(|c| c.is_ascii_digit()).unwrap_or(false)
}

/// Parse a single tab-delimited data line and push the result into `concepts`,
/// or push a message into `errors` if the line is malformed.
fn process_line(
    line_num: usize,
    line: &str,
    concepts: &mut Vec<NciConcept>,
    errors: &mut Vec<String>,
) {
    let cols: Vec<&str> = line.split('\t').collect();
    if cols.len() < 2 {
        errors.push(format!("line {line_num}: too few columns — skipped"));
        return;
    }

    let code = cols[0].trim();
    if code.is_empty() {
        errors.push(format!("line {line_num}: empty code — skipped"));
        return;
    }

    // Column 5 = display name; fall back to column 1 (internal concept name).
    let display = if cols.len() > 5 && !cols[5].trim().is_empty() {
        cols[5].trim()
    } else {
        cols[1].trim()
    };

    // Column 4 = definition.
    let definition = if cols.len() > 4 && !cols[4].trim().is_empty() {
        Some(cols[4].trim().to_string())
    } else {
        None
    };

    // Column 2 = pipe-separated parent codes.
    let parents: Vec<String> = if cols.len() > 2 && !cols[2].trim().is_empty() {
        cols[2]
            .split('|')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect()
    } else {
        Vec::new()
    };

    concepts.push(NciConcept {
        code: code.to_string(),
        display: display.to_string(),
        definition,
        parents,
    });
}

// ── DB helpers ────────────────────────────────────────────────────────────────

/// Insert the NCI Thesaurus CodeSystem row if absent, then return its `id`.
#[cfg(feature = "sqlite")]
fn upsert_code_system(conn: &rusqlite::Connection) -> Result<String, HtsError> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    conn.execute(
        "INSERT OR IGNORE INTO code_systems \
         (id, url, version, name, title, status, content, created_at, updated_at) \
         VALUES (?1, ?2, 'current', ?3, ?4, 'active', 'complete', ?5, ?5)",
        rusqlite::params![id, NCI_URL, NCI_NAME, NCI_TITLE, now],
    )
    .map_err(|e| HtsError::Internal(format!("Upsert CodeSystem: {e}")))?;

    let system_id: String = conn
        .query_row(
            "SELECT id FROM code_systems WHERE url = ?1",
            rusqlite::params![NCI_URL],
            |row| row.get(0),
        )
        .map_err(|e| HtsError::Internal(format!("Fetch CodeSystem id: {e}")))?;

    Ok(system_id)
}

/// Upsert one batch of NCIt concepts inside a single transaction.
#[cfg(feature = "sqlite")]
fn insert_concept_batch(
    conn: &rusqlite::Connection,
    system_id: &str,
    batch: &[NciConcept],
) -> Result<(), HtsError> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| HtsError::Internal(format!("Begin transaction: {e}")))?;

    for c in batch {
        tx.execute(
            "INSERT OR REPLACE INTO concepts (system_id, code, display, definition) \
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![system_id, c.code, c.display, c.definition],
        )
        .map_err(|e| HtsError::Internal(format!("Insert concept '{}': {e}", c.code)))?;
    }

    tx.commit()
        .map_err(|e| HtsError::Internal(format!("Commit: {e}")))?;
    Ok(())
}

/// Insert one `parent_code → code` edge per parent for each concept in the batch.
/// Concepts with no parents (root concepts) produce no edges.
#[cfg(feature = "sqlite")]
fn insert_hierarchy_batch(
    conn: &rusqlite::Connection,
    system_id: &str,
    batch: &[NciConcept],
) -> Result<(), HtsError> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| HtsError::Internal(format!("Begin hierarchy transaction: {e}")))?;

    for c in batch {
        for parent_code in &c.parents {
            tx.execute(
                "INSERT OR IGNORE INTO concept_hierarchy \
                 (system_id, parent_code, child_code) VALUES (?1, ?2, ?3)",
                rusqlite::params![system_id, parent_code, c.code],
            )
            .map_err(|e| {
                HtsError::Internal(format!("Insert hierarchy {parent_code}->{}: {e}", c.code))
            })?;
        }
    }

    tx.commit()
        .map_err(|e| HtsError::Internal(format!("Commit hierarchy: {e}")))?;
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // Header row + 4 concepts; C000003 has two parents.
    const SAMPLE: &str = "\
code\tconcept_name\tparents\tsynonyms\tdefinition\tdisplay_name\tstatus\tsemantic_type\n\
C000001\tRoot Concept\t\t\tThe root.\tRoot Concept\tConcept\tEntity\n\
C000002\tChild One\tC000001\t\tA child.\tChild One\tConcept\tEntity\n\
C000003\tChild Two\tC000001\t\tAnother child.\tChild Two\tConcept\tEntity\n\
C000004\tGrandchild\tC000002|C000003\t\tA grandchild.\tGrandchild\tConcept\tEntity\n\
";

    #[test]
    fn parse_returns_correct_count() {
        let (concepts, errors) = parse_thesaurus_txt(SAMPLE);
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
        assert_eq!(concepts.len(), 4);
    }

    #[test]
    fn parse_extracts_parents() {
        let (concepts, _) = parse_thesaurus_txt(SAMPLE);
        let find = |code: &str| concepts.iter().find(|c| c.code == code).unwrap();

        assert!(find("C000001").parents.is_empty());
        assert_eq!(find("C000002").parents, vec!["C000001"]);
        assert_eq!(find("C000004").parents, vec!["C000002", "C000003"]);
    }

    #[test]
    fn parse_extracts_display_name() {
        let (concepts, _) = parse_thesaurus_txt(SAMPLE);
        let root = concepts.iter().find(|c| c.code == "C000001").unwrap();
        assert_eq!(root.display, "Root Concept");
    }

    #[test]
    fn parse_extracts_definition() {
        let (concepts, _) = parse_thesaurus_txt(SAMPLE);
        let c = concepts.iter().find(|c| c.code == "C000002").unwrap();
        assert_eq!(c.definition.as_deref(), Some("A child."));
    }

    #[test]
    fn parse_skips_lines_with_too_few_columns() {
        // A 2-column line (code + concept name) parses successfully.
        let text = "C000001\tOnly One Column\n";
        let (concepts, errors) = parse_thesaurus_txt(text);
        assert_eq!(concepts.len(), 1);
        assert!(errors.is_empty());

        // A non-NCI-looking single word is treated as a header row and silently
        // skipped (no error recorded).
        let header_only = "NOCOLS\n";
        let (c2, e2) = parse_thesaurus_txt(header_only);
        assert_eq!(c2.len(), 0);
        assert!(
            e2.is_empty(),
            "header-like line should be silently skipped: {e2:?}"
        );

        // A line that looks like an NCI code but has only one column produces an error.
        let bad_nci = "C12345\n";
        let (c3, e3) = parse_thesaurus_txt(bad_nci);
        assert_eq!(c3.len(), 0);
        assert_eq!(
            e3.len(),
            1,
            "single-column NCI-like line should produce an error"
        );
    }

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

        #[test]
        fn dry_run_does_not_write() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let f = make_txt_file(SAMPLE);
            let stats = import_nci_thesaurus(backend.pool(), f.path(), 500, true).unwrap();
            assert_eq!(stats.code_systems, 1);
            assert_eq!(stats.concepts, 4);
            assert_eq!(count(backend.pool(), "code_systems"), 0);
        }

        #[test]
        fn live_import_writes_concepts_and_hierarchy() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let f = make_txt_file(SAMPLE);
            let stats = import_nci_thesaurus(backend.pool(), f.path(), 500, false).unwrap();
            assert_eq!(stats.code_systems, 1);
            assert_eq!(stats.concepts, 4);
            assert_eq!(count(backend.pool(), "concepts"), 4);
            // C000002→C000001, C000003→C000001, C000004→C000002, C000004→C000003 = 4 edges
            assert_eq!(count(backend.pool(), "concept_hierarchy"), 4);
        }

        #[test]
        fn idempotent_reimport() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let f = make_txt_file(SAMPLE);
            import_nci_thesaurus(backend.pool(), f.path(), 500, false).unwrap();
            import_nci_thesaurus(backend.pool(), f.path(), 500, false).unwrap();
            assert_eq!(count(backend.pool(), "code_systems"), 1);
            assert_eq!(count(backend.pool(), "concepts"), 4);
            assert_eq!(count(backend.pool(), "concept_hierarchy"), 4);
        }

        #[test]
        fn batching_preserves_all_concepts() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let f = make_txt_file(SAMPLE);
            let stats = import_nci_thesaurus(backend.pool(), f.path(), 1, false).unwrap();
            assert_eq!(stats.concepts, 4);
            assert_eq!(count(backend.pool(), "concepts"), 4);
        }
    }
}
