//! Chained-search benchmark: the shared application-side chain resolver
//! against the native SQL chain builders (#1341).
//!
//! REST resolves every chained and `_has` search through
//! `helios_persistence::search::resolve_chains` — iterative backend searches,
//! paged, the matches folded into an `_id` filter. The SQLite and PostgreSQL
//! backends also carry a `ChainQueryBuilder` each, reached only through
//! `ChainedSearchProvider::{resolve_chain, resolve_reverse_chain}`, that answers
//! the same question in one nested-subquery statement and has no production
//! caller. #1341 decides between deleting that path and making it the real
//! one; this harness is the measurement the decision asked for.
//!
//! For every query it runs BOTH paths, compares the id sets first — a
//! disagreement is reported and, by default, not timed: a wrong answer has no
//! latency worth quoting — and then measures each path: warm-up, N timed
//! iterations, min / median / p95. For the shared resolver it also reports how
//! many backend searches one resolution issued and how many rows and bytes
//! those searches returned, from one instrumented pass that is not timed.
//!
//! ```text
//! # deterministic synthetic corpus (no external data), 5,000 patients:
//! cargo run --release -p helios-persistence --features sqlite,postgres \
//!     --example chain_bench -- --patients 5000
//!
//! # the local Synthea NDJSON export, first 1,000 patients and everything
//! # that hangs off them:
//! cargo run --release -p helios-persistence --features sqlite,postgres \
//!     --example chain_bench -- --synthea /path/to/ndjson --patients 1000
//! ```
//!
//! Options:
//!
//! * `--patients N`     corpus size in patients (default 500)
//! * `--synthea DIR`    read `Patient.ndjson`, `Encounter.ndjson`,
//!                      `Observation.ndjson`, `Organization*.ndjson` and
//!                      `Practitioner*.ndjson` from DIR instead of generating
//! * `--seed N`         generator seed (default 1341)
//! * `--backends LIST`  comma list of `sqlite-file`, `sqlite-mem`, `postgres`
//!                      (default: all three; `postgres` starts a `postgres:16`
//!                      testcontainer with its default, untuned configuration)
//! * `--iters N`        timed iterations per path (default 20; fewer for a
//!                      query whose single run exceeds `--slow-ms`)
//! * `--warmup N`       untimed runs per path before measuring (default 3)
//! * `--slow-ms N`      a run slower than this drops the query to 5 timed
//!                      iterations and one warm-up (default 3000)
//! * `--db PATH`        SQLite database file (deleted first; default: a temp file)
//! * `--data-dir DIR`   directory holding `search-parameters-r4.json`
//!                      (default: the repository's `data/`)
//! * `--explain`        print `EXPLAIN QUERY PLAN` (SQLite file) and
//!                      `EXPLAIN (ANALYZE, BUFFERS)` (PostgreSQL) of the native
//!                      statement for the 3-hop and `_has` queries
//! * `--time-disagreements` time both paths even when their id sets differ
//! * `--only SUBSTR`    run only queries whose label contains SUBSTR
//!
//! Lines starting with `RESULT` are tab-separated and meant for `grep`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use helios_fhir::FhirVersion;
use helios_persistence::backends::postgres::{PostgresBackend, PostgresConfig};
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::core::{
    BulkProcessingOptions, BulkSubmitProvider, ChainedSearchProvider, ResourceStorage,
    SearchProvider, SearchResult, StreamingBulkSubmitProvider, SubmissionId,
};
use helios_persistence::error::StorageResult;
use helios_persistence::search::{SearchParameterRegistry, parse_typed_values, resolve_chains};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{
    ChainedParameter, Page, ReverseChainedParameter, SearchParameter, SearchQuery, SearchValue,
    StoredResource,
};
use parking_lot::RwLock;
use serde_json::{Value, json};

// ───────────────────────────── arguments ─────────────────────────────

struct Args {
    patients: usize,
    synthea: Option<PathBuf>,
    seed: u64,
    backends: Vec<String>,
    iters: usize,
    warmup: usize,
    slow_ms: u64,
    db: Option<PathBuf>,
    data_dir: PathBuf,
    explain: bool,
    time_disagreements: bool,
    only: Option<String>,
}

fn parse_args() -> Args {
    let mut args = Args {
        patients: 500,
        synthea: None,
        seed: 1341,
        backends: vec!["sqlite-file".into(), "sqlite-mem".into(), "postgres".into()],
        iters: 20,
        warmup: 3,
        slow_ms: 3000,
        db: None,
        data_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("data"),
        explain: false,
        time_disagreements: false,
        only: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().unwrap_or_else(|| panic!("{name} needs a value"));
        match arg.as_str() {
            "--patients" => args.patients = value("--patients").parse().expect("--patients N"),
            "--synthea" => args.synthea = Some(PathBuf::from(value("--synthea"))),
            "--seed" => args.seed = value("--seed").parse().expect("--seed N"),
            "--backends" => {
                args.backends = value("--backends")
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .collect()
            }
            "--iters" => args.iters = value("--iters").parse().expect("--iters N"),
            "--warmup" => args.warmup = value("--warmup").parse().expect("--warmup N"),
            "--slow-ms" => args.slow_ms = value("--slow-ms").parse().expect("--slow-ms N"),
            "--db" => args.db = Some(PathBuf::from(value("--db"))),
            "--data-dir" => args.data_dir = PathBuf::from(value("--data-dir")),
            "--explain" => args.explain = true,
            "--time-disagreements" => args.time_disagreements = true,
            "--only" => args.only = Some(value("--only")),
            other => panic!("unknown argument {other}"),
        }
    }
    assert!(
        args.data_dir.join("search-parameters-r4.json").exists(),
        "{} has no search-parameters-r4.json: without the spec search parameters a backend \
         indexes almost nothing and every number below would be meaningless",
        args.data_dir.display()
    );
    args
}

// ───────────────────────────── corpus ─────────────────────────────

/// What the query planner needs to know about the data to pick terminal values
/// at a chosen selectivity, collected while the corpus is built.
#[derive(Default)]
struct Stats {
    /// Lower-cased name parts (family, given) per patient.
    patient_names: Vec<Vec<String>>,
    patient_ids: Vec<String>,
    birthdates: Vec<String>,
    practitioner_names: Vec<Vec<String>>,
    organization_names: Vec<Vec<String>>,
    /// `system|code` of `Observation.code.coding[0]` -> observations.
    observation_codes: HashMap<String, usize>,
    has_general_practitioner: bool,
}

struct Corpus {
    /// NDJSON per resource type, in ingest order.
    files: Vec<(String, Vec<u8>)>,
    counts: BTreeMap<String, usize>,
    stats: Stats,
    description: String,
}

impl Corpus {
    fn push(&mut self, resource_type: &str, lines: Vec<u8>, count: usize) {
        self.counts.insert(resource_type.to_string(), count);
        self.files.push((resource_type.to_string(), lines));
    }
}

fn name_parts(resource: &Value) -> Vec<String> {
    let mut parts = Vec::new();
    match resource.get("name") {
        // `Organization.name` is one string, and a string search matches from
        // its start — not from the start of each word.
        Some(Value::String(s)) => parts.push(s.to_lowercase()),
        Some(Value::Array(names)) => {
            for name in names {
                if let Some(f) = name.get("family").and_then(Value::as_str) {
                    parts.push(f.to_lowercase());
                }
                // Not `prefix` or `suffix`: the `name` parameter does not index them
                // here ("Mrs." matches no patient), so they are no terminal value.
                for g in name
                    .get("given")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if let Some(g) = g.as_str() {
                        parts.push(g.to_lowercase());
                    }
                }
            }
        }
        _ => {}
    }
    parts
}

