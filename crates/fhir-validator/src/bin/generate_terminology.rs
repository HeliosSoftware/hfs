//! Dev-time generator for the embedded core terminology packs.
//!
//! Reads the FHIR spec value-set bundles already vendored for code generation
//! (`crates/fhir-gen/resources/{R4,R4B,R5,R6}/valuesets.json`, which ship both
//! `CodeSystem` and `ValueSet` resources), expands every ValueSet that can be
//! resolved **offline** — i.e. every `compose.include` names a `complete`
//! CodeSystem present in the bundle, with no filters or nested value-set
//! references — and writes one sorted, gzipped JSON map per version to
//! `crates/fhir-validator/packs/`.
//!
//! ValueSets that reference external code systems (SNOMED, LOINC, ...) or use
//! `filter` / `valueSet` includes are skipped: they cannot be checked without a
//! live terminology server, and the embedded [`CoreTerminology`] provider
//! reports "not checked" for them rather than guessing.
//!
//! A StructureDefinition authored against another FHIR release (the SQL on
//! FHIR ViewDefinition, `fhirVersion: 5.0.0`, ships in every version's spec
//! files) has its core bindings pinned to that release by
//! `generate-schema-packs` (`…/FHIR-version|5.0.0`). Those value sets are
//! expanded from that release's bundle and stored under the pinned URL, so a
//! pack checks them against the codes they actually mean. This reads the
//! schema pack, so run `generate-schema-packs` first.
//!
//! The packs are **committed** and embedded via `include_bytes!` in
//! `src/terminology.rs`. Re-run after spec updates:
//!
//! ```text
//! cargo run -p helios-fhir-validator --features gen-packs --bin generate-terminology
//! ```

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use helios_fhir_validator::terminology::{CORE_VALUE_SET_PREFIX, fhir_release};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let versions: Vec<&str> = if args.is_empty() {
        vec!["R4", "R4B", "R5", "R6"]
    } else {
        args.iter().map(String::as_str).collect()
    };

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let resources_root = manifest_dir.join("../fhir-gen/resources");
    let packs_dir = manifest_dir.join("packs");
    fs::create_dir_all(&packs_dir).expect("create packs dir");

    let mut failed = false;
    for version in versions {
        match generate(
            &resources_root.join(version).join("valuesets.json"),
            &packs_dir,
            version,
        ) {
            Ok(report) => println!("{report}"),
            Err(e) => {
                eprintln!("{version}: {e}");
                failed = true;
            }
        }
    }
    if failed {
        std::process::exit(1);
    }
}

/// A `complete` CodeSystem's fully-enumerated code list.
struct CodeSystem {
    complete: bool,
    codes: Vec<String>,
}

fn collect_concepts(concepts: Option<&Vec<Value>>, out: &mut Vec<String>) {
    let Some(concepts) = concepts else { return };
    for c in concepts {
        if let Some(code) = c.get("code").and_then(Value::as_str) {
            out.push(code.to_string());
        }
        collect_concepts(c.get("concept").and_then(Value::as_array), out);
    }
}

fn generate(source: &Path, packs_dir: &Path, version: &str) -> Result<String, String> {
    if !source.exists() {
        return Ok(format!(
            "{version}: SKIPPED — {} not present (run a fhir-gen build to download)",
            source.display()
        ));
    }
    let (mut table, unresolved) = expand_bundle(source)?;

    // Core value sets the schema pack pins to another release, expanded from
    // that release's bundle under the pinned URL.
    let mut foreign = 0usize;
    for pinned in foreign_pinned_value_sets(packs_dir, version)? {
        let Some((url, pinned_version)) = pinned.split_once('|') else {
            continue;
        };
        let Some(other) = fhir_release(pinned_version).and_then(version_dir) else {
            continue;
        };
        let other_source = source
            .parent()
            .and_then(Path::parent)
            .map(|resources| resources.join(other).join("valuesets.json"))
            .ok_or("no resources root")?;
        let (other_table, _) = expand_bundle(&other_source)?;
        if let Some(codes) = other_table.get(url) {
            table.insert(pinned.clone(), codes.clone());
            foreign += 1;
        }
    }

    // Diff-stable output: BTreeMap already sorts by url; the pairs list is sorted.
    let pairs: Vec<(&String, &Vec<(String, String)>)> = table.iter().collect();
    let json = serde_json::to_vec(&pairs).expect("table serializes");
    let out_path = packs_dir.join(format!("terminology_{}.json.gz", version.to_lowercase()));
    let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(&json).expect("gzip write");
    let compressed = encoder.finish().expect("gzip finish");
    fs::write(&out_path, &compressed).map_err(|e| format!("write pack: {e}"))?;

    Ok(format!(
        "{version}: {} value sets expanded ({foreign} from other releases; {unresolved} skipped as not offline-resolvable) → {} ({} KB raw, {} KB gz)",
        table.len(),
        out_path.display(),
        json.len() / 1024,
        compressed.len() / 1024
    ))
}

/// The spec directory holding a FHIR release (`major.minor`).
fn version_dir(release: &str) -> Option<&'static str> {
    match release {
        "4.0" => Some("R4"),
        "4.3" => Some("R4B"),
        "5.0" => Some("R5"),
        "6.0" => Some("R6"),
        _ => None,
    }
}

