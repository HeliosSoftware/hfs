//! Differential-testing harness: our engine vs. the HL7 reference validator
//! (issue #427). **Spike posture — phase 1.**
//!
//! # What this is (and is not)
//!
//! Issue #427 asks, before any large harness is built, for a *spike* that
//! answers three measured unknowns — is the reference validator reusable, how
//! fast is it, and how comparable is its output — and to **post the numbers
//! before designing the full thing**. This file is that spike, made
//! reproducible and reviewable:
//!
//! - It runs our structural engine over a **bounded, deterministic sample** of
//!   the vendored FHIR example corpus.
//! - It reads the reference validator's captured output for the *same* sample
//!   (produced by `tests/scripts/run_reference_validator.sh`, which provisions
//!   `validator_cli.jar` — the cheap path proven by `hts-ig-conformance.yml`).
//! - It diffs the two into the three buckets of #427 — `both` /
//!   `only_ours` (false positive) / `only_theirs` (**false negative**) — via
//!   the unit-tested [`normalize`] comparability layer.
//! - It writes a machine-readable artifact and a human summary.
//!
//! It is **not** a merge gate, **not** per-PR, and **not** the full
//! 2,329-entry adjudication writer. Those are phase 2, explicitly gated on the
//! throughput/output-shape numbers this produces (see the workflow and the
//! fixtures/differential/README.md).
//!
//! # Running
//!
//! The corpus-consuming tests are `#[ignore]`d (they need the JVM reference
//! output). The [`normalize`] module's unit tests run in the ordinary
//! `cargo test` and need no Java. In CI:
//!
//! ```text
//! tests/scripts/run_reference_validator.sh R4     # provisions jar, runs sample
//! cargo test -p helios-fhir-validator --all-features --test differential -- --ignored --nocapture
//! ```

#[path = "differential/normalize.rs"]
mod normalize;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use helios_fhir::FhirVersion;
use helios_fhir_validator::packs::core_registry;
use helios_fhir_validator::{UnknownProfilePolicy, ValidationOptions, Validator};
use normalize::{
    Diff, Finding, class_of_ours, class_of_reference, diff_file, normalize_path,
    reference_severity_is_error,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Default sample size per version when `DIFFERENTIAL_SAMPLE_SIZE` is unset.
/// The issue's suggested spike is "push 50 resources through it".
const DEFAULT_SAMPLE_SIZE: usize = 50;

// ---------------------------------------------------------------------------
// Reference-validator intermediate format
//
// `run_reference_validator.sh` normalizes `validator_cli.jar`'s per-file
// OperationOutcome output into this shape so the Rust side never has to parse
// the validator's (version-dependent) native output. This is the contract
// between the shell driver and this test.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ReferenceRun {
    version: String,
    results: Vec<ReferenceResult>,
}

#[derive(Debug, Deserialize)]
struct ReferenceResult {
    file: String,
    /// Wall-clock milliseconds the reference validator spent on this file
    /// (JVM start included — the per-resource figure #427 asks for).
    #[serde(rename = "wallMs", default)]
    wall_ms: u64,
    #[serde(default)]
    issues: Vec<ReferenceIssue>,
}

#[derive(Debug, Deserialize)]
struct ReferenceIssue {
    #[serde(default)]
    severity: String,
    #[serde(default)]
    code: String,
    /// FHIRPath expression / location the issue anchors to.
    #[serde(default)]
    expression: String,
}

// ---------------------------------------------------------------------------
// Diff artifact
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct FileDiff {
    file: String,
    both: Vec<String>,
    only_ours: Vec<String>,
    /// False-negative candidates — structural issues the reference validator
    /// raised and we did not.
    only_theirs: Vec<String>,
    /// Terminology/invariant reference findings, excluded from the
    /// false-negative bucket but counted (never silently dropped).
    out_of_scope_theirs: Vec<String>,
}