fn observation_code(resource: &Value) -> Option<String> {
    let coding = resource.pointer("/code/coding/0")?;
    Some(format!(
        "{}|{}",
        coding.get("system")?.as_str()?,
        coding.get("code")?.as_str()?
    ))
}

/// SplitMix64: a seeded generator small enough to keep here, so the synthetic
/// corpus is identical on every machine without a `rand` version to pin.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    /// Skewed towards 0: a few busy organizations and common codes, a long
    /// tail of rare ones — the shape real encounter and lab data has.
    fn skewed(&mut self, n: usize) -> usize {
        let u = self.unit();
        ((u * u * u) * n as f64) as usize % n
    }
}

#[rustfmt::skip]
const FAMILIES: &[&str] = &[
    "Smith", "Johnson", "Williams", "Brown", "Jones", "Garcia", "Miller", "Davis", "Rodriguez",
    "Martinez", "Hernandez", "Lopez", "Gonzalez", "Wilson", "Anderson", "Thomas", "Taylor",
    "Moore", "Jackson", "Martin", "Lee", "Perez", "Thompson", "White", "Harris", "Sanchez",
    "Clark", "Ramirez", "Lewis", "Robinson", "Walker", "Young", "Allen", "King", "Wright",
    "Scott", "Torres", "Nguyen", "Hill", "Flores", "Green", "Adams", "Nelson", "Baker", "Hall",
    "Rivera", "Campbell", "Mitchell", "Carter", "Roberts", "Okafor", "Ivanov", "Yamamoto",
    "Schneider", "Dubois", "Kowalski", "Eriksson", "Quispe", "Underwood", "Vasquez",
];
#[rustfmt::skip]
const GIVENS: &[&str] = &[
    "James", "Mary", "Robert", "Patricia", "John", "Jennifer", "Michael", "Linda", "David",
    "Elizabeth", "William", "Barbara", "Richard", "Susan", "Joseph", "Jessica", "Thomas",
    "Sarah", "Carlos", "Karen", "Daniel", "Lisa", "Matthew", "Nancy", "Anthony", "Betty",
    "Mark", "Sandra", "Felipe", "Ashley", "Hiro", "Ingrid", "Omar", "Priya", "Wei", "Zofia",
];
#[rustfmt::skip]
const ORG_WORDS: &[&str] = &[
    "Mercy", "General", "Riverside", "Summit", "Lakeside", "Harbor", "Valley", "Northside",
    "Cedar", "Pioneer", "Unity", "Beacon", "Evergreen", "Horizon", "Atlas", "Juniper",
];
const ORG_KINDS: &[&str] = &["Hospital", "Clinic", "Medical Center", "Health Partners"];

/// A deterministic corpus with Synthea's reference shape — Observation →
/// Patient and Encounter, Encounter → Patient, Practitioner and Organization —
/// plus what Synthea lacks: `Patient.generalPractitioner` (a Practitioner for
/// nine patients in ten, an Organization for the tenth, so an untyped
/// `general-practitioner` hop is genuinely polymorphic) and
/// `managingOrganization`.
fn generate(patients: usize, seed: u64) -> Corpus {
    let mut rng = Rng(seed);
    let mut corpus = Corpus {
        files: Vec::new(),
        counts: BTreeMap::new(),
        stats: Stats {
            has_general_practitioner: true,
            ..Default::default()
        },
        description: format!("synthetic, seed {seed}"),
    };
    let organizations = (patients / 250).max(4);
    let practitioners = (patients / 50).max(8);
    const ENCOUNTERS: usize = 5;
    const OBS_PER_ENCOUNTER: usize = 4;
    const CODES: usize = 60;

    let mut out = Vec::new();
    for i in 0..organizations {
        let name = format!(
            "{} {} {i}",
            ORG_WORDS[rng.below(ORG_WORDS.len())],
            ORG_KINDS[rng.below(ORG_KINDS.len())]
        );
        let r = json!({"resourceType": "Organization", "id": format!("org-{i:05}"),
            "active": true, "name": name});
        corpus.stats.organization_names.push(name_parts(&r));
        out.extend(serde_json::to_vec(&r).unwrap());
        out.push(b'\n');
    }
    corpus.push("Organization", std::mem::take(&mut out), organizations);

    for i in 0..practitioners {
        let r = json!({"resourceType": "Practitioner", "id": format!("prac-{i:06}"),
            "active": true,
            "name": [{"family": format!("{}{}", FAMILIES[rng.below(FAMILIES.len())], rng.below(1000)),
                      "given": [GIVENS[rng.below(GIVENS.len())]], "prefix": ["Dr."]}]});
        corpus.stats.practitioner_names.push(name_parts(&r));
        out.extend(serde_json::to_vec(&r).unwrap());
        out.push(b'\n');
    }
    corpus.push("Practitioner", std::mem::take(&mut out), practitioners);

    for i in 0..patients {
        let gp = if rng.below(10) == 0 {
            format!("Organization/org-{:05}", rng.below(organizations))
        } else {
            format!("Practitioner/prac-{:06}", rng.below(practitioners))
        };
        let birth = format!(
            "{:04}-{:02}-{:02}",
            1930 + rng.below(95),
            1 + rng.below(12),
            1 + rng.below(28)
        );
        let r = json!({"resourceType": "Patient", "id": format!("pat-{i:07}"),
            "name": [{"use": "official",
                      "family": format!("{}{}", FAMILIES[rng.skewed(FAMILIES.len())], rng.below(1000)),
                      "given": [GIVENS[rng.below(GIVENS.len())]]}],
            "gender": if rng.below(2) == 0 { "female" } else { "male" },
            "birthDate": birth,
            "generalPractitioner": [{"reference": gp}],
            "managingOrganization": {"reference": format!("Organization/org-{:05}", rng.skewed(organizations))}});
        corpus.stats.patient_names.push(name_parts(&r));
        corpus.stats.patient_ids.push(format!("pat-{i:07}"));
        corpus.stats.birthdates.push(birth);
        out.extend(serde_json::to_vec(&r).unwrap());
        out.push(b'\n');
    }
    corpus.push("Patient", std::mem::take(&mut out), patients);

    let mut observations = Vec::new();
    for p in 0..patients {
        for e in 0..ENCOUNTERS {
            let enc = p * ENCOUNTERS + e;
            let day = format!(
                "{:04}-{:02}-{:02}",
                2015 + rng.below(10),
                1 + rng.below(12),
                1 + rng.below(28)
            );
            let r = json!({"resourceType": "Encounter", "id": format!("enc-{enc:08}"),
                "status": "finished",
                "class": {"system": "http://terminology.hl7.org/CodeSystem/v3-ActCode", "code": "AMB"},
                "subject": {"reference": format!("Patient/pat-{p:07}")},
                "participant": [{"individual": {"reference": format!("Practitioner/prac-{:06}", rng.skewed(practitioners))}}],
                "period": {"start": format!("{day}T09:00:00Z"), "end": format!("{day}T09:30:00Z")},
                "serviceProvider": {"reference": format!("Organization/org-{:05}", rng.skewed(organizations))}});
            out.extend(serde_json::to_vec(&r).unwrap());
            out.push(b'\n');
            for o in 0..OBS_PER_ENCOUNTER {
                let code = format!("{}-{}", 1000 + rng.skewed(CODES), 7);
                let r = json!({"resourceType": "Observation",
                    "id": format!("obs-{:09}", enc * OBS_PER_ENCOUNTER + o),
                    "status": "final",
                    "code": {"coding": [{"system": "http://loinc.org", "code": code}]},
                    "subject": {"reference": format!("Patient/pat-{p:07}")},
                    "encounter": {"reference": format!("Encounter/enc-{enc:08}")},
                    "effectiveDateTime": format!("{day}T09:10:00Z"),
                    "valueQuantity": {"value": rng.below(2000) as f64 / 10.0, "unit": "mg/dL",
                                      "system": "http://unitsofmeasure.org", "code": "mg/dL"}});
                *corpus
                    .stats
                    .observation_codes
                    .entry(observation_code(&r).unwrap())
                    .or_default() += 1;
                observations.extend(serde_json::to_vec(&r).unwrap());
                observations.push(b'\n');
            }
        }
    }
    corpus.push("Encounter", out, patients * ENCOUNTERS);
    corpus.push(
        "Observation",
        observations,
        patients * ENCOUNTERS * OBS_PER_ENCOUNTER,
    );
    corpus
}

