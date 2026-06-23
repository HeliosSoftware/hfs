//! Integration tests for `Reference.resolve()` in SQL-on-FHIR ViewDefinitions.
//!
//! These tests pin the behaviour of bundle-level reference resolution: a
//! `ViewDefinition` processing one resource type (e.g. `Encounter`) can follow a
//! `Reference` (`subject.resolve()`) to a *different* resource that lives
//! elsewhere in the same input bundle (e.g. a `Patient`).
//!
//! Background: previously `resolve()` only saw the single resource being
//! processed plus its `contained` children, so `Encounter.subject.resolve()`
//! against a sibling `Patient` returned a typed stub with no fields (null
//! columns). SOF now exposes the whole bundle as the resolution scope, so
//! cross-resource resolution works. See `build_resolution_scope` in
//! `crates/sof/src/lib.rs` and `EvaluationContext::set_resolution_scope`.

use helios_sof::{ContentType, SofBundle, SofViewDefinition, run_view_definition};

fn create_test_bundle(
    resources: &[serde_json::Value],
) -> Result<SofBundle, Box<dyn std::error::Error>> {
    let mut bundle_json = serde_json::json!({
        "resourceType": "Bundle",
        "id": "test-bundle",
        "type": "collection",
        "entry": []
    });

    if let Some(entry_array) = bundle_json["entry"].as_array_mut() {
        for resource in resources {
            entry_array.push(serde_json::json!({ "resource": resource }));
        }
    }

    let bundle: helios_fhir::r4::Bundle = serde_json::from_value(bundle_json)?;
    Ok(SofBundle::R4(bundle))
}

fn parse_view_definition(
    view_json: &serde_json::Value,
) -> Result<SofViewDefinition, Box<dyn std::error::Error>> {
    let mut view_def = view_json.clone();
    if let Some(obj) = view_def.as_object_mut() {
        obj.insert(
            "resourceType".to_string(),
            serde_json::Value::String("ViewDefinition".to_string()),
        );
        obj.insert(
            "status".to_string(),
            serde_json::Value::String("active".to_string()),
        );
    }

    let view_definition: helios_fhir::r4::ViewDefinition = serde_json::from_value(view_def)?;
    Ok(SofViewDefinition::R4(view_definition))
}

fn run(view: &serde_json::Value, bundle: SofBundle) -> Vec<serde_json::Value> {
    let view_definition = parse_view_definition(view).expect("Failed to parse ViewDefinition");
    let result = run_view_definition(view_definition, bundle, ContentType::Json)
        .expect("Failed to run ViewDefinition");
    serde_json::from_slice(&result).expect("Failed to parse result as JSON")
}

/// `Encounter.subject.resolve()` should reach a `Patient` that is a *separate*
/// resource elsewhere in the bundle. This is the core gap the fix addresses.
#[test]
fn resolve_basic_cross_resource() {
    let bundle = create_test_bundle(&[
        serde_json::json!({
            "resourceType": "Patient",
            "id": "pat-1",
            "name": [{ "family": "Sibling", "given": ["Bundle"] }]
        }),
        serde_json::json!({
            "resourceType": "Encounter",
            "id": "enc-1",
            "status": "finished",
            "subject": { "reference": "Patient/pat-1" }
        }),
    ])
    .expect("bundle");

    let view = serde_json::json!({
        "resource": "Encounter",
        "select": [
            { "column": [{ "name": "encounter_id", "path": "id" }] },
            {
                "forEach": "subject.resolve()",
                "column": [
                    { "name": "patient_id", "path": "id" },
                    { "name": "patient_family", "path": "name.family" }
                ]
            }
        ]
    });

    let rows = run(&view, bundle);
    assert_eq!(rows.len(), 1, "expected one row, got {rows:#?}");
    assert_eq!(rows[0]["encounter_id"], serde_json::json!("enc-1"));
    assert_eq!(
        rows[0]["patient_id"],
        serde_json::json!("pat-1"),
        "subject.resolve() must dereference the sibling Patient, not a stub"
    );
    assert_eq!(rows[0]["patient_family"], serde_json::json!("Sibling"));
}

/// A reference that points at a resource not present in the bundle must not
/// error or hang; the resolved-resource columns are simply null.
#[test]
fn resolve_missing_ref_yields_nulls() {
    let bundle = create_test_bundle(&[serde_json::json!({
        "resourceType": "Encounter",
        "id": "enc-1",
        "status": "finished",
        "subject": { "reference": "Patient/does-not-exist" }
    })])
    .expect("bundle");

    let view = serde_json::json!({
        "resource": "Encounter",
        "select": [
            { "column": [{ "name": "encounter_id", "path": "id" }] },
            {
                "forEach": "subject.resolve()",
                "column": [{ "name": "patient_family", "path": "name.family" }]
            }
        ]
    });

    let rows = run(&view, bundle);
    // A typed stub is produced for the unresolved Type/id reference, so the
    // forEach still yields one row, but the target field is null.
    assert_eq!(rows.len(), 1, "expected one row, got {rows:#?}");
    assert_eq!(rows[0]["encounter_id"], serde_json::json!("enc-1"));
    assert!(
        rows[0]["patient_family"].is_null(),
        "missing reference must produce a null column, got {:#?}",
        rows[0]["patient_family"]
    );
}

