//! MeSH (Medical Subject Headings) importer.
//!
//! Parses the NLM MeSH XML distribution and imports all descriptors with
//! tree-number-derived hierarchy into the HTS normalized schema.
//!
//! # No license required
//!
//! MeSH is produced by the U.S. National Library of Medicine (NLM), a US federal
//! agency, and is **public domain**.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use helios_persistence::tenant::TenantContext;

use crate::error::HtsError;
use crate::import::BundleImportBackend;
use crate::import::ImportStats;
use crate::import::bundle_builder::{
    BuilderConcept, BuilderProperty, CodeSystemMeta, build_code_system_bundle,
};

// ── Constants ─────────────────────────────────────────────────────────────────

const MESH_URL: &str = "http://www.nlm.nih.gov/mesh";
const MESH_ID: &str = "mesh";
const MESH_NAME: &str = "MeSH";
const MESH_TITLE: &str = "Medical Subject Headings (MeSH)";

// ── Data types ────────────────────────────────────────────────────────────────

#[derive(Debug)]
struct MeshDescriptor {
    ui: String,
    name: String,
    scope_note: Option<String>,
    tree_numbers: Vec<String>,
}

#[derive(Debug)]
struct MeshParseResult {
    descriptors: Vec<MeshDescriptor>,
    version: Option<String>,
    errors: Vec<String>,
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Import a MeSH XML distribution through the given backend.
pub async fn import_mesh(
    backend: &dyn BundleImportBackend,
    ctx: &TenantContext,
    path: &Path,
    batch_size: usize,
    dry_run: bool,
) -> Result<ImportStats, HtsError> {
    let batch_size = batch_size.max(1);

    let path_owned = path.to_path_buf();
    let parsed = tokio::task::spawn_blocking(move || -> Result<MeshParseResult, HtsError> {
        let xml = read_xml(&path_owned)?;
        parse_mesh_xml(&xml)
    })
    .await
    .map_err(|e| HtsError::Internal(format!("MeSH parser panicked: {e}")))??;

    let MeshParseResult {
        descriptors,
        version,
        errors,
    } = parsed;

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

    // Tree-number → UI map, used to resolve parent edges.
    let tree_to_ui: HashMap<String, String> = descriptors
        .iter()
        .flat_map(|d| d.tree_numbers.iter().map(|t| (t.clone(), d.ui.clone())))
        .collect();

    let version_str = version.as_deref().unwrap_or("current");
    let meta = CodeSystemMeta {
        id: MESH_ID,
        url: MESH_URL,
        version: Some(version_str),
        name: Some(MESH_NAME),
        title: Some(MESH_TITLE),
        status: "active",
        content: "complete",
    };

    // Seed empty CodeSystem.
    let seed = build_code_system_bundle(&meta, &[]);
    let seed_stats = backend.import_bundle(ctx, &seed).await?;
    stats.code_systems = seed_stats.code_systems;
    stats.errors.extend(seed_stats.errors);

    let total = descriptors.len();
    let total_batches = total.div_ceil(batch_size).max(1);

    for (batch_idx, batch) in descriptors.chunks(batch_size).enumerate() {
        // For each descriptor, resolve parent UIs from tree numbers.
        let parents_per_concept: Vec<Vec<String>> = batch
            .iter()
            .map(|d| {
                let mut seen: Vec<String> = Vec::new();
                for tn in &d.tree_numbers {
                    if let Some(parent_tn) = parent_tree_number(tn) {
                        if let Some(parent_ui) = tree_to_ui.get(parent_tn) {
                            if !seen.contains(parent_ui) {
                                seen.push(parent_ui.clone());
                            }
                        }
                    }
                }
                seen
            })
            .collect();

        let extra_props_per_concept: Vec<Vec<BuilderProperty<'_>>> = parents_per_concept
            .iter()
            .map(|parents| {
                parents
                    .iter()
                    .skip(1)
                    .map(|p| BuilderProperty {
                        code: "parent",
                        value_key: "valueCode",
                        value: p.as_str(),
                    })
                    .collect()
            })
            .collect();

        let builder: Vec<BuilderConcept<'_>> = batch
            .iter()
            .enumerate()
            .map(|(i, d)| BuilderConcept {
                code: &d.ui,
                display: Some(&d.name),
                definition: d.scope_note.as_deref(),
                parent_code: parents_per_concept[i].first().map(|s| s.as_str()),
                extra_properties: extra_props_per_concept[i].as_slice(),
                ..Default::default()
            })
            .collect();

        let bytes = build_code_system_bundle(&meta, &builder);
        let chunk = backend.import_bundle(ctx, &bytes).await?;
        stats.errors.extend(chunk.errors);

        eprintln!(
            "[mesh] batch {}/{total_batches} — +{} descriptors (total: {})",
            batch_idx + 1,
            batch.len(),
            ((batch_idx + 1) * batch_size).min(total)
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
                "No MeSH XML file found inside ZIP '{}'.",
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

fn strip_doctype(xml: &str) -> String {
    let start = match xml.find("<!DOCTYPE") {
        Some(pos) => pos,
        None => return xml.to_string(),
    };

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

fn parse_mesh_xml(xml: &str) -> Result<MeshParseResult, HtsError> {
    let xml = strip_doctype(xml);
    let doc = roxmltree::Document::parse(&xml)
        .map_err(|e| HtsError::InvalidRequest(format!("Invalid MeSH XML: {e}")))?;

    let root = doc.root_element();
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

        let name = record
            .children()
            .find(|n| n.is_element() && n.tag_name().name() == "DescriptorName")
            .and_then(|n| find_child_text(&n, "String"))
            .unwrap_or_else(|| ui.clone());

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

    Ok(MeshParseResult {
        descriptors,
        version,
        errors,
    })
}

fn find_child_text(node: &roxmltree::Node, tag: &str) -> Option<String> {
    node.children()
        .find(|n| n.is_element() && n.tag_name().name() == tag)
        .and_then(|n| n.text())
        .map(str::to_string)
}

fn parent_tree_number(tn: &str) -> Option<&str> {
    tn.rfind('.').map(|pos| &tn[..pos])
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

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
        let r = parse_mesh_xml(SAMPLE_XML).unwrap();
        assert!(r.errors.is_empty(), "unexpected errors: {:?}", r.errors);
        assert_eq!(r.descriptors.len(), 2);
    }

    #[test]
    fn parse_extracts_ui_and_name() {
        let r = parse_mesh_xml(SAMPLE_XML).unwrap();
        let d = &r.descriptors[0];
        assert_eq!(d.ui, "D000001");
        assert_eq!(d.name, "Calcimycin");
    }

    #[test]
    fn parse_extracts_scope_note() {
        let r = parse_mesh_xml(SAMPLE_XML).unwrap();
        assert_eq!(
            r.descriptors[0].scope_note.as_deref(),
            Some("A calcium ionophore.")
        );
    }

    #[test]
    fn parse_extracts_tree_numbers() {
        let r = parse_mesh_xml(SAMPLE_XML).unwrap();
        assert_eq!(r.descriptors[1].tree_numbers, vec!["D03.438.221.173"]);
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
            let f = make_xml_file(SAMPLE_XML);
            let stats = import_mesh(&backend, &ctx, f.path(), 500, true)
                .await
                .unwrap();
            assert_eq!(stats.code_systems, 1);
            assert_eq!(stats.concepts, 2);
            assert_eq!(count(&backend, "code_systems"), 0);
        }

        #[tokio::test]
        async fn live_import_writes_concepts_and_hierarchy() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let ctx = TenantContext::system();
            let f = make_xml_file(SAMPLE_XML);
            let stats = import_mesh(&backend, &ctx, f.path(), 500, false)
                .await
                .unwrap();
            assert_eq!(stats.code_systems, 1);
            assert_eq!(stats.concepts, 2);
            assert_eq!(count(&backend, "concepts"), 2);
            assert_eq!(count(&backend, "concept_hierarchy"), 1);
        }

        #[tokio::test]
        async fn idempotent_reimport() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let ctx = TenantContext::system();
            let f = make_xml_file(SAMPLE_XML);
            import_mesh(&backend, &ctx, f.path(), 500, false)
                .await
                .unwrap();
            import_mesh(&backend, &ctx, f.path(), 500, false)
                .await
                .unwrap();
            assert_eq!(count(&backend, "code_systems"), 1);
            assert_eq!(count(&backend, "concepts"), 2);
            assert_eq!(count(&backend, "concept_hierarchy"), 1);
        }
    }
}
