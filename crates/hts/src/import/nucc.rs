//! NUCC Provider Taxonomy importer.
//!
//! Parses the National Uniform Claim Committee (NUCC) Health Care Provider
//! Taxonomy CSV and imports all provider taxonomy codes into the HTS normalized
//! schema.
//!
//! # No license required
//!
//! The NUCC Provider Taxonomy is freely available. Download the current release
//! from:
//! <https://www.nucc.org/index.php/code-sets-mainmenu-41/provider-taxonomy-mainmenu-40/csv-mainmenu-57>
//!
//! # File format
//!
//! The distribution is a CSV file (`nucc_taxonomy_YYYYMMDD.csv`) with these
//! columns:
//!
//! | # | Column | Description |
//! |---|--------|-------------|
//! | 0 | `Code` | 10-character taxonomy code (primary key) |
//! | 1 | `Grouping` | Top-level grouping (level 1) |
//! | 2 | `Classification` | Classification within the grouping (level 2) |
//! | 3 | `Specialization` | Specialization within the classification (level 3, may be empty) |
//! | 4 | `Definition` | Prose definition |
//! | 9 | `Display Name` | Preferred display name |
//!
//! # Hierarchy
//!
//! Hierarchy is inferred from the Grouping / Classification / Specialization
//! columns. HTS inserts synthetic parent codes for each unique grouping and
//! classification:
//!
//! ```text
//! Virtual root  →  "<Grouping>"  →  "<Classification>"  →  <Code>
//! ```
//!
//! When Specialization is non-empty, the code's parent is the Classification.
//! When Specialization is empty, the code *is* the Classification-level entry
//! and its parent is the Grouping.

#[cfg(feature = "sqlite")]
use r2d2::Pool;
#[cfg(feature = "sqlite")]
use r2d2_sqlite::SqliteConnectionManager;

use std::collections::HashSet;
use std::io::Read;
use std::path::Path;

use crate::error::HtsError;
use crate::import::ImportStats;

// ── Constants ─────────────────────────────────────────────────────────────────

const NUCC_URL: &str = "http://nucc.org/provider-taxonomy";
const NUCC_NAME: &str = "NUCC";
const NUCC_TITLE: &str = "NUCC Provider Taxonomy";
const ROOT_CODE: &str = "NUCC";

// ── Data types ────────────────────────────────────────────────────────────────

#[derive(Debug)]
struct NuccConcept {
    code: String,
    display: String,
    definition: Option<String>,
    /// Inferred parent: grouping name, classification name, or virtual root.
    parent: Option<String>,
}

