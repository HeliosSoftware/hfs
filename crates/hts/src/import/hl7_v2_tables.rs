//! HL7 v2 tables importer.
//!
//! Parses the HL7 v2 vocabulary tables XML distribution and imports all table
//! entries as FHIR CodeSystem resources into the HTS normalized schema. Each HL7
//! v2 table becomes a separate CodeSystem.
//!
//! # No license required (with attribution)
//!
//! HL7 v2 tables are published by HL7 International under the HL7 FHIR License
//! and are freely redistributable with attribution. They are also bundled inside
//! the HL7 THO NPM package — if you have already run `hts import <tgz>`, no
//! separate import is needed.
//!
//! For standalone use, download the v2 table definitions from:
//! <https://terminology.hl7.org>
//!
//! **Required attribution when redistributing:**
//! ```text
//! This product includes content from HL7 Terminology (THO).
//! Copyright © Health Level Seven International.
//! Licensed under the HL7 FHIR License.
//! ```
//!
//! # File format
//!
//! The importer accepts an HL7 v2 tables XML file in either of two common
//! formats:
//!
//! **Format A — single root with `<HL7Table>` children:**
//! ```xml
//! <HL7Tables>
//!   <HL7Table id="0001" name="Administrative Sex">
//!     <tableEntry code="F" displayName="Female"/>
//!     <tableEntry code="M" displayName="Male"/>
//!   </HL7Table>
//! </HL7Tables>
//! ```
//!
//! **Format B — single `<HL7Table>` root (one file per table):**
//! ```xml
//! <HL7Table id="0001" name="Administrative Sex">
//!   <tableEntry code="F" displayName="Female"/>
//!   <tableEntry code="M" displayName="Male"/>
//! </HL7Table>
//! ```
//!
//! ZIP archives containing multiple XML files are also supported; each `.xml`
//! file inside is parsed as an independent table.
//!
//! # Hierarchy
//!
//! HL7 v2 tables are flat enumerations. All codes are placed as children of a
//! virtual root concept named after the table ID (e.g. `v2-0001`).

#[cfg(feature = "sqlite")]
use r2d2::Pool;
#[cfg(feature = "sqlite")]
use r2d2_sqlite::SqliteConnectionManager;

use std::io::Read;
use std::path::Path;

use crate::error::HtsError;
use crate::import::ImportStats;

// ── Constants ─────────────────────────────────────────────────────────────────

const V2_URL_PREFIX: &str = "http://terminology.hl7.org/CodeSystem/v2-";

// ── Data types ────────────────────────────────────────────────────────────────

#[derive(Debug)]
struct V2Table {
    /// Table ID, e.g. "0001".
    id: String,
    /// Table name, e.g. "Administrative Sex".
    name: String,
    /// All code entries.
    entries: Vec<V2Entry>,
}

