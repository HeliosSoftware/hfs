//! Tier-1 conformance sweep over the **official FHIR example corpus**.
//!
//! Discussion #215 ("Validation") committed to being green on three
//! independent corpora. Two were built by #232 — the vendored FHIR Schema
//! conformance suite (`tests/fixtures/upstream/`) and the Helios extended
//! fixtures. This file adds the third: *"the spec ships thousands of example
//! resources … we adopt that corpus wholesale."*
//!
//! The corpus is already vendored in this repo, at
//! `crates/fhir/tests/data/json/<VERSION>/` (~2.9k resources per version,
//! 141–157 distinct resource types). Nothing consumed it before this file.
//!
//! # What this asserts
//!
//! Published spec examples are *mostly* valid against the core spec, so this
//! is primarily a **false-positive detector**: the small inline fixtures can
//! only prove we flag what we decided to flag, never that we stay quiet on
//! the ~8.7k resources the ecosystem actually publishes.
//!
//! "Mostly", not "always" — the sweep found that some published examples are
//! genuinely invalid (R4's machine-generated `Questionnaire`s omit the
//! required nested `linkId`; a batch of R4B `CodeSystem`s claim
//! `shareablecodesystem` and omit `publisher`). So a baseline entry is *not*
//! automatically an engine bug. Each one carries a `reason` recording which
//! it is: engine bug, documented engine limitation, or defect in the
//! published example.
//!
//! Structural validation only ([`Validator::validate_sync`]): pure, sync,
//! no network, no FHIRPath. Running the deferred effects pass (invariants
//! and terminology bindings) over the corpus is a worthwhile second tier and
//! deliberately not attempted here — it needs an async runtime and a
//! terminology posture decision, and it would conflate engine bugs with
//! terminology-server availability.
//!
//! # The baseline ratchet
//!
//! Each version has a checked-in manifest under
//! `tests/fixtures/spec-examples/`. The test compares the sweep against it
//! and fails on **any** divergence in either direction:
//!
//! - a newly-failing file — a regression, or a real bug this sweep just found;
//! - a baseline entry that now passes — the manifest is stale and must shrink;
//! - a file failing with *different* issues than recorded — one bug traded
//!   for another;
//! - a change in the resource/non-resource file counts — the corpus moved
//!   under us and the baseline no longer describes it.
//!
//! The manifest can therefore only shrink deliberately, in a reviewable
//! diff. It is a ratchet, not a mute button: nothing here suppresses an
//! issue, it records exactly which files are known-bad and with which error
//! kinds, so the number is visible and reviewable in the repo.
//!
//! # Regenerating a baseline
//!
//! Every run writes the freshly-computed manifest to
//! `target/spec-examples/<version>.actual.json`, whether it passes or fails
//! (CI uploads that directory as an artifact). To accept a change, inspect
//! the diff and copy the file over the checked-in baseline.
//!
//! The sweep never *generates* a `reason`, but it does carry the existing
//! ones across (see [`carry_reasons`]), so copying the generated file over
//! the baseline preserves them. An entry whose issue count or error kinds
//! changed loses its note deliberately: the old explanation may no longer
//! describe what the engine reports.
//!
//! # Why R6 is excluded
//!
//! `crates/fhir/build.rs` **wipes and re-downloads** `tests/data/json/R6`
//! from `build.fhir.org` whenever the R6 feature is on and the local copy is
//! over 24h old. R6 corpus content is therefore volatile and no stable
//! baseline can be pinned to it. R4/R4B/R5 are untouched by that build
//! script and are stable vendored corpora.
//!
//! # Running
//!
//! `#[ignore]`d: the three corpora together are ~600 MB of JSON, which is
//! too slow for the per-PR job. `.github/workflows/validator-conformance.yml`
//! runs them explicitly and asserts the sweep was non-vacuous.
//!
//! ```text
//! cargo test -p helios-fhir-validator --all-features --test spec_examples -- --ignored --nocapture
//! ```

use helios_fhir::FhirVersion;
use helios_fhir_validator::packs::core_registry;
use helios_fhir_validator::{UnknownProfilePolicy, ValidationOptions, Validator};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Floor on resources swept, per version. Guards the vacuity trap that
/// issue #390 describes: without it, a corpus that failed to check out
/// would sweep zero files, find zero failures, and "pass" against a
/// baseline whose `resourcesValidated` we would then also have to trust.
/// Checked *before* the baseline comparison, against a hard-coded number.
const MIN_RESOURCES: usize = 2_000;

