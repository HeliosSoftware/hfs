//! RxNorm RRF importer.
//!
//! Parses the NLM RxNorm full release distribution and imports drug concepts,
//! preferred display names, and `isa` hierarchy edges into the HTS normalized
//! schema.
//!
//! # ⚠️  LICENSE REQUIRED
//!
//! Real RxNorm data requires acceptance of the NLM Terms of Service.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::Path;

use helios_persistence::tenant::TenantContext;

use crate::error::HtsError;
use crate::import::BundleImportBackend;
use crate::import::ImportStats;
use crate::import::bundle_builder::{
    BuilderConcept, BuilderProperty, CodeSystemMeta, build_code_system_bundle,
};

// ── RxNorm constants ──────────────────────────────────────────────────────────

const RXNORM_URL: &str = "http://www.nlm.nih.gov/research/umls/rxnorm";
const RXNORM_ID: &str = "rxnorm";
const RXNORM_NAME: &str = "RxNorm";
const RXNORM_TITLE: &str = "RxNorm — NLM Drug Terminology";

// ── Public entry point ────────────────────────────────────────────────────────

/// Import an RxNorm RRF distribution through the given backend.
pub async fn import_rxnorm_rrf(
    backend: &dyn BundleImportBackend,
    ctx: &TenantContext,
    path: &Path,
    batch_size: usize,
    dry_run: bool,
) -> Result<ImportStats, HtsError> {
    let batch_size = batch_size.max(1);

    type RxnormParsed = (HashMap<String, String>, Vec<(String, String)>, Vec<String>);

    let path_owned = path.to_path_buf();
    let (concepts, edges, parse_errors) =
        tokio::task::spawn_blocking(move || -> Result<RxnormParsed, HtsError> {
            let (conso_bytes, rel_bytes) = read_rrf_files(&path_owned)?;
            let mut parse_errors: Vec<String> = Vec::new();
            let concepts =
                parse_concepts(BufReader::new(conso_bytes.as_slice()), &mut parse_errors)?;
            let edges = parse_relationships(
                BufReader::new(rel_bytes.as_slice()),
                &concepts,
                &mut parse_errors,
            )?;
            Ok((concepts, edges, parse_errors))
        })
        .await
        .map_err(|e| HtsError::Internal(format!("RxNorm parser panicked: {e}")))??;

    let mut stats = ImportStats {
        errors: parse_errors,
        ..Default::default()
    };

    if dry_run {
        stats.code_systems = 1;
        stats.concepts = concepts.len() as u32;
        eprintln!(
            "[rxnorm] dry-run — {} concepts, {} isa edges parsed, no DB writes",
            concepts.len(),
            edges.len()
        );
        return Ok(stats);
    }

    // Build child → parents map (a concept can have multiple parents).
    let mut parents_of: HashMap<String, Vec<String>> = HashMap::new();
    for (child, parent) in &edges {
        parents_of
            .entry(child.clone())
            .or_default()
            .push(parent.clone());
    }

    let meta = CodeSystemMeta {
        id: RXNORM_ID,
        url: RXNORM_URL,
        version: Some("current"),
        name: Some(RXNORM_NAME),
        title: Some(RXNORM_TITLE),
        status: "active",
        content: "complete",
    };

    // Seed empty CodeSystem.
    let seed = build_code_system_bundle(&meta, &[]);
    let seed_stats = backend.import_bundle(ctx, &seed).await?;
    stats.code_systems = seed_stats.code_systems;
    stats.errors.extend(seed_stats.errors);

    let concept_list: Vec<(String, String)> = concepts.into_iter().collect();
    let total = concept_list.len();
    let total_batches = total.div_ceil(batch_size).max(1);

    for (batch_idx, batch) in concept_list.chunks(batch_size).enumerate() {
        let extras_per: Vec<Vec<BuilderProperty<'_>>> = batch
            .iter()
            .map(|(code, _)| {
                parents_of
                    .get(code)
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
                    .unwrap_or_default()
            })
            .collect();

        let builder: Vec<BuilderConcept<'_>> = batch
            .iter()
            .enumerate()
            .map(|(i, (code, display))| BuilderConcept {
                code: code.as_str(),
                display: Some(display.as_str()),
                parent_code: parents_of
                    .get(code)
                    .and_then(|p| p.first().map(|s| s.as_str())),
                extra_properties: extras_per[i].as_slice(),
                ..Default::default()
            })
            .collect();

        let bytes = build_code_system_bundle(&meta, &builder);
        let chunk = backend.import_bundle(ctx, &bytes).await?;
        stats.errors.extend(chunk.errors);

        eprintln!(
            "[rxnorm] concept batch {}/{total_batches} — +{} concepts (total: {})",
            batch_idx + 1,
            batch.len(),
            ((batch_idx + 1) * batch_size).min(total)
        );
    }

    stats.concepts = total as u32;
    Ok(stats)
}

