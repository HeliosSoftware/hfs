//! DICOM (Digital Imaging and Communications in Medicine) code importer.
//!
//! Parses the NEMA DICOM Part 16 code table (exported as CSV or tab-delimited)
//! and imports all code meanings into the HTS normalized schema.
//!
//! # No license required
//!
//! NEMA makes the DICOM standard freely available. The code meanings used in FHIR
//! imaging resources (`ImagingStudy`, `ImagingSelection`) are published in
//! DICOM PS3.16 (Content Mapping Resource). Download from:
//! <https://www.dicomstandard.org/current/>
//!
//! # Hierarchy
//!
//! DICOM codes form a flat list. All codes are placed as children of a virtual
//! root `DCM` concept in the `concept_hierarchy` table.

use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use helios_persistence::tenant::TenantContext;

use crate::error::HtsError;
use crate::import::BundleImportBackend;
use crate::import::ImportStats;
use crate::import::bundle_builder::{BuilderConcept, CodeSystemMeta, build_code_system_bundle};

// ── Constants ─────────────────────────────────────────────────────────────────

const DICOM_URL: &str = "http://dicom.nema.org/resources/ontology/DCM";
const DICOM_ID: &str = "dicom";
const DICOM_NAME: &str = "DCM";
const DICOM_TITLE: &str = "DICOM Controlled Terminology";
const ROOT_CODE: &str = "DCM";

// ── Data types ────────────────────────────────────────────────────────────────

