//! ICD-10-CM tabular XML importer.
//!
//! Parses the CMS tabular XML distribution file (`icd10cm_tabular_YYYY.xml`)
//! and imports all ICD-10-CM diagnosis codes, short/long descriptions, and
//! the parent–child hierarchy into the HTS normalized schema.
//!
//! # No license required
//!
//! ICD-10-CM is maintained by CMS and the CDC and distributed free of charge
//! with no registration or license agreement required.
//! Download from: <https://www.cms.gov/medicare/coding-billing/icd-10-codes>

use std::io::Read;
use std::path::Path;

use helios_persistence::tenant::TenantContext;

use crate::error::HtsError;
use crate::import::BundleImportBackend;
use crate::import::ImportStats;
use crate::import::bundle_builder::{BuilderConcept, CodeSystemMeta, build_code_system_bundle};

// ── ICD-10-CM constants ───────────────────────────────────────────────────────

const ICD10CM_URL: &str = "http://hl7.org/fhir/sid/icd-10-cm";
const ICD10CM_ID: &str = "icd10cm";
const ICD10CM_NAME: &str = "ICD-10-CM";
const ICD10CM_TITLE: &str =
    "ICD-10-CM (International Classification of Diseases, 10th Revision, Clinical Modification)";

// ── Data types ────────────────────────────────────────────────────────────────

#[derive(Debug)]
struct Icd10Concept {
    code: String,
    display: String,
    /// `true` for leaf codes (no child `<diag>`) — directly billable.
    billable: bool,
    /// Parent code, or `None` for the virtual root.
    parent: Option<String>,
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Import an ICD-10-CM tabular XML file through the given backend.
pub async fn import_icd10_cm(
    backend: &dyn BundleImportBackend,
    ctx: &TenantContext,
    path: &Path,
    batch_size: usize,
    dry_run: bool,
) -> Result<ImportStats, HtsError> {
    let batch_size = batch_size.max(1);

    let path_owned = path.to_path_buf();
    let (concepts, errors) = tokio::task::spawn_blocking(
        move || -> Result<(Vec<Icd10Concept>, Vec<String>), HtsError> {
            let xml_bytes = read_xml_bytes(&path_owned)?;
            parse_tabular_xml(&xml_bytes)
        },
    )
    .await
    .map_err(|e| HtsError::Internal(format!("ICD-10-CM parser panicked: {e}")))??;

    let mut stats = ImportStats {
        errors,
        ..Default::default()
    };

    if dry_run {
        stats.code_systems = 1;
        stats.concepts = concepts.len() as u32;
        eprintln!(
            "[icd10-cm] dry-run — {} concepts parsed, no DB writes",
            concepts.len()
        );
        return Ok(stats);
    }

    let meta = CodeSystemMeta {
        id: ICD10CM_ID,
        url: ICD10CM_URL,
        version: Some("current"),
        name: Some(ICD10CM_NAME),
        title: Some(ICD10CM_TITLE),
        status: "active",
        content: "complete",
    };

    // Seed with an empty CodeSystem to register metadata.
    let seed_bytes = build_code_system_bundle(&meta, &[]);
    let seed_stats = backend.import_bundle(ctx, &seed_bytes).await?;
    stats.code_systems = seed_stats.code_systems;
    stats.errors.extend(seed_stats.errors);

    let total = concepts.len();
    let total_batches = total.div_ceil(batch_size).max(1);

    for (batch_idx, batch) in concepts.chunks(batch_size).enumerate() {
        let builder: Vec<BuilderConcept<'_>> = batch
            .iter()
            .map(|c| BuilderConcept {
                code: &c.code,
                display: Some(c.display.as_str()).filter(|s| !s.is_empty()),
                definition: if c.billable { None } else { Some("header") },
                parent_code: c.parent.as_deref(),
                ..Default::default()
            })
            .collect();
        let bytes = build_code_system_bundle(&meta, &builder);
        let chunk = backend.import_bundle(ctx, &bytes).await?;
        stats.errors.extend(chunk.errors);

        eprintln!(
            "[icd10-cm] batch {}/{total_batches} — +{} concepts (total: {})",
            batch_idx + 1,
            batch.len(),
            ((batch_idx + 1) * batch_size).min(total),
        );
    }

    stats.concepts = total as u32;
    Ok(stats)
}

// ── XML reader ────────────────────────────────────────────────────────────────

fn read_xml_bytes(path: &Path) -> Result<Vec<u8>, HtsError> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    if ext == "zip" {
        let file = std::fs::File::open(path).map_err(|e| {
            HtsError::InvalidRequest(format!("Cannot open ZIP '{p}': {e}", p = path.display()))
        })?;
        let mut archive = zip::ZipArchive::new(file).map_err(|e| {
            HtsError::InvalidRequest(format!("Invalid ZIP '{p}': {e}", p = path.display()))
        })?;

        let xml_index = (0..archive.len())
            .find(|&i| {
                archive
                    .by_index(i)
                    .ok()
                    .map(|e| {
                        let n = e.name().to_ascii_lowercase();
                        n.ends_with(".xml") && n.contains("tabular")
                    })
                    .unwrap_or(false)
            })
            .ok_or_else(|| {
                HtsError::InvalidRequest(format!(
                    "No '*tabular*.xml' file found inside ZIP '{}'",
                    path.display()
                ))
            })?;

        let mut entry = archive
            .by_index(xml_index)
            .map_err(|e| HtsError::InvalidRequest(format!("Cannot read ZIP entry: {e}")))?;
        let mut buf = Vec::new();
        entry
            .read_to_end(&mut buf)
            .map_err(|e| HtsError::InvalidRequest(format!("Cannot read XML from ZIP: {e}")))?;
        Ok(buf)
    } else {
        std::fs::read(path).map_err(|e| {
            HtsError::InvalidRequest(format!("Cannot read file '{}': {e}", path.display()))
        })
    }
}