// ── File reader ───────────────────────────────────────────────────────────────

fn read_rrf_files(path: &Path) -> Result<(Vec<u8>, Vec<u8>), HtsError> {
    if path.is_dir() {
        let conso = std::fs::read(path.join("RXNCONSO.RRF")).map_err(|e| {
            HtsError::InvalidRequest(format!(
                "Cannot read RXNCONSO.RRF in '{}': {e}",
                path.display()
            ))
        })?;
        let rel = std::fs::read(path.join("RXNREL.RRF")).map_err(|e| {
            HtsError::InvalidRequest(format!(
                "Cannot read RXNREL.RRF in '{}': {e}",
                path.display()
            ))
        })?;
        return Ok((conso, rel));
    }

    let file = std::fs::File::open(path)
        .map_err(|e| HtsError::InvalidRequest(format!("Cannot open '{}': {e}", path.display())))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| HtsError::InvalidRequest(format!("Invalid ZIP '{}': {e}", path.display())))?;

    let conso_idx = find_zip_entry(&mut archive, "rxnconso.rrf").ok_or_else(|| {
        HtsError::InvalidRequest(format!("RXNCONSO.RRF not found in '{}'", path.display()))
    })?;
    let rel_idx = find_zip_entry(&mut archive, "rxnrel.rrf").ok_or_else(|| {
        HtsError::InvalidRequest(format!("RXNREL.RRF not found in '{}'", path.display()))
    })?;

    let conso = read_zip_entry(&mut archive, conso_idx)?;
    let rel = read_zip_entry(&mut archive, rel_idx)?;
    Ok((conso, rel))
}

fn find_zip_entry(archive: &mut zip::ZipArchive<std::fs::File>, suffix: &str) -> Option<usize> {
    (0..archive.len()).find(|&i| {
        archive
            .by_index(i)
            .ok()
            .map(|e| e.name().to_ascii_lowercase().ends_with(suffix))
            .unwrap_or(false)
    })
}

fn read_zip_entry(
    archive: &mut zip::ZipArchive<std::fs::File>,
    index: usize,
) -> Result<Vec<u8>, HtsError> {
    use std::io::Read;
    let mut entry = archive
        .by_index(index)
        .map_err(|e| HtsError::InvalidRequest(format!("Cannot open ZIP entry: {e}")))?;
    let mut buf = Vec::new();
    entry
        .read_to_end(&mut buf)
        .map_err(|e| HtsError::InvalidRequest(format!("Cannot read ZIP entry: {e}")))?;
    Ok(buf)
}

// ── RRF parsers ───────────────────────────────────────────────────────────────

