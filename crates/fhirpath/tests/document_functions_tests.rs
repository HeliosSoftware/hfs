//! Document functions (`section()`, `entry()`, `resources()`, `sameSubject()`,
//! `hasCoding()`) on documents the upstream `documentFunctions` conformance
//! group doesn't cover: a Composition with a RESTful fullUrl (so relative
//! references resolve), ambiguous fullUrls, fragments and `_history` on
//! references, nested sections, and a Composition evaluated on its own.
//!
//! The conformance cases themselves run in `r4_tests.rs` / `r5_tests.rs`.
#![cfg(feature = "R4")]

use helios_fhir::FhirResource;
use helios_fhirpath::{EvaluationContext, evaluate_expression};
use helios_fhirpath_support::EvaluationResult;
use serde_json::{Value, json};

fn context(resource: Value) -> EvaluationContext {
    let resource: helios_fhir::r4::Resource =
        serde_json::from_value(resource).expect("valid R4 resource");
    EvaluationContext::new(vec![FhirResource::R4(Box::new(resource))])
}

/// The string values the expression returns, in order.
fn strings(expr: &str, ctx: &EvaluationContext) -> Vec<String> {
    let result = evaluate_expression(expr, ctx)
        .unwrap_or_else(|e| panic!("'{expr}' failed to evaluate: {e:?}"));
    let items = match result {
        EvaluationResult::Empty => vec![],
        EvaluationResult::Collection { items, .. } => items,
        other => vec![other],
    };
    items
        .into_iter()
        .map(|item| match item {
            EvaluationResult::String(s, _, _) => s,
            EvaluationResult::Boolean(b, _, _) => b.to_string(),
            EvaluationResult::Integer(i, _, _) => i.to_string(),
            other => panic!("'{expr}' returned a non-string item: {other:?}"),
        })
        .collect()
}

fn condition(id: &str, code: &str, subject: &str) -> Value {
    json!({
        "resourceType": "Condition",
        "id": id,
        "code": { "coding": [{ "system": "http://snomed.info/sct", "code": code }] },
        "subject": { "reference": subject }
    })
}

/// A document whose Composition has a RESTful fullUrl.
fn restful_document() -> Value {
    let base = "http://example.org/fhir/";
    let mut c4 = condition("c4", "4", "Patient/p1");
    c4["meta"] = json!({ "versionId": "3" });
    json!({
        "resourceType": "Bundle",
        "type": "document",
        "entry": [
            {
                "fullUrl": format!("{base}Composition/comp"),
                "resource": {
                    "resourceType": "Composition",
                    "id": "comp",
                    "status": "final",
                    "type": { "text": "summary" },
                    "subject": { "reference": "Patient/p1" },
                    "date": "2026-11-01",
                    "author": [{ "display": "x" }],
                    "title": "t",
                    "section": [
                        {
                            "title": "Problems",
                            "code": { "coding": [{ "system": "http://loinc.org", "code": "11450-4" }] },
                            "entry": [
                                { "reference": "Condition/c1" },
                                { "reference": "Condition/c2#fragment" },
                                { "reference": "Condition/dup" },
                                { "reference": format!("{base}Condition/c4/_history/3") },
                                { "reference": "urn:uuid:missing" }
                            ],
                            "section": [{
                                "title": "Nested",
                                "code": { "coding": [{ "system": "http://loinc.org", "code": "99999-9" }] },
                                "entry": [{ "reference": "Condition/c1" }]
                            }]
                        }
                    ]
                }
            },
            { "fullUrl": format!("{base}Patient/p1"), "resource": { "resourceType": "Patient", "id": "p1" } },
            { "fullUrl": format!("{base}Condition/c1"), "resource": condition("c1", "1", "Patient/p1") },
            { "fullUrl": format!("{base}Condition/c2"), "resource": condition("c2", "2", "Patient/p1") },
            { "fullUrl": format!("{base}Condition/dup"), "resource": condition("dup", "3", "Patient/p1") },
            { "fullUrl": format!("{base}Condition/dup"), "resource": condition("dup", "3", "Patient/p1") },
            { "fullUrl": format!("{base}Condition/c4"), "resource": c4 }
        ]
    })
}

#[test]
fn relative_references_resolve_against_a_restful_composition_full_url() {
    let ctx = context(restful_document());
    // c1 by Type/id, c2 with its fragment set aside, c4 by fullUrl/_history/versionId.
    // `dup` matches two entries, so it is no match; the urn doesn't resolve.
    assert_eq!(
        strings(
            "Bundle.section('http://loinc.org', '11450-4').entry({}, {}).id",
            &ctx
        ),
        ["c1", "c2", "c4"]
    );
}

#[test]
fn nested_sections_are_found() {
    let ctx = context(restful_document());
    assert_eq!(
        strings("Bundle.section('http://loinc.org', '99999-9').title", &ctx),
        ["Nested"]
    );
    assert_eq!(
        strings(
            "Bundle.section('http://loinc.org', '99999-9').entry('http://snomed.info/sct', '1').id",
            &ctx
        ),
        ["c1"]
    );
    assert_eq!(
        strings(
            "Bundle.section('http://loinc.org', {}, 0, '*').count()",
            &ctx
        ),
        ["2"]
    );
}