/// `contained` resolution (the previously-supported case) must keep working
/// alongside the new bundle-level resolution.
#[test]
fn resolve_contained_still_works() {
    let bundle = create_test_bundle(&[serde_json::json!({
        "resourceType": "Encounter",
        "id": "enc-1",
        "status": "finished",
        "contained": [{
            "resourceType": "Patient",
            "id": "pat-1",
            "name": [{ "family": "Contained" }]
        }],
        "subject": { "reference": "#pat-1" }
    })])
    .expect("bundle");

    let view = serde_json::json!({
        "resource": "Encounter",
        "select": [
            { "column": [{ "name": "encounter_id", "path": "id" }] },
            {
                "forEach": "subject.resolve()",
                "column": [
                    { "name": "patient_id", "path": "id" },
                    { "name": "patient_family", "path": "name.family" }
                ]
            }
        ]
    });

    let rows = run(&view, bundle);
    assert_eq!(rows.len(), 1, "expected one row, got {rows:#?}");
    assert_eq!(rows[0]["patient_id"], serde_json::json!("pat-1"));
    assert_eq!(rows[0]["patient_family"], serde_json::json!("Contained"));
}

/// Resolution must work across the *full* bundle: resources are filtered to the
/// ViewDefinition's target type for row generation, but references still resolve
/// to resources of other types. Multiple Encounters resolve to distinct Patients.
#[test]
fn resolve_full_bundle_scope() {
    let bundle = create_test_bundle(&[
        serde_json::json!({
            "resourceType": "Patient",
            "id": "pat-1",
            "name": [{ "family": "Alpha" }]
        }),
        serde_json::json!({
            "resourceType": "Patient",
            "id": "pat-2",
            "name": [{ "family": "Beta" }]
        }),
        serde_json::json!({
            "resourceType": "Encounter",
            "id": "enc-1",
            "status": "finished",
            "subject": { "reference": "Patient/pat-1" }
        }),
        serde_json::json!({
            "resourceType": "Encounter",
            "id": "enc-2",
            "status": "finished",
            "subject": { "reference": "Patient/pat-2" }
        }),
    ])
    .expect("bundle");

    let view = serde_json::json!({
        "resource": "Encounter",
        "select": [
            { "column": [{ "name": "encounter_id", "path": "id" }] },
            {
                "forEach": "subject.resolve()",
                "column": [{ "name": "patient_family", "path": "name.family" }]
            }
        ]
    });

    let mut rows = run(&view, bundle);
    assert_eq!(rows.len(), 2, "expected two rows, got {rows:#?}");
    rows.sort_by(|a, b| a["encounter_id"].as_str().cmp(&b["encounter_id"].as_str()));
    assert_eq!(rows[0]["encounter_id"], serde_json::json!("enc-1"));
    assert_eq!(rows[0]["patient_family"], serde_json::json!("Alpha"));
    assert_eq!(rows[1]["encounter_id"], serde_json::json!("enc-2"));
    assert_eq!(rows[1]["patient_family"], serde_json::json!("Beta"));
}

/// An absolute-URL reference whose trailing `Type/id` matches a bundle resource
/// must resolve.
#[test]
fn resolve_absolute_url() {
    let bundle = create_test_bundle(&[
        serde_json::json!({
            "resourceType": "Patient",
            "id": "pat-1",
            "name": [{ "family": "Absolute" }]
        }),
        serde_json::json!({
            "resourceType": "Encounter",
            "id": "enc-1",
            "status": "finished",
            "subject": { "reference": "http://example.org/fhir/Patient/pat-1" }
        }),
    ])
    .expect("bundle");

    let view = serde_json::json!({
        "resource": "Encounter",
        "select": [{
            "forEach": "subject.resolve()",
            "column": [{ "name": "patient_family", "path": "name.family" }]
        }]
    });

    let rows = run(&view, bundle);
    assert_eq!(rows.len(), 1, "expected one row, got {rows:#?}");
    assert_eq!(rows[0]["patient_family"], serde_json::json!("Absolute"));
}

/// `resolve()` used inside a `where` clause must see the whole bundle too.
#[test]
fn resolve_in_where_clause() {
    let bundle = create_test_bundle(&[
        serde_json::json!({
            "resourceType": "Patient",
            "id": "pat-1",
            "gender": "female"
        }),
        serde_json::json!({
            "resourceType": "Encounter",
            "id": "enc-1",
            "status": "finished",
            "subject": { "reference": "Patient/pat-1" }
        }),
        serde_json::json!({
            "resourceType": "Patient",
            "id": "pat-2",
            "gender": "male"
        }),
        serde_json::json!({
            "resourceType": "Encounter",
            "id": "enc-2",
            "status": "finished",
            "subject": { "reference": "Patient/pat-2" }
        }),
    ])
    .expect("bundle");

    let view = serde_json::json!({
        "resource": "Encounter",
        "where": [{ "path": "subject.resolve().gender = 'female'" }],
        "select": [{ "column": [{ "name": "encounter_id", "path": "id" }] }]
    });

    let rows = run(&view, bundle);
    assert_eq!(
        rows.len(),
        1,
        "only the Encounter whose Patient is female should match, got {rows:#?}"
    );
    assert_eq!(rows[0]["encounter_id"], serde_json::json!("enc-1"));
}