#[derive(Debug)]
struct V2Entry {
    code: String,
    display: String,
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Import HL7 v2 table definitions into the HTS database.
///
/// `path` may point to:
/// - A single XML file (one table or a multi-table document), or
/// - A `.zip` archive containing one or more XML files.
///
/// Each distinct HL7 v2 table becomes a separate `CodeSystem` in HTS.
#[cfg(feature = "sqlite")]
pub fn import_hl7_v2_tables(
    pool: &Pool<SqliteConnectionManager>,
    path: &Path,
    batch_size: usize,
    dry_run: bool,
) -> Result<ImportStats, HtsError> {
    let batch_size = batch_size.max(1);
    let xmls = read_xmls(path)?;

    let mut all_tables: Vec<V2Table> = Vec::new();
    let mut all_errors: Vec<String> = Vec::new();

    for (source_name, xml) in &xmls {
        let (tables, errors) = parse_v2_xml(xml, source_name);
        all_tables.extend(tables);
        all_errors.extend(errors);
    }

    let total_concepts: u32 = all_tables.iter().map(|t| t.entries.len() as u32).sum();

    let mut stats = ImportStats {
        errors: all_errors,
        ..Default::default()
    };

    if dry_run {
        stats.code_systems = all_tables.len() as u32;
        stats.concepts = total_concepts;
        eprintln!(
            "[hl7-v2-tables] dry-run — {} tables, {} codes parsed, no DB writes",
            all_tables.len(),
            total_concepts
        );
        return Ok(stats);
    }

    let conn = pool
        .get()
        .map_err(|e| HtsError::Internal(format!("DB pool error: {e}")))?;

    for table in &all_tables {
        let system_id = upsert_code_system(&conn, table)?;

        // Virtual root concept for hierarchy edges.
        let root_code = format!("v2-{}", table.id);
        conn.execute(
            "INSERT OR IGNORE INTO concepts (system_id, code, display, definition) \
             VALUES (?1, ?2, ?3, 'header')",
            rusqlite::params![system_id, root_code, table.name],
        )
        .map_err(|e| HtsError::Internal(format!("Insert root concept: {e}")))?;

        let total = table.entries.len();
        let total_batches = total.div_ceil(batch_size);

        for (batch_idx, batch) in table.entries.chunks(batch_size).enumerate() {
            insert_entry_batch(&conn, &system_id, &root_code, batch)?;

            let inserted = ((batch_idx + 1) * batch_size).min(total);
            eprintln!(
                "[hl7-v2-tables] table {} ({}) batch {}/{total_batches} — +{} codes (total: {inserted})",
                table.id,
                table.name,
                batch_idx + 1,
                batch.len()
            );
        }

        stats.code_systems += 1;
        stats.concepts += total as u32;
    }

    Ok(stats)
}

// ── File reader ───────────────────────────────────────────────────────────────

/// Returns a list of `(source_name, xml_content)` pairs.
fn read_xmls(path: &Path) -> Result<Vec<(String, String)>, HtsError> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    if ext == "zip" {
        read_xmls_from_zip(path)
    } else {
        let content = std::fs::read_to_string(path).map_err(|e| {
            HtsError::InvalidRequest(format!("Cannot read '{}': {e}", path.display()))
        })?;
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();
        Ok(vec![(name, content)])
    }
}

fn read_xmls_from_zip(path: &Path) -> Result<Vec<(String, String)>, HtsError> {
    let file = std::fs::File::open(path).map_err(|e| {
        HtsError::InvalidRequest(format!("Cannot open ZIP '{}': {e}", path.display()))
    })?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| HtsError::InvalidRequest(format!("Invalid ZIP '{}': {e}", path.display())))?;

    let mut results = Vec::new();
    let indices: Vec<usize> = (0..archive.len())
        .filter(|&i| {
            archive
                .by_index(i)
                .ok()
                .map(|e| e.name().to_ascii_lowercase().ends_with(".xml"))
                .unwrap_or(false)
        })
        .collect();

    for i in indices {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| HtsError::InvalidRequest(format!("Cannot read ZIP entry: {e}")))?;
        let name = entry.name().to_string();
        let mut buf = String::new();
        entry
            .read_to_string(&mut buf)
            .map_err(|e| HtsError::InvalidRequest(format!("Cannot read '{name}' from ZIP: {e}")))?;
        results.push((name, buf));
    }

    if results.is_empty() {
        return Err(HtsError::InvalidRequest(format!(
            "No XML files found inside ZIP '{}'.",
            path.display()
        )));
    }

    Ok(results)
}

// ── Parser ────────────────────────────────────────────────────────────────────