#[test]
fn resources_from_anything_in_the_document() {
    let ctx = context(restful_document());
    // The focus is a resource in the document, so the document is the root.
    assert_eq!(
        strings(
            "Bundle.entry.resource.ofType(Patient).resources('http://snomed.info/sct', '2').id",
            &ctx
        ),
        ["c2"]
    );
    // Each document is searched once, even with several inputs in it.
    assert_eq!(
        strings(
            "(Bundle | Bundle.entry.resource.ofType(Condition)).resources({}, '1', 'Condition').count()",
            &ctx
        ),
        ["1"]
    );
}

#[test]
fn same_subject_needs_a_document_bundle() {
    let ctx = context(restful_document());
    assert_eq!(
        strings(
            "Bundle.entry.resource.ofType(Condition).first().sameSubject()",
            &ctx
        ),
        ["true"]
    );

    let mut collection = restful_document();
    collection["type"] = json!("collection");
    let ctx = context(collection);
    assert!(
        strings(
            "Bundle.entry.resource.ofType(Condition).first().sameSubject()",
            &ctx
        )
        .is_empty()
    );
    // With `other` given, the Bundle's type doesn't matter.
    assert_eq!(
        strings(
            "Bundle.entry.resource.ofType(Condition).first().sameSubject(%resource.entry.resource.ofType(Composition))",
            &ctx
        ),
        ["true"]
    );
}

#[test]
fn a_composition_on_its_own_resolves_only_contained_references() {
    let ctx = context(json!({
        "resourceType": "Composition",
        "id": "comp",
        "status": "final",
        "type": { "text": "summary" },
        "date": "2026-11-01",
        "author": [{ "display": "x" }],
        "title": "t",
        "contained": [condition("inner", "5", "Patient/p1")],
        "section": [{
            "code": { "coding": [{ "system": "http://loinc.org", "code": "11450-4" }] },
            "entry": [{ "reference": "#inner" }, { "reference": "Condition/c1" }]
        }]
    }));
    assert_eq!(
        strings(
            "Composition.section('http://loinc.org', '11450-4').entry({}, {}).id",
            &ctx
        ),
        ["inner"]
    );
}

#[test]
fn has_coding_matches_system_code_and_version() {
    let ctx = context(json!({
        "resourceType": "Condition",
        "id": "c",
        "code": { "coding": [{ "system": "http://snomed.info/sct", "version": "2025", "code": "1" }] },
        "subject": { "reference": "Patient/p1" }
    }));
    assert_eq!(
        strings(
            "code.hasCoding('http://snomed.info/sct', '1', '2025')",
            &ctx
        ),
        ["true"]
    );
    assert_eq!(
        strings(
            "code.hasCoding('http://snomed.info/sct', '1', '2024')",
            &ctx
        ),
        ["false"]
    );
    assert_eq!(strings("code.coding.hasCoding({}, '1')", &ctx), ["true"]);
    assert!(strings("category.hasCoding({}, {})", &ctx).is_empty());
}

#[test]
fn invalid_parameters_are_errors() {
    let ctx = context(restful_document());
    for expr in [
        "Bundle.section('http://loinc.org', '11450-4', 2, 1)",
        "Bundle.section('http://loinc.org', '11450-4', 1, 'many')",
        "Bundle.section('http://loinc.org', '11450-4', 1)",
        "Bundle.resources('http://snomed.info/sct', '1', 'condition')",
        "Bundle.resourcesIn('not a value set')",
        // Checked even though the value set is empty.
        "Bundle.section('http://loinc.org', '11450-4').entryIn({}, 2, 1)",
        "Bundle.resourcesIn({}, 'condition')",
        "Bundle.entry.resource.first().sameSubject(%resource, %resource)",
    ] {
        assert!(
            evaluate_expression(expr, &ctx).is_err(),
            "'{expr}' should be an error"
        );
    }
    // An empty value set needs no terminology server.
    assert!(strings("Bundle.resourcesIn({})", &ctx).is_empty());
}

#[test]
fn resource_id_is_typed_id() {
    let ctx = context(restful_document());
    let result = evaluate_expression("Bundle.entry.resource.ofType(Patient).id", &ctx).unwrap();
    match result {
        EvaluationResult::String(s, Some(t), _) => {
            assert_eq!(
                (s.as_str(), t.namespace.as_str(), t.name.as_str()),
                ("p1", "FHIR", "id")
            );
        }
        other => panic!("expected a typed id, got {other:?}"),
    }
    // Element.id keeps its string type.
    let ctx = context(json!({
        "resourceType": "Patient",
        "id": "p",
        "name": [{ "id": "n1", "family": "F" }]
    }));
    match evaluate_expression("Patient.name.id", &ctx).unwrap() {
        EvaluationResult::String(_, Some(t), _) => assert_eq!(t.name, "string"),
        other => panic!("expected a typed string, got {other:?}"),
    }
}