fn find_file(dir: &Path, resource_type: &str) -> PathBuf {
    let exact = dir.join(format!("{resource_type}.ndjson"));
    if exact.exists() {
        return exact;
    }
    // Synthea names the shared files `Organization.<timestamp>.ndjson`.
    let prefix = format!("{resource_type}.");
    std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(&prefix) && n.ends_with(".ndjson"))
        })
        .unwrap_or_else(|| panic!("no {resource_type}*.ndjson in {}", dir.display()))
}

fn lines(path: &Path) -> impl Iterator<Item = String> {
    let file = std::fs::File::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    BufReader::with_capacity(1 << 20, file)
        .lines()
        .map(|l| l.expect("read line"))
        .filter(|l| !l.trim().is_empty())
}

/// True if `line` references one of `patients` as `Patient/<id>`. A substring
/// scan, so a 7 GB Observation file is filtered without parsing the lines that
/// are dropped.
fn references_patient(line: &str, patients: &HashSet<String>) -> bool {
    let mut rest = line;
    while let Some(at) = rest.find("Patient/") {
        let tail = &rest[at + 8..];
        let end = tail.find('"').unwrap_or(tail.len());
        if patients.contains(&tail[..end]) {
            return true;
        }
        rest = tail;
    }
    false
}

/// The first `patients` patients of a Synthea bulk export and every Encounter
/// and Observation of theirs, with all Organizations and Practitioners.
///
/// Synthea writes Encounter → Practitioner / Organization as conditional
/// references (`Practitioner?identifier=system|value`), which no search index
/// resolves; they are rewritten to the literal reference they denote so the
/// 3-hop chains have something to follow. Nothing else is changed.
fn load_synthea(dir: &Path, patients: usize) -> Corpus {
    let mut corpus = Corpus {
        files: Vec::new(),
        counts: BTreeMap::new(),
        stats: Stats::default(),
        description: format!("Synthea export {}", dir.display()),
    };
    let mut by_identifier: HashMap<String, String> = HashMap::new();
    // Name parts by reference. Only the organizations and practitioners the
    // selected patients' encounters point at go into the statistics: a terminal
    // value picked from the other thousand starts a chain that leads nowhere.
    let mut names: HashMap<String, Vec<String>> = HashMap::new();
    let mut referenced: HashSet<String> = HashSet::new();
    for resource_type in ["Organization", "Practitioner"] {
        let mut out = Vec::new();
        let mut count = 0;
        for line in lines(&find_file(dir, resource_type)) {
            let r: Value = serde_json::from_str(&line).expect("parse");
            let id = r["id"].as_str().expect("id").to_string();
            for ident in r["identifier"].as_array().into_iter().flatten() {
                if let (Some(s), Some(v)) = (ident["system"].as_str(), ident["value"].as_str()) {
                    by_identifier.insert(
                        format!("{resource_type}?identifier={s}|{v}"),
                        format!("{resource_type}/{id}"),
                    );
                }
            }
            names.insert(format!("{resource_type}/{id}"), name_parts(&r));
            out.extend(line.as_bytes());
            out.push(b'\n');
            count += 1;
        }
        corpus.push(resource_type, out, count);
    }

    let mut ids = HashSet::new();
    let mut out = Vec::new();
    for line in lines(&find_file(dir, "Patient")).take(patients) {
        let r: Value = serde_json::from_str(&line).expect("parse");
        let id = r["id"].as_str().expect("id").to_string();
        corpus.stats.patient_names.push(name_parts(&r));
        corpus
            .stats
            .birthdates
            .push(r["birthDate"].as_str().unwrap_or("1900-01-01").to_string());
        corpus.stats.patient_ids.push(id.clone());
        ids.insert(id);
        out.extend(line.as_bytes());
        out.push(b'\n');
    }
    corpus.push("Patient", out, ids.len());

    let mut out = Vec::new();
    let mut count = 0;
    let mut unresolved = 0usize;
    for line in lines(&find_file(dir, "Encounter")) {
        if !references_patient(&line, &ids) {
            continue;
        }
        let mut r: Value = serde_json::from_str(&line).expect("parse");
        let mut fix = |reference: Option<&mut Value>| {
            if let Some(v) = reference {
                if let Some(literal) = v.as_str().and_then(|s| by_identifier.get(s)) {
                    referenced.insert(literal.clone());
                    *v = Value::String(literal.clone());
                } else if v.as_str().is_some_and(|s| s.contains('?')) {
                    unresolved += 1;
                }
            }
        };
        fix(r.pointer_mut("/serviceProvider/reference"));
        if let Some(participants) = r.get_mut("participant").and_then(Value::as_array_mut) {
            for p in participants {
                fix(p.pointer_mut("/individual/reference"));
            }
        }
        out.extend(serde_json::to_vec(&r).unwrap());
        out.push(b'\n');
        count += 1;
    }
    assert_eq!(unresolved, 0, "conditional references left unresolved");
    let mut referenced: Vec<&String> = referenced.iter().collect();
    referenced.sort();
    for reference in referenced {
        let parts = names.get(reference).cloned().unwrap_or_default();
        if reference.starts_with("Organization/") {
            corpus.stats.organization_names.push(parts);
        } else {
            corpus.stats.practitioner_names.push(parts);
        }
    }
    corpus.push("Encounter", out, count);

    let mut out = Vec::new();
    let mut count = 0;
    for line in lines(&find_file(dir, "Observation")) {
        if !references_patient(&line, &ids) {
            continue;
        }
        let r: Value = serde_json::from_str(&line).expect("parse");
        if let Some(code) = observation_code(&r) {
            *corpus.stats.observation_codes.entry(code).or_default() += 1;
        }
        out.extend(line.as_bytes());
        out.push(b'\n');
        count += 1;
    }
    corpus.push("Observation", out, count);
    corpus
}

// ───────────────────────────── query plan ─────────────────────────────

#[derive(Clone)]
enum Spec {
    /// `base?chain=value`, the chain as the client writes it.
    Forward {
        chain: String,
        value: String,
    },
    Has(ReverseChainedParameter),
}

#[derive(Clone)]
struct Case {
    family: &'static str,
    selectivity: &'static str,
    base: &'static str,
    spec: Spec,
    /// The terminal search on its own — `(type, param, value)` — to report how
    /// many resources the chain starts from.
    terminal: Option<(&'static str, &'static str, String)>,
    explain: bool,
}

impl Case {
    fn label(&self) -> String {
        match &self.spec {
            Spec::Forward { chain, value } => format!("{}?{chain}={value}", self.base),
            Spec::Has(rc) => format!("{}?{}", self.base, has_label(rc)),
        }
    }
}

