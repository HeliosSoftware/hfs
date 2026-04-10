//! MeSH (Medical Subject Headings) importer.
//!
//! Parses the NLM MeSH XML distribution and imports all descriptors with
//! tree-number-derived hierarchy into the HTS normalized schema.
//!
//! # No license required
//!
//! MeSH is produced by the U.S. National Library of Medicine (NLM), a US federal
//! agency, and is **public domain**. Download the annual release from:
//! <https://www.nlm.nih.gov/databases/download/mesh.html>
//!
//! # File format
//!
//! The MeSH XML distribution (`mesh2025.xml`) uses a `<DescriptorRecordSet>` root
//! element containing `<DescriptorRecord>` entries:
//!
//! ```xml
//! <DescriptorRecord DescriptorClass="1">
//!   <DescriptorUI>D000001</DescriptorUI>
//!   <DescriptorName><String>Calcimycin</String></DescriptorName>
//!   <TreeNumberList>
//!     <TreeNumber>D03.438.221.173</TreeNumber>
//!   </TreeNumberList>
//!   <ConceptList>
//!     <Concept PreferredConceptYN="Y">
//!       <ScopeNote>A calcium ionophore…</ScopeNote>
//!     </Concept>
//!   </ConceptList>
//! </DescriptorRecord>
//! ```
//!
//! # Hierarchy
//!
//! Tree numbers encode the MeSH hierarchy. The parent of `D03.438.221.173` is
//! the descriptor whose tree number is `D03.438.221`. HTS builds a tree-number →
//! descriptor UI map on parse, then resolves hierarchy edges at import time.
//!
//! Descriptors with no tree number (e.g. check-tags like `D002318`) have no
//! hierarchy edge and appear as orphan concepts.

#[cfg(feature = "sqlite")]
use r2d2::Pool;
#[cfg(feature = "sqlite")]
use r2d2_sqlite::SqliteConnectionManager;

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use crate::error::HtsError;
use crate::import::ImportStats;

// ── Constants ─────────────────────────────────────────────────────────────────

const MESH_URL: &str = "http://www.nlm.nih.gov/mesh";
const MESH_NAME: &str = "MeSH";
const MESH_TITLE: &str = "Medical Subject Headings (MeSH)";

// ── Data types ────────────────────────────────────────────────────────────────

#[derive(Debug)]
struct MeshDescriptor {
    /// MeSH Descriptor Unique Identifier, e.g. `D000001`.
    ui: String,
    /// Preferred English descriptor name.
    name: String,
    /// Scope note from the preferred concept; used as `definition` in HTS.
    scope_note: Option<String>,
    /// All tree numbers assigned to this descriptor (a descriptor may appear
    /// in multiple branches, e.g. `D03.438.221` and `G02.111.570.080`).
    tree_numbers: Vec<String>,
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Import a MeSH XML distribution into the HTS database.
///
/// `path` may point to:
/// - The raw `mesh.xml` / `desc2025.xml` file,
/// - A `.gz` compressed XML, or
/// - A `.zip` archive containing a MeSH XML file.
#[cfg(feature = "sqlite")]
pub fn import_mesh(
    pool: &Pool<SqliteConnectionManager>,
    path: &Path,
    batch_size: usize,
    dry_run: bool,
) -> Result<ImportStats, HtsError> {
    let batch_size = batch_size.max(1);
    let xml = read_xml(path)?;
    let (descriptors, version, errors) = parse_mesh_xml(&xml)?;

    let mut stats = ImportStats {
        errors,
        ..Default::default()
    };

    if dry_run {
        stats.code_systems = 1;
        stats.concepts = descriptors.len() as u32;
        eprintln!(
            "[mesh] dry-run — {} descriptors parsed (version {}), no DB writes",
            descriptors.len(),
            version.as_deref().unwrap_or("unknown")
        );
        return Ok(stats);
    }

    let conn = pool
        .get()
        .map_err(|e| HtsError::Internal(format!("DB pool error: {e}")))?;

    let system_id = upsert_code_system(&conn, version.as_deref())?;
    stats.code_systems = 1;

    // Build tree_number → UI map for hierarchy resolution.
    let tree_to_ui: HashMap<String, String> = descriptors
        .iter()
        .flat_map(|d| d.tree_numbers.iter().map(|t| (t.clone(), d.ui.clone())))
        .collect();

    let total = descriptors.len();
    let total_batches = total.div_ceil(batch_size);

    for (batch_idx, batch) in descriptors.chunks(batch_size).enumerate() {
        insert_concept_batch(&conn, &system_id, batch)?;
        insert_hierarchy_batch(&conn, &system_id, batch, &tree_to_ui)?;

        let inserted = ((batch_idx + 1) * batch_size).min(total);
        eprintln!(
            "[mesh] batch {}/{total_batches} — +{} descriptors (total: {inserted})",
            batch_idx + 1,
            batch.len()
        );
    }

    stats.concepts = total as u32;
    Ok(stats)
}

// ── File reader ───────────────────────────────────────────────────────────────

fn read_xml(path: &Path) -> Result<String, HtsError> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    if name.ends_with(".gz") {
        read_xml_from_gz(path)
    } else if name.ends_with(".zip") {
        read_xml_from_zip(path)
    } else {
        std::fs::read_to_string(path)
            .map_err(|e| HtsError::InvalidRequest(format!("Cannot read '{}': {e}", path.display())))
    }
}