// ── XML parser ────────────────────────────────────────────────────────────────

fn parse_tabular_xml(xml: &[u8]) -> Result<(Vec<Icd10Concept>, Vec<String>), HtsError> {
    let text = std::str::from_utf8(xml)
        .map_err(|e| HtsError::InvalidRequest(format!("XML is not valid UTF-8: {e}")))?;

    let doc = roxmltree::Document::parse(text)
        .map_err(|e| HtsError::InvalidRequest(format!("XML parse error: {e}")))?;

    let root = doc.root_element();
    let mut concepts: Vec<Icd10Concept> = Vec::new();
    let mut errors: Vec<String> = Vec::new();

    concepts.push(Icd10Concept {
        code: "ICD-10-CM".to_string(),
        display: ICD10CM_TITLE.to_string(),
        billable: false,
        parent: None,
    });

    for chapter in root.children().filter(|n| n.has_tag_name("chapter")) {
        let ch_name = child_text(&chapter, "name").unwrap_or_default();
        let ch_desc = child_text(&chapter, "desc").unwrap_or_default();

        if ch_name.is_empty() {
            errors.push(format!("Chapter missing <name>: {ch_desc}"));
            continue;
        }

        let ch_code = format!("CH:{ch_name}");
        concepts.push(Icd10Concept {
            code: ch_code.clone(),
            display: format!("Chapter {ch_name}: {ch_desc}"),
            billable: false,
            parent: Some("ICD-10-CM".to_string()),
        });

        for section in chapter.children().filter(|n| n.has_tag_name("section")) {
            let sec_id = section.attribute("id").unwrap_or_default().to_string();
            let sec_desc = child_text(&section, "desc").unwrap_or_default();

            if sec_id.is_empty() {
                errors.push(format!(
                    "Section missing id attribute under chapter {ch_name}"
                ));
                continue;
            }

            concepts.push(Icd10Concept {
                code: sec_id.clone(),
                display: sec_desc,
                billable: false,
                parent: Some(ch_code.clone()),
            });

            for diag in section.children().filter(|n| n.has_tag_name("diag")) {
                walk_diag(&diag, &sec_id, &mut concepts, &mut errors);
            }
        }
    }

    Ok((concepts, errors))
}

fn walk_diag(
    node: &roxmltree::Node,
    parent_code: &str,
    concepts: &mut Vec<Icd10Concept>,
    errors: &mut Vec<String>,
) {
    let code = match child_text(node, "name") {
        Some(c) if !c.is_empty() => c,
        _ => {
            errors.push(format!("diag missing <name> under parent {parent_code}"));
            return;
        }
    };
    let desc = match child_text(node, "desc") {
        Some(d) => d,
        None => {
            errors.push(format!(
                "diag <{code}> missing <desc> — imported with empty display"
            ));
            String::new()
        }
    };

    let children: Vec<_> = node.children().filter(|n| n.has_tag_name("diag")).collect();
    let billable = children.is_empty();

    concepts.push(Icd10Concept {
        code: code.clone(),
        display: desc,
        billable,
        parent: Some(parent_code.to_string()),
    });

    for child in children {
        walk_diag(&child, &code, concepts, errors);
    }
}