fn has_label(rc: &ReverseChainedParameter) -> String {
    match &rc.nested {
        Some(inner) => format!(
            "_has:{}:{}:{}",
            rc.source_type,
            rc.reference_param,
            has_label(inner)
        ),
        None => format!(
            "_has:{}:{}:{}={}",
            rc.source_type,
            rc.reference_param,
            rc.search_param,
            rc.value.as_ref().map_or("", |v| v.value.as_str())
        ),
    }
}

/// The string-search prefix (1–4 characters, or a whole name part) matched by
/// the number of `names` closest to `target`, with that number.
fn prefix_matching(names: &[Vec<String>], target: usize) -> (String, usize) {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for parts in names {
        let mut seen: HashSet<&str> = HashSet::new();
        for part in parts {
            // Only ASCII name parts, so a byte slice is a character slice;
            // and no comma, which would make the value an OR list.
            if !part.is_ascii() || part.contains(',') {
                continue;
            }
            for len in (1..=part.len().min(4)).chain([part.len()]) {
                seen.insert(&part[..len]);
            }
        }
        for prefix in seen {
            *counts.entry(prefix).or_default() += 1;
        }
    }
    let mut best: Vec<(&str, usize)> = counts.into_iter().collect();
    // Deterministic across runs: closest count, then longest, then alphabetical.
    best.sort_by(|a, b| {
        (a.1.abs_diff(target), std::cmp::Reverse(a.0.len()), a.0).cmp(&(
            b.1.abs_diff(target),
            std::cmp::Reverse(b.0.len()),
            b.0,
        ))
    });
    let (prefix, count) = best.first().expect("corpus has names");
    (prefix.to_string(), *count)
}

/// Three terminal values per family — matching one resource, about 1 % and
/// about 30 % — picked from the corpus rather than hard-coded, so the same
/// plan works for any size and for either corpus.
fn plan(corpus: &Corpus) -> Vec<Case> {
    let stats = &corpus.stats;
    let levels = |n: usize| {
        [
            ("low", 1),
            ("medium", (n / 100).max(2)),
            ("high", n * 3 / 10),
        ]
    };
    let mut cases = Vec::new();

    let patients = stats.patient_names.len();
    for (selectivity, target) in levels(patients) {
        let (prefix, _) = prefix_matching(&stats.patient_names, target);
        cases.push(Case {
            family: "2-hop name",
            selectivity,
            base: "Observation",
            spec: Spec::Forward {
                chain: "subject:Patient.name".into(),
                value: prefix.clone(),
            },
            terminal: Some(("Patient", "name", prefix.clone())),
            explain: false,
        });
        // The same search as clients usually write it: no `:Patient`. `subject`
        // is polymorphic (Patient, Group, Device, Location), so this is the
        // untyped-hop case.
        if selectivity != "high" {
            cases.push(Case {
                family: "2-hop untyped",
                selectivity,
                base: "Observation",
                spec: Spec::Forward {
                    chain: "subject.name".into(),
                    value: prefix.clone(),
                },
                terminal: Some(("Patient", "name", prefix)),
                explain: false,
            });
        }
    }

    // A comma is OR: either of the low- and the medium-selectivity name.
    let (low, _) = prefix_matching(&stats.patient_names, 1);
    let (medium, _) = prefix_matching(&stats.patient_names, (patients / 100).max(2));
    cases.push(Case {
        family: "2-hop name OR list",
        selectivity: "probe",
        base: "Observation",
        spec: Spec::Forward {
            chain: "subject:Patient.name".into(),
            value: format!("{low},{medium}"),
        },
        terminal: Some(("Patient", "name", format!("{low},{medium}"))),
        explain: false,
    });

    // FHIR string search is starts-with. "son" ends family names (Johnson,
    // Wilson, Anderson) far more often than it starts a name part, so a path
    // that matches it as a substring answers a different question.
    cases.push(Case {
        family: "2-hop name infix",
        selectivity: "probe",
        base: "Observation",
        spec: Spec::Forward {
            chain: "subject:Patient.name".into(),
            value: "son".into(),
        },
        terminal: Some(("Patient", "name", "son".into())),
        explain: false,
    });

    let mut births = stats.birthdates.clone();
    births.sort();
    for (selectivity, target) in levels(patients) {
        let cutoff = births[births.len() - target.clamp(1, births.len())].clone();
        cases.push(Case {
            family: "2-hop birthdate",
            selectivity,
            base: "Observation",
            spec: Spec::Forward {
                chain: "subject:Patient.birthdate".into(),
                value: format!("ge{cutoff}"),
            },
            terminal: Some(("Patient", "birthdate", format!("ge{cutoff}"))),
            explain: false,
        });
    }

    for (selectivity, target) in levels(stats.organization_names.len()) {
        let (prefix, _) = prefix_matching(&stats.organization_names, target);
        cases.push(Case {
            family: "3-hop organization",
            selectivity,
            base: "Observation",
            spec: Spec::Forward {
                chain: "encounter:Encounter.service-provider:Organization.name".into(),
                value: prefix.clone(),
            },
            terminal: Some(("Organization", "name", prefix)),
            explain: selectivity == "medium",
        });
    }
    for (selectivity, target) in levels(stats.practitioner_names.len()) {
        let (prefix, _) = prefix_matching(&stats.practitioner_names, target);
        cases.push(Case {
            family: "3-hop practitioner",
            selectivity,
            base: "Observation",
            spec: Spec::Forward {
                chain: "encounter:Encounter.practitioner:Practitioner.name".into(),
                value: prefix.clone(),
            },
            terminal: Some(("Practitioner", "name", prefix.clone())),
            explain: false,
        });
        if stats.has_general_practitioner {
            cases.push(Case {
                family: "3-hop GP",
                selectivity,
                base: "Observation",
                spec: Spec::Forward {
                    chain: "subject:Patient.general-practitioner:Practitioner.name".into(),
                    value: prefix.clone(),
                },
                terminal: Some(("Practitioner", "name", prefix.clone())),
                explain: false,
            });
        }
    }
    if stats.has_general_practitioner {
        // A polymorphic hop whose targets are both present in the data: one
        // patient in ten has an Organization as general practitioner. "M"
        // starts organization names ("Mercy …") and practitioner names alike.
        cases.push(Case {
            family: "polymorphic GP",
            selectivity: "medium",
            base: "Patient",
            spec: Spec::Forward {
                chain: "general-practitioner.name".into(),
                value: "M".into(),
            },
            terminal: None,
            explain: false,
        });
    }

    let mut codes: Vec<(&String, &usize)> = stats.observation_codes.iter().collect();
    codes.sort_by(|a, b| (a.1, a.0).cmp(&(b.1, b.0)));
    let observations: usize = codes.iter().map(|c| c.1).sum();
    let closest = |target: usize| {
        codes
            .iter()
            .min_by_key(|c| (c.1.abs_diff(target), c.0.as_str()))
            .map(|c| c.0.clone())
            .expect("corpus has observation codes")
    };
    for (selectivity, code) in [
        ("low", codes[0].0.clone()),
        ("medium", closest(observations / 100)),
        ("high", codes[codes.len() - 1].0.clone()),
    ] {
        cases.push(Case {
            family: "_has",
            selectivity,
            base: "Patient",
            spec: Spec::Has(ReverseChainedParameter::terminal(
                "Observation",
                "subject",
                "code",
                SearchValue::eq(code.clone()),
            )),
            terminal: Some(("Observation", "code", code.clone())),
            explain: selectivity == "medium",
        });
        cases.push(Case {
            family: "nested _has",
            selectivity,
            base: "Organization",
            spec: Spec::Has(ReverseChainedParameter::nested(
                "Encounter",
                "service-provider",
                ReverseChainedParameter::terminal(
                    "Observation",
                    "encounter",
                    "code",
                    SearchValue::eq(code.clone()),
                ),
            )),
            terminal: Some(("Observation", "code", code)),
            explain: selectivity == "medium",
        });
    }
    // #1389 item 1: `system|` on a chained token terminal — "any code in this
    // system". A correctness probe; it selects every observation, so it is the
    // widest `_has` in the plan as well.
    //
    // Only on a corpus of up to 150,000 observations: the resolver fetches every
    // one of them, a page of 1,000 at a time, and on 545,000 that single probe
    // was still running after twenty minutes.
    let system = codes[0].0.split('|').next().unwrap_or_default().to_string();
    if observations <= 150_000 {
        cases.push(Case {
            family: "_has system|",
            selectivity: "all",
            base: "Patient",
            spec: Spec::Has(ReverseChainedParameter::terminal(
                "Observation",
                "subject",
                "code",
                SearchValue::eq(format!("{system}|")),
            )),
            terminal: Some(("Observation", "code", format!("{system}|"))),
            explain: false,
        });
    }
    cases
}