fn read_xml_from_gz(path: &Path) -> Result<String, HtsError> {
    use flate2::read::GzDecoder;
    let file = std::fs::File::open(path)
        .map_err(|e| HtsError::InvalidRequest(format!("Cannot open '{}': {e}", path.display())))?;
    let mut decoder = GzDecoder::new(file);
    let mut buf = String::new();
    decoder.read_to_string(&mut buf).map_err(|e| {
        HtsError::InvalidRequest(format!("Cannot decompress '{}': {e}", path.display()))
    })?;
    Ok(buf)
}

fn read_xml_from_zip(path: &Path) -> Result<String, HtsError> {
    let file = std::fs::File::open(path).map_err(|e| {
        HtsError::InvalidRequest(format!("Cannot open ZIP '{}': {e}", path.display()))
    })?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| HtsError::InvalidRequest(format!("Invalid ZIP '{}': {e}", path.display())))?;

    let best_index = (0..archive.len())
        .find_map(|i| {
            let entry = archive.by_index(i).ok()?;
            let n = entry.name().to_ascii_lowercase();
            if n.ends_with(".xml") && (n.contains("mesh") || n.contains("desc")) {
                Some(i)
            } else {
                None
            }
        })
        .ok_or_else(|| {
            HtsError::InvalidRequest(format!(
                "No MeSH XML file found inside ZIP '{}'. \
                 Expected a file with 'mesh' or 'desc' in the name.",
                path.display()
            ))
        })?;

    let mut entry = archive
        .by_index(best_index)
        .map_err(|e| HtsError::InvalidRequest(format!("Cannot read ZIP entry: {e}")))?;
    let mut buf = String::new();
    entry
        .read_to_string(&mut buf)
        .map_err(|e| HtsError::InvalidRequest(format!("Cannot read XML from ZIP: {e}")))?;
    Ok(buf)
}

// ── Parser ────────────────────────────────────────────────────────────────────

/// Remove a `<!DOCTYPE …>` declaration from XML so `roxmltree` can parse it.
///
/// MeSH XML files include a DTD reference that roxmltree does not support.
/// The declaration spans from `<!DOCTYPE` to the closing `>` or `]>`.
fn strip_doctype(xml: &str) -> String {
    let start = match xml.find("<!DOCTYPE") {
        Some(pos) => pos,
        None => return xml.to_string(),
    };

    // The DOCTYPE block ends at `]>` (internal subset) or the first bare `>`.
    let after = &xml[start..];
    let end = if let Some(p) = after.find("]>") {
        start + p + 2
    } else if let Some(p) = after.find('>') {
        start + p + 1
    } else {
        return xml.to_string();
    };

    format!("{}{}", &xml[..start], &xml[end..])
}

