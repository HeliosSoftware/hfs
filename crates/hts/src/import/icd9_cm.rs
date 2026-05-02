//! ICD-9-CM flat-text importer.
//!
//! Parses the CMS ICD-9-CM pipe-delimited text distribution and imports all
//! diagnosis codes with an inferred parent–child hierarchy into the HTS
//! normalized schema.
//!
//! # No license required
//!
//! ICD-9-CM is a US government work in the public domain.

use std::io::{BufRead, BufReader};
use std::path::Path;

use helios_persistence::tenant::TenantContext;

use crate::error::HtsError;
use crate::import::BundleImportBackend;
use crate::import::ImportStats;
use crate::import::bundle_builder::{BuilderConcept, CodeSystemMeta, build_code_system_bundle};

// ── Constants ─────────────────────────────────────────────────────────────────

const ICD9CM_URL: &str = "http://hl7.org/fhir/sid/icd-9-cm";
const ICD9CM_ID: &str = "icd9cm";
const ICD9CM_NAME: &str = "ICD-9-CM";
const ICD9CM_TITLE: &str =
    "ICD-9-CM (International Classification of Diseases, 9th Revision, Clinical Modification)";
const ICD9CM_VERSION: &str = "2015";
const ROOT_CODE: &str = "ICD-9-CM";

// ── Data types ────────────────────────────────────────────────────────────────

#[derive(Debug)]
struct Icd9Concept {
    code: String,
    display: String,
    /// Parent code (also with decimal) or `None` when the parent is the
    /// virtual root `ICD-9-CM`.
    parent: Option<String>,
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Import a CMS ICD-9-CM distribution through the given backend.
pub async fn import_icd9_cm(
    backend: &dyn BundleImportBackend,
    ctx: &TenantContext,
    path: &Path,
    batch_size: usize,
    dry_run: bool,
) -> Result<ImportStats, HtsError> {
    let batch_size = batch_size.max(1);

    let path_owned = path.to_path_buf();
    let (concepts, errors) = tokio::task::spawn_blocking(
        move || -> Result<(Vec<Icd9Concept>, Vec<String>), HtsError> {
            let text = read_text(&path_owned)?;
            Ok(parse_pipe_delimited(&text))
        },
    )
    .await
    .map_err(|e| HtsError::Internal(format!("ICD-9-CM parser panicked: {e}")))??;

    let mut stats = ImportStats {
        errors,
        ..Default::default()
    };

    if dry_run {
        stats.code_systems = 1;
        stats.concepts = concepts.len() as u32;
        eprintln!(
            "[icd9-cm] dry-run — {} concepts parsed, no DB writes",
            concepts.len()
        );
        return Ok(stats);
    }

    let meta = CodeSystemMeta {
        id: ICD9CM_ID,
        url: ICD9CM_URL,
        version: Some(ICD9CM_VERSION),
        name: Some(ICD9CM_NAME),
        title: Some(ICD9CM_TITLE),
        status: "active",
        content: "complete",
    };

    // Seed: CodeSystem metadata + virtual root concept.
    let root = BuilderConcept {
        code: ROOT_CODE,
        display: Some(ICD9CM_TITLE),
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
                parent_code: Some(c.parent.as_deref().unwrap_or(ROOT_CODE)),
                ..Default::default()
            })
            .collect();
        let bytes = build_code_system_bundle(&meta, &builder);
        let chunk = backend.import_bundle(ctx, &bytes).await?;
        stats.errors.extend(chunk.errors);

        eprintln!(
            "[icd9-cm] batch {}/{total_batches} — +{} concepts (total: {})",
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
        let bytes = std::fs::read(path).map_err(|e| {
            HtsError::InvalidRequest(format!("Cannot read '{}': {e}", path.display()))
        })?;
        Ok(decode_cms_text(bytes))
    }
}

/// CMS publishes ICD-9 descriptions in Windows-1252 / Latin-1, not UTF-8.
/// We try UTF-8 first (for any hand-edited or re-encoded copy) and fall
/// back to a byte-wise Latin-1 → UTF-8 decode, which is lossless for every
/// Latin-1 codepoint.
fn decode_cms_text(bytes: Vec<u8>) -> String {
    match String::from_utf8(bytes) {
        Ok(s) => s,
        Err(e) => e.into_bytes().into_iter().map(char::from).collect(),
    }
}