/// The versioned core value-set URLs `version`'s schema pack binds to a
/// release other than its own.
fn foreign_pinned_value_sets(packs_dir: &Path, version: &str) -> Result<BTreeSet<String>, String> {
    let path = packs_dir.join(format!("fhir_schemas_{}.json.gz", version.to_lowercase()));
    let mut json = Vec::new();
    GzDecoder::new(
        fs::read(&path)
            .map_err(|e| format!("read {}: {e}", path.display()))?
            .as_slice(),
    )
    .read_to_end(&mut json)
    .map_err(|e| format!("decompress {}: {e}", path.display()))?;
    let pack: Value =
        serde_json::from_slice(&json).map_err(|e| format!("parse schema pack: {e}"))?;
    let own = version_dir_release(version);
    let mut out = BTreeSet::new();
    collect_pinned(&pack, own, &mut out);
    Ok(out)
}

fn version_dir_release(version: &str) -> Option<&'static str> {
    ["4.0", "4.3", "5.0", "6.0"]
        .into_iter()
        .find(|release| version_dir(release) == Some(version))
}

fn collect_pinned(node: &Value, own: Option<&str>, out: &mut BTreeSet<String>) {
    match node {
        Value::Object(map) => {
            if let Some(value_set) = map
                .get("binding")
                .and_then(|b| b.get("valueSet"))
                .and_then(Value::as_str)
                && value_set.starts_with(CORE_VALUE_SET_PREFIX)
                && let Some((_, pinned)) = value_set.split_once('|')
                && fhir_release(pinned).is_some_and(|release| Some(release) != own)
            {
                out.insert(value_set.to_string());
            }
            for child in map.values() {
                collect_pinned(child, own, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_pinned(item, own, out);
            }
        }
        _ => {}
    }
}

/// Every offline-resolvable ValueSet in a spec value-set bundle, expanded,
/// plus the count of those that are not.
#[allow(clippy::type_complexity)]
fn expand_bundle(
    source: &Path,
) -> Result<(BTreeMap<String, Vec<(String, String)>>, usize), String> {
    let raw = fs::read_to_string(source).map_err(|e| format!("read: {e}"))?;
    let bundle: Value = serde_json::from_str(&raw).map_err(|e| format!("parse: {e}"))?;
    let entries = bundle
        .get("entry")
        .and_then(Value::as_array)
        .ok_or("no Bundle.entry")?;
    let resources: Vec<&Value> = entries.iter().filter_map(|e| e.get("resource")).collect();

    // Index CodeSystems by url.
    let mut systems: BTreeMap<String, CodeSystem> = BTreeMap::new();
    for r in &resources {
        if r.get("resourceType").and_then(Value::as_str) != Some("CodeSystem") {
            continue;
        }
        let Some(url) = r.get("url").and_then(Value::as_str) else {
            continue;
        };
        let mut codes = Vec::new();
        collect_concepts(r.get("concept").and_then(Value::as_array), &mut codes);
        systems.insert(
            url.to_string(),
            CodeSystem {
                complete: r.get("content").and_then(Value::as_str) == Some("complete"),
                codes,
            },
        );
    }

    // Expand every offline-resolvable ValueSet.
    let mut table: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    let mut unresolved = 0usize;
    for r in &resources {
        if r.get("resourceType").and_then(Value::as_str) != Some("ValueSet") {
            continue;
        }
        let Some(url) = r.get("url").and_then(Value::as_str) else {
            continue;
        };
        match expand(r, &systems) {
            Some(codes) if !codes.is_empty() => {
                table.insert(url.to_string(), codes);
            }
            _ => unresolved += 1,
        }
    }
    Ok((table, unresolved))
}

/// Fully expand a ValueSet using only `complete` in-bundle CodeSystems, or
/// `None` if any part needs a live terminology server (filters, external
/// systems, nested value sets).
fn expand(vs: &Value, systems: &BTreeMap<String, CodeSystem>) -> Option<Vec<(String, String)>> {
    let compose = vs.get("compose")?;
    let includes = compose.get("include").and_then(Value::as_array)?;

    let mut out: Vec<(String, String)> = Vec::new();
    for inc in includes {
        if inc.get("filter").is_some() || inc.get("valueSet").is_some() {
            return None; // needs a terminology server to resolve
        }
        let system = inc.get("system").and_then(Value::as_str)?;
        let cs = systems.get(system)?;
        if !cs.complete {
            return None;
        }
        match inc.get("concept").and_then(Value::as_array) {
            Some(concepts) => {
                for c in concepts {
                    if let Some(code) = c.get("code").and_then(Value::as_str) {
                        out.push((system.to_string(), code.to_string()));
                    }
                }
            }
            None => {
                for code in &cs.codes {
                    out.push((system.to_string(), code.clone()));
                }
            }
        }
    }

    // Remove enumerated excludes; bail if an exclude needs a server.
    if let Some(excludes) = compose.get("exclude").and_then(Value::as_array) {
        let mut drop = std::collections::HashSet::new();
        for ex in excludes {
            if ex.get("filter").is_some() || ex.get("valueSet").is_some() {
                return None;
            }
            let Some(system) = ex.get("system").and_then(Value::as_str) else {
                continue;
            };
            for c in ex
                .get("concept")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(code) = c.get("code").and_then(Value::as_str) {
                    drop.insert((system.to_string(), code.to_string()));
                }
            }
        }
        out.retain(|pair| !drop.contains(pair));
    }

    out.sort();
    out.dedup();
    Some(out)
}