/// Parse a MeSH XML document into a flat list of descriptors.
///
/// Returns `(descriptors, version, non_fatal_errors)`.
/// `version` is derived from the `DescriptorRecordCount` attribute on the root
/// element (the closest thing to a version tag in MeSH XML).
#[allow(clippy::type_complexity)]
fn parse_mesh_xml(
    xml: &str,
) -> Result<(Vec<MeshDescriptor>, Option<String>, Vec<String>), HtsError> {
    // roxmltree does not support DTD; strip the DOCTYPE declaration if present.
    let xml = strip_doctype(xml);
    let doc = roxmltree::Document::parse(&xml)
        .map_err(|e| HtsError::InvalidRequest(format!("Invalid MeSH XML: {e}")))?;

    let root = doc.root_element();
    // DescriptorRecordCount is a rough version indicator (not a formal version string).
    let version = root
        .attribute("DescriptorRecordCount")
        .map(|v| format!("count={v}"));

    let mut descriptors = Vec::new();
    let mut errors = Vec::new();

    for record in root
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "DescriptorRecord")
    {
        let ui = match find_child_text(&record, "DescriptorUI") {
            Some(u) if !u.is_empty() => u,
            _ => {
                errors.push("DescriptorRecord missing DescriptorUI — skipped".to_string());
                continue;
            }
        };

        // DescriptorName/String
        let name = record
            .children()
            .find(|n| n.is_element() && n.tag_name().name() == "DescriptorName")
            .and_then(|n| find_child_text(&n, "String"))
            .unwrap_or_else(|| ui.clone());

        // ScopeNote lives in ConceptList > Concept[PreferredConceptYN=Y] > ScopeNote.
        let scope_note = record
            .children()
            .find(|n| n.is_element() && n.tag_name().name() == "ConceptList")
            .and_then(|cl| {
                cl.children()
                    .find(|n| {
                        n.is_element()
                            && n.tag_name().name() == "Concept"
                            && n.attribute("PreferredConceptYN") == Some("Y")
                    })
                    .and_then(|c| find_child_text(&c, "ScopeNote"))
            });

        // TreeNumberList/TreeNumber
        let tree_numbers: Vec<String> = record
            .children()
            .find(|n| n.is_element() && n.tag_name().name() == "TreeNumberList")
            .map(|tl| {
                tl.children()
                    .filter(|n| n.is_element() && n.tag_name().name() == "TreeNumber")
                    .filter_map(|n| n.text().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();

        descriptors.push(MeshDescriptor {
            ui,
            name,
            scope_note,
            tree_numbers,
        });
    }

    Ok((descriptors, version, errors))
}

/// Find the text of the first child element matching `tag`.
fn find_child_text(node: &roxmltree::Node, tag: &str) -> Option<String> {
    node.children()
        .find(|n| n.is_element() && n.tag_name().name() == tag)
        .and_then(|n| n.text())
        .map(str::to_string)
}

/// Derive the parent tree number by removing the last dot-separated segment.
///
/// `"D03.438.221.173"` → `Some("D03.438.221")`
/// `"D03"` → `None` (top-level)
fn parent_tree_number(tn: &str) -> Option<&str> {
    tn.rfind('.').map(|pos| &tn[..pos])
}

// ── DB helpers ────────────────────────────────────────────────────────────────

/// Insert the MeSH CodeSystem row if absent, then return its `id`.
#[cfg(feature = "sqlite")]
fn upsert_code_system(
    conn: &rusqlite::Connection,
    version: Option<&str>,
) -> Result<String, HtsError> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let ver = version.unwrap_or("current");

    conn.execute(
        "INSERT OR IGNORE INTO code_systems \
         (id, url, version, name, title, status, content, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, 'active', 'complete', ?6, ?6)",
        rusqlite::params![id, MESH_URL, ver, MESH_NAME, MESH_TITLE, now],
    )
    .map_err(|e| HtsError::Internal(format!("Upsert CodeSystem: {e}")))?;

    let system_id: String = conn
        .query_row(
            "SELECT id FROM code_systems WHERE url = ?1",
            rusqlite::params![MESH_URL],
            |row| row.get(0),
        )
        .map_err(|e| HtsError::Internal(format!("Fetch CodeSystem id: {e}")))?;

    Ok(system_id)
}

/// Upsert one batch of MeSH descriptors inside a single transaction.
#[cfg(feature = "sqlite")]
fn insert_concept_batch(
    conn: &rusqlite::Connection,
    system_id: &str,
    batch: &[MeshDescriptor],
) -> Result<(), HtsError> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| HtsError::Internal(format!("Begin transaction: {e}")))?;

    for d in batch {
        tx.execute(
            "INSERT OR REPLACE INTO concepts (system_id, code, display, definition) \
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![system_id, d.ui, d.name, d.scope_note],
        )
        .map_err(|e| HtsError::Internal(format!("Insert concept '{}': {e}", d.ui)))?;
    }

    tx.commit()
        .map_err(|e| HtsError::Internal(format!("Commit: {e}")))?;
    Ok(())
}