fn read_text_from_zip(path: &Path) -> Result<String, HtsError> {
    let file = std::fs::File::open(path).map_err(|e| {
        HtsError::InvalidRequest(format!("Cannot open ZIP '{}': {e}", path.display()))
    })?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| HtsError::InvalidRequest(format!("Invalid ZIP '{}': {e}", path.display())))?;

    let best_index = (0..archive.len())
        .filter_map(|i| {
            let entry = archive.by_index(i).ok()?;
            let name = entry.name().to_ascii_lowercase();
            if !name.ends_with(".txt") {
                return None;
            }
            if name.contains("readme") || name.contains("license") || name.contains("read_me") {
                return None;
            }
            let score = if name.contains("_desc_long_dx") {
                2u8
            } else if name.contains("_desc_short_dx") {
                1
            } else {
                0
            };
            Some((i, score))
        })
        .max_by_key(|&(_, score)| score)
        .map(|(i, _)| i)
        .ok_or_else(|| {
            HtsError::InvalidRequest(format!(
                "No suitable text file found inside ZIP '{}'. \
                 Expected a pipe-delimited '*_DESC_LONG_DX*.txt' file.",
                path.display()
            ))
        })?;

    let mut entry = archive
        .by_index(best_index)
        .map_err(|e| HtsError::InvalidRequest(format!("Cannot read ZIP entry: {e}")))?;

    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut entry, &mut bytes)
        .map_err(|e| HtsError::InvalidRequest(format!("Cannot read bytes from ZIP: {e}")))?;

    Ok(decode_cms_text(bytes))
}

// ── Parser ────────────────────────────────────────────────────────────────────

fn parse_pipe_delimited(text: &str) -> (Vec<Icd9Concept>, Vec<String>) {
    let mut concepts = Vec::new();
    let mut errors = Vec::new();

    for (line_num, line) in BufReader::new(text.as_bytes()).lines().enumerate() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let Some(pipe_pos) = line.find('|') else {
            errors.push(format!(
                "line {}: no pipe separator — skipped: {line}",
                line_num + 1
            ));
            continue;
        };

        let raw_code = line[..pipe_pos].trim();
        let description = line[pipe_pos + 1..].trim();

        if raw_code.is_empty() {
            errors.push(format!("line {}: empty code — skipped", line_num + 1));
            continue;
        }

        let code = insert_dot(raw_code);
        let parent = parent_of(&code);

        concepts.push(Icd9Concept {
            code,
            display: description.to_string(),
            parent,
        });
    }

    (concepts, errors)
}

fn insert_dot(raw: &str) -> String {
    let base = if raw.starts_with('E') || raw.starts_with('e') {
        4
    } else {
        3
    };
    if raw.len() <= base {
        raw.to_string()
    } else {
        format!("{}.{}", &raw[..base], &raw[base..])
    }
}

