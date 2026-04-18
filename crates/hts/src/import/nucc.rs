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

use std::collections::HashSet;
use std::io::Read;
use std::path::Path;

use helios_persistence::tenant::TenantContext;

use crate::error::HtsError;
use crate::import::BundleImportBackend;
use crate::import::ImportStats;
use crate::import::bundle_builder::{BuilderConcept, CodeSystemMeta, build_code_system_bundle};

// ── Constants ─────────────────────────────────────────────────────────────────

const NUCC_URL: &str = "http://nucc.org/provider-taxonomy";
const NUCC_ID: &str = "nucc";
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

#[derive(Debug)]
struct SyntheticNode {
    code: String,
    display: String,
    parent: Option<String>,
}

#[derive(Debug)]
struct NuccParseResult {
    concepts: Vec<NuccConcept>,
    synthetic_nodes: Vec<SyntheticNode>,
    errors: Vec<String>,
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Import a NUCC Provider Taxonomy CSV through the given backend.
pub async fn import_nucc(
    backend: &dyn BundleImportBackend,
    ctx: &TenantContext,
    path: &Path,
    batch_size: usize,
    dry_run: bool,
) -> Result<ImportStats, HtsError> {
    let _ = batch_size; // NUCC emits everything in a single Bundle.

    let path_owned = path.to_path_buf();
    let parsed = tokio::task::spawn_blocking(move || -> Result<NuccParseResult, HtsError> {
        let text = read_text(&path_owned)?;
        Ok(parse_nucc_csv(&text))
    })
    .await
    .map_err(|e| HtsError::Internal(format!("NUCC parser panicked: {e}")))??;

    let NuccParseResult {
        concepts,
        synthetic_nodes,
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
            "[nucc] dry-run — {} codes parsed ({} synthetic grouping nodes), no DB writes",
            concepts.len(),
            synthetic_nodes.len()
        );
        return Ok(stats);
    }

    // Assemble one CodeSystem that contains:
    //   - virtual root
    //   - synthetic grouping/classification nodes (each referencing its parent)
    //   - real taxonomy codes (each referencing its classification or grouping)
    let root = BuilderConcept {
        code: ROOT_CODE,
        display: Some(NUCC_TITLE),
        definition: Some("header"),
        ..Default::default()
    };

    let mut builder_concepts: Vec<BuilderConcept<'_>> =
        Vec::with_capacity(1 + synthetic_nodes.len() + concepts.len());
    builder_concepts.push(root);
    for n in &synthetic_nodes {
        builder_concepts.push(BuilderConcept {
            code: &n.code,
            display: Some(&n.display),
            definition: Some("grouping"),
            parent_code: Some(n.parent.as_deref().unwrap_or(ROOT_CODE)),
            ..Default::default()
        });
    }
    for c in &concepts {
        builder_concepts.push(BuilderConcept {
            code: &c.code,
            display: Some(&c.display),
            definition: c.definition.as_deref(),
            parent_code: Some(c.parent.as_deref().unwrap_or(ROOT_CODE)),
            ..Default::default()
        });
    }

    let bytes = build_code_system_bundle(
        &CodeSystemMeta {
            id: NUCC_ID,
            url: NUCC_URL,
            version: Some("current"),
            name: Some(NUCC_NAME),
            title: Some(NUCC_TITLE),
            status: "active",
            content: "complete",
        },
        &builder_concepts,
    );

    let imported = backend.import_bundle(ctx, &bytes).await?;
    stats.code_systems = imported.code_systems;
    stats.concepts = concepts.len() as u32;
    stats.errors.extend(imported.errors);

    eprintln!(
        "[nucc] imported {} codes ({} synthetic nodes)",
        concepts.len(),
        synthetic_nodes.len()
    );

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

fn parse_nucc_csv(text: &str) -> NuccParseResult {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(text.as_bytes());

    let mut concepts = Vec::new();
    let mut errors = Vec::new();

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

        if !grouping.is_empty() && seen_groupings.insert(grouping.clone()) {
            synthetic_nodes.push(SyntheticNode {
                code: grouping.clone(),
                display: grouping.clone(),
                parent: Some(ROOT_CODE.to_string()),
            });
        }

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

        let parent = if !classification.is_empty() {
            Some(classification.clone())
        } else if !grouping.is_empty() {
            Some(grouping.clone())
        } else {
            None
        };

        concepts.push(NuccConcept {
            code,
            display,
            definition,
            parent,
        });
    }