/// Insert parent–child edges for one batch of descriptors.
///
/// For each tree number, strips its last dot-segment to get the parent tree
/// number, then looks up the parent's UI in `tree_to_ui`. Descriptors whose
/// parent tree number is not in the map (top-level entries) produce no edge.
#[cfg(feature = "sqlite")]
fn insert_hierarchy_batch(
    conn: &rusqlite::Connection,
    system_id: &str,
    batch: &[MeshDescriptor],
    tree_to_ui: &HashMap<String, String>,
) -> Result<(), HtsError> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| HtsError::Internal(format!("Begin hierarchy transaction: {e}")))?;

    for d in batch {
        for tn in &d.tree_numbers {
            if let Some(parent_tn) = parent_tree_number(tn) {
                if let Some(parent_ui) = tree_to_ui.get(parent_tn) {
                    tx.execute(
                        "INSERT OR IGNORE INTO concept_hierarchy \
                         (system_id, parent_code, child_code) VALUES (?1, ?2, ?3)",
                        rusqlite::params![system_id, parent_ui, d.ui],
                    )
                    .map_err(|e| {
                        HtsError::Internal(format!("Insert hierarchy {parent_ui}->{}: {e}", d.ui))
                    })?;
                }
            }
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

    // Minimal MeSH XML with two descriptors: parent (D000001) and child (D000002).
    const SAMPLE_XML: &str = r#"<?xml version="1.0"?>
<!DOCTYPE DescriptorRecordSet PUBLIC "-//NLM//DTD MeSH 2025//EN" "">
<DescriptorRecordSet DescriptorRecordCount="2">
  <DescriptorRecord DescriptorClass="1">
    <DescriptorUI>D000001</DescriptorUI>
    <DescriptorName><String>Calcimycin</String></DescriptorName>
    <TreeNumberList>
      <TreeNumber>D03.438.221</TreeNumber>
    </TreeNumberList>
    <ConceptList>
      <Concept PreferredConceptYN="Y">
        <ScopeNote>A calcium ionophore.</ScopeNote>
      </Concept>
    </ConceptList>
  </DescriptorRecord>
  <DescriptorRecord DescriptorClass="1">
    <DescriptorUI>D000002</DescriptorUI>
    <DescriptorName><String>Calcimycin Analog</String></DescriptorName>
    <TreeNumberList>
      <TreeNumber>D03.438.221.173</TreeNumber>
    </TreeNumberList>
    <ConceptList>
      <Concept PreferredConceptYN="Y">
        <ScopeNote>An analog.</ScopeNote>
      </Concept>
    </ConceptList>
  </DescriptorRecord>
</DescriptorRecordSet>"#;

    #[test]
    fn parse_returns_correct_count() {
        let (descriptors, _, errors) = parse_mesh_xml(SAMPLE_XML).unwrap();
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
        assert_eq!(descriptors.len(), 2);
    }

    #[test]
    fn parse_extracts_ui_and_name() {
        let (descriptors, _, _) = parse_mesh_xml(SAMPLE_XML).unwrap();
        let d = &descriptors[0];
        assert_eq!(d.ui, "D000001");
        assert_eq!(d.name, "Calcimycin");
    }

    #[test]
    fn parse_extracts_scope_note() {
        let (descriptors, _, _) = parse_mesh_xml(SAMPLE_XML).unwrap();
        assert_eq!(
            descriptors[0].scope_note.as_deref(),
            Some("A calcium ionophore.")
        );
    }

    #[test]
    fn parse_extracts_tree_numbers() {
        let (descriptors, _, _) = parse_mesh_xml(SAMPLE_XML).unwrap();
        assert_eq!(descriptors[1].tree_numbers, vec!["D03.438.221.173"]);
    }

    #[test]
    fn parent_tree_number_strips_last_segment() {
        assert_eq!(parent_tree_number("D03.438.221.173"), Some("D03.438.221"));
        assert_eq!(parent_tree_number("D03.438"), Some("D03"));
        assert_eq!(parent_tree_number("D03"), None);
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

        fn make_xml_file(content: &str) -> tempfile::NamedTempFile {
            let mut f = tempfile::NamedTempFile::with_suffix(".xml").unwrap();
            f.write_all(content.as_bytes()).unwrap();
            f
        }

        #[test]
        fn dry_run_does_not_write() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let f = make_xml_file(SAMPLE_XML);
            let stats = import_mesh(backend.pool(), f.path(), 500, true).unwrap();
            assert_eq!(stats.code_systems, 1);
            assert_eq!(stats.concepts, 2);
            assert_eq!(count(backend.pool(), "code_systems"), 0);
        }

        #[test]
        fn live_import_writes_concepts_and_hierarchy() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let f = make_xml_file(SAMPLE_XML);
            let stats = import_mesh(backend.pool(), f.path(), 500, false).unwrap();
            assert_eq!(stats.code_systems, 1);
            assert_eq!(stats.concepts, 2);
            assert_eq!(count(backend.pool(), "concepts"), 2);
            // D000002 is child of D000001 via tree-number hierarchy.
            assert_eq!(count(backend.pool(), "concept_hierarchy"), 1);
        }

        #[test]
        fn idempotent_reimport() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let f = make_xml_file(SAMPLE_XML);
            import_mesh(backend.pool(), f.path(), 500, false).unwrap();
            import_mesh(backend.pool(), f.path(), 500, false).unwrap();
            assert_eq!(count(backend.pool(), "code_systems"), 1);
            assert_eq!(count(backend.pool(), "concepts"), 2);
            assert_eq!(count(backend.pool(), "concept_hierarchy"), 1);
        }
    }
}