fn child_text(node: &roxmltree::Node, tag: &str) -> Option<String> {
    node.children()
        .find(|n| n.has_tag_name(tag))
        .and_then(|n| n.text())
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    use super::*;
    use crate::backends::SqliteTerminologyBackend;

    const TABULAR_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<ICD10CM.tabular>
  <chapter>
    <name>I</name>
    <desc>Certain infectious and parasitic diseases</desc>
    <section id="A00-A09">
      <desc>Intestinal infectious diseases</desc>
      <diag>
        <name>A00</name>
        <desc>Cholera</desc>
        <diag>
          <name>A00.0</name>
          <desc>Cholera due to Vibrio cholerae 01, biovar cholerae</desc>
        </diag>
        <diag>
          <name>A00.1</name>
          <desc>Cholera due to Vibrio cholerae 01, biovar eltor</desc>
        </diag>
        <diag>
          <name>A00.9</name>
          <desc>Cholera, unspecified</desc>
        </diag>
      </diag>
    </section>
  </chapter>
  <chapter>
    <name>II</name>
    <desc>Neoplasms</desc>
    <section id="C00-C14">
      <desc>Malignant neoplasms of lip, oral cavity and pharynx</desc>
      <diag>
        <name>C00</name>
        <desc>Malignant neoplasm of lip</desc>
        <diag>
          <name>C00.0</name>
          <desc>Malignant neoplasm of external upper lip</desc>
        </diag>
      </diag>
    </section>
  </chapter>
</ICD10CM.tabular>"#;

    fn count_rows(backend: &SqliteTerminologyBackend, table: &str) -> i64 {
        let conn = backend.pool().get().unwrap();
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
    }

    // ── XML parser unit tests ─────────────────────────────────────────────

    #[test]
    fn parse_xml_returns_correct_concept_count() {
        let (concepts, errors) = parse_tabular_xml(TABULAR_XML.as_bytes()).unwrap();
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
        assert_eq!(concepts.len(), 11);
    }

    #[test]
    fn parse_xml_root_concept_has_no_parent() {
        let (concepts, _) = parse_tabular_xml(TABULAR_XML.as_bytes()).unwrap();
        let root = concepts.iter().find(|c| c.code == "ICD-10-CM").unwrap();
        assert!(root.parent.is_none());
        assert!(!root.billable);
    }

    #[test]
    fn parse_xml_chapter_parent_is_root() {
        let (concepts, _) = parse_tabular_xml(TABULAR_XML.as_bytes()).unwrap();
        let ch = concepts.iter().find(|c| c.code == "CH:I").unwrap();
        assert_eq!(ch.parent.as_deref(), Some("ICD-10-CM"));
    }

    #[test]
    fn parse_xml_section_parent_is_chapter() {
        let (concepts, _) = parse_tabular_xml(TABULAR_XML.as_bytes()).unwrap();
        let sec = concepts.iter().find(|c| c.code == "A00-A09").unwrap();
        assert_eq!(sec.parent.as_deref(), Some("CH:I"));
    }

    #[test]
    fn parse_xml_header_diag_is_not_billable() {
        let (concepts, _) = parse_tabular_xml(TABULAR_XML.as_bytes()).unwrap();
        let a00 = concepts.iter().find(|c| c.code == "A00").unwrap();
        assert!(!a00.billable);
        assert_eq!(a00.parent.as_deref(), Some("A00-A09"));
    }

    #[test]
    fn parse_xml_leaf_diag_is_billable() {
        let (concepts, _) = parse_tabular_xml(TABULAR_XML.as_bytes()).unwrap();
        let a001 = concepts.iter().find(|c| c.code == "A00.1").unwrap();
        assert!(a001.billable);
        assert_eq!(a001.parent.as_deref(), Some("A00"));
    }

    #[test]
    fn parse_xml_invalid_bytes_returns_error() {
        let result = parse_tabular_xml(b"\xFF\xFE not utf8");
        assert!(result.is_err());
    }

    #[test]
    fn parse_xml_malformed_xml_returns_error() {
        let result = parse_tabular_xml(b"<not-closed");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn import_icd10_missing_desc_recorded_in_errors() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<ICD10CM.tabular>
  <chapter>
    <name>I</name>
    <desc>Infectious diseases</desc>
    <section id="A00-A09">
      <desc>Intestinal diseases</desc>
      <diag>
        <name>A00</name>
      </diag>
    </section>
  </chapter>
</ICD10CM.tabular>"#;

        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();

        let mut tmp = tempfile::NamedTempFile::with_suffix(".xml").unwrap();
        use std::io::Write;
        tmp.write_all(xml.as_bytes()).unwrap();

        let stats = import_icd10_cm(&backend, &ctx, tmp.path(), 500, true)
            .await
            .unwrap();

        assert_eq!(stats.errors.len(), 1);
        assert!(stats.errors[0].contains("A00"));
        assert!(stats.errors[0].contains("missing <desc>"));
        assert!(stats.concepts > 0);
    }

    fn make_xml_file() -> tempfile::NamedTempFile {
        use std::io::Write;
        let mut f = tempfile::NamedTempFile::with_suffix(".xml").unwrap();
        f.write_all(TABULAR_XML.as_bytes()).unwrap();
        f
    }

    #[tokio::test]
    async fn import_icd10_dry_run_returns_correct_counts() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();
        let f = make_xml_file();

        let stats = import_icd10_cm(&backend, &ctx, f.path(), 500, true)
            .await
            .unwrap();

        assert_eq!(stats.code_systems, 1);
        assert_eq!(stats.concepts, 11);
        assert!(stats.errors.is_empty());
    }

    #[tokio::test]
    async fn import_icd10_dry_run_does_not_write_to_db() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();
        let f = make_xml_file();

        import_icd10_cm(&backend, &ctx, f.path(), 500, true)
            .await
            .unwrap();

        assert_eq!(count_rows(&backend, "code_systems"), 0);
        assert_eq!(count_rows(&backend, "concepts"), 0);
    }

    #[tokio::test]
    async fn import_icd10_live_writes_to_db() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();
        let f = make_xml_file();

        let stats = import_icd10_cm(&backend, &ctx, f.path(), 500, false)
            .await
            .unwrap();

        assert_eq!(stats.code_systems, 1);
        assert_eq!(stats.concepts, 11);
        assert_eq!(count_rows(&backend, "code_systems"), 1);
        assert_eq!(count_rows(&backend, "concepts"), 11);
        assert_eq!(count_rows(&backend, "concept_hierarchy"), 10);
    }

    #[tokio::test]
    async fn import_icd10_idempotent_reimport() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();
        let f = make_xml_file();

        import_icd10_cm(&backend, &ctx, f.path(), 500, false)
            .await
            .unwrap();
        import_icd10_cm(&backend, &ctx, f.path(), 500, false)
            .await
            .unwrap();

        assert_eq!(count_rows(&backend, "code_systems"), 1);
        assert_eq!(count_rows(&backend, "concepts"), 11);
        assert_eq!(count_rows(&backend, "concept_hierarchy"), 10);
    }

    #[tokio::test]
    async fn import_icd10_batching_preserves_all_concepts() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();
        let f = make_xml_file();

        let stats = import_icd10_cm(&backend, &ctx, f.path(), 3, false)
            .await
            .unwrap();

        assert_eq!(stats.concepts, 11);
        assert_eq!(count_rows(&backend, "concepts"), 11);
        assert_eq!(count_rows(&backend, "concept_hierarchy"), 10);
    }

    #[tokio::test]
    async fn import_icd10_missing_file_returns_error() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();
        let result = import_icd10_cm(
            &backend,
            &ctx,
            Path::new("/nonexistent/tabular.xml"),
            500,
            false,
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn import_icd10_lookup_known_code() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();
        let f = make_xml_file();

        import_icd10_cm(&backend, &ctx, f.path(), 500, false)
            .await
            .unwrap();

        let conn = backend.pool().get().unwrap();
        let (display, definition): (String, Option<String>) = conn
            .query_row(
                "SELECT display, definition FROM concepts WHERE code = ?1",
                rusqlite::params!["A00.9"],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();

        assert_eq!(display, "Cholera, unspecified");
        assert!(definition.is_none());
    }

    #[tokio::test]
    async fn import_icd10_header_code_has_header_definition() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();
        let f = make_xml_file();

        import_icd10_cm(&backend, &ctx, f.path(), 500, false)
            .await
            .unwrap();

        let conn = backend.pool().get().unwrap();
        let definition: Option<String> = conn
            .query_row(
                "SELECT definition FROM concepts WHERE code = ?1",
                rusqlite::params!["A00"],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(definition.as_deref(), Some("header"));
    }

    #[tokio::test]
    async fn import_icd10_from_zip() {
        use std::io::Write;

        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("icd10cm_tabular_2025.zip");

        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let opts = zip::write::FileOptions::default();
            zip.start_file("icd10cm_tabular_2025.xml", opts).unwrap();
            zip.write_all(TABULAR_XML.as_bytes()).unwrap();
            zip.finish().unwrap();
        }

        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();

        let stats = import_icd10_cm(&backend, &ctx, &zip_path, 500, false)
            .await
            .unwrap();
        assert_eq!(stats.concepts, 11);
        assert_eq!(count_rows(&backend, "concepts"), 11);
    }
}
