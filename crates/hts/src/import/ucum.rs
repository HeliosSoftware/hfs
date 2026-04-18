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

use std::io::Read;
use std::path::Path;

use helios_persistence::tenant::TenantContext;

use crate::error::HtsError;
use crate::import::BundleImportBackend;
use crate::import::ImportStats;
use crate::import::bundle_builder::{BuilderConcept, CodeSystemMeta, build_code_system_bundle};

// ── Constants ─────────────────────────────────────────────────────────────────

const UCUM_URL: &str = "http://unitsofmeasure.org";
const UCUM_ID: &str = "ucum";
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

/// Result of parsing a UCUM XML file.
#[derive(Debug)]
struct UcumParseResult {
    concepts: Vec<UcumConcept>,
    version: Option<String>,
    errors: Vec<String>,
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Import a UCUM `ucum-essence.xml` distribution through the given backend.
///
/// `path` may point to:
/// - The raw `ucum-essence.xml` file, or
/// - A `.zip` archive containing a file with "ucum" or "essence" in the name.
pub async fn import_ucum(
    backend: &dyn BundleImportBackend,
    ctx: &TenantContext,
    path: &Path,
    batch_size: usize,
    dry_run: bool,
) -> Result<ImportStats, HtsError> {
    let batch_size = batch_size.max(1);

    let path_owned = path.to_path_buf();
    let parsed = tokio::task::spawn_blocking(move || -> Result<UcumParseResult, HtsError> {
        let xml = read_xml(&path_owned)?;
        parse_ucum_xml(&xml)
    })
    .await
    .map_err(|e| HtsError::Internal(format!("UCUM parser panicked: {e}")))??;

    let UcumParseResult {
        concepts,
        version,
        errors,
    } = parsed;

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

    // First bundle carries the virtual root concept so every real code can
    // reference it as `parent`.
    let version_str = version.as_deref().unwrap_or("current");
    let total = concepts.len();
    let total_batches = total.div_ceil(batch_size).max(1);

    // Seed bundle: CodeSystem metadata + the virtual root.
    let root = BuilderConcept {
        code: ROOT_CODE,
        display: Some(UCUM_TITLE),
        definition: Some("header"),
        ..Default::default()
    };
    let seed_bytes = build_code_system_bundle(
        &CodeSystemMeta {
            id: UCUM_ID,
            url: UCUM_URL,
            version: Some(version_str),
            name: Some(UCUM_NAME),
            title: Some(UCUM_TITLE),
            status: "active",
            content: "complete",
        },
        std::slice::from_ref(&root),
    );
    let seed_stats = backend.import_bundle(ctx, &seed_bytes).await?;
    stats.code_systems = seed_stats.code_systems;
    stats.errors.extend(seed_stats.errors);

    for (batch_idx, batch) in concepts.chunks(batch_size).enumerate() {
        let builder_concepts: Vec<BuilderConcept<'_>> = batch
            .iter()
            .map(|c| BuilderConcept {
                code: &c.code,
                display: Some(&c.display),
                parent_code: Some(ROOT_CODE),
                ..Default::default()
            })
            .collect();

        let bytes = build_code_system_bundle(
            &CodeSystemMeta {
                id: UCUM_ID,
                url: UCUM_URL,
                version: Some(version_str),
                name: Some(UCUM_NAME),
                title: Some(UCUM_TITLE),
                status: "active",
                content: "complete",
            },
            &builder_concepts,
        );

        let chunk_stats = backend.import_bundle(ctx, &bytes).await?;
        stats.errors.extend(chunk_stats.errors);

        eprintln!(
            "[ucum] batch {}/{total_batches} — +{} codes",
            batch_idx + 1,
            batch.len(),
        );
    }

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
fn parse_ucum_xml(xml: &str) -> Result<UcumParseResult, HtsError> {
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

    Ok(UcumParseResult {
        concepts,
        version,
        errors,
    })
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
        let r = parse_ucum_xml(SAMPLE_XML).unwrap();
        assert!(r.errors.is_empty(), "unexpected errors: {:?}", r.errors);
        assert_eq!(r.concepts.len(), 3);
        assert_eq!(r.version.as_deref(), Some("2.1"));
    }

    #[test]
    fn parse_extracts_display_names() {
        let r = parse_ucum_xml(SAMPLE_XML).unwrap();
        let find = |code: &str| {
            r.concepts
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
        let r = parse_ucum_xml(xml).unwrap();
        assert_eq!(r.concepts.len(), 1);
        assert_eq!(r.errors.len(), 1);
        assert!(r.errors[0].contains("missing Code attribute"));
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
            let stats = import_ucum(&backend, &ctx, f.path(), 500, true)
                .await
                .unwrap();
            assert_eq!(stats.code_systems, 1);
            assert_eq!(stats.concepts, 3);
            assert_eq!(count(&backend, "code_systems"), 0);
            assert_eq!(count(&backend, "concepts"), 0);
        }

        #[tokio::test]
        async fn live_import_writes_concepts_and_hierarchy() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let ctx = TenantContext::system();
            let f = make_xml_file(SAMPLE_XML);
            let stats = import_ucum(&backend, &ctx, f.path(), 500, false)
                .await
                .unwrap();
            assert_eq!(stats.code_systems, 1);
            assert_eq!(stats.concepts, 3);
            // virtual root + 3 unit codes
            assert_eq!(count(&backend, "concepts"), 4);
            // 3 flat hierarchy edges (UCUM → each code)
            assert_eq!(count(&backend, "concept_hierarchy"), 3);
        }

        #[tokio::test]
        async fn idempotent_reimport() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let ctx = TenantContext::system();
            let f = make_xml_file(SAMPLE_XML);
            import_ucum(&backend, &ctx, f.path(), 500, false)
                .await
                .unwrap();
            import_ucum(&backend, &ctx, f.path(), 500, false)
                .await
                .unwrap();
            assert_eq!(count(&backend, "code_systems"), 1);
            assert_eq!(count(&backend, "concepts"), 4);
            assert_eq!(count(&backend, "concept_hierarchy"), 3);
        }

        #[tokio::test]
        async fn batching_preserves_all_concepts() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let ctx = TenantContext::system();
            let f = make_xml_file(SAMPLE_XML);
            let stats = import_ucum(&backend, &ctx, f.path(), 1, false)
                .await
                .unwrap();
            assert_eq!(stats.concepts, 3);
            assert_eq!(count(&backend, "concepts"), 4);
        }
    }
}