#[derive(Debug)]
struct DicomConcept {
    code: String,
    display: String,
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Import a DICOM Part 16 code table (CSV or TSV) through the given backend.
pub async fn import_dicom(
    backend: &dyn BundleImportBackend,
    ctx: &TenantContext,
    path: &Path,
    batch_size: usize,
    dry_run: bool,
) -> Result<ImportStats, HtsError> {
    let batch_size = batch_size.max(1);

    let path_owned = path.to_path_buf();
    let (concepts, errors) = tokio::task::spawn_blocking(
        move || -> Result<(Vec<DicomConcept>, Vec<String>), HtsError> {
            let text = read_text(&path_owned)?;
            Ok(parse_dicom_table(&text))
        },
    )
    .await
    .map_err(|e| HtsError::Internal(format!("DICOM parser panicked: {e}")))??;

    let mut stats = ImportStats {
        errors,
        ..Default::default()
    };

    if dry_run {
        stats.code_systems = 1;
        stats.concepts = concepts.len() as u32;
        eprintln!(
            "[dicom] dry-run — {} codes parsed, no DB writes",
            concepts.len()
        );
        return Ok(stats);
    }

    let meta = CodeSystemMeta {
        id: DICOM_ID,
        url: DICOM_URL,
        version: Some("current"),
        name: Some(DICOM_NAME),
        title: Some(DICOM_TITLE),
        status: "active",
        content: "complete",
    };

    // Seed: virtual root only.
    let root = BuilderConcept {
        code: ROOT_CODE,
        display: Some(DICOM_TITLE),
        definition: Some("header"),
        ..Default::default()
    };
    let seed = build_code_system_bundle(&meta, std::slice::from_ref(&root));
    let seed_stats = backend.import_bundle(ctx, &seed).await?;
    stats.code_systems = seed_stats.code_systems;
    stats.errors.extend(seed_stats.errors);

    let total = concepts.len();
    let total_batches = total.div_ceil(batch_size).max(1);

    for (batch_idx, batch) in concepts.chunks(batch_size).enumerate() {
        let builder: Vec<BuilderConcept<'_>> = batch
            .iter()
            .map(|c| BuilderConcept {
                code: &c.code,
                display: Some(&c.display),
                parent_code: Some(ROOT_CODE),
                ..Default::default()
            })
            .collect();
        let bytes = build_code_system_bundle(&meta, &builder);
        let chunk = backend.import_bundle(ctx, &bytes).await?;
        stats.errors.extend(chunk.errors);

        eprintln!(
            "[dicom] batch {}/{total_batches} — +{} codes",
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
            let is_data_file =
                name.ends_with(".csv") || name.ends_with(".tsv") || name.ends_with(".txt");
            if is_data_file && (name.contains("dicom") || name.contains("dcm")) {
                Some(i)
            } else {
                None
            }
        })
        .ok_or_else(|| {
            HtsError::InvalidRequest(format!(
                "No DICOM code table found inside ZIP '{}'. \
                 Expected a CSV/TSV file with 'dicom' or 'dcm' in the name.",
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

fn detect_delimiter(text: &str) -> char {
    for line in text.lines() {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            return if trimmed.contains('\t') { '\t' } else { ',' };
        }
    }
    ','
}

fn parse_dicom_table(text: &str) -> (Vec<DicomConcept>, Vec<String>) {
    let delimiter = detect_delimiter(text);
    let mut concepts = Vec::new();
    let mut errors = Vec::new();
    let mut lines = BufReader::new(text.as_bytes()).lines().enumerate();

    if let Some((_, Ok(first))) = lines.next() {
        let trimmed = first.trim();
        let first_col = trimmed.split(delimiter).next().unwrap_or("").trim();
        if looks_like_code(first_col) {
            process_line(0, trimmed, delimiter, &mut concepts, &mut errors);
        }
    }

    for (line_num, line) in lines {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        process_line(line_num + 1, trimmed, delimiter, &mut concepts, &mut errors);
    }

    (concepts, errors)
}

fn looks_like_code(s: &str) -> bool {
    !s.is_empty() && s.len() <= 16 && !s.chars().any(|c| c.is_ascii_lowercase())
}

fn process_line(
    line_num: usize,
    line: &str,
    delimiter: char,
    concepts: &mut Vec<DicomConcept>,
    errors: &mut Vec<String>,
) {
    let cols: Vec<&str> = line.splitn(3, delimiter).collect();

    let (code_col, meaning_col) = if cols.len() >= 3 {
        (cols[0].trim(), cols[2].trim())
    } else if cols.len() == 2 {
        (cols[0].trim(), cols[1].trim())
    } else {
        errors.push(format!("line {line_num}: too few columns — skipped"));
        return;
    };

    if code_col.is_empty() {
        errors.push(format!("line {line_num}: empty code — skipped"));
        return;
    }
    if meaning_col.is_empty() {
        errors.push(format!(
            "line {line_num}: empty meaning for code '{code_col}' — skipped"
        ));
        return;
    }

    concepts.push(DicomConcept {
        code: code_col.to_string(),
        display: meaning_col.to_string(),
    });
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_CSV: &str = "\
CodeValue,CodingSchemeDesignator,CodeMeaning\n\
001,DCM,Quantitative Immunofluorescence\n\
002,DCM,Qualitative Immunofluorescence\n\
003,DCM,Threshold\n\
";

    const SAMPLE_TSV: &str = "\
CodeValue\tCodingSchemeDesignator\tCodeMeaning\n\
001\tDCM\tQuantitative Immunofluorescence\n\
002\tDCM\tQualitative Immunofluorescence\n\
";

    const SAMPLE_2COL: &str = "\
001,Quantitative Immunofluorescence\n\
002,Qualitative Immunofluorescence\n\
";

    #[test]
    fn parse_csv_3col_skips_header() {
        let (concepts, errors) = parse_dicom_table(SAMPLE_CSV);
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
        assert_eq!(concepts.len(), 3);
    }

    #[test]
    fn parse_tsv_3col() {
        let (concepts, errors) = parse_dicom_table(SAMPLE_TSV);
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
        assert_eq!(concepts.len(), 2);
    }

    #[test]
    fn parse_2col_no_header() {
        let (concepts, errors) = parse_dicom_table(SAMPLE_2COL);
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
        assert_eq!(concepts.len(), 2);
        assert_eq!(concepts[0].code, "001");
        assert_eq!(concepts[0].display, "Quantitative Immunofluorescence");
    }

    #[test]
    fn parse_extracts_code_meaning() {
        let (concepts, _) = parse_dicom_table(SAMPLE_CSV);
        assert_eq!(concepts[0].code, "001");
        assert_eq!(concepts[0].display, "Quantitative Immunofluorescence");
    }

    #[test]
    fn detect_delimiter_tab() {
        assert_eq!(detect_delimiter("a\tb\tc\n"), '\t');
    }

    #[test]
    fn detect_delimiter_comma() {
        assert_eq!(detect_delimiter("a,b,c\n"), ',');
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
            let stats = import_dicom(&backend, &ctx, f.path(), 500, true)
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
            let stats = import_dicom(&backend, &ctx, f.path(), 500, false)
                .await
                .unwrap();
            assert_eq!(stats.code_systems, 1);
            assert_eq!(stats.concepts, 3);
            // virtual root + 3 codes
            assert_eq!(count(&backend, "concepts"), 4);
            // 3 flat hierarchy edges
            assert_eq!(count(&backend, "concept_hierarchy"), 3);
        }

        #[tokio::test]
        async fn idempotent_reimport() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let ctx = TenantContext::system();
            let f = make_csv_file(SAMPLE_CSV);
            import_dicom(&backend, &ctx, f.path(), 500, false)
                .await
                .unwrap();
            import_dicom(&backend, &ctx, f.path(), 500, false)
                .await
                .unwrap();
            assert_eq!(count(&backend, "code_systems"), 1);
            assert_eq!(count(&backend, "concepts"), 4);
            assert_eq!(count(&backend, "concept_hierarchy"), 3);
        }
    }
}
