//! UCUM (Unified Code for Units of Measure) importer.
//!
//! Parses the UCUM `ucum-essence.xml` distribution and imports all unit codes
//! into the HTS normalized schema.
//!
//! # No license required
//!
//! UCUM is developed by the Regenstrief Institute and HL7 International under a
//! permissive open license. The `ucum-essence.xml` file is freely available at:
//! <https://github.com/ucum-org/ucum/releases>
//!
//! UCUM is also bundled inside the HL7 THO NPM package — if you have already
//! run `hts import <tgz>`, no separate import is needed.
//!
//! # File format
//!
//! The UCUM XML uses the `http://unitsofmeasure.org/ucum-essence` namespace and
//! contains three element types, each carrying a `Code` attribute and a `<name>`
//! child:
//!
//! - `<prefix>` — SI prefixes (kilo, mega, milli, …)
//! - `<base-unit>` — fundamental SI base units (meter, gram, second, …)
//! - `<unit>` — all other units (litre, inch, mmHg, …)
//!
//! # Hierarchy
//!
//! UCUM codes form a flat list. All codes are placed as children of a virtual
//! root `UCUM` concept in the `concept_hierarchy` table.

#[cfg(feature = "sqlite")]
use r2d2::Pool;
#[cfg(feature = "sqlite")]
use r2d2_sqlite::SqliteConnectionManager;

use std::io::Read;
use std::path::Path;

use crate::error::HtsError;
use crate::import::ImportStats;

// ── Constants ─────────────────────────────────────────────────────────────────

const UCUM_URL: &str = "http://unitsofmeasure.org";
const UCUM_NAME: &str = "UCUM";
const UCUM_TITLE: &str = "Unified Code for Units of Measure (UCUM)";
const ROOT_CODE: &str = "UCUM";

// ── Data types ────────────────────────────────────────────────────────────────