/// Parse one HL7 v2 XML file into a list of tables and non-fatal errors.
///
/// Returns `(tables, non_fatal_errors)`.
/// Handles both Format A (`<HL7Tables>` root with multiple `<HL7Table>` children)
/// and Format B (a single `<HL7Table>` as the root element).
fn parse_v2_xml(xml: &str, source_name: &str) -> (Vec<V2Table>, Vec<String>) {
    let doc = match roxmltree::Document::parse(xml) {
        Ok(d) => d,
        Err(e) => {
            return (
                Vec::new(),
                vec![format!("Invalid XML in '{source_name}': {e}")],
            );
        }
    };

    let root = doc.root_element();
    let root_tag = root.tag_name().name();
    let mut tables = Vec::new();
    let mut errors = Vec::new();

    match root_tag {
        "HL7Tables" => {
            // Format A: multiple tables under a single root.
            for child in root
                .children()
                .filter(|n| n.is_element() && n.tag_name().name() == "HL7Table")
            {
                if let Some(t) = parse_table_element(&child, &mut errors) {
                    tables.push(t);
                }
            }
        }
        "HL7Table" => {
            // Format B: single table as the root element.
            if let Some(t) = parse_table_element(&root, &mut errors) {
                tables.push(t);
            }
        }
        other => {
            errors.push(format!(
                "'{source_name}': unexpected root element <{other}> — expected <HL7Tables> or <HL7Table>"
            ));
        }
    }

    (tables, errors)
}

/// Extract a [`V2Table`] from an `<HL7Table>` element.
///
/// Returns `None` if the required `id` attribute is absent.
fn parse_table_element(node: &roxmltree::Node, errors: &mut Vec<String>) -> Option<V2Table> {
    let id = node.attribute("id")?.trim().to_string();
    let name = node.attribute("name").unwrap_or(&id).trim().to_string();

    let mut entries = Vec::new();
    for entry in node
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "tableEntry")
    {
        let code = match entry.attribute("code") {
            Some(c) if !c.trim().is_empty() => c.trim().to_string(),
            _ => {
                errors.push(format!(
                    "tableEntry in table {id} missing 'code' attribute — skipped"
                ));
                continue;
            }
        };
        let display = entry
            .attribute("displayName")
            .unwrap_or(&code)
            .trim()
            .to_string();
        entries.push(V2Entry { code, display });
    }

    Some(V2Table { id, name, entries })
}

// ── DB helpers ────────────────────────────────────────────────────────────────

/// Insert a CodeSystem row for the given HL7 v2 table if absent, then return
/// its `id`.  Each table maps to a distinct CodeSystem URL (e.g.
/// `http://terminology.hl7.org/CodeSystem/v2-0001`).
#[cfg(feature = "sqlite")]
fn upsert_code_system(conn: &rusqlite::Connection, table: &V2Table) -> Result<String, HtsError> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let url = format!("{V2_URL_PREFIX}{}", table.id);

    conn.execute(
        "INSERT OR IGNORE INTO code_systems \
         (id, url, version, name, title, status, content, created_at, updated_at) \
         VALUES (?1, ?2, 'current', ?3, ?4, 'active', 'complete', ?5, ?5)",
        rusqlite::params![id, url, format!("v2-{}", table.id), table.name, now],
    )
    .map_err(|e| HtsError::Internal(format!("Upsert CodeSystem v2-{}: {e}", table.id)))?;

    let system_id: String = conn
        .query_row(
            "SELECT id FROM code_systems WHERE url = ?1",
            rusqlite::params![url],
            |row| row.get(0),
        )
        .map_err(|e| HtsError::Internal(format!("Fetch CodeSystem id: {e}")))?;

    Ok(system_id)
}

/// Upsert one batch of table entries and their flat hierarchy edges inside a
/// single transaction.  Each entry also gets a `ROOT_CODE → code` edge so
/// the virtual table root connects to all codes.
#[cfg(feature = "sqlite")]
fn insert_entry_batch(
    conn: &rusqlite::Connection,
    system_id: &str,
    root_code: &str,
    batch: &[V2Entry],
) -> Result<(), HtsError> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| HtsError::Internal(format!("Begin transaction: {e}")))?;

    for e in batch {
        tx.execute(
            "INSERT OR REPLACE INTO concepts (system_id, code, display) VALUES (?1, ?2, ?3)",
            rusqlite::params![system_id, e.code, e.display],
        )
        .map_err(|e_| HtsError::Internal(format!("Insert concept '{}': {e_}", e.code)))?;

        tx.execute(
            "INSERT OR IGNORE INTO concept_hierarchy (system_id, parent_code, child_code) \
             VALUES (?1, ?2, ?3)",
            rusqlite::params![system_id, root_code, e.code],
        )
        .map_err(|e_| {
            HtsError::Internal(format!("Insert hierarchy {root_code}->{}: {e_}", e.code))
        })?;
    }

    tx.commit()
        .map_err(|e| HtsError::Internal(format!("Commit: {e}")))?;
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_MULTI: &str = r#"<?xml version="1.0"?>
<HL7Tables>
  <HL7Table id="0001" name="Administrative Sex">
    <tableEntry code="F" displayName="Female"/>
    <tableEntry code="M" displayName="Male"/>
    <tableEntry code="O" displayName="Other"/>
  </HL7Table>
  <HL7Table id="0002" name="Marital Status">
    <tableEntry code="S" displayName="Single"/>
    <tableEntry code="M" displayName="Married"/>
  </HL7Table>