fn parse_concepts(
    reader: impl BufRead,
    errors: &mut Vec<String>,
) -> Result<HashMap<String, String>, HtsError> {
    let mut concepts: HashMap<String, String> = HashMap::new();
    let mut preferred: HashMap<String, bool> = HashMap::new();

    for (line_no, line) in reader.lines().enumerate() {
        let line = line.map_err(|e| {
            HtsError::InvalidRequest(format!("RXNCONSO read error line {}: {e}", line_no + 1))
        })?;
        let line = line.trim_end_matches('|');

        let fields: Vec<&str> = line.split('|').collect();
        if fields.len() < 15 {
            errors.push(format!(
                "RXNCONSO.RRF line {}: expected ≥15 fields, got {} — skipped",
                line_no + 1,
                fields.len()
            ));
            continue;
        }

        let rxcui = fields[0];
        let lat = fields[1];
        let ispref = fields[6];
        let sab = fields[11];
        let str_val = fields[14];

        if lat != "ENG" || sab != "RXNORM" {
            continue;
        }
        if fields.len() > 16 && fields[16] == "O" {
            continue;
        }
        if str_val.is_empty() {
            continue;
        }

        let is_pref = ispref == "Y";
        let already_preferred = *preferred.get(rxcui).unwrap_or(&false);

        if is_pref || !concepts.contains_key(rxcui) {
            if !already_preferred || is_pref {
                concepts.insert(rxcui.to_string(), str_val.to_string());
                preferred.insert(rxcui.to_string(), is_pref);
            }
        }
    }

    Ok(concepts)
}

