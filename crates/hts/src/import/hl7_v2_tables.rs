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

use std::io::Read;
use std::path::Path;

use helios_persistence::tenant::TenantContext;

use crate::error::HtsError;
use crate::import::BundleImportBackend;
use crate::import::ImportStats;
use crate::import::bundle_builder::{BuilderConcept, CodeSystemMeta, build_code_system_bundle};

// ── Constants ─────────────────────────────────────────────────────────────────

const V2_URL_PREFIX: &str = "http://terminology.hl7.org/CodeSystem/v2-";

// ── Data types ────────────────────────────────────────────────────────────────

#[derive(Debug)]
struct V2Table {
    /// Table ID, e.g. "0001".
    id: String,
    /// Table name, e.g. "Administrative Sex".
    name: String,
    entries: Vec<V2Entry>,
}

#[derive(Debug)]
struct V2Entry {
    code: String,
    display: String,
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Import HL7 v2 table definitions through the given backend. Each table
/// becomes a separate FHIR CodeSystem.
pub async fn import_hl7_v2_tables(
    backend: &dyn BundleImportBackend,
    ctx: &TenantContext,
    path: &Path,
    batch_size: usize,
    dry_run: bool,
) -> Result<ImportStats, HtsError> {
    let batch_size = batch_size.max(1);

    let path_owned = path.to_path_buf();
    let (all_tables, all_errors) =
        tokio::task::spawn_blocking(move || -> Result<(Vec<V2Table>, Vec<String>), HtsError> {
            let xmls = read_xmls(&path_owned)?;

            let mut all_tables: Vec<V2Table> = Vec::new();
            let mut all_errors: Vec<String> = Vec::new();

            for (source_name, xml) in &xmls {
                let (tables, errors) = parse_v2_xml(xml, source_name);
                all_tables.extend(tables);
                all_errors.extend(errors);
            }
            Ok((all_tables, all_errors))
        })
        .await
        .map_err(|e| HtsError::Internal(format!("HL7 v2 tables parser panicked: {e}")))??;

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

    for table in &all_tables {
        let url = format!("{V2_URL_PREFIX}{}", table.id);
        let name = format!("v2-{}", table.id);
        let root_code = format!("v2-{}", table.id);
        let cs_id = format!("v2-{}", table.id);

        let meta = CodeSystemMeta {
            id: &cs_id,
            url: &url,
            version: Some("current"),
            name: Some(&name),
            title: Some(&table.name),
            status: "active",
            content: "complete",
        };

        // Seed bundle: virtual root.
        let root = BuilderConcept {
            code: &root_code,
            display: Some(&table.name),
            definition: Some("header"),
            ..Default::default()
        };
        let seed_bytes = build_code_system_bundle(&meta, std::slice::from_ref(&root));
        let seed_stats = backend.import_bundle(ctx, &seed_bytes).await?;
        stats.code_systems += seed_stats.code_systems;
        stats.errors.extend(seed_stats.errors);

        let total = table.entries.len();
        let total_batches = total.div_ceil(batch_size).max(1);

        for (batch_idx, batch) in table.entries.chunks(batch_size).enumerate() {
            let builder: Vec<BuilderConcept<'_>> = batch
                .iter()
                .map(|e| BuilderConcept {
                    code: &e.code,
                    display: Some(&e.display),
                    parent_code: Some(&root_code),
                    ..Default::default()
                })
                .collect();
            let bytes = build_code_system_bundle(&meta, &builder);
            let chunk = backend.import_bundle(ctx, &bytes).await?;
            stats.errors.extend(chunk.errors);

            eprintln!(
                "[hl7-v2-tables] table {} ({}) batch {}/{total_batches} — +{} codes",
                table.id,
                table.name,
                batch_idx + 1,
                batch.len()
            );
        }

        stats.concepts += total as u32;
    }

    Ok(stats)
}

// ── File reader ───────────────────────────────────────────────────────────────

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

        fn count(backend: &SqliteTerminologyBackend, table: &str) -> i64 {
            backend
                .pool()
                .get()
                .unwrap()
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap()
        }

        fn make_xml_file(content: &str) -> tempfile::NamedTempFile {
            let mut f = tempfile::NamedTempFile::with_suffix(".xml").unwrap();
            f.write_all(content.as_bytes()).unwrap();
            f
        }

        #[tokio::test]
        async fn dry_run_does_not_write() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let ctx = TenantContext::system();
            let f = make_xml_file(SAMPLE_MULTI);
            let stats = import_hl7_v2_tables(&backend, &ctx, f.path(), 500, true)
                .await
                .unwrap();
            assert_eq!(stats.code_systems, 2);
            assert_eq!(stats.concepts, 5);
            assert_eq!(count(&backend, "code_systems"), 0);
        }

        #[tokio::test]
        async fn live_import_writes_two_code_systems() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let ctx = TenantContext::system();
            let f = make_xml_file(SAMPLE_MULTI);
            let stats = import_hl7_v2_tables(&backend, &ctx, f.path(), 500, false)
                .await
                .unwrap();
            assert_eq!(stats.code_systems, 2);
            assert_eq!(stats.concepts, 5);
            assert_eq!(count(&backend, "code_systems"), 2);
            // 2 virtual root concepts + 5 codes = 7
            assert_eq!(count(&backend, "concepts"), 7);
            // 5 flat hierarchy edges
            assert_eq!(count(&backend, "concept_hierarchy"), 5);
        }

        #[tokio::test]
        async fn idempotent_reimport() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let ctx = TenantContext::system();
            let f = make_xml_file(SAMPLE_MULTI);
            import_hl7_v2_tables(&backend, &ctx, f.path(), 500, false)
                .await
                .unwrap();
            import_hl7_v2_tables(&backend, &ctx, f.path(), 500, false)
                .await
                .unwrap();
            assert_eq!(count(&backend, "code_systems"), 2);
            assert_eq!(count(&backend, "concepts"), 7);
            assert_eq!(count(&backend, "concept_hierarchy"), 5);
        }
    }
}