</HL7Tables>"#;

    const SAMPLE_SINGLE: &str = r#"<?xml version="1.0"?>
<HL7Table id="0001" name="Administrative Sex">
  <tableEntry code="F" displayName="Female"/>
  <tableEntry code="M" displayName="Male"/>
</HL7Table>"#;

    #[test]
    fn parse_multi_table_format() {
        let (tables, errors) = parse_v2_xml(SAMPLE_MULTI, "test.xml");
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
        assert_eq!(tables.len(), 2);
        assert_eq!(tables[0].id, "0001");
        assert_eq!(tables[0].entries.len(), 3);
        assert_eq!(tables[1].id, "0002");
        assert_eq!(tables[1].entries.len(), 2);
    }

    #[test]
    fn parse_single_table_format() {
        let (tables, errors) = parse_v2_xml(SAMPLE_SINGLE, "test.xml");
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].entries.len(), 2);
    }

    #[test]
    fn parse_extracts_entry_code_and_display() {
        let (tables, _) = parse_v2_xml(SAMPLE_SINGLE, "test.xml");
        let f = tables[0].entries.iter().find(|e| e.code == "F").unwrap();
        assert_eq!(f.display, "Female");
    }

    #[test]
    fn parse_invalid_xml_returns_error() {
        let (tables, errors) = parse_v2_xml("<<not xml>>", "bad.xml");
        assert!(tables.is_empty());
        assert!(!errors.is_empty());
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

        fn make_xml_file(content: &str) -> tempfile::NamedTempFile {
            let mut f = tempfile::NamedTempFile::with_suffix(".xml").unwrap();
            f.write_all(content.as_bytes()).unwrap();
            f
        }

        #[test]
        fn dry_run_does_not_write() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let f = make_xml_file(SAMPLE_MULTI);
            let stats = import_hl7_v2_tables(backend.pool(), f.path(), 500, true).unwrap();
            assert_eq!(stats.code_systems, 2);
            assert_eq!(stats.concepts, 5);
            assert_eq!(count(backend.pool(), "code_systems"), 0);
        }

        #[test]
        fn live_import_writes_two_code_systems() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let f = make_xml_file(SAMPLE_MULTI);
            let stats = import_hl7_v2_tables(backend.pool(), f.path(), 500, false).unwrap();
            assert_eq!(stats.code_systems, 2);
            assert_eq!(stats.concepts, 5);
            assert_eq!(count(backend.pool(), "code_systems"), 2);
            // 2 virtual root concepts + 5 codes = 7
            assert_eq!(count(backend.pool(), "concepts"), 7);
            // 5 flat hierarchy edges
            assert_eq!(count(backend.pool(), "concept_hierarchy"), 5);
        }

        #[test]
        fn idempotent_reimport() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let f = make_xml_file(SAMPLE_MULTI);
            import_hl7_v2_tables(backend.pool(), f.path(), 500, false).unwrap();
            import_hl7_v2_tables(backend.pool(), f.path(), 500, false).unwrap();
            assert_eq!(count(backend.pool(), "code_systems"), 2);
            assert_eq!(count(backend.pool(), "concepts"), 7);
            assert_eq!(count(backend.pool(), "concept_hierarchy"), 5);
        }
    }
}