// ---------------------------------------------------------------------------
// Manifest model
// ---------------------------------------------------------------------------

/// One known-bad example: the file, how many issues it produced, and the
/// sorted set of error kinds. Kinds are compared, not messages — messages
/// carry paths and values and would make the baseline churn on cosmetic
/// wording changes, while the kind set is the behavioral claim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct KnownFailure {
    file: String,
    issues: usize,
    kinds: Vec<String>,
    /// Hand-written root-cause note: engine bug (with its issue number),
    /// documented engine limitation, or a genuine defect in the published
    /// example. Never generated by the sweep, but carried across a
    /// regeneration by [`carry_reasons`] as long as the entry is unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Manifest {
    description: String,
    version: String,
    corpus: String,
    #[serde(rename = "fhirVersion")]
    fhir_version: String,
    /// Files parsed as FHIR resources and actually validated.
    #[serde(rename = "resourcesValidated")]
    resources_validated: usize,
    /// Files in the corpus directory that are not FHIR resources (stray
    /// archives, package metadata). Pinned so one silently appearing or
    /// vanishing is a failure rather than a shrug.
    #[serde(rename = "nonResourceFiles")]
    non_resource_files: Vec<String>,
    #[serde(rename = "knownFailures")]
    known_failures: Vec<KnownFailure>,
}

const MANIFEST_DESCRIPTION: &str = "Baseline for the official FHIR example corpus sweep in \
     crates/fhir-validator/tests/spec_examples.rs. Every entry is a published spec example that \
     the structural validator reports issues on. Most are false positives in the engine, but not \
     all -- some published examples are genuinely invalid. The per-entry 'reason' says which: \
     engine bug (with issue number), documented engine limitation, or a defect in the example. \
     Entries may only be removed by fixing the engine or re-adjudicating the example, never added \
     without review. Regenerate with the copy of this file the test writes to \
     target/spec-examples/<version>.actual.json; existing 'reason' notes are carried across.";

// ---------------------------------------------------------------------------
// Sweep
// ---------------------------------------------------------------------------

/// The vendored corpus directory for a version.
fn corpus_dir(version_dir: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../fhir/tests/data/json")
        .join(version_dir)
}

fn baseline_path(version_dir: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/spec-examples")
        .join(format!(
            "known-failures-{}.json",
            version_dir.to_lowercase()
        ))
}

/// Where the regenerated manifest is written for artifact upload.
fn output_path(version_dir: &str) -> PathBuf {
    let target = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target"));
    target
        .join("spec-examples")
        .join(format!("{}.actual.json", version_dir.to_lowercase()))
}

/// Per-kind sample issues, printed after a sweep and deliberately *not*
/// stored in the baseline.
///
/// The baseline records error kinds, not messages, so it stays stable when
/// message wording changes. But a kind alone ("primitive-value", 5,688
/// times) does not tell you what is actually wrong. These samples do, at
/// zero churn cost: they live in the CI log next to the counts.
type Samples = BTreeMap<String, (usize, Vec<String>)>;

/// Sample issues retained per error kind.
const SAMPLES_PER_KIND: usize = 8;

fn record_sample(samples: &mut Samples, kind: &str, file: &str, path: &str, message: &str) {
    let entry = samples
        .entry(kind.to_string())
        .or_insert_with(|| (0, Vec::new()));
    entry.0 += 1;
    if entry.1.len() < SAMPLES_PER_KIND {
        entry.1.push(format!("{file} :: {path} -- {message}"));
    }
}

fn print_samples(version_dir: &str, samples: &Samples) {
    if samples.is_empty() {
        return;
    }
    println!("\n{version_dir}: issue kinds, with samples");
    for (kind, (total, examples)) in samples {
        println!("  {kind} ({total} total)");
        for example in examples {
            println!("      {example}");
        }
    }
    println!();
}