/// A chain as the REST layer parses it (`parse_chain_in` in
/// `helios-rest/src/extractors/search_query_builder.rs`): the base reference
/// parameter, then one hop per `.`, a hop's `:Type` naming the type the
/// *previous* reference is followed to.
fn forward_parameter(
    registry: &SearchParameterRegistry,
    base: &str,
    chain: &str,
    value: &str,
) -> SearchParameter {
    let split = |part: &str| match part.split_once(':') {
        Some((p, q)) => (p.to_string(), Some(q.to_string())),
        None => (part.to_string(), None),
    };
    let mut parts = chain.split('.');
    let (base_name, mut qualifier) = split(parts.next().expect("chain"));
    let mut reference_param = base_name.clone();
    let mut hops = Vec::new();
    for part in parts {
        let (target_param, next_qualifier) = split(part);
        hops.push(ChainedParameter {
            reference_param: reference_param.clone(),
            target_type: qualifier.take(),
            target_param: target_param.clone(),
        });
        reference_param = target_param;
        qualifier = next_qualifier;
    }
    let raw: Vec<String> = value.split(',').map(str::to_string).collect();
    let (param_type, values) = parse_typed_values(registry, base, &base_name, &raw);
    SearchParameter {
        name: base_name,
        param_type,
        modifier: None,
        values,
        chain: hops,
        components: vec![],
    }
}

// ───────────────────────────── instrumentation ─────────────────────────────

/// Counts what the shared resolver asks of a backend: searches issued, rows
/// returned, and payload bytes. Full searches count JSON content; id-only
/// searches count the ids returned by the SQL projection.
struct Counting<S> {
    inner: Arc<S>,
    searches: AtomicU64,
    rows: AtomicU64,
    bytes: AtomicU64,
}

impl<S> Counting<S> {
    fn new(inner: Arc<S>) -> Self {
        Self {
            inner,
            searches: AtomicU64::new(0),
            rows: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
        }
    }
}

#[async_trait]
impl<S: ResourceStorage> ResourceStorage for Counting<S> {
    fn backend_name(&self) -> &'static str {
        self.inner.backend_name()
    }
    async fn create(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        fhir_version: FhirVersion,
    ) -> StorageResult<StoredResource> {
        self.inner
            .create(tenant, resource_type, resource, fhir_version)
            .await
    }
    async fn create_or_update(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        resource: Value,
        fhir_version: FhirVersion,
    ) -> StorageResult<(StoredResource, bool)> {
        self.inner
            .create_or_update(tenant, resource_type, id, resource, fhir_version)
            .await
    }
    async fn read(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<Option<StoredResource>> {
        self.inner.read(tenant, resource_type, id).await
    }
    async fn update(
        &self,
        tenant: &TenantContext,
        current: &StoredResource,
        resource: Value,
    ) -> StorageResult<StoredResource> {
        self.inner.update(tenant, current, resource).await
    }
    async fn delete(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<()> {
        self.inner.delete(tenant, resource_type, id).await
    }
    async fn count(
        &self,
        tenant: &TenantContext,
        resource_type: Option<&str>,
    ) -> StorageResult<u64> {
        self.inner.count(tenant, resource_type).await
    }
}

#[async_trait]
impl<S: SearchProvider> SearchProvider for Counting<S> {
    async fn search(
        &self,
        tenant: &TenantContext,
        query: &SearchQuery,
    ) -> StorageResult<SearchResult> {
        let result = self.inner.search(tenant, query).await?;
        self.searches.fetch_add(1, Ordering::Relaxed);
        self.rows
            .fetch_add(result.resources.items.len() as u64, Ordering::Relaxed);
        let bytes: usize = result
            .resources
            .items
            .iter()
            .map(|r| serde_json::to_vec(r.content()).map_or(0, |v| v.len()))
            .sum();
        self.bytes.fetch_add(bytes as u64, Ordering::Relaxed);
        Ok(result)
    }
    async fn search_ids(
        &self,
        tenant: &TenantContext,
        query: &SearchQuery,
    ) -> StorageResult<Page<String>> {
        let result = self.inner.search_ids(tenant, query).await?;
        self.searches.fetch_add(1, Ordering::Relaxed);
        self.rows
            .fetch_add(result.items.len() as u64, Ordering::Relaxed);
        let bytes: usize = result.items.iter().map(String::len).sum();
        self.bytes.fetch_add(bytes as u64, Ordering::Relaxed);
        Ok(result)
    }
    async fn search_count(
        &self,
        tenant: &TenantContext,
        query: &SearchQuery,
    ) -> StorageResult<u64> {
        self.inner.search_count(tenant, query).await
    }
    fn search_param_registry(
        &self,
        tenant: &TenantContext,
    ) -> Arc<RwLock<SearchParameterRegistry>> {
        self.inner.search_param_registry(tenant)
    }
}

// ───────────────────────────── measurement ─────────────────────────────

struct Timing {
    n: usize,
    min: Duration,
    median: Duration,
    p95: Duration,
}

fn summarize(mut samples: Vec<Duration>) -> Timing {
    samples.sort();
    let n = samples.len();
    // Nearest-rank percentile.
    let rank = |p: f64| samples[((p * n as f64).ceil() as usize).clamp(1, n) - 1];
    Timing {
        n,
        min: samples[0],
        median: rank(0.5),
        p95: rank(0.95),
    }
}

fn ms(d: Duration) -> String {
    format!("{:.2}", d.as_secs_f64() * 1e3)
}

/// The ids a resolved query's `_id` filter carries; empty for the resolver's
/// "matches nothing" sentinel.
fn resolved_ids(query: &SearchQuery) -> Vec<String> {
    query
        .parameters
        .iter()
        .filter(|p| p.name == "_id")
        .flat_map(|p| p.values.iter().map(|v| v.value.clone()))
        .filter(|id| id != "__chained_search_no_match__")
        .collect()
}

fn chained_query<S: SearchProvider>(
    backend: &S,
    tenant: &TenantContext,
    case: &Case,
) -> SearchQuery {
    match &case.spec {
        Spec::Forward { chain, value } => {
            let reg = backend.search_param_registry(tenant);
            let registry = reg.read();
            SearchQuery::new(case.base)
                .with_parameter(forward_parameter(&registry, case.base, chain, value))
        }
        Spec::Has(rc) => {
            let mut query = SearchQuery::new(case.base);
            query.reverse_chains.push(rc.clone());
            query
        }
    }
}

async fn native<S: ChainedSearchProvider>(
    backend: &S,
    tenant: &TenantContext,
    case: &Case,
) -> StorageResult<Vec<String>> {
    match &case.spec {
        Spec::Forward { chain, value } => {
            backend.resolve_chain(tenant, case.base, chain, value).await
        }
        Spec::Has(rc) => backend.resolve_reverse_chain(tenant, case.base, rc).await,
    }
}

/// Times `run` — `warmup` untimed runs, then `iters` timed ones — dropping to
/// one warm-up and five iterations when a single run exceeds `slow`.
async fn time<F, Fut>(args: &Args, first: Duration, mut run: F) -> Timing
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    // Past half a minute a run is not repeated: the one instrumented pass is
    // the sample (`n=1`). The resolver pages with OFFSET, so a wide hop costs
    // quadratically more, and five more runs of a ten-minute resolution would
    // say nothing the first did not.
    if first > Duration::from_secs(30) {
        return Timing {
            n: 1,
            min: first,
            median: first,
            p95: first,
        };
    }
    let slow = first > Duration::from_millis(args.slow_ms);
    let (warmup, iters) = if slow {
        (1, args.iters.min(5))
    } else {
        (args.warmup, args.iters)
    };
    for _ in 0..warmup {
        run().await;
    }
    let mut samples = Vec::with_capacity(iters);
    for _ in 0..iters {
        let started = Instant::now();
        run().await;
        samples.push(started.elapsed());
    }
    summarize(samples)
}