#[derive(Debug, Serialize)]
struct DiffReport {
    version: String,
    sampled: usize,
    /// Summed reference wall-clock over the sample, and the per-resource mean —
    /// the throughput number the spike exists to produce.
    reference_total_ms: u64,
    reference_mean_ms_per_resource: f64,
    /// Bucket totals across the sample.
    files_with_false_negatives: usize,
    files_with_false_positives: usize,
    files_agreeing: usize,
    total_false_negative_findings: usize,
    total_false_positive_findings: usize,
    total_out_of_scope_findings: usize,
    files: Vec<FileDiff>,
}

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

fn corpus_dir(version_dir: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../fhir/tests/data/json")
        .join(version_dir)
}

fn artifact_dir() -> PathBuf {
    std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target"))
        .join("differential")
}

/// Where the shell driver writes the reference output for a version.
fn reference_path(version_dir: &str) -> PathBuf {
    artifact_dir().join(format!("{}.reference.json", version_dir.to_lowercase()))
}

// ---------------------------------------------------------------------------
// Deterministic sampling
// ---------------------------------------------------------------------------

/// The deterministic sample of corpus files for a version.
///
/// Determinism is essential: the shell driver and this test must pick the
/// **exact same files**, or the diff compares different resources. Both sides
/// use this same rule — sort file names lexically, take the first `n` that
/// parse as FHIR resources. The shell driver mirrors it with `ls | sort | head`.
///
/// This over-samples the alphabetic head rather than trying to be clever: a
/// spike's job is throughput + output-shape, and a fixed, obvious rule is
/// auditable. Richer sampling (baseline entries + clean resources, to load the
/// false-negative bucket) is a phase-2 refinement noted in the README.
fn sample_files(version_dir: &str, n: usize) -> Vec<PathBuf> {
    let dir = corpus_dir(version_dir);
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read corpus dir {}: {e}", dir.display()))
        .map(|e| e.expect("readdir entry").path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    files
        .into_iter()
        .filter(|p| {
            std::fs::read(p)
                .ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                .and_then(|v| {
                    v.get("resourceType")
                        .and_then(Value::as_str)
                        .map(String::from)
                })
                .is_some()
        })
        .take(n)
        .collect()
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .expect("has file name")
        .to_string_lossy()
        .into_owned()
}

/// Run our structural engine over one resource file and return its normalized,
/// error-severity findings.
fn our_findings(validator: &Validator, opts: &ValidationOptions, path: &Path) -> Vec<Finding> {
    let bytes = std::fs::read(path).expect("read sample file");
    let resource: Value = serde_json::from_slice(&bytes).expect("sample parses");
    let outcome = validator.validate_sync(&resource, opts);
    outcome
        .errors
        .iter()
        .map(|e| {
            let kind = serde_json::to_value(e.kind)
                .expect("ErrorKind serializes")
                .as_str()
                .expect("ErrorKind is a string")
                .to_string();
            Finding {
                class: class_of_ours(&kind),
                path: normalize_path(&e.path),
            }
        })
        .collect()
}

/// Convert one reference result's issues into normalized, error-severity findings.
fn reference_findings(result: &ReferenceResult) -> Vec<Finding> {
    result
        .issues
        .iter()
        .filter(|i| reference_severity_is_error(&i.severity))
        .map(|i| Finding {
            class: class_of_reference(&i.code),
            path: normalize_path(&i.expression),
        })
        .collect()
}

/// Spike driver for one version. Reads the reference output the shell driver
/// produced, runs our engine over the same sample, diffs, and writes the report.
fn run_version(version: FhirVersion, version_dir: &str) {
    let ref_path = reference_path(version_dir);
    let raw = std::fs::read_to_string(&ref_path).unwrap_or_else(|e| {
        panic!(
            "reference output {} is missing ({e}).\n\
             Produce it first with:\n  \
             tests/scripts/run_reference_validator.sh {version_dir}\n\
             (it provisions validator_cli.jar and runs the same deterministic sample).",
            ref_path.display()
        )
    });
    let reference: ReferenceRun = serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("cannot parse {}: {e}", ref_path.display()));
    assert_eq!(
        reference.version, version_dir,
        "reference output is for {}, expected {version_dir}",
        reference.version
    );

    // Non-vacuity floor (Persona 4): a spike that adjudicated nothing must not
    // report a clean bill of health. The reference run must cover a real sample.
    assert!(
        !reference.results.is_empty(),
        "{version_dir}: reference output has zero results; the validator run produced nothing to \
         diff. A green here would be meaningless."
    );

    let by_file: BTreeMap<&str, &ReferenceResult> = reference
        .results
        .iter()
        .map(|r| (r.file.as_str(), r))
        .collect();

    let validator = Validator::new(core_registry(version));
    let opts = ValidationOptions {
        profiles: Vec::new(),
        use_meta_profiles: true,
        // Same posture as `spec_examples.rs`: a core-spec sweep ignores
        // unknown (US Core / IHE) profiles rather than drowning in noise.
        unknown_profile: UnknownProfilePolicy::Ignore,
        ..Default::default()
    };

    // Diff each file the reference validator actually processed.
    let mut files: Vec<FileDiff> = Vec::new();
    let mut total_ms: u64 = 0;
    let (mut fn_files, mut fp_files, mut agree_files) = (0usize, 0usize, 0usize);
    let (mut fn_total, mut fp_total, mut oos_total) = (0usize, 0usize, 0usize);

    // Sort for a stable artifact diff.
    let mut result_files: Vec<&str> = by_file.keys().copied().collect();
    result_files.sort();

    for name in result_files {
        let result = by_file[name];
        total_ms += result.wall_ms;
        let path = corpus_dir(version_dir).join(name);
        let ours = if path.is_file() {
            our_findings(&validator, &opts, &path)
        } else {
            // The reference validator processed a file our sampler did not
            // resolve (e.g. removed from the corpus). Record it as all-theirs
            // rather than dropping it.
            Vec::new()
        };
        let theirs = reference_findings(result);
        let Diff {
            both,
            only_ours,
            only_theirs,
            out_of_scope_theirs,
        } = diff_file(&ours, &theirs);

        if !only_theirs.is_empty() {
            fn_files += 1;
        }
        if !only_ours.is_empty() {
            fp_files += 1;
        }
        if only_theirs.is_empty() && only_ours.is_empty() {
            agree_files += 1;
        }
        fn_total += only_theirs.len();
        fp_total += only_ours.len();
        oos_total += out_of_scope_theirs.len();

        files.push(FileDiff {
            file: name.to_string(),
            both: both.into_iter().collect(),
            only_ours: only_ours.into_iter().collect(),
            only_theirs: only_theirs.into_iter().collect(),
            out_of_scope_theirs: out_of_scope_theirs.into_iter().collect(),
        });
    }

    let sampled = reference.results.len();
    let report = DiffReport {
        version: version_dir.to_string(),
        sampled,
        reference_total_ms: total_ms,
        reference_mean_ms_per_resource: if sampled == 0 {
            0.0
        } else {
            total_ms as f64 / sampled as f64
        },
        files_with_false_negatives: fn_files,
        files_with_false_positives: fp_files,
        files_agreeing: agree_files,
        total_false_negative_findings: fn_total,
        total_false_positive_findings: fp_total,
        total_out_of_scope_findings: oos_total,
        files,
    };

    let out_dir = artifact_dir();
    std::fs::create_dir_all(&out_dir)
        .unwrap_or_else(|e| panic!("cannot create {}: {e}", out_dir.display()));
    let out = out_dir.join(format!("{}.diff.json", version_dir.to_lowercase()));
    std::fs::write(
        &out,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&report).expect("report serializes")
        ),
    )
    .unwrap_or_else(|e| panic!("cannot write {}: {e}", out.display()));

    // Human summary — the numbers the issue asks to post.
    println!(
        "\n{version_dir} differential spike ({sampled} resources)\n\
         ------------------------------------------------------------\n\
         reference wall-clock: {total_ms} ms total, {:.1} ms/resource (JVM start incl.)\n\
         files: {agree_files} agree, {fp_files} with false positives, {fn_files} with false negatives\n\
         findings: {fp_total} false-positive, {fn_total} FALSE-NEGATIVE (structural), \
         {oos_total} out-of-scope (terminology/invariant, excluded)\n\
         artifact: {}\n",
        report.reference_mean_ms_per_resource,
        out.display(),
    );
    println!(
        "NOTE: `{fn_total}` structural false-negative findings are the highest-value output of \
         this spike (issue #427). Inspect them in {} before scoping phase 2.",
        out.display(),
    );
}