/// A synthetic grouping or classification node used to build hierarchy.
///
/// NUCC does not assign codes to groupings or classifications — only to leaf
/// taxonomy codes. HTS generates one synthetic node per unique grouping and one
/// per unique (grouping, classification) pair so that hierarchy edges have valid
/// parent concepts.
#[derive(Debug)]
struct SyntheticNode {
    /// The grouping or classification name used as the synthetic concept code.
    code: String,
    /// Human-readable display, equal to `code` for synthetic nodes.
    display: String,
    /// Parent: the grouping for a classification node, or the virtual root for
    /// a top-level grouping node.
    parent: Option<String>,
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Import a NUCC Provider Taxonomy CSV into the HTS database.
///
/// `path` may point to:
/// - The raw `nucc_taxonomy_*.csv` file, or
/// - A `.zip` archive containing such a file.
#[cfg(feature = "sqlite")]
pub fn import_nucc(
    pool: &Pool<SqliteConnectionManager>,
    path: &Path,
    batch_size: usize,
    dry_run: bool,
) -> Result<ImportStats, HtsError> {
    let batch_size = batch_size.max(1);
    let text = read_text(path)?;
    let (concepts, synthetic_nodes, errors) = parse_nucc_csv(&text);

    let mut stats = ImportStats {
        errors,
        ..Default::default()
    };

    if dry_run {
        stats.code_systems = 1;
        stats.concepts = concepts.len() as u32;
        eprintln!(
            "[nucc] dry-run — {} codes parsed ({} synthetic grouping nodes), no DB writes",
            concepts.len(),
            synthetic_nodes.len()
        );
        return Ok(stats);
    }

    let conn = pool
        .get()
        .map_err(|e| HtsError::Internal(format!("DB pool error: {e}")))?;

    let system_id = upsert_code_system(&conn)?;
    stats.code_systems = 1;

    // Insert virtual root.
    conn.execute(
        "INSERT OR IGNORE INTO concepts (system_id, code, display, definition) \
         VALUES (?1, ?2, ?3, 'header')",
        rusqlite::params![system_id, ROOT_CODE, NUCC_TITLE],
    )
    .map_err(|e| HtsError::Internal(format!("Insert virtual root: {e}")))?;

    // Insert synthetic grouping/classification nodes first so hierarchy edges
    // can reference them as parents.
    insert_synthetic_nodes(&conn, &system_id, &synthetic_nodes)?;

    let total = concepts.len();
    let total_batches = total.div_ceil(batch_size);

    for (batch_idx, batch) in concepts.chunks(batch_size).enumerate() {
        insert_concept_batch(&conn, &system_id, batch)?;

        let inserted = ((batch_idx + 1) * batch_size).min(total);
        eprintln!(
            "[nucc] batch {}/{total_batches} — +{} codes (total: {inserted})",
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
            if name.ends_with(".csv") && (name.contains("nucc") || name.contains("taxonomy")) {
                Some(i)
            } else {
                None
            }
        })
        .ok_or_else(|| {
            HtsError::InvalidRequest(format!(
                "No NUCC CSV found inside ZIP '{}'. \
                 Expected a CSV file with 'nucc' or 'taxonomy' in the name.",
                path.display()
            ))
        })?;

    let mut entry = archive
        .by_index(best_index)
        .map_err(|e| HtsError::InvalidRequest(format!("Cannot read ZIP entry: {e}")))?;
    let mut buf = String::new();
    entry
        .read_to_string(&mut buf)
        .map_err(|e| HtsError::InvalidRequest(format!("Cannot read CSV from ZIP: {e}")))?;
    Ok(buf)
}

// ── Parser ────────────────────────────────────────────────────────────────────

/// Parse the NUCC taxonomy CSV into real concepts and synthetic hierarchy nodes.
///
/// Returns `(concepts, synthetic_nodes, non_fatal_errors)`.
/// `synthetic_nodes` must be inserted before `concepts` so that hierarchy edges
/// can reference them as valid parent codes.
fn parse_nucc_csv(text: &str) -> (Vec<NuccConcept>, Vec<SyntheticNode>, Vec<String>) {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(text.as_bytes());

    let mut concepts = Vec::new();
    let mut errors = Vec::new();

    // Track unique groupings and classifications to create synthetic parent nodes.
    let mut seen_groupings: HashSet<String> = HashSet::new();
    let mut seen_classifications: HashSet<String> = HashSet::new();
    let mut synthetic_nodes: Vec<SyntheticNode> = Vec::new();

    for (row_idx, result) in reader.records().enumerate() {
        let record = match result {
            Ok(r) => r,
            Err(e) => {
                errors.push(format!("row {}: CSV parse error — {e}", row_idx + 2));
                continue;
            }
        };

        let code = record.get(0).unwrap_or("").trim().to_string();
        if code.is_empty() {
            continue;
        }

        let grouping = record.get(1).unwrap_or("").trim().to_string();
        let classification = record.get(2).unwrap_or("").trim().to_string();
        let specialization = record.get(3).unwrap_or("").trim().to_string();
        let definition = record
            .get(4)
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.trim().to_string());

        // Display Name is column 9; fall back to Specialization, then Classification.
        let display = record
            .get(9)
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.trim().to_string())
            .or_else(|| {
                if !specialization.is_empty() {
                    Some(specialization.clone())
                } else if !classification.is_empty() {
                    Some(classification.clone())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| code.clone());

        // Build synthetic grouping node (level 1).
        if !grouping.is_empty() && seen_groupings.insert(grouping.clone()) {
            synthetic_nodes.push(SyntheticNode {
                code: grouping.clone(),
                display: grouping.clone(),
                parent: Some(ROOT_CODE.to_string()),
            });
        }

        // Build synthetic classification node (level 2).
        // Use a compound key to detect duplicates: two different groupings may each
        // have a classification named "Other" — they must produce two distinct nodes.
        let class_key = if grouping.is_empty() {
            classification.clone()
        } else {
            format!("{grouping}::{classification}")
        };

        if !classification.is_empty() && seen_classifications.insert(class_key) {
            synthetic_nodes.push(SyntheticNode {
                code: classification.clone(),
                display: classification.clone(),
                parent: if grouping.is_empty() {
                    Some(ROOT_CODE.to_string())
                } else {
                    Some(grouping.clone())
                },
            });
        }

        // Determine the concept's parent.
        let parent = if !classification.is_empty() {
            Some(classification.clone())
        } else if !grouping.is_empty() {
            Some(grouping.clone())
        } else {
            None // orphan → will hang off ROOT_CODE in hierarchy insert
        };

        concepts.push(NuccConcept {
            code,
            display,
            definition,
            parent,
        });
    }

    (concepts, synthetic_nodes, errors)
}

// ── DB helpers ────────────────────────────────────────────────────────────────

/// Insert the NUCC CodeSystem row if absent, then return its `id`.
#[cfg(feature = "sqlite")]
fn upsert_code_system(conn: &rusqlite::Connection) -> Result<String, HtsError> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    conn.execute(
        "INSERT OR IGNORE INTO code_systems \
         (id, url, version, name, title, status, content, created_at, updated_at) \
         VALUES (?1, ?2, 'current', ?3, ?4, 'active', 'complete', ?5, ?5)",
        rusqlite::params![id, NUCC_URL, NUCC_NAME, NUCC_TITLE, now],
    )
    .map_err(|e| HtsError::Internal(format!("Upsert CodeSystem: {e}")))?;

