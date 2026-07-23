//! Regression tests for issue #309 — choice-element (`value[x]`) resolution must
//! consult the FHIR version of the resource being evaluated, not the build's
//! default version.
//!
//! # Why these tests are gated on `all(R4, R5)`
//!
//! The defect only exists in a build where more than one FHIR version feature is
//! compiled in: `polymorphic_access::get_polymorphic_fields` looked up the
//! generated `FIELD_TYPES` table for `FhirVersion::default_enabled()` (R4
//! whenever R4 is enabled) rather than for the resource's own version. In a
//! single-version build the default *is* the resource's version, so there is
//! nothing to catch.
//!
//! That configuration is not exotic: release artifacts are built with
//! `cargo build --workspace --all-features --release` (`.github/workflows/ci.yml`),
//! so every shipped binary has R4/R4B/R5/R6 compiled in with `default_enabled()`
//! == R4.
//!
//! # Why `Observation.value` with `valueAttachment`
//!
//! `Observation.valueAttachment` exists in R5 and R6 but **not** in R4 (verified
//! against the generated tables in `crates/fhir/src/r4.rs` / `r5.rs`). Before the
//! fix, evaluating `Observation.value` on an R5 Observation carrying
//! `valueAttachment` consulted the R4 table, found no `Observation.value*` field
//! matching the data — and, because `consulted_field_types` was set on table
//! *existence* rather than on a match, the prefix-scan fallback was skipped too.
//! The element resolved to **Empty**: silent, total loss of the value rather than
//! a merely mis-typed one.
//!
//! A test using `valueQuantity` would prove nothing — it is present in both
//! tables and resolves either way. That is the trap in the issue's own suggested
//! test ("assert both resolve"), which passes on unfixed code.

#![cfg(all(feature = "R4", feature = "R5"))]

use helios_fhir::{FhirResource, FhirVersion};
use helios_fhirpath::{EvaluationContext, evaluate_expression};
use helios_fhirpath_support::EvaluationResult;

/// Builds an evaluation context whose version is *inferred from the resource*
/// (`EvaluationContext::new` reads `FhirResource::version()`), which is the whole
/// point: the context version must differ from `default_enabled()`.
///
/// Parsing real JSON into the typed `r5::Resource` — rather than hand-building a
/// `HashMap` — is load-bearing. A hand-built object without a `resourceType` key
/// never reaches the `FIELD_TYPES` lookup at all (it falls through to the
/// version-independent prefix scan), so such a test would pass on unfixed code
/// for entirely the wrong reason.
fn r5_context(json: &str) -> EvaluationContext {
    let resource: helios_fhir::r5::Resource =
        serde_json::from_str(json).expect("R5 fixture parses into the typed model");
    EvaluationContext::new(vec![FhirResource::R5(Box::new(resource))])
}

fn r4_context(json: &str) -> EvaluationContext {
    let resource: helios_fhir::r4::Resource =
        serde_json::from_str(json).expect("R4 fixture parses into the typed model");
    EvaluationContext::new(vec![FhirResource::R4(Box::new(resource))])
}

/// Guards against the tests silently becoming vacuous if `default_enabled()` is
/// ever reordered to R5 — at which point the R5 fixtures would stop exercising
/// the cross-version path and would pass for the wrong reason.
fn assert_not_vacuous() {
    assert_ne!(
        FhirVersion::default_enabled(),
        FhirVersion::R5,
        "issue #309 is only exercised when the build default differs from the \
         fixture's version; a default_enabled() reorder makes these tests vacuous"
    );
}

const R5_OBSERVATION_ATTACHMENT: &str = r#"{
    "resourceType": "Observation",
    "id": "obs-attachment",
    "status": "final",
    "code": { "text": "Scanned report" },
    "valueAttachment": { "title": "scanned-report.pdf", "contentType": "application/pdf" }
}"#;

const R4_OBSERVATION_QUANTITY: &str = r#"{
    "resourceType": "Observation",
    "id": "obs-quantity",
    "status": "final",
    "code": { "text": "Body weight" },
    "valueQuantity": { "value": 185, "unit": "lbs", "system": "http://unitsofmeasure.org", "code": "lb_av" }
}"#;

/// The core #309 regression: an R5-only choice variant must resolve through the
/// choice base name when the evaluation context says R5, even though the build
/// default is R4.
#[test]
fn r5_only_choice_variant_resolves_via_choice_base() {
    assert_not_vacuous();
    let context = r5_context(R5_OBSERVATION_ATTACHMENT);

    let exists =
        evaluate_expression("Observation.value.exists()", &context).expect("expression evaluates");
    assert_eq!(
        exists,
        EvaluationResult::boolean(true),
        "Observation.value must resolve to valueAttachment for an R5 resource. \
         Empty here means the R4 FIELD_TYPES table was consulted for an R5 \
         resource (issue #309)."
    );

    let title =
        evaluate_expression("Observation.value.title", &context).expect("expression evaluates");
    assert_eq!(
        title,
        EvaluationResult::string("scanned-report.pdf".to_string()),
        "the resolved choice element must be the Attachment itself"
    );
}

/// Control: the R4 path must be bit-identical after the fix. R4 *is*
/// `default_enabled()`, so any change here means the wrong value was threaded
/// rather than that the bug was fixed.
#[test]
fn r4_choice_variant_still_resolves() {
    let context = r4_context(R4_OBSERVATION_QUANTITY);

    let unit =
        evaluate_expression("Observation.value.unit", &context).expect("expression evaluates");
    assert_eq!(
        unit,
        EvaluationResult::string("lbs".to_string()),
        "R4 choice resolution must be unaffected by threading the context version"
    );
}

/// The same defect on a resource type that exists in *both* tables, isolating
/// "this base is new in R5" from "this parent is new in R5".
///
/// `Person` is present in the R4 table (18 field rows) but gained
/// `deceased[x]` only in R5, so an R4 lookup finds the parent, finds no
/// `deceased*` match, and — before the fix — suppressed the fallback as well.
#[test]
fn r5_only_choice_base_on_a_parent_present_in_both_versions() {
    assert_not_vacuous();
    let context = r5_context(
        r#"{
            "resourceType": "Person",
            "id": "person-deceased",
            "deceasedBoolean": true
        }"#,
    );

    let deceased = evaluate_expression("Person.deceased", &context).expect("expression evaluates");
    assert_eq!(
        deceased,
        EvaluationResult::boolean(true),
        "Person.deceased is R5-only; resolving it must use the R5 table (issue #309)"
    );
}