fn parse_relationships(
    reader: impl BufRead,
    active_concepts: &HashMap<String, String>,
    errors: &mut Vec<String>,
) -> Result<Vec<(String, String)>, HtsError> {
    let mut edges: Vec<(String, String)> = Vec::new();

    for (line_no, line) in reader.lines().enumerate() {
        let line = line.map_err(|e| {
            HtsError::InvalidRequest(format!("RXNREL read error line {}: {e}", line_no + 1))
        })?;
        let line = line.trim_end_matches('|');

        let fields: Vec<&str> = line.split('|').collect();
        if fields.len() < 11 {
            errors.push(format!(
                "RXNREL.RRF line {}: expected ≥11 fields, got {} — skipped",
                line_no + 1,
                fields.len()
            ));
            continue;
        }

        let rxcui1 = fields[0];
        let rxcui2 = fields[4];
        let rela = fields[7];
        let sab = fields[10];

        if sab != "RXNORM" || rela != "isa" {
            continue;
        }
        if fields.len() > 14 && fields[14] == "O" {
            continue;
        }
        if !active_concepts.contains_key(rxcui1) || !active_concepts.contains_key(rxcui2) {
            continue;
        }

        edges.push((rxcui1.to_string(), rxcui2.to_string()));
    }

    edges.sort_unstable();
    edges.dedup();
    Ok(edges)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    use super::*;
    use crate::backends::SqliteTerminologyBackend;

    const CONSO_RRF: &str = "\
1049502|ENG|P|L0000001|PF|S0000001|Y|1049502|||1049502|RXNORM|IN|1049502|acetaminophen|0|N|\n\
1049520|ENG|P|L0000002|PF|S0000002|Y|1049520|||1049520|RXNORM|IN|1049520|ibuprofen|0|N|\n\
198444|ENG|P|L0000003|PF|S0000003|Y|198444|||198444|RXNORM|BN|198444|Tylenol|0|N|\n\
1049527|ENG|P|L0000004|PF|S0000004|Y|1049527|||1049527|RXNORM|SCD|1049527|acetaminophen 325 MG Oral Tablet|0|N|\n\
9999999|ENG|P|L0000005|PF|S0000005|Y|9999999|||9999999|RXNORM|IN|9999999|suppressed_drug|0|O|\n";

    const REL_RRF: &str = "\
198444||RXCUI|RN|1049502||RXCUI|isa|RUI001||RXNORM|||N|N|N|\n\
1049527||RXCUI|RN|1049502||RXCUI|isa|RUI002||RXNORM|||N|N|N|\n\
9999999||RXCUI|RN|1049502||RXCUI|isa|RUI003||RXNORM|||N|N|O|\n";

    fn count_rows(backend: &SqliteTerminologyBackend, table: &str) -> i64 {
        let conn = backend.pool().get().unwrap();
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
    }

    fn make_folder() -> tempfile::TempDir {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        std::fs::File::create(dir.path().join("RXNCONSO.RRF"))
            .unwrap()
            .write_all(CONSO_RRF.as_bytes())
            .unwrap();
        std::fs::File::create(dir.path().join("RXNREL.RRF"))
            .unwrap()
            .write_all(REL_RRF.as_bytes())
            .unwrap();
        dir
    }

    // ── Parser unit tests ─────────────────────────────────────────────────

    #[test]
    fn parse_concepts_returns_four_active_concepts() {
        let mut errors = Vec::new();
        let concepts = parse_concepts(BufReader::new(CONSO_RRF.as_bytes()), &mut errors).unwrap();
        assert_eq!(concepts.len(), 4);
        assert_eq!(concepts["1049502"], "acetaminophen");
        assert_eq!(concepts["198444"], "Tylenol");
        assert!(errors.is_empty());
    }

    #[test]
    fn parse_concepts_filters_non_rxnorm_source() {
        let data =
            "1111111|ENG|P|L1|PF|S1|Y|1111111|||1111111|SNOMEDCT_US|IN|1111111|SomeCode|0|N|\n";
        let mut errors = Vec::new();
        let concepts = parse_concepts(BufReader::new(data.as_bytes()), &mut errors).unwrap();
        assert!(concepts.is_empty());
    }

    #[test]
    fn parse_concepts_filters_non_english() {
        let data = "1111111|SPA|P|L1|PF|S1|Y|1111111|||1111111|RXNORM|IN|1111111|aspirina|0|N|\n";
        let mut errors = Vec::new();
        let concepts = parse_concepts(BufReader::new(data.as_bytes()), &mut errors).unwrap();
        assert!(concepts.is_empty());
    }

    #[test]
    fn parse_concepts_prefers_ispref_y() {
        let data = "\
1049502|ENG|P|L1|PF|S1|N|1049502|||1049502|RXNORM|IN|1049502|acetaminophen alt|0|N|\n\
1049502|ENG|P|L1|PF|S2|Y|1049502|||1049502|RXNORM|IN|1049502|acetaminophen|0|N|\n";
        let mut errors = Vec::new();
        let concepts = parse_concepts(BufReader::new(data.as_bytes()), &mut errors).unwrap();
        assert_eq!(concepts["1049502"], "acetaminophen");
    }

    #[test]
    fn parse_relationships_returns_two_isa_edges() {
        let mut errors = Vec::new();
        let concepts = parse_concepts(BufReader::new(CONSO_RRF.as_bytes()), &mut errors).unwrap();
        let edges = parse_relationships(BufReader::new(REL_RRF.as_bytes()), &concepts, &mut errors)
            .unwrap();
        assert_eq!(edges.len(), 2);
        assert!(edges.contains(&("198444".to_string(), "1049502".to_string())));
        assert!(edges.contains(&("1049527".to_string(), "1049502".to_string())));
    }

    #[test]
    fn parse_relationships_skips_non_isa_rela() {
        let concepts = {
            let mut m = HashMap::new();
            m.insert("A".to_string(), "Drug A".to_string());
            m.insert("B".to_string(), "Drug B".to_string());
            m
        };
        let data = "A||RXCUI|RO|B||RXCUI|ingredient_of|RUI001||RXNORM||||N|\n";
        let mut errors = Vec::new();
        let edges =
            parse_relationships(BufReader::new(data.as_bytes()), &concepts, &mut errors).unwrap();
        assert!(edges.is_empty());
    }

    #[test]
    fn parse_conso_malformed_line_recorded_in_errors() {
        let data = "\
1049502|ENG|P|L0000001|PF|S0000001|Y|1049502|||1049502|RXNORM|IN|1049502|acetaminophen|0|N|\n\
BAD|LINE|ONLY_THREE_FIELDS\n";
        let mut errors = Vec::new();
        let concepts = parse_concepts(BufReader::new(data.as_bytes()), &mut errors).unwrap();
        assert_eq!(concepts.len(), 1);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("line 2"));
    }

    // ── Integration tests ─────────────────────────────────────────────────

    #[tokio::test]
    async fn import_rxnorm_dry_run_returns_correct_counts() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();
        let dir = make_folder();

        let stats = import_rxnorm_rrf(&backend, &ctx, dir.path(), 500, true)
            .await
            .unwrap();

        assert_eq!(stats.code_systems, 1);
        assert_eq!(stats.concepts, 4);
        assert!(stats.errors.is_empty());
    }

    #[tokio::test]
    async fn import_rxnorm_dry_run_does_not_write_to_db() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();
        let dir = make_folder();

        import_rxnorm_rrf(&backend, &ctx, dir.path(), 500, true)
            .await
            .unwrap();

        assert_eq!(count_rows(&backend, "code_systems"), 0);
        assert_eq!(count_rows(&backend, "concepts"), 0);
    }

    #[tokio::test]
    async fn import_rxnorm_live_writes_to_db() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();
        let dir = make_folder();

        let stats = import_rxnorm_rrf(&backend, &ctx, dir.path(), 500, false)
            .await
            .unwrap();

        assert_eq!(stats.code_systems, 1);
        assert_eq!(stats.concepts, 4);
        assert_eq!(count_rows(&backend, "code_systems"), 1);
        assert_eq!(count_rows(&backend, "concepts"), 4);
        assert_eq!(count_rows(&backend, "concept_hierarchy"), 2);
    }

    #[tokio::test]
    async fn import_rxnorm_idempotent_reimport() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();
        let dir = make_folder();

        import_rxnorm_rrf(&backend, &ctx, dir.path(), 500, false)
            .await
            .unwrap();
        import_rxnorm_rrf(&backend, &ctx, dir.path(), 500, false)
            .await
            .unwrap();

        assert_eq!(count_rows(&backend, "code_systems"), 1);
        assert_eq!(count_rows(&backend, "concepts"), 4);
        assert_eq!(count_rows(&backend, "concept_hierarchy"), 2);
    }

    #[tokio::test]
    async fn import_rxnorm_batching_preserves_all_concepts() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();
        let dir = make_folder();

        let stats = import_rxnorm_rrf(&backend, &ctx, dir.path(), 2, false)
            .await
            .unwrap();

        assert_eq!(stats.concepts, 4);
        assert_eq!(count_rows(&backend, "concepts"), 4);
        assert_eq!(count_rows(&backend, "concept_hierarchy"), 2);
    }

    #[tokio::test]
    async fn import_rxnorm_missing_folder_returns_error() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();
        let result =
            import_rxnorm_rrf(&backend, &ctx, Path::new("/nonexistent/rxnorm"), 500, false).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn import_rxnorm_lookup_drug_code() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();
        let dir = make_folder();

        import_rxnorm_rrf(&backend, &ctx, dir.path(), 500, false)
            .await
            .unwrap();

        let conn = backend.pool().get().unwrap();
        let display: String = conn
            .query_row(
                "SELECT display FROM concepts WHERE code = ?1",
                rusqlite::params!["1049527"],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(display, "acetaminophen 325 MG Oral Tablet");
    }

    #[tokio::test]
    async fn import_rxnorm_from_zip() {
        use std::io::Write;

        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("rxnorm_full_current.zip");

        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let opts = zip::write::FileOptions::default();
            zip.start_file("rrf/RXNCONSO.RRF", opts).unwrap();
            zip.write_all(CONSO_RRF.as_bytes()).unwrap();
            zip.start_file("rrf/RXNREL.RRF", opts).unwrap();
            zip.write_all(REL_RRF.as_bytes()).unwrap();
            zip.finish().unwrap();
        }

        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();

        let stats = import_rxnorm_rrf(&backend, &ctx, &zip_path, 500, false)
            .await
            .unwrap();
        assert_eq!(stats.concepts, 4);
        assert_eq!(count_rows(&backend, "concepts"), 4);
        assert_eq!(count_rows(&backend, "concept_hierarchy"), 2);
    }
}