    let system_id: String = conn
        .query_row(
            "SELECT id FROM code_systems WHERE url = ?1",
            rusqlite::params![NUCC_URL],
            |row| row.get(0),
        )
        .map_err(|e| HtsError::Internal(format!("Fetch CodeSystem id: {e}")))?;

    Ok(system_id)
}

/// Insert all synthetic grouping and classification nodes and their hierarchy
/// edges in a single transaction.  Must be called before `insert_concept_batch`
/// so that real concepts can reference these codes as parents.
#[cfg(feature = "sqlite")]
fn insert_synthetic_nodes(
    conn: &rusqlite::Connection,
    system_id: &str,
    nodes: &[SyntheticNode],
) -> Result<(), HtsError> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| HtsError::Internal(format!("Begin synthetic node transaction: {e}")))?;

    for node in nodes {
        tx.execute(
            "INSERT OR IGNORE INTO concepts (system_id, code, display, definition) \
             VALUES (?1, ?2, ?3, 'grouping')",
            rusqlite::params![system_id, node.code, node.display],
        )
        .map_err(|e| HtsError::Internal(format!("Insert synthetic node '{}': {e}", node.code)))?;

        let parent = node.parent.as_deref().unwrap_or(ROOT_CODE);
        tx.execute(
            "INSERT OR IGNORE INTO concept_hierarchy (system_id, parent_code, child_code) \
             VALUES (?1, ?2, ?3)",
            rusqlite::params![system_id, parent, node.code],
        )
        .map_err(|e| {
            HtsError::Internal(format!("Insert hierarchy {parent}->{}: {e}", node.code))
        })?;
    }

    tx.commit()
        .map_err(|e| HtsError::Internal(format!("Commit synthetic nodes: {e}")))?;
    Ok(())
}