fn parent_of(code: &str) -> Option<String> {
    match code.find('.') {
        None => None,
        Some(dot) => {
            let after_dot = &code[dot + 1..];
            if after_dot.len() <= 1 {
                Some(code[..dot].to_string())
            } else {
                Some(code[..code.len() - 1].to_string())
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dot_not_inserted_for_3_char_numeric() {
        assert_eq!(insert_dot("001"), "001");
    }

    #[test]
    fn dot_inserted_for_4_char_numeric() {
        assert_eq!(insert_dot("0010"), "001.0");
    }

    #[test]
    fn dot_inserted_for_5_char_numeric() {
        assert_eq!(insert_dot("00100"), "001.00");
    }

    #[test]
    fn dot_not_inserted_for_e_code_base() {
        assert_eq!(insert_dot("E800"), "E800");
    }

    #[test]
    fn dot_inserted_for_e_code_sub() {
        assert_eq!(insert_dot("E8000"), "E800.0");
    }

    #[test]
    fn dot_not_inserted_for_v_code_base() {
        assert_eq!(insert_dot("V01"), "V01");
    }

    #[test]
    fn dot_inserted_for_v_code_sub() {
        assert_eq!(insert_dot("V010"), "V01.0");
    }

    #[test]
    fn parent_of_top_level_is_none() {
        assert_eq!(parent_of("001"), None);
        assert_eq!(parent_of("E800"), None);
        assert_eq!(parent_of("V01"), None);
    }

    #[test]
    fn parent_of_one_decimal_digit() {
        assert_eq!(parent_of("001.0"), Some("001".to_string()));
        assert_eq!(parent_of("E800.0"), Some("E800".to_string()));
        assert_eq!(parent_of("V01.0"), Some("V01".to_string()));
    }

    #[test]
    fn parent_of_two_decimal_digits() {
        assert_eq!(parent_of("001.00"), Some("001.0".to_string()));
        assert_eq!(parent_of("E800.01"), Some("E800.0".to_string()));
    }

    const SAMPLE: &str = "\
001|Cholera\n\
0010|Cholera due to vibrio cholerae\n\
00100|Cholera due to vibrio cholerae\n\
00101|Cholera due to vibrio cholerae el tor\n\
E800|Railway accidents\n\
E8000|Railway accident injuring occupant of railway vehicle\n\
V01|Contact with or exposure to communicable diseases\n\
";

    #[test]
    fn parse_returns_correct_count() {
        let (concepts, errors) = parse_pipe_delimited(SAMPLE);
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
        assert_eq!(concepts.len(), 7);
    }

    #[test]
    fn parse_inserts_dot_in_codes() {
        let (concepts, _) = parse_pipe_delimited(SAMPLE);
        let codes: Vec<&str> = concepts.iter().map(|c| c.code.as_str()).collect();
        assert!(codes.contains(&"001.0"));
        assert!(codes.contains(&"001.00"));
        assert!(codes.contains(&"E800.0"));
    }

    #[test]
    fn parse_sets_correct_parents() {
        let (concepts, _) = parse_pipe_delimited(SAMPLE);
        let find = |code: &str| concepts.iter().find(|c| c.code == code).unwrap();

        assert_eq!(find("001").parent, None);
        assert_eq!(find("001.0").parent, Some("001".to_string()));
        assert_eq!(find("001.00").parent, Some("001.0".to_string()));
        assert_eq!(find("E800").parent, None);
        assert_eq!(find("E800.0").parent, Some("E800".to_string()));
    }

    #[test]
    fn parse_skips_lines_without_pipe() {
        let text = "001|Cholera\nBADLINE\n0010|Sub-cholera\n";
        let (concepts, errors) = parse_pipe_delimited(text);
        assert_eq!(concepts.len(), 2);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("no pipe separator"));
    }

    #[test]
    fn parse_skips_empty_code() {
        let text = "|No code here\n001|Cholera\n";
        let (concepts, errors) = parse_pipe_delimited(text);
        assert_eq!(concepts.len(), 1);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("empty code"));
    }

    #[test]
    fn parse_ignores_blank_lines() {
        let text = "\n001|Cholera\n\n0010|Sub\n";
        let (concepts, errors) = parse_pipe_delimited(text);
        assert!(errors.is_empty());
        assert_eq!(concepts.len(), 2);
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

        fn make_zip_file(content: &str) -> tempfile::NamedTempFile {
            let tmp = tempfile::NamedTempFile::with_suffix(".zip").unwrap();
            {
                let mut zip = zip::ZipWriter::new(tmp.reopen().unwrap());
                zip.start_file("CMS32_DESC_LONG_DX.txt", zip::write::FileOptions::default())
                    .unwrap();
                zip.write_all(content.as_bytes()).unwrap();
                zip.finish().unwrap();
            }
            tmp
        }

        #[tokio::test]
        async fn dry_run_does_not_write() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let ctx = TenantContext::system();
            let f = make_txt_file(SAMPLE);
            let stats = import_icd9_cm(&backend, &ctx, f.path(), 500, true)
                .await
                .unwrap();
            assert_eq!(stats.code_systems, 1);
            assert_eq!(stats.concepts, 7);
            assert_eq!(count(&backend, "code_systems"), 0);
            assert_eq!(count(&backend, "concepts"), 0);
        }

        #[tokio::test]
        async fn live_import_writes_concepts_and_hierarchy() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let ctx = TenantContext::system();
            let f = make_txt_file(SAMPLE);
            let stats = import_icd9_cm(&backend, &ctx, f.path(), 500, false)
                .await
                .unwrap();
            assert_eq!(stats.code_systems, 1);
            assert_eq!(stats.concepts, 7);
            assert_eq!(count(&backend, "concepts"), 8);
            assert_eq!(count(&backend, "concept_hierarchy"), 7);
        }

        #[tokio::test]
        async fn idempotent_reimport() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let ctx = TenantContext::system();
            let f = make_txt_file(SAMPLE);
            import_icd9_cm(&backend, &ctx, f.path(), 500, false)
                .await
                .unwrap();
            import_icd9_cm(&backend, &ctx, f.path(), 500, false)
                .await
                .unwrap();
            assert_eq!(count(&backend, "code_systems"), 1);
            assert_eq!(count(&backend, "concepts"), 8);
            assert_eq!(count(&backend, "concept_hierarchy"), 7);
        }

        #[tokio::test]
        async fn import_from_zip() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let ctx = TenantContext::system();
            let f = make_zip_file(SAMPLE);
            let stats = import_icd9_cm(&backend, &ctx, f.path(), 500, false)
                .await
                .unwrap();
            assert_eq!(stats.concepts, 7);
            assert_eq!(count(&backend, "concepts"), 8);
        }

        #[tokio::test]
        async fn batching_preserves_all_concepts() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let ctx = TenantContext::system();
            let f = make_txt_file(SAMPLE);
            let stats = import_icd9_cm(&backend, &ctx, f.path(), 2, false)
                .await
                .unwrap();
            assert_eq!(stats.concepts, 7);
            assert_eq!(count(&backend, "concepts"), 8);
        }

        #[tokio::test]
        async fn missing_file_returns_error() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let ctx = TenantContext::system();
            let result = import_icd9_cm(
                &backend,
                &ctx,
                Path::new("/nonexistent/icd9.txt"),
                500,
                false,
            )
            .await;
            assert!(result.is_err());
        }
    }
}