/// Validate every example in one corpus directory.
fn sweep(version: FhirVersion, version_dir: &str) -> (Manifest, Samples) {
    let dir = corpus_dir(version_dir);
    assert!(
        dir.is_dir(),
        "corpus directory {} is missing. It is vendored in this repo \
         (crates/fhir/tests/data/json/{version_dir}); a missing directory means a bad checkout, \
         not a reason to skip the sweep.",
        dir.display()
    );

    let validator = Validator::new(core_registry(version));
    let opts = ValidationOptions {
        profiles: Vec::new(),
        // Examples that declare a profile we ship should be judged against
        // it. Ones declaring a profile we do not ship (US Core, IHE, and the
        // like) are out of scope for a *core-spec* sweep -- `Ignore` keeps
        // them from drowning the baseline in unknown-profile noise. Profile
        // coverage is the Inferno job's business (issue #368).
        use_meta_profiles: true,
        unknown_profile: UnknownProfilePolicy::Ignore,
        // The sweep judges the published examples exactly as a default
        // $validate would — the opt-in extension-context warnings stay off.
        ..Default::default()
    };

    // Sort for determinism: readdir order is filesystem-dependent and the
    // baseline diff must be stable across machines.
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read corpus dir {}: {e}", dir.display()))
        .map(|entry| entry.expect("readdir entry").path())
        .filter(|p| p.is_file())
        .collect();
    files.sort();

    let mut non_resource_files = Vec::new();
    let mut known_failures = Vec::new();
    let mut resources_validated = 0usize;
    let mut samples: Samples = BTreeMap::new();

    for path in &files {
        let name = file_name(path);

        // Not every file in the corpus is a resource: the R4B/R5 directories
        // carry a committed `examples.json.zip`, and R4 a `package-min-ver.json`.
        let Ok(bytes) = std::fs::read(path) else {
            non_resource_files.push(name);
            continue;
        };
        let Ok(resource) = serde_json::from_slice::<Value>(&bytes) else {
            non_resource_files.push(name);
            continue;
        };
        if resource
            .get("resourceType")
            .and_then(Value::as_str)
            .is_none()
        {
            non_resource_files.push(name);
            continue;
        }

        resources_validated += 1;
        let outcome = validator.validate_sync(&resource, &opts);
        if outcome.errors.is_empty() {
            continue;
        }

        let mut kinds: Vec<String> = outcome
            .errors
            .iter()
            .map(|e| {
                let kind = serde_json::to_value(e.kind)
                    .expect("ErrorKind serializes")
                    .as_str()
                    .expect("ErrorKind is a string")
                    .to_string();
                record_sample(&mut samples, &kind, &name, &e.path, &e.message);
                kind
            })
            .collect();
        kinds.sort();
        kinds.dedup();

        known_failures.push(KnownFailure {
            file: name,
            issues: outcome.errors.len(),
            kinds,
            reason: None,
        });
    }

    let manifest = Manifest {
        description: MANIFEST_DESCRIPTION.to_string(),
        version: "1.0.0".to_string(),
        corpus: format!("crates/fhir/tests/data/json/{version_dir}"),
        fhir_version: version_dir.to_string(),
        resources_validated,
        non_resource_files,
        known_failures,
    };
    (manifest, samples)
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .expect("corpus entry has a file name")
        .to_string_lossy()
        .into_owned()
}

// ---------------------------------------------------------------------------
// Baseline comparison
// ---------------------------------------------------------------------------

/// Copy the hand-written `reason` notes from the baseline onto the freshly
/// swept manifest, so that regenerating a baseline (`cp` the generated file
/// over the checked-in one, as the README documents) does not silently
/// discard them -- the sweep never generates a `reason`, so without this the
/// first regeneration after anyone writes one would delete every note.
///
/// A note is carried only when the entry is *behaviorally identical* --
/// same issue count, same error kinds. If either moved, the recorded
/// explanation may no longer describe what the engine now reports, so the
/// note is dropped and has to be written again deliberately.
fn carry_reasons(baseline: &Manifest, actual: &mut Manifest) {
    let notes: BTreeMap<&str, &KnownFailure> = baseline
        .known_failures
        .iter()
        .filter(|f| f.reason.is_some())
        .map(|f| (f.file.as_str(), f))
        .collect();
    for entry in &mut actual.known_failures {
        if let Some(prev) = notes.get(entry.file.as_str())
            && prev.issues == entry.issues
            && prev.kinds == entry.kinds
        {
            entry.reason = prev.reason.clone();
        }
    }
}

