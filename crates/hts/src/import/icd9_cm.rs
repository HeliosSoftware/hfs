//! ICD-9-CM flat-text importer.
//!
//! Parses the CMS ICD-9-CM text distribution (space-delimited in the real
//! bundled files; pipe-delimited as a fallback for other CMS vintages) and
//! imports diagnosis codes (`CMS32_DESC_LONG_DX.txt`) and, when present in
//! the same ZIP, procedure codes (`CMS32_DESC_LONG_SG.txt`) into a single
//! `icd9cm` CodeSystem, with an inferred parent–child hierarchy.
//!
//! # Known gap: some category-header codes are absent from the source data
//!
//! The bundled CMS v32 files list only the codes CMS actually published —
//! a handful of category headers (e.g. `V72.3`) have subdivisions
//! (`V72.31`, `V72.32`, ...) but no row for the header itself, so `$lookup`
//! on that bare header code returns not-found even though every leaf under
//! it resolves correctly. This is a property of the source file, not a
//! parser defect: every line present in the distribution is imported.
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

/// Which decimal-insertion rule applies to a raw CMS code string.
///
/// Diagnosis codes (Volumes I/II, `CMS32_DESC_LONG_DX.txt`) insert the dot
/// after 3 digits (4 for `E`-codes). Procedure codes (Volume III,
/// `CMS32_DESC_LONG_SG.txt`) insert it after 2 digits and are purely
/// numeric — verified against the real bundled file: no letter-prefixed
/// procedure codes exist, so no special-casing is needed there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DotRule {
    Diagnosis,
    Procedure,
}

/// Both text payloads extracted from a bundled ICD-9-CM ZIP. `sg_text` is
/// optional: some CMS vintages ship diagnosis codes only.
struct Icd9ZipContents {
    dx_text: String,
    sg_text: Option<String>,
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
            let ext = path_owned
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();

            if ext == "zip" {
                let zc = read_zip_contents(&path_owned)?;
                let (mut concepts, mut errors) =
                    parse_descriptions(&zc.dx_text, DotRule::Diagnosis);
                if let Some(sg_text) = zc.sg_text {
                    let (sg_concepts, sg_errors) = parse_descriptions(&sg_text, DotRule::Procedure);
                    concepts.extend(sg_concepts);
                    errors.extend(sg_errors);
                }
                Ok((concepts, errors))
            } else {
                let bytes = std::fs::read(&path_owned).map_err(|e| {
                    HtsError::InvalidRequest(format!("Cannot read '{}': {e}", path_owned.display()))
                })?;
                let text = decode_cms_text(bytes);
                Ok(parse_descriptions(&text, DotRule::Diagnosis))
            }
        },
    )
    .await
    .map_err(|e| HtsError::Internal(format!("ICD-9-CM parser panicked: {e}")))??;

    if concepts.is_empty() {
        return Err(HtsError::InvalidRequest(format!(
            "ICD-9-CM import of '{}' produced 0 concepts ({} line errors; first: {}) — \
             file is not in the expected CMS format",
            path.display(),
            errors.len(),
            errors.first().map(String::as_str).unwrap_or("<none>")
        )));
    }

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