// ---------------------------------------------------------------------------
// Per-version tests — `#[ignore]`d: each needs the JVM reference output.
// R6 excluded for the same reason as spec_examples.rs (build.rs re-downloads it).
// ---------------------------------------------------------------------------

#[cfg(feature = "R4")]
#[test]
#[ignore = "needs validator_cli.jar output; run tests/scripts/run_reference_validator.sh R4 first"]
fn r4_differential() {
    run_version(FhirVersion::R4, "R4");
}

#[cfg(feature = "R4B")]
#[test]
#[ignore = "needs validator_cli.jar output; run tests/scripts/run_reference_validator.sh R4B first"]
fn r4b_differential() {
    run_version(FhirVersion::R4B, "R4B");
}

#[cfg(feature = "R5")]
#[test]
#[ignore = "needs validator_cli.jar output; run tests/scripts/run_reference_validator.sh R5 first"]
fn r5_differential() {
    run_version(FhirVersion::R5, "R5");
}

// ---------------------------------------------------------------------------
// Java-free tests: prove the sampler is deterministic and the artifact plumbing
// works, without the reference validator. (The normalize module carries the
// comparability-layer unit tests.)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod harness_tests {
    use super::*;

    fn env_sample_size() -> usize {
        std::env::var("DIFFERENTIAL_SAMPLE_SIZE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_SAMPLE_SIZE)
    }

    #[test]
    fn sample_size_default_is_fifty() {
        // Guards the spike's documented default against silent drift.
        assert_eq!(DEFAULT_SAMPLE_SIZE, 50);
    }

    #[cfg(feature = "R4")]
    #[test]
    fn sampling_is_deterministic_and_bounded() {
        let n = env_sample_size();
        let a = sample_files("R4", n);
        let b = sample_files("R4", n);
        assert_eq!(
            a, b,
            "the sample must be identical across calls (shell driver relies on it)"
        );
        assert!(a.len() <= n, "sample is bounded by n");
        assert!(
            !a.is_empty(),
            "R4 corpus is vendored; sample must be non-empty"
        );
        // Every sampled file parses as a FHIR resource.
        for p in &a {
            let v: Value = serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap();
            assert!(v.get("resourceType").and_then(Value::as_str).is_some());
        }
    }

    #[cfg(feature = "R4")]
    #[test]
    fn our_engine_runs_over_the_sample() {
        // Proves the our-side half works with no JVM: run the engine over a tiny
        // sample and confirm findings normalize without panicking.
        let files = sample_files("R4", 5);
        let validator = Validator::new(core_registry(FhirVersion::R4));
        let opts = ValidationOptions {
            profiles: Vec::new(),
            use_meta_profiles: true,
            unknown_profile: UnknownProfilePolicy::Ignore,
            ..Default::default()
        };
        for p in &files {
            let findings = our_findings(&validator, &opts, p);
            for f in &findings {
                assert!(
                    !f.path.contains('['),
                    "normalized path keeps no FHIRPath index: {}",
                    f.path
                );
            }
        }
        let _ = file_name(&files[0]);
    }
}