async fn ingest<S>(backend: &S, tenant: &TenantContext, corpus: &Corpus)
where
    S: BulkSubmitProvider + StreamingBulkSubmitProvider,
{
    let submission = SubmissionId::generate("chain-bench");
    backend
        .create_submission(tenant, &submission, None)
        .await
        .expect("create submission");
    let manifest = backend
        .add_manifest(tenant, &submission, None, None)
        .await
        .expect("add manifest");
    let mut options = BulkProcessingOptions::new();
    options.batch_size = 1000;
    let started = Instant::now();
    let mut total = 0usize;
    for (resource_type, bytes) in &corpus.files {
        let options = options
            .clone()
            .with_file_url(format!("bench://{resource_type}"));
        let reader: Box<dyn tokio::io::AsyncBufRead + Send + Unpin> = Box::new(
            tokio::io::BufReader::new(std::io::Cursor::new(bytes.clone())),
        );
        let result = backend
            .process_ndjson_stream(
                tenant,
                &submission,
                &manifest.manifest_id,
                resource_type,
                reader,
                &options,
            )
            .await
            .expect("ingest");
        assert_eq!(
            result.counts.error_count(),
            0,
            "{resource_type}: entries failed to ingest"
        );
        total += corpus.counts[resource_type];
    }
    let wall = started.elapsed();
    println!(
        "ingested {total} resources in {:.1}s ({:.0}/s)",
        wall.as_secs_f64(),
        total as f64 / wall.as_secs_f64()
    );
}

async fn try_count<S: SearchProvider>(
    backend: &S,
    tenant: &TenantContext,
    resource_type: &str,
    param: &str,
    value: &str,
) -> StorageResult<u64> {
    let raw: Vec<String> = value.split(',').map(str::to_string).collect();
    let parameter = {
        let reg = backend.search_param_registry(tenant);
        let registry = reg.read();
        let (param_type, values) = parse_typed_values(&registry, resource_type, param, &raw);
        SearchParameter {
            name: param.to_string(),
            param_type,
            modifier: None,
            values,
            chain: vec![],
            components: vec![],
        }
    };
    backend
        .search_count(
            tenant,
            &SearchQuery::new(resource_type).with_parameter(parameter),
        )
        .await
}

async fn count<S: SearchProvider>(
    backend: &S,
    tenant: &TenantContext,
    resource_type: &str,
    param: &str,
    value: &str,
) -> u64 {
    try_count(backend, tenant, resource_type, param, value)
        .await
        .unwrap_or_else(|e| panic!("count {resource_type}?{param}={value}: {e}"))
}

/// The data-dir trap: a backend built without the spec search parameters
/// ingests happily and indexes nothing, and every chain then "agrees" on the
/// empty set. Each reference and terminal parameter the plan uses must find
/// what the corpus says is there before anything is measured.
async fn assert_indexed<S: SearchProvider>(backend: &S, tenant: &TenantContext, corpus: &Corpus) {
    for (resource_type, expected) in &corpus.counts {
        let got = backend
            .count(tenant, Some(resource_type))
            .await
            .expect("count");
        assert_eq!(got, *expected as u64, "{resource_type} resources stored");
    }
    let patient = format!("Patient/{}", corpus.stats.patient_ids[0]);
    for (resource_type, param) in [("Observation", "subject"), ("Encounter", "subject")] {
        let got = count(backend, tenant, resource_type, param, &patient).await;
        assert!(got > 0, "{resource_type}?{param}={patient} found nothing");
    }
    let (code, expected) = corpus
        .stats
        .observation_codes
        .iter()
        .max_by_key(|c| (c.1, c.0))
        .expect("codes");
    let got = count(backend, tenant, "Observation", "code", code).await;
    assert_eq!(got, *expected as u64, "Observation?code={code}");
    let mut births = corpus.stats.birthdates.clone();
    births.sort();
    let median = &births[births.len() / 2];
    let expected = births.iter().filter(|b| *b >= median).count() as u64;
    let got = count(
        backend,
        tenant,
        "Patient",
        "birthdate",
        &format!("ge{median}"),
    )
    .await;
    assert_eq!(got, expected, "Patient?birthdate=ge{median}");
    println!("index positive controls passed (stored counts, subject, code, birthdate)");
}

#[derive(Default)]
struct Summary {
    agreed: usize,
    disagreed: Vec<String>,
}