#[derive(Debug)]
struct UcumConcept {
    /// UCUM unit code, e.g. `m`, `kg`, `[lb_av]`.
    code: String,
    /// Human-readable name taken from the `<name>` child element.
    display: String,
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Import a UCUM `ucum-essence.xml` distribution into the HTS database.
///
/// `path` may point to:
/// - The raw `ucum-essence.xml` file, or
/// - A `.zip` archive containing a file with "ucum" or "essence" in the name.
#[cfg(feature = "sqlite")]
pub fn import_ucum(
    pool: &Pool<SqliteConnectionManager>,
    path: &Path,
    batch_size: usize,
    dry_run: bool,
) -> Result<ImportStats, HtsError> {
    let batch_size = batch_size.max(1);
    let xml = read_xml(path)?;
    let (concepts, version, errors) = parse_ucum_xml(&xml)?;

    let mut stats = ImportStats {
        errors,
        ..Default::default()
    };

    if dry_run {
        stats.code_systems = 1;
        stats.concepts = concepts.len() as u32;
        eprintln!(
            "[ucum] dry-run — {} codes parsed (version {}), no DB writes",
            concepts.len(),
            version.as_deref().unwrap_or("unknown")
        );
        return Ok(stats);
    }

    let conn = pool
        .get()
        .map_err(|e| HtsError::Internal(format!("DB pool error: {e}")))?;

    let system_id = upsert_code_system(&conn, version.as_deref())?;
    stats.code_systems = 1;

    // Virtual root so hierarchy edges have a valid parent.
    conn.execute(
        "INSERT OR IGNORE INTO concepts (system_id, code, display, definition) \
         VALUES (?1, ?2, ?3, 'header')",
        rusqlite::params![system_id, ROOT_CODE, UCUM_TITLE],
    )
    .map_err(|e| HtsError::Internal(format!("Insert virtual root: {e}")))?;

    let total = concepts.len();
    let total_batches = total.div_ceil(batch_size);

    for (batch_idx, batch) in concepts.chunks(batch_size).enumerate() {
        insert_concept_batch(&conn, &system_id, batch)?;

        let inserted = ((batch_idx + 1) * batch_size).min(total);
        eprintln!(
            "[ucum] batch {}/{total_batches} — +{} codes (total: {inserted})",
            batch_idx + 1,
            batch.len()
        );
    }

    // All codes hang off the virtual root (UCUM is a flat enumeration).
    insert_flat_hierarchy(&conn, &system_id, &concepts)?;

    stats.concepts = total as u32;
    Ok(stats)
}

// ── File reader ───────────────────────────────────────────────────────────────

fn read_xml(path: &Path) -> Result<String, HtsError> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    if ext == "zip" {
        read_xml_from_zip(path)
    } else {
        std::fs::read_to_string(path)
            .map_err(|e| HtsError::InvalidRequest(format!("Cannot read '{}': {e}", path.display())))
    }
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
            let name = entry.name().to_ascii_lowercase();
            if name.ends_with(".xml") && (name.contains("ucum") || name.contains("essence")) {
                Some(i)
            } else {
                None
            }
        })
        .ok_or_else(|| {
            HtsError::InvalidRequest(format!(
                "No UCUM XML file found inside ZIP '{}'. \
                 Expected a file containing 'ucum' or 'essence' in the name.",
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

/// Parse UCUM essence XML into a flat list of concepts.
///
/// Returns `(concepts, version, non_fatal_errors)`.
/// `version` is taken from the `version` attribute on the root element.
#[allow(clippy::type_complexity)]
fn parse_ucum_xml(xml: &str) -> Result<(Vec<UcumConcept>, Option<String>, Vec<String>), HtsError> {
    let doc = roxmltree::Document::parse(xml)
        .map_err(|e| HtsError::InvalidRequest(format!("Invalid UCUM XML: {e}")))?;

    let root = doc.root_element();
    let version = root.attribute("version").map(str::to_string);

    let mut concepts = Vec::new();
    let mut errors = Vec::new();

    for node in root.children().filter(|n| n.is_element()) {
        let tag = node.tag_name().name();
        if !matches!(tag, "prefix" | "base-unit" | "unit") {
            continue;
        }

        let code = match node.attribute("Code") {
            Some(c) if !c.is_empty() => c.to_string(),
            _ => {
                errors.push(format!("<{tag}> element missing Code attribute — skipped"));
                continue;
            }
        };

        let display = node
            .children()
            .find(|n| n.is_element() && n.tag_name().name() == "name")
            .and_then(|n| n.text())
            .unwrap_or(&code)
            .to_string();

        concepts.push(UcumConcept { code, display });
    }

    Ok((concepts, version, errors))
}

// ── DB helpers ────────────────────────────────────────────────────────────────

/// Insert the UCUM CodeSystem row if absent, then return its `id`.
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
        rusqlite::params![id, UCUM_URL, ver, UCUM_NAME, UCUM_TITLE, now],
    )
    .map_err(|e| HtsError::Internal(format!("Upsert CodeSystem: {e}")))?;

    let system_id: String = conn
        .query_row(
            "SELECT id FROM code_systems WHERE url = ?1",
            rusqlite::params![UCUM_URL],
            |row| row.get(0),
        )
        .map_err(|e| HtsError::Internal(format!("Fetch CodeSystem id: {e}")))?;

    Ok(system_id)
}

/// Upsert one batch of UCUM concepts inside a single transaction.
#[cfg(feature = "sqlite")]
fn insert_concept_batch(
    conn: &rusqlite::Connection,
    system_id: &str,
    batch: &[UcumConcept],
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
    concepts: &[UcumConcept],
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
        .map_err(|e| HtsError::Internal(format!("Insert hierarchy UCUM->{}: {e}", c.code)))?;
    }

    tx.commit()
        .map_err(|e| HtsError::Internal(format!("Commit hierarchy: {e}")))?;
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<root xmlns="http://unitsofmeasure.org/ucum-essence" version="2.1">
  <prefix Code="k" CODE="K">
    <name>kilo</name>
    <printSymbol>k</printSymbol>
    <value value="1e3"/>
  </prefix>
  <base-unit Code="m" CODE="M" isMetric="yes" class="si">
    <name>meter</name>
    <printSymbol>m</printSymbol>
    <property>length</property>
  </base-unit>
  <unit Code="[lb_av]" CODE="[LB_AV]" isMetric="no" class="avoirdupois">
    <name>pound</name>
    <printSymbol>lb</printSymbol>
    <property>mass</property>
    <value value="7000" Unit="[gr]"/>
  </unit>
</root>"#;

    #[test]
    fn parse_returns_all_codes() {
        let (concepts, version, errors) = parse_ucum_xml(SAMPLE_XML).unwrap();
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
        assert_eq!(concepts.len(), 3);
        assert_eq!(version.as_deref(), Some("2.1"));
    }

    #[test]
    fn parse_extracts_display_names() {
        let (concepts, _, _) = parse_ucum_xml(SAMPLE_XML).unwrap();
        let find = |code: &str| {
            concepts
                .iter()
                .find(|c| c.code == code)
                .map(|c| c.display.as_str())
        };
        assert_eq!(find("k"), Some("kilo"));
        assert_eq!(find("m"), Some("meter"));
        assert_eq!(find("[lb_av]"), Some("pound"));
    }

    #[test]
    fn parse_skips_element_without_code_attribute() {
        let xml = r#"<?xml version="1.0"?>
<root version="2.1">
  <unit><name>no-code</name></unit>
  <unit Code="m"><name>meter</name></unit>
</root>"#;
        let (concepts, _, errors) = parse_ucum_xml(xml).unwrap();
        assert_eq!(concepts.len(), 1);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("missing Code attribute"));
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
            let f = make_xml_file(SAMPLE_XML);
            let stats = import_ucum(backend.pool(), f.path(), 500, true).unwrap();
            assert_eq!(stats.code_systems, 1);
            assert_eq!(stats.concepts, 3);
            assert_eq!(count(backend.pool(), "code_systems"), 0);
            assert_eq!(count(backend.pool(), "concepts"), 0);
        }

        #[test]
        fn live_import_writes_concepts_and_hierarchy() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let f = make_xml_file(SAMPLE_XML);
            let stats = import_ucum(backend.pool(), f.path(), 500, false).unwrap();
            assert_eq!(stats.code_systems, 1);
            assert_eq!(stats.concepts, 3);
            // virtual root + 3 unit codes
            assert_eq!(count(backend.pool(), "concepts"), 4);
            // 3 flat hierarchy edges (UCUM → each code)
            assert_eq!(count(backend.pool(), "concept_hierarchy"), 3);
        }

        #[test]
        fn idempotent_reimport() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let f = make_xml_file(SAMPLE_XML);
            import_ucum(backend.pool(), f.path(), 500, false).unwrap();
            import_ucum(backend.pool(), f.path(), 500, false).unwrap();
            assert_eq!(count(backend.pool(), "code_systems"), 1);
            assert_eq!(count(backend.pool(), "concepts"), 4);
            assert_eq!(count(backend.pool(), "concept_hierarchy"), 3);
        }

        #[test]
        fn batching_preserves_all_concepts() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let f = make_xml_file(SAMPLE_XML);
            let stats = import_ucum(backend.pool(), f.path(), 1, false).unwrap();
            assert_eq!(stats.concepts, 3);
            assert_eq!(count(backend.pool(), "concepts"), 4);
        }
    }
}
