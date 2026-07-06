//! End-to-end FHIRPath invariant evaluation (feature `fhirpath`): real
//! spec constraints from the embedded R4 pack, evaluated by helios-fhirpath
//! through the deferred-effects pipeline.
//!
//! Run with: `cargo test -p helios-fhir-validator --features fhirpath`
//!
//! ## MSVC 14.44 linker caveat
//!
//! This binary links helios-fhirpath's full dependency tree and can trip the
//! MSVC 14.44 (`link.exe` 14.44.35207) `LNK1318: Unexpected PDB error;
//! LIMIT (12)` defect on Windows. If that happens, the same scenarios are
//! covered end-to-end by `crates/rest/tests/validate_operation_tests.rs`
//! (`validate_evaluates_real_fhirpath_invariants`), whose test binary links
//! fine; alternatively add `-C link-arg=/PDBPAGESIZE:8192` to the MSVC
//! target rustflags or use a non-affected MSVC toolchain.

#![cfg(all(feature = "fhirpath", feature = "R4"))]

use helios_fhir::FhirVersion;
use helios_fhir_validator::fhirpath_effects::FhirPathConstraintEvaluator;
use helios_fhir_validator::packs::core_registry;
use helios_fhir_validator::{EffectHandlers, ErrorKind, ValidationOptions, Validator};
use serde_json::json;

fn handlers(evaluator: &FhirPathConstraintEvaluator) -> EffectHandlers<'_> {
    EffectHandlers { constraints: Some(evaluator), ..Default::default() }
}

/// dom-6 (narrative present) is a warning-severity invariant that fires on
/// almost every test resource; suppress it like the server default does.
fn suppress() -> Vec<String> {
    vec!["dom-6".to_string()]
}

#[tokio::test]
async fn real_pat1_invariant_fires_on_detail_less_contact() {
    let validator = Validator::new(core_registry(FhirVersion::R4));
    let evaluator = FhirPathConstraintEvaluator::new();
    let suppress = suppress();
    let mut h = handlers(&evaluator);
    h.suppress_constraints = &suppress;

    // pat-1: "SHALL at least contain a contact's details or a reference to
    // an organization" — a contact with only a gender violates it.
    let violating = json!({
        "resourceType": "Patient",
        "contact": [{ "gender": "male" }]
    });
    let errors = validator
        .validate(&violating, FhirVersion::R4, &ValidationOptions::default(), &h)
        .await;
    let pat1: Vec<_> = errors
        .iter()
        .filter(|e| e.kind == ErrorKind::FhirpathConstraint && e.path == "Patient.contact.0")
        .collect();
    assert_eq!(
        pat1.len(),
        1,
        "pat-1 must fire exactly once at the contact node; all issues: {}",
        serde_json::to_string_pretty(&errors).unwrap()
    );
    assert_eq!(
        pat1[0].extra.get("constraint"),
        Some(&json!("pat-1")),
        "constraint id carried in extras"
    );

    let satisfied = json!({
        "resourceType": "Patient",
        "contact": [{ "gender": "male", "name": { "family": "Smith" } }]
    });
    let errors = validator
        .validate(&satisfied, FhirVersion::R4, &ValidationOptions::default(), &h)
        .await;
    assert!(
        !errors.iter().any(|e| e.kind == ErrorKind::FhirpathConstraint
            && e.extra.get("constraint") == Some(&json!("pat-1"))),
        "pat-1 must pass when the contact has a name; got: {}",
        serde_json::to_string_pretty(&errors).unwrap()
    );
}

#[tokio::test]
async fn absent_nodes_pass_invariants() {
    // FHIR invariant semantics: a constraint over an absent element is
    // vacuously true (empty → pass).
    let validator = Validator::new(core_registry(FhirVersion::R4));
    let evaluator = FhirPathConstraintEvaluator::new();
    let suppress = suppress();
    let mut h = handlers(&evaluator);
    h.suppress_constraints = &suppress;

    let minimal = json!({ "resourceType": "Patient" });
    let errors = validator
        .validate(&minimal, FhirVersion::R4, &ValidationOptions::default(), &h)
        .await;
    let hard_failures: Vec<_> = errors
        .iter()
        .filter(|e| e.severity == helios_fhir_validator::Severity::Error)
        .collect();
    assert!(
        hard_failures.is_empty(),
        "a minimal Patient must have no error-severity issues, got: {}",
        serde_json::to_string_pretty(&errors).unwrap()
    );
}