    NuccParseResult {
        concepts,
        synthetic_nodes,
        errors,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_CSV: &str = "\
Code,Grouping,Classification,Specialization,Definition,Effective Date,Deactivation Date,Last Modified Date,Notes,Display Name\n\
101Y00000X,Behavioral Health,Counselor,,A provider trained in counseling.,20020101,,,, Counselor\n\
101YA0400X,Behavioral Health,Counselor,Addiction (Substance Use Disorder),Specializes in addiction.,20020101,,,, Addiction Counselor\n\
207Q00000X,Allopathic,Family Medicine,,Provides family medicine.,20020101,,,, Family Medicine Physician\n\
";

    #[test]
    fn parse_returns_correct_concept_count() {
        let r = parse_nucc_csv(SAMPLE_CSV);
        assert!(r.errors.is_empty(), "unexpected errors: {:?}", r.errors);
        assert_eq!(r.concepts.len(), 3);
    }

    #[test]
    fn parse_creates_synthetic_grouping_and_classification_nodes() {
        let r = parse_nucc_csv(SAMPLE_CSV);
        let codes: Vec<&str> = r.synthetic_nodes.iter().map(|n| n.code.as_str()).collect();
        assert!(codes.contains(&"Behavioral Health"), "{codes:?}");
        assert!(codes.contains(&"Allopathic"), "{codes:?}");
        assert!(codes.contains(&"Counselor"), "{codes:?}");
        assert!(codes.contains(&"Family Medicine"), "{codes:?}");
    }

    #[test]
    fn parse_sets_parent_to_classification() {
        let r = parse_nucc_csv(SAMPLE_CSV);
        let c = r.concepts.iter().find(|c| c.code == "101Y00000X").unwrap();
        assert_eq!(c.parent.as_deref(), Some("Counselor"));
    }

    #[test]
    fn parse_extracts_display_name() {
        let r = parse_nucc_csv(SAMPLE_CSV);
        let c = r.concepts.iter().find(|c| c.code == "101Y00000X").unwrap();
        assert_eq!(c.display.trim(), "Counselor");
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

        fn make_csv_file(content: &str) -> tempfile::NamedTempFile {
            let mut f = tempfile::NamedTempFile::with_suffix(".csv").unwrap();
            f.write_all(content.as_bytes()).unwrap();
            f
        }

        #[tokio::test]
        async fn dry_run_does_not_write() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let ctx = TenantContext::system();
            let f = make_csv_file(SAMPLE_CSV);
            let stats = import_nucc(&backend, &ctx, f.path(), 500, true)
                .await
                .unwrap();
            assert_eq!(stats.code_systems, 1);
            assert_eq!(stats.concepts, 3);
            assert_eq!(count(&backend, "code_systems"), 0);
        }

        #[tokio::test]
        async fn live_import_writes_concepts_and_hierarchy() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let ctx = TenantContext::system();
            let f = make_csv_file(SAMPLE_CSV);
            let stats = import_nucc(&backend, &ctx, f.path(), 500, false)
                .await
                .unwrap();
            assert_eq!(stats.code_systems, 1);
            assert_eq!(stats.concepts, 3);
            // root + 2 groupings + 2 classifications + 3 codes = 8
            assert_eq!(count(&backend, "concepts"), 8);
        }

        #[tokio::test]
        async fn idempotent_reimport() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let ctx = TenantContext::system();
            let f = make_csv_file(SAMPLE_CSV);
            import_nucc(&backend, &ctx, f.path(), 500, false)
                .await
                .unwrap();
            import_nucc(&backend, &ctx, f.path(), 500, false)
                .await
                .unwrap();
            assert_eq!(count(&backend, "code_systems"), 1);
            assert_eq!(count(&backend, "concepts"), 8);
        }
    }
}