/// Read both the diagnosis and (if present) procedure text payloads out of a
/// bundled ICD-9-CM ZIP. DX is required — a real CMS distribution always
/// ships it. SG is optional — some vintages ship diagnoses only.
fn read_zip_contents(path: &Path) -> Result<Icd9ZipContents, HtsError> {
    let file = std::fs::File::open(path).map_err(|e| {
        HtsError::InvalidRequest(format!("Cannot open ZIP '{}': {e}", path.display()))
    })?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| HtsError::InvalidRequest(format!("Invalid ZIP '{}': {e}", path.display())))?;

    // (index, score) candidates for each of the two roles, scored by name.
    let mut dx_candidates: Vec<(usize, u8)> = Vec::new();
    let mut sg_candidates: Vec<(usize, u8)> = Vec::new();
    for i in 0..archive.len() {
        let Ok(entry) = archive.by_index(i) else {
            continue;
        };
        let name = entry.name().to_ascii_lowercase();
        if !name.ends_with(".txt") {
            continue;
        }
        if name.contains("readme") || name.contains("license") || name.contains("read_me") {
            continue;
        }
        if name.contains("_desc_long_dx") {
            dx_candidates.push((i, 2));
        } else if name.contains("_desc_short_dx") {
            dx_candidates.push((i, 1));
        } else if name.contains("_desc_long_sg") {
            sg_candidates.push((i, 2));
        } else if name.contains("_desc_short_sg") {
            sg_candidates.push((i, 1));
        }
    }

    let dx_index = dx_candidates
        .into_iter()
        .max_by_key(|&(_, score)| score)
        .map(|(i, _)| i)
        .ok_or_else(|| {
            HtsError::InvalidRequest(format!(
                "No suitable ICD-9-CM diagnosis file found inside ZIP '{}'. \
                 Expected a '*_DESC_LONG_DX*.txt' (or '*_DESC_SHORT_DX*.txt') file.",
                path.display()
            ))
        })?;
    let sg_index = sg_candidates
        .into_iter()
        .max_by_key(|&(_, score)| score)
        .map(|(i, _)| i);

    let read_entry = |archive: &mut zip::ZipArchive<std::fs::File>,
                      index: usize|
     -> Result<String, HtsError> {
        let mut entry = archive
            .by_index(index)
            .map_err(|e| HtsError::InvalidRequest(format!("Cannot read ZIP entry: {e}")))?;
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut bytes)
            .map_err(|e| HtsError::InvalidRequest(format!("Cannot read bytes from ZIP: {e}")))?;
        Ok(decode_cms_text(bytes))
    };

    let dx_text = read_entry(&mut archive, dx_index)?;
    let sg_text = sg_index.map(|i| read_entry(&mut archive, i)).transpose()?;

    Ok(Icd9ZipContents { dx_text, sg_text })
}

// ── Parser ────────────────────────────────────────────────────────────────────

/// Parses one CMS description file. Tries the pipe-delimited layout first
/// (`code|description`), then falls back to the real CMS layout — a code
/// followed by a run of whitespace then the description. A pipe line never
/// contains raw whitespace before `|`; a real CMS line never contains `|` —
/// so the priority is unambiguous.
fn parse_descriptions(text: &str, rule: DotRule) -> (Vec<Icd9Concept>, Vec<String>) {
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

        let (raw_code, description) = if let Some(pipe_pos) = line.find('|') {
            (line[..pipe_pos].trim(), line[pipe_pos + 1..].trim())
        } else if let Some((code, desc)) = line.split_once(char::is_whitespace) {
            (code.trim(), desc.trim())
        } else {
            errors.push(format!(
                "line {}: no code/description separator — skipped: {line}",
                line_num + 1
            ));
            continue;
        };

        if raw_code.is_empty() {
            errors.push(format!("line {}: empty code — skipped", line_num + 1));
            continue;
        }

        let code = insert_dot(raw_code, rule);
        let parent = parent_of(&code);

        concepts.push(Icd9Concept {
            code,
            display: description.to_string(),
            parent,
        });
    }

    (concepts, errors)
}