/// Upsert one batch of NUCC taxonomy codes and their hierarchy edges inside a
/// single transaction.
#[cfg(feature = "sqlite")]
fn insert_concept_batch(
    conn: &rusqlite::Connection,
    system_id: &str,
    batch: &[NuccConcept],
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

        let parent = c.parent.as_deref().unwrap_or(ROOT_CODE);
        tx.execute(
            "INSERT OR IGNORE INTO concept_hierarchy (system_id, parent_code, child_code) \
             VALUES (?1, ?2, ?3)",
            rusqlite::params![system_id, parent, c.code],
        )
        .map_err(|e| HtsError::Internal(format!("Insert hierarchy {parent}->{}: {e}", c.code)))?;
    }

    tx.commit()
        .map_err(|e| HtsError::Internal(format!("Commit: {e}")))?;
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // Minimal CSV: header + 3 codes across 2 groupings.
    const SAMPLE_CSV: &str = "\
Code,Grouping,Classification,Specialization,Definition,Effective Date,Deactivation Date,Last Modified Date,Notes,Display Name\n\
101Y00000X,Behavioral Health,Counselor,,A provider trained in counseling.,20020101,,,, Counselor\n\
101YA0400X,Behavioral Health,Counselor,Addiction (Substance Use Disorder),Specializes in addiction.,20020101,,,, Addiction Counselor\n\
207Q00000X,Allopathic,Family Medicine,,Provides family medicine.,20020101,,,, Family Medicine Physician\n\
";

    #[test]
    fn parse_returns_correct_concept_count() {
        let (concepts, _, errors) = parse_nucc_csv(SAMPLE_CSV);
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
        assert_eq!(concepts.len(), 3);
    }

    #[test]
    fn parse_creates_synthetic_grouping_and_classification_nodes() {
        let (_, synthetics, _) = parse_nucc_csv(SAMPLE_CSV);
        // Groupings: "Behavioral Health", "Allopathic"
        // Classifications: "Counselor", "Family Medicine"
        let codes: Vec<&str> = synthetics.iter().map(|n| n.code.as_str()).collect();
        assert!(codes.contains(&"Behavioral Health"), "{codes:?}");
        assert!(codes.contains(&"Allopathic"), "{codes:?}");
        assert!(codes.contains(&"Counselor"), "{codes:?}");
        assert!(codes.contains(&"Family Medicine"), "{codes:?}");
    }

    #[test]
    fn parse_sets_parent_to_classification() {
        let (concepts, _, _) = parse_nucc_csv(SAMPLE_CSV);
        let c = concepts.iter().find(|c| c.code == "101Y00000X").unwrap();
        assert_eq!(c.parent.as_deref(), Some("Counselor"));
    }

    #[test]
    fn parse_extracts_display_name() {
        let (concepts, _, _) = parse_nucc_csv(SAMPLE_CSV);
        let c = concepts.iter().find(|c| c.code == "101Y00000X").unwrap();
        assert_eq!(c.display.trim(), "Counselor");
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
            let stats = import_nucc(backend.pool(), f.path(), 500, true).unwrap();
            assert_eq!(stats.code_systems, 1);
            assert_eq!(stats.concepts, 3);
            assert_eq!(count(backend.pool(), "code_systems"), 0);
        }

        #[test]
        fn live_import_writes_concepts_and_hierarchy() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let f = make_csv_file(SAMPLE_CSV);
            let stats = import_nucc(backend.pool(), f.path(), 500, false).unwrap();
            assert_eq!(stats.code_systems, 1);
            assert_eq!(stats.concepts, 3);
            // root + 2 groupings + 2 classifications + 3 codes = 8
            assert_eq!(count(backend.pool(), "concepts"), 8);
        }

        #[test]
        fn idempotent_reimport() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let f = make_csv_file(SAMPLE_CSV);
            import_nucc(backend.pool(), f.path(), 500, false).unwrap();
            import_nucc(backend.pool(), f.path(), 500, false).unwrap();
            assert_eq!(count(backend.pool(), "code_systems"), 1);
            assert_eq!(count(backend.pool(), "concepts"), 8);
        }
    }
}