async fn run_cases<S>(
    label: &str,
    backend: Arc<S>,
    tenant: &TenantContext,
    cases: &[Case],
    args: &Args,
    size: usize,
) -> Summary
where
    S: SearchProvider + ChainedSearchProvider,
{
    let mut summary = Summary::default();
    println!();
    println!(
        "| query | sel. | terminal | ids | shared med (min / p95) ms | native med (min / p95) ms | \
         shared/native | searches | rows | MB | page ms |"
    );
    println!("|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
    for case in cases {
        let name = case.label();
        let terminal = match &case.terminal {
            Some((t, p, v)) => try_count(&*backend, tenant, t, p, v)
                .await
                .map_or_else(|_| "error".to_string(), |n| n.to_string()),
            None => "-".to_string(),
        };
        let query = chained_query(&*backend, tenant, case);

        // One instrumented pass per path: the answer, the resolver's traffic,
        // and a first duration that decides how many iterations are affordable.
        let counting = Counting::new(backend.clone());
        let started = Instant::now();
        let shared = resolve_chains(&counting, tenant, &query).await;
        let shared_first = started.elapsed();
        let started = Instant::now();
        let native_ids = native(&*backend, tenant, case).await;
        let native_first = started.elapsed();

        let (rewritten, shared_ids, native_ids) = match (shared, native_ids) {
            (Ok(q), Ok(n)) => {
                let ids = resolved_ids(&q);
                (q, ids, n)
            }
            (shared, native_ids) => {
                let text = format!(
                    "{name}: shared = {}, native = {}",
                    shared.map_or_else(
                        |e| format!("ERROR {e}"),
                        |q| format!("{} ids", resolved_ids(&q).len())
                    ),
                    native_ids
                        .map_or_else(|e| format!("ERROR {e}"), |n| format!("{} ids", n.len()))
                );
                println!(
                    "| `{name}` | {} | {terminal} | ERROR — see below |",
                    case.selectivity
                );
                summary.disagreed.push(text);
                continue;
            }
        };
        let shared_set: HashSet<&String> = shared_ids.iter().collect();
        let native_set: HashSet<&String> = native_ids.iter().collect();
        let agree = shared_set == native_set;
        if agree {
            summary.agreed += 1;
        } else {
            let sample = |a: &HashSet<&String>, b: &HashSet<&String>| {
                let mut only: Vec<&String> = a.difference(b).copied().collect();
                only.sort();
                only.truncate(3);
                only.into_iter().cloned().collect::<Vec<_>>().join(", ")
            };
            summary.disagreed.push(format!(
                "{name}: shared {} ids, native {} ids ({} rows); only shared: {} [{}]; only native: {} [{}]",
                shared_set.len(),
                native_set.len(),
                native_ids.len(),
                shared_set.difference(&native_set).count(),
                sample(&shared_set, &native_set),
                native_set.difference(&shared_set).count(),
                sample(&native_set, &shared_set),
            ));
            if !args.time_disagreements {
                println!(
                    "| `{name}` | {} | {terminal} | shared {} ≠ native {} | not timed | not timed | - | {} | {} | {:.1} | - |",
                    case.selectivity,
                    shared_set.len(),
                    native_set.len(),
                    counting.searches.load(Ordering::Relaxed),
                    counting.rows.load(Ordering::Relaxed),
                    counting.bytes.load(Ordering::Relaxed) as f64 / 1e6,
                );
                continue;
            }
        }

        let shared_t = time(args, shared_first, || async {
            resolve_chains(&*backend, tenant, &query)
                .await
                .expect("shared");
        })
        .await;
        let native_t = time(args, native_first, || async {
            native(&*backend, tenant, case).await.expect("native");
        })
        .await;

        // What REST does next with the resolver's answer: the first page of the
        // rewritten query, whose `_id` filter carries every matched id.
        let page_query = rewritten.clone().with_count(20);
        let page = {
            let mut samples = Vec::new();
            let mut error = None;
            for _ in 0..5 {
                let started = Instant::now();
                match backend.search(tenant, &page_query).await {
                    Ok(_) => samples.push(started.elapsed()),
                    Err(e) => {
                        error = Some(e.to_string());
                        break;
                    }
                }
            }
            match error {
                Some(e) => {
                    summary.disagreed.push(format!(
                        "{name}: first page of the rewritten query ({} ids in `_id`) FAILED: {e}",
                        shared_ids.len()
                    ));
                    "FAILED".to_string()
                }
                None => ms(summarize(samples).median),
            }
        };

        let ratio = shared_t.median.as_secs_f64() / native_t.median.as_secs_f64().max(1e-9);
        println!(
            "| `{name}` | {} | {terminal} | {}{} | {} ({} / {}){} | {} ({} / {}){} | {ratio:.1}x | {} | {} | {:.1} | {page} |",
            case.selectivity,
            shared_set.len(),
            if agree { "" } else { " ≠" },
            ms(shared_t.median),
            ms(shared_t.min),
            ms(shared_t.p95),
            if shared_t.n < args.iters {
                format!(" n={}", shared_t.n)
            } else {
                String::new()
            },
            ms(native_t.median),
            ms(native_t.min),
            ms(native_t.p95),
            if native_t.n < args.iters {
                format!(" n={}", native_t.n)
            } else {
                String::new()
            },
            counting.searches.load(Ordering::Relaxed),
            counting.rows.load(Ordering::Relaxed),
            counting.bytes.load(Ordering::Relaxed) as f64 / 1e6,
        );
        println!(
            "RESULT\t{label}\t{size}\t{}\t{}\t{name}\t{}\t{}\t{:.3}\t{:.3}\t{:.3}\t{}\t{:.3}\t{:.3}\t{:.3}\t{}\t{}\t{}\t{}\t{page}",
            case.family,
            case.selectivity,
            agree,
            shared_set.len(),
            shared_t.median.as_secs_f64() * 1e3,
            shared_t.min.as_secs_f64() * 1e3,
            shared_t.p95.as_secs_f64() * 1e3,
            shared_t.n,
            native_t.median.as_secs_f64() * 1e3,
            native_t.min.as_secs_f64() * 1e3,
            native_t.p95.as_secs_f64() * 1e3,
            native_t.n,
            counting.searches.load(Ordering::Relaxed),
            counting.rows.load(Ordering::Relaxed),
            counting.bytes.load(Ordering::Relaxed),
        );
    }
    println!();
    println!(
        "{label}: {} queries agreed, {} findings",
        summary.agreed,
        summary.disagreed.len()
    );
    for d in &summary.disagreed {
        println!("FINDING\t{label}\t{d}");
    }
    summary
}

// ───────────────────────────── EXPLAIN ─────────────────────────────

/// `EXPLAIN QUERY PLAN` of the statement the SQLite `resolve_chain` /
/// `resolve_reverse_chain` runs, rebuilt here from the same public builder.
fn explain_sqlite(db: &Path, backend: &SqliteBackend, tenant: &TenantContext, case: &Case) {
    use helios_persistence::backends::sqlite::search::{ChainQueryBuilder, SqlParam};
    let tenant_id = tenant.tenant_id().as_str();
    let builder =
        ChainQueryBuilder::new(tenant_id, case.base, backend.search_param_registry(tenant))
            .with_param_offset(2);
    let fragment = match &case.spec {
        Spec::Forward { chain, value } => {
            let parsed = builder.parse_chain(chain).expect("parse chain");
            let value = SearchValue::parse_for_type(value, parsed.terminal_type);
            builder.build_forward_chain_sql(&parsed, &value)
        }
        Spec::Has(rc) => builder.build_reverse_chain_sql(rc),
    }
    .expect("build chain SQL");
    let sql = format!(
        "SELECT DISTINCT r.id FROM resources r \
         WHERE r.tenant_id = ?1 AND r.resource_type = ?2 AND r.is_deleted = 0 AND {}",
        fragment.sql
    );
    let mut bound: Vec<Box<dyn rusqlite::ToSql>> = vec![
        Box::new(tenant_id.to_string()),
        Box::new(case.base.to_string()),
    ];
    for p in &fragment.params {
        match p {
            SqlParam::String(s) => bound.push(Box::new(s.clone())),
            SqlParam::Integer(i) => bound.push(Box::new(*i)),
            SqlParam::Float(f) => bound.push(Box::new(*f)),
            SqlParam::Null => bound.push(Box::new(rusqlite::types::Null)),
        }
    }
    let refs: Vec<&dyn rusqlite::ToSql> = bound.iter().map(|p| p.as_ref()).collect();
    let conn = rusqlite::Connection::open(db).expect("open db");
    let mut stmt = conn
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .expect("prepare explain");
    let rows = stmt
        .query_map(refs.as_slice(), |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(3)?,
            ))
        })
        .expect("explain");
    println!(
        "\n#### SQLite `{}`\n\n```sql\n{sql}\n```\n\n```text",
        case.label()
    );
    for row in rows {
        let (id, parent, detail) = row.expect("row");
        println!("{id:>4} {parent:>4}  {detail}");
    }
    println!("```");
}

