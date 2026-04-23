//! RxNorm RRF importer.
//!
//! Parses the NLM RxNorm full release distribution and imports drug concepts,
//! preferred display names, `isa` hierarchy edges, TTY term-type properties,
//! and named role relationships (tradename_of, has_ingredient, has_dose_form,
//! etc.) into the HTS normalized schema.
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

// ── Relationship helpers ──────────────────────────────────────────────────────

/// Returns the semantic inverse of a named RxNorm relationship, or `None` if
/// we only need to store the relationship in the forward direction.
///
/// RxNorm RXNREL contains both directions for most relationships, but some
/// datasets only include one direction. Generating the inverse ensures that
/// FHIR property filters (e.g. `tradename_of=CUI:161`) work regardless of
/// which direction the source file uses.
fn inverse_rela(rela: &str) -> Option<&'static str> {
    match rela {
        "has_tradename" => Some("tradename_of"),
        "ingredient_of" => Some("has_ingredient"),
        "dose_form_of" => Some("has_dose_form"),
        "part_of" => Some("has_part"),
        "quantified_form_of" => Some("has_quantified_form"),
        "contained_in" => Some("consists_of"),
        "constitutes" => Some("reformulated_to"),
        "reformulation_of" => Some("has_reformulated_drug"),
        _ => None,
    }
}

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

    // rxcui -> (display, tty)
    type ConceptMap = HashMap<String, (String, String)>;
    // (rxcui1, rela, rxcui2)
    type RelVec = Vec<(String, String, String)>;
    type RxnormParsed = (ConceptMap, RelVec, Vec<String>);

    let path_owned = path.to_path_buf();
    let (concepts, relationships, parse_errors) =
        tokio::task::spawn_blocking(move || -> Result<RxnormParsed, HtsError> {
            let (conso_bytes, rel_bytes) = read_rrf_files(&path_owned)?;
            let mut parse_errors: Vec<String> = Vec::new();
            let concepts =
                parse_concepts(BufReader::new(conso_bytes.as_slice()), &mut parse_errors)?;
            let relationships = parse_relationships(
                BufReader::new(rel_bytes.as_slice()),
                &concepts,
                &mut parse_errors,
            )?;
            Ok((concepts, relationships, parse_errors))
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
        eprintln!("[rxnorm] dry-run — no DB writes");
        return Ok(stats);
    }

    // Build child → parents map (isa edges; a concept can have multiple parents).
    let mut parents_of: HashMap<String, Vec<String>> = HashMap::new();
    // Build concept → role properties (tradename_of, has_ingredient, etc.)
    // Values are stored as "CUI:{rxcui}" to match the FHIR property filter convention.
    let mut roles_of: HashMap<String, Vec<(String, String)>> = HashMap::new();

    for (rxcui1, rela, rxcui2) in &relationships {
        if rela == "isa" {
            parents_of
                .entry(rxcui1.clone())
                .or_default()
                .push(rxcui2.clone());
        } else {
            // Forward: store the relationship as-is on rxcui1.
            roles_of
                .entry(rxcui1.clone())
                .or_default()
                .push((rela.clone(), format!("CUI:{rxcui2}")));
            // Inverse: some relationships in RXNREL only appear in one direction
            // (e.g. IN→has_tradename→BN but NOT BN→tradename_of→IN). Store the
            // semantic inverse on rxcui2 so property filters work regardless of
            // which direction the row appears in the source file.
            if let Some(inv) = inverse_rela(rela) {
                roles_of
                    .entry(rxcui2.clone())
                    .or_default()
                    .push((inv.to_string(), format!("CUI:{rxcui1}")));
            }
        }
    }

    // Remove duplicate (property, value) pairs that arise when both forward and
    // inverse directions appear in RXNREL for the same concept pair.
    for props in roles_of.values_mut() {
        props.sort_unstable();
        props.dedup();
    }

    // In RxNorm, BN (brand name) concepts are linked to their ingredient (IN/MIN) via `isa`
    // hierarchy, not via a direct RXNREL `tradename_of` row. RXNREL `tradename_of` only
    // appears between SBD (branded drug) and SCD (clinical drug) formulations. Derive the
    // FHIR `tradename_of` property for each BN concept from its `isa` parents that are IN or MIN.
    for (rxcui, (_, tty)) in &concepts {
        if tty == "BN" {
            if let Some(parents) = parents_of.get(rxcui) {
                for parent_rxcui in parents {
                    if let Some((_, parent_tty)) = concepts.get(parent_rxcui) {
                        if parent_tty == "IN" || parent_tty == "MIN" {
                            roles_of
                                .entry(rxcui.clone())
                                .or_default()
                                .push(("tradename_of".to_string(), format!("CUI:{parent_rxcui}")));
                        }
                    }
                }
            }
        }
    }
    for props in roles_of.values_mut() {
        props.sort_unstable();
        props.dedup();
    }

    // ── Diagnostics (temporary — understand BN→isa structure) ────────────────
    {
        let bn_concepts: Vec<&String> = concepts
            .iter()
            .filter(|(_, (_, tty))| tty == "BN")
            .map(|(rxcui, _)| rxcui)
            .collect();
        let bn_with_any_isa = bn_concepts
            .iter()
            .filter(|rxcui| parents_of.contains_key(**rxcui))
            .count();
        let bn_with_in_parent = bn_concepts
            .iter()
            .filter(|rxcui| {
                parents_of.get(**rxcui).map(|parents| {
                    parents.iter().any(|p| {
                        concepts.get(p).map(|(_, tty)| tty == "IN" || tty == "MIN").unwrap_or(false)
                    })
                }).unwrap_or(false)
            })
            .count();
        // Show what isa parents exist for a sample BN concept (the first one that has isa parents)
        let sample_bn_with_isa = bn_concepts.iter()
            .find(|rxcui| parents_of.contains_key(**rxcui));
        let sample_parents: Vec<(String, String)> = sample_bn_with_isa
            .and_then(|rxcui| parents_of.get(*rxcui))
            .map(|parents| parents.iter()
                .take(5)
                .map(|p| (p.clone(), concepts.get(p).map(|(_, tty)| tty.clone()).unwrap_or("?".into())))
                .collect())
            .unwrap_or_default();
        // Show what relationships CUI:161 participates in (forward only)
        let rels_involving_161: Vec<&(String, String, String)> = relationships.iter()
            .filter(|(r1, _, r2)| r1 == "161" || r2 == "161")
            .take(10)
            .collect();
        let tradename_of_derived = roles_of.values()
            .filter(|props| props.iter().any(|(p, _)| p == "tradename_of"))
            .count();
        let tradename_of_161_count = roles_of.values()
            .filter(|props| props.iter().any(|(p, v)| p == "tradename_of" && v == "CUI:161"))
            .count();
        eprintln!("[rxnorm-diag2] BN total={} with_isa={} with_IN_parent={} tradename_of_derived={} tradename_of→161={}",
            bn_concepts.len(), bn_with_any_isa, bn_with_in_parent, tradename_of_derived, tradename_of_161_count);
        eprintln!("[rxnorm-diag2] sample BN {:?} isa parents (rxcui,tty): {:?}", sample_bn_with_isa, sample_parents);
        eprintln!("[rxnorm-diag2] rels involving CUI:161: {:?}", rels_involving_161);
    }
    // ── End diagnostics ──────────────────────────────────────────────────────

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

    let concept_list: Vec<(String, String, String)> = concepts
        .into_iter()
        .map(|(rxcui, (display, tty))| (rxcui, display, tty))
        .collect();
    let total = concept_list.len();

    for batch in concept_list.chunks(batch_size) {
        let extras_per: Vec<Vec<BuilderProperty<'_>>> = batch
            .iter()
            .map(|(code, _, tty)| {
                let mut props: Vec<BuilderProperty<'_>> = Vec::new();
                // TTY term type (IN, BN, SCD, SBD, MIN, SCDC, …)
                props.push(BuilderProperty {
                    code: "TTY",
                    value_key: "valueCode",
                    value: tty.as_str(),
                });
                // Additional isa parents beyond the first (first goes via parent_code)
                if let Some(parents) = parents_of.get(code) {
                    for p in parents.iter().skip(1) {
                        props.push(BuilderProperty {
                            code: "parent",
                            value_key: "valueCode",
                            value: p.as_str(),
                        });
                    }
                }
                // Role relationships: tradename_of, has_ingredient, has_dose_form, …
                if let Some(roles) = roles_of.get(code) {
                    for (rela, cui_val) in roles {
                        props.push(BuilderProperty {
                            code: rela.as_str(),
                            value_key: "valueCode",
                            value: cui_val.as_str(),
                        });
                    }
                }
                props
            })
            .collect();

        let builder: Vec<BuilderConcept<'_>> = batch
            .iter()
            .enumerate()
            .map(|(i, (code, display, _tty))| BuilderConcept {
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

/// Returns a map of RXCUI → (preferred display, TTY term type).
fn parse_concepts(
    reader: impl BufRead,
    errors: &mut Vec<String>,
) -> Result<HashMap<String, (String, String)>, HtsError> {
    // rxcui -> (display, tty, is_preferred)
    let mut raw: HashMap<String, (String, String, bool)> = HashMap::new();

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
        let tty = fields[12];
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
        let already_preferred = raw.get(rxcui).map(|(_, _, p)| *p).unwrap_or(false);

        if is_pref || !raw.contains_key(rxcui) {
            if !already_preferred || is_pref {
                raw.insert(
                    rxcui.to_string(),
                    (str_val.to_string(), tty.to_string(), is_pref),
                );
            }
        }
    }

    Ok(raw
        .into_iter()
        .map(|(rxcui, (display, tty, _))| (rxcui, (display, tty)))
        .collect())
}

/// Returns all named RxNorm relationships as `(rxcui1, rela, rxcui2)` triples.
///
/// Includes `isa` hierarchy edges and role relationships such as `tradename_of`,
/// `has_ingredient`, and `has_dose_form`.  Only rows where both endpoints are
/// active concepts are kept.
fn parse_relationships(
    reader: impl BufRead,
    active_concepts: &HashMap<String, (String, String)>,
    errors: &mut Vec<String>,
) -> Result<Vec<(String, String, String)>, HtsError> {
    let mut relationships: Vec<(String, String, String)> = Vec::new();
    // Diagnostic: collect up to 20 RXNREL rows that involve a BN concept (any RELA/REL).
    let mut bn_rows: Vec<String> = Vec::new();

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
        let rel   = fields[3];
        let rela  = fields[7];
        let sab   = fields[10];

        // Diagnostic: log raw RXNREL rows for BN concepts (before any filtering).
        if bn_rows.len() < 20 && sab == "RXNORM" {
            let c1_tty = active_concepts.get(rxcui1).map(|(_, t)| t.as_str()).unwrap_or("?");
            let c2_tty = active_concepts.get(rxcui2).map(|(_, t)| t.as_str()).unwrap_or("?");
            if c1_tty == "BN" || c2_tty == "BN" {
                bn_rows.push(format!(
                    "({rxcui1}/{c1_tty}) REL={rel} RELA={rela:?} ({rxcui2}/{c2_tty})"
                ));
            }
        }

        // Only RxNorm-sourced, named relationships (rela must be non-empty).
        if sab != "RXNORM" || rela.is_empty() {
            continue;
        }
        if fields.len() > 14 && fields[14] == "O" {
            continue;
        }
        if !active_concepts.contains_key(rxcui1) || !active_concepts.contains_key(rxcui2) {
            continue;
        }

        relationships.push((rxcui1.to_string(), rela.to_string(), rxcui2.to_string()));
    }

    eprintln!("[rxnorm-diag3] BN-related RXNREL rows (first 20, before filters): {bn_rows:?}");

    relationships.sort_unstable();
    relationships.dedup();
    Ok(relationships)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    use super::*;
    use crate::backends::SqliteTerminologyBackend;

    // RXNCONSO fields: RXCUI|LAT|TS|LUI|STT|SUI|ISPREF|RXAUI|SAUI|SCUI|SDUI|SAB|TTY|CODE|STR|SRL|SUPPRESS|CVF
    const CONSO_RRF: &str = "\
1049502|ENG|P|L0000001|PF|S0000001|Y|1049502|||1049502|RXNORM|IN|1049502|acetaminophen|0|N|\n\
1049520|ENG|P|L0000002|PF|S0000002|Y|1049520|||1049520|RXNORM|IN|1049520|ibuprofen|0|N|\n\
198444|ENG|P|L0000003|PF|S0000003|Y|198444|||198444|RXNORM|BN|198444|Tylenol|0|N|\n\
1049527|ENG|P|L0000004|PF|S0000004|Y|1049527|||1049527|RXNORM|SCD|1049527|acetaminophen 325 MG Oral Tablet|0|N|\n\
9999999|ENG|P|L0000005|PF|S0000005|Y|9999999|||9999999|RXNORM|IN|9999999|suppressed_drug|0|O|\n";

    // RXNREL fields: RXCUI1|RXAUI1|STYPE1|REL|RXCUI2|RXAUI2|STYPE2|RELA|RUI|SRUI|SAB|SL|DIR|RG|SUPPRESS|CVF
    const REL_RRF: &str = "\
198444||RXCUI|RN|1049502||RXCUI|isa|RUI001||RXNORM|||N|N|N|\n\
1049527||RXCUI|RN|1049502||RXCUI|isa|RUI002||RXNORM|||N|N|N|\n\
9999999||RXCUI|RN|1049502||RXCUI|isa|RUI003||RXNORM|||N|N|O|\n\
198444||RXCUI|RO|1049502||RXCUI|tradename_of|RUI004||RXNORM|||N|N|N|\n";

    fn count_rows(backend: &SqliteTerminologyBackend, table: &str) -> i64 {
        let conn = backend.pool().get().unwrap();
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
    }

    fn count_property(backend: &SqliteTerminologyBackend, property: &str) -> i64 {
        let conn = backend.pool().get().unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM concept_properties WHERE property = ?1",
            rusqlite::params![property],
            |row| row.get(0),
        )
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
        assert_eq!(concepts["1049502"].0, "acetaminophen");
        assert_eq!(concepts["198444"].0, "Tylenol");
        assert!(errors.is_empty());
    }

    #[test]
    fn parse_concepts_stores_tty() {
        let mut errors = Vec::new();
        let concepts = parse_concepts(BufReader::new(CONSO_RRF.as_bytes()), &mut errors).unwrap();
        assert_eq!(concepts["1049502"].1, "IN");
        assert_eq!(concepts["198444"].1, "BN");
        assert_eq!(concepts["1049527"].1, "SCD");
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
        assert_eq!(concepts["1049502"].0, "acetaminophen");
    }

    #[test]
    fn parse_relationships_returns_isa_and_role_edges() {
        let mut errors = Vec::new();
        let concepts = parse_concepts(BufReader::new(CONSO_RRF.as_bytes()), &mut errors).unwrap();
        let rels = parse_relationships(BufReader::new(REL_RRF.as_bytes()), &concepts, &mut errors)
            .unwrap();
        // 2 isa edges + 1 tradename_of (suppressed isa skipped)
        assert_eq!(rels.len(), 3);
        assert!(rels.contains(&(
            "198444".to_string(),
            "isa".to_string(),
            "1049502".to_string()
        )));
        assert!(rels.contains(&(
            "1049527".to_string(),
            "isa".to_string(),
            "1049502".to_string()
        )));
        assert!(rels.contains(&(
            "198444".to_string(),
            "tradename_of".to_string(),
            "1049502".to_string()
        )));
    }

    #[test]
    fn parse_relationships_stores_non_isa_rela() {
        let concepts = {
            let mut m = HashMap::new();
            m.insert("A".to_string(), ("Drug A".to_string(), "IN".to_string()));
            m.insert("B".to_string(), ("Drug B".to_string(), "IN".to_string()));
            m
        };
        let data = "A||RXCUI|RO|B||RXCUI|ingredient_of|RUI001||RXNORM||||N|\n";
        let mut errors = Vec::new();
        let rels =
            parse_relationships(BufReader::new(data.as_bytes()), &concepts, &mut errors).unwrap();
        assert_eq!(rels.len(), 1);
        assert_eq!(
            rels[0],
            (
                "A".to_string(),
                "ingredient_of".to_string(),
                "B".to_string()
            )
        );
    }

    #[test]
    fn parse_relationships_skips_empty_rela() {
        let concepts = {
            let mut m = HashMap::new();
            m.insert("A".to_string(), ("Drug A".to_string(), "IN".to_string()));
            m.insert("B".to_string(), ("Drug B".to_string(), "IN".to_string()));
            m
        };
        // REL without a RELA value (unnamed relationship)
        let data = "A||RXCUI|RO|B||RXCUI||RUI001||RXNORM||||N|\n";
        let mut errors = Vec::new();
        let rels =
            parse_relationships(BufReader::new(data.as_bytes()), &concepts, &mut errors).unwrap();
        assert!(rels.is_empty());
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
        // One TTY property per concept + one tradename_of for Tylenol
        assert_eq!(count_property(&backend, "TTY"), 4);
        assert_eq!(count_property(&backend, "tradename_of"), 1);
    }

    #[tokio::test]
    async fn import_rxnorm_tty_property_values() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();
        let dir = make_folder();

        import_rxnorm_rrf(&backend, &ctx, dir.path(), 500, false)
            .await
            .unwrap();

        let conn = backend.pool().get().unwrap();
        // Tylenol (198444) should have TTY = BN
        let tty: String = conn
            .query_row(
                "SELECT cp.value FROM concept_properties cp
                 JOIN concepts c ON c.id = cp.concept_id
                 WHERE c.code = ?1 AND cp.property = 'TTY'",
                rusqlite::params!["198444"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tty, "BN");

        // acetaminophen (1049502) should have TTY = IN
        let tty: String = conn
            .query_row(
                "SELECT cp.value FROM concept_properties cp
                 JOIN concepts c ON c.id = cp.concept_id
                 WHERE c.code = ?1 AND cp.property = 'TTY'",
                rusqlite::params!["1049502"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tty, "IN");
    }

    #[tokio::test]
    async fn import_rxnorm_tradename_of_property() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();
        let dir = make_folder();

        import_rxnorm_rrf(&backend, &ctx, dir.path(), 500, false)
            .await
            .unwrap();

        let conn = backend.pool().get().unwrap();
        // Tylenol (198444) tradename_of acetaminophen (1049502) → value "CUI:1049502"
        let val: String = conn
            .query_row(
                "SELECT cp.value FROM concept_properties cp
                 JOIN concepts c ON c.id = cp.concept_id
                 WHERE c.code = ?1 AND cp.property = 'tradename_of'",
                rusqlite::params!["198444"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(val, "CUI:1049502");
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
        assert_eq!(count_property(&backend, "TTY"), 4);
        assert_eq!(count_property(&backend, "tradename_of"), 1);
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
        assert_eq!(count_property(&backend, "TTY"), 4);
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
    async fn expand_property_filters_tty_and_tradename_of() {
        use crate::traits::ValueSetOperations;
        use crate::types::ExpandRequest;

        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();
        let dir = make_folder();

        import_rxnorm_rrf(&backend, &ctx, dir.path(), 500, false)
            .await
            .unwrap();

        // Mirrors EX06: TTY=BN AND tradename_of=CUI:<acetaminophen-rxcui>
        let resp = backend
            .expand(
                &ctx,
                ExpandRequest {
                    value_set: Some(serde_json::json!({
                        "resourceType": "ValueSet",
                        "compose": {
                            "include": [{
                                "system": RXNORM_URL,
                                "filter": [
                                    {"property": "TTY",          "op": "=", "value": "BN"},
                                    {"property": "tradename_of", "op": "=", "value": "CUI:1049502"}
                                ]
                            }]
                        }
                    })),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        // Tylenol (198444) has TTY=BN and tradename_of=CUI:1049502 (acetaminophen)
        assert!(
            !resp.contains.is_empty(),
            "Expected at least one brand; got empty expansion. \
             Concept properties may not be stored during import."
        );
        let codes: Vec<&str> = resp.contains.iter().map(|c| c.code.as_str()).collect();
        assert!(
            codes.contains(&"198444"),
            "Tylenol (198444) must be in results; got: {codes:?}"
        );
    }

    /// Mirrors the CI scenario: RXNREL only has the `has_tradename` direction
    /// (IN → has_tradename → BN) rather than the direct `tradename_of` row.
    /// The inverse logic must produce `tradename_of=CUI:{IN}` on the BN concept
    /// so that expand filters of the form `TTY=BN AND tradename_of=CUI:161` work.
    #[tokio::test]
    async fn expand_property_filters_via_inverse_has_tradename() {
        use crate::traits::ValueSetOperations;
        use crate::types::ExpandRequest;

        // REL data with ONLY the inverse `has_tradename` direction (no direct tradename_of row).
        let conso = "\
1049502|ENG|P|L0000001|PF|S0000001|Y|1049502|||1049502|RXNORM|IN|1049502|acetaminophen|0|N|\n\
198444|ENG|P|L0000002|PF|S0000002|Y|198444|||198444|RXNORM|BN|198444|Tylenol|0|N|\n";
        let rels = "\
1049502||RXCUI|RB|198444||RXCUI|has_tradename|RUI001||RXNORM|||N|N|N|\n";

        let dir = tempfile::tempdir().unwrap();
        {
            use std::io::Write;
            std::fs::File::create(dir.path().join("RXNCONSO.RRF"))
                .unwrap()
                .write_all(conso.as_bytes())
                .unwrap();
            std::fs::File::create(dir.path().join("RXNREL.RRF"))
                .unwrap()
                .write_all(rels.as_bytes())
                .unwrap();
        }

        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();

        import_rxnorm_rrf(&backend, &ctx, dir.path(), 500, false)
            .await
            .unwrap();

        // Verify the inverse property was stored on the BN concept.
        assert_eq!(
            count_property(&backend, "tradename_of"),
            1,
            "tradename_of property must be derived from has_tradename inverse"
        );

        // Verify expand with TTY=BN AND tradename_of=CUI:1049502 returns Tylenol.
        let resp = backend
            .expand(
                &ctx,
                ExpandRequest {
                    value_set: Some(serde_json::json!({
                        "resourceType": "ValueSet",
                        "compose": {
                            "include": [{
                                "system": RXNORM_URL,
                                "filter": [
                                    {"property": "TTY",          "op": "=", "value": "BN"},
                                    {"property": "tradename_of", "op": "=", "value": "CUI:1049502"}
                                ]
                            }]
                        }
                    })),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let codes: Vec<&str> = resp.contains.iter().map(|c| c.code.as_str()).collect();
        assert!(
            codes.contains(&"198444"),
            "Tylenol (198444) must appear when filtering via inverse has_tradename; got: {codes:?}"
        );
    }

    /// Real RxNorm: BN→IN linkage comes only from the `isa` hierarchy — no `tradename_of` or
    /// `has_tradename` row exists in RXNREL for that pair.  This matches the CI finding
    /// where acetaminophen (CUI:161) had zero tradename_of/has_tradename rows on either side.
    #[tokio::test]
    async fn expand_property_filters_via_isa_derived_tradename_of() {
        use crate::traits::ValueSetOperations;
        use crate::types::ExpandRequest;

        let conso = "\
1049502|ENG|P|L0000001|PF|S0000001|Y|1049502|||1049502|RXNORM|IN|1049502|acetaminophen|0|N|\n\
198444|ENG|P|L0000002|PF|S0000002|Y|198444|||198444|RXNORM|BN|198444|Tylenol|0|N|\n";
        // Only isa edge — no tradename_of or has_tradename in RXNREL.
        let rels = "\
198444||RXCUI|RN|1049502||RXCUI|isa|RUI001||RXNORM|||N|N|N|\n";

        let dir = tempfile::tempdir().unwrap();
        {
            use std::io::Write;
            std::fs::File::create(dir.path().join("RXNCONSO.RRF"))
                .unwrap()
                .write_all(conso.as_bytes())
                .unwrap();
            std::fs::File::create(dir.path().join("RXNREL.RRF"))
                .unwrap()
                .write_all(rels.as_bytes())
                .unwrap();
        }

        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();

        import_rxnorm_rrf(&backend, &ctx, dir.path(), 500, false)
            .await
            .unwrap();

        assert_eq!(
            count_property(&backend, "tradename_of"),
            1,
            "tradename_of must be derived from BN isa IN when no direct RXNREL row exists"
        );

        let resp = backend
            .expand(
                &ctx,
                ExpandRequest {
                    value_set: Some(serde_json::json!({
                        "resourceType": "ValueSet",
                        "compose": {
                            "include": [{
                                "system": RXNORM_URL,
                                "filter": [
                                    {"property": "TTY",          "op": "=", "value": "BN"},
                                    {"property": "tradename_of", "op": "=", "value": "CUI:1049502"}
                                ]
                            }]
                        }
                    })),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let codes: Vec<&str> = resp.contains.iter().map(|c| c.code.as_str()).collect();
        assert!(
            codes.contains(&"198444"),
            "Tylenol must appear when tradename_of is derived from isa hierarchy; got: {codes:?}"
        );
    }

    /// When RXNREL has BOTH directions for the same pair (tradename_of AND has_tradename),
    /// dedup must prevent duplicate concept_properties rows.
    #[tokio::test]
    async fn inverse_rela_dedup_prevents_duplicate_properties() {
        let conso = "\
1049502|ENG|P|L0000001|PF|S0000001|Y|1049502|||1049502|RXNORM|IN|1049502|acetaminophen|0|N|\n\
198444|ENG|P|L0000002|PF|S0000002|Y|198444|||198444|RXNORM|BN|198444|Tylenol|0|N|\n";
        // Both directions present — dedup must keep only one tradename_of row.
        let rels = "\
198444||RXCUI|RN|1049502||RXCUI|tradename_of|RUI001||RXNORM|||N|N|N|\n\
1049502||RXCUI|RB|198444||RXCUI|has_tradename|RUI002||RXNORM|||N|N|N|\n";

        let dir = tempfile::tempdir().unwrap();
        {
            use std::io::Write;
            std::fs::File::create(dir.path().join("RXNCONSO.RRF"))
                .unwrap()
                .write_all(conso.as_bytes())
                .unwrap();
            std::fs::File::create(dir.path().join("RXNREL.RRF"))
                .unwrap()
                .write_all(rels.as_bytes())
                .unwrap();
        }

        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();
        import_rxnorm_rrf(&backend, &ctx, dir.path(), 500, false)
            .await
            .unwrap();

        // Dedup must eliminate the extra tradename_of generated by the inverse of has_tradename.
        assert_eq!(
            count_property(&backend, "tradename_of"),
            1,
            "dedup must prevent duplicate tradename_of rows when both directions are in RXNREL"
        );
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
        assert_eq!(count_property(&backend, "TTY"), 4);
        assert_eq!(count_property(&backend, "tradename_of"), 1);
    }
}