/// Sweep one version and adjudicate it against the checked-in baseline.
fn run_version(version: FhirVersion, version_dir: &str) {
    let (mut actual, samples) = sweep(version, version_dir);
    print_samples(version_dir, &samples);

    // Read the baseline before writing the regenerated manifest: its
    // hand-written `reason` notes have to be carried across (see
    // `carry_reasons`). A missing file is not fatal here -- the first run of
    // a new version has none, and the comparison below reports it properly.
    let baseline_file = baseline_path(version_dir);
    let read = std::fs::read_to_string(&baseline_file);
    let existing: Option<Manifest> = read.as_ref().ok().map(|raw| {
        serde_json::from_str(raw)
            .unwrap_or_else(|e| panic!("cannot parse baseline {}: {e}", baseline_file.display()))
    });
    if let Some(baseline) = &existing {
        carry_reasons(baseline, &mut actual);
    }

    // Always publish the regenerated manifest, pass or fail -- it is both the
    // CI artifact and the copy-over source for accepting a change.
    let out = output_path(version_dir);
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)
            .unwrap_or_else(|e| panic!("cannot create {}: {e}", parent.display()));
    }
    let rendered = serde_json::to_string_pretty(&actual).expect("manifest serializes");
    std::fs::write(&out, format!("{rendered}\n"))
        .unwrap_or_else(|e| panic!("cannot write {}: {e}", out.display()));
    println!(
        "{version_dir}: validated {} resources, {} with issues -> {}",
        actual.resources_validated,
        actual.known_failures.len(),
        out.display()
    );

    // Vacuity guard, before any baseline comparison.
    assert!(
        actual.resources_validated >= MIN_RESOURCES,
        "{version_dir}: only {} resources validated, expected at least {MIN_RESOURCES}. \
         The sweep found almost nothing, so a green result here would be meaningless.",
        actual.resources_validated
    );

    let baseline = existing.unwrap_or_else(|| {
        let e = read
            .as_ref()
            .expect_err("the manifest is absent only when the read failed");
        panic!(
            "cannot read baseline {}: {e}\n\
             If this is the first run, copy the freshly generated manifest into place:\n  \
             cp {} {}",
            baseline_file.display(),
            out.display(),
            baseline_file.display()
        )
    });

    let mut problems: Vec<String> = Vec::new();

    if baseline.resources_validated != actual.resources_validated {
        problems.push(format!(
            "resource count moved: baseline {}, actual {}. The corpus changed; \
             regenerate the baseline.",
            baseline.resources_validated, actual.resources_validated
        ));
    }
    if baseline.non_resource_files != actual.non_resource_files {
        problems.push(format!(
            "non-resource files moved: baseline {:?}, actual {:?}",
            baseline.non_resource_files, actual.non_resource_files
        ));
    }

    let expected: BTreeMap<&str, &KnownFailure> = baseline
        .known_failures
        .iter()
        .map(|f| (f.file.as_str(), f))
        .collect();
    let found: BTreeMap<&str, &KnownFailure> = actual
        .known_failures
        .iter()
        .map(|f| (f.file.as_str(), f))
        .collect();

    for (file, got) in &found {
        match expected.get(file) {
            None => problems.push(format!(
                "NEW FAILURE  {file}: {} issue(s) {:?} -- not in the baseline. Either the engine \
                 regressed, or this sweep just found a real false positive.",
                got.issues, got.kinds
            )),
            Some(want) if want.kinds != got.kinds || want.issues != got.issues => {
                problems.push(format!(
                    "CHANGED      {file}: baseline {} issue(s) {:?}, now {} issue(s) {:?}",
                    want.issues, want.kinds, got.issues, got.kinds
                ));
            }
            Some(_) => {}
        }
    }
    for file in expected.keys() {
        if !found.contains_key(file) {
            problems.push(format!(
                "FIXED        {file}: validates clean now. Good -- remove it from the baseline \
                 so the ratchet holds."
            ));
        }
    }

    assert!(
        problems.is_empty(),
        "{version_dir}: {} divergence(s) from {} \
         (swept {} resources, {} with issues).\n\
         Regenerated manifest: {}\n\n{}",
        problems.len(),
        baseline_file.display(),
        actual.resources_validated,
        actual.known_failures.len(),
        out.display(),
        problems.join("\n")
    );
}

// ---------------------------------------------------------------------------
// Tests -- one per stable vendored corpus. R6 is excluded on purpose (see the
// module docs: crates/fhir/build.rs re-downloads and wipes it).
// ---------------------------------------------------------------------------

#[cfg(feature = "R4")]
#[test]
#[ignore = "~200 MB corpus sweep; run via validator-conformance.yml or `-- --ignored`"]
fn r4_spec_examples_match_baseline() {
    run_version(FhirVersion::R4, "R4");
}

#[cfg(feature = "R4B")]
#[test]
#[ignore = "~190 MB corpus sweep; run via validator-conformance.yml or `-- --ignored`"]
fn r4b_spec_examples_match_baseline() {
    run_version(FhirVersion::R4B, "R4B");
}

#[cfg(feature = "R5")]
#[test]
#[ignore = "~210 MB corpus sweep; run via validator-conformance.yml or `-- --ignored`"]
fn r5_spec_examples_match_baseline() {
    run_version(FhirVersion::R5, "R5");
}