async fn explain_postgres(backend: &PostgresBackend, tenant: &TenantContext, case: &Case) {
    use helios_persistence::backends::postgres::search::chain_builder::ChainQueryBuilder;
    use helios_persistence::backends::postgres::search::query_builder::SqlParam;
    use tokio_postgres::types::ToSql;
    let tenant_id = tenant.tenant_id().as_str();
    let builder =
        ChainQueryBuilder::new(tenant_id, case.base, backend.search_param_registry(tenant))
            .with_param_offset(1);
    let fragment = match &case.spec {
        Spec::Forward { chain, value } => {
            let parsed = builder.parse_chain(chain).expect("parse chain");
            let value = SearchValue::parse_for_type(value, parsed.terminal_type);
            builder.build_forward_chain_sql(&parsed, &value)
        }
        Spec::Has(rc) => builder.build_reverse_chain_sql(rc),
    }
    .expect("build chain SQL");
    let sql = format!(
        "SELECT r.id FROM resources r WHERE r.tenant_id = $1 \
         AND r.resource_type = '{}' AND r.is_deleted = FALSE AND {}",
        case.base, fragment.sql
    );
    let mut params: Vec<Box<dyn ToSql + Sync + Send>> = vec![Box::new(tenant_id.to_string())];
    for p in &fragment.params {
        match p {
            SqlParam::Text(s) => params.push(Box::new(s.clone())),
            SqlParam::TextArray(ids) => params.push(Box::new(ids.clone())),
            SqlParam::Float(f) => params.push(Box::new(*f)),
            SqlParam::Integer(i) => params.push(Box::new(*i)),
            SqlParam::Bool(b) => params.push(Box::new(*b)),
            SqlParam::Timestamp(t) => params.push(Box::new(*t)),
            SqlParam::Null => params.push(Box::new(Option::<String>::None)),
        }
    }
    let refs: Vec<&(dyn ToSql + Sync)> = params
        .iter()
        .map(|p| p.as_ref() as &(dyn ToSql + Sync))
        .collect();
    let client = backend.get_client().await.expect("client");
    let rows = client
        .query(&format!("EXPLAIN (ANALYZE, BUFFERS) {sql}"), &refs)
        .await
        .expect("explain");
    println!(
        "\n#### PostgreSQL `{}`\n\n```sql\n{sql}\n```\n\n```text",
        case.label()
    );
    for row in rows {
        println!("{}", row.get::<_, String>(0));
    }
    println!("```");
}

// ───────────────────────────── main ─────────────────────────────

fn load_average() -> String {
    std::fs::read_to_string("/proc/loadavg")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "n/a".to_string())
}

#[tokio::main]
async fn main() {
    let args = parse_args();
    let started = Instant::now();
    let corpus = match &args.synthea {
        Some(dir) => load_synthea(dir, args.patients),
        None => generate(args.patients, args.seed),
    };
    println!(
        "## chain_bench — {} patients, {}",
        args.patients, corpus.description
    );
    println!();
    println!(
        "corpus built in {:.1}s: {}",
        started.elapsed().as_secs_f64(),
        corpus
            .counts
            .iter()
            .map(|(t, n)| format!("{t} {n}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "iterations {} (warm-up {}), load average at start: {}",
        args.iters,
        args.warmup,
        load_average()
    );

    let mut cases = plan(&corpus);
    if let Some(only) = &args.only {
        cases.retain(|c| c.label().contains(only.as_str()) || c.family.contains(only.as_str()));
    }
    let tenant = TenantContext::new(TenantId::new("bench"), TenantPermissions::full_access());
    let mut findings = 0usize;

    for name in &args.backends {
        println!("\n### {name} — {} patients", args.patients);
        match name.as_str() {
            "sqlite-file" | "sqlite-mem" => {
                let temp = tempfile::tempdir().expect("tempdir");
                let path = if name == "sqlite-mem" {
                    PathBuf::from(":memory:")
                } else {
                    let path = args
                        .db
                        .clone()
                        .unwrap_or_else(|| temp.path().join("chain-bench.db"));
                    for suffix in ["", "-wal", "-shm"] {
                        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
                    }
                    path
                };
                let config = SqliteBackendConfig {
                    data_dir: Some(args.data_dir.clone()),
                    ..Default::default()
                };
                let backend = SqliteBackend::with_config(&path, config).expect("open backend");
                backend.init_schema().expect("init schema");
                let backend = Arc::new(backend);
                ingest(&*backend, &tenant, &corpus).await;
                assert_indexed(&*backend, &tenant, &corpus).await;
                if name == "sqlite-file" {
                    let conn = rusqlite::Connection::open(&path).expect("open db");
                    let rows: i64 = conn
                        .query_row("SELECT COUNT(*) FROM search_index", [], |r| r.get(0))
                        .expect("count index rows");
                    // Statistics for the planner, as an operator would have
                    // after `ANALYZE`; harmless to the resolver's searches.
                    conn.execute_batch("ANALYZE;").expect("analyze");
                    println!(
                        "search_index rows: {rows}; database {:.0} MB",
                        std::fs::metadata(&path).map_or(0.0, |m| m.len() as f64 / 1e6)
                    );
                    assert!(rows > 0, "nothing was indexed");
                }
                let summary =
                    run_cases(name, backend.clone(), &tenant, &cases, &args, args.patients).await;
                findings += summary.disagreed.len();
                if args.explain && name == "sqlite-file" {
                    for case in cases.iter().filter(|c| c.explain) {
                        explain_sqlite(&path, &backend, &tenant, case);
                    }
                }
            }
            "postgres" => {
                use testcontainers::ImageExt;
                use testcontainers::runners::AsyncRunner;
                use testcontainers_modules::postgres::Postgres;
                // Default configuration on purpose: `shared_buffers` 128 MB,
                // `work_mem` 4 MB — what `docker run postgres:16` gives, not a
                // tuned server.
                let container = Postgres::default()
                    .with_tag("16")
                    .start()
                    .await
                    .expect("start postgres:16");
                let config = PostgresConfig {
                    host: container.get_host().await.expect("host").to_string(),
                    port: container.get_host_port_ipv4(5432).await.expect("port"),
                    dbname: "postgres".to_string(),
                    user: "postgres".to_string(),
                    password: Some("postgres".to_string()),
                    max_connections: 5,
                    data_dir: Some(args.data_dir.clone()),
                    ..Default::default()
                };
                let backend = PostgresBackend::new(config).await.expect("backend");
                backend.init_schema().await.expect("init schema");
                let backend = Arc::new(backend);
                ingest(&*backend, &tenant, &corpus).await;
                // Planner statistics first. Straight after a bulk load, before
                // autovacuum's analyze has run, PostgreSQL plans the index
                // tables as if they were empty: a plain `Observation?code=`
                // count then runs into the backend's 30 s statement timeout on
                // 10,000 observations. Both paths are measured with statistics.
                backend
                    .get_client()
                    .await
                    .expect("client")
                    .batch_execute("ANALYZE")
                    .await
                    .expect("analyze");
                assert_indexed(&*backend, &tenant, &corpus).await;
                {
                    let client = backend.get_client().await.expect("client");
                    let rows: i64 = client
                        .query_one("SELECT COUNT(*) FROM search_index", &[])
                        .await
                        .expect("count index rows")
                        .get(0);
                    let size: String = client
                        .query_one(
                            "SELECT pg_size_pretty(pg_database_size(current_database()))",
                            &[],
                        )
                        .await
                        .expect("size")
                        .get(0);
                    println!("search_index rows: {rows}; database {size}");
                    assert!(rows > 0, "nothing was indexed");
                }
                let summary =
                    run_cases(name, backend.clone(), &tenant, &cases, &args, args.patients).await;
                findings += summary.disagreed.len();
                if args.explain {
                    for case in cases.iter().filter(|c| c.explain) {
                        explain_postgres(&backend, &tenant, case).await;
                    }
                }
                drop(backend);
                container.rm().await.expect("remove container");
            }
            other => panic!("unknown backend {other}"),
        }
    }
    println!(
        "\nfinished in {:.0}s, load average at end: {}, findings: {findings}",
        started.elapsed().as_secs_f64(),
        load_average()
    );
}