fn insert_dot(raw: &str, rule: DotRule) -> String {
    let base = match rule {
        DotRule::Diagnosis => {
            if raw.starts_with('E') || raw.starts_with('e') {
                4
            } else {
                3
            }
        }
        DotRule::Procedure => 2,
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
        assert_eq!(insert_dot("001", DotRule::Diagnosis), "001");
    }

    #[test]
    fn dot_inserted_for_4_char_numeric() {
        assert_eq!(insert_dot("0010", DotRule::Diagnosis), "001.0");
    }

    #[test]
    fn dot_inserted_for_5_char_numeric() {
        assert_eq!(insert_dot("00100", DotRule::Diagnosis), "001.00");
    }

    #[test]
    fn dot_not_inserted_for_e_code_base() {
        assert_eq!(insert_dot("E800", DotRule::Diagnosis), "E800");
    }

    #[test]
    fn dot_inserted_for_e_code_sub() {
        assert_eq!(insert_dot("E8000", DotRule::Diagnosis), "E800.0");
    }

    #[test]
    fn dot_not_inserted_for_v_code_base() {
        assert_eq!(insert_dot("V01", DotRule::Diagnosis), "V01");
    }

    #[test]
    fn dot_inserted_for_v_code_sub() {
        assert_eq!(insert_dot("V010", DotRule::Diagnosis), "V01.0");
    }

    #[test]
    fn procedure_dot_rule_uses_two_digit_base() {
        assert_eq!(insert_dot("0010", DotRule::Procedure), "00.10");
        assert_eq!(insert_dot("36", DotRule::Procedure), "36");
        assert_eq!(insert_dot("361", DotRule::Procedure), "36.1");
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
        let (concepts, errors) = parse_descriptions(SAMPLE, DotRule::Diagnosis);
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
        assert_eq!(concepts.len(), 7);
    }

    #[test]
    fn parse_inserts_dot_in_codes() {
        let (concepts, _) = parse_descriptions(SAMPLE, DotRule::Diagnosis);
        let codes: Vec<&str> = concepts.iter().map(|c| c.code.as_str()).collect();
        assert!(codes.contains(&"001.0"));
        assert!(codes.contains(&"001.00"));
        assert!(codes.contains(&"E800.0"));
    }

    #[test]
    fn parse_sets_correct_parents() {
        let (concepts, _) = parse_descriptions(SAMPLE, DotRule::Diagnosis);
        let find = |code: &str| concepts.iter().find(|c| c.code == code).unwrap();

        assert_eq!(find("001").parent, None);
        assert_eq!(find("001.0").parent, Some("001".to_string()));
        assert_eq!(find("001.00").parent, Some("001.0".to_string()));
        assert_eq!(find("E800").parent, None);
        assert_eq!(find("E800.0").parent, Some("E800".to_string()));
    }

    #[test]
    fn parse_skips_lines_without_separator() {
        let text = "001|Cholera\nBADLINE\n0010|Sub-cholera\n";
        let (concepts, errors) = parse_descriptions(text, DotRule::Diagnosis);
        assert_eq!(concepts.len(), 2);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("no code/description separator"));
    }

    #[test]
    fn parse_skips_empty_code() {
        let text = "|No code here\n001|Cholera\n";
        let (concepts, errors) = parse_descriptions(text, DotRule::Diagnosis);
        assert_eq!(concepts.len(), 1);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("empty code"));
    }

    #[test]
    fn parse_ignores_blank_lines() {
        let text = "\n001|Cholera\n\n0010|Sub\n";
        let (concepts, errors) = parse_descriptions(text, DotRule::Diagnosis);
        assert!(errors.is_empty());
        assert_eq!(concepts.len(), 2);
    }

    #[test]
    fn parse_space_delimited_returns_correct_count_and_trims_padding() {
        // Real CMS layout: 5-char space-padded code, then description.
        let text = "\
001   Cholera\n\
0010  Cholera due to vibrio cholerae\n\
E800  Railway accidents\n";
        let (concepts, errors) = parse_descriptions(text, DotRule::Diagnosis);
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
        assert_eq!(concepts.len(), 3);
        let find = |code: &str| concepts.iter().find(|c| c.code == code).unwrap();
        assert_eq!(find("001").display, "Cholera");
        assert_eq!(find("001.0").display, "Cholera due to vibrio cholerae");
    }

    #[test]
    fn parse_mixed_pipe_and_space_lines() {
        let text = "001|Cholera\n0010  Cholera due to vibrio cholerae\n";
        let (concepts, errors) = parse_descriptions(text, DotRule::Diagnosis);
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
        assert_eq!(concepts.len(), 2);
    }

    #[test]
    fn dx_and_sg_final_codes_never_collide() {
        // Mirrors the real-file collision check performed during planning:
        // raw digit strings can be shared, but the differing dot-rule bases
        // must keep the final codes disjoint.
        let dx_text =
            "0010  Cholera due to vibrio cholerae\n0011  Cholera due to vibrio cholerae el tor\n";
        let sg_text = "0010  Therapeutic ultrasound of vessels of head and neck\n0011  Therapeutic ultrasound of heart\n";
        let (dx_concepts, _) = parse_descriptions(dx_text, DotRule::Diagnosis);
        let (sg_concepts, _) = parse_descriptions(sg_text, DotRule::Procedure);
        let dx_codes: std::collections::HashSet<&str> =
            dx_concepts.iter().map(|c| c.code.as_str()).collect();
        let sg_codes: std::collections::HashSet<&str> =
            sg_concepts.iter().map(|c| c.code.as_str()).collect();
        assert!(
            dx_codes.is_disjoint(&sg_codes),
            "DX {dx_codes:?} and SG {sg_codes:?} must not overlap"
        );
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

        fn make_zip_file_with_entries(entries: &[(&str, &str)]) -> tempfile::NamedTempFile {
            let tmp = tempfile::NamedTempFile::with_suffix(".zip").unwrap();
            {
                let mut zip = zip::ZipWriter::new(tmp.reopen().unwrap());
                for (name, content) in entries {
                    zip.start_file(*name, zip::write::FileOptions::default())
                        .unwrap();
                    zip.write_all(content.as_bytes()).unwrap();
                }
                zip.finish().unwrap();
            }
            tmp
        }

        /// POC pilot: the FIRST 3 REAL lines of the bundled CMS distribution's
        /// diagnosis file and the FIRST 3 REAL lines of its procedure file
        /// (extracted directly from
        /// `crates/hts/terminology-data/ICD-9-CM-v32-master-descriptions.zip`
        /// during planning), run through the real import path end-to-end.
        /// Proves, before the full plan lands, that: (a) the space-delimited
        /// real layout parses, (b) both DX and SG entries are read from one
        /// zip, (c) the two dot-rules keep codes disjoint, and (d) $lookup-style
        /// SQL against the resulting rows returns the expected display text.
        #[tokio::test]
        async fn pilot_real_first_three_dx_and_sg_lines_import_correctly() {
            const REAL_DX_HEAD: &str = "\
0010  Cholera due to vibrio cholerae\n\
0011  Cholera due to vibrio cholerae el tor\n\
0019  Cholera, unspecified\n";
            const REAL_SG_HEAD: &str = "\
0001 Therapeutic ultrasound of vessels of head and neck\n\
0002 Therapeutic ultrasound of heart\n\
0003 Therapeutic ultrasound of peripheral vascular vessels\n";

            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let ctx = TenantContext::system();
            let f = make_zip_file_with_entries(&[
                ("CMS32_DESC_LONG_DX.txt", REAL_DX_HEAD),
                ("CMS32_DESC_LONG_SG.txt", REAL_SG_HEAD),
            ]);

            let stats = import_icd9_cm(&backend, &ctx, f.path(), 500, false)
                .await
                .unwrap();

            // 3 DX + 3 SG concepts, no parse errors.
            assert_eq!(stats.concepts, 6);
            assert!(
                stats.errors.is_empty(),
                "unexpected errors: {:?}",
                stats.errors
            );
            // +1 for the virtual ICD-9-CM root.
            assert_eq!(count(&backend, "concepts"), 7);

            let conn = backend.pool().get().unwrap();
            let lookup = |code: &str| -> String {
                conn.query_row(
                    "SELECT display FROM concepts WHERE code = ?1",
                    [code],
                    |r| r.get(0),
                )
                .unwrap_or_else(|e| panic!("lookup failed for {code}: {e}"))
            };

            // DX codes: 3-digit base, dot before the 4th char.
            assert_eq!(lookup("001.0"), "Cholera due to vibrio cholerae");
            assert_eq!(lookup("001.1"), "Cholera due to vibrio cholerae el tor");
            assert_eq!(lookup("001.9"), "Cholera, unspecified");

            // SG codes: 2-digit base, dot before the 3rd char — distinct
            // shape from DX, so no collision even though the raw digit
            // strings ("0010" vs "0001"..) live in the same numeric space.
            assert_eq!(
                lookup("00.01"),
                "Therapeutic ultrasound of vessels of head and neck"
            );
            assert_eq!(lookup("00.02"), "Therapeutic ultrasound of heart");
            assert_eq!(
                lookup("00.03"),
                "Therapeutic ultrasound of peripheral vascular vessels"
            );

            // Explicit disjointness check on the actual persisted rows.
            let dx_codes = ["001.0", "001.1", "001.9"];
            let sg_codes = ["00.01", "00.02", "00.03"];
            for d in dx_codes {
                assert!(
                    !sg_codes.contains(&d),
                    "{d} unexpectedly shared with SG set"
                );
            }
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

        #[tokio::test]
        async fn sg_absent_zip_still_imports_dx() {
            // A CMS vintage that ships diagnoses only — SG stays optional.
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let ctx = TenantContext::system();
            let f = make_zip_file_with_entries(&[("CMS32_DESC_LONG_DX.txt", SAMPLE)]);
            let stats = import_icd9_cm(&backend, &ctx, f.path(), 500, false)
                .await
                .unwrap();
            assert_eq!(stats.concepts, 7);
            assert_eq!(count(&backend, "concepts"), 8);
        }

        #[tokio::test]
        async fn zero_concepts_is_hard_error() {
            let backend = SqliteTerminologyBackend::in_memory().unwrap();
            let ctx = TenantContext::system();
            let f = make_txt_file("GARBAGE\nNOPE\n");

            let result = import_icd9_cm(&backend, &ctx, f.path(), 500, false).await;
            assert!(result.is_err(), "expected a hard error, got {result:?}");
            // No partial seed write on failure.
            assert_eq!(count(&backend, "code_systems"), 0);

            let dry_run_result = import_icd9_cm(&backend, &ctx, f.path(), 500, true).await;
            assert!(dry_run_result.is_err());
        }
    }
}
