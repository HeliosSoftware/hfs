//! NCI Thesaurus (NCIt) importer.
//!
//! Parses the NCI Thesaurus flat-text distribution (`Thesaurus.txt`) and imports
//! ~170 k biomedical concepts with parent–child hierarchy into the HTS normalized
//! schema.
//!
//! # No license required
//!
//! The NCI Thesaurus is a product of the National Cancer Institute (NCI), a US
//! federal agency, and is **public domain**.

use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use helios_persistence::tenant::TenantContext;

use crate::error::HtsError;
use crate::import::BundleImportBackend;
use crate::import::ImportStats;
use crate::import::bundle_builder::{
    BuilderConcept, BuilderProperty, CodeSystemMeta, build_code_system_bundle,
};

// ── Constants ─────────────────────────────────────────────────────────────────

const NCI_URL: &str = "http://ncicb.nci.nih.gov/xml/owl/EVS/Thesaurus.owl";
const NCI_ID: &str = "nci-thesaurus";
const NCI_NAME: &str = "NCIt";
const NCI_TITLE: &str = "NCI Thesaurus (NCIt)";

// ── Data types ────────────────────────────────────────────────────────────────

#[derive(Debug)]
struct NciConcept {
    code: String,
    display: String,
    definition: Option<String>,
    /// Zero or more parent concept codes. Empty for root concepts.
    parents: Vec<String>,
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Import an NCI Thesaurus flat-text distribution through the given backend.
pub async fn import_nci_thesaurus(
    backend: &dyn BundleImportBackend,
    ctx: &TenantContext,
    path: &Path,
    batch_size: usize,
    dry_run: bool,
) -> Result<ImportStats, HtsError> {
    let batch_size = batch_size.max(1);

    let path_owned = path.to_path_buf();
    let (concepts, errors) = tokio::task::spawn_blocking(
        move || -> Result<(Vec<NciConcept>, Vec<String>), HtsError> {
            let text = read_text(&path_owned)?;
            Ok(parse_thesaurus_txt(&text))
        },
    )
    .await
    .map_err(|e| HtsError::Internal(format!("NCI Thesaurus parser panicked: {e}")))??;

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

    let meta = CodeSystemMeta {
        id: NCI_ID,
        url: NCI_URL,
        version: Some("current"),
        name: Some(NCI_NAME),
        title: Some(NCI_TITLE),
        status: "active",
        content: "complete",
    };

    // Seed empty CodeSystem to register metadata.
    let seed = build_code_system_bundle(&meta, &[]);
    let seed_stats = backend.import_bundle(ctx, &seed).await?;
    stats.code_systems = seed_stats.code_systems;
    stats.errors.extend(seed_stats.errors);

    let total = concepts.len();
    let total_batches = total.div_ceil(batch_size).max(1);

    for (batch_idx, batch) in concepts.chunks(batch_size).enumerate() {
        // Build extra "parent" properties for concepts with >1 parent.
        let extra_props_per_concept: Vec<Vec<BuilderProperty<'_>>> = batch
            .iter()
            .map(|c| {
                c.parents
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
            .zip(extra_props_per_concept.iter())
            .map(|(c, extras)| BuilderConcept {
                code: &c.code,
                display: Some(&c.display),
                definition: c.definition.as_deref(),
                parent_code: c.parents.first().map(|s| s.as_str()),
                extra_properties: extras.as_slice(),
                ..Default::default()
            })
            .collect();

        let bytes = build_code_system_bundle(&meta, &builder);
        let chunk = backend.import_bundle(ctx, &bytes).await?;
        stats.errors.extend(chunk.errors);

        eprintln!(
            "[nci-thesaurus] batch {}/{total_batches} — +{} concepts (total: {})",
            batch_idx + 1,
            batch.len(),
            ((batch_idx + 1) * batch_size).min(total)
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
                "No Thesaurus.txt found inside ZIP '{}'.",
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

fn parse_thesaurus_txt(text: &str) -> (Vec<NciConcept>, Vec<String>) {
    let mut concepts = Vec::new();
    let mut errors = Vec::new();
    let mut reader = BufReader::new(text.as_bytes()).lines().enumerate();

    let first = reader.next();
    if let Some((_, Ok(line))) = first {
        let trimmed = line.trim();
        let first_col = trimmed.split('\t').next().unwrap_or("");
        if looks_like_nci_code(first_col) {
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

fn looks_like_nci_code(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some('C')) && chars.next().map(|c| c.is_ascii_digit()).unwrap_or(false)
}

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

    let display = if cols.len() > 5 && !cols[5].trim().is_empty() {
        cols[5].trim()
    } else {
        cols[1].trim()
    };

    let definition = if cols.len() > 4 && !cols[4].trim().is_empty() {
        Some(cols[4].trim().to_string())
    } else {
        None
    };

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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

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
        let text = "C000001\tOnly One Column\n";
        let (concepts, errors) = parse_thesaurus_txt(text);
        assert_eq!(concepts.len(), 1);
        assert!(errors.is_empty());

        let header_only = "NOCOLS\n";
        let (c2, e2) = parse_thesaurus_txt(header_only);
        assert_eq!(c2.len(), 0);
        assert!(e2.is_empty());

        let bad_nci = "C12345\n";
        let (c3, e3) = parse_thesaurus_txt(bad_nci);
        assert_eq!(c3.len(), 0);
        assert_eq!(e3.len(), 1);
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

        fn make_txt_file(content: &str) -> tempfile::NamedTempFile {
            let mut f = tempfile::NamedTempFile::with_suffix(".txt").unwrap();
            f.write_all(content.as_bytes()).unwrap();
            f
        }

        #[tokio::test]
        async fn dry_run_does_not_write() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let ctx = TenantContext::system();
            let f = make_txt_file(SAMPLE);
            let stats = import_nci_thesaurus(&backend, &ctx, f.path(), 500, true)
                .await
                .unwrap();
            assert_eq!(stats.code_systems, 1);
            assert_eq!(stats.concepts, 4);
            assert_eq!(count(&backend, "code_systems"), 0);
        }

        #[tokio::test]
        async fn live_import_writes_concepts_and_hierarchy() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let ctx = TenantContext::system();
            let f = make_txt_file(SAMPLE);
            let stats = import_nci_thesaurus(&backend, &ctx, f.path(), 500, false)
                .await
                .unwrap();
            assert_eq!(stats.code_systems, 1);
            assert_eq!(stats.concepts, 4);
            assert_eq!(count(&backend, "concepts"), 4);
            assert_eq!(count(&backend, "concept_hierarchy"), 4);
        }

        #[tokio::test]
        async fn idempotent_reimport() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let ctx = TenantContext::system();
            let f = make_txt_file(SAMPLE);
            import_nci_thesaurus(&backend, &ctx, f.path(), 500, false)
                .await
                .unwrap();
            import_nci_thesaurus(&backend, &ctx, f.path(), 500, false)
                .await
                .unwrap();
            assert_eq!(count(&backend, "code_systems"), 1);
            assert_eq!(count(&backend, "concepts"), 4);
            assert_eq!(count(&backend, "concept_hierarchy"), 4);
        }

        #[tokio::test]
        async fn batching_preserves_all_concepts() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let ctx = TenantContext::system();
            let f = make_txt_file(SAMPLE);
            let stats = import_nci_thesaurus(&backend, &ctx, f.path(), 1, false)
                .await
                .unwrap();
            assert_eq!(stats.concepts, 4);
            assert_eq!(count(&backend, "concepts"), 4);
        }
    }
}
